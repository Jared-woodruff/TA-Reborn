//! TA's own interface data for the classic HUD (`hud_classic.rs`), read from the install
//! on first use: GAF art (drawn through `hud_classic_scale` at the size it is shown), the
//! bitmap fonts, the `.gui` layouts, each side's resource-bar layout
//! (`gamedata/sidedata.tdf`) and the pages of every build menu.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use egui::{Color32, Pos2, Rect, TextureHandle, TextureOptions, Vec2, pos2, vec2};
use rayon::prelude::*;
use ta_formats::gaf::Gaf;
use ta_formats::image::{IndexedImage, Palette};
use ta_formats::pcx::PcxImage;
use ta_formats::{fnt, tdf};
use ta_vfs::Vfs;

use crate::hud_classic_scale::{self as scale, Kind, Look, Plane};

/// Build menus have at most this many GUI pages per builder.
const MAX_PAGES: usize = 16;

/// Sizes kept of a picture or a font at once (the art is drawn at one size; a logo at two).
const KEPT: usize = 4;

/// The height of TA's screen, which its side pages are laid out on (TA pixels).
const TA_SCREEN_HEIGHT: f32 = 480.0;

/// The tallest screen a side panel is laid out for (a mod's pages could ask for any).
const TALLEST_SCREEN: f32 = 1536.0;

/// One frame of interface art: TA's picture (`size` in TA pixels), and the textures made of
/// it for the screen, each at the size it is drawn. A size is made on another thread the first
/// time it is asked for (a panel takes a few milliseconds at 4K, a menu's pictures as many),
/// one at a time (a window being resized asks for a new size every frame): meanwhile the size
/// made last stands in, or TA's own pixels.
pub struct Sprite {
    pub size: Vec2,
    name: String,
    source: Arc<Plane>,
    kind: Kind,
    made: Mutex<Vec<Made>>,
    making: Arc<AtomicBool>,
    stand_in: OnceLock<TextureHandle>,
}

/// A texture of a picture: its screen pixels and look, set once it is made, and the frame it
/// was last drawn.
struct Made {
    px: [usize; 2],
    look: Look,
    texture: Arc<OnceLock<TextureHandle>>,
    used: u64,
}

impl Sprite {
    /// From straight RGBA8, `w` × `h`.
    pub fn new(name: String, rgba: &[u8], w: usize, h: usize) -> Sprite {
        let source = Arc::new(Plane::from_rgba8(rgba, w, h));
        Sprite { size: vec2(w as f32, h as f32), name, source, kind: Kind::Interface, made: Mutex::new(Vec::new()), making: Arc::new(AtomicBool::new(false)), stand_in: OnceLock::new() }
    }

    /// A unit's picture (its edges kept softer when smoothed).
    fn picture(self) -> Sprite {
        Sprite { kind: Kind::Picture, ..self }
    }

    /// The picture as a texture `px` screen pixels across and down, as `look` has it: made the
    /// first time it is asked for and kept while it is drawn.
    pub fn texture(&self, ctx: &egui::Context, px: [usize; 2], look: Look) -> egui::TextureId {
        // (No larger than a texture can be: a picture asked for larger is drawn stretched.)
        let most = ctx.input(|i| i.max_texture_side).max(1);
        let px = [px[0].clamp(1, most), px[1].clamp(1, most)];
        let frame = ctx.cumulative_frame_nr();
        let mut made = self.made.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let slot = match made.iter_mut().find(|m| m.px == px && m.look == look) {
            Some(m) => {
                m.used = frame;
                Some(m.texture.clone())
            }
            // (One size made at a time.)
            None if self.making.swap(true, Ordering::AcqRel) => None,
            None => {
                if made.len() >= KEPT
                    && let Some(oldest) = made.iter().enumerate().min_by_key(|(_, m)| m.used).map(|(i, _)| i)
                {
                    made.swap_remove(oldest);
                }
                let slot = Arc::new(OnceLock::new());
                made.push(Made { px, look, texture: slot.clone(), used: frame });
                let (source, kind, name, ctx, done, making) = (self.source.clone(), self.kind, self.name.clone(), ctx.clone(), slot.clone(), self.making.clone());
                rayon::spawn(move || {
                    let started = std::time::Instant::now();
                    let image = scale::enlarge(&source, px[0], px[1], look, kind);
                    log::debug!("classic art {name} at {}x{} ({look:?}) in {:.1} ms", px[0], px[1], started.elapsed().as_secs_f32() * 1000.0);
                    let _ = done.set(ctx.load_texture(format!("{name}@{}x{}", px[0], px[1]), egui::ColorImage::from_rgba_unmultiplied(px, &image.to_rgba8()), TextureOptions::LINEAR));
                    making.store(false, Ordering::Release);
                    ctx.request_repaint();
                });
                Some(slot)
            }
        };
        if let Some(texture) = slot.as_ref().and_then(|s| s.get()) {
            return texture.id();
        }
        // While it is made: the size drawn last (a window just resized), else TA's own pixels.
        if let Some(texture) = made.iter().filter(|m| m.look == look).filter_map(|m| m.texture.get().map(|t| (m.used, t))).max_by_key(|(used, _)| *used).map(|(_, t)| t) {
            return texture.id();
        }
        self.stand_in
            .get_or_init(|| {
                let (w, h) = (self.source.width, self.source.height);
                ctx.load_texture(format!("{}@ta", self.name), egui::ColorImage::from_rgba_unmultiplied([w, h], &self.source.to_rgba8()), TextureOptions::NEAREST)
            })
            .id()
    }
}

/// The frames of a GAF entry (button states, team colours...).
pub type Frames = Arc<[Sprite]>;

/// Air round each letter in its cell, for its smoothed edges (screen pixels).
const CELL_PAD: usize = 2;
/// A letter's place falls between screen pixels: it is drawn at the nearest of these
/// quarters of a pixel, so lettering keeps TA's even spacing at 2.25 or 4.5 screen pixels to one.
const PHASES: usize = 4;
/// The widest letter atlas (screen pixels).
const ATLAS_WIDTH: usize = 2048;
/// The most screen pixels to one of TA's letters are made at (larger lettering is drawn from
/// these, stretched): 12 is a window 5,760 pixels tall, and its atlas a third of the height a
/// graphics card takes.
const MOST_LETTER_SCALE: f32 = 12.0;

/// A TA bitmap font: its letters, drawn white (tinted when drawn) at the size shown, made on
/// another thread the first time a size is asked for (TA's own pixels meanwhile).
pub struct TaFont {
    name: String,
    pub height: f32,
    /// Per character code: its pixels and width (TA pixels; the width includes the space
    /// after it).
    letters: Arc<Vec<Option<fnt::Glyph>>>,
    atlases: Mutex<Vec<FontMade>>,
    /// A size being made (one at a time).
    making: Arc<AtomicBool>,
    /// The letters as TA drew them, one screen pixel to one of TA's, for while a size is made.
    stand_in: OnceLock<Arc<Atlas>>,
    /// For text with letters the font lacks (other scripts): the interface's own font.
    ctx: egui::Context,
}

/// A size of a font's letters, set once it is made, and the frame it was last drawn.
struct FontMade {
    /// Screen pixels to one of TA's, in 64ths.
    key: (u32, Look),
    atlas: Arc<OnceLock<Arc<Atlas>>>,
    used: u64,
}

/// A font's letters at one size, `scale` screen pixels to one of TA's: where each is in the
/// texture, at each quarter-pixel shift made (the shifts a size needs; others are drawn at the
/// nearest made).
struct Atlas {
    scale: f32,
    texture: TextureHandle,
    /// Per character code and quarter: the cell's place in the texture and its size (screen
    /// pixels at `scale`).
    cells: Vec<[Option<(Rect, Vec2)>; PHASES]>,
}

