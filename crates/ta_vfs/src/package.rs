//! Installing a mod from a downloaded package: a zip, rar or 7z as mods are published
//! (ModDB and the like). Most were made for the original game: their archives and folders
//! sit beside a patched `TotalA.exe`, DLLs, readmes and optional extras, sometimes in a
//! folder of their own ("Talon v1.03/") or several ("Step_3_Install Main Files/").
//!
//! [`Unpacker`] unpacks a package with the system's own tools (7-Zip when installed, which
//! reads every RAR; else Windows' tar). [`plan`] keeps an authored manifest's content layout;
//! otherwise it picks the folder holding the largest TA archive (found by its `HAPI` header, whatever the
//! extension: Total Battletech's is `rev31.mwp`), the valid archives there (empty
//! placeholders that blanked out the original's are left out) and TA's folders beside them.
//! Many of these mods came with a copy of TA's engine patched to read folders of their own
//! (Total Mayhem `guiM`, Escalation `unitsE`, Talon `unitTAL`, Total Battletech `unitsBT`);
//! a folder whose name starts like one of TA's and that holds that folder's kind of file,
//! where the mod has none of TA's own, becomes a rename ([`crate::Renames`]).
//! [`install`] lays that out as a mod folder with its `mod.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use thiserror::Error;

use crate::ARCHIVE_EXTENSIONS;
use crate::mods::ModManifest;

#[derive(Debug, Error)]
pub enum PackageError {
    #[error("unsafe package content: {0}")]
    Unsafe(String),
    #[error("no program to unpack archives was found (install 7-Zip from 7-zip.org)")]
    NoUnpacker,
    #[error("{tool} could not unpack {file}: {message}")]
    Unpack { tool: String, file: String, message: String },
    #[error("only part of {file} could be unpacked; install 7-Zip (7-zip.org) and try again")]
    Incomplete { file: String },
    #[error("no Total Annihilation content was found in the package")]
    NoContent,
    #[error("{path}: {source}")]
    Io { path: String, source: std::io::Error },
    /// Stopped on request (the tool was killed).
    #[error("cancelled")]
    Cancelled,
}

/// What a package may unpack to: more is refused, and stopped as soon as it is seen.
const MAX_PACKAGE_BYTES: u64 = 8 << 30;
const MAX_PACKAGE_ENTRIES: usize = 100_000;

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> PackageError + '_ {
    move |source| PackageError::Io { path: path.display().to_string(), source }
}

/// A program that unpacks archives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unpacker {
    /// 7-Zip's command line (`7z.exe`): zip, 7z and every RAR.
    SevenZip(PathBuf),
    /// bsdtar (Windows' own `tar.exe` since Windows 10): zip, 7z and most RARs.
    Tar(PathBuf),
}

impl Unpacker {
    /// 7-Zip when installed, else the system's bsdtar.
    pub fn find() -> Option<Unpacker> {
        let mut seven: Vec<PathBuf> = ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|p| PathBuf::from(p).join("7-Zip").join("7z.exe"))
            .collect();
        seven.extend(on_path(&["7z", "7zz", "7za"]));
        // Finder does not inherit a shell's Homebrew PATH. Check both native Homebrew
        // prefixes so installing 7-Zip also works for a launcher opened from its icon.
        #[cfg(target_os = "macos")]
        for bin in ["/opt/homebrew/bin", "/usr/local/bin"] {
            seven.extend(["7zz", "7z", "7za"].map(|name| Path::new(bin).join(name)));
        }
        if let Some(p) = seven.into_iter().find(|p| p.is_file()) {
            return Some(Unpacker::SevenZip(p));
        }
        // Windows' tar is bsdtar; elsewhere look for bsdtar by name (GNU tar reads no zip).
        let mut tar: Vec<PathBuf> = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32").join("tar.exe")).into_iter().collect();
        // macOS ships libarchive's bsdtar as `tar`, unlike GNU tar on Linux.
        #[cfg(target_os = "macos")]
        tar.push(PathBuf::from("/usr/bin/tar"));
        tar.extend(on_path(&["bsdtar"]));
        tar.into_iter().find(|p| p.is_file()).map(Unpacker::Tar)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Unpacker::SevenZip(_) => "7-Zip",
            Unpacker::Tar(_) => "tar",
        }
    }

    /// Unpacks `archive` into `into` (created). Fails when the tool does, and when what came
    /// out is far smaller than the package (bsdtar stops without a word inside some RARs).
    pub fn unpack(&self, archive: &Path, into: &Path) -> Result<(), PackageError> {
        self.unpack_cancellable(archive, into, &|| false)
    }

    /// [`Unpacker::unpack`], the tool killed as soon as `cancelled` says so (a cancel, the
    /// launcher closing: it must not go on unpacking behind it).
    pub fn unpack_cancellable(&self, archive: &Path, into: &Path, cancelled: &dyn Fn() -> bool) -> Result<(), PackageError> {
        let archive = std::fs::canonicalize(archive).map_err(io(archive))?;
        std::fs::create_dir_all(into).map_err(io(into))?;
        if crate::is_link(&std::fs::symlink_metadata(into).map_err(io(into))?) || std::fs::read_dir(into).map_err(io(into))?.next().is_some() {
            return Err(PackageError::Unsafe("extraction requires an empty ordinary directory".into()));
        }
        let mut command = match self {
            Unpacker::SevenZip(exe) => {
                let mut c = Command::new(exe);
                c.arg("x").arg("-y").arg("-bd").arg(format!("-o{}", into.display())).arg("--").arg(&archive);
                c
            }
            Unpacker::Tar(exe) => {
                let mut c = Command::new(exe);
                c.arg("-xf").arg(&archive).arg("-C").arg(into);
                c
            }
        };
        #[cfg(windows)]
        {
            // No console window flashing up over the launcher.
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let file = archive.file_name().map_or(String::new(), |f| f.to_string_lossy().to_string());
        let failed = |message: String| PackageError::Unpack { tool: self.name().into(), file: file.clone(), message };
        let mut child = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| failed(e.to_string()))?;
        // (Its complaints read on a thread of their own: a full pipe would stall it.)
        let mut complaints = child.stderr.take();
        let reader = std::thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(pipe) = complaints.as_mut() {
                let _ = std::io::Read::read_to_end(pipe, &mut text);
            }
            text
        });
        // What comes out is measured as it does, every second or so (less often where the
        // walk is slow): a package unpacking to more than the limits (a "zip bomb") is stopped
        // before it fills the drive, not found out after.
        let mut measure_at = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let status = loop {
            if cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(PackageError::Cancelled);
            }
            if let Some(status) = child.try_wait().map_err(|e| failed(e.to_string()))? {
                break status;
            }
            if std::time::Instant::now() >= measure_at {
                let started = std::time::Instant::now();
                if past_limits(into, MAX_PACKAGE_ENTRIES, MAX_PACKAGE_BYTES) {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err(PackageError::Unsafe(format!("{file} unpacks to more than 8 GiB or 100,000 files")));
                }
                measure_at = std::time::Instant::now() + std::time::Duration::from_secs(1).max(started.elapsed() * 10);
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        };
        let complaints = reader.join().unwrap_or_default();
        if !status.success() {
            let message = String::from_utf8_lossy(&complaints).lines().rfind(|l| !l.trim().is_empty()).unwrap_or("failed").trim().to_string();
            return Err(failed(message));
        }
        check_tree(into)?;
        let packed = std::fs::metadata(&archive).map_or(0, |m| m.len());
        if packed > 1 << 20 && tree_size(into) < packed / 2 {
            return Err(PackageError::Incomplete { file });
        }
        Ok(())
    }
}

fn on_path(names: &[&str]) -> Vec<PathBuf> {
    let Some(path) = std::env::var_os("PATH") else { return Vec::new() };
    let exts: &[&str] = if cfg!(windows) { &[".exe", ""] } else { &[""] };
    let mut out = Vec::new();
    for dir in std::env::split_paths(&path) {
        for name in names {
            for ext in exts {
                out.push(dir.join(format!("{name}{ext}")));
            }
        }
    }
    out
}

/// Whether what is under `root` passes `entries` or `bytes` (the walk stops as soon as it
/// does; what cannot be read yet, a file being written, is passed over).
fn past_limits(root: &Path, entries: usize, bytes: u64) -> bool {
    let mut pending = vec![root.to_path_buf()];
    let (mut count, mut total) = (0usize, 0u64);
    while let Some(dir) = pending.pop() {
        let Ok(list) = std::fs::read_dir(&dir) else { continue };
        for entry in list.flatten() {
            count += 1;
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(entry.path()),
                Ok(kind) if kind.is_file() => total = total.saturating_add(entry.metadata().map_or(0, |m| m.len())),
                _ => {}
            }
            if count > entries || total > bytes { return true; }
        }
    }
    false
}

