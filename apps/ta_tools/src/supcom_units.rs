//! `ta-tools supcom-units`: Supreme Commander: Forged Alliance's units made into TA units, into a
//! folder to mount as a mod (docs/SUPCOM_UNITS.md), as `ta_supcom::factions::make` makes them:
//! their models, paint, build pictures, unit files, scripts and weapons, their factions' sides,
//! menus and wrecks, and the sides' emblems (in TA's palette, from the copy of TA found);
//! `--preview` sheets of the pictures, a faction a sheet.

use std::path::Path;

use anyhow::{Context, Result};
use ta_supcom::factions::{make, pictures};
use ta_supcom::files::Files;
use ta_supcom::install::{self, Install, Store};
use ta_supcom::unit::{self, Faction, UnitBlueprint};

pub fn run(out: &Path, only: Option<&str>, faction: Option<&str>, game: Option<&Path>, preview: Option<&Path>) -> Result<()> {
    let install = match game {
        Some(g) => Install { root: install::validate_picked(g).context("no copy of Forged Alliance there")?, store: Store::Folder },
        None => install::discover().into_iter().next().context("no copy of Forged Alliance found (give its folder with --game)")?,
    };
    eprintln!("Forged Alliance: {} ({:?})", install.root.display(), install.store);
    let files = Files::own(&install)?;
    let wanted: Option<Vec<String>> = only.map(|o| o.split(',').map(|s| s.trim().to_ascii_uppercase()).filter(|s| !s.is_empty()).collect());
    let faction = match faction {
        Some(f) => Some(Faction::PLAYABLE.into_iter().find(|p| p.name().eq_ignore_ascii_case(f.trim())).with_context(|| format!("no faction {f} (uef, cybran, aeon, seraphim)"))?),
        None => None,
    };
    let units: Vec<UnitBlueprint> = unit::catalogue(&files)?
        .into_iter()
        .filter(|u| u.playable() && wanted.as_ref().is_none_or(|w| w.contains(&u.id.to_ascii_uppercase())) && faction.as_ref().is_none_or(|f| u.faction == *f))
        .collect();
    // TA's own palette, for the sides' emblems: the original game alone, as every player has it.
    let palette = ta_palette();
    if palette.is_none() {
        eprintln!("no copy of TA found: the sides' emblems are left out");
    }
    let started = std::time::Instant::now();
    let made = make::make(&files, &units, palette.as_ref(), env!("CARGO_PKG_VERSION"), &mut |path, bytes| {
        let at = out.join(path);
        if let Some(dir) = at.parent() {
            std::fs::create_dir_all(dir).map_err(|e| ta_supcom::Error::Format(format!("{}: {e}", dir.display())))?;
        }
        std::fs::write(&at, bytes).map_err(|e| ta_supcom::Error::Format(format!("{}: {e}", at.display())))
    }, &mut |_, _| true)?;
    eprintln!("{} of {} units made into {}: {} files, {} MB, in {:.1} s", made.units, units.len(), out.display(), made.files, made.bytes >> 20, started.elapsed().as_secs_f32());
    for (id, why) in &made.skipped {
        eprintln!("  {id} left out: {why}");
    }
    if let Some(dir) = preview {
        sheets(out, &units, dir)?;
    }
    Ok(())
}

/// TA's palette, from the copy of TA found: none without one.
fn ta_palette() -> Option<ta_formats::image::Palette> {
    let ta = ta_vfs::install::discover(None).ok()?;
    let mut vfs = ta_vfs::Vfs::new();
    vfs.mount_game_dir(&ta.root, "base").ok()?;
    ta_formats::image::Palette::parse(&vfs.read_opt("palettes/palette.pal")?).ok()
}

/// A sheet of the units' build pictures a faction, in order of their ids, eight across.
fn sheets(out: &Path, units: &[UnitBlueprint], dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    for faction in Faction::PLAYABLE {
        let theirs: Vec<&UnitBlueprint> = units.iter().filter(|u| u.faction == faction).collect();
        if theirs.is_empty() {
            continue;
        }
        let side = pictures::SIDE;
        let (across, rows) = (8u32, (theirs.len() as u32).div_ceil(8));
        let mut sheet = image::RgbImage::from_pixel(across * (side + 4), rows * (side + 4), image::Rgb([24, 26, 30]));
        for (k, u) in theirs.iter().enumerate() {
            let Ok(bytes) = std::fs::read(out.join(format!("unitpics/{}.pcx", u.id.to_ascii_lowercase()))) else { continue };
            let Ok(picture) = ta_formats::pcx::PcxImage::parse(&bytes) else { continue };
            let rgba = picture.to_rgba();
            let (x0, y0) = ((k as u32 % across) * (side + 4) + 2, (k as u32 / across) * (side + 4) + 2);
            for y in 0..picture.height.min(side) {
                for x in 0..picture.width.min(side) {
                    let i = ((y * picture.width + x) * 4) as usize;
                    sheet.put_pixel(x0 + x, y0 + y, image::Rgb([rgba[i], rgba[i + 1], rgba[i + 2]]));
                }
            }
        }
        let path = dir.join(format!("{}.png", faction.name().to_ascii_lowercase()));
        sheet.save(&path)?;
        eprintln!("{}: {} pictures in {}", faction.name(), theirs.len(), path.display());
    }
    Ok(())
}
