//! The intro room's pictures (see `intro_room.rs`), painted when the room goes up: the posters,
//! the game's boxes, the calendar, the manual, a CD's insert, a sticky note and the keys'
//! legends. The game's own are made from TA's art in the install (the title plate, the painted
//! battle of its installer, the jungle of its campaign, the Cavedog stone, the ARM and CORE
//! emblems, the units' build pictures) and lettered in TA's own fonts; the rest of the wall
//! (a film's, a sunset's, a band's) is drawn here, original. Nothing of TA's is kept in the
//! repository: without the install the game's posters come out blank.

use ta_formats::fnt::Font;
use ta_formats::pcx::PcxImage;
use ta_render::room::KeyLegend;
use ta_vfs::Vfs;

pub type Rgba = [f32; 4];

/// sRGB bytes to a linear colour.
pub fn rgb(r: u8, g: u8, b: u8) -> Rgba {
    [lin(r), lin(g), lin(b), 1.0]
}

/// With an alpha.
pub fn rgba(r: u8, g: u8, b: u8, a: f32) -> Rgba {
    [lin(r), lin(g), lin(b), a]
}

fn lin(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

/// A picture read from the install (linear, straight alpha).
pub struct Image {
    pub w: usize,
    pub h: usize,
    px: Vec<Rgba>,
}

impl Image {
    /// From opaque sRGB pixels, row by row.
    pub fn from_srgba(w: usize, h: usize, rgba: &[u8]) -> Image {
        let px = rgba.as_chunks::<4>().0.iter().map(|c| [lin(c[0]), lin(c[1]), lin(c[2]), c[3] as f32 / 255.0]).collect();
        Image { w, h, px }
    }

    pub fn pcx(vfs: &Vfs, path: &str) -> Option<Image> {
        let pcx = PcxImage::parse(&vfs.read_opt(path)?).ok()?;
        let rgba = pcx.to_rgba();
        let px = rgba.as_chunks::<4>().0.iter().map(|c| [lin(c[0]), lin(c[1]), lin(c[2]), c[3] as f32 / 255.0]).collect();
        Some(Image { w: pcx.width as usize, h: pcx.height as usize, px })
    }

    fn sample(&self, x: f32, y: f32) -> Rgba {
        bilinear(&self.px, self.w, self.h, x, y)
    }
}

/// A bilinear sample at pixel coordinates (clamped to the picture).
fn bilinear(px: &[Rgba], w: usize, h: usize, x: f32, y: f32) -> Rgba {
    let x = x.clamp(0.0, w as f32 - 1.0);
    let y = y.clamp(0.0, h as f32 - 1.0);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let p = |x: usize, y: usize| px[y * w + x];
    let mut out = [0.0; 4];
    for (k, o) in out.iter_mut().enumerate() {
        let a = p(x0, y0)[k] * (1.0 - fx) + p(x1, y0)[k] * fx;
        let b = p(x0, y1)[k] * (1.0 - fx) + p(x1, y1)[k] * fx;
        *o = a * (1.0 - fy) + b * fy;
    }
    out
}

/// A picture being painted: linear colour, straight alpha.
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    px: Vec<Rgba>,
}

/// A rectangle: left, top, right, bottom (pixels).
pub type Rect = [f32; 4];

impl Canvas {
    pub fn new(w: usize, h: usize, fill: Rgba) -> Canvas {
        Canvas { w, h, px: vec![fill; w * h] }
    }

    fn over(&mut self, x: usize, y: usize, c: Rgba, cover: f32) {
        let a = (c[3] * cover).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let d = &mut self.px[y * self.w + x];
        for k in 0..3 {
            d[k] = d[k] * (1.0 - a) + c[k] * a;
        }
        d[3] = d[3] + (1.0 - d[3]) * a;
    }

    /// Pixels a rectangle touches, and how much of each it covers (edges anti-aliased).
    fn covering(&self, r: Rect, mut f: impl FnMut(usize, usize, f32)) {
        let (x0, y0) = (r[0].max(0.0).floor() as usize, r[1].max(0.0).floor() as usize);
        let (x1, y1) = ((r[2].ceil().max(0.0) as usize).min(self.w), (r[3].ceil().max(0.0) as usize).min(self.h));
        for y in y0..y1 {
            let cy = ((y as f32 + 1.0).min(r[3]) - (y as f32).max(r[1])).clamp(0.0, 1.0);
            for x in x0..x1 {
                let cx = ((x as f32 + 1.0).min(r[2]) - (x as f32).max(r[0])).clamp(0.0, 1.0);
                f(x, y, cx * cy);
            }
        }
    }

    pub fn rect(&mut self, r: Rect, c: Rgba) {
        let mut cells = Vec::new();
        self.covering(r, |x, y, k| cells.push((x, y, k)));
        for (x, y, k) in cells {
            self.over(x, y, c, k);
        }
    }

    /// Top to bottom.
    pub fn gradient(&mut self, r: Rect, top: Rgba, bottom: Rgba) {
        let mut cells = Vec::new();
        self.covering(r, |x, y, k| cells.push((x, y, k)));
        for (x, y, k) in cells {
            let t = ((y as f32 + 0.5 - r[1]) / (r[3] - r[1]).max(1.0)).clamp(0.0, 1.0);
            let c = [0, 1, 2, 3].map(|i| top[i] * (1.0 - t) + bottom[i] * t);
            self.over(x, y, c, k);
        }
    }

    /// A rectangle with rounded corners of `radius`.
    pub fn round_rect(&mut self, r: Rect, radius: f32, c: Rgba) {
        self.shape(r, |x, y| rounded_distance(x, y, r, radius), c);
    }

