//! Loading, sanitising and debounced saving of the shared `config.toml`.
//!
//! Unlike [`ta_config::Config::load`], loading here keeps the parse error so the UI can show
//! it, and an unusable file is backed up (`config.toml.bak`) before it is first overwritten.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ta_config::Config;
#[cfg(test)]
use ta_config::{Audio, Graphics, limits};

/// Quiet period after the last edit before settings are written.
pub const SAVE_DELAY: Duration = Duration::from_millis(700);

/// State of the settings file, shown in the status bar.
#[derive(Clone, Debug, PartialEq)]
pub enum SaveStatus {
    /// Nothing has been changed in this session.
    Unchanged,
    /// Edits are waiting for [`SAVE_DELAY`] to pass.
    Pending,
    Saved(Instant),
    Failed(String),
}

/// Tracks the on-disk config and writes edits after a short delay.
pub struct ConfigStore {
    pub path: PathBuf,
    device_root: PathBuf,
    /// Contents of the file as last loaded or written.
    saved: Config,
    /// The config at the previous tick, to detect edits.
    seen: Config,
    last_edit: Option<Instant>,
    /// Sanitized or recovered defaults must reach disk before launching the game.
    needs_repair: bool,
    had_file: bool,
    pub status: SaveStatus,
    /// Why the existing file could not be used; defaults are in effect.
    pub load_error: Option<String>,
    /// Where the unusable file was backed up, once that happened.
    pub backup: Option<PathBuf>,
}

impl ConfigStore {
    /// Loads (or defaults) the config at `path`.
    #[cfg(test)]
    pub fn open(path: PathBuf) -> (ConfigStore, Config) {
        let root = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Self::open_in(path, root)
    }

    pub fn open_in(path: PathBuf, device_root: PathBuf) -> (ConfigStore, Config) {
        let (config, load_error) = load(&path);
        let had_file = path.is_file();
        Self::loaded(path, device_root, config, load_error, had_file)
    }

    /// A profile handover may not recover defaults: validate and keep the exact
    /// configuration that will be adopted before changing the active identity.
    pub fn try_open_in(path: PathBuf, device_root: PathBuf) -> Result<(ConfigStore, Config), ta_config::LoadError> {
        let config = Config::try_load_from(&path)?;
        Ok(Self::loaded(path, device_root, config, None, true))
    }

    fn loaded(path: PathBuf, device_root: PathBuf, mut config: Config, load_error: Option<String>, had_file: bool) -> (ConfigStore, Config) {
        let path = std::path::absolute(&path).unwrap_or(path);
        let device_root = std::path::absolute(&device_root).unwrap_or(device_root);
        let needs_repair = sanitize(&mut config);
        if needs_repair {
            log::info!("some settings in {} were out of range and have been adjusted", path.display());
        }
        let store = ConfigStore {
            path,
            device_root,
            saved: config.clone(),
            seen: config.clone(),
            last_edit: None,
            needs_repair: needs_repair || load_error.is_some(),
            had_file,
            status: SaveStatus::Unchanged,
            load_error,
            backup: None,
        };
        (store, config)
    }

    /// Shared device root for installed skins, mods, caches and logs.
    pub fn dir(&self) -> PathBuf {
        self.device_root.clone()
    }

    /// Call once per frame: notices edits and saves once they have settled.
    pub fn tick(&mut self, config: &mut Config, ctx: &egui::Context) {
        let now = Instant::now();
        if *config != self.seen {
            self.seen = config.clone();
            self.last_edit = Some(now);
            self.status = if *config == self.saved { SaveStatus::Unchanged } else { SaveStatus::Pending };
        }
        if *config == self.saved {
            return;
        }
        if let Some(edited) = self.last_edit {
            let elapsed = now.duration_since(edited);
            if elapsed >= SAVE_DELAY {
                self.save_now(config);
            } else {
                ctx.request_repaint_after(SAVE_DELAY - elapsed);
            }
        }
    }

