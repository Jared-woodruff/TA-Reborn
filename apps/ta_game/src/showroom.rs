//! The encyclopaedia's showroom: the unit on show standing in a TA world (a battle with no
//! one in it and everything in sight), switched on so it does what it does at rest (or going to
//! and fro, or firing), in its side's colours, the camera turning round it, fitted to the unit
//! as it is drawn; and its wallpaper, the view the preview shows drawn large, with the unit's
//! name and numbers laid out over it.

use std::f32::consts::{PI, TAU};

use egui::{Color32, FontId, Pos2, Rect, pos2, vec2};
use glam::{Mat4, Vec2, Vec3};
use ta_render::post::{DepthOfField, DofMode};
use ta_render::{Camera, WaterSettings};
use ta_sim::{Cheat, Command, Fx, FxVec3, Locomotion, Order, Target, UnitId, UnitType};
use ta_ui::i18n::{n, t, tf};

use crate::game::Game;
use crate::setup::{Controller, GameSetup, SlotSetup};
use crate::theme::*;

/// The showroom's lens (vertical): the battle's own.
const FOV: f32 = 30.0 * PI / 180.0;

/// The view the showroom starts from, and goes back to: in front of the unit and to its left, a
/// little above, as TA's build pictures have it (yaw and pitch, radians; the units are put on
/// show facing south). A unit at a wallpaper's right looks in towards its words.
pub const HOME: (f32, f32) = (-0.6, 0.32);

/// The showroom as it is: the world, the unit on show and the camera's place round it.
pub struct Showroom {
    pub map: String,
    pub unit_name: String,
    pub unit: Option<UnitId>,
    pub hours: f32,
    /// The unit's middle as drawn (where the camera looks); its middle when it was put on
    /// show, and where it stood then (the camera follows it from there, should it move).
    pub centre: Vec3,
    pub ground_centre: Vec3,
    pub placed: Vec3,
    /// How far the unit reaches out from its middle's upright axis, and how tall it stands,
    /// as drawn: what the camera frames, wherever it turns.
    pub reach: f32,
    pub height: f32,
    /// How the sea is staged for it.
    pub sea: Sea,
    /// Frames drawn since it was put on show.
    pub frames: u32,
    pub yaw: f32,
    pub pitch: f32,
    /// How far the camera stands, as a multiple of the framing's.
    pub distance: f32,
    /// The camera glides back to the view it starts from ([`HOME`], as near as it was).
    pub homing: bool,
    /// What the unit on show is doing.
    pub act: Act,
}

impl Showroom {
    pub fn new(map: &str, hours: f32) -> Showroom {
        Showroom {
            map: map.to_string(),
            unit_name: String::new(),
            unit: None,
            hours,
            centre: Vec3::ZERO,
            ground_centre: Vec3::ZERO,
            placed: Vec3::ZERO,
            reach: 30.0,
            height: 30.0,
            sea: Sea::Open,
            frames: 0,
            yaw: HOME.0,
            pitch: HOME.1,
            distance: 1.0,
            homing: false,
            act: Act::Stand,
        }
    }

    /// The radius of a ball round the unit as drawn.
    pub fn radius(&self) -> f32 {
        self.reach.hypot(self.height * 0.5)
    }
}

/// How the sea is staged for the unit on show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sea {
    /// As TA's worlds have it (a unit on land; in battle).
    Open,
    /// Clearer and calmer, round a ship's hull below the waterline.
    Hull,
    /// Clear and calm over a unit under the sea (a submarine, a building on the sea bed),
    /// which murky water lit by the sky would otherwise hide.
    Under,
}

/// The sea's appearance for the showroom's `sea` (TA's own unless something is under it).
pub fn stage_water(water: &mut WaterSettings, sea: Sea) {
    let open = WaterSettings::default();
    let (clarity, opacity, reflectivity, foam, waves) = match sea {
        Sea::Open => (open.clarity, open.opacity, open.reflectivity, open.foam, open.wave_strength),
        Sea::Hull => (70.0, 0.5, 0.2, 0.6, 0.35),
        Sea::Under => (320.0, 0.14, 0.06, 0.2, 0.12),
    };
    water.clarity = clarity;
    water.opacity = opacity;
    water.reflectivity = reflectivity;
    water.foam = foam;
    water.wave_strength = waves;
}

/// The colour a side's units are shown in: TA's own, blue for the Arm and red for the Core; a
/// mod's sides each take one of TA's other colours that read well on a unit (green, purple,
/// yellow, light blue), always the same one.
pub fn side_colour(side: &str) -> u8 {
    let side = side.trim().to_ascii_uppercase();
    match side.as_str() {
        "ARM" => 0,
        "CORE" => 1,
        other => {
            let hash = other.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
            [3, 5, 6, 8][(hash % 4) as usize]
        }
    }
}

