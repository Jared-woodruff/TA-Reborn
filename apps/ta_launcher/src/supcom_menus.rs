//! Supreme Commander: Forged Alliance's look for TA Reborn's menus, imported (`ta_supcom::menus`):
//! what the copy of the game in use has of it (its interface's colours, its menus' pictures, its
//! panels' border art, its interface sounds, its menu music), shown, and made on a worker into
//! one mod (`supcom-menus`), which the game's MENU THEME plays (Options, Interface). As with the
//! maps, the factions and the music (`crate::supcom`, whose look, job and places this tab shares),
//! nothing of the game's is shipped: each player's is made from their own copy. The mod is made
//! whole each time: a stop, or a failure, leaves the one installed as it was.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use ta_config::MenuTheme;
use ta_supcom::files::Files;
use ta_supcom::install::{self, Install};
use ta_supcom::menus;
use ta_ui::i18n::{n, t, tf};

use crate::supcom::{Ending, Installed, Job, Look, Phase, Place, Progress, Roots, State, Went, error_text};

pub use menus::{MOD_ID, VERSION};

/// The mod's name and words as its manifest keeps them (`menus`'), marked to be translated.
pub const MOD_NAME: &str = n("Supreme Commander menus");
pub(crate) const MOD_DESCRIPTION: &str = n("Supreme Commander: Forged Alliance's look for TA Reborn's menus: its colours, pictures, panel art, interface sounds and menu music, made from your own copy of the game.");
/// The pictures' size in the tab (points wide; four to three).
pub const PICTURE_WIDTH: f32 = 160.0;
/// What the mod takes on the disk, about (bytes): its pictures, art, sounds and music.
pub const SIZE: u64 = 20_000_000;

pub fn mod_name() -> &'static str {
    t(MOD_NAME)
}

pub fn mod_copy(mods: &crate::modlist::ModList) -> Option<Installed> {
    Installed::listed(mods, MOD_ID, MOD_NAME)
}

pub fn mod_place(roots: &Roots) -> Place {
    crate::supcom::place(roots, MOD_ID, MOD_NAME)
}

/// The pictures a look read, made small, by their names in the mod.
pub type Pictures = BTreeMap<&'static str, egui::ColorImage>;

/// What a look found.
#[derive(Clone, Default)]
pub struct Found {
    pub install: Option<Install>,
    pub copies: Vec<Install>,
    pub picked_gone: Option<PathBuf>,
    /// What the copy has of the look.
    pub contents: Option<menus::Contents>,
    /// The mod, as installed: none, made by this converter, or by an older one.
    pub state: State,
    pub error: Option<String>,
}

/// What an import came to.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub made: bool,
    /// What could not be made (a picture, a sound), and why.
    pub skipped: Vec<(String, String)>,
    pub stopped: bool,
    pub error: Option<String>,
}

/// MENU THEME as the menus' mod is turned on or off (an import turns it on): Forged Alliance's
/// look once on, so that it is seen without a visit to the game's Options; TA Reborn's own once
/// off. Whether it changed.
pub fn follow(theme: &mut MenuTheme, on: bool) -> bool {
    let wanted = if on { MenuTheme::ForgedAlliance } else { MenuTheme::Reborn };
    let changed = *theme != wanted;
    *theme = wanted;
    changed
}

/// MENU THEME once an import's mod is on ([`follow`]).
pub fn switch_on(theme: &mut MenuTheme) -> bool {
    follow(theme, true)
}

impl Outcome {
    pub fn went(&self) -> Went {
        if self.error.is_some() {
            Went::Badly
        } else if self.stopped || !self.made {
            Went::Nothing
        } else if self.skipped.is_empty() {
            Went::Well
        } else {
            Went::Partly
        }
    }

    pub fn summary(&self) -> String {
        match (&self.error, self.went()) {
            (Some(why), _) => tf("The menus' look could not be imported: {why}", &[("why", why)]),
            (None, Went::Nothing) => t("Stopped: nothing was installed, and the menus' look is as it was.").to_string(),
            (None, Went::Well) => t("Imported Forged Alliance's look for the menus. Its mod is on, and MENU THEME is now FORGED ALLIANCE: change that in Settings, Interface.").to_string(),
            (None, _) => tf("Imported Forged Alliance's look for the menus, but {failed} of its parts could not be made. Its mod is on, and MENU THEME is now FORGED ALLIANCE: change that in Settings, Interface.", &[("failed", &self.skipped.len())]),
        }
    }

