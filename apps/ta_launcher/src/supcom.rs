//! Supreme Commander: Forged Alliance's maps, imported (`ta_supcom`): the game found (GOG's or
//! Steam's copy, or a folder the player picks, kept in the launcher's settings), its skirmish maps
//! and the player's own listed with the game's own pictures of them, and those chosen made into TA
//! maps, one after another on a worker, into one map pack (`map-pack-supreme-commander`), which
//! then shows in the map library and in skirmish. Nothing of the game's is shipped: each player's
//! maps are made from their own copy, the same on every computer. The dialog's other tab imports
//! the game's factions (`crate::supcom_factions`).
//!
//! What both tabs share is here too: a tab's look for the game ([`Look`]: what it found and its
//! pictures, looked for again whenever the tab is shown and its look is not current), an import
//! on a worker ([`Job`], its [`Progress`] by [`Phase`]), where an import's mod is installed and
//! goes ([`Place`]), and the room it needs on the disk.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use anyhow::{Context, Result};
use ta_formats::image::Palette;
use ta_supcom::convert::{self, Options, Stage, kit::Templates};
use ta_supcom::files::Files;
use ta_supcom::install::{self, Install};
use ta_supcom::map::{self, Map, Scenario};
use ta_ui::i18n::{n, t, tf};
use ta_vfs::mods::{ModKind, ModManifest};

use crate::modlist::{APP_FOLDER_LABEL, EXTRA_FOLDER_LABEL, ModFolder, ModList};

/// The map pack the maps go in (a map pack: the library offers to remove it as one).
pub const PACK_ID: &str = "map-pack-supreme-commander";
/// The pack's name and words as its manifest keeps them, in English (a mod's own words): the
/// interface shows them translated ([`pack_name`]; the Mods page translates a description). The
/// name also tells a pack an import made from another mod of its id.
const PACK_NAME: &str = n("Supreme Commander maps");
const PACK_DESCRIPTION: &str = n("Maps of Supreme Commander: Forged Alliance, made into TA maps from your own copy of the game.");
/// The pack's tags, as the factions' mod has its own (the library's filters and search find it by
/// them).
const PACK_TAGS: [&str; 2] = ["maps", "supreme commander"];
/// A map's picture in the list is made at most this many pixels a side (the game's are 256,
/// shown 56 points across).
const PREVIEW_SIDE: u32 = 128;
/// The largest picture of a map read for the list, a side: a player's map may hold one of any
/// size, and one larger is left out (no picture), never decoded.
const PREVIEW_LARGEST: u32 = 2048;
/// The room an import leaves free on the disk beyond what it makes (its working files, the
/// manifest, the disk's own needs).
const SPARE_BYTES: u64 = 64_000_000;
/// The file in the launcher's settings folder (every commander's) that keeps the game's folder as
/// the player chose it.
const CHOICE_FILE: &str = "supcom.toml";

/// The pack's name as the interface shows it, in the language in use.
pub fn pack_name() -> &'static str {
    t(PACK_NAME)
}

/// A mod's name as the interface shows it: this pack's and the factions' in the language in use
/// (their manifests keep them in English), any other's as its manifest has it.
pub fn mod_name(manifest: &ModManifest) -> &str {
    match manifest.id.as_str() {
        PACK_ID if manifest.name.trim() == PACK_NAME => pack_name(),
        crate::supcom_factions::MOD_ID if manifest.name.trim() == crate::supcom_factions::MOD_NAME => crate::supcom_factions::mod_name(),
        _ => &manifest.name,
    }
}

/// The dialog's tabs: the game's maps, or its factions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Maps,
    Factions,
}

impl Tab {
    /// Its name on the dialog's tab strip (marked for translation).
    pub fn label(self) -> &'static str {
        match self {
            Tab::Maps => n("MAPS"),
            Tab::Factions => n("FACTIONS"),
        }
    }
}

/// A map that can be imported.
#[derive(Clone)]
pub struct Entry {
    pub folder: PathBuf,
    /// One of the player's own (downloaded, Forged Alliance Forever's), not the game's.
    pub players_own: bool,
    pub scenario: Scenario,
    pub state: State,
}

/// The game's pictures of the maps a look found, made small (RGBA), by map folder.
pub type Previews = BTreeMap<PathBuf, egui::ColorImage>;

/// Whether a map (or a faction) is in the pack (or the mod) already.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    New,
    Imported,
    /// Imported by an older version of the converter: importing it again makes it better.
    Outdated,
}

impl State {
    pub fn label(self) -> Option<&'static str> {
        match self {
            State::New => None,
            State::Imported => Some(n("IMPORTED")),
            State::Outdated => Some(n("UPDATE")),
        }
    }
}

/// What a look for the game found.
#[derive(Clone, Default)]
pub struct Found {
    /// The copy of the game in use: the player's choice, else the first found.
    pub install: Option<Install>,
    /// Every copy of the game found (GOG's, Steam's...), in the order looked for.
    pub copies: Vec<Install>,
    /// The folder the player chose before, when it no longer holds the game (the copy found is
    /// used meanwhile).
    pub picked_gone: Option<PathBuf>,
    /// The folders of the player's own maps there are; and where they are looked for first,
    /// named when there are none.
    pub own_folders: Vec<PathBuf>,
    pub own_hint: Option<PathBuf>,
    pub maps: Vec<Entry>,
    /// The pack is installed (an import made it).
    pub installed: bool,
    /// Why the game's maps could not be listed (its maps folder would not open).
    pub error: Option<String>,
    /// What could not be read and is not listed: a folder of the player's own maps, a map whose
    /// scenario file would not read (translated, a line each).
    pub skipped: Vec<String>,
}

/// What an import is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// Reading Total Annihilation's and the game's files, before the first map or unit.
    #[default]
    Reading,
    /// Making the maps (or the units), one after another.
    Making,
    /// Installing what was made: it cannot be stopped.
    Installing,
}

/// How an import is going (either tab's).
#[derive(Clone, Default)]
pub struct Progress {
    pub phase: Phase,
    pub done: usize,
    pub total: usize,
    /// The map (or unit) being made.
    pub current: String,
    /// The share of the current map painted (the maps' only).
    pub share: f32,
}

impl Progress {
    /// How far it has got, while it makes the maps or units (no time anyone knows otherwise).
    pub fn fraction(&self) -> Option<f32> {
        (self.phase == Phase::Making && self.total > 0).then(|| ((self.done as f32 + self.share.clamp(0.0, 1.0)) / self.total as f32).clamp(0.0, 1.0))
    }

    /// Which of them is being made (from 1), at most how many there are.
    pub fn number(&self) -> usize {
        (self.done + 1).min(self.total.max(1))
    }
}

/// What an import came to.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub imported: Vec<String>,
    /// The folders (their names, lower case) of the maps made.
    pub made: Vec<String>,
    /// Maps that could not be made, and why.
    pub failed: Vec<(String, String)>,
    pub stopped: bool,
    /// Why the import ended before its last map, the maps made installed all the same (a map
    /// could not be written: the disk full).
    pub cut_short: Option<String>,
    /// Why nothing was installed, when nothing was.
    pub error: Option<String>,
}

/// How an import went, as it is told.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Went {
    /// Every map made.
    Well,
    /// Some made, some not.
    Partly,
    /// Nothing installed: the import failed, or none of the maps could be made.
    Badly,
    /// Stopped before a map was made.
    Nothing,
}

impl Went {
    pub fn colour(self) -> egui::Color32 {
        match self {
            Went::Well => crate::theme::GREEN,
            Went::Partly => crate::theme::AMBER,
            Went::Badly => crate::theme::RED,
            Went::Nothing => crate::theme::STEEL,
        }
    }
}

impl Outcome {
    pub fn went(&self) -> Went {
        if self.error.is_some() || (self.imported.is_empty() && (!self.failed.is_empty() || self.cut_short.is_some())) {
            Went::Badly
        } else if self.imported.is_empty() {
            Went::Nothing
        } else if self.failed.is_empty() && self.cut_short.is_none() {
            Went::Well
        } else {
            Went::Partly
        }
    }

    /// What it came to, as the dialog says it.
    pub fn summary(&self) -> String {
        let (made, failed) = (self.imported.len(), self.failed.len());
        let told = match (&self.error, self.went()) {
            (Some(why), _) => return tf("The maps could not be imported: {why}", &[("why", why)]),
            (None, Went::Badly) if failed == 1 => t("The map could not be made.").to_string(),
            (None, Went::Badly) => tf("None of the {n} maps could be made.", &[("n", &failed)]),
            (None, Went::Nothing) => t("Nothing was imported.").to_string(),
            // (Stopped part of the way: said so, those left over still chosen.)
            (None, Went::Well) | (None, Went::Partly) if failed == 0 && self.stopped => {
                if made == 1 { t("Stopped after 1 map: it is in the map library, and in skirmish. The maps not made yet stay chosen.").to_string() } else { tf("Stopped after {n} maps: they are in the map library, and in skirmish. The maps not made yet stay chosen.", &[("n", &made)]) }
            }
            (None, Went::Well) | (None, Went::Partly) if failed == 0 => {
                if made == 1 { t("Imported 1 map: it is in the map library, and in skirmish.").to_string() } else { tf("Imported {n} maps: they are in the map library, and in skirmish.", &[("n", &made)]) }
            }
            (None, _) if made == 1 => tf("Imported 1 map, but {failed} could not be made: the one made is in the map library, and in skirmish.", &[("failed", &failed)]),
            (None, _) => tf("Imported {n} maps, but {failed} could not be made: those made are in the map library, and in skirmish.", &[("n", &made), ("failed", &failed)]),
        };
        match &self.cut_short {
            Some(why) => tf("{told} The import stopped there: {why}", &[("told", &told), ("why", why)]),
            None => told,
        }
    }

    /// What a notice says of it with the dialog closed, failures and all (nothing for a stop
    /// before any map was made).
    pub fn notice(&self) -> Option<String> {
        let (made, failed) = (self.imported.len(), self.failed.len());
        Some(match (&self.error, self.went()) {
            (Some(why), _) => tf("The Supreme Commander maps could not be imported: {why}", &[("why", why)]),
            (None, Went::Badly) if failed <= 1 => t("The Supreme Commander map could not be made: SUPREME COMMANDER… on the Maps page says why.").to_string(),
            (None, Went::Badly) => tf("None of the {n} Supreme Commander maps could be made: SUPREME COMMANDER… on the Maps page says why.", &[("n", &failed)]),
            (None, Went::Nothing) => return None,
            (None, Went::Well) if made == 1 => t("Imported 1 Supreme Commander map: it is in the map library, and in skirmish.").to_string(),
            (None, Went::Well) => tf("Imported {n} Supreme Commander maps: they are in the map library, and in skirmish.", &[("n", &made)]),
            (None, Went::Partly) if made == 1 => tf("Imported 1 Supreme Commander map, but {failed} could not be made: SUPREME COMMANDER… on the Maps page says why.", &[("failed", &failed.max(1))]),
            (None, Went::Partly) => tf("Imported {n} Supreme Commander maps, but {failed} could not be made: SUPREME COMMANDER… on the Maps page says why.", &[("n", &made), ("failed", &failed.max(1))]),
        })
    }
}

// ---- What both tabs share -----------------------------------------------------------------

/// What a tab's look found, and the pictures it read (by a map's folder, a faction's side).
pub type Seen<F, K> = (F, BTreeMap<K, egui::ColorImage>);

/// A tab's look for the game, made on a worker: what it found (shared: the dialog draws from it
/// every frame) and the pictures it read, as pixels until first shown, then as textures. The
/// pictures are kept only while the dialog is open. A look made for the dialog as it is open is
/// current until the dialog closes or what the look lists changes ([`Look::outdate`]: an import
/// ended); a tab shown looks again unless its look is current, the same rule for both tabs.
pub struct Look<F, K: Ord> {
    scan: Option<mpsc::Receiver<Seen<F, K>>>,
    pub found: Option<Arc<F>>,
    pictures: BTreeMap<K, egui::ColorImage>,
    textures: BTreeMap<K, egui::TextureHandle>,
    current: bool,
    /// The last look ended without a word (its worker failed): the dialog says the look failed,
    /// not that the game is not there.
    pub failed: bool,
    /// What a look finds in a test, in place of looking (never this computer's copy).
    #[cfg(test)]
    pub canned: Option<Arc<dyn Fn() -> Seen<F, K> + Send + Sync>>,
}

