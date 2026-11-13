//! Staged scenes, for filming: `--demo stage` plays the scene a TOML file describes
//! (`TA_STAGE=<file>`), on the map it names or the one `--map` gives (`--map` wins): which units
//! stand where, facing which way, finished or being built; what they are told to do and when;
//! blasts, deaths and nuclear strikes on cue; the time of day, the weather, and where the camera
//! goes: an orbiting camera on a path, or a free camera put anywhere above the ground, looking
//! any way (straight up too), through any lens. Nothing else is on the map (the battle's own
//! Commanders are taken off it), and nothing moves of its own accord: no computer player plays
//! (unless `ai` asks for one), and every unit holds its position (it still fires at what comes in
//! range) until the scene orders it about.
//!
//! The scene starts at the battle's first frame; its clock (`t`, seconds) is the game's, which in
//! a capture on a fixed clock (`TA_CAPTURE_FPS`, as `scratch/trailer/film.py` films) is the
//! capture's own: an order at `t = 2` is carried out on the tick at 2 s, the frame the camera
//! and the time of day call 2 s. Orders go in as a player's commands would; spawning (aircraft
//! in the air too), clearing trees and rocks, taking units off the map, readying a silo's missile
//! and giving an aircraft a flying height of its own use what the scenarios and the other demos
//! use (`Sim::spawn_airborne`, `Sim::set_cruise_alt`). The simulation's rules are its own.
//!
//! # The scene file
//!
//! ```toml
//! map = "The Desert Triad"   # the map, by name or title (`--map` wins)
//! time = 17.4                # the time of day in hours, as TA_TIME: 0 night, 6.4 dawn,
//!                            #   12 TA's own light, 17.75 dusk (default: the settings'); or keys
//!                            #   of [seconds, hours] eased between, the sky, the sun and moon,
//!                            #   the stars, the light and the units' lights following:
//!                            #   time = [[0, 4.7], [1, 4.7], [5, 6.6], [6, 8.5]] runs night to
//!                            #   morning in five seconds (the twilight is 5 to 6 h: linger there)
//! moon = [245, 16]           # where the moon is: compass degrees (0 north, 90 east) and degrees
//!                            #   above the horizon (default: 42 high in TA's north-west); it
//!                            #   lights the night from there
//! rain = true                # in the middle of a shower on any world (false: dry weather;
//!                            #   unset: the world's own)
//! reveal = true              # no fog of war: everything is seen, by everyone (default true)
//! hud = false                # the battle's interface, as player 0's (default false)
//! icons = false              # strategic icons when far out (default false)
//! style = "reborn"           # the units' art: "reborn" or "classic" (default: the settings')
//! ai = false                 # computer players play every side (true) or these ([1, 2])
//! unlimited = true           # every side's stores stay full: builds and weapons never stall
//! metal = 600                # what every side starts with (10,000 by default: more than a store
//! energy = 600               #   holds); with `unlimited = false` the economy is a real one
//! seed = 7                   # the game's seed (its random numbers)
//! orders_per_tick = 10       # orders sending units somewhere of their own given a tick (10 by
//!                            #   default; thousands moving up in formation want hundreds)
//! missing = "skip"           # units of a type the game data lacks (an expansion not installed)
//!                            #   left out, with the orders and events only they were in
//!                            #   (default "error": the scene does not play)
//!
//! [[player]]                 # optional: a side's player (one for each `side` used, at least 2)
//! index = 1                  #   which (as a unit's `side`)
//! faction = "CORE"           #   ARM for even players, CORE for odd by default
//! color = 1                  #   its team colour (default: its index)
//! team = 1                   #   allies share a team (default: its index, all against all)
//! invulnerable = true        #   its units take no harm (TA's god mode; their own blasts do)
//!
//! [[clear]]                  # trees and rocks taken away (those where a unit is put go anyway)
//! at = [1500, 1200]          #   world pixels; or rel = [dx, dz] from the middle of the map
//! radius = 300
//!
//! [[feature]]                # a feature put down (after the clearings): a wreck, a heap, a rock
//! type = "CORLAB_DEAD"       #   its name in the game data (a unit's wreck is its `corpse`); drawn
//!                            #   when the map has its kind (or it is a wreck or a heap): another
//!                            #   world's features, a winter map's lanterns on a green one, are not
//! at = [1600, 1200]          #   or rel = [dx, dz]
//! heading = 30               #   degrees, as a unit's
//!
//! [[unit]]
//! type = "ARMCOM"            # the unit's name in the game data
//! side = 0                   # its player: 0 the Arm player, 1 the Core enemy, others
//! at = [1500, 1200]          # where it stands, in world pixels (x east, z south); or
//!                            #   rel = [dx, dz] from the middle of the map (where it cannot
//!                            #   stand, on a cliff or a slope too steep to build on, at sea:
//!                            #   the nearest place it can, within 256 px)
//! heading = 90               # degrees: 0 faces north (up the map), 90 east, 180 south (the
//!                            #   default, TA's), 270 west
//! label = "com"              # the name orders, events and the camera know it by (a unit's
//!                            #   type name works too: every unit of that type in the scene)
//! count = 12                 # a line abreast of them, centred on `at` and facing `heading`...
//! rows = 2                   #   ...in this many rows, one behind the other (default 1)
//! spacing = 48               #   px between them; or [across, back] (default: by their size)
//! built = 0.0                # how far its construction has got: under 1 it is a nanoframe,
//!                            #   the nano-lattice drawn that far up (default 1, finished)
//! builder = "com"            # who goes on building it, from the start (label or labels)
//! build_seconds = 4          # the rest of it takes this long while being built (default: as
//!                            #   long as its builders take); it can be no longer than they
//!                            #   would take for the whole thing
//! rise_at = 2.0              # with `build_seconds` and no builder: it rises on its own from
//!                            #   this second of the scene, a Commander arriving (default 0);
//!                            #   not in the game at all before (its lattice appears as a
//!                            #   build's does)
//! hold = true                # holds its fire (and its position) until it is first ordered
//! stockpile = 1              # missiles ready (a nuclear silo, an anti-missile defence)
//! airborne = true            # an aircraft already flying: at its height over `at`, heading
//!                            #   `heading` at full speed; it flies on straight ahead (to the
//!                            #   map's edge) until ordered
//! altitude = 70              # an aircraft's flying height over the ground, its own (default its
//!                            #   type's: a Thunder's 200): low passes, close to the lens
//!
//! [[order]]
//! t = 1.5                    # seconds after the scene starts
//! units = "tanks"            # who: a label or type name, or a list of them
//! do = "fight"               # move, fight (attack-move), attack, patrol, guard, repair (help
//!                            #   build or mend), stop, build, self_destruct (TA's five-second
//!                            #   countdown), dgun (fire a D-gun or launch a missile: manual fire),
//!                            #   produce (a factory makes `count` of `type`, at its own pace;
//!                            #   `move` to a factory sends what it makes there), on, off (TA's
//!                            #   on/off order: metal makers, radar, jammers, solar collectors)
//! at = [1900, 1200]          # where: world px, or rel = [dx, dz] from the middle of the map,
//!                            #   or by = [dx, dz] from where each unit stands (move, fight,
//!                            #   patrol, build, attack and dgun on the ground)
//! target = "kbots"           # attack, guard, repair, dgun: a unit (attackers share out its
//!                            #   units), instead of a place
//! type = "ARMLLT"            # build, produce: what
//! count = 4                  # produce: how many (default 1)
//! seconds = 4                # build, produce: its construction takes this long (as
//!                            #   `build_seconds`; produce: each one's)
//! label = "tower"            # build, produce: the label the new building (the units made) get
//! queue = false              # after their other orders rather than instead of them
//! formation = true           # move, fight, patrol to `at`/`rel` (or `by`): the group goes as one
//!                            #   formation order (`Command::Formation`), its middle there (or
//!                            #   moved by the offset), each keeping its place round it: under
//!                            #   the rule "modern movement" (on in scenes; TA_MOVEMENT=ta for
//!                            #   TA's) the ground, hover and naval units march there together,
//!                            #   their ways side by side, at the slowest one's pace. false: to
//!                            #   `at`, one order to the spot for them all (laid out in rows
//!                            #   under the rule); `by`, each unit on its own way at its own
//!                            #   pace (choreography timed unit by unit, the intro's)
//!
//! [[event]]
//! t = 3.0
//! do = "explosion"           # explosion: a weapon's blast, seen and heard (it harms nothing),
//!                            #   kill: units blown up as they die in battle (their wrecks, their
//!                            #   own blasts), nuke: a silo launches at a place, select: the
//!                            #   player's selection, as the interface shows it (`hud`), fire:
//!                            #   the trees and the like within `size` px (160) set alight,
//!                            #   fireworks: the racks within `size` px (500) of it sending their
//!                            #   shells up one after another, by night, for `seconds` (5),
//!                            #   signal: a player's signal to their allies at a place (`side`,
//!                            #   0 by default; `signal`: look, attack, defend, on_my_way,
//!                            #   need_metal, need_energy), computer allies acting on it,
//!                            #   duty: the player's constructors `units` go on a duty (`duty`:
//!                            #   economy, repair, reclaim, defence; none: back under command)
//! at = [1950, 1180]          # explosion, nuke, fire: where (or rel = [dx, dz])
//! size = 120                 # explosion: the blast's radius in px (the weapon whose blast is
//!                            #   nearest that; 48 by default)...
//! weapon = "ATOMIC_BLAST"    #   ...or a weapon's own blast, by name
//! units = "silo"             # kill, select, duty: who; nuke: the silo (given a missile if it has none; it
//!                            #   leaves some 4 s after, and a few hundred px away lands some
//!                            #   11 s later: for a blast on cue, an explosion with the missile's
//!                            #   weapon, CRBLMSSL for the Core's, NUCLEAR_MISSILE the Arm's)
//!
//! [[text]]                   # words in the world (`world_text.rs`): laid on the ground, burning,
//! t = 2.0                    #   projected as light or built by a nanolathe. Seconds after the
//! until = 9.0                #   scene starts it comes, and when it has gone (default t + 8)
//! text = "TOTAL\nANNIHILATION" # its lines (\n between them); `kicker` a small line over them
//! kicker = "THE ARM"
//! picture = "emblem"         # in place of `text`: TA Reborn's emblem, `size` world px across
//!                            #   (paint, burn, or nano: built from its near edge like the letters)
//! at = [1500, 1200]          # the middle of its block on the ground, or rel = [dx, dz]
//! heading = 0                # the way a reader looks, degrees (0 north: the text reads left to
//!                            #   right across it and its first line is the farthest)
//! size = 48                  # world px to a line: how big it is (default 48)
//! style = "burn"             # paint (default), burn (struck by a blast: it ignites in a ripple
//!                            #   out from `blast`, default its middle, and cools to embers),
//!                            #   light (projected, scanned in) or nano (built from the ground up)
//! blast = [1500, 1200]       # burn: where the blast falls, or `blast_rel`
//! color = [0.95, 0.9, 0.75]  # sRGB (default cream; the kicker's amber)
//! weight = "display"         # display (default: condensed bold), strong or regular
//! floor = 0.0                # how much of its paint shows in the dark (0..1)
//!
//! [act]                      # a credits scene's own part (`credits_show.rs`; playing the scene
//! title = "First light"      #   ignores it): its caption, the words of the roles it suits, the time
//! themes = ["art"]           #   of day at its start and end, how its cards look, the camera's height
//! hours = [5.7, 7.2]         #   over a card and distance from it (px) and lens, the clearing round
//! style = "burn"             #   each station (px), the least light painted cards show in, searchlights
//! [[station]]                # where a credits card goes: the middle of its block, the way the camera
//! at = [1500, 1200]          #   looks at it (degrees, 0 north), its look if not the act's, and where
//! heading = 90               #   the blast that sets it alight falls (`blast`)
//!
//! [camera]
//! start = "1500,1200,6,30,12" # as --camera (`camera` names it too): x,z,zoom,yaw,pitch: the
//!                            #   focus; screen pixels to a world pixel there (a window twice as
//!                            #   tall shows twice as much: film.py films 1440 lines); degrees
//!                            #   round (0 looks north, 90 east) and above the horizon (under 0
//!                            #   the eye is below the focus, looking up); an empty field keeps
//!                            #   the default (over the middle of the cast, as a battle starts)
//! path = "0:1500,1200,6,30,12;5:1600,1200,8,40,8"
//!                            # as TA_CAMERA_PATH: keys at seconds of the scene, eased between,
//!                            #   held at the ends; a sixth field lifts the focus that far over
//!                            #   the ground (for looking up at aircraft)
//! follow = "bombers"         # the focus on those units (their middle; with an empty x,z in the
//!                            #   path)
//!
//! [[camera.key]]             # or the free camera (in place of start, path and follow): its keys
//! t = 0                      #   at seconds of the scene, eased between them (a monotone spline:
//!                            #   moves start and stop gently, pass through every key without
//!                            #   overshooting it, and hold where two keys agree)
//! eye = [4700, 6020]         # the eye: [x, z] with `height` over the ground (or the sea) there,
//! height = 3                 #   kept over it between two such keys; or [x, y, z] in the world.
//!                            #   It never goes under the ground or into the sea
//! yaw = 270                  # where it looks: degrees round (0 north, 90 east)...
//! tilt = 20                  #   ...and above the level (90 straight up, -90 straight down); or
//! # look = [4500, 6000]      #   at a place: [x, z] (`look_height` over the ground there) or
//! # look_height = 60         #   [x, y, z]; or at units as they move: track = "bombers" (their
//! # track = "bombers"        #   middle, `look_height` over it)
//! roll = 0                   # degrees, positive banks right (the picture turns anticlockwise)
//! fov = 46                   # the lens: vertical degrees (30 the game's own; 46 is about 75°
//!                            #   across at 16:9); roll and fov carry on from the key before
//!
//! [[ride]]                   # the camera into a unit, as the player's perspective key goes in: it
//! t = 3                      #   dives in at `t` and rides along, the unit going on with its
//! unit = "lead"              #   orders; the first of `unit` (a label or type name) still there.
//! leave = 20                 #   Out again at `leave` (by default it rides to the end), the
//!                            #   camera back over the map, then the scene's own again
//! ```
//!
//! Units are placed exactly where they are asked for (buildings on their footprint's grid, as
//! any are); a unit put where it cannot stand (a tank in deep water, a building on a cliff) is
//! put there anyway, with a warning in the log. A few units given a destination of their own in
//! the same tick plan their own ways; so orders that send each unit somewhere of its own go out
//! ten a tick (a dozen or more would share one route and funnel into it, see `--demo war`).
//!
//! A frame shown part built is a frame in the game (its own progress starts at nothing); the
//! scene draws it as far built as it says, and while it is being built moves that on at the
//! scene's pace (`build_seconds`) or its builders'. When the drawing is done the game finishes it
//! too: its builders' player builds instantly for a tick (TA's cheat), so anything else that
//! player is building or making in that tick is finished then too.
//!
//! The free camera's eye may go anywhere above the ground: low over the sand looking straight up
//! at aircraft, close past a tower; its near plane follows it down (things a few px from the lens
//! stay in view), and what it sees is drawn, shadowed and hazed as the orbiting camera's would be
//! (what stands and flies round an eye that looks up included). Between two keys that both look
//! at something it keeps looking at it as the eye moves; else it turns between their angles. A
//! track on aircraft passing straight overhead swings round fast (as a camera on a head would).
//!
//! Environment variables win over the file: `TA_TIME`, `TA_RAIN`, `TA_HUD`, `TA_ICONS`,
//! `TA_ART_STYLE`, `TA_CAMERA_PATH`, `TA_FOLLOW`; `TA_RIDE` rides a unit of the scene.

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, bail};
use glam::{Vec2, Vec3};
use serde::Deserialize;
use ta_sim::{Cheat, Command, Fx, FxVec3, Locomotion, Order, Sim, SimEvent, Target, UnitId};

use crate::game::Game;
use crate::setup::{Assets, Controller, GameSetup, SlotSetup};

/// Simulation ticks a second.
const TICKS: f32 = 30.0;
/// Most commands that send units somewhere of their own given in a tick (see the module notes).
const PER_TICK: usize = 10;
/// Where units face unless told: south, as TA places them.
const SOUTH: f32 = 180.0;
/// How far round a `fire` event's place the trees and the like catch (px).
const FIRE_RADIUS: f32 = 160.0;
/// A `fireworks` event: the racks within this far of its place (px), and for how long (s).
const VOLLEY_REACH: f32 = 500.0;
const VOLLEY_SECONDS: f32 = 5.0;

/// How long a build ordered by the scene is looked out for (its builder may first walk there).
const WATCH_TICKS: u64 = 60 * 30;
/// How close (world units) the free camera's eye comes to the ground or the sea.
const MIN_EYE: f32 = 2.0;
/// The free camera's focus distance (where the haze begins) when it looks along angles.
const FREE_DISTANCE: f32 = 600.0;
/// How far ahead (px) aircraft put in the air fly on until ordered.
const FLY_ON: f32 = 4000.0;
/// The most players a scene has (its sides 0 to 15).
const MAX_PLAYERS: usize = 16;
/// How long a finished frame's player keeps building instantly at most (its builder has it
/// done the tick after; this is in case it turned away).
const FINISH_TICKS: u64 = 15;

fn yes() -> bool {
    true
}

fn one() -> u32 {
    1
}

fn whole() -> f32 {
    1.0
}

fn south() -> f32 {
    SOUTH
}

/// A scene (see the module's notes for what each key does).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    #[serde(default)]
    pub map: Option<String>,
    #[serde(default)]
    pub time: Option<Hours>,
    #[serde(default)]
    pub moon: Option<[f32; 2]>,
    #[serde(default)]
    pub rain: Option<bool>,
    #[serde(default = "yes")]
    pub reveal: bool,
    #[serde(default)]
    pub hud: bool,
    #[serde(default)]
    pub icons: bool,
    #[serde(default)]
    pub style: Option<Style>,
    #[serde(default)]
    pub ai: Sides,
    #[serde(default = "yes")]
    pub unlimited: bool,
    /// What every side starts with, in metal and energy.
    #[serde(default)]
    pub metal: Option<u64>,
    #[serde(default)]
    pub energy: Option<u64>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// Orders sending units somewhere of their own given in one tick (see [`PER_TICK`]): an army
    /// of thousands moving up in formation needs many more than ten (the simulation plans
    /// thousands' ways together).
    #[serde(default)]
    pub orders_per_tick: Option<usize>,
    /// What becomes of units of a type the game data lacks.
    #[serde(default)]
    pub missing: Missing,
    #[serde(default, rename = "player")]
    pub players: Vec<PlayerSpec>,
    #[serde(default, rename = "clear")]
    pub clearings: Vec<ClearSpec>,
    #[serde(default, rename = "feature")]
    pub features: Vec<FeatureSpec>,
    #[serde(default, rename = "unit")]
    pub units: Vec<UnitSpec>,
    #[serde(default, rename = "order")]
    pub orders: Vec<OrderSpec>,
    #[serde(default, rename = "event")]
    pub events: Vec<EventSpec>,
    #[serde(default, rename = "text")]
    pub texts: Vec<TextEntry>,
    /// The credits' acts (`credits_show.rs`): which a scene is, and where its cards go.
    #[serde(default)]
    pub act: Option<ActSpec>,
    #[serde(default, rename = "station")]
    pub stations: Vec<StationSpec>,
    #[serde(default)]
    pub camera: CameraSpec,
    #[serde(default, rename = "ride")]
    pub rides: Vec<RideSpec>,
}

/// The camera into a unit (perspective): from `t` it dives in and rides along, out again at
/// `leave` (or to the end).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RideSpec {
    pub t: f32,
    /// Who: the first of them still there (a label or type name).
    pub unit: Names,
    #[serde(default)]
    pub leave: Option<f32>,
}

/// What a scene's ride asks of the game this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RideCue {
    /// Into this unit.
    Enter(UnitId),
    /// Back out of the unit ridden.
    Leave,
}

/// The time of day: hours, or keys of [seconds, hours] eased between.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Hours {
    At(f32),
    Keys(Vec<[f32; 2]>),
}

impl Hours {
    /// The hours at `clock` seconds: the one, or the keys eased between (past 24 they go
    /// round the clock: 30.5 is half past six the next morning).
    pub fn at(&self, clock: f32) -> f32 {
        match self {
            Hours::At(h) => *h,
            Hours::Keys(keys) => {
                let mut keys: Vec<(f32, f32)> = keys.iter().map(|&[t, h]| (t, h)).collect();
                keys.sort_by(|a, b| a.0.total_cmp(&b.0));
                eased(&keys, clock)
            }
        }
    }
}

/// What a scene does with units of a type the game data lacks (an expansion not installed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Missing {
    /// The scene does not play.
    #[default]
    Error,
    /// They are left out, with the builds of such types and the orders and events only they
    /// were in.
    Skip,
}

/// The units' art.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    Reborn,
    Classic,
}

/// Which sides something applies to: all of them or none (`true`, `false`), or those listed.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Sides {
    All(bool),
    These(Vec<u8>),
}

impl Default for Sides {
    fn default() -> Sides {
        Sides::All(false)
    }
}

