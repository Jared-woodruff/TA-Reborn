//! Supreme Commander: Forged Alliance's four factions, imported (`ta_supcom::factions`): the
//! playable units of the copy of the game in use counted by faction, and those of the factions
//! chosen made into TA units on a worker, into one mod (`supcom-factions`), which then plays in
//! skirmish, a side each. As with the maps (`crate::supcom`, whose look, job and places this tab
//! shares), nothing of the game's is shipped: each player's units are made from their own copy,
//! the same on every computer. The mod is made whole each time, of the factions chosen: a stop, or
//! a failure, leaves the one installed as it was.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use ta_formats::image::Palette;
use ta_supcom::factions::{faction, make, pictures, units as made_units};
use ta_supcom::files::Files;
use ta_supcom::install::{self, Install};
use ta_supcom::unit::{self, Faction, Motion, UnitBlueprint};
use ta_ui::i18n::{n, t, tf};

use crate::supcom::{Ending, Installed, Job, Look, Phase, Place, Progress, Roots, State, Went, error_text};

pub use make::{MOD_ID, VERSION};

/// The mod's name and words as its manifest keeps them (`make`'s), marked to be translated: the
/// interface shows them in the language in use ([`mod_name`]; the Mods page translates a
/// description). The name also tells the mod an import made from another of its id.
pub const MOD_NAME: &str = n("Supreme Commander factions");
// (The description is shown through the Mods page's own translation of a manifest's words: kept
// here to be collected for translation, and held to `make`'s by a test.)
pub(crate) const MOD_DESCRIPTION: &str = n("Supreme Commander: Forged Alliance's factions, their units and buildings, made from your own copy of the game.");
/// What a unit takes on the disk once made, about (its paint the most, as large as Forged Alliance
/// paints it: since converter version 10; its sounds, since 17, a tenth of it; its weapons' effects
/// and projectiles' models since 18; the drones' own units since 20): about 796 MB for the game's
/// 364.
const UNIT_BYTES: u64 = 2_186_000;

/// The mod's name as the interface shows it, in the language in use.
pub fn mod_name() -> &'static str {
    t(MOD_NAME)
}

/// The factions' mod the launcher uses, as `mods` lists it.
pub fn mod_copy(mods: &crate::modlist::ModList) -> Option<Installed> {
    Installed::listed(mods, MOD_ID, MOD_NAME)
}

/// Where the factions' mod is now (looked for afresh), and where an import installs it.
pub fn mod_place(roots: &Roots) -> Place {
    crate::supcom::place(roots, MOD_ID, MOD_NAME)
}

/// A faction of the copy of the game in use: its playable units by kind, and whether the mod
/// installed holds it.
#[derive(Clone, Debug)]
pub struct Entry {
    pub faction: Faction,
    pub buildings: usize,
    pub land: usize,
    pub air: usize,
    pub naval: usize,
    pub state: State,
}

impl Entry {
    pub fn units(&self) -> usize {
        self.buildings + self.land + self.air + self.naval
    }

    /// Its side's name, as the game's sides are named (UEF, CYBRAN, AEON, SERAPHIM): what it is
    /// chosen by.
    pub fn side(&self) -> &'static str {
        made_units::side(&self.faction)
    }
}

/// What a look for the game's units found.
#[derive(Clone, Default)]
pub struct Found {
    /// The copy of the game in use: the player's choice, else the first found.
    pub install: Option<Install>,
    /// Every copy of the game found (GOG's, Steam's...), in the order looked for.
    pub copies: Vec<Install>,
    /// The folder the player chose before, when it no longer holds the game.
    pub picked_gone: Option<PathBuf>,
    pub factions: Vec<Entry>,
    /// The mod is installed (an import made it).
    pub installed: bool,
    /// Why the game's units could not be listed.
    pub error: Option<String>,
}

/// The factions' commanders' build pictures (as the game will show them), by side.
pub type Pictures = BTreeMap<&'static str, egui::ColorImage>;

/// What an import came to.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    /// The factions installed, by side.
    pub factions: Vec<String>,
    pub units: usize,
    /// Units that could not be made (by their names in the game), and why.
    pub skipped: Vec<(String, String)>,
    pub stopped: bool,
    /// Why nothing was installed, when it failed.
    pub error: Option<String>,
}

impl Outcome {
    pub fn went(&self) -> Went {
        if self.error.is_some() {
            Went::Badly
        } else if self.stopped || self.factions.is_empty() {
            Went::Nothing
        } else if self.skipped.is_empty() {
            Went::Well
        } else {
            Went::Partly
        }
    }

    /// The factions installed, as a list in the language in use ("UEF, CYBRAN and AEON").
    fn names(&self) -> String {
        crate::pages::listed(&self.factions)
    }

