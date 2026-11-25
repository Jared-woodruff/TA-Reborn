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
    /// The `mod.toml` of the package's content (`path`, where the package has it) does not read,
    /// or says what no mod may (an id of the game's own content's, a renamed folder the game
    /// cannot follow): its author meant it, so the package is not installed by guesswork.
    #[error("the package's {path} does not read: {message}")]
    Manifest { path: String, message: String },
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
        self.unpack_within(archive, into, cancelled, MAX_PACKAGE_ENTRIES, MAX_PACKAGE_BYTES)
    }

    /// [`Unpacker::unpack_cancellable`] within these limits: refused before anything is
    /// unpacked where the archive's listing says it holds more, else stopped as soon as what
    /// comes out passes them.
    fn unpack_within(&self, archive: &Path, into: &Path, cancelled: &dyn Fn() -> bool, max_entries: usize, max_bytes: u64) -> Result<(), PackageError> {
        let archive = std::fs::canonicalize(archive).map_err(io(archive))?;
        std::fs::create_dir_all(into).map_err(io(into))?;
        if crate::is_link(&std::fs::symlink_metadata(into).map_err(io(into))?) || std::fs::read_dir(into).map_err(io(into))?.next().is_some() {
            return Err(PackageError::Unsafe("extraction requires an empty ordinary directory".into()));
        }
        let file = archive.file_name().map_or(String::new(), |f| f.to_string_lossy().to_string());
        let failed = |message: String| PackageError::Unpack { tool: self.name().into(), file: file.clone(), message };
        let too_much = || PackageError::Unsafe(format!("{file} unpacks to more than 8 GiB or 100,000 files"));
        let named = Named::new(self, &archive, into).map_err(io(&archive))?;
        // (Its listing first: a package of more entries than the limits, a hundred thousand tiny
        // files, is refused at once, not after minutes of unpacking. A listing that does not come
        // is no verdict: what comes out is measured as it does.)
        match self.listing(&named, cancelled, max_entries) {
            Ok((entries, bytes)) if entries > max_entries || bytes.is_some_and(|bytes| bytes > max_bytes) => return Err(too_much()),
            Err(PackageError::Cancelled) => return Err(PackageError::Cancelled),
            Err(error) => log::info!("{file}: no listing before unpacking ({error})"),
            Ok(_) => {}
        }
        let mut command = self.command(&named, false);
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
                if past_limits(into, max_entries, max_bytes) {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err(too_much());
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

    /// The tool's command, to list the archive's entries or to unpack it, given `named`.
    fn command(&self, named: &Named, list: bool) -> Command {
        let mut command = match self {
            Unpacker::SevenZip(exe) => {
                let mut c = Command::new(exe);
                if list { c.arg("l").arg("-ba").arg("-slt"); } else { c.arg("x").arg("-y").arg("-bd").arg(format!("-o{}", named.into.display())); }
                c.arg("--").arg(&named.archive);
                c
            }
            Unpacker::Tar(exe) => {
                let mut c = Command::new(exe);
                if list { c.arg("-tf").arg(&named.archive); } else { c.arg("-xf").arg(&named.archive).arg("-C").arg(&named.into); }
                c
            }
        };
        if let Some(folder) = &named.folder { command.current_dir(folder); }
        #[cfg(windows)]
        {
            // No console window flashing up over the launcher.
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        command
    }

    /// What the archive holds by the tool's listing, read before anything is unpacked: how many
    /// entries, and (7-Zip's listing says) how many bytes they unpack to. The reading stops past
    /// `most` entries, the tool stopped with it.
    fn listing(&self, named: &Named, cancelled: &dyn Fn() -> bool, most: usize) -> Result<(usize, Option<u64>), PackageError> {
        use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
        let failed = |message: String| PackageError::Unpack { tool: self.name().into(), file: String::new(), message };
        let mut child = self.command(named, true).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| failed(e.to_string()))?;
        let (entries, bytes) = (std::sync::Arc::new(AtomicUsize::new(0)), std::sync::Arc::new(AtomicU64::new(0)));
        let seven = matches!(self, Unpacker::SevenZip(_));
        let output = child.stdout.take();
        let reader = {
            let (entries, bytes) = (entries.clone(), bytes.clone());
            // (Each line as bytes: names come in whatever encoding the archive keeps them.)
            std::thread::spawn(move || {
                let Some(output) = output else { return };
                for line in std::io::BufRead::split(std::io::BufReader::new(output), b'\n') {
                    let Ok(line) = line else { break };
                    let line = line.strip_suffix(b"\r").unwrap_or(&line);
                    if !seven {
                        if !line.iter().all(u8::is_ascii_whitespace) { entries.fetch_add(1, Ordering::Relaxed); }
                    } else if line.starts_with(b"Path = ") {
                        entries.fetch_add(1, Ordering::Relaxed);
                    } else if let Some(size) = line.strip_prefix(b"Size = ").and_then(|n| std::str::from_utf8(n).ok()?.trim().parse::<u64>().ok()) {
                        bytes.fetch_add(size, Ordering::Relaxed);
                    }
                    if entries.load(Ordering::Relaxed) > most { break; }
                }
            })
        };
        let stop = |child: &mut std::process::Child| { let _ = child.kill(); let _ = child.wait(); };
        let mut outcome = Ok(());
        loop {
            if cancelled() {
                stop(&mut child);
                outcome = Err(PackageError::Cancelled);
                break;
            }
            if entries.load(Ordering::Relaxed) > most {
                stop(&mut child);
                break;
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() { outcome = Err(failed(format!("listing ended with {status}"))); }
                    break;
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
                Err(error) => {
                    stop(&mut child);
                    outcome = Err(failed(error.to_string()));
                    break;
                }
            }
        }
        let _ = reader.join();
        outcome.map(|()| (entries.load(Ordering::Relaxed), seven.then(|| bytes.load(Ordering::Relaxed))))
    }
}

/// The archive and the folder to unpack into, as the tool is given them. Windows' bsdtar reads
/// its arguments in the system's ANSI code page, so a path beyond it (a package named in
/// Cyrillic on a Western Windows, or a user name in another script, in the launcher's working
/// folder) reached it as question marks, and nothing unpacked. There, the archive is linked
/// (or copied) beside the folder under a name of plain letters, and both are given relative to
/// the tool's working folder, which Windows resolves by its whole name. 7-Zip reads any name.
struct Named {
    archive: PathBuf,
    into: PathBuf,
    /// The tool's working folder, where the names are relative to it.
    folder: Option<PathBuf>,
    /// The archive's link, removed with this.
    link: Option<PathBuf>,
}

impl Named {
    fn new(unpacker: &Unpacker, archive: &Path, into: &Path) -> std::io::Result<Named> {
        let plain = |path: &Path| path.to_str().is_some_and(|s| s.is_ascii());
        let as_given = Named { archive: archive.to_path_buf(), into: into.to_path_buf(), folder: None, link: None };
        if !cfg!(windows) || matches!(unpacker, Unpacker::SevenZip(_)) || (plain(archive) && plain(into)) {
            return Ok(as_given);
        }
        let (Some(folder), Some(name)) = (into.parent(), into.file_name().map(Path::new).filter(|name| plain(name))) else { return Ok(as_given) };
        let extension = archive.extension().and_then(|e| e.to_str()).filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or("pkg").to_ascii_lowercase();
        let link_name = format!(".package-{}.{extension}", std::process::id());
        let link = folder.join(&link_name);
        let _ = std::fs::remove_file(&link);
        if std::fs::hard_link(archive, &link).is_err() {
            std::fs::copy(archive, &link)?;
        }
        Ok(Named { archive: PathBuf::from(link_name), into: name.to_path_buf(), folder: Some(folder.to_path_buf()), link: Some(link) })
    }
}

impl Drop for Named {
    fn drop(&mut self) {
        if let Some(link) = &self.link { let _ = std::fs::remove_file(link); }
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
/// A mod's documents, by their extensions (lower case): what an install keeps in its `docs/`, and
/// what the launcher lists as its documents.
pub const DOCUMENT_EXTENSIONS: &[&str] = &["txt", "md", "rtf", "pdf", "doc", "docx", "htm", "html", "nfo"];

/// Folders a package keeps its documents in (`Docs/`, `Documentation/`...), in lower case: the
/// documents at their top go into the mod's `docs/` with those beside the content.
const DOCUMENT_FOLDERS: &[&str] = &["doc", "docs", "documentation", "manual", "manuals", "readme", "help"];

/// Whether a file's name (in lower case) is a document's ([`DOCUMENT_EXTENSIONS`]).
pub fn is_document(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, extension)| DOCUMENT_EXTENSIONS.contains(&extension))
}

pub const TA_FOLDERS: &[&str] = &[
    "ai", "anims", "bitmaps", "camps", "download", "features", "fonts", "gamedata", "guis", "maps", "objects3d", "palettes",
    "scripts", "sounds", "textures", "unitpics", "units", "weapons", "music",
];

/// Folders TA Reborn reads that TA did not: `menus/`, a menu theme's pictures, panel art and list
/// (`menus/<theme>/theme.toml`: Supreme Commander's, which the launcher makes of the player's own
/// copy), and `effects/`, a mod's particle effects (`effects/*.tdf`, ta_data's `effect`). An
/// install keeps them whole, as it does TA's ([`TA_FOLDERS`]); neither plays a part in a battle's
/// outcome, so a mod's digest leaves them out.
pub const REBORN_FOLDERS: &[&str] = &["menus", "effects"];

/// Whether a folder of this name (in lower case) at a mod's top holds what the game reads: one of
/// TA's, or one of TA Reborn's own.
pub fn game_folder(name: &str) -> bool {
    TA_FOLDERS.contains(&name) || REBORN_FOLDERS.contains(&name)
}

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
    /// The mod's pictures and fonts: its [`crate::mods::MEDIA_FOLDER`]'s (of
    /// [`crate::mods::MEDIA_EXTENSIONS`]), and those its manifest names elsewhere beside the
    /// content (`banner.png`). Each file, and where it goes in the mod's folder (`media/banner.png`,
    /// as the package spells it).
    pub media: Vec<(PathBuf, String)>,
    /// The package's own manifest, when its content's folder has one: what it says of itself
    /// (its pictures, links and tags) for an install that names the mod otherwise (the
    /// catalogue's).
    pub authored: Option<ModManifest>,
    /// The text of the package's own manifest ([`Plan::authored`], or one a caller takes from
    /// elsewhere in the package): [`install`] writes the installed copy's `mod.toml` over it,
    /// keeping its comments and the keys TA Reborn does not know ([`crate::mods::write_manifest`]).
    pub authored_text: Option<String>,
}

