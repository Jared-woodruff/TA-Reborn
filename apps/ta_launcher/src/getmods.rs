//! The mod browser's background work: the mods' pictures and optional ModDB feeds ([`Media`]),
//! and installing the mods players pick ([`Installer`]): a catalogue mod is downloaded from
//! its provider, checked against its pinned size and digest, unpacked, installed as a mod folder
//! (`ta_vfs::package`) and its archive deleted. Website-only entries import a player-chosen
//! archive without a release-verification claim. A package the player brings (an archive or a
//! folder dropped on the window) is unpacked and installed the same way. One at a time.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use crate::catalogue::{self, Entry};
use crate::moddb::{self, FeedItem};

/// Where an install is.
#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    /// Waiting for the one before it.
    Queued,
    /// Connecting to the mod's download provider.
    Connecting,
    Downloading { got: u64, total: u64, since: Instant },
    /// Copying a classic TA archive from the player's disk.
    Copying { got: u64, total: u64 },
    /// Checking the file against its pinned checksum.
    Checking,
    Unpacking,
    /// Installed (as the mod `id`: a package's own manifest may name it otherwise).
    Done { units: usize, maps: usize, id: String },
    /// Stopped by the player: as if never started.
    Cancelled,
    Failed(String),
}

impl Stage {
    /// Still going (queued or under way).
    pub fn active(&self) -> bool {
        !matches!(self, Stage::Done { .. } | Stage::Cancelled | Stage::Failed(_))
    }
}

/// What to install.
#[derive(Clone, Debug)]
pub enum Source {
    /// A curated catalogue mod: downloaded from its provider, or from the selected local file.
    Catalogue(&'static Entry, Option<PathBuf>),
    /// A package the player brought (an archive or a folder), installed as the mod `id`.
    Package { path: PathBuf, id: String, name: String },
    /// A classic TA archive, kept as a single file in the mods folder.
    Classic { path: PathBuf, id: String, name: String },
    /// A mod its own installer put in the original game's folder, made for an engine of its own
    /// (`ta_vfs::mods::game_folder_mods`): installed as a mod with its archives linked from there
    /// (copied where a link cannot be made), the game's folder left as it is.
    GameFolder(ta_vfs::mods::GameFolderMod),
}

impl Source {
    /// The mod's id once installed.
    pub fn id(&self) -> &str {
        match self {
            Source::Catalogue(entry, _) => entry.id,
            Source::Package { id, .. } | Source::Classic { id, .. } => id,
            Source::GameFolder(found) => &found.id,
        }
    }

    /// A package as the mod it becomes. A folder laid out as a mod already (its `mod.toml`)
    /// keeps its id and name; anything else is named after the file or folder: the id its
    /// letters and digits in lower case, in any script (as `ta-tools mod-install` makes it),
    /// the name as it came. None when nothing of the name is left to make an id of. (An
    /// archive's own manifest is found once it is unpacked.)
    pub fn package(path: &Path) -> Option<Source> {
        if path.is_dir()
            && let Some(own) = own_manifest(path).ok().flatten()
        {
            return Some(Source::Package { path: path.to_path_buf(), id: own.id, name: own.name });
        }
        // The archive of a catalogue mod's tested release (its exact size: the installer checks
        // its digest) installs as that mod, with its name, version and page.
        if let Some(entry) = catalogue_release(path) {
            return Some(Source::Catalogue(entry, Some(path.to_path_buf())));
        }
        let stem = if path.is_dir() { path.file_name() } else { path.file_stem() }?.to_string_lossy().to_string();
        if !path.is_dir() && path.extension().and_then(|e| e.to_str()).is_some_and(|e| ta_vfs::ARCHIVE_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x))) {
            // Match ta_vfs::mods' implicit manifest, including spaces and underscores (and
            // in lower case as it is, any letter).
            return Some(Source::Classic { path: path.to_path_buf(), id: stem.to_lowercase(), name: stem });
        }
        let id: String = stem.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
        (!id.is_empty()).then(|| Source::Package { path: path.to_path_buf(), id, name: stem.trim().to_string() })
    }
}

/// The catalogue mod whose tested release `path` is, by its exact size (a package, not a
/// folder): the installer checks its digest before anything is installed.
pub fn catalogue_release(path: &Path) -> Option<&'static Entry> {
    if path.is_dir() { return None; }
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if !ta_vfs::PACKAGE_EXTENSIONS.contains(&extension.as_str()) { return None; }
    let bytes = std::fs::metadata(path).ok()?.len();
    catalogue::MODS.iter().find(|entry| entry.file.is_some_and(|file| file.bytes == bytes))
}

/// The manifest of a mod laid out as one already (`mod.toml` in `dir`), when it reads and its
/// id can name a folder. Invalid metadata must not silently become an implicit manifest.
fn own_manifest(dir: &Path) -> Result<Option<ta_vfs::mods::ModManifest>, String> {
    use std::io::Read;
    use ta_ui::i18n::{t, tf};
    const LIMIT: u64 = 1 << 20;
    let path = dir.join("mod.toml");
    let invalid = |error: &dyn std::fmt::Display| tf("Invalid manifest {path}: {error}", &[("path", &path.display()), ("error", error)]);
    // This also runs on the UI thread for dropped folders, before package planning.
    let mut text = String::new();
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(invalid(&error)),
    };
    file.take(LIMIT + 1).read_to_string(&mut text).map_err(|error| invalid(&error))?;
    if text.len() > LIMIT as usize {
        return Err(invalid(&t("mod manifest exceeds 1 MiB")));
    }
    let manifest: ta_vfs::mods::ModManifest = toml::from_str(&text).map_err(|error| invalid(&error))?;
    if !(1..=64).contains(&manifest.id.chars().count()) || !ta_vfs::package::safe_component(&manifest.id) {
        return Err(invalid(&t("invalid mod id")));
    }
    Ok(Some(manifest))
}

/// Why an install did not finish.
#[derive(Debug)]
enum Failure {
    Cancelled,
    Error(String),
}

impl From<String> for Failure {
    fn from(e: String) -> Failure {
        Failure::Error(e)
    }
}

pub struct Installer {
    requests: Sender<Source>,
    updates: Receiver<(String, Stage)>,
    cancelled: Arc<Mutex<Vec<String>>>,
    /// The download under way, to stop when the launcher closes.
    current: Arc<Mutex<Option<moddb::Stopper>>>,
    stopped: Arc<AtomicBool>,
    /// An install is under way on the worker (closing waits a moment for it to stop).
    working: Arc<AtomicBool>,
    /// The game runs: an installed mod is not replaced under it (its files may be open).
    game: Arc<AtomicBool>,
    pub stages: BTreeMap<String, Stage>,
    /// The packages asked for (their names), by id: those the catalogue does not name.
    pub packages: BTreeMap<String, String>,
    /// Installs finished since the page last looked (it rescans the mods).
    pub finished: Vec<String>,
    /// Installs that failed since the launcher last looked (it says so where the Mods page is not
    /// in sight).
    pub failed: Vec<String>,
}

impl Installer {
    /// Installs into `mods_dir`; downloads and unpacks under `cache`.
    pub fn new(mods_dir: PathBuf, cache: PathBuf) -> Installer {
        let (requests, jobs) = channel::<Source>();
        let (report, updates) = channel();
        let cancelled: Arc<Mutex<Vec<String>>> = Arc::default();
        let current: Arc<Mutex<Option<moddb::Stopper>>> = Arc::default();
        let stopped = Arc::new(AtomicBool::new(false));
        let closing = stopped.clone();
        let working = Arc::new(AtomicBool::new(false));
        let busy = working.clone();
        let game = Arc::new(AtomicBool::new(false));
        let playing = game.clone();
        let (stop, running) = (cancelled.clone(), current.clone());
        let worker = std::thread::Builder::new()
            .name("mod installs".into())
            .spawn(move || {
                for job in jobs {
                    busy.store(true, Ordering::Release);
                    let id = job.id().to_string();
                    let is_cancelled = || {
                        closing.load(Ordering::Relaxed) || stop.lock().is_ok_and(|mut c| match c.iter().position(|x| *x == id) {
                            Some(at) => {
                                c.remove(at);
                                true
                            }
                            None => false,
                        })
                    };
                    let stage = if is_cancelled() {
                        Stage::Cancelled
                    } else {
                        let send = |s: Stage| {
                            let _ = report.send((id.clone(), s));
                        };
                        match install(&job, &mods_dir, &cache, &send, &is_cancelled, &running, &playing) {
                            Ok((units, maps, installed)) => Stage::Done { units, maps, id: installed },
                            Err(Failure::Cancelled) => Stage::Cancelled,
                            Err(Failure::Error(e)) => {
                                log::warn!("installing {id}: {e}");
                                Stage::Failed(e)
                            }
                        }
                    };
                    let _ = report.send((id, stage));
                    busy.store(false, Ordering::Release);
                }
            });
        if let Err(error) = worker { log::error!("could not start mod installs: {error}"); }
        Installer { requests, updates, cancelled, current, stopped, working, game, stages: BTreeMap::new(), packages: BTreeMap::new(), finished: Vec::new(), failed: Vec::new() }
    }

    /// Whether the game runs now (an installed mod is not replaced under it).
    pub fn set_game_running(&self, running: bool) {
        self.game.store(running, Ordering::Release);
    }

    /// Stops the download and waits for any short installation publication to finish.
    /// Unpacking/copying never holds this mutex, so closing does not wait for it.
    pub fn shutdown(&self) {
        let first = !self.stopped.swap(true, Ordering::Relaxed);
        if let Ok(current) = self.current.lock()
            && let Some(stopper) = current.as_ref()
        {
            stopper.stop();
        }
        // (The install under way stops at its next check, an unpacker killed: a moment's wait
        // lets it take its working folder with it rather than leave it behind.)
        let until = Instant::now() + Duration::from_secs(3);
        while first && self.working.load(Ordering::Acquire) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Installs a catalogue mod (downloading it, or from a file the player has).
    pub fn install(&mut self, entry: &'static Entry, from: Option<PathBuf>) {
        self.start(Source::Catalogue(entry, from));
    }

    /// Installs a mod (unless it is already under way).
    pub fn start(&mut self, source: Source) {
        let id = source.id().to_string();
        if self.stages.get(&id).is_some_and(Stage::active) {
            return;
        }
        if let Source::Catalogue(entry, None) = &source
            && entry.can_import() && entry.file.is_none_or(|file| !file.can_download())
        {
            self.stages.insert(id, Stage::Failed(manual_install_message()));
            return;
        }
        if let Ok(mut c) = self.cancelled.lock() {
            c.retain(|x| *x != id);
        }
        if let Source::Package { name, .. } | Source::Classic { name, .. } | Source::GameFolder(ta_vfs::mods::GameFolderMod { name, .. }) = &source {
            self.packages.insert(id.clone(), name.clone());
        } else {
            self.packages.remove(&id);
        }
        let stage = if self.requests.send(source).is_ok() { Stage::Queued }
            else { Stage::Failed(installer_stopped_message()) };
        self.stages.insert(id, stage);
    }

    pub fn cancel(&mut self, id: &str) {
        if self.stages.get(id).is_some_and(Stage::active)
            && let Ok(mut c) = self.cancelled.lock()
            && !c.iter().any(|x| x == id)
        {
            c.push(id.to_string());
        }
    }

    /// Takes in the worker's news; true when something changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            let (id, stage) = match self.updates.try_recv() {
                Ok(update) => update,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // A panicked or unstartable worker must not leave every queued
                    // import active forever, blocking profiles and application updates.
                    for stage in self.stages.values_mut().filter(|stage| stage.active()) {
                        *stage = Stage::Failed(installer_stopped_message());
                        changed = true;
                    }
                    break;
                }
            };
            match stage {
                Stage::Done { .. } => self.finished.push(id.clone()),
                Stage::Failed(_) => self.failed.push(id.clone()),
                _ => {}
            }
            self.stages.insert(id, stage);
            changed = true;
        }
        changed
    }

    /// Something is queued or under way.
    pub fn busy(&self) -> bool {
        self.stages.values().any(Stage::active)
    }
}

impl Drop for Installer {
    fn drop(&mut self) { self.shutdown(); }
}

fn installer_stopped_message() -> String {
    ta_ui::i18n::t("The mod installer stopped unexpectedly. Restart the launcher and try again.").into()
}

