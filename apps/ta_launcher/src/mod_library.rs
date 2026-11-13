//! The Mods page's library: the catalogue's mods and those installed, as one list to search,
//! filter and order; and, worked out away from the interface, what a mod changes of the game
//! and of the other mods (the files it overrides and those it loses), and the documents it
//! came with.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::JoinHandle;

use ta_ui::i18n::t;
use ta_vfs::Vfs;
use ta_vfs::mods::{ModInfo, ModKind};

use crate::catalogue::{self, Entry, Fit};
use crate::modlist::{ModEntry, ModList};
use crate::validation::ValidationStatus;

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub enum Filter { #[default] All, Installed, Enabled, Updates, PlaysWell }

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub enum Sort { #[default] Spotlight, Name, Newest, Oldest, Smallest, Largest }

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub enum View { #[default] Grid, List }

/// What a mod is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// One of the catalogue's: each remakes the whole game.
    Conversion,
    /// Maps imported on the Maps page.
    MapPack,
    /// A classic TA archive kept as it is (.ufo, .hpi...).
    Archive,
    /// A player's own, or one installed from a package.
    Mod,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Conversion => ta_ui::i18n::n("TOTAL CONVERSION"),
            Self::MapPack => ta_ui::i18n::n("MAP PACK"),
            Self::Archive => ta_ui::i18n::n("TA ARCHIVE"),
            Self::Mod => ta_ui::i18n::n("MOD"),
        }
    }
}

/// One mod of the library: one of the catalogue's, installed or not, or one installed that the
/// catalogue does not name (a map pack, a player's own).
#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub catalogue: Option<&'static Entry>,
    pub installed: bool,
    /// Its place in the catalogue (the spotlight order), or after it.
    order: usize,
}

impl Item {
    pub fn kind(&self, mods: &ModList) -> Kind {
        if self.catalogue.is_some() { return Kind::Conversion; }
        match mods.get(&self.id) {
            Some(entry) if entry.id().starts_with("map-pack-") => Kind::MapPack,
            Some(entry) if entry.is_archive() => Kind::Archive,
            _ => Kind::Mod,
        }
    }
}

/// The catalogue's mods (in its order) and then those installed it does not name (by name).
pub fn items(mods: &ModList) -> Vec<Item> {
    let mut items: Vec<Item> = catalogue::MODS.iter().enumerate().map(|(order, entry)| Item {
        id: entry.id.to_string(), name: entry.name.to_string(), catalogue: Some(entry), installed: mods.get(entry.id).is_some(), order,
    }).collect();
    let after = items.len();
    items.extend(mods.mods.iter().filter(|entry| catalogue::get(entry.id()).is_none()).map(|entry| Item {
        id: entry.id().to_string(), name: entry.info.manifest.name.clone(), catalogue: None, installed: true, order: after,
    }));
    items
}

/// The catalogue release newer than the one installed, if there is one: its tested file (the
/// update downloads, or is imported, as the catalogue's install is).
pub fn update(entry: &Entry, installed: Option<&str>) -> Option<&'static str> {
    let file = entry.file?;
    let installed = installed?.trim().trim_start_matches(['v', 'V']);
    // An independently installed newer release must not be downgraded by
    // UPDATE ALL just because it differs from the curated release.
    (installed != file.version && !crate::update::newer(installed, file.version)).then_some(file.version)
}

/// The catalogue release waiting to be installed over the copy in use, as the rail's badge, the
/// UPDATES filter, a card and UPDATE ALL count it: one the launcher can download ([`update`]'s
/// newer release), over a copy whose version is known. A copy of unknown version (one added from
/// Total Annihilation's folder) waits for nothing: it may be that release or a later one, and the
/// mod's details still offer UPDATE over it. A release the launcher cannot download is the mod's
/// website's to give (the details say a newer one is out): nothing UPDATE ALL could clear.
pub fn waiting_update(entry: &Entry, installed: Option<&str>) -> Option<&'static str> {
    entry.file.filter(|file| file.can_download())?;
    installed.filter(|version| !version.trim().is_empty())?;
    update(entry, installed)
}

/// Search treats dotted and dotless I alike, so translated Turkish tags and English names
/// remain discoverable without changing the text displayed to players.
fn search_fold(text: &str) -> String {
    text.chars().map(|c| if matches!(c, 'İ' | 'ı') { 'i' } else { c }).flat_map(char::to_lowercase).collect()
}

