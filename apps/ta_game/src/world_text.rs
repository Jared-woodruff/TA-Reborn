//! Text in the world: names and words laid on the ground, burning, projected as light or built by
//! a nanolathe, over the battle they are part of (the credits' roll, `credits_show.rs`).
//!
//! The renderer draws glyphs from an atlas of signed distance fields (`ta_render::world_text`);
//! this module cuts them: each character of the interface's face (Bahnschrift, with the system's
//! Chinese, Japanese and Korean faces behind it for the scripts it lacks) is drawn four times
//! larger than its cell, thresholded, and turned into the distance from its outline by two
//! exact distance transforms, and the cell is the average of each four by four. A [`Text`] is
//! lines of such glyphs laid out round the middle of its block, put in the world (where it stands
//! and which way it reads), given a [`Look`] and a life, and each frame turned into the instances
//! the renderer draws ([`Text::instances`]): where each letter is, how far it has appeared, how
//! hot it glows.
//!
//! "Heading" is the way a reader looks (degrees, 0 north, 90 east): the text reads left to right
//! across it and its lines go away from the reader, the first farthest. Lying on the ground a
//! text drapes over the relief (the renderer does it); standing in the air it keeps its own
//! planes.

use std::collections::HashMap;

use ab_glyph::{Font, FontRef, FontVec, GlyphId, PxScale, VariableFont, point};
use glam::{Vec2, Vec3};
use ta_render::Renderer;
use ta_render::world_text::{Cell, GlyphInstance, SPREAD, style};

/// Texels an em is across in a glyph's cell.
pub const EM_TEXELS: f32 = 64.0;
/// A glyph is drawn this many times larger than its cell before its distances are taken.
const SUPER: usize = 4;
/// The renderer's spread, in texels.
const SPREAD_TEXELS: usize = SPREAD as usize;
/// A line's height (ems) and where its baseline falls in it, from its top.
const LINE: f32 = 1.2;
const BASELINE: f32 = 0.92;

/// How heavy a line is set: the interface's display face (condensed, bold), its strong face and
/// its regular one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Weight {
    Display,
    Strong,
    Regular,
}

impl Weight {
    fn index(self) -> usize {
        self as usize
    }

    /// The face's weight and width.
    fn axes(self) -> (f32, f32) {
        match self {
            Weight::Display => (700.0, 75.0),
            Weight::Strong => (600.0, 100.0),
            Weight::Regular => (400.0, 100.0),
        }
    }
}

/// One face to draw glyphs from.
enum Face {
    Owned(FontVec),
    System(FontRef<'static>),
}

macro_rules! face {
    ($f:expr, |$font:ident| $body:expr) => {
        match $f {
            Face::Owned($font) => $body,
            Face::System($font) => $body,
        }
    };
}

impl Face {
    fn units_per_em(&self) -> Option<f32> {
        face!(self, |f| f.units_per_em())
    }

    fn height(&self) -> f32 {
        face!(self, |f| f.height_unscaled())
    }

    fn has(&self, c: char) -> bool {
        face!(self, |f| f.glyph_id(c).0 != 0)
    }

    /// The glyph's advance and its kerning with the next, in ems.
    fn advance(&self, c: char) -> f32 {
        let upem = self.units_per_em().unwrap_or(1000.0);
        face!(self, |f| f.h_advance_unscaled(f.glyph_id(c)) / upem)
    }

    fn kern(&self, a: char, b: char) -> f32 {
        let upem = self.units_per_em().unwrap_or(1000.0);
        face!(self, |f| f.kern_unscaled(f.glyph_id(a), f.glyph_id(b)) / upem)
    }

