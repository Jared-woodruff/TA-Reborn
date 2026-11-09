//! Palette update chunks.
//!
//! A frame whose type byte has bit 0 set begins with a palette chunk. The chunk's first
//! byte gives its total length in units of four bytes (the length byte included). The
//! remaining bytes rebuild the 256 entries in order, starting from entry 0, as a sequence
//! of commands that all refer to the palette as it was *before* this chunk:
//!
//! * `1nnnnnnn` — keep the next `n + 1` entries unchanged;
//! * `01nnnnnn s` — copy `n + 1` consecutive previous entries starting at entry `s`;
//! * `00rrrrrr ..gggggg ..bbbbbb` — a new colour. Components are 6-bit and are widened
//!   to 8 bits by replicating their top bits (`(c << 2) | (c >> 4)`), so 63 maps to 255.
//!
//! The palette starts all black and persists from frame to frame.

use crate::{Result, VideoError};

/// A 256-entry RGB palette.
pub type Palette = [[u8; 3]; 256];

/// Widens a 6-bit VGA colour component to 8 bits.
#[inline]
fn widen(c: u8) -> u8 {
    let c = c & 0x3F;
    (c << 2) | (c >> 4)
}

/// Applies a palette chunk's commands (the bytes after its length byte) to `palette`.
pub(crate) fn apply(commands: &[u8], palette: &mut Palette) -> Result<()> {
    let previous = *palette;
    let eof = |offset: usize| VideoError::Eof { offset: offset + 1, needed: 1 };
    let mut at = 0usize;
    let mut entry = 0usize;
    while entry < 256 {
        let Some(&cmd) = commands.get(at) else {
            // Encoders always describe all 256 entries; tolerate a short chunk by keeping
            // the rest of the palette.
            log::debug!("palette chunk ended after {entry} entries");
            break;
        };
        at += 1;
        if cmd & 0x80 != 0 {
            entry += usize::from(cmd & 0x7F) + 1;
        } else if cmd & 0x40 != 0 {
            let count = usize::from(cmd & 0x3F) + 1;
            let source = usize::from(*commands.get(at).ok_or_else(|| eof(at))?);
            at += 1;
            if source + count > 256 {
                return Err(VideoError::invalid(
                    "palette chunk",
                    format!("copy of {count} entries from {source} runs past the palette"),
                ));
            }
            let count = count.min(256 - entry);
            palette[entry..entry + count].copy_from_slice(&previous[source..source + count]);
            entry += count;
        } else {
            let g = *commands.get(at).ok_or_else(|| eof(at))?;
            let b = *commands.get(at + 1).ok_or_else(|| eof(at + 1))?;
            at += 2;
            palette[entry] = [widen(cmd), widen(g), widen(b)];
            entry += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widening_replicates_top_bits() {
        assert_eq!(widen(0), 0);
        assert_eq!(widen(1), 4);
        assert_eq!(widen(15), 60);
        assert_eq!(widen(16), 65);
        assert_eq!(widen(32), 130);
        assert_eq!(widen(63), 255);
        assert_eq!(widen(0xFF), 255); // high bits of the byte are ignored
    }

    #[test]
    fn keep_copy_and_new_entries() {
        let mut palette: Palette = [[0; 3]; 256];
        for (i, c) in palette.iter_mut().enumerate() {
            *c = [i as u8, 0, 0];
        }
        let chunk = [
            0x00, 0x3F, 0x20, // entry 0: new colour (0, 255, 130)
            0x81, // entries 1-2: keep
            0x42, 10, // entries 3-5: copy of previous 10..=12
            0x40, 3, // entry 6: copy of previous entry 3 (not the value just written)
            0xFF, 0xFF, // entries 7..=262: keep (clamped at 256)
        ];
        apply(&chunk, &mut palette).unwrap();
        assert_eq!(palette[0], [0, 255, 130]);
        assert_eq!(palette[1], [1, 0, 0]);
        assert_eq!(palette[2], [2, 0, 0]);
        assert_eq!(&palette[3..6], &[[10, 0, 0], [11, 0, 0], [12, 0, 0]]);
        assert_eq!(palette[6], [3, 0, 0]);
        assert_eq!(palette[255], [255, 0, 0]);
    }

    #[test]
    fn malformed_chunks_are_rejected_without_panicking() {
        let mut palette: Palette = [[7; 3]; 256];
        assert!(apply(&[0x7F, 250], &mut palette).is_err()); // copy past the end
        assert!(apply(&[0x40], &mut palette).is_err()); // missing copy source
        assert!(apply(&[0x01, 0x02], &mut palette).is_err()); // truncated colour
        // A short chunk keeps the remaining entries.
        let mut palette: Palette = [[7; 3]; 256];
        apply(&[0x01, 0x01, 0x01], &mut palette).unwrap();
        assert_eq!(palette[0], [4, 4, 4]);
        assert_eq!(palette[1], [7, 7, 7]);
    }
}
