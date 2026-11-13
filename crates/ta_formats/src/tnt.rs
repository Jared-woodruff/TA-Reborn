//! TNT map reader (TA format, version `0x2000`).
//!
//! A TA map is a grid of 32×32-pixel graphic tiles over a height/feature grid with 16-pixel
//! cells (so each tile covers 2×2 cells). Header (all i32):
//! `version, width, height` (in 16 px cells), `tile map ptr` (u16 per 32 px tile),
//! `attribute ptr` (4 bytes per cell: height u8, feature u16, pad u8), `tile graphics ptr`
//! (1024 bytes per tile), `tile count`, `feature type count`, `feature names ptr`
//! (132-byte records: i32 index + 128-byte name), `sea level`, `minimap ptr`, then `1` and
//! four zeros (64 bytes). [`TntMap::write`] produces the same layout.
//!
//! TA Reborn's one addition: marks per cell ([`TntMap::flags`], `CELL_*` bits) in each cell's
//! spare 4th attribute byte, which TA writes 0 and never reads. The header's last word
//! ([`FLAGS_MAGIC`]) says the bytes are marks, so a map whose spare bytes hold anything else is
//! never read as marked. A map without marks is written as TA writes it.

use crate::bin::{fixed_str, i32_at, slice, u16_at, u8_at};
use crate::image::{IndexedImage, Palette};
use crate::{FormatError, Result};

pub const TILE_SIZE: usize = 32;
pub const CELL_SIZE: usize = 16;
/// Graphic tile indices use all 16 bits; unlike feature slots, none are reserved.
pub const MAX_TILES: usize = u16::MAX as usize + 1;
const TNT_VERSION_TA: i32 = 0x2000;
/// Bytes of the header [`TntMap::write`] writes (the data starts right after it).
const HEADER_SIZE: usize = 64;
/// Side of the minimap canvas every map stores, whatever its shape.
pub const MINIMAP_CANVAS: usize = 252;
/// The colour TA fills the minimap canvas with outside the map's picture.
pub const MINIMAP_FILLER: u8 = 100;
/// Largest map side, in 16 px cells (262,144 px; TA's own maps are far smaller).
pub const MAX_SIDE: u32 = 16384;
/// Most cells a map may have (16384 × 16384): a corrupt header can't make the reader set
/// aside absurd amounts of memory.
pub const MAX_CELLS: u64 = 1 << 28;

/// Feature slot values with special meaning.
pub const FEATURE_NONE: u16 = 0xFFFF;
/// A cell covered by a feature larger than one cell, whose name index sits in the
/// footprint's top-left cell.
pub const FEATURE_COVERED: u16 = 0xFFFE;
/// Impassable "void" cells, drawn black.
pub const FEATURE_VOID: u16 = 0xFFFC;

/// The header's last word on a map whose cells' spare bytes are TA Reborn's marks
/// ([`TntMap::flags`]).
pub const FLAGS_MAGIC: i32 = i32::from_le_bytes(*b"RBCF");
/// A cliff: too steep for anything that minds slopes to cross, and for building on (a map
/// converted from another game keeps its cliffs, whatever its heights became).
pub const CELL_CLIFF: u8 = 1;

#[derive(Clone, Debug)]
pub struct TntMap {
    /// Width and height in 16-pixel cells.
    pub width: u32,
    pub height: u32,
    pub sea_level: i32,
    /// Graphic tiles, 32×32 palette indices each.
    pub tiles: Vec<Box<[u8; TILE_SIZE * TILE_SIZE]>>,
    /// Tile index per 32-pixel tile, `(width/2) × (height/2)`.
    pub tile_map: Vec<u16>,
    /// Height per cell, `width × height`.
    pub heights: Vec<u8>,
    /// Feature slot per cell (index into `feature_names` or a `FEATURE_*` constant).
    pub features: Vec<u16>,
    pub feature_names: Vec<String>,
    pub minimap: IndexedImage,
    /// TA Reborn's marks per cell (`CELL_*` bits), `width × height`; empty when the map has
    /// none (every map of TA's).
    pub flags: Vec<u8>,
}

/// Shared bounds validation for full terrain loads and lightweight library previews.
struct Header {
    width: u32, height: u32, sea_level: i32,
    tile_map_ptr: usize, attr_ptr: usize, tile_gfx_ptr: usize,
    tile_count: usize, feature_count: usize, feature_ptr: usize, minimap_ptr: usize,
    /// The cells' spare bytes are TA Reborn's marks ([`FLAGS_MAGIC`]).
    flagged: bool,
}