/// The quarter-pixel shifts letters `scale` screen pixels wide fall at: whole multiples of a
/// quarter come round to a few (none at 3, a half at 4.5), others take all four.
fn phases(scale: f32) -> Vec<usize> {
    let quarters = scale * PHASES as f32;
    if (quarters - quarters.round()).abs() > 1e-3 {
        return (0..PHASES).collect();
    }
    let q = quarters.round() as usize;
    let mut out: Vec<usize> = (0..PHASES).map(|k| k * q % PHASES).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// A font's `letters` (`height` tall) drawn at `scale` screen pixels to one of TA's, at the
/// quarter-pixel `shifts`, packed in one texture.
#[allow(clippy::too_many_arguments)]
fn make_atlas(ctx: &egui::Context, name: &str, letters: &[Option<fnt::Glyph>], height: usize, scale: f32, look: Look, shifts: &[usize], filter: TextureOptions) -> Atlas {
    let wanted: Vec<(usize, usize)> = letters.iter().enumerate().filter(|(_, g)| g.is_some()).flat_map(|(c, _)| shifts.iter().map(move |&p| (c, p))).collect();
    let drawn: Vec<(Vec<u8>, usize, usize)> = wanted
        .par_iter()
        .map(|&(c, p)| {
            let g = letters[c].as_ref().expect("a letter");
            scale::letter(&g.pixels, g.width as usize, height, scale, CELL_PAD, p as f32 / PHASES as f32, look)
        })
        .collect();
    // Shelves of cells, as many to a row as fit.
    let ch = drawn.first().map_or(1, |d| d.2);
    let mut places = Vec::with_capacity(drawn.len());
    let (mut x, mut y, mut width) = (0usize, 0usize, 1usize);
    for (_, cw, _) in &drawn {
        if x + cw > ATLAS_WIDTH && x > 0 {
            (x, y) = (0, y + ch + 1);
        }
        places.push((x, y));
        x += cw + 1;
        width = width.max(x);
    }
    let height_px = y + ch;
    let mut rgba = vec![0u8; width * height_px * 4];
    for ((cover, cw, chh), &(px, py)) in drawn.iter().zip(&places) {
        for yy in 0..*chh {
            for xx in 0..*cw {
                let i = ((py + yy) * width + px + xx) * 4;
                rgba[i..i + 4].copy_from_slice(&[255, 255, 255, cover[yy * cw + xx]]);
            }
        }
    }
    let texture = ctx.load_texture(format!("ta-font-{name}@{scale}"), egui::ColorImage::from_rgba_unmultiplied([width, height_px], &rgba), filter);
    let mut cells = vec![[None; PHASES]; letters.len()];
    let (tw, th) = (width as f32, height_px as f32);
    for (((c, p), (_, cw, chh)), &(px, py)) in wanted.iter().zip(&drawn).zip(&places) {
        let uv = Rect::from_min_max(pos2(px as f32 / tw, py as f32 / th), pos2((px + cw) as f32 / tw, (py + chh) as f32 / th));
        cells[*c][*p] = Some((uv, vec2(*cw as f32, *chh as f32)));
    }
    Atlas { scale, texture, cells }
}

impl TaFont {
    fn new(ctx: &egui::Context, name: &str, font: &fnt::Font) -> TaFont {
        TaFont {
            name: name.to_string(),
            height: font.height.max(1) as f32,
            letters: Arc::new(font.glyphs.clone()),
            atlases: Mutex::new(Vec::new()),
            making: Arc::new(AtomicBool::new(false)),
            stand_in: OnceLock::new(),
            ctx: ctx.clone(),
        }
    }

    /// The character code drawn for `c` and its width; capitals stand in for letters a font
    /// lacks.
    fn glyph(&self, c: char) -> Option<(usize, f32)> {
        // (A no-break space, as French sets before ":", is a space.)
        let c = if c == '\u{a0}' { ' ' } else { c };
        let get = |c: char| self.letters.get(c as usize).and_then(Option::as_ref).map(|g| (c as usize, g.width as f32));
        get(c).or_else(|| get(c.to_ascii_uppercase()))
    }

    /// The letters `scale` screen pixels to one of TA's, as `look` has them; asked for the
    /// first time, they are made on another thread and TA's own letters stand in.
    fn atlas(&self, scale: f32, look: Look, frame: u64) -> Arc<Atlas> {
        let key = ((scale.min(MOST_LETTER_SCALE) * 64.0).round().max(1.0) as u32, look);
        let mut atlases = self.atlases.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let slot = match atlases.iter_mut().find(|a| a.key == key) {
            Some(a) => {
                a.used = frame;
                Some(a.atlas.clone())
            }
            // (One size made at a time.)
            None if self.making.swap(true, Ordering::AcqRel) => None,
            None => {
                if atlases.len() >= KEPT
                    && let Some(oldest) = atlases.iter().enumerate().min_by_key(|(_, a)| a.used).map(|(i, _)| i)
                {
                    atlases.swap_remove(oldest);
                }
                let slot = Arc::new(OnceLock::new());
                atlases.push(FontMade { key, atlas: slot.clone(), used: frame });
                let (ctx, name, letters, height, done, making) = (self.ctx.clone(), self.name.clone(), self.letters.clone(), self.height as usize, slot.clone(), self.making.clone());
                let scale = key.0 as f32 / 64.0;
                rayon::spawn(move || {
                    let started = std::time::Instant::now();
                    let atlas = make_atlas(&ctx, &name, &letters, height, scale, look, &phases(scale), TextureOptions::LINEAR);
                    log::debug!("classic font {name} at {scale} ({look:?}) in {:.1} ms", started.elapsed().as_secs_f32() * 1000.0);
                    let _ = done.set(Arc::new(atlas));
                    making.store(false, Ordering::Release);
                    ctx.request_repaint();
                });
                Some(slot)
            }
        };
        if let Some(atlas) = slot.as_ref().and_then(|s| s.get()) {
            return atlas.clone();
        }
        // While it is made: the size drawn last (a window just resized), else TA's own letters.
        if let Some(atlas) = atlases.iter().filter(|a| a.key.1 == look).filter_map(|a| a.atlas.get().map(|t| (a.used, t))).max_by_key(|(used, _)| *used).map(|(_, t)| t.clone()) {
            return atlas;
        }
        self.stand_in.get_or_init(|| Arc::new(make_atlas(&self.ctx, &self.name, &self.letters, self.height as usize, 1.0, Look::Pixels, &[0], TextureOptions::NEAREST))).clone()
    }

    /// Whether the font has every letter of `text` (spaces aside). Other text (Cyrillic,
    /// Chinese...) is lettered in the interface's own font, as tall.
    fn covers(&self, text: &str) -> bool {
        text.chars().all(|c| c == ' ' || c == '\u{a0}' || self.glyph(c).is_some())
    }

    /// The interface's font in place of this one, `scale` screen points per font pixel.
    fn stand_in(&self, scale: f32) -> egui::FontId {
        egui::FontId::proportional((self.height * scale * 0.95).max(6.0))
    }

    /// Width of `text` in font pixels.
    pub fn width(&self, text: &str) -> f32 {
        if !self.covers(text) {
            return self.ctx.fonts_mut(|f| f.layout_no_wrap(text.to_string(), self.stand_in(1.0), Color32::WHITE)).size().x;
        }
        text.chars().filter_map(|c| self.glyph(c)).map(|(_, w)| w).sum()
    }

    /// Draws `text` with its top-left corner at `at`, `scale` screen points per font pixel,
    /// its letters made for the screen's pixels as `look` has them.
    pub fn draw(&self, painter: &egui::Painter, at: Pos2, scale: f32, text: &str, color: Color32, look: Look) {
        if !self.covers(text) {
            let galley = painter.layout_no_wrap(text.to_string(), self.stand_in(scale), color);
            // (In the middle of the font's line.)
            let y = at.y + (self.height * scale - galley.size().y) / 2.0;
            painter.galley(pos2(at.x, y), galley, color);
            return;
        }
        let ppp = painter.pixels_per_point();
        let px = scale * ppp;
        let atlas = self.atlas(px, look, painter.ctx().cumulative_frame_nr());
        let mut mesh = egui::Mesh::with_texture(atlas.texture.id());
        // (Letters made at another size, standing in: stretched to this one.)
        let stretch = px / atlas.scale;
        let exact = (stretch - 1.0).abs() < 0.01;
        let pad = CELL_PAD as f32 * stretch;
        // Each letter on whole screen pixels, drawn shifted the quarter it falls past one.
        let top = if exact { (at.y * ppp).round() } else { at.y * ppp } - pad;
        let mut x = 0.0;
        for c in text.chars() {
            let Some((code, w)) = self.glyph(c) else { continue };
            let place = at.x * ppp + x * px;
            x += w;
            let cells = &atlas.cells[code];
            let (left, cell) = if exact {
                let base = place.floor();
                let want = (place - base) * PHASES as f32;
                // (The nearest shift made, a whole pixel on where that is nearer.)
                let Some(q) = (0..=PHASES).filter(|&q| cells[q % PHASES].is_some()).min_by(|&a, &b| (a as f32 - want).abs().total_cmp(&(b as f32 - want).abs())) else { continue };
                (base + (q / PHASES) as f32, cells[q % PHASES])
            } else {
                (place, cells.iter().flatten().next().copied())
            };
            let Some((uv, size)) = cell else { continue };
            mesh.add_rect_with_uv(Rect::from_min_size(pos2(left - pad, top) / ppp, size * stretch / ppp), uv, color);
        }
        painter.add(egui::Shape::mesh(mesh));
    }

    /// `draw` over a dark copy one font pixel down and right, as TA lettered its panels.
    pub fn draw_shadowed(&self, painter: &egui::Painter, at: Pos2, scale: f32, text: &str, color: Color32, look: Look) {
        self.draw(painter, at + vec2(scale, scale), scale, text, Color32::from_black_alpha(color.a().min(220)), look);
        self.draw(painter, at, scale, text, color, look);
    }
}

/// A gadget of a TA `.gui` file.
pub struct Gadget {
    /// `id` of its COMMON section: 0 panel header, 1 button, 5 label, 7 font...
    pub kind: i32,
    /// Upper case; buttons are named after their art (`ARMATTACK`) or the unit they build.
    pub name: String,
    /// TA pixels, relative to the panel.
    pub rect: Rect,
    /// 16 page tab, 32 build button, 64 order, 256 multi-state button...
    pub attribs: i32,
    pub text: String,
    /// The key that presses it (TA's `quickkey`), a letter in upper case.
    pub quickkey: Option<char>,
}

/// A TA in-game GUI page: a panel with gadgets on it.
pub struct Gui {
    /// Background art (an entry of `anims/commongui.gaf`) and where the panel sits on
    /// TA's 640×480 screen.
    pub panel: String,
    pub origin: Pos2,
    /// The panel's size, as its header gives it (TA's side pages are 128×352).
    pub size: Vec2,
    pub gadgets: Vec<Gadget>,
    /// Where its build buttons go.
    pub grid: Grid,
}

impl Gui {
    fn parse(bytes: &[u8]) -> Gui {
        let doc = tdf::parse_bytes(bytes);
        let mut gui = Gui { panel: String::new(), origin: Pos2::ZERO, size: Vec2::ZERO, gadgets: Vec::new(), grid: Grid::TA };
        for s in &doc.sections {
            let Some(c) = s.section("COMMON") else { continue };
            let kind = c.get_i32_or("id", -1);
            let at = pos2(c.get_i32_or("xpos", 0) as f32, c.get_i32_or("ypos", 0) as f32);
            let rect = Rect::from_min_size(at, vec2(c.get_i32_or("width", 0) as f32, c.get_i32_or("height", 0) as f32));
            if kind == 0 {
                gui.panel = s.get_str("panel", "").trim().to_ascii_uppercase();
                gui.origin = at;
                gui.size = rect.size();
                continue;
            }
            gui.gadgets.push(Gadget {
                kind,
                name: c.get_str("name", "").trim().to_ascii_uppercase(),
                rect,
                attribs: c.get_i32_or("attribs", 0),
                text: s.get_str("text", "").trim().to_string(),
                quickkey: quickkey(&s.get_str("quickkey", "")),
            });
        }
        gui.grid = Grid::of(gui.gadgets.iter().filter(|g| g.is_build_place()).map(|g| g.rect));
        gui
    }

    /// How far down TA's screen the page reaches (TA pixels): the foot of its panel, or of its
    /// lowest gadget.
    pub fn bottom(&self) -> f32 {
        let gadgets = self.gadgets.iter().filter(|g| g.kind == 1).map(|g| g.rect.max.y).fold(0.0, f32::max);
        self.origin.y + self.size.y.max(gadgets)
    }
}

impl Gadget {
    /// A button of a build page's grid (a unit's, or the `IGPATCH` filling a place left empty).
    pub fn is_build_place(&self) -> bool {
        self.kind == 1 && self.attribs & 32 != 0
    }
}

/// Where a build page's buttons go: two columns 64 pixels apart, and rows 64 pixels apart from
/// `top` down (TA pixels, from the panel's corner).
///
/// TA's pages have three rows from 27 pixels down, under the ORDERS and BUILD tabs: six places.
/// The pages of the mods made for the patched engines of later years (TA: Escalation's, Total
/// Mayhem's) fill a side panel as tall as a 768-pixel screen, with the orders on the same page:
/// six rows from the panel's top, twelve places. A page's own buttons say which it has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub top: f32,
    pub rows: usize,
}