/// Each word can match a different field (for example a title and its creator).
pub fn matches_words<'a>(query: &str, fields: impl IntoIterator<Item = &'a str>) -> bool {
    let query = search_fold(query);
    if query.trim().is_empty() { return true; }
    let text = fields.into_iter().map(search_fold).collect::<Vec<_>>().join("\n");
    query.split_whitespace().all(|word| text.contains(word))
}

/// Whether a catalogue entry matches a search: its name, creators, words, tags and release.
pub fn catalogue_matches(entry: &Entry, query: &str) -> bool {
    let release = entry.release();
    matches_words(query, [entry.id, entry.name, entry.team, entry.pitch, t(entry.pitch), entry.note, t(entry.note), release.version, release.date]
        .into_iter().chain(entry.tags.iter().copied()).chain(entry.tags.iter().map(|tag| t(tag))))
}

/// Whether an installed mod matches a search: its name, id, words and authors.
pub fn installed_matches(entry: &ModEntry, query: &str) -> bool {
    let manifest = &entry.info.manifest;
    matches_words(query, [manifest.name.as_str(), manifest.id.as_str(), manifest.description.as_str(), t(&manifest.description)]
        .into_iter().chain(manifest.authors.iter().map(String::as_str)))
}

/// A catalogue play-test recommendation must not hide problems in this player's copy: an
/// installed one also needs a current, clean check of its own.
pub fn plays_well(entry: &Entry, installed: bool, status: &ValidationStatus) -> bool {
    entry.fit == Fit::Plays && (!installed || matches!(status, ValidationStatus::Ready(ta_data::validation::Status::Clean)))
}

/// What the library needs to know of the player's mods to filter them.
pub struct Facts<'a> {
    pub mods: &'a ModList,
    pub enabled: &'a [String],
    pub status: &'a dyn Fn(&str) -> ValidationStatus,
    /// The mods the player hid from the library (`ta_config::Mods::hidden`).
    pub hidden: &'a [String],
}

impl Facts<'_> {
    fn updatable_version(&self, id: &str) -> Option<&str> { self.mods.updatable_version(id) }
}

/// Whether `item` is one of those the player hid.
pub fn hidden(item: &Item, hidden: &[String]) -> bool {
    hidden.contains(&item.id)
}

/// How many of the library's mods it shows (those not hidden), and how many are hidden (of those
/// it has: a hidden id no longer installed counts for nothing).
pub fn counts(mods: &ModList, hidden: &[String]) -> (usize, usize) {
    let all = items(mods);
    let away = all.iter().filter(|item| self::hidden(item, hidden)).count();
    (all.len() - away, away)
}

/// Whether `item` is shown: in the library's view (`hidden_view`: the mods the player hid, else
/// the others), passing the filter and the search.
pub fn accepts(item: &Item, query: &str, filter: Filter, hidden_view: bool, facts: &Facts<'_>) -> bool {
    if hidden(item, facts.hidden) != hidden_view {
        return false;
    }
    let shown = match filter {
        Filter::All => true,
        Filter::Installed => item.installed,
        Filter::Enabled => facts.enabled.contains(&item.id),
        Filter::Updates => item.catalogue.is_some_and(|entry| waiting_update(entry, facts.updatable_version(&item.id)).is_some()),
        Filter::PlaysWell => item.catalogue.is_some_and(|entry| plays_well(entry, item.installed, &(facts.status)(&item.id))),
    };
    shown && match (item.catalogue, facts.mods.get(&item.id)) {
        (Some(entry), installed) => catalogue_matches(entry, query) || installed.is_some_and(|installed| installed_matches(installed, query)),
        (None, Some(installed)) => installed_matches(installed, query),
        (None, None) => false,
    }
}