impl Header {
    fn parse(data: &[u8]) -> Result<Self> {
        let version = i32_at(data, 0)?;
        if version != TNT_VERSION_TA {
            return Err(FormatError::invalid("TNT", format!("unsupported version {version:#x}")));
        }
        let width = i32_at(data, 4)?;
        let height = i32_at(data, 8)?;
        if width <= 0 || height <= 0 || width % 2 != 0 || height % 2 != 0 || width as u32 > MAX_SIDE || height as u32 > MAX_SIDE || width as u64 * height as u64 > MAX_CELLS {
            return Err(FormatError::invalid("TNT", format!("bad size {width}x{height}")));
        }
        let (width, height) = (width as u32, height as u32);
        // (Offsets are i32 in the file; a negative one points nowhere.)
        let ptr = |at: usize| -> Result<usize> { Ok(usize::try_from(i32_at(data, at)?).unwrap_or(usize::MAX)) };
        let tile_map_ptr = ptr(12)?;
        let attr_ptr = ptr(16)?;
        let tile_gfx_ptr = ptr(20)?;
        // (The tile map's u16 indices reach the first 65,536 tiles; the rest can't be used.)
        let tile_count = (i32_at(data, 24)?.max(0) as usize).min(MAX_TILES);
        let feature_count = i32_at(data, 28)?.max(0) as usize;
        let feature_ptr = ptr(32)?;
        let sea_level = i32_at(data, 36)?;
        let minimap_ptr = ptr(40)?;
        let flagged = i32_at(data, 60).is_ok_and(|word| word == FLAGS_MAGIC);
        if tile_count == 0 {
            return Err(FormatError::invalid("TNT", "map has no graphic tiles"));
        }

        let tw = (width / 2) as usize;
        let th = (height / 2) as usize;
        let cells = width as usize * height as usize;
        // Every table must lie within the file before room is made for it.
        let fits = |start: usize, len: usize| start.checked_add(len).is_some_and(|end| end <= data.len());
        // (The last cell's padding byte may be missing.)
        if !fits(tile_map_ptr, tw * th * 2)
            || !fits(attr_ptr, cells * 4 - 1)
            || !fits(tile_gfx_ptr, tile_count.saturating_mul(TILE_SIZE * TILE_SIZE))
            || !fits(feature_ptr, feature_count.saturating_mul(132))
        {
            return Err(FormatError::invalid("TNT", format!("{width}x{height} map, {tile_count} tiles and {feature_count} features do not fit in {} bytes", data.len())));
        }
        Ok(Self { width, height, sea_level, tile_map_ptr, attr_ptr, tile_gfx_ptr,
            tile_count, feature_count, feature_ptr, minimap_ptr, flagged })
    }
}

impl TntMap {
    /// Reads the stored minimap without allocating terrain, tile or feature tables.
    /// Header/table bounds match `parse`; a missing or damaged preview is an error,
    /// rather than an empty image that could leave a library waiting indefinitely.
    pub fn preview(data: &[u8]) -> Result<IndexedImage> {
        let header = Header::parse(data)?;
        if header.minimap_ptr < HEADER_SIZE {
            return Err(FormatError::invalid("TNT", "minimap overlaps the file header"));
        }
        let image = parse_minimap(data, header.minimap_ptr)?;
        if image.width == 0 || image.height == 0 {
            return Err(FormatError::invalid("TNT", "map has no usable preview"));
        }
        Ok(crop_minimap(image, header.width, header.height))
    }

    /// The map's width and height in 16-pixel cells, read from its header alone (with the
    /// same checks as `parse`): a library's size without loading the terrain.
    pub fn size(data: &[u8]) -> Result<(u32, u32)> {
        let header = Header::parse(data)?;
        Ok((header.width, header.height))
    }