/// Installs one mod; returns the units and maps it brought, and the id it was installed as.
fn install(
    source: &Source,
    mods_dir: &Path,
    cache: &Path,
    send: &dyn Fn(Stage),
    cancelled: &dyn Fn() -> bool,
    running: &Mutex<Option<moddb::Stopper>>,
    game: &AtomicBool,
) -> Result<(usize, usize, String), Failure> {
    use ta_ui::i18n::{t, tf};
    let (path, manifest) = match source {
        Source::Classic { path, id, name } => {
            let destination = archive_destination(path, mods_dir).map_err(|error| error.to_string())?;
            game_holds(game, &destination, name)?;
            ta_formats::hpi::HpiArchive::open(path).map_err(|error| error.to_string())?;
            copy_archive(path, mods_dir, &|got, total| send(Stage::Copying { got, total }), cancelled)
                .map_err(|e| if e.kind() == std::io::ErrorKind::Interrupted { Failure::Cancelled } else { Failure::Error(e.to_string()) })?;
            return Ok((0, 0, id.clone()));
        }
        Source::Catalogue(entry, from) => return install_catalogue(entry, from.as_deref(), mods_dir, cache, send, cancelled, running, game),
        Source::GameFolder(found) => return install_from_game_folder(found, mods_dir, cache, send, cancelled, running, game),
        Source::Package { path, id, name } => {
            let manifest = ta_vfs::mods::ModManifest {
                id: id.clone(),
                name: name.clone(),
                // (Unknown: a package says what it is in a mod.toml of its own.)
                version: String::new(),
                description: String::new(),
                authors: Vec::new(),
                depends: Vec::new(),
                load_after: Vec::new(),
                source: String::new(),
                rename: BTreeMap::new(),
                replaces: Vec::new(),
            };
            (path.clone(), manifest)
        }
    };
    if cancelled() {
        return Err(Failure::Cancelled);
    }
    send(Stage::Unpacking);
    if !ta_vfs::package::safe_component(&manifest.id) || !(1..=64).contains(&manifest.id.chars().count()) {
        return Err(t("invalid mod id").to_string().into());
    }
    let legacy_temp = cache.join("unpack").join(&manifest.id);
    // A recovered old staging folder may contain unfinished files. Preserve it and
    // ask the player to move it before importing; never clear it as scratch space.
    if within(&path, &legacy_temp) || within(&legacy_temp, &path) {
        return Err(t("the source overlaps the install staging folder; move it outside the cache first").to_string().into());
    }
    let workspace = InstallWorkspace::create(&cache.join("unpack"))?;
    let temp = workspace.0.join("content");
    (|| -> Result<(usize, usize, String), Failure> {
        let (plan, own) = if path.is_dir() {
            // A folder needs no unpacking: it is planned where it is, and what goes into the mod
            // copied out of it (the player's folder stays as it was). Not a drive, nor a folder
            // holding the launcher's own (it would be copied into itself).
            if path.parent().is_none() || within(cache, &path) || within(mods_dir, &path) {
                return Err(tf("{path} holds much more than a mod: drop the mod's own folder", &[("path", &path.display())]).into());
            }
            // A mod in the mods folder is installed already: copied out and back, what the
            // install leaves out (its sources, anything of its author's) would be lost.
            if within(&path, mods_dir) {
                return Err(tf("{path} is in your mods folder already, so it is installed: turn it on in the library", &[("path", &path.display())]).into());
            }
            let plan = ta_vfs::package::plan(&path).map_err(|e| package_error(&e))?;
            let own = match own_manifest(&plan.root)? { Some(own) => Some(own), None => own_manifest(&path)? };
            (copy_planned(&plan, &temp, cancelled)
                .map_err(|e| if e.kind() == std::io::ErrorKind::Interrupted { Failure::Cancelled } else { Failure::Error(e.to_string()) })?, own)
        } else {
            let unpacker = ta_vfs::package::Unpacker::find().ok_or_else(|| package_error(&ta_vfs::package::PackageError::NoUnpacker))?;
            unpacker.unpack_cancellable(&path, &temp, cancelled).map_err(unpack_failure)?;
            let plan = ta_vfs::package::plan(&temp).map_err(|e| package_error(&e))?;
            let own = match own_manifest(&plan.root)? { Some(own) => Some(own), None => own_manifest(&temp)? };
            (plan, own)
        };
        // A mod laid out as one already keeps its own manifest (its id, what it needs).
        let manifest = own.unwrap_or_else(|| manifest.clone());
        // (Cancelled while it was being unpacked: nothing is installed.)
        if cancelled() {
            return Err(Failure::Cancelled);
        }
        publish_install(&plan, &manifest, mods_dir, cancelled, running, game)?;
        log::info!("installed {} {} ({} units, {} maps; left out: {})", manifest.name, manifest.version, plan.units, plan.maps, plan.skipped.join(", "));
        Ok((plan.units, plan.maps, manifest.id))
    })()
}

/// Installs a mod found in the original game's folder as a mod of its own: its archives linked
/// into a work folder from the game's (copied where they cannot be: another drive), then published
/// as any install is. The catalogue's mods keep their name, team and page; which release the
/// game's folder holds is not known, so neither is its version.
fn install_from_game_folder(
    found: &ta_vfs::mods::GameFolderMod,
    mods_dir: &Path,
    cache: &Path,
    send: &dyn Fn(Stage),
    cancelled: &dyn Fn() -> bool,
    running: &Mutex<Option<moddb::Stopper>>,
    game: &AtomicBool,
) -> Result<(usize, usize, String), Failure> {
    let entry = catalogue::get(&found.id);
    let manifest = ta_vfs::mods::ModManifest {
        id: found.id.clone(),
        name: entry.map_or_else(|| found.name.clone(), |entry| entry.name.to_string()),
        version: String::new(),
        description: entry.map_or_else(String::new, |entry| entry.pitch.to_string()),
        authors: entry.map(|entry| vec![entry.team.to_string()]).unwrap_or_default(),
        depends: Vec::new(),
        load_after: Vec::new(),
        source: entry.map_or_else(String::new, |entry| entry.page()),
        rename: found.rename.clone(),
        replaces: Vec::new(),
    };
    if !ta_vfs::package::safe_component(&manifest.id) || !(1..=ta_vfs::mods::MOST_ID_LETTERS).contains(&manifest.id.chars().count()) {
        return Err(ta_ui::i18n::t("invalid mod id").to_string().into());
    }
    let plan = ta_vfs::package::game_folder_plan(found);
    if plan.archives.is_empty() || plan.archives.iter().any(|path| !path.is_file()) {
        return Err(ta_ui::i18n::tf("{name} is no longer in the game's folder", &[("name", &manifest.name)]).into());
    }
    let total: u64 = plan.archives.iter().map(|path| std::fs::metadata(path).map_or(0, |m| m.len())).sum();
    send(Stage::Copying { got: 0, total });
    let workspace = InstallWorkspace::create(&cache.join("unpack"))?;
    let temp = workspace.0.join("content");
    let linked = link_planned(&plan, &temp, &|got| send(Stage::Copying { got, total }), cancelled)
        .map_err(|e| if e.kind() == std::io::ErrorKind::Interrupted { Failure::Cancelled } else { Failure::Error(e.to_string()) })?;
    if cancelled() {
        return Err(Failure::Cancelled);
    }
    publish_install(&linked, &manifest, mods_dir, cancelled, running, game)?;
    log::info!("added {} from the game's folder ({} units, {} maps; {})", manifest.name, plan.units, plan.maps,
        plan.archives.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", "));
    Ok((plan.units, plan.maps, manifest.id))
}

/// The plan's archives in `to` (made): each a hard link to the original (the same file in two
/// folders, taking no more room), or a copy where the disk makes none (another drive); `progress`
/// told the bytes done.
fn link_planned(plan: &ta_vfs::package::Plan, to: &Path, progress: &dyn Fn(u64), cancelled: &dyn Fn() -> bool) -> std::io::Result<ta_vfs::package::Plan> {
    std::fs::create_dir_all(to)?;
    let mut done = 0u64;
    let mut archives = Vec::new();
    for from in &plan.archives {
        if cancelled() { return Err(std::io::ErrorKind::Interrupted.into()); }
        let target = to.join(from.file_name().unwrap_or_default());
        if std::fs::hard_link(from, &target).is_err() {
            copy_file(from, &target, cancelled)?;
        }
        done += std::fs::metadata(&target).map_or(0, |m| m.len());
        progress(done);
        archives.push(target);
    }
    Ok(ta_vfs::package::Plan { root: to.to_path_buf(), archives, ..plan.clone() })
}

/// Close must not terminate the process between moving the previous version aside
/// and publishing its replacement. The same short gate lets shutdown stop active
/// transfers immediately while preventing any later publication from starting.
fn publish_install(
    plan: &ta_vfs::package::Plan,
    manifest: &ta_vfs::mods::ModManifest,
    mods_dir: &Path,
    cancelled: &dyn Fn() -> bool,
    running: &Mutex<Option<moddb::Stopper>>,
    game: &AtomicBool,
) -> Result<PathBuf, Failure> {
    use ta_vfs::package::PackageError;
    // (Where it goes: the copy in use, or a free folder; never one holding another mod.)
    if let Some(target) = ta_vfs::package::install_target(mods_dir, &manifest.id) {
        game_holds(game, &target, &manifest.name)?;
    }
    ta_vfs::package::install_with_commit_guard(plan, manifest, mods_dir, || {
        let guard = running.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if cancelled() {
            return Err(PackageError::Io { path: mods_dir.display().to_string(), source: std::io::ErrorKind::Interrupted.into() });
        }
        Ok(guard)
    }).map_err(|error| match error {
        PackageError::Io { source, .. } if source.kind() == std::io::ErrorKind::Interrupted => Failure::Cancelled,
        error => Failure::Error(package_error(&error)),
    })
}

/// While the game runs, an installed mod (`installed`, its folder or archive) is not replaced:
/// the game may have its files open (the swap would fail half way, or change them under it).
fn game_holds(game: &AtomicBool, installed: &Path, name: &str) -> Result<(), Failure> {
    if game.load(Ordering::Acquire) && installed.exists() {
        return Err(Failure::Error(ta_ui::i18n::tf("Close the game first: {name} is installed and the game may be using it. Install it again once the game has closed.", &[("name", &name)])));
    }
    Ok(())
}

/// An unpacker's failure as an install's (a cancel stays a cancel).
fn unpack_failure(e: ta_vfs::package::PackageError) -> Failure {
    match e {
        ta_vfs::package::PackageError::Cancelled => Failure::Cancelled,
        e => Failure::Error(package_error(&e)),
    }
}

/// A package's failure in the interface's language: those a player meets (no program to unpack
/// it, an archive only partly unpacked, none of TA's content in it, the unpacker's own failure).
/// Others are as they are: a file system's error, in the system's words, or a refusal of what an
/// unsafe package holds.
pub fn package_error(e: &ta_vfs::package::PackageError) -> String {
    use ta_ui::i18n::{t, tf};
    use ta_vfs::package::PackageError as E;
    match e {
        E::NoUnpacker => t("No program to unpack archives was found: install 7-Zip from 7-zip.org.").to_string(),
        E::Incomplete { file } => tf("Only part of {file} could be unpacked: install 7-Zip (7-zip.org) and try again.", &[("file", file)]),
        E::NoContent => t("No Total Annihilation content was found in the package.").to_string(),
        E::Unpack { tool, file, message } => tf("{tool} could not unpack {file}: {message}", &[("tool", tool), ("file", file), ("message", message)]),
        other => other.to_string(),
    }
}

/// Removes working folders (`<prefix>-<pid>-<n>`) another run of the launcher left behind in
/// `dir`: none of this one's, nor any touched in the last `min_age` (a setup may still be
/// reading its own).
pub fn sweep_stale(dir: &Path, prefix: &str, min_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let ours = std::process::id().to_string();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(rest) = name.strip_prefix(prefix).and_then(|r| r.strip_prefix('-')) else { continue };
        if rest.split('-').next() == Some(ours.as_str()) {
            continue;
        }
        let old = entry.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|age| age >= min_age));
        if old && entry.file_type().is_ok_and(|t| t.is_dir()) {
            match std::fs::remove_dir_all(entry.path()) {
                Ok(()) => log::info!("removed a working folder left behind: {}", entry.path().display()),
                Err(e) => log::warn!("could not remove {}: {e}", entry.path().display()),
            }
        }
    }
}

/// Removes the part-copied classic archives (`.import-<pid>-<n>.part`) another run of the
/// launcher left in the mods folder, untouched for `min_age`.
pub fn sweep_stale_imports(mods_dir: &Path, min_age: Duration) {
    let Ok(entries) = std::fs::read_dir(mods_dir) else { return };
    let ours = std::process::id().to_string();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(rest) = name.strip_prefix(".import-").and_then(|r| r.strip_suffix(".part")) else { continue };
        if rest.split('-').next() == Some(ours.as_str()) || !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        if entry.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|age| age >= min_age)) {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => log::info!("removed a part-copied archive left behind: {}", entry.path().display()),
                Err(e) => log::warn!("could not remove {}: {e}", entry.path().display()),
            }
        }
    }
}