impl Sides {
    pub fn includes(&self, player: u8) -> bool {
        match self {
            Sides::All(all) => *all,
            Sides::These(list) => list.contains(&player),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerSpec {
    pub index: u8,
    #[serde(default)]
    pub faction: Option<String>,
    #[serde(default)]
    pub color: Option<u8>,
    #[serde(default)]
    pub team: Option<u8>,
    #[serde(default)]
    pub invulnerable: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureSpec {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub at: Option<[f32; 2]>,
    #[serde(default)]
    pub rel: Option<[f32; 2]>,
    #[serde(default = "south")]
    pub heading: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearSpec {
    #[serde(default)]
    pub at: Option<[f32; 2]>,
    #[serde(default)]
    pub rel: Option<[f32; 2]>,
    pub radius: f32,
}

/// One name or several (labels, unit type names).
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Names {
    One(String),
    Many(Vec<String>),
}

impl Names {
    fn list(&self) -> Vec<&str> {
        match self {
            Names::One(n) => vec![n.as_str()],
            Names::Many(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

/// Space between units placed together: the same each way, or [across, back].
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(untagged)]
pub enum Spacing {
    Even(f32),
    Grid([f32; 2]),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitSpec {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub side: u8,
    #[serde(default)]
    pub at: Option<[f32; 2]>,
    #[serde(default)]
    pub rel: Option<[f32; 2]>,
    #[serde(default = "south")]
    pub heading: f32,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "one")]
    pub count: u32,
    #[serde(default = "one")]
    pub rows: u32,
    #[serde(default)]
    pub spacing: Option<Spacing>,
    #[serde(default = "whole")]
    pub built: f32,
    #[serde(default)]
    pub builder: Option<Names>,
    #[serde(default)]
    pub build_seconds: Option<f32>,
    #[serde(default)]
    pub rise_at: Option<f32>,
    #[serde(default)]
    pub hold: bool,
    #[serde(default)]
    pub stockpile: u32,
    #[serde(default)]
    pub airborne: bool,
    #[serde(default)]
    pub altitude: Option<f32>,
}

/// What an order tells units to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Move,
    Fight,
    Attack,
    Patrol,
    Guard,
    #[serde(alias = "assist")]
    Repair,
    Stop,
    Build,
    #[serde(alias = "selfdestruct", alias = "self-destruct")]
    SelfDestruct,
    #[serde(alias = "fire", alias = "launch")]
    Dgun,
    #[serde(alias = "make")]
    Produce,
    /// Switched on, as TA's on/off order (`Command::SetActive`): metal makers, radar...
    #[serde(alias = "activate")]
    On,
    /// Switched off.
    #[serde(alias = "deactivate")]
    Off,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderSpec {
    pub t: f32,
    pub units: Names,
    #[serde(rename = "do")]
    pub action: Action,
    #[serde(default)]
    pub at: Option<[f32; 2]>,
    #[serde(default)]
    pub rel: Option<[f32; 2]>,
    #[serde(default)]
    pub by: Option<[f32; 2]>,
    #[serde(default)]
    pub target: Option<Names>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub seconds: Option<f32>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub queue: bool,
    #[serde(default = "yes")]
    pub formation: bool,
}

/// What happens at an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Happening {
    Explosion,
    Kill,
    Nuke,
    Select,
    Fire,
    Fireworks,
    Signal,
    Duty,
}

/// A duty a scene's constructors are put on (`none`: back under the player's command).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DutyName {
    None,
    Economy,
    Repair,
    Reclaim,
    Defence,
}

impl DutyName {
    fn duty(self) -> Option<ta_ai::Duty> {
        match self {
            DutyName::None => None,
            DutyName::Economy => Some(ta_ai::Duty::Economy),
            DutyName::Repair => Some(ta_ai::Duty::Repair),
            DutyName::Reclaim => Some(ta_ai::Duty::Reclaim),
            DutyName::Defence => Some(ta_ai::Duty::Defence),
        }
    }
}

/// A signal's meaning, as a scene names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalName {
    Look,
    Attack,
    Defend,
    OnMyWay,
    NeedMetal,
    NeedEnergy,
}

impl SignalName {
    fn kind(self) -> ta_sim::SignalKind {
        use ta_sim::SignalKind as K;
        match self {
            SignalName::Look => K::Look,
            SignalName::Attack => K::Attack,
            SignalName::Defend => K::Defend,
            SignalName::OnMyWay => K::OnMyWay,
            SignalName::NeedMetal => K::NeedMetal,
            SignalName::NeedEnergy => K::NeedEnergy,
        }
    }
}

/// A credits scene's own part (`[act]`, read by `credits_show.rs`; a scene's play ignores it).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActSpec {
    /// What the caption says of the moment (translatable), and what the act is about (the words
    /// of a role it suits: "programming", "art", "music"...).
    pub title: String,
    #[serde(default)]
    pub themes: Vec<String>,
    /// The time of day (hours) at the start and at the end of the act.
    #[serde(default)]
    pub hours: Option<[f32; 2]>,
    /// How its names look unless a station says (default burn).
    #[serde(default = "burning")]
    pub style: TextStyle,
    /// The picture the camera makes of each card: how high over it and how far back (world px),
    /// and its lens (vertical degrees).
    #[serde(default)]
    pub height: Option<f32>,
    #[serde(default)]
    pub back: Option<f32>,
    #[serde(default)]
    pub fov: Option<f32>,
    /// Trees and rocks taken away round each station (px; default 300).
    #[serde(default)]
    pub clear: Option<f32>,
    /// The widest a card is laid (px; default `credits_show::ROAD`): the road the armies stand
    /// clear of, either side.
    #[serde(default)]
    pub road: Option<f32>,
    /// The least light its painted cards show in (0..1: how much they show in the dark; default
    /// 0.6), and how many searchlights sweep its road (pools of light that reveal them).
    #[serde(default)]
    pub floor: Option<f32>,
    #[serde(default)]
    pub searchlights: u32,
}

/// Where a credits card goes (`[[station]]`): the middle of its block on the ground, and the way
/// the camera looks at it (degrees, 0 north: the card reads left to right across it).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StationSpec {
    #[serde(default)]
    pub at: Option<[f32; 2]>,
    #[serde(default)]
    pub rel: Option<[f32; 2]>,
    #[serde(default)]
    pub heading: f32,
    #[serde(default)]
    pub style: Option<TextStyle>,
    /// A burning card's blast falls here (default: a little ahead of the block).
    #[serde(default)]
    pub blast: Option<[f32; 2]>,
}

/// Words in the world (`[[text]]`).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextEntry {
    #[serde(default)]
    pub t: f32,
    #[serde(default)]
    pub until: Option<f32>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub picture: Option<TextPicture>,
    #[serde(default)]
    pub kicker: Option<String>,
    #[serde(default)]
    pub at: Option<[f32; 2]>,
    #[serde(default)]
    pub rel: Option<[f32; 2]>,
    #[serde(default)]
    pub heading: f32,
    #[serde(default = "text_size")]
    pub size: f32,
    #[serde(default)]
    pub style: TextStyle,
    #[serde(default)]
    pub blast: Option<[f32; 2]>,
    #[serde(default)]
    pub blast_rel: Option<[f32; 2]>,
    #[serde(default)]
    pub color: Option<[f32; 3]>,
    #[serde(default)]
    pub weight: TextWeight,
    #[serde(default)]
    pub floor: f32,
}

fn text_size() -> f32 {
    48.0
}

fn burning() -> TextStyle {
    TextStyle::Burn
}

/// How a `[[text]]` looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextStyle {
    #[default]
    Paint,
    Burn,
    Light,
    Nano,
}

/// A picture a `[[text]]` lays in place of words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextPicture {
    /// TA Reborn's emblem.
    Emblem,
}

/// How heavy a `[[text]]` is set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextWeight {
    #[default]
    Display,
    Strong,
    Regular,
}

impl TextEntry {
    /// The text to put in the world, `start` seconds being the scene's start on the game's
    /// clock; `middle` the map's middle (for `rel`).
    pub fn spec(&self, start: f32, middle: (f32, f32)) -> Option<crate::world_text::TextSpec> {
        use crate::world_text::{LineSpec, Look, TextSpec, Weight};
        if self.text.is_empty() && self.picture.is_none() {
            return None;
        }
        let (x, z) = point(self.at, self.rel, middle)?;
        let weight = match self.weight {
            TextWeight::Display => Weight::Display,
            TextWeight::Strong => Weight::Strong,
            TextWeight::Regular => Weight::Regular,
        };
        let color = self.color.unwrap_or(match self.style {
            TextStyle::Light => [0.55, 0.9, 1.0],
            TextStyle::Nano => [0.82, 0.78, 0.66],
            _ => [0.96, 0.92, 0.8],
        });
        let mut lines = Vec::new();
        if let Some(kicker) = &self.kicker {
            lines.push(LineSpec::new(kicker.to_uppercase(), 0.42, Weight::Strong, [0.95, 0.76, 0.3]).tracking(0.16).gap(0.0));
        }
        for (i, line) in self.text.split('\n').enumerate() {
            lines.push(LineSpec::new(line, 1.0, weight, color).gap(if i == 0 && self.kicker.is_some() { 0.15 } else { 0.0 }));
        }
        let look = match self.style {
            TextStyle::Paint => Look::Paint,
            TextStyle::Burn => {
                let at = point(self.blast, self.blast_rel, middle).unwrap_or((x, z));
                Look::Burn { at: Vec2::new(at.0, at.1), after: 0.0 }
            }
            TextStyle::Light => Look::Light,
            TextStyle::Nano => Look::Nano,
        };
        Some(TextSpec { lines, at: Vec2::new(x, z), heading: self.heading, size: self.size, look, born: start + self.t, life: self.until.unwrap_or(self.t + 8.0) - self.t, floor: self.floor })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventSpec {
    pub t: f32,
    #[serde(rename = "do")]
    pub what: Happening,
    #[serde(default)]
    pub at: Option<[f32; 2]>,
    #[serde(default)]
    pub rel: Option<[f32; 2]>,
    #[serde(default)]
    pub size: Option<f32>,
    #[serde(default)]
    pub weapon: Option<String>,
    #[serde(default)]
    pub units: Option<Names>,
    #[serde(default)]
    pub seconds: Option<f32>,
    /// signal: whose, and what it means.
    #[serde(default)]
    pub side: Option<u8>,
    #[serde(default)]
    pub signal: Option<SignalName>,
    /// duty: which.
    #[serde(default)]
    pub duty: Option<DutyName>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraSpec {
    #[serde(default, alias = "camera")]
    pub start: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub follow: Option<String>,
    /// The free camera's keys (`[[camera.key]]`).
    #[serde(default, rename = "key")]
    pub keys: Vec<ViewKey>,
}

/// A key of the free camera: where the eye is, where it looks, its roll and its lens.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewKey {
    pub t: f32,
    /// [x, z] (with `height` over the ground there) or [x, y, z] (world).
    pub eye: Vec<f32>,
    #[serde(default)]
    pub height: Option<f32>,
    /// [x, z] (`look_height` over the ground there) or [x, y, z]; or the units it tracks.
    #[serde(default)]
    pub look: Option<Vec<f32>>,
    #[serde(default)]
    pub track: Option<Names>,
    #[serde(default)]
    pub look_height: Option<f32>,
    /// Degrees: 0 looks north, 90 east; above the level (90 straight up, -90 straight down).
    #[serde(default)]
    pub yaw: Option<f32>,
    #[serde(default)]
    pub tilt: Option<f32>,
    /// Degrees: positive banks right (the picture turns anticlockwise); the key before's when
    /// left out (0 at first).
    #[serde(default)]
    pub roll: Option<f32>,
    /// Vertical degrees; the key before's when left out (30 at first, the game's own).
    #[serde(default)]
    pub fov: Option<f32>,
}

static STAGE: OnceLock<Option<Stage>> = OnceLock::new();

/// Reads the scene `--demo stage` plays (`TA_STAGE`), once, before the game asks for it.
pub fn init(demo: Option<&str>) -> Result<()> {
    let stage = match demo {
        Some("stage") => {
            let path = std::env::var_os("TA_STAGE").ok_or_else(|| anyhow!("--demo stage plays a scene: set TA_STAGE to its file (see apps/ta_game/src/stage.rs)"))?;
            Some(Stage::load(Path::new(&path))?)
        }
        _ => None,
    };
    let _ = STAGE.set(stage);
    Ok(())
}

/// The scene being played, if this is `--demo stage`.
pub fn current() -> Option<&'static Stage> {
    STAGE.get().and_then(Option::as_ref)
}

/// A place given as `at` (world px) or `rel` (from the middle of the map).
fn point(at: Option<[f32; 2]>, rel: Option<[f32; 2]>, middle: (f32, f32)) -> Option<(f32, f32)> {
    at.map(|[x, z]| (x, z)).or(rel.map(|[dx, dz]| (middle.0 + dx, middle.1 + dz)))
}

/// TA's heading (65536 a turn, 0 facing south, +z) for one in degrees clockwise from north.
pub fn ta_heading(degrees: f32) -> i32 {
    let a = ((SOUTH - degrees) / 360.0 * 65536.0).round() as i64;
    ((a + 32768).rem_euclid(65536) - 32768) as i32
}

/// Where `count` units stand: a line abreast (or `rows` of them, the first in front) centred on
/// `at`, facing `heading` (degrees clockwise from north), `spacing` apart (by default as far as
/// units with this footprint need).
fn block(at: (f32, f32), heading: f32, count: u32, rows: u32, spacing: Option<Spacing>, footprint: (u32, u32)) -> Vec<(f32, f32)> {
    let size = footprint.0.max(footprint.1) as f32 * 16.0 + 24.0;
    let (across, back) = match spacing {
        None => (size, size),
        Some(Spacing::Even(s)) => (s, s),
        Some(Spacing::Grid([a, b])) => (a, b),
    };
    let count = count.max(1) as usize;
    let cols = count.div_ceil(rows.max(1) as usize).max(1);
    let rows = count.div_ceil(cols);
    let (s, c) = heading.to_radians().sin_cos();
    // Ahead and to the right (x east, z south): facing north, ahead is -z and the right +x.
    let (ahead, right) = ((s, -c), (c, s));
    (0..count)
        .map(|i| {
            let (row, col) = (i / cols, i % cols);
            let in_row = cols.min(count - row * cols);
            let a = (col as f32 - (in_row as f32 - 1.0) / 2.0) * across;
            let b = (row as f32 - (rows as f32 - 1.0) / 2.0) * back;
            // (To a 64th of a pixel: a sine's last bit is no part of where they were put.)
            let snap = |v: f32| (v * 64.0).round() / 64.0;
            (snap(at.0 + right.0 * a - ahead.0 * b), snap(at.1 + right.1 * a - ahead.1 * b))
        })
        .collect()
}

/// The tick `t` seconds after the scene's start.
fn tick_at(start: u64, t: f32) -> u64 {
    start + (t.max(0.0) * TICKS).round() as u64
}

fn fx_point((x, z): (f32, f32)) -> FxVec3 {
    FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z))
}

impl Stage {
    /// Reads a scene file.
    pub fn load(path: &Path) -> Result<Stage> {
        let text = std::fs::read_to_string(path).with_context(|| format!("scene {}", path.display()))?;
        Stage::parse(&text).with_context(|| format!("scene {}", path.display()))
    }

    /// Reads a scene, checking what can be checked without the game's data.
    pub fn parse(text: &str) -> Result<Stage> {
        let stage: Stage = toml::from_str(text)?;
        stage.check()?;
        Ok(stage)
    }

    fn check(&self) -> Result<()> {
        let place = |at: Option<[f32; 2]>, rel: Option<[f32; 2]>, what: &dyn Fn() -> String| -> Result<()> {
            match (at, rel) {
                (Some(_), Some(_)) => bail!("{}: `at` or `rel`, not both", what()),
                (None, None) => bail!("{}: needs a place (`at` or `rel`)", what()),
                _ => Ok(()),
            }
        };
        if let Some(Hours::Keys(keys)) = &self.time
            && (keys.is_empty() || keys.iter().any(|[t, h]| !t.is_finite() || *t < 0.0 || !h.is_finite()))
        {
            bail!("`time`: hours, or keys of [seconds from the start, hours]");
        }
        if let Some([_, elevation]) = self.moon
            && !(-90.0..=90.0).contains(&elevation)
        {
            bail!("`moon`: [compass degrees, degrees above the horizon (-90 to 90)]");
        }
        for p in &self.players {
            if p.index as usize >= MAX_PLAYERS {
                bail!("player {}: `index` is a player from 0 to {}", p.index, MAX_PLAYERS - 1);
            }
        }
        for (i, f) in self.features.iter().enumerate() {
            place(f.at, f.rel, &|| format!("feature {} ({})", i + 1, f.kind))?;
        }
        for (i, c) in self.clearings.iter().enumerate() {
            place(c.at, c.rel, &|| format!("clearing {}", i + 1))?;
        }
        for (i, u) in self.units.iter().enumerate() {
            let what = || format!("unit {} ({})", i + 1, u.kind);
            place(u.at, u.rel, &what)?;
            if !(0.0..=1.0).contains(&u.built) {
                bail!("{}: `built` goes from 0 to 1", what());
            }
            if u.count == 0 || u.rows == 0 {
                bail!("{}: `count` and `rows` are 1 or more", what());
            }
            if u.side as usize >= MAX_PLAYERS {
                bail!("{}: `side` is a player from 0 to {}", what(), MAX_PLAYERS - 1);
            }
            if u.build_seconds.is_some_and(|s| s.is_nan() || s <= 0.0) {
                bail!("{}: `build_seconds` is more than 0", what());
            }
            if (u.builder.is_some() || u.build_seconds.is_some()) && u.built >= 1.0 {
                bail!("{}: `builder` and `build_seconds` need a unit not yet built (`built` under 1)", what());
            }
            if u.rise_at.is_some() && (u.builder.is_some() || u.build_seconds.is_none()) {
                bail!("{}: `rise_at` is for a frame rising on its own: `build_seconds` and no `builder`", what());
            }
            if u.rise_at.is_some_and(|t| !t.is_finite() || t < 0.0) {
                bail!("{}: `rise_at` is seconds into the scene, 0 or more", what());
            }
            if u.airborne && u.built < 1.0 {
                bail!("{}: what is in the air is finished (`built` 1)", what());
            }
            if u.altitude.is_some_and(|a| a.is_nan() || a <= 0.0) {
                bail!("{}: `altitude` is px over the ground, more than 0", what());
            }
        }
        for (i, o) in self.orders.iter().enumerate() {
            let what = || format!("order {} ({:?} at {} s)", i + 1, o.action, o.t);
            if !o.t.is_finite() || o.t < 0.0 {
                bail!("{}: `t` is seconds from the start, 0 or more", what());
            }
            let places = [o.at.is_some(), o.rel.is_some(), o.by.is_some()].iter().filter(|&&p| p).count();
            if places > 1 {
                bail!("{}: one of `at`, `rel` and `by`", what());
            }
            let placed = places == 1;
            let ok = match o.action {
                Action::Move | Action::Fight | Action::Patrol => placed && o.target.is_none(),
                Action::Attack | Action::Dgun => placed != o.target.is_some(),
                Action::Guard | Action::Repair => o.target.is_some() && !placed,
                Action::Build => placed && o.kind.is_some(),
                Action::Stop | Action::SelfDestruct | Action::On | Action::Off => !placed && o.target.is_none(),
                Action::Produce => o.kind.is_some() && !placed && o.target.is_none(),
            };
            if !ok {
                bail!(
                    "{}: {}",
                    what(),
                    match o.action {
                        Action::Move | Action::Fight | Action::Patrol => "needs a place (`at`, `rel` or `by`)",
                        Action::Attack | Action::Dgun => "needs a `target` or a place, not both",
                        Action::Guard | Action::Repair => "needs a `target`",
                        Action::Build => "needs a `type` and a place",
                        Action::Stop | Action::SelfDestruct | Action::On | Action::Off => "takes no place or target",
                        Action::Produce => "needs a `type` (no place or target)",
                    }
                );
            }
            if (o.seconds.is_some() || o.label.is_some()) && !matches!(o.action, Action::Build | Action::Produce) {
                bail!("{}: `seconds` and `label` go with `build` and `produce`", what());
            }
            if o.seconds.is_some_and(|s| s.is_nan() || s <= 0.0) {
                bail!("{}: `seconds` is more than 0", what());
            }
            if o.count.is_some() && o.action != Action::Produce {
                bail!("{}: `count` goes with `produce`", what());
            }
            if o.count == Some(0) {
                bail!("{}: `count` is 1 or more", what());
            }
        }
        for (i, e) in self.events.iter().enumerate() {
            let what = || format!("event {} ({:?} at {} s)", i + 1, e.what, e.t);
            if !e.t.is_finite() || e.t < 0.0 {
                bail!("{}: `t` is seconds from the start, 0 or more", what());
            }
            match e.what {
                Happening::Explosion | Happening::Fire | Happening::Fireworks => place(e.at, e.rel, &what)?,
                Happening::Signal => place(e.at, e.rel, &what)?,
                Happening::Nuke => {
                    place(e.at, e.rel, &what)?;
                    if e.units.is_none() {
                        bail!("{}: needs the silo (`units`)", what());
                    }
                }
                Happening::Kill | Happening::Select => {
                    if e.units.is_none() {
                        bail!("{}: needs `units`", what());
                    }
                }
                Happening::Duty => {
                    if e.units.is_none() || e.duty.is_none() {
                        bail!("{}: needs `units` and a `duty`", what());
                    }
                }
            }
            if e.duty.is_some() && e.what != Happening::Duty {
                bail!("{}: `duty` goes with `duty`", what());
            }
            if e.weapon.is_some() && e.what != Happening::Explosion || e.size.is_some() && !matches!(e.what, Happening::Explosion | Happening::Fire | Happening::Fireworks) {
                bail!("{}: `size` goes with `explosion`, `fire` and `fireworks`, `weapon` with `explosion`", what());
            }
            if e.seconds.is_some() && e.what != Happening::Fireworks || e.seconds.is_some_and(|s| !s.is_finite() || s <= 0.0) {
                bail!("{}: `seconds` goes with `fireworks`, more than 0", what());
            }
            if (e.side.is_some() || e.signal.is_some()) && e.what != Happening::Signal {
                bail!("{}: `side` and `signal` go with `signal`", what());
            }
        }
        if let Some(path) = &self.camera.path {
            parse_path(path)?;
        }
        let c = &self.camera;
        if !c.keys.is_empty() && (c.start.is_some() || c.path.is_some() || c.follow.is_some()) {
            bail!("[camera]: the free camera's keys, or `start`, `path` and `follow`: not both");
        }
        for (i, k) in c.keys.iter().enumerate() {
            let what = || format!("camera key {} (at {} s)", i + 1, k.t);
            if !k.t.is_finite() || k.t < 0.0 {
                bail!("{}: `t` is seconds from the start, 0 or more", what());
            }
            match (k.eye.len(), k.height) {
                (2, Some(_)) | (3, None) => {}
                (2, None) => bail!("{}: an `eye` of [x, z] needs its `height` over the ground", what()),
                (3, Some(_)) => bail!("{}: an `eye` of [x, y, z] is where it is: no `height`", what()),
                _ => bail!("{}: `eye` is [x, z] (with `height`) or [x, y, z]", what()),
            }
            if k.eye.iter().chain(&k.look.clone().unwrap_or_default()).any(|v| !v.is_finite()) {
                bail!("{}: a place that is not a number", what());
            }
            if k.look.as_ref().is_some_and(|l| !matches!(l.len(), 2 | 3)) {
                bail!("{}: `look` is [x, z] (with `look_height` over the ground) or [x, y, z]", what());
            }
            let angles = k.yaw.is_some() || k.tilt.is_some();
            if [k.look.is_some(), k.track.is_some(), angles].iter().filter(|&&b| b).count() > 1 {
                bail!("{}: it looks at a place (`look`), at units (`track`) or along `yaw` and `tilt`: one of them", what());
            }
            if k.look_height.is_some() && k.track.is_none() && !k.look.as_ref().is_some_and(|l| l.len() == 2) {
                bail!("{}: `look_height` goes with a `look` of [x, z] or with `track`", what());
            }
            if k.tilt.is_some_and(|t| !(-90.0..=90.0).contains(&t)) {
                bail!("{}: `tilt` is -90 (straight down) to 90 (straight up)", what());
            }
            if k.fov.is_some_and(|f| !(1.0..=170.0).contains(&f)) {
                bail!("{}: `fov` is vertical degrees, 1 to 170", what());
            }
        }
        for (i, r) in self.rides.iter().enumerate() {
            let what = || format!("ride {} (at {} s)", i + 1, r.t);
            if !r.t.is_finite() || r.t < 0.0 {
                bail!("{}: `t` is seconds from the start, 0 or more", what());
            }
            if r.leave.is_some_and(|l| !l.is_finite() || l <= r.t) {
                bail!("{}: `leave` is a second after `t`", what());
            }
        }
        Ok(())
    }

