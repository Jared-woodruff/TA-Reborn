//! The pictures the Supreme Commander mods are known by in the launcher's library: each mod's
//! banner (`media/banner.png`, named by its manifest's `[media]`), made from the player's own copy
//! of the game as the mods themselves are (nothing of the game's is shipped):
//! - the factions' ([`factions`]): the game's hall of the factions (its campaign's), the emblems
//!   of the factions the mod holds on medallions of their colours;
//! - the maps' ([`maps`]): the game's pictures of the maps in the pack, turned as cards in a row,
//!   the middle one facing, over the game's loading rays;
//! - the music's ([`music`]): the menu theme's loudest eight seconds as a row of bars over a
//!   painting of the game's;
//! - the menus' ([`menus`]): the game's logo, and a panel of its menus holding a painting, over
//!   the space its menus show.
//!
//! In whole numbers throughout: the same picture from the same copy on every computer.

use std::path::Path;

use crate::dds::{self, Image};
use crate::files::Files;

/// A banner's size: as large as the launcher keeps one, twice as wide as high (a card's shape;
/// the details' larger one shows a little less of its sides, or of its top and bottom).
pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 640;
/// Where a mod keeps its banner, as its manifest names it.
pub const PATH: &str = "media/banner.png";

/// The manifest's lines naming the banner: its `[media]` table, after the rest.
pub fn manifest_lines() -> String {
    format!("\n[media]\nbanner = \"{PATH}\"\n")
}

/// The factions' hall: the game's campaign's, its three banners (the Cybran's, the UEF's, the
/// Aeon's) hung over its floor.
const HALL: &str = "textures/ui/common/scx_menu/campaign-select/bg.dds";
/// The factions' emblems, in the game's order: each faction's side (as the mod names it), its
/// emblem and its colour.
const EMBLEMS: [(&str, &str, [u8; 3]); 4] = [
    ("UEF", "textures/ui/common/faction_icon-lg/uef_ico.dds", [77, 158, 255]),
    ("AEON", "textures/ui/common/faction_icon-lg/aeon_ico.dds", [128, 255, 89]),
    ("CYBRAN", "textures/ui/common/faction_icon-lg/cybran_ico.dds", [255, 71, 56]),
    ("SERAPHIM", "textures/ui/common/faction_icon-lg/seraphim_ico.dds", [255, 204, 77]),
];

/// Where the game keeps the emblem of the faction of side `side` (as the mod names it: `UEF`,
/// `AEON`, `CYBRAN`, `SERAPHIM`): 60 by 60, its shape in its alpha. (The classic interface's
/// emblems are made of it too: `factions::emblem`.)
pub fn emblem_path(side: &str) -> Option<&'static str> {
    EMBLEMS.iter().find(|(s, _, _)| s.eq_ignore_ascii_case(side.trim())).map(|(_, path, _)| *path)
}

/// The rays the game shows as it loads: behind the maps.
const RAYS: &str = "textures/ui/common/load/background-portal_bmp.dds";
/// The painting behind the music: a fleet over a red sky.
const MUSIC_PAINTING: &str = "textures/ui/common/menus02/background-paint03_bmp.dds";
/// The space the game's menus show, with its planet.
const SPACE: &str = "textures/ui/common/menus02/background_bmp.dds";
/// A panel of the game's menus (the game list's), and the painting shown in it.
const PANEL: &str = "textures/ui/common/scx_menu/gameselect/panel_bmp.dds";
const PANEL_PAINTING: &str = "textures/ui/common/menus02/background-paint04_bmp.dds";
/// The game's logo.
const LOGO: &str = "textures/ui/common/scx_menu/logo/logo.dds";

/// The maps the pack's banner shows first where it has them (by their folders, the game's own):
/// well known, and unlike each other; the middle one first.
pub const SHOWCASE: [&str; 9] = ["scmp_009", "scmp_003", "x1mp_002", "scmp_015", "x1mp_005", "scmp_010", "scmp_020", "x1mp_007", "scmp_005"];
/// The most maps the pack's banner shows.
pub const MOST_MAPS: usize = 9;
/// How many bars the music's banner shows.
pub const BARS: usize = 120;

/// Full: a channel's most, its alpha's whole.
const ONE: u32 = 65535;

/// A picture being made: its texels' red, green, blue and alpha, the colours carried by the alpha
/// (premultiplied), each of 0 to [`ONE`].
#[derive(Clone)]
struct Layer {
    w: u32,
    h: u32,
    px: Vec<[u16; 4]>,
}

impl Layer {
    /// Clear, `w` by `h`.
    fn new(w: u32, h: u32) -> Layer {
        Layer { w, h, px: vec![[0; 4]; w as usize * h as usize] }
    }

    /// `image`, its colours carried by its alpha; or `opaque`, its alpha left out (the game leaves
    /// its pictures' unset).
    fn of(image: &Image, opaque: bool) -> Layer {
        let px = image.rgba.iter().map(|t| {
            let a = if opaque { ONE } else { t[3] as u32 * 257 };
            let c = |v: u8| ((v as u32 * 257 * a + ONE / 2) / ONE) as u16;
            [c(t[0]), c(t[1]), c(t[2]), a as u16]
        }).collect();
        Layer { w: image.width, h: image.height, px }
    }

    /// A dark ground of `colour` darkening downwards, where the game's own picture is missing.
    fn ground(colour: [u8; 3]) -> Layer {
        let mut out = Layer::new(WIDTH, HEIGHT);
        for y in 0..HEIGHT {
            let keep = 1000 - 600 * y / (HEIGHT - 1);
            let c = colour.map(|v| (v as u32 * 257 * keep / 1000) as u16);
            for x in 0..WIDTH {
                out.px[(y * WIDTH + x) as usize] = [c[0], c[1], c[2], ONE as u16];
            }
        }
        out
    }

    fn at(&self, x: u32, y: u32) -> [u16; 4] {
        self.px[(y * self.w + x) as usize]
    }

