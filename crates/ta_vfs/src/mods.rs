//! Mod discovery and load ordering.
//!
//! A mod is either
//! * a directory containing a `mod.toml` manifest plus loose files and/or TA archives, or
//! * a legacy TA archive (`.ufo`, `.hpi`, `.ccx`, `.gp3`, `.gpf`) or a directory without a
//!   manifest dropped into the mods folder — these get an implicit manifest so existing
//!   community content works unchanged.
//!
//! Mods are layered on top of the base game in the resolved load order; later mods win. A mod's
//! archive named like one of the install's replaces it ([`replaced_archives`]): [`mount`] builds
//! every VFS of the game's content, so that this holds wherever the game's data is read.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ARCHIVE_EXTENSIONS;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ModManifest {
    /// The folder it is installed in, on disks that do not tell case apart: kept in lower
    /// case, whatever case its `mod.toml` wrote it in, as its name makes it without one (one
    /// written "MyMod" was another mod to the settings and the catalogue than "mymod", and the
    /// same folder on the disk).
    #[serde(deserialize_with = "lower_case")]
    pub id: String,
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// Mods that must be present and are loaded before this one.
    #[serde(default, deserialize_with = "lower_case_list")]
    pub depends: Vec<String>,
    /// Mods that, if enabled, are loaded before this one.
    #[serde(default, deserialize_with = "lower_case_list")]
    pub load_after: Vec<String>,
    /// Where the mod came from (its page, e.g. on ModDB), when it was installed from there.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// Folders of the mod's own seen as TA's (`guim = "guis"`: see [`crate::Renames`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rename: BTreeMap<String, String>,
    /// Archives of the original game this mod takes the place of beyond its own archives (file
    /// names, in lower case): with it on, the install's archives of these names are not mounted,
    /// as copying the mod into TA's folder overwrote them. The installer writes the empty
    /// placeholders a package ships here (`replaces = ["btdata.ccx"]`) instead of copying them;
    /// an author may name archives a player was told to remove. Its own archives, named like the
    /// install's, replace those without being named here ([`replaced_archives`]).
    #[serde(default, deserialize_with = "lower_case_list", skip_serializing_if = "Vec::is_empty")]
    pub replaces: Vec<String>,
}

/// A mod that does not say its version has none: "" (shown as unknown), never a made-up one.
fn default_version() -> String {
    String::new()
}

/// A mod id as read, in lower case (see [`ModManifest::id`]).
fn lower_case<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    <String as serde::Deserialize>::deserialize(d).map(|id| id.to_lowercase())
}

/// Mod ids as read, in lower case.
fn lower_case_list<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    <Vec<String> as serde::Deserialize>::deserialize(d).map(|ids| ids.into_iter().map(|id| id.to_lowercase()).collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModKind {
    /// Directory with (or without) a manifest; mounted via `Vfs::mount_mod_dir`.
    Directory,
    /// A single TA archive; mounted via `Vfs::mount_archive`.
    Archive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModInfo {
    pub manifest: ModManifest,
    pub path: PathBuf,
    pub kind: ModKind,
}

#[derive(Debug, Error)]
pub enum ModError {
    #[error("mod '{0}' is enabled but was not found")]
    Missing(String),
    #[error("mod '{module}' depends on '{dependency}', which is not enabled")]
    MissingDependency { module: String, dependency: String },
    #[error("mod dependency cycle involving: {0}")]
    Cycle(String),
    #[error("mod dependency nesting exceeds 64 levels at: {0}")]
    Depth(String),
    #[error("invalid manifest {path}: {message}")]
    Manifest { path: PathBuf, message: String },
    /// A mod that is on but could not be read (its folder, or the archive it is): no mod is
    /// mounted then, as when the load order does not resolve.
    #[error("mod '{id}' could not be read: {message}")]
    Unreadable { id: String, message: String },
    /// A second mod of an id an earlier folder has (the earlier one is used).
    #[error("duplicate mod id '{id}' in {}: ignored", path.display())]
    Duplicate { id: String, path: PathBuf },
}

/// A manifest's place as a player is shown it: its mod's folder and its name (`mymod\mod.toml`).
pub fn shown_path(path: &Path) -> String {
    match (path.parent().and_then(Path::file_name), path.file_name()) {
        (Some(folder), Some(file)) => Path::new(folder).join(file).display().to_string(),
        _ => path.display().to_string(),
    }
}

/// Why the mod `id` is not found, when a scan's `problems` tell: a manifest that does not read,
/// in a folder of that name (or the only one that does not read). The game and the launcher say
/// so rather than "not installed".
pub fn broken_manifest<'a>(id: &str, problems: &'a [ModError]) -> Option<&'a ModError> {
    let broken: Vec<&ModError> = problems.iter().filter(|p| matches!(p, ModError::Manifest { .. })).collect();
    let folder_id = |path: &Path| {
        let folder = if path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("mod.toml")) { path.parent() } else { Some(path) };
        folder.and_then(Path::file_name).map(|n| n.to_string_lossy().to_lowercase())
    };
    broken.iter().copied().find(|p| matches!(p, ModError::Manifest { path, .. } if folder_id(path).as_deref() == Some(id)))
        .or_else(|| (broken.len() == 1).then(|| broken[0]))
}

/// Scans a mods folder. Invalid manifests are reported, not fatal to the scan.
pub fn scan(dir: &Path) -> (Vec<ModInfo>, Vec<ModError>) {
    let mut mods = Vec::new();
    let mut errors = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return (mods, errors) };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        // Installers publish with a directory swap. A scan in another process
        // must never mount the unpublished copy or let an old copy shadow the
        // live mod by sorting first, even when those copies have valid manifests.
        if path.is_dir() && install_work_directory(&path) { continue; }
        match read_mod(&path) {
            Ok(Some(m)) => mods.push(m),
            Ok(None) => {}
            Err(e) => errors.push(e),
        }
    }
    (mods, errors)
}

