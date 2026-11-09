//! The maps there are, and how each is made: its size, land, colours, features and weather.
//! Each theme comes as a duel (12 x 12, two players), a team map (20 x 20) and a large one
//! (28 x 28), the menu's three battlefields (32 x 32) apart. The art's detail per area is the
//! first maps': the tile budget grows with the map ([`budget`]).

use super::coasts::{Bank, Coastal, Coasts, Isle};
use super::decks::Decks;
use super::geo::Geo;
use super::paint::desert::Desert;
use super::paint::earth::Earth;
use super::paint::metal::Metal;
use super::paint::snow::Snow;
use super::paint::space::Space;
use super::paint::volcanic::Volcanic;
use super::paint::wild::{Bands, Wild};
use super::paint::{Paint, Scheme, rgb};
use super::place::{EarthKit, Kit, Village};
use super::terrain::Shape;

/// A map to make: its land, colours and features, and what its OTA says.
pub struct Recipe {
    pub name: &'static str,
    pub planet: &'static str,
    /// What the menus say of it (TA's way: the size first, the players last).
    pub description: String,
    pub shape: Shape,
    pub paint: Paint,
    pub kit: Kit,
    /// Tracks worn between the bases and the middle.
    pub tracks: bool,
    /// Wind (TA's units, min and max), tides, sun.
    pub wind: (u32, u32),
    pub tidal: u32,
    pub solar: u32,
    pub surface_metal: u32,
    /// Most tiles the map may have (the game's tile atlas holds them all), and the share of
    /// them the tiles of rock in strata may keep whole ([`super::tiles::group`]).
    pub budget: usize,
    pub whole_rock: f32,
    /// What a moho mine digs (TA's metal worlds: as much as the surface gives).
    pub moho_metal: u32,
    /// Gravity (TA's 112 a world's; less on the rocks in space: shells fly further).
    pub gravity: u32,
}

/// The maps there are, by id.
pub const IDS: &[&str] = &[
    "highland-rift",
    "cinder-basin",
    "sapphire-reach",
    // Forests.
    "mossvale",
    "timberline",
    "greenmantle",
    // Deserts.
    "sunscar-dunes",
    "redstone-mesas",
    "mirage-basin",
    // Islands.
    "twin-reefs",
    "crescent-keys",
    "stormbreak-archipelago",
    "shore-to-shore-reborn",
    // Lava.
    "embergate",
    "furnace-caldera",
    "obsidian-wastes",
    // Metal.
    "foundry-deck",
    "reactor-grid",
    "hammerfall-works",
    // Space.
    "rift-belt",
    "lagrange-station",
    "starfall-expanse",
    // Seasonal: Christmas and the New Year.
    "frostfall-hollow",
    "hollyridge",
    "midnight-square",
    "first-light-peaks",
    // Real places.
    "australia",
    "united-states",
    "the-world",
];

/// Cells across a map `units` of TA's size long (a unit is 512 pixels, 32 cells).
const fn cells(units: usize) -> usize {
    units * 32
}

/// The tile budget of a map `w` x `h` cells: the first maps' detail per area (24,000 tiles over
/// 1024 x 1024 cells), and 4,000 at the least (a duel's shores, cliffs and tracks need their
/// tiles as a big map's do).
fn budget(w: usize, h: usize) -> usize {
    ((24_000.0 * (w * h) as f64 / (1024.0 * 1024.0)).round() as usize).max(4_000)
}

/// A description as TA's skirmish maps have them: "12 X 12  <what it is>. Up to 2 players."
fn describe(shape: &Shape, text: &str) -> String {
    format!("{} X {}  {text} Up to {} players.", shape.w / 32, shape.h / 32, shape.players)
}

/// The green worlds' features.
fn green_kit(forest: f32, cover: f32) -> Kit {
    Kit {
        trees: vec!["Tree1", "Tree2", "Tree3", "Tree4", "Tree5", "Tree6"],
        shrubs: vec!["Shrub1", "Shrub2", "Shrub3"],
        rocks_small: vec!["Rock07", "Rock08", "Rock09", "Rock10", "Rock11", "Rock12", "Rock13", "Rock07a", "Rock09a", "Rock11b"],
        rocks_big: vec![("Rock", 4, 3), ("Rock1a", 4, 3), ("Rock2", 3, 3), ("Rock2b", 3, 3), ("Rock3", 3, 2), ("Rock4", 2, 2), ("Rock05", 2, 2), ("Rock06a", 2, 2)],
        metal: vec!["RockMetal", "RockMetal1", "RockMetal2", "RockMetal3"],
        vents: vec!["Geothermal", "GreenVent01", "GreenVent02", "GreenVent03"],
        forest,
        cover,
        ..Kit::default()
    }
}

/// A temperate world's colours: an old wood's, mossy and green.
fn woodland() -> Wild {
    Wild {
        meadow: rgb(84, 138, 50),
        grass: rgb(100, 124, 56),
        dry: rgb(152, 142, 84),
        heath: rgb(116, 88, 76),
        moss: rgb(62, 96, 40),
        soil: rgb(116, 90, 62),
        litter: rgb(82, 68, 46),
        rock: rgb(142, 138, 126),
        rock_dark: rgb(80, 76, 70),
        lichen: rgb(170, 170, 122),
        sand: rgb(192, 178, 136),
        sand_wet: rgb(128, 116, 88),
        shallow: rgb(72, 118, 118),
        deep: rgb(28, 62, 108),
        track: rgb(134, 108, 74),
        reeds: rgb(116, 124, 62),
        flowers: [rgb(236, 218, 88), rgb(240, 238, 228), rgb(176, 118, 198)],
        bloom: 0.5,
        heath_share: 0.5,
        bare: 0.25,
        outcrops: 0.6,
        wetness: 0.05,
        rock_from: 9.0,
        rock_full: 17.0,
        beach: 4.0,
        beach_cells: 10.0,
        marsh: 0.3,
        reef: 0.0,
        coral: [rgb(220, 120, 110), rgb(236, 180, 80), rgb(150, 110, 190)],
        crisp: false,
        bands: None,
    }
}

/// A desert world's colours: golden sand, red rock.
fn sandstone() -> Desert {
    Desert {
        sand: rgb(226, 192, 114),
        sand_pale: rgb(240, 220, 166),
        sand_red: rgb(224, 176, 106),
        flat: rgb(206, 178, 126),
        gravel: rgb(124, 96, 72),
        strata: [rgb(186, 106, 68), rgb(216, 162, 112), rgb(160, 86, 58), rgb(228, 192, 142)],
        cap: rgb(108, 80, 64),
        scrub: rgb(118, 126, 86),
        silt: rgb(226, 204, 158),
        salt: rgb(240, 236, 228),
        green: rgb(92, 130, 56),
        reeds: rgb(128, 134, 68),
        shallow: rgb(72, 150, 140),
        deep: rgb(30, 92, 112),
        track: rgb(214, 188, 146),
        dune_gap: 26.0,
        wind: 0.35,
        salt_share: 0.0,
        scrub_share: 0.4,
        band: 9.0,
        rock_from: 8.0,
        rock_full: 15.0,
        oases: true,
    }
}

/// The desert worlds' features.
fn desert_kit() -> Kit {
    Kit {
        rocks_big: vec![("DryRock01", 4, 3), ("DryRock03", 2, 2), ("DryRock04", 3, 4), ("DryRock07", 3, 3), ("DryRock08", 2, 2), ("DryRock10", 4, 4), ("DryRock12", 3, 3), ("DryRock15", 3, 3)],
        rocks_small: vec!["DryRock05"],
        metal: vec!["DryMetal01", "DryMetal02", "DryMetal03", "DryMetalB01", "DryMetalB02"],
        vents: vec!["DryVent01", "DryVent02", "DryVent03", "DryVent04"],
        ruins: vec![("DryRuin01", 4, 4), ("DryRuin02", 3, 3), ("DryRuin04", 1, 2), ("DryRuin05", 2, 3), ("DryRuin06", 3, 2), ("DryRuin09", 3, 3), ("DryRuin14", 5, 5), ("DryRuin16", 5, 2), ("DryRuin17", 2, 3), ("DryRuin11", 5, 4)],
        ruin_density: 60.0,
        grove: vec!["Palm01", "Palm02", "Palm03", "Palm04", "Palm05", "Palm06"],
        base_metal: 6,
        metal_fields: 1.2,
        ..Kit::default()
    }
}

/// A tropical world's colours: coral sand, turquoise shallows, palms and jungle.
fn tropical() -> Wild {
    Wild {
        meadow: rgb(72, 148, 58),
        grass: rgb(98, 140, 58),
        dry: rgb(176, 168, 98),
        heath: rgb(126, 108, 70),
        moss: rgb(54, 108, 46),
        soil: rgb(150, 112, 74),
        litter: rgb(80, 78, 46),
        rock: rgb(156, 148, 132),
        rock_dark: rgb(90, 84, 76),
        lichen: rgb(184, 180, 134),
        sand: rgb(238, 224, 180),
        sand_wet: rgb(198, 184, 140),
        shallow: rgb(62, 196, 190),
        deep: rgb(16, 66, 132),
        track: rgb(198, 172, 122),
        reeds: rgb(104, 138, 66),
        flowers: [rgb(236, 112, 160), rgb(246, 210, 72), rgb(244, 240, 230)],
        bloom: 0.35,
        heath_share: 0.15,
        bare: 0.15,
        outcrops: 0.45,
        wetness: 0.1,
        rock_from: 9.0,
        rock_full: 17.0,
        beach: 8.0,
        beach_cells: 20.0,
        marsh: 0.25,
        reef: 0.75,
        coral: [rgb(228, 118, 108), rgb(238, 184, 84), rgb(156, 112, 196)],
        crisp: false,
        bands: None,
    }
}

