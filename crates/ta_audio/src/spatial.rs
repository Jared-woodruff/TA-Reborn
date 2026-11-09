//! Positional audio for an RTS camera.
//!
//! World space follows the renderer: `x` east, `y` up, `z` south, one unit per TA map pixel.
//! The listener sits at the camera's ground focus and "hears" roughly what the player sees:
//! distances are measured in half view widths (`d = 1` is the left/right screen edge), so
//! zooming out widens the audible area together with the visible one.
//!
//! * Gain: full volume within [`SpatialSettings::full_volume_radius`], then a curve that is
//!   quadratic in decibels down to `-falloff_db` at [`SpatialSettings::audible_radius`], and
//!   silence beyond. With the defaults a sound at the screen edge plays at -6.7 dB, half a
//!   screen further out at -27 dB, and a full screen width away from the edge it is culled.
//! * Pan: proportional to the offset along the camera's right vector, reaching
//!   [`SpatialSettings::max_pan`] at the screen edge; equal-power law for mono sources.
//! * Zoom: optionally, zooming out beyond [`SpatialSettings::reference_view_width`] lowers all
//!   positional sounds a little (a distant camera hears the battle as a whole).

use serde::{Deserialize, Serialize};

/// The ear: position and orientation of the camera projected onto the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    /// Camera focus (ground point at the screen centre).
    pub position: [f32; 3],
    /// Unit vector pointing to the top of the screen.
    pub forward: [f32; 3],
    /// Unit vector pointing to the right of the screen.
    pub right: [f32; 3],
    /// Width of the visible ground in world units.
    pub view_width: f32,
}

impl Default for Listener {
    fn default() -> Self {
        // TA's fixed camera: north (-z) is up, east (+x) is right, 1280 px wide.
        Listener { position: [0.0; 3], forward: [0.0, 0.0, -1.0], right: [1.0, 0.0, 0.0], view_width: 1280.0 }
    }
}

impl Listener {
    /// Builds a listener, normalising the axes (degenerate axes fall back to TA's defaults).
    pub fn new(position: [f32; 3], forward: [f32; 3], right: [f32; 3], view_width: f32) -> Listener {
        let d = Listener::default();
        Listener {
            position,
            forward: normalize(forward).unwrap_or(d.forward),
            right: normalize(right).unwrap_or(d.right),
            view_width: if view_width.is_finite() && view_width > 1.0 { view_width } else { d.view_width },
        }
    }
}

fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (len.is_finite() && len > 1e-6).then(|| [v[0] / len, v[1] / len, v[2] / len])
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Tuning of the positional model. Distances are in half view widths.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpatialSettings {
    /// Sounds closer than this play at full volume.
    pub full_volume_radius: f32,
    /// Sounds at or beyond this distance are silent (and not started).
    pub audible_radius: f32,
    /// Attenuation reached at `audible_radius`, in dB.
    pub falloff_db: f32,
    /// Largest pan applied (0 = mono, 1 = hard left/right).
    pub max_pan: f32,
    /// Horizontal offset at which `max_pan` is reached.
    pub pan_radius: f32,
    /// View width (world units) below which zoom attenuation is 0 dB.
    pub reference_view_width: f32,
    /// Exponent of the zoom attenuation `(reference / view_width)^exponent`; 0 disables it.
    pub zoom_rolloff: f32,
    /// Lower bound of the zoom attenuation factor.
    pub min_zoom_gain: f32,
    /// A gain on every placed sound, the world's (the battle behind the title screen steps
    /// back under the music); 1 in a battle. The interface's sounds keep theirs.
    pub world_gain: f32,
}

impl Default for SpatialSettings {
    fn default() -> Self {
        SpatialSettings {
            full_volume_radius: 0.5,
            audible_radius: 2.0,
            falloff_db: 60.0,
            max_pan: 0.8,
            pan_radius: 1.0,
            reference_view_width: 1280.0,
            zoom_rolloff: 0.5,
            min_zoom_gain: 0.5,
            world_gain: 1.0,
        }
    }
}

/// Result of placing a sound relative to the listener.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spatial {
    /// Linear gain in `0..=1`.
    pub gain: f32,
    /// Pan in `-1..=1` (negative = left).
    pub pan: f32,
}

/// Distance attenuation for a distance measured in half view widths.
pub fn distance_gain(distance: f32, settings: &SpatialSettings) -> f32 {
    let r0 = settings.full_volume_radius.max(0.0);
    let r1 = settings.audible_radius.max(r0 + 1e-3);
    if !distance.is_finite() || distance >= r1 {
        0.0
    } else if distance <= r0 {
        1.0
    } else {
        let x = (distance - r0) / (r1 - r0);
        10f32.powf(-settings.falloff_db * x * x / 20.0)
    }
}

