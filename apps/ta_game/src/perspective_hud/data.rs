//! What the cockpit's displays read: the unit's own state (its hull and armour, rank, kills,
//! speed, heading, attitude and where its turret points), the weather and light about it, and the
//! ground under it, gathered once a frame from the simulation and the view. Every display, on the
//! console or projected over the world, reads these and draws them its own way.

use std::f32::consts::TAU;

use ta_sim::{Fx, Locomotion};

use glam::Vec3;

use crate::effects::Weather;
use crate::game::Game;
use crate::perspective::Perspective;

/// A TA pixel taken as a quarter of a metre: a speed of one pixel a tick is 27 km/h.
pub const KMH_PER_SPEED: f32 = 27.0;

/// A compass bearing (degrees clockwise from north, which is -z on the map) of the way (`x`, `z`)
/// points over the ground.
pub fn bearing_of(x: f32, z: f32) -> f32 {
    x.atan2(-z).to_degrees().rem_euclid(360.0)
}

/// The compass bearing a unit's heading (angle units, 65,536 to the turn; 0 along +z) is.
pub fn heading_bearing(heading: i32) -> f32 {
    let a = heading as f32 * TAU / 65536.0;
    bearing_of(a.sin(), a.cos())
}

/// An angle between two bearings, from `from` to `to`, in -180..180 (to the right positive).
pub fn bearing_delta(from: f32, to: f32) -> f32 {
    (to - from + 540.0).rem_euclid(360.0) - 180.0
}

/// The clock's face for an hour of the day (0..24): "07:45".
pub fn clock_time(hours: f32) -> String {
    let minutes = (hours.rem_euclid(24.0) * 60.0).round() as i32 % 1440;
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// The light at an hour of the day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Daylight {
    Day,
    /// The sun on the horizon.
    Twilight,
    Night,
}

impl Daylight {
    pub fn at(hours: f32) -> Daylight {
        let h = hours.rem_euclid(24.0);
        if (7.0..17.5).contains(&h) {
            Daylight::Day
        } else if (5.5..7.0).contains(&h) || (17.5..19.5).contains(&h) {
            Daylight::Twilight
        } else {
            Daylight::Night
        }
    }
}

/// The weather and light about the unit.
#[derive(Clone, Copy, Debug)]
pub struct Environment {
    /// The hour of the day (none: TA's own light, which is that of a fixed noon).
    pub hours: Option<f32>,
    /// How hard the wind blows (what each wind generator makes of it) and the bearing it blows
    /// to.
    pub wind: f32,
    pub wind_to: f32,
    pub weather: Weather,
    /// How hard it rains (0..1) while the weather is a shower.
    pub rain: f32,
    /// The ground's height over the sea under the unit (world units; below the sea negative).
    pub elevation: f32,
}

/// What the displays read of one unit this frame.
#[derive(Clone, Debug)]
pub struct Readouts {
    pub name: String,
    pub kind: String,
    // The unit.
    /// Hull as a share of the whole (0..1) and in hit points.
    pub health: f32,
    pub hp: i32,
    pub hp_max: i32,
    /// 0 recruit to 3 elite; the kills scored from this seat; the veterancy that ranks come from
    /// (0..1), and where each rank begins on it.
    pub rank: u8,
    pub kills: u64,
    pub veterancy: f32,
    // Motion.
    /// Speed in km/h (negative astern), and the throttle that is of the fastest the unit goes
    /// (negative astern), smoothed.
    pub speed: f32,
    pub throttle: f32,
    /// Whether the unit moves at all (a gun house does not).
    pub mobile: bool,
    // Orientation (compass bearings and angles, degrees).
    /// The hull's heading, and the bearing the eye looks along; how far the eye looks off the
    /// heading (to the right positive) and how far the main turret points off it, if the unit
    /// has one that has aimed.
    pub heading: f32,
    pub look: f32,
    pub twist: f32,
    pub turret: Option<f32>,
    /// The hull's pitch (nose up positive) and roll (right side down positive).
    pub pitch: f32,
    pub roll: f32,
    /// Height over the ground (world units).
    pub altitude: f32,
    /// A walker's armour by section: how whole each is (see `perspective::Armour`).
    pub armour: [f32; 8],
    pub env: Environment,
}

