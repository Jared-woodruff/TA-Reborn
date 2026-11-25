//! The menus in Supreme Commander: Forged Alliance's look (Options, INTERFACE: MENU THEME): the
//! theme the launcher makes of the player's own copy (`ta_supcom::menus`, the mod `supcom-menus`;
//! docs/SUPCOM_MENUS.md), whose list ([`THEME`]) names
//! - its colours, which the menus are drawn in ([`Grade`]): every colour of TA Reborn's own that
//!   their text and shapes are drawn in moved onto the theme's (its palette made as a mod's
//!   interface theme's is, `Palette::themed`), the colours between them moved as those nearest
//!   them; pictures, previews and portraits are drawn as they are, and the players' colours, black
//!   and white stay;
//! - its pictures, shown behind the menus one after another in place of the film ([`Pictures`]);
//! - its panels' border art (`ta_ui::panel::set_art`), round the menus' panels in place of the
//!   pipes;
//! - its interface sounds (by TA's interface events) and its menu music (the menus' playlist).
//!
//! The layouts and the widgets' shapes stay TA Reborn's. The battle (its HUD, its dialogs) keeps
//! its own look.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::mpsc;

use egui::{Color32, ColorImage, Rect, TextureHandle, TextureOptions, pos2};
use ta_ui::theme::{Palette, parse_colour};

/// The theme's list in the game's files (the mod `supcom-menus`'s).
pub const THEME: &str = "menus/supcom/theme.toml";

/// The panel's border pieces, in the order `ta_ui::panel::PanelArt` takes them.
pub const PANEL_PLACES: [&str; 9] = ta_ui::panel::PanelArt::PLACES;

/// The theme's list as written.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct ThemeFile {
    pictures: Vec<String>,
    music: Option<String>,
    colours: BTreeMap<String, String>,
    panel: BTreeMap<String, String>,
    sounds: BTreeMap<String, String>,
}

/// A menu theme, as its list has it.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuTheme {
    /// TA Reborn's palette in the theme's colours.
    pub palette: Palette,
    /// Its pictures' files.
    pub pictures: Vec<String>,
    /// Its panel border's nine pieces' files ([`PANEL_PLACES`]), where it has all nine.
    pub panel: Option<[String; 9]>,
    /// The file played for each of TA's interface events (`ta_ui::sound::UiSound::event`).
    pub sounds: BTreeMap<String, String>,
    /// Its menu music's file.
    pub music: Option<String>,
}

impl MenuTheme {
    /// The theme of a list (`None` where it does not read).
    pub fn parse(text: &str) -> Option<MenuTheme> {
        let file: ThemeFile = toml::from_str(text).map_err(|e| log::warn!("{THEME}: {e}")).ok()?;
        let palette = Palette::REBORN.themed(&|key| file.colours.get(key).and_then(|c| parse_colour(c)));
        let panel = PANEL_PLACES.iter().map(|place| file.panel.get(*place).map(|p| ta_vfs::normalize(p))).collect::<Option<Vec<String>>>().and_then(|v| v.try_into().ok());
        Some(MenuTheme {
            palette,
            pictures: file.pictures.iter().map(|p| ta_vfs::normalize(p)).collect(),
            panel,
            sounds: file.sounds.into_iter().map(|(event, path)| (event, ta_vfs::normalize(&path))).collect(),
            music: file.music.map(|m| ta_vfs::normalize(&m)),
        })
    }

    /// The theme the game's files have (its mod installed and on), if any.
    pub fn read(vfs: &ta_vfs::Vfs) -> Option<MenuTheme> {
        let bytes = vfs.read(THEME).ok()?;
        MenuTheme::parse(&String::from_utf8_lossy(&bytes))
    }
}

/// Whether the game's files have a menu theme (MENU THEME offers it).
pub fn installed(vfs: &ta_vfs::Vfs) -> bool {
    vfs.exists(THEME)
}