    /// An ellipse, its edge blurred over `soft` of its radius (0 = sharp).
    pub fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, soft: f32, c: Rgba) {
        let r = [cx - rx, cy - ry, cx + rx, cy + ry];
        let mut cells = Vec::new();
        self.covering(r, |x, y, _| cells.push((x, y)));
        for (x, y) in cells {
            let (dx, dy) = ((x as f32 + 0.5 - cx) / rx, (y as f32 + 0.5 - cy) / ry);
            let d = (dx * dx + dy * dy).sqrt();
            let k = if soft > 0.0 { 1.0 - smooth((d - (1.0 - soft)) / soft) } else { (1.0 - d) * rx.min(ry) + 0.5 };
            self.over(x, y, c, k.clamp(0.0, 1.0));
        }
    }

    /// Fills where `dist` (pixels outside the shape; negative inside) is below zero.
    fn shape(&mut self, r: Rect, dist: impl Fn(f32, f32) -> f32, c: Rgba) {
        let grown = [r[0] - 1.0, r[1] - 1.0, r[2] + 1.0, r[3] + 1.0];
        let mut cells = Vec::new();
        self.covering(grown, |x, y, _| cells.push((x, y)));
        for (x, y) in cells {
            let d = dist(x as f32 + 0.5, y as f32 + 0.5);
            self.over(x, y, c, (0.5 - d).clamp(0.0, 1.0));
        }
    }

    /// A ring round an ellipse's edge, `width` pixels wide.
    pub fn ring(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, width: f32, c: Rgba) {
        let r = [cx - rx - width, cy - ry - width, cx + rx + width, cy + ry + width];
        self.shape(
            r,
            |x, y| {
                let (dx, dy) = ((x - cx) / rx, (y - cy) / ry);
                // (Distance to the edge, roughly, in pixels.)
                ((dx * dx + dy * dy).sqrt() - 1.0).abs() * rx.min(ry) - width * 0.5
            },
            c,
        );
    }

    /// A convex polygon (its corners in order).
    pub fn polygon(&mut self, points: &[[f32; 2]], c: Rgba) {
        if points.len() < 3 {
            return;
        }
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in points {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        // Wound either way: the distance outside is the largest across its edges' lines.
        let area: f32 = (0..points.len()).map(|i| {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            a[0] * b[1] - b[0] * a[1]
        }).sum();
        let sign = if area < 0.0 { -1.0 } else { 1.0 };
        self.shape(
            [lo[0], lo[1], hi[0], hi[1]],
            |x, y| {
                (0..points.len())
                    .map(|i| {
                        let (a, b) = (points[i], points[(i + 1) % points.len()]);
                        let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
                        let len = (ex * ex + ey * ey).sqrt().max(1e-4);
                        // Outwards for a polygon wound counter-clockwise on the picture.
                        sign * ((x - a[0]) * ey - (y - a[1]) * ex) / len
                    })
                    .fold(f32::MIN, f32::max)
            },
            c,
        );
    }

    /// Mottles the colour inside `r` by up to ±`amount`, in blotches `cell` pixels across.
    pub fn noise(&mut self, r: Rect, amount: f32, cell: f32, seed: u32) {
        let mut cells = Vec::new();
        self.covering(r, |x, y, _| cells.push((x, y)));
        for (x, y) in cells {
            let n = value_noise(x as f32 / cell, y as f32 / cell, seed) * 0.65 + value_noise(x as f32 / (cell * 0.25), y as f32 / (cell * 0.25), seed ^ 0x9e37) * 0.35;
            let k = 1.0 + (n * 2.0 - 1.0) * amount;
            let p = &mut self.px[y * self.w + x];
            for c in p.iter_mut().take(3) {
                *c *= k;
            }
        }
    }

    /// Darkens (or lightens, with `amount` below zero) towards the edges of `r`.
    pub fn vignette(&mut self, r: Rect, amount: f32) {
        let mut cells = Vec::new();
        self.covering(r, |x, y, _| cells.push((x, y)));
        let (cx, cy) = ((r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5);
        let (hw, hh) = ((r[2] - r[0]) * 0.5, (r[3] - r[1]) * 0.5);
        for (x, y) in cells {
            let (dx, dy) = ((x as f32 - cx) / hw, (y as f32 - cy) / hh);
            let k = 1.0 - amount * smooth((dx * dx + dy * dy).sqrt() - 0.4);
            let p = &mut self.px[y * self.w + x];
            for c in p.iter_mut().take(3) {
                *c *= k;
            }
        }
    }

    /// Every pixel of `r` remade by `f` from its centre's place and what it was.
    pub fn each(&mut self, r: Rect, mut f: impl FnMut(f32, f32, Rgba) -> Rgba) {
        let (x0, y0) = (r[0].max(0.0) as usize, r[1].max(0.0) as usize);
        let (x1, y1) = ((r[2].ceil().max(0.0) as usize).min(self.w), (r[3].ceil().max(0.0) as usize).min(self.h));
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * self.w + x;
                self.px[i] = f(x as f32 + 0.5, y as f32 + 0.5, self.px[i]);
            }
        }
    }

    /// Light added (not laid over: neon stays bright over what it lights): `c` as bright as
    /// `strength` at (cx, cy), falling to nothing at the ellipse `rx` × `ry`.
    pub fn glow(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, c: Rgba, strength: f32) {
        self.each([cx - rx, cy - ry, cx + rx, cy + ry], |x, y, mut p| {
            let d = ((((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2)).sqrt()).min(1.0);
            let k = (1.0 - d) * (1.0 - d) * strength;
            for i in 0..3 {
                p[i] += c[i] * k;
            }
            p
        });
    }

    /// Part `src` of `img` into `dst`, `opacity` of it, tinted.
    pub fn image(&mut self, img: &Image, src: Rect, dst: Rect, opacity: f32, tint: [f32; 3]) {
        let mut cells = Vec::new();
        self.covering(dst, |x, y, k| cells.push((x, y, k)));
        for (x, y, k) in cells {
            let u = src[0] + (x as f32 + 0.5 - dst[0]) / (dst[2] - dst[0]) * (src[2] - src[0]) - 0.5;
            let v = src[1] + (y as f32 + 0.5 - dst[1]) / (dst[3] - dst[1]) * (src[3] - src[1]) - 0.5;
            let s = img.sample(u, v);
            self.over(x, y, [s[0] * tint[0], s[1] * tint[1], s[2] * tint[2], s[3] * opacity], k);
        }
    }

    /// The whole of `img` fitted into `dst`, keeping its shape and centred.
    pub fn image_fit(&mut self, img: &Image, src: Rect, dst: Rect, opacity: f32) {
        let (sw, sh) = (src[2] - src[0], src[3] - src[1]);
        let (dw, dh) = (dst[2] - dst[0], dst[3] - dst[1]);
        let k = (dw / sw).min(dh / sh);
        let (w, h) = (sw * k, sh * k);
        let (x, y) = (dst[0] + (dw - w) * 0.5, dst[1] + (dh - h) * 0.5);
        self.image(img, src, [x, y, x + w, y + h], opacity, [1.0; 3]);
    }

    /// Part `src` of `img` filling `dst`, cropped to its shape.
    pub fn image_cover(&mut self, img: &Image, src: Rect, dst: Rect, opacity: f32) {
        let (sw, sh) = (src[2] - src[0], src[3] - src[1]);
        let (dw, dh) = (dst[2] - dst[0], dst[3] - dst[1]);
        let k = (dw / sw).max(dh / sh);
        let (w, h) = (dw / k, dh / k);
        let (x, y) = (src[0] + (sw - w) * 0.5, src[1] + (sh - h) * 0.5);
        self.image(img, [x, y, x + w, y + h], dst, opacity, [1.0; 3]);
    }

    /// A line of TA lettering `height` pixels tall from (x, y) (its top left), smoothed as it
    /// is scaled up; `rotated` runs it up the picture (a box's spine). Returns its width.
    #[allow(clippy::too_many_arguments)]
    pub fn text(&mut self, font: &Font, s: &str, x: f32, y: f32, height: f32, c: Rgba, rotated: bool) -> f32 {
        let scale = height / font.height.max(1) as f32;
        let mut pen = 0.0;
        for ch in s.chars() {
            let Some(g) = font.glyph(ch) else { continue };
            let (gw, gh) = (g.width as usize, font.height as usize);
            let coverage = |gx: f32, gy: f32| -> f32 {
                let (fx, fy) = (gx - 0.5, gy - 0.5);
                let (x0, y0) = (fx.floor(), fy.floor());
                let bit = |x: f32, y: f32| -> f32 {
                    if x < 0.0 || y < 0.0 || x >= gw as f32 || y >= gh as f32 {
                        0.0
                    } else if g.pixels[y as usize * gw + x as usize] {
                        1.0
                    } else {
                        0.0
                    }
                };
                let (tx, ty) = (fx - x0, fy - y0);
                let a = bit(x0, y0) * (1.0 - tx) + bit(x0 + 1.0, y0) * tx;
                let b = bit(x0, y0 + 1.0) * (1.0 - tx) + bit(x0 + 1.0, y0 + 1.0) * tx;
                // Sharpened, so scaled-up letters keep their edges.
                smooth((a * (1.0 - ty) + b * ty - 0.25) * 2.0)
            };
            let (w, h) = (gw as f32 * scale, height);
            let cell = if rotated { [x, y - pen - w, x + h, y - pen] } else { [x + pen, y, x + pen + w, y + h] };
            let mut cells = Vec::new();
            self.covering(cell, |px, py, _| cells.push((px, py)));
            for (px, py) in cells {
                let (gx, gy) = if rotated {
                    ((y - pen - (py as f32 + 0.5)) / scale, (px as f32 + 0.5 - x) / scale)
                } else {
                    ((px as f32 + 0.5 - x - pen) / scale, (py as f32 + 0.5 - y) / scale)
                };
                let k = coverage(gx, gy);
                if k > 0.0 {
                    self.over(px, py, c, k);
                }
            }
            pen += w;
        }
        pen
    }

    /// Width of a line of lettering at `height`.
    pub fn text_width(font: &Font, s: &str, height: f32) -> f32 {
        font.width(s) as f32 * height / font.height.max(1) as f32
    }

    /// A line of lettering centred on `cx`.
    pub fn text_centred(&mut self, font: &Font, s: &str, cx: f32, y: f32, height: f32, c: Rgba) {
        let w = Canvas::text_width(font, s, height);
        self.text(font, s, cx - w * 0.5, y, height, c, false);
    }

    /// Into a texture layer `size` pixels square, stretched to fill it (the model's UVs span the
    /// picture whatever its shape), sRGB bytes.
    pub fn into_square_layer(self, size: usize) -> Vec<u8> {
        let mut out = vec![0u8; size * size * 4];
        let (sx, sy) = (self.w as f32 / size as f32, self.h as f32 / size as f32);
        for y in 0..size {
            for x in 0..size {
                let p = if self.w == size && self.h == size { self.px[y * size + x] } else { bilinear(&self.px, self.w, self.h, (x as f32 + 0.5) * sx - 0.5, (y as f32 + 0.5) * sy - 0.5) };
                let o = (y * size + x) * 4;
                out[o] = to_srgb(p[0]);
                out[o + 1] = to_srgb(p[1]);
                out[o + 2] = to_srgb(p[2]);
                out[o + 3] = (p[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        }
        out
    }
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Distance from (x, y) to a rounded rectangle's edge (negative inside).
fn rounded_distance(x: f32, y: f32, r: Rect, radius: f32) -> f32 {
    let (cx, cy) = ((r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5);
    let (hw, hh) = ((r[2] - r[0]) * 0.5 - radius, (r[3] - r[1]) * 0.5 - radius);
    let (dx, dy) = ((x - cx).abs() - hw, (y - cy).abs() - hh);
    let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
    outside + dx.max(dy).min(0.0) - radius
}

fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0
}

/// Smooth noise 0..1.
fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = hash(x0, y0, seed) * (1.0 - sx) + hash(x0 + 1, y0, seed) * sx;
    let b = hash(x0, y0 + 1, seed) * (1.0 - sx) + hash(x0 + 1, y0 + 1, seed) * sx;
    a * (1.0 - sy) + b * sy
}

/// What the pictures are made from, read once from the install.
pub struct Art {
    pub title: Option<Image>,
    pub battle: Option<Image>,
    pub jungle: Option<Image>,
    pub cavedog: Option<Image>,
    pub arm: Option<Image>,
    pub core: Option<Image>,
    pub emblems: Option<Image>,
    /// Build pictures and names: ARM's, then CORE's.
    pub units: Vec<(Image, String, bool)>,
    pub heading: Option<Font>,
    pub small: Option<Font>,
    /// The Core Contingency is installed.
    pub core_contingency: bool,
    /// The battle plan's map: its picture, its name, its first two starts (0..1 across and
    /// down the picture).
    pub plan_map: Option<(Image, String, [[f32; 2]; 2])>,
    /// TA Reborn's emblem (its own art, `ta_ui::icon`), for the computer's case badge.
    pub emblem: Image,
}

/// Units on the field guide poster, six a side.
const GUIDE_ARM: [&str; 8] = ["ARMCOM", "ARMPW", "ARMSTUMP", "ARMBULL", "ARMHAWK", "ARMBRAWL", "ARMMART", "ARMFARK"];
const GUIDE_CORE: [&str; 8] = ["CORCOM", "CORAK", "CORRAID", "CORREAP", "CORVAMP", "CORAPE", "CORSTORM", "CORPYRO"];

impl Art {
    /// None of the art (tests: the pictures come out plain).
    #[cfg(test)]
    pub fn empty() -> Art {
        Art { title: None, battle: None, jungle: None, cavedog: None, arm: None, core: None, emblems: None, units: Vec::new(), heading: None, small: None, core_contingency: false, plan_map: None, emblem: Image::from_srgba(1, 1, &[0, 0, 0, 0]) }
    }

    pub fn load(vfs: &Vfs, data: &ta_data::GameData) -> Art {
        let font = |p: &str| vfs.read_opt(p).and_then(|b| Font::parse(&b).ok());
        let mut units = Vec::new();
        for (names, arm) in [(&GUIDE_ARM, true), (&GUIDE_CORE, false)] {
            for name in names.iter() {
                let Some(def) = data.unit(name) else { continue };
                if let Some(img) = Image::pcx(vfs, &format!("unitpics/{}.pcx", name.to_ascii_lowercase())) {
                    units.push((img, def.display_name.clone(), arm));
                }
            }
        }
        let has = |p: &str| vfs.read_opt(p).is_some();
        Art {
            title: Image::pcx(vfs, "bitmaps/titlscrn.pcx"),
            battle: Image::pcx(vfs, "bitmaps/installglam.pcx"),
            jungle: Image::pcx(vfs, "bitmaps/mission02win.pcx"),
            cavedog: Image::pcx(vfs, "bitmaps/cdlog256.pcx"),
            arm: Image::pcx(vfs, "bitmaps/armbkg.pcx"),
            core: Image::pcx(vfs, "bitmaps/corebkg.pcx"),
            emblems: Image::pcx(vfs, "bitmaps/newcamplogos.pcx"),
            units,
            heading: font("fonts/hatt14.fnt").or_else(|| font("fonts/hatt12.fnt")),
            small: font("fonts/smlfont.fnt").or_else(|| font("fonts/roman10.fnt")),
            core_contingency: has("unitpics/corpyro.pcx") || data.unit("CORPYRO").is_some(),
            plan_map: plan_map(vfs, data),
            emblem: Image::from_srgba(256, 256, &ta_ui::icon::emblem_rgba(256)),
        }
    }
}

/// The map on the battle plan: Coast To Coast, or else the first map of the install with two
/// starts.
fn plan_map(vfs: &Vfs, data: &ta_data::GameData) -> Option<(Image, String, [[f32; 2]; 2])> {
    let read = |name: &str| -> Option<(Image, String, [[f32; 2]; 2])> {
        let map = ta_formats::tnt::TntMap::parse(&vfs.read_opt(&format!("maps/{name}.tnt"))?).ok()?;
        let info = ta_data::MapInfo::from_ota(&ta_formats::tdf::parse_bytes(&vfs.read_opt(&format!("maps/{name}.ota"))?));
        let starts = &info.schemas.first()?.start_positions;
        if starts.len() < 2 {
            return None;
        }
        let picture = ta_render::map_preview::render(&map, &data.palette, 640);
        let at = |&(_, x, z): &(u32, f32, f32)| [(x / picture.area.0.max(1.0)).clamp(0.05, 0.95), (z / picture.area.1.max(1.0)).clamp(0.05, 0.95)];
        let title = if info.name.trim().is_empty() { name.to_string() } else { info.name.clone() };
        Some((Image::from_srgba(picture.width, picture.height, &picture.rgba), title, [at(&starts[0]), at(&starts[1])]))
    };
    read("Coast To Coast").or_else(|| {
        vfs.list("maps", Some("tnt"), false)
            .iter()
            .filter_map(|path| {
                let file = path.rsplit('/').next()?;
                let stem = file.len().checked_sub(4).and_then(|n| file.get(..n))?;
                file[stem.len()..].eq_ignore_ascii_case(".tnt").then(|| stem.to_string())
            })
            .find_map(|name| read(&name))
    })
}

// Where the art sits in TA's pictures (640 × 480).
/// The metal title plate on the title screen.
const TITLE_PLATE: Rect = [86.0, 30.0, 574.0, 280.0];
/// The painted battle in the installer's frame.
const BATTLE_PAINTING: Rect = [74.0, 88.0, 384.0, 322.0];
/// The Cavedog stone.
const CAVEDOG_STONE: Rect = [96.0, 36.0, 548.0, 436.0];
/// The ARM and CORE emblems on their backgrounds (256 × 256).
const EMBLEM: Rect = [0.0, 20.0, 256.0, 236.0];
/// The green wireframe CORE emblem in the campaign logos.
const WIREFRAME_CORE: Rect = [40.0, 30.0, 180.0, 170.0];

/// Paper white, printed ink black.
fn paper() -> Rgba {
    rgb(232, 228, 214)
}

/// The main poster: TA's title plate over the painted battle, and the small print.
pub fn poster_main(art: &Art, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w, h, rgb(12, 14, 18));
    c.gradient([0.0, 0.0, fw, fh], rgb(20, 26, 36), rgb(6, 6, 8));
    let margin = fw * 0.04;
    if let Some(b) = &art.battle {
        c.image_cover(b, BATTLE_PAINTING, [margin, fh * 0.3, fw - margin, fh * 0.8], 1.0);
        c.vignette([margin, fh * 0.3, fw - margin, fh * 0.8], 0.45);
    }
    if let Some(t) = &art.title {
        c.image_fit(t, TITLE_PLATE, [margin, fh * 0.03, fw - margin, fh * 0.31], 1.0);
    }
    if let Some(f) = &art.heading {
        let h1 = fh * 0.04;
        c.text_centred(f, "ARM  VS  CORE", fw * 0.5, fh * 0.825, h1, rgb(220, 196, 120));
        c.text_centred(f, "THE WAR IS 4,000 YEARS OLD", fw * 0.5, fh * 0.875, h1 * 0.7, rgb(190, 190, 196));
    }
    if let Some(f) = &art.small {
        c.text_centred(f, "CAVEDOG ENTERTAINMENT  -  1997", fw * 0.5, fh * 0.935, fh * 0.018, rgb(160, 160, 166));
    }
    frame_poster(&mut c);
    c
}

/// The field guide: ARM's units on the left under their emblem, CORE's on the right.
pub fn poster_guide(art: &Art, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w, h, rgb(24, 28, 34));
    c.gradient([0.0, 0.0, fw, fh], rgb(30, 36, 44), rgb(12, 14, 18));
    c.noise([0.0, 0.0, fw, fh], 0.06, 40.0, 31);
    if let Some(f) = &art.heading {
        c.text_centred(f, "FIELD GUIDE", fw * 0.5, fh * 0.05, fh * 0.07, rgb(226, 204, 132));
    }
    for (arm, x0) in [(true, fw * 0.04), (false, fw * 0.52)] {
        let emblem = if arm { &art.arm } else { &art.core };
        if let Some(e) = emblem {
            c.image(e, EMBLEM, [x0 + fw * 0.13, fh * 0.14, x0 + fw * 0.31, fh * 0.34], 0.5, if arm { [0.7, 0.9, 1.3] } else { [1.4, 0.7, 0.6] });
        }
        if let Some(f) = &art.heading {
            let colour = if arm { rgb(120, 170, 240) } else { rgb(236, 110, 90) };
            c.text_centred(f, if arm { "ARM" } else { "CORE" }, x0 + fw * 0.22, fh * 0.2, fh * 0.07, colour);
        }
        let units: Vec<&(Image, String, bool)> = art.units.iter().filter(|u| u.2 == arm).collect();
        let (cols, cell) = (4, fw * 0.105);
        for (i, (img, name, _)) in units.iter().enumerate().take(8) {
            let (col, row) = (i % cols, i / cols);
            let x = x0 + col as f32 * (cell + fw * 0.006);
            let y = fh * 0.38 + row as f32 * (cell + fh * 0.1);
            c.rect([x - 2.0, y - 2.0, x + cell + 2.0, y + cell + 2.0], rgb(70, 76, 84));
            c.image(img, [0.0, 0.0, img.w as f32, img.h as f32], [x, y, x + cell, y + cell], 1.0, [1.0; 3]);
            if let Some(f) = &art.small {
                let label: String = name.chars().take(12).collect::<String>().to_uppercase();
                c.text_centred(f, &label, x + cell * 0.5, y + cell + fh * 0.012, fh * 0.03, rgb(200, 200, 204));
            }
        }
    }
    c.rect([fw * 0.5 - 1.0, fh * 0.14, fw * 0.5 + 1.0, fh * 0.94], rgba(200, 200, 200, 0.2));
    frame_poster(&mut c);
    c
}

/// The jungle of the campaign, with the expansion's name.
pub fn poster_jungle(art: &Art, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w, h, rgb(10, 14, 10));
    if let Some(j) = &art.jungle {
        // The right of the picture, where the vehicle crosses the river.
        c.image_cover(j, [120.0, 0.0, 640.0, 480.0], [0.0, 0.0, fw, fh * 0.82], 1.0);
        // (A dark picture on a dark wall: lifted.)
        c.each([0.0, 0.0, fw, fh * 0.82], |_, _, p| [p[0] * 1.7, p[1] * 1.7, p[2] * 1.6, p[3]]);
        c.vignette([0.0, 0.0, fw, fh * 0.82], 0.3);
    }
    c.gradient([0.0, fh * 0.6, fw, fh * 0.82], rgba(0, 0, 0, 0.0), rgba(0, 0, 0, 0.9));
    if let Some(f) = &art.heading {
        c.text_centred(f, "TOTAL ANNIHILATION", fw * 0.5, fh * 0.84, fh * 0.045, rgb(210, 214, 200));
        let sub = if art.core_contingency { "THE CORE CONTINGENCY" } else { "THE CORE PREVAILS" };
        c.text_centred(f, sub, fw * 0.5, fh * 0.9, fh * 0.035, rgb(120, 220, 110));
    }
    frame_poster(&mut c);
    c
}

