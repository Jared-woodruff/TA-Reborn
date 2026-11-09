//! GAF sprite atlas and instanced sprite drawing (feature billboards, decals, effects).

use bytemuck::{Pod, Zeroable};
use ta_formats::gaf::{GafFrame, ReducedFrame};
use ta_formats::image::Palette;

use crate::util::{DEPTH_FORMAT, depth_state};

/// Vertices of a lying sprite: 8 x 8 cells of two triangles (see `vs_lying` in sprite.wgsl).
const LYING_VERTICES: u32 = 8 * 8 * 6;
/// Vertices of a decal: up to 4 x 4 cells of two triangles, draped over the ground by the
/// perspective camera (see `vs_decal` in sprite.wgsl).
const DECAL_VERTICES: u32 = 4 * 4 * 6;

/// Side the atlas starts at; it grows as big mods fill it (see [`SpriteAtlas::grow`]).
const ATLAS_START: u32 = 4096;
/// Largest side when the device's is not known (what every device takes).
const ATLAS_MAX_DEFAULT: u32 = 8192;
const PAD: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default)]
pub struct FrameInfo {
    pub uv: [f32; 4],
    /// Size (w, h) and origin (x, y) in pixels.
    pub geom: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct SpriteInstance {
    pub pos: [f32; 3],
    pub frame: u32,
    /// alpha, scale, brightness, rotation
    pub params: [f32; 4],
    pub tint: [f32; 4],
}

impl SpriteInstance {
    pub fn new(pos: [f32; 3], frame: SpriteId) -> Self {
        SpriteInstance { pos, frame: frame.0, params: [1.0, 1.0, 1.0, 0.0], tint: [1.0, 1.0, 1.0, 1.0] }
    }

    pub fn alpha(mut self, a: f32) -> Self {
        self.params[0] = a;
        self
    }

    pub fn scale(mut self, s: f32) -> Self {
        self.params[1] = s;
        self
    }

    pub fn brightness(mut self, b: f32) -> Self {
        self.params[2] = b;
        self
    }

    pub fn rotation(mut self, r: f32) -> Self {
        self.params[3] = r;
        self
    }

    pub fn tint(mut self, rgb: [f32; 3]) -> Self {
        self.tint = [rgb[0], rgb[1], rgb[2], self.tint[3]];
        self
    }

    /// Additive effects: how much (0..1) of what lies behind the sprite it hides, as a
    /// fireball does (perspective camera; 0 = plain light, the default).
    pub fn cover(mut self, c: f32) -> Self {
        self.tint[3] = 1.0 - c.clamp(0.0, 1.0);
        self
    }

    /// Billboards: how much (0..1) the sprite sways in the wind, as a tree does (perspective
    /// camera; 0 = standing still, the default).
    pub fn sway(mut self, s: f32) -> Self {
        self.tint[3] = 1.0 - s.clamp(0.0, 1.0);
        self
    }

    /// Billboards: how much (0..1) of it is drawn, less as the 3D model standing in for it takes
    /// over (see `foliage`); its contact occlusion and its reflection stay. (For effect sprites,
    /// their brightness.)
    pub fn shown(mut self, s: f32) -> Self {
        self.params[2] = s.clamp(0.0, 1.0);
        self
    }

    /// Decals: how rich (0..1) the metal deposit it shows is; its ore glints in the sun
    /// (perspective camera; 0 = no ore, the default).
    pub fn ore(mut self, o: f32) -> Self {
        self.tint[3] = 1.0 - o.clamp(0.0, 1.0);
        self
    }
}

/// Whether a feature standing `height` tall on a footprint `footprint` squares across (its
/// longer side), whose painting reaches `above` and `below` its anchor, was painted lying (a
/// car, a pipe, a barrier, a ruin): TA painted every feature from its own camera high above, so
/// a low thing's painting is mostly its top, round its footprint; stood on end before a low
/// camera it would be a card twice as tall as the thing. It is low (or no taller than half again
/// its footprint is long, and the painting lies round its anchor rather than rising from it, as
/// a tree's or a tower's does).
pub fn painted_lying(height: f32, footprint: u32, above: f32, below: f32) -> bool {
    let round_anchor = above <= 1.4 * below.max(1.0);
    height <= 20.0 || (height <= 60.0 && height <= 1.5 * footprint as f32 * 16.0 && round_anchor)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpriteId(pub u32);

/// Which pipeline a sprite batch uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpriteLayer {
    /// Flat on the ground, translucent (drawn right after terrain).
    Decal,
    /// Features painted lying (see [`painted_lying`]) under the perspective camera: laid over
    /// the ground as the ground's own paint is, lit as it is (drawn after the decals).
    Lying,
    /// Upright, alpha-tested, depth-writing.
    Billboard,
    /// Upright, additive HDR (explosions, flashes).
    Effect,
    /// Upright, alpha-blended (smoke, dust).
    Smoke,
}

/// CPU-side shelf packer; frames are added during loading, then uploaded once. It starts at
/// 4096 × 4096 and grows when full, taller and then wider, 4096 texels at a time, up to the
/// largest texture the graphics card takes (see [`SpriteRenderer::reset_atlas`]); past that,
/// frames are left out and the log says so once.
pub struct SpriteAtlas {
    pixels: Vec<u8>,
    /// Texels across and down.
    width: u32,
    height: u32,
    /// The largest either may grow to, and by how much at a time (its first side: a big mod's
    /// atlas, a little over 8192 × 16384, is 12288 × 16384 rather than twice that).
    max_side: u32,
    step: u32,
    frames: Vec<FrameInfo>,
    /// Each frame's texels (x, y, width, height): its uvs follow the atlas's size.
    rects: Vec<[u32; 4]>,
    /// The column shelves fill (left edge, width): the whole atlas until it widens, then
    /// each widening's new strip on the right.
    column: (u32, u32),
    cursor: (u32, u32),
    shelf_height: u32,
    dirty: bool,
    /// A frame was left out (said once).
    full: bool,
    /// Outlined paintings (trees, rocks, ruins) are calmed as they are added, their palette's
    /// static gone (`model::calm_sprite`): with HD surfaces, as the ground's dither is.
    pub calm_paintings: bool,
}

impl Default for SpriteAtlas {
    fn default() -> Self {
        SpriteAtlas::new(ATLAS_START, ATLAS_MAX_DEFAULT)
    }
}

impl SpriteAtlas {
    /// An empty atlas `start` texels square that may grow to `max_side` a side.
    pub fn new(start: u32, max_side: u32) -> Self {
        let side = start.min(max_side).max(1);
        SpriteAtlas {
            pixels: vec![0; side as usize * side as usize * 4],
            width: side,
            height: side,
            max_side: max_side.max(side),
            step: side,
            frames: vec![FrameInfo::default()],
            rects: vec![[0; 4]],
            column: (0, side),
            cursor: (0, 0),
            shelf_height: 0,
            dirty: true,
            full: false,
            calm_paintings: false,
        }
    }

