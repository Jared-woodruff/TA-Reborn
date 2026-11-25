//! Forged Alliance's look for TA Reborn's menus, made from the player's own copy
//! (docs/SUPCOM_MENUS.md): its interface's colours (`lua/skins/skins.lua`, the `default` skin),
//! its menus' pictures (`menus02`'s backgrounds and paintings, the factions' hall), its panels'
//! border art (`scx_menu/panel-brd`), its interface sounds (the `Interface` bank's menu cues) and
//! its menu music (the `Music` bank's `Main_Menu`), into the mod `supcom-menus`, with [`THEME`]
//! saying what each is for. The game's MENU THEME (Options, INTERFACE) plays it; the layouts and
//! the widgets' shapes stay TA Reborn's.

use std::path::Path;

use crate::files::Files;
use crate::soundtrack::{bank_head, sound_bank, wave_data};
use crate::xact::{Codec, Wave};
use crate::{Error, Result, lua};

/// The mod's id, name, words and tags as its manifest keeps them, in English.
pub const MOD_ID: &str = "supcom-menus";
pub const MOD_NAME: &str = "Supreme Commander menus";
pub const DESCRIPTION: &str = "Supreme Commander: Forged Alliance's look for TA Reborn's menus: its colours, pictures, panel art, interface sounds and menu music, made from your own copy of the game.";
pub const TAGS: [&str; 3] = ["interface", "theme", "supreme commander"];
/// The converter's version, the mod's own (its manifest's `version`): a mod made by an older one
/// is offered to be made again. Raised whenever what is made changes. (2: the launcher's install of
/// version 1 left its `menus/` folder out, its theme with it: made again, it is whole; 3: the
/// colossus's picture left out, letterboxed, and the banner the launcher shows the mod by.)
pub const VERSION: u32 = 3;
/// What the mod's files are for, as the game reads it.
pub const THEME: &str = "menus/supcom/theme.toml";
/// The menu music's file in the mod.
pub const MUSIC: &str = "music/supcom-menus/menu.flac";

/// The menus' pictures: each one's file in the game's archives and its name in the mod. (The game
/// shows them behind its menus one after another. Not the colossus's, `background-colossus_bmp`:
/// letterboxed in black bars of its own, framed, with a wordmark, and `painting_1` again.)
pub const PICTURES: [(&str, &str); 11] = [
    ("textures/ui/common/menus02/background_bmp.dds", "space_1"),
    ("textures/ui/common/menus02/background02_bmp.dds", "space_2"),
    ("textures/ui/common/menus02/background03_bmp.dds", "space_3"),
    ("textures/ui/common/menus02/background04_bmp.dds", "space_4"),
    ("textures/ui/common/menus02/background05_bmp.dds", "space_5"),
    ("textures/ui/common/menus02/background-paint01_bmp.dds", "painting_1"),
    ("textures/ui/common/menus02/background-paint02_bmp.dds", "painting_2"),
    ("textures/ui/common/menus02/background-paint03_bmp.dds", "painting_3"),
    ("textures/ui/common/menus02/background-paint04_bmp.dds", "painting_4"),
    ("textures/ui/common/menus02/background-paint05_bmp.dds", "painting_5"),
    ("textures/ui/common/scx_menu/campaign-select/bg.dds", "factions"),
];

/// Where the panel's border art is in the game's archives.
const PANEL_DIR: &str = "textures/ui/common/scx_menu/panel-brd/";

/// The panel's border, nine pieces: each one's file in [`PANEL_DIR`] and its place, as the theme
/// names it (the corners, the edges between them, the middle).
pub const PANEL: [(&str, &str); 9] = [
    ("panel_brd_ul", "top_left"),
    ("panel_brd_horz_um", "top"),
    ("panel_brd_ur", "top_right"),
    ("panel_brd_vert_l", "left"),
    ("panel_brd_m", "middle"),
    ("panel_brd_vert_r", "right"),
    ("panel_brd_ll", "bottom_left"),
    ("panel_brd_lm", "bottom"),
    ("panel_brd_lr", "bottom_right"),
];

