//! A Supreme Commander map as the converter needs it: from its scenario file (its name, size,
//! armies and files), its save file (where the armies start, the mass and hydrocarbon deposits,
//! the civilians' buildings, walls and wrecks, the areas a script may limit the battle to) and
//! its script (whether it does), and the map file itself ([`crate::scmap`]).

use std::path::{Path, PathBuf};

use crate::files::Files;
use crate::lua::{self, Table, Value};
use crate::scmap::ScMap;
use crate::{Error, Result};

/// What the scenario says of a map: enough to list it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scenario {
    /// The map's folder's name (`SCMP_001`, `Setons Clutch.v0004`).
    pub folder: String,
    pub name: String,
    /// Its description, without the localisation's key (`<LOC SCMP_001_Description>`).
    pub description: String,
    /// `skirmish` or `campaign` (the game's own maps), or what a player's map says.
    pub kind: String,
    /// In world units.
    pub size: (u32, u32),
    /// The armies its players take, in order (`ARMY_1`...).
    pub armies: Vec<String>,
    /// The armies the civilians are (`ARMY_9 NEUTRAL_CIVILIAN`).
    pub civilians: Vec<String>,
    /// The map file, the save file and the script, as the scenario names them.
    pub map: String,
    pub save: String,
    pub script: String,
}

impl Scenario {
    /// Whether players can fight on it in a skirmish: the game's skirmish maps, and players' own
    /// maps (which are made for it) with two armies or more.
    pub fn skirmish(&self) -> bool {
        self.armies.len() >= 2 && !self.kind.eq_ignore_ascii_case("campaign") && !self.kind.eq_ignore_ascii_case("campaign_coop")
    }
}

/// Something the civilians have on a map: a building, a wall, or a wreck.
#[derive(Clone, Debug, PartialEq)]
pub struct Civilian {
    /// Its unit's blueprint id (`uec1101`, `ueb5101`).
    pub unit: String,
    pub position: [f32; 2],
    /// A wreck (the save's `WRECKAGE` group), not a standing building.
    pub wreck: bool,
}

/// A map, read.
#[derive(Clone, Debug, Default)]
pub struct Map {
    pub scenario: Scenario,
    /// Where each army starts (world units), in `scenario.armies`' order.
    pub starts: Vec<[f32; 2]>,
    pub mass: Vec<[f32; 2]>,
    pub hydrocarbons: Vec<[f32; 2]>,
    pub civilians: Vec<Civilian>,
    /// The part of the map the battle is limited to (x0, z0, x1, z1), when its script limits it.
    pub playable: Option<[f32; 4]>,
    pub scmap: ScMap,
}

/// The scenario file in map folder `folder` (`<name>_scenario.lua`, whatever its case).
pub fn scenario_file(folder: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(folder).ok()?.flatten().map(|e| e.path()).filter(|p| {
        p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().ends_with("_scenario.lua"))
    }).collect();
    found.sort();
    found.into_iter().next()
}