    /// Writes immediately (used by "Retry", before launching the game, and on exit). Should the
    /// file have changed since it was read or written here (the game saved its options), only
    /// what was changed here is written over it: the rest stays as the game left it, and
    /// `config` takes it in.
    pub fn save_now(&mut self, config: &mut Config) {
        self.last_edit = None;
        // Reading before acquiring the game's write lock loses a save made while
        // we wait for that lock. Hold one transaction through the merge and write.
        let transaction = match ta_config::SettingsTransaction::open(&self.path) {
            Ok(transaction) => transaction,
            Err(error) => {
                self.seen = config.clone();
                self.status = SaveStatus::Failed(error.to_string());
                return;
            }
        };
        // Recheck the file first: an editor or the game may have damaged it since
        // startup. Never silently replace an unreadable external change.
        match transaction.load() {
            Ok(mut file) => {
                sanitize(&mut file);
                if file != self.saved {
                    let Some(merged) = merge(&self.saved, config, &file) else {
                        self.seen = config.clone();
                        self.status = SaveStatus::Failed(ta_ui::i18n::t("Could not combine the game's settings with your unsaved changes. Your changes are still kept in the launcher.").to_string());
                        return;
                    };
                    log::info!("{} was changed by the game since: its changes kept", self.path.display());
                    *config = merged;
                    // Even if the following write fails, these external values are now
                    // part of our view. They are not local edits on the next retry.
                    self.saved = file;
                }
            }
            Err(ta_config::LoadError::Missing) => {},
            Err(ta_config::LoadError::Invalid(error)) => {
                self.load_error = Some(error);
                self.needs_repair = true;
                if let Err(message) = self.backup_unusable_file() {
                    self.seen = config.clone();
                    self.status = SaveStatus::Failed(message);
                    return;
                }
            }
            Err(error) => {
                self.seen = config.clone();
                self.status = SaveStatus::Failed(error.to_string());
                return;
            }
        }
        // An explicit save can run before tick sees the latest edit. Whether the write
        // succeeds or fails, these values have been observed: only a new edit may restart
        // the debounce, otherwise a failure's Retry action would disappear on the next tick.
        self.seen = config.clone();
        match transaction.save(config) {
            Ok(()) => {
                log::info!("saved settings to {}", self.path.display());
                self.saved = config.clone();
                self.needs_repair = false;
                self.had_file = true;
                self.load_error = None;
                self.status = SaveStatus::Saved(Instant::now());
            }
            Err(e) => {
                log::error!("could not save {}: {e}", self.path.display());
                self.status = SaveStatus::Failed(e.to_string());
            }
        }
    }

    /// Takes in the game's settings without losing edits made here before the save delay,
    /// or after a failed save. Local edits win conflicts; this launcher's own options stay
    /// as it has them. Nothing is written until the usual debounce or an explicit retry.
    pub fn reload_after_game(&mut self, config: &mut Config) -> Result<(), String> {
        let mut file = Config::try_load_from(&self.path).map_err(|error| {
            ta_ui::i18n::tf("Could not reload settings from {path}: {error}", &[("path", &self.path.display()), ("error", &error)])
        })?;
        let needs_repair = sanitize(&mut file);
        let mut merged = merge(&self.saved, config, &file).ok_or_else(|| {
            ta_ui::i18n::t("Could not combine the game's settings with your unsaved changes. Your changes are still kept in the launcher.").to_string()
        })?;
        merged.launcher = config.launcher.clone();
        self.saved = file;
        self.needs_repair = needs_repair;
        self.had_file = true;
        self.load_error = None;
        self.seen = merged.clone();
        *config = merged;
        if *config == self.saved {
            self.last_edit = None;
            self.status = SaveStatus::Unchanged;
        } else if matches!(self.status, SaveStatus::Failed(_)) {
            // Keep the error and Retry action visible; a game exit is not a save retry.
            self.last_edit = None;
        } else {
            self.last_edit.get_or_insert_with(Instant::now);
            self.status = SaveStatus::Pending;
        }
        Ok(())
    }