impl Grid {
    /// TA's own: three rows from 27 pixels down.
    pub const TA: Grid = Grid { top: 27.0, rows: 3 };

    /// The grid of a page's build buttons at `rects`: TA's, or as many rows from as high as
    /// they go (never fewer than TA's three, nor starting lower than TA's first row).
    fn of(rects: impl Iterator<Item = Rect> + Clone) -> Grid {
        let Some(top) = rects.clone().map(|r| r.min.y).reduce(f32::min) else { return Grid::TA };
        let top = top.min(Grid::TA.top);
        // (At most a 4096-pixel panel's worth of rows.)
        let last = rects.map(|r| ((r.min.y - top) / 64.0).round().clamp(0.0, 63.0) as usize).max().unwrap_or(0);
        Grid { top, rows: (last + 1).max(Grid::TA.rows) }
    }

    /// How many places a page has.
    pub fn places(&self) -> usize {
        self.rows * 2
    }

    /// Which place a button at `rect` takes.
    pub fn place_of(&self, rect: Rect) -> usize {
        let col = (rect.min.x / 64.0).round().clamp(0.0, 1.0) as usize;
        let row = ((rect.min.y - self.top) / 64.0).round().clamp(0.0, (self.rows - 1) as f32) as usize;
        row * 2 + col
    }

    /// Whether a button at `rect` is a place of the grid, as TA's are (64 pixels square, on the
    /// grid). The mods' pages also have buttons over a share of a place ([`Grid::share_of`])
    /// and buttons over several (128 square, a Krogoth's).
    pub fn is_place(&self, rect: Rect) -> bool {
        let square = self.square(self.place_of(rect));
        (rect.min - square.min).abs().max_elem() <= 4.0 && (rect.size() - square.size()).abs().max_elem() <= 4.0
    }

    /// The place a button at `rect` is a share of, if it is one: no larger than a place, off
    /// the grid, in the place its top left corner is in (and seen only there: what reaches out
    /// of it is under the next place's button, or off the panel). Total Mayhem's pages have
    /// four round one picture, a long-range gun's variants, each facing another way (the side
    /// of the picture clicked is the way the gun faces), and two halves, a plant's for land and
    /// for the sea.
    pub fn share_of(&self, rect: Rect) -> Option<usize> {
        if self.is_place(rect) || rect.width() > 68.0 || rect.height() > 68.0 {
            return None;
        }
        (0..self.places()).find(|&p| {
            let square = self.square(p);
            rect.min.x >= square.min.x - 2.0 && rect.min.x < square.max.x - 2.0 && rect.min.y >= square.min.y - 2.0 && rect.min.y < square.max.y - 2.0
        })
    }

    /// The square of place `place` (TA pixels, from the panel's corner).
    pub fn square(&self, place: usize) -> Rect {
        Rect::from_min_size(pos2((place % 2) as f32 * 64.0, self.top + (place / 2) as f32 * 64.0), vec2(64.0, 64.0))
    }

    /// The places a button at `rect` covers: those it takes more than a sliver of.
    pub fn covered(&self, rect: Rect) -> Vec<usize> {
        (0..self.places()).filter(|&p| {
            let common = self.square(p).intersect(rect);
            common.width() > 4.0 && common.height() > 4.0
        }).collect()
    }
}

/// A gadget's `quickkey`: TA writes the key's character code (`97`: A), sometimes the letter
/// itself (`O`), and 0 for none. Letters only, in upper case (TA matched them in either).
pub fn quickkey(raw: &str) -> Option<char> {
    let raw = raw.trim();
    let c = match raw.parse::<u32>() {
        Ok(0) => return None,
        Ok(code) => char::from_u32(code)?,
        Err(_) => {
            let mut chars = raw.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            c
        }
    };
    c.is_ascii_alphabetic().then(|| c.to_ascii_uppercase())
}

/// A side's on-screen furniture from `gamedata/sidedata.tdf`: fonts and the boxes where the
/// resource bar and the unit bar show their readouts, on TA's 640×480 screen.
pub struct SideLayout {
    /// Lettering of the bars and of the GUI buttons (`fonts/*.fnt`).
    pub font: String,
    pub font_gui: String,
    boxes: Vec<(String, Rect)>,
}

impl SideLayout {
    /// A named box (`METALBAR`, `UNITNAME`...), its top-left corner first.
    pub fn get(&self, name: &str) -> Option<Rect> {
        self.boxes.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, r)| *r)
    }
}

