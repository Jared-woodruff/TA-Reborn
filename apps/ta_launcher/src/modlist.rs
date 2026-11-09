//! Mod discovery across the launcher's mod folders and editing of the enabled list
//! (`config.mods.enabled`, in load order: later entries win).

use std::path::{Path, PathBuf};

use ta_ui::i18n::tf;
use ta_vfs::mods::{self, ModError, ModInfo, ModKind};

pub use ta_vfs::mods::APP_FOLDER_LABEL;

/// A folder scanned for mods.
#[derive(Clone, Debug)]
pub struct ModFolder {
    /// Short label shown in the UI.
    pub label: &'static str,
    pub path: PathBuf,
    pub exists: bool,
}

/// A mod found in one of the folders.
#[derive(Clone, Debug)]
pub struct ModEntry {
    pub info: ModInfo,
    /// Index into [`ModList::folders`].
    pub folder: usize,
}

impl ModEntry {
    pub fn id(&self) -> &str {
        &self.info.manifest.id
    }

    pub fn is_archive(&self) -> bool {
        self.info.kind == ModKind::Archive
    }
}

/// Result of resolving the enabled list for the current scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// Mod ids in the order the game will mount them.
    Ok(Vec<String>),
    Err(String),
}

/// The mods found on disk plus the resolved load order of the enabled ones.
#[derive(Debug)]
pub struct ModList {
    pub folders: Vec<ModFolder>,
    pub mods: Vec<ModEntry>,
    /// Manifest errors and duplicate ids from the last scan, ready for display.
    pub problems: Vec<String>,
    /// The scan's manifests that do not read, as found (a mod "not installed" may be one).
    broken: Vec<ModError>,
    pub resolution: Resolution,
    /// The enabled list [`Self::resolution`] was computed for, and in which of the interface's
    /// languages (its problem is told in it: [`ta_ui::i18n::generation`]).
    resolved_for: Vec<String>,
    resolved_in: u32,
}

/// The folders to scan: next to the launcher, in the user's config folder, and the optional
/// extra folder from the config. Duplicates (e.g. an extra folder equal to another) are dropped.
pub fn mod_folders(app_dir: Option<&Path>, user_config_dir: &Path, extra: Option<&Path>) -> Vec<ModFolder> {
    mods::standard_folders(app_dir, user_config_dir, extra)
        .into_iter()
        .map(|f| ModFolder { label: f.label, exists: f.path.is_dir(), path: f.path })
        .collect()
}

impl ModList {
    /// Scans `folders` (earlier folders win when two mods share an id).
    pub fn scan(folders: Vec<ModFolder>, enabled: &[String]) -> ModList {
        let mut mods: Vec<ModEntry> = Vec::new();
        let mut problems = Vec::new();
        let mut broken = Vec::new();
        for (index, folder) in folders.iter().enumerate() {
            if !folder.exists {
                continue;
            }
            let (found, errors) = mods::scan(&folder.path);
            problems.extend(errors.iter().map(|e| e.to_string()));
            broken.extend(errors);
            for info in found {
                if let Some(first) = mods.iter().find(|m| m.info.manifest.id == info.manifest.id) {
                    problems.push(tf(
                        "Two mods are called {id}: {used} is used, {ignored} left out",
                        &[("id", &info.manifest.id), ("used", &first.info.path.display()), ("ignored", &info.path.display())],
                    ));
                    continue;
                }
                mods.push(ModEntry { info, folder: index });
            }
        }
        mods.sort_by_cached_key(|m| m.info.manifest.name.to_lowercase());
        let mut list = ModList { folders, mods, problems, broken, resolution: Resolution::Ok(Vec::new()), resolved_for: Vec::new(), resolved_in: 0 };
        list.resolve(enabled);
        list
    }

    pub fn get(&self, id: &str) -> Option<&ModEntry> {
        self.mods.iter().find(|m| m.id() == id)
    }

