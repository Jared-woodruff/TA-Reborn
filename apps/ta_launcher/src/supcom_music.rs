//! Supreme Commander: Forged Alliance's soundtrack, imported (`ta_supcom::soundtrack`): the tracks
//! of the copy of the game in use, listed by their parts (the menus', building, battle, the
//! briefings' and the films'), made on a worker into one mod (`supcom-music`): FLAC files,
//! lossless, which the game's SOUNDTRACK setting plays (Options, Sound) and its jukebox lists. As
//! with the maps and the factions (`crate::supcom`, whose look, job and places this tab shares),
//! nothing of the game's is shipped: each player's music is made from their own copy. The mod is
//! made whole each time: a stop, or a failure, leaves the one installed as it was.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use ta_config::Soundtrack;
use ta_supcom::install::{self, Install};
use ta_supcom::soundtrack::{self, Part, Track};
use ta_ui::i18n::{n, t, tf};

use crate::supcom::{Ending, Installed, Job, Look, Phase, Place, Progress, Roots, State, Went, error_text};

pub use soundtrack::{MOD_ID, VERSION};

/// The mod's name and words as its manifest keeps them (`soundtrack`'s), marked to be translated:
/// the interface shows them in the language in use ([`mod_name`]; the Mods page translates a
/// description). The name also tells the mod an import made from another of its id.
pub const MOD_NAME: &str = n("Supreme Commander music");
// (The description is shown through the Mods page's own translation of a manifest's words: kept
// here to be collected for translation, and held to `soundtrack`'s by a test.)
pub(crate) const MOD_DESCRIPTION: &str = n("Supreme Commander: Forged Alliance's soundtrack, made from your own copy of the game.");
/// What the music takes on the disk once made, about, as a share of its sound uncompressed (FLAC:
/// 226 MB of the game's 443 MB).
const FLAC_SHARE: f64 = 0.52;

/// The mod's name as the interface shows it, in the language in use.
pub fn mod_name() -> &'static str {
    t(MOD_NAME)
}

/// The music's mod the launcher uses, as `mods` lists it.
pub fn mod_copy(mods: &crate::modlist::ModList) -> Option<Installed> {
    Installed::listed(mods, MOD_ID, MOD_NAME)
}

/// Where the music's mod is now (looked for afresh), and where an import installs it.
pub fn mod_place(roots: &Roots) -> Place {
    crate::supcom::place(roots, MOD_ID, MOD_NAME)
}

/// What a look for the game's soundtrack found.
#[derive(Clone, Default)]
pub struct Found {
    /// The copy of the game in use: the player's choice, else the first found.
    pub install: Option<Install>,
    /// Every copy of the game found (GOG's, Steam's...), in the order looked for.
    pub copies: Vec<Install>,
    /// The folder the player chose before, when it no longer holds the game.
    pub picked_gone: Option<PathBuf>,
    pub tracks: Vec<Track>,
    /// The mod, as installed: none, made by this converter, or by an older one.
    pub state: State,
    /// Why the game's music could not be listed.
    pub error: Option<String>,
}

impl Found {
    /// How long the tracks play, all told (seconds).
    pub fn seconds(&self) -> f64 {
        self.tracks.iter().map(|t| t.seconds).sum()
    }
}

/// What an import came to.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub tracks: usize,
    /// Tracks that could not be made (by their names), and why.
    pub skipped: Vec<(String, String)>,
    pub stopped: bool,
    /// Why nothing was installed, when it failed.
    pub error: Option<String>,
    /// SOUNDTRACK was set to Forged Alliance's music once the mod was on ([`switch_on`]).
    pub switched: bool,
}

/// SOUNDTRACK as the music's mod is turned on or off (an import turns it on): Total Annihilation's
/// music alone (the default) becomes Forged Alliance's once on, so that it is heard without a
/// visit to the game's Options, and Forged Alliance's alone TA's again once off. A choice of the
/// player's own (both, by side) stays: without the mod, TA's plays all the same. Whether it
/// changed.
pub fn follow(soundtrack: &mut Soundtrack, on: bool) -> bool {
    let (from, to) = if on {
        (Soundtrack::TotalAnnihilation, Soundtrack::ForgedAlliance)
    } else {
        (Soundtrack::ForgedAlliance, Soundtrack::TotalAnnihilation)
    };
    let changed = *soundtrack == from;
    if changed {
        *soundtrack = to;
    }
    changed
}

