//! Supreme Commander: Forged Alliance's maps, imported (`ta_supcom`): the game found (GOG's or
//! Steam's copy, or a folder the player picks), its skirmish maps and the player's own listed
//! with the game's own pictures of them, and those chosen made into TA maps, one after another
//! on a worker, into one map pack (`map-pack-supreme-commander`), which then shows in the map
//! library and in skirmish. Nothing of the game's is shipped: each player's maps are made from
//! their own copy, the same on every computer.

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
use ta_vfs::mods::ModManifest;

/// The map pack the maps go in (a map pack: the library offers to remove it as one).
pub const PACK_ID: &str = "map-pack-supreme-commander";

/// A map that can be imported.
#[derive(Clone)]
pub struct Entry {
    pub folder: PathBuf,
    /// One of the player's own (downloaded, Forged Alliance Forever's), not the game's.
    pub players_own: bool,
    pub scenario: Scenario,
    /// The game's own picture of it (RGBA, its size).
    pub preview: Option<(u32, u32, Vec<u8>)>,
    pub state: State,
}

/// Whether a map is in the pack already.
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
    pub install: Option<Install>,
    /// The folders of the player's own maps there are.
    pub own_folders: Vec<PathBuf>,
    pub maps: Vec<Entry>,
    /// Why the game's maps could not be listed (its archives would not open).
    pub error: Option<String>,
}

/// How an import is going.
#[derive(Clone, Default)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub current: String,
    /// The share of the current map painted.
    pub share: f32,
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
    /// Why nothing was installed, when nothing was.
    pub error: Option<String>,
}

struct Job {
    handle: JoinHandle<Outcome>,
    ending: Arc<Ending>,
}

/// How an import is asked to end: after the map in hand (`stop`: the player's STOP), or at once,
/// the maps made installed and the one in hand left out (`abandon`: the launcher closing).
#[derive(Default)]
struct Ending {
    stop: AtomicBool,
    abandon: AtomicBool,
}

#[derive(Default)]
pub struct Import {
    pub open: bool,
    /// The game's folder as the player chose it (else the copy found).
    pub picked: Option<PathBuf>,
    scan: Option<mpsc::Receiver<Found>>,
    pub found: Option<Found>,
    /// The maps chosen (their folders).
    pub chosen: BTreeSet<PathBuf>,
    job: Option<Job>,
    pub progress: Arc<Mutex<Progress>>,
    pub outcome: Option<Outcome>,
    /// The pictures, as textures, by map folder.
    pub textures: BTreeMap<PathBuf, egui::TextureHandle>,
    /// Maps to choose and import as soon as the game's maps are listed (their folders' names, in
    /// lower case): a capture's (`--show supcom:<folder>,<folder>`).
    pub auto_start: Option<Vec<String>>,
}

impl Import {
    pub fn scanning(&self) -> bool {
        self.scan.is_some()
    }

    pub fn importing(&self) -> bool {
        self.job.is_some()
    }

    /// Opens the dialog and looks for the game (again).
    pub fn show(&mut self, mods_dir: &Path) {
        self.open = true;
        if !self.importing() {
            self.rescan(mods_dir);
        }
    }

    /// Looks for the game and its maps (in the player's chosen folder, else wherever a copy is).
    pub fn rescan(&mut self, mods_dir: &Path) {
        let (send, receive) = mpsc::channel();
        let picked = self.picked.clone();
        let pack = mods_dir.join(PACK_ID);
        std::thread::Builder::new().name("supcom scan".into()).spawn(move || {
            let _ = send.send(find(picked.as_deref(), &pack));
        }).ok();
        self.scan = Some(receive);
    }

    /// Uses the game in folder `picked` (or one level down in it); false when it is not there.
    pub fn pick(&mut self, picked: &Path, mods_dir: &Path) -> bool {
        match install::validate_picked(picked) {
            Some(root) => {
                self.picked = Some(root);
                self.rescan(mods_dir);
                true
            }
            None => false,
        }
    }