/// One entry of a mods folder, as the game reads it: a folder (its `mod.toml`, or a manifest
/// made up from its name), a TA archive, or nothing the game loads (`None`). A manifest that
/// does not read is an error: the game leaves that mod out.
pub fn read_mod(path: &Path) -> Result<Option<ModInfo>, ModError> {
    if path.is_dir() {
        let manifest_path = path.join("mod.toml");
        let present = match std::fs::symlink_metadata(&manifest_path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(ModError::Manifest { path: manifest_path, message: error.to_string() }),
        };
        let manifest = if present {
            let manifest = read_manifest(&manifest_path)
                .map_err(|e| e.to_string())
                .and_then(|s| toml::from_str::<ModManifest>(&s).map_err(|e| e.to_string()))
                .map_err(|message| ModError::Manifest { path: manifest_path.clone(), message })?;
            if !crate::package::safe_component(&manifest.id) {
                return Err(ModError::Manifest { path: manifest_path, message: "mod id must be a single ordinary directory name".into() });
            }
            manifest
        } else {
            // (A folder's whole name: "TA.Escalation" is not "TA", nor "Mod v1.2" "Mod v1".)
            let mut manifest = implicit_manifest(&path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
            manifest.rename = kept_renames(path, &ModKind::Directory);
            manifest
        };
        Ok(Some(ModInfo { manifest, path: path.to_path_buf(), kind: ModKind::Directory }))
    } else if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ARCHIVE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
    {
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let mut manifest = implicit_manifest(&stem);
        manifest.rename = kept_renames(path, &ModKind::Archive);
        Ok(Some(ModInfo { manifest, path: path.to_path_buf(), kind: ModKind::Archive }))
    } else {
        Ok(None)
    }
}

pub(crate) fn install_work_directory(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else { return false; };
    // On Windows these suffix spellings identify the same directories as those
    // created by the installer. Other dotted or hidden mod folders stay usable.
    #[cfg(windows)]
    let name = name.to_ascii_lowercase();
    name.strip_prefix('.')
        .and_then(|name| name.strip_suffix(".installing").or_else(|| name.strip_suffix(".old")))
        .is_some_and(crate::package::safe_component)
}

fn read_manifest(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take((1 << 20) + 1).read_to_string(&mut text)?;
    if text.len() > 1 << 20 { return Err(std::io::Error::other("mod manifest exceeds 1 MiB")); }
    Ok(text)
}

/// The folders of its own a mod without a manifest uses in place of TA's ([`ModManifest::rename`]),
/// read from what it holds as the installer reads a package's (`package::detected_renames`): a mod
/// made for a patched engine (TA: Escalation's `unitse` and `guie`, Total Mayhem's `guim` and
/// `weaponm`) dropped into a mods folder by hand plays as it does installed from its package.
/// (Without them its units, menus and weapons were never read: TA: Escalation was the original
/// game, Total Mayhem its units with TA's menus and no weapons of their own.)
fn implicit_renames(path: &Path, kind: &ModKind) -> BTreeMap<String, String> {
    let mut contents = crate::package::Contents::new();
    let mut add = |more: crate::package::Contents| {
        for (top, kinds) in more {
            let into = contents.entry(top).or_default();
            for (kind, n) in kinds {
                *into.entry(kind).or_default() += n;
            }
        }
    };
    match kind {
        ModKind::Archive => add(crate::package::archive_contents(path).unwrap_or_default()),
        ModKind::Directory => {
            for archive in crate::archive_files(path).unwrap_or_default() {
                add(crate::package::archive_contents(&archive).unwrap_or_default());
            }
            let mut loose = Vec::new();
            if crate::collect_files(path, path, true, &mut loose).is_ok() {
                let mut found = crate::package::Contents::new();
                for file in loose {
                    crate::package::count_content(&mut found, &crate::normalize(&file.to_string_lossy()));
                }
                add(found);
            }
        }
    }
    crate::package::detected_renames(&contents, &BTreeSet::new())
}

/// A stamp of what [`implicit_renames`] reads: an archive's size and time; a folder's time, and
/// the name, size and time of every file below it, as it reads them all (a file renamed deep in
/// one of its folders changes what it holds, and neither the folder nor those above it).
type RenameStamp = Vec<(String, u64, Option<std::time::SystemTime>)>;

/// Files a folder's stamp takes in at most: beyond, it is read as it is, every time.
const MOST_STAMPED_FILES: usize = 200_000;

/// What [`implicit_renames`] found, by path, with the stamp of what it read then. A scan runs at
/// every visit of the launcher's Mods and Maps pages, on its interface thread: opening every
/// archive and walking every loose file of each mod without a manifest each time stalled it (a
/// mods folder of map packs).
static RENAMES: std::sync::Mutex<KeptRenames> = std::sync::Mutex::new(BTreeMap::new());
type KeptRenames = BTreeMap<PathBuf, (RenameStamp, BTreeMap<String, String>)>;

fn rename_stamp(path: &Path, kind: &ModKind) -> Option<RenameStamp> {
    let root = std::fs::metadata(path).ok()?;
    let mut stamp = vec![(String::new(), root.len(), root.modified().ok())];
    if *kind == ModKind::Directory {
        // (Its folders walked, links left aside as the game leaves them, each file stamped from
        // its folder's listing: no file opened, no archive read.)
        let mut files = Vec::new();
        let mut pending = vec![path.to_path_buf()];
        while let Some(dir) = pending.pop() {
            for entry in std::fs::read_dir(&dir).ok()?.flatten() {
                let Ok(kind) = entry.file_type() else { continue };
                if kind.is_dir() {
                    pending.push(entry.path());
                } else if kind.is_file() && let Ok(meta) = entry.metadata() {
                    let at = entry.path();
                    let name = at.strip_prefix(path).unwrap_or(&at).to_string_lossy().into_owned();
                    files.push((name, meta.len(), meta.modified().ok()));
                    if files.len() > MOST_STAMPED_FILES { return None; }
                }
            }
        }
        files.sort();
        stamp.extend(files);
    }
    Some(stamp)
}

/// [`implicit_renames`], read again only when what it reads changed ([`RENAMES`]).
fn kept_renames(path: &Path, kind: &ModKind) -> BTreeMap<String, String> {
    let Some(stamp) = rename_stamp(path, kind) else { return implicit_renames(path, kind) };
    if let Ok(kept) = RENAMES.lock()
        && let Some((was, renames)) = kept.get(path)
        && *was == stamp
    {
        return renames.clone();
    }
    let renames = implicit_renames(path, kind);
    if let Ok(mut kept) = RENAMES.lock() {
        // (Bounded: mods come and go.)
        if kept.len() >= 4096 { kept.clear(); }
        kept.insert(path.to_path_buf(), (stamp, renames.clone()));
    }
    renames
}

fn implicit_manifest(stem: &str) -> ModManifest {
    ModManifest {
        // (In lower case as a manifest's id and the settings' are: "Übermacht" is
        // "übermacht" to them, so it must be here too, or it could never be turned on.)
        id: stem.to_lowercase(),
        name: stem.to_string(),
        version: default_version(),
        description: "Legacy Total Annihilation content".into(),
        authors: Vec::new(),
        depends: Vec::new(),
        load_after: Vec::new(),
        source: String::new(),
        rename: BTreeMap::new(),
        replaces: Vec::new(),
    }
}

/// What the installation of a mod TA Reborn knows (by its catalogue id) took out of the original
/// game beyond what its files say: the empty placeholders installers before this rule left out
/// (they were not kept, nor written in `mod.toml`), and archives its instructions had the player
/// remove. Applied whatever its manifest says ([`replaces`]), so a mod installed before needs no
/// reinstalling.
const KNOWN: &[(&str, &[&str])] = &[
    // TA: Enhanced (Alpha 0.8v, `TA_Enhanced_Alpha_0_8v.rar`): its package's `btdata.ccx` holds
    // nothing, and its readme has it overwrite Battle Tactics' ("IT IS ABSOLUTELY REQUIRED TO
    // OVERRIDE THE THREE FILES KNOWN AS TOTALA1.HPI, CCDATA.CCX AND BTDATA.CCX"), the six
    // post-release units removed ("MAKE SURE YOU'RE NOT USING ANY OF THE SIX CAVEDOG POST-RELEASE
    // UNITS": its own data defines four of them and leaves the Scarab and Hedgehog out), and the
    // free Core Contingency and Battle Tactics unit packs too ("they will strongly mess up the
    // mod").
    ("taenhanced", &[
        "btdata.ccx", "afark.ufo", "aflea.ufo", "ascarab.ufo", "cornecro.ufo", "cormabm.ufo", "corplas.ufo",
        "ccunits.ccx", "ccfeatures.ccx", "btunits.ccx",
    ]),
];

/// The archive names (file names, in lower case) a mod takes the place of in the install: each
/// archive of its own (at the top of its folder, or the archive it is, whatever it holds: an empty
/// placeholder blanks the original's), those its manifest names ([`ModManifest::replaces`]) and
/// those its known installation removed ([`KNOWN`]).
pub fn replaces(m: &ModInfo) -> BTreeSet<String> {
    let own: Vec<String> = match m.kind {
        ModKind::Directory => crate::archive_files(&m.path).unwrap_or_default().iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().to_lowercase()).collect(),
        ModKind::Archive => m.path.file_name().map(|n| n.to_string_lossy().to_lowercase()).into_iter().collect(),
    };
    own.into_iter().chain(declared(m)).collect()
}

/// What a mod says it replaces beyond its own archives: its manifest's `replaces` and its known
/// installation's ([`KNOWN`]), in lower case (a name with a folder in it is no archive of the
/// install's top, and never matches one).
fn declared(m: &ModInfo) -> BTreeSet<String> {
    let known = KNOWN.iter().filter(|(id, _)| *id == m.manifest.id).flat_map(|(_, names)| names.iter().map(|n| n.to_string()));
    m.manifest.replaces.iter().map(|n| n.trim().to_lowercase()).chain(known)
        .filter(|n| !n.is_empty() && !n.contains(['/', '\\']))
        .collect()
}

/// The install's archives (`install`'s, as named there, sorted whatever their case) the mods
/// replace: those named, in any case, like one a mod replaces ([`replaces`]). Mounted with the
/// mods on, the install leaves them out ([`mount`]).
pub fn replaced_archives(install: &Path, mods: &[&ModInfo]) -> Vec<String> {
    let names: BTreeSet<String> = mods.iter().flat_map(|m| replaces(m)).collect();
    if names.is_empty() {
        return Vec::new();
    }
    // (A mod's archives in the game's folder are never the install's, [`mount`]: none of them is
    // one a mod takes the place of. TA: Escalation added from there names its own `TAESC.gp3`.)
    let theirs: BTreeSet<PathBuf> = game_folder_mods(install).into_iter().flat_map(|m| m.archives).collect();
    let mut out: Vec<String> = crate::archive_files(install).unwrap_or_default().iter()
        .filter(|p| !theirs.contains(*p))
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .filter(|name| names.contains(&name.to_lowercase()))
        .collect();
    out.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)));
    out
}

/// A mod the original game's folder holds, put there as the mod's own installer had a player put
/// it (its archives beside TA's), made for an engine of its own: its archives use folders of the
/// mod's own in place of TA's ([`ModManifest::rename`]), which TA's engine never read. Mounted
/// with the original game, it played neither: TA: Escalation's models and scripts took the place of
/// TA's own units' with nothing of Escalation's own, Total Mayhem's units came without their menus
/// or weapons. Its archives are never the original game's ([`mount`]); the launcher adds it as a
/// mod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameFolderMod {
    /// The id the catalogue knows it by (`escalation`), else its main archive's name.
    pub id: String,
    pub name: String,
    /// Its archives in the game's folder, sorted.
    pub archives: Vec<PathBuf>,
    pub rename: BTreeMap<String, String>,
}

/// The longest mod id the launcher installs, in letters.
pub const MOST_ID_LETTERS: usize = 64;

/// Mods TA Reborn knows by the names of the archives their installers put in the game's folder (in
/// lower case): the first made for the mod's own engine, which says the mod is there; with it, the
/// others are the mod's whatever they hold (TA: Escalation's `t4esc1.ufo` holds only sounds and
/// models). Their ids are the catalogue's.
const KNOWN_ARCHIVES: &[(&str, &str, &[&str])] = &[
    ("escalation", "TA: Escalation", &["taesc.gp3", "t2esc.ufo", "t3esc.ufo", "t4esc1.ufo", "t4esc2.ufo", "t5esc.ufo", "txesc.ufo", "tademo.ufo"]),
    ("totalmayhem", "Total Mayhem", &["mayhem.gp3", "tademom.ufo"]),
];

