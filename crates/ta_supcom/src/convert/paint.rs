//! The ground painted as Supreme Commander paints it, seen from above: the lower stratum, the
//! eight strata over it by their masks (`clamp(2m - 1, 0, 1)` each, later over earlier), the
//! upper stratum by its own alpha, the albedo decals by theirs, lit by the map's sun over the
//! heightmap's slopes, and under the water tinted by the water's ramp of colours by depth.
//!
//! Colours are sRGB (0 to 1) as the game's textures hold them; nothing here uses a platform's
//! powers, roots or trigonometry ([`super::trig`]), so the picture is the same everywhere.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::land::{Frame, height};
use crate::dds::{self, Image};
use crate::files::Files;
use crate::scmap::ScMap;
use crate::{Error, Result};

/// A stratum: its texture (the mip level that suits the map's scale) and the world units one
/// repeat of it covers.
struct Stratum {
    image: Image,
    scale: f64,
}

/// An albedo decal, ready to paint: its corner, the world-to-texture steps along its two sides.
struct Decal {
    image: Arc<Image>,
    corner: [f64; 2],
    /// (cos θ, sin θ) / scale.x and (-sin θ, cos θ) / scale.z.
    along: [f64; 2],
    across: [f64; 2],
}

/// The world is cut into squares of this many units for finding the decals over a place.
const GRID: f64 = 8.0;
/// Texels a pixel the strata are decoded at: at TA's scale a texture's repeat covers a few dozen
/// pixels, and one texel a pixel averaged its grain away.
const SHARPEN: f64 = 2.0;
/// How far the calibration to the game's own picture of the map may brighten or darken it, and
/// how far it may change one channel against the others.
const GAIN: (f64, f64) = (0.75, 1.6);
const TINT: f64 = 0.12;

pub struct Painter<'a> {
    map: &'a ScMap,
    frame: Frame,
    lower: Option<Stratum>,
    /// The masked strata that have a texture: (their mask channel 0..8, the stratum).
    strata: Vec<(usize, Stratum)>,
    upper: Option<Stratum>,
    masks: [Option<Image>; 2],
    decals: Vec<Decal>,
    /// Decals over each square of the world ([`GRID`]), in the map's order.
    grid: Vec<Vec<u32>>,
    grid_w: usize,
    grid_h: usize,
    sun: [f64; 3],
    sun_colour: [f64; 3],
    ambience: [f64; 3],
    /// The light flat ground takes, by which every place's is divided: the textures' colours
    /// on level ground, shaded by the slopes.
    flat: f64,
    /// The water's surface and abyss elevations, and its ramp of colours by depth.
    water: Option<(f64, f64, Option<Image>)>,
    /// What every colour is multiplied by: the map as the game shows it ([`Painter::calibrate`]).
    gain: [f64; 3],
}

/// Decodes texture `path` at the mip level whose larger side is about `side` texels (none when
/// the game has no such file, or it is no texture).
fn texture(files: &Files, path: &str, side: f64) -> Result<Option<Image>> {
    if path.trim().is_empty() {
        return Ok(None);
    }
    let Some(bytes) = files.read(path)? else {
        log::warn!("{path}: not in the game's files");
        return Ok(None);
    };
    let header = match dds::Header::parse(&bytes) {
        Ok(h) => h,
        Err(e) => {
            log::warn!("{path}: {e}");
            return Ok(None);
        }
    };
    let level = header.level_for(side.max(1.0).ceil() as u32);
    match dds::decode(&bytes, level) {
        Ok(image) => Ok(Some(image)),
        Err(e) => {
            log::warn!("{path}: {e}");
            Ok(None)
        }
    }
}

