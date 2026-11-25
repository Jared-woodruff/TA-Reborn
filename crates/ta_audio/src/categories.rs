//! Sound tables from TA's game data.
//!
//! * [`SoundCategories`] — `gamedata/sound.tdf`. Every unit's FBI names a section with
//!   `SoundCategory=ARM_KBOT;`; the section maps events to sound names:
//!
//!   ```text
//!   [ARM_COM]
//!       {
//!       select1=kcarmsel;   ok1=kcarmmov;   arrived1=kcarmstp;   cant1=cantdo4;
//!       underattack=warning2;   build=nanlath1;   repair=repair1;   working=reclaim1;
//!       cloak=kloak1;   uncloak=kloak1un;   count5=count1; ... count0=count6;
//!       canceldestruct=cancel2;
//!       }
//!   ```
//!
//!   The numeric suffix of `select1`/`ok1`/`arrived1`/`cant1` numbers alternative variants
//!   (the shipped data only uses `1`, mods add `select2`...), except for `count5`..`count0`,
//!   the spoken self-destruct countdown. The shipped data has a few dangling references: units
//!   naming `CORE_KBOT`, `CORE_MEX` and `COR_TANK` where the file defines `COR_KBOT`,
//!   `COR_MEX` and `CORE_TANK`, which [`SoundCategories::category`] resolves by trying the
//!   other Core prefix; six units use `NONE`.
//!   TA Reborn reads more than TA's one table: `gamedata/sound_<name>.tdf` add-ons, merged over
//!   it in the order of their paths, a category's keys over its keys ([`SoundCategories::load`]);
//!   more keys of its own (a unit's death, its moves and loops, its alerts: [`SoundEvent`]); and
//!   [sound sets](SoundSet), sections of a `[SOUNDSETS]` section, named wherever a sound's file
//!   name goes.
//! * [`UiSounds`] — `gamedata/allsound.tdf`: interface events (`[BIGBUTTON] { sound=butmain1; }`).
//! * [`WeaponSounds`] — the sound keys of a weapon TDF section.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use ta_formats::tdf::{TdfDocument, TdfSection};

use crate::bank::SoundSource;
use crate::engine::Priority;
use crate::{AudioError, Result};

/// A unit event that can have a sound in `gamedata/sound.tdf` (or a sound add-on): TA's own,
/// then TA Reborn's (from [`SoundEvent::Death`] on).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SoundEvent {
    /// Unit selected (`select1`, `select2`, ...).
    Select,
    /// Order acknowledged (`ok1`, ...).
    Ok,
    /// Move order completed (`arrived1`, ...).
    Arrived,
    /// Order refused (`cant1`, ...).
    Cant,
    /// The unit is being attacked (a global alert).
    UnderAttack,
    /// Construction started (nanolathe).
    Build,
    Repair,
    /// Reclaiming / continuous work.
    Working,
    /// On/off toggles (radar, jammers, metal makers...).
    Activate,
    Deactivate,
    /// A factory finished a unit.
    UnitComplete,
    Cloak,
    Uncloak,
    Capture,
    /// Self-destruct countdown voice: `count5` first, `count0` at detonation.
    Count(u8),
    /// Self-destruct cancelled.
    CancelDestruct,
    /// It dies (`death`), in place of the generic blast: placed where it is.
    Death,
    /// What is left of it hits the ground falling out of the sky (`crash`), starts to sink
    /// (`sink`), a building comes down (`collapse`).
    Crash,
    Sink,
    Collapse,
    /// It sets off (`startmove`), comes to a stop (`stopmove`), and on its way (`moveloop`, a
    /// loop following its speed).
    StartMove,
    StopMove,
    MoveLoop,
    /// An aircraft leaves the ground (`takeoff`) and sets down (`landing`).
    TakeOff,
    Landing,
    /// While it builds (`buildloop`, after `build`), reclaims (`reclaimloop`, after `working`) or
    /// captures (`captureloop`), and as it stops (`buildstop`, `reclaimstop`, `capturestop`).
    BuildLoop,
    BuildStop,
    ReclaimLoop,
    ReclaimStop,
    CaptureLoop,
    CaptureStop,
    /// Finished, where it stands (`donebuilt`; `unitcomplete` is the factory's voice).
    DoneBuilt,
    /// While it is on and at work (`activeloop`), as it opens up and closes (`open`, `close`).
    ActiveLoop,
    Open,
    Close,
    /// Its shield comes up and goes down (`shieldon`, `shieldoff`).
    ShieldOn,
    ShieldOff,
    /// A side's alerts (its alerts category's): the player's commander under attack
    /// (`commanderattacked`), a strategic missile launched (`nuclearlaunch`).
    CommanderAttacked,
    NuclearLaunch,
}

impl SoundEvent {
    /// Every event except the countdown, in `sound.tdf` key order (TA's, then TA Reborn's).
    pub const SIMPLE: [SoundEvent; 38] = [
        SoundEvent::Select,
        SoundEvent::Ok,
        SoundEvent::Arrived,
        SoundEvent::Cant,
        SoundEvent::UnderAttack,
        SoundEvent::Build,
        SoundEvent::Repair,
        SoundEvent::Working,
        SoundEvent::Activate,
        SoundEvent::Deactivate,
        SoundEvent::UnitComplete,
        SoundEvent::Cloak,
        SoundEvent::Uncloak,
        SoundEvent::Capture,
        SoundEvent::CancelDestruct,
        SoundEvent::Death,
        SoundEvent::Crash,
        SoundEvent::Sink,
        SoundEvent::Collapse,
        SoundEvent::StartMove,
        SoundEvent::StopMove,
        SoundEvent::MoveLoop,
        SoundEvent::TakeOff,
        SoundEvent::Landing,
        SoundEvent::BuildLoop,
        SoundEvent::BuildStop,
        SoundEvent::ReclaimLoop,
        SoundEvent::ReclaimStop,
        SoundEvent::CaptureLoop,
        SoundEvent::CaptureStop,
        SoundEvent::DoneBuilt,
        SoundEvent::ActiveLoop,
        SoundEvent::Open,
        SoundEvent::Close,
        SoundEvent::ShieldOn,
        SoundEvent::ShieldOff,
        SoundEvent::CommanderAttacked,
        SoundEvent::NuclearLaunch,
    ];

