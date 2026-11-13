//! Supreme Commander's map files (`.scmap`): the land and everything painted and placed on it.
//! Forged Alliance writes version 60 (its own maps and the original game's, which it re-saved);
//! the original game and maps made with its editor are version 56, which lacks two of 60's
//! blocks. Laid out as the files are (docs/SUPCOM_MAPS.md has the table), little-endian:
//!
//! the header and an embedded preview picture; the heightmap (u16, one more than the map's
//! size each way, at a height scale, 1/128); the lighting; the water (its elevations and the
//! ramp of colours its depth takes); the wave generators; the minimap's colours; the ten
//! albedo layers (the lower stratum, eight masked strata, the upper) and nine normal layers,
//! each with its scale; the decals; the terrain's normal map; the two stratum masks; the
//! water map; the terrain types; (60) the sky; and the props.
//!
//! What a converter does not use is skipped, but the whole file is walked, to its last byte.

use crate::{Error, Result};

/// The most entries of a list a map may claim (a map's largest is 47,000 props).
const MAX_ITEMS: usize = 4_000_000;
/// The longest string read.
const MAX_STRING: usize = 4096;
/// The largest map side read (the game's largest is 4096).
const MAX_SIDE: u32 = 8192;

/// A texture layer: its path and how many world units one repeat of it covers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layer {
    pub path: String,
    pub scale: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lighting {
    pub multiplier: f32,
    /// Towards the sun.
    pub sun_direction: [f32; 3],
    pub ambience: [f32; 3],
    pub sun_colour: [f32; 3],
    pub shadow_fill: [f32; 3],
    pub bloom: f32,
    pub fog_colour: [f32; 3],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Water {
    /// The surface's elevation and the deep and abyss ones (world units).
    pub surface: f32,
    pub deep: f32,
    pub abyss: f32,
    pub colour: [f32; 3],
    /// The ramp of colours by depth (`/textures/engine/waterramp.dds`).
    pub ramp: String,
}

/// A decal: a texture laid on the ground. Its corner is at `position`, it reaches `scale.x` along
/// its own x and `scale.z` along its own z, turned `rotation.y` (radians) about the vertical.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decal {
    /// 1 albedo, 2 normals, 4 water albedo, 6 glow, 8 glow mask.
    pub kind: i32,
    pub textures: Vec<String>,
    pub scale: [f32; 3],
    pub position: [f32; 3],
    pub rotation: [f32; 3],
}

/// A prop: a blueprint (`/env/evergreen/props/trees/pine06_s2_prop.bp`) placed on the map.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prop {
    pub blueprint: String,
    pub position: [f32; 3],
    /// Its own x axis (`atan2(x[2], x[0])` its heading).
    pub x_axis: [f32; 3],
    pub scale: [f32; 3],
}

#[derive(Clone, Debug, Default)]
pub struct ScMap {
    /// 56 or 60.
    pub version: i32,
    /// In world units ("ogrids"; 256 a "5 km" map).
    pub width: u32,
    pub height: u32,
    /// The embedded preview picture (DDS, 256²).
    pub preview: Vec<u8>,
    pub height_scale: f32,
    /// `(width + 1) × (height + 1)` raw heights, row by row (z).
    pub heights: Vec<u16>,
    pub shader: String,
    pub lighting: Lighting,
    pub water: Option<Water>,
    /// The lower stratum, the eight masked strata, the upper stratum.
    pub albedo: Vec<Layer>,
    /// The lower stratum's and the eight strata's normals.
    pub normals: Vec<Layer>,
    pub decals: Vec<Decal>,
    /// The terrain's normal map (DDS): one, or tiles in reading order, each `normal_size`.
    pub normal_maps: Vec<Vec<u8>>,
    pub normal_size: (u32, u32),
    /// The masks of strata 1-4 and 5-8 (DDS, half the map's size each way).
    pub masks: [Vec<u8>; 2],
    /// Depth and land mask (DDS, half size).
    pub water_map: Vec<u8>,
    /// One per world unit, row by row: what the ground is (`/lua/TerrainTypes.lua`'s codes).
    pub terrain_types: Vec<u8>,
    pub props: Vec<Prop>,
}