    /// Texels across and down.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The largest side it may grow to (the largest texture the graphics card takes).
    pub fn max_side(&self) -> u32 {
        self.max_side
    }

    /// Makes room: a step taller, or once as tall as it may be, a step wider (the new strip on
    /// the right a column of shelves of its own). False when it is as big as it may be.
    fn grow(&mut self) -> bool {
        if self.height < self.max_side {
            self.height = (self.height + self.step).min(self.max_side);
            self.pixels.resize(self.width as usize * self.height as usize * 4, 0);
        } else if self.width < self.max_side {
            let width = (self.width + self.step).min(self.max_side);
            let mut pixels = vec![0; width as usize * self.height as usize * 4];
            let (old, new) = (self.width as usize * 4, width as usize * 4);
            for (row, dst) in self.pixels.chunks_exact(old).zip(pixels.chunks_exact_mut(new)) {
                dst[..old].copy_from_slice(row);
            }
            self.pixels = pixels;
            self.column = (self.width, width - self.width);
            self.cursor = (self.width, 0);
            self.shelf_height = 0;
            self.width = width;
        } else {
            return false;
        }
        // Every frame's uvs for the new size.
        let (w, h) = (self.width as f32, self.height as f32);
        for (f, r) in self.frames.iter_mut().zip(&self.rects).skip(1) {
            f.uv = [r[0] as f32 / w, r[1] as f32 / h, (r[0] + r[2]) as f32 / w, (r[1] + r[3]) as f32 / h];
        }
        log::info!("sprite atlas grown to {} x {}", self.width, self.height);
        true
    }

    /// Adds a GAF frame. `shadow` renders every opaque pixel as translucent black (TA's
    /// painted shadows use arbitrary dark indices), its edge softened (see `soften`) and faded
    /// out where its frame cuts it (see `fade_cuts`). Returns None if the atlas is full. (The colours of its edge are carried two
    /// texels out into the transparent ones round it, as far as the enlarged paint's spline
    /// reaches, so filtering, which mixes the two, fades its edge out rather than through a dark
    /// rim: a translucent decal's, a scar's, up close.)
    pub fn add_gaf(&mut self, frame: &GafFrame, palette: &Palette, shadow: bool) -> Option<SpriteId> {
        let img = &frame.image;
        let mut rgba: Vec<u8> = img
            .pixels
            .iter()
            .zip(&img.opaque)
            .flat_map(|(&p, &o)| {
                if !o {
                    [0, 0, 0, 0]
                } else if shadow {
                    [0, 0, 0, 255]
                } else {
                    palette.rgba(p)
                }
            })
            .collect();
        if shadow {
            let (w, h) = (img.width as i32, img.height as i32);
            let cut = cut_sides(w, h, &rgba);
            soften(w, h, &mut rgba);
            fade_cuts(w, h, &mut rgba, cut, SHADOW_CUT_FADE);
        } else {
            bleed(img.width as i32, img.height as i32, &mut rgba, 2);
        }
        self.add_rgba(img.width, img.height, &rgba, frame.origin_x as f32, frame.origin_y as f32)
    }

    /// Adds a GAF frame for a decal lying on the ground (metal deposits, craters, scars, ground
    /// plants): the colours of its edge are carried out into the transparent texels round it, so
    /// filtering fades the edge into the ground rather than through a dark rim, and a patch's
    /// outermost two texels fade out (TA painted its patches on its own ground, whose colour rings
    /// them; see `feather`). A `plant` (a frond, a Mars plant: painted leaf by leaf on nothing)
    /// keeps them.
    pub fn add_gaf_decal(&mut self, frame: &GafFrame, palette: &Palette, plant: bool) -> Option<SpriteId> {
        let img = &frame.image;
        let mut rgba: Vec<u8> = img.pixels.iter().zip(&img.opaque).flat_map(|(&p, &o)| if o { palette.rgba(p) } else { [0, 0, 0, 0] }).collect();
        let (w, h) = (img.width as i32, img.height as i32);
        bleed(w, h, &mut rgba, 2);
        if !plant {
            feather(w, h, &mut rgba);
        }
        self.add_rgba(img.width, img.height, &rgba, frame.origin_x as f32, frame.origin_y as f32)
    }

    /// Adds a GAF frame for an alpha-tested billboard (trees, rocks): its alpha channel holds
    /// the distance to the sprite's outline instead of TA's on/off mask, so the filtered
    /// outline stays smooth however close the camera gets (and anti-aliased with MSAA),
    /// and the colours of the edge are carried outwards so filtering never pulls in black.
    pub fn add_gaf_outlined(&mut self, frame: &GafFrame, palette: &Palette) -> Option<SpriteId> {
        let img = &frame.image;
        let mut rgba: Vec<u8> = img.pixels.iter().zip(&img.opaque).flat_map(|(&p, &o)| if o { palette.rgba(p) } else { [0, 0, 0, 0] }).collect();
        if self.calm_paintings {
            crate::model::calm_sprite(img.width as usize, img.height as usize, &mut rgba);
        }
        outline(img.width, img.height, &mut rgba);
        self.add_rgba(img.width, img.height, &rgba, frame.origin_x as f32, frame.origin_y as f32)
    }

