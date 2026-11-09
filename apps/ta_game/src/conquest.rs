//! Conquest: a galaxy of TA's worlds taken system by system. Each system is a skirmish map;
//! lanes join neighbours, and any enemy system next to one of ours can be attacked. The
//! nearer the enemy capital, the stronger its computer players. Holding systems pays: more
//! metal and energy at the start of each battle and room for more veterans, the units that
//! earned a rank and come along with the Commander (whose own experience carries over too),
//! and each system's own [`Reward`]. Each promotion of the Commander earns a [`Perk`]. Only
//! systems near ours are charted. After a victory the
//! enemy may strike back at a system of ours ([`Counter`]): beaten off in a battle there, or
//! broken up by taking the system it comes from, else it falls. Taking the capital wins. The
//! state is saved after every battle in the settings folder (`conquest.toml`); a battle itself
//! saves like any other game, the conquest data riding in its `GameSetup` (see
//! [`crate::setup::ConquestStart`]).

use serde::{Deserialize, Serialize};
use std::io::{self, Read};
use std::path::Path;
use ta_sim::{Fx, Locomotion};

use crate::game::Game;
use crate::setup::{ConquestStart, Controller, GameSetup, MapEntry, SlotSetup};

/// Bumped when the saved state's layout changes (unsupported saves are reported and preserved).
/// Format 1 (before rewards and counter-attacks) is read and brought up to date.
pub(crate) const FORMAT: u32 = 2;
/// Veterans kept in reserve at most.
const MAX_ROSTER: usize = 24;
/// Veteran berths at most.
const MAX_BERTHS: usize = 12;
/// Units the systems' foundries and airfields send to a battle, at most.
const MAX_REINFORCEMENTS: usize = 4;
/// What each mine and reactor adds to the supply at the start of a battle.
pub const MINES_METAL: u64 = 300;
pub const REACTORS_ENERGY: u64 = 600;
/// Battles the player has to answer a counter-attack before it lands.
pub const COUNTER_DUE: u32 = 2;
const MAX_HISTORY: usize = 128;
/// The journal's latest battles keep who came back and who fell (the rest their numbers).
const DETAILED: usize = 12;
const MAX_SYSTEMS: usize = 24;
const MAX_SAVE_BYTES: u64 = 1 << 20;

/// How hard the enemy fights back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Level {
    Relaxed,
    #[default]
    Standard,
    Hard,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Relaxed, Level::Standard, Level::Hard];

    pub fn label(self) -> &'static str {
        match self {
            Level::Relaxed => ta_ui::i18n::n("EASY"),
            Level::Standard => ta_ui::i18n::n("NORMAL"),
            Level::Hard => ta_ui::i18n::n("HARD"),
        }
    }
}

/// How many systems the galaxy has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Size {
    Small,
    #[default]
    Medium,
    Large,
}

impl Size {
    pub const ALL: [Size; 3] = [Size::Small, Size::Medium, Size::Large];

    pub fn label(self) -> &'static str {
        match self {
            Size::Small => ta_ui::i18n::n("SMALL"),
            Size::Medium => ta_ui::i18n::n("MEDIUM"),
            Size::Large => ta_ui::i18n::n("LARGE"),
        }
    }

    pub fn systems(self) -> usize {
        match self {
            Size::Small => 10,
            Size::Medium => 16,
            Size::Large => 24,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Owner {
    Player,
    Enemy,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct System {
    /// The map's title, and its file stem.
    pub name: String,
    pub map: String,
    pub planet: String,
    /// Where it sits on the galaxy map (0..1 across, 0..1 down).
    pub pos: [f32; 2],
    pub owner: Owner,
    /// Enemy strength: 1 near our home to 3 near their capital.
    pub threat: u8,
}

/// What holding a system brings besides its share of the supply: one for every system but home
/// and the enemy capital.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reward {
    #[default]
    None,
    /// Metal mines: more metal at the start of each battle.
    Mines,
    /// Fusion reactors: more energy at the start of each battle.
    Reactors,
    /// A tank plant: a tank joins each battle.
    Foundry,
    /// An airfield: a gunship joins each battle.
    Airfield,
    /// A war college: the Commander gains experience after each victory.
    Academy,
    /// A sensor array: systems two jumps beyond ours are charted.
    Sensors,
    /// A supply depot: one more veteran berth.
    Depot,
}

impl Reward {
    pub const ALL: [Reward; 7] = [Reward::Mines, Reward::Reactors, Reward::Foundry, Reward::Airfield, Reward::Academy, Reward::Sensors, Reward::Depot];

    pub fn label(self) -> &'static str {
        match self {
            Reward::None => "",
            Reward::Mines => ta_ui::i18n::n("MINES"),
            Reward::Reactors => ta_ui::i18n::n("REACTORS"),
            Reward::Foundry => ta_ui::i18n::n("TANK PLANT"),
            Reward::Airfield => ta_ui::i18n::n("AIRFIELD"),
            Reward::Academy => ta_ui::i18n::n("WAR COLLEGE"),
            Reward::Sensors => ta_ui::i18n::n("SENSOR ARRAY"),
            Reward::Depot => ta_ui::i18n::n("SUPPLY DEPOT"),
        }
    }
}

/// A Commander's perk: one chosen at each of its promotions (of three offered), kept to the war's
/// end. All but the quartermaster's are the battle's (`ta_sim::Perks`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Perk {
    /// It builds and repairs half as fast again.
    Engineer,
    /// It takes 30% less damage.
    Armour,
    /// Its weapons hit a quarter harder.
    Marksman,
    /// Our units gain experience from their kills half as fast again.
    Instructor,
    /// Its D-gun costs half its energy.
    Capacitors,
    /// A quarter more supply at the start of each battle.
    Quartermaster,
}

impl Perk {
    pub const ALL: [Perk; 6] = [Perk::Engineer, Perk::Armour, Perk::Marksman, Perk::Instructor, Perk::Capacitors, Perk::Quartermaster];

    /// The perk a battle's setup names.
    pub fn from_id(id: &str) -> Option<Perk> {
        Perk::ALL.iter().copied().find(|p| p.id() == id)
    }

    /// Its name in a battle's setup.
    pub fn id(self) -> &'static str {
        match self {
            Perk::Engineer => "engineer",
            Perk::Armour => "armour",
            Perk::Marksman => "marksman",
            Perk::Instructor => "instructor",
            Perk::Capacitors => "capacitors",
            Perk::Quartermaster => "quartermaster",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Perk::Engineer => ta_ui::i18n::n("COMBAT ENGINEER"),
            Perk::Armour => ta_ui::i18n::n("REINFORCED ARMOUR"),
            Perk::Marksman => ta_ui::i18n::n("MARKSMAN"),
            Perk::Instructor => ta_ui::i18n::n("FIELD INSTRUCTOR"),
            Perk::Capacitors => ta_ui::i18n::n("CAPACITORS"),
            Perk::Quartermaster => ta_ui::i18n::n("QUARTERMASTER"),
        }
    }

    /// What it does, in words.
    pub fn effect(self) -> &'static str {
        match self {
            Perk::Engineer => ta_ui::i18n::n("The Commander builds and repairs half as fast again"),
            Perk::Armour => ta_ui::i18n::n("The Commander takes 30% less damage"),
            Perk::Marksman => ta_ui::i18n::n("The Commander's weapons hit a quarter harder"),
            Perk::Instructor => ta_ui::i18n::n("Your units gain experience from their kills half as fast again"),
            Perk::Capacitors => ta_ui::i18n::n("The Commander's D-gun costs half its energy"),
            Perk::Quartermaster => ta_ui::i18n::n("A quarter more supply at the start of each battle"),
        }
    }
}

/// The perks a battle's setup names, as the simulation has them (names it does not know, the
/// quartermaster's among them, are passed over).
pub fn battle_perks(names: &[String]) -> ta_sim::Perks {
    let mut perks = ta_sim::Perks::default();
    for name in names {
        match name.as_str() {
            "engineer" => perks.engineer = true,
            "armour" => perks.armour = true,
            "marksman" => perks.marksman = true,
            "instructor" => perks.instructor = true,
            "capacitors" => perks.capacitors = true,
            _ => {}
        }
    }
    perks
}

/// The enemy's counter-attack on a system of ours, from one of theirs beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counter {
    pub system: usize,
    pub from: usize,
    /// The battle whose end it lands at, unless beaten off or broken up before.
    pub due: u32,
}

/// A unit that earned a rank, waiting for the next battle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Veteran {
    pub unit: String,
    pub xp: Fx,
}

impl Veteran {
    /// 1 veteran, 2 hardened, 3 elite (as the simulation ranks them).
    pub fn rank(&self) -> u8 {
        let v = self.xp / (self.xp + Fx::from_int(3));
        if v >= Fx::from_ratio(2, 3) {
            3
        } else if v >= Fx::from_ratio(1, 2) {
            2
        } else if v >= Fx::from_ratio(1, 4) {
            1
        } else {
            0
        }
    }
}

/// What happened in the last battle (shown when the galaxy map opens again).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub system: usize,
    pub won: bool,
    /// Veterans home from the battle (new ones included), and lost.
    pub returned: usize,
    pub lost: usize,
    pub new_veterans: usize,
    /// Who they were, and the Commander's experience (battles recorded before these were kept,
    /// and the journal's older ones, have none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<ReportDetails>,
    /// Fought at a system of ours to beat off a counter-attack (else an attack).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub defence: bool,
    /// What else the war came to at the battle's end: a system of ours that fell to a
    /// counter-attack left unanswered, one the enemy began a counter-attack on, and one whose
    /// counter-attack this battle broke up (by taking the system it came from).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fell: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threat: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broken: Option<usize>,
}

/// Who came back from a battle and who did not, and the Commander's experience before it and
/// after.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportDetails {
    /// Veterans home from the battle; those new to the roster are `promoted` (among them).
    pub returned: Vec<Veteran>,
    pub promoted: Vec<Veteran>,
    pub fallen: Vec<Veteran>,
    pub commander: [Fx; 2],
    /// Veteran berths before the battle and after it (none kept before these were).
    #[serde(default)]
    pub berths: [usize; 2],
}

/// The recent campaign journal. Older saves begin their journal at the next battle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CampaignBattle {
    pub battle: u32,
    pub report: Report,
    pub duration_ticks: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Conquest {
    pub format: u32,
    pub side: String,
    pub enemy: String,
    pub level: Level,
    pub seed: u64,
    pub systems: Vec<System>,
    pub lanes: Vec<(usize, usize)>,
    pub home: usize,
    pub capital: usize,
    pub commander_xp: Fx,
    pub roster: Vec<Veteran>,
    pub battles: u32,
    pub wins: u32,
    pub last: Option<Report>,
    #[serde(default)]
    pub history: Vec<CampaignBattle>,
    /// None automatically picks the strongest veterans; an empty selection sends only
    /// the Commander. Indices refer to the current roster and reset after each battle.
    #[serde(default)]
    pub deployment: Option<Vec<usize>>,
    /// The enemy capital has fallen.
    pub won: bool,
    /// What each system brings ([`Reward`]), and the enemy's counter-attack under way.
    #[serde(default)]
    pub rewards: Vec<Reward>,
    #[serde(default)]
    pub counter: Option<Counter>,
    /// Systems of ours that fell to counter-attacks, and counter-attacks beaten off in battle.
    #[serde(default)]
    pub fallen: u32,
    #[serde(default)]
    pub defended: u32,
    /// The Commander's perks, one chosen at each promotion, and the highest rank it has reached
    /// (the promotions it has earned one for).
    #[serde(default)]
    pub perks: Vec<Perk>,
    #[serde(default)]
    pub best_rank: u8,
}

