//! Perspective: riding along in a unit. The camera sits in it (on its turret, in a walker's
//! head, on a ship's bridge, in a plane's canopy) and the mouse looks round, while the unit goes
//! on with whatever it was doing: its orders are never touched, and nothing is sent to the
//! simulation. The view is the player's alone, so it works the same in multiplayer, in a
//! replay and when spectating: in the player's own units and their allies' (spectating, in
//! anyone's). The cockpit round the view is drawn by `perspective_hud`.
//!
//! Entering, the camera dives from the map view into the unit; leaving (Escape), it climbs back
//! out. A unit destroyed with the player inside loses its signal before the camera pulls away.

use std::sync::{Arc, OnceLock};

use glam::{Mat4, Vec2, Vec3};
use ta_render::cockpit::{CockpitFrame, CockpitModel};
use ta_render::{Camera, CameraMode};
use ta_sim::defs::ModelDef;
use ta_sim::{Fx, Locomotion, Order, Seen, Target, Unit, UnitId, UnitType};

use crate::game::Game;
use crate::input::Input;
use crate::scene::PieceBounds;

/// Seconds the camera takes to dive into the unit, and to climb back out.
const ENTER_TIME: f32 = 1.1;
const LEAVE_TIME: f32 = 0.85;
/// The cockpit comes in round the eye once the dive is this far along (its share of
/// `ENTER_TIME`), taking `SEAT_IN` seconds, from `SLIDE` metres behind the eye: its frame closes
/// in from the edges of the view as the dive ends. It goes back out in `SEAT_OUT`.
const SEAT_AT: f32 = 0.6;
const SEAT_IN: f32 = 0.5;
const SEAT_OUT: f32 = 0.25;
const SLIDE: f32 = 1.0;
/// Seconds after getting in that the cockpit's lamps and screens come on (as it settles).
pub const LAMPS_AT: f32 = 1.0;
/// Seconds a lost signal shows before the camera pulls away.
const LOST_TIME: f64 = 2.4;
/// The sight's magnifications.
pub const ZOOMS: [f32; 3] = [1.0, 2.5, 6.0];
/// How wide the heading tape over the view is, as a share of the view's width (but see
/// [`Cockpit::tape_width`]).
pub const TAPE_WIDTH: f32 = 0.42;
/// Hit points from which a walker is an experimental's (a Krogoth has nearly 30,000, the
/// toughest kbots under 5,000), and gets the heavier cab.
const TITAN_HEALTH: i64 = 10_000;
/// The tank's tillers (radians about their pivots, back towards the eye positive): leaning
/// forward with the speed, the one on the side the tank turns to pulled back. Forward only as far
/// as they clear the console in front (10 degrees: 37 mm to spare, measured on the model; at 14
/// they touch it); pulled back, still in view.
const TILLER_DRIVE: f32 = 0.17;
const TILLER_TURN: f32 = 0.25;
/// The paint on a cockpit's steel (linear), by its side: the Arm's a cool gunmetal, the Core's a
/// dark olive drab, anyone else's a plain field grey. (The paint wears to primer and bare steel
/// at its edges: the cockpit's shader.)
const COCKPIT_PAINT_ARM: Vec3 = Vec3::new(0.074, 0.086, 0.1);
const COCKPIT_PAINT_CORE: Vec3 = Vec3::new(0.078, 0.076, 0.05);
const COCKPIT_PAINT_OTHER: Vec3 = Vec3::new(0.08, 0.08, 0.074);
/// A turret's guns, for where the eye sits across it: the pieces under it that reach within
/// this far (world units) of the furthest forward (twin barrels, a barrel and its flash).
const GUN_TIE: f32 = 2.0;

/// The kind of cockpit a unit has: where the eye sits, how wide it sees and what the view is
/// framed by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cockpit {
    /// Tanks and trucks: the turret hatch.
    Vehicle,
    /// Kbots: a walker's visor.
    Walker,
    /// Experimental walkers (the Krogoth): a heavier cab, set deep in the armour, with more
    /// of everything than a kbot's.
    Titan,
    /// The Commander's own helmet.
    Commander,
    /// Hovercraft: a skirted cab.
    Hover,
    /// Ships: the bridge.
    Bridge,
    /// Submarines: the periscope.
    Periscope,
    /// Fighters and bombers: a canopy.
    Canopy,
    /// Gunships, transports and air constructors: a glass cockpit that hovers.
    Gunship,
    /// Guns on buildings: the gun house.
    Emplacement,
}

impl Cockpit {
    pub fn of(t: &UnitType) -> Cockpit {
        match t.locomotion {
            Locomotion::Static => Cockpit::Emplacement,
            Locomotion::Air if t.is_plane() => Cockpit::Canopy,
            Locomotion::Air => Cockpit::Gunship,
            Locomotion::Naval if t.def.water_line > 0.0 => Cockpit::Periscope,
            Locomotion::Naval => Cockpit::Bridge,
            Locomotion::Hover => Cockpit::Hover,
            Locomotion::Ground if t.def.is_commander => Cockpit::Commander,
            Locomotion::Ground if t.def.upright && t.max_health >= Fx::from_int(TITAN_HEALTH) => Cockpit::Titan,
            Locomotion::Ground if t.def.upright => Cockpit::Walker,
            Locomotion::Ground => Cockpit::Vehicle,
        }
    }

    pub fn flies(self) -> bool {
        matches!(self, Cockpit::Canopy | Cockpit::Gunship)
    }

    /// How wide the heading tape over the view is, as a share of the view's width: a fighter's
    /// canopy arches over the head, and a tape as wide as elsewhere fits only far down under
    /// its sides, over the middle of the sight.
    pub fn tape_width(self) -> f32 {
        if self == Cockpit::Canopy { 0.26 } else { TAPE_WIDTH }
    }

    /// Its modelled cockpit (see `tools/cockpits/build.py`), if it has one: the models are
    /// read once.
    pub fn model(self) -> Option<Arc<CockpitModel>> {
        static MODELS: OnceLock<Vec<(&'static str, Arc<CockpitModel>)>> = OnceLock::new();
        let models = MODELS.get_or_init(|| {
            ta_render::cockpit_models()
                .iter()
                .filter_map(|(name, bytes)| match CockpitModel::from_glb(bytes) {
                    Ok(m) => Some((*name, Arc::new(m))),
                    Err(e) => {
                        log::warn!("cockpit {name}: {e}");
                        None
                    }
                })
                .collect()
        });
        let name = match self {
            Cockpit::Vehicle | Cockpit::Hover => "vehicle",
            Cockpit::Walker => "walker",
            Cockpit::Titan => "titan",
            Cockpit::Commander => "commander",
            Cockpit::Bridge => "bridge",
            Cockpit::Canopy => "canopy",
            Cockpit::Gunship => "gunship",
            Cockpit::Emplacement => "emplacement",
            Cockpit::Periscope => return None,
        };
        models.iter().find(|m| m.0 == name).map(|m| m.1.clone())
    }

    /// Vertical field of view (degrees) without magnification.
    fn fov(self) -> f32 {
        match self {
            Cockpit::Vehicle | Cockpit::Hover => 64.0,
            Cockpit::Walker | Cockpit::Commander => 68.0,
            Cockpit::Titan => 64.0,
            Cockpit::Bridge => 60.0,
            Cockpit::Periscope => 40.0,
            Cockpit::Canopy => 74.0,
            Cockpit::Gunship => 70.0,
            Cockpit::Emplacement => 58.0,
        }
    }

    /// How far above the top of the piece it sits on the eye is (px).
    fn lift(self) -> f32 {
        match self {
            Cockpit::Vehicle | Cockpit::Hover => 7.0,
            Cockpit::Walker | Cockpit::Titan => 1.5,
            Cockpit::Commander => 1.0,
            Cockpit::Bridge => 4.0,
            Cockpit::Periscope => 3.0,
            Cockpit::Canopy => 1.0,
            Cockpit::Gunship => 1.2,
            Cockpit::Emplacement => 3.0,
        }
    }

    /// How far up and down the eye can look (radians).
    fn pitch_range(self) -> (f32, f32) {
        match self {
            Cockpit::Periscope => (-0.18, 0.3),
            Cockpit::Emplacement => (-0.7, 1.1),
            _ => (-0.95, 0.7),
        }
    }
}

/// A damped spring for the instruments' needles and the camera's give (per frame, real time).
#[derive(Clone, Copy, Debug, Default)]
pub struct Needle {
    pub x: f32,
    v: f32,
}

impl Needle {
    pub fn step(&mut self, target: f32, omega: f32, zeta: f32, dt: f32) {
        let steps = (dt / 0.008).ceil().clamp(1.0, 8.0) as usize;
        let h = dt / steps as f32;
        for _ in 0..steps {
            self.v += (omega * omega * (target - self.x) - 2.0 * zeta * omega * self.v) * h;
            self.x += self.v * h;
        }
    }
}

/// A walker's armour by section: how much of the whole each section has lost. The simulation
/// knows one hull; this spreads what it loses over the sections by where the blows came from, so
/// that the diagram's total is the hull's and a flank that took the fire shows it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Armour {
    lost: [f32; 8],
}

impl Armour {
    /// Each section's share of the whole (head, centre torso, left and right torso, left and right
    /// arm, left and right leg).
    pub const SHARE: [f32; 8] = [0.06, 0.22, 0.12, 0.12, 0.10, 0.10, 0.14, 0.14];
    /// The way each section faces across the hull (radians, the right positive): a blow from there
    /// finds it.
    const FACING: [f32; 8] = [0.0, 0.0, -0.9, 0.9, -1.5, 1.5, -2.2, 2.2];

    /// Armour worn evenly: `lost` of the whole gone, by the sections' shares.
    pub fn worn(lost: f32) -> Armour {
        Armour { lost: Self::SHARE.map(|share| share * lost) }
    }