    /// What it came to, as the dialog says it.
    pub fn summary(&self) -> String {
        let (names, units, failed) = (self.names(), &self.units, &self.skipped.len());
        let one = self.factions.len() == 1;
        match (&self.error, self.went()) {
            (Some(why), _) => tf("The factions could not be imported: {why}", &[("why", why)]),
            (None, Went::Nothing) => t("Stopped: nothing was installed, and the factions are as they were.").to_string(),
            (None, Went::Well) if one => tf("Imported the {faction} faction: {units} units. Its mod is on: choose {faction} as a player's side when you set up a skirmish.", &[("faction", &names), ("units", units)]),
            (None, Went::Well) => tf("Imported the {factions} factions: {units} units in all. Their mod is on: choose one as a player's side when you set up a skirmish.", &[("factions", &names), ("units", units)]),
            (None, _) if one => tf("Imported the {faction} faction, but {failed} of its units could not be made: the {units} made are a side to play in skirmish.", &[("faction", &names), ("failed", failed), ("units", units)]),
            (None, _) => tf("Imported the {factions} factions, but {failed} of their units could not be made: the {units} made are in skirmish, a side each.", &[("factions", &names), ("failed", failed), ("units", units)]),
        }
    }

    /// What a notice says of it with the dialog closed (nothing for a stop).
    pub fn notice(&self) -> Option<String> {
        let (names, failed) = (self.names(), &self.skipped.len());
        let one = self.factions.len() == 1;
        Some(match (&self.error, self.went()) {
            (Some(why), _) => tf("The Supreme Commander factions could not be imported: {why}", &[("why", why)]),
            (None, Went::Nothing) => return None,
            (None, Went::Well) if one => tf("Imported Supreme Commander's {faction} faction: a side to play in skirmish.", &[("faction", &names)]),
            (None, Went::Well) => tf("Imported Supreme Commander's {factions} factions: each a side to play in skirmish.", &[("factions", &names)]),
            (None, _) if one => tf("Imported Supreme Commander's {faction} faction, but {failed} of its units could not be made: SUPREME COMMANDER… on the Mods page lists them.", &[("faction", &names), ("failed", failed)]),
            (None, _) => tf("Imported Supreme Commander's {factions} factions, but {failed} of their units could not be made: SUPREME COMMANDER… on the Mods page lists them.", &[("factions", &names), ("failed", failed)]),
        })
    }
}

#[derive(Default)]
pub struct Factions {
    pub(crate) look: Look<Found, &'static str>,
    /// The factions chosen, by side. None until the game's units are first listed, when those the
    /// mod holds are chosen, or every faction where it holds none ([`first_choice`]); after that
    /// what the player chose (until the mod is removed: chosen afresh then).
    pub chosen: Option<BTreeSet<String>>,
    job: Option<Job<Outcome>>,
    pub progress: Arc<Mutex<Progress>>,
    pub outcome: Option<Outcome>,
    /// The last import ended while the dialog showed the maps' tab: said there until this tab is
    /// shown.
    pub unseen: bool,
    /// Factions to choose and import as soon as the game's units are listed (sides, or `all`): a
    /// capture's (`--show supcom-factions:uef,cybran`).
    pub auto_start: Option<Vec<String>>,
}

impl Factions {
    pub fn scanning(&self) -> bool {
        self.look.scanning()
    }

    pub fn importing(&self) -> bool {
        self.job.is_some()
    }

    /// STOP was pressed: the import ends at the next unit, nothing installed.
    pub fn stopping(&self) -> bool {
        self.job.as_ref().is_some_and(Job::stopping)
    }

    /// What the last look found.
    pub fn found(&self) -> Option<Arc<Found>> {
        self.look.found.clone()
    }

    /// Looks for the game and its units (in the player's chosen folder `picked`, else wherever a
    /// copy is), and for the mod where the launcher finds mods (`roots`).
    pub fn rescan(&mut self, picked: Option<PathBuf>, roots: &Roots) {
        let folders = roots.folders.clone();
        self.look.start("supcom factions scan", move || {
            let held = mod_place(&Roots { folders, mods_dir: PathBuf::new() }).copy.filter(|copy| copy.ours && !copy.archive).map(|copy| copy.path);
            find(crate::supcom::copies_found(), picked.as_deref(), held.as_deref())
        });
    }

    /// Whether the faction of side `side` is chosen.
    pub fn is_chosen(&self, side: &str) -> bool {
        self.chosen.as_ref().is_some_and(|chosen| chosen.contains(side))
    }

    /// Chooses the faction of side `side` (`on`), or not.
    pub fn choose(&mut self, side: &str, on: bool) {
        let chosen = self.chosen.get_or_insert_with(BTreeSet::new);
        if on {
            chosen.insert(side.to_string());
        } else {
            chosen.remove(side);
        }
    }