/// Whether a manifest gives a mod something of its own beyond files of TA's: an interface theme,
/// pictures of itself, or archives of the original game it replaces. A package holding no more is
/// a mod all the same (one that only themes the interface, or only blanks archives).
fn presents(manifest: &ModManifest) -> bool {
    !manifest.theme.is_empty() || !manifest.media.is_empty() || !manifest.replaces.is_empty()
}

/// `path` (in `root`, as a manifest names it: on a disk that does not tell case apart it opens
/// in any case of its letters) as the disk spells it: each of its parts below `root` as its folder
/// lists it (the one of that very name, else one of another case). The mod is installed as the
/// package lays it out.
fn as_on_disk(root: &Path, path: &Path) -> PathBuf {
    let Ok(relative) = path.strip_prefix(root) else { return path.to_path_buf() };
    let mut at = root.to_path_buf();
    for part in relative.components() {
        let part = part.as_os_str();
        let listed = std::fs::read_dir(&at).ok().and_then(|entries| {
            let mut names: Vec<std::ffi::OsString> = entries.flatten().map(|entry| entry.file_name()).collect();
            names.sort();
            let lower = part.to_string_lossy().to_lowercase();
            names.iter().find(|name| name.as_os_str() == part).or_else(|| names.iter().find(|name| name.to_string_lossy().to_lowercase() == lower)).cloned()
        });
        at.push(listed.as_deref().unwrap_or(part));
    }
    at
}

/// Whether `path` is `folder` or in it, the names of their parts compared in any case: a manifest
/// may name a file in another case than the disk has it, and on a disk that does not tell case
/// apart (Windows', macOS's) the named path opens as it is written.
fn within_any_case(path: &Path, folder: &Path) -> bool {
    let mut parts = path.components();
    folder.components().all(|part| parts.next().is_some_and(|other| other.as_os_str().to_string_lossy().to_lowercase() == part.as_os_str().to_string_lossy().to_lowercase()))
}

/// The most a mod's pictures and fonts may weigh, each and all together, and how many there may
/// be: more is left out of an install.
pub const MAX_MEDIA_FILE: u64 = 64 << 20;
const MAX_MEDIA_BYTES: u64 = 256 << 20;
const MAX_MEDIA_FILES: usize = 512;

/// Whether a file's name (any case) is of a kind a mod's media keeps.
pub fn is_media(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, extension)| crate::mods::MEDIA_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
}

