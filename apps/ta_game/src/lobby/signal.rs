//! A connection at a glance: three bars lit by how quick its round trip is (all three under
//! 80 ms, two under 160, one beyond), in green, amber or red. The lobby's rows and the battle's
//! top bar show them.

use egui::{Color32, Painter, Rect, pos2, vec2};

use crate::theme::*;

/// A round trip's colour: under 80 ms good, under 160 fair, else slow.
pub fn tone(ms: u16) -> Color32 {
    if ms < 80 {
        GREEN
    } else if ms < 160 {
        AMBER
    } else {
        RED
    }
}

/// The bars at the left of `rect`, rising, lit by `ms` (none: still measuring, all dark).
pub fn bars(painter: &Painter, rect: Rect, ms: Option<u16>) {
    let tone = ms.map_or(STEEL_DIM, tone);
    let lit = match ms {
        Some(ms) if ms < 80 => 3,
        Some(ms) if ms < 160 => 2,
        Some(_) => 1,
        None => 0,
    };
    for k in 0..3 {
        let h = 5.0 + 3.0 * k as f32;
        let bar = Rect::from_min_size(pos2(rect.min.x + 2.0 + 5.0 * k as f32, rect.center().y + 6.0 - h), vec2(3.0, h));
        painter.rect_filled(bar, 0.5, if k < lit { tone } else { Color32::from_white_alpha(28) });
    }
}