impl<F, K: Ord> Default for Look<F, K> {
    fn default() -> Self {
        Look {
            scan: None,
            found: None,
            pictures: BTreeMap::new(),
            textures: BTreeMap::new(),
            current: false,
            failed: false,
            #[cfg(test)]
            canned: None,
        }
    }
}

impl<F: Send + 'static, K: Ord + Send + 'static> Look<F, K> {
    pub fn scanning(&self) -> bool {
        self.scan.is_some()
    }

    /// Whether the look in hand is the dialog's as it is open now, nothing it lists changed since.
    pub fn current(&self) -> bool {
        self.current
    }

    /// Looks (again) on a worker named `name`, `work` doing the looking.
    pub fn start(&mut self, name: &str, work: impl FnOnce() -> Seen<F, K> + Send + 'static) {
        let (send, receive) = mpsc::channel();
        self.scan = Some(receive);
        self.current = false;
        #[cfg(test)]
        if let Some(canned) = self.canned.clone() {
            let _ = send.send(canned());
            return;
        }
        // (A worker that would not start drops `send`: a look that failed, said at the next poll.)
        let _ = std::thread::Builder::new().name(name.into()).spawn(move || {
            let _ = send.send(work());
        });
    }

    /// Takes in a look come in (`Some`: it, and the one it replaces), its pictures kept only for
    /// a dialog `open` to show them; closed, the dialog holds no picture, on the GPU or off it.
    pub fn poll(&mut self, ctx: &egui::Context, open: bool) -> Option<(Arc<F>, Option<Arc<F>>)> {
        let mut arrived = None;
        if let Some(scan) = &self.scan {
            match scan.try_recv() {
                Ok((found, pictures)) => {
                    let found = Arc::new(found);
                    let before = self.found.replace(found.clone());
                    self.textures.clear();
                    self.pictures = if open { pictures } else { BTreeMap::new() };
                    self.current = open;
                    self.failed = false;
                    self.scan = None;
                    arrived = Some((found, before));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.scan = None;
                    self.failed = true;
                    self.current = false;
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            }
        }
        if !open {
            self.let_go();
        }
        arrived
    }

    /// The dialog closed: its pictures let go of, and the look no longer current.
    pub fn let_go(&mut self) {
        self.pictures.clear();
        self.textures.clear();
        self.current = false;
    }

    /// What the look lists may have changed (an import ended): it is looked again when shown.
    pub fn outdate(&mut self) {
        self.current = false;
    }

    /// The picture of `key` as a texture, made from its pixels the first time it is shown (they
    /// are let go of then).
    pub fn texture(&mut self, ctx: &egui::Context, key: &K, name: impl FnOnce() -> String, options: egui::TextureOptions) -> Option<egui::TextureHandle>
    where
        K: Clone,
    {
        if let Some(texture) = self.textures.get(key) {
            return Some(texture.clone());
        }
        let image = self.pictures.remove(key)?;
        let texture = ctx.load_texture(name(), image, options);
        self.textures.insert(key.clone(), texture.clone());
        Some(texture)
    }

    /// How many pictures are held as pixels, and as textures.
    #[cfg(test)]
    pub fn held(&self) -> (usize, usize) {
        (self.pictures.len(), self.textures.len())
    }

    /// A look that found `seen`, taken in at the next poll as a look's are: for tests.
    #[cfg(test)]
    pub fn pretend(&mut self, seen: Seen<F, K>) {
        let (send, receive) = mpsc::channel();
        send.send(seen).unwrap();
        self.scan = Some(receive);
    }
}

/// How an import is asked to end: after the map or unit in hand (`stop`: the player's STOP), or
/// at once, the maps made installed and the one in hand left out (`abandon`: the launcher
/// closing).
#[derive(Default)]
pub struct Ending {
    pub stop: AtomicBool,
    pub abandon: AtomicBool,
}

/// An import on a worker (either tab's), its progress shared with it.
pub struct Job<O> {
    handle: JoinHandle<O>,
    ending: Arc<Ending>,
    progress: Arc<Mutex<Progress>>,
}

impl<O: Send + 'static> Job<O> {
    /// Starts `work` on a worker named `name`, told how it is asked to end.
    pub fn spawn(name: &str, progress: Arc<Mutex<Progress>>, work: impl FnOnce(&Ending) -> O + Send + 'static) -> std::io::Result<Self> {
        let ending = Arc::new(Ending::default());
        let asked = ending.clone();
        let handle = std::thread::Builder::new().name(name.into()).spawn(move || work(&asked))?;
        Ok(Job { handle, ending, progress })
    }

    pub fn finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// Its outcome, `failed`'s where its worker failed.
    pub fn join(self, failed: impl FnOnce() -> O) -> O {
        self.handle.join().unwrap_or_else(|_| failed())
    }

    /// Asks it to stop after the map or unit in hand: not once it installs what it made (that
    /// cannot be stopped, and STOP is greyed then). Whether it was asked. (Asked under the
    /// progress's lock, which the worker holds as it turns to installing: it sees the stop, or
    /// the stop sees it installing, never neither.)
    pub fn stop(&self) -> bool {
        let progress = self.progress.lock().unwrap();
        if progress.phase == Phase::Installing {
            return false;
        }
        self.ending.stop.store(true, Ordering::Relaxed);
        drop(progress);
        true
    }

    /// Asks it to end at once (the launcher closing).
    pub fn abandon(&self) {
        self.ending.abandon.store(true, Ordering::Relaxed);
        self.ending.stop.store(true, Ordering::Relaxed);
    }

    /// It was asked to stop.
    pub fn stopping(&self) -> bool {
        self.ending.stop.load(Ordering::Relaxed)
    }
}

/// Where the launcher finds mods (its folders, in the order it looks in them) and installs them
/// (the settings' mods folder): what the dialog's looks and imports go by.
#[derive(Clone, Debug, Default)]
pub struct Roots {
    pub folders: Vec<ModFolder>,
    pub mods_dir: PathBuf,
}

/// The copy of an import's mod (the map pack, the factions' mod) the launcher uses.
#[derive(Clone, Debug)]
pub struct Installed {
    pub path: PathBuf,
    /// Its name and version as its manifest has them.
    pub name: String,
    pub version: String,
    /// An import made it (its name is the import's): another mod of the same id is not one, and
    /// is not replaced without asking.
    pub ours: bool,
    /// A single archive, which an import cannot replace (a folder beside it would be a second mod
    /// of its id).
    pub archive: bool,
    /// In TA Reborn's program folder, which comes before the mods folder: one made there would
    /// change nothing.
    pub program_folder: bool,
    /// In the extra mods folder, where the player keeps it: made again there.
    pub extra: bool,
}

/// Where an import's mod is now, and the mods folder it is installed in.
#[derive(Clone, Debug, Default)]
pub struct Place {
    pub copy: Option<Installed>,
    pub into: PathBuf,
}

impl Installed {
    /// The copy of `id` the launcher uses, as `mods` lists it, an import's when its name is
    /// `ours`.
    pub fn listed(mods: &ModList, id: &str, ours: &str) -> Option<Installed> {
        let entry = mods.get(id)?;
        let label = mods.folder_label(entry);
        Some(Installed::of(&entry.info, label, ours))
    }

    fn of(info: &ta_vfs::mods::ModInfo, label: &str, ours: &str) -> Installed {
        Installed {
            path: info.path.clone(),
            name: info.manifest.name.clone(),
            version: info.manifest.version.clone(),
            ours: info.manifest.name.trim() == ours,
            archive: info.kind == ModKind::Archive,
            program_folder: label == APP_FOLDER_LABEL,
            extra: label == EXTRA_FOLDER_LABEL,
        }
    }

    /// Why an import cannot take its place, if it cannot.
    pub fn blocks(&self) -> Option<&'static str> {
        if self.program_folder {
            Some(t("This copy is in TA Reborn's program folder, which comes before your mods folder: replace it there to update it."))
        } else if self.archive {
            Some(t("The copy installed is an archive, which an import cannot replace: remove it on the Mods page first."))
        } else {
            None
        }
    }
}

/// The copy of the mod `id` the launcher uses, looked for in `folders` in their order (the first
/// holding one wins, as the launcher's own list has it), with the folder's label.
fn copy_in_use(folders: &[ModFolder], id: &str) -> Option<(ta_vfs::mods::ModInfo, &'static str)> {
    folders.iter().filter(|folder| folder.path.is_dir()).find_map(|folder| {
        ta_vfs::mods::scan(&folder.path).0.into_iter().find(|m| m.manifest.id == id).map(|info| (info, folder.label))
    })
}

/// Where the mod `id` is now, as the launcher finds mods (looked through afresh), an import's when
/// its name is `ours`, and the folder an import of it installs into: the extra mods folder where
/// the copy in use is kept there, else the settings' mods folder.
pub fn place(roots: &Roots, id: &str, ours: &str) -> Place {
    let copy = copy_in_use(&roots.folders, id).map(|(info, label)| Installed::of(&info, label, ours));
    let into = copy.as_ref().filter(|c| c.extra && !c.archive).and_then(|c| c.path.parent().map(Path::to_path_buf)).unwrap_or_else(|| roots.mods_dir.clone());
    Place { copy, into }
}

/// The map pack the launcher uses, as `mods` lists it.
pub fn pack_copy(mods: &ModList) -> Option<Installed> {
    Installed::listed(mods, PACK_ID, PACK_NAME)
}

/// Where the map pack is now (looked for afresh), and where an import installs it.
pub fn pack_place(roots: &Roots) -> Place {
    place(roots, PACK_ID, PACK_NAME)
}

/// The folder the copy in use of `id` is, where it is a folder: what a look reads it from.
fn installed_folder(folders: &[ModFolder], id: &str) -> Option<PathBuf> {
    copy_in_use(folders, id).filter(|(info, _)| info.kind == ModKind::Directory).map(|(info, _)| info.path)
}

/// The tab whose import makes the mod `id` (the map pack, the factions' mod) again, when the copy
/// the launcher uses was made by an older converter than this TA Reborn's: UPDATE on the Mods
/// page. Not another mod of the same id, nor a copy an import cannot replace.
pub fn update_tab(mods: &ModList, id: &str) -> Option<Tab> {
    let (tab, current, ours) = match id {
        PACK_ID => (Tab::Maps, convert::VERSION, PACK_NAME),
        crate::supcom_factions::MOD_ID => (Tab::Factions, crate::supcom_factions::VERSION, crate::supcom_factions::MOD_NAME),
        _ => return None,
    };
    let copy = Installed::listed(mods, id, ours)?;
    let made: u32 = copy.version.trim().parse().unwrap_or(0);
    (copy.ours && copy.blocks().is_none() && made < current).then_some(tab)
}

/// The room free on the disk that holds `path` (or the nearest folder above it that is there),
/// where it can be told.
pub(crate) fn free_space(path: &Path) -> Option<u64> {
    let dir = path.ancestors().find(|p| p.is_dir())?;
    #[cfg(windows)]
    {
        use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let mut free = 0u64;
        // SAFETY: the folder's name is a valid string for the call, and `free` outlives it.
        unsafe { GetDiskFreeSpaceExW(&windows::core::HSTRING::from(dir), Some(&mut free as *mut u64), None, None) }.ok()?;
        Some(free)
    }
    #[cfg(not(windows))]
    {
        // (POSIX `df`: blocks of a kilobyte, the fourth column what may be used.)
        let out = std::process::Command::new("df").arg("-Pk").arg(dir).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines().nth(1)?.split_whitespace().nth(3)?.parse::<u64>().ok().map(|kb| kb * 1024)
    }
}

/// Fails, saying so, when the disks that hold `paths` have less room than `needed` (and a little
/// more): before an import makes anything.
pub(crate) fn enough_room(paths: &[&Path], needed: u64) -> Result<()> {
    for path in paths {
        if let Some(free) = free_space(path)
            && free < needed + SPARE_BYTES
        {
            anyhow::bail!("{}", tf("There is not enough room on the disk: about {needed} is needed, and {free} is free.", &[("needed", &size_text(needed + SPARE_BYTES)), ("free", &size_text(free))]));
        }
    }
    Ok(())
}