    /// A blow of `damage` (a share of the whole) from `from` (radians off the hull's heading, the
    /// right positive): the sections that face it take most.
    pub fn hit(&mut self, damage: f32, from: f32) {
        let exposure: [f32; 8] = std::array::from_fn(|k| Self::SHARE[k] * (0.35 + (from - Self::FACING[k]).cos().max(0.0)));
        let total: f32 = exposure.iter().sum();
        for (lost, e) in self.lost.iter_mut().zip(exposure) {
            *lost += damage * e / total;
        }
    }

    /// The whole's loss made `lost` (the hull repaired, or blows the hits did not see).
    pub fn settle(&mut self, lost: f32) {
        let sum: f32 = self.lost.iter().sum();
        if sum > 1e-6 {
            for v in &mut self.lost {
                *v *= lost / sum;
            }
        } else {
            *self = Armour::worn(lost);
        }
    }

    /// How whole each section is (0..1): its loss against its share.
    pub fn integrity(&self) -> [f32; 8] {
        std::array::from_fn(|k| (1.0 - self.lost[k] / Self::SHARE[k]).clamp(0.0, 1.0))
    }
}

/// Which way `at` lies off a unit's heading, seen from above (radians, the right positive).
fn off_heading(u: &Unit, at: Vec3) -> f32 {
    let h = u.heading as f32 * std::f32::consts::TAU / 65536.0;
    let ahead = Vec3::new(h.sin(), 0.0, h.cos());
    let d = at - Vec3::from_array(u.pos.to_f32());
    d.dot(ahead.cross(Vec3::Y)).atan2(d.dot(ahead))
}

/// What the sight last saw of the unit its guns were on: which, and where its model was as drawn
/// (its pieces' corners in the world, see [`drawn_corners`]). When it is destroyed, its bracket
/// closes up where it was.
#[derive(Clone, Debug)]
pub struct Marked {
    pub id: UnitId,
    pub corners: Vec<Vec3>,
}

/// A unit's model as it is drawn standing at `at` (its foot's middle, as the scene places it
/// this frame), its script's pose with its pieces hidden left out: each drawn piece's bounding
/// corners in the world, into `out` (cleared first). Empty before the models are loaded.
pub fn drawn_corners(game: &Game, u: &Unit, at: Vec3, out: &mut Vec<Vec3>) {
    out.clear();
    let Some(bounds) = game.scene.piece_bounds(u.type_id) else { return };
    let t = game.sim.unit_type(u);
    let root = Mat4::from_rotation_translation(glam::Quat::from_rotation_y(u.heading as f32 * std::f32::consts::TAU / 65536.0), at);
    let mut pieces = Vec::new();
    crate::scene::piece_matrices(u, t, root, &mut pieces);
    for (m, b) in pieces.iter().zip(bounds.iter()) {
        let Some(b) = b else { continue };
        if *m == Mat4::ZERO {
            continue;
        }
        for k in 0..8 {
            let q = Vec3::new(if k & 1 == 0 { b.min.x } else { b.max.x }, if k & 2 == 0 { b.min.y } else { b.max.y }, if k & 4 == 0 { b.min.z } else { b.max.z });
            out.push(m.transform_point3(q));
        }
    }
}

/// A crack in the glass: where it starts (a share of the window's width and height), the way it
/// runs and how far (a share of the picture's height), its pattern, and when it came.
#[derive(Clone, Copy, Debug)]
pub struct Crack {
    pub at: Vec2,
    pub angle: f32,
    pub length: f32,
    pub seed: u32,
    pub since: f64,
}

/// The hull's shares under which the glass has one crack more: half the hull gone, three
/// quarters, and nearly all of it.
pub const CRACK_AT: [f32; 3] = [0.5, 0.25, 0.12];

/// What the cockpit's instruments remember from frame to frame.
#[derive(Default)]
pub struct HudState {
    /// When the mode was entered (interface seconds): the instruments come on.
    pub entered: f64,
    /// The unit's health (fraction) at the last frame, to see hits.
    pub health: f32,
    /// Hits taken: when, where the attacker was (none when it is not known) and how hard (0..1).
    pub hits: Vec<(f64, Option<Vec3>, f32)>,
    /// Kills when the view got in, and when the last one was scored.
    pub kills_before: u64,
    pub kills: u64,
    pub kill_flash: Option<f64>,
    /// The weapons' reload counters at the last frame (a shot starts a new reload).
    reloads: [u32; 3],
    /// Shots fired, by weapon, and when each last fired; when each last came ready.
    pub last_shot: [f64; 3],
    pub ready_since: [f64; 3],
    /// What the guns are on, and since when (the sight's bracket closes in on a new one); the
    /// last one as last seen, and, destroyed, since when its bracket collapses.
    pub target: Option<(UnitId, f64)>,
    pub last_target: Option<Marked>,
    pub destroyed: Option<(Marked, f64)>,
    /// The camera's kick from firing and jolt from hits (radians, decaying).
    pub kick: f32,
    pub jolt: f32,
    /// Needles: speed (fraction of top speed), turning (fraction of the fastest turn, right
    /// positive: the crew's levers and helm follow both), altitude (px).
    pub speed: Needle,
    pub turn: Needle,
    pub altitude: Needle,
    /// The speed and the turn the crew's levers, sticks and helm follow: eased more slowly than
    /// the needles and without overshoot, so a hand on them does not jerk with every
    /// correction of the course.
    pub controls_speed: Needle,
    pub controls_turn: Needle,
    /// The hull's heading at the last frame (radians), to see it turn.
    heading: Option<f32>,
    /// Messages across the lower part of the view (text, since when, how urgent: 0 note, 1
    /// warning).
    pub messages: Vec<(String, f64, u8)>,
    /// The radar's sweep (radians).
    pub sweep: f32,
    /// Cracks in the glass, one for each of [`CRACK_AT`] the hull is under: they come as the unit
    /// is worn down, and go when it is mended.
    pub cracks: Vec<Crack>,
    /// A walker's armour by section.
    pub armour: Armour,
}

impl HudState {
    /// The cracks for a hull at `health` (a share of the whole) on the unit in slot `slot`: as
    /// many as the shares of [`CRACK_AT`] it is under, each where that unit's glass always cracks
    /// (new ones grow from `now`); mended, the newest go.
    pub fn crack_to(&mut self, health: f32, slot: u32, now: f64) {
        let want = CRACK_AT.iter().filter(|&&at| health < at).count();
        self.cracks.truncate(want);
        while self.cracks.len() < want {
            let k = self.cracks.len() as u32;
            let seed = slot.wrapping_mul(2_654_435_761) ^ (k + 1).wrapping_mul(0x9E37_79B9);
            let r = |n: u32| ((seed.wrapping_mul(n * 2 + 1).wrapping_add(n.wrapping_mul(0x632B_E5AB)) >> 8) % 1000) as f32 / 1000.0;
            // Either side of the sight, the first on one side and the next on the other (on the
            // main pane: further out, a cab's struts and mullions stand), and longer the worse it
            // gets.
            let left = (slot.wrapping_mul(2_654_435_761) >> 16) & 1 == 0;
            let left = left != (k % 2 == 1);
            let at = Vec2::new(if left { 0.26 + r(2) * 0.14 } else { 0.6 + r(2) * 0.14 }, 0.12 + r(3) * 0.5);
            self.cracks.push(Crack { at, angle: r(4) * std::f32::consts::TAU, length: 0.09 + r(5) * 0.06 + k as f32 * 0.05, seed, since: now });
        }
    }

    pub fn say(&mut self, text: impl Into<String>, now: f64, urgent: bool) {
        let text = text.into();
        self.messages.retain(|m| m.0 != text);
        self.messages.push((text, now, urgent as u8));
        if self.messages.len() > 3 {
            self.messages.remove(0);
        }
    }
}

pub struct Perspective {
    pub reduced_motion: bool,
    pub reduced_flashes: bool,
    pub unit: UnitId,
    pub type_id: usize,
    pub cockpit: Cockpit,
    /// The modelled cockpit round the view, if this kind has one; how far the cockpit (the
    /// model, or the painted frame) has come in round the eye (0..1: it closes in from behind
    /// the eye as the dive ends, and goes back out on leaving); the model's mounting this frame
    /// (its give as the unit rides); and where its instruments' screens are on the screen
    /// (pixels: bottom left, bottom right, top right, top left).
    pub model: Option<Arc<CockpitModel>>,
    pub seat: f32,
    pub mount: Mat4,
    pub screens: Vec<(String, [Vec2; 4])>,
    /// The lip under the modelled cockpit's window (eye space, see [`sill`]), and how far down
    /// the screen it is this frame (pixels): marks over the world stay above it.
    sill: Option<Vec3>,
    pub sill_y: Option<f32>,
    /// The top of the modelled cockpit's window across the middle of the view (eye space, see
    /// [`lintel`]), and how far down the screen it is this frame (pixels): a heading tape
    /// stays under it.
    lintel: Option<Vec3>,
    pub lintel_y: Option<f32>,
    /// The eye's last place and speed (world units a second), and the cockpit's give (eye
    /// space, metres), for the ride.
    last_eye: Option<Vec3>,
    last_vel: Vec3,
    /// The eye's velocity and acceleration, smoothed (the frames' own unevenness, taken as
    /// motion, shook the cockpit and its instruments' lettering).
    smooth_vel: Vec3,
    smooth_acc: Vec3,
    give: [Needle; 3],
    /// Where the eye looks: a heading (radians, 0 along +z as units face; left is larger)
    /// and a pitch (up positive). Aircraft: an offset from the nose.
    pub look_yaw: f32,
    pub look_pitch: f32,
    /// The sight's magnification (an index into [`ZOOMS`]).
    pub zoom: usize,
    /// Behind the unit instead of in it (V).
    pub chase: bool,
    /// The night sight (N).
    pub night_vision: bool,
    /// The camera before entering (restored on leaving, over where the unit is then).
    before: Camera,
    /// The dive in (0 → 1) and the climb out (1 → 0).
    blend: f32,
    pub leaving: bool,
    /// Leaving is over: the app drops the mode.
    pub done: bool,
    /// This frame's eye, view direction, roll and field of view (in the unit).
    pub eye: Vec3,
    pub forward: Vec3,
    pub roll: f32,
    pub fov: f32,
    /// The middle of the unit's body this frame (behind it, what the eye keeps in sight).
    pub centre: Vec3,
    /// The unit's body at this frame (its matrix), and its pieces' matrices.
    pub body: Mat4,
    pieces: Vec<Mat4>,
    /// Where the eye sits across a turret, over its guns: the turret's piece and the line (its
    /// own x), measured once on the model as built (see [`gun_line`]).
    gun_line: Option<(usize, f32)>,
    pub hud: HudState,
    /// The hour of the battle's day this frame (as the world is lit; none: TA's own light), for
    /// the instruments' clock.
    pub hours: Option<f32>,
    /// The hull's attitude as it is drawn this frame (radians): the nose's pitch (up positive)
    /// and the roll (the right side down positive); the ground's slope under a tank, a plane's
    /// bank.
    pub attitude: (f32, f32),
    /// Where the main gun's turret points across the hull (radians, the right of the hull's
    /// heading positive), and the way the gun points in the world: none when the unit has not
    /// aimed anything yet.
    pub gun_azimuth: Option<f32>,
    pub gun_dir: Option<Vec3>,
    /// The unit is gone (destroyed, or taken by a player whose units the view may not ride
    /// in): since when (interface seconds).
    pub lost: Option<f64>,
    /// Captures: leaving after this many seconds (`TA_PERSPECTIVE_LEAVE`).
    leave_after: Option<f32>,
}

