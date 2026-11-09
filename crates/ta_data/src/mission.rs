//! Campaigns (`camps/*.tdf`) and single-player missions (mission `.ota` files).
//!
//! A mission OTA is a map OTA with extra `GlobalHeader` keys for its briefing and its
//! victory/defeat goals, plus per-difficulty schemas that place units. Placed units carry
//! an `InitialMission` script in a small command language (see [`MissionCommand`]).

use ta_formats::tdf::{self, TdfDocument, TdfSection};
use ta_vfs::Vfs;

/// A campaign: an ordered list of missions for one side.
#[derive(Clone, Debug, Default)]
pub struct Campaign {
    /// File stem, e.g. "arm campaign" (also the key for saved progress).
    pub id: String,
    /// Display title derived from the file name ("Arm Campaign - Core Contingency").
    pub title: String,
    pub side: String,
    pub missions: Vec<CampaignEntry>,
}

#[derive(Clone, Debug, Default)]
pub struct CampaignEntry {
    /// Mission OTA file name, e.g. "AC01.ota".
    pub file: String,
    pub name: String,
}

/// Every campaign in `camps/` (the engine's own `example.tdf` excluded), sorted with the
/// original campaigns first.
pub fn campaigns(vfs: &Vfs) -> Vec<Campaign> {
    let mut out = Vec::new();
    for path in vfs.list("camps", Some("tdf"), false) {
        let stem = path.rsplit('/').next().unwrap_or(&path).trim_end_matches(".tdf").to_string();
        if stem.eq_ignore_ascii_case("example") {
            continue;
        }
        let Some(bytes) = vfs.read_opt(&path) else { continue };
        let doc = tdf::parse_bytes(&bytes);
        let side = doc.section("HEADER").map(|h| h.get_str("campaignside", "")).unwrap_or_default();
        let mut missions: Vec<(u32, CampaignEntry)> = doc
            .sections
            .iter()
            .filter_map(|s| {
                let n = s.name.to_ascii_lowercase().strip_prefix("mission")?.parse::<u32>().ok()?;
                let file = s.get_str("missionfile", "");
                (!file.is_empty()).then(|| (n, CampaignEntry { file, name: s.get_str("missionname", "") }))
            })
            .collect();
        missions.sort_by_key(|m| m.0);
        out.push(Campaign { title: title_case(stem.trim()), id: stem, side, missions: missions.into_iter().map(|m| m.1).collect() });
    }
    // Original campaigns, then Core Contingency, then the rest (Battle Tactics, Krogoth).
    let rank = |c: &Campaign| {
        let id = c.id.to_ascii_lowercase();
        if id == "arm campaign" || id == "core campaign" {
            0
        } else if id.contains("contingency") {
            1
        } else if id.contains("krogoth") {
            2
        } else {
            3
        }
    };
    out.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.id.cmp(&b.id)));
    out
}

fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) if w.len() > 1 || f.is_alphabetic() => f.to_uppercase().collect::<String>() + c.as_str(),
                Some(f) => f.to_string(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A mission objective. The victory goals complete the mission once every one is met (TA's
/// editor: "all the victory conditions must be met for a victory to occur"); any one defeat
/// goal fails it.
#[derive(Clone, Debug, PartialEq)]
pub enum Goal {
    // --- Victory -------------------------------------------------------------------------
    /// Destroy every enemy unit.
    DestroyAllUnits,
    /// Destroy every enemy mobile unit.
    KillAllMobileUnits,
    KillEnemyCommander,
    /// Destroy `count` enemy units of a type.
    KillUnitType { unit: String, count: u32 },
    /// Destroy all enemy units of a type.
    KillAllOfType(String),
    /// Capture a unit of this type.
    CaptureUnitType(String),
    /// Build a unit of this type.
    BuildUnitType(String),
    /// Bring a unit (of a type, or any) within `radius` of a point.
    MoveUnitToRadius { unit: Option<String>, x: f32, z: f32, radius: f32 },
    /// Get a unit of this type across a line (TA's `UnitTypePassesX` and `UnitTypePassesZ`).
    UnitTypePasses { unit: String, line: Line },
    /// Survive this many seconds.
    VictoryTimer(f32),
    // --- Defeat --------------------------------------------------------------------------
    /// Lose every unit.
    AllUnitsKilled,
    CommanderKilled,
    /// Every unit of this type (anyone's) destroyed, e.g. the Gate that must be captured.
    AllUnitsKilledOfType(String),
    /// Lose `count` of your units of this type.
    UnitTypeKilled { unit: String, count: u32 },
    /// Any enemy unit gets across a line (TA's `AnyUnitPassesX` and `AnyUnitPassesZ`).
    AnyUnitPasses(Line),
    /// The mission fails after this many seconds.
    DeathTimer(f32),
}

impl Goal {
    pub fn is_victory(&self) -> bool {
        matches!(
            self,
            Goal::DestroyAllUnits
                | Goal::KillAllMobileUnits
                | Goal::KillEnemyCommander
                | Goal::KillUnitType { .. }
                | Goal::KillAllOfType(_)
                | Goal::CaptureUnitType(_)
                | Goal::BuildUnitType(_)
                | Goal::MoveUnitToRadius { .. }
                | Goal::UnitTypePasses { .. }
                | Goal::VictoryTimer(_)
        )
    }
}

/// A line across the map that a goal's units must (or must not) cross: `x = at` crossed
/// eastward or westward, `z = at` northward or southward. The way is away from the side the
/// units start on (the player's units of the type, or the enemy's), which [`Mission::from_ota`]
/// works out from the placed units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub at: f32,
    pub toward: Toward,
}

impl Line {
    /// Whether the line runs north to south (`x = at`).
    pub fn is_x(&self) -> bool {
        matches!(self.toward, Toward::East | Toward::West)
    }

    /// Whether a point is past the line (on the far side from the start).
    pub fn passed(&self, x: f32, z: f32) -> bool {
        match self.toward {
            Toward::North => z < self.at,
            Toward::South => z > self.at,
            Toward::East => x > self.at,
            Toward::West => x < self.at,
        }
    }
}

/// A compass direction (north is -z, east +x).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toward {
    North,
    South,
    East,
    West,
}

