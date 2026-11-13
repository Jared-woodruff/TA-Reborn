//! Supreme Commander's game data archives (`gamedata/*.scd`): plain zip files. A small reader
//! of our own (no zip library): the central directory read once, a member read on demand by
//! its path, whatever its case or slashes, stored or deflated, its CRC checked. `env.scd` is
//! over a gigabyte, so nothing is read that is not asked for, and reads are positional (any
//! number of threads share one archive).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// The most a member may unpack to (the largest stratum texture is a few megabytes).
pub const MAX_MEMBER: u64 = 256 << 20;
/// The most members an archive may list (FA's largest, `units.scd`, has some tens of thousands).
const MAX_MEMBERS: usize = 1 << 20;

const EOCD: u32 = 0x0605_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

#[derive(Clone, Debug)]
struct Member {
    method: u16,
    crc: u32,
    packed: u64,
    size: u64,
    /// Where its local header starts.
    local: u64,
}

/// An open `.scd` archive.
pub struct Scd {
    path: PathBuf,
    file: File,
    /// Members by their normalised path ([`normalise`]).
    members: BTreeMap<String, Member>,
}

/// A path as the archives are looked up by: lower case, `/` between names, no leading `/`.
pub fn normalise(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase()
}

impl Scd {
    /// Reads the archive's directory.
    pub fn open(path: &Path) -> Result<Scd> {
        let file = File::open(path).map_err(|e| Error::io(path, e))?;
        let len = file.metadata().map_err(|e| Error::io(path, e))?.len();
        let bad = |what: &str| Error::Format(format!("{}: {what}", path.display()));
        // The end record: 22 bytes, then a comment of up to 65,535.
        let tail_len = len.min(22 + 65_535);
        let tail = read_at(&file, len - tail_len, tail_len as usize).map_err(|e| Error::io(path, e))?;
        let at = (0..tail.len().saturating_sub(21)).rev().find(|&i| u32_le(&tail, i) == EOCD && i + 22 + u16_le(&tail, i + 20) as usize <= tail.len())
            .ok_or_else(|| bad("not a zip archive"))?;
        let count = u16_le(&tail, at + 10) as usize;
        let dir_size = u32_le(&tail, at + 12) as u64;
        let dir_at = u32_le(&tail, at + 16) as u64;
        if count == 0xFFFF || dir_size == 0xFFFF_FFFF || dir_at == 0xFFFF_FFFF {
            return Err(bad("a zip64 archive (not one of Supreme Commander's)"));
        }
        if dir_at.checked_add(dir_size).is_none_or(|end| end > len) || count > MAX_MEMBERS {
            return Err(bad("damaged directory"));
        }
        let dir = read_at(&file, dir_at, dir_size as usize).map_err(|e| Error::io(path, e))?;
        let mut members = BTreeMap::new();
        let mut o = 0usize;
        for _ in 0..count {
            if o + 46 > dir.len() || u32_le(&dir, o) != CENTRAL {
                return Err(bad("damaged directory entry"));
            }
            let flags = u16_le(&dir, o + 8);
            let method = u16_le(&dir, o + 10);
            let crc = u32_le(&dir, o + 16);
            let packed = u32_le(&dir, o + 20) as u64;
            let size = u32_le(&dir, o + 24) as u64;
            let (name_len, extra_len, comment_len) = (u16_le(&dir, o + 28) as usize, u16_le(&dir, o + 30) as usize, u16_le(&dir, o + 32) as usize);
            let local = u32_le(&dir, o + 42) as u64;
            let end = o + 46 + name_len + extra_len + comment_len;
            if end > dir.len() {
                return Err(bad("damaged directory entry"));
            }
            let name = String::from_utf8_lossy(&dir[o + 46..o + 46 + name_len]).into_owned();
            o = end;
            // (Folders, and members a reader of ours cannot open: encrypted ones.)
            if name.ends_with('/') || name.ends_with('\\') || flags & 1 != 0 {
                continue;
            }
            members.entry(normalise(&name)).or_insert(Member { method, crc, packed, size, local });
        }
        Ok(Scd { path: path.to_path_buf(), file, members })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the archive holds `path` (any case, either slash).
    pub fn contains(&self, path: &str) -> bool {
        self.members.contains_key(&normalise(path))
    }

    /// The members' normalised paths under `prefix` (normalised too), in order.
    pub fn list<'a>(&'a self, prefix: &str) -> impl Iterator<Item = &'a str> + 'a {
        let prefix = normalise(prefix);
        self.members.range(prefix.clone()..).map(|(k, _)| k.as_str()).take_while(move |k| k.starts_with(&prefix))
    }

