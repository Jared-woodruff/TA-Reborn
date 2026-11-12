//! Layered virtual file system.
//!
//! The VFS presents one case-insensitive namespace (`units/armcom.fbi`) built from:
//!
//! 1. the user's original Total Annihilation install (GOG Commander Pack), mounted with the
//!    same precedence rules the 1997 game used, and
//! 2. zero or more mods, each mounted on top in load order.
//!
//! Within one game or mod directory, archives are layered by extension —
//! `*.hpi` < `*.ufo` < `*.ccx` < `*.gpf` < `*.gp3` — and, within one extension, the
//! alphabetically *first* archive wins (TA scanned with `FindFirstFile` and kept the first
//! hit). This reproduces the shipped data exactly: `rev31.gp3` (3.1 patch) overrides
//! `btdata.ccx`, which overrides `ccdata.ccx`, which overrides `totala1.hpi`.
//! Loose files in a *mod* directory override that mod's archives; loose files in the base
//! install sit below every archive (they are leftovers such as the example mission).
//! A mod may map folders of its own onto TA's ([`Renames`]: Total Mayhem's patched engine
//! read `guiM` where TA reads `guis`); such a folder replaces TA's.
//!
//! A mod's archive named like one of the install's (`totala1.hpi`, `ccdata.ccx`, `rev31.gp3`...,
//! in any case) *replaces* it, as copying the mod into TA's folder overwrote it: with the mod on,
//! the install's archive of that name is not mounted at all, so nothing of it the mod's lacks leaks
//! through (TA: Enhanced ships its whole game as `rev31.gp3` and stubs of `totala1.hpi` and
//! `ccdata.ccx`). An empty placeholder of that name blanks the install's archive. Every VFS of the
//! game's content is built by [`mods::mount`], which applies this; [`Vfs::mount_game_dir`] alone
//! mounts the install whole, as it is without mods.

pub mod install;
pub mod mods;
pub mod package;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ta_formats::hpi::HpiArchive;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VfsError {
    #[error("file not found in VFS: {0}")]
    NotFound(String),
    #[error("{path}: {source}")]
    Format { path: String, source: ta_formats::FormatError },
    #[error("{path}: {source}")]
    Io { path: String, source: std::io::Error },
}

pub type Result<T> = std::result::Result<T, VfsError>;

/// Archive extensions in increasing precedence.
pub const ARCHIVE_EXTENSIONS: [&str; 5] = ["hpi", "ufo", "ccx", "gpf", "gp3"];

/// The packages mods and map packs are published in, unpacked (by 7-Zip) to be installed.
pub const PACKAGE_EXTENSIONS: [&str; 3] = ["zip", "7z", "rar"];

/// Where a mounted source came from; used for diagnostics and the mod manager UI.
#[derive(Clone, Debug)]
pub struct SourceInfo {
    /// Human-readable layer name, e.g. `"base"` or a mod id.
    pub layer: String,
    /// Archive file or loose-file root.
    pub path: PathBuf,
}

enum SourceData {
    Archive(HpiArchive),
    Loose(Vec<PathBuf>),
}

struct Source {
    info: SourceInfo,
    data: SourceData,
}

#[derive(Clone, Copy, Debug)]
struct FileRef {
    source: u32,
    index: u32,
}

/// Folders a mod maps onto TA's (lower-case names, `guim` → `guis`): its files in `guim/`
/// are seen as `guis/`, and whatever the layers beneath had in `guis/` is hidden, as the
/// mod's own engine read only its folder.
#[derive(Clone, Debug, Default)]
pub struct Renames(Vec<(String, String)>);

