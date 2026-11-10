//! Local, deterministic achievement conditions evaluated from eligible career deltas.
//! Stable IDs are persisted; names and descriptions are translated by the shared UI.
//!
//! Each achievement's badge is a cell of `crates/ta_ui/badges/achievements.png`, in the order
//! of [`ALL`]: `python tools/profiles/badges.py` draws them from this file (its `badge` names
//! the glyph, its tier the metal), so it is run again after the list changes.
use crate::profiles::{Career, MatchMode, ModeStats};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category { Campaign, Skirmish, Multiplayer, Battle, Industry, Service }

impl Category {
    pub const ALL: [Category; 6] = [Self::Campaign, Self::Skirmish, Self::Multiplayer, Self::Battle, Self::Industry, Self::Service];
}

/// How hard an achievement is to earn: the badge's metal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier { Bronze, Silver, Gold, Platinum }

impl Tier {
    pub const ALL: [Tier; 4] = [Self::Bronze, Self::Silver, Self::Gold, Self::Platinum];

    /// The XP an achievement of this tier grants when it is earned.
    pub const fn reward(self) -> u64 {
        match self { Self::Bronze => 100, Self::Silver => 250, Self::Gold => 500, Self::Platinum => 1000 }
    }
}

#[derive(Clone, Copy, Debug)]
enum Counter { Finished, Wins, Kills, Built, Buildings, Captures, Seconds, Metal, Energy, HardWins, ArmWins, CoreWins, Rank }

