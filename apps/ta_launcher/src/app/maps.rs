//! Map library actions use the same pinned profile/config launch path as Play.
use super::{LauncherApp, Page};
use crate::maps::{Action, Request};
use crate::theme::*;

impl LauncherApp {
    /// Reads may overlap other library work; publishing a map pack (or the Supreme Commander
    /// factions' mod) must finish before the same shared mod folder can be replaced or removed
    /// through the Mods page: an import would install it again, and turn it on.
    pub(crate) fn map_import_blocks_mod_changes(&mut self) -> bool {
        if self.supcom.importing() {
            self.notify("Wait for the Supreme Commander maps being made, then try again.", AMBER);
            return true;
        }
        if self.supcom.factions.importing() {
            self.notify("Wait for the Supreme Commander factions being made, then try again.", AMBER);
            return true;
        }
        if self.supcom.music.importing() {
            self.notify("Wait for the Supreme Commander music being made, then try again.", AMBER);
            return true;
        }
        if !self.maps.importing() { return false; }
        self.notify("Checking and importing maps…", AMBER);
        true
    }

    /// F5 or RESCAN: the sources are looked through again, whatever is known of them.
    pub(crate) fn rescan_maps(&mut self) { self.maps.needs_scan = true; }

    /// Coming to the page: mods and map packs dropped into a folder since are picked up. The
    /// maps are looked through again only if their sources changed (or on F5).
    pub(crate) fn visit_maps(&mut self) {
        let found = crate::modlist::ModList::scan(super::mod_folders(&self.config, &self.store), &self.config.mods.enabled);
        let same = found.mods.len() == self.mods.mods.len() && found.mods.iter().zip(&self.mods.mods)
            .all(|(a, b)| a.info == b.info);
        if !same { self.discover_mods(); }
    }

    pub(super) fn maps_tick(&mut self, ctx: &egui::Context) {
        if self.maps.cache.is_none() { self.maps.cache = Some(crate::library::cache_dir(&self.store.dir(), "maps")); }
        self.maps.poll(ctx);
        while let Some(id) = self.maps.take_imported() {
            crate::modlist::set_enabled(&mut self.config.mods.enabled, &id, true);
            self.refresh_mods();
            self.maps.needs_scan = true;
        }
        if let Some((message, error)) = self.maps.take_notice() && self.page != Page::Maps {
            self.notify(message, if error { RED } else { GREEN });
        }
        if self.page != Page::Maps { return; }
        self.maps.want_details();
        self.maps.load_history(self.profiles.store.clone(), self.profiles.active.as_ref().map(|profile| profile.id.clone()));
        // (Not while an install may be replacing a mod's archives: read once it is done.)
        if self.maps.busy() || self.installer.busy() { return; }
        let game_dir = self.detection.install().map(|install| install.root.clone());
        let key = format!("{:?}|{:?}|{:?}|{}", game_dir, self.config.mods.enabled,
            self.mods.mods.iter().map(|entry| (&entry.info.path, &entry.info.kind, &entry.info.manifest.id,
                &entry.info.manifest.version, &entry.info.manifest.depends, &entry.info.manifest.load_after,
                &entry.info.manifest.rename)).collect::<Vec<_>>(), ta_ui::i18n::generation());
        if self.maps.loaded_for.as_ref() != Some(&key) || self.maps.needs_scan {
            self.maps.loaded_for = Some(key);
            self.maps.scan(Request { game_dir, builtin: ta_vfs::builtin_dir(),
                mods: self.mods.mods.iter().map(|entry| entry.info.clone()).collect(), enabled: self.config.mods.enabled.clone() });
        }
    }