/// A battle with no one in it on `map`: one player, whose units are on show, no computer
/// players, every place in sight. The player goes on without a Commander ([`place`] takes it
/// away), or the unit on show would blow up with the rest of a defeated army.
pub fn setup(map: &str) -> GameSetup {
    let rules = ta_sim::GameRules { fog_of_war: false, commander_ends_game: false, ..Default::default() };
    GameSetup {
        map: map.to_string(),
        players: vec![SlotSetup { side: "ARM".into(), color: 0, controller: Controller::Human, start: 0, team: 0, name: String::new() }],
        // (Watched: a player alone would have won at once.)
        local_player: None,
        seed: 7,
        mission: None,
        rules,
        conquest: None,
        scenario: None,
    }
}

/// Why a unit could not be put on show.
#[derive(Debug, PartialEq, Eq)]
pub enum Missing {
    /// No such unit in the game data.
    Unit,
    /// It needs the sea and this world has none deep enough.
    Sea,
}

/// A unit put on show: where its middle is, how far it reaches round its middle and how tall
/// it stands, as drawn, and how the sea is staged for it.
pub struct Placed {
    pub id: UnitId,
    pub centre: Vec3,
    pub reach: f32,
    pub height: f32,
    pub sea: Sea,
}

/// Ticks a unit put on show runs before it is framed (six seconds): long enough for it to
/// switch on and for an aircraft, which takes off at first as one does, to set down idle.
const SETTLE: u32 = 180;

/// The showroom's one player (the viewer only watches).
const PLAYER: u8 = 0;

/// Commands from the showroom itself, which a watcher may not give.
fn give(game: &mut Game, commands: Vec<Command>) {
    let spectator = std::mem::replace(&mut game.spectator, false);
    for c in commands {
        game.issue(c);
    }
    game.spectator = spectator;
}

/// Puts the unit of type `name` on show in place of what was there, in its side's colours: on
/// a level stretch clear of rocks and trees as near the middle of the map as there is one (a
/// ship at sea, a unit under it in the shallowest sea deep enough), facing the camera's way,
/// switched on and settled.
pub fn place(game: &mut Game, name: &str) -> Result<Placed, Missing> {
    let type_id = game.sim.types.id(name).ok_or(Missing::Unit)?;
    let ids: Vec<UnitId> = game.sim.units().map(|u| u.id).collect();
    for id in ids {
        game.sim.remove_unit(id);
    }
    let (wet, sea, depth, reach, radius, height, side) = {
        let ty = game.sim.types.get(type_id);
        let wet = ty.def.min_water_depth > 0.0 || ty.locomotion == Locomotion::Naval;
        let sea = match (wet, ty.def.floater) {
            (false, _) => Sea::Open,
            (true, true) => Sea::Hull,
            (true, false) => Sea::Under,
        };
        // (As deep as it needs, with a little to spare: a submarine runs its waterline down.)
        let depth = (ty.def.min_water_depth.max(ty.def.water_line) as f32 + 8.0).max(25.0);
        (wet, sea, depth, 48.0 + ty.footprint.0.max(ty.footprint.1) as f32 * 8.0, ty.radius.to_f32().max(10.0), ty.model.height.to_f32(), ty.def.side.clone())
    };
    let spot = level_spot(game, wet.then_some((depth, sea == Sea::Under)), reach).ok_or(Missing::Sea)?;
    // In its side's colours (the scene reads them each frame).
    if let Some(p) = game.sim.players.get_mut(PLAYER as usize) {
        p.color = side_colour(&side);
    }
    let at = FxVec3::new(Fx::from_f32(spot.x), Fx::ZERO, Fx::from_f32(spot.y));
    // A clear stage: no rock, tree or crystal between the camera, wherever it goes round, and
    // the unit.
    game.sim.clear_features(at, Fx::from_f32(radius * 9.0 + 120.0));
    // (TA's headings: 0 faces south, towards the camera's start.)
    let id = game.sim.spawn_unit(type_id, PLAYER, at, 0, true).ok_or(Missing::Unit)?;
    // Switched on (solar panels open, radar dishes turn), its stores always full and its own
    // shots harmless to it, whatever it is set doing.
    give(game, vec![
        Command::SetActive { player: PLAYER, units: vec![id], on: true },
        Command::Cheat { player: PLAYER, cheat: Cheat::Unlimited(true) },
        Command::Cheat { player: PLAYER, cheat: Cheat::Invulnerable(true) },
    ]);
    // Settled before it is framed: switched on (a radar's dish raised, a solar collector's
    // panels open), an aircraft landed.
    game.fast_forward(SETTLE);
    // Framed as drawn: round its pieces as they stand; the type's size if its model is not
    // loaded.
    let corners = drawn_corners(game, id);
    let (lo, hi) = if corners.is_empty() {
        let ground = game.sim.unit(id).map_or(0.0, |u| u.pos.y.to_f32());
        let height = if height > 1.0 { height } else { radius };
        (Vec3::new(spot.x - radius, ground, spot.y - radius), Vec3::new(spot.x + radius, ground + height, spot.y + radius))
    } else {
        corners.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), &p| (lo.min(p), hi.max(p)))
    };
    let centre = (lo + hi) * 0.5;
    // How far it reaches round its middle's upright axis: its pieces' furthest corner (a radar
    // dish's arms reach out less far than the corners of the box round them all).
    let reach = corners.iter().map(|p| (p.x - centre.x).hypot(p.z - centre.z)).fold(0.0f32, f32::max);
    let reach = if corners.is_empty() { radius } else { reach };
    // (Small ones are framed as a unit 10 across at the least: closer, the ground blurs.)
    Ok(Placed { id, centre, reach: reach.max(10.0), height: (hi.y - lo.y).max(8.0), sea })
}

