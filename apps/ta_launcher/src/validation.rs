//! Serialized, cancellable mod checks. Disk access and parsing stay off the UI thread.
//! Reports describe one mod with its required dependencies over the original game and
//! Reborn content; they never certify gameplay or an arbitrary combination of mods.
//!
//! Each check compares the mod's content with the same content without it (the original
//! game, Reborn's own and its dependencies: the baseline, checked once and kept), so what the
//! mod breaks in inherited files counts against it and what was broken before does not.
//! Only the files the game reads are fingerprinted, and their digests are kept between
//! launches by their size and times; an explicit rescan or recheck reads them again.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc, atomic::{AtomicBool, Ordering}};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ta_data::validation::{self, Issue, Options, Report, Severity, Status};
use ta_vfs::{Vfs, mods::{ModInfo, ModKind}};

const CACHE_VERSION: u32 = 3;
/// The most baselines kept (one per original game, Reborn and dependency set).
const MAX_BASELINES: usize = 8;
const MAX_CACHE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FILES: usize = 100_000;
const MAX_BYTES: u64 = 64 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationStatus { Unknown, Queued, Checking, Ready(Status), Stale, Failed(String) }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedReport {
    pub mod_id: String,
    pub name: String,
    pub version: String,
    pub reborn_version: String,
    pub checked_unix: u64,
    pub scope: Vec<String>,
    pub report: Report,
    pub fingerprint: String,
    cache_version: u32,
    /// Made by a check just now (not read from the cache).
    #[serde(skip)]
    pub fresh: bool,
}

impl SavedReport {
    /// Stable English diagnostics make copied reports useful to mod authors in every
    /// locale. The surrounding launcher controls are translated separately.
    pub fn to_text(&self) -> String {
        format!(
            "TA Reborn mod validation\nMod: {} ({})\nVersion: {}\nReborn: {}\nChecked: {} UTC\nScope: {}\nFingerprint: {}\n\n{}",
            self.name, self.mod_id, self.version, self.reborn_version,
            utc_stamp(self.checked_unix), self.scope.join(" -> "), self.fingerprint,
            self.report.to_text(),
        )
    }
}

struct Entry { status: ValidationStatus, report: Option<SavedReport> }
struct Active {
    id: String,
    force: bool,
    generation: u64,
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Event>,
    worker: JoinHandle<()>,
    /// What the worker is doing, and how far through it is (0..1) where it knows.
    progress: Option<(String, Option<f32>)>,
}
enum Event { Progress(String, Option<f32>), Finished(Result<SavedReport, String>) }

/// A check that finished with a report made just now (not from the cache).
pub struct Finished { pub id: String, pub status: Status }
#[derive(Clone)]
struct Request { id: String, force: bool }

/// Content digests by file stamp (path, size, times), kept between launches in
/// `digests.json`: a file whose stamp is unchanged is not read again. An explicit rescan
/// starts afresh and a recheck reads its mod's files again, so a file replaced by one of
/// the same size and times is caught when asked.
#[derive(Default)]
struct Digests {
    known: BTreeMap<FileStamp, String>,
    /// Those looked up or made this session (what is saved).
    used: BTreeSet<FileStamp>,
}
type DigestMemo = Arc<Mutex<Digests>>;
const DIGESTS: &str = "digests.json";
const MAX_DIGEST_BYTES: u64 = 32 * 1024 * 1024;

pub struct ModValidation {
    pub selected: Option<String>,
    cache_dir: PathBuf,
    base: Option<PathBuf>,
    mods: Arc<Vec<ModInfo>>,
    entries: BTreeMap<String, Entry>,
    queue: VecDeque<Request>,
    active: Option<Active>,
    generation: u64,
    memo: DigestMemo,
    /// Each mod's install as last seen (a reinstall with the same manifest changes it).
    stamps: BTreeMap<String, ModStamp>,
    paused: bool,
    stopped: bool,
}

impl ModValidation {
    pub fn new(cache_dir: PathBuf) -> Self {
        let memo = Arc::new(Mutex::new(load_digests(&cache_dir.join(DIGESTS))));
        Self {
            selected: None, cache_dir, base: None, mods: Arc::default(), entries: BTreeMap::new(),
            queue: VecDeque::new(), active: None, generation: 0, memo, stamps: BTreeMap::new(),
            paused: false, stopped: false,
        }
    }

    /// Call after discovery, installation/removal, or an explicit rescan, never each
    /// frame. Existing results remain readable but cannot award a current badge until
    /// the background worker has checked their fingerprint. `fresh` (an explicit rescan, a
    /// new install folder) reads every file again instead of trusting kept digests.
    pub fn sync(&mut self, base: Option<PathBuf>, mods: Vec<ModInfo>, fresh: bool) {
        if self.stopped { return; }
        self.generation = self.generation.wrapping_add(1);
        if let Some(active) = &self.active { active.cancel.store(true, Ordering::Relaxed); }
        self.base = base;
        if fresh { self.memo = Arc::default(); }
        self.queue.clear();
        let ids: BTreeSet<_> = mods.iter().map(|m| m.manifest.id.clone()).collect();
        self.entries.retain(|id, _| ids.contains(id));
        if self.selected.as_ref().is_some_and(|id| !ids.contains(id)) { self.selected = None; }
        for id in ids {
            let entry = self.entries.entry(id.clone()).or_insert(Entry { status: ValidationStatus::Queued, report: None });
            entry.status = if entry.report.is_some() { ValidationStatus::Stale } else { ValidationStatus::Queued };
            self.queue.push_back(Request { id, force: false });
        }
        self.stamps = mods.iter().map(|m| (m.manifest.id.clone(), mod_stamp(m))).collect();
        self.mods = Arc::new(mods);
    }

    /// Page navigation already discovers the mod inventory. If it has not changed,
    /// retain current checks and badges instead of reading every large archive again.
    /// Explicit refresh/install/edit operations still use `sync` to detect byte changes
    /// whose manifest, length and timestamps may have been preserved.
    pub fn sync_discovered(&mut self, base: Option<PathBuf>, mods: Vec<ModInfo>) {
        if self.base == base && same_inventory(&self.mods, &mods) { return; }
        self.sync(base, mods, false);
    }

    /// After an install, a removal or a game: the mods unchanged since they were last seen
    /// (where they were, their manifests the same, installed when they were) keep their checks;
    /// those new or changed (installed again, a map saved over itself), and those needing one of
    /// them, are checked again. (An explicit rescan checks them all.)
    pub fn sync_changed(&mut self, base: Option<PathBuf>, mods: Vec<ModInfo>) {
        if self.stopped { return; }
        if self.base != base { return self.sync(base, mods, true); }
        let old: BTreeMap<&str, &ModInfo> = self.mods.iter().map(|m| (m.manifest.id.as_str(), m)).collect();
        let stamps: BTreeMap<String, ModStamp> = mods.iter().map(|m| (m.manifest.id.clone(), mod_stamp(m))).collect();
        let changed: BTreeSet<String> = mods.iter()
            .filter(|m| old.get(m.manifest.id.as_str()).is_none_or(|o| !same_inventory(std::slice::from_ref(*o), std::slice::from_ref(*m)))
                || self.stamps.get(&m.manifest.id) != stamps.get(&m.manifest.id))
            .map(|m| m.manifest.id.clone()).collect();
        let removed: BTreeSet<String> = old.keys().filter(|id| !mods.iter().any(|m| m.manifest.id == **id)).map(|id| id.to_string()).collect();
        if changed.is_empty() && removed.is_empty() { return; }
        // The changed mods and every mod needing one of them, however indirectly.
        let mut affected: BTreeSet<String> = changed.clone();
        loop {
            let more: Vec<String> = mods.iter()
                .filter(|m| !affected.contains(&m.manifest.id) && m.manifest.depends.iter().any(|d| affected.contains(d) || removed.contains(d)))
                .map(|m| m.manifest.id.clone()).collect();
            if more.is_empty() { break; }
            affected.extend(more);
        }
        if let Some(active) = &self.active && (affected.contains(&active.id) || removed.contains(&active.id)) {
            active.cancel.store(true, Ordering::Relaxed);
        }
        self.entries.retain(|id, _| !removed.contains(id));
        if self.selected.as_ref().is_some_and(|id| removed.contains(id)) { self.selected = None; }
        self.queue.retain(|request| !removed.contains(&request.id) && !affected.contains(&request.id));
        for id in &affected {
            let entry = self.entries.entry(id.clone()).or_insert(Entry { status: ValidationStatus::Queued, report: None });
            entry.status = if entry.report.is_some() { ValidationStatus::Stale } else { ValidationStatus::Queued };
            self.queue.push_back(Request { id: id.clone(), force: false });
        }
        self.stamps = stamps;
        self.mods = Arc::new(mods);
    }