    pub(super) fn maps_action(&mut self, action: Action, frame: &eframe::Frame) {
        let (edit, skirmish) = (matches!(&action, Action::Edit(_)), matches!(&action, Action::Skirmish(_)));
        match action {
            Action::Rescan => { self.refresh_mods(); self.rescan_maps(); }
            Action::Browse => {
                if self.maps.busy() || self.installer.busy() || self.game_running() { return; }
                if let Some(path) = rfd::FileDialog::new().set_title(ta_ui::i18n::t("Import map archive"))
                    .add_filter(ta_ui::i18n::t("Map archives"), &["ufo", "hpi", "ccx", "gp3", "gpf", "zip", "7z", "rar"]).set_parent(frame).pick_file() {
                    self.maps_action(Action::Import(path), frame);
                }
            }
            Action::Import(path) => {
                if self.game_running() {
                    self.notify("Close the game first: maps cannot be imported while it runs", AMBER);
                    return;
                }
                if self.installer.busy() {
                    self.notify("Wait for the mod being installed, then import the maps again", AMBER);
                    return;
                }
                self.maps.import(path, self.store.dir().join("mods"));
            }
            Action::Toggle(key) => {
                if self.game_running() || self.maps.busy() { return; }
                if let Some(entry) = self.maps.catalogue.entries.iter().find(|entry| entry.key == key)
                    && entry.active && !entry.shadowed && !entry.campaign && entry.players >= 2 {
                    let enabled = !self.config.maps.is_enabled(&entry.layer, &entry.stem);
                    self.config.maps.set_enabled(&entry.layer, &entry.stem, enabled);
                }
            }
            Action::Favourite(key) => {
                if let Some(entry) = self.maps.catalogue.entries.iter().find(|entry| entry.key == key) {
                    let favourite = !self.config.maps.is_favourite(&entry.stem);
                    self.config.maps.set_favourite(&entry.stem, favourite);
                }
            }
            Action::Edit(key) | Action::Play(key) | Action::Skirmish(key) => {
                let Some(entry) = self.maps.catalogue.entries.iter().find(|entry| entry.key == key) else { return; };
                // (A map whose terrain could not be read is neither played nor edited.)
                if !entry.active || entry.shadowed || entry.unreadable.is_some() || !edit && !entry.available(&self.config.maps) || self.maps.busy() { return; }
                let stem = entry.stem.clone();
                let option = if edit { "--editor" } else if skirmish { "--skirmish-map" } else { "--map" };
                self.play(&[option.into(), stem.into()]);
            }
            Action::New => {
                if !self.maps.busy() { self.play(&["--editor".into()]); }
            }
            Action::Random => self.maps.random_choice(&self.config.maps),
            Action::ManageSource(layer) => {
                if self.mods.get(&layer).is_none() { return; }
                self.page = Page::Mods;
                self.mod_library.query.clear();
                self.mod_library.filter = crate::mod_library::Filter::All;
                self.show_mod_details(&layer);
            }
            Action::RemovePack(layer) => {
                if layer.starts_with("map-pack-") { self.actions.push(super::Action::Uninstall(layer)); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use ta_config::profiles::{Allegiance, ProfileStore};

    /// While Supreme Commander's maps or factions are made, the commander is not switched: what
    /// they make is turned on for the one who asked for it. Once they are done, it is.
    #[test]
    fn a_supreme_commander_import_keeps_its_commander() {
        let root = std::env::temp_dir().join(format!("ta-supcom-profile-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let profiles = ProfileStore::open(root.clone()).unwrap();
        let first = profiles.create("First", Allegiance::Arm, None, &Default::default()).unwrap();
        let second = profiles.create("Second", Allegiance::Core, None, &Default::default()).unwrap();
        profiles.select(&first.id).unwrap();
        let ctx = egui::Context::default();
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", root.join("config.toml").to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        app.supcom.factions.pretend_importing();
        assert!(!app.switch_profile(&second.id), "the factions are turned on for the commander who asked");
        app.supcom.factions.stop();
        while app.supcom.factions.importing() && std::time::Instant::now() < deadline {
            app.supcom.factions.poll(&ctx, false);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        app.supcom.pretend_importing(&[]);
        assert!(!app.switch_profile(&second.id), "nor while the maps are made");
        app.supcom.stop();
        while app.supcom.importing() && std::time::Instant::now() < deadline {
            app.supcom.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        while app.maps.busy() && std::time::Instant::now() < deadline {
            app.maps_tick(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(app.switch_profile(&second.id), "done, the commander is switched");
        app.skins.shutdown(); drop(app); drop(profiles);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A copy of the factions in TA Reborn's program folder comes before the mods folder, where an
    /// import makes them: none is made behind it.
    #[test]
    fn the_factions_are_not_made_behind_the_program_folders_copy() {
        let root = std::env::temp_dir().join(format!("ta-supcom-shadow-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let game = root.join("program").join(crate::supcom_factions::MOD_ID);
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("mod.toml"), format!("id = \"{}\"\nname = \"Supreme Commander factions\"\nversion = \"1\"\n", crate::supcom_factions::MOD_ID)).unwrap();
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", root.join("config.toml").to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(egui::Context::default()), args);
        app.skins.shutdown();
        assert!(app.factions_blocked().is_none());
        let folders = vec![crate::modlist::ModFolder { label: crate::modlist::APP_FOLDER_LABEL, path: root.join("program"), exists: true }];
        app.mods = crate::modlist::ModList::scan(folders, &[]);
        assert!(app.factions_blocked().is_some_and(|why| why.contains("program folder")), "{:?}", app.factions_blocked());
        app.skins.shutdown(); drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn imported_map_pack_is_enabled_and_persisted_only_for_the_pinned_profile() {
        let root = std::env::temp_dir().join(format!("ta-map-profile-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let profiles = ProfileStore::open(root.clone()).unwrap();
        let first = profiles.create("First", Allegiance::Arm, None, &Default::default()).unwrap();
        let second = profiles.create("Second", Allegiance::Core, None, &Default::default()).unwrap();
        profiles.select(&first.id).unwrap();
        let input = root.join("my-map.ufo");
        crate::maps::tests::write_map_archive(&input);
        let input_before = std::fs::read(&input).unwrap();
        let ctx = egui::Context::default();
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", root.join("config.toml").to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        assert_eq!(app.profiles.active.as_ref().unwrap().id, first.id);
        app.maps.scan(Request { game_dir: None, builtin: None, mods: Vec::new(), enabled: Vec::new() });
        assert!(app.maps.busy(), "a dropped archive may arrive while initial map discovery is active");
        app.maps_action(Action::Import(input.clone()), &eframe::Frame::_new_kittest());
        assert!(app.maps.importing());
        assert!(!app.switch_profile(&second.id), "profile handover cannot redirect a pending import");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.maps.busy() && std::time::Instant::now() < deadline {
            app.maps_tick(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!app.maps.busy());
        let id = app.config.mods.enabled.iter().find(|id| id.starts_with("map-pack-")).expect("worker completion enables the imported pack").clone();
        assert!(app.mods.get(&id).is_some(), "the published pack is discovered immediately");
        app.config.maps.set_enabled(&id, "moon", false);
        app.store.flush(&mut app.config).unwrap();
        let saved = ta_config::Config::load_from(&profiles.config_path(&first.id).unwrap());
        assert!(saved.mods.enabled.contains(&id));
        assert!(!saved.maps.is_enabled(&id, "moon"));
        assert_eq!(std::fs::read(&input).unwrap(), input_before, "the original archive is unchanged");
        assert!(app.switch_profile(&second.id));
        assert!(!app.config.mods.enabled.contains(&id));
        assert!(app.config.maps.is_enabled(&id, "moon"));
        assert!(app.mods.get(&id).is_some(), "map installation is shared, profile preferences are separate");
        app.skins.shutdown(); drop(app); drop(profiles);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("ta-maps-app-{name}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn launcher(root: &std::path::Path) -> (egui::Context, LauncherApp) {
        let ctx = egui::Context::default();
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", root.join("config.toml").to_str().unwrap()]);
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        (ctx, app)
    }

    /// A frame of the launcher, as eframe runs it.
    fn frame(ctx: &egui::Context, app: &mut LauncherApp, mut input: egui::RawInput) -> egui::FullOutput {
        use eframe::App as _;
        let mut frame = eframe::Frame::_new_kittest();
        input.screen_rect.get_or_insert(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1120.0, 720.0)));
        let mut output = ctx.run_ui(input, |ui| {
            app.logic(ui.ctx(), &mut frame);
            app.ui(ui, &mut frame);
        });
        output.textures_delta.clear();
        output
    }

    /// The window's close box, clicked.
    fn close() -> egui::RawInput {
        let mut input = egui::RawInput::default();
        input.viewports.insert(egui::ViewportId::ROOT, egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() });
        input
    }

    fn sent(output: &egui::FullOutput, command: &egui::ViewportCommand) -> bool {
        output.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|viewport| viewport.commands.contains(command))
    }

    /// Closed while the maps are looked through (20,000 of a mod's took a minute): nothing is at
    /// risk, so nothing is asked and the window closes at once; nor does the exit wait for the look.
    #[test]
    fn closing_during_a_look_through_the_maps_closes_at_once() {
        let root = temp("close_scan");
        let (ctx, mut app) = launcher(&root);
        let hold = app.maps.pretend_scanning();
        assert!(app.maps.busy() && !app.maps.importing());
        assert!(app.work_at_risk().is_empty(), "a look through the maps is no work at risk: {:?}", app.work_at_risk());
        let output = frame(&ctx, &mut app, close());
        assert!(!sent(&output, &egui::ViewportCommand::CancelClose) && !app.confirm_close, "the window closes, nothing asked");
        // (Let go of in ten seconds whatever happens: the exit must not take that long.)
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(10));
            drop(hold);
        });
        let started = std::time::Instant::now();
        eframe::App::on_exit(&mut app, None);
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "the exit waited {:?} for the look", started.elapsed());
        assert!(!app.maps.busy());
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// The close question up while the import it asked about ends: the launcher closes, as the
    /// player asked, rather than the question going with the window left open.
    #[test]
    fn the_close_question_closes_the_window_once_nothing_is_at_risk() {
        let root = temp("close_done");
        let (ctx, mut app) = launcher(&root);
        app.confirm_close = true;
        assert!(app.work_at_risk().is_empty());
        let output = frame(&ctx, &mut app, egui::RawInput::default());
        assert!(sent(&output, &egui::ViewportCommand::Close), "closed as asked");
        assert!(app.close_confirmed && !app.confirm_close);
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A Supreme Commander import that ends with the dialog closed is told, failures and all:
    /// none made (red, the pack not turned on), some made (amber, not green), the import failed
    /// (red, with why).
    #[test]
    fn a_supreme_commander_import_is_told_with_the_dialog_closed_failures_and_all() {
        use crate::supcom::Outcome;
        let root = temp("supcom_told");
        let (ctx, mut app) = launcher(&root);
        let failed = || vec![("Seton's Clutch".to_string(), "It could not be made into a TA map: bad".to_string())];
        let told = |app: &mut LauncherApp, outcome: Outcome| {
            app.notice = None;
            app.notices.clear();
            app.supcom.pretend_import_ending(outcome);
            let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while app.supcom.importing() && std::time::Instant::now() < until {
                frame(&ctx, app, egui::RawInput::default());
            }
            app.notice.as_ref().map(|notice| (notice.text.clone(), notice.color))
        };
        let (text, colour) = told(&mut app, Outcome { failed: failed(), ..Default::default() }).expect("told");
        assert!(text.starts_with("The Supreme Commander map could not be made") && colour == RED, "{text}");
        assert!(!app.config.mods.enabled.iter().any(|id| id == crate::supcom::PACK_ID), "nothing made, nothing turned on");
        let (text, colour) = told(&mut app, Outcome { imported: vec!["Open Palms".into()], made: vec!["scmp_002".into()], failed: failed(), ..Default::default() }).expect("told");
        assert!(text.contains("but 1 could not be made") && colour == AMBER, "{text}");
        assert!(app.config.mods.enabled.iter().any(|id| id == crate::supcom::PACK_ID), "the maps made are turned on");
        let (text, colour) = told(&mut app, Outcome { error: Some("There is not enough space on the disk.".into()), ..Default::default() }).expect("told");
        assert!(text.ends_with("There is not enough space on the disk.") && colour == RED, "{text}");
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }
}