/// What the unit on show does on its stage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Act {
    /// It stands switched on, doing what it does at rest.
    #[default]
    Stand,
    /// It goes to and fro: walks, drives, sails or flies.
    Move,
    /// It fires its weapons at the ground beside it.
    Fire,
}

impl Act {
    pub const ALL: [Act; 3] = [Act::Stand, Act::Move, Act::Fire];

    pub fn label(self) -> &'static str {
        match self {
            Act::Stand => n("STAND"),
            Act::Move => n("MOVE"),
            Act::Fire => n("FIRE"),
        }
    }

    /// Whether a unit of type `ty` can do it.
    pub fn possible(self, game: &Game, ty: &UnitType) -> bool {
        match self {
            Act::Stand => true,
            Act::Move => ty.max_speed > Fx::ZERO,
            Act::Fire => ground_range(game, ty).is_some(),
        }
    }
}

/// How far the unit's longest-reaching weapon fires at the ground (one it fires itself, not a
/// D-gun's, at aircraft only, or held for a stockpile); none when it has no such weapon.
fn ground_range(game: &Game, ty: &UnitType) -> Option<f32> {
    ty.weapons
        .iter()
        .flatten()
        .map(|&w| game.sim.weapon_types.get(w))
        .filter(|w| w.hits_ground && !w.to_air && !w.command_fire && !w.def.stockpile && w.range > Fx::ZERO)
        .map(|w| w.range.to_f32())
        .reduce(f32::max)
}

/// The unit on show set doing `act`, from where it was put on show: going to and fro across the
/// camera's view (aircraft on a long leg, to get up to speed), or firing at the ground across it
/// (within sight of the camera, however far its guns reach); standing goes back to its place.
pub fn act(game: &mut Game, room: &mut Showroom, act: Act) {
    room.act = act;
    let Some(u) = room.unit.and_then(|id| game.sim.unit(id)) else { return };
    let (id, ty) = (u.id, game.sim.unit_type(u));
    if !act.possible(game, ty) {
        return;
    }
    let home = FxVec3::new(Fx::from_f32(room.placed.x), Fx::ZERO, Fx::from_f32(room.placed.z));
    let across = |d: f32| home + FxVec3::new(Fx::from_f32(d), Fx::ZERO, Fx::ZERO);
    let orders = match act {
        Act::Stand if ty.max_speed > Fx::ZERO => vec![Order::Move(home)],
        Act::Stand => vec![Order::Stop],
        Act::Move => {
            let leg = if ty.locomotion == Locomotion::Air { 900.0 } else { room.reach * 4.0 + 120.0 };
            vec![Order::Patrol(across(leg)), Order::Patrol(home)]
        }
        Act::Fire => {
            let range = ground_range(game, ty).unwrap_or(200.0);
            let mut at = across(room.reach + (range * 0.6).clamp(80.0, 320.0));
            let ground = game.sim.world.height_at(at.x, at.z).max(game.sim.world.sea_level);
            at.y = ground;
            vec![Order::Attack(Target::Ground(at))]
        }
    };
    let commands = orders.into_iter().enumerate().map(|(i, order)| Command::Order { player: PLAYER, units: vec![id], order, queue: i > 0 }).collect();
    give(game, commands);
}

/// The corners of the boxes round a unit's pieces as they are posed now (hidden ones left
/// out), in the world; none before its model is loaded.
fn drawn_corners(game: &mut Game, id: UnitId) -> Vec<Vec3> {
    let mut corners = Vec::new();
    let Some(type_id) = game.sim.unit(id).map(|u| u.type_id) else { return corners };
    let mut pieces = Vec::new();
    if game.scene.pose(&game.sim, id, 1.0, &mut pieces).is_none() {
        return corners;
    }
    let Some(bounds) = game.scene.piece_bounds(type_id) else { return corners };
    for (matrix, b) in pieces.iter().zip(bounds.iter()) {
        let Some(b) = b else { continue };
        if *matrix == Mat4::ZERO {
            continue;
        }
        for k in 0..8 {
            let corner = Vec3::new(if k & 1 == 0 { b.min.x } else { b.max.x }, if k & 2 == 0 { b.min.y } else { b.max.y }, if k & 4 == 0 { b.min.z } else { b.max.z });
            corners.push(matrix.transform_point3(corner));
        }
    }
    corners
}

