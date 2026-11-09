//! Converts simulation state into render instances.
//!
//! The simulation ticks at 30 Hz; rendering runs at the display rate. Unit root transforms and
//! their scripts' piece animations are interpolated between the previous and current tick.
//! On top of the simulation's pose the scene adds motion for the eye alone (see [`Motion`]):
//! vehicles squat and dive on their suspension and lean out of turns, hovercraft and aircraft
//! bank into them, gunships tip towards where they are going, ships roll on the swell and
//! submarines run at their depth; and every unit but a building rocks as its gun kicks it back
//! or a big blast near it shoves it away (see [`Jolt`], [`Scene::jolt`]). None of it feeds back
//! into the game.

use glam::{Mat4, Quat, Vec2, Vec3};
use rustc_hash::FxHashMap as HashMap;
use ta_render::model::{FLAG_BUILDING, FLAG_SELECTED};
use ta_render::{DrawInstance, ModelId, Renderer};
use ta_sim::{Locomotion, Sim, SimEvent, Unit, UnitId, UnitType};

mod feet;
mod footing;
mod recoil;
mod still;
mod walk;

pub use feet::{Foot, Pad, walks};
pub use still::standing_pose;

const ANGLE_TO_RAD: f32 = std::f32::consts::TAU / 65536.0;
/// Seconds per simulation tick.
const TICK: f32 = 1.0 / 30.0;

/// Seconds of game time `alpha` of the way through tick `tick`, in f64: exact in a game of any
/// length (an f32 of the tick count loses the fraction of a tick within the first hours).
pub fn game_seconds(tick: u64, alpha: f32) -> f64 {
    (tick as f64 + alpha as f64) / 30.0
}

#[derive(Clone, Copy)]
struct Pose {
    pos: Vec3,
    heading: f32,
    pitch: f32,
    roll: f32,
}

impl Pose {
    fn of(u: &Unit) -> Pose {
        Pose {
            pos: Vec3::from_array(u.pos.to_f32()),
            heading: u.heading as f32 * ANGLE_TO_RAD,
            pitch: u.pitch as f32 * ANGLE_TO_RAD,
            roll: u.roll as f32 * ANGLE_TO_RAD,
        }
    }

    fn lerp(a: Pose, b: Pose, t: f32) -> Pose {
        let ang = |x: f32, y: f32| x + wrap(y - x) * t;
        Pose {
            pos: a.pos.lerp(b.pos, t),
            heading: ang(a.heading, b.heading),
            pitch: ang(a.pitch, b.pitch),
            roll: ang(a.roll, b.roll),
        }
    }

    /// Standing on the ground (as the simulation has it): on the ground as it is drawn (see
    /// [`drawn_ground`]), and a vehicle tilted with it as the simulation tilts it with its own
    /// (`ta_sim::Sim::settle`: the slope under its whole footprint), so that over a building's
    /// pad and the bank round it, as anywhere the drawn ground is not the simulation's, it
    /// rides the ground it is drawn on. What stands level (upright, big walkers:
    /// `ta_sim::UnitType::level`) stays level.
    fn on_drawn_ground(mut self, ground: &Ground, t: &UnitType) -> Pose {
        let p = self.pos;
        let Some(below) = ground.under(p) else { return self };
        self.pos.y += ground.drawn(p.x, p.z) - below;
        if !t.level() && matches!(t.locomotion, Locomotion::Ground | Locomotion::Hover) {
            let (half_l, half_w) = (t.footprint.1.clamp(1, 8) as f32 * 8.0, t.footprint.0.clamp(1, 8) as f32 * 8.0);
            let (s, c) = self.heading.sin_cos();
            let (fwd, left) = (Vec2::new(s, c) * half_l, Vec2::new(c, -s) * half_w);
            let sea = ground.world.sea_level.to_f32();
            let h = |d: Vec2| {
                let g = ground.drawn(p.x + d.x, p.z + d.y);
                if t.locomotion == Locomotion::Hover { g.max(sea) } else { g }
            };
            self.pitch = -(h(fwd) - h(-fwd)).atan2(half_l * 2.0);
            self.roll = (h(left) - h(-left)).atan2(half_w * 2.0);
        }
        self
    }

    fn matrix(&self) -> Mat4 {
        Mat4::from_translation(self.pos)
            * Mat4::from_rotation_y(self.heading)
            * Mat4::from_rotation_x(self.pitch)
            * Mat4::from_rotation_z(self.roll)
    }
}

/// The steepest the ground tilts a vehicle (radians, about 29 degrees). The game tilts it by the
/// heights under its footprint: one straddling a cliff's edge would lie against the cliff's face
/// (60 degrees and more on The Pass), half into the rock.
const GROUND_TILT: f32 = 0.5;

/// The ground's height at world (`x`, `z`) as the terrain draws it: the simulation's heights
/// smoothed within a tolerance (`ta_render::terrain_surface`: a unit or so where units walk,
/// more on cliffs and spikes), levelled under buildings (`ta_render::terrain_pads`), drawn as
/// triangles split along the diagonal `ta_render::terrain::split_flipped` picks. (Without the
/// map's terrain loaded, the simulation's heights so split.)
pub fn drawn_ground(world: &ta_sim::world::World, x: f32, z: f32) -> f32 {
    if let Some(y) = ta_render::terrain::drawn_height(world.width, world.height, x, z) {
        return y;
    }
    simulated_ground(world, x, z)
}

/// The simulation's ground at world (`x`, `z`), drawn as the terrain draws its triangles.
fn simulated_ground(world: &ta_sim::world::World, x: f32, z: f32) -> f32 {
    let (fx, fz) = (x / 16.0, z / 16.0);
    let (x0, z0) = (fx.floor(), fz.floor());
    let (cx, cz) = (x0 as i32, z0 as i32);
    let v = |dx: i32, dz: i32| world.vertex_height(cx + dx, cz + dz).to_f32();
    ta_render::terrain::cell_height([v(0, 0), v(1, 0), v(0, 1), v(1, 1)], fx - x0, fz - z0)
}

/// How far up something at `p` that stands on the simulation's ground is drawn, so that it
/// stands on the drawn ground (see [`drawn_ground`]); 0 for what is off the ground (flying,
/// afloat) and wherever the two agree.
pub fn ground_offset(world: &ta_sim::world::World, p: Vec3) -> f32 {
    let ground = world.height_at(ta_sim::Fx::from_f32(p.x), ta_sim::Fx::from_f32(p.z)).to_f32();
    if (p.y - ground).abs() > 1.0 {
        return 0.0;
    }
    drawn_ground(world, p.x, p.z) - ground
}

/// The ground as it is drawn and as the simulation has it, for posing a battle's units all at
/// once: the drawn ground taken once (see [`drawn_ground`]) rather than asked for under its lock
/// five times for each of twenty thousand units, on every core.
pub(crate) struct Ground<'a> {
    world: &'a ta_sim::world::World,
    drawn: Option<std::sync::Arc<ta_render::terrain::HeightField>>,
}

impl<'a> Ground<'a> {
    pub(crate) fn of(world: &'a ta_sim::world::World) -> Ground<'a> {
        Ground { world, drawn: ta_render::terrain::drawn_field(world.width, world.height) }
    }

    /// As [`drawn_ground`].
    pub(crate) fn drawn(&self, x: f32, z: f32) -> f32 {
        match &self.drawn {
            Some(field) => field.sample(x, z),
            None => simulated_ground(self.world, x, z),
        }
    }

    /// Conservative broad phase for a fragment's bounding sphere. Every cell under the
    /// sphere is covered, including its boundary vertices; triangles cannot exceed those
    /// vertices. Large mod fragments skip this shortcut rather than scanning a huge area.
    pub(crate) fn clear_above(&self, pos: Vec3, radius: f32, sea_floor: f32) -> bool {
        let bottom = pos.y - radius;
        if radius > 64.0 || bottom <= sea_floor {
            return false;
        }
        let x0 = ((pos.x - radius) / 16.0).floor() as i32;
        let x1 = ((pos.x + radius) / 16.0).floor() as i32 + 1;
        let z0 = ((pos.z - radius) / 16.0).floor() as i32;
        let z1 = ((pos.z + radius) / 16.0).floor() as i32 + 1;
        for z in z0..=z1 {
            for x in x0..=x1 {
                let height = match &self.drawn {
                    Some(field) => field.vertex(x, z),
                    None => self.world.vertex_height(x, z).to_f32(),
                };
                if bottom <= height {
                    return false;
                }
            }
        }
        true
    }

    /// The simulation's ground under `p` if it stands on it (within a unit; not flying nor
    /// afloat).
    fn under(&self, p: Vec3) -> Option<f32> {
        let ground = self.world.height_at(ta_sim::Fx::from_f32(p.x), ta_sim::Fx::from_f32(p.z)).to_f32();
        ((p.y - ground).abs() <= 1.0).then_some(ground)
    }
}

/// Whether a unit is a building that levels the drawn ground under it (see [`ground_pad`]): one
/// standing on land (not floating on the water nor standing in it).
fn levels_ground(sim: &Sim, u: &Unit, t: &UnitType) -> bool {
    let sea = sim.world.sea_level;
    t.locomotion == Locomotion::Static
        && u.occupied_origin.is_some()
        && !t.def.floater
        && t.def.min_water_depth <= 0.0
        && (sea <= ta_sim::Fx::ZERO || u.pos.y >= sea + ta_sim::Fx::HALF)
}

/// The ground a building levels under it as it is drawn (`ta_render::GroundPad`): its footprint,
/// at its base as the simulation has it (on the highest ground under the footprint, see
/// `ta_sim::world::World::footprint_top`), so that it stands on the ground all round rather than
/// hanging over it downhill. None for what is not a building on land.
pub fn ground_pad(sim: &Sim, u: &Unit) -> Option<ta_render::GroundPad> {
    let t = sim.types.get(u.type_id);
    let (x0, z0) = u.occupied_origin.filter(|_| levels_ground(sim, u, t))?;
    let (fx, fz) = (t.footprint.0 as i32, t.footprint.1 as i32);
    Some(ta_render::GroundPad { cells: [x0, z0, x0 + fx, z0 + fz], height: u.pos.y.to_f32(), tick: sim.tick })
}

/// An angle difference brought into -π..π.
fn wrap(a: f32) -> f32 {
    let d = a % std::f32::consts::TAU;
    if d > std::f32::consts::PI {
        d - std::f32::consts::TAU
    } else if d < -std::f32::consts::PI {
        d + std::f32::consts::TAU
    } else {
        d
    }
}

/// A piece's state in the unit's script at one tick: offset, rotation, shown.
type PieceState = ([i32; 3], [i32; 3], bool);

/// How a unit carries itself, for the motion the scene adds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Body {
    /// Buildings, and walkers that stand level (their scripts do all their moving).
    Still,
    /// Tracked and wheeled vehicles: suspension.
    Vehicle,
    /// Hovercraft: floaty, banking into turns.
    Hover,
    /// Ships: the swell, heeling out of turns, bows up at speed.
    Ship,
    /// Submarines: below the surface at their waterline, steady.
    Submarine,
    /// Fighters and bombers: bank into turns, nose up to climb.
    Plane,
    /// Gunships, transports, air constructors: tip towards their motion, bob in the hover.
    Vtol,
}

