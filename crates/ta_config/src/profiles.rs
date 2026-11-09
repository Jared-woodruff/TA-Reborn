//! Local player identities and their persistent careers.
//!
//! Device-wide content stays in `config_dir`; personal settings, campaigns, saves and
//! replays live below one validated profile directory. A running game pins its profile
//! once, so changing the launcher's selection cannot redirect a match's progress.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock, atomic::{AtomicU64, Ordering}};
use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use crate::Config;

const VERSION: u32 = 1;
// Older executables reject the richer ledger instead of rewriting away career detail.
// (3: event tallies, battle peaks, maps and worlds won on, unlock details; the redone list.
// 4: matches voided for cheats, and what each match gave the career that totals cannot say;
// 5: each match keeps its XP rate, preserving historical awards when new rates are balanced.)
const CAREER_VERSION: u32 = 5;
const MAX_TALLIES: usize = 64;
const MAX_FEATS: usize = 128;
const ACHIEVEMENT_PICTURES: &str = "achievements";
const MAX_ACHIEVEMENT_PICTURE: usize = 4 * 1024 * 1024;
const MAX_PROFILES: usize = 64;
const MAX_NAME_CHARS: usize = 24;
const MAX_AVATAR_INPUT: usize = 16 * 1024 * 1024;
const MAX_PROFILE_FILE: u64 = 16 * 1024;
const MAX_CAREER_FILE: u64 = 32 * 1024 * 1024;
const MAX_MATCHES: usize = 100_000;
const MAX_COUNTER: u64 = 1_000_000_000_000;
const MAX_TOTAL: u64 = 1_000_000_000_000_000;
/// A corrupted mod's counters cannot grant an unbounded career in a single match.
pub const MAX_MATCH_XP: u64 = 1_000_000;
pub const MAX_LEVEL: u32 = 100;
const PROFILE_FILE: &str = "profile.toml";
const CAREER_FILE: &str = "career.toml";

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Allegiance { #[default] Arm, Core }

impl Allegiance {
    pub const fn label(self) -> &'static str {
        match self { Self::Arm => "ARM", Self::Core => "CORE" }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub allegiance: Allegiance,
    pub has_avatar: bool,
    pub created_at: u64,
    /// A line of the player's own under their name (since 2.2).
    #[serde(default)]
    pub motto: String,
    /// Achievements the player shows on their profile, at most [`MAX_SHOWCASE`].
    #[serde(default)]
    pub showcase: Vec<String>,
}

/// A motto's most characters, and the most achievements a profile shows.
pub const MAX_MOTTO_CHARS: usize = 60;
pub const MAX_SHOWCASE: usize = 5;

/// A recorded activity's actual mode. Unknown preserves pre-career history without
/// guessing that a legacy battle was a skirmish or multiplayer game.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode { #[default] Unknown, Campaign, Skirmish, Multiplayer, Scenario, Conquest, Tutorial }

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CampaignMission { pub campaign: String, pub index: u32 }

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MatchContext {
    pub mode: MatchMode,
    pub map: String,
    pub faction: String,
    pub mission: Option<CampaignMission>,
    pub difficulty: String,
    /// True only for a live network setup with an opposing human playing slot.
    pub human_opponent: bool,
    /// The map's world (TA's `planet`: "Lush", "Metal"...).
    pub world: String,
    /// Players against ours, and on our side besides us (AIs and people alike).
    pub enemies: u32,
    pub allies: u32,
    /// A live network game with another person on our team.
    pub human_ally: bool,
}

impl MatchContext {
    fn bounded(&self) -> Self {
        Self { mode: self.mode, map: display_text(&self.map), faction: display_text(&self.faction).to_ascii_uppercase(),
            mission: self.mission.as_ref().filter(|m| m.index < 100_000)
                .map(|m| CampaignMission { campaign: display_text(&m.campaign).to_ascii_lowercase(), index: m.index })
                .filter(|m| !m.campaign.is_empty()),
            difficulty: display_text(&self.difficulty).to_ascii_lowercase(), human_opponent: self.human_opponent && self.mode == MatchMode::Multiplayer,
            world: display_text(&self.world), enemies: self.enemies.min(64), allies: self.allies.min(64),
            human_ally: self.human_ally && self.mode == MatchMode::Multiplayer }
    }
}

/// Recent history is observational: an imported save contains only actions after its
/// baseline, never the other player's historic kills or playing time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchRecord {
    pub id: String,
    pub context: MatchContext,
    pub counters: MatchCounters,
    pub earned_xp: u64,
    pub started_at: u64,
    pub updated_at: u64,
}

fn display_text(value: &str) -> String { value.chars().filter(|c| !c.is_control()).take(128).collect() }
fn bounded_counts(values: &BTreeMap<String, u64>) -> BTreeMap<String, u64> {
    values.iter().filter(|(k, _)| !k.is_empty() && **k == display_text(k)).take(1024)
        .map(|(k, v)| (k.clone(), (*v).min(MAX_COUNTER))).collect()
}
fn highest_counts(a: &BTreeMap<String, u64>, b: &BTreeMap<String, u64>) -> BTreeMap<String, u64> {
    let mut out = a.clone();
    for (k, v) in b { if out.contains_key(k) || out.len() < 1024 { let e = out.entry(k.clone()).or_default(); *e = (*e).max(*v); } }
    out
}
/// A tally's or a peak's name: `kills.air`, `metal_income`.
fn tally_name(key: &str) -> bool {
    !key.is_empty() && key.len() <= 48 && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_')
}
fn bounded_tallies(values: &BTreeMap<String, u64>) -> BTreeMap<String, u64> {
    values.iter().filter(|(k, v)| tally_name(k) && **v > 0).take(MAX_TALLIES).map(|(k, v)| (k.clone(), (*v).min(MAX_COUNTER))).collect()
}
fn valid_tallies(values: &BTreeMap<String, u64>) -> bool {
    values.len() <= MAX_TALLIES && values.iter().all(|(k, v)| tally_name(k) && *v <= MAX_TOTAL)
}
fn add_tallies(total: &mut BTreeMap<String, u64>, delta: &BTreeMap<String, u64>) {
    for (k, v) in delta {
        if *v > 0 && (total.contains_key(k) || total.len() < MAX_TALLIES) { plus(total.entry(k.clone()).or_default(), *v); }
    }
}

/// Persisted scoring rules: future tuning adds a variant instead of redefining an old rate.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum XpRate {
    Original,
    #[default]
    Balanced,
}

fn original_xp_rate() -> XpRate { XpRate::Original }

impl XpRate {
    fn score(self, counters: &MatchCounters) -> u64 { counters.xp_breakdown_at(self).total() }
}

/// Cumulative counters for one real match, reconstructed along with a saved game.
/// `units_built` includes buildings; `buildings_built` gives those an additional bonus.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MatchCounters {
    pub played_seconds: u64,
    pub kills: u64,
    pub losses: u64,
    pub units_built: u64,
    pub buildings_built: u64,
    pub units_captured: u64,
    pub metal_gathered: u64,
    pub energy_gathered: u64,
    pub unit_builds: BTreeMap<String, u64>,
    pub unit_names: BTreeMap<String, String>,
    pub finished: bool,
    pub won: bool,
    /// Events of the battle by name (`kills.air`, `launched.missile`, `reclaimed`...): what
    /// achievements are made of. They never score XP.
    pub tallies: BTreeMap<String, u64>,
    /// The battle's highest moments (`metal_income` a second, `army` worth in metal): records
    /// of a single battle, not totals.
    pub peaks: BTreeMap<String, u64>,
}

impl MatchCounters {
    fn bounded(&self) -> Self {
        let unit_builds = bounded_counts(&self.unit_builds);
        let unit_names = self.unit_names.iter().filter(|(k, _)| unit_builds.contains_key(*k))
            .map(|(k, v)| (k.clone(), display_text(v))).collect();
        Self {
            played_seconds: self.played_seconds.min(MAX_COUNTER),
            kills: self.kills.min(MAX_COUNTER),
            losses: self.losses.min(MAX_COUNTER),
            units_built: self.units_built.min(MAX_COUNTER),
            buildings_built: self.buildings_built.min(self.units_built).min(MAX_COUNTER),
            units_captured: self.units_captured.min(MAX_COUNTER),
            metal_gathered: self.metal_gathered.min(MAX_COUNTER),
            energy_gathered: self.energy_gathered.min(MAX_COUNTER),
            unit_builds,
            unit_names,
            finished: self.finished,
            won: self.won && self.finished,
            tallies: bounded_tallies(&self.tallies),
            peaks: bounded_tallies(&self.peaks),
        }
    }

    fn highest(&self, other: &Self) -> Self {
        let other = other.bounded();
        Self {
            played_seconds: self.played_seconds.max(other.played_seconds),
            kills: self.kills.max(other.kills),
            losses: self.losses.max(other.losses),
            units_built: self.units_built.max(other.units_built),
            buildings_built: self.buildings_built.max(other.buildings_built),
            units_captured: self.units_captured.max(other.units_captured),
            metal_gathered: self.metal_gathered.max(other.metal_gathered),
            energy_gathered: self.energy_gathered.max(other.energy_gathered),
            unit_builds: highest_counts(&self.unit_builds, &other.unit_builds),
            unit_names: { let mut names = self.unit_names.clone(); for (k, v) in &other.unit_names { if names.contains_key(k) || names.len() < 1024 { names.insert(k.clone(), v.clone()); } } names },
            finished: self.finished || other.finished,
            won: self.won || other.won,
            tallies: highest_counts(&self.tallies, &other.tallies),
            peaks: highest_counts(&self.peaks, &other.peaks),
        }.bounded()
    }

    fn since(&self, baseline: &Self) -> Self {
        Self {
            played_seconds: self.played_seconds.saturating_sub(baseline.played_seconds),
            kills: self.kills.saturating_sub(baseline.kills), losses: self.losses.saturating_sub(baseline.losses),
            units_built: self.units_built.saturating_sub(baseline.units_built),
            buildings_built: self.buildings_built.saturating_sub(baseline.buildings_built),
            units_captured: self.units_captured.saturating_sub(baseline.units_captured),
            metal_gathered: self.metal_gathered.saturating_sub(baseline.metal_gathered),
            energy_gathered: self.energy_gathered.saturating_sub(baseline.energy_gathered),
            unit_builds: self.unit_builds.iter().filter_map(|(k, v)| {
                let delta = v.saturating_sub(baseline.unit_builds.get(k).copied().unwrap_or(0));
                (delta > 0).then(|| (k.clone(), delta))
            }).collect(), unit_names: self.unit_names.clone(),
            finished: self.finished && !baseline.finished, won: self.won && !baseline.finished,
            tallies: self.tallies.iter().filter_map(|(k, v)| {
                let delta = v.saturating_sub(baseline.tallies.get(k).copied().unwrap_or(0));
                (delta > 0).then(|| (k.clone(), delta))
            }).collect(),
            // (A peak is a record, not an amount: those reached since the baseline stand.)
            peaks: self.peaks.iter().filter(|(k, v)| **v > baseline.peaks.get(*k).copied().unwrap_or(0))
                .map(|(k, v)| (k.clone(), *v)).collect(),
        }
    }

    pub fn tally(&self, key: &str) -> u64 { self.tallies.get(key).copied().unwrap_or(0) }
    pub fn peak(&self, key: &str) -> u64 { self.peaks.get(key).copied().unwrap_or(0) }

    /// Score cumulative totals so frequent checkpoints preserve fractional progress.
    pub fn xp(&self) -> u64 { self.xp_breakdown().total() }

    /// [`MatchCounters::xp`] by source.
    pub fn xp_breakdown(&self) -> XpBreakdown {
        self.xp_breakdown_at(XpRate::Balanced)
    }