/// One command of a placed unit's `InitialMission` script. A script runs from the start of
/// the mission, one command after another: each starts once the one before is done.
#[derive(Clone, Debug, PartialEq)]
pub enum MissionCommand {
    /// `m x z` (`m x z x z...` gives each point its own move, in turn).
    Move(f32, f32),
    /// `p x z` — a patrol point; consecutive points (`p x z x z...`, or one `p` after another)
    /// form a loop.
    Patrol(f32, f32),
    /// `w n` — do nothing for n seconds.
    Wait(u32),
    /// `wa [ident]` — do nothing until this unit (or the named one) is attacked.
    WaitAttacked(Option<String>),
    /// `a TYPE/name` — attack a named unit/group, else the nearest enemy unit of a type.
    Attack(String),
    /// `o move fire` — standing move orders (0 hold position, 1 maneuver, 2 roam), then fire
    /// orders (0 hold fire, 1 return fire, 2 fire at will). (The move order comes first: TA's
    /// missions set `o 1 2`, the units' own defaults, on about 900 units, and the first number
    /// is never 2; their guns' scripts open fire with the second, `o 0 0, w 1200, o 0 2`.)
    States { fire: u8, movement: u8 },
    /// `g ident` — guard a unit, until it is gone.
    Guard(String),
    /// `i ident` — start aboard that transport.
    Load(String),
    /// `s` — stop.
    Stop,
    /// `b TYPE [count]` — a factory builds that many (one if no count is given);
    /// `b TYPE [1] x z` — a constructor builds one there (`b TYPE` alone: where it stands).
    Build { unit: String, count: u32, at: Option<(f32, f32)> },
    /// `bw n` — build n missiles (a stockpiling weapon's: the Core anti-nuke's).
    BuildWeapons(u32),
    /// `u [x z]` — a transport sets one of its units down there (or where it is).
    Unload(Option<(f32, f32)>),
    /// `d` — the unit self-destructs, at once.
    SelfDestruct,
}

/// Parses an `InitialMission` script: comma-separated commands, each a word (case aside)
/// and its arguments. Unknown words are skipped, as are commands missing their arguments.
/// A unit named by its ident takes the rest of the command (idents may hold spaces).
pub fn parse_initial_mission(script: &str) -> Vec<MissionCommand> {
    parse_script(script).0
}

