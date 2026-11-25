//! Forged Alliance's sound banks (XACT 3, version 43): wave banks (`sounds/*.xwb`, the sounds
//! themselves) and sound banks (`sounds/*.xsb`, the cues the game plays by name, each a wave of a
//! wave bank or a choice among several), and the global settings (`sounds/*.xgs`: the sounds'
//! categories and their volumes, the curves that shape them). The banks of its soundtrack
//! (`Music`, `Op_Briefing`, `FMV_BG`) and its units hold uncompressed PCM on the copies looked
//! at; a bank's other codecs (XMA, ADPCM, xWMA) are listed, not read.
//!
//! A cue's sound ([`Sound`]) plays its clips' events at once, a [`Layer`] each: a wave (events 1
//! and 4) or one of several (3 and 6), after a delay, at a volume and pitch within ranges (events
//! 4 and 6), once or over and over. A wave is found through the cue's bank index: the sound bank's
//! list of wave banks ([`SoundBank::wave_banks`]) names the wave bank by its own name, which is not
//! always its file's (`XAS_Weapon` is in `XAS_Weapons.xwb`).

use crate::{Error, Result};

/// A wave bank's waves, by their index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WaveBank {
    pub name: String,
    pub waves: Vec<Wave>,
}

/// A wave of a wave bank: how it is coded and where its data is in the bank's file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave {
    pub codec: Codec,
    pub channels: u16,
    pub rate: u32,
    /// Bits a sample (PCM: 8 or 16).
    pub bits: u16,
    /// Its frames (samples a channel).
    pub frames: u32,
    /// Its data's place in the bank's file (bytes).
    pub offset: usize,
    pub length: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Pcm,
    Xma,
    Adpcm,
    Wma,
}

impl Wave {
    /// How long it plays (seconds).
    pub fn seconds(&self) -> f64 {
        if self.rate == 0 { 0.0 } else { self.frames as f64 / self.rate as f64 }
    }
}

fn le16(b: &[u8], at: usize) -> Result<u16> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| Error::Format(format!("sound bank ends at {at}")))
}

fn le32(b: &[u8], at: usize) -> Result<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| Error::Format(format!("sound bank ends at {at}")))
}

fn byte(b: &[u8], at: usize) -> Result<u8> {
    b.get(at).copied().ok_or_else(|| Error::Format(format!("sound bank ends at {at}")))
}

/// A name of a fixed field, up to its first nul.
fn name_at(b: &[u8], at: usize, len: usize) -> String {
    let field = b.get(at..(at + len).min(b.len())).unwrap_or_default();
    let end = field.iter().position(|&c| c == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

impl WaveBank {
    /// Reads a wave bank's header and its waves' entries (not their data: [`WaveBank::data`]).
    pub fn read(b: &[u8]) -> Result<WaveBank> {
        if b.get(..4) != Some(b"WBND") {
            return Err(Error::Format("not a wave bank (no WBND)".into()));
        }
        let version = le32(b, 4)?;
        if version != 43 && version != 44 && version != 45 && version != 46 {
            return Err(Error::Format(format!("wave bank version {version}: not one this reads (43 to 46)")));
        }
        // Five segments: the bank's data, its entries, seek tables, names, the waves' data.
        let segment = |i: usize| -> Result<(usize, usize)> { Ok((le32(b, 12 + 8 * i)? as usize, le32(b, 16 + 8 * i)? as usize)) };
        let (bank, _) = segment(0)?;
        let (entries, _) = segment(1)?;
        let (data, data_length) = segment(4)?;
        let flags = le32(b, bank)?;
        let count = le32(b, bank + 4)? as usize;
        let name = name_at(b, bank + 8, 64);
        let entry_size = le32(b, bank + 72)? as usize;
        // (The compact entries, 4 bytes each, are not used by the soundtrack's banks.)
        if flags & 0x0002_0000 != 0 || entry_size < 24 {
            return Err(Error::Format(format!("wave bank {name}: compact entries are not read")));
        }
        if count > 65_536 {
            return Err(Error::Format(format!("wave bank {name}: {count} waves")));
        }
        let mut waves = Vec::with_capacity(count);
        for i in 0..count {
            let at = entries + i * entry_size;
            let duration = le32(b, at)? >> 4;
            let format = le32(b, at + 4)?;
            let (offset, length) = (le32(b, at + 8)? as usize, le32(b, at + 12)? as usize);
            if offset.checked_add(length).is_none_or(|end| end > data_length) {
                return Err(Error::Format(format!("wave bank {name}: wave {i} runs past its data")));
            }
            let codec = match format & 0x3 {
                0 => Codec::Pcm,
                1 => Codec::Xma,
                2 => Codec::Adpcm,
                _ => Codec::Wma,
            };
            let channels = ((format >> 2) & 0x7) as u16;
            let rate = (format >> 5) & 0x3_FFFF;
            let bits = if format >> 31 == 1 { 16 } else { 8 };
            waves.push(Wave { codec, channels, rate, bits, frames: duration, offset: data + offset, length });
        }
        Ok(WaveBank { name, waves })
    }

    /// The data of wave `index` in the bank's file `b` (PCM: little-endian samples, the channels
    /// interleaved).
    pub fn data<'a>(&self, b: &'a [u8], index: usize) -> Option<&'a [u8]> {
        let w = self.waves.get(index)?;
        b.get(w.offset..w.offset + w.length)
    }
}