/// The interface's sounds: TA Reborn's interface event (as `ta_ui::sound::UiSound::event` names
/// it, TA's `allsound.tdf`'s) and the cue of Forged Alliance's `Interface` bank played for it.
pub const SOUNDS: [(&str, &str); 15] = [
    ("BIGBUTTON", "UI_Menu_MouseDown"),
    ("SMLBUTTON", "UI_Menu_MouseDown_Sml"),
    ("SMALLBUTTON", "UI_Opt_Mini_Button_Click"),
    ("PREVIOUS", "UI_Back_MouseDown"),
    ("MORE", "UI_Tab_Click_01"),
    ("SKIRMISH", "UI_Menu_MouseDown"),
    ("MULTI", "UI_Menu_MouseDown"),
    ("OPTIONS", "UI_Menu_MouseDown"),
    ("EXIT", "UI_Back_MouseDown"),
    ("MessageArrived", "UI_Announcement_Open"),
    ("ENDGAMESCORE", "UI_Objective_Checked"),
    ("SLIDER", "UI_Opt_Mini_Button_Over"),
    ("PANEL", "UI_Main_Window_Open"),
    ("SKIRMISHCHEAT", "UI_Menu_Error_01"),
    ("ENDGAMESTATBAR", "UI_Menu_Accept_01"),
];

/// The skin whose colours the menus take: the game's own default (the UEF's are the same; the
/// other factions' colour only their own screens).
const SKIN: &str = "default";

/// Forged Alliance's interface colours, as its skin names them (`AARRGGBB`), read from the copy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Skin {
    pub colours: Vec<(String, [u8; 4])>,
}

impl Skin {
    /// The colour the skin calls `key` (any case): red, green, blue, alpha.
    pub fn colour(&self, key: &str) -> Option<[u8; 4]> {
        self.colours.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, c)| *c)
    }

    /// TA Reborn's palette in these colours: its keys (as `ta_ui::theme::Palette::themed` takes
    /// them) and each one's colour, those the skin lacks left out. Forged Alliance letters its
    /// buttons and captions in `fontColor` (a pale cyan), its body and column text in `bodyColor`
    /// (a teal), lights what the pointer is on in `fontOverColor` and `highlightColor`, greys what
    /// cannot be used in `disabledColor` and lays its panels in `panelColor` at
    /// `transparentPanelColor`'s alpha over the dark behind them; its dialogs' buttons are
    /// lettered in `dialogButtonColor` (a green).
    pub fn palette(&self) -> Vec<(&'static str, [u8; 3])> {
        let rgb = |c: [u8; 4]| [c[0], c[1], c[2]];
        let mut out = Vec::new();
        let mut put = |key: &'static str, colour: Option<[u8; 3]>| {
            if let Some(c) = colour {
                out.push((key, c));
            }
        };
        let font = self.colour("fontColor").map(rgb);
        put("text", font);
        put("text_bright", self.colour("fontOverColor").map(rgb));
        put("button_text", font);
        put("dim", self.colour("bodyColor").map(rgb));
        // (The accent between the highlight and the lettering: lighter than the one, distinct
        // from the other.)
        put("accent", match (self.colour("highlightColor").map(rgb), font) {
            (Some(h), Some(f)) => Some([0, 1, 2].map(|i| ((h[i] as u16 + f[i] as u16) / 2) as u8)),
            (h, f) => h.or(f),
        });
        // (The panel as it shows over the dark: its colour at its see-through alpha over black.)
        let alpha = self.colour("transparentPanelColor").map_or(255, |c| c[3]) as u16;
        put("panel", self.colour("panelColor").map(|c| [0, 1, 2].map(|i| (c[i] as u16 * alpha / 255) as u8)));
        put("line", self.colour("disabledColor").map(rgb));
        put("good", self.colour("dialogButtonColor").map(rgb));
        put("info", self.colour("tooltipBorderColor").or(self.colour("highlightColor")).map(rgb));
        out
    }
}

