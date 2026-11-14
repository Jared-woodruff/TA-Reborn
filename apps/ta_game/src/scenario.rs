//! Scenarios: battles set up in advance, each with a goal of its own: hold out against
//! waves, hold the hill, get the Commander out, strike a fortress, beat a doomsday clock,
//! race for metal. A scenario is a TOML file (the built-in ones are in
//! `apps/ta_game/scenarios`; more go in the settings folder's `scenarios`) that places units
//! by name at positions given in words that fit any map (`start 2`, `centre`,
//! `start 1 toward start 2 by 600`). Besides its forces, goals and waves it may have allies
//! (`[[ally]]`, on the player's team), reinforcements (a `[[wave]]` with `player = true` or
//! `ally = N`), a skirmish's mutators (`[mutators]`), the time of day (`time`: hours, or keys
//! of [seconds, hours] eased between) and rain (`rain`), and transmissions at their times
//! (`[[message]]`: `at`, `from`, `text`, `side`). The whole file goes into the game's setup,
//! so saved games and replays play out the same whatever becomes of it.

use anyhow::{Context, Result, anyhow, bail};
use ta_ai::Difficulty;
use ta_sim::mission::{MissionRules, Objective, Wave};
use ta_sim::{Fx, FxVec3, Sim};
use ta_ui::i18n::{n, t, tf};

use crate::setup::{Assets, Controller, GameSetup, ScenarioStart, SlotSetup};

#[cfg(test)]
#[path = "scenario_tests.rs"]
mod review_tests;

#[cfg(test)]
#[path = "scenario_placement_tests.rs"]
mod placement_tests;

#[path = "scenario_layout.rs"]
mod layout;

/// The scenarios that come with the game, in the order the menu lists them.
const BUILT_IN: &[&str] = &[
    include_str!("../scenarios/last_stand.toml"),
    include_str!("../scenarios/king_of_the_hill.toml"),
    include_str!("../scenarios/extraction.toml"),
    include_str!("../scenarios/decapitation.toml"),
    include_str!("../scenarios/doomsday_clock.toml"),
    include_str!("../scenarios/metal_rush.toml"),
    include_str!("../scenarios/armageddon.toml"),
    include_str!("../scenarios/endless_siege.toml"),
    include_str!("../scenarios/until_dawn.toml"),
    include_str!("../scenarios/titans.toml"),
    include_str!("../scenarios/iron_rain.toml"),
    include_str!("../scenarios/armistice.toml"),
    include_str!("../scenarios/brothers_in_arms.toml"),
];

/// Most waves a repeating wave sends (an endless one stops after this many: hours).
const MAX_REPEATS: u32 = 200;
/// Most units of one type in one wave, however much the waves grow.
const MAX_WAVE_UNITS: u16 = 60;

fn yes() -> bool {
    true
}

fn one() -> u16 {
    1
}

fn one_f() -> f32 {
    1.0
}

/// What kind of challenge a scenario is (its card says so).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Survive,
    Endless,
    Hold,
    Escort,
    Strike,
    Clock,
    Race,
    Battle,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Survive => ta_ui::i18n::n("SURVIVE"),
            Kind::Endless => ta_ui::i18n::n("ENDLESS"),
            Kind::Hold => ta_ui::i18n::n("HOLD"),
            Kind::Escort => ta_ui::i18n::n("ESCORT"),
            Kind::Strike => ta_ui::i18n::n("STRIKE"),
            Kind::Clock => ta_ui::i18n::n("AGAINST THE CLOCK"),
            Kind::Race => ta_ui::i18n::n("RACE"),
            Kind::Battle => ta_ui::i18n::n("BATTLE"),
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub id: String,
    pub title: String,
    /// One line for the list.
    pub tagline: String,
    pub briefing: String,
    /// The map, by its name or file name.
    pub map: String,
    pub kind: Kind,
    /// About how long it takes, in minutes.
    #[serde(default)]
    pub minutes: u32,
    /// Line of sight (the fog of war), and the map known from the start.
    #[serde(default = "yes")]
    pub fog_of_war: bool,
    #[serde(default = "yes")]
    pub mapped: bool,
    pub player: Force,
    #[serde(default, rename = "enemy")]
    pub enemies: Vec<Force>,
    pub victory: Vec<GoalSpec>,
    #[serde(default)]
    pub defeat: Vec<GoalSpec>,
    #[serde(default, rename = "wave")]
    pub waves: Vec<WaveSpec>,
    /// Forces on the player's side (its team): a computer player each ("easy", "normal",
    /// "hard", "chosen"), or nobody ("none", the default).
    #[serde(default, rename = "ally")]
    pub allies: Vec<Force>,
    /// The battle's mutators, as a skirmish's (`ta_sim::Mutators`, by field:
    /// `no_rush_minutes`, `resources`, `build_speed`, `toughness`, `no_air`...).
    #[serde(default)]
    pub mutators: Option<ta_sim::Mutators>,
    /// The time of day: hours (0 midnight, 6.4 dawn, 12 TA's own light, 17.75 dusk), or keys
    /// of [seconds, hours] eased between, night falling or dawn coming (past 24 the clock goes
    /// round: 30.5 is half past six the next morning).
    #[serde(default)]
    pub time: Option<crate::stage::Hours>,
    /// Rain from the start (`false`: dry weather); else the world's own.
    #[serde(default)]
    pub rain: Option<bool>,
    /// Transmissions at their times.
    #[serde(default, rename = "message")]
    pub messages: Vec<MessageSpec>,
}

/// A transmission at its time: who speaks, and what.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSpec {
    /// Seconds into the battle.
    pub at: u32,
    /// Who speaks ("Arm Command", "Commander Holt").
    pub from: String,
    pub text: String,
    /// Whose link it comes on: "player" (the default), "ally N" or "enemy N" (from 1, in the
    /// file's order).
    #[serde(default)]
    pub side: Option<String>,
    /// Only from this difficulty up.
    #[serde(default)]
    pub from_difficulty: Option<String>,
}

/// A side in a scenario: the player's or an enemy's.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Force {
    /// "ARM", "CORE", or another side of the game data.
    pub side: String,
    /// Where it stands (position words): its Commander, a computer player's base, the camera.
    pub start: String,
    #[serde(default)]
    pub metal: u64,
    #[serde(default)]
    pub energy: u64,
    #[serde(default)]
    pub commander: bool,
    /// Enemies: a computer player plays it ("easy", "normal", "hard", or "chosen": the
    /// difficulty picked), or nobody ("none", the default: its units follow their orders and
    /// fight what comes near).
    #[serde(default)]
    pub ai: Option<String>,
    #[serde(default)]
    pub units: Vec<Placement>,
}

/// Units placed at the start.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub unit: String,
    #[serde(default = "one")]
    pub count: u16,
    pub at: String,
    /// Deliberate rows, grids, or separate metal deposits. Omitted in older scenarios.
    #[serde(default)]
    pub formation: Option<layout::Formation>,
    /// Centre-to-centre distance in pixels (at least the unit's footprint plus clearance).
    #[serde(default)]
    pub spacing: Option<f32>,
    /// A line faces this place; by default it faces the opposing force.
    #[serde(default)]
    pub facing: Option<String>,
    /// They patrol from where they stand to here and back.
    #[serde(default)]
    pub patrol: Option<String>,
    /// Veterans: their experience, in kills.
    #[serde(default)]
    pub xp: u32,
    /// They hold their position (buildings don't move anyway).
    #[serde(default)]
    pub hold: bool,
    /// They fire at will on what their side has on radar or sonar as well, as guns a spotter
    /// directs do (Iron Rain's battery and the radar post that spots for it): everyone else's
    /// fire at will takes on only what their side sees.
    #[serde(default)]
    pub radar_guided: bool,
    /// Only from this difficulty up ("normal", "hard"): extra defenders.
    #[serde(default)]
    pub from: Option<String>,
    /// Only up to this difficulty ("easy", "normal"): help for the player.
    #[serde(default)]
    pub until: Option<String>,
}

/// A goal, in the file: `{ survive = 600 }`, `{ hold = "centre", radius = 400, seconds = 180 }`,
/// `{ destroy = "commander" }`, `{ lose = "commander" }`...
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum GoalSpec {
    /// Victory: still in the game after so many seconds.
    Survive { survive: u32 },
    /// Victory: the player's side alone in the area for so many seconds in all.
    Hold { hold: String, radius: f32, seconds: u32 },
    /// Defeat: the enemy alone in the area for so many seconds in all.
    EnemyHolds { enemy_holds: String, radius: f32, seconds: u32 },
    /// Victory: a unit ("commander", a unit name, or any) gets there.
    Reach {
        reach: String,
        radius: f32,
        #[serde(default)]
        unit: Option<String>,
    },
    /// Victory: "commander" (the enemy's), "everything", "army" (every mobile unit) or every
    /// unit of a type.
    Destroy { destroy: String },
    /// Victory: this much metal made in all.
    Metal { metal: u64 },
    /// Defeat: an enemy made this much metal in all.
    EnemyMetal { enemy_metal: u64 },
    /// Defeat: so many seconds have gone by.
    Time { time: u32 },
    /// Defeat: "commander" (the player's), "everything", or a unit of a type.
    Lose { lose: String },
}

/// Attackers that arrive on schedule.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaveSpec {
    /// Seconds into the battle.
    pub at: u32,
    /// Then again every so many seconds, `times` more (0 with `every`: until the battle ends).
    #[serde(default)]
    pub every: u32,
    #[serde(default)]
    pub times: u32,
    /// Each repeat brings this many times the units of the one before.
    #[serde(default = "one_f")]
    pub grow: f32,
    /// Where they appear (position words); several, split by `|`, take turns.
    pub from: String,
    /// Where they attack-move to; by default the player's forces nearest to where they
    /// appear.
    #[serde(default)]
    pub to: Option<String>,
    /// Once there they stay (a hill's defenders) instead of going after the player's forces.
    #[serde(default)]
    pub stay: bool,
    pub units: Vec<WaveUnit>,
    /// Whose they are: the enemy's number in the file, from 1.
    #[serde(default)]
    pub enemy: Option<usize>,
    /// Reinforcements for the player: they come to `to` (else wait where they appear), the
    /// player's to command.
    #[serde(default)]
    pub player: bool,
    /// An ally's reinforcements: its number in the file, from 1.
    #[serde(default)]
    pub ally: Option<usize>,
    /// Only from this difficulty up.
    #[serde(default)]
    pub from_difficulty: Option<String>,
}

