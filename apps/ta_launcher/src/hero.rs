//! The Play banner's picture when the player has no photo of their own: the map of their newest
//! battle (its title as the game wrote it in the record, "Seven Islands — 2 players"), else one
//! of TA Reborn's own maps, another each day. It is drawn from the install's own art
//! (`ta_render::map_preview`, as the Maps page draws its pictures) on a thread of its own and
//! kept on disk, so a later start shows it at once. Without an install there is no palette to
//! paint it in, and no picture: the banner keeps its radar.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use egui::{ColorImage, TextureHandle, TextureOptions};
use ta_formats::image::Palette;
use ta_formats::tnt::TntMap;
use ta_vfs::Vfs;

/// The map's picture is drawn this many pixels along its longer side, then cut to the band of
/// it most worth showing: `BAND` times as wide as high (the banner, wider still, shows part of
/// it, drifting across).
const SIDE: u32 = 2048;
const BAND: f32 = 2.6;
/// A band with more than this share of black (a void between islands, unpainted ground) is
/// not worth a banner: the next choice is tried.
const MOST_DARK: f32 = 0.4;
/// Bumped when the pictures are drawn otherwise (those cached are then made again).
const VERSION: u32 = 3;

/// Where the banner's map came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The map of the player's newest saved game or replay.
    LastBattle,
    /// One of TA Reborn's own maps, chosen by the day.
    OfTheDay,
}

/// The picture on show: the map's name (as its file and the record have it), where it came
/// from, and how much to darken it (a snowfield as much as a forest: all read alike under the
/// banner's words).
pub struct Shown {
    pub map: String,
    pub source: Source,
    pub texture: TextureHandle,
    pub level: f32,
    /// When it came (egui's time): it fades in over what the banner showed before.
    pub since: f64,
}

type Made = Option<(String, Source, ColorImage)>;

/// How bright a picture is drawn (a share of as painted) so that its mean comes out alike
/// whatever its world: a white-out darkened most, a dark one hardly at all.
pub fn level(image: &ColorImage) -> f32 {
    let n = image.pixels.len().max(1) as f32;
    let mean = image.pixels.iter().map(|p| p.r() as f32 * 0.21 + p.g() as f32 * 0.72 + p.b() as f32 * 0.07).sum::<f32>() / n;
    (100.0 / mean.max(1.0)).clamp(0.5, 1.0)
}

#[derive(Default)]
pub struct Hero {
    /// What the picture was last asked for (the install, the record, the day).
    asked: Option<String>,
    job: Option<Receiver<Made>>,
    pub shown: Option<Shown>,
}

impl Hero {
    /// Asks for the banner's picture: for the install at `root`, the newest battle's record
    /// title `last` and the day `today` (days since 1970), kept in `cache`. Asked again only
    /// when one of them changes.
    pub fn want(&mut self, root: Option<&Path>, last: Option<&str>, cache: PathBuf, today: u64) {
        let key = format!("{root:?}|{last:?}|{today}");
        if self.asked.as_deref() == Some(key.as_str()) {
            return;
        }
        self.asked = Some(key);
        self.job = None;
        let Some(root) = root.map(Path::to_path_buf) else {
            self.shown = None;
            return;
        };
        let last = last.map(str::to_string);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("launcher banner".into()).spawn(move || {
            let made = make(&root, last.as_deref(), ta_vfs::builtin_dir().as_deref(), &cache, today);
            let _ = tx.send(made);
        });
        match spawned {
            Ok(_) => self.job = Some(rx),
            Err(e) => log::warn!("the banner's picture: {e}"),
        }
    }

    /// Takes in the picture once it is made; true when something changed.
    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
        let Some(rx) = &self.job else { return false };
        match rx.try_recv() {
            Ok(made) => {
                self.job = None;
                let since = ctx.input(|i| i.time);
                self.shown = made.map(|(map, source, image)| {
                    let level = level(&image);
                    Shown { map, source, texture: ctx.load_texture("launcher-banner-map", image, TextureOptions::LINEAR), level, since }
                });
                true
            }
            Err(TryRecvError::Empty) => {
                // (Asleep otherwise: woken to take the picture in.)
                ctx.request_repaint_after(std::time::Duration::from_millis(120));
                false
            }
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                true
            }
        }
    }

    /// Still making the picture.
    pub fn loading(&self) -> bool {
        self.job.is_some()
    }
}