impl Renames {
    /// From a manifest's `[rename]` table (folder → TA folder; written as folders too,
    /// `"guiM/" = "guis/"`).
    pub fn new(map: &BTreeMap<String, String>) -> Renames {
        let folder = |name: &str| format!("{}/", normalize(name).trim_end_matches('/'));
        Renames(map.iter().map(|(from, to)| (folder(from), folder(to))).filter(|(from, to)| from != to && from.len() > 1 && to.len() > 1).collect())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The path a normalised path of the mod is seen at.
    fn apply(&self, key: String) -> String {
        for (from, to) in &self.0 {
            if let Some(rest) = key.strip_prefix(from.as_str()) {
                return format!("{to}{rest}");
            }
        }
        key
    }
}

/// A mod's content read and ready to mount, nothing mounted yet: several are read first, and
/// mounted only if every one of them reads ([`crate::mods::mount`]).
pub struct Layer {
    files: Vfs,
    renames: Renames,
}

impl Layer {
    /// A mod's folder: its archives, then its loose files on top.
    pub fn mod_dir(dir: &Path, layer: &str, renames: &Renames) -> Result<Layer> {
        let mut files = Vfs::new();
        files.mount_archives_renamed(dir, layer, renames)?;
        files.mount_loose_renamed(dir, layer, false, renames)?;
        Ok(Layer { files, renames: renames.clone() })
    }

    /// One TA archive.
    pub fn archive(path: &Path, layer: &str, renames: &Renames) -> Result<Layer> {
        let mut files = Vfs::new();
        files.mount_one_archive(path, layer, renames)?;
        Ok(Layer { files, renames: renames.clone() })
    }
}

/// An immutable-after-build, thread-safe view over all mounted data.
#[derive(Default)]
pub struct Vfs {
    sources: Vec<Source>,
    /// Normalised path → winning file. A BTreeMap keeps listings sorted and deterministic,
    /// which matters: load order feeds into simulation IDs.
    index: BTreeMap<String, FileRef>,
}

/// Normalises a path for lookup: lowercase, forward slashes, no leading or doubled slashes.
pub fn normalize(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        let c = if c == '\\' { '/' } else { c.to_ascii_lowercase() };
        if c == '/' && (out.is_empty() || out.ends_with('/')) {
            continue;
        }
        out.push(c);
    }
    while let Some(rest) = out.strip_prefix("./") {
        out = rest.to_string();
    }
    out
}

/// The layer TA Reborn's own content is mounted as.
pub const BUILTIN_LAYER: &str = "reborn";

/// TA Reborn's own content (its maps): `assets/reborn` beside the program, or, running from a
/// build folder (`target/release`; built for a named target, `target/<triple>/release`, its
/// tests a folder deeper in `deps`), in the source tree above it. macOS application bundles
/// keep content under `Contents/Resources`. `TA_REBORN_ASSETS` names another folder.
pub fn builtin_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("TA_REBORN_ASSETS") {
        return Some(PathBuf::from(dir));
    }
    builtin_dir_from_exe(&std::env::current_exe().ok()?)
}

/// Resources for the shipped macOS .app, whose launcher and game are sibling binaries
/// in Contents/MacOS. Pure path handling also lets packaging tests verify the layout.
pub fn bundle_resource_dir(exe_dir: &Path) -> Option<PathBuf> {
    let contents = exe_dir.parent()?;
    (exe_dir.file_name()? == "MacOS" && contents.file_name()? == "Contents"
        && contents.parent()?.extension()?.eq_ignore_ascii_case("app"))
        .then(|| contents.join("Resources"))
}

fn builtin_dir_from_exe(exe: &Path) -> Option<PathBuf> {
    let bundle = exe.parent().and_then(bundle_resource_dir).map(|d| d.join("assets/reborn"));
    bundle.into_iter().chain(exe.ancestors().skip(1).take(5).map(|d| d.join("assets/reborn")))
        .find(|d| d.join("maps").is_dir())
}