/// The archipelago worlds' features, the sea's own among them.
fn isle_kit() -> Kit {
    let mut reef: Vec<(&'static str, usize, usize)> = vec![
        ("AquaCoral02", 2, 2),
        ("AquaCoral03", 2, 2),
        ("AquaCoral04", 2, 2),
        ("AquaCoral05", 2, 2),
        ("AquaCoral06", 2, 2),
        ("aquacoral07", 2, 2),
        ("aquacoral08", 2, 2),
        ("aquacoral09", 2, 2),
        ("aquacoral10", 2, 2),
        ("aquacoral12", 2, 2),
        ("aquacoral14", 2, 2),
        ("aquacoral16", 2, 2),
    ];
    reef.extend([("anemone01", 2, 2), ("anemone04", 2, 2), ("anemone07", 2, 2), ("anemone10", 2, 2), ("anemone13", 2, 2), ("anemone18", 2, 2), ("anemone22", 2, 2), ("anemone27", 2, 2), ("anemone31", 2, 2), ("anemone36", 2, 2)]);
    Kit {
        trees: vec!["ArchiTree01", "ArchiTree02", "ArchiTree03", "ArchiTree04", "ArchiTree05", "ArchiTree06", "Palm01", "Palm02", "Palm03", "Palm04", "Palm05", "Palm06"],
        shrubs: vec!["ArchiFoliage01", "ArchiFoliage02", "ArchiFoliage03", "ArchiFoliage04", "ArchiFoliage05", "ArchiFoliage06"],
        rocks_small: vec!["Rock07", "Rock09", "Rock11", "Rock12"],
        rocks_big: vec![("Rock2", 3, 3), ("Rock3", 3, 2), ("Rock4", 2, 2), ("Rock05", 2, 2)],
        metal: vec!["ArchMetal1", "ArchMetal2", "ArchMetal3", "ArchMetalB1", "ArchMetalB2", "ArchMetalB3"],
        vents: vec!["ArchiVent01", "ArchiVent02"],
        forest: 0.4,
        cover: 0.0,
        glades: 0.5,
        clearing: 48.0,
        base_metal: 6,
        metal_fields: 1.3,
        reef,
        reef_density: 260.0,
        sea_metal: vec!["ArchiAquaOre1", "ArchiAquaOre2", "ArchiAquaOre3"],
        sea_metal_every: 7.0,
        ..Kit::default()
    }
}

/// A lava world's colours: basalt and ash, cinder and sulphur, a sea of fire.
fn basalt() -> Volcanic {
    Volcanic {
        basalt: rgb(92, 84, 80),
        basalt_dark: rgb(62, 56, 54),
        ash: rgb(112, 104, 96),
        cinder: rgb(118, 62, 46),
        flow: rgb(60, 56, 56),
        scorch: rgb(30, 26, 26),
        sulphur: rgb(212, 192, 72),
        glow: rgb(250, 140, 28),
        melt: rgb(176, 32, 6),
        track: rgb(112, 106, 100),
        ash_share: 0.4,
        cinder_share: 0.3,
        flow_share: 0.5,
        rock_from: 7.0,
        rock_full: 14.0,
    }
}

/// The lava worlds' features.
fn lava_kit(landmarks: f32) -> Kit {
    Kit {
        rocks_big: vec![("LavaRockB017", 2, 2), ("LavaRockC012", 3, 2), ("LavaRockC008", 4, 4), ("LavaRockC005", 4, 4), ("LavaRockB012", 4, 4), ("LavaRockC015", 4, 3), ("LavaRockA005", 3, 3), ("LavaRockA016", 2, 2)],
        metal: vec!["LavaMetal1", "LavaMetal2", "LavaMetal3"],
        vents: vec!["Lavavent001", "Lavavent002", "Lavavent003", "Lavavent004", "Lavavent005", "Lavavent006", "Lavavent007", "Lavavent008"],
        landmarks: vec![("LavaSpire006A", 5, 4), ("LavaSpire006C", 6, 5), ("LavaSpire008A", 7, 5), ("LavaSpire003", 7, 7), ("LavaNode015", 3, 3), ("LavaNode010", 4, 4), ("LavaNode003D", 4, 4), ("LavaNode005", 6, 6)],
        landmark_density: landmarks,
        base_metal: 6,
        metal_fields: 1.3,
        ..Kit::default()
    }
}

/// A metal world's colours: steel decks, hazard yellow, coolant in the channels.
fn steel() -> Metal {
    Metal {
        decks: [rgb(122, 126, 130), rgb(110, 114, 120), rgb(128, 124, 118), rgb(104, 112, 122)],
        dark: rgb(40, 42, 46),
        light: rgb(176, 178, 182),
        grate: rgb(44, 46, 50),
        rust: rgb(128, 72, 40),
        grime: rgb(62, 58, 52),
        hazard: rgb(222, 178, 40),
        hazard_dark: rgb(38, 36, 32),
        lamp: rgb(255, 238, 176),
        wall: rgb(100, 102, 106),
        wall_dark: rgb(52, 54, 58),
        bunker: rgb(86, 90, 94),
        coolant: rgb(42, 72, 80),
        coolant_deep: rgb(16, 32, 40),
        rust_share: 0.5,
        grime_share: 0.7,
        rock_from: 9.0,
        rock_full: 16.0,
    }
}

/// The metal worlds' features: comm towers, pipes and steam vents (no deposits: the plating
/// gives metal anywhere).
fn metal_kit(towers: f32, pipes: f32) -> Kit {
    Kit {
        landmarks: vec![("MetalTower01", 5, 5), ("MetalTower02", 5, 5), ("MetalTower03", 8, 7), ("MetalTower04", 7, 7), ("MetalTower05", 5, 5), ("MetalTower06", 5, 5), ("MetalTower07", 6, 5), ("MetalTower08", 5, 6), ("MetalTower11", 5, 5), ("MetalTower12", 5, 5), ("MetalTower13", 5, 5), ("MetalTower14", 5, 5)],
        landmark_density: towers,
        ruins: vec![("Pipe01", 7, 4), ("Pipe02", 6, 4), ("Pipe03", 8, 5), ("Pipe05", 3, 3), ("Pipe06", 5, 4), ("Pipe07", 4, 4), ("Pipe09", 7, 2), ("Pipe10", 7, 2), ("Pipe11", 3, 3), ("Pipe13", 7, 5), ("Pipe14", 4, 2), ("Pipe15", 4, 3), ("Pipe17", 4, 2)],
        ruin_density: pipes,
        vents: vec!["MetalVent01", "MetalVent02", "MetalVent03", "MetalVent04", "MetalVent05", "MetalVent06"],
        vent_size: 3,
        ..Kit::default()
    }
}

/// A world in space's colours: grey regolith, dark rock, plating on the gantries.
fn regolith() -> Space {
    Space {
        dust: rgb(128, 126, 122),
        dust_dark: rgb(98, 96, 94),
        dust_pale: rgb(156, 152, 146),
        rock: rgb(110, 106, 102),
        rock_dark: rgb(48, 46, 46),
        ejecta: rgb(176, 172, 166),
        black: rgb(2, 2, 5),
        nebula: rgb(22, 10, 30),
        plating: Metal { decks: [rgb(118, 124, 132), rgb(108, 114, 122), rgb(124, 124, 126), rgb(104, 112, 124)], rust_share: 0.1, grime_share: 0.4, ..steel() },
        platform: 14.0,
        rock_from: 9.0,
        rock_full: 16.0,
    }
}

/// The moon's features: boulders and craters, ore.
fn moon_kit(boulders: f32) -> Kit {
    Kit {
        rocks_small: vec!["MoonRock01", "MoonRock02", "MoonRock03", "MoonRock04", "MoonRock05"],
        rocks_big: vec![("MoonRock06", 3, 1), ("MoonRock07", 3, 2), ("MoonRock08", 3, 3), ("MoonRock09", 3, 3), ("MoonRock10", 3, 4), ("MoonRock12", 4, 4), ("MoonRock14", 4, 4), ("MoonRock16", 4, 4)],
        metal: vec!["MoonMetal01", "MoonMetal02", "MoonMetal03", "MoonMetalDark01", "MoonMetalDark02", "MoonMetalDark03"],
        landmarks: vec![("MoonRock15", 6, 6), ("MoonRock17", 5, 5), ("MoonRock19", 6, 7), ("MoonRock13", 5, 5), ("MoonCrater06", 8, 8), ("MoonCrater08", 7, 6)],
        landmark_density: boulders,
        base_metal: 6,
        metal_fields: 1.6,
        corridor: true,
        ..Kit::default()
    }
}

/// A winter world's tile budget: half as much again as its size's ([`budget`]): snow is smooth,
/// and a tile shared where another belonged shows on it more than on grass or sand.
const fn winter_budget(w: usize, h: usize) -> usize {
    (24_000 * w * h / (1024 * 1024)) * 3 / 2
}

/// A winter world's colours: snow under a pale sky, blue in its shade, grey-brown rock, pale ice.
fn winter() -> Snow {
    Snow {
        snow: rgb(238, 242, 248),
        shade: rgb(200, 214, 236),
        packed: rgb(222, 226, 233),
        rock: rgb(150, 146, 142),
        rock_dark: rgb(98, 100, 108),
        ice: rgb(176, 208, 228),
        ice_dark: rgb(96, 138, 178),
        road: rgb(212, 213, 216),
        slush: rgb(146, 138, 130),
        cobble: rgb(132, 128, 124),
        soil: rgb(92, 86, 80),
        needles: rgb(66, 80, 62),
        wind: 0.35,
        crust: 0.45,
        rock_from: 7.0,
        rock_full: 13.0,
    }
}

