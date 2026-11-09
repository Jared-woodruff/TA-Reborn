//! Lights of the night (a time of day, see `ta_render::daylight`): as darkness falls, bases
//! light up (a floodlight over every finished building at work, sized to it), factories glow green
//! while they build, units show their running lights and aircraft blink their beacons, and the
//! lanterns and windows of TA Reborn's own festive features shine (`FeatureView::night_lights`).
//! Only what the viewer sees and the camera frames; the renderer keeps the brightest of all the
//! frame's lights (explosions and weapons first).

use glam::{Vec2, Vec3};
use ta_render::PointLight;
use ta_sim::{Locomotion, Seen};

use crate::game::Game;

/// Adds the night's lights to `out`: `lamps` (0..1) is how far they are on, `alpha` the
/// frame's interpolation between ticks, `time` game seconds (for blinking), `view` the ground
/// in view.
pub fn lights(game: &Game, lamps: f32, alpha: f32, time: f32, view: (Vec2, Vec2), out: &mut Vec<PointLight>) {
    if lamps <= 0.0 {
        return;
    }
    game.features.night_lights(&game.sim, lamps, time, view, out);
    let (lo, hi) = (view.0 - Vec2::splat(160.0), view.1 + Vec2::splat(160.0));
    for u in game.sim.units() {
        if u.dead || u.carried_by.is_some() || u.cloaked || u.build_progress < ta_sim::Fx::ONE || game.seen(u) != Seen::Visible {
            continue;
        }
        let Some(p) = game.scene.unit_position(u.id, alpha) else { continue };
        if p.x < lo.x || p.x > hi.x || p.z < lo.y || p.z > hi.y {
            continue;
        }
        let t = game.sim.unit_type(u);
        // Walls and dragon's teeth stay dark.
        if t.def.is_feature {
            continue;
        }
        // And so does what is not at work: a metal maker switched off, a radar short of
        // energy, anything stunned (its lights go out as its lamps' glow does).
        let lamps = lamps * game.scene.lights_on(u.id, alpha);
        if lamps <= 0.0 {
            continue;
        }
        match t.locomotion {
            Locomotion::Static => {
                let size = t.footprint.0.max(t.footprint.1) as f32;
                let k = 0.9 * lamps;
                out.push(PointLight::new((p + Vec3::Y * (18.0 + 5.0 * size)).to_array(), [1.0 * k, 0.8 * k, 0.52 * k], (60.0 + 20.0 * size).min(190.0)));
                if t.is_factory && u.build.is_some() {
                    let flicker = 0.85 + 0.15 * (time * 9.0 + u.id.slot as f32).sin();
                    let g = 1.2 * lamps * flicker;
                    out.push(PointLight::new((p + Vec3::Y * 24.0).to_array(), [0.3 * g, 1.0 * g, 0.45 * g], 120.0));
                }
            }
            Locomotion::Air => {
                // A red beacon, flashing about once a second (each aircraft in its own rhythm).
                let phase = (time * 1.1 + u.id.slot as f32 * 0.37).fract();
                let blink = if phase < 0.18 { 1.0 } else { 0.08 };
                let k = 1.6 * lamps * blink;
                out.push(PointLight::new(p.to_array(), [k, 0.18 * k, 0.12 * k], 38.0));
            }
            _ => {
                let (k, radius) = if t.def.is_commander { (0.8 * lamps, 72.0) } else { (0.45 * lamps, 44.0) };
                out.push(PointLight::new((p + Vec3::Y * 10.0).to_array(), [0.75 * k, 0.85 * k, 1.0 * k], radius));
            }
        }
    }
}