/// A cue of a sound bank: its name, the waves it plays (one of them, or one chosen among them:
/// their wave bank's index in the sound bank's list and the wave's index there), and whether it
/// plays on until stopped; its sound whole, and how many of it may play at once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cue {
    pub name: String,
    pub waves: Vec<(u8, u16)>,
    pub looped: bool,
    pub sound: Option<Sound>,
    /// At most this many at once (255: no limit), and what a cue past that does (0: it does not
    /// play; 2: the oldest gives way; 4: the least important).
    pub instance_limit: u8,
    pub instance_behaviour: u8,
}

/// A cue's sound: its category in the global settings ([`Settings`]), its volume (a volume byte:
/// [`decibels`]), pitch (hundredths of a semitone) and priority, the global settings' curves it
/// follows (their places in that file: the distance's among them), and its layers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sound {
    pub category: u16,
    pub volume: u8,
    pub pitch: i16,
    pub priority: u8,
    pub curves: Vec<u32>,
    pub layers: Vec<Layer>,
}

/// One of a sound's clips' events: the waves it plays one of (their wave bank's index in the sound
/// bank's list, the wave's index, a weight), how it chooses (an XACT variation, its low bits: 3
/// never the same twice running), its clip's volume (a volume byte), its delay after the cue
/// starts and a random delay on top (milliseconds), its loop count (255: on until stopped), and
/// the ranges each play picks its pitch (hundredths of a semitone) and volume (bytes) from, where
/// it varies them (events 4 and 6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layer {
    pub waves: Vec<(u8, u16, u8)>,
    pub variation: u16,
    pub clip_volume: u8,
    pub delay: u16,
    pub random_delay: u16,
    pub loop_count: u8,
    pub pitch_range: Option<(i16, i16)>,
    pub volume_range: Option<(u8, u8)>,
}

impl Layer {
    /// Whether it plays on until stopped.
    pub fn looped(&self) -> bool {
        self.loop_count == 0xFF
    }

    /// Whether it never plays the same wave twice running (its variation's "random, no
    /// immediate repeats").
    pub fn no_repeat(&self) -> bool {
        self.variation & 0x7 == 3
    }
}

/// A volume byte in hundredths of a decibel, by FAudio's reading of XACT's: 3969 log10(b / 28240)
/// plus 8715 (0xB4: 0 dB; 0: silence, -96 dB). From a table worked out once, so that the
/// converter's figures are the same on every computer.
pub fn decibels(byte: u8) -> i32 {
    VOLUME_CENTIBELS[byte as usize] as i32
}

/// A sound bank's cues, and the names of the wave banks its waves are in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SoundBank {
    pub name: String,
    pub wave_banks: Vec<String>,
    pub cues: Vec<Cue>,
}

impl SoundBank {
    /// Reads a sound bank's cues and the waves each plays: a sound of one wave, or of clips whose
    /// events play a wave or one of several (other events are left out).
    pub fn read(b: &[u8]) -> Result<SoundBank> {
        if b.get(..4) != Some(b"SDBK") {
            return Err(Error::Format("not a sound bank (no SDBK)".into()));
        }
        let version = le16(b, 6)?;
        if version != 43 {
            return Err(Error::Format(format!("sound bank version {version}: not one this reads (43)")));
        }
        let simple = le16(b, 0x13)? as usize;
        let complex = le16(b, 0x15)? as usize;
        let banks = byte(b, 0x1B)? as usize;
        let names_length = le16(b, 0x1E)? as usize;
        let (simple_at, complex_at, names_at) = (le32(b, 0x22)? as usize, le32(b, 0x26)? as usize, le32(b, 0x2A)? as usize);
        let banks_at = le32(b, 0x3A)? as usize;
        let name = name_at(b, 0x4A, 64);
        let wave_banks = (0..banks).map(|i| name_at(b, banks_at + 64 * i, 64)).collect();
        let names: Vec<String> = match b.get(names_at..names_at + names_length) {
            Some(table) if names_at != 0xFFFF_FFFF => table.split(|&c| c == 0).map(|n| String::from_utf8_lossy(n).into_owned()).collect(),
            _ => Vec::new(),
        };
        let mut cues = Vec::new();
        for i in 0..simple {
            let at = simple_at + 5 * i;
            let sound = sound(b, le32(b, at + 1)? as usize)?;
            cues.push(Cue { name: names.get(i).cloned().unwrap_or_default(), instance_limit: 0xFF, ..Cue::of(sound) });
        }
        for i in 0..complex {
            let at = complex_at + 15 * i;
            let flags = byte(b, at)?;
            let name = names.get(simple + i).cloned().unwrap_or_default();
            let (instance_limit, instance_behaviour) = (byte(b, at + 9)?, byte(b, at + 14)? >> 3);
            // (A cue of variations of sounds, not of one sound, is not one the game's banks use.)
            let cue = if flags & 0x04 != 0 { Cue::of(sound(b, le32(b, at + 1)? as usize)?) } else { Cue::default() };
            cues.push(Cue { name, instance_limit, instance_behaviour, ..cue });
        }
        Ok(SoundBank { name, wave_banks, cues })
    }

