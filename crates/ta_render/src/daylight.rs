//! Time of day (perspective camera): the light's path through the day, its colour and
//! strength, the moon at night, the sky's colours from dawn to night, and how the painted
//! ground is lit.
//!
//! TA's art has its lighting painted in, for a sun high in the north-west. At noon everything
//! is exactly TA's (the map's own [`crate::Lighting`]); the sun's path keeps it in the
//! northern half of the sky (it rises in the north and sets in the west), so painted hills
//! stay lit on the side their painter lit them, and the moon shines from TA's own direction.
//! Low in the sky the sun turns golden, then red; after it sets, the horizon glows and fades,
//! the ground takes the blue of the night sky, the moon's weaker light and the stars come out.

use glam::Vec3;

/// The hour at which TA's own light shines.
pub const NOON: f32 = 12.0;

/// How far (degrees) the sun's direction turns from TA's by six in the morning or evening.
const AZIMUTH_SWING: f32 = 45.0;
/// The moon's height above the horizon (degrees), in TA's direction.
const MOON_ELEVATION: f32 = 42.0;
/// The moon's light: its colour and strength.
const MOON_COLOR: Vec3 = Vec3::new(0.62, 0.72, 1.0);
const MOON_STRENGTH: f32 = 0.24;

/// The map's light at noon: what [`at`] returns at [`NOON`], and what the times of day
/// depart from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Noon {
    /// Direction towards the sun.
    pub sun_dir: Vec3,
    pub sun_color: Vec3,
    pub sun_intensity: f32,
    pub ambient: Vec3,
    /// The sky at the horizon (the distance haze) and straight up.
    pub horizon: Vec3,
    pub zenith: Vec3,
    pub shadow: f32,
    /// Direction towards the moon, when it is placed (a film's shot); `None`: TA's direction,
    /// [`MOON_ELEVATION`] high.
    pub moon_dir: Option<Vec3>,
}

/// The light, the sky and the ground at one time of day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Daylight {
    /// Direction towards the sun itself (below the horizon at night; normalised).
    pub sun_dir: Vec3,
    /// Direction towards the light: the sun's, or at night the moon's (normalised).
    pub light_dir: Vec3,
    pub light_color: Vec3,
    pub light_intensity: f32,
    /// Ambient light on units.
    pub ambient: Vec3,
    pub horizon: Vec3,
    pub zenith: Vec3,
    /// The painted ground's sunlit term is multiplied by this: 1 at noon (the art as painted),
    /// the light's colour relative to TA's otherwise.
    pub ground_light: Vec3,
    /// Light the sky adds to the painted ground whatever the sun and the moon do (0 by day:
    /// the painting has it already).
    pub ground_skylight: Vec3,
    pub shadow: f32,
    /// 0..1: the warm glow along the horizon on the sun's side (dawn, dusk).
    pub dusk: f32,
    /// 0..1: how dark it is: the stars show, and the disc in the sky is the moon.
    pub night: f32,
    /// 0..1: how far the lights of the world are on (they come on as the sun sets).
    pub lamps: f32,
}

/// The sun's height above the horizon (degrees) at `hours`: TA's noon height at noon, on the
/// horizon at six, as far below at midnight.
pub fn sun_elevation(hours: f32, noon: &Noon) -> f32 {
    let top = noon.sun_dir.normalize_or(Vec3::Y).y.clamp(-1.0, 1.0).asin().to_degrees();
    top * (std::f32::consts::PI * (hours.rem_euclid(24.0) - 6.0) / 12.0).sin()
}

/// Unit vector at `elevation` degrees above the horizon, towards compass angle `azimuth`
/// (degrees, x = cos, z = sin).
fn direction(elevation: f32, azimuth: f32) -> Vec3 {
    let (el, az) = (elevation.to_radians(), azimuth.to_radians());
    Vec3::new(el.cos() * az.cos(), el.sin(), el.cos() * az.sin())
}

fn noon_azimuth(noon: &Noon) -> f32 {
    let d = noon.sun_dir.normalize_or(Vec3::Y);
    d.z.atan2(d.x).to_degrees()
}

/// Smooth step from `e0` to `e1` (either way round).
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A value along keys `(elevation, value)` in rising elevation order, eased between them and
/// held beyond the ends.
fn along<T: Copy + std::ops::Mul<f32, Output = T> + std::ops::Add<Output = T>>(el: f32, keys: &[(f32, T)]) -> T {
    let (first, last) = (keys[0], keys[keys.len() - 1]);
    if el <= first.0 {
        return first.1;
    }
    for pair in keys.windows(2) {
        let ((e0, a), (e1, b)) = (pair[0], pair[1]);
        if el <= e1 {
            let t = smooth(e0, e1, el);
            return a * (1.0 - t) + b * t;
        }
    }
    last.1
}