/// A menu theme in use: its list, its colours' grade, its pictures and its panels' art.
pub struct Look {
    pub theme: MenuTheme,
    pub grade: Grade,
    pub pictures: Pictures,
    pub art: Option<ta_ui::panel::PanelArt>,
}

impl Look {
    /// `theme` made ready for `ctx` from the game's files `vfs` (its panels' art decoded; its
    /// pictures as they are shown), the first picture chosen by `seed`.
    pub fn new(ctx: &egui::Context, vfs: &Arc<ta_vfs::Vfs>, theme: MenuTheme, seed: u64) -> Look {
        let art = theme.panel.as_ref().and_then(|files| panel_art(ctx, vfs, files));
        Look { grade: Grade::new(&theme.palette), pictures: Pictures::new(theme.pictures.clone(), vfs.clone(), seed), art, theme }
    }

    /// Whether its pictures stand behind the menus (in place of the film).
    pub fn covers(&self) -> bool {
        !self.pictures.files.is_empty()
    }
}

// ---- The colours -------------------------------------------------------------------------------

/// How far (in sRGB, 0..255 a channel) a colour may be from the nearest anchor and still move:
/// colours of other kinds stay as they are.
const REACH: f32 = 110.0;
/// How near a players' colour a colour is kept as it is (fully within the first; moved as the
/// others from the second): the players' colours drawn as they are, and their see-through kinds,
/// never pulling the interface's colours near them.
const PIN_NEAR: f32 = 1.5;
const PIN_FAR: f32 = 4.0;

/// The menus' colours moved from TA Reborn's onto a theme's. Each anchor is a colour TA Reborn
/// draws in and where it goes: its palette's colours onto the theme's ([`Palette`]'s fields, as
/// `Palette::themed` makes them from the theme's few), its metal's (the gunmetal, the slots, the
/// pipes' steel, the brass) onto the theme's at their own lightness, and black and white onto
/// themselves. A colour moves as the anchors nearest it do, weighted by how near (the inverse
/// square of the distance), less the further it is from the nearest ([`REACH`]). The players'
/// colours are kept as they are, exactly ([`PIN_NEAR`]). Made for a frame's meshes
/// ([`Grade::apply`]), each colour worked out once.
pub struct Grade {
    anchors: Vec<([f32; 3], [f32; 3])>,
    pins: Vec<[f32; 3]>,
    cache: HashMap<Color32, Color32>,
}

fn rgb(c: Color32) -> [f32; 3] {
    [c.r() as f32, c.g() as f32, c.b() as f32]
}

/// A colour's lightness (its luma, sRGB weights).
fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// `like`'s hue and colourfulness at `c`'s lightness.
fn rehued(c: Color32, like: Color32) -> [f32; 3] {
    let (to, from) = (luma(rgb(c)), luma(rgb(like)).max(1.0));
    rgb(like).map(|v| (v * to / from).clamp(0.0, 255.0))
}