impl Perspective {
    /// Whether the player may ride along in a unit: finished, in sight, able to move or
    /// shoot, and theirs or an ally's (spectating or watching a replay, anyone's).
    pub fn can_enter(game: &Game, id: UnitId) -> bool {
        let Some(u) = game.sim.unit(id) else { return false };
        let t = game.sim.unit_type(u);
        Self::may_watch(game, u) && !u.dead && u.is_complete() && u.carried_by.is_none() && game.seen(u) == Seen::Visible && (t.locomotion != Locomotion::Static || t.weapons.iter().any(|w| w.is_some()))
    }

    /// Whose units the view may ride in: the player's and their allies' (a spectator's,
    /// anyone's).
    fn may_watch(game: &Game, u: &Unit) -> bool {
        game.spectator || u.owner == game.local_player || game.sim.allied(u.owner, game.local_player)
    }

    /// Gets into a unit: the camera dives in. The unit is left to what it was doing.
    pub fn enter(game: &mut Game, camera: &Camera, id: UnitId, now: f64) -> Option<Perspective> {
        if !Self::can_enter(game, id) {
            return None;
        }
        let u = game.sim.unit(id)?;
        let t = game.sim.unit_type(u);
        let cockpit = Cockpit::of(t);
        let heading = u.heading as f32 * std::f32::consts::TAU / 65536.0;
        let health = (u.health / t.max_health.max(Fx::ONE)).to_f32();
        let kills = u.kills;
        let type_id = u.type_id;
        let ours = u.owner == game.local_player || game.spectator;
        let model = cockpit.model();
        let mut p = Perspective {
            reduced_motion: false,
            reduced_flashes: false,
            unit: id,
            type_id,
            cockpit,
            look_yaw: if cockpit.flies() { 0.0 } else { heading },
            look_pitch: if cockpit.flies() { 0.0 } else { -0.12 },
            zoom: 0,
            chase: false,
            night_vision: false,
            before: camera.clone(),
            blend: 0.0,
            leaving: false,
            done: false,
            eye: camera.eye(),
            centre: camera.focus,
            forward: (camera.focus - camera.eye()).normalize_or(Vec3::NEG_Z),
            roll: 0.0,
            fov: cockpit.fov().to_radians(),
            body: Mat4::IDENTITY,
            pieces: Vec::new(),
            gun_line: None,
            hud: HudState { entered: now, health, kills_before: kills, kills, armour: Armour::worn(1.0 - health), ..Default::default() },
            hours: None,
            attitude: (0.0, 0.0),
            gun_azimuth: None,
            gun_dir: None,
            lost: None,
            leave_after: std::env::var("TA_PERSPECTIVE_LEAVE").ok().and_then(|v| v.parse().ok()),
            sill: model.as_deref().and_then(sill),
            sill_y: None,
            lintel: model.as_deref().and_then(|m| lintel(m, cockpit.tape_width())),
            lintel_y: None,
            model,
            seat: 0.0,
            mount: Mat4::IDENTITY,
            screens: Vec::new(),
            last_eye: None,
            last_vel: Vec3::ZERO,
            smooth_vel: Vec3::ZERO,
            smooth_acc: Vec3::ZERO,
            give: Default::default(),
        };
        // (Got into a unit already worn, its glass is cracked already.)
        p.hud.crack_to(health, id.slot, now - 1.0);
        p.hud.say(ta_ui::i18n::t("LINK ESTABLISHED"), now + 0.6, false);
        // Captures: the view to start in (`TA_PERSPECTIVE_VIEW=chase|zoom|night`).
        match std::env::var("TA_PERSPECTIVE_VIEW").as_deref() {
            Ok("chase") => p.chase = true,
            Ok("zoom") => p.zoom = 1,
            Ok("night") => p.night_vision = true,
            _ => {}
        }
        // Captures: looking off the hull's way (`TA_PERSPECTIVE_TWIST`, degrees to the right) and
        // up (`TA_PERSPECTIVE_PITCH`, degrees).
        if let Some(d) = std::env::var("TA_PERSPECTIVE_TWIST").ok().and_then(|v| v.parse::<f32>().ok()) {
            p.look_yaw -= d.to_radians();
        }
        if let Some(d) = std::env::var("TA_PERSPECTIVE_PITCH").ok().and_then(|v| v.parse::<f32>().ok()) {
            p.look_pitch += d.to_radians();
        }
        // The crew answers; a unit of the player's own is selected, to give orders to once
        // the view is back over the map.
        game.voices.push((id, ta_audio::SoundEvent::Select));
        if ours {
            game.selection = vec![id];
        }
        game.follow = None;
        Some(p)
    }

    /// Climbs back out of the unit.
    pub fn leave(&mut self, game: &mut Game) {
        if self.leaving {
            return;
        }
        self.leaving = true;
        game.scene.hide_near = None;
    }

    /// The camera the map view goes back to: as it was, over where the unit is now.
    pub fn camera_after(&self, game: &Game) -> Camera {
        let mut c = self.before.clone();
        let at = game.scene.unit_position(self.unit, game.alpha()).unwrap_or(self.eye);
        c.focus = Vec3::new(at.x, at.y.max(0.0), at.z);
        c.roll = 0.0;
        c
    }

    /// A frame: the look read, the eye put in the unit, the camera set.
    pub fn update(&mut self, game: &mut Game, input: &Input, camera: &mut Camera, dt: f32, now: f64) {
        let viewport = camera.viewport;
        // Still there to ride in? Destroyed, or taken by a player the view may not follow: the
        // signal is lost. Carried off by a transport: the view climbs back out.
        let unit = game.sim.unit(self.unit);
        let watched = unit.is_some_and(|u| !u.dead && Self::may_watch(game, u));
        let aboard = unit.is_some_and(|u| u.carried_by.is_some());
        if !watched && self.lost.is_none() && !self.leaving {
            self.lost = Some(now);
            game.scene.hide_near = None;
        }
        if self.lost.is_some_and(|t| now - t > LOST_TIME) || watched && aboard || self.leave_after.is_some_and(|after| now - self.hud.entered > after as f64) {
            self.leave(game);
        }
        let alive = watched && !aboard;
        // The dive in and the climb out.
        if self.reduced_motion {
            self.blend = if self.leaving { 0.0 } else { 1.0 };
        }
        if self.leaving {
            self.blend = (self.blend - dt / LEAVE_TIME).max(0.0);
            if self.blend <= 0.0 {
                self.done = true;
            }
        } else {
            self.blend = (self.blend + dt / ENTER_TIME).min(1.0);
        }
        if alive && !self.leaving {
            self.look(input, dt, now);
            self.place_eye(game, dt);
        }
        // The camera: between the map view and the eye while diving or climbing.
        let e = ease(self.blend);
        let fov = self.fov / ZOOMS[self.zoom];
        if e >= 1.0 {
            aim_camera(camera, self.eye, self.view_dir(), fov, self.roll);
        } else {
            let from = if self.leaving { self.camera_after(game) } else { self.before.clone() };
            let (e0, f0) = (from.eye(), (from.focus - from.eye()).normalize_or(Vec3::NEG_Z));
            let arc = Vec3::Y * (e * (1.0 - e) * 4.0 * 30.0);
            let eye = e0.lerp(self.eye, e) + arc;
            let fwd = slerp_dir(f0, self.view_dir(), e);
            let fov = from.fov_y + (fov - from.fov_y) * e;
            camera.viewport = viewport;
            aim_camera(camera, eye, fwd, fov, self.roll * e);
        }
        if alive && !self.leaving {
            self.watch(game, now, dt);
            // Seen from inside, the pieces round the eye go (not from behind it); through the
            // magnified sight or from a modelled cockpit, all of it (from up there the barrel
            // only showed as a pale slab).
            let margin = if self.zoom > 0 || self.model.is_some() { 1000.0 } else { 1.2 };
            game.scene.hide_near = (!self.chase).then_some((self.unit, self.eye, margin));
        }
        self.ride(camera, dt);
    }

