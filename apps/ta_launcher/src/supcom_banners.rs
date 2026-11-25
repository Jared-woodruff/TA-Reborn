//! Banners for the Supreme Commander imports' mods installed without one (an older TA Reborn's
//! imports made none): each made from the player's own copy of the game as an import makes it
//! (`ta_supcom::banner`), written into the mod's folder and named by its manifest, on a worker,
//! each mod tried once a launch; the mods are read again once any is made. Without the game, or
//! where its pictures are missing, a mod keeps the plate the library draws for one without a
//! banner.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use ta_supcom::banner;
use ta_supcom::files::Files;
use ta_supcom::install::Install;

use crate::modlist::ModList;
use crate::supcom::Installed;

/// Which import's mod a banner is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Maps,
    Factions,
    Music,
    Menus,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Maps, Kind::Factions, Kind::Music, Kind::Menus];

    /// Its mod's id, and the name its import gives it (another mod of the id is not ours).
    pub fn mod_of(self) -> (&'static str, &'static str) {
        match self {
            Kind::Maps => (crate::supcom::PACK_ID, crate::supcom::PACK_NAME),
            Kind::Factions => (crate::supcom_factions::MOD_ID, crate::supcom_factions::MOD_NAME),
            Kind::Music => (crate::supcom_music::MOD_ID, crate::supcom_music::MOD_NAME),
            Kind::Menus => (crate::supcom_menus::MOD_ID, crate::supcom_menus::MOD_NAME),
        }
    }
}

/// The imports' mods `mods` lists without a banner, each with its folder: those an import made
/// (named as it names them), folders of a mods folder (not an archive, nor in TA Reborn's program
/// folder, which only an administrator may change).
pub fn wanted(mods: &ModList) -> Vec<(Kind, PathBuf)> {
    Kind::ALL
        .into_iter()
        .filter_map(|kind| {
            let (id, name) = kind.mod_of();
            let copy = Installed::listed(mods, id, name)?;
            let bare = mods.get(id)?.info.manifest.media.banner.trim().is_empty();
            (copy.ours && bare && !copy.archive && !copy.program_folder).then_some((kind, copy.path))
        })
        .collect()
}

/// The banner of the `kind` of mod in `folder`, made from the copy `game` (its files `files`).
fn make(kind: Kind, folder: &Path, game: &Install, files: &Files) -> Option<Vec<u8>> {
    match kind {
        Kind::Factions => {
            // (The emblems of the factions it holds.)
            let sides = crate::supcom_factions::installed_sides(folder);
            banner::factions(files, &sides.iter().map(String::as_str).collect::<Vec<_>>())
        }
        Kind::Menus => banner::menus(files),
        Kind::Music => {
            let sounds = ta_vfs::install::find_dir_ci(&game.root, "sounds");
            let loudness = sounds.and_then(|s| ta_supcom::soundtrack::menu_loudness(&s).ok().flatten()).unwrap_or_default();
            Some(banner::music(Some(files), &loudness))
        }
        Kind::Maps => crate::supcom::pack_banner(&folder.join("maps"), &game.maps(), &ta_supcom::install::user_map_folders(), Some(files)),
    }
}

/// `png` written into the mod in `folder` as its banner, and named by its manifest: the
/// manifest kept as it is, its `[media]` table added after it. One with a `[media]` table of its
/// own is left alone (none of the imports' has one: someone wrote it), and so said (false).
pub fn write(folder: &Path, png: &[u8]) -> std::io::Result<bool> {
    let manifest = folder.join("mod.toml");
    let mut text = std::fs::read_to_string(&manifest)?;
    let table: toml::Table = toml::from_str(&text).map_err(std::io::Error::other)?;
    if table.contains_key("media") {
        return Ok(false);
    }
    // (Each file whole or not at all: written beside, then put in its place.)
    let picture = folder.join(banner::PATH);
    if let Some(media) = picture.parent() {
        std::fs::create_dir_all(media)?;
    }
    let part = picture.with_extension("png.part");
    std::fs::write(&part, png)?;
    std::fs::rename(&part, &picture)?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&banner::manifest_lines());
    let part = folder.join("mod.toml.part");
    std::fs::write(&part, text)?;
    std::fs::rename(&part, &manifest)?;
    Ok(true)
}

/// The banners being made, and the mods tried this launch.
#[derive(Default)]
pub struct Banners {
    job: Option<mpsc::Receiver<usize>>,
    tried: BTreeSet<PathBuf>,
}

