//! The editing operations, on the map document alone (no GPU, no UI): height brushes, tile
//! stamps, copy and paste of a region, and scattering features.

use ta_data::GameData;
use ta_formats::tnt::{FEATURE_NONE, FEATURE_VOID, TntMap};
use anyhow::{Result, bail};

use super::doc::{CELL_PX, MapDoc, Rect, Tile};

/// What a height brush does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HeightOp {
    Raise,
    Lower,
    /// Towards the mean of the neighbours.
    Smooth,
    /// Towards the height under the cursor where the stroke began.
    Flatten(f32),
    /// Exactly this height inside the brush.
    Level(f32),
}

/// Height units per second a full-strength raise or lower adds at the brush centre.
pub const RAISE_RATE: f32 = 90.0;

/// A brush stroke's working heights: full precision, so slow strokes accumulate.
pub struct HeightStroke {
    work: Vec<f32>,
}

impl HeightStroke {
    pub fn new(doc: &MapDoc) -> HeightStroke {
        HeightStroke { work: doc.tnt.heights.iter().map(|&h| h as f32).collect() }
    }

    /// Applies `op` around world point (x, z) for `dt` seconds: every height vertex within
    /// `radius` pixels, weighted by a smooth falloff and `strength` (0..1). Returns the cells
    /// that changed.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(&mut self, doc: &mut MapDoc, x: f32, z: f32, radius: f32, strength: f32, op: HeightOp, dt: f32) -> Option<Rect> {
        let t = &mut doc.tnt;
        let (w, h) = (t.width as i32, t.height as i32);
        let r = radius.max(CELL_PX * 0.5);
        let (cx, cz) = (x / CELL_PX, z / CELL_PX);
        let rc = r / CELL_PX;
        let (x0, x1) = (((cx - rc).floor() as i32).max(0), ((cx + rc).ceil() as i32).min(w - 1));
        let (z0, z1) = (((cz - rc).floor() as i32).max(0), ((cz + rc).ceil() as i32).min(h - 1));
        if x0 > x1 || z0 > z1 {
            return None;
        }
        // New values first (smoothing reads the neighbours as they were), then written.
        let work = &self.work;
        let mut updates = Vec::new();
        for vz in z0..=z1 {
            for vx in x0..=x1 {
                let d = ((vx as f32 - cx).powi(2) + (vz as f32 - cz).powi(2)).sqrt() * CELL_PX;
                if d > r {
                    continue;
                }
                let falloff = (1.0 - (d / r).powi(2)).powi(2);
                let i = (vz * w + vx) as usize;
                let cur = work[i];
                let next = match op {
                    HeightOp::Raise => cur + RAISE_RATE * strength * falloff * dt,
                    HeightOp::Lower => cur - RAISE_RATE * strength * falloff * dt,
                    HeightOp::Smooth => {
                        let mut sum = 0.0;
                        for (dx, dz) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                            let (sx, sz) = ((vx + dx).clamp(0, w - 1), (vz + dz).clamp(0, h - 1));
                            sum += work[(sz * w + sx) as usize];
                        }
                        cur + (sum / 9.0 - cur) * (strength * falloff * 12.0 * dt).min(1.0)
                    }
                    HeightOp::Flatten(target) => cur + (target - cur) * (strength * falloff * 12.0 * dt).min(1.0),
                    // (The level inside, eased into the ground round it at the brush's edge, as
                    // fast as the strength says: no cliff where the brush ends.)
                    HeightOp::Level(level) => {
                        let edge = ((1.0 - d / r) / 0.3).clamp(0.0, 1.0);
                        let ease = edge * edge * (3.0 - 2.0 * edge);
                        cur + (level - cur) * (ease * (strength * 40.0 * dt).min(1.0)).min(1.0)
                    }
                };
                updates.push((vx as u32, vz as u32, i, next.clamp(0.0, 255.0)));
            }
        }
        let mut changed: Option<Rect> = None;
        for (vx, vz, i, next) in updates {
            self.work[i] = next;
            let rounded = next.round() as u8;
            if t.heights[i] != rounded {
                t.heights[i] = rounded;
                changed = Rect::merge(changed, Some(Rect::new(vx, vz, vx + 1, vz + 1)));
            }
        }
        changed
    }
}

/// A block of tiles to stamp: indices into the edited map's tile set, row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct TileBlock {
    pub w: u32,
    pub h: u32,
    pub tiles: Vec<u16>,
}