impl WaveSpec {
    fn tick(&self, repeat: u32) -> u64 {
        // A large interval must stay in the future, never wrap back to the opening
        // seconds (or panic in a debug build).
        (self.at as u64 + repeat as u64 * self.every as u64) * 30
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaveUnit {
    pub unit: String,
    #[serde(default = "one")]
    pub count: u16,
    /// Always this many, whatever the difficulty and growth (a Krogoth).
    #[serde(default)]
    pub boss: bool,
}

/// What position words refer to: the map's size and its start positions (world pixels).
pub struct Ground {
    pub size: (f32, f32),
    pub starts: Vec<(f32, f32)>,
}

impl Ground {
    /// Reads position words: a place, `start N` (the map's start position N), `centre`,
    /// `north`, `south`, `east`, `west`, `northeast`..., `X% Z%` (of the map's size) or `X Z`
    /// (pixels); then, if wanted, `toward <place> by <pixels>` and `+ DX DZ` (pixels).
    pub fn place(&self, words: &str) -> Result<(f32, f32)> {
        let tokens: Vec<&str> = words.split_whitespace().collect();
        let mut i = 0;
        let mut p = self.base(&tokens, &mut i).with_context(|| format!("\"{words}\""))?;
        while i < tokens.len() {
            match tokens[i].to_ascii_lowercase().as_str() {
                "toward" | "towards" => {
                    i += 1;
                    let q = self.base(&tokens, &mut i).with_context(|| format!("\"{words}\""))?;
                    if tokens.get(i).map(|w| w.to_ascii_lowercase()) != Some("by".into()) {
                        bail!("\"{words}\": \"toward\" needs \"by <pixels>\"");
                    }
                    let d = number(tokens.get(i + 1)).with_context(|| format!("\"{words}\""))?;
                    i += 2;
                    let (dx, dz) = (q.0 - p.0, q.1 - p.1);
                    let len = (dx * dx + dz * dz).sqrt().max(1e-3);
                    p = (p.0 + dx / len * d, p.1 + dz / len * d);
                }
                "+" => {
                    let dx = number(tokens.get(i + 1)).with_context(|| format!("\"{words}\""))?;
                    let dz = number(tokens.get(i + 2)).with_context(|| format!("\"{words}\""))?;
                    i += 3;
                    p = (p.0 + dx, p.1 + dz);
                }
                other => bail!("\"{words}\": what is \"{other}\"?"),
            }
        }
        anyhow::ensure!(p.0.is_finite() && p.1.is_finite(), "\"{words}\": position must be finite");
        let margin = 48.0;
        Ok((p.0.clamp(margin, (self.size.0 - margin).max(margin)), p.1.clamp(margin, (self.size.1 - margin).max(margin))))
    }

    fn base(&self, tokens: &[&str], i: &mut usize) -> Result<(f32, f32)> {
        let (w, h) = self.size;
        let word = tokens.get(*i).ok_or_else(|| anyhow!("a place is missing"))?.to_ascii_lowercase();
        *i += 1;
        let edge = 0.07;
        let named = match word.as_str() {
            "centre" | "center" | "middle" => Some((0.5, 0.5)),
            "north" => Some((0.5, edge)),
            "south" => Some((0.5, 1.0 - edge)),
            "east" => Some((1.0 - edge, 0.5)),
            "west" => Some((edge, 0.5)),
            "northeast" => Some((1.0 - edge, edge)),
            "northwest" => Some((edge, edge)),
            "southeast" => Some((1.0 - edge, 1.0 - edge)),
            "southwest" => Some((edge, 1.0 - edge)),
            _ => None,
        };
        if let Some((fx, fz)) = named {
            return Ok((w * fx, h * fz));
        }
        if word == "start" {
            let n = tokens.get(*i).and_then(|n| n.parse::<usize>().ok()).ok_or_else(|| anyhow!("start needs a positive whole number"))?;
            *i += 1;
            return n.checked_sub(1).and_then(|k| self.starts.get(k)).copied().ok_or_else(|| anyhow!("the map has no start position {n}"));
        }
        // Two numbers: percentages of the map's size, or pixels.
        let second = tokens.get(*i).ok_or_else(|| anyhow!("\"{word}\" needs a second number"))?;
        *i += 1;
        let pct = |s: &str| s.strip_suffix('%').map(|v| v.parse::<f32>());
        match (pct(&word), pct(second)) {
            (Some(Ok(x)), Some(Ok(z))) => Ok((w * x / 100.0, h * z / 100.0)),
            (None, None) => Ok((number(Some(&word.as_str()))?, number(Some(second))?)),
            _ => bail!("\"{word} {second}\": both in % or both in pixels"),
        }
    }
}

fn number(token: Option<&&str>) -> Result<f32> {
    let s = token.ok_or_else(|| anyhow!("a number is missing"))?;
    let value = s.parse::<f32>().map_err(|_| anyhow!("\"{s}\" is not a number"))?;
    anyhow::ensure!(value.is_finite(), "\"{s}\" is not a finite number");
    Ok(value)
}

/// The difficulty from its word; `None` for "chosen" (the player's pick) or nothing.
fn difficulty_word(word: &str) -> Option<Difficulty> {
    match word.to_ascii_lowercase().as_str() {
        "easy" => Some(Difficulty::Easy),
        "normal" | "medium" => Some(Difficulty::Normal),
        "hard" => Some(Difficulty::Hard),
        _ => None,
    }
}

/// A difficulty as the records keep it.
pub fn difficulty_key(d: Difficulty) -> &'static str {
    match d {
        Difficulty::Easy => "easy",
        Difficulty::Normal => "normal",
        Difficulty::Hard => "hard",
    }
}

fn rank(d: Difficulty) -> u8 {
    match d {
        Difficulty::Easy => 0,
        Difficulty::Normal => 1,
        Difficulty::Hard => 2,
    }
}

/// How many of the enemy's waves a difficulty brings, against Normal's.
fn wave_scale(d: Difficulty) -> f32 {
    match d {
        Difficulty::Easy => 0.6,
        Difficulty::Normal => 1.0,
        Difficulty::Hard => 1.5,
    }
}

/// What the player sees of a goal while playing (see `hud.rs`).
#[derive(Clone, Debug, PartialEq)]
pub enum GoalView {
    Survive { ticks: u64 },
    Hold { ticks: u64 },
    EnemyHolds { ticks: u64 },
    Reach { what: String },
    Destroy { what: String },
    /// Every one of a type (the Krogoths, both Intimidators): its name.
    DestroyEvery { unit: String },
    Metal { metal: f32 },
    EnemyMetal { metal: f32 },
    Time { ticks: u64 },
    Lose { what: String },
}

/// A transmission due (see [`MessageSpec`]): its tick, who speaks and what, whose link it
/// comes on, and the title its card shows.
#[derive(Clone, Debug)]
pub struct Transmission {
    pub tick: u64,
    pub from: String,
    pub text: String,
    pub player: u8,
    pub title: &'static str,
}

/// The scenario being played, for the HUD, the pause menu and the records.
#[derive(Clone, Debug)]
pub struct ScenarioRun {
    pub id: String,
    pub title: String,
    pub briefing: String,
    pub kind: Kind,
    pub difficulty: Difficulty,
    pub victory: Vec<GoalView>,
    pub defeat: Vec<GoalView>,
    /// Places to mark on the map: (x, z, radius).
    pub areas: Vec<(f32, f32, f32)>,
    /// Waves in all (0: none; endless ones count their cap).
    pub waves: u32,
    /// A goal asks for every enemy unit (or every mobile one) destroyed: the last few are
    /// marked on radar once no more waves are due (they may be anywhere by then).
    pub hunt_last: Option<bool>,
    /// The time of day it sets, if it does.
    pub time: Option<crate::stage::Hours>,
    /// Its transmissions, soonest first, and how many have come.
    pub messages: Vec<Transmission>,
    pub next_message: usize,
}

impl Scenario {
    pub fn parse(source: &str) -> Result<Scenario> {
        let s: Scenario = toml::from_str(source)?;
        anyhow::ensure!(s.enemies.len() + s.allies.len() < ta_sim::unit::MAX_PLAYERS, "too many scenario players");
        if s.id.trim().is_empty() || s.victory.is_empty() && s.kind != Kind::Endless {
            bail!("a scenario needs an id and something to win");
        }
        let difficulty = |word: &str| -> Result<Difficulty> {
            difficulty_word(word).ok_or_else(|| anyhow!("unknown scenario difficulty {word}"))
        };
        for force in s.forces() {
            if let Some(ai) = force.ai.as_deref()
                && !ai.eq_ignore_ascii_case("none") && !ai.eq_ignore_ascii_case("chosen")
            { difficulty(ai)?; }
            for placement in &force.units {
                anyhow::ensure!(placement.spacing.is_none_or(|s| s.is_finite() && (16.0..=2048.0).contains(&s)), "placement spacing must be between 16 and 2048 pixels");
                anyhow::ensure!(placement.formation.is_some() || (placement.spacing.is_none() && placement.facing.is_none()), "spacing and facing need a formation");
                let from = placement.from.as_deref().map(difficulty).transpose()?;
                let until = placement.until.as_deref().map(difficulty).transpose()?;
                anyhow::ensure!(from.zip(until).is_none_or(|(from, until)| rank(from) <= rank(until)), "a placement's minimum difficulty exceeds its maximum");
            }
        }
        match &s.time {
            Some(crate::stage::Hours::At(hour)) => anyhow::ensure!(hour.is_finite(), "scenario time must be finite"),
            Some(crate::stage::Hours::Keys(keys)) => anyhow::ensure!(!keys.is_empty() && keys.iter().all(|[time, hour]| time.is_finite() && *time >= 0.0 && hour.is_finite()), "scenario time needs finite [seconds, hours] keys with nonnegative seconds"),
            None => {}
        }
        for m in &s.messages {
            s.side_player(m.side.as_deref())?;
            if let Some(word) = &m.from_difficulty { difficulty(word)?; }
        }
        for goal in s.victory.iter().chain(&s.defeat) {
            if let GoalSpec::Hold { radius, .. } | GoalSpec::EnemyHolds { radius, .. } | GoalSpec::Reach { radius, .. } = goal {
                anyhow::ensure!(radius.is_finite() && *radius > 0.0, "a goal's radius must be finite and positive");
            }
        }
        for w in &s.waves {
            if let Some(word) = &w.from_difficulty { difficulty(word)?; }
            anyhow::ensure!(w.ally.is_none_or(|k| (1..=s.allies.len()).contains(&k)), "a wave for an ally the scenario hasn't");
            anyhow::ensure!(w.player as u8 + w.ally.is_some() as u8 + w.enemy.is_some() as u8 <= 1, "a wave is the player's, an ally's or an enemy's");
            anyhow::ensure!(w.grow.is_finite() && w.grow >= 0.0, "wave growth must be finite and nonnegative");
            // (An enemy's wave needs that enemy: else its units would be nobody's.)
            if !w.player && w.ally.is_none() {
                anyhow::ensure!(!s.enemies.is_empty(), "an enemy wave in a scenario without enemies");
                anyhow::ensure!(w.enemy.is_none_or(|k| (1..=s.enemies.len()).contains(&k)), "a wave for an enemy the scenario hasn't");
            }
        }
        Ok(s)
    }

