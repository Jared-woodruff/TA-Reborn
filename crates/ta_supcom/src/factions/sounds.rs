//! Forged Alliance's sounds for its factions' units (docs/SUPCOM_UNITS.md, "Sounds"), made from the
//! player's own copy's banks (`sounds/` and `sounds/Voice/US/`, read by [`crate::xact`]) into the
//! mod, with TA Reborn's sound keys (docs/MODDING.md, "Sounds"):
//!
//! - **Sets.** Each cue a blueprint names (its `Audio` table's `Sound { Bank, Cue }`) becomes a
//!   sound set, `FA_<BANK>_<CUE>`, in the mod's sound add-on ([`TABLE`]): its variants the cue's
//!   waves, its level and pitch Forged Alliance's (the sound's, its clip's and its categories',
//!   with the ranges its event picks from each time), its cap the cue's instance limit, no repeats
//!   where the cue never plays a wave twice running, its reach from the curve its volume follows
//!   by distance. A cue of several layers plays them mixed into a file a variant, each layer
//!   after its delay.
//! - **Files.** A wave becomes an IMA ADPCM WAV of its own (`sounds/supcom/<wave bank>/<wave>.wav`,
//!   [`crate::ima`]), written once whatever uses it; a delay its leading silence; stereo made mono
//!   (it pans where it plays). Whole numbers alone, in a fixed order: the same bytes on every
//!   computer.
//! - **Categories.** A unit's own (`SUPCOM_<ID>`): its selection, death, moves, building,
//!   reclaiming, capturing, work and shield's sounds, and what it does not give, TA's category for
//!   its kind's (`base`: its replies, its countdown).
//! - **Weapons.** Their keys: firing, a beam's start once a burst (`soundtrigger`), the loops of
//!   beams and barrels (`soundloop`, `soundstop`), firing under the water, the projectile's impacts
//!   on land and water.
//! - **The announcer.** Each side's alerts (`ALERTS_<side>`): the commander under attack (the cue
//!   the game's own `lua/aibrain.lua` plays for it, read as data) and a strategic missile launched
//!   (the cue the silos' blueprints name).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::files::Files;
use crate::lua::Table;
use crate::unit::{Motion, UnitBlueprint, WeaponBlueprint};
use crate::xact::{self, Codec, Layer, Settings, SoundBank, WaveBank};
use crate::{Error, Result};

/// The mod's sound add-on.
pub const TABLE: &str = "gamedata/sound_supcom.tdf";
/// Where its waves are.
const FOLDER: &str = "sounds/supcom";
/// Seconds a loop fades out over as it stops.
const LOOP_FADE: &str = "0.25";

/// The copy's banks by their own names (lower case): its sound banks, read whole, its wave banks'
/// heads with their files, and its global settings.
#[derive(Clone, Debug, Default)]
pub struct Banks {
    sound: BTreeMap<String, SoundBank>,
    wave: BTreeMap<String, (PathBuf, WaveBank)>,
    settings: Settings,
    /// Hundredths of a decibel the sets' levels are taken down by: the weapons' categories' (theirs
    /// at TA's own sounds' level).
    reference: i32,
    /// How far the farthest-reaching distance curve reaches (to 30 dB down): the sets' reach
    /// measured against it.
    farthest: f64,
}

/// A file's bytes, `path` in error messages.
fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| Error::io(path, e))
}

impl Banks {
    /// The banks of the copy whose `sounds` folder is `sounds` (and its `Voice/US`), in the order
    /// of their files' names, the first of a name kept.
    pub fn read(sounds: &Path) -> Result<Banks> {
        let mut dirs = vec![sounds.to_path_buf()];
        if let Some(us) = ta_vfs::install::find_dir_ci(sounds, "Voice").and_then(|v| ta_vfs::install::find_dir_ci(&v, "US")) {
            dirs.push(us);
        }
        let mut banks = Banks::default();
        for dir in dirs {
            let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map_err(|e| Error::io(&dir, e))?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_file()).collect();
            files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()));
            for path in files {
                let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                let stem = path.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
                match ext.as_str() {
                    "xsb" => {
                        let Ok(bank) = SoundBank::read(&read(&path)?) else { continue };
                        let name = if bank.name.is_empty() { stem } else { bank.name.to_ascii_lowercase() };
                        banks.sound.entry(name).or_insert(bank);
                    }
                    "xwb" => {
                        let Some(bank) = wave_bank_head(&path)? else { continue };
                        let name = if bank.name.is_empty() { stem } else { bank.name.to_ascii_lowercase() };
                        banks.wave.entry(name).or_insert((path, bank));
                    }
                    "xgs" if banks.settings.categories.is_empty() => {
                        if let Ok(settings) = Settings::read(&read(&path)?) {
                            banks.settings = settings;
                        }
                    }
                    _ => {}
                }
            }
        }
        if banks.sound.is_empty() {
            return Err(Error::Missing(format!("{}: no sound banks", sounds.display())));
        }
        let weapons = banks.settings.categories.iter().position(|c| c.name.eq_ignore_ascii_case("Weapons"));
        banks.reference = weapons.map_or(0, |i| banks.settings.category_centibels(i as u16));
        banks.farthest = banks.settings.curves.iter().filter(|c| c.variable.eq_ignore_ascii_case("Distance") && c.parameter == 0).filter_map(reach).fold(0.0, f64::max);
        Ok(banks)
    }

    /// Whether the copy has a cue `cue` in a sound bank `bank` (any case).
    pub fn has_cue(&self, bank: &str, cue: &str) -> bool {
        self.cue(bank, cue).is_some()
    }

    /// How many sound banks and wave banks were read, and whether the global settings were.
    pub fn counts(&self) -> (usize, usize, bool) {
        (self.sound.len(), self.wave.len(), !self.settings.categories.is_empty())
    }

    /// The cue `cue` of the sound bank `bank` (any case), with its bank.
    fn cue(&self, bank: &str, cue: &str) -> Option<(&SoundBank, &xact::Cue)> {
        let b = self.sound.get(&bank.to_ascii_lowercase())?;
        Some((b, b.cue(cue)?))
    }

    /// The wave of sound bank `bank`'s wave bank `index` (its list's), number `wave`: its wave
    /// bank's name, where it is 16-bit PCM with samples.
    fn wave(&self, bank: &SoundBank, index: u8, wave: u16) -> Option<String> {
        let name = bank.wave_banks.get(index as usize)?.to_ascii_lowercase();
        let (_, waves) = self.wave.get(&name)?;
        let w = waves.waves.get(wave as usize)?;
        (w.codec == Codec::Pcm && w.bits == 16 && (1..=2).contains(&w.channels) && w.frames > 0 && w.rate > 0).then_some(name)
    }

    /// How far a sound is heard, against an ordinary one's 1, by the curve its volume follows by
    /// distance: as far as one of the farthest-reaching curves (a weapon's, a blast's), not much
    /// less (a unit's moves), less (a building at work), least (a builder's loop); farther than any
    /// without one or one that never falls 30 dB (a missile in flight, a nuke).
    fn radius(&self, sound: &xact::Sound) -> &'static str {
        let curve = sound.curves.iter().filter_map(|&at| self.settings.curve(at)).find(|c| c.variable.eq_ignore_ascii_case("Distance") && c.parameter == 0);
        let Some(curve) = curve else { return "1.25" };
        match reach(curve) {
            None => "1.25",
            Some(_) if self.farthest <= 0.0 => "1",
            Some(d) => match d / self.farthest {
                r if r >= 0.6 => "1",
                r if r >= 0.2 => "0.8",
                r if r >= 0.1 => "0.65",
                _ => "0.5",
            },
        }
    }
}