    /// The cue of this name (in any case).
    pub fn cue(&self, name: &str) -> Option<&Cue> {
        self.cues.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }
}

impl Cue {
    /// A cue of `sound`: its waves, every layer's in order, and whether one plays on.
    fn of(sound: Sound) -> Cue {
        let waves = sound.layers.iter().flat_map(|l| l.waves.iter().map(|&(bank, wave, _)| (bank, wave))).collect();
        let looped = sound.layers.iter().any(Layer::looped);
        Cue { waves, looped, sound: Some(sound), instance_limit: 0xFF, ..Cue::default() }
    }
}

/// The sound at `at`: its settings, its curves, and its clips' events, a layer each. Events of
/// kinds other than these four (volume, pitch, markers: none in the game's banks) end their
/// clip's list.
fn sound(b: &[u8], at: usize) -> Result<Sound> {
    let flags = byte(b, at)?;
    let mut sound = Sound { category: le16(b, at + 1)?, volume: byte(b, at + 3)?, pitch: le16(b, at + 4)? as i16, priority: byte(b, at + 6)?, ..Sound::default() };
    let mut p = at + 9;
    // A sound of one wave: its track and bank.
    if flags & 0x01 == 0 {
        sound.layers.push(Layer { waves: vec![(byte(b, p + 2)?, le16(b, p)?, 0xFF)], clip_volume: 0xB4, ..Layer::default() });
        return Ok(sound);
    }
    let clips = byte(b, p)? as usize;
    p += 1;
    // Its curves (the sound's own, flag 0x02, then each clip's, 0x04: their places in the global
    // settings), skipped by their length.
    if flags & 0x0E != 0 {
        let length = le16(b, p)? as usize;
        if flags & 0x02 != 0 {
            let count = byte(b, p + 2)? as usize;
            for k in 0..count {
                sound.curves.push(le32(b, p + 3 + 4 * k)?);
            }
        }
        p += length;
    }
    for c in 0..clips {
        let clip_volume = byte(b, p + 5 * c)?;
        let mut q = le32(b, p + 1 + 5 * c)? as usize;
        let events = byte(b, q)? as usize;
        q += 1;
        for _ in 0..events {
            let info = le32(b, q)?;
            let (kind, delay, random_delay) = (info & 0x1F, ((info >> 5) & 0xFFFF) as u16, le16(b, q + 4)?);
            q += 6;
            let mut layer = Layer { clip_volume, delay, random_delay, ..Layer::default() };
            // (The effects' ranges of events 4 and 6, as their flags say: 0x40 the pitch's,
            // 0x80 the volume's.)
            let ranges = |at: usize, layer: &mut Layer| -> Result<()> {
                let varies = byte(b, at + 6)?;
                if varies & 0x40 != 0 {
                    layer.pitch_range = Some((le16(b, at)? as i16, le16(b, at + 2)? as i16));
                }
                if varies & 0x80 != 0 {
                    layer.volume_range = Some((byte(b, at + 4)?, byte(b, at + 5)?));
                }
                Ok(())
            };
            // (Several waves: their tracks, banks and weights.)
            let tracks = |at: usize, count: usize| -> Result<Vec<(u8, u16, u8)>> {
                (0..count).map(|t| Ok((byte(b, at + 5 * t + 2)?, le16(b, at + 5 * t)?, byte(b, at + 5 * t + 4)?.saturating_sub(byte(b, at + 5 * t + 3)?)))).collect()
            };
            match kind {
                // Play a wave: a separator, flags, its track, bank and loop count, a pan's angle and
                // arc; event 4 its effects' ranges after them.
                1 | 4 => {
                    layer.waves.push((byte(b, q + 4)?, le16(b, q + 2)?, 0xFF));
                    layer.loop_count = byte(b, q + 5)?;
                    if kind == 4 {
                        ranges(q + 10, &mut layer)?;
                        q += 17;
                    } else {
                        q += 10;
                    }
                }
                // Play one of several: a separator, flags, the loop count, the pan's angle and arc;
                // event 6 its effects' ranges; then the tracks' count, the variation, four bytes
                // and the tracks.
                3 | 6 => {
                    layer.loop_count = byte(b, q + 2)?;
                    let at = if kind == 6 {
                        ranges(q + 7, &mut layer)?;
                        q + 14
                    } else {
                        q + 7
                    };
                    let count = le16(b, at)? as usize;
                    layer.variation = le16(b, at + 2)?;
                    layer.waves = tracks(at + 8, count)?;
                    q = at + 8 + 5 * count;
                }
                _ => break,
            }
            sound.layers.push(layer);
        }
    }
    Ok(sound)
}

/// The global settings (`SupCom.xgs`, XACT's `XGSF` of format 42): the sounds' categories, by
/// their indices (a sound's [`Sound::category`]), and the curves the sounds name (by their places
/// in the file).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub categories: Vec<Category>,
    pub curves: Vec<Curve>,
}

/// A category: its name, its volume (a volume byte) and the category it is under (none at the top).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Category {
    pub name: String,
    pub volume: u8,
    pub parent: Option<u16>,
}

/// A curve of the global settings: its place in the file (what a sound names), the variable it
/// follows (`Distance`...), what it changes (0: the volume, in hundredths of a decibel), its points.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve {
    pub at: u32,
    pub variable: String,
    pub parameter: u16,
    pub points: Vec<(f32, f32)>,
}