/// A Christmas village: the great fir in its square, cottages, lanterns, snowmen, presents, ice
/// sculptures and a sleigh, decorated firs at its edge.
fn christmas(density: f32) -> Village {
    Village {
        centre: ("GreatFir", 4, 4),
        houses: vec![("Cottage1", 4, 4), ("Cottage2", 4, 4), ("Cottage3", 4, 4)],
        lamps: vec!["StreetLantern"],
        decorations: vec![("Snowman1", 1, 1), ("Presents1", 2, 2), ("IceStar", 2, 2), ("Snowman2", 1, 1), ("Presents2", 2, 2), ("Sleigh", 3, 2), ("IceFir", 2, 2), ("IceLanterns", 1, 1)],
        firs: vec!["FestiveFir1", "FestiveFir2"],
        density,
        ..Village::default()
    }
}

/// A New Year's village: a clock tower in its square, fireworks round it waiting for midnight,
/// strings of lanterns, ice lanterns.
fn new_year(density: f32, middle: bool) -> Village {
    Village {
        centre: ("ClockTower", 4, 4),
        houses: vec![("Cottage3", 4, 4), ("Cottage1", 4, 4), ("Cottage2", 4, 4)],
        lamps: vec!["StreetLantern"],
        decorations: vec![("IceLanterns", 1, 1), ("Snowman1", 1, 1), ("IceStar", 2, 2), ("Presents1", 2, 2), ("IceLanterns", 1, 1), ("Snowman2", 1, 1)],
        fireworks: vec![("Fireworks1", 2, 1), ("Fireworks2", 2, 1)],
        strings: vec![("LanternString", 3, 1)],
        firs: vec!["FestiveFir2"],
        density,
        middle,
    }
}

/// A winter world's features: TA Reborn's own snowy firs and boulders, the ice world's metal and
/// vents, and its villages.
fn winter_kit(forest: f32, cover: f32, village: Village) -> Kit {
    Kit {
        trees: vec!["SnowPine1", "SnowPine2", "SnowPine3"],
        rocks_big: vec![("SnowRock1", 2, 2), ("SnowRock2", 2, 2), ("SnowRock3", 3, 3)],
        metal: vec!["IceMetal01", "IceMetal02", "IceMetal03"],
        vents: vec!["IceVent01", "IceVent02", "IceVent03", "IceVent04"],
        forest,
        cover,
        glades: 0.55,
        clearing: 50.0,
        base_metal: 6,
        metal_fields: 1.5,
        corridor: true,
        village: Some(village),
        ..Kit::default()
    }
}

/// The painters of a real place, by biome: the temperate woods and the tropics' (the first maps'
/// own colours), the north's dark conifer woods, prairie and savanna, hot and cold deserts, tundra
/// and the ice caps.
fn earth() -> Earth {
    Earth {
        temperate: Wild { outcrops: 0.5, crisp: true, ..woodland() },
        // (Its deep sea the temperate one's: only the shallows turquoise, no edge across the sea.)
        tropical: Wild { reef: 0.5, deep: woodland().deep, crisp: true, ..tropical() },
        boreal: Wild {
            meadow: rgb(70, 108, 58),
            grass: rgb(82, 104, 60),
            dry: rgb(132, 128, 90),
            heath: rgb(104, 86, 74),
            moss: rgb(58, 88, 50),
            soil: rgb(100, 86, 70),
            litter: rgb(66, 62, 46),
            rock: rgb(132, 132, 128),
            rock_dark: rgb(76, 76, 74),
            lichen: rgb(176, 178, 150),
            bloom: 0.15,
            heath_share: 0.4,
            crisp: true,
            ..woodland()
        },
        prairie: Wild {
            meadow: rgb(150, 156, 86),
            grass: rgb(170, 160, 96),
            dry: rgb(200, 184, 124),
            heath: rgb(152, 124, 88),
            moss: rgb(116, 128, 70),
            soil: rgb(150, 122, 86),
            flowers: [rgb(236, 210, 92), rgb(214, 120, 170), rgb(240, 236, 226)],
            bloom: 0.3,
            heath_share: 0.25,
            bare: 0.15,
            outcrops: 0.3,
            wetness: -0.15,
            crisp: true,
            ..woodland()
        },
        savanna: Wild {
            meadow: rgb(176, 164, 92),
            grass: rgb(192, 170, 98),
            dry: rgb(214, 188, 126),
            heath: rgb(166, 120, 80),
            moss: rgb(126, 132, 72),
            soil: rgb(178, 112, 72),
            litter: rgb(118, 96, 62),
            bloom: 0.1,
            heath_share: 0.2,
            bare: 0.3,
            outcrops: 0.35,
            wetness: -0.2,
            deep: woodland().deep,
            crisp: true,
            ..tropical()
        },
        desert: Desert { scrub_share: 0.3, oases: false, ..sandstone() },
        steppe: Desert {
            oases: false,
            sand: rgb(184, 166, 128),
            sand_pale: rgb(206, 192, 158),
            sand_red: rgb(186, 152, 112),
            flat: rgb(166, 150, 120),
            gravel: rgb(112, 102, 92),
            scrub: rgb(122, 126, 92),
            ..sandstone()
        },
        tundra: Wild {
            meadow: rgb(128, 134, 104),
            grass: rgb(140, 138, 110),
            dry: rgb(164, 156, 128),
            heath: rgb(128, 106, 94),
            moss: rgb(108, 118, 88),
            soil: rgb(114, 106, 96),
            litter: rgb(96, 92, 80),
            rock: rgb(150, 150, 148),
            rock_dark: rgb(92, 94, 96),
            lichen: rgb(198, 198, 170),
            bloom: 0.08,
            heath_share: 0.6,
            bare: 0.35,
            outcrops: 0.8,
            crisp: true,
            ..woodland()
        },
        ice: winter(),
        salt: rgb(236, 232, 222),
    }
}

/// A real place's features: the green worlds', given the tropics', the north's, the deserts'
/// and the cold's by biome where they stand ([`EarthKit`]).
fn earth_kit(forest: f32) -> Kit {
    Kit {
        earth: Some(EarthKit {
            tropical: vec!["LushTree01", "Lush02", "Lush03", "Lush04", "Lush05", "Lush06", "ArchiTree01", "ArchiTree02", "ArchiTree03"],
            boreal: vec!["Tree4", "Tree5", "Tree6"],
            palms: vec!["Palm01", "Palm02", "Palm03", "Palm04", "Palm05", "Palm06"],
            desert_rocks: vec![("DryRock01", 4, 3), ("DryRock03", 2, 2), ("DryRock07", 3, 3), ("DryRock08", 2, 2), ("DryRock12", 3, 3), ("DryRock15", 3, 3)],
            cold_rocks: vec![("SnowRock1", 2, 2), ("SnowRock2", 2, 2), ("SnowRock3", 3, 3)],
            // (Kind for kind with the green kit's RockMetal, RockMetal1, 2 and 3, which hold 187,
            // 127, 86 and 223 a cell: the nearest of the biome's, so a base in the desert or the
            // cold has as much metal as one in the woods.)
            desert_metal: vec!["DryMetal03", "DryMetal01", "DryMetal02", "DryMetal03"],
            cold_metal: vec!["IceMetal03", "IceMetal01", "IceMetal02", "IceMetal03"],
            desert_vents: vec!["DryVent01", "DryVent02", "DryVent03", "DryVent04"],
            cold_vents: vec!["IceVent01", "IceVent02", "IceVent03", "IceVent04"],
        }),
        glades: 0.5,
        clearing: 50.0,
        base_metal: 6,
        metal_fields: 1.5,
        ..green_kit(forest, 0.1)
    }
}

