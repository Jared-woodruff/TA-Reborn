//! What happens around the pages each frame: the game ending, installs finishing, files
//! dropped on the window, keyboard shortcuts, the window's size kept, and the dialog asking
//! whether to remove a mod.

use std::path::PathBuf;
use std::time::Duration;

use egui::{Color32, CornerRadius, Id, Key, LayerId, Modifiers, Order, RichText, Stroke};
use ta_ui::i18n::{t, tf};
use ta_ui::panel::{Exit, Panel};

use super::{Action, LauncherApp, Page};
use crate::battles::Battles;
use crate::getmods::Stage;
use crate::update::Step;

/// What a downloaded update waits for before Setup runs ([`LauncherApp::update_waits_for`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UpdateWait {
    /// A game is running: Setup replaces its files.
    Game,
    /// A skin's work, which closing the launcher would stop.
    SkinWork,
    /// A mod's install, likewise.
    Install,
    /// The map library's work (its status).
    Maps(&'static str),
    /// The profile being edited, which closing would lose.
    Profile,
    /// The skin editor's unsaved changes, likewise.
    SkinEdits,
}

impl UpdateWait {
    /// Why UPDATE NOW or INSTALL NOW cannot be pressed now.
    pub(crate) fn why(self) -> &'static str {
        match self {
            UpdateWait::Game => t("Close the game first: the update replaces its files"),
            UpdateWait::SkinWork => t("Wait for the current skin operation to finish, then try again."),
            UpdateWait::Install => t("Wait for the mod being installed"),
            UpdateWait::Maps(status) => t(status),
            UpdateWait::Profile | UpdateWait::SkinEdits => self.queued(),
        }
    }

    /// What an update asked for waits for, over its progress bar.
    pub(crate) fn queued(self) -> &'static str {
        match self {
            UpdateWait::Game => t("READY · IT INSTALLS WHEN THE GAME CLOSES"),
            UpdateWait::SkinWork => t("READY · IT INSTALLS AFTER THE SKIN WORK"),
            UpdateWait::Install => t("READY · IT INSTALLS AFTER THE MOD"),
            UpdateWait::Maps(status) => t(status),
            UpdateWait::Profile => t("READY · IT INSTALLS WHEN THE PROFILE IS CLOSED"),
            UpdateWait::SkinEdits => t("READY · IT INSTALLS WHEN THE SKIN IS SAVED"),
        }
    }
}
use crate::modlist;
use crate::theme::*;
use crate::widgets::ActionButton;

impl LauncherApp {
    /// The game has ended: the settings it may have changed (its options) taken in rather than
    /// overwritten, the launcher back up if it went down for the game, and the player's
    /// battles looked at again (a new save, a replay, a photo).
    pub(super) fn game_ended(&mut self, ctx: &egui::Context) {
        self.after_launch = None;
        self.launching = None;
        if let Err(error) = self.store.reload_after_game(&mut self.config) {
            self.notify(error, RED);
        }
        if std::mem::take(&mut self.minimised_for_game) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        let language = std::env::var("TA_LANG").unwrap_or_else(|_| self.config.gameplay.language.clone());
        if let Err(error) = ta_ui::i18n::set_language(&language, &self.language_dirs) {
            log::warn!("{error}");
        }
        self.rescan();
        // (A game changes no mod but the map editor's own, which it saves as a new one or
        // over its old self: only what changed is checked again.)
        self.refresh_mods();
        self.rescan_skins();
        // (The battle just fought counts in the maps' order by when they were last played.)
        self.maps.forget_history();
        self.profiles.refresh();
        self.sync_profile_name();
        self.battles = Battles::scan(&self.profiles.data_dir(self.store.dir()), ta_config::pictures_dir());
        // A benchmark's report: shown; none: why.
        let game = &self.game;
        let why_not = || match game {
            crate::launch::GameState::Exited { crash: Some(_), .. } => ta_ui::i18n::t("The game crashed during the benchmark: the Play page has its crash report.").to_string(),
            crate::launch::GameState::Exited { success: false, code: Some(code), .. } => ta_ui::i18n::tf("The game stopped during the benchmark (exit code {code}).", &[("code", code)]),
            crate::launch::GameState::Failed(message) => message.clone(),
            _ => ta_ui::i18n::t("The benchmark was stopped before its end.").to_string(),
        };
        if self.benchmarks.running.is_some() {
            if self.benchmarks.finished(why_not) {
                // (Its game ended as it should: nothing for the Play page to say.)
                self.game = crate::launch::GameState::Idle;
            }
            self.page = Page::Benchmark;
        }
    }