impl Vfs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mounts an original TA install directory (archives first, loose files underneath), whole:
    /// the game without mods. With mods, [`mods::mount`] mounts it, less what they replace. The
    /// archives of mods made for other engines its folder holds are never the original game's
    /// ([`mods::game_folder_mods`]).
    pub fn mount_game_dir(&mut self, dir: &Path, layer: &str) -> Result<()> {
        self.mount_game_dir_without(dir, layer, &[])
    }

    /// [`Self::mount_game_dir`] but the archives named in `left_out` (file names, in any case):
    /// those the mods on replace ([`mods::replaced_archives`]).
    pub fn mount_game_dir_without(&mut self, dir: &Path, layer: &str, left_out: &[String]) -> Result<()> {
        let mut left_out = left_out.to_vec();
        for m in mods::game_folder_mods(dir) {
            left_out.extend(m.archives.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()));
        }
        self.mount_install(dir, layer, &left_out)
    }

    /// The install at `dir`, but the archives named in `left_out` (which [`mods::mount`] works
    /// out: those the mods on replace, and those of mods in the game's folder).
    pub(crate) fn mount_install(&mut self, dir: &Path, layer: &str, left_out: &[String]) -> Result<()> {
        self.mount_loose(dir, layer, true)?;
        self.mount_archives_without(dir, layer, &Renames::default(), left_out)?;
        Ok(())
    }

    /// Mounts TA Reborn's own content (its maps), when it is found beside the program: over the
    /// install, under any mods. Returns the folder mounted.
    pub fn mount_builtin(&mut self) -> Option<PathBuf> {
        let dir = builtin_dir()?;
        match self.mount_mod_dir(&dir, BUILTIN_LAYER) {
            Ok(()) => Some(dir),
            Err(e) => {
                log::warn!("built-in content {}: {e}", dir.display());
                None
            }
        }
    }

    /// Mounts a mod directory: its archives, then its loose files on top.
    pub fn mount_mod_dir(&mut self, dir: &Path, layer: &str) -> Result<()> {
        self.mount_mod_dir_renamed(dir, layer, &Renames::default())
    }

    /// [`Self::mount_mod_dir`] with the mod's folder mapping.
    pub fn mount_mod_dir_renamed(&mut self, dir: &Path, layer: &str, renames: &Renames) -> Result<()> {
        // Resolve the complete layer before hiding or overriding anything already
        // mounted. A failed mod is skipped by callers and must leave their VFS intact.
        self.mount_layer(Layer::mod_dir(dir, layer, renames)?);
        Ok(())
    }

    /// Mounts one archive on top of everything mounted so far.
    pub fn mount_archive(&mut self, path: &Path, layer: &str) -> Result<()> {
        self.mount_archive_renamed(path, layer, &Renames::default())
    }

    /// [`Self::mount_archive`] with a mod's folder mapping (hiding what it replaces).
    pub fn mount_archive_renamed(&mut self, path: &Path, layer: &str, renames: &Renames) -> Result<()> {
        self.mount_layer(Layer::archive(path, layer, renames)?);
        Ok(())
    }

    /// Mounts a layer read before (see [`Layer`]) on top of everything mounted so far.
    pub fn mount_layer(&mut self, layer: Layer) {
        self.publish(layer.files, &layer.renames);
    }

    /// Publish a validated layer, preserving source indices without cloning archive data.
    fn publish(&mut self, staged: Self, renames: &Renames) {
        self.hide(renames);
        let offset = self.sources.len() as u32;
        self.sources.extend(staged.sources);
        self.index.extend(staged.index.into_iter().map(|(path, mut file)| {
            file.source += offset;
            (path, file)
        }));
    }

    /// Mounts an archive's files at their (mapped) paths; hiding is the caller's.
    fn mount_one_archive(&mut self, path: &Path, layer: &str, renames: &Renames) -> Result<()> {
        let archive = HpiArchive::open(path)
            .map_err(|source| VfsError::Format { path: path.display().to_string(), source })?;
        let source = self.sources.len() as u32;
        for (index, f) in archive.files().iter().enumerate() {
            self.index.insert(renames.apply(normalize(&f.path)), FileRef { source, index: index as u32 });
        }
        log::info!("mounted {} ({} files) [{layer}]", path.display(), archive.files().len());
        self.sources.push(Source {
            info: SourceInfo { layer: layer.to_string(), path: path.to_path_buf() },
            data: SourceData::Archive(archive),
        });
        Ok(())
    }

    /// Hides what the layers mounted so far have in the folders a mod replaces.
    fn hide(&mut self, renames: &Renames) {
        for (_, to) in &renames.0 {
            let hidden: Vec<String> = self.index.range(to.clone()..).take_while(|(k, _)| k.starts_with(to.as_str())).map(|(k, _)| k.clone()).collect();
            if !hidden.is_empty() {
                log::info!("hiding {} files in {to} (a mod replaces the folder)", hidden.len());
            }
            for k in hidden {
                self.index.remove(&k);
            }
        }
    }

    fn mount_archives_renamed(&mut self, dir: &Path, layer: &str, renames: &Renames) -> Result<()> {
        self.mount_archives_without(dir, layer, renames, &[])
    }

    fn mount_archives_without(&mut self, dir: &Path, layer: &str, renames: &Renames, left_out: &[String]) -> Result<()> {
        let mut archives: Vec<(usize, String, PathBuf)> = archive_files(dir)?
            .into_iter()
            .filter_map(|path| {
                let ext = path.extension()?.to_str()?.to_ascii_lowercase();
                let rank = ARCHIVE_EXTENSIONS.iter().position(|&x| x == ext)?;
                let name = path.file_name()?.to_string_lossy().to_ascii_uppercase();
                Some((rank, name, path))
            })
            .collect();
        // Lowest precedence first: ascending extension rank, and within one extension
        // reverse-alphabetical so the alphabetically-first archive is mounted last and wins
        // (alphabetical as NTFS lists names, by their upper case: "_x" after the letters).
        archives.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
        for (_, name, path) in archives {
            if left_out.iter().any(|out| out.to_lowercase() == name.to_lowercase()) {
                log::info!("left out {} [{layer}]", path.display());
                continue;
            }
            // (One unreadable archive, e.g. an empty placeholder some mods ship to blank out
            // the original's, does not keep the rest out.)
            if let Err(e) = self.mount_one_archive(&path, layer, renames) {
                log::warn!("{e}; skipped");
            }
        }
        Ok(())
    }

    /// Mounts loose files found in subdirectories of `root`. With `skip_root_files`, files
    /// directly inside `root` (executables, docs, archives) are ignored.
    fn mount_loose(&mut self, root: &Path, layer: &str, skip_root_files: bool) -> Result<()> {
        self.mount_loose_renamed(root, layer, skip_root_files, &Renames::default())
    }

    fn mount_loose_renamed(&mut self, root: &Path, layer: &str, skip_root_files: bool, renames: &Renames) -> Result<()> {
        let mut files = Vec::new();
        collect_files(root, root, skip_root_files, &mut files)
            .map_err(|source| VfsError::Io { path: root.display().to_string(), source })?;
        files.sort();
        if files.is_empty() {
            return Ok(());
        }
        let source = self.sources.len() as u32;
        for (index, rel) in files.iter().enumerate() {
            let key = normalize(&rel.to_string_lossy());
            if key == "mod.toml" {
                continue;
            }
            self.index.insert(renames.apply(key), FileRef { source, index: index as u32 });
        }
        log::info!("mounted {} loose files from {} [{layer}]", files.len(), root.display());
        self.sources.push(Source {
            info: SourceInfo { layer: layer.to_string(), path: root.to_path_buf() },
            data: SourceData::Loose(files),
        });
        Ok(())
    }

    pub fn exists(&self, path: &str) -> bool {
        self.index.contains_key(&normalize(path))
    }

    /// A file's size (unpacked), without reading it.
    pub fn size(&self, path: &str) -> Option<u64> {
        let r = *self.index.get(&normalize(path))?;
        let source = &self.sources[r.source as usize];
        match &source.data {
            SourceData::Archive(a) => Some(u64::from(a.files()[r.index as usize].size)),
            SourceData::Loose(files) => std::fs::metadata(source.info.path.join(&files[r.index as usize])).ok().map(|m| m.len()),
        }
    }

    /// Reads a whole file.
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let key = normalize(path);
        let r = *self.index.get(&key).ok_or_else(|| VfsError::NotFound(path.to_string()))?;
        let source = &self.sources[r.source as usize];
        match &source.data {
            SourceData::Archive(a) => a
                .read(&a.files()[r.index as usize])
                .map_err(|e| VfsError::Format { path: key, source: e }),
            SourceData::Loose(files) => {
                let full = source.info.path.join(&files[r.index as usize]);
                let read = || -> std::io::Result<Vec<u8>> {
                    use std::io::Read;
                    let file = std::fs::File::open(&full)?;
                    let limit = ta_formats::hpi::MAX_FILE_BYTES as u64;
                    if file.metadata()?.len() > limit { return Err(std::io::Error::other("loose file exceeds 256 MiB read limit")); }
                    let mut bytes = Vec::new();
                    file.take(limit + 1).read_to_end(&mut bytes)?;
                    if bytes.len() as u64 > limit { return Err(std::io::Error::other("loose file exceeds 256 MiB read limit")); }
                    Ok(bytes)
                };
                read().map_err(|e| VfsError::Io { path: full.display().to_string(), source: e })
            }
        }
    }

    /// Reads a file if present, logging (not failing) on decode errors.
    pub fn read_opt(&self, path: &str) -> Option<Vec<u8>> {
        match self.read(path) {
            Ok(d) => Some(d),
            Err(VfsError::NotFound(_)) => None,
            Err(e) => {
                log::warn!("{e}");
                None
            }
        }
    }

    /// Which archive or directory provides a path (for diagnostics).
    pub fn origin(&self, path: &str) -> Option<&SourceInfo> {
        let r = self.index.get(&normalize(path))?;
        Some(&self.sources[r.source as usize].info)
    }

    /// Lists files under `dir` (normalised paths). `ext` filters by extension without the
    /// dot; `recursive` includes subdirectories. Output is sorted.
    pub fn list(&self, dir: &str, ext: Option<&str>, recursive: bool) -> Vec<String> {
        let mut prefix = normalize(dir);
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        let ext = ext.map(|e| format!(".{}", e.to_ascii_lowercase()));
        self.index
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .map(|(k, _)| k)
            .filter(|k| recursive || !k[prefix.len()..].contains('/'))
            .filter(|k| ext.as_ref().is_none_or(|e| k.ends_with(e.as_str())))
            .cloned()
            .collect()
    }

    /// Immediate subdirectory names of `dir`.
    pub fn subdirs(&self, dir: &str) -> Vec<String> {
        let mut prefix = normalize(dir);
        if !prefix.is_empty() && !prefix.ends_with('/') {
            prefix.push('/');
        }
        let mut out: Vec<String> = self
            .index
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .filter_map(|(k, _)| k[prefix.len()..].split_once('/').map(|(d, _)| d.to_string()))
            .collect();
        out.dedup();
        out
    }

    pub fn file_count(&self) -> usize {
        self.index.len()
    }

    pub fn sources(&self) -> impl Iterator<Item = &SourceInfo> {
        self.sources.iter().map(|s| &s.info)
    }
}

