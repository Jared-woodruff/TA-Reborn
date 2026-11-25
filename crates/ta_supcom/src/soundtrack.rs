//! Forged Alliance's soundtrack, made from the player's own copy into TA Reborn's music
//! (docs/SUPCOM_MUSIC.md): the menus' theme and the building and battle themes the game plays as
//! a battle goes (its `Music` bank's cues `Main_Menu`, `Base_Building` and `Battle`), the
//! campaign's briefings' music (`Op_Briefing`) and its films' (`FMV_BG`: the intro, the outro and
//! what follows it, the timeline, the credits; not the publishers' jingles). Each track is a FLAC
//! file of the music mod (`music/supcom/<stem>.flac`), lossless, listed with its part in
//! [`LIST`] for the game's playlists and jukebox.

use std::path::Path;

use crate::xact::{Codec, SoundBank, WaveBank};
use crate::{Error, Result};

/// The mod's id, name, words and tags as its manifest keeps them, in English.
pub const MOD_ID: &str = "supcom-music";
pub const MOD_NAME: &str = "Supreme Commander music";
pub const DESCRIPTION: &str = "Supreme Commander: Forged Alliance's soundtrack, made from your own copy of the game.";
pub const TAGS: [&str; 2] = ["music", "supreme commander"];
/// The converter's version, the mod's own (its manifest's `version`): a mod made by an older one
/// is offered to be made again. Raised whenever what is made changes.
pub const VERSION: u32 = 1;
/// The list of the tracks made, each with its part and number, as the game reads it.
pub const LIST: &str = "music/supcom/soundtrack.toml";

/// What a track is to the soundtrack: the game plays the menu's theme in its menus, the building
/// and battle themes as a battle goes, the briefings' over the campaign's; the films' are music
/// to listen to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    Menu,
    Building,
    Battle,
    Briefing,
    Film,
}

impl Part {
    pub const ALL: [Part; 5] = [Part::Menu, Part::Building, Part::Battle, Part::Briefing, Part::Film];

    /// Its key in [`LIST`].
    pub fn key(self) -> &'static str {
        match self {
            Part::Menu => "menu",
            Part::Building => "building",
            Part::Battle => "battle",
            Part::Briefing => "briefing",
            Part::Film => "film",
        }
    }

    pub fn from_key(key: &str) -> Option<Part> {
        Part::ALL.into_iter().find(|p| p.key().eq_ignore_ascii_case(key.trim()))
    }
}

/// The films' music, by their cues in `FMV_BG`, in the order the films come, each with the name
/// its file goes by.
pub const FILMS: [(&str, &str); 5] = [("X_FMV_Intro", "intro"), ("X_FMV_Outro", "outro"), ("X_FMV_Post_Outro", "epilogue"), ("X_Timeline", "timeline"), ("Menu_Credits", "credits")];

/// A track of the soundtrack: its part and its number in it (from 1), the name of its file in
/// the mod, where it comes from (its wave bank's file in `sounds`, the wave's index), how long it
/// plays and what its sound takes uncompressed.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub part: Part,
    pub number: usize,
    pub stem: String,
    pub bank: &'static str,
    pub wave: usize,
    pub seconds: f64,
    pub bytes: u64,
}

impl Track {
    /// Its file in the mod.
    pub fn file(&self) -> String {
        format!("music/supcom/{}.flac", self.stem)
    }
}

/// A file of `sounds` (a game's `sounds` folder), whatever its case.
pub(crate) fn sound_file(sounds: &Path, name: &str) -> Result<std::path::PathBuf> {
    ta_vfs::install::find_file_ci(sounds, name).ok_or_else(|| Error::Missing(format!("sounds/{name}")))
}

