//! HPI ("HAPI") archive reader.
//!
//! Total Annihilation packs its data into HPI v1 archives. The same container is used under
//! several extensions: `.hpi` (base game), `.ccx` (Core Contingency / Battle Tactics),
//! `.gp3`/`.gpf` (patches) and `.ufo` (downloadable units).
//!
//! Layout:
//! * 20-byte header: `"HAPI"`, version `0x00010000`, directory end offset, header key,
//!   directory start offset.
//! * If the header key is non-zero every byte at file position `p` is obfuscated:
//!   `plain = ((p ^ key) as u8) ^ !cipher` where `key = !((hk << 2) | (hk >> 6))`.
//! * The directory is a tree of `{entry count, entry list offset}` nodes whose entries are
//!   packed 9-byte records `{name offset, data offset, is_directory}`. All offsets are
//!   absolute file offsets.
//! * File records are `{data offset, uncompressed size, compression}`. Compressed files are
//!   split into 64 KiB chunks, each prefixed by a 19-byte `"SQSH"` header and compressed with
//!   either TA's LZ77 variant (method 1) or zlib (method 2). Chunks may additionally be
//!   scrambled with a position-dependent byte transform.

use std::path::{Path, PathBuf};

use memmap2::Mmap;

use crate::bin::{Budget, cstr_at, slice, table, u32_at, u8_at};
use crate::{FormatError, Result};

const HAPI_MARKER: u32 = 0x4950_4148; // "HAPI"
const VERSION_TA: u32 = 0x0001_0000;
const VERSION_SAVEGAME: u32 = 0x4B4E_4142; // "BANK"
const SQSH_MARKER: u32 = 0x4853_5153; // "SQSH"
const HEADER_LEN: usize = 20;
const CHUNK_LEN: usize = 65536;
const CHUNK_HEADER_LEN: usize = 19;
const MAX_DIR_DEPTH: usize = 32;
const MAX_DIRECTORY_BYTES: usize = 64 << 20;
/// Whole-file readers reject larger entries before allocating (also used by the VFS).
pub const MAX_FILE_BYTES: usize = 256 << 20;

/// How a file's bytes are stored inside the archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    Lz77,
    Zlib,
}

/// A file entry in an HPI archive.
#[derive(Clone, Debug)]
pub struct HpiFile {
    /// Path inside the archive using `/` separators, in its original case.
    pub path: String,
    pub offset: u32,
    pub size: u32,
    pub compression: Compression,
}

enum Backing {
    Mapped(Mmap),
    Owned(Vec<u8>),
}

impl Backing {
    fn bytes(&self) -> &[u8] {
        match self {
            Backing::Mapped(m) => m,
            Backing::Owned(v) => v,
        }
    }
}

/// An opened HPI archive. Reads are lock-free and may happen from many threads at once.
pub struct HpiArchive {
    path: PathBuf,
    backing: Backing,
    /// Low byte of the stream key; only the low byte survives the per-byte truncation.
    key: u8,
    encrypted: bool,
    files: Vec<HpiFile>,
}

impl std::fmt::Debug for HpiArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HpiArchive")
            .field("path", &self.path)
            .field("encrypted", &self.encrypted)
            .field("files", &self.files.len())
            .finish()
    }
}