    /// Adds an animation frame read reduced (`GafReader::reduced`: a mod's explosion painted
    /// thousands of pixels across), drawn as big as it was painted, each of its texels `scale`
    /// of its pixels across, its origin where it was. Its colours are carried out into the
    /// transparent texels round it as [`Self::add_gaf`]'s are (from every texel it covers at
    /// all: its edge is soft), and the transparent margin beyond them is left out. A frame
    /// with nothing opaque is one transparent texel, so its animation keeps its length.
    pub fn add_reduced(&mut self, frame: &ReducedFrame) -> Option<SpriteId> {
        let (w, h) = (frame.width as i32, frame.height as i32);
        if w == 0 || h == 0 {
            return None;
        }
        let (origin, scale) = ([frame.origin_x as f32, frame.origin_y as f32], frame.scale as f32);
        let mut rgba = frame.rgba.clone();
        const RINGS: i32 = 2;
        bleed_from(w, h, &mut rgba, RINGS, 1);
        let covered = |i: i32| rgba[(i * 4 + 3) as usize] > 0;
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, -1, -1);
        for i in (0..w * h).filter(|&i| covered(i)) {
            let (x, y) = (i % w, i / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        if x1 < 0 {
            return self.add_rgba_drawn(1, 1, &[0; 4], origin, [scale, scale]);
        }
        let (x0, y0, x1, y1) = ((x0 - RINGS).max(0), (y0 - RINGS).max(0), (x1 + RINGS).min(w - 1), (y1 + RINGS).min(h - 1));
        let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
        let mut cropped = Vec::with_capacity((cw * ch * 4) as usize);
        for y in y0..=y1 {
            cropped.extend_from_slice(&rgba[((y * w + x0) * 4) as usize..((y * w + x1 + 1) * 4) as usize]);
        }
        let origin = [origin[0] - x0 as f32 * scale, origin[1] - y0 as f32 * scale];
        self.add_rgba_drawn(cw as u32, ch as u32, &cropped, origin, [cw as f32 * scale, ch as f32 * scale])
    }

    pub fn add_rgba(&mut self, w: u32, h: u32, rgba: &[u8], origin_x: f32, origin_y: f32) -> Option<SpriteId> {
        self.add_rgba_drawn(w, h, rgba, [origin_x, origin_y], [w as f32, h as f32])
    }

    /// As [`Self::add_rgba`], its `w` x `h` texels drawn `drawn` pixels (world units at scale
    /// 1) across and down, its `origin` in those pixels.
    fn add_rgba_drawn(&mut self, w: u32, h: u32, rgba: &[u8], origin: [f32; 2], drawn: [f32; 2]) -> Option<SpriteId> {
        if w == 0 || h == 0 {
            return None;
        }
        let (pw, ph) = (w + PAD * 2, h + PAD * 2);
        loop {
            let (left, wide) = self.column;
            if self.cursor.0 + pw > left + wide {
                self.cursor = (left, self.cursor.1 + self.shelf_height);
                self.shelf_height = 0;
            }
            if self.cursor.1 + ph <= self.height && self.cursor.0 + pw <= left + wide {
                break;
            }
            if !self.grow() {
                if !self.full {
                    self.full = true;
                    log::warn!(
                        "sprite atlas full at {} x {} (the largest this graphics card takes): a {w} x {h} frame and any more are left out",
                        self.width,
                        self.height
                    );
                }
                return None;
            }
        }
        let (x0, y0) = (self.cursor.0 + PAD, self.cursor.1 + PAD);
        // Copy with edge replication into the padding so bilinear filtering never bleeds.
        for y in 0..ph {
            let sy = (y as i32 - PAD as i32).clamp(0, h as i32 - 1) as u32;
            for x in 0..pw {
                let sx = (x as i32 - PAD as i32).clamp(0, w as i32 - 1) as u32;
                let src = ((sy * w + sx) * 4) as usize;
                let dst = ((self.cursor.1 + y) as usize * self.width as usize + (self.cursor.0 + x) as usize) * 4;
                let mut px = [rgba[src], rgba[src + 1], rgba[src + 2], rgba[src + 3]];
                // Padding texels are fully transparent copies of the edge colour.
                if x < PAD || y < PAD || x >= w + PAD || y >= h + PAD {
                    px[3] = 0;
                }
                self.pixels[dst..dst + 4].copy_from_slice(&px);
            }
        }
        self.cursor.0 += pw;
        self.shelf_height = self.shelf_height.max(ph);
        let (aw, ah) = (self.width as f32, self.height as f32);
        self.frames.push(FrameInfo {
            uv: [x0 as f32 / aw, y0 as f32 / ah, (x0 + w) as f32 / aw, (y0 + h) as f32 / ah],
            geom: [drawn[0], drawn[1], origin[0], origin[1]],
        });
        self.rects.push([x0, y0, w, h]);
        self.dirty = true;
        Some(SpriteId(self.frames.len() as u32 - 1))
    }

    pub fn frame(&self, id: SpriteId) -> &FrameInfo {
        &self.frames[id.0 as usize]
    }

    /// Width in pixels (world units at scale 1) of frame `index`, if it exists.
    pub(crate) fn frame_width(&self, index: u32) -> Option<f32> {
        self.frames.get(index as usize).map(|f| f.geom[0])
    }
}

/// Distance (in texels) the outline alpha spans on either side of the edge.
const OUTLINE_RANGE: f32 = 3.0;
/// How far out the outline sits, in texels: a smooth outline cuts the corners of TA's
/// pixel staircases, which thins one-texel twigs and needles into dots; this keeps them
/// whole (and broad shapes as full as the original's squares).
const OUTLINE_BIAS: f32 = 0.35;

/// Rewrites an on/off alpha mask as the signed distance to the outline (0.5 on it, 1 well
/// inside, 0 well outside) and spreads the edge colours into the transparent texels around.
fn outline(w: u32, h: u32, rgba: &mut [u8]) {
    let (w, h) = (w as i32, h as i32);
    let inside = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h && rgba[((y * w + x) * 4 + 3) as usize] >= 128;
    let reach = OUTLINE_RANGE.ceil() as i32 + 1;
    let mut alpha = vec![0u8; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let here = inside(x, y);
            // The nearest texel on the other side of the outline (off the sprite counts as outside).
            let mut nearest = f32::MAX;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    if inside(x + dx, y + dy) != here {
                        nearest = nearest.min(((dx * dx + dy * dy) as f32).sqrt());
                    }
                }
            }
            let distance = (nearest - 0.5).min(OUTLINE_RANGE);
            let signed = if here { distance } else { -distance } + OUTLINE_BIAS;
            alpha[(y * w + x) as usize] = ((0.5 + signed / (2.0 * OUTLINE_RANGE)).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    bleed(w, h, rgba, reach);
    for (i, a) in alpha.into_iter().enumerate() {
        rgba[i * 4 + 3] = a;
    }
}

/// Fades an on/off alpha mask out over its outermost texels: by the distance from each opaque
/// texel's middle to the nearest transparent one's (off the image counts as transparent), a
/// quarter at one texel, three quarters at two, all from two and a half. Only as far as the
/// painting is a body of paint (a metal patch, a crater, a flat rock: the fade keeps most of it):
/// one of strokes and specks (a scar's or a smudge's dither, a vent's cracks), whose outermost
/// texels are most of it, keeps them, its dither its soft edge (the fade, kept to half of it or
/// less, is not made at all; to seven tenths or more, in full).
fn feather(w: i32, h: i32, rgba: &mut [u8]) {
    let inside = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h && rgba[((y * w + x) * 4 + 3) as usize] >= 128;
    const REACH: i32 = 3;
    let fade: Vec<Option<f32>> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            if !inside(x, y) {
                return None;
            }
            let mut nearest = REACH as f32;
            for dy in -REACH..=REACH {
                for dx in -REACH..=REACH {
                    if !inside(x + dx, y + dy) {
                        nearest = nearest.min(((dx * dx + dy * dy) as f32).sqrt());
                    }
                }
            }
            let t = ((nearest - 0.5) / 2.0).clamp(0.0, 1.0);
            Some(t * t * (3.0 - 2.0 * t))
        })
        .collect();
    let (kept, painted) = fade.iter().flatten().fold((0.0f32, 0usize), |(k, n), &a| (k + a, n + 1));
    let share = ((kept / painted.max(1) as f32 - 0.5) / 0.2).clamp(0.0, 1.0);
    let body = share * share * (3.0 - 2.0 * share);
    for (i, a) in fade.into_iter().enumerate() {
        if let Some(a) = a {
            rgba[i * 4 + 3] = ((1.0 - body * (1.0 - a)) * 255.0).round() as u8;
        }
    }
}