    /// The factions chosen, as listed.
    pub fn chosen_entries(&self) -> Vec<Entry> {
        self.look.found.iter().flat_map(|found| found.factions.iter().filter(|e| self.is_chosen(e.side())).cloned()).collect()
    }

    /// The commander's picture of the faction of side `side` as a texture, made from its pixels
    /// the first time it is shown.
    pub fn texture(&mut self, ctx: &egui::Context, side: &'static str) -> Option<egui::TextureHandle> {
        self.look.texture(ctx, &side, || format!("supcom-faction:{side}"), egui::TextureOptions::NEAREST)
    }

    /// How many pictures are held as pixels, and as textures.
    #[cfg(test)]
    pub fn pictures_held(&self) -> (usize, usize) {
        self.look.held()
    }

    /// Starts making the factions chosen into the mod, the launcher's settings in `store_dir`, the
    /// mod where `place` says; the sides' emblems in the palette of the original game at `ta_root`
    /// (none without it).
    pub fn start(&mut self, ta_root: Option<PathBuf>, store_dir: PathBuf, place: Place) {
        let Some(found) = self.look.found.clone() else { return };
        let Some(game) = found.install.clone() else { return };
        if self.importing() {
            return;
        }
        let chosen: Vec<Entry> = found.factions.iter().filter(|e| self.is_chosen(e.side())).cloned().collect();
        if chosen.is_empty() {
            return;
        }
        let needed = size_estimate(&chosen);
        let sides: BTreeSet<String> = chosen.iter().map(|e| e.side().to_string()).collect();
        *self.progress.lock().unwrap() = Progress::default();
        let progress = self.progress.clone();
        self.outcome = None;
        let job = Job::spawn("supcom factions import", self.progress.clone(), move |ending| {
            import(&game, &sides, ta_root.as_deref(), &store_dir, &place, needed, &progress, ending).unwrap_or_else(|e| Outcome { error: Some(error_text(&e)), ..Default::default() })
        });
        match job {
            Ok(job) => self.job = Some(job),
            Err(e) => self.outcome = Some(Outcome { error: Some(e.to_string()), ..Default::default() }),
        }
    }

    /// The factions of `auto_start`, chosen, once the game's units are listed: true when they are
    /// to be imported now.
    pub fn take_auto_start(&mut self) -> bool {
        if self.scanning() || self.importing() {
            return false;
        }
        let (Some(names), Some(found)) = (&self.auto_start, &self.look.found) else { return false };
        let all = names.iter().any(|n| n.eq_ignore_ascii_case("all"));
        let chosen: BTreeSet<String> = found.factions.iter().map(|e| e.side()).filter(|s| all || names.iter().any(|n| n.eq_ignore_ascii_case(s))).map(str::to_string).collect();
        let any = !chosen.is_empty();
        self.chosen = Some(chosen);
        self.auto_start = None;
        any
    }

    /// Whether a capture may take its picture: the units looked for, a capture's factions
    /// imported (or, with `TA_SUPCOM_SHOT_IMPORTING=1`, while they are).
    pub fn capture_ready(&self) -> bool {
        let during = std::env::var_os("TA_SUPCOM_SHOT_IMPORTING").is_some_and(|v| v == "1");
        !self.scanning() && self.auto_start.is_none() && (during || !self.importing())
    }

    /// Stops the import at the next unit: nothing is installed. (Once installing, it finishes:
    /// STOP is greyed then.)
    pub fn stop(&self) {
        if let Some(job) = &self.job {
            job.stop();
        }
    }

    /// Takes in what the workers found and made; `Some` when an import has ended (the mod
    /// installed, when it holds factions).
    pub fn poll(&mut self, ctx: &egui::Context, open: bool) -> Option<Outcome> {
        if let Some((found, before)) = self.look.poll(ctx, open) {
            let same_copy = before.as_ref().is_some_and(|b| b.install == found.install);
            // (The mod removed since: the factions are chosen afresh, as at first.)
            let removed = before.as_ref().is_some_and(|b| b.installed && !found.installed);
            match &mut self.chosen {
                Some(chosen) if same_copy && !removed => chosen.retain(|c| found.factions.iter().any(|e| e.side() == c)),
                _ => self.chosen = Some(first_choice(&found)),
            }
        }
        if self.job.as_ref().is_some_and(Job::finished) {
            let job = self.job.take().unwrap();
            let outcome = job.join(|| Outcome { error: Some(t("The import stopped unexpectedly.").to_string()), ..Default::default() });
            self.outcome = Some(outcome.clone());
            return Some(outcome);
        }
        if self.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        None
    }

    /// Stops an import under way at shutdown and waits for it (a unit's making, or the install
    /// once begun): its outcome.
    pub fn finish_on_exit(&mut self) -> Option<Outcome> {
        self.stop();
        let job = self.job.take()?;
        Some(job.join(Outcome::default))
    }

