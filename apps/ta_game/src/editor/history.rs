//! Undo and redo. Every edit (a brush stroke, a stamp, a feature placed, a start position
//! dragged, the properties changed) is recorded as the smallest rectangles of the heights, tile
//! map and features it changed, before and after, plus the start positions and properties
//! when those changed. The tile set only ever grows while editing (saving drops unused tiles),
//! so tile indices in the records stay valid.

use super::doc::{MapDoc, MapProps, Rect};

/// Most edits kept for undo.
const MAX_EDITS: usize = 256;
/// Most memory the undo history keeps (bytes): a stroke from corner to corner of the largest
/// map is a few megabytes, so the oldest edits go first once the history holds this much.
const MAX_BYTES: usize = 192 << 20;

/// The map's state before an edit, to diff against once the edit is done.
pub struct Snapshot {
    heights: Vec<u8>,
    tile_map: Vec<u16>,
    features: Vec<u16>,
    names: Vec<String>,
    starts: Vec<[f32; 2]>,
    props: MapProps,
    sea_level: i32,
}

impl Snapshot {
    /// Whether an in-progress edit still equals the document it began with.
    #[cfg(test)]
    pub fn matches(&self, doc: &MapDoc) -> bool {
        self.heights == doc.tnt.heights && self.tile_map == doc.tnt.tile_map
            && self.features == doc.tnt.features && self.names == doc.tnt.feature_names
            && self.starts == doc.starts && self.props == doc.props && self.sea_level == doc.tnt.sea_level
    }

    pub fn of(doc: &MapDoc) -> Snapshot {
        Snapshot {
            heights: doc.tnt.heights.clone(),
            tile_map: doc.tnt.tile_map.clone(),
            features: doc.tnt.features.clone(),
            names: doc.tnt.feature_names.clone(),
            starts: doc.starts.clone(),
            props: doc.props.clone(),
            sea_level: doc.tnt.sea_level,
        }
    }
}

/// A rectangle of a grid before and after an edit.
struct Patch<T> {
    rect: Rect,
    before: Vec<T>,
    after: Vec<T>,
}

impl<T: Copy + PartialEq> Patch<T> {
    /// The bounding rectangle of the differences between two `width`-wide grids.
    fn diff(before: &[T], after: &[T], width: u32) -> Option<Patch<T>> {
        let (mut x0, mut z0, mut x1, mut z1) = (u32::MAX, u32::MAX, 0, 0);
        for (i, (a, b)) in before.iter().zip(after).enumerate() {
            if a != b {
                let (x, z) = (i as u32 % width, i as u32 / width);
                x0 = x0.min(x);
                z0 = z0.min(z);
                x1 = x1.max(x + 1);
                z1 = z1.max(z + 1);
            }
        }
        if x0 == u32::MAX {
            return None;
        }
        let rect = Rect::new(x0, z0, x1, z1);
        Some(Patch { rect, before: cut(before, width, rect), after: cut(after, width, rect) })
    }

    fn apply(&self, grid: &mut [T], width: u32, after: bool) {
        let src = if after { &self.after } else { &self.before };
        let w = self.rect.width() as usize;
        for (row, z) in (self.rect.z0..self.rect.z1).enumerate() {
            let start = (z * width + self.rect.x0) as usize;
            grid[start..start + w].copy_from_slice(&src[row * w..(row + 1) * w]);
        }
    }
}

fn cut<T: Copy>(grid: &[T], width: u32, r: Rect) -> Vec<T> {
    let mut out = Vec::with_capacity((r.width() * r.height()) as usize);
    for z in r.z0..r.z1 {
        out.extend_from_slice(&grid[(z * width + r.x0) as usize..(z * width + r.x1) as usize]);
    }
    out
}

/// A value before and after an edit.
type Change<T> = Option<(T, T)>;

/// One undoable edit.
pub struct Edit {
    pub label: String,
    before_version: u64,
    after_version: u64,
    heights: Option<Patch<u8>>,
    tiles: Option<Patch<u16>>,
    features: Option<Patch<u16>>,
    names: Change<Vec<String>>,
    starts: Change<Vec<[f32; 2]>>,
    props: Change<(MapProps, i32)>,
}

/// What an edit, undo or redo changed (for the renderer and the views).
#[derive(Clone, Copy, Debug, Default)]
pub struct Changes {
    /// Cells whose heights changed.
    pub heights: Option<Rect>,
    /// Tiles (32 px) whose graphics changed.
    pub tiles: Option<Rect>,
    pub features: bool,
    pub starts: bool,
    pub props: bool,
    /// The sea level changed (the whole terrain is loaded again).
    pub sea_level: bool,
}