/// How far (texels) a painted shadow fades out from an edge of its frame that cuts it.
const SHADOW_CUT_FADE: f32 = 10.0;

/// Softens a painted shadow's edge into a penumbra a few texels wide (two passes of a box blur
/// two texels each way): TA painted its shadows as hard polygons (a lava spire's with a ruler-
/// straight top and side), which enlarged were cut-outs, not shadows, beside soft light.
fn soften(w: i32, h: i32, rgba: &mut [u8]) {
    let mut a: Vec<f32> = (0..w * h).map(|i| rgba[(i * 4 + 3) as usize] as f32).collect();
    const R: i32 = 2;
    for _ in 0..2 {
        for (dx, dy) in [(1, 0), (0, 1)] {
            let src = a.clone();
            for y in 0..h {
                for x in 0..w {
                    let sum: f32 = (-R..=R)
                        .map(|k| {
                            let (sx, sy) = (x + k * dx, y + k * dy);
                            if sx >= 0 && sy >= 0 && sx < w && sy < h { src[(sy * w + sx) as usize] } else { 0.0 }
                        })
                        .sum();
                    a[(y * w + x) as usize] = sum / (2 * R + 1) as f32;
                }
            }
        }
    }
    for (i, v) in a.into_iter().enumerate() {
        rgba[i * 4 + 3] = v.round().clamp(0.0, 255.0) as u8;
    }
}

/// Which edges of its frame (left, right, top, bottom) a painting runs into: a few of its
/// texels lie on them.
fn cut_sides(w: i32, h: i32, rgba: &[u8]) -> [bool; 4] {
    let opaque = |x: i32, y: i32| rgba[((y * w + x) * 4 + 3) as usize] >= 128;
    let on = |n: i32, at: &dyn Fn(i32) -> bool| (0..n).filter(|&k| at(k)).count() >= 3;
    [on(h, &|y| opaque(0, y)), on(h, &|y| opaque(w - 1, y)), on(w, &|x| opaque(x, 0)), on(w, &|x| opaque(x, h - 1))]
}

/// Fades a painted shadow out towards the edges of its frame that cut it (`cut`, see
/// `cut_sides`: TA cropped some of its shadows' frames, a lava spire's: what reached past them
/// lay under the feature's own painting), over `reach` texels, so the frame's straight edge and
/// corner do not show where the shadow runs into it beside the feature.
fn fade_cuts(w: i32, h: i32, rgba: &mut [u8], cut: [bool; 4], reach: f32) {
    if !cut.contains(&true) {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let away = [x as f32 + 0.5, (w - x) as f32 - 0.5, y as f32 + 0.5, (h - y) as f32 - 0.5];
            let d = (0..4).filter(|&k| cut[k]).map(|k| away[k]).fold(f32::MAX, f32::min);
            let t = (d / reach).clamp(0.0, 1.0);
            let i = ((y * w + x) * 4 + 3) as usize;
            rgba[i] = (rgba[i] as f32 * t * t * (3.0 - 2.0 * t)).round() as u8;
        }
    }
}

/// Spreads the colours of the opaque texels (alpha 128 or more) out into the transparent ones
/// round them, `rings` texels deep, so filtering at the edge never pulls in black.
fn bleed(w: i32, h: i32, rgba: &mut [u8], rings: i32) {
    bleed_from(w, h, rgba, rings, 128);
}

/// As [`bleed`], from the texels of alpha `from` or more (1: a reduced frame's, whose every
/// texel covering anything carries the colour of what it covers).
// (Averages three channels over the same count: one test of it reads better than three
// checked divisions.)
#[allow(clippy::manual_checked_ops)]
fn bleed_from(w: i32, h: i32, rgba: &mut [u8], rings: i32, from: u8) {
    let mut coloured: Vec<bool> = (0..w * h).map(|i| rgba[(i * 4 + 3) as usize] >= from).collect();
    for _ in 0..rings {
        let mut next = coloured.clone();
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                if coloured[i] {
                    continue;
                }
                let (mut sum, mut n) = ([0u32; 3], 0u32);
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && nx < w && ny < h && coloured[(ny * w + nx) as usize] {
                        let j = ((ny * w + nx) * 4) as usize;
                        for k in 0..3 {
                            sum[k] += rgba[j + k] as u32;
                        }
                        n += 1;
                    }
                }
                if n > 0 {
                    for k in 0..3 {
                        rgba[i * 4 + k] = (sum[k] / n) as u8;
                    }
                    next[i] = true;
                }
            }
        }
        coloured = next;
    }
}

pub struct SpriteRenderer {
    pub atlas: SpriteAtlas,
    /// The largest texture the device takes (how far the atlas may grow).
    max_side: u32,
    layout: wgpu::BindGroupLayout,
    texture: wgpu::Texture,
    /// The atlas texture's size (made again when the atlas grows).
    texture_size: (u32, u32),
    frame_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    decal: wgpu::RenderPipeline,
    lying: wgpu::RenderPipeline,
    cutout: wgpu::RenderPipeline,
    additive: wgpu::RenderPipeline,
    smoke: wgpu::RenderPipeline,
    /// Billboards, effects and smoke mirrored in the water (reflection.rs).
    reflection: [wgpu::RenderPipeline; 3],
    instance_buffer: wgpu::Buffer,
    /// Each layer's instances (first, count): decals, billboards, effects, smoke, lying.
    batches: [(u32, u32); 5],
    staging: Vec<SpriteInstance>,
    reflection_instances: wgpu::Buffer,
    reflection_staging: Vec<SpriteInstance>,
    reflection_batches: [(u32, u32); 2],
}