    /// The cockpit this frame: in round the eye as the dive ends (not from behind, not through
    /// the magnified sight); the model's give as the unit rides and fires and is hit, and where
    /// its screens land.
    fn ride(&mut self, camera: &Camera, dt: f32) {
        let shown = !self.chase && self.zoom == 0 && self.lost.is_none() && !self.leaving && self.blend >= SEAT_AT;
        self.seat = if shown { (self.seat + dt / SEAT_IN).min(1.0) } else { (self.seat - dt / SEAT_OUT).max(0.0) };
        if self.reduced_motion { self.seat = if shown { 1.0 } else { 0.0 }; }
        let Some(model) = self.model.clone() else { return };
        // The ride: the cockpit sways against the eye's acceleration (bumps, braking, turns),
        // both smoothed over a tenth of a second or so, and none at all for the slightest.
        let dt = dt.max(1e-3);
        let vel = self.last_eye.map_or(Vec3::ZERO, |e| (self.eye - e) / dt);
        let follow = |tau: f32| 1.0 - (-dt / tau).exp();
        if self.last_eye.is_some() {
            self.smooth_vel += (vel - self.smooth_vel) * follow(0.08);
            let acc = (self.smooth_vel - self.last_vel) / dt;
            self.smooth_acc += (acc - self.smooth_acc) * follow(0.12);
        }
        self.last_eye = Some(self.eye);
        self.last_vel = self.smooth_vel;
        let acc_eye = camera.view().transform_vector3(self.smooth_acc);
        for (k, g) in self.give.iter_mut().enumerate() {
            let push = -acc_eye[k] * 0.000012;
            let target = (push.signum() * (push.abs() - 0.0006).max(0.0)).clamp(-0.012, 0.012);
            g.step(target, 14.0, 0.7, dt);
        }
        // Coming in from behind the eye: straight ahead into place, so that nothing of it
        // crosses the view on the way (risen from below, its roof swept across the middle).
        let back = (1.0 - ease(self.seat)) * SLIDE;
        let t = self.hud.entered as f32 + self.hud.sweep;
        let jolt = self.hud.jolt * 0.6;
        let kick = self.hud.kick;
        self.mount = Mat4::from_translation(Vec3::new(self.give[0].x + (t * 37.0).sin() * jolt, self.give[1].x + (t * 29.0).cos() * jolt, self.give[2].x + kick * 0.9 + back)) * Mat4::from_rotation_x(-kick * 0.6);
        if self.reduced_motion { self.mount = Mat4::IDENTITY; }
        self.screens = model
            .screens
            .iter()
            .map(|s| (s.name.clone(), s.corners().map(|c| ta_render::cockpit::project(camera, self.fov, self.mount, c).unwrap_or(Vec2::splat(-1e4)))))
            .collect();
        self.sill_y = self.sill.and_then(|s| ta_render::cockpit::project(camera, self.fov, self.mount, s)).map(|q| q.y);
        self.lintel_y = self.lintel.and_then(|s| ta_render::cockpit::project(camera, self.fov, self.mount, s)).map(|q| q.y);
    }

    /// How the modelled cockpit is drawn this frame; none while it is down.
    pub fn cockpit_frame(&self, game: &Game, now: f64) -> Option<CockpitFrame> {
        let rain = game.effects.rain(crate::scene::game_seconds(game.sim.tick, game.alpha()) as f32);
        let model = self.model.as_ref()?;
        if self.seat <= 0.0 {
            return None;
        }
        let side = game.sim.unit(self.unit).and_then(|u| game.sim.players.get(u.owner as usize)).map_or("", |p| p.side.as_str());
        // The side's colour on the trim, and its paint on the steel.
        let (trim, paint) = if side.eq_ignore_ascii_case("ARM") {
            (Vec3::new(0.1, 0.26, 0.62), COCKPIT_PAINT_ARM)
        } else if side.eq_ignore_ascii_case("CORE") {
            (Vec3::new(0.62, 0.07, 0.04), COCKPIT_PAINT_CORE)
        } else {
            (Vec3::new(0.62, 0.45, 0.12), COCKPIT_PAINT_OTHER)
        };
        // The crew's levers and sticks follow the unit as it drives itself: tillers lean with
        // its speed, the one on the side it turns to pulls back; sticks lean with the turn too.
        let throttle = self.hud.controls_speed.x.clamp(-0.2, 1.0);
        let steer = self.hud.controls_turn.x.clamp(-1.0, 1.0);
        let parts = model
            .parts
            .iter()
            .map(|(name, _)| match name.as_str() {
                "lever_l" => Mat4::from_rotation_x(TILLER_TURN * (-steer).max(0.0) - TILLER_DRIVE * throttle),
                "lever_r" => Mat4::from_rotation_x(TILLER_TURN * steer.max(0.0) - TILLER_DRIVE * throttle),
                "stick" => Mat4::from_rotation_z(-steer * 0.3),
                // A walker's two sticks: both forward with the speed, apart to turn.
                "stick_l" => Mat4::from_rotation_x(-(throttle - steer * 0.6) * 0.28),
                "stick_r" => Mat4::from_rotation_x(-(throttle + steer * 0.6) * 0.28),
                "helm" => Mat4::from_rotation_z(-steer * 1.6),
                _ => Mat4::IDENTITY,
            })
            .collect();
        let alarm = if self.hud.health < 0.35 { 1.0 } else { 0.0 };
        let flash = if self.reduced_flashes { 0.0 } else { self.hud.hits.last().map_or(0.0, |h| (1.0 - (now - h.0) as f32 / 0.3).clamp(0.0, 1.0) * h.2) };
        // (On once it is in round the eye.)
        let lamps = (((now - self.hud.entered) as f32 - LAMPS_AT) / 0.35).clamp(0.0, 1.0);
        // The cabin lamp, steady; with the hull failing it stutters (not with flashes reduced).
        let stutter = if alarm > 0.0 && !self.reduced_flashes {
            let k = (now * 12.0) as u64;
            let roll = (k.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) % 100;
            if roll < 12 { 0.35 } else if roll < 20 { 0.7 } else { 1.0 }
        } else {
            1.0
        };
        Some(CockpitFrame {
            mount: self.mount,
            fov_y: self.fov,
            parts,
            trim,
            paint,
            lamps,
            alarm,
            flash,
            cabin: Vec3::new(1.0, 0.84, 0.62) * 0.55 * stutter,
            glow: Vec3::new(0.25, 0.9, 0.35) * 0.3 * lamps,
            sun: 0.55,
            // Hold blinking warning lamps visibly on; health text and directional hit
            // indicators remain available without pulsing the whole cabin.
            time: if self.reduced_flashes { 0.25 } else { now as f32 },
            rain: rain.clamp(0.0, 1.0),
            flies: self.cockpit.flies(),
        })
    }

    /// The view's direction this frame (the look, with the firing kick and hits' jolt).
    pub fn view_dir(&self) -> Vec3 {
        self.forward
    }

    /// Where the eye is going (world units a second, smoothed): an aircraft's flight path.
    pub fn velocity(&self) -> Vec3 {
        self.smooth_vel
    }

    /// Reads the mouse (and a gamepad's left stick) into where the eye looks, and the view's
    /// own keys: the sight's magnification, the view from behind and the night sight.
    fn look(&mut self, input: &Input, dt: f32, now: f64) {
        use winit::keyboard::KeyCode as K;
        if input.pressed(K::KeyV) {
            self.chase = !self.chase;
        }
        if input.pressed(K::KeyN) {
            self.night_vision = !self.night_vision;
            let text = if self.night_vision { ta_ui::i18n::n("NIGHT SIGHT ON") } else { ta_ui::i18n::n("NIGHT SIGHT OFF") };
            self.hud.say(ta_ui::i18n::t(text), now, false);
        }
        let zoom_in = input.wheel > 0.0 || input.pressed(K::KeyZ) && !input.modifiers.shift_key();
        let zoom_out = input.wheel < 0.0 || input.pressed(K::KeyZ) && input.modifiers.shift_key();
        if zoom_in {
            self.zoom = (self.zoom + 1).min(ZOOMS.len() - 1);
        } else if zoom_out {
            self.zoom = self.zoom.saturating_sub(1);
        }
        // The mouse: raw motion (the cursor is held in the window), else the cursor's (never
        // both: a frame without raw motion would take the cursor's as more of it); a pad's
        // left stick turns the view as fast as two radians a second. Slower through the sight.
        let delta = if input.raw_seen { input.raw_delta } else { input.mouse_delta };
        let steady = ZOOMS[self.zoom].sqrt();
        let turn = delta * (0.0022 / steady) + input.pad_pan * (2.0 * dt / steady);
        if self.cockpit.flies() {
            // Aircraft: round from the nose, which the view turns with.
            self.look_yaw = (self.look_yaw - turn.x).clamp(-2.4, 2.4);
            self.look_pitch = (self.look_pitch - turn.y).clamp(-1.1, 0.8);
        } else {
            let (lo, hi) = self.cockpit.pitch_range();
            self.look_yaw -= turn.x;
            self.look_pitch = (self.look_pitch - turn.y).clamp(lo, hi);
        }
    }