fn tree_size(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => tree_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

/// TA's data folders (lower case), taken when they sit beside the content's archives.
pub const TA_FOLDERS: &[&str] = &[
    "ai", "anims", "bitmaps", "camps", "download", "features", "fonts", "gamedata", "guis", "maps", "objects3d", "palettes",
    "scripts", "sounds", "textures", "unitpics", "units", "weapons", "music",
];

/// What of a package goes into the mod.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// The folder (in the unpacked package) the content is in.
    pub root: PathBuf,
    /// Valid TA archives there, by path; installed under their names (a TA archive with an
    /// extension of its own, `.mwp`, is installed as a `.gp3`).
    pub archives: Vec<PathBuf>,
    /// TA folders beside them.
    pub folders: Vec<PathBuf>,
    /// Documents beside them (readmes, changelogs), kept in the mod's `docs/`.
    pub docs: Vec<PathBuf>,
    /// Folders the mod's engine read in place of TA's (`guim` → `guis`).
    pub rename: BTreeMap<String, String>,
    /// The empty placeholders beside the content (file names, in lower case): copied into TA's
    /// folder they blanked the original's archives of their names, so the mod's manifest says it
    /// replaces those ([`ModManifest::replaces`]); the files themselves are left out.
    pub replaces: Vec<String>,
    /// Left out, for the report: programs, settings, placeholders, archives that do not read,
    /// optional extras.
    pub skipped: Vec<String>,
    /// Units and maps the content brings (`.fbi` and `.ota` files).
    pub units: usize,
    pub maps: usize,
}

/// Whether a file is a TA archive: a `HAPI` header (checked first) and a directory that
/// reads, with files in it. Not one: `Err(None)` (no header, or nothing in it: a placeholder),
/// or `Err(Some(why))`, one whose directory does not read.
fn ta_archive(path: &Path) -> Result<ta_formats::hpi::HpiArchive, Option<String>> {
    let mut magic = [0u8; 4];
    let mut f = std::fs::File::open(path).map_err(|_| None)?;
    std::io::Read::read_exact(&mut f, &mut magic).map_err(|_| None)?;
    if &magic != b"HAPI" {
        return Err(None);
    }
    match ta_formats::hpi::HpiArchive::open(path) {
        Ok(archive) if archive.files().is_empty() => Err(None),
        Ok(archive) => Ok(archive),
        Err(e) => Err(Some(e.to_string())),
    }
}

/// Whether a file is an empty placeholder: no `HAPI` header at all, or an archive with no file
/// in it (not a damaged archive). Named like one of the install's archives, it blanks that one
/// ([`crate::mods::replaced_archives`]).
pub fn is_placeholder(path: &Path) -> bool {
    matches!(ta_archive(path), Err(None))
}

/// Chooses the TA content of an unpacked package; see the module documentation.
pub fn plan(unpacked: &Path) -> Result<Plan, PackageError> {
    check_tree(unpacked)?;
    // Every TA archive in the package (a few levels down), and every TA folder.
    let mut archives: Vec<(PathBuf, u64)> = Vec::new();
    let mut folder_parents: Vec<PathBuf> = Vec::new();
    let mut stubs: Vec<PathBuf> = Vec::new();
    let mut unreadable: Vec<(PathBuf, String)> = Vec::new();
    let mut manifests = Vec::new();
    walk(unpacked, 0, &mut |path, is_dir| {
        if is_dir {
            let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().to_ascii_lowercase());
            if TA_FOLDERS.contains(&name.as_str()) && let Some(parent) = path.parent() {
                folder_parents.push(parent.to_path_buf());
            }
            return;
        }
        let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
        if path.file_name().is_some_and(|name| name.eq_ignore_ascii_case("mod.toml")) {
            manifests.push(path.parent().unwrap_or(unpacked).to_path_buf());
        }
        let size = std::fs::metadata(path).map_or(0, |m| m.len());
        let named = ARCHIVE_EXTENSIONS.contains(&ext.as_str());
        match if named || size >= 4 { ta_archive(path) } else { Err(None) } {
            Ok(_) => archives.push((path.to_path_buf(), size)),
            // (Not a placeholder: an archive the mod needs, damaged or packed in a way not read.)
            Err(Some(why)) => unreadable.push((path.to_path_buf(), why)),
            Err(None) if named => stubs.push(path.to_path_buf()),
            Err(None) => {}
        }
    });
    // An authored mod's manifest takes precedence over legacy layout guesses when its
    // own folder contains data. A wrapper manifest without data still uses the old layout.
    manifests.sort_by_key(|dir| (dir.components().count(), dir.clone()));
    let mut authored = None;
    for dir in manifests {
        // Probe candidates without letting skipped examples or extras invalidate the
        // package. The selected root's metadata is read and validated below.
        let Ok(Some(own)) = crate::mods::read_mod(&dir) else { continue };
        if folder_parents.contains(&dir) || archives.iter().any(|(path, _)| path.parent() == Some(dir.as_path()))
            || renamed_folders(&own.manifest.rename).is_ok_and(|renamed| renamed.iter().any(|name| crate::install::find_dir_ci(&dir, name).is_some())) {
            authored = Some(dir);
            break;
        }
    }
    // Without an authored layout, content is beside the largest archive, else TA's
    // shallowest folder, as in the original community packages.
    let root = match authored {
        Some(root) => root,
        None => match archives.iter().max_by_key(|(_, size)| *size) {
            Some((path, _)) => path.parent().unwrap_or(unpacked).to_path_buf(),
            None => folder_parents.iter().min_by_key(|p| p.components().count()).cloned().ok_or(PackageError::NoContent)?,
        },
    };
    let own = crate::mods::read_mod(&root).map_err(|error| PackageError::Unsafe(error.to_string()))?
        .ok_or(PackageError::NoContent)?.manifest;
    let declared = renamed_folders(&own.rename)?;
    let mut plan = Plan { root: root.clone(), rename: own.rename.clone(), ..Default::default() };
    let rel = |p: &Path| p.strip_prefix(unpacked).unwrap_or(p).to_string_lossy().replace('\\', "/");
    plan.archives = archives.iter().filter(|(p, _)| p.parent() == Some(root.as_path())).map(|(p, _)| p.clone()).collect();
    plan.archives.sort();
    for (path, _) in archives.iter().filter(|(p, _)| p.parent() != Some(root.as_path())) {
        plan.skipped.push(format!("{} (an extra, not beside the main content)", rel(path)));
    }
    for stub in &stubs {
        plan.skipped.push(format!("{} (empty placeholder)", rel(stub)));
        // (Beside the content, one blanks the original's of its name; elsewhere it is an extra's.)
        if stub.parent() == Some(root.as_path()) && let Some(name) = stub.file_name() {
            plan.replaces.push(name.to_string_lossy().to_lowercase());
        }
    }
    plan.replaces.sort();
    plan.replaces.dedup();
    for (path, why) in &unreadable {
        plan.skipped.push(format!("{} (unreadable: {why})", rel(path)));
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&root).map_err(io(&root))?.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().to_ascii_lowercase());
        if path.is_dir() {
            // (A folder with a name of the mod's own stays only if it proves to be one.)
            if TA_FOLDERS.contains(&name.as_str()) || declared.contains(&name) || rename_candidate(&name).is_some() {
                plan.folders.push(path);
            } else {
                plan.skipped.push(format!("{}/", rel(&path)));
            }
        } else if plan.archives.contains(&path) || stubs.contains(&path) || unreadable.iter().any(|(p, _)| *p == path) {
        } else if ["txt", "md", "rtf", "pdf", "doc", "htm", "html"].iter().any(|e| name.ends_with(&format!(".{e}"))) {
            plan.docs.push(path);
        } else {
            plan.skipped.push(rel(&path));
        }
    }
    // What the content holds, by top folder: how many files of each kind.
    let mut contents = Contents::new();
    let mut content_paths = std::collections::BTreeSet::new();
    let mut count = |p: &str| {
        content_paths.insert(p.to_string());
        count_content(&mut contents, p);
    };
    for path in &plan.archives {
        let Ok(archive) = ta_archive(path) else { continue };
        for f in archive.files() {
            count(&crate::normalize(&f.path));
        }
    }
    for folder in &plan.folders {
        walk(folder, 0, &mut |path, is_dir| {
            if !is_dir {
                count(&crate::normalize(&path.strip_prefix(&root).unwrap_or(path).to_string_lossy()));
            }
        });
    }
    // Folders of the mod's own standing in for TA's; loose ones that do not are left out.
    plan.rename.extend(detected_renames(&contents, &declared));
    let (rename, unpacked_root) = (plan.rename.clone(), unpacked.to_path_buf());
    plan.folders.retain(|f| {
        let name = f.file_name().map_or(String::new(), |n| n.to_string_lossy().to_ascii_lowercase());
        TA_FOLDERS.contains(&name.as_str()) || declared.contains(&name) || rename.contains_key(&name)
    });
    for f in std::fs::read_dir(&root).map_err(io(&root))?.flatten().map(|e| e.path()) {
        let name = f.file_name().map_or(String::new(), |n| n.to_string_lossy().to_ascii_lowercase());
        if f.is_dir() && !TA_FOLDERS.contains(&name.as_str()) && !declared.contains(&name) && rename_candidate(&name).is_some() && !rename.contains_key(&name) {
            plan.skipped.push(format!("{}/", f.strip_prefix(&unpacked_root).unwrap_or(&f).to_string_lossy().replace('\\', "/")));
        }
    }
    if plan.archives.is_empty() && plan.folders.is_empty() {
        return Err(PackageError::NoContent);
    }
    // Count distinct virtual files after applying the same full-path mapping the VFS
    // uses. A nested source ("roster/army/"), case and trailing slashes all matter here.
    let mapped = crate::Renames::new(&plan.rename);
    let paths: std::collections::BTreeSet<_> = content_paths.into_iter().map(|path| mapped.apply(path)).collect();
    for path in paths {
        if path.starts_with("units/") && path.ends_with(".fbi") { plan.units += 1; }
        if path.starts_with("maps/") && path.ends_with(".ota") { plan.maps += 1; }
    }
    Ok(plan)
}

