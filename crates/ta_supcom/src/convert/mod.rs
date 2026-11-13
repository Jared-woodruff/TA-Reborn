//! A Supreme Commander map made into a TA map: its land ([`land`]), the world of TA's it becomes
//! ([`kit`]), what stands on it ([`place`]), its ground painted ([`paint`]) and cut into TA's
//! tiles ([`art`]), then written as TA writes maps: `<stem>.tnt` (with its cliffs marked),
//! `<stem>.ota`, and `<stem>.features` (the map's own trees and rocks).
//!
//! A map converted from the same files by the same version of the converter is the same, byte
//! for byte, on every computer: players compare their maps' bytes before a battle.

pub mod art;
pub mod kit;
pub mod land;
pub mod paint;
pub mod place;
mod trig;

use ta_formats::image::Palette;
use ta_formats::tdf::{TdfDocument, TdfSection};
use ta_formats::tnt::{CELL_CLIFF, TntMap};

use crate::files::Files;
use crate::map::{Map, Scenario};
use crate::Result;

/// The converter's version: written in each map's OTA (`supcomversion`), so a map made by an
/// older one can be told and made again.
pub const VERSION: u32 = 1;
/// The version of the maps' own features, in their names (`SC1_Green_T1`): definitions of
/// another version are other features.
pub const FEATURES_VERSION: u32 = 1;
/// The most players a TA map holds.
pub const MAX_PLAYERS: usize = 10;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// The most tiles a map's art may have (the GPU holds a map's tiles at once: TA Reborn's
    /// own largest maps have about 24,000).
    pub budget: usize,
    /// The most cells a side of a map may have (its scale made smaller to fit).
    pub max_cells: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { budget: 28_000, max_cells: land::MAX_CELLS }
    }
}

/// What the converter is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    Land,
    Features,
    /// Painting the ground: the share done.
    Painting(f32),
    Writing,
}

/// A map made into a TA map.
pub struct Converted {
    /// Its file stem in TA (`Seton's Clutch (SC)`).
    pub stem: String,
    pub title: String,
    pub tnt: Vec<u8>,
    pub ota: Vec<u8>,
    /// Its own features' definitions (`maps/<stem>.features`), if it has any.
    pub features: Option<Vec<u8>>,
    /// Its size in cells, and how many players it is for.
    pub cells: (u32, u32),
    pub players: usize,
    pub tiles: usize,
    pub kit: &'static str,
    pub counts: place::Counts,
    /// Start pairs the game's land units drove between that a tank still cannot.
    pub cut_off: Vec<(usize, usize)>,
}