    fn xp_breakdown_at(&self, rate: XpRate) -> XpBreakdown {
        // Scoring is also used while validating every historical match. Unit labels
        // and per-type counts do not affect XP; do not clone/normalize those maps here.
        let built = self.units_built.min(MAX_COUNTER);
        let mut xp = XpBreakdown {
            time: self.played_seconds.min(MAX_COUNTER) / 30 * 5,
            combat: self.kills.min(MAX_COUNTER).saturating_mul(20),
            construction: built.saturating_mul(10).saturating_add(self.buildings_built.min(built).saturating_mul(15)),
            captures: self.units_captured.min(MAX_COUNTER).saturating_mul(30),
            economy: (self.metal_gathered.min(MAX_COUNTER) / 100).saturating_add(self.energy_gathered.min(MAX_COUNTER) / 1000),
            completion: if self.finished { 100 } else { 0 },
            victory: if self.won && self.finished { 300 } else { 0 },
        };
        if rate == XpRate::Balanced {
            // A moderate 25% reduction. Scale cumulative sources, rather than checkpoint
            // deltas, so frequent saves retain fractional progress and cannot earn more XP.
            for source in [&mut xp.time, &mut xp.combat, &mut xp.construction, &mut xp.captures, &mut xp.economy, &mut xp.completion, &mut xp.victory] {
                *source = source.saturating_mul(3) / 4;
            }
        }
        xp
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Career {
    pub xp: u64,
    pub matches_finished: u64,
    pub wins: u64,
    pub kills: u64,
    pub losses: u64,
    pub units_built: u64,
    pub buildings_built: u64,
    pub units_captured: u64,
    pub played_seconds: u64,
    pub metal_gathered: u64,
    pub energy_gathered: u64,
    pub modes: BTreeMap<MatchMode, ModeStats>,
    /// The competitive subset used by PvP achievements; co-op remains visible in modes.
    pub competitive_multiplayer: ModeStats,
    /// Seconds of eligible play, rather than the faction chosen on the profile avatar.
    pub faction_usage: BTreeMap<String, u64>,
    /// Completed constructions by stable unit identifier (includes buildings).
    pub unit_usage: BTreeMap<String, u64>,
    pub unit_names: BTreeMap<String, String>,
    pub campaign_completed: BTreeMap<String, Vec<u32>>,
    pub hard_completed: BTreeMap<String, Vec<u32>>,
    pub campaign_arm_completed: BTreeMap<String, Vec<u32>>,
    pub campaign_core_completed: BTreeMap<String, Vec<u32>>,
    /// Stable achievement identifiers and the first time each was unlocked.
    pub achievements: BTreeMap<String, u64>,
    /// Where and when in a battle each achievement was earned (since version 3).
    pub unlocks: BTreeMap<String, Unlock>,
    /// Best verified single-battle feats, only when tracked from the beginning.
    pub feats: BTreeMap<String, u64>,
    /// Event tallies of every eligible battle (`MatchCounters::tallies`).
    pub tallies: BTreeMap<String, u64>,
    /// Skirmish and multiplayer wins by map title and by world.
    pub map_wins: BTreeMap<String, u64>,
    pub world_wins: BTreeMap<String, u64>,
}

/// The circumstances of an achievement's unlocking.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Unlock {
    /// The battle it was earned in: its map, mode and how far into it (seconds).
    pub map: String,
    pub mode: MatchMode,
    pub battle_seconds: u64,
    /// Met by play recorded before it was in the list (or before this version): earned from
    /// the service record rather than at a moment of a battle.
    pub earlier: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ModeStats {
    pub matches_finished: u64, pub wins: u64, pub kills: u64, pub losses: u64,
    pub units_built: u64, pub buildings_built: u64, pub units_captured: u64,
    pub played_seconds: u64, pub metal_gathered: u64, pub energy_gathered: u64,
    pub hard_wins: u64, pub arm_wins: u64, pub core_wins: u64,
    pub tallies: BTreeMap<String, u64>,
}

impl ModeStats {
    fn add(&mut self, delta: &MatchCounters, context: &MatchContext) {
        plus(&mut self.matches_finished, u64::from(delta.finished)); plus(&mut self.wins, u64::from(delta.won));
        plus(&mut self.kills, delta.kills); plus(&mut self.losses, delta.losses);
        plus(&mut self.units_built, delta.units_built); plus(&mut self.buildings_built, delta.buildings_built);
        plus(&mut self.units_captured, delta.units_captured); plus(&mut self.played_seconds, delta.played_seconds);
        plus(&mut self.metal_gathered, delta.metal_gathered); plus(&mut self.energy_gathered, delta.energy_gathered);
        add_tallies(&mut self.tallies, &delta.tallies);
        if delta.won {
            plus(&mut self.hard_wins, u64::from(context.difficulty == "hard"));
            plus(&mut self.arm_wins, u64::from(context.faction == "ARM"));
            plus(&mut self.core_wins, u64::from(context.faction == "CORE"));
        }
    }
    /// [`ModeStats::add`] undone: a voided match's share taken back.
    fn take(&mut self, delta: &MatchCounters, context: &MatchContext) {
        minus(&mut self.matches_finished, u64::from(delta.finished)); minus(&mut self.wins, u64::from(delta.won));
        minus(&mut self.kills, delta.kills); minus(&mut self.losses, delta.losses);
        minus(&mut self.units_built, delta.units_built); minus(&mut self.buildings_built, delta.buildings_built);
        minus(&mut self.units_captured, delta.units_captured); minus(&mut self.played_seconds, delta.played_seconds);
        minus(&mut self.metal_gathered, delta.metal_gathered); minus(&mut self.energy_gathered, delta.energy_gathered);
        take_tallies(&mut self.tallies, &delta.tallies);
        if delta.won {
            minus(&mut self.hard_wins, u64::from(context.difficulty == "hard"));
            minus(&mut self.arm_wins, u64::from(context.faction == "ARM"));
            minus(&mut self.core_wins, u64::from(context.faction == "CORE"));
        }
    }
    fn valid(&self) -> bool {
        [self.matches_finished, self.wins, self.kills, self.losses, self.units_built, self.buildings_built,
            self.units_captured, self.played_seconds, self.metal_gathered, self.energy_gathered,
            self.hard_wins, self.arm_wins, self.core_wins].into_iter().all(|v| v <= MAX_TOTAL)
            && self.wins <= self.matches_finished && self.hard_wins <= self.wins
            && self.arm_wins <= self.wins && self.core_wins <= self.wins
            && valid_tallies(&self.tallies)
    }
}
fn plus(total: &mut u64, delta: u64) { *total = total.saturating_add(delta).min(MAX_TOTAL); }
fn minus(total: &mut u64, delta: u64) { *total = total.saturating_sub(delta); }
/// Takes `delta` from a count by name, forgetting the name at nothing.
fn minus_key(counts: &mut BTreeMap<String, u64>, key: &str, delta: u64) {
    if let Some(v) = counts.get_mut(key) {
        *v = v.saturating_sub(delta);
        if *v == 0 { counts.remove(key); }
    }
}
fn take_tallies(total: &mut BTreeMap<String, u64>, delta: &BTreeMap<String, u64>) {
    for (k, v) in delta { minus_key(total, k, *v); }
}
/// The campaign lists by name, for a match's record of the missions it completed.
const MISSION_LISTS: [&str; 4] = ["campaign", "hard", "arm", "core"];
fn note_mission(list: &mut BTreeMap<String, Vec<u32>>, mission: &CampaignMission) {
    if list.len() >= 1024 && !list.contains_key(&mission.campaign) { return; }
    let indexes = list.entry(mission.campaign.clone()).or_default();
    if let Err(index) = indexes.binary_search(&mission.index) { if indexes.len() < 100_000 { indexes.insert(index, mission.index); } }
}

impl Career {
    /// The rank of all the XP earned: battles' and achievements'.
    pub fn rank(&self) -> Rank { rank(self.total_xp()) }

    /// Battles' XP ([`Career::xp`]) and the achievements' rewards together.
    pub fn total_xp(&self) -> u64 { self.xp.saturating_add(self.achievement_xp()).min(MAX_TOTAL) }

    /// The XP the achievements earned grant: their tiers' rewards ([`crate::achievements::Tier::reward`]).
    pub fn achievement_xp(&self) -> u64 {
        self.achievements.keys().filter_map(|id| crate::achievements::find(id)).map(|a| a.tier.reward()).sum()
    }

    fn add(&mut self, old: &MatchCounters, new: &MatchCounters, xp: u64) {
        plus(&mut self.xp, xp);
        plus(&mut self.matches_finished, u64::from(new.finished && !old.finished));
        plus(&mut self.wins, u64::from(new.won && !old.won));
        plus(&mut self.kills, new.kills - old.kills);
        plus(&mut self.losses, new.losses - old.losses);
        plus(&mut self.units_built, new.units_built - old.units_built);
        plus(&mut self.buildings_built, new.buildings_built - old.buildings_built);
        plus(&mut self.units_captured, new.units_captured - old.units_captured);
        plus(&mut self.played_seconds, new.played_seconds - old.played_seconds);
        plus(&mut self.metal_gathered, new.metal_gathered - old.metal_gathered);
        plus(&mut self.energy_gathered, new.energy_gathered - old.energy_gathered);
    }

    /// [`Career::add`] undone: a voided match's totals and XP taken back.
    fn take(&mut self, old: &MatchCounters, new: &MatchCounters, xp: u64) {
        minus(&mut self.xp, xp);
        minus(&mut self.matches_finished, u64::from(new.finished && !old.finished));
        minus(&mut self.wins, u64::from(new.won && !old.won));
        minus(&mut self.kills, new.kills.saturating_sub(old.kills));
        minus(&mut self.losses, new.losses.saturating_sub(old.losses));
        minus(&mut self.units_built, new.units_built.saturating_sub(old.units_built));
        minus(&mut self.buildings_built, new.buildings_built.saturating_sub(old.buildings_built));
        minus(&mut self.units_captured, new.units_captured.saturating_sub(old.units_captured));
        minus(&mut self.played_seconds, new.played_seconds.saturating_sub(old.played_seconds));
        minus(&mut self.metal_gathered, new.metal_gathered.saturating_sub(old.metal_gathered));
        minus(&mut self.energy_gathered, new.energy_gathered.saturating_sub(old.energy_gathered));
    }

    /// [`Career::add_details`] undone for a voided match: its share of the modes, factions,
    /// units, tallies and wins by map taken back; the single-battle records it set (those
    /// nothing has beaten since) put back to the best of what they were before it and of
    /// `others` (the records of the battles left, recounted); the missions it completed first
    /// struck off.
    fn take_details(&mut self, old: &MatchCounters, new: &MatchCounters, context: &MatchContext, gave: &Gave, others: &BTreeMap<&'static str, u64>) {
        let delta = new.since(old);
        if let Some(stats) = self.modes.get_mut(&context.mode) { stats.take(&delta, context); }
        if context.mode == MatchMode::Multiplayer && context.human_opponent {
            self.competitive_multiplayer.take(&delta, context);
        }
        minus_key(&mut self.faction_usage, &context.faction, delta.played_seconds);
        for (id, count) in &delta.unit_builds {
            minus_key(&mut self.unit_usage, id, *count);
            if !self.unit_usage.contains_key(id) { self.unit_names.remove(id); }
        }
        take_tallies(&mut self.tallies, &delta.tallies);
        if delta.won && context.mode == MatchMode::Multiplayer && context.human_ally {
            minus_key(&mut self.tallies, "wins.coop", 1);
        }
        if delta.won && matches!(context.mode, MatchMode::Skirmish | MatchMode::Multiplayer) {
            minus_key(&mut self.map_wins, &context.map, 1);
            minus_key(&mut self.world_wins, &context.world, 1);
        }
        for (key, &(before, set)) in &gave.feats {
            if self.feats.get(key) == Some(&set) {
                let best = before.max(others.get(key.as_str()).copied().unwrap_or(0));
                if best == 0 { self.feats.remove(key); } else { self.feats.insert(key.clone(), best); }
            }
        }
        for (list, campaign, index) in &gave.missions {
            let list = match list.as_str() {
                "campaign" => &mut self.campaign_completed,
                "hard" => &mut self.hard_completed,
                "arm" => &mut self.campaign_arm_completed,
                "core" => &mut self.campaign_core_completed,
                _ => continue,
            };
            if let Some(indexes) = list.get_mut(campaign) {
                indexes.retain(|i| i != index);
                if indexes.is_empty() { list.remove(campaign); }
            }
        }
    }

    /// The campaign lists, by [`MISSION_LISTS`]' names.
    fn mission_lists(&self) -> [&BTreeMap<String, Vec<u32>>; 4] {
        [&self.campaign_completed, &self.hard_completed, &self.campaign_arm_completed, &self.campaign_core_completed]
    }

    pub fn favourite_faction(&self) -> Option<(&str, u64)> { favourite(&self.faction_usage) }
    pub fn favourite_unit(&self) -> Option<(&str, u64)> {
        favourite(&self.unit_usage).map(|(id, count)| (self.unit_names.get(id).map_or(id, String::as_str), count))
    }
    fn add_details(&mut self, old: &MatchCounters, new: &MatchCounters, context: &MatchContext, full_match: bool) {
        let delta = new.since(old);
        self.modes.entry(context.mode).or_default().add(&delta, context);
        if context.mode == MatchMode::Multiplayer && context.human_opponent {
            self.competitive_multiplayer.add(&delta, context);
        }
        if !context.faction.is_empty() && delta.played_seconds > 0 && (self.faction_usage.contains_key(&context.faction) || self.faction_usage.len() < 1024) {
            plus(self.faction_usage.entry(context.faction.clone()).or_default(), delta.played_seconds);
        }
        for (id, count) in &delta.unit_builds {
            if self.unit_usage.contains_key(id) || self.unit_usage.len() < 4096 {
                plus(self.unit_usage.entry(id.clone()).or_default(), *count);
                if let Some(name) = new.unit_names.get(id) { self.unit_names.insert(id.clone(), name.clone()); }
            }
        }
        add_tallies(&mut self.tallies, &delta.tallies);
        if delta.won && context.mode == MatchMode::Multiplayer && context.human_ally {
            add_tallies(&mut self.tallies, &[("wins.coop".to_string(), 1)].into());
        }
        if delta.won && matches!(context.mode, MatchMode::Skirmish | MatchMode::Multiplayer) {
            let win = |list: &mut BTreeMap<String, u64>, name: &str| {
                if !name.is_empty() && (list.contains_key(name) || list.len() < 1024) { plus(list.entry(name.to_string()).or_default(), 1); }
            };
            win(&mut self.map_wins, &context.map);
            win(&mut self.world_wins, &context.world);
        }
        // A battle's records count as they are reached, its outcome's when it is decided.
        if full_match && context.mode != MatchMode::Tutorial {
            for (key, value) in battle_feats(new, context, delta.finished) {
                if value > 0 && (self.feats.contains_key(key) || self.feats.len() < MAX_FEATS) {
                    let v = self.feats.entry(key.to_string()).or_default();
                    *v = (*v).max(value.min(MAX_TOTAL));
                }
            }
        }
        if context.mode == MatchMode::Campaign && delta.won && let Some(mission) = &context.mission {
            note_mission(&mut self.campaign_completed, mission);
            if context.difficulty == "hard" { note_mission(&mut self.hard_completed, mission); }
            if context.faction == "ARM" { note_mission(&mut self.campaign_arm_completed, mission); }
            if context.faction == "CORE" { note_mission(&mut self.campaign_core_completed, mission); }
        }
    }

    /// The career a checkpoint would record once this match's progress moves on from `old`
    /// (what the ledger holds of it) to `new`. The game asks between its checkpoints, so that
    /// it saves, and takes its picture, the moment something is earned. `full_match`: the
    /// match has been tracked from its beginning (its single-battle records count).
    pub fn with_progress(&self, old: &MatchCounters, new: &MatchCounters, context: &MatchContext, full_match: bool) -> Career {
        let old = old.bounded();
        let new = if old.finished { old.clone() } else { old.highest(new) };
        let mut next = self.clone();
        if new != old {
            next.add(&old, &new, 0);
            next.add_details(&old, &new, &context.bounded(), full_match);
        }
        next
    }

    /// Achievements this career has met but not yet recorded, in catalogue order.
    pub fn pending(&self) -> Vec<&'static crate::achievements::Achievement> {
        crate::achievements::ALL.iter().filter(|a| a.is_met(self)).collect()
    }

    pub fn tally(&self, key: &str) -> u64 { self.tallies.get(key).copied().unwrap_or(0) }

    /// Records the achievements met: their unlocking, now, in `battle` (map, mode and seconds),
    /// or as `earlier` play's when `before` (the career as it was) had met them already.
    fn unlock_met(&mut self, before: Option<&Career>, battle: Option<(&MatchContext, u64)>) -> (Vec<String>, Vec<String>) {
        let (mut unlocked, mut earlier) = (Vec::new(), Vec::new());
        let now = unix_time();
        for achievement in self.pending() {
            let old = before.is_none_or(|before| achievement.progress(before) >= achievement.target);
            let (map, mode, battle_seconds) = battle.map_or((String::new(), MatchMode::Unknown, 0), |(c, s)| (c.map.clone(), c.mode, s));
            self.achievements.insert(achievement.id.to_string(), now);
            self.unlocks.insert(achievement.id.to_string(), Unlock { map, mode, battle_seconds, earlier: old });
            unlocked.push(achievement.id.to_string());
            if old { earlier.push(achievement.id.to_string()); }
        }
        (unlocked, earlier)
    }

    /// [`Career::unlock_met`] until nothing more is met: an achievement's XP can raise the rank
    /// to another's.
    fn unlock_all_met(&mut self, before: Option<&Career>, battle: Option<(&MatchContext, u64)>) -> (Vec<String>, Vec<String>) {
        let (mut unlocked, mut earlier) = (Vec::new(), Vec::new());
        loop {
            let (more, old) = self.unlock_met(before, battle);
            if more.is_empty() { break; }
            unlocked.extend(more);
            earlier.extend(old);
        }
        let order = |id: &String| crate::achievements::find(id).map_or(usize::MAX, |a| a.index());
        unlocked.sort_by_key(order);
        earlier.sort_by_key(order);
        (unlocked, earlier)
    }

    fn valid(&self) -> bool {
        let counts = |values: &BTreeMap<String, u64>, max: usize| values.len() <= max && values.iter().all(|(k, v)| !k.is_empty() && *k == display_text(k) && *v <= MAX_TOTAL);
        let missions = |values: &BTreeMap<String, Vec<u32>>| values.len() <= 1024 && values.iter().all(|(k, v)| !k.is_empty() && *k == display_text(k) && v.len() <= 100_000 && v.iter().all(|n| *n < 100_000) && v.windows(2).all(|w| w[0] < w[1]));
        let known = |id: &str| crate::achievements::find(id).is_some();
        [self.xp, self.matches_finished, self.wins, self.kills, self.losses, self.units_built,
            self.buildings_built, self.units_captured, self.played_seconds,
            self.metal_gathered, self.energy_gathered].into_iter().all(|v| v <= MAX_TOTAL)
            && self.wins <= self.matches_finished && self.modes.values().all(ModeStats::valid) && self.competitive_multiplayer.valid()
            && counts(&self.feats, MAX_FEATS) && counts(&self.faction_usage, 1024) && counts(&self.unit_usage, 4096)
            && self.unit_names.len() <= 4096 && self.unit_names.iter().all(|(k, v)| self.unit_usage.contains_key(k) && *v == display_text(v))
            && missions(&self.campaign_completed) && missions(&self.hard_completed)
            && missions(&self.campaign_arm_completed) && missions(&self.campaign_core_completed)
            && valid_tallies(&self.tallies) && counts(&self.map_wins, 1024) && counts(&self.world_wins, 1024)
            && self.achievements.len() <= crate::achievements::ALL.len()
            && self.achievements.iter().all(|(id, date)| known(id) && *date <= i64::MAX as u64)
            && self.unlocks.iter().all(|(id, u)| self.achievements.contains_key(id) && u.map == display_text(&u.map) && u.battle_seconds <= MAX_COUNTER)
    }

    /// A ledger from before the list was redone: its retired achievements go (the rest kept
    /// their identifiers). Unknown ones still make it invalid.
    fn retire_old_achievements(&mut self) {
        self.achievements.retain(|id, _| !crate::achievements::RETIRED.contains(&id.as_str()));
    }
}
/// The single-battle records a battle tracked from its beginning holds at `counters` (played in
/// `context`), its outcome's among them once it is `decided`: what [`Career::add_details`]
/// keeps the best of, and what voiding a battle recounts from those left.
fn battle_feats(counters: &MatchCounters, context: &MatchContext, decided: bool) -> Vec<(&'static str, u64)> {
    let mut out = vec![
        ("battle_kills", counters.kills), ("battle_built", counters.units_built), ("battle_captures", counters.units_captured),
        ("battle_commander_kills", counters.tally("commander.kills")),
        ("battle_metal_income", counters.peak("metal_income")),
        ("battle_energy_income", counters.peak("energy_income")),
        ("battle_army", counters.peak("army")),
    ];
    if decided && counters.won {
        let skirmish = context.mode == MatchMode::Skirmish;
        let alone = skirmish && context.allies == 0;
        let hard = context.difficulty == "hard";
        out.extend([
            ("flawless_victory", u64::from(counters.losses == 0 && counters.played_seconds >= 300)),
            ("fast_skirmish", u64::from(skirmish && (60..=600).contains(&counters.played_seconds))),
            ("fast_hard", u64::from(skirmish && hard && (60..=900).contains(&counters.played_seconds))),
            ("endurance_victory", u64::from(counters.played_seconds >= 3600)),
            ("outnumbered", u64::from(alone && context.enemies >= 3)),
            ("hard_odds", u64::from(alone && hard && context.enemies >= 3)),
            ("ffa_seven", u64::from(alone && context.enemies >= 7)),
        ]);
    }
    out
}

fn favourite(values: &BTreeMap<String, u64>) -> Option<(&str, u64)> {
    values.iter().filter(|(_, n)| **n > 0).max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0))).map(|(k, v)| (k.as_str(), *v))
}