    /// Saves pending edits, if any. A caller starting the game must wait for success so the
    /// game reads the settings the player chose. Failures leave those edits available to retry.
    pub fn flush(&mut self, config: &mut Config) -> Result<(), String> {
        // Launch/Close/Switch are explicit boundaries. Check even without a local
        // edit: deletion or corruption since startup must not make the child silently
        // use defaults, and valid external edits should be reflected in this UI too.
        let disk_changed = match Config::try_load_from(&self.path) {
            Ok(mut file) => sanitize(&mut file) || file != self.saved,
            Err(ta_config::LoadError::Missing) => self.had_file,
            Err(_) => true,
        };
        if *config != self.saved || self.needs_repair || disk_changed || matches!(self.status, SaveStatus::Failed(_)) {
            self.save_now(config);
            if let SaveStatus::Failed(message) = &self.status {
                return Err(message.clone());
            }
        }
        Ok(())
    }

    /// Keeps a copy of a file we could not parse before replacing it, so hand edits survive.
    fn backup_unusable_file(&mut self) -> Result<(), String> {
        if self.load_error.is_none() || !self.path.is_file() {
            return Ok(());
        }
        // An explicit retry need not duplicate the same backup, but a later corrupt
        // edit must never overwrite an earlier recovery copy.
        if self.backup.as_ref().is_some_and(|backup| same_bounded_files(backup, &self.path)) {
            return Ok(());
        }
        let mut name = self.path.file_name().unwrap_or_default().to_os_string();
        name.push(".bak");
        let mut backup = self.path.with_file_name(&name);
        let mut suffix = 1u32;
        while backup.is_file() {
            let mut next = name.clone();
            next.push(format!(".{suffix}"));
            backup = self.path.with_file_name(next);
            suffix = suffix.checked_add(1).ok_or_else(|| "too many settings backups".to_string())?;
        }
        std::fs::copy(&self.path, &backup)
            .map_err(|e| format!("could not back up the unreadable settings file to {}: {e}", backup.display()))?;
        log::info!("backed up unreadable settings to {}", backup.display());
        self.backup = Some(backup);
        Ok(())
    }
}

/// Recovery copies are user-writable too. An old backup must not reintroduce an
/// unrestricted allocation when deciding whether the current corrupt file was kept.
fn same_bounded_files(a: &Path, b: &Path) -> bool {
    use std::io::Read;
    let read = |path: &Path| -> Option<Vec<u8>> {
        let metadata = std::fs::metadata(path).ok()?;
        if !metadata.is_file() || metadata.len() > ta_config::MAX_CONFIG_BYTES { return None; }
        let file = std::fs::File::open(path).ok()?;
        let mut bytes = Vec::new();
        file.take(ta_config::MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() as u64 <= ta_config::MAX_CONFIG_BYTES).then_some(bytes)
    };
    read(a).zip(read(b)).is_some_and(|(a, b)| a == b)
}

/// `mine`, the settings as changed here since `base`, put onto `theirs`, the file as the game
/// left it: each value changed here is taken (tables key by key), the rest is theirs. None
/// when a side does not go into TOML and back.
fn merge(base: &Config, mine: &Config, theirs: &Config) -> Option<Config> {
    let value = |c: &Config| toml::Value::try_from(c).ok();
    let mut merged = value(theirs)?;
    put_changes(&value(base)?, &value(mine)?, &mut merged);
    let mut merged: Config = merged.try_into().ok()?;
    sanitize(&mut merged);
    Some(merged)
}

fn put_changes(base: &toml::Value, mine: &toml::Value, theirs: &mut toml::Value) {
    match (base, mine, theirs) {
        (toml::Value::Table(base), toml::Value::Table(mine), toml::Value::Table(theirs)) => {
            for (key, value) in mine {
                match (base.get(key), theirs.get_mut(key)) {
                    (Some(old), _) if old == value => {}
                    (Some(old), Some(slot)) => put_changes(old, value, slot),
                    _ => {
                        theirs.insert(key.clone(), value.clone());
                    }
                }
            }
            // (What was taken out here goes from theirs too.)
            for key in base.keys().filter(|k| !mine.contains_key(*k)) {
                theirs.remove(key);
            }
        }
        (_, mine, theirs) => *theirs = mine.clone(),
    }
}

