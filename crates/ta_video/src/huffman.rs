//! Smacker's Huffman trees.
//!
//! A tree is transmitted as a pre-order walk of its shape: a `1` bit introduces an interior
//! node, followed by its 0-subtree and then its 1-subtree; a `0` bit introduces a leaf,
//! immediately followed by the leaf's payload. A code is read one bit at a time from the
//! LSB-first stream, starting at the root, with `0` selecting the first-serialised child.
//! A tree consisting of a single leaf therefore has a zero-length code.
//!
//! Two kinds of tree exist:
//!
//! * **Byte trees** have 8-bit leaves. They are the building blocks of the big trees and
//!   are used directly by the audio codec. On the wire: a `present` bit and, if it is set,
//!   the tree followed by a terminating `0` bit. An absent tree decodes every symbol as 0
//!   without consuming any bits.
//! * **Big trees** have 16-bit leaves and drive the video codec (the MMAP, MCLR, FULL and
//!   TYPE trees). On the wire: a `present` bit (absent trees always yield 0); a byte tree
//!   for the low bytes and another for the high bytes; three 16-bit *escape* values; the
//!   tree shape, where each leaf's payload is its low byte coded with the first byte tree
//!   followed by its high byte coded with the second; and a terminating `0` bit.
//!
//!   A leaf whose value equals escape `i` is not a constant symbol but slot `i` of a small
//!   most-recently-used cache: slot 0 holds the last value decoded from the tree, slot 1
//!   the one before a different value arrived, slot 2 the one before that. After every
//!   decode, if the value differs from slot 0, the slots shift down (`2 ← 1`, `1 ← 0`) and
//!   slot 0 takes the value. All three slots are cleared to 0 at the start of every video
//!   frame. If no leaf carries a given escape the slot still exists but is unreachable.
//!
//! Decoding resolves up to [`MAX_TABLE_BITS`] bits with one table lookup and walks the
//! node array for any remaining bits of longer codes.

use crate::bits::BitReader;
use crate::{Result, VideoError};

/// Longest code prefix resolved by a single table probe.
const MAX_TABLE_BITS: u32 = 12;
/// Byte trees can distinguish at most 256 symbols.
pub(crate) const MAX_BYTE_TREE_LEAVES: usize = 256;
/// Upper bound on big-tree leaves: every 16-bit value plus the cache slots, with headroom.
pub(crate) const MAX_BIG_TREE_LEAVES: usize = 1 << 17;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node {
    /// Interior node. Its 0-child is stored right after it; `one` indexes its 1-child.
    Branch { one: u32 },
    /// Leaf carrying a symbol (a byte value, or a big tree's leaf ordinal).
    Leaf { symbol: u32 },
}

/// Result of a table probe: the node reached after consuming `len` bits.
#[derive(Clone, Copy, Debug, Default)]
struct Probe {
    node: u32,
    len: u8,
}

/// The shape of a Huffman tree plus its decoding table.
#[derive(Clone, Debug)]
struct CodeTree {
    nodes: Vec<Node>,
    table: Vec<Probe>,
    table_bits: u32,
}

impl CodeTree {
    /// A tree with a single leaf: every decode yields `symbol` and consumes nothing.
    fn single(symbol: u32) -> CodeTree {
        let nodes = vec![Node::Leaf { symbol }];
        CodeTree { nodes, table: vec![Probe::default()], table_bits: 0 }
    }