/// A wave bank's head: its first 64 KiB, or the whole file where its entries go on past them.
fn wave_bank_head(path: &Path) -> Result<Option<WaveBank>> {
    use std::io::Read;
    let mut head = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(1 << 16).read_to_end(&mut head)).map_err(|e| Error::io(path, e))?;
    if let Ok(bank) = WaveBank::read(&head) {
        return Ok(Some(bank));
    }
    Ok(WaveBank::read(&read(path)?).ok())
}

/// Where a volume curve by distance falls 30 dB (along its points, in straight lines between
/// them): none where it never does.
fn reach(curve: &xact::Curve) -> Option<f64> {
    const DOWN: f64 = -3000.0;
    let points: Vec<(f64, f64)> = curve.points.iter().map(|&(x, y)| (x as f64, y as f64)).collect();
    if points.first().is_some_and(|p| p.1 <= DOWN) {
        return Some(points[0].0.max(0.0));
    }
    points.windows(2).find(|w| w[1].1 <= DOWN).map(|w| {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if y1 == y0 { x1 } else { x0 + (x1 - x0) * (DOWN - y0) / (y1 - y0) }
    })
}

/// 10^(n / 20) for each whole decibel from -96 to +24, in millionths: [`amplitude`]'s table.
const AMPLITUDE_PER_DECIBEL: [u32; 121] = [
    16, 18, 20, 22, 25, 28, 32, 35, 40, 45, 50, 56, 63, 71, 79, 89, 100, 112, 126, 141, 158, 178, 200, 224, 251, 282, 316, 355, 398, 447, 501, 562, 631, 708, 794, 891, 1000,
    1122, 1259, 1413, 1585, 1778, 1995, 2239, 2512, 2818, 3162, 3548, 3981, 4467, 5012, 5623, 6310, 7079, 7943, 8913, 10000, 11220, 12589, 14125, 15849, 17783, 19953, 22387,
    25119, 28184, 31623, 35481, 39811, 44668, 50119, 56234, 63096, 70795, 79433, 89125, 100000, 112202, 125893, 141254, 158489, 177828, 199526, 223872, 251189, 281838, 316228,
    354813, 398107, 446684, 501187, 562341, 630957, 707946, 794328, 891251, 1000000, 1122018, 1258925, 1412538, 1584893, 1778279, 1995262, 2238721, 2511886, 2818383, 3162278,
    3548134, 3981072, 4466836, 5011872, 5623413, 6309573, 7079458, 7943282, 8912509, 10000000, 11220185, 12589254, 14125375, 15848932,
];

/// A gain in hundredths of a decibel as an amplitude in thousandths (1000: as recorded), from a
/// table of whole decibels, in straight lines between them (-96 dB to +24 at most).
pub fn amplitude(centibels: i32) -> u32 {
    let c = centibels.clamp(-9600, 2400) + 9600;
    let (whole, part) = ((c / 100) as usize, (c % 100) as u64);
    let low = AMPLITUDE_PER_DECIBEL[whole] as u64;
    let high = AMPLITUDE_PER_DECIBEL[(whole + 1).min(120)] as u64;
    ((low * (100 - part) + high * part + 50_000) / 100_000) as u32
}