impl Body {
    fn of(t: &UnitType) -> Body {
        match t.locomotion {
            Locomotion::Static => Body::Still,
            Locomotion::Ground if t.level() => Body::Still,
            Locomotion::Ground => Body::Vehicle,
            Locomotion::Hover => Body::Hover,
            Locomotion::Naval if t.def.water_line > 0.0 => Body::Submarine,
            Locomotion::Naval => Body::Ship,
            Locomotion::Air if t.is_plane() => Body::Plane,
            Locomotion::Air => Body::Vtol,
        }
    }
}

/// Jolts ([`Jolt`]): how fast a jolted body swings back (radians a second), how soon it
/// settles (its damping ratio: a few swings, each smaller), and the most it swings (radians).
const JOLT_OMEGA: f32 = 13.0;
const JOLT_ZETA: f32 = 0.3;
const JOLT_MOST: f32 = 0.09;
/// How hard a gun kicks back for the damage of its shot over how heavy the unit is (its health
/// stands for its weight: a Krogoth shrugs off what rocks a Stumpy), and the most one shot kicks
/// (radians a second); what of that each kind of weapon gives: shells all of it, plasma bolts
/// less, missiles a little, lasers and bombs none.
const RECOIL: f32 = 9.0;
const RECOIL_MOST: f32 = 0.9;
/// Blasts big enough to shove the units round them (their area of effect and damage), how
/// far they reach (times their area of effect), how hard they shove (for their damage over
/// the unit's weight, as a gun), and how many of a tick's biggest are felt.
const BLAST_AREA: f32 = 24.0;
const BLAST_DAMAGE: f32 = 60.0;
const BLAST_REACH: f32 = 2.5;
const BLAST: f32 = 1.2;
const BLASTS: usize = 24;

/// How heavy a unit is to a jolt: its health (at least a light unit's).
fn weight(t: &UnitType) -> f32 {
    t.max_health.to_f32().max(200.0)
}

/// What share of a gun's kick a weapon gives.
fn recoil_share(w: &ta_sim::WeaponType) -> f32 {
    kick_share(w.def.beam_weapon, w.def.render_type, w.trajectory)
}

/// What share of a gun's kick a weapon of this kind gives: none for a beam (a laser or lightning
/// as TA draws it, whether or not its data calls it a beam weapon) or a dropped bomb; the most
/// for a shell lobbed on an arc.
fn kick_share(beam: bool, drawn: ta_data::RenderType, trajectory: ta_sim::Trajectory) -> f32 {
    if beam || matches!(drawn, ta_data::RenderType::Laser | ta_data::RenderType::Lightning) {
        return 0.0;
    }
    match trajectory {
        ta_sim::Trajectory::Ballistic => 1.0,
        ta_sim::Trajectory::Straight => 0.45,
        ta_sim::Trajectory::Guided => 0.2,
        ta_sim::Trajectory::Dropped => 0.0,
    }
}

/// A push of strength `k` (radians a second) from the direction `towards` (world, along the
/// ground) on a unit facing `heading`: its pitch and roll kicks. The side towards it lifts as the
/// unit is pushed from it, the nose for a push from ahead (positive pitch dips the nose, positive
/// roll lifts the left side, the model's +x).
fn push(heading: f32, towards: Vec2, k: f32) -> (f32, f32) {
    let length = towards.length();
    if length < 1e-3 {
        return (0.0, 0.0);
    }
    let towards = towards / length;
    let (s, c) = heading.sin_cos();
    let (ahead, left) = (towards.dot(Vec2::new(s, c)), towards.dot(Vec2::new(c, -s)));
    (-ahead * k, left * k)
}

/// A body rocked by jolts (for the eye alone): a gun kicking it back, a blast shoving it. Its
/// pitch and roll swing back to rest on a lively spring, on top of whatever else moves it.
#[derive(Clone, Copy, Default)]
struct Jolt {
    pitch: Spring,
    roll: Spring,
    /// Pitch and roll at the tick before.
    before: (f32, f32),
}

impl Jolt {
    fn kick(&mut self, (pitch, roll): (f32, f32)) {
        self.pitch.v += pitch;
        self.roll.v += roll;
    }

    fn tick(&mut self) {
        self.before = (self.pitch.x, self.roll.x);
        self.pitch.step(0.0, JOLT_OMEGA, JOLT_ZETA);
        self.roll.step(0.0, JOLT_OMEGA, JOLT_ZETA);
        // (Kick on kick, as a rapid gun's, never tips it over.)
        self.pitch.x = self.pitch.x.clamp(-JOLT_MOST, JOLT_MOST);
        self.roll.x = self.roll.x.clamp(-JOLT_MOST, JOLT_MOST);
    }

    /// Its pitch and roll `alpha` of the way through the tick.
    fn at(&self, alpha: f32) -> (f32, f32) {
        (self.before.0 + (self.pitch.x - self.before.0) * alpha, self.before.1 + (self.roll.x - self.before.1) * alpha)
    }
}

/// A damped spring, stepped a tick at a time.
#[derive(Clone, Copy, Default)]
struct Spring {
    x: f32,
    v: f32,
}

impl Spring {
    /// Towards `target` at natural frequency `omega` (radians a second) and damping ratio
    /// `zeta`, over one tick (semi-implicit Euler in two halves).
    fn step(&mut self, target: f32, omega: f32, zeta: f32) {
        let h = TICK * 0.5;
        for _ in 0..2 {
            self.v += (omega * omega * (target - self.x) - 2.0 * zeta * omega * self.v) * h;
            self.x += self.v * h;
        }
    }
}

/// The motion the scene adds to a unit's pose: its pitch and roll as springs chasing what its
/// movement asks for (the terrain's tilt under it, eased, and the leans of speeding up, braking
/// and turning), a tick at a time; and the swell and hover bob, as they are at the moment drawn.
#[derive(Clone, Copy)]
struct Motion {
    body: Body,
    pitch: Spring,
    roll: Spring,
    /// Pitch and roll at the tick before (the springs hold the last tick's).
    before: (f32, f32),
    /// Velocity at the last tick (world units a tick).
    vel: Vec3,
    /// Its own phase for the swell and the bob.
    seed: f32,
    /// How much the swell moves it (small boats more).
    swell: f32,
    /// How far below the surface it rides: the waterline a submarine runs at; a ship's draft
    /// (TA's hulls end at the water plane, and seen floating on it they would sit on glass).
    depth: f32,
    /// How far aircraft bank (the unit's BankScale).
    bank: f32,
    /// In the air (not landed on the ground).
    flying: bool,
    /// Not yet tracking (new, or moved in a jump).
    fresh: bool,
    /// Whether jolts rock it (anything but a building), and how they do.
    rocks: bool,
    jolt: Jolt,
}

impl Motion {
    fn new(u: &Unit, t: &UnitType) -> Motion {
        let seed = (u.id.slot as f32 * 0.618_034).fract() * std::f32::consts::TAU;
        let body = Body::of(t);
        Motion {
            body,
            pitch: Spring::default(),
            roll: Spring::default(),
            before: (0.0, 0.0),
            vel: Vec3::ZERO,
            seed,
            swell: (20.0 / t.radius.to_f32().max(1.0)).clamp(0.3, 1.0),
            depth: match body {
                Body::Submarine => t.waterline.to_f32(),
                Body::Ship => (1.0 + t.radius.to_f32() * 0.035).min(3.0),
                _ => 0.0,
            },
            bank: t.def.bank_scale.clamp(0.0, 2.0) as f32,
            flying: false,
            fresh: true,
            rocks: t.locomotion != Locomotion::Static,
            jolt: Jolt::default(),
        }
    }

    /// Follows the unit over the tick from `prev` to `cur` (`jump`: they are not a tick apart).
    fn tick(&mut self, prev: Pose, cur: Pose, u: &Unit, jump: bool) {
        if jump {
            self.jolt = Jolt::default();
        } else if self.rocks {
            self.jolt.tick();
        }
        if self.body == Body::Still {
            return;
        }
        let vel = cur.pos - prev.pos;
        // Carried, just built, placed or moved in a jump (a replay seek, a rescue from a stuck
        // spot): start again from the pose as it is.
        if jump || u.carried_by.is_some() || !u.is_complete() || vel.length_squared() > 40.0 * 40.0 {
            self.fresh = true;
        }
        let acc = if self.fresh { Vec3::ZERO } else { vel - self.vel };
        self.vel = vel;
        self.flying = !u.landed && self.body.is_air();
        let (s, c) = cur.heading.sin_cos();
        let (fwd, left) = (Vec3::new(s, 0.0, c), Vec3::new(c, 0.0, -s));
        let (v_f, v_l) = (vel.dot(fwd), vel.dot(left));
        let (a_f, a_l) = (acc.dot(fwd), acc.dot(left));
        // Sideways acceleration of something going where it points (positive to its left, the
        // model's +x), from how fast it turns: steadier than differencing positions.
        let yaw = if self.fresh { 0.0 } else { wrap(cur.heading - prev.heading) };
        let turn = yaw * v_f;
        // (Positive pitch dips the nose; positive roll lifts the left side.) The ground tilts a
        // vehicle at most `GROUND_TILT` (see there).
        let ground = |a: f32| a.clamp(-GROUND_TILT, GROUND_TILT);
        let (pitch, roll, omega, zeta) = match self.body {
            Body::Still => unreachable!(),
            // Squats as it speeds up, dives as it brakes, leans out of turns; the ground's tilt
            // taken up on the springs.
            Body::Vehicle => (ground(cur.pitch) - (a_f * 1.8).clamp(-0.05, 0.05), ground(cur.roll) + (turn * 0.45).clamp(-0.05, 0.05), 12.0, 0.45),
            // On its cushion: noses up a little as it speeds up, banks gently into turns, and
            // settles without swinging (the small corrections of its steering must not set it
            // rocking).
            Body::Hover => (ground(cur.pitch) - (a_f * 1.0).clamp(-0.04, 0.04), ground(cur.roll) - (turn * 0.9).clamp(-0.07, 0.07), 6.0, 0.9),
            // Bow up at speed, heeling out of turns.
            Body::Ship => (-(v_f * 0.012).clamp(-0.05, 0.05), (turn * 0.3).clamp(-0.06, 0.06), 3.0, 0.4),
            Body::Submarine => (0.0, 0.0, 3.0, 0.8),
            // Banks into turns (whatever roll the game gives it), noses up to climb.
            Body::Plane => {
                let climb = vel.y.atan2(Vec2::new(vel.x, vel.z).length().max(1.0));
                (-(climb * 0.8).clamp(-0.35, 0.35), -(turn * 0.75 * self.bank).clamp(-0.9, 0.9), 5.0, 0.9)
            }
            // Tips towards where it is going and how it is speeding up.
            Body::Vtol if self.flying => ((v_f * 0.018 + a_f * 1.0).clamp(-0.25, 0.25), -((v_l * 0.018 + a_l * 1.0) * self.bank).clamp(-0.25, 0.25), 4.5, 0.55),
            Body::Vtol => (ground(cur.pitch), ground(cur.roll), 6.0, 0.8),
        };
        if self.fresh {
            self.pitch = Spring { x: pitch, v: 0.0 };
            self.roll = Spring { x: roll, v: 0.0 };
            self.before = (pitch, roll);
            self.fresh = false;
            return;
        }
        self.before = (self.pitch.x, self.roll.x);
        self.pitch.step(pitch, omega, zeta);
        self.roll.step(roll, omega, zeta);
    }