    fn cut(&self, c: char) -> Option<Cut> {
        let upem = self.units_per_em()?;
        let height = self.height();
        face!(self, |f| cut(f, f.glyph_id(c), upem, height))
    }
}

/// The faces, by weight: the interface's face in that weight first, the system's others behind.
struct Fonts {
    faces: Vec<Face>,
    sets: [Vec<usize>; 3],
}

impl Fonts {
    fn load() -> Fonts {
        let (bytes, variable) = ta_ui::theme::ui_font_bytes();
        let mut faces = Vec::new();
        let mut sets: [Vec<usize>; 3] = Default::default();
        for weight in [Weight::Display, Weight::Strong, Weight::Regular] {
            if let Ok(mut font) = FontVec::try_from_vec(bytes.clone()) {
                if variable {
                    let (w, wd) = weight.axes();
                    let _ = font.set_variation(b"wght", w);
                    let _ = font.set_variation(b"wdth", wd);
                }
                sets[weight.index()].push(faces.len());
                faces.push(Face::Owned(font));
            }
        }
        for (data, index) in ta_ui::theme::fallback_faces() {
            if let Ok(font) = FontRef::try_from_slice_and_index(data, index) {
                let at = faces.len();
                faces.push(Face::System(font));
                for set in &mut sets {
                    set.push(at);
                }
            }
        }
        Fonts { faces, sets }
    }