    /// The forces in player order: the player, the enemies, then the allies.
    pub fn forces(&self) -> impl Iterator<Item = &Force> {
        std::iter::once(&self.player).chain(&self.enemies).chain(&self.allies)
    }

    /// An ally's player number (`k` from 0).
    fn ally_player(&self, k: usize) -> u8 {
        (1 + self.enemies.len() + k) as u8
    }

    /// The player a message's side names: the player (0), "enemy N" or "ally N".
    fn side_player(&self, side: Option<&str>) -> Result<u8> {
        let words = side.unwrap_or("player").trim().to_ascii_lowercase();
        let mut parts = words.split_whitespace();
        let (what, which) = (parts.next(), parts.next().map(str::parse::<usize>));
        anyhow::ensure!(parts.next().is_none(), "\"{words}\": too many words in a side");
        match (what, which) {
            (Some("player"), None) => Ok(0),
            (Some("enemy"), Some(Ok(k))) if (1..=self.enemies.len()).contains(&k) => Ok(k as u8),
            (Some("ally"), Some(Ok(k))) if (1..=self.allies.len()).contains(&k) => Ok(self.ally_player(k - 1)),
            _ => bail!("\"{words}\": a side is \"player\", \"enemy N\" or \"ally N\""),
        }
    }

    /// Where each force stands, in player order.
    pub fn starts(&self, ground: &Ground) -> Result<Vec<(f32, f32)>> {
        self.forces().map(|f| ground.place(&f.start)).collect()
    }

    /// Players nobody plays (their units follow their orders).
    pub fn passive(&self, player: u8) -> bool {
        player > 0 && self.forces().nth(player as usize).is_some_and(|f| f.ai.as_deref().is_none_or(|a| a.eq_ignore_ascii_case("none")))
    }

    /// The game setup that starts it.
    pub fn setup(&self, source: &str, difficulty: Difficulty, assets: &Assets, seed: u64) -> Result<GameSetup> {
        check(self, assets)?;
        let map = find_map(assets, &self.map).ok_or_else(|| anyhow!("needs the map {}", self.map))?;
        let mut players = vec![SlotSetup { side: self.player.side.clone(), color: 0, controller: Controller::Human, start: 0, team: 0, name: String::new() }];
        for (i, e) in self.enemies.iter().enumerate() {
            let d = e.ai.as_deref().and_then(difficulty_word).unwrap_or(difficulty);
            players.push(SlotSetup { side: e.side.clone(), color: [1, 3, 5, 7, 2][i % 5], controller: Controller::Ai(d), start: i + 1, team: 1, name: String::new() });
        }
        // Allies: the player's team, in colours near its own.
        for (i, a) in self.allies.iter().enumerate() {
            let d = a.ai.as_deref().and_then(difficulty_word).unwrap_or(difficulty);
            players.push(SlotSetup { side: a.side.clone(), color: [8, 6, 4, 9][i % 4], controller: Controller::Ai(d), start: 1 + self.enemies.len() + i, team: 0, name: String::new() });
        }
        let rules = ta_sim::GameRules {
            start_metal: self.player.metal,
            start_energy: self.player.energy,
            fog_of_war: self.fog_of_war,
            mapped: self.mapped,
            commander_ends_game: false,
            mutators: self.mutators.unwrap_or_default(),
            ..Default::default()
        };
        Ok(GameSetup {
            map,
            players,
            local_player: Some(0),
            seed,
            mission: None,
            rules,
            conquest: None,
            scenario: Some(ScenarioStart { id: self.id.clone(), difficulty, source: source.to_string() }),
        })
    }