/// A place of a builder's own build page that several of its units share ([`Grid::share_of`]):
/// each unit with its zone, the share of the place its button is seen over (0..1 across and
/// down), in the page's order. A point of the place is the first zone's that holds it, as TA
/// had it: Total Mayhem's Core Intimidator lays its north variant over the others.
pub type SharedPlace = Vec<(usize, Rect)>;

/// The zone of a shared place that a point of it (a share of the place, 0..1) is in.
pub fn zone_at(place: &[(usize, Rect)], at: Pos2) -> Option<usize> {
    place.iter().position(|(_, zone)| zone.contains(at))
}

/// The unit a shared place shows when nothing points at one of its zones, among those `shown`
/// says may be: the one the others are named after (`ARMBRTHA`, of `ARMBRTHAE`, `ARMBRTHAW` and
/// `ARMBRTHAN`), else the one with the largest zone (the first of those as large).
pub fn plain_of(types: &ta_sim::UnitTypes, place: &[(usize, Rect)], shown: impl Fn(usize) -> bool) -> Option<usize> {
    let units: Vec<&(usize, Rect)> = place.iter().filter(|(u, _)| shown(*u)).collect();
    let named_after = |a: usize, b: usize| types.get(b).name.to_ascii_uppercase().starts_with(&types.get(a).name.to_ascii_uppercase());
    // (Of zones as large, the first.)
    let largest = || units.iter().fold(None, |best: Option<&&(usize, Rect)>, z| if best.is_some_and(|b| b.1.area() >= z.1.area()) { best } else { Some(z) });
    units.iter().find(|(a, _)| units.iter().all(|(b, _)| named_after(*a, *b))).or_else(largest).map(|(u, _)| *u)
}

/// A place of a page being gathered into a [`SharedPlace`]: the place, the shares over it, and
/// its own button.
type PlaceZones = (usize, SharedPlace, Option<(usize, Rect)>);

/// The places of `builder`'s own build pages that several of the units it builds share, each
/// with its zones (a place's own button among them, the shares over it after it).
pub fn shared_places(vfs: &Vfs, types: &ta_sim::UnitTypes, builder: &ta_sim::UnitType) -> Vec<SharedPlace> {
    let name = builder.name.to_ascii_lowercase();
    let mut out = Vec::new();
    // (As `Art::menu`: the pages run from 1 until one is missing.)
    for n in 1..=MAX_PAGES {
        let Some(gui) = vfs.read_opt(&format!("guis/{name}{n}.gui")).map(|bytes| Gui::parse(&bytes)) else {
            if n > 1 {
                break;
            }
            continue;
        };
        // (By place: the shares, in the page's order, and the place's own button, which holds
        // what the shares over it leave.)
        let mut places: Vec<PlaceZones> = Vec::new();
        for g in gui.gadgets.iter().filter(|g| g.kind == 1 && g.attribs & 32 != 0) {
            let Some(id) = types.id(&g.name).filter(|id| builder.build_options.contains(id)) else { continue };
            let (place, own) = match gui.grid.share_of(g.rect) {
                Some(place) => (place, false),
                None if gui.grid.is_place(g.rect) => (gui.grid.place_of(g.rect), true),
                None => continue,
            };
            let square = gui.grid.square(place);
            let seen = g.rect.intersect(square);
            let zone = Rect::from_min_max(((seen.min - square.min) / 64.0).to_pos2(), ((seen.max - square.min) / 64.0).to_pos2());
            let at = match places.iter().position(|(p, ..)| *p == place) {
                Some(at) => at,
                None => {
                    places.push((place, Vec::new(), None));
                    places.len() - 1
                }
            };
            let (_, shares, whole) = &mut places[at];
            if own {
                whole.get_or_insert((id, zone));
            } else if !shares.iter().any(|(u, _)| *u == id) {
                shares.push((id, zone));
            }
        }
        for (_, mut zones, whole) in places {
            if let Some(whole) = whole
                && !zones.iter().any(|(u, _)| *u == whole.0)
            {
                zones.push(whole);
            }
            if zones.len() > 1 {
                out.push(zones);
            }
        }
    }
    out
}

/// One page of a build menu: the GUI laying it out and the unit type in each place.
#[derive(Clone, Debug, Default)]
pub struct MenuPage {
    /// `guis/<gui>.gui`: the builder's own page, or another page's layout for pages made
    /// up from download entries and build lists.
    pub gui: String,
    /// One a place of the GUI's [`Grid`] (TA's six, or a mod's twelve). A place one of the
    /// page's own `odd` buttons covers holds that button's unit.
    pub slots: Vec<Option<usize>>,
    /// The page's own build buttons that are not a place each, by their index among the GUI's
    /// gadgets, with their units: a share of a place (Total Mayhem's long-range guns, a variant
    /// for each way they face round one picture; its plants' halves, land and sea) or several
    /// places (a Krogoth's button). They keep their own rectangles ([`Grid::is_place`]).
    pub odd: Vec<(usize, usize)>,
}

impl MenuPage {
    /// The unit of the page's build button `index` (its index among the GUI's gadgets) at
    /// `rect`: an odd one's own, else its place's.
    pub fn unit_at(&self, grid: &Grid, index: usize, rect: Rect) -> Option<usize> {
        if !grid.is_place(rect) {
            return self.odd.iter().find(|&&(i, _)| i == index).map(|&(_, unit)| unit);
        }
        self.slots.get(grid.place_of(rect)).copied().flatten()
    }

    /// Every unit on the page (one covering several places, once).
    pub fn units(&self) -> impl Iterator<Item = usize> + '_ {
        let mut seen = Vec::new();
        self.slots.iter().flatten().copied().chain(self.odd.iter().map(|&(_, unit)| unit)).filter(move |u| {
            let new = !seen.contains(u);
            seen.push(*u);
            new
        })
    }
}

/// A download TDF menu entry: builder, `MENU`, `BUTTON`, unit.
type MenuEntry = (String, u32, u32, String);

/// Interface assets, loaded on first use and kept.
#[derive(Default)]
pub struct Art {
    gafs: HashMap<String, Option<Gaf>>,
    frames: HashMap<(String, String, Kind), Option<Frames>>,
    buttons: HashMap<(String, String), Option<Frames>>,
    fonts: HashMap<String, Option<Arc<TaFont>>>,
    guis: HashMap<String, Option<Arc<Gui>>>,
    sides: Option<Vec<(String, Arc<SideLayout>)>>,
    downloads: Option<Vec<MenuEntry>>,
    menus: HashMap<usize, Arc<Vec<MenuPage>>>,
    plates: HashMap<(u32, u32), Arc<Sprite>>,
}

fn sprite(name: String, image: &IndexedImage, palette: &Palette) -> Sprite {
    let (w, h) = (image.width.max(1) as usize, image.height.max(1) as usize);
    let rgba = if image.pixels.is_empty() { vec![0; w * h * 4] } else { image.to_rgba(palette) };
    Sprite::new(name, &rgba, w, h)
}

impl Art {
    /// The frames of entry `entry` of `anims/<gaf>.gaf`.
    pub fn frames(&mut self, vfs: &Vfs, palette: &Palette, gaf: &str, entry: &str) -> Option<Frames> {
        self.frames_of(vfs, palette, gaf, entry, Kind::Interface)
    }

    fn frames_of(&mut self, vfs: &Vfs, palette: &Palette, gaf: &str, entry: &str, kind: Kind) -> Option<Frames> {
        let key = (gaf.to_ascii_lowercase(), entry.to_ascii_uppercase(), kind);
        if let Some(f) = self.frames.get(&key) {
            return f.clone();
        }
        let path = format!("anims/{}.gaf", key.0);
        let parsed = self.gafs.entry(key.0.clone()).or_insert_with(|| vfs.read_opt(&path).and_then(|b| Gaf::parse(&b).ok()));
        let frames = parsed.as_ref().and_then(|g| g.entry(entry)).filter(|e| !e.frames.is_empty()).map(|e| {
            e.frames.iter().enumerate().map(|(i, f)| Sprite { kind, ..sprite(format!("{path}:{entry}:{i}"), &f.image, palette) }).collect::<Vec<_>>().into()
        });
        self.frames.insert(key, frames.clone());
        frames
    }