    /// Starts making the chosen maps into TA maps, the original game at `ta_root`.
    pub fn start(&mut self, ta_root: PathBuf, store_dir: PathBuf) {
        let Some(found) = &self.found else { return };
        let Some(game) = found.install.clone() else { return };
        if self.importing() {
            return;
        }
        let maps: Vec<(PathBuf, bool, String)> = found.maps.iter().filter(|e| self.chosen.contains(&e.folder)).map(|e| (e.folder.clone(), e.players_own, e.scenario.name.clone())).collect();
        if maps.is_empty() {
            return;
        }
        let own = found.own_folders.clone();
        let ending = Arc::new(Ending::default());
        *self.progress.lock().unwrap() = Progress { total: maps.len(), ..Default::default() };
        let progress = self.progress.clone();
        let asked = ending.clone();
        self.outcome = None;
        let handle = std::thread::Builder::new().name("supcom import".into()).spawn(move || {
            import(&game, &own, &maps, &ta_root, &store_dir, &progress, &asked).unwrap_or_else(|e| Outcome { error: Some(format!("{e:#}")), ..Default::default() })
        });
        match handle {
            Ok(handle) => self.job = Some(Job { handle, ending }),
            Err(e) => self.outcome = Some(Outcome { error: Some(e.to_string()), ..Default::default() }),
        }
    }

    /// The maps of `auto_start`, chosen, once the game's maps are listed: true when they are to
    /// be imported now.
    pub fn take_auto_start(&mut self) -> bool {
        if self.scanning() || self.importing() {
            return false;
        }
        let (Some(names), Some(found)) = (&self.auto_start, &self.found) else { return false };
        self.chosen = found.maps.iter().filter(|e| e.folder.file_name().is_some_and(|f| names.iter().any(|n| f.to_string_lossy().eq_ignore_ascii_case(n)))).map(|e| e.folder.clone()).collect();
        self.auto_start = None;
        !self.chosen.is_empty()
    }

    /// Whether a capture may take its picture: the game looked for, a capture's maps imported
    /// (or, with `TA_SUPCOM_SHOT_IMPORTING=1`, while they are).
    pub fn capture_ready(&self) -> bool {
        let during = std::env::var_os("TA_SUPCOM_SHOT_IMPORTING").is_some_and(|v| v == "1");
        !self.open || (!self.scanning() && self.auto_start.is_none() && (during || !self.importing()))
    }

    /// Stops the import after the map being made.
    pub fn stop(&self) {
        if let Some(job) = &self.job {
            job.ending.stop.store(true, Ordering::Relaxed);
        }
    }

    /// Ends the import at once (the launcher closing): the maps made are installed, the one being
    /// made is left out.
    pub fn abandon(&self) {
        if let Some(job) = &self.job {
            job.ending.abandon.store(true, Ordering::Relaxed);
            job.ending.stop.store(true, Ordering::Relaxed);
        }
    }