    /// Places everything and installs the goals and waves (for `Game::new`).
    pub fn apply(&self, sim: &mut Sim, difficulty: Difficulty, ground: &Ground) -> Result<ScenarioRun> {
        self.check_content(ground, &sim.data, &sim.types)?;
        anyhow::ensure!(sim.players.len() == self.forces().count(), "scenario forces do not match the game setup");
        let starts = self.starts(ground)?;
        let fx = |(x, z): (f32, f32)| FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z));
        let unit = |sim: &Sim, name: &str, owner: u8| self.unit_type(&sim.data, &sim.types, name, owner);
        sim.commander_ends_game = false;
        sim.los_enabled = self.fog_of_war;
        sim.set_mutators(self.mutators.unwrap_or_default());
        for (i, force) in self.forces().enumerate() {
            let owner = i as u8;
            sim.set_starting_resources(owner, Fx::from_int(force.metal.min(i64::MAX as u64) as i64), Fx::from_int(force.energy.min(i64::MAX as u64) as i64));
            let start = starts[i];
            // Units face the middle of the battle: the enemy's start (the first enemy's for the
            // player and its allies, the player's for an enemy).
            let enemy_force = (1..=self.enemies.len()).contains(&i);
            let facing = if enemy_force { starts[0] } else { starts.get(1).copied().unwrap_or(start) };
            let heading = ta_sim::fixed::heading_of(fx(facing) - fx(start));
            if force.commander {
                let t = unit(sim, "commander", owner).ok_or_else(|| anyhow!("no commander for {}", force.side))?;
                anyhow::ensure!(!sim.place_units_facing(t, owner, fx(start), 1, heading).is_empty(), "could not place the commander for player {}", i + 1);
            }
            // Claim ore before building the base, and finish buildings before parking its
            // garrison. Keep the original order for embedded older scenario sources.
            let mut placements: Vec<_> = force.units.iter().collect();
            placements.sort_by_key(|p| {
                if p.formation.is_none() { 0 }
                else if p.formation == Some(layout::Formation::Deposits) { 1 }
                else if unit(sim, &p.unit, owner).is_some_and(|t| sim.types.get(t).locomotion == ta_sim::Locomotion::Static) { 2 }
                else { 3 }
            });
            for p in placements {
                if p.from.as_deref().and_then(difficulty_word).is_some_and(|d| rank(difficulty) < rank(d)) || p.until.as_deref().and_then(difficulty_word).is_some_and(|d| rank(difficulty) > rank(d)) {
                    continue;
                }
                let t = unit(sim, &p.unit, owner).ok_or_else(|| anyhow!("no unit {}", p.unit))?;
                let at = ground.place(&p.at)?;
                let placed = if p.formation.is_some() {
                    let face = p.facing.as_deref().map(|w| ground.place(w)).transpose()?.unwrap_or(facing);
                    layout::place(sim, t, owner, p, at, face, start)
                        .with_context(|| format!("{}: player {}, {} at {}", self.id, i + 1, p.unit, p.at))?
                } else {
                    // Legacy scatter, including its route toward the opposing force.
                    sim.place_units_bound(t, owner, fx(at), p.count, heading, Some(fx(facing)))
                };
                let patrol = p.patrol.as_deref().map(|w| ground.place(w)).transpose()?;
                for id in placed {
                    if p.xp > 0 {
                        sim.set_experience(id, Fx::from_int(p.xp as i64));
                    }
                    if p.radar_guided {
                        sim.set_radar_guided(id, true);
                    }
                    let mut orders = Vec::new();
                    if p.hold {
                        // Fire at will, hold position.
                        orders.push(ta_sim::Order::SetStates { fire: 2, movement: 0 });
                    }
                    if let Some(to) = patrol {
                        orders.push(ta_sim::Order::Patrol(fx(to)));
                        orders.push(ta_sim::Order::Patrol(fx(at)));
                    }
                    sim.give_orders(id, orders);
                }
            }
            if i == 0 && self.mapped {
                sim.reveal_map_for(0);
            }
        }
        // Waves: every enemy that sends them keeps playing without units of its own.
        let mut waves = Vec::new();
        for w in &self.waves {
            if w.from_difficulty.as_deref().and_then(difficulty_word).is_some_and(|d| rank(difficulty) < rank(d)) {
                continue;
            }
            // (The player's and its allies' reinforcements are nobody's waves.)
            let (owner, friendly) = match (w.player, w.ally) {
                (true, _) => (0, true),
                (false, Some(k)) => (self.ally_player(k.clamp(1, self.allies.len().max(1)) - 1), true),
                (false, None) => (w.enemy.unwrap_or(1).clamp(1, self.enemies.len().max(1)) as u8, false),
            };
            let froms: Vec<(f32, f32)> = w.from.split('|').map(|f| ground.place(f)).collect::<Result<_>>()?;
            let to = w.to.as_deref().map(|t| ground.place(t)).transpose()?;
            let repeats = if w.every == 0 { 0 } else if w.times == 0 { MAX_REPEATS } else { w.times.min(MAX_REPEATS) };
            let mut scale = wave_scale(difficulty);
            for k in 0..=repeats {
                let units: Vec<(usize, u16)> = w
                    .units
                    .iter()
                    .filter(|u| u.count > 0 && (u.boss || scale > 0.0))
                    .filter_map(|u| {
                        let t = unit(sim, &u.unit, owner)?;
                        let n = if u.boss { u.count } else { (u.count as f32 * scale).round().clamp(1.0, MAX_WAVE_UNITS as f32) as u16 };
                        Some((t, n))
                    })
                    .collect();
                if !units.is_empty() {
                    waves.push(Wave { tick: w.tick(k), owner, units, from: fx(froms[k as usize % froms.len()]), target: to.map(fx), hunt: !w.stay && !friendly, friendly });
                }
                scale *= w.grow.max(0.0);
            }
        }
        waves.sort_by_key(|w| w.tick);
        for wave in waves.iter().filter(|w| !w.friendly) {
            if let Some(player) = sim.players.get_mut(wave.owner as usize) { player.endless = true; }
        }
        let total_waves = waves.iter().filter(|w| !w.friendly).count() as u32;
        // What the enemy's waves bring (a goal to destroy every one of a type counts them).
        let coming: Vec<usize> = waves.iter().filter(|w| !w.friendly).flat_map(|w| w.units.iter().map(|u| u.0)).collect();
        // Goals.
        let secs = |s: u32| s as u64 * 30;
        let mut rules = MissionRules { human: 0, waves, ..Default::default() };
        let mut run = ScenarioRun {
            id: self.id.clone(),
            title: self.title.clone(),
            briefing: self.briefing.clone(),
            kind: self.kind,
            difficulty,
            victory: Vec::new(),
            defeat: Vec::new(),
            areas: Vec::new(),
            waves: total_waves,
            hunt_last: None,
            time: self.time.clone(),
            messages: Vec::new(),
            next_message: 0,
        };
        for m in &self.messages {
            if m.from_difficulty.as_deref().and_then(difficulty_word).is_some_and(|d| rank(difficulty) < rank(d)) {
                continue;
            }
            let player = self.side_player(m.side.as_deref())?;
            let title = if player == 0 {
                n("HEADQUARTERS")
            } else if (1..=self.enemies.len()).contains(&(player as usize)) {
                n("ENEMY COMMAND")
            } else {
                n("ALLIED COMMAND")
            };
            run.messages.push(Transmission { tick: m.at as u64 * 30, from: m.from.clone(), text: m.text.clone(), player, title });
        }
        run.messages.sort_by_key(|m| m.tick);
        let enemy = 1u8;
        for (goal, winning) in self.victory.iter().map(|g| (g, true)).chain(self.defeat.iter().map(|g| (g, false))) {
            let (objective, view) = match goal {
                GoalSpec::Survive { survive } => (Objective::Timer(secs(*survive)), GoalView::Survive { ticks: secs(*survive) }),
                GoalSpec::Time { time } => (Objective::Timer(secs(*time)), GoalView::Time { ticks: secs(*time) }),
                GoalSpec::Hold { hold, radius, seconds } => {
                    let (x, z) = ground.place(hold)?;
                    run.areas.push((x, z, *radius));
                    (Objective::HoldArea { pos: fx((x, z)), radius: Fx::from_f32(*radius), ticks: secs(*seconds) }, GoalView::Hold { ticks: secs(*seconds) })
                }
                GoalSpec::EnemyHolds { enemy_holds, radius, seconds } => {
                    let (x, z) = ground.place(enemy_holds)?;
                    if !run.areas.iter().any(|a| (a.0 - x).abs() < 1.0 && (a.1 - z).abs() < 1.0) {
                        run.areas.push((x, z, *radius));
                    }
                    (Objective::EnemyHoldsArea { pos: fx((x, z)), radius: Fx::from_f32(*radius), ticks: secs(*seconds) }, GoalView::EnemyHolds { ticks: secs(*seconds) })
                }
                GoalSpec::Reach { reach, radius, unit: which } => {
                    let (x, z) = ground.place(reach)?;
                    run.areas.push((x, z, *radius));
                    let t = match which.as_deref() {
                        None => None,
                        Some(name) if name.eq_ignore_ascii_case("any") => None,
                        Some(name) => Some(unit(sim, name, 0).ok_or_else(|| anyhow!("no unit {name}"))?),
                    };
                    let what = match which.as_deref() {
                        Some(name) if name.eq_ignore_ascii_case("commander") => t_str("your Commander"),
                        _ => t.map_or_else(|| t_str("any of your units"), |t| crate::chat::unit_label(sim, t)),
                    };
                    (Objective::MoveUnitToRadius { unit: t, pos: fx((x, z)), radius: Fx::from_f32(*radius) }, GoalView::Reach { what })
                }
                GoalSpec::Destroy { destroy } => match destroy.to_ascii_lowercase().as_str() {
                    "commander" => {
                        let what = if self.enemies.len() > 1 { t_str("the enemy Commanders") } else { t_str("the enemy Commander") };
                        (Objective::KillEnemyCommander, GoalView::Destroy { what })
                    }
                    "everything" => {
                        run.hunt_last = Some(false);
                        (Objective::DestroyAllUnits, GoalView::Destroy { what: t_str("every enemy unit") })
                    }
                    "army" => {
                        run.hunt_last = Some(true);
                        (Objective::KillAllMobileUnits, GoalView::Destroy { what: t_str("the enemy's army") })
                    }
                    _ => {
                        let t = unit(sim, destroy, enemy).ok_or_else(|| anyhow!("no unit {destroy}"))?;
                        // (Brought by waves still to come too: every one of them.)
                        let objective = if coming.contains(&t) { Objective::KillAllOfTypeToCome(t) } else { Objective::KillAllOfType(t) };
                        (objective, GoalView::DestroyEvery { unit: crate::chat::unit_label(sim, t) })
                    }
                },
                GoalSpec::Lose { lose } => match lose.to_ascii_lowercase().as_str() {
                    "commander" => (Objective::CommanderKilled, GoalView::Lose { what: t_str("your Commander") }),
                    "everything" => (Objective::AllUnitsKilled, GoalView::Lose { what: t_str("all your units") }),
                    _ => {
                        let t = unit(sim, lose, 0).ok_or_else(|| anyhow!("no unit {lose}"))?;
                        (Objective::UnitTypeKilled { unit: t, count: 1 }, GoalView::Lose { what: crate::chat::unit_label(sim, t) })
                    }
                },
                GoalSpec::Metal { metal } => (Objective::MetalMade(Fx::from_int((*metal).min(i64::MAX as u64) as i64)), GoalView::Metal { metal: *metal as f32 }),
                GoalSpec::EnemyMetal { enemy_metal } => (Objective::EnemyMetalMade(Fx::from_int((*enemy_metal).min(i64::MAX as u64) as i64)), GoalView::EnemyMetal { metal: *enemy_metal as f32 }),
            };
            if winning {
                rules.victory.push(objective);
                run.victory.push(view);
            } else {
                rules.defeat.push(objective);
                run.defeat.push(view);
            }
        }
        sim.mission = Some(rules);
        Ok(run)
    }

    fn unit_type(&self, data: &ta_data::GameData, types: &ta_sim::UnitTypes, name: &str, owner: u8) -> Option<usize> {
        if name.eq_ignore_ascii_case("commander") {
            let force = self.forces().nth(owner as usize)?;
            types.id(&force_side(data, &force.side)?.commander)
        } else { types.id(name) }
    }

    /// Shared by menu/CLI validation and battle setup, before mutating a simulation.
    fn check_content(&self, ground: &Ground, data: &ta_data::GameData, types: &ta_sim::UnitTypes) -> Result<()> {
        let unit = |name: &str, owner| -> Result<()> {
            anyhow::ensure!(self.unit_type(data, types, name, owner).is_some(), "{}", tf("Needs the unit {unit}", &[("unit", &name)]));
            Ok(())
        };
        for (owner, force) in self.forces().enumerate() {
            anyhow::ensure!(force_side(data, &force.side).is_some(), "{}", tf("Needs the side {side}", &[("side", &force.side)]));
            ground.place(&force.start)?;
            if force.commander { unit("commander", owner as u8)?; }
            for placement in &force.units {
                ground.place(&placement.at)?;
                if let Some(facing) = &placement.facing { ground.place(facing)?; }
                if let Some(patrol) = &placement.patrol { ground.place(patrol)?; }
                unit(&placement.unit, owner as u8)?;
                let t = self.unit_type(data, types, &placement.unit, owner as u8).unwrap();
                anyhow::ensure!(placement.formation != Some(layout::Formation::Deposits) || types.get(t).extracts_metal > Fx::ZERO, "deposit formations need a metal extractor");
            }
        }
        for wave in &self.waves {
            for from in wave.from.split('|') { ground.place(from)?; }
            if let Some(to) = &wave.to { ground.place(to)?; }
            let owner = if wave.player { 0 } else if let Some(ally) = wave.ally { self.ally_player(ally - 1) } else { wave.enemy.unwrap_or(1) as u8 };
            for which in &wave.units { unit(&which.unit, owner)?; }
        }
        for goal in self.victory.iter().chain(&self.defeat) {
            match goal {
                GoalSpec::Hold { hold, .. } => { ground.place(hold)?; }
                GoalSpec::EnemyHolds { enemy_holds, .. } => { ground.place(enemy_holds)?; }
                GoalSpec::Reach { reach, unit: which, .. } => {
                    ground.place(reach)?;
                    if let Some(which) = which && !which.eq_ignore_ascii_case("any") { unit(which, 0)?; }
                }
                GoalSpec::Destroy { destroy } if !["commander", "everything", "army"].iter().any(|word| destroy.eq_ignore_ascii_case(word)) => unit(destroy, 1)?,
                GoalSpec::Lose { lose } if !["commander", "everything"].iter().any(|word| lose.eq_ignore_ascii_case(word)) => unit(lose, 0)?,
                _ => {}
            }
        }
        Ok(())
    }
}