/// What a mod's content holds, by top folder (normalised): how many files of each kind (their
/// extensions).
pub(crate) type Contents = BTreeMap<String, BTreeMap<String, usize>>;

/// Counts a file of a mod's content, at its normalised path, into `contents`.
pub(crate) fn count_content(contents: &mut Contents, path: &str) {
    if let Some((top, rest)) = path.split_once('/') {
        let ext = rest.rsplit_once('.').map_or("", |(_, e)| e).to_string();
        *contents.entry(top.to_string()).or_default().entry(ext).or_default() += 1;
    }
}

/// The folders of a mod's own that stand in for TA's (`guim` → `guis`, `unitse` → `units`): a
/// top folder named like one ([`rename_candidate`]) and mostly of the kind of file that one holds,
/// where the mod has no folder of TA's own name beside it. `declared` (a manifest's own) are left
/// as their manifest has them.
pub(crate) fn detected_renames(contents: &Contents, declared: &std::collections::BTreeSet<String>) -> BTreeMap<String, String> {
    let mut rename = BTreeMap::new();
    for (top, kinds) in contents {
        let Some((folder, kind)) = rename_candidate(top) else { continue };
        let files: usize = kinds.values().sum();
        let fitting = kinds.get(kind).copied().unwrap_or(0);
        if fitting * 2 > files && !contents.contains_key(folder) && !declared.contains(top) {
            rename.insert(top.clone(), folder.to_string());
        }
    }
    rename
}

/// What a TA archive holds ([`Contents`]), or nothing when it does not read.
pub(crate) fn archive_contents(path: &Path) -> Option<Contents> {
    let archive = ta_formats::hpi::HpiArchive::open(path).ok()?;
    let mut contents = Contents::new();
    for f in archive.files() {
        count_content(&mut contents, &crate::normalize(&f.path));
    }
    Some(contents)
}

/// What goes into the mod a mod found in the original game's folder is added as
/// ([`crate::mods::GameFolderMod`]): its archives where they are (the launcher links them into the
/// mod's folder, never taking them out of the game's), the folders of its own it uses, and the
/// units and maps it brings.
pub fn game_folder_plan(found: &crate::mods::GameFolderMod) -> Plan {
    let root = found.archives.first().and_then(|p| p.parent()).map(Path::to_path_buf).unwrap_or_default();
    let mut plan = Plan { root, archives: found.archives.clone(), rename: found.rename.clone(), ..Default::default() };
    let mapped = crate::Renames::new(&plan.rename);
    let mut paths = std::collections::BTreeSet::new();
    for archive in &plan.archives {
        if let Ok(archive) = ta_formats::hpi::HpiArchive::open(archive) {
            paths.extend(archive.files().iter().map(|f| mapped.apply(crate::normalize(&f.path))));
        }
    }
    for path in paths {
        if path.starts_with("units/") && path.ends_with(".fbi") { plan.units += 1; }
        if path.starts_with("maps/") && path.ends_with(".ota") { plan.maps += 1; }
    }
    plan
}

/// Direct children that must be retained for explicitly mapped (possibly nested) paths.
fn renamed_folders(rename: &BTreeMap<String, String>) -> Result<std::collections::BTreeSet<String>, PackageError> {
    let mut folders = std::collections::BTreeSet::new();
    for from in rename.keys() {
        let path = crate::normalize(from);
        let path = path.trim_end_matches('/');
        if path.is_empty() || !path.split('/').all(safe_component) {
            return Err(PackageError::Unsafe(format!("invalid renamed folder: {from}")));
        }
        folders.insert(path.split('/').next().expect("nonempty path").to_string());
    }
    Ok(folders)
}

/// TA folders a mod's engine may have read under names of its own: the folder, the stem
/// such names start with, and the kind of file that fills it (the most specific first).
const RENAMABLE: &[(&str, &str, &str)] = &[
    ("unitpics", "unitpic", "pcx"),
    ("units", "unit", "fbi"),
    ("weapons", "weap", "tdf"),
    ("guis", "gui", "gui"),
    ("gamedata", "game", "tdf"),
    ("download", "download", "tdf"),
];

/// The TA folder a folder of this name may stand in for, and the kind of file that would
/// show it does (`unittal` → units, `.fbi`). TA's own folders stand for themselves.
fn rename_candidate(name: &str) -> Option<(&'static str, &'static str)> {
    let name = name.to_ascii_lowercase();
    if TA_FOLDERS.contains(&name.as_str()) {
        return None;
    }
    RENAMABLE.iter().find(|(_, stem, _)| name.starts_with(stem)).map(|&(folder, _, kind)| (folder, kind))
}

fn walk(dir: &Path, depth: usize, f: &mut dyn FnMut(&Path, bool)) {
    if depth > 6 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            f(&path, true);
            walk(&path, depth + 1, f);
        } else {
            f(&path, false);
        }
    }
}

/// Lays the planned content out as a mod folder in `mods_dir` with its `mod.toml` (the renames
/// the plan found added to the manifest), replacing an earlier version of the mod
/// ([`install_target`]: never a folder holding anything else). Files are moved out of the
/// unpacked package. Returns the mod's folder.
pub fn install(plan: &Plan, manifest: &ModManifest, mods_dir: &Path) -> Result<PathBuf, PackageError> {
    install_with_commit_guard(plan, manifest, mods_dir, || Ok(()))
}

/// The folder of `mods_dir` the mod `id` is installed in: where that mod is already (the copy the
/// game reads, its folder whatever it is called), else `<id>`, else the first free `<id>-<n>`.
/// A folder holding anything else is never the target, so never replaced: another mod (the map
/// the editor saved as "Twilight", id `map-twilight-…`, in the folder TA: Twilight's id names on a
/// disk that does not tell case apart), a folder whose manifest does not read, a file. None when
/// every name is taken.
pub fn install_target(mods_dir: &Path, id: &str) -> Option<PathBuf> {
    let id = id.to_lowercase();
    let (found, _) = crate::mods::scan(mods_dir);
    if let Some(installed) = found.iter().find(|m| m.manifest.id == id && m.kind == crate::mods::ModKind::Directory) {
        return Some(installed.path.clone());
    }
    (1..=999u32)
        .map(|n| mods_dir.join(if n == 1 { id.to_string() } else { format!("{id}-{n}") }))
        .filter(|path| !crate::mods::install_work_directory(path))
        .find(|path| std::fs::symlink_metadata(path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound))
}