/// Whether a file is named as a TA archive (one of [`ARCHIVE_EXTENSIONS`], in any case).
pub fn is_archive_name(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| ARCHIVE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The files named as TA archives at the top of `dir`, in the order listed: what is mounted of
/// an install's or a mod's archives (those in folders below are never opened).
pub fn archive_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = std::fs::read_dir(dir).map_err(|source| VfsError::Io { path: dir.display().to_string(), source })?;
    Ok(entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|path| is_archive_name(path))
        .collect())
}

pub(crate) fn collect_files(root: &Path, dir: &Path, skip_root_files: bool, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if dir.strip_prefix(root).map_or(0, |p| p.components().count()) > 64 {
        return Err(std::io::Error::other("loose content nesting exceeds 64 levels"));
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ty = entry.file_type()?;
        if is_link(&std::fs::symlink_metadata(&path)?) { continue; }
        if ty.is_dir() {
            collect_files(root, &path, false, out)?;
        } else if ty.is_file() && !(skip_root_files && dir == root) {
            let is_archive = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| ARCHIVE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
            if !(is_archive && dir == root) {
                out.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
            }
        }
    }
    Ok(())
}

/// Includes Windows junctions and other reparse points, which are not ordinary
/// directories even when `file_type().is_dir()` says they are.
pub(crate) fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    { metadata.file_type().is_symlink() }
}