    /// Names the scene's orders, events, builders and camera may use: its units' labels, the
    /// labels its builds give, and its units' types.
    /// With `missing = "skip"`: the units of a type `installed` does not know left out, with the
    /// builds of such types and the orders, events, builders and camera tracks that were only
    /// theirs. Returns the types left out (none otherwise).
    pub fn prune_missing(&mut self, installed: impl Fn(&str) -> bool) -> Vec<String> {
        if self.missing != Missing::Skip {
            return Vec::new();
        }
        let mut gone: Vec<String> = self.units.iter().map(|u| u.kind.clone()).chain(self.orders.iter().filter_map(|o| o.kind.clone())).filter(|k| !installed(k)).map(|k| k.to_ascii_uppercase()).collect();
        gone.sort();
        gone.dedup();
        if gone.is_empty() {
            return gone;
        }
        self.units.retain(|u| installed(&u.kind));
        self.orders.retain(|o| o.kind.as_deref().is_none_or(&installed));
        // (The labels the builds of such types would have given went with them.)
        let known = self.known_names();
        let keep = |names: &mut Names| -> bool {
            let list: Vec<String> = names.list().into_iter().filter(|n| known.iter().any(|k| k.eq_ignore_ascii_case(n))).map(str::to_string).collect();
            *names = match list.len() {
                0 => return false,
                1 => Names::One(list[0].clone()),
                _ => Names::Many(list),
            };
            true
        };
        self.orders.retain_mut(|o| keep(&mut o.units) && o.target.as_mut().is_none_or(&keep));
        self.events.retain_mut(|e| e.units.as_mut().is_none_or(&keep));
        for u in &mut self.units {
            if u.builder.as_mut().is_some_and(|b| !keep(b)) {
                u.builder = None;
            }
        }
        if self.camera.follow.as_ref().is_some_and(|f| !known.iter().any(|k| k.eq_ignore_ascii_case(f))) {
            self.camera.follow = None;
        }
        for k in &mut self.camera.keys {
            if k.track.as_mut().is_some_and(|t| !keep(t)) {
                k.track = None;
            }
        }
        self.rides.retain_mut(|r| keep(&mut r.unit));
        gone
    }

    fn known_names(&self) -> Vec<String> {
        let labels = self.units.iter().filter_map(|u| u.label.clone()).chain(self.orders.iter().filter_map(|o| o.label.clone()));
        labels.chain(self.units.iter().map(|u| u.kind.clone())).collect()
    }

    /// Checks the scene against the game's data: the units, weapons and names it uses.
    fn check_names(&self, assets: &Assets) -> Result<()> {
        for u in &self.units {
            if assets.types.id(&u.kind).is_none() {
                bail!("no unit {} in the game data", u.kind);
            }
        }
        for o in &self.orders {
            if let Some(kind) = &o.kind
                && assets.types.id(kind).is_none()
            {
                bail!("order at {} s: no unit {kind} to build", o.t);
            }
        }
        for e in &self.events {
            if let Some(w) = &e.weapon
                && !assets.data.weapons.iter().any(|d| d.name.eq_ignore_ascii_case(w))
            {
                bail!("event at {} s: no weapon {w}", e.t);
            }
        }
        let known = self.known_names();
        let check = |names: &Names, what: &dyn Fn() -> String| -> Result<()> {
            for n in names.list() {
                if !known.iter().any(|k| k.eq_ignore_ascii_case(n)) {
                    bail!("{}: no unit labelled {n} (nor of that type) in the scene", what());
                }
            }
            Ok(())
        };
        for (i, u) in self.units.iter().enumerate() {
            if let Some(b) = &u.builder {
                check(b, &|| format!("unit {} ({})'s builder", i + 1, u.kind))?;
            }
        }
        for o in &self.orders {
            check(&o.units, &|| format!("order at {} s", o.t))?;
            if let Some(t) = &o.target {
                check(t, &|| format!("order at {} s's target", o.t))?;
            }
        }
        for e in &self.events {
            if let Some(u) = &e.units {
                check(u, &|| format!("event at {} s", e.t))?;
            }
        }
        if let Some(f) = &self.camera.follow {
            check(&Names::One(f.clone()), &|| "the camera's `follow`".to_string())?;
        }
        for k in &self.camera.keys {
            if let Some(names) = &k.track {
                check(names, &|| format!("the camera key at {} s's `track`", k.t))?;
            }
        }
        for r in &self.rides {
            check(&r.unit, &|| format!("the ride at {} s", r.t))?;
        }
        Ok(())
    }

    /// The time of day at `clock` seconds of the scene (hours), if it sets one: its hours, or
    /// its keys eased between.
    pub fn hours(&self, clock: f32) -> Option<f32> {
        self.time.as_ref().map(|h| h.at(clock))
    }

    /// The direction towards the moon, if the scene puts it somewhere (`moon`: compass degrees,
    /// 0 north and 90 east; degrees above the horizon).
    pub fn moon_dir(&self) -> Option<Vec3> {
        self.moon.map(|[azimuth, elevation]| {
            let (a, e) = (azimuth.to_radians(), elevation.to_radians());
            Vec3::new(a.sin() * e.cos(), e.sin(), -a.cos() * e.cos())
        })
    }

    /// Players the scene needs: one for each side its units take, at least two.
    pub fn player_count(&self) -> usize {
        let used = self.units.iter().map(|u| u.side as usize + 1).chain(self.players.iter().map(|p| p.index as usize + 1)).max().unwrap_or(0);
        used.max(2)
    }

    /// The battle the scene is played in: on its map (`--map` wins), a player for each side it
    /// uses (computer players for all, retired by [`StageRun::begin`] unless `ai` wants them),
    /// seen as player 0, with no Commander ending the game.
    pub fn setup(&self, map: Option<&str>, assets: &Assets) -> Result<GameSetup> {
        let mut pruned = self.clone();
        if !pruned.prune_missing(|k| assets.types.id(k).is_some()).is_empty() {
            return pruned.setup(map, assets);
        }
        let wanted = map.or(self.map.as_deref()).ok_or_else(|| anyhow!("the scene names no map (`map`), and no --map was given"))?;
        let map = assets.maps.iter().find(|m| m.name.eq_ignore_ascii_case(wanted) || m.title.eq_ignore_ascii_case(wanted)).map_or_else(|| wanted.to_string(), |m| m.name.clone());
        self.check_names(assets)?;
        let players = (0..self.player_count())
            .map(|i| {
                let spec = self.players.iter().find(|p| p.index as usize == i);
                let faction = spec.and_then(|p| p.faction.clone()).unwrap_or_else(|| if i % 2 == 0 { "ARM" } else { "CORE" }.to_string());
                SlotSetup {
                    side: faction.to_ascii_uppercase(),
                    color: spec.and_then(|p| p.color).unwrap_or(i as u8),
                    controller: Controller::Ai(ta_ai::Difficulty::Hard),
                    start: i,
                    team: spec.and_then(|p| p.team).unwrap_or(i as u8),
                    name: String::new(),
                }
            })
            .collect();
        let rules = ta_sim::GameRules {
            start_metal: self.metal.unwrap_or(10_000),
            start_energy: self.energy.unwrap_or(10_000),
            fog_of_war: !self.reveal,
            mapped: true,
            commander_ends_game: false,
            unit_limit: 5000,
            ..Default::default()
        };
        Ok(GameSetup { map, players, local_player: Some(0), seed: self.seed.unwrap_or(0x57A6E), mission: None, rules, conquest: None, scenario: None })
    }
}

/// A unit of the scene and the names it answers to.
#[derive(Clone)]
struct Member {
    id: UnitId,
    label: Option<String>,
    /// Its type's name (upper case).
    kind: String,
}

/// A frame drawn as far built as the scene says (see the module's notes).
#[derive(Clone)]
struct Raise {
    unit: UnitId,
    /// Drawn done at the start.
    from: f32,
    /// Its share gained a tick of work (`build_seconds`); none: as its builders go.
    per_tick: Option<f32>,
    /// Ticks worked on so far; the game's work on it, and the tick, at the last look.
    worked: u32,
    work: u64,
    tick: u64,
    /// Drawn done at the look before the last and at the last (drawn between them).
    shown: (f32, f32),
    /// Drawn finished: its builders' players, building instantly to finish it in the game.
    finishing: Option<Vec<u8>>,
    /// Rising on its own (nobody building it): the tick it starts.
    alone: Option<u64>,
}

/// A frame rising on its own later (`rise_at`): the tick it starts, its unit in the scene, its
/// type and its place.
#[derive(Clone)]
struct Due {
    from: u64,
    spec: usize,
    type_id: usize,
    pos: FxVec3,
}

/// A build the scene ordered whose frame is looked out for: to draw it at the scene's pace, or
/// to label it.
#[derive(Clone)]
struct Watch {
    builders: Vec<UnitId>,
    type_id: usize,
    seconds: Option<f32>,
    label: Option<String>,
    until: u64,
    /// Frames still to look out for (a factory's, one after another).
    left: u32,
}

#[derive(Clone, Copy)]
enum Cue {
    Order(usize),
    Event(usize),
}

/// The value at `t` of keys `(t, value)` in the order of their times, eased between them: a
/// monotone cubic (it never overshoots a key, and holds still between two keys that agree), and
/// starting and stopping gently at the first key and the last; held beyond them.
pub fn eased(keys: &[(f32, f32)], t: f32) -> f32 {
    let n = keys.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 || t <= keys[0].0 {
        return keys[0].1;
    }
    if t >= keys[n - 1].0 {
        return keys[n - 1].1;
    }
    let slope = |k: usize| {
        let (a, b) = (keys[k], keys[k + 1]);
        if b.0 > a.0 { (b.1 - a.1) / (b.0 - a.0) } else { 0.0 }
    };
    // (Fritsch and Butland's: zero at the ends, where the curve turns back and between equal
    // keys; else a weighted harmonic mean of the slopes either side.)
    let tangent = |k: usize| {
        if k == 0 || k == n - 1 {
            return 0.0;
        }
        let (d0, d1) = (slope(k - 1), slope(k));
        if d0 * d1 <= 0.0 {
            return 0.0;
        }
        let (h0, h1) = (keys[k].0 - keys[k - 1].0, keys[k + 1].0 - keys[k].0);
        3.0 * (h0 + h1) / ((2.0 * h1 + h0) / d0 + (h1 + 2.0 * h0) / d1)
    };
    let i = keys.windows(2).position(|w| t < w[1].0).unwrap_or(n - 2);
    let (a, b) = (keys[i], keys[i + 1]);
    let h = b.0 - a.0;
    if h <= 0.0 {
        return b.1;
    }
    let s = (t - a.0) / h;
    let (s2, s3) = (s * s, s * s * s);
    (2.0 * s3 - 3.0 * s2 + 1.0) * a.1 + (s3 - 2.0 * s2 + s) * h * tangent(i) + (3.0 * s2 - 2.0 * s3) * b.1 + (s3 - s2) * h * tangent(i + 1)
}

/// A key of a camera path: its time in seconds, and the camera there: focus x, z, zoom, yaw and
/// pitch (degrees), the focus's height over the ground; a field left empty keeps the camera's.
pub type CameraKey = (f32, [Option<f32>; 6]);

/// One key of a camera path, "t:x,z,zoom,yaw,pitch[,height]" (fields may be empty).
pub fn path_key(key: &str) -> Option<CameraKey> {
    let (t, spec) = key.split_once(':')?;
    let mut fields = [None; 6];
    for (f, s) in fields.iter_mut().zip(spec.split(',')) {
        *f = s.trim().parse().ok();
    }
    Some((t.trim().parse().ok()?, fields))
}

/// A camera path, "key;key;..." (see [`path_key`]), in the order of its times.
pub fn parse_path(spec: &str) -> Result<Vec<CameraKey>> {
    let mut keys = spec
        .split(';')
        .filter(|k| !k.trim().is_empty())
        .map(|k| path_key(k).ok_or_else(|| anyhow!("camera path: {:?} is not seconds:x,z,zoom,yaw,pitch", k.trim())))
        .collect::<Result<Vec<_>>>()?;
    keys.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(keys)
}

/// The camera at `t` seconds along `path`: each stretch between two keys eased in and out, held
/// at the ends; `ground` gives the ground's height for a focus lifted over it.
pub fn apply_path(camera: &mut ta_render::Camera, path: &[CameraKey], t: f32, ground: impl Fn(f32, f32) -> f32) {
    if path.is_empty() {
        return;
    }
    let i = path.iter().rposition(|k| k.0 <= t).unwrap_or(0);
    let (a, b) = (&path[i], path.get(i + 1).unwrap_or(&path[i]));
    let s = if b.0 > a.0 { ((t - a.0) / (b.0 - a.0)).clamp(0.0, 1.0) } else { 0.0 };
    let s = s * s * (3.0 - 2.0 * s);
    let at = |j: usize| match (a.1[j], b.1[j]) {
        (Some(x), Some(y)) => Some(x + (y - x) * s),
        (x, y) => x.or(y),
    };
    if let (Some(x), Some(z)) = (at(0), at(1)) {
        camera.focus.x = x;
        camera.focus.z = z;
    }
    if let Some(zoom) = at(2) {
        camera.zoom = zoom;
    }
    if let Some(yaw) = at(3) {
        camera.yaw = yaw.to_radians();
    }
    if let Some(pitch) = at(4) {
        camera.pitch = pitch.to_radians();
    }
    if let Some(height) = at(5) {
        camera.focus.y = ground(camera.focus.x, camera.focus.z) + height;
    }
}

/// A blast set off at (x, z) now, as a scene's `explosion` event does (the weapon's own, by name, or
/// one of about `size` px): seen and (when `heard`) heard, harming nothing (the credits' cards are
/// struck by them, `credits_show.rs`; one long past when a film catches up with its song is seen
/// as it would be by now, and not heard).
pub fn explode(game: &mut Game, weapon: Option<&str>, size: f32, x: f32, z: f32, heard: bool) {
    let Some(weapon) = blast_weapon(&game.sim, weapon, Some(size)).or_else(|| blast_weapon(&game.sim, None, Some(size))) else { return };
    let p = fx_point((x, z));
    let world = &game.sim.world;
    let ground = world.height_at(p.x, p.z);
    let water = ground < world.sea_level;
    let now = crate::scene::game_seconds(game.sim.tick, game.alpha()) as f32;
    let e = SimEvent::Explosion { weapon, pos: FxVec3::new(p.x, ground.max(world.sea_level), p.z), water, when: Fx::ZERO, player: None };
    game.effects.on_event(&e, &game.sim, now);
    if heard {
        game.events_out.push(e);
    }
}

/// The weapon whose blast an explosion borrows: the one named, or the one whose blast is nearest
/// `size` px in radius among those that go off with a burst of fire and a bang.
fn blast_weapon(sim: &Sim, name: Option<&str>, size: Option<f32>) -> Option<usize> {
    let types = &sim.weapon_types.types;
    if let Some(name) = name {
        return types.iter().position(|w| w.name.eq_ignore_ascii_case(name));
    }
    use ta_data::RenderType as R;
    let size = size.unwrap_or(48.0);
    types
        .iter()
        .filter(|w| !w.def.explosion_gaf.is_empty() && !w.def.sound_hit.is_empty() && !w.def.paralyzer && !w.def.no_explode)
        .filter(|w| !matches!(w.def.render_type, R::Laser | R::Lightning | R::Flame))
        .min_by(|a, b| (a.aoe_radius.to_f32() - size).abs().total_cmp(&(b.aoe_radius.to_f32() - size).abs()))
        .map(|w| w.id)
}

/// Why a unit of a type cannot stand at `pos`, if it can't.
fn cannot_stand(sim: &Sim, type_id: usize, pos: FxVec3) -> Option<String> {
    let t = sim.types.get(type_id);
    match t.locomotion {
        Locomotion::Static => sim.placement_error(type_id, pos).map(|e| format!("{e:?}")),
        Locomotion::Air => None,
        _ => {
            let spec = sim.move_spec(type_id);
            let anchor = ta_sim::world::World::pos_to_anchor(pos, spec.footprint);
            (!sim.world.passable(&spec, anchor, None)).then(|| "it cannot stand there".to_string())
        }
    }
}

/// The least region (anchor cells, 16 px square) a unit is put down in, of its own kind's (where
/// it can go): smaller, it is a ridge's top or a ledge with slopes too steep for it all round,
/// which it cannot leave for the battle it was put down for.
const LEAST_REGION: usize = 1024;

/// Each kind of movement's region sizes, as asked for (`World::region_sizes`).
type RegionSizes = std::collections::HashMap<ta_sim::world::MoveSpec, Vec<usize>>;

/// Why a unit of a type cannot stand at `pos`, or would be stuck there (on ground it cannot
/// leave: [`LEAST_REGION`]), if so.
fn cannot_stand_or_leave(sim: &Sim, type_id: usize, pos: FxVec3, sizes: &mut RegionSizes) -> Option<String> {
    if let Some(why) = cannot_stand(sim, type_id, pos) {
        return Some(why);
    }
    if matches!(sim.types.get(type_id).locomotion, Locomotion::Static | Locomotion::Air) {
        return None;
    }
    let spec = sim.move_spec(type_id);
    let region = sim.world.component(&spec, ta_sim::world::World::pos_to_anchor(pos, spec.footprint));
    let size = sizes.entry(spec).or_insert_with(|| sim.world.region_sizes(&spec)).get(region as usize).copied().unwrap_or(0);
    (size < LEAST_REGION).then(|| format!("it could not leave the {size} cells round it (a ridge's top)"))
}