/// Reads the scenario of map folder `folder`.
pub fn read_scenario(folder: &Path) -> Result<Scenario> {
    let file = scenario_file(folder).ok_or_else(|| Error::Missing(format!("{}: no scenario file", folder.display())))?;
    let text = read_text(&file)?;
    let statements = lua::parse(&text).map_err(|e| Error::Format(format!("{}: {e}", file.display())))?;
    let info = lua::top(&statements, "ScenarioInfo").and_then(Value::table).ok_or_else(|| Error::Format(format!("{}: no ScenarioInfo", file.display())))?;
    let s = |key: &str| info.get_ci(key).and_then(Value::str).unwrap_or("").to_string();
    let size = info.get_ci("size").map(Value::numbers).unwrap_or_default();
    let standard = info.path(&["Configurations", "standard"]);
    let armies: Vec<String> = standard
        .and_then(|c| c.get_ci("teams"))
        .and_then(Value::table)
        .map(|teams| {
            teams.array.iter().filter_map(Value::table).flat_map(|team| {
                team.get_ci("armies").and_then(Value::table).map(|a| a.array.iter().filter_map(Value::str).map(str::to_string).collect::<Vec<_>>()).unwrap_or_default()
            }).collect()
        })
        .unwrap_or_default();
    let civilians = standard
        .and_then(|c| c.path(&["customprops"]))
        .and_then(|p| p.get_ci("ExtraArmies"))
        .and_then(Value::str)
        .map(|s| s.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();
    let folder_name = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let name = s("name");
    Ok(Scenario {
        name: if name.trim().is_empty() { folder_name.clone() } else { name.trim().to_string() },
        folder: folder_name,
        description: strip_loc(&s("description")),
        kind: s("type"),
        size: (size.first().copied().unwrap_or(0.0).max(0.0) as u32, size.get(1).copied().unwrap_or(0.0).max(0.0) as u32),
        armies,
        civilians,
        map: s("map"),
        save: s("save"),
        script: s("script"),
    })
}

/// Text without the localisation's key at its start (`<LOC key>text`).
pub fn strip_loc(text: &str) -> String {
    let t = text.trim_start();
    match t.strip_prefix("<LOC").and_then(|rest| rest.find('>').map(|end| &rest[end + 1..])) {
        Some(rest) => rest.trim().to_string(),
        None => text.trim().to_string(),
    }
}

/// A text file the game wrote (UTF-8, or Windows' code page where it is not).
fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    Ok(match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
    })
}

/// A file a map names (`/maps/SCMP_001/SCMP_001_save.lua`), or the one of its folder's ending
/// in `suffix` when the name leads nowhere (a player renamed the folder).
fn locate(files: &Files, folder: &Path, named: &str, suffix: &str) -> Result<Vec<u8>> {
    if !named.is_empty()
        && let Some(bytes) = files.read(named)?
    {
        return Ok(bytes);
    }
    let own = std::fs::read_dir(folder).ok().and_then(|entries| {
        let mut found: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().ends_with(suffix))).collect();
        found.sort();
        found.into_iter().next()
    });
    match own {
        Some(path) => std::fs::read(&path).map_err(|e| Error::io(&path, e)),
        None => Err(Error::Missing(format!("{}: no {suffix} ({named})", folder.display()))),
    }
}

impl Map {
    /// Reads the map in folder `folder` whole: its scenario, save file, script and map file.
    pub fn read(files: &Files, folder: &Path) -> Result<Map> {
        let scenario = read_scenario(folder)?;
        let scmap = ScMap::parse(&locate(files, folder, &scenario.map, ".scmap")?)
            .map_err(|e| Error::Format(format!("{}: {e}", scenario.folder)))?;
        let save_bytes = locate(files, folder, &scenario.save, "_save.lua")?;
        let save_text: String = String::from_utf8(save_bytes).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect());
        let statements = lua::parse(&save_text).map_err(|e| Error::Format(format!("{} save file: {e}", scenario.folder)))?;
        let save = lua::top(&statements, "Scenario").and_then(Value::table).ok_or_else(|| Error::Format(format!("{}: no Scenario in its save file", scenario.folder)))?;
        let mut map = Map { scenario, scmap, ..Default::default() };
        // The markers.
        let empty = Table::default();
        let markers = save.path(&["MasterChain", "_MASTERCHAIN_", "Markers"]).unwrap_or(&empty);
        let mut starts: Vec<Option<[f32; 2]>> = vec![None; map.scenario.armies.len()];
        for (name, marker) in &markers.fields {
            let Some(marker) = marker.table() else { continue };
            let Some(p) = marker.get_ci("position").and_then(Value::vec3) else { continue };
            let at = [p[0] as f32, p[2] as f32];
            let kind = marker.get_ci("type").and_then(Value::str).unwrap_or("");
            let resource = marker.get_ci("resource").and_then(Value::bool).unwrap_or(true);
            if kind.eq_ignore_ascii_case("Mass") && resource {
                map.mass.push(at);
            } else if kind.eq_ignore_ascii_case("Hydrocarbon") && resource {
                map.hydrocarbons.push(at);
            } else if let Some(i) = map.scenario.armies.iter().position(|a| a.eq_ignore_ascii_case(name)) {
                starts[i] = Some(at);
            }
        }
        // (An army without a marker cannot be played: the players are those that have one.)
        let (armies, starts): (Vec<String>, Vec<[f32; 2]>) = map.scenario.armies.iter().cloned().zip(starts).filter_map(|(a, s)| s.map(|s| (a, s))).unzip();
        map.scenario.armies = armies;
        map.starts = starts;
        // The civilians' buildings, walls and wrecks.
        if let Some(armies) = save.path(&["Armies"]) {
            for (army, value) in &armies.fields {
                let civilian = map.scenario.civilians.iter().any(|c| c.eq_ignore_ascii_case(army)) || army.eq_ignore_ascii_case("NEUTRAL_CIVILIAN");
                let player = map.scenario.armies.iter().any(|a| a.eq_ignore_ascii_case(army));
                if !civilian || player {
                    continue;
                }
                if let Some(units) = value.table().and_then(|t| t.get_ci("Units")).and_then(Value::table) {
                    collect_units(units, false, 0, &mut map.civilians);
                }
            }
        }
        // A script that limits the battle to an area (`SetPlayableArea('AREA_1')`).
        if let Ok(script) = locate(files, folder, &map.scenario.script, "_script.lua") {
            let script = String::from_utf8_lossy(&script);
            if let Some(area) = playable_area_named(&script)
                && let Some(rect) = save.path(&["Areas"]).and_then(|areas| areas.get_ci(&area)).and_then(Value::table).and_then(|a| a.get_ci("rectangle")).map(Value::numbers)
                && rect.len() == 4
            {
                let r = [rect[0] as f32, rect[1] as f32, rect[2] as f32, rect[3] as f32];
                if r[2] > r[0] && r[3] > r[1] {
                    map.playable = Some(r);
                }
            }
        }
        Ok(map)
    }
}