/// A map's file stem in TA: its name, `(SC)` after it; a player's own map's folder too, whole, so
/// two maps of one name stay apart (versions of a map share all but the end of their folders'
/// names). The same for every player who has the map (multiplayer finds maps by it); at most 94
/// characters, none Windows refuses.
pub fn stem(scenario: &Scenario, players_own: bool) -> String {
    let clean = |s: &str| -> String {
        s.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let mut name: String = clean(&scenario.name).trim_matches('.').chars().take(40).collect();
    if name.is_empty() {
        name = clean(&scenario.folder);
    }
    let tail = if players_own {
        let folder = clean(&scenario.folder.replace('.', " "));
        // (A folder's name too long to keep whole: cut, with a mark of the whole name.)
        let folder = if folder.chars().count() <= FOLDER_IN_STEM {
            folder
        } else {
            let cut: String = folder.chars().take(FOLDER_IN_STEM - 7).collect();
            format!("{}~{:06x}", cut.trim_end(), fnv1a(scenario.folder.as_bytes()) & 0xff_ffff)
        };
        format!(" (SC {folder})")
    } else {
        " (SC)".to_string()
    };
    format!("{}{tail}", name.trim())
}

/// The most of a player's own map's folder name a stem keeps whole.
const FOLDER_IN_STEM: usize = 48;

/// FNV-1a, 32 bits: the same on every platform.
fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5u32, |h, &b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

/// Converts `map` (read from `files`), its features drawn from TA's own (`templates`) and its art
/// dithered into `palette`. `players_own`: a map from a player's own folder, not the game's.
pub fn convert(map: &Map, files: &Files, templates: &kit::Templates, palette: &Palette, players_own: bool, options: &Options, stage: &(dyn Fn(Stage) + Sync)) -> Result<Converted> {
    let m = &map.scmap;
    let area = map.playable.map_or([0.0, 0.0, m.width as f64, m.height as f64], |a| a.map(|v| v as f64));
    let frame = land::Frame::within(area, options.max_cells);
    stage(Stage::Land);
    let mut land = land::build(m, frame);
    let kit = kit::choose(m, templates);
    stage(Stage::Features);
    let placed = place::place(map, &mut land, kit, templates, files)?;
    let painter = paint::Painter::new(m, files, frame)?;
    let art = art::paint(&painter, &land, options.budget, &palette.colors, &|share| stage(Stage::Painting(share)));
    stage(Stage::Writing);
    let flags: Vec<u8> = land.cliff.iter().map(|&c| if c { CELL_CLIFF } else { 0 }).collect();
    let mut tnt = TntMap {
        width: frame.w as u32,
        height: frame.h as u32,
        sea_level: land.sea,
        tiles: art.tiles,
        tile_map: art.tile_map,
        heights: land.heights.clone(),
        features: placed.cells.clone(),
        feature_names: placed.names.clone(),
        minimap: Default::default(),
        flags,
    };
    tnt.minimap = tnt.build_minimap(palette);
    let tiles = tnt.tiles.len();
    let tnt_bytes = tnt.write()?;
    let players = placed.starts.len().min(MAX_PLAYERS);
    let ota = ota(map, &land, kit, &placed.starts[..players]).to_bytes();
    let features = (!placed.own.is_empty()).then(|| placed.own.document().to_bytes());
    Ok(Converted {
        stem: stem(&map.scenario, players_own),
        title: map.scenario.name.clone(),
        tnt: tnt_bytes,
        ota,
        features,
        cells: (frame.w as u32, frame.h as u32),
        players,
        tiles,
        kit: kit.name,
        counts: placed.counts,
        cut_off: placed.cut_off,
    })
}

/// The map's OTA, laid out as TA's skirmish maps' are, with what TA Reborn reads of a map made
/// by its own tools (`rebornart`: no TA painter's hatching to clean) and where it came from.
fn ota(map: &Map, land: &land::Land, kit: kit::Kit, starts: &[(f64, f64)]) -> TdfDocument {
    let f = land.frame;
    let mut head = TdfSection::new("GlobalHeader");
    let counts: Vec<String> = (2..=starts.len().max(2)).map(|p| p.to_string()).collect();
    let description = format!("{} x {} {} (Supreme Commander: Forged Alliance)", f.w * 16 / 512, f.h * 16 / 512, map.scenario.description.trim());
    for (k, v) in [
        ("missionname", map.scenario.name.clone()),
        ("missiondescription", description.replace(['\r', '\n', ';', '{', '}', '[', ']'], " ")),
        ("planet", kit.planet.to_string()),
        ("missionhint", String::new()),
        ("brief", String::new()),
        ("narration", String::new()),
        ("glamour", String::new()),
        ("lineofsight", "1".into()),
        ("mapping", "1".into()),
        ("tidalstrength", if land.water.is_some() { "20" } else { "0" }.into()),
        ("solarstrength", "20".into()),
        ("lavaworld", "0".into()),
        ("killmul", "50".into()),
        ("timemul", "0".into()),
        ("minwindspeed", "500".into()),
        ("maxwindspeed", "2500".into()),
        ("gravity", "112".into()),
        ("numplayers", counts.join(", ")),
        ("size", format!("{} x {}", f.w * 16 / 512, f.h * 16 / 512)),
        ("memory", "32 mb".into()),
        ("DestroyAllUnits", "1".into()),
        ("AllUnitsKilled", "1".into()),
        ("SCHEMACOUNT", "1".into()),
        ("rebornart", "1".into()),
        ("supcommap", map.scenario.folder.clone()),
        ("supcomversion", VERSION.to_string()),
    ] {
        head.set(k, v);
    }
    let mut schema = TdfSection::new("Schema 0");
    for (k, v) in [
        ("Type", "Network 1"),
        ("aiprofile", "DEFAULT"),
        ("SurfaceMetal", "3"),
        ("MohoMetal", "40"),
        ("HumanMetal", "1000"),
        ("ComputerMetal", "1000"),
        ("HumanEnergy", "1000"),
        ("ComputerEnergy", "1000"),
        ("MeteorWeapon", ""),
        ("MeteorRadius", "0"),
        ("MeteorDensity", "0"),
        ("MeteorDuration", "0"),
        ("MeteorInterval", "0"),
    ] {
        schema.set(k, v);
    }
    let mut specials = TdfSection::new("specials");
    for (k, &(x, z)) in starts.iter().enumerate() {
        let mut s = TdfSection::new(format!("special{k}"));
        s.set("specialwhat", format!("StartPos{}", k + 1));
        s.set("XPos", (x.round() as i64).to_string());
        s.set("ZPos", (z.round() as i64).to_string());
        specials.sections.push(s);
    }
    schema.sections.push(specials);
    head.sections.push(schema);
    TdfDocument { sections: vec![head] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_are_the_maps_names_and_never_what_windows_refuses() {
        let mut s = Scenario { name: "Seton's Clutch".into(), folder: "SCMP_009".into(), ..Default::default() };
        assert_eq!(stem(&s, false), "Seton's Clutch (SC)");
        s.name = "What? A <map>: \"x\"/y".into();
        s.folder = "my.map.v0004".into();
        assert_eq!(stem(&s, true), "What A map x y (SC my map v0004)");
        s.name = String::new();
        assert_eq!(stem(&s, false), "my.map.v0004 (SC)");
    }

    /// Two of a player's maps of one name stay apart however alike their folders' names: kept
    /// whole, or, past their length, cut with a mark of the whole name.
    #[test]
    fn a_players_maps_of_one_name_keep_their_own_stems() {
        let own = |folder: &str| stem(&Scenario { name: "Seton's Clutch".into(), folder: folder.into(), ..Default::default() }, true);
        assert_eq!(own("adaptive_setons_clutch.v0001"), "Seton's Clutch (SC adaptive_setons_clutch v0001)");
        assert_ne!(own("adaptive_setons_clutch.v0001"), own("adaptive_setons_clutch.v0002"));
        let long = |v: u32| own(&format!("a_very_long_folder_name_for_a_map_of_the_vault_{v}.v0001"));
        let (a, b) = (long(1), long(2));
        assert_ne!(a, b);
        assert!(a.contains('~') && a.chars().count() <= 94, "{a}");
        assert_eq!(a, long(1), "the same for every player");
    }

    /// A small map converted end to end: its TNT, OTA and features read back as TA Reborn reads
    /// them, and converting it again gives the same bytes.
    #[test]
    fn a_map_converts_and_reads_back_the_same_every_time() {
        let dir = crate::tests::Temp::new("convert");
        let maps = dir.0.join("maps");
        let folder = crate::map::tests::write_map(&maps);
        // The ground's textures, in an archive of the game's.
        let plain = |rgba: [u8; 4]| { let mut b = crate::dds::tests::header(8, 8, 1, None); for _ in 0..64 { b.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]); } b };
        let names: Vec<String> = (0..10).map(|i| format!("env/evergreen/layers/a{i}_albedo.dds")).collect();
        let textures: Vec<Vec<u8>> = (0..10).map(|i| plain([40 + i * 10, 120, 60, 255])).collect();
        let members: Vec<(&str, &[u8], bool)> = names.iter().zip(&textures).map(|(n, t)| (n.as_str(), t.as_slice(), false)).collect();
        std::fs::write(dir.0.join("env.scd"), crate::scd::tests::zip(&members)).unwrap();
        let files = Files::of(vec![maps], vec![crate::scd::Scd::open(&dir.0.join("env.scd")).unwrap()]);
        let map = Map::read(&files, &folder).unwrap();
        let mut palette = Palette::default();
        for (i, c) in palette.colors.iter_mut().enumerate() {
            *c = [(i * 7 % 256) as u8, (i * 13 % 256) as u8, (i * 29 % 256) as u8];
        }
        let templates = kit::tests::templates();
        let options = Options::default();
        let a = convert(&map, &files, &templates, &palette, false, &options, &|_| {}).unwrap();
        let b = convert(&map, &files, &templates, &palette, false, &options, &|_| {}).unwrap();
        assert_eq!(a.tnt, b.tnt);
        assert_eq!(a.ota, b.ota);
        assert_eq!(a.stem, "Test Ridge (SC)");
        let tnt = TntMap::parse(&a.tnt).unwrap();
        assert_eq!((tnt.width, tnt.height), (32, 32));
        let ota = ta_formats::tdf::parse_bytes(&a.ota);
        let info = ta_data_like_starts(&ota);
        assert_eq!(info.len(), 2);
        assert_eq!(ota.section_path("GlobalHeader").and_then(|h| h.get("rebornart")), Some("1"));
        assert_eq!(ota.section_path("GlobalHeader").and_then(|h| h.get("supcommap")), Some("TEST_001"));
    }

    /// The start positions an OTA lists (`XPos`, `ZPos` of `StartPos<n>`).
    fn ta_data_like_starts(ota: &TdfDocument) -> Vec<(i64, i64)> {
        ota.section_path("GlobalHeader/Schema 0/specials").map(|s| s.sections.iter().filter_map(|p| Some((p.get_i64("XPos")?, p.get_i64("ZPos")?))).collect()).unwrap_or_default()
    }
}