/// How far (px) a unit asked to stand where it cannot is moved to where it can, at most.
const NUDGE: f32 = 256.0;

/// Where a unit asked to stand at `spot` but unable to (a cliff, the sea, a slope too steep to
/// build on, a ridge's top it could not leave: a map changed under a scene) stands instead: the
/// nearest place it can, a ring at a time outwards (16 px apart, every 16 px round each), clear
/// of the units moved before it (`taken`: where they went, how far round them); none within
/// [`NUDGE`]. The same every time.
fn standing_spot(sim: &Sim, type_id: usize, spot: (f32, f32), taken: &[(f32, f32, f32)], sizes: &mut RegionSizes) -> Option<(f32, f32)> {
    let t = sim.types.get(type_id);
    let room = t.footprint.0.max(t.footprint.1) as f32 * 8.0;
    let (w, h) = sim.world.size_px();
    let (w, h) = (w.to_f32(), h.to_f32());
    let mut ring = 16.0;
    while ring <= NUDGE {
        let steps = ((ring * std::f32::consts::TAU / 16.0).round() as usize).max(8);
        for k in 0..steps {
            let a = k as f32 / steps as f32 * std::f32::consts::TAU;
            let at = (spot.0 + a.cos() * ring, spot.1 + a.sin() * ring);
            if at.0 < room || at.1 < room || at.0 > w - room || at.1 > h - room {
                continue;
            }
            if taken.iter().any(|&(x, z, r)| (at.0 - x).powi(2) + (at.1 - z).powi(2) < (r + room).powi(2)) {
                continue;
            }
            if cannot_stand_or_leave(sim, type_id, fx_point(at), sizes).is_none() {
                return Some(at);
            }
        }
        ring += 16.0;
    }
    None
}

/// Units by their owners (in the order of their owners, each in the order given).
fn by_owner(sim: &Sim, units: &[UnitId]) -> Vec<(u8, Vec<UnitId>)> {
    let mut out: Vec<(u8, Vec<UnitId>)> = Vec::new();
    for &id in units {
        let Some(owner) = sim.unit(id).map(|u| u.owner) else { continue };
        match out.iter_mut().find(|(o, _)| *o == owner) {
            Some((_, list)) => list.push(id),
            None => out.push((owner, vec![id])),
        }
    }
    out.sort_by_key(|(o, _)| *o);
    out
}

/// A scene being played: its cast, what is still to come, and its frames being drawn.
/// (A copy plays on as the original: the benchmark's settings test plays a scene's seconds again.)
#[derive(Clone)]
pub struct StageRun {
    stage: Stage,
    /// The tick the scene started at.
    start: u64,
    cast: Vec<Member>,
    /// Orders and events in the order of their ticks.
    cues: Vec<(u64, Cue)>,
    /// Commands for their ticks (sent the tick before, to be carried out on it).
    outbox: Vec<(u64, Command)>,
    raises: Vec<Raise>,
    /// Frames rising on their own later, put down when their time comes.
    due: Vec<Due>,
    watches: Vec<Watch>,
    /// Players building instantly to finish a frame, and since when.
    instant: Vec<(u8, u64)>,
    /// Frames that rose on their own, drawn finished: to be finished in the game.
    risen: Vec<UnitId>,
    /// The player's selection, as a `select` event made it (for the game to take).
    select: Option<Vec<UnitId>>,
    /// Constructors put on a duty (or taken off one) by `duty` events (for the game to do).
    duties: Vec<(Vec<UnitId>, Option<ta_ai::Duty>)>,
    /// Units holding their fire until their first order.
    held: Vec<UnitId>,
    /// Explosions due (tick, weapon, place), and each event's weapon.
    blasts: Vec<(u64, usize, FxVec3)>,
    weapons: Vec<Option<usize>>,
    path: Vec<CameraKey>,
    /// The free camera's keys in the order of their times, and where each key that tracks units
    /// last saw them.
    keys: Vec<ViewKey>,
    tracked: Vec<Option<Vec3>>,
    /// How far each of the scene's rides has got: 0 not begun, 1 riding, 2 over.
    rides: Vec<u8>,
    /// Fireworks volleys due (round where, how far, how long), for the game's effects.
    volleys: Vec<(Vec3, f32, f32)>,
}

/// The free camera at a moment.
#[derive(Clone, Copy, Debug)]
pub struct FreeView {
    pub eye: Vec3,
    /// Radians: towards (0 north); below the horizon, as the camera's pitch (-π/2 straight up).
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    /// Vertical, radians.
    pub fov: f32,
    /// To what it looks at (where the haze begins), and its near plane.
    pub distance: f32,
    pub near: f32,
}

impl StageRun {
    /// Sets the scene up in the simulation (the game's side of it is [`StageRun::begin`]): takes
    /// whatever stands on the map off it, clears the trees and rocks asked for and those where
    /// units go, places the cast, and lines up its standing orders (fire at will or hold fire,
    /// hold position), its builders, its stores and its cues.
    pub fn start(stage: &Stage, sim: &mut Sim) -> StageRun {
        let mut stage = stage.clone();
        let gone = stage.prune_missing(|k| sim.types.id(k).is_some());
        if !gone.is_empty() {
            log::warn!("scene: not installed, left out: {}", gone.join(", "));
        }
        let stage = &stage;
        let start = sim.tick;
        let (w, h) = sim.world.size_px();
        let middle = (w.to_f32() / 2.0, h.to_f32() / 2.0);
        let mut run = StageRun {
            stage: stage.clone(),
            start,
            cast: Vec::new(),
            cues: Vec::new(),
            outbox: Vec::new(),
            raises: Vec::new(),
            due: Vec::new(),
            watches: Vec::new(),
            instant: Vec::new(),
            risen: Vec::new(),
            select: None,
            duties: Vec::new(),
            held: Vec::new(),
            blasts: Vec::new(),
            weapons: Vec::new(),
            path: stage.camera.path.as_deref().and_then(|p| parse_path(p).ok()).unwrap_or_default(),
            keys: Vec::new(),
            tracked: Vec::new(),
            rides: stage.rides.iter().map(|_| 0).collect(),
            volleys: Vec::new(),
        };
        run.keys = stage.camera.keys.clone();
        run.keys.sort_by(|a, b| a.t.total_cmp(&b.t));
        run.tracked = vec![None; run.keys.len()];
        // Nothing but the scene: the battle's own Commanders go.
        let present: Vec<UnitId> = sim.units().map(|u| u.id).collect();
        for id in present {
            sim.remove_unit(id);
        }
        // Where each unit goes.
        let mut blocks = Vec::new();
        for (i, spec) in stage.units.iter().enumerate() {
            let Some(type_id) = sim.types.id(&spec.kind) else {
                log::warn!("stage: no unit {}", spec.kind);
                continue;
            };
            let Some(at) = point(spec.at, spec.rel, middle) else { continue };
            let spots = block(at, spec.heading, spec.count, spec.rows, spec.spacing, sim.types.get(type_id).footprint);
            blocks.push((i, spec, type_id, spots));
        }
        // Trees and rocks go from the clearings and from under every unit (metal deposits stay).
        let circles: Vec<(f32, f32, f32)> = stage.clearings.iter().filter_map(|c| point(c.at, c.rel, middle).map(|(x, z)| (x, z, c.radius))).collect();
        let mut boxes: Vec<(f32, f32, f32, f32)> = Vec::new();
        for (_, _, type_id, spots) in &blocks {
            let (fx, fz) = sim.types.get(*type_id).footprint;
            boxes.extend(spots.iter().map(|&(x, z)| (x, z, fx as f32 * 8.0 + 4.0, fz as f32 * 8.0 + 4.0)));
        }
        let doomed: Vec<usize> = sim
            .features
            .iter()
            .enumerate()
            .filter(|(_, f)| f.alive)
            .filter(|(_, f)| {
                let (x, z) = (f.pos.x.to_f32(), f.pos.z.to_f32());
                let def = &sim.data.features[f.def];
                let (x0, z0) = (f.origin.0 as f32 * 16.0, f.origin.1 as f32 * 16.0);
                let (x1, z1) = (x0 + def.footprint_x as f32 * 16.0, z0 + def.footprint_z as f32 * 16.0);
                circles.iter().any(|&(cx, cz, r)| (x - cx).powi(2) + (z - cz).powi(2) < r * r) || boxes.iter().any(|&(bx, bz, hx, hz)| x1 > bx - hx && x0 < bx + hx && z1 > bz - hz && z0 < bz + hz)
            })
            .map(|(i, _)| i)
            .collect();
        let cleared = sim.clear_features_where(|i, _| doomed.binary_search(&i).is_ok());
        // Features put down (wrecks, rocks).
        for f in &stage.features {
            let Some(spot) = point(f.at, f.rel, middle) else { continue };
            if sim.place_feature(&f.kind, fx_point(spot), ta_heading(f.heading)).is_none() {
                log::warn!("stage: no feature {}", f.kind);
            }
        }
        // The cast.
        let players = sim.players.len();
        let mut builds: Vec<(Names, Vec<UnitId>)> = Vec::new();
        let mut sizes = RegionSizes::new();
        for (i, spec, type_id, spots) in blocks {
            if spec.side as usize >= players {
                log::warn!("stage: {} for player {}, of {players}", spec.kind, spec.side);
                continue;
            }
            let heading = ta_heading(spec.heading);
            let complete = spec.built >= 1.0;
            let kind = sim.types.get(type_id).name.to_ascii_uppercase();
            // A frame nobody builds rises on its own, from the start or from `rise_at`.
            let alone = (!complete && spec.builder.is_none() && spec.build_seconds.is_some()).then(|| start + (spec.rise_at.unwrap_or(0.0) * TICKS).round() as u64);
            let mut frames = Vec::new();
            let mut moved: Vec<(f32, f32, f32)> = Vec::new();
            for spot in spots {
                // (Where it cannot stand, or could not leave, the nearest place it can: a scene
                // outlives its map's changes.)
                let spot = match cannot_stand_or_leave(sim, type_id, fx_point(spot), &mut sizes) {
                    None => spot,
                    Some(why) => match standing_spot(sim, type_id, spot, &moved, &mut sizes) {
                        Some(near) => {
                            log::debug!("stage: {} at ({:.0}, {:.0}): {why}; put down at ({:.0}, {:.0})", spec.kind, spot.0, spot.1, near.0, near.1);
                            let t = sim.types.get(type_id);
                            moved.push((near.0, near.1, t.footprint.0.max(t.footprint.1) as f32 * 8.0));
                            near
                        }
                        None => {
                            log::warn!("stage: {} at ({:.0}, {:.0}): {why}, and nowhere near", spec.kind, spot.0, spot.1);
                            spot
                        }
                    },
                };
                let pos = fx_point(spot);
                // Rising later: not in the game at all until then (nothing there to see, to bump
                // into or to shoot at), put down when its time comes.
                if let Some(from) = alone.filter(|&from| from > sim.tick) {
                    run.due.push(Due { from, spec: i, type_id, pos });
                    continue;
                }
                let altitude = spec.altitude.map(Fx::from_f32);
                let spawned = if spec.airborne { sim.spawn_airborne(type_id, spec.side, pos, heading, altitude) } else { sim.spawn_unit(type_id, spec.side, pos, heading, complete) };
                let Some(id) = spawned else { continue };
                if !spec.airborne && altitude.is_some() {
                    sim.set_cruise_alt(id, altitude);
                }
                // In the air: it flies on straight ahead (to the map's edge) until ordered.
                if spec.airborne && sim.types.get(type_id).locomotion == Locomotion::Air {
                    let (s, c) = spec.heading.to_radians().sin_cos();
                    let (w, h) = (w.to_f32(), h.to_f32());
                    let ahead = ((spot.0 + s * FLY_ON).clamp(32.0, w - 32.0), (spot.1 - c * FLY_ON).clamp(32.0, h - 32.0));
                    run.outbox.push((start, Command::Order { player: spec.side, units: vec![id], order: Order::Move(fx_point(ahead)), queue: false }));
                }
                run.cast.push(Member { id, label: spec.label.clone(), kind: kind.clone() });
                if spec.stockpile > 0 {
                    sim.grant_stockpile(id, spec.stockpile);
                }
                if spec.hold {
                    run.held.push(id);
                }
                if !complete {
                    let per_tick = spec.build_seconds.map(|s| (1.0 - spec.built) / (s * TICKS));
                    let mut raise = Raise::new(sim, id, spec.built, per_tick);
                    raise.alone = alone;
                    run.raises.push(raise);
                    frames.push(id);
                }
            }
            if let Some(builders) = &spec.builder {
                builds.push((builders.clone(), frames));
            }
        }
        // Standing orders: everyone holds position; fire at will, or held.
        let cast: Vec<UnitId> = run.cast.iter().map(|m| m.id).collect();
        for (owner, ids) in by_owner(sim, &cast) {
            let (held, free): (Vec<UnitId>, Vec<UnitId>) = ids.iter().partition(|id| run.held.contains(id));
            for (units, fire) in [(free, 2), (held, 0)] {
                if !units.is_empty() {
                    run.outbox.push((start, Command::SetStates { player: owner, units, fire: Some(fire), movement: Some(0) }));
                }
            }
        }
        // Builders at their frames, one after another (several builders share them out).
        for (names, frames) in builds {
            let builders = run.find(&names, sim);
            for (k, &b) in builders.iter().enumerate() {
                let Some(owner) = sim.unit(b).map(|u| u.owner) else { continue };
                for j in 0..frames.len() {
                    let frame = frames[(k + j) % frames.len()];
                    run.send(start, Command::Order { player: owner, units: vec![b], order: Order::Repair(frame), queue: j > 0 }, sim);
                }
            }
        }
        if stage.unlimited {
            for p in 0..players as u8 {
                run.outbox.push((start, Command::Cheat { player: p, cheat: Cheat::Unlimited(true) }));
            }
        }
        for p in stage.players.iter().filter(|p| p.invulnerable) {
            run.outbox.push((start, Command::Cheat { player: p.index, cheat: Cheat::Invulnerable(true) }));
        }
        // Silos to launch from have a missile ready; the explosions their weapons.
        for e in &stage.events {
            if e.what == Happening::Nuke && let Some(names) = &e.units {
                for silo in run.find(names, sim) {
                    if sim.unit(silo).is_some_and(|u| u.stockpile == 0) {
                        sim.grant_stockpile(silo, 1);
                    }
                }
            }
            let weapon = (e.what == Happening::Explosion).then(|| blast_weapon(sim, e.weapon.as_deref(), e.size)).flatten();
            if let Some(w) = weapon.filter(|_| e.weapon.is_none()) {
                log::info!("stage: an explosion of {:.0} px at {} s: {}'s blast ({:.0} px)", e.size.unwrap_or(48.0), e.t, sim.weapon_types.get(w).name, sim.weapon_types.get(w).aoe_radius.to_f32());
            }
            run.weapons.push(weapon);
        }
        run.cues = stage
            .orders
            .iter()
            .enumerate()
            .map(|(i, o)| (tick_at(start, o.t), Cue::Order(i)))
            .chain(stage.events.iter().enumerate().map(|(i, e)| (tick_at(start, e.t), Cue::Event(i))))
            .collect();
        run.cues.sort_by_key(|(t, _)| *t);
        log::info!(
            "stage: {} units placed ({} [[unit]] entries) for {players} players, {cleared} trees and rocks cleared; {} orders and {} events to come",
            run.cast.len(),
            stage.units.len(),
            stage.orders.len(),
            stage.events.len()
        );
        run
    }

    /// Sets the scene up in `game` (see [`StageRun::start`]): its computer players retired unless
    /// the scene wants them, the fog of war, the weather, nothing to decide the battle.
    pub fn begin(stage: &Stage, game: &mut Game) -> StageRun {
        let run = StageRun::start(stage, &mut game.sim);
        game.staged = true;
        game.chatter = None;
        game.selection.clear();
        for p in 0..game.sim.players.len() as u8 {
            if !stage.ai.includes(p) {
                game.retire_ai(p);
            }
        }
        if stage.reveal {
            game.set_reveal(crate::chat::Reveal::All);
        } else {
            game.sim.reveal_map_for(game.local_player);
        }
        if let Some(on) = stage.rain
            && std::env::var_os("TA_RAIN").is_none()
        {
            game.effects.set_rain(on);
        }
        game.scene.after_tick(&game.sim);
        run
    }

    /// The units `names` call for, alive: those so labelled, or else those of that type.
    /// A ride's cue due at `clock` seconds of the scene: into the first of its units still there,
    /// or back out of it. (One a frame; a ride whose units are all gone is passed over.)
    pub fn ride_cue(&mut self, sim: &Sim, clock: f32) -> Option<RideCue> {
        for i in 0..self.rides.len() {
            let r = &self.stage.rides[i];
            match self.rides[i] {
                0 if clock >= r.t => {
                    let unit = self.find(&r.unit, sim).first().copied();
                    self.rides[i] = if unit.is_some() { 1 } else { 2 };
                    if let Some(id) = unit {
                        return Some(RideCue::Enter(id));
                    }
                }
                1 if r.leave.is_some_and(|l| clock >= l) => {
                    self.rides[i] = 2;
                    return Some(RideCue::Leave);
                }
                _ => {}
            }
        }
        None
    }