impl Grade {
    /// TA Reborn's colours onto the palette `to`.
    pub fn new(to: &Palette) -> Grade {
        let from = Palette::REBORN;
        let mut anchors: Vec<([f32; 3], [f32; 3])> = Vec::new();
        let mut anchor = |a: Color32, b: [f32; 3]| {
            // (The first anchor of a colour holds: TA Reborn draws two roles in one colour here
            // and there.)
            if !anchors.iter().any(|(f, _)| *f == rgb(a)) {
                anchors.push((rgb(a), b));
            }
        };
        let pairs = [
            (from.bg_deep, to.bg_deep), (from.bg, to.bg), (from.panel, to.panel), (from.panel_hi, to.panel_hi), (from.inset, to.inset),
            (from.line, to.line), (from.line_hi, to.line_hi), (from.steel, to.steel), (from.steel_dim, to.steel_dim), (from.text, to.text),
            (from.text_bright, to.text_bright), (from.accent, to.accent), (from.accent_bright, to.accent_bright), (from.accent_dim, to.accent_dim),
            (from.accent_deep, to.accent_deep), (from.on_accent, to.on_accent), (from.good, to.good), (from.good_deep, to.good_deep), (from.bad, to.bad),
            (from.bad_deep, to.bad_deep), (from.info, to.info), (from.button_text, to.button_text), (from.faint_bg, to.faint_bg), (from.widget, to.widget),
            (from.widget_hover, to.widget_hover), (from.selection, to.selection),
        ];
        for (a, b) in pairs {
            anchor(a, rgb(b));
        }
        // The metal: its steel and gunmetal at the theme's lines' hue, its brass at its accent's.
        let steel = to.line_hi;
        for c in [ta_ui::metal::STEEL_SHINE, ta_ui::metal::STEEL_BODY, ta_ui::metal::STEEL_SHADE, ta_ui::metal::STEEL_EDGE] {
            anchor(c, rehued(c, steel));
        }
        for (r, g, b) in [(102, 103, 98), (58, 59, 55), (106, 107, 102), (56, 57, 53), (33, 32, 27), (22, 21, 18), (150, 148, 140), (78, 76, 70), (92, 90, 84), (98, 96, 88), (38, 35, 29)] {
            let c = Color32::from_rgb(r, g, b);
            anchor(c, rehued(c, steel));
        }
        for (r, g, b) in [(230, 196, 110), (150, 114, 45), (232, 178, 84), (255, 246, 206), (255, 250, 214)] {
            let c = Color32::from_rgb(r, g, b);
            anchor(c, rehued(c, to.accent_bright));
        }
        // Black and white stay, and what is near them goes less.
        anchor(Color32::BLACK, [0.0; 3]);
        anchor(Color32::WHITE, [255.0; 3]);
        // The players' colours (both sets) stay exactly.
        let pins = crate::palette::TEAM_COLORS.into_iter().chain(crate::palette::TEAM_COLORS_CB).map(|c| c.map(f32::from)).collect();
        Grade { anchors, pins, cache: HashMap::new() }
    }

    /// Where the colour `c` (opaque) goes.
    fn moved(&self, c: [f32; 3]) -> [f32; 3] {
        let distance = |a: &[f32; 3]| (0..3).map(|i| (c[i] - a[i]).powi(2)).sum::<f32>().sqrt();
        let pinned = self.pins.iter().map(distance).fold(f32::MAX, f32::min);
        let keep = ((PIN_FAR - pinned) / (PIN_FAR - PIN_NEAR)).clamp(0.0, 1.0);
        let keep = keep * keep * (3.0 - 2.0 * keep);
        if keep >= 1.0 {
            return c;
        }
        let graded = self.graded(c);
        [0, 1, 2].map(|i| graded[i] + (c[i] - graded[i]) * keep)
    }

    /// Where the colour `c` goes by the anchors alone.
    fn graded(&self, c: [f32; 3]) -> [f32; 3] {
        let mut sum = [0.0f32; 3];
        let mut weights = 0.0f32;
        let mut nearest = f32::MAX;
        for (from, to) in &self.anchors {
            let d2 = (0..3).map(|i| (c[i] - from[i]).powi(2)).sum::<f32>();
            if d2 < 0.25 {
                return *to;
            }
            nearest = nearest.min(d2);
            let w = 1.0 / d2;
            weights += w;
            for i in 0..3 {
                sum[i] += w * (to[i] - from[i]);
            }
        }
        let reach = (1.0 - (nearest.sqrt() / REACH)).clamp(0.0, 1.0);
        let reach = reach * reach * (3.0 - 2.0 * reach);
        [0, 1, 2].map(|i| (c[i] + reach * sum[i] / weights).clamp(0.0, 255.0))
    }