impl Settings {
    /// Reads the global settings' categories and curves.
    pub fn read(b: &[u8]) -> Result<Settings> {
        if b.get(..4) != Some(b"XGSF") {
            return Err(Error::Format("not XACT's global settings (no XGSF)".into()));
        }
        let format = le16(b, 6)?;
        if format != 42 {
            return Err(Error::Format(format!("global settings of format {format}: not one this reads (42)")));
        }
        let (categories, variables, curves) = (le16(b, 19)? as usize, le16(b, 21)? as usize, le16(b, 27)? as usize);
        let offset = |i: usize| -> Result<usize> { Ok(le32(b, 33 + 4 * i)? as usize) };
        let (categories_at, variables_at, category_names_at, variable_names_at, curves_at) = (offset(0)?, offset(1)?, offset(6)?, offset(7)?, offset(8)?);
        let names = |at: usize, count: usize| -> Vec<String> {
            b.get(at..).unwrap_or_default().split(|&c| c == 0).take(count).map(|n| String::from_utf8_lossy(n).into_owned()).collect()
        };
        let (category_names, variable_names) = (names(category_names_at, categories), names(variable_names_at, variables));
        let _ = variables_at;
        let mut settings = Settings::default();
        for i in 0..categories {
            let at = categories_at + 10 * i;
            let parent = le16(b, at + 6)?;
            settings.categories.push(Category { name: category_names.get(i).cloned().unwrap_or_default(), volume: byte(b, at + 8)?, parent: (parent != 0xFFFF).then_some(parent) });
        }
        let mut q = curves_at;
        for _ in 0..curves {
            let at = q as u32;
            let variable = le16(b, q)? as usize;
            let count = byte(b, q + 2)? as usize;
            let parameter = le16(b, q + 3)?;
            q += 5;
            let mut points = Vec::with_capacity(count);
            for _ in 0..count {
                let x = f32::from_bits(le32(b, q)?);
                let y = f32::from_bits(le32(b, q + 4)?);
                points.push((x, y));
                q += 9;
            }
            settings.curves.push(Curve { at, variable: variable_names.get(variable).cloned().unwrap_or_default(), parameter, points });
        }
        Ok(settings)
    }

    /// The volume of category `index` and of those over it, but the top one (`Global`, over every
    /// category alike), in hundredths of a decibel.
    pub fn category_centibels(&self, index: u16) -> i32 {
        let mut total = 0;
        let mut at = Some(index);
        for _ in 0..self.categories.len() {
            let Some(c) = at.and_then(|i| self.categories.get(i as usize)) else { break };
            if c.parent.is_none() {
                break;
            }
            total += decibels(c.volume);
            at = c.parent;
        }
        total
    }

    /// The curve at `at` in the file.
    pub fn curve(&self, at: u32) -> Option<&Curve> {
        self.curves.iter().find(|c| c.at == at)
    }
}