    /// An import under way that ends, stopped, once asked to stop: for tests.
    #[cfg(test)]
    pub fn pretend_importing(&mut self) {
        self.progress.lock().unwrap().phase = Phase::Making;
        let job = Job::spawn("pretend factions import", self.progress.clone(), |ending| {
            while !ending.stop.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Outcome { stopped: true, ..Default::default() }
        });
        self.job = Some(job.unwrap());
    }

    /// An import under way that is installing what it made, and ends installed once `done` is
    /// set, whatever STOP said meanwhile: for tests.
    #[cfg(test)]
    pub fn pretend_installing(&mut self, done: Arc<std::sync::atomic::AtomicBool>) {
        self.progress.lock().unwrap().phase = Phase::Installing;
        let job = Job::spawn("pretend factions install", self.progress.clone(), move |ending| {
            while !done.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            let stopped = ending.stop.load(Ordering::Relaxed);
            Outcome { factions: vec!["UEF".into()], units: 92, stopped, ..Default::default() }
        });
        self.job = Some(job.unwrap());
    }

    /// An import that comes to `outcome` at once: for tests.
    #[cfg(test)]
    pub fn pretend_imported(&mut self, outcome: Outcome) {
        self.job = Some(Job::spawn("pretend factions import", self.progress.clone(), move |_| outcome).unwrap());
    }

    /// A look that found `found`, taken in at the next [`Factions::poll`]: for tests.
    #[cfg(test)]
    pub fn pretend_found(&mut self, found: Found, pictures: Pictures) {
        self.look.pretend((found, pictures));
    }
}

/// The factions chosen for the player when the game's units are first listed: those the mod
/// holds (made again, it keeps them), or every one where it holds none.
fn first_choice(found: &Found) -> BTreeSet<String> {
    let held: BTreeSet<String> = found.factions.iter().filter(|e| e.state != State::New).map(|e| e.side().to_string()).collect();
    if held.is_empty() { found.factions.iter().map(|e| e.side().to_string()).collect() } else { held }
}

/// What the factions chosen take on the disk once made, about (bytes).
pub fn size_estimate(entries: &[Entry]) -> u64 {
    entries.iter().map(|e| e.units() as u64 * UNIT_BYTES).sum()
}

/// About how long a unit takes to make and install on a fast computer (milliseconds): the four
/// factions' 364 in about half a minute, their sounds (a few seconds) among it.
const UNIT_MILLIS: u64 = 80;

/// About how long the factions chosen take to make on a fast computer, in seconds (the wizard's
/// estimate; the tab says it in words).
pub fn time_estimate(entries: &[Entry]) -> u64 {
    (entries.iter().map(|e| e.units() as u64).sum::<u64>() * UNIT_MILLIS).div_ceil(1000)
}

/// How a unit is counted: a building, or by how it moves.
fn kind_of(u: &UnitBlueprint) -> usize {
    match u.motion {
        _ if u.is("STRUCTURE") => 0,
        Motion::None => 0,
        Motion::Air => 2,
        Motion::Water | Motion::Sub => 3,
        Motion::Land | Motion::Amphibious | Motion::Hover => 1,
    }
}

/// The converter's version that made the mod in `dir`, and the sides it holds.
fn installed_state(dir: &Path) -> (u32, BTreeSet<String>) {
    let version = ta_vfs::mods::read_mod(dir).ok().flatten().and_then(|m| m.manifest.version.trim().parse().ok()).unwrap_or(0);
    (version, installed_sides(dir))
}

/// The sides the mod in `dir` holds, as its side add-on names them (`UEF`, `CYBRAN`...).
pub(crate) fn installed_sides(dir: &Path) -> BTreeSet<String> {
    std::fs::read(dir.join("gamedata").join("sidedata_supcom.tdf")).map(|bytes| {
        let doc = ta_formats::tdf::parse_bytes(&bytes);
        doc.sections.iter().filter(|s| s.name.to_ascii_lowercase().starts_with("side")).filter_map(|s| s.get("name")).map(|n| n.trim().to_ascii_uppercase()).collect()
    }).unwrap_or_default()
}