/// TA archives made for tests (this crate's and others'): their files stored as they are, no
/// compression.
#[doc(hidden)]
pub mod test_archive {
    use std::collections::BTreeMap;

    enum Node {
        Folder(BTreeMap<String, Node>),
        File(Vec<u8>),
    }

    /// An archive of `files` (paths with `/` between folders).
    pub fn bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut root = BTreeMap::new();
        for (path, content) in files {
            let mut parts: Vec<&str> = path.split('/').collect();
            let name = parts.pop().expect("a file name");
            let mut folder = &mut root;
            for part in parts {
                let next = folder.entry(part.to_string()).or_insert_with(|| Node::Folder(BTreeMap::new()));
                let Node::Folder(next) = next else { panic!("{part} is a file") };
                folder = next;
            }
            folder.insert(name.to_string(), Node::File(content.to_vec()));
        }
        // The header, then the directory (offsets from the file's start), then the files.
        let mut out = vec![0u8; 0x14];
        let mut contents: Vec<(usize, Vec<u8>)> = Vec::new();
        fn folder(out: &mut Vec<u8>, entries: &BTreeMap<String, Node>, contents: &mut Vec<(usize, Vec<u8>)>) -> usize {
            let at = out.len();
            out.extend((entries.len() as u32).to_le_bytes());
            out.extend(((at + 8) as u32).to_le_bytes());
            let list = out.len();
            out.resize(list + 9 * entries.len(), 0);
            for (i, (name, node)) in entries.iter().enumerate() {
                let name_at = out.len();
                out.extend(name.as_bytes());
                out.push(0);
                let (data_at, is_folder) = match node {
                    Node::Folder(entries) => (folder(out, entries, contents), 1),
                    Node::File(bytes) => {
                        let at = out.len();
                        out.extend([0; 4]);
                        out.extend((bytes.len() as u32).to_le_bytes());
                        out.push(0);
                        contents.push((at, bytes.clone()));
                        (at, 0)
                    }
                };
                let entry = list + 9 * i;
                out[entry..entry + 4].copy_from_slice(&(name_at as u32).to_le_bytes());
                out[entry + 4..entry + 8].copy_from_slice(&(data_at as u32).to_le_bytes());
                out[entry + 8] = is_folder;
            }
            at
        }
        folder(&mut out, &root, &mut contents);
        let directory_end = out.len();
        for (at, bytes) in contents {
            let offset = out.len() as u32;
            out[at..at + 4].copy_from_slice(&offset.to_le_bytes());
            out.extend(bytes);
        }
        out[0..4].copy_from_slice(b"HAPI");
        out[4..8].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        out[8..12].copy_from_slice(&(directory_end as u32).to_le_bytes());
        out[16..20].copy_from_slice(&0x14u32.to_le_bytes());
        out
    }

    /// Writes an archive of `files` at `path` (its folder made).
    pub fn write(path: &std::path::Path, files: &[(&str, &[u8])]) {
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("the archive's folder");
        std::fs::write(path, bytes(files)).expect("the archive written");
    }

    #[cfg(test)]
    #[test]
    fn an_archive_made_here_reads() {
        let archive = ta_formats::hpi::HpiArchive::from_bytes("test.hpi", bytes(&[("units/a.fbi", b"aaa"), ("units/b.fbi", b"b"), ("readme.txt", b"")])).unwrap();
        let mut read: Vec<(String, Vec<u8>)> = archive.files().iter().map(|f| (f.path.clone(), archive.read(f).unwrap())).collect();
        read.sort();
        assert_eq!(read, [("readme.txt".into(), Vec::new()), ("units/a.fbi".into(), b"aaa".to_vec()), ("units/b.fbi".into(), b"b".to_vec())]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_assets_resolve_for_app_bundles_portables_and_development_builds() {
        let root = std::env::temp_dir().join(format!("ta_vfs_resource_layout_{}", std::process::id()));
        let app = root.join("TA Reborn.app/Contents");
        std::fs::create_dir_all(app.join("Resources/assets/reborn/maps")).unwrap();
        assert_eq!(builtin_dir_from_exe(&app.join("MacOS/ta-reborn")), Some(app.join("Resources/assets/reborn")));
        let mods = crate::mods::default_folders(Some(&app.join("MacOS")), &root.join("user"), None);
        assert_eq!(mods, [app.join("Resources/mods"), root.join("user/mods")]);
        assert!(bundle_resource_dir(&root.join("other/Contents/MacOS")).is_none(), "Only actual .app structure gets a Resources sibling");
        let portable = root.join("portable");
        std::fs::create_dir_all(portable.join("assets/reborn/maps")).unwrap();
        assert_eq!(builtin_dir_from_exe(&portable.join("ta-reborn")), Some(portable.join("assets/reborn")));
        let dev = root.join("dev");
        std::fs::create_dir_all(dev.join("assets/reborn/maps")).unwrap();
        assert_eq!(builtin_dir_from_exe(&dev.join("target/release/ta-reborn")), Some(dev.join("assets/reborn")));
        // A test built for a named target (the release workflow's): `target/<triple>/<profile>/deps`.
        assert_eq!(builtin_dir_from_exe(&dev.join("target/x86_64-unknown-linux-gnu/ci/deps/ta_launcher-0123")), Some(dev.join("assets/reborn")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn normalizes_paths() {
        assert_eq!(normalize("Units\\ARMCOM.FBI"), "units/armcom.fbi");
        assert_eq!(normalize("/maps//Green.tnt"), "maps/green.tnt");
        assert_eq!(normalize("./a/b"), "a/b");
    }

    #[test]
    fn failed_replacement_leaves_the_previously_mounted_layer_intact() {
        let base = std::env::temp_dir().join(format!("ta_vfs_failed_layer_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let game = base.join("game");
        std::fs::create_dir_all(game.join("guis")).unwrap();
        std::fs::write(game.join("guis/armcom1.gui"), b"base interface").unwrap();
        let mut vfs = Vfs::new();
        vfs.mount_mod_dir(&game, "base").unwrap();
        let renames = Renames::new(&BTreeMap::from([("guim".into(), "guis".into())]));
        let archive = base.join("broken.ufo");
        std::fs::write(&archive, b"not an archive").unwrap();
        assert!(vfs.mount_archive_renamed(&archive, "broken", &renames).is_err());
        assert!(vfs.mount_mod_dir_renamed(&base.join("missing"), "missing", &renames).is_err());
        assert_eq!(vfs.read("guis/armcom1.gui").unwrap(), b"base interface");
        assert_eq!(vfs.origin("guis/armcom1.gui").unwrap().layer, "base");
        assert_eq!(vfs.sources().count(), 1);

        // A failed recursive scan also must not hide the base directory. The portable
        // depth limit gives a deterministic I/O failure without permission assumptions.
        let module = base.join("mod");
        let mut nested = module.clone();
        for _ in 0..65 { nested.push("a"); }
        std::fs::create_dir_all(&nested).unwrap();
        assert!(vfs.mount_mod_dir_renamed(&module, "too-deep", &renames).is_err());
        assert_eq!(vfs.read("guis/armcom1.gui").unwrap(), b"base interface");
        assert_eq!(vfs.sources().count(), 1);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn a_renamed_folder_replaces_the_one_beneath() {
        let base = std::env::temp_dir().join(format!("ta_vfs_rename_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (game, module) = (base.join("game"), base.join("mod"));
        for (dir, file, text) in [(&game, "guis/armcom1.gui", "base"), (&game, "guis/armlab1.gui", "base"), (&game, "units/armcom.fbi", "base"), (&module, "guiM/ARMCOM1.GUI", "mod")] {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        // (Written as folders, as a manifest's author may: the same.)
        for (from, to) in [("guiM", "guis"), ("guiM/", "guis/"), ("guiM\\", "/guis")] {
            let mut vfs = Vfs::new();
            vfs.mount_mod_dir(&game, "base").unwrap();
            let renames = Renames::new(&BTreeMap::from([(from.to_string(), to.to_string())]));
            vfs.mount_mod_dir_renamed(&module, "mod", &renames).unwrap();
            assert_eq!(vfs.read("guis/armcom1.gui").unwrap(), b"mod", "{from} = {to}");
            assert!(!vfs.exists("guis/armlab1.gui"), "the base game's guis are hidden");
            assert!(!vfs.exists("guim/armcom1.gui"));
            assert!(vfs.exists("units/armcom.fbi"), "other folders stay");
        }
        let _ = std::fs::remove_dir_all(base);
    }
}