    fn find(&self, names: &Names, sim: &Sim) -> Vec<UnitId> {
        let mut out = Vec::new();
        for name in names.list() {
            let labelled: Vec<UnitId> = self.cast.iter().filter(|m| m.label.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(name))).map(|m| m.id).collect();
            let chosen = if labelled.is_empty() { self.cast.iter().filter(|m| m.kind.eq_ignore_ascii_case(name)).map(|m| m.id).collect() } else { labelled };
            for id in chosen {
                if sim.unit(id).is_some_and(|u| !u.dead) && !out.contains(&id) {
                    out.push(id);
                }
            }
        }
        out
    }

    /// A command for `tick`, or the first tick after with room for it (see [`PER_TICK`]; orders
    /// to aircraft go at once: they plan no routes, and in the air a tick without one slows them).
    fn send(&mut self, tick: u64, command: Command, sim: &Sim) {
        let flying = matches!(&command, Command::Order { units, .. } if units.iter().all(|&id| sim.unit(id).is_some_and(|u| sim.unit_type(u).locomotion == Locomotion::Air)));
        let per_tick = self.stage.orders_per_tick.unwrap_or(PER_TICK).max(1);
        let mut at = tick;
        while !flying && self.outbox.iter().filter(|(t, _)| *t == at).count() >= per_tick {
            at += 1;
        }
        self.outbox.push((at, command));
    }

    /// The commands to give now, to be carried out on the next tick: the orders and events due
    /// by then, and what was waiting.
    pub fn commands(&mut self, sim: &mut Sim) -> Vec<Command> {
        let next = sim.tick + 1;
        // Frames rising from the coming tick put down first (orders for it find them).
        let (due, later): (Vec<Due>, Vec<Due>) = std::mem::take(&mut self.due).into_iter().partition(|d| d.from <= next);
        self.due = later;
        for d in due {
            self.put_down(d, sim);
        }
        while self.cues.first().is_some_and(|(t, _)| *t <= next) {
            let (tick, cue) = self.cues.remove(0);
            match cue {
                Cue::Order(i) => self.order(i, sim, tick),
                Cue::Event(i) => self.event(i, sim, tick),
            }
        }
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.outbox).into_iter().partition(|(t, _)| *t <= next);
        self.outbox = later;
        due.into_iter().map(|(_, c)| c).collect()
    }

    /// A frame due to rise put down in the game, rising on its own from its tick, under the
    /// cast's standing orders (holding position; fire at will, or held).
    fn put_down(&mut self, d: Due, sim: &mut Sim) {
        let spec = &self.stage.units[d.spec];
        let Some(id) = sim.spawn_unit(d.type_id, spec.side, d.pos, ta_heading(spec.heading), false) else {
            log::warn!("stage: {} could not be put down to rise", spec.kind);
            return;
        };
        if let Some(altitude) = spec.altitude {
            sim.set_cruise_alt(id, Some(Fx::from_f32(altitude)));
        }
        if spec.stockpile > 0 {
            sim.grant_stockpile(id, spec.stockpile);
        }
        if spec.hold {
            self.held.push(id);
        }
        let fire = if spec.hold { 0 } else { 2 };
        self.outbox.push((d.from, Command::SetStates { player: spec.side, units: vec![id], fire: Some(fire), movement: Some(0) }));
        let kind = sim.types.get(d.type_id).name.to_ascii_uppercase();
        self.cast.push(Member { id, label: spec.label.clone(), kind });
        let per_tick = spec.build_seconds.map(|s| (1.0 - spec.built) / (s * TICKS));
        let mut raise = Raise::new(sim, id, spec.built, per_tick);
        raise.alone = Some(d.from);
        self.raises.push(raise);
    }

    fn order(&mut self, i: usize, sim: &mut Sim, tick: u64) {
        let o = self.stage.orders[i].clone();
        let units = self.find(&o.units, sim);
        if units.is_empty() {
            log::warn!("stage: the order at {} s has nobody left to carry it out", o.t);
            return;
        }
        log::info!("stage: {} s: {:?} for {} units", o.t, o.action, units.len());
        // The first order frees those holding their fire (they still hold their position).
        let freed: Vec<UnitId> = units.iter().copied().filter(|id| self.held.contains(id)).collect();
        self.held.retain(|id| !freed.contains(id));
        for (owner, ids) in by_owner(sim, &freed) {
            self.send(tick, Command::SetStates { player: owner, units: ids, fire: Some(2), movement: None }, sim);
        }
        let (w, h) = sim.world.size_px();
        let (w, h) = (w.to_f32(), h.to_f32());
        let middle = (w / 2.0, h / 2.0);
        let on_map = |(x, z): (f32, f32)| (x.clamp(16.0, w - 16.0), z.clamp(16.0, h - 16.0));
        let pos = |id: UnitId| sim.unit(id).map_or((0.0, 0.0), |u| (u.pos.x.to_f32(), u.pos.z.to_f32()));
        let spot = point(o.at, o.rel, middle);
        // Ground, hover and naval units sent somewhere keeping their shape (by an offset, or a
        // formation sent to a spot) go as one formation for each owner: under the rule "modern
        // movement" they march there together, each to its own place; without it each simply
        // goes to its place, as it did. (Aircraft keep their own places, as below.)
        let mut units = units;
        if matches!(o.action, Action::Move | Action::Fight | Action::Patrol) && o.formation && (o.by.is_some() || spot.is_some()) {
            let (surface, air): (Vec<UnitId>, Vec<UnitId>) = units.iter().partition(|&&id| sim.unit(id).is_some_and(|u| sim.unit_type(u).locomotion != Locomotion::Air));
            if surface.len() > 1 {
                let n = surface.len() as f32;
                let (sx, sz) = surface.iter().fold((0.0, 0.0), |(sx, sz), &id| (sx + pos(id).0, sz + pos(id).1));
                let to = match (o.by, spot) {
                    (Some([dx, dz]), _) => on_map((sx / n + dx, sz / n + dz)),
                    (None, Some(p)) => on_map(p),
                    (None, None) => (sx / n, sz / n),
                };
                let order = match o.action {
                    Action::Fight => Order::Fight(fx_point(to)),
                    Action::Patrol => Order::Patrol(fx_point(to)),
                    _ => Order::Move(fx_point(to)),
                };
                for (owner, ids) in by_owner(sim, &surface) {
                    self.send(tick, Command::Formation { player: owner, units: ids, order: order.clone(), queue: o.queue, shape: ta_sim::formation::Shape::Keep }, sim);
                }
                units = air;
                if units.is_empty() {
                    return;
                }
            }
        }
        // Each unit's own destination: by its offset, or keeping its place round the group's
        // middle; else all to the one spot.
        let goals: Vec<(f32, f32)> = match (o.by, spot) {
            (Some([dx, dz]), _) => units.iter().map(|&id| on_map((pos(id).0 + dx, pos(id).1 + dz))).collect(),
            (None, Some((x, z))) if o.formation && units.len() > 1 && matches!(o.action, Action::Move | Action::Fight | Action::Patrol) => {
                let n = units.len() as f32;
                let (sx, sz) = units.iter().fold((0.0, 0.0), |(sx, sz), &id| (sx + pos(id).0, sz + pos(id).1));
                units.iter().map(|&id| on_map((x + pos(id).0 - sx / n, z + pos(id).1 - sz / n))).collect()
            }
            (None, Some(p)) => vec![on_map(p); units.len()],
            (None, None) => Vec::new(),
        };
        let targets = o.target.as_ref().map(|t| self.find(t, sim)).unwrap_or_default();
        if o.target.is_some() && targets.is_empty() {
            log::warn!("stage: the order at {} s has nothing left to aim at", o.t);
            return;
        }
        let each = |order: &dyn Fn(usize) -> Order| -> Vec<(UnitId, Order)> { units.iter().enumerate().map(|(k, &id)| (id, order(k))).collect() };
        let per_unit: Vec<(UnitId, Order)> = match o.action {
            Action::Move => each(&|k| Order::Move(fx_point(goals[k]))),
            Action::Fight => each(&|k| Order::Fight(fx_point(goals[k]))),
            Action::Patrol => each(&|k| Order::Patrol(fx_point(goals[k]))),
            // Attackers share the targets out between them.
            Action::Attack if !targets.is_empty() => each(&|k| Order::Attack(Target::Unit(targets[k % targets.len()]))),
            Action::Attack => each(&|k| Order::Attack(Target::Ground(fx_point(goals[k])))),
            Action::Dgun if !targets.is_empty() => each(&|_| Order::ManualFire(Target::Unit(targets[0]))),
            Action::Dgun => each(&|k| Order::ManualFire(Target::Ground(fx_point(goals[k])))),
            Action::Guard => each(&|_| Order::Guard(targets[0])),
            Action::Repair => each(&|_| Order::Repair(targets[0])),
            Action::Stop => each(&|_| Order::Stop),
            Action::Build => {
                let Some(type_id) = o.kind.as_deref().and_then(|k| sim.types.id(k)) else { return };
                if o.seconds.is_some() || o.label.is_some() {
                    self.watches.push(Watch { builders: units.clone(), type_id, seconds: o.seconds, label: o.label.clone(), until: tick + WATCH_TICKS, left: 1 });
                }
                each(&|k| Order::Build { type_id, pos: fx_point(goals[k]) })
            }
            Action::SelfDestruct => {
                for (owner, ids) in by_owner(sim, &units) {
                    self.send(tick, Command::SelfDestruct { player: owner, units: ids }, sim);
                }
                Vec::new()
            }
            Action::On | Action::Off => {
                for (owner, ids) in by_owner(sim, &units) {
                    self.send(tick, Command::SetActive { player: owner, units: ids, on: o.action == Action::On }, sim);
                }
                Vec::new()
            }
            Action::Produce => {
                let Some(type_id) = o.kind.as_deref().and_then(|k| sim.types.id(k)) else { return };
                let count = o.count.unwrap_or(1).min(i32::MAX as u32);
                for &factory in &units {
                    let Some(owner) = sim.unit(factory).map(|u| u.owner) else { continue };
                    self.send(tick, Command::Produce { player: owner, factory, type_id, count: count as i32 }, sim);
                    if o.seconds.is_some() || o.label.is_some() {
                        self.watches.push(Watch { builders: vec![factory], type_id, seconds: o.seconds, label: o.label.clone(), until: tick + WATCH_TICKS, left: count });
                    }
                }
                Vec::new()
            }
        };
        // Units going somewhere of their own each get a command (ten a tick); the same order
        // for all goes out once for each owner.
        let shared = per_unit.windows(2).all(|p| p[0].1 == p[1].1);
        if shared && let Some((_, order)) = per_unit.first() {
            for (owner, ids) in by_owner(sim, &units) {
                self.send(tick, Command::Order { player: owner, units: ids, order: order.clone(), queue: o.queue }, sim);
            }
        } else {
            for (id, order) in per_unit {
                let Some(owner) = sim.unit(id).map(|u| u.owner) else { continue };
                self.send(tick, Command::Order { player: owner, units: vec![id], order, queue: o.queue }, sim);
            }
        }
    }

    fn event(&mut self, i: usize, sim: &mut Sim, tick: u64) {
        let e = self.stage.events[i].clone();
        let (w, h) = sim.world.size_px();
        let spot = point(e.at, e.rel, (w.to_f32() / 2.0, h.to_f32() / 2.0)).map(fx_point);
        let units = e.units.as_ref().map(|n| self.find(n, sim)).unwrap_or_default();
        log::info!("stage: {} s: {:?}{}", e.t, e.what, if e.units.is_some() { format!(" of {} units", units.len()) } else { String::new() });
        match e.what {
            Happening::Explosion => match (self.weapons.get(i).copied().flatten(), spot) {
                (Some(weapon), Some(p)) => self.blasts.push((tick, weapon, p)),
                _ => log::warn!("stage: no weapon for the explosion at {} s", e.t),
            },
            // As TA's cheat destroys them: each dies as in battle.
            Happening::Kill => {
                for (owner, ids) in by_owner(sim, &units) {
                    self.send(tick, Command::Cheat { player: owner, cheat: Cheat::Destroy(ids) }, sim);
                }
            }
            // (The game's to change: before its next frame.)
            Happening::Select => self.select = Some(units),
            Happening::Duty => self.duties.push((units, e.duty.and_then(DutyName::duty))),
            Happening::Fire => {
                let Some(p) = spot else { return };
                let lit = sim.ignite_around(p, Fx::from_f32(e.size.unwrap_or(FIRE_RADIUS)));
                log::info!("stage: {} s: {lit} set alight", e.t);
            }
            Happening::Signal => {
                let Some(p) = spot else { return };
                let kind = e.signal.map_or(ta_sim::SignalKind::Look, SignalName::kind);
                self.send(tick, Command::Signal { player: e.side.unwrap_or(0), kind, at: p }, sim);
            }
            Happening::Fireworks => {
                let Some(p) = spot else { return };
                let ground = sim.world.height_at(p.x, p.z).to_f32();
                self.volleys.push((Vec3::new(p.x.to_f32(), ground, p.z.to_f32()), e.size.unwrap_or(VOLLEY_REACH), e.seconds.unwrap_or(VOLLEY_SECONDS)));
            }
            Happening::Nuke => {
                let (Some(&silo), Some(p)) = (units.first(), spot) else {
                    log::warn!("stage: no silo left to launch at {} s", e.t);
                    return;
                };
                if sim.unit(silo).is_some_and(|u| u.stockpile == 0) {
                    sim.grant_stockpile(silo, 1);
                }
                let Some(owner) = sim.unit(silo).map(|u| u.owner) else { return };
                self.send(tick, Command::Order { player: owner, units: vec![silo], order: Order::ManualFire(Target::Ground(p)), queue: false }, sim);
            }
        }
    }

    /// After the frame's ticks: the explosions due (weapon, place), the frames drawn on (and the
    /// commands that finish them in the game), the builds ordered looked out for.
    pub fn after_ticks(&mut self, sim: &Sim) -> (Vec<Command>, Vec<(usize, FxVec3)>) {
        let now = sim.tick;
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.blasts).into_iter().partition(|(t, _, _)| *t <= now);
        self.blasts = later;
        let blasts = due.into_iter().map(|(_, w, p)| (w, p)).collect();
        // The frames of builds ordered by the scene, once begun.
        let watches = std::mem::take(&mut self.watches);
        for mut w in watches {
            let frame = w.builders.iter().filter_map(|&b| sim.unit(b)?.build).map(|job| job.target).find(|&t| sim.unit(t).is_some_and(|u| u.type_id == w.type_id && !u.is_complete()));
            let new = frame.filter(|&f| !self.raises.iter().any(|r| r.unit == f) && !self.cast.iter().any(|m| m.id == f));
            if let Some(f) = new {
                let kind = sim.types.get(w.type_id).name.to_ascii_uppercase();
                log::info!("stage: {kind} begun at {:.2} s", (now - self.start) as f32 / TICKS);
                self.cast.push(Member { id: f, label: w.label.clone(), kind });
                let per_tick = w.seconds.map(|s| 1.0 / (s * TICKS));
                self.raises.push(Raise::new(sim, f, 0.0, per_tick));
                // (A factory's next one is looked out for from now.)
                w.left -= 1;
                w.until = now + WATCH_TICKS;
            }
            if w.left > 0 && now <= w.until {
                self.watches.push(w);
            }
        }
        // The frames drawn on: on at the scene's pace or the builders', finished in the game when
        // drawn finished.
        let mut commands = Vec::new();
        let mut instant = std::mem::take(&mut self.instant);
        let mut risen = Vec::new();
        let start = self.start;
        self.raises.retain_mut(|r| {
            let Some(u) = sim.unit(r.unit).filter(|u| !u.dead) else { return false };
            if u.is_complete() {
                log::info!("stage: {} finished at {:.2} s", sim.unit_type(u).name, (now - start) as f32 / TICKS);
                return false;
            }
            let ticks = now.saturating_sub(r.tick);
            if ticks == 0 {
                return true;
            }
            match r.alone {
                Some(from) => r.worked = now.saturating_sub(from) as u32,
                None if u.build_work > r.work => r.worked += ticks as u32,
                None => {}
            }
            r.work = u.build_work;
            r.tick = now;
            let drawn = match r.per_tick {
                Some(p) => r.from + p * r.worked as f32,
                None => r.from + u.build_progress.to_f32(),
            };
            r.shown = (r.shown.1, drawn.min(1.0));
            if drawn >= 1.0 && r.alone.is_some() {
                if r.finishing.is_none() {
                    risen.push(r.unit);
                    r.finishing = Some(Vec::new());
                }
                return true;
            }
            if drawn >= 1.0 && r.finishing.is_none() {
                let mut owners: Vec<u8> = sim.units().filter(|b| b.build.is_some_and(|j| j.target == r.unit)).map(|b| b.owner).collect();
                if owners.is_empty() {
                    owners.push(u.owner);
                }
                owners.sort_unstable();
                owners.dedup();
                for &p in &owners {
                    if !instant.iter().any(|(q, _)| *q == p) {
                        instant.push((p, now));
                        commands.push(Command::Cheat { player: p, cheat: Cheat::InstantBuild(true) });
                    }
                }
                r.finishing = Some(owners);
            }
            true
        });
        // Instant building off once its frames are finished (or given up on).
        instant.retain(|&(p, since)| {
            let waiting = self.raises.iter().any(|r| r.finishing.as_ref().is_some_and(|owners| owners.contains(&p)));
            if waiting && now < since + FINISH_TICKS {
                return true;
            }
            commands.push(Command::Cheat { player: p, cheat: Cheat::InstantBuild(false) });
            false
        });
        self.instant = instant;
        self.risen.extend(risen);
        (commands, blasts)
    }

    /// The frames that rose on their own and are drawn finished, finished in the game (nobody
    /// builds them: they are finished as the instant-build cheat finishes one); and those drawn
    /// at the scene's pace brought on in the game as far as they are drawn (the interface shows
    /// how far a build has got), short of finished.
    pub fn finish_risen(&mut self, sim: &mut Sim) {
        for id in std::mem::take(&mut self.risen) {
            sim.finish_frame(id);
        }
        for r in self.raises.iter_mut().filter(|r| r.per_tick.is_some() && r.shown.1 > 0.0) {
            let Some(total) = sim.unit(r.unit).map(|u| sim.types.get(u.type_id).build_work) else { continue };
            sim.advance_frame(r.unit, (r.shown.1 as f64 * total as f64) as u64);
            // (Work the scene gave is not the builders': it moves the drawing on no further.)
            r.work = sim.unit(r.unit).map_or(r.work, |u| u.build_work);
        }
    }

    /// Frames drawn part built: how far, at the look before the last and at the last.
    pub fn shown(&self) -> impl Iterator<Item = (UnitId, (f32, f32))> + '_ {
        self.raises.iter().map(|r| (r.unit, r.shown))
    }

    /// Before the frame's ticks: the commands due go in.
    pub fn before_update(&mut self, game: &mut Game) {
        for c in self.commands(&mut game.sim) {
            game.direct(c);
        }
        if let Some(units) = self.select.take() {
            game.selection = units;
        }
        for (units, duty) in std::mem::take(&mut self.duties) {
            game.set_duty_of(&units, duty);
        }
    }

    /// After the frame's ticks: the explosions due go off, the frames are drawn as far built as
    /// the scene has them, and what finishes them goes in.
    pub fn after_update(&mut self, game: &mut Game) {
        let (commands, blasts) = self.after_ticks(&game.sim);
        self.finish_risen(&mut game.sim);
        for c in commands {
            game.direct(c);
        }
        let now = crate::scene::game_seconds(game.sim.tick, game.alpha()) as f32;
        for (round, reach, seconds) in self.volleys.drain(..) {
            game.effects.firework_volley(round, reach, now + seconds);
        }
        for (weapon, p) in blasts {
            let world = &game.sim.world;
            let ground = world.height_at(p.x, p.z);
            let water = ground < world.sea_level;
            let e = SimEvent::Explosion { weapon, pos: FxVec3::new(p.x, ground.max(world.sea_level), p.z), water, when: Fx::ZERO, player: None };
            game.effects.on_event(&e, &game.sim, now);
            game.events_out.push(e);
        }
        game.scene.build_shown.clear();
        game.scene.build_shown.extend(self.shown());
    }

    /// Seconds since the scene started, on the game's clock.
    pub fn seconds(&self, game: &Game) -> f32 {
        (crate::scene::game_seconds(game.sim.tick, game.alpha()) - self.start as f64 / TICKS as f64) as f32
    }

    /// The camera the scene starts with: its `start`, else over the middle of its cast as a
    /// battle starts.
    pub fn place_camera(&mut self, game: &Game, camera: &mut ta_render::Camera) {
        if !self.keys.is_empty() {
            self.camera(game, camera, 0.0);
            return;
        }
        let sim = &game.sim;
        let places: Vec<Vec3> = self.cast.iter().filter_map(|m| sim.unit(m.id)).map(|u| Vec3::from_array(u.pos.to_f32())).collect();
        if !places.is_empty() {
            camera.focus = places.iter().sum::<Vec3>() / places.len() as f32;
        }
        camera.zoom = 1.6;
        camera.yaw = 0.0;
        camera.pitch = ta_render::camera::TA_ELEVATION;
        if let Some(spec) = &self.stage.camera.start {
            let v: Vec<Option<f32>> = spec.split(',').map(|s| s.trim().parse().ok()).collect();
            let at = |i: usize| v.get(i).copied().flatten();
            if let (Some(x), Some(z)) = (at(0), at(1)) {
                camera.focus.x = x;
                camera.focus.z = z;
            }
            if let Some(z) = at(2) {
                camera.zoom = z;
            }
            if let Some(y) = at(3) {
                camera.yaw = y.to_radians();
            }
            if let Some(p) = at(4) {
                camera.pitch = p.to_radians();
            }
        }
    }

    /// The time of day at `clock` seconds of the scene, if it sets one.
    pub fn hours(&self, clock: f32) -> Option<f32> {
        self.stage.hours(clock)
    }

    /// The free camera at `clock` seconds of the scene, if it has one (see the module's notes):
    /// each key's eye, what it looks at (a place, the units it tracks as they are now, or its
    /// angles), roll and lens, eased between the keys; the eye kept over the ground and the sea.
    pub fn free_view(&mut self, game: &Game, clock: f32) -> Option<FreeView> {
        let world = &game.sim.world;
        let sea = world.sea_level.to_f32();
        let alpha = game.alpha();
        self.free_view_with(&game.sim, &|x, z| crate::scene::drawn_ground(world, x, z).max(sea), &|id| game.scene.unit_position(id, alpha), clock)
    }

    /// [`StageRun::free_view`] over `ground` (the height of the ground or the sea at x, z) and
    /// the units' places as drawn (`position`).
    fn free_view_with(&mut self, sim: &Sim, ground: &dyn Fn(f32, f32) -> f32, position: &dyn Fn(UnitId) -> Option<Vec3>, clock: f32) -> Option<FreeView> {
        struct Key {
            t: f32,
            x: f32,
            z: f32,
            /// The eye's height in the world, and over the ground; whether it keeps that over the
            /// ground between this key and a next that does too.
            y: f32,
            over: f32,
            follows: bool,
            /// What it looks at, if anything (else a place `distance` along its angles), and its
            /// angles (degrees).
            look: Option<Vec3>,
            point: Vec3,
            yaw: f32,
            tilt: f32,
            distance: f32,
            roll: f32,
            fov: f32,
        }
        if self.keys.is_empty() {
            return None;
        }
        let direction = |yaw: f32, tilt: f32| {
            let (sy, cy) = yaw.to_radians().sin_cos();
            let (st, ct) = tilt.to_radians().sin_cos();
            Vec3::new(sy * ct, st, -cy * ct)
        };
        let (mut roll, mut fov) = (0.0, 30.0);
        let mut keys: Vec<Key> = Vec::with_capacity(self.keys.len());
        for i in 0..self.keys.len() {
            let k = &self.keys[i];
            let (x, z) = (k.eye[0], k.eye[k.eye.len() - 1]);
            let (y, over, follows) = match k.height {
                Some(h) => (ground(x, z) + h, h, true),
                None => (k.eye[1], k.eye[1] - ground(x, z), false),
            };
            let eye = Vec3::new(x, y, z);
            let lift = k.look_height.unwrap_or(0.0);
            let look = if let Some(l) = &k.look {
                Some(if l.len() == 3 { Vec3::new(l[0], l[1], l[2]) } else { Vec3::new(l[0], ground(l[0], l[1]) + lift, l[1]) })
            } else if let Some(names) = k.track.clone() {
                let places: Vec<Vec3> = self.find(&names, sim).into_iter().filter_map(position).collect();
                if !places.is_empty() {
                    self.tracked[i] = Some(places.iter().sum::<Vec3>() / places.len() as f32 + Vec3::Y * lift);
                }
                self.tracked[i]
            } else {
                None
            };
            let k = &self.keys[i];
            let (yaw, tilt) = match look.map(|p| p - eye).filter(|d| d.length() > 1e-3) {
                Some(d) => (d.x.atan2(-d.z).to_degrees(), d.y.atan2(Vec2::new(d.x, d.z).length()).to_degrees()),
                None => (k.yaw.unwrap_or(0.0), k.tilt.unwrap_or(0.0)),
            };
            // (Where the haze begins: as far as a whole map seen from high over it.)
            let distance = look.map_or(FREE_DISTANCE, |p| p.distance(eye).clamp(16.0, 20_000.0));
            roll = k.roll.unwrap_or(roll);
            fov = k.fov.unwrap_or(fov);
            let point = look.unwrap_or(eye + direction(yaw, tilt) * distance);
            keys.push(Key { t: k.t, x, z, y, over, follows, look, point, yaw, tilt, distance, roll, fov });
        }
        // The short way round from key to key.
        for i in 1..keys.len() {
            let before = keys[i - 1].yaw;
            keys[i].yaw = before + (keys[i].yaw - before + 180.0).rem_euclid(360.0) - 180.0;
        }
        let along = |f: &dyn Fn(&Key) -> f32| eased(&keys.iter().map(|k| (k.t, f(k))).collect::<Vec<_>>(), clock);
        // The keys either side of the moment (the first two or the last two outside them).
        let n = keys.len();
        let i = keys.windows(2).position(|w| clock < w[1].t).unwrap_or(n.saturating_sub(2));
        let (a, b) = (&keys[i], &keys[(i + 1).min(n - 1)]);
        let (x, z) = (along(&|k| k.x), along(&|k| k.z));
        let y = if a.follows && b.follows { ground(x, z) + along(&|k| k.over) } else { along(&|k| k.y) };
        let floor = ground(x, z);
        let eye = Vec3::new(x, y.max(floor + MIN_EYE), z);
        let (mut yaw, mut tilt) = (along(&|k| k.yaw).to_radians(), along(&|k| k.tilt).to_radians());
        // Between two keys looking at something: at what is looked at, as it moves from one to
        // the other.
        if a.look.is_some() && b.look.is_some() {
            let d = Vec3::new(along(&|k| k.point.x), along(&|k| k.point.y), along(&|k| k.point.z)) - eye;
            let flat = Vec2::new(d.x, d.z).length();
            if d.length() > 1e-3 {
                tilt = d.y.atan2(flat);
                if flat > 1e-3 {
                    yaw = d.x.atan2(-d.z);
                }
            }
        }
        Some(FreeView {
            eye,
            yaw,
            pitch: -tilt,
            roll: along(&|k| k.roll).to_radians(),
            fov: along(&|k| k.fov).to_radians(),
            distance: along(&|k| k.distance),
            near: ((eye.y - floor) * 0.5).clamp(1.0, 10.0),
        })
    }

    /// The scene's camera this frame: the free camera's view; or on the units it follows (their
    /// middle), along its path at `clock` seconds.
    pub fn camera(&mut self, game: &Game, camera: &mut ta_render::Camera, clock: f32) {
        if let Some(v) = self.free_view(game, clock) {
            camera.fov_y = v.fov;
            camera.roll = v.roll;
            camera.near = Some(v.near);
            camera.place(v.eye, v.yaw, v.pitch, v.distance);
            return;
        }
        if let Some(name) = &self.stage.camera.follow {
            let alpha = game.alpha();
            let places: Vec<Vec3> = self.find(&Names::One(name.clone()), &game.sim).into_iter().filter_map(|id| game.scene.unit_position(id, alpha)).collect();
            if !places.is_empty() {
                camera.focus = places.iter().sum::<Vec3>() / places.len() as f32;
            }
        }
        let world = &game.sim.world;
        apply_path(camera, &self.path, clock, |x, z| world.height_at(Fx::from_f32(x), Fx::from_f32(z)).max(world.sea_level).to_f32());
    }
}