    /// The key without variant number, e.g. `"select"`, `"count3"`.
    pub fn key(self) -> String {
        match self {
            SoundEvent::Count(n) => format!("count{n}"),
            other => other.base_key().to_string(),
        }
    }

    fn base_key(self) -> &'static str {
        match self {
            SoundEvent::Select => "select",
            SoundEvent::Ok => "ok",
            SoundEvent::Arrived => "arrived",
            SoundEvent::Cant => "cant",
            SoundEvent::UnderAttack => "underattack",
            SoundEvent::Build => "build",
            SoundEvent::Repair => "repair",
            SoundEvent::Working => "working",
            SoundEvent::Activate => "activate",
            SoundEvent::Deactivate => "deactivate",
            SoundEvent::UnitComplete => "unitcomplete",
            SoundEvent::Cloak => "cloak",
            SoundEvent::Uncloak => "uncloak",
            SoundEvent::Capture => "capture",
            SoundEvent::Count(_) => "count",
            SoundEvent::CancelDestruct => "canceldestruct",
            SoundEvent::Death => "death",
            SoundEvent::Crash => "crash",
            SoundEvent::Sink => "sink",
            SoundEvent::Collapse => "collapse",
            SoundEvent::StartMove => "startmove",
            SoundEvent::StopMove => "stopmove",
            SoundEvent::MoveLoop => "moveloop",
            SoundEvent::TakeOff => "takeoff",
            SoundEvent::Landing => "landing",
            SoundEvent::BuildLoop => "buildloop",
            SoundEvent::BuildStop => "buildstop",
            SoundEvent::ReclaimLoop => "reclaimloop",
            SoundEvent::ReclaimStop => "reclaimstop",
            SoundEvent::CaptureLoop => "captureloop",
            SoundEvent::CaptureStop => "capturestop",
            SoundEvent::DoneBuilt => "donebuilt",
            SoundEvent::ActiveLoop => "activeloop",
            SoundEvent::Open => "open",
            SoundEvent::Close => "close",
            SoundEvent::ShieldOn => "shieldon",
            SoundEvent::ShieldOff => "shieldoff",
            SoundEvent::CommanderAttacked => "commanderattacked",
            SoundEvent::NuclearLaunch => "nuclearlaunch",
        }
    }

    /// The key's name without a variant number (`"moveloop"`, `"count"`), for the docs and the
    /// mod check.
    pub fn name(self) -> &'static str {
        self.base_key()
    }

    /// Played over and over while what it is of goes on (`moveloop`, `buildloop`,
    /// `reclaimloop`, `captureloop`, `activeloop`), until the game stops it.
    pub fn is_loop(self) -> bool {
        matches!(self, SoundEvent::MoveLoop | SoundEvent::BuildLoop | SoundEvent::ReclaimLoop | SoundEvent::CaptureLoop | SoundEvent::ActiveLoop)
    }

    /// Parses a `sound.tdf` key into its event and variant number: `select1` → (Select, 1),
    /// `underattack` → (UnderAttack, 0), `count3` → (Count(3), 0). Case-insensitive.
    pub fn parse_key(key: &str) -> Option<(SoundEvent, u32)> {
        let key = key.trim().to_ascii_lowercase();
        let digits_at = key.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        let (base, digits) = key.split_at(digits_at);
        if base == "count" {
            return digits.parse::<u8>().ok().filter(|&n| n <= 9).map(|n| (SoundEvent::Count(n), 0));
        }
        let variant = if digits.is_empty() { 0 } else { digits.parse().ok()? };
        SoundEvent::SIMPLE.iter().find(|e| e.base_key() == base).map(|&e| (e, variant))
    }

    /// Spoken unit responses and alerts. TA plays these without position (the player hears
    /// the unit they commanded wherever the camera is); the engine routes them to the voice
    /// bus. Everything else (nanolathes, toggles, cloaking, deaths, moves, loops) is a
    /// positional effect.
    pub fn is_acknowledgement(self) -> bool {
        matches!(
            self,
            SoundEvent::Select
                | SoundEvent::Ok
                | SoundEvent::Arrived
                | SoundEvent::Cant
                | SoundEvent::UnderAttack
                | SoundEvent::UnitComplete
                | SoundEvent::Count(_)
                | SoundEvent::CancelDestruct
                | SoundEvent::CommanderAttacked
                | SoundEvent::NuclearLaunch
        )
    }
}

/// One `[CATEGORY]` section of `sound.tdf`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SoundCategory {
    name: String,
    /// Event → variants sorted by variant number (stable for equal numbers).
    sounds: BTreeMap<SoundEvent, Vec<(u32, String)>>,
    /// Keys this parser does not know (kept for mods and diagnostics).
    extra: Vec<(String, String)>,
}

impl SoundCategory {
    pub fn from_section(section: &TdfSection) -> SoundCategory {
        let mut category = SoundCategory { name: section.name.clone(), ..Default::default() };
        category.merge(section);
        category
    }