impl TileBlock {
    /// Tiles `rect` (in 32 px tiles) of `source`, added to the edited map's tile set.
    pub fn import(doc: &mut MapDoc, source: &TntMap, rect: Rect) -> Result<TileBlock> {
        let tw = source.width / 2;
        let rect = if source.tiles.is_empty() { Rect::new(0, 0, 0, 0) } else { rect.clip(tw, source.height / 2) };
        let mut tiles = Vec::with_capacity((rect.width() * rect.height()) as usize);
        let before = doc.tnt.tiles.len();
        for z in rect.z0..rect.z1 {
            for x in rect.x0..rect.x1 {
                let index = source.tile_map[(z * tw + x) as usize] as usize;
                let tile: &Tile = &source.tiles[index.min(source.tiles.len() - 1)];
                let Some(index) = doc.intern_tile(tile) else {
                    doc.truncate_tiles(before);
                    bail!(ta_ui::i18n::t("This map already uses the maximum number of terrain tiles. Choose a smaller tile block."));
                };
                tiles.push(index);
            }
        }
        Ok(TileBlock { w: rect.width(), h: rect.height(), tiles })
    }

    /// Tiles `rect` of a tile set laid out `columns` a row (the picker's tile-set view): the
    /// set of `source`, added to the edited map's, or the edited map's own. Places past the
    /// end of the set take its last tile.
    pub fn from_set(doc: &mut MapDoc, source: Option<&TntMap>, rect: Rect, columns: u32) -> Result<TileBlock> {
        let count = source.map_or(doc.tnt.tiles.len(), |s| s.tiles.len()) as u32;
        if count == 0 || columns == 0 {
            return Ok(TileBlock { w: 0, h: 0, tiles: Vec::new() });
        }
        let rect = rect.clip(columns, count.div_ceil(columns));
        let mut tiles = Vec::with_capacity((rect.width() * rect.height()) as usize);
        let before = doc.tnt.tiles.len();
        for z in rect.z0..rect.z1 {
            for x in rect.x0..rect.x1 {
                let index = (z * columns + x).min(count - 1);
                tiles.push(match source {
                    Some(s) => match doc.intern_tile(&s.tiles[index as usize]) {
                        Some(index) => index,
                        None => {
                            doc.truncate_tiles(before);
                            bail!(ta_ui::i18n::t("This map already uses the maximum number of terrain tiles. Choose a smaller tile block."));
                        }
                    },
                    None => index as u16,
                });
            }
        }
        Ok(TileBlock { w: rect.width(), h: rect.height(), tiles })
    }

    /// Tiles `rect` of the edited map itself.
    pub fn from_map(doc: &MapDoc, rect: Rect) -> TileBlock {
        let tw = doc.tiles_wide();
        let rect = rect.clip(tw, doc.tiles_high());
        let mut tiles = Vec::with_capacity((rect.width() * rect.height()) as usize);
        for z in rect.z0..rect.z1 {
            for x in rect.x0..rect.x1 {
                tiles.push(doc.tnt.tile_map[(z * tw + x) as usize]);
            }
        }
        TileBlock { w: rect.width(), h: rect.height(), tiles }
    }

    /// Stamps the block with its top-left tile at `(tx, tz)`, clipped to the map. Returns the
    /// tiles that changed.
    pub fn stamp(&self, doc: &mut MapDoc, tx: i32, tz: i32) -> Option<Rect> {
        let (tw, th) = (doc.tiles_wide() as i32, doc.tiles_high() as i32);
        let mut changed = None;
        for bz in 0..self.h as i32 {
            for bx in 0..self.w as i32 {
                let (x, z) = (tx + bx, tz + bz);
                if x < 0 || z < 0 || x >= tw || z >= th {
                    continue;
                }
                let tile = self.tiles[(bz * self.w as i32 + bx) as usize];
                let slot = &mut doc.tnt.tile_map[(z * tw + x) as usize];
                if *slot != tile {
                    *slot = tile;
                    changed = Rect::merge(changed, Some(Rect::new(x as u32, z as u32, x as u32 + 1, z as u32 + 1)));
                }
            }
        }
        changed
    }
}

/// A copied region: its tiles, the heights of the cells under them, and the features and
/// impassable cells there.
#[derive(Clone, Debug)]
pub struct Clip {
    pub tiles: TileBlock,
    /// (2·w) × (2·h) heights, row by row.
    pub heights: Vec<u8>,
    /// The features that start in the region: (cell from its top-left, name).
    pub features: Vec<((u32, u32), String)>,
    /// The region's void (impassable) cells, from its top-left.
    pub void: Vec<(u32, u32)>,
}

/// What a paste brings with its tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PasteWith {
    pub heights: bool,
    /// The copy's rises and dips added to the ground where it lands (its edge's height as the
    /// level), so its edges meet the ground round it; else its heights as they were.
    pub relative: bool,
    /// Its features and impassable cells, in place of those there.
    pub features: bool,
}