// ---- Posters drawn here (nobody's art but ours) ---------------------------------------------------

/// `a` mixed `t` of the way to `b`.
fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// sRGB bytes as a linear colour without its alpha.
fn lin3(r: u8, g: u8, b: u8) -> [f32; 3] {
    [lin(r), lin(g), lin(b)]
}

/// `col` laid over `p`, `a` of it.
fn blend(p: Rgba, col: [f32; 3], a: f32) -> Rgba {
    let a = a.clamp(0.0, 1.0);
    [p[0] * (1.0 - a) + col[0] * a, p[1] * (1.0 - a) + col[1] * a, p[2] * (1.0 - a) + col[2] * a, p[3] + (1.0 - p[3]) * a]
}

/// A line of lettering `gap` pixels between its letters, centred on `cx`.
#[allow(clippy::too_many_arguments)]
fn spaced(c: &mut Canvas, font: &Font, s: &str, cx: f32, y: f32, height: f32, gap: f32, colour: Rgba) {
    let widths: Vec<f32> = s.chars().map(|ch| Canvas::text_width(font, &ch.to_string(), height)).collect();
    let total: f32 = widths.iter().sum::<f32>() + gap * s.chars().count().saturating_sub(1) as f32;
    let mut x = cx - total * 0.5;
    for (ch, w) in s.chars().zip(widths) {
        c.text(font, &ch.to_string(), x, y, height, colour, false);
        x += w + gap;
    }
}

/// Stars scattered over `r`: mostly small and dim, a few bright with a glint.
fn stars(c: &mut Canvas, r: Rect, count: usize, seed: u32, bright: f32) {
    for i in 0..count as i32 {
        let x = r[0] + hash(i, 1, seed) * (r[2] - r[0]);
        let y = r[1] + hash(i, 2, seed) * (r[3] - r[1]);
        let m = hash(i, 3, seed);
        let size = 0.7 + m.powi(8) * 2.4;
        let tint = [1.0, 0.9 + 0.1 * hash(i, 4, seed), 0.75 + 0.25 * hash(i, 5, seed)];
        let a = (0.3 + 0.7 * m) * bright;
        c.ellipse(x, y, size, size, 0.6, [tint[0], tint[1], tint[2], a]);
        if m > 0.94 {
            let len = size * 5.0;
            c.rect([x - len, y - 0.6, x + len, y + 0.6], [tint[0], tint[1], tint[2], a * 0.45]);
            c.rect([x - 0.6, y - len, x + 0.6, y + len], [tint[0], tint[1], tint[2], a * 0.45]);
        }
    }
}