/// Like [`install`], with a caller-owned guard held only during the final directory
/// swap and rollback. Staging/copying and deletion of obsolete content happen
/// outside the guard, so an application's shutdown need only wait for publication.
/// A rejected guard leaves the existing installation untouched.
pub fn install_with_commit_guard<G>(
    plan: &Plan,
    manifest: &ModManifest,
    mods_dir: &Path,
    before_commit: impl FnOnce() -> Result<G, PackageError>,
) -> Result<PathBuf, PackageError> {
    // Discovery and profile settings use lower-case IDs. Publication, transaction locks
    // and the manifest must use that same identity even for callers constructing metadata.
    let mut canonical = manifest.clone();
    canonical.id = canonical.id.to_lowercase();
    let manifest = &canonical;
    if !safe_component(&manifest.id) {
        return Err(PackageError::Unsafe("mod id must be a single ordinary directory name".into()));
    }
    check_tree(&plan.root)?;
    // Public plans may also be built by callers: only direct children of the selected
    // content root can be moved, and no destination may alias an unpacked source.
    for path in plan.archives.iter().chain(&plan.folders).chain(&plan.docs) {
        if path.parent() != Some(plan.root.as_path()) {
            return Err(PackageError::Unsafe("planned file is outside the content root".into()));
        }
    }
    // Archives with unrecognised extensions acquire .gp3 when installed. Check
    // the final names before moving anything, including on case-sensitive disks.
    let mut names = std::collections::BTreeSet::from(["mod.toml".to_string()]);
    for name in plan.archives.iter().map(|p| archive_name(p))
        .chain(plan.folders.iter().map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned()))
        .chain((!plan.docs.is_empty()).then(|| "docs".to_string()))
        .chain(plan.docs.iter().map(|p| format!("docs/{}", p.file_name().unwrap_or_default().to_string_lossy())))
    {
        if !names.insert(name.to_lowercase()) {
            return Err(PackageError::Unsafe(format!("multiple package entries install as {name}")));
        }
    }
    std::fs::create_dir_all(mods_dir).map_err(io(mods_dir))?;
    let root = std::fs::canonicalize(&plan.root).map_err(io(&plan.root))?;
    let destination = std::fs::canonicalize(mods_dir).map_err(io(mods_dir))?;
    if root.starts_with(&destination) || destination.starts_with(&root) {
        return Err(PackageError::Unsafe("installation and unpacked content overlap".into()));
    }
    // Profiles share installed mods, and more than one launcher can be open. Keep
    // staging, replacement and rollback names exclusive for this mod until publication.
    let _transaction = lock_install(mods_dir, &manifest.id)?;
    reject_archive_target(mods_dir, &manifest.id)?;
    let staging = mods_dir.join(format!(".{}.installing", manifest.id));
    let old = mods_dir.join(format!(".{}.old", manifest.id));
    let ordinary = |path: &Path| match std::fs::symlink_metadata(path) {
        Ok(meta) if crate::is_link(&meta) || !meta.is_dir() => Err(PackageError::Unsafe(format!("{} is not an ordinary directory", path.display()))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io(path)(error)),
    };
    ordinary(&staging)?;
    ordinary(&old)?;
    for (path, incomplete) in [(&staging, true), (&old, false)] {
        if !path.exists() { continue; }
        // Only staging may legitimately lack a manifest after an interrupted copy.
        // A published backup must identify the mod it belongs to.
        if incomplete && !path.join("mod.toml").exists() { continue; }
        match crate::mods::read_mod(path) {
            Ok(Some(info)) if info.manifest.id == manifest.id.to_lowercase() => {}
            _ => return Err(PackageError::Unsafe(format!("{} does not belong to mod {}", path.display(), manifest.id))),
        }
    }
    remove_directory(&staging)?;
    std::fs::create_dir(&staging).map_err(io(&staging))?;
    // (Whatever stops it from here leaves no half-made copy behind.)
    let staged = (|| -> Result<(), PackageError> {
        for path in &plan.archives {
            move_path(path, &staging.join(archive_name(path)))?;
        }
        for path in &plan.folders {
            if let Some(name) = path.file_name() {
                move_path(path, &staging.join(name))?;
            }
        }
        if !plan.docs.is_empty() {
            let docs = staging.join("docs");
            std::fs::create_dir_all(&docs).map_err(io(&docs))?;
            for path in &plan.docs {
                if let Some(name) = path.file_name() {
                    move_path(path, &docs.join(name))?;
                }
            }
        }
        let mut manifest = manifest.clone();
        manifest.rename.extend(plan.rename.clone());
        for name in &plan.replaces {
            if !manifest.replaces.contains(name) { manifest.replaces.push(name.clone()); }
        }
        let text = toml::to_string_pretty(&manifest).map_err(|e| PackageError::Io { path: "mod.toml".into(), source: std::io::Error::other(e) })?;
        let toml_path = staging.join("mod.toml");
        std::fs::write(&toml_path, text).map_err(io(&toml_path))
    })();
    if let Err(error) = staged {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    // Obsolete content may be large; clear it outside the short publication guard
    // only when a live copy of the mod exists. After a crash between the two renames, .old
    // is the only previous installation: retain it on cancellation or staging
    // failure, and let the rollback below restore it if publication fails.
    if install_target(mods_dir, &manifest.id).is_some_and(|live| live.exists())
        && let Err(error) = remove_directory(&old)
    {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    // (On the disk before it takes the installed copy's place: see `sync_tree`.)
    sync_tree(&staging);
    let commit = match before_commit() {
        Ok(guard) => guard,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    // One publication at a time in the folder, its target chosen now: another install (another
    // launcher, `ta-tools`), of another mod, may have published to the name this one would have
    // taken while it staged (`foo-2`, beside another mod's `foo`): that is never replaced.
    let publishing = match lock_publication(mods_dir) {
        Ok(lock) => lock,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    let target = match install_target(mods_dir, &manifest.id) {
        Some(target) => target,
        None => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(PackageError::Unsafe(format!("no free folder for {} in {}", manifest.id, mods_dir.display())));
        }
    };
    if let Err(error) = reject_archive_target(mods_dir, &manifest.id) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    if let Err(error) = ordinary(&target) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    // Swap the new version in (the old one out of the way first).
    if target.exists() && let Err(error) = std::fs::rename(&target, &old) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(io(&target)(error));
    }
    if let Err(error) = std::fs::rename(&staging, &target) {
        if old.exists() { let _ = std::fs::rename(&old, &target); }
        let _ = std::fs::remove_dir_all(&staging);
        return Err(io(&staging)(error));
    }
    drop(publishing);
    drop(commit);
    // (The swap itself on the disk too, where a folder can be flushed.)
    #[cfg(unix)]
    let _ = std::fs::File::open(mods_dir).and_then(|dir| dir.sync_all());
    let _ = std::fs::remove_dir_all(&old);
    Ok(target)
}

/// Flushes a staged copy's files (and on Unix its folders) to the disk before it takes the
/// installed copy's place, the previous version then removed at once: the unpacker's output, a
/// copy across drives and `mod.toml` are written without it, and a power cut just after an
/// update could otherwise leave the new copy half written with the old one gone. Best effort: a
/// file that cannot be opened to write (one marked read-only) is left to the system.
fn sync_tree(root: &Path) {
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                // (Write access: Windows flushes a file only through a handle that may write.)
                let _ = std::fs::OpenOptions::new().write(true).open(entry.path()).and_then(|file| file.sync_all());
            }
        }
        #[cfg(unix)]
        let _ = std::fs::File::open(&dir).and_then(|dir| dir.sync_all());
    }
}

fn archive_name(path: &Path) -> String {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    if ARCHIVE_EXTENSIONS.contains(&ext.as_str()) {
        path.file_name().unwrap_or_default().to_string_lossy().into_owned()
    } else {
        format!("{}.gp3", path.file_stem().unwrap_or_default().to_string_lossy())
    }
}

fn reject_archive_target(mods_dir: &Path, id: &str) -> Result<(), PackageError> {
    if let Some(info) = crate::mods::scan(mods_dir).0.into_iter()
        .find(|info| info.kind == crate::mods::ModKind::Archive && info.manifest.id == id.to_lowercase())
    {
        return Err(PackageError::Unsafe(format!("mod {id} is already installed as {}; remove it before installing a directory package", info.path.display())));
    }
    Ok(())
}

fn remove_directory(path: &Path) -> Result<(), PackageError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io(path)(error)),
    }
}

/// The mods folder's publication lock: one install (or recovery) at a time chooses its folder
/// and swaps it in, across mods and processes. Held only for the swap.
fn lock_publication(mods_dir: &Path) -> Result<std::fs::File, PackageError> {
    let (path, file) = open_lock(mods_dir.join(".publish.lock"))?;
    file.lock().map_err(io(&path))?;
    Ok(file)
}

/// Stable across staging-directory swaps and process crashes. Never unlink the lock:
/// another process may already be waiting on that same file handle.
fn lock_install(mods_dir: &Path, id: &str) -> Result<std::fs::File, PackageError> {
    let (path, file) = open_install_lock(mods_dir, id)?;
    file.lock().map_err(io(&path))?;
    Ok(file)
}

/// [`lock_install`] when no install holds it now; None when one does (or it cannot be opened).
fn try_lock_install(mods_dir: &Path, id: &str) -> Option<std::fs::File> {
    let (_, file) = open_install_lock(mods_dir, id).ok()?;
    file.try_lock().ok()?;
    Some(file)
}

fn open_install_lock(mods_dir: &Path, id: &str) -> Result<(PathBuf, std::fs::File), PackageError> {
    open_lock(mods_dir.join(format!(".{id}.install.lock")))
}

/// A lock file of the mods folder (an ordinary file, never a link), opened.
fn open_lock(path: PathBuf) -> Result<(PathBuf, std::fs::File), PackageError> {
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if crate::is_link(&meta) || !meta.is_file() => return Err(PackageError::Unsafe(format!("{} is not an ordinary lock file", path.display()))),
        Ok(_) => {},
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
        Err(e) => return Err(io(&path)(e)),
    }
    let file = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&path).map_err(io(&path))?;
    Ok((path, file))
}

/// Puts right what an install stopped half way (the launcher closed by force, the computer
/// switched off) left in `mods_dir`, for each mod no install is at work on now (its lock free):
/// a copy staged and never published (`.<id>.installing`) is removed; the previous version moved
/// aside for the swap (`.<id>.old`) goes back when it is the only copy left (stopped between the
/// swap's two steps: the scan hides it, so the mod was gone), else it is removed. What was done,
/// for the log.
pub fn recover(mods_dir: &Path) -> Vec<String> {
    let mut done = Vec::new();
    let Ok(entries) = std::fs::read_dir(mods_dir) else { return done };
    let ids: std::collections::BTreeSet<String> = entries.flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let id = name.strip_prefix('.')?.strip_suffix(".installing").or_else(|| name.strip_prefix('.')?.strip_suffix(".old"))?.to_string();
            // (The map editor's own working folders, `.editor-<pid>-<n>.installing`, are its.)
            (safe_component(&id) && !id.starts_with("editor-")).then_some(id)
        })
        .collect();
    // An ordinary folder (not a link) holding the mod `id`, or nothing of any mod yet.
    let ours = |dir: &Path, id: &str, unfinished: bool| {
        !std::fs::symlink_metadata(dir).is_ok_and(|m| crate::is_link(&m)) && dir.is_dir()
            && match crate::mods::read_mod(dir) {
                Ok(Some(m)) => m.manifest.id == id || unfinished && !dir.join("mod.toml").exists(),
                _ => false,
            }
    };
    for id in ids {
        let Some(_lock) = try_lock_install(mods_dir, &id) else { continue };
        let staging = mods_dir.join(format!(".{id}.installing"));
        if ours(&staging, &id, true) {
            match std::fs::remove_dir_all(&staging) {
                Ok(()) => done.push(format!("removed an unfinished install of {id}")),
                Err(e) => done.push(format!("could not remove an unfinished install of {id}: {e}")),
            }
        }
        let old = mods_dir.join(format!(".{id}.old"));
        if !ours(&old, &id, false) { continue; }
        // (Its folder chosen and taken as an install's is: one at a time.)
        let Ok(_publishing) = lock_publication(mods_dir) else { continue };
        let live = crate::mods::scan(mods_dir).0.iter().any(|m| m.manifest.id == id);
        if live {
            match std::fs::remove_dir_all(&old) {
                Ok(()) => done.push(format!("removed the replaced version of {id}")),
                Err(e) => done.push(format!("could not remove the replaced version of {id}: {e}")),
            }
        } else if let Some(target) = install_target(mods_dir, &id) {
            match std::fs::rename(&old, &target) {
                Ok(()) => done.push(format!("restored {id} (its update had stopped half way) to {}", target.display())),
                Err(e) => done.push(format!("could not restore {id} from {}: {e}", old.display())),
            }
        }
    }
    done
}