/// SOUNDTRACK once an import's mod is on ([`follow`]).
pub fn switch_on(soundtrack: &mut Soundtrack) -> bool {
    follow(soundtrack, true)
}

impl Outcome {
    pub fn went(&self) -> Went {
        if self.error.is_some() {
            Went::Badly
        } else if self.stopped || self.tracks == 0 {
            Went::Nothing
        } else if self.skipped.is_empty() {
            Went::Well
        } else {
            Went::Partly
        }
    }

    /// What it came to, as the dialog says it.
    pub fn summary(&self) -> String {
        match (&self.error, self.went()) {
            (Some(why), _) => tf("The music could not be imported: {why}", &[("why", why)]),
            (None, Went::Nothing) => t("Stopped: nothing was installed, and the music is as it was.").to_string(),
            (None, Went::Well) if self.switched => tf("Imported the soundtrack: {n} tracks. Its mod is on, and SOUNDTRACK is now FORGED ALLIANCE: change that in Settings, Sound.", &[("n", &self.tracks)]),
            (None, Went::Well) => tf("Imported the soundtrack: {n} tracks. Its mod is on: choose SOUNDTRACK in Settings, Sound.", &[("n", &self.tracks)]),
            (None, _) if self.switched => tf("Imported {n} tracks of the soundtrack, but {failed} could not be made. Its mod is on, and SOUNDTRACK is now FORGED ALLIANCE: change that in Settings, Sound.", &[("n", &self.tracks), ("failed", &self.skipped.len())]),
            (None, _) => tf("Imported {n} tracks of the soundtrack, but {failed} could not be made. Its mod is on: choose SOUNDTRACK in Settings, Sound.", &[("n", &self.tracks), ("failed", &self.skipped.len())]),
        }
    }

    /// What a notice says of it with the dialog closed (nothing for a stop).
    pub fn notice(&self) -> Option<String> {
        Some(match (&self.error, self.went()) {
            (Some(why), _) => tf("Supreme Commander's music could not be imported: {why}", &[("why", why)]),
            (None, Went::Nothing) => return None,
            (None, Went::Well) if self.switched => t("Imported Supreme Commander's soundtrack: SOUNDTRACK is now FORGED ALLIANCE (Settings, Sound).").to_string(),
            (None, Went::Well) => t("Imported Supreme Commander's soundtrack: choose SOUNDTRACK in Settings, Sound.").to_string(),
            (None, _) => tf("Imported Supreme Commander's soundtrack, but {failed} of its tracks could not be made: SUPREME COMMANDER… on the Mods page lists them.", &[("failed", &self.skipped.len())]),
        })
    }
}

/// A track's name as the interface shows it, by its part (as the game's jukebox names it).
pub fn track_name(track: &Track) -> String {
    match track.part {
        Part::Menu => t("Menu theme").to_string(),
        Part::Building => tf("Building theme {n}", &[("n", &track.number)]),
        Part::Battle => tf("Battle theme {n}", &[("n", &track.number)]),
        Part::Briefing => tf("Briefing {n}", &[("n", &track.number)]),
        Part::Film => match track.stem.as_str() {
            "film_intro" => t("Intro").to_string(),
            "film_outro" => t("Outro").to_string(),
            "film_epilogue" => t("Epilogue").to_string(),
            "film_timeline" => t("Timeline").to_string(),
            "film_credits" => t("Credits").to_string(),
            other => other.replace('_', " "),
        },
    }
}

/// A part's heading in the list.
pub fn part_label(part: Part) -> &'static str {
    match part {
        Part::Menu => n("THE MENUS"),
        Part::Building => n("BUILDING"),
        Part::Battle => n("BATTLE"),
        Part::Briefing => n("BRIEFINGS"),
        Part::Film => n("FILMS"),
    }
}

/// What the music takes on the disk once made, about (bytes).
pub fn size_estimate(tracks: &[Track]) -> u64 {
    (tracks.iter().map(|t| t.bytes).sum::<u64>() as f64 * FLAC_SHARE) as u64
}

#[derive(Default)]
pub struct Music {
    pub(crate) look: Look<Found, u8>,
    job: Option<Job<Outcome>>,
    pub progress: Arc<Mutex<Progress>>,
    pub outcome: Option<Outcome>,
    /// The last import ended while the dialog showed another tab: said there until this one is
    /// shown.
    pub unseen: bool,
    /// The music to be imported as soon as it is listed: a capture's (`--show supcom-music:all`).
    pub auto_start: bool,
}