/// A private workspace keeps concurrent launcher instances and cancelled installs apart.
struct InstallWorkspace(PathBuf);

impl InstallWorkspace {
    fn create(parent: &Path) -> Result<Self, String> {
        use std::sync::atomic::AtomicU64;
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        for _ in 0..100 {
            let path = parent.join(format!("install-{}-{}", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {},
                Err(e) => return Err(e.to_string()),
            }
        }
        Err(ta_ui::i18n::t("could not create a temporary install folder").into())
    }
}

impl Drop for InstallWorkspace {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

/// Curated downloads use a private workspace and the same verification for manual import.
/// Website-only entries accept a player-selected package through the same safe unpacker
/// and content planner, without claiming a pinned version or a verified release digest.
/// The installed version is replaced only after the checked package has unpacked and planned.
#[allow(clippy::too_many_arguments)]
fn install_catalogue(
    entry: &'static Entry,
    from: Option<&Path>,
    mods_dir: &Path,
    cache: &Path,
    send: &dyn Fn(Stage),
    cancelled: &dyn Fn() -> bool,
    running: &Mutex<Option<moddb::Stopper>>,
    game: &AtomicBool,
) -> Result<(usize, usize, String), Failure> {
    use ta_ui::i18n::{t, tf};
    let file = entry.file;
    if !entry.can_import() { return Err(t("There is nothing to download for this mod.").to_string().into()); }
    if from.is_none() && file.is_none_or(|file| !file.can_download()) { return Err(manual_install_message().into()); }
    if file.is_some_and(|file| !ta_vfs::package::safe_component(file.name)) { return Err(t("archive has an invalid file name").to_string().into()); }
    if cancelled() { return Err(Failure::Cancelled); }
    let workspace = InstallWorkspace::create(&cache.join("catalogue"))?;
    let archive = if let Some(source) = from {
        // Pinned releases check size before copying and hash the private copy before use.
        // Website-only imports still copy into a private workspace and use the safe planner.
        let meta = std::fs::metadata(source).map_err(|e| e.to_string())?;
        if !meta.is_file() { return Err(t("archive is not a file").to_string().into()); }
        if let Some(file) = file && meta.len() != file.bytes {
            return Err(tf("{file} does not match the verified release. Download it again from the mod's website.", &[("file", &file.name)]).into());
        }
        let downloads = workspace.0.join("downloads");
        copy_archive(source, &downloads, &|got, total| send(Stage::Copying { got, total }), cancelled)
            .map_err(|e| if e.kind() == std::io::ErrorKind::Interrupted { Failure::Cancelled } else { Failure::Error(e.to_string()) })?;
        let path = downloads.join(source.file_name().ok_or_else(|| t("archive has no file name").to_string())?);
        if let Some(file) = file {
            send(Stage::Checking);
            verify_file(&path, &file, cancelled)?;
        }
        path
    } else {
        download(entry, &file.expect("downloadable release checked above"), &workspace.0, send, cancelled, running)?
    };
    if cancelled() { return Err(Failure::Cancelled); }
    send(Stage::Unpacking);
    let unpacker = ta_vfs::package::Unpacker::find().ok_or_else(|| package_error(&ta_vfs::package::PackageError::NoUnpacker))?;
    let unpacked = workspace.0.join("unpacked");
    unpacker.unpack_cancellable(&archive, &unpacked, cancelled).map_err(unpack_failure)?;
    let plan = ta_vfs::package::plan(&unpacked).map_err(|e| package_error(&e))?;
    let manifest = ta_vfs::mods::ModManifest {
        id: entry.id.into(), name: entry.name.into(), version: file.map_or("unverified", |file| file.version).into(),
        description: entry.pitch.into(), authors: vec![entry.team.into()],
        depends: Vec::new(), load_after: Vec::new(), source: entry.page(), rename: BTreeMap::new(), replaces: Vec::new(),
    };
    if cancelled() { return Err(Failure::Cancelled); }
    publish_install(&plan, &manifest, mods_dir, cancelled, running, game)?;
    log::info!("installed {} {} ({} units, {} maps; left out: {})", manifest.name, manifest.version, plan.units, plan.maps, plan.skipped.join(", "));
    Ok((plan.units, plan.maps, manifest.id))
}

fn digest(path: &Path, sha256: bool, cancelled: &dyn Fn() -> bool) -> Result<String, Failure> {
    use sha2::Digest;
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut sha = sha2::Sha256::new();
    let mut md5 = md5::Md5::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        if cancelled() { return Err(Failure::Cancelled); }
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 { break; }
        if sha256 { sha.update(&buffer[..count]); } else { md5.update(&buffer[..count]); }
    }
    Ok(if sha256 { format!("{:x}", sha.finalize()) } else { format!("{:x}", md5.finalize()) })
}

fn verify_file(path: &Path, file: &catalogue::File, cancelled: &dyn Fn() -> bool) -> Result<(), Failure> {
    let expected = if file.sha256.is_empty() { file.md5 } else { file.sha256 };
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size != file.bytes || digest(path, !file.sha256.is_empty(), cancelled)? != expected {
        return Err(ta_ui::i18n::tf("{file} does not match the verified release. Download it again from the mod's website.", &[("file", &file.name)]).into());
    }
    Ok(())
}

fn manual_install_message() -> String {
    ta_ui::i18n::t("Open the mod's website, then choose Have the file? to install your downloaded archive.").into()
}

/// Downloads a catalogue mod's file into the cache and checks it; returns the file. Whatever
/// goes wrong (the player cancelling too), nothing of it is left behind.
fn download(
    entry: &'static Entry,
    file: &catalogue::File,
    cache: &Path,
    send: &dyn Fn(Stage),
    cancelled: &dyn Fn() -> bool,
    running: &Mutex<Option<moddb::Stopper>>,
) -> Result<PathBuf, Failure> {
    use ta_ui::i18n::{t, tf};
    let (urls, referer, policy) = match file.download {
        catalogue::Download::ManualOnly => return Err(manual_install_message().into()),
        catalogue::Download::ModDb { page, start } => {
            send(Stage::Connecting);
            let slug = entry.moddb().ok_or_else(|| t("invalid download address").to_string())?;
            let start = moddb::start_url(start);
            let page = moddb::fetch_text(&start, Some(&moddb::file_page(slug, page)))?;
            let mirror = moddb::parse_mirror(&page).ok_or_else(|| t("ModDB's download page has changed. Get the file from the mod's page, then install it from the file.").to_string())?;
            (vec![mirror], start, crate::http::Policy::Web)
        }
        catalogue::Download::Direct(urls) => (urls.iter().map(|url| (*url).to_string()).collect(), entry.page(), crate::http::Policy::Update { local_files: false }),
    };
    let urls: Vec<_> = urls.iter().map(String::as_str).collect();
    let result = download_sources(file, &urls, &referer, policy, cache, send, cancelled, running);
    match result {
        Err(Failure::Error(error)) if matches!(file.download, catalogue::Download::Direct(_)) =>
            Err(tf("The download could not be completed: {error}. Open the mod's website or download link in your browser, then choose Have the file? to install it.", &[("error", &error)]).into()),
        other => other,
    }
}

/// Try each curated mirror in order, rejecting incomplete or changed bytes before trying
/// the next. The policy is explicit so direct releases cannot redirect to plaintext HTTP.
fn download_sources(
    file: &catalogue::File,
    urls: &[&str],
    referer: &str,
    policy: crate::http::Policy,
    cache: &Path,
    send: &dyn Fn(Stage),
    cancelled: &dyn Fn() -> bool,
    running: &Mutex<Option<moddb::Stopper>>,
) -> Result<PathBuf, Failure> {
    let part = cache.join("downloads").join(format!("{}.part", file.name));
    let done = part.with_file_name(file.name);
    let mut last = ta_ui::i18n::t("There is nothing to download for this mod.").to_string();
    for url in urls {
        if cancelled() { return Err(Failure::Cancelled); }
        send(Stage::Connecting);
        let result = (|| {
            let mut transfer = match policy {
                crate::http::Policy::Web => moddb::Transfer::start(url, referer, &part, file.bytes)?,
                policy => moddb::Transfer::start_with_policy(url, referer, &part, policy, file.bytes)?,
            };
            if let Ok(mut r) = running.lock() { *r = Some(transfer.stopper()); }
            let since = Instant::now();
            loop {
                if cancelled() {
                    transfer.cancel();
                    return Err(Failure::Cancelled);
                }
                if let Some(result) = transfer.poll() { result?; break; }
                send(Stage::Downloading { got: transfer.received(), total: file.bytes, since });
                std::thread::sleep(Duration::from_millis(150));
            }
            send(Stage::Checking);
            verify_file(&part, file, cancelled)?;
            if cancelled() { return Err(Failure::Cancelled); }
            std::fs::rename(&part, &done).map_err(|e| e.to_string())?;
            Ok(done.clone())
        })();
        if let Ok(mut r) = running.lock() { *r = None; }
        match result {
            Ok(path) => return Ok(path),
            Err(failure) => {
                let _ = std::fs::remove_file(&part);
                let _ = std::fs::remove_file(&done);
                match failure {
                    Failure::Cancelled => return Err(Failure::Cancelled),
                    Failure::Error(error) => {
                        log::info!("{}: mirror {url}: {error}", file.name);
                        last = error;
                    }
                }
            }
        }
    }
    Err(Failure::Error(last))
}

/// Copies what `plan` takes from a folder (its archives, folders and documents) into `to`,
/// and returns the plan for the copy.
fn copy_planned(plan: &ta_vfs::package::Plan, to: &Path, cancelled: &dyn Fn() -> bool) -> std::io::Result<ta_vfs::package::Plan> {
    std::fs::create_dir_all(to)?;
    let copy = |from: &PathBuf| -> std::io::Result<PathBuf> {
        let target = to.join(from.file_name().unwrap_or_default());
        if from.is_dir() {
            copy_dir(from, &target, cancelled)?;
        } else {
            copy_file(from, &target, cancelled)?;
        }
        Ok(target)
    };
    Ok(ta_vfs::package::Plan {
        root: to.to_path_buf(),
        archives: plan.archives.iter().map(copy).collect::<Result<_, _>>()?,
        folders: plan.folders.iter().map(copy).collect::<Result<_, _>>()?,
        docs: plan.docs.iter().map(copy).collect::<Result<_, _>>()?,
        ..plan.clone()
    })
}

/// Whether `place` is `folder` or in it, as the file system has them (links and the case of
/// letters aside; `place` need not be there yet).
fn within(place: &Path, folder: &Path) -> bool {
    let real = |p: &Path| p.ancestors().find_map(|a| Some(std::fs::canonicalize(a).ok()?.join(p.strip_prefix(a).ok()?)));
    matches!((real(place), real(folder)), (Some(p), Some(f)) if p.starts_with(&f))
}

/// Copies the folder `from` into `to` (created), files and folders alike.
fn copy_dir(from: &Path, to: &Path, cancelled: &dyn Fn() -> bool) -> std::io::Result<()> {
    if cancelled() { return Err(std::io::ErrorKind::Interrupted.into()); }
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target, cancelled)?;
        } else {
            copy_file(&entry.path(), &target, cancelled)?;
        }
    }
    Ok(())
}

/// Folder imports also need cancellation within a large file, not only after all
/// their assets have copied. The destination belongs to this request's workspace.
fn copy_file(from: &Path, to: &Path, cancelled: &dyn Fn() -> bool) -> std::io::Result<()> {
    use std::io::{Read, Write};
    if cancelled() { return Err(std::io::ErrorKind::Interrupted.into()); }
    let mut input = std::fs::File::open(from)?;
    let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(to)?;
    let mut buffer = vec![0; 256 << 10];
    loop {
        if cancelled() { return Err(std::io::ErrorKind::Interrupted.into()); }
        let count = input.read(&mut buffer)?;
        if count == 0 { return Ok(()); }
        output.write_all(&buffer[..count])?;
    }
}

/// Import a classic archive without truncating the source or a previously installed copy.
/// Only a completed copy replaces the old file; create_new also refuses a staged symlink.
#[cfg(test)]
fn import_archive(source: &Path, mods: &Path) -> std::io::Result<()> {
    copy_archive(source, mods, &|_, _| {}, &|| false)
}