impl SpriteRenderer {
    pub fn new(device: &wgpu::Device, frame_layout: &wgpu::BindGroupLayout, hdr: wgpu::TextureFormat, samples: u32) -> Self {
        use wgpu::{BindingType as B, ShaderStages as S};
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprites"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: S::VERTEX,
                    ty: B::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: S::FRAGMENT,
                    ty: B::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: S::FRAGMENT, ty: B::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let shader = crate::util::shader(device, "sprites", include_str!("shaders/sprite.wgsl"));
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprites"),
            bind_group_layouts: &[Some(frame_layout), Some(&layout)],
            immediate_size: 0,
        });
        let instance_layout = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SpriteInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32, 2 => Float32x4, 3 => Float32x4],
        })];
        // The scene pass's pipelines, or with `mirrored` those of the reflection pass.
        let make_for = |label: &str, vs: &str, fs: &str, blend: Option<wgpu::BlendState>, depth: wgpu::DepthStencilState, mirrored: bool| {
            // Billboards' outlines become coverage (anti-aliased edges) when multisampling.
            let coverage = !mirrored && samples > 1 && fs == "fs_cutout";
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), buffers: &instance_layout, compilation_options: Default::default() },
                primitive: if mirrored { crate::reflection::primitive() } else { wgpu::PrimitiveState { cull_mode: None, ..Default::default() } },
                depth_stencil: Some(depth),
                multisample: wgpu::MultisampleState { count: if mirrored { 1 } else { samples }, alpha_to_coverage_enabled: coverage, ..Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState { format: hdr, blend, write_mask: wgpu::ColorWrites::ALL })],
                    // (How many samples a painting handing over to its model shares them out of.)
                    compilation_options: wgpu::PipelineCompilationOptions { constants: &[("SCENE_SAMPLES", if mirrored { 1.0 } else { samples as f64 })], ..Default::default() },
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let make = |label: &str, vs: &str, fs: &str, blend: Option<wgpu::BlendState>, depth: wgpu::DepthStencilState| make_for(label, vs, fs, blend, depth, false);
        let premultiplied = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        // (A fragment's alpha is how much of what lies behind it the effect hides: 0 for plain
        // light; see `additive_color` in sprite.wgsl. The target's alpha is left alone.)
        let additive_blend = Some(wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        });
        let decal_depth = wgpu::DepthStencilState {
            depth_write_enabled: Some(false),
            bias: wgpu::DepthBiasState { constant: -8, slope_scale: -1.0, clamp: 0.0 },
            ..depth_state(false)
        };
        let decal = make("sprite decal", "vs_decal", "fs_decal", premultiplied, decal_depth.clone());
        let lying = make("sprite lying", "vs_lying", "fs_lying", premultiplied, decal_depth);
        let cutout = make("sprite billboard", "vs_billboard", "fs_cutout", None, depth_state(true));
        let additive = make("sprite additive", "vs_effect", "fs_additive", additive_blend, wgpu::DepthStencilState { depth_write_enabled: Some(false), ..depth_state(false) });
        let smoke = make("sprite smoke", "vs_effect", "fs_smoke", premultiplied, wgpu::DepthStencilState { depth_write_enabled: Some(false), ..depth_state(false) });
        let reflection = [
            make_for("sprite billboard reflection", "vs_billboard", "fs_cutout_reflection", None, depth_state(true), true),
            make_for("sprite additive reflection", "vs_effect", "fs_additive_reflection", additive_blend, wgpu::DepthStencilState { depth_write_enabled: Some(false), ..depth_state(false) }, true),
            make_for("sprite smoke reflection", "vs_effect", "fs_smoke_reflection", premultiplied, wgpu::DepthStencilState { depth_write_enabled: Some(false), ..depth_state(false) }, true),
        ];
        let _ = DEPTH_FORMAT;

        let max_side = device.limits().max_texture_dimension_2d;
        let atlas = SpriteAtlas::new(ATLAS_START, max_side);
        let texture_size = atlas.size();
        let texture = Self::make_texture(device, texture_size);
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite frames"),
            size: 16 * 1024 * 32,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sprites"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = Self::make_bind_group(device, &layout, &frame_buffer, &texture, &sampler);
        let instance_buffer = Self::make_instances(device, 4096);
        SpriteRenderer {
            atlas,
            max_side,
            layout,
            texture,
            texture_size,
            frame_buffer,
            bind_group,
            sampler,
            decal,
            lying,
            cutout,
            additive,
            smoke,
            reflection,
            instance_buffer,
            batches: [(0, 0); 5],
            staging: Vec::new(),
            reflection_instances: Self::make_instances(device, 1024),
            reflection_staging: Vec::new(),
            reflection_batches: [(0, 0); 2],
        }
    }

    /// An empty atlas for a new set of sprites (a battle's, the editor's), able to grow to the
    /// largest texture the device takes.
    pub fn reset_atlas(&mut self) {
        self.atlas = SpriteAtlas::new(ATLAS_START, self.max_side);
    }

    fn make_texture(device: &wgpu::Device, (width, height): (u32, u32)) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sprite atlas"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn make_instances(device: &wgpu::Device, n: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite instances"),
            size: (n * std::mem::size_of::<SpriteInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        frames: &wgpu::Buffer,
        texture: &wgpu::Texture,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprites"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: frames.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&texture.create_view(&Default::default())),
                },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        })
    }

    /// Uploads the atlas and this frame's instances (decals, billboards, effects, smoke and the
    /// features lying on the ground).
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        decals: &[SpriteInstance],
        billboards: &[SpriteInstance],
        effects: &[SpriteInstance],
        smoke: &[SpriteInstance],
        lying: &[SpriteInstance],
    ) {
        if self.atlas.dirty {
            let size = self.atlas.size();
            let resized = size != self.texture_size;
            if resized {
                self.texture = Self::make_texture(device, size);
                self.texture_size = size;
            }
            crate::util::write_texture_level(queue, &self.texture, 0, size.0, size.1, &self.atlas.pixels, 4);
            let needed = (self.atlas.frames.len() * std::mem::size_of::<FrameInfo>()) as u64;
            if needed > self.frame_buffer.size() {
                self.frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("sprite frames"),
                    size: needed.next_power_of_two(),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.bind_group = Self::make_bind_group(device, &self.layout, &self.frame_buffer, &self.texture, &self.sampler);
            } else if resized {
                self.bind_group = Self::make_bind_group(device, &self.layout, &self.frame_buffer, &self.texture, &self.sampler);
            }
            queue.write_buffer(&self.frame_buffer, 0, bytemuck::cast_slice(&self.atlas.frames));
            self.atlas.dirty = false;
        }
        self.staging.clear();
        let mut batches = [(0u32, 0u32); 5];
        for (i, list) in [decals, billboards, effects, smoke, lying].iter().enumerate() {
            batches[i] = (self.staging.len() as u32, list.len() as u32);
            self.staging.extend_from_slice(list);
        }
        self.batches = batches;
        if self.staging.is_empty() {
            return;
        }
        let bytes = (self.staging.len() * std::mem::size_of::<SpriteInstance>()) as u64;
        if bytes > self.instance_buffer.size() {
            self.instance_buffer = Self::make_instances(device, self.staging.len().next_power_of_two());
        }
        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&self.staging));
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, layer: SpriteLayer) {
        let (index, pipeline) = match layer {
            SpriteLayer::Decal => (0, &self.decal),
            SpriteLayer::Billboard => (1, &self.cutout),
            SpriteLayer::Effect => (2, &self.additive),
            SpriteLayer::Smoke => (3, &self.smoke),
            SpriteLayer::Lying => (4, &self.lying),
        };
        // (A lying sprite is a grid of 8 x 8 cells, a decal of up to 4 x 4, draped over the ground.)
        let vertices = match layer {
            SpriteLayer::Lying => LYING_VERTICES,
            SpriteLayer::Decal => DECAL_VERTICES,
            _ => 6,
        };
        self.draw_batch(pass, index, pipeline, vertices);
    }

    /// Sort translucent layers for the camera actually drawing this frame (including photos).
    /// A reflection needs its own order: mirroring the camera can reverse two plumes' depth.
    pub(crate) fn sort_transparency(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, depth: [f32; 4], reflected: Option<[f32; 4]>) {
        self.reflection_staging.clear();
        for (layer, batch) in [2usize, 3].into_iter().enumerate() {
            let (first, count) = self.batches[batch];
            let sprites = &mut self.staging[first as usize..(first + count) as usize];
            if let Some(mirrored) = reflected {
                let start = self.reflection_staging.len();
                self.reflection_staging.extend_from_slice(sprites);
                sort_back_to_front(&mut self.reflection_staging[start..], mirrored);
                self.reflection_batches[layer] = (start as u32, count);
            }
            sort_back_to_front(sprites, depth);
        }
        let first = self.batches[2].0 as usize;
        let end = (self.batches[3].0 + self.batches[3].1) as usize;
        if end > first {
            queue.write_buffer(&self.instance_buffer, (first * std::mem::size_of::<SpriteInstance>()) as u64, bytemuck::cast_slice(&self.staging[first..end]));
        }
        if !self.reflection_staging.is_empty() {
            let needed = std::mem::size_of_val(self.reflection_staging.as_slice()) as u64;
            if needed > self.reflection_instances.size() {
                self.reflection_instances = Self::make_instances(device, self.reflection_staging.len().next_power_of_two());
            }
            queue.write_buffer(&self.reflection_instances, 0, bytemuck::cast_slice(&self.reflection_staging));
        }
    }

    /// Draws one layer mirrored in the water (inside the reflection pass). Decals and lying
    /// sprites lie on the ground and are left out.
    pub(crate) fn draw_reflection<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, layer: SpriteLayer) {
        let (index, pipeline) = match layer {
            SpriteLayer::Decal | SpriteLayer::Lying => return,
            SpriteLayer::Billboard => (1, &self.reflection[0]),
            SpriteLayer::Effect => (2, &self.reflection[1]),
            SpriteLayer::Smoke => (3, &self.reflection[2]),
        };
        if index == 2 || index == 3 {
            let (first, count) = self.reflection_batches[index - 2];
            if count > 0 {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(1, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.reflection_instances.slice(..));
                pass.draw(0..6, first..first + count);
            }
            return;
        }
        self.draw_batch(pass, index, pipeline, 6);
    }

    fn draw_batch<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, index: usize, pipeline: &'a wgpu::RenderPipeline, vertices: u32) {
        let (first, count) = self.batches[index];
        if count == 0 {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
        pass.draw(0..vertices, first..first + count);
    }
}

