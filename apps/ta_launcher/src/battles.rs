//! The player's own battles, for the Play page: recent saved games (to continue), recent
//! replays (to watch) and the newest picture taken in the game (behind the Play banner). The
//! folders are only listed; of a record, its last few hundred bytes are read (its title, its
//! length and when it was saved, which bincode writes last) and its first four (the version of
//! the game that wrote it); the picture is decoded on a thread of its own.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::SystemTime;

/// A saved game or a replay.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub path: PathBuf,
    /// "Coast To Coast — 2 players", as the game titles it.
    pub title: String,
    /// Game time, in seconds.
    pub length: u64,
    pub when: SystemTime,
    /// The game's autosave (single player, every five minutes).
    pub autosave: bool,
    /// The file layout is readable and, for saves, the simulation revision can resume.
    /// The game still validates actual map/mod compatibility before loading it.
    pub playable: bool,
    /// The mods it was made with, in load order (records from version 46; `None` before, when
    /// they did not say).
    pub mods: Option<Vec<ta_config::records::RecordMod>>,
}

/// What the Play page shows of the player's battles.
#[derive(Default)]
pub struct Battles {
    pub save: Option<Record>,
    pub replay: Option<Record>,
    /// Recent records after the latest one, newest first. Kept small for a quick chooser.
    pub older_saves: Vec<Record>,
    pub older_replays: Vec<Record>,
    pub saves_dir: PathBuf,
    pub replays_dir: PathBuf,
    pub pictures_dir: PathBuf,
    picture: Option<Receiver<Option<(PathBuf, egui::ColorImage)>>>,
    /// The newest picture, and its texture.
    pub backdrop: Option<(PathBuf, egui::TextureHandle)>,
}

/// The picture's longest side once loaded (it fills a banner a window wide).
const PICTURE_SIDE: u32 = 1920;
/// Include the latest record and five alternatives without turning Play into a library.
const RECENT_RECORDS: usize = 6;

impl Battles {
    /// Looks in the settings folder's `saves` and `replays` and in `pictures` (the newest
    /// picture is loaded in the background: [`Battles::poll`]).
    pub fn scan(settings_dir: &Path, pictures: PathBuf) -> Battles {
        let (saves_dir, replays_dir) = (settings_dir.join("saves"), settings_dir.join("replays"));
        let pictures_newest_first = newest_first(&pictures, &["png", "jpg", "jpeg"]);
        let picture = (!pictures_newest_first.is_empty()).then(|| {
            let (tx, rx) = channel();
            let _ = std::thread::Builder::new().name("launcher picture".into()).spawn(move || {
                let picture = pictures_newest_first.into_iter().find_map(|path| load_picture(&path).map(|image| (path, image)));
                let _ = tx.send(picture);
            });
            rx
        });
        let (save, older_saves) = recent_records(&saves_dir, "tasav");
        let (replay, older_replays) = recent_records(&replays_dir, "tarep");
        Battles {
            save,
            replay,
            older_saves,
            older_replays,
            saves_dir,
            replays_dir,
            pictures_dir: pictures,
            picture,
            backdrop: None,
        }
    }

    /// Takes in the picture once it has loaded; true when it did.
    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
        let Some(rx) = &self.picture else { return false };
        match rx.try_recv() {
            Ok(picture) => {
                self.picture = None;
                if let Some((path, image)) = picture {
                    self.backdrop = Some((path, ctx.load_texture("launcher-backdrop", image, egui::TextureOptions::LINEAR)));
                }
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Reduced motion can leave this window fully idle while a large photo
                // decodes. Keep a bounded wakeup until its result can be displayed.
                ctx.request_repaint_after(std::time::Duration::from_millis(120));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.picture = None;
                false
            }
        }
    }

    /// Still loading the picture.
    pub fn loading(&self) -> bool {
        self.picture.is_some()
    }
}

/// Newest files first, with a stable path tie-break when timestamps match.
fn newest_first(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<_> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter(|e| e.path().extension().and_then(|x| x.to_str()).is_some_and(|x| extensions.iter().any(|w| x.eq_ignore_ascii_case(w))))
        .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
        .collect();
    files.sort_unstable_by(|(a, at), (b, bt)| bt.cmp(at).then_with(|| a.cmp(b)));
    files.into_iter().map(|(path, _)| path).collect()
}