/// A battle's outcome for the conquest (see [`result_of`]).
#[derive(Clone)]
pub struct BattleResult {
    pub seed: u64,
    pub battle: u32,
    pub system: usize,
    pub won: bool,
    /// Ranked units of ours still standing.
    pub survivors: Vec<Veteran>,
    /// The actual starting roster, captured in GameSetup rather than a later menu choice.
    pub selected: Vec<Veteran>,
    /// Selected veterans that could not be placed on this map remain in reserve.
    pub undeployed: Vec<Veteran>,
    pub commander_xp: Option<Fx>,
    pub duration_ticks: u64,
}

/// SplitMix64: galaxies are drawn from their seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

impl Conquest {
    /// A new galaxy of `size` systems from the skirmish maps, ours on the left, the enemy
    /// capital on the right.
    pub fn new(maps: &[MapEntry], side: &str, level: Level, size: Size, seed: u64) -> Option<Conquest> {
        let mut rng = Rng(seed ^ 0xC0DE_5EED);
        // Maps for two or more; the capital needs room for two enemies.
        let mut pool: Vec<&MapEntry> = maps.iter().filter(|m| m.start_positions >= 2).collect();
        if pool.len() < 4 {
            return None;
        }
        let n = size.systems().min(pool.len());
        // Reserve the capital's map before another system can consume the only
        // map with enough starts for both defenders.
        let roomy: Vec<usize> = (0..pool.len()).filter(|&i| pool[i].start_positions >= 3).collect();
        let capital_map = (!roomy.is_empty()).then(|| pool.remove(roomy[rng.below(roomy.len())]));
        // A jittered grid, wider than tall.
        let cols = ((n as f32 * 1.8).sqrt().ceil() as usize).max(3);
        let rows = n.div_ceil(cols).max(2);
        let mut cells: Vec<(usize, usize)> = (0..rows).flat_map(|r| (0..cols).map(move |c| (c, r))).collect();
        // Keep the middle row's ends (home and capital); drop cells at random from the rest.
        let mid = rows / 2;
        while cells.len() > n {
            let k = rng.below(cells.len());
            let (c, r) = cells[k];
            if r == mid && (c == 0 || c == cols - 1) {
                continue;
            }
            cells.remove(k);
        }
        let mut systems = Vec::with_capacity(n);
        for &(c, r) in &cells {
            let jitter = |rng: &mut Rng| (rng.unit() - 0.5) * 0.5;
            let x = (c as f32 + 0.5 + jitter(&mut rng)) / cols as f32;
            let y = (r as f32 + 0.5 + jitter(&mut rng)) / rows as f32;
            let home_or_capital = r == mid && (c == 0 || c == cols - 1);
            // The capital gets a map with room for two enemies when there is one.
            let wants_three = r == mid && c == cols - 1;
            let m = if wants_three && capital_map.is_some() { capital_map.unwrap() }
                else { pool.remove(rng.below(pool.len())) };
            systems.push(System {
                name: m.title.clone(),
                map: m.name.clone(),
                planet: m.planet.clone(),
                pos: if home_or_capital { [x, (mid as f32 + 0.5) / rows as f32] } else { [x.clamp(0.04, 0.96), y.clamp(0.06, 0.94)] },
                owner: Owner::Enemy,
                threat: 1,
            });
        }
        let home = cells.iter().position(|&(c, r)| c == 0 && r == mid).unwrap_or(0);
        let capital = cells.iter().position(|&(c, r)| c == cols - 1 && r == mid).unwrap_or(n - 1);
        systems[home].owner = Owner::Player;
        let lanes = lanes(&systems);
        let mut conquest = Conquest {
            format: FORMAT,
            side: side.to_string(),
            enemy: if side.eq_ignore_ascii_case("CORE") { "ARM".into() } else { "CORE".into() },
            level,
            seed,
            systems,
            lanes,
            home,
            capital,
            commander_xp: Fx::ZERO,
            roster: Vec::new(),
            battles: 0,
            wins: 0,
            last: None,
            history: Vec::new(),
            deployment: None,
            won: false,
            rewards: rewards_for(seed, n, home, capital),
            counter: None,
            fallen: 0,
            defended: 0,
            perks: Vec::new(),
            best_rank: 0,
        };
        // Threat grows with the number of jumps from home.
        let jumps = conquest.jumps_from(home);
        let far = jumps.iter().copied().filter(|&j| j != usize::MAX).max().unwrap_or(1).max(1);
        for (i, s) in conquest.systems.iter_mut().enumerate() {
            let j = jumps[i].min(far);
            s.threat = if i == capital { 3 } else { (1 + j * 3 / (far + 1)).clamp(1, 3) as u8 };
        }
        Some(conquest)
    }

    fn neighbours(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        self.lanes.iter().filter_map(move |&(a, b)| if a == i { Some(b) } else if b == i { Some(a) } else { None })
    }

    /// Jumps along lanes from `from` to every system.
    fn jumps_from(&self, from: usize) -> Vec<usize> {
        let mut dist = vec![usize::MAX; self.systems.len()];
        let mut queue = std::collections::VecDeque::from([from]);
        dist[from] = 0;
        while let Some(i) = queue.pop_front() {
            for j in self.neighbours(i).collect::<Vec<_>>() {
                if dist[j] == usize::MAX {
                    dist[j] = dist[i] + 1;
                    queue.push_back(j);
                }
            }
        }
        dist
    }

    /// How many jumps the enemy capital is from home.
    pub fn capital_jumps(&self) -> usize {
        self.jumps_from(self.home)[self.capital]
    }

    /// Shortest remaining route from any held system to the enemy capital.
    pub fn remaining_jumps(&self) -> usize {
        let distances = self.jumps_from(self.capital);
        self.systems.iter().enumerate().filter(|(_, s)| s.owner == Owner::Player)
            .map(|(i, _)| distances[i]).min().unwrap_or(0)
    }

    /// The shortest way from our territory to the enemy capital: the systems along it, from the
    /// held one it sets out from to the capital (none once the capital is ours).
    pub fn route_to_capital(&self) -> Vec<usize> {
        if self.won {
            return Vec::new();
        }
        let dist = self.jumps_from(self.capital);
        let Some(start) = (0..self.systems.len()).filter(|&i| self.systems[i].owner == Owner::Player && dist[i] != usize::MAX).min_by_key(|&i| (dist[i], i)) else {
            return Vec::new();
        };
        let mut route = vec![start];
        let mut at = start;
        while at != self.capital {
            let Some(next) = self.neighbours(at).filter(|&j| dist[j] != usize::MAX && dist[j] + 1 == dist[at]).min() else { break };
            route.push(next);
            at = next;
        }
        route
    }

    /// How many battles were lost at each system (among the journal's).
    pub fn defeats(&self) -> Vec<u32> {
        let mut out = vec![0; self.systems.len()];
        for entry in &self.history {
            if !entry.report.won && let Some(n) = out.get_mut(entry.report.system) {
                *n += 1;
            }
        }
        out
    }

    /// The time fought in the journal's battles, in ticks.
    pub fn journal_ticks(&self) -> u64 {
        self.history.iter().map(|entry| entry.duration_ticks).sum()
    }

    /// An enemy system next to one of ours.
    pub fn attackable(&self, i: usize) -> bool {
        !self.won && self.battles < u32::MAX && self.systems.get(i).is_some_and(|s| s.owner == Owner::Enemy)
            && self.neighbours(i).any(|j| self.systems.get(j).is_some_and(|s| s.owner == Owner::Player))
    }

    /// One of ours the enemy is striking at: a battle there beats the counter-attack off.
    pub fn defendable(&self, i: usize) -> bool {
        !self.won && self.battles < u32::MAX && self.counter.is_some_and(|c| c.system == i)
            && self.systems.get(i).is_some_and(|s| s.owner == Owner::Player)
    }

    pub fn held(&self) -> usize {
        self.systems.iter().filter(|s| s.owner == Owner::Player).count()
    }

    /// What system `i` brings.
    pub fn reward(&self, i: usize) -> Reward {
        self.rewards.get(i).copied().unwrap_or_default()
    }

    /// How many systems of ours bring `reward`.
    pub fn held_count(&self, reward: Reward) -> usize {
        self.systems.iter().zip(&self.rewards).filter(|&(s, &r)| s.owner == Owner::Player && r == reward).count()
    }

    /// Extra metal and energy at the start of a battle: the systems held beyond home, and the
    /// mines and reactors among them.
    pub fn bonus(&self) -> (u64, u64) {
        let extra = self.held().saturating_sub(1) as u64;
        let (mines, reactors) = (self.held_count(Reward::Mines) as u64, self.held_count(Reward::Reactors) as u64);
        let (metal, energy) = ((extra * 150).min(3000) + mines * MINES_METAL, (extra * 300).min(6000) + reactors * REACTORS_ENERGY);
        // (The quartermaster's quarter more.)
        if self.perks.contains(&Perk::Quartermaster) { (metal + metal / 4, energy + energy / 4) } else { (metal, energy) }
    }

    /// The Commander's rank: 0 recruit to 3 elite.
    pub fn commander_rank(&self) -> u8 {
        Veteran { unit: String::new(), xp: self.commander_xp }.rank()
    }

    /// Promotions the Commander has earned a perk for and not yet chosen it.
    pub fn perks_due(&self) -> usize {
        (self.best_rank as usize).saturating_sub(self.perks.len())
    }