    /// Made `w` by `h`: each texel the mean of those it covers where made smaller, between the
    /// two nearest its middle where made larger (each side by itself).
    fn resized(&self, w: u32, h: u32) -> Layer {
        let (w, h) = (w.max(1), h.max(1));
        let mut wide = Layer::new(w, self.h);
        if w == self.w {
            wide.px.clone_from(&self.px);
        } else {
            let k = kernel(self.w, w);
            for y in 0..self.h as usize {
                let row = &self.px[y * self.w as usize..(y + 1) * self.w as usize];
                for (x, (first, weights)) in k.iter().enumerate() {
                    wide.px[y * w as usize + x] = mix(weights.iter().enumerate().map(|(n, &wt)| (row[first + n], wt)));
                }
            }
        }
        if h == self.h {
            return wide;
        }
        let mut out = Layer::new(w, h);
        for (y, (first, weights)) in kernel(self.h, h).iter().enumerate() {
            for x in 0..w as usize {
                out.px[y * w as usize + x] = mix(weights.iter().enumerate().map(|(n, &wt)| (wide.px[(first + n) * w as usize + x], wt)));
            }
        }
        out
    }

    /// Made just large enough to cover `w` by `h` whole, what is left over cut away round
    /// `focus` (thousandths of the way across and down).
    fn cover(&self, w: u32, h: u32, focus: (u32, u32)) -> Layer {
        let (sw, sh) = if w as u64 * self.h as u64 >= h as u64 * self.w as u64 {
            (w, ((self.h as u64 * w as u64).div_ceil(self.w.max(1) as u64) as u32).max(h))
        } else {
            (((self.w as u64 * h as u64).div_ceil(self.h.max(1) as u64) as u32).max(w), h)
        };
        let big = self.resized(sw, sh);
        let (x0, y0) = ((sw - w) * focus.0.min(1000) / 1000, (sh - h) * focus.1.min(1000) / 1000);
        let mut out = Layer::new(w, h);
        for y in 0..h {
            let from = ((y + y0) * sw + x0) as usize;
            out.px[(y * w) as usize..((y + 1) * w) as usize].copy_from_slice(&big.px[from..from + w as usize]);
        }
        out
    }

    /// `w` across, as high as keeps its shape.
    fn across(&self, w: u32) -> Layer {
        self.resized(w, (self.h as u64 * w as u64 / self.w.max(1) as u64).max(1) as u32)
    }

    /// Calls `f` on each texel this shares with `top` laid at (x, y), with `top`'s there.
    fn each(&mut self, top: &Layer, x: i32, y: i32, mut f: impl FnMut(&mut [u16; 4], [u16; 4])) {
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = ((x + top.w as i32).min(self.w as i32), (y + top.h as i32).min(self.h as i32));
        for py in y0..y1 {
            for px in x0..x1 {
                let t = top.px[((py - y) as u32 * top.w + (px - x) as u32) as usize];
                f(&mut self.px[(py as u32 * self.w + px as u32) as usize], t);
            }
        }
    }

    /// `top` laid over at (x, y), `share` (of [`ONE`]) of it.
    fn over(&mut self, top: &Layer, x: i32, y: i32, share: u32) {
        self.each(top, x, y, |d, t| {
            let a = t[3] as u32 * share / ONE;
            for c in 0..3 {
                d[c] = (t[c] as u32 * share / ONE + d[c] as u32 * (ONE - a) / ONE).min(ONE) as u16;
            }
            d[3] = (a + d[3] as u32 * (ONE - a) / ONE).min(ONE) as u16;
        });
    }

    /// `top`'s light added at (x, y), as lights add (each lightens what is under it, never past
    /// white).
    fn screen(&mut self, top: &Layer, x: i32, y: i32) {
        self.each(top, x, y, |d, t| {
            for c in 0..3 {
                let (a, b) = (d[c] as u32, t[c] as u32);
                d[c] = (a + b - a * b / ONE) as u16;
            }
        });
    }

    /// Darkened under `top` at (x, y) as much as its alpha says: its shadow.
    fn shade(&mut self, top: &Layer, x: i32, y: i32) {
        self.each(top, x, y, |d, t| {
            let keep = ONE - t[3] as u32;
            for v in &mut d[..3] {
                *v = (*v as u32 * keep / ONE) as u16;
            }
        });
    }

    /// Its colours made `bright` (thousandths) as bright and `saturated` (thousandths) as
    /// strong.
    fn grade(&mut self, bright: u32, saturated: u32) {
        for p in &mut self.px {
            let rgb = [p[0] as i64, p[1] as i64, p[2] as i64];
            let lum = (2126 * rgb[0] + 7152 * rgb[1] + 722 * rgb[2]) / 10_000;
            for c in 0..3 {
                let v = lum + (rgb[c] - lum) * saturated as i64 / 1000;
                p[c] = (v * bright as i64 / 1000).clamp(0, p[3] as i64) as u16;
            }
        }
    }

    /// Darker towards its edges, `k` (thousandths) darker at its corners.
    fn vignette(&mut self, k: u32) {
        let (w, h) = (self.w as i64, self.h as i64);
        for y in 0..h {
            let v = (2 * y + 1 - h) * 1000 / h;
            for x in 0..w {
                let u = (2 * x + 1 - w) * 1000 / w;
                let d = ((u * u * 3 / 4 + v * v) / 1750).min(1000);
                let keep = 1000 - k as i64 * d / 1000;
                for v in &mut self.px[(y * w + x) as usize][..3] {
                    *v = (*v as i64 * keep / 1000) as u16;
                }
            }
        }
    }

