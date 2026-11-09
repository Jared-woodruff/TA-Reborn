//! The achievement badges and the rank insignia, drawn by `tools/profiles/badges.py`:
//! `badges/achievements.png` a cell for each achievement of the catalogue in its order and a
//! last one for a secret not yet earned; `badges/ranks.png` a shield for each rank of
//! `ta_config::profiles::RANKS`. A badge not earned is shown greyed: its metal unlit.

use std::sync::OnceLock;

use egui::{Color32, Context, Id, Rect, Response, Sense, TextureHandle, Ui, pos2, vec2};
use ta_config::achievements::{self, Achievement, Category, Tier};

use crate::i18n::n;

/// A cell's pixels in an atlas, and cells a row.
pub const CELL: u32 = 128;
const COLS: u32 = 16;
/// The sizes kept (no mipmaps in every host): a badge is drawn from the smallest at least
/// as large as it is on screen.
const CLASSES: [u32; 3] = [32, 64, 128];

/// How a badge is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Look {
    Earned,
    /// Not earned yet: greyed.
    Locked,
    /// A secret not earned yet: the secret's own badge, not its own.
    Secret,
}

impl Look {
    pub fn of(achievement: &Achievement, unlocked: bool) -> Look {
        if unlocked { Look::Earned } else if achievement.hidden { Look::Secret } else { Look::Locked }
    }
}

/// An atlas of cells, decoded once and kept as textures by size.
struct Atlas {
    name: &'static str,
    bytes: &'static [u8],
    decoded: OnceLock<Option<image::RgbaImage>>,
}

static BADGES: Atlas = Atlas { name: "achievement badges", bytes: include_bytes!("../badges/achievements.png"), decoded: OnceLock::new() };
static INSIGNIA: Atlas = Atlas { name: "rank insignia", bytes: include_bytes!("../badges/ranks.png"), decoded: OnceLock::new() };

impl Atlas {
    fn image(&self) -> Option<&image::RgbaImage> {
        self.decoded.get_or_init(|| match image::load_from_memory_with_format(self.bytes, image::ImageFormat::Png) {
            Ok(image) => Some(image.to_rgba8()),
            Err(error) => {
                log::warn!("{}: {error}", self.name);
                None
            }
        }).as_ref()
    }

    fn texture(&self, ctx: &Context, class: u32, grey: bool) -> Option<TextureHandle> {
        let id = Id::new(("ta_ui::badges", self.name, class, grey));
        if let Some(texture) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
            return Some(texture);
        }
        let atlas = self.image()?;
        let mut image = if class == CELL {
            atlas.clone()
        } else {
            let (w, h) = atlas.dimensions();
            image::imageops::resize(atlas, w * class / CELL, h * class / CELL, image::imageops::FilterType::Lanczos3)
        };
        if grey {
            for p in image.pixels_mut() {
                let [r, g, b, a] = p.0;
                let l = 0.30 * r as f32 + 0.59 * g as f32 + 0.11 * b as f32;
                let v = l * 0.42 + 14.0;
                p.0 = [v as u8, (v + 2.0) as u8, (v + 5.0).min(255.0) as u8, a];
            }
        }
        let color = egui::ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw());
        let texture = ctx.load_texture(format!("{} {class}{}", self.name, if grey { " grey" } else { "" }), color, egui::TextureOptions::LINEAR);
        ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
        Some(texture)
    }

    fn paint(&self, ctx: &Context, painter: &egui::Painter, rect: Rect, cell: u32, grey: bool) {
        let pixels = rect.width().max(rect.height()) * ctx.pixels_per_point();
        let class = CLASSES.into_iter().find(|c| *c as f32 >= pixels * 0.9).unwrap_or(CELL);
        let Some(texture) = self.texture(ctx, class, grey) else { return };
        let [w, h] = texture.size();
        let (cw, ch) = (class as f32 / w as f32, class as f32 / h as f32);
        let (col, row) = ((cell % COLS) as f32, (cell / COLS) as f32);
        let uv = Rect::from_min_size(pos2(col * cw, row * ch), vec2(cw, ch));
        painter.image(texture.id(), rect, uv, Color32::WHITE);
    }
}