/// The level stretch (`reach` round it within 10 of one height) clear of rocks and trees
/// nearest the map's middle: on land above the sea, or at sea at least so deep (`wet`, and
/// whether the unit is under the sea: then in the shallowest sea that deep, where the water
/// over it is clearest). On land, on the map's own plain ground if it can: at the height most of
/// its land lies at, not on a mesa's top (Painted Desert's middle is one, a pale, crazed
/// lakebed that washed the units out) or in a pit.
fn level_spot(game: &Game, wet: Option<(f32, bool)>, reach: f32) -> Option<Vec2> {
    let sim = &game.sim;
    let (w, h) = sim.world.size_px();
    let (w, h) = (w.to_f32(), h.to_f32());
    let sea = sim.world.sea_level.to_f32();
    let height = |x: f32, z: f32| sim.world.height_at(Fx::from_f32(x), Fx::from_f32(z)).to_f32();
    let features: Vec<(f32, f32)> = sim.features.iter().filter(|f| f.alive).map(|f| (f.pos.x.to_f32(), f.pos.z.to_f32())).collect();
    let middle = Vec2::new(w / 2.0, h / 2.0);
    let mut best: Option<(Vec2, f32)> = None;
    let step = 64.0;
    // (Under the sea: a bed level enough to stand on, the shallower the better, then the
    // nearer the middle.)
    let shallow = |lo: f32| match wet {
        Some((depth, true)) => ((sea - depth - lo).max(0.0) / 16.0).floor() * 1.0e9,
        _ => 0.0,
    };
    // The plain: the height band (8 units) most of the land above the sea lies in.
    let plain = {
        let mut bands = [0u32; 32];
        let mut z = 16.0;
        while z < h {
            let mut x = 16.0;
            while x < w {
                let y = height(x, z);
                if y > sea + 2.0 {
                    bands[((y / 8.0) as usize).min(31)] += 1;
                }
                x += 32.0;
            }
            z += 32.0;
        }
        bands.iter().enumerate().max_by_key(|&(_, &n)| n).filter(|&(_, &n)| n > 0).map(|(i, _)| i as f32 * 8.0 + 4.0)
    };
    // (Off the plain: behind every spot on it.)
    let off_plain = |lo: f32, hi: f32| match (wet, plain) {
        (None, Some(p)) if (lo + hi) * 0.5 < p - 12.0 || (lo + hi) * 0.5 > p + 12.0 => 1.0e12,
        _ => 0.0,
    };
    let mut z = reach + 96.0;
    while z < h - reach - 96.0 {
        let mut x = reach + 96.0;
        while x < w - reach - 96.0 {
            let samples = (0..=4).flat_map(|i| (0..=4).map(move |j| (i as f32 / 2.0 - 1.0, j as f32 / 2.0 - 1.0))).map(|(dx, dz)| height(x + dx * reach, z + dz * reach));
            let (lo, hi) = samples.fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
            let fits = match wet {
                Some((depth, under)) => hi < sea - depth && (!under || hi - lo < 24.0),
                None => lo > sea + 2.0 && hi - lo < 10.0,
            };
            let clear = !features.iter().any(|&(fx, fz)| (fx - x).abs() < reach + 40.0 && (fz - z).abs() < reach + 40.0);
            if fits && clear {
                let d = Vec2::new(x, z).distance_squared(middle) + shallow(lo) + off_plain(lo, hi);
                if best.is_none_or(|b| d < b.1) {
                    best = Some((Vec2::new(x, z), d));
                }
            }
            x += step;
        }
        z += step;
    }
    best.map(|b| b.0)
}

/// How much of the stage the unit fills, across or down, whichever it fills first.
const FILL: f32 = 0.66;

/// How much of its part of a wallpaper the unit fills (see [`wallpaper_area`]).
const WALLPAPER_FILL: f32 = 0.9;

/// Where on the screen the unit on show stands: the part of it the unit fills (pixels), its
/// middle on that part's middle, `fill` of it across or down (whichever it fills first,
/// wherever the camera turns round it).
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub area: Rect,
    pub fill: f32,
}

impl Frame {
    /// The stage the panels leave it (pixels).
    pub fn stage(stage: Rect) -> Frame {
        Frame { area: stage, fill: FILL }
    }

    /// A wallpaper `picture` large (pixels), with its words or without (see [`wallpaper_area`]).
    pub fn wallpaper(picture: Rect, words: bool) -> Frame {
        Frame { area: wallpaper_area(picture, words), fill: WALLPAPER_FILL }
    }
}

