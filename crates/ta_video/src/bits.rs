//! LSB-first bit reader shared by every Smacker bitstream.
//!
//! Smacker consumes each byte starting from its least significant bit, and multi-bit fields
//! are assembled least-significant bit first. Reading `n` bits therefore yields the low `n`
//! bits of the little-endian integer formed by the remaining stream, which lets the reader
//! keep a 64-bit window that is refilled a whole byte at a time.

use crate::{Result, VideoError};

pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    /// Index of the first byte not yet loaded into `window`.
    pos: usize,
    /// Buffered bits; bit 0 is the next bit of the stream. Bits at and above `count` are 0.
    window: u64,
    /// Number of valid bits in `window`.
    count: u32,
    /// Name of the stream, reported when it runs dry.
    what: &'static str,
}

#[inline]
fn low_mask(n: u32) -> u64 {
    debug_assert!(n <= 32);
    (1u64 << n) - 1
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8], what: &'static str) -> Self {
        BitReader { data, pos: 0, window: 0, count: 0, what }
    }

    /// Tops the window up to at least 57 bits, or with everything that is left.
    #[inline]
    fn refill(&mut self) {
        if self.count > 56 {
            return;
        }
        if let Some(chunk) = self.data.get(self.pos..self.pos + 8) {
            let mut word = [0u8; 8];
            word.copy_from_slice(chunk);
            let word = u64::from_le_bytes(word);
            // Only whole bytes are taken so that `pos` stays byte-aligned.
            let take = (64 - self.count) / 8;
            let bits = take * 8;
            let fresh = if bits == 64 { word } else { word & ((1u64 << bits) - 1) };
            self.window |= fresh << self.count;
            self.pos += take as usize;
            self.count += bits;
        } else {
            while self.count <= 56 {
                let Some(&b) = self.data.get(self.pos) else { break };
                self.window |= u64::from(b) << self.count;
                self.pos += 1;
                self.count += 8;
            }
        }
    }

    /// Returns the next `n` (≤ 32) bits without consuming them. Bits past the end of the
    /// stream read as zero; consuming them with [`BitReader::skip`] fails.
    #[inline]
    pub(crate) fn peek(&mut self, n: u32) -> u32 {
        self.refill();
        (self.window & low_mask(n)) as u32
    }

    /// Consumes `n` (≤ 32) bits.
    #[inline]
    pub(crate) fn skip(&mut self, n: u32) -> Result<()> {
        debug_assert!(n <= 32);
        if n > self.count {
            self.refill();
            if n > self.count {
                return Err(VideoError::OutOfBits(self.what));
            }
        }
        self.window >>= n;
        self.count -= n;
        Ok(())
    }

    #[inline]
    pub(crate) fn bit(&mut self) -> Result<bool> {
        if self.count == 0 {
            self.refill();
            if self.count == 0 {
                return Err(VideoError::OutOfBits(self.what));
            }
        }
        let bit = self.window & 1 != 0;
        self.window >>= 1;
        self.count -= 1;
        Ok(bit)
    }

    /// Reads an `n`-bit (≤ 32) field, least significant bit first.
    #[inline]
    pub(crate) fn bits(&mut self, n: u32) -> Result<u32> {
        let value = self.peek(n);
        self.skip(n)?;
        Ok(value)
    }

    #[inline]
    pub(crate) fn byte(&mut self) -> Result<u8> {
        Ok(self.bits(8)? as u8)
    }

    /// Number of bits not yet consumed.
    pub(crate) fn bits_left(&self) -> usize {
        (self.data.len() - self.pos) * 8 + self.count as usize
    }
}

/// LSB-first bit writer used by the unit tests to build synthetic streams.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct BitWriter {
    bytes: Vec<u8>,
    used: u32,
}