    /// Applies the motion to a pose drawn `alpha` of the way through the tick, `time` seconds
    /// into the game (for the swell and the bob), and its jolts over that.
    fn apply(&self, pose: &mut Pose, alpha: f32, time: f64) {
        self.apply_body(pose, alpha, time);
        if self.rocks {
            let (pitch, roll) = self.jolt.at(alpha);
            pose.pitch += pitch;
            pose.roll += roll;
        }
    }

    fn apply_body(&self, pose: &mut Pose, alpha: f32, time: f64) {
        if self.body == Body::Still || self.fresh {
            return;
        }
        pose.pitch = self.before.0 + (self.pitch.x - self.before.0) * alpha;
        pose.roll = self.before.1 + (self.roll.x - self.before.1) * alpha;
        // (The part of a period taken in f64: as smooth after days as in the first minute.)
        let wave = |period: f32, phase: f32| ((time / period as f64).fract() as f32 * std::f32::consts::TAU + phase).sin();
        let s = self.seed;
        match self.body {
            Body::Ship => {
                // One swell across the sea: neighbours rock alike, a fleet not quite in step.
                let phase = pose.pos.x * 0.004 + pose.pos.z * 0.003 + s * 0.35;
                let k = self.swell;
                pose.roll += 0.035 * k * wave(4.1, phase);
                pose.pitch += 0.02 * k * wave(5.3, phase * 1.3 + 1.0);
                pose.pos.y += 0.8 * k * wave(4.6, phase * 0.9 + 2.0) - self.depth;
            }
            Body::Submarine => pose.pos.y -= self.depth,
            Body::Hover => {
                pose.pos.y += 0.5 * wave(2.3, s);
                pose.pitch += 0.006 * wave(3.1, s * 1.7);
                pose.roll += 0.008 * wave(2.7, s * 2.3);
            }
            Body::Vtol if self.flying => {
                pose.pos.y += 1.2 * wave(3.2, s);
                pose.pitch += 0.012 * wave(3.9, s * 1.3);
                pose.roll += 0.018 * wave(3.4, s * 2.1);
            }
            _ => {}
        }
    }
}

impl Body {
    fn is_air(self) -> bool {
        matches!(self, Body::Plane | Body::Vtol)
    }
}

/// Client-side render state per unit.
struct Visual {
    /// The unit it is of (its slot may go to another unit once it is gone).
    id: UnitId,
    prev: Pose,
    cur: Pose,
    matrices: Vec<Mat4>,
    model: Option<ModelId>,
    motion: Motion,
    /// The script's piece states at the tick before `anim_tick` and at it, for animation
    /// between ticks (kept for units drawn: read when drawn).
    anim_prev: Vec<PieceState>,
    anim_cur: Vec<PieceState>,
    anim_tick: Option<u64>,
    /// Its player's colour.
    team: u32,
    /// The tick `matrices` were last posed at.
    drawn: Option<u64>,
    /// Its health at the last tick, and the last hit it took (tick, how hard 0..1): it
    /// flashes as it is hit.
    health: ta_sim::Fx,
    hit: (u64, f32),
    /// How far it has driven and turned all told, at the tick before and this one (for its
    /// treads).
    travel: [[f32; 2]; 2],
    /// A walker's legs in step with the ground (see [`walk`]).
    walk: walk::Walker,
    /// How far its lights are on (1) or out (0), at the tick before and this one: on while it
    /// works (see [`lights_target`]), coming on and going out over [`LIGHTS_TICKS`].
    lights: [f32; 2],
    /// Its guns' kicks as drawn (see [`recoil`]).
    recoil: recoil::Recoil,
}

/// Ticks a unit's lights take to come on or go out, all the way (a third of a second): a
/// building short of power, paid one tick and not the next, glows at half strength rather
/// than flickering.
const LIGHTS_TICKS: f32 = 10.0;

/// Whether a unit's lights are on: while it works (`Sim::working`: finished, switched on or
/// not switchable, its upkeep paid, neither stunned nor carried). A metal maker switched off or
/// short of energy, a radar tower unpowered, anything an EMP stuns goes dark.
fn lights_target(sim: &Sim, u: &Unit) -> f32 {
    if sim.working(u) { 1.0 } else { 0.0 }
}

/// `from` moved towards `to` by `step` at most.
fn step_towards(from: f32, to: f32, step: f32) -> f32 {
    if from < to { (from + step).min(to) } else { (from - step).max(to) }
}

/// How far lights at `lights` (the tick before and this one, 1 on) are out, `alpha` of the way
/// between the ticks: 0 lit to 255 dark (the flags' bits 24-31).
fn lights_out(lights: [f32; 2], alpha: f32) -> u32 {
    let on = lights[0] + (lights[1] - lights[0]) * alpha.clamp(0.0, 1.0);
    ((1.0 - on.clamp(0.0, 1.0)) * 255.0).round() as u32
}

/// A model piece's extent in its own space, in world units: the faces it draws (not its
/// selection plate), or none when it draws nothing.
#[derive(Clone, Copy, Debug)]
pub struct PieceBounds {
    pub min: Vec3,
    pub max: Vec3,
}

/// Actual visible vertices of a piece, shared by every fragment of that model. A bounding
/// box's empty corners are not a contact surface: tumbling a slanted piece onto such a corner
/// would leave the mesh floating. Selection plates and unused vertices do not touch ground.
#[derive(Debug)]
pub(crate) struct PieceSurface {
    pub(crate) points: Box<[Vec3]>,
    pub(crate) radius: f32,
}

impl PieceSurface {
    fn new(points: Vec<Vec3>) -> Option<std::sync::Arc<Self>> {
        let radius = points.iter().map(|p| p.length_squared()).fold(0.0, f32::max).sqrt();
        (!points.is_empty()).then(|| std::sync::Arc::new(Self { points: points.into_boxed_slice(), radius }))
    }

    pub(crate) fn of_render(library: &ta_render::model::ModelLibrary, model: ModelId) -> Vec<Option<std::sync::Arc<Self>>> {
        // Use the stable base mesh with its repaired forms/caps. Reborn's optional detail
        // adds decorative plate/bolt offsets; a change of its screen-size LOD must not move
        // a settled fragment or alter its contact geometry.
        let mut pieces = vec![Vec::new(); library.info(model).pieces.len()];
        let mut seen = vec![std::collections::HashSet::new(); pieces.len()];
        for (piece, point) in library.corners(model) {
            let i = piece as usize;
            if i < pieces.len() && seen[i].insert(point.to_array().map(f32::to_bits)) {
                pieces[i].push(point);
            }
        }
        pieces.into_iter().map(Self::new).collect()
    }

    #[cfg(test)]
    pub(crate) fn of(m: &ta_formats::model3do::Model3do) -> Vec<Option<std::sync::Arc<Self>>> {
        m.pieces.iter().map(|p| {
            let mut used = vec![false; p.vertices.len()];
            for (i, prim) in p.primitives.iter().enumerate() {
                if Some(i) == p.selection_primitive || prim.indices.len() < 3 {
                    continue;
                }
                for &v in &prim.indices {
                    if let Some(used) = used.get_mut(v as usize) {
                        *used = true;
                    }
                }
            }
            let vertices: Vec<Vec3> = p.vertices.iter().zip(used).filter(|(_, used)| *used).map(|(v, _)| {
                Vec3::new(v[0] as f32, v[1] as f32, -(v[2] as f32)) / ta_formats::model3do::FIXED_ONE
            }).collect();
            Self::new(vertices)
        }).collect()
    }
}

impl PieceBounds {
    /// Every piece's bounds (model space has z negated, as the renderer loads it).
    pub(crate) fn of(m: &ta_formats::model3do::Model3do) -> Vec<Option<PieceBounds>> {
        let one = ta_formats::model3do::FIXED_ONE;
        m.pieces
            .iter()
            .map(|p| {
                let mut b: Option<PieceBounds> = None;
                for (k, prim) in p.primitives.iter().enumerate() {
                    if Some(k) == p.selection_primitive || prim.indices.len() < 3 {
                        continue;
                    }
                    for &i in &prim.indices {
                        let Some(v) = p.vertices.get(i as usize) else { continue };
                        let v = Vec3::new(v[0] as f32 / one, v[1] as f32 / one, -(v[2] as f32) / one);
                        b = Some(match b {
                            Some(b) => PieceBounds { min: b.min.min(v), max: b.max.max(v) },
                            None => PieceBounds { min: v, max: v },
                        });
                    }
                }
                b
            })
            .collect()
    }
}

/// What the scene last showed of a unit gone this tick (see [`Scene::take_departed`]).
pub struct Remnant {
    pub model: ModelId,
    /// World matrices of its pieces as last drawn (hidden pieces zero).
    pub pieces: Vec<Mat4>,
    /// Its velocity at the end (world units a second).
    pub velocity: Vec3,
    pub team: u32,
    /// In the air (not landed).
    pub flying: bool,
}

#[derive(Default)]
pub struct Scene {
    /// Per unit slot of the simulation, the unit there and how it is drawn (twenty thousand
    /// units: looked up by place, not hashed, and gone through on all cores).
    visuals: Vec<Option<Visual>>,
    /// Unit type id → render model id.
    models: HashMap<usize, Option<ModelId>>,
    enemy_models: HashMap<usize, Option<ModelId>>,
    /// Cosmetic reference side: the local player, watched replay side, or side zero.
    pub skin_viewer: u8,
    /// The tick of the last [`Scene::after_tick`].
    last_tick: Option<u64>,
    /// Units gone at the last tick, as they were last drawn (for what is left of them).
    departed: rustc_hash::FxHashMap<UnitId, Remnant>,
    /// Unit type id → its model's pieces' bounds.
    bounds: HashMap<usize, std::sync::Arc<Vec<Option<PieceBounds>>>>,
    /// Ground unit type id → the feet its model stands on (see [`feet`]).
    feet: HashMap<usize, std::sync::Arc<Vec<Foot>>>,
    /// Big walkers the ground no longer tilts (without `Upright=1`) → their legs, which bring
    /// their feet to the ground (see [`footing`]).
    footing: HashMap<usize, std::sync::Arc<Vec<footing::Leg>>>,
    /// Loaded once per unit type, shared by its thrown debris instead of copied per fragment.
    surfaces: HashMap<usize, Vec<Option<std::sync::Arc<PieceSurface>>>>,
    /// Perspective: the unit seen from inside and the eye; its pieces the eye is in, or within
    /// the margin of, are not drawn.
    pub hide_near: Option<(UnitId, Vec3, f32)>,
    /// Walkers' gaits, read from their scripts (see [`walk`]).
    walk: walk::Gaits,
    /// Unit type id → its guns that recoil, and its pieces' faces (see [`recoil`]).
    guns: HashMap<usize, std::sync::Arc<recoil::Guns>>,
    /// The camera of the frame being drawn: walkers too small on it keep their scripts' walk.
    pub camera: Option<ta_render::Camera>,
    /// Keep damage text/bars and worn paint, without the brief bright hit tint.
    pub reduced_flashes: bool,
    /// Staged scenes: frames drawn as far built as the scene has them rather than as far as the
    /// game has them (at the tick before and at this one, drawn between; see `stage.rs`).
    pub build_shown: HashMap<UnitId, (f32, f32)>,
    /// The ground levelled under the buildings put down since the renderer last took them (see
    /// [`ground_pad`], [`Scene::take_pads`]).
    pads: Vec<ta_render::GroundPad>,
}