/// A row of worlds' pictures (the benchmark's scenes' maps, before a first run): each drawn from
/// the install's art at [`WORLD_SIDE`] pixels and kept on disk, all made on one thread.
#[derive(Default)]
pub struct Worlds {
    asked: Option<String>,
    job: Option<Receiver<Vec<(String, ColorImage)>>>,
    pub pictures: Vec<(String, TextureHandle)>,
}

/// A world's picture's longer side.
const WORLD_SIDE: u32 = 400;

impl Worlds {
    /// Asks for the pictures of `maps` in the install at `root`, kept in `cache` (again only
    /// when these change).
    pub fn want(&mut self, root: Option<&Path>, maps: &[&str], cache: PathBuf) {
        let key = format!("{root:?}|{maps:?}");
        if self.asked.as_deref() == Some(key.as_str()) {
            return;
        }
        self.asked = Some(key);
        self.job = None;
        self.pictures.clear();
        let Some(root) = root.map(Path::to_path_buf) else { return };
        let maps: Vec<String> = maps.iter().map(|m| m.to_string()).collect();
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("launcher worlds".into()).spawn(move || {
            let _ = tx.send(make_worlds(&root, &maps, ta_vfs::builtin_dir().as_deref(), &cache));
        });
        match spawned {
            Ok(_) => self.job = Some(rx),
            Err(e) => log::warn!("the worlds' pictures: {e}"),
        }
    }

    /// Takes in the pictures once made; true when they came.
    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
        let Some(rx) = &self.job else { return false };
        match rx.try_recv() {
            Ok(made) => {
                self.job = None;
                self.pictures = made.into_iter().map(|(map, image)| {
                    let texture = ctx.load_texture(format!("launcher-world-{map}"), image, TextureOptions::LINEAR);
                    (map, texture)
                }).collect();
                true
            }
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(120));
                false
            }
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                true
            }
        }
    }

    /// Still making the pictures.
    pub fn loading(&self) -> bool {
        self.job.is_some()
    }
}

/// The pictures of `maps` found in the install (and TA Reborn's own maps), from the cache or
/// drawn and kept there; those that cannot be drawn are left out.
fn make_worlds(root: &Path, maps: &[String], builtin: Option<&Path>, cache: &Path) -> Vec<(String, ColorImage)> {
    let key = |map: &str| format!("world|{}|{}|{}|{WORLD_SIDE}|v{VERSION}", root.display(), map.to_lowercase(), stamp(builtin, map));
    let mut opened: Option<(Vfs, Palette)> = None;
    let mut made = Vec::new();
    for map in maps {
        let file = crate::library::cache_file(cache, &key(map));
        if let Some(image) = crate::library::read_cached(&file) {
            made.push((map.clone(), image));
            continue;
        }
        if opened.is_none() {
            let mut vfs = Vfs::new();
            if let Err(e) = vfs.mount_game_dir(root, "base") {
                log::warn!("the worlds' pictures: {e}");
                return made;
            }
            if let Some(dir) = builtin && let Err(e) = vfs.mount_mod_dir(dir, ta_vfs::BUILTIN_LAYER) {
                log::warn!("the worlds' pictures: {e}");
            }
            let Some(palette) = vfs.read_opt("palettes/palette.pal").and_then(|bytes| Palette::parse(&bytes).ok()) else { return made };
            opened = Some((vfs, palette));
        }
        let Some((vfs, palette)) = &opened else { return made };
        let Some(terrain) = vfs.read_opt(&format!("maps/{map}.tnt")).and_then(|bytes| TntMap::parse(&bytes).ok()) else { continue };
        let picture = ta_render::map_preview::render(&terrain, palette, WORLD_SIDE);
        if picture.width == 0 || picture.height == 0 {
            continue;
        }
        let image = ColorImage::from_rgba_unmultiplied([picture.width, picture.height], &picture.rgba);
        crate::library::write_cached(&file, &image);
        made.push((map.clone(), image));
    }
    made
}

/// The days since 1970 (the map of the day changes at midnight UTC).
pub fn today() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400)
}

