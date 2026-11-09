//! Bounded record compatibility metadata shared by the game and launcher. The fixed
//! trailer follows the command-history body and precedes the optional career trailer.
//! Its small, fixed size lets the launcher inspect it without loading a whole battle.
//!
//! From version 46 the mods a record was made with ([`RecordMod`], in load order) come between
//! the body and that trailer, in a trailer of their own ([`append_mods`]): read from its end, as
//! the launcher reads a record's last bytes. A file ends so:
//! `body | mods (46 on) | compatibility metadata (41 on) | career trailer (optional)`.

use std::io;

pub mod mods;

pub const RECORD_METADATA_LEN: usize = 128;
pub const MAX_RECORD_BYTES: u64 = 512 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"TAREC001";
const BUILD_BYTES: usize = 96;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordMetadata {
    pub simulation_version: u32,
    /// Fingerprint of the shared rules plus this battle's map and mission.
    pub match_data_hash: u64,
    /// Displayable build identity; it does not replace the simulation revision.
    pub engine_build: String,
    /// The stream includes the AI's issued orders as well as the human players'.
    pub full_commands: bool,
}

/// These layouts were checked against their original definitions. Never expand this
/// range just to bypass a version failure: a new layout needs its own decoder/migration.
pub const fn record_version_readable(version: u32) -> bool {
    // (45 is 44's layout: only what its data hash covers changed; 46 adds the mods' trailer.)
    matches!(version, 36..=46)
}

/// Records of this version keep the mods they were made with ([`split_mods`]); older ones do
/// not say (they are told by their data hash alone, as before).
pub const fn record_has_mods(version: u32) -> bool {
    version >= 46
}

pub fn simulation_compatible(version: u32, metadata: Option<&RecordMetadata>) -> bool {
    match version {
        36..=40 => metadata.is_none() && version == crate::SIMULATION_VERSION,
        41..=46 => metadata.is_some_and(|m| m.simulation_version == crate::SIMULATION_VERSION),
        _ => false,
    }
}

/// A mod a record was made with, as it was then: what a player is told to get or turn on, and
/// what tells another copy of it from the same.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RecordMod {
    /// Its id (its folder's name, in lower case, as the settings' load order has it).
    pub id: String,
    pub name: String,
    /// As its manifest gives it ("" when it gives none).
    pub version: String,
    /// The digest of its files as installed (`ta_vfs::mods::content_digest`); all zero when they
    /// could not be read then ([`RecordMod::digest_known`]).
    pub digest: [u8; 16],
}

impl RecordMod {
    pub fn digest_known(&self) -> bool {
        self.digest != [0; 16]
    }

    /// "Total Mayhem 11.3.0", or the name alone when it has no version.
    pub fn label(&self) -> String {
        let name = if self.name.trim().is_empty() { &self.id } else { &self.name };
        match self.version.trim() {
            "" => name.trim().to_string(),
            version => format!("{} {version}", name.trim()),
        }
    }
}

const MODS_MAGIC: &[u8; 8] = b"TAMODS01";
/// More mods than any load order holds (each is a folder or an archive of the mods folders).
pub const MAX_RECORD_MODS: usize = 1024;
/// A mod's id, name or version, in bytes of UTF-8 (an id is a folder's name: at most 255
/// UTF-16 units, 765 bytes).
pub const MAX_MOD_TEXT: usize = 1024;
/// What ends the mods' trailer: its length (a `u32`) and its magic.
pub const MODS_TRAILER_END: usize = 12;
const MODS_TRAILER_MIN: usize = 8 + 2 + MODS_TRAILER_END;
/// The longest a trailer of [`MAX_RECORD_MODS`] mods can be.
pub const MAX_MODS_TRAILER: usize = MODS_TRAILER_MIN + MAX_RECORD_MODS * (6 + 3 * MAX_MOD_TEXT + 16);

/// The text as a record keeps it: no control characters (a manifest's are spaces), at most
/// [`MAX_MOD_TEXT`] bytes (cut at a character).
fn record_text(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().map(|c| if c.is_control() { ' ' } else { c }) {
        if out.len() + c.len_utf8() > MAX_MOD_TEXT { break; }
        out.push(c);
    }
    out
}

