//! Locating the original game: a UI-oriented wrapper around [`ta_vfs::install`].

use std::path::{Path, PathBuf};

use ta_vfs::install::{self, Content, InstallError, InstallSource, TaInstall};

/// Result of a detection pass.
pub struct Detection {
    pub result: Result<TaInstall, InstallError>,
    /// When the configured folder is invalid: a valid install found by auto-detection, offered
    /// to the user as a one-click fix.
    pub suggestion: Option<TaInstall>,
}

impl Detection {
    pub fn install(&self) -> Option<&TaInstall> {
        self.result.as_ref().ok()
    }
}

/// Detects the install. A configured folder is authoritative (an invalid one is reported, not
/// silently replaced); otherwise the registry, Steam and default folders are searched.
/// `pretend_missing` (a debug switch) makes auto-detection find nothing.
pub fn detect(configured: Option<&Path>, pretend_missing: bool) -> Detection {
    match configured {
        Some(dir) => {
            let result = install::discover(Some(dir));
            let suggestion = match &result {
                Err(_) if !pretend_missing => install::discover(None).ok(),
                _ => None,
            };
            Detection { result, suggestion }
        }
        None if pretend_missing => {
            Detection { result: Err(InstallError::NotFound { searched: searched_locations() }), suggestion: None }
        }
        None => Detection { result: install::discover(None), suggestion: None },
    }
}

/// The candidate folders auto-detection looks at, without duplicates.
fn searched_locations() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for (path, _) in install::candidates(None) {
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

/// Outcome of validating a folder chosen in the folder picker.
#[derive(Debug)]
pub enum Picked {
    /// `root` is a valid install; `nested` if it is a sub-folder of what the user picked.
    Valid { root: PathBuf, nested: bool },
    Invalid { path: PathBuf, missing: Vec<String> },
}

/// Validates a picked folder. Users often select the parent (e.g. `C:\GOG Games`), so one
/// level of sub-folders is searched as well.
pub fn validate_picked(path: &Path) -> Picked {
    match install::validate(path) {
        Ok(_) => Picked::Valid { root: path.to_path_buf(), nested: false },
        Err(missing) => match find_in_children(path) {
            Some(root) => Picked::Valid { root, nested: true },
            None => Picked::Invalid { path: path.to_path_buf(), missing },
        },
    }
}

fn find_in_children(dir: &Path) -> Option<PathBuf> {
    let mut children: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .take(512)
        .collect();
    children.sort();
    children.into_iter().find(|child| install::validate(child).is_ok())
}

/// Where the folder picker starts: the usual GOG or Steam install parents, if present.
pub fn browse_start_dir() -> Option<PathBuf> {
    [r"C:\GOG Games", r"C:\Program Files (x86)\GOG Galaxy\Games", r"C:\Program Files (x86)\Steam\steamapps\common", r"C:\Program Files (x86)", r"C:\"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_dir())
}

/// Human-readable detection source.
pub fn source_label(source: InstallSource) -> &'static str {
    match source {
        InstallSource::Explicit => ta_ui::i18n::n("Game folder set in your settings"),
        InstallSource::Environment => ta_ui::i18n::n("TA_REBORN_GAME_DIR environment variable"),
        InstallSource::GogRegistry => ta_ui::i18n::n("GOG installation (registry)"),
        InstallSource::Steam => ta_ui::i18n::n("Steam library"),
        InstallSource::CommonLocation => ta_ui::i18n::n("Default install folder"),
    }
}

/// One line of the detected-content checklist.
pub struct ContentItem {
    /// (Marked for translation; `t` it where it is shown.)
    pub label: &'static str,
    /// The files that indicate this content.
    pub files: &'static str,
    pub present: bool,
}

pub fn content_items(content: &Content) -> [ContentItem; 7] {
    let item = |label, files, present| ContentItem { label, files, present };
    [
        item(ta_ui::i18n::n("Base game"), "totala1.hpi · totala2.hpi", content.base_game),
        item(ta_ui::i18n::n("Campaigns"), "totala4.hpi", content.campaigns),
        item(ta_ui::i18n::n("The Core Contingency"), "ccdata.ccx · ccmaps.ccx", content.core_contingency),
        item(ta_ui::i18n::n("Battle Tactics"), "btdata.ccx", content.battle_tactics),
        item(ta_ui::i18n::n("3.1 patch"), "rev31.gp3", content.patch_3_1),
        item(ta_ui::i18n::n("Music"), r"music\*.mp3 · *.ogg", content.music),
        item(ta_ui::i18n::n("Movies"), r"data\*.zrb", content.movies),
    ]
}

/// Short edition name for the detected content, e.g. "Commander Pack".
pub fn edition_name(content: &Content) -> &'static str {
    match (content.core_contingency, content.battle_tactics) {
        (true, true) => ta_ui::i18n::n("Commander Pack (both expansions)"),
        (true, false) => ta_ui::i18n::n("Base game + The Core Contingency"),
        (false, true) => ta_ui::i18n::n("Base game + Battle Tactics"),
        (false, false) => ta_ui::i18n::n("Base game"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ta_launcher_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_install(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        for f in install::REQUIRED_FILES {
            std::fs::write(dir.join(f), b"").unwrap();
        }
    }

    #[test]
    fn picked_parent_folder_finds_install_below() {
        let root = temp_dir("nested");
        fake_install(&root.join("Total Annihilation"));
        match validate_picked(&root) {
            Picked::Valid { root: found, nested } => {
                assert!(nested);
                assert!(found.ends_with("Total Annihilation"));
            }
            other => panic!("expected a nested install, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn picked_empty_folder_reports_missing_files() {
        let root = temp_dir("empty");
        match validate_picked(&root) {
            Picked::Invalid { missing, .. } => assert_eq!(missing, install::REQUIRED_FILES),
            other => panic!("expected invalid, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pretend_missing_finds_nothing_but_honors_configured_folder() {
        let detection = detect(None, true);
        assert!(matches!(detection.result, Err(InstallError::NotFound { .. })));

        let root = temp_dir("configured");
        fake_install(&root);
        let detection = detect(Some(&root), true);
        assert_eq!(detection.install().map(|i| i.source), Some(InstallSource::Explicit));
        let _ = std::fs::remove_dir_all(root);
    }
}