    /// The first face of the weight that has `c`.
    fn face(&self, weight: Weight, c: char) -> Option<&Face> {
        let set = &self.sets[weight.index()];
        set.iter().map(|&i| &self.faces[i]).find(|f| f.has(c)).or_else(|| set.first().map(|&i| &self.faces[i]))
    }
}

/// A glyph cut: its cell's texels and where the cell lies (ems: its left from the pen, its
/// bottom over the baseline, its size).
struct Cut {
    width: usize,
    height: usize,
    field: Vec<u8>,
    left: f32,
    bottom: f32,
    width_em: f32,
    height_em: f32,
}

/// A glyph's signed distance field cell (see the module's notes); none for a blank glyph.
fn cut<F: Font>(font: &F, id: GlyphId, upem: f32, height: f32) -> Option<Cut> {
    let px_per_em = EM_TEXELS * SUPER as f32;
    let scale = PxScale::from(px_per_em * height / upem);
    let outlined = font.outline_glyph(id.with_scale_and_position(scale, point(0.0, 0.0)))?;
    let bounds = outlined.px_bounds();
    let (gw, gh) = (bounds.width().ceil() as usize, bounds.height().ceil() as usize);
    if gw == 0 || gh == 0 {
        return None;
    }
    let spread = SPREAD_TEXELS * SUPER;
    let (cw, ch) = (gw.div_ceil(SUPER) + 2 * SPREAD_TEXELS, gh.div_ceil(SUPER) + 2 * SPREAD_TEXELS);
    let (bw, bh) = (cw * SUPER, ch * SUPER);
    let mut inside = vec![false; bw * bh];
    outlined.draw(|x, y, c| {
        let (x, y) = (x as usize + spread, y as usize + spread);
        if c >= 0.5 && x < bw && y < bh {
            inside[y * bw + x] = true;
        }
    });
    let signed = signed_distance(&inside, bw, bh);
    // The average of each SUPER x SUPER block, in texels, as a byte: 0.5 on the outline, SPREAD
    // texels to 0 and 1.
    let mut field = vec![0u8; cw * ch];
    let k = 1.0 / (SUPER * SUPER) as f32;
    for ty in 0..ch {
        for tx in 0..cw {
            let mut sum = 0.0;
            for y in 0..SUPER {
                for x in 0..SUPER {
                    sum += signed[(ty * SUPER + y) * bw + tx * SUPER + x];
                }
            }
            let texels = sum * k / SUPER as f32;
            field[ty * cw + tx] = ((0.5 + texels / (2.0 * SPREAD)).clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    let top = -(bounds.min.y - spread as f32) / px_per_em;
    Some(Cut {
        width: cw,
        height: ch,
        field,
        left: (bounds.min.x - spread as f32) / px_per_em,
        bottom: top - bh as f32 / px_per_em,
        width_em: bw as f32 / px_per_em,
        height_em: bh as f32 / px_per_em,
    })
}

/// How far each pixel is from the outline of the `inside` mask, in pixels: positive in it,
/// negative outside (the outline half a pixel from the nearest pixel of the other side).
fn signed_distance(inside: &[bool], w: usize, h: usize) -> Vec<f32> {
    let to_outside = distances(inside, w, h, false);
    let to_inside = distances(inside, w, h, true);
    (0..w * h).map(|i| if inside[i] { to_outside[i] - 0.5 } else { -(to_inside[i] - 0.5) }).collect()
}

/// The distance from each pixel to the nearest pixel of the mask whose value is `of` (0 on
/// those): an exact Euclidean distance transform (Felzenszwalb and Huttenlocher), rows then
/// columns.
fn distances(mask: &[bool], w: usize, h: usize, of: bool) -> Vec<f32> {
    const FAR: f32 = 1.0e9;
    let mut d: Vec<f32> = mask.iter().map(|&m| if m == of { 0.0 } else { FAR }).collect();
    let n = w.max(h);
    let (mut line, mut out, mut v, mut z) = (vec![0.0f32; n], vec![0.0f32; n], vec![0usize; n], vec![0.0f32; n + 1]);
    for x in 0..w {
        for y in 0..h {
            line[y] = d[y * w + x];
        }
        transform(&line[..h], &mut out[..h], &mut v, &mut z);
        for y in 0..h {
            d[y * w + x] = out[y];
        }
    }
    for y in 0..h {
        line[..w].copy_from_slice(&d[y * w..(y + 1) * w]);
        transform(&line[..w], &mut out[..w], &mut v, &mut z);
        d[y * w..(y + 1) * w].copy_from_slice(&out[..w]);
    }
    d.iter().map(|&s| s.sqrt()).collect()
}

/// One dimension of the transform: the lower envelope of the parabolas rooted at each `f[q]`.
fn transform(f: &[f32], out: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f32::MIN;
    z[1] = f32::MAX;
    for q in 1..n {
        let qf = q as f32;
        let mut s;
        loop {
            let p = v[k];
            let pf = p as f32;
            s = ((f[q] + qf * qf) - (f[p] + pf * pf)) / (2.0 * qf - 2.0 * pf);
            if s <= z[k] && k > 0 {
                k -= 1;
            } else {
                break;
            }
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f32::MAX;
    }
    k = 0;
    for (q, o) in out.iter_mut().enumerate() {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        let dq = q as f32 - p as f32;
        *o = dq * dq + f[p];
    }
}

/// A glyph in the atlas: its cell and where it lies (ems, from the pen and over the baseline),
/// or no cell for a blank; and its advance.
#[derive(Clone, Copy, Debug)]
struct Cached {
    cell: Option<Cell>,
    left: f32,
    bottom: f32,
    width: f32,
    height: f32,
    advance: f32,
}

/// One line of a text: what it says and how it is set.
#[derive(Clone, Debug)]
pub struct LineSpec {
    pub text: String,
    /// Its size against the text's (1 for the main lines).
    pub scale: f32,
    pub weight: Weight,
    /// Its colour (sRGB, 0..1).
    pub color: [f32; 3],
    /// Space added between its letters (ems).
    pub tracking: f32,
    /// Room above it (ems of the main size).
    pub gap: f32,
}

impl LineSpec {
    pub fn new(text: impl Into<String>, scale: f32, weight: Weight, color: [f32; 3]) -> LineSpec {
        LineSpec { text: text.into(), scale, weight, color, tracking: 0.0, gap: 0.0 }
    }

    pub fn tracking(mut self, tracking: f32) -> LineSpec {
        self.tracking = tracking;
        self
    }

    pub fn gap(mut self, gap: f32) -> LineSpec {
        self.gap = gap;
        self
    }
}

/// A glyph laid out: its cell, and where the cell lies in its line (ems of the line).
#[derive(Clone, Copy, Debug)]
struct Laid {
    cell: Cell,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[derive(Clone, Debug)]
struct BlockLine {
    glyphs: Vec<Laid>,
    scale: f32,
    color: [f32; 3],
    /// Where its first pen position is (main ems from the block's middle), and its baseline.
    x0: f32,
    baseline: f32,
}

/// Lines of glyphs laid out round the block's middle (main ems; x to the right, y up).
#[derive(Clone, Debug, Default)]
pub struct Block {
    lines: Vec<BlockLine>,
    /// Its extent, in main ems.
    pub width: f32,
    pub height: f32,
}

impl Block {
    pub fn glyph_count(&self) -> usize {
        self.lines.iter().map(|l| l.glyphs.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.glyph_count() == 0
    }
}

/// How a text looks and moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    /// Paint on the ground, lit as it is: the letters coming on one after another.
    Paint,
    /// Struck by a blast at `at` (world x, z) `after` seconds in: the letters ignite in a ripple
    /// out from it, white hot, and cool to embers that keep their glow.
    Burn { at: Vec2, after: f32 },
    /// Light projected on the ground, a scan crossing it.
    Light,
    /// Built by a nanolathe: each letter's frame, solid from the ground up, one after another.
    Nano,
}

/// A text to put in the world.
#[derive(Clone, Debug)]
pub struct TextSpec {
    pub lines: Vec<LineSpec>,
    /// Where the middle of its block lies on the ground (world x, z).
    pub at: Vec2,
    /// The way a reader looks (degrees, 0 north, 90 east).
    pub heading: f32,
    /// World units to a main em: how big it is.
    pub size: f32,
    pub look: Look,
    /// When it comes (the game's seconds) and how long it lasts.
    pub born: f32,
    pub life: f32,
    /// The least light its paint shows in (0..1): how much it shows in the dark.
    pub floor: f32,
}

/// A text in the world.
#[derive(Clone, Debug)]
pub struct Text {
    block: Block,
    anchor: Vec3,
    right: Vec3,
    up: Vec3,
    pub size: f32,
    pub look: Look,
    pub born: f32,
    pub life: f32,
    floor: f32,
    seed: f32,
    /// A picture's cell (the emblem) in place of glyphs, and its size in world units.
    picture: Option<(Cell, f32)>,
}

/// How far over the ground a text lies (world units): over the mesh the ground is drawn with,
/// which the smooth heights it is draped by miss by a unit or two on a slope.
const LIFT: f32 = 2.0;

/// Seconds a text takes to go out at the end of its life.
const GO_OUT: f32 = 1.4;

/// Seconds a nanolathe takes to build a picture (the emblem), its near edge to its far one.
const NANO_PICTURE: f32 = 1.6;

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn linear(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) })
}

impl Text {
    /// The instances to draw at `now` (the game's seconds): where each letter is and how far it
    /// has come, how hot it glows. `calm`: no flicker (reduced flashes).
    pub fn instances(&self, now: f32, calm: bool, out: &mut Vec<GlyphInstance>) {
        let age = now - self.born;
        if age < 0.0 || age > self.life + 0.1 {
            return;
        }
        let fade = 1.0 - smooth((age - (self.life - GO_OUT)) / GO_OUT);
        if let Some((cell, side)) = self.picture {
            self.picture_instance(cell, side, age, fade, out);
            return;
        }
        let total = self.block.glyph_count().max(1) as f32;
        let mut index = 0usize;
        for (li, line) in self.block.lines.iter().enumerate() {
            let line_delay = li as f32 * 0.16;
            for laid in &line.glyphs {
                let i = index;
                index += 1;
                let along = i as f32 / total;
                let cx = line.x0 + (laid.x + laid.w * 0.5) * line.scale;
                let cy = line.baseline + (laid.y + laid.h * 0.5) * line.scale;
                let centre = self.anchor + self.right * (cx * self.size) + self.up * (cy * self.size);
                let half_right = self.right * (laid.w * line.scale * self.size * 0.5);
                let half_up = self.up * (laid.h * line.scale * self.size * 0.5);
                let texel = line.scale * self.size / EM_TEXELS;
                // A little of its own in each letter: no two stamped alike.
                let seed = (self.seed + i as f32 * 0.6180339).fract();
                let mut state = [0.0, 1.0, age, seed];
                let mut color = linear(line.color);
                let mut opacity = fade;
                let mut glow = [0.0, 0.0, 0.0, self.floor];
                let mut look_style = style::PAINT;
                match self.look {
                    Look::Paint => {
                        let t = age - line_delay - along * 0.9;
                        opacity *= smooth(t / 0.25);
                        state[1] = smooth(t / 0.55);
                    }
                    Look::Burn { at, after } => {
                        let d = Vec2::new(centre.x, centre.z).distance(at);
                        let since = age - (after + d / 420.0);
                        opacity *= smooth(since / 0.1);
                        // White hot at the front, cooling to embers that keep a little of it (and
                        // breathe: a coal does).
                        let cooling = (-since.max(0.0) / 0.95).exp();
                        let breathe = if calm { 0.0 } else { 0.035 * (age * 2.1 + seed * 40.0).sin() };
                        state[0] = 0.27 + 0.85 * cooling + breathe;
                        // (Charcoal where the paint was.)
                        color = linear([0.075, 0.06, 0.05]);
                        glow[3] = 0.0;
                    }
                    Look::Light => {
                        let t = age - line_delay - along * 1.1;
                        state[1] = smooth(t / 0.5);
                        opacity *= smooth(t / 0.12);
                        look_style = style::LIGHT;
                        // A projector's light flickers a little; with reduced flashes it holds
                        // steady at its mean.
                        let flicker = if calm { 0.92 } else { 0.92 + 0.08 * (age * 37.0 + seed * 40.0).sin() };
                        color = color.map(|c| c * flicker);
                    }
                    Look::Nano => {
                        let t = age - line_delay - along * 1.3;
                        state[1] = (t / 0.95).clamp(0.0, 1.0);
                        opacity *= smooth(t / 0.15);
                        look_style = style::LATTICE;
                    }
                }
                if opacity <= 0.002 {
                    continue;
                }
                // (A dissolve at the end, a letter at a time.)
                if fade < 1.0 {
                    let gone = smooth((1.0 - fade) * 1.6 - along * 0.6);
                    opacity *= 1.0 - gone;
                    state[0] += gone * 0.5 * state[0].min(1.0);
                }
                glow[0] *= 1.0;
                out.push(GlyphInstance {
                    centre: [centre.x, centre.y, centre.z, look_style],
                    right: [half_right.x, half_right.y, half_right.z, 1.0],
                    up: [half_up.x, half_up.y, half_up.z, texel],
                    uv: laid.cell,
                    color: [color[0], color[1], color[2], opacity],
                    state,
                    glow,
                    wave: [0.0; 4],
                });
            }
        }
    }

    fn picture_instance(&self, cell: Cell, side: f32, age: f32, fade: f32, out: &mut Vec<GlyphInstance>) {
        let half = side * 0.5;
        let (right, up) = (self.right * half, self.up * half);
        // Burning in, as a blast's ripple takes it: hot at first, the heat going down into its
        // own glow; or built by a nanolathe from its near edge, as the letters are.
        let (heat, opacity, reveal) = match self.look {
            Look::Burn { after, .. } => {
                let since = age - after;
                ((1.3 * (-since.max(0.0) / 1.6).exp()) * smooth(since / 0.05), smooth(since / 0.2), 1.0)
            }
            Look::Nano => (0.0, smooth(age / 0.15), (age / NANO_PICTURE).clamp(0.0, 1.0)),
            _ => (0.0, smooth(age / 0.8), 1.0),
        };
        if opacity * fade <= 0.002 {
            return;
        }
        out.push(GlyphInstance {
            centre: [self.anchor.x, self.anchor.y, self.anchor.z, style::PICTURE],
            right: [right.x, right.y, right.z, 1.0],
            up: [up.x, up.y, up.z, side / 512.0],
            uv: cell,
            color: [1.0, 1.0, 1.0, opacity * fade],
            state: [heat, reveal, age, self.seed],
            glow: [0.0, 0.0, 0.0, self.floor],
            wave: [0.0; 4],
        });
    }
}

/// The text of the world: the glyphs cut so far, and the texts in it.
pub struct WorldText {
    fonts: Option<Fonts>,
    cache: HashMap<(Weight, char), Cached>,
    /// The renderer whose atlas holds the cells cut.
    serial: u64,
    emblem: Option<Cell>,
    pub texts: Vec<Text>,
    frame: Vec<GlyphInstance>,
}

impl Default for WorldText {
    fn default() -> Self {
        WorldText { fonts: None, cache: HashMap::new(), serial: 0, emblem: None, texts: Vec::new(), frame: Vec::new() }
    }
}

impl WorldText {
    /// Forgets the cells cut (and the texts that stand on them) when the renderer is another's
    /// (made again, its atlas empty).
    fn sync(&mut self, serial: u64) {
        if self.serial != serial {
            self.serial = serial;
            self.cache.clear();
            self.emblem = None;
            self.texts.clear();
        }
    }

    /// The glyph of `c` in `weight`, cut into the renderer's atlas the first time.
    fn glyph(&mut self, renderer: &mut Renderer, weight: Weight, c: char) -> Cached {
        if let Some(&g) = self.cache.get(&(weight, c)) {
            return g;
        }
        let fonts = self.fonts.get_or_insert_with(Fonts::load);
        let blank = Cached { cell: None, left: 0.0, bottom: 0.0, width: 0.0, height: 0.0, advance: 0.35 };
        let g = match fonts.face(weight, c) {
            None => blank,
            Some(face) => {
                let advance = if c == ' ' { face.advance(' ').max(0.25) } else { face.advance(c) };
                match face.cut(c) {
                    Some(cut) if !c.is_whitespace() => {
                        let cell = renderer.world_text.add_glyph(cut.width as u32, cut.height as u32, &cut.field);
                        Cached { cell, left: cut.left, bottom: cut.bottom, width: cut.width_em, height: cut.height_em, advance }
                    }
                    _ => Cached { advance, ..blank },
                }
            }
        };
        self.cache.insert((weight, c), g);
        g
    }

    /// Lays `lines` out round the middle of a block, centred on one another.
    pub fn lay_out(&mut self, renderer: &mut Renderer, lines: &[LineSpec]) -> Block {
        self.sync(renderer.world_text.serial());
        let mut block = Block::default();
        let mut y = 0.0;
        let mut widths = Vec::new();
        for spec in lines {
            y += spec.gap;
            let top = y;
            y += LINE * spec.scale;
            let mut glyphs = Vec::new();
            let mut x = 0.0;
            let mut last: Option<char> = None;
            for c in spec.text.chars() {
                if let (Some(prev), Some(fonts)) = (last, self.fonts.as_ref())
                    && let Some(face) = fonts.face(spec.weight, c)
                {
                    x += face.kern(prev, c);
                }
                let g = self.glyph(renderer, spec.weight, c);
                if let Some(cell) = g.cell {
                    glyphs.push(Laid { cell, x: x + g.left, y: g.bottom, w: g.width, h: g.height });
                }
                x += g.advance + spec.tracking;
                last = Some(c);
            }
            let width = (x - if last.is_some() { spec.tracking } else { 0.0 }).max(0.0);
            widths.push(width * spec.scale);
            block.lines.push(BlockLine { glyphs, scale: spec.scale, color: spec.color, x0: -width * spec.scale * 0.5, baseline: -(top + BASELINE * spec.scale) });
        }
        // The block's middle is the middle of its height.
        block.height = y;
        for line in &mut block.lines {
            line.baseline += y * 0.5;
        }
        block.width = widths.iter().copied().fold(0.0, f32::max);
        block
    }

    /// A text of the spec, laid out and put in the world (not yet added to it).
    pub fn make(&mut self, renderer: &mut Renderer, spec: &TextSpec) -> Text {
        let block = self.lay_out(renderer, &spec.lines);
        let (s, c) = spec.heading.to_radians().sin_cos();
        let forward = Vec3::new(s, 0.0, -c);
        let right = Vec3::new(c, 0.0, s);
        Text {
            block,
            anchor: Vec3::new(spec.at.x, LIFT, spec.at.y),
            right,
            up: forward,
            size: spec.size,
            look: spec.look,
            born: spec.born,
            life: spec.life,
            floor: spec.floor,
            seed: (spec.at.x * 0.013 + spec.at.y * 0.007).fract().abs(),
            picture: None,
        }
    }

    /// The emblem on the ground, `side` world units across, at `at`, read from `heading`.
    pub fn make_emblem(&mut self, renderer: &mut Renderer, at: Vec2, heading: f32, side: f32, look: Look, born: f32, life: f32, floor: f32) -> Option<Text> {
        self.sync(renderer.world_text.serial());
        if self.emblem.is_none() {
            let rgba = ta_ui::icon::emblem_rgba(512);
            self.emblem = renderer.world_text.add_picture(512, 512, &rgba);
        }
        let cell = self.emblem?;
        let (s, c) = heading.to_radians().sin_cos();
        Some(Text {
            block: Block::default(),
            anchor: Vec3::new(at.x, LIFT * 0.7, at.y),
            right: Vec3::new(c, 0.0, s),
            up: Vec3::new(s, 0.0, -c),
            size: side,
            look,
            born,
            life,
            floor,
            seed: 0.5,
            picture: Some((cell, side)),
        })
    }

    /// Puts a text in the world.
    pub fn add(&mut self, text: Text) {
        self.texts.push(text);
    }

    /// Takes every text away.
    pub fn clear(&mut self) {
        self.texts.clear();
    }

    /// The instances of every text at `now` (the game's seconds), for the renderer (whose atlas
    /// is `serial`'s).
    pub fn frame(&mut self, serial: u64, now: f32, calm: bool) -> &[GlyphInstance] {
        self.sync(serial);
        self.frame.clear();
        for text in &self.texts {
            text.instances(now, calm, &mut self.frame);
        }
        &self.frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face() -> Face {
        let (bytes, variable) = ta_ui::theme::ui_font_bytes();
        let mut font = FontVec::try_from_vec(bytes).expect("the interface's face");
        if variable {
            let _ = font.set_variation(b"wght", 700.0);
        }
        Face::Owned(font)
    }

    #[test]
    fn the_distance_transform_is_exact() {
        // A 9 x 7 mask with one pixel inside: the others' distance to it is Euclidean.
        let (w, h) = (9, 7);
        let mut mask = vec![false; w * h];
        mask[3 * w + 4] = true;
        let d = distances(&mask, w, h, true);
        for y in 0..h {
            for x in 0..w {
                let want = (((x as f32 - 4.0).powi(2) + (y as f32 - 3.0).powi(2)) as f32).sqrt();
                assert!((d[y * w + x] - want).abs() < 1e-3, "({x}, {y}): {} against {want}", d[y * w + x]);
            }
        }
    }

    #[test]
    fn a_glyphs_field_is_half_on_its_outline_and_signed() {
        let f = face();
        let cut = f.cut('H').expect("an H has an outline");
        assert_eq!(cut.field.len(), cut.width * cut.height);
        // The corner of the cell is far outside, the middle of the H's left stem inside it.
        assert!(cut.field[0] < 30, "outside: {}", cut.field[0]);
        let stem_x = (SPREAD_TEXELS as f32 + 3.0) as usize;
        let mid_y = cut.height / 2;
        assert!(cut.field[mid_y * cut.width + stem_x] > 140, "inside: {}", cut.field[mid_y * cut.width + stem_x]);
        // A glyph stands on its baseline: its cell reaches from under it to a cap's height.
        assert!(cut.bottom < 0.0 && cut.bottom + cut.height_em > 0.6, "{} {}", cut.bottom, cut.height_em);
        // Blank for a space.
        assert!(f.cut(' ').is_none());
    }

    #[test]
    fn an_advance_is_a_fraction_of_an_em() {
        let f = face();
        let a = f.advance('H');
        assert!((0.3..1.0).contains(&a), "{a}");
    }

    #[test]
    fn texts_come_go_and_burn_outwards() {
        // A text on its own (no renderer: the cells are made up).
        let cell = [0.0, 0.0, 0.1, 0.1];
        let lines = vec![BlockLine { glyphs: (0..6).map(|i| Laid { cell, x: i as f32, y: 0.0, w: 0.8, h: 1.0 }).collect(), scale: 1.0, color: [1.0; 3], x0: -3.0, baseline: -0.5 }];
        let block = Block { lines, width: 6.0, height: 1.2 };
        let mut text = Text { block, anchor: Vec3::new(100.0, 0.6, 100.0), right: Vec3::X, up: -Vec3::Z, size: 10.0, look: Look::Burn { at: Vec2::new(40.0, 100.0), after: 0.5 }, born: 2.0, life: 6.0, floor: 0.0, seed: 0.3, picture: None };
        let mut out = Vec::new();
        text.instances(1.0, false, &mut out);
        assert!(out.is_empty(), "not yet");
        // Just after the blast's own moment the ripple has not reached the far letters.
        text.instances(2.6, false, &mut out);
        let near = out.len();
        out.clear();
        text.instances(3.4, false, &mut out);
        assert!(out.len() > near, "{near} then {}: the ripple reaches the rest", out.len());
        assert!(out.iter().all(|g| g.state[0] > 0.3), "hot, then embers");
        out.clear();
        text.instances(8.5, false, &mut out);
        assert!(out.is_empty(), "gone by the end of its life");
        // Painted, it comes in order, left to right.
        text.look = Look::Paint;
        out.clear();
        text.instances(2.5, false, &mut out);
        let first = out.len();
        out.clear();
        text.instances(3.5, false, &mut out);
        assert!(first < out.len());
    }

    /// A picture built by a nanolathe comes up from its near edge, and stays whole once built.
    #[test]
    fn a_nanolathed_picture_is_built_then_whole() {
        let cell = [0.0, 0.0, 0.5, 0.5];
        let text = Text { block: Block::default(), anchor: Vec3::new(100.0, 0.6, 100.0), right: Vec3::X, up: -Vec3::Z, size: 300.0, look: Look::Nano, born: 1.0, life: 10.0, floor: 0.0, seed: 0.5, picture: Some((cell, 300.0)) };
        let reveal = |now: f32| {
            let mut out = Vec::new();
            text.instances(now, false, &mut out);
            assert_eq!(out.len(), 1, "one picture");
            out[0].state[1]
        };
        let (early, half, done) = (reveal(1.2), reveal(1.0 + NANO_PICTURE * 0.5), reveal(1.0 + NANO_PICTURE + 0.5));
        assert!(early < half && half < done, "{early} {half} {done}: building");
        assert!((half - 0.5).abs() < 0.02 && (done - 1.0).abs() < 1e-6, "{half} {done}");
    }

    /// Projected light flickers, and holds steady with reduced flashes (at the flicker's mean).
    #[test]
    fn projected_light_is_steady_with_reduced_flashes() {
        let cell = [0.0, 0.0, 0.1, 0.1];
        let lines = vec![BlockLine { glyphs: vec![Laid { cell, x: 0.0, y: 0.0, w: 0.8, h: 1.0 }], scale: 1.0, color: [1.0; 3], x0: -0.5, baseline: -0.5 }];
        let block = Block { lines, width: 1.0, height: 1.2 };
        let text = Text { block, anchor: Vec3::new(100.0, 0.6, 100.0), right: Vec3::X, up: -Vec3::Z, size: 10.0, look: Look::Light, born: 0.0, life: 60.0, floor: 0.0, seed: 0.3, picture: None };
        let brightness = |now: f32, calm: bool| {
            let mut out = Vec::new();
            text.instances(now, calm, &mut out);
            out[0].color[0]
        };
        let times = [3.0, 3.01, 3.02, 3.05, 3.11];
        let flickering: Vec<f32> = times.iter().map(|&t| brightness(t, false)).collect();
        assert!(flickering.iter().any(|&b| (b - flickering[0]).abs() > 0.02), "it flickers: {flickering:?}");
        assert!(times.iter().all(|&t| (brightness(t, true) - 0.92).abs() < 1e-6), "steady with reduced flashes");
    }
}
