//! The map being edited: TA's TNT grids (heights, tiles, features) plus what its OTA holds
//! (name, description, planet, wind, tides, gravity, metal, start positions).
//!
//! Heights live per cell *vertex* at world `(x·16, z·16)`, as the renderer's height field
//! reads them. Tiles are 32 px squares of painted art; the art at ground point `(x, y, z)` is
//! pixel `(x, z − y/2)` (TA's oblique projection, see FORMATS.md), so tools that paint what
//! the cursor shows work in art coordinates. A feature's name index sits in the top-left
//! cell of its footprint.

use std::collections::HashMap;

use anyhow::{Context, Result};
use ta_data::{GameData, MapInfo};
use ta_formats::tdf::{self, TdfDocument, TdfSection};
use ta_formats::tnt::{CELL_SIZE, FEATURE_COVERED, FEATURE_NONE, FEATURE_VOID, MAX_TILES, TILE_SIZE, TntMap};
use ta_vfs::Vfs;

/// TA's limit on start positions (StartPos1..StartPos10).
pub const MAX_STARTS: usize = 10;
/// Map sizes go in steps of 512 pixels (32 cells, 16 tiles).
pub const SIZE_STEP: u32 = 512;
/// The largest map: 40 steps (20,480 px, like TA's largest).
pub const MAX_SIZE_STEPS: u32 = 40;
/// Pixels per tile, as a float.
pub const TILE_PX: f32 = TILE_SIZE as f32;
/// Pixels per cell, as a float.
pub const CELL_PX: f32 = CELL_SIZE as f32;

/// A tile (graphics) of 32×32 palette indices.
pub type Tile = [u8; TILE_SIZE * TILE_SIZE];

/// A rectangle of grid positions (cells or tiles), `x0..x1` × `z0..z1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: u32,
    pub z0: u32,
    pub x1: u32,
    pub z1: u32,
}

impl Rect {
    pub fn new(x0: u32, z0: u32, x1: u32, z1: u32) -> Rect {
        Rect { x0, z0, x1, z1 }
    }