    /// A member's bytes, unpacked and checked; `None` when the archive has no such member.
    pub fn read(&self, path: &str) -> Result<Option<Vec<u8>>> {
        let Some(m) = self.members.get(&normalise(path)) else { return Ok(None) };
        let bad = |what: &str| Error::Format(format!("{} in {}: {what}", path, self.path.display()));
        if m.size > MAX_MEMBER || m.packed > MAX_MEMBER {
            return Err(bad("too large"));
        }
        let head = read_at(&self.file, m.local, 30).map_err(|e| Error::io(&self.path, e))?;
        if u32_le(&head, 0) != LOCAL {
            return Err(bad("damaged member"));
        }
        let data_at = m.local + 30 + u16_le(&head, 26) as u64 + u16_le(&head, 28) as u64;
        let packed = read_at(&self.file, data_at, m.packed as usize).map_err(|e| Error::io(&self.path, e))?;
        let bytes = match m.method {
            0 => packed,
            8 => {
                let mut out = Vec::with_capacity(m.size as usize);
                flate2::read::DeflateDecoder::new(&packed[..]).take(m.size + 1).read_to_end(&mut out).map_err(|_| bad("damaged deflate stream"))?;
                out
            }
            other => return Err(bad(&format!("packed by method {other}"))),
        };
        if bytes.len() as u64 != m.size {
            return Err(bad("unpacked to the wrong size"));
        }
        let mut crc = flate2::Crc::new();
        crc.update(&bytes);
        if crc.sum() != m.crc {
            return Err(bad("checksum mismatch"));
        }
        Ok(Some(bytes))
    }
}

fn u16_le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// `len` bytes at `offset`, without moving a shared file cursor (threads read one archive).
fn read_at(file: &File, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    let mut done = 0;
    while done < len {
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_read(file, &mut buf[done..], offset + done as u64)?;
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::read_at(file, &mut buf[done..], offset + done as u64)?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "archive ends early"));
        }
        done += n;
    }
    Ok(buf)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// A zip of `files` (stored or deflated), as Supreme Commander's archives are made.
    pub(crate) fn zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for &(name, data, deflate) in files {
            let packed = if deflate {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let mut crc = flate2::Crc::new();
            crc.update(data);
            let local = out.len() as u32;
            let method: u16 = if deflate { 8 } else { 0 };
            out.extend_from_slice(&LOCAL.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&crc.sum().to_le_bytes());
            out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&packed);
            central.extend_from_slice(&CENTRAL.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 4]);
            central.extend_from_slice(&crc.sum().to_le_bytes());
            central.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&local.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let dir_at = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&EOCD.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&dir_at.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn members_are_found_in_any_case_and_unpacked_and_checked() {
        let dir = crate::tests::Temp::new("scd");
        let big: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        let path = dir.0.join("env.scd");
        std::fs::write(&path, zip(&[("env/Evergreen/layers/Grass_albedo.dds", &big, true), ("env/evergreen/props/rock01_prop.bp", b"PropBlueprint {}", false), ("env/", b"", false)])).unwrap();
        let scd = Scd::open(&path).unwrap();
        assert_eq!(scd.read("/ENV/evergreen/LAYERS/grass_albedo.dds").unwrap().unwrap(), big);
        assert_eq!(scd.read("env\\evergreen\\props\\rock01_prop.bp").unwrap().unwrap(), b"PropBlueprint {}");
        assert!(scd.read("env/evergreen/layers/missing.dds").unwrap().is_none());
        assert_eq!(scd.list("/env/evergreen/").collect::<Vec<_>>(), ["env/evergreen/layers/grass_albedo.dds", "env/evergreen/props/rock01_prop.bp"]);
        // A member whose bytes were changed fails its checksum.
        let mut bytes = std::fs::read(&path).unwrap();
        let at = bytes.windows(16).position(|w| w == b"PropBlueprint {}").unwrap();
        bytes[at] = b'X';
        std::fs::write(&path, bytes).unwrap();
        assert!(Scd::open(&path).unwrap().read("env/evergreen/props/rock01_prop.bp").is_err());
        // Not an archive at all.
        std::fs::write(&path, b"not a zip").unwrap();
        assert!(Scd::open(&path).is_err());
    }
}