impl ScMap {
    /// The raw height at a heightmap vertex (clamped to the map).
    #[inline]
    pub fn raw(&self, x: i64, z: i64) -> u16 {
        let (w, h) = (self.width as i64 + 1, self.height as i64 + 1);
        self.heights[(z.clamp(0, h - 1) * w + x.clamp(0, w - 1)) as usize]
    }

    /// The height (world units) at a heightmap vertex.
    #[inline]
    pub fn height_at(&self, x: i64, z: i64) -> f32 {
        self.raw(x, z) as f32 * self.height_scale
    }

    /// Whether the map uses the original game's terrain shader, which reads only the first
    /// mask (its second is a copy) and four strata.
    pub fn four_strata(&self) -> bool {
        !self.shader.to_ascii_lowercase().contains("xp")
    }

    /// The first part of a map file only: its size and preview picture, for a list of maps.
    pub fn read_preview(path: &std::path::Path) -> Result<(u32, u32, Vec<u8>)> {
        use std::io::Read;
        let mut f = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;
        let mut head = vec![0u8; 34];
        f.read_exact(&mut head).map_err(|e| Error::io(path, e))?;
        let mut r = Reader { b: &head, i: 0 };
        r.header()?;
        let len = r.i32()?;
        if !(0..=64 << 20).contains(&len) {
            return Err(Error::Format("scmap: bad preview".into()));
        }
        let mut rest = vec![0u8; len as usize + 16];
        f.read_exact(&mut rest).map_err(|e| Error::io(path, e))?;
        let preview = rest[..len as usize].to_vec();
        let mut r = Reader { b: &rest[len as usize..], i: 0 };
        let _version = r.i32()?;
        let (w, h) = (r.i32()?, r.i32()?);
        if w <= 0 || h <= 0 || w as u32 > MAX_SIDE || h as u32 > MAX_SIDE {
            return Err(Error::Format(format!("scmap: bad size {w}x{h}")));
        }
        Ok((w as u32, h as u32, preview))
    }

