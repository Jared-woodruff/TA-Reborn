//! Crisp faction marks shared by profile badges and seasonal decoration.
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, vec2};
use ta_config::profiles::Allegiance;

pub fn paint(painter: &Painter, rect: Rect, allegiance: Allegiance) {
    let scale = rect.width().min(rect.height()) / 14.0;
    let center = rect.center();
    let at = |x: f32, y: f32| center + vec2(x, y) * scale;
    mark(painter, &at, scale, allegiance == Allegiance::Arm);
}

/// Small vector interpretations of the faction insignia: ARM's diamond and split wings,
/// CORE's domed helmet and inner chevron. These are drawn shapes, not bundled TA art.
pub(crate) fn mark(painter: &Painter, at: &impl Fn(f32, f32) -> Pos2, scale: f32, arm: bool) {
    let ink = Color32::from_rgb(237, 244, 249);
    if arm {
        painter.add(Shape::convex_polygon(vec![at(0.0, -2.5), at(1.8, 0.0), at(0.0, 2.5), at(-1.8, 0.0)], ink, Stroke::NONE));
        for side in [-1.0, 1.0] {
            painter.add(Shape::line(vec![at(side * 0.7, -4.2), at(side * 3.1, -1.0), at(side * 5.5, -1.0)], Stroke::new(1.05 * scale, ink)));
            painter.add(Shape::line(vec![at(side * 5.5, 1.0), at(side * 3.1, 1.0), at(0.0, 5.1)], Stroke::new(1.05 * scale, ink)));
        }
    } else {
        for side in [-1.0, 1.0] {
            painter.add(Shape::convex_polygon(vec![at(side * 0.5, -5.2), at(side * 2.7, -4.4), at(side * 4.0, -2.7), at(side * 4.2, -1.4)], ink, Stroke::NONE));
            painter.add(Shape::convex_polygon(vec![at(0.0, -4.0), at(side * 5.0, 0.1), at(side * 3.4, 1.7), at(0.0, -1.6)], ink, Stroke::NONE));
            painter.add(Shape::convex_polygon(vec![at(side * 5.0, 0.1), at(side * 3.4, 1.7), at(side * 3.7, 6.1)], ink, Stroke::NONE));
        }
        painter.add(Shape::convex_polygon(vec![at(0.0, -0.7), at(2.7, 1.8), at(0.0, 2.6), at(-2.7, 1.8)], ink, Stroke::NONE));
    }
}