    pub fn request(&mut self, id: &str) {
        if self.stopped { return; }
        let Some(entry) = self.entries.get_mut(id) else { return; };
        entry.status = if entry.report.is_some() { ValidationStatus::Stale } else { ValidationStatus::Queued };
        self.queue.retain(|request| request.id != id);
        self.queue.push_front(Request { id: id.into(), force: true });
        // Explicit rechecks must read the mod's content again, including files whose
        // metadata an external editor preserved (the original game's are kept).
        if let Some(module) = self.mods.iter().find(|m| m.manifest.id == id) && let Ok(mut digests) = self.memo.lock() {
            digests.known.retain(|stamp, _| !stamp.path.starts_with(&module.path));
        }
        if let Some(active) = &self.active {
            if active.id == id { active.cancel.store(true, Ordering::Relaxed); }
        }
    }

    pub fn set_paused(&mut self, paused: bool) {
        if paused && !self.paused {
            if let Some(active) = &self.active {
                active.cancel.store(true, Ordering::Relaxed);
                if self.entries.contains_key(&active.id) && !self.queue.iter().any(|r| r.id == active.id) {
                    self.queue.push_front(Request { id: active.id.clone(), force: active.force });
                    if let Some(entry) = self.entries.get_mut(&active.id) {
                        entry.status = if entry.report.is_some() { ValidationStatus::Stale } else { ValidationStatus::Queued };
                    }
                }
            }
        }
        self.paused = paused;
    }

    pub fn busy(&self) -> bool { self.active.is_some() || (!self.paused && !self.queue.is_empty()) }
    pub fn status(&self, id: &str) -> ValidationStatus {
        self.entries.get(id).map_or(ValidationStatus::Unknown, |entry| entry.status.clone())
    }
    pub fn report(&self, id: &str) -> Option<&SavedReport> { self.entries.get(id)?.report.as_ref() }
    pub fn text(&self, id: &str) -> Option<String> {
        let report = self.report(id)?;
        let mut text = report.to_text();
        if !matches!(self.status(id), ValidationStatus::Ready(_)) {
            text.insert_str(0, "PREVIOUS REPORT: a new check is pending or the content has changed.\n\n");
        }
        Some(text)
    }
    pub fn progress(&self, id: &str) -> Option<(String, Option<f32>)> {
        self.active.as_ref().filter(|active| active.id == id && !active.cancel.load(Ordering::Relaxed))?.progress.clone()
    }

    /// How far the check under way is, where its worker knows.
    pub fn fraction(&self) -> Option<f32> {
        self.active.as_ref()?.progress.as_ref()?.1
    }

    /// Carries the checks on; returns one that has just finished with a fresh report.
    pub fn tick(&mut self, ctx: &egui::Context) -> Option<Finished> {
        let mut fresh = None;
        let mut finished = None;
        if let Some(active) = &mut self.active {
            while let Ok(event) = active.receiver.try_recv() {
                match event {
                    Event::Progress(text, fraction) => active.progress = Some((text, fraction)),
                    Event::Finished(result) => finished = Some(result),
                }
            }
            if finished.is_none() && active.worker.is_finished() {
                // The worker sends its final event before returning. Read once more to
                // avoid confusing a just-finished normal worker with a panic.
                while let Ok(event) = active.receiver.try_recv() {
                    if let Event::Finished(result) = event { finished = Some(result); }
                }
                if finished.is_none() { finished = Some(Err("The validation worker stopped unexpectedly. Run validation again.".into())); }
            }
        }
        if let Some(result) = finished {
            let active = self.active.take().expect("active validation");
            let current = active.generation == self.generation && !active.cancel.load(Ordering::Relaxed);
            if current {
                if let Some(entry) = self.entries.get_mut(&active.id) {
                    match result {
                        Ok(report) => {
                            entry.status = ValidationStatus::Ready(report.report.status());
                            if report.fresh { fresh = Some(Finished { id: active.id.clone(), status: report.report.status() }); }
                            entry.report = Some(report);
                        }
                        Err(error) => entry.status = ValidationStatus::Failed(error),
                    }
                }
            }
            // No UI wait: the worker has sent its result; dropping a thread handle is
            // non-blocking, and no source or cache work follows that final event.
            drop(active.worker);
            ctx.request_repaint();
        }
        if self.stopped || self.paused || self.active.is_some() { return fresh; }
        let Some(request) = self.queue.pop_front() else { return fresh; };
        let Some(entry) = self.entries.get_mut(&request.id) else { return fresh; };
        entry.status = ValidationStatus::Checking;
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let work_cancel = cancel.clone();
        let mods = self.mods.clone();
        let base = self.base.clone();
        let cache = self.cache_dir.clone();
        let memo = self.memo.clone();
        let generation = self.generation;
        let id = request.id.clone();
        let force = request.force;
        let repaint = ctx.clone();
        let worker = std::thread::spawn(move || {
            let mut last_progress = Instant::now() - Duration::from_secs(1);
            let mut progress = |message: String, fraction: Option<f32>| {
                if last_progress.elapsed() >= Duration::from_millis(150) {
                    let _ = sender.send(Event::Progress(message, fraction));
                    repaint.request_repaint();
                    last_progress = Instant::now();
                }
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(||
                run(&request, base.as_deref(), &mods, &cache, &memo, &work_cancel, &mut progress)
            )).unwrap_or_else(|_| Err("The validation worker stopped unexpectedly. Run validation again and include this mod in a bug report if the problem persists.".into()));
            let _ = sender.send(Event::Finished(result));
            repaint.request_repaint();
        });
        self.active = Some(Active { id, force, generation, cancel, receiver, worker, progress: None });
        ctx.request_repaint();
        fresh
    }

    pub fn shutdown(&mut self) {
        if self.stopped { return; }
        self.stopped = true;
        self.queue.clear();
        if let Some(active) = &self.active { active.cancel.store(true, Ordering::Relaxed); }
        if let Ok(digests) = self.memo.lock() && !digests.used.is_empty() {
            let kept: Vec<(&FileStamp, &String)> = digests.known.iter().filter(|(stamp, _)| digests.used.contains(*stamp)).collect();
            if let Ok(bytes) = serde_json::to_vec(&kept) && (bytes.len() as u64) <= MAX_DIGEST_BYTES
                && let Err(error) = ta_config::write_atomic_bytes(&self.cache_dir.join(DIGESTS), &bytes)
            {
                log::warn!("could not keep the mod validation digests: {error}");
            }
        }
    }
}

impl Drop for ModValidation { fn drop(&mut self) { self.shutdown(); } }

/// A mod's install seen from outside, cheaply: its folder's time and its manifest's size and
/// time, or its archive's. A reinstall, or a re-save leaving the manifest as it was, changes it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ModStamp(Vec<(u64, Option<SystemTime>)>);

fn mod_stamp(module: &ModInfo) -> ModStamp {
    let stat = |path: &Path| std::fs::metadata(path).map_or((0, None), |m| (m.len(), m.modified().ok()));
    ModStamp(match module.kind {
        ModKind::Archive => vec![stat(&module.path)],
        ModKind::Directory => vec![stat(&module.path), stat(&module.path.join("mod.toml"))],
    })
}

/// Compare only discovery's already-read values; this path runs on the UI thread.
fn same_inventory(left: &[ModInfo], right: &[ModInfo]) -> bool {
    if left.len() != right.len() { return false; }
    let left_by_id: BTreeMap<_, _> = left.iter().map(|m| (m.manifest.id.as_str(), m)).collect();
    let right_by_id: BTreeMap<_, _> = right.iter().map(|m| (m.manifest.id.as_str(), m)).collect();
    if left_by_id.len() != left.len() || right_by_id.len() != right.len() { return false; }
    left_by_id.iter().all(|(id, a)| right_by_id.get(id).is_some_and(|b| {
        // Exhaustive destructuring makes adding a manifest field require a deliberate
        // cache/discovery decision instead of silently ignoring the new setting.
        let ta_vfs::mods::ModManifest { id, name, version, description, authors, depends, load_after, source, rename, replaces } = &a.manifest;
        a.path == b.path && a.kind == b.kind && id == &b.manifest.id && name == &b.manifest.name
            && version == &b.manifest.version && description == &b.manifest.description && authors == &b.manifest.authors
            && depends == &b.manifest.depends && load_after == &b.manifest.load_after && source == &b.manifest.source
            && rename == &b.manifest.rename && replaces == &b.manifest.replaces
    }))
}

fn dependency_order<'a>(id: &str, mods: &'a [ModInfo]) -> Result<Vec<&'a ModInfo>, String> {
    let by_id: BTreeMap<_, _> = mods.iter().map(|m| (m.manifest.id.as_str(), m)).collect();
    let mut required = BTreeSet::new();
    let mut pending = vec![id.to_string()];
    while let Some(id) = pending.pop() {
        if !required.insert(id.clone()) { continue; }
        let Some(module) = by_id.get(id.as_str()) else { return Err(format!("Required mod '{id}' is not installed.")); };
        if required.len() > 1000 { return Err("The dependency set exceeds 1,000 mods.".into()); }
        pending.extend(module.manifest.depends.iter().cloned());
    }
    ta_vfs::mods::resolve_load_order(&required.into_iter().collect::<Vec<_>>(), mods).map_err(|e| e.to_string())
}