    pub fn notice(&self) -> Option<String> {
        Some(match (&self.error, self.went()) {
            (Some(why), _) => tf("Supreme Commander's look for the menus could not be imported: {why}", &[("why", why)]),
            (None, Went::Nothing) => return None,
            (None, _) => t("Imported Supreme Commander's look for the menus: MENU THEME is now FORGED ALLIANCE (Settings, Interface).").to_string(),
        })
    }
}

/// The skin's colours as swatches show them, each with what it is for (marked for translation).
pub fn swatches(contents: &menus::Contents) -> Vec<(&'static str, egui::Color32)> {
    let named = [("text", n("Lettering")), ("dim", n("Labels")), ("accent", n("Accent")), ("panel", n("Panels")), ("line", n("Lines")), ("good", n("Buttons"))];
    let palette = contents.skin.palette();
    named.iter().filter_map(|(key, label)| palette.iter().find(|(k, _)| k == key).map(|(_, c)| (*label, egui::Color32::from_rgb(c[0], c[1], c[2])))).collect()
}

/// How many of the pictures the tab shows.
pub const PICTURES_SHOWN: usize = 6;

#[derive(Default)]
pub struct Menus {
    pub(crate) look: Look<Found, &'static str>,
    job: Option<Job<Outcome>>,
    pub progress: Arc<Mutex<Progress>>,
    pub outcome: Option<Outcome>,
    /// The last import ended while the dialog showed another tab: said there until this one is
    /// shown.
    pub unseen: bool,
    /// The look to be imported as soon as it is listed: a capture's (`--show supcom-menus:all`).
    pub auto_start: bool,
}

impl Menus {
    pub fn scanning(&self) -> bool {
        self.look.scanning()
    }

    pub fn importing(&self) -> bool {
        self.job.is_some()
    }

    /// STOP was pressed: the import ends at the next step, nothing installed.
    pub fn stopping(&self) -> bool {
        self.job.as_ref().is_some_and(Job::stopping)
    }

    /// What the last look found.
    pub fn found(&self) -> Option<Arc<Found>> {
        self.look.found.clone()
    }

    /// Looks for the game and what it has of the look (in the player's chosen folder `picked`,
    /// else wherever a copy is), its first pictures made small, and for the mod where the
    /// launcher finds mods (`roots`).
    pub fn rescan(&mut self, picked: Option<PathBuf>, roots: &Roots) {
        let folders = roots.folders.clone();
        self.look.start("supcom menus scan", move || {
            let held = mod_place(&Roots { folders, mods_dir: PathBuf::new() }).copy.filter(|copy| copy.ours && !copy.archive).map(|copy| copy.path);
            find(crate::supcom::copies_found(), picked.as_deref(), held.as_deref())
        });
    }

    /// A picture as a texture, made from its pixels the first time it is shown.
    pub fn texture(&mut self, ctx: &egui::Context, name: &'static str) -> Option<egui::TextureHandle> {
        self.look.texture(ctx, &name, || format!("supcom-menus:{name}"), egui::TextureOptions::LINEAR)
    }

    /// Starts making the look into the mod, the launcher's settings in `store_dir`, the mod where
    /// `place` says.
    pub fn start(&mut self, store_dir: PathBuf, place: Place) {
        let Some(found) = self.look.found.clone() else { return };
        let Some(game) = found.install.clone() else { return };
        if self.importing() || found.contents.is_none() {
            return;
        }
        *self.progress.lock().unwrap() = Progress::default();
        let progress = self.progress.clone();
        self.outcome = None;
        let job = Job::spawn("supcom menus import", self.progress.clone(), move |ending| {
            import(&game, &store_dir, &place, &progress, ending).unwrap_or_else(|e| Outcome { error: Some(error_text(&e)), ..Default::default() })
        });
        match job {
            Ok(job) => self.job = Some(job),
            Err(e) => self.outcome = Some(Outcome { error: Some(e.to_string()), ..Default::default() }),
        }
    }

    /// The look of `auto_start`, once listed: true when it is to be imported now.
    pub fn take_auto_start(&mut self) -> bool {
        if self.scanning() || self.importing() || !self.auto_start {
            return false;
        }
        let Some(found) = &self.look.found else { return false };
        self.auto_start = false;
        found.contents.is_some()
    }