/// A wave bank's header and entries: read from its start alone, not its waves' data.
pub(crate) fn bank_head(sounds: &Path, name: &str) -> Result<WaveBank> {
    use std::io::Read;
    let path = sound_file(sounds, name)?;
    let mut head = Vec::new();
    std::fs::File::open(&path).and_then(|f| f.take(1 << 16).read_to_end(&mut head)).map_err(|e| Error::io(&path, e))?;
    WaveBank::read(&head).map_err(|e| Error::Format(format!("{}: {e}", path.display())))
}

/// A sound bank of `sounds`, read whole.
pub(crate) fn sound_bank(sounds: &Path, name: &str) -> Result<SoundBank> {
    let path = sound_file(sounds, name)?;
    let bytes = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
    SoundBank::read(&bytes).map_err(|e| Error::Format(format!("{}: {e}", path.display())))
}

/// The soundtrack of the copy whose `sounds` folder is `sounds`: each track by the cue that plays
/// it, the menus', the building and battle themes, the briefings', the films'. Waves of codecs
/// other than PCM, and cues the copy does not have, are left out.
pub fn tracks(sounds: &Path) -> Result<Vec<Track>> {
    let mut out: Vec<Track> = Vec::new();
    let mut take = |bank_file: &'static str, bank: &WaveBank, part: Part, stem: String, wave: u16| {
        let Some(w) = bank.waves.get(wave as usize).filter(|w| w.codec == Codec::Pcm && w.channels > 0) else { return };
        let number = out.iter().filter(|t| t.part == part).count() + 1;
        out.push(Track { part, number, stem, bank: bank_file, wave: wave as usize, seconds: w.seconds(), bytes: w.length as u64 });
    };
    // The game's music: the menus', then the building and battle themes in the cues' order.
    let music = bank_head(sounds, "Music.xwb")?;
    let cues = sound_bank(sounds, "Music.xsb")?;
    for (cue, part, name) in [("Main_Menu", Part::Menu, "menu"), ("Base_Building", Part::Building, "building"), ("Battle", Part::Battle, "battle")] {
        let waves: Vec<u16> = cues.cue(cue).map(|c| c.waves.iter().map(|w| w.1).collect()).unwrap_or_default();
        let one = waves.len() == 1;
        for (i, wave) in waves.into_iter().enumerate() {
            let stem = if one { name.to_string() } else { format!("{name}_{}", i + 1) };
            take("Music.xwb", &music, part, stem, wave);
        }
    }
    // The briefings' music, an operation each (where the copy has it).
    if let (Ok(bank), Ok(cues)) = (bank_head(sounds, "Op_Briefing.xwb"), sound_bank(sounds, "Op_Briefing.xsb")) {
        let mut waves: Vec<(String, u16)> = cues.cues.iter().filter_map(|c| Some((c.name.clone(), c.waves.first()?.1))).collect();
        waves.sort();
        for (i, (_, wave)) in waves.into_iter().enumerate() {
            take("Op_Briefing.xwb", &bank, Part::Briefing, format!("briefing_{}", i + 1), wave);
        }
    }
    // The films' music, in their order.
    if let (Ok(bank), Ok(cues)) = (bank_head(sounds, "FMV_BG.xwb"), sound_bank(sounds, "FMV_BG.xsb")) {
        for (cue, name) in FILMS {
            if let Some(&(_, wave)) = cues.cue(cue).and_then(|c| c.waves.first()) {
                take("FMV_BG.xwb", &bank, Part::Film, format!("film_{name}"), wave);
            }
        }
    }
    if out.is_empty() {
        return Err(Error::Missing("the game's music (sounds/Music.xwb)".into()));
    }
    Ok(out)
}

/// The data of wave `index` of the wave bank `name` in `sounds` (`bank`, its head), read from
/// its place in the file alone.
pub(crate) fn wave_data(sounds: &Path, name: &str, bank: &WaveBank, index: usize) -> Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let wave = bank.waves.get(index).ok_or_else(|| Error::Format(format!("{name}: no wave {index}")))?;
    let path = sound_file(sounds, name)?;
    let mut file = std::fs::File::open(&path).map_err(|e| Error::io(&path, e))?;
    file.seek(SeekFrom::Start(wave.offset as u64)).map_err(|e| Error::io(&path, e))?;
    let mut data = vec![0u8; wave.length];
    file.read_exact(&mut data).map_err(|e| Error::io(&path, e))?;
    Ok(data)
}