impl Changes {
    pub fn any(&self) -> bool {
        self.heights.is_some() || self.tiles.is_some() || self.features || self.starts || self.props || self.sea_level
    }
}

impl Edit {
    /// About how much memory the edit keeps.
    fn bytes(&self) -> usize {
        let patch = |n: usize, size: usize| 2 * n * size;
        self.heights.as_ref().map_or(0, |p| patch(p.before.len(), 1))
            + self.tiles.as_ref().map_or(0, |p| patch(p.before.len(), 2))
            + self.features.as_ref().map_or(0, |p| patch(p.before.len(), 2))
            + self.names.as_ref().map_or(0, |(a, b)| (a.len() + b.len()) * 24)
            + self.starts.as_ref().map_or(0, |(a, b)| (a.len() + b.len()) * 8)
            + 256
    }

    fn changes(&self) -> Changes {
        Changes {
            heights: self.heights.as_ref().map(|p| p.rect),
            tiles: self.tiles.as_ref().map(|p| p.rect),
            features: self.features.is_some() || self.names.is_some(),
            starts: self.starts.is_some(),
            props: self.props.is_some(),
            sea_level: self.props.as_ref().is_some_and(|(a, b)| a.1 != b.1),
        }
    }

    fn apply(&self, doc: &mut MapDoc, after: bool) {
        let t = &mut doc.tnt;
        if let Some(p) = &self.heights {
            p.apply(&mut t.heights, t.width, after);
        }
        if let Some(p) = &self.tiles {
            p.apply(&mut t.tile_map, t.width / 2, after);
        }
        if let Some((before, now)) = &self.names {
            t.feature_names = if after { now.clone() } else { before.clone() };
        }
        if let Some(p) = &self.features {
            p.apply(&mut t.features, t.width, after);
        }
        if let Some((before, now)) = &self.starts {
            doc.starts = if after { now.clone() } else { before.clone() };
        }
        if let Some((before, now)) = &self.props {
            let (props, sea) = if after { now } else { before };
            doc.props = props.clone();
            doc.tnt.sea_level = *sea;
        }
    }
}

#[derive(Default)]
pub struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    /// Identity of the current state. Undo/redo restore that identity, so returning
    /// to the saved state clears the dirty marker. New branches get fresh identities.
    pub version: u64,
    next_version: u64,
}

impl History {
    /// Records what changed since `before` as an edit. Returns the changes, or `None` when
    /// nothing did.
    pub fn record(&mut self, before: Snapshot, doc: &MapDoc, label: &str) -> Option<Changes> {
        let t = &doc.tnt;
        let edit = Edit {
            label: label.to_string(),
            before_version: self.version,
            after_version: self.next_version + 1,
            heights: Patch::diff(&before.heights, &t.heights, t.width),
            tiles: Patch::diff(&before.tile_map, &t.tile_map, t.width / 2),
            features: Patch::diff(&before.features, &t.features, t.width),
            names: (before.names != t.feature_names).then(|| (before.names, t.feature_names.clone())),
            starts: (before.starts != doc.starts).then(|| (before.starts, doc.starts.clone())),
            props: (before.props != doc.props || before.sea_level != t.sea_level).then(|| ((before.props, before.sea_level), (doc.props.clone(), t.sea_level))),
        };
        let changes = edit.changes();
        if !changes.any() {
            return None;
        }
        self.version = edit.after_version;
        self.next_version = self.version;
        self.undo.push(edit);
        self.redo.clear();
        // (The oldest edits go once there are too many, or they keep too much memory.)
        let mut bytes: usize = self.undo.iter().map(Edit::bytes).sum();
        while self.undo.len() > 1 && (self.undo.len() > MAX_EDITS || bytes > MAX_BYTES) {
            bytes -= self.undo.remove(0).bytes();
        }
        Some(changes)
    }

    pub fn undo(&mut self, doc: &mut MapDoc) -> Option<Changes> {
        let edit = self.undo.pop()?;
        edit.apply(doc, false);
        self.version = edit.before_version;
        let changes = edit.changes();
        self.redo.push(edit);
        Some(changes)
    }

    pub fn redo(&mut self, doc: &mut MapDoc) -> Option<Changes> {
        let edit = self.redo.pop()?;
        edit.apply(doc, true);
        self.version = edit.after_version;
        let changes = edit.changes();
        self.undo.push(edit);
        Some(changes)
    }

    /// What undo and redo would do next.
    pub fn next_undo(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    pub fn next_redo(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }
}