/// 2^(n / 12) for each semitone from -48 to +48, in millionths: [`rate`]'s table.
const RATE_PER_SEMITONE: [u32; 97] = [
    62500, 66216, 70154, 74325, 78745, 83427, 88388, 93644, 99213, 105112, 111362, 117984, 125000, 132433, 140308, 148651, 157490, 166855, 176777, 187288, 198425, 210224,
    222725, 235969, 250000, 264866, 280616, 297302, 314980, 333710, 353553, 374577, 396850, 420448, 445449, 471937, 500000, 529732, 561231, 594604, 629961, 667420, 707107,
    749154, 793701, 840896, 890899, 943874, 1000000, 1059463, 1122462, 1189207, 1259921, 1334840, 1414214, 1498307, 1587401, 1681793, 1781797, 1887749, 2000000, 2118926,
    2244924, 2378414, 2519842, 2669680, 2828427, 2996614, 3174802, 3363586, 3563595, 3775497, 4000000, 4237852, 4489848, 4756828, 5039684, 5339359, 5656854, 5993228,
    6349604, 6727171, 7127190, 7550995, 8000000, 8475705, 8979696, 9513657, 10079368, 10678719, 11313708, 11986457, 12699208, 13454343, 14254379, 15101989, 16000000,
];

/// A pitch in hundredths of a semitone as a playback rate in thousandths (1000: as recorded), from
/// a table of whole semitones, in straight lines between them (four octaves each way at most).
pub fn rate(cents: i32) -> u32 {
    let c = cents.clamp(-4800, 4800) + 4800;
    let (whole, part) = ((c / 100) as usize, (c % 100) as u64);
    let low = RATE_PER_SEMITONE[whole] as u64;
    let high = RATE_PER_SEMITONE[(whole + 1).min(96)] as u64;
    ((low * (100 - part) + high * part + 50_000) / 100_000) as u32
}

/// Thousandths as a number of a sound table: `0.631`, `1`.
fn thousandths(v: u32) -> String {
    let mut s = format!("{}.{:03}", v / 1000, v % 1000);
    while s.ends_with('0') {
        s.pop();
    }
    s.trim_end_matches('.').to_string()
}

/// A name of letters, digits and underscores (any other mark an underscore).
fn clean(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

/// Where a cue's sound comes from: a wave of a wave bank (its name, lower case), with silence
/// before it (milliseconds); or layers mixed.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Source {
    Wave { bank: String, wave: u16, delay: u16 },
    Mix(Vec<Part>),
}

/// A layer of a mix: its wave, its delay, its gain (thousandths) and the times it plays.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Part {
    bank: String,
    wave: u16,
    delay: u16,
    gain: u32,
    times: u16,
}

/// A sound set made of a cue.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Set {
    /// Its files (sound names: their paths without `.wav`) and their weights.
    variants: Vec<(String, u32)>,
    volume: (u32, u32),
    pitch: (u32, u32),
    radius: &'static str,
    max_playing: u8,
    no_repeat: bool,
    looped: bool,
}