/// The game in use (of the `copies` found, or the folder `picked` while it still holds the game),
/// its playable units counted by faction (with what the mod installed in `held` holds), and each
/// faction's commander's build picture.
fn find(copies: Vec<Install>, picked: Option<&Path>, held: Option<&Path>) -> (Found, Pictures) {
    let picked_gone = picked.filter(|p| !install::validate(p)).map(Path::to_path_buf);
    let install = crate::supcom::in_use(&copies, picked.filter(|_| picked_gone.is_none()));
    let mut found = Found { install: install.clone(), copies, picked_gone, installed: held.is_some(), ..Default::default() };
    let mut pictures = Pictures::new();
    let Some(game) = install else { return (found, pictures) };
    let (version, held) = held.map(installed_state).unwrap_or_default();
    let units = match Files::own(&game).and_then(|files| Ok((unit::catalogue(&files)?, files))) {
        Ok(units) => units,
        Err(e) => {
            found.error = Some(tf("The game's units could not be read: {why}", &[("why", &e)]));
            return (found, pictures);
        }
    };
    let (units, files) = units;
    for f in Faction::PLAYABLE {
        let theirs: Vec<&UnitBlueprint> = units.iter().filter(|u| u.playable() && u.faction == f).collect();
        if theirs.is_empty() {
            continue;
        }
        let mut counts = [0usize; 4];
        for u in &theirs {
            counts[kind_of(u)] += 1;
        }
        let side = made_units::side(&f);
        let state = if !held.contains(side) { State::New } else if version >= make::VERSION { State::Imported } else { State::Outdated };
        if let Some(commander) = faction::commander(&theirs)
            && let Some(picture) = picture_of(&files, commander)
        {
            pictures.insert(side, picture);
        }
        found.factions.push(Entry { faction: f, buildings: counts[0], land: counts[1], air: counts[2], naval: counts[3], state });
    }
    if found.factions.is_empty() {
        found.error = Some(t("No units of the four factions were found in this copy of the game.").to_string());
    }
    (found, pictures)
}

/// A unit's build picture as the game will show it (the converter's own).
fn picture_of(files: &Files, u: &UnitBlueprint) -> Option<egui::ColorImage> {
    let icon = files.read(&pictures::icon_path(u)).ok()??;
    let frame = pictures::frame_path(u).and_then(|p| files.read(&p).ok().flatten());
    let pcx = ta_formats::pcx::PcxImage::parse(&pictures::picture(&icon, frame.as_deref()).ok()?).ok()?;
    Some(egui::ColorImage::from_rgba_unmultiplied([pcx.width as usize, pcx.height as usize], &pcx.to_rgba()))
}

/// What a unit is called while it is made: its side, and its name in the game.
fn called(u: &UnitBlueprint) -> String {
    let name = [&u.name, &u.description].into_iter().find(|n| !n.trim().is_empty()).map_or(u.id.as_str(), |n| n.as_str());
    format!("{} {}", made_units::side(&u.faction), unit::strip_loc(name))
}

/// TA's own palette, from the original game at `ta_root` (the sides' emblems are made in it, as
/// TA's own are): none where it does not read.
fn ta_palette(ta_root: &Path) -> Option<Palette> {
    let mut vfs = ta_vfs::Vfs::new();
    let read = vfs.mount_game_dir(ta_root, "base").ok().and_then(|_| Palette::parse(&vfs.read_opt("palettes/palette.pal")?).ok());
    if read.is_none() {
        log::warn!("{}: no palette read, the factions' emblems are left out", ta_root.display());
    }
    read
}

/// Makes the factions of `sides` of the copy `game` into the mod (their emblems in the palette of
/// TA at `ta_root`, where it is found) and installs it where `place` says, in place of the one
/// there: when there is room on the disk for `needed` bytes. Nothing is installed when stopped
/// before every unit is made.
#[allow(clippy::too_many_arguments)]
fn import(game: &Install, sides: &BTreeSet<String>, ta_root: Option<&Path>, store_dir: &Path, place: &Place, needed: u64, progress: &Arc<Mutex<Progress>>, ending: &Ending) -> Result<Outcome> {
    let work_root = store_dir.join("cache").join("faction-imports");
    crate::supcom::enough_room(&[&work_root, &place.into], needed)?;
    let palette = ta_root.and_then(ta_palette);
    let files = Files::own(game).context(t("Supreme Commander: Forged Alliance's units could not be read."))?;
    let units: Vec<UnitBlueprint> = unit::catalogue(&files)?.into_iter().filter(|u| u.playable() && sides.contains(made_units::side(&u.faction))).collect();
    if units.is_empty() {
        bail!("{}", t("None of the factions chosen has units in this copy of the game."));
    }
    let names: BTreeMap<String, String> = units.iter().map(|u| (u.id.clone(), called(u))).collect();
    let work = crate::maps::Workspace::new(&work_root)?;
    let payload = work.0.join("payload");
    let factions: Vec<String> = Faction::PLAYABLE.iter().map(made_units::side).filter(|s| sides.contains(*s)).map(str::to_string).collect();
    let into = place.into.clone();
    // (Forged Alliance's sounds from the copy's own banks, where it has them.)
    let sounds = ta_vfs::install::find_dir_ci(&game.root, "sounds");
    build(&payload, &names, factions, progress, ending, |write, watch| Ok(make::make(&files, &units, palette.as_ref(), sounds.as_deref(), env!("CARGO_PKG_VERSION"), write, watch)?), || install_made(&payload, &into).map(|_| ()))
}