    /// Where the colour `c` goes (premultiplied, as egui keeps colours: its alpha kept).
    pub fn colour(&mut self, c: Color32) -> Color32 {
        if let Some(to) = self.cache.get(&c) {
            return *to;
        }
        let a = c.a();
        let to = if a == 0 {
            // (Added light, no coverage: its colour moved as it is.)
            let [r, g, b] = self.moved(rgb(c)).map(|v| v.round() as u8);
            Color32::from_rgba_premultiplied(r, g, b, 0)
        } else {
            let unmultiplied = rgb(c).map(|v| (v * 255.0 / a as f32).min(255.0));
            let [r, g, b] = self.moved(unmultiplied).map(|v| (v * a as f32 / 255.0).round().min(a as f32) as u8);
            Color32::from_rgba_premultiplied(r, g, b, a)
        };
        // (A frame's colours are few, those of the next the same: kept while they are few.)
        if self.cache.len() > 4096 {
            self.cache.clear();
        }
        self.cache.insert(c, to);
        to
    }

    /// A frame's meshes in the theme's colours: those drawn in egui's own texture (shapes and
    /// text); not pictures, previews, portraits or the theme's own art (textures of their own).
    pub fn apply(&mut self, prims: &mut [egui::ClippedPrimitive]) {
        for prim in prims {
            if let egui::epaint::Primitive::Mesh(mesh) = &mut prim.primitive
                && mesh.texture_id == egui::TextureId::default()
            {
                for v in &mut mesh.vertices {
                    v.color = self.colour(v.color);
                }
            }
        }
    }
}

// ---- The pictures ------------------------------------------------------------------------------

/// How long a picture is shown before the next comes in, and how long the next takes to come in
/// (seconds).
const PICTURE_SECONDS: f64 = 24.0;
const FADE_SECONDS: f64 = 2.5;
/// How much a picture grows while it is shown (with reduced motion: none).
const DRIFT: f32 = 0.06;

/// The theme's pictures behind the menus: one shown, growing slowly, the next fading in over it,
/// each decoded on a worker before its turn (two held as textures at most).
pub struct Pictures {
    files: Vec<String>,
    vfs: Arc<ta_vfs::Vfs>,
    /// The picture shown, and when it came in.
    shown: usize,
    since: f64,
    textures: HashMap<usize, TextureHandle>,
    decoding: Option<(usize, mpsc::Receiver<Option<ColorImage>>)>,
    /// The pictures that would not read: passed over.
    broken: std::collections::BTreeSet<usize>,
}

impl Pictures {
    /// The pictures `files` of the game's files `vfs`, the first chosen by `seed`.
    pub fn new(files: Vec<String>, vfs: Arc<ta_vfs::Vfs>, seed: u64) -> Pictures {
        let shown = if files.is_empty() { 0 } else { (seed % files.len() as u64) as usize };
        Pictures { files, vfs, shown, since: f64::NAN, textures: HashMap::new(), decoding: None, broken: Default::default() }
    }