    /// Parses a serialised shape. `read_leaf` reads a leaf's payload and returns its
    /// symbol; it receives the leaf's ordinal (leaves are numbered in stream order).
    fn parse(
        bits: &mut BitReader<'_>,
        max_leaves: usize,
        mut read_leaf: impl FnMut(&mut BitReader<'_>, u32) -> Result<u32>,
    ) -> Result<CodeTree> {
        let mut nodes = Vec::new();
        // Branches whose 0-subtree is being read and whose 1-child is not yet placed.
        let mut pending: Vec<usize> = Vec::new();
        let mut branches = 0usize;
        let mut leaves = 0usize;
        loop {
            if bits.bit()? {
                // A full binary tree with `b` branches has `b + 1` leaves.
                branches += 1;
                if branches >= max_leaves {
                    return Err(too_large(max_leaves));
                }
                pending.push(nodes.len());
                nodes.push(Node::Branch { one: 0 });
            } else {
                if leaves == max_leaves {
                    return Err(too_large(max_leaves));
                }
                let symbol = read_leaf(bits, leaves as u32)?;
                leaves += 1;
                nodes.push(Node::Leaf { symbol });
                // The leaf completes the 0-subtree of the innermost pending branch, whose
                // 1-subtree starts next; with no pending branch the whole tree is done.
                match pending.pop() {
                    Some(branch) => nodes[branch] = Node::Branch { one: nodes.len() as u32 },
                    None => break,
                }
            }
        }
        let (table, table_bits) = build_table(&nodes);
        Ok(CodeTree { nodes, table, table_bits })
    }

    #[inline]
    fn decode(&self, bits: &mut BitReader<'_>) -> Result<u32> {
        let probe = self.table[bits.peek(self.table_bits) as usize];
        bits.skip(u32::from(probe.len))?;
        let mut node = probe.node as usize;
        loop {
            match self.nodes[node] {
                Node::Leaf { symbol } => return Ok(symbol),
                Node::Branch { one } => {
                    node = if bits.bit()? { one as usize } else { node + 1 };
                }
            }
        }
    }
}

fn too_large(max_leaves: usize) -> VideoError {
    VideoError::invalid("Huffman tree", format!("more than {max_leaves} leaves"))
}

/// Builds the direct-lookup table over the first `min(depth, MAX_TABLE_BITS)` code bits.
/// Codes are LSB-first, so a leaf at depth `d` with code `c` owns every slot whose low `d`
/// bits equal `c`; a branch at the table depth owns exactly one slot.
fn build_table(nodes: &[Node]) -> (Vec<Probe>, u32) {
    let mut max_depth = 0u32;
    let mut stack = vec![(0usize, 0u32)];
    while let Some((node, depth)) = stack.pop() {
        match nodes[node] {
            Node::Leaf { .. } => max_depth = max_depth.max(depth),
            Node::Branch { one } => {
                stack.push((node + 1, depth + 1));
                stack.push((one as usize, depth + 1));
            }
        }
    }

    let table_bits = max_depth.min(MAX_TABLE_BITS);
    let size = 1usize << table_bits;
    let mut table = vec![Probe::default(); size];
    let mut stack = vec![(0usize, 0usize, 0u32)];
    while let Some((node, code, depth)) = stack.pop() {
        let probe = Probe { node: node as u32, len: depth as u8 };
        match nodes[node] {
            Node::Leaf { .. } => {
                for slot in table.iter_mut().skip(code).step_by(1 << depth) {
                    *slot = probe;
                }
            }
            Node::Branch { .. } if depth == table_bits => table[code] = probe,
            Node::Branch { one } => {
                stack.push((node + 1, code, depth + 1));
                stack.push((one as usize, code | (1 << depth), depth + 1));
            }
        }
    }
    (table, table_bits)
}

/// A Huffman tree over byte values.
#[derive(Clone, Debug)]
pub(crate) struct ByteTree {
    code: CodeTree,
}

impl ByteTree {
    /// Reads a byte tree with its `present` flag and terminating bit.
    pub(crate) fn read(bits: &mut BitReader<'_>) -> Result<ByteTree> {
        if !bits.bit()? {
            return Ok(ByteTree { code: CodeTree::single(0) });
        }
        let code =
            CodeTree::parse(bits, MAX_BYTE_TREE_LEAVES, |bits, _| Ok(u32::from(bits.byte()?)))?;
        bits.bit()?; // terminator
        Ok(ByteTree { code })
    }

    #[inline]
    pub(crate) fn decode(&self, bits: &mut BitReader<'_>) -> Result<u8> {
        // Symbols of byte trees are the byte values themselves.
        Ok(self.code.decode(bits)? as u8)
    }
}

/// A Huffman tree over 16-bit values with the three-slot recent-value cache.
#[derive(Clone, Debug)]
pub(crate) struct BigTree {
    code: CodeTree,
    /// Current value of every leaf, indexed by leaf ordinal. The cache slots live here too.
    values: Vec<u16>,
    /// Leaf ordinals of cache slots 0, 1 and 2.
    cache: [usize; 3],
}

impl BigTree {
    /// A tree that always yields `value` (used for absent trees).
    fn constant(value: u16) -> BigTree {
        // Leaf 0 is the only reachable leaf; 1–3 are unreachable cache slots.
        BigTree { code: CodeTree::single(0), values: vec![value, 0, 0, 0], cache: [1, 2, 3] }
    }