/// Reads the config, returning the parse or read error instead of hiding it.
/// A missing file is not an error: it simply means defaults (first run).
fn load(path: &Path) -> (Config, Option<String>) {
    match Config::try_load_from(path) {
        Ok(config) => (config, None),
        Err(ta_config::LoadError::Missing) => (Config::default(), None),
        Err(e) => (Config::default(), Some(format!("the file could not be read: {e}"))),
    }
}

/// Clamps values to [`ta_config::limits`] and replaces non-finite numbers (a NaN would also defeat
/// change detection). Returns whether anything changed.
pub fn sanitize(config: &mut Config) -> bool {
    config.sanitize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_settings_are_reported_without_backup_or_replacement() {
        let path = temp_file("oversized");
        let size = ta_config::MAX_CONFIG_BYTES + 1;
        std::fs::File::create(&path).unwrap().set_len(size).unwrap();
        let (mut store, mut config) = ConfigStore::open(path.clone());
        assert!(store.load_error.as_ref().is_some_and(|error| error.contains("4 MiB")));
        config.audio.music = 0.25;
        assert!(store.flush(&mut config).is_err(), "recovery must not replace a file that was never read");
        assert!(store.backup.is_none(), "oversized files are not reread for backup comparison");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), size);
        // Removing/fixing the bad file makes the same retained edit retryable.
        std::fs::remove_file(&path).unwrap();
        store.flush(&mut config).unwrap();
        assert_eq!(Config::try_load_from(&path).unwrap().audio.music, 0.25);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn enlarged_old_backup_cannot_block_preserving_new_corrupt_settings() {
        let path = temp_file("oversized_backup");
        let original = "graphics = [[[ broken externally";
        std::fs::write(&path, original).unwrap();
        let (mut store, mut config) = ConfigStore::open(path.clone());
        store.flush(&mut config).unwrap();
        let previous = store.backup.clone().unwrap();
        std::fs::File::create(&previous).unwrap().set_len(ta_config::MAX_CONFIG_BYTES + 1).unwrap();
        std::fs::write(&path, original).unwrap();
        store.flush(&mut config).unwrap();
        assert_ne!(store.backup.as_ref(), Some(&previous));
        assert_eq!(std::fs::metadata(&previous).unwrap().len(), ta_config::MAX_CONFIG_BYTES + 1);
        assert_eq!(std::fs::read_to_string(store.backup.as_ref().unwrap()).unwrap(), original);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ta_launcher_persist_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.toml")
    }

    #[test]
    fn sanitize_fixes_out_of_range_and_nan() {
        let mut c = Config::default();
        assert!(!sanitize(&mut c), "defaults are within limits");
        c.graphics.msaa = 3;
        c.graphics.terrain_relight = f32::NAN;
        c.audio.music = 7.0;
        c.gameplay.player_name = "x".repeat(100);
        assert!(sanitize(&mut c));
        assert_eq!(c.graphics.msaa, Graphics::default().msaa);
        assert_eq!(c.graphics.terrain_relight, Graphics::default().terrain_relight);
        assert_eq!(c.audio.music, 1.0);
        assert_eq!(c.gameplay.player_name.len(), limits::PLAYER_NAME_CHARS);
    }

    #[test]
    fn accessibility_ranges_reject_invalid_saved_values() {
        for (cursor, duration, expected_cursor, expected_duration) in [
            (f32::NAN, f32::INFINITY, 1.0, 1.0),
            (0.0, -3.0, 1.0, 1.0),
            (9.0, 8.0, 2.5, 4.0),
            (1.75, 2.5, 1.75, 2.5),
        ] {
            let mut config = Config::default();
            config.gameplay.cursor_scale = cursor;
            config.gameplay.message_duration = duration;
            sanitize(&mut config);
            assert_eq!(config.gameplay.cursor_scale, expected_cursor);
            assert_eq!(config.gameplay.message_duration, expected_duration);
        }
    }

    #[test]
    fn controller_tuning_is_sanitized_before_saving() {
        for (deadzone, speed, expected_deadzone, expected_speed) in [
            (f32::NAN, f32::INFINITY, 0.18, 1.0),
            (-0.3, -2.0, 0.0, 0.25),
            (0.9, 8.0, 0.5, 3.0),
            (0.22, 1.75, 0.22, 1.75),
        ] {
            let mut config = Config::default();
            config.gameplay.gamepad_deadzone = deadzone;
            config.gameplay.gamepad_sensitivity = speed;
            sanitize(&mut config);
            assert_eq!(config.gameplay.gamepad_deadzone, expected_deadzone);
            assert_eq!(config.gameplay.gamepad_sensitivity, expected_speed);
        }
    }

    #[test]
    fn invalid_file_is_reported_and_backed_up_before_overwrite() {
        let path = temp_file("invalid");
        std::fs::write(&path, "graphics = [[[ not toml").unwrap();
        let (mut store, mut config) = ConfigStore::open(path.clone());
        assert!(store.load_error.is_some());
        assert_eq!(config, Config::default());

        config.gameplay.player_name = "Tester".into();
        store.save_now(&mut config);
        assert!(matches!(store.status, SaveStatus::Saved(_)), "{:?}", store.status);
        let backup = store.backup.clone().expect("backup written");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "graphics = [[[ not toml");
        let (_, reloaded) = ConfigStore::open(path.clone());
        assert_eq!(reloaded.gameplay.player_name, "Tester");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn partial_installer_file_keeps_defaults() {
        let path = temp_file("partial");
        std::fs::write(&path, "game_dir = 'C:\\GOG Games\\Total Annihilation'\n").unwrap();
        let (store, config) = ConfigStore::open(path.clone());
        assert!(store.load_error.is_none());
        assert_eq!(config.game_dir.as_deref(), Some(Path::new(r"C:\GOG Games\Total Annihilation")));
        assert_eq!(config.graphics, Graphics::default());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn edits_are_saved_once_they_settle() {
        let path = temp_file("debounce");
        let ctx = egui::Context::default();
        let (mut store, mut config) = ConfigStore::open(path.clone());
        store.tick(&mut config, &ctx);
        assert_eq!(store.status, SaveStatus::Unchanged);

        // An edit that is undone before the delay never touches the disk.
        config.audio.music = 0.25;
        store.tick(&mut config, &ctx);
        assert_eq!(store.status, SaveStatus::Pending);
        config.audio.music = Audio::default().music;
        store.tick(&mut config, &ctx);
        assert_eq!(store.status, SaveStatus::Unchanged);

        config.audio.music = 0.25;
        store.tick(&mut config, &ctx);
        assert!(!path.exists(), "nothing is written before the delay");
        std::thread::sleep(SAVE_DELAY + Duration::from_millis(50));
        store.tick(&mut config, &ctx);
        assert!(matches!(store.status, SaveStatus::Saved(_)), "{:?}", store.status);
        let (_, reloaded) = ConfigStore::open(path.clone());
        assert_eq!(reloaded.audio.music, 0.25);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The game saved its options while the launcher ran: the launcher's own change goes over
    /// the file, the game's stay (and the launcher shows them).
    #[test]
    fn changes_from_the_game_are_kept() {
        let path = temp_file("merge");
        let (mut store, mut config) = ConfigStore::open(path.clone());
        config.mods.enabled = vec!["big".into()];
        store.save_now(&mut config);
        let mut game = Config::load_from(&path);
        game.audio.music = 0.3;
        game.graphics.bloom = !game.graphics.bloom;
        game.save_to(&path).unwrap();
        config.gameplay.player_name = "Tester".into();
        config.mods.enabled.clear();
        store.save_now(&mut config);
        let file = Config::load_from(&path);
        assert_eq!((file.audio.music, file.graphics.bloom), (0.3, game.graphics.bloom), "the game's");
        assert_eq!(file.gameplay.player_name, "Tester", "the launcher's");
        assert!(file.mods.enabled.is_empty(), "the launcher's, a list emptied");
        assert_eq!(config, file);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_waiting_launcher_save_merges_the_game_transaction_that_finishes_first() {
        use std::sync::mpsc;
        let path = temp_file("concurrent_game_save");
        Config::default().save_to(&path).unwrap();
        let (mut store, mut config) = ConfigStore::open(path.clone());
        config.audio.music = 0.25;
        let game_transaction = ta_config::SettingsTransaction::open(&path).unwrap();
        let mut game = game_transaction.load().unwrap();
        game.graphics.width = 1920;
        game.audio.effects = 0.6;
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            store.save_now(&mut config);
            done_tx.send((store, config)).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(done_rx.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)),
            "the launcher waits while the game owns the settings transaction");
        game_transaction.save(&game).unwrap();
        drop(game_transaction);
        let (store, config) = done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        writer.join().unwrap();
        assert!(matches!(store.status, SaveStatus::Saved(_)));
        let file = Config::try_load_from(&path).unwrap();
        assert_eq!(file.audio.music, 0.25, "the launcher's edit is saved");
        assert_eq!((file.graphics.width, file.audio.effects), (1920, 0.6), "the game's concurrent changes survive");
        assert_eq!(config, file, "the launcher adopts the merged values too");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn game_exit_keeps_debounced_edits_and_takes_in_external_changes() {
        let path = temp_file("reload_pending");
        Config::default().save_to(&path).unwrap();
        let (mut store, mut config) = ConfigStore::open(path.clone());
        let ctx = egui::Context::default();
        config.audio.music = 0.25;
        store.tick(&mut config, &ctx);
        let edited = store.last_edit;
        let mut game = Config::load_from(&path);
        game.gameplay.player_name = "From the game".into();
        game.save_to(&path).unwrap();

        store.reload_after_game(&mut config).unwrap();
        assert_eq!(config.audio.music, 0.25, "the pending launcher edit survives");
        assert_eq!(config.gameplay.player_name, "From the game");
        assert_eq!(store.status, SaveStatus::Pending);
        assert_eq!(store.last_edit, edited, "the existing save deadline is kept");
        assert_eq!(Config::load_from(&path), game, "reloading does not write the file");
        store.flush(&mut config).unwrap();
        assert_eq!(Config::load_from(&path), config);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn local_changes_win_reload_conflicts_even_before_the_next_tick() {
        let path = temp_file("reload_conflict");
        Config::default().save_to(&path).unwrap();
        let (mut store, mut config) = ConfigStore::open(path.clone());
        let launcher = config.launcher.clone();
        config.audio.music = 0.25;
        let mut game = Config::load_from(&path);
        game.audio.music = 0.75;
        game.launcher.on_play = ta_config::OnPlay::Close;
        game.save_to(&path).unwrap();

        store.reload_after_game(&mut config).unwrap();
        assert_eq!(config.audio.music, 0.25, "local edits win the same-field conflict");
        assert_eq!(config.launcher, launcher, "this launcher's own options are kept");
        assert_eq!(store.status, SaveStatus::Pending);
        assert!(store.last_edit.is_some(), "previously unseen edits get a save deadline");
        store.flush(&mut config).unwrap();
        assert_eq!(Config::load_from(&path), config);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn failed_save_survives_game_exit_until_a_successful_retry() {
        let path = temp_file("reload_failed");
        Config::default().save_to(&path).unwrap();
        let (mut store, mut config) = ConfigStore::open(path.clone());
        // A directory at the file path fails reliably, even with an administrator account.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        config.audio.music = 0.25;
        let error = store.flush(&mut config).expect_err("the settings cannot be written");
        assert!(!error.is_empty());
        assert_eq!(store.status, SaveStatus::Failed(error.clone()));
        assert_eq!(config.audio.music, 0.25, "failure must retain the local edit");
        store.tick(&mut config, &egui::Context::default());
        assert_eq!(store.status, SaveStatus::Failed(error.clone()), "tick must not hide a fresh flush failure");
        assert!(store.last_edit.is_none(), "a failed flush is not retried by the debounce");
        std::fs::remove_dir(&path).unwrap();
        let mut game = Config::default();
        game.gameplay.player_name = "From the game".into();
        game.save_to(&path).unwrap();

        store.reload_after_game(&mut config).unwrap();
        assert_eq!(config.audio.music, 0.25);
        assert_eq!(config.gameplay.player_name, "From the game");
        assert_eq!(store.status, SaveStatus::Failed(error), "Retry remains available");
        store.tick(&mut config, &egui::Context::default());
        assert!(matches!(store.status, SaveStatus::Failed(_)), "no automatic retry on reload");
        assert_eq!(Config::load_from(&path), game);
        store.flush(&mut config).expect("an explicit retry now succeeds");
        assert!(matches!(store.status, SaveStatus::Saved(_)));
        assert_eq!(Config::load_from(&path), config);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn backup_failure_stays_actionable_until_retry_or_a_new_edit() {
        let path = temp_file("backup_failed");
        std::fs::write(&path, "graphics = [[[ not toml").unwrap();
        let (mut store, mut config) = ConfigStore::open(path.clone());
        let backup = path.with_file_name("config.toml.bak");
        std::fs::create_dir(&backup).unwrap();
        config.audio.music = 0.25;
        let error = store.flush(&mut config).expect_err("the unreadable file cannot be backed up");

        store.tick(&mut config, &egui::Context::default());
        assert_eq!(store.status, SaveStatus::Failed(error));
        assert!(store.last_edit.is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "graphics = [[[ not toml");
        // Editing a value deliberately schedules another save; idle ticks do not.
        config.audio.music = 0.5;
        store.tick(&mut config, &egui::Context::default());
        assert_eq!(store.status, SaveStatus::Pending);
        assert!(store.last_edit.is_some());
        std::fs::remove_dir(&backup).unwrap();
        store.flush(&mut config).unwrap();
        assert_eq!(Config::load_from(&path).audio.music, 0.5);
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "graphics = [[[ not toml");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_unreadable_external_file_does_not_discard_local_changes() {
        let path = temp_file("reload_invalid");
        Config::default().save_to(&path).unwrap();
        let (mut store, mut config) = ConfigStore::open(path.clone());
        config.audio.music = 0.25;
        store.tick(&mut config, &egui::Context::default());
        let before = config.clone();
        std::fs::write(&path, "graphics = [[[ not toml").unwrap();

        assert!(store.reload_after_game(&mut config).is_err());
        assert_eq!(config, before);
        assert_eq!(store.status, SaveStatus::Pending);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "graphics = [[[ not toml");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn flushing_unchanged_settings_succeeds_without_writing_a_file() {
        let path = temp_file("flush_unchanged");
        let (mut store, mut config) = ConfigStore::open(path.clone());
        store.flush(&mut config).unwrap();
        assert_eq!(store.status, SaveStatus::Unchanged);
        assert!(!path.exists(), "no changes need to be saved");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn corrupt_external_edits_are_backed_up_without_replacing_older_recovery_files() {
        let path = temp_file("external_corruption");
        Config::default().save_to(&path).unwrap();
        let (mut store, mut config) = ConfigStore::open(path.clone());
        let old_backup = path.with_file_name("config.toml.bak");
        std::fs::write(&old_backup, "older hand edits").unwrap();
        std::fs::write(&path, "graphics = [[[ broken externally").unwrap();
        config.audio.music = 0.25;
        store.flush(&mut config).unwrap();
        assert_eq!(Config::try_load_from(&path).unwrap().audio.music, 0.25);
        assert_eq!(std::fs::read_to_string(&old_backup).unwrap(), "older hand edits");
        let first = store.backup.clone().unwrap();
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "graphics = [[[ broken externally");
        assert!(store.load_error.is_none(), "successful recovery clears the warning");

        std::fs::write(&path, "audio = [[[ second broken edit").unwrap();
        config.audio.music = 0.5;
        store.flush(&mut config).unwrap();
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "graphics = [[[ broken externally");
        assert_eq!(std::fs::read_to_string(store.backup.as_ref().unwrap()).unwrap(), "audio = [[[ second broken edit");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn launch_flush_repairs_defaulted_or_sanitized_settings_without_a_manual_edit() {
        let path = temp_file("repair_before_launch");
        for contents in ["graphics = [[[ invalid", "[graphics]\nwidth = 1\nmsaa = 3"] {
            std::fs::write(&path, contents).unwrap();
            let (mut store, mut config) = ConfigStore::open(path.clone());
            store.flush(&mut config).unwrap();
            assert_eq!(Config::try_load_from(&path).unwrap(), config);
            assert!(matches!(store.status, SaveStatus::Saved(_)));
            assert!(!store.needs_repair);
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn a_failed_merge_retry_does_not_write_back_stale_external_values() {
        let path = temp_file("failed_merged_retry");
        Config::default().save_to(&path).unwrap();
        let (mut store, mut config) = ConfigStore::open(path.clone());
        config.audio.music = 0.25;
        let mut game = Config::try_load_from(&path).unwrap();
        game.audio.effects = 0.5;
        game.save_to(&path).unwrap();
        use std::os::windows::fs::OpenOptionsExt;
        // Permit reads but deny replacement while the launcher adopts external edits.
        let blocker = std::fs::OpenOptions::new().read(true).share_mode(1).open(&path).unwrap();
        store.flush(&mut config).expect_err("the atomic write is blocked");
        assert_eq!(config.audio.effects, 0.5, "external value adopted before the failed write");
        drop(blocker);
        game.audio.effects = 0.8;
        game.save_to(&path).unwrap();
        store.flush(&mut config).unwrap();
        let saved = Config::try_load_from(&path).unwrap();
        assert_eq!(saved.audio.music, 0.25, "pending local edit survives");
        assert_eq!(saved.audio.effects, 0.8, "latest external edit survives retry");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn unchanged_launch_flush_recovers_deleted_or_corrupted_external_settings() {
        let path = temp_file("external_damage_unchanged");
        let mut original = Config::default();
        original.audio.music = 0.2;
        original.save_to(&path).unwrap();
        let (mut store, mut config) = ConfigStore::open(path.clone());
        std::fs::remove_file(&path).unwrap();
        store.flush(&mut config).unwrap();
        assert_eq!(Config::try_load_from(&path).unwrap(), original);
        std::fs::write(&path, "audio = [[[ damaged").unwrap();
        store.flush(&mut config).unwrap();
        assert_eq!(Config::try_load_from(&path).unwrap(), original);
        assert_eq!(std::fs::read_to_string(store.backup.as_ref().unwrap()).unwrap(), "audio = [[[ damaged");
        let mut external = original;
        external.audio.music = 0.4;
        external.graphics.width = 1;
        external.save_to(&path).unwrap();
        store.flush(&mut config).unwrap();
        assert_eq!(config.audio.music, 0.4);
        assert_eq!(config.graphics.width, *limits::WIDTH.start());
        assert_eq!(Config::try_load_from(&path).unwrap(), config);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn installer_escaped_fallback_format_parses() {
        // For paths with quotes or non-ASCII characters the installer writes an ASCII-only
        // basic string with \\ and \uXXXX escapes.
        let path = temp_file("escaped");
        std::fs::write(&path, "game_dir = \"D:\\\\Spiele\\\\Total Annihilation \\u00DCbung\"\n").unwrap();
        let (store, config) = ConfigStore::open(path.clone());
        assert!(store.load_error.is_none(), "{:?}", store.load_error);
        assert_eq!(config.game_dir.as_deref(), Some(Path::new("D:\\Spiele\\Total Annihilation Übung")));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
