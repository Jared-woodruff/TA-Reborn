//! Shared assets loaded once per session, game setup descriptions and the map catalogue.

use std::sync::Arc;

use ta_data::{GameData, MapInfo};
use ta_formats::tdf;
use ta_render::Renderer;
use ta_sim::UnitTypes;
use ta_vfs::Vfs;

/// Everything that does not change between matches.
#[derive(Clone)]
pub struct Assets {
    pub vfs: Arc<Vfs>,
    pub data: Arc<GameData>,
    pub types: Arc<UnitTypes>,
    /// Authoritative metadata, independent of the local profile. Network setup,
    /// scenarios and existing battles must see identical start positions on every peer.
    pub maps: Vec<MapEntry>,
    /// Only new local battle choices honor the profile's deactivated map versions.
    pub selectable_maps: Vec<MapEntry>,
    pub campaigns: Vec<ta_data::mission::Campaign>,
    /// Game data fingerprint, every input (maps included): what old records and caches go by.
    /// Multiplayer peers match on [`Assets::network_hash`].
    pub data_hash: u64,
    pub compatibility: ta_data::Compatibility,
    /// The mods mounted, which saved games and replays made now name (`record_mods`).
    pub mods: crate::record_mods::InPlay,
}

impl Assets {
    /// What multiplayer peers must match on: the game data without its maps (the lobby checks
    /// the battle's map on its own), so a map pack one player has keeps nobody out.
    pub fn network_hash(&self) -> u64 {
        self.compatibility.network()
    }

    /// What a record of `setup` made now is checked against: the shared rules, its map and
    /// mission, and the units its mission allows (named in the mission's OTA, itself in the hash).
    pub fn record_data_hash(&self, setup: &GameSetup) -> u64 {
        let mission = setup.mission.as_ref();
        let use_only = mission.and_then(|m| ta_data::mission::Mission::load(&self.vfs, &m.file)).map(|m| m.use_only);
        self.compatibility.for_battle(&setup.map, mission.map(|m| m.file.as_str()), use_only.as_deref())
    }

    /// The hash a record of `version` was made with (41 to 44 left a mission's units out).
    pub fn expected_record_hash(&self, version: u32, setup: &GameSetup) -> u64 {
        match version {
            45.. => self.record_data_hash(setup),
            41..=44 => self.compatibility.for_match(&setup.map, setup.mission.as_ref().map(|m| m.file.as_str())),
            _ => self.data_hash,
        }
    }

    /// Whether a record can be opened (a saved game made with other data only when the player
    /// chose to: [`crate::replay::Record::opened_anyway`]); for a replay, whether it plays back
    /// approximately.
    pub fn validate_record(&self, record: &crate::replay::Record) -> anyhow::Result<bool> {
        let hash = self.expected_record_hash(record.version, &record.setup);
        if record.kind == crate::replay::RecordKind::Save {
            if record.opened_anyway { record.validate_anyway()? } else { record.validate(hash)? }
            Ok(false)
        } else {
            record.validate_playback(hash)
        }
    }

    pub fn load(vfs: Arc<Vfs>, mods: crate::record_mods::InPlay, renderer: &mut Renderer, skin: Option<&ta_skins::Pack>, enemy_skin: Option<&ta_skins::Pack>, maps: &ta_config::maps::Maps) -> anyhow::Result<Assets> {
        let t0 = std::time::Instant::now();
        let data = Arc::new(GameData::load(&vfs));
        let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
        crate::scene::init_models(&vfs, renderer, &data.palette, skin, enemy_skin);
        let selectable_maps = MapEntry::enabled_catalogue(&vfs, maps);
        let maps = MapEntry::catalogue(&vfs);
        let campaigns = ta_data::mission::campaigns(&vfs);
        let compatibility = ta_data::Compatibility::load(&vfs);
        let data_hash = compatibility.global();
        log::info!(
            "assets loaded in {:.0} ms: {} unit types, {} skirmish maps, {} campaigns",
            t0.elapsed().as_secs_f64() * 1000.0,
            types.types.len(),
            maps.len(),
            campaigns.len()
        );
        Ok(Assets { vfs, data, types, maps, selectable_maps, campaigns, data_hash, compatibility, mods })
    }
}