    /// Takes in what the workers found and made; `Some` when an import has just installed the
    /// pack (its maps to be looked through again, the pack turned on).
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<Outcome> {
        if let Some(scan) = &self.scan {
            match scan.try_recv() {
                Ok(found) => {
                    // (Every map not yet imported is chosen at first; then what the player chose.)
                    if self.chosen.is_empty() || self.found.as_ref().is_none_or(|f| f.install != found.install) {
                        self.chosen = found.maps.iter().filter(|e| e.state != State::Imported).map(|e| e.folder.clone()).collect();
                    } else {
                        self.chosen.retain(|c| found.maps.iter().any(|e| &e.folder == c));
                    }
                    self.textures.clear();
                    self.found = Some(found);
                    self.scan = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.scan = None,
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            }
        }
        if self.job.as_ref().is_some_and(|j| j.handle.is_finished()) {
            let job = self.job.take().unwrap();
            let outcome = job.handle.join().unwrap_or_else(|_| Outcome { error: Some(t("The import stopped unexpectedly.").to_string()), ..Default::default() });
            // (The maps made are chosen no more: what is left chosen is what was not made.)
            if outcome.error.is_none() {
                let made: BTreeSet<&str> = outcome.made.iter().map(|f| f.as_str()).collect();
                self.chosen.retain(|f| !made.contains(m_source(f).as_str()));
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
        job.handle.join().ok()
    }

    /// An import under way that ends, stopped, once asked to stop, having made the maps of
    /// folders `made`: for tests.
    #[cfg(test)]
    pub fn pretend_importing(&mut self, made: &[&str]) {
        let ending = Arc::new(Ending::default());
        let asked = ending.clone();
        let made: Vec<String> = made.iter().map(|m| m.to_string()).collect();
        let handle = std::thread::spawn(move || {
            while !asked.stop.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Outcome { imported: made.clone(), made, stopped: true, ..Default::default() }
        });
        self.job = Some(Job { handle, ending });
    }
}

/// The game, the player's own map folders and every map that can be imported, with which are in
/// the pack (`pack`, its folder) already.
fn find(picked: Option<&Path>, pack: &Path) -> Found {
    let install = match picked {
        Some(root) => Some(Install { root: root.to_path_buf(), store: install::Store::Folder }),
        None => install::discover().into_iter().next(),
    };
    let own_folders = install::user_map_folders();
    let mut found = Found { install: install.clone(), own_folders: own_folders.clone(), ..Default::default() };
    let Some(game) = install else { return found };
    // What the pack holds: each map's source folder and the converter that made it.
    let mut made: BTreeMap<String, u32> = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(pack.join("maps")) {
        for e in entries.flatten() {
            if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("ota"))
                && let Ok(bytes) = std::fs::read(e.path())
            {
                let doc = ta_formats::tdf::parse_bytes(&bytes);
                if let Some(h) = doc.section("GlobalHeader")
                    && let Some(source) = h.get("supcommap")
                {
                    made.insert(source.trim().to_ascii_lowercase(), h.get_i64_or("supcomversion", 0) as u32);
                }
            }
        }
    }
    let mut dirs: Vec<(PathBuf, bool)> = own_folders.iter().map(|d| (d.clone(), true)).collect();
    dirs.push((game.maps(), false));
    let mut seen = BTreeSet::new();
    for (dir, players_own) in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut folders: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        folders.sort();
        for folder in folders {
            let Ok(scenario) = map::read_scenario(&folder) else { continue };
            if !scenario.skirmish() || !seen.insert(scenario.folder.to_ascii_lowercase()) {
                continue;
            }
            let preview = scmap_of(&folder).and_then(|p| ta_supcom::scmap::ScMap::read_preview(&p).ok()).and_then(|(_, _, dds)| ta_supcom::dds::decode(&dds, 0).ok()).map(|i| {
                (i.width, i.height, i.rgba.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect())
            });
            let state = match made.get(&scenario.folder.to_ascii_lowercase()) {
                Some(&v) if v >= convert::VERSION => State::Imported,
                Some(_) => State::Outdated,
                None => State::New,
            };
            found.maps.push(Entry { folder, players_own, scenario, preview, state });
        }
    }
    found.maps.sort_by(|a, b| a.scenario.name.to_lowercase().cmp(&b.scenario.name.to_lowercase()).then(a.folder.cmp(&b.folder)));
    found
}

/// A map folder's map file (its own `<name>.scmap`, any case; else the first there).
fn scmap_of(folder: &Path) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("scmap"))).collect();
    files.sort();
    files.into_iter().next()
}