/// A ball lit from `light` (a unit direction towards the light, x right, y down, z at the
/// viewer), its limb darkened.
fn ball(c: &mut Canvas, cx: f32, cy: f32, r: f32, base: [f32; 3], light: [f32; 3]) {
    c.each([cx - r - 2.0, cy - r - 2.0, cx + r + 2.0, cy + r + 2.0], |x, y, p| {
        let (dx, dy) = ((x - cx) / r, (y - cy) / r);
        let d2 = dx * dx + dy * dy;
        if d2 >= 1.0 + 3.0 / r {
            return p;
        }
        let cover = ((1.0 - d2.sqrt()) * r + 0.5).clamp(0.0, 1.0);
        let nz = (1.0 - d2.min(1.0)).sqrt();
        let lit = (dx * light[0] + dy * light[1] + nz * light[2]).max(0.0);
        let pits = value_noise(dx * 5.0 + 3.0, dy * 5.0, 21);
        let k = (0.03 + 0.97 * lit) * (0.8 + 0.4 * pits);
        blend(p, [base[0] * k, base[1] * k, base[2] * k], cover)
    });
}

/// A gas giant's disc: bands of rust and cream wrapped to the rings' plane, lit from the upper
/// left, an atmosphere's blue on its lit rim. `tilt` turns the rings' plane on the picture.
fn gas_giant(c: &mut Canvas, cx: f32, cy: f32, r: f32, tilt: f32, seed: u32) {
    let (st, ct) = tilt.sin_cos();
    let light = [-0.55f32, -0.5, 0.67];
    let len = (light[0] * light[0] + light[1] * light[1] + light[2] * light[2]).sqrt();
    let light = [light[0] / len, light[1] / len, light[2] / len];
    c.each([cx - r - 2.0, cy - r - 2.0, cx + r + 2.0, cy + r + 2.0], |x, y, p| {
        let (dx, dy) = ((x - cx) / r, (y - cy) / r);
        let d2 = dx * dx + dy * dy;
        if d2 >= 1.0 + 3.0 / r {
            return p;
        }
        let cover = ((1.0 - d2.sqrt()) * r + 0.5).clamp(0.0, 1.0);
        let nz = (1.0 - d2.min(1.0)).sqrt();
        let lit = (dx * light[0] + dy * light[1] + nz * light[2]).max(0.0);
        // The globe's latitude in the rings' frame, the bands bowed as the equator is seen from a
        // little above it.
        let (u, v) = (dx * ct + dy * st, -dx * st + dy * ct);
        let lat = v - 0.2537 * (1.0 - u * u).max(0.0).sqrt();
        let warp = value_noise(u * 2.2, lat * 8.0, seed) - 0.5;
        let stripe = ((lat * 14.0 + warp * 4.0).sin() * 0.5 + 0.5).powf(0.8);
        let fine = value_noise(u * 7.0, lat * 46.0, seed ^ 5);
        let base = mix3(lin3(168, 92, 56), lin3(238, 206, 150), stripe);
        let base = mix3(base, lin3(96, 40, 30), fine * 0.35);
        let shade = 0.03 + 0.97 * lit.powf(0.85);
        let limb = (1.0 - nz).powi(3) * (0.2 + lit);
        let col = [base[0] * shade + 0.10 * limb, base[1] * shade + 0.28 * limb, base[2] * shade + 0.55 * limb];
        blend(p, col, cover)
    });
}

/// The rings of a planet at (cx, cy), `r` its radius: bands and a division, tilted; `front`
/// draws the near half (over the planet), else the far half (behind it).
fn planet_rings(c: &mut Canvas, cx: f32, cy: f32, r: f32, tilt: f32, front: bool, seed: u32) {
    let (st, ct) = tilt.sin_cos();
    let (ra, rb) = (r * 2.05, r * 0.52);
    let reach = ra + 2.0;
    c.each([cx - reach, cy - reach, cx + reach, cy + reach], |x, y, p| {
        let (dx, dy) = (x - cx, y - cy);
        let (u, v) = (dx * ct + dy * st, -dx * st + dy * ct);
        let rho = ((u / ra).powi(2) + (v / rb).powi(2)).sqrt();
        if !(0.56..=1.0).contains(&rho) || (v > 0.0) != front {
            return p;
        }
        if !front && dx * dx + dy * dy < r * r {
            return p;
        }
        let band = 0.5 + 0.5 * (rho * 95.0).sin() * (0.6 + 0.4 * value_noise(rho * 40.0, 3.0, seed));
        let division = 1.0 - 0.85 * smooth(1.0 - ((rho - 0.77) / 0.014).abs());
        let edge = smooth((rho - 0.56) / 0.03) * smooth((1.0 - rho) / 0.03);
        // Lit on the planet's lit side, a little darker with the planet's shadow behind it.
        let lit = 0.55 + 0.45 * (-(u / ra) * 0.9 + 0.1).clamp(-1.0, 1.0);
        let col = mix3(lin3(120, 100, 80), lin3(232, 214, 176), lit);
        blend(p, col, (0.18 + 0.62 * band) * division * edge)
    });
}

/// A small ship, nose to the right at `angle`, a delta of two wings round a needle of a hull,
/// dark with a lit edge, its engines trailing light.
fn ship(c: &mut Canvas, cx: f32, cy: f32, size: f32, angle: f32) {
    let (s, co) = angle.sin_cos();
    let at = |x: f32, y: f32| [cx + (x * co - y * s) * size, cy + (x * s + y * co) * size];
    for k in 0..18 {
        let t = k as f32 / 17.0;
        let p = at(-0.62 - t * 2.6, 0.0);
        c.glow(p[0], p[1], size * (0.17 + 0.12 * (1.0 - t)), size * (0.17 + 0.12 * (1.0 - t)), rgb(255, 150, 70), 0.55 * (1.0 - t).powi(2));
    }
    let hull: [&[(f32, f32)]; 3] = [&[(1.0, 0.0), (-0.35, 0.10), (-0.58, 0.0), (-0.35, -0.10)], &[(0.25, 0.06), (-0.5, 0.70), (-0.62, 0.62), (-0.30, 0.06)], &[(0.25, -0.06), (-0.5, -0.70), (-0.62, -0.62), (-0.30, -0.06)]];
    for (dx, dy, colour) in [(-1.6, -1.6, rgb(128, 146, 182)), (0.0, 0.0, rgb(14, 16, 24))] {
        for poly in hull {
            let pts: Vec<[f32; 2]> = poly.iter().map(|&(x, y)| { let p = at(x, y); [p[0] + dx, p[1] + dy] }).collect();
            c.polygon(&pts, colour);
        }
    }
    let nose = at(0.55, 0.0);
    c.ellipse(nose[0], nose[1], size * 0.12, size * 0.035, 0.5, rgb(90, 190, 230));
}

/// A film's poster, drawn here: a ringed planet, a ship leaving it, the title in gold.
pub fn poster_space(art: &Art, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w, h, rgb(4, 5, 14));
    c.gradient([0.0, 0.0, fw, fh], rgb(3, 4, 15), rgb(34, 14, 58));
    // A faint nebula, violet with a teal edge.
    c.each([0.0, 0.0, fw, fh], |x, y, mut p| {
        let n = value_noise(x / 170.0, y / 170.0, 7) * 0.65 + value_noise(x / 55.0, y / 55.0, 9) * 0.35;
        let k = ((n - 0.5) * 2.2).clamp(0.0, 1.0) * (1.0 - y / fh * 0.6);
        p[0] += 0.05 * k;
        p[1] += 0.015 * k + 0.03 * k * k;
        p[2] += 0.13 * k;
        p
    });
    stars(&mut c, [0.0, 0.0, fw, fh * 0.8], 320, 11, 1.0);
    let (px, py, pr, tilt) = (fw * 0.60, fh * 0.37, fw * 0.255, -0.30f32);
    c.glow(px, py, pr * 2.6, pr * 2.6, rgb(120, 70, 200), 0.18);
    planet_rings(&mut c, px, py, pr, tilt, false, 5);
    gas_giant(&mut c, px, py, pr, tilt, 5);
    planet_rings(&mut c, px, py, pr, tilt, true, 5);
    ball(&mut c, fw * 0.17, fh * 0.2, fw * 0.045, lin3(190, 190, 205), [-0.4, -0.5, 0.77]);
    ship(&mut c, fw * 0.30, fh * 0.60, fw * 0.12, -0.42);
    // Under the title the picture goes dark.
    c.gradient([0.0, fh * 0.66, fw, fh], rgba(0, 0, 0, 0.0), rgba(2, 2, 8, 0.92));
    if let Some(f) = &art.heading {
        spaced(&mut c, f, "BEYOND", fw * 0.5, fh * 0.755, fh * 0.075, fw * 0.022, rgb(238, 208, 142));
        spaced(&mut c, f, "THE RINGS", fw * 0.5, fh * 0.845, fh * 0.046, fw * 0.014, rgb(238, 208, 142));
    }
    if let Some(f) = &art.small {
        c.text_centred(f, "THE LAST SHIP LEAVES AT DAWN", fw * 0.5, fh * 0.925, fh * 0.021, rgb(196, 196, 208));
        c.text_centred(f, "A HARLAN VOSS FILM  -  THIS SUMMER", fw * 0.5, fh * 0.955, fh * 0.015, rgb(140, 140, 156));
    }
    wear(&mut c);
    c
}

