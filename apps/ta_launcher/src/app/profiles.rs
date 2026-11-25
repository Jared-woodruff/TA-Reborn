//! Local profile selector and the settings/data handover between commanders.
use ta_ui::tooltip::TooltipExt as _;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use egui::{Align, Layout, Rect, RichText, Ui, UiBuilder, pos2, vec2};
use ta_config::profiles::{Career, Problem, Profile, ProfileStore, Unreadable};
use ta_ui::i18n::{grouped, n, t, tf};
use ta_ui::profiles::{CareerData, CareerView, ProfileDraft, ProfileEditor};
use ta_ui::panel::{Exit, Panel};
use crate::persist::ConfigStore;
use crate::theme::*;
use super::LauncherApp;

/// The parts of the Profile page: the service record's tabs and the commanders of this device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ProfileTab { #[default] Overview, Achievements, Campaign, History, Commanders }

/// What the Profile page loads off the interface's thread: the active commander's record, and
/// the careers of every commander of this device (their ranks, in the Commanders tab; those
/// that cannot be read apart).
pub(super) struct Details {
    pub record: CareerData,
    pub careers: std::collections::BTreeMap<String, Career>,
    pub unreadable: std::collections::BTreeSet<String>,
}

/// Why the Profile page's record could not be read: the commander's career (`career`), or
/// another of their files; the file's or the system's own words.
pub(super) struct DetailsFailed {
    pub career: bool,
    pub error: String,
}

/// What is known of the active commander's career, their service record (`career.toml`): it is
/// read off the interface's thread, and may be unreadable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum CareerFile {
    /// Not read yet: being read.
    #[default]
    Unknown,
    /// Read: [`Profiles::career`] is it.
    Read,
    /// It cannot be read: why, in the file's or the system's own words. No battle is started
    /// until it is put right (the Profile page offers a fresh career, the damaged one kept).
    Damaged(String),
}

/// A career read: it, and the achievements it had met that were unlocked as it was read.
type CareerRead = Result<(Career, Vec<String>), String>;

pub(super) struct Profiles {
    pub store: Option<ProfileStore>,
    pub active: Option<Profile>,
    pub(super) list: Vec<Profile>,
    /// This device's profiles that cannot be read: listed apart, with their folders, to be put
    /// right or removed. They never disable a healthy commander, nor keep a name from one.
    pub(super) unreadable: Vec<Unreadable>,
    /// The active commander's career as last read, and what is known of its file.
    pub(super) career: Career,
    pub(super) career_file: CareerFile,
    /// The career is to be read again, off the interface's thread; and on opening a profile, with
    /// what its play has met unlocked first (an update having brought it), their notices shown
    /// when `notify_unlocks`.
    career_stale: bool,
    unlock_due: bool,
    notify_unlocks: bool,
    /// The read under way: whose career, and where it comes.
    career_pending: Option<(String, Receiver<CareerRead>)>,
    pub error: Option<String>,
    /// The profiles' folder could not be listed.
    pub(super) list_error: Option<String>,
    editor: Option<ProfileEditor>,
    editing: bool,
    pub required: bool,
    /// The Profile page's tab, and a page asked to be shown (`--show`, the menu).
    pub(super) tab: ProfileTab,
    pub(super) open_page: Option<ProfileTab>,
    pub(super) view: CareerView,
    pub(super) details: Option<Details>,
    pub(super) details_pending: Option<Receiver<Result<Details, DetailsFailed>>>,
    pub(super) details_error: Option<String>,
    /// A commander whose profile is to be deleted, once asked.
    pub(super) confirm_delete: Option<String>,
    /// A fresh career, the damaged one set aside: once asked.
    pub(super) confirm_fresh: bool,
    /// The Profile page is laid out at its own height in a page that scrolls whole (a short
    /// window, a large interface), its tabs without scrolls of their own.
    pub(super) natural: bool,
    open_menu: bool,
    /// Notices of achievements the commander's earlier play has earned (an update having
    /// brought them), shown on opening the profile.
    pub toasts: ta_ui::profiles::CareerNotices,
}

impl Profiles {
    pub fn open(root: PathBuf, required: bool) -> Self {
        let mut state = Self { store: None, active: None, list: Vec::new(), unreadable: Vec::new(), career: Career::default(),
            career_file: CareerFile::Unknown, career_stale: false, unlock_due: false, notify_unlocks: false, career_pending: None,
            error: None, list_error: None, editor: None, editing: false, required, tab: ProfileTab::Overview, open_page: None,
            view: CareerView::default(), details: None, details_pending: None, details_error: None, confirm_delete: None,
            confirm_fresh: false, natural: false, open_menu: false, toasts: Default::default() };
        let result = (|| -> std::io::Result<()> {
            let store = ProfileStore::open(root.clone())?;
            state.store = Some(store.clone());
            state.active = if cfg!(test) { store.active()? } else { ta_config::profiles::initialize(root, None)? };
            state.store = Some(store);
            Ok(())
        })();
        if let Err(error) = result { state.error = Some(error.to_string()); }
        // (The career is read off the interface's thread from the first frame: what its play
        // has met unlocked first, with notices unless captured.)
        state.begin_career(required);
        state.refresh_list();
        if state.active.is_none() && required && state.error.is_none() { state.edit(false); }
        state
    }
    pub(super) fn dialog_open(&self) -> bool { self.editor.is_some() || self.confirm_delete.is_some() || self.confirm_fresh }
    pub(super) fn unsaved_edits(&self) -> bool { self.editor.as_ref().is_some_and(ProfileEditor::changed) }