/// `items` in the chosen order: unknown dates and sizes last either way.
pub fn sort(items: &mut [Item], sort: Sort) {
    fn known_first<T: Ord>(a: Option<T>, b: Option<T>, descending: bool) -> std::cmp::Ordering {
        match (a, b) {
            (Some(a), Some(b)) => if descending { b.cmp(&a) } else { a.cmp(&b) },
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    }
    let name = |item: &Item| item.name.to_lowercase();
    let date = |item: &Item| item.catalogue.map(|entry| entry.release().date).filter(|date| !date.is_empty());
    let bytes = |item: &Item| item.catalogue.and_then(|entry| entry.release().bytes);
    items.sort_by(|a, b| {
        let order = match sort {
            Sort::Spotlight => a.order.cmp(&b.order),
            Sort::Name => std::cmp::Ordering::Equal,
            Sort::Newest | Sort::Oldest => known_first(date(a), date(b), sort == Sort::Newest),
            Sort::Smallest | Sort::Largest => known_first(bytes(a), bytes(b), sort == Sort::Largest),
        };
        order.then_with(|| name(a).cmp(&name(b))).then_with(|| a.id.cmp(&b.id))
    });
}

/// What one mod changes: of the game and of the other mods on, and what the mods after it
/// change of it. Its documents too.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    /// How many files it has.
    pub files: usize,
    /// The sources whose files it replaces (the game's, Reborn's, a mod's before it), with how
    /// many of them.
    pub overrides: Vec<(String, usize)>,
    /// The mods after it that replace files of its own, with how many.
    pub loses_to: Vec<(String, usize)>,
    /// The original game's archives it takes the place of, as the install names them: with it
    /// on, they are not read at all (`ta_vfs::mods::replaced_archives`).
    pub replaces: Vec<String>,
    /// Its documents: read-me files and the like in its folder.
    pub docs: Vec<PathBuf>,
}

/// What to work out the changes of a mod against.
#[derive(Clone)]
pub struct ChangesRequest {
    pub game_dir: Option<PathBuf>,
    pub builtin: Option<PathBuf>,
    /// Every installed mod.
    pub mods: Vec<ModInfo>,
    /// The mods on, in the order they load (resolved).
    pub order: Vec<String>,
    pub target: String,
}

impl ChangesRequest {
    /// What the result is for: a different order, mod or game is another question.
    pub fn key(&self) -> String {
        let relevant: Vec<_> = self.mods.iter()
            .filter(|info| info.manifest.id == self.target || self.order.contains(&info.manifest.id))
            .map(|info| (&info.path, &info.kind, &info.manifest)).collect();
        format!("{}|{:?}|{:?}|{:?}|{:?}", self.target, self.game_dir, self.builtin, self.order, relevant)
    }
}


fn mount(vfs: &mut Vfs, info: &ModInfo) -> ta_vfs::Result<()> {
    let renames = ta_vfs::Renames::new(&info.manifest.rename);
    let layer = format!("mod:{}", info.manifest.id);
    match info.kind {
        ModKind::Archive => vfs.mount_archive_renamed(&info.path, &layer, &renames),
        ModKind::Directory => vfs.mount_mod_dir_renamed(&info.path, &layer, &renames),
    }
}

/// Works out what `request.target` changes: each of its files is looked up in the game as it
/// would load without it, and counted against the source that has it (before it: replaced by
/// it; after it: replacing it). A mod that is off counts as loaded last.
pub fn changes(request: &ChangesRequest) -> Result<Changes, String> {
    let target = request.mods.iter().find(|info| info.manifest.id == request.target).ok_or_else(|| t("The mod is no longer installed.").to_string())?;
    let mut alone = Vfs::new();
    mount(&mut alone, target).map_err(|error| error.to_string())?;
    // (The game reads what is in folders: a read-me beside them is no game data.)
    let files: Vec<String> = alone.list("", None, true).into_iter().filter(|path| path.contains('/') && !path.starts_with("docs/")).collect();
    // The sources before it (the game, Reborn's own, the mods loaded earlier) and those after
    // it, each stack mounted by itself: a file of its own may replace one before it and be
    // replaced by one after it.
    let mut after = Vfs::new();
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    // (A mod that is off counts as loaded last.)
    let place = request.order.iter().position(|id| *id == request.target).unwrap_or(request.order.len());
    let earlier: Vec<&ModInfo> = request.order.iter().take(place)
        .filter_map(|id| request.mods.iter().find(|info| info.manifest.id == *id)).collect();
    // The game before it as the game mounts it: what the mods before it replace of the install
    // left out.
    let mounted = ta_vfs::mods::mount(request.game_dir.as_deref(), request.builtin.as_deref(), &earlier, &|m| format!("mod:{}", m.manifest.id));
    if request.game_dir.is_some() && mounted.install.is_ok() {
        names.insert("base".into(), t("Total Annihilation").to_string());
    }
    if request.builtin.is_some() && mounted.builtin.is_ok() {
        names.insert(ta_vfs::BUILTIN_LAYER.into(), "TA Reborn".into());
    }
    if mounted.mods.is_ok() {
        names.extend(earlier.iter().map(|info| (format!("mod:{}", info.manifest.id), info.manifest.name.clone())));
    }
    let before = mounted.vfs;
    for id in request.order.iter().skip(place + 1) {
        let Some(info) = request.mods.iter().find(|info| info.manifest.id == *id) else { continue };
        if mount(&mut after, info).is_ok() { names.insert(format!("mod:{id}"), info.manifest.name.clone()); }
    }
    let mut overrides: BTreeMap<String, usize> = BTreeMap::new();
    let mut loses_to: BTreeMap<String, usize> = BTreeMap::new();
    for file in &files {
        for (stack, counts) in [(&before, &mut overrides), (&after, &mut loses_to)] {
            if let Some(name) = stack.origin(file).and_then(|origin| names.get(&origin.layer)) {
                *counts.entry(name.clone()).or_default() += 1;
            }
        }
    }
    let ranked = |counts: BTreeMap<String, usize>| {
        let mut counts: Vec<_> = counts.into_iter().collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        counts
    };
    let replaces = request.game_dir.as_deref().map(|dir| ta_vfs::mods::replaced_archives(dir, &[target])).unwrap_or_default();
    Ok(Changes { files: files.len(), overrides: ranked(overrides), loses_to: ranked(loses_to), replaces, docs: docs(target) })
}