fn recent_records(dir: &Path, extension: &str) -> (Option<Record>, Vec<Record>) {
    // Interrupted saves and copied, damaged files must not hide usable battles. Read only
    // each record's tiny metadata tail, and stop once the compact chooser is full.
    let mut records = newest_first(dir, &[extension]).into_iter().filter_map(|path| read_record(&path));
    let newest = records.next();
    (newest, records.take(RECENT_RECORDS - 1).collect())
}

/// A record's title, length and time, as the game wrote it (`replay::Record`, bincode): the
/// version first (a `u32`), and last the game's length (`end_tick`, a `u64`), when it was
/// saved (a `u64`) and its title (a `u64` length and its UTF-8). The title's length is found
/// by trying each one that ends the file. After them, from version 46, the mods it was made with
/// (`ta_config::records::append_mods`), then the trailers of 41 on.
pub fn read_record(path: &Path) -> Option<Record> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    if !(40..=ta_config::records::MAX_RECORD_BYTES).contains(&size) { return None; }
    let mut head = [0u8; 8];
    file.read_exact(&mut head).ok()?;
    let version = u32::from_le_bytes(head[..4].try_into().ok()?);
    if !ta_config::records::record_version_readable(version) { return None; }
    let kind = u32::from_le_bytes(head[4..].try_into().ok()?);
    let extension = path.extension()?.to_str()?;
    let replay = match kind {
        0 if extension.eq_ignore_ascii_case("tarep") => true,
        1 if extension.eq_ignore_ascii_case("tasav") => false,
        _ => return None,
    };
    // (The title and what follows it, with room for a few mods' names; read again, longer, when
    // the mods' trailer is longer than that.)
    const TAIL: u64 = 24 + 4096 + ta_config::profiles::RECORD_PROFILE_TRAILER_LEN as u64 + ta_config::records::RECORD_METADATA_LEN as u64;
    let mut extra = 4096u64;
    let (tail, metadata, mods) = loop {
        let take = (TAIL + extra).min(size);
        file.seek(SeekFrom::Start(size - take)).ok()?;
        let mut bytes = vec![0u8; take as usize];
        file.read_exact(&mut bytes).ok()?;
        if ta_config::profiles::record_metadata(&bytes).is_some() {
            bytes.truncate(bytes.len() - ta_config::profiles::RECORD_PROFILE_TRAILER_LEN);
        }
        let (rest, metadata) = ta_config::records::split_metadata(&bytes).ok()?;
        if (version >= 41) != metadata.is_some() { return None; }
        if !ta_config::records::record_has_mods(version) {
            break (rest.to_vec(), metadata, None);
        }
        let len = ta_config::records::mods_trailer_len(rest)? as u64;
        if (rest.len() as u64) < len + 24 + 4096 && take < size && extra < len {
            extra = len;
            continue;
        }
        let (body, mods) = ta_config::records::split_mods(rest).ok()?;
        break (body.to_vec(), metadata, Some(mods));
    };
    let u64_at = |at: usize| tail.get(at..at + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap_or_default()));
    let n = tail.len();
    let (title, end_tick, saved_at) = (0..=n.saturating_sub(24).min(4096)).find_map(|len| {
        let at = n.checked_sub(len + 8)?;
        (u64_at(at)? == len as u64).then_some(())?;
        let title = std::str::from_utf8(&tail[at + 8..]).ok()?;
        let saved_at = u64_at(at.checked_sub(8)?)?;
        let end_tick = u64_at(at.checked_sub(16)?)?;
        // (A time from 2020 to 2200, and a game shorter than a year, or it is not the title.)
        ((1_577_836_800..7_258_118_400).contains(&saved_at) && end_tick < ta_config::MAX_RECORD_TICKS).then(|| (title.to_string(), end_tick, saved_at))
    })?;
    let stem = path.file_stem().map_or(String::new(), |s| s.to_string_lossy().to_string());
    Some(Record {
        path: path.to_path_buf(),
        title,
        length: end_tick / 30,
        when: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(saved_at),
        autosave: stem.eq_ignore_ascii_case("autosave"),
        playable: replay || ta_config::records::simulation_compatible(version, metadata.as_ref()),
        mods,
    })
}

/// A battle the player asked to continue or watch that was made with other mods than those on:
/// asked about first ([`mod_check`]).
pub struct BattlePrompt {
    pub record: Record,
    /// `--load` or `--replay`.
    pub flag: &'static str,
    pub check: ta_config::records::mods::ModCheck,
}