/// The camera's zoom (pixels a world unit at the unit) that fits the unit as drawn into `fill`
/// of `area` (pixels) from `pitch`, wherever the camera turns round it: as wide as it reaches
/// round its middle, as tall as its height foreshortened and the depth of that reach.
fn fitted_zoom(room: &Showroom, area: Rect, fill: f32, pitch: f32) -> f32 {
    let (sin, cos) = pitch.sin_cos();
    let across = 2.0 * room.reach;
    let down = room.height * cos + 2.0 * room.reach * sin;
    (area.width().max(80.0) * fill / across).min(area.height().max(80.0) * fill / down)
}

/// The camera round the unit, framing it as `frame` says: `turning` turns it slowly; dragging
/// (`drag`, pixels) looks round, the wheel (`wheel`) comes closer, either letting go of a glide
/// back to the view it starts from. It eases towards where it should be.
pub fn update_camera(room: &mut Showroom, camera: &mut Camera, frame: Frame, dt: f32, turning: bool, drag: Vec2, wheel: f32) {
    let k = 1.0 - (-dt * 5.0).exp();
    if drag != Vec2::ZERO || wheel != 0.0 {
        room.homing = false;
    }
    if room.homing {
        // (The nearer way round.)
        let turn = (HOME.0 - room.yaw + PI).rem_euclid(TAU) - PI;
        room.yaw += turn * k;
        room.pitch += (HOME.1 - room.pitch) * k;
        room.distance += (1.0 - room.distance) * k;
        if turn.abs() < 1e-3 && (HOME.1 - room.pitch).abs() < 1e-3 && (1.0 - room.distance).abs() < 1e-3 {
            room.homing = false;
        }
    } else if turning && drag == Vec2::ZERO {
        room.yaw += dt * 0.22;
    }
    room.yaw += drag.x * 0.008;
    room.pitch = (room.pitch + drag.y * 0.005).clamp(0.06, 1.25);
    room.distance = (room.distance * (-wheel * 0.14).exp()).clamp(0.45, 3.0);
    camera.mode = ta_render::CameraMode::Perspective;
    camera.roll = 0.0;
    camera.fov_y = FOV;
    camera.near = None;
    camera.yaw = room.yaw;
    camera.pitch += (room.pitch - camera.pitch) * k;
    let zoom = fitted_zoom(room, frame.area, frame.fill, camera.pitch) / room.distance;
    camera.zoom += (zoom - camera.zoom) * k;
    // The unit in the middle of its part of the screen: the picture shifted there, not turned
    // (it is seen as it would be in the middle, and the camera goes round the unit itself).
    let offset = Vec2::new(frame.area.center().x, frame.area.center().y) - camera.viewport * 0.5;
    camera.view_offset += (offset - camera.view_offset) * k;
    // (Quicker after a moving unit: an aircraft circling.)
    camera.focus += (room.centre - camera.focus) * (1.0 - (-dt * 9.0).exp());
}

/// How much of a picture of the unit it fills (see [`picture_camera`]).
const PICTURE_FILL: f32 = 0.86;

/// The camera for a picture of the unit `size` pixels square (for one without a build
/// picture of its own): from in front and to its left, a little above, as TA's build pictures
/// are, the unit filling it.
pub fn picture_camera(room: &Showroom, camera: &Camera, size: u32) -> Camera {
    let mut c = camera.clone();
    c.mode = ta_render::CameraMode::Perspective;
    c.roll = 0.0;
    c.fov_y = FOV;
    (c.view_offset, c.lift) = (Vec2::ZERO, 0.0);
    (c.yaw, c.pitch) = (HOME.0, 0.5);
    c.focus = room.centre;
    c.viewport = Vec2::splat(size as f32);
    let square = Rect::from_min_size(Pos2::ZERO, vec2(size as f32, size as f32));
    c.zoom = fitted_zoom(room, square, PICTURE_FILL, c.pitch);
    c
}

/// The part of a wallpaper `picture` large the unit fills: the right of it when the words take
/// the left, else its middle.
pub fn wallpaper_area(picture: Rect, words: bool) -> Rect {
    let at = |x: f32, y: f32| picture.min + vec2(picture.width() * x, picture.height() * y);
    if words { Rect::from_min_max(at(0.42, 0.08), at(0.96, 0.92)) } else { Rect::from_min_max(at(0.16, 0.08), at(0.84, 0.92)) }
}

/// The camera for a wallpaper `size` pixels large, `words` on it or not: the view its preview
/// shows (`share` of the window's height, framed by [`Frame::wallpaper`]), drawn at that size.
/// The same eye, its lens narrowed to the preview's part of the window, so the picture is the
/// preview's, only larger.
pub fn wallpaper_camera(room: &Showroom, camera: &Camera, size: (u32, u32), words: bool, share: f32) -> Camera {
    let mut c = camera.clone();
    c.mode = ta_render::CameraMode::Perspective;
    (c.roll, c.near) = (0.0, None);
    c.viewport = Vec2::new(size.0 as f32, size.1 as f32);
    c.fov_y = 2.0 * ((FOV * 0.5).tan() * share.clamp(0.1, 1.0)).atan();
    (c.focus, c.yaw, c.pitch) = (room.centre, room.yaw, room.pitch);
    let frame = Frame::wallpaper(Rect::from_min_size(Pos2::ZERO, vec2(c.viewport.x, c.viewport.y)), words);
    c.zoom = fitted_zoom(room, frame.area, frame.fill, c.pitch) / room.distance;
    c.view_offset = Vec2::new(frame.area.center().x, frame.area.center().y) - c.viewport * 0.5;
    c
}