    /// Darkened from the row `from` down, `k` (thousandths) dark at the bottom.
    fn fade_down(&mut self, from: u32, k: u32) {
        let span = (self.h - 1).saturating_sub(from).max(1);
        for y in from..self.h {
            let keep = 1000 - k * (y - from) / span;
            for x in 0..self.w {
                for v in &mut self.px[(y * self.w + x) as usize][..3] {
                    *v = (*v as u32 * keep / 1000) as u16;
                }
            }
        }
    }
}

/// Along one side, `from` texels made `to`: each new texel's first old one, and the weights of
/// those it takes (summing to 65536): the mean of those it covers when fewer, the two nearest its
/// middle when more.
pub(crate) fn kernel(from: u32, to: u32) -> Vec<(usize, Vec<u32>)> {
    let (from, to) = (from.max(1) as u64, to.max(1) as u64);
    (0..to)
        .map(|i| {
            if to < from {
                // (It covers [i·from, (i+1)·from) in to-ths of an old texel.)
                let (start, end) = (i * from, (i + 1) * from);
                let first = start / to;
                let mut weights: Vec<u32> = (first..=(end - 1) / to).map(|j| ((end.min((j + 1) * to) - start.max(j * to)) * 65536 / from) as u32).collect();
                let short = 65536 - weights.iter().sum::<u32>();
                let most = (0..weights.len()).max_by_key(|&k| weights[k]).unwrap_or(0);
                weights[most] += short;
                (first as usize, weights)
            } else {
                // (Its middle, in 256ths of an old texel from the first one's middle.)
                let p = ((2 * i + 1) * from * 256 / (2 * to)) as i64 - 128;
                let (j, f) = (p.div_euclid(256), p.rem_euclid(256) as u32);
                let last = from as i64 - 1;
                let (a, b) = (j.clamp(0, last), (j + 1).clamp(0, last));
                if a == b { (a as usize, vec![65536]) } else { (a as usize, vec![(256 - f) * 256, f * 256]) }
            }
        })
        .collect()
}

/// Texels mixed by their weights (of 65536).
fn mix(taps: impl Iterator<Item = ([u16; 4], u32)>) -> [u16; 4] {
    let mut sum = [0u64; 4];
    for (t, wt) in taps {
        for c in 0..4 {
            sum[c] += t[c] as u64 * wt as u64;
        }
    }
    sum.map(|s| ((s + 32_768) >> 16).min(ONE as u64) as u16)
}

/// One box blur of `src` (`w` by `h`) along its rows (`across`) or its columns, `r` texels each
/// way, nothing beyond its edges.
fn box_pass(src: &[u32], w: usize, h: usize, r: usize, across: bool) -> Vec<u32> {
    let (lines, len) = if across { (h, w) } else { (w, h) };
    let at = |line: usize, i: usize| if across { line * w + i } else { i * w + line };
    let span = (2 * r + 1) as u64;
    let mut out = vec![0u32; src.len()];
    for line in 0..lines {
        let mut sum: u64 = (0..=r.min(len - 1)).map(|i| src[at(line, i)] as u64).sum();
        for i in 0..len {
            out[at(line, i)] = ((sum + span / 2) / span) as u32;
            if i + r + 1 < len {
                sum += src[at(line, i + r + 1)] as u64;
            }
            if i >= r {
                sum -= src[at(line, i - r)] as u64;
            }
        }
    }
    out
}

/// `top`'s alpha spread by about `r` texels (three box blurs each way), `strength` (thousandths)
/// of it, in a layer `r` twice over larger each side: the light round it, or its shadow.
fn halo(top: &Layer, r: u32, strength: u32, colour: [u8; 3]) -> (Layer, i32) {
    let pad = 2 * r;
    let (w, h) = (top.w + 2 * pad, top.h + 2 * pad);
    let mut mask = vec![0u32; w as usize * h as usize];
    for y in 0..top.h {
        for x in 0..top.w {
            mask[((y + pad) * w + x + pad) as usize] = top.at(x, y)[3] as u32;
        }
    }
    let step = (r as usize / 2).max(1);
    for _ in 0..3 {
        mask = box_pass(&mask, w as usize, h as usize, step, true);
        mask = box_pass(&mask, w as usize, h as usize, step, false);
    }
    let mut out = Layer::new(w, h);
    for (p, m) in out.px.iter_mut().zip(mask) {
        let a = (m as u64 * strength as u64 / 1000).min(ONE as u64) as u32;
        let c = |k: usize| (colour[k] as u32 * 257 * a / ONE) as u16;
        *p = [c(0), c(1), c(2), a as u16];
    }
    (out, pad as i32)
}

/// `top` laid at (x, y) with its shadow (`r` soft, `drop` texels below it, `strength`
/// thousandths dark) and, where `light` is given, the light round it.
fn lay(canvas: &mut Layer, top: &Layer, x: i32, y: i32, shadow: (u32, i32, u32), light: Option<(u32, [u8; 3], u32)>) {
    if let Some((r, colour, strength)) = light {
        let (glow, pad) = halo(top, r, strength, colour);
        canvas.screen(&glow, x - pad, y - pad);
    }
    let (r, drop, strength) = shadow;
    let (dark, pad) = halo(top, r, strength, [0, 0, 0]);
    canvas.shade(&dark, x - pad, y - pad + drop);
    canvas.over(top, x, y, ONE);
}

/// A medallion `side` texels across: dark glass lighter at its top, ringed `ring` texels wide in
/// `colour`, its edge smooth.
fn medallion(side: u32, colour: [u8; 3], ring: u32) -> Layer {
    let mut out = Layer::new(side, side);
    // (In 256ths of a texel: its radius, its ring's width and its middle line.)
    let radius = (side as i64 - 2) * 128;
    let width = ring as i64 * 256;
    let middle = radius - width / 2;
    for y in 0..side {
        let shade = 6554 + 5243 * (side - 1 - y) / (side - 1).max(1);
        let glass = [shade * 8 / 10, shade * 9 / 10, shade];
        for x in 0..side {
            let (dx, dy) = (2 * x as i64 + 1 - side as i64, 2 * y as i64 + 1 - side as i64);
            let d = (((dx * dx + dy * dy) as u64) << 14).isqrt() as i64;
            let inside = (radius - d + 128).clamp(0, 256) as u32;
            let rim = (width / 2 + 128 - (d - middle).abs()).clamp(0, 256) as u32 * inside / 256;
            let a = inside * ONE / 256 * 92 / 100;
            let c: [u32; 3] = std::array::from_fn(|k| (glass[k] * (256 - rim) + colour[k] as u32 * 257 * rim) / 256);
            out.px[(y * side + x) as usize] = [(c[0] * a / ONE) as u16, (c[1] * a / ONE) as u16, (c[2] * a / ONE) as u16, a as u16];
        }
    }
    out
}