impl HpiArchive {
    /// Memory-maps and indexes an archive on disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = std::fs::File::open(&path)?;
        // SAFETY: the archive is opened read-only; TA data files are not modified while the
        // game runs. A concurrent external truncation would be a user error we cannot guard.
        let mmap = unsafe { Mmap::map(&file)? };
        Self::from_backing(path, Backing::Mapped(mmap))
    }

    /// Indexes an archive held in memory (used by tests and nested archives).
    pub fn from_bytes(name: impl Into<PathBuf>, bytes: Vec<u8>) -> Result<Self> {
        Self::from_backing(name.into(), Backing::Owned(bytes))
    }

    fn from_backing(path: PathBuf, backing: Backing) -> Result<Self> {
        let data = backing.bytes();
        if data.len() < HEADER_LEN {
            return Err(FormatError::invalid("HPI header", "file too small"));
        }
        let marker = u32_at(data, 0)?;
        if marker != HAPI_MARKER {
            return Err(FormatError::invalid("HPI header", format!("bad marker {marker:#010x}")));
        }
        let version = u32_at(data, 4)?;
        if version == VERSION_SAVEGAME {
            return Err(FormatError::invalid("HPI header", "saved-game BANK files are not archives"));
        }
        if version != VERSION_TA {
            return Err(FormatError::invalid(
                "HPI header",
                format!("unsupported version {version:#010x} (TA:Kingdoms archives are not supported)"),
            ));
        }
        let dir_end = u32_at(data, 8)? as usize;
        let header_key = u32_at(data, 12)?;
        let dir_start = u32_at(data, 16)? as usize;
        if dir_end > MAX_DIRECTORY_BYTES {
            return Err(FormatError::invalid("HPI header", "directory exceeds 64 MiB resource limit"));
        }
        if dir_start < HEADER_LEN || dir_start > dir_end || dir_end > data.len() {
            return Err(FormatError::invalid(
                "HPI header",
                format!("directory range {dir_start}..{dir_end} outside file of {} bytes", data.len()),
            ));
        }

        let encrypted = header_key != 0;
        let key = if encrypted { !((header_key << 2) | (header_key >> 6)) as u8 } else { 0 };

        let mut archive = HpiArchive { path, backing, key, encrypted, files: Vec::new() };

        // Directory offsets are absolute, so decode into a buffer positioned at 0.
        let mut dir = vec![0u8; dir_end];
        archive.read_decrypted_into(dir_start, &mut dir[dir_start..])?;
        let mut files = Vec::new();
        walk_directory(&dir, dir_start, String::new(), 0, &mut files, &mut std::collections::HashSet::new(), &mut Budget(MAX_DIRECTORY_BYTES))?;
        // (Two compressed entries at one place: each read would inflate the same data again, a
        // small archive filling memory. TA's own never share. An empty file reads nothing from
        // its place, which some packers, Total Mayhem's, give as the next file's.)
        let mut packed = std::collections::HashSet::new();
        if files.iter().filter(|f| f.compression != Compression::None && f.size > 0).any(|f| !packed.insert(f.offset)) {
            return Err(FormatError::invalid("HPI directory", "compressed entries share their data"));
        }
        archive.files = files;
        Ok(archive)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn files(&self) -> &[HpiFile] {
        &self.files
    }

    pub fn is_encrypted(&self) -> bool {
        self.encrypted
    }

    /// Reads and fully decompresses a file.
    pub fn read(&self, file: &HpiFile) -> Result<Vec<u8>> {
        let size = file.size as usize;
        let offset = file.offset as usize;
        if size > MAX_FILE_BYTES {
            return Err(FormatError::invalid("HPI file", "decoded size exceeds 256 MiB"));
        }
        match file.compression {
            Compression::None => {
                slice(self.backing.bytes(), offset, size)?;
                let mut out = vec![0u8; size];
                self.read_decrypted_into(offset, &mut out)?;
                Ok(out)
            }
            Compression::Lz77 | Compression::Zlib => {
                let chunk_count = size.div_ceil(CHUNK_LEN);
                let mut sizes = vec![0u8; chunk_count * 4];
                self.read_decrypted_into(offset, &mut sizes)?;
                let mut pos = offset + sizes.len();
                // Validate every compressed extent before allocating any chunk buffer.
                let mut end = pos;
                for i in 0..chunk_count {
                    let len = u32_at(&sizes, i * 4)? as usize;
                    if !(CHUNK_HEADER_LEN..=CHUNK_LEN * 2).contains(&len) {
                        return Err(FormatError::invalid("HPI chunk", "compressed chunk size exceeds resource limit"));
                    }
                    slice(self.backing.bytes(), end, len)?;
                    end += len;
                }
                let mut out = Vec::new();
                let mut chunk = Vec::new();
                let mut zlib = None;
                for i in 0..chunk_count {
                    let chunk_len = u32_at(&sizes, i * 4)? as usize;
                    chunk.resize(chunk_len, 0);
                    self.read_decrypted_into(pos, &mut chunk)?;
                    pos += chunk_len;
                    let remaining = (size - out.len()).min(CHUNK_LEN);
                    decode_chunk(&mut chunk, &mut out, remaining, &mut zlib).map_err(|e| {
                        FormatError::invalid(
                            "HPI chunk",
                            format!("{} chunk {i} of '{}': {e}", self.path.file_name().map_or_else(|| self.path.display().to_string(), |n| n.to_string_lossy().into_owned()), file.path),
                        )
                    })?;
                }
                if out.len() != size {
                    return Err(FormatError::invalid(
                        "HPI file",
                        format!("'{}' decompressed to {} bytes, expected {size}", file.path, out.len()),
                    ));
                }
                Ok(out)
            }
        }
    }

    /// Copies `out.len()` bytes starting at `offset`, undoing the archive-level obfuscation.
    fn read_decrypted_into(&self, offset: usize, out: &mut [u8]) -> Result<()> {
        let data = self.backing.bytes();
        let src = offset
            .checked_add(out.len())
            .and_then(|end| data.get(offset..end))
            .ok_or(FormatError::Eof { offset, needed: out.len() })?;
        if self.encrypted {
            for (i, (dst, &b)) in out.iter_mut().zip(src).enumerate() {
                let tkey = (offset.wrapping_add(i) as u8) ^ self.key;
                *dst = tkey ^ !b;
            }
        } else {
            out.copy_from_slice(src);
        }
        Ok(())
    }
}

