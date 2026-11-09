//! Strategic zoom: far out, where units are a few pixels across, each one the viewer sees
//! also shows as an icon in its owner's colour (`ta_render::icons`), fading in as the camera
//! pulls back. The shape says what it is: the Commander a star; buildings squares (factories
//! hollow, armed ones hexagons); land units circles (constructors hollow); aircraft
//! triangles; ships and hovercraft diamonds. Bigger, costlier units get bigger icons, and the
//! selection wears white rings.

use glam::{Vec2, Vec3};
use ta_render::Camera;
use ta_render::icons::{IconInstance, IconShape};
use ta_sim::{Locomotion, Seen, UnitType};

use crate::game::Game;

/// Zoom (screen points per world unit) below which icons show fully, and above which they are
/// gone: a light tank (about 32 units long) is 8 and 16 points long there.
const ZOOM_FULL: f32 = 0.25;
const ZOOM_NONE: f32 = 0.5;

/// How far the icons show at `camera`'s zoom (0..1), `ppp` physical pixels per point.
pub fn amount(camera: &Camera, ppp: f32) -> f32 {
    let zoom = camera.zoom / ppp.max(0.25);
    let t = ((ZOOM_NONE - zoom) / (ZOOM_NONE - ZOOM_FULL)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How far the icons show for `game`'s player: [`amount`], or not at all when the player turned
/// them off (Interface: STRATEGIC ICONS). What the icons stand in for far out (the selection's
/// rings, the markers' height over the units) follows this; what only depends on how far out the
/// camera is (picking a unit a pixel or two across, the build times fading) follows [`amount`].
pub fn shown(game: &Game, camera: &Camera, ppp: f32) -> f32 {
    if game.strategic_icons { amount(camera, ppp) } else { 0.0 }
}

/// A unit type's icon and its size in points.
pub fn icon_of(t: &UnitType) -> (IconShape, f32) {
    // 0 for the cheapest units, 1 for the costliest (in metal).
    let weight = ((t.cost_metal.to_f32() - 40.0) / 1400.0).clamp(0.0, 1.0).sqrt();
    if t.def.is_commander {
        return (IconShape::Star, 18.0);
    }
    match t.locomotion {
        Locomotion::Static => {
            let size = 9.0 + 1.0 * t.footprint.0.max(t.footprint.1).min(6) as f32;
            if t.is_factory {
                (IconShape::HollowSquare, size + 2.0)
            } else if t.weapons.iter().any(|w| w.is_some()) {
                (IconShape::Hexagon, size)
            } else {
                (IconShape::Square, size - 1.0)
            }
        }
        Locomotion::Air => (IconShape::Triangle, 10.0 + 4.0 * weight),
        Locomotion::Naval | Locomotion::Hover => (IconShape::Diamond, 10.0 + 5.0 * weight),
        Locomotion::Ground if t.is_builder => (IconShape::HollowCircle, 11.0),
        Locomotion::Ground => (IconShape::Circle, 8.0 + 5.0 * weight),
    }
}

/// Adds this frame's icons to `out`: the units the viewer sees on the ground in `view` (with a
/// margin), at their interpolated positions (`alpha`); `ppp` physical pixels per point.
pub fn icons(game: &Game, camera: &Camera, ppp: f32, alpha: f32, view: (Vec2, Vec2), out: &mut Vec<IconInstance>) {
    let k = shown(game, camera, ppp);
    if k <= 0.0 {
        return;
    }
    let (lo, hi) = (view.0 - Vec2::splat(200.0), view.1 + Vec2::splat(200.0));
    let selected: rustc_hash::FxHashSet<ta_sim::UnitId> = game.selection.iter().copied().collect();
    // Each player's colour, once (not once a unit).
    let colours: Vec<[f32; 3]> = game
        .sim
        .players
        .iter()
        .map(|pl| egui::Rgba::from(crate::palette::team_color(pl.color)))
        .map(|c| [c.r(), c.g(), c.b()])
        .collect();
    let fallback = {
        let c = egui::Rgba::from(crate::palette::team_color(0));
        [c.r(), c.g(), c.b()]
    };
    // (Drawn in order: buildings under ground units under ships under aircraft, the
    // Commander and the selection's rings on top.)
    let mut layers: [Vec<IconInstance>; 6] = Default::default();
    for u in game.sim.units() {
        if u.dead || u.carried_by.is_some() {
            continue;
        }
        let Some(p) = game.scene.unit_position(u.id, alpha) else { continue };
        if p.x < lo.x || p.x > hi.x || p.z < lo.y || p.z > hi.y || game.seen(u) != Seen::Visible {
            continue;
        }
        let t = game.sim.unit_type(u);
        let (shape, size) = icon_of(t);
        let color = colours.get(u.owner as usize).copied().unwrap_or(fallback);
        // Unfinished buildings are paler.
        let opacity = k * if u.is_complete() { 1.0 } else { 0.55 };
        let at = (p + Vec3::Y * 8.0).to_array();
        let layer = match (t.def.is_commander, t.locomotion) {
            (true, _) => 4,
            (_, Locomotion::Static) => 0,
            (_, Locomotion::Ground) => 1,
            (_, Locomotion::Naval | Locomotion::Hover) => 2,
            (_, Locomotion::Air) => 3,
        };
        layers[layer].push(IconInstance::new(at, size * ppp, shape, color, opacity));
        if selected.contains(&u.id) {
            layers[5].push(IconInstance::new(at, (size + 8.0) * ppp, IconShape::Ring, [1.0, 1.0, 1.0], k));
        }
    }
    for layer in layers {
        out.extend(layer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_fade_in_as_the_camera_pulls_back() {
        let mut camera = Camera::new(glam::Vec2::new(1920.0, 1080.0));
        camera.zoom = 1.0;
        assert_eq!(amount(&camera, 1.0), 0.0);
        camera.zoom = 0.2;
        assert_eq!(amount(&camera, 1.0), 1.0);
        camera.zoom = 0.375;
        assert!((amount(&camera, 1.0) - 0.5).abs() < 1e-5, "halfway");
        // A high-DPI screen: the same size in points.
        camera.zoom = 0.4;
        assert!((amount(&camera, 2.0) - 1.0).abs() < 1e-5);
    }
}