/// An `AARRGGBB` colour as the skins write them (`"FFbadbdb"`), or `RRGGBB`.
fn hex_colour(text: &str) -> Option<[u8; 4]> {
    let t = text.trim().trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(t.get(i..i + 2)?, 16).ok();
    match t.len() {
        8 => Some([byte(2)?, byte(4)?, byte(6)?, byte(0)?]),
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        _ => None,
    }
}

/// The interface's colours of the copy whose files are `files`: its default skin's.
pub fn skin(files: &Files) -> Result<Skin> {
    let bytes = files.read("lua/skins/skins.lua")?.ok_or_else(|| Error::Missing("lua/skins/skins.lua".into()))?;
    let text = String::from_utf8_lossy(&bytes);
    let statements = lua::parse(&text)?;
    let skins = lua::top(&statements, "skins").and_then(lua::Value::table).ok_or_else(|| Error::Format("lua/skins/skins.lua: no skins".into()))?;
    let default = skins.get_ci(SKIN).and_then(lua::Value::table).ok_or_else(|| Error::Format(format!("lua/skins/skins.lua: no {SKIN} skin")))?;
    let colours = default.fields.iter().filter(|(k, _)| k.to_ascii_lowercase().ends_with("color")).filter_map(|(k, v)| Some((k.clone(), hex_colour(v.str()?)?))).collect();
    Ok(Skin { colours })
}

/// What the copy has of the theme: its colours, which pictures, whether its panel art, which
/// sounds (by their cues), and its menu music (how long it plays).
#[derive(Clone, Debug, Default)]
pub struct Contents {
    pub skin: Skin,
    /// The pictures there are, by their names in the mod.
    pub pictures: Vec<&'static str>,
    pub panel: bool,
    pub sounds: Vec<&'static str>,
    pub music_seconds: Option<f64>,
}

/// The wave the cue `cue` of the sound bank `bank` (in `sounds`) plays first, of the bank's own
/// wave bank (`bank.xwb`), where it is PCM.
fn cue_wave(sounds: &Path, bank: &str, cue: &str) -> Result<Option<(crate::xact::WaveBank, usize)>> {
    let cues = sound_bank(sounds, &format!("{bank}.xsb"))?;
    let waves = bank_head(sounds, &format!("{bank}.xwb"))?;
    let Some(&(_, wave)) = cues.cue(cue).and_then(|c| c.waves.first()) else { return Ok(None) };
    let ok = waves.waves.get(wave as usize).is_some_and(|w| w.codec == Codec::Pcm && w.channels > 0);
    Ok(ok.then_some((waves, wave as usize)))
}

/// What of the theme the copy has, its `files` and its `sounds` folder (none: no sounds, no
/// music).
pub fn contents(files: &Files, sounds: Option<&Path>) -> Result<Contents> {
    let skin = skin(files)?;
    // (What the archives hold, listed once: not read.)
    let listed = files.list("textures/ui/common/");
    let has = |path: &str| listed.binary_search_by(|p| p.as_str().cmp(path)).is_ok();
    let pictures = PICTURES.iter().filter(|(path, _)| has(path)).map(|(_, name)| *name).collect();
    let panel = PANEL.iter().all(|(file, _)| has(&format!("{PANEL_DIR}{file}.dds")));
    let mut found = Contents { skin, pictures, panel, ..Default::default() };
    if let Some(sounds) = sounds {
        let interface = sound_bank(sounds, "Interface.xsb").ok();
        let mut cues: Vec<&'static str> = SOUNDS.iter().map(|(_, cue)| *cue).filter(|cue| interface.as_ref().is_some_and(|b| b.cue(cue).is_some())).collect();
        cues.sort_unstable();
        cues.dedup();
        found.sounds = cues;
        found.music_seconds = cue_wave(sounds, "Music", "Main_Menu").ok().flatten().map(|(bank, wave)| bank.waves[wave].seconds());
    }
    Ok(found)
}