/// A script's commands, and the pieces of it that are none (TA's missions have a few: a
/// missing comma or letter, a list of unit types after one `a`).
fn parse_script(script: &str) -> (Vec<MissionCommand>, Vec<String>) {
    // The numbers in a command's arguments (stray characters around them aside).
    let numbers = |args: &str| -> Vec<f32> {
        args.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).filter(|t| !t.is_empty()).filter_map(|t| t.parse().ok()).collect()
    };
    let points = |nums: &[f32]| -> Vec<(f32, f32)> { nums.as_chunks::<2>().0.iter().map(|&[x, z]| (x, z)).collect() };
    let whole = |n: f32| n.max(0.0) as u32;
    let mut out = Vec::new();
    let mut unknown = Vec::new();
    for raw in script.split(',') {
        let piece = raw.trim();
        let before = out.len();
        let split = piece.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(piece.len());
        let (word, args) = (piece[..split].to_ascii_lowercase(), piece[split..].trim());
        let nums = numbers(args);
        let ident = (!args.is_empty()).then(|| args.to_string());
        match word.as_str() {
            "m" => out.extend(points(&nums).into_iter().map(|(x, z)| MissionCommand::Move(x, z))),
            "p" => out.extend(points(&nums).into_iter().map(|(x, z)| MissionCommand::Patrol(x, z))),
            "w" => out.extend(nums.first().map(|&n| MissionCommand::Wait(whole(n)))),
            "wa" => out.push(MissionCommand::WaitAttacked(ident)),
            "a" => out.extend(ident.map(MissionCommand::Attack)),
            "o" if nums.len() >= 2 => out.push(MissionCommand::States { movement: nums[0].clamp(0.0, 2.0) as u8, fire: nums[1].clamp(0.0, 2.0) as u8 }),
            "g" => out.extend(ident.map(MissionCommand::Guard)),
            "i" => out.extend(ident.map(MissionCommand::Load)),
            "s" => out.push(MissionCommand::Stop),
            "b" => {
                // The type, then numbers: a count, a point, or both.
                let mut parts = args.splitn(2, char::is_whitespace);
                if let Some(unit) = parts.next().filter(|u| u.starts_with(|c: char| c.is_ascii_alphabetic())) {
                    let (count, at) = match *numbers(parts.next().unwrap_or_default()).as_slice() {
                        [] => (1, None),
                        [n] => (whole(n), None),
                        [x, z] => (1, Some((x, z))),
                        [n, x, z, ..] => (whole(n), Some((x, z))),
                    };
                    out.push(MissionCommand::Build { unit: unit.to_string(), count, at });
                }
            }
            "bw" => out.extend(nums.first().map(|&n| MissionCommand::BuildWeapons(whole(n)))),
            "u" => out.push(MissionCommand::Unload(points(&nums).first().copied())),
            "d" => out.push(MissionCommand::SelfDestruct),
            _ => {}
        }
        if out.len() == before && !piece.is_empty() {
            unknown.push(piece.to_string());
        }
    }
    (out, unknown)
}

/// A unit placed by a mission schema.
#[derive(Clone, Debug, Default)]
pub struct PlacedUnit {
    pub name: String,
    /// Script name other units' commands refer to (may be empty).
    pub ident: String,
    pub x: f32,
    pub z: f32,
    /// TA player number (1 = the human, 2+ = computer players).
    pub player: u32,
    /// 0..1
    pub health: f32,
    /// Degrees; 0 faces south (+z), 90 east.
    pub angle: f32,
    pub kills: u64,
    pub script: Vec<MissionCommand>,
    /// Pieces of its script that are no command (left out).
    pub unknown: Vec<String>,
    /// Nothing harms it (`Immunity=1`: a Gate to capture, Core mission 1's Commander).
    pub immune: bool,
    /// `InitialGroup`: a name its group of units goes by, which scripts refer to as to an
    /// ident (Skirmish on the Sand's `patrol`); a digit puts the player's units in that
    /// control group (Dogfight over Grantham Deep's Vamps in 1, its Finks in 2).
    pub group: String,
    /// `AiIgnore=1`: computer players act as though it weren't there.
    pub ai_ignore: bool,
    /// `AiPriorityTarget=1`: computer players go for it first (Miniement Landing's Shipyard).
    pub ai_priority: bool,
    /// `MissionCriticalUnit=1`: the mission is lost with it (a computer player's: no effect).
    pub critical: bool,
    /// `BuildPriority=TYPE`: its computer player builds that type first (empty: none).
    pub build_priority: String,
    /// `OffMapUnit=1` (no mission sets it; not acted on).
    pub off_map: bool,
    /// `CreationCountdown` in seconds (0 in every mission; not acted on).
    pub creation_countdown: f32,
}

/// One difficulty level of a mission.
#[derive(Clone, Debug, Default)]
pub struct MissionSchema {
    /// "Easy", "Medium" or "Hard".
    pub kind: String,
    pub human_metal: f64,
    pub human_energy: f64,
    pub computer_metal: f64,
    pub computer_energy: f64,
    pub surface_metal: f64,
    pub units: Vec<PlacedUnit>,
    /// The human's start position (camera focus).
    pub start: Option<(f32, f32)>,
}