/// One palm's silhouette: a bowed trunk, a crown of long fronds that rise and arch over, each
/// with its fringe of leaflets.
fn palm(c: &mut Canvas, x: f32, base_y: f32, height: f32, lean: f32, ink: Rgba) {
    let trunk: Vec<[f32; 2]> = (0..=24).map(|i| { let t = i as f32 / 24.0; [x + lean * height * (t * t * 0.8 + t * 0.2), base_y - height * t] }).collect();
    for (i, pair) in trunk.windows(2).enumerate() {
        pen(c, pair, height * (0.034 - 0.020 * i as f32 / 24.0), ink);
    }
    let top = trunk[trunk.len() - 1];
    for k in 0..9 {
        let side = if k % 2 == 0 { -1.0 } else { 1.0 };
        let spread = [0.0f32, 0.35, 0.65, 0.9, 1.0][(k + 1) / 2];
        let len = height * (0.36 + 0.10 * hash(k as i32, 1, 77));
        let frond: Vec<[f32; 2]> = (0..=14)
            .map(|i| {
                let t = i as f32 / 14.0;
                [top[0] + side * len * spread * 1.15 * t + lean * len * 0.2 * t, top[1] - len * (0.9 - 0.55 * spread) * (t * std::f32::consts::PI * 0.5).sin() * (1.0 - 0.2 * t) + len * 0.75 * spread * t * t]
            })
            .collect();
        for (i, pair) in frond.windows(2).enumerate() {
            pen(c, pair, height * 0.026 * (1.0 - i as f32 / 14.0) + 1.0, ink);
        }
        // The leaflets, swept back along the frond and hanging a little.
        for i in 2..14 {
            let (p, q) = (frond[i], frond[i + 1]);
            let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
            let l = (dx * dx + dy * dy).sqrt().max(1e-3);
            let (dx, dy) = (dx / l, dy / l);
            let size = len * 0.11 * (1.0 - i as f32 / 17.0);
            for s in [-1.0f32, 1.0] {
                let tip = [p[0] + (dx * 0.5 - dy * s) * size, p[1] + (dy * 0.5 + dx * s) * size + size * 0.45];
                pen(c, &[p, tip], 1.6, ink);
            }
        }
    }
}

/// A sunset poster, drawn here: a striped sun going down behind the mountains over a neon
/// grid, palms either side, the title in pink.
pub fn poster_outrun(art: &Art, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let horizon = fh * 0.60;
    let mut c = Canvas::new(w, h, rgb(10, 4, 24));
    c.gradient([0.0, 0.0, fw, fh * 0.30], rgb(9, 3, 28), rgb(66, 13, 105));
    c.gradient([0.0, fh * 0.30, fw, fh * 0.52], rgb(66, 13, 105), rgb(222, 52, 128));
    c.gradient([0.0, fh * 0.52, fw, horizon], rgb(222, 52, 128), rgb(255, 140, 66));
    stars(&mut c, [0.0, 0.0, fw, fh * 0.34], 90, 31, 0.8);
    // The sun: half in the mountains' notch, yellow above to pink below, slits cut across its
    // lower half that widen as they go down.
    let (sx, sy, sr) = (fw * 0.5, horizon - fh * 0.035, fw * 0.215);
    c.glow(sx, sy, sr * 1.9, sr * 1.5, rgb(255, 80, 150), 0.5);
    c.each([sx - sr - 2.0, sy - sr - 2.0, sx + sr + 2.0, horizon], |x, y, p| {
        let (dx, dy) = (x - sx, y - sy);
        let d = (dx * dx + dy * dy).sqrt();
        if d >= sr + 1.0 {
            return p;
        }
        let k = (dy / sr + 1.0) * 0.5;
        if dy > 0.0 {
            let band = fh * 0.03;
            let phase = (dy / band).fract();
            if phase < (0.1 + 0.55 * (dy / sr)).clamp(0.0, 0.75) {
                return p;
            }
        }
        blend(p, mix3(lin3(255, 232, 92), lin3(255, 40, 130), k.clamp(0.0, 1.0).powf(0.9)), ((sr - d) + 0.5).clamp(0.0, 1.0))
    });
    // The mountains, a dark ridge with a neon edge, lowest in the middle where the sun is.
    let ridge = |x: f32| {
        let side = smooth(((x - fw * 0.5).abs() / (fw * 0.32)).min(1.0));
        let n = value_noise(x / 55.0, 3.0, 41) * 0.6 + value_noise(x / 18.0, 5.0, 43) * 0.4;
        horizon - fh * (0.012 + (0.04 + 0.13 * n) * (0.12 + 0.88 * side))
    };
    c.each([0.0, 0.0, fw, horizon + 1.0], |x, y, p| {
        let top = ridge(x);
        if y < top - 1.0 {
            p
        } else if y < top + 1.2 {
            blend(p, lin3(255, 80, 190), 0.95)
        } else {
            blend(p, mix3(lin3(30, 8, 54), lin3(14, 4, 30), ((y - top) / (fh * 0.08)).min(1.0)), 1.0)
        }
    });
    // The ground and its glowing horizon.
    c.gradient([0.0, horizon, fw, fh], rgb(26, 5, 46), rgb(5, 1, 12));
    c.glow(fw * 0.5, horizon, fw * 0.75, fh * 0.07, rgb(255, 80, 160), 0.8);
    let cyan = rgb(70, 225, 255);
    for k in 1..=14 {
        let t = k as f32 / 14.0;
        let y = horizon + (fh - horizon) * t.powf(2.3);
        let wide = 1.0 + 2.4 * t;
        c.rect([0.0, y - wide * 1.6, fw, y + wide * 1.6], [cyan[0], cyan[1], cyan[2], 0.10]);
        c.rect([0.0, y - wide * 0.5, fw, y + wide * 0.5], [cyan[0], cyan[1], cyan[2], 0.35 + 0.5 * t]);
    }
    for i in -14..=14 {
        let (top, foot) = ([fw * 0.5 + i as f32 * fw * 0.034, horizon], [fw * 0.5 + i as f32 * fw * 0.44, fh]);
        pen(&mut c, &[top, foot], 5.0, [cyan[0], cyan[1], cyan[2], 0.07]);
        pen(&mut c, &[top, foot], 1.5, [cyan[0], cyan[1], cyan[2], 0.55]);
    }
    // Palms against the sky.
    let ink = rgb(5, 2, 14);
    palm(&mut c, fw * 0.11, fh * 0.92, fh * 0.52, 0.12, ink);
    palm(&mut c, fw * 0.93, fh * 0.9, fh * 0.44, -0.10, ink);
    // The title.
    if let Some(f) = &art.heading {
        for (dx, dy, a) in [(-4.0, 0.0, 0.2), (4.0, 0.0, 0.2), (0.0, 4.0, 0.2), (0.0, -4.0, 0.2), (-2.0, 2.0, 0.3), (2.0, -2.0, 0.3)] {
            spaced(&mut c, f, "NIGHT DRIVE", fw * 0.5 + dx, fh * 0.775 + dy, fh * 0.115, fw * 0.012, rgba(255, 60, 170, a));
        }
        spaced(&mut c, f, "NIGHT DRIVE", fw * 0.5, fh * 0.775, fh * 0.115, fw * 0.012, rgb(255, 238, 250));
    }
    if let Some(f) = &art.small {
        c.text_centred(f, "ORIGINAL MOTION PICTURE SOUNDTRACK", fw * 0.5, fh * 0.915, fh * 0.034, rgb(255, 170, 214));
    }
    wear(&mut c);
    c
}

/// A band's poster, drawn here: a sunburst in acid yellow behind a black disc and a bolt, the
/// name in white over red, the dates along the foot.
pub fn poster_band(art: &Art, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w, h, rgb(12, 12, 12));
    let (cx, cy) = (fw * 0.5, fh * 0.37);
    let reach = fh * 1.6;
    let wedges = 26;
    for k in 0..wedges {
        if k % 2 == 1 {
            continue;
        }
        let (a0, a1) = (k as f32 / wedges as f32 * std::f32::consts::TAU, (k + 1) as f32 / wedges as f32 * std::f32::consts::TAU);
        c.polygon(&[[cx, cy], [cx + a0.cos() * reach, cy + a0.sin() * reach], [cx + a1.cos() * reach, cy + a1.sin() * reach]], rgb(246, 226, 18));
    }
    // Dark at the corners, the poster's ink worn off at its edges.
    c.vignette([0.0, 0.0, fw, fh], 0.55);
    // The disc and its rings, the bolt across it.
    let dr = fw * 0.31;
    c.ellipse(cx, cy, dr * 1.08, dr * 1.08, 0.0, rgb(12, 12, 12));
    c.ring(cx, cy, dr * 1.08, dr * 1.08, fw * 0.012, rgb(246, 244, 232));
    c.ellipse(cx, cy, dr, dr, 0.0, rgb(18, 18, 20));
    c.ring(cx, cy, dr * 0.92, dr * 0.92, fw * 0.006, rgb(246, 226, 18));
    let bolt = |x: f32, y: f32| [cx + x * dr, cy + y * dr];
    for poly in [[(0.05, -0.78), (0.42, -0.78), (0.0, -0.05), (-0.34, -0.05)], [(-0.34, -0.05), (0.30, -0.05), (0.30, 0.04), (-0.34, 0.04)], [(-0.30, 0.04), (0.16, 0.04), (-0.12, 0.80), (-0.44, 0.80)]] {
        let pts: Vec<[f32; 2]> = poly.iter().map(|&(x, y)| bolt(x, y)).collect();
        c.polygon(&pts, rgb(246, 226, 18));
    }
    // The name.
    c.gradient([0.0, fh * 0.68, fw, fh], rgba(0, 0, 0, 0.0), rgba(6, 6, 6, 0.95));
    if let Some(f) = &art.heading {
        let height = fh * 0.12;
        spaced(&mut c, f, "VOLTAGE", fw * 0.5 + fw * 0.012, fh * 0.705 + fw * 0.012, height, fw * 0.014, rgb(220, 30, 30));
        spaced(&mut c, f, "VOLTAGE", fw * 0.5, fh * 0.705, height, fw * 0.014, rgb(248, 246, 238));
        spaced(&mut c, f, "LIVE IN CONCERT", fw * 0.5, fh * 0.832, fh * 0.034, fw * 0.012, rgb(246, 226, 18));
    }
    if let Some(f) = &art.small {
        let ink = rgb(222, 222, 216);
        for (i, line) in ["SEP 12  METRO HALL", "SEP 19  THE DOCKYARD", "OCT 03  CITY ARENA", "OCT 10  TOWN HALL"].iter().enumerate() {
            let col = i % 2;
            let row = i / 2;
            c.text(f, line, fw * (0.10 + 0.43 * col as f32), fh * (0.895 + 0.036 * row as f32), fh * 0.024, ink, false);
        }
        c.text_centred(f, "WORLD TOUR 1997", fw * 0.5, fh * 0.968, fh * 0.017, rgb(150, 150, 146));
    }
    wear(&mut c);
    c
}