/// A size as the dialog tells it: in gigabytes with a decimal from a tenth of one, else in whole
/// megabytes (at least 1), the decimal point as the language in use writes it.
pub fn size_text(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 0.1 {
        tf("{size} GB", &[("size", &one_decimal(gb))])
    } else {
        tf("{size} MB", &[("size", &((bytes as f64 / 1e6).round() as u64).max(1))])
    }
}

/// `value` with one decimal, its point as the language in use writes it: a comma where its
/// thousands are grouped otherwise than with commas (`ta_ui::i18n::grouped`'s languages).
pub fn one_decimal(value: f64) -> String {
    one_decimal_with(value, ta_ui::i18n::grouped(1000).contains(','))
}

/// `value` with one decimal, after a point (`point`) or a comma.
pub fn one_decimal_with(value: f64, point: bool) -> String {
    let text = format!("{value:.1}");
    if point { text } else { text.replace('.', ",") }
}

/// An error's words and its causes', each said once: a cause whose words are already said (the
/// system's error a path's message carries) is left out.
pub(crate) fn error_text(error: &anyhow::Error) -> String {
    let mut text = String::new();
    for cause in error.chain() {
        let words = cause.to_string();
        if words.trim().is_empty() || text.contains(&words) {
            continue;
        }
        if !text.is_empty() {
            text.push_str(": ");
        }
        text.push_str(&words);
    }
    text
}

/// The game's folder as the player chose it, as the launcher's settings keep it (`store_dir`'s).
pub(crate) fn load_choice(store_dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(store_dir.join(CHOICE_FILE)).ok()?;
    let table: toml::Table = toml::from_str(&text).ok()?;
    table.get("game")?.as_str().filter(|path| !path.trim().is_empty()).map(PathBuf::from)
}

/// Keeps the game's folder as the player chose it (`picked`; none: the copy found) in the
/// launcher's settings.
pub(crate) fn save_choice(store_dir: &Path, picked: Option<&Path>) -> std::io::Result<()> {
    let path = store_dir.join(CHOICE_FILE);
    let Some(picked) = picked else {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    };
    let mut table = toml::Table::new();
    table.insert("game".into(), toml::Value::String(picked.to_string_lossy().into_owned()));
    let text = format!("# Where Supreme Commander: Forged Alliance is, as chosen in the launcher.\n{}", toml::to_string(&table).map_err(std::io::Error::other)?);
    std::fs::create_dir_all(store_dir)?;
    let part = store_dir.join(format!("{CHOICE_FILE}.part"));
    std::fs::write(&part, text)?;
    std::fs::rename(&part, &path)
}

/// What a folder the player chose came to, checked on a worker.
enum Checked {
    /// The game, there or one level down.
    Game(PathBuf),
    /// The original Supreme Commander, not Forged Alliance.
    Original(PathBuf),
    Nothing,
}

// ---- The maps' tab, and the dialog --------------------------------------------------------

/// The maps' import: the dialog's state, its maps' tab's, and its factions' tab ([`Factions`]).
#[derive(Default)]
pub struct Import {
    pub open: bool,
    pub tab: Tab,
    /// The game's folder as the player chose it (else the copy found), kept in the settings.
    pub picked: Option<PathBuf>,
    /// The choice changed since the settings last kept it.
    choice_changed: bool,
    /// A folder the player chose, being checked on a worker (a slow or far folder freezes nothing).
    checking: Option<(PathBuf, mpsc::Receiver<Checked>)>,
    /// Why the folder chosen last is not the game: said in the dialog, by the game's folder.
    pub folder_error: Option<String>,
    /// Where the mods are, as the dialog was opened.
    pub(crate) roots: Roots,
    pub(crate) look: Look<Found, PathBuf>,
    /// The maps chosen (their folders). None until the game's maps are first listed, when the
    /// game's own not imported yet are chosen for the player ([`first_choice`]); after that what
    /// the player chose, none at all if so (until the pack is removed: chosen afresh then).
    pub chosen: Option<BTreeSet<PathBuf>>,
    job: Option<Job<Outcome>>,
    pub progress: Arc<Mutex<Progress>>,
    pub outcome: Option<Outcome>,
    /// The last import ended while the dialog showed the factions' tab: said there until this
    /// tab is shown.
    pub unseen: bool,
    /// Maps to choose and import as soon as the game's maps are listed (their folders' names, in
    /// lower case): a capture's (`--show supcom:<folder>,<folder>`).
    pub auto_start: Option<Vec<String>>,
    /// The game's factions, the other tab's.
    pub factions: crate::supcom_factions::Factions,
    /// An import asked for over another mod of its id, waiting for the player's word.
    pub confirm_replace: Option<(Tab, Installed)>,
}

impl Import {
    /// The dialog, closed, the game's folder as the player chose it before (`picked`; none: the
    /// copy found).
    pub fn new(picked: Option<PathBuf>) -> Self {
        Import { picked, ..Default::default() }
    }

    pub fn scanning(&self) -> bool {
        self.look.scanning()
    }

    pub fn importing(&self) -> bool {
        self.job.is_some()
    }

    /// STOP AFTER THIS MAP was pressed: the import ends once the map in hand is made.
    pub fn stopping(&self) -> bool {
        self.job.as_ref().is_some_and(Job::stopping)
    }

    /// A folder the player chose is being checked.
    pub fn checking(&self) -> Option<&Path> {
        self.checking.as_ref().map(|(dir, _)| dir.as_path())
    }

    /// What the last look found.
    pub fn found(&self) -> Option<Arc<Found>> {
        self.look.found.clone()
    }

    /// Opens the dialog at `tab`, or at the other where only its import is under way (the one
    /// shown could not start meanwhile), looking for the game again where the tab's look is not
    /// current: the pictures were let go of when it last closed. (An import under way goes on.)
    pub fn show(&mut self, tab: Tab, roots: Roots) {
        self.open = true;
        self.roots = roots;
        let tab = match tab {
            Tab::Maps if !self.importing() && self.factions.importing() => Tab::Factions,
            Tab::Factions if !self.factions.importing() && self.importing() => Tab::Maps,
            tab => tab,
        };
        self.set_tab(tab);
    }

    /// LOOK AGAIN (the game not found, or the look failed): both tabs look for it afresh, the one
    /// shown at once; the game may have been installed since the dialog opened.
    pub fn look_again(&mut self) {
        self.look.outdate();
        self.factions.look.outdate();
        self.set_tab(self.tab);
    }