/// The sounds the mod is made of: the sets and the files they play, the units' categories, the
/// weapons' keys, and what the blueprints name that the copy has not.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    sets: BTreeMap<String, Set>,
    files: BTreeMap<String, Source>,
    cues: BTreeMap<(String, String), Option<String>>,
    categories: BTreeMap<String, String>,
    alerts: Vec<(&'static str, String)>,
    /// Each weapon's keys, by its section's name ([`super::units::weapon_name`]).
    pub weapons: BTreeMap<String, Vec<(&'static str, String)>>,
    /// The (bank, cue) the blueprints name whose sound the copy does not have (a bank, a cue or
    /// its waves missing).
    pub missing: BTreeSet<(String, String)>,
}

/// A blueprint's sound for `key` (its `Audio` table's `Sound { Bank, Cue }`).
fn named(audio: Option<&Table>, key: &str) -> Option<(String, String)> {
    let sound = audio?.get_ci(key)?.table()?;
    let bank = sound.get_ci("Bank")?.str()?.trim();
    let cue = sound.get_ci("Cue")?.str()?.trim();
    (!bank.is_empty() && !cue.is_empty()).then(|| (bank.to_string(), cue.to_string()))
}

/// A unit's keys from its blueprint's (the first of each list it gives), each a loop or not.
fn unit_keys(u: &UnitBlueprint, scripted: bool) -> Vec<(&'static str, &'static [&'static str], bool)> {
    let air = u.motion == Motion::Air;
    let mut keys: Vec<(&'static str, &'static [&'static str], bool)> = vec![
        ("select1", &["UISelection"], false),
        ("death", if air { &["Killed", "DeathExplosion", "Destroyed"] } else { &["Destroyed", "DeathExplosion", "Killed"] }, false),
        ("startmove", &["StartMove", "StartMoveSub", "StartMoveLand", "StartMoveWater"], false),
        ("stopmove", &["StopMove", "StopMoveSub", "StopMoveLand", "StopMoveWater"], false),
        ("moveloop", &["AmbientMove", "AmbientMoveSub", "AmbientMoveLand", "AmbientMoveWater"], true),
        ("takeoff", &["TakeOff", "Thruster"], false),
        ("landing", &["Landing"], false),
        ("build", &["Construct"], false),
        ("buildloop", &["ConstructLoop"], true),
        ("buildstop", &["ConstructStop"], false),
        ("working", &["StartReclaim"], false),
        ("reclaimloop", &["ReclaimLoop"], true),
        ("captureloop", &["CaptureLoop"], true),
        ("donebuilt", &["DoneBeingBuilt"], false),
        ("activeloop", &["ActiveLoop"], true),
        ("shieldon", &["ShieldOn"], false),
        ("shieldoff", &["ShieldOff"], false),
    ];
    if air {
        keys.push(("crash", &["Destroyed", "AirUnitWaterImpact"], false));
    }
    // (A unit whose weapon unpacks plays its opening and closing in its script, as it unpacks.)
    if !scripted {
        keys.push(("open", &["Open", "Unpack"], false));
        keys.push(("close", &["Close", "Pack"], false));
    }
    keys
}

impl Plan {
    /// The set of the cue `bank`'s `cue`, made once: its name, or none where the copy has not its
    /// sound. A `voice` (spoken over the battle: a unit's selection, the announcer) is as loud as
    /// its sound says, its categories left out (the battle's are measured against the weapons').
    fn set(&mut self, banks: &Banks, bank: &str, cue: &str, voice: bool) -> Option<String> {
        let key = (bank.to_ascii_lowercase(), cue.to_ascii_lowercase());
        if let Some(made) = self.cues.get(&key) {
            return made.clone();
        }
        let made = self.make_set(banks, bank, cue, voice);
        if made.is_none() {
            self.missing.insert((bank.to_string(), cue.to_string()));
        }
        self.cues.insert(key, made.clone());
        made
    }

    fn make_set(&mut self, banks: &Banks, bank: &str, cue_name: &str, voice: bool) -> Option<String> {
        let (sb, cue) = banks.cue(bank, cue_name)?;
        let sound = cue.sound.as_ref()?;
        let layers: Vec<&Layer> = sound.layers.iter().filter(|l| !l.waves.is_empty() && l.clip_volume > 0).collect();
        if layers.is_empty() {
            return None;
        }
        let name = format!("FA_{}_{}", clean(&sb.name), clean(&cue.name)).to_ascii_uppercase();
        let level = xact::decibels(sound.volume) + if voice { 0 } else { banks.settings.category_centibels(sound.category) - banks.reference };
        let radius = if voice { "1" } else { banks.radius(sound) };
        let pitch = sound.pitch as i32;
        let pitch_of = |l: &Layer| match l.pitch_range {
            Some((lo, hi)) => (rate(pitch + lo as i32), rate(pitch + hi as i32)),
            None => (rate(pitch), rate(pitch)),
        };
        let max_playing = if cue.instance_limit < 0xFF { cue.instance_limit } else { 0 };
        let set = if let [layer] = layers[..] {
            let delay = if layer.looped() { 0 } else { layer.delay };
            let mut variants = Vec::new();
            for &(index, wave, weight) in &layer.waves {
                let Some(wave_bank) = banks.wave(sb, index, wave) else { continue };
                let path = format!("{FOLDER}/{}/{wave:04}{}", clean(&wave_bank), if delay > 0 { format!("_d{delay}") } else { String::new() });
                self.files.entry(path.clone()).or_insert(Source::Wave { bank: wave_bank, wave, delay });
                variants.push((path, (weight as u32).max(1)));
            }
            if variants.is_empty() {
                return None;
            }
            let level = level + xact::decibels(layer.clip_volume);
            let volume = match layer.volume_range {
                Some((lo, hi)) => (amplitude(level + xact::decibels(lo)), amplitude(level + xact::decibels(hi))),
                None => (amplitude(level), amplitude(level)),
            };
            let no_repeat = layer.no_repeat() && variants.len() > 1;
            // (Weights all alike: none written.)
            if variants.iter().all(|v| v.1 == variants[0].1) {
                variants.iter_mut().for_each(|v| v.1 = 1);
            }
            Set { variants, volume, pitch: pitch_of(layer), radius, max_playing, no_repeat, looped: layer.looped() }
        } else {
            // Layers mixed into a file a variant (as many as its layer of most waves has), each at
            // its loudest against the loudest of them, after its delay, as many times as it plays.
            let looped = layers.iter().all(|l| l.looped());
            let loudest = |l: &Layer| xact::decibels(l.clip_volume) + l.volume_range.map_or(0, |(_, hi)| xact::decibels(hi));
            let top = layers.iter().map(|l| loudest(l)).max().unwrap_or(0);
            let count = layers.iter().map(|l| l.waves.len()).max().unwrap_or(0);
            let mut variants = Vec::new();
            for k in 0..count {
                let parts: Vec<Part> = layers
                    .iter()
                    .filter_map(|l| {
                        let (index, wave, _) = l.waves[k % l.waves.len()];
                        let bank = banks.wave(sb, index, wave)?;
                        let times = if l.looped() { 1 } else { l.loop_count as u16 + 1 };
                        Some(Part { bank, wave, delay: if looped { 0 } else { l.delay }, gain: amplitude(loudest(l) - top), times })
                    })
                    .collect();
                if parts.is_empty() {
                    continue;
                }
                let path = format!("{FOLDER}/mix/{}_{}_{k}", clean(&sb.name), clean(&cue.name)).to_ascii_lowercase();
                self.files.insert(path.clone(), Source::Mix(parts));
                variants.push((path, 1));
            }
            if variants.is_empty() {
                return None;
            }
            let level = level + top;
            // (Its level picked from the range of the first layer that varies it, below its loudest;
            // its pitch the first's that varies it.)
            let volume = match layers.iter().find_map(|l| l.volume_range) {
                Some((lo, hi)) => (amplitude(level + xact::decibels(lo) - xact::decibels(hi)), amplitude(level)),
                None => (amplitude(level), amplitude(level)),
            };
            let pitch = layers.iter().find(|l| l.pitch_range.is_some()).map_or((rate(pitch), rate(pitch)), |l| pitch_of(l));
            Set { variants, volume, pitch, radius, max_playing, no_repeat: false, looped }
        };
        self.sets.insert(name.clone(), set);
        Some(name)
    }

    /// The set for the first of `keys` the `audio` table names that the copy has, a loop where
    /// `looped` (a one-shot otherwise): none where none.
    fn first(&mut self, banks: &Banks, audio: Option<&Table>, keys: &[&str], looped: bool) -> Option<String> {
        self.first_of(banks, audio, keys, looped, false)
    }

    /// [`Plan::first`], a voice where `voice` ([`Plan::set`]).
    fn first_of(&mut self, banks: &Banks, audio: Option<&Table>, keys: &[&str], looped: bool, voice: bool) -> Option<String> {
        for key in keys {
            if let Some((bank, cue)) = named(audio, key)
                && let Some(set) = self.set(banks, &bank, &cue, voice)
                && self.sets.get(&set).is_some_and(|s| s.looped == looped)
            {
                return Some(set);
            }
        }
        None
    }

    /// Unit `u`'s own sound category (`SUPCOM_<ID>`), where its blueprint gives it a sound the
    /// copy has: its keys, TA's category for its kind (`base`) for those it gives none of; and its
    /// weapons' keys. None (its weapons' TA's) where it gives none.
    pub fn unit(&mut self, banks: &Banks, u: &UnitBlueprint, base: &str) -> Option<String> {
        let audio = u.table.path(&["Audio"]);
        let scripted = u.weapons.iter().any(|w| w.unpack.as_ref().is_some_and(|p| p.animation.is_some()));
        let mut section = String::new();
        for (key, fa, looped) in unit_keys(u, scripted) {
            if let Some(set) = self.first_of(banks, audio, fa, looped, key == "select1") {
                let _ = writeln!(section, "\t{key}={set};");
            }
        }
        for w in &u.weapons {
            let keys = self.weapon(banks, w);
            if !keys.is_empty() {
                self.weapons.insert(super::units::weapon_name(u, w), keys);
            }
        }
        // (A silo's strategic launch: what its blueprint names the announcer says.)
        if self.alerts.iter().all(|(k, _)| *k != "nuclearlaunch")
            && let Some(set) = self.first_of(banks, audio, &["NuclearLaunchDetected"], false, true)
        {
            self.alerts.push(("nuclearlaunch", set));
        }
        if section.is_empty() {
            return None;
        }
        let name = format!("SUPCOM_{}", u.id.to_ascii_uppercase());
        self.categories.insert(name.clone(), format!("[{name}]\n\t{{\n\tbase={base};\n{section}\t}}\n"));
        Some(name)
    }

    /// What weapon `w` plays to unpack and pack before it fires (its own `Unpack` or `Open`, else
    /// its unit's; `Pack` or `Close`): the sets, for its script's `play-sound`.
    pub fn unpacking(&mut self, banks: &Banks, u: &UnitBlueprint, w: &WeaponBlueprint) -> (Option<String>, Option<String>) {
        let (own, unit) = (w.table.path(&["Audio"]), u.table.path(&["Audio"]));
        let open = self.first(banks, own, &["Unpack", "Open"], false).or_else(|| self.first(banks, unit, &["Unpack", "Open"], false));
        let close = self.first(banks, own, &["Pack", "Close"], false).or_else(|| self.first(banks, unit, &["Pack", "Close"], false));
        (open, close)
    }

    /// A weapon's keys: firing (a beam's start once a burst), its loop and its end, firing from
    /// under the water, its projectile's impacts.
    fn weapon(&mut self, banks: &Banks, w: &WeaponBlueprint) -> Vec<(&'static str, String)> {
        let audio = w.table.path(&["Audio"]);
        let mut keys = Vec::new();
        let beam_start = self.first(banks, audio, &["BeamStart"], false);
        let start = self.first(banks, audio, &["Fire", "MuzzleChargeStart"], false).or_else(|| beam_start.clone());
        if let Some(start) = start {
            // (A beam's start, or a salvo leaving together: once a burst.)
            let together = w.salvo_delay <= 0.0 && w.salvo_size > 1;
            if beam_start.is_some() || together {
                keys.push(("soundtrigger", "1".to_string()));
            }
            keys.push(("soundstart", start));
        }
        if let Some(looped) = self.first(banks, audio, &["BeamLoop", "BarrelLoop"], true) {
            keys.push(("soundloop", looped));
            if let Some(stop) = self.first(banks, audio, &["BeamStop", "BarrelStop"], false) {
                keys.push(("soundstop", stop));
            }
        }
        if let Some(water) = self.first(banks, audio, &["FireUnderWater"], false) {
            keys.push(("soundstartwater", water));
        }
        let shot = w.shot.as_ref().and_then(|s| s.table.path(&["Audio"]));
        if let Some(hit) = self.first(banks, shot, &["NukeExplosion", "Impact", "ImpactTerrain"], false) {
            keys.push(("soundhit", hit.clone()));
            let water = self.first(banks, shot, &["ImpactWater"], false).unwrap_or(hit);
            keys.push(("soundwater", water));
        }
        keys
    }

    /// The commander under attack, as the game's announcer says it: the cue its `lua/aibrain.lua`
    /// plays for it (`OnPlayCommanderUnderAttackVO`), read as data.
    pub fn commander_alert(&mut self, banks: &Banks, files: &Files) -> Result<()> {
        let Some(bytes) = files.read("lua/aibrain.lua")? else { return Ok(()) };
        let text = String::from_utf8_lossy(&bytes);
        let Some(at) = text.find("OnPlayCommanderUnderAttackVO") else { return Ok(()) };
        let body = &text[at..text.len().min(at + 800)];
        let quoted = |key: &str| -> Option<String> {
            let k = body.find(key)?;
            let rest = &body[k + key.len()..];
            let open = rest.find(['\'', '"'])?;
            let quote = rest[open..].chars().next()?;
            let rest = &rest[open + 1..];
            Some(rest[..rest.find(quote)?].to_string())
        };
        if let (Some(bank), Some(cue)) = (quoted("Bank"), quoted("Cue"))
            && let Some(set) = self.set(banks, &bank, &cue, true)
        {
            self.alerts.retain(|(k, _)| *k != "commanderattacked");
            self.alerts.insert(0, ("commanderattacked", set));
        }
        Ok(())
    }

    /// The sound add-on: the units' categories, each side's alerts (`sides`), then the sets.
    pub fn table(&self, sides: &[&str]) -> String {
        let mut s = String::from("// Forged Alliance's sounds for its factions, made from the player's own copy of the game.\n");
        for section in self.categories.values() {
            s.push_str(section);
        }
        if !self.alerts.is_empty() {
            for side in sides {
                let _ = writeln!(s, "[ALERTS_{}]\n\t{{", side.to_ascii_uppercase());
                for (key, set) in &self.alerts {
                    let _ = writeln!(s, "\t{key}={set};");
                }
                s.push_str("\t}\n");
            }
        }
        s.push_str("[SOUNDSETS]\n\t{\n");
        for (name, set) in &self.sets {
            let _ = writeln!(s, "\t[{name}]\n\t\t{{");
            for (k, (file, weight)) in set.variants.iter().enumerate() {
                let _ = writeln!(s, "\t\tsound{}={file};", k + 1);
                if *weight != 1 {
                    let _ = writeln!(s, "\t\tweight{}={weight};", k + 1);
                }
            }
            let range = |(lo, hi): (u32, u32)| if lo == hi { thousandths(lo) } else { format!("{} {}", thousandths(lo), thousandths(hi)) };
            let _ = writeln!(s, "\t\tvolume={};", range(set.volume));
            if set.pitch != (1000, 1000) {
                let _ = writeln!(s, "\t\tpitch={};", range(set.pitch));
            }
            if set.radius != "1" {
                let _ = writeln!(s, "\t\tradius={};", set.radius);
            }
            if set.max_playing > 0 {
                let _ = writeln!(s, "\t\tmaxplaying={};", set.max_playing);
            }
            if set.no_repeat {
                s.push_str("\t\tnorepeat=1;\n");
            }
            if set.looped {
                let _ = writeln!(s, "\t\tfadeout={LOOP_FADE};");
            }
            s.push_str("\t\t}\n");
        }
        s.push_str("\t}\n");
        s
    }

    /// The files to write, by their paths in the mod, in order.
    pub fn files(&self) -> impl Iterator<Item = String> + '_ {
        self.files.keys().map(|p| format!("{p}.wav"))
    }

    /// How many files there are to write.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Each file's path in the mod and, for a wave of its own, its wave bank, wave and delay (none
    /// for a mix): what the files are made of, to hold them to it.
    pub fn sources(&self) -> impl Iterator<Item = (String, Option<(&str, u16, u16)>)> + '_ {
        self.files.iter().map(|(path, source)| {
            let wave = match source {
                Source::Wave { bank, wave, delay } => Some((bank.as_str(), *wave, *delay)),
                Source::Mix(_) => None,
            };
            (format!("{path}.wav"), wave)
        })
    }

    /// The sets made, by name, with their variants' files and whether each is a loop.
    pub fn sets(&self) -> impl Iterator<Item = (&str, Vec<&str>, bool)> + '_ {
        self.sets.iter().map(|(name, set)| (name.as_str(), set.variants.iter().map(|(f, _)| f.as_str()).collect(), set.looped))
    }

    /// The files' bytes, `chunk` at a time (encoded side by side, written in order), each handed
    /// to `put` with its path.
    pub fn write(&self, banks: &Banks, chunk: usize, put: &mut dyn FnMut(&str, &[u8]) -> Result<()>) -> Result<()> {
        use rayon::prelude::*;
        let all: Vec<(&String, &Source)> = self.files.iter().collect();
        for part in all.chunks(chunk.max(1)) {
            let made: Vec<Result<Vec<u8>>> = part.par_iter().map(|(_, source)| file(banks, source)).collect();
            for ((path, _), bytes) in part.iter().zip(made) {
                put(&format!("{path}.wav"), &bytes?)?;
            }
        }
        Ok(())
    }
}