    /// A commander's career to be read anew (on opening their profile): nothing known of it yet,
    /// what its play has met to be unlocked as it is read, with notices when `notify`.
    fn begin_career(&mut self, notify: bool) {
        self.career = Career::default();
        self.career_file = CareerFile::Unknown;
        self.career_pending = None;
        self.career_stale = false;
        self.unlock_due = self.active.is_some();
        self.notify_unlocks = notify;
    }

    /// The career from a read under way, and another begun when one is due, off the interface's
    /// thread (`page_reads`: the Profile page reads the record, and the career with it).
    pub(super) fn poll_career(&mut self, ctx: &egui::Context, page_reads: bool) {
        if let Some((id, receiver)) = &self.career_pending {
            let read = match receiver.try_recv() {
                Ok(read) => read,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => Err(t("Could not load profile.").to_string()),
            };
            let id = id.clone();
            self.career_pending = None;
            self.take_career(&id, read);
        }
        if self.unlock_due || (self.career_stale && !page_reads) {
            self.read_career(Some(ctx.clone()));
        }
    }

    /// Reads the active commander's career on a thread of its own (`ctx` asked to draw again when
    /// it comes).
    fn read_career(&mut self, ctx: Option<egui::Context>) {
        self.career_stale = false;
        let unlock = std::mem::take(&mut self.unlock_due);
        let (Some(store), Some(id)) = (self.store.clone(), self.active.as_ref().map(|p| p.id.clone())) else { return };
        let (send, receive) = std::sync::mpsc::channel();
        let reader = { let id = id.clone(); move || {
            let _ = send.send(read_career(&store, &id, unlock));
            if let Some(ctx) = ctx { ctx.request_repaint(); }
        } };
        match std::thread::Builder::new().name("career".into()).spawn(reader) {
            Ok(_) => self.career_pending = Some((id, receive)),
            Err(error) => {
                log::warn!("could not read the career off the interface's thread: {error}");
                self.unlock_due = unlock;
                self.settle_career();
            }
        }
    }

    /// A career read taken in, if it is still the active commander's.
    fn take_career(&mut self, id: &str, read: CareerRead) {
        if self.active.as_ref().is_none_or(|p| p.id != id) { return; }
        match read {
            Ok((career, unlocked)) => {
                self.career = career;
                self.career_file = CareerFile::Read;
                if !unlocked.is_empty() {
                    if self.notify_unlocks { self.toasts.push(&unlocked, &unlocked); }
                    // (The record shown is read again, with them.)
                    self.details = None;
                    self.details_pending = None;
                    self.details_error = None;
                }
            }
            Err(error) => {
                log::warn!("the career of profile {id} cannot be read: {error}");
                self.career_file = CareerFile::Damaged(error);
            }
        }
    }

    /// The career as the Profile page's record read it (with its history), or why it could not.
    pub(super) fn career_from_record(&mut self, read: Result<&Career, &str>) {
        self.career_stale = false;
        match read {
            Ok(career) => {
                self.career = career.clone();
                self.career_file = CareerFile::Read;
            }
            Err(error) => {
                log::warn!("the career cannot be read: {error}");
                self.career_file = CareerFile::Damaged(error.to_string());
            }
        }
    }

    /// The career known, for a battle about to start (it must be recorded): the read under way
    /// waited for, or read here when nothing is known of it, or it could not be read (it may
    /// have been put right since). A career read already is not read again.
    pub(super) fn settle_career(&mut self) {
        let mut taken = false;
        if let Some((id, receiver)) = self.career_pending.take()
            && let Ok(read) = receiver.recv()
        {
            self.take_career(&id, read);
            taken = true;
        }
        if !taken && (self.unlock_due || self.career_file != CareerFile::Read)
            && let (Some(store), Some(id)) = (self.store.clone(), self.active.as_ref().map(|p| p.id.clone()))
        {
            let unlock = std::mem::take(&mut self.unlock_due);
            self.career_stale = false;
            self.take_career(&id, read_career(&store, &id, unlock));
        }
    }

    /// Whether the profile control has the career it shows, and the Profile page, when shown
    /// (`on_page`), what it shows.
    pub(super) fn capture_ready(&self, on_page: bool) -> bool {
        self.active.is_none() || (self.career_file != CareerFile::Unknown && self.career_pending.is_none()
            && (!on_page || self.details.is_some() || self.details_error.is_some() || matches!(self.career_file, CareerFile::Damaged(_))))
    }