/// A wallpaper's lens, `camera` its camera (and its preview's, `share` of the window's height):
/// the unit sharp through, the world beyond it falling softly out of focus.
pub fn wallpaper_lens(room: &Showroom, camera: &Camera, share: f32) -> DepthOfField {
    let focus = camera.view_distance(room.centre).max(1.0);
    DepthOfField { mode: DofMode::Lens, focus, blur: 0.004 * share, band: (room.radius() / focus).clamp(0.03, 0.3) }
}

/// The words of a wallpaper, laid out over its picture `rect` (points; laid out for a picture
/// 1080 high, and scaled to this one): the side and kind, the name, what it is, its numbers and
/// its weapons, bottom left over a shade; a small "TA REBORN" bottom right.
pub fn wallpaper_words(painter: &egui::Painter, rect: Rect, game: &Game, ty: &UnitType) {
    let s = rect.height() / 1080.0;
    // The shade the words sit on, and the picture's edges darkened.
    let x = |f: f32| rect.min.x + rect.width() * f;
    ta_ui::widgets::gradient_h(painter, Rect::from_min_max(rect.min, pos2(x(0.30), rect.max.y)), Color32::from_black_alpha(225), Color32::from_black_alpha(185));
    ta_ui::widgets::gradient_h(painter, Rect::from_min_max(pos2(x(0.30), rect.min.y), pos2(x(0.62), rect.max.y)), Color32::from_black_alpha(185), Color32::from_black_alpha(0));
    ta_ui::widgets::gradient_v(painter, Rect::from_min_max(pos2(rect.min.x, rect.max.y - 260.0 * s), rect.max), Color32::from_black_alpha(0), Color32::from_black_alpha(150));
    ta_ui::widgets::gradient_v(painter, Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + 140.0 * s)), Color32::from_black_alpha(110), Color32::from_black_alpha(0));
    let left = rect.min.x + 120.0 * s;
    let width = rect.width() * 0.4;
    let kind = t(crate::encyclopaedia::Kind::of(ty).label());
    let description = ty.def.description.trim();
    let facts = numbers(game, ty);
    let weapons: Vec<String> = ty.weapons.iter().flatten().map(|&w| game.sim.weapon_types.get(w)).map(crate::perspective::weapon_name).collect();
    // Bottom up: the block sits on the lower left, clear of the desktop's icons.
    let name = ty.def.display_name.to_uppercase();
    let title_size = if name.chars().count() > 16 { 84.0 } else { 112.0 };
    let title = painter.layout(name, display(title_size * s), Color32::WHITE, width);
    let mut lines: Vec<(String, FontId, Color32)> = Vec::new();
    if !description.is_empty() && !description.eq_ignore_ascii_case(ty.def.display_name.trim()) {
        lines.push((description.to_string(), body(30.0 * s), TEXT_BRIGHT));
    }
    if !facts.is_empty() {
        lines.push((facts.join("    "), strong(22.0 * s), CREAM));
    }
    if !weapons.is_empty() {
        lines.push((weapons.join("  ·  "), body(22.0 * s), STEEL));
    }
    let side = painter.layout(format!("{}  ·  {}", ty.def.side.to_uppercase(), kind), strong(22.0 * s), AMBER, width);
    let galleys: Vec<_> = lines.into_iter().map(|(text, font, colour)| painter.layout(text, font, colour, width)).collect();
    let height = side.size().y + 10.0 * s + title.size().y + 52.0 * s + galleys.iter().map(|g| g.size().y + 12.0 * s).sum::<f32>();
    let mut y = rect.max.y - 130.0 * s - height;
    painter.galley(pos2(left, y), side.clone(), AMBER);
    y += side.size().y + 10.0 * s;
    ta_ui::metal::chrome(painter, pos2(left, y), title.clone());
    y += title.size().y + 24.0 * s;
    // The brass bar under the name, as on the title screen.
    let bar = Rect::from_min_size(pos2(left + 4.0 * s, y), vec2(120.0 * s, 6.0 * s));
    painter.rect_filled(bar.expand(1.5 * s), 2.0 * s, ta_ui::metal::STEEL_EDGE);
    painter.add(ta_ui::metal::vgrad(bar, AMBER_BRIGHT, AMBER_DIM));
    y += 28.0 * s;
    for g in galleys {
        let h = g.size().y;
        painter.galley_with_override_text_color(pos2(left + 2.0 * s, y + 2.5 * s), g.clone(), Color32::from_black_alpha(170));
        painter.galley(pos2(left, y), g, TEXT_BRIGHT);
        y += h + 12.0 * s;
    }
    // Whose picture it is, small.
    let mark = painter.layout_no_wrap("TA REBORN".to_string(), display(26.0 * s), STEEL);
    let at: Pos2 = rect.max - vec2(mark.size().x + 64.0 * s, mark.size().y + 52.0 * s);
    ta_ui::metal::embossed(painter, at, mark, STEEL);
}