/// Appends the mods' trailer (`mods` in load order; [`MAX_RECORD_MODS`] at most): its magic, their
/// count, each mod's id, name and version (a `u16` length and the UTF-8) and digest, then the
/// trailer's whole length and its magic again, so it is found from its end.
pub fn append_mods(bytes: &mut Vec<u8>, mods: &[RecordMod]) -> io::Result<()> {
    if mods.len() > MAX_RECORD_MODS { return Err(invalid("too many mods for a record")); }
    let start = bytes.len();
    bytes.extend_from_slice(MODS_MAGIC);
    bytes.extend_from_slice(&(mods.len() as u16).to_le_bytes());
    for m in mods {
        for text in [&m.id, &m.name, &m.version] {
            let text = record_text(text);
            bytes.extend_from_slice(&(text.len() as u16).to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
        }
        bytes.extend_from_slice(&m.digest);
    }
    let len = bytes.len() - start + MODS_TRAILER_END;
    bytes.extend_from_slice(&(len as u32).to_le_bytes());
    bytes.extend_from_slice(MODS_MAGIC);
    Ok(())
}

/// The whole length of the mods' trailer `bytes` end with, from its last [`MODS_TRAILER_END`]
/// bytes (how much of a file's end to read for it), or `None` when they are not its end.
pub fn mods_trailer_len(bytes: &[u8]) -> Option<usize> {
    let end = bytes.get(bytes.len().checked_sub(MODS_TRAILER_END)?..)?;
    if &end[4..] != MODS_MAGIC { return None; }
    let len = u32::from_le_bytes(end[..4].try_into().ok()?) as usize;
    (MODS_TRAILER_MIN..=MAX_MODS_TRAILER).contains(&len).then_some(len)
}

/// Splits the mods' trailer off a record of version 46 or later (after its compatibility
/// metadata is split off): the body before it, and the mods in load order. A record of that
/// version without one, or with one that does not read, is an error.
pub fn split_mods(bytes: &[u8]) -> io::Result<(&[u8], Vec<RecordMod>)> {
    let len = mods_trailer_len(bytes).ok_or_else(|| invalid("missing record mods"))?;
    let start = bytes.len().checked_sub(len).ok_or_else(|| invalid("truncated record mods"))?;
    let trailer = &bytes[start..bytes.len() - MODS_TRAILER_END];
    if &trailer[..8] != MODS_MAGIC { return Err(invalid("invalid record mods")); }
    let mut at = 8;
    let mut take = |n: usize| -> io::Result<&[u8]> {
        let piece = trailer.get(at..at + n).ok_or_else(|| invalid("truncated record mods"))?;
        at += n;
        Ok(piece)
    };
    let count = u16::from_le_bytes(take(2)?.try_into().unwrap()) as usize;
    if count > MAX_RECORD_MODS { return Err(invalid("too many record mods")); }
    let mut mods = Vec::with_capacity(count.min(64));
    for _ in 0..count {
        let mut text = || -> io::Result<String> {
            let n = u16::from_le_bytes(take(2)?.try_into().unwrap()) as usize;
            if n > MAX_MOD_TEXT { return Err(invalid("record mod text exceeds size limit")); }
            let text = std::str::from_utf8(take(n)?).map_err(|_| invalid("invalid record mod text"))?;
            if text.chars().any(char::is_control) { return Err(invalid("invalid record mod text")); }
            Ok(text.to_string())
        };
        let (id, name, version) = (text()?, text()?, text()?);
        let digest = take(16)?.try_into().unwrap();
        mods.push(RecordMod { id, name, version, digest });
    }
    if at != trailer.len() { return Err(invalid("invalid record mods")); }
    Ok((&bytes[..start], mods))
}

fn invalid(message: &'static str) -> io::Error { io::Error::new(io::ErrorKind::InvalidData, message) }

/// Call after removing a validated career trailer. A malformed recognized trailer is
/// an error; unrecognized trailing bytes remain in the body for its strict decoder.
pub fn split_metadata(bytes: &[u8]) -> io::Result<(&[u8], Option<RecordMetadata>)> {
    let Some(start) = bytes.len().checked_sub(RECORD_METADATA_LEN) else { return Ok((bytes, None)) };
    let trailer = &bytes[start..];
    if &trailer[..8] != MAGIC { return Ok((bytes, None)); }
    let simulation_version = u32::from_le_bytes(trailer[8..12].try_into().unwrap());
    let match_data_hash = u64::from_le_bytes(trailer[12..20].try_into().unwrap());
    let len = trailer[21] as usize;
    if simulation_version == 0 || trailer[20] > 1 || len > BUILD_BYTES || trailer[22 + len..].iter().any(|b| *b != 0) {
        return Err(invalid("invalid record compatibility metadata"));
    }
    let engine_build = std::str::from_utf8(&trailer[22..22 + len]).map_err(|_| invalid("invalid record build identity"))?;
    if engine_build.chars().any(char::is_control) { return Err(invalid("invalid record build identity")); }
    Ok((&bytes[..start], Some(RecordMetadata { simulation_version, match_data_hash, engine_build: engine_build.to_owned(), full_commands: trailer[20] != 0 })))
}

pub fn append_metadata(bytes: &mut Vec<u8>, metadata: &RecordMetadata) -> io::Result<()> {
    if metadata.simulation_version == 0 || metadata.engine_build.len() > BUILD_BYTES || metadata.engine_build.chars().any(char::is_control) {
        return Err(invalid("invalid record compatibility metadata"));
    }
    let mut trailer = [0; RECORD_METADATA_LEN];
    trailer[..8].copy_from_slice(MAGIC);
    trailer[8..12].copy_from_slice(&metadata.simulation_version.to_le_bytes());
    trailer[12..20].copy_from_slice(&metadata.match_data_hash.to_le_bytes());
    trailer[20] = u8::from(metadata.full_commands);
    trailer[21] = metadata.engine_build.len() as u8;
    trailer[22..22 + metadata.engine_build.len()].copy_from_slice(metadata.engine_build.as_bytes());
    bytes.extend_from_slice(&trailer);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> RecordMetadata {
        RecordMetadata { simulation_version: crate::SIMULATION_VERSION, match_data_hash: 123, engine_build: "2.1.0 (abc123)".into(), full_commands: true }
    }

    #[test]
    fn trailer_roundtrip_is_bounded_and_leaves_body_unchanged() {
        let mut bytes = b"body".to_vec();
        append_metadata(&mut bytes, &fixture()).unwrap();
        assert_eq!(bytes.len(), 4 + RECORD_METADATA_LEN);
        let (body, metadata) = split_metadata(&bytes).unwrap();
        assert_eq!(body, b"body");
        assert_eq!(metadata, Some(fixture()));
        assert_eq!(split_metadata(b"legacy").unwrap(), (&b"legacy"[..], None));
    }

    #[test]
    fn corrupt_metadata_is_not_silently_accepted() {
        let mut valid = Vec::new();
        append_metadata(&mut valid, &fixture()).unwrap();
        for (at, value) in [(20, 2), (21, 255), (22, 255), (127, 1)] {
            let mut corrupt = valid.clone();
            corrupt[at] = value;
            assert!(split_metadata(&corrupt).is_err());
        }
        let mut metadata = fixture();
        metadata.engine_build = "x".repeat(BUILD_BYTES + 1);
        assert!(append_metadata(&mut Vec::new(), &metadata).is_err());
        metadata.engine_build = "bad\nidentity".into();
        assert!(append_metadata(&mut Vec::new(), &metadata).is_err());
    }

    fn mayhem() -> RecordMod {
        RecordMod { id: "totalmayhem".into(), name: "Total Mayhem".into(), version: "11.3.0".into(), digest: [7; 16] }
    }

    #[test]
    fn the_mods_trailer_roundtrips_and_is_found_from_its_end() {
        let mods = vec![mayhem(), RecordMod { id: "übermacht".into(), name: "Übermacht".into(), version: String::new(), digest: [0; 16] }];
        for list in [Vec::new(), mods] {
            let mut bytes = b"body".to_vec();
            append_mods(&mut bytes, &list).unwrap();
            assert_eq!(mods_trailer_len(&bytes), Some(bytes.len() - 4), "its length, from its last bytes alone");
            let (body, read) = split_mods(&bytes).unwrap();
            assert_eq!((body, read), (&b"body"[..], list.clone()));
        }
        assert_eq!(mayhem().label(), "Total Mayhem 11.3.0");
        assert!(!RecordMod::default().digest_known());
    }

    #[test]
    fn a_mods_trailer_that_does_not_read_is_refused_not_guessed() {
        let mut valid = b"body".to_vec();
        append_mods(&mut valid, &[mayhem()]).unwrap();
        assert!(split_mods(b"a record of before, its title last").is_err(), "a record of 46 needs one");
        for cut in 5..valid.len() {
            assert!(split_mods(&valid[cut..]).is_err(), "its start cut off");
            assert!(split_mods(&valid[..cut]).is_err(), "its end cut off");
        }
        // (A length past the bytes there are; a count beyond the mods given; text that is not UTF-8;
        // a control character; one byte too many inside.)
        let end = valid.len();
        let mut corrupt = valid.clone();
        corrupt[end - 12..end - 8].copy_from_slice(&(end as u32 + 1).to_le_bytes());
        assert!(split_mods(&corrupt).is_err());
        let mut corrupt = valid.clone();
        corrupt[12..14].copy_from_slice(&2u16.to_le_bytes());
        assert!(split_mods(&corrupt).is_err());
        let mut corrupt = valid.clone();
        corrupt[16] = 0xff;
        assert!(split_mods(&corrupt).is_err());
        let mut corrupt = valid.clone();
        corrupt[16] = b'\n';
        assert!(split_mods(&corrupt).is_err());
        let mut longer = b"body".to_vec();
        longer.extend_from_slice(&valid[4..end - 12]);
        longer.push(0);
        longer.extend_from_slice(&((end - 4 + 1) as u32).to_le_bytes());
        longer.extend_from_slice(MODS_MAGIC);
        assert!(split_mods(&longer).is_err());
        // A manifest's control characters and endless text are cut down as they are written.
        let mut odd = mayhem();
        odd.name = format!("Total\nMayhem{}", "é".repeat(MAX_MOD_TEXT));
        let mut bytes = Vec::new();
        append_mods(&mut bytes, &[odd]).unwrap();
        let read = split_mods(&bytes).unwrap().1;
        assert!(read[0].name.starts_with("Total Mayhem") && read[0].name.len() <= MAX_MOD_TEXT);
        assert!(append_mods(&mut Vec::new(), &vec![mayhem(); MAX_RECORD_MODS + 1]).is_err());
    }

    #[test]
    fn readable_layout_does_not_imply_safe_save_resume() {
        for version in 36..=46 { assert!(record_version_readable(version)); }
        for version in [0, 35, 47, u32::MAX] { assert!(!record_version_readable(version)); }
        assert!(record_has_mods(46) && !record_has_mods(45));
        assert!(simulation_compatible(46, Some(&fixture())));
        assert!(!simulation_compatible(46, None));
        // (The layouts before metadata name their simulation revision by their own number:
        // compatible only while the simulation is still that revision.)
        assert_eq!(simulation_compatible(40, None), crate::SIMULATION_VERSION == 40);
        assert!(!simulation_compatible(39, None));
        assert!(simulation_compatible(41, Some(&fixture())));
        assert!(simulation_compatible(42, Some(&fixture())));
        assert!(simulation_compatible(43, Some(&fixture())));
        assert!(simulation_compatible(44, Some(&fixture())));
        assert!(!simulation_compatible(42, None));
        assert!(!simulation_compatible(43, None));
        assert!(!simulation_compatible(44, None));
        assert!(!simulation_compatible(41, None));
        let mut changed = fixture();
        changed.simulation_version += 1;
        assert!(!simulation_compatible(41, Some(&changed)));
        changed.simulation_version = crate::SIMULATION_VERSION - 1;
        assert!(!simulation_compatible(46, Some(&changed)), "older simulation rules cannot safely resume a save");
        assert!(!simulation_compatible(40, Some(&fixture())));
    }
}
