//! Little-endian binary access helpers with bounds checking.
//!
//! Every TA format stores absolute offsets to other structures, so parsers work on a
//! whole-file byte slice and read fields at explicit offsets rather than streaming.

use crate::{FormatError, Result};

#[inline]
pub(crate) fn slice(data: &[u8], offset: usize, len: usize) -> Result<&[u8]> {
    offset
        .checked_add(len)
        .and_then(|end| data.get(offset..end))
        .ok_or(FormatError::Eof { offset, needed: len })
}

/// Validate a complete table before reserving memory from its untrusted count.
pub(crate) fn table(data: &[u8], offset: usize, count: usize, stride: usize) -> Result<&[u8]> {
    let len = count.checked_mul(stride).ok_or_else(|| FormatError::invalid("table", "size overflow"))?;
    slice(data, offset, len)
}

/// Aggregate work/allocation allowance, including repeated references to the same data.
pub(crate) struct Budget(pub usize);

impl Budget {
    pub fn take(&mut self, amount: usize, format: &'static str) -> Result<()> {
        self.0 = self.0.checked_sub(amount).ok_or_else(|| FormatError::invalid(format, "decoded content exceeds resource limit"))?;
        Ok(())
    }
}

#[inline]
pub(crate) fn u8_at(data: &[u8], offset: usize) -> Result<u8> {
    data.get(offset).copied().ok_or(FormatError::Eof { offset, needed: 1 })
}

#[inline]
pub(crate) fn u16_at(data: &[u8], offset: usize) -> Result<u16> {
    let b = slice(data, offset, 2)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

#[inline]
pub(crate) fn i16_at(data: &[u8], offset: usize) -> Result<i16> {
    Ok(u16_at(data, offset)? as i16)
}

#[inline]
pub(crate) fn u32_at(data: &[u8], offset: usize) -> Result<u32> {
    let b = slice(data, offset, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[inline]
pub(crate) fn i32_at(data: &[u8], offset: usize) -> Result<i32> {
    Ok(u32_at(data, offset)? as i32)
}

/// Reads a NUL-terminated string. TA data is Windows-1252; we decode as Latin-1, which
/// never fails and maps every byte to a char.
pub(crate) fn cstr_at(data: &[u8], offset: usize) -> Result<String> {
    let rest = data.get(offset..).ok_or(FormatError::Eof { offset, needed: 1 })?;
    // Binary names are short. Bound scans and allocations even when thousands of
    // pointers alias a long, unterminated string in a corrupt archive/model/script.
    let end = rest.iter().take(4097).position(|&b| b == 0).unwrap_or(rest.len());
    if end > 4096 {
        return Err(FormatError::invalid("string", "name exceeds 4096 bytes"));
    }
    Ok(latin1(&rest[..end]))
}

/// Reads a fixed-size, NUL-padded string field.
pub(crate) fn fixed_str(data: &[u8], offset: usize, len: usize) -> Result<String> {
    let b = slice(data, offset, len)?;
    let end = b.iter().position(|&c| c == 0).unwrap_or(len);
    Ok(latin1(&b[..end]))
}

pub(crate) fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}