    /// Reads a big tree with its `present` flag and terminating bit.
    pub(crate) fn read(bits: &mut BitReader<'_>, max_leaves: usize) -> Result<BigTree> {
        if !bits.bit()? {
            return Ok(BigTree::constant(0));
        }
        let low = ByteTree::read(bits)?;
        let high = ByteTree::read(bits)?;
        let escapes = [bits.bits(16)? as u16, bits.bits(16)? as u16, bits.bits(16)? as u16];

        let mut values = Vec::new();
        let mut cache = [None; 3];
        let code = CodeTree::parse(bits, max_leaves, |bits, ordinal| {
            let lo = low.decode(bits)?;
            let hi = high.decode(bits)?;
            let mut value = u16::from_le_bytes([lo, hi]);
            // The first matching escape wins if the file repeats one.
            if let Some(slot) = escapes.iter().position(|&e| e == value) {
                cache[slot] = Some(ordinal as usize);
                value = 0;
            }
            values.push(value);
            Ok(ordinal)
        })?;
        bits.bit()?; // terminator

        let cache = cache.map(|slot| {
            slot.unwrap_or_else(|| {
                values.push(0);
                values.len() - 1
            })
        });
        Ok(BigTree { code, values, cache })
    }

    /// Clears the recent-value cache (done at the start of every frame).
    pub(crate) fn reset_cache(&mut self) {
        for &slot in &self.cache {
            self.values[slot] = 0;
        }
    }

    #[inline]
    pub(crate) fn decode(&mut self, bits: &mut BitReader<'_>) -> Result<u16> {
        let leaf = self.code.decode(bits)? as usize;
        let value = self.values[leaf];
        let [c0, c1, c2] = self.cache;
        if value != self.values[c0] {
            self.values[c2] = self.values[c1];
            self.values[c1] = self.values[c0];
            self.values[c0] = value;
        }
        Ok(value)
    }
}

/// Test helpers that serialise trees, shared with the codec tests.
#[cfg(test)]
pub(crate) mod testutil {
    use crate::bits::BitWriter;

    /// A tree shape with `u16` leaf values (byte trees use the low byte).
    #[derive(Clone, Debug)]
    pub(crate) enum Shape {
        Leaf(u16),
        Node(Box<Shape>, Box<Shape>),
    }

    impl Shape {
        pub(crate) fn node(zero: Shape, one: Shape) -> Shape {
            Shape::Node(Box::new(zero), Box::new(one))
        }

        /// A balanced tree over `values` (which must be non-empty).
        pub(crate) fn balanced(values: &[u16]) -> Shape {
            match values {
                [single] => Shape::Leaf(*single),
                _ => {
                    let (a, b) = values.split_at(values.len() / 2);
                    Shape::node(Shape::balanced(a), Shape::balanced(b))
                }
            }
        }

        /// The code of the first leaf carrying `value`, as bits in stream order.
        pub(crate) fn code(&self, value: u16) -> Option<Vec<bool>> {
            match self {
                Shape::Leaf(v) => (*v == value).then(Vec::new),
                Shape::Node(zero, one) => {
                    for (bit, child) in [(false, zero), (true, one)] {
                        if let Some(mut rest) = child.code(value) {
                            rest.insert(0, bit);
                            return Some(rest);
                        }
                    }
                    None
                }
            }
        }

        fn leaves(&self, out: &mut Vec<u16>) {
            match self {
                Shape::Leaf(v) => out.push(*v),
                Shape::Node(zero, one) => {
                    zero.leaves(out);
                    one.leaves(out);
                }
            }
        }
    }

    fn write_shape(w: &mut BitWriter, shape: &Shape, leaf: &mut dyn FnMut(&mut BitWriter, u16)) {
        match shape {
            Shape::Leaf(v) => {
                w.bit(false);
                leaf(w, *v);
            }
            Shape::Node(zero, one) => {
                w.bit(true);
                write_shape(w, zero, leaf);
                write_shape(w, one, leaf);
            }
        }
    }

    /// Writes a present byte tree (flag, shape, terminator).
    pub(crate) fn write_byte_tree(w: &mut BitWriter, shape: &Shape) {
        w.bit(true);
        write_shape(w, shape, &mut |w, v| w.bits(u32::from(v), 8));
        w.bit(false);
    }