/// A side a player can pick: the key a setup keeps, and what the player reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideChoice {
    pub key: String,
    pub label: String,
}

/// The sides of the game data (a mod's third race, Total Battletech's Inner Sphere and
/// Clans...), in its order. A name several sides share (The Secret Forces' armies all go by
/// ARM) is the first's key; the others get keys of their own (`ARM#2`), labelled with their
/// Commander.
pub fn side_choices(data: &ta_data::GameData) -> Vec<SideChoice> {
    let mut out: Vec<SideChoice> = Vec::new();
    // A side whose Commander is no unit of the data (a mod's army whose units are in another mod,
    // off) is not offered: a player on it would start with nothing and lose at once. (Unless no
    // side has its Commander: then the data is not TA's kind, and they are all offered.)
    let playable = |s: &ta_data::SideDef| !s.name.is_empty() && !s.commander.is_empty() && data.unit(&s.commander).is_some();
    let any_playable = data.sides.iter().any(playable);
    for (i, s) in data.sides.iter().enumerate() {
        if s.name.is_empty() || s.commander.is_empty() || any_playable && !playable(s) {
            continue;
        }
        // (The name alone only for the data's first side of that name, offered or not: that is
        // the side `resolve_side` finds by the name.)
        if data.sides.iter().position(|o| o.name == s.name) != Some(i) {
            let commander = data.unit(&s.commander).map_or(s.commander.clone(), |u| u.display_name.clone());
            out.push(SideChoice { key: format!("{}#{i}", s.name), label: format!("{} ({commander})", s.name) });
        } else {
            out.push(SideChoice { key: s.name.clone(), label: s.name.clone() });
        }
    }
    if out.is_empty() {
        out = ["ARM", "CORE"].iter().map(|s| SideChoice { key: s.to_string(), label: s.to_string() }).collect();
    }
    out
}

/// A side's name (what players see) and its entry in the data, from its key.
pub fn resolve_side<'a>(data: &'a ta_data::GameData, key: &'a str) -> (&'a str, Option<&'a ta_data::SideDef>) {
    match key.split_once('#') {
        Some((name, index)) => (name, index.parse::<usize>().ok().and_then(|i| data.sides.get(i)).filter(|s| s.name == name)),
        None => (key, data.sides.iter().find(|s| s.name == key)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Controller {
    Human,
    Ai(ta_ai::Difficulty),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SlotSetup {
    pub side: String,
    pub color: u8,
    pub controller: Controller,
    /// Index into the map's start positions.
    pub start: usize,
    /// Alliance (players on one team share vision and win together).
    pub team: u8,
    /// The person's name in multiplayer ("" for the local player and computers).
    pub name: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct GameSetup {
    pub map: String,
    #[serde(deserialize_with = "ta_sim::serde_limits::vector::<_, _, 64>")]
    pub players: Vec<SlotSetup>,
    /// The player this client controls; `None` spectates (attract mode).
    pub local_player: Option<u8>,
    pub seed: u64,
    /// A campaign mission instead of a skirmish.
    pub mission: Option<MissionStart>,
    /// Skirmish options (missions bring their own).
    pub rules: ta_sim::GameRules,
    /// A Conquest battle: what the player brings along (see `conquest.rs`).
    pub conquest: Option<ConquestStart>,
    /// A scenario (see `scenario.rs`).
    pub scenario: Option<ScenarioStart>,
}

/// A scenario: which, on what difficulty, and its whole file (so saved games and replays of
/// it play out the same whatever becomes of the file).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ScenarioStart {
    pub id: String,
    pub difficulty: ta_ai::Difficulty,
    pub source: String,
}

/// What a Conquest battle starts with beyond a skirmish's, applied as the game is made (so
/// saved games and replays of it play out the same).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ConquestStart {
    /// The conquest (its seed) and the battle's number in it, for the result.
    pub seed: u64,
    pub battle: u32,
    /// The system fought over.
    pub system: usize,
    /// Extra starting metal and energy for the player (the systems held).
    pub metal: u64,
    pub energy: u64,
    pub commander_xp: ta_sim::Fx,
    /// Veterans deployed beside the Commander: unit type and experience.
    #[serde(deserialize_with = "ta_sim::serde_limits::command_list")]
    pub veterans: Vec<(String, ta_sim::Fx)>,
    /// Fresh units beside them, from the systems' tank plants and airfields.
    #[serde(default, deserialize_with = "ta_sim::serde_limits::command_list")]
    pub reinforcements: Vec<String>,
    /// Fought at a system of ours to beat off a counter-attack (else an attack).
    #[serde(default)]
    pub defence: bool,
    /// The Commander's perks, by name, applied as the battle is made (unknown names are
    /// passed over).
    #[serde(default, deserialize_with = "ta_sim::serde_limits::command_list")]
    pub perks: Vec<String>,
}

/// [`GameSetup`] as saved games and replays before record version 42 laid it out, its
/// Conquest battle without reinforcements, defence or perks (and its rules without mutators):
/// read from those and brought up to date, and written so again when such a record is (it never
/// had either).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct LegacySetup {
    pub map: String,
    #[serde(deserialize_with = "ta_sim::serde_limits::vector::<_, _, 64>")]
    pub players: Vec<SlotSetup>,
    pub local_player: Option<u8>,
    pub seed: u64,
    pub mission: Option<MissionStart>,
    pub rules: LegacyRules,
    pub conquest: Option<LegacyConquestStart>,
    pub scenario: Option<ScenarioStart>,
}

/// [`GameSetup`] as record version 42 laid it out: the rules without mutators.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SetupV42 {
    pub map: String,
    #[serde(deserialize_with = "ta_sim::serde_limits::vector::<_, _, 64>")]
    pub players: Vec<SlotSetup>,
    pub local_player: Option<u8>,
    pub seed: u64,
    pub mission: Option<MissionStart>,
    pub rules: LegacyRules,
    pub conquest: Option<ConquestStart>,
    pub scenario: Option<ScenarioStart>,
}