/// The units of a save file's group (`GROUP { Units = { ... } }`), its groups' too: wrecks in a
/// group named `WRECKAGE`.
fn collect_units(group: &Table, wreck: bool, depth: usize, out: &mut Vec<Civilian>) {
    if depth > 16 {
        return;
    }
    let Some(units) = group.get_ci("Units").and_then(Value::table) else { return };
    for (name, value) in &units.fields {
        let Some(t) = value.table() else { continue };
        if let Some(unit) = t.get_ci("type").and_then(Value::str) {
            if let Some(p) = t.get_ci("Position").and_then(Value::vec3) {
                out.push(Civilian { unit: unit.to_ascii_lowercase(), position: [p[0] as f32, p[2] as f32], wreck });
            }
        } else {
            collect_units(t, wreck || name.eq_ignore_ascii_case("WRECKAGE"), depth + 1, out);
        }
    }
}

/// The area a map's script limits the battle to (`ScenarioFramework.SetPlayableArea('AREA_1')`).
fn playable_area_named(script: &str) -> Option<String> {
    let at = script.find("SetPlayableArea")?;
    let rest = &script[at..];
    let open = rest.find(['\'', '"'])?;
    let quote = rest.as_bytes()[open] as char;
    let end = rest[open + 1..].find(quote)?;
    let name = &rest[open + 1..open + 1 + end];
    (!name.is_empty() && name.len() < 64).then(|| name.to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SCENARIO: &str = "version = 3\nScenarioInfo = {\n    name = 'Test Ridge',\n    description = '<LOC TEST_Description>Two bases and a ridge.',\n    type = 'skirmish',\n    size = {16, 8},\n    map = '/maps/TEST_001/TEST_001.scmap',\n    save = '/maps/TEST_001/TEST_001_save.lua',\n    script = '/maps/TEST_001/TEST_001_script.lua',\n    Configurations = {\n        ['standard'] = {\n            teams = {\n                { name = 'FFA', armies = {'ARMY_1','ARMY_2','ARMY_3',} },\n            },\n            customprops = {\n                ['ExtraArmies'] = STRING( 'ARMY_9 NEUTRAL_CIVILIAN' ),\n            },\n        },\n    }}\n";

    pub(crate) const SAVE: &str = "Scenario = {\n  Areas = {\n    ['AREA_1'] = { ['rectangle'] = RECTANGLE( 1, 0, 15, 8 ) },\n  },\n  MasterChain = {\n    ['_MASTERCHAIN_'] = {\n      Markers = {\n        ['ARMY_1'] = { ['type'] = STRING( 'Blank Marker' ), ['position'] = VECTOR3( 2.5, 0, 4.5 ) },\n        ['ARMY_2'] = { ['type'] = STRING( 'Blank Marker' ), ['position'] = VECTOR3( 13.5, 0, 4.5 ) },\n        ['Mass 01'] = { ['type'] = STRING( 'Mass' ), ['resource'] = BOOLEAN( true ), ['position'] = VECTOR3( 3.5, 0, 6.5 ) },\n        ['Mass 02'] = { ['type'] = STRING( 'Mass' ), ['resource'] = BOOLEAN( true ), ['position'] = VECTOR3( 12.5, 0, 6.5 ) },\n        ['Hydrocarbon 01'] = { ['type'] = STRING( 'Hydrocarbon' ), ['resource'] = BOOLEAN( true ), ['position'] = VECTOR3( 8, 0, 2 ) },\n        ['Rally Point 01'] = { ['type'] = STRING( 'Rally Point' ), ['position'] = VECTOR3( 8, 0, 6 ) },\n      },\n    },\n  },\n  Armies = {\n    ['ARMY_1'] = { Units = GROUP { Units = { } } },\n    ['NEUTRAL_CIVILIAN'] = {\n      Units = GROUP {\n        Units = {\n          ['INITIAL'] = GROUP { Units = { ['UNIT_1'] = { type = 'UEC1101', Position = { 8.5, 1, 5.5 } } } },\n          ['WRECKAGE'] = GROUP { Units = { ['UNIT_2'] = { type = 'uel0201', Position = { 9.5, 1, 6.5 } } } },\n        },\n      },\n    },\n  },\n}\n";

    /// A map folder of the test's (`TEST_001`, its files in lower case as some of the game's are)
    /// in `maps`.
    pub(crate) fn write_map(maps: &Path) -> PathBuf {
        let folder = maps.join("TEST_001");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("test_001_scenario.lua"), SCENARIO).unwrap();
        std::fs::write(folder.join("test_001_save.lua"), SAVE).unwrap();
        std::fs::write(folder.join("TEST_001_script.lua"), "ScenarioFramework.SetPlayableArea('AREA_1', false)\n").unwrap();
        std::fs::write(folder.join("TEST_001.scmap"), crate::scmap::tests::write(&crate::scmap::tests::sample(60))).unwrap();
        folder
    }

    #[test]
    fn a_map_reads_its_armies_markers_civilians_and_area() {
        let dir = crate::tests::Temp::new("map");
        let maps = dir.0.join("maps");
        let folder = write_map(&maps);
        let scenario = read_scenario(&folder).unwrap();
        assert_eq!(scenario.name, "Test Ridge");
        assert_eq!(scenario.description, "Two bases and a ridge.");
        assert_eq!(scenario.size, (16, 8));
        assert_eq!(scenario.armies, ["ARMY_1", "ARMY_2", "ARMY_3"]);
        assert_eq!(scenario.civilians, ["ARMY_9", "NEUTRAL_CIVILIAN"]);
        assert!(scenario.skirmish());
        let files = Files::of(vec![maps], Vec::new());
        let map = Map::read(&files, &folder).unwrap();
        assert_eq!(map.scenario.armies, ["ARMY_1", "ARMY_2"], "ARMY_3 has no marker: not a player's");
        assert_eq!(map.starts, [[2.5, 4.5], [13.5, 4.5]]);
        assert_eq!(map.mass, [[3.5, 6.5], [12.5, 6.5]]);
        assert_eq!(map.hydrocarbons, [[8.0, 2.0]]);
        assert_eq!(map.civilians, [Civilian { unit: "uec1101".into(), position: [8.5, 5.5], wreck: false }, Civilian { unit: "uel0201".into(), position: [9.5, 6.5], wreck: true }]);
        assert_eq!(map.playable, Some([1.0, 0.0, 15.0, 8.0]));
        assert_eq!(map.scmap.width, 16);
        assert_eq!(strip_loc("Plain"), "Plain");
    }
}