    /// Whether a capture may take its picture: the look looked for, a capture's imported (or,
    /// with `TA_SUPCOM_SHOT_IMPORTING=1`, while it is).
    pub fn capture_ready(&self) -> bool {
        let during = std::env::var_os("TA_SUPCOM_SHOT_IMPORTING").is_some_and(|v| v == "1");
        !self.scanning() && !self.auto_start && (during || !self.importing())
    }

    /// Stops the import at the next step: nothing is installed.
    pub fn stop(&self) {
        if let Some(job) = &self.job {
            job.stop();
        }
    }

    /// Takes in what the workers found and made; `Some` when an import has ended (the mod
    /// installed, when it was made).
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
        let job = Job::spawn("pretend menus import", self.progress.clone(), |ending| {
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
        self.job = Some(Job::spawn("pretend menus import", self.progress.clone(), move |_| outcome).unwrap());
    }

    /// A look that found `found`, taken in at the next poll: for tests.
    #[cfg(test)]
    pub fn pretend_found(&mut self, found: Found, pictures: Pictures) {
        self.look.pretend((found, pictures));
    }
}

/// The converter's version that made the mod in `dir`.
fn installed_version(dir: &Path) -> u32 {
    ta_vfs::mods::read_mod(dir).ok().flatten().and_then(|m| m.manifest.version.trim().parse().ok()).unwrap_or(0)
}

/// How the mod installed in `dir` stands: made by this converter and whole (its theme there), or to
/// be made again (an older converter's, or one installed without its theme).
fn installed_state(dir: &Path) -> State {
    if installed_version(dir) >= VERSION && dir.join(menus::THEME).is_file() { State::Imported } else { State::Outdated }
}

/// A picture made small for the tab: [`PICTURE_WIDTH`] wide at twice the detail.
fn small(image: &ta_formats::dds::Image) -> egui::ColorImage {
    let width = (PICTURE_WIDTH * 2.0) as usize;
    let height = ((image.height as f32 / image.width.max(1) as f32) * width as f32).round().max(1.0) as usize;
    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let sx = (x * image.width as usize / width).min(image.width as usize - 1);
            let sy = (y * image.height as usize / height).min(image.height as usize - 1);
            let p = image.rgba[sy * image.width as usize + sx];
            pixels.extend_from_slice(&[p[0], p[1], p[2], 255]);
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([width, height], &pixels)
}

/// The game in use (of the `copies` found, or the folder `picked` while it still holds the game),
/// what it has of the look (its first pictures made small), and the mod installed in `held`.
fn find(copies: Vec<Install>, picked: Option<&Path>, held: Option<&Path>) -> (Found, Pictures) {
    let picked_gone = picked.filter(|p| !install::validate(p)).map(Path::to_path_buf);
    let install = crate::supcom::in_use(&copies, picked.filter(|_| picked_gone.is_none()));
    let state = held.map_or(State::New, installed_state);
    let mut found = Found { install: install.clone(), copies, picked_gone, state, ..Default::default() };
    let mut pictures = Pictures::new();
    let Some(game) = install else { return (found, pictures) };
    let sounds = ta_vfs::install::find_dir_ci(&game.root, "sounds");
    match Files::own(&game).and_then(|files| Ok((menus::contents(&files, sounds.as_deref())?, files))) {
        Ok((contents, files)) => {
            for name in contents.pictures.iter().take(PICTURES_SHOWN) {
                if let Ok(Some(image)) = menus::picture(&files, name) {
                    pictures.insert(name, small(&image));
                }
            }
            found.contents = Some(contents);
        }
        Err(e) => found.error = Some(tf("The game's interface could not be read: {why}", &[("why", &e)])),
    }
    (found, pictures)
}

/// Makes the look of the copy `game` into the mod and installs it where `place` says, in place
/// of the one there: when there is room on the disk for it. Nothing is installed when stopped
/// before all is made.
fn import(game: &Install, store_dir: &Path, place: &Place, progress: &Arc<Mutex<Progress>>, ending: &Ending) -> Result<Outcome> {
    let work_root = store_dir.join("cache").join("menus-imports");
    crate::supcom::enough_room(&[&work_root, &place.into], SIZE)?;
    let files = Files::own(game).context(t("Supreme Commander: Forged Alliance's interface could not be read."))?;
    let sounds = ta_vfs::install::find_dir_ci(&game.root, "sounds");
    let work = crate::maps::Workspace::new(&work_root)?;
    let payload = work.0.join("payload");
    {
        let mut p = progress.lock().unwrap();
        p.total = menus::STEPS;
        p.phase = Phase::Making;
    }
    let made = menus::make(&files, sounds.as_deref(), env!("CARGO_PKG_VERSION"), &mut |path, bytes| write_into(&payload, path, bytes), &mut |done, what| {
        let mut p = progress.lock().unwrap();
        p.done = done;
        p.current = what.to_string();
        !ending.stop.load(Ordering::Relaxed)
    })?;
    if made.stopped {
        return Ok(Outcome { stopped: true, ..Default::default() });
    }
    if made.pictures == 0 && !made.panel && made.sounds == 0 && !made.music {
        bail!("{}", t("Nothing of the look could be made."));
    }
    {
        let mut p = progress.lock().unwrap();
        p.done = menus::STEPS;
        // (A stop asked for while the last step was made is taken now, before the install.)
        if ending.stop.load(Ordering::Relaxed) {
            return Ok(Outcome { stopped: true, ..Default::default() });
        }
        p.phase = Phase::Installing;
    }
    let manifest = ta_vfs::mods::read_mod(&payload)?.context(t("The mod made has no manifest."))?.manifest;
    ta_vfs::package::install(&ta_vfs::package::plan(&payload)?, &manifest, &place.into)?;
    Ok(Outcome { made: true, skipped: made.skipped, ..Default::default() })
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
        assert_eq!(MOD_NAME, menus::MOD_NAME);
        assert_eq!(MOD_DESCRIPTION, menus::DESCRIPTION);
    }