    /// The art of the button building `unit` on a page laid out by `guis/<gui>.gui`: the
    /// page's own GAF, the unit's download gadget (`anims/<unit>_gadget.gaf`), or its build
    /// picture as a single frame.
    pub fn unit_button(&mut self, vfs: &Vfs, palette: &Palette, gui: &str, unit: &str) -> Option<Frames> {
        let key = (gui.to_ascii_lowercase(), unit.to_ascii_uppercase());
        if let Some(f) = self.buttons.get(&key) {
            return f.clone();
        }
        let lower = unit.to_ascii_lowercase();
        let found = [key.0.clone(), format!("{lower}_gadget")]
            .iter()
            .filter(|g| vfs.exists(&format!("anims/{g}.gaf")))
            .find_map(|g| self.frames_of(vfs, palette, g, unit, Kind::Picture))
            .or_else(|| {
                let pcx = PcxImage::parse(&vfs.read_opt(&format!("unitpics/{lower}.pcx"))?).ok()?;
                let picture = Sprite::new(format!("unitpics/{lower}.pcx"), &pcx.to_rgba(), pcx.width as usize, pcx.height as usize).picture();
                // (Shown in TA's 64-pixel place whatever its own size.)
                Some(vec![Sprite { size: vec2(64.0, 64.0), ..picture }].into())
            });
        self.buttons.insert(key, found.clone());
        found
    }

    /// A blank button face `w`×`h` TA pixels for commands TA has no art for, made like its
    /// order buttons: speckled dark metal, a light rim along the left and bottom edges (and
    /// enlarged as they are).
    pub fn plate(&mut self, w: u32, h: u32) -> Arc<Sprite> {
        let (w, h) = (w.max(4), h.max(4));
        self.plates
            .entry((w, h))
            .or_insert_with(|| {
                // TA's button greys, with a few of its blue specks.
                const METAL: [[u8; 3]; 8] = [[43, 43, 43], [59, 59, 59], [59, 59, 59], [75, 75, 75], [43, 43, 43], [31, 31, 31], [35, 55, 79], [59, 59, 59]];
                let mut seed = 0x2545_f491u32 ^ (w << 16) ^ h;
                let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                for y in 0..h {
                    for x in 0..w {
                        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        let mut c = METAL[(seed >> 24) as usize % METAL.len()];
                        if x == 0 || y == 0 {
                            c = [23, 23, 23];
                        } else if x == w - 1 || y == h - 1 {
                            c = [15, 15, 15];
                        } else if x == 1 || y == h - 2 {
                            c = [123, 123, 123];
                        } else if x == w - 2 || y == 1 {
                            c = [31, 31, 31];
                        }
                        rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
                    }
                }
                Arc::new(Sprite::new(format!("classic-plate-{w}x{h}"), &rgba, w as usize, h as usize))
            })
            .clone()
    }

    /// A font from `fonts/` (or `guis/`).
    pub fn font(&mut self, ctx: &egui::Context, vfs: &Vfs, name: &str) -> Option<Arc<TaFont>> {
        let name = name.to_ascii_lowercase();
        self.fonts
            .entry(name.clone())
            .or_insert_with(|| {
                let bytes = vfs.read_opt(&format!("fonts/{name}.fnt")).or_else(|| vfs.read_opt(&format!("guis/{name}.fnt")))?;
                match fnt::Font::parse(&bytes) {
                    Ok(f) => Some(Arc::new(TaFont::new(ctx, &name, &f))),
                    Err(e) => {
                        log::warn!("font {name}: {e}");
                        None
                    }
                }
            })
            .clone()
    }

    /// How tall a screen a side's panel is laid out for (TA pixels): as far down as its orders
    /// (`<prefix>gen`), main and Commander's first build (`template`) pages reach, TA's 480 at
    /// least. The mods made for the patched engines of later years (TA: Escalation, Total
    /// Mayhem) lay their pages out on a 768-pixel screen, the orders under twelve build places:
    /// on TA's 480 their arrows and orders fell off the foot of the panel.
    pub fn layout_height(&mut self, vfs: &Vfs, prefix: &str, template: &str) -> f32 {
        let prefix = prefix.to_ascii_lowercase();
        [format!("{prefix}gen"), format!("{prefix}main"), template.to_ascii_lowercase()]
            .iter()
            .filter_map(|page| self.gui(vfs, page))
            .map(|gui| gui.bottom())
            .fold(TA_SCREEN_HEIGHT, f32::max)
            .min(TALLEST_SCREEN)
    }

    /// `guis/<name>.gui`.
    pub fn gui(&mut self, vfs: &Vfs, name: &str) -> Option<Arc<Gui>> {
        let name = name.to_ascii_lowercase();
        self.guis.entry(name.clone()).or_insert_with(|| vfs.read_opt(&format!("guis/{name}.gui")).map(|b| Arc::new(Gui::parse(&b)))).clone()
    }

    /// A side's layout (by side name, e.g. `ARM`); sides sidedata.tdf doesn't lay out
    /// borrow the first side's.
    pub fn side_layout(&mut self, vfs: &Vfs, side: &str) -> Option<Arc<SideLayout>> {
        let sides = self.sides.get_or_insert_with(|| {
            let Some(doc) = vfs.read_opt("gamedata/sidedata.tdf").map(|b| tdf::parse_bytes(&b)) else { return Vec::new() };
            doc.sections
                .iter()
                .filter(|s| s.name.to_ascii_uppercase().starts_with("SIDE"))
                .map(|s| {
                    let boxes = s
                        .sections
                        .iter()
                        .map(|b| {
                            let (x1, y1) = (b.get_i32_or("x1", 0) as f32, b.get_i32_or("y1", 0) as f32);
                            let (x2, y2) = (b.get_i32_or("x2", 0) as f32, b.get_i32_or("y2", 0) as f32);
                            // Some boxes only mean their first corner (the second is junk).
                            (b.name.to_ascii_uppercase(), Rect::from_min_max(pos2(x1, y1), pos2(x2.max(x1), y2.max(y1))))
                        })
                        .collect();
                    let layout = SideLayout { font: s.get_str("font", "console"), font_gui: s.get_str("fontgui", "armbutt"), boxes };
                    (s.get_str("name", "").trim().to_ascii_uppercase(), Arc::new(layout))
                })
                .collect()
        });
        sides.iter().find(|(n, _)| n.eq_ignore_ascii_case(side)).or(sides.first()).map(|(_, l)| l.clone())
    }

    fn downloads(&mut self, vfs: &Vfs) -> &[MenuEntry] {
        self.downloads.get_or_insert_with(|| {
            let mut out = Vec::new();
            for path in vfs.list("download", Some("tdf"), false) {
                let Some(doc) = vfs.read_opt(&path).map(|b| tdf::parse_bytes(&b)) else { continue };
                for e in &doc.sections {
                    let (Some(builder), Some(unit)) = (e.get("UNITMENU"), e.get("UNITNAME")) else { continue };
                    let menu = e.get_i32_or("MENU", 2).max(0) as u32;
                    let button = e.get_i32_or("BUTTON", 0).max(0) as u32;
                    out.push((builder.trim().to_ascii_uppercase(), menu, button, unit.trim().to_ascii_uppercase()));
                }
            }
            out
        })
    }