    /// Writes a present big tree whose byte trees are balanced over the bytes it uses.
    pub(crate) fn write_big_tree(w: &mut BitWriter, shape: &Shape, escapes: [u16; 3]) {
        let mut values = Vec::new();
        shape.leaves(&mut values);
        let mut lows: Vec<u16> = values.iter().map(|v| v & 0xFF).collect();
        let mut highs: Vec<u16> = values.iter().map(|v| v >> 8).collect();
        for list in [&mut lows, &mut highs] {
            list.sort_unstable();
            list.dedup();
        }
        let (low, high) = (Shape::balanced(&lows), Shape::balanced(&highs));
        w.bit(true);
        write_byte_tree(w, &low);
        write_byte_tree(w, &high);
        for e in escapes {
            w.bits(u32::from(e), 16);
        }
        write_shape(w, shape, &mut |w, v| {
            w.code(&low.code(v & 0xFF).unwrap());
            w.code(&high.code(v >> 8).unwrap());
        });
        w.bit(false);
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::{Shape, write_big_tree, write_byte_tree};
    use super::*;
    use crate::bits::BitWriter;

    fn leaf(v: u16) -> Shape {
        Shape::Leaf(v)
    }

    #[test]
    fn byte_tree_round_trip() {
        // Deliberately unbalanced: depths 1, 2, 3, 3.
        let [a, b, c, d] = b"abcd".map(|v| leaf(v.into()));
        let shape = Shape::node(a, Shape::node(b, Shape::node(c, d)));
        let message = b"abacabadd";
        let mut w = BitWriter::default();
        write_byte_tree(&mut w, &shape);
        for &c in message {
            w.code(&shape.code(c.into()).unwrap());
        }
        let data = w.finish();
        let mut r = BitReader::new(&data, "test");
        let tree = ByteTree::read(&mut r).unwrap();
        let decoded: Vec<u8> = message.iter().map(|_| tree.decode(&mut r).unwrap()).collect();
        assert_eq!(decoded, message);
        assert!(r.bits_left() < 8);
    }

    #[test]
    fn serialisation_is_preorder_with_lsb_first_codes() {
        // Hand-assembled: 1 (node) 0 'x' 1 (node) 0 'y' 0 'z' then terminator.
        let mut w = BitWriter::default();
        w.bit(true); // present
        w.bit(true);
        w.bit(false);
        w.bits(0x78, 8);
        w.bit(true);
        w.bit(false);
        w.bits(0x79, 8);
        w.bit(false);
        w.bits(0x7A, 8);
        w.bit(false); // terminator
        // Codes: x = 0, y = 1 then 0, z = 1 then 1.
        w.code(&[true, true, false, true, false]);
        let data = w.finish();
        let mut r = BitReader::new(&data, "test");
        let tree = ByteTree::read(&mut r).unwrap();
        assert_eq!(tree.decode(&mut r).unwrap(), 0x7A);
        assert_eq!(tree.decode(&mut r).unwrap(), 0x78);
        assert_eq!(tree.decode(&mut r).unwrap(), 0x79);
    }

    #[test]
    fn absent_and_single_leaf_trees_consume_no_bits() {
        let mut w = BitWriter::default();
        w.bit(false); // absent byte tree
        w.bit(true); // present single-leaf tree: leaf 0x42
        w.bit(false);
        w.bits(0x42, 8);
        w.bit(false);
        let data = w.finish();
        let mut r = BitReader::new(&data, "test");
        let absent = ByteTree::read(&mut r).unwrap();
        let single = ByteTree::read(&mut r).unwrap();
        let left = r.bits_left();
        for _ in 0..10 {
            assert_eq!(absent.decode(&mut r).unwrap(), 0);
            assert_eq!(single.decode(&mut r).unwrap(), 0x42);
        }
        assert_eq!(r.bits_left(), left);
    }

    #[test]
    fn deep_codes_fall_back_to_tree_walk() {
        // A 20-deep "comb" exceeds the 12-bit table and exercises the node walk.
        let mut shape = leaf(20);
        for v in (0..20).rev() {
            shape = Shape::node(leaf(v), shape);
        }
        let message = [0u16, 19, 20, 5, 12, 13, 20, 11];
        let mut w = BitWriter::default();
        write_byte_tree(&mut w, &shape);
        for &v in &message {
            w.code(&shape.code(v).unwrap());
        }
        let data = w.finish();
        let mut r = BitReader::new(&data, "test");
        let tree = ByteTree::read(&mut r).unwrap();
        for &v in &message {
            assert_eq!(u16::from(tree.decode(&mut r).unwrap()), v);
        }
        assert!(r.bits_left() < 8);
    }

    #[test]
    fn big_tree_values_and_recent_value_cache() {
        const E0: u16 = 0xFFF0;
        const E1: u16 = 0xFFF1;
        const E2: u16 = 0xFFF2;
        let shape = Shape::node(
            Shape::node(leaf(0x1234), leaf(0xBEEF)),
            Shape::node(leaf(E0), Shape::node(leaf(E1), leaf(E2))),
        );
        let code = |v| shape.code(v).unwrap();
        let mut w = BitWriter::default();
        write_big_tree(&mut w, &shape, [E0, E1, E2]);
        // (symbol coded, value expected); the cache after each step is shown as
        // [slot 0, slot 1, slot 2].
        let steps: [(u16, u16); 9] = [
            (E0, 0),          // cache starts cleared            [0, 0, 0]
            (0x1234, 0x1234), //                                  [1234, 0, 0]
            (E0, 0x1234),     // equals slot 0: no shift          [1234, 0, 0]
            (0xBEEF, 0xBEEF), //                                  [BEEF, 1234, 0]
            (E1, 0x1234),     // differs from slot 0: shift       [1234, BEEF, 1234]
            (E1, 0xBEEF),     //                                  [BEEF, 1234, BEEF]
            (0x1234, 0x1234), //                                  [1234, BEEF, 1234]
            (E2, 0x1234),     // equals slot 0: no shift          [1234, BEEF, 1234]
            (E1, 0xBEEF),     //                                  [BEEF, 1234, BEEF]
        ];
        for (symbol, _) in steps {
            w.code(&code(symbol));
        }
        // After a reset every cache slot reads 0 again.
        w.code(&code(E0));
        w.code(&code(E2));
        let data = w.finish();

        let mut r = BitReader::new(&data, "test");
        let mut tree = BigTree::read(&mut r, MAX_BIG_TREE_LEAVES).unwrap();
        for (i, (_, expected)) in steps.iter().enumerate() {
            assert_eq!(tree.decode(&mut r).unwrap(), *expected, "step {i}");
        }
        tree.reset_cache();
        assert_eq!(tree.decode(&mut r).unwrap(), 0);
        assert_eq!(tree.decode(&mut r).unwrap(), 0);
    }

    #[test]
    fn absent_big_tree_yields_zero() {
        let data = [0u8];
        let mut r = BitReader::new(&data, "test");
        let mut tree = BigTree::read(&mut r, MAX_BIG_TREE_LEAVES).unwrap();
        for _ in 0..4 {
            assert_eq!(tree.decode(&mut r).unwrap(), 0);
        }
    }

    #[test]
    fn missing_escape_leaves_get_unreachable_slots() {
        let shape = Shape::node(leaf(7), leaf(9));
        let mut w = BitWriter::default();
        write_big_tree(&mut w, &shape, [1, 2, 3]);
        for v in [7, 9, 9, 7] {
            w.code(&shape.code(v).unwrap());
        }
        let data = w.finish();
        let mut r = BitReader::new(&data, "test");
        let mut tree = BigTree::read(&mut r, MAX_BIG_TREE_LEAVES).unwrap();
        for v in [7, 9, 9, 7] {
            assert_eq!(tree.decode(&mut r).unwrap(), v);
        }
    }

    #[test]
    fn oversized_and_truncated_trees_are_rejected() {
        // 300 leaves cannot be a byte tree.
        let values: Vec<u16> = (0..300).map(|v| v % 256).collect();
        let mut w = BitWriter::default();
        write_byte_tree(&mut w, &Shape::balanced(&values));
        let data = w.finish();
        assert!(ByteTree::read(&mut BitReader::new(&data, "test")).is_err());

        // A stream of `1` bits never closes the tree.
        let data = [0xFFu8; 64];
        assert!(ByteTree::read(&mut BitReader::new(&data, "test")).is_err());
        assert!(BigTree::read(&mut BitReader::new(&data, "test"), MAX_BIG_TREE_LEAVES).is_err());

        // Truncation anywhere inside a valid tree is an error, not a panic.
        let shape = Shape::balanced(&[1, 2, 3, 0x100, 0x201]);
        let mut w = BitWriter::default();
        write_big_tree(&mut w, &shape, [0, 0, 0]);
        let data = w.finish();
        for len in 0..data.len() {
            assert!(BigTree::read(&mut BitReader::new(&data[..len], "test"), 64).is_err());
        }
        assert!(BigTree::read(&mut BitReader::new(&data, "test"), 64).is_ok());
    }
}
