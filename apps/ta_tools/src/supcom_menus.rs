//! `ta-tools supcom-menus`: Supreme Commander: Forged Alliance's look for TA Reborn's menus, into a
//! folder to mount as a mod (docs/SUPCOM_MENUS.md), as `ta_supcom::menus::make` makes it: its
//! colours, its menus' pictures, its panel art, its interface sounds and its menu music, with the
//! theme the game's MENU THEME reads.

use std::path::Path;

use anyhow::{Context, Result};
use ta_supcom::files::Files;
use ta_supcom::install::{self, Install, Store};
use ta_supcom::menus;

pub fn run(out: &Path, game: Option<&Path>) -> Result<()> {
    let install = match game {
        Some(g) => Install { root: install::validate_picked(g).context("no copy of Forged Alliance there")?, store: Store::Folder },
        None => install::discover().into_iter().next().context("no copy of Forged Alliance found (give its folder with --game)")?,
    };
    eprintln!("Forged Alliance: {} ({:?})", install.root.display(), install.store);
    let files = Files::own(&install)?;
    let sounds = ta_vfs::install::find_dir_ci(&install.root, "sounds");
    let found = menus::contents(&files, sounds.as_deref())?;
    for (key, c) in found.skin.palette() {
        eprintln!("  colour {key:<12} #{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
    }
    eprintln!("  {} of {} pictures, panel art: {}, {} sounds, menu music: {}", found.pictures.len(), menus::PICTURES.len(), found.panel, found.sounds.len(), found.music_seconds.map_or("none".into(), |s| format!("{s:.1} s")));
    let started = std::time::Instant::now();
    let made = menus::make(&files, sounds.as_deref(), env!("CARGO_PKG_VERSION"), &mut |path, bytes| {
        let at = out.join(path);
        if let Some(dir) = at.parent() {
            std::fs::create_dir_all(dir).map_err(|e| ta_supcom::Error::Format(format!("{}: {e}", dir.display())))?;
        }
        std::fs::write(&at, bytes).map_err(|e| ta_supcom::Error::Format(format!("{}: {e}", at.display())))
    }, &mut |_, _| true)?;
    eprintln!("made into {}: {} pictures, panel {}, {} sounds, music {}: {} files, {} KB, in {:.1} s", out.display(), made.pictures, made.panel, made.sounds, made.music, made.files, made.bytes >> 10, started.elapsed().as_secs_f32());
    for (what, why) in &made.skipped {
        eprintln!("  {what} left out: {why}");
    }
    Ok(())
}