    /// Puts the eye in the unit as it is drawn this frame, looking where the view looks.
    fn place_eye(&mut self, game: &mut Game, dt: f32) {
        let alpha = game.alpha();
        let Some(body) = game.scene.pose(&game.sim, self.unit, alpha, &mut self.pieces) else { return };
        self.body = body;
        let Some(u) = game.sim.unit(self.unit) else { return };
        let t = game.sim.unit_type(u);
        let bounds = game.scene.piece_bounds(self.type_id);
        let bound = |i: usize| bounds.as_ref().and_then(|b| b.get(i).copied().flatten()).map_or((Vec3::ZERO, Vec3::ZERO), |b| (b.min, b.max));
        let named = |names: &[&str]| names.iter().find_map(|n| t.model.pieces.iter().position(|p| p.name.eq_ignore_ascii_case(n))).filter(|&i| i < self.pieces.len() && self.pieces[i] != Mat4::ZERO);
        // The piece the main gun aims from, and the piece under the body that carries it (a
        // barrel's turret).
        let aim_piece = (0..3).filter(|&s| t.weapons[s].is_some()).find_map(|s| game.sim.aim_from_piece(u, s)).filter(|&i| i < self.pieces.len());
        let gun_mount = aim_piece.map(|mut i| {
            while let Some(parent) = t.model.pieces.get(i).and_then(|p| p.parent) {
                if t.model.pieces.get(parent).is_none_or(|p| p.parent.is_none()) {
                    break;
                }
                i = parent;
            }
            i
        });
        // The hull's attitude, and where its main turret and gun point.
        self.attitude = attitude_of(&body);
        let drawn = |i: usize| self.pieces.get(i).filter(|m| **m != Mat4::ZERO);
        self.gun_azimuth = gun_mount.and_then(drawn).map(|m| turret_azimuth(&body, m));
        self.gun_dir = aim_piece.and_then(drawn).map(|m| m.transform_vector3(Vec3::Z).normalize_or(Vec3::Z));
        let lift = self.cockpit.lift();
        let mid = |lo: Vec3, hi: Vec3| (lo + hi) * 0.5;
        // Where the crew sits: in a walker's head (the visor at its front), in a turret's
        // cupola (on top, towards the back), over an aircraft's nose, high amidships on a ship.
        let (piece, local) = match self.cockpit {
            Cockpit::Walker | Cockpit::Titan | Cockpit::Commander => match named(&["head"]) {
                Some(i) => {
                    let (lo, hi) = bound(i);
                    (Some(i), Vec3::new(mid(lo, hi).x, lo.y + (hi.y - lo.y) * 0.6, mid(lo, hi).z + (hi.z - mid(lo, hi).z) * 0.4))
                }
                None => {
                    let i = named(&["torso", "chest", "body"]).or(gun_mount);
                    let (lo, hi) = i.map_or((Vec3::ZERO, Vec3::ZERO), bound);
                    (i, Vec3::new(mid(lo, hi).x, hi.y + lift, mid(lo, hi).z + (hi.z - mid(lo, hi).z) * 0.4))
                }
            },
            Cockpit::Vehicle | Cockpit::Hover | Cockpit::Emplacement => {
                let i = named(&["turret", "cupola", "cab", "cockpit"]).or(gun_mount);
                let (lo, hi) = i.map_or((Vec3::ZERO, Vec3::ZERO), bound);
                // Right over the gun, so its barrel runs out ahead below the eye (between twin
                // barrels), measured once on the model as built: measured on the pieces as
                // posed, the eye hopped from barrel to barrel as each recoiled or flashed, or as
                // a rounding put one level with the other ahead while the turret turned.
                let over_gun = i
                    .and_then(|i| {
                        if self.gun_line.is_none_or(|(at, _)| at != i) {
                            self.gun_line = bounds.as_deref().and_then(|b| gun_line(&t.model, i, b)).map(|x| (i, x));
                        }
                        self.gun_line.map(|(_, x)| x)
                    })
                    .unwrap_or(mid(lo, hi).x);
                (i, Vec3::new(over_gun, hi.y + lift, lo.z + (hi.z - lo.z) * 0.3))
            }
            Cockpit::Canopy | Cockpit::Gunship => {
                let i = named(&["cockpit", "canopy"]).or(Some(0));
                let (lo, hi) = i.map_or((Vec3::ZERO, Vec3::ZERO), bound);
                (i, Vec3::new(mid(lo, hi).x, hi.y + lift, mid(lo, hi).z + (hi.z - mid(lo, hi).z) * 0.45))
            }
            Cockpit::Bridge | Cockpit::Periscope => {
                let (lo, hi) = bound(0);
                let top = t.model.height.to_f32().max(hi.y);
                (Some(0), Vec3::new(mid(lo, hi).x, top * 0.75 + lift, mid(lo, hi).z + (hi.z - lo.z) * 0.1))
            }
        };
        let eye = match piece.filter(|&i| i < self.pieces.len() && self.pieces[i] != Mat4::ZERO) {
            Some(i) => self.pieces[i].transform_point3(local),
            None => body.transform_point3(Vec3::new(0.0, t.model.height.to_f32() + lift, 0.0)),
        };
        // A submarine's periscope breaks the surface.
        let eye = if self.cockpit == Cockpit::Periscope { Vec3::new(eye.x, eye.y.max(game.sim.world.sea_level.to_f32() + lift), eye.z) } else { eye };
        let (fwd, up) = (body.transform_vector3(Vec3::Z).normalize_or(Vec3::Z), body.transform_vector3(Vec3::Y).normalize_or(Vec3::Y));
        let mut look = if self.cockpit.flies() {
            // Along the nose, a little down to see the ground ahead, turned by the look.
            let nose = fwd;
            let side = nose.cross(Vec3::Y).normalize_or(Vec3::X);
            let down = glam::Quat::from_axis_angle(side, -0.1 + self.look_pitch);
            glam::Quat::from_axis_angle(up, self.look_yaw) * down * nose
        } else {
            let (sy, cy) = self.look_yaw.sin_cos();
            let (sp, cp) = self.look_pitch.sin_cos();
            Vec3::new(sy * cp, sp, cy * cp)
        };
        // The kick of a shot and the jolt of a hit, dying away.
        self.hud.kick *= (-dt * 9.0).exp();
        self.hud.jolt *= (-dt * 7.0).exp();
        if self.reduced_motion {
            self.hud.kick = 0.0;
            self.hud.jolt = 0.0;
        }
        look.y += self.hud.kick;
        // (Phases in f64, reduced to a turn: as smooth in a long game as in a short one.)
        let t_now = (game.sim.tick as f64 + alpha as f64) * 0.37;
        let phase = |k: f64| ((t_now * k) % std::f64::consts::TAU) as f32;
        look += Vec3::new(phase(13.0).sin(), phase(17.0).cos(), 0.0) * self.hud.jolt;
        let look = look.normalize_or(fwd);
        // Aircraft roll with their bank; the rest keep the horizon level.
        self.roll = if self.cockpit.flies() && !self.reduced_motion {
            let right = look.cross(Vec3::Y).normalize_or(Vec3::X);
            let level_up = right.cross(look);
            let tilted = up - look * up.dot(look);
            tilted.dot(right).atan2(tilted.dot(level_up))
        } else {
            0.0
        };
        if self.chase {
            // Behind and above, looking the same way.
            let r = t.radius.to_f32().max(8.0);
            let flat = Vec3::new(look.x, 0.0, look.z).normalize_or(fwd);
            let centre = body.transform_point3(Vec3::new(0.0, t.model.height.to_f32() * 0.5, 0.0));
            self.centre = centre;
            let mut at = centre - flat * (r * 4.0 + 40.0) + Vec3::Y * (r * 1.4 + 18.0);
            // (Over the ground as it is drawn: a building's pad and its bank raise it.)
            let ground = crate::scene::drawn_ground(&game.sim.world, at.x, at.z).max(game.sim.world.sea_level.to_f32());
            at.y = at.y.max(ground + 6.0);
            self.eye = at;
            self.forward = (centre + look * 120.0 - at).normalize_or(look);
        } else {
            self.eye = eye;
            self.forward = look;
        }
        self.fov = self.cockpit.fov().to_radians();
    }

