//! The map in miniature, in a corner of the view: its art as painted, the start positions and
//! the ground the camera shows; a click or a drag on it takes the camera there.

use egui::{Color32, Pos2, Rect as ERect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use glam::{Vec2, Vec3};
use ta_formats::image::Palette;
use ta_formats::tnt::TILE_SIZE;
use ta_render::Camera;

use super::doc::MapDoc;
use crate::palette::team_color;
use crate::theme::*;

/// The picture's longer side, in pixels.
const PICTURE: usize = 256;
/// Seconds between pictures while the art keeps changing (a stroke in progress).
const REDRAW_EVERY: f32 = 0.4;

#[derive(Default)]
pub struct Navigator {
    texture: Option<egui::TextureHandle>,
    /// The art changed since the picture was made.
    pub dirty: bool,
    /// Seconds since the picture was made.
    age: f32,
    /// A drag that began on the navigator (it keeps the camera while held).
    dragging: bool,
}

impl Navigator {
    /// Shows the navigator `size` points across at the bottom right of `area` (the view);
    /// returns a point of the map to look at when it was clicked or dragged over.
    pub fn show(&mut self, ui: &mut egui::Ui, area: ERect, size: f32, doc: &MapDoc, palette: &Palette, camera: &Camera) -> Option<[f32; 2]> {
        self.age += ui.ctx().input(|i| i.stable_dt).min(0.1);
        if self.texture.is_none() || self.dirty && self.age >= REDRAW_EVERY {
            let picture = picture(doc, palette);
            match &mut self.texture {
                Some(texture) => texture.set(picture, egui::TextureOptions::LINEAR),
                None => self.texture = Some(ui.ctx().load_texture("editor-navigator", picture, egui::TextureOptions::LINEAR)),
            }
            (self.dirty, self.age) = (false, 0.0);
        }
        let texture = self.texture.as_ref()?;
        let (mw, mh) = (doc.width_px().max(1.0), doc.height_px().max(1.0));
        let scale = (size / mw).min(size / mh);
        let picture = vec2(mw * scale, mh * scale);
        let frame = ERect::from_min_size(area.max - picture - vec2(12.0, 12.0), picture);
        let response = egui::Area::new(egui::Id::new("editor_navigator")).fixed_pos(frame.min - vec2(4.0, 4.0)).order(egui::Order::Middle).show(ui.ctx(), |ui| {
            let (rect, response) = ui.allocate_exact_size(picture + vec2(8.0, 8.0), Sense::click_and_drag());
            let inner = rect.shrink(4.0);
            let p = ui.painter();
            p.rect(rect, 3.0, Color32::from_black_alpha(200), Stroke::new(1.0, LINE_HI), StrokeKind::Inside);
            p.image(texture.id(), inner, ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            let at = |x: f32, z: f32| pos2(inner.min.x + x * scale, inner.min.y + z * scale);
            // The ground the camera shows (its corners on the ground at the focus's height).
            let corners = footprint(camera, area, ui.ctx().pixels_per_point());
            let quad: Vec<Pos2> = corners.iter().map(|c| {
                let q = at(c.x.clamp(0.0, mw), c.y.clamp(0.0, mh));
                q.clamp(inner.min, inner.max)
            }).collect();
            if quad.len() == 4 {
                p.add(Shape::closed_line(quad, Stroke::new(1.5, AMBER_BRIGHT)));
            }
            for (i, s) in doc.starts.iter().enumerate() {
                let c = at(s[0], s[1]);
                p.circle(c, 3.5, team_color(i as u8), Stroke::new(1.0, Color32::from_black_alpha(220)));
            }
            response
        }).inner;
        let response = response.on_hover_cursor(egui::CursorIcon::Crosshair).on_hover_text(ta_ui::i18n::t("Click or drag to look there"));
        if response.drag_started() || response.clicked() {
            self.dragging = true;
        }
        if !response.dragged() && !response.clicked() {
            self.dragging = false;
        }
        let pointer = response.interact_pointer_pos().filter(|_| self.dragging || response.clicked())?;
        let x = ((pointer.x - frame.min.x) / scale).clamp(0.0, mw);
        let z = ((pointer.y - frame.min.y) / scale).clamp(0.0, mh);
        Some([x, z])
    }
}

/// Where the corners of `area` (egui points) meet the ground at the camera's focus height, as
/// map points; a corner showing the sky is taken far along its ray.
fn footprint(camera: &Camera, area: ERect, pixels_per_point: f32) -> Vec<Vec2> {
    let level = camera.focus.y;
    [area.left_top(), area.right_top(), area.right_bottom(), area.left_bottom()]
        .iter()
        .map(|c| {
            let (origin, dir) = camera.screen_ray(Vec2::new(c.x, c.y) * pixels_per_point);
            let point = if dir.y < -0.02 {
                origin + dir * ((level - origin.y) / dir.y)
            } else {
                let flat = Vec3::new(dir.x, 0.0, dir.z).normalize_or(Vec3::Z);
                Vec3::new(camera.focus.x, level, camera.focus.z) + flat * 8000.0
            };
            Vec2::new(point.x, point.z)
        })
        .collect()
}

/// The map's art, `PICTURE` pixels on its longer side: each pixel the mean of up to 4 × 4
/// samples of the tiles under it.
fn picture(doc: &MapDoc, palette: &Palette) -> egui::ColorImage {
    let t = &doc.tnt;
    let (aw, ah) = (doc.tiles_wide() as usize * TILE_SIZE, doc.tiles_high() as usize * TILE_SIZE);
    let (w, h) = if aw >= ah { (PICTURE, (PICTURE * ah / aw.max(1)).max(1)) } else { ((PICTURE * aw / ah.max(1)).max(1), PICTURE) };
    let tw = doc.tiles_wide() as usize;
    let mut rgba = vec![0u8; w * h * 4];
    for py in 0..h {
        let (y0, y1) = (py * ah / h, ((py + 1) * ah / h).max(py * ah / h + 1));
        for px in 0..w {
            let (x0, x1) = (px * aw / w, ((px + 1) * aw / w).max(px * aw / w + 1));
            let (sx, sy) = ((x1 - x0).div_ceil(4).max(1), (y1 - y0).div_ceil(4).max(1));
            let (mut sum, mut n) = ([0u32; 3], 0u32);
            for y in (y0..y1).step_by(sy) {
                for x in (x0..x1).step_by(sx) {
                    let tile = t.tile_map.get((y / TILE_SIZE) * tw + x / TILE_SIZE).copied().unwrap_or(0) as usize;
                    let index = t.tiles.get(tile).map_or(0, |tile| tile[(y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE]);
                    let [r, g, b] = palette.rgb(index);
                    sum[0] += r as u32;
                    sum[1] += g as u32;
                    sum[2] += b as u32;
                    n += 1;
                }
            }
            let n = n.max(1);
            let o = (py * w + px) * 4;
            rgba[o..o + 4].copy_from_slice(&[(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, 255]);
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba)
}