/// What was made.
#[derive(Clone, Debug, Default)]
pub struct Made {
    pub tracks: usize,
    pub files: usize,
    pub bytes: u64,
    /// Tracks left out, and why.
    pub skipped: Vec<(String, String)>,
    /// Stopped before every track was made (the watch said so): the files written are no whole mod.
    pub stopped: bool,
}

/// The mod's manifest.
pub fn manifest(reborn: &str) -> String {
    let tags: Vec<String> = TAGS.iter().map(|t| format!("\"{t}\"")).collect();
    format!(
        "id = \"{MOD_ID}\"\nname = \"{MOD_NAME}\"\nversion = \"{VERSION}\"\nrequires_reborn = \"{reborn}\"\ndescription = \"{DESCRIPTION}\"\ntags = [{}]\n",
        tags.join(", ")
    )
}

/// The list of `tracks` made, as the game reads it ([`LIST`]).
pub fn list(tracks: &[Track]) -> String {
    let mut s = String::from("# Supreme Commander: Forged Alliance's soundtrack, made from the player's own copy: each track's\n# file, its part (menu, building, battle, briefing, film) and its number in it.\n");
    for t in tracks {
        s.push_str(&format!("\n[[track]]\nfile = \"{}\"\npart = \"{}\"\nnumber = {}\nseconds = {:.1}\n", t.file(), t.part.key(), t.number, t.seconds));
    }
    s
}

/// A wave's PCM as FLAC: its samples (16 bits, little-endian, or 8 bits, unsigned), interleaved.
pub fn flac(pcm: &[u8], channels: u16, rate: u32, bits: u16) -> Result<Vec<u8>> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let samples: Vec<i32> = if bits == 16 {
        pcm.chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]]) as i32).collect()
    } else {
        pcm.iter().map(|&s| s as i32 - 128).collect()
    };
    let config = flacenc::config::Encoder::default().into_verified().map_err(|(_, e)| Error::Format(format!("FLAC settings: {e:?}")))?;
    let block = config.block_size;
    let source = flacenc::source::MemSource::from_samples(&samples, channels as usize, bits as usize, rate as usize);
    let mut stream = flacenc::encode_with_fixed_block_size(&config, source, block).map_err(|e| Error::Format(format!("FLAC: {e:?}")))?;
    // (Its blocks are all `block` long but the last: the stream's information says so, as FLAC
    // has it, the last block left out of its least size. The encoder's threads give the last
    // block's length there, and a decoder then takes the blocks to vary and reads none: the
    // game's own, symphonia, as an end of the stream.)
    stream.stream_info_mut().set_block_sizes(block, block).map_err(|e| Error::Format(format!("FLAC: {e:?}")))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| Error::Format(format!("FLAC: {e:?}")))?;
    Ok(sink.as_slice().to_vec())
}

/// The menu theme's loudness in [`banner::BARS`](crate::banner::BARS) stretches, as the music's
/// banner shows it, read from the copy whose `sounds` folder is `sounds`: none where it has no
/// menu theme.
pub fn menu_loudness(sounds: &Path) -> Result<Option<Vec<u16>>> {
    let Some(track) = tracks(sounds)?.into_iter().find(|t| t.part == Part::Menu) else { return Ok(None) };
    let bank = bank_head(sounds, track.bank)?;
    let pcm = wave_data(sounds, track.bank, &bank, track.wave)?;
    let wave = &bank.waves[track.wave];
    Ok(Some(crate::banner::loudness(&pcm, wave.bits, wave.channels, wave.rate, crate::banner::BARS)))
}