    pub fn parse(data: &[u8]) -> Result<TntMap> {
        let Header { width, height, sea_level, tile_map_ptr, attr_ptr, tile_gfx_ptr,
            tile_count, feature_count, feature_ptr, minimap_ptr, flagged } = Header::parse(data)?;
        let tw = (width / 2) as usize;
        let th = (height / 2) as usize;
        let cells = width as usize * height as usize;
        let mut tile_map = Vec::with_capacity(tw * th);
        for i in 0..tw * th {
            let t = u16_at(data, tile_map_ptr + i * 2)?;
            tile_map.push(if (t as usize) < tile_count { t } else { 0 });
        }

        let mut heights = Vec::with_capacity(cells);
        let mut features = Vec::with_capacity(cells);
        let mut flags = Vec::with_capacity(if flagged { cells } else { 0 });
        for i in 0..cells {
            let o = attr_ptr + i * 4;
            heights.push(u8_at(data, o)?);
            features.push(u16_at(data, o + 1)?);
            if flagged {
                // (The last cell's spare byte may be missing.)
                flags.push(u8_at(data, o + 3).unwrap_or(0));
            }
        }
        if flags.iter().all(|&f| f == 0) {
            flags = Vec::new();
        }

        let mut tiles = Vec::with_capacity(tile_count);
        for t in 0..tile_count {
            let src = slice(data, tile_gfx_ptr + t * TILE_SIZE * TILE_SIZE, TILE_SIZE * TILE_SIZE)?;
            let mut tile = Box::new([0u8; TILE_SIZE * TILE_SIZE]);
            tile.copy_from_slice(src);
            tiles.push(tile);
        }

        let mut feature_names = vec![String::new(); feature_count];
        for i in 0..feature_count {
            let o = feature_ptr + i * 132;
            let index = i32_at(data, o)?;
            let name = fixed_str(data, o + 4, 128)?;
            match usize::try_from(index).ok().filter(|&x| x < feature_count) {
                Some(x) => feature_names[x] = name,
                None => feature_names[i] = name,
            }
        }

        let minimap = crop_minimap(parse_minimap(data, minimap_ptr).unwrap_or_default(), width, height);

        Ok(TntMap { width, height, sea_level, tiles, tile_map, heights, features, feature_names, minimap, flags })
    }

    /// The marks of a cell (`CELL_*` bits; 0 off the map or on a map without marks).
    #[inline]
    pub fn flags_at(&self, cx: u32, cz: u32) -> u8 {
        if cx >= self.width || cz >= self.height { return 0; }
        self.flags.get((cz * self.width + cx) as usize).copied().unwrap_or(0)
    }

    pub fn width_px(&self) -> u32 {
        self.width * CELL_SIZE as u32
    }

    /// The ground the minimap picture shows, in pixels from the top-left corner: all but
    /// the last 2 columns and 8 rows of cells (see docs/FORMATS.md).
    pub fn minimap_area_px(&self) -> (u32, u32) {
        (self.width.saturating_sub(2).max(1) * CELL_SIZE as u32, self.height.saturating_sub(8).max(1) * CELL_SIZE as u32)
    }

    pub fn height_px(&self) -> u32 {
        self.height * CELL_SIZE as u32
    }

    #[inline]
    pub fn height_at(&self, cx: u32, cz: u32) -> u8 {
        self.heights[(cz.min(self.height - 1) * self.width + cx.min(self.width - 1)) as usize]
    }