    /// The perks offered for the next choice: three of those not yet taken, drawn from the
    /// seed and the choice's number (the same war offers the same).
    pub fn perk_offer(&self) -> Vec<Perk> {
        let mut left: Vec<Perk> = Perk::ALL.iter().copied().filter(|p| !self.perks.contains(p)).collect();
        let mut rng = Rng(self.seed ^ 0x9E2C_5EED_F00C_E500 ^ (self.perks.len() as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        for k in (1..left.len()).rev() {
            left.swap(k, rng.below(k + 1));
        }
        left.truncate(3);
        left.sort();
        left
    }

    /// Takes `perk`, one of those offered, for a promotion due. False when none is due or it
    /// was not offered.
    pub fn choose_perk(&mut self, perk: Perk) -> bool {
        if self.won || self.perks_due() == 0 || !self.perk_offer().contains(&perk) {
            return false;
        }
        self.perks.push(perk);
        true
    }

    /// Veterans that can come along to a battle: a berth for every two systems held beyond
    /// home, and the depots'.
    pub fn slots(&self) -> usize {
        (self.territory_berths() + self.held_count(Reward::Depot)).min(MAX_BERTHS)
    }

    fn territory_berths(&self) -> usize {
        4 + self.held().saturating_sub(1) / 2
    }

    /// Additional systems needed for the next transport berth, if this galaxy has room.
    pub fn next_slot_in(&self) -> Option<usize> {
        let next = (self.territory_berths() - 3) * 2 + 1;
        (self.slots() < MAX_BERTHS && next <= self.systems.len()).then(|| next.saturating_sub(self.held()))
    }

    /// The units our tank plants and airfields send to each battle, fresh, beside the veterans.
    pub fn reinforcements(&self) -> Vec<String> {
        self.systems.iter().zip(&self.rewards).filter(|(s, _)| s.owner == Owner::Player)
            .filter_map(|(_, &r)| reinforcement(&self.side, r)).take(MAX_REINFORCEMENTS).map(str::to_string).collect()
    }

    /// Which systems we know: ours and those beside them (two jumps out with a sensor array);
    /// the rest are uncharted until then. All of them once the war is won.
    pub fn charted(&self) -> Vec<bool> {
        let n = self.systems.len();
        if self.won {
            return vec![true; n];
        }
        let sight = if self.held_count(Reward::Sensors) > 0 { 2 } else { 1 };
        let mut dist = vec![usize::MAX; n];
        let mut queue = std::collections::VecDeque::new();
        for (i, s) in self.systems.iter().enumerate() {
            if s.owner == Owner::Player {
                dist[i] = 0;
                queue.push_back(i);
            }
        }
        while let Some(i) = queue.pop_front() {
            if dist[i] >= sight {
                continue;
            }
            for j in self.neighbours(i).collect::<Vec<_>>() {
                if dist[j] == usize::MAX {
                    dist[j] = dist[i] + 1;
                    queue.push_back(j);
                }
            }
        }
        dist.into_iter().map(|d| d <= sight).collect()
    }

    pub fn deployment_indices(&self) -> Vec<usize> {
        if let Some(selected) = &self.deployment { return selected.clone(); }
        let mut selected: Vec<_> = (0..self.roster.len()).collect();
        selected.sort_by_key(|&i| std::cmp::Reverse(self.roster[i].xp));
        selected.truncate(self.slots());
        selected
    }

    pub fn selected_veterans(&self) -> Vec<Veteran> {
        self.deployment_indices().into_iter().filter_map(|i| self.roster.get(i).cloned()).collect()
    }

    /// The enemy's computer players at system `i`: how many, and how good. At a system of ours
    /// under attack, the attackers, as strong as the system they strike from.
    pub fn enemies(&self, i: usize) -> (usize, ta_ai::Difficulty) {
        use ta_ai::Difficulty::{Easy, Hard, Normal};
        let at = self.counter.filter(|_| self.defendable(i)).map_or(i, |c| c.from);
        let threat = self.systems.get(at).map_or(1, |s| s.threat);
        let difficulty = match (self.level, threat) {
            (Level::Relaxed, 3) | (Level::Standard, 2) | (Level::Hard, 1) => Normal,
            (Level::Relaxed, _) | (Level::Standard, 1) => Easy,
            _ => Hard,
        };
        (if at == self.capital { 2 } else { 1 }, difficulty)
    }

    /// The battle for system `i`: an attack on one of theirs, or the defence of one of ours.
    pub fn setup(&self, i: usize, maps: &[MapEntry]) -> Option<GameSetup> {
        let defence = self.defendable(i);
        if !(defence || self.attackable(i)) || self.validate().is_err() { return None; }
        let system = self.systems.get(i)?;
        let (count, difficulty) = self.enemies(i);
        let starts = maps.iter().find(|m| m.name == system.map)?.start_positions;
        if starts < 2 { return None; }
        let enemies = count.min(starts.saturating_sub(1)).max(1);
        let mut players = vec![SlotSetup { side: self.side.clone(), color: 0, controller: Controller::Human, start: 0, team: 0, name: String::new() }];
        for k in 0..enemies {
            players.push(SlotSetup { side: self.enemy.clone(), color: 1 + k as u8 * 4, controller: Controller::Ai(difficulty), start: 1 + k, team: 1, name: String::new() });
        }
        let (metal, energy) = self.bonus();
        let veterans = self.selected_veterans().into_iter().map(|v| (v.unit, v.xp)).collect();
        Some(GameSetup {
            map: system.map.clone(),
            players,
            local_player: Some(0),
            seed: self.seed ^ ((self.battles as u64 + 1) * 0x9E37_79B9),
            mission: None,
            rules: Default::default(),
            conquest: Some(ConquestStart { seed: self.seed, battle: self.battles, system: i, metal, energy, commander_xp: self.commander_xp, veterans, reinforcements: self.reinforcements(), defence, perks: self.perks.iter().map(|p| p.id().to_string()).collect() }),
            scenario: None,
        })
    }

    /// Takes a battle's outcome in. False when it belongs to another conquest or battle.
    pub fn record(&mut self, r: BattleResult) -> bool {
        let defence = self.defendable(r.system);
        if r.seed != self.seed || r.battle != self.battles || !(defence || self.attackable(r.system))
            || self.validate().is_err() || r.commander_xp.is_some_and(|xp| xp < Fx::ZERO)
            || r.survivors.iter().any(|v| !valid_veteran(v)) || r.selected.len() > self.slots()
            || r.duration_ticks >= ta_config::MAX_RECORD_TICKS {
            return false;
        }
        // A menu may change deployment while a saved battle is in progress. Its
        // casualties must still come from the battle's original roster, with duplicate
        // unit/XP pairs counted independently. Reject before mutating campaign state.
        let mut reserve = self.roster.clone();
        let mut sorted = r.selected;
        for veteran in &sorted {
            let Some(index) = reserve.iter().position(|v| v == veteran) else { return false };
            reserve.remove(index);
        }
        for veteran in r.undeployed {
            let Some(index) = sorted.iter().position(|v| *v == veteran) else { return false };
            reserve.push(sorted.remove(index));
        }
        sorted.sort_by_key(|v| std::cmp::Reverse(v.xp));
        let deployed = sorted.len();
        self.battles += 1;
        let berths_before = self.slots();
        let (returned, lost, new_veterans);
        let commander_before = self.commander_xp;
        let (mut promoted, fallen);
        let mut broken = None;
        if r.won {
            self.wins += 1;
            if defence {
                // The counter-attack beaten off.
                self.defended += 1;
                self.counter = None;
            } else {
                self.systems[r.system].owner = Owner::Player;
                self.won |= r.system == self.capital;
                // Taking the system a counter-attack comes from breaks it up (and the war's end
                // ends it).
                if let Some(c) = self.counter.filter(|c| c.from == r.system || self.won) {
                    broken = (c.from == r.system).then_some(c.system);
                    self.counter = None;
                }
            }
            // Survivors that were not in the roster before are new veterans; those that went in
            // and are not among the survivors fell.
            let mut before: Vec<&Veteran> = sorted.iter().collect();
            promoted = Vec::new();
            for s in &r.survivors {
                match before.iter().position(|v| v.unit == s.unit && v.xp <= s.xp) {
                    Some(k) => {
                        before.remove(k);
                    }
                    None => promoted.push(s.clone()),
                }
            }
            fallen = before.into_iter().cloned().collect::<Vec<_>>();
            returned = r.survivors.len();
            lost = fallen.len();
            new_veterans = promoted.len();
            self.roster = reserve.into_iter().chain(r.survivors.iter().cloned()).collect();
            if let Some(xp) = r.commander_xp {
                self.commander_xp = xp;
            }
            // The war colleges' lessons: half a point each.
            self.commander_xp += Fx::from_ratio(self.held_count(Reward::Academy) as i64, 2);
        } else {
            // A lost defence loses the system.
            if defence {
                self.systems[r.system].owner = Owner::Enemy;
                self.fallen += 1;
                self.counter = None;
            }
            // A lost battle loses the army sent in; a new Commander keeps half the old one's
            // experience.
            promoted = Vec::new();
            fallen = sorted.clone();
            returned = 0;
            lost = deployed;
            new_veterans = 0;
            self.roster = reserve;
            self.commander_xp = self.commander_xp / 2;
        }
        // A promotion earns a perk (kept, though a defeat halves the experience).
        self.best_rank = self.best_rank.max(self.commander_rank());
        // A counter-attack left unanswered lands.
        let mut fell = None;
        if let Some(c) = self.counter.filter(|c| self.battles >= c.due) {
            self.systems[c.system].owner = Owner::Enemy;
            self.fallen += 1;
            self.counter = None;
            fell = Some(c.system);
        }
        // The enemy may strike back for the system it lost.
        let threat = if r.won && !defence { self.strike_back() } else { None };
        promoted.sort_by_key(|v| std::cmp::Reverse(v.xp));
        let mut back: Vec<Veteran> = if r.won { r.survivors } else { Vec::new() };
        back.sort_by_key(|v| std::cmp::Reverse(v.xp));
        let details = ReportDetails { returned: back, promoted, fallen, commander: [commander_before, self.commander_xp], berths: [berths_before, self.slots()] };
        self.roster.sort_by_key(|v| std::cmp::Reverse(v.xp));
        self.roster.truncate(MAX_ROSTER);
        self.last = Some(Report { system: r.system, won: r.won, returned, lost, new_veterans, details: Some(details), defence, fell, threat, broken });
        self.history.push(CampaignBattle { battle: self.battles, report: self.last.as_ref().unwrap().clone(), duration_ticks: r.duration_ticks });
        if self.history.len() > MAX_HISTORY { self.history.remove(0); }
        let plain = self.history.len().saturating_sub(DETAILED);
        for entry in &mut self.history[..plain] {
            entry.report.details = None;
        }
        self.deployment = None;
        true
    }

    /// After a victory of ours the enemy may strike back at a system of ours beside one of
    /// theirs (never home; not on Relaxed, nor before the third battle), from the strongest of
    /// its neighbours: the counter-attack lands `COUNTER_DUE` battles later unless beaten off
    /// there first, or broken up by taking the system it comes from. Drawn from the seed and
    /// the battle, so a war strikes back the same way however often it is replayed. Returns
    /// the system struck.
    fn strike_back(&mut self) -> Option<usize> {
        let chance = match self.level {
            Level::Relaxed => return None,
            Level::Standard => 40,
            Level::Hard => 65,
        };
        if self.won || self.counter.is_some() || self.battles < 3 {
            return None;
        }
        let mut rng = Rng(self.seed ^ 0x57_81CE_BAC4 ^ u64::from(self.battles).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        if rng.below(100) >= chance {
            return None;
        }
        let front: Vec<usize> = (0..self.systems.len())
            .filter(|&i| i != self.home && self.systems[i].owner == Owner::Player && self.neighbours(i).any(|j| self.systems[j].owner == Owner::Enemy))
            .collect();
        let &system = front.get(rng.below(front.len()))?;
        let from = self.attacker_of(system)?;
        self.counter = Some(Counter { system, from, due: self.battles + COUNTER_DUE });
        Some(system)
    }

    /// The enemy system a counter-attack on `system` comes from: the strongest beside it.
    fn attacker_of(&self, system: usize) -> Option<usize> {
        self.neighbours(system).filter(|&j| self.systems[j].owner == Owner::Enemy).max_by_key(|&j| (self.systems[j].threat, std::cmp::Reverse(j)))
    }

    /// A conquest some battles in, for captures (`--open conquest-demo`; never saved).
    pub fn demo(maps: &[MapEntry]) -> Option<Conquest> {
        let mut c = Conquest::new(maps, "ARM", Level::Standard, Size::Medium, 0xDE30)?;
        for _ in 0..4 {
            // (Tank plants and airfields first, for their reinforcements to show.)
            let Some(i) = (0..c.systems.len()).filter(|&i| c.attackable(i)).min_by_key(|&i| (!matches!(c.reward(i), Reward::Foundry | Reward::Airfield), i)) else { break };
            let survivors = ["ARMPW", "ARMZEUS", "ARMSTUMP", "ARMSAM", "ARMBULL"]
                .iter()
                .enumerate()
                .map(|(k, u)| Veteran { unit: u.to_string(), xp: Fx::from_int(1 + (k as i64 * 3 + c.battles as i64) % 9) })
                .collect();
            let r = BattleResult { seed: c.seed, battle: c.battles, system: i, won: true, survivors, selected: c.selected_veterans(), undeployed: vec![], commander_xp: Some(Fx::from_int(4 + c.battles as i64 * 3)), duration_ticks: 30 * (840 + u64::from(c.battles) * 123) };
            c.record(r);
        }
        // (The Commander's perks chosen: the first offered each time.)
        while c.perks_due() > 0 {
            let first = c.perk_offer()[0];
            c.choose_perk(first);
        }
        // (A counter-attack under way, to show: on the last system taken, if the war's own
        // draw has not struck yet.)
        if c.counter.is_none()
            && let Some(system) = c.last.as_ref().filter(|r| r.won && !r.defence).map(|r| r.system)
            && let Some(from) = c.attacker_of(system)
        {
            c.counter = Some(Counter { system, from, due: c.battles + COUNTER_DUE });
            for report in c.last.iter_mut().chain(c.history.last_mut().map(|entry| &mut entry.report)) {
                report.threat = Some(system);
            }
        }
        Some(c)
    }

    /// The demo conquest fought on along the shortest route until the enemy capital falls
    /// (`--open conquest-won`; never saved).
    pub fn demo_won(maps: &[MapEntry]) -> Option<Conquest> {
        let mut c = Conquest::demo(maps)?;
        for _ in 0..64 {
            if c.won {
                break;
            }
            let Some(&next) = c.route_to_capital().get(1) else { break };
            let selected = c.selected_veterans();
            let survivors = selected.iter().map(|v| Veteran { unit: v.unit.clone(), xp: v.xp + Fx::ONE }).collect();
            let r = BattleResult { seed: c.seed, battle: c.battles, system: next, won: true, survivors, selected, undeployed: vec![], commander_xp: Some(c.commander_xp + Fx::from_int(2)), duration_ticks: 30 * (900 + u64::from(c.battles) * 97) };
            if !c.record(r) {
                break;
            }
        }
        Some(c)
    }

    pub fn path() -> std::path::PathBuf {
        ta_config::player_data_dir().join("conquest.toml")
    }

    /// The conquest in progress. Damage is reported, never silently renamed or
    /// replaced by a new campaign; the player can retry or explicitly abandon it.
    pub fn load_at(path: &Path) -> io::Result<Option<Conquest>> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        ordinary_save(&metadata)?;
        let file = std::fs::File::open(path)?;
        ordinary_save(&file.metadata()?)?;
        let mut text = String::new();
        file.take(MAX_SAVE_BYTES + 1).read_to_string(&mut text)?;
        if text.len() as u64 > MAX_SAVE_BYTES { return Err(invalid("conquest save exceeds the 1 MiB limit")); }
        let conquest: Conquest = toml::from_str(&text).map_err(|e: toml::de::Error| invalid(&format!("invalid conquest save: {}", e.message())))?;
        let conquest = conquest.migrated();
        conquest.validate()?;
        Ok(Some(conquest))
    }

    /// Creation must not overwrite a campaign another game created after the
    /// new-galaxy page opened. Even an unreadable existing save needs abandonment.
    pub fn create_at(&self, path: &Path) -> io::Result<()> {
        let _transaction = ta_config::SettingsTransaction::open(path)?;
        match std::fs::symlink_metadata(path) {
            Ok(_) => return Err(io::Error::new(io::ErrorKind::AlreadyExists, "a conquest save already exists")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
        self.write_at(path)
    }

    #[cfg(test)]
    pub fn save_at(&self, path: &Path) -> io::Result<()> {
        let _transaction = ta_config::SettingsTransaction::open(path)?;
        self.write_at(path)
    }

    fn write_at(&self, path: &Path) -> io::Result<()> {
        self.validate()?;
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
        if text.len() as u64 > MAX_SAVE_BYTES { return Err(invalid("conquest save exceeds the 1 MiB limit")); }
        ta_config::write_atomic(path, &text)
    }

    /// Serialize read/merge/write with creation and abandonment, including across
    /// processes. A retry after abandonment must not recreate the old campaign.
    pub fn record_at(path: &Path, result: BattleResult) -> io::Result<bool> {
        let _transaction = ta_config::SettingsTransaction::open(path)?;
        let Some(mut conquest) = Self::load_at(path)? else { return Ok(false) };
        if !conquest.record(result) { return Ok(false); }
        conquest.write_at(path)?;
        Ok(true)
    }

    /// Roster choices share the campaign transaction. A stale page may not overwrite
    /// battle progress or choose indices from a different roster.
    pub fn update_deployment_at(path: &Path, expected: &Conquest, deployment: Option<Vec<usize>>) -> io::Result<Conquest> {
        let _transaction = ta_config::SettingsTransaction::open(path)?;
        let Some(mut current) = Self::load_at(path)? else { return Err(io::Error::other("conquest no longer exists; reload it before selecting veterans")) };
        if current != *expected { return Err(io::Error::other("conquest changed in another game; reload it before selecting veterans")); }
        current.deployment = deployment;
        current.write_at(path)?;
        Ok(current)
    }

    pub fn delete_at(path: &Path) -> io::Result<()> {
        let _transaction = ta_config::SettingsTransaction::open(path)?;
        match std::fs::remove_file(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    /// A page opened before another game's battle finished may abandon only the
    /// version the player actually reviewed, never newer progress or a replacement.
    pub fn delete_matching_at(path: &Path, expected: &Conquest) -> io::Result<()> {
        let _transaction = ta_config::SettingsTransaction::open(path)?;
        match Self::load_at(path)? {
            None => Ok(()),
            Some(current) if current == *expected => std::fs::remove_file(path),
            Some(_) => Err(io::Error::other("conquest changed in another game; reload it before abandoning it")),
        }
    }

    /// Explicit abandonment of a damaged save must not delete a campaign another
    /// game repaired or replaced after the error page was displayed.
    pub fn delete_unreadable_at(path: &Path) -> io::Result<()> {
        let _transaction = ta_config::SettingsTransaction::open(path)?;
        match Self::load_at(path) {
            Ok(None) => Ok(()),
            Ok(Some(_)) => Err(io::Error::other("conquest changed in another game; reload it before abandoning it")),
            Err(_) => std::fs::remove_file(path),
        }
    }

    /// A save of an older format brought up to date: format 1's galaxy gets its rewards, dealt
    /// as a new galaxy from the same seed deals them.
    fn migrated(mut self) -> Conquest {
        if self.format == 1 {
            self.rewards = rewards_for(self.seed, self.systems.len(), self.home, self.capital);
            self.format = FORMAT;
        }
        // (Saves from before perks: the Commander's rank earns them now.)
        self.best_rank = self.best_rank.max(self.commander_rank());
        self
    }

    /// Takes `perk` for the conquest on disk, under the campaign's lock: refused for a page
    /// showing an older state (as [`Conquest::update_deployment_at`]).
    pub fn choose_perk_at(path: &Path, expected: &Conquest, perk: Perk) -> io::Result<Conquest> {
        let _transaction = ta_config::SettingsTransaction::open(path)?;
        let Some(mut current) = Self::load_at(path)? else { return Err(io::Error::other("conquest no longer exists; reload it before choosing a perk")) };
        if current != *expected { return Err(io::Error::other("conquest changed in another game; reload it before choosing a perk")); }
        if !current.choose_perk(perk) { return Err(invalid("that perk is not on offer")); }
        current.write_at(path)?;
        Ok(current)
    }

    /// Validate before the UI indexes topology or the game consumes experience.
    fn validate(&self) -> io::Result<()> {
        let n = self.systems.len();
        if self.format != FORMAT { return Err(invalid("unsupported conquest format")); }
        if !(4..=MAX_SYSTEMS).contains(&n) || self.home >= n || self.capital >= n || self.home == self.capital {
            return Err(invalid("invalid conquest systems or capitals"));
        }
        if !valid_name(&self.side, 128) || !valid_name(&self.enemy, 128)
            || self.commander_xp < Fx::ZERO || self.roster.len() > MAX_ROSTER || self.roster.iter().any(|v| !valid_veteran(v)) {
            return Err(invalid("invalid conquest faction or experience"));
        }
        if self.systems.iter().any(|s| !valid_name(&s.name, 4096) || !valid_name(&s.map, 4096)
            || s.planet.len() > 256 || !(1..=3).contains(&s.threat)
            || s.pos.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))) {
            return Err(invalid("invalid conquest system data"));
        }
        let mut lanes = std::collections::BTreeSet::new();
        if self.lanes.len() > n * (n - 1) / 2 || self.lanes.iter().any(|&(a, b)| a >= n || b >= n || a == b || !lanes.insert((a.min(b), a.max(b)))) {
            return Err(invalid("invalid conquest lanes"));
        }
        if self.jumps_from(self.home).contains(&usize::MAX) { return Err(invalid("disconnected conquest galaxy")); }
        if self.rewards.len() != n || self.rewards[self.home] != Reward::None || self.rewards[self.capital] != Reward::None {
            return Err(invalid("invalid conquest rewards"));
        }
        let mut taken = std::collections::BTreeSet::new();
        if self.best_rank > 3 || self.commander_rank() > self.best_rank || self.perks.len() > self.best_rank as usize
            || self.perks.iter().any(|p| !taken.insert(*p)) {
            return Err(invalid("invalid conquest perks"));
        }
        // Every victory took a system or beat off a counter-attack; every system lost fell to one.
        // (A save holding nothing, its home system lost, is caught just below: no overflow first.)
        let taken = self.held().saturating_sub(1) as u64 + u64::from(self.fallen) + u64::from(self.defended);
        if self.systems[self.home].owner != Owner::Player || self.won != (self.systems[self.capital].owner == Owner::Player)
            || self.wins > self.battles || u64::from(self.wins) != taken {
            return Err(invalid("invalid conquest progress"));
        }
        if self.counter.is_some_and(|c| self.won || c.system >= n || c.from >= n || c.system == self.home
            || self.systems[c.system].owner != Owner::Player || self.systems[c.from].owner != Owner::Enemy
            || !self.neighbours(c.system).any(|j| j == c.from)
            || c.due <= self.battles || c.due > self.battles.saturating_add(COUNTER_DUE)) {
            return Err(invalid("invalid conquest counter-attack"));
        }
        if self.last.is_none() != (self.battles == 0) || self.last.as_ref().is_some_and(|r| !report_fits(r, n, self.home)
            || r.won != (self.systems[r.system].owner == Owner::Player)) {
            return Err(invalid("invalid conquest battle report"));
        }
        let mut selected = std::collections::BTreeSet::new();
        if self.deployment.as_ref().is_some_and(|indices| indices.len() > self.slots()
            || indices.iter().any(|&i| i >= self.roster.len() || !selected.insert(i))) {
            return Err(invalid("invalid conquest deployment"));
        }
        // The journal replayed: what it says of each system's owner, battle by battle, holds
        // from one to the next and agrees with the galaxy now.
        let mut owner: Vec<Option<bool>> = vec![None; n];
        let mut consistent = true;
        for entry in &self.history {
            let r = &entry.report;
            if !report_fits(r, n, self.home) {
                consistent = false;
                break;
            }
            // (A defence at a system of ours; an attack on one of theirs.)
            consistent &= owner[r.system] != Some(!r.defence);
            owner[r.system] = Some(r.won);
            if let Some(f) = r.fell {
                consistent &= owner[f] != Some(false);
                owner[f] = Some(false);
            }
            if let Some(t) = r.threat {
                consistent &= owner[t] != Some(false) && r.won && !r.defence;
            }
            consistent &= r.broken.is_none() || (r.won && !r.defence);
        }
        consistent &= owner.iter().zip(&self.systems).all(|(o, s)| o.is_none_or(|o| o == (s.owner == Owner::Player)));
        if !consistent || self.history.len() > MAX_HISTORY || self.history.iter().any(|entry| entry.battle == 0 || entry.battle > self.battles
            || entry.duration_ticks >= ta_config::MAX_RECORD_TICKS)
            || self.history.windows(2).any(|pair| pair[0].battle.checked_add(1) != Some(pair[1].battle))
            || self.history.last().is_some_and(|entry| entry.battle != self.battles || Some(&entry.report) != self.last.as_ref())
            || self.history.iter().filter(|entry| entry.report.won).count() > self.wins as usize
            || self.history.iter().filter(|entry| !entry.report.won).count() > (self.battles - self.wins) as usize {
            return Err(invalid("invalid conquest battle journal"));
        }
        Ok(())
    }
}

fn invalid(message: &str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }

/// A report's own data in range: its systems (never home), its numbers, its details.
fn report_fits(r: &Report, n: usize, home: usize) -> bool {
    r.system < n && r.system != home && r.new_veterans <= r.returned && r.lost <= MAX_BERTHS && details_agree(r)
        && [r.fell, r.threat, r.broken].into_iter().flatten().all(|s| s < n && s != home)
        && r.fell != Some(r.system)
        && (r.won || (r.returned == 0 && r.new_veterans == 0))
}

/// A report's details (when it has them) list as many veterans as it counts, each a valid one.
fn details_agree(r: &Report) -> bool {
    r.details.as_ref().is_none_or(|d| {
        d.returned.len() == r.returned && d.promoted.len() == r.new_veterans && d.fallen.len() == r.lost
            && d.returned.len() <= MAX_ROSTER + 12
            && d.returned.iter().chain(&d.promoted).chain(&d.fallen).all(valid_veteran)
            && d.commander.iter().all(|xp| *xp >= Fx::ZERO)
            && d.berths.iter().all(|&b| b <= MAX_BERTHS)
            && (r.won || d.returned.is_empty())
    })
}

/// What each system brings: nothing for home and the enemy capital; the others dealt from the
/// seed, every kind in turn (shuffled), so each comes up about as often.
fn rewards_for(seed: u64, n: usize, home: usize, capital: usize) -> Vec<Reward> {
    let mut rng = Rng(seed ^ 0x05EE_D4E3_A2D5);
    let mut deck: Vec<Reward> = Vec::new();
    (0..n)
        .map(|i| {
            if i == home || i == capital {
                return Reward::None;
            }
            if deck.is_empty() {
                deck = Reward::ALL.to_vec();
                for k in (1..deck.len()).rev() {
                    deck.swap(k, rng.below(k + 1));
                }
            }
            deck.pop().unwrap_or_default()
        })
        .collect()
}

/// The unit a tank plant (a tank) or an airfield (a gunship) of `side` sends to each battle.
pub fn reinforcement(side: &str, reward: Reward) -> Option<&'static str> {
    let core = side.eq_ignore_ascii_case("CORE");
    match reward {
        Reward::Foundry => Some(if core { "CORRAID" } else { "ARMSTUMP" }),
        Reward::Airfield => Some(if core { "CORAPE" } else { "ARMBRAWL" }),
        _ => None,
    }
}
fn valid_name(name: &str, max: usize) -> bool { !name.trim().is_empty() && name.len() <= max && !name.chars().any(char::is_control) }
fn valid_veteran(veteran: &Veteran) -> bool { valid_name(&veteran.unit, 128) && veteran.xp >= Fx::ZERO }
fn ordinary_save(metadata: &std::fs::Metadata) -> io::Result<()> {
    #[cfg(windows)]
    let linked = { use std::os::windows::fs::MetadataExt; metadata.file_attributes() & 0x400 != 0 };
    #[cfg(not(windows))]
    let linked = metadata.file_type().is_symlink();
    if linked || !metadata.is_file() { return Err(invalid("conquest save must be an ordinary file")); }
    if metadata.len() > MAX_SAVE_BYTES { return Err(invalid("conquest save exceeds the 1 MiB limit")); }
    Ok(())
}

/// Lanes: a minimum spanning tree (every system reachable) plus short links to the nearest
/// neighbours that cross no other lane.
fn lanes(systems: &[System]) -> Vec<(usize, usize)> {
    let n = systems.len();
    // Distances on the map's 16:9 canvas.
    let d = |a: usize, b: usize| {
        let (p, q) = (systems[a].pos, systems[b].pos);
        ((p[0] - q[0]) * 1.78).hypot(p[1] - q[1])
    };
    let mut lanes: Vec<(usize, usize)> = Vec::new();
    let mut inside = vec![false; n];
    inside[0] = true;
    for _ in 1..n {
        let best = (0..n)
            .filter(|&a| inside[a])
            .flat_map(|a| (0..n).filter(|&b| !inside[b]).map(move |b| (a, b)))
            .min_by(|x, y| d(x.0, x.1).total_cmp(&d(y.0, y.1)));
        let Some((a, b)) = best else { break };
        inside[b] = true;
        lanes.push((a.min(b), a.max(b)));
    }
    let crosses = |lanes: &[(usize, usize)], a: usize, b: usize| {
        lanes.iter().any(|&(c, e)| c != a && c != b && e != a && e != b && segments_cross(systems[a].pos, systems[b].pos, systems[c].pos, systems[e].pos))
    };
    for a in 0..n {
        let mut near: Vec<usize> = (0..n).filter(|&b| b != a).collect();
        near.sort_by(|&x, &y| d(a, x).total_cmp(&d(a, y)));
        for &b in near.iter().take(3) {
            let lane = (a.min(b), a.max(b));
            if !lanes.contains(&lane) && d(a, b) < 0.42 && !crosses(&lanes, a, b) {
                lanes.push(lane);
            }
        }
    }
    lanes.sort_unstable();
    lanes
}

fn segments_cross(p1: [f32; 2], p2: [f32; 2], q1: [f32; 2], q2: [f32; 2]) -> bool {
    let side = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let (d1, d2) = (side(q1, q2, p1), side(q1, q2, p2));
    let (d3, d4) = (side(p1, p2, q1), side(p1, p2, q2));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// The conquest side of a finished battle: who won, and the ranked units of ours still
/// standing with their experience (the Commander's apart).
pub fn result_of(game: &Game, won: bool) -> Option<BattleResult> {
    let cs = game.setup.conquest.as_ref()?;
    let me = game.local_player;
    let sim = &game.sim;
    let mut survivors = Vec::new();
    let mut commander_xp = None;
    for u in sim.units().filter(|u| u.owner == me && !u.dead && u.is_complete()) {
        let t = sim.unit_type(u);
        if t.def.is_commander {
            commander_xp = Some(u.xp);
        } else if t.locomotion != Locomotion::Static && t.max_speed > Fx::ZERO && u.rank() >= 1 {
            survivors.push(Veteran { unit: t.name.clone(), xp: u.xp });
        }
    }
    Some(BattleResult { seed: cs.seed, battle: cs.battle, system: cs.system, won, survivors,
        selected: cs.veterans.iter().map(|(unit, xp)| Veteran { unit: unit.clone(), xp: *xp }).collect(),
        undeployed: game.undeployed_conquest_veterans.clone(), commander_xp, duration_ticks: sim.tick })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maps(n: usize) -> Vec<MapEntry> {
        (0..n)
            .map(|i| MapEntry {
                name: format!("map{i}"),
                title: format!("Map {i}"),
                description: String::new(),
                planet: ["Green Planet", "Desert", "Lava", "Metal"][i % 4].into(),
                size: "10 x 10".into(),
                start_positions: 2 + i % 3,
                info: Default::default(),
            })
            .collect()
    }

    #[test]
    fn galaxies_are_connected_and_grow_harder_towards_the_capital() {
        for (size, seed) in [(Size::Small, 1), (Size::Medium, 7), (Size::Large, 42)] {
            let c = Conquest::new(&maps(40), "ARM", Level::Standard, size, seed).unwrap();
            assert_eq!(c.systems.len(), size.systems());
            assert!(c.jumps_from(c.home).iter().all(|&j| j != usize::MAX), "every system reachable");
            assert_eq!(c.systems[c.capital].threat, 3);
            assert!(c.systems.iter().enumerate().filter(|&(i, _)| i != c.home).all(|(_, s)| s.owner == Owner::Enemy));
            assert!(c.systems[c.capital].pos[0] > c.systems[c.home].pos[0]);
            // Something to attack from the start.
            assert!((0..c.systems.len()).any(|i| c.attackable(i)));
            assert_eq!(Conquest::new(&maps(40), "ARM", Level::Standard, size, seed), Some(c));
        }
    }

    #[test]
    fn battles_move_veterans_and_systems() {
        let mut c = Conquest::new(&maps(40), "CORE", Level::Standard, Size::Small, 3).unwrap();
        c.rewards.fill(Reward::None);
        assert_eq!(c.enemy, "ARM");
        let target = (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
        let setup = c.setup(target, &maps(40)).unwrap();
        assert_eq!(setup.players.len(), 2);
        let vet = Veteran { unit: "CORAK".into(), xp: Fx::from_int(2) };
        let won = BattleResult { survivors: vec![vet.clone()], commander_xp: Some(Fx::from_int(5)), ..outcome(&c, target, true) };
        assert!(c.record(won));
        assert_eq!(c.systems[target].owner, Owner::Player);
        assert_eq!(c.roster, vec![vet]);
        assert_eq!(c.commander_xp, Fx::from_int(5));
        assert_eq!(c.last.as_ref().map(|r| r.new_veterans), Some(1));
        // A stale result (the same battle again) changes nothing.
        let again = BattleResult { battle: 0, ..outcome(&c, target, true) };
        assert!(!c.record(again));
        // Losing loses the veterans that went in and half the Commander's experience.
        let next = (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
        let lost = outcome(&c, next, false);
        assert!(c.record(lost));
        assert!(c.roster.is_empty());
        assert_eq!(c.last.as_ref().map(|r| r.lost), Some(1));
        assert_eq!(c.systems[next].owner, Owner::Enemy);
        assert_eq!(c.battles, 2);
    }

    #[test]
    fn reports_name_who_came_home_who_was_ranked_and_who_fell() {
        let mut c = Conquest::new(&maps(40), "CORE", Level::Standard, Size::Small, 3).unwrap();
        c.rewards.fill(Reward::None);
        let attackable = |c: &Conquest| (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
        let ak = Veteran { unit: "CORAK".into(), xp: Fx::from_int(2) };
        let thud = Veteran { unit: "CORTHUD".into(), xp: Fx::from_int(4) };
        let target = attackable(&c);
        assert!(c.record(BattleResult { survivors: vec![ak.clone(), thud.clone()], commander_xp: Some(Fx::from_int(3)), ..outcome(&c, target, true) }));
        let d = c.last.as_ref().and_then(|r| r.details.clone()).unwrap();
        assert_eq!(d.promoted, vec![thud.clone(), ak.clone()]);
        assert_eq!(d.commander, [Fx::ZERO, Fx::from_int(3)]);
        // Both go in: the AK comes home seasoned, the Thud falls, a Storm is newly ranked.
        let seasoned = Veteran { unit: "CORAK".into(), xp: Fx::from_int(5) };
        let storm = Veteran { unit: "CORSTORM".into(), xp: Fx::from_int(1) };
        let next = attackable(&c);
        let r = BattleResult { survivors: vec![storm.clone(), seasoned.clone()], commander_xp: Some(Fx::from_int(6)), ..outcome(&c, next, true) };
        assert_eq!(r.selected.len(), 2);
        assert!(c.record(r));
        let report = c.last.clone().unwrap();
        let d = report.details.clone().unwrap();
        assert_eq!(d.returned, vec![seasoned.clone(), storm.clone()]);
        assert_eq!(d.promoted, vec![storm.clone()]);
        assert_eq!(d.fallen, vec![thud.clone()]);
        assert_eq!(d.commander, [Fx::from_int(3), Fx::from_int(6)]);
        assert_eq!((report.returned, report.new_veterans, report.lost), (2, 1, 1));
        // A defeat: everyone sent in falls, and the Commander keeps half.
        let third = attackable(&c);
        assert!(c.record(outcome(&c, third, false)));
        let d = c.last.as_ref().and_then(|r| r.details.clone()).unwrap();
        assert!(d.returned.is_empty() && d.promoted.is_empty());
        assert_eq!(d.fallen, vec![seasoned, storm]);
        assert_eq!(d.commander, [Fx::from_int(6), Fx::from_int(3)]);
        // The details are saved, and checked against the counts when read.
        let text = toml::to_string_pretty(&c).unwrap();
        assert_eq!(toml::from_str::<Conquest>(&text).unwrap(), c);
        let mut bad = c.clone();
        bad.last.as_mut().unwrap().details.as_mut().unwrap().fallen.pop();
        assert!(bad.validate().is_err());
        let mut bad = c.clone();
        bad.last.as_mut().unwrap().details.as_mut().unwrap().returned.push(thud);
        assert!(bad.validate().is_err());
        // Only the journal's latest battles keep theirs.
        for _ in 0..DETAILED + 4 {
            assert!(c.record(outcome(&c, third, false)));
        }
        assert_eq!(c.history.iter().filter(|e| e.report.details.is_some()).count(), DETAILED);
        assert!(c.history.iter().rev().take(DETAILED).all(|e| e.report.details.is_some()));
        c.validate().unwrap();
        assert_eq!(c.defeats()[third], DETAILED as u32 + 5);
    }

    #[test]
    fn the_route_to_the_capital_steps_one_jump_nearer_each_time() {
        let mut c = Conquest::new(&maps(24), "ARM", Level::Standard, Size::Large, 5).unwrap();
        loop {
            let route = c.route_to_capital();
            if c.won {
                assert!(route.is_empty());
                break;
            }
            let distances = c.jumps_from(c.capital);
            assert_eq!(c.systems[route[0]].owner, Owner::Player);
            assert_eq!(*route.last().unwrap(), c.capital);
            assert_eq!(route.len(), distances[route[0]] + 1);
            for pair in route.windows(2) {
                assert!(c.neighbours(pair[0]).any(|j| j == pair[1]));
                assert_eq!(distances[pair[1]] + 1, distances[pair[0]]);
            }
            assert!(c.attackable(route[1]));
            assert!(c.record(outcome(&c, route[1], true)));
        }
    }

    #[test]
    fn rewards_are_dealt_from_the_seed_and_paid_while_held() {
        let maps = maps(40);
        let c = Conquest::new(&maps, "ARM", Level::Standard, Size::Large, 21).unwrap();
        assert_eq!(c.rewards, Conquest::new(&maps, "ARM", Level::Standard, Size::Large, 21).unwrap().rewards, "the same seed deals the same");
        assert_eq!((c.reward(c.home), c.reward(c.capital)), (Reward::None, Reward::None));
        let n = c.systems.len();
        let others = n - 2;
        assert!((0..n).filter(|&i| i != c.home && i != c.capital).all(|i| c.reward(i) != Reward::None), "every other system brings something");
        for kind in Reward::ALL {
            let count = c.rewards.iter().filter(|&&r| r == kind).count();
            assert!(others / 7 <= count && count <= others.div_ceil(7), "{kind:?} dealt {count} times");
        }
        // Held, they pay: supply, a berth, units for each battle.
        let mut held = c.clone();
        let of = |kind: Reward| (0..n).find(|&i| c.reward(i) == kind).unwrap();
        for kind in [Reward::Mines, Reward::Reactors, Reward::Depot, Reward::Foundry, Reward::Airfield] {
            held.systems[of(kind)].owner = Owner::Player;
        }
        let extra = (held.held() - 1) as u64;
        assert_eq!(held.bonus(), (extra * 150 + MINES_METAL, extra * 300 + REACTORS_ENERGY));
        assert_eq!(held.slots(), 4 + (held.held() - 1) / 2 + 1);
        let mut units = held.reinforcements();
        units.sort();
        assert_eq!(units, vec!["ARMBRAWL".to_string(), "ARMSTUMP".to_string()]);
        held.side = "CORE".into();
        let mut units = held.reinforcements();
        units.sort();
        assert_eq!(units, vec!["CORAPE".to_string(), "CORRAID".to_string()]);
        // Charted: home and its neighbours; a sensor array sees two jumps out; all once won.
        let charted = c.charted();
        assert!((0..n).all(|i| charted[i] == (i == c.home || c.neighbours(c.home).any(|j| j == i))));
        let mut seeing = c.clone();
        let sensors = of(Reward::Sensors);
        seeing.systems[sensors].owner = Owner::Player;
        let mut blind = seeing.clone();
        blind.rewards[sensors] = Reward::None;
        let (far, near) = (seeing.charted(), blind.charted());
        assert!(far.iter().zip(&near).all(|(&f, &n)| f || !n));
        assert!(far.iter().filter(|&&f| f).count() > near.iter().filter(|&&n| n).count(), "the array sees further");
        let mut won = c.clone();
        won.won = true;
        assert!(won.charted().iter().all(|&known| known));
    }

    #[test]
    fn counter_attacks_strike_back_and_fall_unless_answered() {
        let maps = maps(40);
        let attackable = |c: &Conquest| (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
        // Never on Relaxed.
        let mut relaxed = Conquest::new(&maps, "ARM", Level::Relaxed, Size::Large, 5).unwrap();
        for _ in 0..10 {
            let target = attackable(&relaxed);
            assert!(relaxed.record(outcome(&relaxed, target, true)));
            assert!(relaxed.counter.is_none() && relaxed.last.as_ref().unwrap().threat.is_none());
        }
        // On Hard the enemy soon strikes back, from beside the system it strikes at.
        let mut c = Conquest::new(&maps, "ARM", Level::Hard, Size::Large, 5).unwrap();
        while c.counter.is_none() {
            assert!(c.battles < 20, "the enemy strikes back on Hard");
            let target = attackable(&c);
            assert!(c.record(outcome(&c, target, true)));
        }
        let counter = c.counter.unwrap();
        assert!(c.battles >= 3);
        assert_ne!(counter.system, c.home);
        assert_eq!(c.systems[counter.system].owner, Owner::Player);
        assert_eq!(c.systems[counter.from].owner, Owner::Enemy);
        assert!(c.neighbours(counter.system).any(|j| j == counter.from));
        assert_eq!(counter.due, c.battles + COUNTER_DUE);
        assert_eq!(c.last.as_ref().unwrap().threat, Some(counter.system));
        assert!(c.defendable(counter.system) && !c.attackable(counter.system));
        c.validate().unwrap();
        // Its defence is fought against the attackers' strength.
        let setup = c.setup(counter.system, &maps).unwrap();
        assert!(setup.conquest.as_ref().is_some_and(|cs| cs.defence && cs.system == counter.system));
        let mut calm = c.clone();
        calm.counter = None;
        assert_eq!(c.enemies(counter.system), calm.enemies(counter.from));

        // Beaten off: ours still, a counter-attack defended.
        let mut held = c.clone();
        assert!(held.record(outcome(&held, counter.system, true)));
        assert!(held.counter.is_none() && held.last.as_ref().unwrap().defence);
        assert_eq!(held.systems[counter.system].owner, Owner::Player);
        assert_eq!((held.defended, held.fallen), (1, 0));
        held.validate().unwrap();
        // Lost: the system falls with the battle.
        let mut lost = c.clone();
        assert!(lost.record(outcome(&lost, counter.system, false)));
        assert_eq!(lost.systems[counter.system].owner, Owner::Enemy);
        assert_eq!((lost.defended, lost.fallen), (0, 1));
        assert!(lost.counter.is_none());
        lost.validate().unwrap();
        // Unanswered: it lands at the end of its due battle.
        let mut ignored = c.clone();
        while ignored.battles < counter.due {
            assert_eq!(ignored.systems[counter.system].owner, Owner::Player);
            let target = (0..ignored.systems.len()).find(|&i| ignored.attackable(i) && i != counter.from).unwrap();
            assert!(ignored.record(outcome(&ignored, target, false)));
        }
        assert_eq!(ignored.systems[counter.system].owner, Owner::Enemy);
        assert_eq!(ignored.last.as_ref().unwrap().fell, Some(counter.system));
        assert!(ignored.counter.is_none());
        assert_eq!(ignored.fallen, 1);
        ignored.validate().unwrap();
        // Broken up: by taking the system it comes from.
        let mut broken = c.clone();
        assert!(broken.attackable(counter.from));
        assert!(broken.record(outcome(&broken, counter.from, true)));
        assert_eq!(broken.last.as_ref().unwrap().broken, Some(counter.system));
        assert_eq!(broken.systems[counter.system].owner, Owner::Player);
        assert!(broken.counter.is_none_or(|next| next.due > counter.due), "only a new counter-attack");
        broken.validate().unwrap();

        // The war's turns stand checked: the journal replayed, the counter-attack, the totals.
        let both = |c: &mut Conquest, change: fn(&mut Report)| {
            change(c.last.as_mut().unwrap());
            change(&mut c.history.last_mut().unwrap().report);
        };
        let mutations: [fn(&mut Conquest); 7] = [
            |c| c.fallen += 1,
            |c| c.defended += 1,
            |c| c.counter.as_mut().unwrap().system = c.home,
            |c| c.counter.as_mut().unwrap().due += COUNTER_DUE,
            |c| c.counter.as_mut().unwrap().from = c.home,
            |c| c.rewards[c.capital] = Reward::Mines,
            |c| c.rewards.pop().map_or((), drop),
        ];
        for mutate in mutations {
            let mut bad = c.clone();
            mutate(&mut bad);
            assert!(bad.validate().is_err());
        }
        let mut bad = ignored.clone();
        both(&mut bad, |r| r.fell = None);
        assert!(bad.validate().is_err(), "a system lost without its fall in the journal");
        let mut bad = held.clone();
        both(&mut bad, |r| r.defence = false);
        assert!(bad.validate().is_err(), "an attack on a system of ours");
        let mut bad = c.clone();
        both(&mut bad, |r| r.threat = Some(usize::MAX));
        assert!(bad.validate().is_err(), "a counter-attack on no system");
    }

    #[test]
    fn promotions_earn_perks_offered_from_the_seed_and_kept() {
        let maps = maps(40);
        let attackable = |c: &Conquest| (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
        let mut c = Conquest::new(&maps, "ARM", Level::Relaxed, Size::Large, 7).unwrap();
        c.rewards.fill(Reward::None);
        assert_eq!(c.perks_due(), 0);
        let offer = c.perk_offer();
        assert_eq!(offer.len(), 3);
        assert_eq!(offer, Conquest::new(&maps, "ARM", Level::Relaxed, Size::Large, 7).unwrap().perk_offer(), "the same war offers the same");
        assert!(!c.choose_perk(offer[0]), "none before a promotion");
        // A victory that makes the Commander a veteran earns one: one of three offered.
        let target = attackable(&c);
        assert!(c.record(BattleResult { commander_xp: Some(Fx::from_int(2)), ..outcome(&c, target, true) }));
        assert_eq!((c.commander_rank(), c.best_rank, c.perks_due()), (1, 1, 1));
        let offer = c.perk_offer();
        let not_offered = *Perk::ALL.iter().find(|p| !offer.contains(p)).unwrap();
        assert!(!c.choose_perk(not_offered), "only what is offered");
        // Chosen under the campaign's lock, and never for a page showing an older state.
        let path = save_path();
        c.save_at(&path).unwrap();
        let chosen = Conquest::choose_perk_at(&path, &c, offer[1]).unwrap();
        assert!(Conquest::choose_perk_at(&path, &c, offer[2]).is_err(), "a stale page");
        assert_eq!(Conquest::load_at(&path).unwrap(), Some(chosen.clone()));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        c = chosen;
        assert_eq!((c.perks.clone(), c.perks_due()), (vec![offer[1]], 0));
        assert!(!c.perk_offer().contains(&offer[1]), "never offered twice");
        // The battle's setup names it.
        let setup = c.setup(attackable(&c), &maps).unwrap();
        assert_eq!(setup.conquest.unwrap().perks, vec![offer[1].id().to_string()]);
        // A defeat halves the experience, not the perks; promotions count from the best rank.
        let target = attackable(&c);
        assert!(c.record(outcome(&c, target, false)));
        assert_eq!((c.perks.len(), c.best_rank), (1, 1));
        let target = attackable(&c);
        assert!(c.record(BattleResult { commander_xp: Some(Fx::from_int(9)), ..outcome(&c, target, true) }));
        assert_eq!((c.commander_rank(), c.best_rank, c.perks_due()), (3, 3, 2));
        c.validate().unwrap();
        let mutations: [fn(&mut Conquest); 3] = [
            |c| c.perks.push(c.perks[0]),
            |c| c.best_rank = 4,
            |c| c.perks = Perk::ALL[..4].to_vec(),
        ];
        for mutate in mutations {
            let mut bad = c.clone();
            mutate(&mut bad);
            assert!(bad.validate().is_err());
        }
    }

    #[test]
    fn the_quartermaster_brings_a_quarter_more_and_battles_take_the_rest() {
        let mut c = Conquest::new(&maps(16), "ARM", Level::Standard, Size::Small, 9).unwrap();
        c.rewards.fill(Reward::None);
        for i in (0..c.systems.len()).filter(|&i| i != c.capital).take(4) {
            c.systems[i].owner = Owner::Player;
        }
        let (metal, energy) = c.bonus();
        assert!(metal > 0 && energy > 0);
        c.perks.push(Perk::Quartermaster);
        assert_eq!(c.bonus(), (metal + metal / 4, energy + energy / 4));
        let names: Vec<String> = ["engineer", "marksman", "quartermaster", "nonsense"].iter().map(|s| s.to_string()).collect();
        assert_eq!(battle_perks(&names), ta_sim::Perks { engineer: true, marksman: true, ..Default::default() });
        for perk in Perk::ALL {
            assert_eq!(battle_perks(&[perk.id().to_string()]) != ta_sim::Perks::default(), perk != Perk::Quartermaster, "{perk:?}");
        }
    }

    #[test]
    fn format_one_saves_are_dealt_their_rewards_when_read() {
        let mut c = Conquest::new(&maps(16), "ARM", Level::Standard, Size::Small, 9).unwrap();
        let target = (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
        assert!(c.record(outcome(&c, target, true)));
        let mut document = toml::Value::try_from(&c).unwrap();
        let fields = document.as_table_mut().unwrap();
        for field in ["rewards", "counter", "fallen", "defended"] {
            fields.remove(field);
        }
        fields.insert("format".into(), toml::Value::Integer(1));
        let path = save_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, toml::to_string(&document).unwrap()).unwrap();
        let read = Conquest::load_at(&path).unwrap().unwrap();
        assert_eq!(read.format, FORMAT);
        assert_eq!(read, c, "its rewards dealt as a new galaxy deals them");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn saves_read_back() {
        let c = Conquest::new(&maps(40), "ARM", Level::Hard, Size::Medium, 9).unwrap();
        let text = toml::to_string_pretty(&c).unwrap();
        assert_eq!(toml::from_str::<Conquest>(&text).unwrap(), c);
    }

    #[test]
    fn legacy_campaigns_keep_progress_and_begin_a_journal_on_the_next_battle() {
        let mut c = Conquest::new(&maps(16), "ARM", Level::Standard, Size::Small, 9).unwrap();
        let target = (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
        assert!(c.record(outcome(&c, target, false)));
        let mut document = toml::Value::try_from(&c).unwrap();
        let fields = document.as_table_mut().unwrap();
        fields.remove("history");
        fields.remove("deployment");
        let mut migrated: Conquest = document.try_into().unwrap();
        migrated.validate().unwrap();
        assert_eq!(migrated.battles, 1);
        assert_eq!(migrated.last, c.last);
        assert!(migrated.history.is_empty() && migrated.deployment.is_none());
        assert!(migrated.record(outcome(&migrated, target, true)));
        assert_eq!(migrated.history.len(), 1);
        assert_eq!(migrated.history[0].battle, 2);
        migrated.validate().unwrap();
    }

    #[test]
    fn custom_deployment_and_commander_only_reach_setup_and_protect_reserves() {
        let maps = maps(16);
        let mut c = Conquest::new(&maps, "ARM", Level::Standard, Size::Small, 9).unwrap();
        let target = (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
        c.roster = (1..=6).map(|xp| Veteran { unit: format!("ARM{xp}"), xp: Fx::from_int(xp) }).collect();
        assert_eq!(c.deployment_indices(), vec![5, 4, 3, 2], "automatic selection ranks an unsorted roster");
        c.deployment = Some(vec![0, 4]);
        let veterans = c.setup(target, &maps).unwrap().conquest.unwrap().veterans;
        assert_eq!(veterans, vec![("ARM1".into(), Fx::ONE), ("ARM5".into(), Fx::from_int(5))]);
        let battle = outcome(&c, target, false);
        // Changing the menu while this battle is saved must not change its casualties.
        c.deployment = Some(vec![2, 3]);
        assert!(c.record(battle));
        assert_eq!(c.last.as_ref().unwrap().lost, 2);
        assert_eq!(c.roster.iter().map(|v| v.unit.as_str()).collect::<Vec<_>>(), vec!["ARM6", "ARM4", "ARM3", "ARM2"]);
        assert!(c.deployment.is_none(), "the next battle returns to automatic selection");
        c.deployment = Some(Vec::new());
        assert!(c.setup(target, &maps).unwrap().conquest.unwrap().veterans.is_empty());
        let reserve = c.roster.clone();
        assert!(c.record(outcome(&c, target, false)));
        assert_eq!(c.roster, reserve);
        assert_eq!(c.last.as_ref().unwrap().lost, 0);
        c.validate().unwrap();
    }

    #[test]
    fn battle_selection_is_a_bounded_multiset_of_the_saved_roster() {
        let mut c = Conquest::new(&maps(16), "ARM", Level::Standard, Size::Small, 9).unwrap();
        let target = (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
        c.roster = vec![Veteran { unit: "ARMPW".into(), xp: Fx::ONE }];
        let original = c.clone();
        let mut duplicate = outcome(&c, target, false);
        duplicate.selected.push(c.roster[0].clone());
        assert!(!c.record(duplicate));
        let mut unknown = outcome(&c, target, false);
        unknown.selected[0].xp = Fx::from_int(2);
        assert!(!c.record(unknown));
        let mut double_reserve = outcome(&c, target, false);
        double_reserve.undeployed = vec![c.roster[0].clone(); 2];
        assert!(!c.record(double_reserve));
        let mut invalid_duration = outcome(&c, target, false);
        invalid_duration.duration_ticks = ta_config::MAX_RECORD_TICKS;
        assert!(!c.record(invalid_duration));
        assert_eq!(c, original, "invalid battle snapshots do not partially alter the campaign");
    }

    #[test]
    fn journal_is_bounded_ordered_and_validated_without_losing_totals() {
        let mut c = Conquest::new(&maps(16), "ARM", Level::Standard, Size::Small, 9).unwrap();
        let target = (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
        for _ in 0..140 { assert!(c.record(outcome(&c, target, false))); }
        assert_eq!(c.history.len(), MAX_HISTORY);
        assert_eq!(c.history[0].battle, 13);
        assert_eq!(c.history.last().unwrap().battle, 140);
        assert_eq!(c.battles, 140);
        c.validate().unwrap();
        let mutations: &[fn(&mut Conquest)] = &[
            |c| c.history[0].battle = 0,
            |c| c.history[1].battle = c.history[0].battle,
            |c| c.history[0].duration_ticks = ta_config::MAX_RECORD_TICKS,
            |c| c.history[0].report.system = usize::MAX,
            |c| c.history[0].report.returned = 1,
            |c| c.history[0].report.won = true,
            |c| c.history.last_mut().unwrap().report.lost = 1,
            |c| c.history.push(c.history.last().unwrap().clone()),
        ];
        for mutate in mutations {
            let mut bad = c.clone();
            mutate(&mut bad);
            assert!(bad.validate().is_err());
        }
        // A defeat remains a defeat in history after the same system is captured.
        assert!(c.record(outcome(&c, target, true)));
        c.validate().unwrap();
        assert_eq!(c.wins, 1);
    }

    #[test]
    fn capital_route_and_reinforcement_milestones_follow_territory() {
        // (Relaxed: no counter-attacks, so territory only grows.)
        let mut c = Conquest::new(&maps(24), "ARM", Level::Relaxed, Size::Large, 9).unwrap();
        assert_eq!(c.remaining_jumps(), c.capital_jumps());
        assert_eq!(c.next_slot_in(), Some(2));
        while !c.won {
            let remaining = c.remaining_jumps();
            let distances = c.jumps_from(c.capital);
            let target = (0..c.systems.len()).filter(|&i| c.attackable(i)).min_by_key(|&i| distances[i]).unwrap();
            assert!(c.record(outcome(&c, target, true)));
            assert_eq!(c.remaining_jumps(), remaining - 1);
        }
        // (Territory's berths alone: no depots.)
        c.rewards.fill(Reward::None);
        for held in 1..=24 {
            for (i, system) in c.systems.iter_mut().enumerate() { system.owner = if i < held { Owner::Player } else { Owner::Enemy }; }
            assert_eq!(c.next_slot_in(), if held >= 17 { None } else { Some(if held % 2 == 0 { 1 } else { 2 }) });
        }
    }

    #[test]
    fn deployment_updates_reject_stale_and_invalid_choices_without_overwriting_progress() {
        let mut c = Conquest::new(&maps(16), "ARM", Level::Standard, Size::Small, 9).unwrap();
        c.roster = vec![Veteran { unit: "ARMPW".into(), xp: Fx::ONE }; 6];
        let path = save_path();
        c.save_at(&path).unwrap();
        for selection in [vec![0, 0], vec![6], vec![0, 1, 2, 3, 4]] {
            assert!(Conquest::update_deployment_at(&path, &c, Some(selection)).is_err());
            assert_eq!(Conquest::load_at(&path).unwrap(), Some(c.clone()));
        }
        let current = Conquest::update_deployment_at(&path, &c, Some(vec![2])).unwrap();
        assert_eq!(current.deployment, Some(vec![2]));
        assert!(Conquest::update_deployment_at(&path, &c, None).is_err(), "stale roster pages cannot overwrite a newer selection");
        let target = (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
        assert!(Conquest::record_at(&path, outcome(&current, target, true)).unwrap());
        let progressed = Conquest::load_at(&path).unwrap().unwrap();
        assert!(Conquest::update_deployment_at(&path, &current, Some(vec![])).is_err());
        assert_eq!(Conquest::load_at(&path).unwrap(), Some(progressed));
        Conquest::delete_at(&path).unwrap();
        assert!(Conquest::update_deployment_at(&path, &current, None).is_err());
        assert!(!path.exists(), "a stale roster page cannot recreate an abandoned campaign");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    fn outcome(c: &Conquest, system: usize, won: bool) -> BattleResult {
        BattleResult { seed: c.seed, battle: c.battles, system, won, survivors: vec![], selected: c.selected_veterans(), undeployed: vec![], commander_xp: None, duration_ticks: 30 * 90 }
    }

    fn save_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ta_conquest_{}", ta_config::profiles::new_id())).join("conquest.toml")
    }

    #[test]
    fn capital_keeps_the_only_map_with_room_for_both_defenders() {
        for n in [4, 10, 24] {
            let mut maps = maps(n);
            for map in &mut maps { map.start_positions = 2; }
            maps[0].start_positions = 3;
            for seed in 0..100 {
                let c = Conquest::new(&maps, "ARM", Level::Standard, Size::Large, seed).unwrap();
                assert_eq!(c.systems[c.capital].map, maps[0].name, "seed {seed}, {n} maps");
                assert_eq!(c.systems.iter().map(|s| &s.map).collect::<std::collections::BTreeSet<_>>().len(), n);
                c.validate().unwrap();
            }
        }
    }

    #[test]
    fn unavailable_targets_and_invalid_results_leave_progress_unchanged() {
        let maps = maps(16);
        let mut c = Conquest::new(&maps, "ARM", Level::Standard, Size::Small, 42).unwrap();
        let target = (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
        let distant = (0..c.systems.len()).find(|&i| i != c.home && !c.attackable(i)).unwrap();
        for i in [c.home, distant, usize::MAX] {
            assert!(c.setup(i, &maps).is_none());
            let before = c.clone();
            assert!(!c.record(outcome(&c, i, true)));
            assert_eq!(c, before);
        }
        assert!(c.setup(target, &[]).is_none(), "removed map cannot start a battle");
        let mut changed_maps = maps.clone();
        changed_maps.iter_mut().find(|m| m.name == c.systems[target].map).unwrap().start_positions = 1;
        assert!(c.setup(target, &changed_maps).is_none());
        assert!(c.setup(target, &maps).is_some());
        let mut corrupt = outcome(&c, target, true);
        corrupt.survivors.push(Veteran { unit: "ARMPW".into(), xp: Fx::from_int(-3) });
        assert!(!c.record(corrupt));
        let before = c.clone();
        let mut unknown = outcome(&c, target, true);
        unknown.undeployed.push(Veteran { unit: "ARMPW".into(), xp: Fx::ONE });
        assert!(!c.record(unknown), "undeployed must be drawn from the selected roster");
        assert_eq!(c, before);
        c.battles = u32::MAX;
        assert!(!c.record(outcome(&c, target, false)), "battle counter cannot overflow");
    }

    #[test]
    fn units_that_cannot_deploy_are_not_killed_or_counted_as_casualties() {
        for won in [false, true] {
            let mut c = Conquest::new(&maps(16), "ARM", Level::Standard, Size::Small, 1).unwrap();
            let target = (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
            let ship = Veteran { unit: "ARMDEST".into(), xp: Fx::from_int(6) };
            let land = Veteran { unit: "ARMPW".into(), xp: Fx::from_int(2) };
            c.roster = vec![ship.clone(), ship.clone(), land.clone()];
            let mut result = outcome(&c, target, won);
            // One identical ship deploys; the other remains aboard its transport.
            result.undeployed.push(ship.clone());
            if won { result.survivors.push(Veteran { xp: Fx::from_int(3), ..land.clone() }); }
            assert!(c.record(result));
            assert_eq!(c.roster.iter().filter(|v| v.unit == ship.unit).count(), 1);
            assert_eq!(c.last.as_ref().unwrap().lost, if won { 1 } else { 2 });
            assert_eq!(c.last.as_ref().unwrap().new_veterans, 0);
            c.validate().unwrap();
        }
    }

    #[test]
    fn malformed_topology_and_progress_are_rejected_before_display_or_write() {
        let valid = Conquest::new(&maps(16), "ARM", Level::Standard, Size::Small, 1).unwrap();
        let mutations: &[fn(&mut Conquest)] = &[
            |c| c.systems.clear(), |c| c.home = usize::MAX, |c| c.capital = c.home,
            |c| c.lanes.push((0, usize::MAX)), |c| c.lanes.push(c.lanes[0]),
            |c| c.lanes.clear(), |c| c.systems[0].pos[0] = f32::NAN,
            |c| c.systems[0].pos[1] = 2.0, |c| c.systems[0].threat = 0,
            |c| c.systems[c.home].owner = Owner::Enemy, |c| c.won = true,
            |c| c.wins = 1, |c| c.battles = 1, |c| c.commander_xp = Fx::from_int(-3),
            |c| c.roster = vec![Veteran { unit: "ARMPW".into(), xp: Fx::ONE }; MAX_ROSTER + 1],
        ];
        let path = save_path();
        for mutate in mutations {
            valid.save_at(&path).unwrap();
            let original = std::fs::read(&path).unwrap();
            let mut bad = valid.clone();
            mutate(&mut bad);
            assert!(bad.save_at(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), original, "invalid writes preserve the valid save");
            let corrupt = toml::to_string(&bad).unwrap();
            std::fs::write(&path, &corrupt).unwrap();
            assert!(Conquest::load_at(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), corrupt, "invalid loads preserve evidence and allow repair");
        }
        std::fs::write(&path, vec![b'x'; MAX_SAVE_BYTES as usize + 1]).unwrap();
        assert!(Conquest::load_at(&path).unwrap_err().to_string().contains("1 MiB"));
        assert_eq!(std::fs::metadata(&path).unwrap().len(), MAX_SAVE_BYTES + 1);
        Conquest::delete_at(&path).unwrap();
        assert!(Conquest::load_at(&path).unwrap().is_none());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn concurrent_result_writers_apply_a_battle_only_once() {
        let c = Conquest::new(&maps(16), "ARM", Level::Standard, Size::Small, 1).unwrap();
        let path = save_path();
        c.save_at(&path).unwrap();
        let target = (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
        let result = outcome(&c, target, true);
        let gate = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            let writes: Vec<_> = (0..2).map(|_| scope.spawn(|| {
                gate.wait();
                Conquest::record_at(&path, result.clone()).unwrap()
            })).collect();
            assert_eq!(writes.into_iter().map(|write| write.join().unwrap()).filter(|written| *written).count(), 1);
        });
        let saved = Conquest::load_at(&path).unwrap().unwrap();
        assert_eq!((saved.battles, saved.wins), (1, 1));
        Conquest::delete_at(&path).unwrap();
        assert!(!Conquest::record_at(&path, result).unwrap(), "retry after abandonment does not recreate the save");
        assert!(Conquest::load_at(&path).unwrap().is_none());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn creation_never_overwrites_an_existing_or_damaged_campaign() {
        let first = Conquest::new(&maps(16), "ARM", Level::Standard, Size::Small, 1).unwrap();
        let other = Conquest::new(&maps(16), "CORE", Level::Hard, Size::Small, 2).unwrap();
        let path = save_path();
        first.create_at(&path).unwrap();
        assert_eq!(other.create_at(&path).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(Conquest::load_at(&path).unwrap(), Some(first));
        std::fs::write(&path, "systems = [ damaged").unwrap();
        assert_eq!(other.create_at(&path).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "systems = [ damaged");
        Conquest::delete_at(&path).unwrap();
        other.create_at(&path).unwrap();
        assert_eq!(Conquest::load_at(&path).unwrap(), Some(other));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn stale_abandonment_preserves_new_battle_progress_and_replacement_campaigns() {
        let first = Conquest::new(&maps(16), "ARM", Level::Standard, Size::Small, 1).unwrap();
        let path = save_path();
        first.create_at(&path).unwrap();
        let target = (0..first.systems.len()).find(|&i| first.attackable(i)).unwrap();
        assert!(Conquest::record_at(&path, outcome(&first, target, true)).unwrap());
        let progressed = Conquest::load_at(&path).unwrap().unwrap();
        assert!(Conquest::delete_matching_at(&path, &first).is_err());
        assert_eq!(Conquest::load_at(&path).unwrap(), Some(progressed.clone()));
        Conquest::delete_matching_at(&path, &progressed).unwrap();
        let replacement = Conquest::new(&maps(16), "CORE", Level::Hard, Size::Small, 2).unwrap();
        replacement.create_at(&path).unwrap();
        assert!(Conquest::delete_matching_at(&path, &progressed).is_err());
        assert_eq!(Conquest::load_at(&path).unwrap(), Some(replacement.clone()));
        Conquest::delete_matching_at(&path, &replacement).unwrap();
        Conquest::delete_matching_at(&path, &replacement).unwrap();
        assert!(Conquest::load_at(&path).unwrap().is_none());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn damaged_save_abandonment_rechecks_for_a_repaired_campaign() {
        let valid = Conquest::new(&maps(16), "ARM", Level::Standard, Size::Small, 1).unwrap();
        let path = save_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "systems = [ damaged").unwrap();
        assert!(Conquest::load_at(&path).is_err());
        valid.save_at(&path).unwrap();
        assert!(Conquest::delete_unreadable_at(&path).is_err(), "a repaired save is no longer eligible for error-page abandonment");
        assert_eq!(Conquest::load_at(&path).unwrap(), Some(valid));
        std::fs::write(&path, "systems = [ damaged").unwrap();
        Conquest::delete_unreadable_at(&path).unwrap();
        assert!(Conquest::load_at(&path).unwrap().is_none());
        Conquest::delete_unreadable_at(&path).unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn pending_results_read_the_latest_campaign_after_taking_the_lock() {
        let old = Conquest::new(&maps(16), "ARM", Level::Standard, Size::Small, 1).unwrap();
        let new = Conquest::new(&maps(16), "CORE", Level::Hard, Size::Small, 2).unwrap();
        let target = (0..old.systems.len()).find(|&i| old.attackable(i)).unwrap();
        let path = save_path();
        old.save_at(&path).unwrap();
        let held = ta_config::SettingsTransaction::open(&path).unwrap();
        let (started, receive) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let writer = scope.spawn(|| {
                started.send(()).unwrap();
                Conquest::record_at(&path, outcome(&old, target, true)).unwrap()
            });
            receive.recv().unwrap();
            // This is the creation transaction that owns the lock. The result
            // writer must read its replacement, never overwrite it with old data.
            new.write_at(&path).unwrap();
            drop(held);
            assert!(!writer.join().unwrap());
        });
        assert_eq!(Conquest::load_at(&path).unwrap(), Some(new));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
