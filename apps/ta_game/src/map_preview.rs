//! Maps' pictures in the menus' pickers (skirmish, scenarios, the lobby, Conquest, the
//! briefing): drawn from each map's own art at the size they are shown
//! (`ta_render::map_preview`, on a worker: TA's own minimap stands in for the moment it
//! takes), kept while the game runs, framed in a steel bezel that hugs the picture, with the
//! pickers' markers kept on it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui::emath::GuiRounding;
use egui::{Color32, Pos2, Rect, Stroke, pos2, vec2};
use ta_formats::tnt::TntMap;

use crate::setup::Assets;

/// A map's picture and the ground it shows (world pixels from the map's top left).
#[derive(Clone)]
pub struct Picture {
    pub texture: egui::TextureHandle,
    pub area: (f32, f32),
}

impl Picture {
    /// The picture's shape, width over height.
    pub fn aspect(&self) -> f32 {
        self.area.0 / self.area.1.max(1.0)
    }
}

/// A map's picture as it stands: TA's minimap until the drawing comes (none for a map that
/// could not be read).
#[derive(Default)]
struct Entry {
    picture: Option<Picture>,
    drawing: Option<std::sync::mpsc::Receiver<ta_render::map_preview::MapPicture>>,
}

/// The pictures made so far, by map (lower case) and size in pixels.
type Cache = Arc<Mutex<HashMap<(String, u32), Entry>>>;

fn cache(ctx: &egui::Context) -> Cache {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(egui::Id::new("map_pictures")).clone())
}

/// The picture of map `name` for showing `long_side` points along its longer side.
pub fn picture(ctx: &egui::Context, assets: &Assets, name: &str, long_side: f32) -> Option<Picture> {
    // (In steps of 64 pixels: a page shown a little smaller or larger keeps its picture.)
    let pixels = ((long_side * ctx.pixels_per_point() / 64.0).ceil() as u32 * 64).clamp(64, 2048);
    let key = (name.to_ascii_lowercase(), pixels);
    let cache = cache(ctx);
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    let entry = cache.entry(key).or_insert_with(|| {
        let Some(map) = assets.vfs.read_opt(&format!("maps/{name}.tnt")).and_then(|b| TntMap::parse(&b).ok()) else { return Entry::default() };
        // TA's minimap for now (the same ground), the picture drawn from the art meanwhile.
        let (aw, ah) = map.minimap_area_px();
        let mm = &map.minimap;
        let image = egui::ColorImage::from_rgba_unmultiplied([mm.width.max(1) as usize, mm.height.max(1) as usize], &mm.to_rgba(&assets.data.palette));
        let quick = Picture { texture: ctx.load_texture(format!("map-minimap-{name}"), image, egui::TextureOptions::LINEAR), area: (aw as f32, ah as f32) };
        let (send, drawing) = std::sync::mpsc::channel();
        let palette = assets.data.palette.clone();
        let repaint = ctx.clone();
        rayon::spawn(move || {
            if send.send(ta_render::map_preview::render(&map, &palette, pixels)).is_ok() {
                repaint.request_repaint();
            }
        });
        Entry { picture: Some(quick), drawing: Some(drawing) }
    });
    if let Some(p) = entry.drawing.as_ref().and_then(|d| d.try_recv().ok()) {
        let image = egui::ColorImage::from_rgba_unmultiplied([p.width, p.height], &p.rgba);
        entry.picture = Some(Picture { texture: ctx.load_texture(format!("map-picture-{name}-{pixels}"), image, egui::TextureOptions::LINEAR), area: p.area });
        entry.drawing = None;
    }
    entry.picture.clone()
}

/// Forgets map `name`'s pictures (the map editor saved a new version of it).
pub fn forget(ctx: &egui::Context, name: &str) {
    let cache = cache(ctx);
    cache.lock().unwrap_or_else(|e| e.into_inner()).retain(|(k, _), _| !k.eq_ignore_ascii_case(name));
}

/// Draws `picture` as large as fits in `frame`, centred, in a riveted steel bezel that hugs
/// it; returns where the picture is, and how to put a point of the ground on it.
pub fn show(painter: &egui::Painter, picture: &Picture, frame: Rect) -> Placed {
    let room = frame.shrink(5.0);
    let aspect = picture.aspect();
    let size = if room.width() / room.height() > aspect { vec2(room.height() * aspect, room.height()) } else { vec2(room.width(), room.width() / aspect) };
    let rect = Rect::from_center_size(room.center(), size).round_to_pixels(painter.pixels_per_point());
    painter.image(picture.texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    painter.add(ta_ui::metal::bezel(rect, 4.0, true));
    Placed { rect, area: picture.area }
}

/// A picture as drawn: points of the ground on it.
pub struct Placed {
    pub rect: Rect,
    area: (f32, f32),
}

impl Placed {
    /// Where the ground at world (`x`, `z`) is on the picture.
    pub fn at(&self, x: f32, z: f32) -> Pos2 {
        self.rect.min + vec2(x / self.area.0 * self.rect.width(), z / self.area.1 * self.rect.height())
    }

    /// [`Placed::at`], kept a marker of `radius` inside the picture.
    pub fn marker_at(&self, x: f32, z: f32, radius: f32) -> Pos2 {
        let inner = self.rect.shrink(radius + 2.0);
        self.at(x, z).clamp(inner.min, inner.max)
    }

    /// How many points across a world pixel is.
    pub fn scale(&self) -> f32 {
        self.rect.width() / self.area.0.max(1.0)
    }
}

/// A start position's marker: a disc of the player's colour (grey for nobody), outlined dark
/// on a shadow, numbered.
pub fn start_marker(painter: &egui::Painter, p: Pos2, fill: Color32, label: &str) {
    painter.circle(p + vec2(0.0, 1.0), 10.0, Color32::from_black_alpha(90), Stroke::NONE);
    painter.circle(p, 8.5, fill, Stroke::new(1.5, Color32::from_black_alpha(220)));
    // (Dark numerals on light colours: white is lost on them.)
    let light = fill.r() as f32 * 0.299 + fill.g() as f32 * 0.587 + fill.b() as f32 * 0.114 > 150.0;
    painter.text(p, egui::Align2::CENTER_CENTER, label, crate::theme::strong(11.5), if light { Color32::from_rgb(20, 20, 20) } else { Color32::WHITE });
}

#[cfg(test)]
mod tests {
    /// Every map of the install drawn at a 4K screen's size, the slowest timed.
    #[test]
    #[ignore]
    fn every_map_draws_quickly() {
        let Ok(install) = ta_vfs::install::discover(None) else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let palette = ta_data::GameData::load(&vfs).palette;
        let mut slowest = (std::time::Duration::ZERO, String::new());
        for path in vfs.list("maps", Some("tnt"), false) {
            let Some(map) = vfs.read_opt(&path).and_then(|b| ta_formats::tnt::TntMap::parse(&b).ok()) else { continue };
            let start = std::time::Instant::now();
            let p = ta_render::map_preview::render(&map, &palette, 1024);
            let took = start.elapsed();
            assert!(p.width > 0 && p.height > 0);
            if took > slowest.0 {
                slowest = (took, format!("{path} ({} x {} cells)", map.width, map.height));
            }
        }
        println!("slowest: {:?} for {}", slowest.0, slowest.1);
    }
}