    /// What keeps UPDATE NOW and INSTALL NOW from being pressed: Setup replaces the game's files,
    /// and closing the launcher for it would stop a skin's work, a mod's install or the map
    /// library's work.
    pub(crate) fn update_blocked(&self) -> Option<UpdateWait> {
        if self.game_running() { Some(UpdateWait::Game) }
        else if self.skins.busy() { Some(UpdateWait::SkinWork) }
        else if self.installer.busy() { Some(UpdateWait::Install) }
        else { self.maps.status().map(UpdateWait::Maps) }
    }

    /// What an update asked for waits for before Setup runs: what keeps it from being asked
    /// for ([`Self::update_blocked`]), then what closing the launcher would lose (the profile
    /// being edited, a skin's unsaved changes). Its progress bar says which.
    pub(crate) fn update_waits_for(&self) -> Option<UpdateWait> {
        self.update_blocked().or_else(|| {
            if self.profiles.dialog_open() { Some(UpdateWait::Profile) }
            else if self.skins.editor.as_ref().is_some_and(|e| e.changed()) { Some(UpdateWait::SkinEdits) }
            else { None }
        })
    }

    /// The new version on its way: its news taken in, and once its setup is checked and was
    /// asked for, the setup run and the launcher closed for it (not while the game runs or a mod
    /// is being installed: it waits for them, [`Self::update_waits_for`]).
    pub(super) fn updates(&mut self, ctx: &egui::Context) {
        if self.autoupdate && matches!(self.update, crate::update::UpdateState::Available(_)) {
            self.autoupdate = false;
            self.start_update();
        }
        let waits_for = self.update_waits_for();
        let Some(download) = &mut self.download else { return };
        if download.poll() {
            ctx.request_repaint();
        }
        match &download.step {
            Step::Downloading { .. } | Step::Checking => ctx.request_repaint_after(Duration::from_millis(150)),
            Step::Ready(setup) if self.update_go && waits_for.is_none() => {
                self.update_go = false;
                // Setup closes/replaces this process. Do not rely on on_exit,
                // where a failed save can no longer leave the player a Retry action.
                if let Err(error) = self.store.flush(&mut self.config) {
                    self.update_error = Some(error);
                    return;
                }
                let here = crate::launch::app_dir().unwrap_or_default();
                match crate::update::run_setup(setup, &here) {
                    Ok(()) => {
                        download.retain_for_setup();
                        log::info!("updating to {}: {} runs, the launcher closes for it", download.version, setup.display());
                        // (Closing for the setup: nothing to ask about.)
                        self.close_confirmed = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    Err(e) => {
                        log::warn!("the update did not start: {e}");
                        self.update_error = Some(e);
                    }
                }
            }
            _ => {}
        }
    }

    /// The window's size as the player leaves it (not while minimised or full screen).
    pub(super) fn keep_window(&mut self, ctx: &egui::Context) {
        let Some((size, maximized)) = &mut self.window else { return };
        // Viewport rectangles are in zoomed egui points. Persist the window's native logical
        // size, or an enlarged interface makes the window shrink again on each launch.
        let zoom = ctx.zoom_factor();
        ctx.input(|i| {
            let v = i.viewport();
            if v.minimized == Some(true) || v.fullscreen == Some(true) {
                return;
            }
            *maximized = v.maximized.unwrap_or(false);
            if let Some(inner) = v.inner_rect.filter(|_| !*maximized) {
                *size = [(inner.width() * zoom).round(), (inner.height() * zoom).round()];
            }
        });
    }

    /// Whatever the page: the installs' news taken in, and those finished found (the mods
    /// rescanned), a catalogue mod turned on, and celebrated.
    pub(super) fn installs(&mut self, ctx: &egui::Context) {
        if self.installer.poll() {
            ctx.request_repaint();
        }
        if self.installer.busy() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        let now = ctx.input(|i| i.time);
        for key in std::mem::take(&mut self.installer.failed) {
            // (On the Mods page the failure is on its card or in its row.)
            if self.page != Page::Mods && let Some(Stage::Failed(why)) = self.installer.stages.get(&key) {
                let name = crate::catalogue::get(&key).map(|entry| entry.name.to_string()).or_else(|| self.installer.packages.get(&key).cloned()).unwrap_or_else(|| key.clone());
                self.notify(tf("Could not install {name}: {error}", &[("name", &name), ("error", why)]), RED);
            }
        }
        for key in std::mem::take(&mut self.installer.finished) {
            // (Installed when it was asked for: an update.)
            let updated = self.updates_requested.remove(&key);
            self.refresh_mods();
            let (units, maps, id) = match self.installer.stages.get(&key) {
                Some(Stage::Done { units, maps, id }) => (*units, *maps, id.clone()),
                _ => (0, 0, key.clone()),
            };
            match crate::catalogue::get(&key).filter(|_| !self.installer.packages.contains_key(&key)) {
                // An update keeps the mod on or off as it was (the next game is the one chosen).
                Some(entry) if updated => {
                    self.notify(tf("{name} is updated: {units} units, {maps} maps", &[("name", &entry.name), ("units", &units), ("maps", &maps)]), GREEN);
                }
                Some(entry) => {
                    let before = self.config.mods.enabled.clone();
                    modlist::turn_on_conversion(&mut self.config.mods.enabled, entry.id);
                    let off: Vec<String> = before.iter().filter(|id| !self.config.mods.enabled.contains(id))
                        .map(|id| self.mods.get(id).map_or_else(|| id.clone(), |m| m.info.manifest.name.clone())).collect();
                    if off.is_empty() {
                        self.notify(tf("{name} is installed and on: {units} units, {maps} maps", &[("name", &entry.name), ("units", &units), ("maps", &maps)]), GREEN);
                    } else {
                        self.notify(tf("{name} is installed and on in place of {others}: {units} units, {maps} maps", &[("name", &entry.name), ("others", &off.join(", ")), ("units", &units), ("maps", &maps)]), GREEN);
                    }
                }
                None => {
                    let name = self.mods.get(&id).map(|m| m.info.manifest.name.clone()).or_else(|| self.installer.packages.get(&key).cloned()).unwrap_or_else(|| id.clone());
                    // Classic archives are kept intact; their contents are read by the game.
                    let classic = self.mods.get(&id).is_some_and(|m| m.info.path.is_file());
                    let message = if classic {
                        tf("{name} is installed: turn it on to play with it", &[("name", &name)])
                    } else {
                        tf("{name} is installed: {units} units, {maps} maps. Turn it on to play with it.", &[("name", &name), ("units", &units), ("maps", &maps)])
                    };
                    self.notify(message, GREEN);
                }
            }
            self.celebrate = Some((id, now));
        }
    }

    /// Files dropped on the window: a saved game is continued, a replay watched, the game's
    /// folder used as the game's, and anything else installed as a mod.
    pub(super) fn dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if self.file_drop_blocked(ctx) {
            if !dropped.is_empty() {
                self.notify("File drops are unavailable while a dialog or key binding is active.", AMBER);
            }
            return;
        }
        for path in dropped {
            log::info!("dropped on the window: {}", path.display());
            let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
            match ext.as_str() {
                "tasav" => self.push(Action::PlayWith(vec!["--load".into(), path.into_os_string()])),
                "tarep" => self.push(Action::PlayWith(vec!["--replay".into(), path.into_os_string()])),
                "taskin" => self.push(Action::ImportSkin(path)),
                // The game's folder (or one of its own files' folder: never installed as a mod).
                _ if path.is_dir() && ta_vfs::install::REQUIRED_FILES.iter().any(|f| ta_vfs::install::find_file_ci(&path, f).is_some()) => {
                    self.page = Page::Play;
                    self.use_install_folder(&path);
                }
                _ if path.is_dir() && matches!(crate::detect::validate_picked(&path), crate::detect::Picked::Valid { .. }) => {
                    self.page = Page::Play;
                    self.use_install_folder(&path);
                }
                _ if self.page == Page::Maps => self.push(Action::Maps(crate::maps::Action::Import(path))),
                _ => self.push(Action::InstallPackage(path)),
            }
        }
    }