/// [`ta_sim::GameRules`] as records before version 43 laid them out: without the mutators
/// (which they never had).
#[derive(serde::Serialize, serde::Deserialize)]
pub struct LegacyRules {
    pub start_metal: u64,
    pub start_energy: u64,
    pub fog_of_war: bool,
    pub mapped: bool,
    pub commander_ends_game: bool,
    pub unit_limit: u32,
    pub random_starts: bool,
    pub give_way: bool,
    #[serde(default)]
    pub true_sight: bool,
}

impl From<LegacyRules> for ta_sim::GameRules {
    fn from(old: LegacyRules) -> ta_sim::GameRules {
        ta_sim::GameRules {
            start_metal: old.start_metal,
            start_energy: old.start_energy,
            fog_of_war: old.fog_of_war,
            mapped: old.mapped,
            commander_ends_game: old.commander_ends_game,
            unit_limit: old.unit_limit,
            random_starts: old.random_starts,
            give_way: old.give_way,
            true_sight: old.true_sight,
            mutators: ta_sim::Mutators::default(),
        }
    }
}

impl From<&ta_sim::GameRules> for LegacyRules {
    fn from(rules: &ta_sim::GameRules) -> LegacyRules {
        LegacyRules {
            start_metal: rules.start_metal,
            start_energy: rules.start_energy,
            fog_of_war: rules.fog_of_war,
            mapped: rules.mapped,
            commander_ends_game: rules.commander_ends_game,
            unit_limit: rules.unit_limit,
            random_starts: rules.random_starts,
            give_way: rules.give_way,
            true_sight: rules.true_sight,
        }
    }
}

impl From<SetupV42> for GameSetup {
    fn from(old: SetupV42) -> GameSetup {
        GameSetup { map: old.map, players: old.players, local_player: old.local_player, seed: old.seed, mission: old.mission, rules: old.rules.into(), conquest: old.conquest, scenario: old.scenario }
    }
}

impl From<&GameSetup> for SetupV42 {
    fn from(setup: &GameSetup) -> SetupV42 {
        SetupV42 {
            map: setup.map.clone(),
            players: setup.players.clone(),
            local_player: setup.local_player,
            seed: setup.seed,
            mission: setup.mission.clone(),
            rules: LegacyRules::from(&setup.rules),
            conquest: setup.conquest.clone(),
            scenario: setup.scenario.clone(),
        }
    }
}