/// What a paste changed: tiles, cells' heights, features.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pasted {
    pub tiles: Option<Rect>,
    pub cells: Option<Rect>,
    pub features: bool,
}

impl Clip {
    /// Copies tiles `rect` and the cells under them.
    pub fn copy(doc: &MapDoc, rect: Rect) -> Clip {
        let tiles = TileBlock::from_map(doc, rect);
        let w = doc.tnt.width;
        let (x0, z0, x1, z1) = (rect.x0 * 2, rect.z0 * 2, (rect.x0 + tiles.w) * 2, (rect.z0 + tiles.h) * 2);
        let mut heights = Vec::with_capacity((tiles.w * tiles.h * 4) as usize);
        let (mut features, mut void) = (Vec::new(), Vec::new());
        for z in z0..z1 {
            for x in x0..x1 {
                heights.push(doc.tnt.heights[(z * w + x) as usize]);
                if doc.void_at(x, z) {
                    void.push((x - x0, z - z0));
                } else if let Some(name) = doc.feature_name_at(x, z) {
                    features.push(((x - x0, z - z0), name.to_string()));
                }
            }
        }
        Clip { tiles, heights, features, void }
    }

    /// Pastes with the top-left tile at `(tx, tz)`: the tiles, and what `with` asks for.
    pub fn paste(&self, doc: &mut MapDoc, data: &GameData, tx: i32, tz: i32, with: PasteWith) -> Pasted {
        let mut pasted = Pasted { tiles: self.tiles.stamp(doc, tx, tz), ..Default::default() };
        let (w, h) = (doc.tnt.width as i32, doc.tnt.height as i32);
        let (cw, ch) = (self.tiles.w as i32 * 2, self.tiles.h as i32 * 2);
        let inside = |x: i32, z: i32| x >= 0 && z >= 0 && x < w && z < h;
        if with.heights {
            // (Relative: the copy's edge is its level, laid on the ground there.)
            let level = if with.relative { edge_mean(&self.heights, cw as usize, ch as usize) } else { 0.0 };
            for (i, &value) in self.heights.iter().enumerate() {
                let (x, z) = (tx * 2 + i as i32 % cw, tz * 2 + i as i32 / cw);
                if !inside(x, z) {
                    continue;
                }
                let slot = &mut doc.tnt.heights[(z * w + x) as usize];
                let value = if with.relative { (*slot as f32 + value as f32 - level).round().clamp(0.0, 255.0) as u8 } else { value };
                if *slot != value {
                    *slot = value;
                    pasted.cells = Rect::merge(pasted.cells, Some(Rect::new(x as u32, z as u32, x as u32 + 1, z as u32 + 1)));
                }
            }
        }
        if with.features {
            // What starts in the region goes (and its void cells), then the copy's are put in.
            let (x0, z0) = (tx * 2, tz * 2);
            let gone: Vec<(u32, u32)> = doc
                .features()
                .map(|(o, _)| o)
                .filter(|&(x, z)| (x as i32) >= x0 && (z as i32) >= z0 && (x as i32) < x0 + cw && (z as i32) < z0 + ch)
                .collect();
            for o in gone {
                pasted.features |= doc.remove_feature(data, o);
            }
            for z in z0.max(0)..(z0 + ch).min(h) {
                for x in x0.max(0)..(x0 + cw).min(w) {
                    let cell = &mut doc.tnt.features[(z * w + x) as usize];
                    if *cell == FEATURE_VOID {
                        *cell = FEATURE_NONE;
                        pasted.features = true;
                    }
                }
            }
            for &(x, z) in &self.void {
                let (x, z) = (x0 + x as i32, z0 + z as i32);
                if inside(x, z) {
                    doc.tnt.features[(z * w + x) as usize] = FEATURE_VOID;
                    pasted.features = true;
                }
            }
            for ((x, z), name) in &self.features {
                let (x, z) = (x0 + *x as i32, z0 + *z as i32);
                if inside(x, z) {
                    pasted.features |= doc.place_feature(data, name, (x as u32, z as u32));
                }
            }
        }
        pasted
    }
}

/// The mean of a `w` × `h` grid's outermost cells.
fn edge_mean(heights: &[u8], w: usize, h: usize) -> f32 {
    let (mut sum, mut count) = (0.0, 0.0);
    for z in 0..h {
        for x in 0..w {
            if x == 0 || z == 0 || x + 1 == w || z + 1 == h {
                sum += heights[z * w + x] as f32;
                count += 1.0;
            }
        }
    }
    if count > 0.0 { sum / count } else { 0.0 }
}