    /// Serializes the map as a TA TNT file (version `0x2000`): the 64-byte header, then the
    /// tile map, the cell attributes (height, feature slot, a zero byte; from the next 16-byte
    /// boundary, as in TA's files), the tile graphics,
    /// the feature names (`index` + 128-byte NUL-padded name, Windows-1252) and the minimap:
    /// declared 252×252 like TA's, the picture in the top-left corner of the canvas and the
    /// rest [`MINIMAP_FILLER`] (0 when the picture's own last row or column is all 100, so
    /// reading the file back crops it to the same picture).
    ///
    /// Fails when the grids do not match the size (width and height must be even: a tile
    /// covers 2×2 cells), a tile index or feature slot points past its table, or a feature
    /// name does not fit its 127 bytes.
    pub fn write(&self) -> Result<Vec<u8>> {
        let invalid = |detail: String| FormatError::invalid("TNT", detail);
        let (w, h) = (self.width as usize, self.height as usize);
        if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 || w > MAX_SIDE as usize || h > MAX_SIDE as usize || (w * h) as u64 > MAX_CELLS {
            return Err(invalid(format!("bad size {w}x{h} (cells, must be even)")));
        }
        let (tw, th) = (w / 2, h / 2);
        if self.tile_map.len() != tw * th || self.heights.len() != w * h || self.features.len() != w * h || !(self.flags.is_empty() || self.flags.len() == w * h) {
            return Err(invalid(format!(
                "grid sizes {} tiles, {} heights, {} features, {} marks do not match {w}x{h} cells",
                self.tile_map.len(),
                self.heights.len(),
                self.features.len(),
                self.flags.len()
            )));
        }
        // (Marks only when a cell has one: a map without is TA's, byte for byte.)
        let flagged = self.flags.iter().any(|&f| f != 0);
        if self.tiles.is_empty() || self.tiles.len() > MAX_TILES {
            return Err(invalid(format!("{} tiles", self.tiles.len())));
        }
        if let Some(t) = self.tile_map.iter().find(|&&t| t as usize >= self.tiles.len()) {
            return Err(invalid(format!("tile index {t} past the {} tiles", self.tiles.len())));
        }
        let names = self.feature_names.len();
        if let Some(f) = self.features.iter().find(|&&f| f < FEATURE_VOID && f as usize >= names) {
            return Err(invalid(format!("feature slot {f} past the {names} feature names")));
        }
        let mut encoded_names = Vec::with_capacity(names);
        for name in &self.feature_names {
            let bytes: Vec<u8> = name.chars().map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect();
            if bytes.len() > 127 || bytes.contains(&0) {
                return Err(invalid(format!("feature name {name:?} does not fit")));
            }
            encoded_names.push(bytes);
        }
        let mm = &self.minimap;
        let (mw, mh) = (mm.width as usize, mm.height as usize);
        if mw > MINIMAP_CANVAS || mh > MINIMAP_CANVAS || mm.pixels.len() != mw * mh {
            return Err(invalid(format!("minimap {mw}x{mh} is larger than {MINIMAP_CANVAS}x{MINIMAP_CANVAS}")));
        }

        let tile_map_ptr = HEADER_SIZE;
        // TA's files start the attributes on a 16-byte boundary (zeros in between); the
        // sections after them fall on one anyway, except the minimap.
        let attr_ptr = (tile_map_ptr + tw * th * 2).next_multiple_of(16);
        let tile_gfx_ptr = attr_ptr + w * h * 4;
        let feature_ptr = tile_gfx_ptr + self.tiles.len() * TILE_SIZE * TILE_SIZE;
        let minimap_ptr = feature_ptr + names * 132;
        let total = minimap_ptr + 8 + MINIMAP_CANVAS * MINIMAP_CANVAS;
        // (The header's offsets are i32.)
        if total > i32::MAX as usize {
            return Err(invalid(format!("{total} bytes is more than a TNT file can address")));
        }
        let mut out = Vec::with_capacity(total);
        let header = [
            TNT_VERSION_TA,
            w as i32,
            h as i32,
            tile_map_ptr as i32,
            attr_ptr as i32,
            tile_gfx_ptr as i32,
            self.tiles.len() as i32,
            names as i32,
            feature_ptr as i32,
            self.sea_level,
            minimap_ptr as i32,
            1,
            0,
            0,
            0,
            if flagged { FLAGS_MAGIC } else { 0 },
        ];
        for v in header {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for &t in &self.tile_map {
            out.extend_from_slice(&t.to_le_bytes());
        }
        out.resize(attr_ptr, 0);
        for (i, (&height, &feature)) in self.heights.iter().zip(&self.features).enumerate() {
            out.push(height);
            out.extend_from_slice(&feature.to_le_bytes());
            out.push(if flagged { self.flags[i] } else { 0 });
        }
        for tile in &self.tiles {
            out.extend_from_slice(&tile[..]);
        }
        for (i, name) in encoded_names.iter().enumerate() {
            out.extend_from_slice(&(i as i32).to_le_bytes());
            let mut field = [0u8; 128];
            field[..name.len()].copy_from_slice(name);
            out.extend_from_slice(&field);
        }
        // The minimap: TA declares the whole canvas.
        let filler = minimap_filler(mm);
        out.extend_from_slice(&(MINIMAP_CANVAS as i32).to_le_bytes());
        out.extend_from_slice(&(MINIMAP_CANVAS as i32).to_le_bytes());
        let mut canvas = vec![filler; MINIMAP_CANVAS * MINIMAP_CANVAS];
        for y in 0..mh {
            canvas[y * MINIMAP_CANVAS..y * MINIMAP_CANVAS + mw].copy_from_slice(&mm.pixels[y * mw..(y + 1) * mw]);
        }
        out.extend_from_slice(&canvas);
        debug_assert_eq!(out.len(), total);
        Ok(out)
    }

    /// The minimap picture TA would store for this map: the art of its [`minimap_window`]
    /// box-filtered down to [`minimap_size`], each pixel the palette colour nearest to the
    /// mean of the art it covers. Index 0 and the canvas filler are never used, so the
    /// picture can't be mistaken for the canvas around it.
    pub fn build_minimap(&self, palette: &Palette) -> IndexedImage {
        let tw = self.width_px() as usize / TILE_SIZE;
        let (ww, wh) = minimap_window(self.width, self.height);
        let (art_w, art_h) = ((ww as usize * CELL_SIZE).min(self.width_px() as usize), (wh as usize * CELL_SIZE).min(self.height_px() as usize));
        let (mw, mh) = minimap_size(self.width, self.height);
        let pixel = |x: usize, y: usize| -> u8 {
            let tile = self.tile_map.get((y / TILE_SIZE) * tw + x / TILE_SIZE).copied().unwrap_or(0) as usize;
            self.tiles.get(tile).map_or(0, |t| t[(y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE])
        };
        let mut nearest = NearestColor::new(palette, &[0, MINIMAP_FILLER]);
        let mut img = IndexedImage::new(mw as u32, mh as u32, 0);
        for my in 0..mh {
            let (y0, y1) = (my * art_h / mh, ((my + 1) * art_h / mh).max(my * art_h / mh + 1));
            for mx in 0..mw {
                let (x0, x1) = (mx * art_w / mw, ((mx + 1) * art_w / mw).max(mx * art_w / mw + 1));
                // At most 8×8 samples per minimap pixel: plenty for a mean colour.
                let (sx, sy) = ((x1 - x0).div_ceil(8), (y1 - y0).div_ceil(8));
                let (mut sum, mut n) = ([0u32; 3], 0u32);
                for y in (y0..y1).step_by(sy.max(1)) {
                    for x in (x0..x1).step_by(sx.max(1)) {
                        let [r, g, b] = palette.rgb(pixel(x, y));
                        sum[0] += r as u32;
                        sum[1] += g as u32;
                        sum[2] += b as u32;
                        n += 1;
                    }
                }
                let n = n.max(1);
                img.pixels[my * mw + mx] = nearest.find([(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]);
            }
        }
        img.opaque.fill(true);
        img
    }

    /// Assembles the full map image as palette indices (`width_px × height_px`).
    pub fn render_indices(&self) -> Vec<u8> {
        let (w, h) = (self.width_px() as usize, self.height_px() as usize);
        let tw = w / TILE_SIZE;
        let mut out = vec![0u8; w * h];
        for ty in 0..h / TILE_SIZE {
            for tx in 0..tw {
                let tile = &self.tiles[self.tile_map[ty * tw + tx] as usize];
                for row in 0..TILE_SIZE {
                    let dst = (ty * TILE_SIZE + row) * w + tx * TILE_SIZE;
                    out[dst..dst + TILE_SIZE].copy_from_slice(&tile[row * TILE_SIZE..(row + 1) * TILE_SIZE]);
                }
            }
        }
        out
    }
}

/// The part of a `width` × `height`-cell map a minimap shows, in cells from the top-left
/// corner: all but the last 2 columns and the last 8 rows (what the shipped minimaps show,
/// matched against their art).
pub fn minimap_window(width: u32, height: u32) -> (u32, u32) {
    (width.saturating_sub(2).max(1), height.saturating_sub(8).max(1))
}

/// Size of the minimap picture for a map of `width` × `height` cells: its
/// [`minimap_window`] scaled to 252 pixels along the longer side, the other side in
/// proportion, rounded down. Every skirmish map but one of the GOG install matches exactly.
pub fn minimap_size(width: u32, height: u32) -> (usize, usize) {
    let (w, h) = minimap_window(width, height);
    let (w, h) = (w as usize, h as usize);
    if w >= h {
        (MINIMAP_CANVAS, (MINIMAP_CANVAS * h / w).clamp(1, MINIMAP_CANVAS))
    } else {
        ((MINIMAP_CANVAS * w / h).clamp(1, MINIMAP_CANVAS), MINIMAP_CANVAS)
    }
}

/// The canvas colour [`TntMap::write`] puts around a minimap picture: TA's filler, unless the
/// picture's last row or column is entirely that colour (reading back would crop it), then 0.
fn minimap_filler(img: &IndexedImage) -> u8 {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 || w == MINIMAP_CANVAS && h == MINIMAP_CANVAS {
        return MINIMAP_FILLER;
    }
    let edge_is = |c: u8| img.pixels[(h - 1) * w..].iter().all(|&p| p == c) || (0..h).all(|y| img.pixels[y * w + w - 1] == c);
    if edge_is(MINIMAP_FILLER) { 0 } else { MINIMAP_FILLER }
}

/// Nearest palette colour (squared RGB distance), cached per colour.
struct NearestColor<'a> {
    palette: &'a Palette,
    skip: &'a [u8],
    cache: std::collections::HashMap<[u8; 3], u8>,
}

impl<'a> NearestColor<'a> {
    fn new(palette: &'a Palette, skip: &'a [u8]) -> Self {
        NearestColor { palette, skip, cache: Default::default() }
    }

    fn find(&mut self, c: [u8; 3]) -> u8 {
        let (palette, skip) = (self.palette, self.skip);
        *self.cache.entry(c).or_insert_with(|| {
            let d = |i: usize| {
                let p = palette.colors[i];
                (0..3).map(|k| (p[k] as i32 - c[k] as i32).pow(2)).sum::<i32>()
            };
            (0..256usize).filter(|&i| !skip.contains(&(i as u8))).min_by_key(|&i| d(i)).unwrap_or(0) as u8
        })
    }
}

/// Minimaps are declared 252×252 even for maps that are not square: the map is drawn in the
/// top-left corner and the rest is a flat filler colour (index 100 on most maps, 0 on some
/// campaign maps). The filler is the canvas's bottom-right colour when that pixel's whole row
/// or column is uniform; trailing rows and columns entirely of it are cropped, never below
/// 90% of the size the map's aspect predicts.
fn crop_minimap(img: IndexedImage, map_w: u32, map_h: u32) -> IndexedImage {
    let (w, h) = (img.width as usize, img.height as usize);
    if map_w == 0 || map_h == 0 || w == 0 || h == 0 {
        return img;
    }
    let px = |x: usize, y: usize| img.pixels[y * w + x];
    let fill = px(w - 1, h - 1);
    let row_is = |y: usize, cols: usize| (0..cols).all(|x| px(x, y) == fill);
    let col_is = |x: usize, rows: usize| (0..rows).all(|y| px(x, y) == fill);
    if fill != 100 && fill != 0 || !row_is(h - 1, w) && !col_is(w - 1, h) {
        return img; // No filler: a square map, or real content in the corner.
    }
    // The aspect gives a lower bound on the content.
    let (aw, ah) = if map_w >= map_h { (w, (w as u64 * map_h as u64 / map_w as u64) as usize) } else { ((h as u64 * map_w as u64 / map_h as u64) as usize, h) };
    let mut ch = h;
    while ch > (ah * 9 / 10).max(1) && row_is(ch - 1, w) {
        ch -= 1;
    }
    let mut cw = w;
    while cw > (aw * 9 / 10).max(1) && col_is(cw - 1, ch) {
        cw -= 1;
    }
    if (cw, ch) == (w, h) {
        return img;
    }
    let mut out = IndexedImage::new(cw as u32, ch as u32, 0);
    for y in 0..ch {
        out.pixels[y * cw..(y + 1) * cw].copy_from_slice(&img.pixels[y * w..y * w + cw]);
    }
    out.opaque.fill(true);
    out
}

/// Minimap: `{width: i32, height: i32}` followed by a 252×252 byte canvas of which the
/// top-left `width × height` is used.
fn parse_minimap(data: &[u8], ptr: usize) -> Result<IndexedImage> {
    const CANVAS: usize = 252;
    let w = i32_at(data, ptr)?.clamp(0, CANVAS as i32) as usize;
    let h = i32_at(data, ptr + 4)?.clamp(0, CANVAS as i32) as usize;
    let canvas = slice(data, ptr + 8, CANVAS * CANVAS)?;
    let mut img = IndexedImage::new(w as u32, h as u32, 0);
    for y in 0..h {
        img.pixels[y * w..(y + 1) * w].copy_from_slice(&canvas[y * CANVAS..y * CANVAS + w]);
    }
    img.opaque.fill(true);
    Ok(img)
}

#[cfg(test)]
mod size_tests {
    use super::*;

    fn flat(width: u32, height: u32) -> TntMap {
        let cells = (width * height) as usize;
        TntMap {
            width,
            height,
            sea_level: 0,
            tiles: vec![Box::new([7; TILE_SIZE * TILE_SIZE])],
            tile_map: vec![0; cells / 4],
            heights: (0..cells).map(|i| (i % 251) as u8).collect(),
            features: vec![FEATURE_NONE; cells],
            feature_names: Vec::new(),
            minimap: IndexedImage::new(1, 1, 3),
            flags: Vec::new(),
        }
    }

    /// A map's marks (a converted map's cliffs) read back as written; a map without any is
    /// written as TA writes maps; spare bytes a map's header does not call marks are not read.
    #[test]
    fn a_maps_cell_marks_read_back_and_only_where_the_header_says_so() {
        let mut map = flat(4, 4);
        let plain = map.write().unwrap();
        assert_eq!(i32_at(&plain, 60).unwrap(), 0, "no marks: TA's header");
        map.flags = vec![0; 16];
        assert_eq!(map.write().unwrap(), plain, "marks all clear: TA's bytes");
        map.flags[5] = CELL_CLIFF;
        map.flags[15] = CELL_CLIFF;
        let marked = map.write().unwrap();
        assert_eq!(i32_at(&marked, 60).unwrap(), FLAGS_MAGIC);
        let again = TntMap::parse(&marked).unwrap();
        assert_eq!(again.flags, map.flags);
        assert_eq!((again.flags_at(1, 1), again.flags_at(0, 0), again.flags_at(9, 9)), (CELL_CLIFF, 0, 0));
        assert_eq!(again.heights, map.heights, "the heights and features beside them are as they were");
        // The same spare bytes under TA's header: a map of another tool's, read unmarked.
        let mut stray = marked.clone();
        stray[60..64].copy_from_slice(&[0; 4]);
        assert!(TntMap::parse(&stray).unwrap().flags.is_empty());
        // Marks of the wrong size are refused.
        map.flags = vec![CELL_CLIFF; 3];
        assert!(map.write().is_err());
    }

    #[test]
    fn maps_past_tas_old_limit_read_back() {
        // 8192 cells across (twice the old limit of 4096), two down.
        let map = flat(8192, 2);
        let again = TntMap::parse(&map.write().unwrap()).unwrap();
        assert_eq!((again.width, again.height), (8192, 2));
        assert_eq!(again.heights, map.heights);
        assert!(flat(MAX_SIDE + 2, 2).write().is_err(), "past the largest side");
    }

    #[test]
    fn every_u16_graphic_tile_index_can_be_written_and_read() {
        let mut map = flat(4, 4);
        map.tiles = (0..MAX_TILES).map(|i| {
            let mut tile = Box::new([7; TILE_SIZE * TILE_SIZE]);
            tile[..2].copy_from_slice(&(i as u16).to_le_bytes());
            tile
        }).collect();
        map.tile_map = vec![0, 32768, u16::MAX, u16::MAX];
        let bytes = map.write().unwrap();
        assert_eq!(i32_at(&bytes, 24).unwrap(), MAX_TILES as i32);
        map.tiles.push(Box::new([0; TILE_SIZE * TILE_SIZE]));
        assert!(map.write().is_err(), "a 65,537th tile cannot be indexed");
        drop(map);
        let parsed = TntMap::parse(&bytes).unwrap();
        assert_eq!(parsed.tiles.len(), MAX_TILES);
        assert_eq!(parsed.tile_map, [0, 32768, u16::MAX, u16::MAX]);
        assert_eq!(&parsed.tiles[32768][..2], &32768u16.to_le_bytes());
        assert_eq!(&parsed.tiles[u16::MAX as usize][..2], &u16::MAX.to_le_bytes());
    }

    #[test]
    fn corrupt_headers_are_refused_before_making_room() {
        let mut bytes = flat(4, 4).write().unwrap();
        // A header claiming the largest map (2^28 cells) over a file of a few hundred bytes.
        bytes[4..8].copy_from_slice(&(MAX_SIDE as i32).to_le_bytes());
        bytes[8..12].copy_from_slice(&(MAX_SIDE as i32).to_le_bytes());
        assert!(TntMap::parse(&bytes).is_err());
        // Two billion tiles or features.
        let mut bytes = flat(4, 4).write().unwrap();
        bytes[24..28].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(TntMap::parse(&bytes).is_err());
        let mut bytes = flat(4, 4).write().unwrap();
        bytes[28..32].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(TntMap::parse(&bytes).is_err());
        // Past the largest side.
        let mut bytes = flat(4, 4).write().unwrap();
        bytes[4..8].copy_from_slice(&(MAX_SIDE as i32 + 2).to_le_bytes());
        assert!(TntMap::parse(&bytes).is_err());
        // A drawable map needs complete 2x2-cell tiles and at least one graphic tile.
        let mut bytes = flat(4, 4).write().unwrap();
        bytes[4..8].copy_from_slice(&3i32.to_le_bytes());
        assert!(TntMap::parse(&bytes).is_err());
        let mut bytes = flat(4, 4).write().unwrap();
        bytes[24..28].copy_from_slice(&0i32.to_le_bytes());
        assert!(TntMap::parse(&bytes).is_err());
    }

    #[test]
    fn lightweight_preview_matches_full_load_for_rectangular_and_legacy_canvases() {
        for (width, height) in [(4, 4), (210, 126), (126, 210), (512, 512)] {
            let mut map = flat(width, height);
            let (mw, mh) = minimap_size(width, height);
            map.minimap = IndexedImage::new(mw as u32, mh as u32, 9);
            for (index, pixel) in map.minimap.pixels.iter_mut().enumerate() { *pixel = 7 + (index % 3) as u8; }
            let mut bytes = map.write().unwrap();
            assert_eq!(TntMap::preview(&bytes).unwrap(), TntMap::parse(&bytes).unwrap().minimap);
            // Some TA campaigns declare a 256-row canvas; both readers cap at 252.
            let at = i32_at(&bytes, 40).unwrap() as usize;
            bytes[at + 4..at + 8].copy_from_slice(&256i32.to_le_bytes());
            assert_eq!(TntMap::preview(&bytes).unwrap(), TntMap::parse(&bytes).unwrap().minimap);
        }
    }

    #[test]
    fn lightweight_preview_checks_terrain_bounds_and_rejects_missing_or_truncated_images() {
        let valid = flat(16, 16).write().unwrap();
        for (at, value) in [(0, 0), (4, 3), (8, MAX_SIDE as i32 + 2), (12, -1), (16, i32::MAX),
            (20, -1), (24, 0), (28, i32::MAX), (32, -1)] {
            let mut bytes = valid.clone();
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            assert!(TntMap::parse(&bytes).is_err(), "full parser must reject field {at}");
            assert!(TntMap::preview(&bytes).is_err(), "preview must validate field {at} too");
        }
        let at = i32_at(&valid, 40).unwrap() as usize;
        for bytes in [&valid[..0], &valid[..40], &valid[..at], &valid[..valid.len() - 1]] {
            assert!(TntMap::preview(bytes).is_err());
        }
        for field in [at, at + 4] {
            let mut bytes = valid.clone();
            bytes[field..field + 4].copy_from_slice(&0i32.to_le_bytes());
            assert!(TntMap::preview(&bytes).is_err(), "empty canvases are not usable previews");
        }
        let mut bytes = valid;
        bytes[40..44].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(TntMap::preview(&bytes).is_err());
        for offset in [0i32, 4, 40, 60] {
            bytes[40..44].copy_from_slice(&offset.to_le_bytes());
            assert!(TntMap::preview(&bytes).is_err(), "a header field must not become minimap pixels");
        }
    }

    /// A diagnostic, not a wall-clock assertion: compare identical synthetic map bytes
    /// without disk/decompression noise and report the owned buffers each route produces.
    #[test]
    #[ignore = "performance diagnostic"]
    fn lightweight_preview_cost() {
        use std::{hint::black_box, time::Instant};
        let mut map = flat(2048, 2048);
        map.tiles = (0..512).map(|_| Box::new([7; TILE_SIZE * TILE_SIZE])).collect();
        map.minimap = IndexedImage::new(252, 252, 7);
        let bytes = map.write().unwrap();
        drop(map);
        let parsed = TntMap::parse(&bytes).unwrap();
        let preview = TntMap::preview(&bytes).unwrap();
        assert_eq!(preview, parsed.minimap);
        let image_bytes = |image: &IndexedImage| image.pixels.capacity() + image.opaque.capacity();
        let full_bytes = parsed.tile_map.capacity() * 2 + parsed.heights.capacity() + parsed.features.capacity() * 2
            + parsed.tiles.len() * TILE_SIZE * TILE_SIZE + parsed.tiles.capacity() * std::mem::size_of::<Box<[u8; 1024]>>()
            + image_bytes(&parsed.minimap);
        let preview_bytes = image_bytes(&preview);
        drop(parsed); drop(preview);
        let start = Instant::now();
        for _ in 0..30 { black_box(TntMap::parse(black_box(&bytes)).unwrap()); }
        let full_ms = start.elapsed().as_secs_f64() * 1000.0 / 30.0;
        let start = Instant::now();
        for _ in 0..300 { black_box(TntMap::preview(black_box(&bytes)).unwrap()); }
        let preview_ms = start.elapsed().as_secs_f64() * 1000.0 / 300.0;
        println!("2048x2048 cells, 512 tiles: full {full_ms:.3} ms / {full_bytes} owned buffer bytes; preview {preview_ms:.3} ms / {preview_bytes} owned buffer bytes");
    }
}

#[cfg(test)]
mod minimap_tests {
    use super::*;

    fn canvas(content_w: usize, content_h: usize, fill: u8) -> IndexedImage {
        let mut img = IndexedImage::new(252, 252, fill);
        for y in 0..content_h {
            for x in 0..content_w {
                img.pixels[y * 252 + x] = 7 + ((x + y) % 3) as u8;
            }
        }
        img
    }

    #[test]
    fn filler_rows_and_columns_are_cropped() {
        let wide = crop_minimap(canvas(252, 142, 100), 210, 126);
        assert_eq!((wide.width, wide.height), (252, 142));
        let tall = crop_minimap(canvas(216, 252, 0), 386, 456);
        assert_eq!((tall.width, tall.height), (216, 252));
        // Square maps keep everything, even a uniform bottom that is real content.
        let mut sea = canvas(252, 252, 100);
        for p in &mut sea.pixels[200 * 252..] {
            *p = 57;
        }
        let square = crop_minimap(sea, 640, 640);
        assert_eq!((square.width, square.height), (252, 252));
    }
}