/// [`ConquestStart`] before record version 42.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct LegacyConquestStart {
    pub seed: u64,
    pub battle: u32,
    pub system: usize,
    pub metal: u64,
    pub energy: u64,
    pub commander_xp: ta_sim::Fx,
    #[serde(deserialize_with = "ta_sim::serde_limits::command_list")]
    pub veterans: Vec<(String, ta_sim::Fx)>,
}

impl From<LegacySetup> for GameSetup {
    fn from(old: LegacySetup) -> GameSetup {
        let conquest = old.conquest.map(|c| ConquestStart {
            seed: c.seed,
            battle: c.battle,
            system: c.system,
            metal: c.metal,
            energy: c.energy,
            commander_xp: c.commander_xp,
            veterans: c.veterans,
            reinforcements: Vec::new(),
            defence: false,
            perks: Vec::new(),
        });
        GameSetup { map: old.map, players: old.players, local_player: old.local_player, seed: old.seed, mission: old.mission, rules: old.rules.into(), conquest, scenario: old.scenario }
    }
}

impl From<&GameSetup> for LegacySetup {
    fn from(setup: &GameSetup) -> LegacySetup {
        let conquest = setup.conquest.as_ref().map(|c| LegacyConquestStart {
            seed: c.seed,
            battle: c.battle,
            system: c.system,
            metal: c.metal,
            energy: c.energy,
            commander_xp: c.commander_xp,
            veterans: c.veterans.clone(),
        });
        LegacySetup {
            map: setup.map.clone(),
            players: setup.players.clone(),
            local_player: setup.local_player,
            seed: setup.seed,
            mission: setup.mission.clone(),
            rules: LegacyRules::from(&setup.rules),
            conquest,
            scenario: setup.scenario.clone(),
        }
    }
}

/// Which campaign mission to play.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct MissionStart {
    /// Campaign file stem (progress key).
    pub campaign: String,
    pub index: usize,
    /// Mission OTA file.
    pub file: String,
    /// "Easy", "Medium" or "Hard".
    pub difficulty: String,
}

impl GameSetup {
    /// Validate setup fields before they can index simulation state. Replays and remote
    /// hosts supply these fields too, bypassing the local setup controls.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!((1..=ta_sim::unit::MAX_PLAYERS).contains(&self.players.len()), "invalid player count");
        anyhow::ensure!(self.local_player.is_none_or(|p| (p as usize) < self.players.len()), "invalid local player");
        Ok(())
    }

    /// A campaign mission: the human (TA player 1) on the campaign's side against the
    /// computer players the mission places.
    pub fn mission(assets: &Assets, campaign: &ta_data::mission::Campaign, index: usize, difficulty: &str) -> Option<GameSetup> {
        let entry = campaign.missions.get(index)?;
        let mission = ta_data::mission::Mission::load(&assets.vfs, &entry.file)?;
        let schema = mission.schema(difficulty)?;
        let computers = (schema.units.iter().map(|u| u.player).max().unwrap_or(2).max(2) as usize).min(ta_sim::unit::MAX_PLAYERS);
        let own = if campaign.side.eq_ignore_ascii_case("CORE") { "CORE" } else { "ARM" };
        let other = if own == "ARM" { "CORE" } else { "ARM" };
        let ai = match difficulty.to_ascii_lowercase().as_str() {
            "easy" => ta_ai::Difficulty::Easy,
            "hard" => ta_ai::Difficulty::Hard,
            _ => ta_ai::Difficulty::Normal,
        };
        let players = (0..computers)
            .map(|i| {
                let side = if i == 0 { own } else { other };
                SlotSetup {
                    side: side.into(),
                    color: if side == "ARM" { 0 } else { 1 + (i as u8).saturating_sub(1) * 2 },
                    controller: if i == 0 { Controller::Human } else { Controller::Ai(ai) },
                    start: i,
                    // The computer players of a mission are all against the human.
                    team: (i > 0) as u8,
                    name: String::new(),
                }
            })
            .collect();
        let map = mission.map_path().trim_start_matches("maps/").trim_end_matches(".tnt").to_string();
        Some(GameSetup {
            map,
            players,
            local_player: Some(0),
            seed: 0x7A5EED ^ index as u64,
            rules: Default::default(),
            mission: Some(MissionStart { campaign: campaign.id.clone(), index, file: entry.file.clone(), difficulty: difficulty.to_string() }),
            conquest: None,
            scenario: None,
        })
    }
}