/// The picture at `path` in the game's files, decoded; `opaque`: its alpha left out.
fn picture(files: &Files, path: &str, opaque: bool) -> Option<Layer> {
    let bytes = files.read(path).ok()??;
    let image = dds::decode(&bytes, 0).ok()?;
    (image.width > 0 && image.height > 0 && image.rgba.len() == image.width as usize * image.height as usize).then(|| Layer::of(&image, opaque))
}

/// The game's picture of the map in `folder` (its map file's, 256 texels square): none where it
/// has none.
pub fn map_preview(folder: &Path) -> Option<Image> {
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(folder).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("scmap"))).collect();
    files.sort();
    let (_, _, bytes) = crate::scmap::ScMap::read_preview(files.first()?).ok()?;
    let image = dds::decode(&bytes, 0).ok()?;
    (image.width > 0 && image.height > 0 && image.rgba.len() == image.width as usize * image.height as usize).then_some(image)
}

/// The order the pack's banner shows its maps in, by their folders (lower case): those of
/// [`SHOWCASE`] first, in its order, then the rest by name. (It shows the first [`MOST_MAPS`]
/// whose pictures there are.)
pub fn map_order(sources: &[String]) -> Vec<String> {
    let mut rest: Vec<String> = sources.iter().map(|s| s.to_ascii_lowercase()).filter(|s| !SHOWCASE.contains(&s.as_str())).collect();
    rest.sort();
    rest.dedup();
    SHOWCASE.iter().map(|s| s.to_string()).filter(|s| sources.iter().any(|o| o.eq_ignore_ascii_case(s))).chain(rest).collect()
}

/// The factions' banner: their hall, darkened to its floor, and the emblems of those the mod
/// holds (`sides`, as it names them: `UEF`, `AEON`, `CYBRAN`, `SERAPHIM`; none named, all four)
/// on medallions of their colours along it, in the game's order. None when the copy has neither.
pub fn factions(files: &Files, sides: &[&str]) -> Option<Vec<u8>> {
    let hall = picture(files, HALL, true);
    let held = |side: &str| sides.is_empty() || sides.iter().any(|s| s.trim().eq_ignore_ascii_case(side));
    let emblems: Vec<(Layer, [u8; 3])> = EMBLEMS.iter().filter(|(side, _, _)| held(side)).filter_map(|(_, path, colour)| picture(files, path, false).map(|e| (e, *colour))).collect();
    if hall.is_none() && emblems.is_empty() {
        return None;
    }
    let mut canvas = hall.map_or_else(|| Layer::ground([16, 20, 26]), |h| h.cover(WIDTH, HEIGHT, (500, 300)));
    canvas.grade(1080, 1100);
    canvas.fade_down(HEIGHT / 2, 820);
    canvas.vignette(450);
    let (side, gap, emblem) = (132i32, 196i32, 92u32);
    let x0 = WIDTH as i32 / 2 - gap * (emblems.len() as i32 - 1) / 2 - side / 2;
    let y = HEIGHT as i32 - side - 92;
    for (i, (picture, colour)) in emblems.iter().enumerate() {
        let x = x0 + i as i32 * gap;
        lay(&mut canvas, &medallion(side as u32, *colour, 3), x, y, (10, 6, 800), Some((26, *colour, 900)));
        let inset = (side - emblem as i32) / 2;
        let picture = picture.resized(emblem, emblem);
        let (glow, pad) = halo(&picture, 10, 800, *colour);
        canvas.screen(&glow, x + inset - pad, y + inset - pad);
        canvas.over(&picture, x + inset, y + inset, ONE);
    }
    Some(png(&canvas))
}

/// A square map picture turned about its upright middle as a card in a row is: `narrow`
/// (thousandths) as wide, its far edge (on the left, `far_left`) `near` (thousandths) as tall,
/// each column about its middle. The card alone (what casts its shadow), and the card with its
/// reflection on the floor beneath each column's foot, fading.
fn card(tile: &Layer, narrow: u32, near: u32, far_left: bool) -> (Layer, Layer) {
    let side = tile.h;
    let w = (tile.w * narrow / 1000).max(2);
    let squeezed = tile.resized(w, side);
    let tall = side + 6 + side / 2;
    let (mut face, mut both) = (Layer::new(w, side), Layer::new(w, tall));
    for x in 0..w {
        let u = (2 * x + 1) * 1000 / (2 * w);
        let far = if far_left { 1000 - u } else { u };
        let hgt = (side * (1000 - (1000 - near) * far / 1000) / 1000).max(1);
        let column: Vec<[u16; 4]> = (0..side).map(|y| squeezed.at(x, y)).collect();
        let (first, weights): (Vec<usize>, Vec<Vec<u32>>) = kernel(side, hgt).into_iter().unzip();
        let top = (side - hgt) / 2;
        for (n, (start, wts)) in first.iter().zip(&weights).enumerate() {
            let t = mix(wts.iter().enumerate().map(|(k, &wt)| (column[start + k], wt)));
            face.px[((top + n as u32) * w + x) as usize] = t;
            both.px[((top + n as u32) * w + x) as usize] = t;
            // (Mirrored below its own bottom edge, fading over half its height.)
            let below = hgt - 1 - n as u32;
            let fade = hgt / 2;
            if below < fade {
                let y = top + hgt + 6 + below;
                let a = 19_661 * (fade - below) / fade;
                if y < tall {
                    both.px[(y * w + x) as usize] = t.map(|v| (v as u32 * a / ONE) as u16);
                }
            }
        }
    }
    (face, both)
}

