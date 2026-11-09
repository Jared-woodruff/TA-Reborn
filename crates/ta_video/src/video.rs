//! Video bitstream: 4×4 block coding.
//!
//! The four big Huffman trees are stored once, after the frame table, in a single
//! bitstream: MMAP, MCLR, FULL, then TYPE. Each frame's video data is its own bitstream.
//!
//! A frame is a grid of 4×4 blocks visited left to right, top to bottom (partial blocks
//! at the right and bottom edges are not coded). Decoding repeatedly reads a *type
//! descriptor* from the TYPE tree
//!
//! ```text
//! bits 0–1   block type: 0 MONO, 1 FULL, 2 VOID, 3 SOLID
//! bits 2–7   run-length index: 0–58 mean 1–59 blocks, 59–63 mean 128, 256, 512, 1024, 2048
//! bits 8–15  type data: the colour of SOLID blocks
//! ```
//!
//! and applies the type to the next `run` blocks (clipped at the end of the frame):
//!
//! * **MONO** — per block, a colour pair from MCLR (high byte for set mask bits, low byte
//!   for clear ones) and a 16-bit mask from MMAP, consumed LSB first, row by row;
//! * **FULL** — per block, eight FULL codes: in each row the first code gives pixels 2 and
//!   3, the second pixels 0 and 1, low byte first. In `SMK4` files the run is prefixed by
//!   a mode: `1` = double (per block two codes, each covering two rows as 2×2 cells: low
//!   byte left, high byte right), `01` = half (per block four codes coding rows 0 and 2 as
//!   in normal mode, each row repeated below itself), `00` = normal;
//! * **VOID** — blocks keep the previous frame's pixels;
//! * **SOLID** — blocks are filled with the type data colour.
//!
//! The recent-value caches of all four trees are cleared before each frame.

use crate::Result;
use crate::bits::BitReader;
use crate::huffman::{BigTree, MAX_BIG_TREE_LEAVES};

const BLOCK_MONO: u16 = 0;
const BLOCK_FULL: u16 = 1;
const BLOCK_VOID: u16 = 2;

/// The block-run length for a descriptor's 6-bit run-length index.
#[inline]
fn run_length(index: u16) -> usize {
    let index = usize::from(index & 0x3F);
    if index < 59 { index + 1 } else { 128 << (index - 59) }
}

#[derive(Clone, Debug)]
pub(crate) struct VideoTrees {
    mmap: BigTree,
    mclr: BigTree,
    full: BigTree,
    types: BigTree,
}

impl VideoTrees {
    /// Reads the four trees from the header's tree block.
    pub(crate) fn read(block: &[u8]) -> Result<VideoTrees> {
        let mut bits = BitReader::new(block, "Huffman tree");
        let mmap = BigTree::read(&mut bits, MAX_BIG_TREE_LEAVES)?;
        let mclr = BigTree::read(&mut bits, MAX_BIG_TREE_LEAVES)?;
        let full = BigTree::read(&mut bits, MAX_BIG_TREE_LEAVES)?;
        let types = BigTree::read(&mut bits, MAX_BIG_TREE_LEAVES)?;
        Ok(VideoTrees { mmap, mclr, full, types })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FullMode {
    Normal,
    Double,
    Half,
}

/// The frame buffer seen as a grid of 4×4 blocks.
struct Blocks<'a> {
    pixels: &'a mut [u8],
    stride: usize,
    across: usize,
}

impl Blocks<'_> {
    /// Index of the top-left pixel of block `block`.
    #[inline]
    fn origin(&self, block: usize) -> usize {
        (block / self.across) * 4 * self.stride + (block % self.across) * 4
    }

    #[inline]
    fn put_row(&mut self, origin: usize, row: usize, pixels: [u8; 4]) {
        let at = origin + row * self.stride;
        self.pixels[at..at + 4].copy_from_slice(&pixels);
    }
}