/// Decodes the badges and insignia on a worker thread, so that the first one shown (an
/// achievement earned in the middle of a battle) does not hold up a frame.
pub fn preload() {
    if BADGES.decoded.get().is_none() || INSIGNIA.decoded.get().is_none() {
        std::thread::spawn(|| {
            BADGES.image();
            INSIGNIA.image();
        });
    }
}

/// Paints the badge of the achievement at `index` of the catalogue into `rect`.
pub fn paint(ctx: &Context, painter: &egui::Painter, rect: Rect, index: usize, look: Look) {
    let cell = if look == Look::Secret { achievements::ALL.len() } else { index } as u32;
    BADGES.paint(ctx, painter, rect, cell, look == Look::Locked);
}

/// Paints the insignia of the rank of `grade` (its place in `RANKS`) into `rect`.
pub fn paint_insignia(ctx: &Context, painter: &egui::Painter, rect: Rect, grade: usize) {
    INSIGNIA.paint(ctx, painter, rect, grade.min(ta_config::profiles::RANKS.len() - 1) as u32, false);
}

/// The insignia of a rank's `grade`, `size` points across.
pub fn insignia(ui: &mut Ui, grade: usize, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    if ui.is_rect_visible(rect) { paint_insignia(ui.ctx(), ui.painter(), rect, grade); }
    response
}

/// An achievement's badge, `size` points across.
pub fn badge(ui: &mut Ui, achievement: &Achievement, size: f32, look: Look) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    if ui.is_rect_visible(rect) { paint(ui.ctx(), ui.painter(), rect, achievement.index(), look); }
    response
}

/// The colour of a tier's metal, for its name and its bars.
pub fn tier_color(tier: Tier) -> Color32 {
    match tier {
        Tier::Bronze => Color32::from_rgb(206, 132, 78),
        Tier::Silver => Color32::from_rgb(200, 206, 214),
        Tier::Gold => Color32::from_rgb(236, 192, 82),
        Tier::Platinum => Color32::from_rgb(178, 228, 244),
    }
}

pub fn tier_label(tier: Tier) -> &'static str {
    match tier { Tier::Bronze => n("BRONZE"), Tier::Silver => n("SILVER"), Tier::Gold => n("GOLD"), Tier::Platinum => n("PLATINUM") }
}

pub fn category_label(category: Category) -> &'static str {
    match category {
        Category::Campaign => n("CAMPAIGN"), Category::Skirmish => n("SKIRMISH"), Category::Multiplayer => n("MULTIPLAYER"),
        Category::Battle => n("BATTLE"), Category::Industry => n("INDUSTRY"), Category::Service => n("SERVICE"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_atlas_holds_a_badge_for_every_achievement_and_the_secret() {
        let atlas = BADGES.image().expect("the badges decode");
        assert_eq!(atlas.width(), COLS * CELL);
        let cells = (atlas.width() / CELL) * (atlas.height() / CELL);
        assert!(cells as usize > achievements::ALL.len(), "run tools/profiles/badges.py after changing the list");
        // Every cell in use is drawn (its middle opaque), the first one past them empty.
        let opaque = |cell: u32| atlas.get_pixel((cell % COLS) * CELL + CELL / 2, (cell / COLS) * CELL + CELL / 2).0[3] > 200;
        for cell in 0..=achievements::ALL.len() as u32 { assert!(opaque(cell), "cell {cell} is empty"); }
        if achievements::ALL.len() as u32 + 1 < cells { assert!(!opaque(achievements::ALL.len() as u32 + 1), "more badges than achievements"); }
    }

    #[test]
    fn there_is_an_insignia_for_every_rank() {
        let atlas = INSIGNIA.image().expect("the insignia decode");
        let ranks = ta_config::profiles::RANKS.len() as u32;
        let opaque = |cell: u32| atlas.get_pixel((cell % COLS) * CELL + CELL / 2, (cell / COLS) * CELL + CELL / 2).0[3] > 200;
        for cell in 0..ranks { assert!(opaque(cell), "rank {cell} has no insignia: run tools/profiles/badges.py --ranks"); }
        assert!((atlas.width() / CELL) * (atlas.height() / CELL) <= ranks + COLS, "more insignia than ranks");
    }
}
