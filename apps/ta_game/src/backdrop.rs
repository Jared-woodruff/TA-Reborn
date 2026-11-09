//! The title menu's backdrop: a film. Short scenes, each staged live on one of TA Reborn's own
//! worlds (`backdrop/*.toml`, in `--demo stage`'s format: see `stage.rs`), a world a visit to the
//! menu and its scenes in turn, cut together through black. As the screen goes dark the scene
//! before is taken away (its units, its wrecks, fires and smoke, its sounds) and the next one's
//! cast put down; its orders and events come on its own clock, and its light, weather and camera
//! are its own. A caption names the world and the moment as each scene comes up.
//!
//! The camera is the scene's free camera, the picture shifted right of the title menu
//! ([`Film::camera`]: a lens shift, the perspective unchanged) and leaning a little towards the
//! pointer; with reduced motion it holds each scene's first view. Left alone on the title menu a
//! while (or asked to: WATCH), the menu steps aside and the film plays on its own between black
//! bars, its subject in the middle ([`Film::watching`]); any key, click or move brings the menu
//! back.
//!
//! The worlds take turns from one visit to the next. In a season (the launcher's seasonal
//! setting: Halloween, Christmas, the New Year) its scenes join their worlds' own, and the first
//! visit opens on one of them. With a mod that leaves out the units the scenes need, the battle of old plays instead
//! ([`Plan::choose`] finds nothing to play), as it does when the player picks it (Settings ·
//! Interface · MENU BACKDROP). Captures: `TA_BACKDROP=<world>/<scene>` starts there (`<world>`
//! alone: its opener), `TA_BACKDROP_WATCH=1` with the menu stepped aside; `TA_BACKDROP_DIR=<dir>`
//! reads the scenes from that folder's files (`backdrop/`'s, being written) rather than the game's.

use glam::{Vec2, Vec3};
use ta_sim::{Cheat, Command};
use ta_ui::i18n::n;
use ta_ui::seasonal::Celebration;

use crate::game::Game;
use crate::setup::{Assets, GameSetup};
use crate::stage::{FreeView, Stage, StageRun};

/// A scene of the film.
pub struct Shot {
    /// Its world: a map of TA Reborn's own (all a world's scenes are on it).
    pub world: &'static str,
    /// Its name among the world's scenes (`TA_BACKDROP=<world>/<id>`).
    pub id: &'static str,
    /// What the caption says of the moment (translatable).
    pub caption: &'static str,
    /// Seconds on screen, the fades through black at either end included.
    pub length: f32,
    /// Shown only in its season (then among its world's own scenes), or always (None).
    pub season: Option<Celebration>,
    /// The scene its world opens on.
    pub opener: bool,
    /// The scene itself (`stage.rs`'s format; its `map` is its world's): its file in
    /// `backdrop/`, and its text.
    pub scene: (&'static str, &'static str),
}

/// A scene file of `backdrop/`: its name and its text.
macro_rules! scene {
    ($file:literal) => {
        ($file, include_str!(concat!("../backdrop/", $file)))
    };
}