/// A wallpaper with no words: its edges darkened a little, round the unit in its middle.
pub fn wallpaper_vignette(painter: &egui::Painter, rect: Rect) {
    let (w, h) = (rect.width(), rect.height());
    ta_ui::widgets::gradient_v(painter, Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + h * 0.18)), Color32::from_black_alpha(90), Color32::from_black_alpha(0));
    ta_ui::widgets::gradient_v(painter, Rect::from_min_max(pos2(rect.min.x, rect.max.y - h * 0.24), rect.max), Color32::from_black_alpha(0), Color32::from_black_alpha(120));
    ta_ui::widgets::gradient_h(painter, Rect::from_min_max(rect.min, pos2(rect.min.x + w * 0.14, rect.max.y)), Color32::from_black_alpha(80), Color32::from_black_alpha(0));
    ta_ui::widgets::gradient_h(painter, Rect::from_min_max(pos2(rect.max.x - w * 0.14, rect.min.y), rect.max), Color32::from_black_alpha(0), Color32::from_black_alpha(80));
}

/// "HEALTH 3000", "SPEED 1.2", "RANGE 300", "1250 METAL": what a wallpaper says in numbers.
fn numbers(game: &Game, ty: &UnitType) -> Vec<String> {
    let mut out = vec![tf("HEALTH {n}", &[("n", &format!("{:.0}", ty.max_health.to_f32()))])];
    if ty.locomotion != Locomotion::Static {
        out.push(tf("SPEED {n}", &[("n", &format!("{:.1}", ty.def.max_velocity))]));
    }
    let range = ty.weapons.iter().flatten().map(|&w| game.sim.weapon_types.get(w)).filter(|w| !w.command_fire).map(|w| w.range.to_f32()).fold(0.0f32, f32::max);
    if range > 0.0 {
        out.push(tf("RANGE {n}", &[("n", &format!("{range:.0}"))]));
    }
    out.push(tf("{n} METAL", &[("n", &format!("{:.0}", ty.cost_metal.to_f32()))]));
    out.push(tf("{n} ENERGY", &[("n", &format!("{:.0}", ty.cost_energy.to_f32()))]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where `point` lands in `camera`'s picture (pixels).
    fn project(camera: &Camera, point: Vec3) -> Pos2 {
        let clip = camera.view_proj() * point.extend(1.0);
        pos2((clip.x / clip.w + 1.0) * camera.viewport.x * 0.5, (1.0 - clip.y / clip.w) * camera.viewport.y * 0.5)
    }

    /// Points round a unit `reach` across and `tall` high about `centre`, wherever it turns.
    fn round(centre: Vec3, reach: f32, tall: f32) -> Vec<Vec3> {
        (0..16).flat_map(|k| {
            let a = k as f32 * TAU / 16.0;
            [-0.5, 0.5].map(|y| centre + Vec3::new(a.cos() * reach, y * tall, a.sin() * reach))
        }).collect()
    }

    #[test]
    fn showroom_frames_the_unit_between_panels_at_different_dpi_scales() {
        for (width, height, scale) in [(1600.0, 900.0, 1.5), (1920.0, 1080.0, 1.0), (1280.0, 720.0, 2.0)] {
            let logical = Rect::from_min_size(Pos2::ZERO, vec2(width / scale, height / scale));
            let layout = crate::encyclopaedia::showroom_layout(logical, crate::encyclopaedia::DECK);
            let stage = Rect::from_min_max(layout.stage.min * scale, layout.stage.max * scale);
            assert!(stage.width() > width * 0.3, "panels must leave a useful inspection stage");
            let mut camera = Camera::new(Vec2::new(width, height));
            for (reach, tall) in [(30.0, 30.0), (12.0, 90.0), (90.0, 20.0)] {
                let mut room = Showroom::new("test", 12.0);
                room.centre = Vec3::new(1000.0, 50.0, 1000.0);
                (room.reach, room.height) = (reach, tall);
                update_camera(&mut room, &mut camera, Frame::stage(stage), 10.0, false, Vec2::ZERO, 0.0);
                assert!(project(&camera, room.centre).distance(stage.center()) < 0.5, "the unit belongs in the visible stage, not the full window's centre");
                // Whichever way the camera has turned, the unit as drawn keeps to the stage.
                for p in round(room.centre, reach, tall) {
                    assert!(stage.contains(project(&camera, p)), "the unit must fit outside the panel bounds ({reach} by {tall})");
                }
            }
        }
    }

    #[test]
    fn a_wallpaper_is_its_preview_drawn_larger() {
        // A 16:10 window showing a 16:9 preview over its bar, saved at 4K and at 1080p, with the
        // words and without, the camera turned and closer: what lands where in the preview
        // lands at the same place in the picture.
        let window = Vec2::new(1920.0, 1200.0);
        let preview = Rect::from_min_size(pos2(110.0, 40.0), vec2(1600.0, 900.0));
        let share = preview.height() / window.y;
        for words in [true, false] {
            for (reach, tall) in [(20.0, 40.0), (60.0, 25.0)] {
                let mut room = Showroom::new("test", 12.0);
                room.centre = Vec3::new(900.0, 40.0, 1200.0);
                (room.reach, room.height) = (reach, tall);
                (room.yaw, room.pitch, room.distance) = (2.1, 0.45, 0.8);
                let mut camera = Camera::new(window);
                update_camera(&mut room, &mut camera, Frame::wallpaper(preview, words), 10.0, false, Vec2::ZERO, 0.0);
                for size in [(3840u32, 2160u32), (1920, 1080)] {
                    let picture = wallpaper_camera(&room, &camera, size, words, share);
                    assert!(picture.eye().distance(camera.eye()) < 0.05 * camera.distance(), "the same eye ({size:?})");
                    let to_picture = size.1 as f32 / preview.height();
                    for p in round(room.centre, reach, tall).into_iter().chain([room.centre]) {
                        let seen = project(&camera, p);
                        let drawn = project(&picture, p);
                        let expected = (seen - preview.min) * to_picture;
                        assert!((drawn.to_vec2() - expected).length() < 0.002 * size.1 as f32, "{p:?} at {drawn:?}, the preview has it at {expected:?} ({size:?}, words {words})");
                    }
                    // The unit fills its part of the picture, and keeps to it.
                    let area = wallpaper_area(Rect::from_min_size(Pos2::ZERO, vec2(size.0 as f32, size.1 as f32)), words);
                    assert!(project(&picture, room.centre).distance(area.center()) < 1.0);
                }
            }
        }
    }

    #[test]
    fn a_wallpaper_frames_the_unit_large() {
        // The Commander as drawn, about: a wallpaper with words puts it right of the middle and
        // over half the picture's height.
        let mut room = Showroom::new("test", 12.0);
        room.centre = Vec3::new(500.0, 20.0, 500.0);
        (room.reach, room.height) = (21.0, 38.0);
        let camera = Camera::new(Vec2::new(1920.0, 1080.0));
        let picture = wallpaper_camera(&room, &camera, (1920, 1080), true, 0.8);
        let top = project(&picture, room.centre + Vec3::Y * 19.0);
        let foot = project(&picture, room.centre - Vec3::Y * 19.0);
        let tall = (foot.y - top.y) / 1080.0;
        assert!(tall > 0.45 && tall < 0.85, "the unit {tall:.2} of the picture's height");
        assert!(project(&picture, room.centre).x > 1920.0 * 0.6, "right of the words");
    }

    #[test]
    fn going_home_takes_the_nearer_way_round() {
        let mut room = Showroom::new("test", 12.0);
        let mut camera = Camera::new(Vec2::new(1280.0, 720.0));
        let stage = Frame::stage(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0)));
        // Turned round nearly twice, from above and close: home again by the short way.
        (room.yaw, room.pitch, room.distance, room.homing) = (HOME.0 + 2.0 * TAU - 0.4, 1.0, 0.5, true);
        let start = room.yaw;
        for _ in 0..240 {
            update_camera(&mut room, &mut camera, stage, 1.0 / 60.0, false, Vec2::ZERO, 0.0);
            assert!(room.yaw >= start - 1e-3, "it turns 0.4 on, not 2π back");
        }
        assert!(!room.homing);
        assert!(((room.yaw - HOME.0).rem_euclid(TAU)).min(TAU - (room.yaw - HOME.0).rem_euclid(TAU)) < 2e-3);
        assert!((room.pitch - HOME.1).abs() < 2e-3 && (room.distance - 1.0).abs() < 2e-3);
        // A drag lets go of the glide.
        room.homing = true;
        update_camera(&mut room, &mut camera, stage, 1.0 / 60.0, false, Vec2::new(5.0, 0.0), 0.0);
        assert!(!room.homing);
    }

    #[test]
    fn each_side_keeps_its_colour() {
        assert_eq!((side_colour("ARM"), side_colour("core"), side_colour(" Core ")), (0, 1, 1));
        for side in ["TAL", "GOK", "ARMADA", "XYZ"] {
            let c = side_colour(side);
            assert!(![0, 1, 2, 4, 7].contains(&c), "{side}: a colour of its own that reads on a unit");
            assert_eq!(c, side_colour(&side.to_ascii_lowercase()));
        }
    }
}