/// Manifest IDs are user-defined and may legitimately be "base" or "reborn".
/// Keep validation ownership distinct from those inherited VFS layers.
fn mod_layer(id: &str) -> String { format!("mod:{id}") }

fn run(
    request: &Request, base: Option<&Path>, mods: &[ModInfo], cache: &Path, memo: &DigestMemo,
    cancel: &AtomicBool, progress: &mut impl FnMut(String, Option<f32>),
) -> Result<SavedReport, String> {
    let target = mods.iter().find(|m| m.manifest.id == request.id).ok_or("The mod is no longer installed.")?;
    let target_layer = mod_layer(&target.manifest.id);
    let mut saved = SavedReport {
        mod_id: target.manifest.id.clone(), name: target.manifest.name.clone(), version: target.manifest.version.clone(),
        reborn_version: crate::version_label(), checked_unix: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        scope: vec!["Original game".into(), "Reborn built-in content".into()],
        report: Report { target_layer: Some(target_layer.clone()), ..Report::default() },
        fingerprint: String::new(), cache_version: CACHE_VERSION, fresh: false,
    };
    check_cancel(cancel)?;
    let order = match dependency_order(&request.id, mods) {
        Ok(order) => order,
        Err(error) => { saved.scope.push(scope_label(target)); return Ok(failure(saved, "DEPENDENCY", &error)); }
    };
    saved.scope.extend(order.iter().map(|m| scope_label(m)));
    for module in &order {
        if let Err(error) = check_manifest(module) { return Ok(failure(saved, "MANIFEST_CHANGED", &error)); }
    }
    let Some(base) = base else { return Ok(failure(saved, "BASE_MISSING", "The original Total Annihilation installation is not configured.")); };
    let builtin = ta_vfs::builtin_dir();
    if builtin.is_none() { saved.scope.retain(|s| s != "Reborn built-in content"); }
    let mut roots = vec![("base".into(), base.to_path_buf(), true)];
    if let Some(path) = &builtin { roots.push((ta_vfs::BUILTIN_LAYER.into(), path.clone(), false)); }
    roots.extend(order.iter().map(|m| (mod_layer(&m.manifest.id), m.path.clone(), false)));
    let folders: Vec<&Path> = roots.iter().map(|(_, path, _)| path.as_path()).collect();
    progress(ta_ui::i18n::t("Reading content fingerprints").to_string(), None);
    let before = match snapshot(&roots, cancel) {
        Ok(snapshot) => snapshot,
        Err(error) => { check_cancel(cancel)?; return Ok(failure(saved, "SOURCE_UNREADABLE", &without_folders(&error, &folders))); }
    };
    saved.fingerprint = match fingerprint(&before, &order, memo, cancel, None) {
        Ok(hash) => hash,
        Err(error) => { check_cancel(cancel)?; return Ok(failure(saved, "SOURCE_UNREADABLE", &without_folders(&error, &folders))); }
    };
    let cache_path = cache.join(cache_name(&saved.mod_id));
    if !request.force {
        if let Some(cached) = read_cache(&cache_path, &saved) {
            if snapshot(&roots, cancel)? == before { check_cancel(cancel)?; return Ok(cached); }
            return Err("Content changed while its cached report was checked. Run validation again.".into());
        }
    }
    // The same content without the mod, checked once and kept: what is wrong there already
    // is not this mod's doing, what is wrong only with it is (validation::compare).
    let dependencies: Vec<&ModInfo> = order.iter().copied().filter(|m| m.manifest.id != request.id).collect();
    let baseline_print = fingerprint(&before, &dependencies, memo, cancel, Some(&target_layer)).map_err(|e| without_folders(&e, &folders))?;
    let baseline_path = cache.join(format!("baseline-{}.json", &baseline_print[..16]));
    let baseline = match read_baseline(&baseline_path, &baseline_print) {
        Some(report) => report,
        None => {
            progress(ta_ui::i18n::t("Checking the original game without the mod (once)").to_string(), None);
            let vfs = match mount(base, builtin.as_deref(), &dependencies, None, &mut Vec::new(), cancel) {
                Ok(vfs) => vfs,
                Err(Mount::Cancelled) => return Err("Validation cancelled.".into()),
                Err(Mount::Failed(code, error)) => return Ok(failure(saved, code, &without_folders(&error, &folders))),
                Err(Mount::Unreadable(_)) => unreachable!("only the target's archive is reported as unreadable"),
            };
            let report = validation::validate(&vfs, &Options::default(), |p| {
                progress(ta_ui::i18n::tf("Original game: {checked} of {total} files", &[("checked", &ta_ui::i18n::grouped(p.checked)), ("total", &ta_ui::i18n::grouped(p.total))]).to_string(), Some(p.checked as f32 / p.total.max(1) as f32));
                !cancel.load(Ordering::Relaxed)
            });
            check_cancel(cancel)?;
            if report.complete { write_baseline(&baseline_path, &baseline_print, &report); }
            report
        }
    };
    let mut issues = Vec::new();
    progress(ta_ui::i18n::t("Mounting the original game and required dependencies").to_string(), None);
    let vfs = match mount(base, builtin.as_deref(), &order, Some(&target_layer), &mut issues, cancel) {
        Ok(vfs) => vfs,
        Err(Mount::Cancelled) => return Err("Validation cancelled.".into()),
        Err(Mount::Failed(code, error)) => return Ok(failure(saved, code, &without_folders(&error, &folders))),
        Err(Mount::Unreadable(issue)) => {
            // The mod is one archive the game cannot open: that is the whole report.
            saved.report = Report { target_layer: Some(target_layer), complete: true, issues: vec![*issue], ..Report::default() };
            if let Err(error) = write_cache(&cache_path, &saved) { log::warn!("could not save mod validation report: {error}"); }
            saved.fresh = true;
            return Ok(saved);
        }
    };
    saved.report = validation::validate(&vfs, &Options { target_layer: Some(target_layer), ..Options::default() }, |p| {
        progress(ta_ui::i18n::tf("{checked} of {total} files", &[("checked", &ta_ui::i18n::grouped(p.checked)), ("total", &ta_ui::i18n::grouped(p.total))]).to_string(), Some(p.checked as f32 / p.total.max(1) as f32));
        !cancel.load(Ordering::Relaxed)
    });
    // (Told apart only from a whole check of the content without it: against part of one,
    // what the rest has would all look the mod's doing.)
    if baseline.complete { validation::compare(&mut saved.report, &baseline); }
    // (A folder whose content is all in archives the game cannot open: those are its findings.)
    validation::content_unreadable(&mut saved.report, &issues);
    merge_issues(&mut saved.report, issues);
    check_cancel(cancel)?;
    if snapshot(&roots, cancel)? != before {
        return Err("Content changed during validation. Run validation again after installation or editing finishes.".into());
    }
    // The UI rejects cancelled generations too; this final gate also prevents a
    // stopped job from doing routine cache writes after shutdown or a rescan.
    check_cancel(cancel)?;
    if let Err(error) = write_cache(&cache_path, &saved) {
        // A read-only/full cache never hides a useful report or converts valid game
        // content into an error badge. The check is repeated at the next launch.
        log::warn!("could not save mod validation report: {error}");
    }
    saved.fresh = true;
    Ok(saved)
}

/// A mod as the report's scope names it.
fn scope_label(module: &ModInfo) -> String {
    let version = module.manifest.version.trim();
    if version.is_empty() { format!("{} [{}]", module.manifest.name, module.manifest.id) } else { format!("{} {version} [{}]", module.manifest.name, module.manifest.id) }
}

/// A message without the folders on this computer (a report is shared): each root named by
/// its last part.
fn without_folders(message: &str, roots: &[&Path]) -> String {
    let mut message = message.to_string();
    for root in roots {
        let name = root.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        for shown in [root.display().to_string(), root.display().to_string().replace('\\', "/")] {
            if !shown.is_empty() { message = message.replace(&shown, &name); }
        }
    }
    message
}