    /// Hits taken, kills scored, shots fired, and the instruments' needles.
    fn watch(&mut self, game: &mut Game, now: f64, dt: f32) {
        let Some(u) = game.sim.unit(self.unit) else { return };
        let t = game.sim.unit_type(u);
        let health = (u.health / t.max_health.max(Fx::ONE)).to_f32().clamp(0.0, 1.0);
        let top = t.max_speed.to_f32().max(0.01);
        // Hits: the way they came from, and a jolt as hard as the blow.
        if health < self.hud.health - 0.001 {
            let blow = ((self.hud.health - health) * 6.0).clamp(0.15, 1.0);
            let from = u.last_attacker.and_then(|a| game.sim.unit(a)).map(|a| Vec3::from_array(a.pos.to_f32()));
            // (The armour takes it from the hull's side.)
            self.hud.armour.hit(self.hud.health - health, from.map_or(0.0, |q| off_heading(u, q)));
            self.hud.hits.push((now, from, blow));
            self.hud.jolt = (self.hud.jolt + blow * 0.02).min(0.05);
            for (mark, text) in [(0.6, ta_ui::i18n::n("HULL DAMAGED")), (0.35, ta_ui::i18n::n("HULL BREACHED")), (0.15, ta_ui::i18n::n("HULL CRITICAL"))] {
                if self.hud.health >= mark && health < mark {
                    self.hud.say(ta_ui::i18n::t(text), now, true);
                }
            }
        }
        self.hud.crack_to(health, u.id.slot, now);
        self.hud.hits.retain(|h| now - h.0 < 2.5);
        self.hud.health = health;
        self.hud.armour.settle(1.0 - health);
        // Kills (the sight's bracket closes up on a target destroyed; the unit's page counts them).
        if u.kills > self.hud.kills {
            self.hud.kills = u.kills;
            self.hud.kill_flash = Some(now);
        }
        // What the guns are on: since when, and how it was last seen; gone dead, its bracket
        // collapses where it was.
        let target = self.target(game);
        if target.is_some_and(|id| self.hud.target.is_none_or(|(old, _)| old != id)) {
            self.hud.target = target.map(|id| (id, now));
        } else if target.is_none() {
            self.hud.target = None;
        }
        if let Some(gone) = self.hud.last_target.take_if(|m| Some(m.id) != target)
            && game.sim.unit(gone.id).is_none_or(|g| g.dead)
        {
            self.hud.destroyed = Some((gone, now));
        }
        if let Some(on) = target.and_then(|id| game.sim.unit(id))
            && let Some(at) = game.scene.unit_position(on.id, game.alpha())
        {
            let mut corners = self.hud.last_target.take().map(|m| m.corners).unwrap_or_default();
            drawn_corners(game, on, at, &mut corners);
            self.hud.last_target = Some(Marked { id: on.id, corners });
        }
        if self.hud.destroyed.as_ref().is_some_and(|d| now - d.1 > 1.0) {
            self.hud.destroyed = None;
        }
        // Shots: a reload starting is a shot; the camera kicks with the gun. A reload over is
        // the weapon ready.
        for s in 0..3 {
            let r = u.weapons[s].reload;
            if r > self.hud.reloads[s] + 1 {
                self.hud.last_shot[s] = now;
                if let Some(w) = t.weapons[s] {
                    let wt = game.sim.weapon_types.get(w);
                    let heavy = (wt.default_damage.to_f32() / 400.0).clamp(0.2, 1.0);
                    self.hud.kick = (self.hud.kick + 0.004 + heavy * 0.012).min(0.03);
                }
            }
            if r == 0 && self.hud.reloads[s] > 0 {
                self.hud.ready_since[s] = now;
            }
            self.hud.reloads[s] = r;
        }
        self.hud.speed.step(u.speed.to_f32() / top, 7.0, 0.55, dt);
        // Turning: the hull's heading a second, against the fastest it turns (right is a
        // smaller heading).
        let heading = u.heading as f32 * std::f32::consts::TAU / 65536.0;
        let fastest = (t.turn_rate.max(1) as f32 * std::f32::consts::TAU / 65536.0 * ta_sim::cob::TICKS_PER_SECOND as f32).max(0.01);
        let turning = self.hud.heading.map_or(0.0, |h| -wrap(heading - h) / dt.max(1e-3) / fastest);
        self.hud.heading = Some(heading);
        self.hud.turn.step(turning.clamp(-1.0, 1.0), 8.0, 0.7, dt);
        self.hud.controls_speed.step(self.hud.speed.x, 3.0, 1.0, dt);
        self.hud.controls_turn.step(self.hud.turn.x, 3.0, 1.0, dt);
        let ground = game.sim.world.height_at(u.pos.x, u.pos.z).max(game.sim.world.sea_level);
        self.hud.altitude.step((u.pos.y - ground).to_f32(), 5.0, 0.8, dt);
        self.hud.sweep = (self.hud.sweep + dt * 2.2) % std::f32::consts::TAU;
        self.hud.messages.retain(|m| now - m.1 < 3.2);
    }

    /// The unit's weapons, for the instruments.
    pub fn weapons(&self, game: &Game) -> Vec<WeaponState> {
        let Some(u) = game.sim.unit(self.unit) else { return Vec::new() };
        let t = game.sim.unit_type(u);
        let has_normal = t.weapons.iter().flatten().any(|&w| !game.sim.weapon_types.get(w).command_fire);
        (0..3)
            .filter_map(|s| {
                let w = game.sim.weapon_types.get(t.weapons[s]?);
                let full = w.reload_ticks.max(1) as f32;
                Some(WeaponState {
                    slot: s,
                    name: weapon_name(w),
                    left: (u.weapons[s].reload as f32 / full).clamp(0.0, 1.0),
                    ready: u.weapons[s].reload == 0,
                    range: w.range.to_f32(),
                    reload: full / ta_sim::cob::TICKS_PER_SECOND as f32,
                    manual: w.command_fire && s != 0 && has_normal,
                })
            })
            .collect()
    }

    /// What the unit's guns are on, else the unit an attack order names: when the viewer can
    /// see it.
    pub fn target(&self, game: &Game) -> Option<UnitId> {
        let u = game.sim.unit(self.unit)?;
        let aimed = u.weapons.iter().find_map(|w| match w.target {
            Some(Target::Unit(id)) => Some(id),
            _ => None,
        });
        let ordered = match u.orders.front() {
            Some(Order::Attack(Target::Unit(id)) | Order::ManualFire(Target::Unit(id))) => Some(*id),
            _ => None,
        };
        aimed.or(ordered).filter(|&id| game.sim.unit(id).is_some_and(|t| !t.dead && game.seen(t) == Seen::Visible))
    }
}

/// A weapon's name as the interface shows it: TA's own, its one misspelling put right (the
/// Anaconda's "Light Guass"), the data left as it is.
pub(crate) fn weapon_name(w: &ta_sim::WeaponType) -> String {
    let name = if w.def.display_name.trim().is_empty() { w.name.trim() } else { w.def.display_name.trim() };
    name.replace("Guass", "Gauss")
}

/// One of the unit's weapons, as the instruments show it.
pub struct WeaponState {
    /// Its place on the unit (0..3).
    pub slot: usize,
    pub name: String,
    /// The share of its reload still to come (0: loaded), and whether it is loaded.
    pub left: f32,
    pub ready: bool,
    /// Its reach (world units) and its reload (seconds).
    pub range: f32,
    pub reload: f32,
    /// Fired only when ordered to (the Commander's D-gun beside its laser).
    pub manual: bool,
}

/// The lip under a modelled cockpit's window, ahead of the eye (eye space, metres): where its
/// own geometry rises highest into the view below the line of sight, across the middle of the
/// view (its moving parts and see-through glass aside). Under it are the instruments.
fn sill(model: &CockpitModel) -> Option<Vec3> {
    model
        .vertices
        .iter()
        .filter(|v| v.part == 0 && model.materials.get(v.material as usize).is_none_or(|m| m.role != ta_render::cockpit::Role::Glass))
        .map(|v| Vec3::from_array(v.pos))
        .filter(|q| q.z < -0.05 && q.y < 0.0 && q.x.abs() < -q.z * 0.3)
        .max_by(|a, b| (a.y / -a.z).total_cmp(&(b.y / -b.z)))
        .map(|q| Vec3::new(0.0, q.y, q.z))
}

/// The top of a modelled cockpit's window over the middle of the view (eye space, metres): where
/// its own geometry comes lowest above the line of sight, across as much of the view as a
/// heading tape `tape` wide (a share of the view's width) spans, with a margin for wide screens
/// (its moving parts and see-through glass aside). Under it is the window.
fn lintel(model: &CockpitModel, tape: f32) -> Option<Vec3> {
    let reach = tape * 1.8;
    model
        .vertices
        .iter()
        .filter(|v| v.part == 0 && model.materials.get(v.material as usize).is_none_or(|m| m.role != ta_render::cockpit::Role::Glass))
        .map(|v| Vec3::from_array(v.pos))
        .filter(|q| q.z < -0.05 && q.y > 0.0 && q.x.abs() < -q.z * reach)
        .min_by(|a, b| (a.y / -a.z).total_cmp(&(b.y / -b.z)))
        .map(|q| Vec3::new(0.0, q.y, q.z))
}

/// Where a turret's guns run out ahead, across it (the turret piece's own x): the middle of the
/// pieces under it that reach within [`GUN_TIE`] of the furthest forward, as the model is built,
/// before any script moves, hides or shows them. None without pieces under it that draw.
fn gun_line(model: &ModelDef, turret: usize, bounds: &[Option<PieceBounds>]) -> Option<f32> {
    // Where a piece under the turret sits in the turret's space: at rest, its offsets alone.
    let place = |mut d: usize| {
        let mut at = Vec3::ZERO;
        loop {
            let p = model.pieces.get(d)?;
            at += Vec3::from_array(p.offset.to_f32());
            match p.parent {
                Some(q) if q == turret => return Some(at),
                Some(q) => d = q,
                None => return None,
            }
        }
    };
    // Each piece's front, and its middle across.
    let pieces: Vec<(f32, f32)> = (0..model.pieces.len())
        .filter_map(|d| {
            let b = bounds.get(d).copied().flatten()?;
            let at = place(d)?;
            Some((at.z + b.max.z, at.x + (b.min.x + b.max.x) * 0.5))
        })
        .collect();
    let front = pieces.iter().map(|p| p.0).reduce(f32::max)?;
    let ahead: Vec<f32> = pieces.iter().filter(|p| p.0 >= front - GUN_TIE).map(|p| p.1).collect();
    Some(ahead.iter().sum::<f32>() / ahead.len() as f32)
}

/// Sets `cam` to look from `eye` along `forward` with a vertical field of view and a roll.
pub fn aim_camera(cam: &mut Camera, eye: Vec3, forward: Vec3, fov: f32, roll: f32) {
    /// The focus sits this close in front of the eye (its distance sets the near plane).
    const REACH: f32 = 16.0;
    let f = forward.normalize_or(Vec3::NEG_Z);
    cam.mode = CameraMode::Perspective;
    cam.fov_y = fov;
    cam.pitch = (-f.y).clamp(-1.0, 1.0).asin();
    cam.yaw = f.x.atan2(-f.z);
    cam.zoom = cam.viewport.y / (2.0 * (fov * 0.5).tan() * REACH);
    cam.focus = eye + f * REACH;
    cam.roll = roll;
    cam.lift = 0.0;
}