#[derive(Clone, Copy, Debug)]
enum Metric {
    /// A career total.
    Total(Counter),
    /// A total of one mode's battles (multiplayer: those against people).
    Mode(MatchMode, Counter),
    /// Different campaign missions won, of every campaign (`hard`: on Hard).
    Missions { hard: bool },
    /// Different missions won as ARM and as CORE: the fewer of the two.
    BothSides,
    /// Different missions won of one campaign (its id), or of the campaigns whose ids begin so.
    Campaign(&'static str),
    HardCampaign(&'static str),
    Campaigns(&'static str),
    /// The best of a single battle (`Career::feats`).
    Feat(&'static str),
    /// An event tally of every battle (`Career::tallies`), or of one mode's.
    Tally(&'static str),
    ModeTally(MatchMode, &'static str),
    /// Units of these types built.
    Types(&'static [&'static str]),
    /// Different kinds of unit built.
    Kinds,
    /// Different maps, and different worlds, won on in skirmish and multiplayer battles.
    Maps,
    Worlds,
    /// Battles won as ARM and as CORE, in any mode: the fewer of the two.
    BothFactions,
}

#[derive(Clone, Copy, Debug)]
pub struct Achievement {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub category: Category,
    pub tier: Tier,
    /// The glyph `tools/profiles/badges.py` draws on its badge.
    pub badge: &'static str,
    pub target: u64,
    /// Kept secret until earned: the list shows that there is one, not what it asks.
    pub hidden: bool,
    metric: Metric,
}

impl Achievement {
    pub fn is_unlocked(&self, career: &Career) -> bool { career.achievements.contains_key(self.id) }

    /// Its badge's cell in the atlas: its place in [`ALL`].
    pub fn index(&self) -> usize { ALL.iter().position(|a| a.id == self.id).unwrap_or(0) }

    pub fn progress(&self, career: &Career) -> u64 {
        let missions = |list: &std::collections::BTreeMap<String, Vec<u32>>| list.values().map(|v| v.len() as u64).sum::<u64>();
        let campaign = |list: &std::collections::BTreeMap<String, Vec<u32>>, id: &str| list.get(id).map_or(0, |v| v.len() as u64);
        let distinct = |wins: &std::collections::BTreeMap<String, u64>| {
            let mut names: Vec<String> = wins.iter().filter(|(_, n)| **n > 0).map(|(k, _)| k.to_lowercase()).collect();
            names.sort();
            names.dedup();
            names.len() as u64
        };
        let wins = |counter: Counter| career.modes.values().map(|s| counter.mode(s)).sum::<u64>();
        match self.metric {
            Metric::Feat(key) => career.feats.get(key).copied().unwrap_or(0),
            Metric::Missions { hard: false } => missions(&career.campaign_completed),
            Metric::Missions { hard: true } => missions(&career.hard_completed),
            Metric::BothSides => missions(&career.campaign_arm_completed).min(missions(&career.campaign_core_completed)),
            Metric::Campaign(id) => campaign(&career.campaign_completed, id),
            Metric::HardCampaign(id) => campaign(&career.hard_completed, id),
            Metric::Campaigns(prefix) => career.campaign_completed.iter().filter(|(k, _)| k.starts_with(prefix)).map(|(_, v)| v.len() as u64).sum(),
            Metric::Tally(key) => career.tallies.get(key).copied().unwrap_or(0),
            Metric::ModeTally(MatchMode::Multiplayer, key) => career.competitive_multiplayer.tallies.get(key).copied().unwrap_or(0),
            Metric::ModeTally(mode, key) => career.modes.get(&mode).and_then(|s| s.tallies.get(key)).copied().unwrap_or(0),
            Metric::Types(types) => types.iter().map(|t| career.unit_usage.get(*t).copied().unwrap_or(0)).sum(),
            Metric::Kinds => career.unit_usage.values().filter(|n| **n > 0).count() as u64,
            Metric::Maps => distinct(&career.map_wins),
            Metric::Worlds => distinct(&career.world_wins),
            Metric::BothFactions => wins(Counter::ArmWins).min(wins(Counter::CoreWins)),
            Metric::Mode(MatchMode::Multiplayer, counter) => counter.mode(&career.competitive_multiplayer),
            Metric::Mode(mode, counter) => career.modes.get(&mode).map_or(0, |s| counter.mode(s)),
            Metric::Total(counter) => match counter {
                Counter::Rank => career.rank().level as u64,
                Counter::Finished => career.matches_finished, Counter::Wins => career.wins,
                Counter::Kills => career.kills, Counter::Built => career.units_built,
                Counter::Buildings => career.buildings_built, Counter::Captures => career.units_captured,
                Counter::Seconds => career.played_seconds, Counter::Metal => career.metal_gathered,
                Counter::Energy => career.energy_gathered,
                Counter::HardWins | Counter::ArmWins | Counter::CoreWins => wins(counter),
            },
        }.min(self.target)
    }

    /// Whether its target is a time, in seconds (shown in hours).
    pub fn counts_seconds(&self) -> bool { matches!(self.metric, Metric::Total(Counter::Seconds) | Metric::Mode(_, Counter::Seconds)) }

    /// Earned by the career but not yet recorded as unlocked.
    pub fn is_met(&self, career: &Career) -> bool { !self.is_unlocked(career) && self.progress(career) >= self.target }
}

impl Counter {
    fn mode(self, s: &ModeStats) -> u64 {
        match self {
            Self::Finished => s.matches_finished, Self::Wins => s.wins, Self::Kills => s.kills,
            Self::Built => s.units_built, Self::Buildings => s.buildings_built, Self::Captures => s.units_captured,
            Self::Seconds => s.played_seconds, Self::Metal => s.metal_gathered, Self::Energy => s.energy_gathered,
            Self::HardWins => s.hard_wins, Self::ArmWins => s.arm_wins, Self::CoreWins => s.core_wins,
            Self::Rank => 0,
        }
    }
}

pub fn find(id: &str) -> Option<&'static Achievement> { ALL.iter().find(|a| a.id == id) }

/// Achievements of the previous list that this one dropped: a ledger written before it loses them
/// (an unknown identifier is otherwise a damaged ledger). Those kept kept their identifiers.
pub(crate) const RETIRED: &[&str] = &[
    "campaign_missions_3", "campaign_missions_5", "campaign_missions_15", "campaign_missions_20", "campaign_missions_30",
    "campaign_missions_40", "campaign_arm_1", "campaign_arm_5", "campaign_arm_10", "campaign_arm_15", "campaign_arm_20",
    "campaign_arm_25", "campaign_core_1", "campaign_core_5", "campaign_core_10", "campaign_core_15", "campaign_core_20",
    "campaign_core_25", "campaign_hard_5", "campaign_hard_20", "campaign_hard_30", "campaign_construction",
    "skirmish_wins_5", "skirmish_wins_25", "skirmish_finished_1", "skirmish_finished_10", "skirmish_finished_50",
    "skirmish_finished_100", "skirmish_kills_100", "skirmish_kills_500", "skirmish_kills_1000", "skirmish_kills_5000",
    "skirmish_built_100", "skirmish_built_500", "skirmish_built_1000", "skirmish_buildings_50", "skirmish_buildings_250",
    "skirmish_buildings_1000", "skirmish_captures_1", "skirmish_captures_10", "skirmish_captures_50", "skirmish_hours_1",
    "skirmish_hours_5", "multiplayer_finished_10", "multiplayer_finished_50", "multiplayer_kills_100", "multiplayer_built",
    "rank_5", "rank_20", "rank_30", "rank_40", "rank_60",
];

const SILOS: &[&str] = &["ARMSILO", "CORSILO", "ARMEMP", "CORTRON"];
const FUSION: &[&str] = &["ARMFUS", "CORFUS", "ARMCKFUS", "CORCKFUS", "ARMUWFUS", "CORUWFUS"];

use Category as C;
use Counter::*;
use Metric::*;
use Tier as T;

#[allow(clippy::too_many_arguments)]
const fn a(id: &'static str, name: &'static str, description: &'static str, category: Category, tier: Tier, badge: &'static str, target: u64, metric: Metric) -> Achievement {
    Achievement { id, name, description, category, tier, badge, target, hidden: false, metric }
}
#[allow(clippy::too_many_arguments)]
const fn hidden(id: &'static str, name: &'static str, description: &'static str, category: Category, tier: Tier, badge: &'static str, target: u64, metric: Metric) -> Achievement {
    Achievement { id, name, description, category, tier, badge, target, hidden: true, metric }
}

// This catalogue is also read directly by ta-tools i18n (names and descriptions), by
// tools/profiles/badges.py (the badges) and by tools/profiles/export_achievements.py (the
// reference): one achievement a line, in this form.
pub const ALL: &[Achievement] = &[
    a("campaign_missions_1", "First Orders", "Win a campaign mission.", C::Campaign, T::Bronze, "flag", 1, Missions { hard: false }),
    a("campaign_missions_10", "Into the War", "Win 10 different campaign missions.", C::Campaign, T::Bronze, "flag", 10, Missions { hard: false }),
    a("campaign_missions_25", "Campaign Trail", "Win 25 different campaign missions.", C::Campaign, T::Silver, "flag", 25, Missions { hard: false }),
    a("campaign_missions_50", "The Long March", "Win 50 different campaign missions.", C::Campaign, T::Gold, "scroll", 50, Missions { hard: false }),
    a("campaign_missions_100", "War Chronicle", "Win 100 different campaign missions.", C::Campaign, T::Platinum, "scroll", 100, Missions { hard: false }),
    a("arm_campaign", "Liberation", "Win every mission of the Arm campaign.", C::Campaign, T::Gold, "arm", 25, Campaign("arm campaign")),
    a("core_campaign", "Total Integration", "Win every mission of the Core campaign.", C::Campaign, T::Gold, "core", 25, Campaign("core campaign")),
    a("arm_contingency", "Contingency Plan", "Win every Arm mission of Core Contingency.", C::Campaign, T::Gold, "arm", 12, Campaign("arm campaign - core contingency")),
    a("core_contingency", "Second Front", "Win every Core mission of Core Contingency.", C::Campaign, T::Gold, "core", 12, Campaign("core campaign - core contingency")),
    a("krogoth_encounter", "Close Encounter", "Win the Krogoth Encounter.", C::Campaign, T::Silver, "giant", 1, Campaign("krogoth encounter")),
    a("battle_tactics_10", "Tactician", "Win 10 different Battle Tactics missions.", C::Campaign, T::Bronze, "knight", 10, Campaigns("battle tactics")),
    a("battle_tactics_50", "Field Manual", "Win 50 different Battle Tactics missions.", C::Campaign, T::Silver, "knight", 50, Campaigns("battle tactics")),
    a("battle_tactics_100", "Battle Tested", "Win all 100 Battle Tactics missions.", C::Campaign, T::Gold, "knight", 100, Campaigns("battle tactics")),
    a("campaign_hard_1", "Hard Orders", "Win a campaign mission on Hard.", C::Campaign, T::Bronze, "flame", 1, Missions { hard: true }),
    a("campaign_hard_10", "Tested in Fire", "Win 10 different campaign missions on Hard.", C::Campaign, T::Silver, "flame", 10, Missions { hard: true }),
    a("campaign_hard_25", "No Easy Victories", "Win 25 different campaign missions on Hard.", C::Campaign, T::Gold, "flame", 25, Missions { hard: true }),
    a("arm_campaign_hard", "Unbreakable", "Win every mission of the Arm campaign on Hard.", C::Campaign, T::Platinum, "arm", 25, HardCampaign("arm campaign")),
    a("core_campaign_hard", "Inexorable", "Win every mission of the Core campaign on Hard.", C::Campaign, T::Platinum, "core", 25, HardCampaign("core campaign")),
    a("campaign_both", "Both Sides of the War", "Win campaign missions as both ARM and CORE.", C::Campaign, T::Bronze, "sides", 1, BothSides),
    a("campaign_victories", "Return to the Front", "Win 50 campaign battles. Replaying a mission counts.", C::Campaign, T::Silver, "flag", 50, Mode(MatchMode::Campaign, Wins)),
    a("campaign_kills", "Story of Steel", "Destroy 1,000 enemy units in campaign missions.", C::Campaign, T::Silver, "crosshair", 1000, Mode(MatchMode::Campaign, Kills)),
    a("campaign_captures", "Spoils of War", "Capture 10 enemy units in campaign missions.", C::Campaign, T::Bronze, "magnet", 10, Mode(MatchMode::Campaign, Captures)),

    a("skirmish_wins_1", "Skirmish Victor", "Win a skirmish.", C::Skirmish, T::Bronze, "swords", 1, Mode(MatchMode::Skirmish, Wins)),
    a("skirmish_wins_10", "Field Commander", "Win 10 skirmishes.", C::Skirmish, T::Bronze, "swords", 10, Mode(MatchMode::Skirmish, Wins)),
    a("skirmish_wins_50", "Battlefield Authority", "Win 50 skirmishes.", C::Skirmish, T::Silver, "trophy", 50, Mode(MatchMode::Skirmish, Wins)),
    a("skirmish_wins_100", "Skirmish Legend", "Win 100 skirmishes.", C::Skirmish, T::Gold, "trophy", 100, Mode(MatchMode::Skirmish, Wins)),
    a("skirmish_wins_500", "Five Hundred Fields", "Win 500 skirmishes.", C::Skirmish, T::Platinum, "trophy", 500, Mode(MatchMode::Skirmish, Wins)),
    a("skirmish_hard_1", "Hard-Fought Victory", "Win a skirmish with every enemy AI on Hard.", C::Skirmish, T::Silver, "flame", 1, Mode(MatchMode::Skirmish, HardWins)),
    a("skirmish_hard_10", "Master of the Field", "Win 10 skirmishes with every enemy AI on Hard.", C::Skirmish, T::Gold, "flame", 10, Mode(MatchMode::Skirmish, HardWins)),
    a("skirmish_hard_50", "Grand Master", "Win 50 skirmishes with every enemy AI on Hard.", C::Skirmish, T::Platinum, "flame", 50, Mode(MatchMode::Skirmish, HardWins)),
    a("skirmish_arm", "ARM Field Victory", "Win a skirmish as ARM.", C::Skirmish, T::Bronze, "arm", 1, Mode(MatchMode::Skirmish, ArmWins)),
    a("skirmish_core", "CORE Field Victory", "Win a skirmish as CORE.", C::Skirmish, T::Bronze, "core", 1, Mode(MatchMode::Skirmish, CoreWins)),
    a("outnumbered", "Outnumbered", "Win a skirmish alone against three or more enemies.", C::Skirmish, T::Silver, "odds", 1, Feat("outnumbered")),
    a("against_all_odds", "Against All Odds", "Win a skirmish alone against three or more Hard AIs.", C::Skirmish, T::Gold, "odds", 1, Feat("hard_odds")),
    a("last_one_standing", "Last One Standing", "Win a skirmish alone against seven enemies.", C::Skirmish, T::Platinum, "crown", 1, Feat("ffa_seven")),
    a("fast_skirmish", "Rapid Assault", "Win a skirmish in under ten minutes.", C::Skirmish, T::Silver, "stopwatch", 1, Feat("fast_skirmish")),
    a("lightning_war", "Lightning War", "Win a skirmish against Hard AIs in under fifteen minutes.", C::Skirmish, T::Gold, "stopwatch", 1, Feat("fast_hard")),
    a("maps_5", "World Traveller", "Win skirmish or multiplayer battles on 5 different maps.", C::Skirmish, T::Bronze, "globe", 5, Maps),
    a("maps_25", "Cartographer", "Win skirmish or multiplayer battles on 25 different maps.", C::Skirmish, T::Silver, "globe", 25, Maps),
    a("maps_100", "Seen It All", "Win skirmish or multiplayer battles on 100 different maps.", C::Skirmish, T::Gold, "globe", 100, Maps),
    a("worlds_5", "Planet Hopper", "Win skirmish or multiplayer battles on 5 different worlds.", C::Skirmish, T::Silver, "planet", 5, Worlds),
    a("worlds_10", "Across the Galaxy", "Win skirmish or multiplayer battles on 10 different worlds.", C::Skirmish, T::Gold, "planet", 10, Worlds),
    a("skirmish_hours_10", "Battlefield Regular", "Play 10 hours of skirmishes.", C::Skirmish, T::Silver, "hourglass", 36_000, Mode(MatchMode::Skirmish, Seconds)),

    a("multiplayer_finished_1", "Connected Commander", "Finish a multiplayer battle against another person.", C::Multiplayer, T::Bronze, "link", 1, Mode(MatchMode::Multiplayer, Finished)),
    a("multiplayer_finished_25", "Regular Opponent", "Finish 25 multiplayer battles against other people.", C::Multiplayer, T::Silver, "link", 25, Mode(MatchMode::Multiplayer, Finished)),
    a("multiplayer_finished_100", "Multiplayer Veteran", "Finish 100 multiplayer battles against other people.", C::Multiplayer, T::Gold, "link", 100, Mode(MatchMode::Multiplayer, Finished)),
    a("multiplayer_wins_1", "Human Opposition", "Win a multiplayer battle against another person.", C::Multiplayer, T::Bronze, "swords", 1, Mode(MatchMode::Multiplayer, Wins)),
    a("multiplayer_wins_10", "Contested Ground", "Win 10 multiplayer battles against other people.", C::Multiplayer, T::Silver, "swords", 10, Mode(MatchMode::Multiplayer, Wins)),
    a("multiplayer_wins_50", "Multiplayer Champion", "Win 50 multiplayer battles against other people.", C::Multiplayer, T::Gold, "trophy", 50, Mode(MatchMode::Multiplayer, Wins)),
    a("multiplayer_wins_250", "Hall of Fame", "Win 250 multiplayer battles against other people.", C::Multiplayer, T::Platinum, "trophy", 250, Mode(MatchMode::Multiplayer, Wins)),
    a("multiplayer_commander", "Decapitation", "Destroy another person's Commander in a multiplayer battle.", C::Multiplayer, T::Silver, "target", 1, ModeTally(MatchMode::Multiplayer, "kills.commander")),
    a("multiplayer_kills_1000", "Online Offensive", "Destroy 1,000 enemy units in multiplayer battles against other people.", C::Multiplayer, T::Silver, "crosshair", 1000, Mode(MatchMode::Multiplayer, Kills)),
    a("coop_win", "Comrades in Arms", "Win a multiplayer battle with another person on your team.", C::Multiplayer, T::Bronze, "team", 1, Tally("wins.coop")),
    a("multiplayer_hour", "Time in Company", "Play an hour of multiplayer battles against other people.", C::Multiplayer, T::Bronze, "hourglass", 3600, Mode(MatchMode::Multiplayer, Seconds)),
    a("multiplayer_hours_25", "Old Hand", "Play 25 hours of multiplayer battles against other people.", C::Multiplayer, T::Gold, "hourglass", 90_000, Mode(MatchMode::Multiplayer, Seconds)),

    a("career_kills_1", "First Strike", "Destroy an enemy unit.", C::Battle, T::Bronze, "crosshair", 1, Total(Kills)),
    a("career_kills_100", "Combat Record", "Destroy 100 enemy units.", C::Battle, T::Bronze, "crosshair", 100, Total(Kills)),
    a("career_kills_1000", "Destroyer", "Destroy 1,000 enemy units.", C::Battle, T::Silver, "crosshair", 1000, Total(Kills)),
    a("career_kills_10000", "Warmonger", "Destroy 10,000 enemy units.", C::Battle, T::Gold, "crosshair", 10_000, Total(Kills)),
    a("career_kills_100000", "Scourge of the Galaxy", "Destroy 100,000 enemy units.", C::Battle, T::Platinum, "skull", 100_000, Total(Kills)),
    a("battle_kills_100", "Centurion", "Destroy 100 enemy units in one battle.", C::Battle, T::Silver, "sword", 100, Feat("battle_kills")),
    a("battle_kills", "Annihilation Force", "Destroy 250 enemy units in one battle.", C::Battle, T::Gold, "sword", 250, Feat("battle_kills")),
    a("battle_kills_1000", "Utter Annihilation", "Destroy 1,000 enemy units in one battle.", C::Battle, T::Platinum, "skull", 1000, Feat("battle_kills")),
    a("commander_kills_1", "Decapitation Strike", "Destroy an enemy Commander.", C::Battle, T::Bronze, "target", 1, Tally("kills.commander")),
    a("commander_kills_25", "Commander Hunter", "Destroy 25 enemy Commanders.", C::Battle, T::Silver, "target", 25, Tally("kills.commander")),
    a("commander_kills_100", "Kingslayer", "Destroy 100 enemy Commanders.", C::Battle, T::Gold, "target", 100, Tally("kills.commander")),
    hidden("commander_duel", "Duel of Commanders", "Destroy an enemy Commander with your own Commander.", C::Battle, T::Gold, "duel", 1, Tally("commander.commanders")),
    a("army_of_one", "Army of One", "Destroy 50 enemy units with your Commander in one battle.", C::Battle, T::Gold, "helmet", 50, Feat("battle_commander_kills")),
    a("air_kills_100", "Flak Curtain", "Shoot down 100 enemy aircraft.", C::Battle, T::Silver, "plane", 100, Tally("kills.air")),
    a("air_kills_1000", "Clear Skies", "Shoot down 1,000 enemy aircraft.", C::Battle, T::Gold, "plane", 1000, Tally("kills.air")),
    a("naval_kills_100", "Depth Charge", "Sink 100 enemy ships.", C::Battle, T::Silver, "anchor", 100, Tally("kills.naval")),
    a("naval_kills_1000", "Ruler of the Waves", "Sink 1,000 enemy ships.", C::Battle, T::Gold, "anchor", 1000, Tally("kills.naval")),
    a("building_kills_500", "Demolition Crew", "Destroy 500 enemy buildings.", C::Battle, T::Silver, "rubble", 500, Tally("kills.building")),
    a("building_kills_5000", "Scorched Earth", "Destroy 5,000 enemy buildings.", C::Battle, T::Gold, "rubble", 5000, Tally("kills.building")),
    a("experimental_kill", "Giant Slayer", "Destroy an enemy experimental unit, such as a Krogoth.", C::Battle, T::Gold, "giant", 1, Tally("kills.experimental")),
    a("missile_1", "Launch Authorised", "Launch a strategic missile from a silo.", C::Battle, T::Silver, "nuke", 1, Tally("launched.missile")),
    a("missile_25", "Mutually Assured Destruction", "Launch 25 strategic missiles.", C::Battle, T::Gold, "nuke", 25, Tally("launched.missile")),
    a("career_captures_1", "Change of Allegiance", "Capture an enemy unit.", C::Battle, T::Bronze, "magnet", 1, Total(Captures)),
    a("career_captures_25", "Acquisition Specialist", "Capture 25 enemy units.", C::Battle, T::Silver, "magnet", 25, Total(Captures)),
    a("battle_captures", "Salvage Commander", "Capture 10 enemy units in one battle.", C::Battle, T::Gold, "magnet", 10, Feat("battle_captures")),
    a("flawless_victory", "Unbroken Line", "Win a battle lasting at least five minutes without losing a unit.", C::Battle, T::Gold, "shield", 1, Feat("flawless_victory")),
    a("endurance_victory", "Endurance Victory", "Win a battle lasting at least an hour.", C::Battle, T::Silver, "hourglass", 1, Feat("endurance_victory")),

    a("career_built_1", "Nanoframe Complete", "Build a unit or a building.", C::Industry, T::Bronze, "wrench", 1, Total(Built)),
    a("career_built_100", "Production Line", "Build 100 units or buildings.", C::Industry, T::Bronze, "wrench", 100, Total(Built)),
    a("career_built_1000", "Mass Production", "Build 1,000 units or buildings.", C::Industry, T::Silver, "gear", 1000, Total(Built)),
    a("career_built_10000", "War Machine", "Build 10,000 units or buildings.", C::Industry, T::Gold, "gear", 10_000, Total(Built)),
    a("career_built_100000", "Endless Assembly", "Build 100,000 units or buildings.", C::Industry, T::Platinum, "gear", 100_000, Total(Built)),
    a("career_buildings_1", "Foundations", "Build a building.", C::Industry, T::Bronze, "factory", 1, Total(Buildings)),
    a("career_buildings_100", "Base Architect", "Build 100 buildings.", C::Industry, T::Bronze, "factory", 100, Total(Buildings)),
    a("career_buildings_1000", "Urban Planner", "Build 1,000 buildings.", C::Industry, T::Silver, "factory", 1000, Total(Buildings)),
    a("battle_built", "Army in a Battle", "Build 500 units or buildings in one battle.", C::Industry, T::Gold, "crates", 500, Feat("battle_built")),
    a("unit_kinds_25", "Arsenal", "Build 25 different kinds of unit.", C::Industry, T::Bronze, "blueprint", 25, Kinds),
    a("unit_kinds_75", "Full Arsenal", "Build 75 different kinds of unit.", C::Industry, T::Silver, "blueprint", 75, Kinds),
    a("unit_kinds_150", "Complete Arsenal", "Build 150 different kinds of unit.", C::Industry, T::Gold, "blueprint", 150, Kinds),
    a("build_experimental", "Titan Forge", "Build an experimental unit, such as a Krogoth.", C::Industry, T::Gold, "giant", 1, Tally("built.experimental")),
    a("build_silo", "Strategic Deterrent", "Build a missile silo.", C::Industry, T::Silver, "silo", 1, Types(SILOS)),
    a("build_heavy_artillery", "Long Reach", "Build a long-range plasma cannon.", C::Industry, T::Silver, "cannon", 1, Types(&["ARMBRTHA", "CORINT"])),
    a("build_rapid_artillery", "Rain of Fire", "Build a rapid-fire plasma cannon.", C::Industry, T::Gold, "gatling", 1, Types(&["ARMVULC", "CORBUZZ"])),
    a("build_fusion", "Fusion Power", "Build a fusion reactor.", C::Industry, T::Bronze, "atom", 1, Types(FUSION)),
    a("build_moho", "Deep Core Mining", "Build a moho mine.", C::Industry, T::Bronze, "drill", 1, Types(&["ARMMOHO", "CORMOHO"])),
    a("solar_100", "Solar Farm", "Build 100 solar collectors.", C::Industry, T::Bronze, "solar", 100, Types(&["ARMSOLAR", "CORSOLAR"])),
    a("air_built_100", "Air Wing", "Build 100 aircraft.", C::Industry, T::Silver, "plane", 100, Tally("built.air")),
    a("naval_built_100", "Admiralty", "Build 100 ships.", C::Industry, T::Silver, "anchor", 100, Tally("built.naval")),
    a("transported_100", "Ferryman", "Carry 100 units in transports.", C::Industry, T::Bronze, "hook", 100, Tally("transported")),
    hidden("transport_commander", "Special Delivery", "Carry your Commander in a transport.", C::Industry, T::Silver, "hook", 1, Tally("transported.commander")),
    a("reclaimed_100", "Scavenger", "Reclaim 100 wrecks or trees.", C::Industry, T::Bronze, "recycle", 100, Tally("reclaimed")),
    a("reclaimed_1000", "Recycler", "Reclaim 1,000 wrecks or trees.", C::Industry, T::Silver, "recycle", 1000, Tally("reclaimed")),
    a("career_metal_100000", "Metalworks", "Produce 100,000 metal.", C::Industry, T::Bronze, "ingot", 100_000, Total(Metal)),
    a("career_metal_1000000", "Metal Magnate", "Produce 1,000,000 metal.", C::Industry, T::Silver, "ingot", 1_000_000, Total(Metal)),
    a("career_metal_10000000", "Metal Empire", "Produce 10,000,000 metal.", C::Industry, T::Gold, "ingot", 10_000_000, Total(Metal)),
    a("career_energy_1000000", "Power Grid", "Produce 1,000,000 energy.", C::Industry, T::Bronze, "bolt", 1_000_000, Total(Energy)),
    a("career_energy_10000000", "Power Station", "Produce 10,000,000 energy.", C::Industry, T::Silver, "bolt", 10_000_000, Total(Energy)),
    a("career_energy_100000000", "Star Power", "Produce 100,000,000 energy.", C::Industry, T::Gold, "bolt", 100_000_000, Total(Energy)),
    a("metal_income_50", "Metal Rush", "Reach a metal income of 50 a second in one battle.", C::Industry, T::Silver, "graph", 50, Feat("battle_metal_income")),
    a("energy_income_2000", "Power Surge", "Reach an energy income of 2,000 a second in one battle.", C::Industry, T::Silver, "graph", 2000, Feat("battle_energy_income")),
    a("army_10000", "Battalion", "Field an army worth 10,000 metal in one battle.", C::Industry, T::Silver, "army", 10_000, Feat("battle_army")),
    a("army_50000", "Grand Army", "Field an army worth 50,000 metal in one battle.", C::Industry, T::Gold, "army", 50_000, Feat("battle_army")),

    a("rank_2", "First Promotion", "Reach rank 2.", C::Service, T::Bronze, "chevron", 2, Total(Rank)),
    a("rank_10", "Enlisted", "Reach rank 10.", C::Service, T::Bronze, "chevron", 10, Total(Rank)),
    a("rank_25", "Quarter Century", "Reach rank 25.", C::Service, T::Silver, "chevron", 25, Total(Rank)),
    a("rank_50", "Halfway to the Top", "Reach rank 50.", C::Service, T::Gold, "chevron", 50, Total(Rank)),
    a("rank_75", "Old Guard", "Reach rank 75.", C::Service, T::Gold, "chevron", 75, Total(Rank)),
    a("rank_100", "Highest Command", "Reach rank 100.", C::Service, T::Platinum, "star", 100, Total(Rank)),
    a("matches_10", "Regular", "Finish 10 battles.", C::Service, T::Bronze, "medal", 10, Total(Finished)),
    a("matches_100", "Hundred Battles", "Finish 100 battles.", C::Service, T::Silver, "medal", 100, Total(Finished)),
    a("matches_1000", "Living Legend", "Finish 1,000 battles.", C::Service, T::Platinum, "medal", 1000, Total(Finished)),
    a("career_hours_10", "Dedicated Commander", "Play for 10 hours.", C::Service, T::Bronze, "hourglass", 36_000, Total(Seconds)),
    a("career_hours_100", "Lifer", "Play for 100 hours.", C::Service, T::Gold, "hourglass", 360_000, Total(Seconds)),
    a("both_factions", "Double Agent", "Win battles as both ARM and CORE.", C::Service, T::Bronze, "sides", 1, BothFactions),
    a("tutorial", "Basic Training", "Finish a tutorial.", C::Service, T::Bronze, "book", 1, Mode(MatchMode::Tutorial, Wins)),
    a("scenario_win", "Scenario Specialist", "Win a scenario.", C::Service, T::Bronze, "tower", 1, Mode(MatchMode::Scenario, Wins)),
    a("scenario_hard", "Siege Breaker", "Win a scenario on Hard.", C::Service, T::Silver, "tower", 1, Mode(MatchMode::Scenario, HardWins)),
    a("conquest_win", "Conqueror", "Win a Conquest battle.", C::Service, T::Bronze, "conquest", 1, Mode(MatchMode::Conquest, Wins)),
    a("conquest_wins_25", "Galactic Conqueror", "Win 25 Conquest battles.", C::Service, T::Silver, "conquest", 25, Mode(MatchMode::Conquest, Wins)),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_has_unique_names_and_persistent_ids() {
        use std::collections::BTreeSet;
        assert_eq!(ALL.iter().map(|a| a.id).collect::<BTreeSet<_>>().len(), ALL.len());
        assert_eq!(ALL.iter().map(|a| a.name).collect::<BTreeSet<_>>().len(), ALL.len());
        for category in Category::ALL { assert!(ALL.iter().filter(|a| a.category == category).count() >= 10, "{category:?}"); }
        for tier in Tier::ALL { assert!(ALL.iter().any(|a| a.tier == tier), "{tier:?}"); }
        for a in ALL {
            assert!(a.target > 0 && !a.name.is_empty() && a.description.ends_with('.') && !a.badge.is_empty(), "{}", a.id);
            assert!(!RETIRED.contains(&a.id), "{} is both kept and retired", a.id);
            assert_eq!(ALL[a.index()].id, a.id);
        }
        // A category's achievements stay together: the badges' order and the list's.
        let mut seen = Vec::new();
        for a in ALL {
            if seen.last() != Some(&a.category) { assert!(!seen.contains(&a.category), "{} is out of its group", a.id); seen.push(a.category); }
        }
    }

    #[test]
    fn every_achievement_is_attainable_and_new_profiles_unlock_none() {
        let mut career = Career { xp: crate::profiles::rank_threshold(100), kills: 100_000, units_built: 100_000,
            buildings_built: 100_000, units_captured: 100_000, metal_gathered: 100_000_000, energy_gathered: 100_000_000,
            played_seconds: 100_000_000, matches_finished: 100_000, wins: 100_000, ..Default::default() };
        let busy = ModeStats { matches_finished: 100_000, wins: 100_000, kills: 100_000,
            units_built: 100_000, buildings_built: 100_000, units_captured: 100_000, played_seconds: 100_000_000,
            hard_wins: 100_000, arm_wins: 100_000, core_wins: 100_000,
            tallies: [("kills.commander".to_string(), 100)].into(), ..Default::default() };
        for mode in [MatchMode::Campaign, MatchMode::Skirmish, MatchMode::Multiplayer, MatchMode::Tutorial, MatchMode::Scenario, MatchMode::Conquest] {
            career.modes.insert(mode, busy.clone());
        }
        career.competitive_multiplayer = busy;
        for list in [&mut career.campaign_completed, &mut career.hard_completed, &mut career.campaign_arm_completed, &mut career.campaign_core_completed] {
            for campaign in ["arm campaign", "core campaign", "arm campaign - core contingency", "core campaign - core contingency", "krogoth encounter", "battle tactics - arm short"] {
                list.insert(campaign.to_string(), (0..100).collect());
            }
        }
        for a in ALL {
            match a.metric {
                Metric::Feat(key) => { career.feats.insert(key.to_string(), 100_000); }
                Metric::Tally(key) => { career.tallies.insert(key.to_string(), 100_000); }
                Metric::Types(types) => for t in types { career.unit_usage.insert(t.to_string(), 100); },
                _ => {}
            }
        }
        for n in 0..200 { career.unit_usage.insert(format!("UNIT{n}"), 1); }
        for n in 0..100 { career.map_wins.insert(format!("Map {n}"), 1); }
        for n in 0..10 { career.world_wins.insert(format!("World {n}"), 1); }
        for a in ALL {
            assert!(a.progress(&Career::default()) < a.target, "fresh profile: {}", a.id);
            assert_eq!(a.progress(&career), a.target, "unattainable: {}", a.id);
        }
    }

    #[test]
    fn maps_and_worlds_count_once_whatever_their_case() {
        let career = Career { world_wins: [("Green Planet".to_string(), 2), ("Green planet".to_string(), 1), ("Lava".to_string(), 1), ("Ice".to_string(), 0)].into(), ..Default::default() };
        assert_eq!(find("worlds_5").unwrap().progress(&career), 2);
    }
}
