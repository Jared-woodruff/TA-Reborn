//! Installed map discovery and bounded, map-only imports. File work and pictures run
//! away from the UI thread; original game files are never changed.
use std::collections::{BTreeMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use ta_formats::{image::Palette, tdf, tnt::TntMap};
use ta_vfs::{Vfs, mods::{ModInfo, ModKind, ModManifest}, package};
use ta_ui::i18n::n;
use crate::library::{self, Picture, Pictures};

const MAX_QUEUED_IMPORTS: usize = 32;
/// Bumped when the pictures are drawn differently, so that those cached are drawn again.
const PICTURE_VERSION: u32 = 1;
/// The longer side of a card's picture, and of the details pane's, in pixels.
const CARD_SIDE: u32 = 320;
const DETAILS_SIDE: u32 = 960;
/// The menus' unit of a map's size ("12 × 12"): 512 pixels, 32 cells.
const UNIT_CELLS: u32 = 32;

#[derive(Clone, Debug)]
pub enum Action {
    Rescan, Browse, Import(PathBuf), Toggle(String), Favourite(String), Edit(String), Play(String), Skirmish(String), New, Random,
    /// The Mods page, at the mod or map pack (by layer) that supplies a map.
    ManageSource(String),
    /// An imported map pack (by layer) to the Recycle Bin, after asking.
    RemovePack(String),
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub enum Filter { #[default] All, Available, Unavailable, Campaign }

/// How many players a map is for.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub enum Players { #[default] Any, Two, Four, Many }

/// A map's size by its longer side, in the menus' units: up to 8, 12, 20, and beyond.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub enum Size { #[default] Any, Small, Medium, Large, Huge }

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub enum Sort { #[default] Name, Players, Size, Recent, Source }

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub enum View { #[default] Grid, List }

impl Players {
    fn accepts(self, players: usize) -> bool {
        match self { Self::Any => true, Self::Two => players == 2, Self::Four => (3..=4).contains(&players), Self::Many => players >= 5 }
    }
}

impl Size {
    pub fn of((width, height): (u32, u32)) -> Self {
        match width.max(height) { 0..=8 => Self::Small, 9..=12 => Self::Medium, 13..=20 => Self::Large, _ => Self::Huge }
    }
}

#[derive(Clone)]
pub struct Entry {
    pub key: String,
    pub stem: String,
    pub title: String,
    /// The description without the size its designer typed at its start (as the game shows it).
    pub blurb: String,
    pub planet: String,
    pub size: String,
    /// The size typed in the map's header or description, in the menus' units: what the library
    /// goes by until it has read the terrain's own.
    pub typed: Option<(u32, u32)>,
    pub players: usize,
    /// Where the players start, in world pixels, slot by slot.
    pub starts: Vec<(f32, f32)>,
    pub source: String,
    pub layer: String,
    pub path: PathBuf,
    pub active: bool,
    pub shadowed: bool,
    pub campaign: bool,
    pub min_wind: f64,
    pub max_wind: f64,
    pub tidal: f64,
    pub gravity: f64,
    pub lava: bool,
    /// What the sea does to what goes in it each second (an acid sea), or 0.
    pub water_damage: f64,
    /// The identity of the map's picture: its terrain's file, the palette it is painted in and
    /// how pictures are drawn. Cached pictures go by it.
    pub picture: String,
    palette: Arc<Palette>,
    /// What a search looks in, in lower case.
    search: String,
    vfs: Arc<Vfs>,
}

impl Entry {
    pub fn available(&self, preferences: &ta_config::maps::Maps) -> bool {
        self.active && !self.shadowed && !self.campaign && self.players >= 2 && preferences.is_enabled(&self.layer, &self.stem)
    }
    pub fn status(&self, preferences: &ta_config::maps::Maps) -> &'static str {
        if !self.active { n("SOURCE DISABLED") }
        else if self.shadowed { n("OVERRIDDEN") }
        else if self.campaign { n("CAMPAIGN MAP") }
        else if self.players < 2 { n("EDITOR ONLY") }
        else if !preferences.is_enabled(&self.layer, &self.stem) { n("HIDDEN") }
        else { n("AVAILABLE") }
    }
    /// An imported map pack, which the library can remove.
    pub fn pack(&self) -> bool { self.layer.starts_with("map-pack-") }
    /// One of TA Reborn's own maps (`assets/reborn/maps`, mounted over the game).
    pub fn reborn(&self) -> bool { self.layer == ta_vfs::BUILTIN_LAYER }
    /// Where the map stands in the library: the player's favourites first, then TA Reborn's own
    /// maps, then the rest.
    pub fn group(&self, preferences: &ta_config::maps::Maps) -> Group {
        if preferences.is_favourite(&self.stem) { Group::Favourite } else if self.reborn() { Group::Reborn } else { Group::Other }
    }
}

/// The library's sections, in their order (see [`Entry::group`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group { Favourite, Reborn, Other }

/// Cells to the menus' units.
pub fn units((width, height): (u32, u32)) -> (u32, u32) {
    (width.div_ceil(UNIT_CELLS).max(1), height.div_ceil(UNIT_CELLS).max(1))
}

#[derive(Clone)]
pub struct Request {
    pub game_dir: Option<PathBuf>,
    pub builtin: Option<PathBuf>,
    pub mods: Vec<ModInfo>,
    pub enabled: Vec<String>,
}

#[derive(Default)]
pub struct Catalogue { pub entries: Vec<Entry>, pub warnings: Vec<String>, palette: Palette }

pub struct Library {
    pub catalogue: Catalogue,
    pub query: String,
    pub filter: Filter,
    pub source: String,
    pub players: Players,
    pub size: Size,
    /// A world ("Moon"), or every world.
    pub world: String,
    pub favourites: bool,
    pub sort: Sort,
    pub view: View,
    pub selected: String,
    pub feedback: Option<(String, bool)>,
    pub needs_scan: bool,
    /// The maps' mounted game and mods let go of ([`Library::release`]), until the next scan.
    released: bool,
    pub loaded_for: Option<String>,
    /// Where cards' pictures are kept between runs.
    pub cache: Option<PathBuf>,
    /// When each map (by its name in lower case) was last played in this profile, in seconds
    /// since 1970.
    pub played: BTreeMap<String, u64>,
    pub thumbs: Pictures,
    pub details: Pictures,
    /// On a narrow page, the selected map's details are open in place of the library.
    pub details_open: bool,
    /// The map whose card had the keys last frame: the selection follows the keys as they move.
    pub focused: Option<String>,
    notice: Option<(String, bool)>,
    history: Option<mpsc::Receiver<BTreeMap<String, u64>>>,
    /// The profile the play history was read for (`Some(None)`: no profile), or `None` to read it.
    history_for: Option<Option<String>>,
    visible: Option<(u64, Vec<usize>)>,
    generation: u64,
    reveal_selected: Option<String>,
    job: Option<Job>,
    imported: VecDeque<String>,
    pending_imports: VecDeque<(PathBuf, PathBuf)>,
    import_errors: Vec<String>,
}

impl Default for Library {
    fn default() -> Self {
        Self { catalogue: Catalogue::default(), query: String::new(), filter: Filter::default(), source: String::new(), players: Players::default(),
            size: Size::default(), world: String::new(), favourites: false, sort: Sort::default(), view: View::default(), selected: String::new(),
            feedback: None, needs_scan: false, released: false, loaded_for: None, cache: None, played: BTreeMap::new(),
            thumbs: Pictures::new("map cards", 2, 192), details: Pictures::new("map details", 1, 6), details_open: false, focused: None, notice: None,
            history: None, history_for: None, visible: None, generation: 0, reveal_selected: None, job: None, imported: VecDeque::new(),
            pending_imports: VecDeque::new(), import_errors: Vec::new() }
    }
}

struct Job { receive: mpsc::Receiver<Result<Completed, String>>, worker: JoinHandle<()>, kind: JobKind }
enum JobKind { Scan, Import }
impl JobKind {
    fn label(&self) -> &'static str {
        match self {
            Self::Scan => n("Looking for installed maps…"),
            Self::Import => n("Checking and importing maps…"),
        }
    }
}
enum Completed { Scan(Box<Catalogue>), Import(String, usize) }

impl Library {
    /// Discovery or an import is under way (pictures and play history are not counted: they
    /// hold nothing up).
    pub fn busy(&self) -> bool { self.job.is_some() || !self.pending_imports.is_empty() }
    pub fn importing(&self) -> bool { !self.pending_imports.is_empty() || self.job.as_ref().is_some_and(|job| matches!(job.kind, JobKind::Import)) }
    /// Everything the page shows has come in (for captures): the maps, their pictures and the
    /// selected map's large one.
    pub fn ready(&self) -> bool {
        self.loaded_for.is_some() && !self.needs_scan && !self.busy() && self.history.is_none() && self.thumbs.idle() && self.details.idle()
            && self.catalogue.entries.iter().find(|entry| entry.key == self.selected)
                .is_none_or(|entry| self.details.has(&entry.picture) || self.details.failed(&entry.picture))
    }
    /// Wait only at application shutdown, then collect a completed import before the
    /// profile's settings are flushed. A closed launcher must not abandon publication (the
    /// import under way); those not started are dropped (closing asked first).
    pub fn finish(&mut self, ctx: &egui::Context) -> Vec<String> {
        // (Imports not started yet are dropped: the one under way finishes, the window does not
        // wait minutes for the rest.)
        if !self.pending_imports.is_empty() {
            log::info!("closing: {} map imports not started are dropped", self.pending_imports.len());
            self.pending_imports.clear();
        }
        while self.busy() {
            self.poll(ctx);
            if self.busy() { std::thread::sleep(std::time::Duration::from_millis(10)); }
        }
        self.imported.drain(..).collect()
    }
    pub fn status(&self) -> Option<&'static str> { self.job.as_ref().map(|job| job.kind.label()).or_else(|| self.importing().then(|| JobKind::Import.label())) }
    fn start(&mut self, kind: JobKind, work: impl FnOnce() -> Result<Completed> + Send + 'static) {
        if self.job.is_some() { return; }
        let (send, receive) = mpsc::channel();
        match std::thread::Builder::new().name("map library".into()).spawn(move || { let _ = send.send(work().map_err(|error| format!("{error:#}"))); }) {
            Ok(worker) => self.job = Some(Job { receive, worker, kind }),
            Err(error) => {
                log::warn!("could not start map operation: {error}");
                self.failed(kind, ta_ui::i18n::t("The map operation stopped unexpectedly. Please try again.").into());
            }
        }
    }
    /// Lets go of the mounted game and mods the catalogue's maps are read from, and of the
    /// pictures still waiting to be made from them: on Windows a mod's archive cannot be
    /// replaced or removed while it is open, so an install or a removal asks for this first
    /// (a visit to the page held them all session). The pictures made stay; the maps are looked
    /// for again on the next visit.
    pub fn release(&mut self) {
        if self.released { return; }
        self.released = true;
        let empty = Arc::new(Vfs::new());
        for entry in &mut self.catalogue.entries { entry.vfs = empty.clone(); }
        self.thumbs.drop_queued();
        self.details.drop_queued();
        self.needs_scan = true;
    }

    pub fn scan(&mut self, request: Request) {
        if self.busy() { self.needs_scan = true; return; }
        self.needs_scan = false;
        self.start(JobKind::Scan, move || scan(request).map(|catalogue| Completed::Scan(Box::new(catalogue))));
    }
    pub fn import(&mut self, path: PathBuf, root: PathBuf) {
        if !self.importing() { self.feedback = None; self.import_errors.clear(); }
        let active = usize::from(self.job.as_ref().is_some_and(|job| matches!(job.kind, JobKind::Import)));
        if self.pending_imports.len() + active >= MAX_QUEUED_IMPORTS {
            self.failed(JobKind::Import, ta_ui::i18n::t("The map import queue is full. Wait for an import to finish, then try again.").into());
            return;
        }
        self.pending_imports.push_back((path, root));
        self.start_next_import();
    }
    fn start_next_import(&mut self) {
        if self.job.is_some() { return; }
        let Some((path, root)) = self.pending_imports.pop_front() else { return; };
        self.start(JobKind::Import, move || {
            let (id, count) = import(&path, &root).with_context(|| path.display().to_string())?;
            Ok(Completed::Import(id, count))
        });
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.thumbs.poll(ctx);
        self.details.poll(ctx);
        if let Some(history) = &self.history {
            match history.try_recv() {
                Ok(played) => { self.played = played; self.history = None; self.visible = None; }
                Err(mpsc::TryRecvError::Disconnected) => self.history = None,
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
            }
        }
        if self.job.is_none() { self.start_next_import(); }
        let Some(job) = &self.job else { return; };
        let result = match job.receive.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => { ctx.request_repaint_after(std::time::Duration::from_millis(50)); return; }
            Err(mpsc::TryRecvError::Disconnected) => Err(ta_ui::i18n::t("The map operation stopped unexpectedly. Please try again.").into()),
        };
        let job = self.job.take().expect("received from current job");
        let _ = job.worker.join();
        match result {
            Ok(Completed::Scan(catalogue)) => {
                self.catalogue = *catalogue;
                // (Read again: a picture that could not be made from what was let go of may be now.)
                self.released = false;
                self.thumbs.forget_failures();
                self.details.forget_failures();
                self.generation += 1;
                self.visible = None;
                if !self.catalogue.entries.iter().any(|entry| entry.layer == self.source) { self.source.clear(); }
                if !self.catalogue.entries.iter().any(|entry| entry.planet == self.world) { self.world.clear(); }
                if !self.catalogue.entries.iter().any(|entry| entry.key == self.selected) {
                    // The page reconciles this against the profile's preferences and
                    // current filters before selecting a playable default.
                    self.selected.clear();
                }
                self.prefetch();
            }
            Ok(Completed::Import(id, count)) => {
                if self.import_errors.is_empty() {
                    let message = ta_ui::i18n::tf("Imported {n} maps. The map pack is now enabled for this profile.", &[("n", &count)]);
                    self.notice = Some((message.clone(), false));
                    self.feedback = Some((message, false));
                }
                self.imported.push_back(id); self.needs_scan = true;
            }
            Err(error) => self.failed(job.kind, error),
        }
        self.start_next_import();
        ctx.request_repaint();
    }
    fn failed(&mut self, kind: JobKind, error: String) {
        match kind {
            JobKind::Import => {
                // Keep a batch's errors visible even when later files succeed. The
                // queue is bounded; repeated overflow drops do not grow this report.
                if self.import_errors.len() <= MAX_QUEUED_IMPORTS && !self.import_errors.contains(&error) { self.import_errors.push(error.clone()); }
                self.feedback = Some((self.import_errors.join("\n"), true));
                self.notice = Some((error, true));
            }
            JobKind::Scan => {
                self.feedback = Some((error.clone(), true));
                self.notice = Some((error, true));
            }
        }
    }
    pub fn take_imported(&mut self) -> Option<String> { self.imported.pop_front() }
    /// What finished since it was last asked, once (an import, or a failure) and whether it failed.
    pub fn take_notice(&mut self) -> Option<(String, bool)> { self.notice.take() }
    /// Only automatic selection changes request scrolling. Consume once; a later
    /// manual choice (including in the compact picker) cancels the old request.
    pub fn take_selection_to_reveal(&mut self) -> Option<String> {
        self.reveal_selected.take().filter(|key| key == &self.selected)
    }
    /// Brings `key` into view (a choice made other than by clicking it).
    pub fn reveal(&mut self, key: String) { self.reveal_selected = Some(key); }

    /// The profile's play history is read again when next wanted (a battle was fought, or
    /// another profile is chosen).
    pub fn forget_history(&mut self) {
        self.history_for = None;
        self.history = None;
        self.played.clear();
        self.visible = None;
    }

    /// Reads when profile `id` last played each map, away from the interface.
    pub fn load_history(&mut self, store: Option<ta_config::profiles::ProfileStore>, id: Option<String>) {
        if self.history_for.as_ref() == Some(&id) { return; }
        // A previous commander's pending reply cannot populate the new identity,
        // even when it has no profile or its history worker fails to start.
        self.history = None;
        self.played.clear();
        self.visible = None;
        self.history_for = Some(id.clone());
        let (Some(store), Some(id)) = (store, id) else { self.played.clear(); self.visible = None; return; };
        let (send, receive) = mpsc::channel();
        let read = std::thread::Builder::new().name("map history".into()).spawn(move || {
            let mut played = BTreeMap::new();
            for record in store.history(&id).unwrap_or_default() {
                let name = normal_stem(&record.context.map);
                if !name.is_empty() {
                    let last = played.entry(name).or_insert(0u64);
                    *last = (*last).max(record.started_at);
                }
            }
            let _ = send.send(played);
        });
        if read.is_ok() { self.history = Some(receive); }
    }

    /// The size of a map in the menus' units: its terrain's own once read, else what it says.
    pub fn size_of(&self, entry: &Entry) -> Option<(u32, u32)> {
        self.thumbs.size(&entry.picture).or_else(|| self.details.size(&entry.picture)).map(units).or(entry.typed)
    }

    /// When a map was last played in this profile.
    pub fn last_played(&self, entry: &Entry) -> Option<u64> { self.played.get(&entry.stem.to_ascii_lowercase()).copied() }

    /// The maps that pass the filters and the search, in the chosen order (indices into the
    /// catalogue), worked out again only when something they depend on changed.
    pub fn visible(&mut self, preferences: &ta_config::maps::Maps) -> Vec<usize> {
        let mut hash = std::hash::DefaultHasher::new();
        (self.generation, &self.query, self.filter, &self.source, self.players, self.size, &self.world, self.favourites, self.sort).hash(&mut hash);
        (&preferences.disabled, &preferences.favourites).hash(&mut hash);
        if self.size != Size::Any || self.sort == Sort::Size { (self.thumbs.generation(), self.details.generation()).hash(&mut hash); }
        let stamp = hash.finish();
        if let Some((cached, visible)) = &self.visible && *cached == stamp { return visible.clone(); }
        let query = self.query.trim().to_lowercase();
        let mut visible: Vec<usize> = self.catalogue.entries.iter().enumerate().filter(|(_, entry)| {
            accepts(entry, preferences, &query, self.filter, &self.source)
                && self.players.accepts(entry.players)
                && (self.size == Size::Any || self.size_of(entry).is_some_and(|size| Size::of(size) == self.size))
                && (self.world.is_empty() || entry.planet == self.world)
                && (!self.favourites || preferences.is_favourite(&entry.stem))
        }).map(|(index, _)| index).collect();
        let entries = &self.catalogue.entries;
        let name = |index: &usize| (entries[*index].title.to_lowercase(), entries[*index].source.to_lowercase());
        match self.sort {
            Sort::Name => visible.sort_by_cached_key(name),
            Sort::Players => visible.sort_by_cached_key(|index| (std::cmp::Reverse(entries[*index].players), name(index))),
            Sort::Size => visible.sort_by_cached_key(|index| (std::cmp::Reverse(self.size_of(&entries[*index]).map(|(w, h)| w * h)), name(index))),
            Sort::Recent => visible.sort_by_cached_key(|index| (std::cmp::Reverse(self.last_played(&entries[*index])), name(index))),
            Sort::Source => visible.sort_by_cached_key(|index| (entries[*index].source.to_lowercase(), name(index))),
        }
        // The favourites first, then TA Reborn's own maps, each in the order chosen.
        visible.sort_by_key(|index| entries[*index].group(preferences));
        self.visible = Some((stamp, visible.clone()));
        visible
    }

    /// How many of `visible` (in [`Library::visible`]'s order) each section holds: the
    /// favourites, TA Reborn's maps, the rest.
    pub fn groups(&self, visible: &[usize], preferences: &ta_config::maps::Maps) -> [usize; 3] {
        let mut counts = [0; 3];
        for index in visible {
            counts[self.catalogue.entries[*index].group(preferences) as usize] += 1;
        }
        counts
    }

    pub fn reconcile_selection(&mut self, preferences: &ta_config::maps::Maps) {
        let visible = self.visible(preferences);
        if !visible.iter().any(|index| self.catalogue.entries[*index].key == self.selected) {
            self.selected = visible.iter().map(|index| &self.catalogue.entries[*index]).find(|entry| entry.available(preferences))
                .or_else(|| visible.first().map(|index| &self.catalogue.entries[*index])).map_or(String::new(), |entry| entry.key.clone());
            self.reveal_selected = (!self.selected.is_empty()).then(|| self.selected.clone());
        }
    }

    /// Chooses one of the maps on show at random, a playable one where there is one.
    pub fn random_choice(&mut self, preferences: &ta_config::maps::Maps) {
        let visible = self.visible(preferences);
        let playable: Vec<usize> = visible.iter().copied().filter(|index| self.catalogue.entries[*index].available(preferences)).collect();
        let pool = if playable.is_empty() { visible } else { playable };
        let others: Vec<usize> = pool.iter().copied().filter(|index| self.catalogue.entries[*index].key != self.selected).collect();
        let pool = if others.is_empty() { pool } else { others };
        if pool.is_empty() { return; }
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_nanos() as u64);
        let pick = pool[(seed ^ (seed >> 29)).wrapping_mul(0x9E37_79B9_7F4A_7C15) as usize % pool.len()];
        self.selected = self.catalogue.entries[pick].key.clone();
        self.reveal_selected = Some(self.selected.clone());
    }

    /// The card picture of the catalogue's map `index`, asked for if it is not made yet; and
    /// whether it could not be.
    pub fn thumb(&mut self, index: usize) -> (Option<egui::TextureHandle>, bool) {
        let Some(entry) = self.catalogue.entries.get(index) else { return (None, true) };
        if self.thumbs.wanted(&entry.picture) { self.thumbs.request(entry.picture.clone(), make_picture(entry, self.cache.clone(), CARD_SIDE)); }
        (self.thumbs.get(&entry.picture), self.thumbs.failed(&entry.picture))
    }

    /// The selected map's large picture, asked for if it is not made yet (only the newest ask
    /// is kept waiting); and whether it could not be.
    pub fn details_picture(&mut self) -> (Option<egui::TextureHandle>, bool) {
        self.want_details();
        let Some(entry) = self.catalogue.entries.iter().find(|entry| entry.key == self.selected) else { return (None, false) };
        (self.details.get(&entry.picture), self.details.failed(&entry.picture))
    }

    /// Asks for the selected map's large picture, if it is not made yet: the page may not show
    /// the details yet (a narrow one), and they open at once when it does.
    pub fn want_details(&mut self) {
        let Some(entry) = self.catalogue.entries.iter().find(|entry| entry.key == self.selected) else { return };
        if self.details.wanted(&entry.picture) {
            self.details.cancel_queued();
            self.details.request(entry.picture.clone(), make_picture(entry, None, DETAILS_SIDE));
        }
    }

    /// After a scan: every map's card picture looked ahead for (their sizes order and filter
    /// the library), those cached on disk quickly.
    fn prefetch(&mut self) {
        for entry in &self.catalogue.entries {
            self.thumbs.prefetch(entry.picture.clone(), make_picture(entry, self.cache.clone(), CARD_SIDE));
        }
    }
}

impl Drop for Library {
    fn drop(&mut self) { if let Some(job) = self.job.take() { let _ = job.worker.join(); } }
}

/// A map's name as a battle records it, in lower case without its folder or extension.
fn normal_stem(map: &str) -> String {
    let map = map.trim().replace('\\', "/").to_ascii_lowercase();
    let map = map.strip_prefix("maps/").unwrap_or(&map);
    map.strip_suffix(".tnt").or_else(|| map.strip_suffix(".ota")).unwrap_or(map).to_string()
}

/// Makes the picture of `entry`'s terrain, `side` pixels along its longer side: from `cache`
/// where it was made before (with the terrain's size beside it), else drawn and kept there.
fn make_picture(entry: &Entry, cache: Option<PathBuf>, side: u32) -> impl FnOnce() -> Result<Picture> + Send + 'static {
    let (vfs, stem, palette, key) = (entry.vfs.clone(), entry.stem.clone(), entry.palette.clone(), format!("{}|{side}", entry.picture));
    move || {
        let file = cache.as_deref().map(|dir| library::cache_file(dir, &key));
        let sidecar = |file: &Path| file.with_extension("size");
        if let Some(file) = &file
            && let Some(size) = std::fs::read_to_string(sidecar(file)).ok().and_then(|text| {
                let mut numbers = text.split_whitespace().map(str::parse::<u32>);
                Some((numbers.next()?.ok()?, numbers.next()?.ok()?))
            })
            && let Some(image) = library::read_cached(file) {
            return Ok(Picture { image, size: Some(size) });
        }
        let bytes = vfs.read(&format!("maps/{stem}.tnt"))?;
        let map = TntMap::parse(&bytes)?;
        let picture = ta_render::map_preview::render(&map, &palette, side);
        ensure!(picture.width > 0 && picture.height > 0, "this map has no picture");
        let image = egui::ColorImage::from_rgba_unmultiplied([picture.width, picture.height], &picture.rgba);
        if let Some(file) = &file
            && std::fs::create_dir_all(file.parent().unwrap_or(Path::new("."))).is_ok()
            && std::fs::write(sidecar(file), format!("{} {}", map.width, map.height)).is_ok() {
            library::write_cached(file, &image);
        }
        Ok(Picture { image, size: Some((map.width, map.height)) })
    }
}