pub fn recipe(id: &str) -> Option<Recipe> {
    let r = match id {
        "highland-rift" => Recipe {
            name: "Highland Rift",
            planet: "Green planet",
            description: "32 X 32  A river cuts a canyon through green highlands: forests, lakes, cliffs and passes. Up to 10 players.".into(),
            shape: Shape {
                w: 1024,
                h: 1024,
                seed: 0x4852_1F07,
                sea: 40.0,
                levels: vec![64.0, 104.0, 146.0, 186.0],
                steps: vec![0.36, 0.68, 0.90],
                hills: 12.0,
                hill_scale: 70.0,
                peaks: 52.0,
                river: true,
                lakes: 2,
                ocean: 0.0,
                scale: 2.4,
                players: 10,
                ..Shape::default()
            },
            paint: Paint::Classic(Scheme {
                grass_lush: rgb(58, 124, 36),
                grass_wet: rgb(66, 110, 38),
                grass_dry: rgb(122, 126, 62),
                dirt: rgb(118, 92, 60),
                forest: rgb(36, 60, 26),
                rock: rgb(132, 126, 112),
                rock_dark: rgb(80, 74, 66),
                peak: rgb(150, 146, 134),
                sand: rgb(196, 178, 128),
                sand_wet: rgb(142, 128, 94),
                shallow: rgb(66, 120, 132),
                deep: rgb(30, 68, 128),
                rock_from: 9.0,
                rock_full: 17.0,
                bare: 0.28,
                beach: 4.5,
                beach_cells: 16.0,
                molten: false,
            }),
            kit: green_kit(0.42, 0.0),
            tracks: false,
            wind: (600, 2400),
            tidal: 20,
            solar: 150,
            surface_metal: 3,
            budget: 24_000,
            whole_rock: 0.0,
            moho_metal: 30,
            gravity: 112,
        },
        "cinder-basin" => Recipe {
            name: "Cinder Basin",
            planet: "Lava",
            description: "32 X 32  Basalt terraces over a river of fire and its molten lakes, spires on the heights. Up to 8 players.".into(),
            shape: Shape {
                w: 1024,
                h: 1024,
                seed: 0x0C1D_E4B5,
                sea: 42.0,
                levels: vec![62.0, 98.0, 140.0, 182.0],
                steps: vec![0.40, 0.70, 0.90],
                hills: 9.0,
                hill_scale: 55.0,
                peaks: 60.0,
                river: true,
                lakes: 3,
                ocean: 0.0,
                scale: 2.8,
                players: 8,
                ..Shape::default()
            },
            paint: Paint::Classic(Scheme {
                grass_lush: rgb(38, 32, 30),
                grass_wet: rgb(56, 48, 44),
                grass_dry: rgb(88, 78, 70),
                dirt: rgb(124, 56, 32),
                forest: rgb(48, 42, 40),
                rock: rgb(92, 80, 72),
                rock_dark: rgb(38, 32, 30),
                peak: rgb(140, 128, 116),
                sand: rgb(58, 46, 40),
                sand_wet: rgb(150, 64, 22),
                shallow: rgb(250, 140, 28),
                deep: rgb(176, 32, 6),
                rock_from: 7.0,
                rock_full: 14.0,
                bare: 0.75,
                beach: 3.0,
                beach_cells: 8.0,
                molten: true,
            }),
            kit: Kit {
                rocks_big: vec![("LavaRockB017", 2, 2), ("LavaRockC012", 3, 2), ("LavaRockC008", 4, 4), ("LavaRockC005", 4, 4), ("LavaRockB012", 4, 4), ("LavaRockC015", 4, 3)],
                metal: vec!["LavaMetal1", "LavaMetal2", "LavaMetal3"],
                vents: vec!["Lavavent001", "Lavavent002", "Lavavent003", "Lavavent004", "Lavavent005", "Lavavent006"],
                landmarks: vec![("LavaSpire006A", 5, 4), ("LavaSpire006C", 6, 5), ("LavaSpire008A", 7, 5), ("LavaSpire012", 9, 8), ("LavaSpire003", 7, 7), ("LavaNode015", 3, 3), ("LavaNode010", 4, 4), ("LavaNode003D", 4, 4), ("LavaNode005", 6, 6)],
                landmark_density: 260.0,
                ..Kit::default()
            },
            tracks: false,
            wind: (200, 1400),
            tidal: 0,
            solar: 120,
            surface_metal: 4,
            budget: 24_000,
            whole_rock: 0.0,
            moho_metal: 30,
            gravity: 112,
        },
        "sapphire-reach" => Recipe {
            name: "Sapphire Reach",
            planet: "Archipelago",
            description: "32 X 32  Wooded islands on a turquoise sea: beaches, coves and deep channels for the fleets. Up to 8 players.".into(),
            shape: Shape {
                w: 1024,
                h: 1024,
                seed: 0x5A9F_1E2D,
                sea: 60.0,
                levels: vec![40.0, 84.0, 122.0, 160.0],
                steps: vec![0.50, 0.78, 0.93],
                hills: 14.0,
                hill_scale: 60.0,
                peaks: 40.0,
                river: false,
                lakes: 0,
                ocean: 0.52,
                scale: 4.2,
                players: 8,
                ..Shape::default()
            },
            paint: Paint::Classic(Scheme {
                grass_lush: rgb(56, 132, 46),
                grass_wet: rgb(74, 128, 46),
                grass_dry: rgb(146, 146, 76),
                dirt: rgb(146, 114, 72),
                forest: rgb(34, 72, 28),
                rock: rgb(150, 140, 120),
                rock_dark: rgb(92, 84, 74),
                peak: rgb(170, 162, 144),
                sand: rgb(226, 208, 152),
                sand_wet: rgb(172, 154, 112),
                shallow: rgb(58, 170, 168),
                deep: rgb(18, 58, 128),
                rock_from: 9.0,
                rock_full: 17.0,
                bare: 0.2,
                beach: 9.0,
                beach_cells: 22.0,
                molten: false,
            }),
            kit: Kit {
                trees: vec!["ArchiTree01", "ArchiTree02", "ArchiTree03", "ArchiTree04", "Palm01", "Palm02", "Palm03", "Palm04", "Palm05", "Palm06"],
                shrubs: vec!["ArchiFoliage01", "ArchiFoliage02", "ArchiFoliage03", "ArchiFoliage04", "ArchiFoliage05"],
                rocks_small: vec!["Rock07", "Rock09", "Rock11", "Rock12"],
                rocks_big: vec![("Rock2", 3, 3), ("Rock3", 3, 2), ("Rock4", 2, 2)],
                metal: vec!["ArchMetal1", "ArchMetal2", "ArchMetal3", "ArchMetalB1", "ArchMetalB2"],
                vents: vec!["ArchiVent01", "ArchiVent02"],
                forest: 0.38,
                ..Kit::default()
            },
            tracks: false,
            wind: (900, 3000),
            tidal: 30,
            solar: 170,
            surface_metal: 3,
            budget: 24_000,
            whole_rock: 0.0,
            moho_metal: 30,
            gravity: 112,
        },
        // ---- Forests --------------------------------------------------------------------------
        "mossvale" => {
            let shape = Shape {
                w: cells(12),
                h: cells(12),
                seed: 0x3055_7A1E,
                sea: 30.0,
                levels: vec![50.0, 84.0, 120.0],
                steps: vec![0.42, 0.80],
                hills: 10.0,
                hill_scale: 50.0,
                peaks: 0.0,
                river: true,
                lakes: 1,
                scale: 1.6,
                players: 2,
                symmetric: true,
                ramps: 0.1,
                guide: vec![(0.16, 0.16, 0.16, 0.9)],
                connect: true,
                ..Shape::default()
            };
            Recipe {
                name: "Mossvale",
                planet: "Green planet",
                description: describe(&shape, "An old wood in a mossy vale: a stream and its tarn between two clearings, fords and forest paths."),
                shape,
                paint: Paint::Wild(Wild { bloom: 0.6, heath_share: 0.2, ..woodland() }),
                kit: Kit { glades: 0.7, clearing: 52.0, base_metal: 6, metal_fields: 2.0, ..green_kit(0.45, 0.08) },
                tracks: true,
                wind: (500, 1800),
                tidal: 15,
                solar: 140,
                surface_metal: 3,
                budget: 0,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        "timberline" => {
            let shape = Shape {
                w: cells(20),
                h: cells(20),
                seed: 0x71B3_E2C9,
                sea: 32.0,
                levels: vec![54.0, 92.0, 132.0, 170.0],
                steps: vec![0.38, 0.68, 0.88],
                hills: 12.0,
                hill_scale: 60.0,
                peaks: 46.0,
                river: true,
                lakes: 2,
                scale: 2.2,
                players: 6,
                symmetric: true,
                axis: std::f32::consts::PI,
                ramps: 0.05,
                connect: true,
                ..Shape::default()
            };
            Recipe {
                name: "Timberline",
                planet: "Green planet",
                description: describe(&shape, "Forested ridges climb to bare peaks over a river and its lakes: passes, fords and long timber trails."),
                shape,
                paint: Paint::Wild(Wild { heath_share: 0.7, outcrops: 0.8, wetness: -0.04, ..woodland() }),
                kit: Kit { glades: 0.6, clearing: 55.0, base_metal: 6, metal_fields: 1.5, ..green_kit(0.45, 0.06) },
                tracks: true,
                wind: (700, 2600),
                tidal: 15,
                solar: 130,
                surface_metal: 3,
                budget: 0,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        "greenmantle" => {
            let shape = Shape {
                w: cells(28),
                h: cells(28),
                seed: 0x6EE4_0D35,
                sea: 34.0,
                levels: vec![52.0, 88.0, 126.0],
                steps: vec![0.46, 0.82],
                hills: 14.0,
                hill_scale: 64.0,
                peaks: 24.0,
                river: true,
                lakes: 3,
                scale: 2.8,
                players: 8,
                symmetric: true,
                axis: std::f32::consts::FRAC_PI_4 * 3.0,
                ramps: 0.15,
                pools: (3, 16.0),
                connect: true,
                ..Shape::default()
            };
            Recipe {
                name: "Greenmantle",
                planet: "Lush",
                description: describe(&shape, "A rainforest without end: rivers and lakes, reed marshes and tarns, and the jungle closing over the trails."),
                shape,
                paint: Paint::Wild(Wild {
                    meadow: rgb(70, 140, 46),
                    grass: rgb(82, 128, 48),
                    dry: rgb(138, 138, 74),
                    moss: rgb(52, 98, 40),
                    litter: rgb(70, 64, 40),
                    reeds: rgb(104, 128, 56),
                    flowers: [rgb(236, 90, 70), rgb(246, 196, 80), rgb(240, 236, 226)],
                    bloom: 0.45,
                    heath_share: 0.15,
                    wetness: 0.15,
                    marsh: 0.75,
                    ..woodland()
                }),
                kit: Kit {
                    trees: vec!["LushTree01", "Lush02", "Lush03", "Lush04", "Lush05", "Lush06"],
                    shrubs: vec!["lushfoliage01", "lushfoliage02", "lushfoliage03", "lushfoliage04", "lushfoliage05", "lushvine01", "lushvine02"],
                    rocks_small: vec!["LushRock01", "LushRock02"],
                    rocks_big: vec![("LushRock03", 2, 2), ("LushRock04", 2, 2), ("LushRock05", 3, 3), ("LushRock06", 4, 4)],
                    metal: vec!["LushMetal1", "LushMetal2", "LushMetal3"],
                    vents: vec!["LushVent01", "LushVent02", "LushVent03"],
                    forest: 0.5,
                    cover: 0.14,
                    glades: 0.45,
                    clearing: 60.0,
                    base_metal: 6,
                    metal_fields: 1.2,
                    ..Kit::default()
                },
                tracks: true,
                wind: (300, 1500),
                tidal: 25,
                solar: 120,
                surface_metal: 3,
                budget: 0,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        // ---- Deserts --------------------------------------------------------------------------
        "sunscar-dunes" => {
            let shape = Shape {
                w: cells(12),
                h: cells(12),
                seed: 0x5C4A_D11E,
                sea: 22.0,
                levels: vec![40.0, 92.0, 150.0],
                steps: vec![0.56, 0.84],
                hills: 6.0,
                hill_scale: 50.0,
                peaks: 0.0,
                scale: 1.8,
                players: 2,
                symmetric: true,
                ramps: 0.05,
                guide: vec![(0.17, 0.17, 0.15, 1.0)],
                dunes: (30.0, 28.0, 0.35),
                pools: (1, 7.0),
                connect: true,
                ..Shape::default()
            };
            Recipe {
                name: "Sunscar Dunes",
                planet: "Desert",
                description: describe(&shape, "A sea of dunes between two rock plateaus, an oasis on either flank and ruins half buried in the sand."),
                shape,
                paint: Paint::Desert(sandstone()),
                kit: Kit { metal_fields: 2.0, ..desert_kit() },
                tracks: true,
                wind: (1200, 3200),
                tidal: 0,
                solar: 200,
                surface_metal: 3,
                budget: 0,
                whole_rock: 0.28,
                moho_metal: 30,
                gravity: 112,
            }
        }
        "redstone-mesas" => {
            let shape = Shape {
                w: cells(20),
                h: cells(20),
                seed: 0x4ED5_70E1,
                sea: 20.0,
                levels: vec![38.0, 96.0, 156.0, 206.0],
                steps: vec![0.46, 0.74, 0.92],
                hills: 5.0,
                hill_scale: 50.0,
                peaks: 0.0,
                river: true,
                dry: true,
                scale: 3.6,
                players: 6,
                symmetric: true,
                axis: std::f32::consts::FRAC_PI_2 * 3.0,
                ramps: -0.12,
                dunes: (18.0, 24.0, 0.6),
                connect: true,
                ..Shape::default()
            };
            Recipe {
                name: "Redstone Mesas",
                planet: "Desert",
                description: describe(&shape, "Red mesas and buttes over a dry river's canyon: talus, desert pavement and the ramps between the heights."),
                shape,
                paint: Paint::Desert(Desert {
                    sand: rgb(216, 166, 100),
                    sand_red: rgb(208, 150, 88),
                    flat: rgb(200, 160, 110),
                    silt: rgb(222, 196, 148),
                    strata: [rgb(184, 98, 58), rgb(212, 144, 92), rgb(160, 80, 52), rgb(226, 180, 128)],
                    cap: rgb(98, 64, 52),
                    dune_gap: 22.0,
                    wind: 0.6,
                    band: 7.5,
                    scrub_share: 0.3,
                    ..sandstone()
                }),
                kit: Kit { ruin_density: 30.0, grove: Vec::new(), metal_fields: 1.5, ..desert_kit() },
                tracks: true,
                wind: (1000, 3000),
                tidal: 0,
                solar: 210,
                surface_metal: 3,
                budget: 0,
                whole_rock: 0.28,
                moho_metal: 30,
                gravity: 112,
            }
        }
        "mirage-basin" => {
            let shape = Shape {
                w: cells(28),
                h: cells(28),
                seed: 0x3122_A5E7,
                sea: 18.0,
                levels: vec![34.0, 80.0, 128.0, 170.0],
                steps: vec![0.52, 0.78, 0.93],
                hills: 6.0,
                hill_scale: 60.0,
                peaks: 0.0,
                river: true,
                dry: true,
                scale: 2.6,
                players: 8,
                symmetric: true,
                axis: std::f32::consts::FRAC_PI_4,
                guide: vec![(0.5, 0.5, 0.2, -0.5)],
                dunes: (26.0, 32.0, 0.2),
                pools: (3, 9.0),
                connect: true,
                ..Shape::default()
            };
            Recipe {
                name: "Mirage Basin",
                planet: "Desert",
                description: describe(&shape, "A great basin of salt pans and dunes ringed by mesas, oases strung along dry river beds."),
                shape,
                paint: Paint::Desert(Desert { salt_share: 0.9, dune_gap: 30.0, wind: 0.2, scrub_share: 0.45, ..sandstone() }),
                kit: desert_kit(),
                tracks: true,
                wind: (1400, 3600),
                tidal: 0,
                solar: 220,
                surface_metal: 3,
                budget: 0,
                whole_rock: 0.28,
                moho_metal: 30,
                gravity: 112,
            }
        }
        // ---- Islands -------------------------------------------------------------------------
        "twin-reefs" => {
            let shape = Shape {
                w: cells(12),
                h: cells(12),
                seed: 0x7E1F_5A11,
                sea: 52.0,
                levels: vec![40.0, 76.0, 112.0],
                steps: vec![0.55, 0.85],
                hills: 10.0,
                hill_scale: 45.0,
                peaks: 20.0,
                ocean: 0.6,
                scale: 1.9,
                players: 2,
                symmetric: true,
                guide: vec![(0.22, 0.22, 0.15, 1.1), (0.5, 0.5, 0.1, -1.0), (0.8, 0.18, 0.05, 0.9)],
                shoals: vec![(0.36, 0.36, 0.64, 0.64, 4.0, -5.0)],
                connect: true,
                causeway: Some(-5.0),
                harbours: 60.0,
                ..Shape::default()
            };
            Recipe {
                name: "Twin Reefs",
                planet: "Archipelago",
                description: describe(&shape, "Two islands face each other across a coral sea, a sand bar to wade between them, islets and a harbour by each base for the fleets."),
                shape,
                paint: Paint::Wild(tropical()),
                kit: Kit { metal_fields: 2.0, ..isle_kit() },
                tracks: true,
                wind: (1100, 3000),
                tidal: 32,
                solar: 180,
                surface_metal: 3,
                // (Its shallows and reefs need more tiles than a duel of land.)
                budget: 5_000,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        "crescent-keys" => {
            let shape = Shape {
                w: cells(20),
                h: cells(20),
                seed: 0x5C3E_7A0B,
                sea: 54.0,
                levels: vec![40.0, 80.0, 118.0, 150.0],
                steps: vec![0.6, 0.84, 0.95],
                hills: 12.0,
                hill_scale: 55.0,
                peaks: 30.0,
                ocean: 0.7,
                scale: 4.6,
                players: 6,
                symmetric: true,
                axis: std::f32::consts::PI,
                guide: vec![(0.5, 0.5, 0.16, -1.0), (0.345, 0.768, 0.075, 1.25), (0.190, 0.500, 0.075, 1.25), (0.345, 0.232, 0.075, 1.25), (0.268, 0.634, 0.045, -1.0), (0.128, 0.715, 0.025, 0.65), (0.267, 0.366, 0.045, -1.0), (0.128, 0.285, 0.025, 0.65), (0.500, 0.232, 0.045, -1.0), (0.500, 0.070, 0.025, 0.65)],
                harbours: 60.0,
                ..Shape::default()
            };
            Recipe {
                name: "Crescent Keys",
                planet: "Archipelago",
                description: describe(&shape, "A ring of wooded keys round a turquoise lagoon, reefs in the shallows and deep water between: a map for fleets."),
                shape,
                paint: Paint::Wild(tropical()),
                kit: isle_kit(),
                tracks: true,
                wind: (1000, 3200),
                tidal: 36,
                solar: 180,
                surface_metal: 3,
                budget: 0,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        "stormbreak-archipelago" => {
            let shape = Shape {
                w: cells(28),
                h: cells(28),
                seed: 0x57B7_EA4C,
                sea: 56.0,
                levels: vec![40.0, 84.0, 124.0, 164.0],
                steps: vec![0.6, 0.83, 0.94],
                hills: 14.0,
                hill_scale: 60.0,
                peaks: 56.0,
                ocean: 0.64,
                scale: 6.0,
                players: 8,
                symmetric: true,
                axis: std::f32::consts::FRAC_PI_4 * 3.0,
                harbours: 60.0,
                ..Shape::default()
            };
            Recipe {
                name: "Stormbreak Archipelago",
                planet: "Archipelago",
                description: describe(&shape, "Islands without number under rocky peaks, reefs and channels between them: fleets, landings and the air."),
                shape,
                paint: Paint::Wild(Wild { outcrops: 0.7, ..tropical() }),
                kit: isle_kit(),
                tracks: true,
                wind: (1400, 3600),
                tidal: 40,
                solar: 160,
                surface_metal: 3,
                budget: 0,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        // TA's "Shore to Shore" (29 x 4: two coasts, a wide ocean between and a few sandy
        // islands) as a map of our own: four bases on each coast, a coast of headlands and bays,
        // coves and river mouths, island chains, and a bank across the ocean.
        "shore-to-shore-reborn" => {
            let shape = Shape {
                w: cells(48),
                h: cells(20),
                seed: 0x5503_E2E0,
                sea: 76.0,
                hills: 10.0,
                hill_scale: 60.0,
                peaks: 80.0,
                players: 8,
                symmetric: true,
                axis: std::f32::consts::PI,
                ramps: 0.05,
                // (Each coast's bases are joined by land, `coasts::build`; across the ocean, by
                // sea and air only.)
                connect: false,
                whole: true,
                harbours: 64.0,
                coasts: Some(Coasts {
                    line: 290.0,
                    wander: 14.0,
                    features: vec![
                        Coastal::Headland { z: 0.04, reach: 56.0, width: 80.0, bend: -16.0 },
                        Coastal::Bay { z: 0.15, depth: 34.0, width: 110.0 },
                        Coastal::Cove { z: 0.205, radius: 18.0, mouth: 13.0 },
                        Coastal::River { z: 0.262, source: (0.05, 0.20), mouth: 26.0 },
                        Coastal::Headland { z: 0.315, reach: 60.0, width: 72.0, bend: 10.0 },
                        Coastal::Cove { z: 0.40, radius: 20.0, mouth: 14.0 },
                        Coastal::Bay { z: 0.50, depth: 46.0, width: 124.0 },
                        Coastal::Lagoon { z: 0.50, length: 92.0, width: 14.0, back: 30.0 },
                        Coastal::River { z: 0.555, source: (0.05, 0.47), mouth: 30.0 },
                        Coastal::Cove { z: 0.67, radius: 19.0, mouth: 13.0 },
                        Coastal::Headland { z: 0.75, reach: 74.0, width: 84.0, bend: 16.0 },
                        Coastal::Cove { z: 0.815, radius: 19.0, mouth: 13.0 },
                        Coastal::Bay { z: 0.915, depth: 40.0, width: 100.0 },
                    ],
                    isles: vec![
                        // Sea stacks off the headlands.
                        Isle { x: 0.245, z: 0.03, r: 6.0, height: 14.0, peak: 0.0, stretch: 1.0, angle: 0.0 },
                        Isle { x: 0.25, z: 0.33, r: 5.0, height: 12.0, peak: 0.0, stretch: 1.0, angle: 0.0 },
                        Isle { x: 0.262, z: 0.775, r: 8.0, height: 16.0, peak: 0.0, stretch: 1.0, angle: 0.0 },
                        // The outer chain, beyond the shelf.
                        Isle { x: 0.30, z: 0.11, r: 24.0, height: 22.0, peak: 12.0, stretch: 1.4, angle: 0.5 },
                        Isle { x: 0.345, z: 0.25, r: 16.0, height: 18.0, peak: 0.0, stretch: 1.3, angle: -0.3 },
                        Isle { x: 0.37, z: 0.64, r: 28.0, height: 26.0, peak: 20.0, stretch: 1.3, angle: -0.4 },
                        Isle { x: 0.31, z: 0.86, r: 20.0, height: 20.0, peak: 8.0, stretch: 1.2, angle: 0.9 },
                        // A cay on the bank.
                        Isle { x: 0.40, z: 0.52, r: 9.0, height: 8.0, peak: 0.0, stretch: 1.6, angle: 0.2 },
                        // The middle: a great island with its peak, and a sentinel to the north (its
                        // twin to the south).
                        Isle { x: 0.5, z: 0.5, r: 52.0, height: 30.0, peak: 60.0, stretch: 1.25, angle: 1.2 },
                        Isle { x: 0.5, z: 0.17, r: 22.0, height: 22.0, peak: 16.0, stretch: 1.5, angle: 0.3 },
                    ],
                    bank: Some(Bank { course: vec![(0.215, 0.50), (0.29, 0.47), (0.37, 0.53), (0.44, 0.49)], half: 26.0, depth: 16.0, channels: vec![(0.425, 15.0)] }),
                    seamounts: vec![(0.40, 0.25, 28.0, 22.0), (0.44, 0.78, 24.0, 24.0)],
                    plain: 13.0,
                    upland: 46.0,
                    highland: 92.0,
                    scarps: (118.0, 205.0),
                    shelf: 18.0,
                    shelf_width: 48.0,
                    abyss: 64.0,
                    bases: vec![(0.62, 54.0), (0.14, 54.0), (0.38, 54.0), (0.86, 54.0)],
                }),
                ..Shape::default()
            };
            Recipe {
                name: "Shore to Shore Reborn",
                planet: "Archipelago",
                description: describe(&shape, "Two coasts face each other across a wide ocean: headlands and beaches, coves and river mouths, island chains rich in metal, and a shallow bank for hovercraft cut by deep channels for the fleets."),
                shape,
                paint: Paint::Wild(Wild {
                    outcrops: 0.6,
                    reef: 0.5,
                    crisp: true,
                    bands: Some(Bands { shelf: rgb(44, 150, 168), abyss: rgb(9, 34, 86), deep: 21.0, abyss_from: 42.0 }),
                    ..tropical()
                }),
                kit: Kit {
                    forest: 0.45,
                    cover: 0.05,
                    metal_fields: 0.8,
                    corridor: true,
                    isle_metal: vec!["ArchMetal3", "ArchMetalB3", "ArchMetalC3"],
                    isle_metal_every: 700.0,
                    isle_vent: 900,
                    tight: true,
                    reef_density: 420.0,
                    ..isle_kit()
                },
                tracks: true,
                wind: (1400, 3800),
                tidal: 36,
                solar: 150,
                surface_metal: 3,
                // (Its shelves of coral and the bank need more tiles than land of its size: the
                // most a map may have.)
                budget: 24_000,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        // ---- Lava ----------------------------------------------------------------------------
        "embergate" => {
            let shape = Shape {
                w: cells(12),
                h: cells(12),
                seed: 0xE3B0_6A7E,
                sea: 34.0,
                levels: vec![50.0, 88.0, 124.0],
                steps: vec![0.4, 0.78],
                hills: 8.0,
                hill_scale: 40.0,
                peaks: 30.0,
                river: true,
                lakes: 1,
                fords: false,
                scale: 1.6,
                players: 2,
                symmetric: true,
                guide: vec![(0.17, 0.17, 0.14, 0.9)],
                shoals: vec![(0.58, 0.3, 0.7, 0.42, 4.0, 4.0)],
                connect: true,
                causeway: Some(4.0),
                ..Shape::default()
            };
            Recipe {
                name: "Embergate",
                planet: "Lava",
                description: describe(&shape, "Two basalt plateaus over a river of fire and its molten lake, crossed by bridges of rock."),
                shape,
                paint: Paint::Volcanic(basalt()),
                kit: Kit { metal_fields: 2.0, ..lava_kit(120.0) },
                tracks: true,
                wind: (200, 1400),
                tidal: 0,
                solar: 110,
                surface_metal: 4,
                budget: 0,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        "furnace-caldera" => {
            let shape = Shape {
                w: cells(20),
                h: cells(20),
                seed: 0xCA1D_E7A5,
                sea: 36.0,
                levels: vec![56.0, 96.0, 136.0, 172.0],
                steps: vec![0.2, 0.56, 0.86],
                hills: 10.0,
                hill_scale: 50.0,
                peaks: 36.0,
                ocean: 0.07,
                scale: 2.2,
                players: 6,
                symmetric: true,
                axis: std::f32::consts::FRAC_PI_2 * 3.0,
                rings: vec![(0.5, 0.5, 0.21, 0.06, 1.7)],
                guide: vec![(0.5, 0.5, 0.11, -1.6)],
                connect: true,
                causeway: Some(4.0),
                ..Shape::default()
            };
            Recipe {
                name: "Furnace Caldera",
                planet: "Lava",
                description: describe(&shape, "A volcano's great crater, its molten lake ringed by the rim, terraces of basalt and ash falling away to seas of fire."),
                shape,
                paint: Paint::Volcanic(Volcanic { cinder_share: 0.4, ..basalt() }),
                kit: lava_kit(160.0),
                tracks: true,
                wind: (300, 1600),
                tidal: 0,
                solar: 100,
                surface_metal: 4,
                budget: 0,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        "obsidian-wastes" => {
            let shape = Shape {
                w: cells(28),
                h: cells(28),
                seed: 0x0B51_D1A5,
                sea: 38.0,
                levels: vec![60.0, 100.0, 140.0],
                steps: vec![0.55, 0.85],
                hills: 10.0,
                hill_scale: 55.0,
                peaks: 46.0,
                ocean: 0.4,
                scale: 3.0,
                players: 8,
                symmetric: true,
                axis: std::f32::consts::FRAC_PI_4,
                connect: true,
                causeway: Some(4.0),
                ..Shape::default()
            };
            Recipe {
                name: "Obsidian Wastes",
                planet: "Lava",
                description: describe(&shape, "A continent of basalt in a sea of fire, its arms reaching out across it, spires on the ash and young flows down the slopes."),
                shape,
                paint: Paint::Volcanic(Volcanic { flow_share: 0.6, ..basalt() }),
                kit: lava_kit(200.0),
                tracks: true,
                wind: (200, 1200),
                tidal: 0,
                solar: 100,
                surface_metal: 4,
                budget: 0,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        // ---- Metal ---------------------------------------------------------------------------
        "foundry-deck" => {
            let shape = Shape {
                w: cells(12),
                h: cells(12),
                seed: 0xF0D1_DECC,
                sea: 40.0,
                players: 2,
                symmetric: true,
                connect: true,
                decks: Some(Decks { block: 32, levels: vec![64.0, 96.0, 128.0], floor: 22.0, channels: 0.2, bunkers: 0.35, bunker_h: 30.0, ramps: 0.65 }),
                ..Shape::default()
            };
            Recipe {
                name: "Foundry Deck",
                planet: "Metal",
                description: describe(&shape, "Decks of plating over coolant channels, ramps between the levels and bunkers to fight from."),
                shape,
                paint: Paint::Metal(steel()),
                kit: metal_kit(60.0, 120.0),
                tracks: false,
                wind: (0, 3000),
                // (No tides: the coolant is 18 deep, a tidal generator needs 20.)
                tidal: 0,
                solar: 100,
                surface_metal: 255,
                // (Its hazard stripes and plating need more tiles than a duel of land.)
                budget: 5_000,
                whole_rock: 0.0,
                moho_metal: 255,
                gravity: 112,
            }
        }
        "reactor-grid" => {
            let shape = Shape {
                w: cells(20),
                h: cells(20),
                seed: 0x5EAC_7012,
                sea: 40.0,
                players: 6,
                symmetric: true,
                axis: std::f32::consts::PI,
                connect: true,
                decks: Some(Decks { block: 40, levels: vec![60.0, 92.0], floor: 22.0, channels: 0.12, bunkers: 0.55, bunker_h: 34.0, ramps: 0.75 }),
                ..Shape::default()
            };
            Recipe {
                name: "Reactor Grid",
                planet: "Metal",
                description: describe(&shape, "A reactor's grid of armoured blocks on two levels of plating, cut by coolant canals."),
                shape,
                paint: Paint::Metal(Metal { rust_share: 0.3, ..steel() }),
                kit: metal_kit(80.0, 100.0),
                tracks: false,
                wind: (0, 3000),
                // (No tides: the coolant is 18 deep, a tidal generator needs 20.)
                tidal: 0,
                solar: 100,
                surface_metal: 255,
                budget: 0,
                whole_rock: 0.0,
                moho_metal: 255,
                gravity: 112,
            }
        }
        "hammerfall-works" => {
            let shape = Shape {
                w: cells(28),
                h: cells(28),
                seed: 0x4A3F_A11E,
                sea: 40.0,
                players: 8,
                symmetric: true,
                axis: std::f32::consts::FRAC_PI_4 * 3.0,
                connect: true,
                decks: Some(Decks { block: 32, levels: vec![56.0, 88.0, 120.0, 152.0], floor: 20.0, channels: 0.2, bunkers: 0.25, bunker_h: 30.0, ramps: 0.55 }),
                ..Shape::default()
            };
            Recipe {
                name: "Hammerfall Works",
                planet: "Metal",
                description: describe(&shape, "A world-forge of decks stacked four high over coolant canals, towers and pipework on every level."),
                shape,
                paint: Paint::Metal(Metal { rust_share: 0.7, grime_share: 0.85, ..steel() }),
                kit: metal_kit(100.0, 160.0),
                tracks: false,
                wind: (0, 3500),
                tidal: 22,
                solar: 90,
                surface_metal: 255,
                budget: 0,
                whole_rock: 0.0,
                moho_metal: 255,
                gravity: 112,
            }
        }
        // ---- Space ---------------------------------------------------------------------------
        "rift-belt" => {
            let shape = Shape {
                w: cells(12),
                h: cells(12),
                seed: 0x41F7_BE17,
                sea: 0.0,
                levels: vec![60.0, 92.0, 120.0],
                steps: vec![0.72, 0.9],
                hills: 12.0,
                hill_scale: 40.0,
                peaks: 0.0,
                ocean: 0.72,
                scale: 3.0,
                players: 2,
                symmetric: true,
                guide: vec![(0.25, 0.25, 0.12, 1.8), (0.74, 0.2, 0.05, 1.4), (0.44, 0.58, 0.045, 1.3), (0.5, 0.5, 0.1, -0.8)],
                shoals: vec![(0.34, 0.24, 0.7, 0.2, 3.0, 0.0)],
                space: true,
                craters: (140.0, 18.0),
                connect: true,
                causeway: Some(84.0),
                ..Shape::default()
            };
            Recipe {
                name: "Rift Belt",
                planet: "Asteroid Belt",
                description: describe(&shape, "Two rocks drifting in the void, joined by gantries, outcrops for the bold and the stars below."),
                shape,
                paint: Paint::Space(regolith()),
                kit: Kit { metal_fields: 2.2, ..moon_kit(80.0) },
                tracks: true,
                wind: (0, 0),
                tidal: 0,
                solar: 30,
                surface_metal: 3,
                budget: 5_000,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 60,
            }
        }
        "lagrange-station" => {
            let shape = Shape {
                w: cells(20),
                h: cells(20),
                seed: 0x7A62_A57E,
                sea: 0.0,
                levels: vec![60.0, 96.0, 128.0],
                steps: vec![0.74, 0.9],
                hills: 12.0,
                hill_scale: 50.0,
                peaks: 0.0,
                ocean: 0.74,
                scale: 4.0,
                players: 6,
                symmetric: true,
                axis: std::f32::consts::PI,
                guide: vec![(0.5, 0.5, 0.08, 1.8), (0.335, 0.786, 0.085, 1.8), (0.170, 0.500, 0.085, 1.8), (0.335, 0.214, 0.085, 1.8)],
                shoals: vec![(0.5, 0.5, 0.335, 0.786, 3.0, 0.0), (0.5, 0.5, 0.170, 0.500, 3.0, 0.0), (0.5, 0.5, 0.335, 0.214, 3.0, 0.0)],
                space: true,
                craters: (110.0, 22.0),
                connect: true,
                causeway: Some(90.0),
                ..Shape::default()
            };
            Recipe {
                name: "Lagrange Station",
                planet: "Asteroid Belt",
                description: describe(&shape, "Six rocks round a station at the balance point, gantries spoked out to them across the void."),
                shape,
                paint: Paint::Space(Space { platform: 16.0, ..regolith() }),
                kit: moon_kit(90.0),
                tracks: true,
                wind: (0, 0),
                tidal: 0,
                solar: 30,
                surface_metal: 3,
                budget: 0,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 60,
            }
        }
        "starfall-expanse" => {
            let shape = Shape {
                w: cells(28),
                h: cells(28),
                seed: 0x57A2_FA11,
                sea: 0.0,
                levels: vec![56.0, 90.0, 124.0, 150.0],
                steps: vec![0.72, 0.86, 0.95],
                hills: 14.0,
                hill_scale: 56.0,
                peaks: 30.0,
                ocean: 0.68,
                scale: 6.0,
                players: 8,
                symmetric: true,
                axis: std::f32::consts::FRAC_PI_4 * 3.0,
                space: true,
                craters: (120.0, 26.0),
                connect: true,
                causeway: Some(96.0),
                ..Shape::default()
            };
            Recipe {
                name: "Starfall Expanse",
                planet: "Asteroid Belt",
                description: describe(&shape, "A field of rocks without number adrift in the void, a web of gantries between them, and the stars all round."),
                shape,
                paint: Paint::Space(regolith()),
                kit: moon_kit(110.0),
                tracks: true,
                wind: (0, 0),
                tidal: 0,
                solar: 30,
                surface_metal: 3,
                budget: 0,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 60,
            }
        }
        "frostfall-hollow" => {
            let shape = Shape {
                w: cells(16),
                h: cells(16),
                seed: 0x5F05_7FA1,
                sea: 34.0,
                levels: vec![54.0, 90.0, 126.0],
                steps: vec![0.42, 0.76],
                hills: 10.0,
                hill_scale: 52.0,
                peaks: 30.0,
                river: true,
                lakes: 1,
                fords: false,
                scale: 2.0,
                players: 2,
                symmetric: true,
                ramps: 0.1,
                connect: true,
                whole: true,
                frozen: true,
                ..Shape::default()
            };
            let budget = winter_budget(shape.w, shape.h);
            Recipe {
                name: "Frostfall Hollow",
                planet: "Ice",
                description: describe(&shape, "Christmas in a snowbound valley: a frozen river and its pond, two villages round their great firs, lanterns lit along the roads through the pines."),
                shape,
                paint: Paint::Snow(winter()),
                kit: winter_kit(0.4, 0.05, christmas(7.6)),
                tracks: true,
                wind: (1800, 4200),
                // (No tides: the water is frozen.)
                tidal: 0,
                solar: 25,
                surface_metal: 3,
                budget,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        "hollyridge" => {
            let shape = Shape {
                w: cells(20),
                h: cells(20),
                seed: 0x4011_7D6E,
                sea: 32.0,
                levels: vec![54.0, 92.0, 132.0, 170.0],
                steps: vec![0.38, 0.68, 0.88],
                hills: 12.0,
                hill_scale: 60.0,
                peaks: 40.0,
                river: true,
                lakes: 2,
                fords: false,
                scale: 2.2,
                players: 4,
                symmetric: true,
                axis: std::f32::consts::PI,
                ramps: 0.08,
                connect: true,
                whole: true,
                frozen: true,
                ..Shape::default()
            };
            let budget = winter_budget(shape.w, shape.h);
            Recipe {
                name: "Hollyridge",
                planet: "Ice",
                description: describe(&shape, "Four bases on the snowy ridges, a frozen river winding through the vale below, villages along its banks hung with lights for Christmas."),
                shape,
                paint: Paint::Snow(Snow { crust: 0.55, ..winter() }),
                kit: winter_kit(0.42, 0.07, christmas(10.0)),
                tracks: true,
                wind: (1800, 4400),
                // (No tides: the water is frozen.)
                tidal: 0,
                solar: 25,
                surface_metal: 3,
                budget,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        "midnight-square" => {
            let shape = Shape {
                w: cells(16),
                h: cells(16),
                seed: 0x3D1D_2F00,
                sea: 32.0,
                levels: vec![62.0, 98.0, 132.0],
                steps: vec![0.5, 0.82],
                hills: 7.0,
                hill_scale: 48.0,
                peaks: 20.0,
                scale: 2.0,
                players: 2,
                symmetric: true,
                guide: vec![(0.5, 0.5, 0.16, -0.9)],
                pools: (3, 12.0),
                ramps: 0.1,
                connect: true,
                whole: true,
                frozen: true,
                ..Shape::default()
            };
            let budget = winter_budget(shape.w, shape.h);
            Recipe {
                name: "Midnight Square",
                planet: "Ice",
                description: describe(&shape, "A town in the snow waits for the New Year: its clock tower in the square between the bases, fireworks ready round it, lanterns strung over the lanes."),
                shape,
                paint: Paint::Snow(Snow { crust: 0.35, ..winter() }),
                kit: winter_kit(0.36, 0.04, new_year(7.6, true)),
                tracks: true,
                wind: (1600, 3800),
                // (No tides: the water is frozen.)
                tidal: 0,
                solar: 25,
                surface_metal: 3,
                budget,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        "first-light-peaks" => {
            let shape = Shape {
                w: cells(24),
                h: cells(24),
                seed: 0x1F57_1C47,
                sea: 34.0,
                levels: vec![56.0, 94.0, 134.0, 176.0],
                steps: vec![0.36, 0.66, 0.88],
                hills: 13.0,
                hill_scale: 64.0,
                peaks: 56.0,
                river: true,
                lakes: 3,
                fords: false,
                scale: 2.6,
                players: 6,
                symmetric: true,
                axis: std::f32::consts::PI,
                ramps: 0.08,
                connect: true,
                whole: true,
                frozen: true,
                ..Shape::default()
            };
            let budget = winter_budget(shape.w, shape.h);
            Recipe {
                name: "First Light Peaks",
                planet: "Ice",
                description: describe(&shape, "Mountain villages ring in the New Year over three frozen lakes: clock towers and fireworks in their squares, the passes lit with lanterns."),
                shape,
                paint: Paint::Snow(Snow { crust: 0.5, rock_from: 6.5, ..winter() }),
                kit: winter_kit(0.4, 0.06, new_year(8.0, false)),
                tracks: true,
                wind: (2000, 4600),
                // (No tides: the water is frozen.)
                tidal: 0,
                solar: 25,
                surface_metal: 3,
                budget,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        "australia" => {
            let shape = Shape {
                w: cells(28),
                h: cells(24),
                seed: 0xA057_4A11,
                sea: 34.0,
                levels: vec![44.0, 70.0, 100.0, 136.0, 172.0],
                steps: vec![0.2, 0.32, 0.46, 0.6],
                hills: 9.0,
                hill_scale: 60.0,
                peaks: 30.0,
                ramps: 0.12,
                players: 8,
                connect: true,
                whole: true,
                geo: Some(Geo {
                    land: include_bytes!("../../geo/australia_land.png"),
                    lakes: include_bytes!("../../geo/australia_lakes.png"),
                    relief: include_bytes!("../../geo/australia_relief.png"),
                    biome: include_bytes!("../../geo/australia_biome.png"),
                    warmth: include_bytes!("../../geo/australia_warmth.png"),
                    // Perth, Brisbane, Darwin, Melbourne, Cairns, Adelaide, Port Hedland, Sydney.
                    starts: vec![(0.1250, 0.6507), (0.8917, 0.5360), (0.4396, 0.1413), (0.7208, 0.8027), (0.7417, 0.2587), (0.5979, 0.7280), (0.1854, 0.3547), (0.8500, 0.6987)],
                    dry_lakes: true,
                    snow_line: 205.0,
                }),
                // (Ports: the bases on the coast have harbours.)
                harbours: 56.0,
                ..Shape::default()
            };
            Recipe {
                name: "Australia",
                planet: "Earth",
                description: describe(&shape, "The island continent: the red centre's deserts and salt lakes, the tropical north, the Great Dividing Range and the bases round the coasts."),
                shape,
                paint: Paint::Earth(Box::new(earth())),
                kit: Kit { corridor: true, ..earth_kit(0.55) },
                tracks: true,
                wind: (1200, 3600),
                tidal: 20,
                solar: 120,
                surface_metal: 3,
                budget: 0,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        "united-states" => {
            let shape = Shape {
                w: cells(32),
                h: cells(18),
                seed: 0x0005_A1A8,
                sea: 34.0,
                levels: vec![44.0, 70.0, 100.0, 136.0, 172.0, 204.0],
                steps: vec![0.12, 0.23, 0.36, 0.52, 0.7],
                hills: 10.0,
                hill_scale: 60.0,
                peaks: 40.0,
                ramps: 0.12,
                players: 8,
                connect: true,
                whole: true,
                geo: Some(Geo {
                    land: include_bytes!("../../geo/usa_land.png"),
                    lakes: include_bytes!("../../geo/usa_lakes.png"),
                    relief: include_bytes!("../../geo/usa_relief.png"),
                    biome: include_bytes!("../../geo/usa_biome.png"),
                    warmth: include_bytes!("../../geo/usa_warmth.png"),
                    // Seattle, Miami, Los Angeles, New York, Denver, Chicago, Houston, Minneapolis.
                    starts: vec![(0.0623, 0.1148), (0.7475, 0.9074), (0.1328, 0.6074), (0.8443, 0.3556), (0.3475, 0.4000), (0.6230, 0.3222), (0.4984, 0.7630), (0.5344, 0.2037)],
                    dry_lakes: false,
                    snow_line: 238.0,
                }),
                // (Ports: the bases on the coast have harbours.)
                harbours: 56.0,
                ..Shape::default()
            };
            Recipe {
                name: "United States",
                planet: "Earth",
                description: describe(&shape, "From sea to shining sea: the Rockies and the Sierra, the south-west's deserts, the Great Plains, the Great Lakes and the forests of the east."),
                shape,
                paint: Paint::Earth(Box::new(earth())),
                kit: Kit { corridor: true, ..earth_kit(0.55) },
                tracks: true,
                wind: (900, 3000),
                tidal: 20,
                solar: 90,
                surface_metal: 3,
                budget: 19_000,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        "the-world" => {
            let shape = Shape {
                w: cells(40),
                h: cells(20),
                seed: 0x0E47_7400,
                sea: 34.0,
                levels: vec![44.0, 70.0, 100.0, 136.0, 172.0, 204.0],
                steps: vec![0.12, 0.24, 0.38, 0.54, 0.74],
                hills: 9.0,
                hill_scale: 50.0,
                peaks: 36.0,
                ramps: 0.1,
                players: 10,
                connect: false,
                whole: true,
                geo: Some(Geo {
                    land: include_bytes!("../../geo/world_land.png"),
                    lakes: include_bytes!("../../geo/world_lakes.png"),
                    relief: include_bytes!("../../geo/world_relief.png"),
                    biome: include_bytes!("../../geo/world_biome.png"),
                    warmth: include_bytes!("../../geo/world_warmth.png"),
                    // Washington, Beijing, Paris, Sydney, Sao Paulo, Moscow, Lagos, Delhi, Los
                    // Angeles, Nairobi.
                    starts: vec![(0.2833, 0.4016), (0.8222, 0.3984), (0.5069, 0.3368), (0.9167, 0.8157), (0.3694, 0.7535), (0.6042, 0.2871), (0.5111, 0.5848), (0.7139, 0.4660), (0.1736, 0.4310), (0.6022, 0.6340)],
                    dry_lakes: false,
                    snow_line: 205.0,
                }),
                // (Ports: the bases on the coast have harbours.)
                harbours: 56.0,
                ..Shape::default()
            };
            Recipe {
                name: "The World",
                planet: "Earth",
                description: describe(&shape, "The whole world from the Arctic's ice to the Southern Ocean: the deserts and the rainforests, a base on every inhabited continent, the oceans between them for fleets."),
                shape,
                paint: Paint::Earth(Box::new(earth())),
                kit: earth_kit(0.4),
                tracks: false,
                wind: (800, 3200),
                tidal: 25,
                solar: 100,
                surface_metal: 3,
                budget: 0,
                whole_rock: 0.0,
                moho_metal: 30,
                gravity: 112,
            }
        }
        _ => return None,
    };
    let budget = if r.budget > 0 { r.budget } else { budget(r.shape.w, r.shape.h) };
    let whole_rock = if r.whole_rock > 0.0 { r.whole_rock } else { super::tiles::WHOLE_SHARE };
    Some(Recipe { budget, whole_rock, ..r })
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_map_has_a_recipe_and_a_name_of_its_own() {
        let mut names = std::collections::HashSet::new();
        for id in super::IDS {
            let r = super::recipe(id).unwrap_or_else(|| panic!("{id}"));
            assert!(names.insert(r.name), "{} twice", r.name);
            assert!(r.budget > 1000 && r.budget <= 24_000, "{id}: {}", r.budget);
            assert!(r.description.starts_with(&format!("{} X {}  ", r.shape.w / 32, r.shape.h / 32)), "{id}: {}", r.description);
            assert_eq!(r.shape.w % 32, 0, "{id}");
            assert_eq!(r.shape.h % 32, 0, "{id}");
        }
    }
}