/// The ranks and the level each begins at: a title every five levels, the last one at the top
/// level. A rank's place here is its grade (its insignia). UI callers translate the titles.
pub const RANKS: [(u32, &str); 21] = [
    (1, "Recruit"), (5, "Private"), (10, "Private First Class"), (15, "Lance Corporal"), (20, "Corporal"),
    (25, "Sergeant"), (30, "Staff Sergeant"), (35, "Master Sergeant"), (40, "Sergeant Major"), (45, "Warrant Officer"),
    (50, "Second Lieutenant"), (55, "Lieutenant"), (60, "Captain"), (65, "Major"), (70, "Lieutenant Colonel"),
    (75, "Colonel"), (80, "Brigadier"), (85, "Major General"), (90, "Lieutenant General"), (95, "General"),
    (100, "Commander"),
];
/// Past the top level, a service star for each this much XP, up to [`MAX_STARS`].
pub const STAR_XP: u64 = 100_000;
pub const MAX_STARS: u32 = 99;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rank {
    pub level: u32,
    /// UI callers translate this stable English label.
    pub title: &'static str,
    /// Its place in [`RANKS`]: the insignia.
    pub grade: usize,
    /// Service stars earned past the top level.
    pub stars: u32,
    pub xp_into_level: u64,
    /// XP needed from the beginning of this level; `None` at the top level, where the
    /// progress is toward the next service star.
    pub xp_for_next_level: Option<u64>,
    pub progress: f32,
    /// The next rank: its title and the level it begins at.
    pub next: Option<(&'static str, u32)>,
}

/// Total XP at the beginning of a level. Rank 1 starts at zero; rank 100 is finite.
pub const fn rank_threshold(level: u32) -> u64 {
    let level = if level < 1 { 1 } else if level > MAX_LEVEL { MAX_LEVEL } else { level } as u64;
    250 * (level - 1) * level
}

pub fn rank(xp: u64) -> Rank {
    let mut level = 1;
    while level < MAX_LEVEL && xp >= rank_threshold(level + 1) { level += 1; }
    let grade = RANKS.iter().rposition(|(from, _)| *from <= level).unwrap_or(0);
    let title = RANKS[grade].1;
    let next = RANKS.get(grade + 1).map(|(from, title)| (*title, *from));
    let xp_into_level = xp.saturating_sub(rank_threshold(level));
    let xp_for_next_level = (level < MAX_LEVEL).then(|| rank_threshold(level + 1) - rank_threshold(level));
    let stars = if level < MAX_LEVEL { 0 } else { (xp_into_level / STAR_XP).min(MAX_STARS as u64) as u32 };
    let progress = match xp_for_next_level {
        Some(required) => (xp_into_level as f64 / required as f64).clamp(0.0, 1.0) as f32,
        None if stars >= MAX_STARS => 1.0,
        None => (xp_into_level % STAR_XP) as f32 / STAR_XP as f32,
    };
    Rank { level, title, grade, stars, xp_into_level, xp_for_next_level, progress, next }
}

/// Where a match's XP came from: each source's part of [`MatchCounters::xp`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct XpBreakdown {
    /// Time played.
    pub time: u64,
    /// Enemies destroyed, 15 each at the current rate.
    pub combat: u64,
    /// Units and buildings finished.
    pub construction: u64,
    /// Units captured.
    pub captures: u64,
    /// Metal and energy produced.
    pub economy: u64,
    /// Finishing the battle, 75; winning it, 225 more at the current rate.
    pub completion: u64,
    pub victory: u64,
}