/// `image`'s colour at texel position (x, y) (texel centres at .5), between texels; repeated
/// (`wrap`) or clamped at the edges.
#[inline]
fn sample(image: &Image, x: f64, y: f64, wrap: bool) -> [f64; 4] {
    let (x, y) = (x - 0.5, y - 0.5);
    let (fx, fy) = (x.floor(), y.floor());
    let (tx, ty) = (x - fx, y - fy);
    let (ix, iy) = (fx as i64, fy as i64);
    let get = |dx: i64, dy: i64| if wrap { image.wrapped(ix + dx, iy + dy) } else { image.clamped(ix + dx, iy + dy) };
    let (a, b, c, d) = (get(0, 0), get(1, 0), get(0, 1), get(1, 1));
    std::array::from_fn(|k| {
        let top = a[k] as f64 + (b[k] as f64 - a[k] as f64) * tx;
        let bottom = c[k] as f64 + (d[k] as f64 - c[k] as f64) * tx;
        (top + (bottom - top) * ty) / 255.0
    })
}

impl Stratum {
    /// Its colour at world position (u, v).
    #[inline]
    fn at(&self, u: f64, v: f64) -> [f64; 4] {
        let (w, h) = (self.image.width as f64, self.image.height as f64);
        sample(&self.image, u / self.scale * w, v / self.scale * h, true)
    }
}

#[inline]
fn lerp3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn normalise(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-9 { [v[0] / l, v[1] / l, v[2] / l] } else { [0.0, 1.0, 0.0] }
}