/// Where each rank begins on the veterancy scale (see `Unit::rank`).
pub const RANK_AT: [f32; 3] = [0.25, 0.5, 2.0 / 3.0];

impl Readouts {
    pub fn of(p: &Perspective, game: &Game) -> Option<Readouts> {
        let u = game.sim.unit(p.unit)?;
        let ty = game.sim.unit_type(u);
        let max = ty.max_health.max(Fx::ONE);
        let health = (u.health / max).to_f32().clamp(0.0, 1.0);
        let world = &game.sim.world;
        let ground = world.height_at(u.pos.x, u.pos.z).max(world.sea_level);
        let wind_to = heading_bearing(game.sim.wind_heading);
        let to_deg = f32::to_degrees;
        let look = bearing_of(p.forward.x, p.forward.z);
        let heading = heading_bearing(u.heading);
        // (Whether it rains is the weather's own clock, as the glass of the cockpit sees it.)
        let now = crate::scene::game_seconds(game.sim.tick, game.alpha()) as f32;
        Some(Readouts {
            name: ty.def.display_name.clone(),
            kind: ty.def.description.clone(),
            health,
            hp: u.health.to_f32().round() as i32,
            hp_max: ty.max_health.to_f32().round() as i32,
            rank: u.rank(),
            kills: p.hud.kills.saturating_sub(p.hud.kills_before),
            veterancy: u.veterancy().to_f32(),
            speed: u.speed.to_f32() * KMH_PER_SPEED,
            throttle: p.hud.speed.x,
            mobile: ty.locomotion != Locomotion::Static,
            heading,
            look,
            twist: bearing_delta(heading, look),
            turret: p.gun_azimuth.map(to_deg),
            pitch: to_deg(p.attitude.0),
            roll: to_deg(p.attitude.1),
            altitude: p.hud.altitude.x,
            armour: p.hud.armour.integrity(),
            env: Environment {
                hours: p.hours,
                wind: game.sim.wind.to_f32(),
                wind_to,
                weather: game.effects.weather_kind(),
                rain: game.effects.rain(now),
                elevation: (ground - world.sea_level).to_f32(),
            },
        })
    }
}

/// Where the line of sight meets the ground (world units from the eye), if it does within
/// `reach`: what a laser range finder reads off.
pub fn ground_range(game: &Game, eye: Vec3, dir: Vec3, reach: f32) -> Option<f32> {
    let world = &game.sim.world;
    let sea = world.sea_level.to_f32();
    march(eye, dir, reach, |x, z| crate::scene::drawn_ground(world, x, z).max(sea))
}