/// Cross-platform safe filename component (including Windows devices and ADS).
pub fn safe_component(name: &str) -> bool {
    if name.is_empty() || name == "." || name == ".." || name.ends_with(['.', ' '])
        || name.chars().any(|c| c.is_control() || "<>:\"/\\|?*".contains(c)) { return false; }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$")
        && !stem.strip_prefix("COM").or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|n| matches!(n, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"))
}

/// Validate before walking/copying: links (including junctions) never become package
/// content, and directory recursion and aggregate filesystem work are bounded.
fn check_tree(root: &Path) -> Result<(), PackageError> {
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let (mut count, mut bytes) = (0usize, 0u64);
    while let Some((path, depth)) = pending.pop() {
        let meta = std::fs::symlink_metadata(&path).map_err(io(&path))?;
        count += 1;
        bytes = bytes.saturating_add(if meta.is_file() { meta.len() } else { 0 });
        if crate::is_link(&meta) || depth > 64 || count > MAX_PACKAGE_ENTRIES || bytes > MAX_PACKAGE_BYTES || (!meta.is_file() && !meta.is_dir()) {
            return Err(PackageError::Unsafe(format!("{} is linked or exceeds package resource limits", path.display())));
        }
        if meta.is_dir() {
            for child in std::fs::read_dir(&path).map_err(io(&path))? {
                let child = child.map_err(io(&path))?;
                if count + pending.len() >= MAX_PACKAGE_ENTRIES {
                    return Err(PackageError::Unsafe("package exceeds 100,000 filesystem entries".into()));
                }
                pending.push((child.path(), depth + 1));
            }
        }
    }
    Ok(())
}

/// Moves a file or folder (copying when a rename cannot, across drives).
fn move_path(from: &Path, to: &Path) -> Result<(), PackageError> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        std::fs::create_dir_all(to).map_err(io(to))?;
        for entry in std::fs::read_dir(from).map_err(io(from))?.flatten() {
            move_path(&entry.path(), &to.join(entry.file_name()))?;
        }
        let _ = std::fs::remove_dir_all(from);
        Ok(())
    } else {
        std::fs::copy(from, to).map_err(io(to))?;
        let _ = std::fs::remove_file(from);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planner_review_ignores_invalid_manifests_outside_the_selected_content() {
        let root = std::env::temp_dir().join(format!("ta-package-review-optional-manifest-{}", std::process::id()));
        for (index, invalid) in ["not TOML", "id = '../escape'\nname = 'Unsafe'", "id = 'example'\nname = 'Example'\n[rename]\n'../outside' = 'units'"].iter().enumerate() {
            let input = root.join(format!("input{index}"));
            let main = input.join("Main");
            let example = input.join("Examples");
            std::fs::create_dir_all(main.join("units")).unwrap();
            std::fs::write(main.join("units/tank.fbi"), b"[UNITINFO]{UnitName=TANK;}").unwrap();
            std::fs::create_dir_all(&example).unwrap();
            std::fs::write(example.join("mod.toml"), invalid).unwrap();
            let plan = plan(&input).expect("an unrelated example's metadata must not prevent importing the main content");
            assert_eq!(plan.root, main);
            assert_eq!(plan.units, 1);
            let installed = install(&plan, &manifest(&format!("main-{index}"), "1"), &root.join("mods")).unwrap();
            assert_eq!(std::fs::read(installed.join("units/tank.fbi")).unwrap(), b"[UNITINFO]{UnitName=TANK;}");
            assert_eq!(std::fs::read_to_string(example.join("mod.toml")).unwrap(), *invalid);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn planner_review_keeps_invalid_selected_metadata_fatal() {
        let root = std::env::temp_dir().join(format!("ta-package-review-selected-manifest-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Main/units")).unwrap();
        std::fs::write(root.join("Main/units/tank.fbi"), b"unit").unwrap();
        for invalid in ["not TOML", "id = '../escape'\nname = 'Unsafe'", "id = 'main'\nname = 'Main'\n[rename]\n'../outside' = 'units'"] {
            std::fs::write(root.join("Main/mod.toml"), invalid).unwrap();
            assert!(matches!(plan(&root), Err(PackageError::Unsafe(_))), "selected metadata must fail validation: {invalid}");
            assert!(root.join("Main/units/tank.fbi").is_file());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_music_only_mod_is_installable_and_keeps_its_tracks() {
        let root = std::env::temp_dir().join(format!("ta-mod-qa-music-{}", std::process::id()));
        let input = root.join("input");
        std::fs::create_dir_all(input.join("music")).unwrap();
        std::fs::write(input.join("music/7.ogg"), b"test music bytes").unwrap();
        let plan = plan(&input).expect("the game reads mod music from the VFS");
        let installed = install(&plan, &manifest("music-addon", "1"), &root.join("mods")).unwrap();
        let mut vfs = crate::Vfs::new();
        vfs.mount_mod_dir(&installed, "music-addon").unwrap();
        assert_eq!(vfs.read("music/7.ogg").unwrap(), b"test music bytes");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_manifest_renames_preserve_the_authors_folders() {
        let root = std::env::temp_dir().join(format!("ta-mod-qa-renames-{}", std::process::id()));
        let input = root.join("input");
        std::fs::create_dir_all(input.join("units")).unwrap();
        std::fs::create_dir_all(input.join("custom_army")).unwrap();
        std::fs::write(input.join("units/unused.fbi"), b"not the selected army").unwrap();
        std::fs::write(input.join("custom_army/tank.fbi"), b"[UNITINFO]{UnitName=TANK;}").unwrap();
        let mut own = manifest("custom-army", "1");
        own.rename.insert("custom_army".into(), "units".into());
        std::fs::write(input.join("mod.toml"), toml::to_string(&own).unwrap()).unwrap();
        let plan = plan(&input).unwrap();
        let installed = install(&plan, &own, &root.join("mods")).unwrap();
        let info = crate::mods::read_mod(&installed).unwrap().unwrap();
        let mounted = crate::mods::mount_enabled(None, None, &[own.id], &[info]);
        assert!(mounted.mods.is_ok());
        assert_eq!(mounted.vfs.read("units/tank.fbi").unwrap(), b"[UNITINFO]{UnitName=TANK;}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_only_layouts_keep_nested_and_non_ascii_folders_and_correct_counts() {
        let root = std::env::temp_dir().join(format!("ta-mod-qa-custom-layout-{}", std::process::id()));
        for (index, from) in ["Roster/Army/", "UnitÄrmy/"].iter().enumerate() {
            let input = root.join(format!("input{index}"));
            std::fs::create_dir_all(input.join(from)).unwrap();
            std::fs::write(input.join(from).join("tank.fbi"), b"[UNITINFO]{UnitName=TANK;}").unwrap();
            let mut own = manifest(&format!("custom-{index}"), "1");
            own.rename.insert((*from).into(), "units/".into());
            std::fs::write(input.join("mod.toml"), toml::to_string(&own).unwrap()).unwrap();
            let plan = plan(&input).unwrap();
            assert_eq!(plan.units, 1, "count the unit at its explicitly mapped path: {from}");
            let installed = install(&plan, &own, &root.join("mods")).unwrap();
            let info = crate::mods::read_mod(&installed).unwrap().unwrap();
            let mounted = crate::mods::mount_enabled(None, None, &[own.id], &[info]);
            assert!(mounted.mods.is_ok());
            assert_eq!(mounted.vfs.read("units/tank.fbi").unwrap(), b"[UNITINFO]{UnitName=TANK;}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn differently_cased_install_ids_update_the_same_mod() {
        let root = std::env::temp_dir().join(format!("ta-mod-qa-update-case-{}", std::process::id()));
        let mods = root.join("mods");
        let first = install(&content(&root, b"first"), &manifest("MixedCase", "1"), &mods).unwrap();
        let next = install(&content(&root, b"next"), &manifest("mixedcase", "2"), &mods).unwrap();
        assert_eq!(first, next, "the on-disk identity is case-insensitive on every platform");
        let final_copy = install(&content(&root, b"final"), &manifest("MIXEDCASE", "3"), &mods).unwrap();
        assert_eq!(final_copy, first);
        let (found, errors) = crate::mods::scan(&mods);
        assert!(errors.is_empty());
        assert_eq!(found.len(), 1, "an update must not publish a second copy with the same ID");
        assert_eq!(found[0].manifest.version, "3");
        assert_eq!(std::fs::read(first.join("units/NEW.FBI")).unwrap(), b"final");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// macOS must import ordinary mod ZIPs out of the box, including when launched by
    /// Finder with no Homebrew tools in PATH. This exercises the actual system fallback.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_tar_imports_a_zip_package() {
        let base = std::env::temp_dir().join(format!("ta_package_macos_zip_{}", std::process::id()));
        let input = base.join("input");
        std::fs::create_dir_all(input.join("units")).unwrap();
        std::fs::write(input.join("units/test.fbi"), b"[UNITINFO]{UnitName=TEST;}").unwrap();
        let zip = base.join("mod.zip");
        let tar = PathBuf::from("/usr/bin/tar");
        assert!(Command::new(&tar).args(["--format=zip", "-cf"]).arg(&zip).arg("-C").arg(&input).arg("units").status().unwrap().success());
        assert!(Unpacker::find().is_some(), "macOS must discover its system archive reader");
        let output = base.join("output");
        Unpacker::Tar(tar).unpack(&zip, &output).unwrap();
        assert_eq!(plan(&output).unwrap().units, 1);
        assert_eq!(std::fs::read(output.join("units/test.fbi")).unwrap(), b"[UNITINFO]{UnitName=TEST;}");
        std::fs::remove_dir_all(base).unwrap();
    }

    /// A cancel stops the unpacker (killed, not waited for) and says so; uncancelled, the same
    /// package unpacks whole. (Windows' own tar makes the package.)
    #[cfg(windows)]
    #[test]
    fn a_cancelled_unpack_stops_the_tool() {
        let Some(unpacker) = Unpacker::find() else { return };
        let Some(tar) = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32").join("tar.exe")).filter(|p| p.is_file()) else { return };
        let base = std::env::temp_dir().join(format!("ta_package_cancel_{}", std::process::id()));
        let input = base.join("input");
        std::fs::create_dir_all(input.join("units")).unwrap();
        std::fs::write(input.join("units/test.fbi"), b"[UNITINFO]{UnitName=TEST;}").unwrap();
        let zip = base.join("mod.zip");
        assert!(Command::new(&tar).arg("-a").arg("-cf").arg(&zip).arg("-C").arg(&input).arg("units").status().unwrap().success());
        assert!(matches!(unpacker.unpack_cancellable(&zip, &base.join("stopped"), &|| true), Err(PackageError::Cancelled)));
        unpacker.unpack(&zip, &base.join("whole")).unwrap();
        assert_eq!(std::fs::read(base.join("whole/units/test.fbi")).unwrap(), b"[UNITINFO]{UnitName=TEST;}");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn what_unpacks_is_measured_against_the_limits() {
        let root = std::env::temp_dir().join(format!("ta_package_limits_{}", std::process::id()));
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/big.bin"), vec![0u8; 3000]).unwrap();
        std::fs::write(root.join("small.bin"), vec![0u8; 10]).unwrap();
        assert!(!past_limits(&root, 10, 4000));
        assert!(past_limits(&root, 10, 2000), "bytes");
        assert!(past_limits(&root, 3, 1 << 20), "entries");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folders_a_mods_engine_may_have_renamed() {
        assert_eq!(rename_candidate("guiM"), Some(("guis", "gui")));
        assert_eq!(rename_candidate("unitpicE"), Some(("unitpics", "pcx")));
        assert_eq!(rename_candidate("unitTAL"), Some(("units", "fbi")));
        assert_eq!(rename_candidate("weapsBT"), Some(("weapons", "tdf")));
        assert_eq!(rename_candidate("gamedate"), Some(("gamedata", "tdf")));
        assert_eq!(rename_candidate("downloadse"), Some(("download", "tdf")));
        assert_eq!(rename_candidate("guis"), None);
        assert_eq!(rename_candidate("unitpics"), None);
        assert_eq!(rename_candidate("sounds"), None);
    }

    #[test]
    fn unsafe_ids_cannot_escape_or_remove_an_existing_directory() {
        let base = std::env::temp_dir().join(format!("ta_package_unsafe_{}", std::process::id()));
        std::fs::create_dir_all(base.join("victim")).unwrap();
        std::fs::write(base.join("victim/canary"), b"keep").unwrap();
        for id in ["../victim", "..\\victim", "C:\\victim", "x:stream", "NUL", "COM1.txt", "COM¹.txt", "LPT³", "CONOUT$", "trailing."] {
            let manifest = ModManifest { id: id.into(), name: "test".into(), version: "1".into(), description: String::new(), authors: Vec::new(), depends: Vec::new(), load_after: Vec::new(), source: String::new(), rename: BTreeMap::new(), replaces: Vec::new() };
            assert!(install(&Plan::default(), &manifest, &base.join("mods")).is_err(), "{id}");
            assert_eq!(std::fs::read(base.join("victim/canary")).unwrap(), b"keep");
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn concurrent_mod_install_waits_without_deleting_the_active_staging_directory() {
        use std::sync::mpsc;
        use std::time::Duration;
        let base = std::env::temp_dir().join(format!("ta_package_concurrent_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mods = base.join("mods");
        let unpacked = base.join("unpacked");
        std::fs::create_dir_all(unpacked.join("units")).unwrap();
        std::fs::write(unpacked.join("units/NEW.FBI"), b"new version").unwrap();
        let plan = plan(&unpacked).unwrap();
        let staging = mods.join(".testmod.installing");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("canary"), b"first install is still using this").unwrap();
        let held = lock_install(&mods, "testmod").unwrap();
        let manifest = ModManifest { id: "testmod".into(), name: "Test".into(), version: "1".into(), description: String::new(), authors: Vec::new(), depends: Vec::new(), load_after: Vec::new(), source: String::new(), rename: BTreeMap::new(), replaces: Vec::new() };
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let other_mods = mods.clone();
        let installer = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            done_tx.send(install(&plan, &manifest, &other_mods)).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(done_rx.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)), "a concurrent install must wait before touching staging or rollback folders");
        assert_eq!(std::fs::read(staging.join("canary")).unwrap(), b"first install is still using this");
        std::fs::rename(&staging, mods.join("testmod")).unwrap();
        drop(held);
        let installed = done_rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        installer.join().unwrap();
        assert_eq!(std::fs::read(installed.join("units/NEW.FBI")).unwrap(), b"new version");
        assert!(!staging.exists() && !mods.join(".testmod.old").exists());
        let (found, errors) = crate::mods::scan(&mods);
        assert!(errors.is_empty());
        assert_eq!(found.len(), 1, "transaction locks are not visible as mods");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn publication_guard_covers_only_the_complete_directory_swap() {
        use std::cell::Cell;
        struct Published<'a> { mods: &'a Path, released: &'a Cell<bool> }
        impl Drop for Published<'_> {
            fn drop(&mut self) {
                assert!(self.mods.join("testmod/units/NEW.FBI").is_file(), "release after the replacement is visible");
                assert!(self.mods.join(".testmod.old/canary").is_file(), "obsolete content is deleted outside the guard");
                self.released.set(true);
            }
        }
        let base = std::env::temp_dir().join(format!("ta_package_publication_{}", std::process::id()));
        let mods = base.join("mods");
        let source = base.join("source");
        std::fs::create_dir_all(source.join("units")).unwrap();
        std::fs::write(source.join("units/NEW.FBI"), b"new version").unwrap();
        std::fs::create_dir_all(mods.join("testmod")).unwrap();
        std::fs::write(mods.join("testmod/canary"), b"previous version").unwrap();
        let manifest = ModManifest { id: "testmod".into(), name: "Test".into(), version: "1".into(), description: String::new(), authors: Vec::new(), depends: Vec::new(), load_after: Vec::new(), source: String::new(), rename: BTreeMap::new(), replaces: Vec::new() };
        let released = Cell::new(false);
        let installed = install_with_commit_guard(&plan(&source).unwrap(), &manifest, &mods, || {
            assert!(mods.join(".testmod.installing/units/NEW.FBI").is_file(), "copying is finished before acquiring the guard");
            assert!(mods.join(".testmod.installing/mod.toml").is_file());
            assert_eq!(std::fs::read(mods.join("testmod/canary")).unwrap(), b"previous version");
            Ok(Published { mods: &mods, released: &released })
        }).unwrap();
        assert!(released.get());
        assert_eq!(installed, mods.join("testmod"));
        assert!(!mods.join(".testmod.old").exists());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn interrupted_publication_keeps_the_only_previous_install_until_replacement_succeeds() {
        for outcome in ["cancel", "publish-failure", "success"] {
            let base = std::env::temp_dir().join(format!("ta_package_interrupted_{outcome}_{}", std::process::id()));
            let mods = base.join("mods");
            let source = base.join("source");
            std::fs::create_dir_all(source.join("units")).unwrap();
            std::fs::write(source.join("units/NEW.FBI"), b"new version").unwrap();
            // A crash after target -> .old and before staging -> target leaves
            // this backup as the only surviving copy of the installed mod.
            let backup = mods.join(".testmod.old");
            std::fs::create_dir_all(&backup).unwrap();
            std::fs::write(backup.join("canary"), b"previous version").unwrap();
            let manifest = ModManifest { id: "testmod".into(), name: "Test".into(), version: "2".into(), description: String::new(), authors: Vec::new(), depends: Vec::new(), load_after: Vec::new(), source: String::new(), rename: BTreeMap::new(), replaces: Vec::new() };
            std::fs::write(backup.join("mod.toml"), toml::to_string(&manifest).unwrap()).unwrap();
            let result = install_with_commit_guard(&plan(&source).unwrap(), &manifest, &mods, || {
                if outcome == "cancel" { return Err(PackageError::Cancelled); }
                if outcome == "publish-failure" {
                    // Inject a failed final rename after staging succeeded.
                    std::fs::remove_dir_all(mods.join(".testmod.installing")).unwrap();
                }
                Ok(())
            });
            let target = mods.join("testmod");
            match outcome {
                "cancel" => {
                    assert!(matches!(result, Err(PackageError::Cancelled)));
                    assert_eq!(std::fs::read(backup.join("canary")).unwrap(), b"previous version");
                    assert!(!target.exists(), "a rejected publication guard must not publish a mod");
                }
                "publish-failure" => {
                    assert!(result.is_err());
                    assert_eq!(std::fs::read(target.join("canary")).unwrap(), b"previous version", "failed publication restores the surviving backup");
                }
                _ => {
                    assert_eq!(result.unwrap(), target);
                    assert_eq!(std::fs::read(target.join("units/NEW.FBI")).unwrap(), b"new version");
                    assert!(!backup.exists(), "obsolete content can be deleted after publication");
                }
            }
            assert!(!mods.join(".testmod.installing").exists());
            std::fs::remove_dir_all(base).unwrap();
        }
    }

    fn manifest(id: &str, version: &str) -> ModManifest {
        ModManifest { id: id.into(), name: id.into(), version: version.into(), description: String::new(), authors: Vec::new(), depends: Vec::new(), load_after: Vec::new(), source: String::new(), rename: BTreeMap::new(), replaces: Vec::new() }
    }

    #[test]
    fn launcher_review_preserves_foreign_or_unreadable_transaction_directories() {
        for suffix in ["installing", "old"] {
            for contents in ["id = 'other'\nname = 'Other'", "invalid TOML"] {
                let base = std::env::temp_dir().join(format!("ta_package_review_ownership_{suffix}_{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&base);
                let mods = base.join("mods");
                let reserved = mods.join(format!(".testmod.{suffix}"));
                std::fs::create_dir_all(&reserved).unwrap();
                std::fs::write(reserved.join("mod.toml"), contents).unwrap();
                std::fs::write(reserved.join("canary"), b"must survive").unwrap();
                let live = mods.join("testmod");
                std::fs::create_dir_all(&live).unwrap();
                std::fs::write(live.join("mod.toml"), toml::to_string(&manifest("testmod", "1")).unwrap()).unwrap();
                let plan = content(&base, b"new");
                assert!(install(&plan, &manifest("testmod", "2"), &mods).is_err());
                assert_eq!(std::fs::read(reserved.join("canary")).unwrap(), b"must survive");
                assert_eq!(crate::mods::read_mod(&live).unwrap().unwrap().manifest.version, "1");
                assert!(plan.root.join("units/NEW.FBI").exists(), "reject before consuming any source files");
                std::fs::remove_dir_all(base).unwrap();
            }
        }
    }

    #[test]
    fn launcher_review_rejects_archive_output_name_collisions_before_moving_content() {
        for names in [["payload.dat", "payload.gp3"], ["payload.UFO", "payload.ufo"]] {
            // The second case needs a case-sensitive disk to hold both files.
            let base = std::env::temp_dir().join(format!("ta_package_review_names_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&base);
            let source = base.join("source");
            std::fs::create_dir_all(&source).unwrap();
            let paths: Vec<_> = names.iter().map(|name| source.join(name)).collect();
            std::fs::write(&paths[0], b"first").unwrap();
            std::fs::write(&paths[1], b"second").unwrap();
            let plan = Plan { root: source, archives: paths.clone(), ..Plan::default() };
            assert!(install(&plan, &manifest("testmod", "1"), &base.join("mods")).is_err());
            assert!(paths.iter().all(|path| path.exists()));
            if std::fs::read(&paths[0]).unwrap() == b"first" {
                assert_eq!(std::fs::read(&paths[1]).unwrap(), b"second");
            }
            std::fs::remove_dir_all(base).unwrap();
        }
    }

    #[test]
    fn launcher_review_directory_package_cannot_shadow_an_installed_archive() {
        let base = std::env::temp_dir().join(format!("ta_package_review_archive_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mods = base.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        let archive = mods.join("TestMod.UFO");
        std::fs::write(&archive, b"previous archive").unwrap();
        let plan = content(&base, b"new");
        assert!(install(&plan, &manifest("testmod", "2"), &mods).is_err());
        assert_eq!(std::fs::read(archive).unwrap(), b"previous archive");
        assert!(plan.root.join("units/NEW.FBI").exists());
        assert_eq!(crate::mods::scan(&mods).0.len(), 1);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn launcher_review_reserved_work_directory_names_remain_discoverable_mods() {
        let base = std::env::temp_dir().join(format!("ta_package_review_reserved_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let mods = base.join("mods");
        let target = install(&content(&base, b"first"), &manifest(".custom.old", "1"), &mods).unwrap();
        assert_ne!(target, mods.join(".custom.old"), "live mods must not use a transaction directory's name");
        assert_eq!(crate::mods::scan(&mods).0[0].manifest.id, ".custom.old");
        assert_eq!(install(&content(&base, b"updated"), &manifest(".custom.old", "2"), &mods).unwrap(), target);
        assert_eq!(crate::mods::scan(&mods).0.len(), 1);
        std::fs::remove_dir_all(base).unwrap();
    }

    /// A unit to install, as a package's content.
    fn content(base: &Path, text: &[u8]) -> Plan {
        let source = base.join(format!("source-{}", base.read_dir().map_or(0, |d| d.count())));
        std::fs::create_dir_all(source.join("units")).unwrap();
        std::fs::write(source.join("units/NEW.FBI"), text).unwrap();
        plan(&source).unwrap()
    }

    /// A folder already holding another mod (the map the editor saved as "Twilight", where TA:
    /// Twilight's id would go), or one whose manifest does not read, or a file, is never what an
    /// install replaces: the mod goes in beside it, and what was there stays as it was.
    #[test]
    fn an_install_never_replaces_a_folder_holding_something_else() {
        let base = std::env::temp_dir().join(format!("ta_package_other_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mods = base.join("mods");
        std::fs::create_dir_all(mods.join("twilight/maps")).unwrap();
        std::fs::write(mods.join("twilight/mod.toml"), "id = \"map-twilight-0123456789abcdef\"\nname = \"Twilight (map)\"\n").unwrap();
        std::fs::write(mods.join("twilight/maps/Twilight.tnt"), b"the player's own map").unwrap();
        let installed = install(&content(&base, b"new"), &manifest("twilight", "1"), &mods).unwrap();
        assert_eq!(std::fs::read(mods.join("twilight/maps/Twilight.tnt")).unwrap(), b"the player's own map", "the map is kept");
        assert_ne!(installed, mods.join("twilight"));
        assert_eq!(std::fs::read(installed.join("units/NEW.FBI")).unwrap(), b"new");
        let ids = |mods: &Path| { let mut ids: Vec<String> = crate::mods::scan(mods).0.into_iter().map(|m| m.manifest.id).collect(); ids.sort(); ids };
        assert_eq!(ids(&mods), ["map-twilight-0123456789abcdef", "twilight"]);
        // Installed again (an update): the copy in use is replaced, nothing else.
        let again = install(&content(&base, b"newer"), &manifest("twilight", "2"), &mods).unwrap();
        assert_eq!(again, installed);
        assert_eq!(std::fs::read(again.join("units/NEW.FBI")).unwrap(), b"newer");
        assert_eq!(ids(&mods), ["map-twilight-0123456789abcdef", "twilight"]);
        // A folder whose manifest does not read, and a file, are left alone too.
        std::fs::create_dir_all(mods.join("broken")).unwrap();
        std::fs::write(mods.join("broken/mod.toml"), "id = ").unwrap();
        std::fs::write(mods.join("plain"), b"a file").unwrap();
        for id in ["broken", "plain"] {
            let installed = install(&content(&base, b"x"), &manifest(id, "1"), &mods).unwrap();
            assert_ne!(installed, mods.join(id));
        }
        assert_eq!(std::fs::read(mods.join("broken/mod.toml")).unwrap(), b"id = ");
        assert_eq!(std::fs::read(mods.join("plain")).unwrap(), b"a file");
        std::fs::remove_dir_all(base).unwrap();
    }

    /// The same map saved by the editor in a folder named in capitals: the id's folder, on a disk
    /// that does not tell case apart.
    #[cfg(windows)]
    #[test]
    fn an_install_never_replaces_a_folder_differing_only_in_case() {
        let base = std::env::temp_dir().join(format!("ta_package_case_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mods = base.join("mods");
        std::fs::create_dir_all(mods.join("Twilight/maps")).unwrap();
        std::fs::write(mods.join("Twilight/mod.toml"), "# Written by the TA Reborn map editor.\nid = \"map-twilight-0123456789abcdef\"\nname = \"Twilight (map)\"\n").unwrap();
        std::fs::write(mods.join("Twilight/maps/Twilight.tnt"), b"the player's own map").unwrap();
        install(&content(&base, b"new"), &manifest("twilight", "1"), &mods).unwrap();
        assert_eq!(std::fs::read(mods.join("Twilight/maps/Twilight.tnt")).unwrap(), b"the player's own map");
        std::fs::remove_dir_all(base).unwrap();
    }

    /// Two installs of different mods at once (two launchers, or one and `ta-tools`): one going
    /// beside a folder `foo` of another mod's, to `foo-2`, the other publishing the mod `foo-2`
    /// there meanwhile. The first never replaces it: its folder is chosen as it publishes.
    #[test]
    fn an_install_never_replaces_a_mod_another_install_published_meanwhile() {
        let base = std::env::temp_dir().join(format!("ta_package_race_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mods = base.join("mods");
        std::fs::create_dir_all(mods.join("foo")).unwrap();
        std::fs::write(mods.join("foo/mod.toml"), toml::to_string(&manifest("other", "1")).unwrap()).unwrap();
        let (first, second) = (content(&base, b"foo"), content(&base, b"foo-2"));
        let published = install_with_commit_guard(&first, &manifest("foo", "1"), &mods, || {
            // (Meanwhile, after this one has staged its copy: another install, of the mod foo-2.)
            install(&second, &manifest("foo-2", "1"), &mods).map(|_| ())
        }).unwrap();
        assert_eq!(std::fs::read(mods.join("foo-2/units/NEW.FBI")).unwrap(), b"foo-2", "the other install's mod is kept");
        assert_eq!(std::fs::read(published.join("units/NEW.FBI")).unwrap(), b"foo");
        let mut ids: Vec<String> = crate::mods::scan(&mods).0.into_iter().map(|m| m.manifest.id).collect();
        ids.sort();
        assert_eq!(ids, ["foo", "foo-2", "other"]);
        std::fs::remove_dir_all(base).unwrap();
    }

    /// An update replaces the copy the game reads, in its folder whatever that is called (one put
    /// there by hand): it does not land beside it as a second copy the first one hides.
    #[test]
    fn an_update_replaces_the_copy_in_use_wherever_it_is() {
        let base = std::env::temp_dir().join(format!("ta_package_update_in_place_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mods = base.join("mods");
        std::fs::create_dir_all(mods.join("Total Mayhem 11.2")).unwrap();
        std::fs::write(mods.join("Total Mayhem 11.2/mod.toml"), toml::to_string(&manifest("totalmayhem", "11.2")).unwrap()).unwrap();
        let installed = install(&content(&base, b"new"), &manifest("totalmayhem", "11.3"), &mods).unwrap();
        assert_eq!(installed, mods.join("Total Mayhem 11.2"));
        let (found, errors) = crate::mods::scan(&mods);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(found.iter().map(|m| (m.manifest.id.as_str(), m.manifest.version.as_str())).collect::<Vec<_>>(), [("totalmayhem", "11.3")]);
        std::fs::remove_dir_all(base).unwrap();
    }

    /// What an install stopped half way left: an unpublished copy goes, a version moved aside
    /// comes back when it is the only copy, and goes when the swap had finished; nothing is
    /// touched while an install holds the mod, nor the editor's working folders, nor a folder
    /// of another mod's.
    #[test]
    fn what_an_interrupted_install_left_is_put_right() {
        let base = std::env::temp_dir().join(format!("ta_package_recover_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mods = base.join("mods");
        let folder = |name: &str, id: Option<&str>| {
            std::fs::create_dir_all(mods.join(name)).unwrap();
            std::fs::write(mods.join(name).join("canary"), name).unwrap();
            if let Some(id) = id { std::fs::write(mods.join(name).join("mod.toml"), toml::to_string(&manifest(id, "1")).unwrap()).unwrap(); }
        };
        folder(".gone.old", Some("gone"));
        folder(".done.old", Some("done"));
        folder("done", Some("done"));
        folder(".stale.installing", None);
        folder(".busy.installing", None);
        folder(".editor-1-2.installing", None);
        folder(".other.old", Some("not-other"));
        let held = lock_install(&mods, "busy").unwrap();
        let log = recover(&mods);
        assert_eq!(std::fs::read(mods.join("gone/canary")).unwrap(), b".gone.old", "the only copy is back: {log:?}");
        assert!(!mods.join(".gone.old").exists() && !mods.join(".done.old").exists() && !mods.join(".stale.installing").exists(), "{log:?}");
        assert_eq!(std::fs::read(mods.join("done/canary")).unwrap(), b"done");
        for kept in [".busy.installing", ".editor-1-2.installing", ".other.old"] {
            assert!(mods.join(kept).join("canary").is_file(), "{kept} is kept: {log:?}");
        }
        drop(held);
        let ids: Vec<String> = crate::mods::scan(&mods).0.into_iter().map(|m| m.manifest.id).collect();
        assert!(ids.contains(&"gone".to_string()) && ids.contains(&"done".to_string()), "{ids:?}");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn linked_package_content_is_rejected() {
        let base = std::env::temp_dir().join(format!("ta_package_link_{}", std::process::id()));
        std::fs::create_dir_all(base.join("outside")).unwrap();
        std::fs::create_dir_all(base.join("unpacked")).unwrap();
        std::fs::write(base.join("outside/canary"), b"keep").unwrap();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(base.join("outside"), base.join("unpacked/units"));
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(base.join("outside"), base.join("unpacked/units"));
        if linked.is_ok() {
            assert!(plan(&base.join("unpacked")).is_err());
            assert_eq!(std::fs::read(base.join("outside/canary")).unwrap(), b"keep");
            std::fs::remove_dir(base.join("unpacked/units")).or_else(|_| std::fs::remove_file(base.join("unpacked/units"))).unwrap();
        } else {
            eprintln!("symlink test unavailable without platform symlink permission");
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    /// A package laid out like a ModDB download: a folder of its own with an archive (any
    /// extension, as long as it is one), a stub, TA folders, a program, a readme, an extra.
    #[test]
    fn plans_and_installs_the_content_of_a_package() {
        let base = std::env::temp_dir().join(format!("ta_package_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let unpacked = base.join("unpacked");
        let content = unpacked.join("Mod v1.0");
        let write = |p: PathBuf, bytes: &[u8]| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        };
        write(content.join("gamedata/sidedata.tdf"), b"[side0]{name=ARM;}");
        write(content.join("guiM/ARMCOM1.GUI"), b"gui");
        write(content.join("guide/how to play.txt"), b"not a gui");
        write(content.join("units/NEWUNIT.FBI"), b"[UNITINFO]{UnitName=NEWUNIT;}");
        write(content.join("totala1.hpi"), b"");
        // (TA: Enhanced's: a HAPI header and one empty folder, no file.)
        write(content.join("BTDATA.CCX"), &empty_archive());
        // (A header whose directory lies past the file's end: damaged, not a placeholder.)
        write(content.join("damaged.gp3"), b"HAPI\0\0\x01\0\xff\xff\0\0\0\0\0\0\x14\0\0\0");
        write(content.join("TotalA.exe"), b"MZ");
        write(content.join("Readme.txt"), b"hello");
        write(unpacked.join("Extras/optional/units/OTHER.FBI"), b"x");
        // (An extra's placeholder blanks nothing of the mod's.)
        write(unpacked.join("Extras/optional/ccdata.ccx"), b"");
        let plan = plan(&unpacked).unwrap();
        assert_eq!(plan.root, content, "the content's folder, not the extras'");
        assert_eq!(plan.folders.len(), 3, "gamedata, guiM, units: {:?}", plan.folders);
        assert_eq!(plan.rename.get("guim").map(String::as_str), Some("guis"));
        assert!(!plan.rename.contains_key("guide") && plan.skipped.iter().any(|s| s.ends_with("guide/")), "a folder of text is no gui folder");
        assert!(plan.archives.is_empty() && plan.skipped.iter().any(|s| s.contains("totala1.hpi (empty placeholder)")), "{:?}", plan.skipped);
        assert!(plan.skipped.iter().any(|s| s.contains("BTDATA.CCX (empty placeholder)")), "{:?}", plan.skipped);
        assert_eq!(plan.replaces, ["btdata.ccx", "totala1.hpi"], "the placeholders beside the content blank the original's, not an extra's");
        assert!(plan.skipped.iter().any(|s| s.contains("damaged.gp3 (unreadable: ")), "an archive that does not read is told as one: {:?}", plan.skipped);
        assert!(plan.skipped.iter().any(|s| s.ends_with("TotalA.exe")));
        assert_eq!(plan.units, 1);

        let mods = base.join("mods");
        let manifest = ModManifest {
            id: "testmod".into(),
            name: "Test Mod".into(),
            version: "1.0".into(),
            description: String::new(),
            authors: vec!["Someone".into()],
            depends: Vec::new(),
            load_after: Vec::new(),
            source: "https://www.moddb.com/mods/test".into(),
            rename: BTreeMap::new(),
            replaces: Vec::new(),
        };
        let dir = install(&plan, &manifest, &mods).unwrap();
        assert!(dir.join("units/NEWUNIT.FBI").is_file() && dir.join("docs/Readme.txt").is_file() && !dir.join("TotalA.exe").exists());
        assert!(!dir.join("totala1.hpi").exists() && !dir.join("BTDATA.CCX").exists(), "placeholders are not copied: the manifest names them");
        let (found, errors) = crate::mods::scan(&mods);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(found[0].manifest.rename.get("guim").map(String::as_str), Some("guis"));
        assert_eq!(found[0].manifest.source, manifest.source);
        assert_eq!(found[0].manifest.replaces, ["btdata.ccx", "totala1.hpi"]);
        let _ = std::fs::remove_dir_all(base);
    }

    /// An archive with one empty folder and no file, laid out as TA: Enhanced's `btdata.ccx` is.
    pub(crate) fn empty_archive() -> Vec<u8> {
        let bytes = concat!(
            // HAPI, version 1.0, the directory ending at 0x37, no key, starting at 0x14:
            "HAPI\0\0\x01\0", "\x37\0\0\0", "\0\0\0\0", "\x14\0\0\0",
            // the root: one entry, at 0x1c; it is named at 0x25, its folder at 0x2e:
            "\x01\0\0\0", "\x1c\0\0\0", "\x25\0\0\0", "\x2e\0\0\0", "\x01", "unitpics\0",
            // the folder: no entries.
            "\0\0\0\0", "\x36\0\0\0", "\0",
        ).as_bytes().to_vec();
        let archive = ta_formats::hpi::HpiArchive::from_bytes("btdata.ccx", bytes.clone()).expect("an archive");
        assert!(archive.files().is_empty());
        bytes
    }
}