#[cfg(test)]
impl BitWriter {
    pub(crate) fn bit(&mut self, bit: bool) {
        if self.used == 0 {
            self.bytes.push(0);
        }
        if bit {
            *self.bytes.last_mut().unwrap() |= 1 << self.used;
        }
        self.used = (self.used + 1) % 8;
    }

    pub(crate) fn bits(&mut self, value: u32, n: u32) {
        for i in 0..n {
            self.bit((value >> i) & 1 != 0);
        }
    }

    pub(crate) fn code(&mut self, code: &[bool]) {
        for &b in code {
            self.bit(b);
        }
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lsb_first() {
        let data = [0b1010_0110u8, 0xFF, 0x00, 0x81];
        let mut r = BitReader::new(&data, "test");
        assert!(!r.bit().unwrap());
        assert!(r.bit().unwrap());
        assert!(r.bit().unwrap());
        assert_eq!(r.bits(5).unwrap(), 0b10100);
        // A field straddling bytes 1 and 2: all of 0xFF, then the low nibble of 0x00.
        assert_eq!(r.bits(12).unwrap(), 0x0FF);
        assert_eq!(r.bits(4).unwrap(), 0);
        assert_eq!(r.bits(8).unwrap(), 0x81);
        assert_eq!(r.bits_left(), 0);
        assert!(matches!(r.bit(), Err(VideoError::OutOfBits("test"))));
    }

    #[test]
    fn multi_byte_fields_are_little_endian() {
        let data = 0x1234_5678_9ABC_DEF0_u64.to_le_bytes();
        let mut r = BitReader::new(&data, "test");
        assert_eq!(r.bits(16).unwrap(), 0xDEF0);
        assert_eq!(r.bits(32).unwrap(), 0x5678_9ABC);
        assert_eq!(r.bits(4).unwrap(), 0x4);
        assert_eq!(r.bits(12).unwrap(), 0x123);
    }

    #[test]
    fn long_streams_refill_correctly() {
        // Every refill path (8-byte fast path and the byte-wise tail) must agree with a
        // straightforward bit-by-bit reference.
        let data: Vec<u8> = (0..101u32).map(|i| (i * 37 + 11) as u8).collect();
        let reference: Vec<bool> =
            data.iter().flat_map(|&b| (0..8).map(move |i| (b >> i) & 1 != 0)).collect();
        let widths = [1u32, 3, 7, 8, 13, 16, 5, 32, 2, 9];
        let mut r = BitReader::new(&data, "test");
        let mut at = 0usize;
        let mut k = 0usize;
        while at < reference.len() {
            let n = widths[k % widths.len()].min((reference.len() - at) as u32);
            k += 1;
            let expected =
                (0..n as usize).fold(0u32, |acc, i| acc | ((reference[at + i] as u32) << i));
            assert_eq!(r.peek(n), expected, "peek at bit {at}");
            assert_eq!(r.bits(n).unwrap(), expected, "read at bit {at}");
            at += n as usize;
            assert_eq!(r.bits_left(), reference.len() - at);
        }
    }

    #[test]
    fn peek_past_end_is_zero_padded_but_not_consumable() {
        let data = [0xFFu8];
        let mut r = BitReader::new(&data, "test");
        assert_eq!(r.peek(12), 0xFF);
        assert!(r.skip(9).is_err());
        assert_eq!(r.bits(8).unwrap(), 0xFF);
        assert!(r.bits(1).is_err());
        let mut empty = BitReader::new(&[], "test");
        assert_eq!(empty.peek(8), 0);
        assert!(empty.skip(0).is_ok());
        assert!(empty.bit().is_err());
    }

    #[test]
    fn writer_round_trips() {
        let mut w = BitWriter::default();
        w.bits(0x5, 3);
        w.bits(0xABCD, 16);
        w.bit(true);
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes, "test");
        assert_eq!(r.bits(3).unwrap(), 0x5);
        assert_eq!(r.bits(16).unwrap(), 0xABCD);
        assert!(r.bit().unwrap());
    }
}