fn accepts(entry: &Entry, preferences: &ta_config::maps::Maps, query: &str, filter: Filter, source: &str) -> bool {
    if !source.is_empty() && entry.layer != source { return false; }
    let available = entry.available(preferences);
    let shown = match filter {
        Filter::All => true,
        Filter::Available => available,
        Filter::Unavailable => !available && !entry.campaign,
        Filter::Campaign => entry.campaign,
    };
    shown && (query.is_empty() || query.split_whitespace().all(|word| entry.search.contains(word)))
}

/// Whether `entry` passes the status filter, the source and the search (the other filters
/// are the library's).
#[cfg(test)]
pub fn matches(entry: &Entry, preferences: &ta_config::maps::Maps, query: &str, filter: Filter, source: &str) -> bool {
    accepts(entry, preferences, &query.trim().to_lowercase(), filter, source)
}

/// "W x H" at the start of `text` (in either case, or ×), as TA's map headers and descriptions
/// give a map's size.
fn typed_size(text: &str) -> Option<(u32, u32)> {
    let number = |text: &str| -> Option<(u32, usize)> {
        let end = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
        Some((text[..end].parse().ok()?, end))
    };
    let text = text.trim_start();
    let (width, end) = number(text)?;
    let rest = text[end..].trim_start().strip_prefix(['x', 'X', '×'])?.trim_start();
    let (height, _) = number(rest)?;
    ((1..=64).contains(&width) && (1..=64).contains(&height)).then_some((width, height))
}