    /// The pages of a builder's build menu as TA laid them out: the units of its own GUI
    /// pages where those put them, download entries in the place they ask for, then the rest
    /// of what it can build (`build_options`, e.g. 3.1's CANBUILD lists) in free places.
    /// Pages without a GUI of their own use `template`'s layout.
    pub fn menu(&mut self, vfs: &Vfs, types: &ta_sim::UnitTypes, builder: usize, template: &str) -> Arc<Vec<MenuPage>> {
        if let Some(m) = self.menus.get(&builder) {
            return m.clone();
        }
        let t = types.get(builder);
        let options = t.build_options.clone();
        let name = t.name.to_ascii_lowercase();
        let mut pages: Vec<MenuPage> = Vec::new();
        let mut placed: Vec<usize> = Vec::new();
        let mut own_layout: Option<String> = None;
        // Like `ta_data`: the pages run from 1 until one is missing.
        for n in 1..=MAX_PAGES {
            let gui_name = format!("{name}{n}");
            let Some(gui) = self.gui(vfs, &gui_name) else {
                if n > 1 {
                    break;
                }
                continue;
            };
            let mut page = MenuPage { gui: gui_name.clone(), slots: vec![None; gui.grid.places()], odd: Vec::new() };
            let builds = || gui.gadgets.iter().enumerate().filter(|(_, g)| g.attribs & 32 != 0);
            for (_, g) in builds().filter(|(_, g)| gui.grid.is_place(g.rect)) {
                let Some(id) = types.id(&g.name).filter(|id| options.contains(id) && !placed.contains(id)) else { continue };
                let slot = gui.grid.place_of(g.rect);
                if page.slots[slot].is_none() {
                    page.slots[slot] = Some(id);
                    placed.push(id);
                }
            }
            // The rest keep their own rectangles, the places they cover taken: a share of a
            // place, the place it is in; a button over several, those. (Rounded to a place each,
            // Total Mayhem's four variants of a gun round one picture took three places and the
            // metal maker's below, and its other buildings went to other pages.)
            for (index, g) in builds().filter(|(_, g)| !gui.grid.is_place(g.rect)) {
                let Some(id) = types.id(&g.name).filter(|id| options.contains(id) && !placed.contains(id)) else { continue };
                page.odd.push((index, id));
                placed.push(id);
                let covered = gui.grid.share_of(g.rect).map_or_else(|| gui.grid.covered(g.rect), |p| vec![p]);
                for p in covered {
                    page.slots[p].get_or_insert(id);
                }
            }
            own_layout.get_or_insert(gui_name);
            pages.push(page);
        }
        let layout = own_layout.unwrap_or_else(|| template.to_string());
        // (Pages made up here have as many places as that layout.)
        let places = self.gui(vfs, &layout).map_or(Grid::TA.places(), |g| g.grid.places());
        let place = |pages: &mut Vec<MenuPage>, id: usize, page: usize, slot: usize| {
            // The asked-for place (or its page's last), else the next free one after it.
            let (mut p, mut s) = (page, None);
            loop {
                while pages.len() <= p {
                    pages.push(MenuPage { gui: layout.clone(), slots: vec![None; places], odd: Vec::new() });
                }
                let slots = &mut pages[p].slots;
                let mut at = s.unwrap_or_else(|| slot.min(slots.len().saturating_sub(1)));
                while at < slots.len() {
                    if slots[at].is_none() {
                        slots[at] = Some(id);
                        return;
                    }
                    at += 1;
                }
                (p, s) = (p + 1, Some(0));
            }
        };
        // Download entries count the orders menu as menu 1: MENU=2 is the first build page.
        let entries: Vec<MenuEntry> = self.downloads(vfs).iter().filter(|(b, ..)| b.eq_ignore_ascii_case(&t.name)).cloned().collect();
        for (_, menu, button, unit) in entries {
            let Some(id) = types.id(&unit).filter(|id| options.contains(id) && !placed.contains(id)) else { continue };
            place(&mut pages, id, menu.saturating_sub(2) as usize, button as usize);
            placed.push(id);
        }
        for &id in &options {
            if !placed.contains(&id) {
                place(&mut pages, id, 0, 0);
                placed.push(id);
            }
        }
        while pages.len() > 1 && pages.last().is_some_and(|p| p.slots.iter().all(Option::is_none)) {
            pages.pop();
        }
        let pages = Arc::new(pages);
        self.menus.insert(builder, pages.clone());
        pages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_are_made_at_the_shifts_their_size_falls_at() {
        // Three screen pixels to TA's one: every letter starts on a whole pixel; 4.5, on a
        // whole or a half; 2.25, on any quarter; sizes between quarters, all four.
        assert_eq!(phases(3.0), [0]);
        assert_eq!(phases(4.5), [0, 2]);
        assert_eq!(phases(2.25), [0, 1, 2, 3]);
        assert_eq!(phases(2.0), [0]);
        assert_eq!(phases(2.6), [0, 1, 2, 3]);
    }

    #[test]
    fn build_places_run_two_across_three_down() {
        let at = |x: f32, y: f32| Grid::TA.place_of(Rect::from_min_size(pos2(x, y), vec2(64.0, 64.0)));
        assert_eq!([at(0.0, 27.0), at(64.0, 27.0), at(0.0, 91.0), at(64.0, 91.0), at(0.0, 155.0), at(64.0, 155.0)], [0, 1, 2, 3, 4, 5]);
    }

    /// A build page as the patched engines' mods lay them out (TA: Escalation's `ARMCK1.gui`):
    /// a panel as tall as a 768-pixel screen, twelve places from its top, the page arrows and
    /// the orders under them.
    const TALL_PAGE: &[u8] = b"[GADGET0]{[COMMON]{id=0;name=HEADER;xpos=0;ypos=128;width=128;height=640;}panel=ARMPAN;}\n\
        [GADGET1]{[COMMON]{id=1;name=ARMSOLAR;xpos=0;ypos=0;width=64;height=64;attribs=32;}}\n\
        [GADGET2]{[COMMON]{id=1;name=ARMWIN;xpos=64;ypos=0;width=64;height=64;attribs=32;}}\n\
        [GADGET3]{[COMMON]{id=1;name=ARMLAB;ypos=256;width=64;height=64;attribs=32;}}\n\
        [GADGET4]{[COMMON]{id=1;name=IGPATCH;xpos=64;ypos=320;width=64;height=64;attribs=32;}}\n\
        [GADGET5]{[COMMON]{id=1;name=ARMPREV;xpos=8;ypos=388;width=44;height=16;attribs=2;}}\n\
        [GADGET6]{[COMMON]{id=1;name=ARMSTOP;xpos=66;ypos=543;width=55;height=31;attribs=0;}}\n\
        [GADGET7]{[COMMON]{id=1;name=ARMATTACK;xpos=7;ypos=607;width=55;height=31;attribs=64;}}";

    #[test]
    fn a_tall_page_has_the_places_its_buttons_take() {
        let gui = Gui::parse(TALL_PAGE);
        assert_eq!(gui.grid, Grid { top: 0.0, rows: 6 }, "six rows from the panel's top");
        assert_eq!(gui.grid.places(), 12);
        let place = |name: &str| gui.grid.place_of(gui.gadgets.iter().find(|g| g.name == name).unwrap().rect);
        assert_eq!([place("ARMSOLAR"), place("ARMWIN"), place("ARMLAB"), place("IGPATCH")], [0, 1, 8, 11], "each in a place of its own");
        assert_eq!(gui.bottom(), 768.0, "it reaches the foot of a 768-pixel screen");
        // TA's own page stays as it was: six places from 27 pixels down, ending at TA's 480.
        let ta = Gui::parse(
            b"[GADGET0]{[COMMON]{id=0;name=HEADER;xpos=0;ypos=128;width=128;height=352;}panel=ARMPAN;}\n\
              [GADGET1]{[COMMON]{id=1;name=ARMSOLAR;xpos=0;ypos=27;width=64;height=64;attribs=32;}}\n\
              [GADGET2]{[COMMON]{id=1;name=ARMMEX;xpos=0;ypos=155;width=64;height=64;attribs=32;}}",
        );
        assert_eq!((ta.grid, ta.bottom()), (Grid::TA, 480.0));
        // A page that fills only its lower rows keeps TA's grid, not one starting lower.
        let low = Gui::parse(b"[GADGET0]{[COMMON]{id=0;name=HEADER;ypos=128;width=128;height=352;}}\n[GADGET1]{[COMMON]{id=1;name=X;xpos=64;ypos=155;width=64;height=64;attribs=32;}}");
        assert_eq!(low.grid, Grid::TA);
    }

    #[test]
    fn quickkeys_read_as_codes_or_letters() {
        assert_eq!(quickkey("97"), Some('A'));
        assert_eq!(quickkey("83"), Some('S'));
        assert_eq!(quickkey("O"), Some('O'));
        assert_eq!(quickkey("0"), None);
        assert_eq!(quickkey(""), None);
        assert_eq!(quickkey("13"), None);
    }

    #[test]
    fn gui_pages_read_their_panel_and_gadgets() {
        let gui = Gui::parse(
            b"[GADGET0]{[COMMON]{id=0;name=HEADER;xpos=0;ypos=128;width=128;height=352;}panel=ARMPAN;}\n\
              [GADGET1]{[COMMON]{id=1;name=ARMSTOP;xpos=64;ypos=247;width=55;height=31;attribs=0;}}\n\
              [GADGET2]{[COMMON]{id=1;name=armsolar;xpos=0;ypos=27;width=64;height=64;attribs=32;}text=;}",
        );
        assert_eq!((gui.panel.as_str(), gui.origin), ("ARMPAN", pos2(0.0, 128.0)));
        assert_eq!(gui.gadgets.len(), 2, "the header is the panel, not a gadget");
        assert_eq!(gui.gadgets[0].rect, Rect::from_min_size(pos2(64.0, 247.0), vec2(55.0, 31.0)));
        assert_eq!((gui.gadgets[1].name.as_str(), gui.gadgets[1].attribs), ("ARMSOLAR", 32));
    }

    fn install() -> Option<Vfs> {
        let install = ta_vfs::install::discover(None).ok()?;
        let mut vfs = Vfs::new();
        vfs.mount_game_dir(&install.root, "base").ok()?;
        Some(vfs)
    }

    #[test]
    fn build_menus_keep_the_pages_ta_laid_out() {
        let Some(vfs) = install() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let data = ta_data::GameData::load(&vfs);
        let types = ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p));
        let mut art = Art::default();
        let mut menu = |name: &str| art.menu(&vfs, &types, types.id(name).unwrap(), "armcom1");
        let names = |page: &MenuPage| -> Vec<Option<String>> { page.slots.iter().map(|s| s.map(|id| types.get(id).name.clone())).collect() };
        let some = |list: &[&str]| -> Vec<Option<String>> { list.iter().map(|n| Some(n.to_string())).collect() };
        // The Commander's four GUI pages as they are.
        let com = menu("ARMCOM");
        assert_eq!(com.len(), 4);
        assert_eq!(names(&com[0]), some(&["ARMSOLAR", "ARMWIN", "ARMESTOR", "ARMMSTOR", "ARMMEX", "ARMMAKR"]));
        assert_eq!(names(&com[3])[0].as_deref(), Some("ARMTL"));
        // Download entries fill the places a GUI page leaves empty (MENU=3 is its second
        // page), and make up pages of their own in the builder's layout.
        let ack = menu("ARMACK");
        assert_eq!(names(&ack[1])[3..], some(&["ARMFLAK", "ARMFORT", "ARMAMB"])[..]);
        assert_eq!((ack[2].gui.as_str(), names(&ack[2])[0].as_deref()), ("armack1", Some("ARMCKFUS")));
        // Everything a builder can build is on a page, once.
        for b in ["ARMCOM", "ARMACK", "ARMLAB", "CORCOM", "CORCK", "CORAVP"] {
            let mut shown: Vec<usize> = menu(b).iter().flat_map(|p| p.units().collect::<Vec<_>>()).collect();
            let mut want = types.get(types.id(b).unwrap()).build_options.clone();
            shown.sort_unstable();
            want.sort_unstable();
            assert_eq!(shown, want, "{b}");
        }
    }

    /// A Commander's build menu as a mod made for the patched engines lays it out, over the
    /// install: its own pages of twelve places hold its units where they put them, the side panel
    /// is laid out on a 768-pixel screen, and everything the Commander builds is on a page once.
    #[test]
    fn a_mods_tall_pages_keep_their_twelve_places() {
        let Some(mut vfs) = install() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let mut art = Art::default();
        assert_eq!(art.layout_height(&vfs, "ARM", "armcom1"), 480.0, "TA's own pages: TA's screen");
        let dir = std::env::temp_dir().join(format!("ta_classic_tall_{}", std::process::id()));
        let guis = dir.join("guis");
        std::fs::create_dir_all(&guis).unwrap();
        let page = |units: &[&str]| {
            let mut s = String::from("[GADGET0]{[COMMON]{id=0;name=HEADER;xpos=0;ypos=128;width=128;height=640;}panel=ARMPAN;}\n");
            for i in 0..12 {
                let name = units.get(i).copied().unwrap_or("IGPATCH");
                s += &format!("[GADGET{}]{{[COMMON]{{id=1;name={name};xpos={};ypos={};width=64;height=64;attribs=32;commonattribs=4;}}}}\n", i + 1, (i % 2) * 64, (i / 2) * 64);
            }
            s + "[GADGET13]{[COMMON]{id=1;name=ARMNEXT;xpos=74;ypos=388;width=44;height=16;attribs=2;}}\n\
                 [GADGET14]{[COMMON]{id=1;name=ARMATTACK;xpos=7;ypos=607;width=55;height=31;attribs=64;}}\n"
        };
        let first = ["ARMSOLAR", "ARMWIN", "ARMESTOR", "ARMMSTOR", "ARMMEX", "ARMMAKR", "ARMLLT", "ARMRAD", "ARMLAB", "ARMVP", "ARMAP", "ARMSY"];
        let second = ["ARMSONAR", "ARMTIDE", "ARMUWES", "ARMUWMS", "ARMUWMEX", "ARMFMKR", "ARMTL"];
        std::fs::write(guis.join("armcom1.gui"), page(&first)).unwrap();
        std::fs::write(guis.join("armcom2.gui"), page(&second)).unwrap();
        std::fs::write(guis.join("armgen.gui"), "[GADGET0]{[COMMON]{id=0;name=HEADER;xpos=0;ypos=128;width=128;height=640;}panel=ARMPAN2;}\n[GADGET1]{[COMMON]{id=1;name=ARMSTOP;xpos=66;ypos=543;width=55;height=31;}}").unwrap();
        vfs.mount_mod_dir(&dir, "tall").unwrap();
        let data = ta_data::GameData::load(&vfs);
        let types = ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p));
        let mut art = Art::default();
        assert_eq!(art.layout_height(&vfs, "ARM", "armcom1"), 768.0, "the mod's pages: a 768-pixel screen");
        let com = art.menu(&vfs, &types, types.id("ARMCOM").unwrap(), "armcom1");
        let names = |page: &MenuPage| -> Vec<Option<String>> { page.slots.iter().map(|s| s.map(|id| types.get(id).name.clone())).collect() };
        assert_eq!(com[0].slots.len(), 12);
        assert_eq!(names(&com[0]), first.iter().map(|n| Some(n.to_string())).collect::<Vec<_>>(), "twelve units, each where its page puts it");
        assert_eq!(names(&com[1])[..7], second.iter().map(|n| Some(n.to_string())).collect::<Vec<_>>()[..]);
        let mut shown: Vec<usize> = com.iter().flat_map(|p| p.slots.iter().flatten().copied()).collect();
        let mut want = types.get(types.id("ARMCOM").unwrap()).build_options.clone();
        shown.sort_unstable();
        want.sort_unstable();
        assert_eq!(shown, want, "everything the Commander builds, once");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Total Mayhem's pages share a place among several buttons: Arm's long-range guns, four
    /// variants round one picture (strips east and west, the north one over the plain one), Core's
    /// laid over each other at a place's size off the grid, a plant's halves; and they lay a
    /// button over several places (a Krogoth's).
    #[test]
    fn buttons_over_a_share_of_a_place_or_over_several() {
        let grid = Grid { top: 0.0, rows: 6 };
        let r = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pos2(x, y), vec2(w, h));
        assert!(grid.is_place(r(64.0, 128.0, 64.0, 64.0)) && grid.share_of(r(64.0, 128.0, 64.0, 64.0)).is_none());
        for share in [r(110.0, 64.0, 18.0, 64.0), r(82.0, 96.0, 28.0, 32.0), r(64.0, 64.0, 18.0, 64.0), r(82.0, 64.0, 28.0, 32.0)] {
            assert!(!grid.is_place(share));
            assert_eq!(grid.share_of(share), Some(3), "Arm's, in the place at (64, 64): {share:?}");
        }
        assert_eq!([grid.share_of(r(82.0, 96.0, 64.0, 64.0)), grid.share_of(r(82.0, 64.0, 64.0, 64.0))], [Some(3), Some(3)], "Core's, in the place their corner is in");
        assert_eq!([grid.share_of(r(0.0, 192.0, 32.0, 64.0)), grid.share_of(r(32.0, 192.0, 32.0, 64.0))], [Some(6), Some(6)], "halves");
        let big = r(0.0, 256.0, 128.0, 128.0);
        assert_eq!((grid.share_of(big), grid.covered(big)), (None, vec![8, 9, 10, 11]), "a Krogoth's: over four places");
        // The first zone holding a point takes it (shares of the place): Core's north variant, laid
        // over the others after them, takes what they leave of it.
        let third = |a: f32| a / 64.0;
        let zones: Vec<(usize, Rect)> = vec![
            (1, Rect::from_min_max(pos2(third(46.0), 0.0), pos2(1.0, 1.0))),
            (2, Rect::from_min_max(pos2(third(18.0), 0.5), pos2(1.0, 1.0))),
            (3, Rect::from_min_max(pos2(0.0, 0.0), pos2(third(18.0), 1.0))),
            (4, Rect::from_min_max(pos2(third(18.0), 0.0), pos2(1.0, 1.0))),
        ];
        let at = |x: f32, y: f32| zone_at(&zones, pos2(x, y));
        assert_eq!([at(0.9, 0.25), at(0.5, 0.75), at(0.1, 0.5), at(0.5, 0.25)], [Some(0), Some(1), Some(2), Some(3)]);
    }

    /// A Total Mayhem build page over the install: the Advanced Construction Kbot's first page with
    /// a Big Bertha in four variants round one place, as the mod lays it out. Each keeps its own
    /// zone, the place is taken (the metal maker keeps its own, below), nothing is on two pages,
    /// and the modern panel shows the four as one picture, the plain Bertha's.
    #[test]
    fn a_place_four_variants_share_keeps_them_round_one_picture() {
        let Some(mut vfs) = install() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let dir = std::env::temp_dir().join(format!("ta_classic_compass_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("guis")).unwrap();
        std::fs::create_dir_all(dir.join("units")).unwrap();
        for (name, angle) in [("ARMBRTHA", 0), ("ARMBRTHAE", 90), ("ARMBRTHAW", 270), ("ARMBRTHAN", 180)] {
            let fbi = format!("[UNITINFO]\n{{\nUnitName={name};\nName=Big Bertha {angle};\nSide=ARM;\nObjectname=ARMGUARD;\nFootprintX=4;\nFootprintZ=4;\nBuildCostMetal=4812;\nBuildCostEnergy=60000;\nBuildTime=90000;\nMaxDamage=1800;\n}}\n");
            std::fs::write(dir.join("units").join(format!("{}.fbi", name.to_ascii_lowercase())), fbi).unwrap();
        }
        let button = |i: usize, name: &str, x: u32, y: u32, w: u32, h: u32| format!("[GADGET{i}]{{[COMMON]{{id=1;name={name};xpos={x};ypos={y};width={w};height={h};attribs=32;commonattribs=4;}}}}\n");
        let page = String::from("[GADGET0]{[COMMON]{id=0;name=HEADER;xpos=0;ypos=128;width=128;height=640;}panel=ARMPAN;}\n")
            + &button(1, "ARMFUS", 0, 0, 64, 64)
            + &button(2, "ARMARAD", 0, 64, 64, 64)
            + &button(3, "ARMBRTHAE", 110, 64, 18, 64)
            + &button(4, "ARMBRTHA", 82, 96, 28, 32)
            + &button(5, "ARMBRTHAW", 64, 64, 18, 64)
            + &button(6, "ARMBRTHAN", 82, 64, 28, 32)
            + &button(7, "ARMMOHO", 0, 128, 64, 64)
            + &button(8, "ARMMMKR", 64, 128, 64, 64);
        std::fs::write(dir.join("guis").join("armack1.gui"), page).unwrap();
        vfs.mount_mod_dir(&dir, "compass").unwrap();
        let data = ta_data::GameData::load(&vfs);
        let types = ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p));
        let id = |name: &str| types.id(name).unwrap();
        let ack = types.get(id("ARMACK"));
        let pages = Art::default().menu(&vfs, &types, ack.id, "armcom1");
        let first = &pages[0];
        let berthas = ["ARMBRTHAE", "ARMBRTHA", "ARMBRTHAW", "ARMBRTHAN"].map(id);
        // (By index among the gadgets, the header not one of them.)
        assert_eq!(first.odd, (2..=5).zip(berthas).collect::<Vec<_>>(), "each in its own zone");
        assert_eq!((first.slots[2], first.slots[3], first.slots[5]), (Some(id("ARMARAD")), Some(berthas[0]), Some(id("ARMMMKR"))), "the place taken; the metal maker's its own");
        let mut shown: Vec<usize> = pages.iter().flat_map(|p| p.units().collect::<Vec<_>>()).collect();
        let mut want = ack.build_options.clone();
        shown.sort_unstable();
        want.sort_unstable();
        assert_eq!(shown, want, "everything it builds, once");
        let places = shared_places(&vfs, &types, ack);
        assert_eq!(places.len(), 1);
        let place = &places[0];
        assert_eq!(place.iter().map(|&(u, _)| u).collect::<Vec<_>>(), berthas, "in the page's order");
        let at = |x: f32, y: f32| zone_at(place, pos2(x, y)).map(|k| types.get(place[k].0).name.clone());
        assert_eq!([at(0.1, 0.5), at(0.9, 0.5), at(0.5, 0.25), at(0.5, 0.75)].map(Option::unwrap), ["ARMBRTHAW", "ARMBRTHAE", "ARMBRTHAN", "ARMBRTHA"], "the side of the picture is the way it faces");
        assert_eq!(plain_of(&types, place, |_| true), Some(id("ARMBRTHA")), "the one the others are named after");
        assert_eq!(plain_of(&types, place, |u| u != id("ARMBRTHA")).map(|u| types.get(u).name.clone()).as_deref(), Some("ARMBRTHAE"), "else the largest zone's");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn interface_art_fonts_and_layouts_load() {
        let Some(vfs) = install() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        let ctx = egui::Context::default();
        let mut art = Art::default();
        for gaf in ["armint", "corint"] {
            for entry in ["PANELSIDE", "PANELTOP", "PANELBOT"] {
                assert!(art.frames(&vfs, &palette, gaf, entry).is_some(), "{gaf} {entry}");
            }
        }
        // Buttons are up, down and greyed; state buttons their states, then mixed, down and
        // greyed.
        for (entry, frames) in [("ARMORDERS", 3), ("ARMATTACK", 3), ("CORSTOP", 3), ("ARMFIREORD", 6), ("CORONOFF", 5), ("ARMCLOAK", 5), ("IGPATCH", 1)] {
            assert_eq!(art.frames(&vfs, &palette, "commongui", entry).map(|f| f.len()), Some(frames), "{entry}");
        }
        for name in ["console", "armbutt", "corbutt", "hatt12"] {
            let font = art.font(&ctx, &vfs, name).unwrap_or_else(|| panic!("{name}"));
            assert!(font.width("METAL 1000") > 20.0, "{name}");
        }
        // Letters made for the screen: at 2.25 screen pixels to one of TA's, at every quarter
        // pixel; at 3, on whole pixels only; each in a cell its size and the padding.
        let console = art.font(&ctx, &vfs, "console").unwrap();
        let made = |scale: f32, look: Look| make_atlas(&ctx, "console", &console.letters, console.height as usize, scale, look, &phases(scale), TextureOptions::LINEAR);
        for look in [Look::Smooth, Look::Pixels] {
            assert!(made(2.25, look).cells['A' as usize].iter().all(Option::is_some), "{look:?}");
            let at_three = made(3.0, look);
            let cells: Vec<_> = at_three.cells['A' as usize].iter().flatten().collect();
            assert_eq!(cells.len(), 1, "{look:?}");
            let w = console.glyph('A').unwrap().1;
            assert_eq!(cells[0].1, vec2(w * 3.0 + 2.0 * CELL_PAD as f32, console.height * 3.0 + 2.0 * CELL_PAD as f32));
        }
        // ...made on another thread the first time a size is asked for (TA's letters meanwhile,
        // one screen pixel to one of TA's, stretched).
        let first = console.atlas(4.5, Look::Smooth, 0);
        assert!(first.scale == 1.0 || first.scale == 4.5, "{}", first.scale);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while console.atlas(4.5, Look::Smooth, 1).scale != 4.5 {
            assert!(std::time::Instant::now() < deadline, "the letters at 4.5 were never made");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // Art made at the size it is shown, and kept while it is.
        let side = art.frames(&vfs, &palette, "armint", "PANELSIDE").unwrap();
        let _ = side[0].texture(&ctx, [288, 1080], Look::Smooth);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let made = || side[0].made.lock().unwrap().iter().find_map(|m| m.texture.get().map(TextureHandle::id));
        while made().is_none() {
            assert!(std::time::Instant::now() < deadline, "the panel at 1080 lines was never made");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(Some(side[0].texture(&ctx, [288, 1080], Look::Smooth)), made());
        assert_eq!(side[0].made.lock().unwrap().len(), 1, "kept, not made again");
        let arm = art.side_layout(&vfs, "ARM").unwrap();
        assert_eq!(arm.get("metalbar"), Some(Rect::from_min_max(pos2(218.0, 12.0), pos2(345.0, 14.0))));
        assert_eq!((arm.font.as_str(), arm.font_gui.as_str()), ("console", "armbutt"));
        let orders = art.gui(&vfs, "armgen").unwrap();
        assert_eq!(orders.panel, "ARMPAN");
        for name in ["ARMORDERS", "ARMMOVE", "ARMFIREORD", "ARMBLAST", "ARMUNLOAD"] {
            assert!(orders.gadgets.iter().any(|g| g.name == name), "{name}");
        }
        // TA's own shortcut keys.
        let key = |name: &str| orders.gadgets.iter().find(|g| g.name == name).and_then(|g| g.quickkey);
        assert_eq!((key("ARMSTOP"), key("ARMFIREORD"), key("ARMBUILD"), key("ARMONOFF")), (Some('S'), Some('F'), Some('B'), Some('X')));
        // A button of the builder's own page, and one a download adds.
        assert_eq!(art.unit_button(&vfs, &palette, "armcom1", "ARMSOLAR").map(|f| f.len()), Some(3));
        assert_eq!(art.unit_button(&vfs, &palette, "armack1", "ARMCKFUS").map(|f| f.len()), Some(3));
    }
}