impl Banners {
    /// Makes on a worker the banners the mods `wanted` lack, of those not tried yet this launch,
    /// from the copy of the game in use (the folder the player chose, kept in `store_dir`'s
    /// settings, while it holds the game; else the first copy found): none without one.
    pub fn start(&mut self, wanted: Vec<(Kind, PathBuf)>, store_dir: &Path) {
        if self.job.is_some() {
            return;
        }
        let wanted: Vec<(Kind, PathBuf)> = wanted.into_iter().filter(|(_, folder)| self.tried.insert(folder.clone())).collect();
        if wanted.is_empty() {
            return;
        }
        let picked = crate::supcom::load_choice(store_dir).filter(|p| ta_supcom::install::validate(p));
        let (send, receive) = mpsc::channel();
        self.job = Some(receive);
        let spawned = std::thread::Builder::new().name("supcom banners".into()).spawn(move || {
            let made = (|| {
                let game = crate::supcom::in_use(&crate::supcom::copies_found(), picked.as_deref())?;
                let files = Files::own(&game).ok()?;
                Some(wanted.iter().filter(|(kind, folder)| {
                    let Some(png) = make(*kind, folder, &game, &files) else { return false };
                    write(folder, &png).inspect_err(|e| log::warn!("{}: its banner could not be written: {e}", folder.display())).unwrap_or(false)
                }).count())
            })()
            .unwrap_or(0);
            let _ = send.send(made);
        });
        if spawned.is_err() {
            self.job = None;
        }
    }

    /// How many banners were made, once the worker is done (the mods to be read again then).
    pub fn poll(&mut self) -> Option<usize> {
        let made = match self.job.as_ref()?.try_recv() {
            Ok(made) => made,
            Err(mpsc::TryRecvError::Disconnected) => 0,
            Err(mpsc::TryRecvError::Empty) => return None,
        };
        self.job = None;
        Some(made)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mod's manifest names the banner written beside it, kept as it was otherwise; one with a
    /// `[media]` table of its own is left as it is.
    #[test]
    fn a_banner_is_written_and_named() {
        let dir = std::env::temp_dir().join(format!("ta-launcher-banner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = "# made by an import\nid = \"supcom-music\"\nname = \"Supreme Commander music\"\nversion = \"1\"\ntags = [\"music\"]";
        std::fs::write(dir.join("mod.toml"), manifest).unwrap();
        assert!(write(&dir, b"\x89PNG banner").unwrap());
        assert_eq!(std::fs::read(dir.join(banner::PATH)).unwrap(), b"\x89PNG banner");
        let text = std::fs::read_to_string(dir.join("mod.toml")).unwrap();
        assert!(text.starts_with(manifest) && text.ends_with("[media]\nbanner = \"media/banner.png\"\n"), "{text}");
        let info = ta_vfs::mods::read_mod(&dir).unwrap().unwrap();
        assert_eq!((info.manifest.id.as_str(), info.manifest.media.banner.as_str(), info.manifest.tags.len()), ("supcom-music", "media/banner.png", 1));
        assert!(!dir.join("mod.toml.part").exists() && !dir.join("media/banner.png.part").exists());
        // (Its own `[media]` table: left alone.)
        let own = "id = \"x\"\nname = \"X\"\n\n[media]\nicon = \"icon.png\"\n";
        std::fs::write(dir.join("mod.toml"), own).unwrap();
        assert!(!write(&dir, b"other").unwrap());
        assert_eq!(std::fs::read_to_string(dir.join("mod.toml")).unwrap(), own);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The mods wanting a banner: an import's (named as it names them) without one; not another
    /// mod of the id, nor one with a banner already.
    #[test]
    fn the_imports_mods_without_a_banner_want_one() {
        let dir = std::env::temp_dir().join(format!("ta-launcher-banners-wanted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mods = dir.join("mods");
        let put = |id: &str, name: &str, extra: &str| {
            std::fs::create_dir_all(mods.join(id)).unwrap();
            std::fs::write(mods.join(id).join("mod.toml"), format!("id = \"{id}\"\nname = \"{name}\"\nversion = \"1\"\n{extra}")).unwrap();
        };
        put(crate::supcom_music::MOD_ID, crate::supcom_music::MOD_NAME, "");
        put(crate::supcom_menus::MOD_ID, crate::supcom_menus::MOD_NAME, "[media]\nbanner = \"media/banner.png\"\n");
        put(crate::supcom_factions::MOD_ID, "Someone's own factions", "");
        put(crate::supcom::PACK_ID, crate::supcom::PACK_NAME, "");
        let list = ModList::scan(vec![crate::modlist::ModFolder { label: "Mods", path: mods.clone(), exists: true }], &[]);
        let wanted: Vec<Kind> = wanted(&list).into_iter().map(|(kind, _)| kind).collect();
        assert_eq!(wanted, [Kind::Maps, Kind::Music]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