/// A record's mods against those the game would play with now (the load order of the mods on,
/// as `mods` resolves it) and those installed; `None` for a record that does not name its mods.
/// (The launcher reads no file to tell copies apart: a mod's version tells it here, its files in
/// the game.)
pub fn mod_check(record: &Record, mods: &crate::modlist::ModList) -> Option<ta_config::records::mods::ModCheck> {
    let recorded = record.mods.as_ref()?;
    let as_record = |info: &ta_vfs::mods::ModInfo| ta_config::records::RecordMod { id: info.manifest.id.clone(), name: info.manifest.name.clone(), version: info.manifest.version.clone(), digest: [0; 16] };
    let on: Vec<_> = match &mods.resolution {
        crate::modlist::Resolution::Ok(order) => order.iter().filter_map(|id| mods.get(id)).map(|m| as_record(&m.info)).collect(),
        // (A load order that does not resolve: the game plays without mods.)
        crate::modlist::Resolution::Err(_) => Vec::new(),
    };
    let installed: Vec<_> = mods.mods.iter().map(|m| as_record(&m.info)).collect();
    Some(ta_config::records::mods::check(recorded, &on, &installed))
}

/// Whether to ask before opening: a mod it needs is not as it was, the order is another, or a mod
/// it was made without is on (but a map pack of the Maps page, which changes no battle but one on
/// its maps, and those it names).
pub fn asks(check: &ta_config::records::mods::ModCheck) -> bool {
    !check.matches()
}

fn load_picture(path: &Path) -> Option<egui::ColorImage> {
    // Thumbnails come from user-writable folders. Bound source dimensions and decoded
    // memory before allocating; resizing after an unrestricted decode is too late.
    if std::fs::metadata(path).ok()?.len() > 128 * 1024 * 1024 { return None; }
    let mut reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| log::info!("{}: {e}", path.display())).ok()?;
    let image = if image.width().max(image.height()) > PICTURE_SIDE { image.thumbnail(PICTURE_SIDE, PICTURE_SIDE) } else { image };
    let rgba = image.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

/// How long ago, in words ("just now", "5 min ago", "3 h ago", "2 days ago"), translated.
pub fn age(when: SystemTime) -> String {
    use ta_ui::i18n::{t, tf};
    let secs = SystemTime::now().duration_since(when).map_or(0, |d| d.as_secs());
    match secs {
        0..60 => t("just now").to_string(),
        60..3600 => tf("{n} min ago", &[("n", &(secs / 60))]),
        3600..86_400 => tf("{n} h ago", &[("n", &(secs / 3600))]),
        86_400..172_800 => t("yesterday").to_string(),
        _ => tf("{n} days ago", &[("n", &(secs / 86_400))]),
    }
}