/// The maps' banner: the pictures of the pack's maps (`previews`, in [`map_order`]'s order, the
/// first the middle one) as cards in a row, the middle one facing, the others turned away
/// further out and darker, over the game's loading rays (from `files`, where given): a pack of
/// fewer maps than the row holds shows them again, round and round (one alone stands by itself).
/// None with no picture.
pub fn maps(previews: &[Image], files: Option<&Files>) -> Option<Vec<u8>> {
    if previews.is_empty() {
        return None;
    }
    let mut canvas = files.and_then(|f| picture(f, RAYS, true)).map_or_else(|| Layer::ground([10, 34, 48]), |r| r.cover(WIDTH, HEIGHT, (500, 500)));
    canvas.grade(550, 800);
    let side = 300u32;
    let top = (HEIGHT / 2 - 20 - side / 2) as i32;
    // (Their places in the row, outermost first so the nearer lie over the farther: 0 the
    // middle, then -1, 1, -2, 2... as the pictures come.)
    let cards = if previews.len() == 1 { 1 } else { MOST_MAPS };
    let places: Vec<i32> = (0..cards as i32).map(|i| if i == 0 { 0 } else if i % 2 == 1 { -(i + 1) / 2 } else { i / 2 }).collect();
    let mut order: Vec<usize> = (0..places.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(places[i].abs()), places[i] > 0));
    for i in order {
        let place = places[i];
        // (Fewer maps than cards: the maps round and round along the row, neighbours unlike.)
        let picture = if previews.len() >= cards { i } else { place.rem_euclid(previews.len() as i32) as usize };
        let mut tile = Layer::of(&previews[picture], true).resized(side, side);
        let (face, both, x) = if place == 0 {
            let (face, both) = card(&tile, 1000, 1000, false);
            (face, both, (WIDTH / 2 - side / 2) as i32)
        } else {
            let away = place.unsigned_abs();
            tile.grade(920u32.saturating_sub(170 * away).max(350), 1000);
            let (face, both) = card(&tile, 500, 800, place < 0);
            let off = (side / 2 + 34 + (away - 1) * 118) as i32;
            let x = if place < 0 { WIDTH as i32 / 2 - off - face.w as i32 } else { WIDTH as i32 / 2 + off };
            (face, both, x)
        };
        let (dark, pad) = halo(&face, 14, 850, [0, 0, 0]);
        canvas.shade(&dark, x - pad, top - pad + 10);
        canvas.over(&both, x, top, ONE);
    }
    canvas.vignette(550);
    Some(png(&canvas))
}

/// How long a stretch of the menu theme the music's banner shows, in seconds: round its loudest
/// moment, as a row of bars a sixth of a second or so each.
pub const STRETCH_SECONDS: usize = 8;

/// The loudness of a track's `pcm` (as its wave bank holds it: `bits` 16, signed, or 8, unsigned;
/// `channels` interleaved, `rate` frames a second) round its loudest second, over
/// [`STRETCH_SECONDS`] (all of it, where shorter), in `bars` even stretches: each one's
/// thousandths of the way from the quietest's to the loudest's.
pub fn loudness(pcm: &[u8], bits: u16, channels: u16, rate: u32, bars: usize) -> Vec<u16> {
    let samples = if bits == 16 { pcm.len() / 2 } else { pcm.len() };
    if samples == 0 || bars == 0 {
        return vec![0; bars];
    }
    let level = |i: usize| -> u64 {
        if bits == 16 { i16::from_le_bytes([pcm[2 * i], pcm[2 * i + 1]]).unsigned_abs() as u64 } else { (pcm[i] as i32 - 128).unsigned_abs() as u64 * 256 }
    };
    let mean = |from: usize, to: usize| -> u64 { if to <= from { 0 } else { (from..to).map(level).sum::<u64>() / (to - from) as u64 } };
    // (Its loudest second, the first where two are as loud; the stretch round it, kept inside
    // the track.)
    let second = (rate as usize * channels.max(1) as usize).max(1);
    let loudest = (0..samples / second).max_by_key(|&s| (mean(s * second, (s + 1) * second), std::cmp::Reverse(s))).unwrap_or(0);
    let span = (STRETCH_SECONDS * second).min(samples);
    let start = (loudest * second + second / 2).saturating_sub(span / 2).min(samples - span);
    let means: Vec<u64> = (0..bars)
        .map(|b| {
            let from = start + span * b / bars;
            mean(from, (start + span * (b + 1) / bars).max(from + 1).min(start + span))
        })
        .collect();
    let (low, high) = (means.iter().copied().min().unwrap_or(0), means.iter().copied().max().unwrap_or(0));
    means.iter().map(|m| ((m - low) * 1000 / (high - low).max(1)) as u16).collect()
}