/// A mod folder's documents: its read-me files and those in its `docs` folder (a few).
fn docs(target: &ModInfo) -> Vec<PathBuf> {
    if target.kind != ModKind::Directory { return Vec::new(); }
    let mut found = Vec::new();
    for folder in [target.path.clone(), target.path.join("docs")] {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        let mut here: Vec<PathBuf> = entries.flatten().map(|entry| entry.path())
            .filter(|path| path.is_file() && path.extension().and_then(|e| e.to_str()).is_some_and(|e| ta_vfs::package::DOCUMENT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())))
            .collect();
        here.sort();
        found.extend(here);
    }
    found.truncate(8);
    found
}

type Answer = (String, Result<Changes, String>);

/// The changes of the mods asked about, worked out one at a time on a worker (the newest ask
/// next), and kept for those asked about again.
#[derive(Default)]
pub struct ChangesWork {
    known: BTreeMap<String, Result<Changes, String>>,
    order: VecDeque<String>,
    running: Option<(String, mpsc::Receiver<Answer>, JoinHandle<()>)>,
    waiting: Option<ChangesRequest>,
    generation: u64,
    running_generation: u64,
}

impl ChangesWork {
    /// What `request` came to, if it is worked out; asks for it otherwise.
    pub fn get(&mut self, request: impl FnOnce() -> ChangesRequest, key: &str) -> Option<&Result<Changes, String>> {
        if !self.known.contains_key(key) && (self.running_generation != self.generation || self.running.as_ref().is_none_or(|(running, ..)| running != key))
            && self.waiting.as_ref().is_none_or(|waiting| waiting.key() != key) {
            self.waiting = Some(request());
        }
        self.known.get(key)
    }

    /// Takes in an answer and starts the next question.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let current = self.running_generation == self.generation;
        if let Some((key, receive, _)) = &self.running {
            match receive.try_recv() {
                Ok((_, answer)) => {
                    let key = key.clone();
                    if let Some((_, _, worker)) = self.running.take() { let _ = worker.join(); }
                    if current {
                        self.known.insert(key.clone(), answer);
                        self.order.push_back(key);
                    }
                    while self.order.len() > 32 { if let Some(old) = self.order.pop_front() { self.known.remove(&old); } }
                    ctx.request_repaint();
                }
                Err(mpsc::TryRecvError::Empty) => { ctx.request_repaint_after(std::time::Duration::from_millis(60)); return; }
                Err(mpsc::TryRecvError::Disconnected) => {
                    let key = key.clone();
                    self.running = None;
                    if current { self.known.insert(key, Err(t("The mod could not be read.").to_string())); }
                }
            }
        }
        let Some(request) = self.waiting.take() else { return };
        let key = request.key();
        let (send, receive) = mpsc::channel();
        let answer_key = key.clone();
        match std::thread::Builder::new().name("mod changes".into()).spawn(move || {
            let answer = std::panic::catch_unwind(|| changes(&request)).unwrap_or_else(|_| Err(t("The mod could not be read.").to_string()));
            let _ = send.send((answer_key, answer));
        }) {
            Ok(worker) => { self.running = Some((key, receive, worker)); self.running_generation = self.generation; ctx.request_repaint_after(std::time::Duration::from_millis(60)); }
            Err(error) => { self.known.insert(key, Err(error.to_string())); }
        }
    }

    /// Forgets what was worked out (the mods or the game changed).
    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.known.clear();
        self.order.clear();
        self.waiting = None;
    }

    /// Nothing is being worked out or waiting.
    pub fn idle(&self) -> bool { self.running.is_none() && self.waiting.is_none() }
}