    /// Shows the tab `tab`, its look made again unless it is current (the rule both tabs keep).
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        match tab {
            Tab::Maps => {
                self.unseen = false;
                if !self.look.current() && !self.look.scanning() {
                    self.rescan();
                }
            }
            Tab::Factions => {
                self.factions.unseen = false;
                if !self.factions.look.current() && !self.factions.look.scanning() {
                    self.factions.rescan(self.picked.clone(), &self.roots);
                }
            }
        }
    }

    /// Closes the dialog: its pictures let go of (an import under way goes on).
    pub fn close(&mut self) {
        self.open = false;
        self.folder_error = None;
        self.confirm_replace = None;
        self.look.let_go();
        self.factions.look.let_go();
    }

    /// Looks for the game and its maps (in the player's chosen folder, else wherever a copy is).
    pub fn rescan(&mut self) {
        let picked = self.picked.clone();
        let folders = self.roots.folders.clone();
        self.look.start("supcom scan", move || {
            let pack = installed_folder(&folders, PACK_ID);
            find(copies_found(), install::user_map_folders(), install::user_maps_folder(), picked.as_deref(), pack.as_deref())
        });
    }

    /// Whether the map in `folder` is chosen.
    pub fn is_chosen(&self, folder: &Path) -> bool {
        self.chosen.as_ref().is_some_and(|chosen| chosen.contains(folder))
    }

    /// Chooses the map in `folder` (`on`), or not.
    pub fn choose(&mut self, folder: &Path, on: bool) {
        let chosen = self.chosen.get_or_insert_with(BTreeSet::new);
        if on {
            chosen.insert(folder.to_path_buf());
        } else {
            chosen.remove(folder);
        }
    }

    /// Chooses every map listed (`on`), or none.
    pub fn choose_all(&mut self, on: bool) {
        let all = || self.look.found.iter().flat_map(|found| found.maps.iter().map(|e| e.folder.clone())).collect();
        self.chosen = Some(if on { all() } else { BTreeSet::new() });
    }

    /// The picture of the map in `folder` as a texture, made from its pixels the first time it is
    /// shown.
    pub fn texture(&mut self, ctx: &egui::Context, folder: &Path) -> Option<egui::TextureHandle> {
        self.look.texture(ctx, &folder.to_path_buf(), || format!("supcom:{}", folder.display()), egui::TextureOptions::LINEAR)
    }

    /// How many pictures are held as pixels, and as textures.
    #[cfg(test)]
    pub fn pictures_held(&self) -> (usize, usize) {
        self.look.held()
    }

    /// Checks the folder `dir` the player chose, on a worker: the game there (or one level down,
    /// Forged Alliance before the original Supreme Commander) is used once found.
    pub fn check_folder(&mut self, dir: PathBuf) {
        let (send, receive) = mpsc::channel();
        let to_check = dir.clone();
        let _ = std::thread::Builder::new().name("supcom folder".into()).spawn(move || {
            let checked = match install::validate_picked(&to_check) {
                Some(root) if install::is_forged_alliance(&root) => Checked::Game(root),
                Some(root) => Checked::Original(root),
                None => Checked::Nothing,
            };
            let _ = send.send(checked);
        });
        self.checking = Some((dir, receive));
        self.folder_error = None;
    }

    /// Uses the game in `root` (a copy found, or a folder chosen and checked): both tabs list that
    /// copy's then (the factions' where they have been looked at), and the settings keep it.
    pub fn pick(&mut self, root: PathBuf) {
        self.picked = Some(root);
        self.choice_changed = true;
        self.folder_error = None;
        if self.look.found.is_some() || self.tab == Tab::Maps {
            self.rescan();
        }
        if self.factions.look.found.is_some() || self.tab == Tab::Factions {
            self.factions.rescan(self.picked.clone(), &self.roots);
        }
    }

    /// The game's folder as chosen, once, when it changed: for the settings to keep.
    pub fn take_choice(&mut self) -> Option<Option<PathBuf>> {
        std::mem::take(&mut self.choice_changed).then(|| self.picked.clone())
    }

    /// Starts making the chosen maps into TA maps, the original game at `ta_root`, the pack where
    /// `place` says.
    pub fn start(&mut self, ta_root: PathBuf, store_dir: PathBuf, place: Place) {
        let Some(found) = self.look.found.clone() else { return };
        let Some(game) = found.install.clone() else { return };
        if self.importing() {
            return;
        }
        let chosen: Vec<&Entry> = found.maps.iter().filter(|e| self.is_chosen(&e.folder)).collect();
        if chosen.is_empty() {
            return;
        }
        let needed = size_estimate(&chosen);
        let maps: Vec<(PathBuf, bool, String)> = chosen.iter().map(|e| (e.folder.clone(), e.players_own, e.scenario.name.clone())).collect();
        let own = found.own_folders.clone();
        *self.progress.lock().unwrap() = Progress { total: maps.len(), ..Default::default() };
        let progress = self.progress.clone();
        self.outcome = None;
        let job = Job::spawn("supcom import", self.progress.clone(), move |ending| {
            import(&game, &own, &maps, &ta_root, &store_dir, &place, needed, &progress, ending).unwrap_or_else(|e| Outcome { error: Some(error_text(&e)), ..Default::default() })
        });
        match job {
            Ok(job) => self.job = Some(job),
            Err(e) => self.outcome = Some(Outcome { error: Some(e.to_string()), ..Default::default() }),
        }
    }

    /// The maps of `auto_start`, chosen, once the game's maps are listed: true when they are to
    /// be imported now.
    pub fn take_auto_start(&mut self) -> bool {
        if self.scanning() || self.importing() {
            return false;
        }
        let (Some(names), Some(found)) = (&self.auto_start, &self.look.found) else { return false };
        let chosen: BTreeSet<PathBuf> = found.maps.iter().filter(|e| e.folder.file_name().is_some_and(|f| names.iter().any(|n| f.to_string_lossy().eq_ignore_ascii_case(n)))).map(|e| e.folder.clone()).collect();
        let any = !chosen.is_empty();
        self.chosen = Some(chosen);
        self.auto_start = None;
        any
    }

    /// Whether a capture may take its picture: the game looked for, a capture's maps imported
    /// (or, with `TA_SUPCOM_SHOT_IMPORTING=1`, while they are).
    pub fn capture_ready(&self) -> bool {
        let during = std::env::var_os("TA_SUPCOM_SHOT_IMPORTING").is_some_and(|v| v == "1");
        !self.open || (!self.scanning() && self.checking.is_none() && self.auto_start.is_none() && (during || !self.importing()) && self.factions.capture_ready())
    }

    /// Stops the import after the map being made (not once it installs them).
    pub fn stop(&self) {
        if let Some(job) = &self.job {
            job.stop();
        }
    }

    /// Ends the import at once (the launcher closing): the maps made are installed, the one being
    /// made is left out.
    pub fn abandon(&self) {
        if let Some(job) = &self.job {
            job.abandon();
        }
    }

    /// Takes in what the workers found and made; `Some` when an import has ended (the pack
    /// installed when it made maps: they are to be looked through again, the pack turned on).
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<Outcome> {
        if let Some((found, before)) = self.look.poll(ctx, self.open) {
            let same_copy = before.as_ref().is_some_and(|b| b.install == found.install);
            // (The pack removed since: its maps are chosen afresh, as at first.)
            let removed = before.as_ref().is_some_and(|b| b.installed && !found.installed);
            match &mut self.chosen {
                // (What the player chose, less the maps no longer there: none if so.)
                Some(chosen) if same_copy && !removed => chosen.retain(|c| found.maps.iter().any(|e| &e.folder == c)),
                // (The game's maps listed for the first time, or another copy's.)
                _ => self.chosen = Some(first_choice(&found)),
            }
        }
        if let Some((dir, receive)) = &self.checking {
            match receive.try_recv() {
                Ok(checked) => {
                    let dir = dir.clone();
                    self.checking = None;
                    match checked {
                        Checked::Game(root) => self.pick(root),
                        Checked::Original(root) => self.folder_error = Some(tf("{folder} holds the original Supreme Commander, not Forged Alliance: choose the folder Forged Alliance is installed in.", &[("folder", &root.display())])),
                        Checked::Nothing => self.folder_error = Some(tf("Supreme Commander: Forged Alliance is not in {folder}: choose the folder it is installed in (it holds gamedata and maps).", &[("folder", &dir.display())])),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    let dir = dir.clone();
                    self.checking = None;
                    self.folder_error = Some(tf("{folder} could not be looked through: choose another folder.", &[("folder", &dir.display())]));
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            }
        }
        if self.job.as_ref().is_some_and(Job::finished) {
            let job = self.job.take().unwrap();
            let outcome = job.join(|| Outcome { error: Some(t("The import stopped unexpectedly.").to_string()), ..Default::default() });
            // (The maps made are chosen no more: what is left chosen is what was not made, those
            // that could not be among them.)
            if outcome.error.is_none()
                && let Some(chosen) = &mut self.chosen
            {
                let made: BTreeSet<&str> = outcome.made.iter().map(|f| f.as_str()).collect();
                chosen.retain(|f| !made.contains(m_source(f).as_str()));
            }
            self.outcome = Some(outcome.clone());
            return Some(outcome);
        }
        if self.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        None
    }

    /// Ends an import under way at shutdown and waits for it: the maps made are installed (the
    /// pack whole or not at all), the one being made left out. Its outcome, the maps made to be
    /// turned on.
    pub fn finish_on_exit(&mut self) -> Option<Outcome> {
        self.abandon();
        let job = self.job.take()?;
        Some(job.join(Outcome::default))
    }

    /// An import under way that ends, stopped, once asked to stop, having made the maps of
    /// folders `made`: for tests.
    #[cfg(test)]
    pub fn pretend_importing(&mut self, made: &[&str]) {
        let made: Vec<String> = made.iter().map(|m| m.to_string()).collect();
        self.progress.lock().unwrap().phase = Phase::Making;
        let job = Job::spawn("pretend import", self.progress.clone(), move |ending| {
            while !ending.stop.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Outcome { imported: made.clone(), made, stopped: true, ..Default::default() }
        });
        self.job = Some(job.unwrap());
    }

    /// An import under way that comes to `outcome` at once: for tests.
    #[cfg(test)]
    pub fn pretend_import_ending(&mut self, outcome: Outcome) {
        self.job = Some(Job::spawn("pretend import", self.progress.clone(), move |_| outcome).unwrap());
    }

    /// A look that found `found` and the pictures `previews`, taken in at the next
    /// [`Import::poll`] as a look's are: for tests.
    #[cfg(test)]
    pub fn pretend_found(&mut self, found: Found, previews: Previews) {
        self.look.pretend((found, previews));
    }
}

/// The copies of the game on this computer; none with `TA_SUPCOM_MISSING=1` (captures: the dialog
/// as a player without the game sees it).
pub(crate) fn copies_found() -> Vec<Install> {
    if std::env::var_os("TA_SUPCOM_MISSING").is_some_and(|v| v == "1") {
        return Vec::new();
    }
    install::discover()
}

/// The maps chosen for the player when the game's maps are first listed: those an older converter
/// made, where the pack holds some (its update: the Mods page's UPDATE… opens this tab for it, and
/// making every map not imported yet would take twenty minutes); else the game's own not imported
/// yet. The player's own maps are theirs to choose: a vault holds hundreds, gigabytes once made.
fn first_choice(found: &Found) -> BTreeSet<PathBuf> {
    let outdated: BTreeSet<PathBuf> = found.maps.iter().filter(|e| e.state == State::Outdated).map(|e| e.folder.clone()).collect();
    if !outdated.is_empty() {
        return outdated;
    }
    found.maps.iter().filter(|e| !e.players_own && e.state != State::Imported).map(|e| e.folder.clone()).collect()
}

/// The game in use (of the `copies` found, or the folder `picked`, while it still holds the
/// game) and every map that can be imported, the game's and those in the player's own map folders
/// (`own_folders`; where they are looked for first, `own_hint`), with which are in the pack
/// (`pack`, its folder) already; and the game's pictures of them.
fn find(copies: Vec<Install>, own_folders: Vec<PathBuf>, own_hint: Option<PathBuf>, picked: Option<&Path>, pack: Option<&Path>) -> (Found, Previews) {
    // (A folder chosen before that holds the game no more: the copy found is used meanwhile.)
    let picked_gone = picked.filter(|p| !install::validate(p)).map(Path::to_path_buf);
    let install = in_use(&copies, picked.filter(|_| picked_gone.is_none()));
    let mut found = Found { install: install.clone(), copies, picked_gone, own_folders: own_folders.clone(), own_hint, installed: pack.is_some(), ..Default::default() };
    let mut previews = Previews::new();
    let Some(game) = install else { return (found, previews) };
    // What the pack holds: each map's source folder and the converter that made it.
    let made = pack.map(|pack| made_by(&pack.join("maps"))).unwrap_or_default();
    let mut dirs: Vec<(PathBuf, bool)> = own_folders.iter().map(|d| (d.clone(), true)).collect();
    dirs.push((game.maps(), false));
    let mut seen = BTreeSet::new();
    for (dir, players_own) in dirs {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                let why = tf("{folder} could not be read: {why}", &[("folder", &dir.display()), ("why", &e)]);
                if players_own {
                    found.skipped.push(why);
                } else {
                    found.error = Some(why);
                }
                continue;
            }
        };
        let mut folders: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        folders.sort();
        for folder in folders {
            let scenario = match map::read_scenario(&folder) {
                Ok(scenario) => scenario,
                // (A folder without a scenario is no map.)
                Err(ta_supcom::Error::Missing(_)) => continue,
                Err(e) => {
                    let name = folder.file_name().map_or_else(|| folder.display().to_string(), |n| n.to_string_lossy().into_owned());
                    found.skipped.push(tf("{map}: its scenario file could not be read: {why}", &[("map", &name), ("why", &e)]));
                    continue;
                }
            };
            if !scenario.skirmish() || !seen.insert(scenario.folder.to_ascii_lowercase()) {
                continue;
            }
            if let Some(preview) = preview_of(&folder) {
                previews.insert(folder.clone(), preview);
            }
            let state = match made.get(&scenario.folder.to_ascii_lowercase()) {
                Some(&v) if v >= convert::VERSION => State::Imported,
                Some(_) => State::Outdated,
                None => State::New,
            };
            found.maps.push(Entry { folder, players_own, scenario, state });
        }
    }
    // (By name, those to update first: the ones chosen for an update in sight.)
    found.maps.sort_by(|a, b| (a.state != State::Outdated).cmp(&(b.state != State::Outdated))
        .then_with(|| a.scenario.name.to_lowercase().cmp(&b.scenario.name.to_lowercase()))
        .then(a.folder.cmp(&b.folder)));
    (found, previews)
}

/// The maps a pack's `maps` folder holds that were made from Supreme Commander's: each by its
/// source folder (lower case), with the converter's version that made it.
fn made_by(maps: &Path) -> BTreeMap<String, u32> {
    let mut made = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(maps) else { return made };
    for e in entries.flatten() {
        if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("ota"))
            && let Ok(bytes) = std::fs::read(e.path())
        {
            let doc = ta_formats::tdf::parse_bytes(&bytes);
            if let Some(h) = doc.section("GlobalHeader")
                && let Some(source) = h.get("supcommap")
            {
                made.insert(source.trim().to_ascii_lowercase(), h.get_i64_or("supcomversion", 0).max(0) as u32);
            }
        }
    }
    made
}

/// The game's picture of the map in `folder`, made small for the list: none where it has none, or
/// one too large to read (a player's map may hold any size: it is not decoded).
fn preview_of(folder: &Path) -> Option<egui::ColorImage> {
    let (_, _, dds) = ta_supcom::scmap::ScMap::read_preview(&scmap_of(folder)?).ok()?;
    let header = ta_supcom::dds::Header::parse(&dds).ok()?;
    // (Its first level bounded: a smaller one missing, that is the one decoded.)
    if header.width.max(header.height) > PREVIEW_LARGEST {
        return None;
    }
    let image = ta_supcom::dds::decode(&dds, header.level_for(PREVIEW_SIDE)).ok()?;
    (image.rgba.len() == image.width as usize * image.height as usize).then(|| shrink(&image, PREVIEW_SIDE))
}

/// `image` made at most `side` pixels along its longer side (each pixel the mean of those it
/// covers), opaque: the game's pictures leave their alpha unset.
fn shrink(image: &ta_supcom::dds::Image, side: u32) -> egui::ColorImage {
    let (w, h) = (image.width.max(1) as usize, image.height.max(1) as usize);
    let (longer, side) = (w.max(h), side.max(1) as usize);
    let (to_w, to_h) = if longer <= side { (w, h) } else { ((w * side).div_ceil(longer).max(1), (h * side).div_ceil(longer).max(1)) };
    let mut pixels = Vec::with_capacity(to_w * to_h);
    for y in 0..to_h {
        let (top, bottom) = (y * h / to_h, ((y + 1) * h / to_h).max(y * h / to_h + 1));
        for x in 0..to_w {
            let (left, right) = (x * w / to_w, ((x + 1) * w / to_w).max(x * w / to_w + 1));
            let mut sum = [0u32; 3];
            for row in top..bottom {
                for texel in &image.rgba[row * w + left..row * w + right] {
                    for (s, v) in sum.iter_mut().zip(texel) {
                        *s += u32::from(*v);
                    }
                }
            }
            let count = ((bottom - top) * (right - left)) as u32;
            pixels.push(egui::Color32::from_rgb((sum[0] / count) as u8, (sum[1] / count) as u8, (sum[2] / count) as u8));
        }
    }
    egui::ColorImage::new([to_w, to_h], pixels)
}