fn sort_back_to_front(sprites: &mut [SpriteInstance], depth: [f32; 4]) {
    let distance = |s: &SpriteInstance| s.pos[0] * depth[0] + s.pos[1] * depth[1] + s.pos[2] * depth[2] + depth[3];
    // Stable for equal depths, so coplanar puffs retain their emission order.
    sprites.sort_by(|a, b| distance(b).total_cmp(&distance(a)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translucent_sprites_follow_view_depth_including_reflections() {
        let original = [
            SpriteInstance::new([1000.0, 10.0, 10.0], SpriteId(0)),
            SpriteInstance::new([0.0, 30.0, 30.0], SpriteId(1)),
            SpriteInstance::new([0.0, 30.0, 30.0], SpriteId(2)),
        ];
        let mut sprites = original;
        sort_back_to_front(&mut sprites, [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(sprites.map(|s| s.frame), [1, 2, 0], "depth, not Euclidean distance; ties stay stable");
        sort_back_to_front(&mut sprites, [0.0, -2.0, 1.0, 0.0]);
        assert_eq!(sprites.map(|s| s.frame), [0, 1, 2], "a mirrored view can reverse two plumes");
    }

    #[test]
    #[ignore = "requires a GPU; run explicitly for rendering changes"]
    fn gpu_sprite_filter_stays_in_frame_and_effects_do_not_sway() {
        use crate::gpu::CheckedBufferInit;
        let source = include_str!("shaders/sprite.wgsl");
        let section = |start: &str, end: &str| {
            let at = source.find(start).expect(start);
            &source[at..at + source[at..].find(end).expect(end)]
        };
        // Execute the production filter and vertex function. The wind is a known displacement
        // so the test can distinguish a feature's sway from an effect's opacity/cover field.
        let shader = format!(
            r#"
{}
{}
{}
{}
struct Frame {{ view_proj: mat4x4<f32>, billboard_right: vec4<f32>, billboard_up: vec4<f32>, camera_pos: vec4<f32>, wind: vec4<f32>, }};
var<private> frame: Frame;
fn wind_bend(p: vec3<f32>, h: f32, rise: f32) -> vec3<f32> {{ return vec3<f32>(2.0, 0.0, 0.0); }}
@group(0) @binding(3) var<storage, read_write> results: array<vec4<f32>>;
@compute @workgroup_size(1)
fn check() {{
    frame.view_proj = mat4x4<f32>(vec4<f32>(1.0, 0.0, 0.0, 0.0), vec4<f32>(0.0, 1.0, 0.0, 0.0), vec4<f32>(0.0, 0.0, 1.0, 0.0), vec4<f32>(0.0, 0.0, 0.0, 1.0));
    frame.billboard_right = vec4<f32>(1.0, 0.0, 0.0, 0.0);
    frame.billboard_up = vec4<f32>(0.0, 1.0, 0.0, 0.0);
    frame.camera_pos = vec4<f32>(0.0, 10.0, 20.0, 1.0);
    frame.wind = vec4<f32>(0.0, 0.0, 1.0, 0.0);
    var vertex: VIn;
    vertex.vi = 0u;
    vertex.pos = vec3<f32>(0.0);
    vertex.frame_index = 0u;
    vertex.params = vec4<f32>(1.0, 1.0, 1.0, 0.0);
    vertex.tint = vec4<f32>(1.0, 1.0, 1.0, 0.15);
    results[0] = vec4<f32>(hd_sprite(vec2<f32>(0.255), vec4<f32>(0.25, 0.25, 0.75, 0.75)), 1.0);
    results[1] = vec4<f32>(billboard_vertex(vertex, false).world, 1.0);
    results[2] = vec4<f32>(billboard_vertex(vertex, true).world, 1.0);
}}
"#,
            section("struct SpriteFrame", "fn corner").replace("@group(1)", "@group(0)"),
            section("fn corner", "@vertex"),
            section("fn billboard_vertex", "@vertex"),
            section("fn hd_sprite", "// Opaque-ish")
        );
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let module = crate::util::shader_standalone(&device, "sprite regression", &shader);
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: None, module: &module, entry_point: Some("check"), compilation_options: Default::default(), cache: None });
        let frame = FrameInfo { uv: [0.25, 0.25, 0.75, 0.75], geom: [2.0, 2.0, 1.0, 1.0] };
        let frames = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::bytes_of(&frame), usage: wgpu::BufferUsages::STORAGE });
        let texture = device.create_texture(&wgpu::TextureDescriptor { label: None, size: wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
        let pixels: Vec<_> = (0..16).flat_map(|i| if (1..3).contains(&(i % 4)) && (1..3).contains(&(i / 4)) { [255u8, 0, 0, 255] } else { [0u8, 255, 0, 255] }).collect();
        crate::util::write_texture_level(&queue, &texture, 0, 4, 4, &pixels, 4);
        let results = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 48, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 48, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None, layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: frames.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&texture.create_view(&Default::default())) },
                wgpu::BindGroupEntry { binding: 3, resource: results.as_entire_binding() },
            ],
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&results, 0, &read, 0, 48);
        queue.submit([encoder.finish()]);
        read.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).unwrap();
        let bytes = read.get_mapped_range(..).unwrap();
        let values: &[f32] = bytemuck::cast_slice(&bytes);
        // Hue calming preserves the red texel's light, with no neighbouring green contribution.
        assert!((values[0] - 1.0).abs() < 1e-5 && values[1].abs() < 1e-5 && values[2].abs() < 1e-5, "edge colour: {:?}", &values[..3]);
        assert!((values[4] + 1.0).abs() < 1e-5, "effect moved in the wind: {}", values[4]);
        assert!((values[8] - values[4] - 1.7).abs() < 1e-5, "standing feature should still sway");
    }

    /// A decal's edge colours spread into the transparent texels round it (filtering at its edge
    /// fades into the ground, not through black), its transparency kept.
    #[test]
    fn edge_colours_bleed_into_the_transparent_texels() {
        // A 6 x 1 strip: two opaque texels on the left, then four transparent ones.
        let mut rgba = vec![200, 100, 50, 255, 200, 100, 50, 255];
        rgba.extend([0u8; 16]);
        bleed(6, 1, &mut rgba, 2);
        assert_eq!(&rgba[8..12], &[200, 100, 50, 0], "the first ring takes the edge's colour");
        assert_eq!(&rgba[12..16], &[200, 100, 50, 0], "so does the second");
        assert_eq!(&rgba[16..20], &[0, 0, 0, 0], "and no further");
    }

    /// A painted shadow fades out towards an edge of its frame that cuts it, and nowhere else.
    #[test]
    fn shadows_fade_where_their_frame_cuts_them() {
        // 30 x 30: the top 20 rows opaque, running into the frame's top, left and right edges.
        let (w, h) = (30i32, 30i32);
        let mut rgba: Vec<u8> = (0..w * h).flat_map(|i| if i / w < 20 { [0, 0, 0, 255] } else { [0, 0, 0, 0] }).collect();
        let cut = cut_sides(w, h, &rgba);
        assert_eq!(cut, [true, true, true, false], "cut at its left, right and top");
        fade_cuts(w, h, &mut rgba, cut, 10.0);
        let alpha = |x: i32, y: i32| rgba[((y * w + x) * 4 + 3) as usize];
        assert!(alpha(15, 0) < 10, "faded at the top edge: {}", alpha(15, 0));
        assert!(alpha(0, 15) < 10, "and at the left: {}", alpha(0, 15));
        assert_eq!(alpha(15, 15), 255, "not in the middle");
        assert_eq!(alpha(15, 19), 255, "nor at its own edge, inside the frame");
        assert_eq!(alpha(15, 25), 0, "transparent as it was");
    }

    /// A decal painted as a patch (a body of paint) fades out over its outermost texels; one of
    /// specks (a dithered scar, a frond's leaves) and a plant keep every texel as painted.
    #[test]
    fn only_patches_fade_at_their_edges() {
        let mut palette = Palette::default();
        palette.colors[7] = [90, 140, 60];
        const N: usize = 20;
        let frame = |opaque: Vec<bool>| GafFrame {
            image: ta_formats::image::IndexedImage { width: N as u32, height: N as u32, pixels: opaque.iter().map(|&o| if o { 7 } else { 0 }).collect(), opaque },
            origin_x: 0,
            origin_y: 0,
            transparency_index: 0,
            duration: 0,
        };
        let alphas = |atlas: &SpriteAtlas, id: SpriteId| {
            let f = atlas.frame(id);
            let (x0, y0) = ((f.uv[0] * 256.0).round() as usize, (f.uv[1] * 256.0).round() as usize);
            (0..N * N).map(|i| atlas.pixels[((y0 + i / N) * 256 + x0 + i % N) * 4 + 3]).collect::<Vec<u8>>()
        };
        let mut atlas = SpriteAtlas::new(256, 256);
        // A disc sixteen texels across.
        let middle = (N / 2) as f32 - 0.5;
        let disc: Vec<bool> = (0..N * N).map(|i| ((i % N) as f32 - middle).hypot((i / N) as f32 - middle) < 8.0).collect();
        let id = atlas.add_gaf_decal(&frame(disc.clone()), &palette, false).unwrap();
        let patch = alphas(&atlas, id);
        assert_eq!(patch[10 * N + 10], 255, "a patch's middle as painted");
        assert!(disc[10 * N + 2] && patch[10 * N + 2] < 64, "its rim faded ({})", patch[10 * N + 2]);
        // Every other texel: specks.
        let specks: Vec<bool> = (0..N * N).map(|i| (i % N + i / N).is_multiple_of(2)).collect();
        let id = atlas.add_gaf_decal(&frame(specks.clone()), &palette, false).unwrap();
        let a = alphas(&atlas, id);
        assert!(specks.iter().zip(&a).all(|(&o, &a)| a == if o { 255 } else { 0 }), "specks as painted");
        let id = atlas.add_gaf_decal(&frame(disc.clone()), &palette, true).unwrap();
        let a = alphas(&atlas, id);
        assert!(disc.iter().zip(&a).all(|(&o, &a)| a == if o { 255 } else { 0 }), "a plant as painted");
    }

    /// A GAF frame's transparent texels next to its edge take the edge's colour (a shadow's stay
    /// black), its opaque texels and its transparency as they were.
    #[test]
    fn gaf_frames_carry_their_edge_colours_out() {
        let mut palette = Palette::default();
        palette.colors[7] = [200, 100, 50];
        let frame = |w: u32| GafFrame {
            image: ta_formats::image::IndexedImage { width: w, height: 1, pixels: vec![7, 7, 0, 0, 0], opaque: vec![true, true, false, false, false] },
            origin_x: 0,
            origin_y: 0,
            transparency_index: 0,
            duration: 0,
        };
        let mut atlas = SpriteAtlas::new(64, 256);
        let texels = |atlas: &SpriteAtlas, id: SpriteId| {
            let f = atlas.frame(id);
            let (x, y) = ((f.uv[0] * 64.0).round() as usize, (f.uv[1] * 64.0).round() as usize);
            (0..5).map(|k| atlas.pixels[(y * 64 + x + k) * 4..(y * 64 + x + k) * 4 + 4].to_vec()).collect::<Vec<_>>()
        };
        let id = atlas.add_gaf(&frame(5), &palette, false).unwrap();
        let t = texels(&atlas, id);
        assert_eq!(t[1], palette.rgba(7).to_vec(), "opaque as painted");
        assert_eq!(t[2], vec![200, 100, 50, 0], "the edge's colour, transparent");
        assert_eq!(t[3], vec![200, 100, 50, 0], "two texels out");
        assert_eq!(t[4], vec![0, 0, 0, 0], "no further");
        let shadow = atlas.add_gaf(&frame(5), &palette, true).unwrap();
        assert_eq!(texels(&atlas, shadow)[2][..3], [0, 0, 0], "a shadow's stay black (its penumbra spreads into them)");
    }

    /// A frame read reduced is drawn as big as it was painted, cropped to what it covers (and
    /// the colours carried round that), its origin where it was; an empty one is one texel.
    #[test]
    fn reduced_frames_are_drawn_as_big_as_painted() {
        // 8 x 6 texels at 2.5 pixels each (a 20 x 15 frame, origin 10, 7): a red texel at
        // (3, 2) and a faint blue one beside it.
        let mut rgba = vec![0u8; 8 * 6 * 4];
        rgba[(2 * 8 + 3) * 4..(2 * 8 + 3) * 4 + 4].copy_from_slice(&[255, 0, 0, 255]);
        rgba[(2 * 8 + 4) * 4..(2 * 8 + 4) * 4 + 4].copy_from_slice(&[0, 0, 255, 64]);
        let frame = ReducedFrame { width: 8, height: 6, rgba, scale: 2.5, full_width: 20, full_height: 15, origin_x: 10, origin_y: 7, duration: 0 };
        let mut atlas = SpriteAtlas::new(64, 64);
        let id = atlas.add_reduced(&frame).unwrap();
        let f = *atlas.frame(id);
        // Texels 1 to 6 across, 0 to 4 down (two round what it covers): 15 x 12.5 pixels, its
        // corner 2.5 pixels in from the frame's, so its origin 2.5 nearer.
        assert_eq!(f.geom, [15.0, 12.5, 7.5, 7.0]);
        assert_eq!((((f.uv[2] - f.uv[0]) * 64.0).round(), ((f.uv[3] - f.uv[1]) * 64.0).round()), (6.0, 5.0));
        let texel = |x: usize, y: usize| {
            let (x0, y0) = ((f.uv[0] * 64.0).round() as usize, (f.uv[1] * 64.0).round() as usize);
            atlas.pixels[((y0 + y) * 64 + x0 + x) * 4..((y0 + y) * 64 + x0 + x) * 4 + 4].to_vec()
        };
        assert_eq!(texel(2, 2), [255, 0, 0, 255], "the red texel, as read");
        assert_eq!(texel(3, 2), [0, 0, 255, 64], "the faint one keeps its own colour");
        assert_eq!(texel(4, 2), [0, 0, 255, 0], "its colour carried out, transparent");
        assert_eq!(texel(0, 0), [255, 0, 0, 0], "two rings out");
        let empty = ReducedFrame { rgba: vec![0; 8 * 6 * 4], ..frame };
        let id = atlas.add_reduced(&empty).unwrap();
        assert_eq!(atlas.frame(id).geom, [2.5, 2.5, 10.0, 7.0], "one texel, drawing nothing");
    }

    /// A full atlas grows (taller, then wider), keeping every frame where it was and their uvs
    /// right; at the largest side it takes no more.
    #[test]
    fn the_atlas_grows_when_full() {
        let mut atlas = SpriteAtlas::new(64, 256);
        let tile = |c: u8| vec![c; 30 * 30 * 4];
        let mut ids = Vec::new();
        while let Some(id) = atlas.add_rgba(30, 30, &tile(ids.len() as u8), 0.0, 0.0) {
            ids.push(id);
        }
        assert_eq!(atlas.size(), (256, 256), "grew to the largest side");
        // 32 px a frame with its padding: 8 shelves of 8 in 256 x 256.
        assert_eq!(ids.len(), 64);
        for (k, &id) in ids.iter().enumerate() {
            let f = atlas.frame(id);
            let (x, y) = ((f.uv[0] * 256.0).round() as usize, (f.uv[1] * 256.0).round() as usize);
            assert_eq!(((f.uv[2] - f.uv[0]) * 256.0).round(), 30.0);
            assert_eq!(atlas.pixels[(y * 256 + x) * 4], k as u8, "frame {k} where its uvs say");
            assert_eq!(atlas.pixels[((y + 29) * 256 + x + 29) * 4], k as u8);
        }
        assert!(atlas.add_rgba(30, 30, &tile(0), 0.0, 0.0).is_none(), "full at the largest side");
        assert!(atlas.add_rgba(300, 2, &[0; 300 * 2 * 4], 0.0, 0.0).is_none(), "wider than the largest side");
    }
}