impl Drop for ChangesWork {
    fn drop(&mut self) { if let Some((_, _, worker)) = self.running.take() { let _ = worker.join(); } }
}

/// The page's state: what is looked for and how it is shown, the chosen mod.
#[derive(Default)]
pub struct Library {
    pub query: String,
    pub filter: Filter,
    pub sort: Sort,
    pub view: View,
    /// The chosen mod's id.
    pub selected: String,
    /// On a narrow page, the chosen mod's details are open in place of the library.
    pub details_open: bool,
    /// The mod whose card had the keys last frame.
    pub focused: Option<String>,
    /// The library shows the mods the player hid (the toolbar's HIDDEN), and the one chosen
    /// before it did (chosen again when it no longer does).
    pub hidden_view: bool,
    pub chosen_before_hidden: Option<String>,
    reveal: Option<String>,
    pub changes: ChangesWork,
    /// The mod being dragged in the load order: its id (the order may change under the drag, an
    /// install finishing), and where it was taken from.
    pub dragging: Option<(String, usize)>,
    /// A mod just moved in the load order, to bring into view.
    pub order_reveal: Option<String>,
}

impl Library {
    /// The items to show, in order.
    pub fn visible(&self, facts: &Facts<'_>) -> Vec<Item> {
        let mut shown: Vec<Item> = items(facts.mods).into_iter().filter(|item| accepts(item, &self.query, self.filter, self.hidden_view, facts)).collect();
        sort(&mut shown, self.sort);
        shown
    }

    /// Shows the mods the player hid, or the library again (its choice then as it was before).
    pub fn show_hidden(&mut self, on: bool) {
        if on == self.hidden_view { return; }
        self.hidden_view = on;
        if on {
            self.chosen_before_hidden = Some(self.selected.clone());
        } else if let Some(before) = self.chosen_before_hidden.take().filter(|before| !before.is_empty()) {
            self.choose(&before);
        }
    }

    /// Keeps the choice among the items shown (the first, when it is not).
    pub fn reconcile(&mut self, visible: &[Item]) {
        if !visible.iter().any(|item| item.id == self.selected) {
            self.selected = visible.first().map_or(String::new(), |item| item.id.clone());
            if !self.selected.is_empty() { self.reveal = Some(self.selected.clone()); }
        }
    }

    /// Chooses `id` and brings it into view (a choice made other than by clicking it).
    pub fn choose(&mut self, id: &str) {
        self.selected = id.to_string();
        self.reveal = Some(id.to_string());
    }

    pub fn take_reveal(&mut self) -> Option<String> { self.reveal.take() }