impl<'a> Painter<'a> {
    /// Gathers what `map` is painted with, from the game's files, for painting at `frame`'s scale.
    pub fn new(map: &'a ScMap, files: &Files, frame: Frame) -> Result<Painter<'a>> {
        let stratum = |layer: &crate::scmap::Layer| -> Result<Option<Stratum>> {
            let scale = if layer.scale.is_finite() && layer.scale > 0.0 { layer.scale as f64 } else { 4.0 };
            // (One repeat covers `scale` world units, `scale * s` pixels: the level that size.)
            Ok(texture(files, &layer.path, scale * frame.s * SHARPEN)?.map(|image| Stratum { image, scale }))
        };
        let albedo = &map.albedo;
        let lower = match albedo.first() { Some(l) => stratum(l)?, None => None };
        let count = if map.four_strata() { 4 } else { 8 };
        let mut strata = Vec::new();
        for (k, layer) in albedo.iter().enumerate().skip(1).take(count) {
            if let Some(s) = stratum(layer)? {
                strata.push((k - 1, s));
            }
        }
        let upper = match albedo.get(9) { Some(l) => stratum(l)?, None => None };
        let mask = |bytes: &[u8]| dds::decode(bytes, 0).ok().filter(|m| m.width > 0 && m.height > 0);
        let masks = [mask(&map.masks[0]), if map.four_strata() { None } else { mask(&map.masks[1]) }];
        // The albedo decals, each texture decoded once at the size it is painted at.
        let mut images: BTreeMap<(String, u32), Arc<Image>> = BTreeMap::new();
        let mut decals = Vec::new();
        for d in &map.decals {
            if d.kind != 1 || d.textures.is_empty() || !(d.scale[0] > 0.0 && d.scale[2] > 0.0) {
                continue;
            }
            let path = d.textures[0].to_ascii_lowercase();
            let side = (d.scale[0].max(d.scale[2]) as f64 * frame.s).ceil().max(1.0) as u32;
            // (Sides rounded up to a power of two: one decode serves decals of about one size.)
            let side = side.next_power_of_two();
            let image = match images.get(&(path.clone(), side)) {
                Some(i) => i.clone(),
                None => {
                    let Some(i) = texture(files, &path, side as f64)? else { continue };
                    let i = Arc::new(i);
                    images.insert((path, side), i.clone());
                    i
                }
            };
            let (s, c) = super::trig::sin_cos(d.rotation[1] as f64);
            let (sx, sz) = (d.scale[0] as f64, d.scale[2] as f64);
            decals.push(Decal { image, corner: [d.position[0] as f64, d.position[2] as f64], along: [c / sx, s / sx], across: [-s / sz, c / sz] });
        }
        let (grid_w, grid_h) = ((map.width as f64 / GRID).ceil() as usize + 1, (map.height as f64 / GRID).ceil() as usize + 1);
        let mut grid = vec![Vec::new(); grid_w * grid_h];
        for (i, d) in decals.iter().enumerate() {
            // Its four corners in the world, and the squares their box covers.
            let (sx, sz) = (1.0 / (d.along[0] * d.along[0] + d.along[1] * d.along[1]).sqrt(), 1.0 / (d.across[0] * d.across[0] + d.across[1] * d.across[1]).sqrt());
            let ax = [d.along[0] * sx * sx, d.along[1] * sx * sx];
            let az = [d.across[0] * sz * sz, d.across[1] * sz * sz];
            let corners = [d.corner, [d.corner[0] + ax[0], d.corner[1] + ax[1]], [d.corner[0] + az[0], d.corner[1] + az[1]], [d.corner[0] + ax[0] + az[0], d.corner[1] + ax[1] + az[1]]];
            let (x0, x1) = corners.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c[0]), b.max(c[0])));
            let (z0, z1) = corners.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(c[1]), b.max(c[1])));
            let cell = |v: f64, n: usize| ((v / GRID).floor().max(0.0) as usize).min(n - 1);
            for gz in cell(z0, grid_h)..=cell(z1, grid_h) {
                for gx in cell(x0, grid_w)..=cell(x1, grid_w) {
                    grid[gz * grid_w + gx].push(i as u32);
                }
            }
        }
        let l = &map.lighting;
        let sun = normalise(l.sun_direction.map(|v| v as f64));
        let sun = if sun[1] > 0.05 { sun } else { normalise([0.4, 0.8, 0.45]) };
        let sane = |c: [f32; 3], d: [f64; 3]| { let c = c.map(|v| v as f64); if c.iter().all(|v| v.is_finite() && *v >= 0.0 && *v < 16.0) && c.iter().any(|&v| v > 0.0) { c } else { d } };
        let sun_colour = sane(l.sun_colour, [1.0, 1.0, 1.0]);
        let ambience = sane(l.ambience, [0.25, 0.25, 0.25]);
        let flat_light = [ambience[0] + sun_colour[0] * sun[1], ambience[1] + sun_colour[1] * sun[1], ambience[2] + sun_colour[2] * sun[1]];
        let flat = (0.2126 * flat_light[0] + 0.7152 * flat_light[1] + 0.0722 * flat_light[2]).max(1e-3);
        let water = match &map.water {
            Some(w) => {
                let ramp = texture(files, &w.ramp, 256.0)?;
                Some((w.surface as f64, (w.abyss as f64).min(w.surface as f64 - 1.0), ramp))
            }
            None => None,
        };
        if lower.is_none() && strata.is_empty() {
            return Err(Error::Missing(format!("the ground's textures ({})", albedo.first().map_or("", |l| l.path.as_str()))));
        }
        let mut painter = Painter { map, frame, lower, strata, upper, masks, decals, grid, grid_w, grid_h, sun, sun_colour, ambience, flat, water, gain: [1.0; 3] };
        painter.calibrate();
        Ok(painter)
    }

    /// The albedo decals, and how many lie over a square of the world on average and at most
    /// (the painting's cost: every pixel tries those over its square).
    pub fn decal_load(&self) -> (usize, f64, usize) {
        let total: usize = self.grid.iter().map(Vec::len).sum();
        (self.decals.len(), total as f64 / self.grid.len().max(1) as f64, self.grid.iter().map(Vec::len).max().unwrap_or(0))
    }

    /// The ground's colour (sRGB, 0 to 1) at world position (u, v), unlit, as its textures and
    /// decals paint it.
    pub fn albedo(&self, u: f64, v: f64) -> [f64; 3] {
        let mut c = match &self.lower {
            Some(s) => { let t = s.at(u, v); [t[0], t[1], t[2]] }
            None => [0.4, 0.4, 0.4],
        };
        if !self.strata.is_empty() {
            let (w, h) = (self.map.width as f64, self.map.height as f64);
            let mut weights = [0.0f64; 8];
            for (m, mask) in self.masks.iter().enumerate() {
                let Some(mask) = mask else { continue };
                let t = sample(mask, u / w * mask.width as f64, v / h * mask.height as f64, false);
                for k in 0..4 {
                    weights[m * 4 + k] = t[k];
                }
            }
            for (k, s) in &self.strata {
                let weight = (2.0 * weights[*k] - 1.0).clamp(0.0, 1.0);
                if weight > 0.0 {
                    let t = s.at(u, v);
                    c = lerp3(c, [t[0], t[1], t[2]], weight);
                }
            }
        }
        if let Some(s) = &self.upper {
            let t = s.at(u, v);
            c = lerp3(c, [t[0], t[1], t[2]], t[3]);
        }
        let (gx, gz) = ((u / GRID).floor(), (v / GRID).floor());
        if gx >= 0.0 && gz >= 0.0 && (gx as usize) < self.grid_w && (gz as usize) < self.grid_h {
            for &i in &self.grid[gz as usize * self.grid_w + gx as usize] {
                let d = &self.decals[i as usize];
                let (dx, dz) = (u - d.corner[0], v - d.corner[1]);
                let a = dx * d.along[0] + dz * d.along[1];
                let b = dx * d.across[0] + dz * d.across[1];
                if (0.0..1.0).contains(&a) && (0.0..1.0).contains(&b) {
                    let t = sample(&d.image, a * d.image.width as f64, b * d.image.height as f64, false);
                    c = lerp3(c, [t[0], t[1], t[2]], t[3]);
                }
            }
        }
        c
    }

    /// The ground's colour (sRGB, 0 to 1) at world position (u, v): painted, lit, and under the
    /// water tinted by its depth.
    pub fn colour(&self, u: f64, v: f64) -> [f32; 3] {
        let albedo = self.albedo(u, v);
        // The slope from the heights a cell apart (a world unit at the game's own scale).
        let d = self.frame.cell().max(1.0);
        let m = self.map;
        let (hx0, hx1) = (height(m, u - d, v), height(m, u + d, v));
        let (hz0, hz1) = (height(m, u, v - d), height(m, u, v + d));
        let n = normalise([-(hx1 - hx0) / (2.0 * d), 1.0, -(hz1 - hz0) / (2.0 * d)]);
        let ndl = (n[0] * self.sun[0] + n[1] * self.sun[1] + n[2] * self.sun[2]).max(0.0);
        let mut c: [f64; 3] = std::array::from_fn(|k| albedo[k] * (self.ambience[k] + self.sun_colour[k] * ndl) / self.flat);
        if let Some((surface, abyss, ramp)) = &self.water {
            let h = height(m, u, v);
            if h < *surface {
                let t = ((surface - h) / (surface - abyss)).clamp(0.0, 1.0);
                let tint = match ramp {
                    Some(r) => sample(r, t * (r.width as f64 - 1.0) + 0.5, r.height as f64 * 0.5, false),
                    None => {
                        let w = m.water.as_ref().map_or([0.0, 0.1, 0.2], |w| w.colour);
                        [w[0] as f64, w[1] as f64, w[2] as f64, 0.15 + 0.7 * t]
                    }
                };
                c = lerp3(c, [tint[0], tint[1], tint[2]], tint[3]);
            }
        }
        std::array::from_fn(|k| (c[k] * self.gain[k]).clamp(0.0, 1.0) as f32)
    }

    /// Matches the painting's colours on land to the game's own picture of the map (its preview,
    /// which the game drew with the map's light): the mean of each channel over the land, sampled
    /// on a grid, scaled to the picture's, within [`GAIN`] and [`TINT`]. A map whose picture is
    /// missing or blank keeps its colours.
    fn calibrate(&mut self) {
        let Ok(preview) = dds::decode(&self.map.preview, 0) else { return };
        if preview.width < 16 || preview.height < 16 {
            return;
        }
        let m = self.map;
        let (w, h) = (m.width as f64, m.height as f64);
        let surface = m.water.as_ref().map_or(f64::MIN, |w| w.surface as f64 + 1.0);
        let (mut ours, mut theirs, mut n) = ([0.0f64; 3], [0.0f64; 3], 0usize);
        const N: usize = 48;
        for j in 0..N {
            for i in 0..N {
                let (u, v) = ((i as f64 + 0.5) / N as f64 * w, (j as f64 + 0.5) / N as f64 * h);
                if height(m, u, v) < surface {
                    continue;
                }
                // The picture's pixel there, and ours over the ground it covers.
                let p = sample(&preview, u / w * preview.width as f64, v / h * preview.height as f64, false);
                let span = w / preview.width as f64;
                let mut c = [0.0f64; 3];
                for (du, dv) in [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)] {
                    let q = self.colour(u + du * span, v + dv * span);
                    for k in 0..3 {
                        c[k] += q[k] as f64 / 4.0;
                    }
                }
                for k in 0..3 {
                    ours[k] += c[k];
                    theirs[k] += p[k];
                }
                n += 1;
            }
        }
        // (Too little land to tell, or a blank picture.)
        if n < N * N / 20 || theirs.iter().all(|&t| t < 0.02 * n as f64) {
            return;
        }
        let gain: [f64; 3] = std::array::from_fn(|k| if ours[k] > 1e-6 { theirs[k] / ours[k] } else { 1.0 });
        let mean = (gain[0] + gain[1] + gain[2]) / 3.0;
        let level = mean.clamp(GAIN.0, GAIN.1);
        self.gain = gain.map(|g| level * (g / mean).clamp(1.0 - TINT, 1.0 + TINT));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scmap::{Layer, Lighting};

    /// A texture of one colour (uncompressed), `side` texels.
    fn plain(side: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut b = crate::dds::tests::header(side, side, 1, None);
        for _ in 0..side * side {
            b.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        }
        b
    }

    #[test]
    fn strata_paint_by_their_masks_and_decals_over_them() {
        let dir = crate::tests::Temp::new("paint");
        let archive = dir.0.join("env.scd");
        let (red, green, blue) = (plain(4, [200, 0, 0, 255]), plain(4, [0, 200, 0, 255]), plain(4, [0, 0, 200, 255]));
        std::fs::write(&archive, crate::scd::tests::zip(&[("env/x/red.dds", &red, false), ("env/x/green.dds", &green, false), ("env/x/blue.dds", &blue, false)])).unwrap();
        let files = Files::of(Vec::new(), vec![crate::scd::Scd::open(&archive).unwrap()]);
        // A 32 x 32 map: red under everything, green where mask 0's red channel is full (its
        // west half), a blue decal 4 units square at (20, 20).
        let (w, h) = (32u32, 32u32);
        let mut mask = crate::dds::tests::header(16, 16, 1, None);
        for _y in 0..16 {
            for x in 0..16 {
                let r = if x < 8 { 255 } else { 0 };
                mask.extend_from_slice(&[0, 0, r, 0]);
            }
        }
        let mut albedo = vec![Layer { path: "/env/x/red.dds".into(), scale: 4.0 }, Layer { path: "/env/x/green.dds".into(), scale: 4.0 }];
        albedo.resize(10, Layer::default());
        let map = ScMap {
            width: w,
            height: h,
            height_scale: 1.0 / 128.0,
            heights: vec![1280; ((w + 1) * (h + 1)) as usize],
            shader: "TTerrainXP".into(),
            albedo,
            masks: [mask, Vec::new()],
            decals: vec![crate::scmap::Decal { kind: 1, textures: vec!["/env/x/blue.dds".into()], scale: [4.0, 4.0, 4.0], position: [20.0, 10.0, 20.0], rotation: [0.0; 3] }],
            lighting: Lighting { multiplier: 1.0, sun_direction: [0.0, 1.0, 0.0], ambience: [0.2; 3], sun_colour: [1.0; 3], ..Default::default() },
            ..Default::default()
        };
        let frame = Frame::of([0.0, 0.0, 32.0, 32.0]);
        let p = Painter::new(&map, &files, frame).unwrap();
        let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 0.01);
        let g = 200.0 / 255.0;
        assert!(near(p.colour(4.0, 4.0), [0.0, g, 0.0]), "{:?}", p.colour(4.0, 4.0));
        assert!(near(p.colour(28.0, 4.0), [g, 0.0, 0.0]), "{:?}", p.colour(28.0, 4.0));
        assert!(near(p.colour(22.0, 22.0), [0.0, 0.0, g]), "{:?}", p.colour(22.0, 22.0));
        assert!(near(p.colour(25.0, 25.0), [g, 0.0, 0.0]), "past the decal: {:?}", p.colour(25.0, 25.0));
    }
}