/// The wear of a poster that has hung a while: the print's grain, a fold or two, the paper's
/// edge worn pale.
fn wear(c: &mut Canvas) {
    let (fw, fh) = (c.w as f32, c.h as f32);
    c.noise([0.0, 0.0, fw, fh], 0.05, 60.0, 43);
    c.noise([0.0, 0.0, fw, fh], 0.04, 3.0, 47);
    c.rect([0.0, fh * 0.5 - 0.75, fw, fh * 0.5 + 0.75], rgba(255, 255, 255, 0.04));
    c.rect([fw * 0.5 - 0.75, 0.0, fw * 0.5 + 0.75, fh], rgba(0, 0, 0, 0.05));
    let m = fw.min(fh) * 0.006;
    c.rect([0.0, 0.0, fw, m], rgba(232, 228, 214, 0.5));
    c.rect([0.0, fh - m, fw, fh], rgba(232, 228, 214, 0.5));
    c.rect([0.0, 0.0, m, fh], rgba(232, 228, 214, 0.5));
    c.rect([fw - m, 0.0, fw, fh], rgba(232, 228, 214, 0.5));
}

/// A white border and the wear of the wall: creases and a corner curling.
fn frame_poster(c: &mut Canvas) {
    let (fw, fh) = (c.w as f32, c.h as f32);
    let m = fw.min(fh) * 0.018;
    c.rect([0.0, 0.0, fw, m], paper());
    c.rect([0.0, fh - m, fw, fh], paper());
    c.rect([0.0, 0.0, m, fh], paper());
    c.rect([fw - m, 0.0, fw, fh], paper());
    c.noise([0.0, 0.0, fw, fh], 0.05, 60.0, 41);
    // Folds from its time in the box.
    c.rect([0.0, fh * 0.5 - 1.0, fw, fh * 0.5 + 1.0], rgba(255, 255, 255, 0.06));
    c.rect([fw * 0.5 - 1.0, 0.0, fw * 0.5 + 1.0, fh], rgba(0, 0, 0, 0.08));
}

/// A game box's front: `title` over its art, the publisher's stone in the corner.
pub fn box_front(art: &Art, which: usize, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w, h, rgb(10, 10, 12));
    match which {
        // Total Annihilation: the title plate over the battle.
        0 => {
            c.gradient([0.0, 0.0, fw, fh], rgb(28, 30, 36), rgb(4, 4, 6));
            if let Some(b) = &art.battle {
                c.image_cover(b, BATTLE_PAINTING, [0.0, fh * 0.3, fw, fh], 1.0);
                c.gradient([0.0, fh * 0.3, fw, fh * 0.45], rgba(0, 0, 0, 0.9), rgba(0, 0, 0, 0.0));
            }
            if let Some(t) = &art.title {
                c.image_fit(t, TITLE_PLATE, [fw * 0.04, fh * 0.04, fw * 0.96, fh * 0.36], 1.0);
            }
        }
        // The Core Contingency: the green wireframe emblem over the jungle.
        1 => {
            c.gradient([0.0, 0.0, fw, fh], rgb(8, 30, 12), rgb(2, 6, 3));
            if let Some(j) = &art.jungle {
                c.image_cover(j, [0.0, 60.0, 640.0, 480.0], [0.0, fh * 0.35, fw, fh], 0.85);
            }
            if let Some(e) = &art.emblems {
                c.image_fit(e, WIREFRAME_CORE, [fw * 0.25, fh * 0.1, fw * 0.75, fh * 0.42], 1.0);
            }
            if let Some(f) = &art.heading {
                c.text_centred(f, "THE CORE", fw * 0.5, fh * 0.44, fh * 0.07, rgb(150, 240, 120));
                c.text_centred(f, "CONTINGENCY", fw * 0.5, fh * 0.52, fh * 0.07, rgb(150, 240, 120));
            }
        }
        // Battle Tactics: the ARM emblem over a line-up of build pictures.
        _ => {
            c.gradient([0.0, 0.0, fw, fh], rgb(18, 34, 64), rgb(4, 8, 18));
            if let Some(a) = &art.arm {
                c.image(a, EMBLEM, [fw * 0.15, fh * 0.08, fw * 0.85, fh * 0.45], 0.6, [0.8, 1.0, 1.4]);
            }
            for (i, (img, _, _)) in art.units.iter().take(8).enumerate() {
                let (col, row) = (i % 4, i / 4);
                let s = fw * 0.2;
                let (x, y) = (fw * 0.08 + col as f32 * (s + fw * 0.013), fh * 0.62 + row as f32 * (s + fw * 0.013));
                c.image(img, [0.0, 0.0, img.w as f32, img.h as f32], [x, y, x + s, y + s], 1.0, [1.0; 3]);
            }
            if let Some(f) = &art.heading {
                c.text_centred(f, "BATTLE", fw * 0.5, fh * 0.44, fh * 0.07, rgb(230, 210, 140));
                c.text_centred(f, "TACTICS", fw * 0.5, fh * 0.52, fh * 0.07, rgb(230, 210, 140));
            }
        }
    }
    if let Some(d) = &art.cavedog {
        c.image_fit(d, CAVEDOG_STONE, [fw * 0.74, fh * 0.9, fw * 0.96, fh * 0.985], 0.9);
    }
    if let Some(f) = &art.small {
        c.round_rect([fw * 0.04, fh * 0.915, fw * 0.3, fh * 0.965], 4.0, rgb(200, 30, 30));
        c.text(f, "PC CD-ROM", fw * 0.06, fh * 0.925, fh * 0.03, rgb(255, 255, 255), false);
    }
    c.noise([0.0, 0.0, fw, fh], 0.04, 50.0, 51 + which as u32);
    c
}

/// A box's spine: its title running up it.
pub fn box_spine(art: &Art, which: usize, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let colour = [rgb(24, 26, 30), rgb(10, 30, 12), rgb(16, 30, 58)][which.min(2)];
    let mut c = Canvas::new(w, h, colour);
    c.gradient([0.0, 0.0, fw, fh], rgba(255, 255, 255, 0.05), rgba(0, 0, 0, 0.3));
    if let Some(f) = &art.heading {
        let title = ["TOTAL ANNIHILATION", "THE CORE CONTINGENCY", "BATTLE TACTICS"][which.min(2)];
        let height = fw * 0.55;
        let len = Canvas::text_width(f, title, height);
        c.text(f, title, fw * 0.2, fh * 0.5 + len * 0.5, height, rgb(220, 214, 196), true);
    }
    c
}

/// The mouse mat: dark blue cloth with ARM's emblem.
pub fn mouse_mat(art: &Art, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w, h, rgb(20, 34, 70));
    c.noise([0.0, 0.0, fw, fh], 0.1, 2.0, 61);
    if let Some(a) = &art.arm {
        c.image(a, EMBLEM, [fw * 0.1, fh * 0.1, fw * 0.9, fh * 0.9], 0.35, [0.6, 0.8, 1.2]);
    }
    c
}

/// The turbo display's digits ("166", as fast as a desk got in 1997), lit red behind its
/// filter.
pub fn turbo_digits(w: usize, h: usize) -> Canvas {
    let mut c = Canvas::new(w, h, rgb(14, 3, 3));
    let (fw, fh) = (w as f32, h as f32);
    seven_segments(&mut c, &[Some(1), Some(6), Some(6)], [fw * 0.08, fh * 0.05, fw * 0.95, fh * 0.95], false);
    c
}

/// The clock radio's display: the time it tells (a quarter to two in the morning, but for the
/// seasons), its dot for "PM" dark.
pub fn clock_digits(w: usize, h: usize, time: [Option<u8>; 4]) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w, h, rgb(10, 2, 2));
    seven_segments(&mut c, &time, [fw * 0.1, fh * 0.12, fw * 0.92, fh * 0.88], true);
    c
}

/// Seven-segment digits across `r` (None: a digit left dark), lit red, the unlit segments
/// faintly there; `colon` between the second and third.
fn seven_segments(c: &mut Canvas, digits: &[Option<u8>], r: Rect, colon: bool) {
    let (x0, y0, width, height) = (r[0], r[1], r[2] - r[0], r[3] - r[1]);
    let cell = width / digits.len() as f32;
    // Segments: a top, b top right, c bottom right, d bottom, e bottom left, f top left, g middle.
    let lit = |d: Option<u8>| -> [bool; 7] {
        match d {
            Some(0) => [true, true, true, true, true, true, false],
            Some(1) => [false, true, true, false, false, false, false],
            Some(2) => [true, true, false, true, true, false, true],
            Some(3) => [true, true, true, true, false, false, true],
            Some(4) => [false, true, true, false, false, true, true],
            Some(5) => [true, false, true, true, false, true, true],
            Some(6) => [true, false, true, true, true, true, true],
            Some(7) => [true, true, true, false, false, false, false],
            Some(8) => [true; 7],
            Some(9) => [true, true, true, true, false, true, true],
            _ => [false; 7],
        }
    };
    let (on, off) = (rgb(255, 50, 30), rgb(34, 8, 7));
    for (k, d) in digits.iter().enumerate() {
        let x = x0 + k as f32 * cell;
        let (w2, t) = (cell * 0.72, height * 0.075);
        let (top, mid, bottom) = (y0, y0 + height * 0.5, y0 + height);
        let segs = [
            [x + t, top, x + w2 - t, top + t],
            [x + w2 - t, top + t, x + w2, mid - t * 0.5],
            [x + w2 - t, mid + t * 0.5, x + w2, bottom - t],
            [x + t, bottom - t, x + w2 - t, bottom],
            [x, mid + t * 0.5, x + t, bottom - t],
            [x, top + t, x + t, mid - t * 0.5],
            [x + t, mid - t * 0.5, x + w2 - t, mid + t * 0.5],
        ];
        for (s, lit) in segs.iter().zip(lit(*d)) {
            c.rect(*s, if lit { on } else { off });
        }
        if colon && k == 1 {
            let cx = x + cell * 0.86;
            for y in [y0 + height * 0.3, y0 + height * 0.7] {
                c.rect([cx - t * 0.5, y - t * 0.5, cx + t * 0.5, y + t * 0.5], on);
            }
        }
    }
}

/// A month of the calendar: its name and year, the weekday of its first (0 Sunday), its days, the
/// day ringed in red and what is written beside it.
pub struct Month {
    pub name: &'static str,
    pub year: u32,
    pub first: usize,
    pub days: usize,
    pub ringed: usize,
    pub note: &'static str,
}