/// A map file's identity for its cached picture: where it is, how long, when it was written.
fn file_stamp(source: &Path, file: &str) -> String {
    let stamp = |path: &Path| std::fs::metadata(path).ok().map(|metadata| {
        let written = metadata.modified().ok().and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |time| time.as_secs());
        format!("{}|{written}", metadata.len())
    });
    let loose = source.join(file);
    (if source.is_dir() { stamp(&loose) } else { None }).or_else(|| stamp(source)).unwrap_or_default()
}

fn palette_hash(palette: &Palette) -> String {
    let mut hash = Sha256::new();
    for colour in &palette.colors { hash.update(colour); }
    hash.finalize()[..8].iter().map(|byte| format!("{byte:02x}")).collect()
}

fn mount(vfs: &mut Vfs, info: &ModInfo) -> Result<()> {
    let renames = ta_vfs::Renames::new(&info.manifest.rename);
    match info.kind {
        ModKind::Archive => vfs.mount_archive_renamed(&info.path, &info.manifest.id, &renames)?,
        ModKind::Directory => vfs.mount_mod_dir_renamed(&info.path, &info.manifest.id, &renames)?,
    }
    Ok(())
}

pub fn scan(request: Request) -> Result<Catalogue> {
    let mut out = Catalogue::default();
    let mut sources = Vec::new();
    // What the game plays with, mounted as the game mounts it: the install less the archives the
    // mods on replace, Reborn's own maps, the mods (all of them, or none if one does not read).
    let game = ta_vfs::mods::mount_enabled(request.game_dir.as_deref(), request.builtin.as_deref(), &request.enabled, &request.mods);
    if let Some(root) = &request.game_dir {
        let mut base = Vfs::new();
        // (The install's maps as the game has them: those of an archive a mod on replaces are not.)
        match base.mount_game_dir_without(root, "base", &game.replaced) {
            Ok(()) => {
                out.palette = base.read_opt("palettes/palette.pal").and_then(|bytes| Palette::parse(&bytes).ok()).unwrap_or_default();
                sources.push((Arc::new(base), ta_ui::i18n::t("Total Annihilation").to_string(), true));
            }
            Err(error) => out.warnings.push(error.to_string()),
        }
    }
    if let Some(root) = &request.builtin {
        let mut builtin = Vfs::new();
        match builtin.mount_mod_dir(root, ta_vfs::BUILTIN_LAYER) {
            Ok(()) => sources.push((Arc::new(builtin), "TA Reborn".into(), true)),
            Err(error) => out.warnings.push(error.to_string()),
        }
    }
    let mounted = match game.mods {
        Ok(ids) => ids,
        Err(error) => {
            out.warnings.push(error.to_string());
            Vec::new()
        }
    };
    let effective = game.vfs;
    for info in &request.mods {
        let mut vfs = Vfs::new();
        match mount(&mut vfs, info) {
            Ok(()) => sources.push((Arc::new(vfs), info.manifest.name.clone(), mounted.contains(&info.manifest.id))),
            Err(error) => out.warnings.push(error.to_string()),
        }
    }
    out.palette = effective.read_opt("palettes/palette.pal").and_then(|bytes| Palette::parse(&bytes).ok()).unwrap_or(out.palette);
    let effective = Arc::new(effective);
    let effective_campaigns = campaign_maps(&effective);
    let shared = (Arc::new(out.palette.clone()), palette_hash(&out.palette));
    for (vfs, source, active) in sources {
        // Most entries share the active campaign catalogue. Read a separate source
        // only if its disabled/overridden terrain actually needs isolated metadata.
        let mut source_campaigns = None;
        // (A source painted in a palette of its own is rare: map packs share the game's.)
        let own = vfs.read_opt("palettes/palette.pal").and_then(|bytes| Palette::parse(&bytes).ok())
            .map(|palette| { let hash = palette_hash(&palette); (Arc::new(palette), hash) });
        for path in vfs.list("maps", Some("tnt"), false) {
            let stem = path.trim_start_matches("maps/").trim_end_matches(".tnt").to_string();
            let Some(origin) = vfs.origin(&path) else { continue; };
            let shadowed = active && effective.origin(&path).is_some_and(|winner| winner.layer != origin.layer);
            // An enabled mod can replace only an OTA or the shared palette. Active
            // terrain must describe and preview the same layered data the game loads;
            // disabled/overridden versions remain inspectable as their own source.
            let assets = if active && !shadowed { &effective } else { &vfs };
            let campaigns = if active && !shadowed { &effective_campaigns }
                else { source_campaigns.get_or_insert_with(|| campaign_maps(&vfs)) };
            let bytes = assets.read_opt(&format!("maps/{stem}.ota")).unwrap_or_default();
            if bytes.len() > 1 << 20 { out.warnings.push(format!("{stem}: map description exceeds 1 MiB")); continue; }
            let info = ta_data::MapInfo::from_ota(&tdf::parse_bytes(&bytes));
            let listed = info.schemas.first().map_or(0, |schema| schema.start_positions.len());
            let designed = info.num_players.split(|c: char| !c.is_ascii_digit()).filter_map(|number| number.parse::<usize>().ok()).max();
            let players = designed.map_or(listed, |count| count.min(listed));
            let campaign_name = campaigns.get(&stem);
            let campaign = campaign_name.is_some() || is_campaign(&stem);
            let title = map_title(&info.name, campaign_name.map(String::as_str), &stem);
            let (palette, palette_id) = if active && !shadowed { shared.clone() } else { own.clone().unwrap_or_else(|| shared.clone()) };
            let picture = format!("{}|{stem}|{}|{}|{palette_id}|v{PICTURE_VERSION}", origin.layer, origin.path.display(), file_stamp(&origin.path, &path));
            let starts = info.schemas.first().map_or_else(Vec::new, |schema| schema.start_positions.iter().take(players).map(|(_, x, z)| (*x, *z)).collect());
            let search = [&title, &info.description, &info.planet, &source, &stem].map(|text| text.to_lowercase()).join("\n");
            out.entries.push(Entry {
                key: ta_config::maps::Maps::key(&origin.layer, &stem),
                typed: typed_size(&info.size).or_else(|| typed_size(&info.description)),
                blurb: ta_data::map::blurb(&info.description),
                title, stem, planet: info.planet, size: info.size, starts,
                players, source: source.clone(), layer: origin.layer.clone(), path: origin.path.clone(),
                active, shadowed, campaign, min_wind: info.min_wind, max_wind: info.max_wind, tidal: info.tidal_strength,
                gravity: info.gravity, lava: info.lava_world, water_damage: if info.water_does_damage { info.water_damage.max(0.0) } else { 0.0 },
                picture, palette, search, vfs: assets.clone(),
            });
        }
    }
    out.entries.sort_by_cached_key(|entry| (entry.title.to_lowercase(), entry.source.to_lowercase()));
    Ok(out)
}