/// Makes `maps` (folder, the player's own, name) into TA maps and installs the pack: the maps
/// already in it kept, those made again replaced.
fn import(game: &Install, own: &[PathBuf], maps: &[(PathBuf, bool, String)], ta_root: &Path, store_dir: &Path, progress: &Arc<Mutex<Progress>>, ending: &Ending) -> Result<Outcome> {
    // TA's own features and palette: from the original game alone, as every player has it.
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(ta_root, "base").context(t("Total Annihilation's own data could not be read."))?;
    let palette = Arc::new(Palette::parse(&vfs.read("palettes/palette.pal")?)?);
    let templates = Arc::new(Templates::load(&vfs));
    let files = Arc::new(Files::open(game, own)?);
    let mods_dir = store_dir.join("mods");
    let work = crate::maps::Workspace::new(&store_dir.join("cache").join("map-imports"))?;
    let payload = work.0.join("payload");
    let maps_out = payload.join("maps");
    std::fs::create_dir_all(&maps_out)?;
    // The maps the pack has, kept (those made again are written over).
    if let Ok(entries) = std::fs::read_dir(mods_dir.join(PACK_ID).join("maps")) {
        for e in entries.flatten() {
            if e.path().is_file() {
                std::fs::copy(e.path(), maps_out.join(e.file_name()))?;
            }
        }
    }
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
        let Some(made) = convert_one(&files, &templates, &palette, folder, *players_own, progress, ending) else {
            // (Abandoned, the launcher closing: the maps made are installed now.)
            outcome.stopped = true;
            break;
        };
        match made {
            Ok(c) => {
                // (An older version of this map, under another name, goes.)
                remove_made_from(&maps_out, &m_source(folder));
                // (Another map's files of the same name are never written over.)
                if taken(&maps_out, &c.stem) {
                    outcome.failed.push((name.clone(), tf("Another map in the pack has the file name {stem}.", &[("stem", &c.stem)]).to_string()));
                    continue;
                }
                std::fs::write(maps_out.join(format!("{}.tnt", c.stem)), &c.tnt)?;
                std::fs::write(maps_out.join(format!("{}.ota", c.stem)), &c.ota)?;
                if let Some(f) = &c.features {
                    std::fs::write(maps_out.join(format!("{}.features", c.stem)), f)?;
                }
                outcome.imported.push(c.title);
                outcome.made.push(m_source(folder));
            }
            Err(e) => outcome.failed.push((name.clone(), format!("{e:#}"))),
        }
    }
    progress.lock().unwrap().done = outcome.imported.len() + outcome.failed.len();
    if outcome.imported.is_empty() {
        return Ok(outcome);
    }
    let manifest = ModManifest {
        id: PACK_ID.into(),
        name: "Supreme Commander maps".into(),
        version: convert::VERSION.to_string(),
        description: "Maps of Supreme Commander: Forged Alliance, made into TA maps from your own copy of the game.".into(),
        authors: Vec::new(),
        depends: Vec::new(),
        load_after: Vec::new(),
        source: String::new(),
        rename: BTreeMap::new(),
        replaces: Vec::new(),
    };
    ta_vfs::package::install(&ta_vfs::package::plan(&payload)?, &manifest, &mods_dir)?;
    Ok(outcome)
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

/// Removes the files of the map made from `source` (its OTA saying so) from `maps`.
fn remove_made_from(maps: &Path, source: &str) {
    let Ok(entries) = std::fs::read_dir(maps) else { return };
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

/// The size a map is called by in the game (`20 km`).
pub fn km(scenario: &Scenario) -> String {
    let side = scenario.size.0.max(scenario.size.1);
    tf("{n} km", &[("n", &(side / 50))]).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_made_again_replaces_its_older_files_only() {
        let dir = std::env::temp_dir().join(format!("ta-supcom-import-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
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
        let dir = std::env::temp_dir().join(format!("ta-supcom-find-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
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
        let pack = dir.join("mods").join(PACK_ID);
        std::fs::create_dir_all(pack.join("maps")).unwrap();
        std::fs::write(pack.join("maps/Open Palms (SC).ota"), format!("[GlobalHeader]{{supcommap=SCMP_002;supcomversion={};}}", convert::VERSION)).unwrap();
        std::fs::write(pack.join("maps/Beta Fork (SC).ota"), "[GlobalHeader]{supcommap=scmp_003;supcomversion=0;}").unwrap();
        let found = find(Some(&game), &pack);
        let listed: Vec<(&str, State)> = found.maps.iter().map(|e| (e.scenario.name.as_str(), e.state)).collect();
        assert_eq!(listed, [("Beta Fork", State::Outdated), ("Open Palms", State::Imported), ("Zulu Ridge", State::New)]);
        assert_eq!(found.install.as_ref().map(|i| i.root.clone()), Some(game.clone()));
        assert!(found.maps.iter().all(|e| !e.players_own && e.scenario.armies.len() == 2));
        // No game there: nothing to list, and said so.
        let none = find(Some(&dir.join("elsewhere")), &pack);
        assert!(none.maps.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sizes_are_told_as_the_game_tells_them() {
        let s = Scenario { size: (1024, 1024), ..Default::default() };
        assert_eq!(km(&s), "20 km");
        let e = Entry { folder: PathBuf::new(), players_own: false, scenario: s, preview: None, state: State::New };
        assert_eq!(size_estimate(&[&e, &e]), 54_000_000);
    }
}