impl Raise {
    fn new(sim: &Sim, unit: UnitId, from: f32, per_tick: Option<f32>) -> Raise {
        let work = sim.unit(unit).map_or(0, |u| u.build_work);
        Raise { unit, from, per_tick, worked: 0, work, tick: sim.tick, shown: (from, from), finishing: None, alone: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example in the module's notes, as a scene of its own.
    const EXAMPLE: &str = r#"
map = "The Desert Triad"
time = 17.4
rain = true
reveal = true
hud = false
icons = false
style = "reborn"
ai = [1]
unlimited = true
seed = 7

[[player]]
index = 1
faction = "CORE"
color = 1
team = 1

[[clear]]
at = [1500, 1200]
radius = 300

[[unit]]
type = "ARMCOM"
side = 0
at = [1500, 1200]
heading = 90
label = "com"

[[unit]]
type = "ARMGUARD"
rel = [10, 20]
built = 0.0
builder = "com"
build_seconds = 4
label = "guardian"

[[unit]]
type = "CORRAID"
side = 1
at = [2200, 1200]
heading = 270
count = 12
rows = 2
spacing = [40, 60]
label = "tanks"
hold = true

[[unit]]
type = "CORSILO"
side = 1
at = [2600, 600]
stockpile = 1
label = "silo"

[[order]]
t = 1.5
units = ["tanks"]
do = "fight"
by = [-400, 0]

[[order]]
t = 2
units = "com"
do = "build"
type = "ARMLLT"
at = [1450, 1300]
seconds = 3
label = "tower"

[[order]]
t = 3
units = "tower"
do = "attack"
target = "tanks"

[[event]]
t = 3.0
do = "explosion"
at = [1950, 1180]
size = 120

[[event]]
t = 4.0
do = "kill"
units = "guardian"

[[event]]
t = 5
do = "nuke"
units = "silo"
rel = [0, 0]

[camera]
camera = "1500,1200,3,30,12"
path = "0:1500,1200,3,30,12;5:1600,1200,4,40,8,30"
follow = "com"
"#;

    #[test]
    fn reads_a_scene() {
        let s = Stage::parse(EXAMPLE).unwrap();
        assert_eq!(s.units.len(), 4);
        assert_eq!(s.orders[0].action, Action::Fight);
        assert_eq!(s.events[2].what, Happening::Nuke);
        assert!(s.ai.includes(1) && !s.ai.includes(0));
        assert_eq!(s.camera.start.as_deref(), Some("1500,1200,3,30,12"));
        assert_eq!(s.player_count(), 2);
        let path = parse_path(s.camera.path.as_deref().unwrap()).unwrap();
        assert_eq!(path[1], (5.0, [Some(1600.0), Some(1200.0), Some(4.0), Some(40.0), Some(8.0), Some(30.0)]));
        // Defaults.
        let s = Stage::parse("[[unit]]\ntype = \"ARMPW\"\nat = [1, 2]").unwrap();
        let u = &s.units[0];
        assert!(s.reveal && !s.hud && !s.icons && s.unlimited && !s.ai.includes(0));
        assert_eq!((u.side, u.heading, u.count, u.rows, u.built, u.hold), (0, 180.0, 1, 1, 1.0, false));
    }

    #[test]
    fn refuses_what_it_cannot_play() {
        for bad in [
            "colour = 3",
            "[[unit]]\ntype = \"ARMPW\"",
            "[[unit]]\ntype = \"ARMPW\"\nat = [1, 2]\nrel = [1, 2]",
            "[[unit]]\ntype = \"ARMPW\"\nat = [1, 2]\nbuilt = 2",
            "[[unit]]\ntype = \"ARMPW\"\nat = [1, 2]\nbuilder = \"x\"",
            "[[unit]]\ntype = \"ARMPW\"\nat = [1, 2]\nfacing = 3",
            "[[order]]\nt = 1\nunits = \"a\"\ndo = \"move\"",
            "[[order]]\nt = 1\nunits = \"a\"\ndo = \"teleport\"\nat = [1, 2]",
            "[[order]]\nt = -1\nunits = \"a\"\ndo = \"stop\"",
            "[[order]]\nt = 1\nunits = \"a\"\ndo = \"guard\"\nat = [1, 2]",
            "[[order]]\nt = 1\nunits = \"a\"\ndo = \"attack\"\nat = [1, 2]\ntarget = \"b\"",
            "[[order]]\nt = 1\nunits = \"a\"\ndo = \"build\"\nat = [1, 2]",
            "[[order]]\nt = 1\nunits = \"a\"\ndo = \"off\"\nat = [1, 2]",
            "[[order]]\nt = 1\nunits = \"a\"\ndo = \"on\"\ntarget = \"b\"",
            "[[event]]\nt = 1\ndo = \"explosion\"",
            "[[event]]\nt = 1\ndo = \"kill\"",
            "[[event]]\nt = 1\ndo = \"nuke\"\nat = [1, 2]",
            "[[event]]\nt = 1\ndo = \"fireworks\"",
            "[[event]]\nt = 1\ndo = \"fireworks\"\nat = [1, 2]\nseconds = 0",
            "[[event]]\nt = 1\ndo = \"duty\"\nunits = \"a\"",
            "[[event]]\nt = 1\ndo = \"duty\"\nduty = \"economy\"",
            "[[event]]\nt = 1\ndo = \"kill\"\nunits = \"a\"\nduty = \"economy\"",
            "[[event]]\nt = 1\ndo = \"duty\"\nunits = \"a\"\nduty = \"napping\"",
            "[[event]]\nt = 1\ndo = \"explosion\"\nat = [1, 2]\nseconds = 3",
            "[camera]\npath = \"1500,1200,3\"",
            "[[ride]]\nt = -1\nunit = \"a\"",
            "[[ride]]\nt = 4\nunit = \"a\"\nleave = 3",
            "[[ride]]\nt = 1",
        ] {
            assert!(Stage::parse(bad).is_err(), "{bad:?} was taken");
        }
    }

    /// TA's on/off order: switched off and on again, as the player switches them.
    #[test]
    fn switches_units_off_and_on() {
        let s = Stage::parse("[[order]]\nt = 2\nunits = \"makers\"\ndo = \"off\"\n[[order]]\nt = 9\nunits = [\"makers\", \"radar\"]\ndo = \"activate\"").unwrap();
        assert_eq!(s.orders.iter().map(|o| o.action).collect::<Vec<_>>(), [Action::Off, Action::On]);
    }

    /// A duty event puts constructors on a duty (or back under command); the stores a scene starts
    /// with are its own to set.
    #[test]
    fn puts_constructors_on_duty() {
        let s = Stage::parse("metal = 300\nenergy = 80\n[[event]]\nt = 1\ndo = \"duty\"\nunits = \"ck\"\nduty = \"reclaim\"\n[[event]]\nt = 2\ndo = \"duty\"\nunits = \"ck\"\nduty = \"none\"").unwrap();
        assert_eq!((s.metal, s.energy), (Some(300), Some(80)));
        assert_eq!(s.events[0].what, Happening::Duty);
        assert_eq!(s.events[0].duty.and_then(DutyName::duty), Some(ta_ai::Duty::Reclaim));
        assert_eq!(s.events[1].duty.and_then(DutyName::duty), None, "none: back under command");
        for (name, duty) in [("economy", ta_ai::Duty::Economy), ("repair", ta_ai::Duty::Repair), ("reclaim", ta_ai::Duty::Reclaim), ("defence", ta_ai::Duty::Defence)] {
            assert_eq!(duty.name(), name);
        }
    }

    /// A ride: into the first of its units still there at its second, out again at `leave`; one
    /// whose units are all gone is passed over. A volley of fireworks is the game's to fire.
    #[test]
    fn rides_and_volleys() {
        let stage = Stage::parse("[[ride]]\nt = 2\nunit = \"lead\"\nleave = 5\n[[ride]]\nt = 6\nunit = \"gone\"\n[[event]]\nt = 1\ndo = \"fireworks\"\nat = [1, 2]\nsize = 300\nseconds = 4").unwrap();
        assert_eq!(stage.rides.len(), 2);
        assert_eq!(stage.rides[0].leave, Some(5.0));
        assert_eq!(stage.events[0].what, Happening::Fireworks);
        // (Leaving the ride's units out leaves the ride out.)
        let mut pruned = Stage::parse("[[unit]]\ntype = \"ARMPW\"\nat = [1, 2]\nlabel = \"lead\"\n[[unit]]\ntype = \"CORKROG\"\nat = [1, 2]\nlabel = \"big\"\n[[ride]]\nt = 2\nunit = \"big\"\nmissing = \"skip\"").ok();
        assert!(pruned.is_none(), "`missing` is the scene's, not the ride's");
        pruned = Stage::parse("missing = \"skip\"\n[[unit]]\ntype = \"ARMPW\"\nat = [1, 2]\nlabel = \"lead\"\n[[unit]]\ntype = \"CORKROG\"\nat = [1, 2]\nlabel = \"big\"\n[[ride]]\nt = 2\nunit = \"big\"").ok();
        let mut pruned = pruned.unwrap();
        assert_eq!(pruned.prune_missing(|k| !k.eq_ignore_ascii_case("CORKROG")), ["CORKROG"]);
        assert!(pruned.rides.is_empty());
        // The cues, against a battle (none without the install).
        let Some(assets) = test_assets() else { return };
        let stage = Stage::parse("map = \"Highland Rift\"\n[[unit]]\ntype = \"ARMBULL\"\nat = [5800, 8640]\nlabel = \"lead\"\n[[ride]]\nt = 2\nunit = \"lead\"\nleave = 5\n[[ride]]\nt = 6\nunit = \"ARMBULL\"").unwrap();
        let mut sim = battle(&assets, &stage);
        let mut run = StageRun::start(&stage, &mut sim);
        let lead = run.find(&Names::One("lead".into()), &sim)[0];
        assert_eq!(run.ride_cue(&sim, 1.0), None);
        assert_eq!(run.ride_cue(&sim, 2.0), Some(RideCue::Enter(lead)));
        assert_eq!(run.ride_cue(&sim, 3.0), None);
        assert_eq!(run.ride_cue(&sim, 5.0), Some(RideCue::Leave));
        assert_eq!(run.ride_cue(&sim, 6.0), Some(RideCue::Enter(lead)));
        assert_eq!(run.ride_cue(&sim, 30.0), None, "ridden to the end");
    }

    #[test]
    fn headings_and_formations() {
        // Degrees clockwise from north to TA's angle (0 facing south, a quarter turn east).
        assert_eq!(ta_heading(180.0), 0);
        assert_eq!(ta_heading(90.0), 16384);
        assert_eq!(ta_heading(270.0), -16384);
        assert_eq!(ta_heading(0.0), -32768);
        assert_eq!(ta_heading(360.0), -32768);
        // TA's angle points where the degrees say: (sin, cos) of it, x east and z south.
        for deg in [0.0f32, 45.0, 90.0, 200.0, 315.0] {
            let a = ta_heading(deg) as f32 / 65536.0 * std::f32::consts::TAU;
            let (s, c) = deg.to_radians().sin_cos();
            assert!((a.sin() - s).abs() < 1e-3 && (a.cos() + c).abs() < 1e-3, "{deg}");
        }
        // A line of three facing east runs north to south through the place.
        assert_eq!(block((100.0, 100.0), 90.0, 3, 1, Some(Spacing::Even(40.0)), (2, 2)), vec![(100.0, 60.0), (100.0, 100.0), (100.0, 140.0)]);
        // Two rows facing north: the first in front (north), each row centred.
        let b = block((0.0, 0.0), 0.0, 5, 2, Some(Spacing::Grid([10.0, 20.0])), (1, 1));
        let round = |v: Vec<(f32, f32)>| v.into_iter().map(|(x, z)| (x.round(), z.round())).collect::<Vec<_>>();
        assert_eq!(round(b), vec![(-10.0, -10.0), (0.0, -10.0), (10.0, -10.0), (-5.0, 10.0), (5.0, 10.0)]);
        // One unit stands on the spot; by default spaced by their size.
        assert_eq!(block((7.0, 9.0), 33.0, 1, 1, None, (3, 3)), vec![(7.0, 9.0)]);
        assert_eq!(block((0.0, 0.0), 180.0, 2, 1, None, (2, 2)).len(), 2);
    }

    /// A free camera's scene: the time of day running from night to morning, the moon placed, an
    /// aircraft in the air low, and the camera's keys.
    const FREE: &str = r#"
map = "The Desert Triad"
time = [[0, 0.5], [1, 0.5], [5.3, 9]]
moon = [235, 9]

[[unit]]
type = "ARMHLT"
at = [2420, 480]
label = "tower"

[[unit]]
type = "ARMTHUND"
at = [2000, 400]
heading = 90
airborne = true
altitude = 60
label = "bomber"

[[camera.key]]
t = 0
eye = [2300, 480]
height = 4
yaw = 90
tilt = 15
fov = 46

[[camera.key]]
t = 2
eye = [2300, 480]
height = 4
look = [2420, 480]
look_height = 40

[[camera.key]]
t = 3
eye = [2350, 40, 520]
track = "bomber"

[[camera.key]]
t = 4
eye = [2350, 520]
height = -30
yaw = 90
tilt = 90
roll = 10
"#;

    #[test]
    fn reads_the_free_camera_and_the_time() {
        let s = Stage::parse(FREE).unwrap();
        assert_eq!(s.camera.keys.len(), 4);
        assert!(s.units[1].airborne && s.units[1].altitude == Some(60.0));
        // Night, held a second, then morning in 4.3 s, eased.
        assert_eq!(s.hours(0.5), Some(0.5));
        assert!((s.hours(3.15).unwrap() - 4.75).abs() < 0.05, "half way: {:?}", s.hours(3.15));
        assert_eq!(s.hours(9.0), Some(9.0));
        assert_eq!(Stage::parse("time = 17.5").unwrap().hours(3.0), Some(17.5));
        // The moon low in the south-west.
        let m = s.moon_dir().unwrap();
        assert!(m.y > 0.1 && m.y < 0.2 && m.x < 0.0 && m.z > 0.0, "{m}");
        for bad in [
            "[[camera.key]]
t = 0
eye = [1, 2]",
            "[[camera.key]]
t = 0
eye = [1, 2, 3]
height = 4",
            "[[camera.key]]
t = 0
eye = [1, 2]
height = 4
look = [5, 6]
yaw = 3",
            "[[camera.key]]
t = 0
eye = [1, 2]
height = 4
tilt = 95",
            "[[camera.key]]
t = 0
eye = [1, 2]
height = 4
fov = 0",
            "[[camera.key]]
t = 0
eye = [1, 2]
height = 4
look_height = 9",
            "[[camera.key]]
t = 0
eye = [1, 2]
height = 4
[camera]
follow = \"x\"",
            "time = []",
            "moon = [90, 120]",
            "[[unit]]
type = \"ARMTHUND\"
at = [1, 2]
airborne = true
built = 0.5",
            "[[unit]]
type = \"ARMTHUND\"
at = [1, 2]
altitude = 0",
        ] {
            assert!(Stage::parse(bad).is_err(), "{bad:?} was taken");
        }
    }

    #[test]
    fn eased_moves_start_and_stop_gently() {
        let keys = [(0.0, 0.0), (2.0, 10.0)];
        assert_eq!((eased(&keys, -1.0), eased(&keys, 0.0), eased(&keys, 2.0), eased(&keys, 3.0)), (0.0, 0.0, 10.0, 10.0));
        assert!((eased(&keys, 1.0) - 5.0).abs() < 1e-5);
        assert!(eased(&keys, 0.02) < 0.01 && eased(&keys, 1.98) > 9.99, "at rest at both ends");
        // Through every key, never past one, and still between two that agree.
        let keys = [(0.0, 0.0), (1.0, 10.0), (2.0, 10.0), (3.0, 4.0), (5.0, 30.0)];
        for &(t, v) in &keys {
            assert!((eased(&keys, t) - v).abs() < 1e-4);
        }
        let mut last = 0.0;
        for i in 0..=500 {
            let t = i as f32 * 0.01;
            let v = eased(&keys, t);
            assert!((-1e-4..=30.0001).contains(&v), "{t}: {v}");
            if (1.0..=2.0).contains(&t) {
                assert!((v - 10.0).abs() < 1e-4, "held at {t}: {v}");
            }
            if t > 3.0 {
                assert!(v >= last - 1e-4, "rising to the last key at {t}");
            }
            last = v;
        }
        // Through keys in a line at an even speed (only the ends ease).
        let line = [(0.0, 0.0), (1.0, 1.0), (2.0, 2.0), (3.0, 3.0)];
        assert!((eased(&line, 1.5) - 1.5).abs() < 1e-5);
    }

    #[test]
    fn camera_paths() {
        let path = parse_path("4:100,200,2,90,10;0:0,0,1,0,20").unwrap();
        assert_eq!(path[0].0, 0.0);
        let mut c = ta_render::Camera::new(glam::Vec2::new(1920.0, 1080.0));
        apply_path(&mut c, &path, 2.0, |_, _| 0.0);
        assert!((c.focus.x - 50.0).abs() < 1e-3 && (c.zoom - 1.5).abs() < 1e-3);
        apply_path(&mut c, &path, 9.0, |_, _| 0.0);
        assert_eq!((c.focus.x, c.focus.z), (100.0, 200.0));
        // The sixth field lifts the focus over the ground; empty fields keep the camera's.
        let lifted = parse_path("0:,,3,,,25").unwrap();
        c.focus = Vec3::new(5.0, 0.0, 6.0);
        apply_path(&mut c, &lifted, 0.0, |x, z| x + z);
        assert_eq!((c.focus.x, c.focus.y, c.focus.z, c.zoom), (5.0, 36.0, 6.0, 3.0));
    }

    fn test_assets() -> Option<Assets> {
        use std::sync::Arc;
        let install = ta_vfs::install::discover(None).ok()?;
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").ok()?;
        // (TA Reborn's own maps, over the install as the game has them.)
        vfs.mount_builtin();
        let data = Arc::new(ta_data::GameData::load(&vfs));
        let types = Arc::new(ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
        let maps = crate::setup::MapEntry::catalogue(&vfs);
        Some(Assets { vfs: Arc::new(vfs), data, types, selectable_maps: maps.clone(), maps, campaigns: Vec::new(), data_hash: 0, compatibility: Default::default(), mods: Default::default() })
    }

    /// The scene's battle as `Game::new` makes it (without the pictures): the duel's Commanders
    /// placed, the rules applied.
    fn battle(assets: &Assets, stage: &Stage) -> Sim {
        use ta_formats::tdf;
        use ta_formats::tnt::TntMap;
        let setup = stage.setup(None, assets).unwrap();
        let vfs = assets.vfs.as_ref();
        let map = TntMap::parse(&vfs.read(&format!("maps/{}.tnt", setup.map)).unwrap()).unwrap();
        let info = ta_data::MapInfo::from_ota(&tdf::parse_bytes(&vfs.read(&format!("maps/{}.ota", setup.map)).unwrap()));
        let players: Vec<ta_sim::PlayerSetup> = setup
            .players
            .iter()
            .enumerate()
            .map(|(i, s)| ta_sim::PlayerSetup {
                side: s.side.clone(),
                color: s.color,
                start: FxVec3::from_ints(300 + 400 * i as i64, 0, 300),
                metal: Fx::from_int(setup.rules.start_metal as i64),
                energy: Fx::from_int(setup.rules.start_energy as i64),
                ai: true,
            })
            .collect();
        let mut sim = Sim::new(assets.types.clone(), assets.data.for_map(vfs, &setup.map), &map, &info, 0, &players, setup.seed);
        for (i, s) in setup.players.iter().enumerate() {
            sim.set_team(i as u8, s.team);
        }
        sim.commander_ends_game = setup.rules.commander_ends_game;
        sim.los_enabled = setup.rules.fog_of_war;
        sim.unit_limit = setup.rules.unit_limit;
        sim.cheats_allowed = true;
        for (i, p) in players.iter().enumerate() {
            sim.set_starting_resources(i as u8, p.metal, p.energy);
            if let Some(t) = sim.types.id(if i % 2 == 0 { "ARMCOM" } else { "CORCOM" }) {
                sim.spawn_unit(t, i as u8, p.start, 0, true);
            }
        }
        sim
    }

    /// Plays `ticks` of a scene as the game does: the commands due before each tick, and those
    /// that finish its frames after it (given before the next).
    fn play(sim: &mut Sim, run: &mut StageRun, ticks: u64, mut each: impl FnMut(&Sim)) {
        let mut carried = Vec::new();
        for _ in 0..ticks {
            let mut commands = std::mem::take(&mut carried);
            commands.extend(run.commands(sim));
            sim.step(&commands);
            sim.take_events();
            carried = run.after_ticks(sim).0;
            run.finish_risen(sim);
            each(sim);
        }
    }

    /// The free camera in a scene: its eye where its keys put it (never under the ground), looking
    /// along their angles, at their places, at the units they track; an aircraft in the air.
    #[test]
    fn the_free_camera_goes_where_its_keys_say() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let stage = Stage::parse(FREE).unwrap();
        let mut sim = battle(&assets, &stage);
        let mut run = StageRun::start(&stage, &mut sim);
        let ground = |x: f32, z: f32| sim.world.height_at(Fx::from_f32(x), Fx::from_f32(z)).max(sim.world.sea_level).to_f32();
        let bomber = run.find(&Names::One("bomber".into()), &sim)[0];
        // The aircraft is in the air, low.
        let u = sim.unit(bomber).unwrap();
        assert!(!u.landed && (u.pos.y.to_f32() - ground(2000.0, 400.0) - 60.0).abs() < 0.01);
        let sim_ref = &sim;
        let position = |id: UnitId| sim_ref.unit(id).map(|u| Vec3::from_array(u.pos.to_f32()));
        let mut at = |t: f32| run.free_view_with(sim_ref, &ground, &position, t).unwrap();
        // At the first key: 4 over the ground, looking east 15° up, at the lens asked for.
        let v = at(0.0);
        assert!((v.eye - Vec3::new(2300.0, ground(2300.0, 480.0) + 4.0, 480.0)).length() < 0.01);
        assert!((v.yaw.to_degrees() - 90.0).abs() < 0.01 && (v.pitch.to_degrees() + 15.0).abs() < 0.01 && (v.fov.to_degrees() - 46.0).abs() < 0.01);
        // At the second: looking at the tower's top (the lens kept).
        let v = at(2.0);
        let top = Vec3::new(2420.0, ground(2420.0, 480.0) + 40.0, 480.0);
        let forward = Vec3::new(v.yaw.sin() * v.pitch.cos(), -v.pitch.sin(), -v.yaw.cos() * v.pitch.cos());
        assert!(forward.dot((top - v.eye).normalize()) > 0.9999, "looking at the tower");
        assert!((v.fov.to_degrees() - 46.0).abs() < 0.01 && (v.distance - top.distance(v.eye)).abs() < 0.01);
        // At the third: at the bomber as it is now.
        let v = at(3.0);
        let b = position(bomber).unwrap();
        let forward = Vec3::new(v.yaw.sin() * v.pitch.cos(), -v.pitch.sin(), -v.yaw.cos() * v.pitch.cos());
        assert!(forward.dot((b - v.eye).normalize()) > 0.9999, "tracking the bomber");
        assert!((v.eye.y - 40.0).abs() < 0.01, "a world height");
        // At the last, asked under the ground: kept over it, looking straight up, rolled.
        let v = at(9.0);
        assert!(v.eye.y >= ground(2350.0, 520.0) + MIN_EYE - 0.01 && (v.pitch.to_degrees() + 90.0).abs() < 0.01 && (v.roll.to_degrees() - 10.0).abs() < 0.01);
        // In between, never under the ground, and moving smoothly.
        let mut last = at(0.0).eye;
        for i in 1..=400 {
            let v = at(i as f32 * 0.0125);
            assert!(v.eye.y >= ground(v.eye.x, v.eye.z) + MIN_EYE - 0.01);
            assert!(v.eye.distance(last) < 40.0, "a jump at {} s", i as f32 * 0.0125);
            last = v.eye;
        }
    }

    /// The benchmark's battles on TA Reborn's maps as they are now: every unit staged stands
    /// where it can, and can go from there (moved near where a map's change put a cliff or the
    /// sea under it, or a ridge's top it could not leave), and every one asked for is there.
    #[test]
    fn the_benchmarks_battles_put_every_unit_where_it_can_stand() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        for scene in &crate::benchmark::SCENES {
            let (name, stage) = (scene.id, crate::benchmark::scene_stage(scene));
            let mut sim = battle(&assets, &stage);
            let asked: usize = stage.units.iter().filter(|u| sim.types.id(&u.kind).is_some()).map(|u| u.count as usize).sum();
            let run = StageRun::start(&stage, &mut sim);
            let mut stuck = Vec::new();
            let mut sizes = RegionSizes::new();
            for m in &run.cast {
                let Some(u) = sim.unit(m.id) else { continue };
                if !u.landed && sim.types.get(u.type_id).locomotion == Locomotion::Air {
                    continue;
                }
                // (A building put down occupies its own ground.)
                if let Some(why) = cannot_stand_or_leave(&sim, u.type_id, u.pos, &mut sizes).filter(|why| why != "Occupied") {
                    stuck.push(format!("{} at ({:.0}, {:.0}): {why}", m.kind, u.pos.x.to_f32(), u.pos.z.to_f32()));
                }
            }
            assert!(stuck.is_empty(), "{name}: {} units where they cannot stand: {stuck:?}", stuck.len());
            assert!(run.cast.len() + run.due.len() >= asked, "{name}: {} of {asked} units put down", run.cast.len() + run.due.len());
        }
    }

    /// The credits' scenes (`apps/ta_game/credits/`): their units are all in the game, and stand
    /// where they can; their `[[text]]`s and stations parse.
    #[test]
    fn the_credits_scenes_put_every_unit_where_it_can_stand() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let mut on_cards = Vec::new();
        for scene in crate::credits_show::scenes() {
            let stage = scene.stage.clone();
            let missing: Vec<&str> = stage.units.iter().map(|u| u.kind.as_str()).filter(|k| assets.types.id(k).is_none()).collect();
            assert!(missing.is_empty(), "{}: not in the game: {missing:?}", scene.id);
            let mut sim = battle(&assets, &stage);
            let asked: usize = stage.units.iter().map(|u| u.count as usize).sum();
            let mut run = StageRun::start(&stage, &mut sim);
            let mut stuck = Vec::new();
            let mut sizes = RegionSizes::new();
            for m in &run.cast {
                let Some(u) = sim.unit(m.id) else { continue };
                if !u.landed && sim.types.get(u.type_id).locomotion == Locomotion::Air {
                    continue;
                }
                if let Some(why) = cannot_stand_or_leave(&sim, u.type_id, u.pos, &mut sizes).filter(|why| why != "Occupied") {
                    stuck.push(format!("{} at ({:.0}, {:.0}): {why}", m.kind, u.pos.x.to_f32(), u.pos.z.to_f32()));
                }
            }
            assert!(stuck.is_empty(), "{}: {} units where they cannot stand: {stuck:?}", scene.id, stuck.len());
            assert!(run.cast.len() + run.due.len() >= asked, "{}: {} of {asked} units put down", scene.id, run.cast.len() + run.due.len());
            // And the battle stands off the names: a while into the act (its works raised, every unit
            // where it ended up: moved off a cliff, pushed aside by its neighbours), nothing on the
            // ground of a card (`credits_show::card_ground`). Aircraft fly over.
            play(&mut sim, &mut run, (12.0 * TICKS) as u64, |_| {});
            for u in sim.units() {
                let t = sim.types.get(u.type_id);
                if t.locomotion == Locomotion::Air && !u.landed {
                    continue;
                }
                let (x, z) = (u.pos.x.to_f32(), u.pos.z.to_f32());
                let reach = t.footprint.0.max(t.footprint.1) as f32 * 8.0;
                for (i, st) in scene.stations.iter().enumerate() {
                    let (Some((half_road, half_ground)), Some([sx, sz])) = (crate::credits_show::card_ground(&scene, i), st.at) else { continue };
                    // (Along the road ahead, and across it to the right: `credits_show::basis`.)
                    let (s, c) = st.heading.to_radians().sin_cos();
                    let (ahead, across) = ((x - sx) * s - (z - sz) * c, (x - sx) * c + (z - sz) * s);
                    if across.abs() - reach < half_road && ahead.abs() - reach < half_ground {
                        on_cards.push(format!("{}: {} at ({x:.0}, {z:.0}): station {i}, {across:+.0} across, {ahead:+.0} along", scene.id, t.name));
                    }
                }
            }
            // Every station stands on the map, on land.
            let (w, h) = sim.world.size_px();
            for (i, st) in scene.stations.iter().enumerate() {
                let [x, z] = st.at.expect("a place");
                assert!(x > 0.0 && z > 0.0 && x < w.to_f32() && z < h.to_f32(), "{}: station {i} off the map", scene.id);
                let ground = sim.world.height_at(Fx::from_f32(x), Fx::from_f32(z));
                assert!(ground >= sim.world.sea_level, "{}: station {i} at ({x:.0}, {z:.0}) is under the sea", scene.id);
            }
        }
        assert!(on_cards.is_empty(), "{} units on the cards' ground: {on_cards:#?}", on_cards.len());
    }