/// A picture of the theme, decoded (its first level): for showing what an import makes.
pub fn picture(files: &Files, name: &str) -> Result<Option<crate::dds::Image>> {
    let Some((path, _)) = PICTURES.iter().find(|(_, n)| *n == name) else { return Ok(None) };
    match files.read(path)? {
        Some(bytes) => Ok(Some(crate::dds::decode(&bytes, 0)?)),
        None => Ok(None),
    }
}

/// A wave's PCM as a WAV file (`RIFF`, PCM, its samples as they are).
pub fn wav(pcm: &[u8], wave: &Wave) -> Vec<u8> {
    let (channels, rate, bits) = (wave.channels as u32, wave.rate, wave.bits as u32);
    let block = channels * bits / 8;
    let mut b = Vec::with_capacity(44 + pcm.len());
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&(channels as u16).to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * block).to_le_bytes());
    b.extend_from_slice(&(block as u16).to_le_bytes());
    b.extend_from_slice(&(bits as u16).to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    b.extend_from_slice(pcm);
    b
}

/// A sound's file in the mod, by its cue.
pub fn sound_path(cue: &str) -> String {
    format!("sounds/supcom-menus/{}.wav", cue.to_ascii_lowercase())
}

/// The mod's manifest.
pub fn manifest(reborn: &str) -> String {
    let tags: Vec<String> = TAGS.iter().map(|t| format!("\"{t}\"")).collect();
    format!(
        "id = \"{MOD_ID}\"\nname = \"{MOD_NAME}\"\nversion = \"{VERSION}\"\nrequires_reborn = \"{reborn}\"\ndescription = \"{DESCRIPTION}\"\ntags = [{}]\n",
        tags.join(", ")
    )
}

/// What was made.
#[derive(Clone, Debug, Default)]
pub struct Made {
    pub pictures: usize,
    pub sounds: usize,
    pub music: bool,
    pub panel: bool,
    pub files: usize,
    pub bytes: u64,
    /// What was left out (a picture, a sound...), and why.
    pub skipped: Vec<(String, String)>,
    /// Stopped before all was made (the watch said so): the files written are no whole mod.
    pub stopped: bool,
}

/// The theme's list: what each of the mod's files is for (`pictures`, the `music`, the
/// `[colours]`, the `[panel]`'s pieces, the `[sounds]` by TA Reborn's interface events).
pub fn theme(skin: &Skin, pictures: &[&str], panel: bool, sounds: &[(&str, String)], music: bool) -> String {
    let mut s = String::from("# Supreme Commander: Forged Alliance's look for TA Reborn's menus, made from the player's own copy\n# of the game: what each file is for. The game's MENU THEME (Options, Interface) reads it.\n");
    let quoted: Vec<String> = pictures.iter().map(|p| format!("\"menus/supcom/pictures/{p}.dds\"")).collect();
    s.push_str(&format!("pictures = [{}]\n", quoted.join(", ")));
    if music {
        s.push_str(&format!("music = \"{MUSIC}\"\n"));
    }
    s.push_str("\n[colours]\n");
    for (key, c) in skin.palette() {
        s.push_str(&format!("{key} = \"#{:02x}{:02x}{:02x}\"\n", c[0], c[1], c[2]));
    }
    if panel {
        s.push_str("\n[panel]\n");
        for (_, place) in PANEL {
            s.push_str(&format!("{place} = \"menus/supcom/panel/{place}.dds\"\n"));
        }
    }
    if !sounds.is_empty() {
        s.push_str("\n[sounds]\n");
        for (event, path) in sounds {
            s.push_str(&format!("{event} = \"{path}\"\n"));
        }
    }
    s
}