/// Decodes one frame's video bitstream on top of the previous frame in `pixels`
/// (`width × height`, row-major).
pub(crate) fn decode_frame(
    bits: &mut BitReader<'_>,
    trees: &mut VideoTrees,
    pixels: &mut [u8],
    width: usize,
    height: usize,
    smk4: bool,
) -> Result<()> {
    debug_assert_eq!(pixels.len(), width * height);
    for tree in [&mut trees.mmap, &mut trees.mclr, &mut trees.full, &mut trees.types] {
        tree.reset_cache();
    }
    let mut blocks = Blocks { pixels, stride: width, across: width / 4 };
    let total = (width / 4) * (height / 4);
    let mut block = 0usize;
    while block < total {
        let descriptor = trees.types.decode(bits)?;
        let end = total.min(block + run_length(descriptor >> 2));
        match descriptor & 3 {
            BLOCK_MONO => {
                for b in block..end {
                    let [lo, hi] = trees.mclr.decode(bits)?.to_le_bytes();
                    let mut mask = trees.mmap.decode(bits)?;
                    let origin = blocks.origin(b);
                    for row in 0..4 {
                        let mut line = [0u8; 4];
                        for px in &mut line {
                            *px = if mask & 1 != 0 { hi } else { lo };
                            mask >>= 1;
                        }
                        blocks.put_row(origin, row, line);
                    }
                }
            }
            BLOCK_FULL => {
                let mode = if !smk4 {
                    FullMode::Normal
                } else if bits.bit()? {
                    FullMode::Double
                } else if bits.bit()? {
                    FullMode::Half
                } else {
                    FullMode::Normal
                };
                for b in block..end {
                    let origin = blocks.origin(b);
                    match mode {
                        FullMode::Normal => {
                            for row in 0..4 {
                                let [p2, p3] = trees.full.decode(bits)?.to_le_bytes();
                                let [p0, p1] = trees.full.decode(bits)?.to_le_bytes();
                                blocks.put_row(origin, row, [p0, p1, p2, p3]);
                            }
                        }
                        FullMode::Double => {
                            for pair in 0..2 {
                                let [left, right] = trees.full.decode(bits)?.to_le_bytes();
                                let line = [left, left, right, right];
                                blocks.put_row(origin, 2 * pair, line);
                                blocks.put_row(origin, 2 * pair + 1, line);
                            }
                        }
                        FullMode::Half => {
                            for pair in 0..2 {
                                let [p2, p3] = trees.full.decode(bits)?.to_le_bytes();
                                let [p0, p1] = trees.full.decode(bits)?.to_le_bytes();
                                let line = [p0, p1, p2, p3];
                                blocks.put_row(origin, 2 * pair, line);
                                blocks.put_row(origin, 2 * pair + 1, line);
                            }
                        }
                    }
                }
            }
            BLOCK_VOID => {}
            _ => {
                let colour = (descriptor >> 8) as u8;
                for b in block..end {
                    let origin = blocks.origin(b);
                    for row in 0..4 {
                        blocks.put_row(origin, row, [colour; 4]);
                    }
                }
            }
        }
        block = end;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod testutil {
    //! A tiny encoder for synthetic video streams.

    use crate::bits::BitWriter;
    use crate::huffman::testutil::{Shape, write_big_tree};

    /// Escapes that never collide with the test symbols.
    pub(crate) const ESCAPES: [u16; 3] = [0xFFF0, 0xFFF1, 0xFFF2];

    pub(crate) fn descriptor(kind: u16, run_index: u16, data: u8) -> u16 {
        kind | (run_index << 2) | (u16::from(data) << 8)
    }

    /// Four trees, each balanced over the symbols it has to code.
    pub(crate) struct Trees {
        pub mmap: Shape,
        pub mclr: Shape,
        pub full: Shape,
        pub types: Shape,
    }

    impl Trees {
        pub(crate) fn new(mmap: &[u16], mclr: &[u16], full: &[u16], types: &[u16]) -> Trees {
            let shape = |values: &[u16]| {
                let mut v = values.to_vec();
                v.sort_unstable();
                v.dedup();
                Shape::balanced(&v)
            };
            Trees { mmap: shape(mmap), mclr: shape(mclr), full: shape(full), types: shape(types) }
        }

        pub(crate) fn serialise(&self) -> Vec<u8> {
            let mut w = BitWriter::default();
            for shape in [&self.mmap, &self.mclr, &self.full, &self.types] {
                write_big_tree(&mut w, shape, ESCAPES);
            }
            w.finish()
        }
    }

    pub(crate) fn put(w: &mut BitWriter, tree: &Shape, value: u16) {
        w.code(&tree.code(value).expect("symbol missing from test tree"));
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use crate::bits::BitWriter;

    #[test]
    fn run_lengths() {
        assert_eq!(run_length(0), 1);
        assert_eq!(run_length(58), 59);
        assert_eq!(run_length(59), 128);
        assert_eq!(run_length(60), 256);
        assert_eq!(run_length(63), 2048);
    }

    fn decode(trees_bytes: &[u8], frame: &[u8], w: usize, h: usize, smk4: bool) -> Vec<u8> {
        let mut trees = VideoTrees::read(trees_bytes).unwrap();
        let mut pixels = vec![0xEE; w * h];
        decode_frame(&mut BitReader::new(frame, "video"), &mut trees, &mut pixels, w, h, smk4)
            .unwrap();
        pixels
    }

    #[test]
    fn all_block_types() {
        // 12×4 frame: three blocks, plus a 3-pixel strip (not coded) on the right.
        let (w, h) = (15, 5);
        let solid = descriptor(3, 0, 9);
        let mono = descriptor(BLOCK_MONO, 0, 0);
        let full = descriptor(BLOCK_FULL, 0, 0);
        let fulls: Vec<u16> = (0..8).map(|i| 0x1110 * i + 0x0201).collect();
        let t = Trees::new(&[0b1000_0100_0010_0001], &[0x0304], &fulls, &[solid, mono, full]);
        let mut s = BitWriter::default();
        put(&mut s, &t.types, solid);
        put(&mut s, &t.types, mono);
        put(&mut s, &t.mclr, 0x0304);
        put(&mut s, &t.mmap, 0b1000_0100_0010_0001);
        put(&mut s, &t.types, full);
        for &v in &fulls {
            put(&mut s, &t.full, v);
        }
        let px = decode(&t.serialise(), &s.finish(), w, h, false);
        let row = |y: usize| &px[y * w..y * w + w];
        // Block 0 SOLID 9; block 1 MONO (lo 4, hi 3) with the mask selecting the diagonal.
        assert_eq!(row(0)[..8], [9, 9, 9, 9, 3, 4, 4, 4]);
        assert_eq!(row(1)[..8], [9, 9, 9, 9, 4, 3, 4, 4]);
        assert_eq!(row(3)[..8], [9, 9, 9, 9, 4, 4, 4, 3]);
        // Block 2 FULL, row 0: code 0x0201 gives pixels 2,3 = 01,02; then 0x1311 gives
        // pixels 0,1 = 11,13. Later rows follow with the next pairs of codes.
        assert_eq!(row(0)[8..12], [0x11, 0x13, 0x01, 0x02]);
        assert_eq!(row(1)[8..12], [0x31, 0x35, 0x21, 0x24]);
        assert_eq!(row(3)[8..12], [0x71, 0x79, 0x61, 0x68]);
        // The uncoded 3-pixel strip and the uncoded fifth row stay untouched.
        assert!(row(0)[12..].iter().all(|&p| p == 0xEE));
        assert!(row(4).iter().all(|&p| p == 0xEE));
    }

    #[test]
    fn runs_and_void_blocks() {
        // 16×8 = 8 blocks: SOLID ×3 (run index 2), VOID ×2, SOLID ×2048 (clipped to 3).
        let (w, h) = (16, 8);
        let a = descriptor(3, 2, 5);
        let void = descriptor(BLOCK_VOID, 1, 0);
        let b = descriptor(3, 63, 6);
        let t = Trees::new(&[0], &[0], &[0], &[a, void, b]);
        let mut s = BitWriter::default();
        for d in [a, void, b] {
            put(&mut s, &t.types, d);
        }
        let px = decode(&t.serialise(), &s.finish(), w, h, false);
        let block_colour = |b: usize| px[(b / 4) * 4 * w + (b % 4) * 4];
        let colours: Vec<u8> = (0..8).map(block_colour).collect();
        assert_eq!(colours, [5, 5, 5, 0xEE, 0xEE, 6, 6, 6]);
    }

    #[test]
    fn smk4_full_modes() {
        let (w, h) = (12, 4);
        let full = descriptor(BLOCK_FULL, 0, 0);
        let codes: Vec<u16> = (1..=14).map(|i| i * 0x0101 + 0x1000).collect();
        let t = Trees::new(&[0], &[0], &codes, &[full]);
        let mut s = BitWriter::default();
        // Block 0: double mode (bit 1), two codes.
        put(&mut s, &t.types, full);
        s.bit(true);
        put(&mut s, &t.full, codes[0]);
        put(&mut s, &t.full, codes[1]);
        // Block 1: half mode (bits 0, 1), four codes.
        put(&mut s, &t.types, full);
        s.bit(false);
        s.bit(true);
        for &c in &codes[2..6] {
            put(&mut s, &t.full, c);
        }
        // Block 2: normal mode (bits 0, 0), eight codes.
        put(&mut s, &t.types, full);
        s.bit(false);
        s.bit(false);
        for &c in &codes[6..14] {
            put(&mut s, &t.full, c);
        }
        let px = decode(&t.serialise(), &s.finish(), w, h, true);
        let lo = |i: usize| (codes[i] & 0xFF) as u8;
        let hi = |i: usize| (codes[i] >> 8) as u8;
        for y in 0..4 {
            let row = &px[y * w..y * w + w];
            let d = y / 2;
            assert_eq!(row[0..4], [lo(d), lo(d), hi(d), hi(d)], "double, row {y}");
            let (a, b) = (2 + 2 * d, 3 + 2 * d);
            assert_eq!(row[4..8], [lo(b), hi(b), lo(a), hi(a)], "half, row {y}");
            let (a, b) = (6 + 2 * y, 7 + 2 * y);
            assert_eq!(row[8..12], [lo(b), hi(b), lo(a), hi(a)], "normal, row {y}");
        }
    }

    #[test]
    fn truncated_frames_fail_cleanly() {
        let (w, h) = (8, 8);
        let mono = descriptor(BLOCK_MONO, 3, 0);
        let t = Trees::new(&[1, 2, 3], &[4, 5], &[0], &[mono, descriptor(3, 0, 1)]);
        let mut s = BitWriter::default();
        put(&mut s, &t.types, mono);
        for _ in 0..4 {
            put(&mut s, &t.mclr, 5);
            put(&mut s, &t.mmap, 3);
        }
        let stream = s.finish();
        let trees_bytes = t.serialise();
        for len in 0..stream.len() {
            let mut trees = VideoTrees::read(&trees_bytes).unwrap();
            let mut pixels = vec![0; w * h];
            let mut bits = BitReader::new(&stream[..len], "video");
            assert!(decode_frame(&mut bits, &mut trees, &mut pixels, w, h, false).is_err());
        }
    }
}