impl XpBreakdown {
    pub fn total(&self) -> u64 {
        [self.time, self.combat, self.construction, self.captures, self.economy, self.completion, self.victory]
            .into_iter().fold(0u64, u64::saturating_add)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Award {
    /// The battle's XP this checkpoint added, and the rewards of the achievements it unlocked.
    pub xp: u64,
    pub achievement_xp: u64,
    /// Cumulative battle sources under this match's recorded rate, for the result page.
    pub breakdown: XpBreakdown,
    /// The career's XP of every kind now ([`Career::total_xp`]).
    pub total_xp: u64,
    pub old_rank: Rank,
    pub new_rank: Rank,
    /// Achievements this checkpoint unlocked, in catalogue order.
    pub achievements: Vec<String>,
    /// Those of them that earlier play had already met (the list or the version being new):
    /// earned from the service record, not at this moment of the battle.
    pub earlier: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Selection { version: u32, active: Option<String> }

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct MatchEntry {
    /// Entries without this field were scored before the rate reduction. Keep their
    /// awards valid and finish those battles consistently, including imported baselines.
    #[serde(default = "original_xp_rate")]
    xp_rate: XpRate,
    high_water: MatchCounters,
    /// Historical score imported from another profile or an old, unprofiled save.
    baseline_xp: u64,
    awarded_xp: u64,
    context: Option<MatchContext>,
    baseline_counters: MatchCounters,
    baseline_awarded_xp: u64,
    details_ready: bool,
    started_at: u64,
    updated_at: u64,
    /// What the match gave the career that its totals cannot say, so that it can be taken back.
    gave: Gave,
    /// Cheats were used in it: it gives the career nothing, and what it gave was taken back.
    voided: bool,
}

/// What a match gave the career beyond its totals: the achievements it unlocked at a moment of
/// it, the single-battle records it set (each as it was before, and as the match left it), the
/// campaign missions it was the first to complete (by list, campaign and index).
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Gave {
    achievements: Vec<String>,
    feats: BTreeMap<String, (u64, u64)>,
    missions: Vec<(String, String, u32)>,
}

/// What voiding a match took back from the career: the battle's XP, the achievements (in
/// catalogue order), and the career's XP of every kind and rank before and after.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Voided {
    pub xp: u64,
    pub achievements: Vec<String>,
    pub total_xp: u64,
    pub old_rank: Option<Rank>,
    pub new_rank: Option<Rank>,
}

#[derive(Serialize, Deserialize)]
struct Ledger {
    version: u32,
    career: Career,
    matches: BTreeMap<String, MatchEntry>,
}

impl Default for Ledger {
    fn default() -> Self { Self { version: CAREER_VERSION, career: Career::default(), matches: BTreeMap::new() } }
}

/// No credentials or remote account: separate local histories on this device.
#[derive(Clone, Debug)]
pub struct ProfileStore { root: PathBuf }

impl ProfileStore {
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        if root.as_os_str().is_empty() { return Err(invalid("profile root is empty")); }
        Ok(Self { root: std::path::absolute(root)? })
    }
    pub fn root(&self) -> &Path { &self.root }

    fn profiles_dir(&self) -> io::Result<PathBuf> {
        let dir = self.root.join("profiles");
        reject_link(&dir)?;
        Ok(dir)
    }

    fn lock(&self) -> io::Result<File> {
        fs::create_dir_all(&self.root)?;
        let path = self.root.join("profiles.lock");
        reject_link(&path)?;
        let file = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(path)?;
        // Released on drop, including errors/process exit. Nothing waits for UI/network
        // input while holding it; every writer reloads the current file under this lock.
        file.lock()?;
        Ok(file)
    }

    /// Lists only fully created profiles. A failed creation never becomes selectable.
    pub fn list(&self) -> io::Result<Vec<Profile>> {
        self.scan_profiles(false).map(|(profiles, _)| profiles)
    }

    /// A chooser can still offer healthy identities when an unrelated profile is
    /// damaged. Writers use the strict list above so unread names cannot bypass
    /// uniqueness checks. Warnings identify skipped profiles without modifying them.
    pub fn list_available(&self) -> io::Result<(Vec<Profile>, Vec<String>)> {
        self.scan_profiles(true)
    }

    fn scan_profiles(&self, skip_invalid: bool) -> io::Result<(Vec<Profile>, Vec<String>)> {
        let entries = match fs::read_dir(self.profiles_dir()?) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), Vec::new())),
            Err(e) => return Err(e),
        };
        let mut profiles = Vec::new();
        let mut warnings = Vec::new();
        for (n, entry) in entries.enumerate() {
            if n >= MAX_PROFILES * 4 { return Err(invalid("too many profile folders")); }
            let entry = entry?;
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else { continue };
            if !valid_id(&id) { continue; }
            let result = reject_link(&entry.path()).and_then(|()| {
                if entry.path().join(PROFILE_FILE).exists() { self.get(&id).map(Some) }
                else { Ok(None) }
            });
            match result {
                Ok(Some(profile)) => profiles.push(profile),
                Ok(None) => continue,
                Err(error) if skip_invalid => warnings.push(format!("{}: {error}", entry.path().display())),
                Err(error) => return Err(error),
            }
            if profiles.len() > MAX_PROFILES { return Err(invalid("too many profiles")); }
        }
        profiles.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.name.cmp(&b.name)).then_with(|| a.id.cmp(&b.id)));
        Ok((profiles, warnings))
    }

    pub fn active(&self) -> io::Result<Option<Profile>> {
        self.selection()?.active.map(|id| self.get(&id)).transpose()
    }

    /// Older versions kept skirmish presets globally even after profiles were created.
    /// Adopt that preset for the oldest remaining profile, as with other legacy data.
    fn adopt_legacy_skirmish(&self, id: &str) -> io::Result<()> {
        let source = self.root.join("skirmish.toml");
        if !source.exists() { return Ok(()); }
        let _lock = self.lock()?;
        let dest = self.data_dir(id)?.join("skirmish.toml");
        reject_link(&source)?;
        reject_link(&dest)?;
        let profiles = self.list()?;
        if dest.exists() || profiles.first().is_none_or(|p| p.id != id) {
            return Ok(());
        }
        copy_legacy_file(&source, &dest, &mut (16 * 1024 * 1024), 16 * 1024 * 1024)
    }

    pub fn get(&self, id: &str) -> io::Result<Profile> {
        let dir = self.unchecked_dir(id)?;
        let mut profile: Profile = read_toml(&dir.join(PROFILE_FILE), MAX_PROFILE_FILE)?;
        if profile.id != id || clean_name(&profile.name)? != profile.name || profile.created_at > i64::MAX as u64 {
            return Err(invalid("invalid profile metadata"));
        }
        // (A motto or a showcase this version cannot show, written by hand or by another
        // version, is left out: the profile itself still reads.)
        if clean_motto(&profile.motto).ok().as_ref() != Some(&profile.motto) { profile.motto.clear(); }
        tidy_showcase(&mut profile.showcase);
        Ok(profile)
    }

    fn unchecked_dir(&self, id: &str) -> io::Result<PathBuf> {
        if !valid_id(id) { return Err(invalid("invalid profile ID")); }
        let dir = self.profiles_dir()?.join(id);
        reject_link(&dir)?;
        Ok(dir)
    }

    /// Validates both the ID and profile before exposing a personal storage path.
    pub fn data_dir(&self, id: &str) -> io::Result<PathBuf> {
        self.get(id)?;
        self.unchecked_dir(id)
    }

    pub fn config_path(&self, id: &str) -> io::Result<PathBuf> {
        let path = self.data_dir(id)?.join(crate::CONFIG_FILE);
        reject_link(&path)?;
        Ok(path)
    }

    pub fn avatar_path(&self, id: &str) -> io::Result<PathBuf> {
        let path = self.data_dir(id)?.join("portrait.png");
        reject_link(&path)?;
        Ok(path)
    }

    fn selection(&self) -> io::Result<Selection> {
        match read_toml::<Selection>(&self.root.join("profiles.toml"), MAX_PROFILE_FILE) {
            Ok(selection) if selection.version == VERSION => {
                if selection.active.as_deref().is_some_and(|id| !valid_id(id)) { return Err(invalid("invalid active profile ID")); }
                Ok(selection)
            }
            Ok(_) => Err(invalid("unsupported profile list version")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Selection { version: VERSION, active: None }),
            Err(e) => Err(e),
        }
    }

    pub fn select(&self, id: &str) -> io::Result<Profile> {
        let _lock = self.lock()?;
        let profile = self.get(id)?;
        write_toml(&self.root.join("profiles.toml"), &Selection { version: VERSION, active: Some(id.to_owned()) })?;
        Ok(profile)
    }

    /// Select and pin the same validated identity. No fallible filesystem access
    /// follows the selection write, so a failed handover cannot leave the on-disk
    /// selection pointing at a different commander from this process's pin.
    pub fn select_and_pin(&self, id: &str) -> io::Result<Profile> {
        let profile = self.select(id)?;
        set_context(self.root.clone(), Some(profile.id.clone()));
        Ok(profile)
    }

    pub fn create(&self, name: &str, allegiance: Allegiance, avatar: Option<&[u8]>, seed: &Config) -> io::Result<Profile> {
        let name = clean_name(name)?;
        // Decode outside the short filesystem transaction. Only the canonical PNG is
        // written, never a source path or unchecked original image.
        let avatar = avatar.map(normalize_avatar).transpose()?;
        let _lock = self.lock()?;
        let profiles = self.list()?;
        if profiles.len() >= MAX_PROFILES { return Err(invalid("profile limit reached")); }
        if profiles.iter().any(|p| same_name(&p.name, &name)) { return Err(invalid("a profile with this name already exists")); }
        // Never reset an unreadable active selection while creating another profile.
        self.selection()?;
        fs::create_dir_all(self.profiles_dir()?)?;
        let (id, dir) = loop {
            let id = new_id();
            let dir = self.unchecked_dir(&id)?;
            match fs::create_dir(&dir) {
                Ok(()) => break (id, dir),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        };
        let mut config = seed.clone();
        config.gameplay.player_name = name.clone();
        config.save_to(&dir.join(crate::CONFIG_FILE))?;
        let inherited_avatar = if profiles.is_empty() { copy_legacy(&self.root, &dir)? } else { false };
        if let Some(bytes) = &avatar { write_bytes(&dir.join("portrait.png"), bytes)?; }
        let profile = Profile { id, name, allegiance, has_avatar: avatar.is_some() || inherited_avatar, created_at: unix_time(), ..Default::default() };
        // Publish metadata last. Before this point, list() ignores incomplete folders.
        write_toml(&dir.join(PROFILE_FILE), &profile)?;
        write_toml(&self.root.join("profiles.toml"), &Selection { version: VERSION, active: Some(profile.id.clone()) })?;
        Ok(profile)
    }

    pub fn update(&self, id: &str, name: &str, allegiance: Allegiance) -> io::Result<Profile> {
        let name = clean_name(name)?;
        let _lock = self.lock()?;
        let mut profile = self.get(id)?;
        if self.list()?.iter().any(|p| p.id != id && same_name(&p.name, &name)) { return Err(invalid("a profile with this name already exists")); }
        let path = self.config_path(id)?;
        let mut config = Config::try_load_from(&path).map_err(|e| io::Error::other(e.to_string()))?;
        config.gameplay.player_name = name.clone();
        config.save_keeping_launcher(&path)?;
        profile.name = name;
        profile.allegiance = allegiance;
        write_toml(&self.unchecked_dir(id)?.join(PROFILE_FILE), &profile)?;
        Ok(profile)
    }

    /// Sets the line under the player's name (trimmed; empty for none).
    pub fn set_motto(&self, id: &str, motto: &str) -> io::Result<Profile> {
        let motto = clean_motto(motto)?;
        let _lock = self.lock()?;
        let mut profile = self.get(id)?;
        if profile.motto != motto {
            profile.motto = motto;
            write_toml(&self.unchecked_dir(id)?.join(PROFILE_FILE), &profile)?;
        }
        Ok(profile)
    }

    /// Sets the achievements shown on the profile, in order (known ones, at most [`MAX_SHOWCASE`]).
    pub fn set_showcase(&self, id: &str, showcase: &[String]) -> io::Result<Profile> {
        if !valid_showcase(showcase) { return Err(invalid("a profile shows up to five different achievements")); }
        let _lock = self.lock()?;
        let mut profile = self.get(id)?;
        profile.showcase = showcase.to_vec();
        write_toml(&self.unchecked_dir(id)?.join(PROFILE_FILE), &profile)?;
        Ok(profile)
    }

    /// Lets go of a profile about to be removed: it is no longer the one chosen. Returns its
    /// folder, for the caller to move to the Recycle Bin (none of it is deleted here).
    pub fn release(&self, id: &str) -> io::Result<PathBuf> {
        let _lock = self.lock()?;
        let dir = self.data_dir(id)?;
        if self.selection()?.active.as_deref() == Some(id) {
            write_toml(&self.root.join("profiles.toml"), &Selection { version: VERSION, active: None })?;
        }
        Ok(dir)
    }

    pub fn set_avatar(&self, id: &str, bytes: &[u8]) -> io::Result<Profile> {
        let png = normalize_avatar(bytes)?;
        let _lock = self.lock()?;
        let mut profile = self.get(id)?;
        write_bytes(&self.avatar_path(id)?, &png)?;
        profile.has_avatar = true;
        write_toml(&self.unchecked_dir(id)?.join(PROFILE_FILE), &profile)?;
        Ok(profile)
    }

    pub fn remove_avatar(&self, id: &str) -> io::Result<Profile> {
        let _lock = self.lock()?;
        let mut profile = self.get(id)?;
        // Stop referencing it first; interruption can only leave an unused picture.
        profile.has_avatar = false;
        write_toml(&self.unchecked_dir(id)?.join(PROFILE_FILE), &profile)?;
        match fs::remove_file(self.avatar_path(id)?) {
            Ok(()) => {}, Err(e) if e.kind() == io::ErrorKind::NotFound => {}, Err(e) => return Err(e),
        }
        Ok(profile)
    }

    fn ledger(&self, id: &str) -> io::Result<Ledger> {
        let path = self.data_dir(id)?.join(CAREER_FILE);
        let mut ledger = match read_toml::<Ledger>(&path, MAX_CAREER_FILE) {
            Ok(ledger) => ledger,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Ledger::default()),
            Err(e) => return Err(e),
        };
        if !(1..=CAREER_VERSION).contains(&ledger.version) { return Err(invalid("invalid career ledger")); }
        if ledger.version < 3 { ledger.career.retire_old_achievements(); }
        if ledger.matches.len() > MAX_MATCHES || !ledger.career.valid() {
            return Err(invalid("invalid career ledger"));
        }
        for (id, entry) in &ledger.matches {
            if !valid_match_id(id) || entry.high_water != entry.high_water.bounded()
                || entry.baseline_counters != entry.baseline_counters.bounded()
                || entry.context.as_ref().is_some_and(|c| *c != c.bounded())
                || entry.started_at > i64::MAX as u64 || entry.updated_at > i64::MAX as u64
                || entry.baseline_awarded_xp > entry.awarded_xp
                || entry.baseline_xp > entry.xp_rate.score(&entry.high_water)
                || entry.awarded_xp > entry.xp_rate.score(&entry.high_water).saturating_sub(entry.baseline_xp).min(MAX_MATCH_XP) {
                return Err(invalid("invalid career match history"));
            }
        }
        ledger.version = CAREER_VERSION;
        Ok(ledger)
    }

    pub fn career(&self, id: &str) -> io::Result<Career> { Ok(self.ledger(id)?.career) }

    /// The dossier's totals and history from one validated file snapshot. Reading
    /// twice both doubles large-ledger work and can mix two concurrent checkpoints.
    pub fn career_with_history(&self, id: &str) -> io::Result<(Career, Vec<MatchRecord>)> {
        let ledger = self.ledger(id)?;
        Ok((ledger.career, match_history(ledger.matches)))
    }

    /// Insert historical counters without awarding them. Existing entries are untouched.
    pub fn baseline(&self, id: &str, match_id: &str, counters: &MatchCounters) -> io::Result<()> {
        if !valid_match_id(match_id) { return Err(invalid("invalid match ID")); }
        let _lock = self.lock()?;
        let mut ledger = self.ledger(id)?;
        if ledger.matches.contains_key(match_id) { return Ok(()); }
        if ledger.matches.len() >= MAX_MATCHES { return Err(invalid("career match history limit reached")); }
        let high_water = counters.bounded();
        let baseline_xp = high_water.xp();
        ledger.matches.insert(match_id.to_owned(), MatchEntry { baseline_counters: high_water.clone(), high_water, baseline_xp, details_ready: true, ..Default::default() });
        write_toml_limited(&self.data_dir(id)?.join(CAREER_FILE), &ledger, MAX_CAREER_FILE)
    }

    /// Records with no observed post-import play are omitted, as are legacy entries
    /// whose map/mode were never recorded. Aggregate legacy career totals remain intact.
    pub fn history(&self, id: &str) -> io::Result<Vec<MatchRecord>> {
        Ok(match_history(self.ledger(id)?.matches))
    }

    /// Legacy callers still award genuine totals, without guessing a battle's mode.
    pub fn award(&self, id: &str, match_id: &str, counters: &MatchCounters) -> io::Result<Award> {
        self.award_inner(id, match_id, counters, None)
    }

    pub fn award_match(&self, id: &str, match_id: &str, counters: &MatchCounters, context: &MatchContext) -> io::Result<Award> {
        self.award_inner(id, match_id, counters, Some(context))
    }

    /// Award only increases above a persistent high-water mark. Saves/restarts/rewinds
    /// cannot pay twice. Caller excludes replays, benchmarks, menus and cheated sessions.
    fn award_inner(&self, id: &str, match_id: &str, counters: &MatchCounters, context: Option<&MatchContext>) -> io::Result<Award> {
        if !valid_match_id(match_id) { return Err(invalid("invalid match ID")); }
        let _lock = self.lock()?;
        let mut ledger = self.ledger(id)?;
        let old_rank = ledger.career.rank();
        let exists = ledger.matches.contains_key(match_id);
        if !exists && ledger.matches.len() >= MAX_MATCHES { return Err(invalid("career match history limit reached")); }
        let entry = ledger.matches.entry(match_id.to_owned()).or_default();
        // (A match with cheats in it gives nothing, from any copy of its saves.)
        if entry.voided {
            let rank = ledger.career.rank();
            return Ok(Award { xp: 0, achievement_xp: 0, breakdown: XpBreakdown::default(), total_xp: ledger.career.total_xp(), old_rank: rank, new_rank: rank, achievements: Vec::new(), earlier: Vec::new() });
        }
        let mut changed = !exists;
        // Old ledgers predate per-unit and loss tracking. Establish those fields at
        // their first observation so replayed historical constructions never count twice.
        if !entry.details_ready {
            if exists {
                entry.high_water.losses = counters.losses.min(MAX_COUNTER);
                entry.high_water.unit_builds = bounded_counts(&counters.unit_builds);
                entry.high_water.unit_names = counters.bounded().unit_names;
            }
            entry.details_ready = true;
            changed = true;
        }
        if entry.context.is_none() && !entry.high_water.finished && let Some(context) = context {
            entry.context = Some(context.bounded());
            entry.baseline_counters = entry.high_water.clone();
            entry.baseline_awarded_xp = entry.awarded_xp;
            entry.started_at = unix_time();
            changed = true;
        }
        // A concluded match is final, including its metadata and achievement conditions.
        let next = if entry.high_water.finished { entry.high_water.clone() } else { entry.high_water.highest(counters) };
        let earned = entry.xp_rate.score(&next).saturating_sub(entry.baseline_xp).min(MAX_MATCH_XP);
        let xp = earned.saturating_sub(entry.awarded_xp);
        let advanced = entry.high_water != next;
        changed |= advanced;
        let before = ledger.career.clone();
        ledger.career.add(&entry.high_water, &next, xp);
        if advanced && let Some(context) = &entry.context {
            ledger.career.add_details(&entry.high_water, &next, context, entry.baseline_counters == MatchCounters::default());
            entry.updated_at = unix_time();
            // What the totals cannot say, for taking it back should cheats void the match: the
            // records it raised (as they were before it first did) and the missions it added.
            for (key, &value) in &ledger.career.feats {
                let was = before.feats.get(key).copied().unwrap_or(0);
                if value > was { entry.gave.feats.entry(key.clone()).and_modify(|(_, set)| *set = value).or_insert((was, value)); }
            }
            for ((name, after), earlier_list) in MISSION_LISTS.iter().zip(ledger.career.mission_lists()).zip(before.mission_lists()) {
                for (campaign, indexes) in after {
                    for index in indexes {
                        if !earlier_list.get(campaign).is_some_and(|l| l.contains(index)) {
                            entry.gave.missions.push((name.to_string(), campaign.clone(), *index));
                        }
                    }
                }
            }
        }
        entry.high_water = next;
        entry.awarded_xp = earned;
        let breakdown = entry.high_water.xp_breakdown_at(entry.xp_rate);
        let battle = entry.context.as_ref().map(|c| (c, entry.high_water.played_seconds));
        let (achievements, earlier) = ledger.career.unlock_all_met(Some(&before), battle);
        for id in achievements.iter().filter(|id| !earlier.contains(id)) {
            if !entry.gave.achievements.contains(id) { entry.gave.achievements.push(id.clone()); }
        }
        changed |= !achievements.is_empty();
        if changed { write_toml_limited(&self.data_dir(id)?.join(CAREER_FILE), &ledger, MAX_CAREER_FILE)?; }
        let achievement_xp = achievements.iter().filter_map(|id| crate::achievements::find(id)).map(|a| a.tier.reward()).sum();
        Ok(Award { xp, achievement_xp, breakdown, total_xp: ledger.career.total_xp(), old_rank, new_rank: ledger.career.rank(), achievements, earlier })
    }

    /// Cheats were used in a match: what it gave the career is taken back for good (its XP, its
    /// share of the totals and records, the missions it completed first, the achievements it
    /// unlocked that the career no longer meets, and their pictures), and it gives nothing more,
    /// from any copy of its saves. Voiding a match twice takes nothing the second time.
    pub fn void_match(&self, id: &str, match_id: &str) -> io::Result<Voided> {
        if !valid_match_id(match_id) { return Err(invalid("invalid match ID")); }
        let _lock = self.lock()?;
        let mut ledger = self.ledger(id)?;
        let exists = ledger.matches.contains_key(match_id);
        if !exists && ledger.matches.len() >= MAX_MATCHES { return Err(invalid("career match history limit reached")); }
        if ledger.matches.get(match_id).is_some_and(|m| m.voided) {
            return Ok(Voided { total_xp: ledger.career.total_xp(), ..Voided::default() });
        }
        // The single-battle records of the battles left (those tracked from their beginning), for
        // the records this one set: one it beat is its again.
        let mut others: BTreeMap<&'static str, u64> = BTreeMap::new();
        for (other, m) in &ledger.matches {
            let Some(context) = m.context.as_ref().filter(|c| c.mode != MatchMode::Tutorial) else { continue };
            if other == match_id || m.voided || m.baseline_counters != MatchCounters::default() { continue; }
            for (key, value) in battle_feats(&m.high_water, context, m.high_water.finished) {
                let best = others.entry(key).or_default();
                *best = (*best).max(value.min(MAX_TOTAL));
            }
        }
        let entry = ledger.matches.entry(match_id.to_owned()).or_default();
        let old_rank = ledger.career.rank();
        let xp = entry.awarded_xp.saturating_sub(entry.baseline_awarded_xp);
        ledger.career.take(&entry.baseline_counters, &entry.high_water, xp);
        if let Some(context) = &entry.context {
            ledger.career.take_details(&entry.baseline_counters, &entry.high_water, context, &entry.gave, &others);
        }
        // Re-evaluate this match's achievements without their rewards, then restore those
        // still earned. Otherwise a rank badge's own XP can keep its threshold satisfied
        // after the battle that earned it was voided.
        let mut candidates = Vec::new();
        for id in &entry.gave.achievements {
            if let Some(at) = ledger.career.achievements.remove(id) {
                candidates.push((id.clone(), at, ledger.career.unlocks.remove(id)));
            }
        }
        loop {
            let before = candidates.len();
            candidates.retain(|(id, at, unlock)| {
                if crate::achievements::find(id).is_some_and(|a| a.progress(&ledger.career) >= a.target) {
                    ledger.career.achievements.insert(id.clone(), *at);
                    if let Some(unlock) = unlock { ledger.career.unlocks.insert(id.clone(), unlock.clone()); }
                    false
                } else { true }
            });
            if candidates.len() == before { break; }
        }
        let mut taken: Vec<String> = candidates.into_iter().map(|(id, _, _)| id).collect();
        let order = |id: &String| crate::achievements::find(id).map_or(usize::MAX, |a| a.index());
        taken.sort_by_key(order);
        entry.voided = true;
        entry.awarded_xp = entry.baseline_awarded_xp;
        entry.gave = Gave::default();
        write_toml_limited(&self.data_dir(id)?.join(CAREER_FILE), &ledger, MAX_CAREER_FILE)?;
        // (Their pictures were of a moment that no longer counts.)
        for a in &taken {
            if let Ok(path) = self.achievement_picture_path(id, a) {
                let _ = fs::remove_file(path);
            }
        }
        Ok(Voided { xp, achievements: taken, total_xp: ledger.career.total_xp(), old_rank: Some(old_rank), new_rank: Some(ledger.career.rank()) })
    }

    /// Unlocks what the career has already met: after an update brought achievements that
    /// earlier play earns (the launcher and the game ask on opening a profile). Returns them.
    pub fn refresh_achievements(&self, id: &str) -> io::Result<Vec<String>> {
        let _lock = self.lock()?;
        let mut ledger = self.ledger(id)?;
        if ledger.career.pending().is_empty() { return Ok(Vec::new()); }
        let (unlocked, _) = ledger.career.unlock_all_met(None, None);
        write_toml_limited(&self.data_dir(id)?.join(CAREER_FILE), &ledger, MAX_CAREER_FILE)?;
        Ok(unlocked)
    }

    /// Where the picture of an achievement's moment is kept (a JPEG): `achievements/<id>.jpg`
    /// in the profile's folder.
    pub fn achievement_picture_path(&self, id: &str, achievement: &str) -> io::Result<PathBuf> {
        if crate::achievements::find(achievement).is_none() { return Err(invalid("unknown achievement")); }
        let dir = self.data_dir(id)?.join(ACHIEVEMENT_PICTURES);
        reject_link(&dir)?;
        let path = dir.join(format!("{achievement}.jpg"));
        reject_link(&path)?;
        Ok(path)
    }

    /// Keeps `jpeg` as the picture of each of `achievements` (unlocked together, in one
    /// moment). An achievement's first picture is kept: a second one never replaces it.
    pub fn save_achievement_picture(&self, id: &str, achievements: &[String], jpeg: &[u8]) -> io::Result<()> {
        if jpeg.len() > MAX_ACHIEVEMENT_PICTURE || !jpeg.starts_with(&[0xff, 0xd8]) { return Err(invalid("achievement picture is not a bounded JPEG")); }
        let dir = self.data_dir(id)?.join(ACHIEVEMENT_PICTURES);
        reject_link(&dir)?;
        fs::create_dir_all(&dir)?;
        for achievement in achievements {
            let path = self.achievement_picture_path(id, achievement)?;
            if !path.exists() { write_bytes(&path, jpeg)?; }
        }
        Ok(())
    }

    /// The achievements that have a picture of their moment.
    pub fn achievement_pictures(&self, id: &str) -> io::Result<std::collections::BTreeSet<String>> {
        let dir = self.data_dir(id)?.join(ACHIEVEMENT_PICTURES);
        reject_link(&dir)?;
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Default::default()),
            Err(e) => return Err(e),
        };
        Ok(entries.filter_map(Result::ok).filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".jpg")).map(str::to_owned))
            .filter(|n| crate::achievements::find(n).is_some()).collect())
    }

}