#[derive(Clone, Debug, Default)]
pub struct Mission {
    /// Mission OTA file name; the map is the TNT of the same stem.
    pub file: String,
    pub name: String,
    pub planet: String,
    /// Briefing text file stem under `camps/briefs/`.
    pub brief: String,
    /// Narration sound stem under `camps/briefs/`.
    pub narration: String,
    /// Briefing picture stem under `bitmaps/glamour/`.
    pub glamour: String,
    /// Sound stem under `camps/briefs/` that goes with the picture when the mission is won
    /// (`glamoursound`: the Core Contingency's last missions, `exp1armvict`).
    pub glamour_sound: String,
    /// Unit whitelist file stem under `camps/useonly/` (empty = no restriction).
    pub use_only: String,
    pub line_of_sight: bool,
    pub mapping: bool,
    /// `maxunits`: how many units each player may own (`None`: the mission sets no limit).
    pub max_units: Option<u32>,
    /// Every one must be met to win (TA's rule); any one of `defeat` loses.
    pub victory: Vec<Goal>,
    pub defeat: Vec<Goal>,
    pub schemas: Vec<MissionSchema>,
}

impl Mission {
    pub fn load(vfs: &Vfs, file: &str) -> Option<Mission> {
        let bytes = vfs.read_opt(&format!("maps/{file}"))?;
        Some(Mission::from_ota(file, &tdf::parse_bytes(&bytes)))
    }

    pub fn from_ota(file: &str, doc: &TdfDocument) -> Mission {
        let Some(h) = doc.section("GlobalHeader") else { return Mission { file: file.into(), ..Default::default() } };
        let st = |k: &str| h.get(k).unwrap_or("").trim().to_string();
        let mut goals = Vec::new();
        let unit_count = |v: &str| -> (String, u32) {
            let mut parts = v.split(',').map(str::trim);
            let unit = parts.next().unwrap_or("").to_string();
            let count = parts.next().and_then(|c| c.parse().ok()).unwrap_or(1);
            (unit, count)
        };
        // A line `x = n` or `z = n`: which way it is crossed is worked out once the units are
        // read (see `crossing`); until then, westward or northward.
        let line = |key: &str, at: f32| Line { at, toward: if key.ends_with('x') { Toward::West } else { Toward::North } };
        let unit_at = |v: &str| -> (String, f32) {
            let mut parts = v.split(',').map(str::trim);
            let unit = parts.next().unwrap_or("").to_string();
            (unit, parts.next().and_then(|c| c.parse().ok()).unwrap_or(0.0))
        };
        for (k, v) in &h.entries {
            let v = v.trim();
            let on = v != "0" && !v.is_empty();
            let num = || v.parse::<f32>().unwrap_or(0.0);
            let key = k.to_ascii_lowercase();
            let goal = match key.as_str() {
                "destroyallunits" if on => Some(Goal::DestroyAllUnits),
                "killallmobileunits" if on => Some(Goal::KillAllMobileUnits),
                "killenemycommander" if on => Some(Goal::KillEnemyCommander),
                "killunittype" if on => {
                    let (unit, count) = unit_count(v);
                    Some(Goal::KillUnitType { unit, count })
                }
                "killalloftype" if on => Some(Goal::KillAllOfType(v.to_string())),
                "captureunittype" if on => Some(Goal::CaptureUnitType(v.to_string())),
                "buildunittype" if on => Some(Goal::BuildUnitType(v.to_string())),
                "moveunittoradius" if on => {
                    let p: Vec<&str> = v.split(',').map(str::trim).collect();
                    let f = |i: usize| p.get(i).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                    let unit = p.first().filter(|u| !u.eq_ignore_ascii_case("anytype")).map(|u| u.to_string());
                    Some(Goal::MoveUnitToRadius { unit, x: f(1), z: f(2), radius: f(3).max(16.0) })
                }
                "unittypepassesx" | "unittypepassesz" if on => {
                    let (unit, at) = unit_at(v);
                    Some(Goal::UnitTypePasses { unit, line: line(&key, at) })
                }
                "victorytimerrunsout" if on => Some(Goal::VictoryTimer(num())),
                "allunitskilled" if on => Some(Goal::AllUnitsKilled),
                "commanderkilled" if on => Some(Goal::CommanderKilled),
                "allunitskilledoftype" if on => Some(Goal::AllUnitsKilledOfType(v.to_string())),
                "unittypekilled" if on => {
                    let (unit, count) = unit_count(v);
                    Some(Goal::UnitTypeKilled { unit, count })
                }
                "anyunitpassesx" | "anyunitpassesz" if on => Some(Goal::AnyUnitPasses(line(&key, num()))),
                "deathtimerrunsout" if on => Some(Goal::DeathTimer(num())),
                _ => None,
            };
            goals.extend(goal);
        }
        let schemas: Vec<MissionSchema> = h.sections.iter().filter(|s| s.name.to_ascii_lowercase().starts_with("schema")).map(parse_schema).collect();
        for g in &mut goals {
            match g {
                Goal::UnitTypePasses { unit, line } => line.toward = crossing(&schemas, *line, Some(unit.as_str())),
                Goal::AnyUnitPasses(line) => line.toward = crossing(&schemas, *line, None),
                _ => {}
            }
        }
        let (victory, defeat) = goals.into_iter().partition(Goal::is_victory);
        // File names, stems wanted: the editor's file picker wrote some with their extension
        // (the Battle Tactics' `brief=T17Brief.txt`, `glamour=tabtarm.pcx`).
        let stem = |k: &str, ext: &str| {
            let v = st(k);
            match v.len().checked_sub(ext.len()) {
                Some(cut) if v.is_char_boundary(cut) && v[cut..].eq_ignore_ascii_case(ext) => v[..cut].to_string(),
                _ => v,
            }
        };
        Mission {
            file: file.to_string(),
            name: st("missionname"),
            planet: st("planet"),
            brief: stem("brief", ".txt"),
            narration: stem("narration", ".wav"),
            glamour: stem("glamour", ".pcx"),
            glamour_sound: stem("glamoursound", ".wav"),
            use_only: stem("useonlyunits", ".tdf"),
            line_of_sight: h.get_bool_or("lineofsight", true),
            mapping: h.get_bool_or("mapping", true),
            max_units: h.get_i64("maxunits").filter(|&n| n > 0).map(|n| n.min(u32::MAX as i64) as u32),
            victory,
            defeat,
            schemas,
        }
    }