    pub(crate) fn dialog_open(&self) -> bool {
        self.profiles.dialog_open() || self.show_shortcuts || self.whats_new.is_some()
            || self.confirm_uninstall.is_some() || self.confirm_replace.is_some() || self.mod_validation.selected.is_some() || self.conversion_prompt.is_some() || self.battle_prompt.is_some() || self.show_mod_folders
            || self.confirm_close || self.confirm_end_game || self.show_progress
            || self.skins.ask.is_some()
    }

    pub(crate) fn modal_open(&self, ctx: &egui::Context) -> bool {
        self.dialog_open() || egui::Popup::is_any_open(ctx)
    }

    fn file_drop_blocked(&self, ctx: &egui::Context) -> bool {
        self.modal_open(ctx) || (self.page == Page::Settings && self.settings_view.capturing())
    }

    /// A gamepad's input for this frame ([`crate::pad`]): its moves as egui's events, its bumpers'
    /// pages for [`Self::shortcuts`], the system's pointer where its right stick puts it. The pads
    /// are read while the settings have them on, never in a capture, a film or a test; their
    /// input is taken while the window has the focus and no game is running.
    pub(super) fn pad_input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let wanted = self.config.gameplay.gamepad && self.capture.is_none() && self.film.is_none() && !cfg!(test);
        if wanted != self.pad.is_some() {
            self.pad = wanted.then(|| crate::pad::Pad::start(ctx));
        }
        let game_running = self.game_running();
        let Some(pad) = &mut self.pad else { return };
        pad.listen(raw.focused && !game_running);
        let readings = pad.take();
        let moved = crate::pad::mouse_moved(&raw.events);
        let mouse = moved.or_else(|| ctx.input(|i| i.pointer.latest_pos()));
        let screen = raw.screen_rect.map_or_else(|| ctx.content_rect().size(), |r| r.size());
        let time = raw.time.unwrap_or_else(|| ctx.input(|i| i.time));
        let focused = ctx.memory(|m| m.focused());
        let frame = pad.state.frame(&readings, time, mouse, moved, screen, focused, ctx.text_edit_focused(), self.config.gameplay.gamepad_deadzone, self.config.gameplay.gamepad_sensitivity);
        raw.events.extend(frame.events);
        self.pad_pages += frame.pages;
        if let Some(at) = frame.cursor {
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorPosition(at));
        }
        if frame.busy {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    /// Keys: Ctrl+1 to Ctrl+8 the rail's pages in order, Enter plays (on the Play page), F5 looks
    /// again (the game, mods, skins, maps or profile of the page shown). Not while typing, while a dialog
    /// is open, a key is awaited for an action (Settings, Controls) or a control has the
    /// keyboard's focus.
    pub(super) fn shortcuts(&mut self, ctx: &egui::Context) {
        let setting_a_key = self.page == Page::Settings && self.settings_view.capturing();
        // (A pad's bumpers, here as Ctrl+1 to Ctrl+8 are: not under a dialog.)
        let pad_pages = std::mem::take(&mut self.pad_pages);
        if self.modal_open(ctx) || setting_a_key || self.film.is_some() {
            return;
        }
        if pad_pages != 0 {
            let at = super::PAGES.iter().position(|&p| p == self.page).unwrap_or(0) as i32;
            let page = super::PAGES[(at + pad_pages).clamp(0, super::PAGES.len() as i32 - 1) as usize];
            if page != self.page {
                ta_ui::sound::play(ctx, ta_ui::sound::UiSound::Tab);
                self.open_page(page);
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F1)) {
            self.show_shortcuts = true;
            return;
        }
        // Ctrl+1 to Ctrl+8: the rail's pages in order, typing in a box or not (Ctrl and a
        // number type nothing).
        let page = ctx.input_mut(|i| [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8].iter().position(|&k| i.consume_key(Modifiers::COMMAND, k)));
        if let Some(page) = page.map(|n| super::PAGES[n]).filter(|&p| p != self.page) {
            ta_ui::sound::play(ctx, ta_ui::sound::UiSound::Tab);
            self.open_page(page);
        }
        if ctx.egui_wants_keyboard_input() { return; }
        let focused = ctx.memory(|m| m.focused().is_some());
        let (enter, f5) = ctx.input_mut(|i| {
            let enter = !focused && i.consume_key(Modifiers::NONE, Key::Enter);
            (enter, i.consume_key(Modifiers::NONE, Key::F5))
        });
        if enter && self.page == Page::Play && self.detection.install().is_some() && !self.game_running() {
            ta_ui::sound::play(ctx, ta_ui::sound::UiSound::BigButton);
            self.push(Action::Play);
        }
        if f5 {
            match self.page {
                Page::Play => self.push(Action::Rescan),
                Page::Mods => self.push(Action::RescanMods),
                Page::Skins => self.push(Action::RescanSkins),
                Page::Maps => self.push(Action::Maps(crate::maps::Action::Rescan)),
                Page::Profile => self.refresh_profile(),
                _ => {}
            }
        }
    }

    /// The question before a mod is removed.
    pub(super) fn uninstall_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.confirm_uninstall.clone() else { return };
        let Some(entry) = self.mods.get(&id) else {
            self.confirm_uninstall = None;
            return;
        };
        let (name, path) = (entry.info.manifest.name.clone(), entry.info.path.clone());
        // (A catalogue mod downloaded by hand has no GET button to install it again.)
        let catalogued = crate::catalogue::get(&id).is_some_and(|entry| entry.file.is_some_and(|file| file.can_download()));
        let title = tf("REMOVE {name}?", &[("name", &name.to_uppercase())]);
        let mut remove = false;
        let response = ta_ui::panel::modal(ctx, "launcher_uninstall", Panel::new(&title).exit(Exit::Close).width((ctx.content_rect().width() - 80.0).clamp(1.0, 560.0)).max_height((ctx.content_rect().height() - 24.0).max(1.0)).accent(RED), true, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let text = if catalogued {
                t("It goes to the Recycle Bin and out of the load order. Its GET button installs it again.")
            } else {
                t("It goes to the Recycle Bin and out of the load order: put it back from there if you change your mind.")
            };
            ui.add(egui::Label::new(RichText::new(text).color(TEXT)).wrap());
            ui.add(egui::Label::new(RichText::new(path.display().to_string()).font(mono(12.0)).color(STEEL)).wrap());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                remove = ui.add_enabled(!self.game_running() && !self.installer.busy(), ActionButton::danger("REMOVE")).clicked();
                if ui.add(ActionButton::secondary("KEEP")).clicked() {
                    self.confirm_uninstall = None;
                }
            });
        });
        if remove {
            self.confirm_uninstall = None;
            self.uninstall(&id);
        } else if response.exit {
            self.confirm_uninstall = None;
        }
    }

    /// The question before a package the player brought takes the place of the installed mod of
    /// its id.
    pub(super) fn replace_dialog(&mut self, ctx: &egui::Context) {
        let Some(source) = self.confirm_replace.clone() else { return };
        let Some(entry) = self.mods.get(source.id()) else {
            // (No longer installed: nothing in its way.)
            self.confirm_replace = None;
            self.installer.start(source);
            return;
        };
        let (name, version, path) = (entry.info.manifest.name.clone(), entry.info.manifest.version.clone(), entry.info.path.clone());
        let version = if version.trim().is_empty() || version == "unverified" { t("Unknown").to_string() } else { version };
        let file = match &source {
            crate::getmods::Source::Package { path, .. } | crate::getmods::Source::Classic { path, .. } => path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()),
            _ => String::new(),
        };
        let title = tf("REPLACE {name}?", &[("name", &name.to_uppercase())]);
        let mut replace = false;
        let response = ta_ui::panel::modal(ctx, "launcher_replace", Panel::new(&title).exit(Exit::Close).width((ctx.content_rect().width() - 80.0).clamp(1.0, 560.0)).max_height((ctx.content_rect().height() - 24.0).max(1.0)).accent(AMBER), true, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let text = tf("{file} would take the place of the {name} installed now (version {version}): its files would be removed.", &[("file", &file), ("name", &name), ("version", &version)]);
            ui.add(egui::Label::new(RichText::new(text).color(TEXT)).wrap());
            ui.add(egui::Label::new(RichText::new(path.display().to_string()).font(mono(12.0)).color(STEEL)).wrap());
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                replace = ui.add_enabled(!self.game_running() && !self.installer.busy(), ActionButton::danger("REPLACE")).clicked();
                if ui.add(ActionButton::secondary("KEEP")).clicked() {
                    self.confirm_replace = None;
                }
            });
        });
        if replace {
            self.confirm_replace = None;
            if !self.game_holds_mods(Some(source.id())) {
                self.installer.start(source);
            }
        } else if response.exit {
            self.confirm_replace = None;
        }
    }

    /// What is new in a version: the changelog's section for it, in a dialog that scrolls.
    pub(super) fn whats_new_dialog(&mut self, ctx: &egui::Context) {
        let Some((version, notes)) = &self.whats_new else { return };
        let title = tf("WHAT'S NEW IN {version}", &[("version", version)]);
        let height = (ctx.content_rect().height() - 24.0).clamp(1.0, 860.0);
        let width = (ctx.content_rect().width() - 80.0).clamp(1.0, 820.0);
        let response = ta_ui::panel::modal(ctx, "launcher_whats_new", Panel::new(&title).exit(Exit::Close).width(width).max_height(height).accent(SKY), true, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            crate::news::show(ui, notes);
        });
        if response.exit {
            self.whats_new = None;
        }
    }

    /// Files held over the window: what dropping them does, on a plate over everything (the
    /// window behind it dimmed, a dashed brass rim breathing round the edge).
    pub(super) fn paint_drop_target(&self, ctx: &egui::Context) {
        if self.file_drop_blocked(ctx) { return; }
        let hovered = ctx.input(|i| i.raw.hovered_files.len());
        if hovered == 0 && !self.show_drop {
            return;
        }
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("launcher_drop_target")));
        painter.rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(225));
        let rim = screen.shrink(12.0);
        let time = ctx.input(|i| i.time) as f32;
        let still = ta_ui::motion::still(ctx) || self.config.gameplay.reduced_flashes;
        let glow = if still { 1.0 } else { 0.7 + 0.3 * (time * 4.0).sin() };
        if !still { ctx.request_repaint(); }
        // The rim: dashes along each side, brass, breathing.
        let dash = Stroke::new(2.0, AMBER.gamma_multiply(glow));
        let step = 22.0;
        let mut x = rim.min.x;
        while x < rim.max.x {
            for y in [rim.min.y, rim.max.y] {
                painter.hline(x..=(x + 12.0).min(rim.max.x), y, dash);
            }
            x += step;
        }
        let mut y = rim.min.y;
        while y < rim.max.y {
            for x in [rim.min.x, rim.max.x] {
                painter.vline(x, y..=(y + 12.0).min(rim.max.y), dash);
            }
            y += step;
        }
        // The plate in the middle: a lamp, the words, what each kind of file does.
        let archives = if self.page == Page::Maps {
            t("A map archive (.ufo .hpi .zip .7z .rar): its maps imported as a map pack")
        } else {
            t("A mod's archive (.zip .rar .7z), a TA archive (.ufo .hpi .gp3) or its folder: installed")
        };
        let lines = [
            t("A skin pack (.taskin): checked and added to your skin library"),
            archives,
            t("A saved game or a replay: the game opens it"),
            t("Your Total Annihilation folder: the launcher uses it"),
        ];
        // Keep the backdrop and instructions in one ordered paint layer so the dimmer
        // cannot be sorted above the text on subsequent frames.
        egui::Area::new(Id::new("launcher_drop_target"))
            .order(Order::Foreground).anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                Panel::new(t(if self.page == Page::Maps { "DROP TO IMPORT" } else { "DROP TO INSTALL" }))
                    .width((screen.width() - 96.0).clamp(1.0, 780.0))
                    .max_height((screen.height() - 40.0).max(1.0))
                    .show(ui, |ui| {
                        for line in lines {
                            ui.add(egui::Label::new(RichText::new(line).font(body(14.0)).color(TEXT)).wrap());
                            ui.add_space(6.0);
                        }
                    });
            });
    }
}