fn t_str(text: &str) -> String {
    t(text).to_string()
}

/// A force's side key, including a mod's distinct races sharing the same name.
fn force_side<'a>(data: &'a ta_data::GameData, key: &str) -> Option<&'a ta_data::SideDef> {
    match key.split_once('#') {
        Some((name, index)) => data.sides.get(index.parse::<usize>().ok()?).filter(|s| s.name.eq_ignore_ascii_case(name)),
        None => data.sides.iter().find(|s| s.name.eq_ignore_ascii_case(key)),
    }
}

/// A map of the catalogue by its title or file name; its file name.
fn find_map(assets: &Assets, name: &str) -> Option<String> {
    assets.maps.iter().find(|m| m.name.eq_ignore_ascii_case(name) || m.title.eq_ignore_ascii_case(name)).map(|m| m.name.clone())
}

/// A scenario the menu offers: where it came from, and what is wrong with it if it can't be
/// played here.
#[derive(Clone, Debug)]
pub struct Entry {
    pub scenario: Scenario,
    pub source: String,
    /// From the settings folder rather than built in.
    pub custom: bool,
    /// Why it can't be played (a missing map, a unit the game lacks...).
    pub problem: Option<String>,
}

/// Every scenario: the built-in ones, then the settings folder's (`scenarios/*.toml`).
pub fn catalogue(assets: &Assets) -> Vec<Entry> {
    catalogue_and_unread(assets).0
}

/// [`catalogue`], and the files of the settings folder that could not be read, and why (their
/// file names: they were left out without a word).
pub fn catalogue_and_unread(assets: &Assets) -> (Vec<Entry>, Vec<(String, String)>) {
    let mut unread: Vec<(String, String)> = Vec::new();
    let name = |file: &str| std::path::Path::new(file).file_name().map_or(file.to_string(), |n| n.to_string_lossy().into_owned());
    let mut sources: Vec<(String, bool, String)> = BUILT_IN.iter().map(|s| (s.to_string(), false, String::new())).collect();
    let dir = ta_config::config_dir().join("scenarios");
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut files: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml"))).collect();
        files.sort();
        for f in files {
            match std::fs::read_to_string(&f) {
                Ok(text) => sources.push((text, true, f.display().to_string())),
                Err(e) => {
                    log::warn!("scenario {}: {e}", f.display());
                    unread.push((name(&f.display().to_string()), e.to_string()));
                }
            }
        }
    }
    let mut out: Vec<Entry> = Vec::new();
    for (source, custom, file) in sources {
        match Scenario::parse(&source) {
            Ok(scenario) => {
                if out.iter().any(|e| e.scenario.id.eq_ignore_ascii_case(&scenario.id)) {
                    log::warn!("scenario {file}: the id {} is taken", scenario.id);
                    unread.push((name(&file), ta_ui::i18n::tf("another scenario has its id, {id}", &[("id", &scenario.id)])));
                    continue;
                }
                let problem = check(&scenario, assets).err().map(|e| format!("{e:#}"));
                out.push(Entry { scenario, source, custom, problem });
            }
            Err(e) => {
                log::warn!("scenario {file}: {e:#}");
                unread.push((name(&file), format!("{e:#}")));
            }
        }
    }
    (out, unread)
}

/// The setup of the scenario with this id (`--scenario`).
pub fn setup_by_id(assets: &Assets, id: &str, difficulty: Difficulty, seed: u64) -> Option<GameSetup> {
    let entry = catalogue(assets).into_iter().find(|e| e.scenario.id.eq_ignore_ascii_case(id))?;
    entry.scenario.setup(&entry.source, difficulty, assets, seed).map_err(|e| log::warn!("scenario {id}: {e:#}")).ok()
}

/// The difficulty `TA_SCENARIO_DIFFICULTY` asks for (Normal by default).
pub fn difficulty_from_env() -> Difficulty {
    std::env::var("TA_SCENARIO_DIFFICULTY").ok().as_deref().and_then(difficulty_word).unwrap_or(Difficulty::Normal)
}