/// The copy of the game in use: the folder picked (named by its store when it is one of the
/// `copies` found, GOG's or Steam's), else the first found.
pub(crate) fn in_use(copies: &[Install], picked: Option<&Path>) -> Option<Install> {
    match picked {
        Some(root) => Some(copies.iter().find(|copy| install::same_folder(&copy.root, root)).cloned().unwrap_or_else(|| Install { root: root.to_path_buf(), store: install::Store::Folder })),
        None => copies.first().cloned(),
    }
}

/// A map folder's map file (its own `<name>.scmap`, any case; else the first there).
fn scmap_of(folder: &Path) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("scmap"))).collect();
    files.sort();
    files.into_iter().next()
}

/// A map made, as the import writes it into the pack.
struct MadeMap {
    stem: String,
    title: String,
    tnt: Vec<u8>,
    ota: Vec<u8>,
    features: Option<Vec<u8>>,
}

/// Makes `maps` (folder, the player's own, name) into TA maps and installs the pack where `place`
/// says, the maps already in it kept, those made again replaced: when there is room on the disk
/// for `needed` bytes.
#[allow(clippy::too_many_arguments)]
fn import(game: &Install, own: &[PathBuf], maps: &[(PathBuf, bool, String)], ta_root: &Path, store_dir: &Path, place: &Place, needed: u64, progress: &Arc<Mutex<Progress>>, ending: &Ending) -> Result<Outcome> {
    let work_root = store_dir.join("cache").join("map-imports");
    // (The pack's maps kept are linked into the new one, not copied: what the maps chosen take.)
    enough_room(&[&work_root, &place.into], needed)?;
    // TA's own features and palette: from the original game alone, as every player has it.
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(ta_root, "base").context(t("Total Annihilation's own data could not be read."))?;
    let palette = Arc::new(Palette::parse(&vfs.read("palettes/palette.pal")?)?);
    let templates = Arc::new(Templates::load(&vfs));
    let files = Arc::new(Files::open(game, own)?);
    let work = crate::maps::Workspace::new(&work_root)?;
    // (Another mod of the pack's id, replaced as the player agreed, leaves nothing in it.)
    let existing = place.copy.as_ref().filter(|c| c.ours && c.blocks().is_none()).map(|c| c.path.clone());
    let mut convert = |folder: &Path, players_own: bool| {
        convert_one(&files, &templates, &palette, folder, players_own, progress, ending).map(|made| {
            made.map(|c| MadeMap { stem: c.stem, title: c.title, tnt: c.tnt, ota: c.ota, features: c.features })
        })
    };
    let into = place.into.clone();
    make_pack(&work.0, existing.as_deref(), maps, &mut convert, progress, ending, |payload, manifest| {
        ta_vfs::package::install(&ta_vfs::package::plan(payload)?, manifest, &into)?;
        Ok(())
    })
}

/// The pack made in `work`: the maps of `existing` (the pack installed) kept, linked rather than
/// copied where the disk allows it, and `maps` made by `convert` (`None`: the import abandoned)
/// written beside them, a map made again in place of its older self; then installed (`install`,
/// given the folder made and its manifest) when any map was made. A map that cannot be written
/// (the disk full) is said against it, and ends the making there: the maps made so far are
/// installed all the same.
fn make_pack(
    work: &Path,
    existing: Option<&Path>,
    maps: &[(PathBuf, bool, String)],
    convert: &mut dyn FnMut(&Path, bool) -> Option<Result<MadeMap>>,
    progress: &Mutex<Progress>,
    ending: &Ending,
    install: impl FnOnce(&Path, &ModManifest) -> Result<()>,
) -> Result<Outcome> {
    let payload = work.join("payload");
    let maps_out = payload.join("maps");
    std::fs::create_dir_all(&maps_out)?;
    // The maps the pack has, kept (those made again are replaced).
    if let Some(pack) = existing
        && let Ok(entries) = std::fs::read_dir(pack.join("maps"))
    {
        for e in entries.flatten() {
            if e.path().is_file() {
                link_or_copy(&e.path(), &maps_out.join(e.file_name())).with_context(|| e.path().display().to_string())?;
            }
        }
    }
    progress.lock().unwrap().phase = Phase::Making;
    let mut outcome = Outcome::default();
    for (i, (folder, players_own, name)) in maps.iter().enumerate() {
        if ending.stop.load(Ordering::Relaxed) {
            outcome.stopped = true;
            break;
        }
        {
            let mut p = progress.lock().unwrap();
            p.done = i;
            p.current = name.clone();
            p.share = 0.0;
        }
        let Some(made) = convert(folder, *players_own) else {
            // (Abandoned, the launcher closing: the maps made are installed now.)
            outcome.stopped = true;
            break;
        };
        match made {
            Ok(map) => {
                // (An older version of this map, under its name or another, goes; it comes back,
                // as the pack installed has it, if the new one is not written whole.)
                let older = remove_made_from(&maps_out, &m_source(folder));
                let restore = |maps_out: &Path| {
                    if let Some(pack) = existing {
                        for stem in &older {
                            restore_map_files(&pack.join("maps"), maps_out, stem);
                        }
                    }
                };
                // (Another map's files of the same name are never written over.)
                if taken(&maps_out, &map.stem) {
                    restore(&maps_out);
                    outcome.failed.push((name.clone(), tf("Another map in the pack has the file name {stem}.", &[("stem", &map.stem)])));
                    continue;
                }
                match write_map(&maps_out, &map) {
                    Ok(()) => {
                        outcome.imported.push(map.title);
                        outcome.made.push(m_source(folder));
                    }
                    Err(e) => {
                        // (The disk full, most likely: what this map wrote goes and its older
                        // version comes back, the maps made are installed, the rest left chosen
                        // for later.)
                        remove_map_files(&maps_out, &map.stem);
                        restore(&maps_out);
                        outcome.failed.push((name.clone(), tf("It could not be written: {why}", &[("why", &e)])));
                        outcome.cut_short = Some(e.to_string());
                        break;
                    }
                }
            }
            // (The converter's own words, about the map's files, in ours.)
            Err(e) => outcome.failed.push((name.clone(), tf("It could not be made into a TA map: {why}", &[("why", &error_text(&e))]))),
        }
    }
    progress.lock().unwrap().done = outcome.imported.len() + outcome.failed.len();
    if outcome.imported.is_empty() {
        return Ok(outcome);
    }
    // (STOP asked for from now changes nothing: the maps made are installed either way.)
    progress.lock().unwrap().phase = Phase::Installing;
    // (The pack's version is its oldest map's converter's: one made by an older converter than
    // this TA Reborn's is offered as an update.)
    let version = made_by(&maps_out).values().copied().min().unwrap_or(convert::VERSION);
    let manifest = ModManifest { id: PACK_ID.into(), name: PACK_NAME.into(), version: version.to_string(), description: PACK_DESCRIPTION.into(), tags: PACK_TAGS.map(String::from).to_vec(), ..Default::default() };
    install(&payload, &manifest)?;
    Ok(outcome)
}

/// `to` as a second name of the file `from` (on the same disk, it takes no room), else a copy.
fn link_or_copy(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::hard_link(from, to).or_else(|_| std::fs::copy(from, to).map(|_| ()))
}

/// Writes a map's files into `maps`: each a new file, never written through a name it shares with
/// the pack installed ([`link_or_copy`]).
fn write_map(maps: &Path, map: &MadeMap) -> std::io::Result<()> {
    let files = [("tnt", Some(&map.tnt)), ("ota", Some(&map.ota)), ("features", map.features.as_ref())];
    for (ext, bytes) in files {
        let Some(bytes) = bytes else { continue };
        let path = maps.join(format!("{}.{ext}", map.stem));
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        std::fs::write(&path, bytes)?;
    }
    Ok(())
}

/// Removes the files a map of stem `stem` has in `maps`.
fn remove_map_files(maps: &Path, stem: &str) {
    for ext in ["tnt", "ota", "features"] {
        let _ = std::fs::remove_file(maps.join(format!("{stem}.{ext}")));
    }
}

/// Makes the map in `folder` into a TA map on a thread of its own, the import waiting for it:
/// `None` when the import is abandoned meanwhile (the thread is left to the launcher's closing:
/// a large map takes minutes, and it writes nothing).
fn convert_one(files: &Arc<Files>, templates: &Arc<Templates>, palette: &Arc<Palette>, folder: &Path, players_own: bool, progress: &Arc<Mutex<Progress>>, ending: &Ending) -> Option<Result<convert::Converted>> {
    let (send, receive) = mpsc::channel();
    let (files, templates, palette, folder, progress) = (files.clone(), templates.clone(), palette.clone(), folder.to_path_buf(), progress.clone());
    let spawned = std::thread::Builder::new().name("supcom map".into()).spawn(move || {
        let made = Map::read(&files, &folder).map_err(anyhow::Error::from).and_then(|m| {
            Ok(convert::convert(&m, &files, &templates, &palette, players_own, &Options::default(), &|s| {
                if let Stage::Painting(share) = s {
                    progress.lock().unwrap().share = share;
                }
            })?)
        });
        let _ = send.send(made);
    });
    if let Err(e) = spawned {
        return Some(Err(e.into()));
    }
    loop {
        match receive.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(made) => return Some(made),
            Err(mpsc::RecvTimeoutError::Timeout) if ending.abandon.load(Ordering::Relaxed) => return None,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // (Its thread ended without a word: it failed.)
            Err(mpsc::RecvTimeoutError::Disconnected) => return Some(Err(anyhow::anyhow!(t("The map's conversion stopped unexpectedly.").to_string()))),
        }
    }
}

/// The source folder's name as a converted map's OTA records it (`supcommap`).
fn m_source(folder: &Path) -> String {
    folder.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Removes the files of the map made from `source` (its OTA saying so) from `maps`: the stems
/// of those removed.
fn remove_made_from(maps: &Path, source: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(maps) else { return Vec::new() };
    let mut removed = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if !path.extension().is_some_and(|x| x.eq_ignore_ascii_case("ota")) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let doc = ta_formats::tdf::parse_bytes(&bytes);
        if doc.section("GlobalHeader").and_then(|h| h.get("supcommap")).is_some_and(|s| s.trim().eq_ignore_ascii_case(source)) {
            for ext in ["tnt", "ota", "features"] {
                let _ = std::fs::remove_file(path.with_extension(ext));
            }
            if let Some(stem) = path.file_stem() {
                removed.push(stem.to_string_lossy().into_owned());
            }
        }
    }
    removed.sort();
    removed
}

/// Puts the files a map of stem `stem` has in `from` (the pack installed's maps) back in `maps`,
/// each where `maps` has none: linked, as the pack's maps kept are ([`link_or_copy`]).
fn restore_map_files(from: &Path, maps: &Path, stem: &str) {
    for ext in ["tnt", "ota", "features"] {
        let (source, target) = (from.join(format!("{stem}.{ext}")), maps.join(format!("{stem}.{ext}")));
        if source.is_file() && !target.exists() {
            let _ = link_or_copy(&source, &target);
        }
    }
}

/// Whether `maps` holds a map of file stem `stem` (whatever its case: the game finds maps so).
fn taken(maps: &Path, stem: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(maps) else { return false };
    entries.flatten().any(|e| {
        let path = e.path();
        path.extension().is_some_and(|x| x.eq_ignore_ascii_case("tnt") || x.eq_ignore_ascii_case("ota"))
            && path.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(stem))
    })
}

/// What importing `entries` takes on the disk, about (bytes): a map's art grows with its size.
pub fn size_estimate(entries: &[&Entry]) -> u64 {
    entries.iter().map(|e| match e.scenario.size.0.max(e.scenario.size.1) {
        0..=300 => 17_000_000u64,
        301..=600 => 21_000_000,
        _ => 27_000_000,
    }).sum()
}

/// About how long the maps take to make on a fast computer, in seconds, by their size: measured on
/// a 32-thread desktop, where the game's 54 take about 20 minutes, a 5 km map seconds and an 81 km
/// one about a minute.
pub fn time_estimate(entries: &[&Entry]) -> u64 {
    entries.iter().map(|e| match e.scenario.size.0.max(e.scenario.size.1) {
        0..=300 => 3u64,
        301..=600 => 10,
        601..=1100 => 25,
        1101..=2100 => 40,
        _ => 65,
    }).sum()
}