impl Music {
    pub fn scanning(&self) -> bool {
        self.look.scanning()
    }

    pub fn importing(&self) -> bool {
        self.job.is_some()
    }

    /// STOP was pressed: the import ends at the next track, nothing installed.
    pub fn stopping(&self) -> bool {
        self.job.as_ref().is_some_and(Job::stopping)
    }

    /// What the last look found.
    pub fn found(&self) -> Option<Arc<Found>> {
        self.look.found.clone()
    }

    /// Looks for the game and its music (in the player's chosen folder `picked`, else wherever a
    /// copy is), and for the mod where the launcher finds mods (`roots`).
    pub fn rescan(&mut self, picked: Option<PathBuf>, roots: &Roots) {
        let folders = roots.folders.clone();
        self.look.start("supcom music scan", move || {
            let held = mod_place(&Roots { folders, mods_dir: PathBuf::new() }).copy.filter(|copy| copy.ours && !copy.archive).map(|copy| copy.path);
            (find(crate::supcom::copies_found(), picked.as_deref(), held.as_deref()), BTreeMap::new())
        });
    }

    /// Starts making the music into the mod, the launcher's settings in `store_dir`, the mod where
    /// `place` says.
    pub fn start(&mut self, store_dir: PathBuf, place: Place) {
        let Some(found) = self.look.found.clone() else { return };
        let Some(game) = found.install.clone() else { return };
        if self.importing() || found.tracks.is_empty() {
            return;
        }
        let needed = size_estimate(&found.tracks);
        *self.progress.lock().unwrap() = Progress::default();
        let progress = self.progress.clone();
        self.outcome = None;
        let job = Job::spawn("supcom music import", self.progress.clone(), move |ending| {
            import(&game, &store_dir, &place, needed, &progress, ending).unwrap_or_else(|e| Outcome { error: Some(error_text(&e)), ..Default::default() })
        });
        match job {
            Ok(job) => self.job = Some(job),
            Err(e) => self.outcome = Some(Outcome { error: Some(e.to_string()), ..Default::default() }),
        }
    }

    /// The music of `auto_start`, once listed: true when it is to be imported now.
    pub fn take_auto_start(&mut self) -> bool {
        if self.scanning() || self.importing() || !self.auto_start {
            return false;
        }
        let Some(found) = &self.look.found else { return false };
        self.auto_start = false;
        !found.tracks.is_empty()
    }

    /// Whether a capture may take its picture: the music looked for, a capture's imported (or,
    /// with `TA_SUPCOM_SHOT_IMPORTING=1`, while it is).
    pub fn capture_ready(&self) -> bool {
        let during = std::env::var_os("TA_SUPCOM_SHOT_IMPORTING").is_some_and(|v| v == "1");
        !self.scanning() && !self.auto_start && (during || !self.importing())
    }

    /// Stops the import at the next track: nothing is installed.
    pub fn stop(&self) {
        if let Some(job) = &self.job {
            job.stop();
        }
    }