pub(crate) fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// An angle brought into -π..π.
fn wrap(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// A body's attitude from its matrix (z forward, y up, x to its left): the nose's pitch, up
/// positive, and the roll, the right side down positive (radians).
pub(crate) fn attitude_of(body: &Mat4) -> (f32, f32) {
    let fwd = body.transform_vector3(Vec3::Z).normalize_or(Vec3::Z);
    let up = body.transform_vector3(Vec3::Y).normalize_or(Vec3::Y);
    let right = fwd.cross(Vec3::Y).normalize_or(Vec3::NEG_X);
    (fwd.y.clamp(-1.0, 1.0).asin(), up.dot(right).atan2(up.y))
}

/// Where a piece points across its body, as seen from above (radians, to the right of the
/// body's heading positive): a turret's azimuth.
pub(crate) fn turret_azimuth(body: &Mat4, piece: &Mat4) -> f32 {
    let (f, b) = (piece.transform_vector3(Vec3::Z), body.transform_vector3(Vec3::Z));
    // (A larger heading is to the left.)
    wrap(b.x.atan2(b.z) - f.x.atan2(f.z))
}

/// Between two directions, `t` of the way round.
fn slerp_dir(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    let q = glam::Quat::from_rotation_arc(a.normalize_or(Vec3::Z), b.normalize_or(Vec3::Z));
    glam::Quat::IDENTITY.slerp(q, t) * a
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The nearest point of the triangle `a`, `b`, `c` to `p` (Ericson's regions).
    fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
        let (ab, ac, ap) = (b - a, c - a, p - a);
        let (d1, d2) = (ab.dot(ap), ac.dot(ap));
        if d1 <= 0.0 && d2 <= 0.0 {
            return a;
        }
        let bp = p - b;
        let (d3, d4) = (ab.dot(bp), ac.dot(bp));
        if d3 >= 0.0 && d4 <= d3 {
            return b;
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            return a + ab * (d1 / (d1 - d3));
        }
        let cp = p - c;
        let (d5, d6) = (ab.dot(cp), ac.dot(cp));
        if d6 >= 0.0 && d5 <= d6 {
            return c;
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            return a + ac * (d2 / (d2 - d6));
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
            return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
        }
        let denom = 1.0 / (va + vb + vc);
        a + ab * (vb * denom) + ac * (vc * denom)
    }

    #[test]
    fn the_tanks_tillers_clear_the_console_however_they_lean() {
        // The shipped tank cockpit: its tillers turned as far forward (full speed) and back
        // (turning hard, reversing) as the game leans them, measured against everything else in
        // it but the windows' glass. (Their grips touched the console upright and went into it
        // leaning forward: they flickered where the two met. The boot at each one's pivot sits
        // on the floor, as built.)
        let bytes = ta_render::cockpit_models().iter().find(|(name, _)| *name == "vehicle").map(|(_, b)| *b).expect("the tank cockpit");
        let model = ta_render::cockpit::CockpitModel::from_glb(bytes).expect("it reads");
        let position = |i: u32| Vec3::from(model.vertices[i as usize].pos);
        let still: Vec<[Vec3; 3]> = model
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|t| {
                let v = &model.vertices[t[0] as usize];
                v.part == 0 && model.materials.get(v.material as usize).is_none_or(|m| m.role != ta_render::cockpit::Role::Glass)
            })
            .map(|t| [position(t[0]), position(t[1]), position(t[2])])
            .collect();
        for name in ["lever_l", "lever_r"] {
            let part = model.part(name).expect("the tillers") as u32;
            let rest = model.parts[part as usize].1;
            // (Above the boot: more than 15 cm up from the pivot.)
            let lever: Vec<Vec3> = model.vertices.iter().filter(|v| v.part == part).map(|v| Vec3::from(v.pos)).filter(|p| p.length() > 0.15).collect();
            assert!(lever.len() > 50, "{name}: {} vertices", lever.len());
            for angle in [-TILLER_DRIVE, -TILLER_DRIVE * 0.5, 0.0, TILLER_TURN, TILLER_TURN + TILLER_DRIVE * 0.2] {
                let m = rest * Mat4::from_rotation_x(angle);
                let points: Vec<Vec3> = lever.iter().map(|&p| m.transform_point3(p)).collect();
                let (lo, hi) = points.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
                let near: Vec<&[Vec3; 3]> = still
                    .iter()
                    .filter(|t| t.iter().fold(Vec3::splat(f32::MAX), |m, v| m.min(*v)).cmple(hi + 0.05).all() && t.iter().fold(Vec3::splat(f32::MIN), |m, v| m.max(*v)).cmpge(lo - 0.05).all())
                    .collect();
                let gap = points.iter().map(|&p| near.iter().map(|t| p.distance(closest_on_triangle(p, t[0], t[1], t[2]))).fold(f32::MAX, f32::min)).fold(f32::MAX, f32::min);
                assert!(gap > 0.015, "{name} at {:.1} degrees: {:.1} mm from the cockpit", angle.to_degrees(), gap * 1000.0);
            }
        }
    }

    #[test]
    fn the_camera_looks_from_the_eye_along_the_view() {
        let mut cam = Camera::new(Vec2::new(1600.0, 900.0));
        for f in [Vec3::new(1.0, -0.2, 0.3), Vec3::new(-0.2, 0.4, -1.0), Vec3::new(0.0, -1.0, 0.01)] {
            let eye = Vec3::new(500.0, 80.0, 700.0);
            aim_camera(&mut cam, eye, f, 1.1, 0.0);
            assert!(cam.eye().distance(eye) < 0.01, "{f}: eye {}", cam.eye());
            let (o, d) = cam.screen_ray(cam.viewport * 0.5);
            assert!(d.dot(f.normalize()) > 0.9999, "{f}: centre ray {d}");
            assert!(o.distance(eye) < 2.0);
        }
    }

    #[test]
    fn a_roll_turns_the_picture() {
        let mut cam = Camera::new(Vec2::new(1000.0, 1000.0));
        aim_camera(&mut cam, Vec3::ZERO, Vec3::Z, 1.0, 0.3);
        // A point to the right of the view goes up the screen as the picture turns
        // anticlockwise.
        let p = cam.view_proj() * glam::Vec4::new(-10.0, 0.0, 100.0, 1.0);
        assert!(p.y / p.w > 0.01, "{p}");
    }

    #[test]
    fn the_sill_is_the_lip_under_the_window() {
        use ta_render::cockpit::{CockpitMaterial, CockpitVertex, Role};
        let v = |pos: [f32; 3], material: u32, part: u32| CockpitVertex { pos, material, part, ..Default::default() };
        let material = CockpitMaterial::plain;
        let model = CockpitModel {
            vertices: vec![
                // The dashboard, lower in the view; the lip under the window, rising highest.
                v([0.0, -0.3, -0.6], 0, 0),
                v([0.05, -0.1, -0.8], 0, 0),
                // Not it: glass (seen through), a lever (a moving part), a pillar off to the
                // side, the roof above the line of sight.
                v([0.0, -0.02, -0.5], 1, 0),
                v([0.0, -0.01, -0.4], 0, 1),
                v([0.6, -0.05, -0.5], 0, 0),
                v([0.0, 0.2, -0.5], 0, 0),
            ],
            materials: vec![material(Role::Surface), material(Role::Glass)],
            ..Default::default()
        };
        assert_eq!(sill(&model), Some(Vec3::new(0.0, -0.1, -0.8)));
        assert_eq!(sill(&CockpitModel::default()), None);
    }

    #[test]
    fn the_lintel_is_the_top_of_the_window() {
        use ta_render::cockpit::{CockpitMaterial, CockpitVertex, Role};
        let v = |pos: [f32; 3], material: u32, part: u32| CockpitVertex { pos, material, part, ..Default::default() };
        let material = CockpitMaterial::plain;
        let model = CockpitModel {
            vertices: vec![
                // The roof over the middle; the frame coming lower towards the side, still over
                // the heading tape's span.
                v([0.0, 0.3, -0.6], 0, 0),
                v([0.3, 0.2, -0.6], 0, 0),
                // Not it: glass, a moving part, a pillar off to the side, the dashboard below.
                v([0.0, 0.05, -0.6], 1, 0),
                v([0.0, 0.04, -0.6], 0, 1),
                v([0.7, 0.01, -0.6], 0, 0),
                v([0.0, -0.3, -0.6], 0, 0),
            ],
            materials: vec![material(Role::Surface), material(Role::Glass)],
            ..Default::default()
        };
        assert_eq!(lintel(&model, TAPE_WIDTH), Some(Vec3::new(0.0, 0.2, -0.6)));
        assert_eq!(lintel(&CockpitModel::default(), TAPE_WIDTH), None);
        // The cockpits without a heading screen of their own leave the tape room under their
        // window's top, well inside the view.
        for k in [Cockpit::Canopy, Cockpit::Gunship] {
            let Some(m) = k.model() else { continue };
            assert!(!m.screens.iter().any(|s| s.name == "compass"));
            let up = lintel(&m, k.tape_width()).map(|q| (q.y / -q.z).atan().to_degrees());
            assert!(up.is_some_and(|a| (15.0..k.fov() * 0.5 - 4.0).contains(&a)), "{k:?}: {up:?}");
        }
    }

    #[test]
    fn a_body_reads_its_pitch_and_roll_from_its_matrix() {
        let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4;
        assert!(near(attitude_of(&Mat4::IDENTITY), (0.0, 0.0)));
        // The nose up: a turn about x raises z (a body's x is to its left).
        assert!(near(attitude_of(&Mat4::from_rotation_x(-0.3)), (0.3, 0.0)), "{:?}", attitude_of(&Mat4::from_rotation_x(-0.3)));
        assert!(near(attitude_of(&Mat4::from_rotation_x(0.3)), (-0.3, 0.0)));
        // The right side down: the body's up leans to its right, which is -x.
        let banked = attitude_of(&Mat4::from_rotation_z(0.4));
        assert!(banked.0.abs() < 1e-4 && (banked.1 - 0.4).abs() < 1e-4, "{banked:?}");
        // Turning about the vertical changes neither.
        assert!(near(attitude_of(&Mat4::from_rotation_y(1.2)), (0.0, 0.0)));
    }

    #[test]
    fn a_turret_reads_its_azimuth_across_the_hull() {
        let hull = Mat4::from_rotation_y(0.8);
        assert!(turret_azimuth(&hull, &hull).abs() < 1e-5);
        // A larger heading is to the left: the turret turned that way reads negative (right
        // positive), and the other way positive.
        assert!((turret_azimuth(&hull, &(hull * Mat4::from_rotation_y(0.5))) + 0.5).abs() < 1e-4);
        assert!((turret_azimuth(&hull, &(hull * Mat4::from_rotation_y(-0.5))) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn blows_wear_the_sections_that_face_them() {
        let mut front = Armour::default();
        front.hit(0.2, 0.0);
        let i = front.integrity();
        assert!(i[1] < i[4] && i[1] < i[6], "{i:?}");
        let mut right = Armour::default();
        right.hit(0.2, std::f32::consts::FRAC_PI_2);
        let i = right.integrity();
        assert!(i[5] < i[4] && i[3] < i[2] && i[7] < i[6], "{i:?}");
        // The left is the right's mirror.
        let mut left = Armour::default();
        left.hit(0.2, -std::f32::consts::FRAC_PI_2);
        let (l, r) = (left.integrity(), right.integrity());
        assert!((l[4] - r[5]).abs() < 1e-5 && (l[2] - r[3]).abs() < 1e-5 && (l[6] - r[7]).abs() < 1e-5);
    }

    #[test]
    fn the_armours_total_follows_the_hull() {
        let sum = |a: &Armour| a.lost.iter().sum::<f32>();
        let mut a = Armour::worn(0.1);
        assert!((sum(&a) - 0.1).abs() < 1e-6);
        a.hit(0.3, 1.0);
        assert!((sum(&a) - 0.4).abs() < 1e-5);
        // Repaired to 80%: the loss is 0.2 again, spread as it lay.
        a.settle(0.2);
        assert!((sum(&a) - 0.2).abs() < 1e-5);
        a.settle(0.0);
        assert!(a.integrity().iter().all(|&v| v == 1.0));
        // From whole, a loss the hits did not see spreads by the shares.
        a.settle(0.5);
        assert!(a.integrity().iter().all(|&v| (v - 0.5).abs() < 1e-5));
    }

    #[test]
    fn a_point_lies_off_the_heading_to_the_right_or_the_left() {
        let mut u = Unit::new(UnitId { slot: 0, generation: 0 }, 0, 0, ta_sim::FxVec3::from_ints(0, 0, 0));
        u.heading = 0;
        // Facing +z; the unit's right is -x.
        assert!(off_heading(&u, Vec3::new(0.0, 0.0, 9.0)).abs() < 1e-5);
        assert!((off_heading(&u, Vec3::new(-9.0, 0.0, 0.0)) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!((off_heading(&u, Vec3::new(9.0, 0.0, 0.0)) + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!((off_heading(&u, Vec3::new(0.0, 0.0, -9.0)).abs() - std::f32::consts::PI).abs() < 1e-5);
    }

    #[test]
    fn the_glass_cracks_as_the_hull_falls_and_is_whole_again_mended() {
        let mut hud = HudState::default();
        hud.crack_to(0.8, 7, 1.0);
        assert!(hud.cracks.is_empty());
        // Under half: one crack; under a quarter, another; nearly gone, a third.
        hud.crack_to(0.45, 7, 2.0);
        assert_eq!(hud.cracks.len(), 1);
        hud.crack_to(0.2, 7, 3.0);
        hud.crack_to(0.05, 7, 4.0);
        assert_eq!(hud.cracks.len(), 3);
        // Each came when the hull fell past its mark (it grows from then), and a crack already
        // there stays as it was.
        assert_eq!(hud.cracks.iter().map(|c| c.since).collect::<Vec<_>>(), [2.0, 3.0, 4.0]);
        // The same unit's glass cracks in the same places; another's elsewhere. Each is to a side
        // of the sight in the middle, the first two on opposite sides.
        let mut again = HudState::default();
        again.crack_to(0.05, 7, 9.0);
        let at = |h: &HudState| h.cracks.iter().map(|c| c.at).collect::<Vec<_>>();
        assert_eq!(at(&hud), at(&again));
        let mut other = HudState::default();
        other.crack_to(0.05, 8, 9.0);
        assert_ne!(at(&hud), at(&other));
        for c in &hud.cracks {
            assert!((0.25..0.41).contains(&c.at.x) || (0.59..0.75).contains(&c.at.x), "{:?}", c.at);
        }
        assert!((hud.cracks[0].at.x < 0.5) != (hud.cracks[1].at.x < 0.5));
        // Mended past a mark, the newest goes.
        hud.crack_to(0.3, 7, 5.0);
        assert_eq!(hud.cracks.len(), 1);
    }

    #[test]
    fn needles_settle() {
        let mut n = Needle::default();
        for _ in 0..200 {
            n.step(0.7, 8.0, 0.6, 1.0 / 60.0);
        }
        assert!((n.x - 0.7).abs() < 0.001);
    }

    /// What a piece draws: the corners of its box, if anything.
    type Drawn = Option<([f32; 3], [f32; 3])>;

    /// A model from its pieces (parent, offset, what it draws), and its pieces' bounds.
    fn model(pieces: &[(Option<usize>, [f32; 3], Drawn)]) -> (ModelDef, Vec<Option<PieceBounds>>) {
        let def = ModelDef {
            pieces: pieces
                .iter()
                .enumerate()
                .map(|(i, &(parent, [x, y, z], _))| ta_sim::defs::PieceDef { name: format!("p{i}"), parent, offset: ta_sim::FxVec3::new(Fx::from_f32(x), Fx::from_f32(y), Fx::from_f32(z)), top: Fx::ZERO })
                .collect(),
            ..Default::default()
        };
        let bounds = pieces.iter().map(|p| p.2.map(|(min, max)| PieceBounds { min: Vec3::from_array(min), max: Vec3::from_array(max) })).collect();
        (def, bounds)
    }

    const BARREL: Drawn = Some(([-1.0, -1.0, 0.0], [1.0, 1.0, 18.0]));
    const FLARE: Drawn = Some(([-0.5, -0.5, -0.5], [0.5, 0.5, 0.5]));
    const BOX: Drawn = Some(([-8.0, -3.0, -8.0], [8.0, 3.0, 8.0]));

    #[test]
    fn the_eye_sits_between_twin_barrels() {
        // A Bulldog's: twin barrels on a sleeve, their flashes not quite level.
        let (def, bounds) = model(&[
            (None, [0.0, 0.0, 0.0], BOX),
            (Some(0), [0.0, 15.0, 11.0], BOX),
            (Some(1), [0.0, 0.0, 1.0], None),
            (Some(2), [5.5, 2.0, 6.0], BARREL),
            (Some(3), [-0.15, 0.2, 17.8], FLARE),
            (Some(2), [-5.5, 2.0, 6.0], BARREL),
            (Some(5), [0.0, 0.25, 17.25], FLARE),
        ]);
        let x = gun_line(&def, 1, &bounds).unwrap();
        assert!(x.abs() < 0.1, "between the barrels, not over one: {x}");
    }

    #[test]
    fn the_eye_sits_over_a_lone_barrel() {
        let (def, bounds) = model(&[(None, [0.0; 3], BOX), (Some(0), [0.0, 12.0, 4.0], BOX), (Some(1), [3.0, 1.0, 5.0], BARREL), (Some(2), [0.0, 0.0, 18.0], FLARE)]);
        let x = gun_line(&def, 1, &bounds).unwrap();
        assert!((x - 3.0).abs() < 0.01, "over the barrel: {x}");
    }

    #[test]
    fn a_shorter_gun_or_one_off_the_turret_leaves_the_eye_over_the_barrel() {
        let (def, bounds) = model(&[
            (None, [0.0; 3], BOX),
            (Some(0), [0.0, 12.0, 4.0], BOX),
            (Some(1), [3.0, 1.0, 5.0], BARREL),
            // A machine gun beside it, well short of its muzzle.
            (Some(1), [-4.0, 1.0, 0.0], BARREL),
            // On the hull, reaching furthest of all: not the turret's.
            (Some(0), [-6.0, 4.0, 30.0], BARREL),
        ]);
        let x = gun_line(&def, 1, &bounds).unwrap();
        assert!((x - 3.0).abs() < 0.01, "over the main barrel: {x}");
    }

    #[test]
    fn a_turret_with_nothing_under_it_has_no_gun_line() {
        let (def, bounds) = model(&[(None, [0.0; 3], BOX), (Some(0), [0.0, 12.0, 4.0], BOX), (Some(1), [0.0, 1.0, 5.0], None)]);
        assert_eq!(gun_line(&def, 1, &bounds), None);
    }

    /// The installed Bulldog, whose eye hopped between its barrels as they recoiled in turn.
    #[test]
    fn the_bulldogs_eye_sits_between_its_barrels() {
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        let mut vfs = ta_vfs::Vfs::new();
        if vfs.mount_game_dir(&install.root, "base").is_err() {
            return;
        }
        let data = Arc::new(ta_data::GameData::load(&vfs));
        let types = ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p));
        let t = types.types.iter().find(|t| t.name.eq_ignore_ascii_case("ARMBULL")).expect("the Bulldog");
        let bytes = vfs.read_opt(&format!("objects3d/{}.3do", t.def.object.to_ascii_lowercase())).expect("its model");
        let bounds = PieceBounds::of(&ta_formats::model3do::Model3do::parse(&bytes).unwrap());
        let turret = t.model.pieces.iter().position(|p| p.name.eq_ignore_ascii_case("turret")).expect("its turret");
        let x = gun_line(&t.model, turret, &bounds).unwrap();
        // (Its barrels stand 5.5 either side of the middle.)
        assert!(x.abs() < 1.0, "between the barrels: {x}");
    }
}