/// Every scene, world by world.
pub const SHOTS: &[Shot] = &[
    Shot {
        world: "Highland Rift",
        id: "crossing",
        caption: n("The Arm fords the river at first light"),
        length: 26.0,
        season: None,
        opener: true,
        scene: scene!("highland_crossing.toml"),
    },
    Shot {
        world: "Highland Rift",
        id: "base",
        caption: n("A Commander builds as the sun goes down"),
        length: 28.0,
        season: None,
        opener: false,
        scene: scene!("highland_base.toml"),
    },
    Shot {
        world: "Highland Rift",
        id: "storm",
        caption: n("The lines meet in the rain"),
        length: 26.0,
        season: None,
        opener: false,
        scene: scene!("highland_storm.toml"),
    },
    Shot {
        world: "Highland Rift",
        id: "halloween",
        caption: n("Pyros in the woods under the hunter's moon"),
        length: 26.0,
        season: Some(Celebration::Halloween),
        opener: false,
        scene: scene!("highland_halloween.toml"),
    },
    Shot {
        world: "Hollyridge",
        id: "christmas",
        caption: n("Dusk in the village: the patrol stops by the great fir"),
        length: 26.0,
        season: Some(Celebration::Christmas),
        opener: false,
        scene: scene!("hollyridge_christmas.toml"),
    },
    Shot {
        world: "Midnight Square",
        id: "newyear",
        caption: n("A truce in the square as the New Year comes in"),
        length: 26.0,
        season: Some(Celebration::NewYear),
        opener: false,
        scene: scene!("midnight_newyear.toml"),
    },
    Shot {
        world: "Sapphire Reach",
        id: "fleet",
        caption: n("An Arm fleet sails the channel"),
        length: 26.0,
        season: None,
        opener: true,
        scene: scene!("sapphire_fleet.toml"),
    },
    Shot {
        world: "Sapphire Reach",
        id: "raid",
        caption: n("Thunders out of the setting sun"),
        length: 26.0,
        season: None,
        opener: false,
        scene: scene!("sapphire_raid.toml"),
    },
    Shot {
        world: "Sapphire Reach",
        id: "broadside",
        caption: n("Battle lines at nightfall"),
        length: 26.0,
        season: None,
        opener: false,
        scene: scene!("sapphire_broadside.toml"),
    },
    Shot {
        world: "Cinder Basin",
        id: "titan",
        caption: n("A Krogoth walks out of the dusk"),
        length: 26.0,
        season: None,
        opener: true,
        scene: scene!("cinder_titan.toml"),
    },
    Shot {
        world: "Cinder Basin",
        id: "siege",
        caption: n("The Core holds the river by night"),
        length: 26.0,
        season: None,
        opener: false,
        scene: scene!("cinder_siege.toml"),
    },
    Shot {
        world: "Mirage Basin",
        id: "wadi",
        caption: n("An Arm column comes down the wadi at sundown"),
        length: 26.0,
        season: None,
        opener: true,
        scene: scene!("mirage_wadi.toml"),
    },
    Shot {
        world: "Mirage Basin",
        id: "oasis",
        caption: n("The Core raids the oasis by moonlight"),
        length: 26.0,
        season: None,
        opener: false,
        scene: scene!("mirage_oasis.toml"),
    },
];

/// Seconds of the fade from black that opens a scene, and of the fade to black that ends it.
const FADE_IN: f32 = 1.4;
const FADE_OUT: f32 = 1.0;
/// The caption: in from this second of a scene, out by that one, fading over this long.
const CAPTION_FROM: f32 = 1.6;
const CAPTION_UNTIL: f32 = 9.5;
const CAPTION_FADE: f32 = 0.8;
/// How far the eye leans towards the pointer: this share of the distance to what it looks at,
/// at most this many world px (and half that up and down).
const LEAN: f32 = 0.025;
const MOST_LEAN: f32 = 28.0;
/// The eye stays this far over the ground or the sea, leaning or not.
const LOWEST_EYE: f32 = 2.0;
/// How fast the lean, the picture's shift and the watching ease to where they are going (per
/// second, exponentially).
const LEAN_EASE: f32 = 2.2;
const COMPOSE_EASE: f32 = 1.6;
const WATCH_EASE: f32 = 2.0;
/// How fast what the camera looks at follows where the scene puts it (per second,
/// exponentially): a camera tracking units turns smoothly when one of them dies or a new one
/// joins (their middle jumps), and through a swing after aircraft; a still or slowly eased
/// look is followed all but exactly.
const LOOK_EASE: f32 = 5.0;
/// Seconds the title menu is left alone before the film plays on its own, and how long the
/// pointer may still move (after WATCH) before a move brings the menu back.
pub const WATCH_AFTER: f32 = 45.0;
pub const WATCH_GRACE: f32 = 0.8;
/// Seconds into playing on its own that the pointer is hidden (the menu has stepped aside by
/// then); a move shows it again with the menu.
pub const HIDE_POINTER_AFTER: f32 = 1.5;
/// How far the film steps back behind the title menu (the menus' backdrop grade: 1 behind a
/// page).
pub const TITLE_BACKDROP: f32 = 0.3;
/// How loud its battles are, of the effects volume: under the menu's music, close as the camera
/// is to them; a little louder while the film plays on its own (it is the point then), quieter
/// behind a page (the page is).
pub const SOUND_LEVEL: f32 = 0.55;
const WATCH_SOUND_LEVEL: f32 = 0.75;
const PAGE_SOUND_LEVEL: f32 = 0.3;
/// The lens: its largest blur (of the picture's height), and how far either side of the focus
/// stays sharp (a share of the distance).
const LENS_BLUR: f32 = 0.0045;
const LENS_BAND: f32 = 0.3;