    /// Decodes picture `index` on a worker, unless one is decoding or it is held.
    fn want(&mut self, index: usize) {
        if self.decoding.is_some() || self.textures.contains_key(&index) || self.broken.contains(&index) {
            return;
        }
        let Some(path) = self.files.get(index).cloned() else { return };
        let vfs = self.vfs.clone();
        let (send, receive) = mpsc::channel();
        let started = std::thread::Builder::new().name("menu picture".into()).spawn(move || {
            let image = vfs.read(&path).ok().and_then(|bytes| ta_formats::dds::decode(&bytes, 0).map_err(|e| log::warn!("{path}: {e}")).ok()).map(|image| {
                let pixels: Vec<u8> = image.rgba.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect();
                ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &pixels)
            });
            let _ = send.send(image);
        });
        if started.is_ok() {
            self.decoding = Some((index, receive));
        }
    }

    /// Takes in a picture decoded.
    fn poll(&mut self, ctx: &egui::Context) {
        let Some((index, receive)) = &self.decoding else { return };
        match receive.try_recv() {
            Ok(image) => {
                let index = *index;
                self.decoding = None;
                match image {
                    Some(image) => {
                        self.textures.insert(index, ctx.load_texture(format!("menu picture {index}"), image, TextureOptions::LINEAR));
                    }
                    None => {
                        self.broken.insert(index);
                    }
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                let index = *index;
                self.decoding = None;
                self.broken.insert(index);
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
        }
    }

    /// Paints the pictures over `screen` with `painter` (the background's), at time `now`:
    /// whether one was painted (else what is behind shows).
    pub fn paint(&mut self, ctx: &egui::Context, painter: &egui::Painter, screen: Rect, now: f64, reduced_motion: bool) -> bool {
        if self.files.is_empty() || self.broken.len() >= self.files.len() {
            return false;
        }
        self.poll(ctx);
        // (A picture that would not read passed over for the next.)
        while self.broken.contains(&self.shown) {
            self.shown = (self.shown + 1) % self.files.len();
            self.since = f64::NAN;
        }
        self.want(self.shown);
        let Some(current) = self.textures.get(&self.shown).cloned() else { return false };
        if self.since.is_nan() {
            self.since = now;
        }
        let age = now - self.since;
        let mut next = (self.shown + 1) % self.files.len();
        while self.broken.contains(&next) && next != self.shown {
            next = (next + 1) % self.files.len();
        }
        // (The next decoded ahead of its turn.)
        if age > PICTURE_SECONDS - FADE_SECONDS - 6.0 {
            self.want(next);
        }
        draw(painter, screen, &current, age, reduced_motion, 1.0);
        if age > PICTURE_SECONDS - FADE_SECONDS
            && let Some(incoming) = self.textures.get(&next).cloned()
        {
            let t = ((age - (PICTURE_SECONDS - FADE_SECONDS)) / FADE_SECONDS).clamp(0.0, 1.0) as f32;
            draw(painter, screen, &incoming, 0.0, reduced_motion, t);
            if t >= 1.0 {
                self.textures.retain(|i, _| *i == next);
                self.shown = next;
                self.since = now;
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(if reduced_motion { 250 } else { 33 }));
        true
    }

    /// Lets the pictures held go (the menus left).
    pub fn let_go(&mut self) {
        self.textures.clear();
        self.decoding = None;
        self.since = f64::NAN;
    }
}

/// A picture over `screen`, covering it (cut at its sides or top and bottom), grown by its `age`
/// (seconds shown), at `opacity`; darkened a little, more at the left where the menu stands.
fn draw(painter: &egui::Painter, screen: Rect, texture: &TextureHandle, age: f64, reduced_motion: bool, opacity: f32) {
    let size = texture.size_vec2();
    let grow = if reduced_motion { 1.0 } else { 1.0 + DRIFT * (age / PICTURE_SECONDS).clamp(0.0, 1.0) as f32 };
    let scale = (screen.width() / size.x).max(screen.height() / size.y) * grow;
    let shown = size * scale;
    let rect = Rect::from_center_size(screen.center(), shown);
    let tint = Color32::from_white_alpha((opacity * 255.0) as u8);
    painter.image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
    // (The menu stands at the left: the picture darker there, for its words.)
    let shade = |a: f32| Color32::from_black_alpha((a * opacity * 255.0) as u8);
    let mut mesh = egui::Mesh::default();
    let (l, r) = (screen.left(), screen.right());
    let mid = l + screen.width() * 0.55;
    for (x0, x1, a0, a1) in [(l, mid, 0.55, 0.25), (mid, r, 0.25, 0.15)] {
        let at = mesh.vertices.len() as u32;
        mesh.colored_vertex(pos2(x0, screen.top()), shade(a0));
        mesh.colored_vertex(pos2(x1, screen.top()), shade(a1));
        mesh.colored_vertex(pos2(x1, screen.bottom()), shade(a1));
        mesh.colored_vertex(pos2(x0, screen.bottom()), shade(a0));
        mesh.add_triangle(at, at + 1, at + 2);
        mesh.add_triangle(at, at + 2, at + 3);
    }
    painter.add(mesh);
}

/// The panel border's pieces of the theme as textures, decoded from the game's files.
pub fn panel_art(ctx: &egui::Context, vfs: &ta_vfs::Vfs, files: &[String; 9]) -> Option<ta_ui::panel::PanelArt> {
    let mut pieces = Vec::with_capacity(9);
    for (place, path) in PANEL_PLACES.iter().zip(files) {
        let bytes = vfs.read(path).map_err(|e| log::warn!("{path}: {e}")).ok()?;
        let image = ta_formats::dds::decode(&bytes, 0).map_err(|e| log::warn!("{path}: {e}")).ok()?;
        let pixels: Vec<u8> = image.rgba.iter().flatten().copied().collect();
        let colour = ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &pixels);
        pieces.push(ctx.load_texture(format!("menu panel {place}"), colour, TextureOptions::LINEAR));
    }
    Some(ta_ui::panel::PanelArt { pieces: pieces.try_into().ok()? })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A theme's list as the launcher writes it (`ta_supcom::menus::theme`).
    const LIST: &str = r##"# Supreme Commander: Forged Alliance's look for TA Reborn's menus
pictures = ["menus/supcom/pictures/space_1.dds", "menus/supcom/pictures/factions.dds"]
music = "music/supcom-menus/menu.flac"

[colours]
text = "#badbdb"
text_bright = "#ffffff"
button_text = "#badbdb"
dim = "#6fafaf"
accent = "#9cc1bf"
panel = "#313e47"
line = "#47555a"
good = "#69ab4d"
info = "#7fa8a4"

[panel]
top_left = "menus/supcom/panel/top_left.dds"
top = "menus/supcom/panel/top.dds"
top_right = "menus/supcom/panel/top_right.dds"
left = "menus/supcom/panel/left.dds"
middle = "menus/supcom/panel/middle.dds"
right = "menus/supcom/panel/right.dds"
bottom_left = "menus/supcom/panel/bottom_left.dds"
bottom = "menus/supcom/panel/bottom.dds"
bottom_right = "menus/supcom/panel/bottom_right.dds"

[sounds]
BIGBUTTON = "sounds/supcom-menus/ui_menu_mousedown.wav"
PREVIOUS = "sounds/supcom-menus/ui_back_mousedown.wav"
"##;

    /// The list read: its pictures, music, sounds, the panel's nine pieces in their order, and the
    /// palette made of its colours; without all nine pieces, no panel art.
    #[test]
    fn a_themes_list_is_read() {
        let theme = MenuTheme::parse(LIST).unwrap();
        assert_eq!(theme.pictures, ["menus/supcom/pictures/space_1.dds", "menus/supcom/pictures/factions.dds"]);
        assert_eq!(theme.music.as_deref(), Some("music/supcom-menus/menu.flac"));
        assert_eq!(theme.sounds.get("PREVIOUS").map(String::as_str), Some("sounds/supcom-menus/ui_back_mousedown.wav"));
        assert_eq!(theme.panel.as_ref().map(|p| p[8].as_str()), Some("menus/supcom/panel/bottom_right.dds"));
        assert_eq!(theme.palette.text, Color32::from_rgb(0xba, 0xdb, 0xdb));
        assert_eq!(theme.palette.panel, Color32::from_rgb(0x31, 0x3e, 0x47));
        assert_ne!(theme.palette.bg, Palette::REBORN.bg, "the shades made from the theme's");
        let without = LIST.replace("middle = \"menus/supcom/panel/middle.dds\"\n", "");
        assert!(MenuTheme::parse(&without).unwrap().panel.is_none());
        assert!(MenuTheme::parse("pictures = 3").is_none());
    }

    /// The pictures behind the menus: one that does not read is passed over (not decoded again
    /// frame after frame), the next shown once decoded; let go of, none held.
    #[test]
    fn a_picture_that_does_not_read_is_passed_over() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("ta-menu-pictures-{}-{nanos}", std::process::id()));
        let pictures = dir.join("menus/supcom/pictures");
        std::fs::create_dir_all(&pictures).unwrap();
        std::fs::write(pictures.join("broken.dds"), b"not a picture").unwrap();
        let mut good = ta_formats::dds::header(8, 6, 1, None);
        for _ in 0..8 * 6 {
            good.extend_from_slice(&[30, 60, 90, 255]);
        }
        std::fs::write(pictures.join("good.dds"), good).unwrap();
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_mod_dir(&dir, "theme").unwrap();
        let files = vec!["menus/supcom/pictures/broken.dds".to_string(), "menus/supcom/pictures/good.dds".to_string()];
        let mut shown = Pictures::new(files, Arc::new(vfs), 0);
        let ctx = egui::Context::default();
        let mut painted = false;
        for frame in 0..400 {
            ctx.run_ui(Default::default(), |ui| {
                let painter = ui.ctx().layer_painter(egui::LayerId::background());
                painted = shown.paint(ui.ctx(), &painter, ui.ctx().content_rect(), frame as f64 / 30.0, true);
            }).drop_without_applying_deltas();
            if painted {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(painted, "the good picture shown");
        assert!(shown.broken.contains(&0) && shown.shown == 1, "the broken one passed over");
        assert!(shown.decoding.is_none(), "nothing decoding again");
        shown.let_go();
        assert!(shown.textures.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// TA Reborn's colours go onto the theme's exactly; the players' colours, black and white
    /// stay; a colour between two of TA Reborn's goes between theirs; a colour far from every
    /// anchor stays; alpha is kept.
    #[test]
    fn the_grade_moves_reborns_colours_onto_the_themes() {
        let theme = MenuTheme::parse(LIST).unwrap();
        let mut grade = Grade::new(&theme.palette);
        let reborn = Palette::REBORN;
        assert_eq!(grade.colour(reborn.accent), theme.palette.accent);
        assert_eq!(grade.colour(reborn.text), theme.palette.text);
        assert_eq!(grade.colour(reborn.panel), theme.palette.panel);
        for [r, g, b] in crate::palette::TEAM_COLORS {
            assert_eq!(grade.colour(Color32::from_rgb(r, g, b)), Color32::from_rgb(r, g, b), "a player's colour stays");
        }
        // (And see-through, as a player's colour is drawn faded; yet the hover colour beside the
        // players' dark grey moves as the rest of the interface.)
        let faded = Color32::from_rgb(210, 40, 30).gamma_multiply(0.5);
        assert_eq!(grade.colour(faded), faded);
        assert_eq!(grade.colour(reborn.panel_hi), theme.palette.panel_hi);
        assert_eq!(grade.colour(Color32::BLACK), Color32::BLACK);
        assert_eq!(grade.colour(Color32::WHITE), Color32::WHITE);
        // (Half way from TA Reborn's text to its accent: near half way between the theme's.)
        let between = grade.colour(reborn.text.lerp_to_gamma(reborn.accent, 0.5));
        let mid = theme.palette.text.lerp_to_gamma(theme.palette.accent, 0.5);
        let off = (0..3).map(|i| (between.to_array()[i] as i32 - mid.to_array()[i] as i32).abs()).max().unwrap();
        assert!(off < 24, "{between:?} near {mid:?}");
        // (A saturated magenta, far from every anchor, stays.)
        let magenta = Color32::from_rgb(255, 0, 255);
        assert_eq!(grade.colour(magenta), magenta);
        // (A colour half see-through: moved as it is unmultiplied, its alpha kept.)
        let half = reborn.accent.gamma_multiply(0.5);
        let faded = grade.colour(half);
        assert_eq!(faded.a(), half.a());
        let expect = theme.palette.accent.gamma_multiply(0.5);
        let off = (0..3).map(|i| (faded.to_array()[i] as i32 - expect.to_array()[i] as i32).abs()).max().unwrap();
        assert!(off <= 2, "{faded:?} near {expect:?}");
    }
}