/// Makes the menus' mod of the copy whose files are `files` and whose `sounds` folder is `sounds`,
/// for TA Reborn `reborn` and newer: the pictures and the panel's art as the game has them (DDS),
/// the sounds as WAV, the music as FLAC, the theme, the manifest and the banner the launcher shows
/// the mod by ([`crate::banner::menus`]), written through `write` (a path in the mod, its bytes).
/// `watch` is told before each step how many are done and what it makes: false from it stops the
/// making there ([`Made::stopped`]).
pub fn make(files: &Files, sounds: Option<&Path>, reborn: &str, write: &mut dyn FnMut(&str, &[u8]) -> Result<()>, watch: &mut dyn FnMut(usize, &str) -> bool) -> Result<Made> {
    let skin = skin(files)?;
    let mut made = Made::default();
    let mut put = |path: &str, bytes: &[u8], made: &mut Made| -> Result<()> {
        made.files += 1;
        made.bytes += bytes.len() as u64;
        write(path, bytes)
    };
    // The banner the launcher shows the mod by, where the copy has the pictures it is made of;
    // the manifest naming it then.
    let banner = crate::banner::menus(files);
    let mut text = manifest(reborn);
    if banner.is_some() {
        text.push_str(&crate::banner::manifest_lines());
    }
    put("mod.toml", text.as_bytes(), &mut made)?;
    if let Some(banner) = &banner {
        put(crate::banner::PATH, banner, &mut made)?;
    }
    let mut step = 0;
    let mut stop = |step: &mut usize, what: &str, made: &mut Made| -> bool {
        let go = watch(*step, what);
        *step += 1;
        made.stopped = !go;
        !go
    };
    // The pictures, as they are.
    let mut pictures = Vec::new();
    for (path, name) in PICTURES {
        if stop(&mut step, name, &mut made) {
            return Ok(made);
        }
        match files.read(path).and_then(|b| b.ok_or_else(|| Error::Missing(path.into()))) {
            Ok(bytes) => match crate::dds::Header::parse(&bytes) {
                Ok(_) => {
                    put(&format!("menus/supcom/pictures/{name}.dds"), &bytes, &mut made)?;
                    pictures.push(name);
                }
                Err(e) => made.skipped.push((name.to_string(), e.to_string())),
            },
            Err(e) => made.skipped.push((name.to_string(), e.to_string())),
        }
    }
    made.pictures = pictures.len();
    // The panel's border: all nine pieces, or none.
    if stop(&mut step, "panel", &mut made) {
        return Ok(made);
    }
    let pieces: Result<Vec<(&str, Vec<u8>)>> = PANEL.iter().map(|(file, place)| {
        let path = format!("{PANEL_DIR}{file}.dds");
        let bytes = files.read(&path)?.ok_or_else(|| Error::Missing(path.clone()))?;
        crate::dds::Header::parse(&bytes)?;
        Ok((*place, bytes))
    }).collect();
    match pieces {
        Ok(pieces) => {
            for (place, bytes) in pieces {
                put(&format!("menus/supcom/panel/{place}.dds"), &bytes, &mut made)?;
            }
            made.panel = true;
        }
        Err(e) => made.skipped.push(("panel".into(), e.to_string())),
    }
    // The interface's sounds, each cue once.
    if stop(&mut step, "sounds", &mut made) {
        return Ok(made);
    }
    let mut played: Vec<(&str, String)> = Vec::new();
    if let Some(sounds) = sounds {
        let mut done: Vec<(&str, String)> = Vec::new();
        for (event, cue) in SOUNDS {
            if let Some((_, path)) = done.iter().find(|(c, _)| *c == cue) {
                played.push((event, path.clone()));
                continue;
            }
            match cue_wave(sounds, "Interface", cue).and_then(|w| w.ok_or_else(|| Error::Missing(format!("sounds/Interface.xsb: {cue}")))) {
                Ok((bank, wave)) => {
                    let pcm = wave_data(sounds, "Interface.xwb", &bank, wave)?;
                    let path = sound_path(cue);
                    put(&path, &wav(&pcm, &bank.waves[wave]), &mut made)?;
                    done.push((cue, path.clone()));
                    played.push((event, path));
                }
                Err(e) => made.skipped.push((cue.to_string(), e.to_string())),
            }
        }
        made.sounds = done.len();
    }
    // The menu music.
    if stop(&mut step, "music", &mut made) {
        return Ok(made);
    }
    if let Some(sounds) = sounds {
        let music = cue_wave(sounds, "Music", "Main_Menu").and_then(|w| w.ok_or_else(|| Error::Missing("sounds/Music.xsb: Main_Menu".into()))).and_then(|(bank, wave)| {
            let pcm = wave_data(sounds, "Music.xwb", &bank, wave)?;
            let w = &bank.waves[wave];
            crate::soundtrack::flac(&pcm, w.channels, w.rate, w.bits)
        });
        match music {
            Ok(flac) => {
                put(MUSIC, &flac, &mut made)?;
                made.music = true;
            }
            Err(e) => made.skipped.push(("Main_Menu".into(), e.to_string())),
        }
    }
    if stop(&mut step, "theme", &mut made) {
        return Ok(made);
    }
    put(THEME, theme(&skin, &pictures, made.panel, &played, made.music).as_bytes(), &mut made)?;
    Ok(made)
}