/// Places a sound at `position` relative to the listener.
pub fn spatialize(listener: &Listener, settings: &SpatialSettings, position: [f32; 3]) -> Spatial {
    let half = listener.view_width * 0.5;
    let l = listener.position;
    let d = [position[0] - l[0], position[1] - l[1], position[2] - l[2]];
    let across = dot(d, listener.right);
    let along = dot(d, listener.forward);
    let total_sq = dot(d, d);
    // Whatever is neither across nor along the screen is height; it counts at full weight.
    let distance = total_sq.max(across * across + along * along).sqrt() / half;
    let zoom = if settings.zoom_rolloff > 0.0 && listener.view_width > settings.reference_view_width {
        (settings.reference_view_width / listener.view_width).powf(settings.zoom_rolloff).max(settings.min_zoom_gain)
    } else {
        1.0
    };
    let pan = (across / (half * settings.pan_radius.max(1e-3))).clamp(-1.0, 1.0) * settings.max_pan.clamp(0.0, 1.0);
    let world = if settings.world_gain.is_finite() { settings.world_gain.clamp(0.0, 1.0) } else { 1.0 };
    Spatial { gain: distance_gain(distance, settings) * zoom * world, pan: if pan.is_finite() { pan } else { 0.0 } }
}

/// Output gains `[left, right]` for a pan position. Mono sources use the equal-power law
/// (-3 dB per side at the centre); stereo sources use a balance law (unity at the centre).
pub fn pan_gains(pan: f32, source_channels: u16) -> [f32; 2] {
    let p = pan.clamp(-1.0, 1.0);
    if source_channels >= 2 {
        [(1.0 - p).min(1.0), (1.0 + p).min(1.0)]
    } else {
        let angle = (p + 1.0) * std::f32::consts::FRAC_PI_4;
        [angle.cos(), angle.sin()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, z: f32) -> [f32; 3] {
        [1000.0 + x, 0.0, 1000.0 + z]
    }

    fn listener() -> Listener {
        Listener::new([1000.0, 0.0, 1000.0], [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], 1280.0)
    }

    #[test]
    fn gain_falls_off_with_distance() {
        let s = SpatialSettings::default();
        let l = listener();
        assert_eq!(spatialize(&l, &s, at(0.0, 0.0)).gain, 1.0);
        assert_eq!(spatialize(&l, &s, at(300.0, 0.0)).gain, 1.0);
        let edge = spatialize(&l, &s, at(640.0, 0.0)).gain;
        assert!((20.0 * edge.log10() + 6.67).abs() < 0.1, "screen edge at {} dB", 20.0 * edge.log10());
        let far = spatialize(&l, &s, at(0.0, 960.0)).gain;
        assert!(far < 0.05 && far > 0.0);
        assert_eq!(spatialize(&l, &s, at(1280.0, 0.0)).gain, 0.0);
        // Monotonic.
        let gains: Vec<f32> = (0..300).map(|i| spatialize(&l, &s, at(i as f32 * 5.0, 0.0)).gain).collect();
        assert!(gains.windows(2).all(|w| w[1] <= w[0]));
    }

    #[test]
    fn the_world_gain_steps_placed_sounds_back() {
        let near = SpatialSettings::default();
        let back = SpatialSettings { world_gain: 0.5, ..near };
        let l = listener();
        assert_eq!(spatialize(&l, &back, at(0.0, 0.0)).gain, 0.5);
        let edge = (spatialize(&l, &near, at(640.0, 0.0)).gain, spatialize(&l, &back, at(640.0, 0.0)).gain);
        assert!((edge.1 - edge.0 * 0.5).abs() < 1e-6);
        assert_eq!(spatialize(&l, &SpatialSettings { world_gain: f32::NAN, ..near }, at(0.0, 0.0)).gain, 1.0, "nonsense is ignored");
    }

    #[test]
    fn pans_follow_camera_orientation() {
        let s = SpatialSettings::default();
        let east = spatialize(&listener(), &s, at(320.0, 0.0));
        assert!((east.pan - 0.4).abs() < 1e-5);
        assert!(spatialize(&listener(), &s, at(-2000.0, 0.0)).pan == -0.8);
        // Camera rotated 180°: east is now on the left.
        let turned = Listener::new([1000.0, 0.0, 1000.0], [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], 1280.0);
        assert!(spatialize(&turned, &s, at(320.0, 0.0)).pan < 0.0);
        // Up/down the screen does not pan.
        assert_eq!(spatialize(&listener(), &s, at(0.0, -400.0)).pan, 0.0);
    }

    #[test]
    fn audible_area_scales_with_zoom() {
        let s = SpatialSettings::default();
        let near = listener();
        let wide = Listener { view_width: 5120.0, ..near };
        let p = at(1280.0, 0.0);
        assert_eq!(spatialize(&near, &s, p).gain, 0.0);
        let g = spatialize(&wide, &s, p).gain;
        // Inside the wide view's full-volume radius, attenuated only by the zoom factor.
        assert!((g - 0.5).abs() < 1e-6, "{g}");
        let off = SpatialSettings { zoom_rolloff: 0.0, ..s };
        assert_eq!(spatialize(&wide, &off, p).gain, 1.0);
    }

    #[test]
    fn pan_laws() {
        let [l, r] = pan_gains(0.0, 1);
        assert!((l - r).abs() < 1e-6 && (l * l + r * r - 1.0).abs() < 1e-6);
        let [l, r] = pan_gains(1.0, 1);
        assert!(l.abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
        assert_eq!(pan_gains(0.0, 2), [1.0, 1.0]);
        assert_eq!(pan_gains(-0.5, 2), [1.0, 0.5]);
    }

    #[test]
    fn degenerate_listener_is_repaired() {
        let l = Listener::new([0.0; 3], [0.0; 3], [f32::NAN, 0.0, 0.0], -5.0);
        assert_eq!(l, Listener::default());
    }
}