    /// Lays `section`'s keys over this category's (a sound add-on's section of its name): each
    /// key it gives in place of the one of that key and variant, the others kept.
    pub fn merge(&mut self, section: &TdfSection) {
        for (key, value) in &section.entries {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match SoundEvent::parse_key(key) {
                Some((event, variant)) => {
                    let list = self.sounds.entry(event).or_default();
                    // A repeated key overrides (last assignment wins, as in TA).
                    list.retain(|(v, _)| *v != variant);
                    list.push((variant, value.to_string()));
                    list.sort_by_key(|(v, _)| *v);
                }
                None => {
                    self.extra.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
                    self.extra.push((key.clone(), value.to_string()));
                }
            }
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_empty(&self) -> bool {
        self.sounds.is_empty()
    }

    /// The first variant for an event.
    pub fn get(&self, event: SoundEvent) -> Option<&str> {
        self.sounds.get(&event).and_then(|v| v.first()).map(|(_, s)| s.as_str())
    }

    /// All variants for an event (`select1`, `select2`, ...).
    pub fn variants(&self, event: SoundEvent) -> impl Iterator<Item = &str> {
        self.sounds.get(&event).into_iter().flatten().map(|(_, s)| s.as_str())
    }

    /// Picks a variant using caller-provided randomness.
    pub fn pick(&self, event: SoundEvent, random: u32) -> Option<&str> {
        let list = self.sounds.get(&event)?;
        list.get(random as usize % list.len().max(1)).map(|(_, s)| s.as_str())
    }

    /// Events that have at least one sound.
    pub fn events(&self) -> impl Iterator<Item = SoundEvent> + '_ {
        self.sounds.keys().copied()
    }

    /// Unrecognised `key=value` pairs.
    pub fn extra(&self) -> &[(String, String)] {
        &self.extra
    }

    /// The category it takes the keys it does not give from (TA Reborn's `base`), if any.
    pub fn base(&self) -> Option<&str> {
        self.extra.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case("base")).map(|(_, v)| v.as_str()).filter(|v| !v.trim().is_empty())
    }

    /// Takes every event `base` gives that it gives none of.
    fn inherit(&mut self, base: &SoundCategory) {
        for (event, variants) in &base.sounds {
            self.sounds.entry(*event).or_insert_with(|| variants.clone());
        }
    }
}

/// All unit sound categories from `gamedata/sound.tdf` and its add-ons, and the sound sets they
/// give.
#[derive(Clone, Debug, Default)]
pub struct SoundCategories {
    /// Upper-case section name → category.
    categories: HashMap<String, SoundCategory>,
    sets: Arc<SoundSets>,
    empty: SoundCategory,
}

/// The key of the category `name` (in capitals) names among `categories`, TA's two Core prefixes
/// taken for each other (as [`SoundCategories::try_category`] does).
fn aliased(categories: &HashMap<String, SoundCategory>, name: &str) -> Option<String> {
    if categories.contains_key(name) {
        return Some(name.to_string());
    }
    let alias = match name.strip_prefix("CORE_") {
        Some(rest) => format!("COR_{rest}"),
        None => format!("CORE_{}", name.strip_prefix("COR_")?),
    };
    categories.contains_key(&alias).then_some(alias)
}

/// Whether `path` (a VFS path, normalised) is one of the sound tables: TA's `gamedata/sound.tdf`
/// or an add-on, `gamedata/sound_<name>.tdf`.
pub fn is_sound_table(path: &str) -> bool {
    path == SoundCategories::PATH || is_sound_add_on(path)
}

/// Whether `path` (a VFS path, normalised) is a sound add-on, `gamedata/sound_<name>.tdf`.
pub fn is_sound_add_on(path: &str) -> bool {
    path.strip_prefix("gamedata/sound_").is_some_and(|rest| rest.len() > 4 && rest.ends_with(".tdf") && !rest.contains('/'))
}

impl SoundCategories {
    pub const PATH: &'static str = "gamedata/sound.tdf";
    /// The section of a sound table holding its [sound sets](SoundSet), a section each.
    pub const SETS: &'static str = "SOUNDSETS";

    pub fn from_tdf(doc: &TdfDocument) -> SoundCategories {
        SoundCategories::from_tables(std::slice::from_ref(doc))
    }

    /// The tables in the order they are read (TA's, then its add-ons): each category's keys laid
    /// over those of its name before it, each set in place of one of its name before it.
    pub fn from_tables(docs: &[TdfDocument]) -> SoundCategories {
        let mut categories: HashMap<String, SoundCategory> = HashMap::new();
        let mut sets = SoundSets::default();
        for doc in docs {
            for section in &doc.sections {
                let name = section.name.trim().to_ascii_uppercase();
                if name == Self::SETS {
                    for set in &section.sections {
                        sets.insert(SoundSet::from_section(set));
                    }
                    continue;
                }
                match categories.get_mut(&name) {
                    Some(category) => category.merge(section),
                    None => {
                        categories.insert(name, SoundCategory::from_section(section));
                    }
                }
            }
        }
        // A category's `base`: what it does not give, its base's (and that one's base's...).
        let mut names: Vec<String> = categories.keys().cloned().collect();
        names.sort();
        for name in names {
            let mut chain: Vec<String> = Vec::new();
            let mut at = categories.get(&name).and_then(|c| c.base()).map(|b| b.trim().to_ascii_uppercase());
            while let Some(base) = at.and_then(|b| aliased(&categories, &b)) {
                if base == name || chain.contains(&base) || chain.len() >= 16 {
                    break;
                }
                at = categories.get(&base).and_then(|c| c.base()).map(|b| b.trim().to_ascii_uppercase());
                chain.push(base);
            }
            for base in chain {
                let Some(base) = categories.get(&base).cloned() else { break };
                if let Some(category) = categories.get_mut(&name) {
                    category.inherit(&base);
                }
            }
        }
        SoundCategories { categories, sets: Arc::new(sets), empty: SoundCategory::default() }
    }

    pub fn parse(bytes: &[u8]) -> SoundCategories {
        SoundCategories::from_tdf(&ta_formats::tdf::parse_bytes(bytes))
    }

    /// The sound tables `source` holds, in the order they are read: `gamedata/sound.tdf`, then
    /// the add-ons (`gamedata/sound_<name>.tdf`) by their paths.
    pub fn tables(source: &dyn SoundSource) -> Vec<String> {
        let mut add_ons: Vec<String> = source.list("gamedata").into_iter().map(|p| ta_vfs::normalize(&p)).filter(|p| is_sound_add_on(p)).collect();
        add_ons.sort();
        add_ons.dedup();
        std::iter::once(Self::PATH.to_string()).chain(add_ons).collect()
    }