    /// Takes in what the workers found and made; `Some` when an import has ended (the mod
    /// installed, when it holds tracks).
    pub fn poll(&mut self, ctx: &egui::Context, open: bool) -> Option<Outcome> {
        self.look.poll(ctx, open);
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

    /// Once the import's mod is on: SOUNDTRACK set to play it ([`switch_on`]), and the outcome the
    /// dialog tells says so. Whether it was changed.
    pub fn switch_on(&mut self, soundtrack: &mut Soundtrack) -> bool {
        let switched = switch_on(soundtrack);
        if let Some(outcome) = self.outcome.as_mut() {
            outcome.switched = switched;
        }
        switched
    }

    /// Stops an import under way at shutdown and waits for it: its outcome.
    pub fn finish_on_exit(&mut self) -> Option<Outcome> {
        self.stop();
        let job = self.job.take()?;
        Some(job.join(Outcome::default))
    }

    /// An import under way that ends, stopped, once asked to stop: for tests.
    #[cfg(test)]
    pub fn pretend_importing(&mut self) {
        self.progress.lock().unwrap().phase = Phase::Making;
        let job = Job::spawn("pretend music import", self.progress.clone(), |ending| {
            while !ending.stop.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Outcome { stopped: true, ..Default::default() }
        });
        self.job = Some(job.unwrap());
    }

    /// An import that comes to `outcome` at once: for tests.
    #[cfg(test)]
    pub fn pretend_imported(&mut self, outcome: Outcome) {
        self.job = Some(Job::spawn("pretend music import", self.progress.clone(), move |_| outcome).unwrap());
    }

    /// A look that found `found`, taken in at the next poll: for tests.
    #[cfg(test)]
    pub fn pretend_found(&mut self, found: Found) {
        self.look.pretend((found, BTreeMap::new()));
    }
}

/// The converter's version that made the mod in `dir`.
fn installed_version(dir: &Path) -> u32 {
    ta_vfs::mods::read_mod(dir).ok().flatten().and_then(|m| m.manifest.version.trim().parse().ok()).unwrap_or(0)
}

/// The game in use (of the `copies` found, or the folder `picked` while it still holds the game),
/// its soundtrack's tracks, and the mod installed in `held`.
fn find(copies: Vec<Install>, picked: Option<&Path>, held: Option<&Path>) -> Found {
    let picked_gone = picked.filter(|p| !install::validate(p)).map(Path::to_path_buf);
    let install = crate::supcom::in_use(&copies, picked.filter(|_| picked_gone.is_none()));
    let state = match held.map(installed_version) {
        None => State::New,
        Some(v) if v >= VERSION => State::Imported,
        Some(_) => State::Outdated,
    };
    let mut found = Found { install: install.clone(), copies, picked_gone, state, ..Default::default() };
    let Some(game) = install else { return found };
    let sounds = ta_vfs::install::find_dir_ci(&game.root, "sounds");
    match sounds.ok_or_else(|| ta_supcom::Error::Missing("sounds".into())).and_then(|dir| soundtrack::tracks(&dir)) {
        Ok(tracks) => found.tracks = tracks,
        Err(e) => found.error = Some(tf("The game's music could not be read: {why}", &[("why", &e)])),
    }
    found
}

/// Makes the soundtrack of the copy `game` into the mod and installs it where `place` says, in
/// place of the one there: when there is room on the disk for `needed` bytes. Nothing is
/// installed when stopped before every track is made.
fn import(game: &Install, store_dir: &Path, place: &Place, needed: u64, progress: &Arc<Mutex<Progress>>, ending: &Ending) -> Result<Outcome> {
    let work_root = store_dir.join("cache").join("music-imports");
    crate::supcom::enough_room(&[&work_root, &place.into], needed)?;
    let sounds = ta_vfs::install::find_dir_ci(&game.root, "sounds").context(t("Supreme Commander: Forged Alliance's music could not be read."))?;
    let tracks = soundtrack::tracks(&sounds)?;
    let names: BTreeMap<String, String> = tracks.iter().map(|t| (t.stem.clone(), track_name(t))).collect();
    let work = crate::maps::Workspace::new(&work_root)?;
    let payload = work.0.join("payload");
    {
        let mut p = progress.lock().unwrap();
        p.total = tracks.len();
        p.phase = Phase::Making;
    }
    // (The game's own archives, for the banner's painting: without them, it is made without.)
    let files = ta_supcom::files::Files::own(game).ok();
    let mut made = soundtrack::make(&sounds, files.as_ref(), env!("CARGO_PKG_VERSION"), &mut |path, bytes| write_into(&payload, path, bytes), &mut |done, stem| {
        let mut p = progress.lock().unwrap();
        p.done = done;
        p.current = names.get(stem).cloned().unwrap_or_else(|| stem.to_string());
        !ending.stop.load(Ordering::Relaxed)
    })?;
    for (track, _) in &mut made.skipped {
        if let Some(name) = names.get(track.as_str()) {
            *track = name.clone();
        }
    }
    if made.stopped {
        return Ok(Outcome { stopped: true, ..Default::default() });
    }
    if made.tracks == 0 {
        bail!("{}", t("None of the tracks could be made."));
    }
    {
        let mut p = progress.lock().unwrap();
        p.done = tracks.len();
        // (A stop asked for while the last track was made is taken now, before the install.)
        if ending.stop.load(Ordering::Relaxed) {
            return Ok(Outcome { stopped: true, ..Default::default() });
        }
        p.phase = Phase::Installing;
    }
    let manifest = ta_vfs::mods::read_mod(&payload)?.context(t("The mod made has no manifest."))?.manifest;
    ta_vfs::package::install(&ta_vfs::package::plan(&payload)?, &manifest, &place.into)?;
    Ok(Outcome { tracks: made.tracks, skipped: made.skipped, ..Default::default() })
}

/// Writes a file the converter made into the mod's folder being made.
fn write_into(payload: &Path, path: &str, bytes: &[u8]) -> ta_supcom::Result<()> {
    let at = payload.join(path);
    if let Some(dir) = at.parent() {
        std::fs::create_dir_all(dir).map_err(|e| ta_supcom::Error::Format(format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(&at, bytes).map_err(|e| ta_supcom::Error::Format(format!("{}: {e}", at.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words the interface translates are the converter's own.
    #[test]
    fn the_mods_words_are_the_converters() {
        assert_eq!(MOD_NAME, soundtrack::MOD_NAME);
        assert_eq!(MOD_DESCRIPTION, soundtrack::DESCRIPTION);
    }

    /// An import is told as it went: every track made, some not, failed, stopped (no notice).
    #[test]
    fn an_import_is_told_as_it_went() {
        let well = Outcome { tracks: 21, ..Default::default() };
        let partly = Outcome { skipped: vec![("Intro".into(), "no data".into())], ..well.clone() };
        let broken = Outcome { error: Some("disk full".into()), ..Default::default() };
        let stopped = Outcome { stopped: true, ..Default::default() };
        assert_eq!([&well, &partly, &broken, &stopped].map(Outcome::went), [Went::Well, Went::Partly, Went::Badly, Went::Nothing]);
        assert!(well.summary().starts_with("Imported the soundtrack: 21 tracks."), "{}", well.summary());
        assert!(partly.notice().unwrap().contains("1 of its tracks could not be made"), "{:?}", partly.notice());
        assert!(broken.notice().unwrap().ends_with("disk full"));
        assert!(stopped.notice().is_none());
        // (SOUNDTRACK set for it, the words say so; left as the player chose it, where to choose.)
        let switched = Outcome { switched: true, ..well.clone() };
        assert!(switched.summary().contains("SOUNDTRACK is now FORGED ALLIANCE"), "{}", switched.summary());
        assert!(switched.notice().unwrap().contains("SOUNDTRACK is now FORGED ALLIANCE"));
        assert!(Outcome { switched: true, ..partly.clone() }.summary().contains("SOUNDTRACK is now FORGED ALLIANCE"));
        assert!(well.summary().contains("choose SOUNDTRACK") && well.notice().unwrap().contains("choose SOUNDTRACK"));
    }

    /// SOUNDTRACK goes from Total Annihilation's music alone to Forged Alliance's once the mod is
    /// on (an import's too), and back once it is off; a choice of the player's own (both, by side)
    /// stays either way.
    #[test]
    fn the_music_follows_the_mod() {
        let mut soundtrack = Soundtrack::TotalAnnihilation;
        assert!(switch_on(&mut soundtrack));
        assert_eq!(soundtrack, Soundtrack::ForgedAlliance);
        assert!(follow(&mut soundtrack, false));
        assert_eq!(soundtrack, Soundtrack::TotalAnnihilation);
        assert!(!follow(&mut soundtrack, false));
        for chosen in [Soundtrack::Both, Soundtrack::BySide] {
            for on in [true, false] {
                let mut soundtrack = chosen;
                assert!(!follow(&mut soundtrack, on));
                assert_eq!(soundtrack, chosen);
            }
        }
        let mut soundtrack = Soundtrack::ForgedAlliance;
        assert!(!switch_on(&mut soundtrack));
        assert_eq!(soundtrack, Soundtrack::ForgedAlliance);
    }

    /// The tracks' names by their parts, as the jukebox names them.
    #[test]
    fn tracks_go_by_their_parts() {
        let track = |part, number, stem: &str| Track { part, number, stem: stem.into(), bank: "Music.xwb", wave: 0, seconds: 1.0, bytes: 100 };
        assert_eq!(track_name(&track(Part::Menu, 1, "menu")), "Menu theme");
        assert_eq!(track_name(&track(Part::Building, 3, "building_3")), "Building theme 3");
        assert_eq!(track_name(&track(Part::Briefing, 6, "briefing_6")), "Briefing 6");
        assert_eq!(track_name(&track(Part::Film, 5, "film_credits")), "Credits");
        assert_eq!(size_estimate(&[track(Part::Menu, 1, "menu"), track(Part::Battle, 1, "battle")]), 104);
    }
}