/// Builds the model texture library from `textures/*.gaf`: the textures the models paint with
/// (`painted_textures`), as a mod's GAFs often hold thousands its models never name.
pub fn init_models(vfs: &ta_vfs::Vfs, renderer: &mut Renderer, palette: &ta_formats::image::Palette, skin: Option<&ta_skins::Pack>, enemy_skin: Option<&ta_skins::Pack>) {
    use rayon::prelude::*;
    // (In their order, on all cores: the first file holding a name gives it.)
    let gafs: Vec<ta_formats::gaf::Gaf> = vfs
        .list("textures", Some("gaf"), false)
        .par_iter()
        .filter_map(|p| vfs.read_opt(p).and_then(|d| ta_formats::gaf::Gaf::parse(&d).ok()))
        .collect();
    let used = painted_textures(vfs);
    renderer.init_models_with_skins(&gafs, palette, skin, enemy_skin, Some(&used));
}

/// Every texture a model of the game's data paints with (lower case): what every model in
/// `objects3d` names (units', wrecks', features' and projectiles' alike: any of them may be
/// drawn), read on all cores (a few tens of milliseconds, against about a second's work for each
/// thousand textures loaded). TA: Enhanced's models name 1,923 of the 4,663 its GAFs hold.
pub fn painted_textures(vfs: &ta_vfs::Vfs) -> std::collections::HashSet<String> {
    use rayon::prelude::*;
    let started = std::time::Instant::now();
    let models = vfs.list("objects3d", Some("3do"), true);
    let used: std::collections::HashSet<String> = models
        .par_iter()
        .filter_map(|p| vfs.read_opt(p).and_then(|b| ta_formats::model3do::Model3do::parse(&b).ok()))
        .flat_map_iter(|m| ta_render::model::drawn_textures(&m).collect::<Vec<_>>())
        .collect();
    log::info!("model textures: {} named by {} models, read in {:.0} ms", used.len(), models.len(), started.elapsed().as_secs_f64() * 1000.0);
    used
}

impl Scene {
    pub fn model_for(&self, type_id: usize) -> Option<ModelId> {
        self.model_for_side(type_id, false)
    }

    pub fn model_for_side(&self, type_id: usize, enemy: bool) -> Option<ModelId> {
        let models = if enemy { &self.enemy_models } else { &self.models };
        models.get(&type_id).copied().flatten()
    }

    /// The source model for a thrown piece, including a unit removed from the simulation
    /// this tick. Its last visible paint survives into debris and whole-body remains.
    pub fn model_for_unit(&self, sim: &Sim, unit: UnitId, type_id: usize, viewer: u8) -> Option<ModelId> {
        if let Some(u) = sim.unit(unit) {
            let models = if sim.is_enemy(u.owner, viewer) { &self.enemy_models } else { &self.models };
            return models.get(&type_id).copied().flatten();
        }
        self.visual(unit).and_then(|v| v.model)
            .or_else(|| self.departed.get(&unit).map(|remains| remains.model))
    }

    /// Registers the model for every unit type up front so the GPU buffers are built once.
    pub fn preload_models(&mut self, sim: &Sim, renderer: &mut Renderer, load: &dyn Fn(&str) -> Option<Vec<u8>>) {
        let Some(models) = renderer.models.as_mut() else { return };
        let palette = &sim.data.palette;
        for t in &sim.types.types {
            let object = t.def.object.to_ascii_lowercase();
            let source = load(&format!("objects3d/{object}.3do"));
            let mut parsed = source.as_ref().and_then(|b| ta_formats::model3do::Model3do::parse(b).ok());
            // A building's pads (an air plant's runway) clear of the ground it stands on.
            if t.locomotion == ta_sim::Locomotion::Static
                && let Some(m) = parsed.as_mut()
            {
                ta_render::model::lift_ground_pads(m);
            }
            if let Some(m) = &parsed {
                self.bounds.insert(t.id, std::sync::Arc::new(PieceBounds::of(m)));
                self.walk.add_model(t, m);
                if t.locomotion == Locomotion::Ground {
                    // (The walkers' feet, for their prints: what their scripts hide once they
                    // are built, a second body for an upgrade, a gun until it fires, is none.)
                    let feet = if feet::walks(t) { feet::measure(m, &feet::hidden_when_built(t)) } else { Vec::new() };
                    if t.big_walker && !t.def.upright {
                        let legs = footing::legs(m);
                        log::debug!(
                            "{}: stands level, {} feet brought to the ground ({:?})",
                            t.name,
                            legs.len(),
                            legs.iter().filter_map(|l| m.pieces.get(l.foot()).map(|p| p.name.as_str())).collect::<Vec<_>>()
                        );
                        if !legs.is_empty() {
                            self.footing.insert(t.id, std::sync::Arc::new(legs));
                        }
                    }
                    if !feet.is_empty() {
                        self.feet.insert(t.id, std::sync::Arc::new(feet));
                    }
                }
            }
            let ids = parsed.map(|m| {
                let own = models.library.add_unit(&t.def.name, &object, &m, &models.textures, palette);
                let enemy = models.library.add_unit_for_side(&t.def.name, &object, &m, &models.textures, palette, true);
                // Cosmetic variants share the repaired source geometry used for ground contact.
                self.surfaces.insert(t.id, PieceSurface::of_render(&models.library, own));
                // (And for the room its guns have to recoil in: the faces as drawn.)
                if let Some(guns) = recoil::Guns::of(t, models.library.corners(own)) {
                    self.guns.insert(t.id, std::sync::Arc::new(guns));
                }
                models.library.register_unit(&object, own);
                (own, enemy)
            });
            self.models.insert(t.id, ids.map(|ids| ids.0));
            self.enemy_models.insert(t.id, ids.map(|ids| ids.1));
        }
    }

    /// A unit type's model pieces' bounds (none before the models are loaded).
    pub fn piece_bounds(&self, type_id: usize) -> Option<std::sync::Arc<Vec<Option<PieceBounds>>>> {
        self.bounds.get(&type_id).cloned()
    }

    /// The feet a walking unit type's model stands on (`feet::walks`), in the order they come
    /// down (none before the models are loaded, or for a model with none near the ground).
    pub fn feet(&self, type_id: usize) -> Option<std::sync::Arc<Vec<Foot>>> {
        self.feet.get(&type_id).cloned()
    }

    pub(crate) fn piece_surface(&self, type_id: usize, piece: usize) -> Option<std::sync::Arc<PieceSurface>> {
        self.surfaces.get(&type_id)?.get(piece)?.clone()
    }

    /// A unit's visual, if it has one.
    fn visual(&self, id: UnitId) -> Option<&Visual> {
        self.visuals.get(id.slot as usize)?.as_ref().filter(|v| v.id == id)
    }

    fn visual_mut(&mut self, id: UnitId) -> Option<&mut Visual> {
        self.visuals.get_mut(id.slot as usize)?.as_mut().filter(|v| v.id == id)
    }

    /// Poses one unit as it is drawn this frame, `alpha` of the way through the tick: its
    /// pieces' world matrices into `out` (hidden pieces zero), returning its body's matrix.
    /// (Perspective puts the eye on a piece, so it must be where the piece is drawn.)
    pub fn pose(&mut self, sim: &Sim, id: UnitId, alpha: f32, out: &mut Vec<Mat4>) -> Option<Mat4> {
        let u = sim.unit(id)?;
        let time = game_seconds(sim.tick, alpha);
        let moment = walk::Moment::new(&self.walk, sim, alpha, time, self.camera.as_ref());
        let legs = self.footing.get(&u.type_id).cloned();
        let guns = self.guns.get(&u.type_id).cloned();
        let v = self.visuals.get_mut(id.slot as usize)?.as_mut().filter(|v| v.id == id)?;
        let t = sim.types.get(u.type_id);
        let mut pose = Pose::lerp(v.prev, v.cur, alpha);
        v.motion.apply(&mut pose, alpha, time);
        refresh_anim(v, u, t, sim.tick);
        let root = pose.matrix();
        let [travel, turned] = [0, 1].map(|k| v.travel[0][k] + (v.travel[1][k] - v.travel[0][k]) * alpha);
        let (before, now) = v.walk.states(&moment, u, t, pose.pos, travel, turned, &v.anim_prev, &v.anim_cur);
        let (before, now) = v.recoil.states(guns.as_deref(), moment.close(pose.pos, t.radius.to_f32()), before, now);
        piece_matrices_between(t, before, now, alpha, root, out);
        plant_feet(legs.as_ref().map(|l| l.as_slice()), u, &pose, &Ground::of(&sim.world), out);
        Some(root)
    }

    /// Call after every simulation tick: records poses for interpolation. (Called twice at
    /// the same tick, as after a load or a seek, it settles every unit where it stands.)
    pub fn after_tick(&mut self, sim: &Sim) {
        use rayon::prelude::*;
        let jump = self.last_tick.is_none_or(|t| t + 1 != sim.tick);
        self.last_tick = Some(sim.tick);
        let slots = sim.unit_slots();
        if self.visuals.len() < slots.len() {
            self.visuals.resize_with(slots.len(), || None);
        }
        // Units gone (their slot empty, or another's now): what was last drawn of them.
        self.departed.clear();
        for (slot, visual) in self.visuals.iter_mut().enumerate() {
            let here = slots.get(slot).and_then(|u| u.as_ref()).map(|u| u.id);
            if visual.as_ref().is_none_or(|v| Some(v.id) == here) {
                continue;
            }
            let Some(v) = visual.take() else { continue };
            // (Only what was on screen a moment ago: its pieces are posed as drawn.)
            if let (Some(model), Some(at)) = (v.model, v.drawn)
                && at + 3 >= sim.tick
                && !v.matrices.is_empty()
            {
                let velocity = if v.motion.body == Body::Still { Vec3::ZERO } else { v.motion.vel / TICK };
                self.departed.insert(v.id, Remnant { model, pieces: v.matrices, velocity, team: v.team, flying: v.motion.flying });
            }
        }
        // Every unit's pose, motion and hits, each its own (on all cores for a big battle); the
        // buildings new to it, which level the ground under them.
        let models = &self.models;
        let enemy_models = &self.enemy_models;
        let skin_viewer = self.skin_viewer;
        let tick = sim.tick;
        let ground = Ground::of(&sim.world);
        let follow = |(slot, visual): (usize, &mut Option<Visual>)| -> Option<usize> {
            let u = slots.get(slot).and_then(|u| u.as_ref())?;
            let t = sim.types.get(u.type_id);
            // (A building on land stands on its pad: the drawn ground levelled at its base.)
            let padded = levels_ground(sim, u, t);
            let pose = if padded { Pose::of(u) } else { Pose::of(u).on_drawn_ground(&ground, t) };
            let new = visual.is_none();
            let lit = lights_target(sim, u);
            let v = visual.get_or_insert_with(|| Visual {
                id: u.id,
                prev: pose,
                cur: pose,
                matrices: Vec::new(),
                model: models.get(&u.type_id).copied().flatten(),
                motion: Motion::new(u, t),
                anim_prev: Vec::new(),
                anim_cur: Vec::new(),
                anim_tick: None,
                team: 0,
                drawn: None,
                health: u.health,
                hit: (0, 0.0),
                travel: [[0.0; 2]; 2],
                walk: walk::Walker::default(),
                lights: [lit; 2],
                recoil: recoil::Recoil::default(),
            });
            // Its lights follow its work, a step a tick; after a load or a seek, as they are.
            v.lights = if jump || new { [lit; 2] } else { [v.lights[1], step_towards(v.lights[1], lit, 1.0 / LIGHTS_TICKS)] };
            // A restored snapshot has no hits from the discarded timeline. Keeping a future
            // hit tick makes saturating_sub below hold its flash at full strength after a
            // benchmark preset change or replay rewind.
            if jump {
                v.hit = (0, 0.0);
            } else if u.is_complete() && u.health < v.health {
                // A flash as bright as the blow was hard (building up doesn't count).
                let lost = ((v.health - u.health) / t.max_health.max(ta_sim::Fx::ONE)).to_f32();
                v.hit = (tick, (lost * 10.0).clamp(0.35, 1.0));
            }
            v.health = u.health;
            v.team = sim.players.get(u.owner as usize).map_or(0, |p| p.color as u32);
            let variants = if sim.is_enemy(u.owner, skin_viewer) { enemy_models } else { models };
            v.model = variants.get(&u.type_id).copied().flatten();
            v.prev = if jump { pose } else { v.cur };
            v.cur = pose;
            v.motion.tick(v.prev, v.cur, u, jump);
            // Driven: along where it faces; turned: its heading's change.
            v.travel[0] = v.travel[1];
            let (s, c) = v.cur.heading.sin_cos();
            v.travel[1][0] += (v.cur.pos - v.prev.pos).dot(Vec3::new(s, 0.0, c));
            v.travel[1][1] += wrap(v.cur.heading - v.prev.heading);
            (new && padded).then_some(slot)
        };
        let built: Vec<usize> = if slots.len() >= PARALLEL_POSES * 4 {
            self.visuals.par_iter_mut().enumerate().with_min_len(256).filter_map(follow).collect()
        } else {
            self.visuals.iter_mut().enumerate().filter_map(follow).collect()
        };
        self.pads.extend(built.into_iter().filter_map(|slot| ground_pad(sim, slots.get(slot)?.as_ref()?)));
    }