    /// Loads `gamedata/sound.tdf` and its add-ons from a sound source (normally the VFS):
    /// [`SoundCategories::from_tables`]. Not found when there is none of them.
    pub fn load(source: &dyn SoundSource) -> Result<SoundCategories> {
        let docs: Vec<TdfDocument> = Self::tables(source).iter().filter_map(|path| source.read(path)).map(|bytes| ta_formats::tdf::parse_bytes(&bytes)).collect();
        if docs.is_empty() {
            return Err(AudioError::NotFound(Self::PATH.to_string()));
        }
        Ok(SoundCategories::from_tables(&docs))
    }

    /// The sound sets the tables give.
    pub fn sets(&self) -> &Arc<SoundSets> {
        &self.sets
    }

    /// A side's alerts: the category its side data names (`alerts`), else the one named
    /// `ALERTS_<side>` (`ALERTS_UEF`); none for a side without (TA's own).
    pub fn alerts(&self, side: &str, named: Option<&str>) -> Option<&SoundCategory> {
        if let Some(name) = named.map(str::trim).filter(|n| !n.is_empty()) {
            return self.try_category(name);
        }
        let side = side.trim();
        (!side.is_empty()).then(|| self.categories.get(&format!("ALERTS_{}", side.to_ascii_uppercase()))).flatten()
    }

    /// The category a unit's `SoundCategory=` names. Unknown names (including TA's `NONE`)
    /// return an empty category whose lookups yield `None`.
    pub fn category(&self, name: &str) -> &SoundCategory {
        self.try_category(name).unwrap_or(&self.empty)
    }

    /// Like [`category`](Self::category) but distinguishes unknown names. Falls back from
    /// `CORE_x` to `COR_x` and vice versa, repairing TA's inconsistent Core prefixes.
    pub fn try_category(&self, name: &str) -> Option<&SoundCategory> {
        let key = name.trim().to_ascii_uppercase();
        if let Some(c) = self.categories.get(&key) {
            return Some(c);
        }
        let alias = if let Some(rest) = key.strip_prefix("CORE_") {
            format!("COR_{rest}")
        } else {
            let rest = key.strip_prefix("COR_")?;
            format!("CORE_{rest}")
        };
        self.categories.get(&alias)
    }

    pub fn len(&self) -> usize {
        self.categories.len()
    }

    pub fn is_empty(&self) -> bool {
        self.categories.is_empty()
    }

    /// Category names (upper case, sorted).
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.categories.keys().map(|s| s.as_str()).collect();
        names.sort_unstable();
        names
    }

    /// Every sound name referenced by any category (lower case), and every file the sets play,
    /// for preloading/validation.
    pub fn referenced_sounds(&self) -> BTreeSet<String> {
        self.categories
            .values()
            .flat_map(|c| c.sounds.values().flatten().map(|(_, s)| s.to_ascii_lowercase()))
            .chain(self.sets.sets.values().flat_map(|s| s.variants.iter().map(|(file, _)| file.to_ascii_lowercase())))
            .collect()
    }
}

/// A sound set: a section of a sound table's `[SOUNDSETS]`, its name usable wherever a sound's
/// file name is (a unit's sound category's keys, a weapon's `soundstart`...). A name naming no
/// set is a file, as in TA.
///
/// ```text
/// [SOUNDSETS]
///     {
///     [TANK_FIRE]
///         {
///         sound1=tank_fire1;   // its variants: files (sounds/tank_fire1.wav), not sets
///         sound2=tank_fire2;
///         weight2=2;           // a variant's chance against the others' (1 where not given)
///         volume=0.7 1;        // its level, or a level at random between two (1: as recorded)
///         pitch=0.95 1.05;     // its rate, or a rate at random between two (2: an octave up)
///         priority=normal;     // low, normal or high, when voices run out
///         radius=1.5;          // how far it is heard, against an ordinary sound's 1
///         maxplaying=3;        // at most this many playing at once
///         norepeat=1;          // never the same variant twice in a row
///         fadeout=0.3;         // a loop of it stopped fades out over so many seconds
///         }
///     }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct SoundSet {
    /// As written.
    pub name: String,
    /// Its files and their weights, in the order of their numbers (`sound` alone first).
    pub variants: Vec<(String, u32)>,
    /// Linear levels and rates: one picked between the two (log-evenly for the rate).
    pub volume: (f32, f32),
    pub pitch: (f32, f32),
    pub priority: Option<Priority>,
    pub radius: f32,
    /// 0: no cap.
    pub max_playing: u16,
    pub no_repeat: bool,
    /// Seconds a loop of it fades out over as it is stopped (0: the engine's own, a moment).
    pub fade_out: f32,
    /// Its voices' group in the mixer (its caps, its merging): a hash of its name, never 0.
    pub group: u32,
}

impl Default for SoundSet {
    fn default() -> Self {
        SoundSet { name: String::new(), variants: Vec::new(), volume: (1.0, 1.0), pitch: (1.0, 1.0), priority: None, radius: 1.0, max_playing: 0, no_repeat: false, fade_out: 0.0, group: 1 }
    }
}

/// One number, or two (a range: `0.7 1`, `0.7,1`), each positive.
fn range(value: &str) -> Option<(f32, f32)> {
    let numbers: Vec<f32> = value.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).map(|s| s.parse::<f32>().ok()).collect::<Option<_>>()?;
    let (a, b) = match numbers[..] {
        [a] => (a, a),
        [a, b] => (a.min(b), a.max(b)),
        _ => return None,
    };
    (a.is_finite() && b.is_finite() && a > 0.0).then_some((a, b))
}