enum Mount {
    Cancelled,
    /// Mounting failed outright (a code and the reason).
    Failed(&'static str, String),
    /// The target is one archive the game cannot open (the finding).
    Unreadable(Box<Issue>),
}

/// Mounts the original game, Reborn's content and `modules` in order, as the game does
/// (`ta_vfs::mods::mount`: what they replace of the install left out), noting archives it would
/// skip in `issues`: the `target` layer's count, the others' are inherited.
fn mount(base: &Path, builtin: Option<&Path>, modules: &[&ModInfo], target: Option<&str>, issues: &mut Vec<Issue>, cancel: &AtomicBool) -> Result<Vfs, Mount> {
    let cancelled = |_| Mount::Cancelled;
    // (The install's archives the mods replace are not mounted, nor checked.)
    let replaced = ta_vfs::mods::replaced_archives(base, modules);
    let mut install = Vec::new();
    check_archives(base, "base", false, target, &mut install, cancel).map_err(cancelled)?;
    issues.extend(install.into_iter().filter(|issue| !replaced.iter().any(|name| name.eq_ignore_ascii_case(&issue.path))));
    if let Some(path) = builtin {
        check_archives(path, ta_vfs::BUILTIN_LAYER, false, target, issues, cancel).map_err(cancelled)?;
    }
    let mut found = Vec::new();
    for module in modules {
        check_cancel(cancel).map_err(cancelled)?;
        let layer = mod_layer(&module.manifest.id);
        found.push(issues.len());
        check_archives(&module.path, &layer, module.kind == ModKind::Archive, target, issues, cancel).map_err(cancelled)?;
    }
    check_cancel(cancel).map_err(cancelled)?;
    let mounted = ta_vfs::mods::mount(Some(base), builtin, modules, &|m| mod_layer(&m.manifest.id));
    mounted.install.map_err(|e| Mount::Failed("BASE_MOUNT", e.to_string()))?;
    mounted.builtin.map_err(|e| Mount::Failed("BUILTIN_MOUNT", e.to_string()))?;
    if let Err(error) = mounted.mods {
        let failed = match &error { ta_vfs::mods::ModError::Unreadable { id, .. } => modules.iter().position(|m| m.manifest.id == *id), _ => None };
        if let Some(index) = failed {
            let module = modules[index];
            let layer = mod_layer(&module.manifest.id);
            let first = found[index];
            let next = found.get(index + 1).copied().unwrap_or(issues.len());
            if Some(layer.as_str()) == target && module.kind == ModKind::Archive && next > first {
                return Err(Mount::Unreadable(Box::new(issues.remove(first))));
            }
        }
        return Err(Mount::Failed("MOD_MOUNT", error.to_string()));
    }
    for module in modules {
        let layer = mod_layer(&module.manifest.id);
        let replaced = ta_vfs::mods::replaced_archives(base, &[module]);
        // (An empty placeholder of a mod's, named like an archive of the install's, blanks that
        // one on purpose: no archive the game fails to read.)
        issues.retain(|issue| !(issue.code == "ARCHIVE_UNREADABLE" && issue.layer == layer && module.kind == ModKind::Directory
            && replaced.iter().any(|name| name.eq_ignore_ascii_case(&issue.path))
            && ta_vfs::package::is_placeholder(&module.path.join(&issue.path))));
        // (Which of the original game's archives the mod checked takes the place of: notes.)
        if Some(layer.as_str()) == target {
            issues.extend(replaced.iter().map(|archive| validation::replaced_archive(&layer, archive)));
        }
    }
    Ok(mounted.vfs)
}

fn failure(mut saved: SavedReport, code: &str, message: &str) -> SavedReport {
    // Preflight problems prevented a full scan. Preserve the concrete error while
    // keeping both the badge and copied report explicit about incomplete coverage.
    saved.report.complete = false;
    saved.report.issues.push(Issue {
        severity: Severity::Error, code: code.into(), layer: mod_layer(&saved.mod_id), message: message.into(),
        suggestion: "Restore the required files or dependencies, then run validation again.".into(), affects_status: true,
        ..Issue::default()
    });
    saved
}

fn merge_issues(report: &mut Report, issues: Vec<Issue>) {
    report.issues.extend(issues);
    let severity = |severity| match severity { Severity::Error => 0, Severity::Warning => 1, Severity::Info => 2 };
    // Archive mount diagnostics are collected before parsing but appended afterward.
    // Keep actionable errors on the first report page, ahead of inherited context.
    report.issues.sort_by(|a, b| (!a.affects_status, severity(a.severity), a.origin, &a.path, a.line, &a.code, &a.message)
        .cmp(&(!b.affects_status, severity(b.severity), b.origin, &b.path, b.line, &b.code, &b.message)));
}

/// Discovery runs before this worker. Do not use its old dependency/rename table if
/// an external editor changed the manifest while another mod was being checked.
fn check_manifest(module: &ModInfo) -> Result<(), String> {
    if module.kind != ModKind::Directory { return Ok(()); }
    let path = module.path.join("mod.toml");
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if is_link(&metadata) || metadata.len() > 1024 * 1024 => return Err(format!("The manifest for '{}' is linked or exceeds 1 MiB. Rescan the mod library after correcting it.", module.manifest.id)),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    // Read as the game reads it (a folder without one named by its whole name, its id in lower
    // case as the scan makes it: a copy of that here would drift from it).
    let disk = match ta_vfs::mods::read_mod(&module.path) {
        Ok(Some(found)) => found.manifest,
        Ok(None) => return Err(format!("'{}' is no longer a mod folder. Rescan the mod library.", module.manifest.id)),
        Err(error) => return Err(format!("The manifest for '{}' is invalid: {error}", module.manifest.id)),
    };
    if serde_json::to_vec(&disk).map_err(|e| e.to_string())? != serde_json::to_vec(&module.manifest).map_err(|e| e.to_string())? {
        return Err(format!("The manifest for '{}' changed after discovery. Rescan the mod library before validating it.", module.manifest.id));
    }
    Ok(())
}

fn check_archives(path: &Path, layer: &str, single: bool, target: Option<&str>, issues: &mut Vec<Issue>, cancel: &AtomicBool) -> Result<(), String> {
    let archives = if single { vec![path.to_path_buf()] } else {
        let entries = std::fs::read_dir(path).map_err(|e| e.to_string())?;
        let mut archives = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_file() && is_archive(&entry.path()) { archives.push(entry.path()); }
        }
        archives.sort();
        archives
    };
    for archive in archives {
        check_cancel(cancel)?;
        if let Err(error) = ta_formats::hpi::HpiArchive::open(&archive) {
            // (The target's own count; one the original game or a dependency ships is
            // skipped for every mod alike.)
            let own = target.is_none_or(|target| target == layer);
            let name = archive.file_name().unwrap_or_default().to_string_lossy().into_owned();
            issues.push(Issue {
                severity: Severity::Error, code: "ARCHIVE_UNREADABLE".into(), path: name.clone(), layer: layer.into(),
                message: format!("The game skips this archive because it cannot be read: {}", without_folders(&error.to_string(), &[path])),
                suggestion: "Replace the corrupt archive. Remove it only if it is an intentional empty placeholder.".into(),
                affects_status: own, origin: if own { validation::Origin::Mod } else { validation::Origin::Inherited }, ..Issue::default()
            });
        }
    }
    Ok(())
}

fn is_archive(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| ta_vfs::ARCHIVE_EXTENSIONS.contains(&s.to_ascii_lowercase().as_str()))
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) { Err("Validation cancelled.".into()) } else { Ok(()) }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct FileStamp { path: PathBuf, len: u64, modified: u128, created: u128 }
type Snapshot = Vec<(String, Vec<FileStamp>)>;

fn snapshot(roots: &[(String, PathBuf, bool)], cancel: &AtomicBool) -> Result<Snapshot, String> {
    fn visit(path: &Path, depth: usize, skip_root_files: bool, files: &mut Vec<FileStamp>, total: &mut u64, limit: usize, cancel: &AtomicBool) -> Result<(), String> {
        check_cancel(cancel)?;
        if depth > 64 || files.len() >= limit { return Err("Content exceeds the validation directory/file limit.".into()); }
        // (A root may be a link, the game on another drive behind a junction: what is below it
        // may not.)
        let metadata = if depth == 0 { std::fs::metadata(path) } else { std::fs::symlink_metadata(path) }.map_err(|e| format!("{}: {e}", path.display()))?;
        if depth > 0 && is_link(&metadata) { return Err(format!("Linked content is not validated: {}", path.display())); }
        let directory = metadata.is_dir();
        if !directory && !metadata.is_file() { return Err(format!("Content is not a regular file: {}", path.display())); }
        if !directory {
            *total = total.saturating_add(metadata.len());
            if *total > MAX_BYTES { return Err("Content exceeds the 64 GiB validation limit.".into()); }
        }
        let time = |time: std::io::Result<SystemTime>| time.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |t| t.as_nanos());
        // Only what the game reads (the music, movies and documents beside it changing
        // makes no difference to a check).
        let read = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| is_archive(path) || validation::checked_file(&n.to_ascii_lowercase()));
        if !directory && read {
            files.push(FileStamp { path: path.to_path_buf(), len: metadata.len(), modified: time(metadata.modified()), created: time(metadata.created()) });
        }
        if directory {
            let entries = std::fs::read_dir(path).map_err(|e| e.to_string())?;
            let mut children = Vec::new();
            for entry in entries {
                if children.len() + files.len() >= limit { return Err("Content exceeds the validation file limit.".into()); }
                let entry = entry.map_err(|e| e.to_string())?;
                // Base-game root files are not in the loose VFS. In particular, do
                // not hash the original executable, its movies or installers.
                if skip_root_files && depth == 0 && !entry.file_type().map_err(|e| e.to_string())?.is_dir() && !is_archive(&entry.path()) { continue; }
                children.push(entry.path());
            }
            children.sort();
            for child in children { visit(&child, depth + 1, skip_root_files, files, total, limit, cancel)?; }
        }
        Ok(())
    }
    let mut snapshot = Vec::new();
    let mut total = 0;
    let mut files_seen = 0;
    // (Its errors name no folder of this computer: every caller's report may be shared.)
    let folders: Vec<&Path> = roots.iter().map(|(_, path, _)| path.as_path()).collect();
    for (layer, path, skip_root_files) in roots {
        let mut files = Vec::new();
        visit(path, 0, *skip_root_files, &mut files, &mut total, MAX_FILES - files_seen, cancel).map_err(|e| without_folders(&e, &folders))?;
        files_seen += files.len();
        snapshot.push((layer.clone(), files));
    }
    Ok(snapshot)
}

fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    { use std::os::windows::fs::MetadataExt; metadata.file_attributes() & 0x400 != 0 }
    #[cfg(not(windows))]
    { metadata.file_type().is_symlink() }
}

/// The content's fingerprint: the checks' versions (not the launcher's build: a rebuild that
/// leaves the checks alone keeps every report), the manifests of `order` and what they replace of
/// the original game, and every file's digest, but those of the `without` layer.
fn fingerprint(snapshot: &Snapshot, order: &[&ModInfo], memo: &DigestMemo, cancel: &AtomicBool, without: Option<&str>) -> Result<String, String> {
    let mut hash = Sha256::new();
    hash.update(format!("{}:{}", CACHE_VERSION, validation::SCHEMA_VERSION));
    for module in order {
        hash.update(serde_json::to_vec(&module.manifest).map_err(|e| e.to_string())?);
        // What it replaces of the original game, when it replaces any: a report made before the
        // install's archives of those names were left out is not this mod's as mounted now.
        let replaced = ta_vfs::mods::replaced_claims(module);
        if !replaced.is_empty() { hash.update(serde_json::to_vec(&replaced).map_err(|e| e.to_string())?); }
    }
    for (layer, files) in snapshot {
        if Some(layer.as_str()) == without { continue; }
        hash.update(layer.as_bytes());
        for stamp in files {
            check_cancel(cancel)?;
            hash.update(serde_json::to_vec(stamp).map_err(|e| e.to_string())?);
            let cached = {
                let mut digests = memo.lock().map_err(|_| "Content fingerprint cache was interrupted.")?;
                let digest = digests.known.get(stamp).cloned();
                if digest.is_some() { digests.used.insert(stamp.clone()); }
                digest
            };
            let digest = if let Some(digest) = cached { digest } else {
                let mut file = std::fs::File::open(&stamp.path).map_err(|e| e.to_string())?;
                let mut content = Sha256::new();
                let mut buffer = [0u8; 128 * 1024];
                let mut bytes = 0u64;
                loop {
                    check_cancel(cancel)?;
                    let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
                    if n == 0 { break; }
                    bytes = bytes.saturating_add(n as u64);
                    if bytes > stamp.len { return Err("Content changed while it was being fingerprinted.".into()); }
                    content.update(&buffer[..n]);
                }
                if bytes != stamp.len { return Err("Content changed while it was being fingerprinted.".into()); }
                let digest = format!("{:x}", content.finalize());
                let mut digests = memo.lock().map_err(|_| "Content fingerprint cache was interrupted.")?;
                digests.known.insert(stamp.clone(), digest.clone());
                digests.used.insert(stamp.clone());
                digest
            };
            hash.update(digest.as_bytes());
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn cache_name(id: &str) -> String { format!("{:x}.json", Sha256::digest(id.as_bytes())) }
fn read_cache(path: &Path, expected: &SavedReport) -> Option<SavedReport> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_CACHE_BYTES { return None; }
    let mut bytes = Vec::new();
    file.take(MAX_CACHE_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_CACHE_BYTES { return None; }
    let cached: SavedReport = serde_json::from_slice(&bytes).ok()?;
    (cached.cache_version == CACHE_VERSION && cached.report.schema_version == validation::SCHEMA_VERSION
        && cached.mod_id == expected.mod_id && cached.version == expected.version && cached.name == expected.name
        && cached.fingerprint == expected.fingerprint
        && cached.scope == expected.scope && cached.report.target_layer == expected.report.target_layer
        && !cached.fingerprint.is_empty() && cached.report.complete).then_some(cached)
}
/// A baseline (the content without a mod) as kept: its fingerprint and report.
#[derive(Serialize, Deserialize)]
struct Baseline { fingerprint: String, cache_version: u32, report: Report }

fn read_baseline(path: &Path, fingerprint: &str) -> Option<Report> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_CACHE_BYTES { return None; }
    let mut bytes = Vec::new();
    file.take(MAX_CACHE_BYTES + 1).read_to_end(&mut bytes).ok()?;
    let kept: Baseline = serde_json::from_slice(&bytes).ok()?;
    (kept.fingerprint == fingerprint && kept.cache_version == CACHE_VERSION && kept.report.schema_version == validation::SCHEMA_VERSION && kept.report.complete).then_some(kept.report)
}

/// Keeps a baseline, and the `MAX_BASELINES` newest of them.
fn write_baseline(path: &Path, fingerprint: &str, report: &Report) {
    let kept = Baseline { fingerprint: fingerprint.into(), cache_version: CACHE_VERSION, report: report.clone() };
    match serde_json::to_vec(&kept) {
        Ok(bytes) if bytes.len() as u64 <= MAX_CACHE_BYTES => {
            if let Err(error) = ta_config::write_atomic_bytes(path, &bytes) { log::warn!("could not keep a validation baseline: {error}"); }
        }
        _ => return,
    }
    let Some(dir) = path.parent() else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut baselines: Vec<(SystemTime, PathBuf)> = entries.flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("baseline-"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    baselines.sort_by_key(|b| std::cmp::Reverse(b.0));
    for (_, old) in baselines.into_iter().skip(MAX_BASELINES) { let _ = std::fs::remove_file(old); }
}

/// The digests kept from earlier launches (none if unreadable).
fn load_digests(path: &Path) -> Digests {
    let read = || -> Option<Vec<(FileStamp, String)>> {
        let file = std::fs::File::open(path).ok()?;
        if file.metadata().ok()?.len() > MAX_DIGEST_BYTES { return None; }
        let mut bytes = Vec::new();
        file.take(MAX_DIGEST_BYTES + 1).read_to_end(&mut bytes).ok()?;
        serde_json::from_slice(&bytes).ok()
    };
    Digests { known: read().unwrap_or_default().into_iter().collect(), used: BTreeSet::new() }
}

fn write_cache(path: &Path, report: &SavedReport) -> Result<(), String> {
    let bytes = serde_json::to_vec(report).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_CACHE_BYTES { return Err("Validation report exceeds the 16 MiB cache limit.".into()); }
    ta_config::write_atomic_bytes(path, &bytes).map_err(|e| e.to_string())
}

/// Gregorian UTC date without an extra runtime/date dependency.
fn utc_stamp(seconds: u64) -> String {
    let days = (seconds / 86_400).min(i64::MAX as u64 - 719_468) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}", seconds / 3600 % 24, seconds / 60 % 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use ta_vfs::mods::ModManifest;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("ta-validation-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    fn module(id: &str, root: &Path, dependencies: &[&str]) -> ModInfo {
        ModInfo {
            manifest: ModManifest {
                id: id.into(), name: format!("Mod {id}"), version: "1.0".into(), description: String::new(),
                authors: vec![], depends: dependencies.iter().map(|s| s.to_string()).collect(), load_after: vec![], source: String::new(), rename: BTreeMap::new(), replaces: Vec::new(),
            },
            path: root.into(), kind: ModKind::Directory,
        }
    }
    fn report(id: &str) -> SavedReport {
        SavedReport {
            mod_id: id.into(), name: format!("Mod {id}"), version: "1.0".into(), reborn_version: crate::version_label(),
            checked_unix: 1_782_864_000, scope: vec!["Original game".into(), format!("Mod {id} 1.0 [{id}]")],
            report: Report { target_layer: Some(mod_layer(id)), complete: true, ..Report::default() },
            fingerprint: "abc".into(), cache_version: CACHE_VERSION, fresh: false,
        }
    }

    #[test]
    fn dependency_scope_includes_required_mods_but_not_unrelated_enabled_content() {
        let mut a = module("a", Path::new("a"), &["b"]);
        a.manifest.load_after.push("unrelated".into());
        let mods = vec![a, module("b", Path::new("b"), &["c"]), module("c", Path::new("c"), &[]), module("unrelated", Path::new("u"), &[])];
        let ids: Vec<_> = dependency_order("a", &mods).unwrap().iter().map(|m| m.manifest.id.as_str()).collect();
        assert_eq!(ids, ["c", "b", "a"]);
        assert!(dependency_order("a", &mods[..2]).unwrap_err().contains("'c'"));
        let cycle = vec![module("a", Path::new("a"), &["b"]), module("b", Path::new("b"), &["a"])];
        assert!(dependency_order("a", &cycle).unwrap_err().contains("cycle"));
    }

    #[test]
    fn user_mod_ids_cannot_collide_with_original_or_builtin_content_ownership() {
        let dir = Temp::new();
        let original = dir.0.join("original");
        let builtin = dir.0.join("builtin");
        let installed = dir.0.join("installed");
        std::fs::create_dir_all(original.join("unitpics")).unwrap();
        std::fs::create_dir_all(builtin.join("unitpics")).unwrap();
        std::fs::create_dir_all(installed.join("weapons")).unwrap();
        std::fs::write(original.join("unitpics/base.pcx"), b"unrelated broken base picture").unwrap();
        std::fs::write(builtin.join("unitpics/builtin.pcx"), b"unrelated broken builtin picture").unwrap();
        std::fs::write(installed.join("weapons/test.tdf"), b"[TestWeapon]{rendertype=0;}").unwrap();
        for id in ["base", ta_vfs::BUILTIN_LAYER, "mod:base"] {
            let installed = module(id, &installed, &[]);
            let vfs = mount(&original, Some(&builtin), &[&installed], Some(&mod_layer(id)), &mut Vec::new(), &AtomicBool::new(false)).unwrap_or_else(|_| panic!("{id} mounts"));
            assert_eq!(vfs.origin("weapons/test.tdf").unwrap().layer, mod_layer(id));
            let report = validation::validate(&vfs, &Options { target_layer: Some(mod_layer(id)), ..Options::default() }, |_| true);
            assert_eq!(report.status(), Status::Clean, "{}", report.to_text());
            assert_eq!(report.issues.len(), 2);
            assert!(report.issues.iter().all(|issue| !issue.affects_status));
        }
    }

    /// A folder without a manifest is checked as the scan found it (named by its whole name, in
    /// lower case as the settings keep ids), not as "changed after discovery".
    #[test]
    fn a_folder_without_a_manifest_is_checked_as_discovered() {
        let dir = Temp::new();
        for name in ["Mod v1.2", "Übermacht"] {
            let folder = dir.0.join(name);
            std::fs::create_dir_all(folder.join("units")).unwrap();
            let found = ta_vfs::mods::read_mod(&folder).unwrap().unwrap();
            assert_eq!(found.manifest.id, name.to_lowercase());
            assert_eq!(check_manifest(&found), Ok(()), "{name}");
        }
    }

    /// What reads the content's files fails without a folder of this computer in its words: the
    /// report it ends up in is copied and shared (the cache's check and the one after a check
    /// pass it on as it is).
    #[test]
    fn snapshot_errors_name_no_folder_of_this_computer() {
        let dir = Temp::new();
        let gone = dir.0.join("gone mod");
        let error = snapshot(&[(mod_layer("gone"), gone.clone(), false)], &AtomicBool::new(false)).unwrap_err();
        assert!(!error.contains(&dir.0.display().to_string()), "{error}");
        assert!(error.contains("gone mod"), "{error}");
    }

    /// A mod folder whose content is all in an archive the game cannot open has that as its
    /// finding, an error: not a check that could not complete.
    #[test]
    fn a_folder_of_unreadable_archives_reports_errors_not_an_incomplete_check() {
        let dir = Temp::new();
        let (base, folder) = (dir.0.join("original"), dir.0.join("broken"));
        std::fs::create_dir_all(base.join("units")).unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        let broken = module("broken", &folder, &[]);
        std::fs::write(folder.join("mod.toml"), toml::to_string(&broken.manifest).unwrap()).unwrap();
        std::fs::write(folder.join("content.ufo"), b"HAPI and nothing a reader can use").unwrap();
        let result = run(&Request { id: "broken".into(), force: true }, Some(&base), std::slice::from_ref(&broken), &dir.0.join("cache"), &Arc::default(), &AtomicBool::new(false), &mut |_, _| {}).unwrap();
        assert!(result.report.complete, "{}", result.to_text());
        assert_eq!(result.report.status(), Status::Errors, "{}", result.to_text());
        assert!(result.report.issues.iter().any(|i| i.code == "ARCHIVE_UNREADABLE" && i.affects_status));
    }

    /// A mod's archive named like one of the original game's replaces it in the check as in the
    /// game (TA: Enhanced's `rev31.gp3`): the original's units in it are not read with the mod, and
    /// a note says which archive it takes the place of.
    #[test]
    fn a_mods_archive_named_like_the_originals_replaces_it_in_the_check() {
        let dir = Temp::new();
        let base = dir.0.join("original");
        ta_vfs::test_archive::write(&base.join("rev31.gp3"), &[("units/armsub.fbi", b"[UNITINFO]{UnitName=ARMSUB;Objectname=ARMSUB;Weapon1=ARM_TORPEDO;}")]);
        ta_vfs::test_archive::write(&base.join("ccmaps.ccx"), &[("units/corsub.fbi", b"[UNITINFO]{UnitName=CORSUB;Objectname=CORSUB;Weapon1=CORE_TORPEDO;}")]);
        ta_vfs::test_archive::write(&base.join("btdata.ccx"), &[("units/armhp.fbi", b"[UNITINFO]{UnitName=ARMHP;Objectname=ARMHP;}")]);
        let folder = dir.0.join("enhanced");
        let enhanced = module("enhanced", &folder, &[]);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("mod.toml"), toml::to_string(&enhanced.manifest).unwrap()).unwrap();
        ta_vfs::test_archive::write(&folder.join("REV31.GP3"), &[("units/armcom.fbi", b"[UNITINFO]{UnitName=ARMCOM;Objectname=ARMCOM;}")]);
        // (Kept by a hand install: an empty placeholder, which blanks the original's on purpose.)
        std::fs::write(folder.join("btdata.ccx"), b"").unwrap();
        let result = run(&Request { id: "enhanced".into(), force: true }, Some(&base), std::slice::from_ref(&enhanced), &dir.0.join("cache"), &Arc::default(), &AtomicBool::new(false), &mut |_, _| {}).unwrap();
        assert!(result.report.complete, "{}", result.to_text());
        assert!(!result.report.issues.iter().any(|i| i.path.starts_with("units/armsub") || i.path.starts_with("units/armhp")), "the original's units in the archives it replaces are not read: {}", result.to_text());
        assert!(result.report.issues.iter().any(|i| i.path.starts_with("units/corsub") && i.message.contains("CORE_TORPEDO")), "the original's other archives are: {}", result.to_text());
        assert!(!result.report.issues.iter().any(|i| i.code == "ARCHIVE_UNREADABLE"), "a placeholder is no broken archive: {}", result.to_text());
        let notes: Vec<(&str, Severity, &str)> = result.report.issues.iter().filter(|i| i.code == "packaging.replaces_original").map(|i| (i.path.as_str(), i.severity, i.layer.as_str())).collect();
        assert_eq!(notes, [("btdata.ccx", Severity::Info, "mod:enhanced"), ("rev31.gp3", Severity::Info, "mod:enhanced")], "{}", result.to_text());
        // (A note: it counts nothing against the mod.)
        let mut without = result.report.clone();
        without.issues.retain(|i| i.code != "packaging.replaces_original");
        assert_eq!(result.report.status(), without.status());
    }

    #[test]
    fn missing_dependencies_produce_a_copyable_error_report() {
        let mods = vec![module("test", Path::new("unused"), &["missing"] )];
        let result = run(&Request { id: "test".into(), force: false }, None, &mods, Path::new("unused-cache"), &Arc::default(), &AtomicBool::new(false), &mut |_, _| {}).unwrap();
        assert_eq!(result.report.status(), Status::Incomplete);
        assert!(!result.report.complete);
        assert!(result.to_text().contains("missing"));
        assert!(result.to_text().contains("DEPENDENCY"));
        assert!(result.to_text().contains("Mod test"));
    }

    #[test]
    fn edits_between_discovery_and_validation_cannot_reuse_stale_dependency_metadata() {
        let dir = Temp::new();
        let discovered = module("test", &dir.0, &[]);
        let path = dir.0.join("mod.toml");
        std::fs::write(&path, toml::to_string(&discovered.manifest).unwrap()).unwrap();
        check_manifest(&discovered).unwrap();
        let mut edited = discovered.manifest.clone();
        edited.depends.push("new-required-mod".into());
        std::fs::write(&path, toml::to_string(&edited).unwrap()).unwrap();
        assert!(check_manifest(&discovered).unwrap_err().contains("changed after discovery"));
        std::fs::remove_file(&path).unwrap();
        assert!(check_manifest(&discovered).is_err());
    }

    #[test]
    fn fingerprints_detect_same_version_replacements_even_with_preserved_metadata() {
        let dir = Temp::new();
        let file = dir.0.join("content.tdf");
        std::fs::write(&file, b"before").unwrap();
        let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
        let roots = vec![("test".into(), dir.0.clone(), false)];
        let mods = [module("test", &dir.0, &[])];
        let order = [&mods[0]];
        let cancel = AtomicBool::new(false);
        let before = snapshot(&roots, &cancel).unwrap();
        let a = fingerprint(&before, &order, &Arc::default(), &cancel, None).unwrap();
        std::fs::write(&file, b"after!").unwrap();
        std::fs::File::options().write(true).open(&file).unwrap().set_times(std::fs::FileTimes::new().set_modified(modified)).unwrap();
        let after = snapshot(&roots, &cancel).unwrap();
        let b = fingerprint(&after, &order, &Arc::default(), &cancel, None).unwrap();
        assert_ne!(a, b, "a fresh rescan must hash bytes even when a replacement preserves the manifest and file metadata");
        std::fs::write(dir.0.join("readme.txt"), "not read by the game").unwrap();
        assert_eq!(after, snapshot(&roots, &cancel).unwrap(), "what the game never reads does not change the fingerprint");
        std::fs::write(dir.0.join("added.tdf"), "new").unwrap();
        assert_ne!(after, snapshot(&roots, &cancel).unwrap());
    }

    #[test]
    fn base_fingerprints_skip_unmounted_root_programs_but_track_archives_and_loose_content() {
        let dir = Temp::new();
        std::fs::write(dir.0.join("TotalA.exe"), "not game data").unwrap();
        std::fs::write(dir.0.join("intro.zrb"), "not mounted from the root").unwrap();
        std::fs::write(dir.0.join("data.HPI"), "archive").unwrap();
        std::fs::create_dir(dir.0.join("units")).unwrap();
        std::fs::write(dir.0.join("units").join("unit.fbi"), "unit").unwrap();
        let files = snapshot(&[("base".into(), dir.0.clone(), true)], &AtomicBool::new(false)).unwrap();
        let names: Vec<_> = files[0].1.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["data.HPI", "unit.fbi"]);
    }

    #[test]
    fn cache_rejects_different_content_engine_schema_dependency_scope_and_partial_reports() {
        let dir = Temp::new();
        let path = dir.0.join(cache_name("../unsafe/name"));
        assert_eq!(path.parent(), Some(dir.0.as_path()));
        let valid = report("test");
        write_cache(&path, &valid).unwrap();
        assert!(read_cache(&path, &valid).is_some());
        for changed in 0..6 {
            let mut expected = valid.clone();
            match changed {
                0 => expected.fingerprint.push('x'),
                1 => expected.scope.push("New dependency".into()),
                2 => expected.version.push('x'),
                3 => expected.mod_id.push('x'),
                4 => expected.name.push('x'),
                _ => expected.report.target_layer = None,
            }
            assert!(read_cache(&path, &expected).is_none());
        }
        let mut rebuilt = valid.clone();
        rebuilt.reborn_version.push_str(" (another build)");
        assert!(read_cache(&path, &rebuilt).is_some(), "a launcher rebuilt with the same checks keeps its reports");
        let mut old = valid.clone();
        old.report.schema_version = validation::SCHEMA_VERSION + 1;
        write_cache(&path, &old).unwrap();
        assert!(read_cache(&path, &valid).is_none());
        old = valid.clone();
        old.report.complete = false;
        write_cache(&path, &old).unwrap();
        assert!(read_cache(&path, &valid).is_none());
        std::fs::write(&path, b"{truncated").unwrap();
        assert!(read_cache(&path, &valid).is_none());
    }

    #[test]
    fn corrupt_top_level_archives_are_reported_even_when_vfs_would_skip_them() {
        let dir = Temp::new();
        std::fs::write(dir.0.join("broken.UFO"), b"broken archive").unwrap();
        let mut issues = Vec::new();
        check_archives(&dir.0, &mod_layer("test"), false, Some(&mod_layer("test")), &mut issues, &AtomicBool::new(false)).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, "ARCHIVE_UNREADABLE");
        assert_eq!(issues[0].layer, "mod:test");
        assert_eq!(issues[0].path, "broken.UFO");
        assert!(issues[0].affects_status);
        assert!(!issues[0].message.contains(&dir.0.display().to_string()), "no folder of this computer in a shared report: {}", issues[0].message);
        // The original game's broken archive is skipped for every mod alike: not this one's doing.
        let mut inherited = Vec::new();
        check_archives(&dir.0, "base", false, Some(&mod_layer("test")), &mut inherited, &AtomicBool::new(false)).unwrap();
        assert!(!inherited[0].affects_status && inherited[0].origin == validation::Origin::Inherited);

        let mut report = Report::default();
        report.issues = (0..51).map(|i| Issue { path: format!("inherited/{i}.gaf"), affects_status: false, ..Issue::default() }).collect();
        merge_issues(&mut report, issues);
        assert_eq!(report.issues[0].code, "ARCHIVE_UNREADABLE", "the fatal archive diagnostic must remain on the first report page");
    }

    #[test]
    fn rescan_removes_missing_mods_and_never_publishes_a_replaced_workers_report() {
        let dir = Temp::new();
        let mut validation = ModValidation::new(dir.0.clone());
        validation.sync(None, vec![module("old", &dir.0, &[])], true);
        validation.entries.get_mut("old").unwrap().report = Some(report("old"));
        validation.selected = Some("old".into());
        let (sender, receiver) = mpsc::channel();
        sender.send(Event::Finished(Ok(report("old")))).unwrap();
        validation.active = Some(Active {
            id: "old".into(), force: false, generation: validation.generation, cancel: Arc::new(AtomicBool::new(false)), receiver,
            worker: std::thread::spawn(|| {}), progress: None,
        });
        validation.sync(None, vec![module("new", &dir.0, &[])], true);
        validation.set_paused(true);
        validation.tick(&egui::Context::default());
        assert!(validation.selected.is_none());
        assert_eq!(validation.status("old"), ValidationStatus::Unknown);
        assert!(validation.report("old").is_none());
        assert_eq!(validation.status("new"), ValidationStatus::Queued);
        assert!(validation.report("new").is_none());
        assert_eq!(validation.queue.len(), 1);

        // Reinstalling the same ID is the subtle case: the destination entry still
        // exists, but an old worker must not award the replacement its old badge.
        let (sender, receiver) = mpsc::channel();
        sender.send(Event::Finished(Ok(report("new")))).unwrap();
        validation.active = Some(Active {
            id: "new".into(), force: false, generation: validation.generation, cancel: Arc::new(AtomicBool::new(false)), receiver,
            worker: std::thread::spawn(|| {}), progress: None,
        });
        let mut replacement = module("new", &dir.0, &[]);
        replacement.manifest.version = "2.0".into();
        validation.sync(None, vec![replacement], true);
        validation.tick(&egui::Context::default());
        assert!(validation.report("new").is_none());
        assert_eq!(validation.status("new"), ValidationStatus::Queued);
    }

    #[test]
    fn after_an_install_only_the_changed_mods_and_those_needing_them_are_checked_again() {
        let mods = vec![module("base", Path::new("not-on-disk-a"), &[]), module("addon", Path::new("not-on-disk-b"), &["base"]),
            module("other", Path::new("not-on-disk-c"), &[]), module("addon2", Path::new("not-on-disk-e"), &["addon"])];
        let mut validation = ModValidation::new(PathBuf::from("unused-cache"));
        validation.sync(None, mods.clone(), false);
        validation.queue.clear();
        for id in ["base", "addon", "other", "addon2"] {
            let entry = validation.entries.get_mut(id).unwrap();
            entry.status = ValidationStatus::Ready(Status::Clean);
            entry.report = Some(report(id));
        }
        let memo = validation.memo.clone();
        validation.sync_changed(None, mods.clone());
        assert!(validation.queue.is_empty(), "nothing changed: nothing is checked again");
        let mut updated = mods.clone();
        updated[0].manifest.version = "2.0".into();
        updated.push(module("new", Path::new("not-on-disk-d"), &[]));
        validation.sync_changed(None, updated.clone());
        let queued: Vec<&str> = validation.queue.iter().map(|request| request.id.as_str()).collect();
        assert_eq!(queued, ["addon", "addon2", "base", "new"], "the changed mod, those needing it however indirectly, and the new one");
        assert_eq!(validation.status("other"), ValidationStatus::Ready(Status::Clean), "an unchanged mod keeps its check");
        assert_eq!(validation.status("base"), ValidationStatus::Stale);
        assert!(Arc::ptr_eq(&memo, &validation.memo), "the content hashes already read are kept");
        updated.retain(|module| module.manifest.id != "other");
        validation.sync_changed(None, updated);
        assert_eq!(validation.status("other"), ValidationStatus::Unknown, "a removed mod is forgotten");
    }

    #[test]
    fn a_reinstall_with_the_same_manifest_is_checked_again() {
        let temp = Temp::new();
        let root = temp.0.join("same");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("mod.toml"), "id = \"same\"\nname = \"Same\"\n").unwrap();
        let mods = vec![module("same", &root, &[])];
        let mut validation = ModValidation::new(temp.0.join("cache"));
        validation.sync(None, mods.clone(), false);
        validation.queue.clear();
        let entry = validation.entries.get_mut("same").unwrap();
        entry.status = ValidationStatus::Ready(Status::Clean);
        entry.report = Some(report("same"));
        validation.sync_changed(None, mods.clone());
        assert!(validation.queue.is_empty(), "nothing changed: nothing is checked again");
        // Installed again (or a map saved over itself): the same manifest, written anew.
        let manifest = std::fs::OpenOptions::new().write(true).open(root.join("mod.toml")).unwrap();
        manifest.set_modified(SystemTime::now() + Duration::from_secs(60)).unwrap();
        drop(manifest);
        validation.sync_changed(None, mods);
        assert_eq!(validation.queue.iter().map(|request| request.id.as_str()).collect::<Vec<_>>(), ["same"]);
        assert_eq!(validation.status("same"), ValidationStatus::Stale);
    }

    #[test]
    fn passive_discovery_preserves_work_and_badges_until_inventory_changes() {
        // These deliberately nonexistent paths also keep this discovery-only test
        // independent of any source-file reads or original game installation.
        let mods = vec![module("active", Path::new("not-on-disk-a"), &[]), module("ready", Path::new("not-on-disk-b"), &[])];
        let mut validation = ModValidation::new(PathBuf::from("unused-cache"));
        validation.sync(None, mods.clone(), false);
        validation.queue.clear();
        validation.entries.get_mut("active").unwrap().status = ValidationStatus::Checking;
        let ready = validation.entries.get_mut("ready").unwrap();
        ready.status = ValidationStatus::Ready(Status::Clean);
        ready.report = Some(report("ready"));
        let (_sender, receiver) = mpsc::channel();
        validation.active = Some(Active {
            id: "active".into(), force: false, generation: validation.generation, cancel: Arc::new(AtomicBool::new(false)), receiver,
            worker: std::thread::spawn(|| {}), progress: Some(("5 / 20".into(), Some(0.25))),
        });
        let generation = validation.generation;
        let memo = validation.memo.clone();
        let mut reordered = mods.clone();
        reordered.reverse();
        validation.sync_discovered(None, reordered);
        assert_eq!(validation.generation, generation);
        assert!(Arc::ptr_eq(&memo, &validation.memo));
        assert!(!validation.active.as_ref().unwrap().cancel.load(Ordering::Relaxed));
        assert!(validation.queue.is_empty());
        assert_eq!(validation.status("active"), ValidationStatus::Checking);
        assert_eq!(validation.status("ready"), ValidationStatus::Ready(Status::Clean));
        assert_eq!(validation.progress("active").map(|p| p.0).as_deref(), Some("5 / 20"));
        assert_eq!(validation.fraction(), Some(0.25));

        let mut edited = mods.clone();
        edited[0].manifest.depends.push("ready".into());
        validation.sync_discovered(None, edited.clone());
        assert_ne!(validation.generation, generation);
        assert!(Arc::ptr_eq(&memo, &validation.memo), "passive discovery keeps the digests already read");
        assert!(validation.active.as_ref().unwrap().cancel.load(Ordering::Relaxed));
        assert_eq!(validation.queue.len(), 2);
        assert_eq!(validation.status("ready"), ValidationStatus::Stale);

        let generation = validation.generation;
        edited.push(module("new", Path::new("not-on-disk-c"), &[]));
        validation.sync_discovered(None, edited);
        assert_ne!(validation.generation, generation);
        assert_eq!(validation.status("new"), ValidationStatus::Queued);
        assert_eq!(validation.queue.len(), 3);
    }

    #[test]
    fn rechecks_and_pause_cancel_and_requeue_without_losing_the_previous_report() {
        let dir = Temp::new();
        let mut validation = ModValidation::new(dir.0.clone());
        validation.sync(None, vec![module("test", &dir.0, &[])], false);
        validation.entries.get_mut("test").unwrap().report = Some(report("test"));
        validation.request("test");
        validation.request("test");
        assert_eq!(validation.queue.len(), 1);
        assert!(validation.queue.front().unwrap().force);
        assert_eq!(validation.status("test"), ValidationStatus::Stale);
        assert!(validation.text("test").unwrap().starts_with("PREVIOUS REPORT"));
        validation.set_paused(true);
        validation.tick(&egui::Context::default());
        assert!(validation.active.is_none());
        validation.set_paused(false);
        validation.tick(&egui::Context::default());
        assert!(validation.active.is_some());
        validation.set_paused(true);
        assert!(validation.active.as_ref().unwrap().cancel.load(Ordering::Relaxed));
        assert_eq!(validation.queue.len(), 1);
        assert!(validation.queue.front().unwrap().force, "pausing must preserve an explicitly requested fresh check");
        validation.shutdown();
        assert!(validation.queue.is_empty());
    }

    #[test]
    fn a_mod_that_is_one_unreadable_archive_is_reported_as_its_finding() {
        let dir = Temp::new();
        let base = dir.0.join("base");
        std::fs::create_dir_all(base.join("units")).unwrap();
        let archive = dir.0.join("Broken Pack.ufo");
        std::fs::write(&archive, b"not an archive").unwrap();
        let mut module = module("broken", &archive, &[]);
        module.kind = ModKind::Archive;
        let mut issues = Vec::new();
        let result = mount(&base, None, &[&module], Some(&mod_layer("broken")), &mut issues, &AtomicBool::new(false));
        let Err(Mount::Unreadable(issue)) = result else { panic!("an unreadable archive mod is a finding, not an incomplete check") };
        assert_eq!((issue.code.as_str(), issue.path.as_str(), issue.affects_status), ("ARCHIVE_UNREADABLE", "Broken Pack.ufo", true));
        assert!(!issue.message.contains(&dir.0.display().to_string()), "{}", issue.message);
        let report = Report { complete: true, issues: vec![*issue], ..Report::default() };
        assert_eq!(report.status(), Status::Errors);
    }

    #[test]
    fn digests_are_kept_between_launches_and_a_recheck_reads_its_mod_again() {
        let dir = Temp::new();
        let content = dir.0.join("mod");
        std::fs::create_dir_all(content.join("weapons")).unwrap();
        std::fs::write(content.join("weapons/w.tdf"), b"[W]{rendertype=0;}").unwrap();
        let cache = dir.0.join("cache");
        let module = module("test", &content, &[]);
        let roots = vec![(mod_layer("test"), content.clone(), false)];
        let cancel = AtomicBool::new(false);
        let mut validation = ModValidation::new(cache.clone());
        validation.sync(None, vec![module.clone()], false);
        let snap = snapshot(&roots, &cancel).unwrap();
        let first = fingerprint(&snap, &[&module], &validation.memo, &cancel, None).unwrap();
        assert_eq!(validation.memo.lock().unwrap().known.len(), 1);
        validation.shutdown();
        // The next launch: the digest is there without reading the file.
        let validation = ModValidation::new(cache.clone());
        assert_eq!(validation.memo.lock().unwrap().known.len(), 1, "kept in {}", DIGESTS);
        assert_eq!(fingerprint(&snap, &[&module], &validation.memo, &cancel, None).unwrap(), first);
        let mut validation = validation;
        validation.sync(None, vec![module.clone()], false);
        validation.request("test");
        assert!(validation.memo.lock().unwrap().known.is_empty(), "a recheck reads its mod's files again");
    }

    #[test]
    fn baselines_are_kept_by_fingerprint_and_only_the_newest_few() {
        let dir = Temp::new();
        let report = Report { complete: true, ..Report::default() };
        for n in 0..MAX_BASELINES + 3 {
            let print = format!("{n:016x}{}", "0".repeat(48));
            let path = dir.0.join(format!("baseline-{}.json", &print[..16]));
            write_baseline(&path, &print, &report);
            assert!(read_baseline(&path, &print).is_some());
            assert!(read_baseline(&path, &format!("{n:016x}{}", "1".repeat(48))).is_none(), "another content's baseline is not this one's");
            // (Distinct times for the pruning.)
            std::thread::sleep(Duration::from_millis(15));
        }
        let kept = std::fs::read_dir(&dir.0).unwrap().count();
        assert_eq!(kept, MAX_BASELINES);
        let partial = Report { complete: false, ..Report::default() };
        let path = dir.0.join("baseline-partial.json");
        write_baseline(&path, "p", &partial);
        assert!(read_baseline(&path, "p").is_none(), "an incomplete baseline is never used");
    }

    #[test]
    fn copied_reports_include_version_scope_codes_and_utc_time() {
        let mut saved = report("test");
        saved.report.issues.push(Issue {
            severity: Severity::Warning, code: "TEST_MISSING_MODEL".into(), path: "units/test.fbi".into(),
            layer: "test".into(), message: "A referenced model is missing.".into(), suggestion: "Include objects3d/test.3do.".into(),
            affects_status: true, ..Issue::default()
        });
        let text = saved.to_text();
        for expected in ["TA Reborn mod validation", "Version: 1.0", "Original game", "TEST_MISSING_MODEL", "units/test.fbi", "objects3d/test.3do", "UTC"] {
            assert!(text.contains(expected), "{expected} missing from copied report");
        }
        assert_eq!(utc_stamp(0), "1970-01-01 00:00:00");
        assert_eq!(utc_stamp(951_827_696), "2000-02-29 12:34:56");
    }
}