/// How far along `dir` from `eye` the ground (its height at (x, z) given by `ground`) is met, within
/// `reach`: marched in steps of 8 units, then halved to the crossing.
fn march(eye: Vec3, dir: Vec3, reach: f32, ground: impl Fn(f32, f32) -> f32) -> Option<f32> {
    let below = |d: f32| {
        let q = eye + dir * d;
        q.y < ground(q.x, q.z)
    };
    let step = 8.0;
    let (mut near, mut d) = (0.0, step);
    while d <= reach {
        if below(d) {
            let (mut lo, mut hi) = (near, d);
            for _ in 0..8 {
                let mid = (lo + hi) * 0.5;
                if below(mid) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            return Some(hi);
        }
        near = d;
        d += step;
    }
    None
}

#[cfg(test)]
impl Readouts {
    /// A unit in the field, for the displays' tests.
    pub fn sample() -> Readouts {
        Readouts {
            name: "Bulldog".into(),
            kind: "Heavy assault tank".into(),
            health: 0.72,
            hp: 1512,
            hp_max: 2100,
            rank: 1,
            kills: 3,
            veterancy: 0.4,
            speed: 31.0,
            throttle: 0.7,
            mobile: true,
            heading: 93.0,
            look: 120.0,
            twist: 27.0,
            turret: Some(-14.0),
            pitch: 2.0,
            roll: -1.0,
            altitude: 120.0,
            armour: [1.0, 0.8, 0.5, 0.9, 1.0, 0.3, 1.0, 0.7],
            env: Environment { hours: Some(14.5), wind: 12.0, wind_to: 270.0, weather: Weather::Dust, rain: 0.0, elevation: 42.0 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearings_are_the_compasss() {
        // A unit at heading 0 faces +z, which is south; a quarter turn to its left (heading
        // 16384) faces +x, east. (North is read as just short of a full turn, or none.)
        let at = |heading: i32| heading_bearing(heading).round() % 360.0;
        assert_eq!(at(0), 180.0);
        assert_eq!(at(16384), 90.0);
        assert_eq!(at(32768), 0.0);
        assert_eq!(at(49152), 270.0);
        assert_eq!(bearing_of(0.0, -1.0).round() % 360.0, 0.0);
        assert_eq!(bearing_of(1.0, 0.0).round(), 90.0);
    }

    #[test]
    fn the_angle_between_bearings_goes_the_short_way() {
        assert_eq!(bearing_delta(350.0, 10.0), 20.0);
        assert_eq!(bearing_delta(10.0, 350.0), -20.0);
        assert_eq!(bearing_delta(90.0, 90.0), 0.0);
        assert!((bearing_delta(0.0, 180.0).abs() - 180.0).abs() < 1e-4);
    }

    #[test]
    fn the_range_finder_reads_where_the_line_of_sight_meets_the_ground() {
        let flat = |_x: f32, _z: f32| 0.0;
        // Level ground 100 units below, looking down at 45 degrees.
        let down = Vec3::new(0.0, -1.0, 1.0).normalize();
        let r = march(Vec3::new(0.0, 100.0, 0.0), down, 2600.0, flat).unwrap();
        assert!((r - 100.0 * 2f32.sqrt()).abs() < 0.2, "{r}");
        // Looking up, or along a level line, it never meets it; nor does ground beyond the reach.
        assert_eq!(march(Vec3::new(0.0, 100.0, 0.0), Vec3::new(0.0, 0.5, 1.0).normalize(), 2600.0, flat), None);
        assert_eq!(march(Vec3::new(0.0, 100.0, 0.0), down, 100.0, flat), None);
        // A hill across the way is met sooner than the plain behind it: a wall from z = 300.
        let hill = |_x: f32, z: f32| if z > 300.0 { 200.0 } else { 0.0 };
        let r = march(Vec3::new(0.0, 100.0, 0.0), Vec3::Z, 2600.0, hill).unwrap();
        assert!((r - 300.0).abs() < 0.2, "{r}");
    }

    #[test]
    fn the_clock_reads_hours_and_minutes() {
        assert_eq!(clock_time(0.0), "00:00");
        assert_eq!(clock_time(6.5), "06:30");
        assert_eq!(clock_time(17.75), "17:45");
        assert_eq!(clock_time(23.999), "00:00");
        assert_eq!(clock_time(-1.0), "23:00");
    }

    #[test]
    fn the_light_has_a_day_a_twilight_and_a_night() {
        assert_eq!(Daylight::at(12.0), Daylight::Day);
        assert_eq!(Daylight::at(6.4), Daylight::Twilight);
        assert_eq!(Daylight::at(18.5), Daylight::Twilight);
        assert_eq!(Daylight::at(0.0), Daylight::Night);
        assert_eq!(Daylight::at(23.0), Daylight::Night);
    }
}