/// The film's depth of field, focused `focus` world px away.
pub fn lens(focus: f32) -> ta_render::post::DepthOfField {
    ta_render::post::DepthOfField { mode: ta_render::post::DofMode::Lens, focus, blur: LENS_BLUR, band: LENS_BAND }
}

/// A world's film, ready to load: its scenes, the first of them first.
pub struct Plan {
    world: &'static str,
    /// (an index into [`SHOTS`], its scene as read, with what is not installed left out).
    scenes: Vec<(usize, Stage)>,
    seed: u64,
}

impl Plan {
    /// The film for a visit to the menu: `TA_BACKDROP`'s world (and scene); else the world after
    /// `last` (or, the first visit, one by the `seed`), and in a season with scenes of its own
    /// the first visit's world is one of the season's, its scene first; none when no world can
    /// play (its map missing, or a mod without the units its scenes need). In season a world's
    /// scenes are its own and the season's; out of it, its own.
    pub fn choose(assets: &Assets, season: Option<Celebration>, last: Option<&str>, seed: u64) -> Option<Plan> {
        let wanted = std::env::var("TA_BACKDROP").ok();
        let (wanted_world, wanted_scene) = match wanted.as_deref().map(|w| w.split_once('/').map_or((w, None), |(w, s)| (w, Some(s)))) {
            Some((w, s)) => (Some(w.trim().to_string()), s.map(|s| s.trim().to_string())),
            None => (None, None),
        };
        let seasonal = |s: &Shot| s.season.is_some() && s.season == season;
        let shown = |s: &Shot| wanted_world.is_some() || s.season.is_none() || seasonal(s);
        for world in worlds_in_turn(season, last, seed, wanted_world.as_deref()) {
            if !assets.maps.iter().any(|m| m.name.eq_ignore_ascii_case(world)) {
                log::warn!("menu film: no world {world}");
                continue;
            }
            let mut scenes: Vec<(usize, Stage)> = Vec::new();
            for (i, shot) in SHOTS.iter().enumerate().filter(|(_, s)| s.world == world && shown(s)) {
                let text = match std::env::var_os("TA_BACKDROP_DIR") {
                    Some(dir) => std::fs::read_to_string(std::path::Path::new(&dir).join(shot.scene.0)).unwrap_or_else(|_| shot.scene.1.to_string()),
                    None => shot.scene.1.to_string(),
                };
                let mut stage = match Stage::parse(&text) {
                    Ok(stage) => stage,
                    Err(e) => {
                        log::warn!("menu film: {}/{}: {e:#}", world_key(world), shot.id);
                        continue;
                    }
                };
                let gone = stage.prune_missing(|k| assets.types.id(k).is_some());
                if stage.units.is_empty() {
                    log::info!("menu film: {}/{} left out (not installed: {})", world_key(world), shot.id, gone.join(", "));
                    continue;
                }
                scenes.push((i, stage));
            }
            if scenes.is_empty() {
                continue;
            }
            // The scene asked for first, or the season's, or its opener.
            let first = wanted_scene
                .as_deref()
                .and_then(|w| scenes.iter().position(|(i, _)| SHOTS[*i].id.eq_ignore_ascii_case(w)))
                .or_else(|| scenes.iter().position(|(i, _)| seasonal(&SHOTS[*i])))
                .or_else(|| scenes.iter().position(|(i, _)| SHOTS[*i].opener))
                .unwrap_or(seed as usize % scenes.len());
            scenes.swap(0, first);
            return Some(Plan { world, scenes, seed });
        }
        None
    }