/// Makes the music mod of the copy whose `sounds` folder is `sounds`, for TA Reborn `reborn` and
/// newer: each track a FLAC file, the list, the manifest, and the banner the launcher shows it by
/// (the menu theme's loudness over a painting of the game's, from its `files` where given),
/// written through `write` (a path in the mod, its bytes). `watch` is told before each track how
/// many are made and the track's file name: false from it stops the making there
/// ([`Made::stopped`]).
pub fn make(sounds: &Path, files: Option<&crate::files::Files>, reborn: &str, write: &mut dyn FnMut(&str, &[u8]) -> Result<()>, watch: &mut dyn FnMut(usize, &str) -> bool) -> Result<Made> {
    let tracks = tracks(sounds)?;
    let mut made = Made::default();
    let mut put = |path: &str, bytes: &[u8], made: &mut Made| -> Result<()> {
        made.files += 1;
        made.bytes += bytes.len() as u64;
        write(path, bytes)
    };
    put("mod.toml", (manifest(reborn) + &crate::banner::manifest_lines()).as_bytes(), &mut made)?;
    let mut done: Vec<Track> = Vec::new();
    // (The menu theme's loudness, for the banner, taken as its track is made.)
    let mut loudness: Vec<u16> = Vec::new();
    // (A bank's file read once for its tracks, the tracks in its order.)
    let mut held: Option<(&str, Vec<u8>, WaveBank)> = None;
    for (i, track) in tracks.iter().enumerate() {
        if !watch(i, &track.stem) {
            made.stopped = true;
            made.tracks = done.len();
            return Ok(made);
        }
        if held.as_ref().is_none_or(|(name, _, _)| *name != track.bank) {
            let path = sound_file(sounds, track.bank)?;
            let bytes = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
            let bank = WaveBank::read(&bytes).map_err(|e| Error::Format(format!("{}: {e}", path.display())))?;
            held = Some((track.bank, bytes, bank));
        }
        let (_, bytes, bank) = held.as_ref().expect("the track's bank, read");
        let wave = &bank.waves[track.wave];
        if track.part == Part::Menu && loudness.is_empty()
            && let Some(pcm) = bank.data(bytes, track.wave)
        {
            loudness = crate::banner::loudness(pcm, wave.bits, wave.channels, wave.rate, crate::banner::BARS);
        }
        let made_flac = bank.data(bytes, track.wave).ok_or_else(|| Error::Format(format!("{}: wave {} has no data", track.bank, track.wave))).and_then(|pcm| flac(pcm, wave.channels, wave.rate, wave.bits));
        match made_flac {
            Ok(flac) => {
                put(&track.file(), &flac, &mut made)?;
                done.push(track.clone());
            }
            Err(why) => made.skipped.push((track.stem.clone(), why.to_string())),
        }
    }
    put(LIST, list(&done).as_bytes(), &mut made)?;
    put(crate::banner::PATH, &crate::banner::music(files, &loudness), &mut made)?;
    made.tracks = done.len();
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xact::tests::{sound_bank, wave_bank};

    /// A copy's `sounds` folder: the game's music (two building themes, one battle theme, the
    /// menu's) and the films' (the intro's), in banks as XACT writes them.
    fn copy(dir: &Path) {
        let tone = |n: usize, f: i32| (0..n).map(|i| ((i as i32 * f) % 2000 - 1000) as i16).collect::<Vec<i16>>();
        let music = wave_bank("Music", &[(44_100, 2, tone(800, 7)), (44_100, 2, tone(600, 11)), (44_100, 2, tone(400, 13)), (32_000, 1, tone(300, 17))]);
        std::fs::write(dir.join("Music.xwb"), music).unwrap();
        std::fs::write(dir.join("Music.xsb"), sound_bank(&[("Main_Menu", vec![3], true), ("Base_Building", vec![1, 0], true), ("Battle", vec![2], true)])).unwrap();
        std::fs::write(dir.join("FMV_BG.xwb"), wave_bank("FMV_BG", &[(44_100, 2, tone(200, 3)), (44_100, 2, tone(200, 5))])).unwrap();
        std::fs::write(dir.join("FMV_BG.xsb"), sound_bank(&[("THQ_Logo", vec![0], false), ("X_FMV_Intro", vec![1], false)])).unwrap();
    }

    /// The tracks by their cues: the menu's theme, the building themes in the cue's order, the
    /// battle's, and of the films' the intro's (not a publisher's jingle); each with its length.
    #[test]
    fn the_soundtrack_is_its_cues_tracks() {
        let dir = crate::tests::Temp::new("soundtrack");
        copy(&dir.0);
        let tracks = tracks(&dir.0).unwrap();
        let named: Vec<(Part, usize, &str, usize)> = tracks.iter().map(|t| (t.part, t.number, t.stem.as_str(), t.wave)).collect();
        assert_eq!(named, [
            (Part::Menu, 1, "menu", 3),
            (Part::Building, 1, "building_1", 1),
            (Part::Building, 2, "building_2", 0),
            (Part::Battle, 1, "battle", 2),
            (Part::Film, 1, "film_intro", 1),
        ]);
        assert!((tracks[0].seconds - 300.0 / 32_000.0).abs() < 1e-9);
        assert_eq!(tracks[1].file(), "music/supcom/building_1.flac");
    }

    /// The mod made: its manifest naming its banner, a FLAC file a track, the list naming each
    /// with its part, and the banner (a PNG, made without the game's painting where its files are
    /// not given); stopped, no list.
    #[test]
    fn the_music_mod_is_made_a_flac_file_a_track() {
        let dir = crate::tests::Temp::new("soundtrack_make");
        copy(&dir.0);
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        let made = make(&dir.0, None, "1.0.9", &mut |path, bytes| {
            files.push((path.to_string(), bytes.to_vec()));
            Ok(())
        }, &mut |_, _| true).unwrap();
        assert_eq!((made.tracks, made.skipped.len(), made.stopped), (5, 0, false));
        let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, ["mod.toml", "music/supcom/menu.flac", "music/supcom/building_1.flac", "music/supcom/building_2.flac", "music/supcom/battle.flac", "music/supcom/film_intro.flac", LIST, crate::banner::PATH]);
        let manifest = String::from_utf8(files[0].1.clone()).unwrap();
        assert!(manifest.ends_with("\n[media]\nbanner = \"media/banner.png\"\n"), "{manifest}");
        let flac = &files[1].1;
        assert_eq!(&flac[..4], b"fLaC");
        // (Its least block size its most, the last block aside, as FLAC has it: else the game's
        // decoder takes the blocks to vary and reads none.)
        assert_eq!(flac[8..10], flac[10..12], "fixed blocks: {:02x?}", &flac[..12]);
        let list = String::from_utf8(files[files.len() - 2].1.clone()).unwrap();
        assert!(list.contains("file = \"music/supcom/battle.flac\"\npart = \"battle\"\nnumber = 1"), "{list}");
        assert_eq!(&files.last().unwrap().1[..8], b"\x89PNG\r\n\x1a\n");
        let mut seen = 0;
        let stopped = make(&dir.0, None, "1.0.9", &mut |_, _| Ok(()), &mut |i, _| {
            seen = i;
            i < 2
        }).unwrap();
        assert!(stopped.stopped && stopped.tracks == 2 && seen == 2);
    }

    #[test]
    fn parts_go_by_their_keys() {
        for part in Part::ALL {
            assert_eq!(Part::from_key(part.key()), Some(part));
        }
        assert_eq!(Part::from_key(" Battle "), Some(Part::Battle));
        assert_eq!(Part::from_key("victory"), None);
    }
}