/// Whether a path a plan moves within its mod's folder (`media/hud/panel.png`) is one: relative,
/// each part an ordinary name.
fn safe_relative(path: &str) -> bool {
    !path.is_empty() && path.split('/').all(safe_component)
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
    // Every TA archive in the package (a few levels down), and every TA folder; TA Reborn's own
    // folders apart: they tell a manifest's folder holds data, but never where a package without
    // one has its content (a `Menus/` of an old package's own is not the mod).
    let mut archives: Vec<(PathBuf, u64)> = Vec::new();
    let mut folder_parents: Vec<PathBuf> = Vec::new();
    let mut reborn_parents: Vec<PathBuf> = Vec::new();
    let mut stubs: Vec<PathBuf> = Vec::new();
    let mut unreadable: Vec<(PathBuf, String)> = Vec::new();
    let mut manifests = Vec::new();
    walk(unpacked, 0, &mut |path, is_dir| {
        if is_dir {
            let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().to_ascii_lowercase());
            if let Some(parent) = path.parent() {
                if TA_FOLDERS.contains(&name.as_str()) {
                    folder_parents.push(parent.to_path_buf());
                } else if REBORN_FOLDERS.contains(&name.as_str()) {
                    reborn_parents.push(parent.to_path_buf());
                }
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
    // (Else, the shallowest of a mod that brings nothing of TA's: one that only themes the
    // interface, presents itself or blanks archives. Taken only where nothing else is found.)
    let mut presenting = None;
    for dir in manifests {
        // Probe candidates without letting skipped examples or extras invalidate the
        // package. The selected root's metadata is read and validated below.
        let Ok(Some(own)) = crate::mods::read_mod(&dir) else { continue };
        if folder_parents.contains(&dir) || reborn_parents.contains(&dir) || archives.iter().any(|(path, _)| path.parent() == Some(dir.as_path()))
            || renamed_folders(&own.manifest.rename).is_ok_and(|renamed| renamed.iter().any(|name| crate::install::find_dir_ci(&dir, name).is_some())) {
            authored = Some(dir);
            break;
        }
        if presenting.is_none() && presents(&own.manifest) {
            presenting = Some(dir);
        }
    }
    // Without an authored layout, content is beside the largest archive, else TA's
    // shallowest folder, as in the original community packages.
    let root = match authored {
        Some(root) => root,
        None => match archives.iter().max_by_key(|(_, size)| *size) {
            Some((path, _)) => path.parent().unwrap_or(unpacked).to_path_buf(),
            None => match folder_parents.iter().min_by_key(|p| p.components().count()) {
                Some(parent) => parent.clone(),
                None => presenting.ok_or(PackageError::NoContent)?,
            },
        },
    };
    let rel = |p: &Path| p.strip_prefix(unpacked).unwrap_or(p).to_string_lossy().replace('\\', "/");
    let manifest_file = crate::mods::manifest_path(&root);
    let has_manifest = std::fs::symlink_metadata(&manifest_file).is_ok();
    let (own, authored_text) = if has_manifest {
        // (Its author meant it: one that does not read is told as that, where the package has it.)
        let unreadable = |message: String| PackageError::Manifest { path: rel(&manifest_file), message };
        let own = match crate::mods::read_mod(&root) {
            Ok(own) => own.ok_or(PackageError::NoContent)?.manifest,
            Err(crate::mods::ModError::Manifest { message, .. }) => return Err(unreadable(message)),
            Err(error) => return Err(unreadable(error.to_string())),
        };
        (own, Some(crate::mods::read_manifest(&manifest_file).map_err(|error| unreadable(error.to_string()))?))
    } else {
        // (Named by its folder, as a folder without a manifest is, whatever that is called: the
        // install names the mod itself, so a folder named like the game's own content, `Base/`,
        // is no matter.)
        (crate::mods::implicit_folder_manifest(&root), None)
    };
    let declared = renamed_folders(&own.rename)?;
    let mut plan = Plan { root: root.clone(), rename: own.rename.clone(), authored: has_manifest.then(|| own.clone()), authored_text, ..Default::default() };
    // The pictures (and its theme's files) its manifest names beside the content, wherever they
    // are there, in the order it names them (those in a folder kept whole come with it: below).
    let mut named: Vec<PathBuf> = Vec::new();
    for path in own.media.files().chain(own.theme.files()).filter_map(|file| crate::mods::media_path(&root, file)) {
        let path = as_on_disk(&root, &path);
        if path.file_name().is_some_and(|name| is_media(&name.to_string_lossy())) && !named.contains(&path) {
            named.push(path);
        }
    }
    let mut media_folders = Vec::new();
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
    // (Folders of documents after the documents beside the content: those take their names.)
    let mut document_folders = Vec::new();
    for path in entries {
        let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().to_ascii_lowercase());
        if path.is_dir() {
            // (A folder with a name of the mod's own stays only if it proves to be one.)
            if game_folder(&name) || declared.contains(&name) || rename_candidate(&name).is_some() {
                plan.folders.push(path);
            } else if DOCUMENT_FOLDERS.contains(&name.as_str()) {
                document_folders.push(path);
            } else if name == crate::mods::MEDIA_FOLDER {
                media_folders.push(path);
            } else {
                plan.skipped.push(format!("{}/", rel(&path)));
            }
        } else if plan.archives.contains(&path) || stubs.contains(&path) || unreadable.iter().any(|(p, _)| *p == path) {
        } else if named.contains(&path) {
            // (One of its pictures: below.)
        } else if has_manifest && name == "mod.toml" {
            // (Its manifest: the installed copy's is written over it.)
        } else if is_document(&name) {
            plan.docs.push(path);
        } else {
            plan.skipped.push(rel(&path));
        }
    }
    // A folder of documents: those at its top join the rest, one of a name taken once (on every
    // system alike); the rest of it, pictures and folders, stays behind.
    for folder in document_folders {
        let mut documents: Vec<PathBuf> = std::fs::read_dir(&folder).map_err(io(&folder))?.flatten().map(|e| e.path())
            .filter(|p| p.is_file() && p.file_name().is_some_and(|n| is_document(&n.to_string_lossy().to_ascii_lowercase()))).collect();
        documents.sort();
        let named = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().to_lowercase());
        for document in documents {
            if plan.docs.iter().any(|taken| named(taken) == named(&document)) {
                plan.skipped.push(rel(&document));
            } else {
                plan.docs.push(document);
            }
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
        game_folder(&name) || declared.contains(&name) || rename.contains_key(&name)
    });
    for f in std::fs::read_dir(&root).map_err(io(&root))?.flatten().map(|e| e.path()) {
        let name = f.file_name().map_or(String::new(), |n| n.to_string_lossy().to_ascii_lowercase());
        if f.is_dir() && !game_folder(&name) && !declared.contains(&name) && rename_candidate(&name).is_some() && !rename.contains_key(&name) {
            plan.skipped.push(format!("{}/", f.strip_prefix(&unpacked_root).unwrap_or(&f).to_string_lossy().replace('\\', "/")));
        }
    }
    // Its pictures and fonts, once the folders it keeps whole are known (one named like TA's that
    // proved to be none is not kept: a picture named in it is taken on its own). Those its
    // manifest names first, wherever they are beside the content (outside the folders kept whole,
    // which bring theirs), so that none of the rest crowds them out; then the media folder's (of
    // their kinds, as many as may be).
    let mut weight = 0u64;
    let in_root = |path: &Path| path.strip_prefix(&root).ok().map(|p| p.to_string_lossy().replace('\\', "/"));
    let take = |plan: &mut Plan, path: &Path, weight: &mut u64| {
        let Some(to) = in_root(path).filter(|to| safe_relative(to)) else { plan.skipped.push(rel(path)); return };
        let size = std::fs::metadata(path).map_or(u64::MAX, |m| m.len());
        if !path.file_name().is_some_and(|name| is_media(&name.to_string_lossy())) {
            plan.skipped.push(format!("{} (not a picture or a font)", rel(path)));
        } else if plan.media.iter().any(|(_, taken)| taken.to_lowercase() == to.to_lowercase()) {
            // (Named, and in the media folder too: taken once.)
        } else if size > MAX_MEDIA_FILE || weight.saturating_add(size) > MAX_MEDIA_BYTES || plan.media.len() >= MAX_MEDIA_FILES {
            plan.skipped.push(format!("{} (beyond what a mod's pictures may weigh)", rel(path)));
        } else {
            *weight += size;
            plan.media.push((path.to_path_buf(), to));
        }
    };
    let outside: Vec<PathBuf> = named.iter().filter(|path| !plan.folders.iter().any(|folder| within_any_case(path, folder))).cloned().collect();
    for path in &outside { take(&mut plan, path, &mut weight); }
    for folder in &media_folders {
        let mut files = Vec::new();
        walk(folder, 0, &mut |path, is_dir| if !is_dir { files.push(path.to_path_buf()) });
        files.sort();
        for file in files { take(&mut plan, &file, &mut weight); }
    }
    // (A manifest of a mod that brings nothing of TA's but its theme, its pictures or the archives
    // it replaces is a mod all the same.)
    if plan.archives.is_empty() && plan.folders.is_empty() && !plan.authored.as_ref().is_some_and(presents) {
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
    if game_folder(&name) {
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
    // content root can be moved (and documents in a folder of documents there), and no
    // destination may alias an unpacked source.
    let in_document_folder = |path: &PathBuf| path.parent().is_some_and(|folder| {
        folder.parent() == Some(plan.root.as_path())
            && folder.file_name().is_some_and(|name| DOCUMENT_FOLDERS.contains(&name.to_string_lossy().to_lowercase().as_str()))
    });
    for path in plan.archives.iter().chain(&plan.folders).chain(&plan.docs) {
        if path.parent() != Some(plan.root.as_path()) && !(plan.docs.contains(path) && in_document_folder(path)) {
            return Err(PackageError::Unsafe("planned file is outside the content root".into()));
        }
    }
    // (Its pictures and fonts: files in the content's folder, each going to a place of ordinary
    // names in the mod's, beside none of the rest.)
    for (path, to) in &plan.media {
        if !path.starts_with(&plan.root) || !safe_relative(to) || !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
            return Err(PackageError::Unsafe(format!("planned picture {to} is not a file in the content root")));
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
    // (Each picture a place of its own: no file there already, in the documents' folder too
    // (pictures may share it, and their own folders, with each other and with the documents), a
    // picture beside the manifest not named as an archive or a folder is, a picture's folder not
    // an archive's name.)
    let archive_names: std::collections::BTreeSet<String> = plan.archives.iter().map(|p| archive_name(p).to_lowercase()).collect();
    for (_, to) in &plan.media {
        let to = to.to_lowercase();
        let top = to.split('/').next().unwrap_or_default();
        if names.contains(&to) || (to.contains('/') && archive_names.contains(top)) || !names.insert(to.clone()) {
            return Err(PackageError::Unsafe(format!("multiple package entries install as {to}")));
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
    for path in [&staging, &old] {
        if !path.exists() { continue; }
        // (Its reserved name is the installer's own: one without a manifest is a copy staged and
        // never finished, or a version replaced that had none, a folder installed by hand.
        // One with a manifest must be this mod's.)
        if !crate::mods::manifest_path(path).exists() { continue; }
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
        for (path, to) in &plan.media {
            let target = staging.join(to);
            if let Some(folder) = target.parent() { std::fs::create_dir_all(folder).map_err(io(folder))?; }
            move_path(path, &target)?;
        }
        let mut manifest = manifest.clone();
        manifest.rename.extend(plan.rename.clone());
        for name in &plan.replaces {
            if !manifest.replaces.contains(name) { manifest.replaces.push(name.clone()); }
        }
        // (Over the package's own, where it has one: its comments, and keys this TA Reborn does not
        // know, a later one's, kept.)
        let text = crate::mods::write_manifest(plan.authored_text.as_deref(), &manifest)
            .map_err(|e| PackageError::Io { path: "mod.toml".into(), source: std::io::Error::other(e) })?;
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
        && let Err(error) = discard(mods_dir, &old)
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
    if target.exists() && let Err(error) = rename_patiently(&target, &old) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(io(&target)(error));
    }
    if let Err(error) = rename_patiently(&staging, &target) {
        if old.exists() { let _ = rename_patiently(&old, &target); }
        let _ = std::fs::remove_dir_all(&staging);
        return Err(io(&staging)(error));
    }
    drop(publishing);
    drop(commit);
    // (The swap itself on the disk too, where a folder can be flushed.)
    #[cfg(unix)]
    let _ = std::fs::File::open(mods_dir).and_then(|dir| dir.sync_all());
    let _ = discard(mods_dir, &old);
    Ok(target)
}

/// How long a rename of the swap waits for a file in the way to be let go of.
const RENAME_PATIENCE: std::time::Duration = std::time::Duration::from_secs(5);

/// A rename of the swap, tried again for a few seconds where Windows refuses it because a file
/// in the folder is open for a moment: an antivirus reading the files just written, a check or
/// a picture still reading the version replaced. (The map editor's swap too.)
pub fn rename_patiently(from: &Path, to: &Path) -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + RENAME_PATIENCE;
    loop {
        match std::fs::rename(from, to) {
            Err(error) if in_use(&error) && std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(100)),
            result => return result,
        }
    }
}

/// Whether Windows refused a file operation because something has a file open (access denied,
/// a sharing violation, a lock).
fn in_use(error: &std::io::Error) -> bool {
    cfg!(windows) && (error.kind() == std::io::ErrorKind::PermissionDenied || matches!(error.raw_os_error(), Some(5 | 32 | 33)))
}

/// The folders of `mods_dir` being discarded (`.trash-<n>`): what is left of one is swept away
/// by [`recover`].
pub(crate) const TRASH_PREFIX: &str = ".trash-";

/// Removes a version replaced (`path`, in `mods_dir`): renamed out of its reserved name first,
/// in one step, then deleted. Stopped half way (the launcher closed, the power cut), what is
/// left is a folder of no mod's that [`recover`] sweeps away, never a version half deleted
/// under the name the next install of the mod would take for one of its own. (The map editor
/// discards the copy of a map it replaced so too.)
pub fn discard(mods_dir: &Path, path: &Path) -> Result<(), PackageError> {
    if std::fs::symlink_metadata(path).is_err() { return Ok(()); }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let trash = mods_dir.join(format!("{TRASH_PREFIX}{}-{nanos}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    rename_patiently(path, &trash).map_err(io(path))?;
    // (Best effort: what stays behind is swept away by the next start.)
    let _ = std::fs::remove_dir_all(&trash);
    Ok(())
}

/// Clears Windows' read-only mark of the file at `path`, if it has one: files from a CD, and
/// 7-Zip keeps the mark as it unpacks them. A mod's files so marked could not be flushed to the
/// disk before an update, nor replaced by the next one. (Elsewhere nothing: a file's mode is
/// its owner's to set.)
pub fn clear_read_only(path: &Path) {
    #[cfg(windows)]
    if let Ok(meta) = std::fs::symlink_metadata(path) && meta.is_file() && meta.permissions().readonly() {
        let mut permissions = meta.permissions();
        // (Windows' read-only attribute, not a Unix mode: nothing is made writable to others.)
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        let _ = std::fs::set_permissions(path, permissions);
    }
    #[cfg(not(windows))]
    let _ = path;
}

/// Flushes a staged copy's files (and on Unix its folders) to the disk before it takes the
/// installed copy's place, the previous version then removed at once: the unpacker's output, a
/// copy across drives and `mod.toml` are written without it, and a power cut just after an
/// update could otherwise leave the new copy half written with the old one gone. A file marked
/// read-only (Windows flushes only through a handle that may write) has the mark cleared first;
/// on Unix one the owner may not write is flushed through a handle that reads.
fn sync_tree(root: &Path) {
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                let path = entry.path();
                clear_read_only(&path);
                // (Write access: Windows flushes a file only through a handle that may write.)
                let flushed = std::fs::OpenOptions::new().write(true).open(&path).and_then(|file| file.sync_all());
                #[cfg(unix)]
                if flushed.is_err() { let _ = std::fs::File::open(&path).and_then(|file| file.sync_all()); }
                #[cfg(not(unix))]
                let _ = flushed;
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
    let entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
    // (A version replaced whose deletion was stopped half way: no mod's, no name a mod takes.)
    for entry in &entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(TRASH_PREFIX) && entry.file_type().is_ok_and(|t| t.is_dir()) {
            match std::fs::remove_dir_all(entry.path()) {
                Ok(()) => done.push(format!("removed what was left of a replaced version ({name})")),
                Err(e) => done.push(format!("could not remove what was left of a replaced version ({name}): {e}")),
            }
        }
    }
    let ids: std::collections::BTreeSet<String> = entries.iter()
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
                Ok(Some(m)) => m.manifest.id == id || unfinished && !crate::mods::manifest_path(dir).exists(),
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
        // (Without a manifest too: its reserved name is the installer's own, and a version
        // installed by hand has none.)
        if !ours(&old, &id, true) { continue; }
        // (Its folder chosen and taken as an install's is: one at a time.)
        let Ok(_publishing) = lock_publication(mods_dir) else { continue };
        let live = crate::mods::scan(mods_dir).0.iter().any(|m| m.manifest.id == id);
        if live {
            match discard(mods_dir, &old) {
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
    // (A map editor's save stopped half way: put right while no editor saves, its lock free.)
    if entries.iter().any(|entry| entry.file_name().to_str().is_some_and(|name| editor_backup(name).is_some()))
        && let Ok((_, lock)) = open_lock(mods_dir.join(EDITOR_SAVE_LOCK))
        && lock.try_lock().is_ok()
        && let Ok(_publishing) = lock_publication(mods_dir)
    {
        done.extend(restore_editor_backups(mods_dir));
    }
    done
}

/// The lock the map editor holds on a mods folder while it saves a map there: what a save stopped
/// half way left is put right ([`restore_editor_backups`]) only while no save holds it.
pub const EDITOR_SAVE_LOCK: &str = ".editor-save.lock";

/// The folder the map editor moves the previous copy of a map's folder (`folder`, in the mods
/// folder) aside to while it puts the new copy in its place (`work`: the save's own working name,
/// `.editor-<pid>-<n>`): named after the map's folder, so that a save stopped between its two
/// steps can be put right. Hidden from discovery, as an install's working folders are.
pub fn editor_backup_name(work: &str, folder: &str) -> String {
    format!("{work}-{folder}.old")
}

/// A name of the map editor's backups read ([`editor_backup_name`]): the map's folder it was
/// moved aside from, or None for one an editor from before that named it so left
/// (`.editor-<pid>-<n>.old`). None of either: no backup of the editor's.
fn editor_backup(name: &str) -> Option<Option<String>> {
    let rest = name.strip_prefix(".editor-")?.strip_suffix(".old")?;
    let digits = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit());
    let (pid, rest) = rest.split_once('-')?;
    if !digits(pid) { return None; }
    match rest.split_once('-') {
        Some((n, folder)) if digits(n) && !folder.is_empty() => Some(Some(folder.to_string())),
        None if digits(rest) => Some(None),
        _ => None,
    }
}

/// Where a backup an older map editor left belongs ([`editor_backup`]: its name does not tell): the
/// folder named like the one map it holds (`maps/<name>.tnt`, the editor's folder for it). None
/// when it holds none, or several.
fn older_editor_backup_folder(backup: &Path) -> Option<String> {
    let maps = crate::install::find_dir_ci(backup, "maps")?;
    let stems: Vec<String> = std::fs::read_dir(maps).ok()?.flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            let (stem, extension) = name.rsplit_once('.')?;
            (extension.eq_ignore_ascii_case("tnt") && entry.file_type().is_ok_and(|t| t.is_file())).then(|| stem.to_string())
        })
        .collect();
    match stems.as_slice() {
        [stem] => Some(stem.clone()),
        _ => None,
    }
}

/// Puts right what a map editor's save stopped half way (the game closed by force, the power
/// gone) left in `mods_dir`: the previous copy of a map's folder, moved aside for the swap
/// ([`editor_backup_name`]), goes back to its folder when that is missing (discovery hides the
/// copy aside: the map was gone), and is discarded when the folder holds the same map again (the
/// swap had finished: the same mod id). One an older editor left, whose name does not tell its
/// folder, goes back to the folder named like the one map it holds when that is missing and no mod
/// there has its id. Anything else is left as it is, and said. The caller holds the editor's save
/// lock ([`EDITOR_SAVE_LOCK`]: the editor's next save does, [`recover`] takes it). What was done,
/// for the log.
pub fn restore_editor_backups(mods_dir: &Path) -> Vec<String> {
    let mut done = Vec::new();
    let Ok(entries) = std::fs::read_dir(mods_dir) else { return done };
    let mut backups: Vec<(String, PathBuf, Option<String>)> = entries.flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            let folder = editor_backup(&name)?;
            Some((name, entry.path(), folder))
        })
        .collect();
    backups.sort();
    let id_of = |dir: &Path| crate::mods::read_mod(dir).ok().flatten().map(|m| m.manifest.id);
    for (name, backup, folder) in backups {
        let older = folder.is_none();
        let Some(folder) = folder.or_else(|| older_editor_backup_folder(&backup)).filter(|folder| safe_component(folder)) else {
            done.push(format!("left {name} as it is: which map's folder it was is not known"));
            continue;
        };
        let target = mods_dir.join(&folder);
        let id = id_of(&backup);
        match std::fs::symlink_metadata(&target) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if older && id.as_ref().is_some_and(|id| crate::mods::scan(mods_dir).0.iter().any(|m| m.manifest.id == *id)) {
                    done.push(format!("left {name} as it is: the map {folder} is installed elsewhere"));
                    continue;
                }
                match rename_patiently(&backup, &target) {
                    Ok(()) => done.push(format!("restored the map {folder} (its save had stopped half way) from {name}")),
                    Err(e) => done.push(format!("could not restore the map {folder} from {name}: {e}")),
                }
            }
            Ok(_) if id.is_some() && id_of(&target) == id => match discard(mods_dir, &backup) {
                Ok(()) => done.push(format!("removed the replaced copy of the map {folder} ({name})")),
                Err(e) => done.push(format!("could not remove the replaced copy of the map {folder} ({name}): {e}")),
            },
            Ok(_) => done.push(format!("left {name} as it is: {folder} holds another mod")),
            Err(e) => done.push(format!("left {name} as it is: {folder}: {e}")),
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
        std::fs::copy(from, to).map_err(|error| {
            // (Named for the one at fault: the file moved, gone or not to be opened, else where it goes.)
            let source = std::fs::File::open(from).is_err();
            io(if source { from } else { to })(error)
        })?;
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
            // (Told as the manifest that does not read, where the package has it, with why: not as
            // unsafe content at a path of the cache's.)
            let result = plan(&root);
            assert!(matches!(&result, Err(PackageError::Manifest { path, message }) if path == "Main/mod.toml" && !message.is_empty()), "selected metadata must fail validation: {invalid}: {result:?}");
            let said = result.unwrap_err().to_string();
            assert!(said.starts_with("the package's Main/mod.toml does not read: ") && !said.contains(&root.display().to_string()), "{said}");
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

    /// A package of more entries than the limits is refused from its listing, before anything
    /// is unpacked; within them, it unpacks. (Windows' own tar makes the package, and lists it.)
    #[cfg(windows)]
    #[test]
    fn a_package_of_too_many_entries_is_refused_from_its_listing() {
        let Some(tar) = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32").join("tar.exe")).filter(|p| p.is_file()) else { return };
        let base = std::env::temp_dir().join(format!("ta_package_listing_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let input = base.join("input");
        std::fs::create_dir_all(input.join("units")).unwrap();
        for n in 0..5 { std::fs::write(input.join(format!("units/u{n}.fbi")), b"[UNITINFO]{UnitName=U;}").unwrap(); }
        let zip = base.join("mod.zip");
        assert!(Command::new(&tar).arg("-a").arg("-cf").arg(&zip).arg("-C").arg(&input).arg("units").status().unwrap().success());
        for unpacker in [Some(Unpacker::Tar(tar.clone())), Unpacker::find()].into_iter().flatten() {
            let refused = base.join(format!("refused-{}", unpacker.name()));
            assert!(matches!(unpacker.unpack_within(&zip, &refused, &|| false, 4, 1 << 30), Err(PackageError::Unsafe(_))), "{}", unpacker.name());
            assert_eq!(std::fs::read_dir(&refused).unwrap().count(), 0, "nothing was unpacked ({})", unpacker.name());
            let whole = base.join(format!("whole-{}", unpacker.name()));
            unpacker.unpack_within(&zip, &whole, &|| false, 10, 1 << 30).unwrap();
            assert_eq!(std::fs::read_dir(whole.join("units")).unwrap().count(), 5);
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    /// Windows' tar reads its arguments in the system's ANSI code page: a package, and a folder,
    /// named beyond it (Cyrillic on a Western Windows) unpack all the same, nothing left beside.
    #[cfg(windows)]
    #[test]
    fn windows_tar_unpacks_a_package_named_in_any_script() {
        let Some(tar) = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32").join("tar.exe")).filter(|p| p.is_file()) else { return };
        let root = std::env::temp_dir().join(format!("ta_package_named_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // (Made under plain names, which the tool reads, then moved under the others.)
        let input = root.join("input");
        std::fs::create_dir_all(input.join("units")).unwrap();
        std::fs::write(input.join("units/test.fbi"), b"[UNITINFO]{UnitName=TEST;}").unwrap();
        let made = root.join("made.zip");
        assert!(Command::new(&tar).arg("-a").arg("-cf").arg(&made).arg("-C").arg(&input).arg("units").status().unwrap().success());
        let base = root.join("Моды и карты");
        std::fs::create_dir_all(&base).unwrap();
        let zip = base.join("Мод 1.2.zip");
        std::fs::rename(&made, &zip).unwrap();
        let output = base.join("unpacked");
        Unpacker::Tar(tar).unpack(&zip, &output).unwrap();
        assert_eq!(std::fs::read(output.join("units/test.fbi")).unwrap(), b"[UNITINFO]{UnitName=TEST;}");
        let beside: Vec<String> = std::fs::read_dir(&base).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.starts_with(".package-")).collect();
        assert!(beside.is_empty(), "the link is gone: {beside:?}");
        assert!(zip.is_file(), "the package itself stays");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A package's documents go into the mod's `docs/`: those beside its content, and those at the
    /// top of a folder of documents of its own (`Docs/`), one of a name once; the rest of that
    /// folder stays behind.
    #[test]
    fn a_packages_documents_folder_is_kept() {
        let base = std::env::temp_dir().join(format!("ta_package_docs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let source = base.join("source");
        std::fs::create_dir_all(source.join("units")).unwrap();
        std::fs::write(source.join("units/NEW.FBI"), b"[UNITINFO]{UnitName=NEW;}").unwrap();
        std::fs::write(source.join("Readme.txt"), b"beside").unwrap();
        std::fs::create_dir_all(source.join("Docs/pictures")).unwrap();
        std::fs::write(source.join("Docs/Guide.html"), b"guide").unwrap();
        std::fs::write(source.join("Docs/Credits.NFO"), b"credits").unwrap();
        std::fs::write(source.join("Docs/readme.txt"), b"in the folder").unwrap();
        std::fs::write(source.join("Docs/pictures/shot.png"), b"png").unwrap();
        let planned = plan(&source).unwrap();
        let mut names: Vec<String> = planned.docs.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        names.sort();
        assert_eq!(names, ["Credits.NFO", "Guide.html", "Readme.txt"]);
        let mods = base.join("mods");
        let installed = install(&planned, &manifest("docmod", "1"), &mods).unwrap();
        assert!(installed.join("docs/Guide.html").is_file() && installed.join("docs/Credits.NFO").is_file());
        assert_eq!(std::fs::read(installed.join("docs/Readme.txt")).unwrap(), b"beside", "the one beside the content");
        assert!(!installed.join("docs/pictures").exists());
        assert!(is_document("manual.docx") && !is_document("shot.png"));
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
            let manifest = ModManifest { id: id.into(), name: "test".into(), version: "1".into(), description: String::new(), authors: Vec::new(), depends: Vec::new(), load_after: Vec::new(), source: String::new(), rename: BTreeMap::new(), replaces: Vec::new(), ..Default::default() };
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
        let manifest = ModManifest { id: "testmod".into(), name: "Test".into(), version: "1".into(), description: String::new(), authors: Vec::new(), depends: Vec::new(), load_after: Vec::new(), source: String::new(), rename: BTreeMap::new(), replaces: Vec::new(), ..Default::default() };
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
        let manifest = ModManifest { id: "testmod".into(), name: "Test".into(), version: "1".into(), description: String::new(), authors: Vec::new(), depends: Vec::new(), load_after: Vec::new(), source: String::new(), rename: BTreeMap::new(), replaces: Vec::new(), ..Default::default() };
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
            let manifest = ModManifest { id: "testmod".into(), name: "Test".into(), version: "2".into(), description: String::new(), authors: Vec::new(), depends: Vec::new(), load_after: Vec::new(), source: String::new(), rename: BTreeMap::new(), replaces: Vec::new(), ..Default::default() };
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
        ModManifest { id: id.into(), name: id.into(), version: version.into(), description: String::new(), authors: Vec::new(), depends: Vec::new(), load_after: Vec::new(), source: String::new(), rename: BTreeMap::new(), replaces: Vec::new(), ..Default::default() }
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
    /// A package's pictures and fonts are kept: its media folder's (of their kinds only), and a
    /// picture its manifest names beside the content; the installed manifest keeps how it names
    /// them, its links and its tags, and the plan the package's own manifest (a catalogue install
    /// takes its pictures from it).
    #[test]
    fn an_install_keeps_the_mods_pictures_and_fonts() {
        let base = std::env::temp_dir().join(format!("ta_package_media_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let source = base.join("package/My Mod");
        std::fs::create_dir_all(source.join("units")).unwrap();
        std::fs::create_dir_all(source.join("Media/hud")).unwrap();
        std::fs::write(source.join("units/NEW.FBI"), b"[UNITINFO]{UnitName=NEW;}").unwrap();
        std::fs::write(source.join("mod.toml"), "id = \"mymod\"\nname = \"My Mod\"\ntags = [\"units\", \"balance\"]\n\n[media]\nbanner = \"banner.png\"\npreview = \"Media/gameplay.gif\"\nicon = \"media/icon.png\"\nscreenshots = [\"media/shot1.jpg\"]\n\n[links]\nDiscord = \"https://discord.gg/example\"\n").unwrap();
        std::fs::write(source.join("banner.png"), b"png").unwrap();
        std::fs::write(source.join("Media/gameplay.gif"), b"gif").unwrap();
        std::fs::write(source.join("Media/icon.png"), b"png").unwrap();
        std::fs::write(source.join("Media/shot1.jpg"), b"jpg").unwrap();
        std::fs::write(source.join("Media/hud/panel.png"), b"png").unwrap();
        std::fs::write(source.join("Media/hud/Font.TTF"), b"ttf").unwrap();
        std::fs::write(source.join("Media/notes.psd"), b"psd").unwrap();
        std::fs::write(source.join("cover.png"), b"not named").unwrap();
        std::fs::write(source.join("TotalA.exe"), b"MZ").unwrap();
        let plan = plan(&base.join("package")).unwrap();
        let mut kept: Vec<&str> = plan.media.iter().map(|(_, to)| to.as_str()).collect();
        kept.sort();
        assert_eq!(kept, ["Media/gameplay.gif", "Media/hud/Font.TTF", "Media/hud/panel.png", "Media/icon.png", "Media/shot1.jpg", "banner.png"]);
        assert!(plan.skipped.iter().any(|s| s.contains("notes.psd")), "not a picture or a font: {:?}", plan.skipped);
        assert!(plan.skipped.iter().any(|s| s.ends_with("cover.png")), "a picture the manifest does not name stays behind: {:?}", plan.skipped);
        let own = plan.authored.clone().expect("the package's own manifest");
        assert_eq!(own.media.banner, "banner.png");
        let mods = base.join("mods");
        let dir = install(&plan, &own, &mods).unwrap();
        for file in ["banner.png", "Media/gameplay.gif", "Media/hud/panel.png", "Media/hud/Font.TTF"] {
            assert!(dir.join(file).is_file(), "{file} installed");
        }
        assert!(!dir.join("cover.png").exists() && !dir.join("TotalA.exe").exists() && !dir.join("Media/notes.psd").exists());
        let (found, errors) = crate::mods::scan(&mods);
        assert!(errors.is_empty(), "{errors:?}");
        let manifest = &found[0].manifest;
        assert_eq!(manifest.tags, ["units", "balance"]);
        assert_eq!(manifest.media.preview, "Media/gameplay.gif");
        assert_eq!(manifest.links.get("Discord").map(String::as_str), Some("https://discord.gg/example"));
        // (Named in another case than the disk's, each is found.)
        assert!(crate::mods::media_path(&dir, &manifest.media.icon).is_some_and(|path| path.is_file()));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// A picture the manifest names in the documents' folder installs beside the documents: the
    /// folder is shared, only one file of a name there.
    #[test]
    fn a_named_picture_shares_the_documents_folder() {
        let base = std::env::temp_dir().join(format!("ta_package_docs_media_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let source = base.join("package");
        std::fs::create_dir_all(source.join("units")).unwrap();
        std::fs::create_dir_all(source.join("docs")).unwrap();
        std::fs::write(source.join("units/NEW.FBI"), b"[UNITINFO]{UnitName=NEW;}").unwrap();
        std::fs::write(source.join("mod.toml"), "id = \"docsmod\"
name = \"Docs Mod\"

[media]
banner = \"docs/banner.png\"
").unwrap();
        std::fs::write(source.join("Readme.txt"), b"read me").unwrap();
        std::fs::write(source.join("docs/banner.png"), b"png").unwrap();
        let plan = plan(&source).unwrap();
        assert_eq!(plan.media.iter().map(|(_, to)| to.as_str()).collect::<Vec<_>>(), ["docs/banner.png"]);
        let own = plan.authored.clone().unwrap();
        let dir = install(&plan, &own, &base.join("mods")).expect("pictures and documents share docs/");
        assert!(dir.join("docs/banner.png").is_file() && dir.join("docs/Readme.txt").is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

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
        // (A version installed by hand, with no manifest, moved aside for an update that stopped
        // there; one whose deletion stopped half way, its manifest gone first; and what was left
        // of one being discarded.)
        folder(".bare.old", None);
        folder(".halfgone.old", None);
        folder("halfgone", Some("halfgone"));
        folder(".trash-1-2-3", None);
        let held = lock_install(&mods, "busy").unwrap();
        let log = recover(&mods);
        assert_eq!(std::fs::read(mods.join("gone/canary")).unwrap(), b".gone.old", "the only copy is back: {log:?}");
        assert_eq!(std::fs::read(mods.join("bare/canary")).unwrap(), b".bare.old", "the only copy is back, manifest or not: {log:?}");
        assert!(!mods.join(".halfgone.old").exists() && mods.join("halfgone/canary").is_file(), "{log:?}");
        assert!(!mods.join(".trash-1-2-3").exists(), "{log:?}");
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

    /// A version replaced whose deletion stopped half way (its manifest went first) holds up no
    /// later install of the mod: its reserved name is the installer's own. A version replaced is
    /// renamed out of that name before it is deleted, so none is left half deleted under it.
    #[test]
    fn a_half_deleted_replaced_version_holds_up_no_install() {
        let base = std::env::temp_dir().join(format!("ta_package_halfgone_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mods = base.join("mods");
        let content = |name: &str, text: &[u8]| {
            let source = base.join(name);
            std::fs::create_dir_all(source.join("units")).unwrap();
            std::fs::write(source.join("units/NEW.FBI"), text).unwrap();
            source
        };
        install(&plan(&content("one", b"first")).unwrap(), &manifest("testmod", "1"), &mods).unwrap();
        let old = mods.join(".testmod.old");
        std::fs::create_dir_all(old.join("weapons")).unwrap();
        std::fs::write(old.join("weapons/left.tdf"), b"left behind").unwrap();
        let target = install(&plan(&content("two", b"second")).unwrap(), &manifest("testmod", "2"), &mods).unwrap();
        assert_eq!(std::fs::read(target.join("units/NEW.FBI")).unwrap(), b"second");
        assert!(!old.exists());
        let left: Vec<String> = std::fs::read_dir(&mods).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.starts_with(TRASH_PREFIX) || n.ends_with(".old")).collect();
        assert!(left.is_empty(), "nothing replaced is left: {left:?}");
        // (One of another mod's is still refused.)
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("mod.toml"), toml::to_string(&manifest("other", "1")).unwrap()).unwrap();
        assert!(matches!(install(&plan(&content("three", b"third")).unwrap(), &manifest("testmod", "3"), &mods), Err(PackageError::Unsafe(_))));
        std::fs::remove_dir_all(base).unwrap();
    }

    /// What is left of a version being discarded (its deletion failed, its manifest still there)
    /// is no mod: discovery passes it over, and no install takes it for the mod's folder, where
    /// the next start's sweep would delete what was published into it.
    #[test]
    fn a_discarded_version_left_behind_is_no_mod() {
        let base = std::env::temp_dir().join(format!("ta_package_trash_left_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mods = base.join("mods");
        let source = base.join("one");
        std::fs::create_dir_all(source.join("units")).unwrap();
        std::fs::write(source.join("units/NEW.FBI"), b"first").unwrap();
        let live = install(&plan(&source).unwrap(), &manifest("testmod", "1"), &mods).unwrap();
        let left = mods.join(format!("{TRASH_PREFIX}1-2-3"));
        std::fs::create_dir_all(left.join("units")).unwrap();
        std::fs::write(left.join("mod.toml"), toml::to_string(&manifest("testmod", "0")).unwrap()).unwrap();
        let (found, errors) = crate::mods::scan(&mods);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(found.iter().map(|m| m.path.as_path()).collect::<Vec<_>>(), [live.as_path()], "only the live copy");
        assert_eq!(install_target(&mods, "testmod"), Some(live.clone()));
        let source = base.join("two");
        std::fs::create_dir_all(source.join("units")).unwrap();
        std::fs::write(source.join("units/NEW.FBI"), b"second").unwrap();
        assert_eq!(install(&plan(&source).unwrap(), &manifest("testmod", "2"), &mods).unwrap(), live);
        assert!(install_target(&mods, "other").is_some_and(|target| !target.file_name().unwrap().to_string_lossy().starts_with(TRASH_PREFIX)));
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
            replaces: Vec::new(), ..Default::default()
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

    /// A fresh folder of its own for a test (removed first if a run before left it).
    fn fresh(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("ta_package_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn write(path: PathBuf, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// Pictures a manifest names in a folder whose name starts like one of TA's (`gameplay/`,
    /// `gui/`) but that proves to be none are installed: the folder is left out, what the manifest
    /// names in it is not.
    #[test]
    fn pictures_named_in_a_folder_named_like_one_of_tas_are_installed() {
        let base = fresh("named_in_candidates");
        let source = base.join("package");
        write(source.join("units/NEW.FBI"), b"[UNITINFO]{UnitName=NEW;}");
        write(source.join("mod.toml"), b"id = \"shots\"\nname = \"Shots\"\n\n[media]\npreview = \"gameplay/preview.gif\"\nscreenshots = [\"gameplay/shot1.png\"]\n\n[theme]\npanel_texture = \"gui/panel.png\"\n");
        write(source.join("gameplay/shot1.png"), b"png");
        write(source.join("gameplay/preview.gif"), b"gif");
        write(source.join("gameplay/notes.txt"), b"not named");
        write(source.join("gui/panel.png"), b"png");
        let plan = plan(&source).unwrap();
        let mut kept: Vec<&str> = plan.media.iter().map(|(_, to)| to.as_str()).collect();
        kept.sort();
        assert_eq!(kept, ["gameplay/preview.gif", "gameplay/shot1.png", "gui/panel.png"], "{:?}", plan.skipped);
        assert!(plan.folders.iter().all(|f| !f.ends_with("gameplay") && !f.ends_with("gui")), "{:?}", plan.folders);
        assert!(plan.skipped.iter().any(|s| s == "gameplay/") && plan.skipped.iter().any(|s| s == "gui/"), "{:?}", plan.skipped);
        let dir = install(&plan, plan.authored.as_ref().unwrap(), &base.join("mods")).unwrap();
        for file in ["gameplay/preview.gif", "gameplay/shot1.png", "gui/panel.png"] {
            assert!(dir.join(file).is_file(), "{file} installed");
        }
        assert!(!dir.join("gameplay/notes.txt").exists());
        let installed = crate::mods::read_mod(&dir).unwrap().unwrap().manifest;
        assert!(installed.theme.texture("arm", "panel").and_then(|named| crate::mods::media_path(&dir, named)).is_some_and(|path| path.is_file()));
        std::fs::remove_dir_all(base).unwrap();
    }

    /// A file a manifest names in another case than the disk's, inside a folder of TA's that is
    /// kept whole (`font = "Fonts/hud.ttf"` for `fonts/hud.ttf`; an old package's `BITMAPS/`), comes
    /// with its folder, once: on a disk that does not tell case apart it was planned twice, with
    /// the folder and on its own, and the install failed.
    #[test]
    fn a_file_named_in_another_case_in_a_kept_folder_comes_with_it() {
        let base = fresh("named_case");
        let source = base.join("package");
        write(source.join("UNITS/NEW.FBI"), b"[UNITINFO]{UnitName=NEW;}");
        write(source.join("fonts/hud.ttf"), b"ttf");
        write(source.join("BITMAPS/banner.png"), b"png");
        write(source.join("mod.toml"), b"id = \"cased\"\nname = \"Cased\"\n\n[media]\nbanner = \"bitmaps/banner.png\"\n\n[theme]\nfont = \"Fonts/hud.ttf\"\n");
        let plan = plan(&source).unwrap();
        assert!(plan.media.is_empty(), "with their folders, not on their own: {:?}", plan.media);
        let dir = install(&plan, plan.authored.as_ref().unwrap(), &base.join("mods")).unwrap();
        assert!(dir.join("fonts/hud.ttf").is_file() && dir.join("BITMAPS/banner.png").is_file());
        let installed = crate::mods::read_mod(&dir).unwrap().unwrap().manifest;
        assert!(crate::mods::media_path(&dir, &installed.theme.font).is_some_and(|path| path.is_file()));
        assert!(crate::mods::media_path(&dir, &installed.media.banner).is_some_and(|path| path.is_file()));
        std::fs::remove_dir_all(base).unwrap();
    }

    /// A mod that brings nothing of TA's but its interface theme (with its pictures), or the
    /// archives of the original game it replaces, is installed as it reads put in a mods folder by
    /// hand. A manifest of a name and nothing else is still no mod, nor a folder of pictures; and
    /// a package's top manifest of pictures alone does not take the place of its content's folder.
    #[test]
    fn a_mod_of_a_theme_or_of_what_it_replaces_alone_is_installed() {
        let base = fresh("theme_only");
        let mods = base.join("mods");
        let themed = base.join("themed");
        write(themed.join("mod.toml"), b"id = \"blue-hud\"\nname = \"Blue HUD\"\n\n[theme]\naccent = \"#5ab4ff\"\npanel_texture = \"media/hud/panel.png\"\n");
        write(themed.join("media/hud/panel.png"), b"png");
        let planned = plan(&themed).unwrap();
        assert_eq!((planned.units, planned.maps, planned.media.len()), (0, 0, 1));
        let dir = install(&planned, planned.authored.as_ref().unwrap(), &mods).unwrap();
        let installed = crate::mods::read_mod(&dir).unwrap().unwrap().manifest;
        assert_eq!(installed.theme.colour("arm", "accent"), Some("#5ab4ff"));
        assert!(installed.theme.texture("core", "panel").and_then(|named| crate::mods::media_path(&dir, named)).is_some_and(|path| path.is_file()));
        let blanking = base.join("blanking");
        write(blanking.join("mod.toml"), b"id = \"no-bt\"\nname = \"No Battle Tactics\"\nreplaces = [\"btdata.ccx\"]\n");
        let planned = plan(&blanking).unwrap();
        let dir = install(&planned, planned.authored.as_ref().unwrap(), &mods).unwrap();
        assert_eq!(crate::mods::read_mod(&dir).unwrap().unwrap().manifest.replaces, ["btdata.ccx"]);
        let named_only = base.join("named");
        write(named_only.join("mod.toml"), b"id = \"nothing\"\nname = \"Nothing\"\ntags = [\"units\"]\n");
        assert!(matches!(plan(&named_only), Err(PackageError::NoContent)));
        let pictures = base.join("pictures");
        write(pictures.join("media/banner.png"), b"png");
        assert!(matches!(plan(&pictures), Err(PackageError::NoContent)));
        let wrapped = base.join("wrapped");
        write(wrapped.join("mod.toml"), b"id = \"wrapped\"\nname = \"Wrapped\"\n[media]\nbanner = \"banner.png\"\n");
        write(wrapped.join("banner.png"), b"png");
        write(wrapped.join("Main/units/NEW.FBI"), b"[UNITINFO]{UnitName=NEW;}");
        let planned = plan(&wrapped).unwrap();
        assert_eq!((planned.root.clone(), planned.units), (wrapped.join("Main"), 1), "the content's folder, as before");
        std::fs::remove_dir_all(base).unwrap();
    }

    /// TA Reborn's own folders (`menus/`: a menu theme's pictures, art and list) are kept whole as
    /// TA's are: the launcher's import of Supreme Commander's look for the menus installs its
    /// theme, not only its sounds and music.
    #[test]
    fn reborns_own_folders_are_installed_whole() {
        let base = fresh("reborn_folders");
        let package = base.join("payload");
        write(package.join("mod.toml"), b"id = \"supcom-menus\"\nname = \"Supreme Commander menus\"\nversion = \"2\"\n");
        write(package.join("menus/supcom/theme.toml"), b"pictures = [\"menus/supcom/pictures/space_1.dds\"]\n");
        write(package.join("menus/supcom/pictures/space_1.dds"), b"DDS picture");
        write(package.join("sounds/supcom-menus/ui_menu_mousedown.wav"), b"RIFF");
        let planned = plan(&package).unwrap();
        assert_eq!(planned.root, package);
        assert!(planned.skipped.is_empty(), "nothing left out: {:?}", planned.skipped);
        let dir = install(&planned, &manifest("supcom-menus", "2"), &base.join("mods")).unwrap();
        assert!(dir.join("menus/supcom/theme.toml").is_file() && dir.join("menus/supcom/pictures/space_1.dds").is_file(), "the theme installed");
        assert!(dir.join("sounds/supcom-menus/ui_menu_mousedown.wav").is_file());
        assert!(game_folder("menus") && game_folder("effects") && game_folder("units") && !game_folder("extras"));

        // A theme alone beside its manifest is a mod of its own.
        let theme_only = base.join("theme_only");
        write(theme_only.join("Theme/mod.toml"), b"id = \"menu-look\"\nname = \"Menu look\"\nversion = \"1\"\n");
        write(theme_only.join("Theme/menus/look/theme.toml"), b"pictures = []\n");
        let planned = plan(&theme_only).unwrap();
        assert_eq!(planned.root, theme_only.join("Theme"));
        assert_eq!(planned.folders, vec![theme_only.join("Theme/menus")]);

        // Without a manifest, the content is still found by TA's folders alone: a `Menus/` of the
        // package's own, nearer its top, does not take the mod's place.
        let old = base.join("old");
        write(old.join("Pack/Menus/picture.dds"), b"DDS picture");
        write(old.join("Pack/Mod/units/tank.fbi"), b"[UNITINFO]{UnitName=TANK;}");
        let planned = plan(&old).unwrap();
        assert_eq!(planned.root, old.join("Pack/Mod"));
        assert_eq!(planned.folders, vec![old.join("Pack/Mod/units")]);
        std::fs::remove_dir_all(base).unwrap();
    }

    /// The content's folder of a package without a manifest of its own may be called anything,
    /// `Base` or `Reborn` too: the install names the mod, not the folder.
    #[test]
    fn a_content_folder_named_like_the_games_own_content_is_planned() {
        let base = fresh("base_folder");
        for folder in ["Base", "reborn"] {
            let package = base.join(format!("pkg-{folder}"));
            write(package.join(folder).join("units/a.fbi"), b"[UNITINFO]{UnitName=A;}");
            write(package.join("Extras/Optional/units/b.fbi"), b"[UNITINFO]{UnitName=B;}");
            let planned = plan(&package).unwrap_or_else(|e| panic!("{folder}: {e}"));
            assert_eq!((planned.root.clone(), planned.units), (package.join(folder), 1));
            assert!(planned.authored.is_none());
            let dir = install(&planned, &manifest(&format!("pack-{folder}"), "1"), &base.join("mods")).unwrap();
            assert!(dir.join("units/a.fbi").is_file());
        }
        // (One whose own manifest takes such an id is still refused: it would never be found.)
        let named = base.join("named");
        write(named.join("units/a.fbi"), b"[UNITINFO]{UnitName=A;}");
        write(named.join("mod.toml"), b"id = \"base\"\nname = \"Base\"\n");
        assert!(matches!(plan(&named), Err(PackageError::Manifest { .. })));
        std::fs::remove_dir_all(base).unwrap();
    }

    /// The package's own `mod.toml` is used, not left out: it is never listed among what the
    /// install leaves behind. A file moved whose source is gone is named, not where it was going.
    #[test]
    fn the_manifest_is_not_left_out_and_a_missing_source_is_named() {
        let base = fresh("manifest_skipped");
        let source = base.join("package");
        write(source.join("units/NEW.FBI"), b"[UNITINFO]{UnitName=NEW;}");
        write(source.join("Mod.toml"), b"id = \"listed\"\nname = \"Listed\"\n");
        write(source.join("setup.exe"), b"MZ");
        let planned = plan(&source).unwrap();
        assert!(planned.skipped.iter().any(|s| s == "setup.exe"), "{:?}", planned.skipped);
        assert!(!planned.skipped.iter().any(|s| s.to_lowercase().contains("mod.toml")), "{:?}", planned.skipped);
        let missing = base.join("gone.png");
        let to = base.join("there").join("gone.png");
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        match move_path(&missing, &to) {
            Err(PackageError::Io { path, .. }) => assert_eq!(path, missing.display().to_string()),
            other => panic!("{other:?}"),
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    /// The pictures and the font a manifest names are taken first: a media folder of more files
    /// than a mod's pictures may number never crowds them out, whatever their names.
    #[test]
    fn the_named_pictures_are_taken_before_the_rest_of_the_media_folder() {
        let base = fresh("named_first");
        let source = base.join("package");
        write(source.join("units/NEW.FBI"), b"[UNITINFO]{UnitName=NEW;}");
        for n in 0..MAX_MEDIA_FILES + 8 {
            write(source.join(format!("media/a{n:04}.png")), b"png");
        }
        write(source.join("media/zz_banner.png"), b"png");
        write(source.join("media/zz_font.ttf"), b"ttf");
        write(source.join("mod.toml"), b"id = \"many\"\nname = \"Many\"\n[media]\nbanner = \"media/zz_banner.png\"\n[theme]\nfont = \"media/zz_font.ttf\"\n");
        let planned = plan(&source).unwrap();
        assert_eq!(planned.media.len(), MAX_MEDIA_FILES);
        assert_eq!(planned.media[..2].iter().map(|(_, to)| to.as_str()).collect::<Vec<_>>(), ["media/zz_banner.png", "media/zz_font.ttf"]);
        assert!(planned.skipped.iter().any(|s| s.ends_with("(beyond what a mod's pictures may weigh)")));
        assert!(!planned.skipped.iter().any(|s| s.contains("zz_")), "{:?}", planned.skipped);
        std::fs::remove_dir_all(base).unwrap();
    }

    /// An install of a package with a `mod.toml` of its own keeps it as written where the install
    /// changes nothing: its comments, and keys this TA Reborn does not know (a later one's), are
    /// in the installed copy's; what the install sets (another id, the renamed folders it found) is
    /// written in.
    #[test]
    fn an_install_keeps_the_packages_manifest_as_written() {
        let base = fresh("manifest_kept");
        let source = base.join("package");
        write(source.join("guiM/ARMCOM1.GUI"), b"gui");
        write(source.join("units/NEW.FBI"), b"[UNITINFO]{UnitName=NEW;}");
        let text = "# Naval Additions: three ships.\nid = \"naval\"\nname = \"Naval Additions\" # shown in the launcher\nversion = \"1.2\"\nfuture_key = \"kept\"\n\n[future_table]\nanswer = 42\n";
        write(source.join("mod.toml"), text.as_bytes());
        let planned = plan(&source).unwrap();
        assert_eq!(planned.authored_text.as_deref(), Some(text));
        let mut own = planned.authored.clone().unwrap();
        own.id = "naval-2".into();
        let dir = install(&planned, &own, &base.join("mods")).unwrap();
        let written = std::fs::read_to_string(dir.join("mod.toml")).unwrap();
        for kept in ["# Naval Additions: three ships.", "name = \"Naval Additions\" # shown in the launcher", "future_key = \"kept\"", "[future_table]\nanswer = 42"] {
            assert!(written.contains(kept), "{kept}:\n{written}");
        }
        let installed = crate::mods::read_mod(&dir).unwrap().unwrap().manifest;
        assert_eq!((installed.id.as_str(), installed.version.as_str()), ("naval-2", "1.2"));
        assert_eq!(installed.rename.get("guim").map(String::as_str), Some("guis"), "{written}");
        std::fs::remove_dir_all(base).unwrap();
    }

    /// A map editor's save stopped between its two renames left the map's only copy aside (hidden
    /// from discovery): it goes back to its folder when that is missing, and is discarded when the
    /// folder holds the same map again; one that does not say which map's it was, or whose folder
    /// holds another mod, is left as it is. An older editor's, whose name does not tell its folder,
    /// goes back to the folder of the one map it holds. Nothing is touched while an editor saves.
    #[test]
    fn a_map_editors_interrupted_save_is_put_right() {
        let base = fresh("editor_backups");
        let mods = base.join("mods");
        let map = |dir: &str, id: &str, maps: &[&str]| {
            write(mods.join(dir).join("mod.toml"), format!("id = \"{id}\"\nname = \"{id}\"\n").as_bytes());
            for name in maps { write(mods.join(dir).join("maps").join(format!("{name}.tnt")), b"tnt"); }
        };
        map(&editor_backup_name(".editor-7-0", "Test Map"), "map-test-map-1", &["Test Map"]);
        map(&editor_backup_name(".editor-7-1", "Done"), "map-done-2", &["Done"]);
        map("Done", "map-done-2", &["Done"]);
        map(&editor_backup_name(".editor-7-2", "Taken"), "map-taken-3", &["Taken"]);
        map("Taken", "someone-elses", &[]);
        map(".editor-6-0.old", "map-legacy", &["Legacy"]);
        map(".editor-6-1.old", "map-two", &["One", "Two"]);
        assert!(crate::mods::scan(&mods).0.iter().all(|m| !m.manifest.id.starts_with("map-test")), "hidden while aside");
        // (While an editor saves, nothing.)
        let (_, held) = open_lock(mods.join(EDITOR_SAVE_LOCK)).unwrap();
        held.lock().unwrap();
        let log = recover(&mods);
        assert!(mods.join(editor_backup_name(".editor-7-0", "Test Map")).is_dir() && !mods.join("Test Map").exists(), "{log:?}");
        drop(held);
        let log = recover(&mods);
        assert!(mods.join("Test Map/maps/Test Map.tnt").is_file(), "the only copy is back: {log:?}");
        assert!(!mods.join(editor_backup_name(".editor-7-1", "Done")).exists() && mods.join("Done/maps/Done.tnt").is_file(), "{log:?}");
        assert!(mods.join(editor_backup_name(".editor-7-2", "Taken")).is_dir() && mods.join("Taken/mod.toml").is_file(), "another mod's folder: both kept: {log:?}");
        assert!(mods.join("Legacy/maps/Legacy.tnt").is_file() && !mods.join(".editor-6-0.old").exists(), "{log:?}");
        assert!(mods.join(".editor-6-1.old").is_dir(), "which of its maps is not known: {log:?}");
        let ids: Vec<String> = crate::mods::scan(&mods).0.into_iter().map(|m| m.manifest.id).collect();
        assert!(ids.contains(&"map-test-map-1".to_string()) && ids.contains(&"map-legacy".to_string()), "{ids:?}");
        // (What is done is done once: nothing more the next time.)
        let again = restore_editor_backups(&mods);
        assert!(again.iter().all(|line| line.starts_with("left ")), "{again:?}");
        std::fs::remove_dir_all(base).unwrap();
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