    /// Jolts from a tick's events (after [`Scene::after_tick`]; for the eye alone): a gun kicks
    /// its unit back from where it fired, as hard as its shot is heavy for the unit's size, and a
    /// big blast shoves the units round it (those drawn lately) away from it, harder the nearer.
    /// Aircraft in flight ride both out.
    pub fn jolt(&mut self, sim: &Sim, events: &[SimEvent]) {
        let mut blasts: Vec<(Vec2, f32, f32)> = Vec::new();
        for event in events {
            match event {
                SimEvent::WeaponFired { unit, weapon, pos, target } => {
                    let w = sim.weapon_types.get(*weapon);
                    let share = recoil_share(w);
                    let Some(u) = sim.unit(*unit).filter(|_| share > 0.0) else { continue };
                    let weight = weight(sim.types.get(u.type_id));
                    let Some(v) = self.visual_mut(*unit).filter(|v| v.motion.rocks && !v.motion.flying) else { continue };
                    let (from, to) = (pos.to_f32(), target.to_f32());
                    let k = (w.default_damage.to_f32() / weight * RECOIL * share).min(RECOIL_MOST);
                    v.motion.jolt.kick(push(v.cur.heading, Vec2::new(to[0] - from[0], to[2] - from[2]), k));
                }
                SimEvent::Explosion { weapon, pos, .. } => {
                    let w = sim.weapon_types.get(*weapon);
                    let (area, damage) = (w.aoe_radius.to_f32(), w.default_damage.to_f32());
                    if area >= BLAST_AREA && damage >= BLAST_DAMAGE {
                        let at = pos.to_f32();
                        blasts.push((Vec2::new(at[0], at[2]), area * BLAST_REACH, damage));
                    }
                }
                _ => {}
            }
        }
        if blasts.is_empty() {
            return;
        }
        blasts.sort_by(|a, b| b.2.total_cmp(&a.2));
        blasts.truncate(BLASTS);
        let (tick, slots) = (sim.tick, sim.unit_slots());
        // (Each unit's own: on all cores, a big battle's every unit in view near its blasts.)
        use rayon::prelude::*;
        self.visuals.par_iter_mut().enumerate().with_min_len(256).for_each(|(slot, visual)| {
            let Some(v) = visual.as_mut().filter(|v| v.motion.rocks && !v.motion.flying && v.drawn.is_some_and(|at| at + 3 >= tick)) else { return };
            let Some(u) = slots.get(slot).and_then(|u| u.as_ref()).filter(|u| u.id == v.id) else { return };
            let weight = weight(sim.types.get(u.type_id));
            let here = Vec2::new(v.cur.pos.x, v.cur.pos.z);
            for &(centre, reach, damage) in &blasts {
                let towards = centre - here;
                let near = 1.0 - towards.length() / reach;
                if near > 0.0 {
                    let k = (damage / weight * BLAST * near * near).min(RECOIL_MOST * 2.0);
                    v.motion.jolt.kick(push(v.cur.heading, towards, k));
                }
            }
        });
    }

    /// The ground levelled under the buildings put down since this was last asked, for the
    /// renderer (`Renderer::add_ground_pads`).
    pub fn take_pads(&mut self) -> Vec<ta_render::GroundPad> {
        std::mem::take(&mut self.pads)
    }

    /// The game went back (see `Game::restore`) to `sim`, the copy of it at its tick: what was
    /// put down after it is not, and every building standing then lays its pad again (with that
    /// tick; laid once, see `Renderer::add_ground_pads`). Those seen first after a jump (a
    /// game caught up, fast-forwarded) were laid with the tick they were seen at, which the
    /// renderer takes up with those laid after it.
    pub fn forget_pads_after(&mut self, sim: &Sim) {
        self.pads.retain(|p| p.tick <= sim.tick);
        self.pads.extend(sim.units().filter_map(|u| ground_pad(sim, u)));
    }

    /// What the scene last showed of a unit that died at the last tick, if it was in view.
    pub fn take_departed(&mut self, id: UnitId) -> Option<Remnant> {
        self.departed.remove(&id)
    }

    /// Interpolated world position of a unit (for selection, UI anchors).
    pub fn unit_position(&self, id: UnitId, alpha: f32) -> Option<Vec3> {
        self.visual(id).map(|v| Pose::lerp(v.prev, v.cur, alpha).pos)
    }

    /// How far a unit's lights are on, 0 (out) to 1, `alpha` of the way to the next tick: on
    /// while it works (its lamps' glow, its lights by night). 1 for a unit not followed yet.
    pub fn lights_on(&self, id: UnitId, alpha: f32) -> f32 {
        self.visual(id).map_or(1.0, |v| 1.0 - lights_out(v.lights, alpha) as f32 / 255.0)
    }

    /// Builds this frame's draw list: units the viewer can see whose position falls inside
    /// `bounds` (the ground rectangle in view, with a margin for shadows), the view's footprint
    /// on everything up to `top` high, widened for flying units above that. (Widened by every
    /// unit's whole height, it took in a band of units out of sight round the view, 300 px wide
    /// on high ground, each posed and drawn.)
    pub fn instances<'a>(
        &'a mut self,
        sim: &'a Sim,
        alpha: f32,
        selected: &dyn Fn(UnitId) -> bool,
        visible: &(dyn Fn(&Unit) -> bool + Sync),
        bounds: (Vec2, Vec2),
        top: f32,
    ) -> Vec<DrawInstance<'a>> {
        use rayon::prelude::*;
        let (lo, hi) = bounds;
        let in_view = |p: Vec3, r: f32| {
            let m = r + (p.y - top).max(0.0) * 1.5;
            p.x >= lo.x - m && p.x <= hi.x + m && p.z >= lo.y - m && p.z <= hi.y + m
        };
        let time = game_seconds(sim.tick, alpha);
        let tick = sim.tick;
        let slots = sim.unit_slots();
        let moment = walk::Moment::new(&self.walk, sim, alpha, time, self.camera.as_ref());
        let models = &self.models;
        let enemy_models = &self.enemy_models;
        let skin_viewer = self.skin_viewer;
        let footing = &self.footing;
        let guns = &self.guns;
        let ground = Ground::of(&sim.world);
        // Who is drawn, posed: each unit its own work, on all cores when a big battle is in
        // view (the visual of each slot is only its unit's).
        let draw = |(slot, visual): (usize, &mut Option<Visual>)| -> Option<usize> {
            let u = slots.get(slot)?.as_ref()?;
            let v = visual.as_mut().filter(|v| v.id == u.id)?;
            // A spectator can change sides while paused; alliances and captured ownership
            // also change which cosmetic choice applies without reloading any mesh.
            let variants = if sim.is_enemy(u.owner, skin_viewer) { enemy_models } else { models };
            v.model = variants.get(&u.type_id).copied().flatten();
            v.model?;
            if u.carried_by.is_some() && u.carry_piece.is_none() {
                return None;
            }
            let t = sim.types.get(u.type_id);
            let mut pose = Pose::lerp(v.prev, v.cur, alpha);
            if !in_view(pose.pos, t.radius.to_f32()) || !visible(u) {
                return None;
            }
            v.motion.apply(&mut pose, alpha, time);
            refresh_anim(v, u, t, tick);
            let root = pose.matrix();
            // (Walkers step with the ground they cover.)
            let [travel, turned] = [0, 1].map(|k| v.travel[0][k] + (v.travel[1][k] - v.travel[0][k]) * alpha);
            let (before, now) = v.walk.states(&moment, u, t, pose.pos, travel, turned, &v.anim_prev, &v.anim_cur);
            // (Guns kick back as far as they run clear.)
            let (before, now) = v.recoil.states(guns.get(&u.type_id).map(|g| &**g), moment.close(pose.pos, t.radius.to_f32()), before, now);
            piece_matrices_between(t, before, now, alpha, root, &mut v.matrices);
            plant_feet(footing.get(&u.type_id).map(|l| l.as_slice()), u, &pose, &ground, &mut v.matrices);
            v.drawn = Some(tick);
            Some(slot)
        };
        let drawn: Vec<usize> = if slots.len() >= PARALLEL_POSES {
            self.visuals.par_iter_mut().enumerate().with_min_len(64).filter_map(draw).collect()
        } else {
            self.visuals.iter_mut().enumerate().filter_map(draw).collect()
        };
        // Seen from inside: the pieces round the eye go.
        if let Some((id, eye, margin)) = self.hide_near
            && let Some(bounds) = sim.unit(id).and_then(|u| self.bounds.get(&u.type_id)).cloned()
            && let Some(v) = self.visual_mut(id).filter(|v| v.drawn == Some(tick))
        {
            for (m, b) in v.matrices.iter_mut().zip(bounds.iter()) {
                let Some(b) = b else { continue };
                if *m == Mat4::ZERO || m.determinant().abs() < 1e-9 {
                    continue;
                }
                let local = m.inverse().transform_point3(eye);
                if local.cmpge(b.min - margin).all() && local.cmple(b.max + margin).all() {
                    *m = Mat4::ZERO;
                }
            }
        }
        let mut out = Vec::with_capacity(drawn.len());
        for slot in drawn {
            let (Some(Some(u)), Some(Some(v))) = (slots.get(slot), self.visuals.get(slot)) else { continue };
            let Some(model) = v.model else { continue };
            let mut flags = 0;
            if selected(u.id) {
                flags |= FLAG_SELECTED;
            }
            if !u.is_complete() {
                flags |= FLAG_BUILDING;
            }
            if u.cloaked {
                flags |= ta_render::model::FLAG_CLOAKED;
            }
            if v.motion.body == Body::Submarine {
                flags |= ta_render::model::FLAG_SUBMERGED;
            }
            if u.paralyzed_until > sim.tick {
                flags |= ta_render::model::FLAG_STUNNED;
            }
            // The flash of a hit, fading over a fifth of a second (bits 8-15).
            let since = ((sim.tick.saturating_sub(v.hit.0)) as f32 + alpha) * TICK;
            let flash = v.hit.1 * (1.0 - since / HIT_FLASH).max(0.0);
            if flash > 0.0 && !self.reduced_flashes {
                flags |= ((flash * 255.0) as u32).min(255) << ta_render::model::FLAG_HIT_SHIFT;
            }
            let color = sim.players.get(u.owner as usize).map_or(0, |p| p.color as u32);
            // Battle damage shows on finished units (a frame's health follows its progress).
            let t = sim.types.get(u.type_id);
            let mut damage = if u.is_complete() && !t.max_health.is_zero() { (1.0 - (u.health / t.max_health).to_f32()).clamp(0.0, 1.0) } else { 0.0 };
            // Captures: `TA_TEST_DAMAGE=<0..1>` shows every unit that damaged.
            if let Some(d) = test_damage() {
                damage = d;
            }
            let travel = [0, 1].map(|k| v.travel[0][k] + (v.travel[1][k] - v.travel[0][k]) * alpha);
            // How fast its treads run (world units a second; turning, its sides' too): marks on
            // them passing too fast to follow blur away.
            let run = ((v.travel[1][0] - v.travel[0][0]).abs() + (v.travel[1][1] - v.travel[0][1]).abs() * t.radius.to_f32()) / TICK;
            flags |= (run.round() as u32).min(255) << ta_render::model::FLAG_RUN_SHIFT;
            // Its lights out as far as it has stopped working (bits 24-31).
            flags |= lights_out(v.lights, alpha) << ta_render::model::FLAG_LIGHTS_OUT_SHIFT;
            let build = self.build_shown.get(&u.id).map_or(u.build_progress.to_f32(), |&(before, now)| before + (now - before) * alpha);
            // Its animated textures play where its script lets them (TA's `dont-cache`).
            let playing = u.textures_playing(t);
            out.push(DrawInstance { model, pieces: &v.matrices, team: color, flags, build, damage, piece: None, travel, playing });
        }
        out
    }
}