/// The mods made for other engines in the original game's folder `install` ([`GameFolderMod`]):
/// those TA Reborn knows by name ([`KNOWN_ARCHIVES`]), then each other archive with folders of its
/// own in place of TA's, with those sharing one of them (a mod's several archives). The archives
/// of the original game's releases are never one's ([`crate::install::ARCHIVES`]), nor archives
/// with TA's folders alone: unit packs and maps TA's own engine read too.
pub fn game_folder_mods(install: &Path) -> Vec<GameFolderMod> {
    // (It opens every archive in the folder that is not the original game's, a map pack's too,
    // and is asked at every mount of the game, every visit of the launcher's Mods and Maps pages:
    // kept, while the folder's archives are those it found, each as large and as old.)
    type Key = Vec<(PathBuf, u64, Option<std::time::SystemTime>)>;
    static KEPT: std::sync::Mutex<Option<(PathBuf, Key, Vec<GameFolderMod>)>> = std::sync::Mutex::new(None);
    let key: Key = crate::archive_files(install)
        .unwrap_or_default()
        .into_iter()
        .map(|path| {
            let meta = std::fs::metadata(&path).ok();
            let (len, modified) = meta.map_or((0, None), |m| (m.len(), m.modified().ok()));
            (path, len, modified)
        })
        .collect();
    if let Some((dir, kept_key, mods)) = KEPT.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        && dir == install
        && *kept_key == key
    {
        return mods.clone();
    }
    let mods = scan_game_folder(install);
    *KEPT.lock().unwrap_or_else(|e| e.into_inner()) = Some((install.to_path_buf(), key, mods.clone()));
    mods
}

/// [`game_folder_mods`], worked out.
fn scan_game_folder(install: &Path) -> Vec<GameFolderMod> {
    struct Found {
        path: PathBuf,
        name: String,
        rename: BTreeMap<String, String>,
        size: u64,
    }
    let mut found: Vec<Found> = crate::archive_files(install)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|path| {
            let name = path.file_name()?.to_string_lossy().to_lowercase();
            if crate::install::ARCHIVES.contains(&name.as_str()) {
                return None;
            }
            let rename = crate::package::detected_renames(&crate::package::archive_contents(&path)?, &BTreeSet::new());
            let size = std::fs::metadata(&path).map_or(0, |m| m.len());
            Some(Found { path, name, rename, size })
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
    let mut mods = Vec::new();
    let mut taken = BTreeSet::new();
    for (id, name, archives) in KNOWN_ARCHIVES {
        if !found.iter().any(|f| f.name == archives[0] && !f.rename.is_empty()) {
            continue;
        }
        let own: Vec<&Found> = found.iter().filter(|f| archives.contains(&f.name.as_str())).collect();
        taken.extend(own.iter().map(|f| f.name.clone()));
        mods.push(GameFolderMod {
            id: id.to_string(),
            name: name.to_string(),
            archives: own.iter().map(|f| f.path.clone()).collect(),
            rename: own.iter().flat_map(|f| f.rename.clone()).collect(),
        });
    }
    // The others: archives sharing a folder of their own are one mod's.
    let mut groups: Vec<Vec<&Found>> = Vec::new();
    for f in found.iter().filter(|f| !f.rename.is_empty() && !taken.contains(&f.name)) {
        let mut group = vec![f];
        let mut i = groups.len();
        while i > 0 {
            i -= 1;
            if groups[i].iter().any(|other| other.rename.keys().any(|folder| f.rename.contains_key(folder))) {
                group.extend(groups.remove(i));
            }
        }
        groups.push(group);
    }
    for mut group in groups {
        group.sort_by(|a, b| a.name.cmp(&b.name));
        let main = group.iter().max_by(|a, b| a.size.cmp(&b.size).then_with(|| b.name.cmp(&a.name))).expect("a group has an archive");
        let name = main.path.file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
        // (An id the installer takes, however long the archive's name: room left for a "-2".)
        let short: String = name.to_lowercase().chars().take(MOST_ID_LETTERS - 4).collect();
        let base = Some(short.trim_end_matches(['.', ' ']).to_string()).filter(|id| crate::package::safe_component(id)).unwrap_or_else(|| "game-folder-mod".to_string());
        let id = (1..).map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") }).find(|id| !mods.iter().any(|m: &GameFolderMod| m.id == *id)).expect("a free id");
        mods.push(GameFolderMod { id, name, archives: group.iter().map(|f| f.path.clone()).collect(), rename: group.iter().flat_map(|f| f.rename.clone()).collect() });
    }
    mods
}

/// The game's content as [`mount`] mounted it, and what of it could not be.
pub struct Mounted {
    pub vfs: crate::Vfs,
    /// The original game's install, mounted (as `"base"`) or why not: nothing of it is then.
    pub install: crate::Result<()>,
    /// TA Reborn's own content, mounted (as [`crate::BUILTIN_LAYER`]) or why not.
    pub builtin: crate::Result<()>,
    /// The mods mounted, their ids in load order; or why none is (one did not read, or the load
    /// order does not resolve): the install is then mounted whole.
    pub mods: Result<Vec<String>, ModError>,
    /// The install's archives left out, as named there: the mods mounted replace them.
    pub replaced: Vec<String>,
    /// The mods made for other engines in the install's folder ([`game_folder_mods`]), whose
    /// archives are left out of the original game too.
    pub game_folder_mods: Vec<GameFolderMod>,
}

/// Mounts the game's content as the game does: the original game's install at `install`, less the
/// archives the mods replace ([`replaced_archives`]); TA Reborn's own content (`builtin`, over the
/// install); then the mods, in the order given (their load order), each as the layer `layer` names
/// it, all of them or none: each is read first, and one that does not read ([`ModError::Unreadable`]:
/// its folder, or the archive it is) leaves every mod out and the install whole, as a load order
/// that does not resolve does (the mods that need it would run without it; the rest, without the
/// player knowing). Every VFS of the game's content is made here (the game, the map editor's
/// remount, the launcher's checks, previews and maps, `ta-tools`, the tests), so that a mod
/// replaces the install's archives wherever the game's data is read.
pub fn mount(install: Option<&Path>, builtin: Option<&Path>, mods: &[&ModInfo], layer: &dyn Fn(&ModInfo) -> String) -> Mounted {
    let mut layers = Vec::with_capacity(mods.len());
    let mut problem = None;
    for m in mods {
        let renames = crate::Renames::new(&m.manifest.rename);
        let name = layer(m);
        let read = match m.kind {
            ModKind::Directory => crate::Layer::mod_dir(&m.path, &name, &renames),
            ModKind::Archive => crate::Layer::archive(&m.path, &name, &renames),
        };
        match read {
            Ok(read) => layers.push(read),
            Err(e) => {
                // (Its folders by the mod's own name: the player knows where their mods are.)
                let shown = m.path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                problem = Some(ModError::Unreadable { id: m.manifest.id.clone(), message: e.to_string().replace(&m.path.display().to_string(), &shown) });
                break;
            }
        }
    }
    let on: &[&ModInfo] = if problem.is_none() { mods } else { &[] };
    let replaced = install.map(|dir| replaced_archives(dir, on)).unwrap_or_default();
    // (Mods made for other engines in the game's folder are never the original game's.)
    let game_folder = install.map(game_folder_mods).unwrap_or_default();
    let mut left_out = replaced.clone();
    for m in &game_folder {
        let names: Vec<String> = m.archives.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
        log::info!("{} is in the game's folder, made for its own engine: its archives are left out of the original game ({})", m.name, names.join(", "));
        left_out.extend(names.into_iter().filter(|n| !replaced.iter().any(|r| r.eq_ignore_ascii_case(n))));
    }
    let mut vfs = crate::Vfs::new();
    let install = install.map_or(Ok(()), |dir| vfs.mount_install(dir, "base", &left_out));
    let builtin = builtin.map_or(Ok(()), |dir| vfs.mount_mod_dir(dir, crate::BUILTIN_LAYER));
    let mods = match problem {
        Some(problem) => Err(problem),
        None => {
            for read in layers {
                vfs.mount_layer(read);
            }
            Ok(mods.iter().map(|m| m.manifest.id.clone()).collect())
        }
    };
    Mounted { vfs, install, builtin, mods, replaced, game_folder_mods: game_folder }
}

/// [`mount`] with the mods `enabled` (found among `available`) in their load order
/// ([`resolve_load_order`]), each mounted as its id: what the game plays with. A load order that
/// does not resolve mounts no mod.
pub fn mount_enabled(install: Option<&Path>, builtin: Option<&Path>, enabled: &[String], available: &[ModInfo]) -> Mounted {
    match resolve_load_order(enabled, available) {
        Ok(order) => mount(install, builtin, &order, &|m| m.manifest.id.clone()),
        Err(e) => {
            let mut mounted = mount(install, builtin, &[], &|m| m.manifest.id.clone());
            mounted.mods = Err(e);
            mounted
        }
    }
}

/// Label of the mods folder in TA Reborn's own program folder, next to the executables (not
/// Total Annihilation's folder, which players call the game folder).
pub const APP_FOLDER_LABEL: &str = "Program folder";
/// Label of the mods folder among the player's settings, where mods are installed.
pub const USER_FOLDER_LABEL: &str = "User folder";
/// Label of the extra mods folder a player may add.
pub const EXTRA_FOLDER_LABEL: &str = "Extra folder";

/// A place mods are looked for, with a short label for user interfaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModFolder {
    pub label: &'static str,
    pub path: PathBuf,
}