/// The tile rectangle between two art positions (either order), at most `max` tiles a side.
pub fn tile_rect(a: (i32, i32), b: (i32, i32), tw: u32, th: u32, max: u32) -> Rect {
    let clamp = |v: i32, hi: u32| v.clamp(0, hi as i32 - 1) as u32;
    // (At most `max` tiles from where the drag began, whichever way it went.)
    let span = |a: i32, b: i32| {
        let reach = max.max(1) as i32 - 1;
        let b = b.clamp(a - reach, a + reach);
        (a.min(b), a.max(b))
    };
    let ((x0, x1), (z0, z1)) = (span(a.0, b.0), span(a.1, b.1));
    let (x0, x1) = (clamp(x0, tw), clamp(x1, tw));
    let (z0, z1) = (clamp(z0, th), clamp(z1, th));
    Rect::new(x0, z0, x1 + 1, z1 + 1)
}

/// Where the scatter brush puts features: on land, or under water.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ground {
    Land,
    Water,
}

/// The scatter brush.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scatter {
    pub radius: f32,
    /// How thick: from 1 (five and a half of a feature's sizes apart) to 10 (as close as they
    /// fit).
    pub density: f32,
    pub ground: Ground,
}

impl Scatter {
    /// How far apart the features are kept, in their own sizes.
    fn apart(&self) -> f32 {
        1.0 + (10.0 - self.density.clamp(1.0, 10.0)) * 0.5
    }

    /// `tries` features dropped at random in the circle round `at`, each one `pick`ed (from a
    /// number in 0..1), where the ground suits and no other feature stands nearer than the
    /// density allows. Returns how many were placed.
    pub fn spray(&self, doc: &mut MapDoc, data: &GameData, seed: &mut u64, at: (f32, f32), tries: usize, pick: impl Fn(f32) -> usize) -> usize {
        let sea = doc.tnt.sea_level as f32;
        let mut placed = 0;
        for _ in 0..tries {
            let (u, v, k) = (random(seed), random(seed), random(seed));
            let (r, a) = (self.radius * u.sqrt(), v * std::f32::consts::TAU);
            let (x, z) = (at.0 + r * a.cos(), at.1 + r * a.sin());
            if x < 0.0 || z < 0.0 || x >= doc.width_px() || z >= doc.height_px() {
                continue;
            }
            if (doc.ground(x, z) < sea) != (self.ground == Ground::Water) {
                continue;
            }
            let d = &data.features[pick(k)];
            let (fx, fz) = (d.footprint_x.max(1), d.footprint_z.max(1));
            let Some(origin) = footprint_origin(doc, x, z, fx, fz) else { continue };
            let middle = ((origin.0 as f32 + fx as f32 / 2.0) * CELL_PX, (origin.1 as f32 + fz as f32 / 2.0) * CELL_PX);
            if doc.feature_near(data, middle, fx.max(fz) as f32 * CELL_PX * self.apart()) {
                continue;
            }
            if doc.place_feature(data, &d.name, origin) {
                placed += 1;
            }
        }
        placed
    }
}

/// The next number in 0..1 of a xorshift sequence (`seed` never 0).
pub fn random(seed: &mut u64) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed >> 40) as f32 / (1u64 << 24) as f32
}

/// The origin cell that centres a `fx` × `fz` footprint on world point (x, z), inside the map.
pub fn footprint_origin(doc: &MapDoc, x: f32, z: f32, fx: u32, fz: u32) -> Option<(u32, u32)> {
    let ox = (x / CELL_PX - fx as f32 / 2.0).round() as i32;
    let oz = (z / CELL_PX - fz as f32 / 2.0).round() as i32;
    let (w, h) = (doc.tnt.width as i32, doc.tnt.height as i32);
    if fx as i32 > w || fz as i32 > h {
        return None;
    }
    Some((ox.clamp(0, w - fx as i32) as u32, oz.clamp(0, h - fz as i32) as u32))
}

/// Removes every feature whose footprint the circle of `radius` round world point (x, z)
/// touches. Returns how many.
pub fn erase_features(doc: &mut MapDoc, data: &GameData, x: f32, z: f32, radius: f32) -> usize {
    let found: Vec<(u32, u32)> = doc
        .features()
        .filter(|&((ox, oz), name)| {
            let (fx, fz) = MapDoc::footprint(data, name);
            let (x0, z0) = (ox as f32 * CELL_PX, oz as f32 * CELL_PX);
            let (x1, z1) = (x0 + fx as f32 * CELL_PX, z0 + fz as f32 * CELL_PX);
            let (nx, nz) = (x.clamp(x0, x1), z.clamp(z0, z1));
            (nx - x).powi(2) + (nz - z).powi(2) <= radius * radius
        })
        .map(|(o, _)| o)
        .collect();
    for &o in &found {
        doc.remove_feature(data, o);
    }
    found.len()
}