/// A big walker's feet brought to the ground as it is drawn (see [`footing`]): its pieces' world
/// matrices `out`, posed level at `pose` (its middle on the drawn ground), its `legs`. Only a
/// finished walker standing on the ground (not one carried, nor a frame being built).
fn plant_feet(legs: Option<&[footing::Leg]>, u: &Unit, pose: &Pose, ground: &Ground, out: &mut [Mat4]) {
    let Some(legs) = legs else { return };
    if !u.is_complete() || u.carried_by.is_some() || u.dead || ground.under(Vec3::from_array(u.pos.to_f32())).is_none() {
        return;
    }
    let forward = Vec3::new(pose.heading.sin(), 0.0, pose.heading.cos());
    footing::plant(legs, out, pose.pos.y, forward, &|x, z| ground.drawn(x, z));
}

/// Brings a unit's record of its script's piece states up to `tick`: the states at the tick
/// before and at this one, for animation between them.
fn refresh_anim(v: &mut Visual, u: &Unit, t: &UnitType, tick: u64) {
    if v.anim_tick == Some(tick) {
        return;
    }
    let next = v.anim_tick.is_some_and(|k| k + 1 == tick);
    std::mem::swap(&mut v.anim_prev, &mut v.anim_cur);
    v.anim_cur.clear();
    v.anim_cur.extend((0..t.model.pieces.len()).map(|i| u.piece_anim(t, i)));
    if !next || v.anim_prev.len() != v.anim_cur.len() {
        v.anim_prev.clone_from(&v.anim_cur);
    }
    v.anim_tick = Some(tick);
}

/// `TA_TEST_DAMAGE`, read once.
fn test_damage() -> Option<f32> {
    static DAMAGE: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *DAMAGE.get_or_init(|| std::env::var("TA_TEST_DAMAGE").ok().and_then(|v| v.parse().ok()))
}

/// Seconds a unit's hit flash lasts.
const HIT_FLASH: f32 = 0.2;

/// Units in view from which their poses are computed on all cores.
const PARALLEL_POSES: usize = 256;

/// World matrices for every model piece of a unit as it stands now, on the ground as it is drawn:
/// what a structure the viewer sees is remembered as (see `fog.rs`).
pub fn standing_matrices(sim: &Sim, u: &Unit, out: &mut Vec<Mat4>) {
    let mut pose = Pose::of(u);
    pose.pos.y += ground_offset(&sim.world, pose.pos);
    piece_matrices(u, sim.unit_type(u), pose.matrix(), out);
}

/// World matrices for every model piece of a unit as its script has it now.
pub fn piece_matrices(u: &Unit, t: &UnitType, root: Mat4, out: &mut Vec<Mat4>) {
    let states: Vec<PieceState> = (0..t.model.pieces.len()).map(|i| u.piece_anim(t, i)).collect();
    piece_matrices_between(t, &states, &states, 1.0, root, out);
}

/// A piece's turns as its script has them (TA's angle units about x, y and z) as a rotation in
/// its parent's frame: Ry · Rx · Rz, z turns negated (see FORMATS.md).
fn local_rotation(turns: [f32; 3]) -> Quat {
    Quat::from_rotation_y(turns[1] * ANGLE_TO_RAD) * Quat::from_rotation_x(turns[0] * ANGLE_TO_RAD) * Quat::from_rotation_z(-turns[2] * ANGLE_TO_RAD)
}