fn walk_directory(
    dir: &[u8],
    node_offset: usize,
    prefix: String,
    depth: usize,
    files: &mut Vec<HpiFile>,
    visited: &mut std::collections::HashSet<usize>,
    budget: &mut Budget,
) -> Result<()> {
    if depth > MAX_DIR_DEPTH || visited.len() >= 100_000 || !visited.insert(node_offset) {
        return Err(FormatError::invalid("HPI directory", "nesting too deep (corrupt archive?)"));
    }
    let count = u32_at(dir, node_offset)? as usize;
    let list = u32_at(dir, node_offset + 4)? as usize;
    if count > dir.len() / 9 {
        return Err(FormatError::invalid("HPI directory", format!("implausible entry count {count}")));
    }
    table(dir, list, count, 9)?;
    budget.take(count * std::mem::size_of::<HpiFile>(), "HPI directory")?;
    for i in 0..count {
        let entry = list + i * 9;
        let name_offset = u32_at(dir, entry)? as usize;
        let data_offset = u32_at(dir, entry + 4)? as usize;
        let is_dir = u8_at(dir, entry + 8)? != 0;
        let name = cstr_at(dir, name_offset)?;
        if name.split(['/', '\\']).any(|part| {
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' '])
                || part.chars().any(|c| c.is_control() || "<>:\"|?*".contains(c))
                || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$")
                || stem.strip_prefix("COM").or_else(|| stem.strip_prefix("LPT"))
                    .is_some_and(|n| matches!(n, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"))
        }) {
            return Err(FormatError::invalid("HPI directory", "unsafe entry path"));
        }
        let path = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        budget.take(path.len(), "HPI directory")?;
        if is_dir {
            walk_directory(dir, data_offset, path, depth + 1, files, visited, budget)?;
        } else {
            let offset = u32_at(dir, data_offset)?;
            let size = u32_at(dir, data_offset + 4)?;
            let compression = match u8_at(dir, data_offset + 8)? {
                0 => Compression::None,
                1 => Compression::Lz77,
                2 => Compression::Zlib,
                other => {
                    return Err(FormatError::invalid(
                        "HPI file entry",
                        format!("'{path}' has unknown compression flag {other}"),
                    ));
                }
            };
            files.push(HpiFile { path, offset, size, compression });
        }
    }
    Ok(())
}