    /// The world.
    pub fn world(&self) -> &'static str {
        self.world
    }

    /// The battle its scenes play in: the scene with the most players', so there are enough for
    /// every scene (a world's scenes share their players' sides, colours and teams), watched by
    /// nobody's eyes (as the menu's battle always is).
    pub fn setup(&self, assets: &Assets) -> anyhow::Result<GameSetup> {
        let widest = self.scenes.iter().max_by_key(|(_, s)| s.player_count()).map_or(&self.scenes[0].1, |(_, s)| s);
        let mut setup = widest.setup(Some(self.world), assets)?;
        setup.local_player = None;
        setup.seed = self.seed;
        Ok(setup)
    }
}

/// The worlds to try, the first first: those with scenes to show (their own, out of every
/// season, and the `season`'s), from the one after `last` round the list; on the first visit
/// (no `last`) from the season's world (or by the `seed`). `wanted`: that world only (by name,
/// or `TA_BACKDROP`'s key), whatever its scenes' seasons.
fn worlds_in_turn(season: Option<Celebration>, last: Option<&str>, seed: u64, wanted: Option<&str>) -> Vec<&'static str> {
    let seasonal = |s: &Shot| s.season.is_some() && s.season == season;
    let mut worlds: Vec<&'static str> = Vec::new();
    for s in SHOTS.iter().filter(|s| wanted.is_some() || s.season.is_none() || seasonal(s)) {
        if !worlds.contains(&s.world) {
            worlds.push(s.world);
        }
    }
    if let Some(w) = wanted {
        worlds.retain(|x| x.eq_ignore_ascii_case(w) || world_key(x) == w.to_ascii_lowercase());
    }
    if worlds.is_empty() {
        return worlds;
    }
    let season_world = SHOTS.iter().find(|s| seasonal(s)).and_then(|s| worlds.iter().position(|w| *w == s.world));
    let start = match last.and_then(|l| worlds.iter().position(|w| w.eq_ignore_ascii_case(l))) {
        Some(i) => i + 1,
        None => season_world.unwrap_or(seed as usize % worlds.len()),
    };
    (0..worlds.len()).map(|k| worlds[(start + k) % worlds.len()]).collect()
}

/// `TA_BACKDROP`'s name for a world: its name in lower case, without spaces.
fn world_key(world: &str) -> String {
    world.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase()
}

/// The film playing behind the menu.
pub struct Film {
    world: &'static str,
    scenes: Vec<(usize, Stage)>,
    /// The scenes still to come this round (indices into `scenes`), the next last.
    queue: Vec<usize>,
    /// The scene playing (an index into `scenes`) and its run.
    current: usize,
    run: StageRun,
    /// How many features the world had before any scene: those after are the scenes' own
    /// (wrecks and heaps), taken away at each cut.
    features: usize,
    rng: u64,
    /// The view held with reduced motion: the scene's first.
    held: Option<FreeView>,
    /// Eased: how far the eye leans towards the pointer (-1..1 each way), how far the picture
    /// stands right of the middle (0..1 of the way the title menu asks), and how far the menu
    /// has stepped aside for the film (0..1).
    lean: Vec2,
    compose: f32,
    watch: f32,
    /// What the camera looks at, following the scene's (see [`LOOK_EASE`]); none until the
    /// scene's first view (each cut starts it again).
    look: Option<Vec3>,
    /// The film plays on its own, the menu stepped aside; for how long so far (seconds).
    pub watching: bool,
    watched: f32,
}