/// Classic imports update the installed identity even if its filename/extension differs.
/// Packages used as private catalogue downloads keep their download filename.
fn archive_destination(source: &Path, mods: &Path) -> std::io::Result<PathBuf> {
    use std::io;
    use ta_ui::i18n::{t, tf};
    let name = source.file_name().ok_or_else(|| io::Error::other(t("archive has no file name")))?;
    if !name.to_str().is_some_and(ta_vfs::package::safe_component) {
        return Err(io::Error::other(t("archive has an invalid file name")));
    }
    if source.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ta_vfs::ARCHIVE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())) {
        let id = source.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
        let matching: Vec<_> = ta_vfs::mods::scan(mods).0.into_iter().filter(|info| info.manifest.id == id).collect();
        if matching.len() > 1 || matching.iter().any(|info| info.kind != ta_vfs::mods::ModKind::Archive) {
            return Err(io::Error::other(tf("Mod {id} has a conflicting installation; remove it in the mod library before importing this archive", &[("id", &id)])));
        }
        if let Some(info) = matching.into_iter().next() { return Ok(info.path); }
    }
    let destination = mods.join(name);
    if destination.is_dir() {
        return Err(io::Error::other(tf("Mod {id} has a conflicting installation; remove it in the mod library before importing this archive", &[("id", &name.to_string_lossy())])));
    }
    Ok(destination)
}

/// Copy in bounded chunks so cancellation also works on slow disks. The temporary file
/// lives beside the destination: committing the completed copy is a single rename.
fn copy_archive(source: &Path, mods: &Path, progress: &dyn Fn(u64, u64), cancelled: &dyn Fn() -> bool) -> std::io::Result<()> {
    use std::io::{self, Read, Write};
    use std::sync::atomic::AtomicU64;
    use ta_ui::i18n::t;
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let check_cancelled = || if cancelled() { Err(io::Error::from(io::ErrorKind::Interrupted)) } else { Ok(()) };
    check_cancelled()?;
    let destination = archive_destination(source, mods)?;
    std::fs::create_dir_all(mods)?;
    let source_real = std::fs::canonicalize(source)?;
    if std::fs::canonicalize(&destination).is_ok_and(|path| path == source_real) {
        return Ok(());
    }
    let mut input = std::fs::File::open(&source_real)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other(t("archive is not a file")));
    }
    let total = metadata.len();
    progress(0, total);
    let temporary = mods.join(format!(".import-{}-{}.part", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)));
    let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let result = (|| {
        let mut buffer = vec![0; 256 << 10];
        let mut got = 0u64;
        let mut shown = Instant::now();
        loop {
            check_cancelled()?;
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
            got += count as u64;
            if shown.elapsed() >= Duration::from_millis(120) {
                progress(got, total);
                shown = Instant::now();
            }
        }
        // A source that changed while copying is not a complete import of the chosen file.
        if got != total {
            return Err(io::Error::other(t("the archive changed while it was being copied; try importing it again")));
        }
        check_cancelled()?;
        output.sync_all()?;
        progress(got, total);
        check_cancelled()
    })();
    drop(output);
    let result = result.and_then(|()| std::fs::rename(&temporary, &destination));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// The mods' pictures and their newest files on ModDB, fetched in the background (from the
/// cache when fresh: pictures for a month, feeds for a day).
pub struct Media {
    updates: Option<Receiver<MediaUpdate>>,
    started: bool,
    wake: Arc<Mutex<Option<egui::Context>>>,
    pub pictures: BTreeMap<&'static str, egui::TextureHandle>,
    /// Each mod's files on ModDB, newest first.
    pub files: BTreeMap<&'static str, Vec<FeedItem>>,
}

enum MediaUpdate {
    Picture(&'static str, egui::ColorImage),
    Feed(&'static str, Vec<FeedItem>),
    Finished,
}

const PICTURE_AGE: Duration = Duration::from_secs(30 * 24 * 3600);
const FEED_AGE: Duration = Duration::from_secs(24 * 3600);

impl Media {
    pub fn new() -> Media {
        Media { updates: None, started: false, wake: Arc::default(), pictures: BTreeMap::new(), files: BTreeMap::new() }
    }

    /// Starts fetching (once); `offline` keeps to the cache.
    pub fn start(&mut self, cache: PathBuf, offline: bool) {
        if self.started {
            return;
        }
        self.started = true;
        let (tx, rx) = channel();
        self.updates = Some(rx);
        let wake = self.wake.clone();
        let worker = std::thread::Builder::new().name("mod pictures".into()).spawn(move || {
            let send = |update| {
                let sent = tx.send(update).is_ok();
                if let Ok(wake) = wake.lock() && let Some(ctx) = wake.as_ref() {
                    ctx.request_repaint();
                }
                sent
            };
            // Bundled art must not wait behind a slow or unavailable remote image host.
            for entry in catalogue::MODS.iter().filter(|e| e.bundled_image().is_some()) {
                if let Some(image) = picture(entry, &cache.join("pictures"), offline)
                    && !send(MediaUpdate::Picture(entry.id, image))
                {
                    return;
                }
            }
            for entry in catalogue::MODS.iter().filter(|e| !e.image.is_empty() && e.bundled_image().is_none()) {
                if let Some(image) = picture(entry, &cache.join("pictures"), offline)
                    && !send(MediaUpdate::Picture(entry.id, image))
                {
                    return;
                }
            }
            for entry in catalogue::MODS.iter().filter(|e| e.file.is_some() && e.moddb().is_some()) {
                let path = cache.join("feeds").join(format!("{}.xml", entry.id));
                let Some(xml) = cached(&path, FEED_AGE, offline, |to| {
                    let xml = moddb::fetch_text(&moddb::feed_url(entry.moddb().expect("ModDB entries only")), None)?;
                    std::fs::create_dir_all(to.parent().unwrap_or(to)).map_err(|e| e.to_string())?;
                    std::fs::write(to, xml).map_err(|e| e.to_string())
                }) else {
                    continue;
                };
                let items = moddb::parse_feed(&String::from_utf8_lossy(&xml));
                if !items.is_empty() && !send(MediaUpdate::Feed(entry.id, items)) {
                    return;
                }
            }
            send(MediaUpdate::Finished);
        });
        if let Err(e) = worker {
            log::warn!("could not start mod pictures: {e}");
            self.updates = None;
        }
    }

    /// Takes in what has come; true when something did.
    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
        // Register before draining: a result sent before registration is taken below;
        // anything sent afterwards wakes the idle UI immediately.
        if let Ok(mut wake) = self.wake.lock() && wake.is_none() {
            *wake = Some(ctx.clone());
        }
        let Some(rx) = &self.updates else { return false };
        let mut changed = false;
        let mut finished = false;
        loop {
            let update = match rx.try_recv() {
                Ok(update) => update,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            };
            match update {
                MediaUpdate::Picture(id, image) => {
                    self.pictures.insert(id, ctx.load_texture(format!("mod-{id}"), image, egui::TextureOptions::LINEAR));
                }
                MediaUpdate::Feed(id, items) => {
                    self.files.insert(id, items);
                }
                MediaUpdate::Finished => {
                    finished = true;
                    break;
                }
            }
            changed = true;
        }
        if finished {
            self.updates = None;
        }
        changed
    }

    /// A file on ModDB newer than the one offered, if any.
    pub fn newer(&self, entry: &Entry) -> Option<&FeedItem> {
        entry.moddb()?;
        let catalogue::Download::ModDb { page, .. } = entry.file?.download else { return None };
        let items = self.files.get(entry.id)?;
        let ours = items.iter().position(|i| i.file() == page)?;
        items[..ours].first()
    }
}

/// A file from the cache when young enough, else fetched with `fetch` into it (never when
/// offline); a stale copy beats nothing.
fn cached(path: &Path, max_age: Duration, offline: bool, fetch: impl FnOnce(&Path) -> Result<(), String>) -> Option<Vec<u8>> {
    use std::io::Read;
    const LIMIT: u64 = 16 << 20;
    let fresh = std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|age| age < max_age);
    if !fresh && !offline {
        // Refresh beside the cached file. A timeout must not truncate/delete the
        // stale image or feed that we can still show while the service is down.
        use std::sync::atomic::AtomicU64;
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let temporary = path.with_file_name(format!(".media-{}-{}.part", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)));
        let refreshed = fetch(&temporary).and_then(|()| {
            let size = std::fs::metadata(&temporary).map_err(|e| e.to_string())?.len();
            if size > LIMIT {
                return Err(ta_ui::i18n::t("the server's response is too large or could not be read").into());
            }
            std::fs::rename(&temporary, path).map_err(|e| e.to_string())
        });
        if let Err(e) = refreshed {
            log::info!("{}: {e}", path.display());
        }
        let _ = std::fs::remove_file(temporary);
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(LIMIT + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= LIMIT as usize).then_some(bytes)
}

fn picture(entry: &Entry, dir: &Path, offline: bool) -> Option<egui::ColorImage> {
    if let Some(bytes) = entry.bundled_image() {
        return bounded_picture(bytes, 1600, 1000).map_err(|e| log::warn!("{} bundled banner: {e}", entry.id)).ok();
    }
    let ext = entry.image.rsplit('.').next().unwrap_or("png");
    let path = dir.join(format!("{}.{ext}", entry.id));
    let bytes = cached(&path, PICTURE_AGE, offline, |to| moddb::fetch_file(entry.image, to))?;
    thumbnail(&bytes).map_err(|e| log::info!("{}: {e}", path.display())).ok()
}

/// ModDB's images are thumbnails. Bound decoding before expanding pixels or creating a
/// texture, including when an old or replaced cache file bypasses the download size limit.
fn thumbnail(bytes: &[u8]) -> image::ImageResult<egui::ColorImage> {
    bounded_picture(bytes, 620, 2000)
}