/// Decodes one `SQSH` chunk (in place descrambling) and appends its output.
fn decode_chunk(chunk: &mut [u8], out: &mut Vec<u8>, remaining: usize, zlib: &mut Option<flate2::Decompress>) -> Result<()> {
    if chunk.len() < CHUNK_HEADER_LEN {
        return Err(FormatError::invalid("SQSH", "chunk shorter than header"));
    }
    let marker = u32_at(chunk, 0)?;
    if marker != SQSH_MARKER {
        return Err(FormatError::invalid("SQSH", format!("bad marker {marker:#010x}")));
    }
    let method = chunk[5];
    let scrambled = chunk[6] != 0;
    let comp_len = u32_at(chunk, 7)? as usize;
    let decomp_len = u32_at(chunk, 11)? as usize;
    if decomp_len > remaining || decomp_len > CHUNK_LEN {
        return Err(FormatError::invalid("SQSH", "decoded chunk exceeds file/chunk limit"));
    }
    let checksum = u32_at(chunk, 15)?;
    let payload = chunk
        .get_mut(CHUNK_HEADER_LEN..CHUNK_HEADER_LEN + comp_len)
        .ok_or(FormatError::Eof { offset: CHUNK_HEADER_LEN, needed: comp_len })?;

    // The checksum covers the scrambled bytes.
    let sum = payload.iter().fold(0u32, |acc, &b| acc.wrapping_add(b as u32));
    if sum != checksum {
        return Err(FormatError::invalid("SQSH", format!("checksum {sum:#x} != {checksum:#x}")));
    }
    if scrambled {
        for (x, b) in payload.iter_mut().enumerate() {
            let x = x as u8;
            *b = b.wrapping_sub(x) ^ x;
        }
    }

    let before = out.len();
    match method {
        0 if payload.len() <= decomp_len => out.extend_from_slice(payload),
        0 => return Err(FormatError::invalid("SQSH", "stored payload exceeds declared output")),
        1 => lz77_decompress(payload, out, before + decomp_len)?,
        2 => {
            // Every chunk is an independent stream. Reuse its state for this file and
            // decode directly into bounded output instead of rebuilding a buffered
            // reader and copying its output for every 64 KiB chunk.
            let decoder = zlib.get_or_insert_with(|| flate2::Decompress::new(true));
            decoder.reset(true);
            // One extra byte detects streams that exceed the declared size, just as
            // the previous Read::take limit did. Header/extent checks precede this.
            out.resize(before + decomp_len + 1, 0);
            let status = decoder.decompress(payload, &mut out[before..], flate2::FlushDecompress::Finish)
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "corrupt deflate stream"))?;
            out.truncate(before + decoder.total_out() as usize);
            if status != flate2::Status::StreamEnd {
                return Err(FormatError::invalid("SQSH", "incomplete or oversized deflate stream"));
            }
        }
        other => return Err(FormatError::invalid("SQSH", format!("unknown method {other}"))),
    }
    let produced = out.len() - before;
    if produced != decomp_len {
        return Err(FormatError::invalid(
            "SQSH",
            format!("decompressed {produced} bytes, header says {decomp_len}"),
        ));
    }
    Ok(())
}

/// TA's LZ77 variant: a 4 KiB ring buffer starting at position 1, 8 tag bits per control
/// byte (LSB first), literals for clear bits and 16-bit back-references for set bits
/// (`offset = v >> 4`, `length = (v & 15) + 2`). A back-reference with offset 0 terminates.
fn lz77_decompress(input: &[u8], out: &mut Vec<u8>, limit: usize) -> Result<()> {
    let eof = |offset: usize| FormatError::Eof { offset, needed: 1 };
    let mut window = [0u8; 4096];
    let mut wpos = 1usize;
    let mut ip = 0usize;
    let mut tag = *input.first().ok_or(eof(0))? as u32;
    ip += 1;
    let mut bit = 1u32;
    loop {
        if tag & bit == 0 {
            if out.len() >= limit { return Err(FormatError::invalid("LZ77", "output exceeds declared length")); }
            let b = *input.get(ip).ok_or(eof(ip))?;
            ip += 1;
            out.push(b);
            window[wpos] = b;
            wpos = (wpos + 1) & 0xFFF;
        } else {
            let lo = *input.get(ip).ok_or(eof(ip))? as usize;
            let hi = *input.get(ip + 1).ok_or(eof(ip + 1))? as usize;
            ip += 2;
            let v = lo | (hi << 8);
            let mut src = v >> 4;
            if src == 0 {
                return Ok(());
            }
            if out.len().saturating_add((v & 0x0F) + 2) > limit { return Err(FormatError::invalid("LZ77", "output exceeds declared length")); }
            for _ in 0..(v & 0x0F) + 2 {
                let b = window[src];
                out.push(b);
                window[wpos] = b;
                src = (src + 1) & 0xFFF;
                wpos = (wpos + 1) & 0xFFF;
            }
        }
        bit <<= 1;
        if bit & 0x100 != 0 {
            bit = 1;
            tag = *input.get(ip).ok_or(eof(ip))? as u32;
            ip += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lz77_literals_and_backref() {
        // tag 0b0000_0100: two literals ('a','b'), then a back-reference to window pos 1
        // with length 2+2=4 → "abab" repeated pattern, then a terminator.
        // Window: pos1='a', pos2='b'. ref v = (1 << 4) | 2  → src 1, len 4 → a b a b
        let mut input = vec![0b0000_1100u8, b'a', b'b'];
        input.extend_from_slice(&((1u16 << 4) | 2).to_le_bytes());
        input.extend_from_slice(&0u16.to_le_bytes()); // terminator (tag bit 3 set)
        let mut out = Vec::new();
        lz77_decompress(&input, &mut out, 6).unwrap();
        assert_eq!(out, b"ababab");
    }

    #[test]
    fn rejects_garbage() {
        assert!(HpiArchive::from_bytes("x", vec![0; 64]).is_err());
    }
}