/// [`decibels`]' table: FAudio's formula at each byte, in hundredths of a decibel, rounded (0 and
/// the bytes under 1, -96 dB).
const VOLUME_CENTIBELS: [i16; 256] = [
    -9600, -8950, -7756, -7057, -6561, -6176, -5862, -5596, -5366, -5163, -4981, -4817, -4667, -4529, -4401, -4283, -4171, -4067, -3968, -3875, -3787, -3703, -3622, -3546,
    -3472, -3402, -3334, -3269, -3207, -3146, -3088, -3031, -2977, -2924, -2872, -2822, -2774, -2726, -2680, -2636, -2592, -2549, -2508, -2467, -2428, -2389, -2351, -2314,
    -2278, -2242, -2207, -2173, -2140, -2107, -2075, -2043, -2012, -1981, -1951, -1922, -1893, -1865, -1836, -1809, -1782, -1755, -1729, -1703, -1677, -1652, -1627, -1603,
    -1579, -1555, -1532, -1508, -1486, -1463, -1441, -1419, -1397, -1376, -1355, -1334, -1313, -1293, -1272, -1253, -1233, -1213, -1194, -1175, -1156, -1138, -1119, -1101,
    -1083, -1065, -1047, -1030, -1012, -995, -978, -962, -945, -928, -912, -896, -880, -864, -848, -833, -817, -802, -787, -772, -757, -742, -727, -713,
    -698, -684, -670, -656, -642, -628, -614, -600, -587, -574, -560, -547, -534, -521, -508, -495, -482, -470, -457, -445, -432, -420, -408, -396,
    -384, -372, -360, -348, -337, -325, -314, -302, -291, -279, -268, -257, -246, -235, -224, -213, -202, -192, -181, -170, -160, -149, -139, -129,
    -118, -108, -98, -88, -78, -68, -58, -48, -38, -28, -19, -9, 1, 10, 20, 29, 39, 48, 57, 66, 76, 85, 94, 103,
    112, 121, 130, 139, 147, 156, 165, 174, 182, 191, 199, 208, 216, 225, 233, 242, 250, 258, 266, 275, 283, 291, 299, 307,
    315, 323, 331, 339, 347, 354, 362, 370, 378, 385, 393, 401, 408, 416, 423, 431, 438, 446, 453, 460, 468, 475, 482, 489,
    497, 504, 511, 518, 525, 532, 539, 546, 553, 560, 567, 574, 581, 588, 594, 601,
];

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A wave bank of 16-bit PCM waves (rate, channels, frames each), as XACT writes one.
    pub(crate) fn wave_bank(name: &str, waves: &[(u32, u16, Vec<i16>)]) -> Vec<u8> {
        let entries = 52 + 96;
        let data = (entries + 24 * waves.len()).next_multiple_of(2048);
        let mut payload = Vec::new();
        let mut meta = Vec::new();
        for (rate, channels, samples) in waves {
            let offset = payload.len() as u32;
            for s in samples {
                payload.extend_from_slice(&s.to_le_bytes());
            }
            let frames = samples.len() as u32 / *channels as u32;
            let format = 1u32 << 31 | ((2 * *channels as u32) << 23) | (rate << 5) | ((*channels as u32) << 2);
            meta.extend_from_slice(&(frames << 4).to_le_bytes());
            meta.extend_from_slice(&format.to_le_bytes());
            meta.extend_from_slice(&offset.to_le_bytes());
            meta.extend_from_slice(&(samples.len() as u32 * 2).to_le_bytes());
            meta.extend_from_slice(&[0; 8]);
        }
        let mut b = b"WBND".to_vec();
        b.extend_from_slice(&43u32.to_le_bytes());
        b.extend_from_slice(&42u32.to_le_bytes());
        for (offset, length) in [(52, 96), (entries, meta.len()), (0, 0), (0, 0), (data, payload.len())] {
            b.extend_from_slice(&(offset as u32).to_le_bytes());
            b.extend_from_slice(&(length as u32).to_le_bytes());
        }
        b.extend_from_slice(&0x0008_0001u32.to_le_bytes());
        b.extend_from_slice(&(waves.len() as u32).to_le_bytes());
        let mut field = [0u8; 64];
        field[..name.len()].copy_from_slice(name.as_bytes());
        b.extend_from_slice(&field);
        b.extend_from_slice(&24u32.to_le_bytes());
        b.extend_from_slice(&64u32.to_le_bytes());
        b.extend_from_slice(&2048u32.to_le_bytes());
        b.resize(entries, 0);
        b.extend_from_slice(&meta);
        b.resize(data, 0);
        b.extend_from_slice(&payload);
        b
    }

    /// A sound bank of complex cues, each a sound of one clip playing one wave (looped) or one of
    /// several, as XACT writes one; its names after.
    pub(crate) fn sound_bank(cues: &[(&str, Vec<u16>, bool)]) -> Vec<u8> {
        let mut b = vec![0u8; 0x8A];
        b[..4].copy_from_slice(b"SDBK");
        b[4..6].copy_from_slice(&43u16.to_le_bytes());
        b[6..8].copy_from_slice(&43u16.to_le_bytes());
        b[0x12] = 1;
        b[0x15..0x17].copy_from_slice(&(cues.len() as u16).to_le_bytes());
        b[0x1B] = 1;
        b[0x1C..0x1E].copy_from_slice(&(cues.len() as u16).to_le_bytes());
        b[0x22..0x26].copy_from_slice(&u32::MAX.to_le_bytes());
        b[0x4A..0x4F].copy_from_slice(b"Music");
        // The wave bank's name.
        let banks_at = b.len();
        let mut field = [0u8; 64];
        field[..5].copy_from_slice(b"Music");
        b.extend_from_slice(&field);
        // The sounds.
        let mut sounds = Vec::new();
        for (_, tracks, looped) in cues {
            let at = b.len();
            sounds.push(at);
            let mut s = vec![0x05, 2, 0, 0x3C, 0, 0, 0, 0, 0, 1];
            // (Effects' settings: seven bytes, their length first.)
            s.extend_from_slice(&[7, 0, 1, 0x7E, 4, 0, 0]);
            let clip = at + s.len() + 5;
            s.push(0xB4);
            s.extend_from_slice(&(clip as u32).to_le_bytes());
            s.push(1);
            if tracks.len() == 1 {
                s.extend_from_slice(&0x2000_0001u32.to_le_bytes());
                s.extend_from_slice(&[0, 0, 0xFF, 0x0C]);
                s.extend_from_slice(&tracks[0].to_le_bytes());
                s.push(0);
                s.push(if *looped { 0xFF } else { 0 });
                s.extend_from_slice(&[0, 0, 0xA0, 0x8C]);
            } else {
                s.extend_from_slice(&0x2000_0003u32.to_le_bytes());
                s.extend_from_slice(&[0, 0, 0xFF, 0x0C, if *looped { 0xFF } else { 0 }, 0, 0, 0xA0, 0x8C]);
                s.extend_from_slice(&(tracks.len() as u16).to_le_bytes());
                s.extend_from_slice(&[0x43, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
                for t in tracks {
                    s.extend_from_slice(&t.to_le_bytes());
                    s.extend_from_slice(&[0, 0, 0xFF]);
                }
            }
            let length = s.len() as u16;
            s[7..9].copy_from_slice(&length.to_le_bytes());
            b.extend_from_slice(&s);
        }
        // The cues.
        let complex_at = b.len();
        for at in &sounds {
            b.push(0x05);
            b.extend_from_slice(&(*at as u32).to_le_bytes());
            b.extend_from_slice(&[0; 4]);
            b.extend_from_slice(&[1, 0, 0, 0xC8, 0, 0x11]);
        }
        let names_at = b.len();
        for (name, _, _) in cues {
            b.extend_from_slice(name.as_bytes());
            b.push(0);
        }
        let names_length = (b.len() - names_at) as u16;
        b[0x1E..0x20].copy_from_slice(&names_length.to_le_bytes());
        b[0x26..0x2A].copy_from_slice(&(complex_at as u32).to_le_bytes());
        b[0x2A..0x2E].copy_from_slice(&(names_at as u32).to_le_bytes());
        b[0x3A..0x3E].copy_from_slice(&(banks_at as u32).to_le_bytes());
        b[0x46..0x4A].copy_from_slice(&(sounds[0] as u32).to_le_bytes());
        b
    }

    /// An event of a fixture's cue, a clip of its own: its kind (1, 3, 4 or 6), its waves (their
    /// wave bank's index and track), its clip's volume, delay, loop count, the ranges it varies
    /// (events 4 and 6) and its variation.
    #[derive(Clone, Debug, Default)]
    pub(crate) struct Event {
        pub kind: u8,
        pub waves: Vec<(u8, u16)>,
        pub clip_volume: u8,
        pub delay: u16,
        pub loop_count: u8,
        pub pitch: Option<(i16, i16)>,
        pub volume: Option<(u8, u8)>,
        pub variation: u16,
    }

    /// A cue of a fixture's sound bank: its sound's volume, pitch, category and curves, its
    /// instance limit, its events.
    #[derive(Clone, Debug, Default)]
    pub(crate) struct FixtureCue {
        pub name: String,
        pub volume: u8,
        pub pitch: i16,
        pub category: u16,
        pub curves: Vec<u32>,
        pub limit: u8,
        pub events: Vec<Event>,
    }

    /// A sound bank of complex cues with events of every kind the game's banks use, as XACT writes
    /// one (`name`, its wave banks' names after).
    pub(crate) fn cue_bank(name: &str, wave_banks: &[&str], cues: &[FixtureCue]) -> Vec<u8> {
        let mut b = vec![0u8; 0x8A];
        b[..4].copy_from_slice(b"SDBK");
        b[4..6].copy_from_slice(&43u16.to_le_bytes());
        b[6..8].copy_from_slice(&43u16.to_le_bytes());
        b[0x15..0x17].copy_from_slice(&(cues.len() as u16).to_le_bytes());
        b[0x1B] = wave_banks.len() as u8;
        b[0x22..0x26].copy_from_slice(&u32::MAX.to_le_bytes());
        b[0x4A..0x4A + name.len()].copy_from_slice(name.as_bytes());
        let banks_at = b.len();
        for bank in wave_banks {
            let mut field = [0u8; 64];
            field[..bank.len()].copy_from_slice(bank.as_bytes());
            b.extend_from_slice(&field);
        }
        let mut sounds = Vec::new();
        for cue in cues {
            let at = b.len();
            sounds.push(at);
            let flags = if cue.curves.is_empty() { 0x01 } else { 0x03 };
            let mut s = vec![flags];
            s.extend_from_slice(&cue.category.to_le_bytes());
            s.push(cue.volume);
            s.extend_from_slice(&cue.pitch.to_le_bytes());
            s.extend_from_slice(&[0, 0, 0]);
            s.push(cue.events.len() as u8);
            if !cue.curves.is_empty() {
                let length = 3 + 4 * cue.curves.len() as u16;
                s.extend_from_slice(&length.to_le_bytes());
                s.push(cue.curves.len() as u8);
                for c in &cue.curves {
                    s.extend_from_slice(&c.to_le_bytes());
                }
            }
            // The clips, then each clip's events.
            let clips_at = s.len();
            s.resize(clips_at + 5 * cue.events.len(), 0);
            for (i, e) in cue.events.iter().enumerate() {
                let events_at = (at + s.len()) as u32;
                s[clips_at + 5 * i] = e.clip_volume;
                s[clips_at + 5 * i + 1..clips_at + 5 * i + 5].copy_from_slice(&events_at.to_le_bytes());
                s.push(1);
                s.extend_from_slice(&((e.kind as u32) | ((e.delay as u32) << 5)).to_le_bytes());
                s.extend_from_slice(&[0, 0, 0xFF, 0x0C]);
                let effects = |s: &mut Vec<u8>| {
                    let (p, v) = (e.pitch.unwrap_or((-100, 100)), e.volume.unwrap_or((151, 214)));
                    s.extend_from_slice(&p.0.to_le_bytes());
                    s.extend_from_slice(&p.1.to_le_bytes());
                    s.extend_from_slice(&[v.0, v.1, if e.pitch.is_some() { 0x40 } else { 0 } | if e.volume.is_some() { 0x80 } else { 0 }]);
                };
                match e.kind {
                    1 | 4 => {
                        s.extend_from_slice(&e.waves[0].1.to_le_bytes());
                        s.push(e.waves[0].0);
                        s.push(e.loop_count);
                        s.extend_from_slice(&[0, 0, 0xA0, 0x8C]);
                        if e.kind == 4 {
                            effects(&mut s);
                        }
                    }
                    _ => {
                        s.push(e.loop_count);
                        s.extend_from_slice(&[0, 0, 0xA0, 0x8C]);
                        if e.kind == 6 {
                            effects(&mut s);
                        }
                        s.extend_from_slice(&(e.waves.len() as u16).to_le_bytes());
                        s.extend_from_slice(&e.variation.to_le_bytes());
                        s.extend_from_slice(&[0xFF; 4]);
                        for (bank, track) in &e.waves {
                            s.extend_from_slice(&track.to_le_bytes());
                            s.extend_from_slice(&[*bank, 0, 0xFF]);
                        }
                    }
                }
            }
            let length = s.len() as u16;
            s[7..9].copy_from_slice(&length.to_le_bytes());
            b.extend_from_slice(&s);
        }
        let complex_at = b.len();
        for (cue, at) in cues.iter().zip(&sounds) {
            b.push(0x05);
            b.extend_from_slice(&(*at as u32).to_le_bytes());
            b.extend_from_slice(&[0; 4]);
            b.extend_from_slice(&[cue.limit, 0, 0, 0, 0, 0x10]);
        }
        let names_at = b.len();
        for cue in cues {
            b.extend_from_slice(cue.name.as_bytes());
            b.push(0);
        }
        let names_length = (b.len() - names_at) as u16;
        b[0x1E..0x20].copy_from_slice(&names_length.to_le_bytes());
        b[0x26..0x2A].copy_from_slice(&(complex_at as u32).to_le_bytes());
        b[0x2A..0x2E].copy_from_slice(&(names_at as u32).to_le_bytes());
        b[0x3A..0x3E].copy_from_slice(&(banks_at as u32).to_le_bytes());
        b
    }

    /// Global settings of format 42: `categories` (name, volume byte, parent), and curves
    /// (variable, points: in hundredths of a decibel by it); where each curve lands in the file.
    pub(crate) fn settings(categories: &[(&str, u8, Option<u16>)], curves: &[(&str, Vec<(f32, f32)>)]) -> (Vec<u8>, Vec<u32>) {
        let variables: Vec<&str> = curves.iter().map(|(v, _)| *v).collect();
        let mut b = b"XGSF".to_vec();
        b.extend_from_slice(&43u16.to_le_bytes());
        b.extend_from_slice(&42u16.to_le_bytes());
        b.extend_from_slice(&[0; 2 + 8 + 1]);
        for count in [categories.len(), variables.len(), 0, 0, curves.len(), 0, 0] {
            b.extend_from_slice(&(count as u16).to_le_bytes());
        }
        let offsets_at = b.len();
        b.resize(offsets_at + 44, 0xFF);
        let mut offsets = [u32::MAX; 11];
        offsets[0] = b.len() as u32;
        for (_, volume, parent) in categories {
            b.extend_from_slice(&[0xFF, 0, 0, 0, 0, 0]);
            b.extend_from_slice(&parent.unwrap_or(0xFFFF).to_le_bytes());
            b.extend_from_slice(&[*volume, 2]);
        }
        offsets[6] = b.len() as u32;
        for (name, _, _) in categories {
            b.extend_from_slice(name.as_bytes());
            b.push(0);
        }
        offsets[7] = b.len() as u32;
        for v in &variables {
            b.extend_from_slice(v.as_bytes());
            b.push(0);
        }
        offsets[8] = b.len() as u32;
        let mut at = Vec::new();
        for (i, (_, points)) in curves.iter().enumerate() {
            at.push(b.len() as u32);
            b.extend_from_slice(&(i as u16).to_le_bytes());
            b.push(points.len() as u8);
            b.extend_from_slice(&0u16.to_le_bytes());
            for (x, y) in points {
                b.extend_from_slice(&x.to_bits().to_le_bytes());
                b.extend_from_slice(&y.to_bits().to_le_bytes());
                b.push(0);
            }
        }
        for (i, o) in offsets.iter().enumerate() {
            b[offsets_at + 4 * i..offsets_at + 4 * i + 4].copy_from_slice(&o.to_le_bytes());
        }
        (b, at)
    }

    /// Events 4 and 6 read whole: their waves (another wave bank's by its index), weights, the
    /// ranges their flags say they vary, delays, loops and variations; the sound's volume, pitch,
    /// category and curves; the cue's instance limit; events 1 and 3 as before.
    #[test]
    fn every_event_the_banks_use_is_read() {
        let cue = |name: &str, events: Vec<Event>| FixtureCue { name: name.into(), volume: 0xA0, pitch: -300, category: 6, curves: vec![0x396], limit: 3, events };
        let bytes = cue_bank("UELWeapon", &["UELWeapon", "UEL"], &[
            cue("Gun", vec![Event { kind: 4, waves: vec![(1, 7)], clip_volume: 0xB4, pitch: Some((-200, 100)), volume: Some((143, 180)), ..Default::default() }]),
            cue("Boom", vec![Event { kind: 6, waves: vec![(0, 1), (0, 2), (1, 3)], clip_volume: 0xB0, volume: Some((160, 180)), variation: 3, ..Default::default() }]),
            cue("Layered", vec![
                Event { kind: 1, waves: vec![(0, 4)], clip_volume: 0xB4, ..Default::default() },
                Event { kind: 4, waves: vec![(0, 5)], clip_volume: 0x90, delay: 100, loop_count: 0xFF, pitch: Some((0, 200)), ..Default::default() },
                Event { kind: 3, waves: vec![(0, 6), (0, 8)], clip_volume: 0xB4, delay: 1700, ..Default::default() },
            ]),
        ]);
        let bank = SoundBank::read(&bytes).unwrap();
        assert_eq!(bank.wave_banks, ["UELWeapon", "UEL"]);
        let gun = bank.cue("gun").unwrap();
        assert_eq!((gun.instance_limit, gun.instance_behaviour, gun.waves.as_slice()), (3, 2, &[(1, 7)][..]));
        let sound = gun.sound.as_ref().unwrap();
        assert_eq!((sound.volume, sound.pitch, sound.category, sound.curves.as_slice()), (0xA0, -300, 6, &[0x396][..]));
        assert_eq!(sound.layers, [Layer { waves: vec![(1, 7, 0xFF)], clip_volume: 0xB4, pitch_range: Some((-200, 100)), volume_range: Some((143, 180)), ..Default::default() }]);
        let boom = bank.cue("Boom").unwrap().sound.clone().unwrap();
        let layer = &boom.layers[0];
        assert_eq!((layer.waves.len(), layer.pitch_range, layer.volume_range, layer.no_repeat(), layer.clip_volume), (3, None, Some((160, 180)), true, 0xB0), "the pitch not varied: its flag clear");
        let layered = bank.cue("Layered").unwrap();
        let layers = &layered.sound.as_ref().unwrap().layers;
        assert_eq!(layers.len(), 3);
        assert_eq!((layers[1].delay, layers[1].looped(), layers[1].pitch_range, layers[1].clip_volume), (100, true, Some((0, 200)), 0x90));
        assert_eq!((layers[2].delay, layers[2].waves.len()), (1700, 2));
        assert!(layered.looped && layered.waves == [(0, 4), (0, 5), (0, 6), (0, 8)]);
        assert_eq!((decibels(0xB4), decibels(0), decibels(255), decibels(143)), (1, -9600, 601, -396));
    }

    /// The global settings: each category's volume and the one over it (a sound's volume with its
    /// categories' but the top one's), and the curves where the sounds name them.
    #[test]
    fn the_global_settings_give_categories_and_curves() {
        let (bytes, at) = settings(&[("Global", 255, None), ("World", 202, Some(0)), ("Weapons", 180, Some(1)), ("Air", 113, Some(1))], &[("Distance", vec![(0.0, 0.0), (500.0, -2400.0), (2000.0, -9600.0)]), ("Duck", vec![(0.0, 0.0)])]);
        let s = Settings::read(&bytes).unwrap();
        assert_eq!(s.categories.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Global", "World", "Weapons", "Air"]);
        assert_eq!(s.category_centibels(2), decibels(202) + decibels(180), "the top one left out");
        assert_eq!(s.category_centibels(3), decibels(202) + decibels(113));
        assert_eq!(s.category_centibels(0), 0);
        let distance = s.curve(at[0]).unwrap();
        assert_eq!((distance.variable.as_str(), distance.points.len(), distance.points[1]), ("Distance", 3, (500.0, -2400.0)));
        assert_eq!(s.curve(at[1]).unwrap().variable, "Duck");
        assert!(s.curve(at[0] + 1).is_none());
        assert!(Settings::read(b"XGSF\x2b\0\x2b\0").is_err(), "another format");
    }

    /// A wave bank's waves read as written: coded, rate, channels, length, and their data.
    #[test]
    fn a_wave_bank_lists_its_waves_and_their_data() {
        let bytes = wave_bank("Music", &[(44_100, 2, vec![1, -1, 2, -2]), (32_000, 1, vec![7, 8, 9])]);
        let bank = WaveBank::read(&bytes).unwrap();
        assert_eq!(bank.name, "Music");
        assert_eq!(bank.waves.len(), 2);
        let w = &bank.waves[0];
        assert_eq!((w.codec, w.channels, w.rate, w.bits, w.frames), (Codec::Pcm, 2, 44_100, 16, 2));
        assert_eq!(bank.data(&bytes, 0).unwrap(), [1i16, -1, 2, -2].iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<u8>>());
        assert_eq!((bank.waves[1].channels, bank.waves[1].rate, bank.waves[1].frames), (1, 32_000, 3));
        assert!(WaveBank::read(b"WBNDxxxx").is_err());
        assert!(WaveBank::read(b"RIFF").is_err());
    }

    /// A sound bank's cues by name, each with the waves it plays and whether it plays on.
    #[test]
    fn a_sound_banks_cues_name_their_waves() {
        let bytes = sound_bank(&[("Main_Menu", vec![4], true), ("Base_Building", vec![7, 1, 3], true), ("Battle", vec![5, 2], false)]);
        let bank = SoundBank::read(&bytes).unwrap();
        assert_eq!(bank.wave_banks, ["Music"]);
        let menu = bank.cue("main_menu").unwrap();
        assert_eq!((menu.name.as_str(), menu.waves.as_slice(), menu.looped), ("Main_Menu", &[(0, 4)][..], true));
        assert_eq!(bank.cue("Base_Building").unwrap().waves, [(0, 7), (0, 1), (0, 3)]);
        assert_eq!(bank.cue("Battle").unwrap().waves, [(0, 5), (0, 2)]);
        assert!(!bank.cue("Battle").unwrap().looped);
        assert!(SoundBank::read(b"SDBK").is_err());
    }
}