    pub fn parse(bytes: &[u8]) -> Result<ScMap> {
        let mut r = Reader { b: bytes, i: 0 };
        let mut m = ScMap::default();
        r.header()?;
        m.preview = r.blob()?.to_vec();
        m.version = r.i32()?;
        if m.version != 56 && m.version != 60 {
            return Err(Error::Format(format!("scmap: version {} (only 56 and 60 are read)", m.version)));
        }
        let (w, h) = (r.i32()?, r.i32()?);
        if w <= 0 || h <= 0 || w as u32 > MAX_SIDE || h as u32 > MAX_SIDE {
            return Err(Error::Format(format!("scmap: bad size {w}x{h}")));
        }
        (m.width, m.height) = (w as u32, h as u32);
        m.height_scale = r.f32()?;
        if !(m.height_scale.is_finite() && m.height_scale > 0.0) {
            m.height_scale = 1.0 / 128.0;
        }
        let vertices = (w as usize + 1) * (h as usize + 1);
        let raw = r.bytes(vertices * 2)?;
        m.heights = raw.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
        r.u8()?;
        m.shader = r.cstr()?;
        let _background = r.cstr()?;
        let _sky = r.cstr()?;
        for _ in 0..r.count()? {
            r.cstr()?;
            r.cstr()?;
        }
        m.lighting.multiplier = r.f32()?;
        m.lighting.sun_direction = r.vec3()?;
        m.lighting.ambience = r.vec3()?;
        m.lighting.sun_colour = r.vec3()?;
        m.lighting.shadow_fill = r.vec3()?;
        r.bytes(16)?; // specular (garbage in many maps)
        m.lighting.bloom = r.f32()?;
        m.lighting.fog_colour = r.vec3()?;
        r.bytes(8)?; // fog start, end
        let has_water = r.u8()? != 0;
        let (surface, deep, abyss) = (r.f32()?, r.f32()?, r.f32()?);
        let colour = r.vec3()?;
        r.bytes(80 - 12)?;
        let _cubemap = r.cstr()?;
        let ramp = r.cstr()?;
        if has_water {
            m.water = Some(Water { surface, deep, abyss, colour, ramp });
        }
        r.bytes(16)?; // wave textures' repeat rates
        for _ in 0..4 {
            r.bytes(8)?;
            r.cstr()?;
        }
        for _ in 0..r.count()? {
            r.cstr()?;
            r.cstr()?;
            r.bytes(12 + 4 + 12 + 40)?;
        }
        r.bytes(24)?; // the minimap's colours
        if m.version > 56 {
            r.f32()?;
        }
        for _ in 0..10 {
            m.albedo.push(Layer { path: r.cstr()?, scale: r.f32()? });
        }
        for _ in 0..9 {
            m.normals.push(Layer { path: r.cstr()?, scale: r.f32()? });
        }
        r.bytes(8)?; // next decal and group ids
        for _ in 0..r.count()? {
            let _id = r.i32()?;
            let kind = r.i32()?;
            let n = r.count()?;
            let mut textures = Vec::with_capacity(n.min(4));
            for _ in 0..n {
                textures.push(r.lstr()?);
            }
            let (scale, position, rotation) = (r.vec3()?, r.vec3()?, r.vec3()?);
            r.bytes(12)?; // cut-off LODs, army
            m.decals.push(Decal { kind, textures, scale, position, rotation });
        }
        for _ in 0..r.count()? {
            r.i32()?;
            r.cstr()?;
            let n = r.count()?;
            r.bytes(n * 4)?;
        }
        let (nw, nh) = (r.i32()?, r.i32()?);
        m.normal_size = (nw.max(0) as u32, nh.max(0) as u32);
        for _ in 0..r.count()? {
            m.normal_maps.push(r.blob()?.to_vec());
        }
        m.masks = [r.blob()?.to_vec(), r.blob()?.to_vec()];
        r.i32()?;
        m.water_map = r.blob()?.to_vec();
        let half = (w as usize / 2) * (h as usize / 2);
        r.bytes(half * 3)?; // foam, flatness, depth bias
        m.terrain_types = r.bytes(w as usize * h as usize)?.to_vec();
        if m.version > 56 {
            r.bytes(12 + 12 + 8 + 4 + 12 + 12 + 4)?;
            r.cstr()?;
            r.cstr()?;
            let planets = r.count()?;
            r.bytes(planets * 40)?;
            r.bytes(3 + 4 + 12)?;
            r.cstr()?;
            let layers = r.count()?;
            r.bytes(layers * 20 + 4)?;
        }
        for _ in 0..r.count()? {
            let blueprint = r.cstr()?;
            let position = r.vec3()?;
            let x_axis = r.vec3()?;
            r.bytes(24)?; // the y and z axes
            let scale = r.vec3()?;
            m.props.push(Prop { blueprint, position, x_axis, scale });
        }
        if r.i != bytes.len() {
            return Err(Error::Format(format!("scmap: {} bytes past the props", bytes.len() - r.i)));
        }
        Ok(m)
    }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.i.checked_add(n).filter(|&e| e <= self.b.len()).ok_or_else(|| Error::Format(format!("scmap: ends early (at byte {})", self.i)))?;
        let out = &self.b[self.i..end];
        self.i = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    fn i32(&mut self) -> Result<i32> {
        let b = self.bytes(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f32(&mut self) -> Result<f32> {
        let b = self.bytes(4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn vec3(&mut self) -> Result<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }

    /// A count of entries to come.
    fn count(&mut self) -> Result<usize> {
        let n = self.i32()?;
        if !(0..=MAX_ITEMS as i32).contains(&n) {
            return Err(Error::Format(format!("scmap: a list of {n}")));
        }
        Ok(n as usize)
    }

    /// A NUL-ended string.
    fn cstr(&mut self) -> Result<String> {
        let rest = &self.b[self.i.min(self.b.len())..];
        let len = rest.iter().take(MAX_STRING + 1).position(|&c| c == 0).ok_or_else(|| Error::Format("scmap: a string without its end".into()))?;
        let s = String::from_utf8_lossy(&rest[..len]).into_owned();
        self.i += len + 1;
        Ok(s)
    }

    /// A string after its length.
    fn lstr(&mut self) -> Result<String> {
        let n = self.i32()?;
        if !(0..=MAX_STRING as i32).contains(&n) {
            return Err(Error::Format("scmap: a string too long".into()));
        }
        Ok(String::from_utf8_lossy(self.bytes(n as usize)?).into_owned())
    }

    /// Bytes after their length (an embedded picture).
    fn blob(&mut self) -> Result<&'a [u8]> {
        let n = self.i32()?;
        if n < 0 {
            return Err(Error::Format("scmap: a negative length".into()));
        }
        self.bytes(n as usize)
    }

    /// The header: "Map\x1a", its major version, a constant, its size as floats, two zeros.
    fn header(&mut self) -> Result<()> {
        if self.bytes(4)? != b"Map\x1a" {
            return Err(Error::Format("not a Supreme Commander map".into()));
        }
        self.bytes(4 + 4 + 4 + 8 + 4 + 2)?;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A map file of `m` (as the game writes one), for the tests: every block there, the ones
    /// the reader skips filled with nothing in particular.
    pub(crate) fn write(m: &ScMap) -> Vec<u8> {
        let mut o = Vec::new();
        let i32 = |o: &mut Vec<u8>, v: i32| o.extend_from_slice(&v.to_le_bytes());
        let f32 = |o: &mut Vec<u8>, v: f32| o.extend_from_slice(&v.to_le_bytes());
        let cstr = |o: &mut Vec<u8>, s: &str| {
            o.extend_from_slice(s.as_bytes());
            o.push(0);
        };
        let blob = |o: &mut Vec<u8>, b: &[u8]| {
            o.extend_from_slice(&(b.len() as i32).to_le_bytes());
            o.extend_from_slice(b);
        };
        o.extend_from_slice(b"Map\x1a");
        i32(&mut o, 2);
        o.extend_from_slice(&0xBEEF_FEEDu32.to_le_bytes());
        i32(&mut o, 2);
        f32(&mut o, m.width as f32);
        f32(&mut o, m.height as f32);
        i32(&mut o, 0);
        o.extend_from_slice(&0i16.to_le_bytes());
        blob(&mut o, &m.preview);
        i32(&mut o, m.version);
        i32(&mut o, m.width as i32);
        i32(&mut o, m.height as i32);
        f32(&mut o, m.height_scale);
        for &h in &m.heights {
            o.extend_from_slice(&h.to_le_bytes());
        }
        o.push(0);
        for s in [m.shader.as_str(), "/textures/environment/defaultbackground.dds", "/textures/environment/defaultskycube.dds"] {
            cstr(&mut o, s);
        }
        i32(&mut o, 1);
        cstr(&mut o, "<default>");
        cstr(&mut o, "/textures/environment/defaultenvcube.dds");
        let l = &m.lighting;
        f32(&mut o, l.multiplier);
        for v in [l.sun_direction, l.ambience, l.sun_colour, l.shadow_fill] {
            v.iter().for_each(|&x| f32(&mut o, x));
        }
        o.extend_from_slice(&[0; 16]);
        f32(&mut o, l.bloom);
        l.fog_colour.iter().for_each(|&x| f32(&mut o, x));
        o.extend_from_slice(&[0; 8]);
        let water = m.water.clone().unwrap_or(Water { surface: -10000.0, deep: -10000.0, abyss: -10000.0, ..Default::default() });
        o.push(m.water.is_some() as u8);
        for v in [water.surface, water.deep, water.abyss] {
            f32(&mut o, v);
        }
        water.colour.iter().for_each(|&x| f32(&mut o, x));
        o.extend_from_slice(&[0; 68]);
        cstr(&mut o, "/textures/engine/waterCubemap.dds");
        cstr(&mut o, &water.ramp);
        o.extend_from_slice(&[0; 16]);
        for _ in 0..4 {
            o.extend_from_slice(&[0; 8]);
            cstr(&mut o, "/textures/engine/waves.dds");
        }
        i32(&mut o, 1);
        cstr(&mut o, "/textures/engine/waves.dds");
        cstr(&mut o, "");
        o.extend_from_slice(&[0; 68]);
        o.extend_from_slice(&[0; 24]);
        if m.version > 56 {
            f32(&mut o, 0.0);
        }
        for i in 0..10 {
            let layer = m.albedo.get(i).cloned().unwrap_or_default();
            cstr(&mut o, &layer.path);
            f32(&mut o, layer.scale);
        }
        for i in 0..9 {
            let layer = m.normals.get(i).cloned().unwrap_or_default();
            cstr(&mut o, &layer.path);
            f32(&mut o, layer.scale);
        }
        i32(&mut o, m.decals.len() as i32 + 1);
        i32(&mut o, 1);
        i32(&mut o, m.decals.len() as i32);
        for (id, d) in m.decals.iter().enumerate() {
            i32(&mut o, id as i32);
            i32(&mut o, d.kind);
            i32(&mut o, d.textures.len() as i32);
            for t in &d.textures {
                i32(&mut o, t.len() as i32);
                o.extend_from_slice(t.as_bytes());
            }
            for v in [d.scale, d.position, d.rotation] {
                v.iter().for_each(|&x| f32(&mut o, x));
            }
            f32(&mut o, 1000.0);
            f32(&mut o, 0.0);
            i32(&mut o, -1);
        }
        i32(&mut o, 1);
        i32(&mut o, 0);
        cstr(&mut o, "group");
        i32(&mut o, 0);
        i32(&mut o, m.normal_size.0 as i32);
        i32(&mut o, m.normal_size.1 as i32);
        i32(&mut o, m.normal_maps.len() as i32);
        for n in &m.normal_maps {
            blob(&mut o, n);
        }
        blob(&mut o, &m.masks[0]);
        blob(&mut o, &m.masks[1]);
        i32(&mut o, 1);
        blob(&mut o, &m.water_map);
        o.extend(std::iter::repeat_n(0u8, (m.width as usize / 2) * (m.height as usize / 2) * 3));
        o.extend_from_slice(&m.terrain_types);
        if m.version > 56 {
            o.extend_from_slice(&[0; 12 + 12 + 8 + 4 + 12 + 12 + 4]);
            cstr(&mut o, "/textures/environment/defaultskybox.dds");
            cstr(&mut o, "");
            i32(&mut o, 1);
            o.extend_from_slice(&[0; 40]);
            o.extend_from_slice(&[0; 3 + 4 + 12]);
            cstr(&mut o, "/textures/environment/cirrus.dds");
            i32(&mut o, 4);
            o.extend_from_slice(&[0; 4 * 20 + 4]);
        }
        i32(&mut o, m.props.len() as i32);
        for p in &m.props {
            cstr(&mut o, &p.blueprint);
            for v in [p.position, p.x_axis, [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], p.scale] {
                v.iter().for_each(|&x| f32(&mut o, x));
            }
        }
        o
    }

    /// A small map with everything in it.
    pub(crate) fn sample(version: i32) -> ScMap {
        let (w, h) = (16u32, 8u32);
        ScMap {
            version,
            width: w,
            height: h,
            preview: crate::dds::tests::header(4, 4, 1, None).into_iter().chain([7u8; 64]).collect(),
            height_scale: 1.0 / 128.0,
            heights: (0..(w + 1) * (h + 1)).map(|i| (i * 37 % 5000) as u16).collect(),
            shader: "TTerrainXP".into(),
            lighting: Lighting { multiplier: 1.5, sun_direction: [0.2, 0.9, 0.4], ambience: [0.1, 0.1, 0.1], sun_colour: [1.2, 1.1, 1.0], shadow_fill: [0.2, 0.2, 0.3], bloom: 0.08, fog_colour: [0.3, 0.3, 0.4] },
            water: Some(Water { surface: 17.5, deep: 15.0, abyss: 2.5, colour: [0.0, 0.1, 0.2], ramp: "/textures/engine/waterramp.dds".into() }),
            albedo: (0..10).map(|i| Layer { path: format!("/env/evergreen/layers/a{i}_albedo.dds"), scale: 4.0 + i as f32 }).collect(),
            normals: (0..9).map(|i| Layer { path: format!("/env/evergreen/layers/a{i}_normals.dds"), scale: 4.0 }).collect(),
            decals: vec![Decal { kind: 1, textures: vec!["/env/common/decals/x.dds".into(), String::new()], scale: [8.0, 8.0, 4.0], position: [3.0, 1.0, 2.0], rotation: [0.0, 0.5, 0.0] }],
            normal_maps: vec![vec![1, 2, 3]],
            normal_size: (w, h),
            masks: [vec![4, 5], vec![6]],
            water_map: vec![8, 9, 10],
            terrain_types: (0..w * h).map(|i| (i % 256) as u8).collect(),
            props: vec![Prop { blueprint: "/env/evergreen/props/trees/pine06_s2_prop.bp".into(), position: [5.5, 10.0, 3.25], x_axis: [1.0, 0.0, 0.0], scale: [1.0; 3] }],
        }
    }

    #[test]
    fn both_versions_read_back_to_their_last_byte() {
        for version in [56, 60] {
            let m = sample(version);
            let bytes = write(&m);
            let got = ScMap::parse(&bytes).unwrap();
            assert_eq!((got.version, got.width, got.height), (version, 16, 8));
            assert_eq!(got.heights, m.heights);
            assert_eq!(got.raw(16, 8), m.heights[8 * 17 + 16]);
            assert_eq!(got.raw(99, -3), m.heights[16], "clamped to the map");
            assert_eq!((got.lighting.multiplier, got.lighting.sun_direction), (1.5, [0.2, 0.9, 0.4]));
            assert_eq!(got.water, m.water);
            assert_eq!(got.albedo, m.albedo);
            assert_eq!(got.normals, m.normals);
            assert_eq!(got.decals, m.decals);
            assert_eq!((got.normal_maps.clone(), got.normal_size), (m.normal_maps.clone(), (16, 8)));
            assert_eq!(got.masks, m.masks);
            assert_eq!(got.water_map, m.water_map);
            assert_eq!(got.terrain_types, m.terrain_types);
            assert_eq!(got.props, m.props);
            assert!(!got.four_strata());
            // A byte more, or less, is a damaged map.
            let mut longer = bytes.clone();
            longer.push(0);
            assert!(ScMap::parse(&longer).is_err());
            assert!(ScMap::parse(&bytes[..bytes.len() - 1]).is_err());
        }
        let mut old = write(&sample(60));
        let at = 34 + sample(60).preview.len();
        old[at..at + 4].copy_from_slice(&53i32.to_le_bytes());
        assert!(ScMap::parse(&old).is_err(), "version 53 is not read");
    }

    #[test]
    fn a_maps_preview_and_size_read_alone() {
        let dir = crate::tests::Temp::new("scmap");
        let path = dir.0.join("x.scmap");
        let m = sample(60);
        std::fs::write(&path, write(&m)).unwrap();
        let (w, h, preview) = ScMap::read_preview(&path).unwrap();
        assert_eq!((w, h, preview), (16, 8, m.preview));
    }
}