    /// The map (TNT) path for this mission.
    pub fn map_path(&self) -> String {
        let stem = self.file.rsplit_once('.').map_or(self.file.as_str(), |(s, _)| s);
        format!("maps/{stem}.tnt")
    }

    /// Schema for a difficulty ("Easy"/"Medium"/"Hard"), falling back to the first.
    pub fn schema(&self, difficulty: &str) -> Option<&MissionSchema> {
        self.schemas.iter().find(|s| s.kind.eq_ignore_ascii_case(difficulty)).or(self.schemas.first())
    }
}

fn parse_schema(s: &TdfSection) -> MissionSchema {
    let f = |k: &str, d: f64| s.get_f64_or(k, d);
    let mut out = MissionSchema {
        kind: s.get_str("Type", ""),
        human_metal: f("HumanMetal", 1000.0),
        human_energy: f("HumanEnergy", 1000.0),
        computer_metal: f("ComputerMetal", 1000.0),
        computer_energy: f("ComputerEnergy", 1000.0),
        surface_metal: f("SurfaceMetal", 0.0),
        ..Default::default()
    };
    if let Some(units) = s.section("units") {
        for u in &units.sections {
            let name = u.get_str("Unitname", "");
            if name.is_empty() {
                continue;
            }
            let (script, unknown) = parse_script(u.get("InitialMission").unwrap_or(""));
            let text = |k: &str| u.get(k).unwrap_or("").trim().to_string();
            out.units.push(PlacedUnit {
                name,
                ident: u.get_str("Ident", ""),
                x: u.get_f32_or("XPos", 0.0),
                z: u.get_f32_or("ZPos", 0.0),
                player: u.get_i32_or("Player", 1).max(1) as u32,
                health: (u.get_f32_or("HealthPercentage", 100.0) / 100.0).clamp(0.01, 1.0),
                angle: u.get_f32_or("Angle", 0.0),
                kills: u.get_i64_or("Kills", 0).max(0) as u64,
                script,
                unknown,
                immune: u.get_bool_or("Immunity", false),
                group: text("InitialGroup"),
                ai_ignore: u.get_bool_or("AiIgnore", false),
                ai_priority: u.get_bool_or("AiPriorityTarget", false),
                critical: u.get_bool_or("MissionCriticalUnit", false),
                build_priority: text("BuildPriority"),
                off_map: u.get_bool_or("OffMapUnit", false),
                creation_countdown: u.get_f32_or("CreationCountdown", 0.0).max(0.0),
            });
        }
    }
    if let Some(specials) = s.section("specials") {
        for sp in &specials.sections {
            if sp.get_str("specialwhat", "").eq_ignore_ascii_case("startpos1") {
                out.start = Some((sp.get_f32_or("XPos", 0.0), sp.get_f32_or("ZPos", 0.0)));
            }
        }
    }
    out
}