/// What the film shows this frame, for the menu to draw over it.
#[derive(Clone, Debug, Default)]
pub struct Showing {
    /// How dark the cut through black is (0 none, 1 black).
    pub black: f32,
    /// The world, the moment, and how far the caption is in (0..1).
    pub caption: Option<(&'static str, &'static str, f32)>,
    /// How far the menu has stepped aside for the film (0 not at all, 1 gone, the bars in).
    pub watch: f32,
    /// How far the hint on bringing the menu back is in (0..1: at first, while the film plays
    /// on its own).
    pub hint: f32,
}

impl Film {
    /// Starts the film in its battle (loaded from [`Plan::setup`]): the first scene's cast put
    /// down, the battle's own players' computers retired.
    pub fn begin(plan: Plan, game: &mut Game) -> Film {
        let features = game.sim.features.len();
        game.tally.enabled = false;
        let run = start_scene(&plan.scenes[0].1, game, true);
        let mut film = Film {
            world: plan.world,
            scenes: plan.scenes,
            queue: Vec::new(),
            current: 0,
            run,
            features,
            rng: plan.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
            held: None,
            lean: Vec2::ZERO,
            compose: 1.0,
            watch: 0.0,
            look: None,
            watching: std::env::var("TA_BACKDROP_WATCH").is_ok_and(|v| v == "1"),
            watched: 0.0,
        };
        if film.watching {
            film.watch = 1.0;
            film.compose = 0.0;
        }
        film.refill();
        film
    }