/// The map a record's title names: "Autosave · Seven Islands — 2 players" is Seven Islands. A
/// mission's record is titled by the mission, which names no map.
pub fn map_of_record(title: &str) -> Option<String> {
    let title = title.strip_prefix("Autosave · ").unwrap_or(title);
    let (map, _) = title.rsplit_once(" — ")?;
    let map = map.trim();
    (!map.is_empty() && !map.contains(['/', '\\', ':'])).then(|| map.to_string())
}

/// TA Reborn's map for `today`: its own maps in turn, a day each.
pub fn map_of_the_day(builtin: &Path, today: u64) -> Option<String> {
    let mut maps: Vec<String> = std::fs::read_dir(builtin.join("maps")).ok()?.flatten()
        .filter_map(|entry| {
            let path = entry.path();
            path.extension().is_some_and(|e| e.eq_ignore_ascii_case("tnt")).then(|| path.file_stem()?.to_str().map(str::to_string)).flatten()
        })
        .collect();
    maps.sort_by_key(|m| m.to_lowercase());
    (!maps.is_empty()).then(|| maps[(today % maps.len() as u64) as usize].clone())
}

/// What a picture is cached by: the install, the map (and, for one of TA Reborn's own, its file's
/// size and time: an update that redraws a map draws its picture again), and how pictures are
/// drawn.
fn cache_key(root: &Path, map: &str, builtin: Option<&Path>) -> String {
    format!("banner|{}|{}|{}|{SIDE}|v{VERSION}", root.display(), map.to_lowercase(), stamp(builtin, map))
}

/// A built-in map's file's size and time ("" for a map of the install's, which do not change),
/// its name matched as TA's are, whatever its case.
fn stamp(builtin: Option<&Path>, map: &str) -> String {
    let file = builtin.and_then(|dir| std::fs::read_dir(dir.join("maps")).ok()).and_then(|entries| entries.flatten().find(|entry| {
        let path = entry.path();
        path.extension().is_some_and(|e| e.eq_ignore_ascii_case("tnt")) && path.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case(map))
    }));
    file.and_then(|entry| entry.metadata().ok()).map_or(String::new(), |meta| {
        let time = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        format!("{}@{time}", meta.len())
    })
}

/// The banner's picture: the newest battle's map, else the map of the day; read from the
/// cache when it was made before, else drawn and kept there.
fn make(root: &Path, last: Option<&str>, builtin: Option<&Path>, cache: &Path, today: u64) -> Made {
    let mut choices = Vec::new();
    // (`TA_BANNER_MAP=<map>`: that map, for captures and films.)
    if let Some(map) = std::env::var("TA_BANNER_MAP").ok().filter(|m| !m.trim().is_empty()) {
        choices.push((map.trim().to_string(), Source::OfTheDay));
    }
    if let Some(map) = last.and_then(map_of_record) {
        choices.push((map, Source::LastBattle));
    }
    // (A world of asteroids is mostly void: the next day's map stands in.)
    for day in today..today + 3 {
        if let Some(map) = builtin.and_then(|dir| map_of_the_day(dir, day)) && !choices.iter().any(|(m, _): &(String, Source)| *m == map) {
            choices.push((map, Source::OfTheDay));
        }
    }
    // (The install's archives are opened only when a picture has to be drawn.)
    let mut opened: Option<(Vfs, Palette)> = None;
    for (map, source) in choices {
        let file = crate::library::cache_file(cache, &cache_key(root, &map, builtin));
        if let Some(image) = crate::library::read_cached(&file) {
            return Some((map, source, image));
        }
        // (A map found mostly void before is not drawn again to find it so.)
        let void = file.with_extension("void");
        if void.is_file() {
            continue;
        }
        if opened.is_none() {
            let mut vfs = Vfs::new();
            if let Err(e) = vfs.mount_game_dir(root, "base") {
                log::warn!("the banner's picture: {e}");
                return None;
            }
            if let Some(dir) = builtin && let Err(e) = vfs.mount_mod_dir(dir, ta_vfs::BUILTIN_LAYER) {
                log::warn!("the banner's picture: {e}");
            }
            let palette = vfs.read_opt("palettes/palette.pal").and_then(|bytes| Palette::parse(&bytes).ok())?;
            opened = Some((vfs, palette));
        }
        let Some((vfs, palette)) = &opened else { return None };
        let Some(bytes) = vfs.read_opt(&format!("maps/{map}.tnt")) else { continue };
        let Ok(terrain) = TntMap::parse(&bytes) else { continue };
        let picture = ta_render::map_preview::render(&terrain, palette, SIDE);
        let Some((band, dark)) = best_band(&picture.rgba, picture.width, picture.height) else { continue };
        if dark > MOST_DARK {
            log::info!("the banner: {map} is {:.0}% void at best, passed over", dark * 100.0);
            if std::fs::create_dir_all(cache).is_ok() {
                let _ = std::fs::write(&void, b"");
            }
            continue;
        }
        let [x0, y0, x1, y1] = band;
        let mut rgba = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
        for y in y0..y1 {
            rgba.extend_from_slice(&picture.rgba[(y * picture.width + x0) * 4..(y * picture.width + x1) * 4]);
        }
        let image = ColorImage::from_rgba_unmultiplied([x1 - x0, y1 - y0], &rgba);
        crate::library::write_cached(&file, &image);
        return Some((map, source, image));
    }
    None
}