/// The music's banner: a painting of the game's (from `files`, where given) and over it the
/// menu theme's `loudness` (thousandths, as [`loudness`] gives it) as [`BARS`] bars from amber
/// to the game's cyan, mirrored on a line of light.
pub fn music(files: Option<&Files>, loudness: &[u16]) -> Vec<u8> {
    let mut canvas = files.and_then(|f| picture(f, MUSIC_PAINTING, true)).map_or_else(|| Layer::ground([60, 24, 12]), |p| p.cover(WIDTH, HEIGHT, (500, 400)));
    canvas.grade(720, 1050);
    canvas.vignette(550);
    // The bars' heights: each stretch's loudness (or a quiet hum without one), a little lower
    // towards the ends.
    let heights: Vec<u32> = (0..BARS)
        .map(|i| {
            let level = if loudness.is_empty() { 350 } else { loudness[i * loudness.len() / BARS].min(1000) as u32 };
            let t = (2 * i as i64 + 1 - BARS as i64) * 1000 / BARS as i64;
            let window = 600 + 400 * (1_000_000 - t * t) as u32 / 1_000_000;
            10 + 170 * (level * window / 1000) / 1000
        })
        .collect();
    let (bar, gap) = (6u32, 4u32);
    let total = BARS as u32 * (bar + gap) - gap;
    let x0 = (WIDTH - total) / 2;
    let mid = HEIGHT * 60 / 100;
    // A soft dark band behind them.
    let mut band = Layer::new(WIDTH, HEIGHT);
    for y in 0..HEIGHT {
        let (from, to, edge) = (mid as i64 - 190, mid as i64 + 90, 60i64);
        let y = y as i64;
        let inside = (y - (from - edge)).min((to + edge) - y).clamp(0, 2 * edge);
        let a = (22_937 * inside / (2 * edge)) as u16;
        for x in 0..WIDTH {
            band.px[(y as u32 * WIDTH + x) as usize][3] = a;
        }
    }
    canvas.shade(&band, 0, 0);
    let mut bars = Layer::new(WIDTH, HEIGHT);
    let (amber, cyan) = ([255u32, 168, 56], [102u32, 224, 255]);
    for (i, &hgt) in heights.iter().enumerate() {
        let t = i as u32 * 1000 / (BARS as u32 - 1);
        let colour: [u32; 3] = std::array::from_fn(|c| (amber[c] * (1000 - t) + cyan[c] * t) / 1000 * 257);
        let x = x0 + i as u32 * (bar + gap);
        for n in 0..hgt {
            // (Brighter at its top: `n` counts up from its foot.)
            let lit = 887 + 113 * n / hgt.max(1);
            let a = 62_915u32;
            let px = [0, 1, 2].map(|c| (colour[c] * lit / 1000 * a / ONE) as u16);
            for dx in 0..bar {
                bars.px[((mid - 1 - n) * WIDTH + x + dx) as usize] = [px[0], px[1], px[2], a as u16];
            }
        }
        let mirrored = hgt * 4 / 10;
        for n in 0..mirrored {
            let a = 24_903 * (mirrored - n) / mirrored.max(1);
            let px = colour.map(|v| (v * a / ONE) as u16);
            for dx in 0..bar {
                bars.px[((mid + 4 + n) * WIDTH + x + dx) as usize] = [px[0], px[1], px[2], a as u16];
            }
        }
    }
    let (glow, pad) = halo(&bars, 16, 850, [255, 184, 102]);
    canvas.screen(&glow, -pad, -pad);
    canvas.over(&bars, 0, 0, ONE);
    let mut line = Layer::new(total + 60, 2);
    let lit = [255u32, 230, 179].map(|v| (v * 257 * 58_982 / ONE) as u16);
    line.px.fill([lit[0], lit[1], lit[2], 58_982]);
    let (glow, pad) = halo(&line, 8, 1000, [255, 204, 128]);
    canvas.screen(&glow, x0 as i32 - 30 - pad, mid as i32 - pad);
    canvas.over(&line, x0 as i32 - 30, mid as i32, ONE);
    png(&canvas)
}

/// The menus' banner: the space the game's menus show, its logo on the left, and on the right a
/// panel of its menus holding one of its paintings, lit round its edge. None when the copy has
/// none of them.
pub fn menus(files: &Files) -> Option<Vec<u8>> {
    let (space, panel, logo) = (picture(files, SPACE, true), picture(files, PANEL, false), picture(files, LOGO, false));
    if space.is_none() && panel.is_none() && logo.is_none() {
        return None;
    }
    let mut canvas = space.map_or_else(|| Layer::ground([10, 22, 34]), |s| s.cover(WIDTH, HEIGHT, (500, 400)));
    canvas.grade(1000, 1100);
    canvas.vignette(450);
    if let Some(panel) = panel {
        let panel = panel.across(560);
        let (x, y) = ((WIDTH - panel.w - 70) as i32, ((HEIGHT - panel.h) / 2 + 10) as i32);
        lay(&mut canvas, &panel, x, y, (18, 12, 900), Some((24, [89, 204, 255], 450)));
        // (The painting in its well: below its title bar, above its foot.)
        if let Some(painting) = picture(files, PANEL_PAINTING, true) {
            let (iw, ih) = (panel.w * 950 / 1000, panel.h * 700 / 1000);
            canvas.over(&painting.cover(iw, ih, (500, 400)), x + (panel.w * 25 / 1000) as i32, y + (panel.h * 135 / 1000) as i32, ONE * 92 / 100);
        }
    }
    if let Some(logo) = logo {
        let logo = logo.across(440);
        lay(&mut canvas, &logo, 80, (HEIGHT / 2) as i32 - 70, (8, 4, 850), None);
    }
    Some(png(&canvas))
}

/// The picture as a PNG: eight bits a colour, no alpha (laid over black), each row filtered by
/// its neighbours (Paeth's), deflated.
fn png(canvas: &Layer) -> Vec<u8> {
    use std::io::Write;
    let (w, h) = (canvas.w as usize, canvas.h as usize);
    let rgb: Vec<u8> = canvas.px.iter().flat_map(|p| [p[0], p[1], p[2]].map(|v| ((v as u32 + 128) / 257) as u8)).collect();
    let stride = w * 3;
    let mut raw = Vec::with_capacity((stride + 1) * h);
    for y in 0..h {
        raw.push(4);
        for i in 0..stride {
            let left = if i >= 3 { rgb[y * stride + i - 3] } else { 0 };
            let up = if y > 0 { rgb[(y - 1) * stride + i] } else { 0 };
            let corner = if y > 0 && i >= 3 { rgb[(y - 1) * stride + i - 3] } else { 0 };
            raw.push(rgb[y * stride + i].wrapping_sub(paeth(left, up, corner)));
        }
    }
    let mut zip = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    zip.write_all(&raw).expect("written to memory");
    let data = zip.finish().expect("written to memory");
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut head = Vec::with_capacity(13);
    head.extend_from_slice(&(w as u32).to_be_bytes());
    head.extend_from_slice(&(h as u32).to_be_bytes());
    head.extend_from_slice(&[8, 2, 0, 0, 0]);
    for (kind, body) in [(b"IHDR", head.as_slice()), (b"IDAT", data.as_slice()), (b"IEND", &[][..])] {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(body);
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out.extend_from_slice(&crc.sum().to_be_bytes());
    }
    out
}