/// A set's `priority`: `low`, `normal` or `high` (or 0, 1, 2).
pub fn parse_priority(value: &str) -> Option<Priority> {
    match value.trim().to_ascii_lowercase().as_str() {
        "low" | "0" => Some(Priority::Low),
        "normal" | "1" => Some(Priority::Normal),
        "high" | "2" => Some(Priority::High),
        _ => None,
    }
}

impl SoundSet {
    /// The keys a set reads (each `sound<n>` and `weight<n>` with or without its number).
    pub const KEYS: [&'static str; 9] = ["sound", "weight", "volume", "pitch", "priority", "radius", "maxplaying", "norepeat", "fadeout"];

    pub fn from_section(section: &TdfSection) -> SoundSet {
        let name = section.name.trim().to_string();
        let mut files: BTreeMap<u32, String> = BTreeMap::new();
        let mut weights: BTreeMap<u32, u32> = BTreeMap::new();
        let mut set = SoundSet { group: group_of(&name), name, ..Default::default() };
        for (key, value) in &section.entries {
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();
            let digits_at = key.trim_end_matches(|c: char| c.is_ascii_digit()).len();
            let (base, digits) = key.split_at(digits_at);
            let number = if digits.is_empty() { 0 } else { digits.parse::<u32>().unwrap_or(u32::MAX) };
            match base {
                "sound" if !value.is_empty() => {
                    files.insert(number, value.to_string());
                }
                "weight" => {
                    if let Ok(w) = value.parse::<f64>() {
                        weights.insert(number, w.round().clamp(0.0, 1_000_000.0) as u32);
                    }
                }
                "volume" if digits.is_empty() => {
                    if let Some(r) = range(value) {
                        set.volume = r;
                    }
                }
                "pitch" if digits.is_empty() => {
                    if let Some(r) = range(value) {
                        set.pitch = r;
                    }
                }
                "priority" if digits.is_empty() => set.priority = parse_priority(value),
                "radius" if digits.is_empty() => {
                    if let Ok(r) = value.parse::<f32>()
                        && r.is_finite()
                        && r > 0.0
                    {
                        set.radius = r.min(16.0);
                    }
                }
                "maxplaying" if digits.is_empty() => set.max_playing = value.parse::<f64>().map_or(0, |n| n.round().clamp(0.0, 1024.0) as u16),
                "norepeat" if digits.is_empty() => set.no_repeat = value.parse::<f64>().is_ok_and(|n| n != 0.0),
                "fadeout" if digits.is_empty() => {
                    if let Ok(f) = value.parse::<f32>()
                        && f.is_finite()
                    {
                        set.fade_out = f.clamp(0.0, 10.0);
                    }
                }
                _ => {}
            }
        }
        set.variants = files.into_iter().map(|(n, file)| (file, weights.get(&n).copied().unwrap_or(1))).filter(|(_, w)| *w > 0).collect();
        set
    }

    /// The variant to play, by `random` (any number), `last` the one played before (passed over
    /// with `norepeat` where there are others): none for a set without variants.
    pub fn pick(&self, random: u32, last: Option<usize>) -> Option<usize> {
        let skip = if self.no_repeat && self.variants.len() > 1 { last } else { None };
        let total: u64 = self.variants.iter().enumerate().filter(|(i, _)| Some(*i) != skip).map(|(_, (_, w))| *w as u64).sum();
        if total == 0 {
            return None;
        }
        let mut r = random as u64 % total;
        for (i, (_, w)) in self.variants.iter().enumerate() {
            if Some(i) == skip {
                continue;
            }
            if r < *w as u64 {
                return Some(i);
            }
            r -= *w as u64;
        }
        None
    }

    /// The level for `u` (0 to 1) along its range.
    pub fn volume_at(&self, u: f32) -> f32 {
        let u = u.clamp(0.0, 1.0);
        self.volume.0 + (self.volume.1 - self.volume.0) * u
    }

    /// The rate for `u` (0 to 1) along its range, evenly in pitch (log-evenly in rate).
    pub fn pitch_at(&self, u: f32) -> f32 {
        let u = u.clamp(0.0, 1.0);
        self.pitch.0 * (self.pitch.1 / self.pitch.0).powf(u)
    }
}

/// A set's group: FNV-1a of its name in capitals, never 0.
fn group_of(name: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in name.trim().bytes() {
        h ^= b.to_ascii_uppercase() as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h.max(1)
}

/// The sound sets of the sound tables, by name.
#[derive(Clone, Debug, Default)]
pub struct SoundSets {
    /// Upper-case name → set.
    sets: HashMap<String, SoundSet>,
}

impl SoundSets {
    /// Adds a set, in place of one of its name.
    pub fn insert(&mut self, set: SoundSet) {
        self.sets.insert(set.name.to_ascii_uppercase(), set);
    }

    /// The set a sound name names (any case), if any.
    pub fn get(&self, name: &str) -> Option<&SoundSet> {
        if self.sets.is_empty() {
            return None;
        }
        self.sets.get(&name.trim().to_ascii_uppercase())
    }

    pub fn len(&self) -> usize {
        self.sets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    /// The sets, in the order of their names.
    pub fn iter(&self) -> impl Iterator<Item = &SoundSet> {
        let mut sets: Vec<&SoundSet> = self.sets.values().collect();
        sets.sort_by(|a, b| a.name.cmp(&b.name));
        sets.into_iter()
    }
}

/// Interface sounds from `gamedata/allsound.tdf`: `[EVENT] { sound=name; }`.
#[derive(Clone, Debug, Default)]
pub struct UiSounds {
    /// Upper-case event name → sound name.
    sounds: BTreeMap<String, String>,
}

impl UiSounds {
    pub const PATH: &'static str = "gamedata/allsound.tdf";

    pub fn from_tdf(doc: &TdfDocument) -> UiSounds {
        let sounds = doc
            .sections
            .iter()
            .filter_map(|s| {
                let sound = s.get("sound")?.trim();
                (!sound.is_empty()).then(|| (s.name.to_ascii_uppercase(), sound.to_string()))
            })
            .collect();
        UiSounds { sounds }
    }

    pub fn parse(bytes: &[u8]) -> UiSounds {
        UiSounds::from_tdf(&ta_formats::tdf::parse_bytes(bytes))
    }

    pub fn load(source: &dyn SoundSource) -> Result<UiSounds> {
        let bytes = source.read(Self::PATH).ok_or_else(|| AudioError::NotFound(Self::PATH.to_string()))?;
        Ok(UiSounds::parse(&bytes))
    }

    /// Sound for an interface event such as `"BIGBUTTON"` or `"NOMETAL"` (case-insensitive).
    pub fn get(&self, event: &str) -> Option<&str> {
        self.sounds.get(&event.trim().to_ascii_uppercase()).map(|s| s.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.sounds.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn len(&self) -> usize {
        self.sounds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sounds.is_empty()
    }
}

/// The sound keys of one weapon section (`weapons/*.tdf`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WeaponSounds {
    /// `soundstart`: played at the muzzle when the weapon fires.
    pub start: Option<String>,
    /// `soundhit`: played at the impact point when the projectile detonates.
    pub hit: Option<String>,
    /// `soundwater`: played instead of `hit` when the impact is on water.
    pub water: Option<String>,
    /// `soundtrigger=1`: play `start` once a burst, as its first shot leaves (TA's own say it for
    /// guns of a shot a burst; without it every shot plays it).
    pub trigger: bool,
    /// `soundstartwater`: played in place of `start` for a shot leaving from under the water.
    pub start_water: Option<String>,
    /// `soundloop`: on from the first shot while the shots keep coming (a beam's hum, a
    /// gatling's barrels), `soundstop` as it stops.
    pub looped: Option<String>,
    pub stop: Option<String>,
}

impl WeaponSounds {
    pub fn from_section(section: &TdfSection) -> WeaponSounds {
        let name = |key: &str| section.get(key).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        WeaponSounds {
            start: name("soundstart"),
            hit: name("soundhit"),
            water: name("soundwater"),
            trigger: section.get_bool_or("soundtrigger", false),
            start_water: name("soundstartwater"),
            looped: name("soundloop"),
            stop: name("soundstop"),
        }
    }

    /// The sound for an impact: `soundwater` on water when present, else `soundhit`.
    pub fn impact(&self, on_water: bool) -> Option<&str> {
        if on_water {
            self.water.as_deref().or(self.hit.as_deref())
        } else {
            self.hit.as_deref()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mod check reads a sound category's keys as the game plays them
    /// (`ta_data::validation::PLAYED_SOUND_KEYS`): every event's, no other.
    #[test]
    fn the_mod_check_knows_the_sounds_the_game_plays() {
        let mut ours: Vec<&str> = SoundEvent::SIMPLE.iter().map(|e| e.base_key()).collect();
        let mut check: Vec<&str> = ta_data::validation::PLAYED_SOUND_KEYS.to_vec();
        ours.sort_unstable();
        check.sort_unstable();
        assert_eq!(ours, check);
    }

    const SOUND_TDF: &str = "// unit sounds\n\
        [ARM_COM]\t\t// Ship\n\t{\n\tselect1=kcarmsel;\n\tunitcomplete=kcarmmov;\n\tok1=kcarmmov;\n\
        \tarrived1=kcarmstp;\n\tcant1=cantdo4;\n\tunderattack=warning2;\n\tbuild=nanlath1;\n\
        \trepair=repair1;\n\tworking=reclaim1;\n\tcloak=kloak1;\n\tuncloak=kloak1un;\n\
        \tcount5=count1;\n\tcount0=count6;\n\tcanceldestruct=cancel2;\n\t}\n\
        [COR_KBOT]\n\t{\n\tselect1=kbcorsel;\n\tselect2=kbcorsl2;\n\tselect=kbcorsl0;\n\tok1=kbcormov;\n\
        \t//working=mexrun;\n\tbogus=zzz;\n\t}\n\
        [CORE_TANK]\n\t{\n\tselect1=tcorsel;\n\t}\n";

    #[test]
    fn parses_sound_tdf_categories() {
        let cats = SoundCategories::parse(SOUND_TDF.as_bytes());
        assert_eq!(cats.len(), 3);
        let com = cats.category("arm_com");
        assert_eq!(com.name(), "ARM_COM");
        assert_eq!(com.get(SoundEvent::Select), Some("kcarmsel"));
        assert_eq!(com.get(SoundEvent::UnderAttack), Some("warning2"));
        assert_eq!(com.get(SoundEvent::Uncloak), Some("kloak1un"));
        assert_eq!(com.get(SoundEvent::Count(5)), Some("count1"));
        assert_eq!(com.get(SoundEvent::Count(0)), Some("count6"));
        assert_eq!(com.get(SoundEvent::Count(3)), None);
        assert_eq!(com.get(SoundEvent::Activate), None);

        let kbot = cats.category("COR_KBOT");
        let variants: Vec<&str> = kbot.variants(SoundEvent::Select).collect();
        assert_eq!(variants, ["kbcorsl0", "kbcorsel", "kbcorsl2"]);
        assert_eq!(kbot.pick(SoundEvent::Select, 4), Some("kbcorsel"));
        assert_eq!(kbot.get(SoundEvent::Working), None, "commented-out key");
        assert_eq!(kbot.extra(), &[("bogus".to_string(), "zzz".to_string())]);
        assert!(cats.referenced_sounds().contains("kbcorsl2"));
    }

    /// TA Reborn's keys parse as TA's do, numbered or not; the loops are loops, the alerts and
    /// spoken keys non-positional, the rest placed.
    #[test]
    fn the_new_keys_parse() {
        assert_eq!(SoundEvent::parse_key("moveloop"), Some((SoundEvent::MoveLoop, 0)));
        assert_eq!(SoundEvent::parse_key("Death2"), Some((SoundEvent::Death, 2)));
        assert_eq!(SoundEvent::parse_key("commanderattacked"), Some((SoundEvent::CommanderAttacked, 0)));
        for event in SoundEvent::SIMPLE {
            assert_eq!(SoundEvent::parse_key(&format!("{}3", event.key())), Some((event, 3)), "{event:?}");
        }
        assert!(SoundEvent::MoveLoop.is_loop() && SoundEvent::ActiveLoop.is_loop() && !SoundEvent::StartMove.is_loop());
        assert!(SoundEvent::NuclearLaunch.is_acknowledgement() && !SoundEvent::Death.is_acknowledgement() && !SoundEvent::DoneBuilt.is_acknowledgement());
    }

    /// A sound add-on's categories laid over the table's, key by key (its own added, the table's
    /// kept), its sets in place of those of their names; a table without the base file still
    /// reads.
    #[test]
    fn sound_add_ons_are_laid_over_the_table() {
        let mut source = crate::MemorySource::new();
        source.insert("gamedata/sound.tdf", SOUND_TDF.as_bytes().to_vec());
        source.insert("gamedata/sound_b.tdf", b"[ARM_COM]\n{\nselect1=mine;\ndeath=boom;\n}\n[SOUNDSETS]\n{\n[BOOM]\n{\nsound1=late;\n}\n}\n".to_vec());
        source.insert("gamedata/sound_a.tdf", b"[MYTANK]\n{\nmoveloop=engine;\n}\n[SOUNDSETS]\n{\n[BOOM]\n{\nsound1=early;\n}\n}\n".to_vec());
        source.insert("gamedata/sounds.tdf", b"[ARM_COM]\n{\nselect1=never;\n}\n".to_vec());
        assert_eq!(SoundCategories::tables(&source), ["gamedata/sound.tdf", "gamedata/sound_a.tdf", "gamedata/sound_b.tdf"]);
        let cats = SoundCategories::load(&source).unwrap();
        let com = cats.category("ARM_COM");
        assert_eq!(com.get(SoundEvent::Select), Some("mine"), "the add-on's key over the table's");
        assert_eq!(com.get(SoundEvent::Death), Some("boom"));
        assert_eq!(com.get(SoundEvent::UnderAttack), Some("warning2"), "the table's others kept");
        assert_eq!(cats.category("MYTANK").get(SoundEvent::MoveLoop), Some("engine"));
        assert_eq!(cats.sets().get("boom").unwrap().variants, [("late".to_string(), 1)], "the later add-on's set");
        assert!(cats.try_category(SoundCategories::SETS).is_none(), "the sets are no category");
        assert!(cats.referenced_sounds().contains("late"));
        assert!(is_sound_table("gamedata/sound.tdf") && is_sound_add_on("gamedata/sound_supcom.tdf"));
        assert!(!is_sound_add_on("gamedata/sound_.tdf") && !is_sound_add_on("gamedata/sounds.tdf") && !is_sound_add_on("gamedata/sound_x/y.tdf"));
        let mut only_add_on = crate::MemorySource::new();
        only_add_on.insert("gamedata/sound_x.tdf", b"[A]\n{\nselect1=a;\n}\n".to_vec());
        assert_eq!(SoundCategories::load(&only_add_on).unwrap().category("A").get(SoundEvent::Select), Some("a"));
        assert!(SoundCategories::load(&crate::MemorySource::new()).is_err());
    }

    /// A sound set's keys: its variants by their numbers with their weights (none at 0), its
    /// ranges (one number or two, either way round), priority, radius, cap, no repeats, fade.
    #[test]
    fn sound_sets_read_their_keys() {
        let doc = ta_formats::tdf::parse(
            "[SOUNDSETS]\n{\n[Fire]\n{\nsound2=b;\nweight2=3;\nsound1=a;\nsound3=c;\nweight3=0;\nvolume=1 0.5;\npitch=0.9,1.1;\npriority=High;\nradius=2;\nmaxplaying=3;\nnorepeat=1;\nfadeout=0.4;\n}\n\
             [Plain]\n{\nsound=x;\nvolume=-1;\nradius=0;\npriority=loud;\n}\n}\n",
        );
        let cats = SoundCategories::from_tdf(&doc);
        let fire = cats.sets().get("FIRE").unwrap();
        assert_eq!(fire.variants, [("a".to_string(), 1), ("b".to_string(), 3)], "c weighs nothing");
        assert_eq!((fire.volume, fire.pitch), ((0.5, 1.0), (0.9, 1.1)));
        assert_eq!((fire.priority, fire.radius, fire.max_playing, fire.no_repeat, fire.fade_out), (Some(Priority::High), 2.0, 3, true, 0.4));
        assert!((fire.pitch_at(0.5) - (0.99f32).sqrt()).abs() < 1e-5, "evenly in pitch");
        assert_eq!((fire.volume_at(0.0), fire.volume_at(1.0)), (0.5, 1.0));
        let plain = cats.sets().get("plain").unwrap();
        assert_eq!((plain.variants.len(), plain.volume, plain.radius, plain.priority), (1, (1.0, 1.0), 1.0, None), "nonsense is ignored");
        assert_ne!(fire.group, plain.group);
        assert!(fire.group != 0 && plain.group != 0);
    }

    /// Variants by their weights, the one before passed over with `norepeat` (never stuck where
    /// there is one alone).
    #[test]
    fn a_set_picks_by_weight_and_never_twice_running() {
        let set = SoundSet { variants: vec![("a".into(), 1), ("b".into(), 3)], ..Default::default() };
        let mut counts = [0; 2];
        for r in 0..4000u32 {
            counts[set.pick(r.wrapping_mul(2_654_435_761), None).unwrap()] += 1;
        }
        assert!((counts[1] as f32 / counts[0] as f32 - 3.0).abs() < 0.2, "{counts:?}");
        let set = SoundSet { variants: vec![("a".into(), 1), ("b".into(), 1), ("c".into(), 1)], no_repeat: true, ..Default::default() };
        let mut last = None;
        for r in 0..300u32 {
            let pick = set.pick(r.wrapping_mul(2_654_435_761), last);
            assert!(pick.is_some() && pick != last);
            last = pick;
        }
        let one = SoundSet { variants: vec![("a".into(), 1)], no_repeat: true, ..Default::default() };
        assert_eq!(one.pick(7, Some(0)), Some(0));
        assert_eq!(SoundSet::default().pick(7, None), None);
    }

    /// A category's `base`: the keys it does not give, its base's (and that one's), TA's Core
    /// prefixes taken for each other; its own kept; a circle of bases ends.
    #[test]
    fn a_category_takes_what_it_lacks_from_its_base() {
        let cats = SoundCategories::parse(
            b"[MINE]\n{\nbase=CORE_KBOT;\nselect1=mine;\n}\n[COR_KBOT]\n{\nbase=ROOT;\nselect1=kbot;\nok1=kbot_ok;\n}\n[ROOT]\n{\ncount5=five;\nok1=root_ok;\n}\n\
              [LOOP_A]\n{\nbase=LOOP_B;\nselect1=a;\n}\n[LOOP_B]\n{\nbase=LOOP_A;\nok1=b;\n}\n",
        );
        let mine = cats.category("MINE");
        assert_eq!((mine.get(SoundEvent::Select), mine.get(SoundEvent::Ok), mine.get(SoundEvent::Count(5))), (Some("mine"), Some("kbot_ok"), Some("five")));
        assert_eq!(cats.category("LOOP_A").get(SoundEvent::Ok), Some("b"));
        assert_eq!(cats.category("LOOP_B").get(SoundEvent::Select), Some("a"));
    }

    /// A side's alerts: the category its side data names, else `ALERTS_<side>`; none for TA's.
    #[test]
    fn a_sides_alerts_are_found_by_name() {
        let cats = SoundCategories::parse(b"[ALERTS_UEF]\n{\ncommanderattacked=a;\n}\n[OTHER]\n{\nnuclearlaunch=b;\n}\n");
        assert_eq!(cats.alerts("uef", None).and_then(|c| c.get(SoundEvent::CommanderAttacked)), Some("a"));
        assert_eq!(cats.alerts("UEF", Some("other")).and_then(|c| c.get(SoundEvent::NuclearLaunch)), Some("b"));
        assert!(cats.alerts("ARM", None).is_none());
    }

    #[test]
    fn repairs_core_prefixes_and_handles_none() {
        let cats = SoundCategories::parse(SOUND_TDF.as_bytes());
        assert_eq!(cats.category("CORE_KBOT").get(SoundEvent::Ok), Some("kbcormov"));
        assert_eq!(cats.category("COR_TANK").get(SoundEvent::Select), Some("tcorsel"));
        assert!(cats.try_category("NONE").is_none());
        assert_eq!(cats.category("NONE").get(SoundEvent::Select), None);
        assert!(cats.category("ARM_NOTHING").is_empty());
    }

    #[test]
    fn parses_keys() {
        assert_eq!(SoundEvent::parse_key("SELECT1"), Some((SoundEvent::Select, 1)));
        assert_eq!(SoundEvent::parse_key("ok12"), Some((SoundEvent::Ok, 12)));
        assert_eq!(SoundEvent::parse_key("underattack"), Some((SoundEvent::UnderAttack, 0)));
        assert_eq!(SoundEvent::parse_key("count4"), Some((SoundEvent::Count(4), 0)));
        assert_eq!(SoundEvent::parse_key("count"), None);
        assert_eq!(SoundEvent::parse_key("selected"), None);
        assert_eq!(SoundEvent::Count(2).key(), "count2");
        assert!(SoundEvent::Select.is_acknowledgement() && !SoundEvent::Build.is_acknowledgement());
    }

    #[test]
    fn parses_allsound_and_weapons() {
        let ui = UiSounds::parse(
            b"[BIGBUTTON]\r\n\t{\r\n\tsound=butmain1;\r\n\t}\r\n[Victory Condition]\r\n\t{\r\n\tsound=victory2;\r\n\t}",
        );
        assert_eq!(ui.get("bigbutton"), Some("butmain1"));
        assert_eq!(ui.get("Victory Condition"), Some("victory2"));
        assert_eq!(ui.len(), 2);

        let doc = ta_formats::tdf::parse(
            "[ARM_LIGHTLASER]\n{\n soundtrigger=1;\n soundstart=lasrfir3;\n soundhit=lasrhit2;\n}\n\
             [METEOR]\n{\n soundhit=xplomed2;\n soundwater=splshbig;\n}\n",
        );
        let laser = WeaponSounds::from_section(doc.section("arm_lightlaser").unwrap());
        assert_eq!(laser.start.as_deref(), Some("lasrfir3"));
        assert!(laser.trigger && laser.looped.is_none() && laser.start_water.is_none());
        let beam = ta_formats::tdf::parse("[BEAM]\n{\nsoundloop=hum;\nsoundstop=off;\nsoundstartwater=bubble;\n}\n");
        let beam = WeaponSounds::from_section(beam.section("BEAM").unwrap());
        assert_eq!((beam.looped.as_deref(), beam.stop.as_deref(), beam.start_water.as_deref()), (Some("hum"), Some("off"), Some("bubble")));
        assert_eq!(laser.impact(true), Some("lasrhit2"));
        let meteor = WeaponSounds::from_section(doc.section("METEOR").unwrap());
        assert_eq!(meteor.impact(true), Some("splshbig"));
        assert_eq!(meteor.impact(false), Some("xplomed2"));
        assert!(!meteor.trigger && meteor.start.is_none());
    }
}