fn campaign_maps(vfs: &Vfs) -> BTreeMap<String, String> {
    let mut maps = BTreeMap::new();
    for campaign in ta_data::mission::campaigns(vfs) {
        for mission in campaign.missions {
            let file = ta_vfs::normalize(&mission.file);
            let stem = file.trim_start_matches("maps/").trim_end_matches(".ota").to_string();
            let name = maps.entry(stem).or_insert_with(String::new);
            if usable_title(name).is_none() { *name = mission.name; }
        }
    }
    maps
}

fn usable_title(title: &str) -> Option<&str> {
    let title = title.trim();
    (!title.is_empty() && title.chars().any(|c| !matches!(c, '?' | '\u{fffd}') && !c.is_whitespace())).then_some(title)
}

fn map_title(name: &str, campaign_name: Option<&str>, stem: &str) -> String {
    // Some shipped mission OTAs contain "???" or no title at all. Their campaign
    // supplies the real name; loose maps still have a useful filename fallback.
    if let Some(name) = usable_title(name).or_else(|| campaign_name.and_then(usable_title)) { return name.to_string(); }
    stem.split_whitespace().map(|word| {
        let mut chars = word.chars();
        chars.next().map_or(String::new(), |first| first.to_uppercase().collect::<String>() + chars.as_str())
    }).collect::<Vec<_>>().join(" ")
}

fn is_campaign(stem: &str) -> bool {
    stem.len() <= 9 && ["ac", "cc", "bt", "exp1ac", "exp1cc", "ec", "am", "cm", "ct"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
    })
}

/// Only terrain and its feature assets may enter through Maps. Mods containing AI,
/// units, weapons, palettes or global settings must use the explicit Mods workflow.
fn map_asset(path: &str) -> bool {
    let path = ta_vfs::normalize(path);
    // Archive readers validate member names, but a ZIP extracted on Unix can
    // contain literal backslashes in loose filenames. VFS normalization turns
    // those into separators, so check the resulting components before joining
    // any virtual path to the private on-disk payload directory.
    if !path.split('/').all(package::safe_component) { return false; }
    let Some((folder, file)) = path.split_once('/') else { return false; };
    let extension = file.rsplit('.').next().unwrap_or("");
    match folder {
        "maps" => !file.contains('/') && matches!(extension, "tnt" | "ota"),
        "features" => extension == "tdf",
        "anims" | "textures" => extension == "gaf",
        "objects3d" => extension == "3do",
        "sounds" => extension == "wav",
        _ => false,
    }
}