/// Standard mod folders: next to the executable, in the user's config folder, and an
/// optional extra folder. Duplicates (e.g. an extra folder equal to another) are dropped;
/// earlier folders win when two mods share an id. Shared by the game and the launcher.
pub fn standard_folders(app_dir: Option<&Path>, user_config_dir: &Path, extra: Option<&Path>) -> Vec<ModFolder> {
    let mut candidates: Vec<ModFolder> = Vec::new();
    if let Some(dir) = app_dir {
        let resources = crate::bundle_resource_dir(dir).unwrap_or_else(|| dir.to_path_buf());
        candidates.push(ModFolder { label: APP_FOLDER_LABEL, path: resources.join("mods") });
    }
    candidates.push(ModFolder { label: USER_FOLDER_LABEL, path: user_config_dir.join("mods") });
    if let Some(extra) = extra {
        candidates.push(ModFolder { label: EXTRA_FOLDER_LABEL, path: extra.to_path_buf() });
    }
    let mut seen = std::collections::BTreeSet::new();
    candidates.into_iter().filter(|f| seen.insert(folder_key(&f.path))).collect()
}

/// Paths of [`standard_folders`].
pub fn default_folders(app_dir: Option<&Path>, user_config_dir: &Path, extra: Option<&Path>) -> Vec<PathBuf> {
    standard_folders(app_dir, user_config_dir, extra).into_iter().map(|f| f.path).collect()
}

/// Comparison key for folders: canonical when possible, case-insensitive on Windows.
fn folder_key(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = path.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
    if cfg!(windows) { text.to_lowercase() } else { text }
}

/// Scans several folders; a mod id seen in an earlier folder shadows later ones. The problems
/// are manifests that do not read and the ids found twice ([`ModError::Duplicate`]).
pub fn scan_folders(folders: &[PathBuf]) -> (Vec<ModInfo>, Vec<ModError>) {
    let mut mods: Vec<ModInfo> = Vec::new();
    let mut problems = Vec::new();
    for folder in folders.iter().filter(|f| f.is_dir()) {
        let (found, errors) = scan(folder);
        problems.extend(errors);
        for m in found {
            if mods.iter().any(|x| x.manifest.id == m.manifest.id) {
                problems.push(ModError::Duplicate { id: m.manifest.id.clone(), path: m.path.clone() });
            } else {
                mods.push(m);
            }
        }
    }
    (mods, problems)
}

/// Orders the enabled mods so dependencies load first; otherwise the user's order is kept. A
/// mod's wish to load after another (`load_after`) gives way where it would make a cycle (with
/// what that one needs, or an earlier wish): only dependencies make a cycle an error.
pub fn resolve_load_order<'a>(enabled: &[String], available: &'a [ModInfo]) -> Result<Vec<&'a ModInfo>, ModError> {
    let by_id: BTreeMap<&str, &ModInfo> = available.iter().map(|m| (m.manifest.id.as_str(), m)).collect();
    let enabled_set: BTreeSet<&str> = enabled.iter().map(|s| s.as_str()).collect();
    for id in enabled {
        let m = by_id.get(id.as_str()).ok_or_else(|| ModError::Missing(id.clone()))?;
        for dep in &m.manifest.depends {
            if !enabled_set.contains(dep.as_str()) {
                return Err(ModError::MissingDependency { module: id.clone(), dependency: dep.clone() });
            }
        }
    }

    // What loads before each mod: its dependencies, then the mods it wishes to follow.
    let mut before: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for id in enabled {
        let m = by_id[id.as_str()];
        before.entry(m.manifest.id.as_str()).or_insert_with(|| m.manifest.depends.iter().map(String::as_str).collect());
    }
    for id in enabled {
        let m = by_id[id.as_str()];
        let me = m.manifest.id.as_str();
        for wish in m.manifest.load_after.iter().map(String::as_str) {
            if enabled_set.contains(wish) && wish != me && !before[me].contains(&wish) && !comes_after(&before, wish, me) {
                before.get_mut(me).expect("every enabled mod").push(wish);
            }
        }
    }

    // Depth-first topological sort that visits mods in user order.
    let mut order = Vec::new();
    let mut state: BTreeMap<&str, u8> = BTreeMap::new(); // 1 = visiting, 2 = done
    fn visit<'a>(
        id: &'a str,
        by_id: &BTreeMap<&str, &'a ModInfo>,
        before: &BTreeMap<&str, Vec<&'a str>>,
        state: &mut BTreeMap<&'a str, u8>,
        order: &mut Vec<&'a ModInfo>,
        depth: usize,
    ) -> Result<(), ModError> {
        match state.get(id) {
            Some(2) => return Ok(()),
            Some(1) => return Err(ModError::Cycle(id.to_string())),
            _ => {}
        }
        if depth > 64 { return Err(ModError::Depth(id.to_string())); }
        let m = by_id[id];
        state.insert(m.manifest.id.as_str(), 1);
        for &dep in before.get(id).into_iter().flatten() {
            visit(by_id[dep].manifest.id.as_str(), by_id, before, state, order, depth + 1)?;
        }
        state.insert(m.manifest.id.as_str(), 2);
        order.push(m);
        Ok(())
    }
    for id in enabled {
        visit(by_id[id.as_str()].manifest.id.as_str(), &by_id, &before, &mut state, &mut order, 0)?;
    }
    Ok(order)
}

/// The digest saved games and replays keep of a mod they were made with (`ta_config`'s
/// `RecordMod`): of the files the game mounts from it as installed (a folder's archives and loose
/// files at any depth, but its `mod.toml` and links; an archive mod's archive), by path (in lower
/// case, `/` between folders), size and content, of the folders its manifest renames, and of the
/// original game's archives it replaces, when it replaces any (its archives of their names, those it
/// says it replaces: [`replaced_claims`]; a mod that replaces none has the digest it had before
/// archives were replaced). An update or an edited file gives another; the same files, wherever
/// installed, the same. (The first 16 bytes of a SHA-256: a few hundred megabytes of mod take a
/// fraction of a second.)
pub fn content_digest(m: &ModInfo) -> std::io::Result<[u8; 16]> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    match m.kind {
        ModKind::Directory => mod_files(&m.path, &m.path, 0, &mut files)?,
        ModKind::Archive => files.push((crate::normalize(&m.path.file_name().unwrap_or_default().to_string_lossy()), m.path.clone())),
    }
    files.sort();
    let mut hash = Sha256::new();
    hash.update(b"TA Reborn mod content v1\0");
    let feed = |hash: &mut Sha256, text: &str| {
        hash.update((text.len() as u64).to_le_bytes());
        hash.update(text.as_bytes());
    };
    hash.update((m.manifest.rename.len() as u64).to_le_bytes());
    for (from, to) in &m.manifest.rename {
        feed(&mut hash, &crate::normalize(from));
        feed(&mut hash, &crate::normalize(to));
    }
    let replaced = replaced_claims(m);
    if !replaced.is_empty() {
        hash.update(b"replaces\0");
        hash.update((replaced.len() as u64).to_le_bytes());
        for name in &replaced {
            feed(&mut hash, name);
        }
    }
    hash.update((files.len() as u64).to_le_bytes());
    let mut buffer = vec![0u8; 1 << 20];
    for (name, path) in files {
        let mut file = std::fs::File::open(&path)?;
        let mut content = Sha256::new();
        let mut len = 0u64;
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 { break; }
            len += n as u64;
            content.update(&buffer[..n]);
        }
        feed(&mut hash, &name);
        hash.update(len.to_le_bytes());
        hash.update(content.finalize());
    }
    let digest = hash.finalize();
    Ok(digest[..16].try_into().expect("SHA-256 has 16 bytes"))
}

/// What of the original game a mod replaces wherever it is installed, for its digest: its
/// archives named like one the original game's releases install ([`crate::install::ARCHIVES`]),
/// and what it says it replaces ([`declared`]). (Which the install on a computer has is that
/// computer's: a digest is the mod's.)
pub fn replaced_claims(m: &ModInfo) -> BTreeSet<String> {
    let mut claims = declared(m);
    claims.extend(replaces(m).into_iter().filter(|name| crate::install::ARCHIVES.contains(&name.as_str())));
    claims
}

/// The files of a mod's folder the game mounts (see [`content_digest`]): relative to `root`,
/// normalised, with their paths.
fn mod_files(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) -> std::io::Result<()> {
    if depth > 64 { return Err(std::io::Error::other("mod content nesting exceeds 64 levels")); }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let meta = std::fs::symlink_metadata(&path)?;
        if crate::is_link(&meta) { continue; }
        let relative = crate::normalize(&path.strip_prefix(root).unwrap_or(&path).to_string_lossy());
        if meta.is_dir() {
            mod_files(root, &path, depth + 1, out)?;
        } else if meta.is_file() && relative != "mod.toml" {
            out.push((relative, path));
        }
    }
    Ok(())
}