/// World matrices for every model piece, `alpha` of the way from the script's states `before`
/// to `now` (turns the short way round): T(offset + move) · Ry · Rx · Rz per piece, the same
/// composition the deterministic simulation uses for piece positions. (Script x-axis moves
/// and z-axis turns are negated: see FORMATS.md.)
fn piece_matrices_between(t: &UnitType, before: &[PieceState], now: &[PieceState], alpha: f32, root: Mat4, out: &mut Vec<Mat4>) {
    out.clear();
    let shift = |a: i32, b: i32| a as f32 + b.wrapping_sub(a) as f32 * alpha;
    let turn = |a: i32, b: i32| a as f32 + (b.wrapping_sub(a) as i16) as f32 * alpha;
    // Hidden pieces still carry their children, so they are blanked only at the end.
    let mut hidden: u128 = 0;
    let mut hidden_more: Vec<usize> = Vec::new();
    for (i, p) in t.model.pieces.iter().enumerate() {
        let ((off0, rot0, _), (off, rot, visible)) = (before.get(i).copied().unwrap_or(([0; 3], [0; 3], true)), now.get(i).copied().unwrap_or(([0; 3], [0; 3], true)));
        let offset = Vec3::new(
            p.offset.x.to_f32() - shift(off0[0], off[0]) / 65536.0,
            p.offset.y.to_f32() + shift(off0[1], off[1]) / 65536.0,
            p.offset.z.to_f32() + shift(off0[2], off[2]) / 65536.0,
        );
        let rotation = local_rotation([0, 1, 2].map(|k| turn(rot0[k], rot[k])));
        let local = Mat4::from_rotation_translation(rotation, offset);
        let m = match p.parent {
            Some(parent) => out[parent] * local,
            None => root * local,
        };
        out.push(m);
        if !visible {
            if i < 128 {
                hidden |= 1 << i;
            } else {
                hidden_more.push(i);
            }
        }
    }
    while hidden != 0 {
        let i = hidden.trailing_zeros() as usize;
        out[i] = Mat4::ZERO;
        hidden &= hidden - 1;
    }
    for i in hidden_more {
        out[i] = Mat4::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debris_ground_broad_phase_covers_cell_edges_and_drawn_pads() {
        let map = ta_formats::tnt::TntMap { width: 8, height: 8, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![10; 64], features: vec![ta_formats::tnt::FEATURE_NONE; 64], feature_names: Vec::new(), minimap: Default::default() };
        let world = ta_sim::world::World::new(&map);
        let ground = Ground { world: &world, drawn: None };
        let pos = Vec3::new(31.0, 30.0, 31.0);
        assert!(ground.clear_above(pos, 2.0, -30.0));
        assert!(!ground.clear_above(pos, 2.0, 29.0), "water floor is still a contact plane");
        assert!(!ground.clear_above(pos, 65.0, -30.0), "large mod geometry skips the bounded shortcut");
        let mut field = ta_render::terrain::HeightField { width: 8, height: 8, heights: vec![10; 64], sea_level: 0.0, display: vec![10.0; 64] };
        field.display[3 * 8 + 3] = 100.0; // The far vertex of a cell touched by the sphere.
        let ground = Ground { world: &world, drawn: Some(std::sync::Arc::new(field)) };
        assert!(!ground.clear_above(pos, 2.0, -30.0), "a raised pad at the edge must reach the exact contact test");
        assert!(ground.clear_above(Vec3::new(31.0, 103.0, 31.0), 2.0, -30.0));
    }

    /// Lasers and lightning give no kick, beam weapons or not; shells and missiles do.
    #[test]
    fn beams_do_not_kick() {
        use ta_data::RenderType;
        use ta_sim::Trajectory;
        assert_eq!(kick_share(false, RenderType::Laser, Trajectory::Straight), 0.0);
        assert_eq!(kick_share(false, RenderType::Lightning, Trajectory::Straight), 0.0);
        assert_eq!(kick_share(true, RenderType::Shell, Trajectory::Straight), 0.0);
        assert_eq!(kick_share(false, RenderType::Shell, Trajectory::Straight), 0.45);
        assert_eq!(kick_share(false, RenderType::Shell, Trajectory::Ballistic), 1.0);
        assert_eq!(kick_share(false, RenderType::Model, Trajectory::Guided), 0.2);
        assert_eq!(kick_share(false, RenderType::Bomb, Trajectory::Dropped), 0.0);
    }

    #[test]
    fn springs_settle_on_their_target() {
        let mut s = Spring::default();
        for _ in 0..90 {
            s.step(0.3, 12.0, 0.45);
        }
        assert!((s.x - 0.3).abs() < 1e-3, "{}", s.x);
    }

    /// A jolt swings the body (back and forth, each swing smaller) and settles in under a
    /// second; kick on kick never tips it past the most it swings.
    #[test]
    fn a_jolt_swings_and_settles_and_never_tips_it_over() {
        let mut jolt = Jolt::default();
        jolt.kick((-0.6, 0.0));
        let mut swings = Vec::new();
        for _ in 0..30 {
            jolt.tick();
            swings.push(jolt.pitch.x);
        }
        assert!(swings.iter().copied().fold(0.0f32, f32::min) < -0.02, "the nose comes up: {swings:?}");
        assert!(swings.iter().any(|&p| p > 0.0), "and swings back past rest");
        assert!(jolt.pitch.x.abs() < 0.004 && jolt.pitch.v.abs() < 0.08, "settled in a second: {} {}", jolt.pitch.x, jolt.pitch.v);
        let (before, after) = (jolt.before.0, jolt.pitch.x);
        assert_eq!(jolt.at(0.0).0, before);
        assert!((jolt.at(1.0).0 - after).abs() < 1e-6, "drawn between its ticks");
        let mut gun = Jolt::default();
        for _ in 0..200 {
            gun.kick((-RECOIL_MOST, RECOIL_MOST));
            gun.tick();
            assert!(gun.pitch.x.abs() <= JOLT_MOST && gun.roll.x.abs() <= JOLT_MOST);
        }
    }

    /// How hard units' guns kick them, and how far that swings them (degrees), from the game's
    /// own data (`TA_UNITS=a,b` for others), for tuning [`RECOIL`] (on this computer only).
    #[test]
    #[ignore = "requires the locally installed original game"]
    fn recoil_table() {
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let data = std::sync::Arc::new(ta_data::GameData::load(&vfs));
        let types = ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p));
        let weapons = ta_sim::WeaponTypes::compile(&data);
        let names = std::env::var("TA_UNITS").unwrap_or_else(|_| "ARMPW,CORAK,ARMSTUMP,CORRAID,ARMBULL,CORGOL,ARMMART,CORMART,ARMZEUS,CORCAN,ARMFARK,CORKROG,ARMCOM,ARMROY,CORSHIP".into());
        for name in names.split(',') {
            let Some(t) = types.id(name).map(|id| types.get(id)) else { continue };
            let weight = weight(t);
            for &w in t.weapons.iter().flatten() {
                let w = weapons.get(w);
                let k = (w.default_damage.to_f32() / weight * RECOIL * recoil_share(w)).min(RECOIL_MOST);
                let mut jolt = Jolt::default();
                jolt.kick((-k, 0.0));
                let peak = (0..30).map(|_| { jolt.tick(); -jolt.pitch.x }).fold(0.0f32, f32::max);
                eprintln!("{name:9} weight {weight:6.0}  {:18} damage {:6.0} every {:3} ticks: kick {k:.2}, {:.2}°", w.name, w.default_damage.to_f32(), w.reload_ticks, peak.to_degrees());
            }
        }
    }

    /// Pushed from ahead the nose lifts, from its left the left side, whichever way it faces.
    #[test]
    fn a_push_lifts_the_side_it_comes_from() {
        for heading in [0.0f32, 1.0, 2.5, -2.0] {
            let (s, c) = heading.sin_cos();
            let (ahead, left) = (Vec2::new(s, c), Vec2::new(c, -s));
            let (pitch, roll) = push(heading, ahead * 30.0, 0.5);
            assert!((pitch + 0.5).abs() < 1e-5 && roll.abs() < 1e-5, "{heading}: ahead {pitch} {roll}");
            let (pitch, roll) = push(heading, left * 3.0, 0.5);
            assert!(pitch.abs() < 1e-5 && (roll - 0.5).abs() < 1e-5, "{heading}: left {pitch} {roll}");
            let (pitch, roll) = push(heading, -ahead - left, 0.5);
            assert!(pitch > 0.3 && roll < -0.3, "{heading}: from behind and the right, the tail and the right side lift");
        }
        assert_eq!(push(0.0, Vec2::ZERO, 1.0), (0.0, 0.0), "right on it: no way to be pushed");
    }

    /// Game data from TDF files (path, text), loaded as a mod's folder is.
    fn data_of(files: &[(&str, &str)]) -> ta_data::GameData {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("ta-scene-pads-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        for (path, text) in files {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_mod_dir(&dir, "test").unwrap();
        let data = ta_data::GameData::load(&vfs);
        std::fs::remove_dir_all(&dir).ok();
        data
    }

    /// Quality comparisons restore a snapshot from before the battle. A later hit must not
    /// tint its restored unit until the old clock catches up; real damage must still flash.
    #[test]
    fn benchmark_rewind_discards_future_hit_flashes() {
        use ta_sim::{Fx, FxVec3};
        let data = std::sync::Arc::new(data_of(&[("units/test.fbi", "[UNITINFO]{UnitName=TEST;MaxDamage=1000;BuildTime=100;FootprintX=1;FootprintZ=1;}")]));
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        let map = ta_formats::tnt::TntMap { width: 32, height: 32, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![10; 1024], features: vec![ta_formats::tnt::FEATURE_NONE; 1024], feature_names: Vec::new(), minimap: Default::default() };
        let players = [ta_sim::PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }];
        let mut sim = Sim::new(types, data, &map, &ta_data::MapInfo::default(), 0, &players, 7);
        let kind = sim.types.id("TEST").unwrap();
        let id = sim.spawn_unit(kind, 0, FxVec3::from_ints(200, 0, 200), 0, true).unwrap();
        sim.tick = 480;
        let saved = sim.clone();
        let mut scene = Scene::default();
        scene.models.insert(kind, Some(0));
        scene.after_tick(&sim);
        let appearance = |scene: &mut Scene, sim: &Sim| {
            let drawn = scene.instances(sim, 0.0, &|_| false, &|_| true, (Vec2::ZERO, Vec2::splat(512.0)), 0.0);
            assert_eq!(drawn.len(), 1);
            ((drawn[0].flags >> ta_render::model::FLAG_HIT_SHIFT) & 255, drawn[0].damage)
        };
        sim.tick = 1199;
        scene.after_tick(&sim);
        sim.tick += 1;
        scene.after_tick(&sim);
        scene.visual_mut(id).unwrap().hit = (sim.tick, 1.0);
        assert!(appearance(&mut scene, &sim).0 > 0, "ordinary damage flashes");
        let before = appearance(&mut scene, &sim);
        scene.reduced_flashes = true;
        assert_eq!(appearance(&mut scene, &sim), (0, before.1), "the preference removes the flash without losing damage wear");
        scene.reduced_flashes = false;
        assert_eq!(appearance(&mut scene, &sim), before, "toggling back keeps the original hit timing");
        sim = saved;
        scene.after_tick(&sim);
        scene.after_tick(&sim); // Game::restore settles interpolation at the snapshot.
        assert_eq!(appearance(&mut scene, &sim), (0, 0.0), "no damage from the discarded future");
        for _ in 0..195 { // The complete settings pass, well before the discarded hit.
            sim.tick += 1;
            scene.after_tick(&sim);
            assert_eq!(appearance(&mut scene, &sim).0, 0);
        }
        sim.tick += 1;
        scene.after_tick(&sim);
        scene.visual_mut(id).unwrap().hit = (sim.tick, 1.0);
        assert!(appearance(&mut scene, &sim).0 > 0, "new hits still flash");
        for _ in 0..7 {
            sim.tick += 1;
            scene.after_tick(&sim);
        }
        assert_eq!(appearance(&mut scene, &sim).0, 0, "the new flash fades normally");
    }

    #[test]
    fn side_skins_follow_allies_capture_spectators_and_dying_units() {
        use ta_sim::{Fx, FxVec3};
        let data = std::sync::Arc::new(data_of(&[("units/test.fbi", "[UNITINFO]{UnitName=TEST;MaxDamage=1000;BuildTime=100;FootprintX=1;FootprintZ=1;}")]));
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        let map = ta_formats::tnt::TntMap { width: 32, height: 32, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![10; 1024], features: vec![ta_formats::tnt::FEATURE_NONE; 1024], feature_names: Vec::new(), minimap: Default::default() };
        let players: Vec<_> = (0..3).map(|color| ta_sim::PlayerSetup { side: "ARM".into(), color, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }).collect();
        let mut sim = Sim::new(types, data, &map, &ta_data::MapInfo::default(), 0, &players, 7);
        sim.set_team(2, 0);
        let kind = sim.types.id("TEST").unwrap();
        let ids: Vec<_> = (0..3).map(|owner| sim.spawn_unit(kind, owner, FxVec3::from_ints(100 + i64::from(owner) * 100, 0, 200), 0, true).unwrap()).collect();
        let mut scene = Scene::default();
        scene.models.insert(kind, Some(11));
        scene.enemy_models.insert(kind, Some(22));
        scene.after_tick(&sim);
        let appearance = |scene: &mut Scene, sim: &Sim| {
            scene.instances(sim, 0.0, &|_| false, &|_| true, (Vec2::ZERO, Vec2::splat(512.0)), 0.0).iter().map(|d| (d.team, d.model)).collect::<Vec<_>>()
        };
        let checksum = sim.checksum();
        assert_eq!(appearance(&mut scene, &sim), [(0, 11), (1, 22), (2, 11)], "side zero and its ally use own paint in benchmarks/omniscient views");
        scene.skin_viewer = 1;
        assert_eq!(appearance(&mut scene, &sim), [(0, 22), (1, 11), (2, 22)], "a spectator switches perspective even while paused");
        assert_eq!(sim.checksum(), checksum, "cosmetic rendering cannot change simulation state");
        scene.skin_viewer = 0;
        sim.set_team(2, 2);
        assert_eq!(appearance(&mut scene, &sim), [(0, 11), (1, 22), (2, 22)], "changed alliances update immediately");
        sim.transfer_unit(ids[1], 0);
        assert_eq!(appearance(&mut scene, &sim), [(0, 11), (0, 11), (2, 22)], "a captured unit adopts its new owner's cosmetic choice");
        sim.transfer_unit(ids[1], 1);
        appearance(&mut scene, &sim);
        assert_eq!(scene.model_for_unit(&sim, ids[1], kind, 0), Some(22));
        // The fixture has no real mesh, so supply a last drawn piece for the remnant path.
        scene.visual_mut(ids[1]).unwrap().matrices = vec![Mat4::IDENTITY];
        scene.visual_mut(ids[0]).unwrap().matrices = vec![Mat4::IDENTITY];
        sim.remove_unit(ids[1]);
        sim.remove_unit(ids[0]);
        sim.step(&[]);
        scene.after_tick(&sim);
        assert!(sim.unit(ids[1]).is_none());
        assert_eq!(scene.model_for_unit(&sim, ids[1], kind, 0), Some(22), "thrown pieces retain the departed enemy's paint");
        assert_eq!(scene.model_for_unit(&sim, ids[0], kind, 0), Some(11), "simultaneous losses retain their own side's paint");
        assert_eq!(scene.take_departed(ids[1]).unwrap().model, 22, "whole-body remains retain the same paint");
        assert_eq!(scene.take_departed(ids[0]).unwrap().model, 11, "remnants can be consumed in event order, independently of unit slots");
    }

    /// Buildings on land level the drawn ground under them, at their base: a pad over the
    /// footprint at the height the simulation stands them (the highest ground under it, on a
    /// slope), which is where they are drawn; laid once, as they are put down (a frame yet).
    /// What floats on the sea or stands in it lays none.
    #[test]
    fn buildings_on_land_lay_pads_at_their_base() {
        use ta_sim::{Fx, FxVec3};
        let building = |name: &str, keys: &str| (format!("units/{}.fbi", name.to_ascii_lowercase()), format!("[UNITINFO]\n{{\nUnitName={name};\nObjectname={name};\nMaxDamage=1000;\nBuildTime=100;\nFootprintX=4;\nFootprintZ=3;\n{keys}\n}}\n"));
        let files = [building("PLANT", ""), building("RAFT", "Floater=1;"), building("TIDAL", "MinWaterDepth=10;")];
        let data = std::sync::Arc::new(data_of(&files.iter().map(|(p, t)| (p.as_str(), t.as_str())).collect::<Vec<_>>()));
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        // 64 x 64 cells rising 3 a cell westwards, out of a sea at 40 in the east (from x 54).
        let cells = 64 * 64;
        let heights = (0..cells).map(|i| (200 - 3 * (i % 64) as i32) as u8).collect();
        let map = ta_formats::tnt::TntMap { width: 64, height: 64, sea_level: 40, tiles: Vec::new(), tile_map: Vec::new(), heights, features: vec![ta_formats::tnt::FEATURE_NONE; cells], feature_names: Vec::new(), minimap: Default::default() };
        let players = [ta_sim::PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }];
        let mut sim = Sim::new(types, data, &map, &ta_data::MapInfo::default(), 0, &players, 7);
        let id = |sim: &Sim, name: &str| sim.types.id(name).unwrap();
        let plant = sim.spawn_unit(id(&sim, "PLANT"), 0, FxVec3::from_ints(400, 0, 400), 0, false).unwrap();
        let wet = [
            sim.spawn_unit(id(&sim, "PLANT"), 0, FxVec3::from_ints(900, 0, 200), 0, true).unwrap(),
            sim.spawn_unit(id(&sim, "RAFT"), 0, FxVec3::from_ints(960, 0, 400), 0, true).unwrap(),
            sim.spawn_unit(id(&sim, "TIDAL"), 0, FxVec3::from_ints(960, 0, 700), 0, true).unwrap(),
        ];
        let mut scene = Scene::default();
        scene.after_tick(&sim);
        let pads = scene.take_pads();
        assert_eq!(pads.len(), 1, "one pad, the frame's on land: {pads:?}");
        let u = sim.unit(plant).unwrap();
        let (x0, z0) = u.occupied_origin.unwrap();
        assert_eq!(pads[0].cells, [x0, z0, x0 + 4, z0 + 3]);
        assert_eq!(pads[0].height, (200 - 3 * x0) as f32, "at the footprint's west, uphill, edge");
        assert_eq!(pads[0].height, u.pos.y.to_f32());
        assert_eq!(scene.unit_position(plant, 0.5).unwrap().y, pads[0].height, "drawn on it");
        for w in wet {
            assert!(ground_pad(&sim, sim.unit(w).unwrap()).is_none());
        }
        scene.after_tick(&sim);
        assert!(scene.take_pads().is_empty(), "laid once");
    }

    #[test]
    fn turns_interpolate_the_short_way_round() {
        let turn = |a: i32, b: i32, alpha: f32| a as f32 + (b.wrapping_sub(a) as i16) as f32 * alpha;
        // From just short of a full turn to just past it: forward through zero.
        assert_eq!(turn(65000, 536, 0.5), 65536.0);
        assert_eq!(turn(-100, 100, 0.25), -50.0);
    }

    /// The pieces are drawn where the simulation has them (fire points, nano sprays): both
    /// apply the scripts' moves and turns about every axis with the same signs (x moves and
    /// z turns negated, docs/FORMATS.md). Skipped without a TA install.
    #[test]
    fn pieces_are_drawn_where_the_simulation_has_them() {
        use ta_sim::FxVec3;
        use ta_sim::cob::CobInstance;
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        let mut vfs = ta_vfs::Vfs::new();
        if vfs.mount_game_dir(&install.root, "base").is_err() {
            return;
        }
        let data = ta_data::GameData::load(&vfs);
        let types = ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p));
        for name in ["ARMAVP", "CORACK", "ARMSOLAR", "CORVP", "ARMCOM"] {
            let Some(id) = types.id(name) else { continue };
            let t = types.get(id);
            let Some(program) = t.script.clone() else { continue };
            let mut u = Unit::new(UnitId { slot: 0, generation: 0 }, id, 0, FxVec3::from_ints(1000, 50, 700));
            (u.heading, u.pitch, u.roll) = (9000, 700, -400);
            // Every piece moved and turned on every axis, by different amounts.
            let mut cob = CobInstance::new(program);
            for (k, p) in cob.pieces.iter_mut().enumerate() {
                let k = k as i32 + 1;
                p.offset = [(k * 37 % 11 - 5) << 16, (k * 13 % 7 - 3) << 16, (k * 29 % 9 - 4) << 16];
                p.rotation = [k * 2311 % 8000 - 4000, k * 4057 % 16000 - 8000, k * 3001 % 6000 - 3000];
            }
            u.cob = Some(cob);
            let mut drawn = Vec::new();
            piece_matrices(&u, t, Pose::of(&u).matrix(), &mut drawn);
            for (i, m) in drawn.iter().enumerate() {
                let sim = Vec3::from_array(ta_sim::piece_world_position(&u, t, u.cob.as_ref(), i).to_f32());
                let at = m.w_axis.truncate();
                assert!((at - sim).length() < 0.05, "{name} piece {i}: drawn at {at}, the simulation has it at {sim}");
            }
        }
    }

    /// A unit's lights shine while it works (`Sim::working`): switched off or out of energy they
    /// go out over a third of a second, come on again as gently, and after a load or a seek
    /// stand as they are at once.
    #[test]
    fn a_units_lights_follow_its_work() {
        use ta_render::model::FLAG_LIGHTS_OUT_SHIFT;
        use ta_sim::{Command, Fx, FxVec3};
        let data = std::sync::Arc::new(data_of(&[(
            "units/maker.fbi",
            "[UNITINFO]{UnitName=MAKER;MaxDamage=1000;BuildTime=100;FootprintX=2;FootprintZ=2;EnergyUse=60;MakesMetal=1;OnOffable=1;ActivateWhenBuilt=1;}",
        )]));
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        let map = ta_formats::tnt::TntMap { width: 32, height: 32, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![10; 1024], features: vec![ta_formats::tnt::FEATURE_NONE; 1024], feature_names: Vec::new(), minimap: Default::default() };
        let players = [ta_sim::PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::from_int(1000), energy: Fx::from_int(1000), ai: false }];
        let mut sim = Sim::new(types, data, &map, &ta_data::MapInfo::default(), 0, &players, 7);
        let kind = sim.types.id("MAKER").unwrap();
        let id = sim.spawn_unit(kind, 0, FxVec3::from_ints(200, 0, 200), 0, true).unwrap();
        let mut scene = Scene::default();
        scene.models.insert(kind, Some(11));
        let step = |scene: &mut Scene, sim: &mut Sim, commands: &[Command]| {
            sim.step(commands);
            scene.after_tick(sim);
        };
        let out = |scene: &mut Scene, sim: &Sim| {
            let drawn = scene.instances(sim, 1.0, &|_| false, &|_| true, (Vec2::ZERO, Vec2::splat(512.0)), 0.0);
            (drawn[0].flags >> FLAG_LIGHTS_OUT_SHIFT, drawn[0].playing)
        };
        step(&mut scene, &mut sim, &[]);
        assert_eq!(out(&mut scene, &sim), (0, 0), "a maker at work shines; nothing of one without a script plays");
        assert_eq!(scene.lights_on(id, 1.0), 1.0);
        // Switched off: out over a third of a second.
        step(&mut scene, &mut sim, &[Command::SetActive { player: 0, units: vec![id], on: false }]);
        let fading = out(&mut scene, &sim).0;
        assert!(fading > 0 && fading < 255, "its lights go out gently: {fading}");
        for _ in 0..LIGHTS_TICKS as usize {
            step(&mut scene, &mut sim, &[]);
        }
        assert_eq!(out(&mut scene, &sim).0, 255, "switched off, dark");
        assert_eq!(scene.lights_on(id, 1.0), 0.0);
        // Switched on with no energy to run it on: still dark.
        sim.players[0].economy.energy = Fx::ZERO;
        step(&mut scene, &mut sim, &[Command::SetActive { player: 0, units: vec![id], on: true }]);
        for _ in 0..LIGHTS_TICKS as usize {
            sim.players[0].economy.energy = Fx::ZERO;
            step(&mut scene, &mut sim, &[]);
        }
        assert!(sim.unit(id).unwrap().activated);
        assert_eq!(out(&mut scene, &sim).0, 255, "on but unpowered, dark");
        // Powered again: on over a third of a second.
        sim.players[0].economy.energy = Fx::from_int(1000);
        for _ in 0..LIGHTS_TICKS as usize + 1 {
            step(&mut scene, &mut sim, &[]);
        }
        assert_eq!(out(&mut scene, &sim).0, 0, "powered, it shines again");
        // A seek (the same tick followed twice) shows it as it is at once.
        step(&mut scene, &mut sim, &[Command::SetActive { player: 0, units: vec![id], on: false }]);
        assert!(out(&mut scene, &sim).0 < 255);
        scene.after_tick(&sim);
        assert_eq!(out(&mut scene, &sim).0, 255, "after a seek, its lights stand as they are");
    }

    /// The scene hands the renderer the pieces whose animated textures play, as the unit's
    /// script has them: an Arm metal maker's glowing sphere while it is on, and not while off.
    #[test]
    fn a_metal_makers_glow_plays_only_while_it_is_on() {
        use ta_sim::{Command, Fx, FxVec3};
        let Some(install) = ta_vfs::install::discover(None).ok() else { eprintln!("TA install not found; skipping"); return };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let data = std::sync::Arc::new(ta_data::GameData::load(&vfs));
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
        let cells = 64 * 64;
        let map = ta_formats::tnt::TntMap { width: 64, height: 64, sea_level: 0, tiles: Vec::new(), tile_map: Vec::new(), heights: vec![60; cells], features: vec![ta_formats::tnt::FEATURE_NONE; cells], feature_names: Vec::new(), minimap: Default::default() };
        let players = [ta_sim::PlayerSetup { side: "ARM".into(), color: 0, start: FxVec3::ZERO, metal: Fx::from_int(10000), energy: Fx::from_int(10000), ai: false }];
        let mut sim = Sim::new(types, data, &map, &ta_data::MapInfo::default(), 0, &players, 7);
        let kind = sim.types.id("ARMMAKR").unwrap();
        let sphere = sim.types.get(kind).model.pieces.iter().position(|p| p.name.eq_ignore_ascii_case("sphere")).unwrap();
        let id = sim.spawn_unit(kind, 0, FxVec3::from_ints(300, 60, 300), 0, true).unwrap();
        let mut scene = Scene::default();
        scene.models.insert(kind, Some(11));
        let playing = |sim: &mut Sim, scene: &mut Scene, on: Option<bool>| {
            let commands: Vec<Command> = on.map(|on| Command::SetActive { player: 0, units: vec![id], on }).into_iter().collect();
            sim.step(&commands);
            for _ in 0..90 {
                sim.step(&[]);
            }
            scene.after_tick(sim);
            let drawn = scene.instances(sim, 1.0, &|_| false, &|_| true, (Vec2::ZERO, Vec2::splat(1024.0)), 0.0);
            ta_render::model::piece_plays(drawn[0].playing, sphere as u32)
        };
        assert!(playing(&mut sim, &mut scene, None), "its glow plays while it works");
        assert!(!playing(&mut sim, &mut scene, Some(false)), "and holds while it is switched off");
        assert!(playing(&mut sim, &mut scene, Some(true)), "and plays again when it is on");
    }
}