    pub fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.z0 >= self.z1
    }

    pub fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Rect { x0: self.x0.min(other.x0), z0: self.z0.min(other.z0), x1: self.x1.max(other.x1), z1: self.z1.max(other.z1) }
    }

    /// `a` ∪ `b` where either may be missing.
    pub fn merge(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
        match (a, b) {
            (Some(a), Some(b)) => Some(a.union(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn width(&self) -> u32 {
        self.x1.saturating_sub(self.x0)
    }

    pub fn height(&self) -> u32 {
        self.z1.saturating_sub(self.z0)
    }

    /// Clipped to a `w` × `h` grid.
    pub fn clip(self, w: u32, h: u32) -> Rect {
        Rect { x0: self.x0.min(w), z0: self.z0.min(h), x1: self.x1.min(w), z1: self.z1.min(h) }
    }
}

/// The OTA's properties the editor shows (the sea level lives in the TNT).
#[derive(Clone, Debug, PartialEq)]
pub struct MapProps {
    pub name: String,
    pub description: String,
    pub planet: String,
    pub min_wind: f32,
    pub max_wind: f32,
    pub tidal: f32,
    pub gravity: f32,
    /// Metal in every cell of the map (TA's `SurfaceMetal`): what an extractor finds on bare
    /// ground; deposits (features) carry their own.
    pub surface_metal: f32,
    /// TA's `MohoMetal`, kept for TA's own use.
    pub moho_metal: f32,
}

impl Default for MapProps {
    fn default() -> Self {
        MapProps {
            name: "New Map".into(),
            description: String::new(),
            planet: "Green Planet".into(),
            min_wind: 0.0,
            max_wind: 2000.0,
            tidal: 20.0,
            gravity: 112.0,
            surface_metal: 3.0,
            moho_metal: 30.0,
        }
    }
}

impl MapProps {
    fn from_info(info: &MapInfo) -> MapProps {
        let schema = info.schemas.first();
        MapProps {
            name: info.name.clone(),
            description: info.description.clone(),
            planet: info.planet.clone(),
            // (The editor's sliders cover TA's ranges, well within an f32.)
            min_wind: info.min_wind as f32,
            max_wind: info.max_wind as f32,
            tidal: info.tidal_strength as f32,
            gravity: info.gravity as f32,
            surface_metal: schema.map_or(3.0, |s| s.surface_metal as f32),
            moho_metal: schema.map_or(30.0, |s| s.moho_metal as f32),
        }
    }
}

/// What a new map is made of.
#[derive(Clone, Debug, PartialEq)]
pub struct NewMap {
    pub name: String,
    /// Size in steps of [`SIZE_STEP`] pixels.
    pub width_steps: u32,
    pub height_steps: u32,
    pub sea_level: u8,
    pub height: u8,
    /// The map whose tiles make the new map's palette (file stem).
    pub palette_map: String,
}

impl Default for NewMap {
    fn default() -> Self {
        NewMap { name: "New Map".into(), width_steps: 8, height_steps: 8, sea_level: 0, height: 40, palette_map: "Painted Desert".into() }
    }
}

/// A map being edited.
pub struct MapDoc {
    pub tnt: TntMap,
    pub props: MapProps,
    /// Start positions in world pixels, StartPos1 first.
    pub starts: Vec<[f32; 2]>,
    /// The OTA the map came from: its other keys (briefing, victory rules, other schemas'
    /// objects) are kept when saving.
    pub ota: Option<TdfDocument>,
    /// The name of the map this one started from (for a new map, the map whose tiles it
    /// took), for the title bar.
    pub origin: String,
    /// A new map's tile palette: the map whose tiles it started with (file stem).
    pub tiles_from: Option<String>,
    /// The map's own features (`maps/<name>.features`, a converted map's trees and rocks), saved
    /// beside it again: a copy names them too.
    pub features: Option<Vec<u8>>,
    /// Tile graphics already in `tnt.tiles`, by content hash (tiles imported from another map
    /// are shared, not duplicated).
    tile_lookup: HashMap<u64, Vec<u16>>,
}

fn tile_hash(tile: &Tile) -> u64 {
    // FNV-1a over the 1,024 bytes.
    tile.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

impl MapDoc {
    fn with(tnt: TntMap, props: MapProps, starts: Vec<[f32; 2]>, ota: Option<TdfDocument>, origin: String) -> MapDoc {
        let mut doc = MapDoc { tnt, props, starts, ota, origin, tiles_from: None, features: None, tile_lookup: HashMap::new() };
        doc.rebuild_tile_lookup();
        doc
    }

    fn rebuild_tile_lookup(&mut self) {
        self.tile_lookup.clear();
        for (i, t) in self.tnt.tiles.iter().enumerate() {
            self.tile_lookup.entry(tile_hash(t)).or_default().push(i as u16);
        }
    }

    /// Opens `maps/<name>.tnt` and its OTA.
    pub fn open(vfs: &Vfs, name: &str) -> Result<MapDoc> {
        let path = format!("maps/{name}.tnt");
        let tnt = TntMap::parse(&vfs.read(&path).with_context(|| path.clone())?).with_context(|| path.clone())?;
        let ota = vfs.read_opt(&format!("maps/{name}.ota")).map(|b| tdf::parse_bytes(&b));
        let info = ota.as_ref().map(MapInfo::from_ota).unwrap_or_default();
        // (A map without its OTA gets TA's usual climate, not gravity 0 and no planet.)
        let mut props = if ota.is_some() { MapProps::from_info(&info) } else { MapProps::default() };
        if props.name.trim().is_empty() {
            props.name = name.to_string();
        }
        let starts = info.schemas.first().map(|s| s.start_positions.iter().map(|&(_, x, z)| [x, z]).take(MAX_STARTS).collect()).unwrap_or_default();
        let origin = props.name.clone();
        let mut doc = MapDoc::with(tnt, props, starts, ota, origin);
        doc.features = vfs.read_opt(&format!("maps/{name}.features"));
        Ok(doc)
    }

    /// A new, flat map: `new`'s size, sea level and height, tiled with the most used tile of
    /// `palette` (whose whole tile set it starts with) and with its planet and climate.
    pub fn new_blank(new: &NewMap, palette: &TntMap, palette_info: Option<&MapInfo>) -> MapDoc {
        let steps = |s: u32| s.clamp(1, MAX_SIZE_STEPS) * (SIZE_STEP / CELL_SIZE as u32);
        let (w, h) = (steps(new.width_steps), steps(new.height_steps));
        let mut uses = vec![0u32; palette.tiles.len().max(1)];
        for &t in &palette.tile_map {
            if let Some(n) = uses.get_mut(t as usize) {
                *n += 1;
            }
        }
        let fill = uses.iter().enumerate().max_by_key(|&(i, n)| (*n, std::cmp::Reverse(i))).map_or(0, |(i, _)| i) as u16;
        let tiles = if palette.tiles.is_empty() { vec![Box::new([0u8; TILE_SIZE * TILE_SIZE])] } else { palette.tiles.clone() };
        let mut tnt = TntMap {
            width: w,
            height: h,
            sea_level: new.sea_level as i32,
            tiles,
            tile_map: vec![fill; ((w / 2) * (h / 2)) as usize],
            heights: vec![new.height; (w * h) as usize],
            features: vec![FEATURE_NONE; (w * h) as usize],
            feature_names: Vec::new(),
            minimap: Default::default(),
            flags: Vec::new(),
        };
        tnt.minimap = ta_formats::image::IndexedImage::new(1, 1, 0);
        let mut props = palette_info.map_or_else(MapProps::default, MapProps::from_info);
        props.name = new.name.clone();
        props.description = String::new();
        let (wp, hp) = (w as f32 * CELL_PX, h as f32 * CELL_PX);
        let starts = vec![[wp * 0.2, hp * 0.2], [wp * 0.8, hp * 0.8]];
        MapDoc::with(tnt, props, starts, None, new.palette_map.clone())
    }

    pub fn width_px(&self) -> f32 {
        self.tnt.width as f32 * CELL_PX
    }

    pub fn height_px(&self) -> f32 {
        self.tnt.height as f32 * CELL_PX
    }

    /// Size in tiles.
    pub fn tiles_wide(&self) -> u32 {
        self.tnt.width / 2
    }

    pub fn tiles_high(&self) -> u32 {
        self.tnt.height / 2
    }

    /// Ground height at a world point (the terrain mesh's triangulation).
    pub fn ground(&self, x: f32, z: f32) -> f32 {
        let (w, h) = (self.tnt.width as i32, self.tnt.height as i32);
        let at = |cx: i32, cz: i32| self.tnt.heights[(cz.clamp(0, h - 1) * w + cx.clamp(0, w - 1)) as usize] as f32;
        let (fx, fz) = (x / CELL_PX, z / CELL_PX);
        let (x0, z0) = (fx.floor(), fz.floor());
        let (tx, tz) = (fx - x0, fz - z0);
        let (x0, z0) = (x0 as i32, z0 as i32);
        let (h00, h11) = (at(x0, z0), at(x0 + 1, z0 + 1));
        if tx >= tz {
            let h10 = at(x0 + 1, z0);
            h00 + (h10 - h00) * tx + (h11 - h10) * tz
        } else {
            let h01 = at(x0, z0 + 1);
            h00 + (h11 - h01) * tx + (h01 - h00) * tz
        }
    }

    /// The tile the art shows at ground point (x, y, z): art pixel (x, z − y/2).
    pub fn art_tile_at(&self, x: f32, y: f32, z: f32) -> (i32, i32) {
        ((x / TILE_PX).floor() as i32, ((z - y * 0.5) / TILE_PX).floor() as i32)
    }

    /// The ground point that shows art pixel (ax, ay): z solves z − h(x, z)/2 = ay.
    pub fn ground_for_art(&self, ax: f32, ay: f32) -> [f32; 3] {
        let mut z = ay;
        for _ in 0..4 {
            z = ay + self.ground(ax, z) * 0.5;
        }
        [ax, self.ground(ax, z), z]
    }

    /// The index of a tile graphic in the map's tile set, added if new.
    pub fn intern_tile(&mut self, tile: &Tile) -> Option<u16> {
        let hash = tile_hash(tile);
        if let Some(list) = self.tile_lookup.get(&hash) && let Some(&i) = list.iter().find(|&&i| self.tnt.tiles[i as usize][..] == tile[..]) {
            return Some(i);
        }
        if self.tnt.tiles.len() >= MAX_TILES { return None; }
        let i = self.tnt.tiles.len() as u16;
        self.tnt.tiles.push(Box::new(*tile));
        self.tile_lookup.entry(hash).or_default().push(i);
        Some(i)
    }

    /// Roll back an unsuccessful brush import without cloning the tile graphics.
    pub(super) fn truncate_tiles(&mut self, count: usize) {
        self.tnt.tiles.truncate(count);
        self.tile_lookup.retain(|_, entries| { entries.retain(|&i| (i as usize) < count); !entries.is_empty() });
    }

    /// The feature name table index of `name`, added if new.
    pub fn feature_index(&mut self, name: &str) -> Option<u16> {
        match self.tnt.feature_names.iter().take(FEATURE_VOID as usize).position(|n| n.eq_ignore_ascii_case(name)) {
            Some(i) => Some(i as u16),
            None => {
                if self.tnt.feature_names.len() >= FEATURE_VOID as usize { return None; }
                self.tnt.feature_names.push(name.to_string());
                Some((self.tnt.feature_names.len() - 1) as u16)
            }
        }
    }

    /// The feature whose origin is cell `(x, z)`, if any.
    pub fn feature_name_at(&self, x: u32, z: u32) -> Option<&str> {
        if x >= self.tnt.width || z >= self.tnt.height { return None; }
        let f = *self.tnt.features.get((z * self.tnt.width + x) as usize)?;
        (f < FEATURE_VOID).then(|| self.tnt.feature_names.get(f as usize)).flatten().map(|s| s.as_str())
    }

    /// The feature whose footprint covers cell `(x, z)` (the one starting nearest, should two):
    /// its origin and its name.
    pub fn feature_covering(&self, data: &GameData, x: u32, z: u32) -> Option<((u32, u32), &str)> {
        // (It starts on the cell, or up to its size, 16 cells at most, left of it and above.)
        for oz in (z.saturating_sub(15)..=z).rev() {
            for ox in (x.saturating_sub(15)..=x).rev() {
                if let Some(name) = self.feature_name_at(ox, oz) {
                    let (fx, fz) = MapDoc::footprint(data, name);
                    if x < ox + fx && z < oz + fz {
                        return Some(((ox, oz), name));
                    }
                }
            }
        }
        None
    }

    /// Whether a feature's footprint has its middle nearer than `distance` to world point `at`.
    pub fn feature_near(&self, data: &GameData, at: (f32, f32), distance: f32) -> bool {
        // (A feature's origin is up to its size, 16 cells at most, short of its middle.)
        let reach = (distance / CELL_PX).ceil() as i64 + 16;
        let (cx, cz) = ((at.0 / CELL_PX) as i64, (at.1 / CELL_PX) as i64);
        let (w, h) = (self.tnt.width as i64, self.tnt.height as i64);
        for z in (cz - reach).max(0)..(cz + reach + 1).min(h) {
            for x in (cx - reach).max(0)..(cx + reach + 1).min(w) {
                if let Some(name) = self.feature_name_at(x as u32, z as u32) {
                    let (fx, fz) = MapDoc::footprint(data, name);
                    let (mx, mz) = ((x as f32 + fx as f32 / 2.0) * CELL_PX, (z as f32 + fz as f32 / 2.0) * CELL_PX);
                    if (mx - at.0).powi(2) + (mz - at.1).powi(2) < distance * distance {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Footprint of a named feature (1×1 when unknown).
    pub fn footprint(data: &GameData, name: &str) -> (u32, u32) {
        data.feature(name).map_or((1, 1), |d| (d.footprint_x.max(1), d.footprint_z.max(1)))
    }

    /// Every feature: (origin cell, name).
    pub fn features(&self) -> impl Iterator<Item = ((u32, u32), &str)> {
        let w = self.tnt.width;
        self.tnt.features.iter().enumerate().filter_map(move |(i, &f)| (f < FEATURE_VOID).then(|| self.tnt.feature_names.get(f as usize)).flatten().map(|n| ((i as u32 % w, i as u32 / w), n.as_str())))
    }

    /// Places `name` with its footprint's top-left cell at `origin`. False when the footprint
    /// leaves the map, or covers another feature or impassable ground (see [`MapDoc::taken`]).
    pub fn place_feature(&mut self, data: &GameData, name: &str, origin: (u32, u32)) -> bool {
        let (fx, fz) = MapDoc::footprint(data, name);
        if origin.0 >= self.tnt.width || origin.1 >= self.tnt.height
            || fx > self.tnt.width - origin.0 || fz > self.tnt.height - origin.1
            || self.taken(data, origin, (fx, fz)) {
            return false;
        }
        let Some(index) = self.feature_index(name) else { return false; };
        let i = (origin.1 * self.tnt.width + origin.0) as usize;
        self.tnt.features[i] = index;
        true
    }

    /// Whether a footprint `size` cells at `origin` would cover a feature already there (its
    /// own footprint, not only the cell it starts on) or impassable ground (TA's void cells,
    /// which the editor keeps as they are).
    pub fn taken(&self, data: &GameData, origin: (u32, u32), size: (u32, u32)) -> bool {
        let (w, h) = (self.tnt.width, self.tnt.height);
        let (x1, z1) = ((origin.0 + size.0).min(w), (origin.1 + size.1).min(h));
        // (A feature starting up to its own size to the left or above may reach in.)
        const REACH: u32 = 16;
        for z in origin.1.saturating_sub(REACH)..z1 {
            for x in origin.0.saturating_sub(REACH)..x1 {
                let cell = self.tnt.features[(z * w + x) as usize];
                let inside = x >= origin.0 && z >= origin.1;
                if inside && (cell == FEATURE_VOID || cell == FEATURE_COVERED) {
                    return true;
                }
                if cell < FEATURE_VOID && let Some(name) = self.tnt.feature_names.get(cell as usize) {
                    let (fx, fz) = MapDoc::footprint(data, name);
                    if x + fx > origin.0 && z + fz > origin.1 {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Whether cell `(x, z)` is impassable ground (a void cell).
    pub fn void_at(&self, x: u32, z: u32) -> bool {
        x < self.tnt.width && z < self.tnt.height && self.tnt.features[(z * self.tnt.width + x) as usize] == FEATURE_VOID
    }

    /// Removes the feature starting at `origin`, with the covered-cell marks of its footprint
    /// (some of TA's maps have them).
    pub fn remove_feature(&mut self, data: &GameData, origin: (u32, u32)) -> bool {
        let w = self.tnt.width;
        let Some(name) = self.feature_name_at(origin.0, origin.1).map(str::to_string) else { return false };
        let (fx, fz) = MapDoc::footprint(data, &name);
        for z in origin.1..(origin.1 + fz).min(self.tnt.height) {
            for x in origin.0..(origin.0 + fx).min(w) {
                let cell = &mut self.tnt.features[(z * w + x) as usize];
                if (x, z) == origin || *cell == FEATURE_COVERED {
                    *cell = FEATURE_NONE;
                }
            }
        }
        true
    }

    /// The map as it is written: unused tiles and feature names dropped (indices renumbered)
    /// and a fresh minimap from the art.
    pub fn compacted(&self, palette: &ta_formats::image::Palette) -> TntMap {
        let t = &self.tnt;
        // Every u16 value is a valid graphic tile index, including 65535.
        let mut tile_new = vec![None; t.tiles.len()];
        let mut tiles = Vec::new();
        let mut tile_map = Vec::with_capacity(t.tile_map.len());
        for &i in &t.tile_map {
            let i = (i as usize).min(t.tiles.len() - 1);
            let index = *tile_new[i].get_or_insert_with(|| {
                let index = tiles.len() as u16;
                tiles.push(t.tiles[i].clone());
                index
            });
            tile_map.push(index);
        }
        let mut name_new = vec![u16::MAX; t.feature_names.len()];
        let mut names = Vec::new();
        let features = t
            .features
            .iter()
            .map(|&f| match (f < FEATURE_VOID).then(|| t.feature_names.get(f as usize)).flatten() {
                Some(n) => {
                    if name_new[f as usize] == u16::MAX {
                        name_new[f as usize] = names.len() as u16;
                        names.push(n.clone());
                    }
                    name_new[f as usize]
                }
                None => f,
            })
            .collect();
        let mut out = TntMap {
            width: t.width,
            height: t.height,
            sea_level: t.sea_level,
            tiles,
            tile_map,
            heights: t.heights.clone(),
            features,
            feature_names: names,
            minimap: Default::default(),
            // (A converted map's cliffs stay where they were, whatever was edited.)
            flags: t.flags.clone(),
        };
        out.minimap = out.build_minimap(palette);
        out
    }

    /// The OTA: the one the map came from with the edited keys replaced, or TA's usual set.
    pub fn to_ota(&self) -> TdfDocument {
        let p = &self.props;
        let mut doc = self.ota.clone().unwrap_or_default();
        if doc.section("GlobalHeader").is_none() {
            doc.sections.insert(0, standard_header());
        }
        let Some(header) = doc.sections.iter_mut().find(|s| s.name.eq_ignore_ascii_case("GlobalHeader")) else { return doc };
        let num = |v: f32| if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v}") };
        header.set("missionname", p.name.trim());
        header.set("missiondescription", p.description.trim());
        header.set("planet", p.planet.trim());
        header.set("tidalstrength", num(p.tidal));
        header.set("minwindspeed", num(p.min_wind));
        header.set("maxwindspeed", num(p.max_wind.max(p.min_wind)));
        header.set("gravity", num(p.gravity));
        let n = self.starts.len();
        header.set("numplayers", if n >= 2 { (2..=n).map(|k| k.to_string()).collect::<Vec<_>>().join(", ") } else { n.max(1).to_string() });
        let size = |px: f32| ((px / 512.0).round() as u32).max(1);
        header.set("size", format!("{} x {}", size(self.width_px()), size(self.height_px())));
        let lava = p.planet.to_ascii_lowercase().contains("lava");
        if lava || header.get("lavaworld").is_some() {
            header.set("lavaworld", if lava { "1" } else { "0" });
        }
        // Acid planets' seas do damage (TA's acid maps: 10 a second), others' don't.
        let acid = p.planet.to_ascii_lowercase().contains("acid");
        if acid {
            header.set("waterdoesdamage", "1");
            if header.get("waterdamage").is_none_or(|v| v.trim().parse::<f32>().is_ok_and(|d| d <= 0.0)) {
                header.set("waterdamage", "10");
            }
        } else if header.get("waterdoesdamage").is_some() {
            header.set("waterdoesdamage", "0");
        }
        if !header.sections.iter().any(|s| s.name.to_ascii_lowercase().starts_with("schema")) {
            header.sections.push(standard_schema(0));
        }
        let schemas = header.sections.iter().filter(|s| s.name.to_ascii_lowercase().starts_with("schema")).count();
        header.set("SCHEMACOUNT", schemas.to_string());
        for schema in header.sections.iter_mut().filter(|s| s.name.to_ascii_lowercase().starts_with("schema")) {
            schema.set("SurfaceMetal", num(p.surface_metal));
            schema.set("MohoMetal", num(p.moho_metal));
            let specials = schema.section_mut("specials");
            // Start positions are replaced; anything else the schema places is kept.
            specials.sections.retain(|s| !s.get("specialwhat").is_some_and(|w| w.trim().to_ascii_lowercase().starts_with("startpos")));
            let others = std::mem::take(&mut specials.sections);
            for (k, [x, z]) in self.starts.iter().enumerate() {
                let mut s = TdfSection::new(format!("special{k}"));
                s.set("specialwhat", format!("StartPos{}", k + 1));
                s.set("XPos", format!("{}", x.round() as i64));
                s.set("ZPos", format!("{}", z.round() as i64));
                specials.sections.push(s);
            }
            for (k, mut s) in others.into_iter().enumerate() {
                s.name = format!("special{}", self.starts.len() + k);
                specials.sections.push(s);
            }
        }
        doc
    }
}

/// TA's global header keys for a skirmish map (the values `to_ota` does not set).
fn standard_header() -> TdfSection {
    let mut h = TdfSection::new("GlobalHeader");
    for (k, v) in [
        ("missionname", ""),
        ("missiondescription", ""),
        ("planet", ""),
        ("missionhint", ""),
        ("brief", ""),
        ("narration", ""),
        ("glamour", ""),
        ("lineofsight", "0"),
        ("mapping", "0"),
        ("tidalstrength", "20"),
        ("solarstrength", "20"),
        ("killmul", "50"),
        ("timemul", "0"),
        ("minwindspeed", "0"),
        ("maxwindspeed", "2000"),
        ("gravity", "112"),
        ("numplayers", "2"),
        ("size", "1 x 1"),
        ("memory", "32 mb"),
        ("useonlyunits", ""),
        ("DestroyAllUnits", "1"),
        ("AllUnitsKilled", "1"),
        ("SCHEMACOUNT", "1"),
    ] {
        h.set(k, v);
    }
    h
}

/// A skirmish schema with TA's usual starting resources.
fn standard_schema(n: usize) -> TdfSection {
    let mut s = TdfSection::new(format!("Schema {n}"));
    for (k, v) in [
        ("Type", "Network 1"),
        ("aiprofile", "DEFAULT"),
        ("SurfaceMetal", "3"),
        ("MohoMetal", "30"),
        ("HumanMetal", "1000"),
        ("ComputerMetal", "1000"),
        ("HumanEnergy", "1000"),
        ("ComputerEnergy", "1000"),
        ("MeteorWeapon", ""),
        ("MeteorRadius", "0"),
        ("MeteorDensity", "0"),
        ("MeteorDuration", "0"),
        ("MeteorInterval", "0"),
    ] {
        s.set(k, v);
    }
    s.sections.push(TdfSection::new("specials"));
    s
}