/// The month the room is in: September 1997, the 30th (the day TA came out) ringed.
pub const SEPTEMBER_1997: Month = Month { name: "SEPTEMBER", year: 1997, first: 1, days: 30, ringed: 30, note: "TA!" };
/// At Christmas: December, the 25th ringed.
pub const DECEMBER_1997: Month = Month { name: "DECEMBER", year: 1997, first: 1, days: 31, ringed: 25, note: "XMAS" };
/// At New Year: January, the 1st ringed.
pub const JANUARY_1998: Month = Month { name: "JANUARY", year: 1998, first: 4, days: 31, ringed: 1, note: "PARTY" };

/// The calendar by the window, open at `month`: a picture of the battle over the month, a day
/// ringed in red marker.
pub fn calendar(art: &Art, w: usize, h: usize, month: &Month) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w, h, paper());
    c.noise([0.0, 0.0, fw, fh], 0.03, 30.0, 71);
    let picture = [fw * 0.06, fh * 0.06, fw * 0.94, fh * 0.43];
    match &art.battle {
        Some(b) => c.image_cover(b, BATTLE_PAINTING, picture, 1.0),
        None => c.gradient(picture, rgb(60, 70, 90), rgb(20, 24, 30)),
    }
    // The spiral binding along the top.
    for i in 0..11 {
        let x = fw * 0.08 + i as f32 * fw * 0.084;
        c.ellipse(x, fh * 0.025, fw * 0.012, fh * 0.009, 0.0, rgb(40, 40, 44));
        c.rect([x - fw * 0.003, 0.0, x + fw * 0.003, fh * 0.025], rgb(150, 150, 156));
    }
    let ink = rgb(36, 34, 32);
    let red = rgb(178, 34, 28);
    if let Some(f) = &art.heading {
        c.text_centred(f, month.name, fw * 0.5, fh * 0.455, fh * 0.042, ink);
        c.text_centred(f, &month.year.to_string(), fw * 0.5, fh * 0.5, fh * 0.03, red);
    }
    // The month: Sundays first.
    let (gx0, gy0, gx1, gy1) = (fw * 0.06, fh * 0.56, fw * 0.94, fh * 0.965);
    let cw = (gx1 - gx0) / 7.0;
    let head = fh * 0.035;
    let ch = (gy1 - gy0 - head) / 5.0;
    for col in 0..=7 {
        let x = gx0 + col as f32 * cw;
        c.rect([x - 0.75, gy0 + head, x + 0.75, gy1], rgba(0, 0, 0, 0.25));
    }
    for row in 0..=5 {
        let y = gy0 + head + row as f32 * ch;
        c.rect([gx0, y - 0.75, gx1, y + 0.75], rgba(0, 0, 0, 0.25));
    }
    if let Some(f) = &art.small {
        for (i, d) in ["S", "M", "T", "W", "T", "F", "S"].iter().enumerate() {
            c.text_centred(f, d, gx0 + cw * (i as f32 + 0.5), gy0 + head * 0.15, head * 0.7, if i == 0 { red } else { ink });
        }
        for day in 1..=month.days {
            let at = day + month.first - 1;
            let (col, row) = (at % 7, at / 7);
            let x = gx0 + col as f32 * cw + cw * 0.1;
            let y = gy0 + head + row as f32 * ch + ch * 0.08;
            c.text(f, &day.to_string(), x, y, ch * 0.32, if col == 0 { red } else { ink }, false);
        }
    }
    // The ringed day, and a word beside it.
    let at = month.ringed + month.first - 1;
    let (col, row) = (at % 7, at / 7);
    let (cx, cy) = (gx0 + (col as f32 + 0.5) * cw, gy0 + head + (row as f32 + 0.5) * ch);
    c.ring(cx, cy, cw * 0.52, ch * 0.5, fw * 0.009, rgba(200, 20, 20, 0.9));
    if let Some(f) = &art.heading {
        // (To the left of a day at the week's end, where the right has no room.)
        let width = Canvas::text_width(f, month.note, ch * 0.42);
        let x = if col >= 5 { cx - cw * 0.55 - width } else { cx + cw * 0.55 };
        c.text(f, month.note, x, cy - ch * 0.15, ch * 0.42, rgba(200, 20, 20, 0.9), false);
    }
    c
}

/// The banner's flags (a 4 x 4 picture, each cell a flag's cloth with its letter or star in the
/// upper middle, where the triangle is wide): HAPPY NEW YEAR.
pub fn banner(art: &Art, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let (cw, ch) = (fw / 4.0, fh / 4.0);
    let cloth = [rgb(210, 40, 58), rgb(238, 186, 54), rgb(50, 106, 214), rgb(226, 70, 150), rgb(46, 168, 96)];
    let mut c = Canvas::new(w, h, rgb(240, 240, 236));
    for (i, letter) in "HAPPY*NEW*YEAR".chars().enumerate() {
        let (col, row) = (i % 4, i / 4);
        let r = [col as f32 * cw, row as f32 * ch, (col + 1) as f32 * cw, (row + 1) as f32 * ch];
        c.rect(r, cloth[i % cloth.len()]);
        c.gradient(r, rgba(255, 255, 255, 0.2), rgba(0, 0, 0, 0.14));
        let ink = if i % cloth.len() == 1 { rgb(60, 36, 20) } else { rgb(252, 250, 244) };
        let middle = r[0] + cw * 0.5;
        if letter == '*' {
            // A five-pointed star.
            let (cy, outer) = (r[1] + ch * 0.34, ch * 0.2);
            let points: Vec<[f32; 2]> = (0..10).map(|k| { let a = -std::f32::consts::FRAC_PI_2 + k as f32 * std::f32::consts::PI / 5.0; let rr = if k % 2 == 0 { outer } else { outer * 0.42 }; [middle + a.cos() * rr, cy + a.sin() * rr] }).collect();
            let centre = [middle, cy];
            for k in 0..10 {
                c.polygon(&[centre, points[k], points[(k + 1) % 10]], ink);
            }
        } else if let Some(f) = &art.heading {
            let height = ch * 0.3;
            let width = Canvas::text_width(f, &letter.to_string(), height);
            c.text(f, &letter.to_string(), middle - width * 0.5, r[1] + ch * 0.12, height, ink, false);
        }
    }
    c
}

/// The game's manual: TA's title over the battle, as on the box.
pub fn manual(art: &Art, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w, h, rgb(12, 12, 14));
    c.gradient([0.0, 0.0, fw, fh], rgb(30, 32, 38), rgb(6, 6, 8));
    if let Some(b) = &art.battle {
        c.image_cover(b, BATTLE_PAINTING, [0.0, fh * 0.34, fw, fh * 0.84], 0.95);
        c.gradient([0.0, fh * 0.34, fw, fh * 0.46], rgba(0, 0, 0, 0.9), rgba(0, 0, 0, 0.0));
        c.gradient([0.0, fh * 0.72, fw, fh * 0.84], rgba(0, 0, 0, 0.0), rgba(0, 0, 0, 0.9));
    }
    if let Some(t) = &art.title {
        c.image_fit(t, TITLE_PLATE, [fw * 0.05, fh * 0.04, fw * 0.95, fh * 0.33], 1.0);
    }
    if let Some(f) = &art.heading {
        c.text_centred(f, "GAME MANUAL", fw * 0.5, fh * 0.87, fh * 0.045, rgb(222, 200, 136));
    }
    if let Some(d) = &art.cavedog {
        c.image_fit(d, CAVEDOG_STONE, [fw * 0.78, fh * 0.92, fw * 0.96, fh * 0.99], 0.8);
    }
    // Handled: the corners scuffed pale, the cover's sheen worn.
    c.noise([0.0, 0.0, fw, fh], 0.06, 40.0, 81);
    c.vignette([0.0, 0.0, fw, fh], -0.25);
    c
}

/// The insert in the top CD case: the ARM and CORE emblems facing across TA's title.
pub fn cd_insert(art: &Art, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w, h, rgb(8, 8, 10));
    c.gradient([0.0, 0.0, fw, fh], rgb(22, 24, 30), rgb(4, 4, 6));
    if let Some(a) = &art.arm {
        c.image(a, EMBLEM, [fw * 0.02, fh * 0.42, fw * 0.48, fh * 0.98], 0.75, [0.8, 0.95, 1.3]);
    }
    if let Some(core) = &art.core {
        c.image(core, EMBLEM, [fw * 0.52, fh * 0.42, fw * 0.98, fh * 0.98], 0.75, [1.3, 0.8, 0.7]);
    }
    if let Some(t) = &art.title {
        c.image_fit(t, TITLE_PLATE, [fw * 0.06, fh * 0.04, fw * 0.94, fh * 0.42], 1.0);
    }
    if let Some(f) = &art.small {
        c.text_centred(f, "PC CD-ROM", fw * 0.5, fh * 0.92, fh * 0.04, rgb(210, 210, 214));
    }
    c
}

/// A pen's line through `points`, `width` pixels wide.
fn pen(c: &mut Canvas, points: &[[f32; 2]], width: f32, ink: Rgba) {
    for pair in points.windows(2) {
        let ([ax, ay], [bx, by]) = (pair[0], pair[1]);
        let (dx, dy) = (bx - ax, by - ay);
        let length = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (nx, ny) = (-dy / length * width * 0.5, dx / length * width * 0.5);
        c.polygon(&[[ax + nx, ay + ny], [bx + nx, by + ny], [bx - nx, by - ny], [ax - nx, ay - ny]], ink);
        c.ellipse(bx, by, width * 0.5, width * 0.5, 0.5, ink);
    }
}

/// An arrow in pen from `from` to `to`, bowed to one side by `bow` (of its length), its head
/// at `to`.
fn pen_arrow(c: &mut Canvas, from: [f32; 2], to: [f32; 2], bow: f32, width: f32, ink: Rgba) {
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let control = [(from[0] + to[0]) * 0.5 - dy * bow, (from[1] + to[1]) * 0.5 + dx * bow];
    let points: Vec<[f32; 2]> = (0..=24)
        .map(|k| {
            let t = k as f32 / 24.0;
            let u = 1.0 - t;
            [u * u * from[0] + 2.0 * u * t * control[0] + t * t * to[0], u * u * from[1] + 2.0 * u * t * control[1] + t * t * to[1]]
        })
        .collect();
    pen(c, &points, width, ink);
    // Its head along the curve's last stretch.
    let [px, py] = points[points.len() - 4];
    let (ex, ey) = (to[0] - px, to[1] - py);
    let length = (ex * ex + ey * ey).sqrt().max(1e-3);
    let (ux, uy) = (ex / length, ey / length);
    let size = width * 6.0;
    for side in [-1.0f32, 1.0] {
        let back = [to[0] - ux * size + side * -uy * size * 0.6, to[1] - uy * size + side * ux * size * 0.6];
        pen(c, &[back, to], width, ink);
    }
}