/// The light, sky and ground at `hours` (0..24; noon is [`NOON`]).
pub fn at(hours: f32, noon: &Noon) -> Daylight {
    let h = hours.rem_euclid(24.0);
    let el = sun_elevation(h, noon);
    let az = noon_azimuth(noon);
    // Morning light comes more from the north, evening light more from the west.
    let sun_dir = if (h - NOON).abs() < 1e-4 { noon.sun_dir.normalize_or(Vec3::Y) } else { direction(el, az - (h - NOON) * AZIMUTH_SWING / 6.0) };
    let moon_dir = noon.moon_dir.map_or_else(|| direction(MOON_ELEVATION, az), |m| m.normalize_or(Vec3::Y));

    // The sun fades out a little below the horizon (the sky still glows), the moon comes up
    // as twilight ends; whichever is brighter lights the world (both are faint at the switch).
    let sun_strength = ((el + 4.0) / 12.0).clamp(0.0, 1.0) * (0.72 + 0.28 * smooth(8.0, 40.0, el)) * noon.sun_intensity;
    let moon_strength = MOON_STRENGTH * smooth(-5.0, -14.0, el);
    let sun_color = along(el, &[(0.0, Vec3::new(1.0, 0.46, 0.22)), (6.0, Vec3::new(1.0, 0.66, 0.4)), (14.0, Vec3::new(1.0, 0.84, 0.66)), (30.0, noon.sun_color)]);
    let (light_dir, light_color, light_intensity) =
        if sun_strength >= moon_strength { (sun_dir, sun_color, sun_strength) } else { (moon_dir, MOON_COLOR, moon_strength) };

    let ambient = along(
        el,
        &[
            (-14.0, Vec3::new(0.05, 0.06, 0.1)),
            (-5.0, Vec3::new(0.15, 0.14, 0.21)),
            (5.0, noon.ambient * Vec3::new(0.85, 0.75, 0.75)),
            (25.0, noon.ambient),
        ],
    );
    let horizon = along(
        el,
        &[
            (-14.0, Vec3::new(0.011, 0.014, 0.026)),
            (-6.0, Vec3::new(0.3, 0.22, 0.34)),
            (0.0, Vec3::new(0.98, 0.52, 0.32)),
            (8.0, noon.horizon * 0.4 + Vec3::new(0.95, 0.72, 0.52) * 0.6),
            (22.0, noon.horizon),
        ],
    );
    let zenith = along(
        el,
        &[
            (-14.0, Vec3::new(0.002, 0.0035, 0.01)),
            (-6.0, Vec3::new(0.06, 0.08, 0.2)),
            (0.0, Vec3::new(0.2, 0.28, 0.52)),
            (8.0, noon.zenith * 0.9),
            (22.0, noon.zenith),
        ],
    );
    let ground_skylight = along(el, &[(-14.0, Vec3::new(0.03, 0.035, 0.06)), (-5.0, Vec3::new(0.12, 0.11, 0.16)), (2.0, Vec3::new(0.03, 0.025, 0.03)), (10.0, Vec3::ZERO)]);
    let shadow = if sun_strength >= moon_strength { noon.shadow * smooth(-2.0, 10.0, el) } else { noon.shadow * 0.55 * smooth(-5.0, -14.0, el) };
    let base = noon.sun_color.max(Vec3::splat(1e-3));
    Daylight {
        sun_dir,
        light_dir,
        light_color,
        light_intensity,
        ambient,
        horizon,
        zenith,
        // (A fifth of the painting's own light stays: its colours hold through the evening.)
        ground_light: Vec3::ONE.lerp(light_color / base, 0.8),
        ground_skylight,
        shadow,
        dusk: smooth(-8.0, -1.0, el) * (1.0 - smooth(6.0, 18.0, el)),
        night: smooth(-4.0, -14.0, el),
        lamps: smooth(3.0, -8.0, el),
    }
}