impl GameSetup {
    /// Human (Arm) against one AI (Core) on `map`.
    pub fn duel(map: &str) -> GameSetup {
        GameSetup {
            map: map.to_string(),
            players: vec![
                SlotSetup { side: "ARM".into(), color: 0, controller: Controller::Human, start: 0, team: 0, name: String::new() },
                SlotSetup { side: "CORE".into(), color: 1, controller: Controller::Ai(ta_ai::Difficulty::Normal), start: 1, team: 1, name: String::new() },
            ],
            local_player: Some(0),
            seed: 0xC0FFEE,
            mission: None,
            rules: Default::default(),
            conquest: None,
            scenario: None,
        }
    }
}

/// Deals out start positions: in slot order, or with random starts `players` of the map's
/// `available` positions drawn from `seed` (every peer draws the same). In a team game, where
/// the map's start `spots` are known, allies take neighbouring spots and the teams spread round
/// the map (from the first spot, in slot order; random, from a turn and an order of the teams
/// drawn from the seed).
pub fn assign_starts(players: &mut [SlotSetup], available: usize, seed: u64, random: bool, spots: &[(f32, f32)]) {
    let total = available.max(players.len());
    let mut teams: Vec<u8> = Vec::new();
    for p in players.iter() {
        if !teams.contains(&p.team) {
            teams.push(p.team);
        }
    }
    let spots = &spots[..spots.len().min(total)];
    if teams.len() > 1 && teams.len() < players.len() && spots.len() >= players.len() {
        let order = angular_order(spots);
        let n = order.len();
        let mut x = seed ^ 0xA11E_5EED_0000_0001;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut turn = 0;
        if random {
            for i in (1..teams.len()).rev() {
                teams.swap(i, (next() % (i as u64 + 1)) as usize);
            }
            turn = (next() % n as u64) as usize;
        }
        let mut taken = vec![false; n];
        for (t, team) in teams.iter().enumerate() {
            let mut k = (turn + t * n / teams.len()) % n;
            for p in players.iter_mut().filter(|p| p.team == *team) {
                while taken[k] {
                    k = (k + 1) % n;
                }
                taken[k] = true;
                p.start = order[k];
            }
        }
        return;
    }
    let mut starts: Vec<usize> = (0..total).collect();
    if random {
        // Fisher-Yates with xorshift64.
        let mut x = seed ^ 0x9E37_79B9_7F4A_7C15;
        for i in (1..starts.len()).rev() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            starts.swap(i, (x % (i as u64 + 1)) as usize);
        }
    }
    for (p, s) in players.iter_mut().zip(starts) {
        p.start = s;
    }
}

/// The spots' indices round their middle, one way round from one place: with integer
/// arithmetic only, so every machine of a multiplayer game deals the same.
fn angular_order(spots: &[(f32, f32)]) -> Vec<usize> {
    let points: Vec<(i64, i64)> = spots.iter().map(|&(x, z)| (x.round() as i64, z.round() as i64)).collect();
    let n = (points.len() as i64).max(1);
    let middle = (points.iter().map(|p| p.0).sum::<i64>() / n, points.iter().map(|p| p.1).sum::<i64>() / n);
    let half = |(x, z): (i64, i64)| u8::from(!(z < 0 || z == 0 && x >= 0));
    let mut order: Vec<usize> = (0..points.len()).collect();
    order.sort_by(|&a, &b| {
        let pa = (points[a].0 - middle.0, points[a].1 - middle.1);
        let pb = (points[b].0 - middle.0, points[b].1 - middle.1);
        half(pa)
            .cmp(&half(pb))
            .then_with(|| (pa.0 * pb.1 - pa.1 * pb.0).cmp(&0).reverse())
            .then_with(|| (pa.0 * pa.0 + pa.1 * pa.1).cmp(&(pb.0 * pb.0 + pb.1 * pb.1)))
            .then(a.cmp(&b))
    });
    order
}

/// Where a map's players start (its first schema's positions), by slot.
pub fn start_spots(assets: &Assets, map: &str) -> Vec<(f32, f32)> {
    assets.maps.iter().find(|m| m.name == map).and_then(|m| m.info.schemas.first()).map(|sc| sc.start_positions.iter().map(|&(_, x, z)| (x, z)).collect()).unwrap_or_default()
}