fn bounded_picture(bytes: &[u8], width: u32, height: u32) -> image::ImageResult<egui::ColorImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 << 20);
    reader.limits(limits);
    let image = reader.decode()?.thumbnail(width, height).into_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dropped_release_of_a_catalogue_mod_installs_as_that_mod() {
        let dir = std::env::temp_dir().join(format!("ta-dropped-release-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let entry = catalogue::MODS.iter().find(|entry| entry.file.is_some()).unwrap();
        let bytes = entry.file.unwrap().bytes;
        let path = dir.join("renamed download.zip");
        std::fs::File::create(&path).unwrap().set_len(bytes).unwrap();
        assert!(matches!(Source::package(&path), Some(Source::Catalogue(found, Some(_))) if found.id == entry.id),
            "the release's exact size names the mod: the installer checks its digest");
        std::fs::File::create(&path).unwrap().set_len(bytes + 1).unwrap();
        assert!(matches!(Source::package(&path), Some(Source::Package { .. })), "any other package is a mod of its own");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ta_launcher_getmods_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wait(installer: &mut Installer, id: &str) -> Stage {
        let until = Instant::now() + Duration::from_secs(30);
        while Instant::now() < until {
            installer.poll();
            if let Some(stage) = installer.stages.get(id).filter(|s| !s.active()) {
                return stage.clone();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("{id} never finished");
    }

    fn pinned_file(bytes: &[u8]) -> catalogue::File {
        use sha2::Digest;
        catalogue::File {
            name: "test.zip", version: "test", bytes: bytes.len() as u64,
            md5: "", sha256: Box::leak(format!("{:x}", sha2::Sha256::digest(bytes)).into_boxed_str()),
            download: catalogue::Download::Direct(&["https://example.invalid/test.zip"]),
            ..catalogue::get("totalmayhem").unwrap().file.unwrap()
        }
    }

    fn website_entry(file: catalogue::File) -> &'static Entry {
        Box::leak(Box::new(Entry {
            id: "verifiedtest", name: "Verified test", image: "", file: Some(file),
            source: catalogue::Provider::Website { url: "https://example.invalid/mod", name: "Test site" },
            project: None,
            ..*catalogue::get("totalmayhem").unwrap()
        }))
    }

    /// A small stored ZIP containing one loose TA unit, with no external test assets.
    fn unit_zip() -> Vec<u8> {
        let name = b"units/TEST.FBI";
        let content = b"[UNITINFO]{UnitName=TEST;}";
        let mut crc = !0u32;
        for byte in content {
            crc ^= *byte as u32;
            for _ in 0..8 { crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1)); }
        }
        crc = !crc;
        let mut out = Vec::new();
        let word = |out: &mut Vec<u8>, n: u16| out.extend_from_slice(&n.to_le_bytes());
        let long = |out: &mut Vec<u8>, n: u32| out.extend_from_slice(&n.to_le_bytes());
        long(&mut out, 0x0403_4b50);
        for n in [20, 0, 0, 0, 0] { word(&mut out, n); }
        long(&mut out, crc); long(&mut out, content.len() as u32); long(&mut out, content.len() as u32);
        word(&mut out, name.len() as u16); word(&mut out, 0);
        out.extend_from_slice(name); out.extend_from_slice(content);
        let central = out.len() as u32;
        long(&mut out, 0x0201_4b50);
        for n in [20, 20, 0, 0, 0, 0] { word(&mut out, n); }
        long(&mut out, crc); long(&mut out, content.len() as u32); long(&mut out, content.len() as u32);
        for n in [name.len() as u16, 0, 0, 0, 0] { word(&mut out, n); }
        long(&mut out, 0); long(&mut out, 0); out.extend_from_slice(name);
        let size = out.len() as u32 - central;
        long(&mut out, 0x0605_4b50);
        for n in [0, 0, 1, 1] { word(&mut out, n); }
        long(&mut out, size); long(&mut out, central); word(&mut out, 0);
        out
    }

    #[test]
    fn final_frontier_rejects_automatic_install_without_network_or_disk_work() {
        let dir = temp_dir("manual_only");
        let entry = catalogue::get("finalfrontier").unwrap();
        let file = entry.file.unwrap();
        let mods = dir.join("mods");
        let cache = dir.join("cache");
        let mut installer = Installer::new(mods.clone(), cache.clone());
        installer.install(entry, None);
        assert!(!installer.busy(), "manual-only requests never enter the install queue");
        assert!(matches!(installer.stages.get(entry.id), Some(Stage::Failed(why)) if why.contains("Have the file?")));
        assert!(installer.finished.is_empty());
        let stages = std::cell::RefCell::new(Vec::new());
        let send = |stage| stages.borrow_mut().push(stage);
        let running = Mutex::default();
        let result = install(&Source::Catalogue(entry, None), &mods, &cache, &send, &|| false, &running, &AtomicBool::new(false));
        assert!(matches!(result, Err(Failure::Error(why)) if why.contains("Have the file?")));
        let result = download(entry, &file, &cache, &send, &|| false, &running);
        assert!(matches!(result, Err(Failure::Error(why)) if why.contains("Have the file?")));
        assert!(stages.borrow().is_empty(), "no connecting, downloading or unpacking progress may start");
        assert!(running.lock().unwrap().is_none());
        assert!(!cache.exists() && !mods.exists(), "no download cache or install workspace is created");
        installer.shutdown();
        drop(installer);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn final_frontier_banner_loads_offline_without_a_cache() {
        let dir = temp_dir("bundled_frontier");
        let cache = dir.join("missing-cache");
        let entry = catalogue::get("finalfrontier").unwrap();
        let image = picture(entry, &cache, true).expect("the bundled banner needs no network or cache");
        assert!(image.width() >= 620 && image.width() <= 1600);
        assert!(image.height() > 0 && image.height() <= 1000);
        assert!(!cache.exists(), "bundled art never writes into the remote cache");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn direct_mirrors_retry_changed_files_and_keep_only_verified_bytes() {
        let dir = temp_dir("direct_mirrors");
        let bytes = b"verified bytes";
        let file = pinned_file(bytes);
        let bad = dir.join("bad.zip");
        let good = dir.join("good.zip");
        std::fs::write(&bad, b"different data").unwrap();
        std::fs::write(&good, bytes).unwrap();
        let urls = [url::Url::from_file_path(&bad).unwrap(), url::Url::from_file_path(&good).unwrap()];
        let attempts = std::cell::Cell::new(0);
        let running = Mutex::default();
        let downloaded = download_sources(&file, &[urls[0].as_str(), urls[1].as_str()], "", crate::http::Policy::Update { local_files: true },
            &dir.join("cache"), &|stage| if stage == Stage::Connecting { attempts.set(attempts.get() + 1); }, &|| false, &running).unwrap();
        assert_eq!(attempts.get(), 2);
        assert_eq!(std::fs::read(downloaded).unwrap(), bytes);
        assert!(!dir.join("cache/downloads/test.zip.part").exists());
        assert!(running.lock().unwrap().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn direct_downloads_require_https_and_explain_manual_recovery() {
        let dir = temp_dir("direct_protocol");
        let source = dir.join("source.zip");
        std::fs::write(&source, b"test").unwrap();
        let url = Box::leak(url::Url::from_file_path(&source).unwrap().to_string().into_boxed_str());
        let file = catalogue::File { download: catalogue::Download::Direct(Box::leak(Box::new([url as &str, "http://example.invalid/test.zip"]))), ..pinned_file(b"test") };
        let result = download(website_entry(file), &file, &dir.join("cache"), &|_| {}, &|| false, &Mutex::default());
        assert!(matches!(result, Err(Failure::Error(error)) if error.contains("Have the file?")));
        assert!(!dir.join("cache").exists(), "unsupported URLs must not read or write download files");
        assert_eq!(std::fs::read(source).unwrap(), b"test");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn existing_moddb_checksums_still_verify_and_can_be_cancelled() {
        let dir = temp_dir("legacy_digest");
        let source = dir.join("source.zip");
        std::fs::write(&source, b"The quick brown fox jumps over the lazy dog").unwrap();
        let file = catalogue::File { bytes: 43, sha256: "", md5: "9e107d9d372bb6826bd81d3542a419d6", ..catalogue::get("totalmayhem").unwrap().file.unwrap() };
        verify_file(&source, &file, &|| false).unwrap();
        assert!(matches!(verify_file(&source, &file, &|| true), Err(Failure::Cancelled)));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cancelled_direct_verification_does_not_try_another_mirror_or_keep_bytes() {
        let dir = temp_dir("cancel_direct");
        let bytes = b"verified bytes";
        let source = dir.join("source.zip");
        std::fs::write(&source, bytes).unwrap();
        let url = url::Url::from_file_path(&source).unwrap();
        let stop = std::cell::Cell::new(false);
        let attempts = std::cell::Cell::new(0);
        let running = Mutex::default();
        let result = download_sources(&pinned_file(bytes), &[url.as_str(), url.as_str()], "", crate::http::Policy::Update { local_files: true },
            &dir.join("cache"), &|stage| {
                if stage == Stage::Connecting { attempts.set(attempts.get() + 1); }
                if stage == Stage::Checking { stop.set(true); }
            }, &|| stop.get(), &running);
        assert!(matches!(result, Err(Failure::Cancelled)));
        assert_eq!(attempts.get(), 1);
        assert!(running.lock().unwrap().is_none());
        assert_eq!(std::fs::read_dir(dir.join("cache/downloads")).unwrap().count(), 0);
        assert_eq!(std::fs::read(source).unwrap(), bytes);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn manual_catalogue_import_verifies_before_replacing_the_previous_install() {
        let dir = temp_dir("verified_import");
        let bytes = unit_zip();
        let entry = website_entry(catalogue::File { download: catalogue::Download::ManualOnly, ..pinned_file(&bytes) });
        let source = dir.join("test.zip");
        let old = dir.join("mods/verifiedtest");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("canary"), b"previous install").unwrap();
        let mut changed = bytes.clone();
        *changed.last_mut().unwrap() ^= 1;
        std::fs::write(&source, &changed).unwrap();
        let result = install(&Source::Catalogue(entry, Some(source.clone())), &dir.join("mods"), &dir.join("cache"), &|_| {}, &|| false, &Mutex::default(), &AtomicBool::new(false));
        assert!(matches!(result, Err(Failure::Error(_))));
        assert_eq!(std::fs::read(old.join("canary")).unwrap(), b"previous install");
        assert_eq!(std::fs::read(&source).unwrap(), changed);
        assert_eq!(std::fs::read_dir(dir.join("cache/catalogue")).unwrap().count(), 0);
        std::fs::write(&source, &bytes).unwrap();
        if ta_vfs::package::Unpacker::find().is_some() {
            let result = install(&Source::Catalogue(entry, Some(source.clone())), &dir.join("mods"), &dir.join("cache"), &|_| {}, &|| false, &Mutex::default(), &AtomicBool::new(false)).unwrap();
            assert_eq!(result, (1, 0, "verifiedtest".into()));
            assert_eq!(std::fs::read(old.join("units/TEST.FBI")).unwrap(), b"[UNITINFO]{UnitName=TEST;}");
            assert!(!old.join("canary").exists());
            assert_eq!(std::fs::read(&source).unwrap(), bytes);
            assert_eq!(std::fs::read_dir(dir.join("cache/catalogue")).unwrap().count(), 0);
            let manifest: ta_vfs::mods::ModManifest = toml::from_str(&std::fs::read_to_string(old.join("mod.toml")).unwrap()).unwrap();
            assert_eq!(manifest.source, "https://example.invalid/mod");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn project_only_releases_never_queue_a_download_or_offer_a_moddb_update() {
        let dir = temp_dir("project_only");
        let entry = catalogue::get("escalation").unwrap();
        let cache = dir.join("cache");
        let mods = dir.join("mods");
        let mut installer = Installer::new(mods.clone(), cache.clone());
        installer.install(entry, None);
        assert!(!installer.busy());
        assert!(matches!(installer.stages.get(entry.id), Some(Stage::Failed(why)) if why.contains("Have the file?")));
        assert!(matches!(install(&Source::Catalogue(entry, None), &mods, &cache, &|_| panic!("no work before a local file is selected"), &|| false, &Mutex::default(), &AtomicBool::new(false)), Err(Failure::Error(_))));
        assert!(!cache.exists() && !mods.exists());
        let mut media = Media::new();
        media.files.insert(entry.id, vec![
            FeedItem { title: "Old ModDB update".into(), link: "https://www.moddb.com/mods/total-annihilation-escalation/downloads/taesc-old".into(), date: String::new(), image: None },
            FeedItem { title: "Gold 9.4.0".into(), link: "https://www.moddb.com/mods/total-annihilation-escalation/downloads/taesc-gold-9-4-0".into(), date: String::new(), image: None },
        ]);
        assert!(media.newer(entry).is_none(), "cached ModDB feeds are not the project's current releases");
        installer.shutdown();
        drop(installer);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn project_import_uses_safe_staging_and_does_not_claim_a_pinned_release() {
        if ta_vfs::package::Unpacker::find().is_none() { return; }
        let dir = temp_dir("project_import");
        // Unpinned project imports remain supported independently of the current
        // Escalation release, which now has a verified manual archive.
        let entry = Box::leak(Box::new(Entry { file: None, fit: catalogue::Fit::Unverified, ..*catalogue::get("escalation").unwrap() }));
        let old = dir.join("mods/escalation");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("canary"), b"previous install").unwrap();
        let source = dir.join("user-chosen.zip");
        std::fs::write(&source, b"not an archive").unwrap();
        let job = Source::Catalogue(entry, Some(source.clone()));
        let install = |send: &dyn Fn(Stage), cancelled: &dyn Fn() -> bool| super::install(&job, &dir.join("mods"), &dir.join("cache"), send, cancelled, &Mutex::default(), &AtomicBool::new(false));
        assert!(install(&|_| {}, &|| false).is_err());
        assert_eq!(std::fs::read(old.join("canary")).unwrap(), b"previous install");
        assert_eq!(std::fs::read(&source).unwrap(), b"not an archive");
        let bytes = unit_zip();
        std::fs::write(&source, &bytes).unwrap();
        assert!(matches!(install(&|_| panic!("cancelled before work"), &|| true), Err(Failure::Cancelled)));
        assert!(old.join("canary").exists());
        let result = install(&|stage| assert!(!matches!(stage, Stage::Checking | Stage::Connecting | Stage::Downloading { .. })), &|| false).unwrap();
        assert_eq!(result, (1, 0, "escalation".into()));
        assert!(!old.join("canary").exists());
        assert!(old.join("units/TEST.FBI").is_file());
        let manifest: ta_vfs::mods::ModManifest = toml::from_str(&std::fs::read_to_string(old.join("mod.toml")).unwrap()).unwrap();
        assert_eq!((manifest.id.as_str(), manifest.version.as_str()), (entry.id, "unverified"));
        assert_eq!(manifest.source, "https://taesc.tauniverse.com/?p=downloads");
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(dir.join("cache/catalogue")).unwrap().count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn install_workspaces_are_exclusive_and_cleanup_only_their_own_files() {
        let dir = temp_dir("private_workspaces");
        let one = InstallWorkspace::create(&dir).unwrap();
        let two = InstallWorkspace::create(&dir).unwrap();
        assert_ne!(one.0, two.0);
        std::fs::write(two.0.join("canary"), b"another install").unwrap();
        drop(one);
        assert_eq!(std::fs::read(two.0.join("canary")).unwrap(), b"another install");
        drop(two);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn lost_installer_worker_fails_active_jobs_without_losing_completed_results() {
        let (requests, jobs) = channel();
        let (report, updates) = channel();
        drop(jobs);
        let mut installer = Installer { requests, updates, cancelled: Arc::default(), current: Arc::default(),
            stopped: Arc::default(), working: Arc::default(), game: Arc::default(), stages: BTreeMap::new(), packages: BTreeMap::new(), finished: Vec::new(), failed: Vec::new() };
        installer.stages.insert("done".into(), Stage::Unpacking);
        installer.stages.insert("waiting".into(), Stage::Queued);
        installer.stages.insert("working".into(), Stage::Checking);
        let done = Stage::Done { units: 1, maps: 0, id: "done".into() };
        report.send(("done".into(), done.clone())).unwrap();
        drop(report);
        assert!(installer.poll());
        assert_eq!(installer.stages["done"], done);
        assert_eq!(installer.finished, ["done"]);
        for id in ["waiting", "working"] {
            assert!(matches!(&installer.stages[id], Stage::Failed(error) if error.contains("Restart the launcher")));
        }
        assert!(!installer.busy(), "worker failure must release profile/update busy gates");
        assert!(!installer.poll(), "disconnection should not keep changing terminal states");
        installer.start(Source::Package { path: "retry.zip".into(), id: "retry".into(), name: "Retry".into() });
        assert!(matches!(&installer.stages["retry"], Stage::Failed(_)), "failed queue sends are immediately actionable");
        assert!(!installer.busy());
    }

    #[test]
    fn an_installed_mod_is_not_replaced_while_the_game_runs() {
        let dir = temp_dir("game_holds");
        let mods = dir.join("mods");
        let cache = dir.join("cache");
        let source = dir.join("incoming");
        std::fs::create_dir_all(source.join("units")).unwrap();
        std::fs::write(source.join("units/test.fbi"), "[UNITINFO]{UnitName=TEST;}").unwrap();
        let package = Source::Package { path: source.clone(), id: "myunits".into(), name: "My units".into() };
        let running = AtomicBool::new(true);
        // Not installed yet: it installs, the game running or not.
        assert!(install(&package, &mods, &cache, &|_| {}, &|| false, &Mutex::default(), &running).is_ok());
        // Installed: not replaced while the game runs, and said why.
        let held = install(&package, &mods, &cache, &|_| {}, &|| false, &Mutex::default(), &running);
        assert!(matches!(&held, Err(Failure::Error(why)) if why.contains("Close the game first")), "{held:?}");
        assert!(mods.join("myunits/units/test.fbi").is_file(), "the installed copy is left as it was");
        running.store(false, Ordering::Release);
        assert!(install(&package, &mods, &cache, &|_| {}, &|| false, &Mutex::default(), &running).is_ok(), "once it closed");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn working_folders_left_behind_by_other_runs_are_swept() {
        let dir = temp_dir("sweep");
        let ours = dir.join(format!("install-{}-0", std::process::id()));
        let theirs = dir.join(format!("install-{}-3", std::process::id().wrapping_add(1)));
        let unrelated = dir.join("installer-notes");
        for folder in [&ours, &theirs, &unrelated] {
            std::fs::create_dir_all(folder.join("downloads")).unwrap();
            std::fs::write(folder.join("downloads/mod.zip.part"), b"partial").unwrap();
        }
        let file = dir.join("install-1-1");
        std::fs::write(&file, b"a file, not a working folder").unwrap();
        // (Too young: left, a setup may be reading it.)
        sweep_stale(&dir, "install", Duration::from_secs(600));
        assert!(theirs.is_dir());
        sweep_stale(&dir, "install", Duration::ZERO);
        assert!(!theirs.exists(), "another run's folder goes");
        assert!(ours.is_dir(), "this run's stays");
        assert!(unrelated.is_dir() && file.is_file(), "nothing else is touched");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn shutdown_cancels_a_staged_install_waiting_to_publish() {
        let dir = temp_dir("shutdown_publication");
        let mods = dir.join("mods");
        let source = dir.join("source");
        std::fs::create_dir_all(source.join("units")).unwrap();
        std::fs::write(source.join("units/test.fbi"), b"new version").unwrap();
        std::fs::create_dir_all(mods.join("testmod")).unwrap();
        std::fs::write(mods.join("testmod/canary"), b"previous version").unwrap();
        let plan = ta_vfs::package::plan(&source).unwrap();
        let manifest = ta_vfs::mods::ModManifest { id: "testmod".into(), name: "Test".into(), version: "1".into(), description: String::new(), authors: Vec::new(), depends: Vec::new(), load_after: Vec::new(), source: String::new(), rename: BTreeMap::new(), replaces: Vec::new() };
        let installer = Installer::new(mods.clone(), dir.join("cache"));
        let gate = installer.current.clone();
        let stopped = installer.stopped.clone();
        let held = gate.lock().unwrap();
        let worker_gate = gate.clone();
        let worker_stopped = stopped.clone();
        let worker_mods = mods.clone();
        let worker = std::thread::spawn(move || publish_install(&plan, &manifest, &worker_mods,
            &|| worker_stopped.load(Ordering::Relaxed), &worker_gate, &AtomicBool::new(false)));
        let until = Instant::now() + Duration::from_secs(5);
        while !mods.join(".testmod.installing/mod.toml").is_file() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(mods.join(".testmod.installing/mod.toml").is_file(), "staging must not hold the publication gate");
        let (closed_tx, closed_rx) = channel();
        let closer = std::thread::spawn(move || { installer.shutdown(); closed_tx.send(()).unwrap(); });
        let until = Instant::now() + Duration::from_secs(5);
        while !stopped.load(Ordering::Relaxed) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(stopped.load(Ordering::Relaxed));
        assert!(matches!(closed_rx.try_recv(), Err(TryRecvError::Empty)), "shutdown waits for the publication gate");
        drop(held);
        closed_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(worker.join().unwrap(), Err(Failure::Cancelled)), "publication cannot start after shutdown");
        closer.join().unwrap();
        assert_eq!(std::fs::read(mods.join("testmod/canary")).unwrap(), b"previous version");
        assert!(!mods.join(".testmod.installing").exists());
        assert!(!mods.join(".testmod.old").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn generic_import_never_deletes_another_instances_staging_directory() {
        let dir = temp_dir("generic_private_workspace");
        let cache = dir.join("cache");
        let previous = cache.join("unpack/myunits");
        std::fs::create_dir_all(&previous).unwrap();
        std::fs::write(previous.join("canary"), b"another instance or interrupted import").unwrap();
        let source = dir.join("incoming");
        std::fs::create_dir_all(source.join("units")).unwrap();
        std::fs::write(source.join("units/test.fbi"), "[UNITINFO]{UnitName=TEST;}").unwrap();
        let package = Source::Package { path: source.clone(), id: "myunits".into(), name: "My units".into() };
        let mods = dir.join("mods");
        assert_eq!(install(&package, &mods, &cache, &|_| {}, &|| false, &Mutex::default(), &AtomicBool::new(false)).unwrap(), (1, 0, "myunits".into()));
        assert_eq!(std::fs::read(previous.join("canary")).unwrap(), b"another instance or interrupted import");
        assert!(mods.join("myunits/units/test.fbi").is_file());
        assert!(source.join("units/test.fbi").is_file());
        assert_eq!(std::fs::read_dir(cache.join("unpack")).unwrap().count(), 1, "only this invocation's private workspace is removed");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn website_releases_ignore_moddb_feed_updates() {
        let entry = website_entry(pinned_file(b"test"));
        assert!(entry.moddb().is_none());
        let mut media = Media::new();
        media.files.insert(entry.id, vec![FeedItem { title: "Unrelated update".into(), link: "https://www.moddb.com/mods/unrelated/downloads/other".into(), date: String::new(), image: None }]);
        assert!(media.newer(entry).is_none());
        let frontier = catalogue::get("finalfrontier").expect("The Final Frontier is in the catalogue");
        assert!(frontier.file.is_some() && frontier.moddb().is_none());
        assert!(!catalogue::MODS.iter().filter(|e| e.file.is_some() && e.moddb().is_some()).any(|e| e.id == frontier.id));
    }

    #[test]
    fn a_package_is_named_after_its_file() {
        match Source::package(&Path::new("Downloads").join("TA_Frenzy_1.41.rar")) {
            Some(Source::Package { id, name, .. }) => assert_eq!((id.as_str(), name.as_str()), ("tafrenzy141", "TA_Frenzy_1.41")),
            other => panic!("{other:?}"),
        }
        // (Names in other scripts keep their letters.)
        match Source::package(&Path::new("Downloads").join("Моды 2.zip")) {
            Some(Source::Package { id, name, .. }) => assert_eq!((id.as_str(), name.as_str()), ("моды2", "Моды 2")),
            other => panic!("{other:?}"),
        }
        assert!(Source::package(Path::new("___.zip")).is_none(), "nothing to make an id of");
        // A classic archive is the mod the game reads it as (its id in lower case, any letter:
        // as the settings keep it).
        match Source::package(&Path::new("Downloads").join("Übermacht Karten.UFO")) {
            Some(Source::Classic { id, .. }) => assert_eq!(id, "übermacht karten"),
            other => panic!("{other:?}"),
        }
    }

    /// A folder dropped on the window is installed as a mod (copied: the folder stays), its
    /// units counted; one without TA content fails with the package's reason.
    #[test]
    fn a_dropped_folder_is_installed_and_kept() {
        let dir = temp_dir("folder");
        let package = dir.join("My Units");
        std::fs::create_dir_all(package.join("units")).unwrap();
        std::fs::write(package.join("units").join("myunit.fbi"), "[UNITINFO]\n{\nUnitName=MYUNIT;\n}\n").unwrap();
        let empty = dir.join("Holiday Photos");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::write(empty.join("beach.jpg"), b"not a unit").unwrap();
        let mods = dir.join("mods");
        let mut installer = Installer::new(mods.clone(), dir.join("cache"));
        installer.start(Source::package(&package).unwrap());
        assert_eq!(wait(&mut installer, "myunits"), Stage::Done { units: 1, maps: 0, id: "myunits".into() });
        assert!(mods.join("myunits").join("mod.toml").is_file() && mods.join("myunits").join("units").join("myunit.fbi").is_file());
        assert!(package.join("units").join("myunit.fbi").is_file(), "the player's folder stays as it was");
        assert_eq!(installer.finished, ["myunits"]);
        installer.start(Source::package(&empty).unwrap());
        assert!(matches!(wait(&mut installer, "holidayphotos"), Stage::Failed(_)));
        assert!(!mods.join("holidayphotos").exists());
        assert!(!dir.join("cache").join("unpack").join("holidayphotos").exists(), "nothing left in the cache");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_install_never_clears_its_own_source_or_an_invalid_id_target() {
        let dir = temp_dir("staging_source");
        let cache = dir.join("cache");
        let source = cache.join("unpack").join("my_mod");
        std::fs::create_dir_all(source.join("units")).unwrap();
        let content = source.join("units").join("unit.fbi");
        std::fs::write(&content, "[UNITINFO]{UnitName=TEST;}").unwrap();
        let package = Source::Package { path: source.clone(), id: "my_mod".into(), name: "My mod".into() };
        assert!(install(&package, &dir.join("mods"), &cache, &|_| {}, &|| false, &Mutex::new(None), &AtomicBool::new(false)).is_err());
        assert!(content.is_file(), "refusing the install must preserve the dropped source");
        let valuable = dir.join("valuable");
        std::fs::create_dir_all(&valuable).unwrap();
        std::fs::write(valuable.join("keep"), b"kept").unwrap();
        let package = Source::Package { path: source, id: "../../valuable".into(), name: "Invalid".into() };
        assert!(install(&package, &dir.join("mods"), &cache, &|_| {}, &|| false, &Mutex::new(None), &AtomicBool::new(false)).is_err());
        assert_eq!(std::fs::read(valuable.join("keep")).unwrap(), b"kept");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A mod folder already in the mods folder, dropped on the window, is installed: copying it
    /// out and back would leave behind what an install does not take (its author's sources).
    #[test]
    fn a_dropped_folder_already_in_the_mods_folder_is_left_as_it_is() {
        let dir = temp_dir("in_mods");
        let mods = dir.join("mods");
        let work = mods.join("mywork");
        std::fs::create_dir_all(work.join("units")).unwrap();
        std::fs::create_dir_all(work.join("src")).unwrap();
        std::fs::write(work.join("mod.toml"), "id = \"mywork\"\nname = \"My work\"\n").unwrap();
        std::fs::write(work.join("units/unit.fbi"), "[UNITINFO]{UnitName=TEST;}").unwrap();
        std::fs::write(work.join("src/unit.bos"), b"the author's script source").unwrap();
        let source = Source::package(&work).unwrap();
        assert!(install(&source, &mods, &dir.join("cache"), &|_| {}, &|| false, &Mutex::new(None), &AtomicBool::new(false)).is_err());
        assert_eq!(std::fs::read(work.join("src/unit.bos")).unwrap(), b"the author's script source");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A package named like a folder of the mods folder holding another mod (the map the editor
    /// saved as "Islands") goes in beside it: the map is kept.
    #[test]
    fn a_package_never_replaces_another_mods_folder() {
        let dir = temp_dir("beside");
        let mods = dir.join("mods");
        std::fs::create_dir_all(mods.join("islands/maps")).unwrap();
        std::fs::write(mods.join("islands/mod.toml"), "# Written by the TA Reborn map editor.\nid = \"map-islands-0123456789abcdef\"\nname = \"Islands (map)\"\n").unwrap();
        std::fs::write(mods.join("islands/maps/Islands.tnt"), b"the player's map").unwrap();
        let package = dir.join("Islands");
        std::fs::create_dir_all(package.join("units")).unwrap();
        std::fs::write(package.join("units/unit.fbi"), "[UNITINFO]{UnitName=TEST;}").unwrap();
        let source = Source::package(&package).unwrap();
        assert_eq!(source.id(), "islands");
        let (_, _, id) = install(&source, &mods, &dir.join("cache"), &|_| {}, &|| false, &Mutex::new(None), &AtomicBool::new(false)).map_err(|e| format!("{e:?}")).unwrap();
        assert_eq!(id, "islands");
        assert_eq!(std::fs::read(mods.join("islands/maps/Islands.tnt")).unwrap(), b"the player's map");
        let mut ids: Vec<String> = ta_vfs::mods::scan(&mods).0.into_iter().map(|m| m.manifest.id).collect();
        ids.sort();
        assert_eq!(ids, ["islands", "map-islands-0123456789abcdef"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn part_copied_archives_left_behind_are_swept() {
        let dir = temp_dir("imports");
        let ours = dir.join(format!(".import-{}-0.part", std::process::id()));
        let theirs = dir.join(format!(".import-{}-4.part", std::process::id().wrapping_add(1)));
        let archive = dir.join("pack.ufo");
        for file in [&ours, &theirs, &archive] { std::fs::write(file, b"bytes").unwrap(); }
        sweep_stale_imports(&dir, Duration::from_secs(600));
        assert!(theirs.is_file(), "too young: another launcher may be copying it");
        sweep_stale_imports(&dir, Duration::ZERO);
        assert!(!theirs.exists() && ours.is_file() && archive.is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dropped_manifest_reads_are_bounded() {
        let dir = temp_dir("large_manifest");
        std::fs::write(dir.join("mod.toml"), format!("id = \"oversized\"\nname = \"Too large\"\n#{}", "x".repeat(1 << 20))).unwrap();
        assert!(own_manifest(&dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_media_refresh_preserves_the_stale_cache() {
        let dir = temp_dir("stale_media");
        let path = dir.join("picture.png");
        std::fs::write(&path, b"last complete picture").unwrap();
        let bytes = cached(&path, Duration::ZERO, false, |to| {
            std::fs::write(to, b"incomplete replacement").unwrap();
            Err("connection reset".into())
        }).unwrap();
        assert_eq!(bytes, b"last complete picture");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let bytes = cached(&path, Duration::ZERO, false, |to| {
            std::fs::write(to, b"new complete picture").map_err(|e| e.to_string())
        }).unwrap();
        assert_eq!(bytes, b"new complete picture");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn oversized_media_refresh_keeps_the_previous_cache() {
        let dir = temp_dir("oversized_refresh");
        let path = dir.join("picture.png");
        std::fs::write(&path, b"previous picture").unwrap();
        let bytes = cached(&path, Duration::ZERO, false, |to| {
            std::fs::File::create(to).unwrap().set_len((16 << 20) + 1).unwrap();
            Ok(())
        }).unwrap();
        assert_eq!(bytes, b"previous picture");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn thumbnail_decode_and_texture_dimensions_are_bounded() {
        let png = |width, height| {
            let image = image::DynamicImage::new_luma8(width, height);
            let mut bytes = std::io::Cursor::new(Vec::new());
            image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
            bytes.into_inner()
        };
        assert!(thumbnail(&png(4097, 1)).is_err());
        assert!(thumbnail(&png(1, 4097)).is_err());
        let image = thumbnail(&png(2048, 32)).unwrap();
        assert!(image.size[0] <= 620 && image.size[1] <= 2000);
        let dir = temp_dir("oversized_cached_image");
        let path = dir.join("old.png");
        std::fs::File::create(&path).unwrap().set_len((16 << 20) + 1).unwrap();
        assert!(cached(&path, PICTURE_AGE, true, |_| unreachable!()).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn classic_archive_import_preserves_sources_and_failed_replacements() {
        let dir = temp_dir("archive_import");
        let mods = dir.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        let installed = mods.join("test.ufo");
        std::fs::write(&installed, b"original archive").unwrap();
        import_archive(&installed, &mods).unwrap();
        assert_eq!(std::fs::read(&installed).unwrap(), b"original archive");
        let source = dir.join("test.ufo");
        std::fs::write(&source, b"replacement archive").unwrap();
        import_archive(&source, &mods).unwrap();
        assert_eq!(std::fs::read(&installed).unwrap(), b"replacement archive");
        assert_eq!(std::fs::read(&source).unwrap(), b"replacement archive");
        std::fs::remove_file(&source).unwrap();
        assert!(import_archive(&source, &mods).is_err());
        assert_eq!(std::fs::read(&installed).unwrap(), b"replacement archive");
        assert_eq!(std::fs::read_dir(&mods).unwrap().count(), 1, "no incomplete import remains");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn classic_archives_use_the_installer_and_keep_their_original_names() {
        let dir = temp_dir("classic_worker");
        let source = dir.join("My_Map Pack.UFO");
        crate::maps::tests::write_map_archive(&source);
        let original = std::fs::read(&source).unwrap();
        let job = Source::package(&source).unwrap();
        assert!(matches!(&job, Source::Classic { id, name, .. } if id == "my_map pack" && name == "My_Map Pack"));
        let mods = dir.join("mods");
        let mut installer = Installer::new(mods.clone(), dir.join("cache"));
        installer.start(job);
        assert_eq!(wait(&mut installer, "my_map pack"), Stage::Done { units: 0, maps: 0, id: "my_map pack".into() });
        assert_eq!(std::fs::read(mods.join("My_Map Pack.UFO")).unwrap(), original);
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(ta_vfs::mods::scan(&mods).0[0].manifest.id, "my_map pack");
        assert_eq!(installer.finished, ["my_map pack"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cancelling_a_classic_copy_preserves_the_installed_archive_and_removes_staging() {
        let dir = temp_dir("classic_cancel");
        let mods = dir.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        let destination = mods.join("test.ufo");
        let source = dir.join("test.ufo");
        let original = vec![7; 1 << 20];
        std::fs::write(&source, &original).unwrap();
        std::fs::write(&destination, b"previous version").unwrap();
        let checks = std::cell::Cell::new(0);
        let cancelled = || {
            checks.set(checks.get() + 1);
            checks.get() >= 4 // The first two chunks have been copied.
        };
        let result = copy_archive(&source, &mods, &|_, _| {}, &cancelled);
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
        assert_eq!(std::fs::read(&destination).unwrap(), b"previous version");
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::read_dir(&mods).unwrap().count(), 1);
        // Cancellation reported after the final progress update still prevents replacement.
        let stop = std::cell::Cell::new(false);
        let result = copy_archive(&source, &mods, &|got, total| stop.set(got == total), &|| stop.get());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
        assert_eq!(std::fs::read(&destination).unwrap(), b"previous version");
        assert_eq!(std::fs::read_dir(&mods).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cancelling_a_folder_import_stops_within_a_large_asset_and_preserves_both_copies() {
        let dir = temp_dir("folder_cancel");
        let source = dir.join("incoming");
        let cache = dir.join("cache");
        let mods = dir.join("mods");
        std::fs::create_dir_all(source.join("units")).unwrap();
        std::fs::create_dir_all(mods.join("incoming")).unwrap();
        let content = vec![7u8; 2 << 20];
        std::fs::write(source.join("units/test.fbi"), &content).unwrap();
        std::fs::write(mods.join("incoming/canary"), b"previous installed version").unwrap();
        let copied_when_cancelled = std::cell::Cell::new(0);
        let cancelled = || {
            let copied = std::fs::read_dir(cache.join("unpack")).ok().into_iter().flatten().filter_map(Result::ok)
                .filter_map(|entry| std::fs::metadata(entry.path().join("content/units/test.fbi")).ok())
                .map(|metadata| metadata.len()).max().unwrap_or(0);
            if copied > 0 { copied_when_cancelled.set(copied); true } else { false }
        };
        let result = install(&Source::package(&source).unwrap(), &mods, &cache, &|_| {}, &cancelled, &Mutex::default(), &AtomicBool::new(false));
        assert!(matches!(result, Err(Failure::Cancelled)), "cancellation is not a failed install");
        assert!((1..content.len() as u64).contains(&copied_when_cancelled.get()), "stop before copying the remainder of the asset");
        assert_eq!(std::fs::read(source.join("units/test.fbi")).unwrap(), content);
        assert_eq!(std::fs::read(mods.join("incoming/canary")).unwrap(), b"previous installed version");
        assert_eq!(std::fs::read_dir(cache.join("unpack")).unwrap().count(), 0, "private partial copies are cleaned up");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn shutting_down_cancels_queued_classic_imports() {
        let dir = temp_dir("classic_shutdown");
        let source = dir.join("stopped.ufo");
        std::fs::write(&source, b"kept").unwrap();
        let mut installer = Installer::new(dir.join("mods"), dir.join("cache"));
        installer.shutdown();
        installer.start(Source::package(&source).unwrap());
        assert_eq!(wait(&mut installer, "stopped"), Stage::Cancelled);
        assert!(!dir.join("mods").join("stopped.ufo").exists());
        assert_eq!(std::fs::read(&source).unwrap(), b"kept");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn media_wakes_an_idle_ui_and_finishes_without_restarting() {
        let dir = temp_dir("media_wake");
        let entry = catalogue::MODS.iter().find(|e| !e.image.is_empty()).unwrap();
        let pictures = dir.join("pictures");
        std::fs::create_dir_all(&pictures).unwrap();
        let ext = entry.image.rsplit('.').next().unwrap();
        image::RgbImage::new(8, 8).save(pictures.join(format!("{}.{}", entry.id, ext))).unwrap();
        let ctx = egui::Context::default();
        let (tx, rx) = channel();
        ctx.set_request_repaint_callback(move |_| { let _ = tx.send(()); });
        let mut media = Media::new();
        media.poll(&ctx); // Register before starting, then do not poll until woken.
        media.start(dir.clone(), true);
        rx.recv_timeout(Duration::from_secs(5)).expect("worker wakes the idle UI without mouse input");
        let until = Instant::now() + Duration::from_secs(5);
        while media.updates.is_some() && Instant::now() < until {
            media.poll(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(media.pictures.contains_key(entry.id));
        assert!(media.updates.is_none(), "finished workers do not need periodic UI polling");
        media.start(dir.clone(), true);
        assert!(media.updates.is_none(), "opening the page again does not refetch everything");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A mod laid out as one already (its own `mod.toml`) keeps its id and what it needs, and
    /// only its content is copied out of the folder; a folder holding the launcher's own is
    /// not installed at all.
    #[test]
    fn a_dropped_mod_keeps_its_manifest() {
        let dir = temp_dir("manifest");
        let package = dir.join("My Mod v2");
        std::fs::create_dir_all(package.join("units")).unwrap();
        std::fs::write(package.join("units").join("a.fbi"), "[UNITINFO]\n{\n}\n").unwrap();
        std::fs::write(package.join("mod.toml"), "id = \"my_mod\"\nname = \"My Mod\"\nversion = \"2.0\"\ndepends = [\"base_units\"]\n").unwrap();
        std::fs::create_dir_all(package.join("videos")).unwrap();
        std::fs::write(package.join("videos").join("trailer.mp4"), b"not content").unwrap();
        let mods = dir.join("mods");
        let mut installer = Installer::new(mods.clone(), dir.join("cache"));
        let source = Source::package(&package).unwrap();
        assert_eq!(source.id(), "my_mod");
        installer.start(source);
        assert_eq!(wait(&mut installer, "my_mod"), Stage::Done { units: 1, maps: 0, id: "my_mod".into() });
        let manifest: ta_vfs::mods::ModManifest = toml::from_str(&std::fs::read_to_string(mods.join("my_mod").join("mod.toml")).unwrap()).unwrap();
        assert_eq!((manifest.name.as_str(), manifest.version.as_str()), ("My Mod", "2.0"));
        assert_eq!(manifest.depends, ["base_units"]);
        assert!(!mods.join("my_mod").join("videos").exists(), "only the mod's content");
        assert!(package.join("videos").join("trailer.mp4").is_file(), "the player's folder stays as it was");
        // The folder holding the mods and the cache.
        let source = Source::package(&dir).unwrap();
        let id = source.id().to_string();
        installer.start(source);
        assert!(matches!(wait(&mut installer, &id), Stage::Failed(_)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn launcher_review_invalid_manifests_fail_without_replacing_the_installed_mod() {
        for invalid in ["not TOML".to_string(), " ".repeat((1 << 20) + 1), "id = '../escape'\nname = 'Unsafe'".to_string()] {
            let dir = temp_dir("review_invalid_manifest");
            let source = dir.join("testmod");
            std::fs::create_dir_all(source.join("units")).unwrap();
            std::fs::write(source.join("units/a.fbi"), b"new").unwrap();
            std::fs::write(source.join("mod.toml"), &invalid).unwrap();
            let mods = dir.join("mods");
            let installed = mods.join("testmod");
            std::fs::create_dir_all(&installed).unwrap();
            std::fs::write(installed.join("mod.toml"), "id = 'testmod'\nname = 'Previous'\nversion = '1'").unwrap();
            std::fs::write(installed.join("canary"), b"previous").unwrap();
            let mut installer = Installer::new(mods.clone(), dir.join("cache"));
            let job = Source::package(&source).unwrap();
            let id = job.id().to_string();
            installer.start(job);
            assert!(matches!(wait(&mut installer, &id), Stage::Failed(_)));
            assert_eq!(std::fs::read(installed.join("canary")).unwrap(), b"previous");
            assert_eq!(std::fs::read_to_string(source.join("mod.toml")).unwrap(), invalid);
            assert_eq!(ta_vfs::mods::scan(&mods).0.len(), 1);
            drop(installer);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn planner_review_imports_legacy_content_with_a_broken_optional_manifest() {
        let dir = temp_dir("planner_review_optional_manifest");
        let source = dir.join("legacy");
        std::fs::create_dir_all(source.join("units")).unwrap();
        std::fs::write(source.join("units/tank.fbi"), b"[UNITINFO]{UnitName=TANK;}").unwrap();
        std::fs::create_dir_all(source.join("examples")).unwrap();
        std::fs::write(source.join("examples/mod.toml"), "not TOML").unwrap();
        let mods = dir.join("mods");
        let mut installer = Installer::new(mods.clone(), dir.join("cache"));
        installer.start(Source::package(&source).unwrap());
        assert_eq!(wait(&mut installer, "legacy"), Stage::Done { units: 1, maps: 0, id: "legacy".into() });
        assert_eq!(std::fs::read(mods.join("legacy/units/tank.fbi")).unwrap(), b"[UNITINFO]{UnitName=TANK;}");
        assert!(!mods.join("legacy/examples").exists());
        assert_eq!(std::fs::read_to_string(source.join("examples/mod.toml")).unwrap(), "not TOML");
        drop(installer);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn launcher_review_safe_dotted_and_unicode_manifest_ids_keep_their_identity() {
        let dir = temp_dir("review_manifest_id");
        let source = dir.join("download folder");
        std::fs::create_dir_all(source.join("units")).unwrap();
        std::fs::write(source.join("units/a.fbi"), b"unit").unwrap();
        std::fs::write(source.join("mod.toml"), "id = '作者.mod v2'\nname = 'My Mod'\nversion = '2'\ndepends = ['base.units']").unwrap();
        let mods = dir.join("mods");
        let mut installer = Installer::new(mods.clone(), dir.join("cache"));
        let job = Source::package(&source).unwrap();
        assert_eq!(job.id(), "作者.mod v2");
        installer.start(job);
        assert!(matches!(wait(&mut installer, "作者.mod v2"), Stage::Done { .. }));
        let info = ta_vfs::mods::scan(&mods).0.pop().unwrap();
        assert_eq!(info.manifest.id, "作者.mod v2");
        assert_eq!(info.manifest.depends, ["base.units"]);
        assert_eq!(info.manifest.version, "2");
        drop(installer);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn launcher_review_classic_archive_replaces_the_existing_id_across_extensions() {
        let dir = temp_dir("review_classic_id");
        let mods = dir.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        let installed = mods.join("mymod.gp3");
        std::fs::write(&installed, b"previous").unwrap();
        let source = dir.join("MyMod.UFO");
        crate::maps::tests::write_map_archive(&source);
        let bytes = std::fs::read(&source).unwrap();
        let mut installer = Installer::new(mods.clone(), dir.join("cache"));
        installer.start(Source::package(&source).unwrap());
        assert!(matches!(wait(&mut installer, "mymod"), Stage::Done { .. }));
        assert_eq!(std::fs::read(&installed).unwrap(), bytes);
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert!(!mods.join("MyMod.UFO").exists());
        assert_eq!(ta_vfs::mods::scan(&mods).0.len(), 1);
        drop(installer);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn launcher_review_classic_archive_cannot_shadow_an_installed_directory() {
        let dir = temp_dir("review_classic_directory");
        let mods = dir.join("mods");
        let installed = mods.join("mymod");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("mod.toml"), "id = 'mymod'\nname = 'Installed'").unwrap();
        std::fs::write(installed.join("canary"), b"previous").unwrap();
        let source = dir.join("MyMod.UFO");
        crate::maps::tests::write_map_archive(&source);
        let mut installer = Installer::new(mods.clone(), dir.join("cache"));
        installer.start(Source::package(&source).unwrap());
        assert!(matches!(wait(&mut installer, "mymod"), Stage::Failed(_)));
        assert_eq!(std::fs::read(installed.join("canary")).unwrap(), b"previous");
        assert!(!mods.join("MyMod.UFO").exists());
        drop(installer);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn launcher_review_corrupt_classic_archive_cannot_replace_a_working_copy() {
        let dir = temp_dir("review_corrupt_archive");
        let mods = dir.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        let installed = mods.join("mymod.ufo");
        crate::maps::tests::write_map_archive(&installed);
        let before = std::fs::read(&installed).unwrap();
        let source = dir.join("mymod.ufo");
        std::fs::write(&source, b"not an archive").unwrap();
        let mut installer = Installer::new(mods.clone(), dir.join("cache"));
        installer.start(Source::package(&source).unwrap());
        assert!(matches!(wait(&mut installer, "mymod"), Stage::Failed(_)));
        assert_eq!(std::fs::read(installed).unwrap(), before);
        assert_eq!(std::fs::read(source).unwrap(), b"not an archive");
        drop(installer);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn launcher_review_classic_archive_identity_updates_wait_for_the_running_game() {
        let dir = temp_dir("review_classic_running");
        let mods = dir.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        let installed = mods.join("mymod.gp3");
        crate::maps::tests::write_map_archive(&installed);
        let before = std::fs::read(&installed).unwrap();
        let source = dir.join("MyMod.UFO");
        crate::maps::tests::write_map_archive(&source);
        let mut installer = Installer::new(mods.clone(), dir.join("cache"));
        installer.set_game_running(true);
        installer.start(Source::package(&source).unwrap());
        assert!(matches!(wait(&mut installer, "mymod"), Stage::Failed(message) if message.contains("Close the game first")));
        assert_eq!(std::fs::read(&installed).unwrap(), before);
        assert!(!mods.join("MyMod.UFO").exists());
        installer.set_game_running(false);
        installer.start(Source::package(&source).unwrap());
        assert!(matches!(wait(&mut installer, "mymod"), Stage::Done { .. }));
        assert_eq!(ta_vfs::mods::scan(&mods).0.len(), 1);
        drop(installer);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A cancel that comes while the package is being unpacked is honoured: nothing is installed,
    /// and nothing is left in the cache.
    #[test]
    fn a_cancel_while_unpacking_installs_nothing() {
        let dir = temp_dir("cancel_unpacking");
        let package = dir.join("Late");
        std::fs::create_dir_all(package.join("units")).unwrap();
        std::fs::write(package.join("units").join("a.fbi"), "[UNITINFO]
{
}
").unwrap();
        let (mods, cache) = (dir.join("mods"), dir.join("cache"));
        // (Not cancelled when it starts; cancelled by the time it is unpacked.)
        let asked = std::cell::Cell::new(0);
        let cancelled = || {
            asked.set(asked.get() + 1);
            asked.get() > 1
        };
        let result = install(&Source::package(&package).unwrap(), &mods, &cache, &|_| {}, &cancelled, &Mutex::new(None), &AtomicBool::new(false));
        assert!(matches!(result, Err(Failure::Cancelled)));
        assert!(!mods.join("late").exists(), "nothing installed");
        assert!(!cache.join("unpack").join("late").exists(), "nothing left in the cache");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Cancelling an install before it starts leaves it as if never asked for (no failure).
    #[test]
    fn a_cancelled_install_is_no_failure() {
        let dir = temp_dir("cancel");
        let package = dir.join("Slow");
        std::fs::create_dir_all(package.join("units")).unwrap();
        std::fs::write(package.join("units").join("a.fbi"), "[UNITINFO]\n{\n}\n").unwrap();
        let mut installer = Installer::new(dir.join("mods"), dir.join("cache"));
        // (The worker takes the second job only once the first is through.)
        installer.start(Source::package(&package).unwrap());
        let second = dir.join("Second");
        std::fs::create_dir_all(second.join("units")).unwrap();
        std::fs::write(second.join("units").join("b.fbi"), "[UNITINFO]\n{\n}\n").unwrap();
        installer.start(Source::package(&second).unwrap());
        installer.cancel("second");
        wait(&mut installer, "slow");
        assert_eq!(wait(&mut installer, "second"), Stage::Cancelled);
        assert!(!dir.join("mods").join("second").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod game_folder_tests {
    use super::*;

    /// A mod its own installer put in the original game's folder is added as a mod of its own:
    /// its archive in the mod's folder (linked: the same file), the game's folder left as it is,
    /// the folders of its own in its manifest, and the catalogue's name, team and page for a mod
    /// the catalogue has (which release it is, no one can say).
    #[test]
    fn a_mod_in_the_game_folder_is_added_with_its_files_left_there() {
        let dir = std::env::temp_dir().join(format!("ta_game_folder_install_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let game = dir.join("game");
        ta_vfs::test_archive::write(&game.join("totala1.hpi"), &[("units/armcom.fbi", b"base")]);
        ta_vfs::test_archive::write(&game.join("mayhem.gp3"), &[("units/armcom.fbi", b"mayhem"), ("guim/armcom1.gui", b"mayhem"), ("weaponm/weapons.tdf", b"mayhem")]);
        let found = ta_vfs::mods::game_folder_mods(&game);
        assert_eq!(found.len(), 1, "TA's own archive is no mod's");
        let (mods, cache) = (dir.join("mods"), dir.join("cache"));
        let result = install(&Source::GameFolder(found[0].clone()), &mods, &cache, &|_| {}, &|| false, &Mutex::default(), &AtomicBool::new(false));
        assert_eq!(result.unwrap(), (1, 0, "totalmayhem".to_string()));
        let installed = ta_vfs::mods::read_mod(&mods.join("totalmayhem")).unwrap().unwrap();
        assert_eq!((installed.manifest.name.as_str(), installed.manifest.version.as_str()), ("Total Mayhem", ""));
        assert_eq!(installed.manifest.source, "https://mayhem.tauniverse.com/totalm.htm");
        assert_eq!(installed.manifest.rename, BTreeMap::from([("guim".to_string(), "guis".to_string()), ("weaponm".to_string(), "weapons".to_string())]));
        assert!(game.join("mayhem.gp3").is_file(), "the game's folder is left as it is");
        assert_eq!(std::fs::read(mods.join("totalmayhem").join("mayhem.gp3")).unwrap(), std::fs::read(game.join("mayhem.gp3")).unwrap());
        // Played, it is the mod: its menus and weapons where TA reads them.
        let mounted = ta_vfs::mods::mount(Some(&game), None, &[&installed], &|m| m.manifest.id.clone());
        assert_eq!(mounted.vfs.read("guis/armcom1.gui").unwrap(), b"mayhem");
        assert_eq!(mounted.vfs.read("units/armcom.fbi").unwrap(), b"mayhem");
        // One that has gone from the game's folder meanwhile is not installed.
        std::fs::remove_file(game.join("mayhem.gp3")).unwrap();
        assert!(install(&Source::GameFolder(found[0].clone()), &dir.join("mods2"), &cache, &|_| {}, &|| false, &Mutex::default(), &AtomicBool::new(false)).is_err());
        assert!(!dir.join("mods2").join("totalmayhem").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
