//! Forged Alliance's sound banks (XACT 3, version 43): wave banks (`sounds/*.xwb`, the sounds
//! themselves) and sound banks (`sounds/*.xsb`, the cues the game plays by name, each a wave of a
//! wave bank or a choice among several). The banks of its soundtrack (`Music`, `Op_Briefing`,
//! `FMV_BG`) hold uncompressed PCM on the copies looked at; a bank's other codecs (XMA, ADPCM,
//! xWMA) are listed, not read.

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
/// plays on until stopped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cue {
    pub name: String,
    pub waves: Vec<(u8, u16)>,
    pub looped: bool,
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
            let (waves, looped) = sound(b, le32(b, at + 1)? as usize)?;
            cues.push(Cue { name: names.get(i).cloned().unwrap_or_default(), waves, looped });
        }
        for i in 0..complex {
            let at = complex_at + 15 * i;
            let flags = byte(b, at)?;
            // (A cue of variations of sounds, not of one sound, is not one the soundtrack uses.)
            let (waves, looped) = if flags & 0x04 != 0 { sound(b, le32(b, at + 1)? as usize)? } else { (Vec::new(), false) };
            cues.push(Cue { name: names.get(simple + i).cloned().unwrap_or_default(), waves, looped });
        }
        Ok(SoundBank { name, wave_banks, cues })
    }

    /// The cue of this name (in any case).
    pub fn cue(&self, name: &str) -> Option<&Cue> {
        self.cues.iter().find(|c| c.name.eq_ignore_ascii_case(name))
    }
}

/// The waves a sound at `at` plays, and whether it plays on (a wave played with its loop count
/// 255).
fn sound(b: &[u8], at: usize) -> Result<(Vec<(u8, u16)>, bool)> {
    let flags = byte(b, at)?;
    let mut p = at + 9;
    // A sound of one wave: its track and bank.
    if flags & 0x01 == 0 {
        return Ok((vec![(byte(b, p + 2)?, le16(b, p)?)], false));
    }
    let clips = byte(b, p)? as usize;
    p += 1;
    // (Its effects' settings, skipped.)
    if flags & 0x0E != 0 {
        p += le16(b, p)? as usize;
    }
    let mut waves = Vec::new();
    let mut looped = false;
    for c in 0..clips {
        let mut q = le32(b, p + 1 + 5 * c)? as usize;
        let events = byte(b, q)? as usize;
        q += 1;
        for _ in 0..events {
            let kind = le32(b, q)? & 0x1F;
            q += 6;
            match kind {
                // Play a wave: its track, bank and loop count.
                1 => {
                    waves.push((byte(b, q + 4)?, le16(b, q + 2)?));
                    looped |= byte(b, q + 5)? == 0xFF;
                    q += 10;
                }
                // Play one of several: their tracks and banks.
                3 => {
                    looped |= byte(b, q + 2)? == 0xFF;
                    q += 7;
                    let tracks = le16(b, q)? as usize;
                    q += 8;
                    for t in 0..tracks {
                        waves.push((byte(b, q + 5 * t + 2)?, le16(b, q + 5 * t)?));
                    }
                    q += 5 * tracks;
                }
                // (Other events: volume, pitch, markers... none the soundtrack needs.)
                _ => break,
            }
        }
    }
    Ok((waves, looped))
}

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
        assert_eq!(bank.cue("main_menu").unwrap(), &Cue { name: "Main_Menu".into(), waves: vec![(0, 4)], looped: true });
        assert_eq!(bank.cue("Base_Building").unwrap().waves, [(0, 7), (0, 1), (0, 3)]);
        assert_eq!(bank.cue("Battle").unwrap().waves, [(0, 5), (0, 2)]);
        assert!(!bank.cue("Battle").unwrap().looped);
        assert!(SoundBank::read(b"SDBK").is_err());
    }
}