/// Which way a goal's line is crossed: away from where the units concerned start. For a unit
/// type's crossing, the player's units of that type (else all the player's units), for "any
/// unit" the enemy's; the first schema placing any decides (TA player 1 is the player). With
/// no units to go by, westward or northward.
fn crossing(schemas: &[MissionSchema], line: Line, unit: Option<&str>) -> Toward {
    let coord = |u: &PlacedUnit| if line.is_x() { u.x } else { u.z };
    let mean = |pick: &dyn Fn(&PlacedUnit) -> bool| {
        schemas.iter().find_map(|s| {
            let picked: Vec<f32> = s.units.iter().filter(|u| pick(u)).map(coord).collect();
            (!picked.is_empty()).then(|| picked.iter().sum::<f32>() / picked.len() as f32)
        })
    };
    let start = match unit {
        Some(name) => mean(&|u| u.player == 1 && u.name.eq_ignore_ascii_case(name)).or_else(|| mean(&|u| u.player == 1)),
        None => mean(&|u| u.player != 1),
    };
    match (line.is_x(), start.is_some_and(|s| s < line.at)) {
        (true, true) => Toward::East,
        (true, false) => Toward::West,
        (false, true) => Toward::South,
        (false, false) => Toward::North,
    }
}

/// Unit names a mission allows (`camps/useonly/<name>.tdf`: one empty section per unit).
pub fn use_only_units(vfs: &Vfs, name: &str) -> Option<Vec<String>> {
    if name.is_empty() {
        return None;
    }
    let bytes = vfs.read_opt(&format!("camps/useonly/{}.tdf", name.to_ascii_lowercase()))?;
    Some(tdf::parse_bytes(&bytes).sections.iter().map(|s| s.name.to_ascii_uppercase()).collect())
}

