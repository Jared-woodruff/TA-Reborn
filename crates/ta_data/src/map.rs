//! Map metadata from `maps/<name>.ota`.

use ta_formats::tdf::{TdfDocument, TdfSection};

/// One game-type schema of a map (TA maps define several: skirmish/network variants).
#[derive(Clone, Debug, Default)]
pub struct MapSchema {
    pub kind: String,
    pub ai_profile: String,
    pub surface_metal: f64,
    pub moho_metal: f64,
    pub human_metal: f64,
    pub computer_metal: f64,
    pub human_energy: f64,
    pub computer_energy: f64,
    pub meteor_weapon: String,
    pub meteor_radius: f64,
    pub meteor_density: f64,
    pub meteor_duration: f64,
    pub meteor_interval: f64,
    /// Start positions by player slot (1-based in TA; stored in slot order).
    pub start_positions: Vec<(u32, f32, f32)>,
    /// Extra units/features placed by the schema (missions).
    pub placed: Vec<PlacedObject>,
}

#[derive(Clone, Debug, Default)]
pub struct PlacedObject {
    pub what: String,
    pub x: f32,
    pub z: f32,
    pub player: u32,
    pub ident: String,
}

#[derive(Clone, Debug, Default)]
pub struct MapInfo {
    pub name: String,
    pub description: String,
    pub planet: String,
    pub tidal_strength: f64,
    pub solar_strength: f64,
    pub lava_world: bool,
    pub min_wind: f64,
    pub max_wind: f64,
    pub gravity: f64,
    pub kill_mul: f64,
    pub num_players: String,
    pub size: String,
    pub water_does_damage: bool,
    pub water_damage: f64,
    /// Its art was painted by TA Reborn's own tools (`rebornart=1`: a map converted from
    /// Supreme Commander), not by TA's painters: the renderer cleans no painter's hatching from it.
    pub reborn_art: bool,
    pub schemas: Vec<MapSchema>,
}

impl MapInfo {
    pub fn from_ota(doc: &TdfDocument) -> MapInfo {
        let Some(h) = doc.section("GlobalHeader") else { return MapInfo::default() };
        let f = |k: &str, d: f64| h.get_f64_or(k, d);
        let st = |k: &str| h.get(k).unwrap_or("").trim().to_string();
        let schemas = h
            .sections
            .iter()
            .filter(|s| s.name.to_ascii_lowercase().starts_with("schema"))
            .map(parse_schema)
            .collect();
        MapInfo {
            name: st("missionname"),
            description: st("missiondescription"),
            planet: st("planet"),
            tidal_strength: f("tidalstrength", 0.0),
            solar_strength: f("solarstrength", 0.0),
            lava_world: h.get_bool_or("lavaworld", false),
            min_wind: f("minwindspeed", 0.0),
            max_wind: f("maxwindspeed", 0.0),
            gravity: f("gravity", 112.0),
            kill_mul: f("killmul", 50.0),
            num_players: st("numplayers"),
            size: st("size"),
            water_does_damage: h.get_bool_or("waterdoesdamage", false),
            water_damage: f("waterdamage", 0.0),
            reborn_art: h.get_bool_or("rebornart", false),
            schemas,
        }
    }
}

fn parse_schema(s: &TdfSection) -> MapSchema {
    let f = |k: &str, d: f64| s.get_f64_or(k, d);
    let mut schema = MapSchema {
        kind: s.get_str("Type", ""),
        ai_profile: s.get_str("aiprofile", "DEFAULT"),
        surface_metal: f("SurfaceMetal", 0.0),
        moho_metal: f("MohoMetal", 0.0),
        human_metal: f("HumanMetal", 1000.0),
        computer_metal: f("ComputerMetal", 1000.0),
        human_energy: f("HumanEnergy", 1000.0),
        computer_energy: f("ComputerEnergy", 1000.0),
        meteor_weapon: s.get_str("MeteorWeapon", ""),
        meteor_radius: f("MeteorRadius", 0.0),
        meteor_density: f("MeteorDensity", 0.0),
        meteor_duration: f("MeteorDuration", 0.0),
        meteor_interval: f("MeteorInterval", 0.0),
        ..Default::default()
    };
    for group in ["specials", "units", "features"] {
        let Some(list) = s.section(group) else { continue };
        for item in &list.sections {
            let what = item.get("specialwhat").or_else(|| item.get("Unitname")).or_else(|| item.get("featurename")).unwrap_or("");
            let x = item.get_f32_or("XPos", 0.0);
            let z = item.get_f32_or("ZPos", 0.0);
            let lower = what.trim().to_ascii_lowercase();
            if let Some(n) = lower.strip_prefix("startpos").and_then(|n| n.trim().parse::<u32>().ok()) {
                schema.start_positions.push((n, x, z));
            } else if !lower.is_empty() {
                schema.placed.push(PlacedObject {
                    what: what.trim().to_string(),
                    x,
                    z,
                    player: item.get_i32_or("Player", 0).max(0) as u32,
                    ident: item.get_str("Ident", ""),
                });
            }
        }
    }
    schema.start_positions.sort_by_key(|p| p.0);
    schema
}

/// TA's map descriptions open with the size their designer typed ("18 X 18  Battle in the
/// Many Mesas area."), now and then wrong; the menus show the map's own size beside them. The
/// text without it, its runs of spaces closed up.
pub fn blurb(description: &str) -> String {
    let text = description.trim();
    let rest = size_prefix_end(text).map_or(text, |i| &text[i..]);
    rest.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where a leading "W x H" ends (in either case or as ×, then perhaps "map", and the stops
/// after it).
fn size_prefix_end(s: &str) -> Option<usize> {
    let digits = |s: &str| s.len() - s.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let width = digits(s);
    if width == 0 {
        return None;
    }
    let rest = s[width..].trim_start().strip_prefix(['x', 'X', '×'])?.trim_start();
    let height = digits(rest);
    if height == 0 {
        return None;
    }
    let mut rest = &rest[height..];
    let word = rest.trim_start();
    if word.get(..3).is_some_and(|m| m.eq_ignore_ascii_case("map")) && !word[3..].starts_with(|c: char| c.is_alphanumeric()) {
        rest = &word[3..];
    }
    let rest = rest.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '.' | ',' | ':' | ';' | '-' | '–'));
    Some(s.len() - rest.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_descriptions_lose_their_typed_size() {
        for (typed, shown) in [
            ("18 X 18  Battle in the Many Mesas area.", "Battle in the Many Mesas area."),
            ("12 x 12  Islands", "Islands"),
            ("11 X 10 map.  Intermittent winds", "Intermittent winds"),
            ("14 X 14, Battle for the Evad River Delta", "Battle for the Evad River Delta"),
            ("9 X 9   Many  spaces", "Many spaces"),
            ("23 X 21  4 Symmetrical Metal Islands", "4 Symmetrical Metal Islands"),
            ("20 X 20 Great map for team play.  Light side vs. dark side.", "Great map for team play. Light side vs. dark side."),
            ("Fight in the open.", "Fight in the open."),
            ("2 v 2 on islands", "2 v 2 on islands"),
            ("10 Xenon pits", "10 Xenon pits"),
            ("", ""),
            ("16 X 16", ""),
        ] {
            assert_eq!(blurb(typed), shown, "{typed:?}");
        }
    }
}