    /// `[[text]]`, `[act]` and `[[station]]` read.
    #[test]
    fn text_acts_and_stations_read() {
        let stage = Stage::parse(
            r#"
map = "Highland Rift"
[act]
title = "t"
themes = ["art"]
hours = [5.0, 7.0]
[[station]]
at = [100, 200]
heading = 90
style = "nano"
[[text]]
t = 1.5
text = "A"
kicker = "k"
at = [10, 20]
style = "burn"
blast = [0, 0]
"#,
        )
        .unwrap();
        assert_eq!(stage.stations.len(), 1);
        assert_eq!(stage.act.as_ref().map(|a| a.style), Some(TextStyle::Burn), "burning by default");
        let spec = stage.texts[0].spec(10.0, (0.0, 0.0)).unwrap();
        assert_eq!(spec.lines.len(), 2, "its kicker and its line");
        assert!((spec.born - 11.5).abs() < 1e-4 && (spec.life - 8.0).abs() < 1e-4);
        assert!(matches!(spec.look, crate::world_text::Look::Burn { .. }));
    }

    /// `[[text]]` lays the emblem in place of words, and nothing without either.
    #[test]
    fn a_text_can_be_the_emblem() {
        let stage = Stage::parse(
            r#"
map = "Highland Rift"
[[text]]
picture = "emblem"
at = [500, 600]
size = 400
style = "nano"
[[text]]
at = [10, 20]
"#,
        )
        .unwrap();
        assert_eq!(stage.texts[0].picture, Some(TextPicture::Emblem));
        let spec = stage.texts[0].spec(0.0, (0.0, 0.0)).expect("the emblem, though it has no words");
        assert!((spec.size - 400.0).abs() < 1e-4 && matches!(spec.look, crate::world_text::Look::Nano));
        assert!(stage.texts[1].spec(0.0, (0.0, 0.0)).is_none(), "neither words nor a picture");
    }

    /// The benchmark's aircraft stay in the air to their scenes' ends (one with nothing left to
    /// do lands, in the middle of a shot: the bombers once their targets were gone, the
    /// construction aircraft once its radar was up).
    #[test]
    fn the_benchmarks_aircraft_stay_in_the_air() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        for scene in crate::benchmark::SCENES.iter().filter(|s| !matches!(s.id, "swarm" | "cinder")) {
            let stage = crate::benchmark::scene_stage(scene);
            let mut sim = battle(&assets, &stage);
            let mut run = StageRun::start(&stage, &mut sim);
            let flyers: Vec<(UnitId, String)> = run.cast.iter().filter(|m| sim.unit(m.id).is_some_and(|u| sim.types.get(u.type_id).locomotion == Locomotion::Air)).map(|m| (m.id, m.kind.clone())).collect();
            let mut landed = std::collections::BTreeSet::new();
            play(&mut sim, &mut run, (scene.seconds * TICKS) as u64, |sim| {
                for (id, kind) in &flyers {
                    if sim.unit(*id).is_some_and(|u| !u.dead && u.landed) {
                        landed.insert(kind.clone());
                    }
                }
            });
            assert!(landed.is_empty(), "{}: landed during the scene: {landed:?}", scene.id);
        }
    }

    /// (Ignored: a report.) The benchmark's ground units sent somewhere that stalled on the way:
    /// alive at the scene's end, nothing to shoot at over its last ten seconds and less than
    /// 24 px moved in them; by scene, with where each stands and where it started
    /// (`TA_SCENES=highland` for one).
    #[test]
    #[ignore]
    fn benchmark_units_that_stalled() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let only = std::env::var("TA_SCENES").ok();
        for scene in &crate::benchmark::SCENES {
            if only.as_deref().is_some_and(|o| !o.split(',').any(|n| n == scene.id)) {
                continue;
            }
            let stage = crate::benchmark::scene_stage(scene);
            let sent: Vec<&str> = stage.orders.iter().filter(|o| matches!(o.action, Action::Move | Action::Fight | Action::Patrol)).flat_map(|o| o.units.list()).collect();
            let mut sim = battle(&assets, &stage);
            let mut run = StageRun::start(&stage, &mut sim);
            let walkers: Vec<(UnitId, String, FxVec3)> = run
                .cast
                .iter()
                .filter(|m| m.label.as_deref().is_some_and(|l| sent.contains(&l)))
                .filter_map(|m| sim.unit(m.id).filter(|u| !matches!(sim.types.get(u.type_id).locomotion, Locomotion::Static | Locomotion::Air)).map(|u| (m.id, m.kind.clone(), u.pos)))
                .collect();
            // Each one's place and whether it had a target, once a second.
            let mut seen: std::collections::BTreeMap<UnitId, Vec<(FxVec3, bool)>> = std::collections::BTreeMap::new();
            let mut tick = 0u64;
            play(&mut sim, &mut run, (scene.seconds * TICKS) as u64, |sim| {
                tick += 1;
                if tick % TICKS as u64 == 0 {
                    for (id, _, _) in &walkers {
                        if let Some(u) = sim.unit(*id).filter(|u| !u.dead) {
                            seen.entry(*id).or_default().push((u.pos, u.weapons.iter().any(|w| w.target.is_some())));
                        }
                    }
                }
            });
            let window = 10;
            let stalled: Vec<String> = walkers
                .iter()
                .filter_map(|(id, kind, from)| {
                    let track = seen.get(id)?;
                    sim.unit(*id).filter(|u| !u.dead)?;
                    let last = &track[track.len().saturating_sub(window)..];
                    if last.len() < window || last.iter().any(|(_, fighting)| *fighting) {
                        return None;
                    }
                    let (a, b) = (last[0].0, last[last.len() - 1].0);
                    (a.distance_xz(b).to_f32() < 24.0).then(|| format!("{kind} at ({:.0}, {:.0}) from ({:.0}, {:.0})", b.x.to_f32(), b.z.to_f32(), from.x.to_f32(), from.z.to_f32()))
                })
                .collect();
            eprintln!("{}: {} of {} sent stalled: {stalled:?}", scene.id, stalled.len(), walkers.len());
        }
    }

    /// (Ignored: a report.) The benchmark's ground units sent somewhere (moving, fighting, on
    /// patrol) that never got going: alive at the scene's end within 48 px of where they were
    /// put down, and never with anything to shoot at, by scene (`TA_SCENES=highland,space` for
    /// some).
    #[test]
    #[ignore]
    fn benchmark_units_that_never_moved() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let only = std::env::var("TA_SCENES").ok();
        for scene in &crate::benchmark::SCENES {
            if only.as_deref().is_some_and(|o| !o.split(',').any(|n| n == scene.id)) {
                continue;
            }
            let stage = crate::benchmark::scene_stage(scene);
            let sent: Vec<&str> = stage.orders.iter().filter(|o| matches!(o.action, Action::Move | Action::Fight | Action::Patrol)).flat_map(|o| o.units.list()).collect();
            let mut sim = battle(&assets, &stage);
            let mut run = StageRun::start(&stage, &mut sim);
            let walkers: Vec<(UnitId, String, FxVec3)> = run
                .cast
                .iter()
                .filter(|m| m.label.as_deref().is_some_and(|l| sent.contains(&l)))
                .filter_map(|m| sim.unit(m.id).filter(|u| !matches!(sim.types.get(u.type_id).locomotion, Locomotion::Static | Locomotion::Air)).map(|u| (m.id, m.kind.clone(), u.pos)))
                .collect();
            // (Those that had something to shoot at stood to fight, as sent to.)
            let mut fought = std::collections::BTreeSet::new();
            play(&mut sim, &mut run, (scene.seconds * TICKS) as u64, |sim| {
                for (id, _, _) in &walkers {
                    if sim.unit(*id).is_some_and(|u| u.weapons.iter().any(|w| w.target.is_some())) {
                        fought.insert(*id);
                    }
                }
            });
            let still: Vec<String> = walkers
                .iter()
                .filter(|(id, _, _)| !fought.contains(id))
                .filter_map(|(id, kind, from)| {
                    let u = sim.unit(*id).filter(|u| !u.dead)?;
                    (u.pos.distance_xz(*from).to_f32() < 48.0).then(|| format!("{kind} at ({:.0}, {:.0})", from.x.to_f32(), from.z.to_f32()))
                })
                .collect();
            eprintln!("{}: {} of {} sent never got going (nor fought): {still:?}", scene.id, still.len(), walkers.len());
        }
    }

    /// The menu's film's scenes (`backdrop.rs`) on TA Reborn's worlds as they are now: every unit
    /// of the game's own (none left out but in a mod), every one standing where it can, and every
    /// one asked for there.
    #[test]
    fn the_menu_films_scenes_put_every_unit_where_it_can_stand() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        for shot in crate::backdrop::SHOTS {
            let name = format!("{}/{}", shot.world, shot.id);
            let stage = Stage::parse(shot.scene.1).unwrap();
            let missing: Vec<&str> = stage.units.iter().map(|u| u.kind.as_str()).filter(|k| assets.types.id(k).is_none()).collect();
            assert!(missing.is_empty(), "{name}: not in the game: {missing:?}");
            let mut sim = battle(&assets, &stage);
            let asked: usize = stage.units.iter().map(|u| u.count as usize).sum();
            let run = StageRun::start(&stage, &mut sim);
            let mut stuck = Vec::new();
            let mut sizes = RegionSizes::new();
            for m in &run.cast {
                let Some(u) = sim.unit(m.id) else { continue };
                if !u.landed && sim.types.get(u.type_id).locomotion == Locomotion::Air {
                    continue;
                }
                if let Some(why) = cannot_stand_or_leave(&sim, u.type_id, u.pos, &mut sizes).filter(|why| why != "Occupied") {
                    stuck.push(format!("{} at ({:.0}, {:.0}): {why}", m.kind, u.pos.x.to_f32(), u.pos.z.to_f32()));
                }
            }
            assert!(stuck.is_empty(), "{name}: {} units where they cannot stand: {stuck:?}", stuck.len());
            assert!(run.cast.len() + run.due.len() >= asked, "{name}: {} of {asked} units put down", run.cast.len() + run.due.len());
        }
    }

    /// A scene played in the simulation: everyone where and as they were put, nobody straying,
    /// the frame built on the scene's clock, orders and deaths on cue.
    #[test]
    fn plays_a_scene_as_staged() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let scene = r#"