/// Hours of a day and night passing once every `cycle` seconds, at `seconds` into it (from
/// seven in the morning). Nights pass three times faster than days, so most of the time the
/// battlefield is in daylight: 15 hours of day (05:00 to 20:00) fill three quarters of the
/// cycle, 9 hours of night the last quarter.
pub fn cycle_hours(seconds: f32, cycle: f32) -> f32 {
    const DAY_START: f32 = 5.0;
    const DAY_HOURS: f32 = 15.0;
    const DAY_SHARE: f32 = 0.75;
    // Seven in the morning is 2/15 of the day in.
    let start = (7.0 - DAY_START) / DAY_HOURS * DAY_SHARE;
    let p = (seconds / cycle.max(1.0) + start).rem_euclid(1.0);
    if p < DAY_SHARE {
        DAY_START + p / DAY_SHARE * DAY_HOURS
    } else {
        (DAY_START + DAY_HOURS + (p - DAY_SHARE) / (1.0 - DAY_SHARE) * (24.0 - DAY_HOURS)).rem_euclid(24.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noon() -> Noon {
        Noon {
            sun_dir: Vec3::new(-0.45, 0.8, -0.4),
            sun_color: Vec3::new(1.0, 0.97, 0.92),
            sun_intensity: 1.0,
            ambient: Vec3::new(0.42, 0.45, 0.5),
            horizon: Vec3::new(0.55, 0.62, 0.7),
            zenith: Vec3::new(0.26, 0.38, 0.68),
            shadow: 0.85,
            moon_dir: None,
        }
    }

    #[test]
    fn noon_is_ta_s_own_light() {
        let n = noon();
        let d = at(NOON, &n);
        assert!((d.light_dir - n.sun_dir.normalize()).length() < 1e-5);
        assert_eq!((d.light_color, d.ambient, d.horizon, d.zenith), (n.sun_color, n.ambient, n.horizon, n.zenith));
        assert!((d.light_intensity - 1.0).abs() < 1e-5 && (d.shadow - n.shadow).abs() < 1e-5);
        assert!((d.ground_light - Vec3::ONE).length() < 1e-5, "the art as painted");
        assert_eq!((d.ground_skylight, d.dusk, d.night), (Vec3::ZERO, 0.0, 0.0));
    }

    #[test]
    fn the_sun_rises_in_the_north_and_sets_in_the_west() {
        let n = noon();
        let (morning, evening) = (at(7.0, &n).light_dir, at(17.0, &n).light_dir);
        assert!(morning.y > 0.0 && evening.y > 0.0);
        assert!(-morning.z > -morning.x, "morning light from the north: {morning}");
        assert!(-evening.x > -evening.z, "evening light from the west: {evening}");
        assert!(at(17.8, &n).dusk > 0.5, "a glow at sunset");
    }

    #[test]
    fn nights_are_moonlit_and_dark_blue() {
        let n = noon();
        let d = at(0.0, &n);
        assert_eq!(d.night, 1.0);
        assert_eq!(d.light_color, MOON_COLOR);
        assert!(d.light_intensity < 0.5 && d.light_dir.y > 0.5, "the moon, high in TA's direction");
        assert!(d.zenith.z > d.zenith.x && d.horizon.length() < 0.1);
        assert!(d.ground_skylight.z > d.ground_skylight.x, "a blue night");
    }

    #[test]
    fn the_moon_can_be_placed() {
        let low = Vec3::new(-0.9, 0.12, 0.3);
        let n = Noon { moon_dir: Some(low), ..noon() };
        let d = at(0.0, &n);
        assert!((d.light_dir - low.normalize()).length() < 1e-5, "the moon lights the night from where it was put");
        assert!((at(NOON, &n).light_dir - n.sun_dir.normalize()).length() < 1e-5, "and the sun is the sun");
    }

    #[test]
    fn the_evening_darkens_without_a_jump() {
        let n = noon();
        let mut last = at(15.0, &n);
        let mut h = 15.0;
        while h < 23.0 {
            h += 0.01;
            let d = at(h, &n);
            let seen = |d: &Daylight| d.light_intensity * d.light_color.length() + d.ground_skylight.length();
            assert!((seen(&d) - seen(&last)).abs() < 0.05, "no jump at {h}: {} to {}", seen(&last), seen(&d));
            assert!((d.horizon - last.horizon).length() < 0.05);
            last = d;
        }
    }

    #[test]
    fn a_cycle_starts_in_the_morning_and_hurries_through_the_night() {
        assert!((cycle_hours(0.0, 1200.0) - 7.0).abs() < 1e-3);
        let night = (0..1200).filter(|&s| {
            let h = cycle_hours(s as f32, 1200.0);
            !(5.0..20.0).contains(&h)
        });
        assert_eq!(night.count(), 300, "a quarter of the cycle is night");
        assert!((cycle_hours(1200.0, 1200.0) - 7.0).abs() < 1e-3, "and it comes round");
    }
}
