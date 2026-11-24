//! `ta-tools supcom-music`: Supreme Commander: Forged Alliance's soundtrack made into TA Reborn's
//! music, into a folder to mount as a mod (docs/SUPCOM_MUSIC.md), as `ta_supcom::soundtrack::make`
//! makes it: a FLAC file a track and the list of them.

use std::path::Path;

use anyhow::{Context, Result};
use ta_supcom::install::{self, Install, Store};
use ta_supcom::soundtrack;

pub fn run(out: &Path, game: Option<&Path>) -> Result<()> {
    let install = match game {
        Some(g) => Install { root: install::validate_picked(g).context("no copy of Forged Alliance there")?, store: Store::Folder },
        None => install::discover().into_iter().next().context("no copy of Forged Alliance found (give its folder with --game)")?,
    };
    eprintln!("Forged Alliance: {} ({:?})", install.root.display(), install.store);
    let sounds = ta_vfs::install::find_dir_ci(&install.root, "sounds").context("the copy has no sounds folder")?;
    let tracks = soundtrack::tracks(&sounds)?;
    for t in &tracks {
        eprintln!("  {:<10} {:>2}  {:<18} {:>6.1} s  ({} wave {})", t.part.key(), t.number, t.stem, t.seconds, t.bank, t.wave);
    }
    let started = std::time::Instant::now();
    let made = soundtrack::make(&sounds, env!("CARGO_PKG_VERSION"), &mut |path, bytes| {
        let at = out.join(path);
        if let Some(dir) = at.parent() {
            std::fs::create_dir_all(dir).map_err(|e| ta_supcom::Error::Format(format!("{}: {e}", dir.display())))?;
        }
        std::fs::write(&at, bytes).map_err(|e| ta_supcom::Error::Format(format!("{}: {e}", at.display())))
    }, &mut |_, _| true)?;
    let minutes = tracks.iter().map(|t| t.seconds).sum::<f64>() / 60.0;
    let raw: u64 = tracks.iter().map(|t| t.bytes).sum();
    eprintln!("{} of {} tracks ({minutes:.0} minutes) made into {}: {} files, {} MB ({} MB uncompressed), in {:.1} s", made.tracks, tracks.len(), out.display(), made.files, made.bytes >> 20, raw >> 20, started.elapsed().as_secs_f32());
    for (stem, why) in &made.skipped {
        eprintln!("  {stem} left out: {why}");
    }
    Ok(())
}