/// A game's length, "mm:ss" or "h:mm:ss".
pub fn length(secs: u64) -> String {
    if secs >= 3600 { format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60) } else { format!("{:02}:{:02}", secs / 60, secs % 60) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_photo_load_repaints_without_animation_and_stops_after_completion() {
        let ctx = egui::Context::default();
        ta_ui::motion::set_reduced(&ctx, true);
        let (send, receive) = channel();
        let mut battles = Battles { picture: Some(receive), ..Default::default() };
        let frame = |battles: &mut Battles| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| { battles.poll(ui.ctx()); });
            output.textures_delta.clear();
            output.viewport_output[&egui::ViewportId::ROOT].repaint_delay
        };
        for _ in 0..4 { frame(&mut battles); }
        assert!(frame(&mut battles) <= std::time::Duration::from_millis(250), "a decode finishing after idle must still become visible");
        send.send(Some((PathBuf::from("photo.png"), egui::ColorImage::filled([1, 1], egui::Color32::BLUE)))).unwrap();
        frame(&mut battles);
        assert!(!battles.loading());
        assert!(battles.backdrop.is_some());
        for _ in 0..4 { frame(&mut battles); }
        assert!(frame(&mut battles) > std::time::Duration::from_secs(1), "idle polling stops once the image arrives");
        let (send, receive) = channel();
        battles.picture = Some(receive); drop(send);
        frame(&mut battles);
        assert!(!battles.loading(), "a failed worker cannot keep the window polling forever");
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ta_launcher_battles_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A record as bincode lays the game's out: version and kind, the setup and the orders
    /// (any bytes here), then the game's length, when it was saved and the title.
    fn record(version: u32, replay: bool, middle: &[u8], end_tick: u64, saved_at: u64, title: &str) -> Vec<u8> {
        let mut out = version.to_le_bytes().to_vec();
        out.extend_from_slice(&u32::from(!replay).to_le_bytes());
        out.extend_from_slice(middle);
        out.extend_from_slice(&end_tick.to_le_bytes());
        out.extend_from_slice(&saved_at.to_le_bytes());
        out.extend_from_slice(&(title.len() as u64).to_le_bytes());
        out.extend_from_slice(title.as_bytes());
        if ta_config::records::record_has_mods(version) {
            ta_config::records::append_mods(&mut out, &[]).unwrap();
        }
        if version >= 41 {
            ta_config::records::append_metadata(&mut out, &ta_config::records::RecordMetadata {
                simulation_version: ta_config::SIMULATION_VERSION, match_data_hash: 123,
                engine_build: "fixture".into(), full_commands: true,
            }).unwrap();
        }
        out
    }

    /// A file too short to be a record (cut off): nothing read, and no sum below zero.
    #[test]
    fn short_files_are_no_records() {
        let dir = temp_dir("short");
        for size in [0, 3, 4, 6, 15, 23, 24, 31] {
            let path = dir.join(format!("short{size}.tasav"));
            std::fs::write(&path, vec![1u8; size]).unwrap();
            assert!(read_record(&path).is_none(), "{size} bytes");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_record_is_read_from_its_ends() {
        let dir = temp_dir("records");
        let saves = dir.join("saves");
        std::fs::create_dir_all(&saves).unwrap();
        // The middle full of numbers that could pass for a title's length.
        let middle: Vec<u8> = (0..3000u32).flat_map(|k| (k as u64 % 40).to_le_bytes()).collect();
        std::fs::write(saves.join("1761000000-Coast_To_Coast.tasav"), record(ta_config::RECORD_VERSION, false, &middle, 30 * 754, 1_761_000_000, "Coast To Coast — 2 players")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(saves.join("autosave.tasav"), record(39, false, &middle, 30 * 61, 1_761_000_500, "Gods of War — 4 players")).unwrap();
        let battles = Battles::scan(&dir, dir.join("pictures"));
        let save = battles.save.clone().expect("the newest save");
        assert_eq!((save.title.as_str(), save.length, save.autosave, save.playable), ("Gods of War — 4 players", 61, true, false));
        let older = read_record(&saves.join("1761000000-Coast_To_Coast.tasav")).unwrap();
        assert_eq!((older.title.as_str(), older.length, older.autosave, older.playable), ("Coast To Coast — 2 players", 754, false, true));
        assert_eq!(older.when, SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_761_000_000));
        assert!(battles.replay.is_none() && !battles.loading(), "no replays, no pictures");
        // Not a record at all.
        std::fs::write(saves.join("junk.tasav"), b"nothing to see here at all").unwrap();
        assert!(read_record(&saves.join("junk.tasav")).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A record of version 46 names its mods in a trailer between its title and the metadata:
    /// read from the file's end, however long the list; one of before names none.
    #[test]
    fn a_records_mods_are_read_from_its_end_however_many() {
        let dir = temp_dir("record_mods");
        let path = dir.join("battle.tasav");
        let middle: Vec<u8> = (0..3000u32).flat_map(|k| (k as u64 % 40).to_le_bytes()).collect();
        let mayhem = ta_config::records::RecordMod { id: "totalmayhem".into(), name: "Total Mayhem".into(), version: "11.3.0".into(), digest: [5; 16] };
        // (Many, with long names: a trailer longer than the end first read.)
        let many: Vec<ta_config::records::RecordMod> = (0..40).map(|i| ta_config::records::RecordMod { id: format!("pack{i}"), name: "Map pack ".repeat(20), version: "1".into(), digest: [i; 16] }).collect();
        for mods in [vec![mayhem], many, Vec::new()] {
            // `record` writes an empty list: this one, with these mods before its metadata.
            let mut bytes = record(45, false, &middle, 30 * 60, 1_761_000_000, "Coast To Coast — 2 players");
            bytes.truncate(bytes.len() - ta_config::records::RECORD_METADATA_LEN);
            bytes[..4].copy_from_slice(&46u32.to_le_bytes());
            ta_config::records::append_mods(&mut bytes, &mods).unwrap();
            ta_config::records::append_metadata(&mut bytes, &ta_config::records::RecordMetadata {
                simulation_version: ta_config::SIMULATION_VERSION, match_data_hash: 1, engine_build: "fixture".into(), full_commands: true,
            }).unwrap();
            ta_config::profiles::append_record_metadata(&mut bytes, &ta_config::profiles::new_id(), false).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            let read = read_record(&path).expect("read with its mods");
            assert_eq!((read.title.as_str(), read.length, read.mods), ("Coast To Coast — 2 players", 60, Some(mods)));
        }
        std::fs::write(&path, record(45, false, &middle, 30, 1_761_000_000, "Before")).unwrap();
        assert_eq!(read_record(&path).unwrap().mods, None, "a record of 45 names none");
        // A record of 46 without the trailer is no record.
        let mut bytes = record(45, false, &middle, 30, 1_761_000_000, "Forged");
        bytes[..4].copy_from_slice(&46u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(read_record(&path).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn optional_career_metadata_keeps_legacy_save_cards_readable() {
        let dir = temp_dir("career_metadata");
        let path = dir.join("battle.tasav");
        let title = "Profile battle — ARM";
        let body = record(ta_config::RECORD_VERSION, false, &[2; 1000], 1800, 1_761_000_000, title);
        std::fs::write(&path, &body).unwrap();
        let legacy = read_record(&path).unwrap();
        let mut bytes = body.clone();
        ta_config::profiles::append_record_metadata(&mut bytes, &ta_config::profiles::new_id(), false).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let current = read_record(&path).unwrap();
        assert_eq!((current.title, current.length, current.when, current.playable), (legacy.title, legacy.length, legacy.when, legacy.playable));
        bytes.push(1);
        std::fs::write(&path, bytes).unwrap();
        assert!(read_record(&path).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn legacy_replays_remain_watchable_without_enabling_unsafe_save_resume() {
        let dir = temp_dir("compatible_versions");
        for version in 36..=ta_config::RECORD_VERSION {
            for replay in [false, true] {
                let path = dir.join(if replay { "battle.tarep" } else { "battle.tasav" });
                std::fs::write(&path, record(version, replay, &[2; 100], 1800, 1_761_000_000, "Battle")).unwrap();
                let found = read_record(&path).unwrap();
                // (Saves resume on the simulation that made them: from 41 the fixture's
                // metadata names the current one; 36 to 40 were simulation revision 40's.)
                assert_eq!(found.playable, replay || version >= 41, "version {version}, replay {replay}");
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn compatibility_tail_kind_and_size_are_checked_before_showing_a_playable_card() {
        let dir = temp_dir("compatibility_tail");
        let path = dir.join("battle.tasav");
        let valid = record(41, false, &[2; 100], 1800, 1_761_000_000, "Battle");
        for removed in [1, ta_config::records::RECORD_METADATA_LEN] {
            std::fs::write(&path, &valid[..valid.len() - removed]).unwrap();
            assert!(read_record(&path).is_none(), "new records require complete compatibility metadata");
        }
        let mut corrupt = valid.clone();
        *corrupt.last_mut().unwrap() = 1;
        std::fs::write(&path, corrupt).unwrap();
        assert!(read_record(&path).is_none());
        for version in [35, ta_config::RECORD_VERSION + 1, u32::MAX] {
            std::fs::write(&path, record(version, false, &[2; 100], 1800, 1_761_000_000, "Unknown layout")).unwrap();
            assert!(read_record(&path).is_none());
        }
        std::fs::write(&path, record(41, true, &[2; 100], 1800, 1_761_000_000, "Renamed replay")).unwrap();
        assert!(read_record(&path).is_none(), "renaming a replay cannot turn it into a save");
        let mut future_simulation = valid.clone();
        let at = future_simulation.len() - ta_config::records::RECORD_METADATA_LEN + 8;
        future_simulation[at..at + 4].copy_from_slice(&(ta_config::SIMULATION_VERSION + 1).to_le_bytes());
        std::fs::write(&path, future_simulation).unwrap();
        assert!(!read_record(&path).unwrap().playable);
        std::fs::write(&path, valid).unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_len(ta_config::records::MAX_RECORD_BYTES + 1).unwrap();
        assert!(read_record(&path).is_none(), "size limits agree with the game reader");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn long_record_titles_still_show_with_both_metadata_trailers() {
        let dir = temp_dir("long_title");
        let path = dir.join("battle.tarep");
        let title = "界".repeat(1365);
        let mut bytes = record(41, true, &[2; 100], 1800, 1_761_000_000, &title);
        ta_config::profiles::append_record_metadata(&mut bytes, &ta_config::profiles::new_id(), false).unwrap();
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(read_record(&path).unwrap().title, title);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_newest_picture_is_loaded_small() {
        let dir = temp_dir("pictures");
        let pictures = dir.join("pictures");
        std::fs::create_dir_all(pictures.join("Wallpapers")).unwrap();
        image::RgbaImage::from_pixel(2400, 1200, image::Rgba([10, 20, 30, 255])).save(pictures.join("Coast To Coast 001.png")).unwrap();
        // (A wallpaper, in its own folder, is not one of the player's pictures.)
        image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 255])).save(pictures.join("Wallpapers").join("Krogoth 4K.png")).unwrap();
        let mut battles = Battles::scan(&dir, pictures.clone());
        let ctx = egui::Context::default();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while battles.loading() && std::time::Instant::now() < until {
            battles.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let (path, texture) = battles.backdrop.expect("loaded");
        assert_eq!(path, pictures.join("Coast To Coast 001.png"));
        assert_eq!(texture.size(), [1920, 960]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_damaged_newest_record_does_not_hide_the_previous_battle() {
        let dir = temp_dir("damaged_newest");
        for (folder, extension) in [("saves", "tasav"), ("replays", "tarep")] {
            let folder = dir.join(folder);
            std::fs::create_dir(&folder).unwrap();
            let valid = folder.join(format!("valid.{extension}"));
            std::fs::write(&valid, record(ta_config::RECORD_VERSION, extension == "tarep", &[2; 100], 1800, 1_761_000_000, "My battle")).unwrap();
            std::fs::File::options().write(true).open(&valid).unwrap().set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_761_000_000)).unwrap();
            std::fs::write(folder.join(format!("broken.{extension}")), b"interrupted copy").unwrap();
        }
        let battles = Battles::scan(&dir, dir.join("pictures"));
        assert_eq!(battles.save.unwrap().title, "My battle");
        assert_eq!(battles.replay.unwrap().title, "My battle");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pictures_are_bounded_before_decode_and_bad_newest_photos_are_skipped() {
        let dir = temp_dir("bounded_pictures");
        let valid = dir.join("photo.png");
        image::RgbaImage::from_pixel(8, 4, image::Rgba([10, 20, 30, 255])).save(&valid).unwrap();
        std::fs::File::options().write(true).open(&valid).unwrap().set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_761_000_000)).unwrap();
        let excessive = dir.join("oversize.png");
        image::GrayImage::new(16_385, 1).save(&excessive).unwrap();
        assert!(load_picture(&excessive).is_none(), "dimension limit applies before decoding/resizing");
        std::fs::write(dir.join("broken.png"), b"not an image").unwrap();
        let mut battles = Battles::scan(&dir, dir.clone());
        let ctx = egui::Context::default();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while battles.loading() && std::time::Instant::now() < until {
            battles.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let (path, texture) = battles.backdrop.expect("previous valid photo remains available");
        assert_eq!(path, valid);
        assert_eq!(texture.size(), [8, 4]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn recent_history_keeps_six_valid_records_in_newest_order() {
        let dir = temp_dir("recent_history");
        for (folder, extension) in [("saves", "tasav"), ("replays", "tarep")] {
            let folder = dir.join(folder);
            std::fs::create_dir(&folder).unwrap();
            for index in 0..12 {
                let path = folder.join(format!("battle{index}.{extension}"));
                let time = 1_761_000_000 + index;
                let version = if index % 2 == 0 { ta_config::RECORD_VERSION } else { 39 };
                std::fs::write(&path, record(version, extension == "tarep", &[2; 100], 1800, time, &format!("Battle {index}"))).unwrap();
                std::fs::File::options().write(true).open(path).unwrap()
                    .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(time)).unwrap();
            }
            let broken = folder.join(format!("interrupted.{extension}"));
            std::fs::write(&broken, b"interrupted copy").unwrap();
            std::fs::File::options().write(true).open(broken).unwrap()
                .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_761_000_999)).unwrap();
            let (latest, older) = recent_records(&folder, extension);
            let latest = latest.unwrap();
            assert_eq!(latest.title, "Battle 11");
            assert_eq!(latest.playable, extension == "tarep", "old replays can be watched; old saves stay visible without unsafe resume");
            assert_eq!(older.len(), RECENT_RECORDS - 1);
            assert_eq!(older.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["Battle 10", "Battle 9", "Battle 8", "Battle 7", "Battle 6"]);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn lengths_read_well() {
        assert_eq!(length(754), "12:34");
        assert_eq!(length(3725), "1:02:05");
    }
}