    /// An import is told as it went: made, partly, failed, stopped (no notice).
    #[test]
    fn an_import_is_told_as_it_went() {
        let well = Outcome { made: true, ..Default::default() };
        let partly = Outcome { skipped: vec![("space_2".into(), "missing".into())], ..well.clone() };
        let broken = Outcome { error: Some("disk full".into()), ..Default::default() };
        let stopped = Outcome { stopped: true, ..Default::default() };
        assert_eq!([&well, &partly, &broken, &stopped].map(Outcome::went), [Went::Well, Went::Partly, Went::Badly, Went::Nothing]);
        assert!(well.summary().contains("MENU THEME is now FORGED ALLIANCE"), "{}", well.summary());
        assert!(well.notice().unwrap().contains("MENU THEME is now FORGED ALLIANCE"));
        assert!(partly.summary().contains("1 of its parts could not be made"), "{}", partly.summary());
        assert!(broken.notice().unwrap().ends_with("disk full"));
        assert!(stopped.notice().is_none());
    }

    /// MENU THEME goes to Forged Alliance's look once the mod is on (an import's too), whatever it
    /// was, and back to TA Reborn's own once it is off.
    #[test]
    fn the_look_follows_the_mod() {
        let mut theme = MenuTheme::Reborn;
        assert!(switch_on(&mut theme));
        assert_eq!(theme, MenuTheme::ForgedAlliance);
        assert!(!follow(&mut theme, true), "already Forged Alliance's");
        assert_eq!(theme, MenuTheme::ForgedAlliance);
        assert!(follow(&mut theme, false));
        assert_eq!(theme, MenuTheme::Reborn);
        assert!(!follow(&mut theme, false));
    }

    /// A copy is imported only whole and made by this converter: one without its theme (as the
    /// launcher's install of version 1 left it) or an older converter's is made again.
    #[test]
    fn a_copy_without_its_theme_is_made_again() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("ta-menus-state-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(dir.join("sounds")).unwrap();
        std::fs::write(dir.join("mod.toml"), menus::manifest("1.0.9")).unwrap();
        assert_eq!(installed_state(&dir), State::Outdated, "no theme");
        std::fs::create_dir_all(dir.join("menus/supcom")).unwrap();
        std::fs::write(dir.join(menus::THEME), "pictures = []\n").unwrap();
        assert_eq!(installed_state(&dir), State::Imported);
        std::fs::write(dir.join("mod.toml"), menus::manifest("1.0.9").replace(&format!("version = \"{VERSION}\""), "version = \"1\"")).unwrap();
        assert_eq!(installed_state(&dir), State::Outdated, "an older converter's");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A picture made small for the tab: its proportions kept, opaque.
    #[test]
    fn a_picture_is_made_small_for_the_tab() {
        let image = ta_formats::dds::Image { width: 1024, height: 768, rgba: vec![[10, 20, 30, 128]; 1024 * 768] };
        let small = small(&image);
        assert_eq!(small.size, [320, 240]);
        assert_eq!(small.pixels[0], egui::Color32::from_rgb(10, 20, 30));
    }
}