    /// The world playing.
    pub fn world(&self) -> &'static str {
        self.world
    }

    fn shot(&self) -> &'static Shot {
        &SHOTS[self.scenes[self.current].0]
    }

    fn stage(&self) -> &Stage {
        &self.scenes[self.current].1
    }

    /// Seconds into the scene playing.
    fn clock(&self, game: &Game) -> f32 {
        self.run.seconds(game).max(0.0)
    }

    /// A new round of the world's scenes, shuffled (the one just shown not first again).
    fn refill(&mut self) {
        let mut order: Vec<usize> = (0..self.scenes.len()).filter(|&i| i != self.current || self.scenes.len() == 1).collect();
        for i in (1..order.len()).rev() {
            let j = (self.next_random() % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
        // (Popped from the end: the scene just shown, when every one is, goes last.)
        if self.scenes.len() > 1 {
            order.insert(0, self.current);
        }
        self.queue = order;
    }

    fn next_random(&mut self) -> u64 {
        // xorshift64*: the order the scenes come in (nothing of the battle's own).
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Before the frame's ticks: the scene's orders and events due.
    pub fn before_update(&mut self, game: &mut Game) {
        self.run.before_update(game);
    }

    /// After the frame's ticks: the scene's blasts and frames; at its end, the cut to the next
    /// one (true: the app takes away what is drawn of the last, its particles and sounds).
    pub fn after_update(&mut self, game: &mut Game) -> bool {
        self.run.after_update(game);
        if self.clock(game) < self.shot().length {
            return false;
        }
        if self.queue.is_empty() {
            self.refill();
        }
        self.current = self.queue.pop().unwrap_or(0);
        // What the last scene left goes: its fires, its wrecks and heaps, its effects.
        game.sim.extinguish_fires();
        let first = self.features;
        game.sim.clear_features_where(|i, _| i >= first);
        game.effects.clear_battle();
        let stage = self.scenes[self.current].1.clone();
        self.run = start_scene(&stage, game, false);
        self.held = None;
        self.look = None;
        true
    }

    /// How loud the scene's battle is this frame, of the effects volume: [`SOUND_LEVEL`] behind
    /// the title menu (`on_title`), more as the film plays on its own, less behind a page; fading
    /// out with the picture into each cut and in again after it (what is still sounding at the
    /// cut is stopped by then, not cut off at full strength).
    pub fn sound_level(&self, game: &Game, on_title: bool) -> f32 {
        let level = if on_title { SOUND_LEVEL + (WATCH_SOUND_LEVEL - SOUND_LEVEL) * self.watch } else { PAGE_SOUND_LEVEL };
        level * (1.0 - self.black(game))
    }

    /// How dark the cut through black is this frame (0 none, 1 black).
    fn black(&self, game: &Game) -> f32 {
        let t = self.clock(game);
        let length = self.shot().length;
        (1.0 - smooth(t / FADE_IN)).max(smooth((t - (length - FADE_OUT)) / FADE_OUT))
    }

    /// The time of day the scene has now, if it sets one.
    pub fn hours(&self, game: &Game) -> Option<f32> {
        self.stage().hours(self.clock(game))
    }

    /// Where the scene's moon is, if it puts it somewhere.
    pub fn moon_dir(&self) -> Option<Vec3> {
        self.stage().moon_dir()
    }

    /// What the menu draws over the film this frame.
    pub fn showing(&self, game: &Game) -> Showing {
        let t = self.clock(game);
        let black = self.black(game);
        let caption_in = smooth((t - CAPTION_FROM) / CAPTION_FADE).min(1.0 - smooth((t - CAPTION_UNTIL) / CAPTION_FADE));
        let caption = (caption_in > 0.0).then(|| (self.world, self.shot().caption, caption_in));
        let hint = if self.watching { smooth((self.watched - 0.8) / 0.8).min(1.0 - smooth((self.watched - 6.0) / 1.0)) } else { 0.0 };
        Showing { black, caption, watch: self.watch, hint }
    }

    /// Seconds the film has played on its own (none while the menu is up).
    pub fn watched(&self) -> f32 {
        self.watched
    }

    /// The camera this frame: the scene's free camera at its clock (its first view held, with
    /// `still`), leaning towards the `pointer` (-1..1 across and down the window, or None), the
    /// picture shifted `shift` px right (the title menu's room, or none while watching), its
    /// depth of field focused on what it looks at (the distance, returned).
    pub fn camera(&mut self, game: &Game, camera: &mut ta_render::Camera, pointer: Option<Vec2>, shift: f32, still: bool, dt: f32) -> Option<f32> {
        let k = |rate: f32| 1.0 - (-dt.max(0.0) * rate).exp();
        self.watched = if self.watching { self.watched + dt.max(0.0) } else { 0.0 };
        let watch_to = if self.watching { 1.0 } else { 0.0 };
        self.watch += (watch_to - self.watch) * k(WATCH_EASE);
        if (self.watch - watch_to).abs() < 0.002 {
            self.watch = watch_to;
        }
        let compose_to = 1.0 - self.watch;
        self.compose += (compose_to - self.compose) * k(COMPOSE_EASE);
        let lean_to = if still { Vec2::ZERO } else { pointer.unwrap_or(Vec2::ZERO).clamp(Vec2::splat(-1.0), Vec2::splat(1.0)) };
        self.lean += (lean_to - self.lean) * k(LEAN_EASE);
        let clock = self.clock(game);
        let view = if still {
            match self.held {
                Some(v) => Some(v),
                None => {
                    let v = self.run.free_view(game, 0.0);
                    self.held = v;
                    v
                }
            }
        } else {
            self.run.free_view(game, clock)
        };
        camera.view_offset = Vec2::new(shift * self.compose, 0.0);
        let Some(mut v) = view else {
            // (A scene on a path rather than the free camera: as it is filmed.)
            self.run.camera(game, camera, clock);
            return None;
        };
        // What it looks at, followed smoothly (held still, the first view is all there is).
        if !still {
            let target = looked_at(&v);
            let look = self.look.map_or(target, |l| l + (target - l) * k(LOOK_EASE));
            self.look = Some(look);
            look_at(&mut v, look);
        }
        let world = &game.sim.world;
        let sea = world.sea_level.to_f32();
        let ground = |x: f32, z: f32| crate::scene::drawn_ground(world, x, z).max(sea);
        lean(&mut v, self.lean, &ground);
        camera.fov_y = v.fov;
        camera.roll = v.roll;
        camera.near = Some(v.near);
        camera.place(v.eye, v.yaw, v.pitch, v.distance);
        Some(v.distance)
    }
}

/// Smooth step over 0..1 (held at the ends).
fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Turns the view to look at `at` from where its eye is (the distance to it too: where the lens
/// focuses).
fn look_at(v: &mut FreeView, at: Vec3) {
    let d = at - v.eye;
    if d.length() < 1.0 {
        return;
    }
    v.yaw = d.x.atan2(-d.z);
    v.pitch = -d.y.atan2(Vec2::new(d.x, d.z).length());
    v.distance = d.length();
}

/// What a view looks at: the place `distance` along its angles.
fn looked_at(v: &FreeView) -> Vec3 {
    let (sy, cy) = v.yaw.sin_cos();
    let (sp, cp) = v.pitch.sin_cos();
    v.eye + Vec3::new(sy * cp, -sp, -cy * cp) * v.distance
}

/// Puts a scene's cast down in the battle (what stood there before taken off), with its weather
/// and nothing else's: its rain or the world's own weather, its players' god mode or none.
fn start_scene(stage: &Stage, game: &mut Game, first: bool) -> StageRun {
    if !first {
        for p in 0..game.sim.players.len() as u8 {
            if !stage.players.iter().any(|s| s.index == p && s.invulnerable) {
                game.direct(Command::Cheat { player: p, cheat: Cheat::Invulnerable(false) });
            }
        }
    }
    let run = StageRun::begin(stage, game);
    game.effects.scene_weather(stage.rain.unwrap_or(false), crate::scene::game_seconds(game.sim.tick, 0.0) as f32);
    run
}

/// Leans the eye `by` (-1..1 across and down) round what it looks at: a little to the side and
/// up or down, still looking there, never under the ground.
fn lean(v: &mut FreeView, by: Vec2, ground: &dyn Fn(f32, f32) -> f32) {
    if by.length_squared() < 1e-6 {
        return;
    }
    let (sy, cy) = v.yaw.sin_cos();
    let (sp, cp) = v.pitch.sin_cos();
    let forward = Vec3::new(sy * cp, -sp, -cy * cp);
    let look = v.eye + forward * v.distance;
    let right = Vec3::new(cy, 0.0, sy);
    let up = right.cross(forward).normalize_or(Vec3::Y);
    let reach = (v.distance * LEAN).min(MOST_LEAN);
    let mut eye = v.eye + right * (by.x * reach) - up * (by.y * reach * 0.5);
    let floor = ground(eye.x, eye.z);
    eye.y = eye.y.max(floor + LOWEST_EYE);
    let d = look - eye;
    if d.length() < 1.0 {
        return;
    }
    v.yaw = d.x.atan2(-d.z);
    v.pitch = -d.y.atan2(Vec2::new(d.x, d.z).length());
    v.distance = d.length();
    v.near = ((eye.y - floor) * 0.5).clamp(1.0, 10.0);
    v.eye = eye;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scene_reads_and_is_on_its_world() {
        for s in SHOTS {
            let stage = Stage::parse(s.scene.1).unwrap_or_else(|e| panic!("{}/{}: {e:#}", world_key(s.world), s.id));
            assert!(stage.map.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(s.world)), "{}/{}: on {:?}", s.world, s.id, stage.map);
            assert!(!stage.camera.keys.is_empty(), "{}/{}: the film needs the free camera", s.world, s.id);
            assert!(s.length > FADE_IN + FADE_OUT + CAPTION_UNTIL, "{}/{}: too short for its caption", s.world, s.id);
            assert_eq!(stage.missing, crate::stage::Missing::Skip, "{}/{}: missing = \"skip\" (a mod may lack some units)", s.world, s.id);
        }
        // A world's scenes share their players (the battle is one): no two give one another side,
        // colour or team.
        let mut players: std::collections::BTreeMap<(&str, u8), (Option<String>, Option<u8>, Option<u8>)> = Default::default();
        for s in SHOTS {
            for p in Stage::parse(s.scene.1).unwrap().players {
                let spec = (p.faction.map(|f| f.to_ascii_uppercase()), p.color, p.team);
                let before = players.entry((s.world, p.index)).or_insert_with(|| spec.clone());
                assert_eq!(*before, spec, "{}/{}: player {} unlike in the world's other scenes", s.world, s.id, p.index);
            }
        }
        // Every world opens on one scene (a season's world, on its season's).
        let mut worlds: Vec<&str> = SHOTS.iter().map(|s| s.world).collect();
        worlds.dedup();
        for w in worlds {
            let own = SHOTS.iter().filter(|s| s.world == w && s.season.is_none()).count();
            let openers = SHOTS.iter().filter(|s| s.world == w && s.opener).count();
            assert_eq!(openers, usize::from(own > 0), "{w}: one opener among its own scenes");
            assert!(SHOTS.iter().filter(|s| s.world == w && s.opener).all(|s| s.season.is_none()), "{w}: a season's scene opens in its season only");
        }
    }

    #[test]
    fn the_worlds_take_turns_and_a_season_opens_on_its_own() {
        let own: Vec<&str> = worlds_in_turn(None, None, 0, None);
        assert!(own.len() >= 3, "{own:?}");
        // (Out of season no season's world is in the list.)
        assert!(!own.iter().any(|w| SHOTS.iter().filter(|s| s.world == *w).all(|s| s.season.is_some())), "{own:?}");
        // Each visit the next world, round the list.
        for (i, w) in own.iter().enumerate() {
            assert_eq!(worlds_in_turn(None, Some(w), 0, None)[0], own[(i + 1) % own.len()]);
        }
        // A season's first visit is on its world; later visits go round them all, its own too.
        for season in [Celebration::Halloween, Celebration::Christmas, Celebration::NewYear] {
            let world = SHOTS.iter().find(|s| s.season == Some(season)).map(|s| s.world).expect("a scene for every season");
            let first = worlds_in_turn(Some(season), None, 7, None);
            assert_eq!(first[0], world, "{season:?}");
            assert!(first.len() >= own.len(), "{season:?}: {first:?}");
            assert!(worlds_in_turn(Some(season), Some(world), 7, None).contains(&world));
        }
        // Asked for: that world alone, by its name or its key.
        assert_eq!(worlds_in_turn(None, None, 0, Some("hollyridge")), vec!["Hollyridge"]);
        assert_eq!(worlds_in_turn(None, None, 0, Some("Sapphire Reach")), vec!["Sapphire Reach"]);
    }

    #[test]
    fn leaning_keeps_looking_at_the_same_place() {
        let mut v = FreeView { eye: Vec3::new(100.0, 50.0, 100.0), yaw: 0.7, pitch: 0.2, roll: 0.0, fov: 0.6, distance: 400.0, near: 5.0 };
        let forward = |v: &FreeView| Vec3::new(v.yaw.sin() * v.pitch.cos(), -v.pitch.sin(), -v.yaw.cos() * v.pitch.cos());
        let look = v.eye + forward(&v) * v.distance;
        lean(&mut v, Vec2::new(1.0, -0.5), &|_, _| 0.0);
        let after = v.eye + forward(&v) * v.distance;
        assert!(after.distance(look) < 0.01, "looks at {after:?}, not {look:?}");
        assert!(v.eye.distance(Vec3::new(100.0, 50.0, 100.0)) > 5.0, "the eye moved");
        // Never under the ground.
        let mut low = FreeView { eye: Vec3::new(0.0, 3.0, 0.0), yaw: 0.0, pitch: -0.3, roll: 0.0, fov: 0.6, distance: 300.0, near: 1.0 };
        lean(&mut low, Vec2::new(0.0, 1.0), &|_, _| 0.0);
        assert!(low.eye.y >= LOWEST_EYE - 1e-4, "eye at {}", low.eye.y);
    }
}