/// Paeth's guess at a byte from its left, upper and upper left neighbours.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An uncompressed DDS picture, `w` by `h`, every texel `rgba`.
    fn dds_of(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut b = dds::header(w, h, 1, None);
        for _ in 0..w * h {
            b.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        }
        b
    }

    /// A copy of the game whose archive holds the pictures the banners are made of (`with`: those
    /// of these paths), each small and of one colour.
    fn copy(dir: &Path, with: &[&str]) -> Files {
        let pictures: Vec<(String, Vec<u8>)> = with
            .iter()
            .map(|path| {
                let picture = match *path {
                    p if EMBLEMS.iter().any(|(_, e, _)| *e == p) => dds_of(6, 6, [250, 250, 250, 200]),
                    PANEL | LOGO => dds_of(10, 8, [120, 140, 150, 220]),
                    _ => dds_of(16, 9, [40, 60, 90, 255]),
                };
                (path.to_string(), picture)
            })
            .collect();
        let entries: Vec<(&str, &[u8], bool)> = pictures.iter().map(|(p, b)| (p.as_str(), b.as_slice(), true)).collect();
        let archive = dir.join(format!("textures{}.scd", with.len()));
        std::fs::write(&archive, crate::scd::tests::zip(&entries)).unwrap();
        Files::of(Vec::new(), vec![crate::scd::Scd::open(&archive).unwrap()])
    }

    /// A PNG read back: its size and its texels (eight-bit red, green, blue), its chunks' sums
    /// checked and its rows unfiltered.
    fn read_png(png: &[u8]) -> (u32, u32, Vec<u8>) {
        use std::io::Read;
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let (mut at, mut size, mut data) = (8, (0, 0), Vec::new());
        while at < png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let (kind, body) = (&png[at + 4..at + 8], &png[at + 8..at + 8 + len]);
            let mut crc = flate2::Crc::new();
            crc.update(kind);
            crc.update(body);
            assert_eq!(crc.sum().to_be_bytes(), png[at + 8 + len..at + 12 + len], "{kind:?}: its sum");
            match kind {
                b"IHDR" => {
                    size = (u32::from_be_bytes(body[..4].try_into().unwrap()), u32::from_be_bytes(body[4..8].try_into().unwrap()));
                    assert_eq!(&body[8..], [8, 2, 0, 0, 0], "eight bits a colour, no alpha");
                }
                b"IDAT" => data.extend_from_slice(body),
                _ => {}
            }
            at += 12 + len;
        }
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(&data[..]).read_to_end(&mut raw).unwrap();
        let stride = size.0 as usize * 3;
        let mut rgb = vec![0u8; stride * size.1 as usize];
        for y in 0..size.1 as usize {
            let row = &raw[y * (stride + 1)..(y + 1) * (stride + 1)];
            assert_eq!(row[0], 4, "Paeth's filter");
            for i in 0..stride {
                let left = if i >= 3 { rgb[y * stride + i - 3] } else { 0 };
                let up = if y > 0 { rgb[(y - 1) * stride + i] } else { 0 };
                let corner = if y > 0 && i >= 3 { rgb[(y - 1) * stride + i - 3] } else { 0 };
                rgb[y * stride + i] = row[i + 1].wrapping_add(paeth(left, up, corner));
            }
        }
        (size.0, size.1, rgb)
    }

    /// Each new texel's weights come to one whole, made smaller or larger; a picture of one
    /// colour stays that colour either way.
    #[test]
    fn pictures_are_made_smaller_and_larger_evenly() {
        for (from, to) in [(1824, 1280), (7, 3), (60, 92), (3, 3), (5, 1), (1, 4), (256, 300)] {
            let k = kernel(from, to);
            assert_eq!(k.len(), to as usize);
            for (first, weights) in &k {
                assert_eq!(weights.iter().sum::<u32>(), 65536, "{from} to {to}");
                assert!(first + weights.len() <= from as usize, "{from} to {to}: within its texels");
            }
        }
        let plain = Layer::of(&Image { width: 4, height: 4, rgba: vec![[200, 100, 50, 255]; 16] }, false);
        for (w, h) in [(3, 5), (9, 2), (4, 4)] {
            let made = plain.resized(w, h);
            assert_eq!((made.w, made.h), (w, h));
            assert!(made.px.iter().all(|&p| p == plain.px[0]), "{w} by {h}: {:?}", made.px[0]);
        }
        // (A square picture covering a tall place: cut round its focus, the size asked.)
        let covered = plain.cover(10, 30, (500, 500));
        assert_eq!((covered.w, covered.h), (10, 30));
    }

    /// The PNG holds the picture as it is, over black where it is see-through.
    #[test]
    fn the_png_holds_the_picture() {
        let mut canvas = Layer::new(3, 2);
        let full = ONE as u16;
        canvas.px = vec![[full, 0, 0, full], [0, full, 0, full], [0, 0, full, full], [32_768, 32_768, 32_768, full], [0, 0, 0, 0], [257 * 10, 257 * 20, 257 * 30, full]];
        let (w, h, rgb) = read_png(&png(&canvas));
        assert_eq!((w, h), (3, 2));
        assert_eq!(rgb, [255, 0, 0, 0, 255, 0, 0, 0, 255, 128, 128, 128, 0, 0, 0, 10, 20, 30]);
    }

    /// A medallion: clear at its corners, its glass dark in its middle, its ring its colour.
    #[test]
    fn a_medallion_is_ringed_in_its_colour() {
        let m = medallion(40, [255, 0, 0], 3);
        assert_eq!(m.at(0, 0)[3], 0, "clear at its corners");
        let middle = m.at(20, 20);
        assert!(middle[3] > 50_000 && middle[0] < 15_000, "dark glass: {middle:?}");
        let ring = m.at(1, 20);
        assert!(ring[0] > 4 * ring[1].max(1) && ring[3] > 20_000, "red ring: {ring:?}");
    }

    /// A card turned away to its left is narrower, its far (left) column shorter than its near
    /// one, its reflection beneath it fading.
    #[test]
    fn a_card_turns_away() {
        let tile = Layer::of(&Image { width: 20, height: 20, rgba: vec![[90, 90, 90, 255]; 400] }, false);
        let (face, both) = card(&tile, 500, 800, true);
        assert_eq!((face.w, face.h, both.h), (10, 20, 20 + 6 + 10));
        let tall = |x: u32| (0..face.h).filter(|&y| face.at(x, y)[3] > 0).count();
        // (Each column as tall as its middle says: the nearest a little short of the whole.)
        assert!(tall(0) < tall(9) && tall(9) >= 19, "{} {}", tall(0), tall(9));
        let reflected: Vec<u16> = (26..36).map(|y| both.at(9, y)[3]).collect();
        assert!(reflected[0] > reflected[5] && reflected[5] > 0, "{reflected:?}");
    }

    /// The maps the pack's banner shows first: those of its showcase in its order, then the rest
    /// by name, whatever their case.
    #[test]
    fn the_showcase_maps_come_first() {
        let sources = ["x1mp_017", "SCMP_009", "scmp_001", "scmp_003"].map(String::from);
        assert_eq!(map_order(&sources), ["scmp_009", "scmp_003", "scmp_001", "x1mp_017"]);
    }

    /// The loudness round a track's loudest second: its stretch's bars from the quietest (0) to
    /// the loudest (1000), the loud one where the track is loud; eight bits a sample too.
    #[test]
    fn the_loudness_is_round_the_loudest_second() {
        // (Thirty seconds of a hundred samples, quiet but for the 20th second, loud.)
        let rate = 100u32;
        let level = |s: usize| if (2000..2100).contains(&s) { 20_000i16 } else { 500 + (s % 7) as i16 * 100 };
        let pcm: Vec<u8> = (0..3000).flat_map(|s| level(s).to_le_bytes()).collect();
        let bars = loudness(&pcm, 16, 1, rate, 16);
        assert_eq!(bars.len(), 16);
        assert_eq!((bars.iter().copied().min(), bars.iter().copied().max()), (Some(0), Some(1000)));
        // (The stretch, eight seconds from 16.5 s: its loud second from its 3.5th, bars 7 and 8.)
        let loudest = bars.iter().position(|&b| b == 1000).unwrap();
        assert!((6..=8).contains(&loudest), "{bars:?}");
        let pcm8: Vec<u8> = (0..3000).map(|s| if (2000..2100).contains(&s) { 250 } else { 130 }).collect();
        let bars8 = loudness(&pcm8, 8, 1, rate, 16);
        assert_eq!(bars8.iter().position(|&b| b == 1000), Some(loudest));
        assert_eq!(loudness(&[], 16, 2, rate, 4), [0; 4]);
    }

    /// The banners are made of the copy's pictures, as large as the launcher keeps them, the
    /// same each time; without the copy's pictures, none (the music's is made all the same, over
    /// a ground of its own).
    #[test]
    fn the_banners_are_made_of_the_copys_pictures() {
        let dir = crate::tests::Temp::new("banners");
        let all: Vec<&str> = [HALL, RAYS, MUSIC_PAINTING, SPACE, PANEL, PANEL_PAINTING, LOGO].into_iter().chain(EMBLEMS.iter().map(|(_, p, _)| *p)).collect();
        let files = copy(&dir.0, &all);
        let previews = vec![Image { width: 8, height: 8, rgba: vec![[30, 120, 200, 0]; 64] }, Image { width: 8, height: 8, rgba: vec![[200, 160, 90, 0]; 64] }];
        let made = [factions(&files, &[]), maps(&previews, Some(&files)), maps(&previews[..1], Some(&files)), Some(music(Some(&files), &[0, 1000, 500])), menus(&files)];
        for png in &made {
            let (w, h, rgb) = read_png(png.as_ref().expect("made"));
            assert_eq!((w, h), (WIDTH, HEIGHT));
            assert!(rgb.iter().any(|&c| c > 40), "not black throughout");
        }
        assert_eq!(factions(&files, &[]), made[0], "the same each time");
        // (A mod of some of the factions shows theirs alone: none named is all four.)
        assert_eq!(factions(&files, &["UEF", "AEON", "CYBRAN", "SERAPHIM"]), made[0]);
        let uef = factions(&files, &["uef"]).unwrap();
        assert_ne!(Some(&uef), made[0].as_ref());
        assert_eq!(read_png(&uef).0, WIDTH);
        assert_eq!(Some(music(Some(&files), &[0, 1000, 500])), made[3], "the same each time");
        let none = copy(&dir.0, &[]);
        assert!(factions(&none, &[]).is_none() && menus(&none).is_none() && maps(&[], Some(&none)).is_none());
        assert_eq!(read_png(&music(None, &[])).0, WIDTH);
        // (The emblems alone: the factions' banner over a ground of its own.)
        let emblems = copy(&dir.0, &EMBLEMS.map(|(_, p, _)| p));
        assert!(factions(&emblems, &["CYBRAN"]).is_some());
    }
}