    /// The rank's line as the profile control shows it: the rank, or that the career is being
    /// read, or that it cannot be; in its colour.
    pub(super) fn rank_line(&self) -> (String, egui::Color32) {
        match &self.career_file {
            CareerFile::Read => (ta_ui::profiles::rank_text(&self.career), AMBER),
            CareerFile::Unknown => (t("Loading service record…").to_string(), STEEL),
            CareerFile::Damaged(_) => (t("Service record unreadable").to_string(), RED),
        }
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
    /// tutorial): known once their career is read.
    pub(crate) fn newcomer(&self) -> bool {
        self.active.is_some() && self.career_file == CareerFile::Read && self.career.matches_finished == 0
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
        let mut editor = ProfileEditor::new(if existing { self.active.as_ref() } else { None });
        // (The other commanders' names, said to be taken before saving.)
        let own = self.active.as_ref().filter(|_| existing).map(|p| p.id.clone());
        editor.taken = self.list.iter().filter(|p| Some(&p.id) != own.as_ref()).map(|p| p.name.clone()).collect();
        self.editor = Some(editor);
    }
    pub fn data_dir(&self, fallback: PathBuf) -> PathBuf {
        self.store.as_ref().zip(self.active.as_ref()).and_then(|(store, p)| store.data_dir(&p.id).ok()).unwrap_or(fallback)
    }
    pub fn refresh(&mut self) {
        // (What the Profile page shows is read again when it is next shown, and the career with
        // it or on its own: off the interface's thread, never here.)
        self.details = None;
        self.details_pending = None;
        self.details_error = None;
        let Some(store) = &self.store else { return };
        let result = (|| -> std::io::Result<()> {
            if let Some(active) = &mut self.active {
                *active = store.get(&active.id)?;
            }
            Ok(())
        })();
        self.error = result.err().map(|e| e.to_string());
        self.career_stale = self.active.is_some();
        self.refresh_list();
    }

    /// The career to be read again at once (TRY AGAIN, after a fresh one began): nothing known of
    /// it until it is.
    pub(super) fn reread_career(&mut self) {
        self.career_file = CareerFile::Unknown;
        self.career_pending = None;
        self.career_stale = self.active.is_some();
        self.details = None;
        self.details_pending = None;
        self.details_error = None;
    }

    fn refresh_list(&mut self) {
        let Some(store) = &self.store else { return };
        match store.list_available() {
            Ok((list, unreadable)) => {
                self.list = list;
                for profile in &unreadable { log::warn!("profile {} cannot be read: {}", profile.folder.display(), profile.error); }
                self.unreadable = unreadable;
                self.list_error = None;
            }
            Err(error) => {
                // A directory-listing problem must not discard an already validated
                // active identity. The chooser still offers it and reports the issue.
                self.list = self.active.iter().cloned().collect();
                self.unreadable.clear();
                self.list_error = Some(error.to_string());
            }
        }
    }
}

/// A commander's career read (on a thread of its own), with what its play has met unlocked first
/// when `unlock` ([`ProfileStore::career_refreshed`]).
fn read_career(store: &ProfileStore, id: &str, unlock: bool) -> CareerRead {
    let read = if unlock { store.career_refreshed(id) } else { store.career(id).map(|career| (career, Vec::new())) };
    read.map_err(|error| error.to_string())
}

/// A profile store's error, for the editor or a notice: what was refused, in the player's
/// language; else what could not be done (`doing`, a translated sentence with `{error}`) and the
/// file's or the system's own words.
pub(super) fn store_error(error: &std::io::Error, doing: &str) -> String {
    match Problem::of(error) {
        Some(problem) => ta_ui::profiles::problem_text(problem).to_string(),
        None => tf(doing, &[("error", &ta_ui::profiles::first_line(&error.to_string()))]),
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
                self.notify(tf("Could not update profile: {error}", &[("error", &ta_ui::profiles::error_text(&error))]), RED);
            }
        }
        // A normalized or rejected name must also reach disk before an immediate Play,
        // profile switch or Close; do not leave it waiting on the next frame's debounce.
        let _ = self.store.flush(&mut self.config);
    }

    /// A single compact identity control in the launcher's top-right corner. The
    /// full career and profile management live in its menu rather than another toolbar.
    pub(super) fn profile_widget(&mut self, ui: &mut Ui) {
        // (The career comes in off the interface's thread; the Profile page reads it with its record.)
        let page_reads = self.page == super::Page::Profile && self.profiles.active.is_some();
        self.profiles.poll_career(ui.ctx(), page_reads);
        let mut choose = None;
        let blocked = self.profile_blocked();
        let running = blocked.is_some();
        let profile = self.profiles.active.clone();
        let name = profile.as_ref().map_or_else(|| t("CHOOSE PROFILE").to_string(), |p| p.name.clone());
        let (rank, rank_color) = self.profiles.rank_line();
        let read = self.profiles.career_file == CareerFile::Read;
        let damaged = matches!(self.profiles.career_file, CareerFile::Damaged(_));
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
            // (The rank's insignia, before the menu's arrow: once the career is read.)
            let has_rank = profile.is_some() && read;
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
                text.add(egui::Label::new(RichText::new(&rank).font(body(10.5)).color(rank_color)).truncate());
                if read {
                    let bar = Rect::from_min_size(pos2(text_rect.min.x, rect.max.y - 10.0), vec2(text_rect.width(), 5.0));
                    ta_ui::profiles::paint_xp_track(&text, bar, self.profiles.career.rank().progress);
                }
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
                    ui.label(RichText::new(&rank).color(rank_color));
                    if read {
                        let rank = self.profiles.career.rank();
                        ui.label(ta_ui::profiles::progress_text(&rank));
                    }
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
                            if read {
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
                            } else {
                                // (Not a Recruit's empty record: the career is being read, or cannot be.)
                                ui.add(egui::Label::new(RichText::new(&name).font(strong(19.0)).color(TEXT_BRIGHT)).wrap());
                                let line = if damaged { t("Your service record could not be read: no battle starts until it is put right.") } else { rank.as_str() };
                                ui.add(egui::Label::new(RichText::new(line).color(rank_color)).wrap());
                            }
                            if damaged {
                                if ui.add(ta_ui::widgets::ActionButton::primary("PUT IT RIGHT").size(ui.available_width(), 34.0))
                                    .hover_tip(t("The Profile page: start a fresh career, the damaged record kept, or try reading it again")).clicked() {
                                    self.profiles.show_career(); ui.close();
                                }
                            } else {
                                if ui.add(ta_ui::widgets::ActionButton::primary("VIEW PROFILE").size(ui.available_width(), 34.0)).clicked() {
                                    self.profiles.show_career(); ui.close();
                                }
                                if ui.add(ta_ui::widgets::ActionButton::secondary("ACHIEVEMENTS").size(ui.available_width(), 30.0)).clicked() {
                                    self.profiles.show_achievements(); ui.close();
                                }
                            }
                            ui.separator();
                        }
                        ui.label(ta_ui::widgets::caption("PLAYING ON THIS DEVICE"));
                        if let Some(error) = &self.profiles.list_error {
                            ui.add(egui::Label::new(RichText::new(tf("The profiles' folder could not be read: {error}", &[("error", &ta_ui::profiles::first_line(error))])).color(AMBER).size(12.0)).wrap());
                        }
                        // (Profiles that cannot be read: where they are shown, with what to do.)
                        let unreadable = match self.profiles.unreadable.len() {
                            0 => None,
                            1 => Some(t("A profile on this device cannot be read: MANAGE PROFILES shows it, with what to do.").to_string()),
                            count => Some(tf("{count} profiles on this device cannot be read: MANAGE PROFILES shows them, with what to do.", &[("count", &count)])),
                        };
                        if let Some(text) = unreadable {
                            ui.add(egui::Label::new(RichText::new(text).color(AMBER).size(12.0)).wrap());
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
                        if (profile.is_some() || !self.profiles.unreadable.is_empty()) && ui.add(ta_ui::widgets::ActionButton::secondary("MANAGE PROFILES").size(ui.available_width(), 30.0))
                            .hover_tip(t("Every commander on this device: switch, edit or delete them")).clicked() {
                            self.profiles.show_page(ProfileTab::Commanders); ui.close();
                        }
                        if self.profiles.error.is_some() && ui.add(ta_ui::widgets::ActionButton::secondary("PROFILE ERROR").size(ui.available_width(), 30.0)).clicked() {
                            self.profiles.edit(profile.is_some()); ui.close();
                        }
                    });
                });
            });
        if let Some(id) = choose { self.switch_profile(&id); }
    }

    pub(super) fn profile_dialog(&mut self, ctx: &egui::Context) {
        if self.profiles.confirm_delete.is_some() { self.delete_profile_dialog(ctx); return; }
        if self.profiles.confirm_fresh { self.fresh_career_dialog(ctx); return; }
        let game_running = self.game_running();
        let Some(editor) = &mut self.profiles.editor else { return };
        let required = self.profiles.active.is_none() && self.profiles.required;
        let width = (ctx.content_rect().width() - 76.0).clamp(180.0, 460.0);
        let panel = Panel::new(if self.profiles.editing { "EDIT PROFILE" } else { "CREATE PROFILE" }).width(width).max_height((ctx.content_rect().height() - 24.0).max(120.0))
            .exit(if required { Exit::None } else { Exit::Close });
        let response = ta_ui::panel::modal(ctx, "commander_setup", panel, !required, |ui| {
            if let Some(error) = &self.profiles.error {
                ui.add(egui::Label::new(RichText::new(tf("Could not load profile: {error}", &[("error", &ta_ui::profiles::first_line(error))])).color(RED)).wrap());
            }
            ui.add_enabled_ui(!game_running && !self.installer.busy() && !self.skins.busy() && !self.maps.busy(), |ui| editor.show(ui)).inner
        });
        if let Some(draft) = response.inner { self.save_profile(draft); }
        else if response.exit && !required { self.profiles.editor = None; }
    }

    pub(super) fn save_profile(&mut self, draft: ProfileDraft) {
        if let Some(why) = self.profile_blocked() { self.notify(why.to_string(), AMBER); return; }
        // (What the store refuses is said in the player's language; a file out of reach in its own words.)
        let failed = |error: std::io::Error| store_error(&error, n("Could not save the profile: {error}"));
        let result = (|| -> Result<String, String> {
            self.store.flush(&mut self.config)?;
            self.sync_profile_name();
            self.store.flush(&mut self.config)?;
            let store = self.profiles.store.as_ref().ok_or_else(|| self.profiles.error.clone().unwrap_or_default())?;
            if self.profiles.editing {
                let id = self.profiles.active.as_ref().ok_or_else(|| t("Choose a profile first.").to_string())?.id.clone();
                store.update(&id, &draft.name, draft.allegiance).map_err(failed)?;
                store.set_motto(&id, &draft.motto).map_err(failed)?;
                if let Some(bytes) = &draft.avatar_png { store.set_avatar(&id, bytes).map_err(failed)?; }
                else if draft.clear_avatar { store.remove_avatar(&id).map_err(failed)?; }
                Ok(id)
            } else {
                let profile = store.create(&draft.name, draft.allegiance, draft.avatar_png.as_deref(), &self.config).map_err(failed)?;
                if !draft.motto.is_empty() { store.set_motto(&profile.id, &draft.motto).map_err(failed)?; }
                if draft.clear_avatar { store.remove_avatar(&profile.id).map_err(failed)?; }
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
            let path = profiles.config_path(id).map_err(|e| ta_ui::profiles::error_text(&e))?;
            // Prepare the exact values to adopt before changing either identity.
            // Do not reopen after selection: an intervening read failure would
            // silently replace this commander's settings with recovered defaults.
            let (store, config) = ConfigStore::try_open_in(path, self.store.dir()).map_err(|e| ta_ui::profiles::first_line(&e.to_string()))?;
            // (The career is the new commander's own state, read off the interface's thread once
            // they are chosen: one that cannot be read is told of, with how to put it right, and
            // keeps them from no setting, only from battles.)
            let profile = if cfg!(test) { profiles.select(id) } else { profiles.select_and_pin(id) }.map_err(|e| ta_ui::profiles::error_text(&e))?;
            self.profiles.toasts = Default::default();
            self.store = store;
            self.config = config;
            // (Their mods are as they left them: nothing turned on or off.)
            self.followed_mods = crate::supcom::Followed::of(&self.config);
            self.profiles.active = Some(profile);
            // A record is a snapshot of one pinned identity, never shared on handover.
            self.profiles.details = None;
            self.profiles.details_pending = None;
            self.profiles.details_error = None;
            self.profiles.view = CareerView::default();
            self.profiles.begin_career(!cfg!(test));
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

    /// The names the active commander's cannot be: the device's other commanders'.
    pub(crate) fn profile_names_taken(&self) -> Vec<String> {
        let active = self.profiles.active.as_ref().map(|p| p.id.as_str());
        self.profiles.list.iter().filter(|p| Some(p.id.as_str()) != active).map(|p| p.name.clone()).collect()
    }

    /// The active commander's service record cannot be read: no battle starts until it is put
    /// right (the Play page's banner says so; the Profile page offers a fresh career).
    pub(crate) fn career_unreadable(&self) -> bool {
        matches!(self.profiles.career_file, CareerFile::Damaged(_))
    }

    /// The Profile page, where the career is put right.
    pub(crate) fn show_career_problem(&mut self) {
        self.profiles.show_career();
    }

    /// Asks before a damaged career is set aside for a fresh one: it is the player's history.
    fn fresh_career_dialog(&mut self, ctx: &egui::Context) {
        let width = (ctx.content_rect().width() - 80.0).clamp(1.0, 520.0);
        let max_height = (ctx.content_rect().height() - 24.0).max(1.0);
        let folder = self.profiles.store.as_ref().zip(self.profiles.active.as_ref()).and_then(|(s, p)| s.data_dir(&p.id).ok());
        let blocked = self.profile_blocked();
        let mut start = false;
        let response = ta_ui::panel::modal(ctx, "launcher_fresh_career", Panel::new("START A FRESH CAREER?").exit(Exit::Close).width(width).max_height(max_height).accent(RED), true, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.add(egui::Label::new(RichText::new(t("Your service record cannot be read. It is set aside in your profile folder (renamed career.damaged-…, with the pictures of your achievements): nothing is deleted, and it can be put back if it is ever repaired."))
                .color(TEXT)).wrap());
            ui.add(egui::Label::new(RichText::new(t("Your career then starts again from Recruit: rank, XP, achievements and match history begin anew. Your settings, saved games, replays and campaign progress stay as they are."))
                .color(TEXT)).wrap());
            if let Some(folder) = &folder { ui.add(egui::Label::new(RichText::new(folder.display().to_string()).font(mono(12.0)).color(STEEL)).wrap()); }
            if let Some(why) = blocked { ui.add(egui::Label::new(RichText::new(why).color(AMBER).size(12.0)).wrap()); }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                start = ui.add_enabled(blocked.is_none(), ta_ui::widgets::ActionButton::danger("START A FRESH CAREER")).clicked();
                if ui.add(ta_ui::widgets::ActionButton::secondary("CANCEL")).clicked() { self.profiles.confirm_fresh = false; }
            });
        });
        if start {
            self.profiles.confirm_fresh = false;
            self.start_fresh_career();
        } else if response.exit {
            self.profiles.confirm_fresh = false;
        }
    }

    /// The damaged career set aside (kept in the profile's folder) and a fresh one begun.
    pub(super) fn start_fresh_career(&mut self) {
        if let Some(why) = self.profile_blocked() { self.notify(why.to_string(), AMBER); return; }
        let (Some(store), Some(id)) = (self.profiles.store.clone(), self.profiles.active.as_ref().map(|p| p.id.clone())) else { return };
        match store.set_aside_career(&id) {
            Ok(Some(file)) => self.notify(tf("Your damaged service record is kept in your profile folder as {file}: a fresh career begins.", &[("file", &file)]), AMBER),
            Ok(None) => self.notify(t("Your service record can be read again: it is left as it is."), GREEN),
            Err(error) => {
                log::warn!("could not set the career of profile {id} aside: {error}");
                self.notify(tf("Could not set your service record aside: {error}", &[("error", &ta_ui::profiles::error_text(&error))]), RED);
                return;
            }
        }
        self.profiles.reread_career();
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
        assert_eq!(app.last_notice().map(|notice| notice.text.clone()).as_deref(), Some("Could not update profile: Another commander on this device already has this name."),
            "said in the player's words, not the store's");
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

    /// A commander whose career cannot be read is switched to whole (identity, settings), and told
    /// of it: no battle starts (the benchmark, which records none, does), nothing is written to
    /// the file, and a fresh career begins only when asked for, the damaged file kept.
    #[test]
    fn an_unreadable_career_is_told_and_put_right_without_splitting_identities() {
        let (root, mut app, profiles, first, second) = launcher("career_handover");
        let career = profiles.data_dir(&second.id).unwrap().join("career.toml");
        std::fs::write(&career, "version = [broken career").unwrap();
        app.config.audio.music = 0.25;
        let first_settings = app.store.path.clone();
        assert!(app.switch_profile(&second.id), "the career is the commander's own state: it splits no identity");
        assert_eq!(profiles.active().unwrap().unwrap().id, second.id);
        assert_eq!(app.store.path, profiles.config_path(&second.id).unwrap(), "the settings are the new commander's");
        assert_eq!(Config::try_load_from(&first_settings).unwrap().audio.music, 0.25, "the last commander's edits were saved");
        app.profiles.settle_career();
        assert!(app.career_unreadable() && app.profiles.error.is_none());
        assert!(!app.profiles.newcomer(), "not a newcomer's empty record");
        assert_eq!(app.profiles.rank_line().0, "Service record unreadable");
        app.play(&[]);
        assert!(matches!(&app.game, crate::launch::GameState::Failed(m) if m.contains("service record")), "{:?}", app.game);
        app.game = crate::launch::GameState::Idle;
        app.play(&["--benchmark".into(), root.join("report.json").into_os_string()]);
        assert!(!matches!(&app.game, crate::launch::GameState::Failed(m) if m.contains("service record")), "the benchmark records no battle");
        assert_eq!(std::fs::read_to_string(&career).unwrap(), "version = [broken career", "nothing is written to it");
        // (Put right: a fresh career, the damaged file kept beside it; once the new commander's
        // skins are read, as anything done to profiles waits for.)
        app.skins.shutdown();
        app.start_fresh_career();
        app.profiles.settle_career();
        assert!(!app.career_unreadable(), "{:?}", app.last_notice().map(|n| n.text.clone()));
        assert_eq!(app.profiles.career, Career::default());
        let kept: Vec<_> = std::fs::read_dir(career.parent().unwrap()).unwrap().flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("career.damaged-")).collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(std::fs::read_to_string(kept[0].path()).unwrap(), "version = [broken career");
        assert!(app.switch_profile(&first.id));
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
        app.profiles.settle_career();
        let mut reopened = Profiles::open(root.clone(), true);
        reopened.settle_career();
        for state in [&app.profiles, &reopened] {
            assert!(state.error.is_none(), "unrelated metadata must not disable Play for a healthy commander");
            assert_eq!(state.active.as_ref().unwrap().id, first.id);
            assert_eq!(state.career.xp, award.xp, "career still loads after startup and refresh");
            assert_eq!(state.unreadable.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(), [second.id.as_str()]);
            assert!(state.list_error.is_none());
            assert_eq!(state.list.len(), 2);
            assert!(!state.list.iter().any(|profile| profile.id == second.id));
        }
        assert!(app.switch_profile(&third.id), "other healthy commanders remain selectable");
        app.profiles.settle_career();
        assert_eq!(app.profiles.career.xp, 0);
        assert!(app.profiles.error.is_none());
        std::fs::write(&path, original).unwrap();
        app.profiles.refresh();
        assert!(app.profiles.unreadable.is_empty());
        assert_eq!(app.profiles.list.len(), 3);
        app.skins.shutdown();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Another commander's damaged profile.toml blocks no edit (motto, allegiance, a rename), no
    /// new commander, and it can be removed; what the store refuses is said in the player's
    /// words, not its own.
    #[test]
    fn a_damaged_profile_blocks_no_edit_and_is_removed_when_asked() {
        let draft = |name: &str, motto: &str, allegiance| ProfileDraft { name: name.into(), motto: motto.into(), allegiance, avatar_png: None, clear_avatar: false };
        let (root, mut app, profiles, first, second) = launcher("damaged_edit");
        let path = profiles.data_dir(&second.id).unwrap().join("profile.toml");
        std::fs::write(&path, "name = [broken metadata").unwrap();
        app.profiles.refresh();
        assert_eq!(app.profiles.unreadable.iter().map(|u| u.folder.clone()).collect::<Vec<_>>(), [path.parent().unwrap().to_path_buf()]);
        app.profiles.edit(true);
        app.save_profile(draft("First", "Still here", Allegiance::Core));
        assert!(app.profiles.editor.is_none(), "{:?}", app.profiles.editor.as_ref().and_then(|e| e.error.clone()));
        assert_eq!((profiles.get(&first.id).unwrap().motto.as_str(), profiles.get(&first.id).unwrap().allegiance), ("Still here", Allegiance::Core));
        app.skins.shutdown();
        app.profiles.edit(true);
        app.save_profile(draft("Renamed", "Still here", Allegiance::Core));
        assert_eq!(profiles.get(&first.id).unwrap().name, "Renamed");
        app.skins.shutdown();
        app.profiles.edit(false);
        app.save_profile(draft("Third", "", Allegiance::Arm));
        assert!(app.profiles.editor.is_none(), "a new commander: {:?}", app.profiles.editor.as_ref().and_then(|e| e.error.clone()));
        app.skins.shutdown();
        app.profiles.edit(true);
        assert_eq!(app.profiles.editor.as_ref().unwrap().taken, ["Renamed"], "the other commanders' names, said before saving");
        app.save_profile(draft("renamed", "", Allegiance::Arm));
        assert_eq!(app.profiles.editor.as_ref().unwrap().error.as_deref(), Some("Another commander on this device already has this name."));
        app.profiles.editor = None;
        assert_eq!(app.profile_names_taken(), ["Renamed"], "the settings' name says so too");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "name = [broken metadata", "left as it is");
        app.delete_profile(&second.id, "The unreadable profile");
        assert!(!path.exists(), "removed when asked (to the Recycle Bin, outside tests)");
        assert!(app.profiles.unreadable.is_empty());
        app.skins.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// The career is read off the interface's thread: refreshing the profile (a visit to its page,
    /// F5, a game ended) reads none of it there, and the read comes in with the frames after.
    #[test]
    fn the_career_is_never_read_on_the_interfaces_thread() {
        let (root, mut app, profiles, first, _) = launcher("career_thread");
        app.profiles.settle_career();
        assert_eq!(app.profiles.career.kills, 0);
        profiles.award(&first.id, "later", &ta_config::profiles::MatchCounters { kills: 7, ..Default::default() }).unwrap();
        app.profiles.refresh();
        assert_eq!((app.profiles.career.kills, &app.profiles.career_file), (0, &CareerFile::Read), "the career last read stands until the new read comes");
        let ctx = egui::Context::default();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.profiles.career.kills != 7 && std::time::Instant::now() < until {
            app.profiles.poll_career(&ctx, false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(app.profiles.career.kills, 7, "read off the interface's thread");
        // (On the Profile page the record's read brings it: no second read is begun.)
        app.profiles.refresh();
        app.profiles.poll_career(&ctx, true);
        assert!(app.profiles.career_pending.is_none());
        // (A commander switched to: nothing known of their career until it is read.)
        let third = profiles.create("Third", Allegiance::Arm, None, &Config::default()).unwrap();
        profiles.select(&first.id).unwrap();
        app.profiles.refresh();
        assert!(app.switch_profile(&third.id));
        assert_eq!(app.profiles.career_file, CareerFile::Unknown);
        assert!(!app.profiles.newcomer(), "not taken for a newcomer before the career is read");
        app.profiles.settle_career();
        assert!(app.profiles.newcomer());
        app.skins.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A career that cannot be read is said where the commander looks: the profile control, the
    /// Play page's banner (its button no longer said ready), the Profile page with the ways out.
    #[test]
    fn an_unreadable_career_is_said_on_the_banner_the_control_and_the_profile_page() {
        use eframe::App as _;
        fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
            fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
                match shape {
                    egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                    _ => {}
                }
            }
            let mut out = Vec::new();
            for clipped in shapes { collect(&clipped.shape, &mut out); }
            out
        }
        let (root, mut app, profiles, first, _) = launcher("career_said");
        let game = root.join("game");
        std::fs::create_dir_all(&game).unwrap();
        for file in ta_vfs::install::REQUIRED_FILES { std::fs::write(game.join(file), b"").unwrap(); }
        app.detection = crate::detect::detect(Some(&game), true);
        app.config.gameplay.reduced_motion = true;
        std::fs::write(profiles.data_dir(&first.id).unwrap().join("career.toml"), "version = 5\n").unwrap();
        app.profiles.reread_career();
        let ctx = egui::Context::default();
        ta_ui::theme::install(&ctx);
        let mut frame = eframe::Frame::_new_kittest();
        let mut run = |app: &mut LauncherApp| {
            let mut seen = Vec::new();
            for _ in 0..4 {
                let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 720.0))), ..Default::default() };
                let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
                output.textures_delta.clear();
                seen = texts(&output.shapes);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            seen
        };
        app.page = super::super::Page::Play;
        let start = std::time::Instant::now();
        let mut seen = run(&mut app);
        while !app.career_unreadable() && start.elapsed().as_secs() < 20 { seen = run(&mut app); }
        for wanted in ["Service record unreadable", "SERVICE RECORD UNREADABLE", "CAREER NEEDS ATTENTION", "PUT IT RIGHT"] {
            assert!(seen.iter().any(|t| t == wanted), "{wanted:?} not in {seen:?}");
        }
        assert!(!seen.iter().any(|t| t == "READY FOR DEPLOYMENT" || t == "ALL SYSTEMS GO" || t == "Recruit · Level 1"));
        app.profiles.show_career();
        let seen = run(&mut app);
        for wanted in ["YOUR SERVICE RECORD COULD NOT BE READ", "START A FRESH CAREER…", "TRY AGAIN"] {
            assert!(seen.iter().any(|t| t == wanted), "{wanted:?} not in {seen:?}");
        }
        app.skins.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// The Profile page in a short window, or at a large interface scale, scrolls whole: its head
    /// at its most compact, the achievements running on in the page rather than in a scroll of a
    /// few lines of their own.
    #[test]
    fn a_short_profile_page_scrolls_whole_with_its_lists_at_their_own_height() {
        use eframe::App as _;
        fn shown(shape: &egui::Shape, clip: Rect, wanted: &[&str]) -> bool {
            match shape {
                egui::Shape::Text(text) => wanted.contains(&text.galley.text()) && clip.contains_rect(shape.visual_bounding_rect()),
                egui::Shape::Vec(shapes) => shapes.iter().any(|s| shown(s, clip, wanted)),
                _ => false,
            }
        }
        let (root, mut app, profiles, first, _) = launcher("short_profile_page");
        profiles.set_motto(&first.id, "Hold the line").unwrap();
        app.profiles.refresh();
        app.config.gameplay.reduced_motion = true;
        // (Achievements far down the list: past the first twenty.)
        let deep: Vec<&str> = ta_config::achievements::ALL.iter().skip(20).filter(|a| !a.hidden).map(|a| a.name).collect();
        // (The smallest window, a short one, a large interface: the page scrolls whole, its head
        // compact where it is shortest; the launcher's own size as it was.)
        for (points, scale, natural, compact) in [(vec2(940.0, 600.0), 1.0, true, true), (vec2(1120.0, 700.0), 1.0, true, false),
            (vec2(1120.0, 700.0) / 1.5, 1.5, true, true), (vec2(1120.0, 720.0), 1.0, false, false)] {
            app.config.gameplay.ui_scale = scale;
            let ctx = egui::Context::default();
            ta_ui::theme::install(&ctx);
            ctx.set_zoom_factor(scale);
            let mut frame = eframe::Frame::_new_kittest();
            app.profiles.show_page(ProfileTab::Achievements);
            app.capture_scroll = None;
            let mut run = |app: &mut LauncherApp, wanted: &[&str]| {
                let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), points)), ..Default::default() };
                let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
                output.textures_delta.clear();
                output.shapes.iter().any(|s| shown(&s.shape, s.clip_rect, wanted))
            };
            // (egui lays out the frame a zoom changes in at the old size: a few frames first.)
            let start = std::time::Instant::now();
            for n in 0.. {
                run(&mut app, &[]);
                if (n >= 3 && app.profiles.details.is_some()) || start.elapsed().as_secs() > 60 { break; }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(app.profiles.details.is_some());
            assert_eq!(app.profiles.natural, natural, "{points:?} at {scale}: whether the page scrolls whole");
            assert_eq!(run(&mut app, &["\u{201c}Hold the line\u{201d}"]), !compact, "{points:?} at {scale}: whether the head is at its most compact");
            if natural {
                app.capture_scroll = Some(1800.0);
                let mut deep_shown = false;
                for _ in 0..3 { deep_shown = run(&mut app, &deep); }
                assert!(deep_shown, "{points:?} at {scale}: the achievements run on in the page");
            }
        }
        app.skins.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn switching_profiles_discards_open_dossier_and_pending_previous_player_data() {
        let (root, mut app, _, _, second) = launcher("dossier_handover");
        app.profiles.show_career();
        app.profiles.details = Some(Details { record: CareerData { career: Career { xp: 10000, ..Default::default() }, ..Default::default() }, careers: Default::default(), unreadable: Default::default() });
        let (send, receive) = std::sync::mpsc::channel();
        app.profiles.details_pending = Some(receive);
        assert!(app.switch_profile(&second.id));
        assert!(app.profiles.details.is_none() && app.profiles.details_pending.is_none());
        assert!(send.send(Ok(Details { record: CareerData::default(), careers: Default::default(), unreadable: Default::default() })).is_err(), "old reader cannot deliver into the new profile");
        app.profiles.show_career();
        app.profiles.settle_career();
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