/// The mod made into `payload` by `make` (given where each file goes, and told before each unit
/// how many are made and its id: false from it stops the making), unit by unit (`names`, by id,
/// what each is called), of `factions`; then installed (`install`) unless stopped before every
/// unit was made. A unit that could not be made is told by its name.
fn build(
    payload: &Path,
    names: &BTreeMap<String, String>,
    factions: Vec<String>,
    progress: &Mutex<Progress>,
    ending: &Ending,
    make: impl FnOnce(&mut dyn FnMut(&str, &[u8]) -> ta_supcom::Result<()>, &mut dyn FnMut(usize, &str) -> bool) -> Result<make::Made>,
    install: impl FnOnce() -> Result<()>,
) -> Result<Outcome> {
    {
        let mut p = progress.lock().unwrap();
        p.total = names.len();
        p.phase = Phase::Making;
    }
    let mut made = make(&mut |path, bytes| write_into(payload, path, bytes), &mut |done, id| {
        let mut p = progress.lock().unwrap();
        p.done = done;
        // (Once the units are made, their sounds.)
        p.current = if id == make::SOUNDS { t("the units' sounds").to_string() } else { names.get(id).cloned().unwrap_or_else(|| id.to_string()) };
        !ending.stop.load(Ordering::Relaxed)
    })?;
    for (unit, _) in &mut made.skipped {
        if let Some(name) = names.get(unit.as_str()) {
            *unit = name.clone();
        }
    }
    conclude(made, factions, names.len(), progress, ending, install)
}

/// What a making of `total` units came to: the mod made installed (by `install`), unless it was
/// stopped, before a unit or while the last was made (a stop asked for meanwhile: nothing is
/// installed either). Once installing, it finishes: STOP is greyed by then.
fn conclude(made: make::Made, factions: Vec<String>, total: usize, progress: &Mutex<Progress>, ending: &Ending, install: impl FnOnce() -> Result<()>) -> Result<Outcome> {
    if made.stopped {
        return Ok(Outcome { stopped: true, ..Default::default() });
    }
    if made.units == 0 {
        bail!("{}", t("None of the units could be made."));
    }
    {
        let mut p = progress.lock().unwrap();
        p.done = total;
        // (A stop asked for while the last unit was made is taken now, before the install.)
        if ending.stop.load(Ordering::Relaxed) {
            return Ok(Outcome { stopped: true, ..Default::default() });
        }
        p.phase = Phase::Installing;
    }
    install()?;
    Ok(Outcome { factions, units: made.units, skipped: made.skipped, ..Default::default() })
}