/// The samples of wave `wave` of the wave bank `bank` (its name), mono (a stereo wave's channels
/// averaged), and its rate.
pub fn samples(banks: &Banks, bank: &str, wave: u16) -> Result<(Vec<i16>, u32)> {
    use std::io::{Read, Seek, SeekFrom};
    let (path, waves) = banks.wave.get(bank).ok_or_else(|| Error::Missing(format!("wave bank {bank}")))?;
    let w = waves.waves.get(wave as usize).ok_or_else(|| Error::Format(format!("{bank}: no wave {wave}")))?;
    let mut f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
    f.seek(SeekFrom::Start(w.offset as u64)).map_err(|e| Error::io(path, e))?;
    let mut data = vec![0u8; w.length];
    f.read_exact(&mut data).map_err(|e| Error::io(path, e))?;
    let pcm: Vec<i16> = data.as_chunks::<2>().0.iter().map(|&s| i16::from_le_bytes(s)).collect();
    let mono = if w.channels == 2 { pcm.as_chunks::<2>().0.iter().map(|f| ((f[0] as i32 + f[1] as i32) >> 1) as i16).collect() } else { pcm };
    Ok((mono, w.rate))
}

/// A file's bytes: its wave (after its silence), or its layers mixed (each at the first's rate,
/// taken in straight lines between samples; turned down as a whole where they would clip).
fn file(banks: &Banks, source: &Source) -> Result<Vec<u8>> {
    match source {
        Source::Wave { bank, wave, delay } => {
            let (pcm, rate) = samples(banks, bank, *wave)?;
            let lead = (*delay as u64 * rate as u64 / 1000) as usize;
            let mut out = vec![0i16; lead];
            out.extend_from_slice(&pcm);
            Ok(crate::ima::wav(&out, rate))
        }
        Source::Mix(parts) => {
            let mut rate = 0;
            let mut sum: Vec<i64> = Vec::new();
            for p in parts {
                let (pcm, own) = samples(banks, &p.bank, p.wave)?;
                if rate == 0 {
                    rate = own;
                }
                // (At the mix's rate: each sample of it the wave's at that time.)
                let len = (pcm.len() as u64 * rate as u64 / own as u64) as usize;
                let at = |i: usize| -> i64 {
                    let pos = i as u64 * own as u64;
                    let (k, frac) = ((pos / rate as u64) as usize, pos % rate as u64);
                    let a = pcm.get(k).copied().unwrap_or(0) as i64;
                    let b = pcm.get(k + 1).copied().unwrap_or(0) as i64;
                    a + (b - a) * frac as i64 / rate as i64
                };
                let start = (p.delay as u64 * rate as u64 / 1000) as usize;
                for t in 0..p.times as usize {
                    let offset = start + t * len;
                    if sum.len() < offset + len {
                        sum.resize(offset + len, 0);
                    }
                    for i in 0..len {
                        sum[offset + i] += at(i) * p.gain as i64 / 1000;
                    }
                }
            }
            let peak = sum.iter().map(|v| v.abs()).max().unwrap_or(0).max(1);
            let out: Vec<i16> = sum.iter().map(|&v| if peak > 32767 { (v * 32767 / peak) as i16 } else { v as i16 }).collect();
            Ok(crate::ima::wav(&out, rate.max(1)))
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::xact::tests::{Event, FixtureCue, cue_bank, settings, wave_bank};

    /// Whole numbers from tables: as recorded at 0 dB and no shift, half the amplitude at -6 dB
    /// (near enough), an octave up at 1200 cents, in straight lines between.
    #[test]
    fn levels_and_rates_come_from_tables() {
        assert_eq!((amplitude(0), amplitude(-600), amplitude(600), amplitude(-9600), amplitude(-20000)), (1000, 501, 1995, 0, 0));
        assert_eq!(amplitude(-50), (891 + 1000_u32).div_ceil(2), "halfway between -1 dB and 0");
        assert_eq!((rate(0), rate(1200), rate(-1200), rate(100), rate(-691)), (1000, 2000, 500, 1059, 671));
        assert_eq!((thousandths(1000), thousandths(631), thousandths(1250), thousandths(5)), ("1".to_string(), "0.631".to_string(), "1.25".to_string(), "0.005".to_string()));
    }

    /// A copy's `sounds` folder of synthetic banks: a weapon bank whose cues use its own waves and
    /// another bank's, of every event the game's banks use; the other bank, stereo; the
    /// announcer's bank under `Voice/US`, its wave bank's file named otherwise than the bank;
    /// global settings with categories and distance curves.
    pub(crate) fn sounds_folder(dir: &Path) {
        std::fs::create_dir_all(dir.join("Voice/US")).unwrap();
        let tone = |n: usize, f: i32| -> Vec<i16> { (0..n).map(|i| ((i as i32 * f) % 2000 - 1000) as i16 * 8).collect() };
        std::fs::write(dir.join("UELWeapon.xwb"), wave_bank("UELWeapon", &[(32_000, 1, tone(3200, 37)), (32_000, 1, tone(1600, 53)), (22_050, 1, tone(2205, 71))])).unwrap();
        std::fs::write(dir.join("UEL.xwb"), wave_bank("UEL", &[(32_000, 2, tone(6400, 29)), (32_000, 1, tone(4000, 13))])).unwrap();
        std::fs::write(dir.join("Voice/US/Announcer_Waves.xwb"), wave_bank("XGG", &[(44_100, 1, tone(4410, 7))])).unwrap();
        let (xgs, curves) = settings(
            &[("Global", 255, None), ("World", 202, Some(0)), ("Weapons", 180, Some(1)), ("Units", 160, Some(1))],
            &[("Distance", vec![(0.0, 0.0), (4000.0, -3000.0), (6000.0, -9600.0)]), ("Distance", vec![(0.0, 0.0), (500.0, -3000.0)]), ("Distance", vec![(0.0, -100.0), (300.0, -300.0)])],
        );
        std::fs::write(dir.join("SupCom.xgs"), xgs).unwrap();
        let cue = |name: &str, category: u16, curve: u32, limit: u8, events: Vec<Event>| FixtureCue { name: name.into(), volume: 0xB4, pitch: 0, category, curves: vec![curve], limit, events };
        std::fs::write(dir.join("UELWeapon.xsb"), cue_bank("UELWeapon", &["UELWeapon", "UEL"], &[
            cue("Gun", 2, curves[0], 3, vec![Event { kind: 6, waves: vec![(0, 0), (0, 1)], clip_volume: 0xB4, pitch: Some((-200, 100)), volume: Some((143, 180)), variation: 3, ..Default::default() }]),
            cue("Engine", 3, curves[1], 0xFF, vec![Event { kind: 4, waves: vec![(1, 0)], clip_volume: 0xB4, loop_count: 0xFF, volume: Some((160, 180)), ..Default::default() }]),
            cue("Late", 3, curves[1], 2, vec![Event { kind: 1, waves: vec![(0, 2)], clip_volume: 0xB4, delay: 100, ..Default::default() }]),
            cue("Layered", 2, curves[2], 0xFF, vec![
                Event { kind: 1, waves: vec![(0, 0)], clip_volume: 0xB4, ..Default::default() },
                Event { kind: 1, waves: vec![(1, 1)], clip_volume: 0x90, delay: 50, ..Default::default() },
            ]),
            cue("Missing", 2, curves[0], 2, vec![Event { kind: 1, waves: vec![(0, 9)], clip_volume: 0xB4, ..Default::default() }]),
        ])).unwrap();
        std::fs::write(dir.join("Voice/US/XGG.xsb"), cue_bank("XGG", &["XGG"], &[cue("Alarm", 1, curves[0], 1, vec![Event { kind: 1, waves: vec![(0, 0)], clip_volume: 0xB4, ..Default::default() }])])).unwrap();
    }

    /// The banks read by their own names (the announcer's under `Voice/US`, its wave bank by its
    /// name, not its file's); each cue a set as Forged Alliance plays it: its waves (another bank's
    /// too, a stereo one made mono), a delay as leading silence, layers mixed, its level, pitch,
    /// cap, no repeats, reach and loop; a cue whose wave is missing told so.
    #[test]
    fn cues_become_sets_and_waves_files() {
        let dir = crate::tests::Temp::new("sounds-sets");
        sounds_folder(&dir.0);
        let banks = Banks::read(&dir.0).unwrap();
        assert!(banks.sound.contains_key("xgg") && banks.wave.contains_key("xgg") && banks.wave.contains_key("uel"));
        assert_eq!(banks.reference, xact::decibels(202) + xact::decibels(180));
        let mut plan = Plan::default();
        let gun = plan.set(&banks, "uelweapon", "GUN", false).unwrap();
        assert_eq!(gun, "FA_UELWEAPON_GUN");
        let set = plan.sets[&gun].clone();
        assert_eq!(set.variants, [("sounds/supcom/uelweapon/0000".to_string(), 1), ("sounds/supcom/uelweapon/0001".to_string(), 1)], "weights all alike");
        assert_eq!((set.volume, set.pitch, set.max_playing, set.no_repeat, set.looped, set.radius), ((amplitude(2 * xact::decibels(0xB4) + xact::decibels(143)), amplitude(2 * xact::decibels(0xB4) + xact::decibels(180))), (rate(-200), rate(100)), 3, true, false, "1"));
        let engine = plan.set(&banks, "UELWeapon", "Engine", false).unwrap();
        let set = plan.sets[&engine].clone();
        assert_eq!((set.variants[0].0.as_str(), set.looped, set.radius, set.max_playing), ("sounds/supcom/uel/0000", true, "0.65", 0), "another bank's wave; the units' curve reaching a quarter as far");
        assert!(set.volume.1 < 1000, "the units' category under the weapons'");
        let late = plan.set(&banks, "UELWeapon", "Late", false).unwrap();
        assert_eq!(plan.sets[&late].variants[0].0, "sounds/supcom/uelweapon/0002_d100");
        let layered = plan.set(&banks, "UELWeapon", "Layered", false).unwrap();
        assert_eq!((plan.sets[&layered].variants[0].0.as_str(), plan.sets[&layered].radius), ("sounds/supcom/mix/uelweapon_layered_0", "1.25"), "a curve never 30 dB down");
        assert_eq!(plan.set(&banks, "UELWeapon", "Missing", false), None);
        assert_eq!(plan.set(&banks, "NoBank", "Gun", false), None);
        assert_eq!(plan.missing.iter().map(|(b, c)| format!("{b}/{c}")).collect::<Vec<_>>(), ["NoBank/Gun", "UELWeapon/Missing"]);
        assert!(plan.set(&banks, "xgg", "alarm", false).is_some());
        // The files: each decodes as the game decodes it, at its rate, mono, the delay's silence
        // first; the mix as long as its longer layer after its delay.
        let mut written: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        plan.write(&banks, 2, &mut |path, bytes| {
            written.insert(path.to_string(), bytes.to_vec());
            Ok(())
        }).unwrap();
        assert_eq!(written.len(), plan.file_count());
        let decode = |path: &str| ta_audio::decode::wav::decode(&written[path]).unwrap().0;
        let stereo = decode("sounds/supcom/uel/0000.wav");
        assert_eq!((stereo.channels(), stereo.sample_rate()), (1, 32_000));
        assert!((3200..3208).contains(&stereo.frames()), "{}", stereo.frames());
        let late = decode("sounds/supcom/uelweapon/0002_d100.wav");
        assert_eq!(late.sample_rate(), 22_050);
        assert!(late.samples()[..2200].iter().all(|s| s.abs() < 0.002), "a tenth of a second's silence first");
        assert!(late.samples()[2205..2300].iter().any(|s| s.abs() > 0.05));
        let mix = decode("sounds/supcom/mix/uelweapon_layered_0.wav");
        assert!(mix.frames() >= 1600 + 4000, "{}", mix.frames());
        // Made again, the same bytes.
        let mut again: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        plan.write(&banks, 7, &mut |path, bytes| {
            again.insert(path.to_string(), bytes.to_vec());
            Ok(())
        }).unwrap();
        assert_eq!(again, written);
    }
}