map = "The Desert Triad"
[[clear]]
at = [2400, 480]
radius = 600
[[unit]]
type = "ARMCOM"
at = [2050, 480]
heading = 90
label = "com"
[[unit]]
type = "ARMGUARD"
at = [2126, 480]
built = 0.0
builder = "com"
build_seconds = 4
hold = true
label = "guardian"
[[unit]]
type = "CORAK"
side = 1
at = [2500, 480]
heading = 90
count = 6
rows = 2
label = "squad"
hold = true
[[unit]]
type = "ARMPW"
at = [2050, 590]
heading = 45
label = "lookout"
[[order]]
t = 6
units = "squad"
do = "move"
by = [150, 0]
[[event]]
t = 8
do = "kill"
units = "lookout"
"#;
        let stage = Stage::parse(scene).unwrap();
        let mut sim = battle(&assets, &stage);
        let mut run = StageRun::start(&stage, &mut sim);
        let cast: Vec<(String, UnitId)> = run.cast.iter().map(|m| (m.label.clone().unwrap_or_default(), m.id)).collect();
        assert_eq!(cast.len(), 9, "the Commander, the frame, six A.K.s and a Peewee");
        assert_eq!(sim.units().filter(|u| !u.dead).count(), 9, "nothing else on the map");
        let id = |label: &str| cast.iter().find(|(l, _)| l == label).unwrap().1;
        let xz = |sim: &Sim, u: UnitId| sim.unit(u).map(|u| (u.pos.x.to_f32(), u.pos.z.to_f32())).unwrap();
        // Placed exactly, facing as asked.
        assert_eq!(xz(&sim, id("com")), (2050.0, 480.0));
        assert_eq!(sim.unit(id("com")).unwrap().heading, 16384);
        assert_eq!(sim.unit(id("lookout")).unwrap().heading, ta_heading(45.0));
        let squad: Vec<UnitId> = cast.iter().filter(|(l, _)| l == "squad").map(|&(_, u)| u).collect();
        let spots = block((2500.0, 480.0), 90.0, 6, 2, None, sim.types.get(sim.unit(squad[0]).unwrap().type_id).footprint);
        for (&k, &(x, z)) in squad.iter().zip(&spots) {
            let (kx, kz) = xz(&sim, k);
            assert!((kx - x).abs() < 0.01 && (kz - z).abs() < 0.01, "an A.K. at ({kx}, {kz}), not ({x}, {z})");
            assert_eq!(sim.unit(k).unwrap().heading, 16384);
        }
        let frame = id("guardian");
        assert!(!sim.unit(frame).unwrap().is_complete());
        // Five seconds: the frame drawn on and finished on the scene's four seconds of work (the
        // Commander turns its arm to it first); nobody moves; those held hold their fire.
        let mut finished = None;
        play(&mut sim, &mut run, 2 * 30, |_| {});
        // Half way, the game's progress is the drawing's (the interface reads it), and the frame
        // as sturdy as building that far makes it.
        let drawn = run.shown().find(|&(u, _)| u == frame).map(|(_, (_, now))| now).unwrap();
        let u = sim.unit(frame).unwrap();
        let (progress, health) = (u.build_progress.to_f32(), (u.health / sim.unit_type(u).max_health).to_f32());
        assert!(drawn > 0.2 && (progress - drawn).abs() < 0.05, "drawn {drawn}, built {progress} in the game");
        assert!((health - progress).abs() < 0.05, "built {progress}, but at {health} of its health");
        play(&mut sim, &mut run, 3 * 30, |sim| {
            if finished.is_none() && sim.unit(frame).is_some_and(|u| u.is_complete()) {
                finished = Some(sim.tick);
            }
        });
        let finished = finished.expect("the Guardian was finished");
        assert!(sim.players.iter().all(|p| p.stats.lost == 0), "the battle's own Commanders, taken off for the scene, counted as lost");
        assert!((120..=150).contains(&finished), "the Guardian finished at tick {finished}");
        assert!(cast.iter().all(|&(_, u)| sim.unit(u).is_some_and(|u| !u.dead)), "nobody died");
        assert_eq!(xz(&sim, id("lookout")), (2050.0, 590.0), "nobody wandered");
        assert_eq!(xz(&sim, id("com")), (2050.0, 480.0));
        for &k in &squad {
            let u = sim.unit(k).unwrap();
            assert_eq!((u.fire_state, u.move_state), (0, 0), "held");
        }
        assert!(sim.players.iter().all(|p| !p.cheats.instant_build), "instant building is off again");
        // At 6 s the A.K.s walk on 150 px east, each its own way, and fire at will again.
        play(&mut sim, &mut run, 7 * 30, |_| {});
        for (&k, &(x, z)) in squad.iter().zip(&spots) {
            let (kx, kz) = xz(&sim, k);
            // (Each its own way, to where it was sent: units stop a little short of it.)
            assert!((kx - (x + 150.0)).abs() < 24.0 && (kz - z).abs() < 12.0, "an A.K. at ({kx}, {kz}), sent to ({}, {z})", x + 150.0);
            assert_eq!(sim.unit(k).unwrap().fire_state, 2);
        }
        // At 8 s the Peewee died.
        assert!(sim.unit(id("lookout")).is_none_or(|u| u.dead));
        assert_eq!(xz(&sim, id("com")), (2050.0, 480.0));
    }

    /// A frame nobody builds, rising on its own on the scene's clock from `rise_at`: not in the
    /// game at all before, drawn (and as sturdy) as far as its time says while it rises, then
    /// finished in the game (whole, and taking its orders).
    #[test]
    fn a_frame_rises_on_its_own() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let scene = r#"
map = "The Desert Triad"
[[clear]]
at = [2400, 480]
radius = 400
[[unit]]
type = "ARMCOM"
at = [2400, 480]
heading = 90
label = "com"
built = 0.0
build_seconds = 2
rise_at = 1
[[order]]
t = 4
units = "com"
do = "move"
by = [120, 0]
"#;
        let stage = Stage::parse(scene).unwrap();
        let mut sim = battle(&assets, &stage);
        let mut run = StageRun::start(&stage, &mut sim);
        let com = |run: &StageRun| run.cast.iter().find(|m| m.label.as_deref() == Some("com")).map(|m| m.id);
        play(&mut sim, &mut run, 27, |_| {});
        assert_eq!(com(&run), None, "not in the scene before it rises");
        assert_eq!(sim.units().filter(|u| !u.dead).count(), 0, "nothing in the game to see, bump into or shoot at");
        play(&mut sim, &mut run, 30, |_| {});
        let com = com(&run).expect("put down when it rises");
        let drawn = |run: &StageRun| run.shown().find(|&(u, _)| u == com).map(|(_, (_, now))| now);
        let half = drawn(&run).unwrap();
        assert!((0.4..=0.6).contains(&half), "half way up at 2 s: {half}");
        let u = sim.unit(com).unwrap();
        let (progress, health) = (u.build_progress.to_f32(), (u.health / sim.unit_type(u).max_health).to_f32());
        assert!(!u.is_complete() && (health - progress).abs() < 0.05, "built {progress}, but at {health} of its health");
        play(&mut sim, &mut run, 35, |_| {});
        let u = sim.unit(com).unwrap();
        assert!(u.is_complete(), "finished in the game");
        assert_eq!(u.health, sim.unit_type(u).max_health);
        assert_eq!(drawn(&run), None, "no longer drawn as a frame");
        play(&mut sim, &mut run, 4 * 30, |_| {});
        let x = sim.unit(com).unwrap().pos.x.to_f32();
        assert!(x > 2490.0, "it took its order: at x {x}");
        assert!(Stage::parse("[[unit]]\ntype = \"ARMCOM\"\nat = [1, 2]\nrise_at = 1").is_err(), "rise_at without build_seconds");
        assert!(Stage::parse("[[unit]]\ntype = \"ARMCOM\"\nat = [1, 2]\nbuilt = 0\nbuild_seconds = 2\nbuilder = \"x\"\nrise_at = 1\n[[unit]]\ntype = \"ARMCOM\"\nat = [1, 2]\nlabel = \"x\"").is_err(), "rise_at with a builder");
    }

    /// Features put down where the scene says (a wreck, trees), and a `fire` event setting the
    /// trees near its place alight on its time (and none further off).
    #[test]
    fn features_are_put_down_and_set_alight() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let scene = r#"
map = "The Desert Triad"
[[clear]]
at = [2400, 480]
radius = 600
[[feature]]
type = "CORLAB_DEAD"
at = [2300, 480]
heading = 90
[[feature]]
type = "Tree1"
at = [2500, 420]
[[feature]]
type = "Tree1"
at = [2540, 460]
[[feature]]
type = "Tree1"
at = [2900, 480]
[[event]]
t = 1
do = "fire"
at = [2520, 440]
size = 100
"#;
        let stage = Stage::parse(scene).unwrap();
        let mut sim = battle(&assets, &stage);
        let mut run = StageRun::start(&stage, &mut sim);
        let named = |sim: &Sim, name: &str| -> Vec<usize> {
            let id = sim.data.feature_id(name).unwrap();
            sim.features.iter().enumerate().filter(|(_, f)| f.alive && f.def == id).map(|(i, _)| i).collect()
        };
        let wrecks = named(&sim, "CORLAB_DEAD");
        assert_eq!(wrecks.len(), 1, "the wreck put down");
        let at = sim.features[wrecks[0]].pos;
        assert!((at.x.to_f32() - 2300.0).abs() < 48.0 && (at.z.to_f32() - 480.0).abs() < 48.0, "the wreck at {at:?}");
        let trees = named(&sim, "Tree1");
        assert_eq!(trees.len(), 3);
        play(&mut sim, &mut run, 20, |_| {});
        assert!(sim.burning_features().is_empty(), "nothing burning before its time");
        play(&mut sim, &mut run, 20, |_| {});
        let burning: Vec<usize> = trees.iter().copied().filter(|i| sim.burning_features().contains(i)).collect();
        assert_eq!(burning.len(), 2, "the two trees near the place alight, not the one far off: {burning:?}");
        assert!(Stage::parse("[[event]]\nt = 1\ndo = \"fire\"").is_err(), "fire needs a place");
        assert!(Stage::parse("[[feature]]\ntype = \"Tree1\"").is_err(), "a feature needs a place");
    }

    /// A `select` event hands the game the units to select, on its time.
    #[test]
    fn a_scene_selects_units() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let scene = "map = \"The Desert Triad\"\n[[unit]]\ntype = \"ARMCOM\"\nat = [2400, 480]\nlabel = \"com\"\n[[unit]]\ntype = \"ARMPW\"\nat = [2500, 480]\n[[event]]\nt = 1\ndo = \"select\"\nunits = \"com\"\n";
        let stage = Stage::parse(scene).unwrap();
        let mut sim = battle(&assets, &stage);
        let mut run = StageRun::start(&stage, &mut sim);
        let com = run.cast.iter().find(|m| m.label.as_deref() == Some("com")).unwrap().id;
        play(&mut sim, &mut run, 20, |_| {});
        assert_eq!(run.select, None, "nothing selected before its time");
        play(&mut sim, &mut run, 20, |_| {});
        assert_eq!(run.select, Some(vec![com]));
        assert!(Stage::parse("[[event]]\nt = 1\ndo = \"select\"").is_err(), "select needs units");
    }

    /// A player made invulnerable: its units take no harm under fire.
    #[test]
    fn a_player_can_be_invulnerable() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let scene = |god: bool| {
            format!(
                "map = \"The Desert Triad\"\n[[player]]\nindex = 0\ninvulnerable = {god}\n[[clear]]\nat = [2400, 480]\nradius = 400\n[[unit]]\ntype = \"ARMPW\"\nat = [2400, 480]\nlabel = \"pw\"\nhold = true\n[[unit]]\ntype = \"CORHLT\"\nside = 1\nat = [2520, 480]\n"
            )
        };
        for god in [true, false] {
            let stage = Stage::parse(&scene(god)).unwrap();
            let mut sim = battle(&assets, &stage);
            let mut run = StageRun::start(&stage, &mut sim);
            let pw = run.cast.iter().find(|m| m.label.as_deref() == Some("pw")).unwrap().id;
            play(&mut sim, &mut run, 15 * 30, |_| {});
            let alive = sim.unit(pw).is_some_and(|u| !u.dead && u.health == sim.unit_type(u).max_health);
            assert_eq!(alive, god, "invulnerable {god}: the Peewee unharmed {alive}");
        }
    }

    /// A factory told to produce makes its units at its own pace and sends them where it was
    /// told to (a `move` to a factory).
    #[test]
    fn a_factory_produces_on_cue() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let scene = r#"
map = "The Desert Triad"
unlimited = true
[[clear]]
at = [2400, 480]
radius = 500
[[unit]]
type = "ARMLAB"
at = [2400, 480]
label = "lab"
[[order]]
t = 0.5
units = "lab"
do = "move"
at = [2400, 760]
[[order]]
t = 1
units = "lab"
do = "produce"
type = "ARMPW"
count = 2
seconds = 2
label = "squad"
[[order]]
t = 18
units = "squad"
do = "move"
by = [200, 0]
formation = false
"#;
        let stage = Stage::parse(scene).unwrap();
        let mut sim = battle(&assets, &stage);
        let mut run = StageRun::start(&stage, &mut sim);
        let pw = sim.types.id("ARMPW").unwrap();
        let peewees = |sim: &Sim| sim.units().filter(|u| u.type_id == pw && !u.dead && u.is_complete()).map(|u| (u.pos.x.to_f32(), u.pos.z.to_f32())).collect::<Vec<_>>();
        play(&mut sim, &mut run, 30, |_| {});
        assert!(peewees(&sim).is_empty(), "none before the order");
        // Two seconds each, at the scene's pace (the factory's own would take far longer; each
        // walks out of its yard before the next starts).
        play(&mut sim, &mut run, 10 * 30, |_| {});
        assert_eq!(peewees(&sim).len(), 2, "two made by 11 s");
        play(&mut sim, &mut run, 6 * 30, |_| {});
        let made = peewees(&sim);
        for &(x, z) in &made {
            assert!((x - 2400.0).abs() < 120.0 && (z - 760.0).abs() < 120.0, "a Peewee at ({x}, {z}), not at the rally point");
        }
        // Labelled: told on by their label.
        play(&mut sim, &mut run, 8 * 30, |_| {});
        for ((x0, _), (x1, _)) in made.iter().zip(peewees(&sim)) {
            assert!(x1 - x0 > 150.0, "a Peewee of the squad stayed: from {x0} to {x1}");
        }
        assert!(Stage::parse("[[unit]]\ntype = \"ARMLAB\"\nat = [1, 2]\nlabel = \"lab\"\n[[order]]\nt = 1\nunits = \"lab\"\ndo = \"produce\"").is_err(), "produce without a type");
        assert!(Stage::parse("[[unit]]\ntype = \"ARMPW\"\nat = [1, 2]\nlabel = \"pw\"\n[[order]]\nt = 1\nunits = \"pw\"\ndo = \"move\"\nby = [1, 1]\ncount = 2").is_err(), "count without produce");
    }

    /// The orders a group is given: moved by an offset, one formation keeping its shape (as
    /// staged lines march), or with `formation = false` each unit on its own way (choreography
    /// timed unit by unit); sent to a spot with `formation = false`, one order to it for them all.
    #[test]
    fn group_orders_go_as_staged() {
        let Some(assets) = test_assets() else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let scene = r#"
map = "The Desert Triad"
[[clear]]
at = [2500, 480]
radius = 500
[[unit]]
type = "CORAK"
side = 1
at = [2400, 360]
heading = 90
count = 3
label = "kept"
[[unit]]
type = "CORAK"
side = 1
at = [2400, 480]
heading = 90
count = 3
label = "loose"
[[unit]]
type = "CORAK"
side = 1
at = [2400, 600]
heading = 90
count = 3
label = "spot"
[[order]]
t = 1
units = "kept"
do = "move"
by = [150, 0]
[[order]]
t = 1
units = "loose"
do = "move"
by = [150, 0]
formation = false
[[order]]
t = 1
units = "spot"
do = "move"
at = [2700, 600]
formation = false
"#;
        let stage = Stage::parse(scene).unwrap();
        let mut sim = battle(&assets, &stage);
        let mut run = StageRun::start(&stage, &mut sim);
        let squad = |label: &str| -> Vec<UnitId> { run.cast.iter().filter(|m| m.label.as_deref() == Some(label)).map(|m| m.id).collect() };
        let (kept, loose, spot) = (squad("kept"), squad("loose"), squad("spot"));
        let at = |sim: &Sim, u: UnitId| sim.unit(u).map(|u| (u.pos.x.to_f32(), u.pos.z.to_f32())).unwrap();
        // (Where they stand when the orders go: the squads settle a little after being put.)
        let mut before = Vec::new();
        let mut sent = Vec::new();
        let mut carried = Vec::new();
        for _ in 0..2 * 30 {
            let mut commands = std::mem::take(&mut carried);
            commands.extend(run.commands(&mut sim));
            let orders: Vec<Command> = commands.iter().filter(|c| matches!(c, Command::Order { .. } | Command::Formation { .. })).cloned().collect();
            if before.is_empty() && !orders.is_empty() {
                before = kept.iter().chain(&loose).map(|&u| (u, at(&sim, u))).collect();
            }
            sent.extend(orders);
            sim.step(&commands);
            sim.take_events();
            carried = run.after_ticks(&mut sim).0;
        }
        let near = |p: FxVec3, (x, z): (f32, f32)| (p.x.to_f32() - x).abs() < 0.5 && (p.z.to_f32() - z).abs() < 0.5;
        let sorted = |mut v: Vec<UnitId>| {
            v.sort_by_key(|u| u.slot);
            v
        };
        // Kept: one formation, its middle moved by the offset.
        let middle = before.iter().take(3).fold((0.0, 0.0), |(x, z), (_, p)| (x + p.0 / 3.0, z + p.1 / 3.0));
        let formations: Vec<&Command> = sent.iter().filter(|c| matches!(c, Command::Formation { .. })).collect();
        assert_eq!(formations.len(), 1, "one formation: {sent:?}");
        let Command::Formation { units, order: Order::Move(to), shape: ta_sim::formation::Shape::Keep, .. } = formations[0] else { panic!("{:?}", formations[0]) };
        assert_eq!(sorted(units.clone()), sorted(kept.clone()));
        assert!(near(*to, (middle.0 + 150.0, middle.1)), "the formation sent to {to:?}");
        // Loose: an order each, to its own place moved by the offset.
        for &(u, (x, z)) in &before[3..] {
            let own: Vec<&Command> = sent.iter().filter(|c| matches!(c, Command::Order { units, .. } if units.contains(&u))).collect();
            assert_eq!(own.len(), 1, "{u:?}: {own:?}");
            let Command::Order { units, order: Order::Move(to), .. } = own[0] else { panic!("{:?}", own[0]) };
            assert_eq!(units, &vec![u]);
            assert!(near(*to, (x + 150.0, z)), "{u:?} sent to {to:?}");
        }
        // Spot: one order to it for the three.
        let to_spot: Vec<&Command> = sent.iter().filter(|c| matches!(c, Command::Order { units, .. } if units.iter().any(|u| spot.contains(u)))).collect();
        assert_eq!(to_spot.len(), 1, "{to_spot:?}");
        let Command::Order { units, order: Order::Move(to), .. } = to_spot[0] else { panic!("{:?}", to_spot[0]) };
        assert_eq!(sorted(units.clone()), sorted(spot.clone()));
        assert!(near(*to, (2700.0, 600.0)));
        assert!(!sent.iter().any(|c| matches!(c, Command::Order { units, .. } if units.iter().any(|u| kept.contains(u)))), "the kept squad has no orders of its own");
    }
}