struct Workspace(PathBuf);
impl Workspace {
    fn new(root: &Path) -> Result<Self> {
        // The folders made for imports (`cache` and `map-imports` in the settings folder) must be
        // the launcher's own. Above them the folders are the player's: a settings folder (or a
        // whole user folder) moved to another drive and linked back is theirs to make.
        for folder in [root.parent(), Some(root)].into_iter().flatten() {
            if let Ok(metadata) = std::fs::symlink_metadata(folder) {
                ensure!(metadata.is_dir() && !is_link(&metadata), "map workspace path contains a link or non-directory");
            }
        }
        std::fs::create_dir_all(root)?;
        ensure!(!is_link(&std::fs::symlink_metadata(root)?), "map library must be an ordinary directory");
        for sequence in 0..1000 {
            let path = root.join(format!(".maps-{}-{}-{sequence}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        bail!("could not create private map workspace")
    }
}
impl Drop for Workspace { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)] { use std::os::windows::fs::MetadataExt; metadata.file_attributes() & 0x400 != 0 }
    #[cfg(not(windows))] { metadata.file_type().is_symlink() }
}

/// What a map import tells the player when it cannot go ahead: marked for translation, and
/// translated where the error is made (the worker's message reaches the page as it is).
mod import_errors {
    use ta_ui::i18n::n;
    pub const ORDINARY_FILE: &str = n("choose an ordinary map archive file");
    pub const PACKAGE_TOO_LARGE: &str = n("map package exceeds 2 GiB");
    pub const KIND: &str = n("choose a ZIP, 7z, RAR or TA map archive");
    pub const SEVEN_ZIP: &str = n("Install 7-Zip to import this map package.");
    pub const NO_MAPS_FOUND: &str = n("No directly installable maps were found. Extract nested ZIP files first, then import each map archive.");
    pub const FOLDERS: &str = n("This package contains multiple content folders. Extract it first and import each map archive separately.");
    pub const UNREADABLE: &str = n("This package contains unreadable map archives. Extract it first and import each working map archive separately.");
    pub const GAME_DATA: &str = n("This package changes game data. Install it from Mods instead.");
    pub const NON_MAP_DATA: &str = n("This package contains non-map game data ({path}). Install it from Mods instead.");
    pub const MAP_COUNT: &str = n("map package must contain between 1 and 1,000 maps");
    pub const CONTENT_TOO_LARGE: &str = n("map content exceeds 2 GiB");
    pub const NO_OTA: &str = n("map is missing its OTA description: {path}");
    pub const BAD_TERRAIN: &str = n("invalid terrain: {path}");
    /// Every one (the test that each is translated).
    #[cfg(test)]
    pub const ALL: [&str; 13] = [ORDINARY_FILE, PACKAGE_TOO_LARGE, KIND, SEVEN_ZIP, NO_MAPS_FOUND, FOLDERS, UNREADABLE,
        GAME_DATA, NON_MAP_DATA, MAP_COUNT, CONTENT_TOO_LARGE, NO_OTA, BAD_TERRAIN];
}

pub fn import(path: &Path, mods_dir: &Path) -> Result<(String, usize)> {
    use ta_ui::i18n::t;
    let metadata = std::fs::symlink_metadata(path).context("opening map package")?;
    ensure!(metadata.is_file() && !is_link(&metadata), "{}", t(import_errors::ORDINARY_FILE));
    ensure!(metadata.len() <= 2 << 30, "{}", t(import_errors::PACKAGE_TOO_LARGE));
    if let Ok(metadata) = std::fs::symlink_metadata(mods_dir) {
        ensure!(metadata.is_dir() && !is_link(&metadata), "map library must be an ordinary directory");
    }
    let workspace = Workspace::new(&mods_dir.parent().context("map library has no parent directory")?.join("cache").join("map-imports"))?;
    let mut vfs = Vfs::new();
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
    if ta_vfs::ARCHIVE_EXTENSIONS.contains(&extension.as_str()) {
        // Snapshot the input so later changes cannot alter what was inspected.
        let snapshot = workspace.0.join(format!("source.{extension}"));
        std::fs::copy(path, &snapshot)?;
        vfs.mount_archive(&snapshot, "map-import")?;
    } else {
        ensure!(ta_vfs::PACKAGE_EXTENSIONS.contains(&extension.as_str()), "{}", t(import_errors::KIND));
        let snapshot = workspace.0.join(format!("source.{extension}"));
        std::fs::copy(path, &snapshot)?;
        let unpacked = workspace.0.join("unpacked");
        package::Unpacker::find().ok_or_else(|| anyhow::anyhow!("{}", t(import_errors::SEVEN_ZIP)))?.unpack(&snapshot, &unpacked)?;
        let plan = package::plan(&unpacked).context(t(import_errors::NO_MAPS_FOUND))?;
        ensure!(plan.skipped.iter().all(|item| !item.contains("an extra")), "{}", t(import_errors::FOLDERS));
        ensure!(plan.skipped.iter().all(|item| !item.contains("empty placeholder") && !item.contains("(unreadable: ")), "{}", t(import_errors::UNREADABLE));
        ensure!(plan.rename.is_empty(), "{}", t(import_errors::GAME_DATA));
        vfs.mount_mod_dir(&plan.root, "map-import")?;
    }
    install_map_data(&vfs, mods_dir, &workspace.0, path.file_stem().and_then(|name| name.to_str()).unwrap_or("Map pack"))
}

fn install_map_data(vfs: &Vfs, mods_dir: &Path, workspace: &Path, name: &str) -> Result<(String, usize)> {
    use ta_ui::i18n::{t, tf};
    // The archive containers and readmes themselves are not game data. Root files
    // and documentation are never installed, including any bundled executables.
    let files: Vec<_> = vfs.list("", None, true).into_iter().filter(|path| path.contains('/') && !path.starts_with("docs/")).collect();
    ensure!(files.len() <= 20_000, "map package has too many files");
    for path in &files {
        ensure!(path.split('/').all(package::safe_component), "unsafe map asset path: {path}");
        ensure!(map_asset(path), "{}", tf(import_errors::NON_MAP_DATA, &[("path", path)]));
    }
    let maps = vfs.list("maps", Some("tnt"), false);
    ensure!(!maps.is_empty() && maps.len() <= 1000, "{}", t(import_errors::MAP_COUNT));
    let payload = workspace.join("payload");
    std::fs::create_dir(&payload)?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    for path in files {
        let bytes = vfs.read(&path)?;
        total += bytes.len() as u64;
        ensure!(total <= 2 << 30, "{}", t(import_errors::CONTENT_TOO_LARGE));
        if path.ends_with(".tnt") {
            let ota = path.trim_end_matches(".tnt").to_string() + ".ota";
            ensure!(vfs.exists(&ota), "{}", tf(import_errors::NO_OTA, &[("path", &path)]));
            TntMap::parse(&bytes).with_context(|| tf(import_errors::BAD_TERRAIN, &[("path", &path)]))?;
        }
        hash.update(path.as_bytes()); hash.update((bytes.len() as u64).to_le_bytes()); hash.update(&bytes);
        let target = payload.join(&path);
        std::fs::create_dir_all(target.parent().unwrap())?;
        std::fs::write(target, bytes)?;
    }
    let id = format!("map-pack-{:x}", hash.finalize());
    let manifest = ModManifest { id: id.clone(), name: name.to_string(), version: String::new(),
        description: "Imported map pack".into(), authors: Vec::new(), depends: Vec::new(), load_after: Vec::new(), source: String::new(), rename: BTreeMap::new(), replaces: Vec::new() };
    // The existing package transaction validates paths, locks the pack, stages files,
    // and rolls back a failed replacement. Only the private copy is moved.
    package::install(&package::plan(&payload)?, &manifest, mods_dir)?;
    Ok((id, maps.len()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn changing_or_refreshing_map_history_discards_the_previous_profiles_pending_reply() {
        let ctx = egui::Context::default();
        let mut library = Library::default();
        for refresh in [false, true] {
            let (send, receive) = mpsc::channel();
            send.send(BTreeMap::from([("old-map".into(), 123)])).unwrap();
            library.history = Some(receive);
            library.history_for = Some(Some("old-profile".into()));
            library.played.insert("old-map".into(), 100);
            if refresh { library.forget_history(); }
            else { library.load_history(None, None); }
            library.poll(&ctx);
            assert!(library.played.is_empty(), "stale history must not reappear after invalidation");
            assert!(library.history.is_none());
        }
    }
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("ta-map-library-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
            std::fs::create_dir_all(&root).unwrap();
            // macOS temp_dir can begin with /var, an OS-owned symlink to
            // /private/var. Exercise map imports from an ordinary fixture root.
            Self(std::fs::canonicalize(root).unwrap())
        }
    }
    impl Drop for Temp { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
    fn terrain() -> Vec<u8> {
        let width = 16;
        let height = 16;
        TntMap { width, height, sea_level: 0, tiles: vec![Box::new([12; 1024])], tile_map: vec![0; (width * height / 4) as usize],
            heights: vec![0; (width * height) as usize], features: vec![ta_formats::tnt::FEATURE_NONE; (width * height) as usize],
            feature_names: Vec::new(), minimap: ta_formats::image::IndexedImage::new(252, 144, 12) }.write().unwrap()
    }
    /// Real HPI container with tiny original terrain, for integration tests of the
    /// launcher worker. It needs no external unpacker or copyrighted game data.
    pub(crate) fn write_map_archive(path: &Path) {
        write_map_archive_named(path, "moon");
    }
    fn write_map_archive_named(path: &Path, stem: &str) {
        let files = [(format!("maps/{stem}.tnt"), terrain()), (format!("maps/{stem}.ota"), b"[GlobalHeader]{missionname=Moon;numplayers=2;[Schema 0]{[specials]{[special0]{specialwhat=StartPos1;}[special1]{specialwhat=StartPos2;}}}}".to_vec())];
        let directory_end = 28 + files.len() * 9 + files.iter().map(|(name, _)| name.len() + 1 + 9).sum::<usize>();
        let mut bytes = vec![0u8; directory_end];
        fn put(bytes: &mut [u8], at: usize, value: usize) { bytes[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes()); }
        bytes[..4].copy_from_slice(b"HAPI");
        put(&mut bytes, 4, 0x10000); put(&mut bytes, 8, directory_end); put(&mut bytes, 16, 20);
        put(&mut bytes, 20, files.len()); put(&mut bytes, 24, 28);
        let mut cursor = 28 + files.len() * 9;
        for (index, (name, payload)) in files.iter().enumerate() {
            let record = cursor + name.len() + 1;
            put(&mut bytes, 28 + index * 9, cursor); put(&mut bytes, 32 + index * 9, record);
            bytes[cursor..cursor + name.len()].copy_from_slice(name.as_bytes());
            let offset = bytes.len(); put(&mut bytes, record, offset); put(&mut bytes, record + 4, payload.len());
            bytes.extend_from_slice(payload); cursor = record + 9;
        }
        std::fs::write(path, bytes).unwrap();
    }
    fn map(root: &Path, stem: &str, title: &str) {
        std::fs::create_dir_all(root.join("maps")).unwrap();
        std::fs::write(root.join("maps").join(format!("{stem}.tnt")), terrain()).unwrap();
        std::fs::write(root.join("maps").join(format!("{stem}.ota")), format!(
            "[GlobalHeader]{{missionname={title}; numplayers=2,3,4; planet=Moon; [Schema 0]{{[specials]{{[special0]{{specialwhat=StartPos1;XPos=32;ZPos=32;}} [special1]{{specialwhat=StartPos2;XPos=128;ZPos=128;}}}}}}}}"
        )).unwrap();
    }
    /// A catalogue of two maps in a game folder under `root` (Coast, Moon Base), for the pages'
    /// tests.
    pub(crate) fn sample_catalogue(root: &Path) -> Catalogue {
        let source = root.join("source");
        map(&source, "coast", "Coast");
        map(&source, "moon", "Moon Base");
        scan(Request { game_dir: Some(source), builtin: None, mods: Vec::new(), enabled: Vec::new() }).unwrap()
    }
    fn info(root: PathBuf, id: &str) -> ModInfo {
        ModInfo { path: root, kind: ModKind::Directory, manifest: ModManifest {
            id: id.into(), name: format!("Source {id}"), version: "1".into(), description: String::new(), authors: Vec::new(),
            depends: Vec::new(), load_after: Vec::new(), source: String::new(), rename: BTreeMap::new(), replaces: Vec::new(),
        } }
    }

    #[test]
    fn discovery_shows_disabled_sources_and_overridden_originals() {
        let temp = Temp::new();
        let base = temp.0.join("base"); let builtin = temp.0.join("builtin");
        let active = temp.0.join("active"); let disabled = temp.0.join("disabled");
        map(&base, "Coast", "Original Coast"); map(&base, "ac1", "First Mission");
        map(&builtin, "Reborn", "Reborn Terrain"); map(&active, "Coast", "Modified Coast"); map(&disabled, "Moon", "Silent Moon");
        let catalogue = scan(Request { game_dir: Some(base), builtin: Some(builtin), mods: vec![info(active, "active"), info(disabled, "disabled")], enabled: vec!["active".into()] }).unwrap();
        assert_eq!(catalogue.entries.len(), 5);
        let find = |layer: &str, stem: &str| catalogue.entries.iter().find(|entry| entry.layer == layer && entry.stem == stem).unwrap();
        assert!(find("base", "coast").shadowed);
        assert!(find("active", "coast").available(&Default::default()));
        assert_eq!(find("active", "coast").players, 2, "actual start slots cap designer's player count");
        assert!(!find("disabled", "moon").active);
        assert!(find("base", "ac1").campaign);
        assert!(find(ta_vfs::BUILTIN_LAYER, "reborn").active);
        let mut preferences = ta_config::maps::Maps::default(); preferences.set_enabled("active", "coast", false);
        assert!(!find("active", "coast").available(&preferences));
        assert!(matches(find("disabled", "moon"), &preferences, "moon", Filter::Unavailable, "disabled"));
        assert!(matches(find("active", "coast"), &preferences, "coast", Filter::Unavailable, ""), "a deactivated map is unavailable");
        assert!(!matches(find("base", "ac1"), &preferences, "", Filter::Unavailable, ""), "campaign maps have their own filter");
        assert!(!matches(find("disabled", "moon"), &preferences, "moon", Filter::Available, ""));
    }

    #[test]
    fn favourites_come_first_then_reborn_maps_then_the_rest() {
        let temp = Temp::new();
        let base = temp.0.join("base"); let builtin = temp.0.join("builtin");
        map(&base, "aardvark", "Aardvark Run"); map(&base, "zinc", "Zinc Flats"); map(&base, "moon", "Moon Base");
        map(&builtin, "reborn", "Reborn Terrain"); map(&builtin, "highland", "Highland Rift");
        let mut library = Library::default();
        library.catalogue = scan(Request { game_dir: Some(base), builtin: Some(builtin), mods: Vec::new(), enabled: Vec::new() }).unwrap();
        let mut preferences = ta_config::maps::Maps::default();
        preferences.set_favourite("zinc", true);
        preferences.set_favourite("reborn", true);
        let visible = library.visible(&preferences);
        let titles: Vec<&str> = visible.iter().map(|&index| library.catalogue.entries[index].title.as_str()).collect();
        // Each section in the order chosen (by name): a favourite of Reborn's own is a favourite.
        assert_eq!(titles, ["Reborn Terrain", "Zinc Flats", "Highland Rift", "Aardvark Run", "Moon Base"]);
        assert_eq!(library.groups(&visible, &preferences), [2, 1, 2]);
        assert!(library.catalogue.entries[visible[2]].reborn() && !library.catalogue.entries[visible[3]].reborn());
        // The first featured map is the one chosen when nothing is.
        library.reconcile_selection(&preferences);
        assert_eq!(library.selected, library.catalogue.entries[visible[0]].key);
    }

    #[test]
    fn the_library_filters_and_orders_by_players_size_world_favourites_and_play() {
        let temp = Temp::new(); let source = temp.0.join("source");
        map(&source, "coast", "Coast"); map(&source, "moon", "Moon Base"); map(&source, "vast", "Vast Plains"); map(&source, "duel", "Duel");
        let ota = |stem: &str| source.join(format!("maps/{stem}.ota"));
        std::fs::write(ota("vast"), std::fs::read_to_string(ota("vast")).unwrap()
            .replace("planet=Moon;", "planet=Green Planet; missiondescription=24 X 22  Plenty of room;")
            .replace("[special1]{specialwhat=StartPos2;XPos=128;ZPos=128;}", "[special1]{specialwhat=StartPos2;XPos=128;ZPos=128;} [special2]{specialwhat=StartPos3;} [special3]{specialwhat=StartPos4;}")).unwrap();
        std::fs::write(ota("duel"), std::fs::read_to_string(ota("duel")).unwrap().replace("numplayers=2,3,4;", "numplayers=2; size=6 x 6;")).unwrap();
        let mut library = Library::default();
        library.catalogue = scan(Request { game_dir: Some(source), builtin: None, mods: Vec::new(), enabled: Vec::new() }).unwrap();
        let mut preferences = ta_config::maps::Maps::default();
        let titles = |library: &mut Library, preferences: &ta_config::maps::Maps| library.visible(preferences).into_iter()
            .map(|index| library.catalogue.entries[index].title.clone()).collect::<Vec<_>>();
        assert_eq!(titles(&mut library, &preferences), ["Coast", "Duel", "Moon Base", "Vast Plains"]);
        let vast = library.catalogue.entries.iter().find(|entry| entry.stem == "vast").unwrap();
        assert_eq!((vast.players, vast.typed, vast.blurb.as_str()), (4, Some((24, 22)), "Plenty of room"), "the typed size is read and left out of the blurb");
        assert_eq!(vast.starts.len(), 4);
        library.players = Players::Four;
        assert_eq!(titles(&mut library, &preferences), ["Vast Plains"]);
        library.players = Players::Any; library.size = Size::Small;
        assert_eq!(titles(&mut library, &preferences), ["Duel"], "maps of unknown size wait for their terrain to be read");
        library.size = Size::Any; library.world = "Green Planet".into();
        assert_eq!(titles(&mut library, &preferences), ["Vast Plains"]);
        library.world.clear(); library.favourites = true;
        assert!(titles(&mut library, &preferences).is_empty());
        preferences.set_favourite("moon", true);
        assert_eq!(titles(&mut library, &preferences), ["Moon Base"], "a favourite changes what is shown at once");
        // (Favourites come first whatever the order: the orders are looked at without one.)
        preferences.set_favourite("moon", false);
        library.favourites = false; library.sort = Sort::Players;
        assert_eq!(titles(&mut library, &preferences)[0], "Vast Plains");
        library.sort = Sort::Size;
        assert_eq!(titles(&mut library, &preferences)[..2], ["Vast Plains", "Duel"], "known sizes first, largest first");
        library.played = [("duel".to_string(), 20), ("coast".to_string(), 10)].into();
        library.visible = None;
        library.sort = Sort::Recent;
        assert_eq!(titles(&mut library, &preferences)[..2], ["Duel", "Coast"]);
        library.query = "plains room".into();
        assert_eq!(titles(&mut library, &preferences), ["Vast Plains"], "every word of a search must be found");
        library.query.clear(); library.selected.clear();
        for _ in 0..8 {
            library.random_choice(&preferences);
            assert!(library.catalogue.entries.iter().any(|entry| entry.key == library.selected && entry.available(&preferences)));
        }
    }

    #[test]
    fn play_history_is_read_once_for_each_profile() {
        let mut library = Library::default();
        library.load_history(None, None);
        library.visible = Some((1, vec![0]));
        library.load_history(None, None);
        assert!(library.visible.is_some(), "with no profile the history is not read again every frame");
        library.forget_history();
        library.load_history(None, None);
        assert!(library.visible.is_none(), "after a battle it is");
    }

    #[test]
    fn map_pictures_are_drawn_from_the_terrain_and_kept_with_its_size() {
        let temp = Temp::new(); let source = temp.0.join("source"); map(&source, "moon", "Moon");
        let catalogue = scan(Request { game_dir: Some(source.clone()), builtin: None, mods: Vec::new(), enabled: Vec::new() }).unwrap();
        let entry = &catalogue.entries[0];
        let cache = temp.0.join("cache");
        let made = make_picture(entry, Some(cache.clone()), 64)().unwrap();
        assert_eq!(made.size, Some((16, 16)));
        assert!(made.image.size[0] > 0 && made.image.size[0] <= 64);
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 2, "the picture and its size are kept");
        std::fs::remove_file(source.join("maps/moon.tnt")).unwrap();
        let again = make_picture(entry, Some(cache), 64)().unwrap();
        assert_eq!((again.size, again.image.size), (made.size, made.image.size), "a cached picture needs no terrain");
        assert_eq!(units((16, 16)), (1, 1));
        assert_eq!(units((384, 320)), (12, 10), "TA's 12 x 10 maps");
        assert_eq!(normal_stem("Maps\\Coast To Coast.tnt"), "coast to coast");
        assert_eq!(typed_size(" 12 x 10  Islands"), Some((12, 10)));
        assert_eq!(typed_size("2 v 2 on islands"), None);
    }

    #[test]
    fn invalid_mod_order_does_not_advertise_unmounted_maps() {
        let temp = Temp::new(); let source = temp.0.join("source"); map(&source, "Moon", "Moon");
        let catalogue = scan(Request { game_dir: None, builtin: None, mods: vec![info(source, "source")], enabled: vec!["missing".into(), "source".into()] }).unwrap();
        assert!(!catalogue.warnings.is_empty());
        assert!(!catalogue.entries[0].active);
    }

    #[test]
    fn active_map_details_and_preview_use_the_same_overlays_as_the_game() {
        let temp = Temp::new();
        let base = temp.0.join("base"); let patch = temp.0.join("patch"); let disabled = temp.0.join("disabled");
        map(&base, "Moon", "Original Moon"); map(&disabled, "Moon", "Disabled Moon");
        std::fs::create_dir_all(patch.join("maps")).unwrap();
        std::fs::write(patch.join("maps/moon.ota"), "[GlobalHeader]{missionname=Patched Moon;numplayers=1;[Schema 0]{[specials]{[special0]{specialwhat=StartPos1;}}}}").unwrap();
        for (root, color) in [(&base, [12, 24, 36]), (&patch, [90, 80, 70]), (&disabled, [60, 50, 40])] {
            std::fs::create_dir_all(root.join("palettes")).unwrap();
            let mut palette = vec![0; 1024]; palette[12 * 4..12 * 4 + 3].copy_from_slice(&color);
            std::fs::write(root.join("palettes/palette.pal"), palette).unwrap();
        }
        let catalogue = scan(Request { game_dir: Some(base), builtin: None,
            mods: vec![info(patch, "patch"), info(disabled, "disabled")], enabled: vec!["patch".into()] }).unwrap();
        let active = catalogue.entries.iter().find(|entry| entry.layer == "base").unwrap();
        assert_eq!(active.title, "Patched Moon", "an OTA-only patch changes the playable map's metadata");
        assert_eq!(active.players, 1);
        assert!(!active.available(&Default::default()), "the game cannot start a skirmish with one position");
        assert_eq!(Palette::parse(&active.vfs.read("palettes/palette.pal").unwrap()).unwrap().rgb(12), [90, 80, 70]);
        let inactive = catalogue.entries.iter().find(|entry| entry.layer == "disabled").unwrap();
        assert_eq!(inactive.title, "Disabled Moon", "disabled source details remain inspectable independently");
        assert_eq!(Palette::parse(&inactive.vfs.read("palettes/palette.pal").unwrap()).unwrap().rgb(12), [60, 50, 40]);
    }

    #[test]
    fn campaign_metadata_replaces_placeholder_titles_without_hiding_maps() {
        let temp = Temp::new(); let base = temp.0.join("base"); let disabled = temp.0.join("disabled");
        map(&base, "rainforest construction", "???");
        map(&base, "exp1ac01", "");
        map(&base, "unlabelled valley", " ? ? ? ");
        map(&base, "question mark", "Where to now?");
        map(&disabled, "rainforest construction", "Skirmish Forest");
        std::fs::create_dir(base.join("camps")).unwrap();
        std::fs::write(base.join("camps/story.tdf"),
            "[HEADER]{campaignside=CORE;}[MISSION0]{missionfile=Rainforest Construction.OTA;missionname=Rainforest Construction;}[MISSION1]{missionfile=EXP1AC01.ota;missionname=A New Beginning;}").unwrap();
        let catalogue = scan(Request { game_dir: Some(base), builtin: None,
            mods: vec![info(disabled, "disabled")], enabled: Vec::new() }).unwrap();
        assert_eq!(catalogue.entries.len(), 5, "placeholder metadata must not remove terrain from the library");
        let find = |layer: &str, stem: &str| catalogue.entries.iter().find(|entry| entry.layer == layer && entry.stem == stem).unwrap();
        assert_eq!(find("base", "rainforest construction").title, "Rainforest Construction");
        assert!(find("base", "rainforest construction").campaign, "Battle Tactics uses descriptive filenames, not campaign prefixes");
        assert_eq!(find("base", "exp1ac01").title, "A New Beginning");
        assert_eq!(find("base", "unlabelled valley").title, "Unlabelled Valley");
        assert_eq!(find("base", "question mark").title, "Where to now?", "real titles may contain question marks");
        assert!(!find("disabled", "rainforest construction").campaign, "disabled versions use their own campaign metadata");
        assert_eq!(find("disabled", "rainforest construction").title, "Skirmish Forest");
    }

    #[test]
    fn default_selection_prefers_a_playable_map_and_keeps_explicit_choices() {
        let temp = Temp::new(); let source = temp.0.join("source");
        map(&source, "editor", "A Editor Terrain");
        map(&source, "coast", "B Coast");
        map(&source, "ac1", "C Campaign");
        map(&source, "moon", "D Moon");
        let editor = source.join("maps/editor.ota");
        std::fs::write(&editor, std::fs::read_to_string(&editor).unwrap().replace("numplayers=2,3,4", "numplayers=1")).unwrap();
        let mut preferences = ta_config::maps::Maps::default();
        preferences.set_enabled("base", "coast", false);
        let mut library = Library::default();
        library.catalogue = scan(Request { game_dir: Some(source), builtin: None, mods: Vec::new(), enabled: Vec::new() }).unwrap();
        library.reconcile_selection(&preferences);
        assert!(library.selected.ends_with("|moon"), "default selection skips editor-only, deactivated and campaign maps");
        library.selected = ta_config::maps::Maps::key("base", "editor");
        library.reconcile_selection(&preferences);
        assert!(library.selected.ends_with("|editor"), "a deliberate selection remains inspectable");
        library.query = "campaign".into(); library.reconcile_selection(&preferences);
        assert!(library.selected.ends_with("|ac1"), "search may select non-playable terrain when it is the only result");
        library.query.clear(); library.filter = Filter::Unavailable; library.reconcile_selection(&preferences);
        assert!(library.selected.ends_with("|editor"), "the default must respect active filters");
        library.players = Players::Two; library.reconcile_selection(&preferences);
        assert!(library.selected.ends_with("|coast"), "and the players filter");
    }

    #[test]
    fn filtered_selection_never_keeps_an_unrelated_action_target() {
        let temp = Temp::new(); let source = temp.0.join("source");
        map(&source, "Moon", "Moon"); map(&source, "ac1", "First Mission");
        let mut library = Library::default();
        library.catalogue = scan(Request { game_dir: Some(source), builtin: None, mods: Vec::new(), enabled: Vec::new() }).unwrap();
        let preferences = ta_config::maps::Maps::default();
        library.reconcile_selection(&preferences);
        library.filter = Filter::Campaign; library.reconcile_selection(&preferences);
        assert!(library.selected.ends_with("|ac1"));
        let previous = library.selected.clone();
        library.query = "does not exist".into(); library.reconcile_selection(&preferences);
        assert!(library.selected.is_empty(), "empty results have no old PLAY or DEACTIVATE target");
        library.query.clear(); library.reconcile_selection(&preferences);
        assert_eq!(library.selected, previous);
    }

    #[test]
    fn automatic_selection_scrolls_once_without_overriding_manual_browsing() {
        let temp = Temp::new(); let source = temp.0.join("source");
        map(&source, "coast", "Coast"); map(&source, "moon", "Silent Moon");
        let mut library = Library::default();
        library.catalogue = scan(Request { game_dir: Some(source), builtin: None, mods: Vec::new(), enabled: Vec::new() }).unwrap();
        let preferences = ta_config::maps::Maps::default();
        library.reconcile_selection(&preferences);
        assert_eq!(library.take_selection_to_reveal(), Some(library.selected.clone()));
        library.reconcile_selection(&preferences);
        assert!(library.take_selection_to_reveal().is_none(), "later frames and preview updates must not snap back the list");
        library.query = "moon".into(); library.reconcile_selection(&preferences);
        assert!(library.selected.ends_with("|coast"), "both maps share the Moon planet, so a valid selection remains");
        assert!(library.take_selection_to_reveal().is_none(), "a filter that keeps the same selection must not move the list");
        library.query = "silent".into(); library.reconcile_selection(&preferences);
        assert_eq!(library.take_selection_to_reveal(), Some(ta_config::maps::Maps::key("base", "moon")));
        library.query.clear(); library.selected.clear(); library.reconcile_selection(&preferences);
        library.selected = ta_config::maps::Maps::key("base", "moon");
        assert!(library.take_selection_to_reveal().is_none(), "an explicit choice cancels an older automatic request");
        library.reconcile_selection(&preferences);
        assert!(library.take_selection_to_reveal().is_none(), "clicking another visible map must not move the list");
        library.query = "missing".into(); library.reconcile_selection(&preferences);
        assert!(library.take_selection_to_reveal().is_none(), "empty results have no scroll target");
    }

    /// Every map import error a player can meet is translated in every language that ships
    /// (they reached the page in English whatever the language), its placeholder kept.
    #[test]
    fn map_import_errors_are_translated_in_every_language() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lang");
        let mut languages = 0;
        for path in std::fs::read_dir(&dir).unwrap().flatten().map(|entry| entry.path()) {
            if path.extension().is_none_or(|e| e != "toml") || path.file_stem().is_some_and(|s| s == "template") { continue; }
            let table: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
            let strings = table["strings"].as_table().unwrap();
            for message in import_errors::ALL {
                let translated = strings.get(message).and_then(|v| v.as_str()).unwrap_or_default();
                assert!(!translated.is_empty(), "{}: {message}", path.display());
                assert_eq!(translated.contains("{path}"), message.contains("{path}"), "{}: {message}", path.display());
            }
            languages += 1;
        }
        assert!(languages >= 13, "the languages that ship: {languages}");
    }

    /// A settings folder that a link puts on another drive (the player's own doing, above the
    /// launcher's folders) still imports maps; a link where the launcher keeps its import
    /// workspace is still refused.
    #[cfg(windows)]
    #[test]
    fn map_import_works_in_a_settings_folder_reached_through_a_link() {
        let temp = Temp::new();
        // (mklink takes a plain path, not the canonical `\\?\` form `Temp` has.)
        let plain = |path: &Path| PathBuf::from(path.to_string_lossy().trim_start_matches(r"\\?\").to_string());
        let junction = |link: &Path, target: &Path| {
            std::fs::create_dir_all(target).unwrap();
            let made = std::process::Command::new("cmd").arg("/C").arg("mklink").arg("/J").arg(plain(link)).arg(plain(target))
                .stdout(std::process::Stdio::null()).status().unwrap();
            assert!(made.success(), "mklink /J {}", link.display());
        };
        let elsewhere = temp.0.join("other drive").join("TA Reborn");
        let settings = temp.0.join("TA Reborn");
        junction(&settings, &elsewhere);
        let archive = temp.0.join("moon.ufo");
        write_map_archive(&archive);
        let (_, count) = import(&archive, &settings.join("mods")).expect("a settings folder behind a link imports maps");
        assert_eq!(count, 1);
        assert!(elsewhere.join("mods").is_dir(), "the pack is installed where the link leads");
        // (The launcher's own working folder made a link: refused, nothing written through it.)
        let outside = temp.0.join("outside");
        std::fs::remove_dir_all(elsewhere.join("cache")).unwrap();
        junction(&elsewhere.join("cache"), &outside);
        let error = import(&archive, &settings.join("mods")).unwrap_err();
        assert!(format!("{error:#}").contains("link"), "{error:#}");
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0, "nothing is written through the link");
        // (Junctions are removed as links, their targets left: `Temp` cleans up the rest.)
        std::fs::remove_dir(elsewhere.join("cache")).unwrap();
        std::fs::remove_dir(&settings).unwrap();
    }

    /// The library holds the game and the mods its maps are read from, mounted, for their
    /// pictures: on Windows a mod's folder holding an archive so opened cannot be moved aside
    /// for its update, nor removed. Let go of, it can (and the maps are looked for again).
    #[cfg(windows)]
    #[test]
    fn a_released_library_holds_no_mods_archive_open() {
        let temp = Temp::new();
        let mods = temp.0.join("mods");
        let folder = mods.join("moonpack");
        std::fs::create_dir_all(&folder).unwrap();
        write_map_archive(&folder.join("moon.ufo"));
        std::fs::write(folder.join("mod.toml"), "id = \"moonpack\"
name = \"Moon pack\"
").unwrap();
        let (found, errors) = ta_vfs::mods::scan(&mods);
        assert!(errors.is_empty(), "{errors:?}");
        let mut library = Library::default();
        library.catalogue = scan(Request { game_dir: None, builtin: None, mods: found, enabled: vec!["moonpack".into()] }).unwrap();
        assert!(library.catalogue.entries.iter().any(|entry| entry.stem == "moon"));
        let aside = mods.join(".moonpack.old");
        assert!(std::fs::rename(&folder, &aside).is_err(), "the mounted archive keeps its folder where it is");
        library.release();
        std::fs::rename(&folder, &aside).expect("let go of, the folder moves");
        assert!(library.needs_scan, "the maps are looked for again");
        library.release();
    }

    #[test]
    fn map_import_rejects_gameplay_changes_without_publishing() {
        let temp = Temp::new(); let source = temp.0.join("source"); map(&source, "Moon", "Moon");
        std::fs::create_dir(source.join("units")).unwrap(); std::fs::write(source.join("units/armcom.fbi"), b"modified commander").unwrap();
        let mut vfs = Vfs::new(); vfs.mount_mod_dir(&source, "import").unwrap();
        let work = Workspace::new(&temp.0.join("work")).unwrap();
        let mods = temp.0.join("mods");
        let error = install_map_data(&vfs, &mods, &work.0, "Bad package").unwrap_err().to_string();
        assert!(error.contains("Mods instead"), "{error}");
        assert!(!mods.exists());
        assert!(source.join("units/armcom.fbi").exists(), "source files are unchanged");
    }

    #[test]
    fn map_import_is_validated_atomic_and_keeps_original_files() {
        let temp = Temp::new(); let source = temp.0.join("source"); map(&source, "Moon", "Moon");
        std::fs::write(source.join("README.txt"), "This readme is not mounted game data.").unwrap();
        let bytes = std::fs::read(source.join("maps/Moon.tnt")).unwrap();
        let mut vfs = Vfs::new(); vfs.mount_mod_dir(&source, "import").unwrap();
        let mods = temp.0.join("mods");
        let work = Workspace::new(&temp.0.join("work")).unwrap();
        let (id, count) = install_map_data(&vfs, &mods, &work.0, "My maps").unwrap();
        assert_eq!(count, 1);
        assert!(mods.join(&id).join("mod.toml").is_file());
        assert_eq!(std::fs::read(mods.join(&id).join("maps/moon.tnt")).unwrap(), bytes);
        assert_eq!(std::fs::read(source.join("maps/Moon.tnt")).unwrap(), bytes);
        assert!(!mods.join(&id).join("README.txt").exists());
        let mut installed = Vfs::new(); installed.mount_mod_dir(&mods.join(&id), &id).unwrap();
        assert!(TntMap::parse(&installed.read("maps/moon.tnt").unwrap()).is_ok());
        let work_again = Workspace::new(&temp.0.join("work")).unwrap();
        let (again, _) = install_map_data(&vfs, &mods, &work_again.0, "My maps").unwrap();
        assert_eq!(id, again, "same content imports into the same pack, without duplicates");
    }

    #[test]
    fn map_imports_not_started_are_dropped_at_shutdown_and_their_files_left_alone() {
        let temp = Temp::new();
        let first = temp.0.join("first.ufo");
        write_map_archive_named(&first, "moon");
        let original = std::fs::read(&first).unwrap();
        let root = temp.0.join("mods");
        let mut library = Library::default();
        let (release, wait) = mpsc::channel();
        library.start(JobKind::Scan, move || {
            wait.recv()?;
            Ok(Completed::Scan(Box::default()))
        });
        library.import(first.clone(), root.clone());
        assert!(library.importing());
        release.send(()).unwrap();
        assert!(library.finish(&egui::Context::default()).is_empty(), "closing does not wait for imports not started");
        assert!(!library.busy() && !library.importing());
        assert!(!root.exists(), "nothing half imported");
        assert_eq!(std::fs::read(first).unwrap(), original);
    }

    #[test]
    fn queued_map_imports_survive_reads_without_losing_successes_or_errors() {
        let temp = Temp::new();
        let first = temp.0.join("first.ufo"); let second = temp.0.join("second.ufo");
        write_map_archive_named(&first, "moon"); write_map_archive_named(&second, "mars");
        let originals = [std::fs::read(&first).unwrap(), std::fs::read(&second).unwrap()];
        let root = temp.0.join("mods");
        let mut library = Library::default();
        let (release, wait) = mpsc::channel();
        library.start(JobKind::Scan, move || {
            wait.recv()?;
            Ok(Completed::Scan(Box::default()))
        });
        library.import(first.clone(), root.clone());
        library.import(temp.0.join("missing.ufo"), root.clone());
        library.import(second.clone(), root.clone());
        assert!(library.busy() && library.importing(), "queued writes block profile/game/mod handover even behind a scan");
        assert!(library.take_imported().is_none());
        release.send(()).unwrap();
        // Every accepted operation runs in turn as frames come, and every successful pack is
        // handed over for the pinned profile to enable.
        let ctx = egui::Context::default();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while library.busy() && std::time::Instant::now() < until {
            library.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let imported: Vec<String> = std::iter::from_fn(|| library.take_imported()).collect();
        assert_eq!(imported.len(), 2);
        assert_ne!(imported[0], imported[1]);
        for id in &imported { assert!(root.join(id).join("mod.toml").is_file()); }
        assert!(root.join(&imported[0]).join("maps/moon.tnt").is_file());
        assert!(root.join(&imported[1]).join("maps/mars.tnt").is_file());
        assert_eq!(std::fs::read(first).unwrap(), originals[0]);
        assert_eq!(std::fs::read(second).unwrap(), originals[1]);
        let (message, error) = library.feedback.as_ref().unwrap();
        assert!(*error && message.contains("missing.ufo"), "later success must not hide an earlier file's failure");
        assert!(!library.busy() && !library.importing());
    }

    #[test]
    fn full_map_queue_reports_rejected_drops_and_remains_bounded() {
        let temp = Temp::new();
        let mut library = Library::default();
        let (release, wait) = mpsc::channel();
        library.start(JobKind::Scan, move || {
            wait.recv()?;
            anyhow::bail!("old scan failed")
        });
        for index in 0..MAX_QUEUED_IMPORTS + 10 {
            library.import(temp.0.join(format!("missing-{index}.ufo")), temp.0.join("mods"));
        }
        assert_eq!(library.pending_imports.len(), MAX_QUEUED_IMPORTS);
        assert!(library.feedback.as_ref().is_some_and(|(message, error)| *error && message.contains("queue is full")));
        release.send(()).unwrap();
        assert!(library.finish(&egui::Context::default()).is_empty());
        assert!(!library.busy());
        assert!(!temp.0.join("mods").exists());
    }

    #[test]
    fn broken_terrain_never_replaces_an_installed_map_pack() {
        let temp = Temp::new(); let source = temp.0.join("source"); map(&source, "Moon", "Moon");
        std::fs::write(source.join("maps/Moon.tnt"), b"broken").unwrap();
        let mut vfs = Vfs::new(); vfs.mount_mod_dir(&source, "import").unwrap();
        let work = Workspace::new(&temp.0.join("work")).unwrap(); let mods = temp.0.join("mods");
        assert!(install_map_data(&vfs, &mods, &work.0, "Bad map").is_err());
        assert!(!mods.exists());
    }

    /// The sizes TA's maps say they are against their terrain's, in the menus' units.
    #[test]
    #[ignore = "reads the TA install"]
    fn typed_sizes_match_the_terrain() {
        let Some(game) = crate::detect::detect(None, false).install().map(|install| install.root.clone()) else { return };
        let catalogue = scan(Request { game_dir: Some(game), builtin: None, mods: Vec::new(), enabled: Vec::new() }).unwrap();
        let (mut same, mut typed) = (0, 0);
        for entry in &catalogue.entries {
            let Some(said) = entry.typed else { continue };
            let cells = TntMap::size(&entry.vfs.read(&format!("maps/{}.tnt", entry.stem)).unwrap()).unwrap();
            typed += 1;
            if units(cells) == said { same += 1; } else { println!("{}: says {said:?}, is {:?} ({cells:?} cells)", entry.title, units(cells)); }
        }
        println!("{same} of {typed} typed sizes match");
        assert!(same * 10 >= typed * 8, "the units are not TA's");
    }

    #[cfg(unix)]
    #[test]
    fn map_workspace_rejects_linked_ancestors_before_creating_files() {
        let temp = Temp::new();
        let ordinary = temp.0.join("ordinary");
        std::fs::create_dir(&ordinary).unwrap();
        let link = temp.0.join("linked");
        std::os::unix::fs::symlink(&ordinary, &link).unwrap();
        assert!(Workspace::new(&link.join("work")).is_err());
        assert!(!ordinary.join("work").exists(), "a linked ancestor must not redirect workspace creation");
        let workspace = Workspace::new(&ordinary.join("work")).unwrap();
        assert!(workspace.0.is_dir(), "the canonical ordinary directory remains usable");
    }

    #[test]
    fn allowed_map_assets_never_include_units_ai_or_executables() {
        for path in ["maps/moon.ota", "maps/moon.tnt", "features/moon/rocks.tdf", "objects3d/moonrock.3do", "anims/rocks.gaf", "sounds/moon.wav"] {
            assert!(map_asset(path), "{path}");
        }
        for path in ["ai/default.txt", "units/armcom.fbi", "weapons/cannon.tdf", "gamedata/sidedata.tdf", "palettes/palette.pal", "maps/setup.exe", "maps/nested/moon.tnt",
            "features/../../outside.tdf", "features/..\\..\\outside.tdf", "features/./rocks.tdf",
            "sounds/nul.wav", "features/drive:stream.tdf", "features/alias. /rocks.tdf"] {
            assert!(!map_asset(path), "{path}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn normalized_loose_map_names_cannot_escape_the_private_payload() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        map(&source, "Moon", "Moon");
        std::fs::create_dir(source.join("features")).unwrap();
        // This is one legal Unix filename, as an unpacker can produce from ZIP.
        // The VFS exposes its backslashes as virtual directory separators.
        std::fs::write(source.join("features/..\\..\\..\\..\\canary.tdf"), b"replacement").unwrap();
        let canary = temp.0.join("canary.tdf");
        std::fs::write(&canary, b"keep existing content").unwrap();
        let mut vfs = Vfs::new();
        vfs.mount_mod_dir(&source, "import").unwrap();
        assert!(vfs.exists("features/../../../../canary.tdf"));
        let work = Workspace::new(&temp.0.join("work")).unwrap();
        let mods = temp.0.join("mods");
        assert!(install_map_data(&vfs, &mods, &work.0, "Unsafe map").is_err());
        assert_eq!(std::fs::read(&canary).unwrap(), b"keep existing content");
        assert!(!work.0.join("payload").exists(), "reject before staging any files");
        assert!(!mods.exists(), "nothing is published");
    }
}