/// How many steps [`make`] tells its watch of: a picture each, the panel, the sounds, the music,
/// the theme.
pub const STEPS: usize = PICTURES.len() + 4;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xact::tests::{sound_bank as xsb, wave_bank as xwb};

    const SKINS: &str = r#"-- This file defines skin directories
skins = {
    uef = {
        default = "default",
        fontColor = "FFbadbdb",
        tooltipBorderColor = "FF7392a5",
    },
    default = {
        -- note, no default value here
        texturesPath = "/textures/ui/common",
        buttonFont = "Zeroes Three",
        fontColor = "FFbadbdb",
        fontOverColor = "FFFFFFFF",
        bodyColor = "FF6FAFAF",
        dialogButtonColor = "FF69AB4D",
        highlightColor = "FF7FA8A4",
        disabledColor = "FF47555a",
        panelColor = "FF4A5D6B",
        transparentPanelColor = "AA4A5D6B",
        menuFontSize = 18,
        cursors = {
            RULEUCC_Attack = {'/textures/ui/common/game/cursors/attack-.dds', 15, 15, 11, 12},
        },
    },
}
"#;

    /// A plain DDS picture, `side` texels square, of one colour.
    fn picture(side: u32) -> Vec<u8> {
        let mut b = crate::dds::header(side, side, 1, None);
        for _ in 0..side * side {
            b.extend_from_slice(&[40, 60, 80, 255]);
        }
        b
    }

    /// A copy of the game: its skins, two of the pictures, the panel's art, and the sounds
    /// folder's interface and music banks.
    fn copy(dir: &Path) -> Files {
        let pic = picture(8);
        let mut members: Vec<(String, Vec<u8>)> = vec![("lua/skins/skins.lua".into(), SKINS.as_bytes().to_vec())];
        members.push((PICTURES[0].0.into(), pic.clone()));
        members.push((PICTURES[10].0.into(), pic.clone()));
        for (file, _) in PANEL {
            members.push((format!("{PANEL_DIR}{file}.dds"), picture(4)));
        }
        let entries: Vec<(&str, &[u8], bool)> = members.iter().map(|(p, b)| (p.as_str(), b.as_slice(), true)).collect();
        let archive = dir.join("textures.scd");
        std::fs::write(&archive, crate::scd::tests::zip(&entries)).unwrap();
        let sounds = dir.join("sounds");
        std::fs::create_dir_all(&sounds).unwrap();
        let click = |n: usize| (0..n).map(|i| ((i as i32 * 97) % 3000) as i16).collect::<Vec<i16>>();
        std::fs::write(sounds.join("Interface.xwb"), xwb("Interface", &[(32_000, 1, click(100)), (32_000, 1, click(60))])).unwrap();
        std::fs::write(sounds.join("Interface.xsb"), xsb(&[("UI_Menu_MouseDown", vec![0], false), ("UI_Back_MouseDown", vec![1], false)])).unwrap();
        std::fs::write(sounds.join("Music.xwb"), xwb("Music", &[(44_100, 2, click(800))])).unwrap();
        std::fs::write(sounds.join("Music.xsb"), xsb(&[("Main_Menu", vec![0], true)])).unwrap();
        Files::of(Vec::new(), vec![crate::scd::Scd::open(&archive).unwrap()])
    }

    /// The default skin's colours, read from the game's script; and the palette made of them.
    #[test]
    fn the_skins_colours_make_the_palette() {
        let dir = crate::tests::Temp::new("menus_skin");
        let files = copy(&dir.0);
        let skin = skin(&files).unwrap();
        assert_eq!(skin.colour("fontcolor"), Some([0xba, 0xdb, 0xdb, 0xff]));
        assert_eq!(skin.colour("transparentPanelColor"), Some([0x4a, 0x5d, 0x6b, 0xaa]));
        assert_eq!(skin.colour("tooltipBorderColor"), None, "the UEF's, not the default skin's");
        let palette = skin.palette();
        let get = |k: &str| palette.iter().find(|(key, _)| *key == k).map(|(_, c)| *c);
        assert_eq!(get("text"), Some([0xba, 0xdb, 0xdb]));
        assert_eq!(get("dim"), Some([0x6f, 0xaf, 0xaf]));
        assert_eq!(get("accent"), Some([(0x7fu16 + 0xba) / 2, (0xa8 + 0xdb) / 2, (0xa4 + 0xdb) / 2].map(|v| v as u8)));
        assert_eq!(get("panel"), Some([0x4au32 * 0xaa / 255, 0x5d * 0xaa / 255, 0x6b * 0xaa / 255].map(|v| v as u8)));
        assert_eq!(get("good"), Some([0x69, 0xab, 0x4d]));
        assert_eq!(get("info"), Some([0x7f, 0xa8, 0xa4]), "the highlight, where the skin has no tooltip border");
        assert_eq!(hex_colour("#112233"), Some([0x11, 0x22, 0x33, 0xff]));
        assert_eq!(hex_colour("bad"), None);
    }

    /// What the copy has: the two pictures, the panel, the two sounds of the menus' cues it
    /// has, and the menu music's length.
    #[test]
    fn the_copys_contents_are_listed() {
        let dir = crate::tests::Temp::new("menus_contents");
        let files = copy(&dir.0);
        let found = contents(&files, Some(&dir.0.join("sounds"))).unwrap();
        assert_eq!(found.pictures, ["space_1", "factions"]);
        assert!(found.panel);
        assert_eq!(found.sounds, ["UI_Back_MouseDown", "UI_Menu_MouseDown"]);
        assert!((found.music_seconds.unwrap() - 400.0 / 44_100.0).abs() < 1e-9);
        let image = picture_of(&files);
        assert_eq!((image.width, image.height, image.rgba[0]), (8, 8, [80, 60, 40, 255]));
        let none = contents(&files, None).unwrap();
        assert!(none.sounds.is_empty() && none.music_seconds.is_none());
    }

    fn picture_of(files: &Files) -> crate::dds::Image {
        super::picture(files, "space_1").unwrap().unwrap()
    }

    /// The mod made: its manifest, the pictures there are, the panel's nine pieces, each sound
    /// once (the events sharing one cue, one file), the music, and the theme naming them all; a
    /// cue the copy lacks is said; stopped, no theme.
    #[test]
    fn the_menus_mod_is_made_with_its_theme() {
        let dir = crate::tests::Temp::new("menus_make");
        let files = copy(&dir.0);
        let sounds = dir.0.join("sounds");
        let mut out: Vec<(String, Vec<u8>)> = Vec::new();
        let made = make(&files, Some(&sounds), "1.0.9", &mut |path, bytes| {
            out.push((path.to_string(), bytes.to_vec()));
            Ok(())
        }, &mut |_, _| true).unwrap();
        assert_eq!((made.pictures, made.panel, made.sounds, made.music, made.stopped), (2, true, 2, true, false));
        let paths: Vec<&str> = out.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"menus/supcom/pictures/space_1.dds") && paths.contains(&"menus/supcom/panel/bottom_right.dds"), "{paths:?}");
        // (Its banner, of the space the copy has, named by its manifest.)
        assert_eq!(&paths[..2], ["mod.toml", crate::banner::PATH]);
        assert!(String::from_utf8_lossy(&out[0].1).ends_with("\n[media]\nbanner = \"media/banner.png\"\n"));
        assert_eq!(&out[1].1[..8], b"\x89PNG\r\n\x1a\n");
        assert!(paths.contains(&"sounds/supcom-menus/ui_menu_mousedown.wav") && paths.contains(&MUSIC) && paths.last() == Some(&THEME), "{paths:?}");
        let wav = &out.iter().find(|(p, _)| p == "sounds/supcom-menus/ui_back_mousedown.wav").unwrap().1;
        assert_eq!((&wav[..4], &wav[8..16], u32::from_le_bytes(wav[24..28].try_into().unwrap()), wav.len()), (&b"RIFF"[..], &b"WAVEfmt "[..], 32_000, 44 + 120));
        let theme = String::from_utf8(out.last().unwrap().1.clone()).unwrap();
        assert!(theme.contains("pictures = [\"menus/supcom/pictures/space_1.dds\", \"menus/supcom/pictures/factions.dds\"]"), "{theme}");
        assert!(theme.contains("music = \"music/supcom-menus/menu.flac\""), "{theme}");
        assert!(theme.contains("[colours]\ntext = \"#badbdb\""), "{theme}");
        assert!(theme.contains("top_left = \"menus/supcom/panel/top_left.dds\""), "{theme}");
        // (The four events of the menus' big buttons share one cue's file; an event whose cue the
        // copy lacks is left to TA's sound.)
        assert!(theme.contains("BIGBUTTON = \"sounds/supcom-menus/ui_menu_mousedown.wav\"\n") && theme.contains("OPTIONS = \"sounds/supcom-menus/ui_menu_mousedown.wav\"\n"), "{theme}");
        assert!(!theme.contains("PANEL ="), "{theme}");
        assert!(made.skipped.iter().any(|(what, _)| what == "UI_Main_Window_Open") && made.skipped.iter().any(|(what, _)| what == "space_2"));
        let stopped = make(&files, Some(&sounds), "1.0.9", &mut |_, _| Ok(()), &mut |i, _| i < 3).unwrap();
        assert!(stopped.stopped && stopped.pictures == 0);
    }

    /// A WAV of a wave as it is: its format and its samples.
    #[test]
    fn a_wave_becomes_a_wav_file() {
        let wave = Wave { codec: Codec::Pcm, channels: 2, rate: 44_100, bits: 16, frames: 2, offset: 0, length: 8 };
        let wav = wav(&[1, 2, 3, 4, 5, 6, 7, 8], &wave);
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 2);
        assert_eq!(u32::from_le_bytes(wav[28..32].try_into().unwrap()), 44_100 * 4);
        assert_eq!(&wav[44..], [1, 2, 3, 4, 5, 6, 7, 8]);
    }
}