fn match_history(matches: BTreeMap<String, MatchEntry>) -> Vec<MatchRecord> {
    let mut history: Vec<_> = matches.into_iter().filter(|(_, entry)| !entry.voided).filter_map(|(id, entry)| {
        let context = entry.context?;
        let counters = entry.high_water.since(&entry.baseline_counters);
        (entry.updated_at != 0).then_some(MatchRecord { id, context, counters, earned_xp: entry.awarded_xp.saturating_sub(entry.baseline_awarded_xp),
            started_at: entry.started_at, updated_at: entry.updated_at })
    }).collect();
    history.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| b.id.cmp(&a.id)));
    history
}

#[derive(Clone)]
struct Context { root: PathBuf, id: Option<String> }
static CONTEXT: OnceLock<RwLock<Context>> = OnceLock::new();

pub(crate) fn context_root() -> Option<PathBuf> {
    CONTEXT.get().map(|c| c.read().unwrap_or_else(|e| e.into_inner()).root.clone())
}
pub(crate) fn context_data_dir() -> Option<PathBuf> {
    CONTEXT.get().map(|c| {
        let c = c.read().unwrap_or_else(|e| e.into_inner());
        c.id.as_ref().map_or_else(|| c.root.clone(), |id| c.root.join("profiles").join(id))
    })
}

/// The pinned profile's ID, independent of later launcher selection changes.
pub fn current_profile_id() -> Option<String> {
    CONTEXT.get().and_then(|c| c.read().unwrap_or_else(|e| e.into_inner()).id.clone())
}

pub fn current_profile() -> io::Result<Option<Profile>> {
    let Some(context) = CONTEXT.get() else { return Ok(None) };
    let context = context.read().unwrap_or_else(|e| e.into_inner()).clone();
    context.id.map(|id| ProfileStore::open(context.root)?.get(&id)).transpose()
}

fn set_context(root: PathBuf, id: Option<String>) {
    let context = CONTEXT.get_or_init(|| RwLock::new(Context { root: root.clone(), id: None }));
    *context.write().unwrap_or_else(|e| e.into_inner()) = Context { root, id };
}

/// Call at startup before settings/progress load. An explicit unknown ID is an error,
/// never a fallback to another local player's identity.
pub fn initialize(root: PathBuf, requested_id: Option<&str>) -> io::Result<Option<Profile>> {
    let store = ProfileStore::open(root.clone())?;
    let profile = match requested_id { Some(id) => Some(store.get(id)?), None => store.active()? };
    if let Some(profile) = &profile {
        if let Err(e) = store.adopt_legacy_skirmish(&profile.id) { log::warn!("could not adopt the legacy skirmish preset: {e}"); }
    }
    set_context(store.root.clone(), profile.as_ref().map(|p| p.id.clone()));
    Ok(profile)
}

/// Explicitly switch the local UI's pinned identity. Games call this before a match.
pub fn pin(root: PathBuf, id: &str) -> io::Result<Profile> {
    let store = ProfileStore::open(root.clone())?;
    let profile = store.get(id)?;
    if let Err(e) = store.adopt_legacy_skirmish(&profile.id) { log::warn!("could not adopt the legacy skirmish preset: {e}"); }
    set_context(store.root.clone(), Some(profile.id.clone()));
    Ok(profile)
}

/// Filesystem-safe identity independent of deterministic simulation seeds. Exclusive
/// directory creation also guarantees no clock collision can reuse a profile folder.
pub fn new_id() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let unique = stamp ^ ((std::process::id() as u128) << 64) ^ ((SEQUENCE.fetch_add(1, Ordering::Relaxed) as u128) << 96);
    format!("{unique:032x}")
}

fn unix_time() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs().min(i64::MAX as u64) }
fn valid_id(id: &str) -> bool { id.len() == 32 && id.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) }
fn valid_match_id(id: &str) -> bool { !id.is_empty() && id.len() <= 96 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') }
fn same_name(a: &str, b: &str) -> bool { a.to_lowercase() == b.to_lowercase() }
fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }

/// Normalizes a display name; paths always use an unrelated generated identity.
pub fn clean_name(name: &str) -> io::Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS || name.len() > 128
        || name.chars().any(|c| c.is_control() || ('\u{202a}'..='\u{202e}').contains(&c) || ('\u{2066}'..='\u{2069}').contains(&c)) {
        return Err(invalid("profile name must contain 1 to 24 visible characters"));
    }
    Ok(name.to_owned())
}

/// A motto as kept: trimmed, at most [`MAX_MOTTO_CHARS`], no control or direction characters.
pub fn clean_motto(motto: &str) -> io::Result<String> {
    let motto = motto.trim();
    if motto.chars().count() > MAX_MOTTO_CHARS || motto.len() > 4 * MAX_MOTTO_CHARS
        || motto.chars().any(|c| c.is_control() || ('\u{202a}'..='\u{202e}').contains(&c) || ('\u{2066}'..='\u{2069}').contains(&c)) {
        return Err(invalid("a motto has at most 60 visible characters"));
    }
    Ok(motto.to_owned())
}

/// Keeps a showcase's known achievements, each once, at most [`MAX_SHOWCASE`] of them.
fn tidy_showcase(showcase: &mut Vec<String>) {
    let mut kept: Vec<String> = Vec::new();
    for id in showcase.drain(..) {
        if kept.len() < MAX_SHOWCASE && crate::achievements::find(&id).is_some() && !kept.contains(&id) { kept.push(id); }
    }
    *showcase = kept;
}

fn valid_showcase(showcase: &[String]) -> bool {
    showcase.len() <= MAX_SHOWCASE && showcase.iter().all(|id| crate::achievements::find(id).is_some())
        && showcase.iter().enumerate().all(|(i, id)| !showcase[..i].contains(id))
}

/// Bounded PNG/JPEG import, cropped to a square and stored as a 256-pixel PNG.
pub fn normalize_avatar(bytes: &[u8]) -> io::Result<Vec<u8>> {
    if bytes.is_empty() || bytes.len() > MAX_AVATAR_INPUT { return Err(invalid("profile picture exceeds the 16 MiB limit")); }
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(io::Error::other)?;
    if !matches!(reader.format(), Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)) { return Err(invalid("profile picture must be a PNG or JPEG")); }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(io::Error::other)?;
    let avatar = image.resize_to_fill(256, 256, image::imageops::FilterType::Lanczos3).to_rgba8();
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(avatar).write_to(&mut output, image::ImageFormat::Png).map_err(io::Error::other)?;
    Ok(output.into_inner())
}

fn reject_link(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            #[cfg(windows)]
            let linked = { use std::os::windows::fs::MetadataExt; meta.file_attributes() & 0x400 != 0 };
            #[cfg(not(windows))]
            let linked = meta.file_type().is_symlink();
            if linked { return Err(invalid("linked paths are not allowed in player profiles")); }
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> io::Result<T> {
    reject_link(path)?;
    let file = File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > limit { return Err(invalid("profile file exceeds size limit")); }
    let mut text = String::new();
    file.take(limit + 1).read_to_string(&mut text)?;
    if text.len() as u64 > limit { return Err(invalid("profile file exceeds size limit")); }
    toml::from_str(&text).map_err(io::Error::other)
}

fn write_toml<T: Serialize>(path: &Path, value: &T) -> io::Result<()> { write_toml_limited(path, value, MAX_PROFILE_FILE) }
fn write_toml_limited<T: Serialize>(path: &Path, value: &T, limit: u64) -> io::Result<()> {
    let text = toml::to_string_pretty(value).map_err(io::Error::other)?;
    if text.len() as u64 > limit { return Err(invalid("profile file exceeds size limit")); }
    write_bytes(path, text.as_bytes())
}

fn write_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    reject_link(path)?;
    let parent = path.parent().ok_or_else(|| invalid("profile file has no parent"))?;
    reject_link(parent)?;
    let tmp = parent.join(format!(".{}.tmp", new_id()));
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() { let _ = fs::remove_file(&tmp); }
    result
}

/// Copy only known personal files into the first profile, leaving originals in place.
/// Mods, skin packs, caches, logs and executable content are never copied.
fn copy_legacy(root: &Path, dest: &Path) -> io::Result<bool> {
    let mut budget = 8 * 1024 * 1024 * 1024u64;
    for file in ["progress.toml", "conquest.toml", "reconnect.json", "skirmish.toml"] {
        let source = root.join(file);
        reject_link(&source)?;
        if source.is_file() { copy_legacy_file(&source, &dest.join(file), &mut budget, 16 * 1024 * 1024)?; }
    }
    for (folder, extension) in [("saves", "tasav"), ("replays", "tarep")] {
        let source = root.join(folder);
        reject_link(&source)?;
        if !source.exists() { continue; }
        let mut count = 0;
        copy_legacy_records(&source, &dest.join(folder), extension, &mut budget, &mut count, 0)?;
    }
    let portrait = root.join("portrait.png");
    reject_link(&portrait)?;
    if portrait.is_file() {
        let file = File::open(portrait)?;
        let mut bytes = Vec::new();
        file.take(MAX_AVATAR_INPUT as u64 + 1).read_to_end(&mut bytes)?;
        match normalize_avatar(&bytes) {
            Ok(png) => { write_bytes(&dest.join("portrait.png"), &png)?; return Ok(true); }
            Err(e) => log::warn!("legacy profile picture could not be adopted: {e}"),
        }
    }
    Ok(false)
}

fn copy_legacy_file(source: &Path, dest: &Path, budget: &mut u64, max_file: u64) -> io::Result<()> {
    reject_link(source)?;
    let source = File::open(source)?;
    let meta = source.metadata()?;
    if !meta.is_file() || meta.len() > max_file || meta.len() > *budget { return Err(invalid("legacy player data exceeds copy limits")); }
    let mut target = OpenOptions::new().write(true).create_new(true).open(dest)?;
    let copied = io::copy(&mut source.take(meta.len() + 1), &mut target)?;
    if copied != meta.len() { return Err(invalid("legacy player data changed while being copied")); }
    *budget -= copied;
    target.sync_all()?;
    Ok(())
}

fn copy_legacy_records(source: &Path, dest: &Path, ext: &str, budget: &mut u64, count: &mut usize, depth: usize) -> io::Result<()> {
    if depth > 8 { return Err(invalid("legacy records are nested too deeply")); }
    fs::create_dir(dest)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        *count += 1;
        if *count > 10_000 { return Err(invalid("too many legacy records")); }
        let path = entry.path();
        reject_link(&path)?;
        if entry.file_type()?.is_dir() {
            copy_legacy_records(&path, &dest.join(entry.file_name()), ext, budget, count, depth + 1)?;
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) {
            copy_legacy_file(&path, &dest.join(entry.file_name()), budget, 256 * 1024 * 1024)?;
        }
    }
    Ok(())
}

/// Optional extension to the existing record body, leaving deterministic format intact.
pub const RECORD_PROFILE_TRAILER_LEN: usize = 48;
const RECORD_PROFILE_MAGIC: &[u8; 8] = b"TAXP0001";

pub fn record_metadata(bytes: &[u8]) -> Option<(&str, bool)> {
    let trailer = bytes.get(bytes.len().checked_sub(RECORD_PROFILE_TRAILER_LEN)?..)?;
    if &trailer[..8] != RECORD_PROFILE_MAGIC || trailer[40] > 1 || trailer[41..].iter().any(|b| *b != 0) { return None; }
    let id = std::str::from_utf8(&trailer[8..40]).ok()?;
    valid_id(id).then_some((id, trailer[40] == 1))
}

