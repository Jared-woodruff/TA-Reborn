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
//! * [`UiSounds`] — `gamedata/allsound.tdf`: interface events (`[BIGBUTTON] { sound=butmain1; }`).
//! * [`WeaponSounds`] — the sound keys of a weapon TDF section.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ta_formats::tdf::{TdfDocument, TdfSection};

use crate::bank::SoundSource;
use crate::{AudioError, Result};

/// A unit event that can have a sound in `gamedata/sound.tdf`.
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
}

impl SoundEvent {
    /// Every event except the countdown, in `sound.tdf` key order.
    pub const SIMPLE: [SoundEvent; 15] = [
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
        }
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
    /// bus. Everything else (nanolathes, toggles, cloaking) is a positional effect.
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
        for (key, value) in &section.entries {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match SoundEvent::parse_key(key) {
                Some((event, variant)) => {
                    let list = category.sounds.entry(event).or_default();
                    // A repeated key overrides (last assignment wins, as in TA).
                    list.retain(|(v, _)| *v != variant);
                    list.push((variant, value.to_string()));
                    list.sort_by_key(|(v, _)| *v);
                }
                None => category.extra.push((key.clone(), value.to_string())),
            }
        }
        category
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
}

/// All unit sound categories from `gamedata/sound.tdf`.
#[derive(Clone, Debug, Default)]
pub struct SoundCategories {
    /// Upper-case section name → category.
    categories: HashMap<String, SoundCategory>,
    empty: SoundCategory,
}

impl SoundCategories {
    pub const PATH: &'static str = "gamedata/sound.tdf";

    pub fn from_tdf(doc: &TdfDocument) -> SoundCategories {
        let mut categories = HashMap::new();
        for section in &doc.sections {
            categories.insert(section.name.to_ascii_uppercase(), SoundCategory::from_section(section));
        }
        SoundCategories { categories, empty: SoundCategory::default() }
    }

    pub fn parse(bytes: &[u8]) -> SoundCategories {
        SoundCategories::from_tdf(&ta_formats::tdf::parse_bytes(bytes))
    }

    /// Loads `gamedata/sound.tdf` from a sound source (normally the VFS).
    pub fn load(source: &dyn SoundSource) -> Result<SoundCategories> {
        let bytes = source.read(Self::PATH).ok_or_else(|| AudioError::NotFound(Self::PATH.to_string()))?;
        Ok(SoundCategories::parse(&bytes))
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

    /// Every sound name referenced by any category (lower case), for preloading/validation.
    pub fn referenced_sounds(&self) -> BTreeSet<String> {
        self.categories
            .values()
            .flat_map(|c| c.sounds.values().flatten().map(|(_, s)| s.to_ascii_lowercase()))
            .collect()
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
    /// `soundtrigger=1`: play `start` for every shot of a burst, not only the first.
    pub trigger: bool,
}

impl WeaponSounds {
    pub fn from_section(section: &TdfSection) -> WeaponSounds {
        let name = |key: &str| section.get(key).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        WeaponSounds {
            start: name("soundstart"),
            hit: name("soundhit"),
            water: name("soundwater"),
            trigger: section.get_bool_or("soundtrigger", false),
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
        assert!(laser.trigger);
        assert_eq!(laser.impact(true), Some("lasrhit2"));
        let meteor = WeaponSounds::from_section(doc.section("METEOR").unwrap());
        assert_eq!(meteor.impact(true), Some("splshbig"));
        assert_eq!(meteor.impact(false), Some("xplomed2"));
        assert!(!meteor.trigger && meteor.start.is_none());
    }
}