/// A time to make as the dialog says it: under a minute, about a minute, about 20 minutes.
pub fn time_text(seconds: u64) -> String {
    match (seconds + 30) / 60 {
        0 => t("under a minute").to_string(),
        1 => t("about a minute").to_string(),
        minutes => tf("about {n} minutes", &[("n", &minutes)]),
    }
}

/// The size a map is called by in the game (`20 km`).
pub fn km(scenario: &Scenario) -> String {
    let side = scenario.size.0.max(scenario.size.1);
    tf("{n} km", &[("n", &(side / 50))]).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ta-supcom-{name}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_map_made_again_replaces_its_older_files_only() {
        let dir = temp("import");
        let maps = dir.join("maps");
        std::fs::create_dir_all(&maps).unwrap();
        for (stem, source) in [("Old Name (SC)", "SCMP_001"), ("Other (SC)", "SCMP_002")] {
            std::fs::write(maps.join(format!("{stem}.ota")), format!("[GlobalHeader]{{missionname={stem};supcommap={source};supcomversion=1;}}")).unwrap();
            std::fs::write(maps.join(format!("{stem}.tnt")), b"tnt").unwrap();
        }
        remove_made_from(&maps, "scmp_001");
        assert!(!maps.join("Old Name (SC).ota").exists() && !maps.join("Old Name (SC).tnt").exists());
        assert!(maps.join("Other (SC).ota").exists());
        // Another map's name is taken, whatever its case; the one gone is free.
        assert!(taken(&maps, "other (sc)"));
        assert!(!taken(&maps, "Old Name (SC)"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A copy of the game's maps listed: its skirmish maps (not its campaign's), in name order,
    /// those the pack holds marked as imported (or as made by an older converter).
    #[test]
    fn the_games_skirmish_maps_are_listed_with_what_the_pack_holds() {
        let dir = temp("find");
        let game = dir.join("Supreme Commander Forged Alliance");
        std::fs::create_dir_all(game.join("gamedata")).unwrap();
        std::fs::write(game.join("gamedata/env.scd"), b"PK").unwrap();
        let scenario = |folder: &str, name: &str, kind: &str| {
            let f = game.join("maps").join(folder);
            std::fs::create_dir_all(&f).unwrap();
            std::fs::write(f.join(format!("{folder}_scenario.lua")), format!("ScenarioInfo = {{ name = '{name}', type = '{kind}', size = {{512, 512}}, Configurations = {{ ['standard'] = {{ teams = {{ {{ name = 'FFA', armies = {{'ARMY_1','ARMY_2'}} }} }} }} }} }}")).unwrap();
        };
        scenario("SCMP_002", "Open Palms", "skirmish");
        scenario("SCMP_001", "Zulu Ridge", "skirmish");
        scenario("SCMP_003", "Beta Fork", "skirmish");
        scenario("X1CA_001", "X1CA_001", "campaign");
        // (A scenario that does not read is said, not passed over in silence; a folder with none
        // is no map.)
        std::fs::create_dir_all(game.join("maps/BROKEN")).unwrap();
        std::fs::write(game.join("maps/BROKEN/BROKEN_scenario.lua"), "ScenarioInfo = {{{").unwrap();
        std::fs::create_dir_all(game.join("maps/notes")).unwrap();
        // The pack where the installer put it, its first name taken: found by its id.
        let mods = dir.join("mods");
        std::fs::create_dir_all(mods.join(PACK_ID)).unwrap();
        std::fs::write(mods.join(PACK_ID).join("mod.toml"), "id = \"someone-elses\"\nname = \"Not ours\"\n").unwrap();
        let pack = mods.join(format!("{PACK_ID}-2"));
        std::fs::create_dir_all(pack.join("maps")).unwrap();
        std::fs::write(pack.join("mod.toml"), format!("id = \"{PACK_ID}\"\nname = \"Supreme Commander maps\"\n")).unwrap();
        std::fs::write(pack.join("maps/Open Palms (SC).ota"), format!("[GlobalHeader]{{supcommap=SCMP_002;supcomversion={};}}", convert::VERSION)).unwrap();
        std::fs::write(pack.join("maps/Beta Fork (SC).ota"), "[GlobalHeader]{supcommap=scmp_003;supcomversion=0;}").unwrap();
        let folders = vec![ModFolder { label: ta_vfs::mods::USER_FOLDER_LABEL, path: mods.clone(), exists: true }];
        assert_eq!(installed_folder(&folders, PACK_ID), Some(pack.clone()));
        let (found, previews) = find(Vec::new(), Vec::new(), None, Some(&game), installed_folder(&folders, PACK_ID).as_deref());
        let listed: Vec<(&str, State)> = found.maps.iter().map(|e| (e.scenario.name.as_str(), e.state)).collect();
        assert_eq!(listed, [("Beta Fork", State::Outdated), ("Open Palms", State::Imported), ("Zulu Ridge", State::New)]);
        assert_eq!(found.install.as_ref().map(|i| i.root.clone()), Some(game.clone()));
        assert!(found.installed && found.picked_gone.is_none());
        assert!(found.maps.iter().all(|e| !e.players_own && e.scenario.armies.len() == 2));
        assert!(previews.is_empty(), "no map file, no picture");
        assert_eq!(found.skipped.len(), 1, "{:?}", found.skipped);
        assert!(found.skipped[0].starts_with("BROKEN: "), "{:?}", found.skipped);
        assert!(found.error.is_none());
        // A folder chosen before that holds the game no more: said, and nothing listed (no other
        // copy found).
        let (none, _) = find(Vec::new(), Vec::new(), None, Some(&dir.join("elsewhere")), Some(&pack));
        assert!(none.maps.is_empty() && none.install.is_none());
        assert_eq!(none.picked_gone, Some(dir.join("elsewhere")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A map of the list, for the tests.
    fn entry(folder: &str, players_own: bool, state: State) -> Entry {
        let name = Path::new(folder).file_name().unwrap().to_string_lossy().into_owned();
        Entry { folder: PathBuf::from(folder), players_own, scenario: Scenario { folder: name.clone(), name, ..Default::default() }, state }
    }

    fn chosen(import: &Import) -> Vec<String> {
        import.chosen.iter().flatten().map(|f| f.to_string_lossy().replace('\\', "/")).collect()
    }

    /// At first the game's own maps not imported yet are chosen (not the player's own, perhaps
    /// hundreds); after an import only what was not made stays chosen, and the look that follows
    /// keeps the player's choice, even none at all; another copy of the game chooses afresh, and
    /// so does the pack removed.
    #[test]
    fn the_first_choice_is_the_games_maps_and_an_import_leaves_what_was_not_made() {
        use ta_supcom::install::Store;
        let ctx = egui::Context::default();
        let gog = Install { root: PathBuf::from("C:/Game"), store: Store::Gog };
        let found = |install: &Install, states: [State; 3]| Found {
            install: Some(install.clone()),
            maps: vec![entry("C:/Game/maps/SCMP_001", false, states[0]), entry("C:/Game/maps/SCMP_002", false, states[1]), entry("C:/Game/maps/SCMP_003", false, states[2]),
                entry("D:/Own/maps/mine.v0001", true, State::New)],
            installed: states.contains(&State::Imported) || states.contains(&State::Outdated),
            ..Default::default()
        };
        let mut import = Import { open: true, ..Default::default() };
        import.pretend_found(found(&gog, [State::New, State::Imported, State::Outdated]), Previews::new());
        import.poll(&ctx);
        assert_eq!(chosen(&import), ["C:/Game/maps/SCMP_003"], "a pack to update: what an older converter made, alone, not every map not imported yet");
        // The player adds a map of the game's and their own; those two are made, SCMP_003 is not.
        import.choose(Path::new("C:/Game/maps/SCMP_001"), true);
        import.choose(Path::new("D:/Own/maps/mine.v0001"), true);
        import.pretend_importing(&["scmp_001", "mine.v0001"]);
        import.stop();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while import.poll(&ctx).is_none() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(chosen(&import), ["C:/Game/maps/SCMP_003"], "only what was not made stays chosen");
        // The look after the import does not choose every map not imported again.
        import.pretend_found(found(&gog, [State::Imported, State::Imported, State::Outdated]), Previews::new());
        import.poll(&ctx);
        assert_eq!(chosen(&import), ["C:/Game/maps/SCMP_003"]);
        import.choose_all(false);
        import.pretend_found(found(&gog, [State::Imported, State::Imported, State::Imported]), Previews::new());
        import.poll(&ctx);
        assert!(chosen(&import).is_empty(), "none chosen is the player's choice too");
        // REMOVE MAP PACK: the game's maps are chosen afresh, not IMPORT 0 MAPS.
        import.pretend_found(found(&gog, [State::New, State::New, State::New]), Previews::new());
        import.poll(&ctx);
        assert_eq!(chosen(&import), ["C:/Game/maps/SCMP_001", "C:/Game/maps/SCMP_002", "C:/Game/maps/SCMP_003"]);
        import.choose_all(true);
        assert_eq!(chosen(&import).len(), 4);
        // Another copy of the game: its own maps not imported yet.
        let steam = Install { root: PathBuf::from("C:/Steam/Game"), store: Store::Steam };
        import.pretend_found(found(&steam, [State::Imported, State::New, State::Imported]), Previews::new());
        import.poll(&ctx);
        assert_eq!(chosen(&import), ["C:/Game/maps/SCMP_002"]);
    }

    /// The game's pictures are made small for the list, a player's map's picture too large to
    /// read left out undecoded; each is uploaded once (its pixels let go of then), and all of them
    /// are let go of when the dialog closes, a look that comes in meanwhile keeping none.
    #[test]
    fn map_pictures_are_bounded_made_small_and_let_go_of_with_the_dialog() {
        let dir = temp("pictures");
        // A map file holding a picture: an uncompressed DDS (BGRA, 32 bits) or a DXT1 one.
        let map_file = |folder: &str, width: u32, height: u32, dxt1: bool| {
            let mut dds = vec![0u8; 128];
            let put = |dds: &mut Vec<u8>, at: usize, v: u32| dds[at..at + 4].copy_from_slice(&v.to_le_bytes());
            dds[..4].copy_from_slice(b"DDS ");
            put(&mut dds, 4, 124);
            put(&mut dds, 12, height);
            put(&mut dds, 16, width);
            put(&mut dds, 28, 1);
            put(&mut dds, 76, 32);
            if dxt1 {
                put(&mut dds, 80, 0x4);
                dds[84..88].copy_from_slice(b"DXT1");
                dds.resize(128 + (width as usize / 4) * (height as usize / 4) * 8, 0x55);
            } else {
                put(&mut dds, 80, 0x41);
                for (at, mask) in [(88, 32), (92, 0x00ff_0000), (96, 0x0000_ff00), (100, 0x0000_00ff), (104, 0xff00_0000)] {
                    put(&mut dds, at, mask);
                }
                // (Left half white, right half black; the alpha unset, as the game leaves it.)
                for _y in 0..height {
                    for x in 0..width {
                        dds.extend_from_slice(if x < width / 2 { &[255, 255, 255, 0] } else { &[0, 0, 0, 0] });
                    }
                }
            }
            let f = dir.join(folder);
            std::fs::create_dir_all(&f).unwrap();
            let mut file = b"Map\x1a".to_vec();
            file.extend_from_slice(&[0; 26]);
            file.extend_from_slice(&(dds.len() as i32).to_le_bytes());
            file.extend_from_slice(&dds);
            for v in [60i32, 512, 512, 0] {
                file.extend_from_slice(&v.to_le_bytes());
            }
            std::fs::write(f.join(format!("{folder}.scmap")), file).unwrap();
            f
        };
        let game = map_file("SCMP_001", 256, 256, false);
        let preview = preview_of(&game).expect("the game's picture");
        assert_eq!(preview.size, [128, 128]);
        assert_eq!((preview.pixels[0], preview.pixels[127]), (egui::Color32::WHITE, egui::Color32::BLACK), "opaque, halves kept");
        let wide = map_file("wide.v0001", 64, 16, false);
        assert_eq!(preview_of(&wide).unwrap().size, [64, 16], "a small picture is not made larger");
        let huge = map_file("huge.v0001", 4096, 2048, true);
        assert!(preview_of(&huge).is_none(), "a picture far larger than the list's is not decoded");
        // Uploaded once, and its pixels let go of; all let go of when the dialog closes.
        let ctx = egui::Context::default();
        let mut import = Import { open: true, ..Default::default() };
        let previews: Previews = [(game.clone(), preview.clone()), (wide.clone(), preview.clone())].into();
        import.pretend_found(Found { maps: vec![entry("SCMP_001", false, State::New)], ..Default::default() }, previews.clone());
        import.poll(&ctx);
        assert_eq!(import.pictures_held(), (2, 0));
        let texture = import.texture(&ctx, &game).expect("its texture");
        assert_eq!(import.texture(&ctx, &game).map(|t| t.id()), Some(texture.id()), "made once");
        assert_eq!(import.pictures_held(), (1, 1));
        import.close();
        import.poll(&ctx);
        assert_eq!(import.pictures_held(), (0, 0), "closed, the dialog holds no picture");
        import.pretend_found(Found::default(), previews);
        import.poll(&ctx);
        assert_eq!(import.pictures_held(), (0, 0), "nor from a look that came in meanwhile");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The dialog reopened by either route looks again for the tab it shows, whose pictures went
    /// with its closing: from the Maps page at the maps; from the Mods page at the factions, then
    /// at the maps when their tab is shown (not left with dark squares and stale marks); and at
    /// the factions while they are being made.
    #[test]
    fn the_dialog_reopened_by_either_route_looks_again_for_its_pictures() {
        use crate::supcom_factions::{self as factions, Pictures};
        let ctx = egui::Context::default();
        let maps_seen = || -> Seen<Found, PathBuf> {
            let folder = PathBuf::from("C:/Game/maps/SCMP_001");
            (Found { maps: vec![entry("C:/Game/maps/SCMP_001", false, State::New)], ..Default::default() }, [(folder, egui::ColorImage::filled([8, 8], egui::Color32::RED))].into())
        };
        let factions_seen = || -> Seen<factions::Found, &'static str> { (factions::Found::default(), Pictures::from([("UEF", egui::ColorImage::filled([8, 8], egui::Color32::BLUE))])) };
        let mut import = Import::default();
        import.look.canned = Some(Arc::new(maps_seen));
        import.factions.look.canned = Some(Arc::new(factions_seen));
        let take_in = |import: &mut Import| {
            import.poll(&ctx);
            import.factions.poll(&ctx, import.open);
        };
        // From the Maps page: the maps looked for, their pictures held while it is open.
        import.show(Tab::Maps, Roots::default());
        take_in(&mut import);
        assert_eq!(import.pictures_held(), (1, 0));
        import.close();
        take_in(&mut import);
        assert_eq!(import.pictures_held(), (0, 0));
        // From the Mods page: the factions looked for; the maps, once their tab is shown.
        import.show(Tab::Factions, Roots::default());
        take_in(&mut import);
        assert_eq!(import.factions.pictures_held(), (1, 0));
        assert!(!import.scanning(), "the maps not looked for before their tab is shown");
        import.set_tab(Tab::Maps);
        assert!(import.scanning(), "the maps looked for again: their pictures went with the dialog");
        take_in(&mut import);
        assert_eq!(import.pictures_held(), (1, 0), "not dark squares");
        // Shown again within the open dialog: the looks are current, none made again.
        import.set_tab(Tab::Factions);
        assert!(!import.factions.look.scanning());
        // Reopened from the Maps page while the factions are made: at the factions, looked for
        // again (their pictures back) though their import goes on.
        import.close();
        take_in(&mut import);
        import.factions.pretend_importing();
        import.show(Tab::Maps, Roots::default());
        assert_eq!(import.tab, Tab::Factions, "at the tab whose import is under way");
        assert!(import.factions.look.scanning());
        take_in(&mut import);
        assert_eq!(import.factions.pictures_held(), (1, 0));
        import.factions.stop();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while import.factions.importing() && std::time::Instant::now() < until {
            import.factions.poll(&ctx, true);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// An import is told as it went: every map made, some not (amber, not green), none made or
    /// the import failed (red), a stop before any map was made (no notice); one map is told as
    /// one, and a map that could not be written ends the import, the maps made kept.
    #[test]
    fn an_import_is_told_as_it_went() {
        let failed = || vec![("Seton's Clutch".to_string(), "It could not be made into a TA map: bad".to_string())];
        let well = Outcome { imported: vec!["Open Palms".into()], ..Default::default() };
        let partly = Outcome { imported: vec!["Open Palms".into()], failed: failed(), ..Default::default() };
        let badly = Outcome { failed: failed(), ..Default::default() };
        let broken = Outcome { error: Some("disk full".into()), ..Default::default() };
        let nothing = Outcome { stopped: true, ..Default::default() };
        let full = Outcome { imported: vec!["Open Palms".into(), "Zulu Ridge".into()], failed: failed(), cut_short: Some("There is not enough space on the disk.".into()), ..Default::default() };
        assert_eq!([&well, &partly, &badly, &broken, &nothing, &full].map(Outcome::went), [Went::Well, Went::Partly, Went::Badly, Went::Badly, Went::Nothing, Went::Partly]);
        assert_eq!(partly.went().colour(), crate::theme::AMBER);
        assert!(partly.notice().unwrap().contains("but 1 could not be made"));
        assert_eq!(badly.notice().unwrap(), "The Supreme Commander map could not be made: SUPREME COMMANDER… on the Maps page says why.");
        assert_eq!(badly.summary(), "The map could not be made.");
        assert!(broken.notice().unwrap().ends_with("disk full") && broken.summary().ends_with("disk full"));
        assert!(nothing.notice().is_none());
        assert!(well.notice().unwrap().starts_with("Imported 1 Supreme Commander map:"), "{:?}", well.notice());
        assert!(well.summary().starts_with("Imported 1 map: it is"), "{}", well.summary());
        assert!(full.summary().starts_with("Imported 2 maps, but 1 could not be made") && full.summary().ends_with("The import stopped there: There is not enough space on the disk."), "{}", full.summary());
    }

    /// A map made, for the tests.
    fn made(stem: &str, source: &str) -> MadeMap {
        MadeMap {
            stem: stem.into(),
            title: stem.into(),
            tnt: format!("tnt of {stem}").into_bytes(),
            ota: format!("[GlobalHeader]{{missionname={stem};supcommap={source};supcomversion={};}}", convert::VERSION).into_bytes(),
            features: Some(b"[features]{}".to_vec()),
        }
    }

    /// The import itself, its maps made by a stand-in for the converter: the pack's maps kept are
    /// linked (a map made again never writes through to the pack installed); a map that cannot be
    /// written (here its file's place is taken by a folder, as a full disk refuses it) is said
    /// against it and ends the making, the maps made so far installed all the same, its own files
    /// gone; the launcher closing installs those made before the map in hand; and nothing made,
    /// nothing installed.
    #[test]
    fn the_import_installs_what_it_made_when_a_map_cannot_be_written_or_it_is_abandoned() {
        let dir = temp("make");
        let pack = dir.join("mods").join(PACK_ID);
        std::fs::create_dir_all(pack.join("maps")).unwrap();
        std::fs::write(pack.join("maps/Kept (SC).tnt"), b"kept tnt").unwrap();
        std::fs::write(pack.join("maps/Kept (SC).ota"), "[GlobalHeader]{supcommap=scmp_009;supcomversion=0;}").unwrap();
        std::fs::write(pack.join("maps/Again (SC).tnt"), b"old tnt").unwrap();
        std::fs::write(pack.join("maps/Again (SC).ota"), "[GlobalHeader]{supcommap=scmp_001;supcomversion=0;}").unwrap();
        // (The third map is in the pack already: made again, it cannot be written.)
        std::fs::write(pack.join("maps/Third (SC).tnt"), b"old third tnt").unwrap();
        std::fs::write(pack.join("maps/Third (SC).ota"), "[GlobalHeader]{supcommap=scmp_003;supcomversion=0;}").unwrap();
        let maps: Vec<(PathBuf, bool, String)> = ["SCMP_001", "SCMP_002", "SCMP_003"].iter().map(|f| (PathBuf::from(format!("C:/Game/maps/{f}")), false, f.to_string())).collect();
        let stems = |folder: &Path| match m_source(folder).as_str() { "scmp_001" => "Again (SC)", "scmp_002" => "Second (SC)", _ => "Third (SC)" };
        let progress = Mutex::new(Progress::default());
        let installed = std::cell::RefCell::new(None::<(Vec<String>, String)>);
        let install = |payload: &Path, manifest: &ModManifest| {
            let mut files: Vec<String> = std::fs::read_dir(payload.join("maps")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            files.sort();
            *installed.borrow_mut() = Some((files, manifest.version.clone()));
            Ok::<(), anyhow::Error>(())
        };
        // The third map cannot be written: its features' file has a folder in its place.
        let work = dir.join("work-full");
        std::fs::create_dir_all(work.join("payload/maps/Third (SC).features")).unwrap();
        let mut convert = |folder: &Path, _: bool| Some(Ok::<_, anyhow::Error>(made(stems(folder), &m_source(folder))));
        let outcome = make_pack(&work, Some(&pack), &maps, &mut convert, &progress, &Ending::default(), install).unwrap();
        assert_eq!(outcome.imported, ["Again (SC)", "Second (SC)"]);
        assert_eq!(outcome.failed.len(), 1);
        assert!(outcome.failed[0].0 == "SCMP_003" && outcome.failed[0].1.starts_with("It could not be written: "), "{:?}", outcome.failed);
        assert!(outcome.cut_short.is_some() && outcome.went() == Went::Partly);
        let (files, version) = installed.borrow_mut().take().expect("the maps made are installed");
        assert_eq!(
            files,
            ["Again (SC).features", "Again (SC).ota", "Again (SC).tnt", "Kept (SC).ota", "Kept (SC).tnt", "Second (SC).features", "Second (SC).ota", "Second (SC).tnt", "Third (SC).features", "Third (SC).ota", "Third (SC).tnt"],
            "the third map's new files gone (its folder in the way is not one), its older version back"
        );
        assert_eq!(std::fs::read(work.join("payload/maps/Third (SC).tnt")).unwrap(), b"old third tnt", "the map that could not be written keeps the version the pack had");
        assert_eq!(std::fs::read(pack.join("maps/Third (SC).tnt")).unwrap(), b"old third tnt");
        assert_eq!(version, "0", "the pack's version is its oldest map's");
        assert_eq!(std::fs::read(pack.join("maps/Again (SC).tnt")).unwrap(), b"old tnt", "the pack installed is not written through");
        assert_eq!(std::fs::read(work.join("payload/maps/Kept (SC).tnt")).unwrap(), b"kept tnt", "the maps kept are in the new pack");
        assert_eq!(progress.lock().unwrap().phase, Phase::Installing);
        // Abandoned while the second map is made: the first is installed, the rest left out.
        let mut calls = 0;
        let mut convert = |folder: &Path, _: bool| {
            calls += 1;
            (calls == 1).then(|| Ok::<_, anyhow::Error>(made(stems(folder), &m_source(folder))))
        };
        let outcome = make_pack(&dir.join("work-closing"), Some(&pack), &maps, &mut convert, &progress, &Ending::default(), install).unwrap();
        assert!(outcome.stopped && outcome.imported == ["Again (SC)"]);
        assert!(installed.borrow().is_some());
        // Nothing made: nothing installed.
        *installed.borrow_mut() = None;
        let mut convert = |_: &Path, _: bool| Some(Err::<MadeMap, _>(anyhow::anyhow!("bad")));
        let outcome = make_pack(&dir.join("work-none"), None, &maps, &mut convert, &progress, &Ending::default(), install).unwrap();
        assert!(outcome.imported.is_empty() && outcome.failed.len() == 3 && outcome.went() == Went::Badly);
        assert!(installed.borrow().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A map made again under a name another map of the pack has is not written, and its own
    /// older version, under its old name, stays in the pack.
    #[test]
    fn a_map_made_again_under_a_name_taken_keeps_its_older_version() {
        let dir = temp("taken");
        let pack = dir.join("mods").join(PACK_ID);
        std::fs::create_dir_all(pack.join("maps")).unwrap();
        std::fs::write(pack.join("maps/Kept (SC).tnt"), b"kept tnt").unwrap();
        std::fs::write(pack.join("maps/Kept (SC).ota"), "[GlobalHeader]{supcommap=scmp_009;supcomversion=0;}").unwrap();
        std::fs::write(pack.join("maps/Old Name (SC).tnt"), b"old tnt").unwrap();
        std::fs::write(pack.join("maps/Old Name (SC).ota"), "[GlobalHeader]{supcommap=scmp_002;supcomversion=0;}").unwrap();
        let maps = vec![(PathBuf::from("C:/Game/maps/SCMP_002"), false, "SCMP_002".to_string())];
        let progress = Mutex::new(Progress::default());
        let installed = std::cell::RefCell::new(None::<Vec<String>>);
        let install = |payload: &Path, _: &ModManifest| {
            let mut files: Vec<String> = std::fs::read_dir(payload.join("maps")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            files.sort();
            *installed.borrow_mut() = Some(files);
            Ok::<(), anyhow::Error>(())
        };
        // (Made again, it would take the other map's name.)
        let mut convert = |folder: &Path, _: bool| Some(Ok::<_, anyhow::Error>(made("Kept (SC)", &m_source(folder))));
        let work = dir.join("work");
        let outcome = make_pack(&work, Some(&pack), &maps, &mut convert, &progress, &Ending::default(), install).unwrap();
        assert!(outcome.imported.is_empty() && outcome.failed.len() == 1, "{:?}", outcome.failed);
        assert!(outcome.failed[0].1.starts_with("Another map in the pack has the file name"), "{:?}", outcome.failed);
        assert!(installed.borrow().is_none(), "nothing made, nothing installed");
        let mut files: Vec<String> = std::fs::read_dir(work.join("payload/maps")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        files.sort();
        assert_eq!(files, ["Kept (SC).ota", "Kept (SC).tnt", "Old Name (SC).ota", "Old Name (SC).tnt"], "its older version back beside the other map");
        assert_eq!(std::fs::read(work.join("payload/maps/Kept (SC).tnt")).unwrap(), b"kept tnt", "the other map is not written over");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Too little room on the disk refuses an import before anything is made, and says how much
    /// is needed and free; sizes read in megabytes below a tenth of a gigabyte.
    #[test]
    fn an_import_needs_room_on_the_disk() {
        let dir = temp("room");
        assert!(enough_room(&[&dir], 1).is_ok());
        let refused = enough_room(&[&dir.join("not made yet")], u64::MAX / 4).unwrap_err().to_string();
        assert!(refused.starts_with("There is not enough room on the disk: about "), "{refused}");
        assert_eq!(size_text(1_234_000_000), "1.2 GB");
        assert_eq!(size_text(17_000_000), "17 MB");
        assert_eq!(size_text(1), "1 MB");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The system's error is said once, not again after the path it is about.
    #[test]
    fn an_error_is_said_once() {
        let os = std::io::Error::from_raw_os_error(183);
        let words = os.to_string();
        let error = anyhow::Error::from(ta_vfs::package::PackageError::Io { path: "C:/mods".into(), source: os });
        assert_eq!(format!("{error:#}"), format!("C:/mods: {words}: {words}"), "as it was told");
        assert_eq!(error_text(&error), format!("C:/mods: {words}"));
        let wrapped = anyhow::anyhow!("outer").context("context");
        assert_eq!(error_text(&wrapped), "context: outer");
    }

    /// The folder chosen is kept in the launcher's settings (none: the copy found), and read back.
    #[test]
    fn the_folder_chosen_is_kept_in_the_settings() {
        let dir = temp("choice");
        assert_eq!(load_choice(&dir), None);
        let game = PathBuf::from(r"D:\Games\Supreme Commander Forged Alliance");
        save_choice(&dir, Some(&game)).unwrap();
        assert_eq!(load_choice(&dir), Some(game));
        save_choice(&dir, None).unwrap();
        assert_eq!(load_choice(&dir), None);
        save_choice(&dir, None).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Where an import's mod is: the copy the launcher uses (the program folder's before the mods
    /// folder's, the extra folder's after them), whether an import made it, and where it is made
    /// again: in the extra folder where the copy is kept there.
    #[test]
    fn an_imports_mod_is_found_where_the_launcher_finds_mods() {
        let dir = temp("place");
        let mod_at = |folder: &str, name: &str| {
            let at = dir.join(folder).join(PACK_ID);
            std::fs::create_dir_all(&at).unwrap();
            std::fs::write(at.join("mod.toml"), format!("id = \"{PACK_ID}\"\nname = \"{name}\"\nversion = \"0\"\n")).unwrap();
            at
        };
        let folder = |label: &'static str, name: &str| ModFolder { label, path: dir.join(name), exists: true };
        let roots = Roots { folders: vec![folder(APP_FOLDER_LABEL, "program"), folder(ta_vfs::mods::USER_FOLDER_LABEL, "mods"), folder(EXTRA_FOLDER_LABEL, "extra")], mods_dir: dir.join("mods") };
        let none = place(&roots, PACK_ID, PACK_NAME);
        assert!(none.copy.is_none() && none.into == roots.mods_dir);
        let extra = mod_at("extra", PACK_NAME);
        let kept = place(&roots, PACK_ID, PACK_NAME);
        assert_eq!(kept.copy.as_ref().map(|c| (c.path.clone(), c.ours, c.extra)), Some((extra.clone(), true, true)));
        assert_eq!(kept.into, dir.join("extra"), "made again where it is kept");
        mod_at("mods", "Bob's own maps");
        let theirs = place(&roots, PACK_ID, PACK_NAME);
        assert_eq!(theirs.copy.as_ref().map(|c| (c.name.as_str(), c.ours)), Some(("Bob's own maps", false)), "another mod of its id, before the extra folder's");
        mod_at("program", PACK_NAME);
        let program = place(&roots, PACK_ID, PACK_NAME);
        assert!(program.copy.as_ref().is_some_and(|c| c.program_folder && c.blocks().is_some()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The imports' mods made by an older converter than this TA Reborn's are updates on the Mods
    /// page (its UPDATES, the rail's count), made again at their tab; not one made by this
    /// converter, nor another mod of their id.
    #[test]
    fn an_imports_mod_made_by_an_older_converter_is_an_update() {
        use crate::mod_library::{Facts, Filter, Library};
        let dir = temp("update");
        let write = |id: &str, name: &str, version: &str| {
            std::fs::create_dir_all(dir.join(id)).unwrap();
            std::fs::write(dir.join(id).join("mod.toml"), format!("id = \"{id}\"\nname = \"{name}\"\nversion = \"{version}\"\n")).unwrap();
        };
        let list = || ModList::scan(vec![ModFolder { label: ta_vfs::mods::USER_FOLDER_LABEL, path: dir.clone(), exists: true }], &[]);
        let updates = |list: &ModList| {
            let status = |_: &str| crate::validation::ValidationStatus::Unknown;
            let facts = Facts { mods: list, enabled: &[], status: &status, hidden: &[] };
            let mut library = Library::default();
            library.filter = Filter::Updates;
            library.visible(&facts).into_iter().map(|item| item.id).filter(|id| id == PACK_ID || id == crate::supcom_factions::MOD_ID).collect::<Vec<_>>()
        };
        write(PACK_ID, PACK_NAME, "0");
        write(crate::supcom_factions::MOD_ID, crate::supcom_factions::MOD_NAME, &crate::supcom_factions::VERSION.to_string());
        assert_eq!(update_tab(&list(), PACK_ID), Some(Tab::Maps));
        assert_eq!(update_tab(&list(), crate::supcom_factions::MOD_ID), None, "made by this converter");
        assert_eq!(updates(&list()), [PACK_ID]);
        write(crate::supcom_factions::MOD_ID, crate::supcom_factions::MOD_NAME, "1");
        assert_eq!(update_tab(&list(), crate::supcom_factions::MOD_ID), Some(Tab::Factions));
        write(PACK_ID, "Bob's own maps", "0");
        assert_eq!(update_tab(&list(), PACK_ID), None, "another mod of the pack's id is not the import's to update");
        assert_eq!(updates(&list()), [crate::supcom_factions::MOD_ID]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A folder chosen is looked through on a worker: the original Supreme Commander, or no game,
    /// said in the dialog (not in a notice under it); Forged Alliance there is used, and kept.
    #[test]
    fn a_folder_chosen_is_checked_on_a_worker_and_told_in_the_dialog() {
        let dir = temp("checked");
        let ctx = egui::Context::default();
        let mut import = Import { open: true, ..Default::default() };
        import.look.canned = Some(Arc::new(|| (Found::default(), Previews::new())));
        let checked = |import: &mut Import| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while import.checking().is_some() && std::time::Instant::now() < until {
                import.poll(&ctx);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        };
        import.check_folder(dir.clone());
        assert_eq!(import.checking(), Some(dir.as_path()), "looked through on a worker");
        checked(&mut import);
        assert!(import.folder_error.as_deref().is_some_and(|e| e.starts_with("Supreme Commander: Forged Alliance is not in ")), "{:?}", import.folder_error);
        let original = dir.join("Supreme Commander");
        std::fs::create_dir_all(original.join("gamedata")).unwrap();
        std::fs::write(original.join("gamedata/env.scd"), b"PK").unwrap();
        std::fs::create_dir_all(original.join("maps/SCMP_001")).unwrap();
        import.check_folder(dir.clone());
        checked(&mut import);
        assert!(import.folder_error.as_deref().is_some_and(|e| e.contains("the original Supreme Commander")), "{:?}", import.folder_error);
        let forged = dir.join("Supreme Commander Forged Alliance");
        std::fs::create_dir_all(forged.join("gamedata")).unwrap();
        std::fs::write(forged.join("gamedata/env.scd"), b"PK").unwrap();
        std::fs::create_dir_all(forged.join("maps/X1MP_001")).unwrap();
        import.check_folder(dir.clone());
        checked(&mut import);
        assert_eq!((import.folder_error.clone(), import.picked.clone()), (None, Some(forged.clone())), "Forged Alliance before the original");
        assert_eq!(import.take_choice(), Some(Some(forged)), "kept in the settings");
        assert_eq!(import.take_choice(), None, "once");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// With GOG's and Steam's copies both found: the first in use unless one is picked, a copy
    /// picked keeping its store's name, any other folder picked used as a folder.
    #[test]
    fn the_copy_in_use_is_the_one_picked_with_its_stores_name() {
        use ta_supcom::install::Store;
        let gog = Install { root: PathBuf::from("C:/GOG Games/Supreme Commander Forged Alliance"), store: Store::Gog };
        let steam = Install { root: PathBuf::from("C:/Program Files (x86)/Steam/steamapps/common/Supreme Commander Forged Alliance"), store: Store::Steam };
        let copies = [gog.clone(), steam.clone()];
        assert_eq!(in_use(&copies, None), Some(gog));
        assert_eq!(in_use(&copies, Some(&steam.root)), Some(steam));
        let elsewhere = PathBuf::from("D:/Games/Forged Alliance");
        assert_eq!(in_use(&copies, Some(&elsewhere)), Some(Install { root: elsewhere, store: Store::Folder }));
        assert_eq!(in_use(&[], None), None);
    }

    #[test]
    fn sizes_are_told_as_the_game_tells_them() {
        let s = Scenario { size: (1024, 1024), ..Default::default() };
        assert_eq!(km(&s), "20 km");
        let e = Entry { folder: PathBuf::new(), players_own: false, scenario: s, state: State::New };
        assert_eq!(size_estimate(&[&e, &e]), 54_000_000);
    }

    /// "About 1.2 GB" in English, "etwa 1,2 GB" in German (whose thousands are grouped with
    /// points).
    #[test]
    fn sizes_are_written_with_the_languages_decimal_point() {
        assert_eq!(one_decimal(1.234), "1.2", "English");
        assert_eq!(one_decimal_with(9.68, false), "9,7");
    }
}