/// The band of a picture (`w` × `h` sRGB pixels) most worth a banner: [`BAND`] times as wide as
/// high where the picture is taller than that (else its whole height), placed where it shows
/// the least black void and the most variety; its pixels `[x0, y0, x1, y1]` and the share of it
/// that is void.
fn best_band(rgba: &[u8], w: usize, h: usize) -> Option<([usize; 4], f32)> {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return None;
    }
    // Along the picture's length (down a tall one, across a wide one): each line's void, and
    // its ground's brightness and that squared (the edge of a void is no variety).
    let across = w as f32 / h as f32 > BAND;
    let (length, band) = if across { (w, ((h as f32 * BAND) as usize).clamp(1, w)) } else { (h, ((w as f32 / BAND) as usize).clamp(1, h)) };
    let breadth = if across { h } else { w };
    let mut lines = vec![(0u64, 0u64, 0u64); length];
    for y in 0..h {
        for x in 0..w {
            let p = &rgba[(y * w + x) * 4..];
            let luma = (p[0] as u64 * 54 + p[1] as u64 * 183 + p[2] as u64 * 19) >> 8;
            let line = &mut lines[if across { x } else { y }];
            if luma < 28 {
                line.0 += 1;
            } else {
                line.1 += luma;
                line.2 += luma * luma;
            }
        }
    }
    let n = (band * breadth) as f64;
    let step = (band / 6).max(1);
    let mut best: Option<(f64, usize, f32)> = None;
    let mut start = 0;
    loop {
        let (dark, sum, square) = lines[start..start + band].iter().fold((0u64, 0u64, 0u64), |a, l| (a.0 + l.0, a.1 + l.1, a.2 + l.2));
        let ground = (n - dark as f64).max(1.0);
        let dark = dark as f64 / n;
        let mean = sum as f64 / ground;
        let spread = (square as f64 / ground - mean * mean).max(0.0).sqrt();
        // (Ground over void; variety over flat colour; a little less for a white-out.)
        let score = (1.0 - dark) * (0.6 + 0.4 * (spread / 48.0).min(1.0)) * if mean > 205.0 { 0.85 } else { 1.0 };
        if best.is_none_or(|(s, _, _)| score > s) {
            best = Some((score, start, dark as f32));
        }
        if start + band >= length {
            break;
        }
        start = (start + step).min(length - band);
    }
    let (_, start, dark) = best?;
    let band = if across { [start, 0, start + band, h] } else { [0, start, w, start + band] };
    Some((band, dark))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_the_install_discards_pending_banner_artwork() {
        let (send, receive) = channel();
        let mut hero = Hero { job: Some(receive), ..Default::default() };
        hero.want(None, None, PathBuf::new(), 0);
        assert!(!hero.loading(), "a removed install must not keep its old reader active");
        let image = ColorImage::filled([1, 1], egui::Color32::GREEN);
        assert!(send.send(Some(("Old map".into(), Source::LastBattle, image))).is_err());
        assert!(!hero.poll(&egui::Context::default()));
        assert!(hero.shown.is_none());
    }

    #[test]
    fn removing_the_install_discards_pending_world_artwork() {
        let (send, receive) = channel();
        let mut worlds = Worlds { job: Some(receive), ..Default::default() };
        worlds.want(None, &["Old map"], PathBuf::new());
        assert!(!worlds.loading());
        let image = ColorImage::filled([1, 1], egui::Color32::GREEN);
        assert!(send.send(vec![("Old map".into(), image)]).is_err());
        assert!(!worlds.poll(&egui::Context::default()));
        assert!(worlds.pictures.is_empty());
    }

    #[test]
    fn a_records_title_names_its_map() {
        assert_eq!(map_of_record("Seven Islands — 2 players").as_deref(), Some("Seven Islands"));
        assert_eq!(map_of_record("Autosave · Coast To Coast — 4 players").as_deref(), Some("Coast To Coast"));
        // (A name with a dash of its own keeps it: the players come after the last.)
        assert_eq!(map_of_record("Dark — Side — 2 players").as_deref(), Some("Dark — Side"));
        assert_eq!(map_of_record("Gather the Army"), None, "a mission names no map");
        assert_eq!(map_of_record(" — 2 players"), None);
        assert_eq!(map_of_record("../x — 2 players"), None, "never a path");
    }

    #[test]
    fn the_banner_shows_the_ground_not_the_void() {
        // A square picture: void above, painted ground (striped) below.
        let (w, h) = (260, 260);
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let v = if y < 150 { 4 } else if (x / 7 + y / 5) % 2 == 0 { 90 } else { 170 };
                rgba[(y * w + x) * 4..][..4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let ([x0, y0, x1, y1], dark) = best_band(&rgba, w, h).unwrap();
        assert_eq!((x0, x1), (0, w), "the whole width of a square map");
        assert_eq!(y1 - y0, (w as f32 / BAND) as usize);
        assert!(y0 >= 150 && dark < 0.01, "the band on the ground: {y0}..{y1}, {dark}");
        // A wide picture: a band across it, its full height.
        let (w, h) = (400, 100);
        let rgba = vec![120u8; w * h * 4];
        let ([x0, y0, x1, y1], _) = best_band(&rgba, w, h).unwrap();
        assert_eq!((y0, y1, x1 - x0), (0, h, 260));
        // All void: said so.
        assert!(best_band(&vec![0u8; 64 * 64 * 4], 64, 64).unwrap().1 > 0.99);
        assert!(best_band(&[], 0, 0).is_none());
    }

    /// A built-in map redrawn by an update is drawn again: its picture's key follows the file.
    #[test]
    fn a_redrawn_built_in_map_is_pictured_anew() {
        let dir = std::env::temp_dir().join(format!("ta_launcher_banner_stamp_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("maps")).unwrap();
        std::fs::write(dir.join("maps/Sapphire Reach.tnt"), b"first").unwrap();
        let root = Path::new("C:/Games/TA");
        let before = cache_key(root, "Sapphire Reach", Some(&dir));
        assert_eq!(before, cache_key(root, "SAPPHIRE REACH", Some(&dir)), "as the scenes name it, the same picture");
        std::fs::write(dir.join("maps/Sapphire Reach.tnt"), b"redrawn, longer").unwrap();
        assert_ne!(before, cache_key(root, "Sapphire Reach", Some(&dir)));
        // (A map of the install's has no stamp: its key stands.)
        assert_eq!(cache_key(root, "Coast To Coast", Some(&dir)), cache_key(root, "Coast To Coast", None));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_map_of_the_day_turns_over_daily_through_every_map() {
        let dir = std::env::temp_dir().join(format!("ta_launcher_banner_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("maps")).unwrap();
        for name in ["Bravo.tnt", "alpha.tnt", "Charlie.TNT", "notes.txt"] {
            std::fs::write(dir.join("maps").join(name), b"").unwrap();
        }
        let days: Vec<String> = (0..4).map(|day| map_of_the_day(&dir, 300 + day).unwrap()).collect();
        assert_eq!(days, ["alpha", "Bravo", "Charlie", "alpha"]);
        assert_eq!(map_of_the_day(&dir.join("missing"), 1), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