/// A map offered for skirmish.
#[derive(Clone, Debug)]
pub struct MapEntry {
    /// File stem (`maps/<name>.tnt`).
    pub name: String,
    pub title: String,
    pub description: String,
    pub planet: String,
    pub size: String,
    pub start_positions: usize,
    pub info: MapInfo,
}

impl MapEntry {
    /// A profile can hide a particular source's map from new battle choices without
    /// deleting data needed by campaigns, existing saves, or editing a copy.
    pub fn enabled_catalogue(vfs: &Vfs, preferences: &ta_config::maps::Maps) -> Vec<MapEntry> {
        Self::catalogue(vfs).into_iter().filter(|entry| {
            vfs.origin(&format!("maps/{}.tnt", entry.name))
                .is_none_or(|source| preferences.is_enabled(&source.layer, &entry.name))
        }).collect()
    }

    /// Maps with at least two start positions, sorted by name. Campaign missions (which
    /// have scripted objectives) are excluded by their `ac*/cc*/bt*`-style numbering.
    pub fn catalogue(vfs: &Vfs) -> Vec<MapEntry> {
        let mut out = Vec::new();
        for path in vfs.list("maps", Some("ota"), false) {
            let Some(bytes) = vfs.read_opt(&path) else { continue };
            let info = MapInfo::from_ota(&tdf::parse_bytes(&bytes));
            // OTAs list StartPos1..10 even on small maps; `numplayers` ("2, 3, 4") is the
            // designer's real limit.
            let listed = info.schemas.first().map_or(0, |s| s.start_positions.len());
            let designed = info.num_players.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse::<usize>().ok()).max();
            let starts = designed.map_or(listed, |d| d.min(listed));
            let stem = path.trim_start_matches("maps/").trim_end_matches(".ota").to_string();
            if starts < 2 || campaign_stem(&stem) || !vfs.exists(&format!("maps/{stem}.tnt")) {
                continue;
            }
            // Title-case the file stem when the OTA has no name.
            let title = if info.name.trim().is_empty() { stem.clone() } else { info.name.trim().to_string() };
            out.push(MapEntry {
                title,
                description: ta_data::map::blurb(&info.description),
                planet: info.planet.clone(),
                size: info.size.clone(),
                start_positions: starts,
                name: stem,
                info,
            });
        }
        out.sort_by_key(|m| m.title.to_ascii_lowercase());
        out
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_source_maps_leave_saved_game_and_editor_data_available() {
        let root = std::env::temp_dir().join(format!("ta-map-filter-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let original = root.join("original"); let changed = root.join("changed");
        for dir in [&original, &changed] {
            std::fs::create_dir_all(dir.join("maps")).unwrap();
            std::fs::write(dir.join("maps/moon.tnt"), b"terrain is still mounted").unwrap();
            std::fs::write(dir.join("maps/moon.ota"), b"[GlobalHeader]{missionname=Moon;numplayers=2;[Schema 0]{[specials]{[special0]{specialwhat=StartPos1;}[special1]{specialwhat=StartPos2;}}}}").unwrap();
        }
        let mut preferences = ta_config::maps::Maps::default();
        preferences.set_enabled("changed", "moon", false);
        let mut vfs = Vfs::new(); vfs.mount_mod_dir(&original, "base").unwrap();
        assert_eq!(MapEntry::enabled_catalogue(&vfs, &preferences).len(), 1, "the original version keeps its own preference");
        vfs.mount_mod_dir(&changed, "changed").unwrap();
        assert!(MapEntry::enabled_catalogue(&vfs, &preferences).is_empty());
        assert_eq!(MapEntry::catalogue(&vfs).len(), 1, "unfiltered metadata remains accessible");
        assert_eq!(vfs.read("maps/moon.tnt").unwrap(), b"terrain is still mounted", "saved games and editing retain terrain");
        preferences.set_enabled("changed", "moon", true);
        assert_eq!(MapEntry::enabled_catalogue(&vfs, &preferences).len(), 1);
        drop(vfs); std::fs::remove_dir_all(root).unwrap();
    }


    /// A side whose Commander is not a unit of the data is not offered: a player on it would
    /// start with nothing and be defeated at the first tick.
    #[test]
    fn a_side_without_its_commander_is_not_offered() {
        let root = std::env::temp_dir().join(format!("ta-sides-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        for (path, text) in [
            ("gamedata/sidedata.tdf", "[SIDE0]{name=ARM;commander=ARMCOM;} [SIDE1]{name=TAL;commander=TALCOM;} [SIDE2]{name=CORE;commander=CORCOM;}"),
            ("units/armcom.fbi", "[UNITINFO]{UnitName=ARMCOM;}"),
            ("units/corcom.fbi", "[UNITINFO]{UnitName=CORCOM;}"),
        ] {
            std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            std::fs::write(root.join(path), text).unwrap();
        }
        let keys = |root: &std::path::Path| {
            let mut vfs = Vfs::new();
            vfs.mount_mod_dir(root, "base").unwrap();
            side_choices(&GameData::load(&vfs)).into_iter().map(|s| s.key).collect::<Vec<_>>()
        };
        assert_eq!(keys(&root), ["ARM", "CORE"], "TAL's Commander is in no unit file");
        std::fs::write(root.join("units/talcom.fbi"), "[UNITINFO]{UnitName=TALCOM;}").unwrap();
        assert_eq!(keys(&root), ["ARM", "TAL", "CORE"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Each side offered resolves to itself, also where sides share a name and the first of
    /// them is not offered (its Commander missing): the next is not given the name alone, which
    /// would find the first again (Codex's review of #64).
    #[test]
    fn every_side_offered_resolves_to_itself() {
        let root = std::env::temp_dir().join(format!("ta-side-keys-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        for (path, text) in [
            ("gamedata/sidedata.tdf", "[SIDE0]{name=ARM;commander=MISSING;} [SIDE1]{name=ARM;commander=ARMCOM;} [SIDE2]{name=ARM;commander=ARMCOM2;} [SIDE3]{name=CORE;commander=CORCOM;}"),
            ("units/armcom.fbi", "[UNITINFO]{UnitName=ARMCOM;}"),
            ("units/armcom2.fbi", "[UNITINFO]{UnitName=ARMCOM2;}"),
            ("units/corcom.fbi", "[UNITINFO]{UnitName=CORCOM;}"),
        ] {
            std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            std::fs::write(root.join(path), text).unwrap();
        }
        let mut vfs = Vfs::new();
        vfs.mount_mod_dir(&root, "base").unwrap();
        let data = GameData::load(&vfs);
        let choices = side_choices(&data);
        assert_eq!(choices.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(), ["ARM#1", "ARM#2", "CORE"]);
        for choice in &choices {
            let side = resolve_side(&data, &choice.key).1.unwrap_or_else(|| panic!("{} resolves", choice.key));
            assert!(data.unit(&side.commander).is_some(), "{} resolves to a side with its Commander, not {}", choice.key, side.commander);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A save or replay of a campaign battle made from version 45 is checked against the units its
    /// mission allows (`camps/useonly`, named in its OTA); one made before, against what it was
    /// made with. A skirmish's hash is the same either way.
    #[test]
    fn records_from_version_45_cover_the_units_a_mission_allows() {
        let root = std::env::temp_dir().join(format!("ta-record-useonly-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        for (path, text) in [
            ("maps/battle.tnt", "terrain"),
            ("maps/battle.ota", "[GlobalHeader]{missionname=Battle;}"),
            ("maps/mission.ota", "[GlobalHeader]{missionname=Mission;useonlyunits=Only Arm.tdf;}"),
            ("camps/useonly/only arm.tdf", "[ARMCOM]{}"),
        ] {
            std::fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            std::fs::write(root.join(path), text).unwrap();
        }
        let assets = |root: &std::path::Path| {
            let mut vfs = Vfs::new();
            vfs.mount_mod_dir(root, "base").unwrap();
            let data = Arc::new(GameData::load(&vfs));
            let types = Arc::new(UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
            let compatibility = ta_data::Compatibility::load(&vfs);
            Assets { vfs: Arc::new(vfs), data, types, maps: Vec::new(), selectable_maps: Vec::new(), campaigns: Vec::new(), data_hash: compatibility.global(), compatibility, mods: Default::default() }
        };
        let mut campaign = GameSetup::duel("battle");
        campaign.mission = Some(MissionStart { campaign: "test".into(), index: 0, file: "mission.ota".into(), difficulty: "Easy".into() });
        let skirmish = GameSetup::duel("battle");
        let before = assets(&root);
        let made = before.record_data_hash(&campaign);
        assert_eq!(made, before.expected_record_hash(ta_config::RECORD_VERSION, &campaign));
        assert_eq!(before.expected_record_hash(45, &skirmish), before.expected_record_hash(44, &skirmish), "a skirmish: the same");
        // A mod changes the units the mission allows.
        std::fs::write(root.join("camps/useonly/only arm.tdf"), "[ARMCOM]{} [ARMPW]{}").unwrap();
        let after = assets(&root);
        assert_ne!(after.expected_record_hash(45, &campaign), made, "the battle is not the one saved");
        assert_eq!(after.expected_record_hash(44, &campaign), before.expected_record_hash(44, &campaign), "a record made before keeps its own check");
        std::fs::remove_dir_all(root).unwrap();
    }

    fn players(n: usize) -> Vec<SlotSetup> {
        (0..n).map(|i| SlotSetup { side: "ARM".into(), color: i as u8, controller: Controller::Human, start: 99, team: i as u8, name: String::new() }).collect()
    }

    #[test]
    fn allies_start_next_to_each_other_and_teams_apart() {
        // Four corners, two teams sitting in turn: allies take corners side by side.
        let corners = [(0.0, 0.0), (1000.0, 0.0), (1000.0, 1000.0), (0.0, 1000.0)];
        let mut ps = players(4);
        for (i, p) in ps.iter_mut().enumerate() {
            p.team = (i % 2) as u8;
        }
        let neighbours = |a: usize, b: usize| corners[a].0 == corners[b].0 || corners[a].1 == corners[b].1;
        for (seed, random) in [(1, false), (2, true), (3, true), (4, true)] {
            assign_starts(&mut ps, 4, seed, random, &corners);
            assert!(neighbours(ps[0].start, ps[2].start) && neighbours(ps[1].start, ps[3].start), "allies side by side: {:?}", ps.iter().map(|p| p.start).collect::<Vec<_>>());
            let mut all: Vec<usize> = ps.iter().map(|p| p.start).collect();
            all.sort_unstable();
            assert_eq!(all, [0, 1, 2, 3], "every player a spot of their own");
        }
        // Everyone on a team of their own: as before, slot order.
        let mut ffa = players(4);
        assign_starts(&mut ffa, 4, 1, false, &corners);
        assert_eq!(ffa.iter().map(|p| p.start).collect::<Vec<_>>(), [0, 1, 2, 3]);
    }

    #[test]
    fn random_starts_are_distinct_drawn_from_the_seed_and_shared() {
        let mut fixed = players(3);
        assign_starts(&mut fixed, 10, 42, false, &[]);
        assert_eq!(fixed.iter().map(|p| p.start).collect::<Vec<_>>(), [0, 1, 2], "slot order");
        let draw = |seed| {
            let mut ps = players(4);
            assign_starts(&mut ps, 10, seed, true, &[]);
            ps.iter().map(|p| p.start).collect::<Vec<_>>()
        };
        let a = draw(7);
        assert_eq!(a, draw(7), "every peer draws the same from the seed");
        let mut sorted = a.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 4, "no two players share a position");
        assert!(a.iter().all(|&s| s < 10));
        assert!((0..20).any(|seed| draw(seed) != draw(seed + 1)), "different seeds, different draws");
        assert!((0..20).any(|seed| draw(seed).iter().any(|&s| s >= 4)), "any of the map's positions, not just the first few");
    }
}

/// Whether a map's file stem is one of the campaigns' ("cc5", "ac12", "exp1ac3"...): Skirmish
/// doesn't list those.
pub fn campaign_stem(stem: &str) -> bool {
    let stem = stem.to_ascii_lowercase();
    stem.len() <= 9
        && ["ac", "cc", "bt", "exp1ac", "exp1cc", "ec", "am", "cm", "ct"].iter().any(|p| {
            stem.strip_prefix(p).is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
        })
}
