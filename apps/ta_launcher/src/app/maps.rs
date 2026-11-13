//! Map library actions use the same pinned profile/config launch path as Play.
use super::{LauncherApp, Page};
use crate::maps::{Action, Request};
use crate::theme::*;

impl LauncherApp {
    /// Reads may overlap other library work; publishing a map pack must finish before
    /// the same shared mod folder can be replaced or removed through the Mods page.
    pub(crate) fn map_import_blocks_mod_changes(&mut self) -> bool {
        if self.supcom.importing() {
            self.notify("Wait for the Supreme Commander maps being made, then try again.", AMBER);
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

    pub(super) fn maps_action(&mut self, action: Action) {
        let (edit, skirmish) = (matches!(&action, Action::Edit(_)), matches!(&action, Action::Skirmish(_)));
        match action {
            Action::Rescan => { self.refresh_mods(); self.rescan_maps(); }
            Action::Browse => {
                if self.maps.busy() || self.installer.busy() || self.game_running() { return; }
                if let Some(path) = rfd::FileDialog::new().set_title(ta_ui::i18n::t("Import map archive"))
                    .add_filter(ta_ui::i18n::t("Map archives"), &["ufo", "hpi", "ccx", "gp3", "gpf", "zip", "7z", "rar"]).pick_file() {
                    self.maps_action(Action::Import(path));
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
                if !entry.active || entry.shadowed || !edit && !entry.available(&self.config.maps) || self.maps.busy() { return; }
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
        app.maps_action(Action::Import(input.clone()));
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
}