    pub fn filters_set(&self) -> bool { !self.query.is_empty() || self.filter != Filter::All }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_analysis_keys_include_renames_and_builtin_content() {
        let root = std::env::temp_dir().join(format!("ta-mod-qa-change-key-{}", std::process::id()));
        let target = folder(&root, "test", &["units/tank.fbi"]);
        let request = ChangesRequest { game_dir: None, builtin: Some(root.join("builtin")),
            mods: vec![target], order: vec!["test".into()], target: "test".into() };
        let mut changed = request.clone();
        changed.mods[0].manifest.rename.insert("units".into(), "features".into());
        assert_ne!(request.key(), changed.key(), "the same path can mount a different virtual layout");
        let mut changed = request.clone();
        changed.builtin = Some(root.join("other-builtin"));
        assert_ne!(request.key(), changed.key());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clearing_analysis_cannot_publish_an_older_workers_answer() {
        let (send, receive) = mpsc::channel();
        let key = "old-content".to_string();
        let mut work = ChangesWork::default();
        work.running = Some((key.clone(), receive, std::thread::spawn(|| {})));
        work.clear();
        send.send((key.clone(), Err("outdated answer".into()))).unwrap();
        work.poll(&egui::Context::default());
        assert!(!work.known.contains_key(&key), "a cleared inventory must not regain stale analysis");
    }

    #[test]
    fn mod_names_cannot_relabel_original_or_builtin_overrides() {
        let root = std::env::temp_dir().join(format!("ta-mod-qa-origin-{}", std::process::id()));
        let original = root.join("original");
        std::fs::create_dir_all(original.join("units")).unwrap();
        std::fs::write(original.join("units/original.fbi"), b"original").unwrap();
        for (id, label) in [("base", "Total Annihilation"), ("reborn", "TA Reborn")] {
            // (Ids no mod read from its folder may take now, `ta_vfs::mods::RESERVED_IDS`: one an
            // older version read, made by hand here, relabels nothing either.)
            let mut named = folder(&root, &format!("{id}-mod"), &["units/mod.fbi"]);
            (named.manifest.id, named.manifest.name) = (id.into(), format!("Mod {id}"));
            let target = folder(&root, "target", &["units/original.fbi", "units/mod.fbi"]);
            let request = ChangesRequest { game_dir: (id == "base").then(|| original.clone()),
                builtin: (id == "reborn").then(|| original.clone()), mods: vec![named, target],
                order: vec![id.into(), "target".into()], target: "target".into() };
            let result = changes(&request).unwrap();
            assert_eq!(result.overrides, [(format!("Mod {id}"), 1), (label.into(), 1)]);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    fn folder(root: &std::path::Path, id: &str, files: &[&str]) -> ModInfo {
        let dir = root.join(id);
        for file in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, id).unwrap();
        }
        std::fs::write(dir.join("mod.toml"), format!("id = \"{id}\"\nname = \"Mod {id}\"\n")).unwrap();
        ta_vfs::mods::scan(root).0.into_iter().find(|info| info.manifest.id == id).unwrap()
    }

    #[test]
    fn changes_count_what_a_mod_replaces_and_what_replaces_it() {
        let root = std::env::temp_dir().join(format!("ta-mod-changes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let game = root.join("game");
        std::fs::create_dir_all(game.join("units")).unwrap();
        std::fs::write(game.join("units/armcom.fbi"), b"base").unwrap();
        let mods = root.join("mods");
        let first = folder(&mods, "first", &["units/armcom.fbi", "units/new.fbi", "readme.txt"]);
        let second = folder(&mods, "second", &["units/new.fbi", "maps/moon.ota"]);
        let third = folder(&mods, "third", &["units/new.fbi"]);
        let request = |order: &[&str], target: &str| ChangesRequest {
            game_dir: None, builtin: Some(game.clone()), mods: vec![first.clone(), second.clone(), third.clone()],
            order: order.iter().map(|id| id.to_string()).collect(), target: target.into(),
        };
        let changes_of_second = changes(&request(&["first", "second", "third"], "second")).unwrap();
        assert_eq!(changes_of_second.overrides, [("Mod first".to_string(), 1)], "it replaces what the mod before it has");
        assert_eq!(changes_of_second.loses_to, [("Mod third".to_string(), 1)], "and loses what the mod after it has");
        let changes_of_first = changes(&request(&["first", "second"], "first")).unwrap();
        assert_eq!(changes_of_first.overrides, [("TA Reborn".to_string(), 1)]);
        assert_eq!(changes_of_first.loses_to, [("Mod second".to_string(), 1)]);
        assert_eq!(changes_of_first.docs.len(), 1, "its read-me is found");
        let off = changes(&request(&["first"], "third")).unwrap();
        assert_eq!(off.overrides, [("Mod first".to_string(), 1)], "a mod that is off counts as loaded last");
        assert!(off.loses_to.is_empty());
        assert_ne!(request(&["first"], "third").key(), request(&["second"], "third").key());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_library_holds_the_catalogue_and_the_installed_mods_and_filters_them() {
        let root = std::env::temp_dir().join(format!("ta-mod-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        folder(&root, "map-pack-1", &["maps/moon.tnt"]);
        folder(&root, "navy", &["units/ship.fbi"]);
        let manifest = root.join("totalmayhem/mod.toml");
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        std::fs::write(manifest, "id = \"totalmayhem\"\nname = \"Total Mayhem\"\nversion = \"11.2.0\"\n").unwrap();
        let list = ModList::scan(vec![crate::modlist::ModFolder { label: "User folder", path: root.clone(), exists: true }], &[]);
        let all = items(&list);
        assert_eq!(all.len(), catalogue::MODS.len() + 2, "the catalogue's, then the player's map pack and mod");
        let enabled = vec!["navy".to_string()];
        let status = |_: &str| ValidationStatus::Unknown;
        let facts = Facts { mods: &list, enabled: &enabled, status: &status, hidden: &[] };
        let shown = |filter: Filter, query: &str| {
            let library = Library { filter, query: query.into(), sort: Sort::Name, ..Default::default() };
            library.visible(&facts).into_iter().map(|item| item.id).collect::<Vec<_>>()
        };
        assert_eq!(shown(Filter::Installed, ""), ["map-pack-1", "navy", "totalmayhem"]);
        assert_eq!(shown(Filter::Enabled, ""), ["navy"]);
        assert_eq!(shown(Filter::Updates, ""), ["totalmayhem"], "an older release than the catalogue's tested one");
        assert_eq!(shown(Filter::All, "mod navy"), ["navy"], "every word must match");
        assert!(!shown(Filter::PlaysWell, "").contains(&"totalmayhem".to_string()), "an installed copy needs a clean check of its own");
        assert_eq!(all.iter().find(|item| item.id == "map-pack-1").unwrap().kind(&list), Kind::MapPack);
        let mut sorted = all.clone();
        sort(&mut sorted, Sort::Largest);
        assert!(sorted[0].catalogue.is_some() && sorted.last().unwrap().catalogue.is_none(), "unknown sizes last");
        let _ = std::fs::remove_dir_all(root);
    }

    /// Hidden mods leave the library and its counts, whatever they are (an offer of the
    /// catalogue's, or a mod installed and on); the hidden view lists only them, searched and
    /// filtered as the library is; and leaving it chooses again what was chosen before.
    #[test]
    fn hidden_mods_leave_the_library_and_its_counts_and_the_hidden_view_lists_them() {
        let root = std::env::temp_dir().join(format!("ta-mod-library-hidden-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        folder(&root, "navy", &["units/ship.fbi"]);
        let list = ModList::scan(vec![crate::modlist::ModFolder { label: "User folder", path: root.clone(), exists: true }], &[]);
        let enabled = vec!["navy".to_string()];
        let hidden = vec!["navy".to_string(), "totalmayhem".to_string(), "gone-mod".to_string()];
        let status = |_: &str| ValidationStatus::Unknown;
        let facts = Facts { mods: &list, enabled: &enabled, status: &status, hidden: &hidden };
        let all = items(&list).len();
        assert_eq!(counts(&list, &hidden), (all - 2, 2), "a hidden id no longer installed counts for nothing");
        assert_eq!(counts(&list, &[]), (all, 0));
        let mut library = Library { sort: Sort::Name, ..Default::default() };
        let ids = |library: &Library| library.visible(&facts).into_iter().map(|item| item.id).collect::<Vec<_>>();
        let shown = ids(&library);
        assert_eq!(shown.len(), all - 2);
        assert!(!shown.contains(&"navy".to_string()) && !shown.contains(&"totalmayhem".to_string()), "hidden, even installed and on");
        library.filter = Filter::Enabled;
        assert!(ids(&library).is_empty(), "the library's filters leave hidden mods out too");
        library.filter = Filter::All;
        library.selected = "escalation".into();
        library.show_hidden(true);
        assert_eq!(ids(&library), ["navy", "totalmayhem"], "the hidden view lists only them");
        library.query = "mayhem".into();
        assert_eq!(ids(&library), ["totalmayhem"], "searched as the library is");
        library.filter = Filter::Enabled;
        library.query.clear();
        assert_eq!(ids(&library), ["navy"], "and filtered: a hidden mod that is on is still on");
        library.filter = Filter::All;
        library.reconcile(&library.visible(&facts));
        assert_eq!(library.selected, "navy");
        library.show_hidden(false);
        assert_eq!(library.selected, "escalation", "leaving it chooses again what was chosen before");
        assert_eq!(enabled, ["navy"], "nothing is turned off");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn release_orders_keep_unknown_values_last_in_both_directions() {
        let list = ModList::scan(Vec::new(), &[]);
        for (order, first) in [(Sort::Newest, "escalation"), (Sort::Oldest, "battletech"), (Sort::Smallest, "secretforces"), (Sort::Largest, "escalation")] {
            let mut shown = items(&list);
            sort(&mut shown, order);
            assert_eq!(shown[0].id, first, "{order:?}");
            let known: Vec<bool> = shown.iter().map(|item| {
                let release = item.catalogue.unwrap().release();
                if matches!(order, Sort::Newest | Sort::Oldest) { !release.date.is_empty() } else { release.bytes.is_some() }
            }).collect();
            assert!(!known.windows(2).any(|pair| !pair[0] && pair[1]), "unknowns follow known values: {order:?}");
        }
        let mut shown = items(&list);
        sort(&mut shown, Sort::Spotlight);
        assert_eq!(shown[0].id, "escalation", "the newest conversion, painted and drawn whole, is the spotlight");
        assert!(catalogue::get("finalfrontier").unwrap().release().date.is_empty(), "do not invent unknown release dates");
    }

    #[test]
    fn mod_search_matches_words_across_fields_and_ignores_case_and_spacing() {
        assert!(matches_words("  ARM  ÄNDERUNG ", ["Arm Expansion", "Kleine Änderung"]));
        assert!(matches_words("creator aircraft", ["New aircraft", "A. Creator"]));
        assert!(matches_words(" ", ["Anything"]));
        assert!(!matches_words("navy aircraft", ["New aircraft", "A. Creator"]));
    }

    #[test]
    fn mod_search_matches_turkish_dotted_and_dotless_i_in_queries_and_tags() {
        assert!(matches_words("birim ırk", ["500 BİRİM", "YENİ IRK"]));
        assert!(matches_words("BİRİM IRK", ["500 birim", "Yeni ırk"]));
        assert!(matches_words("ıron", ["IRON Legion"]));
        assert!(!matches_words("birim deniz", ["500 BİRİM", "YENİ IRK"]));
    }

    #[test]
    fn installed_search_matches_manifest_metadata() {
        let manifest = toml::from_str(r#"id = "map-pack"
name = "Island Battles"
authors = ["Builder"]
description = "Naval maps"
"#).unwrap();
        let entry = ModEntry {
            info: ta_vfs::mods::ModInfo { manifest, path: "map-pack".into(), kind: ta_vfs::mods::ModKind::Directory },
            folder: 0,
        };
        assert!(installed_matches(&entry, "builder naval"));
        assert!(installed_matches(&entry, "MAP-PACK"));
        assert!(!installed_matches(&entry, "desert"));
        assert_eq!(entry.info.manifest.version, "", "a mod that does not say its version has none");
    }

    #[test]
    fn plays_well_needs_the_review_and_a_clean_check_of_an_installed_copy() {
        use ta_data::validation::Status;
        let entry = catalogue::get("totalmayhem").unwrap();
        assert!(plays_well(entry, false, &ValidationStatus::Unknown), "uninstalled releases keep the project review");
        assert!(!plays_well(entry, true, &ValidationStatus::Ready(Status::Issues)));
        assert!(plays_well(entry, true, &ValidationStatus::Ready(Status::Clean)));
        assert!(!plays_well(catalogue::get("secretforces").unwrap(), true, &ValidationStatus::Ready(Status::Clean)), "a clean check never overrides known gaps");
        assert_eq!(update(entry, Some("11.2.0")), Some("11.3.0"));
        assert_eq!(update(entry, Some("11.3.0")), None);
        assert_eq!(update(entry, Some("v11.3.0")), None);
        assert_eq!(update(entry, Some("11.4.0")), None, "updates never downgrade a newer manual install");
        assert_eq!(update(entry, Some("unverified")), Some("11.3.0"));
        assert_eq!(update(entry, None), None, "nothing to update when it is not installed");
        // What waits, as the badge and UPDATE ALL count it: a downloadable release over a copy of
        // a known version (one added from TA's folder has none: it waits for nothing).
        assert_eq!(waiting_update(entry, Some("11.2.0")), Some("11.3.0"));
        assert_eq!(waiting_update(entry, Some("")), None);
        assert_eq!(update(entry, Some("")), Some("11.3.0"), "the details still offer it");
        let manual = catalogue::MODS.iter().find(|e| e.file.is_some_and(|f| !f.can_download())).expect("a release downloaded by hand");
        assert_eq!(waiting_update(manual, Some("0.1")), None, "UPDATE ALL could never clear it");
    }

    #[test]
    fn catalogue_updates_do_not_replace_a_later_beta_or_stable_release() {
        let entry = catalogue::get("devolution").unwrap();
        for installed in ["5.1 Beta 2", "5.1 Beta 10", "5.1", "5.1.0"] {
            assert_eq!(update(entry, Some(installed)), None, "must keep the newer installed {installed}");
        }
        assert_eq!(update(entry, Some("5.1 Beta 0")), Some("5.1 Beta 1"));
        assert_eq!(update(entry, Some("5.0")), Some("5.1 Beta 1"));
    }
}