/// Whether `from` loads after `to`, by what loads before it, however indirectly.
fn comes_after(before: &BTreeMap<&str, Vec<&str>>, from: &str, to: &str) -> bool {
    let mut seen = BTreeSet::new();
    let mut pending = vec![from];
    while let Some(id) = pending.pop() {
        if id == to { return true; }
        if seen.insert(id) { pending.extend(before.get(id).into_iter().flatten().copied()); }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_path_that_is_a_directory_cannot_become_an_implicit_mod() {
        let root = std::env::temp_dir().join(format!("ta-mod-qa-manifest-dir-{}", std::process::id()));
        std::fs::create_dir_all(root.join("mod.toml")).unwrap();
        assert!(matches!(read_mod(&root), Err(ModError::Manifest { .. })), "present metadata must read, or discovery must report it");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsafe_declared_mod_ids_are_reported_during_discovery() {
        let root = std::env::temp_dir().join(format!("ta-mod-qa-manifest-id-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for id in ["", "../other", "nested/name", "NUL"] {
            std::fs::write(root.join("mod.toml"), format!("id = {id:?}\nname = \"Test\"\n")).unwrap();
            assert!(matches!(read_mod(&root), Err(ModError::Manifest { .. })), "{id:?}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    fn m(id: &str, depends: &[&str], after: &[&str]) -> ModInfo {
        let mut manifest = implicit_manifest(id);
        manifest.depends = depends.iter().map(|s| s.to_string()).collect();
        manifest.load_after = after.iter().map(|s| s.to_string()).collect();
        ModInfo { manifest, path: PathBuf::new(), kind: ModKind::Directory }
    }

    #[test]
    fn dependencies_load_first() {
        let mods = vec![m("a", &["b"], &[]), m("b", &[], &[]), m("c", &[], &["a"])];
        let enabled = vec!["c".to_string(), "a".to_string(), "b".to_string()];
        let order: Vec<&str> =
            resolve_load_order(&enabled, &mods).unwrap().iter().map(|m| m.manifest.id.as_str()).collect();
        assert_eq!(order, ["b", "a", "c"]);
    }

    #[test]
    fn detects_missing_dependency_and_cycles() {
        let mods = vec![m("a", &["b"], &[]), m("b", &[], &[])];
        assert!(matches!(
            resolve_load_order(&["a".to_string()], &mods),
            Err(ModError::MissingDependency { .. })
        ));
        let cyclic = vec![m("a", &["b"], &[]), m("b", &["a"], &[])];
        assert!(matches!(
            resolve_load_order(&["a".to_string(), "b".to_string()], &cyclic),
            Err(ModError::Cycle(_))
        ));
    }

    #[test]
    fn a_wish_to_load_after_gives_way_to_a_cycle() {
        let ids = |order: Vec<&ModInfo>| order.iter().map(|m| m.manifest.id.clone()).collect::<Vec<_>>();
        let enabled = vec!["a".to_string(), "b".to_string()];
        // Each wishes to follow the other: the first one's wish is kept.
        let mods = vec![m("a", &[], &["b"]), m("b", &[], &["a"])];
        assert_eq!(ids(resolve_load_order(&enabled, &mods).unwrap()), ["b", "a"]);
        // b needs a: a's wish to follow b gives way.
        let mods = vec![m("a", &[], &["b"]), m("b", &["a"], &[])];
        assert_eq!(ids(resolve_load_order(&enabled, &mods).unwrap()), ["a", "b"]);
        // Round three mods: the last wish closing the circle gives way.
        let mods = vec![m("a", &[], &["c"]), m("b", &[], &["a"]), m("c", &[], &["b"])];
        let three = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(ids(resolve_load_order(&three, &mods).unwrap()), ["c", "a", "b"]);
    }

    /// A mod's digest tells another copy from the same: an edited file, a file more, renamed
    /// folders; not where it is installed, its manifest's words, or its files' case.
    #[test]
    fn a_mods_digest_follows_its_files_not_where_or_how_it_is_named() {
        let root = std::env::temp_dir().join(format!("ta_mod_digest_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let make = |name: &str, unit: &str| {
            let dir = root.join(name);
            std::fs::create_dir_all(dir.join("units")).unwrap();
            std::fs::write(dir.join("mod.toml"), format!("id = \"{name}\"\nname = \"{name}\"\nversion = \"1\"\n")).unwrap();
            std::fs::write(dir.join("units/tank.fbi"), unit).unwrap();
            std::fs::write(dir.join("pack.ufo"), b"archive bytes").unwrap();
            read_mod(&dir).unwrap().unwrap()
        };
        let a = make("a", "[UNITINFO]{MaxDamage=100;}");
        let same = make("b", "[UNITINFO]{MaxDamage=100;}");
        let digest = content_digest(&a).unwrap();
        assert_eq!(digest, content_digest(&same).unwrap(), "the same files elsewhere, under another manifest");
        std::fs::rename(same.path.join("units/tank.fbi"), same.path.join("units/TANK.FBI")).unwrap();
        assert_eq!(digest, content_digest(&same).unwrap(), "the game reads names in any case");
        let edited = make("c", "[UNITINFO]{MaxDamage=900;}");
        assert_ne!(digest, content_digest(&edited).unwrap(), "a unit changed");
        std::fs::write(a.path.join("readme.txt"), b"notes").unwrap();
        let more = content_digest(&a).unwrap();
        assert_ne!(digest, more, "a file more");
        let mut renamed = a.clone();
        renamed.manifest.rename.insert("unitsm".into(), "units".into());
        assert_ne!(more, content_digest(&renamed).unwrap(), "its folders seen as others");
        let archive = ModInfo { manifest: implicit_manifest("pack"), path: a.path.join("pack.ufo"), kind: ModKind::Archive };
        assert_ne!(content_digest(&archive).unwrap(), [0; 16]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn one_entry_is_read_as_the_scan_reads_it() {
        let root = std::env::temp_dir().join(format!("ta_read_mod_{}", std::process::id()));
        std::fs::create_dir_all(root.join("Good")).unwrap();
        std::fs::create_dir_all(root.join("broken")).unwrap();
        std::fs::write(root.join("broken/mod.toml"), "id = ").unwrap();
        std::fs::write(root.join("Pack.UFO"), b"").unwrap();
        std::fs::write(root.join("notes.txt"), b"").unwrap();
        assert_eq!(read_mod(&root.join("Good")).unwrap().unwrap().manifest.id, "good");
        assert!(matches!(read_mod(&root.join("broken")), Err(ModError::Manifest { .. })));
        assert_eq!(read_mod(&root.join("Pack.UFO")).unwrap().unwrap().kind, ModKind::Archive);
        assert!(read_mod(&root.join("notes.txt")).unwrap().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scans_only_published_mods_while_installation_copies_exist() {
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("ta_mod_publication_{}_{nonce}", std::process::id()));
        for (folder, version) in [("testmod", "live"), (".testmod.installing", "unpublished"), (".testmod.old", "previous")] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
            let mut manifest = implicit_manifest("testmod");
            manifest.version = version.into();
            std::fs::write(root.join(folder).join("mod.toml"), toml::to_string(&manifest).unwrap()).unwrap();
        }
        // Incomplete staging has no manifest yet; ordinary dotted/hidden folders
        // remain legitimate legacy mods rather than being excluded wholesale.
        std::fs::create_dir_all(root.join(".unfinished.installing")).unwrap();
        let ordinary = ["ordinary.old", "another.installing", ".hidden", ".old"];
        for folder in ordinary { std::fs::create_dir_all(root.join(folder)).unwrap(); }
        let (found, errors) = scan(&root);
        assert!(errors.is_empty());
        assert_eq!(found.len(), ordinary.len() + 1);
        for folder in ordinary { assert!(found.iter().any(|m| m.path == root.join(folder)), "ordinary folder {folder} remains visible"); }
        let live = found.iter().filter(|m| m.manifest.id == "testmod").collect::<Vec<_>>();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].path, root.join("testmod"));
        assert_eq!(live[0].manifest.version, "live");
        let (available, problems) = scan_folders(std::slice::from_ref(&root));
        assert!(problems.is_empty(), "internal copies must not produce duplicate-id errors");
        assert_eq!(resolve_load_order(&["testmod".into()], &available).unwrap()[0].path, root.join("testmod"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn excessive_manifest_and_dependency_depth_are_errors() {
        let root = std::env::temp_dir().join(format!("ta_manifest_limit_{}", std::process::id()));
        std::fs::create_dir_all(root.join("mod")).unwrap();
        std::fs::write(root.join("mod/mod.toml"), vec![b' '; (1 << 20) + 1]).unwrap();
        let (found, errors) = scan(&root);
        assert!(found.is_empty());
        assert!(errors[0].to_string().contains("1 MiB"));
        std::fs::remove_dir_all(root).unwrap();
        let ids: Vec<_> = (0..100).map(|i| format!("mod{i}")).collect();
        let mods: Vec<_> = ids.iter().enumerate().map(|(i, id)| {
            m(id, &ids.get(i + 1).map(|s| vec![s.as_str()]).unwrap_or_default(), &[])
        }).collect();
        assert!(matches!(resolve_load_order(&ids, &mods), Err(ModError::Depth(_))));
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ta_mods_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// A folder without a manifest is named by its whole name: two with dots in theirs are two
    /// mods, not one id twice, and a version in a name is kept.
    #[test]
    fn a_folder_is_named_by_its_whole_name() {
        let root = temp("dots");
        for folder in ["TA.Escalation", "TA.Mayhem", "Mod v1.2"] { std::fs::create_dir_all(root.join(folder)).unwrap(); }
        std::fs::write(root.join("Pack.v2.UFO"), b"").unwrap();
        let (found, problems) = scan_folders(std::slice::from_ref(&root));
        assert!(problems.is_empty(), "{problems:?}");
        let mut named: Vec<(String, String)> = found.iter().map(|m| (m.manifest.id.clone(), m.manifest.name.clone())).collect();
        named.sort();
        assert_eq!(named, [("mod v1.2", "Mod v1.2"), ("pack.v2", "Pack.v2"), ("ta.escalation", "TA.Escalation"), ("ta.mayhem", "TA.Mayhem")].map(|(a, b)| (a.to_string(), b.to_string())));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The settings keep ids in lower case, any letter (`ta_config::Mods::enabled`): a mod
    /// without a manifest named in capitals beyond ASCII must be found by the id they keep.
    #[test]
    fn an_id_without_a_manifest_is_in_lower_case_as_the_settings_keep_it() {
        let root = temp("unicode");
        std::fs::create_dir_all(root.join("Übermacht")).unwrap();
        std::fs::write(root.join("ÄRA Karten.ufo"), b"").unwrap();
        let (found, _) = scan_folders(std::slice::from_ref(&root));
        let settings: Vec<String> = ["Übermacht", "ÄRA Karten"].iter().map(|id| id.to_lowercase()).collect();
        let order = resolve_load_order(&settings, &found).unwrap();
        assert_eq!(order.len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// One mod that is on and does not read (a damaged archive) mounts none of them: the game
    /// says so and plays the original game, as for a load order that does not resolve.
    #[test]
    fn one_mod_that_does_not_read_mounts_none() {
        let root = temp("unreadable");
        std::fs::create_dir_all(root.join("good/units")).unwrap();
        std::fs::write(root.join("good/units/good.fbi"), b"[UNITINFO]{UnitName=GOOD;}").unwrap();
        std::fs::write(root.join("broken.ufo"), b"not an archive").unwrap();
        let (found, _) = scan_folders(std::slice::from_ref(&root));
        let mounted = mount_enabled(None, None, &["good".to_string(), "broken".to_string()], &found);
        let result = &mounted.mods;
        assert!(matches!(result, Err(ModError::Unreadable { id, message }) if id == "broken" && message.starts_with("broken.ufo: ")), "{result:?}");
        assert_eq!(mounted.vfs.file_count(), 0, "nothing mounted");
        let mounted = mount_enabled(None, None, &["good".to_string()], &found);
        assert_eq!(mounted.mods.unwrap(), ["good"]);
        assert!(mounted.vfs.exists("units/good.fbi"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A mod that is on and whose manifest does not read is told as that, not as not installed.
    #[test]
    fn a_manifest_that_does_not_read_is_named_for_its_mod() {
        let root = temp("broken_manifest");
        for (folder, text) in [("mymod", "id = \"mymod\"\nversion = 1.0\n"), ("fine", "id = \"fine\"\nname = \"Fine\"\n")] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
            std::fs::write(root.join(folder).join("mod.toml"), text).unwrap();
        }
        let (found, problems) = scan_folders(std::slice::from_ref(&root));
        assert!(matches!(resolve_load_order(&["mymod".into()], &found), Err(ModError::Missing(_))));
        assert!(matches!(broken_manifest("mymod", &problems), Some(ModError::Manifest { path, .. }) if path.ends_with("mymod/mod.toml")));
        // (The only one that does not read, whatever its folder is called.)
        assert!(broken_manifest("other", &problems).is_some());
        std::fs::create_dir_all(root.join("second")).unwrap();
        std::fs::write(root.join("second/mod.toml"), "id = ").unwrap();
        let (_, problems) = scan_folders(std::slice::from_ref(&root));
        assert!(broken_manifest("other", &problems).is_none(), "two: which one is not known");
        assert!(broken_manifest("second", &problems).is_some());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod id_case_tests {
    use super::*;

    #[test]
    fn ids_read_in_lower_case() {
        let manifest: ModManifest = toml::from_str("id = \"MyMod\"\nname = \"My Mod\"\ndepends = [\"Base_Units\"]\nload_after = [\"OTHER\"]\n").unwrap();
        assert_eq!((manifest.id.as_str(), manifest.name.as_str()), ("mymod", "My Mod"), "the name as written");
        assert_eq!((manifest.depends, manifest.load_after), (vec!["base_units".to_string()], vec!["other".to_string()]));
    }
}

#[cfg(test)]
mod replacement_tests {
    use super::*;
    use crate::test_archive;

    fn temp(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ta_replace_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// An install with the archives TA: Enhanced's overwrite, and others.
    fn install(root: &Path) -> PathBuf {
        let game = root.join("game");
        test_archive::write(&game.join("totala1.hpi"), &[("units/armcom.fbi", b"base"), ("units/armsy.fbi", b"base"), ("palettes/palette.pal", b"base")]);
        test_archive::write(&game.join("CCDATA.CCX"), &[("units/armsy.fbi", b"core contingency"), ("units/armlance.fbi", b"core contingency")]);
        test_archive::write(&game.join("btdata.ccx"), &[("units/armhp.fbi", b"battle tactics")]);
        test_archive::write(&game.join("ccmaps.ccx"), &[("maps/coast.tnt", b"map")]);
        test_archive::write(&game.join("rev31.gp3"), &[("units/armcom.fbi", b"patch"), ("weapons/weapons.tdf", b"patch")]);
        test_archive::write(&game.join("AScarab.ufo"), &[("units/armscab.fbi", b"post-release")]);
        game
    }

    type Archive<'a> = (&'a str, &'a [(&'a str, &'a [u8])]);

    fn mod_dir(root: &Path, id: &str, manifest: &str, archives: &[Archive]) -> ModInfo {
        let dir = root.join("mods").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mod.toml"), format!("id = \"{id}\"\nname = \"{id}\"\n{manifest}")).unwrap();
        for (name, files) in archives {
            test_archive::write(&dir.join(name), files);
        }
        read_mod(&dir).unwrap().unwrap()
    }

    fn ids(m: &ModInfo) -> String {
        m.manifest.id.clone()
    }

    /// TA: Enhanced's layout: its whole game in `rev31.gp3`, stubs of `totala1.hpi` and
    /// `ccdata.ccx`, Battle Tactics' blanked: nothing of those archives of the install's is seen
    /// with it on, whatever the case of their names, and the others' are.
    #[test]
    fn a_mods_archive_replaces_the_installs_of_its_name() {
        let root = temp("enhanced");
        let game = install(&root);
        let enhanced = mod_dir(&root, "enhanced", "replaces = [\"BTDATA.CCX\"]\n", &[
            ("REV31.GP3", &[("units/armcom.fbi", b"mod"), ("weapons/weapons.tdf", b"mod")]),
            ("totala1.hpi", &[("palettes/palette.pal", b"mod")]),
            ("ccdata.ccx", &[("objects3d/armamph.3do", b"mod")]),
        ]);
        assert_eq!(enhanced.manifest.replaces, ["btdata.ccx"], "read in lower case");
        let mounted = mount(Some(&game), None, &[&enhanced], &ids);
        assert!(mounted.install.is_ok() && mounted.builtin.is_ok());
        assert_eq!(mounted.mods.as_deref().unwrap(), ["enhanced"]);
        assert_eq!(mounted.replaced, ["btdata.ccx", "CCDATA.CCX", "rev31.gp3", "totala1.hpi"], "as named on the disk, in order whatever their case");
        let vfs = &mounted.vfs;
        assert_eq!(vfs.read("units/armcom.fbi").unwrap(), b"mod");
        assert_eq!(vfs.read("palettes/palette.pal").unwrap(), b"mod");
        for leaked in ["units/armsy.fbi", "units/armlance.fbi", "units/armhp.fbi"] {
            assert!(!vfs.exists(leaked), "{leaked} of the install's replaced archives is not seen");
        }
        assert!(vfs.exists("maps/coast.tnt"), "the install's other archives stay");
        assert_eq!(vfs.read("units/armscab.fbi").unwrap(), b"post-release", "an archive no mod names stays");
        let base: Vec<String> = vfs.sources().filter(|s| s.layer == "base").map(|s| s.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(base, ["AScarab.ufo", "ccmaps.ccx"]);

        // The same mod off: the install whole, as mounted without mods.
        let without = mount(Some(&game), None, &[], &ids);
        let mut alone = crate::Vfs::new();
        alone.mount_game_dir(&game, "base").unwrap();
        let listing = |vfs: &crate::Vfs| vfs.list("", None, true).into_iter().map(|p| (vfs.origin(&p).unwrap().path.clone(), vfs.read(&p).unwrap(), p)).collect::<Vec<_>>();
        assert_eq!(listing(&without.vfs), listing(&alone), "no mod: the install as it always was");
        assert!(without.replaced.is_empty());
        assert_eq!(without.vfs.read("units/armsy.fbi").unwrap(), b"core contingency");
        let _ = std::fs::remove_dir_all(root);
    }

    /// A mod added from the game's folder holds archives of the names it has there (they are
    /// those, linked): none of them is the install's it takes the place of (the install never
    /// mounts them). One of the original game's names still is.
    #[test]
    fn a_mod_added_from_the_game_folder_replaces_none_of_its_own_archives() {
        let root = temp("game_folder_replaces");
        let game = install(&root);
        test_archive::write(&game.join("TAESC.gp3"), &[("unitse/armcom.fbi", b"escalation"), ("guie/armcom1.gui", b"escalation")]);
        let added = mod_dir(&root, "escalation", "", &[("TAESC.gp3", &[("unitse/armcom.fbi", b"escalation"), ("guie/armcom1.gui", b"escalation")])]);
        assert!(replaced_archives(&game, &[&added]).is_empty(), "its own archive is no replacement");
        let enhanced = mod_dir(&root, "enhanced", "", &[("rev31.gp3", &[("units/armcom.fbi", b"enhanced")])]);
        assert_eq!(replaced_archives(&game, &[&enhanced]), ["rev31.gp3"], "the original game's still is");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Mods made for other engines, put in the game's folder by their own installers: TA:
    /// Escalation's (folders of its own, and an archive of its name holding sounds alone), Total
    /// Mayhem's (TA's `units` beside folders of its own), and one TA Reborn does not know by name
    /// (two archives sharing a folder of their own). Each is found as a mod, and none of their
    /// archives is the original game's, whatever mods are on: TA's Commander stays TA's, not Total
    /// Mayhem's, its model not Escalation's. A map pack and TA's own archives stay.
    #[test]
    fn mods_in_the_game_folder_are_never_the_original_games() {
        let root = temp("game_folder");
        let game = install(&root);
        test_archive::write(&game.join("TAESC.gp3"), &[("unitse/armcom.fbi", b"escalation"), ("guie/armcom1.gui", b"escalation"), ("downloadse/esc.tdf", b"escalation"), ("objects3d/armcom.3do", b"escalation")]);
        test_archive::write(&game.join("T4ESC1.ufo"), &[("sounds/boom.wav", b"escalation")]);
        test_archive::write(&game.join("mayhem.gp3"), &[("units/armcom.fbi", b"mayhem"), ("guim/armcom1.gui", b"mayhem"), ("weaponm/weapons.tdf", b"mayhem")]);
        test_archive::write(&game.join("Talon.gp3"), &[("unittal/talcom.fbi", b"talon"), ("guital/talcom1.gui", b"talon")]);
        test_archive::write(&game.join("TalonExtra.ufo"), &[("guital/talcom2.gui", b"talon")]);
        test_archive::write(&game.join("MyMaps.ufo"), &[("maps/mine.tnt", b"maps"), ("maps/mine.ota", b"maps")]);
        let found = game_folder_mods(&game);
        let names = |m: &GameFolderMod| m.archives.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        let pairs = |list: &[(&str, &str)]| list.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<BTreeMap<String, String>>();
        assert_eq!(found.iter().map(|m| (m.id.as_str(), m.name.as_str())).collect::<Vec<_>>(), [("escalation", "TA: Escalation"), ("totalmayhem", "Total Mayhem"), ("talon", "Talon")]);
        assert_eq!(names(&found[0]), ["T4ESC1.ufo", "TAESC.gp3"], "an archive of its name is the mod's whatever it holds");
        assert_eq!(found[0].rename, pairs(&[("downloadse", "download"), ("guie", "guis"), ("unitse", "units")]));
        assert_eq!((names(&found[1]), found[1].rename.clone()), (vec!["mayhem.gp3".to_string()], pairs(&[("guim", "guis"), ("weaponm", "weapons")])));
        assert_eq!((names(&found[2]), found[2].rename.clone()), (vec!["Talon.gp3".to_string(), "TalonExtra.ufo".to_string()], pairs(&[("guital", "guis"), ("unittal", "units")])));
        let plan = crate::package::game_folder_plan(&found[2]);
        assert_eq!((plan.root.clone(), plan.units, plan.maps), (game.clone(), 1, 0), "its unit counted where its engine read it");
        for on in [Vec::new(), vec![mod_dir(&root, "maps", "", &[("more.ufo", &[("maps/more.tnt", b"more")])])]] {
            let mounted = mount(Some(&game), None, &on.iter().collect::<Vec<_>>(), &ids);
            assert_eq!(mounted.game_folder_mods, found);
            let vfs = &mounted.vfs;
            assert_eq!(vfs.read("units/armcom.fbi").unwrap(), b"patch", "TA's own Commander, not Total Mayhem's");
            for gone in ["objects3d/armcom.3do", "sounds/boom.wav", "guim/armcom1.gui", "guital/talcom2.gui", "unitse/armcom.fbi"] {
                assert!(!vfs.exists(gone), "{gone} is a mod's, not the original game's");
            }
            assert!(vfs.exists("maps/mine.tnt"), "a map pack stays the original game's, as TA read it");
            assert!(vfs.exists("units/armscab.fbi"), "and TA's own post-release units");
        }
        // Mounted alone (the launcher's pictures and sounds, the tests), the install leaves them out
        // too.
        let mut alone = crate::Vfs::new();
        alone.mount_game_dir(&game, "base").unwrap();
        assert_eq!(alone.read("units/armcom.fbi").unwrap(), b"patch");
        assert!(!alone.exists("objects3d/armcom.3do") && alone.exists("maps/mine.tnt"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// A mod in the game's folder whose archive's name is longer than a mod id may be gets an id
    /// the installer takes, a second of the same name one too.
    #[test]
    fn a_long_named_mod_in_the_game_folder_gets_an_id_the_installer_takes() {
        let root = temp("long_name");
        let game = install(&root);
        let long = format!("{} Extra Units.", "Very".repeat(20));
        test_archive::write(&game.join(format!("{long}gp3")), &[("unitsx/a.fbi", b"a"), ("unitsx/b.fbi", b"b")]);
        test_archive::write(&game.join(format!("{long}ufo")), &[("unitsy/c.fbi", b"c")]);
        let found = game_folder_mods(&game);
        assert_eq!(found.len(), 2, "two mods, sharing no folder of their own");
        for m in &found {
            assert!((1..=MOST_ID_LETTERS).contains(&m.id.chars().count()), "{} is too long", m.id);
            assert!(crate::package::safe_component(&m.id), "{}", m.id);
        }
        assert_ne!(found[0].id, found[1].id);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A mod made for a patched engine dropped into a mods folder by hand, without a manifest (a
    /// folder of its archives, an archive alone, loose folders): its folders of its own are read as
    /// TA's, as the installer has them; a mod with TA's folders alone, a manifest's renames, are
    /// left as they are.
    #[test]
    fn a_mod_without_a_manifest_reads_its_own_engines_folders() {
        let root = temp("implicit");
        let game = install(&root);
        let mods = root.join("mods");
        test_archive::write(&mods.join("Total Mayhem").join("mayhem.gp3"), &[("units/armcom.fbi", b"mayhem"), ("guim/armcom1.gui", b"mayhem"), ("weaponm/weapons.tdf", b"mayhem"), ("unitpicm/armcom.pcx", b"mayhem")]);
        test_archive::write(&mods.join("TAESC.gp3"), &[("unitse/armcom.fbi", b"escalation"), ("guie/armcom1.gui", b"escalation"), ("gamedate/sidedata.tdf", b"escalation"), ("downloadse/a.tdf", b"escalation")]);
        std::fs::create_dir_all(mods.join("talon").join("unittal")).unwrap();
        std::fs::write(mods.join("talon").join("unittal").join("talcom.fbi"), b"talon").unwrap();
        test_archive::write(&mods.join("units.ufo"), &[("units/new.fbi", b"new")]);
        let (found, problems) = scan(&mods);
        assert!(problems.is_empty(), "{problems:?}");
        let rename = |id: &str| found.iter().find(|m| m.manifest.id == id).unwrap_or_else(|| panic!("{id}")).manifest.rename.clone();
        let pairs = |list: &[(&str, &str)]| list.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<BTreeMap<String, String>>();
        assert_eq!(rename("total mayhem"), pairs(&[("guim", "guis"), ("unitpicm", "unitpics"), ("weaponm", "weapons")]));
        assert_eq!(rename("taesc"), pairs(&[("downloadse", "download"), ("gamedate", "gamedata"), ("guie", "guis"), ("unitse", "units")]));
        assert_eq!(rename("talon"), pairs(&[("unittal", "units")]));
        assert!(rename("units").is_empty(), "TA's own folders stand for themselves");
        let mayhem = found.iter().find(|m| m.manifest.id == "total mayhem").unwrap();
        let mounted = mount(Some(&game), None, &[mayhem], &ids);
        assert!(mounted.mods.is_ok(), "{:?}", mounted.mods.err());
        assert_eq!(mounted.vfs.read("guis/armcom1.gui").unwrap(), b"mayhem", "its menus where TA reads them");
        assert_eq!(mounted.vfs.read("weapons/weapons.tdf").unwrap(), b"mayhem", "and its weapons, in place of TA's");
        assert_eq!(mounted.vfs.read("units/armcom.fbi").unwrap(), b"mayhem");
        let escalation = found.iter().find(|m| m.manifest.id == "taesc").unwrap();
        let mounted = mount(Some(&game), None, &[escalation], &ids);
        assert_eq!(mounted.vfs.read("units/armcom.fbi").unwrap(), b"escalation", "its units in place of TA's");
        assert!(!mounted.vfs.exists("units/armsy.fbi"), "TA's own hidden beneath, as its engine read only its folder");
        // (Kept from scan to scan, and read again when the archive changes: the mount, which
        // maps it, let go first.)
        drop(mounted);
        test_archive::write(&mods.join("TAESC.gp3"), &[("unitse/armcom.fbi", b"escalation, again")]);
        let (found, _) = scan(&mods);
        assert_eq!(found.iter().find(|m| m.manifest.id == "taesc").unwrap().manifest.rename, pairs(&[("unitse", "units")]), "read again once it changed");
        // (A folder's file renamed deep in it, the folders above it unchanged: read again too.)
        std::fs::create_dir_all(mods.join("deep").join("unitse").join("arm")).unwrap();
        std::fs::write(mods.join("deep/unitse/arm/unit.txt"), b"deep").unwrap();
        let renamed = |mods: &Path| scan(mods).0.into_iter().find(|m| m.manifest.id == "deep").unwrap().manifest.rename;
        assert!(renamed(&mods).is_empty(), "nothing of TA's in it yet");
        std::fs::rename(mods.join("deep/unitse/arm/unit.txt"), mods.join("deep/unitse/arm/unit.fbi")).unwrap();
        assert_eq!(renamed(&mods), pairs(&[("unitse", "units")]), "its unit found once renamed");
        let _ = std::fs::remove_dir_all(root);
    }

    /// An empty placeholder of an archive's name (nothing at all, as some packages ship) blanks
    /// the install's: the mod mounts, its placeholder left out with a warning.
    #[test]
    fn an_empty_placeholder_blanks_the_installs_archive() {
        let root = temp("placeholder");
        let game = install(&root);
        let module = mod_dir(&root, "blanks", "", &[("units.ufo", &[("units/new.fbi", b"mod")])]);
        std::fs::write(module.path.join("btdata.ccx"), b"").unwrap();
        let mounted = mount(Some(&game), None, &[&module], &ids);
        assert!(mounted.mods.is_ok(), "{:?}", mounted.mods.err());
        assert_eq!(mounted.replaced, ["btdata.ccx"]);
        assert!(!mounted.vfs.exists("units/armhp.fbi") && mounted.vfs.exists("units/new.fbi") && mounted.vfs.exists("units/armsy.fbi"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// A mod that is one archive named like the install's replaces it too; one of another name
    /// (Escalation's, Total Mayhem's) replaces nothing.
    #[test]
    fn an_archive_mod_replaces_the_installs_of_its_name_and_others_nothing() {
        let root = temp("archive_mod");
        let game = install(&root);
        let mods = root.join("mods");
        test_archive::write(&mods.join("Rev31.gp3"), &[("units/armcom.fbi", b"dropped in")]);
        test_archive::write(&mods.join("TAESC.gp3"), &[("units/armcom.fbi", b"escalation")]);
        let (found, _) = scan(&mods);
        let patch = found.iter().find(|m| m.manifest.id == "rev31").unwrap();
        let escalation = found.iter().find(|m| m.manifest.id == "taesc").unwrap();
        let mounted = mount(Some(&game), None, &[patch], &ids);
        assert_eq!(mounted.replaced, ["rev31.gp3"]);
        assert!(!mounted.vfs.exists("weapons/weapons.tdf"), "the install's patch is gone with it");
        let mounted = mount(Some(&game), None, &[escalation], &ids);
        assert!(mounted.replaced.is_empty());
        assert_eq!(mounted.vfs.read("weapons/weapons.tdf").unwrap(), b"patch", "layered over the install, as before");
        assert_eq!(mounted.vfs.read("units/armcom.fbi").unwrap(), b"escalation");
        let _ = std::fs::remove_dir_all(root);
    }

    /// One mod that does not read leaves every mod out, and the install whole: nothing of it is
    /// replaced by a mod that is not mounted.
    #[test]
    fn a_mod_that_does_not_read_replaces_nothing() {
        let root = temp("unreadable");
        let game = install(&root);
        let enhanced = mod_dir(&root, "enhanced", "", &[("rev31.gp3", &[("units/armcom.fbi", b"mod")])]);
        std::fs::write(root.join("mods/broken.ufo"), b"not an archive").unwrap();
        let broken = read_mod(&root.join("mods/broken.ufo")).unwrap().unwrap();
        let mounted = mount(Some(&game), None, &[&enhanced, &broken], &ids);
        assert!(matches!(&mounted.mods, Err(ModError::Unreadable { id, .. }) if id == "broken"), "{:?}", mounted.mods);
        assert!(mounted.replaced.is_empty());
        assert_eq!(mounted.vfs.read("units/armcom.fbi").unwrap(), b"patch");
        // (A load order that does not resolve: the same.)
        let mounted = mount_enabled(Some(&game), None, &["enhanced".into(), "missing".into()], std::slice::from_ref(&enhanced));
        assert!(matches!(mounted.mods, Err(ModError::Missing(_))));
        assert!(mounted.replaced.is_empty() && mounted.vfs.exists("units/armsy.fbi"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// TA: Enhanced installed before placeholders were kept (no `replaces`, no `btdata.ccx`)
    /// mounts as installed now: what its package blanked and its readme removed is known by its
    /// catalogue id.
    #[test]
    fn ta_enhanced_installed_before_replaces_what_its_package_blanked() {
        let root = temp("known");
        let game = install(&root);
        let archives: &[Archive] = &[("rev31.gp3", &[("units/armcom.fbi", b"mod")]), ("totala1.hpi", &[("palettes/palette.pal", b"mod")]), ("ccdata.ccx", &[("objects3d/x.3do", b"mod")])];
        let before = mod_dir(&root, "taenhanced", "", archives);
        let mounted = mount(Some(&game), None, &[&before], &ids);
        assert_eq!(mounted.replaced, ["AScarab.ufo", "btdata.ccx", "CCDATA.CCX", "rev31.gp3", "totala1.hpi"]);
        assert!(!mounted.vfs.exists("units/armhp.fbi") && !mounted.vfs.exists("units/armscab.fbi"));
        // The same installed now: its placeholder named in its manifest. The same game, the same
        // digest.
        let now_root = temp("known_now");
        let now = mod_dir(&now_root, "taenhanced", "replaces = [\"btdata.ccx\"]\n", archives);
        assert_eq!(content_digest(&before).unwrap(), content_digest(&now).unwrap());
        assert_eq!(mount(Some(&game), None, &[&now], &ids).replaced, mounted.replaced);
        // Under another id (installed by hand): what its files say only.
        let other = mod_dir(&root, "ta enhanced", "", archives);
        assert_eq!(mount(Some(&game), None, &[&other], &ids).replaced, ["CCDATA.CCX", "rev31.gp3", "totala1.hpi"]);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(now_root);
    }

    /// The digest of a mod that replaces nothing of the original game is the one it had before
    /// archives were replaced (records made with Escalation or Total Mayhem still match); one
    /// that does replace says so, so a record of it made before reads as made with other files.
    #[test]
    fn only_a_mod_that_replaces_has_another_digest() {
        use sha2::{Digest, Sha256};
        let root = temp("digest");
        let escalation = mod_dir(&root, "escalation", "", &[("TAESC.gp3", &[("units/a.fbi", b"a")]), ("T2ESC.ufo", &[("units/b.fbi", b"b")])]);
        // (How the digest was made before: the files, by path, size and content.)
        let mut files: Vec<(String, Vec<u8>)> = ["T2ESC.ufo", "TAESC.gp3"].iter().map(|n| (n.to_lowercase(), std::fs::read(escalation.path.join(n)).unwrap())).collect();
        files.sort();
        let mut hash = Sha256::new();
        hash.update(b"TA Reborn mod content v1\0");
        hash.update(0u64.to_le_bytes());
        hash.update((files.len() as u64).to_le_bytes());
        for (name, bytes) in &files {
            hash.update((name.len() as u64).to_le_bytes());
            hash.update(name.as_bytes());
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(Sha256::digest(bytes));
        }
        let before: [u8; 16] = hash.finalize()[..16].try_into().unwrap();
        assert_eq!(content_digest(&escalation).unwrap(), before);
        assert!(replaced_claims(&escalation).is_empty());
        // The same files named like the original game's: a mod that replaces.
        let patch = mod_dir(&root, "patch", "", &[("rev31.gp3", &[("units/a.fbi", b"a")])]);
        assert_eq!(replaced_claims(&patch).into_iter().collect::<Vec<_>>(), ["rev31.gp3"]);
        let mut said = patch.clone();
        said.manifest.replaces = vec!["btdata.ccx".into()];
        assert_ne!(content_digest(&patch).unwrap(), content_digest(&said).unwrap(), "what it says it replaces is the mod's");
        let _ = std::fs::remove_dir_all(root);
    }
}