/// The battle plan: a colour print of a map from the install (`Art::plan_map`), paler than the
/// screen showed it, its name printed over it, and a battle planned on it in biro: our start
/// ringed in blue, the enemy's crossed out in red, the attack's arrows across the map and a
/// feint round the side.
pub fn plan(art: &Art, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w, h, paper());
    c.noise([0.0, 0.0, fw, fh], 0.025, 5.0, 101);
    let print = [fw * 0.07, fh * 0.13, fw * 0.93, fh * 0.93];
    let mut starts = [[0.25, 0.7], [0.75, 0.3]];
    if let Some((map, name, at)) = &art.plan_map {
        // Fitted inside the margins at its own shape.
        let (pw, ph) = (print[2] - print[0], print[3] - print[1]);
        let scale = (pw / map.w as f32).min(ph / map.h as f32);
        let (mw, mh) = (map.w as f32 * scale, map.h as f32 * scale);
        let dst = [print[0] + (pw - mw) / 2.0, print[1] + (ph - mh) / 2.0, print[0] + (pw + mw) / 2.0, print[1] + (ph + mh) / 2.0];
        c.image(map, [0.0, 0.0, map.w as f32, map.h as f32], dst, 0.9, [1.0, 1.0, 1.0]);
        // An inkjet's paler colours, and its bands.
        c.rect(dst, rgba(232, 228, 214, 0.18));
        for k in 0..((mh / 9.0) as usize) {
            let y = dst[1] + k as f32 * 9.0;
            c.rect([dst[0], y, dst[2], y + 1.0], rgba(232, 228, 214, 0.08));
        }
        starts = at.map(|[x, y]| [dst[0] + x * mw, dst[1] + y * mh]);
        if let Some(f) = &art.small {
            let height = fh * 0.045;
            c.text(f, &name.to_uppercase(), print[0], fh * 0.045, height, rgb(40, 40, 46), false);
        }
    } else {
        starts = starts.map(|[x, y]| [print[0] + x * (print[2] - print[0]), print[1] + y * (print[3] - print[1])]);
    }
    let blue = rgba(30, 48, 150, 0.9);
    let red = rgba(176, 26, 22, 0.88);
    let width = fw * 0.006;
    let [ours, theirs] = starts;
    // Ours ringed twice round, theirs crossed out.
    c.ring(ours[0], ours[1], fw * 0.045, fw * 0.042, width, blue);
    c.ring(ours[0] + fw * 0.004, ours[1] - fw * 0.003, fw * 0.052, fw * 0.047, width * 0.8, blue);
    let x = fw * 0.03;
    pen(&mut c, &[[theirs[0] - x, theirs[1] - x], [theirs[0] + x, theirs[1] + x]], width * 1.2, red);
    pen(&mut c, &[[theirs[0] + x, theirs[1] - x], [theirs[0] - x, theirs[1] + x]], width * 1.2, red);
    // The attack, straight at them, and a feint bowed round the side; stopped short of the
    // marks.
    let toward = |a: [f32; 2], b: [f32; 2], from: f32, to: f32| [[a[0] + (b[0] - a[0]) * from, a[1] + (b[1] - a[1]) * from], [a[0] + (b[0] - a[0]) * to, a[1] + (b[1] - a[1]) * to]];
    let [a, b] = toward(ours, theirs, 0.16, 0.82);
    pen_arrow(&mut c, a, b, 0.06, width, red);
    let [a, b] = toward(ours, theirs, 0.2, 0.78);
    pen_arrow(&mut c, a, b, -0.3, width * 0.85, blue);
    // A tick in the margin and the paper's handling.
    pen(&mut c, &[[fw * 0.84, fh * 0.06], [fw * 0.86, fh * 0.085], [fw * 0.9, fh * 0.03]], width, blue);
    c.vignette([0.0, 0.0, fw, fh], -0.12);
    c
}

/// The computer's case badge: TA Reborn's emblem as a fan's decal on the case's own beige (a
/// sticker over the maker's own, in 1997: no frame, no metal to catch the eye).
pub fn badge(art: &Art, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w, h, rgb(206, 201, 183));
    let e = &art.emblem;
    let side = fw.min(fh) * 0.9;
    let dst = [(fw - side) / 2.0, (fh - side) / 2.0, (fw + side) / 2.0, (fh + side) / 2.0];
    c.image(e, [0.0, 0.0, e.w as f32, e.h as f32], dst, 1.0, [1.0, 1.0, 1.0]);
    c
}

/// A sticky note on the monitor: the cheats a friend swore by, in biro.
pub fn sticky_note(art: &Art, w: usize, h: usize) -> Canvas {
    let (fw, fh) = (w as f32, h as f32);
    let mut c = Canvas::new(w, h, rgb(246, 226, 104));
    c.gradient([0.0, 0.0, fw, fh * 0.16], rgb(232, 210, 92), rgb(246, 226, 104));
    c.noise([0.0, 0.0, fw, fh], 0.04, 12.0, 91);
    let ink = rgba(28, 44, 140, 0.92);
    if let Some(f) = art.small.as_ref().or(art.heading.as_ref()) {
        let lines = ["CHEATS!", "+ATM", "+NOWISEE", "+RADAR", "+DOUBLESHOT"];
        for (i, line) in lines.iter().enumerate() {
            // (A hand's lines wander.)
            let x = fw * (0.1 + 0.02 * (i as f32 * 1.7).sin());
            let y = fh * (0.12 + i as f32 * 0.165);
            let height = if i == 0 { fh * 0.13 } else { fh * 0.11 };
            let width = Canvas::text_width(f, line, height).max(1.0);
            let height = height * (fw * 0.84 / width).min(1.0);
            c.text(f, line, x, y, height, ink, false);
        }
        // Underlined twice.
        c.rect([fw * 0.1, fh * 0.255, fw * 0.62, fh * 0.262], ink);
        c.rect([fw * 0.12, fh * 0.272, fw * 0.6, fh * 0.278], ink);
    }
    c
}

/// The keys' legends: each key's top in its colour with its legend printed in dark grey, top
/// left (`keys`, from the model: where each key's top is on the picture), `size` pixels square.
pub fn key_legends(art: &Art, keys: &[KeyLegend], size: usize) -> Canvas {
    let s = size as f32;
    let mut c = Canvas::new(size, size, rgb(168, 160, 138));
    // Large, heavy and dark, as a real board's are: from the seat a key's top is a few pixels
    // deep, and thin legends a quarter of the cap high averaged away to blank caps.
    let ink = rgb(40, 38, 34);
    for k in keys {
        let r = [k.cell[0] * s, k.cell[1] * s, k.cell[2] * s, k.cell[3] * s];
        c.rect(r, [k.colour[0], k.colour[1], k.colour[2], 1.0]);
        let ch = r[3] - r[1];
        let lines: Vec<&str> = k.label.split('\n').collect();
        let height = ch * if lines.len() > 1 { 0.34 } else { 0.44 };
        let margin = ch * 0.11;
        // (Each line struck twice, a little apart: TA's fonts have hairline strokes.)
        let bold = (height * 0.08).max(1.0);
        for (i, line) in lines.iter().enumerate() {
            // One line: top left; two: the shifted sign above, the key's own below.
            let y = if i == 0 { r[1] + margin } else { r[3] - margin - height };
            let x = r[0] + margin;
            match *line {
                "<up>" | "<down>" | "<left>" | "<right>" | "<back>" => arrow(&mut c, line, [x, y, x + height * 1.6, y + height], ink),
                text => {
                    let Some(f) = art.heading.as_ref().or(art.small.as_ref()) else { continue };
                    let room = (r[2] - r[0] - margin * 2.0 - bold).max(1.0);
                    let width = Canvas::text_width(f, text, height).max(1.0);
                    let h = height * (room / width).min(1.0);
                    for dx in [0.0, bold] {
                        c.text(f, text, x + dx, y + (height - h) * 0.5, h, ink, false);
                    }
                }
            }
        }
    }
    c
}

/// An arrow in `r` pointing as `which` says (`<back>`: left, with a tail as long as the key).
fn arrow(c: &mut Canvas, which: &str, r: Rect, ink: Rgba) {
    let (cx, cy) = ((r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5);
    let k = (r[3] - r[1]) * 0.5;
    let t = k * 0.22;
    let (head, shaft): (Vec<[f32; 2]>, Rect) = match which {
        "<up>" => (vec![[cx, cy - k], [cx + k * 0.7, cy - k * 0.1], [cx - k * 0.7, cy - k * 0.1]], [cx - t, cy - k * 0.2, cx + t, cy + k]),
        "<down>" => (vec![[cx, cy + k], [cx - k * 0.7, cy + k * 0.1], [cx + k * 0.7, cy + k * 0.1]], [cx - t, cy - k, cx + t, cy + k * 0.2]),
        "<right>" => (vec![[cx + k, cy], [cx + k * 0.1, cy + k * 0.7], [cx + k * 0.1, cy - k * 0.7]], [cx - k, cy - t, cx + k * 0.2, cy + t]),
        _ => {
            let tail = if which == "<back>" { r[2] + k * 2.0 } else { cx + k };
            (vec![[r[0], cy], [r[0] + k * 0.9, cy - k * 0.7], [r[0] + k * 0.9, cy + k * 0.7]], [r[0] + k * 0.8, cy - t, tail, cy + t])
        }
    };
    c.polygon(&head, ink);
    c.rect(shaft, ink);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The posters drawn here need nothing of the install: without it they have no lettering,
    /// but they are pictures (and every pixel of them a number: a stray NaN is a black spot).
    #[test]
    fn the_drawn_posters_are_pictures_without_the_install() {
        let art = Art::empty();
        for (name, canvas) in [("space", poster_space(&art, 240, 340)), ("outrun", poster_outrun(&art, 340, 270)), ("band", poster_band(&art, 250, 340))] {
            assert!(canvas.px.iter().all(|p| p.iter().all(|c| c.is_finite())), "{name} has a pixel that is not a number");
            let luma = |p: &Rgba| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
            let lo = canvas.px.iter().map(luma).fold(f32::MAX, f32::min);
            let hi = canvas.px.iter().map(luma).fold(f32::MIN, f32::max);
            assert!(hi - lo > 0.4, "{name} is flat: {lo}..{hi}");
        }
    }
}