/// A briefing's text with its markup parsed into coloured runs. The first line (a mission
/// code) and the closing "END" are dropped.
pub fn briefing(vfs: &Vfs, brief: &str) -> Option<Vec<(BriefColor, String)>> {
    let bytes = vfs.read_opt(&format!("camps/briefs/{}.txt", brief.to_ascii_lowercase()))?;
    let text: String = bytes.iter().map(|&b| b as char).collect();
    let mut lines: Vec<&str> = text.lines().collect();
    if lines.first().is_some_and(|l| l.trim_start().to_ascii_uppercase().starts_with("MISSION")) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty() || l.trim().eq_ignore_ascii_case("END")) {
        lines.pop();
    }
    let body = lines.join("\n");
    // `&Y...&` yellow, `&R...&` red; anything else is plain.
    let mut out: Vec<(BriefColor, String)> = Vec::new();
    let mut rest = body.as_str();
    while let Some(i) = rest.find('&') {
        if i > 0 {
            out.push((BriefColor::Plain, rest[..i].to_string()));
        }
        let tail = &rest[i + 1..];
        let color = match tail.chars().next() {
            Some('Y') | Some('y') => BriefColor::Yellow,
            Some('R') | Some('r') => BriefColor::Red,
            _ => BriefColor::Plain,
        };
        let inner = if color == BriefColor::Plain { tail } else { &tail[1..] };
        match inner.find('&') {
            Some(j) => {
                out.push((color, inner[..j].to_string()));
                rest = &inner[j + 1..];
            }
            None => {
                out.push((color, inner.to_string()));
                rest = "";
            }
        }
    }
    if !rest.is_empty() {
        out.push((BriefColor::Plain, rest.to_string()));
    }
    Some(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BriefColor {
    Plain,
    Yellow,
    Red,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_mission_language() {
        let cmds = parse_initial_mission("o 1 2,w 1200,m 1755 3752,p 1646 1935,p2042 3642,wa DOOM1,a ARMCOM,i TRANSPORT5,s,");
        assert_eq!(
            cmds,
            vec![
                // (`o 1 2`: maneuver and fire at will, TA's own defaults.)
                MissionCommand::States { movement: 1, fire: 2 },
                MissionCommand::Wait(1200),
                MissionCommand::Move(1755.0, 3752.0),
                MissionCommand::Patrol(1646.0, 1935.0),
                MissionCommand::Patrol(2042.0, 3642.0),
                MissionCommand::WaitAttacked(Some("DOOM1".into())),
                MissionCommand::Attack("ARMCOM".into()),
                MissionCommand::Load("TRANSPORT5".into()),
                MissionCommand::Stop,
            ]
        );
        assert_eq!(parse_initial_mission("wa"), vec![MissionCommand::WaitAttacked(None)]);
        // A gun held, then set firing (AC10's Punishers): the fire order is the second number.
        assert_eq!(
            parse_initial_mission("o 0 0, w 1200, o 0 2"),
            vec![MissionCommand::States { movement: 0, fire: 0 }, MissionCommand::Wait(1200), MissionCommand::States { movement: 0, fire: 2 }]
        );
    }

    /// The commands added for the missions that use them: factories' and constructors' builds,
    /// unloading, self-destruct, stockpiles; several points to one command; idents with spaces.
    #[test]
    fn builds_unloads_and_self_destructs() {
        use MissionCommand::*;
        let build = |unit: &str, count: u32, at: Option<(f32, f32)>| Build { unit: unit.into(), count, at };
        // Factories (AC02's lab, Steel Stronghold's plant): a type and a count, or a type alone.
        assert_eq!(parse_initial_mission("w 700,b CORAK,w 300,b CORAK 2"), vec![Wait(700), build("CORAK", 1, None), Wait(300), build("CORAK", 2, None)]);
        // Constructors: a type and a point, with or without a count of 1, or where they stand.
        assert_eq!(parse_initial_mission("b CORMEX 5084 2750"), vec![build("CORMEX", 1, Some((5084.0, 2750.0)))]);
        assert_eq!(parse_initial_mission("b corint 1 1601 1883"), vec![build("corint", 1, Some((1601.0, 1883.0)))]);
        assert_eq!(parse_initial_mission("m 4773 4832,b ARMBRTHA,"), vec![Move(4773.0, 4832.0), build("ARMBRTHA", 1, None)]);
        // A count before the type is no build (Three Passes' `b 2 corstorm`).
        assert_eq!(parse_initial_mission("b 2 corstorm, bw 4"), vec![BuildWeapons(4)]);
        // Transports set one unit down a `u` (Miniement Landing), or where they are (AC13).
        assert_eq!(parse_initial_mission("w 90, m 2300 2048, u 2384 1958, u 2426 2007, m 111 111, d"), vec![
            Wait(90),
            Move(2300.0, 2048.0),
            Unload(Some((2384.0, 1958.0))),
            Unload(Some((2426.0, 2007.0))),
            Move(111.0, 111.0),
            SelfDestruct,
        ]);
        assert_eq!(parse_initial_mission("w 3500,m 759 2927,u,m 2019 3824"), vec![Wait(3500), Move(759.0, 2927.0), Unload(None), Move(2019.0, 3824.0)]);
        // Several points to a move or patrol: each in turn.
        assert_eq!(parse_initial_mission("p 2600 5950 2760 5750 2930 5930"), vec![Patrol(2600.0, 5950.0), Patrol(2760.0, 5750.0), Patrol(2930.0, 5930.0)]);
        assert_eq!(parse_initial_mission("m 1049 2030 1030 4800"), vec![Move(1049.0, 2030.0), Move(1030.0, 4800.0)]);
        // Idents hold spaces (Firebase Prasad, Cleanup on Kral).
        assert_eq!(parse_initial_mission("g Power Mike,wa solar 2"), vec![Guard("Power Mike".into()), WaitAttacked(Some("solar 2".into()))]);
        assert_eq!(parse_initial_mission("a Power Mike"), vec![Attack("Power Mike".into())]);
        // Pieces TA's missions got wrong: a missing letter, a list of types after one `a`, a
        // missing comma (what comes after the first command is lost).
        assert_eq!(parse_initial_mission("0 1 2,a CORREAP,CORINT,w 300 b CORFAV,S"), vec![Attack("CORREAP".into()), Wait(300), Stop]);
    }

    #[test]
    fn goals_split_into_victory_and_defeat() {
        let doc = tdf::parse(
            "[GlobalHeader]{missionname=Test;brief=b;MoveUnitToRadius=ANYTYPE, 992, 656, 64;AllUnitsKilled=1;CommanderKilled=1;UnitTypePassesZ=CORTSHIP, 408;AnyUnitPassesZ=60;[Schema 0]{Type=Easy;[units]{[unit0]{Unitname=ARMFAV;XPos=10;ZPos=20;Player=1;HealthPercentage=50;InitialMission=p 1 2, p 3 4;}}[specials]{[special0]{specialwhat=StartPos1;XPos=5;ZPos=6;}}}}",
        );
        let m = Mission::from_ota("T.ota", &doc);
        assert_eq!(m.victory.len(), 2);
        assert!(m.victory.contains(&Goal::MoveUnitToRadius { unit: None, x: 992.0, z: 656.0, radius: 64.0 }));
        // No transport placed: the player's units (north of the line) cross it southward.
        assert!(m.victory.contains(&Goal::UnitTypePasses { unit: "CORTSHIP".into(), line: Line { at: 408.0, toward: Toward::South } }));
        assert_eq!(m.defeat.len(), 3);
        assert!(m.defeat.contains(&Goal::AnyUnitPasses(Line { at: 60.0, toward: Toward::North })) && m.defeat.contains(&Goal::CommanderKilled));
        let s = m.schema("easy").unwrap();
        assert_eq!(s.units.len(), 1);
        assert_eq!(s.units[0].health, 0.5);
        assert_eq!(s.units[0].script.len(), 2);
        assert_eq!(s.start, Some((5.0, 6.0)));
        assert_eq!(m.map_path(), "maps/T.tnt");
    }

    /// The unit keys of TA's mission editor and the missions' unit limit, as Two Continents'
    /// test schema writes them all (and as TA's missions use them one at a time).
    #[test]
    fn unit_keys_and_the_unit_limit() {
        let doc = tdf::parse(
            "[GlobalHeader]{maxunits=250;KillAllOfType=ARMBATS;[Schema 0]{Type=Easy;[units]{\
             [unit0]{Unitname=CORCOM;Ident=;XPos=2271;ZPos=1739;Player=2;OffMapUnit=1;Immunity=1;CreationCountdown=0;InitialGroup=;\
             InitialMission=;AiIgnore=1;AiPriorityTarget=1;MissionCriticalUnit=1;BuildPriority=;}\
             [unit1]{Unitname=CORHRK;Player=2;InitialGroup=Rockos;BuildPriority=CORHRK;InitialMission=w 60;}\
             [unit2]{Unitname=CORTITAN;Player=1;InitialGroup=1;AiIgnore=0;MissionCriticalUnit=0;}}}}",
        );
        let m = Mission::from_ota("T.ota", &doc);
        assert_eq!(m.max_units, Some(250));
        // File names come as stems, whether the editor wrote the extension or not.
        let files = Mission::from_ota("T.ota", &tdf::parse("[GlobalHeader]{brief=T17Brief.txt;narration=Exp1acn12;glamour=tabtarm.PCX;useonlyunits=Sink the Battleship.tdf;}"));
        assert_eq!([files.brief, files.narration, files.glamour, files.use_only], ["T17Brief", "Exp1acn12", "tabtarm", "Sink the Battleship"]);
        let u = &m.schemas[0].units;
        assert!(u[0].off_map && u[0].immune && u[0].ai_ignore && u[0].ai_priority && u[0].critical);
        assert_eq!((u[0].group.as_str(), u[0].build_priority.as_str(), u[0].creation_countdown), ("", "", 0.0));
        assert_eq!((u[1].group.as_str(), u[1].build_priority.as_str()), ("Rockos", "CORHRK"));
        assert!(!u[1].ai_ignore && !u[1].ai_priority && !u[1].critical && !u[1].off_map);
        assert_eq!(u[2].group, "1");
        assert!(!u[2].ai_ignore && !u[2].critical);
        // The original fifty missions set no limit.
        assert_eq!(Mission::from_ota("T.ota", &tdf::parse("[GlobalHeader]{DestroyAllUnits=1;}")).max_units, None);
    }

    /// Force the Strait's transport crosses `x = 6000` eastward, from where it starts; Lightning
    /// Recon is lost when an enemy unit gets west of `x = 4500`, from their base in the east.
    #[test]
    fn lines_are_crossed_away_from_the_start() {
        let doc = tdf::parse(
            "[GlobalHeader]{UnitTypePassesX=ARMTSHIP, 6000;AnyUnitPassesX=4500;[Schema 0]{Type=Easy;[units]{\
             [unit0]{Unitname=ARMTSHIP;XPos=80;ZPos=2304;Player=1;}[unit1]{Unitname=ARMPT;XPos=7000;ZPos=2000;Player=1;}\
             [unit2]{Unitname=ARMFAV;XPos=8064;ZPos=5024;Player=2;}[unit3]{Unitname=ARMMEX;XPos=6992;ZPos=5008;Player=2;}}}}",
        );
        let m = Mission::from_ota("T.ota", &doc);
        let line = Line { at: 6000.0, toward: Toward::East };
        assert_eq!(m.victory, vec![Goal::UnitTypePasses { unit: "ARMTSHIP".into(), line }]);
        assert!(line.passed(6001.0, 0.0) && !line.passed(5999.0, 0.0) && line.is_x());
        let enemy = Line { at: 4500.0, toward: Toward::West };
        assert_eq!(m.defeat, vec![Goal::AnyUnitPasses(enemy)]);
        assert!(enemy.passed(4499.0, 0.0) && !enemy.passed(4501.0, 0.0));
    }
}