/// Writes a file the converter made into the mod's folder being made.
fn write_into(payload: &Path, path: &str, bytes: &[u8]) -> ta_supcom::Result<()> {
    let at = payload.join(path);
    if let Some(dir) = at.parent() {
        std::fs::create_dir_all(dir).map_err(|e| ta_supcom::Error::Format(format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(&at, bytes).map_err(|e| ta_supcom::Error::Format(format!("{}: {e}", at.display())))
}

/// Installs the mod made in `payload` (its own manifest, `make`'s) in `mods_dir`, in place of the
/// one installed: its folder.
fn install_made(payload: &Path, mods_dir: &Path) -> Result<PathBuf> {
    let manifest = ta_vfs::mods::read_mod(payload)?.context(t("The mod made has no manifest."))?.manifest;
    Ok(ta_vfs::package::install(&ta_vfs::package::plan(payload)?, &manifest, mods_dir)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ta-supcom-factions-{name}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The words the interface translates are the converter's own.
    #[test]
    fn the_mods_words_are_the_converters() {
        assert_eq!(MOD_NAME, make::MOD_NAME);
        assert_eq!(MOD_DESCRIPTION, make::DESCRIPTION);
    }

    /// A mod made is installed under its own manifest, in place of the one there: what the new
    /// one does not make is gone; the factions it holds and the converter that made it are read
    /// back from it.
    #[test]
    fn a_mod_made_replaces_the_one_installed() {
        let dir = temp("install");
        let mods = dir.join("mods");
        let payload = |name: &str, sides: &[&str]| {
            let p = dir.join(name);
            for (path, bytes) in [("mod.toml", make::manifest("1.0.7")), ("units/uel0201.fbi", "[UNITINFO]{UnitName=UEL0201;}".to_string())] {
                write_into(&p, path, bytes.as_bytes()).unwrap();
            }
            let add_on: String = sides.iter().enumerate().map(|(i, s)| format!("[SIDE{i}]{{name={s};commander={s}0001;}}")).collect();
            write_into(&p, "gamedata/sidedata_supcom.tdf", add_on.as_bytes()).unwrap();
            p
        };
        let first = payload("first", &["UEF", "CYBRAN"]);
        write_into(&first, "units/url0107.fbi", b"[UNITINFO]{UnitName=URL0107;}").unwrap();
        let installed = install_made(&first, &mods).unwrap();
        let roots = Roots { folders: vec![crate::modlist::ModFolder { label: ta_vfs::mods::USER_FOLDER_LABEL, path: mods.clone(), exists: true }], mods_dir: mods.clone() };
        assert_eq!(crate::supcom::place(&roots, MOD_ID, MOD_NAME).copy.map(|copy| (copy.path, copy.ours)), Some((installed.clone(), true)));
        let info = ta_vfs::mods::read_mod(&installed).unwrap().unwrap();
        assert_eq!((info.manifest.id.as_str(), info.manifest.version.as_str(), info.manifest.requires_reborn.as_str()), (MOD_ID, make::VERSION.to_string().as_str(), "1.0.7"));
        assert_eq!(installed_state(&installed), (make::VERSION, ["CYBRAN".to_string(), "UEF".to_string()].into()));
        // Made again of the UEF alone: the Cybran's unit is gone.
        let again = install_made(&payload("second", &["UEF"]), &mods).unwrap();
        assert_eq!(again, installed);
        assert!(installed.join("units/uel0201.fbi").exists() && !installed.join("units/url0107.fbi").exists());
        assert_eq!(installed_state(&installed).1, ["UEF".to_string()].into());
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn entry(faction: Faction, state: State) -> Entry {
        Entry { faction, buildings: 47, land: 20, air: 12, naval: 10, state }
    }

    /// At first the factions the mod holds are chosen (made again, it keeps them), or all four
    /// where it holds none; a look after keeps the player's choice, until the mod is removed.
    #[test]
    fn the_first_choice_is_what_the_mod_holds_or_every_faction() {
        let ctx = egui::Context::default();
        let found = |states: [State; 4]| Found {
            install: Some(Install { root: PathBuf::from("C:/Game"), store: ta_supcom::install::Store::Gog }),
            factions: Faction::PLAYABLE.into_iter().zip(states).map(|(f, s)| entry(f, s)).collect(),
            installed: states.iter().any(|s| *s != State::New),
            ..Default::default()
        };
        let mut factions = Factions::default();
        factions.pretend_found(found([State::New; 4]), Pictures::new());
        factions.poll(&ctx, true);
        assert_eq!(factions.chosen_entries().len(), 4);
        let mut held = Factions::default();
        held.pretend_found(found([State::Imported, State::New, State::Outdated, State::New]), Pictures::new());
        held.poll(&ctx, true);
        assert_eq!(held.chosen_entries().iter().map(Entry::side).collect::<Vec<_>>(), ["UEF", "AEON"]);
        held.choose("SERAPHIM", true);
        held.choose("UEF", false);
        held.pretend_found(found([State::Imported; 4]), Pictures::new());
        held.poll(&ctx, true);
        assert_eq!(held.chosen_entries().iter().map(Entry::side).collect::<Vec<_>>(), ["AEON", "SERAPHIM"], "the player's choice is kept");
        assert_eq!(size_estimate(&held.chosen_entries()), 2 * 89 * UNIT_BYTES);
        // The mod removed: every faction chosen afresh.
        held.pretend_found(found([State::New; 4]), Pictures::new());
        held.poll(&ctx, true);
        assert_eq!(held.chosen_entries().len(), 4);
        // A capture's factions, chosen once listed.
        held.auto_start = Some(vec!["cybran".into()]);
        assert!(held.take_auto_start());
        assert_eq!(held.chosen_entries().iter().map(Entry::side).collect::<Vec<_>>(), ["CYBRAN"]);
    }

    /// A stop asked for while the last unit was made installs nothing, as one before a unit does;
    /// without one the mod is installed, and told of.
    #[test]
    fn a_stop_while_the_last_unit_is_made_installs_nothing() {
        let made = || make::Made { units: 2, files: 9, ..Default::default() };
        let progress = Mutex::new(Progress::default());
        let installed = std::cell::Cell::new(false);
        let install = || {
            installed.set(true);
            Ok::<(), anyhow::Error>(())
        };
        let stopping = Ending::default();
        stopping.stop.store(true, Ordering::Relaxed);
        let outcome = conclude(made(), vec!["UEF".into()], 2, &progress, &stopping, install).unwrap();
        assert!(outcome.stopped && outcome.factions.is_empty() && !installed.get(), "{outcome:?}");
        let outcome = conclude(made(), vec!["UEF".into()], 2, &progress, &Ending::default(), install).unwrap();
        assert!(!outcome.stopped && outcome.factions == ["UEF"] && outcome.units == 2 && installed.get(), "{outcome:?}");
        assert_eq!(progress.lock().unwrap().phase, Phase::Installing);
        let outcome = conclude(make::Made { stopped: true, ..made() }, vec!["UEF".into()], 2, &progress, &Ending::default(), || panic!("installed")).unwrap();
        assert!(outcome.stopped, "stopped before a unit");
    }

    /// The making itself, by a stand-in for the converter: units that could not be made are told
    /// by their names; a file that cannot be written fails the import, nothing installed (the mod
    /// installed stays as it was).
    #[test]
    fn the_factions_import_names_its_units_and_installs_nothing_when_it_fails() {
        let dir = temp("build");
        let names: BTreeMap<String, String> = [("UEL0201".to_string(), "UEF MA12 Striker".to_string()), ("UEL0401".to_string(), "UEF Fatboy".to_string())].into();
        let progress = Mutex::new(Progress::default());
        let installed = std::cell::Cell::new(false);
        let install = || {
            installed.set(true);
            Ok::<(), anyhow::Error>(())
        };
        let make_two = |write: &mut dyn FnMut(&str, &[u8]) -> ta_supcom::Result<()>, watch: &mut dyn FnMut(usize, &str) -> bool| -> Result<make::Made> {
            write("mod.toml", b"id = \"supcom-factions\"\n")?;
            assert!(watch(0, "UEL0201") && watch(1, "UEL0401"));
            write("units/uel0201.fbi", b"[UNITINFO]{}")?;
            Ok(make::Made { units: 1, files: 2, skipped: vec![("UEL0401".into(), "no mesh".into())], ..Default::default() })
        };
        let outcome = build(&dir.join("payload"), &names, vec!["UEF".into()], &progress, &Ending::default(), make_two, install).unwrap();
        assert_eq!(outcome.skipped, [("UEF Fatboy".to_string(), "no mesh".to_string())], "told by its name");
        assert!(installed.get() && outcome.went() == Went::Partly);
        assert!(outcome.notice().unwrap().contains("but 1 of its units could not be made"), "{:?}", outcome.notice());
        // A file that cannot be written (its folder's place taken by a file): nothing installed.
        installed.set(false);
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, b"not a folder").unwrap();
        let make_fails = |write: &mut dyn FnMut(&str, &[u8]) -> ta_supcom::Result<()>, _: &mut dyn FnMut(usize, &str) -> bool| -> Result<make::Made> {
            write("units/uel0201.fbi", b"[UNITINFO]{}")?;
            Ok(make::Made { units: 1, ..Default::default() })
        };
        let failed = build(&blocked, &names, vec!["UEF".into()], &progress, &Ending::default(), make_fails, install);
        assert!(failed.is_err() && !installed.get());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Once the mod is being installed, STOP asks nothing (it is greyed then): the install ends,
    /// the mod installed.
    #[test]
    fn stop_while_installing_does_not_stop_the_install() {
        let ctx = egui::Context::default();
        let mut factions = Factions::default();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        factions.pretend_installing(done.clone());
        factions.stop();
        assert!(!factions.stopping(), "not asked: it cannot stop now");
        done.store(true, Ordering::Relaxed);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let outcome = loop {
            if let Some(outcome) = factions.poll(&ctx, true) {
                break outcome;
            }
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert!(!outcome.stopped && outcome.factions == ["UEF"], "{outcome:?}");
    }

    /// An import is told as it went: every unit made, some not (amber, and said so in its notice),
    /// failed (red), stopped (no notice); the factions named as a list, one as one.
    #[test]
    fn an_import_is_told_as_it_went() {
        let well = Outcome { factions: vec!["UEF".into(), "AEON".into()], units: 180, ..Default::default() };
        let partly = Outcome { skipped: vec![("UEF Fatboy".into(), "no mesh".into())], ..well.clone() };
        let broken = Outcome { error: Some("disk full".into()), ..Default::default() };
        let stopped = Outcome { stopped: true, ..Default::default() };
        let one = Outcome { factions: vec!["CYBRAN".into()], units: 91, ..Default::default() };
        assert_eq!([&well, &partly, &broken, &stopped].map(Outcome::went), [Went::Well, Went::Partly, Went::Badly, Went::Nothing]);
        assert_eq!(well.summary(), "Imported the UEF and AEON factions: 180 units in all. Their mod is on: choose one as a player's side when you set up a skirmish.");
        assert!(partly.summary().contains("but 1 of their units could not be made"), "{}", partly.summary());
        assert!(partly.notice().unwrap().contains("could not be made"), "a notice says some units failed: {:?}", partly.notice());
        assert_eq!(one.summary(), "Imported the CYBRAN faction: 91 units. Its mod is on: choose CYBRAN as a player's side when you set up a skirmish.");
        assert!(broken.notice().unwrap().ends_with("disk full"));
        assert!(stopped.notice().is_none());
    }
}