pub fn append_record_metadata(bytes: &mut Vec<u8>, id: &str, disqualified: bool) -> io::Result<()> {
    if !valid_id(id) { return Err(invalid("invalid record match ID")); }
    bytes.extend_from_slice(RECORD_PROFILE_MAGIC);
    bytes.extend_from_slice(id.as_bytes());
    bytes.push(u8::from(disqualified));
    bytes.extend_from_slice(&[0; 7]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestRoot(PathBuf);
    impl TestRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("ta-profile-test-{}", new_id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn store(&self) -> ProfileStore { ProfileStore::open(self.0.clone()).unwrap() }
    }
    impl Drop for TestRoot { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
    fn create(store: &ProfileStore, name: &str) -> Profile { store.create(name, Allegiance::Arm, None, &Config::default()).unwrap() }

    #[test]
    fn profiles_isolate_settings_history_and_selection() {
        let root = TestRoot::new();
        let store = root.store();
        assert!(store.active().unwrap().is_none());
        let a = create(&store, "Alex");
        let b = create(&store, "Sam");
        assert_ne!(a.id, b.id);
        assert_eq!(store.active().unwrap().unwrap(), b);
        assert_eq!(store.list().unwrap().len(), 2);
        store.select(&a.id).unwrap();
        assert_eq!(store.active().unwrap().unwrap(), a);
        assert_ne!(store.data_dir(&a.id).unwrap(), store.data_dir(&b.id).unwrap());
        let mut a_config = Config::load_from(&store.config_path(&a.id).unwrap());
        a_config.graphics.unit_skin = "christmas".into();
        a_config.save_to(&store.config_path(&a.id).unwrap()).unwrap();
        assert_eq!(Config::load_from(&store.config_path(&b.id).unwrap()).graphics.unit_skin, "reborn");
        store.award(&a.id, "match-a", &MatchCounters { kills: 2, ..Default::default() }).unwrap();
        assert_eq!(store.career(&a.id).unwrap().xp, 30);
        assert_eq!(store.career(&b.id).unwrap().xp, 0);
        let reopened = root.store();
        assert_eq!(reopened.active().unwrap().unwrap().id, a.id);
        assert_eq!(reopened.career(&a.id).unwrap().xp, 30);
    }

    #[test]
    fn chooser_isolates_invalid_profiles_without_relaxing_writer_validation() {
        let root = TestRoot::new();
        let store = root.store();
        let healthy = create(&store, "Healthy");
        let damaged = create(&store, "Damaged");
        store.select(&healthy.id).unwrap();
        let path = store.data_dir(&damaged.id).unwrap().join(PROFILE_FILE);
        let original = fs::read(&path).unwrap();
        fs::write(&path, "name = [broken metadata").unwrap();

        let (available, warnings) = store.list_available().unwrap();
        assert_eq!(available, [healthy.clone()]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains(&damaged.id));
        assert_eq!(store.active().unwrap(), Some(healthy.clone()));
        assert!(store.list().is_err(), "writers still need every identity for uniqueness checks");
        assert!(store.create("New", Allegiance::Arm, None, &Config::default()).is_err());
        assert!(store.update(&healthy.id, "Renamed", Allegiance::Arm).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "name = [broken metadata");

        fs::write(&path, original).unwrap();
        let (available, warnings) = store.list_available().unwrap();
        assert_eq!(available.len(), 2);
        assert!(warnings.is_empty(), "a repaired identity returns to the chooser");
    }

    #[test]
    fn first_profile_copies_legacy_without_removing_or_sharing_it() {
        let root = TestRoot::new();
        let store = root.store();
        fs::write(root.0.join("progress.toml"), "difficulty = 'Hard'\n").unwrap();
        fs::write(root.0.join("conquest.toml"), "seed = 123\n").unwrap();
        fs::write(root.0.join("skirmish.toml"), "map = 'Sherwood'\n").unwrap();
        fs::create_dir(root.0.join("saves")).unwrap();
        fs::write(root.0.join("saves/autosave.tasav"), b"legacy").unwrap();
        fs::write(root.0.join("saves/run.exe"), b"not copied").unwrap();
        fs::create_dir(root.0.join("mods")).unwrap();
        let a = create(&store, "First");
        let b = create(&store, "Second");
        let a_dir = store.data_dir(&a.id).unwrap();
        let b_dir = store.data_dir(&b.id).unwrap();
        assert!(root.0.join("progress.toml").exists());
        assert!(root.0.join("saves/autosave.tasav").exists());
        assert_eq!(fs::read(a_dir.join("saves/autosave.tasav")).unwrap(), b"legacy");
        assert!(a_dir.join("progress.toml").exists() && a_dir.join("conquest.toml").exists());
        assert_eq!(fs::read_to_string(a_dir.join("skirmish.toml")).unwrap(), "map = 'Sherwood'\n");
        assert!(root.0.join("skirmish.toml").exists(), "keep the legacy setup intact");
        assert!(!a_dir.join("saves/run.exe").exists() && !a_dir.join("mods").exists());
        assert!(!b_dir.join("progress.toml").exists() && !b_dir.join("saves").exists());
        assert!(!b_dir.join("skirmish.toml").exists(), "later profiles start with their own setup");
    }

    #[test]
    fn existing_profiles_adopt_the_legacy_skirmish_once_without_sharing_it() {
        let root = TestRoot::new();
        let store = root.store();
        let a = create(&store, "First");
        let b = create(&store, "Second");
        // Simulate an older version saving a global preset after onboarding.
        fs::write(root.0.join("skirmish.toml"), "map = 'Sherwood'\n").unwrap();
        store.adopt_legacy_skirmish(&b.id).unwrap();
        let second_preset = store.data_dir(&b.id).unwrap().join("skirmish.toml");
        assert!(!second_preset.exists());
        fs::write(&second_preset, "map = 'Coast To Coast'\n").unwrap();
        store.adopt_legacy_skirmish(&a.id).unwrap();
        let preset = store.data_dir(&a.id).unwrap().join("skirmish.toml");
        assert_eq!(fs::read_to_string(&preset).unwrap(), "map = 'Sherwood'\n");
        assert_eq!(fs::read_to_string(&second_preset).unwrap(), "map = 'Coast To Coast'\n");
        fs::write(&preset, "map = 'Seven Islands'\n").unwrap();
        store.adopt_legacy_skirmish(&a.id).unwrap();
        assert_eq!(fs::read_to_string(&preset).unwrap(), "map = 'Seven Islands'\n");
        assert!(root.0.join("skirmish.toml").exists());
    }

    #[test]
    fn ids_names_and_corrupt_selection_never_fall_back_to_another_profile() {
        let root = TestRoot::new();
        let store = root.store();
        let a = create(&store, "Player");
        for id in ["", "..", "../../outside", "C:\\outside", "A1234567890123456789012345678901", "001"] {
            assert!(store.data_dir(id).is_err());
            assert!(store.select(id).is_err());
        }
        for name in ["", " \t ", "hidden\nname", "bad\u{202e}name", "123456789012345678901234567890123"] {
            assert!(store.create(name, Allegiance::Core, None, &Config::default()).is_err());
        }
        assert!(store.create(" player ", Allegiance::Core, None, &Config::default()).is_err());
        assert_eq!(store.active().unwrap().unwrap(), a);
        fs::write(root.0.join("profiles.toml"), "broken = [").unwrap();
        assert!(store.active().is_err());
        assert!(store.create("New", Allegiance::Core, None, &Config::default()).is_err());
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn checkpoints_reload_and_rewinds_do_not_duplicate_xp() {
        let root = TestRoot::new();
        let store = root.store();
        let p = create(&store, "Player");
        let mut c = MatchCounters { played_seconds: 60, kills: 2, units_built: 3, buildings_built: 1, units_captured: 1, metal_gathered: 250, energy_gathered: 2400, ..Default::default() };
        let initial = store.award(&p.id, "match", &c).unwrap();
        assert_eq!(initial.xp, c.xp());
        assert_eq!(store.award(&p.id, "match", &c).unwrap().xp, 0);
        assert_eq!(root.store().award(&p.id, "match", &c).unwrap().xp, 0);
        assert_eq!(store.award(&p.id, "match", &MatchCounters::default()).unwrap().xp, 0);
        c.kills += 1;
        c.played_seconds = 90;
        c.finished = true;
        c.won = true;
        assert_eq!(store.award(&p.id, "match", &c).unwrap().xp, 319);
        assert_eq!(store.award(&p.id, "match", &c).unwrap().xp, 0);
        let career = store.career(&p.id).unwrap();
        assert_eq!((career.xp, career.matches_finished, career.wins, career.kills, career.units_built), (c.xp(), 1, 1, 3, 3));
    }

    #[test]
    fn completed_matches_never_award_another_ending_or_rewound_actions() {
        let root = TestRoot::new();
        let store = root.store();
        let p = create(&store, "Player");
        let final_counters = MatchCounters { played_seconds: 90, kills: 2, finished: true, ..Default::default() };
        store.award(&p.id, "finished", &final_counters).unwrap();
        let original = store.career(&p.id).unwrap();
        let alternate = MatchCounters { played_seconds: 300, kills: 20, finished: true, won: true, ..Default::default() };
        assert_eq!(store.award(&p.id, "finished", &alternate).unwrap().xp, 0);
        assert_eq!(store.career(&p.id).unwrap(), original);
        store.baseline(&p.id, "imported-finished", &final_counters).unwrap();
        assert_eq!(store.award(&p.id, "imported-finished", &alternate).unwrap().xp, 0);
        assert_eq!(store.career(&p.id).unwrap(), original);
    }

    #[test]
    fn imported_save_baseline_earns_only_new_actions() {
        let root = TestRoot::new();
        let store = root.store();
        let p = create(&store, "Player");
        let mut c = MatchCounters { played_seconds: 300, kills: 50, units_built: 20, metal_gathered: 10000, ..Default::default() };
        store.baseline(&p.id, "imported", &c).unwrap();
        assert_eq!(store.award(&p.id, "imported", &c).unwrap().xp, 0);
        assert_eq!(store.career(&p.id).unwrap(), Career::default());
        c.kills += 1;
        c.played_seconds += 30;
        assert_eq!(store.award(&p.id, "imported", &c).unwrap().xp, 19);
        store.baseline(&p.id, "imported", &MatchCounters::default()).unwrap();
        assert_eq!(store.award(&p.id, "imported", &c).unwrap().xp, 0);
        let career = store.career(&p.id).unwrap();
        assert_eq!((career.kills, career.played_seconds, career.xp), (1, 30, 19));
    }

    #[test]
    fn frequent_checkpoints_preserve_fractional_resource_and_time_xp() {
        let root = TestRoot::new();
        let store = root.store();
        let a = create(&store, "Frequent");
        let b = create(&store, "Once");
        for n in 1..=90 {
            store.award(&a.id, "game", &MatchCounters { played_seconds: n, metal_gathered: n * 5, energy_gathered: n * 60, ..Default::default() }).unwrap();
        }
        store.award(&b.id, "game", &MatchCounters { played_seconds: 90, metal_gathered: 450, energy_gathered: 5400, ..Default::default() }).unwrap();
        assert_eq!(store.career(&a.id).unwrap(), store.career(&b.id).unwrap());
    }

    #[test]
    fn concurrent_instances_merge_career_without_losing_matches() {
        let root = TestRoot::new();
        let store = root.store();
        let p = create(&store, "Player");
        let threads: Vec<_> = (0..8).map(|n| {
            let store = store.clone();
            let id = p.id.clone();
            std::thread::spawn(move || store.award(&id, &format!("match-{n}"), &MatchCounters { kills: 1, finished: true, ..Default::default() }).unwrap())
        }).collect();
        for thread in threads { thread.join().unwrap(); }
        let career = store.career(&p.id).unwrap();
        assert_eq!((career.xp, career.kills, career.matches_finished), (720, 8, 8));
    }

    #[test]
    fn invalid_ledger_is_not_reset_and_extreme_stats_are_bounded() {
        let root = TestRoot::new();
        let store = root.store();
        let p = create(&store, "Player");
        let c = MatchCounters { kills: u64::MAX, units_built: u64::MAX, buildings_built: u64::MAX, energy_gathered: u64::MAX, ..Default::default() };
        assert_eq!(store.award(&p.id, "huge-mod", &c).unwrap().xp, MAX_MATCH_XP);
        assert_eq!(store.award(&p.id, "huge-mod", &c).unwrap().xp, 0);
        assert_eq!(store.career(&p.id).unwrap().kills, MAX_COUNTER);
        let path = store.data_dir(&p.id).unwrap().join(CAREER_FILE);
        fs::write(&path, "invalid = [").unwrap();
        assert!(store.award(&p.id, "another", &MatchCounters { kills: 1, ..Default::default() }).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "invalid = [");
    }

    #[test]
    fn ranks_have_exact_thresholds_and_a_finite_cap() {
        assert_eq!(rank(0).level, 1);
        for level in 2..=MAX_LEVEL {
            let threshold = rank_threshold(level);
            assert_eq!(rank(threshold - 1).level, level - 1);
            assert_eq!(rank(threshold).level, level);
            assert_eq!(rank(threshold).progress, 0.0, "the top level's progress is toward its first star");
        }
        let max = rank(u64::MAX);
        assert_eq!(max.level, MAX_LEVEL);
        assert_eq!(max.xp_for_next_level, None);
        assert_eq!((max.stars, max.progress), (MAX_STARS, 1.0));
    }

    #[test]
    fn ranks_change_every_five_levels_and_stars_follow_the_top() {
        let at = |level: u32| rank(rank_threshold(level));
        assert_eq!((at(1).title, at(1).grade, at(1).next), ("Recruit", 0, Some(("Private", 5))));
        assert_eq!((at(4).title, at(5).title, at(5).grade), ("Recruit", "Private", 1));
        assert_eq!((at(32).title, at(32).next), ("Staff Sergeant", Some(("Master Sergeant", 35))));
        assert_eq!((at(99).title, at(99).grade), ("General", 19));
        assert_eq!((at(100).title, at(100).grade, at(100).next, at(100).stars), ("Commander", 20, None, 0));
        assert!(RANKS.windows(2).all(|w| w[0].0 < w[1].0) && RANKS.last().unwrap().0 == MAX_LEVEL);
        let starred = rank(rank_threshold(MAX_LEVEL) + 2 * STAR_XP + STAR_XP / 2);
        assert_eq!((starred.stars, starred.progress), (2, 0.5));
    }

    #[test]
    fn xp_by_source_adds_up_to_the_match_xp() {
        let c = MatchCounters { played_seconds: 95, kills: 3, units_built: 4, buildings_built: 2, units_captured: 1,
            metal_gathered: 250, energy_gathered: 3500, finished: true, won: true, ..Default::default() };
        let b = c.xp_breakdown();
        assert_eq!((b.time, b.combat, b.construction, b.captures, b.economy, b.completion, b.victory), (11, 45, 52, 22, 3, 75, 225));
        assert_eq!(b.total(), c.xp());
    }

    #[test]
    fn new_battles_earn_career_xp_at_a_more_moderate_rate() {
        let counters = MatchCounters {
            played_seconds: 1800, kills: 100, units_built: 80, buildings_built: 20,
            units_captured: 4, metal_gathered: 50_000, energy_gathered: 400_000,
            finished: true, won: true, ..Default::default()
        };
        let b = counters.xp_breakdown();
        assert_eq!((b.time, b.combat, b.construction, b.captures, b.economy, b.completion, b.victory),
            (225, 1500, 825, 90, 675, 75, 225));
        assert_eq!(b.total(), 3615, "a representative half-hour victory earns 25% less battle XP");
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Steady");
        assert_eq!(store.award(&p.id, "battle", &counters).unwrap().xp, 3615);
        assert_eq!(store.award(&p.id, "battle", &counters).unwrap().xp, 0, "repeating a checkpoint cannot award XP again");
    }

    #[test]
    fn original_career_scores_and_in_progress_matches_survive_the_rate_change() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Veteran");
        let path = store.data_dir(&p.id).unwrap().join(CAREER_FILE);
        fs::write(&path, "version = 4\n[career]\nxp = 100\nkills = 5\n[matches.original]\nawarded_xp = 100\ndetails_ready = true\n[matches.original.high_water]\nkills = 5\n[matches.imported]\nbaseline_xp = 100\ndetails_ready = true\n[matches.imported.high_water]\nkills = 5\n").unwrap();
        assert_eq!(store.career(&p.id).unwrap().xp, 100, "existing XP is preserved");
        let counters = MatchCounters { kills: 6, ..Default::default() };
        let award = store.award(&p.id, "original", &counters).unwrap();
        assert_eq!(award.xp, 20, "a battle already being tracked keeps its original scoring rules");
        assert_eq!(award.breakdown.combat, 120, "the result page must use that battle's recorded rate too");
        assert_eq!(root.store().award(&p.id, "original", &counters).unwrap().xp, 0);
        assert_eq!(store.award(&p.id, "new", &MatchCounters { kills: 1, ..Default::default() }).unwrap().xp, 15);
        assert_eq!(root.store().career(&p.id).unwrap().xp, 135);
        assert_eq!(store.award(&p.id, "imported", &counters).unwrap().xp, 20, "original imported baselines retain their score too");
        assert_eq!(root.store().career(&p.id).unwrap().xp, 155);
    }

    #[test]
    fn achievements_grant_xp_by_tier_and_can_raise_the_rank_to_another() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Rewards");
        // 130 XP of battle and four bronze achievements (400) cross rank 2 (500): its own
        // achievement follows in the same checkpoint.
        let counters = MatchCounters { kills: 1, units_built: 1, buildings_built: 1, units_captured: 1, finished: true, ..Default::default() };
        let award = store.award_match(&p.id, "first", &counters, &battle_context(MatchMode::Skirmish)).unwrap();
        assert_eq!(award.xp, 130);
        assert_eq!(award.achievements, ["career_kills_1", "career_captures_1", "career_built_1", "career_buildings_1", "rank_2"]);
        assert_eq!(award.achievement_xp, 500);
        assert_eq!((award.old_rank.level, award.new_rank.level, award.total_xp), (1, 2, 630));
        let career = store.career(&p.id).unwrap();
        assert_eq!((career.xp, career.achievement_xp(), career.total_xp()), (130, 500, 630));
        assert!(!career.unlocks["rank_2"].earlier, "earned in this battle");
    }

    #[test]
    fn avatar_is_canonicalized_and_never_shared() {
        let root = TestRoot::new();
        let store = root.store();
        let p = create(&store, "Player");
        let img = image::RgbaImage::from_pixel(640, 480, image::Rgba([30, 80, 160, 255]));
        let mut source = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut source, image::ImageFormat::Png).unwrap();
        assert!(store.set_avatar(&p.id, &source.into_inner()).unwrap().has_avatar);
        let avatar = image::load_from_memory(&fs::read(store.avatar_path(&p.id).unwrap()).unwrap()).unwrap();
        assert_eq!((avatar.width(), avatar.height()), (256, 256));
        assert!(store.set_avatar(&p.id, b"not an image").is_err());
        assert!(store.get(&p.id).unwrap().has_avatar);
        assert!(!store.remove_avatar(&p.id).unwrap().has_avatar);
        assert!(!store.avatar_path(&p.id).unwrap().exists());
    }

    #[test]
    fn edit_updates_name_without_touching_career_or_other_settings() {
        let root = TestRoot::new();
        let store = root.store();
        let p = create(&store, "Player");
        let path = store.config_path(&p.id).unwrap();
        let mut config = Config::load_from(&path);
        config.audio.music = 0.12;
        config.save_to(&path).unwrap();
        store.award(&p.id, "match", &MatchCounters { kills: 10, ..Default::default() }).unwrap();
        let p = store.update(&p.id, "New Name", Allegiance::Core).unwrap();
        assert_eq!((p.name.as_str(), p.allegiance), ("New Name", Allegiance::Core));
        let updated = Config::load_from(&path);
        assert_eq!((updated.gameplay.player_name.as_str(), updated.audio.music), ("New Name", 0.12));
        assert_eq!(store.career(&p.id).unwrap().xp, 150);
    }

    #[test]
    fn incomplete_profile_and_invalid_avatar_never_become_selectable() {
        let root = TestRoot::new();
        let store = root.store();
        let a = create(&store, "Existing");
        let incomplete = root.0.join("profiles").join(new_id());
        fs::create_dir(&incomplete).unwrap();
        fs::write(incomplete.join("config.toml"), "").unwrap();
        assert_eq!(store.list().unwrap(), [a.clone()]);
        assert!(store.create("Bad Avatar", Allegiance::Arm, Some(b"bad"), &Config::default()).is_err());
        assert_eq!(store.list().unwrap(), [a.clone()]);
        assert_eq!(store.active().unwrap().unwrap(), a);
    }

    #[test]
    fn record_metadata_is_optional_bounded_and_validated() {
        let id = new_id();
        let mut bytes = b"legacy record body".to_vec();
        assert!(record_metadata(&bytes).is_none());
        append_record_metadata(&mut bytes, &id, true).unwrap();
        assert_eq!(record_metadata(&bytes), Some((id.as_str(), true)));
        let len = bytes.len();
        for i in [len - 48, len - 40, len - 8, len - 1] {
            let mut bad = bytes.clone();
            bad[i] = 255;
            assert!(record_metadata(&bad).is_none());
        }
        let before = bytes.clone();
        assert!(append_record_metadata(&mut bytes, "../bad", false).is_err());
        assert_eq!(bytes, before);
    }

    #[cfg(unix)]
    #[test]
    fn linked_personal_paths_are_rejected() {
        use std::os::unix::fs::symlink;
        let root = TestRoot::new();
        let outside = TestRoot::new();
        let store = root.store();
        let p = create(&store, "Player");
        symlink(&outside.0, store.data_dir(&p.id).unwrap().join("portrait.png")).unwrap();
        assert!(store.avatar_path(&p.id).is_err());
        let fresh = TestRoot::new();
        symlink(&outside.0, fresh.0.join("saves")).unwrap();
        assert!(fresh.store().create("Linked", Allegiance::Arm, None, &Config::default()).is_err());
        assert!(fresh.store().list().unwrap().is_empty());
    }

    fn battle_context(mode: MatchMode) -> MatchContext {
        MatchContext { mode, map: "Coast To Coast".into(), faction: "ARM".into(), difficulty: "hard".into(), ..Default::default() }
    }

    /// A battle with cheats in it gives back all it gave: XP, totals, records, achievements and
    /// its place in the history; it gives nothing more, even from a save made before the cheat;
    /// a battle played honestly before it keeps what it earned.
    #[test]
    fn a_match_with_cheats_gives_back_all_it_gave_and_gives_nothing_more() {
        let root = TestRoot::new();
        let store = root.store();
        let p = create(&store, "Player");
        let context = battle_context(MatchMode::Skirmish);
        let honest = MatchCounters { played_seconds: 600, kills: 3, units_built: 5, finished: true, ..Default::default() };
        store.award_match(&p.id, "honest", &honest, &context).unwrap();
        let before = store.career(&p.id).unwrap();
        let history = store.history(&p.id).unwrap();
        let mut c = MatchCounters { played_seconds: 300, kills: 120, units_built: 40, buildings_built: 10, metal_gathered: 50_000, ..Default::default() };
        c.tallies.insert("kills.air".into(), 4);
        c.unit_builds.insert("ARMPW".into(), 30);
        c.unit_names.insert("ARMPW".into(), "Peewee".into());
        let first = store.award_match(&p.id, "cheated", &c, &context).unwrap();
        (c.finished, c.won, c.played_seconds) = (true, true, 400);
        let second = store.award_match(&p.id, "cheated", &c, &context).unwrap();
        let earned: Vec<String> = first.achievements.iter().chain(&second.achievements).cloned().collect();
        assert!(first.xp + second.xp > 0 && !earned.is_empty(), "the battle earned nothing to take back");
        assert_ne!(store.career(&p.id).unwrap(), before);
        store.save_achievement_picture(&p.id, &earned[..1], &[0xff, 0xd8, 0xff, 0xd9]).unwrap();
        let voided = store.void_match(&p.id, "cheated").unwrap();
        assert!(!store.achievement_pictures(&p.id).unwrap().contains(&earned[0]), "the picture of a moment that no longer counts goes");
        assert_eq!(voided.xp, first.xp + second.xp);
        assert_eq!(voided.achievements.len(), earned.len());
        assert_eq!(store.career(&p.id).unwrap(), before);
        assert_eq!(store.history(&p.id).unwrap(), history);
        c.kills += 50;
        assert_eq!(store.award_match(&p.id, "cheated", &c, &context).unwrap().xp, 0);
        assert_eq!(store.career(&p.id).unwrap(), before);
        assert_eq!(store.void_match(&p.id, "cheated").unwrap(), Voided { total_xp: before.total_xp(), ..Voided::default() });
        // (Voided before it earned anything: it never will.)
        store.void_match(&p.id, "early").unwrap();
        assert_eq!(store.award_match(&p.id, "early", &honest, &context).unwrap().xp, 0);
        assert_eq!(store.career(&p.id).unwrap(), before);
    }

    /// Rank rewards must not sustain themselves after their source battle is taken back.
    #[test]
    fn a_voided_rank_badge_cannot_keep_itself_earned_with_its_own_xp() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Honest");
        let context = battle_context(MatchMode::Skirmish);
        store.award_match(&p.id, "honest", &MatchCounters { kills: 15, finished: true, ..Default::default() }, &context).unwrap();
        let before = store.career(&p.id).unwrap();
        assert_eq!(before.total_xp(), 400);
        let award = store.award_match(&p.id, "cheated", &MatchCounters { units_built: 1, ..Default::default() }, &context).unwrap();
        assert!(award.achievements.iter().any(|id| id == "rank_2"));
        let voided = store.void_match(&p.id, "cheated").unwrap();
        assert!(voided.achievements.iter().any(|id| id == "rank_2"));
        assert_eq!(store.career(&p.id).unwrap(), before);
    }

    /// A record the voided battle set stands where another battle has beaten it since.
    #[test]
    fn voiding_a_match_keeps_the_records_others_have_beaten_since() {
        let root = TestRoot::new();
        let store = root.store();
        let p = create(&store, "Player");
        let context = battle_context(MatchMode::Skirmish);
        store.award_match(&p.id, "cheated", &MatchCounters { played_seconds: 120, kills: 10, units_built: 30, ..Default::default() }, &context).unwrap();
        store.award_match(&p.id, "later", &MatchCounters { played_seconds: 120, kills: 25, units_built: 3, ..Default::default() }, &context).unwrap();
        store.void_match(&p.id, "cheated").unwrap();
        let career = store.career(&p.id).unwrap();
        assert_eq!((career.feats.get("battle_kills"), career.feats.get("battle_built")), (Some(&25), Some(&3)));
        assert_eq!((career.kills, career.units_built, career.played_seconds), (25, 3, 120));
    }

    /// A campaign mission the voided battle completed first is struck off; one completed
    /// before it stays.
    #[test]
    fn voiding_a_match_strikes_off_the_missions_it_completed_first() {
        let root = TestRoot::new();
        let store = root.store();
        let p = create(&store, "Player");
        let mission = |index| MatchContext { mode: MatchMode::Campaign, mission: Some(CampaignMission { campaign: "arm".into(), index }), ..battle_context(MatchMode::Campaign) };
        let won = MatchCounters { played_seconds: 600, finished: true, won: true, ..Default::default() };
        store.award_match(&p.id, "first", &won, &mission(1)).unwrap();
        store.award_match(&p.id, "cheated", &won, &mission(2)).unwrap();
        assert_eq!(store.career(&p.id).unwrap().campaign_completed.get("arm"), Some(&vec![1, 2]));
        store.void_match(&p.id, "cheated").unwrap();
        let career = store.career(&p.id).unwrap();
        for list in [&career.campaign_completed, &career.hard_completed, &career.campaign_arm_completed] {
            assert_eq!(list.get("arm"), Some(&vec![1]));
        }
    }

    #[test]
    fn history_modes_favourites_and_unlocks_are_atomic_and_idempotent() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Career");
        let context = battle_context(MatchMode::Skirmish);
        let counters = MatchCounters { played_seconds: 400, kills: 250, losses: 3, units_built: 500, buildings_built: 50,
            units_captured: 10, unit_builds: [("ARMPEEP".into(), 12)].into(), unit_names: [("ARMPEEP".into(), "Peeper".into())].into(),
            finished: true, won: true, ..Default::default() };
        let first = store.award_match(&p.id, "match", &counters, &context).unwrap();
        assert!(first.achievements.iter().any(|id| id == "skirmish_wins_1"));
        assert!(first.achievements.iter().any(|id| id == "battle_kills"));
        let career = store.career(&p.id).unwrap();
        assert_eq!(career.favourite_faction(), Some(("ARM", 400)));
        assert_eq!(career.favourite_unit(), Some(("Peeper", 12)));
        assert_eq!(career.modes[&MatchMode::Skirmish].hard_wins, 1);
        assert!(!career.achievements.contains_key("flawless_victory"));
        let history = store.history(&p.id).unwrap();
        assert_eq!(history.len(), 1); assert_eq!(history[0].counters, counters);
        assert_eq!(history[0].earned_xp, first.xp);
        // A resumed or forged alternate setup cannot reclassify the same battle.
        let again = store.award_match(&p.id, "match", &counters, &battle_context(MatchMode::Multiplayer)).unwrap();
        assert_eq!(again.xp, 0); assert!(again.achievements.is_empty());
        assert_eq!(store.career(&p.id).unwrap(), career);
        assert_eq!(store.history(&p.id).unwrap(), history);
    }

    #[test]
    fn imported_history_only_counts_new_play_and_never_awards_full_battle_feats() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Import");
        let before = MatchCounters { played_seconds: 3600, kills: 500, units_built: 600,
            unit_builds: [("ARMPEEP".into(), 600)].into(), ..Default::default() };
        store.baseline(&p.id, "import", &before).unwrap();
        let context = battle_context(MatchMode::Skirmish);
        let zero = store.award_match(&p.id, "import", &before, &context).unwrap();
        assert_eq!(zero.xp, 0); assert!(zero.achievements.is_empty());
        assert!(store.history(&p.id).unwrap().is_empty());
        let after = MatchCounters { played_seconds: 3630, kills: 501, units_built: 601,
            unit_builds: [("ARMPEEP".into(), 601)].into(), finished: true, won: true, ..before };
        store.award_match(&p.id, "import", &after, &context).unwrap();
        let career = store.career(&p.id).unwrap();
        assert_eq!((career.kills, career.units_built, career.played_seconds), (1, 1, 30));
        assert_eq!(career.unit_usage["ARMPEEP"], 1);
        assert!(career.feats.is_empty(), "an imported save cannot claim the original player's feats");
        let history = store.history(&p.id).unwrap();
        assert_eq!((history[0].counters.kills, history[0].counters.played_seconds), (1, 30));
        assert_eq!(career.modes[&MatchMode::Skirmish].wins, 1);
    }

    #[test]
    fn old_ledgers_keep_totals_and_baseline_new_per_unit_fields() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Legacy");
        let path = store.data_dir(&p.id).unwrap().join(CAREER_FILE);
        fs::write(&path, "version = 1\n[career]\nxp = 60\nkills = 2\nunits_built = 2\n[matches.legacy]\nawarded_xp = 60\n[matches.legacy.high_water]\nkills = 2\nunits_built = 2\n").unwrap();
        let before = MatchCounters { kills: 2, units_built: 2, losses: 1, unit_builds: [("ARMPW".into(), 2)].into(), ..Default::default() };
        let context = battle_context(MatchMode::Skirmish);
        store.award_match(&p.id, "legacy", &before, &context).unwrap();
        let migrated = store.career(&p.id).unwrap();
        assert!(fs::read_to_string(&path).unwrap().starts_with(&format!("version = {CAREER_VERSION}")), "older executables must not silently discard the new career fields");
        assert_eq!((migrated.xp, migrated.kills, migrated.units_built, migrated.losses), (60, 2, 2, 0));
        assert!(migrated.unit_usage.is_empty()); assert!(store.history(&p.id).unwrap().is_empty());
        let after = MatchCounters { kills: 3, units_built: 3, unit_builds: [("ARMPW".into(), 3)].into(), ..before };
        store.award_match(&p.id, "legacy", &after, &context).unwrap();
        let updated = store.career(&p.id).unwrap();
        assert_eq!((updated.xp, updated.kills, updated.units_built), (90, 3, 3));
        assert_eq!(updated.unit_usage["ARMPW"], 1);
        let history = store.history(&p.id).unwrap();
        assert_eq!((history[0].earned_xp, history[0].counters.kills), (30, 1));
    }

    #[test]
    fn campaign_unique_missions_and_hard_wins_do_not_count_retries_twice() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Campaign");
        let context = MatchContext { mission: Some(CampaignMission { campaign: "arm campaign".into(), index: 0 }), ..battle_context(MatchMode::Campaign) };
        let win = MatchCounters { played_seconds: 301, finished: true, won: true, ..Default::default() };
        for id in ["first", "replay-as-new"] { store.award_match(&p.id, id, &win, &context).unwrap(); }
        let career = store.career(&p.id).unwrap();
        assert_eq!(career.campaign_completed["arm campaign"], [0]);
        assert_eq!(career.hard_completed["arm campaign"], [0]);
        assert_eq!(career.campaign_arm_completed["arm campaign"], [0]);
        assert_eq!(career.modes[&MatchMode::Campaign].wins, 2);
        assert!(career.achievements.contains_key("campaign_missions_1"));
        assert!(!career.achievements.contains_key("campaign_missions_10"));
        assert!(!career.achievements.keys().any(|id| id.starts_with("skirmish_")));
        assert!(!career.achievements.keys().any(|id| id.starts_with("multiplayer_")));
    }

    #[test]
    fn finished_import_never_creates_a_new_campaign_win_or_history() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Finished");
        let counters = MatchCounters { kills: 100, finished: true, won: true, ..Default::default() };
        store.baseline(&p.id, "finished", &counters).unwrap();
        store.award_match(&p.id, "finished", &counters, &battle_context(MatchMode::Campaign)).unwrap();
        assert_eq!(store.career(&p.id).unwrap(), Career::default());
        assert!(store.history(&p.id).unwrap().is_empty());
    }

    #[test]
    fn malformed_extended_ledger_is_preserved_and_text_input_is_bounded() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Bounds");
        let counters = MatchCounters { unit_builds: [("bad\nkey".into(), 10)].into(), unit_names: [("bad\nkey".into(), "Bad".into())].into(), ..Default::default() };
        store.award_match(&p.id, "bad-text", &counters, &MatchContext { map: "x\n".repeat(200), ..battle_context(MatchMode::Skirmish) }).unwrap();
        assert!(store.career(&p.id).is_ok(), "bounded data must remain readable after writing");
        let many = MatchCounters { unit_builds: (0..1100).map(|n| (format!("unit-{n:04}"), 1)).collect(),
            unit_names: (0..1100).map(|n| (format!("unit-{n:04}"), "Unit".to_string())).collect(), ..Default::default() };
        store.award_match(&p.id, "bounded", &many, &battle_context(MatchMode::Skirmish)).unwrap();
        assert_eq!(store.career(&p.id).unwrap().unit_usage.len(), 1024);
        let later = MatchCounters { unit_builds: [("zz-new-unit".into(), 1)].into(), unit_names: [("zz-new-unit".into(), "New Unit".into())].into(), ..Default::default() };
        store.award_match(&p.id, "bounded", &later, &battle_context(MatchMode::Skirmish)).unwrap();
        assert!(store.career(&p.id).is_ok(), "an overflow unit label must not make the bounded ledger unreadable");
        let path = store.data_dir(&p.id).unwrap().join(CAREER_FILE);
        let text = "version = 1\n[career.achievements]\nunrecognised = 2\n[matches]\n";
        fs::write(&path, text).unwrap();
        assert!(store.award(&p.id, "another", &MatchCounters::default()).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), text);
    }

    #[test]
    fn cooperative_history_is_multiplayer_but_pvp_achievements_require_human_opposition() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Network");
        let counters = MatchCounters { played_seconds: 3600, kills: 1000, units_built: 100, finished: true, won: true, ..Default::default() };
        let coop = battle_context(MatchMode::Multiplayer);
        store.award_match(&p.id, "coop", &counters, &coop).unwrap();
        let career = store.career(&p.id).unwrap();
        assert_eq!(career.modes[&MatchMode::Multiplayer].wins, 1);
        assert_eq!(career.competitive_multiplayer, ModeStats::default());
        assert!(!career.achievements.keys().any(|id| id.starts_with("multiplayer_")));
        assert_eq!(store.history(&p.id).unwrap()[0].context.mode, MatchMode::Multiplayer);
        let pvp = MatchContext { human_opponent: true, ..coop };
        let award = store.award_match(&p.id, "pvp", &counters, &pvp).unwrap();
        assert!(award.achievements.iter().any(|id| id == "multiplayer_wins_1"));
        let career = store.career(&p.id).unwrap();
        assert_eq!(career.modes[&MatchMode::Multiplayer].wins, 2);
        assert_eq!(career.competitive_multiplayer.wins, 1);
        assert_eq!(career.competitive_multiplayer.kills, 1000);
    }

    #[test]
    fn sanitized_campaign_ids_cannot_poison_the_next_ledger_read() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Campaign names");
        let win = MatchCounters { finished: true, won: true, ..Default::default() };
        for (id, campaign, index, expected) in [
            ("controls", "\r\n\t", 0, None),
            ("empty", "", 0, None),
            ("index", "arm campaign", 100_000, None),
            ("valid", "ARM\n Campaign", 0, Some("arm campaign")),
        ] {
            let context = MatchContext { mission: Some(CampaignMission { campaign: campaign.into(), index }), ..battle_context(MatchMode::Campaign) };
            store.award_match(&p.id, id, &win, &context).unwrap();
            let history = store.history(&p.id).expect("a successful award must leave a readable ledger");
            let observed = history.iter().find(|r| r.id == id).unwrap().context.mission.as_ref();
            assert_eq!(observed.map(|m| m.campaign.as_str()), expected);
        }
        let career = store.career(&p.id).unwrap();
        assert_eq!(career.campaign_completed, [("arm campaign".into(), vec![0])].into());
        assert_eq!(career.wins, 4, "missing mission identity does not discard genuine completed play");
    }

    #[test]
    fn xp_uses_bounded_totals_without_scoring_per_unit_detail() {
        let mut counters = MatchCounters { played_seconds: 60, kills: 1, units_built: 5, buildings_built: 99,
            units_captured: 1, metal_gathered: 199, energy_gathered: 1999, finished: true, won: true,
            ..Default::default() };
        assert_eq!(counters.xp(), 438, "buildings are capped by all constructions; resource fractions stay cumulative");
        counters.unit_builds = (0..1024).map(|n| (format!("unit-{n:04}"), u64::MAX)).collect();
        counters.unit_names = counters.unit_builds.keys().map(|id| (id.clone(), "Display name".repeat(30))).collect();
        assert_eq!(counters.xp(), 438, "detail is informational and cannot increase XP");
        counters.finished = false;
        assert_eq!(counters.xp(), 138, "an unfinished win cannot award either completion bonus");
        counters.played_seconds = u64::MAX; counters.kills = u64::MAX;
        counters.units_built = u64::MAX; counters.buildings_built = u64::MAX;
        counters.units_captured = u64::MAX; counters.metal_gathered = u64::MAX; counters.energy_gathered = u64::MAX;
        assert_eq!(counters.xp(), 56_383_249_999_998);
        counters.finished = true;
        assert_eq!(counters.xp(), 56_383_250_000_298);
    }

    #[test]
    fn a_ledger_from_before_the_new_list_keeps_its_kept_achievements_and_drops_retired_ones() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Veteran");
        let path = store.data_dir(&p.id).unwrap().join(CAREER_FILE);
        fs::write(&path, "version = 2\n[career]\nxp = 900\nkills = 1200\nunits_built = 3\n[career.achievements]\ncareer_kills_1 = 100\ncareer_kills_100 = 200\nrank_5 = 300\nskirmish_wins_5 = 400\n[matches]\n").unwrap();
        let career = store.career(&p.id).unwrap();
        assert_eq!(career.achievements.keys().map(String::as_str).collect::<Vec<_>>(), ["career_kills_1", "career_kills_100"]);
        assert_eq!(career.achievements["career_kills_1"], 100, "a kept achievement keeps its date");
        // 1,200 kills earned the new list's 1,000 already: it comes from the service record.
        assert_eq!(store.refresh_achievements(&p.id).unwrap(), ["career_kills_1000", "career_built_1", "rank_2"]);
        let career = store.career(&p.id).unwrap();
        assert!(career.unlocks["career_kills_1000"].earlier);
        assert!(fs::read_to_string(&path).unwrap().starts_with(&format!("version = {CAREER_VERSION}")));
        assert!(store.refresh_achievements(&p.id).unwrap().is_empty(), "nothing is unlocked twice");
    }

    #[test]
    fn tallies_peaks_and_battle_records_unlock_during_the_battle() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Records");
        let context = MatchContext { enemies: 3, world: "Lush".into(), ..battle_context(MatchMode::Skirmish) };
        let mut c = MatchCounters { played_seconds: 600, kills: 99, tallies: [("kills.commander".into(), 1), ("kills.air".into(), 40)].into(),
            peaks: [("metal_income".into(), 51)].into(), ..Default::default() };
        let first = store.award_match(&p.id, "battle", &c, &context).unwrap();
        for id in ["commander_kills_1", "metal_income_50"] { assert!(first.achievements.iter().any(|a| a == id), "{id}: {:?}", first.achievements); }
        assert!(first.earlier.is_empty(), "earned in this battle");
        assert!(!first.achievements.iter().any(|a| a == "battle_kills_100"));
        c.kills = 100;
        let second = store.award_match(&p.id, "battle", &c, &context).unwrap();
        assert_eq!(second.achievements, ["career_kills_100", "battle_kills_100"], "a battle's record counts the moment it is reached");
        let career = store.career(&p.id).unwrap();
        assert_eq!((career.tally("kills.air"), career.feats["battle_metal_income"]), (40, 51));
        let unlock = &career.unlocks["battle_kills_100"];
        assert_eq!((unlock.map.as_str(), unlock.mode, unlock.battle_seconds, unlock.earlier), ("Coast To Coast", MatchMode::Skirmish, 600, false));
        assert!(career.map_wins.is_empty(), "nothing is won yet");
        c.finished = true; c.won = true;
        let won = store.award_match(&p.id, "battle", &c, &context).unwrap();
        for id in ["skirmish_wins_1", "outnumbered", "against_all_odds", "fast_skirmish", "lightning_war"] {
            assert!(won.achievements.iter().any(|a| a == id), "{id}: {:?}", won.achievements);
        }
        let career = store.career(&p.id).unwrap();
        assert_eq!((career.map_wins["Coast To Coast"], career.world_wins["Lush"]), (1, 1));
        // With an ally it is not a stand alone; a campaign win is no map of the skirmish list.
        let allied = MatchContext { allies: 1, ..context.clone() };
        store.award_match(&p.id, "allied", &MatchCounters { played_seconds: 900, finished: true, won: true, ..Default::default() }, &allied).unwrap();
        store.award_match(&p.id, "mission", &MatchCounters { finished: true, won: true, ..Default::default() }, &MatchContext { map: "Another".into(), ..battle_context(MatchMode::Campaign) }).unwrap();
        let career = store.career(&p.id).unwrap();
        assert_eq!(career.feats["outnumbered"], 1);
        assert_eq!(career.map_wins.len(), 1);
    }

    #[test]
    fn a_preview_of_the_next_checkpoint_unlocks_what_the_checkpoint_will() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Preview");
        let context = battle_context(MatchMode::Skirmish);
        let saved = MatchCounters { played_seconds: 120, kills: 0, ..Default::default() };
        store.award_match(&p.id, "live", &saved, &context).unwrap();
        let career = store.career(&p.id).unwrap();
        let live = MatchCounters { played_seconds: 150, kills: 1, tallies: [("transported.commander".into(), 1)].into(), ..Default::default() };
        let preview = career.with_progress(&saved, &live, &context, true);
        let pending: Vec<_> = preview.pending().iter().map(|a| a.id).collect();
        assert_eq!(pending, ["career_kills_1", "transport_commander"]);
        assert!(career.pending().is_empty());
        let award = store.award_match(&p.id, "live", &live, &context).unwrap();
        assert_eq!(award.achievements, pending);
        // An imported battle's records are not the player's: no preview counts them either.
        let records = MatchCounters { kills: 120, ..live.clone() };
        assert!(!career.with_progress(&saved, &records, &context, false).pending().iter().any(|a| a.id == "battle_kills_100"));
        assert!(career.with_progress(&saved, &records, &context, true).pending().iter().any(|a| a.id == "battle_kills_100"));
    }

    #[test]
    fn an_achievements_picture_is_kept_once_and_only_for_known_achievements() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Pictures");
        let jpeg = |n: u8| vec![0xff, 0xd8, 0xff, n];
        store.save_achievement_picture(&p.id, &["career_kills_1".into(), "rank_2".into()], &jpeg(1)).unwrap();
        store.save_achievement_picture(&p.id, &["career_kills_1".into()], &jpeg(2)).unwrap();
        assert_eq!(fs::read(store.achievement_picture_path(&p.id, "career_kills_1").unwrap()).unwrap(), jpeg(1), "the first moment stands");
        assert_eq!(store.achievement_pictures(&p.id).unwrap(), ["career_kills_1".to_string(), "rank_2".to_string()].into());
        assert!(store.achievement_picture_path(&p.id, "../portrait").is_err());
        assert!(store.save_achievement_picture(&p.id, &["no_such_thing".into()], &jpeg(3)).is_err());
        assert!(store.save_achievement_picture(&p.id, &["rank_10".into()], b"not a jpeg").is_err());
        let other = create(&store, "Other");
        assert!(store.achievement_pictures(&other.id).unwrap().is_empty());
    }

    #[test]
    fn motto_and_showcase_are_kept_and_checked() {
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Player");
        assert_eq!(store.set_motto(&p.id, "  Victory through industry  ").unwrap().motto, "Victory through industry");
        assert!(store.set_motto(&p.id, &"x".repeat(61)).is_err());
        assert!(store.set_motto(&p.id, "bad\nline").is_err());
        let showcase = vec!["career_kills_1".to_string(), "rank_2".to_string()];
        assert_eq!(store.set_showcase(&p.id, &showcase).unwrap().showcase, showcase);
        assert!(store.set_showcase(&p.id, &["no_such_thing".to_string()]).is_err());
        assert!(store.set_showcase(&p.id, &["rank_2".to_string(), "rank_2".to_string()]).is_err());
        assert!(store.set_showcase(&p.id, &vec!["rank_2".to_string(); 6]).is_err());
        let reread = root.store().get(&p.id).unwrap();
        assert_eq!((reread.motto.as_str(), reread.showcase.len()), ("Victory through industry", 2));
        // A profile written before these existed still reads.
        let path = store.data_dir(&p.id).unwrap().join(PROFILE_FILE);
        fs::write(&path, format!("id = \"{}\"\nname = \"Player\"\nallegiance = \"ARM\"\nhas_avatar = false\ncreated_at = 5\n", p.id)).unwrap();
        assert_eq!(store.get(&p.id).unwrap().motto, "");
        // So does one whose motto or showcase this version cannot show (a newer version's
        // achievement, a hand edit): those are left out.
        let showcase = r#"["from_the_future", "rank_2", "rank_2", "career_kills_1", "rank_10", "rank_25", "rank_50", "rank_75"]"#;
        fs::write(&path, format!("id = \"{}\"\nname = \"Player\"\nallegiance = \"ARM\"\nhas_avatar = false\ncreated_at = 5\nmotto = \"{}\"\nshowcase = {showcase}\n",
            p.id, "x".repeat(80))).unwrap();
        let reread = store.get(&p.id).unwrap();
        assert_eq!(reread.motto, "");
        assert_eq!(reread.showcase, ["rank_2", "career_kills_1", "rank_10", "rank_25", "rank_50"]);
    }

    #[test]
    fn a_released_profile_is_no_longer_chosen_and_others_stay() {
        let root = TestRoot::new(); let store = root.store();
        let a = create(&store, "First");
        let b = create(&store, "Second");
        assert_eq!(store.active().unwrap().unwrap().id, b.id);
        let dir = store.release(&b.id).unwrap();
        assert!(dir.ends_with(&b.id) && dir.exists(), "nothing is deleted here");
        assert!(store.active().unwrap().is_none());
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(store.list().unwrap(), std::slice::from_ref(&a));
        store.select(&a.id).unwrap();
        store.release(&a.id).unwrap();
        assert!(store.active().unwrap().is_none());
        assert!(store.release("../../outside").is_err());
    }

    #[test]
    fn career_snapshot_keeps_totals_and_history_from_the_same_read() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let root = TestRoot::new(); let store = root.store(); let p = create(&store, "Snapshot");
        let writer_done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for kills in 1..=12 {
                    store.award_match(&p.id, "live", &MatchCounters { kills, ..Default::default() }, &battle_context(MatchMode::Skirmish)).unwrap();
                }
                writer_done.store(true, Ordering::Release);
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                assert!(std::time::Instant::now() < deadline, "career writer did not finish");
                let (career, history) = store.career_with_history(&p.id).unwrap();
                assert_eq!(career.kills, history.iter().map(|r| r.counters.kills).sum());
                assert_eq!(career.xp, history.iter().map(|r| r.earned_xp).sum());
                if writer_done.load(Ordering::Acquire) { break; }
                std::thread::yield_now();
            }
        });
        let (career, history) = store.career_with_history(&p.id).unwrap();
        assert_eq!(career.kills, 12);
        assert_eq!(history, store.history(&p.id).unwrap());
        assert_eq!(career, store.career(&p.id).unwrap());
    }

}