/// Whether a scenario can be played with this game data: its map, and its units' names.
fn check(s: &Scenario, assets: &Assets) -> Result<()> {
    let map = find_map(assets, &s.map).ok_or_else(|| anyhow!("{}", tf("Needs the map {map}", &[("map", &s.map)])))?;
    let entry = assets.maps.iter().find(|m| m.name == map);
    let mut starts: Vec<(f32, f32)> = entry.and_then(|m| m.info.schemas.first()).map(|sc| sc.start_positions.iter().map(|&(_, x, z)| (x, z)).collect()).unwrap_or_default();
    // Runtime pads starts to the force count. Reserve those indexes for this
    // metadata-only check; apply rechecks their actual coordinates on the loaded map.
    starts.resize(starts.len().max(s.forces().count()), (2048.0, 2048.0));
    // (Only whether the words make sense: the size is the map's once it is loaded.)
    let ground = Ground { size: (4096.0, 4096.0), starts };
    s.check_content(&ground, &assets.data, &assets.types)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground() -> Ground {
        Ground { size: (4000.0, 2000.0), starts: vec![(500.0, 1000.0), (3500.0, 1000.0)] }
    }

    #[test]
    fn position_words() {
        let g = ground();
        assert_eq!(g.place("centre").unwrap(), (2000.0, 1000.0));
        assert_eq!(g.place("start 2").unwrap(), (3500.0, 1000.0));
        assert_eq!(g.place("25% 50%").unwrap(), (1000.0, 1000.0));
        assert_eq!(g.place("1234 567").unwrap(), (1234.0, 567.0));
        assert_eq!(g.place("start 1 toward start 2 by 600").unwrap(), (1100.0, 1000.0));
        assert_eq!(g.place("start 1 toward centre by 100 + 0 -50").unwrap(), (600.0, 950.0));
        let (x, z) = g.place("north").unwrap();
        assert!((x - 2000.0).abs() < 1.0 && z < 200.0);
        // Kept on the map.
        assert_eq!(g.place("start 1 + -2000 0").unwrap().0, 48.0);
        for bad in ["start 3", "somewhere", "start 1 toward", "start 1 toward start 2", "10% 20", "start 1 + 5"] {
            assert!(g.place(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn nonfinite_positions_and_fractional_start_indices_are_rejected() {
        let g = ground();
        for words in ["nan 100", "100 inf", "nan% 50%", "50% inf%", "1e39 100",
            "centre + nan 0", "centre toward north by inf", "start 1.5", "start -1", "start nan"]
        {
            assert!(g.place(words).is_err(), "invalid position was accepted: {words}");
        }
        assert_eq!(g.place("-100 9999999").unwrap(), (48.0, 1952.0), "ordinary finite positions still clamp to the map");
    }

    #[test]
    fn long_wave_intervals_remain_ordered_instead_of_wrapping_to_the_start() {
        let wave: WaveSpec = toml::from_str("at = 4294967295\nevery = 4294967295\nfrom = 'north'\nunits = []").unwrap();
        assert_eq!(wave.tick(0), 128_849_018_850);
        assert_eq!(wave.tick(1), 257_698_037_700);
        assert!(wave.tick(MAX_REPEATS) > wave.tick(MAX_REPEATS - 1));
    }

    #[test]
    fn malformed_scenario_goals_growth_and_ambiguous_wave_owners_are_rejected() {
        let source = r#"
id = "test"
title = "Test"
tagline = "Test"
briefing = "Test"
map = "Test"
kind = "battle"
victory = [{hold = "centre", radius = 100.0, seconds = 10}]
[player]
side = "ARM"
start = "start 1"
[[enemy]]
side = "CORE"
start = "start 2"
[[ally]]
side = "ARM"
start = "start 1"
[[wave]]
at = 30
from = "north"
grow = 1.0
enemy = 1
units = []
"#;
        let scenario = Scenario::parse(source).unwrap();
        assert!(scenario.side_player(Some("enemy 1 extra")).is_err());
        assert!(Scenario::parse(&source.replace("enemy = 1", "enemy = 1\nally = 1")).is_err());
        for value in ["nan", "inf", "-inf", "-1.0"] {
            assert!(Scenario::parse(&source.replace("grow = 1.0", &format!("grow = {value}"))).is_err());
        }
        for value in ["nan", "inf", "-inf", "-1.0", "0.0"] {
            assert!(Scenario::parse(&source.replace("radius = 100.0", &format!("radius = {value}"))).is_err());
        }
    }

    pub(super) fn test_assets() -> Option<Assets> {
        use std::sync::Arc;
        let install = ta_vfs::install::discover(None).ok()?;
        // (TA Reborn's own maps too, as the game mounts them.) `TA_TEST_MOD=<mod folder>` (its
        // mod.toml, as the launcher installs it): the scenarios over a mod, as the game mounts it.
        let module = std::env::var("TA_TEST_MOD").ok().map(|dir| ta_vfs::mods::read_mod(std::path::Path::new(&dir)).unwrap().expect("TA_TEST_MOD: a mod's folder"));
        let mounted = ta_vfs::mods::mount(Some(&install.root), ta_vfs::builtin_dir().as_deref(), &module.iter().collect::<Vec<_>>(), &|m| m.manifest.id.clone());
        mounted.install.ok()?;
        if let Some(id) = mounted.mods.unwrap().first() {
            println!("mod: {id}");
        }
        let vfs = mounted.vfs;
        let data = Arc::new(ta_data::GameData::load(&vfs));
        let types = Arc::new(ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
        let maps = crate::setup::MapEntry::catalogue(&vfs);
        Some(Assets { vfs: Arc::new(vfs), data, types, selectable_maps: maps.clone(), maps, campaigns: Vec::new(), data_hash: 0, compatibility: Default::default(), mods: Default::default() })
    }

    /// A scenario's battle as `Game::new` makes it (without the pictures): the simulation, what
    /// the HUD shows, the computer players.
    pub(super) fn battle(assets: &Assets, entry: &Entry, difficulty: Difficulty) -> (Sim, ScenarioRun, Vec<ta_ai::SkirmishAi>) {
        let (mut sim, ground) = bare_battle(assets, entry, difficulty);
        let setup = entry.scenario.setup(&entry.source, difficulty, assets, 7).unwrap();
        let run = entry.scenario.apply(&mut sim, difficulty, &ground).unwrap();
        let ais = setup
            .players
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s.controller {
                Controller::Ai(d) if !entry.scenario.passive(i as u8) => Some(ta_ai::SkirmishAi::new(i as u8, &sim, d)),
                _ => None,
            })
            .collect();
        (sim, run, ais)
    }

    /// [`battle`]'s simulation before the scenario places anything (the map's features on
    /// it), and the ground its position words are read on.
    pub(super) fn bare_battle(assets: &Assets, entry: &Entry, difficulty: Difficulty) -> (Sim, Ground) {
        use ta_formats::tdf;
        use ta_formats::tnt::TntMap;
        // (`TA_TEST_SEED=n`: the battle played with another seed, 7 by default.)
        let seed = std::env::var("TA_TEST_SEED").ok().and_then(|s| s.trim().parse().ok()).unwrap_or(7);
        let setup = entry.scenario.setup(&entry.source, difficulty, assets, seed).unwrap();
        let vfs = assets.vfs.as_ref();
        let map = TntMap::parse(&vfs.read(&format!("maps/{}.tnt", setup.map)).unwrap()).unwrap();
        let info = ta_data::MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{}.ota", setup.map)).unwrap()));
        let map_starts = info.schemas.first().map(|s| s.start_positions.iter().map(|&(_, x, z)| (x, z)).collect()).unwrap_or_default();
        let ground = Ground { size: (map.width_px() as f32, map.height_px() as f32), starts: map_starts };
        let starts = entry.scenario.starts(&ground).unwrap();
        let fx = |(x, z): (f32, f32)| FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z));
        let players: Vec<ta_sim::PlayerSetup> = setup
            .players
            .iter()
            .enumerate()
            .map(|(i, s)| ta_sim::PlayerSetup {
                side: crate::setup::resolve_side(&assets.data, &s.side).0.to_string(),
                color: s.color,
                start: fx(starts[i]),
                metal: Fx::ZERO,
                energy: Fx::ZERO,
                ai: matches!(s.controller, Controller::Ai(_)),
            })
            .collect();
        let mut sim = Sim::new(assets.types.clone(), assets.data.for_map(vfs, &setup.map), &map, &info, 0, &players, setup.seed);
        for (i, s) in setup.players.iter().enumerate() {
            sim.set_team(i as u8, s.team);
        }
        (sim, ground)
    }

    #[test]
    fn all_scenario_starting_forces_are_complete() {
        use std::collections::BTreeMap;
        let Some(assets) = test_assets() else { eprintln!("TA install not found; skipping"); return };
        let entries: Vec<_> = catalogue(&assets).into_iter().filter(|e| !e.custom).collect();
        let mut missing = Vec::new();
        let mut report = String::from("scenario,difficulty,owner,unit,x,z,width,depth,building,factory,extractor\n");
        for difficulty in [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard] {
            for entry in &entries {
                let s = &entry.scenario;
                let (sim, _, _) = battle(&assets, entry, difficulty);
                for (owner, force) in s.forces().enumerate() {
                    let mut expected = BTreeMap::<usize, usize>::new();
                    if force.commander { *expected.entry(s.unit_type(&assets.data, &assets.types, "commander", owner as u8).unwrap()).or_default() += 1; }
                    for p in &force.units {
                        if p.from.as_deref().and_then(difficulty_word).is_some_and(|d| rank(difficulty) < rank(d))
                            || p.until.as_deref().and_then(difficulty_word).is_some_and(|d| rank(difficulty) > rank(d)) { continue; }
                        *expected.entry(s.unit_type(&assets.data, &assets.types, &p.unit, owner as u8).unwrap()).or_default() += p.count as usize;
                    }
                    for (ty, count) in expected {
                        let actual = sim.units().filter(|u| u.owner as usize == owner && u.type_id == ty && !u.dead && u.is_complete()).count();
                        if actual != count { missing.push(format!("{} {difficulty:?} owner {owner}: {} {actual}/{count}", s.id, assets.types.get(ty).name)); }
                    }
                    let base: Vec<_> = sim.units().filter(|u| u.owner as usize == owner).collect();
                    if base.iter().any(|u| sim.unit_type(u).is_factory) {
                        let mines = base.iter().filter(|u| sim.unit_type(u).extracts_metal > Fx::ZERO).count();
                        assert!(mines >= 2, "{} {difficulty:?} owner {owner}: base needs working metal extraction", s.id);
                        let surplus = base.iter().fold(Fx::ZERO, |total, u| { let f = sim.type_flows(u.type_id, u.pos); total + f.energy_make - f.energy_use });
                        assert!(surplus >= Fx::from_int(20), "{} {difficulty:?} owner {owner}: idle energy surplus {surplus:?}", s.id);
                    }
                    let armed_types = base.iter().map(|u| u.type_id).chain(sim.mission.as_ref().unwrap().waves.iter().filter(|w| w.owner as usize == owner).flat_map(|w| w.units.iter().map(|&(t, _)| t)));
                    let shot = armed_types.flat_map(|i| assets.types.get(i).weapons.iter().flatten().map(|&w| sim.weapon_types.get(w)))
                        .filter(|w| !w.def.stockpile && !w.def.command_fire).map(|w| w.energy_per_shot).max().unwrap_or(Fx::ZERO);
                    let power = base.iter().any(|u| sim.type_flows(u.type_id, u.pos).energy_make > Fx::ZERO);
                    assert!(sim.players[owner].economy.energy >= shot || power, "{}: owner {owner} cannot power its energy weapons", s.id);
                }
                for u in sim.units() {
                    let t = sim.unit_type(u);
                    assert!(layout::clear_except(&sim, u.type_id, u.id), "{} {difficulty:?}: {} overlaps another starting unit or a factory exit", s.id, t.name);
                    if t.extracts_metal > Fx::ZERO { assert!(sim.extraction_at(u.type_id, u.pos) > Fx::ZERO, "{}: unproductive {}", s.id, t.name); }
                    if t.is_factory { assert!(layout::factory_exit_clear(&sim, t, u.pos), "{}: blocked {} exit", s.id, t.name); }
                    report.push_str(&format!("{},{difficulty:?},{},{},{:.0},{:.0},{},{},{},{},{}\n", s.id, u.owner, t.name, u.pos.x.to_f32(), u.pos.z.to_f32(), t.footprint.0 * 16, t.footprint.1 * 16, t.locomotion == ta_sim::Locomotion::Static, t.is_factory, t.extracts_metal > Fx::ZERO));
                }
            }
        }
        if let Some(path) = std::env::var_os("TA_LAYOUT_REPORT") { std::fs::write(path, report).unwrap(); }
        assert!(missing.is_empty(), "starting units missing:\n{}", missing.join("\n"));
    }

    /// Exercise original factory scripts and unit movement in each authored base. Extra
    /// resources/build speed isolate the layout from the scenario's combat and economy.
    #[test]
    fn scenario_factories_produce_and_release_units() {
        let Some(assets) = test_assets() else { eprintln!("TA install not found; skipping"); return; };
        let entries: Vec<_> = catalogue(&assets).into_iter().filter(|e| !e.custom).collect();
        for (entry, difficulty) in entries.iter().flat_map(|e| [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard].into_iter().map(move |d| (e, d))) {
            let (mut sim, _, _) = battle(&assets, entry, difficulty);
            sim.mission = None;
            sim.set_mutators(ta_sim::Mutators { build_speed: 16, ..Default::default() });
            for owner in 0..sim.players.len() { sim.set_team(owner as u8, 0); sim.set_starting_resources(owner as u8, Fx::from_int(100_000), Fx::from_int(100_000)); }
            let factories: Vec<_> = sim.units().filter(|u| sim.unit_type(u).is_factory).map(|u| {
                let t = sim.unit_type(u);
                let product = t.build_options.iter().copied().filter(|&i| assets.types.get(i).locomotion != ta_sim::Locomotion::Static)
                    .min_by_key(|&i| assets.types.get(i).build_work).unwrap();
                (u.id, u.owner, product, u.pos)
            }).collect();
            let first_new = sim.units().map(|u| u.id.slot).max().unwrap_or(0) + 1;
            let commands: Vec<_> = factories.iter().map(|&(factory, player, type_id, _)| ta_sim::Command::Produce { player, factory, type_id, count: 2 }).collect();
            sim.step(&commands);
            for _ in 0..30 * 45 { sim.step(&[]); }
            for (factory, owner, product, pos) in factories {
                let built: Vec<_> = sim.units().filter(|u| u.id.slot >= first_new && u.owner == owner && u.type_id == product && u.is_complete() && !u.dead && u.factory.is_none()).collect();
                assert!(built.len() >= 2, "{}: {} failed to release two {} (released {})", entry.scenario.id, sim.unit_type(sim.unit(factory).unwrap()).name, assets.types.get(product).name, built.len());
                assert!(built.iter().any(|u| !u.pos.within_xz(pos, Fx::from_int(80))), "{}: products stuck in factory", entry.scenario.id);
            }
        }
    }

    /// Every built-in scenario on its map: its forces placed, its goals installed, and a minute
    /// of battle (its computer players included) with the waves on time and nothing decided.
    #[test]
    fn every_scenario_sets_up_and_plays() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let entries: Vec<Entry> = catalogue(&assets).into_iter().filter(|e| !e.custom).collect();
        assert_eq!(entries.len(), BUILT_IN.len());
        let difficulty = difficulty_from_env();
        for entry in &entries {
            let s = &entry.scenario;
            assert_eq!(entry.problem, None, "{}", s.id);
            let (mut sim, run, mut ais) = battle(&assets, entry, difficulty);
            let placed = |sim: &Sim, owner: u8| sim.units().filter(|u| u.owner == owner && !u.dead).count();
            let wanted = s.player.units.iter().filter(|p| {
                p.from.as_deref().and_then(difficulty_word).is_none_or(|d| rank(difficulty) >= rank(d))
                    && p.until.as_deref().and_then(difficulty_word).is_none_or(|d| rank(difficulty) <= rank(d))
            }).map(|p| p.count as usize).sum::<usize>() + s.player.commander as usize;
            assert!(placed(&sim, 0) >= wanted * 9 / 10, "{}: {} of the player's {wanted} units placed", s.id, placed(&sim, 0));
            assert_eq!(run.victory.len(), s.victory.len(), "{}", s.id);
            let due = sim.mission.as_ref().unwrap().waves.iter().filter(|w| w.tick <= 60 * 30 && !w.friendly).count() as u32;
            let mut arrived = 0;
            for _ in 0..60 * 30 {
                let mut commands = Vec::new();
                for ai in &mut ais {
                    commands.extend(ai.think(&sim));
                }
                sim.step(&commands);
                for e in sim.take_events() {
                    if let ta_sim::SimEvent::WaveArrived { units, owner, .. } = e {
                        assert!(units > 0, "{}: an empty wave", s.id);
                        arrived += sim.is_enemy(owner, 0) as u32;
                    }
                }
            }
            // (Allies on the player's side, enemies not.)
            for k in 0..s.allies.len() {
                assert!(sim.allied(0, s.ally_player(k)), "{}: ally {k}", s.id);
            }
            for k in 1..=s.enemies.len() {
                assert!(sim.is_enemy(0, k as u8), "{}: enemy {k}", s.id);
            }
            let m = sim.mission.as_ref().unwrap();
            assert_eq!(arrived, due, "{}: waves in the first minute", s.id);
            assert_eq!(m.waves_sent, due, "{}", s.id);
            assert_eq!(m.outcome, None, "{}: decided in the first minute", s.id);
            assert!(!sim.players[0].defeated, "{}", s.id);
            println!("{} ({difficulty:?}): {} of ours, {} of theirs after a minute; {} waves in all", s.id, placed(&sim, 0), sim.units().filter(|u| u.owner != 0 && !u.dead).count(), run.waves);
        }
    }

    /// A scenario played to its end by a computer player in the player's place (a rough idea
    /// of how hard it is): `TA_SCENARIO=<id>` (all by default), `TA_SCENARIO_DIFFICULTY`,
    /// `TA_TEST_MINUTES` (the most to play, 20); `TA_PILOT=none` leaves the player's side to
    /// itself, `TA_PILOT_TO=x,z` sends it all there at the start (the Commander behind; several
    /// points, `x,z|x,z`, one after another), `TA_PILOT_ATTACK=1` its aircraft at the enemy
    /// Commander; `TA_TEST_SEED=n` another seed; `TA_TEST_LOG=1` says where the enemy's units
    /// are at the end, which kinds fired and killed, what killed the Commander, and what stood
    /// on hold fire a minute in; `TA_TEST_WATCH=CORARAD,CORINT` when each enemy unit of those
    /// kinds fell, and how often they fired each minute.
    #[test]
    #[ignore]
    fn autopilot() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let wanted = std::env::var("TA_SCENARIO").ok();
        let difficulty = difficulty_from_env();
        let minutes: u64 = std::env::var("TA_TEST_MINUTES").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
        for entry in catalogue(&assets).iter().filter(|e| wanted.as_deref().is_none_or(|w| e.scenario.id == w)) {
            let (mut sim, _, mut ais) = battle(&assets, entry, difficulty);
            // (`TA_PILOT=none`: nobody plays the player's side; its units hold and fight back.)
            let piloted = std::env::var("TA_PILOT").ok().as_deref() != Some("none");
            let mut pilot = ta_ai::SkirmishAi::new(0, &sim, Difficulty::Hard);
            let mut tick = 0;
            // (`TA_TEST_LOG`: which kinds fired how often and killed how many, and what killed
            // the player's Commander.)
            let log = std::env::var("TA_TEST_LOG").is_ok();
            let mut shots: std::collections::BTreeMap<(&str, String), u32> = Default::default();
            let mut kills: std::collections::BTreeMap<(&str, String), u32> = Default::default();
            let mut fallen: Option<(u64, Option<(&str, String)>)> = None;
            let mut hurt_by: Option<(&str, String)> = None;
            // (`TA_TEST_WATCH`: kinds of the enemy's to follow, and their shots this minute.)
            let watch: Vec<String> = std::env::var("TA_TEST_WATCH").map(|v| v.split(',').map(|n| n.trim().to_ascii_uppercase()).filter(|n| !n.is_empty()).collect()).unwrap_or_default();
            let watched = |sim: &Sim, u: &ta_sim::Unit| !sim.allied(u.owner, 0) && watch.iter().any(|n| n.eq_ignore_ascii_case(&sim.unit_type(u).name));
            let mut watched_shots = 0u32;
            let points: Vec<(i64, i64)> = std::env::var("TA_PILOT_TO")
                .map(|v| v.split('|').filter_map(|p| p.split_once(',').and_then(|(x, z)| Some((x.trim().parse::<i64>().ok()?, z.trim().parse::<i64>().ok()?)))).collect())
                .unwrap_or_default();
            while tick < minutes * 60 * 30 && sim.mission.as_ref().is_some_and(|m| m.outcome.is_none()) {
                let mut commands = if piloted { pilot.think(&sim) } else { Vec::new() };
                // `TA_PILOT_TO=x,z`: everything the player has attack-moves there at the start
                // (an escort run, played straight).
                if tick == 1
                    && let Some(&(x, z)) = points.last()
                {
                    // (The Commander keeps behind the first of the escort.)
                    let commander = |u: &ta_sim::Unit| sim.unit_type(u).def.is_commander;
                    let units: Vec<ta_sim::UnitId> = sim.units().filter(|u| u.owner == 0 && !u.dead && !commander(u)).map(|u| u.id).collect();
                    let com: Vec<ta_sim::UnitId> = sim.units().filter(|u| u.owner == 0 && !u.dead && commander(u)).map(|u| u.id).collect();
                    if let Some(&lead) = units.first() {
                        commands.push(ta_sim::Command::Order { player: 0, units: com.clone(), order: ta_sim::Order::Guard(lead), queue: false });
                    }
                    for (k, &(px, pz)) in points.iter().enumerate() {
                        commands.push(ta_sim::Command::Order { player: 0, units: units.clone(), order: ta_sim::Order::Fight(FxVec3::from_ints(px, 0, pz)), queue: k > 0 });
                    }
                    commands.push(ta_sim::Command::Order { player: 0, units: com, order: ta_sim::Order::Move(FxVec3::from_ints(x, 0, z)), queue: true });
                    // `TA_PILOT_ATTACK=1`: aircraft go for the enemy Commander itself.
                    if std::env::var("TA_PILOT_ATTACK").is_ok()
                        && let Some(target) = sim.units().find(|u| u.owner != 0 && !u.dead && sim.unit_type(u).def.is_commander).map(|u| u.id)
                    {
                        let air: Vec<ta_sim::UnitId> = units.iter().copied().filter(|&id| sim.unit(id).is_some_and(|u| sim.unit_type(u).locomotion == ta_sim::Locomotion::Air)).collect();
                        commands.push(ta_sim::Command::Order { player: 0, units: air, order: ta_sim::Order::Attack(ta_sim::Target::Unit(target)), queue: false });
                    }
                }
                for ai in &mut ais {
                    commands.extend(ai.think(&sim));
                }
                sim.step(&commands);
                for e in sim.take_events() {
                    if !log {
                        continue;
                    }
                    // (Whose side: the player's and its allies', or the enemy's.)
                    let side = |u: &ta_sim::Unit| if sim.allied(u.owner, 0) { "ours" } else { "theirs" };
                    match e {
                        ta_sim::SimEvent::WeaponFired { unit, .. } => {
                            if let Some(u) = sim.unit(unit) {
                                *shots.entry((side(u), sim.unit_type(u).name.clone())).or_default() += 1;
                                watched_shots += watched(&sim, u) as u32;
                            }
                        }
                        ta_sim::SimEvent::UnitDestroyed { type_id, owner, killer, removed: false, .. } => {
                            let by = killer.and_then(|k| sim.unit(k)).map(|k| (side(k), sim.unit_type(k).name.clone()));
                            if let Some(by) = &by {
                                *kills.entry(by.clone()).or_default() += 1;
                            }
                            let name = &assets.types.get(type_id).name;
                            if !sim.allied(owner, 0) && watch.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                                println!("  their {name} fell at {}:{:02} to {by:?}", tick / 1800, tick / 30 % 60);
                            }
                            if owner == 0 && assets.types.get(type_id).def.is_commander {
                                fallen = Some((tick, by));
                            }
                        }
                        _ => {}
                    }
                }
                // (What last hurt the Commander: its killer may be gone by the time it falls.)
                if log
                    && let Some(c) = sim.units().find(|u| u.owner == 0 && !u.dead && sim.unit_type(u).def.is_commander)
                    && let Some(a) = c.last_attacker.and_then(|a| sim.unit(a))
                {
                    let side = if sim.allied(a.owner, 0) { "ours" } else { "theirs" };
                    hurt_by = Some((side, sim.unit_type(a).name.clone()));
                }
                tick += 1;
                // Armed units still on hold fire a minute in (TA's `NoAutoFire` guns nobody set
                // going, bombers): theirs and ours.
                if log && tick == 60 * 30 {
                    let mut held: std::collections::BTreeMap<(&str, String), usize> = Default::default();
                    for u in sim.units().filter(|u| !u.dead && u.fire_state == 0) {
                        let t = sim.unit_type(u);
                        if t.weapons.iter().flatten().any(|&w| !sim.weapon_types.get(w).def.interceptor) {
                            *held.entry((if sim.allied(u.owner, 0) { "ours" } else { "theirs" }, t.name.clone())).or_default() += 1;
                        }
                    }
                    println!("  on hold fire after a minute: {held:?}");
                }
                if tick % (60 * 30) == 0 {
                    let m = sim.mission.as_ref().unwrap();
                    let ours = sim.units().filter(|u| u.owner == 0 && !u.dead).count();
                    let theirs = sim.units().filter(|u| u.owner != 0 && !u.dead).count();
                    println!("  {} min {}: {ours} of ours, {theirs} of theirs, waves {}, holds {:?}, metal {:.0} / {:.0}", entry.scenario.id, tick / 1800, m.waves_sent, m.holds, sim.players[0].stats.metal_produced.to_f32(), sim.players.get(1).map_or(0.0, |p| p.stats.metal_produced.to_f32()));
                    if !watch.is_empty() {
                        println!("    {watch:?} fired {watched_shots} times this minute");
                        watched_shots = 0;
                    }
                }
            }
            // Where the enemy's units are, in a coarse grid (stuck waves show up).
            if std::env::var("TA_TEST_LOG").is_ok() {
                let mut cells: std::collections::BTreeMap<(i64, i64), (usize, usize)> = Default::default();
                for u in sim.units().filter(|u| u.owner != 0 && !u.dead) {
                    let c = cells.entry((u.pos.x.to_int() / 512, u.pos.z.to_int() / 512)).or_default();
                    c.0 += 1;
                    c.1 += u.orders.is_empty() as usize;
                }
                println!("  enemy units by 512-px cell (count, idle): {cells:?}");
                let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
                for u in sim.units().filter(|u| u.owner != 0 && !u.dead) {
                    *kinds.entry(sim.unit_type(u).name.clone()).or_default() += 1;
                }
                println!("  enemy units left: {kinds:?}");
                println!("  shots fired: {shots:?}");
                println!("  kills: {kills:?}");
                if let Some((at, by)) = &fallen {
                    println!("  the Commander fell at {}:{:02} to {by:?} (last hurt by {hurt_by:?})", at / 1800, at / 30 % 60);
                }
            }
            let outcome = sim.mission.as_ref().and_then(|m| m.outcome);
            println!("{} ({difficulty:?}): {} after {}:{:02}", entry.scenario.id, match outcome { Some(true) => "WON", Some(false) => "LOST", None => "undecided" }, tick / 1800, tick / 30 % 60);
        }
    }

    /// Iron Rain's guns shell the Arm base as its briefing says ("every eight seconds each of
    /// them will put a shell among your buildings"): they are their battery's, a force nobody
    /// plays, set going (`hold`) and on stores of their own (a shot costs 2,000 energy), and a
    /// radar post in the canyon spots the base for them. Left alone, each fires every eight
    /// seconds from the first minute; with the radar post gone, they see the base no more.
    #[test]
    fn iron_rains_guns_shell_the_base_as_briefed() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let entry = catalogue(&assets).into_iter().find(|e| e.scenario.id == "iron-rain").expect("Iron Rain");
        for difficulty in [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard] {
            let (mut sim, _, mut ais) = battle(&assets, &entry, difficulty);
            let gun = sim.types.id("CORINT").unwrap();
            let post = sim.units().find(|u| sim.unit_type(u).name.eq_ignore_ascii_case("CORARAD")).map(|u| u.id).expect("the radar post");
            // The guns' shots over so many seconds, nobody at the player's controls (`first`: the
            // commands of the first tick).
            let run = |sim: &mut Sim, ais: &mut Vec<ta_ai::SkirmishAi>, seconds: u64, first: Vec<ta_sim::Command>| -> usize {
                let mut shots = 0;
                let mut cmds = first;
                for _ in 0..seconds * 30 {
                    for ai in ais.iter_mut() {
                        cmds.extend(ai.think(sim));
                    }
                    sim.step(&std::mem::take(&mut cmds));
                    shots += sim.take_events().iter().filter(|e| matches!(e, ta_sim::SimEvent::WeaponFired { unit, .. } if sim.unit(*unit).is_some_and(|u| u.type_id == gun))).count();
                }
                shots
            };
            // (Their first shells on the way.)
            run(&mut sim, &mut ais, 20, Vec::new());
            let shelling = run(&mut sim, &mut ais, 60, Vec::new());
            // (Every eight seconds each: 14 to 16 a minute, as their reloads fall; one retarget's
            // turn of the turret allowed for.)
            assert!(shelling >= 12, "{difficulty:?}: the two guns fired {shelling} times in a minute, not every eight seconds each");
            sim.cheats_allowed = true;
            run(&mut sim, &mut ais, 20, vec![ta_sim::Command::Cheat { player: 0, cheat: ta_sim::Cheat::Destroy(vec![post]) }]);
            let blind = run(&mut sim, &mut ais, 60, Vec::new());
            println!("iron-rain ({difficulty:?}): the guns fired {shelling} times in a minute, {blind} in the next without the radar post");
            assert!(blind <= 2, "{difficulty:?}: with the radar post gone the guns fired {blind} times in a minute");
        }
    }

    /// The coach's advice to build is for a side that can build: Armageddon's army, with no
    /// Commander and no builder, was told two minutes in "No factory yet ... Your Commander can
    /// build a Kbot Lab"; a base with a Commander or builders still hears it.
    #[test]
    fn only_a_side_that_can_build_is_told_to_build() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        for (id, can) in [("armageddon", false), ("last-stand", true), ("decapitation", true)] {
            let entry = catalogue(&assets).into_iter().find(|e| e.scenario.id == id).unwrap();
            let (sim, _, _) = battle(&assets, &entry, Difficulty::Normal);
            assert_eq!(crate::coach::can_build(&sim, 0, |t| t.is_factory), can, "{id}: a factory");
            assert_eq!(crate::coach::can_build(&sim, 0, |_| true), can, "{id}: anything");
        }
    }

    /// A scenario's extras: allies on the player's side, reinforcements, mutators, the time of
    /// day, rain, and transmissions on the sides they name.
    #[test]
    fn scenario_extras_read() {
        let source = r#"
id = "extras"
title = "Extras"
tagline = "A line."
briefing = "Words."
map = "Mossvale"
kind = "battle"
time = [[0, 21.0], [600, 30.5]]
rain = true
victory = [{ destroy = "commander" }]

[mutators]
no_rush_minutes = 3
no_air = true

[player]
side = "ARM"
start = "start 1"

[[enemy]]
side = "CORE"
start = "start 2"
ai = "chosen"

[[ally]]
side = "ARM"
start = "start 1 + 300 0"
ai = "easy"

[[wave]]
at = 60
player = true
from = "start 1"
units = [{ unit = "ARMPW", count = 2 }]

[[message]]
at = 5
side = "ally 1"
from = "An ally"
text = "Hello."
"#;
        let s = Scenario::parse(source).unwrap();
        assert_eq!(s.forces().count(), 3);
        assert_eq!(s.ally_player(0), 2);
        assert!(!s.passive(1) && !s.passive(2));
        assert_eq!(s.side_player(None).unwrap(), 0);
        assert_eq!(s.side_player(Some("enemy 1")).unwrap(), 1);
        assert_eq!(s.side_player(Some("Ally 1")).unwrap(), 2);
        assert!(s.side_player(Some("ally 2")).is_err() && s.side_player(Some("enemy")).is_err());
        let m = s.mutators.unwrap();
        assert!(m.no_air && m.no_rush_minutes == 3 && m.resources == 4);
        assert!((s.time.as_ref().unwrap().at(300.0) - 25.75).abs() < 1e-3, "eased halfway round the clock");
        assert_eq!(s.rain, Some(true));
        assert!(Scenario::parse(&source.replace("side = \"ally 1\"", "side = \"ally 3\"")).is_err(), "an ally it hasn't");
        assert!(Scenario::parse(&source.replace("player = true", "player = true
ally = 1")).is_err(), "a wave of two sides");
        // An enemy's wave needs that enemy (its units would be nobody's).
        assert!(Scenario::parse(&source.replace("player = true", "enemy = 2")).is_err(), "an enemy it hasn't");
        let no_enemy = source.replace("[[enemy]]\nside = \"CORE\"\nstart = \"start 2\"\nai = \"chosen\"\n", "");
        assert_ne!(no_enemy, source);
        assert!(Scenario::parse(&no_enemy.replace("player = true", "enemy = 1")).is_err(), "an enemy wave without enemies");
    }

    #[test]
    fn the_built_in_scenarios_read() {
        let mut ids = std::collections::HashSet::new();
        for source in BUILT_IN {
            let s = Scenario::parse(source).unwrap_or_else(|e| panic!("{e:#}\n{}", &source[..source.len().min(200)]));
            assert!(ids.insert(s.id.clone()), "{} twice", s.id);
            assert!(!s.title.is_empty() && !s.tagline.is_empty() && !s.briefing.is_empty(), "{}", s.id);
            assert!(!s.enemies.is_empty(), "{}: nobody to fight", s.id);
        }
    }
}
