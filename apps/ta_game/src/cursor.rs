//! TA's own mouse cursors (`anims/cursors.gaf`): animated, and chosen from what a click
//! would do. The system cursor is hidden while one is drawn.

use std::collections::HashMap;

use egui::{Color32, Rect, TextureHandle, pos2, vec2};
use ta_formats::gaf::Gaf;
use ta_formats::image::Palette;
use ta_vfs::Vfs;

/// Cursor animation speed (frames per second).
const FPS: f64 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CursorKind {
    Normal,
    Select,
    Move,
    Attack,
    /// TA's air-strike sight: bombers sent at something (`game::clicks::bombers_only`).
    AirStrike,
    Patrol,
    Defend,
    Repair,
    Reclaim,
    Capture,
    Revive,
    Load,
    Unload,
}

impl CursorKind {
    const ALL: [CursorKind; 13] = [
        CursorKind::Normal,
        CursorKind::Select,
        CursorKind::Move,
        CursorKind::Attack,
        CursorKind::AirStrike,
        CursorKind::Patrol,
        CursorKind::Defend,
        CursorKind::Repair,
        CursorKind::Reclaim,
        CursorKind::Capture,
        CursorKind::Revive,
        CursorKind::Load,
        CursorKind::Unload,
    ];

    fn gaf_name(self) -> &'static str {
        match self {
            CursorKind::Normal => "cursornormal",
            CursorKind::Select => "cursorselect",
            CursorKind::Move => "cursormove",
            CursorKind::Attack => "cursorattack",
            CursorKind::AirStrike => "cursorairstrike",
            CursorKind::Patrol => "cursorpatrol",
            CursorKind::Defend => "cursordefend",
            CursorKind::Repair => "cursorrepair",
            CursorKind::Reclaim => "cursorreclamate",
            CursorKind::Capture => "cursorcapture",
            CursorKind::Revive => "cursorrevive",
            CursorKind::Load => "cursorload",
            CursorKind::Unload => "cursorunload",
        }
    }
}

struct Frame {
    texture: TextureHandle,
    size: egui::Vec2,
    /// The hot spot, from the frame's top-left.
    origin: egui::Vec2,
}

/// The cursor images, loaded on first use.
#[derive(Default)]
pub struct Cursors {
    loaded: bool,
    /// TA's cursor was drawn this frame: the system's must stay hidden to the frame's end
    /// (see `drawn`), whatever a dialog drawn later asks for over its buttons.
    drawn: bool,
    anims: HashMap<CursorKind, Vec<Frame>>,
}

impl Cursors {
    /// A frame without TA's cursor (the setting is off).
    pub fn draw_none(&mut self) {
        self.drawn = false;
    }

    /// Whether TA's cursor stands in for the system's this frame.
    pub fn drawn(&self) -> bool {
        self.drawn
    }

    fn load(&mut self, ctx: &egui::Context, vfs: &Vfs, palette: &Palette) {
        self.loaded = true;
        let Some(gaf) = vfs.read_opt("anims/cursors.gaf").and_then(|b| Gaf::parse(&b).ok()) else {
            log::warn!("anims/cursors.gaf not found: using the system cursor");
            return;
        };
        for kind in CursorKind::ALL {
            let Some(entry) = gaf.entry(kind.gaf_name()) else { continue };
            let frames = entry
                .frames
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let (w, h) = (f.image.width.max(1) as usize, f.image.height.max(1) as usize);
                    let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &f.image.to_rgba(palette));
                    Frame {
                        texture: ctx.load_texture(format!("{}-{i}", kind.gaf_name()), image, egui::TextureOptions::NEAREST),
                        size: vec2(w as f32, h as f32),
                        origin: vec2(f.origin_x as f32, f.origin_y as f32),
                    }
                })
                .collect();
            self.anims.insert(kind, frames);
        }
    }

    /// Draws `kind` at the pointer in place of the system cursor. Nothing happens (and the
    /// system cursor stays) when the pointer is outside the window or the images are missing.
    pub fn draw(&mut self, ctx: &egui::Context, vfs: &Vfs, palette: &Palette, kind: CursorKind, scale: f32, high_contrast: bool) {
        self.drawn = false;
        if !self.loaded {
            self.load(ctx, vfs, palette);
        }
        let Some(pos) = ctx.pointer_hover_pos() else { return };
        let Some(frames) = self.anims.get(&kind).or_else(|| self.anims.get(&CursorKind::Normal)).filter(|f| !f.is_empty()) else { return };
        let still = ta_ui::motion::still(ctx);
        let time = if still { 0.0 } else { ctx.input(|i| i.time) };
        let frame = &frames[(time * FPS) as usize % frames.len()];
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("ta_cursor")));
        let scale = scale.max(1.0).min(2.5);
        let rect = cursor_rect(pos, frame.origin, frame.size, scale);
        // A static black/white locator remains legible over both bright and dark terrain.
        // Centre it on the actual click position; enlarging the art must not move the hot spot.
        if high_contrast {
            painter.circle_stroke(pos, 13.0 * scale, egui::Stroke::new(4.0, Color32::BLACK));
            painter.circle_stroke(pos, 13.0 * scale, egui::Stroke::new(2.0, Color32::WHITE));
        }
        painter.image(frame.texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        ctx.set_cursor_icon(egui::CursorIcon::None);
        self.drawn = true;
        if frames.len() > 1 && !still {
            ctx.request_repaint();
        }
    }
}

fn cursor_rect(pos: egui::Pos2, origin: egui::Vec2, size: egui::Vec2, scale: f32) -> Rect {
    Rect::from_min_size(pos - origin * scale, size * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enlarged_cursors_keep_the_click_at_the_original_hot_spot() {
        let pointer = pos2(120.0, 80.0);
        for scale in [1.0, 1.5, 2.5] {
            let arrow = cursor_rect(pointer, vec2(0.0, 0.0), vec2(24.0, 24.0), scale);
            assert_eq!(arrow.min, pointer);
            assert_eq!(arrow.size(), vec2(24.0, 24.0) * scale);
            let crosshair = cursor_rect(pointer, vec2(16.0, 16.0), vec2(32.0, 32.0), scale);
            assert_eq!(crosshair.center(), pointer);
        }
    }

    #[test]
    fn every_cursor_loads_from_the_game_data() {
        let Ok(install) = ta_vfs::install::discover(None) else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let mut vfs = Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let palette = Palette::parse(&vfs.read("palettes/palette.pal").unwrap()).unwrap();
        let ctx = egui::Context::default();
        let mut cursors = Cursors::default();
        cursors.load(&ctx, &vfs, &palette);
        for kind in CursorKind::ALL {
            let frames = cursors.anims.get(&kind).unwrap_or_else(|| panic!("{kind:?} missing"));
            assert!(!frames.is_empty(), "{kind:?}");
            for f in frames {
                // The hot spot lies on (or at the edge of) the image.
                assert!(f.origin.x >= 0.0 && f.origin.y >= 0.0 && f.origin.x <= f.size.x && f.origin.y <= f.size.y, "{kind:?} hot spot {:?} in {:?}", f.origin, f.size);
            }
        }
        // The arrow points from its top-left corner; the attack sight from its centre.
        assert_eq!(cursors.anims[&CursorKind::Normal][0].origin, vec2(0.0, 0.0));
        assert_eq!(cursors.anims[&CursorKind::Attack][0].origin, vec2(16.0, 16.0));
    }
}