    /// Label of the folder `entry` was found in.
    pub fn folder_label(&self, entry: &ModEntry) -> &'static str {
        self.folders.get(entry.folder).map_or("", |f| f.label)
    }

    /// Whether the copy of `id` in use is the game folder's: it comes before the mods folder,
    /// where an update is installed (one there would change nothing).
    pub fn in_game_folder(&self, id: &str) -> bool {
        self.get(id).is_some_and(|m| self.folder_label(m) == APP_FOLDER_LABEL)
    }

    /// The version of `id` an update would replace: none when it is not installed, or when the
    /// copy in use is the game folder's ([`ModList::in_game_folder`]).
    pub fn updatable_version(&self, id: &str) -> Option<&str> {
        self.get(id).filter(|m| self.folder_label(m) != APP_FOLDER_LABEL).map(|m| m.info.manifest.version.as_str())
    }

    /// Recomputes the load order if `enabled` changed since the last call. After a change of the
    /// interface's language it scans again: the problems are told in it.
    pub fn refresh(&mut self, enabled: &[String]) {
        if self.resolved_in != ta_ui::i18n::generation() {
            *self = ModList::scan(std::mem::take(&mut self.folders), enabled);
        } else if self.resolved_for != enabled {
            self.resolve(enabled);
        }
    }

    fn resolve(&mut self, enabled: &[String]) {
        let infos: Vec<ModInfo> = self.mods.iter().map(|m| m.info.clone()).collect();
        self.resolution = match mods::resolve_load_order(enabled, &infos) {
            Ok(order) => Resolution::Ok(order.iter().map(|m| m.manifest.id.clone()).collect()),
            Err(e) => Resolution::Err(self.describe(&e)),
        };
        self.resolved_for = enabled.to_vec();
        self.resolved_in = ta_ui::i18n::generation();
    }

    /// A load order's problem in the player's words (and language), the mods by their names.
    fn describe(&self, e: &ModError) -> String {
        let name = |id: &str| self.get(id).map_or(id.to_string(), |m| m.info.manifest.name.clone());
        match e {
            ModError::Missing(id) => match mods::broken_manifest(id, &self.broken) {
                Some(ModError::Manifest { path, message }) => tf("{name} is on, but its manifest {file} does not read: {error}", &[("name", &name(id)), ("file", &mods::shown_path(path)), ("error", &message.trim())]),
                _ => tf("{name} is on, but it is not installed", &[("name", &name(id))]),
            },
            ModError::MissingDependency { module, dependency } => tf("{name} needs {other}, which is not on", &[("name", &name(module)), ("other", &name(dependency))]),
            ModError::Cycle(ids) => tf("These mods each need another loaded before them: {mods}", &[("mods", ids)]),
            ModError::Unreadable { id, message } => tf("{name} could not be read: {error}", &[("name", &name(id)), ("error", message)]),
            ModError::Manifest { .. } | ModError::Depth(_) | ModError::Duplicate { .. } => e.to_string(),
        }
    }

    /// Dependencies of `id` that are installed but not enabled (candidates for a quick fix).
    pub fn disabled_dependencies(&self, id: &str, enabled: &[String]) -> Vec<String> {
        self.get(id)
            .map(|m| {
                m.info
                    .manifest
                    .depends
                    .iter()
                    .filter(|dep| !enabled.contains(dep) && self.get(dep).is_some())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Enables (appends to the end of the load order) or disables a mod.
pub fn set_enabled(enabled: &mut Vec<String>, id: &str, on: bool) {
    let present = enabled.iter().any(|e| e == id);
    if on && !present {
        enabled.push(id.to_owned());
    } else if !on {
        enabled.retain(|e| e != id);
    }
}

/// Turns a catalogue mod on and the other catalogue mods off: each remakes the whole game.
/// Other mods (map packs, a player's own) stay as they are.
pub fn turn_on_conversion(enabled: &mut Vec<String>, id: &str) {
    if crate::catalogue::get(id).is_some() {
        enabled.retain(|e| e == id || crate::catalogue::get(e).is_none());
    }
    if !enabled.iter().any(|e| e == id) {
        enabled.push(id.to_string());
    }
}

/// The catalogue's mods other than `id` that are on: each remakes the whole game, so turning
/// on another asks first.
pub fn other_conversions(enabled: &[String], id: &str) -> Vec<String> {
    if crate::catalogue::get(id).is_none() { return Vec::new(); }
    enabled.iter().filter(|e| *e != id && crate::catalogue::get(e).is_some()).cloned().collect()
}

/// Moves the entry at `from` to `to` in the load order (the others closing up behind it).
pub fn move_to(enabled: &mut Vec<String>, from: usize, to: usize) {
    if from >= enabled.len() { return; }
    let id = enabled.remove(from);
    enabled.insert(to.min(enabled.len()), id);
}


#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn enable_disable_and_reorder() {
        let mut enabled = ids(&["a", "b"]);
        set_enabled(&mut enabled, "c", true);
        set_enabled(&mut enabled, "a", true);
        assert_eq!(enabled, ids(&["a", "b", "c"]));
        move_to(&mut enabled, 2, 1);
        assert_eq!(enabled, ids(&["a", "c", "b"]));
        move_to(&mut enabled, 0, 1);
        assert_eq!(enabled, ids(&["c", "a", "b"]));
        set_enabled(&mut enabled, "a", false);
        assert_eq!(enabled, ids(&["c", "b"]));
    }

    #[test]
    fn moves_to_any_place_and_names_the_other_conversions() {
        let mut enabled = ids(&["a", "b", "c", "d"]);
        move_to(&mut enabled, 3, 0);
        assert_eq!(enabled, ids(&["d", "a", "b", "c"]));
        move_to(&mut enabled, 0, 9);
        assert_eq!(enabled, ids(&["a", "b", "c", "d"]), "past the end is the end");
        move_to(&mut enabled, 1, 2);
        assert_eq!(enabled, ids(&["a", "c", "b", "d"]));
        move_to(&mut enabled, 7, 0);
        assert_eq!(enabled, ids(&["a", "c", "b", "d"]), "out of range: ignored");
        let on = ids(&["maps_pack", "escalation", "totalmayhem"]);
        assert_eq!(other_conversions(&on, "totalmayhem"), ids(&["escalation"]));
        assert!(other_conversions(&on, "maps_pack").is_empty(), "a mod of the player's own asks nothing");
    }

    #[test]
    fn one_conversion_at_a_time() {
        let mut enabled = ids(&["maps_pack", "escalation"]);
        turn_on_conversion(&mut enabled, "totalmayhem");
        assert_eq!(enabled, ids(&["maps_pack", "totalmayhem"]), "other mods stay, other conversions go");
        turn_on_conversion(&mut enabled, "totalmayhem");
        assert_eq!(enabled.len(), 2);
    }

    #[test]
    fn playing_an_addon_keeps_the_conversion_it_extends() {
        let mut enabled = ids(&["escalation", "map-pack-test"]);
        turn_on_conversion(&mut enabled, "custom-addon");
        assert_eq!(enabled, ids(&["escalation", "map-pack-test", "custom-addon"]));
    }

    #[test]
    fn scans_folders_reports_problems_and_resolves() {
        let root = std::env::temp_dir().join(format!("ta_launcher_mods_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (one, two) = (root.join("one"), root.join("two"));
        for dir in [&one, &two] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::create_dir_all(one.join("base_units")).unwrap();
        std::fs::write(one.join("base_units/mod.toml"), "id = \"base_units\"\nname = \"Base Units\"\n").unwrap();
        std::fs::create_dir_all(one.join("big")).unwrap();
        std::fs::write(one.join("big/mod.toml"), "id = \"big\"\nname = \"Big\"\ndepends = [\"base_units\"]\n").unwrap();
        std::fs::create_dir_all(one.join("broken")).unwrap();
        std::fs::write(one.join("broken/mod.toml"), "this is not toml = = =").unwrap();
        std::fs::create_dir_all(two.join("big_copy")).unwrap();
        std::fs::write(two.join("big_copy/mod.toml"), "id = \"big\"\nname = \"Big again\"\n").unwrap();
        std::fs::write(two.join("Legacy.ufo"), b"").unwrap();

        let folders = mod_folders(None, &root, Some(&two));
        // The user folder is `<root>/mods`, which does not exist; `two` is the extra folder.
        let folders: Vec<ModFolder> =
            std::iter::once(ModFolder { label: "One", path: one.clone(), exists: true }).chain(folders).collect();
        let mut list = ModList::scan(folders, &ids(&["big"]));
        assert_eq!(list.mods.len(), 3, "base_units, big, legacy: {:?}", list.mods);
        assert!(list.get("legacy").is_some_and(|m| m.is_archive()));
        assert_eq!(list.problems.len(), 2, "broken manifest + duplicate id: {:?}", list.problems);
        assert!(matches!(list.resolution, Resolution::Err(_)), "dependency not enabled");
        assert_eq!(list.disabled_dependencies("big", &ids(&["big"])), ids(&["base_units"]));

        list.refresh(&ids(&["big", "base_units"]));
        assert_eq!(list.resolution, Resolution::Ok(ids(&["base_units", "big"])));
        list.refresh(&ids(&["base_units", "big", "gone"]));
        assert!(matches!(list.resolution, Resolution::Err(ref e) if e.contains("gone")));
        list.refresh(&ids(&["big"]));
        assert_eq!(list.resolution, Resolution::Err("Big needs Base Units, which is not on".into()), "named, not by id");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A mod that is on and whose manifest no longer reads is told as that, not as missing.
    #[test]
    fn a_mod_whose_manifest_does_not_read_is_told_so() {
        let root = std::env::temp_dir().join(format!("ta_launcher_mods_broken_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("mymod")).unwrap();
        std::fs::write(root.join("mymod/mod.toml"), "id = \"mymod\"\nname = \"Mine\"\nversion = 1.0\n").unwrap();
        let folders = vec![ModFolder { label: "User folder", path: root.clone(), exists: true }];
        let list = ModList::scan(folders, &ids(&["mymod"]));
        match &list.resolution {
            Resolution::Err(e) => assert!(e.contains("mod.toml") && e.contains("does not read"), "{e}"),
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(root);
    }

    /// The game folder's copy of a mod comes before the mods folder's: no update is offered for
    /// it (one would go to the mods folder, behind it).
    #[test]
    fn a_mod_in_the_game_folder_is_not_updated_from_the_mods_folder() {
        let root = std::env::temp_dir().join(format!("ta_launcher_mods_game_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (game, user) = (root.join("game"), root.join("user"));
        for (dir, id, version) in [(&game, "Shared", "1.0"), (&user, "shared", "0.9"), (&user, "mine", "2.0")] {
            std::fs::create_dir_all(dir.join(id)).unwrap();
            std::fs::write(dir.join(id).join("mod.toml"), format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\n")).unwrap();
        }
        let folders = vec![ModFolder { label: APP_FOLDER_LABEL, path: game.clone(), exists: true }, ModFolder { label: "User folder", path: user.clone(), exists: true }];
        let list = ModList::scan(folders, &[]);
        assert_eq!(list.get("shared").map(|m| m.info.manifest.version.as_str()), Some("1.0"), "the game folder's, whatever the case of its id");
        assert!(list.in_game_folder("shared") && !list.in_game_folder("mine"));
        assert_eq!((list.updatable_version("shared"), list.updatable_version("mine"), list.updatable_version("gone")), (None, Some("2.0"), None));
        let _ = std::fs::remove_dir_all(root);
    }
}
