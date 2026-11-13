//! Window, screen flow (intro → menu → game), main loop, camera controls, dev overlay.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use glam::{Vec2, Vec3};
use ta_render::{Camera, CameraMode, Renderer};
use ta_vfs::Vfs;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::KeyCode;
use winit::window::{Window, WindowId};

mod progress;
mod lifecycle;
mod credits_play;
mod clip_play;
mod spy;
mod memory_guard;

use crate::Args;
use crate::audio::GameAudio;
use crate::game::{Game, Outcome};
use crate::hud::Hud;
use crate::input::{Input, MouseButtonId};
use crate::intro::MoviePlayer;
use crate::menu::{Menu, MenuAction};
use crate::minimap::Minimap;
use crate::setup::{Assets, Controller, GameSetup, SlotSetup};

pub fn run(args: Args, vfs: Vfs, mods: crate::record_mods::InPlay, config: ta_config::Config, profile: Option<ta_config::profiles::Profile>) -> Result<()> {
    let event_loop = EventLoop::<egui_winit::accesskit_winit::Event>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App { args, vfs: Arc::new(vfs), mods, config, profile, state: None, active_load: None, error: None, proxy: event_loop.create_proxy(), accessibility_active: false };
    event_loop.run_app(&mut app)?;
    app.goodbye();
    crate::photo::wait_for_saves(std::time::Duration::from_secs(5));
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

struct App {
    proxy: EventLoopProxy<egui_winit::accesskit_winit::Event>,
    accessibility_active: bool,
    args: Args,
    vfs: Arc<Vfs>,
    /// The mods mounted with `vfs` (what saved games and replays name).
    mods: crate::record_mods::InPlay,
    config: ta_config::Config,
    profile: Option<ta_config::profiles::Profile>,
    state: Option<State>,
    active_load: Option<ActiveLoad>,
    error: Option<anyhow::Error>,
}

impl App {
    /// The program closing in a multiplayer battle or lobby: the others told we left (not
    /// that our connection failed), the word given a moment to go out.
    fn goodbye(&mut self) {
        let Some(state) = self.state.as_mut() else { return };
        let wait = std::time::Duration::from_millis(300);
        if let Some(net) = state.game.net.as_mut() {
            net.goodbye(wait);
        }
        if let Some(lobby) = state.menu.lobby.as_mut() {
            lobby.goodbye(wait);
        }
    }
}

struct Egui {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    /// Seconds added to the input state's clock: its own restarts at zero when it is rebuilt
    /// (after a loading screen), and egui's clock must not (see `carry_clock`).
    clock: f64,
}

impl Egui {
    /// The window's input for the next pass, on the clock that carries on.
    fn take_input(&mut self, window: &Window) -> egui::RawInput {
        let mut raw = self.state.take_egui_input(window);
        if let Some(time) = raw.time.as_mut() {
            *time += self.clock;
        }
        raw
    }
}

/// Forgets the input of the passes so far (held buttons, queued events), keeping the time of
/// the last: what the input state's clock must go on from once rebuilt. egui fades each panel
/// in from the time it last appeared, kept from pass to pass; on a clock started again from
/// zero, the panels up when a battle was left (the lesson's instructions, the deck, the
/// minimap, back since the cockpit view was left minutes in) stayed see-through in the next
/// battle until it had run as long.
///
/// What the window is stays too: its size, its scale, the card's largest texture. A state made
/// new has egui's placeholder for a size, 10,000 points square, and a zoom change waiting for
/// the next pass lays that pass out on the state's size, scaled, not on the window's. A battle
/// in a window under 1,024 x 600 points (its interface zoomed to fit, the menu's not) was laid
/// out for the placeholder for its first frame: the classic interface made its panels and its
/// letters nineteen times too large for it, seconds of work, and at 800 x 600 on a display at
/// 150% a letter atlas 17,357 pixels tall, more than a graphics card takes.
fn carry_clock(ctx: &egui::Context) -> f64 {
    let options = ctx.options(|o| o.input_options);
    ctx.input_mut(|input| {
        let time = input.time;
        let window = egui::RawInput {
            viewport_id: input.raw.viewport_id,
            viewports: input.raw.viewports.clone(),
            screen_rect: Some(input.viewport_rect()),
            safe_area_insets: Some(input.safe_area_insets()),
            max_texture_side: Some(input.max_texture_side),
            time: Some(time),
            focused: input.focused,
            ..Default::default()
        };
        let pixels_per_point = input.pixels_per_point;
        let mut fresh = egui::InputState::default();
        fresh.time = time;
        *input = fresh.begin_pass(window, false, pixels_per_point, options);
        time
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    /// Cutscenes before the menu.
    Intro,
    /// Title / skirmish setup over the attract-mode battle.
    Menu,
    Playing,
    /// The map editor (see `editor`; its frames are [`State::editor_frame`]).
    Editor,
    /// A space scene of the film's (`--open space`, see `space`; its frames are
    /// [`State::space_frame`]).
    Space,
}

struct State {
    /// The identity selected at process start stays pinned while the launcher switches users.
    profile: Option<ta_config::profiles::Profile>,
    career: ta_config::profiles::Career,
    profile_eligible: bool,
    profile_editor: Option<ta_ui::profiles::ProfileEditor>,
    career_window: ta_ui::profiles::CareerWindow,
    profile_resume: Option<(Screen, Option<Transition>)>,
    profile_network: Option<(Option<u16>, Option<String>)>,
    match_xp: u64,
    /// Notices of achievements earned and promotions, shown at the top right one at a time.
    achievement_toasts: ta_ui::profiles::CareerNotices,
    /// The career's XP of every kind when this battle began (with a profile), and the
    /// achievements earned in it and their XP: its record on the result's page.
    battle_start_xp: Option<u64>,
    battle_achievements: Vec<String>,
    battle_achievement_xp: u64,
    /// Achievements earned this moment, waiting for their picture (the battle, without the
    /// interface) at the next frame drawn.
    unlock_picture: Option<Vec<String>>,
    /// The tick the next look ahead for achievements earned between checkpoints is due, and
    /// those a look ahead has already saved early for (never twice for one).
    unlock_look_tick: u64,
    unlock_looked: std::collections::BTreeSet<&'static str>,
    window: Arc<Window>,
    /// Whether the loading presenter has exposed the initialized native window.
    window_shown: bool,
    renderer: RenderSlot,
    /// A completed worker load, consumed by the existing transition finalizer.
    loaded_game: Option<Game>,
    loaded_stage: Option<crate::stage::StageRun>,
    camera: Camera,
    input: Input,
    egui: Egui,
    assets: Assets,
    audio: GameAudio,
    game: Game,
    screen: Screen,
    menu: Menu,
    minimap: Minimap,
    hud: Hud,
    dev: DevPanel,
    movie: Option<MoviePlayer>,
    /// Captures: `TA_ROOM_TUBE=<dir>` plays that folder's frames (`f_0.png`, `f_1.png`...) on the
    /// intro room's tube, `TA_ROOM_TUBE_FPS` (30) a second, in place of TA's movies: the seconds
    /// since the monitor warmed up, and the frame shown.
    room_tube: Option<(f32, usize)>,
    movie_queue: Vec<&'static str>,
    /// The bedroom the intro plays in (see `intro_room.rs`), while it is up.
    intro_room: Option<crate::intro_room::IntroRoom>,
    /// A space scene (see `space.rs`), while it is up.
    space: Option<crate::space::SpaceView>,
    in_game_menu: bool,
    /// Camera bookmarks (Ctrl+F6..F9 store, F6..F9 recall): focus, zoom, yaw, pitch.
    bookmarks: [Option<(Vec3, f32, f32, f32)>; 4],
    attract: Attract,
    /// The film behind the title menu (`backdrop.rs`) while it plays (else the battle of old
    /// does); one the loading worker made, waiting; the world it last showed (the next visit
    /// shows another); seconds since the player last did anything on the title menu; whether
    /// its lens is on (taken off when it stops).
    menu_film: Option<crate::backdrop::Film>,
    loaded_menu_film: Option<crate::backdrop::Film>,
    menu_film_world: Option<&'static str>,
    menu_film_idle: f32,
    menu_film_lens: bool,
    /// How far the battle has stepped back behind a dialog over it (0..1, eased; see
    /// [`App::battle_veiled`]), and whether its blur is the lens in the picture.
    veil: f32,
    veil_lens: bool,
    last_frame: Instant,
    frames: u32,
    screenshot_path: Option<std::path::PathBuf>,
    screenshot_frame: u32,
    /// Scripted capture (`--screenshot`): exit once the image is saved.
    exit_after_screenshot: bool,
    /// `TA_SHOTS=n` with `--screenshot x.png`: a strip of n frames (x_0.png, x_1.png...),
    /// `TA_SHOT_STEP` frames apart (4), for judging motion.
    shots: Option<Shots>,
    /// Scripted demo actions (frame, action) for automated captures.
    script: Vec<(u32, DemoAction)>,
    /// Keys a script presses at the start of the next frame (`DemoAction::AppKey`).
    demo_keys: Vec<KeyCode>,
    /// Pointer events a script gives the interface on the next frame (`DemoAction::UiMove`,
    /// `DemoAction::UiButton`).
    demo_ui: Vec<egui::Event>,
    /// `--camera`: where the camera looks, also after a demo moves it (captures).
    camera_spec: Option<String>,
    pending: Option<Transition>,
    /// When to try loading the menu's own world again, after the graphics card had no room for it.
    backdrop_retry: Option<Instant>,
    /// A settings pass rewinds the benchmark; cover the rewind with the same interstitial.
    pending_pass: Option<crate::benchmark::Next>,
    /// Loading-screen preview for `--open loading` captures.
    loading: Option<String>,
    /// After a loading screen, its bay lifting off what was loaded: `Some(None)` until the first
    /// frame draws it, then when it began (the interface's clock).
    curtain: Option<Option<f64>>,
    /// `--demo siege`: the middle of the defence line (the nuke's target, and where a ride is
    /// taken).
    siege_middle: Option<Vec3>,
    /// `--demo war`: orders waiting for their tick (a group of lines a tick: lines sent off in
    /// the same tick share one route, and a whole army funnels into it).
    war_queue: Vec<(u64, ta_sim::Command)>,
    /// `--demo stage`: the scene being played (see `stage.rs`).
    stage: Option<crate::stage::StageRun>,
    /// `--benchmark`: the run (see `benchmark.rs`).
    benchmark: Option<crate::benchmark::Benchmark>,
    /// The last frame's time as it was (not held to the game's longest step): the benchmark's.
    frame_ms: f32,
    /// The map overlay uploaded last, and what it was built from (redrawn when that changes).
    overlay_key: Option<(crate::overlay::Overlay, u64)>,
    /// `--demo gallery` with `TA_GALLERY_TOUR`: units to photograph one by one, where the
    /// pictures go, the next one's index, and the tick the tour starts at.
    tour: Vec<(ta_sim::UnitId, String)>,
    tour_dir: Option<std::path::PathBuf>,
    tour_step: usize,
    tour_from: u64,
    /// The interface's clock in a capture at a fixed rate (`TA_CAPTURE_FPS`).
    capture_clock: f64,
    /// Whose fog of war the renderer holds (none: no fog), or unknown: to be uploaded again.
    fog_key: Option<Option<u8>>,
    /// Campaign progress (saved to its own file when a mission is won).
    progress: ta_config::CampaignProgress,
    /// User settings (edited from the in-game options, saved to config.toml).
    settings: ta_config::Config,
    /// The options panel is open, and its settings screen (the category shown...).
    options_open: bool,
    settings_view: ta_ui::settings::SettingsView,
    /// The controls reference (F1).
    controls_open: bool,
    /// Successful five-minute autosaves and bounded retries after storage failures.
    autosave: lifecycle::Autosave,
    /// The mission outcome has been captured once, independently of storage retries.
    progress_recorded: bool,
    progress_writes: progress::PendingWrites,
    /// The victory or defeat music is playing for this battle.
    outcome_music: bool,
    /// Where the camera glides once the battle is decided: where it was decided (the last
    /// Commander to fall), if that was lately.
    result_glide: Option<Vec3>,
    /// The antialiasing the renderer was made with (changes need a restart).
    msaa_started: u32,
    /// The tutorial lesson being played, if any.
    tutorial: Option<crate::tutorial::Tutorial>,
    /// The tip shown at the start of a skirmish, and a counter to pick the next one.
    tip: Option<&'static str>,
    tips_shown: u64,
    /// The coach's advice in a profile's first battles (see `coach.rs`).
    coach: Option<crate::coach::Coach>,
    /// Gamepads, and this frame's pad input (also handed to egui).
    gamepad: crate::gamepad::Gamepad,
    pad: crate::gamepad::PadFrame,
    /// The pad's d-pad moved the keyboard focus since its pointer or the mouse last moved: its A
    /// works what has the focus, not what is under the pointer.
    pad_focus: bool,
    /// Scrolls the control the pad's d-pad moved the focus to into view.
    pad_follow: ta_ui::pad::FocusFollow,
    /// A script's pad buttons for the next reading of the pads (`TA_UI_PAD`), and whether it
    /// has pressed any (a pad in use, as far as the menus are told).
    demo_pad: Vec<crate::gamepad::Nav>,
    demo_pad_used: bool,
    keyboard_pointer: crate::keyboard_pointer::KeyboardPointer,
    /// Testing: host auto-start player count and the lockstep probe tick.
    auto_start: Option<usize>,
    net_check_tick: Option<u64>,
    /// The map editor, while it is open.
    editor: Option<crate::editor::Editor>,
    /// The editor waiting, as it was left, for a test battle on its map to end (TEST PLAY).
    test_editor: Option<crate::editor::Editor>,
    /// Photo mode, while it is on; and the pause menu asked for it.
    photo: Option<crate::photo::PhotoMode>,
    /// A replay sought back is starting again (no tip, the companion screen as it was).
    rewinding: bool,
    /// The encyclopaedia's showroom: the unit on show in a world of its own (the menu's
    /// backdrop while the Units page is open).
    showroom: Option<crate::showroom::Showroom>,
    /// Text in the world: a staged scene's `[[text]]`s, the credits' roll (`world_text.rs`).
    world_text: crate::world_text::WorldText,
    /// The credits' film, while the credits' page is open (`credits_show.rs`, `app/credits_play.rs`).
    credits_show: Option<crate::credits_show::Show>,
    /// The jukebox (J).
    jukebox: crate::jukebox::Jukebox,
    /// The wallpapers' words: an interface of their own, drawn at the picture's size.
    wallpaper_ui: Option<(egui::Context, egui_wgpu::Renderer)>,
    open_photo: bool,
    /// Captures (`--open photo...`): photo mode's choices and the frame it opens at (the
    /// battle runs until then), and the frame a photo is taken at.
    photo_capture: Option<(String, u32)>,
    photo_take_frame: Option<u32>,
    /// The battle's highlights being watched.
    reel: Option<crate::highlights::Reel>,
    /// Captures (`--open highlights`): an AI battle fast-forwarded, then its highlights; (`--open
    /// cast`) one watched in the cast's view.
    highlights_capture: bool,
    cast_capture: bool,
    /// A replay being made into a film (`film.rs`), and a capture's (`TA_FILM`) waiting for its
    /// replay's scan.
    film: Option<crate::film::Film>,
    /// The exported picture's UI owns its input state and font textures separately
    /// from the window's progress page, whose Cancel button must remain interactive.
    film_ui: Option<crate::film::PictureUi>,
    film_capture: Option<crate::film::FilmSettings>,
    /// The EXPORT CLIP dialog's pictures being taken (`app/clip_play.rs`), and a capture's clip
    /// (`TA_CLIP`) waiting for its replay's scan.
    clip_previews: Option<clip_play::ClipPreviews>,
    clip_capture: Option<crate::clips::ClipSpec>,
    /// `--demo spy`'s radar planes over the enemy's base, a soak test of memory (`app/spy.rs`).
    spy: Option<spy::Spy>,
    /// The watch on the game's own memory (`app/memory_guard.rs`).
    memory: memory_guard::Guard,
    /// The companion screen (`companion.rs`), and whether the player was told it can be had.
    companion: Option<crate::companion::Companion>,
    companion_hinted: bool,
    /// Captures (`--open companion`): the companion screen opens with the battle.
    companion_capture: bool,
    /// Perspective: riding along in a unit (see `perspective.rs`); the picture's settings as
    /// they were, and its instruments' unit pictures.
    perspective: Option<crate::perspective::Perspective>,
    perspective_post: Option<ta_render::post::PostSettings>,
    cockpit_pics: rustc_hash::FxHashMap<usize, Option<egui::TextureHandle>>,
    /// A modelled cockpit's instruments, painted onto its glass (made when first needed).
    instruments: Option<crate::instruments::InstrumentPainter>,
    /// Draws the interface's portraits of units (made when first needed), and the portraits
    /// drawn (a cockpit's screens show them too, through a renderer of their own).
    portrait_studio: Option<ta_render::portrait::PortraitStudio>,
    portrait_views: rustc_hash::FxHashMap<crate::hud::PortraitKey, wgpu::TextureView>,
}

/// The most portraits of units drawn in a frame (the rest wait for the next).
const PORTRAITS_PER_FRAME: usize = 6;

/// Deferred screen changes (applied between frames, outside UI borrows).
// (One at a time: the size difference between its variants costs nothing.)
#[allow(clippy::large_enum_variant)]
enum Transition {
    StartGame(GameSetup),
    /// A tutorial lesson (see `tutorial.rs`).
    StartTutorial(usize),
    /// Continue a saved game, or watch a replay.
    Play(crate::replay::Record),
    /// Watch a replay again from the start, fast-forwarding to a tick (seeking backwards).
    Rewind(std::sync::Arc<crate::replay::Record>, u64),
    /// A multiplayer game with its lockstep session.
    StartNet(GameSetup, ta_net::Session),
    ToMenu,
    /// After a multiplayer battle: back to its lobby with the same people.
    ToLobby,
    /// A multiplayer battle whose connection failed: back into it (connecting to this address).
    Rejoin(String),
    /// Return from the unit showroom without changing the selected menu page.
    RestoreMenuBackdrop,
    /// Back to the campaign page with a mission selected (after a mission).
    ToCampaign { campaign: String, mission: usize },
    /// Back to the galaxy map (after a Conquest battle).
    ToConquest,
    /// Back to the scenarios (after one).
    ToScenarios,
    /// The skirmish setup, the computer players Easy (after the last lesson).
    ToSkirmish,
    PlayIntro,
    Quit,
    /// Open the map editor (with one of its dialogs or tools open, for captures).
    Editor(crate::editor::Start, Option<String>),
    /// Back to the editor that waited for a test battle on its map.
    ResumeEditor,
    /// Watch the battle's highlights (see `highlights.rs`).
    Highlights,
    /// The benchmark's scene `n` (see `benchmark.rs`).
    Benchmark(usize),
}

impl Transition {
    /// These routes all replace the renderer's terrain and prepare five minutes of
    /// simulated backdrop. They must use the loading worker, even when reached from
    /// the editor or an existing menu page rather than from a played battle.
    fn prepares_menu_backdrop(&self) -> bool {
        matches!(self, Self::ToMenu | Self::ToLobby | Self::Rejoin(_) | Self::ToCampaign { .. } | Self::ToConquest | Self::ToScenarios | Self::ToSkirmish | Self::RestoreMenuBackdrop)
    }

    fn leave_network_for_menu(&self, connection: &mut Option<ta_net::Session>) {
        if self.prepares_menu_backdrop() && let Some(mut session) = connection.take() {
            session.leave();
        }
    }
}

/// The coach for a battle starting: in a profile's first battles, or always, as set; in the
/// player's own battles (not a mission's, a replay's or a scripted run's). `TA_COACH=1` turns it
/// on, and `TA_COACH_SAY=<advice>` says that at once (captures).
fn coach_for(settings: &ta_config::Config, career: &ta_config::profiles::Career, setup: &crate::setup::GameSetup, unscripted: bool) -> Option<crate::coach::Coach> {
    let forced = std::env::var("TA_COACH").is_ok_and(|v| v == "1");
    let wanted = match settings.gameplay.coach {
        ta_config::Coach::Always => true,
        ta_config::Coach::FirstBattles => career.matches_finished < crate::coach::FIRST_BATTLES,
        ta_config::Coach::Off => false,
    };
    if !forced && !(wanted && unscripted && setup.mission.is_none() && setup.local_player.is_some()) {
        return None;
    }
    let mut coach = crate::coach::Coach::new(&settings.gameplay.coach_muted);
    if let Some(advice) = std::env::var("TA_COACH_SAY").ok().and_then(|n| crate::coach::Advice::named(&n)) {
        coach.say(advice);
    }
    Some(coach)
}

/// Slow cinematic camera for the attract mode.
struct Attract {
    target: Vec3,
    retarget_in: f32,
    /// Seconds of backdrop so far (drives the camera's slow swing).
    clock: f32,
    /// Simulation ticks still to fast-forward so the backdrop shows a developed battle.
    warmup: u32,
    /// The camera goes straight to the fighting next frame (the battle was just fast-forwarded).
    snap: bool,
}

/// Developer overlay state.
struct DevPanel {
    visible: bool,
    fps: f32,
    sun_azimuth: f32,
    sun_elevation: f32,
    /// Per-frame CPU timings (ms): simulation + input, scene preparation, render submit.
    times: FrameTimes,
    /// GPU pass timing wanted, and whether the renderer has it on.
    gpu_timing: bool,
    gpu_timing_on: bool,
    /// Snapshots taken before the UI: GPU particles and GPU time per pass.
    particles: ta_render::gpu_particles::ParticleStats,
    gpu_times: Vec<(&'static str, f32)>,
    /// The frame rate the player's counter shows (Display: FRAME RATE COUNTER).
    meter: FrameMeter,
}

/// The frame rate for the player's counter: frames counted over half a second at a time, so that
/// the figures can be read (a figure a frame flickers), and the frame time their average.
#[derive(Default)]
struct FrameMeter {
    frames: u32,
    seconds: f32,
    /// What is shown: frames a second, and a frame's milliseconds.
    shown: Option<(f32, f32)>,
}

impl FrameMeter {
    const PERIOD: f32 = 0.5;

    /// A frame that took `ms` milliseconds (the wall clock's, whatever a capture's clock says).
    fn tick(&mut self, ms: f32) {
        self.frames += 1;
        self.seconds += ms.clamp(0.0, 1000.0) / 1000.0;
        if self.seconds >= Self::PERIOD {
            self.shown = Some((self.frames as f32 / self.seconds, self.seconds * 1000.0 / self.frames as f32));
            (self.frames, self.seconds) = (0, 0.0);
        }
    }
}

/// Frame timing samples, summarised in the dev panel and when a scripted capture exits.
#[derive(Default)]
struct FrameTimes {
    frame: Vec<f32>,
    update: Vec<f32>,
    prepare: Vec<f32>,
    render: Vec<f32>,
    /// With GPU pass timing on (`--dev-panel`): each pass's milliseconds summed over the frames
    /// measured, and how many, in the order they first came; the frame last summed.
    gpu: Vec<(&'static str, f64, u32)>,
    gpu_frame: u64,
}

impl FrameTimes {
    fn push(&mut self, frame: f32, update: f32, prepare: f32, render: f32) {
        if self.frame.len() >= 600 {
            for v in [&mut self.frame, &mut self.update, &mut self.prepare, &mut self.render] {
                v.drain(..300);
            }
        }
        self.frame.push(frame);
        self.update.push(update);
        self.prepare.push(prepare);
        self.render.push(render);
    }

    /// The GPU's time for each pass of the frame `measured` (once a frame: results come a few
    /// frames late, and each is summed once).
    fn push_gpu(&mut self, measured: u64, passes: &[ta_render::timing::PassTime]) {
        if measured == 0 || measured == self.gpu_frame || passes.is_empty() {
            return;
        }
        self.gpu_frame = measured;
        for p in passes {
            match self.gpu.iter_mut().find(|(pass, ..)| *pass == p.pass) {
                Some((_, sum, n)) => {
                    *sum += p.ms as f64;
                    *n += 1;
                }
                None => self.gpu.push((p.pass, p.ms as f64, 1)),
            }
        }
    }

    fn stats(v: &[f32]) -> (f32, f32) {
        if v.is_empty() {
            return (0.0, 0.0);
        }
        let mut s = v.to_vec();
        s.sort_by(f32::total_cmp);
        (s.iter().sum::<f32>() / s.len() as f32, s[(s.len() * 95 / 100).min(s.len() - 1)])
    }

    fn summary(&self) -> String {
        let f = |name: &str, v: &[f32]| {
            let (avg, p95) = Self::stats(v);
            format!("{name} {avg:.2}/{p95:.2}")
        };
        let mut line = format!(
            "ms avg/p95: {}, {}, {}, {}",
            f("frame", &self.frame),
            f("update", &self.update),
            f("prepare", &self.prepare),
            f("render", &self.render)
        );
        if !self.gpu.is_empty() {
            let passes: Vec<String> = self.gpu.iter().map(|(pass, sum, n)| format!("{pass} {:.3} ({n})", sum / *n as f64)).collect();
            line.push_str(&format!("; gpu ms avg: {}", passes.join(", ")));
        }
        line
    }
}

#[derive(Clone, Debug)]
enum DemoAction {
    /// Ends the battle as won or lost, for a picture of its result (nothing is recorded).
    Outcome(Outcome),
    SelectCommander,
    BuildNear(&'static str, f32, f32),
    Produce(&'static str, &'static str, i32),
    /// Spawns two armies at (x, z) facing each other and sends them in.
    Battle(f32, f32),
    /// Spawns an enemy squad in front of the Commander and D-guns the nearest one.
    DGun,
    /// Sends the Commander to reclaim the nearest reclaimable feature.
    Reclaim,
    /// An area reclaim order around the nearest reclaimable feature.
    AreaReclaim,
    /// Starts placing a metal extractor with the cursor near the nearest metal deposit.
    PlaceExtractor,
    /// Starts placing a building with the cursor at this offset from the Commander.
    PlaceNear(&'static str, f32, f32),
    /// Starts placing a building, Shift held, over the Commander's last planned one.
    PlaceOnPlan(&'static str),
    /// Orders extractors over the metal nearest the Commander (an area of this radius).
    AreaMex(f32),
    /// Replays: jumps to a tick (backwards starts the replay over).
    Seek(u64),
    /// Multiplayer: says something to everyone (false) or to our allies (true).
    Say(&'static str, bool),
    /// Makes the selection control group n.
    Group(usize),
    /// Sets the flammable feature nearest the Commander alight, and looks at it.
    Ignite,
    /// Two huge armies (`TA_SWARM` units per side, default 1500) clash mid-map.
    Swarm,
    /// `--demo spy`: radar planes over the enemy's base, a soak test of memory (`app/spy.rs`).
    Spy,
    /// Thousands of Peewees (`TA_SIEGE`, default 3000) storm a Core defence line in depth
    /// across the middle of the map (a flat one shows it best: Gasplant Plain).
    Siege,
    /// The siege's silo fires at the middle of the line (`TA_SIEGE_NUKE=<frame>`).
    SiegeNuke,
    /// A war across the whole map (`--demo war`, made for Sapphire Reach): armies on land, fleets
    /// at sea, a landing on a fortified shore and an air war over them (`TA_WAR=<scale>`).
    War,
    /// The war's Silencer fires at the middle of the Arm army (`TA_WAR_NUKE=<frame>`).
    WarNuke,
    /// Into the unit of type `TA_RIDE` nearest `TA_RIDE_NEAR=x,z` (px), or else the camera's
    /// focus, at frame `TA_RIDE_AT` (any demo; captures).
    RideNearest,
    /// The radial menu opened on our first unit of type `TA_RADIAL` at frame `TA_RADIAL_AT` (any
    /// demo; captures): `TA_RADIAL_RING=fire|move|more|build|buildings|units|confirm` on that ring,
    /// `TA_RADIAL_HOVER=n` with choice n lit, `TA_RADIAL_HELD=1` as opened by holding.
    OpenRadial,
    /// Into a unit of the siege (`TA_SIEGE_RIDE=arm|core`, at `TA_SIEGE_RIDE_AT=<frame>`): the
    /// Peewee nearest the middle of the line, or the Light Laser Tower there.
    SiegeRide,
    /// A Retaliator nukes an enemy base (`NukeLaunch` fires the prepared silo).
    NukeSetup,
    NukeLaunch,
    /// Opens the pause menu's statistics window.
    ShowStats,
    /// Two small fleets (ships and hovercraft) meet on open water.
    Naval,
    /// Sets the game speed.
    Speed(f32),
    /// Cloaks the local Commander.
    Cloak,
    /// Selects every unit of the local player.
    SelectAll,
    /// Clears the selection.
    Deselect,
    /// Makes player 1 our ally and gives it the selection.
    Give,
    /// Spawns `n` units of a type for a player in a row at (x, z).
    Spawn(&'static str, u8, u32, f32, f32),
    /// Points the cursor at the reclaimable feature nearest the Commander.
    HoverFeature,
    /// Rests the pointer on the Commander (its card).
    HoverUnit,
    /// Tutorial lesson 2: the selection fights its way to the enemy outpost.
    TutorialFight,
    /// A click (press and release) with this mouse button, orders given with that one, on
    /// the Commander (true) or on the ground beside it (false).
    Click(ta_config::OrderButton, MouseButtonId, bool),
    /// Prints what the clicks so far came to (`--demo clicks`): the selection, and whether
    /// the Commander was sent somewhere.
    Check(&'static str),
    /// Spawns `n` units of a type for a player in a row at this offset from our Commander.
    SpawnNear(&'static str, u8, u32, f32, f32),
    /// Ctrl and a click (press and release) with this button, orders given with that one, on
    /// the ground beside the Commander.
    CtrlClick(ta_config::OrderButton, MouseButtonId),
    /// Ctrl and a key.
    CtrlKey(KeyCode),
    /// Lets go of Ctrl.
    ClearMods,
    /// Holds Shift down (to the end of the capture).
    ShiftHold,
    /// Prints the selection (by type), the order waiting for a click, the Commander's fire and
    /// move orders and its first order, and group 3 (`--demo controls`).
    Report(&'static str),
    /// A key pressed this frame (scripted keyboard checks).
    Key(KeyCode),
    /// The interface's pointer to (x, y) in window pixels (`TA_UI_CLICKS`).
    UiMove(f32, f32),
    /// The pointer rests at (x, y) in window pixels, for the interface and the game alike
    /// (`TA_UI_HOVER`).
    UiHover(f32, f32),
    /// The interface's left button pressed (true) or let go (false) at (x, y) in window pixels.
    UiButton(f32, f32, bool),
    /// The mouse to (x, y) in window pixels, for the interface and the game alike, as the
    /// window's own events come: a button pressed (true) or let go (false) there, if one
    /// (`TA_UI_MOUSE`: box selections, orders on the map).
    UiMouse(f32, f32, Option<(MouseButtonId, bool)>),
    /// A key the app itself answers to (F1, F2), pressed at the start of the next frame.
    AppKey(KeyCode),
    /// Text typed into the interface's field with the focus (`TA_UI_TEXT`).
    UiText(&'static str),
    /// A gamepad's button for the interface (`TA_UI_PAD`), as the next reading of the pads.
    UiPad(crate::gamepad::Nav),
    /// Points the camera at (x, z).
    LookAt(f32, f32),
    /// Selects our first unit of this type.
    SelectType(&'static str),
    /// Switches the selected units on or off.
    Switch(bool),
    /// Our units come under attack at this offset from the Commander: the notice and the place
    /// for the alert key, as when a fight begins there.
    Attacked(f32, f32),
    /// Rests the pointer on our first unit of this type.
    HoverType(&'static str),
    /// Rests the pointer on our first unit of this type for the interface as well, so that TA's
    /// cursor is drawn there (`--demo assist`).
    PointAt(&'static str),
    /// A click (press and release) with this button on our first unit of this type, orders
    /// given with that scheme's button.
    ClickOn(ta_config::OrderButton, MouseButtonId, &'static str),
    /// Our units of the first type guard our first unit of the second (assisting it, when it
    /// builds).
    GuardWith(&'static str, &'static str),
    /// Puts a finished unit of this type on the richest metal near our Commander.
    SpawnOnMetal(&'static str),
    /// Prints the selection, the cursor the pointer shows and the Commander's first order
    /// (`--demo assist`).
    Assist(&'static str),
    /// Prints the Commander's fire order, the pending order and the classic side panel's tab.
    KeyReport(&'static str),
    /// Perspective: a unit of ours (`TA_PERSPECTIVE_UNIT`, a Stumpy) facing a few enemies
    /// (`TA_PERSPECTIVE_ENEMY`, Raiders, `TA_PERSPECTIVE_RANGE` px away), and into it; with
    /// `TA_PERSPECTIVE_ORDER=move|patrol|fight|build` it goes on with that order, given before;
    /// `TA_PERSPECTIVE_HULL=0.2` starts it with its hull worn to that share.
    Perspective,
    /// Every unit type on show, a page at a time, finished and switched on, for checking
    /// models and animations: `TA_GALLERY=<page>` (default 0), `TA_GALLERY_SIZE=<units per
    /// page>` (12), `TA_GALLERY_UNITS=a,b,c` for chosen ones; ships go in the sea when the
    /// map has one.
    Gallery,
    /// A staged scene, for filming (`--demo stage`, the scene in `TA_STAGE`; see `stage.rs`).
    Stage,
}

/// Maps that make good backdrops for the menu.
/// The menu's battle is fought on TA Reborn's own big maps (the camera can look to the horizon
/// over them), or failing those on some of TA's.
const ATTRACT_MAPS: [&str; 3] = ["Highland Rift", "Cinder Basin", "Sapphire Reach"];
const ATTRACT_FALLBACK: [&str; 6] = ["Painted Desert", "Sherwood", "Greenhaven", "The Pass", "Lava Run", "Rock Alley"];
const ATTRACT_WARMUP_TICKS: u32 = 30 * 60 * 5;

/// Called only by the loading worker, while the event loop presents the interstitial: the
/// film's first scene put down, or the battle fast-forwarded into its fighting.
fn prepare_attract(game: &mut Game, film: Option<crate::backdrop::Plan>, status: &LoadProgress) -> Option<crate::backdrop::Film> {
    progress(status, ta_ui::i18n::n("Preparing command centre"), "");
    if let Some(plan) = film {
        log::info!("menu film: {}", plan.world());
        return Some(crate::backdrop::Film::begin(plan, game));
    }
    let started = Instant::now();
    game.tally.enabled = false;
    game.fast_forward(ATTRACT_WARMUP_TICKS);
    log::info!("menu backdrop prepared on loading worker in {:.2}s", started.elapsed().as_secs_f64());
    None
}

/// The menu's backdrop: the film of a world (the one after `last`), its first scene's battle;
/// or, with the battle of old chosen (or `TA_ATTRACT_MAP` asking for it, or no world able to
/// play), an AI battle on a map that suits it.
fn backdrop_setup(assets: &Assets, settings: &ta_config::Config, last: Option<&str>, seed: u64) -> (GameSetup, Option<crate::backdrop::Plan>) {
    if settings.gameplay.menu_backdrop == ta_config::MenuBackdrop::Film
        && std::env::var_os("TA_ATTRACT_MAP").is_none()
        && let Some(plan) = crate::backdrop::Plan::choose(assets, ta_ui::seasonal::today(&settings.launcher), last, seed)
    {
        match plan.setup(assets) {
            Ok(setup) => return (setup, Some(plan)),
            Err(e) => log::warn!("menu film: {e:#}"),
        }
    }
    (attract_setup(assets, seed), None)
}

/// The renderer is lent to a worker only while App routes every event and redraw to
/// ActiveLoad. Keeping the slot private makes accidental use outside that lifecycle fail
/// immediately, instead of silently drawing an obsolete battlefield.
struct RenderSlot(Option<Renderer>);
impl std::ops::Deref for RenderSlot {
    type Target = Renderer;
    fn deref(&self) -> &Renderer { self.0.as_ref().expect("renderer belongs to active loading worker") }
}
impl std::ops::DerefMut for RenderSlot {
    fn deref_mut(&mut self) -> &mut Renderer { self.0.as_mut().expect("renderer belongs to active loading worker") }
}

/// Why a battlefield did not load, when the graphics card ran out of memory for it.
const SHORT_OF_MEMORY: &str = ta_ui::i18n::n("Not enough graphics memory for this battlefield. Close other programs that use the graphics card, or choose a lower GRAPHICS LEVEL in Options, then try again.");
/// Why the game closes when the graphics device was lost (to the driver, or to a card out of
/// memory): nothing made with it can be drawn again.
const DEVICE_LOST: &str = ta_ui::i18n::n("The graphics card stopped working (it may have run out of memory), so TA Reborn has to close. Close other programs that use the graphics card, then start it again.");
/// The same, a battle under way saved first.
const DEVICE_LOST_SAVED: &str = ta_ui::i18n::n("The graphics card stopped working (it may have run out of memory), so TA Reborn has to close. Your battle was saved as the autosave: close other programs that use the graphics card, then start TA Reborn again and load it from saved games.");
/// The message's title, on the menu.
const SHORT_OF_MEMORY_TITLE: &str = ta_ui::i18n::n("NOT ENOUGH GRAPHICS MEMORY");
/// Why the game did not start, for the same reason.
const SHORT_OF_MEMORY_AT_START: &str = ta_ui::i18n::n("Not enough graphics memory to start TA Reborn. Close other programs that use the graphics card, then try again.");

/// A load that failed for want of graphics memory: its reason, translated as it is shown.
#[derive(Debug)]
struct ShortOfGraphicsMemory(&'static str);

impl std::fmt::Display for ShortOfGraphicsMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(ta_ui::i18n::t(self.0))
    }
}

impl std::error::Error for ShortOfGraphicsMemory {}

/// A load during which the graphics card refused memory fails cleanly, whatever came of it: what
/// it made then is invalid (drawing with it fails), so a battle goes back to the menu with the
/// reason, and the game's own start stops with it, rather than running on half a battlefield.
fn short_of_graphics_memory(loaded: Result<Loaded>, refused_before: u32) -> Result<Loaded> {
    let refused = ta_render::out_of_memory_events().saturating_sub(refused_before);
    if refused == 0 {
        return loaded;
    }
    log::error!("graphics memory: the card refused {refused} allocations while loading");
    match loaded {
        Ok(Loaded::Battle(renderer, ..) | Loaded::BattleFailed(renderer, _)) => Ok(Loaded::BattleFailed(renderer, ShortOfGraphicsMemory(SHORT_OF_MEMORY).into())),
        Ok(Loaded::BenchmarkPass(next)) => Ok(Loaded::BenchmarkPass(next)),
        Ok(Loaded::Startup(_)) | Err(_) => Err(ShortOfGraphicsMemory(SHORT_OF_MEMORY_AT_START).into()),
    }
}

/// `TA_GPU_BALLAST_MB=<n>`: n MB of the graphics card's memory taken and held before the next
/// battlefield loads (with `TA_GPU_BALLAST_START=1`, before the game's own start too), so that
/// running short of it can be tried on any card, taken once and never given back; `fill[:<m>]`: all
/// the card will give (with `TA_GPU_BUDGET`, up to that share of the memory the system gives the
/// game) but m MB (128), taken on the loading worker as the next battlefield starts to load and
/// given back when the load ends (memory short in the middle of a load, as when another program
/// takes it meanwhile).
static BALLAST: std::sync::Mutex<Option<(wgpu::Device, Vec<wgpu::Buffer>)>> = std::sync::Mutex::new(None);

/// What `fill` took, given back: its buffers dropped and the card waited on, so that the memory is
/// free again before anything else is loaded.
fn give_back_ballast() {
    if let Some((device, held)) = BALLAST.lock().unwrap_or_else(|p| p.into_inner()).take() {
        drop(held);
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        log::warn!("TA_GPU_BALLAST_MB: given back");
    }
}

/// What `TA_GPU_BALLAST_MB` asks for: the MB to take, or (`fill`) the MB to leave.
enum Ballast {
    Take(u64),
    Fill(u64),
}

fn ballast_wanted() -> Option<Ballast> {
    let want = std::env::var("TA_GPU_BALLAST_MB").ok()?;
    match want.strip_prefix("fill") {
        Some(m) => Some(Ballast::Fill(m.trim_start_matches(':').parse::<u64>().unwrap_or(128))),
        None => want.parse::<u64>().ok().map(Ballast::Take),
    }
}

/// The ballast taken now, if it is asked for this load (`fill` on the loading worker, the rest
/// before the transition's check; a battlefield's, or with `TA_GPU_BALLAST_START=1` the start's
/// too) and was not taken yet this run.
fn hold_ballast(device: &wgpu::Device, queue: &wgpu::Queue, battle: bool, on_worker: bool) {
    let Some(wanted) = ballast_wanted() else { return };
    if matches!(wanted, Ballast::Fill(_)) != on_worker {
        return;
    }
    if !battle && std::env::var("TA_GPU_BALLAST_START").map_or(true, |v| v != "1") {
        return;
    }
    static TAKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if TAKEN.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let mut slot = BALLAST.lock().unwrap_or_else(|p| p.into_inner());
    let (_, held) = slot.insert((device.clone(), Vec::new()));
    match wanted {
        Ballast::Take(mb) => *held = ta_render::gpu::take_ballast(device, queue, mb),
        Ballast::Fill(leave) => {
            *held = ta_render::gpu::take_ballast(device, queue, 1 << 16);
            // (The last taken first: the smallest, a MB each.)
            let mut freed = 0;
            while freed < leave && let Some(b) = held.pop() {
                freed += b.size() >> 20;
            }
            // (Given back to the card now, not when it next gets round to it.)
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
        }
    }
    log::warn!("TA_GPU_BALLAST_MB: {} MB of graphics memory held", held.iter().map(|b| b.size() >> 20).sum::<u64>());
}

type LoadProgress = std::sync::mpsc::Sender<(&'static str, String)>;
fn progress(tx: &LoadProgress, status: &'static str, detail: &str) {
    let _ = tx.send((status, detail.to_owned()));
}
struct PreparedStartup {
    renderer: Renderer,
    assets: Assets,
    game: Game,
    benchmark: Option<crate::benchmark::Benchmark>,
    screen: Screen,
    tutorial_lesson: Option<usize>,
    settings: ta_config::Config,
    loaded_stage: Option<crate::stage::StageRun>,
    attract_ready: bool,
    menu_film: Option<crate::backdrop::Film>,
}
enum Loaded {
    Startup(PreparedStartup),
    Battle(Renderer, Game, Option<crate::stage::StageRun>, Option<crate::backdrop::Film>),
    BattleFailed(Renderer, anyhow::Error),
    BenchmarkPass(crate::benchmark::Next),
}
struct ActiveLoad {
    host: crate::loading::LoadingHost,
    window: Arc<Window>,
    shown: bool,
    result: std::sync::mpsc::Receiver<Result<Loaded>>,
    progress: std::sync::mpsc::Receiver<(&'static str, String)>,
    ready: Option<Result<Loaded>>,
    transition: Option<Transition>,
    input: Input,
    keys: crate::keys::Bindings,
    next_frame: Instant,
}

impl App {
    fn start_loading(&mut self, event_loop: &ActiveEventLoop, window: Arc<Window>, gpu: ta_render::Gpu, settings: &ta_config::Config,
        transition: Option<Transition>, title: String, work: impl FnOnce(LoadProgress) -> Result<Loaded> + Send + 'static) -> Result<()> {
        use ta_ui::loading::LoadingKind;
        if let Some(state) = self.state.as_mut() {
            state.audio.reset_scene();
            state.audio.update_idle(0.0);
            // (What the screen showed went with it: no page or dialog of it goes out over the
            // next one.)
            ta_ui::panel::forget_exits(&state.egui.ctx);
        }
        // (The game's own start: a battlefield's or a benchmark pass's is checked before its
        // renderer is lent, in `draw`.)
        if self.state.is_none() {
            hold_ballast(&gpu.device, &gpu.queue, false, false);
            anyhow::ensure!(ta_render::can_allocate(&gpu.device), "{}", ta_ui::i18n::t(SHORT_OF_MEMORY_AT_START));
        }
        let (host_device, host_queue) = (gpu.device.clone(), gpu.queue.clone());
        let benchmark = self.args.benchmark.is_some();
        let mut host = crate::loading::LoadingHost::new(window.clone(), gpu, if benchmark { LoadingKind::Benchmark } else { LoadingKind::Game }, settings.gameplay.ui_scale);
        let adapter = self.state.as_mut().and_then(|s| s.egui.state.accesskit.take());
        host.init_accessibility(event_loop, self.proxy.clone(), adapter, self.accessibility_active);
        host.set_reduced_motion(settings.gameplay.reduced_motion);
        host.set_seasonal(&settings.launcher, settings.gameplay.reduced_flashes);
        if let Some(Transition::Benchmark(n)) = &transition { host.set_hint_offset(*n); }
        let stage = if transition.as_ref().is_some_and(Transition::prepares_menu_backdrop) { ta_ui::i18n::n("Preparing command centre") }
            else if self.state.is_some() { ta_ui::i18n::n("Preparing battlefield") }
            else { ta_ui::i18n::n("Starting TA Reborn") };
        host.set_stage(ta_ui::i18n::t(stage).to_string(), title);
        let _ = window.set_cursor_grab(winit::window::CursorGrabMode::None);
        window.set_cursor_visible(!benchmark);
        // Paint before exposing the native window or starting any expensive work.
        let shown = host.frame()?;
        if shown { window.set_visible(true); }
        let (tx, result) = std::sync::mpsc::channel();
        let (status, progress) = std::sync::mpsc::channel();
        let (ballast_device, ballast_queue, battle) = (host_device.clone(), host_queue.clone(), self.state.is_some());
        std::thread::Builder::new().name("battlefield-loader".into()).spawn(move || {
            let refused = ta_render::out_of_memory_events();
            hold_ballast(&ballast_device, &ballast_queue, battle, true);
            let loaded = work(status);
            let _ = tx.send(short_of_graphics_memory(loaded, refused));
        })?;
        log::info!("loading screen started");
        self.active_load = Some(ActiveLoad { host, window, shown, result, progress, ready: None, transition, input: Input::default(), keys: crate::keys::Bindings::from_config(&settings.keys), next_frame: Instant::now() });
        Ok(())
    }

    fn load_frame(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        use std::sync::mpsc::TryRecvError;
        if let Some(state) = self.state.as_mut() {
            let now = Instant::now();
            let background = !state.focused() && state.screenshot_path.is_none();
            state.audio.set_background(background);
            state.audio.update_idle((now - state.last_frame).as_secs_f32().min(0.1));
            state.last_frame = now;
        }
        let load = self.active_load.as_mut().expect("active load");
        for (status, detail) in load.progress.try_iter() {
            load.host.set_stage(ta_ui::i18n::t(status).to_string(), detail);
        }
        if load.ready.is_none() {
            match load.result.try_recv() {
                Ok(result) => {
                    if matches!(ballast_wanted(), Some(Ballast::Fill(_))) {
                        give_back_ballast();
                    }
                    load.ready = Some(result);
                }
                Err(TryRecvError::Disconnected) => anyhow::bail!(ta_ui::i18n::t("The battlefield loading worker stopped unexpectedly")),
                Err(TryRecvError::Empty) => {}
            }
        }
        load.next_frame = Instant::now() + std::time::Duration::from_millis(33);
        if load.host.frame()? && !load.shown {
            load.window.set_visible(true);
            load.shown = true;
        }
        // A short minimum avoids flashing the interstitial on fast machines. No game
        // frames, simulation ticks or benchmark samples run while this screen is up.
        if load.ready.is_none() || !load.shown || load.host.elapsed() < 0.65 { return Ok(()); }
        // Then the words and the emblem fade off the bay (`LoadingHost::leave`), and what was
        // loaded comes up from under the bay as it lifts (`State::curtain`).
        load.host.leave();
        if !load.host.gone() { return Ok(()); }
        let mut load = self.active_load.take().expect("completed load");
        let adapter = load.host.take_accessibility();
        let loaded = load.ready.take().expect("loaded resources")?;
        // (Not over a capture's first frames, which start as they always have; nor between a
        // benchmark's scenes.)
        let curtain = !(matches!(loaded, Loaded::Startup(_)) && self.args.screenshot.is_some()) && self.args.benchmark.is_none() && !matches!(loaded, Loaded::BenchmarkPass(_));
        match loaded {
            Loaded::Startup(prepared) => {
                let mut state = State::finish(load.window.clone(), &self.args, prepared, self.profile.clone())?;
                // (Straight into a battle: the companion screen as for any battle; captures,
                // `TA_COMPANION=1`, have it over the menus too.)
                if state.screen == Screen::Playing {
                    state.companion_at_battle(event_loop);
                } else if std::env::var("TA_COMPANION").is_ok_and(|v| v == "1") {
                    state.toggle_companion(event_loop);
                }
                self.state = Some(state);
            }
            Loaded::BenchmarkPass(next) => {
                let state = self.state.as_mut().expect("benchmark state");
                let size = state.window.inner_size();
                state.renderer.resize(size.width, size.height);
                state.camera.viewport = Vec2::new(size.width.max(1) as f32, size.height.max(1) as f32);
                state.apply_benchmark_pass(next);
            }
            Loaded::BattleFailed(renderer, error) => {
                let state = self.state.as_mut().expect("battle loading state");
                state.renderer.0 = Some(renderer);
                let size = state.window.inner_size();
                state.renderer.resize(size.width, size.height);
                if error.is::<ShortOfGraphicsMemory>() {
                    state.menu.failure = Some((SHORT_OF_MEMORY_TITLE, format!("{error:#}")));
                } else {
                    state.menu.record_error = Some(format!("{error:#}"));
                }
                state.game.paused = true;
                state.screen = Screen::Menu;
                if let Some(Transition::Play(record)) = &load.transition { state.menu.open_records(record.kind); }
                log::warn!("battle load failed: {error:#}");
                // (Short of graphics memory: what the load made of its battlefield is not to be drawn,
                // and holds memory; the menu's own world is loaded in its place, the page kept,
                // unless loading that is what failed.)
                if error.is::<ShortOfGraphicsMemory>() && !load.transition.as_ref().is_some_and(Transition::prepares_menu_backdrop) {
                    state.pending = Some(Transition::RestoreMenuBackdrop);
                }
            }
            Loaded::Battle(renderer, game, stage, menu_film) => {
                let state = self.state.as_mut().expect("battle loading state");
                state.renderer.0 = Some(renderer);
                let size = state.window.inner_size();
                state.renderer.resize(size.width, size.height);
                state.camera.viewport = Vec2::new(size.width.max(1) as f32, size.height.max(1) as f32);
                state.loaded_game = Some(game);
                state.loaded_stage = stage;
                state.loaded_menu_film = menu_film;
                state.apply_transition(load.transition.take().expect("battle transition"), event_loop)?;
                debug_assert!(state.loaded_game.is_none());
            }
        }
        if let Some(state) = &mut self.state {
            state.input = Input::default();
            // Loading owns input for a while. Discard the previous UI's held buttons,
            // queued events and cached native cursor so neither can leak into the battle.
            state.egui.clock = carry_clock(&state.egui.ctx);
            state.egui.state = egui_winit::State::new(
                state.egui.ctx.clone(), egui::ViewportId::ROOT, &state.window,
                Some(state.window.scale_factor() as f32), None,
                Some(state.renderer.gpu.device.limits().max_texture_dimension_2d as usize),
            );
            state.egui.state.accesskit = adapter;
            if self.accessibility_active { state.egui.ctx.enable_accesskit(); } else { state.egui.ctx.disable_accesskit(); }
            let _ = state.egui.state.on_window_event(&state.window, &WindowEvent::Focused(state.window.has_focus()));
            state.pad = Default::default();
            state.last_frame = Instant::now();
            state.frame_ms = 0.0;
            state.window.set_cursor_visible(state.benchmark.is_none());
            state.curtain = (curtain && !state.settings.gameplay.reduced_motion).then_some(None);
        }
        log::info!("loading screen complete; battlefield ready");
        Ok(())
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        // (The graphics device lost: nothing can be drawn again. A battle under way is saved, its
        // simulation untouched by the card, and the game closes with the reason, not a crash.)
        if ta_render::gpu::device_lost() {
            let saved = self.state.as_mut().is_some_and(State::save_before_closing);
            anyhow::bail!("{}", ta_ui::i18n::t(if saved { DEVICE_LOST_SAVED } else { DEVICE_LOST }));
        }
        if self.active_load.is_some() { return self.load_frame(event_loop); }
        let Some(state) = &mut self.state else { return Ok(()) };
        if let Some(next) = state.pending_pass.take() {
            let detail = match next {
                crate::benchmark::Next::Level(level) => ta_ui::i18n::tf("Settings test · {level}", &[("level", &ta_ui::i18n::t(level.label()))]),
                crate::benchmark::Next::Effect(ta_config::benchmark::Effect::High) => ta_ui::i18n::t("Effects test · High").to_string(),
                crate::benchmark::Next::Effect(e) => ta_ui::i18n::tf("Effects test · {effect} off", &[("effect", &ta_ui::i18n::t(ta_ui::benchmark::effect_label(e)))]),
                _ => ta_ui::i18n::t("Graphics card").to_string(),
            };
            let window = state.window.clone();
            let gpu = state.renderer.gpu.loading_clone();
            let settings = state.settings.clone();
            anyhow::ensure!(ta_render::can_allocate(&gpu.device), "{}", ta_ui::i18n::t(SHORT_OF_MEMORY));
            self.start_loading(event_loop, window, gpu, &settings, None, detail, move |_| Ok(Loaded::BenchmarkPass(next)))?;
            return Ok(());
        }
        if let Some(t) = state.pending.take() {
            let t = if matches!(t, Transition::ToMenu) && state.test_editor.is_some() { Transition::ResumeEditor } else { t };
            let validation = match &t {
                Transition::Play(record) => state.assets.validate_record(record),
                Transition::Rewind(record, _) => state.assets.validate_record(record),
                _ => Ok(false),
            };
            if let Err(error) = validation {
                state.menu.record_error = Some(format!("{error:#}"));
                state.game.paused = true;
                state.screen = Screen::Menu;
                if let Transition::Play(record) = &t { state.menu.open_records(record.kind); }
                return Ok(());
            }
            let menu_backdrop = t.prepares_menu_backdrop();
            let mut film_plan = None;
            let setup = match &t {
                Transition::StartGame(s) | Transition::StartNet(s, _) => Some(s.clone()),
                Transition::StartTutorial(n) => Some(crate::tutorial::setup(*n)),
                Transition::Benchmark(n) => state.benchmark.as_ref().map(|b| b.scene(*n).setup(None, &state.assets)).transpose()?,
                Transition::Play(record) => Some(record.setup.clone()),
                Transition::Rewind(record, _) => Some(record.setup.clone()),
                _ if menu_backdrop => {
                    let (setup, plan) = backdrop_setup(&state.assets, &state.settings, state.menu_film_world, state.game.sim.tick ^ 0x9E37);
                    film_plan = plan;
                    Some(setup)
                }
                _ => None,
            };
            if let Some(setup) = setup {
                // (The card out of memory before the loading screen's interface is made: back to the
                // menu with the reason, as for a record that cannot be played.)
                hold_ballast(&state.renderer.gpu.device, &state.renderer.gpu.queue, true, false);
                if !ta_render::can_allocate(&state.renderer.gpu.device) {
                    if matches!(t, Transition::RestoreMenuBackdrop) {
                        // (The menu's own world, in place of a battlefield that ran short: again in a
                        // moment, the menu drawn meanwhile.)
                        state.backdrop_retry = Some(Instant::now() + std::time::Duration::from_secs(2));
                    } else {
                        state.menu.failure = Some((SHORT_OF_MEMORY_TITLE, ta_ui::i18n::t(SHORT_OF_MEMORY).to_string()));
                        state.game.paused = true;
                        state.screen = Screen::Menu;
                        if let Transition::Play(record) = &t { state.menu.open_records(record.kind); }
                    }
                    return Ok(());
                }
                state.checkpoint_profile(true);
                // No game/network polling runs while the loading host owns the
                // event loop. Leave before that handoff so the other players do
                // not wait for this client's next lockstep tick during warm-up.
                // After a multiplayer battle, back to its lobby: the battle's connections become
                // the lobby's before the warm-up (nothing is polled during it).
                if matches!(t, Transition::ToLobby) && let Some(session) = state.game.net.take() {
                    state.menu.lobby = crate::lobby::NetLobby::after_battle(session, &state.menu.connect.name, &state.assets);
                }
                t.leave_network_for_menu(&mut state.game.net);
                let title = if menu_backdrop { String::new() } else { state.battle_title(&t).unwrap_or_else(|| setup.map.clone()) };
                // Restore a photo's render settings before the worker inherits them.
                if let Some(photo) = state.photo.take() {
                    photo.leave(&mut state.game, &mut state.renderer.post.settings, &mut state.camera);
                }
                let stage = match &t {
                    Transition::Benchmark(n) => state.benchmark.as_ref().map(|b| b.scene(*n).clone()),
                    _ => None,
                };
                let mut renderer = state.renderer.0.take().expect("idle renderer");
                let gpu = renderer.gpu.loading_clone();
                let assets = state.assets.clone();
                let window = state.window.clone();
                let settings = state.settings.clone();
                // (The battle's own name stays under the stage, a mission's or a replay's: not
                // its map's file name in its place a moment later.)
                let named = title.clone();
                self.start_loading(event_loop, window, gpu, &settings, Some(t), title, move |status| {
                    if !menu_backdrop { progress(&status, ta_ui::i18n::n("Preparing battlefield"), &named); }
                    let mut game = match Game::new(&assets, &mut renderer, &setup) {
                        Ok(game) => game,
                        Err(error) => return Ok(Loaded::BattleFailed(renderer, error)),
                    };
                    let menu_film = if menu_backdrop { prepare_attract(&mut game, film_plan, &status) } else { None };
                    let run = stage.as_ref().map(|s| crate::stage::StageRun::begin(s, &mut game));
                    Ok(Loaded::Battle(renderer, game, run, menu_film))
                })?;
                return Ok(());
            }
            state.apply_transition(t, event_loop)?;
            if event_loop.exiting() { return Ok(()); }
        }
        state.frame(event_loop)?;
        if !state.window_shown && state.renderer.gpu.has_presented() && !event_loop.exiting() {
            state.window.set_visible(true);
            state.window_shown = true;
        }
        Ok(())
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(e) = self.draw(event_loop) {
            self.error = Some(e);
            event_loop.exit();
        }
    }
}

impl ApplicationHandler<egui_winit::accesskit_winit::Event> for App {
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: egui_winit::accesskit_winit::Event) {
        use egui_winit::accesskit_winit::WindowEvent as AccessibilityEvent;
        match &event.window_event {
            AccessibilityEvent::InitialTreeRequested => self.accessibility_active = true,
            AccessibilityEvent::AccessibilityDeactivated => self.accessibility_active = false,
            AccessibilityEvent::ActionRequested(_) => {}
        }
        if let Some(load) = &mut self.active_load {
            if event.window_id == load.window.id() { load.host.accessibility_event(event.window_event); }
        } else if let Some(state) = &mut self.state
            && event.window_id == state.window.id()
        {
            crate::accessibility::native_event(&mut state.egui.state, event.window_event);
            state.window.request_redraw();
        }
    }
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() || self.active_load.is_some() { return; }
        let start = (|| -> Result<()> {
            let (window, gpu, settings) = State::begin(event_loop, &self.args, &self.config)?;
            let loader_gpu = gpu.loading_clone();
            let args = self.args.clone();
            let vfs = self.vfs.clone();
            let mods = self.mods.clone();
            let worker_settings = settings.clone();
            self.start_loading(event_loop, window, loader_gpu, &settings, None, String::new(), move |status| {
                State::prepare(args, vfs, mods, worker_settings, gpu, status).map(Loaded::Startup)
            })
        })();
        if let Err(e) = start { self.error = Some(e); event_loop.exit(); }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // A Mac's full screen without the Dock and the menu bar coming over the game at the
        // screen's edges (see macos.rs), whenever the window comes into view, takes the
        // keyboard or changes size (into full screen and out of it); loading or not.
        #[cfg(target_os = "macos")]
        if matches!(event, WindowEvent::Occluded(false) | WindowEvent::Focused(true) | WindowEvent::Resized(_)) {
            let main = self.active_load.as_ref().map(|load| &load.window).or_else(|| self.state.as_ref().map(|state| &state.window));
            if let Some(window) = main.filter(|window| window.id() == id) {
                crate::macos::keep_dock_and_menu_bar_away(window.fullscreen().is_some());
                // (And the pointer the window wants set again: see macos.rs.)
                crate::macos::refresh_cursor(window);
            }
        }
        // The companion screen's window: its own (closing it closes only it).
        if let Some(state) = &mut self.state
            && let Some(companion) = state.companion.as_mut().filter(|c| c.id() == id)
        {
            if companion.event(&event) {
                state.companion = None;
            }
            return;
        }
        if let Some(load) = &mut self.active_load {
            load.input.benchmark_event(&event);
            if matches!(event, WindowEvent::CloseRequested) || load.keys.pressed(&load.input, "quit") {
                // The previous battle still belongs to State until its replacement
                // arrives. Closing during a transition must retain its replay and
                // progress just as closing the normal game window does.
                if let Some(state) = &mut self.state { state.archive_replay(); }
                event_loop.exit();
                return;
            }
            load.input.end_frame();
            load.host.event(&event);
            if matches!(event, WindowEvent::RedrawRequested) { self.redraw(event_loop); }
            return;
        }
        // A capture (`--screenshot`) plays only what its script gives it: the mouse and keys of
        // whoever is at the computer while it runs would turn a ride's view, hover units and
        // buttons, and press keys in the picture. (Scripted clicks and keys come another way.)
        if self.args.screenshot.is_some()
            && matches!(
                event,
                WindowEvent::CursorMoved { .. }
                    | WindowEvent::CursorEntered { .. }
                    | WindowEvent::CursorLeft { .. }
                    | WindowEvent::MouseInput { .. }
                    | WindowEvent::MouseWheel { .. }
                    | WindowEvent::KeyboardInput { .. }
                    | WindowEvent::ModifiersChanged(_)
                    | WindowEvent::Touch(_)
            )
        {
            return;
        }
        let Some(state) = &mut self.state else { return };
        if state.benchmark.is_some() && state.input.benchmark_event(&event) {
            state.window.set_cursor_visible(false);
            return;
        }
        match event {
            WindowEvent::CursorEntered { .. } => state.input.cursor_inside = true,
            WindowEvent::CursorLeft { .. } => state.input.cursor_inside = false,
            WindowEvent::Focused(false) => {
                // Losing focus cancels a drag; it must never look like a release that
                // commits a formation or a row of buildings, even if egui consumes it.
                state.input.on_event(&event, false, false);
                state.game.cancel_pointer_gestures();
            }
            _ => {}
        }
        let response = state.egui.state.on_window_event(&state.window, &event);
        let wants_pointer = state.egui.ctx.egui_wants_pointer_input();
        let wants_keyboard = state.egui.ctx.egui_wants_keyboard_input();
        // egui calls Tab its own always (it moves the keyboard focus); in battle, with no text
        // being typed, Tab is the game's (the players panel; see `battle_keys_only`).
        let tab = matches!(&event, WindowEvent::KeyboardInput { event: k, .. } if k.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Tab));
        let keyboard_pointer_key = state.keyboard_pointer_available() && matches!(&event,
            WindowEvent::KeyboardInput { event: k, .. } if matches!(k.physical_key,
                winit::keyboard::PhysicalKey::Code(code) if
                    state.settings.gameplay.keyboard_pointer && crate::keyboard_pointer::RESERVED.contains(&code)
                    || crate::keys::Binding::parse(state.settings.keys.get("keyboard_pointer")).is_some_and(|b| b.key == code)));
        // A focused non-text widget must not swallow the history shortcut. The
        // Playing update still checks its modifiers and whether history may open.
        let history_key = state.screen == Screen::Playing && !state.egui.ctx.text_edit_focused() && matches!(&event,
            WindowEvent::KeyboardInput { event: k, .. } if matches!(k.physical_key,
                winit::keyboard::PhysicalKey::Code(code) if
                    crate::keys::Binding::parse(state.settings.keys.get("event_history")).is_some_and(|b| b.key == code)));
        let game_key = keyboard_pointer_key || history_key;
        if !response.consumed || (tab && !wants_keyboard && state.screen == Screen::Playing) || game_key {
            state.input.on_event(&event, wants_pointer, wants_keyboard && !game_key);
        } else if let WindowEvent::MouseInput { .. } | WindowEvent::KeyboardInput { .. } = event {
            // Still track releases so buttons never get stuck.
            state.input.on_event(&event, true, true);
        }
        match event {
            WindowEvent::CloseRequested => {
                if state.screen == Screen::Editor && state.editor.as_mut().is_some_and(|editor| !editor.may_quit()) {
                    state.window.request_redraw();
                    return;
                }
                // (A multiplayer battle is asked about only by its host, whose leaving ends it for
                // everyone: a player's goes on without them.)
                let networked = state.game.net.is_some() && !state.game.hosting_others();
                if lifecycle::confirm_window_close(state.screen, networked, state.game.leaving_asks(), &mut state.game.leave_asked) {
                    state.in_game_menu = true;
                    state.window.request_redraw();
                    return;
                }
                state.archive_replay();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                state.renderer.resize(size.width, size.height);
                state.camera.viewport = Vec2::new(size.width.max(1) as f32, size.height.max(1) as f32);
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            // (A picture dropped on the map editor comes into its Import dialog.)
            WindowEvent::DroppedFile(path) if state.screen == Screen::Editor => {
                if let Some(editor) = state.editor.as_mut() {
                    editor.dropped(&path);
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device: winit::event::DeviceId, event: winit::event::DeviceEvent) {
        if self.args.screenshot.is_some() || self.args.benchmark.is_some() || self.active_load.is_some() {
            return;
        }
        if let (Some(state), winit::event::DeviceEvent::MouseMotion { delta }) = (&mut self.state, event)
            && state.window.has_focus()
        {
            state.input.raw_delta += Vec2::new(delta.0 as f32, delta.1 as f32);
            state.input.raw_seen = true;
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(load) = &self.active_load {
            // Bound animation to 30 fps even with benchmark vsync disabled, and poll
            // completion while minimized/occluded (when Windows can suppress redraws).
            if Instant::now() >= load.next_frame { self.redraw(event_loop); }
            if let Some(load) = &self.active_load {
                event_loop.set_control_flow(ControlFlow::WaitUntil(load.next_frame));
            }
            return;
        }
        if let Some(s) = &self.state {
            let capturing = s.screenshot_path.is_some() || s.benchmark.is_some() || s.film.is_some();
            // Minimised: nothing to draw, and Windows sends no redraws, but the battle goes on (a
            // multiplayer game's above all): its frames come from here, thirty a second. (The
            // loop spun a whole core asking for frames.)
            if !capturing && s.window_shown && s.window.is_minimized() == Some(true) {
                if Instant::now() >= s.last_frame + BACKGROUND_FRAME {
                    self.redraw(event_loop);
                }
                if let Some(s) = &self.state {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(s.last_frame + BACKGROUND_FRAME));
                }
                return;
            }
            // A frame rate limit: the player's (vertical sync off), and thirty a second with
            // another window in front, when power is saved in the background.
            let g = &s.settings.graphics;
            let mut gap = (!g.vsync && g.fps_limit > 0).then(|| std::time::Duration::from_secs_f64(1.0 / g.fps_limit as f64));
            if g.save_power_in_background && !s.focused() {
                gap = Some(gap.map_or(BACKGROUND_FRAME, |g| g.max(BACKGROUND_FRAME)));
            }
            if let Some(gap) = gap.filter(|_| !capturing) {
                let due = s.last_frame + gap;
                if Instant::now() < due {
                    event_loop.set_control_flow(ControlFlow::WaitUntil(due));
                    return;
                }
            }
            event_loop.set_control_flow(ControlFlow::Poll);
            if !s.window_shown {
                // Hidden windows do not receive RedrawRequested on Windows. Drive startup
                // frames here, including retries after a skipped/unavailable surface frame.
                self.redraw(event_loop);
            } else {
                s.window.request_redraw();
            }
        }
    }
}

/// The time between frames in the background (minimised, or with another window in front and
/// power saved): thirty a second.
const BACKGROUND_FRAME: std::time::Duration = std::time::Duration::from_micros(33_333);

/// The battle behind a dialog over it (see `App::battle_veiled`): how far it is graded down (as
/// the menus' backdrop, 1), how much it blurs (a fraction of the picture's height), and how dark
/// the dialog's backdrop is then (alpha; [`ta_ui::panel::MODAL_DIM`] otherwise).
const VEIL_GRADE: f32 = 0.7;
const VEIL_BLUR: f32 = 0.006;
const VEIL_DIM: u8 = 96;

/// An all-AI match used as the living backdrop of the menu.
/// Lowest camera pitch (radians above the horizon): low enough to see the horizon and sky.
const MIN_PITCH: f32 = 0.1;
/// The battle camera's closest zoom (pixels a world unit at its focus).
const MAX_ZOOM: f32 = 8.0;
/// The farthest it zooms out on the largest maps, on a window 1,080 pixels high (more pixels
/// a unit on a higher one, so that every screen takes in as much of the map).
const FARTHEST_ZOOM_1080: f32 = 0.08;
/// A film's cinematic camera: how low it looks (as the highlights' does).
const CINEMATIC_PITCH: f32 = 0.72;
/// How far round the line from the eye to a unit followed in Perspective the 3D trees give way
/// (world units).
const SIGHT_RADIUS: f32 = 34.0;

/// What the menu asked of the showroom this frame.
enum ShowroomRequest {
    Show(String, String, f32, crate::showroom::Act),
    Leave,
    Wallpaper(u32, u32, bool),
}

fn attract_setup(assets: &Assets, seed: u64) -> GameSetup {
    let present = |list: &[&'static str]| -> Vec<&'static str> { list.iter().copied().filter(|m| assets.maps.iter().any(|e| e.name.eq_ignore_ascii_case(m))).collect() };
    let mut candidates = present(&ATTRACT_MAPS);
    if candidates.is_empty() {
        candidates = present(&ATTRACT_FALLBACK);
    }
    // (`TA_ATTRACT_MAP` picks one, for captures.)
    if let Ok(m) = std::env::var("TA_ATTRACT_MAP") {
        candidates = vec![Box::leak(m.into_boxed_str())];
    }
    let map = if candidates.is_empty() {
        assets.maps.first().map(|m| m.name.clone()).unwrap_or_else(|| "Coast To Coast".into())
    } else {
        candidates[(seed as usize) % candidates.len()].to_string()
    };
    let starts = assets.maps.iter().find(|m| m.name.eq_ignore_ascii_case(&map)).map_or(2, |m| m.start_positions);
    let n = starts.clamp(2, 4);
    let players = (0..n)
        .map(|i| SlotSetup {
            side: if i % 2 == 0 { "ARM" } else { "CORE" }.into(),
            color: i as u8,
            controller: Controller::Ai(ta_ai::Difficulty::Hard),
            start: i,
            team: i as u8,
            name: String::new(),
        })
        .collect();
    GameSetup { map, players, local_player: None, seed, mission: None, rules: Default::default(), conquest: None, scenario: None }
}

impl State {
    fn begin(event_loop: &ActiveEventLoop, args: &Args, config: &ta_config::Config) -> Result<(Arc<Window>, ta_render::Gpu, ta_config::Config)> {
        // The very first start (the launcher never ran): set up for this computer, its
        // graphics level, the screen's own size and full screen (`Config::set_up_for`). After
        // that the settings are the player's.
        let mut first = config.clone();
        if first.graphics.preset == ta_config::Preset::Auto && args.screenshot.is_none() {
            let hw = ta_config::hardware::Hardware::detect();
            if first.first_start() {
                first.set_up_for(&hw);
                log::info!("first start on {}: {} graphics", hw.summary(), first.graphics.preset.label());
                // The menus in the computer's own language, when there is a translation of it.
                let dirs = ta_ui::i18n::language_dirs(&ta_config::config_dir());
                if first.gameplay.language.is_empty()
                    && std::env::var_os("TA_LANG").is_none()
                    && let Some(code) = ta_config::hardware::system_language().and_then(|l| ta_ui::i18n::closest(&l, &ta_ui::i18n::languages(&dirs)))
                {
                    log::info!("first start: the menus in {code}, as the computer's");
                    first.gameplay.language = code;
                    if let Err(e) = ta_ui::i18n::set_language(&first.gameplay.language, &dirs) {
                        log::warn!("{e}");
                    }
                }
            } else {
                // The level put back to Auto in the options (or still to be picked on Linux): the
                // level alone, where the computer tells it; else from the card, once it is open.
                first.graphics.pick_level_for(&hw);
            }
            if first != *config && let Err(e) = first.save() {
                log::warn!("could not save settings: {e}");
            }
        }
        let config = &first;
        let g = &config.graphics;
        let (w, h) = lifecycle::window_size(args.width, args.height, g);
        let icon = winit::window::Icon::from_rgba(ta_ui::icon::emblem_rgba(64), 64, 64).ok();
        // Shader compilation and asset loading can take seconds. Windows otherwise exposes
        // an unpainted white client area until the first frame is ready.
        let mut attrs = Window::default_attributes().with_title("TA Reborn").with_inner_size(PhysicalSize::new(w, h)).with_window_icon(icon).with_visible(false);
        if args.screenshot.is_none() {
            attrs = match g.window_mode {
                ta_config::WindowMode::Windowed => attrs,
                ta_config::WindowMode::Borderless | ta_config::WindowMode::Fullscreen => {
                    attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)))
                }
            };
        }
        let window = Arc::new(event_loop.create_window(attrs)?);
        let size = window.inner_size();
        // (A benchmark runs with the frame rate unlimited.)
        let gpu = ta_render::Gpu::new(window.clone(), size.width, size.height, g.vsync && args.benchmark.is_none(), ray_tracing_wanted(g))?;
        // The first start picks the graphics level for this card (and notes whether it traces
        // rays, for the options).
        let mut settings = config.clone();
        let traces = gpu.adapter.features().contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY);
        let mut noted = settings.graphics.gpu_ray_tracing != traces;
        settings.graphics.gpu_ray_tracing = traces;
        let info = gpu.adapter.get_info();
        // (The screen's own pixels in full screen: a Mac's window takes the screen's size only
        // once its move into full screen has played, after this.)
        let native = window.current_monitor().or_else(|| window.primary_monitor()).map(|m| (m.size().width, m.size().height));
        let pixels = settings.graphics.picture_pixels(native.or(Some((size.width, size.height))));
        // Once: the level's render scale for this screen and graphics (a Mac's Retina screen has
        // more pixels than its graphics draw the battlefield at in time; the launcher picked the
        // level before the game had a window, and on a Mac without knowing the screen).
        if args.screenshot.is_none() {
            let before = settings.graphics.render_scale;
            if settings.graphics.fit_render_scale_once(gpu_kind(info.device_type), &info.name, pixels) {
                noted = true;
                if settings.graphics.render_scale != before {
                    log::info!("render scale {:.0}% for {pixels} pixels on {}", settings.graphics.render_scale * 100.0, info.name);
                }
            }
        }
        if (auto_graphics(&mut settings.graphics, &info, pixels) || noted) && args.screenshot.is_none() && let Err(e) = settings.save() {
            log::warn!("could not save settings: {e}");
        }
        Ok((window, gpu, settings))
    }

    fn prepare(args: Args, vfs: Arc<Vfs>, mods: crate::record_mods::InPlay, settings: ta_config::Config, gpu: ta_render::Gpu, status: LoadProgress) -> Result<PreparedStartup> {
        let size = PhysicalSize::new(gpu.config.width, gpu.config.height);
        let g = &settings.graphics;
        progress(&status, ta_ui::i18n::n("Preparing graphics"), "");
        let mut renderer = Renderer::with_gpu(gpu, size.width, size.height, ta_render::RendererOptions { msaa: g.msaa })?;
        renderer.set_render_scale(g.render_scale);
        renderer.set_art_style(art_style_reborn(g));
        log::info!("art style: {}", if art_style_reborn(g) { "reborn" } else { "classic" });
        renderer.lighting.terrain_relight = g.terrain_relight;
        apply_effect_settings(g, &mut renderer.lighting, &mut renderer.post.settings);
        progress(&status, ta_ui::i18n::n("Loading units and textures"), "");
        // Skins are presentation-only and never enter the VFS, simulation data or
        // multiplayer fingerprint. Benchmarks keep the player's selected paint as well.
        let load_skin = |id: &str, side: &str| match ta_skins::load(id, &ta_config::config_dir().join("skins")) {
            Ok(pack) => { log::info!("{side} unit skin: {}", pack.manifest.id); Some(pack) }
            Err(e) => { log::warn!("{side} unit skin {id:?}: {e}; using Reborn"); None }
        };
        // (`TA_SKIN=<id>`, and `TA_ENEMY_SKIN=<id>` for the enemy, else the same: the skins a
        // capture is drawn with, whatever the settings say.)
        let own_id = std::env::var("TA_SKIN").unwrap_or_else(|_| g.unit_skin.clone());
        let enemy_id = std::env::var("TA_ENEMY_SKIN").unwrap_or_else(|_| if std::env::var("TA_SKIN").is_ok() { own_id.clone() } else { g.enemy_skin().to_string() });
        let skin = load_skin(&own_id, "your");
        let same_skin = enemy_id == own_id;
        let enemy_skin = if same_skin { None } else { load_skin(&enemy_id, "enemy") };
        let assets = Assets::load(vfs.clone(), mods, &mut renderer, skin.as_ref(), if same_skin { skin.as_ref() } else { enemy_skin.as_ref() }, &settings.maps)?;
        drop((skin, enemy_skin)); // Decoded pixel copies are needed only while uploading.
        // `--benchmark`: its scenes (what is not installed of them left out), the memory measured.
        let benchmark = match &args.benchmark {
            Some(out) => {
                let mode = ta_config::benchmark::Mode::from_name(&args.benchmark_mode).unwrap_or_default();
                let traced = ray_tracing_wanted(&settings.graphics) && renderer.ray_tracing_available();
                let mut b = crate::benchmark::Benchmark::new(out.clone(), mode, traced, |kind| assets.types.id(kind).is_some())?;
                b.measure_memory();
                Some(b)
            }
            None => None,
        };
        // Direct launch (`--map`) skips the front end; otherwise intro then menu.
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_secs());
        // `--mission "<campaign>:<index>"` starts a campaign mission directly.
        let mission_setup = args.mission.as_deref().and_then(|spec| {
            let (name, index) = spec.rsplit_once(':').unwrap_or((spec, "0"));
            let campaign = assets.campaigns.iter().find(|c| c.id.eq_ignore_ascii_case(name.trim()) || c.title.eq_ignore_ascii_case(name.trim()))?;
            let saved = ta_config::CampaignProgress::load().difficulty;
            let difficulty = if saved.is_empty() { "Medium".to_string() } else { saved };
            GameSetup::mission(&assets, campaign, index.trim().parse().unwrap_or(0), &difficulty)
        });
        let tutorial_lesson = args.tutorial.map(|n| n.clamp(1, crate::tutorial::LESSONS.len()) - 1);
        let mission_setup = mission_setup.or(tutorial_lesson.map(crate::tutorial::setup));
        let mission_setup = mission_setup.or_else(|| args.scenario.as_deref().and_then(|id| crate::scenario::setup_by_id(&assets, id, crate::scenario::difficulty_from_env(), seed)));
        // `--demo stage`: the scene's battle (on `--map`, or the map it names).
        // `--benchmark`: its first scene.
        let mission_setup = match (&benchmark, mission_setup) {
            (Some(b), None) => Some(b.stage().setup(None, &assets)?),
            (_, setup) => setup,
        };
        let staged = crate::stage::current().filter(|_| args.demo.as_deref() == Some("stage"));
        let mission_setup = match (mission_setup, staged) {
            (None, Some(stage)) => Some(stage.setup(args.map.as_deref(), &assets)?),
            (setup, _) => setup,
        };
        let mut film_plan = None;
        let (setup, screen) = match (&args.map, mission_setup) {
            (_, Some(setup)) => (setup, Screen::Playing),
            (Some(map), None) => {
                let mut duel = GameSetup::duel(map);
                duel.rules.unit_limit = settings.gameplay.unit_limit;
                // Captures: `TA_TRUE_SIGHT=1` plays it with TA's "true" line of sight;
                // `TA_MUTATORS=no_rush=5,resources=8` with a skirmish's mutators.
                duel.rules.true_sight = std::env::var_os("TA_TRUE_SIGHT").is_some();
                if let Ok(spec) = std::env::var("TA_MUTATORS") {
                    duel.rules.mutators = ta_sim::Mutators::parse(&spec);
                }
                // Captures: `TA_RESULT_MINUTES=n` (with `--open victory|defeat`): the player's side
                // played by a computer player too, its first n minutes before the result.
                // (`--demo spy` too: both sides played, see `app/spy.rs`.)
                if (std::env::var_os("TA_RESULT_MINUTES").is_some() || args.demo.as_deref() == Some("spy"))
                    && let Some(me) = duel.players.first_mut()
                {
                    me.controller = crate::setup::Controller::Ai(ta_ai::Difficulty::Hard);
                }
                (duel, Screen::Playing)
            }
            (None, None) => {
                // (Straight in with a replay or saved game to open, too.)
                let skip = args.skip_intro || args.screenshot.is_some() || args.host.is_some() || args.join.is_some() || args.load.is_some() || args.replay.is_some();
                let (setup, plan) = backdrop_setup(&assets, &settings, None, seed);
                film_plan = plan;
                (setup, if skip { Screen::Menu } else { Screen::Intro })
            }
        };
        // Captures: `TA_MOVEMENT=ta` plays a battle or a staged scene with TA's movement (no
        // formations; units queue rather than give way).
        let mut setup = setup;
        if std::env::var("TA_MOVEMENT").is_ok_and(|v| v == "ta") {
            setup.rules.give_way = false;
        }
        progress(&status, ta_ui::i18n::n("Preparing battlefield"), &setup.map);
        let mut game = Game::new(&assets, &mut renderer, &setup)?;
        if screen == Screen::Playing {
            game.speed = lifecycle::starting_speed(settings.gameplay.speed, setup.local_player, benchmark.is_some() || staged.is_some());
            meet_the_enemy(&mut game, &settings, crate::profile_progress::ordinary_run(&args));
        }

        let loaded_stage = benchmark.as_ref().map(|b| b.stage()).or(staged)
            .map(|stage| crate::stage::StageRun::begin(stage, &mut game));
        let attract_ready = matches!(screen, Screen::Menu | Screen::Intro) && game.spectator;
        let menu_film = if attract_ready { prepare_attract(&mut game, film_plan, &status) } else { None };
        Ok(PreparedStartup { renderer, assets, game, benchmark, screen, tutorial_lesson, settings, loaded_stage, attract_ready, menu_film })
    }

    fn finish(window: Arc<Window>, args: &Args, prepared: PreparedStartup, profile: Option<ta_config::profiles::Profile>) -> Result<Self> {
        let PreparedStartup { mut renderer, assets, game, benchmark, screen, tutorial_lesson, settings, loaded_stage, attract_ready, menu_film } = prepared;
        if let Some(f) = &menu_film {
            renderer.lighting.moon_dir = f.moon_dir();
        }
        let initial_config = settings.clone();
        let config = &initial_config;
        let g = &settings.graphics;
        let msaa_started = g.msaa;
        let size = window.inner_size();
        renderer.resize(size.width, size.height);
        let staged = crate::stage::current().filter(|_| args.demo.as_deref() == Some("stage"));
        let mut audio = GameAudio::new(assets.vfs.clone(), &config.audio);
        audio.set_user_music(&ta_config::music_files(&config.audio), config.audio.user_music);
        let mut camera = Camera::new(Vec2::new(size.width as f32, size.height as f32));
        camera.zoom = 1.6;
        if let Some(c) = game.local_commander().filter(|_| !game.spectator) {
            crate::hud::focus_on(&mut camera, &game, c);
            camera.focus.z += 60.0;
        } else if let Some(p) = game.start_focus.filter(|_| !game.spectator) {
            camera.focus = Vec3::new(p.x, game.sim.world.sea_level.to_f32(), p.z);
            camera.pitch = ta_render::camera::TA_ELEVATION;
        } else {
            let p = game.map.width_px() as f32 * 0.5;
            camera.focus = Vec3::new(p, 0.0, game.map.height_px() as f32 * 0.5);
            camera.zoom = 1.1;
            camera.pitch = ATTRACT_PITCH;
        }
        // A battle started straight away is seen the way the player chose (`--classic`: TA's
        // view); the menu's backdrop always with the modern camera, drifting over it.
        // (A staged scene is filmed with the modern camera, which turns and tilts.)
        if screen == Screen::Playing || args.classic {
            let style = if args.classic {
                ta_config::CameraStyle::Classic
            } else if staged.is_some() || benchmark.is_some() {
                ta_config::CameraStyle::Modern
            } else {
                g.camera
            };
            set_camera_style(&mut camera, style);
        }
        if let Some(spec) = &args.camera {
            apply_camera_spec(&mut camera, spec);
        }

        let ctx = egui::Context::default();
        crate::theme::install_compact(&ctx);
        ta_ui::sound::enable(&ctx, true);
        let egui_state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(renderer.gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = crate::loading::ui_renderer(
            &renderer.gpu.device,
            renderer.gpu.config.format,
            egui_wgpu::RendererOptions { msaa_samples: 1, depth_stencil_format: None, dithering: true, ..Default::default() },
        )
        .ok_or(ShortOfGraphicsMemory(SHORT_OF_MEMORY_AT_START))?;
        let sun = renderer.lighting.sun_dir.normalize();
        let script = match args.demo.as_deref() {
            Some("build") => vec![
                (5, DemoAction::SelectCommander),
                (10, DemoAction::BuildNear("ARMSOLAR", 120.0, 40.0)),
                (12, DemoAction::BuildNear("ARMLAB", -60.0, 150.0)),
            ],
            Some("battle") => vec![(2, DemoAction::Battle(2750.0, 800.0))],
            Some("naval") => vec![(2, DemoAction::Naval)],
            Some("cloak") => vec![(5, DemoAction::SelectCommander), (6, DemoAction::Cloak)],
            // Selecting and ordering with either button giving orders (prints what happened).
            Some("clicks") => {
                use ta_config::OrderButton::{Left, Right};
                let (l, r) = (MouseButtonId::Left, MouseButtonId::Right);
                vec![
                    (20, DemoAction::Click(Left, l, true)),
                    (21, DemoAction::Check("left orders: click the Commander -> selected")),
                    (22, DemoAction::Click(Left, l, false)),
                    (40, DemoAction::Check("left orders: click the ground -> it moves")),
                    (41, DemoAction::Click(Left, r, false)),
                    (42, DemoAction::Check("left orders: right-click -> nothing selected")),
                    (50, DemoAction::Click(Right, l, true)),
                    (51, DemoAction::Check("right orders: click the Commander -> selected")),
                    (52, DemoAction::Click(Right, r, false)),
                    (70, DemoAction::Check("right orders: right-click the ground -> it moves")),
                    (71, DemoAction::Click(Right, l, false)),
                    (72, DemoAction::Check("right orders: click the ground -> nothing selected")),
                    // The radial menu (the frames between give it time to fade out): where the
                    // right click would not give an order it opens the unit's menu.
                    (80, DemoAction::Click(Right, r, true)),
                    (81, DemoAction::Check("right orders: right-click the Commander, nothing selected -> its menu")),
                    (84, DemoAction::Key(KeyCode::Escape)),
                    (120, DemoAction::Check("right orders: Escape -> the menu is gone")),
                    (121, DemoAction::Click(Right, l, true)),
                    (122, DemoAction::Check("right orders: click the Commander -> selected, no menu")),
                    (123, DemoAction::Click(Right, r, true)),
                    (124, DemoAction::Check("right orders: right-click the selected Commander -> its menu (no move)")),
                    (127, DemoAction::Key(KeyCode::Escape)),
                    (170, DemoAction::Click(Left, r, true)),
                    (171, DemoAction::Check("left orders: right-click the Commander -> its menu (the click only cleared the selection before)")),
                    (174, DemoAction::Key(KeyCode::Escape)),
                    (210, DemoAction::Click(Left, r, false)),
                    (211, DemoAction::Check("left orders: right-click the ground -> nothing selected, no menu")),
                ]
            }
            Some("army") => vec![(2, DemoAction::Battle(2750.0, 800.0)), (4, DemoAction::SelectAll), (5, DemoAction::Group(1))],
            // TA's keys and Ctrl+click, played in and reported: force fire, the selections by
            // category, the order and standing-order keys, a factory's squad.
            Some("controls") => {
                use ta_config::OrderButton::{Left, Right};
                let (l, r) = (MouseButtonId::Left, MouseButtonId::Right);
                let mut steps = vec![
                    (3, DemoAction::SpawnNear("ARMCV", 0, 2, 120.0, 60.0)),
                    (3, DemoAction::SpawnNear("ARMLAB", 0, 1, -200.0, 0.0)),
                    (3, DemoAction::SpawnNear("ARMSTUMP", 0, 3, 160.0, -80.0)),
                    (3, DemoAction::SpawnNear("ARMFIG", 0, 2, 0.0, 200.0)),
                    (3, DemoAction::SpawnNear("ARMPEEP", 0, 1, 60.0, 200.0)),
                    (3, DemoAction::SpawnNear("ARMRAD", 0, 1, -120.0, 160.0)),
                    (3, DemoAction::SpawnNear("ARMMSTOR", 0, 1, -120.0, -160.0)),
                ];
                // Each step, then its report a few ticks later.
                let at = std::cell::Cell::new(10u32);
                let step = |steps: &mut Vec<(u32, DemoAction)>, actions: Vec<DemoAction>, what: &'static str| {
                    for a in actions {
                        steps.push((at.get(), a));
                        at.set(at.get() + 1);
                    }
                    at.set(at.get() + 14);
                    steps.push((at.get(), DemoAction::Report(what)));
                    at.set(at.get() + 2);
                };
                step(&mut steps, vec![DemoAction::SelectCommander, DemoAction::CtrlClick(Right, r), DemoAction::ClearMods], "right orders: Ctrl+right-click the ground");
                step(&mut steps, vec![DemoAction::SelectCommander, DemoAction::CtrlClick(Left, l), DemoAction::ClearMods], "left orders: Ctrl+click the ground");
                for (key, what) in [
                    (KeyCode::KeyB, "Ctrl+B"),
                    (KeyCode::KeyF, "Ctrl+F"),
                    (KeyCode::KeyW, "Ctrl+W"),
                    (KeyCode::KeyV, "Ctrl+V"),
                    (KeyCode::KeyP, "Ctrl+P"),
                    (KeyCode::KeyR, "Ctrl+R"),
                    (KeyCode::KeyS, "Ctrl+S"),
                    (KeyCode::KeyM, "Ctrl+M (no mines)"),
                ] {
                    step(&mut steps, vec![DemoAction::CtrlKey(key), DemoAction::ClearMods], what);
                }
                step(&mut steps, vec![DemoAction::SelectCommander, DemoAction::Key(KeyCode::KeyF)], "F");
                step(&mut steps, vec![DemoAction::Key(KeyCode::KeyV)], "V");
                step(&mut steps, vec![DemoAction::Key(KeyCode::KeyM)], "M");
                step(&mut steps, vec![DemoAction::Key(KeyCode::KeyR)], "R");
                step(&mut steps, vec![DemoAction::Key(KeyCode::Escape), DemoAction::CtrlKey(KeyCode::KeyF), DemoAction::ClearMods, DemoAction::CtrlKey(KeyCode::Digit3), DemoAction::ClearMods, DemoAction::Produce("ARMLAB", "ARMPW", 1), DemoAction::Speed(10.0)], "factory in group 3 (just made)");
                steps.push((at.get() + 250, DemoAction::Report("factory in group 3, a Peewee built")));
                // TA's N and function keys (after the factory's Peewee).
                at.set(at.get() + 260);
                step(&mut steps, vec![DemoAction::Speed(1.0), DemoAction::Deselect, DemoAction::LookAt(60.0, 60.0)], "camera in a corner, nothing selected");
                step(&mut steps, vec![DemoAction::Key(KeyCode::KeyN)], "N: a unit of ours off the screen");
                step(&mut steps, vec![DemoAction::Key(KeyCode::KeyN)], "N again (the rest are on the screen now)");
                step(&mut steps, vec![DemoAction::SelectCommander, DemoAction::AppKey(KeyCode::F1)], "F1 on the Commander");
                step(&mut steps, vec![DemoAction::Key(KeyCode::Escape)], "Escape closes it, the Commander stays selected");
                step(&mut steps, vec![DemoAction::Say("hello", false), DemoAction::Key(KeyCode::Delete)], "Delete: the chat cleared");
                step(&mut steps, vec![DemoAction::Deselect, DemoAction::AppKey(KeyCode::F2)], "F2: the menu");
                step(&mut steps, vec![DemoAction::AppKey(KeyCode::F2)], "F2 again: closed");
                steps
            }
            // With --classic-ui: TA's own keys on the orders page (F cycles the fire orders, as
            // it does in the modern HUD); prints what happened.
            Some("classic-keys") => vec![
                (5, DemoAction::SelectCommander),
                (40, DemoAction::KeyReport("before")),
                (41, DemoAction::Key(KeyCode::KeyO)),
                (50, DemoAction::Key(KeyCode::KeyF)),
                (60, DemoAction::KeyReport("after F")),
                (61, DemoAction::Key(KeyCode::KeyA)),
                (70, DemoAction::KeyReport("after A")),
            ],
            // An order given with a click: how it is acknowledged (`TA_SHOTS` films it).
            Some("orders") => {
                use ta_config::OrderButton::Right;
                vec![(5, DemoAction::SelectCommander), (120, DemoAction::Click(Right, MouseButtonId::Right, false))]
            }
            Some("give") => vec![(2, DemoAction::Battle(2750.0, 800.0)), (4, DemoAction::SelectAll), (60, DemoAction::Give)],
            // Spiders' paralyzers stun the Core line.
            Some("stun") => vec![(2, DemoAction::Spawn("CORTHUD", 1, 4, 2840.0, 740.0)), (3, DemoAction::Spawn("ARMSPID", 0, 4, 2680.0, 740.0))],
            Some("dgun") => vec![(5, DemoAction::SelectCommander), (8, DemoAction::DGun)],
            Some("reclaim") => vec![(5, DemoAction::SelectCommander), (8, DemoAction::Reclaim)],
            Some("hover") => vec![(5, DemoAction::HoverFeature)],
            Some("hovercard") => vec![(10, DemoAction::HoverUnit)],
            // A sensor's reach (`TA_RADAR_UNIT=ARMSONAR|ARMJAM|ARMSEER...`, a radar tower by default):
            // selected, then pointed at with the Commander selected, then another being placed
            // (`TA_SHOTS=3 TA_SHOT_STEP=55 TA_CAPTURE_FPS=30`, the first at `--frames 30`).
            Some("radar") => {
                let unit: &'static str = std::env::var("TA_RADAR_UNIT").map(|s| &*Box::leak(s.to_ascii_uppercase().into_boxed_str())).unwrap_or("ARMRAD");
                let mut script = vec![
                    (3, DemoAction::SpawnNear(unit, 0, 1, 0.0, 190.0)),
                    (5, DemoAction::SelectType(unit)),
                    (60, DemoAction::SelectCommander),
                    (61, DemoAction::PointAt(unit)),
                    (112, DemoAction::PlaceNear(unit, 200.0, -30.0)),
                ];
                // (`TA_RADAR_OFF=1`: switched off from the start, to see how it says so.)
                if std::env::var_os("TA_RADAR_OFF").is_some() {
                    script.push((6, DemoAction::Switch(false)));
                }
                script
            }
            // Notices and advice that point somewhere, to click: units under attack far off (a notice
            // with LOOK on it), and with `TA_COACH=1 TA_COACH_SAY=factory_idle` (or `metal_dry`...) the
            // coach's advice about the idle lab. `TA_UI_CLICKS` presses the chips, `TA_HINT_AT=dx,dz`
            // puts the attack elsewhere (`TA_SHOTS=3 TA_SHOT_STEP=40 TA_CAPTURE_FPS=30`).
            Some("hints") => {
                let (dx, dz) = std::env::var("TA_HINT_AT").ok().and_then(|v| v.split_once(',').and_then(|(x, z)| Some((x.trim().parse::<f32>().ok()?, z.trim().parse::<f32>().ok()?)))).unwrap_or((900.0, 600.0));
                vec![
                    (3, DemoAction::SpawnNear("ARMLAB", 0, 1, -240.0, 120.0)),
                    (3, DemoAction::SpawnNear("ARMCK", 0, 1, 90.0, 140.0)),
                    (20, DemoAction::Attacked(dx, dz)),
                ]
            }
            // The Commander and two constructors guarding it build a lab: the time left over it,
            // growing shorter as they help (`TA_SHOTS=6 TA_SHOT_STEP=40 TA_CAPTURE_FPS=30` films it).
            Some("eta") => vec![
                (3, DemoAction::SpawnNear("ARMCK", 0, 2, 40.0, 150.0)),
                (4, DemoAction::SelectCommander),
                (10, DemoAction::BuildNear("ARMLAB", 140.0, 60.0)),
                (130, DemoAction::GuardWith("ARMCK", "ARMCOM")),
            ],
            // Buildings of ours, each under the pointer in turn (`TA_SHOTS=6 TA_SHOT_STEP=45
            // TA_CAPTURE_FPS=30` takes the cards): what each makes and uses of metal and energy.
            Some("figures") => {
                let mut script = vec![
                    (3, DemoAction::SpawnOnMetal("ARMMEX")),
                    (3, DemoAction::SpawnNear("ARMSOLAR", 0, 1, -150.0, 130.0)),
                    (3, DemoAction::SpawnNear("ARMWIN", 0, 1, -90.0, 130.0)),
                    (3, DemoAction::SpawnNear("ARMMMKR", 0, 1, -30.0, 130.0)),
                    (3, DemoAction::SpawnNear("ARMRAD", 0, 1, 30.0, 130.0)),
                    (3, DemoAction::SpawnNear("ARMMSTOR", 0, 1, 90.0, 130.0)),
                    (3, DemoAction::SpawnNear("ARMLAB", 0, 1, 190.0, 130.0)),
                    (6, DemoAction::Produce("ARMLAB", "ARMPW", 4)),
                    (7, DemoAction::SelectCommander),
                ];
                for (k, name) in ["ARMMEX", "ARMSOLAR", "ARMWIN", "ARMMMKR", "ARMRAD", "ARMMSTOR", "ARMLAB"].into_iter().enumerate() {
                    script.push((60 + 50 * k as u32, DemoAction::PointAt(name)));
                }
                script
            }
            // A constructor guarding a factory at work, and the Commander over that factory: the
            // cursor says it will help (the wrench), the click helps, with either button giving
            // orders (`TA_ASSIST_LEFT=1`); prints what the cursor was and what the click did.
            Some("assist") => {
                use ta_config::OrderButton::{Left, Right};
                let (scheme, button) = if std::env::var_os("TA_ASSIST_LEFT").is_some() { (Left, MouseButtonId::Left) } else { (Right, MouseButtonId::Right) };
                vec![
                    (3, DemoAction::SpawnNear("ARMLAB", 0, 1, 150.0, 130.0)),
                    (3, DemoAction::SpawnNear("ARMCK", 0, 1, 70.0, 130.0)),
                    (6, DemoAction::Produce("ARMLAB", "ARMPW", 4)),
                    (8, DemoAction::GuardWith("ARMCK", "ARMLAB")),
                    (120, DemoAction::SelectCommander),
                    (121, DemoAction::PointAt("ARMLAB")),
                    (124, DemoAction::Assist("the Commander over a lab at work")),
                    (126, DemoAction::ClickOn(scheme, button, "ARMLAB")),
                    (140, DemoAction::Assist("after the click")),
                ]
            }
            // Shift over a mine (a nuclear one): its trigger circle and its blast's.
            Some("mine") => vec![(3, DemoAction::SpawnNear("ARMMINE6", 0, 1, 110.0, 30.0)), (20, DemoAction::HoverType("ARMMINE6")), (22, DemoAction::ShiftHold)],
            // TA's F1 on a unit of ours (`TA_INFO_UNIT`, the Commander by default).
            Some("info") => {
                let unit: &'static str = std::env::var("TA_INFO_UNIT").map(|s| &*Box::leak(s.to_ascii_uppercase().into_boxed_str())).unwrap_or("ARMCOM");
                let mut steps = vec![(8, DemoAction::SelectType(unit)), (12, DemoAction::AppKey(KeyCode::F1))];
                if unit != "ARMCOM" {
                    steps.insert(0, (3, DemoAction::SpawnNear(unit, 0, 1, 90.0, 40.0)));
                }
                steps
            }
            // Shift over the Commander (it cloaks): the circle an enemy uncovers it in, and with
            // a plan queued, the constructors' plans.
            Some("shift-hover") => vec![(5, DemoAction::SelectCommander), (8, DemoAction::BuildNear("ARMSOLAR", 160.0, 80.0)), (8, DemoAction::BuildNear("ARMWIN", -160.0, 120.0)), (8, DemoAction::BuildNear("ARMMSTOR", 40.0, 220.0)), (10, DemoAction::Deselect), (12, DemoAction::HoverUnit), (13, DemoAction::ShiftHold), (57, DemoAction::HoverUnit), (58, DemoAction::HoverUnit)],
            Some("area") => vec![(5, DemoAction::SelectCommander), (8, DemoAction::AreaReclaim)],
            Some("metal") => vec![(5, DemoAction::SelectCommander), (8, DemoAction::PlaceExtractor)],
            // With --replay: some seconds in, jump back to the start.
            Some("rewind") => vec![(1500, DemoAction::Seek(60))],
            Some("fire") => vec![(5, DemoAction::Ignite)],
            // Multiplayer: one line to everyone, one to allies only.
            Some("chat") => vec![(1500, DemoAction::Say("hello everyone", false)), (1510, DemoAction::Say("our secret plan", true))],
            // A building held over the ground, with its ranges and what it would make and use
            // (`TA_RANGES_UNIT=ARMRAD`: another; the Light Laser Tower by default).
            Some("ranges") => {
                let unit: &'static str = std::env::var("TA_RANGES_UNIT").map(|s| &*Box::leak(s.to_ascii_uppercase().into_boxed_str())).unwrap_or("ARMLLT");
                vec![(5, DemoAction::SelectCommander), (8, DemoAction::PlaceNear(unit, 160.0, 60.0))]
            }
            Some("areamex") => vec![(5, DemoAction::SelectCommander), (8, DemoAction::AreaMex(420.0))],
            // Two solars planned, then a wind generator held over the second (it must not fit).
            Some("plans") => vec![
                (5, DemoAction::SelectCommander),
                (10, DemoAction::BuildNear("ARMSOLAR", 120.0, 40.0)),
                (11, DemoAction::BuildNear("ARMSOLAR", 260.0, 40.0)),
                (60, DemoAction::PlaceOnPlan("ARMWIN")),
            ],
            // TA's squares round the plans: Shift over the Commander with plans queued.
            Some("plan-squares") => vec![
                (5, DemoAction::SelectCommander),
                (10, DemoAction::BuildNear("ARMSOLAR", 120.0, 40.0)),
                (11, DemoAction::BuildNear("ARMSOLAR", 260.0, 40.0)),
                (12, DemoAction::BuildNear("ARMSOLAR", 260.0, 180.0)),
                (14, DemoAction::HoverUnit),
                (15, DemoAction::ShiftHold),
            ],
            Some("swarm") => vec![(2, DemoAction::Swarm)],
            // A skirmish both sides of which the computer plays, fast-forwarded, then radar planes
            // of ours over the enemy's base, the memory logged (`app/spy.rs`).
            Some("spy") => vec![(2, DemoAction::Spy)],
            Some("war") => {
                let mut v = vec![(2, DemoAction::War)];
                if let Some(f) = std::env::var("TA_WAR_NUKE").ok().and_then(|v| v.parse::<u32>().ok()) {
                    v.push((f.max(3), DemoAction::WarNuke));
                }
                v
            }
            Some("siege") => {
                let frame = |key: &str| std::env::var(key).ok().and_then(|v| v.parse::<u32>().ok());
                let mut v = vec![(2, DemoAction::Siege)];
                if let Some(f) = frame("TA_SIEGE_NUKE") {
                    v.push((f.max(3), DemoAction::SiegeNuke));
                }
                if std::env::var("TA_SIEGE_RIDE").is_ok() {
                    v.push((frame("TA_SIEGE_RIDE_AT").unwrap_or(3).max(3), DemoAction::SiegeRide));
                }
                v.sort_by_key(|(f, _)| *f);
                v
            }
            Some("gallery") => vec![(2, DemoAction::Gallery)],
            // (From the first frame: the scene's clock is the game's.)
            Some("stage") => vec![(0, DemoAction::Stage)],
            Some("perspective") => vec![(3, DemoAction::Perspective)],
            // With --tutorial 2: the lesson played through (select, group, fight).
            Some("tutorial-fight") => vec![(5, DemoAction::SelectAll), (8, DemoAction::Group(1)), (10, DemoAction::TutorialFight), (12, DemoAction::Speed(4.0))],
            // With --tutorial n: everything selected (the guide's next pointer, for captures).
            Some("tutorial-select") => vec![(5, DemoAction::SelectAll)],
            Some("nuke") => vec![(2, DemoAction::NukeSetup), (40, DemoAction::NukeLaunch)],
            Some("stats") => vec![(2, DemoAction::Battle(2750.0, 800.0)), (3, DemoAction::Speed(8.0)), (1500, DemoAction::ShowStats)],
            // The command deck with nothing selected: the Commander building, some economy.
            Some("overview") => vec![
                (5, DemoAction::SelectCommander),
                (10, DemoAction::BuildNear("ARMSOLAR", 120.0, 40.0)),
                (12, DemoAction::BuildNear("ARMLAB", -60.0, 150.0)),
                (14, DemoAction::Speed(4.0)),
                (20, DemoAction::Deselect),
            ],
            Some("factory") => vec![
                (5, DemoAction::SelectCommander),
                (10, DemoAction::BuildNear("ARMVP", 0.0, 170.0)),
                (900, DemoAction::Produce("ARMVP", "ARMSTUMP", 3)),
                (901, DemoAction::Produce("ARMVP", "ARMFLASH", 2)),
            ],
            _ => Vec::new(),
        };
        // `TA_UI_CLICKS="frame:x,y;..."`: left clicks on the interface at those frames, at those
        // window pixels (as a screenshot shows them), with or without a demo.
        let mut script = script;
        // (The benchmark's scenes begin on their first frame, as a staged scene does.)
        if benchmark.is_some() {
            script = vec![(0, DemoAction::Stage)];
        }
        // `TA_RADIAL=<unit>`: its radial menu at `TA_RADIAL_AT` (any demo).
        if std::env::var("TA_RADIAL").is_ok() {
            let at = std::env::var("TA_RADIAL_AT").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(40).max(3);
            script.push((at, DemoAction::OpenRadial));
        }
        // `TA_RIDE=<unit>`: into the nearest of that type at `TA_RIDE_AT` (any demo).
        if std::env::var("TA_RIDE").is_ok() {
            let at = std::env::var("TA_RIDE_AT").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(3).max(3);
            script.push((at, DemoAction::RideNearest));
        }
        for click in std::env::var("TA_UI_CLICKS").unwrap_or_default().split(';').filter(|c| !c.trim().is_empty()) {
            let parsed = click.split_once(':').and_then(|(f, at)| {
                let (x, y) = at.split_once(',')?;
                Some((f.trim().parse::<u32>().ok()?, x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?))
            });
            match parsed {
                Some((f, x, y)) => script.extend([(f, DemoAction::UiMove(x, y)), (f + 1, DemoAction::UiButton(x, y, true)), (f + 2, DemoAction::UiButton(x, y, false))]),
                None => log::warn!("TA_UI_CLICKS: {click:?} is not frame:x,y"),
            }
        }
        // `TA_UI_HOVER="frame:x,y;..."`: the pointer rests there from those frames (over the map
        // as well as the interface).
        for hover in std::env::var("TA_UI_HOVER").unwrap_or_default().split(';').filter(|c| !c.trim().is_empty()) {
            let parsed = hover.split_once(':').and_then(|(f, at)| {
                let (x, y) = at.split_once(',')?;
                Some((f.trim().parse::<u32>().ok()?, x.trim().parse::<f32>().ok()?, y.trim().parse::<f32>().ok()?))
            });
            match parsed {
                Some((f, x, y)) => script.push((f, DemoAction::UiHover(x, y))),
                None => log::warn!("TA_UI_HOVER: {hover:?} is not frame:x,y"),
            }
        }
        // `TA_UI_MOUSE="frame:x,y[,button];..."`: the mouse to window pixels (x, y) at those
        // frames, for the interface and the map alike; `+left` presses the button there, `-left`
        // lets it go, `left` clicks (pressed, let go the next frame); right and middle too.
        // A drag: "60:500,300,+left;62:620,380;64:700,450,-left".
        for event in std::env::var("TA_UI_MOUSE").unwrap_or_default().split(';').filter(|c| !c.trim().is_empty()) {
            let parsed = event.split_once(':').and_then(|(f, rest)| {
                let mut parts = rest.split(',').map(str::trim);
                let (x, y) = (parts.next()?.parse::<f32>().ok()?, parts.next()?.parse::<f32>().ok()?);
                let button = match parts.next() {
                    None => None,
                    Some(b) => {
                        let (edge, name) = match b.as_bytes().first() {
                            Some(b'+') => (Some(true), &b[1..]),
                            Some(b'-') => (Some(false), &b[1..]),
                            _ => (None, b),
                        };
                        let id = match name.to_ascii_lowercase().as_str() {
                            "left" => MouseButtonId::Left,
                            "right" => MouseButtonId::Right,
                            "middle" => MouseButtonId::Middle,
                            _ => return None,
                        };
                        Some((id, edge))
                    }
                };
                Some((f.trim().parse::<u32>().ok()?, x, y, button))
            });
            match parsed {
                Some((f, x, y, None)) => script.push((f, DemoAction::UiMouse(x, y, None))),
                Some((f, x, y, Some((b, Some(pressed))))) => script.push((f, DemoAction::UiMouse(x, y, Some((b, pressed))))),
                Some((f, x, y, Some((b, None)))) => script.extend([(f, DemoAction::UiMouse(x, y, Some((b, true)))), (f + 1, DemoAction::UiMouse(x, y, Some((b, false))))]),
                None => log::warn!("TA_UI_MOUSE: {event:?} is not frame:x,y[,(+|-)left|right|middle]"),
            }
        }
        // `TA_UI_TEXT="frame:text;..."`: text typed at those frames into the field with the focus.
        for typed in std::env::var("TA_UI_TEXT").unwrap_or_default().split(';').filter(|c| !c.trim().is_empty()) {
            match typed.split_once(':').and_then(|(f, text)| Some((f.trim().parse::<u32>().ok()?, text))) {
                // (A capture's few words, kept for the run.)
                Some((f, text)) => script.push((f, DemoAction::UiText(Box::leak(text.to_string().into_boxed_str())))),
                None => log::warn!("TA_UI_TEXT: {typed:?} is not frame:text"),
            }
        }
        // `TA_UI_PAD="frame:down;..."`: a gamepad's buttons for the interface at those frames
        // (up, down, left, right: the d-pad; a, b, start), through the pad's own path.
        for press in std::env::var("TA_UI_PAD").unwrap_or_default().split(';').filter(|c| !c.trim().is_empty()) {
            use crate::gamepad::Nav;
            let nav = |name: &str| {
                Some(match name.to_ascii_lowercase().as_str() {
                    "up" => Nav::Up,
                    "down" => Nav::Down,
                    "left" => Nav::Left,
                    "right" => Nav::Right,
                    "a" => Nav::Activate,
                    "b" => Nav::Back,
                    "start" => Nav::Menu,
                    _ => return None,
                })
            };
            match press.split_once(':').and_then(|(f, b)| Some((f.trim().parse::<u32>().ok()?, nav(b.trim())?))) {
                Some((f, nav)) => script.push((f, DemoAction::UiPad(nav))),
                None => log::warn!("TA_UI_PAD: {press:?} is not frame:button"),
            }
        }
        // `TA_UI_KEYS="frame:Escape;..."`: keys pressed at those frames (Escape, Enter, Space, Tab,
        // Home, F1-F12, A-Z, 0-9), with or without a demo.
        for press in std::env::var("TA_UI_KEYS").unwrap_or_default().split(';').filter(|c| !c.trim().is_empty()) {
            match press.split_once(':').and_then(|(f, k)| Some((f.trim().parse::<u32>().ok()?, key_named(k.trim())?))) {
                Some((f, key)) => script.push((f, DemoAction::AppKey(key))),
                None => log::warn!("TA_UI_KEYS: {press:?} is not frame:key"),
            }
        }
        let mut menu = Menu::new(&assets);
        menu.rules.unit_limit = config.gameplay.unit_limit;
        menu.connect = crate::lobby::ConnectForm::remembered(&config.gameplay);
        // `--host PORT` / `--join ADDRESS` go straight to the lobby.
        let setup_required = profile.is_none() && crate::profile_progress::ordinary_run(args);
        let lobby = match (&args.host, &args.join) {
            _ if setup_required => None,
            (Some(port), _) => Some(crate::lobby::NetLobby::host_now(*port, &menu.connect.name, &assets)),
            (None, Some(addr)) => Some(crate::lobby::NetLobby::join_now(addr, &menu.connect.name, &assets).map(|mut l| {
                l.auto_ready = args.auto_start.is_some() || args.net_check_tick.is_some();
                l.auto_watch = args.watch;
                l
            })),
            _ => None,
        };
        match lobby {
            Some(Ok(l)) => {
                menu.lobby = Some(l);
                menu.page = crate::menu::Page::Lobby;
            }
            Some(Err(e)) => {
                menu.connect.tell(format!("{e}"));
                menu.page = crate::menu::Page::Multiplayer;
            }
            None => {}
        }
        if args.open_skirmish || args.skirmish_map.is_some() {
            menu.open_skirmish();
        }
        if args.open_campaign {
            if let Some(c) = assets.campaigns.first() {
                let id = c.id.clone();
                menu.campaign.select(&assets, &id, ta_config::CampaignProgress::load().unlocked(&id));
            }
            menu.campaign.set_difficulty(&ta_config::CampaignProgress::load().difficulty);
            menu.page = crate::menu::Page::Campaign;
        }
        // (Achievements earlier play has met, an update having brought them, are recorded
        // first: their notices come up on the title screen.)
        let earlier = profile.as_ref().filter(|_| crate::profile_progress::ordinary_run(args))
            .and_then(|p| ta_config::profiles::ProfileStore::open(ta_config::config_dir()).ok()?.refresh_achievements(&p.id).ok()).unwrap_or_default();
        let career = profile.as_ref().and_then(|p| ta_config::profiles::ProfileStore::open(ta_config::config_dir()).ok()?.career(&p.id).ok()).unwrap_or_default();
        ta_ui::badges::preload();
        if let Some(profile) = &profile { menu.use_profile(&assets, profile); } else { menu.restore_skirmish(&assets); }
        // (The command line's map over the one kept from last time.)
        if let Some(map) = &args.skirmish_map {
            menu.choose_skirmish_map(&assets, map);
        }
        let mut state = State {
            profile,
            career,
            profile_eligible: crate::profile_progress::ordinary_run(args),
            profile_editor: None,
            career_window: ta_ui::profiles::CareerWindow::default(),
            profile_resume: None,
            profile_network: setup_required.then(|| (args.host, args.join.clone())),
            match_xp: 0,
            achievement_toasts: Default::default(),
            battle_start_xp: None,
            battle_achievements: Vec::new(),
            battle_achievement_xp: 0,
            unlock_picture: None,
            unlock_look_tick: 0,
            unlock_looked: Default::default(),
            window,
            window_shown: true,
            renderer: RenderSlot(Some(renderer)),
            loaded_game: None,
            loaded_stage,
            camera,
            input: Input::default(),
            egui: Egui { ctx, state: egui_state, renderer: egui_renderer, clock: 0.0 },
            attract: Attract { target: Vec3::ZERO, retarget_in: 0.0, clock: 0.0, warmup: if attract_ready { 0 } else { 30 * 60 * 5 }, snap: attract_ready },
            menu_film_world: menu_film.as_ref().map(|f| f.world()),
            menu_film,
            loaded_menu_film: None,
            menu_film_idle: 0.0,
            menu_film_lens: false,
            veil: 0.0,
            veil_lens: false,
            assets,
            audio,
            game,
            screen,
            menu,
            minimap: Minimap::default(),
            hud: Hud::new(args.classic_ui),
            dev: DevPanel {
                visible: args.dev_panel,
                fps: 0.0,
                sun_azimuth: sun.z.atan2(sun.x).to_degrees(),
                sun_elevation: sun.y.asin().to_degrees(),
                times: FrameTimes::default(),
                gpu_timing: args.dev_panel || args.benchmark.is_some(),
                gpu_timing_on: false,
                particles: Default::default(),
                gpu_times: Vec::new(),
                meter: FrameMeter::default(),
            },
            movie: None,
            movie_queue: Vec::new(),
            room_tube: None,
            intro_room: None,
            space: None,
            in_game_menu: false,
            bookmarks: [None; 4],
            last_frame: Instant::now(),
            frames: 0,
            screenshot_path: args.screenshot.clone(),
            screenshot_frame: args.frames,
            exit_after_screenshot: args.screenshot.is_some(),
            shots: args.screenshot.as_ref().and_then(|p| Shots::from_env(p)),
            script,
            demo_keys: Vec::new(),
            demo_ui: Vec::new(),
            camera_spec: args.camera.clone(),
            pending: None,
            backdrop_retry: None,
            pending_pass: None,
            loading: None,
            curtain: None,
            siege_middle: None,
            war_queue: Vec::new(),
            stage: None,
            benchmark,
            frame_ms: 0.0,
            overlay_key: None,
            tour: Vec::new(),
            tour_dir: None,
            tour_step: 0,
            tour_from: 0,
            capture_clock: 0.0,
            fog_key: None,
            progress: ta_config::CampaignProgress::load(),
            settings,
            options_open: false,
            settings_view: Default::default(),
            controls_open: false,
            autosave: Default::default(),
            progress_recorded: false,
            progress_writes: Default::default(),
            outcome_music: false,
            result_glide: None,
            msaa_started,
            tutorial: None,
            tip: None,
            tips_shown: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
            coach: None,
            gamepad: crate::gamepad::Gamepad::new(config.gameplay.gamepad && args.screenshot.is_none()),
            pad: Default::default(),
            pad_focus: false,
            pad_follow: Default::default(),
            demo_pad: Vec::new(),
            demo_pad_used: false,
            keyboard_pointer: Default::default(),
            auto_start: args.auto_start,
            net_check_tick: args.net_check_tick,
            editor: None,
            test_editor: None,
            photo: None,
            rewinding: false,
            showroom: None,
            world_text: Default::default(),
            credits_show: None,
            jukebox: Default::default(),
            wallpaper_ui: None,
            open_photo: false,
            photo_capture: None,
            photo_take_frame: None,
            reel: None,
            highlights_capture: false,
            cast_capture: false,
            film: None,
            film_ui: None,
            film_capture: None,
            clip_previews: None,
            clip_capture: None,
            spy: None,
            memory: Default::default(),
            companion: None,
            companion_hinted: false,
            companion_capture: false,
            perspective: None,
            perspective_post: None,
            cockpit_pics: Default::default(),
            instruments: None,
            portrait_studio: None,
            portrait_views: Default::default(),
        };
        if let Some(lesson) = tutorial_lesson {
            let mut t = crate::tutorial::Tutorial::start(lesson, &mut state.game);
            if let Some(step) = std::env::var("TA_TUTORIAL_STEP").ok().and_then(|v| v.parse().ok()) {
                t.jump_to(step, &mut state.game);
            }
            // (`TA_TUTORIAL_STUCK=1`: the step's hint shows at once, for captures.)
            if std::env::var("TA_TUTORIAL_STUCK").is_ok_and(|v| v == "1") {
                t.stuck();
            }
            state.tutorial = Some(t);
        } else if state.screen == Screen::Playing && state.game.setup.mission.is_none() && !state.game.spectator && state.settings.gameplay.start_tips && std::env::var("TA_TIPS").map_or(true, |v| v != "0") {
            // (`TA_TIPS=0` leaves the tip out, for captures.)
            state.tips_shown += 1;
            state.tip = Some(crate::tips::pick(state.tips_shown, &state.settings.gameplay));
        }
        if state.screen == Screen::Playing && state.tutorial.is_none() && !state.game.spectator {
            state.coach = coach_for(&state.settings, &state.career, &state.game.setup, state.script.is_empty());
        }
        // Captures: `TA_REVEAL=1` shows the whole map, as the `+nowisee` cheat does (only what
        // is shown; the battle is the same).
        if std::env::var_os("TA_REVEAL").is_some() {
            state.game.set_reveal(crate::chat::Reveal::All);
        }
        // Captures: a page or dialog open from the start.
        match args.open.as_deref() {
            Some("tutorial") => state.menu.page = crate::menu::Page::Tutorial,
            Some("conquest") => state.menu.open_conquest(),
            Some(page @ ("conquest-demo" | "conquest-target" | "conquest-briefing" | "conquest-defence" | "conquest-perks" | "conquest-report" | "conquest-won")) => {
                state.menu.open_conquest();
                state.menu.conquest.state = if page == "conquest-won" { crate::conquest::Conquest::demo_won(&state.assets.maps) } else { crate::conquest::Conquest::demo(&state.assets.maps) };
                match page {
                    "conquest-target" => state.menu.conquest.select_target(),
                    "conquest-briefing" => state.menu.conquest.brief_target(),
                    "conquest-defence" => state.menu.conquest.brief_defence(),
                    // (A promotion's choice: the demo's last perk put back.)
                    "conquest-perks" => {
                        if let Some(c) = state.menu.conquest.state.as_mut() {
                            c.perks.pop();
                        }
                        state.menu.conquest.show_perks();
                    }
                    "conquest-report" => state.menu.conquest.show_report(),
                    _ => {}
                }
            }
            // The demo conquest's next battle (not saved: its result goes nowhere), or its end
            // won or lost a moment in; `TA_CONQUEST_DEFEND=1` fights off its counter-attack.
            Some(page @ ("conquest-battle" | "conquest-victory" | "conquest-defeat")) => {
                let defend = std::env::var("TA_CONQUEST_DEFEND").is_ok_and(|v| v == "1");
                if let Some(c) = crate::conquest::Conquest::demo(&state.assets.maps)
                    && let Some(i) = if defend { c.counter.map(|counter| counter.system) } else { (0..c.systems.len()).find(|&i| c.attackable(i)) }
                {
                    state.pending = c.setup(i, &state.assets.maps).map(Transition::StartGame);
                }
                match page {
                    "conquest-victory" => state.script.push((2, DemoAction::Outcome(Outcome::Victory))),
                    "conquest-defeat" => state.script.push((2, DemoAction::Outcome(Outcome::Defeat))),
                    _ => {}
                }
            }
            Some("records") => state.menu.open_records(crate::replay::RecordKind::Save),
            Some("replays") => state.menu.open_records(crate::replay::RecordKind::Replay),
            Some("credits") => state.menu.page = crate::menu::Page::Credits,
            Some("single-player") => state.menu.show_title_menu(crate::menu::TitleMenu::SinglePlayer),
            Some("archives" | "extras") => state.menu.show_title_menu(crate::menu::TitleMenu::Extras),
            Some("timeline") => state.menu.page = crate::menu::Page::Timeline,
            Some("jukebox") => state.jukebox.open = true,
            // The first campaign's next mission, its briefing playing (`TA_BRIEF_AT`), or
            // another's (`TA_BRIEF_MISSION=<campaign id>:<index>`).
            Some("briefing") => {
                state.menu.page = crate::menu::Page::Campaign;
                state.menu.campaign.auto_brief = true;
                if let Some((id, index)) = std::env::var("TA_BRIEF_MISSION").ok().and_then(|v| v.rsplit_once(':').and_then(|(id, i)| Some((id.trim().to_string(), i.trim().parse::<usize>().ok()?))))
                    && let Some(c) = state.assets.campaigns.iter().find(|c| c.id.eq_ignore_ascii_case(&id) || c.title.eq_ignore_ascii_case(&id))
                {
                    let id = c.id.clone();
                    state.menu.campaign.select(&state.assets, &id, index);
                }
            }
            Some("units") => state.menu.open_units(),
            Some("scenarios") => {
                state.menu.open_scenarios(&state.assets);
                // (`TA_SCENARIO=<id>`: that one chosen.)
                if let Ok(id) = std::env::var("TA_SCENARIO") {
                    state.menu.select_scenario(&id);
                }
            }
            Some("multiplayer") => state.menu.page = crate::menu::Page::Multiplayer,
            Some("options") => state.options_open = true,
            // A category of the options (`options-controls`), or a search (`options-search`,
            // for `TA_SETTINGS_SEARCH`).
            Some(o) if o.starts_with("options-") => {
                state.options_open = true;
                let wanted = &o["options-".len()..];
                if wanted == "search" {
                    state.settings_view.set_search(&std::env::var("TA_SETTINGS_SEARCH").unwrap_or_default());
                } else if let Some(c) = ta_ui::settings::Category::ALL.into_iter().find(|c| c.label().eq_ignore_ascii_case(wanted)) {
                    state.settings_view.open(c);
                }
            }
            Some("pause") => state.in_game_menu = true,
            // The battle menu with its question before leaving up.
            Some("leave") => {
                state.in_game_menu = true;
                state.game.leave_asked = Some(crate::game::Leave::Menu);
            }
            Some("history") => state.game.history_open = true,
            // The cheat panel, and the chat with `TA_CHAT`'s lines ("|" between them) said.
            Some("cheats") => state.game.cheat_panel = true,
            Some("chat") => {
                for line in std::env::var("TA_CHAT").unwrap_or_default().split('|').filter(|l| !l.trim().is_empty()) {
                    state.game.say(line, false);
                }
                state.game.open_chat(false);
            }
            Some("stats") => {
                state.in_game_menu = true;
                state.game.stats_open = true;
            }
            Some("controls") => state.controls_open = true,
            // (A picture of the end panel: it records nothing, campaign or scenario.)
            Some(page @ ("victory" | "defeat")) if let Some(minutes) = std::env::var("TA_RESULT_MINUTES").ok().and_then(|v| v.parse::<u32>().ok()) => {
                state.game.fast_forward(minutes * 30 * 60);
                state.game.outcome = Some(if page == "victory" { crate::game::Outcome::Victory } else { crate::game::Outcome::Defeat });
                state.progress_recorded = true;
                state.game.career_record = sample_battle_record(&state.career);
            }
            // The companion screen over a battle `TA_RESULT_MINUTES` in (the player's side played
            // by a computer player too), for captures (`TA_COMPANION_SHOT`).
            Some("companion") => {
                if let Some(minutes) = std::env::var("TA_RESULT_MINUTES").ok().and_then(|v| v.parse::<u32>().ok()) {
                    state.game.fast_forward(minutes * 30 * 60);
                }
                state.companion_capture = true;
            }
            Some("victory") => {
                state.game.outcome = Some(crate::game::Outcome::Victory);
                state.progress_recorded = true;
                state.game.career_record = sample_battle_record(&state.career);
            }
            Some("defeat") => {
                state.game.outcome = Some(crate::game::Outcome::Defeat);
                state.progress_recorded = true;
                state.game.career_record = sample_battle_record(&state.career);
            }
            Some("loading") => state.loading = Some("Painted Desert".into()),
            // The intro (in its room, with the setting on) for captures: `TA_ROOM_LEAN=1` leans
            // the camera in after four seconds (`TA_ROOM_LEAN_AT=<seconds>`: after that many).
            Some("intro") => {
                state.start_intro();
                if std::env::var("TA_ROOM_LEAN").is_ok_and(|v| v == "1")
                    && let Some(r) = state.intro_room.as_mut()
                {
                    r.close_after(std::env::var("TA_ROOM_LEAN_AT").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(4.0));
                }
            }
            // A space scene of the film's (`TA_SPACE=<scene.toml>`, else the built-in one).
            // (A capture of a scene that does not read fails, rather than film the menu.)
            Some("space") => match crate::space::SpaceView::open(&mut state.renderer) {
                Ok(view) => {
                    state.space = Some(view);
                    state.screen = Screen::Space;
                }
                Err(e) if args.screenshot.is_some() => return Err(e.context("--open space")),
                Err(e) => log::error!("--open space: {e:#}"),
            },
            Some("overlay-metal") => state.game.overlay = Some(crate::overlay::Overlay::Metal),
            Some("overlay-height") => state.game.overlay = Some(crate::overlay::Overlay::Height),
            Some("overlay-path") => state.game.overlay = Some(crate::overlay::Overlay::Path),
            Some("overlay-sight") => state.game.overlay = Some(crate::overlay::Overlay::Sight),
            // An AI battle on the map (`TA_HIGHLIGHTS_MINUTES`, 12) watched as a replay: fast-
            // forwarded to its end, then its highlights; or (`cast`) in the cast's view.
            Some(open @ ("highlights" | "cast")) => {
                let minutes: u64 = std::env::var("TA_HIGHLIGHTS_MINUTES").ok().and_then(|v| v.parse().ok()).unwrap_or(12);
                let mut setup = state.game.setup.clone();
                // (`TA_CAST_PLAYERS=n`: that many, as many as the map has places for.)
                if let Some(n) = std::env::var("TA_CAST_PLAYERS").ok().and_then(|v| v.parse::<usize>().ok()) {
                    let starts = state.assets.maps.iter().find(|m| m.name.eq_ignore_ascii_case(&setup.map)).map_or(2, |m| m.start_positions);
                    setup.players = (0..n.clamp(2, starts.max(2)))
                        .map(|i| crate::setup::SlotSetup { side: if i % 2 == 0 { "ARM" } else { "CORE" }.into(), color: i as u8, controller: crate::setup::Controller::Human, start: i, team: i as u8, name: String::new() })
                        .collect();
                }
                for p in &mut setup.players {
                    p.controller = crate::setup::Controller::Ai(ta_ai::Difficulty::Hard);
                }
                setup.local_player = None;
                let title = format!("{} (AI battle)", setup.map);
                let hash = state.assets.record_data_hash(&setup);
                let record = crate::replay::Record::new(crate::replay::RecordKind::Replay, hash, state.assets.mods.record(), setup, Vec::new(), minutes * 30 * 60, title);
                state.pending = Some(Transition::Play(record));
                state.highlights_capture = open == "highlights";
                state.cast_capture = open == "cast";
            }
            // Photo mode, a few frames before the capture (the battle runs until then), with
            // any of: `-lens` or `-tilt` (a depth of field), `-hidden` (no panel), `-take` (a
            // photo a frame before the capture, to `TA_PICTURES`).
            Some(o) if o == "photo" || o.starts_with("photo-") => {
                state.photo_capture = Some((o.to_string(), args.frames.saturating_sub(8)));
                if o.split('-').any(|w| w == "take") {
                    state.photo_take_frame = Some(args.frames.saturating_sub(1));
                }
            }
            Some("profile" | "career" | "achievements") => {} // Opened after profile initialization below.
            Some(other) => log::warn!("--open {other}: no such page or dialog"),
            None => {}
        }
        // `--editor [map]`: straight into the map editor.
        if let Some(map) = &args.editor {
            let start = if map.trim().is_empty() { crate::editor::Start::New(crate::editor::default_new(&state.assets)) } else { crate::editor::Start::Open(map.trim().to_string()) };
            state.screen = Screen::Menu;
            state.pending = Some(Transition::Editor(start, args.editor_open.clone()));
        }
        // The first music: a battle's, or the menus'; the intro's movies play alone (the menus'
        // music starts when they end, see `next_movie`).
        match state.screen {
            Screen::Playing => state.audio.set_mood(ta_audio::Mood::Peace),
            Screen::Menu => state.audio.set_mood(ta_audio::Mood::Briefing),
            Screen::Intro | Screen::Editor | Screen::Space => {}
        }
        if state.screen == Screen::Intro && state.intro_room.is_none() && state.movie.is_none() {
            state.start_intro();
        }
        if let Some(path) = args.load.as_ref().or(args.replay.as_ref()) {
            match crate::replay::Record::read(path) {
                // (Made with other mods or game data, it is asked about first, over the menu; not
                // when the launcher asked already: `--anyway`.)
                Ok(mut record) => {
                    record.opened_anyway = args.anyway;
                    let installed = crate::record_mods::installed(&state.settings);
                    if let Some(MenuAction::Play(record)) = state.menu.offer(path.clone(), record, &state.assets, &installed) {
                        state.pending = Some(Transition::Play(record));
                    }
                }
                Err(e) => {
                    // Opened from Explorer, most likely: say why, then show the menu.
                    log::error!("could not read {}: {e:#}", path.display());
                    crate::message_box("TA Reborn", &ta_ui::i18n::tf("Could not open {file}: {error}", &[("file", &path.display()), ("error", &format!("{e:#}"))]));
                }
            }
        }
        state.game.profile_allegiance = state.profile.as_ref().map(|p| p.allegiance);
        state.game.profile_progress.disqualified = !state.profile_eligible;
        state.achievement_toasts.push(&earlier, &earlier);
        // (Captures: `TA_ACHIEVEMENT_TOAST=a,b` shows those achievements' notices, as if earned
        // now; `TA_ACHIEVEMENT_TOAST_EARLIER=1` as if from the service record.)
        if let Ok(list) = std::env::var("TA_ACHIEVEMENT_TOAST") {
            let ids: Vec<String> = list.split(',').map(|s| s.trim().to_string()).filter(|s| ta_config::achievements::find(s).is_some()).collect();
            let earlier = if std::env::var_os("TA_ACHIEVEMENT_TOAST_EARLIER").is_some() { ids.clone() } else { Vec::new() };
            state.achievement_toasts.push(&ids, &earlier);
        }
        // (`TA_PROMOTION_TOAST=<level>`: the notice of a promotion to that level's rank.)
        if let Some(level) = std::env::var("TA_PROMOTION_TOAST").ok().and_then(|v| v.trim().parse::<u32>().ok()) {
            state.achievement_toasts.push_promotion(ta_config::profiles::rank(ta_config::profiles::rank_threshold(level)));
        }
        if state.profile.is_none() && state.profile_eligible || args.open.as_deref() == Some("profile") {
            if state.screen == Screen::Intro { state.finish_intro(); }
            state.profile_resume = Some((state.screen, state.pending.take()));
            state.profile_editor = Some(ta_ui::profiles::ProfileEditor::new(None));
            state.screen = Screen::Menu;
            state.attract.warmup = 0;
            state.game.paused = true;
        }
        if matches!(args.open.as_deref(), Some("career" | "achievements")) && state.profile.is_some() {
            if state.screen == Screen::Intro { state.finish_intro(); }
            state.screen = Screen::Menu;
            state.open_career(args.open.as_deref() == Some("achievements"));
        }
        Ok(state)
    }

    /// Queues TA's intro movies: the Cavedog logo, then the main intro. With the intro room
    /// on, they wait for its monitor to warm up.
    fn start_intro(&mut self) {
        self.audio.begin_intro();
        self.movie_queue = vec!["2.zrb", "1.zrb"];
        if self.settings.graphics.intro_room {
            self.movie = None;
            self.audio.stop_movie_audio();
            self.intro_room = Some(crate::intro_room::IntroRoom::new(&self.assets, crate::intro_room::season(&self.settings.launcher), self.settings.gameplay.reduced_flashes));
            self.screen = Screen::Intro;
        } else {
            self.next_movie();
        }
    }

    fn next_movie(&mut self) {
        self.movie = None;
        self.audio.stop_movie_audio();
        while let Some(name) = self.movie_queue.pop() {
            // Movies are loose files in the install's Data folder (also exposed by the VFS).
            if let Some(bytes) = self.assets.vfs.read_opt(&format!("data/{name}")) && let Some(mut m) = MoviePlayer::open(bytes, name) {
                m.for_room = self.intro_room.is_some();
                // The last, the main intro (widescreen), is watched closer.
                if self.movie_queue.is_empty()
                    && let Some(room) = self.intro_room.as_mut()
                {
                    room.focus_on_tube();
                }
                self.movie = Some(m);
                self.screen = Screen::Intro;
                return;
            }
        }
        // The last is over: in the room, the camera leans in to the screen first.
        if let Some(room) = self.intro_room.as_mut() {
            room.close();
            return;
        }
        self.screen = Screen::Menu;
        self.audio.set_mood(ta_audio::Mood::Briefing);
    }

    /// The intro is over: the room comes down and the title menu comes up.
    fn finish_intro(&mut self) {
        self.intro_room = None;
        self.renderer.set_room(None);
        self.movie = None;
        self.movie_queue.clear();
        self.audio.stop_movie_audio();
        self.screen = Screen::Menu;
        self.audio.set_mood(ta_audio::Mood::Briefing);
    }

    /// The title of the battle a transition loads (for the loading panel), if it loads one.
    fn battle_title(&self, t: &Transition) -> Option<String> {
        let map = |setup: &crate::setup::GameSetup| {
            if let Some(m) = &setup.mission {
                return self.assets.campaigns.iter().find(|c| c.id == m.campaign).and_then(|c| c.missions.get(m.index)).map_or(setup.map.clone(), |e| e.name.clone());
            }
            self.assets.maps.iter().find(|m| m.name.eq_ignore_ascii_case(&setup.map)).map_or(setup.map.clone(), |m| m.title.clone())
        };
        match t {
            Transition::StartGame(setup) | Transition::StartNet(setup, _) => Some(map(setup)),
            Transition::Play(record) => Some(record.title.clone()),
            Transition::Highlights => Some(ta_ui::i18n::t("Highlights").to_string()),
            Transition::StartTutorial(n) => Some(ta_ui::i18n::tf("Tutorial · lesson {n}", &[("n", &(n + 1))])),
            Transition::Benchmark(n) => self.benchmark.as_ref().and_then(|b| b.scene(*n).map.clone()),
            _ => None,
        }
    }

    fn apply_transition(&mut self, t: Transition, event_loop: &ActiveEventLoop) -> Result<()> {
        // Leaving the battle leaves photo mode (the picture's settings as they were) and the
        // highlights.
        if let Some(photo) = self.photo.take() {
            photo.leave(&mut self.game, &mut self.renderer.post.settings, &mut self.camera);
        }
        self.drop_perspective();
        self.reel = None;
        // (A staged scene is the battle it was set up in: it ends with it, and its moon.)
        if !matches!(t, Transition::Quit) && self.stage.take().is_some() {
            self.renderer.lighting.moon_dir = None;
        }
        match t {
            Transition::Benchmark(n) => {
                let Some(bench) = self.benchmark.as_ref() else { return Ok(()) };
                let setup = bench.scene(n).setup(None, &self.assets)?;
                // (A battle as any other, then the scene begun at once; at the game's own pace,
                // with no chatter.)
                self.script.clear();
                self.apply_transition(Transition::StartGame(setup), event_loop)?;
                self.game.speed = 1.0;
                self.game.chatter = None;
                self.tip = None;
                set_camera_style(&mut self.camera, ta_config::CameraStyle::Modern);
                self.stage_begin();
                if let Some(b) = self.benchmark.as_mut() {
                    b.loaded();
                }
            }
            Transition::Highlights => {
                if !self.game.highlights_available() {
                    self.game.message(ta_ui::i18n::t(crate::replay::RECORDING_STOPPED));
                    return Ok(());
                }
                let tally = std::mem::take(&mut self.game.tally);
                let moments = tally.moments();
                log::info!("highlights: {}", tally.summary());
                if moments.is_empty() {
                    self.game.tally = tally;
                    self.game.message(ta_ui::i18n::t("Nothing to show yet"));
                    return Ok(());
                }
                log::info!("highlights: {}", moments.iter().map(|m| format!("{:?} {}-{} at {:.0},{:.0} ({} units)", m.kind, m.start, m.end, m.focus.x, m.focus.z, m.units)).collect::<Vec<_>>().join("; "));
                let g = &self.game;
                let record = match g.playback.as_ref().and_then(|p| p.source.clone()) {
                    Some(r) => r,
                    None => {
                        let Some(ticks) = g.record.snapshot() else {
                            self.game.tally = tally;
                            self.game.message(ta_ui::i18n::t(crate::replay::RECORDING_STOPPED));
                            return Ok(());
                        };
                        Arc::new(crate::replay::Record::new(crate::replay::RecordKind::Replay, self.assets.data_hash, self.assets.mods.record(), g.setup.clone(), ticks, g.sim.tick, record_title(g, &self.assets)).with_profile(g))
                    }
                };
                if let Some(net) = self.game.net.as_mut() {
                    net.leave();
                }
                self.apply_transition(Transition::Play((*record).clone()), event_loop)?;
                self.game.tally.enabled = false;
                self.tip = None;
                self.game.messages.clear();
                self.reel = Some(crate::highlights::Reel::new(moments, tally.snapshots, record));
            }
            Transition::Quit => {
                self.archive_replay();
                event_loop.exit();
            }
            Transition::Play(record) => {
                let approximate = self.assets.validate_record(&record)?;
                let resume = record.kind == crate::replay::RecordKind::Save;
                let full_commands = record.metadata.as_ref().is_some_and(|m| m.full_commands);
                // Recorded local-player identity participates in mission and Conquest
                // initialization. Switch the view to spectator only after constructing it.
                self.apply_transition(Transition::StartGame(record.setup.clone()), event_loop)?;
                if !resume {
                    self.game.spectator = true;
                    self.game.selection.clear();
                    self.game.view_as = None;
                    self.game.replay_approximate = approximate;
                    log::info!("replay compatibility: {}", if approximate { "approximate; current engine/data" } else { "matching engine/data" });
                }
                self.tip = None;
                if !resume {
                    self.game.message(ta_ui::i18n::tf("Replay: {title}", &[("title", &record.title)]));
                }
                if resume && let Some((id, disqualified)) = &record.profile_match {
                    self.game.profile_progress.resume(id.clone(), *disqualified);
                }
                let (end_tick, ticks) = (record.end_tick, record.ticks.iter().cloned().collect());
                // (A saved game's constructors on duty, put back once it has caught up.)
                let duties = if resume { record.duties.clone() } else { Vec::new() };
                // Replays keep their record to start over from when seeking backwards.
                let source = (!resume).then(|| std::sync::Arc::new(record));
                // (Their big moments found ahead, on a copy of the battle from its start.)
                if let Some(record) = &source {
                    self.game.replay_scan = Some(crate::replay_scan::ReplayScan::start(record.clone(), self.game.sim.clone(), self.game.ais().to_vec()));
                }
                self.game.playback = Some(crate::game::Playback { ticks, end_tick, resume, source, seek_to: None, full_commands, duties });
                // (Captures: `TA_CAST=1` with the cast's view on, `TA_REPLAY_AT=<seconds>` that far in.)
                if !resume && (std::mem::take(&mut self.cast_capture) || std::env::var("TA_CAST").is_ok_and(|v| v == "1")) {
                    (self.game.cast, self.game.action_camera) = (true, true);
                }
                if !resume && let Some(at) = std::env::var("TA_REPLAY_AT").ok().and_then(|v| v.parse::<u64>().ok()) {
                    self.game.seek_replay(at * 30);
                }
                if !resume {
                    self.film_capture = crate::film::FilmSettings::from_env();
                    self.clip_capture = std::env::var("TA_CLIP").ok().and_then(|v| crate::clips::ClipSpec::parse(&v));
                }
            }
            Transition::Rewind(record, tick) => {
                let approximate = self.assets.validate_record(&record)?;
                let full_commands = record.metadata.as_ref().is_some_and(|m| m.full_commands);
                let (camera, view_as, speed, paused) = (self.camera.clone(), self.game.view_as, self.game.speed, self.game.paused);
                // (The replay's scan and the cast go on across the new start.)
                let (scan, cast, action) = (self.game.replay_scan.take(), self.game.cast, self.game.action_camera);
                self.rewinding = true;
                let started = self.apply_transition(Transition::StartGame(record.setup.clone()), event_loop);
                self.rewinding = false;
                started?;
                self.tip = None;
                self.hud.clear_transmissions();
                self.camera = camera;
                self.game.spectator = true;
                self.game.selection.clear();
                self.game.replay_approximate = approximate;
                (self.game.view_as, self.game.speed, self.game.paused) = (view_as, speed, paused);
                (self.game.replay_scan, self.game.cast, self.game.action_camera) = (scan.filter(|s| s.is_for(&record)), cast, action);
                let ticks = record.ticks.iter().cloned().collect();
                self.game.playback = Some(crate::game::Playback { ticks, end_tick: record.end_tick, resume: false, source: Some(record), seek_to: Some(tick), full_commands, duties: Vec::new() });
            }
            Transition::PlayIntro => self.start_intro(),
            Transition::StartTutorial(lesson) => {
                self.apply_transition(Transition::StartGame(crate::tutorial::setup(lesson)), event_loop)?;
                // (Lessons play at TA's own speed.)
                self.game.speed = 1.0;
                self.tutorial = Some(crate::tutorial::Tutorial::start(lesson, &mut self.game));
                self.tip = None;
                self.coach = None;
            }
            Transition::ResumeEditor => {
                let Some(mut editor) = self.test_editor.take() else {
                    return self.apply_transition(Transition::ToMenu, event_loop);
                };
                log::info!("map editor: back from the test battle");
                self.archive_replay();
                self.tip = None;
                self.tutorial = None;
                editor.resume(&self.assets, &mut self.renderer);
                self.audio.reset_scene();
                self.editor = Some(editor);
                self.screen = Screen::Editor;
                self.in_game_menu = false;
                self.options_open = false;
                self.fog_key = None;
                self.audio.set_mood(ta_audio::Mood::Briefing);
            }
            Transition::Editor(start, panel) => match crate::editor::Editor::start(&self.assets, &mut self.renderer, start, panel.as_deref()) {
                Ok(editor) => {
                    self.audio.reset_scene();
                    self.editor = Some(editor);
                    self.screen = Screen::Editor;
                    self.in_game_menu = false;
                    self.options_open = false;
                    self.fog_key = None;
                    self.audio.set_mood(ta_audio::Mood::Briefing);
                }
                Err(e) => {
                    log::error!("map editor: {e:#}");
                    self.menu.failure = Some((ta_ui::i18n::n("MAP EDITOR"), ta_ui::i18n::tf("The map editor could not open: {error}", &[("error", &format!("{e:#}"))])));
                }
            },
            Transition::StartNet(setup, session) => {
                log::info!("multiplayer game on {}: {}", setup.map, setup.local_player.map_or("watching".to_string(), |p| format!("playing as player {}", p + 1)));
                self.apply_transition(Transition::StartGame(setup), event_loop)?;
                self.tip = None;
                self.game.profile_progress.id = crate::profile_progress::network_id(&self.game.setup, self.assets.data_hash);
                let resume_at_tick = session.backlog() as u64;
                if resume_at_tick > 0 {
                    self.game.profile_progress.needs_baseline = true;
                    self.game.profile_progress.resume_at_tick = resume_at_tick;
                }
                self.game.net = Some(session);
                self.game.player_name = self.menu.connect.name.clone();
                self.game.chatter = None;
                self.game.speed = 1.0;
                self.game.paused = false;
            }
            Transition::ToMenu => {
                self.archive_replay();
                self.hud.clear_transmissions();
                self.editor = None;
                self.showroom = None;
                self.tutorial = None;
                self.tip = None;
                self.menu.conquest.reload();
                if let Some(net) = self.game.net.as_mut() {
                    net.leave();
                }
                self.start_attract()?;
                self.screen = Screen::Menu;
                self.in_game_menu = false;
                self.audio.set_mood(ta_audio::Mood::Briefing);
            }
            Transition::RestoreMenuBackdrop => self.start_attract()?,
            Transition::Rejoin(address) => {
                self.apply_transition(Transition::ToMenu, event_loop)?;
                self.menu.page = crate::menu::Page::Multiplayer;
                self.menu.connect.rejoin(address, &self.assets);
            }
            Transition::ToLobby => {
                // (The battle's connections became the lobby before the menu's warm-up: see
                // `App::draw`.)
                self.apply_transition(Transition::ToMenu, event_loop)?;
                if self.menu.lobby.is_some() {
                    self.menu.page = crate::menu::Page::Lobby;
                } else {
                    self.menu.connect.tell(ta_ui::i18n::t("The lobby is gone: the host has left.").to_string());
                    self.menu.page = crate::menu::Page::Multiplayer;
                }
            }
            Transition::ToConquest => {
                // (After a battle of the conquest: its report opens once the result is in.)
                let battle = self.game.setup.conquest.as_ref().map(|cs| cs.battle);
                self.apply_transition(Transition::ToMenu, event_loop)?;
                self.menu.open_conquest();
                if let Some(battle) = battle {
                    self.menu.conquest.open_after_battle(battle);
                }
            }
            Transition::ToScenarios => {
                self.apply_transition(Transition::ToMenu, event_loop)?;
                self.menu.open_scenarios(&self.assets);
            }
            Transition::ToSkirmish => {
                self.apply_transition(Transition::ToMenu, event_loop)?;
                self.menu.open_skirmish();
                self.menu.easy_opponents();
            }
            Transition::ToCampaign { campaign, mission } => {
                let (mission, difficulty) = campaign_return(&self.progress, self.game.setup.mission.as_ref(), &campaign, mission);
                self.apply_transition(Transition::ToMenu, event_loop)?;
                self.menu.campaign.select(&self.assets, &campaign, mission);
                self.menu.campaign.set_difficulty(&difficulty);
                self.menu.page = crate::menu::Page::Campaign;
            }
            Transition::StartGame(setup) => {
                if self.screen == Screen::Playing {
                    self.archive_replay();
                }
                self.autosave = Default::default();
                self.match_xp = 0;
                self.unlock_picture = None;
                self.unlock_look_tick = 0;
                self.unlock_looked.clear();
                self.battle_start_xp = self.profile.as_ref().map(|_| self.career.total_xp());
                self.battle_achievements.clear();
                self.battle_achievement_xp = 0;
                // Scripted demos count frames from the start of play.
                let base = self.frames;
                for (f, _) in &mut self.script {
                    *f += base;
                }
                self.audio.reset_scene();
                if let Some(ms) = &setup.mission {
                    self.progress.difficulty = ms.difficulty.clone();
                    self.progress_writes.progress(progress::Update::Difficulty(ms.difficulty.clone()));
                }
                self.progress_recorded = false;
                self.fog_key = None;
                self.tutorial = None;
                // A tip to start a skirmish with (not missions; replays clear it).
                self.tip = (setup.mission.is_none() && setup.local_player.is_some() && self.settings.gameplay.start_tips).then(|| {
                    self.tips_shown += 1;
                    crate::tips::pick(self.tips_shown, &self.settings.gameplay)
                });
                self.coach = coach_for(&self.settings, &self.career, &setup, self.script.is_empty());
                self.game = match self.loaded_game.take() {
                    Some(game) => game,
                    None => Game::new(&self.assets, &mut self.renderer, &setup)?,
                };
                self.game.profile_allegiance = self.profile.as_ref().map(|p| p.allegiance);
                self.game.from_editor = self.test_editor.is_some();
                self.game.profile_progress.disqualified = !self.profile_eligible;
                self.controls_open = false;
                // Single-player battles start at the speed of the settings (not replays).
                if self.game.net.is_none() && self.game.playback.is_none() && setup.local_player.is_some() {
                    self.game.speed = self.settings.gameplay.speed.clamp(0.25, 10.0);
                }
                meet_the_enemy(&mut self.game, &self.settings, self.profile_eligible);
                self.screen = Screen::Playing;
                self.in_game_menu = false;
                self.camera.zoom = 1.6;
                self.camera.pitch = ta_render::camera::TA_ELEVATION;
                self.camera.yaw = 0.0;
                // (The menu's backdrop had the modern camera: every battle, replay and lesson
                // is seen the way the player chose.)
                set_camera_style(&mut self.camera, self.settings.graphics.camera);
                if let Some(c) = self.game.local_commander() {
                    crate::hud::focus_on(&mut self.camera, &self.game, c);
                    self.camera.focus.z += 60.0;
                } else if let Some(p) = self.game.start_focus.or_else(|| {
                    // Missions without a commander: look at our forces.
                    let own: Vec<Vec3> = self.game.sim.units().filter(|u| u.owner == self.game.local_player).map(|u| Vec3::from_array(u.pos.to_f32())).collect();
                    (!own.is_empty()).then(|| own.iter().sum::<Vec3>() / own.len() as f32)
                }) {
                    self.camera.focus = Vec3::new(p.x, p.y.max(self.game.sim.world.sea_level.to_f32()), p.z);
                }
                self.audio.set_mood(ta_audio::Mood::Peace);
                self.outcome_music = false;
                if !self.rewinding {
                    self.companion_at_battle(event_loop);
                }
            }
        }
        Ok(())
    }

    /// `--demo gallery`: see [`DemoAction::Gallery`].
    fn gallery(&mut self) {
        use ta_sim::{Fx, FxVec3, Locomotion};
        let env = |k: &str| std::env::var(k).ok();
        let page: usize = env("TA_GALLERY").and_then(|v| v.parse().ok()).unwrap_or(0);
        let size: usize = env("TA_GALLERY_SIZE").and_then(|v| v.parse().ok()).unwrap_or(12).max(1);
        let chosen: Option<Vec<String>> = env("TA_GALLERY_UNITS").map(|v| v.split(',').map(|s| s.trim().to_ascii_uppercase()).collect());
        // A tour photographs every unit (or the chosen ones) one at a time.
        let tour = env("TA_GALLERY_TOUR").map(std::path::PathBuf::from);
        let (page, size) = if tour.is_some() { (0, usize::MAX / 2) } else { (page, size) };
        let sim = &self.game.sim;
        let mut all: Vec<usize> = sim.types.types.iter().filter(|t| chosen.as_ref().is_none_or(|c| c.contains(&t.name.to_ascii_uppercase()))).map(|t| t.id).collect();
        all.sort_by_key(|&t| sim.types.get(t).name.to_ascii_uppercase());
        let shown: Vec<usize> = all.into_iter().skip(page * size).take(size).collect();
        log::info!("gallery page {page}: {}", shown.iter().map(|&t| sim.types.get(t).name.clone()).collect::<Vec<_>>().join(", "));
        // A level stretch of land (and of sea) near the middle of the map.
        let (w, h) = sim.world.size_px();
        let (w, h) = (w.to_f32(), h.to_f32());
        let sea = sim.world.sea_level.to_f32();
        let height = |x: f32, z: f32| sim.world.height_at(Fx::from_f32(x), Fx::from_f32(z)).to_f32();
        let cols = (shown.len().min(size) as f32).sqrt().ceil().max(1.0) as usize;
        let rows = shown.len().min(size).div_ceil(cols).max(1);
        let cell = 210.0;
        let (gw, gh) = (cols as f32 * cell, rows as f32 * cell);
        let fits = |x0: f32, z0: f32, wet: bool| {
            (0..=8).all(|i| {
                (0..=8).all(|j| {
                    let y = height(x0 + gw * i as f32 / 8.0, z0 + gh * j as f32 / 8.0);
                    if wet { y < sea - 25.0 } else { y > sea }
                })
            })
        };
        let find = |wet: bool| {
            let mut best: Option<(f32, f32, f32)> = None;
            let mut z = 64.0;
            while z + gh < h - 64.0 {
                let mut x = 64.0;
                while x + gw < w - 64.0 {
                    if fits(x, z, wet) {
                        let d = (x + gw / 2.0 - w / 2.0).powi(2) + (z + gh / 2.0 - h / 2.0).powi(2);
                        if best.is_none_or(|b| d < b.2) {
                            best = Some((x, z, d));
                        }
                    }
                    x += 128.0;
                }
                z += 128.0;
            }
            best.map(|(x, z, _)| (x, z))
        };
        let land = find(false).unwrap_or((w / 2.0 - gw / 2.0, h / 2.0 - gh / 2.0));
        let water = find(true);
        let focus = Vec3::new(land.0 + gw / 2.0, height(land.0 + gw / 2.0, land.1 + gh / 2.0), land.1 + gh / 2.0);
        // A tour gives every unit a level spot of its own, clear of rocks and trees, wherever
        // the map has one (a single grid that big would run over hills and buried pads).
        let spots: Vec<(f32, f32, bool)> = if tour.is_some() {
            let features: Vec<(f32, f32)> = sim.features.iter().filter(|f| f.alive).map(|f| (f.pos.x.to_f32(), f.pos.z.to_f32())).collect();
            let mut out = Vec::new();
            let mut z = 200.0;
            while z < h - 200.0 {
                let mut x = 200.0;
                while x < w - 200.0 {
                    let samples: Vec<f32> = (0..=4).flat_map(|i| (0..=4).map(move |j| (i as f32 * 70.0 - 140.0, j as f32 * 70.0 - 140.0))).map(|(dx, dz)| height(x + dx, z + dz)).collect();
                    let (lo, hi) = samples.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &y| (lo.min(y), hi.max(y)));
                    let clear = !features.iter().any(|&(fx, fz)| (fx - x).abs() < 130.0 && (fz - z).abs() < 130.0);
                    if hi - lo < 12.0 && clear {
                        let wet = hi < sea - 25.0;
                        if wet || lo > sea {
                            out.push((x, z, wet));
                        }
                    }
                    x += 300.0;
                }
                z += 300.0;
            }
            log::info!("gallery tour: {} level spots ({} at sea)", out.len(), out.iter().filter(|s| s.2).count());
            out
        } else {
            Vec::new()
        };
        let (mut dry, mut wet_spots) = (spots.iter().filter(|s| !s.2), spots.iter().filter(|s| s.2));
        let mut placed = Vec::new();
        for (k, &t) in shown.iter().enumerate() {
            let ty = self.game.sim.types.get(t);
            let wet = ty.def.min_water_depth > 0.0 || ty.locomotion == Locomotion::Naval;
            let name = ty.name.clone();
            let (x0, z0) = if wet { water.unwrap_or(land) } else { land };
            let (c, r) = (k % cols, k / cols);
            let mut at = FxVec3::new(Fx::from_f32(x0 + (c as f32 + 0.5) * cell), Fx::ZERO, Fx::from_f32(z0 + (r as f32 + 0.5) * cell));
            if tour.is_some() {
                let spot = if wet { wet_spots.next().or_else(|| dry.next()) } else { dry.next() };
                match spot {
                    Some(&(x, z, _)) => at = FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z)),
                    None => log::warn!("gallery tour: no level spot left for {name}"),
                }
            }
            if let Some(id) = self.game.sim.spawn_unit(t, self.game.local_player, at, 0, true) {
                placed.push(id);
                if tour.is_some() {
                    self.tour.push((id, name));
                }
            }
        }
        if let Some(dir) = tour {
            let _ = std::fs::create_dir_all(&dir);
            self.tour_dir = Some(dir);
            // (On a stopped animation clock, lamps and glows at their first frame: two tours
            // compare picture by picture.)
            self.renderer.set_animation_time(Some(0.0));
            // Time for every opening and unfolding to finish first.
            self.tour_from = self.game.sim.tick + 30 * 5;
        }
        // Switch on whatever can be (open solar panels, spinning radar dishes...).
        let me = self.game.local_player;
        self.game.issue(ta_sim::Command::SetActive { player: me, units: placed.clone(), on: true });
        // `TA_GALLERY_ACTION`: set them working, to see the animations: `produce` (factories
        // open their yards), `patrol` (mobile units walk, drive, fly), `build` (constructors
        // put up a building beside them), `destruct` (all blow up five seconds on, aircraft
        // in flight: how each kind dies), `attack` (an enemy squad and missile towers see to it).
        let action = env("TA_GALLERY_ACTION").unwrap_or_default();
        for &id in &placed {
            let Some(u) = self.game.sim.unit(id) else { continue };
            let t = self.game.sim.unit_type(u);
            let pos = u.pos;
            let command = match action.as_str() {
                "produce" if t.is_factory => t
                    .build_options
                    .iter()
                    .copied()
                    .find(|&o| self.game.sim.types.get(o).locomotion != Locomotion::Static)
                    .map(|o| ta_sim::Command::Produce { player: me, factory: id, type_id: o, count: 5 }),
                "patrol" | "destruct" | "attack" if t.max_speed > Fx::ZERO && !t.is_factory && (action == "patrol" || t.locomotion == Locomotion::Air) => {
                    // (Aircraft need a long leg to get up to speed.)
                    let leg = if t.locomotion == Locomotion::Air { 900 } else { 120 };
                    let there = pos + FxVec3::new(Fx::from_int(leg), Fx::ZERO, Fx::from_int(0));
                    Some(ta_sim::Command::Order { player: me, units: vec![id], order: ta_sim::Order::Patrol(there), queue: false })
                }
                // `move`: a plain walk or drive along `TA_GALLERY_LEG=dx,dz` (600 east by default),
                // for a unit seen coming towards the camera (a Commander on patrol stops to reclaim).
                "move" if t.max_speed > Fx::ZERO && !t.is_factory => {
                    let leg: Vec<i64> = env("TA_GALLERY_LEG").map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect()).unwrap_or_default();
                    let (dx, dz) = (leg.first().copied().unwrap_or(600), leg.get(1).copied().unwrap_or(0));
                    let there = pos + FxVec3::from_ints(dx, 0, dz);
                    Some(ta_sim::Command::Order { player: me, units: vec![id], order: ta_sim::Order::Move(there), queue: false })
                }
                "build" if t.is_builder && !t.is_factory => t.build_options.iter().copied().find(|&o| self.game.sim.types.get(o).footprint.0 <= 2).map(|o| {
                    let site = pos + FxVec3::new(Fx::from_int(-64), Fx::ZERO, Fx::from_int(40));
                    ta_sim::Command::Order { player: me, units: vec![id], order: ta_sim::Order::Build { type_id: o, pos: site }, queue: false }
                }),
                _ => None,
            };
            if let Some(c) = command {
                self.game.issue(c);
            }
            // `destruct`: everything blows itself up five seconds on (aircraft in flight), to
            // see how each kind dies.
            if action == "destruct" {
                self.game.issue(ta_sim::Command::SelfDestruct { player: me, units: vec![id] });
            }
        }
        // `attack`: an enemy squad fights its way in, and missile towers wait under the
        // aircraft's route (deaths as battles deal them, not the worst a script has).
        if action == "attack" {
            let them = if me == 0 { 1 } else { 0 };
            let spectator = self.game.spectator;
            self.game.spectator = false;
            let mut squad = Vec::new();
            for (i, name) in ["CORRAID", "CORRAID", "CORRAID", "CORRAID", "CORSTORM", "CORSTORM", "CORLEVLR", "CORLEVLR"].iter().enumerate() {
                let Some(t) = self.game.sim.types.id(name) else { continue };
                let at = FxVec3::new(Fx::from_f32(focus.x + 420.0 + (i / 4) as f32 * 40.0), Fx::ZERO, Fx::from_f32(focus.z - 60.0 + (i % 4) as f32 * 40.0));
                if let Some(u) = self.game.sim.spawn_unit(t, them, at, -16384, true) {
                    squad.push(u);
                }
            }
            for (k, name) in ["CORRL", "CORRL", "CORRL"].iter().enumerate() {
                let Some(t) = self.game.sim.types.id(name) else { continue };
                let at = FxVec3::new(Fx::from_f32(focus.x + 250.0 + k as f32 * 200.0), Fx::ZERO, Fx::from_f32(focus.z + 150.0));
                let _ = self.game.sim.spawn_unit(t, them, at, 0, true);
            }
            let centre = FxVec3::new(Fx::from_f32(focus.x), Fx::ZERO, Fx::from_f32(focus.z));
            self.game.issue(ta_sim::Command::Order { player: them, units: squad, order: ta_sim::Order::Fight(centre), queue: false });
            self.game.spectator = spectator;
            self.game.sim.los_enabled = false;
        }
        self.camera.focus = focus;
        self.camera.zoom = 1.45;
    }

    /// Options and Conquest reflow and scroll: they keep a larger requested scale than the
    /// rest, as far as it leaves them 800 x 480 points (at 2.0 in a 1280 x 720 window, Options
    /// had room for its search and little else, and START CONQUEST covered the sides). The
    /// battlefield and remaining menus still need 1024 x 600 points and keep their fit limit.
    fn ui_zoom(&self) -> f32 {
        let size = self.window.inner_size();
        let reflowing = self.options_open || self.screen == Screen::Menu && self.menu.page == crate::menu::Page::Conquest;
        ui_zoom_for(self.settings.gameplay.ui_scale, (size.width, size.height), self.window.scale_factor() as f32, reflowing)
    }

    /// In battle, Tab is the game's (held, the players panel): egui would otherwise move the
    /// keyboard focus to the first button on the screen with it, after which the arrow keys
    /// and Space work that button instead of the game. While a text field has the focus (chat,
    /// a search), Tab stays egui's.
    fn battle_keys_only(&self, raw: &mut egui::RawInput) {
        if self.settings.gameplay.keyboard_pointer && self.keyboard_pointer_available() {
            // These keys already made pointer events. Do not also activate a focused HUD
            // button or move keyboard focus out from under the virtual pointer.
            raw.events.retain(|e| !crate::keyboard_pointer::reserved_event(e));
        } else if self.screen == Screen::Playing && !self.egui.ctx.egui_wants_keyboard_input() && !self.battle_overlay_open() {
            raw.events.retain(|e| !matches!(e, egui::Event::Key { key: egui::Key::Tab, .. }));
        }
    }

    /// Whether the game has the player's attention: its window, or the companion screen's, in
    /// front.
    fn focused(&self) -> bool {
        self.window.has_focus() || self.companion.as_ref().is_some_and(|c| c.window.has_focus())
    }

    fn keyboard_pointer_available(&self) -> bool {
        self.screen == Screen::Playing && self.benchmark.is_none() && (self.window.has_focus() || self.exit_after_screenshot)
            && !self.game.spectator && self.game.playback.is_none() && !self.battle_overlay_open()
            && self.perspective.is_none() && self.photo.is_none() && self.reel.is_none()
            && !self.egui.ctx.text_edit_focused()
    }

    /// Closing a panel with its own shortcut remains possible while battle actions are
    /// blocked. Claim the edge so the ordinary input path cannot reopen it this frame.
    fn close_battle_panel_keys(&mut self) {
        if self.egui.ctx.text_edit_focused() { return; }
        for (action, open) in [("cheats", self.game.cheat_panel), ("share", self.game.share_panel)] {
            if open && self.game.keys.pressed(&self.input, action) {
                if action == "cheats" { self.game.cheat_panel = false; } else { self.game.share_panel = false; }
                if let Some(binding) = crate::keys::Binding::parse(self.settings.keys.get(action)) {
                    self.input.claim_keys(&[binding.key]);
                }
            }
        }
    }

    /// A dialog over the battle that the battle steps back behind, graded down and softly out
    /// of focus, so the dialog reads over it: the pause menu, the statistics, the controls, the
    /// event history, a unit's page, sharing. Not the options (their effects are judged on the
    /// battle), the jukebox, the chat line, the cheats (their tools work on the map) or a
    /// result (which has its own).
    fn battle_veiled(&self) -> bool {
        self.screen == Screen::Playing && self.photo.is_none() && self.perspective.is_none() && self.reel.is_none() && self.game.outcome.is_none() && !self.options_open
            && (self.in_game_menu || self.controls_open || self.game.history_open || self.game.unit_info.is_some() || self.game.share_panel)
    }

    /// The picture's settings without the veil's blur: what photo mode and the perspective
    /// camera keep and put back when they are left (the veil itself is gone by then).
    fn settled_post(&self) -> ta_render::post::PostSettings {
        let mut post = self.renderer.post.settings;
        if self.veil_lens {
            post.dof = None;
        }
        post
    }

    /// Modal battle panels retain keyboard and pointer input even in multiplayer, where
    /// opening a panel does not freeze the simulation.
    fn battle_overlay_open(&self) -> bool {
        self.profile_editor.is_some() || self.career_window.is_open()
            || self.in_game_menu || self.options_open || self.controls_open || self.jukebox.open
            || self.game.chat_input.is_some() || (self.game.cheat_panel && self.game.spawn_tool.is_none()) || self.game.share_panel
            || self.game.unit_info.is_some() || self.game.history_open || self.game.outcome.is_some()
            // (The replay's EXPORT VIDEO and EXPORT CLIP dialogs: Escape closes them, not opens
            // the menu over them.)
            || self.hud.export_dialog_open()
    }

    /// The script's pointer events for the interface due this frame (`TA_UI_CLICKS`), on any
    /// screen: the menus' pages as well as the battle's panels.
    fn ui_script(&mut self) {
        let ppp = self.egui.ctx.pixels_per_point();
        let frame = self.frames;
        for (_, action) in self.script.iter().filter(|(f, _)| *f == frame) {
            match *action {
                DemoAction::UiMove(x, y) => self.demo_ui.push(egui::Event::PointerMoved(egui::pos2(x / ppp, y / ppp))),
                DemoAction::UiHover(x, y) => {
                    self.demo_ui.push(egui::Event::PointerMoved(egui::pos2(x / ppp, y / ppp)));
                    self.input.mouse = glam::Vec2::new(x, y);
                }
                DemoAction::UiButton(x, y, pressed) => {
                    self.demo_ui.push(egui::Event::PointerButton { pos: egui::pos2(x / ppp, y / ppp), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::default() });
                }
                // (The game had it at the start of the frame: `run_demo`.)
                DemoAction::UiMouse(x, y, button) => {
                    let pos = egui::pos2(x / ppp, y / ppp);
                    self.demo_ui.push(egui::Event::PointerMoved(pos));
                    let button = button.and_then(|(b, pressed)| {
                        let b = match b {
                            MouseButtonId::Left => egui::PointerButton::Primary,
                            MouseButtonId::Right => egui::PointerButton::Secondary,
                            MouseButtonId::Middle => egui::PointerButton::Middle,
                            MouseButtonId::Other(_) => return None,
                        };
                        Some((b, pressed))
                    });
                    if let Some((button, pressed)) = button {
                        self.demo_ui.push(egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::default() });
                    }
                }
                DemoAction::UiText(text) => self.demo_ui.push(egui::Event::Text(text.to_string())),
                DemoAction::UiPad(nav) => self.demo_pad.push(nav),
                _ => {}
            }
        }
    }

    /// `TA_RADIAL`: opens the radial menu on our first unit of that type (see
    /// [`DemoAction::OpenRadial`]), at the unit's place on the screen.
    fn open_radial_demo(&mut self) {
        use crate::radial::Menu;
        let name = std::env::var("TA_RADIAL").unwrap_or_default();
        let me = self.game.local_player;
        let found = self.game.sim.units().find(|u| u.owner == me && self.game.sim.unit_type(u).name.eq_ignore_ascii_case(&name)).map(|u| u.id);
        let Some(id) = found else {
            log::warn!("TA_RADIAL: no {name} of ours");
            return;
        };
        let Some(p) = self.game.scene.unit_position(id, self.game.alpha()) else { return };
        let Some(screen) = Game::screen_of(&self.camera, p + Vec3::Y * 8.0) else { return };
        self.input.mouse = screen;
        self.game.open_radial(id, screen, std::env::var("TA_RADIAL_HELD").is_ok_and(|v| v == "1"));
        if let Some(mut r) = self.game.radial.take() {
            match std::env::var("TA_RADIAL_RING").unwrap_or_default().as_str() {
                "fire" => r.enter(&self.game, Menu::Fire),
                "move" => r.enter(&self.game, Menu::Movement),
                "more" => r.enter(&self.game, Menu::More(1)),
                "build" => r.enter(&self.game, Menu::Purposes),
                "buildings" => {
                    r.enter(&self.game, Menu::Purposes);
                    r.enter(&self.game, Menu::Buildings(0, 0));
                }
                "units" => r.enter(&self.game, Menu::Units(0)),
                "confirm" => r.enter(&self.game, Menu::Confirm),
                "duty" => r.enter(&self.game, Menu::Duties),
                _ => {}
            }
            // (A capture's pointer is wherever the desktop's is: the light is the capture's own.)
            r.hover = std::env::var("TA_RADIAL_HOVER").ok().and_then(|v| v.parse::<usize>().ok());
            r.from_pad = true;
            self.game.radial = Some(r);
        }
    }

    fn run_demo(&mut self) {
        // The war's orders whose tick has come.
        if self.war_queue.first().is_some_and(|(t, _)| *t <= self.game.sim.tick) {
            let tick = self.game.sim.tick;
            let spectator = self.game.spectator;
            self.game.spectator = false;
            while self.war_queue.first().is_some_and(|(t, _)| *t <= tick) {
                let (_, c) = self.war_queue.remove(0);
                self.game.issue(c);
            }
            self.game.spectator = spectator;
        }
        let frame = self.frames;
        let due: Vec<DemoAction> = self.script.iter().filter(|(f, _)| *f == frame).map(|(_, a)| a.clone()).collect();
        for action in due {
            match action {
                DemoAction::Outcome(outcome) => {
                    self.game.outcome = Some(outcome);
                    self.progress_recorded = true;
                }
                DemoAction::SelectCommander => {
                    if let Some(c) = self.game.local_commander() {
                        self.game.selection = vec![c];
                    }
                }
                DemoAction::BuildNear(name, dx, dz) => {
                    let Some(c) = self.game.local_commander() else { continue };
                    let Some(t) = self.game.sim.types.id(name) else { continue };
                    let offset = ta_sim::FxVec3::new(ta_sim::Fx::from_f32(dx), ta_sim::Fx::ZERO, ta_sim::Fx::from_f32(dz));
                    let base = self.game.sim.unit(c).map(|u| u.pos + offset);
                    if let Some(site) = base.and_then(|b| self.game.sim.find_build_site(t, b, 20)) {
                        self.game.issue(ta_sim::Command::Order {
                            player: self.game.local_player,
                            units: vec![c],
                            order: ta_sim::Order::Build { type_id: t, pos: site },
                            queue: true,
                        });
                    }
                }
                DemoAction::Produce(factory, unit, n) => {
                    let (Some(ft), Some(ut)) = (self.game.sim.types.id(factory), self.game.sim.types.id(unit)) else { continue };
                    let factories: Vec<ta_sim::UnitId> =
                        self.game.sim.units().filter(|u| u.type_id == ft && u.owner == 0).map(|u| u.id).collect();
                    for f in factories {
                        self.game.issue(ta_sim::Command::Produce { player: 0, factory: f, type_id: ut, count: n });
                        self.game.selection = vec![f];
                    }
                }
                DemoAction::DGun => {
                    let Some(c) = self.game.local_commander() else { continue };
                    let Some(base) = self.game.sim.unit(c).map(|u| u.pos) else { continue };
                    let mut victims = Vec::new();
                    for (i, name) in ["CORAK", "CORAK", "CORTHUD"].iter().enumerate() {
                        let Some(t) = self.game.sim.types.id(name) else { continue };
                        let want = base + ta_sim::FxVec3::new(ta_sim::Fx::from_int(-40 + 40 * i as i64), ta_sim::Fx::ZERO, ta_sim::Fx::from_int(150));
                        let Some(p) = self.game.sim.find_build_site(t, want, 12) else { continue };
                        if let Some(id) = self.game.sim.spawn_unit(t, 1, p, -32768, true) {
                            victims.push(id);
                        }
                    }
                    if let Some(&v) = victims.get(1) {
                        self.game.issue(ta_sim::Command::Order { player: 0, units: vec![c], order: ta_sim::Order::ManualFire(ta_sim::Target::Unit(v)), queue: false });
                    }
                    self.game.scene.after_tick(&self.game.sim);
                }
                DemoAction::Reclaim => {
                    let Some(c) = self.game.local_commander() else { continue };
                    let Some(base) = self.game.sim.unit(c).map(|u| u.pos) else { continue };
                    let nearest = self
                        .game
                        .sim
                        .features
                        .iter()
                        .enumerate()
                        .filter(|(_, f)| f.alive && self.game.sim.data.features[f.def].reclaimable)
                        .min_by_key(|(_, f)| (f.pos - base).length_sq_raw())
                        .map(|(i, _)| i);
                    if let Some(i) = nearest {
                        self.game.issue(ta_sim::Command::Order { player: 0, units: vec![c], order: ta_sim::Order::Reclaim(i), queue: false });
                        let fp = self.game.sim.features[i].pos.to_f32();
                        self.camera.focus = Vec3::new((fp[0] + base.x.to_f32()) * 0.5, fp[1], (fp[2] + base.z.to_f32()) * 0.5);
                    }
                }
                DemoAction::AreaReclaim => {
                    let Some(base) = self.game.local_commander().and_then(|c| self.game.sim.unit(c)).map(|u| u.pos) else { continue };
                    let nearest = self.game.sim.features.iter().filter(|f| f.alive && self.game.sim.data.features[f.def].reclaimable).min_by_key(|f| (f.pos - base).length_sq_raw()).map(|f| Vec3::from_array(f.pos.to_f32()));
                    if let Some(c) = nearest {
                        self.game.area_order(crate::game::CommandMode::Reclaim, c, 220.0, false);
                        self.camera.focus = c;
                        // Leave the ring up, as if the drag were still going on (for captures).
                        self.game.set_command_mode(Some(crate::game::CommandMode::Reclaim));
                        self.game.area = Some((c, 220.0));
                    }
                }
                DemoAction::Seek(tick) => self.game.seek_replay(tick),
                DemoAction::Say(text, allies) => self.game.say(text, allies),
                DemoAction::Group(n) => self.game.groups[n % 10] = self.game.selection.clone(),
                DemoAction::Ignite => {
                    let Some(base) = self.game.local_commander().and_then(|c| self.game.sim.unit(c)).map(|u| u.pos) else { continue };
                    let sim = &self.game.sim;
                    let tree = (0..sim.features.len())
                        .filter(|&i| sim.features[i].alive && sim.data.features[sim.features[i].def].flamable)
                        .min_by_key(|&i| (sim.features[i].pos - base).length_sq_raw());
                    if let Some(i) = tree {
                        let p = Vec3::from_array(self.game.sim.features[i].pos.to_f32());
                        self.game.sim.ignite(i);
                        self.camera.focus = p;
                    }
                }
                DemoAction::PlaceNear(name, dx, dz) => {
                    let Some(base) = self.game.local_commander().and_then(|c| self.game.sim.unit(c)).map(|u| Vec3::from_array(u.pos.to_f32())) else { continue };
                    let Some(t) = self.game.sim.types.id(name) else { continue };
                    let (x, z) = (base.x + dx, base.z + dz);
                    let p = Vec3::new(x, self.game.sim.world.height_at(ta_sim::Fx::from_f32(x), ta_sim::Fx::from_f32(z)).to_f32(), z);
                    self.game.begin_placement(t);
                    self.game.cursor_override = Some(p);
                    self.camera.focus = p;
                }
                DemoAction::PlaceOnPlan(name) => {
                    let Some(c) = self.game.local_commander().and_then(|c| self.game.sim.unit(c)) else { continue };
                    let plan = c.orders.iter().rev().find_map(|o| match o {
                        ta_sim::Order::Build { pos, .. } => Some(Vec3::from_array(pos.to_f32())),
                        _ => None,
                    });
                    let (Some(p), Some(t)) = (plan, self.game.sim.types.id(name)) else { continue };
                    self.game.begin_placement(t);
                    self.game.cursor_override = Some(p);
                    self.game.demo_shift = true;
                    self.camera.focus = p;
                }
                DemoAction::AreaMex(radius) => {
                    let Some(base) = self.game.local_commander().and_then(|c| self.game.sim.unit(c)).map(|u| u.pos) else { continue };
                    let w = &self.game.sim.world;
                    let (bx, bz) = ((base.x.to_int() / 16) as i32, (base.z.to_int() / 16) as i32);
                    let nearest = (0..w.height as i32)
                        .flat_map(|z| (0..w.width as i32).map(move |x| (x, z)))
                        .filter(|&(x, z)| w.metal_under((x, z), (1, 1)) >= 100)
                        .min_by_key(|&(x, z)| (x - bx).pow(2) + (z - bz).pow(2));
                    if let Some((x, z)) = nearest {
                        let (px, pz) = (x as f32 * 16.0 + 8.0, z as f32 * 16.0 + 8.0);
                        let p = Vec3::new(px, w.height_at(ta_sim::Fx::from_f32(px), ta_sim::Fx::from_f32(pz)).to_f32(), pz);
                        self.game.set_command_mode(Some(crate::game::CommandMode::AreaMex));
                        self.game.area_order(crate::game::CommandMode::AreaMex, p, radius, false);
                        self.camera.focus = p;
                    }
                }
                DemoAction::PlaceExtractor => {
                    let Some(base) = self.game.local_commander().and_then(|c| self.game.sim.unit(c)).map(|u| u.pos) else { continue };
                    let me = self.game.local_player;
                    let side = self.game.sim.players[me as usize].side.to_ascii_uppercase();
                    let Some(mex) = self.game.sim.types.id(if side == "CORE" { "CORMEX" } else { "ARMMEX" }) else { continue };
                    // The nearest rich cell, then point a little off it (the ghost snaps).
                    let w = &self.game.sim.world;
                    let (bx, bz) = ((base.x.to_int() / 16) as i32, (base.z.to_int() / 16) as i32);
                    let nearest = (0..w.height as i32)
                        .flat_map(|z| (0..w.width as i32).map(move |x| (x, z)))
                        .filter(|&(x, z)| w.metal_under((x, z), (1, 1)) >= 100)
                        .min_by_key(|&(x, z)| (x - bx).pow(2) + (z - bz).pow(2));
                    if let Some((x, z)) = nearest {
                        let (px, pz) = (x as f32 * 16.0 + 24.0, z as f32 * 16.0 + 24.0);
                        let p = Vec3::new(px, w.height_at(ta_sim::Fx::from_f32(px), ta_sim::Fx::from_f32(pz)).to_f32(), pz);
                        self.game.begin_placement(mex);
                        self.game.cursor_override = Some(p);
                        self.camera.focus = p;
                    }
                }
                DemoAction::NukeSetup => {
                    let Some(c) = self.game.local_commander() else { continue };
                    let Some(base) = self.game.sim.unit(c).map(|u| u.pos) else { continue };
                    let me = self.game.local_player;
                    if let Some(t) = self.game.sim.types.id("ARMSILO") {
                        let want = base + ta_sim::FxVec3::from_ints(-150, 0, 150);
                        if let Some(p) = self.game.sim.find_build_site(t, want, 30) && let Some(silo) = self.game.sim.spawn_unit(t, me, p, 0, true) {
                            self.game.sim.grant_stockpile(silo, 1);
                            // (Selected: its panel shows the missile ready and the next one.)
                            self.game.selection = vec![silo];
                        }
                    }
                    // An enemy outpost to the east.
                    let target = base + ta_sim::FxVec3::from_ints(900, 0, 0);
                    for (i, name) in ["CORLLT", "CORRAD", "CORSOLAR", "CORSOLAR", "CORLAB", "CORTHUD", "CORTHUD", "CORAK", "CORAK", "CORAK"].iter().enumerate() {
                        let Some(t) = self.game.sim.types.id(name) else { continue };
                        let off = ta_sim::FxVec3::from_ints((i as i64 % 4) * 90 - 135, 0, (i as i64 / 4) * 90 - 90);
                        if let Some(p) = self.game.sim.find_build_site(t, target + off, 12) {
                            self.game.sim.spawn_unit(t, 1 - me.min(1), p, 0, true);
                        }
                    }
                    self.game.sim.reveal_map_for(me);
                    self.game.sim.los_enabled = false;
                    self.camera.focus = Vec3::from_array(target.to_f32());
                    self.camera.zoom = 0.8;
                    self.game.scene.after_tick(&self.game.sim);
                }
                DemoAction::Speed(s) => self.game.speed = s,
                DemoAction::TutorialFight => {
                    if let Some(p) = self.tutorial.as_ref().and_then(|t| t.outpost_centre(&self.game)) {
                        let (player, units) = (self.game.local_player, self.game.selection.clone());
                        let at = ta_sim::FxVec3::new(ta_sim::Fx::from_f32(p.x), ta_sim::Fx::from_f32(p.y), ta_sim::Fx::from_f32(p.z));
                        self.game.issue(ta_sim::Command::Order { player, units, order: ta_sim::Order::Fight(at), queue: false });
                    }
                }
                DemoAction::SelectAll => {
                    let me = self.game.local_player;
                    self.game.selection = self.game.sim.units().filter(|u| u.owner == me).map(|u| u.id).collect();
                }
                DemoAction::Deselect => self.game.selection.clear(),
                DemoAction::Spawn(name, player, n, x, z) => {
                    if let Some(t) = self.game.sim.types.id(name) {
                        for i in 0..n {
                            let p = ta_sim::FxVec3::new(ta_sim::Fx::from_f32(x), ta_sim::Fx::ZERO, ta_sim::Fx::from_f32(z + i as f32 * 40.0));
                            self.game.sim.spawn_unit(t, player, p, 16384, true);
                        }
                    }
                }
                DemoAction::HoverFeature => {
                    let Some(base) = self.game.local_commander().and_then(|c| self.game.sim.unit(c)).map(|u| Vec3::from_array(u.pos.to_f32())) else { continue };
                    let sim = &self.game.sim;
                    let nearest = sim
                        .features
                        .iter()
                        .filter(|f| f.alive && sim.data.features[f.def].reclaimable && sim.data.features[f.def].metal + sim.data.features[f.def].energy > 0.0)
                        .map(|f| Vec3::from_array(f.pos.to_f32()))
                        .min_by(|a, b| a.distance_squared(base).total_cmp(&b.distance_squared(base)));
                    if let Some(p) = nearest {
                        self.game.cursor_override = Some(p);
                        self.camera.focus = p;
                    }
                }
                DemoAction::HoverType(name) => {
                    let me = self.game.local_player;
                    let t = self.game.sim.types.id(name);
                    let Some(id) = self.game.sim.units().find(|u| u.owner == me && Some(u.type_id) == t).map(|u| u.id) else {
                        log::warn!("demo: no {name} to hover");
                        continue;
                    };
                    let Some(p) = self.game.scene.unit_position(id, self.game.alpha()) else {
                        log::warn!("demo: {name} has no position yet");
                        continue;
                    };
                    if let Some(screen) = Game::screen_of(&self.camera, p + Vec3::Y * 4.0) {
                        self.input.mouse = screen;
                        self.game.cursor_override = Some(p);
                    }
                }
                DemoAction::PointAt(name) => {
                    let me = self.game.local_player;
                    let t = self.game.sim.types.id(name);
                    let Some(id) = self.game.sim.units().find(|u| u.owner == me && Some(u.type_id) == t).map(|u| u.id) else { continue };
                    let Some(p) = self.game.scene.unit_position(id, self.game.alpha()) else { continue };
                    let ppp = self.egui.ctx.pixels_per_point();
                    if let Some(screen) = Game::screen_of(&self.camera, p + Vec3::Y * 8.0) {
                        self.input.mouse = screen;
                        self.demo_ui.push(egui::Event::PointerMoved(egui::pos2(screen.x / ppp, screen.y / ppp)));
                    }
                }
                DemoAction::ClickOn(scheme, button, name) => {
                    self.settings.gameplay.order_button = scheme;
                    self.game.order_button = scheme;
                    let me = self.game.local_player;
                    let t = self.game.sim.types.id(name);
                    let Some(id) = self.game.sim.units().find(|u| u.owner == me && Some(u.type_id) == t).map(|u| u.id) else { continue };
                    let Some(p) = self.game.scene.unit_position(id, self.game.alpha()) else { continue };
                    if let Some(screen) = Game::screen_of(&self.camera, p + Vec3::Y * 8.0) {
                        self.input.mouse = screen;
                        self.input.buttons_pressed.insert(button);
                        self.input.buttons_released.insert(button);
                    }
                }
                DemoAction::SpawnOnMetal(name) => {
                    let Some(base) = self.game.local_commander().and_then(|c| self.game.sim.unit(c)).map(|u| u.pos) else { continue };
                    if let Some(t) = self.game.sim.types.id(name) {
                        let at = self.game.sim.best_extraction_site(t, base, 24);
                        self.game.sim.spawn_unit(t, self.game.local_player, at, 0, true);
                    }
                }
                DemoAction::GuardWith(guards, target) => {
                    let me = self.game.local_player;
                    let (g, t) = (self.game.sim.types.id(guards), self.game.sim.types.id(target));
                    let guards: Vec<ta_sim::UnitId> = self.game.sim.units().filter(|u| u.owner == me && Some(u.type_id) == g).map(|u| u.id).collect();
                    let Some(target) = self.game.sim.units().find(|u| u.owner == me && Some(u.type_id) == t).map(|u| u.id) else { continue };
                    self.game.issue(ta_sim::Command::Order { player: me, units: guards, order: ta_sim::Order::Guard(target), queue: false });
                }
                DemoAction::Assist(what) => {
                    let g = &self.game;
                    let c = g.local_commander().and_then(|c| g.sim.unit(c));
                    let hovered = g.hovered.and_then(|h| g.sim.unit(h)).map(|u| g.sim.unit_type(u).name.clone());
                    let working = g.sim.units().filter(|u| g.sim.unit_type(u).is_factory && u.owner == g.local_player).map(|u| (u.build.is_some(), u.factory_queue.len())).collect::<Vec<_>>();
                    println!(
                        "assist: {what}: {} orders, hovered {hovered:?}, cursor {:?}, selection {}, Commander's orders {:?}, builds {:?}, factories (working, queued) {working:?}",
                        if g.order_button == ta_config::OrderButton::Left { "left-button" } else { "right-button" },
                        g.cursor_kind(false),
                        g.selection.len(),
                        c.map(|u| u.orders.iter().cloned().collect::<Vec<_>>()),
                        c.and_then(|u| u.build.map(|b| b.target)),
                    );
                }
                DemoAction::HoverUnit => {
                    let Some(p) = self.game.local_commander().and_then(|c| self.game.scene.unit_position(c, self.game.alpha())) else { continue };
                    if let Some(screen) = Game::screen_of(&self.camera, p + Vec3::Y * 8.0) {
                        self.input.mouse = screen;
                        self.game.cursor_override = Some(p);
                    }
                }
                DemoAction::Give => {
                    let team = self.game.sim.players[self.game.local_player as usize].team;
                    self.game.sim.set_team(1, team);
                    self.game.give_selection(1);
                }
                DemoAction::Click(scheme, button, on_commander) => {
                    self.settings.gameplay.order_button = scheme;
                    self.game.order_button = scheme;
                    let Some(c) = self.game.local_commander() else { continue };
                    let Some(p) = self.game.scene.unit_position(c, self.game.alpha()) else { continue };
                    // (Ordering from where it stands: forget any earlier move.)
                    self.game.issue(ta_sim::Command::Order { player: self.game.local_player, units: vec![c], order: ta_sim::Order::Stop, queue: false });
                    let at = if on_commander { p + Vec3::Y * 10.0 } else { p + Vec3::new(160.0, 0.0, 40.0) };
                    if let Some(screen) = Game::screen_of(&self.camera, at) {
                        self.input.mouse = screen;
                        self.input.buttons_pressed.insert(button);
                        self.input.buttons_released.insert(button);
                    }
                }
                DemoAction::Key(k) => self.input.press(k),
                // The game's half (the interface's: `ui_script`): a press over the interface
                // is the interface's, as the window's events reach the game.
                DemoAction::UiMouse(x, y, button) => {
                    use winit::event::{DeviceId, ElementState, MouseButton};
                    let over_ui = self.egui.ctx.egui_wants_pointer_input() || self.egui.ctx.is_pointer_over_egui();
                    let position = winit::dpi::PhysicalPosition::new(x as f64, y as f64);
                    self.input.on_event(&WindowEvent::CursorMoved { device_id: DeviceId::dummy(), position }, false, false);
                    if let Some((b, pressed)) = button {
                        let button = match b {
                            MouseButtonId::Left => MouseButton::Left,
                            MouseButtonId::Right => MouseButton::Right,
                            MouseButtonId::Middle => MouseButton::Middle,
                            MouseButtonId::Other(n) => MouseButton::Other(n),
                        };
                        let state = if pressed { ElementState::Pressed } else { ElementState::Released };
                        self.input.on_event(&WindowEvent::MouseInput { device_id: DeviceId::dummy(), state, button }, over_ui, false);
                    }
                }
                // On every screen: see `ui_script`.
                DemoAction::UiMove(..) | DemoAction::UiHover(..) | DemoAction::UiButton(..) | DemoAction::UiText(..) | DemoAction::UiPad(..) => {}
                DemoAction::AppKey(k) => self.demo_keys.push(k),
                DemoAction::Switch(on) => self.game.set_active(on),
                DemoAction::Attacked(dx, dz) => {
                    let Some(c) = self.game.local_commander() else { continue };
                    let Some(base) = self.game.sim.unit(c).map(|u| Vec3::from_array(u.pos.to_f32())) else { continue };
                    let me = self.game.local_player;
                    // (A unit of ours that is no Commander: the plain notice.)
                    let unit = self.game.sim.units().find(|u| u.owner == me && u.id != c).map_or(c, |u| u.id);
                    let now = self.game.sim.tick as f32 / 30.0;
                    self.game.units_attacked(base + Vec3::new(dx, 0.0, dz), unit, now);
                }
                DemoAction::SelectType(name) => {
                    let me = self.game.local_player;
                    let t = self.game.sim.types.id(name);
                    self.game.selection = self.game.sim.units().filter(|u| u.owner == me && Some(u.type_id) == t).map(|u| u.id).take(1).collect();
                }
                DemoAction::LookAt(x, z) => {
                    let y = self.game.sim.world.height_at(ta_sim::Fx::from_f32(x), ta_sim::Fx::from_f32(z)).to_f32();
                    self.camera.focus = Vec3::new(x, y, z);
                }
                DemoAction::SpawnNear(name, player, n, dx, dz) => {
                    let Some(base) = self.game.local_commander().and_then(|c| self.game.sim.unit(c)).map(|u| u.pos) else { continue };
                    if let Some(t) = self.game.sim.types.id(name) {
                        let spectator = self.game.spectator;
                        self.game.spectator = false;
                        for i in 0..n {
                            let p = base + ta_sim::FxVec3::new(ta_sim::Fx::from_f32(dx + i as f32 * 48.0), ta_sim::Fx::ZERO, ta_sim::Fx::from_f32(dz));
                            let at = self.game.sim.find_build_site(t, p, 12).unwrap_or(p);
                            self.game.sim.spawn_unit(t, player, at, 0, true);
                        }
                        self.game.spectator = spectator;
                    }
                }
                DemoAction::CtrlClick(scheme, button) => {
                    self.settings.gameplay.order_button = scheme;
                    self.game.order_button = scheme;
                    let Some(c) = self.game.local_commander() else { continue };
                    let Some(p) = self.game.scene.unit_position(c, self.game.alpha()) else { continue };
                    self.game.issue(ta_sim::Command::Order { player: self.game.local_player, units: vec![c], order: ta_sim::Order::Stop, queue: false });
                    if let Some(screen) = Game::screen_of(&self.camera, p + Vec3::new(-60.0, 0.0, 250.0)) {
                        self.input.mouse = screen;
                        self.input.modifiers = winit::keyboard::ModifiersState::CONTROL;
                        self.input.buttons_pressed.insert(button);
                        self.input.buttons_released.insert(button);
                    }
                }
                DemoAction::CtrlKey(k) => {
                    self.input.modifiers = winit::keyboard::ModifiersState::CONTROL;
                    self.input.press(k);
                }
                DemoAction::ClearMods => self.input.modifiers = winit::keyboard::ModifiersState::empty(),
                DemoAction::ShiftHold => {
                    self.input.modifiers = winit::keyboard::ModifiersState::SHIFT;
                    self.game.demo_shift = true;
                }
                DemoAction::Report(what) => {
                    let g = &self.game;
                    let mut types: std::collections::BTreeMap<String, usize> = Default::default();
                    for u in g.selection.iter().filter_map(|&id| g.sim.unit(id)) {
                        *types.entry(g.sim.unit_type(u).name.clone()).or_default() += 1;
                    }
                    let c = g.local_commander().and_then(|c| g.sim.unit(c));
                    let group: Vec<String> = g.groups[3].iter().filter_map(|&id| g.sim.unit(id)).map(|u| g.sim.unit_type(u).name.clone()).collect();
                    println!(
                        "controls: {what}: selection {types:?}, waiting {:?}, commander fire {:?} move {:?} order {:?}, group 3 {group:?}, camera ({:.0}, {:.0}), info {:?}, menu {}, messages {} chat {}",
                        g.command_mode,
                        c.map(|u| u.fire_state),
                        c.map(|u| u.move_state),
                        c.and_then(|u| u.orders.front().cloned()),
                        self.camera.focus.x,
                        self.camera.focus.z,
                        g.unit_info.map(|i| g.sim.types.get(i.type_id).name.clone()),
                        self.in_game_menu,
                        g.messages.len(),
                        g.chat_log.len()
                    );
                }
                DemoAction::KeyReport(what) => {
                    let fire = self.game.local_commander().and_then(|c| self.game.sim.unit(c)).map(|u| u.fire_state);
                    println!("keys: {what}: fire order {fire:?}, pending {:?}", self.game.command_mode);
                }
                DemoAction::Check(what) => {
                    let c = self.game.local_commander();
                    let selected = c.is_some_and(|c| self.game.selection == [c]);
                    let moving = c.and_then(|c| self.game.sim.unit(c)).is_some_and(|u| u.orders.iter().any(|o| matches!(o, ta_sim::Order::Move(_))));
                    let menu = self.game.radial.as_ref().is_some_and(|r| r.leaving.is_none());
                    println!("clicks: {what}: selected {selected}, selection {}, moving {moving}, menu {menu}", self.game.selection.len());
                }
                DemoAction::Cloak => {
                    if let Some(c) = self.game.local_commander() {
                        self.game.issue(ta_sim::Command::SetCloak { player: self.game.local_player, units: vec![c], on: true });
                    }
                }
                DemoAction::Naval => {
                    // Open water: a spot and another 600 px east, both at least 30 px deep.
                    let sim = &self.game.sim;
                    let (w, h) = sim.world.size_px();
                    let sea = sim.world.sea_level;
                    let deep = |x: i64, z: i64| sim.world.height_at(ta_sim::Fx::from_int(x), ta_sim::Fx::from_int(z)) < sea - ta_sim::Fx::from_int(30);
                    let spot = (200..h.to_int() - 200).step_by(64).flat_map(|z| (200..w.to_int() - 800).step_by(64).map(move |x| (x, z))).find(|&(x, z)| (0..=600).step_by(100).all(|dx| deep(x + dx, z) && deep(x + dx, z - 100) && deep(x + dx, z + 100)));
                    let Some((x0, z0)) = spot else { continue };
                    let fleets = [["ARMPT", "ARMPT", "ARMROY", "ARMSH", "ARMSH", "ARMDECADE"], ["CORPT", "CORPT", "CORROY", "CORSH", "CORSH", "CORESUPP"]];
                    let spectator = self.game.spectator;
                    self.game.spectator = false;
                    for (side, names) in fleets.iter().enumerate() {
                        let x = if side == 0 { x0 } else { x0 + 600 };
                        let heading = if side == 0 { 16384 } else { -16384 };
                        let mut ids = Vec::new();
                        for (i, name) in names.iter().enumerate() {
                            let Some(t) = self.game.sim.types.id(name) else { continue };
                            let p = ta_sim::FxVec3::from_ints(x, 0, z0 - 100 + i as i64 * 40);
                            if let Some(id) = self.game.sim.spawn_unit(t, side as u8, p, heading, true) {
                                ids.push(id);
                            }
                        }
                        let dest = ta_sim::FxVec3::from_ints(if side == 0 { x0 + 450 } else { x0 + 150 }, 0, z0);
                        self.game.issue(ta_sim::Command::Order { player: side as u8, units: ids, order: ta_sim::Order::Fight(dest), queue: false });
                    }
                    self.game.spectator = spectator;
                    self.game.sim.los_enabled = false;
                    self.camera.focus = Vec3::new(x0 as f32 + 300.0, 0.0, z0 as f32);
                    self.game.scene.after_tick(&self.game.sim);
                }
                DemoAction::ShowStats => {
                    self.in_game_menu = true;
                    self.game.stats_open = true;
                }
                DemoAction::NukeLaunch => {
                    let Some(t) = self.game.sim.types.id("ARMSILO") else { continue };
                    let me = self.game.local_player;
                    let Some(silo) = self.game.sim.units().find(|u| u.type_id == t && u.owner == me).map(|u| u.id) else { continue };
                    let Some(c) = self.game.local_commander() else { continue };
                    let Some(base) = self.game.sim.unit(c).map(|u| u.pos) else { continue };
                    let target = base + ta_sim::FxVec3::from_ints(900, 0, 0);
                    self.game.issue(ta_sim::Command::Order { player: me, units: vec![silo], order: ta_sim::Order::ManualFire(ta_sim::Target::Ground(target)), queue: false });
                }
                DemoAction::Gallery => self.gallery(),
                DemoAction::Stage => self.stage_begin(),
                DemoAction::Perspective => self.perspective_demo(),
                DemoAction::Spy => self.spy_begin(),
                DemoAction::Swarm => {
                    let per_side: usize = std::env::var("TA_SWARM").ok().and_then(|v| v.parse().ok()).unwrap_or(1500);
                    let arm = ["ARMSTUMP", "ARMFLASH", "ARMPW", "ARMHAM", "ARMROCK", "ARMJETH", "ARMSAM", "ARMBULL"];
                    let core = ["CORAK", "CORTHUD", "CORRAID", "CORGATOR", "CORSTORM", "CORCRASH", "CORLEVLR", "CORREAP"];
                    let (w, h) = self.game.sim.world.size_px();
                    let (cx, cz) = (w.to_f32() / 2.0, h.to_f32() / 2.0);
                    let cols = ((per_side as f32).sqrt() * 0.8).ceil().max(1.0) as usize;
                    let spacing = 36.0;
                    let mut ids = [Vec::new(), Vec::new()];
                    for (side, names) in [arm, core].iter().enumerate() {
                        let types: Vec<usize> = names.iter().filter_map(|n| self.game.sim.types.id(n)).collect();
                        if types.is_empty() {
                            continue;
                        }
                        let sign = if side == 0 { -1.0 } else { 1.0 };
                        for i in 0..per_side {
                            let (row, col) = (i / cols, i % cols);
                            let x = cx + sign * (700.0 + row as f32 * spacing);
                            let z = cz + (col as f32 - cols as f32 / 2.0) * spacing;
                            let p = ta_sim::FxVec3::new(ta_sim::Fx::from_f32(x), ta_sim::Fx::ZERO, ta_sim::Fx::from_f32(z));
                            let heading = if side == 0 { 16384 } else { -16384 };
                            if let Some(id) = self.game.sim.spawn_unit(types[i % types.len()], side as u8, p, heading, true) {
                                ids[side].push(id);
                            }
                        }
                    }
                    let spectator = self.game.spectator;
                    self.game.spectator = false;
                    for (side, list) in ids.iter().enumerate() {
                        let dest_x = if side == 0 { cx + 900.0 } else { cx - 900.0 };
                        let dest = ta_sim::FxVec3::new(ta_sim::Fx::from_f32(dest_x), ta_sim::Fx::ZERO, ta_sim::Fx::from_f32(cz));
                        self.game.issue(ta_sim::Command::Order { player: side as u8, units: list.clone(), order: ta_sim::Order::Fight(dest), queue: false });
                    }
                    self.game.spectator = spectator;
                    self.camera.focus = Vec3::new(cx, 100.0, cz);
                    self.camera.zoom = 0.45;
                    self.game.scene.after_tick(&self.game.sim);
                    log::info!("swarm: {} + {} units", ids[0].len(), ids[1].len());
                }
                DemoAction::Siege => self.siege_demo(),
                DemoAction::SiegeNuke => self.siege_nuke(),
                DemoAction::War => self.war_demo(),
                DemoAction::RideNearest => self.ride_nearest(),
                DemoAction::OpenRadial => self.open_radial_demo(),
                DemoAction::WarNuke => self.war_nuke(),
                DemoAction::SiegeRide => self.siege_ride(),
                DemoAction::Battle(cx, cz) => {
                    let arm = ["ARMSTUMP", "ARMSTUMP", "ARMFLASH", "ARMFLASH", "ARMPW", "ARMPW", "ARMROCK", "ARMHAM", "ARMSAM", "ARMJETH", "ARMBULL", "ARMMART"];
                    let core = ["CORAK", "CORAK", "CORTHUD", "CORTHUD", "CORRAID", "CORRAID", "CORGATOR", "CORLEVLR", "CORMIST", "CORSTORM", "CORREAP", "CORWOLV"];
                    let mut ids = [Vec::new(), Vec::new()];
                    for (side, names) in [arm, core].iter().enumerate() {
                        let x = if side == 0 { cx - 200.0 } else { cx + 200.0 };
                        let heading = if side == 0 { 16384 } else { -16384 };
                        for (i, name) in names.iter().enumerate() {
                            let Some(t) = self.game.sim.types.id(name) else { continue };
                            let (row, col) = (i / 4, i % 4);
                            let p = ta_sim::FxVec3::new(
                                ta_sim::Fx::from_f32(x + if side == 0 { -(row as f32) * 44.0 } else { row as f32 * 44.0 }),
                                ta_sim::Fx::ZERO,
                                ta_sim::Fx::from_f32(cz - 66.0 + col as f32 * 44.0),
                            );
                            if let Some(id) = self.game.sim.spawn_unit(t, side as u8, p, heading, true) {
                                ids[side].push(id);
                            }
                        }
                    }
                    // Scripted battles bypass the local-player restriction on orders.
                    let spectator = self.game.spectator;
                    self.game.spectator = false;
                    for (side, list) in ids.iter().enumerate() {
                        let dest_x = if side == 0 { cx + 120.0 } else { cx - 120.0 };
                        for &id in list {
                            let z = self.game.sim.unit(id).map_or(cz, |u| u.pos.z.to_f32());
                            let dest = ta_sim::FxVec3::new(ta_sim::Fx::from_f32(dest_x), ta_sim::Fx::ZERO, ta_sim::Fx::from_f32(z));
                            self.game.issue(ta_sim::Command::Order { player: side as u8, units: vec![id], order: ta_sim::Order::Move(dest), queue: false });
                        }
                    }
                    self.game.spectator = spectator;
                    self.camera.focus = Vec3::new(cx, 86.0, cz + 40.0);
                    self.game.scene.after_tick(&self.game.sim);
                }
            }
            // `--camera` wins over where a demo puts the camera.
            if let Some(spec) = &self.camera_spec {
                apply_camera_spec(&mut self.camera, spec);
            }
        }
        // A staged scene's orders and events due by the next tick.
        if let Some(run) = self.stage.as_mut() {
            run.before_update(&mut self.game);
        }
    }

    /// `--benchmark`: the frame measured (`cpu_ms`: its work before drawing), and what the run
    /// asks for next done: the battle copied, a level of the settings test played from the copy,
    /// the next scene loaded, or the report written and the game closed.
    fn benchmark_frame(&mut self, event_loop: &ActiveEventLoop, cpu_ms: f32) -> Result<()> {
        use crate::benchmark::Next;
        let Some(bench) = self.benchmark.as_mut() else { return Ok(()) };
        let gpu_ms = self.renderer.gpu_timings().iter().find(|p| p.pass == "frame").map(|p| p.ms);
        let gpu_ms = bench.gpu_sample(self.renderer.gpu_timings_frame(), gpu_ms);
        // The scene's picture, read back on this frame (asked for the frame before).
        if let Some(shot) = self.renderer.take_screenshot() {
            bench.picture(shot);
        }
        // (Each pass's time with the frame's, when the graphics card's timing is new.)
        let passes = gpu_ms.is_some().then(|| self.renderer.gpu_timings());
        let seconds = self.stage.as_ref().filter(|_| self.screen == Screen::Playing).map(|r| r.seconds(&self.game));
        let frame = ta_config::benchmark::Frame { t: 0.0, ms: self.frame_ms, cpu_ms, gpu_ms, ticks: self.game.frame_ticks, tick_ms: self.game.frame_tick_ms, units: self.game.sim.unit_count() as u32 };
        let next = bench.frame(frame, passes, seconds);
        match next {
            Next::On => {}
            Next::Picture => self.renderer.request_screenshot(),
            Next::Copy => {
                if let Some(run) = &self.stage {
                    let copy = crate::benchmark::Copy { sim: self.game.sim.clone(), ais: self.game.ais().to_vec(), run: run.clone() };
                    bench.keep(copy, seconds.unwrap_or(crate::benchmark::TEST_FROM));
                }
            }
            Next::Level(_) | Next::Reference | Next::Effect(_) => self.pending_pass = Some(next),
            Next::Load(n) => {
                // (After the settings test, the player's settings back.)
                if let Some(saved) = bench.saved.take() {
                    { let renderer = &mut *self.renderer; apply_effect_settings(&saved, &mut renderer.lighting, &mut renderer.post.settings); }
                    self.settings.graphics = saved;
                }
                self.pending = Some(Transition::Benchmark(n));
            }
            Next::Finish => {
                let hw = ta_config::hardware::Hardware::detect();
                let info = self.renderer.gpu.adapter.get_info();
                let system = ta_config::benchmark::System {
                    gpu: info.name.clone(),
                    backend: format!("{:?}", info.backend),
                    driver: format!("{} {}", info.driver, info.driver_info).trim().to_string(),
                    cpu: hw.cpu.clone(),
                    threads: hw.threads as u32,
                    ram_mb: hw.ram_mb,
                    vram_mb: hw.vram_mb,
                };
                let size = self.window.inner_size();
                let refresh = self.window.current_monitor().and_then(|m| m.refresh_rate_millihertz()).map_or(hw.refresh_hz as f32, |mhz| mhz as f32 / 1000.0);
                let skin = self.renderer.models.as_ref().map_or("reborn", |m| m.textures.unit_skin_id());
                let enemy_skin = self.renderer.models.as_ref().map_or("reborn", |m| m.textures.enemy_unit_skin_id());
                let report = bench.report(system, &self.settings.graphics, skin, enemy_skin, &self.settings.gameplay, (size.width, size.height), refresh, self.renderer.msaa_samples());
                ta_config::write_atomic(&bench.out, &serde_json::to_string(&report)?)?;
                let s = report.scores();
                log::info!("benchmark: {} (processor {}, graphics {}, memory {}): report written to {}", s.overall, s.cpu, s.gpu, s.ram, bench.out.display());
                event_loop.exit();
            }
        }
        Ok(())
    }

    /// Rewind a short settings pass while its loading frame still covers the window.
    /// This is an in-memory copy only: no map decoding, shader compilation or I/O.
    fn apply_benchmark_pass(&mut self, next: crate::benchmark::Next) {
        use crate::benchmark::Next;
        let Some(bench) = self.benchmark.as_mut() else { return };
        // The level's settings (the antialiasing as the game started: it changes only
        // then; the reference pass High's, drawing about 1080p's pixels), and the copied
        // battle again.
        let saved = bench.saved.get_or_insert_with(|| self.settings.graphics.clone()).clone();
        let mut g = saved.clone();
        match next {
            Next::Level(level) => level.apply(&mut g),
            Next::Effect(effect) => effect.apply(&mut g),
            _ => {
                let size = self.window.inner_size();
                ta_config::Preset::High.apply(&mut g);
                g.render_scale = ta_config::benchmark::reference_scale(size.width, size.height);
            }
        }
        g.msaa = saved.msaa;
        // (The ground in the wind is a matter of taste, outside the levels: every pass has it, so
        // that a player who turned it off scores as everyone does.)
        g.ground_life = true;
        { let renderer = &mut *self.renderer; apply_effect_settings(&g, &mut renderer.lighting, &mut renderer.post.settings); }
        // (Ray tracing is the renderer's own: off for its pass of the effects test, and for the
        // reference pass, whose time scores every graphics card alike.)
        if matches!(next, Next::Effect(ta_config::benchmark::Effect::RayTracing) | Next::Reference) {
            self.renderer.lighting.ray_tracing = false;
        }
        self.settings.graphics = g;
        if let Some(copy) = bench.copy() {
            self.game.restore(&copy.sim, &copy.ais, copy.sim.tick, None, &mut self.renderer);
            self.stage = Some(copy.run.clone());
        }
    }

    /// `--demo stage` (and the benchmark's scenes): the scene set up (see `stage.rs`), and the
    /// camera where it starts.
    fn stage_begin(&mut self) {
        let stage = match &self.benchmark {
            Some(b) => b.stage(),
            None => match crate::stage::current() {
                Some(stage) => stage,
                None => return,
            },
        };
        let texts = stage.texts.clone();
        let mut run = self.loaded_stage.take().unwrap_or_else(|| crate::stage::StageRun::begin(stage, &mut self.game));
        self.renderer.lighting.moon_dir = stage.moon_dir();
        run.place_camera(&self.game, &mut self.camera);
        self.tip = None;
        self.stage = Some(run);
        self.add_stage_texts(&texts);
    }

    /// The scene's `[[text]]`s put in the world (their times the scene's, from now).
    fn add_stage_texts(&mut self, texts: &[crate::stage::TextEntry]) {
        self.world_text.clear();
        if texts.is_empty() {
            return;
        }
        let start = crate::scene::game_seconds(self.game.sim.tick, 0.0) as f32;
        let (w, h) = self.game.sim.world.size_px();
        let middle = (w.to_f32() / 2.0, h.to_f32() / 2.0);
        for entry in texts {
            let Some(spec) = entry.spec(start, middle) else { continue };
            let text = match entry.picture {
                // (The emblem in place of words, `size` world units across.)
                Some(crate::stage::TextPicture::Emblem) => self.world_text.make_emblem(&mut self.renderer, spec.at, spec.heading, spec.size, spec.look, spec.born, spec.life, spec.floor),
                None => Some(self.world_text.make(&mut self.renderer, &spec)),
            };
            if let Some(text) = text {
                self.world_text.add(text);
            }
        }
    }

    /// The menu's film's camera this frame (on the menu, no showroom up), if it plays: its lens's
    /// focus. Off the menu (and the intro before it) the film stops: its battle is gone.
    fn menu_film_camera(&mut self, dt: f32) -> Option<f32> {
        if !matches!(self.screen, Screen::Menu | Screen::Intro) {
            self.menu_film = None;
            return None;
        }
        if self.screen == Screen::Intro || self.showroom.is_some() {
            return None;
        }
        let film = self.menu_film.as_mut()?;
        let viewport = self.camera.viewport;
        let ppp = self.egui.ctx.pixels_per_point();
        // The title menu's room on the left: the picture's middle halfway along what it leaves.
        let shift = crate::menu::title_column_right(viewport.y / ppp) * ppp * 0.5;
        let focused = self.window.has_focus() || self.exit_after_screenshot;
        let pointer = (focused && self.input.cursor_inside).then(|| self.input.mouse / viewport.max(Vec2::ONE) * 2.0 - Vec2::ONE);
        let still = self.settings.gameplay.reduced_motion;
        film.camera(&self.game, &mut self.camera, pointer, shift, still, dt)
    }

    /// The title menu left alone a while steps aside and the film plays on its own; any key,
    /// click, wheel or move of the pointer (after a moment's grace, for the pointer that just
    /// clicked WATCH) brings it back.
    fn watch_menu_film(&mut self, dt: f32) {
        let on_title = self.menu.page == crate::menu::Page::Title && !self.options_open && !self.jukebox.open && self.menu.record_error.is_none();
        let pressed = !self.input.pressed_keys().is_empty() || !self.input.buttons_pressed.is_empty() || self.input.wheel != 0.0 || self.input.pad_pan != Vec2::ZERO
            || self.egui.ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::PointerButton { pressed: true, .. })));
        let moved = self.input.mouse_delta.length() > 2.0;
        let Some(film) = self.menu_film.as_mut() else { return };
        if film.watching {
            if pressed || moved && film.watched() > crate::backdrop::WATCH_GRACE || !on_title {
                film.watching = false;
                self.menu_film_idle = 0.0;
            }
            return;
        }
        if pressed || moved || !on_title {
            self.menu_film_idle = 0.0;
        } else {
            self.menu_film_idle += dt;
            if self.menu_film_idle > crate::backdrop::WATCH_AFTER {
                film.watching = true;
            }
        }
    }

    /// Cinematic camera for the menu backdrop: drift towards the action and orbit slowly.
    fn update_attract_camera(&mut self, dt: f32) {
        // Establish the initial view, then keep the menu's camera still.
        if self.settings.gameplay.reduced_motion && !self.attract.snap { return; }
        let dt = if self.settings.gameplay.reduced_motion { 0.0 } else { dt };
        self.attract.retarget_in -= dt;
        if self.attract.retarget_in <= 0.0 {
            self.attract.retarget_in = 9.0;
            // Head for the unit nearest the average of all fighting (units with targets).
            let mut sum = Vec3::ZERO;
            let mut n = 0.0;
            for u in self.game.sim.units() {
                if u.weapons.iter().any(|w| w.target.is_some()) || n == 0.0 {
                    sum += Vec3::from_array(u.pos.to_f32());
                    n += 1.0;
                }
            }
            if n > 0.0 {
                self.attract.target = sum / n;
            }
        }
        // Keep well inside the map so the backdrop never shows the void past its edges.
        let (mw, mh) = (self.game.map.width_px() as f32, self.game.map.height_px() as f32);
        let margin_x = (mw * 0.5 - 1.0).min(700.0);
        let margin_z = (mh * 0.5 - 1.0).min(900.0);
        let target = Vec3::new(self.attract.target.x.clamp(margin_x, mw - margin_x), 0.0, self.attract.target.z.clamp(margin_z, mh - margin_z * 0.6));
        let cam = &mut self.camera;
        let k = if std::mem::take(&mut self.attract.snap) { 1.0 } else { 1.0 - (-dt * 0.35).exp() };
        cam.focus.x += (target.x - cam.focus.x) * k;
        cam.focus.z += (target.z - cam.focus.z) * k;
        cam.yaw += dt * 0.035;
        // A slow cinematic cycle, low over the battle: the camera sinks towards the horizon
        // (the sky and the land beyond the map in view), then rises a little over the fighting.
        self.attract.clock += dt;
        let swing = (self.attract.clock * std::f32::consts::TAU / 75.0).sin();
        let pitch = ATTRACT_PITCH - 0.17 * swing;
        let zoom = 1.25 - 0.35 * (1.0 - swing).min(1.0) * 0.5;
        cam.pitch += (pitch - cam.pitch) * k;
        cam.zoom += (zoom - cam.zoom) * k;
    }

    fn update_camera(&mut self, dt: f32) {
        for (i, key) in [KeyCode::F6, KeyCode::F7, KeyCode::F8, KeyCode::F9].into_iter().enumerate() {
            if !self.input.pressed(key) {
                continue;
            }
            let cam = &mut self.camera;
            if self.input.modifiers.control_key() {
                self.bookmarks[i] = Some((cam.focus, cam.zoom, cam.yaw, cam.pitch));
                self.game.message(ta_ui::i18n::tf("Camera position {n} saved (F{f} recalls it)", &[("n", &(i + 1)), ("f", &(i + 6))]));
            } else if let Some((focus, zoom, yaw, pitch)) = self.bookmarks[i] {
                cam.focus = focus;
                cam.zoom = zoom;
                cam.yaw = yaw;
                cam.pitch = pitch;
            }
        }
        // Photo mode frees the camera: nearly level with the ground, closer and farther (as near
        // and as far whatever its lens: a long one magnifies as much again).
        let lens = 15f32.to_radians().tan() / (self.camera.fov_y * 0.5).tan().max(1e-3);
        let map = self.renderer.terrain.as_ref().map(|t| t.field.size_px());
        let held = self.players_view();
        let (min_pitch, max_pitch, zooms) = if self.photo.is_some() {
            (0.02, 1.55, (0.04 * lens, 24.0 * lens))
        } else if held {
            (MIN_PITCH, 1.52, (zoom_out_limit(self.camera.viewport, map), MAX_ZOOM))
        } else {
            (MIN_PITCH, 1.52, (FARTHEST_ZOOM_1080, MAX_ZOOM))
        };
        // (Held there whatever moved it: a window made larger, another map.)
        if held {
            self.camera.zoom = self.camera.zoom.clamp(zooms.0, zooms.1);
        }
        let input = &self.input;
        let keys = &self.game.keys;
        let cam = &mut self.camera;
        let mut pan = Vec2::ZERO;
        if keys.key_held(input, "scroll_up") {
            pan.y -= 1.0;
        }
        if keys.key_held(input, "scroll_down") {
            pan.y += 1.0;
        }
        if keys.key_held(input, "scroll_left") {
            pan.x -= 1.0;
        }
        if keys.key_held(input, "scroll_right") {
            pan.x += 1.0;
        }
        if self.game.radial.is_none() {
            pan += input.pad_pan;
        }
        let fast = if input.modifiers.shift_key() { 2.5 } else { 1.0 };
        let scroll = 1100.0 * fast * self.settings.gameplay.scroll_speed.clamp(0.1, 10.0) * dt;
        if input.modifiers.control_key() && cam.mode == CameraMode::Perspective {
            // Ctrl+arrows orbit and tilt (the classic view never turns: they scroll it).
            cam.yaw += pan.x * 1.5 * dt;
            cam.pitch = (cam.pitch - pan.y * 0.8 * dt).clamp(min_pitch, max_pitch);
        } else {
            cam.pan_pixels(pan * scroll);
        }
        // The screen's edges: the cursor within a few pixels of one (inside the window, which
        // has the keyboard) scrolls that way. (Not in captures: where the mouse happens to be
        // must not move their view.)
        if self.settings.gameplay.edge_scroll && input.cursor_inside && self.window.has_focus() && self.screenshot_path.is_none() {
            const EDGE: f32 = 3.0;
            let (m, size) = (input.mouse, cam.viewport);
            let edge = Vec2::new(
                if m.x <= EDGE { -1.0 } else if m.x >= size.x - 1.0 - EDGE { 1.0 } else { 0.0 },
                if m.y <= EDGE { -1.0 } else if m.y >= size.y - 1.0 - EDGE { 1.0 } else { 0.0 },
            );
            cam.pan_pixels(edge * scroll);
            // (Scrolling the view takes it back from a unit followed or the action camera.)
            if edge != Vec2::ZERO {
                self.game.follow = None;
                self.game.action_camera = false;
            }
        }

        // (Ctrl+right-drag draws a formation when the right button gives orders.)
        let right_orders = self.game.order_button == ta_config::OrderButton::Right;
        let drawing = self.photo.is_none() && (self.game.formation.is_some() || right_orders && input.modifiers.control_key() && !self.game.selection.is_empty());
        let orbiting = input.button_held(MouseButtonId::Right) && !drawing && self.game.radial.is_none() || (input.button_held(MouseButtonId::Middle) && input.modifiers.alt_key());
        if orbiting && cam.mode == CameraMode::Perspective {
            cam.yaw += input.mouse_delta.x * 0.006;
            cam.pitch = (cam.pitch - input.mouse_delta.y * 0.004).clamp(min_pitch, max_pitch);
        } else if input.button_held(MouseButtonId::Middle) {
            cam.pan_pixels(-input.mouse_delta);
        }
        if cam.mode == CameraMode::Perspective && keys.key_held(input, "tilt_up") {
            cam.pitch = (cam.pitch + 0.8 * dt).min(max_pitch);
        }
        if cam.mode == CameraMode::Perspective && keys.key_held(input, "tilt_down") {
            cam.pitch = (cam.pitch - 0.8 * dt).max(min_pitch);
        }
        if keys.pressed(input, "reset_view") {
            cam.yaw = 0.0;
            cam.pitch = ta_render::camera::TA_ELEVATION;
        }
        // The other camera, kept as the player's choice (the options show it).
        if keys.pressed(input, "camera_style") {
            let style = match cam.mode {
                CameraMode::Classic => ta_config::CameraStyle::Modern,
                CameraMode::Perspective => ta_config::CameraStyle::Classic,
            };
            set_camera_style(cam, style);
            self.settings.graphics.camera = style;
            self.save_settings();
            let key = self.game.keys.text("camera_style");
            let args: [(&str, &dyn std::fmt::Display); 1] = [("key", &key)];
            self.game.message(match style {
                ta_config::CameraStyle::Modern => ta_ui::i18n::tf("Modern 3D camera ({key}: classic top-down)", &args),
                ta_config::CameraStyle::Classic => ta_ui::i18n::tf("Classic top-down camera ({key}: modern 3D)", &args),
            });
        }

        // Zoom towards the cursor: keep the ground point under the mouse fixed.
        if self.input.wheel != 0.0 {
            let mouse = self.input.mouse;
            let before = self.ground_under(mouse);
            let cam = &mut self.camera;
            cam.zoom = (cam.zoom * 1.15f32.powf(self.input.wheel)).clamp(zooms.0, zooms.1);
            if let (Some(b), Some(a)) = (before, self.ground_under(mouse)) {
                let cam = &mut self.camera;
                cam.focus.x += b.x - a.x;
                cam.focus.z += b.z - a.z;
            }
        }
    }

    /// The battle decided: the camera glides to where it was decided, and (a perspective one)
    /// turns slowly round it while the result is up.
    fn result_camera(&mut self, dt: f32) {
        if self.game.outcome.is_none() {
            self.result_glide = None;
            return;
        }
        if let Some(target) = self.result_glide {
            let k = 1.0 - (-dt * 1.4).exp();
            let cam = &mut self.camera;
            cam.focus.x += (target.x - cam.focus.x) * k;
            cam.focus.z += (target.z - cam.focus.z) * k;
            if (target.x - cam.focus.x).hypot(target.z - cam.focus.z) < 2.0 {
                self.result_glide = None;
            }
        }
        if self.camera.mode == CameraMode::Perspective && !self.settings.gameplay.reduced_motion {
            self.camera.yaw += dt * 0.025;
        }
    }

    /// Whether the camera is the player's own in a battle, held to the map: zoomed out no further
    /// than the whole of it (`zoom_out_limit`), kept to it when zoomed out (`view_margins`). Not a
    /// staged scene's, a film's or the highlights' camera, which frame their own pictures, nor the
    /// photographer's, which goes where it likes.
    fn players_view(&self) -> bool {
        self.screen == Screen::Playing && self.stage.is_none() && self.film.is_none() && self.reel.is_none() && self.photo.is_none()
    }

    fn clamp_camera(&mut self, dt: f32) {
        // A jump to look at something (the Commander, an alert, the next unit) lets go of a
        // unit followed and of the action camera, which would pull the view back.
        if let Some(p) = self.game.focus_request.take() {
            self.camera.focus.x = p.x;
            self.camera.focus.z = p.z;
            self.game.follow = None;
            self.game.action_camera = false;
        }
        // Following a unit: the camera glides after it; scrolling the view lets go. (Not in
        // photo mode: the camera is the photographer's.)
        if let Some(id) = self.game.follow.filter(|_| self.photo.is_none()) {
            let scrolled = ["scroll_up", "scroll_down", "scroll_left", "scroll_right"].iter().any(|a| self.game.keys.key_held(&self.input, a))
                || self.input.button_held(MouseButtonId::Middle)
                || self.input.pad_pan != Vec2::ZERO;
            match self.game.scene.unit_position(id, self.game.alpha()) {
                Some(p) if !scrolled => {
                    let k = 1.0 - (-dt * 6.0).exp();
                    self.camera.focus.x += (p.x - self.camera.focus.x) * k;
                    self.camera.focus.z += (p.z - self.camera.focus.z) * k;
                }
                _ => self.game.follow = None,
            }
        }
        // Spectators and replays: the action camera, a director. It holds on a fight for a few
        // seconds at least, moves to another when that one is half as busy again or its own dies
        // down, cutting to one far off and gliding to one near; in the cast it frames the fight
        // too. Scrolling the view (or, in the cast, zooming it) takes back control.
        if self.game.action_camera && self.photo.is_none() && (self.game.spectator || self.game.playback.is_some()) {
            let scrolled = ["scroll_up", "scroll_down", "scroll_left", "scroll_right"].iter().any(|a| self.game.keys.key_held(&self.input, a))
                || self.input.button_held(MouseButtonId::Middle)
                || self.game.cast && self.input.wheel != 0.0;
            if scrolled {
                self.game.action_camera = false;
            } else {
                self.game.action_retarget -= dt;
                self.game.action_held += dt;
                if self.game.action_retarget <= 0.0 {
                    self.game.action_retarget = 2.0;
                    let busy = self.game.action_target.map_or(0.0, |p| self.game.firing_near(p) as f32);
                    if let Some(best) = self.game.fights().first().copied() {
                        let held = self.game.action_held;
                        let elsewhere = self.game.action_target.is_none_or(|p| p.distance(best.at) > 700.0);
                        let switch = self.game.action_target.is_none() || elsewhere && (held >= 5.0 && best.firing as f32 > busy.max(1.0) * 1.5 || held >= 3.0 && busy < 2.0);
                        if switch {
                            if self.game.action_target.is_none_or(|p| p.distance(best.at) > 1600.0) {
                                self.camera.focus.x = best.at.x;
                                self.camera.focus.z = best.at.z;
                            }
                            self.game.action_target = Some(best.at);
                            self.game.action_spread = best.spread;
                            self.game.action_held = 0.0;
                        } else if !elsewhere {
                            // (The same fight, moved on.)
                            self.game.action_target = Some(best.at);
                            self.game.action_spread += (best.spread - self.game.action_spread) * 0.5;
                        }
                    }
                }
                if let Some(p) = self.game.action_target {
                    // (In the cast, the fight in the middle of the picture under the scoreboard.)
                    let p = match self.hud.board_share().filter(|_| self.game.cast) {
                        Some(share) => {
                            let yaw = if self.camera.mode == CameraMode::Classic { 0.0 } else { self.camera.yaw };
                            let below = share * self.camera.viewport.y / 2.0;
                            p + Vec3::new(yaw.sin(), 0.0, -yaw.cos()) * (below / self.camera.zoom.max(0.1))
                        }
                        None => p,
                    };
                    let k = 1.0 - (-dt * 0.9).exp();
                    self.camera.focus.x += (p.x - self.camera.focus.x) * k;
                    self.camera.focus.z += (p.z - self.camera.focus.z) * k;
                    if self.game.cast || self.film.is_some() {
                        let framing = self.film.as_ref().map_or(crate::film::Shot::Director, |f| f.settings.shot).framing();
                        let want = crate::highlights::zoom_for(&self.camera, self.game.action_spread * framing);
                        self.camera.zoom += (want - self.camera.zoom) * (1.0 - (-dt * 0.6).exp());
                    }
                }
            }
        }
        let now = self.game.sim.tick as f32 / 30.0;
        self.game.pings.retain(|p| p.age(now) < 1.0);
        // Blast shake: a quick jitter of the focus that dies away (not with TA's +noshake, which
        // lasts a battle, nor with the player's SCREEN SHAKE off or reduced motion).
        if self.game.view_cheats.no_shake || self.settings.gameplay.reduced_motion || !self.settings.gameplay.screen_shake {
            self.game.effects.shake = 0.0;
        }
        let shake = &mut self.game.effects.shake;
        if *shake > 0.05 {
            let t = self.game.sim.tick as f32 * 1.7 + self.frames as f32 * 0.9;
            self.camera.focus.x += (t * 13.1).sin() * *shake * 0.5;
            self.camera.focus.z += (t * 17.3).cos() * *shake * 0.5;
            *shake *= (1.0 - dt * 2.5).max(0.0);
        }
        // The part of the screen the interface leaves to the battle (the whole of it when the
        // interface is hidden, and away from a battle): the camera centres on its middle. (The
        // showroom frames its unit itself, its focus on the unit's middle.)
        let showroom = self.screen == Screen::Menu && self.showroom.is_some();
        // (Behind the battle's result, with the interface stepped aside: the whole window's,
        // eased there.)
        let result = self.screen == Screen::Playing && self.game.outcome.is_some();
        let free = free_view(self.game.battle_view.filter(|_| self.screen == Screen::Playing && !hud_hidden() && self.photo.is_none() && !result), self.camera.viewport);
        let offset = (free.0 + free.1) * 0.5 - self.camera.viewport * 0.5;
        if result {
            self.camera.view_offset += (offset - self.camera.view_offset) * (1.0 - (-dt * 3.0).exp());
        } else if !showroom {
            self.camera.view_offset = offset;
        }
        let held = self.players_view();
        if let Some(t) = &self.renderer.terrain {
            let size = t.field.size_px();
            let cam = &mut self.camera;
            if cam.mode == CameraMode::Classic {
                clamp_classic_view(cam, size, free);
            } else if !showroom {
                // (Zoomed out over much of the map, the view keeps to it: see `view_margins`.)
                let margins = if held { view_margins(cam, size) } else { Vec2::ZERO };
                cam.focus.x = cam.focus.x.clamp(margins.x, size.x - margins.x);
                cam.focus.z = cam.focus.z.clamp(margins.y, size.y - margins.y);
                let ground = t.field.sample(cam.focus.x, cam.focus.z).max(t.field.sea_level);
                cam.focus.y += (ground - cam.focus.y) * (1.0 - (-dt * 3.0).exp());
            }
            cam.keep_above_ground(|x, z| camera_ground(&t.field, x, z), dt);
        } else {
            self.camera.lift = 0.0;
        }
    }

    /// Into a unit: the camera, the cursor and the picture's settings are the mode's until it
    /// is left.
    fn enter_perspective(&mut self, id: ta_sim::UnitId, now: f64) {
        let Some(p) = crate::perspective::Perspective::enter(&mut self.game, &self.camera, id, now) else { return };
        self.renderer.set_cockpit(p.model.as_deref());
        self.perspective = Some(p);
        self.perspective_post = Some(self.settled_post());
        self.game.command_mode = None;
        self.game.placement = None;
        // The cursor stays in the window, hidden: the mouse looks round.
        if self.screenshot_path.is_none() {
            use winit::window::CursorGrabMode;
            if self.window.set_cursor_grab(CursorGrabMode::Confined).or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Locked)).is_err() {
                log::debug!("perspective: the cursor could not be held in the window");
            }
            self.window.set_cursor_visible(false);
        }
    }

    /// A frame inside a unit.
    fn perspective_frame(&mut self, dt: f32, now: f64) {
        let Some(p) = self.perspective.as_mut() else { return };
        p.reduced_motion = self.settings.gameplay.reduced_motion;
        p.reduced_flashes = self.settings.gameplay.reduced_flashes;
        // (The clock's hour is the one the world is lit by.)
        p.hours = self.renderer.lighting.time_of_day;
        // (The unit's eye looks out of the middle of the window, cockpit and sights with it.)
        self.camera.view_offset = Vec2::ZERO;
        // The battle decided: out, to its end.
        if self.game.outcome.is_some() {
            p.leave(&mut self.game);
        }
        // (A benchmark's ride looks where the unit looks, whatever the mouse does: every run sees
        // the same.)
        let still = crate::input::Input::default();
        let input = if self.benchmark.is_some() { &still } else { &self.input };
        p.update(&mut self.game, input, &mut self.camera, dt, now);
        self.renderer.set_cockpit_frame(p.cockpit_frame(&self.game, now));
        // A modelled cockpit's instruments, painted onto its glass before the frame is drawn.
        if p.model.is_some()
            && let Some(atlas) = self.renderer.cockpit_instruments()
        {
            let gpu = &self.renderer.gpu;
            // (Not while the card is out of memory: the screens stay dark.)
            if self.instruments.is_none() {
                self.instruments = crate::instruments::InstrumentPainter::new(&gpu.device);
            }
            if let Some(painter) = self.instruments.as_mut() {
                let (game, vfs, p) = (&self.game, &self.assets.vfs, &*p);
                painter.paint(&gpu.device, &gpu.queue, atlas, now, &mut |ctx, pics, portraits, regions| crate::perspective_hud::draw_instruments(p, ctx, game, regions, pics, portraits, vfs, now));
                // The portraits its screens show: the HUD's, once drawn (asked of it meanwhile).
                for key in painter.wanted_portraits() {
                    match self.portrait_views.get(&key) {
                        Some(view) => painter.add_portrait(&gpu.device, key, view),
                        None => self.hud.request_portrait(key),
                    }
                }
            }
        }
        // The night sight: brighter and nearly without colour.
        if let Some(before) = self.perspective_post {
            let mut post = before;
            if p.night_vision {
                post.exposure = before.exposure * 2.6;
                post.saturation = 0.12;
            }
            self.renderer.post.settings = post;
        }
        if p.done {
            let back = p.camera_after(&self.game);
            let viewport = self.camera.viewport;
            self.camera = back;
            self.camera.viewport = viewport;
            self.drop_perspective();
        }
    }

    /// Out of the unit at once, the cockpit and the picture's settings as they were (a new
    /// battle: nothing of the last one's ride stays in it).
    fn drop_perspective(&mut self) {
        if self.perspective.take().is_none() {
            return;
        }
        self.game.scene.hide_near = None;
        self.renderer.set_cockpit(None);
        if let Some(post) = self.perspective_post.take() {
            self.renderer.post.settings = post;
        }
        use winit::window::CursorGrabMode;
        let _ = self.window.set_cursor_grab(CursorGrabMode::None);
        // (A benchmark keeps the pointer hidden throughout.)
        self.window.set_cursor_visible(self.benchmark.is_none());
    }

    /// How the 3D trees and rocks are seen: only by the perspective camera, from a unit's eyes or
    /// low over the battlefield (the eye under about 650 units above the ground it looks at);
    /// from higher up TA's own paintings show. (`TA_FOLIAGE=1` shows them from anywhere the
    /// perspective camera is, to compare.)
    fn foliage_view(&self) -> crate::features::FoliageView {
        // The eye's heights (at 1080 lines) from which the 3D trees and rocks begin to take over
        // from TA's paintings and under which they all have: with medium effects, and with high
        // ones (the models drawn from much further, the overview camera included).
        const FOLIAGE_NEAR: (f32, f32) = (520.0, 780.0);
        const FOLIAGE_FAR: (f32, f32) = (1900.0, 2600.0);
        let cam = &self.camera;
        let eye = cam.eye();
        // The eye's height over what it looks at, as at 1080 lines (at a given zoom a taller
        // window puts the eye higher, but shows the trees as many pixels big); each tree takes
        // over at its own point of the way down (see `FeatureView::update`).
        let h = (eye.y - cam.focus.y) * 1080.0 / cam.viewport.y.max(1.0);
        let within = |(near, far): (f32, f32)| ((far - h) / (far - near)).clamp(0.0, 1.0);
        let (close, close_flat) = if cam.mode != ta_render::CameraMode::Perspective || !self.settings.graphics.trees_3d {
            (0.0, 0.0)
        } else if self.perspective.is_some() || self.screen == Screen::Menu || std::env::var("TA_FOLIAGE").is_ok_and(|v| v == "1") {
            // (The menus' worlds too: their camera swings through every height, and at 4K it
            // would never come down to the trees.)
            (1.0, 1.0)
        } else if matches!(self.settings.graphics.effects, ta_config::Quality::High | ta_config::Quality::Ultra) {
            (within(FOLIAGE_FAR), within(FOLIAGE_NEAR))
        } else {
            (within(FOLIAGE_NEAR), within(FOLIAGE_NEAR))
        };
        let mirror = (self.renderer.lighting.water_reflections != ta_render::WaterReflections::Off && self.renderer.water.is_some()).then(|| self.game.sim.world.sea_level.to_f32());
        let daylight = self.renderer.daylight(cam);
        let strength = daylight.map_or(self.renderer.lighting.shadow_strength, |d| d.shadow);
        let light = (strength > 0.0).then(|| daylight.map_or(self.renderer.lighting.sun_dir, |d| d.light_dir));
        crate::features::FoliageView { eye, close, close_flat, scale: cam.viewport.y / (2.0 * (cam.fov_y * 0.5).tan()), planes: crate::features::clip_planes(cam.view_proj()), mirror, light }
    }

    /// `--demo siege`: thousands of Peewees (`TA_SIEGE`, 3000) storm a Core defence line built
    /// in depth across the middle of the map, north to south: Light Laser Towers in front, Gaat
    /// Guns, Toasters and Punishers behind them, Doomsday Machines at the back, and fusion plants
    /// and storage behind those to power the lasers. The Peewees set off in a block to the west
    /// and fight their way east, each column straight ahead.
    fn siege_demo(&mut self) {
        use ta_sim::{Fx, FxVec3};
        let peewees: usize = std::env::var("TA_SIEGE").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
        let (w, h) = self.game.sim.world.size_px();
        let (w, h) = (w.to_f32(), h.to_f32());
        let (cx, cz) = (w / 2.0, h / 2.0);
        // The line east of the middle (the depth of it and the silo behind need 1,300 px), the
        // horde's block filling the west: on Gasplant Plain 98 columns of up to 53 Peewees.
        let half = ((h - 500.0) / 2.0).clamp(300.0, 1700.0);
        let front = (cx + 600.0).min(w - 1300.0).max(cx);
        // A clear field: the plants and rocks over the whole battle go (they turn the horde's
        // routes into a few lanes and hide it), metal deposits stay.
        let field = FxVec3::new(Fx::from_f32(cx), Fx::ZERO, Fx::from_f32(cz));
        self.game.sim.clear_features(field, Fx::from_f32(w.max(h)));
        let spectator = self.game.spectator;
        self.game.spectator = false;
        // The line: (unit, distance behind the front, spacing along it, offset of the first).
        // (Each within its reach of where the Peewees stop to shoot the front row, 180 px out:
        // the Doomsday Machines' 600, the Toasters' 885; the Punishers' 1,200 shell the march.)
        let rows: [(&str, f32, f32, f32); 7] = [
            ("CORLLT", 0.0, 72.0, 0.0),
            ("CORHLT", 80.0, 120.0, 40.0),
            ("CORTOAST", 170.0, 200.0, 100.0),
            ("CORDOOM", 270.0, 300.0, 150.0),
            ("CORPUN", 400.0, 210.0, 0.0),
            ("CORFUS", 620.0, 520.0, 0.0),
            ("CORESTOR", 760.0, 520.0, 260.0),
        ];
        let mut towers = 0;
        for (name, back, step, first) in rows {
            let Some(t) = self.game.sim.types.id(name) else { continue };
            let mut z = cz - half + first;
            while z <= cz + half {
                let want = FxVec3::new(Fx::from_f32(front + back), Fx::ZERO, Fx::from_f32(z));
                if let Some(p) = self.game.sim.find_build_site(t, want, 6) && self.game.sim.spawn_unit(t, 1, p, -16384, true).is_some() {
                    towers += 1;
                }
                z += step;
            }
        }
        // The horde, in columns across the whole front.
        let mut columns: Vec<Vec<ta_sim::UnitId>> = Vec::new();
        if let Some(t) = self.game.sim.types.id("ARMPW") {
            let spacing = 34.0;
            let cols = ((2.0 * half / spacing) as usize).max(1);
            columns = vec![Vec::new(); cols];
            for i in 0..peewees {
                let (row, col) = (i / cols, i % cols);
                let x = front - 1150.0 - row as f32 * spacing;
                let z = cz - half + (col as f32 + 0.5) * spacing;
                if x < 40.0 {
                    break;
                }
                let p = FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z));
                if let Some(id) = self.game.sim.spawn_unit(t, 0, p, 16384, true) {
                    columns[col].push(id);
                }
            }
            // Each column straight at the line, then on through it: routes planned through the
            // line from the start would all bend for its few gaps, and the horde would funnel.
            // Under the rule "modern movement" each column goes as a formation keeping its file,
            // its head brought up to the line (one in rows for each of a hundred columns side by
            // side would have them all forming up on the same ground first).
            let deep_x = (front + 1400.0).min(w - 60.0);
            let head = front - 1150.0;
            for (col, units) in columns.iter().enumerate().filter(|(_, u)| !u.is_empty()) {
                let z = Fx::from_f32(cz - half + (col as f32 + 0.5) * spacing);
                let middle = units.iter().filter_map(|&u| self.game.sim.unit(u)).map(|u| u.pos.x.to_f32()).sum::<f32>() / units.len() as f32;
                for (x, queue) in [(front - 30.0, false), (deep_x, true)] {
                    let command = if self.game.sim.group_formations {
                        let order = ta_sim::Order::Fight(FxVec3::new(Fx::from_f32(middle + x - head), Fx::ZERO, z));
                        ta_sim::Command::Formation { player: 0, units: units.clone(), order, queue, shape: ta_sim::formation::Shape::Keep }
                    } else {
                        ta_sim::Command::Order { player: 0, units: units.clone(), order: ta_sim::Order::Fight(FxVec3::new(Fx::from_f32(x), Fx::ZERO, z)), queue }
                    };
                    self.game.issue(command);
                }
            }
        }
        // A silo with a missile ready, far behind the line (`TA_SIEGE_NUKE_BY=arm` puts it
        // behind the horde instead).
        let arm_nukes = std::env::var("TA_SIEGE_NUKE_BY").is_ok_and(|v| v == "arm");
        let (silo_type, silo_owner, silo_x) = if arm_nukes { ("ARMSILO", 0u8, 200.0) } else { ("CORSILO", 1u8, (front + 1050.0).min(w - 120.0)) };
        if let Some(t) = self.game.sim.types.id(silo_type) {
            let want = FxVec3::new(Fx::from_f32(silo_x), Fx::ZERO, Fx::from_f32(cz + 300.0));
            if let Some(p) = self.game.sim.find_build_site(t, want, 12) && let Some(silo) = self.game.sim.spawn_unit(t, silo_owner, p, -16384, true) {
                self.game.sim.grant_stockpile(silo, 1);
            }
        }
        self.siege_middle = Some(Vec3::new(front, 0.0, cz));
        self.game.spectator = spectator;
        self.camera.focus = Vec3::new(front - 300.0, 100.0, cz);
        self.camera.zoom = 0.5;
        self.game.scene.after_tick(&self.game.sim);
        log::info!("siege: {} Peewees against {towers} buildings", columns.iter().map(Vec::len).sum::<usize>());
    }

    /// The siege's silo fires at the middle of the line.
    fn siege_nuke(&mut self) {
        let Some(m) = self.siege_middle else { return };
        let silo = self.game.sim.units().find(|u| matches!(self.game.sim.unit_type(u).def.name.as_str(), "ARMSILO" | "CORSILO") && u.stockpile > 0).map(|u| (u.id, u.owner));
        let Some((silo, owner)) = silo else { return };
        let target = ta_sim::FxVec3::new(ta_sim::Fx::from_f32(m.x - 40.0), ta_sim::Fx::ZERO, ta_sim::Fx::from_f32(m.z));
        let spectator = self.game.spectator;
        self.game.spectator = false;
        self.game.issue(ta_sim::Command::Order { player: owner, units: vec![silo], order: ta_sim::Order::ManualFire(ta_sim::Target::Ground(target)), queue: false });
        self.game.spectator = spectator;
        log::info!("siege: nuke away at tick {}", self.game.sim.tick);
    }

    /// Into the Peewee nearest the middle of the line (`TA_SIEGE_RIDE=arm`), or the Light
    /// Laser Tower there (`core`).
    fn siege_ride(&mut self) {
        let Some(m) = self.siege_middle else { return };
        let core = std::env::var("TA_SIEGE_RIDE").is_ok_and(|v| v == "core");
        let want = if core { "CORLLT" } else { "ARMPW" };
        // (A Peewee a little behind the front: one at it falls to the towers in seconds.)
        let behind = if core { 0.0 } else { std::env::var("TA_SIEGE_RIDE_BEHIND").ok().and_then(|v| v.parse().ok()).unwrap_or(160.0) };
        let near = |u: &ta_sim::Unit| {
            let p = u.pos.to_f32();
            (p[0] - (m.x - behind)).powi(2) + (p[2] - m.z).powi(2)
        };
        let id = self.game.sim.units().filter(|u| !u.dead && self.game.sim.unit_type(u).def.name.eq_ignore_ascii_case(want)).min_by(|a, b| near(a).total_cmp(&near(b))).map(|u| u.id);
        if let Some(id) = id {
            // (Watching the enemy's tower: as a spectator, who may watch any unit.)
            if core {
                self.game.spectator = true;
            }
            let now = self.egui.ctx.input(|i| i.time);
            self.game.selection = vec![id];
            self.enter_perspective(id, now);
        }
    }

    /// `--demo war` (made for Sapphire Reach, places given as fractions of the map), staged to
    /// be read at a glance: on the plateau in the middle, cleared of trees, the Arm advances in
    /// three firing lines (tanks in front, kbots behind, artillery at the back, each spaced by its
    /// size) on a Core line of the same and opens fire across the gap; from 4 s two Krogoths
    /// stride out through an opening in the Core's ranks into the gap. Two battle lines of ships
    /// sail east down the channel to the north, trading broadsides; bombers in formation strike
    /// each side's artillery and fighters patrol over the gap; Arm hovercraft land under a Core
    /// fortress on the northern island's shore. A Silencer with a missile ready waits in the east
    /// (`TA_WAR_NUKE`). `TA_WAR=<scale>` scales the lines and fleets (1: 20 Arm and 18 Core a rank).
    fn war_demo(&mut self) {
        let scale: f32 = std::env::var("TA_WAR").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0f32).clamp(0.1, 8.0);
        let n = |k: usize| ((k as f32 * scale).round() as usize).max(1);
        let spectator = self.game.spectator;
        self.game.spectator = false;
        // The Core's computer player would send its lines off to attack.
        self.game.retire_ai(1);
        let (north, east, west) = (32768, 16384, -16384);
        let (w, _) = self.game.sim.world.size_px();
        let px = w.to_f32();
        // The land battle, between the plateau's cliffs and the canyon to the south: the fronts
        // stand 280 apart (within the front ranks' 320); the Arm's ranks start 400 px back.
        let (mid, centre_z, gap, advance) = (0.4425f32, 0.546f32, 280.0f32, 400.0f32);
        // A clear field: the trees and rocks among the lines go, thinning out round them (they
        // stood in front of every low shot and among the units); metal deposits stay.
        let (lo, hi) = (self.war_point((mid - 1000.0 / px, centre_z - 960.0 / px)).to_f32(), self.war_point((mid + 760.0 / px, centre_z + 960.0 / px)).to_f32());
        let (cx, cz, hx, hz) = ((lo[0] + hi[0]) / 2.0, (lo[2] + hi[2]) / 2.0, (hi[0] - lo[0]) / 2.0, (hi[2] - lo[2]) / 2.0);
        let cleared = self.game.sim.clear_features_where(|i, p| {
            let p = p.to_f32();
            let d = (((p[0] - cx) / hx).powi(4) + ((p[2] - cz) / hz).powi(4)).powf(0.25);
            let luck = ((i as u32).wrapping_mul(2_654_435_761) >> 22) as f32 / 1024.0;
            d < 0.85 || (d < 1.25 && luck < (1.25 - d) / 0.4)
        });
        let ranks: [([&str; 2], [&str; 2], f32); 3] = [
            (["ARMBULL", "ARMHAM"], ["CORREAP", "CORGOL"], 0.0),
            (["ARMFIDO", "ARMJETH"], ["CORMORT", "CORCRASH"], 110.0),
            (["ARMMART", "ARMMERL"], ["CORMART", "CORVROC"], 260.0),
        ];
        let mut lines: [Vec<Vec<ta_sim::UnitId>>; 2] = [Vec::new(), Vec::new()];
        for (arm_names, core_names, back) in ranks {
            let stand = mid - (gap / 2.0 + back) / px;
            let start = stand - advance / px;
            let line = self.war_block(&arm_names, 0, (start, centre_z), n(20), n(20), 0.0, east);
            let dx = ta_sim::Fx::from_f32(advance);
            self.war_each(0, &line, 1, |p| ta_sim::Order::Move(ta_sim::FxVec3::new(p.x + dx, ta_sim::Fx::ZERO, p.z)));
            lines[0].push(line);
            // The Core's ranks in two halves, 400 px apart in the middle for the Krogoths.
            let stand = mid + (gap / 2.0 + back) / px;
            let half = n(18).div_ceil(2);
            let reach = ((half - 1) as f32 * 80.0 + 400.0) / 2.0 / px;
            let mut line = self.war_block(&core_names, 1, (stand, centre_z - reach), half, half, 80.0, west);
            line.extend(self.war_block(&core_names, 1, (stand, centre_z + reach), half, half, 80.0, west));
            lines[1].push(line);
        }
        for (side, ranks) in lines.iter().enumerate() {
            let all: Vec<ta_sim::UnitId> = ranks.iter().flatten().copied().collect();
            self.game.issue(ta_sim::Command::SetStates { player: side as u8, units: all, fire: None, movement: Some(0) });
        }
        // The Krogoths, side by side behind the opening, walk out into the middle of the gap from
        // 4 s on (about 20 s at their pace) and hold there.
        let krogs = self.war_block(&["CORKROG"], 1, (mid + 560.0 / px, centre_z), n(2), n(2), 150.0, west);
        self.game.issue(ta_sim::Command::SetStates { player: 1, units: krogs.clone(), fire: None, movement: Some(0) });
        let into_gap = ta_sim::Fx::from_f32(mid * px);
        self.war_each(1, &krogs, 120, |p| ta_sim::Order::Move(ta_sim::FxVec3::new(into_gap, ta_sim::Fx::ZERO, p.z)));
        // The battle lines at sea: columns 300 px apart (where the destroyers and battleships
        // see the other line; ships see 210-350 px) heading east abreast, the fastest ships ahead
        // (so none runs into the one in front), sailing in step to places as far apart as they
        // start.
        let arm_ships = ["ARMROY", "ARMROY", "ARMROY", "ARMCRUS", "ARMCRUS", "ARMCRUS", "ARMBATS", "ARMBATS"];
        let core_ships = ["CORROY", "CORROY", "CORROY", "CORCRUS", "CORCRUS", "CORCRUS", "CORBATS", "CORBATS"];
        let mut fleets = Vec::new();
        for (side, names, z, head) in [(0u8, arm_ships, 0.349f32, 0.262f32), (1u8, core_ships, 0.3673f32, 0.262f32)] {
            let mut column = Vec::new();
            for (k, name) in names.iter().enumerate().take(n(8)) {
                column.extend(self.war_block(&[name], side, (head - k as f32 * 170.0 / px, z), 1, 1, 0.0, east));
            }
            let dx = ta_sim::Fx::from_f32((0.47 - head) * px);
            self.war_each(side, &column, 1, |p| ta_sim::Order::Move(ta_sim::FxVec3::new(p.x + dx, ta_sim::Fx::ZERO, p.z)));
            fleets.push(column);
        }
        // The fortress on the northern island's shore, and the landing on it.
        let mut fort = Vec::new();
        for (name, fz, every) in [("CORHLT", 0.250, 160.0), ("CORTOAST", 0.238, 240.0), ("CORFLAK", 0.232, 320.0), ("CORDOOM", 0.226, 360.0), ("CORPUN", 0.214, 300.0)] {
            let mut x = 0.295 * 16384.0;
            while x < 0.395 * 16384.0 {
                fort.extend(self.war_block(&[name], 1, (x / 16384.0, fz), 1, 1, 0.0, 0));
                x += every;
            }
        }
        let landing = self.war_block(&["ARMANAC", "ARMANAC", "ARMANAC", "ARMAH"], 0, (0.345, 0.295), n(20), 10, 0.0, north);
        let beach = self.war_point((0.345, 0.19));
        self.war_each(0, &landing, 1, |p| ta_sim::Order::Fight(ta_sim::FxVec3::new(p.x, ta_sim::Fx::ZERO, beach.z)));
        // The air war: bombers in formation on the other side's artillery, fighters on patrol.
        let arm_bombers = self.war_block(&["ARMTHUND"], 0, (0.29, 0.52), n(9), 3, 150.0, east);
        let core_bombers = self.war_block(&["CORSHAD"], 1, (0.56, 0.60), n(6), 3, 150.0, west);
        for (side, bombers, targets) in [(0u8, &arm_bombers, &lines[1][2]), (1u8, &core_bombers, &lines[0][2])] {
            let picks: Vec<ta_sim::UnitId> = targets.iter().step_by((targets.len() / bombers.len().max(1)).max(1)).copied().collect();
            for (k, (&b, &t)) in bombers.iter().zip(picks.iter().cycle()).enumerate() {
                let order = ta_sim::Order::Attack(ta_sim::Target::Unit(t));
                self.war_queue.push((60 + side as u64 * 30 + k as u64, ta_sim::Command::Order { player: side, units: vec![b], order, queue: false }));
            }
        }
        let arm_fighters = self.war_block(&["ARMHAWK"], 0, (0.33, 0.50), n(8), 4, 130.0, east);
        let core_fighters = self.war_block(&["CORVAMP"], 1, (0.52, 0.55), n(8), 4, 130.0, west);
        self.war_orders(0, &arm_fighters, ta_sim::Order::Patrol(self.war_point((mid + 0.02, centre_z))));
        self.war_orders(1, &core_fighters, ta_sim::Order::Patrol(self.war_point((mid - 0.02, centre_z))));
        // The Silencer in the east.
        if let Some(t) = self.game.sim.types.id("CORSILO") {
            let want = self.war_point((0.60, 0.62));
            if let Some(p) = self.game.sim.find_build_site(t, want, 40) && let Some(silo) = self.game.sim.spawn_unit(t, 1, p, west, true) {
                self.game.sim.grant_stockpile(silo, 1);
            }
        }
        // The orders in the order of their ticks (within a tick, as they were given).
        let now = self.game.sim.tick;
        for (t, _) in self.war_queue.iter_mut() {
            *t += now;
        }
        self.war_queue.sort_by_key(|(t, _)| *t);
        self.game.sim.reveal_map_for(self.game.local_player);
        self.game.spectator = spectator;
        let focus = self.war_point((mid, centre_z));
        self.camera.focus = Vec3::from_array(focus.to_f32());
        self.camera.zoom = 0.4;
        self.game.scene.after_tick(&self.game.sim);
        log::info!(
            "war: lines {:?} + {:?} (and {} Krogoths) on a field cleared of {cleared} features, fleets {} + {}, fortress {}, landing {}, bombers {} + {}, fighters {} + {}",
            lines[0].iter().map(Vec::len).collect::<Vec<_>>(),
            lines[1].iter().map(Vec::len).collect::<Vec<_>>(),
            krogs.len(),
            fleets[0].len(),
            fleets[1].len(),
            fort.len(),
            landing.len(),
            arm_bombers.len(),
            core_bombers.len(),
            arm_fighters.len(),
            core_fighters.len()
        );
    }

    /// Each of `units` its own order (from where it stands), from `tick` ticks on, ten a tick:
    /// eleven or fewer sent off in a tick each plan their own way (a dozen or more share one
    /// route and would funnel into it).
    fn war_each(&mut self, owner: u8, units: &[ta_sim::UnitId], tick: u64, order: impl Fn(ta_sim::FxVec3) -> ta_sim::Order) {
        // Under the rule "modern movement", one order: the group keeps its shape and marches
        // there together (each unit's place is where `order` sends it).
        let sim = &self.game.sim;
        if sim.group_formations && units.len() > 1 {
            let middle = ta_sim::formation::centre(units.iter().filter_map(|&id| sim.unit(id)).map(|u| u.pos));
            let cmd = ta_sim::Command::Formation { player: owner, units: units.to_vec(), order: order(middle), queue: false, shape: ta_sim::formation::Shape::Keep };
            self.war_queue.push((tick, cmd));
            return;
        }
        for &id in units {
            let Some(pos) = self.game.sim.unit(id).map(|u| u.pos) else { continue };
            // (The first tick from `tick` on with room for another.)
            let mut at = tick;
            while self.war_queue.iter().filter(|(t, _)| *t == at).count() >= 10 {
                at += 1;
            }
            let cmd = ta_sim::Command::Order { player: owner, units: vec![id], order: order(pos), queue: false };
            self.war_queue.push((at, cmd));
        }
    }

    /// A place on the map given as fractions of its width and height.
    fn war_point(&self, (fx, fz): (f32, f32)) -> ta_sim::FxVec3 {
        let (w, h) = self.game.sim.world.size_px();
        ta_sim::FxVec3::new(ta_sim::Fx::from_f32(fx * w.to_f32()), ta_sim::Fx::ZERO, ta_sim::Fx::from_f32(fz * h.to_f32()))
    }

    /// `n` units of `names` in turn, in rows of `cols` `gap` px apart round a place (fractions of
    /// the map; a `gap` of 0 spaces them by their size), facing `heading`: each where its type
    /// can be (a ship in water, a tank on land), or not at all.
    #[allow(clippy::too_many_arguments)]
    fn war_block(&mut self, names: &[&str], owner: u8, at: (f32, f32), n: usize, cols: usize, gap: f32, heading: i32) -> Vec<ta_sim::UnitId> {
        let types: Vec<usize> = names.iter().filter_map(|name| self.game.sim.types.id(name)).collect();
        if types.is_empty() {
            return Vec::new();
        }
        let centre = self.war_point(at).to_f32();
        // (0: as far apart as the largest of them needs, and a little more.)
        let gap = if gap > 0.0 { gap } else { types.iter().map(|&t| self.game.sim.types.get(t).footprint.0.max(self.game.sim.types.get(t).footprint.1)).max().unwrap_or(2) as f32 * 16.0 + 32.0 };
        let rows = n.div_ceil(cols.max(1));
        let mut out = Vec::new();
        for i in 0..n {
            let (row, col) = (i / cols.max(1), i % cols.max(1));
            let x = centre[0] + (row as f32 - (rows as f32 - 1.0) / 2.0) * gap * if heading == 16384 { -1.0 } else { 1.0 };
            let z = centre[2] + (col as f32 - (cols.min(n) as f32 - 1.0) / 2.0) * gap;
            let t = types[i % types.len()];
            let want = ta_sim::FxVec3::new(ta_sim::Fx::from_f32(x), ta_sim::Fx::ZERO, ta_sim::Fx::from_f32(z));
            let sim = &self.game.sim;
            let wet = sim.world.height_at(want.x, want.z) < sim.world.sea_level - ta_sim::Fx::from_int(12);
            let site = match sim.types.get(t).locomotion {
                ta_sim::Locomotion::Naval | ta_sim::Locomotion::Hover if wet => Some(want),
                ta_sim::Locomotion::Naval => None,
                _ => sim.find_build_site(t, want, 2),
            };
            if let Some(p) = site && let Some(id) = self.game.sim.spawn_unit(t, owner, p, heading, true) {
                out.push(id);
            }
        }
        out
    }

    /// The same order for each of `units`, ten at a time (a dozen or more sent together share
    /// one route, and would funnel into it).
    fn war_orders(&mut self, owner: u8, units: &[ta_sim::UnitId], order: ta_sim::Order) {
        for group in units.chunks(10) {
            self.game.issue(ta_sim::Command::Order { player: owner, units: group.to_vec(), order: order.clone(), queue: false });
        }
    }

    /// `TA_RIDE`: into the unit of that type nearest `TA_RIDE_NEAR=x,z` (px) or the camera's
    /// focus; an enemy's as a spectator (who may watch any unit).
    fn ride_nearest(&mut self) {
        let Ok(name) = std::env::var("TA_RIDE") else { return };
        let near: Option<Vec<f32>> = std::env::var("TA_RIDE_NEAR").ok().map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect());
        let (x, z) = match near.as_deref() {
            Some([x, z, ..]) => (*x, *z),
            _ => (self.camera.focus.x, self.camera.focus.z),
        };
        let sim = &self.game.sim;
        let dist = |u: &ta_sim::Unit| {
            let p = u.pos.to_f32();
            (p[0] - x).powi(2) + (p[2] - z).powi(2)
        };
        let Some((id, owner)) = sim.units().filter(|u| !u.dead && u.is_complete() && sim.unit_type(u).def.name.eq_ignore_ascii_case(&name)).min_by(|a, b| dist(a).total_cmp(&dist(b))).map(|u| (u.id, u.owner)) else {
            log::warn!("TA_RIDE: no {name} to ride");
            return;
        };
        if !self.game.sim.allied(owner, self.game.local_player) {
            self.game.spectator = true;
        }
        let now = self.egui.ctx.input(|i| i.time);
        self.game.selection = vec![id];
        self.enter_perspective(id, now);
        log::info!("ride: into {name} {id:?} at tick {}", self.game.sim.tick);
    }

    /// The war's Silencer fires at the middle of what is left of the Arm army on the plain.
    fn war_nuke(&mut self) {
        let silo = self.game.sim.units().find(|u| self.game.sim.unit_type(u).def.name == "CORSILO" && u.stockpile > 0).map(|u| u.id);
        let Some(silo) = silo else { return };
        let (w, h) = self.game.sim.world.size_px();
        let (w, h) = (w.to_f32(), h.to_f32());
        let army: Vec<[f32; 3]> = self
            .game
            .sim
            .units()
            .filter(|u| u.owner == 0 && !u.dead && self.game.sim.unit_type(u).locomotion == ta_sim::Locomotion::Ground)
            .map(|u| u.pos.to_f32())
            .filter(|p| p[2] > 0.5 * h && p[0] < 0.55 * w)
            .collect();
        if army.is_empty() {
            return;
        }
        let (sx, sz) = army.iter().fold((0.0, 0.0), |(x, z), p| (x + p[0], z + p[2]));
        let target = ta_sim::FxVec3::new(ta_sim::Fx::from_f32(sx / army.len() as f32), ta_sim::Fx::ZERO, ta_sim::Fx::from_f32(sz / army.len() as f32));
        let spectator = self.game.spectator;
        self.game.spectator = false;
        self.game.issue(ta_sim::Command::Order { player: 1, units: vec![silo], order: ta_sim::Order::ManualFire(ta_sim::Target::Ground(target)), queue: false });
        self.game.spectator = spectator;
        log::info!("war: nuke away at tick {} for {:?}", self.game.sim.tick, target.to_f32());
    }

    /// `--demo perspective`: a unit of ours facing enemies, and into it.
    fn perspective_demo(&mut self) {
        use ta_sim::{Fx, FxVec3};
        let name = std::env::var("TA_PERSPECTIVE_UNIT").unwrap_or_else(|_| "ARMSTUMP".into());
        let asked = std::env::var("TA_PERSPECTIVE_ENEMY").ok();
        let Some(t) = self.game.sim.types.id(&name) else {
            log::warn!("perspective demo: no unit {name}");
            return;
        };
        let me = self.game.local_player;
        let ty = self.game.sim.types.get(t);
        let sim = &self.game.sim;
        let (w, h) = sim.world.size_px();
        let sea = sim.world.sea_level;
        // Ships go to open water, the rest by the Commander (or the middle of the map).
        let naval = matches!(ty.locomotion, ta_sim::Locomotion::Naval | ta_sim::Locomotion::Hover);
        let near = self.game.local_commander().and_then(|c| sim.unit(c)).map_or(FxVec3::new(w / 2, Fx::ZERO, h / 2), |u| u.pos);
        let at = if naval {
            let deep = |x: i64, z: i64| sim.world.height_at(Fx::from_int(x), Fx::from_int(z)) < sea - Fx::from_int(30);
            (200..h.to_int() - 200)
                .step_by(64)
                .flat_map(|z| (200..w.to_int() - 800).step_by(64).map(move |x| (x, z)))
                .find(|&(x, z)| (0..=600).step_by(100).all(|dx| deep(x + dx, z)))
                .map(|(x, z)| FxVec3::from_ints(x, 0, z))
        } else {
            let spot = near + FxVec3::from_ints(120, 0, 60);
            Some(self.game.sim.find_build_site(t, spot, 24).unwrap_or(spot))
        };
        let Some(at) = at else { return };
        let heading = 16384;
        let spectator = self.game.spectator;
        self.game.spectator = false;
        let Some(id) = self.game.sim.spawn_unit(t, me, at, heading, true) else { return };
        // `TA_PERSPECTIVE_HULL`: its hull worn down to that share of the whole (0..1), for the
        // cockpit's look when badly damaged.
        if let Some(share) = std::env::var("TA_PERSPECTIVE_HULL").ok().and_then(|v| v.parse::<f32>().ok()) {
            let most = self.game.sim.types.get(t).max_health;
            self.game.sim.set_health(id, Fx::from_f32(most.to_f32() * share.clamp(0.0, 1.0)));
        }
        // The enemy ahead of it (`TA_PERSPECTIVE_RANGE` px away): at sea, one that floats (a
        // tank there would lie on the sea bed).
        let range: i64 = std::env::var("TA_PERSPECTIVE_RANGE").ok().and_then(|v| v.parse().ok()).unwrap_or(if naval { 520 } else { 330 });
        let at_sea = |name: &str| self.game.sim.types.id(name).is_some_and(|e| matches!(self.game.sim.types.get(e).locomotion, ta_sim::Locomotion::Naval | ta_sim::Locomotion::Hover | ta_sim::Locomotion::Air));
        let enemy = match asked {
            Some(e) if naval && !at_sea(&e) => {
                log::warn!("perspective demo: {e} cannot stand at sea; CORPT in its place");
                "CORPT".to_string()
            }
            Some(e) => e,
            None if naval => "CORPT".to_string(),
            None => "CORRAID".to_string(),
        };
        if let Some(e) = self.game.sim.types.id(&enemy) {
            for k in 0..3i64 {
                let p = at + FxVec3::from_ints(range + k * 30, 0, (k - 1) * 60);
                self.game.sim.spawn_unit(e, 1, p, -16384, true);
            }
        }
        // `TA_PERSPECTIVE_ORDER`: an order it goes on with while the view rides along, to a
        // point past the enemy (a patrol, back and forth between there and here); a builder
        // builds the first thing it can a little ahead.
        let to = at + FxVec3::from_ints(range * 2, 0, 0);
        let to = FxVec3::new(to.x.clamp(Fx::ZERO, w), to.y, to.z.clamp(Fx::ZERO, h));
        let order = match std::env::var("TA_PERSPECTIVE_ORDER").as_deref() {
            Ok("move") => Some(ta_sim::Order::Move(to)),
            Ok("patrol") => Some(ta_sim::Order::Patrol(to)),
            Ok("fight") => Some(ta_sim::Order::Fight(to)),
            Ok("build") => self.game.sim.types.get(t).build_options.first().copied().map(|type_id| {
                let spot = at + FxVec3::from_ints(160, 0, 0);
                ta_sim::Order::Build { type_id, pos: self.game.sim.find_build_site(type_id, spot, 24).unwrap_or(spot) }
            }),
            _ => None,
        };
        if let Some(order) = order {
            self.game.issue(ta_sim::Command::Order { player: me, units: vec![id], order, queue: false });
        }
        self.game.spectator = spectator;
        self.game.sim.los_enabled = false;
        self.game.scene.after_tick(&self.game.sim);
        let p = Vec3::from_array(at.to_f32());
        self.camera.focus = p;
        let now = self.egui.ctx.input(|i| i.time);
        self.game.selection = vec![id];
        self.enter_perspective(id, now);
    }

    /// What the highlights' buttons asked for.
    fn reel_actions(&mut self) {
        let Some(reel) = self.reel.as_mut() else { return };
        match std::mem::replace(&mut reel.action, crate::highlights::ReelAction::None) {
            crate::highlights::ReelAction::None => {}
            crate::highlights::ReelAction::Again => reel.restart(),
            crate::highlights::ReelAction::FullReplay => self.pending = Some(Transition::Play((*reel.record).clone())),
            crate::highlights::ReelAction::Leave => self.pending = Some(Transition::ToMenu),
        }
    }

    /// A frame of photo mode: the free camera, and clicks on the picture set the focus.
    fn photo_frame(&mut self, dt: f32) {
        if self.game.keys.pressed(&self.input, "screenshot")
            && let Some(photo) = self.photo.as_mut()
        {
            photo.take = true;
        }
        let typing = self.egui.ctx.egui_wants_keyboard_input();
        if !typing && self.input.pressed(KeyCode::KeyH) && let Some(photo) = self.photo.as_mut() {
            photo.hide_panel = !photo.hide_panel;
        }
        self.update_camera(dt);
        // Its lens, and what it asks of the battle (its pace, its weather).
        let now = crate::scene::game_seconds(self.game.sim.tick, 0.0) as f32;
        if let Some(photo) = self.photo.as_mut() {
            photo.apply_lens(&mut self.camera);
            photo.sync(&mut self.game, now);
        }
        let ui_pointer = self.egui.ctx.egui_wants_pointer_input() || self.egui.ctx.is_pointer_over_egui();
        if !ui_pointer && self.input.button_pressed(MouseButtonId::Left) {
            let mouse = self.input.mouse;
            let ground = self.ground_under(mouse);
            let (ppp, now) = (self.egui.ctx.pixels_per_point(), self.egui.ctx.input(|i| i.time));
            if let Some(photo) = self.photo.as_mut() {
                photo.click(mouse, &self.camera, ground, ppp, now);
            }
        }
    }

    fn ground_under(&self, screen: Vec2) -> Option<Vec3> {
        let terrain = self.renderer.terrain.as_ref()?;
        let (o, d) = self.camera.screen_ray(screen);
        terrain.field.raycast(o, d)
    }

    /// A single-player battle under way written to the autosave before the game closes (the
    /// graphics device lost): whether one was.
    fn save_before_closing(&mut self) -> bool {
        let g = &self.game;
        if self.screen != Screen::Playing || g.net.is_some() || g.playback.is_some() || g.spectator || g.outcome.is_some() || g.tutorial {
            return false;
        }
        let Some(ticks) = g.record.snapshot() else { return false };
        let title = format!("Autosave · {}", record_title(g, &self.assets));
        let record = crate::replay::Record::new(crate::replay::RecordKind::Save, self.assets.data_hash, self.assets.mods.record(), g.setup.clone(), ticks, g.sim.tick, title).with_profile(g).with_duties(g);
        match record.write_as("autosave") {
            Ok(_) => true,
            Err(e) => {
                log::error!("the battle could not be saved before closing: {e}");
                false
            }
        }
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        if self.backdrop_retry.is_some_and(|at| Instant::now() >= at) && self.pending.is_none() {
            self.backdrop_retry = None;
            self.pending = Some(Transition::RestoreMenuBackdrop);
        }
        self.game.effects.reduced_flashes = self.settings.gameplay.reduced_flashes;
        self.renderer.post.settings.reduced_motion = self.settings.gameplay.reduced_motion;
        self.renderer.post.settings.reduced_flashes = self.settings.gameplay.reduced_flashes;
        ta_ui::motion::set_reduced(&self.egui.ctx, self.settings.gameplay.reduced_motion);
        ta_ui::motion::set_reduced_flashes(&self.egui.ctx, self.settings.gameplay.reduced_flashes);
        // (`TA_SOUND_LOG`: the sounds this frame starts are at its time on a capture's clock.)
        self.audio.set_frame(self.frames, self.capture_dt().is_some());
        self.audio.set_background(!self.focused() && self.screenshot_path.is_none());
        self.poll_gamepad();
        for k in std::mem::take(&mut self.demo_keys) {
            self.input.press(k);
        }
        // Poll first: otherwise the controller can put battle actions back after the
        // dialog discarded keyboard input. Its separate egui navigation stays available.
        if self.profile_editor.is_some() || self.career_window.is_open() { self.input.ignore(); }
        if matches!(self.screen, Screen::Editor | Screen::Space) { self.flush_progress(false); }
        if self.screen == Screen::Editor {
            return self.editor_frame(event_loop);
        }
        if self.screen == Screen::Space {
            return self.space_frame(event_loop);
        }
        let now = Instant::now();
        // (A film moves the battle on a picture's worth a frame, however long the frame takes.)
        let dt = self.film.as_ref().map(|f| f.dt()).or_else(|| self.capture_dt()).unwrap_or_else(|| (now - self.last_frame).as_secs_f32().min(0.1));
        self.frame_ms = (now - self.last_frame).as_secs_f32() * 1000.0;
        self.dev.meter.tick(self.frame_ms);
        self.last_frame = now;
        self.game.paced = self.film.is_none() && self.capture_dt().is_none() && self.benchmark.is_none();
        self.dev.fps = self.dev.fps * 0.95 + (1.0 / dt.max(1e-4)) * 0.05;
        // A benchmark: Escape or the quit key stops it (with no report: the launcher says so);
        // nothing else reaches the battle, which plays the same on every computer.
        if self.benchmark.is_some() {
            if self.input.pressed(KeyCode::Escape) || self.game.keys.pressed(&self.input, "quit") {
                log::info!("benchmark stopped");
                self.pending = Some(Transition::Quit);
            }
            self.input.ignore();
        }
        // While a film is made only its page takes the pointer and the keys.
        if self.film.is_some() {
            self.input.ignore();
            self.input.wheel = 0.0;
        }

        // The companion screen: its key, or the pause menu's button.
        let companion_key = self.game.keys.pressed(&self.input, "companion") && !self.egui.ctx.egui_wants_keyboard_input();
        if companion_key || std::mem::take(&mut self.game.companion_toggle) {
            self.toggle_companion(event_loop);
        }
        self.game.companion_open = self.companion.is_some();

        // F1: TA's information on the selected unit (or the one under the pointer) in a battle,
        // the controls reference otherwise; Ctrl+F1: the developer panel.
        if self.game.keys.pressed(&self.input, "help") {
            if self.game.unit_info.take().is_none() {
                match (self.screen == Screen::Playing && !self.controls_open).then(|| self.game.info_target()).flatten() {
                    Some(info) => self.game.unit_info = Some(info),
                    // (The reference is a battle's: on a menu page the key used to set it to pop
                    // up when the next battle began.)
                    None if self.screen == Screen::Playing => self.controls_open = !self.controls_open,
                    None => {}
                }
            }
        } else if self.input.modifiers.control_key() && self.game.keys.key_pressed(&self.input, "help") {
            self.dev.visible = !self.dev.visible;
        }
        // The quit key: in a battle it opens the pause menu, where the game is left (and its
        // replay kept); elsewhere it closes the program as the menu's QUIT does.
        if self.game.keys.pressed(&self.input, "quit") {
            if self.screen == Screen::Playing {
                self.in_game_menu = true;
            } else {
                self.pending = Some(Transition::Quit);
            }
        }
        if matches!(self.screen, Screen::Menu | Screen::Playing) && !self.game.history_open && !self.egui.ctx.egui_wants_keyboard_input() && self.game.chat_input.is_none() && self.game.keys.pressed(&self.input, "jukebox") {
            self.jukebox.open = !self.jukebox.open;
        }

        let t_update = Instant::now();
        // A battle begun while we were still in the last one's result: straight into it.
        if self.screen == Screen::Playing && self.pending.is_none() && self.game.net.as_ref().is_some_and(|n| n.next_battle.is_some())
            && let Some(session) = self.game.net.take()
            && let Some((setup, session)) = crate::lobby::NetLobby::next_battle(session, &self.assets)
        {
            self.pending = Some(Transition::StartNet(setup, session));
        }
        if let (Screen::Menu, Some(players)) = (self.screen, self.auto_start) && let Some(crate::lobby::LobbyAction::Start(setup, session)) = self.menu.lobby.as_mut().and_then(|l| l.try_auto_start(players, &self.assets)) {
            self.menu.lobby = None;
            self.menu.page = crate::menu::Page::Title;
            self.pending = Some(Transition::StartNet(setup, session));
        }
        if let Some(t) = self.net_check_tick {
            if self.screen == Screen::Playing {
                self.game.probe_tick = Some(t);
            }
            if let Some(h) = self.game.probe_hash {
                println!("net-check tick {t} checksum {h:016x} units {} desync {:?}", self.game.sim.unit_count(), self.game.net.as_ref().and_then(|n| n.desync));
                self.archive_replay();
                event_loop.exit();
            }
        }
        self.credits_sync();
        match self.screen {
            Screen::Intro => {
                // The menu's battle fast-forwarded a slice at a time behind the intro.
                self.warm_up_attract(INTRO_WARMUP_TICKS);
                let skip = self.input.pressed(KeyCode::Escape)
                    || self.input.pressed(KeyCode::Space)
                    || self.input.pressed(KeyCode::Enter)
                    || self.input.button_pressed(MouseButtonId::Left);
                match self.intro_room.as_mut() {
                    // The room: the first movie once the monitor is warm (a key before that skips
                    // the lot), then movie after movie, then the camera leans in.
                    Some(room) => {
                        room.poll(&mut self.renderer);
                        if self.input.pressed(KeyCode::Escape) {
                            // Escape: straight to the menu.
                            self.finish_intro();
                        } else if room.failed() {
                            self.intro_room = None;
                            self.next_movie();
                        } else if room.closing() {
                            if room.closed() || skip {
                                self.finish_intro();
                            }
                        } else if self.movie.is_none() {
                            if skip {
                                room.close();
                            } else if room.warm() {
                                if std::env::var_os("TA_ROOM_TUBE").is_some() {
                                    self.room_tube.get_or_insert((0.0, usize::MAX));
                                } else {
                                    self.next_movie();
                                }
                            }
                        } else if skip || self.movie.as_ref().is_some_and(|m| m.finished) {
                            self.next_movie();
                        }
                    }
                    None => {
                        if skip || self.movie.as_ref().is_none_or(|m| m.finished) {
                            self.next_movie();
                        }
                    }
                }
            }
            Screen::Menu if self.credits_show.is_some() => self.credits_update(dt),
            Screen::Menu if self.showroom.is_some() => {
                self.game.update(dt);
                // Dragging round the unit and the wheel (the interface's own pointer aside).
                let drag = if self.input.button_held(MouseButtonId::Left) { self.input.mouse_delta } else { Vec2::ZERO };
                let wheel = self.input.wheel;
                let encyclopaedia = &mut self.menu.encyclopaedia;
                let (turning, reset, preview) = (encyclopaedia.turning(), encyclopaedia.take_reset(), encyclopaedia.preview());
                if let Some(room) = self.showroom.as_mut() {
                    // The camera stays with the unit on show (an aircraft takes off and circles).
                    if let Some(u) = room.unit.and_then(|id| self.game.sim.unit(id)) {
                        room.centre = Vec3::from_array(u.pos.to_f32()) + (room.ground_centre - room.placed);
                    }
                    if reset {
                        room.homing = true;
                    }
                    // The unit on the stage the panels leave it, or framed for the wallpaper in
                    // its preview.
                    let ctx = &self.egui.ctx;
                    let scale = ctx.pixels_per_point();
                    let px = |r: egui::Rect| egui::Rect::from_min_max(r.min * scale, r.max * scale);
                    let frame = match preview {
                        Some((aspect, words)) => crate::showroom::Frame::wallpaper(px(crate::encyclopaedia::preview_rect(ctx.content_rect(), crate::encyclopaedia::bar_height(ctx), aspect)), words),
                        None => crate::showroom::Frame::stage(px(crate::encyclopaedia::showroom_layout(ctx.content_rect(), crate::encyclopaedia::deck_height(ctx)).stage)),
                    };
                    crate::showroom::update_camera(room, &mut self.camera, frame, dt, turning, drag, wheel);
                    room.frames += 1;
                }
                self.draw_unit_picture();
            }
            Screen::Menu if self.menu_film.is_some() => {
                if let Some(film) = self.menu_film.as_mut() {
                    film.before_update(&mut self.game);
                    self.game.update(dt);
                    // The cut: the last scene's particles and sounds go too, and the next one's
                    // moon comes up.
                    if film.after_update(&mut self.game) {
                        self.renderer.clear_particles();
                        self.audio.cut_scene();
                        self.renderer.lighting.moon_dir = film.moon_dir();
                    }
                }
                self.watch_menu_film(dt);
            }
            Screen::Menu => {
                if self.attract.warmup > 0 && self.game.spectator {
                    // Normal returns arrive fully prepared by the loading worker. Keep
                    // this fallback bounded too, so a future route cannot freeze a frame.
                    self.warm_up_attract(INTRO_WARMUP_TICKS);
                } else {
                    self.game.update(dt);
                }
                self.update_attract_camera(dt);
            }
            Screen::Playing if self.game.catch_up_progress().is_some() => {
                // A saved game re-simulates its recorded ticks, a slice per frame (and so
                // does a replay jumping ahead).
                self.renderer.clear_particles();
                // (A film's page, or the EXPORT CLIP dialog's pictures, wait on it: more of each
                // frame.)
                self.game.catch_up(std::time::Duration::from_millis(if self.film.is_some() || self.clip_previews.is_some() { 60 } else { 14 }));
            }
            Screen::Playing if self.game.request_rewind.is_some() => {
                let tick = self.game.request_rewind.take().unwrap_or(0);
                if let Some(record) = self.game.playback.as_ref().and_then(|p| p.source.clone()) {
                    self.pending = Some(Transition::Rewind(record, tick));
                }
            }
            Screen::Playing => {
                self.run_demo();
                self.close_battle_panel_keys();
                if self.game.radial.is_some() && (self.photo.is_some() || self.perspective.is_some() || self.reel.is_some() || self.battle_overlay_open() || self.game.perspective_request.is_some()) {
                    self.game.radial = None;
                }
                if self.benchmark.is_none() && self.photo.is_none() && self.perspective.is_none() && self.reel.is_none()
                    && (!self.battle_overlay_open() || self.game.history_open || self.in_game_menu)
                    && !self.options_open && !self.controls_open && self.game.chat_input.is_none()
                    && !self.egui.ctx.text_edit_focused() && self.game.keys.pressed(&self.input, "event_history")
                {
                    self.game.history_open = !self.game.history_open;
                    self.game.cancel_pointer_gestures();
                }
                if let Some((spec, frame)) = self.photo_capture.take_if(|c| c.1 <= self.frames) {
                    let post = self.settled_post();
                    let mut photo = crate::photo::PhotoMode::enter(&mut self.game, post, &self.camera);
                    for word in spec.split('-').skip(1) {
                        match word {
                            "lens" => photo.focus = crate::photo::Focus::Lens,
                            "tilt" => photo.focus = crate::photo::Focus::TiltShift,
                            "hidden" => photo.hide_panel = true,
                            "take" => {}
                            other if photo.choose(other) => {}
                            other => log::warn!("--open photo: no such choice {other} (frame {frame})"),
                        }
                    }
                    self.photo = Some(photo);
                }
                // Photo mode: F11 (or the pause menu) in, F11 or Escape out.
                let photo_key = !self.game.history_open && !self.egui.ctx.text_edit_focused() && self.game.chat_input.is_none() && self.game.keys.pressed(&self.input, "photo_mode");
                let mut left_photo = false;
                if self.photo.as_ref().is_some_and(|p| p.leave || photo_key || self.input.pressed(KeyCode::Escape)) {
                    if let Some(photo) = self.photo.take() {
                        photo.leave(&mut self.game, &mut self.renderer.post.settings, &mut self.camera);
                    }
                    left_photo = true;
                } else if self.photo.is_none() && self.perspective.is_none() && (photo_key || self.open_photo) && self.game.outcome.is_none() {
                    let post = self.settled_post();
                    self.photo = Some(crate::photo::PhotoMode::enter(&mut self.game, post, &self.camera));
                    self.in_game_menu = false;
                    self.options_open = false;
                    self.controls_open = false;
                    self.game.stats_open = false;
                }
                self.open_photo = false;
                // Captures: `TA_FILM`'s film, once the replay's scan has found its big moments.
                let scanned = self.game.replay_scan.as_ref().is_none_or(|s| s.state().lock().is_ok_and(|s| s.finished));
                if scanned && self.film.is_none() && let Some(settings) = self.film_capture.take() {
                    self.start_film(settings);
                }
                // (And `TA_CLIP`'s clip.)
                if scanned && self.film.is_none() && let Some(spec) = self.clip_capture.take() {
                    self.start_clip_capture(spec);
                }
                // Captures: once the AI battle has been fast-forwarded, its highlights.
                if self.highlights_capture && let Some(pb) = self.game.playback.as_mut() {
                    if self.game.sim.tick == 0 && pb.seek_to.is_none() {
                        pb.seek_to = Some(pb.end_tick);
                    } else if pb.seek_to.is_none() {
                        self.highlights_capture = false;
                        self.pending = Some(Transition::Highlights);
                    }
                }
                // Perspective: the key or the HUD's button into the selected unit, else (the key)
                // the one under the pointer, an ally's or, spectating, anyone's; Escape (or the
                // key again) out.
                let now_ui = self.egui.ctx.input(|i| i.time);
                let perspective_key = !self.game.history_open && self.game.keys.pressed(&self.input, "perspective") && !self.egui.ctx.egui_wants_keyboard_input() && self.game.chat_input.is_none();
                if self.perspective.is_none() && self.photo.is_none() && self.reel.is_none() && self.game.outcome.is_none() && !self.in_game_menu {
                    let wanted = self.game.perspective_request.take().or_else(|| {
                        let game = &self.game;
                        perspective_key.then(|| game.selection.iter().copied().chain(game.hovered).find(|&id| crate::perspective::Perspective::can_enter(game, id))).flatten()
                    });
                    if let Some(id) = wanted {
                        self.enter_perspective(id, now_ui);
                    }
                } else if let Some(p) = self.perspective.as_mut()
                    && (self.input.pressed(KeyCode::Escape) || perspective_key)
                {
                    p.leave(&mut self.game);
                }
                if self.perspective.is_some() {
                    self.perspective_frame(dt, now_ui);
                    self.game.update(dt);
                } else if self.photo.is_some() {
                    self.photo_frame(dt);
                    // (The battle goes on in multiplayer.)
                    self.game.update(dt);
                } else if let Some(reel) = self.reel.as_mut() {
                    // Escape: straight to the end of the highlights (not the Escape that left
                    // photo mode); the pause key pauses them.
                    if self.input.pressed(KeyCode::Escape) && !left_photo {
                        reel.skip_to_end(&mut self.game);
                    } else if self.game.keys.pressed(&self.input, "pause") {
                        let paused = !self.game.paused;
                        self.game.set_pace(Some(paused), None);
                    }
                    reel.update(&mut self.game, &mut self.camera, &mut self.renderer, dt);
                    self.game.update(dt);
                    self.reel_actions();
                } else {
                // Escape with nothing to cancel, or TA's F2: the menu.
                let menu_key = self.game.keys.pressed(&self.input, "game_menu") && !self.egui.ctx.egui_wants_keyboard_input()
                    && !self.options_open && !self.controls_open && !self.game.history_open && (self.in_game_menu || !self.battle_overlay_open());
                // Escape opens the menu once nothing else wants it (a placement, the selection...),
                // and closes it whatever is selected.
                let escape = self.input.pressed(KeyCode::Escape)
                    && !self.game.history_open
                    && !self.options_open
                    && !self.controls_open
                    && (self.in_game_menu || (!self.battle_overlay_open() && self.game.placement.is_none() && self.game.spawn_tool.is_none() && self.game.selection.is_empty()));
                if !left_photo && (escape || menu_key) {
                    self.in_game_menu = !self.in_game_menu;
                }
                // (Tutorial enemies are players without a computer: count connections.)
                // The menu closed: RESIGN disarms, and the statistics page is left (the next
                // F2 opens the menu itself).
                if !self.in_game_menu {
                    self.game.resign_armed = false;
                    self.game.stats_open = false;
                }
                let single_player = self.game.net.is_none();
                let frozen = self.in_game_menu && single_player;
                if !frozen {
                    let input_free = (self.window.has_focus() || self.exit_after_screenshot || self.film.is_some()) && !self.battle_overlay_open() && !self.egui.ctx.text_edit_focused();
                    if input_free { self.update_camera(dt); }
                    self.result_camera(dt);
                    let ui_pointer = self.egui.ctx.egui_wants_pointer_input() || self.egui.ctx.is_pointer_over_egui();
                    self.game.ppp = self.egui.ctx.pixels_per_point();
                    let free = self.hud.free_view(&self.egui.ctx);
                    self.game.battle_view = Some((Vec2::new(free.min.x, free.min.y) * self.game.ppp, Vec2::new(free.max.x, free.max.y) * self.game.ppp));
                    // The classic interface's page takes TA's own letters (from its last frame).
                    self.game.keys.claimed = self.hud.classic_claims();
                    let keyboard_free = input_free && !self.egui.ctx.egui_wants_keyboard_input();
                    let m = self.input.modifiers;
                    let letters = if keyboard_free && !m.control_key() && !m.alt_key() { self.input.pressed_keys() } else { Vec::new() };
                    self.hud.set_classic_keys(letters);
                    if self.benchmark.is_some() || !input_free {
                        self.game.cancel_pointer_gestures();
                    } else {
                        self.game.handle_input(&self.input, &self.camera, &self.renderer, ui_pointer);
                    }
                    self.game.update(dt);
                    if let Some(t) = self.tutorial.as_mut() {
                        t.update(&mut self.game, &self.camera, dt);
                    }
                    if let Some(c) = self.coach.as_mut() {
                        c.update(&self.game);
                    }
                } else {
                    self.game.cancel_pointer_gestures();
                    self.hud.set_classic_keys(Vec::new());
                }
                }
            }
            // Their frames are `editor_frame` and `space_frame`.
            Screen::Editor | Screen::Space => {}
        }
        if self.perspective.is_none() {
            self.clamp_camera(dt);
        }
        self.film_after_update(dt);
        self.clip_previews_after_update();
        if let Some(at) = self.game.radial.as_mut().and_then(|r| r.warp.take()) {
            self.input.mouse = at;
            self.pad.cursor = Some(at);
            let _ = self.window.set_cursor_position(winit::dpi::PhysicalPosition::new(at.x as f64, at.y as f64));
        }
        if let Some(at) = self.keyboard_pointer.snap_target(&self.game, &self.camera) {
            self.input.mouse = at;
            self.pad.cursor = Some(at);
            let _ = self.window.set_cursor_position(winit::dpi::PhysicalPosition::new(at.x as f64, at.y as f64));
        }
        // TA's victory or defeat music when the battle is decided (once; a new battle starts at
        // peace).
        if let Some(outcome) = self.game.outcome.filter(|_| !self.outcome_music && self.screen == Screen::Playing) {
            self.audio.set_mood(if outcome == Outcome::Victory { ta_audio::Mood::Victory } else { ta_audio::Mood::Defeat });
            self.outcome_music = true;
            // The camera to where it was decided: the enemy Commander that fell last, or ours,
            // within the last minute.
            let (me, now) = (self.game.local_player, self.game.sim.tick);
            let sim = &self.game.sim;
            let decided = self.game.tally.chronicle.commanders.iter().rev().filter(|m| now.saturating_sub(m.tick) <= 30 * 60).find(|m| match outcome {
                Outcome::Victory => sim.is_enemy(m.player, me),
                Outcome::Defeat => m.player == me,
            });
            self.result_glide = decided.map(|m| m.pos).filter(|_| !self.settings.gameplay.reduced_motion);
            if self.result_glide.is_some() {
                self.game.follow = None;
            }
        }
        self.checkpoint_profile(false);
        let genuine_play = self.profile_eligible && self.profile_editor.is_none() && self.screen == Screen::Playing
            && !self.game.spectator && self.game.playback.is_none() && !self.game.staged && !self.game.cheated()
            && !self.game.profile_progress.disqualified;
        // Capture a completed result only once. Failed disk writes remain in a separate
        // queue, so continued play cannot inflate times or change Conquest survivors.
        if genuine_play && !self.progress_recorded && self.game.outcome == Some(Outcome::Victory) && let Some(ms) = &self.game.setup.mission {
            self.progress.complete(&ms.campaign, ms.index);
            self.progress_writes.progress(progress::Update::Mission { campaign: ms.campaign.clone(), index: ms.index });
            self.progress_recorded = true;
        }
        if genuine_play && !self.progress_recorded && self.game.setup.conquest.is_some() && let Some(outcome) = self.game.outcome {
            if let Some(result) = crate::conquest::result_of(&self.game, outcome == Outcome::Victory) {
                self.progress_writes.conquest(result);
            }
            self.progress_recorded = true;
        }
        if genuine_play && !self.progress_recorded && let (Some(outcome), Some(run)) = (self.game.outcome, &self.game.scenario) {
            let secs = (self.game.sim.tick / 30) as u32;
            let difficulty = crate::scenario::difficulty_key(run.difficulty);
            let best = self.progress.record_scenario(&run.id, difficulty, outcome == Outcome::Victory, secs, run.kind == crate::scenario::Kind::Endless);
            self.progress_writes.progress(progress::Update::Scenario { id: run.id.clone(), difficulty: difficulty.into(), won: outcome == Outcome::Victory, seconds: secs, endless: run.kind == crate::scenario::Kind::Endless });
            if best && (outcome == Outcome::Victory || run.kind == crate::scenario::Kind::Endless) {
                self.game.message(ta_ui::i18n::t("A new best time!"));
            }
            self.progress_recorded = true;
        }
        if genuine_play && let Some(t) = self.tutorial.as_ref().filter(|t| t.finished && !self.progress.tutorials.contains(&(t.lesson as u8))) {
            self.progress.complete_tutorial(t.lesson);
            self.progress_writes.progress(progress::Update::Tutorial(t.lesson));
        }
        self.flush_progress(false);
        // The memory: logged each minute of a battle, and the battle kept safe when it climbs
        // too far (`app/memory_guard.rs`).
        self.memory_watch(now);
        // Autosave: single-player games, every five minutes of game time, one rolling file.
        let g = &self.game;
        if self.screen == Screen::Playing && self.settings.gameplay.autosave && g.net.is_none() && g.playback.is_none() && !g.spectator && g.outcome.is_none() && !g.tutorial && self.script.is_empty() {
            if self.autosave.due(g.sim.tick, now) && let Some(ticks) = g.record.snapshot() {
                let title = format!("Autosave · {}", record_title(g, &self.assets));
                let record = crate::replay::Record::new(crate::replay::RecordKind::Save, self.assets.data_hash, self.assets.mods.record(), g.setup.clone(), ticks, g.sim.tick, title).with_profile(g).with_duties(g);
                match record.write_as("autosave") {
                    Ok(_) => {
                        self.autosave.saved(self.game.sim.tick);
                        self.game.saved_at = Some(self.game.sim.tick);
                        self.game.done(ta_ui::i18n::t("Autosaved"));
                    }
                    Err(e) => {
                        self.autosave.failed(now);
                        log::warn!("autosave failed: {e}");
                        self.game.trouble(ta_ui::i18n::tf("Could not autosave: {error}", &[("error", &e)]));
                    }
                }
            }
        }
        // Quick save (single player), one rolling file; the main menu's Continue loads it.
        if self.screen == Screen::Playing && self.game.keys.pressed(&self.input, "quick_save") {
            let g = &self.game;
            if g.net.is_none() && g.playback.is_none() && !g.spectator && g.outcome.is_none() && !g.tutorial && let Some(ticks) = g.record.snapshot() {
                let title = ta_ui::i18n::tf("Quick save · {game}", &[("game", &record_title(g, &self.assets))]);
                let record = crate::replay::Record::new(crate::replay::RecordKind::Save, self.assets.data_hash, self.assets.mods.record(), g.setup.clone(), ticks, g.sim.tick, title).with_profile(g).with_duties(g);
                match record.write_as("quicksave") {
                    Ok(_) => {
                        self.game.saved_at = Some(self.game.sim.tick);
                        self.game.done(ta_ui::i18n::t("Game saved"));
                    }
                    Err(e) => self.game.trouble(ta_ui::i18n::tf("Could not save: {error}", &[("error", &e)])),
                }
            } else if !g.record.available() {
                self.game.message(ta_ui::i18n::t(crate::replay::RECORDING_STOPPED));
            } else {
                self.game.message(ta_ui::i18n::t("Only single-player games can be saved"));
            }
        }
        // Route game events and voice requests to audio.
        // In a unit, the ears are in it too. Place them before new sounds are culled/panned.
        let ear = self.perspective.as_ref().filter(|p| p.lost.is_none() && !p.leaving).map(|p| (p.eye, p.forward));
        self.audio.set_listener(&self.camera, ear);
        let local = if self.game.spectator { 255 } else { self.game.local_player };
        let viewer = self.game.viewer();
        for e in std::mem::take(&mut self.game.events_out) {
            self.audio.on_event(&e, &self.game.sim, local, viewer);
        }
        self.audio.sing = self.game.view_cheats.sing;
        for (u, ev) in std::mem::take(&mut self.game.voices) {
            self.audio.unit_voice(&self.game.sim, u, ev);
        }
        for request in std::mem::take(&mut self.game.music_requests) {
            self.audio.music_request(request);
        }
        if std::mem::take(&mut self.game.chat_beep) {
            self.audio.play_interface("MessageArrived");
        }
        for event in std::mem::take(&mut self.game.ui_sounds) {
            self.audio.play_interface(event);
        }
        for (name, pos, volume) in std::mem::take(&mut self.game.effects.sounds) {
            self.audio.play_ambient(name, pos, volume);
        }
        if self.screen != Screen::Intro {
            self.audio.set_wind(self.game.sim.wind.to_f32(), &self.camera, dt);
        }
        // (The film's battle under the menu's music, quieter behind a page, fading out and in with
        // each cut; silent while a briefing is read out, as the music is.)
        let on_title = self.menu.page == crate::menu::Page::Title && !self.options_open;
        let briefing = self.audio.briefing_speaking();
        let film_level = self
            .menu_film
            .as_ref()
            .filter(|_| self.screen == Screen::Menu && self.showroom.is_none())
            .map(|f| if briefing { 0.0 } else { f.sound_level(&self.game, on_title) });
        // (Quiet while the EXPORT CLIP dialog's pictures are taken: the battle jumps about behind it.)
        let previews = self.clip_previews.is_some() && self.screen == Screen::Playing;
        self.audio.scene_level = if let Some(level) = self.credits_sound_level().filter(|_| self.screen == Screen::Menu) {
            level
        } else if let Some(level) = film_level {
            level
        } else if previews {
            0.0
        } else {
            1.0
        };
        self.audio.update(&self.camera, ear, dt);

        // UI. (TA's ~ steps through when units show health bars, and keeps the choice.)
        if self.screen == Screen::Playing && !self.egui.ctx.egui_wants_keyboard_input() && self.game.keys.pressed(&self.input, "health_bars") {
            use ta_config::HealthBars as H;
            let (next, say) = match self.settings.gameplay.health_bars {
                H::Auto => (H::Damaged, ta_ui::i18n::n("Health bars: every damaged unit")),
                H::Damaged => (H::Selected, ta_ui::i18n::n("Health bars: the selection only")),
                H::Selected => (H::Auto, ta_ui::i18n::n("Health bars: our damaged units, up close")),
            };
            self.settings.gameplay.health_bars = next;
            self.game.message(ta_ui::i18n::t(say));
            self.save_settings();
        }
        self.game.health_bars = self.settings.gameplay.health_bars;
        self.game.ta_cursors = self.settings.gameplay.ta_cursors;
        self.game.team_markers = self.settings.gameplay.team_markers;
        // (A benchmark's scenes have their icons or not as each was staged, whatever the player's.)
        self.game.strategic_icons = self.settings.gameplay.strategic_icons || crate::benchmark::running();
        self.game.cursor_scale = self.settings.gameplay.cursor_scale;
        self.game.high_contrast_cursor = self.settings.gameplay.high_contrast_cursor;
        self.game.message_duration = self.settings.gameplay.message_duration;
        self.game.order_button = self.settings.gameplay.order_button;
        self.game.radial_enabled = self.settings.gameplay.radial_menu;
        self.game.radial_opens = self.settings.gameplay.radial_opens;
        self.hud.classic = self.settings.gameplay.classic_interface || self.hud.force_classic;
        self.hud.set_classic_look(classic_look(&self.settings.gameplay));
        self.game.keys.sync(&self.settings.keys);
        crate::theme::set_zoom(&self.egui.ctx, self.ui_zoom());
        self.renderer.set_render_scale(self.settings.graphics.render_scale);
        self.renderer.set_art_style(art_style_reborn(&self.settings.graphics));
        crate::palette::set_colour_blind(self.settings.gameplay.colour_blind);
        self.renderer.set_colour_blind(self.settings.gameplay.colour_blind);
        ta_ui::motion::set_reduced(&self.egui.ctx, self.settings.gameplay.reduced_motion);
        ta_ui::motion::set_reduced_flashes(&self.egui.ctx, self.settings.gameplay.reduced_flashes);
        let mut raw = self.egui.take_input(&self.window);
        self.capture_time(&mut raw);
        self.feed_pad_to_egui(&mut raw);
        self.ui_script();
        raw.events.append(&mut self.demo_ui);
        self.battle_keys_only(&mut raw);
        let screen = self.screen;
        // (The hints on the interface come when the pointer rests a moment, a little longer in a
        // battle, where it crosses the deck on its way to the map.)
        let hint_delay = if screen == Screen::Playing { 0.8 } else { 0.5 };
        if (self.egui.ctx.global_style().interaction.tooltip_delay - hint_delay).abs() > 0.001 {
            self.egui.ctx.all_styles_mut(|style| style.interaction.tooltip_delay = hint_delay);
        }
        let on_glass = self.renderer.cockpit_instruments().is_some();
        if self.dev.visible {
            self.dev.particles = self.renderer.particle_stats();
            self.dev.gpu_times = self.renderer.gpu_timings().iter().map(|p| (p.pass, p.ms)).collect();
        }
        if self.dev.gpu_timing != self.dev.gpu_timing_on {
            self.dev.gpu_timing_on = self.renderer.set_gpu_timing(self.dev.gpu_timing) && self.dev.gpu_timing;
            if self.dev.gpu_timing && !self.dev.gpu_timing_on {
                self.dev.gpu_timing = false; // not supported by this device
            }
        }
        let keyboard_pointer_hint = self.settings.gameplay.keyboard_pointer && self.keyboard_pointer_available();
        let tip = self.tip.filter(|_| self.game.sim.tick < crate::tips::SHOWN_TICKS && !keyboard_pointer_hint);
        let msaa_started = self.msaa_started;
        let ray_tracing_running = self.renderer.ray_tracing_available();
        let native = self.window.current_monitor().map(|m| (m.size().width, m.size().height));
        let gpu_kind = gpu_kind(self.renderer.gpu.adapter.get_info().device_type);
        // (A wallpaper the size of the screen the game is on.)
        self.menu.encyclopaedia.screen = native;
        let before = self.settings.clone();
        let mut changes = ta_ui::settings::Changes::default();
        let settings_view = &mut self.settings_view;
        let dev = &mut self.dev;
        let renderer = &mut *self.renderer;
        let lighting = &mut renderer.lighting;
        let post = &mut renderer.post.settings;
        let camera = &mut self.camera;
        let hud = &mut self.hud;
        let minimap = &mut self.minimap;
        let game = &mut self.game;
        let assets = &self.assets;
        // (Where the way out of a battle leads: the page it came from.)
        let back_to = self.menu.page;
        let menu = &mut self.menu;
        let movie = &mut self.movie;
        let audio = &mut self.audio;
        let in_game_menu = &mut self.in_game_menu;
        let progress = &self.progress;
        let settings = &mut self.settings;
        let controls_open = &mut self.controls_open;
        let options_open = &mut self.options_open;
        let loading = self.loading.clone();
        let curtain = &mut self.curtain;
        let tutorial = &mut self.tutorial;
        let coach = &mut self.coach;
        let photo = &mut self.photo;
        let perspective = &mut self.perspective;
        let cockpit_pics = &mut self.cockpit_pics;
        let open_photo = &mut self.open_photo;
        let reel = &mut self.reel;
        let window_px = (self.window.inner_size().width, self.window.inner_size().height);
        let battle_hours = time_of_day(&settings.graphics, crate::scene::game_seconds(game.sim.tick, game.alpha()));
        let mut transition = None;
        let mut showroom_request = None;
        let mut record_mods_request = None;
        let mut watch = false;
        let mut options_closed = false;
        // (A tip put away, and the hints the player asked to hear no more of: kept in the settings.)
        let mut tip_put_away = false;
        let mut hints_changed = false;
        menu.film = self.menu_film.as_ref().filter(|_| self.showroom.is_none()).map(|f| f.showing(game));
        menu.credits = self.credits_show.as_ref().map(|s| s.showing());
        let (now_playing, music_on) = audio.now_playing();
        let ta_tracks = if self.jukebox.open { audio.ta_tracks() } else { Vec::new() };
        let music_position = audio.music_position();
        let music_source = audio.music_source();
        // (The music's spectrum is only worked out while the jukebox shows it.)
        let mut music_bands = [0.0f32; crate::jukebox::BANDS];
        if self.jukebox.open && music_on {
            audio.music_spectrum(&mut music_bands);
        }
        let jukebox = &mut self.jukebox;
        let mut jukebox_requests = Vec::new();
        let room = self.intro_room.as_ref().map(|r| (r.ready, r.fade()));
        let bench = self.benchmark.as_ref();
        let bench_seconds = self.stage.as_ref().filter(|_| screen == Screen::Playing).map(|r| r.seconds(game));
        // (TA's cursor is drawn afresh each frame, if at all.)
        hud.begin_frame();
        let profile_editor = &mut self.profile_editor;
        let career_window = &mut self.career_window;
        let profile = &self.profile;
        let career = &self.career;
        let achievement_toasts = &mut self.achievement_toasts;
        let mut profile_draft = None;
        let mut open_career = false;
        let mut career_action = None;
        let seeking = game.catch_up_progress();
        let film = &mut self.film;
        let mut film_action = None;
        let output = self.egui.ctx.run_ui(raw, |ui| {
            // A replay being made into a film: how it goes, nothing else.
            if let Some(film) = film.as_mut() {
                film_action = film.page(ui, seeking);
                return;
            }
            if let Some(editor) = profile_editor {
                let room = ui.ctx().content_rect();
                let panel = ta_ui::panel::Panel::new("Create your commander profile")
                    .width(460.0_f32.min((room.width() - 56.0).max(180.0)))
                    .max_height((room.height() - 24.0).max(96.0));
                profile_draft = ta_ui::panel::modal(ui.ctx(), "game_profile_setup", panel, false, |ui| editor.show(ui)).inner;
                return;
            }
            if career_window.is_open() && let Some(profile) = profile {
                career_action = career_window.show(ui.ctx(), profile);
                return;
            }
            if let Some(b) = bench {
                b.overlay(ui.ctx(), bench_seconds, dev.fps, "Esc");
            }
            match screen {
                Screen::Intro => {
                    if let Some(m) = movie.as_mut() {
                        m.update(ui.ctx(), audio);
                        if !m.for_room {
                            m.draw(ui);
                        }
                    }
                    // The room: dark until it is built; fading out as the camera leans in.
                    if let Some((ready, fade)) = room {
                        let dark = if ready { fade } else { 1.0 };
                        if dark > 0.0 {
                            ui.painter().rect_filled(ui.ctx().content_rect(), 0.0, egui::Color32::from_black_alpha((dark * 255.0) as u8));
                        }
                    }
                }
                Screen::Menu if *options_open => {
                    if options_panel(ui.ctx(), settings_view, settings, msaa_started, ray_tracing_running, native, gpu_kind, &mut changes) {
                        *options_open = false;
                        options_closed = true;
                    }
                }
                Screen::Menu => match menu.draw(ui, assets, progress, game) {
                    MenuAction::Watch => watch = true,
                    MenuAction::Options => *options_open = true,
                    MenuAction::Start(setup) => transition = Some(Transition::StartGame(setup)),
                    MenuAction::PlayIntro => transition = Some(Transition::PlayIntro),
                    MenuAction::Quit => transition = Some(Transition::Quit),
                    MenuAction::Narrate(Some(path)) => audio.play_briefing(&path),
                    MenuAction::Narrate(None) => audio.stop_narration(),
                    MenuAction::StartNet(setup, session) => transition = Some(Transition::StartNet(setup, session)),
                    MenuAction::Play(record) => transition = Some(Transition::Play(record)),
                    action @ (MenuAction::RestartWithMods { .. } | MenuAction::GetModInLauncher(_)) => record_mods_request = Some(action),
                    MenuAction::StartTutorial(n) => transition = Some(Transition::StartTutorial(n)),
                    MenuAction::Editor => transition = Some(Transition::Editor(crate::editor::Start::New(crate::editor::default_new(assets)), Some("new".into()))),
                    MenuAction::Show { unit, map, hours, act } => showroom_request = Some(ShowroomRequest::Show(unit, map, hours, act)),
                    MenuAction::LeaveShowroom => showroom_request = Some(ShowroomRequest::Leave),
                    MenuAction::Wallpaper { width, height, words } => showroom_request = Some(ShowroomRequest::Wallpaper(width, height, words)),
                    MenuAction::Jukebox => jukebox.open = !jukebox.open,
                    MenuAction::None => {}
                },
                Screen::Playing if photo.is_some() => {
                    if let Some(p) = photo.as_mut() {
                        p.draw(ui.ctx(), camera, window_px, battle_hours, game.net.is_some());
                    }
                }
                Screen::Playing if perspective.is_some() => {
                    if let Some(p) = perspective.as_mut() {
                        let now = ui.ctx().input(|i| i.time);
                        let ctx = ui.ctx().clone();
                        let owner = game.sim.unit(p.unit).map_or(game.local_player, |u| u.owner);
                        let type_id = p.type_id;
                        crate::perspective_hud::draw(p, ui.ctx(), game, camera, cockpit_pics, &assets.vfs, now, on_glass, &mut |size| hud.model_portrait(&ctx, game, type_id, size, owner));
                    }
                }
                Screen::Playing if reel.is_some() => {
                    if let Some(r) = reel.as_mut() {
                        r.draw(ui.ctx(), game);
                    }
                }
                Screen::Playing if hud_hidden() => {}
                // The battle decided: its result takes the screen, the interface stepped aside.
                Screen::Playing if game.outcome.is_some() && !*options_open && !game.history_open => {
                    hud.accessibility.show(ui.ctx(), game, settings.gameplay.screen_reader_announcements);
                    let best = game.debrief.best_type().and_then(|ty| hud.unit_pic_of(ui.ctx(), &assets.vfs, game, ty, 64.0, game.local_player));
                    use crate::result::Choice;
                    // (Its way home goes where the battle menu's way out does: named so.)
                    let next = match crate::result::draw(ui.ctx(), game, best, back_label(back_to)) {
                        // (Beaten while hosting a battle others fight on: our leaving would end it
                        // for them too, so it is asked first.)
                        Some(Choice::Menu) if game.hosting_others() => {
                            game.leave_asked = Some(crate::game::Leave::Menu);
                            None
                        }
                        Some(Choice::Menu) => {
                            // (After a mission, a Conquest battle or a scenario, MAIN MENU is the
                            // title menu: the menu kept their page, which their own buttons go to:
                            // CONTINUE CAMPAIGN, CONTINUE CONQUEST, SCENARIOS.)
                            menu.page = crate::result::home_page(crate::result::main_menu_to_title_after(game), menu.page);
                            Some(Transition::ToMenu)
                        }
                        Some(Choice::Lobby) => Some(Transition::ToLobby),
                        Some(Choice::Again) => Some(Transition::StartGame(game.setup.clone())),
                        Some(Choice::Campaign { campaign, mission }) => Some(Transition::ToCampaign { campaign, mission }),
                        Some(Choice::Scenarios) => Some(Transition::ToScenarios),
                        Some(Choice::Conquest) => Some(Transition::ToConquest),
                        Some(Choice::Highlights) => Some(Transition::Highlights),
                        Some(Choice::KeepWatching) | None => None,
                    };
                    if next.is_some() {
                        transition = next;
                    }
                    // The host's question, over the result.
                    if game.hosting_others()
                        && let Some(leave) = game.leave_asked
                        && let Some(answer) = leave_question(ui.ctx(), game, assets, leave)
                    {
                        game.leave_asked = None;
                        if answer.is_some() {
                            transition = answer;
                        }
                    }
                }
                Screen::Playing => {
                    hud.accessibility.show(ui.ctx(), game, settings.gameplay.screen_reader_announcements);
                    game.menu_open = *in_game_menu;
                    hud.draw(ui, &assets.vfs, game, camera, minimap);
                    if hud.take_open_menu() {
                        *in_game_menu = true;
                    }
                    let view = hud.free_view(ui.ctx());
                    if keyboard_pointer_hint { crate::keyboard_pointer::hint(ui.ctx(), view, &settings.keys); }
                    if let Some(t) = tutorial.as_mut() {
                        match t.draw(ui.ctx(), game, camera, view, hud.anchors()) {
                            crate::tutorial::TutorialAction::Start(n) => transition = Some(Transition::StartTutorial(n)),
                            crate::tutorial::TutorialAction::Menu => transition = Some(Transition::ToMenu),
                            crate::tutorial::TutorialAction::Skirmish => transition = Some(Transition::ToSkirmish),
                            crate::tutorial::TutorialAction::None => {}
                        }
                        // Its words aloud as they come (a new step's cut the last short).
                        if let Some(words) = t.take_line()
                            && settings.gameplay.tutorial_voice
                            && let Some(path) = crate::tutorial::voice_path(&assets.vfs, words, game.keys.config(), game.order_button)
                        {
                            audio.play_narration(&path);
                        }
                    } else if let Some(tip) = tip.filter(|_| game.chat_input.is_none() && !*in_game_menu && !*options_open && game.outcome.is_none() && !game.stats_open && !game.history_open) {
                        // (Not over the chat while the player types, nor over the menu or a page
                        // open over the battle: it covered their buttons.)
                        match crate::tips::strip(ui.ctx(), &crate::keys::fill(tip, &settings.keys, settings.gameplay.order_button), game.sim.tick, view) {
                            Some(crate::hint::Pressed::Away) => tip_put_away = true,
                            Some(crate::hint::Pressed::Never) => {
                                tip_put_away = true;
                                settings.gameplay.start_tips = false;
                                hints_changed = true;
                            }
                            _ => {}
                        }
                    }
                    // (Not over the chat, the menu or a page open over the battle either.)
                    if let Some(c) = coach.as_mut().filter(|_| game.chat_input.is_none() && !*in_game_menu && !*options_open && game.outcome.is_none() && !game.stats_open && !game.history_open) {
                        if c.draw(ui.ctx(), game, view).is_some() {
                            settings.gameplay.coach_muted = c.muted().to_vec();
                            hints_changed = true;
                        }
                    }
                    if *options_open {
                        if options_panel(ui.ctx(), settings_view, settings, msaa_started, ray_tracing_running, native, gpu_kind, &mut changes) {
                            *options_open = false;
                            options_closed = true;
                        }
                    } else if !game.history_open && let Some(t) = in_game_overlays(ui.ctx(), game, assets, in_game_menu, options_open, controls_open, open_photo, back_to, view.shrink(16.0)) {
                        transition = Some(t);
                    }
                    // "Change keys" in the reference opens the options at the controls. (It keeps
                    // clear of the interface's bars.)
                    if *controls_open && crate::controls::window(ui.ctx(), controls_open, &settings.keys, &settings.gameplay, view.shrink(16.0)) {
                        settings_view.open(ta_ui::settings::Category::Controls);
                        *options_open = true;
                    }
                }
                Screen::Editor | Screen::Space => {}
            }
            // The commander's identity card, on the title screen (`shows_identity_card`).
            let film_watch = menu.film.as_ref().map_or(0.0, |f| f.watch);
            if screen == Screen::Menu && shows_identity_card(menu.page, *options_open, film_watch) && let Some(profile) = profile {
                egui::Area::new(egui::Id::new("game_profile_identity"))
                    .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-18.0, 14.0))
                    .show(ui.ctx(), |ui| {
                        ui.multiply_opacity(1.0 - film_watch);
                        ui.set_width(250.0_f32.min((ui.ctx().content_rect().width() - 36.0).max(160.0)));
                        // (In the title menu's own dark slot in a steel rim, lit when pointed at.)
                        let slot = ui.painter().add(egui::Shape::Noop);
                        let card = egui::Frame::new().inner_margin(egui::Margin::symmetric(11, 10)).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ta_ui::profiles::avatar(ui, profile, 42.0);
                                ui.vertical(|ui| {
                                    ui.set_width((ui.available_width() - 40.0).max(60.0));
                                    ui.add(egui::Label::new(egui::RichText::new(&profile.name).strong()).truncate());
                                    ui.add(egui::Label::new(ta_ui::profiles::rank_text(career)).truncate());
                                });
                                ta_ui::profiles::rank_insignia(ui, career, 36.0);
                            });
                            ta_ui::profiles::xp_progress(ui, career);
                        });
                        let response = card.response.interact(egui::Sense::click()).on_hover_text(ta_ui::i18n::t("VIEW PROFILE")).on_hover_cursor(egui::CursorIcon::PointingHand);
                        let hover = ui.ctx().animate_bool_with_time(response.id.with("hover"), response.hovered() || response.has_focus(), 0.12);
                        let rect = card.response.rect;
                        ui.painter().set(slot, egui::Shape::Vec(vec![
                            ta_ui::metal::slab(ui.ctx(), rect, ta_ui::metal::Slab::Steel, hover * 0.6, false),
                            ta_ui::metal::well(rect.shrink(3.0), egui::Color32::from_rgb(20, 19, 16).lerp_to_gamma(egui::Color32::from_rgb(38, 35, 29), hover)),
                        ]));
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), ta_ui::i18n::t("VIEW PROFILE")));
                        ta_ui::widgets::reveal_focus(ui, &response);
                        ta_ui::widgets::focus_ring(ui, &response, response.rect);
                        if response.clicked() { open_career = true; }
                    });
            }
            // Achievements earned: their notices at the top right (on the title screen, under
            // the commander's card).
            if loading.is_none() && matches!(screen, Screen::Menu | Screen::Playing) && !achievement_toasts.is_empty() {
                let room = if screen == Screen::Playing && !hud_hidden() { hud.notices_room(ui.ctx()) } else {
                    let r = ui.ctx().content_rect();
                    egui::Rect::from_min_max(egui::pos2(r.min.x, r.min.y + if screen == Screen::Menu { 104.0 } else { 0.0 }), r.max)
                };
                achievement_toasts.show(ui.ctx(), room);
            }
            if matches!(screen, Screen::Menu | Screen::Playing) {
                let music = crate::jukebox::Music { track: now_playing.as_deref(), on: music_on, position: music_position, bands: &music_bands };
                // (Clear of the title screen's footer, and its shadow.)
                let bottom = if screen == Screen::Menu { 88.0 } else { 20.0 };
                jukebox_requests = jukebox.draw(ui.ctx(), &ta_tracks, &music, &music_source, &settings.audio, bottom);
            }
            if dev.visible {
                draw_dev_panel(ui.ctx(), dev, lighting, post, camera, game);
            }
            // The player's frame rate counter (Display: FRAME RATE COUNTER), in battle (not in a
            // benchmark, whose own overlay counts the frames).
            if settings.graphics.show_frame_rate && screen == Screen::Playing && loading.is_none() && !crate::benchmark::running() {
                frame_rate_counter(ui.ctx(), &dev.meter, game.battle_view, game.ppp);
            }
            // The pages and dialogs no longer drawn go out (a page sliding away as the next
            // comes in, a dialog sinking as it closes).
            ta_ui::panel::exits(ui.ctx());
            // A loading screen's bay lifting off what it loaded, over everything.
            if let Some(since) = curtain {
                const LIFT: f32 = 0.45;
                let now = ui.input(|i| i.time);
                let p = ta_ui::motion::progress(now, *since.get_or_insert(now), LIFT);
                if p < 1.0 {
                    ta_ui::loading::curtain(ui.ctx(), 1.0 - ta_ui::motion::smooth(p));
                    ui.ctx().request_repaint();
                } else {
                    *curtain = None;
                }
            }
            if let Some(title) = &loading {
                ta_ui::loading::LoadingScreen::new(ta_ui::loading::LoadingKind::Game)
                    .with_seasonal(&settings.launcher, settings.gameplay.reduced_flashes)
                    .show(ui.ctx(), ta_ui::i18n::t("Preparing battlefield"), title, ui.input(|i| i.time));
            }
        });
        if options_closed || hints_changed {
            self.save_settings();
        }
        if tip_put_away {
            self.tip = None;
        }
        if let Some(draft) = profile_draft { self.create_profile(draft); }
        if open_career { self.open_career(false); }
        // A lobby made on the Multiplayer page: its port and name, or its address, kept for the
        // next time; a game joined goes first among those joined lately, with the battle's name
        // once its lobby tells it.
        let learned = self.menu.lobby.as_mut().and_then(crate::lobby::NetLobby::take_learned);
        if let Some(keep) = self.menu.connect.keep.take() {
            // The name hosted or joined under is the player's from now on, as Options' COMMANDER
            // NAME is (with a profile, its commander's): it was lost at the next start.
            let renamed = matches!(keep, crate::lobby::Keep::Hosted { .. } | crate::lobby::Keep::Joined(_))
                .then(|| self.menu.connect.name.trim().to_string())
                .filter(|name| !name.is_empty() && *name != self.settings.gameplay.player_name);
            let g = &mut self.settings.gameplay;
            match keep {
                crate::lobby::Keep::Hosted { port, title } => {
                    g.host_port = port;
                    g.host_title = title;
                }
                crate::lobby::Keep::Joined(address) => {
                    g.join_address = address.clone();
                    let title = g.recent_games.iter().find(|r| r.address == address).map(|r| r.title.clone()).unwrap_or_default();
                    g.recent_games.retain(|r| r.address != address);
                    g.recent_games.insert(0, ta_config::RecentGame { address, title });
                    g.recent_games.truncate(crate::lobby::connect_recent());
                }
                crate::lobby::Keep::Forget(address) => g.recent_games.retain(|r| r.address != address),
            }
            self.menu.connect.recent = g.recent_games.iter().map(|r| (r.address.clone(), r.title.clone())).collect();
            if let Some(name) = renamed {
                self.rename_commander(name);
            }
            self.save_settings();
        }
        if let Some((address, title)) = learned
            && let Some(r) = self.settings.gameplay.recent_games.iter_mut().find(|r| r.address == address)
            && r.title != title
        {
            r.title = title;
            self.menu.connect.recent = self.settings.gameplay.recent_games.iter().map(|r| (r.address.clone(), r.title.clone())).collect();
            self.save_settings();
        }
        if let Some(ta_ui::profiles::CareerAction::Showcase(list)) = career_action && let Some(profile) = &self.profile {
            match ta_config::profiles::ProfileStore::open(ta_config::config_dir()).and_then(|store| store.set_showcase(&profile.id, &list)) {
                Ok(profile) => self.profile = Some(profile),
                Err(error) => log::warn!("could not change the profile's showcase: {error}"),
            }
        }
        if transition.is_some() {
            self.pending = transition;
        }
        for request in jukebox_requests {
            use crate::chat::MusicRequest as M;
            use crate::jukebox::JukeboxRequest as J;
            match request {
                J::Play(track) => self.audio.music_request(M::Track(track)),
                J::Previous => self.audio.music_request(M::Previous),
                J::Next => self.audio.music_request(M::Next),
                J::Stop => self.audio.music_request(M::Stop),
                J::Start => self.audio.music_request(M::Start),
                J::Seek(seconds) => self.audio.seek_music(seconds),
                J::Volume { level, done } => {
                    self.settings.audio.music = level;
                    self.apply_jukebox_settings(done);
                }
                J::Shuffle(on) => {
                    self.settings.audio.music_shuffle = on;
                    self.apply_jukebox_settings(true);
                }
                J::Repeat(on) => {
                    self.settings.audio.music_repeat = on;
                    self.apply_jukebox_settings(true);
                }
                J::Mode(mode) => {
                    self.settings.audio.user_music = mode;
                    self.save_settings();
                    self.apply_user_music();
                }
                J::Rescan => self.apply_user_music(),
            }
        }
        if watch && let Some(film) = self.menu_film.as_mut() {
            film.watching = true;
        }
        match showroom_request {
            Some(ShowroomRequest::Show(unit, map, hours, act)) => {
                if let Err(e) = self.show_unit(&unit, &map, hours, act) {
                    log::warn!("showroom: {e}");
                }
            }
            Some(ShowroomRequest::Leave) => {
                self.showroom = None;
                self.renderer.post.settings.dof = None;
                self.camera.view_offset = Vec2::ZERO;
                self.pending = Some(Transition::RestoreMenuBackdrop);
            }
            Some(ShowroomRequest::Wallpaper(width, height, words)) => self.wallpaper(width, height, words),
            None => {}
        }
        match record_mods_request {
            Some(MenuAction::RestartWithMods { path, replay, order }) => self.restart_with_mods(&path, replay, order),
            Some(MenuAction::GetModInLauncher(id)) => self.get_mod_in_launcher(&id),
            _ => {}
        }
        self.apply_settings(&before, changes);
        if let Some(action) = film_action {
            self.film_page_action(action);
        }
        if let Some(settings) = self.hud.take_film_request() {
            self.start_film(settings);
        }
        // The EXPORT CLIP dialog's pictures: those it asks for taken, and when it closes without a
        // clip, the replay back where it was.
        let shots = self.hud.take_clip_shots();
        self.queue_clip_previews(shots);
        if !self.hud.clip_dialog_open() && self.clip_previews.is_some() && self.film.is_none() {
            self.end_clip_previews();
        }
        // Multiplayer: what the interface ordered goes out now, with the frame's other orders,
        // rather than with the next frame's.
        if let Some(net) = self.game.net.as_mut() {
            net.flush();
        }
        // (TA's own sound for what was done, queueing a unit say, takes the button's place.)
        if let Some(sound) = ta_ui::sound::take(&self.egui.ctx)
            && self.game.ui_sounds.is_empty()
        {
            self.audio.play_interface(sound.event());
        }
        // TA's cursor drawn: the system's stays hidden, also over a dialog drawn after it (the
        // pause menu, the controls) that asked for the pointing hand over its buttons.
        let mut platform = output.platform_output;
        // (Nor while the menu's film plays on its own, once the menu has stepped aside: a move
        // brings both back.)
        let film_alone = self.screen == Screen::Menu && self.menu_film.as_ref().is_some_and(|f| f.watching && f.watched() > crate::backdrop::HIDE_POINTER_AFTER);
        if self.benchmark.is_some() || (self.screen == Screen::Playing && self.hud.ta_cursor_drawn()) || film_alone {
            platform.cursor_icon = egui::CursorIcon::None;
        }
        self.egui.state.handle_platform_output(&self.window, platform);
        // (A Mac's system pointer kept off TA's: the pointer the window wants set again now and
        // then, see macos.rs.)
        #[cfg(target_os = "macos")]
        crate::macos::refresh_cursor_now_and_then(&self.window);
        let ppp = output.pixels_per_point;
        let prims = self.egui.ctx.tessellate(output.shapes, ppp);
        let mut textures_delta = output.textures_delta;
        for (id, deltas) in textures_delta.set.drain() {
            for delta in deltas {
                self.egui.renderer.update_texture(&self.renderer.gpu.device, &self.renderer.gpu.queue, id, &delta);
            }
        }

        // The portraits of units the interface asked for, drawn now, before the frame's models are
        // prepared (they draw with the same buffers): shown from the next frame on.
        // (By size: the studio keeps the targets of the last size it drew.)
        let mut wanted = self.hud.take_portrait_requests();
        // (And the menus': the Archives' list and card, Conquest's veterans.)
        let menus = self.menu.portraits.take_requests();
        for key in &menus {
            if !wanted.contains(key) {
                wanted.push(*key);
            }
        }
        wanted.truncate(PORTRAITS_PER_FRAME);
        wanted.sort_by_key(|k| k.2);
        for key @ (type_id, colour, size, enemy) in wanted {
            let id = self.game.scene.model_for_side(type_id, enemy).and_then(|model| {
                let (pieces, ground) = (crate::scene::standing_pose(self.game.sim.types.get(type_id)), self.game.sim.types.get(type_id).def.stands_on_land());
                let studio = self.portrait_studio.get_or_insert_with(|| ta_render::portrait::PortraitStudio::new(&self.renderer));
                let picture = studio.render(&mut self.renderer, &ta_render::portrait::PortraitRequest { model, pieces: &pieces, team: colour, size, ground })?;
                let view = picture.create_view(&wgpu::TextureViewDescriptor::default());
                let id = self.egui.renderer.register_native_texture(&self.renderer.gpu.device, &view, wgpu::FilterMode::Linear);
                self.portrait_views.insert(key, view);
                Some(id)
            });
            self.hud.set_portrait(key, id);
            if menus.contains(&key) {
                self.menu.portraits.set(key, id);
            }
        }

        // As for the companion screen: share portrait pixels, but register them with
        // the film's own renderer before the battle's model buffers are prepared.
        self.film_portraits();
        let t_prepare = Instant::now();
        // A staged scene after the frame's ticks: its explosions, its frames drawn as far built as
        // it has them, and its camera (on the capture's fixed clock, else the game's; not while
        // riding in a unit or taking photos).
        let fixed_clock = self.capture_dt().is_some().then_some(self.capture_clock as f32);
        let mut stage_hours = None;
        let mut ride = None;
        if self.screen == Screen::Playing
            && let Some(run) = self.stage.as_mut()
        {
            run.after_update(&mut self.game);
            let clock = fixed_clock.unwrap_or_else(|| run.seconds(&self.game));
            stage_hours = run.hours(clock);
            ride = run.ride_cue(&self.game.sim, clock);
            if self.perspective.is_none() && self.photo.is_none() {
                run.camera(&self.game, &mut self.camera, clock);
            }
        }
        // A scene's ride: into its unit (as the player's key goes in, the next frame; an enemy's
        // as a spectator), or back out.
        match ride {
            Some(crate::stage::RideCue::Enter(id)) => {
                if self.game.sim.unit(id).is_some_and(|u| !self.game.sim.allied(u.owner, self.game.local_player)) {
                    self.game.spectator = true;
                }
                self.game.perspective_request = Some(id);
            }
            Some(crate::stage::RideCue::Leave) => {
                if let Some(p) = self.perspective.as_mut() {
                    p.leave(&mut self.game);
                }
            }
            None => {}
        }
        // The menu's film: its scene's camera (the picture right of the title menu, leaning
        // towards the pointer) and time of day; its lens focused on what it looks at.
        let menu_film_focus = self.menu_film_camera(dt).or_else(|| self.credits_camera());
        if let Some(film) = self.menu_film.as_ref().filter(|_| self.screen == Screen::Menu && self.showroom.is_none()) {
            stage_hours = film.hours(&self.game);
        }
        if let Some(hours) = self.credits_hours() {
            stage_hours = Some(hours);
        }
        // The ground levelled under the buildings put down since the last frame (before anything
        // is set on it).
        self.renderer.add_ground_pads(&self.game.scene.take_pads());
        let (view_min, view_max) = self.camera.ground_bounds(260.0);
        let alpha = self.game.alpha();
        // Game time in f64 (the time of day's cycle taken from it exactly); what keeps it for
        // effects' timestamps gets an f32 of it, which keeps the fraction of a tick for days.
        let seconds = crate::scene::game_seconds(self.game.sim.tick, alpha);
        let time = seconds as f32;
        let photo_hours = self.photo.as_ref().and_then(|p| p.hours).or_else(|| self.showroom.as_ref().filter(|_| self.screen == Screen::Menu).map(|r| r.hours));
        // (`TA_TIME` over a staged scene's time, over a scenario's (night falling, dawn
        // coming), over the settings'.)
        let scenario_hours = self.game.scenario.as_ref().filter(|_| self.screen == Screen::Playing).and_then(|r| r.time.as_ref()).map(|h| h.at(time));
        self.renderer.lighting.time_of_day = photo_hours.or(forced_time()).or(stage_hours).or(scenario_hours).or_else(|| time_of_day(&self.settings.graphics, seconds));
        // Beyond the map's edges: in a battle a sea of dark clouds under it, the battlefield the
        // focus; the land and sea carried on to the horizon in the films (the menu's, a staged
        // scene's, the credits').
        self.renderer.lighting.sky.beyond = beyond_the_map(self.screen == Screen::Playing && self.stage.is_none() && !self.credits_playing());
        // The showroom's sea: clear and calm over a unit under it, so the model reads (TA's own
        // everywhere else).
        let sea = self.showroom.as_ref().filter(|_| self.screen == Screen::Menu).map_or(crate::showroom::Sea::Open, |r| r.sea);
        crate::showroom::stage_water(&mut self.renderer.lighting.water, sea);
        // The battle behind the menus graded down, easing in and out (never the showroom's
        // world: the page is about it, and its pictures are drawn from it).
        if self.showroom.is_some() {
            self.renderer.post.backdrop = 0.0;
        } else {
            // (And the battle behind its result, as the menus' backdrop is.)
            let result = self.screen == Screen::Playing && self.game.outcome.is_some() && self.perspective.is_none() && self.reel.is_none() && !hud_hidden();
            // (The film behind the title menu stepping back a little only, and not at all while
            // it plays on its own: its pictures are the point.)
            let film_title = self.menu_film.as_ref().filter(|_| self.screen == Screen::Menu && self.menu.page == crate::menu::Page::Title && !self.options_open);
            let veiled = self.battle_veiled();
            let backdrop = match film_title {
                Some(film) => crate::backdrop::TITLE_BACKDROP * (1.0 - film.showing(&self.game).watch),
                None if self.credits_playing() => 0.0,
                None if self.screen == Screen::Menu => 1.0,
                None if result => 0.8,
                None if veiled => VEIL_GRADE,
                None => 0.0,
            };
            // (Behind a dialog, out of focus as well; and its backdrop dims it less.)
            self.veil += (if veiled { 1.0 } else { 0.0 } - self.veil) * (1.0 - (-dt * 9.0).exp());
            ta_ui::panel::set_modal_dim(&self.egui.ctx, if veiled { VEIL_DIM } else { ta_ui::panel::MODAL_DIM });
            // (Slowly behind a result: its decisive moment is seen first.)
            let rate = if result { 1.5 } else { 5.0 };
            self.renderer.post.backdrop += (backdrop - self.renderer.post.backdrop) * (1.0 - (-dt * rate).exp());
        }
        // The film's lens: what the scene looks at sharp, the near and the far soft.
        match menu_film_focus {
            Some(focus) => self.renderer.post.settings.dof = Some(crate::backdrop::lens(focus)),
            None if self.menu_film_lens => self.renderer.post.settings.dof = None,
            None => {}
        }
        self.menu_film_lens = menu_film_focus.is_some();
        // The battle behind a dialog, softly out of focus (see `battle_veiled`).
        if self.veil > 0.01 && self.screen == Screen::Playing && self.photo.is_none() && self.perspective.is_none() {
            let blur = VEIL_BLUR * self.veil;
            self.renderer.post.settings.dof = Some(ta_render::post::DepthOfField { mode: ta_render::post::DofMode::Uniform, focus: 0.0, blur, band: 0.0 });
            self.veil_lens = true;
        } else if std::mem::take(&mut self.veil_lens) {
            self.renderer.post.settings.dof = None;
            self.veil = 0.0;
        }
        // The showroom's lens: the unit and the ground round it sharp, the world beyond them
        // falling softly out of focus (in a wallpaper's preview, the wallpaper's own).
        if let Some(room) = self.showroom.as_ref().filter(|_| self.screen == Screen::Menu) {
            self.renderer.post.settings.dof = Some(match self.menu.encyclopaedia.preview() {
                Some((aspect, _)) => crate::showroom::wallpaper_lens(room, &self.camera, self.wallpaper_share(aspect)),
                None => ta_render::post::DepthOfField { mode: ta_render::post::DofMode::Lens, focus: self.camera.view_distance(room.centre), blur: 0.003, band: 0.12 },
            });
        }
        if let Some(photo) = &self.photo {
            self.renderer.post.settings = photo.post(&self.camera);
        }
        self.renderer.post.settings.reduced_motion = self.settings.gameplay.reduced_motion;
        self.renderer.post.settings.reduced_flashes = self.settings.gameplay.reduced_flashes;
        let effect_dt = if self.game.paused || self.game.replay_finished() || self.game.catch_up_progress().is_some() { 0.0 } else { dt * self.game.speed };
        self.game.effects.view = Some((view_min, view_max));
        self.game.effects.detail = crate::effects::Detail::from_settings(&self.settings.graphics);
        self.game.effects.modern = self.camera.mode == ta_render::CameraMode::Perspective;
        self.game.effects.eye = self.game.effects.modern.then(|| self.camera.eye());
        self.game.effects.focus = self.game.effects.modern.then(|| Vec2::new(self.camera.focus.x, self.camera.focus.z));
        self.game.effects.reduced_flashes = self.settings.gameplay.reduced_flashes;
        self.game.scene.reduced_flashes = self.settings.gameplay.reduced_flashes;
        // Burning 3D trees burn in their crowns (last frame's, see `FeatureView::flames`).
        self.game.effects.tree_flames.clone_from(self.game.features.flames());
        self.game.effects.update(&self.game.sim, time, effect_dt, alpha);
        // At night bases, factories, units and aircraft light up.
        let lamps = self.renderer.daylight(&self.camera).map_or(0.0, |d| d.lamps);
        if lamps > 0.0 {
            let mut night = Vec::new();
            crate::night::lights(&self.game, lamps, alpha, time, (view_min, view_max), &mut night);
            self.game.effects.lights.extend(night);
        }
        // The credits' searchlights, sweeping their road.
        if let Some(show) = self.credits_show.as_ref().filter(|s| s.run.is_some()) {
            let clock = show.run.as_ref().map_or(0.0, |r| r.seconds(&self.game).max(0.0));
            let world = &self.game.sim.world;
            show.searchlights(clock, &|x, z| crate::scene::drawn_ground(world, x, z), &mut self.game.effects.lights);
        }
        // And over TA Reborn's racks of fireworks the New Year is seen in.
        let racks = self.game.features.fireworks_in_view(&self.game.sim, (view_min, view_max));
        if !racks.is_empty() {
            self.game.effects.fireworks(&racks, lamps, time, effect_dt);
        }
        self.game.features.modern = self.camera.mode == ta_render::CameraMode::Perspective;
        let foliage = self.foliage_view();
        self.game.scene.camera = Some(self.camera.clone());
        // Reflections and long shadows need objects beyond the visible ground. Keep this
        // expansion out of the HUD/effect bounds above, which describe the picture itself.
        let top = self.game.features.source_height();
        let (source_min, source_max) = self.camera.scene_bounds(top, foliage.mirror, foliage.light);
        let instances = self.game.draw_list(&self.renderer, source_min, source_max, top, time, &foliage);
        let renderer = &mut self.renderer;
        renderer.prepare_models_for_view(&self.camera, instances);
        // Following a unit from behind (Perspective), the leaves and branches between the eye and
        // it give way.
        // (So do they in the showroom, between the camera and the unit on show.)
        let showroom_sight = self.showroom.as_ref().filter(|_| self.screen == Screen::Menu).map(|r| (r.centre, r.radius() * 1.6 + 24.0));
        renderer.prepare_foliage(self.game.features.foliage(), self.perspective.as_ref().filter(|p| p.chase).map(|p| (p.centre, SIGHT_RADIUS)).or(showroom_sight));
        let (decals, billboards, lying) = self.game.features.sprites();
        // Map decals (metal patches, painted shadows) under the battle's scorch marks.
        let mut all_decals = Vec::with_capacity(decals.len() + self.game.effects.decals.len());
        all_decals.extend_from_slice(decals);
        all_decals.extend_from_slice(&self.game.effects.decals);
        renderer.prepare_sprites(&all_decals, billboards, &self.game.effects.additive, &self.game.effects.smoke_sprites, lying);
        renderer.prepare_beams(&self.game.effects.beams);
        // (The selection's rings, and the effects': shockwaves, ripples.)
        let mut rings = self.hud.rings().to_vec();
        rings.extend_from_slice(&self.game.effects.rings);
        renderer.prepare_rings(&rings);
        // (Text in the world: lines of names laid on the ground, a staged scene's or the credits'.)
        let serial = renderer.world_text.serial();
        let glyphs = self.world_text.frame(serial, time, self.settings.gameplay.reduced_flashes);
        renderer.prepare_world_text(glyphs);
        renderer.prepare_wakes(&self.game.effects.wake_points, &self.game.effects.wake_trails);
        renderer.set_lights(&self.game.effects.lights);
        // Strategic zoom: far out, icons over the units (in battle; not in photos, nor with
        // `TA_ICONS=0`, for captures).
        let mut icons = Vec::new();
        if self.screen == Screen::Playing && self.photo.is_none() && self.reel.is_none() && !icons_hidden() {
            crate::strategic::icons(&self.game, &self.camera, ppp, alpha, (view_min, view_max), &mut icons);
        }
        renderer.set_icons(&icons);
        for &(pos, radius, strength) in &self.game.effects.heat {
            renderer.add_heat_source(pos.to_array(), radius, strength);
        }
        self.game.effects.submit_particles(renderer, effect_dt);
        // Map overlays: rebuilt when what they show changes (sight with the vision, movement
        // with the buildings and the selection's type).
        {
            use crate::overlay::{self, Overlay};
            let shown = if self.screen == Screen::Playing && self.photo.is_none() && self.reel.is_none() { self.game.shown_overlay() } else { None };
            let game = &self.game;
            let viewer = game.viewer().unwrap_or(game.local_player);
            let path_type = overlay::path_type(&game.sim, &game.selection, game.local_player);
            let key = shown.map(|o| {
                let v = match o {
                    Overlay::Sight => game.sim.vision.version ^ ((viewer as u64) << 56),
                    Overlay::Path => game.sim.world.occupancy_version ^ ((path_type.unwrap_or(0) as u64) << 40) ^ ((crate::palette::colour_blind() as u64) << 63),
                    _ => 0,
                };
                (o, v)
            });
            if key != self.overlay_key {
                self.overlay_key = key;
                if let (Some(o), Some(terrain)) = (shown, renderer.terrain.as_ref()) {
                    let rgba = match o {
                        Overlay::Metal => Some(overlay::metal(&game.sim)),
                        Overlay::Path => path_type.map(|t| overlay::path(&game.sim, t)),
                        Overlay::Sight => Some(overlay::sight(&game.sim, viewer)),
                        Overlay::Height => None,
                    };
                    if let Some(rgba) = rgba {
                        terrain.set_overlay(&renderer.gpu.queue, &rgba);
                    }
                }
            }
            renderer.set_overlay(shown.map_or(ta_render::MapOverlay::None, |o| o.render_mode()));
        }
        // Fog of war: the viewer's sight, eased from one state to the next (see `fog.rs`), uploaded
        // whenever it moves.
        let fog_viewer = self.game.viewer().filter(|_| self.game.sim.los_enabled);
        match fog_viewer {
            Some(player) => {
                let moved = self.game.fog.update(&self.game.sim, player, effect_dt);
                if moved || self.fog_key != Some(Some(player)) {
                    let (width, height, texels) = self.game.fog.texels();
                    renderer.set_fog_of_war(Some(ta_render::FogOfWar { width: width as u32, height: height as u32, cell: ta_sim::vision::VISION_CELL as f32, texels }));
                }
            }
            None if self.fog_key != Some(None) => renderer.set_fog_of_war(None),
            None => {}
        }
        self.fog_key = Some(fog_viewer);

        // `--demo spy`: its planes kept over the enemy's base, the camera on them, the memory logged.
        self.spy_frame();
        // Captures: `TA_FOLLOW=<unit name>` keeps the camera on the first unit of that kind; a
        // weapon's name (`NUCLEAR_MISSILE`, `CRBLMSSL`) on the first of its shots in the air.
        if let Some(name) = follow_unit() {
            let sim = &self.game.sim;
            let alpha = self.game.alpha();
            let shot = || {
                sim.projectiles.iter().find(|p| !p.dead && sim.weapon_types.get(p.weapon).name.eq_ignore_ascii_case(name)).map(|p| {
                    let (before, now) = (Vec3::from_array(p.prev_pos.to_f32()), Vec3::from_array(p.pos.to_f32()));
                    before.lerp(now, alpha)
                })
            };
            if let Some(p) = sim.units().find(|u| sim.types.get(u.type_id).name.eq_ignore_ascii_case(name)).and_then(|u| self.game.scene.unit_position(u.id, alpha)).or_else(shot) {
                self.camera.focus = p;
            }
        }
        // ... and `TA_CAMERA_PATH` moves it.
        if self.screenshot_path.is_some() {
            apply_camera_path(&mut self.camera, self.capture_clock as f32, &self.game.sim.world);
        }
        // Gallery tour: look at the next unit and photograph it a few frames later.
        if let Some(dir) = self.tour_dir.clone() && self.screenshot_path.is_none() && self.game.sim.tick >= self.tour_from {
            match self.tour.get(self.tour_step) {
                Some((id, name)) => {
                    // (`TA_GALLERY_SOLO=1`: that unit alone drawn, nothing of its neighbours in
                    // the way of a low camera.)
                    if std::env::var("TA_GALLERY_SOLO").is_ok_and(|v| v == "1") {
                        self.game.solo = Some(*id);
                    }
                    if let Some(u) = self.game.sim.unit(*id) {
                        let p = u.pos.to_f32();
                        let bounds = self.game.scene.model_for(u.type_id).and_then(|model| self.renderer.models.as_ref().map(|m| m.library.info(model)));
                        let (radius, height) = bounds.map_or((24.0, 32.0), |m| (m.radius.max(4.0), m.height.max(4.0)));
                        self.camera.focus = Vec3::new(p[0], p[1] + height * 0.5, p[2]);
                        // (`TA_GALLERY_FILL`: the share of the picture it fills, 0.55 by default.)
                        let fill = std::env::var("TA_GALLERY_FILL").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.55);
                        self.camera.zoom = std::env::var("TA_GALLERY_ZOOM").ok().and_then(|v| v.parse().ok())
                            .unwrap_or_else(|| self.camera.viewport.y.min(self.camera.viewport.x) * fill / (radius * 2.0).max(height));
                        // (`TA_GALLERY_YAW`, `TA_GALLERY_PITCH`: degrees round and above the level.)
                        let degrees = |k: &str, or: f32| std::env::var(k).ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(or);
                        self.camera.yaw = degrees("TA_GALLERY_YAW", 35.0).to_radians();
                        self.camera.pitch = degrees("TA_GALLERY_PITCH", 30.0).to_radians();
                    }
                    self.screenshot_path = Some(dir.join(format!("{}.png", name.to_ascii_lowercase())));
                    self.screenshot_frame = self.frames + 6;
                    self.tour_step += 1;
                }
                None => {
                    log::info!("gallery tour: {} pictures in {}", self.tour.len(), dir.display());
                    event_loop.exit();
                }
            }
        }
        if self.screenshot_path.is_some() && self.frames == self.screenshot_frame {
            self.renderer.request_screenshot();
        }
        // A photo: drawn on its own at its size, before the frame on screen (a capture's is
        // saved before it ends).
        if self.photo_take_frame == Some(self.frames) && let Some(photo) = self.photo.as_mut() {
            photo.take = true;
        }
        if let Some(photo) = self.photo.as_mut().filter(|p| p.take) {
            photo.take = false;
            let size = self.window.inner_size();
            let (width, height) = photo.size((size.width, size.height));
            let now = self.egui.ctx.input(|i| i.time);
            match self.renderer.render_image(&self.camera, width, height) {
                Some(shot) => {
                    let title = map_title(&self.game, &self.assets);
                    let caption = crate::photo::Caption { title: "TA Reborn".into(), line: format!("{title} · {}", crate::photo::today()) };
                    photo.save(shot, ta_config::pictures_dir(), title, Some(caption), now, self.exit_after_screenshot);
                }
                None => photo.failed(now),
            }
        }
        // A film's picture of this frame, and its sound.
        if self.film.as_ref().is_some_and(|f| f.rolling()) {
            self.film_picture();
        }
        // The EXPORT CLIP dialog's picture of a climax arrived at.
        self.clip_preview_pictures();
        self.take_unlock_picture();
        let t_render = Instant::now();
        let size = self.window.inner_size();
        // The intro room: this frame of it, with the movie's newest frame on its tube.
        let room_frame = match (self.screen, self.intro_room.as_mut()) {
            (Screen::Intro, Some(room)) if room.ready => {
                if let Some((w, h, rgba)) = self.movie.as_mut().and_then(|m| m.take_frame()) {
                    room.picture(&mut self.renderer, w, h, &rgba);
                }
                if let Some((elapsed, shown)) = self.room_tube.as_mut() {
                    *elapsed += dt;
                    let fps: f32 = std::env::var("TA_ROOM_TUBE_FPS").ok().and_then(|v| v.parse().ok()).unwrap_or(30.0);
                    let dir = std::path::PathBuf::from(std::env::var("TA_ROOM_TUBE").unwrap_or_default());
                    // (The last frame holds once the folder runs out.)
                    let mut k = (*elapsed * fps) as usize;
                    while k > 0 && !dir.join(format!("f_{k}.png")).exists() {
                        k -= 1;
                    }
                    if k != *shown
                        && let Ok(img) = image::open(dir.join(format!("f_{k}.png")))
                    {
                        let img = img.to_rgba8();
                        room.picture(&mut self.renderer, img.width(), img.height(), img.as_raw());
                        *shown = k;
                    }
                }
                // (A capture's view is the room's own, or `TA_ROOM_LOOK=x,y` pulls it.)
                let mouse = if self.exit_after_screenshot {
                    std::env::var("TA_ROOM_LOOK").ok().and_then(|v| v.split_once(',').and_then(|(x, y)| Some(Vec2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))))
                } else {
                    self.input.cursor_inside.then(|| Vec2::new(self.input.mouse.x / size.width.max(1) as f32 * 2.0 - 1.0, self.input.mouse.y / size.height.max(1) as f32 * 2.0 - 1.0))
                };
                let picture = self.movie.as_ref().is_some_and(|m| m.showing()) || self.room_tube.is_some();
                Some(room.frame(dt, mouse, picture))
            }
            _ => None,
        };
        let screenshot = self.renderer.screenshot_pending();
        let film_window = self.film.as_mut().map(|f| f.window_due() || screenshot);
        let egui_renderer = &mut self.egui.renderer;
        let ui_pass = |device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView| {
            let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [size.width, size.height], pixels_per_point: ppp };
            // (Not when the card refuses its staging memory, as egui-wgpu panics then: this frame
            // goes without the interface.)
            if ta_render::gpu::unless_short_of_memory(|| egui_renderer.update_buffers(device, queue, encoder, &prims, &sd)).is_none() {
                return;
            }
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            egui_renderer.render(&mut pass, &prims, &sd);
        };
        match room_frame {
            Some(frame) => self.renderer.render_room(&frame, ui_pass),
            // (`TA_RENDER_FROM=n`: a capture draws nothing before its frame n, the game playing on
            // as fast as it can: a staged battle's armies moving up before the shot.)
            None if self.exit_after_screenshot && self.frames < render_from() => self.renderer.skip_frame(),
            // (While a film is made: its page alone, some twenty times a second, so that waiting on
            // the screen does not hold the film back.)
            None if film_window == Some(true) => self.renderer.render_ui(ui_pass),
            None if film_window == Some(false) => self.renderer.skip_frame(),
            None => self.renderer.render(&self.camera, ui_pass),
        }
        let t_end = Instant::now();
        self.dev.times.push(dt * 1000.0, (t_prepare - t_update).as_secs_f32() * 1000.0, (t_render - t_prepare).as_secs_f32() * 1000.0, (t_end - t_render).as_secs_f32() * 1000.0);
        if self.dev.gpu_timing_on {
            self.dev.times.push_gpu(self.renderer.gpu_timings_frame(), self.renderer.gpu_timings());
        }
        if self.benchmark.is_some() {
            self.benchmark_frame(event_loop, (t_render - t_update).as_secs_f32() * 1000.0)?;
        }
        for id in textures_delta.free.drain() {
            self.egui.renderer.free_texture(&id);
        }
        self.companion_frame();

        // F12: the screen as it is, interface and all (in photo mode F12 takes a photo).
        if self.game.keys.pressed(&self.input, "screenshot") && self.photo.is_none() && self.screenshot_path.is_none() {
            self.renderer.request_screenshot();
            let stem = if self.screen == Screen::Playing { map_title(&self.game, &self.assets) } else { "TA Reborn".to_string() };
            self.screenshot_path = Some(ta_config::next_picture(&ta_config::pictures_dir(), &stem));
            self.screenshot_frame = self.frames + 1;
        }
        if let Some(shot) = self.renderer.take_screenshot() {
            if let Some(path) = self.screenshot_path.take() {
                self.save_screenshot(&path, &shot)?;
            }
            if let Some(next) = self.shots.as_mut().and_then(|s| s.next()) {
                self.screenshot_path = Some(next);
                self.screenshot_frame = self.frames + self.shots.as_ref().map_or(1, |s| s.step);
            } else if self.exit_after_screenshot {
                println!("{} units, {}", self.game.sim.unit_count(), self.dev.times.summary());
                self.archive_replay();
                event_loop.exit();
            }
        }
        self.frames += 1;
        self.input.end_frame();
        Ok(())
    }
}

/// Pause menu, the end-of-battle panel and the notices over a battle, in the middle of `room`,
/// the part of the screen the battle's interface leaves free (over it, the statistics covered
/// the deck's charts).
#[allow(clippy::too_many_arguments)]
/// Captures: `TA_BATTLE_RECORD=<xp>,<achievements>` shows the result's career record as if the
/// battle had earned that much XP (by a made-up mix of sources) and those achievements (`a+b`).
fn sample_battle_record(career: &ta_config::profiles::Career) -> Option<ta_ui::profiles::BattleRecord> {
    let value = std::env::var("TA_BATTLE_RECORD").ok()?;
    // (`cheats`: a battle with cheats in it, its record taken back.)
    if value.trim() == "cheats" {
        let now = career.total_xp();
        return Some(ta_ui::profiles::BattleRecord { xp_before: now, xp_now: now, voided: true, ..Default::default() });
    }
    let (xp, ids) = value.split_once(',').unwrap_or((&value, ""));
    let achievements: Vec<String> = ids.split('+').filter(|id| ta_config::achievements::find(id).is_some()).map(str::to_string).collect();
    let achievement_xp = achievements.iter().filter_map(|id| ta_config::achievements::find(id)).map(|a| a.tier.reward()).sum::<u64>();
    let battle_xp = xp.trim().parse::<u64>().unwrap_or(2000);
    let part = |share: u64| battle_xp * share / 100;
    let breakdown = ta_config::profiles::XpBreakdown { combat: part(40), construction: part(30), economy: part(8), time: part(7),
        completion: 100.min(battle_xp), victory: battle_xp.saturating_sub(part(85) + 100.min(battle_xp)), ..Default::default() };
    let xp_now = career.total_xp();
    Some(ta_ui::profiles::BattleRecord { xp_before: xp_now.saturating_sub(battle_xp + achievement_xp), xp_now, battle_xp,
        breakdown: Some(breakdown), achievements, achievement_xp, voided: false })
}

fn in_game_overlays(
    ctx: &egui::Context,
    game: &mut Game,
    assets: &Assets,
    in_game_menu: &mut bool,
    options_open: &mut bool,
    controls_open: &mut bool,
    open_photo: &mut bool,
    back_to: crate::menu::Page,
    room: egui::Rect,
) -> Option<Transition> {
    use crate::theme::*;
    use egui::RichText;
    use ta_ui::panel::{self, Exit, Panel};
    use ta_ui::widgets::{self as w, ActionButton};
    let mut out = None;
    // A dialog's width in the room (its frame takes 48 more, and it keeps 10 from the edges).
    let fit_room = |preferred: f32| preferred.min(room.width() - 68.0).max(320.0);
    let secs = game.sim.tick / 30;
    let clock = format!("{:02}:{:02}", secs / 60, secs % 60);
    if let Some(p) = game.catch_up_progress() {
        let saved = game.playback.as_ref().is_some_and(|p| p.resume);
        let page = Panel::new(if saved { "LOADING SAVED GAME" } else { "FAST-FORWARDING" }).subtitle("REPLAYING THE BATTLE SO FAR").width(440.0);
        panel::modal_within(ctx, "catch_up", page, false, room, |ui| {
            panel::progress_bar(ui, p, ui.available_width());
        });
        return None;
    }
    // Multiplayer, watching a battle that is over: the host has gone back to the lobby (a
    // player follows), or, for the host, the battle is won (everyone back).
    if let Some(net) = game.net.as_ref().filter(|n| !n.host_lost && game.outcome.is_none())
        && (net.rematch || (net.is_host() && crate::lobby::battle_over(game)))
    {
        let title = if net.is_host() {
            ta_ui::i18n::t("THE BATTLE IS OVER").to_string()
        } else {
            let host = net.host_player().map(|p| game.player_label(p)).unwrap_or_else(|| net.host_name());
            ta_ui::i18n::tf("{name} IS BACK IN THE LOBBY", &[("name", &host.to_uppercase())])
        };
        panel::strip(ctx, "back_to_lobby", egui::Align2::CENTER_TOP, egui::vec2(0.0, 110.0), |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).font(strong(15.0)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
                ui.add_space(12.0);
                if ui.add(ActionButton::primary("BACK TO THE LOBBY")).clicked() {
                    out = Some(Transition::ToLobby);
                }
                if ui.add(ActionButton::secondary("MAIN MENU")).clicked() {
                    out = Some(Transition::ToMenu);
                }
            });
        });
    }
    // Multiplayer: without the host no more ticks come; say so and offer the way out.
    if let Some(net) = game.net.as_ref().filter(|n| n.host_lost && game.outcome.is_none()) {
        // The host left; or it went on without us, or our connection failed: then we may come
        // back (the address and credential kept for it).
        let rejoin = net.rejoin_address.clone().filter(|_| net.dropped || net.link_failed);
        let subtitle = if net.dropped {
            ta_ui::i18n::n("THE HOST WENT ON WITHOUT YOU")
        } else if net.link_failed {
            ta_ui::i18n::n("YOUR CONNECTION TO THE HOST FAILED")
        } else {
            ta_ui::i18n::n("THE HOST HAS LEFT THE GAME")
        };
        let page = Panel::new("CONNECTION LOST").subtitle(subtitle).width(460.0).accent(RED);
        panel::modal_within(ctx, "host_lost", page, false, room, |ui| {
            // (A failed link may be the host's: it may have stopped too.)
            let message = if net.dropped {
                ta_ui::i18n::t("The battle goes on without you. Come back within a minute and your army is still yours.")
            } else if net.link_failed {
                ta_ui::i18n::t("If the battle goes on without you, come back within a minute and your army is still yours.")
            } else if game.record.available() {
                ta_ui::i18n::t("The game cannot go on without the host. The battle so far is kept as a replay.")
            } else {
                ta_ui::i18n::t("The game cannot go on without the host. Recording stopped at its memory limit, so no replay was saved.")
            };
            ui.add(egui::Label::new(RichText::new(message).color(TEXT)).wrap());
            ui.add_space(16.0);
            if let Some(address) = &rejoin
                && ui.add(ActionButton::primary("REJOIN").size(ui.available_width(), 40.0)).clicked()
            {
                out = Some(Transition::Rejoin(address.clone()));
            }
            let home = if rejoin.is_some() { ActionButton::secondary("RETURN TO MAIN MENU") } else { ActionButton::primary("RETURN TO MAIN MENU") };
            if ui.add(home.size(ui.available_width(), 40.0)).clicked() {
                out = Some(Transition::ToMenu);
            }
        });
    }
    if game.replay_finished() {
        panel::strip(ctx, "replay_end", egui::Align2::CENTER_TOP, egui::vec2(0.0, 60.0), |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(ta_ui::i18n::t("END OF REPLAY")).font(display(24.0)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.5)));
                ui.add_space(18.0);
                if game.tally.has_moments() && ui.add_enabled(game.highlights_available(), ActionButton::primary("WATCH HIGHLIGHTS")).on_disabled_hover_text(ta_ui::i18n::t(crate::replay::RECORDING_STOPPED)).clicked() {
                    out = Some(Transition::Highlights);
                }
                if ui.add(ActionButton::secondary("RETURN TO MAIN MENU")).clicked() {
                    out = Some(Transition::ToMenu);
                }
            });
        });
    }
    // (The battle's result is a screen of its own: `result.rs`.)
    if *in_game_menu && game.stats_open {
        let subtitle = ta_ui::i18n::tf("BATTLE TIME {clock}", &[("clock", &clock)]);
        let page = Panel::new("BATTLE STATISTICS").subtitle(&subtitle).width(fit_room(940.0)).exit(Exit::Close);
        let response = panel::modal_within(ctx, "stats", page, false, room, |ui| {
            let mut series = game.stats_series;
            crate::stats::panel(ui, game, &mut series);
            game.stats_series = series;
        });
        if response.exit {
            game.stats_open = false;
        }
    } else if *in_game_menu {
        let subtitle = format!("{} · {clock}", map_title(game, assets).to_uppercase());
        let briefing = game.mission.is_some() || game.scenario.is_some();
        // Its buttons in two columns, as wide as their longest label (translated) needs.
        let companion = if game.companion_open { "CLOSE THE COMPANION SCREEN" } else { "COMPANION SCREEN" };
        let resign = if game.resign_armed { ta_ui::i18n::n("REALLY RESIGN?") } else { ta_ui::i18n::n("RESIGN") };
        // (The way back names where it goes: the page the battle came from.)
        let leave = if game.from_editor { ta_ui::i18n::n("BACK TO THE EDITOR") } else { back_label(back_to) };
        let labels = ["SAVE GAME", "STATISTICS", "BATTLE EVENT HISTORY", "PHOTO MODE", companion, "OPTIONS", "CONTROLS", "CHEATS", resign, "RESTART", leave, "QUIT TO DESKTOP"];
        let column = labels.iter().map(|label| button_width(ctx, label)).fold(196.0, f32::max).ceil();
        let buttons = column * 2.0 + 10.0;
        // (A multiplayer battle goes on while the menu is open.)
        let (title, subtitle) = if game.net.is_some() && !game.paused {
            (ta_ui::i18n::n("BATTLE MENU"), format!("{} · {}", ta_ui::i18n::t("THE BATTLE GOES ON"), subtitle))
        } else {
            (ta_ui::i18n::n("PAUSED"), subtitle)
        };
        let page = Panel::new(title).subtitle(&subtitle).width(if briefing { 460.0 } else { 268.0 } + 22.0 + buttons).exit(Exit::Close);
        let response = panel::modal_within(ctx, "pause", page, false, room, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 22.0;
                // The battle so far, at a glance (a mission's or a scenario's briefing in its
                // place).
                if !briefing {
                    ui.vertical(|ui| {
                        ui.set_width(268.0);
                        battle_card(ui, game);
                    });
                }
                if let Some(m) = &game.mission {
                    // Mission objectives and briefing, as a reminder.
                    ui.vertical(|ui| {
                        ui.set_width(460.0);
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if !m.name.is_empty() {
                            w::fitted_title(ui, &crate::campaign::strip_number(&m.name), 24.0, TEXT_BRIGHT);
                            ui.add_space(6.0);
                        }
                        panel::section(ui, "OBJECTIVES");
                        crate::campaign::objectives(ui, &m.victory, &m.defeat, assets);
                        if !game.briefing.is_empty() {
                            ui.add_space(10.0);
                            let narration = format!("camps/briefs/{}.wav", m.narration.to_ascii_lowercase());
                            ui.horizontal(|ui| {
                                ui.label(w::caption("BRIEFING"));
                                // The narrator again.
                                if !m.narration.is_empty() && assets.vfs.exists(&narration) && ui.add(ActionButton::secondary("HEAR IT AGAIN")).clicked() {
                                    game.music_requests.push(crate::chat::MusicRequest::Narration(narration.clone()));
                                }
                            });
                            let area = egui::ScrollArea::vertical().max_height(ta_ui::scroll::whole_lines(240.0, crate::campaign::BRIEF_LINE, 6.0));
                            ta_ui::scroll::well(ui, area, |ui| {
                                egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    crate::campaign::briefing_text(ui, &game.briefing);
                                });
                            });
                        }
                    });
                }
                if let Some(run) = &game.scenario {
                    // The scenario's briefing, as a reminder (its goals are on screen).
                    ui.vertical(|ui| {
                        ui.set_width(460.0);
                        ui.spacing_mut().item_spacing.x = 8.0;
                        w::fitted_title(ui, ta_ui::i18n::t(&run.title), 24.0, TEXT_BRIGHT);
                        ui.add_space(6.0);
                        panel::section(ui, "BRIEFING");
                        ta_ui::scroll::well(ui, egui::ScrollArea::vertical().max_height(300.0), |ui| {
                            egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.add(egui::Label::new(RichText::new(ta_ui::i18n::t(run.briefing.trim())).color(TEXT)).wrap());
                            });
                        });
                    });
                }
                ui.vertical(|ui| {
                    ui.set_width(buttons);
                    ui.spacing_mut().item_spacing.y = 5.0;
                    if ui.add(ActionButton::primary(if game.net.is_some() && !game.paused { "BACK TO THE BATTLE" } else { "RESUME" }).size(buttons, 42.0)).clicked() {
                        *in_game_menu = false;
                    }
                    if !game.record.available() {
                        ui.label(RichText::new(ta_ui::i18n::t(crate::replay::RECORDING_STOPPED)).color(AMBER));
                    }
                    ui.add_space(4.0);
                    let b = |ui: &mut egui::Ui, button: ActionButton| ui.add(button.size(column, 34.0)).clicked();
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        // The battle: keeping it, its figures, its story, its pictures.
                        ui.vertical(|ui| {
                            ui.set_width(column);
                            ui.spacing_mut().item_spacing.y = 5.0;
                            ui.label(w::caption("THE BATTLE"));
                            let can_save = game.net.is_none() && game.playback.is_none() && !game.spectator && !game.tutorial;
                            if can_save && ui.add_enabled(game.record.available(), ActionButton::secondary("SAVE GAME").size(column, 34.0)).on_disabled_hover_text(ta_ui::i18n::t(crate::replay::RECORDING_STOPPED)).clicked() {
                                save_battle(game, assets);
                            }
                            // (When it was last saved, in the battle's time: the menu stays open.)
                            if can_save && let Some(at) = game.saved_at {
                                let secs = at / 30;
                                ui.label(RichText::new(ta_ui::i18n::tf("Saved at {time}", &[("time", &format!("{:02}:{:02}", secs / 60, secs % 60))])).font(body(12.0)).color(STEEL));
                            }
                            if b(ui, ActionButton::secondary("STATISTICS")) {
                                game.stats_open = !game.stats_open;
                            }
                            if ui.add(ActionButton::secondary("BATTLE EVENT HISTORY").size(column, 34.0))
                                .on_hover_text(ta_ui::i18n::tf("What happened in this battle, and where ({key})", &[("key", &game.keys.text("event_history"))])).clicked() {
                                game.history_open = true;
                            }
                            if b(ui, ActionButton::secondary("PHOTO MODE")) {
                                *open_photo = true;
                                *in_game_menu = false;
                            }
                            if ui.add(ActionButton::secondary(if game.companion_open { "CLOSE THE COMPANION SCREEN" } else { "COMPANION SCREEN" }).size(column, 34.0))
                                .on_hover_text(ta_ui::i18n::tf("The map and intel in a window of their own, for a second display ({key})", &[("key", &game.keys.text("companion"))])).clicked() {
                                game.companion_toggle = true;
                            }
                        });
                        // The settings, and the ways out.
                        ui.vertical(|ui| {
                            ui.set_width(column);
                            ui.spacing_mut().item_spacing.y = 5.0;
                            ui.label(w::caption("SETTINGS"));
                            if b(ui, ActionButton::secondary("OPTIONS")) {
                                *options_open = true;
                            }
                            if b(ui, ActionButton::secondary("CONTROLS")) {
                                *controls_open = !*controls_open;
                            }
                            if game.cheats_available() && b(ui, ActionButton::secondary("CHEATS")) {
                                game.cheat_panel = true;
                                *in_game_menu = false;
                            }
                            ui.add_space(4.0);
                            ui.label(w::caption("LEAVE"));
                            // Resign (skirmish and multiplayer): a second click confirms.
                            // (In a team game the army goes to the allies: said on hover.)
                            let to_allies = !game.allies().is_empty();
                            if game.mission.is_none() && !game.tutorial && !game.spectator && game.playback.is_none() && {
                                let r = ui.add(ActionButton::danger(resign).size(column, 34.0));
                                (if to_allies { r.on_hover_text(ta_ui::i18n::t("Your army goes to your allies; your Commander blows up")) } else { r }).clicked()
                            } {
                                if game.resign_armed {
                                    game.issue(ta_sim::Command::Resign { player: game.local_player });
                                    *in_game_menu = false;
                                }
                                game.resign_armed = !game.resign_armed;
                            }
                            // Starting over (single player): asked first.
                            if game.can_save() && !game.from_editor && game.outcome.is_none()
                                && ui.add(ActionButton::secondary("RESTART").size(column, 34.0)).on_hover_text(ta_ui::i18n::t("The same battle again, from its start")).clicked()
                            {
                                game.leave_asked = Some(crate::game::Leave::Restart);
                            }
                            // The ways out: asked first while there is something to lose.
                            if b(ui, ActionButton::secondary(leave)) {
                                if game.leaving_asks() { game.leave_asked = Some(crate::game::Leave::Menu); } else { out = Some(Transition::ToMenu); }
                            }
                            if b(ui, ActionButton::secondary("QUIT TO DESKTOP")) {
                                if game.leaving_asks() { game.leave_asked = Some(crate::game::Leave::Quit); } else { out = Some(Transition::Quit); }
                            }
                        });
                    });
                });
            });
            ui.add_space(10.0);
            panel::rule(ui);
            ui.add_space(2.0);
            let back = if game.net.is_some() && !game.paused { ta_ui::i18n::n("BACK TO THE BATTLE") } else { ta_ui::i18n::n("RESUME") };
            panel::key_hints(ui, &[("Esc", back), (game.keys.text("photo_mode"), ta_ui::i18n::n("PHOTO MODE")), (game.keys.text("screenshot"), ta_ui::i18n::n("SCREENSHOT"))]);
        });
        if response.exit {
            *in_game_menu = false;
            game.leave_asked = None;
        }
        // Leaving (or starting over) asked: the question over the menu.
        if let Some(leave) = game.leave_asked
            && let Some(answer) = leave_question(ctx, game, assets, leave)
        {
            game.leave_asked = None;
            if let Some(transition) = answer {
                out = Some(transition);
            }
        }
    }
    out
}

/// Whether the commander's identity card shows, at the top right over the menu: on the title
/// screen alone, the film not being watched nor the options open. Every other page has its way
/// back there (the campaign's, the briefing's, the skirmish setup's BACK; Conquest's MAIN MENU),
/// where the card lay over it and took its clicks in windows up to about 2,000 pixels wide
/// (1920 × 1080 among them). (Not over a lobby either: the profile's pages cover the menu, whose
/// lobby must keep up with its network.)
fn shows_identity_card(page: crate::menu::Page, options_open: bool, film_watch: f32) -> bool {
    page == crate::menu::Page::Title && !options_open && film_watch < 0.98
}

/// Where the campaign page opens after a mission (CONTINUE CAMPAIGN, BACK TO BRIEFING): at that
/// mission, but no further than the campaign has got (a victory that recorded nothing, one with
/// cheats, leaves the next mission locked), and on the difficulty the mission was played at (the
/// page may not have been open before: a mission loaded from a save, or started from the command
/// line).
fn campaign_return(progress: &ta_config::CampaignProgress, played: Option<&crate::setup::MissionStart>, campaign: &str, mission: usize) -> (usize, String) {
    let difficulty = played.map_or_else(|| progress.difficulty.clone(), |m| m.difficulty.clone());
    (mission.min(progress.unlocked(campaign)), difficulty)
}

/// The way back from a battle, named after where it goes (the page it came from).
fn back_label(page: crate::menu::Page) -> &'static str {
    use crate::menu::Page;
    use ta_ui::i18n::n;
    match page {
        Page::Skirmish => n("BACK TO THE SKIRMISH SETUP"),
        Page::Campaign => n("BACK TO THE CAMPAIGN"),
        Page::Scenarios => n("BACK TO THE SCENARIOS"),
        Page::Records => n("BACK TO SAVED GAMES"),
        Page::Tutorial => n("BACK TO THE LESSONS"),
        Page::Conquest => n("BACK TO THE CONQUEST"),
        _ => n("MAIN MENU"),
    }
}

/// Saves the battle (SAVE GAME, SAVE AND LEAVE): said in a message; true when saved.
fn save_battle(game: &mut Game, assets: &Assets) -> bool {
    let Some(ticks) = game.record.snapshot() else {
        game.message(ta_ui::i18n::t(crate::replay::RECORDING_STOPPED));
        return false;
    };
    let title = record_title(game, assets);
    let record = crate::replay::Record::new(crate::replay::RecordKind::Save, assets.data_hash, assets.mods.record(), game.setup.clone(), ticks, game.sim.tick, title).with_profile(game).with_duties(game);
    match record.write() {
        Ok(_) => {
            game.saved_at = Some(game.sim.tick);
            // (Orders given while paused go in with the next tick: not in this save.)
            if game.commands.is_empty() {
                game.done(ta_ui::i18n::t("Game saved"));
            } else {
                game.done(ta_ui::i18n::t("Game saved: the orders given while paused are not in it"));
            }
            true
        }
        Err(e) => {
            game.trouble(ta_ui::i18n::tf("Could not save: {error}", &[("error", &e)]));
            false
        }
    }
}

/// The question before leaving a battle (or starting it over): what would be lost, and
/// SAVE AND LEAVE where it can be saved. `Some(answer)` once answered: the transition, or
/// none (STAY).
fn leave_question(ctx: &egui::Context, game: &mut Game, assets: &Assets, leave: crate::game::Leave) -> Option<Option<Transition>> {
    use crate::game::Leave;
    use crate::theme::*;
    use egui::RichText;
    use ta_ui::panel::{self, Exit, Panel};
    use ta_ui::widgets::ActionButton;
    use ta_ui::i18n::{n, t, tf};
    let (title, go) = match leave {
        Leave::Menu => (n("LEAVE THE BATTLE?"), n("LEAVE")),
        Leave::Quit => (n("QUIT THE GAME?"), n("QUIT")),
        Leave::Restart => (n("START THE BATTLE AGAIN?"), n("RESTART")),
    };
    let restart = game.setup.clone();
    let transition = move || match leave {
        Leave::Menu => Transition::ToMenu,
        Leave::Quit => Transition::Quit,
        Leave::Restart => Transition::StartGame(restart.clone()),
    };
    let lost = if game.hosting_others() {
        // (Every machine takes its ticks from the host's: without it nobody plays on.)
        t("You are the host: if you leave, the battle ends for everyone.").to_string()
    } else if game.net.is_some() {
        t("The battle goes on without you: the others see you leave.").to_string()
    } else {
        match game.saved_at {
            Some(at) => {
                let secs = at / 30;
                tf("Last saved at {time} of the battle: what happened since is lost.", &[("time", &format!("{:02}:{:02}", secs / 60, secs % 60))])
            }
            None => t("This battle has not been saved: it is lost.").to_string(),
        }
    };
    let saves = game.can_save() && leave != Leave::Restart && game.record.available();
    let mut answer = None;
    let page = Panel::new(title).width(460.0).exit(Exit::Close).accent(RED);
    let response = panel::modal(ctx, "leave_battle", page, true, |ui| {
        ui.add(egui::Label::new(RichText::new(&lost).color(TEXT)).wrap());
        if leave == Leave::Restart {
            ui.add(egui::Label::new(RichText::new(t("Its replay so far is kept.")).color(STEEL)).wrap());
        }
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if saves && ui.add(ActionButton::primary(if leave == Leave::Quit { "SAVE AND QUIT" } else { "SAVE AND LEAVE" })).clicked() && save_battle(game, assets) {
                answer = Some(Some(transition()));
            }
            if ui.add(ActionButton::danger(go)).clicked() {
                answer = Some(Some(transition()));
            }
            if ui.add(ActionButton::secondary("STAY")).clicked() {
                answer = Some(None);
            }
        });
    });
    if response.exit && answer.is_none() {
        answer = Some(None);
    }
    answer
}

/// How wide a pause menu's button must be for its label (`label`, translated): as an
/// `ActionButton` of its height sizes itself.
fn button_width(ctx: &egui::Context, label: &str) -> f32 {
    let text = ta_ui::i18n::t(label);
    let width = ctx.fonts_mut(|f| f.layout_no_wrap(text.to_string(), crate::theme::strong(13.5), egui::Color32::WHITE).size().x);
    width + ta_ui::i18n::tracking(0.8) * text.chars().count().saturating_sub(1) as f32 + 30.0
}

/// The pause menu's card of the battle so far, for the player: the army and the economy now,
/// what has been built, lost and destroyed, and the Commander's health.
fn battle_card(ui: &mut egui::Ui, game: &Game) {
    use crate::theme::*;
    use egui::RichText;
    use ta_ui::i18n::n;
    use ta_ui::widgets as w;
    let me = game.local_player as usize;
    let Some(player) = game.sim.players.get(me) else { return };
    let now = crate::stats::finals(game).get(me).copied().flatten();
    w::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 4.0;
        w::card_title(ui, "YOUR FORCES");
        // A figure, large, over what it is.
        let figure = |ui: &mut egui::Ui, value: String, what: &str, colour: egui::Color32| {
            ui.vertical(|ui| {
                ui.label(RichText::new(value).font(display(26.0)).color(colour));
                ui.label(w::caption(what));
            });
        };
        ui.columns(2, |cols| {
            figure(&mut cols[0], now.map_or(player.unit_count, |s| s.units).to_string(), n("UNITS"), TEXT_BRIGHT);
            figure(&mut cols[1], now.map_or_else(|| "0".to_string(), |s| crate::stats::big(s.army_value)), n("ARMY VALUE"), TEXT_BRIGHT);
        });
        ui.add_space(4.0);
        ta_ui::panel::rule(ui);
        // A caption with its figure at the right.
        let row = |ui: &mut egui::Ui, what: &str, value: String, colour: egui::Color32| {
            ui.horizontal(|ui| {
                ui.label(w::caption(what));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(value).font(strong(15.0)).color(colour));
                });
            });
        };
        // The economy: what each brings in a second.
        row(ui, n("METAL"), format!("+{:.1}", now.map_or(0.0, |s| s.metal_income.to_f32())), STEEL);
        row(ui, n("ENERGY"), format!("+{:.1}", now.map_or(0.0, |s| s.energy_income.to_f32())), AMBER);
        ui.add_space(4.0);
        ta_ui::panel::rule(ui);
        let s = &player.stats;
        row(ui, n("BUILT"), s.built.to_string(), TEXT);
        row(ui, n("LOST"), s.lost.to_string(), if s.lost > 0 { RED } else { TEXT });
        row(ui, n("KILLS"), s.kills.to_string(), if s.kills > 0 { GREEN } else { TEXT });
        // The Commander, while there is one.
        if let Some(unit) = game.local_commander().and_then(|id| game.sim.unit(id)).filter(|_| !game.spectator) {
            ui.add_space(4.0);
            ta_ui::panel::rule(ui);
            let health = unit.health_percent(game.sim.unit_type(unit));
            ui.horizontal(|ui| {
                ui.label(w::caption("COMMANDER"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let colour = if health > 60 { GREEN } else if health > 30 { AMBER } else { RED };
                    ui.label(RichText::new(format!("{health}%")).font(strong(15.0)).color(colour));
                });
            });
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), egui::Sense::hover());
            ui.painter().add(ta_ui::metal::well(rect, INSET));
            let fill = egui::Rect::from_min_max(rect.min + egui::vec2(1.0, 1.0), egui::pos2(rect.min.x + 1.0 + (rect.width() - 2.0) * health as f32 / 100.0, rect.max.y - 1.0));
            if fill.width() > 0.5 {
                let (top, bottom) = if health > 60 { (GREEN, GREEN.gamma_multiply(0.45)) } else if health > 30 { (AMBER_BRIGHT, AMBER_DIM) } else { (RED, RED.gamma_multiply(0.45)) };
                ui.painter().add(ta_ui::metal::vgrad(fill, top, bottom));
            }
        }
        // The skirmish's mutators, and the ceasefire's time left.
        let tags = crate::menu::mutator_tags(&game.sim.mutators);
        if !tags.is_empty() {
            ui.add_space(4.0);
            ta_ui::panel::rule(ui);
            ui.horizontal(|ui| {
                ui.label(w::caption("MUTATORS"));
                if let Some(left) = game.sim.ceasefire_left() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let s = left / 30;
                        let clock = format!("{:02}:{:02}", s / 60, s % 60);
                        ui.label(RichText::new(ta_ui::i18n::tf("ceasefire {clock}", &[("clock", &clock)])).font(strong(13.0)).color(AMBER));
                    });
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                for tag in &tags {
                    w::tag(ui, tag, AMBER);
                }
            });
        }
    });
}

/// A new battle's single-player touches: our name on the chat line, and the enemy's computer
/// players, who talk (not in campaign missions, which tell their own story), remembering the
/// player from battles before where this run keeps records (`remember`).
fn meet_the_enemy(game: &mut Game, settings: &ta_config::Config, remember: bool) {
    game.player_name = settings.gameplay.player_name.clone();
    let setup = &game.setup;
    let computers: Vec<u8> = setup.players.iter().enumerate().filter(|(_, s)| matches!(s.controller, crate::setup::Controller::Ai(_))).map(|(i, _)| i as u8).collect();
    game.chatter = match setup.local_player {
        Some(me) if settings.gameplay.ai_chatter && setup.mission.is_none() => {
            // (Captures: `TA_RIVALS=won|lost` makes every persona a rival the player beat last
            // time, or lost to; such a record is never kept.)
            let rivals = match std::env::var("TA_RIVALS").ok().as_deref() {
                Some(r @ ("won" | "lost")) => crate::chatter::every_rival(r == "won"),
                _ if remember => crate::rivals::Rivals::load(),
                _ => crate::rivals::Rivals::default(),
            };
            crate::chatter::Chatter::new(&game.sim, me, &computers, setup.seed, rivals, remember && std::env::var("TA_RIVALS").is_err())
        }
        _ => None,
    };
}

/// The battle's map as the menus name it (photos and screenshots are named after it); a
/// campaign mission's name.
fn map_title(game: &Game, assets: &Assets) -> String {
    if let Some(m) = game.mission.as_ref().filter(|m| !m.name.is_empty()) {
        return crate::campaign::strip_number(&m.name).to_string();
    }
    assets.maps.iter().find(|m| m.name.eq_ignore_ascii_case(&game.setup.map)).map_or_else(|| game.setup.map.clone(), |m| m.title.clone())
}

/// The player's frame rate counter: frames a second and a frame's milliseconds, small, at the bottom
/// right of the battlefield the interface leaves free (`battle_view`, window pixels; without one, the
/// window's corner), so that it covers none of the interface.
fn frame_rate_counter(ctx: &egui::Context, meter: &FrameMeter, battle_view: Option<(Vec2, Vec2)>, ppp: f32) {
    use crate::theme::{STEEL, mono};
    let Some((fps, ms)) = meter.shown else { return };
    let ppp = ppp.max(0.1);
    let corner = battle_view.map_or(ctx.content_rect().right_bottom(), |(_, max)| egui::pos2(max.x / ppp, max.y / ppp));
    egui::Area::new(egui::Id::new("frame_rate_counter"))
        .order(egui::Order::Foreground)
        .interactable(false)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(corner - egui::vec2(8.0, 8.0))
        .show(ctx, |ui| {
            egui::Frame::new().fill(egui::Color32::from_black_alpha(150)).corner_radius(3).inner_margin(egui::Margin::symmetric(6, 2)).show(ui, |ui| {
                ui.label(egui::RichText::new(format!("{fps:.0} FPS · {ms:.1} ms")).font(mono(12.0)).color(STEEL));
            });
        });
}

fn draw_dev_panel(
    ctx: &egui::Context,
    o: &mut DevPanel,
    lighting: &mut ta_render::Lighting,
    post: &mut ta_render::post::PostSettings,
    camera: &Camera,
    game: &Game,
) {
    egui::Window::new("Developer").default_pos([260.0, 60.0]).default_width(300.0).show(ctx, |ui| {
        ui.label(format!("{:.0} fps · sim tick {:.2} ms · {} units", o.fps, game.last_tick_ms, game.sim.unit_count()));
        ui.label(o.times.summary());
        let p = &o.particles;
        ui.label(format!("GPU particles {} / {} (+{} last frame)", p.live, p.capacity, p.emitted));
        ui.checkbox(&mut o.gpu_timing, "GPU time per pass");
        if o.gpu_timing_on && !o.gpu_times.is_empty() {
            let line: Vec<String> = o.gpu_times.iter().map(|(pass, ms)| format!("{pass} {ms:.2}")).collect();
            ui.label(format!("GPU ms: {}", line.join(" · ")));
        }
        ui.label(format!(
            "camera {:?} focus ({:.0}, {:.0}, {:.0}) zoom {:.2} yaw {:.0}° pitch {:.0}°",
            camera.mode,
            camera.focus.x,
            camera.focus.y,
            camera.focus.z,
            camera.zoom,
            camera.yaw.to_degrees(),
            camera.pitch.to_degrees()
        ));
        if let Some(p) = game.cursor_ground {
            ui.label(format!("cursor ({:.0}, {:.0}, {:.0})", p.x, p.y, p.z));
        }
        ui.collapsing("Lighting", |ui| {
            let mut changed = ui.add(egui::Slider::new(&mut o.sun_azimuth, -180.0..=180.0).text("sun azimuth")).changed();
            changed |= ui.add(egui::Slider::new(&mut o.sun_elevation, 5.0..=90.0).text("sun elevation")).changed();
            if changed {
                let (az, el) = (o.sun_azimuth.to_radians(), o.sun_elevation.to_radians());
                lighting.sun_dir = Vec3::new(az.cos() * el.cos(), el.sin(), az.sin() * el.cos());
            }
            ui.add(egui::Slider::new(&mut lighting.sun_intensity, 0.2..=2.0).text("sun intensity"));
            ui.add(egui::Slider::new(&mut lighting.terrain_relight, 0.0..=1.5).text("terrain relight"));
            ui.add(egui::Slider::new(&mut lighting.shadow_strength, 0.0..=1.0).text("shadows"));
            ui.add(egui::Slider::new(&mut lighting.fog_density, 0.0..=0.001).text("fog"));
        });
        ui.collapsing("Post", |ui| {
            ui.add(egui::Slider::new(&mut post.exposure, 0.3..=2.5).text("exposure"));
            ui.add(egui::Slider::new(&mut post.bloom, 0.0..=1.5).text("bloom"));
            ui.add(egui::Slider::new(&mut post.bloom_threshold, 0.2..=3.0).text("bloom threshold"));
            ui.add(egui::Slider::new(&mut post.vignette, 0.0..=0.8).text("vignette"));
            ui.add(egui::Slider::new(&mut post.saturation, 0.0..=1.6).text("saturation"));
        });
        ui.separator();
        let help = game.keys.text("help");
        ui.small(format!("{help}: every control and its key · Ctrl+{help}: this panel"));
    });
}


/// A short description of a game for save and replay lists.
fn record_title(game: &Game, assets: &Assets) -> String {
    if let Some(m) = &game.mission {
        return if m.name.is_empty() { game.setup.map.clone() } else { m.name.clone() };
    }
    let map = assets.maps.iter().find(|m| m.name.eq_ignore_ascii_case(&game.setup.map)).map_or(game.setup.map.clone(), |m| m.title.clone());
    ta_ui::i18n::tf("{map} — {n} players", &[("map", &map), ("n", &game.setup.players.len())])
}

impl State {
    /// Reads the gamepads into the input: the cursor (the system's follows where it can),
    /// clicks unless over the interface, keys, actions and bumpers.
    /// Saves a screenshot: a scripted capture's failure ends the run, a player's is reported.
    fn save_screenshot(&mut self, path: &std::path::Path, shot: &ta_render::Screenshot) -> Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).ok();
        }
        match image::save_buffer(path, &shot.rgba, shot.width, shot.height, image::ColorType::Rgba8) {
            Ok(()) => {
                log::info!("saved screenshot {}", path.display());
                if !self.exit_after_screenshot && self.tour_dir.is_none() {
                    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    self.game.message(ta_ui::i18n::tf("Screenshot saved: {name}", &[("name", &name)]));
                }
                Ok(())
            }
            Err(e) if self.exit_after_screenshot || self.tour_dir.is_some() => Err(e.into()),
            Err(e) => {
                log::error!("could not save {}: {e}", path.display());
                self.game.trouble(ta_ui::i18n::tf("Could not save the screenshot: {error}", &[("error", &e)]));
                Ok(())
            }
        }
    }

    fn poll_gamepad(&mut self) {
        if self.benchmark.is_some() {
            self.pad = Default::default();
            return;
        }
        let dt = self.last_frame.elapsed().as_secs_f32().clamp(0.001, 0.1);
        let size = self.window.inner_size();
        let window = Vec2::new(size.width.max(1) as f32, size.height.max(1) as f32);
        // Moving the mouse hands the cursor back to it.
        if self.input.mouse_delta.length_squared() > 4.0 && self.pad.cursor.is_none() {
            self.gamepad.active = false;
        }
        self.pad = self.gamepad.update(dt, self.input.mouse, window, self.settings.gameplay.gamepad_deadzone, self.settings.gameplay.gamepad_sensitivity, self.window.has_focus());
        if !self.demo_pad.is_empty() {
            self.pad.nav.append(&mut self.demo_pad);
            self.demo_pad_used = true;
        }
        self.pad_on_interface();
        let available = self.keyboard_pointer_available();
        if available && self.game.keys.pressed(&self.input, "keyboard_pointer") {
            self.settings.gameplay.keyboard_pointer = !self.settings.gameplay.keyboard_pointer;
            self.game.cancel_pointer_gestures();
            self.game.message(if self.settings.gameplay.keyboard_pointer {
                ta_ui::i18n::t("Keyboard pointer on: arrows move, Enter selects, End gives orders, Tab selects units")
            } else { ta_ui::i18n::t("Keyboard pointer off") });
            self.save_settings();
        }
        let active = available && self.settings.gameplay.keyboard_pointer;
        if active { self.keyboard_pointer.traverse(&mut self.game, &self.input); }
        if !active && self.keyboard_pointer.has_pending_click() { self.game.cancel_pointer_gestures(); }
        let keyboard = self.keyboard_pointer.update(&mut self.input, dt, window, active, self.settings.gameplay.order_button);
        if active { self.input.claim_keys(crate::keyboard_pointer::RESERVED); }
        if keyboard.cursor.is_some() { self.pad.cursor = keyboard.cursor; }
        if keyboard.left.is_some() { self.pad.left = keyboard.left; }
        if keyboard.right.is_some() { self.pad.right = keyboard.right; }
        self.pad.zoom += keyboard.zoom;
        let over_ui = self.egui.ctx.is_pointer_over_egui() || self.egui.ctx.egui_wants_pointer_input();
        self.input.apply_pad(&self.pad, over_ui);
        if let Some(c) = self.pad.cursor {
            let _ = self.window.set_cursor_position(winit::dpi::PhysicalPosition::new(c.x as f64, c.y as f64));
        }
    }

    fn feed_pad_to_egui(&mut self, raw: &mut egui::RawInput) {
        let m = self.input.modifiers;
        let modifiers = egui::Modifiers { alt: m.alt_key(), ctrl: m.control_key(), shift: m.shift_key(), mac_cmd: false, command: m.control_key() };
        raw.events.extend(crate::gamepad::Gamepad::egui_events(&self.pad, self.input.mouse, self.egui.ctx.pixels_per_point(), modifiers));
        // The d-pad's moves as keys, and the control they reach scrolled into view.
        let focused = self.egui.ctx.memory(|m| m.focused());
        let typing = self.egui.ctx.text_edit_focused();
        let (events, waiting) = self.pad_follow.events(&self.pad.nav, focused, typing);
        raw.events.extend(events);
        if waiting {
            self.egui.ctx.request_repaint();
        }
    }

    /// On the menus and over a battle's dialogs the pad works the interface as keys do
    /// (`ta_ui::pad::nav_events`): its d-pad no control group there (they did nothing but that,
    /// and nothing went back but the pointer taken to BACK), nor its B a right-click; its A, once
    /// the d-pad has moved the focus, no click wherever the pointer happens to be. Elsewhere its
    /// moves are dropped. (The menu shows how, while a pad is in use.)
    fn pad_on_interface(&mut self) {
        use crate::gamepad::Nav;
        self.menu.pad = self.gamepad.active || self.demo_pad_used;
        // A unit's radial menu has the d-pad while it is up (and the pointer's clicks, which are
        // A and B).
        if self.screen == Screen::Playing && self.game.radial.is_some() {
            let nav = std::mem::take(&mut self.pad.nav);
            self.game.radial_nav(&nav);
            self.pad.keys.retain(|k| !matches!(k, KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4));
            self.pad_focus = false;
            return;
        }
        if !(self.screen == Screen::Menu || self.screen == Screen::Playing && self.battle_overlay_open()) {
            self.pad.nav.clear();
            self.pad_focus = false;
            return;
        }
        if self.screen == Screen::Playing {
            // (Start stays the battle's Escape: the game menu.)
            self.pad.nav.retain(|&n| n != Nav::Menu);
        }
        self.pad.keys.retain(|k| !matches!(k, KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4));
        if self.pad.cursor.is_some() || self.input.mouse_delta.length_squared() > 4.0 {
            self.pad_focus = false;
        }
        if self.pad.nav.iter().any(|n| matches!(n, Nav::Up | Nav::Down | Nav::Left | Nav::Right)) {
            self.pad_focus = true;
        }
        let focused = self.egui.ctx.memory(|m| m.focused()).is_some();
        if self.pad_focus && focused && self.pad.nav.contains(&Nav::Activate) {
            self.pad.left = self.pad.left.filter(|&down| !down);
        } else {
            self.pad.nav.retain(|&n| n != Nav::Activate);
        }
        if self.pad.nav.iter().any(|&n| matches!(n, Nav::Back | Nav::Menu)) {
            self.pad.right = self.pad.right.filter(|&down| !down);
        }
    }

    /// Keeps a replay of the game being left (real games of 20 s or more; not the attract
    /// mode, tutorials or replays themselves).
    /// The player's own music, as the settings have it, into the music's playlists (and the
    /// jukebox's list read again).
    /// The jukebox changed the music's settings: to the music now, and kept (`keep`).
    fn apply_jukebox_settings(&mut self, keep: bool) {
        self.audio.apply_config(&self.settings.audio);
        if keep {
            self.save_settings();
        }
    }

    fn apply_user_music(&mut self) {
        let files = self.jukebox.rescan(&self.settings.audio);
        log::info!("music: {} tracks of the player's own ({:?})", files.len(), self.settings.audio.user_music);
        self.audio.set_user_music(&files, self.settings.audio.user_music);
    }

    /// Bounded fallback for a backdrop not yet prepared by the loading worker; once
    /// complete, the camera goes straight to the fighting. (Fast-forwarded
    /// eight seconds a frame over the menu's first second, the battle ran under it at 240 times
    /// its speed, trees falling and the wind changing every frame, while the camera came in
    /// from wherever it was.)
    fn warm_up_attract(&mut self, most: u32) {
        if self.attract.warmup == 0 || !self.game.spectator {
            return;
        }
        let n = self.attract.warmup.min(most);
        self.game.fast_forward(n);
        self.attract.warmup -= n;
        if self.attract.warmup == 0 {
            log::info!("menu battle fast-forwarded into its fighting ({} s)", self.game.sim.tick / 30);
            self.attract.retarget_in = 0.0;
            self.attract.snap = true;
        }
    }

    /// The menu's backdrop: an AI battle on one of the maps that suit it, fast-forwarded
    /// into its fighting, the camera drifting over it.
    fn start_attract(&mut self) -> Result<()> {
        self.audio.reset_scene();
        // Terrain upload and all 9,000 warm-up ticks have already run behind the
        // loading screen. Never silently fall back to doing them on the UI thread.
        self.game = self.loaded_game.take().context("menu backdrop was not prepared by the loading worker")?;
        self.menu_film = self.loaded_menu_film.take();
        if let Some(f) = &self.menu_film {
            self.menu_film_world = Some(f.world());
            self.renderer.lighting.moon_dir = f.moon_dir();
        }
        self.menu_film_idle = 0.0;
        self.fog_key = None;
        self.camera.zoom = 1.1;
        self.camera.pitch = ATTRACT_PITCH;
        self.camera.mode = ta_render::CameraMode::Perspective;
        self.attract.retarget_in = 0.0;
        self.attract.warmup = 0;
        self.attract.snap = true;
        Ok(())
    }

    /// The encyclopaedia's showroom: `unit` (a type's name) on show in the world of `map` at
    /// `hours` of the day, doing `act`. A new world is a new battle with no one in it; a ship
    /// goes to a world with a sea when this one has none.
    fn show_unit(&mut self, unit: &str, map: &str, hours: f32, act: crate::showroom::Act) -> Result<()> {
        self.show_unit_in(unit, map, hours, act, 0)
    }

    fn show_unit_in(&mut self, unit: &str, map: &str, hours: f32, act: crate::showroom::Act, tries: u32) -> Result<()> {
        self.menu_film = None;
        let fresh = self.showroom.as_ref().is_none_or(|r| !r.map.eq_ignore_ascii_case(map));
        if fresh {
            self.audio.reset_scene();
            self.game = Game::new(&self.assets, &mut self.renderer, &crate::showroom::setup(map))?;
            self.game.tally.enabled = false;
            let mut room = crate::showroom::Showroom::new(map, hours);
            // (The camera keeps its angle from one world to the next.)
            if let Some(old) = &self.showroom {
                (room.yaw, room.pitch, room.distance) = (old.yaw, old.pitch, old.distance);
            }
            self.showroom = Some(room);
        }
        let Some(room) = self.showroom.as_mut() else { return Ok(()) };
        room.hours = hours;
        if !fresh && room.unit_name == unit {
            // (The same unit: only what it does may have changed.)
            if room.act != act {
                crate::showroom::act(&mut self.game, room, act);
            }
            return Ok(());
        }
        match crate::showroom::place(&mut self.game, unit) {
            Ok(p) => {
                let placed = self.game.sim.unit(p.id).map_or(p.centre, |u| Vec3::from_array(u.pos.to_f32()));
                (room.unit, room.unit_name, room.centre, room.ground_centre, room.placed) = (Some(p.id), unit.to_string(), p.centre, p.centre, placed);
                (room.reach, room.height, room.sea, room.frames) = (p.reach, p.height, p.sea, 0);
                // Under the sea, looked down on through it (the sky's reflection grazes the
                // surface from low down).
                if p.sea == crate::showroom::Sea::Under {
                    room.pitch = room.pitch.max(0.62);
                }
                if fresh {
                    self.camera.focus = p.centre;
                    self.camera.pitch = room.pitch;
                }
                room.act = crate::showroom::Act::Stand;
                if act != crate::showroom::Act::Stand {
                    crate::showroom::act(&mut self.game, room, act);
                }
            }
            Err(crate::showroom::Missing::Sea) => {
                // A ship needs the sea: the first world with one.
                let sea = self.menu.encyclopaedia.sea_world(map, tries);
                match sea {
                    Some(sea) if tries < 3 => {
                        self.menu.encyclopaedia.moved_to(&sea);
                        return self.show_unit_in(unit, &sea, hours, act, tries + 1);
                    }
                    _ => log::warn!("showroom: no world with a sea for {unit}"),
                }
            }
            Err(crate::showroom::Missing::Unit) => log::warn!("showroom: no unit {unit}"),
        }
        Ok(())
    }

    /// A picture of the unit on show, when it has no build picture of its own (TA ships none
    /// for a few): drawn once it has stood on the stage a few frames, for the Archives' list and
    /// card (which keep it).
    fn draw_unit_picture(&mut self) {
        const SIZE: u32 = 192;
        let Some(room) = self.showroom.as_ref().filter(|r| r.frames == 3) else { return };
        if !self.menu.encyclopaedia.wants_picture(&room.unit_name) {
            return;
        }
        let Some(type_id) = self.game.sim.types.id(&room.unit_name) else { return };
        let camera = crate::showroom::picture_camera(room, &self.camera, SIZE);
        let saved = self.renderer.post.settings;
        let focus = camera.view_distance(room.centre);
        self.renderer.post.settings.dof = Some(ta_render::post::DepthOfField { mode: ta_render::post::DofMode::Lens, focus, blur: 0.004, band: 0.0 });
        let shot = self.renderer.render_image(&camera, SIZE, SIZE);
        self.renderer.post.settings = saved;
        if let Some(shot) = shot {
            let name = room.unit_name.clone();
            self.menu.encyclopaedia.picture_drawn(&self.egui.ctx, &self.assets, type_id, &name, shot.width, shot.height, &shot.rgba);
        }
    }

    /// How much of the window's height a wallpaper's preview takes, the picture `aspect` (width
    /// over height) wide: its lens is narrowed to that share, so the picture is the preview's.
    /// Opens the companion screen, or closes it.
    fn toggle_companion(&mut self, event_loop: &ActiveEventLoop) {
        if self.companion.take().is_some() {
            log::info!("companion screen closed");
            return;
        }
        match crate::companion::Companion::open(event_loop, &self.renderer.gpu, &self.window, self.exit_after_screenshot) {
            Ok(companion) => self.companion = Some(companion),
            Err(e) => {
                log::warn!("{e:#}");
                self.game.trouble(ta_ui::i18n::tf("The companion screen could not be opened: {error}", &[("error", &format!("{e:#}"))]));
            }
        }
    }

    /// A battle begins: the companion screen opens on a second display when the settings say
    /// so (captures: `TA_COMPANION=1`), or the player is told, once, that it can be had.
    fn companion_at_battle(&mut self, event_loop: &ActiveEventLoop) {
        if self.companion.is_some() || self.benchmark.is_some() {
            return;
        }
        let capture = std::env::var("TA_COMPANION").is_ok_and(|v| v == "1") || self.companion_capture;
        let second = !self.exit_after_screenshot && crate::companion::other_display(event_loop, &self.window).is_some();
        if capture || second && self.settings.gameplay.companion {
            self.toggle_companion(event_loop);
        } else if second && !self.companion_hinted {
            self.companion_hinted = true;
            self.game.message(ta_ui::i18n::tf("{key} opens the companion screen on your second display: the map and intel beside the battle", &[("key", &self.game.keys.text("companion"))]));
        }
    }

    /// The companion screen's frame (some thirty a second): its page, and the portraits its
    /// panels asked for, drawn as the battle's are and handed to its interface.
    fn companion_frame(&mut self) {
        let Some(companion) = self.companion.as_mut() else { return };
        // (Captures: `TA_COMPANION_SHOT=<file>` at `TA_COMPANION_AT=<frame>` saves its picture.)
        let snapshot = std::env::var("TA_COMPANION_AT").ok().and_then(|v| v.parse::<u32>().ok()).filter(|&at| at == self.frames).and(std::env::var_os("TA_COMPANION_SHOT")).map(std::path::PathBuf::from);
        if !companion.due() && snapshot.is_none() {
            return;
        }
        for key @ (type_id, colour, size, enemy) in companion.hud.take_portrait_requests().into_iter().take(PORTRAITS_PER_FRAME) {
            if !self.portrait_views.contains_key(&key) {
                let view = self.game.scene.model_for_side(type_id, enemy).and_then(|model| {
                    let (pieces, ground) = (crate::scene::standing_pose(self.game.sim.types.get(type_id)), self.game.sim.types.get(type_id).def.stands_on_land());
                    let studio = self.portrait_studio.get_or_insert_with(|| ta_render::portrait::PortraitStudio::new(&self.renderer));
                    let picture = studio.render(&mut self.renderer, &ta_render::portrait::PortraitRequest { model, pieces: &pieces, team: colour, size, ground })?;
                    Some(picture.create_view(&wgpu::TextureViewDescriptor::default()))
                });
                if let Some(view) = view {
                    self.portrait_views.insert(key, view);
                }
            }
            let id = self.portrait_views.get(&key).map(|view| companion.register_portrait(&self.renderer.gpu, view));
            companion.hud.set_portrait(key, id);
        }
        let battle = self.screen == Screen::Playing;
        let key = self.game.keys.text("companion").to_string();
        let (game, camera, vfs) = (&mut self.game, &mut self.camera, &self.assets.vfs);
        companion.snapshot = snapshot;
        let waiting = (!battle).then_some(key.as_str());
        if let Some(sound) = companion.frame(&self.renderer.gpu, |ui, hud, minimap| hud.companion_page(ui, vfs, game, camera, minimap, waiting)) {
            self.audio.play_interface(sound.event());
        }
        // (Its own close button.)
        if self.companion.as_mut().is_some_and(|c| c.hud.take_companion_close()) {
            self.companion = None;
            log::info!("companion screen closed");
        }
    }

    /// Starts a film of the replay as `settings` chose (`film.rs`): its file opened, the battle
    /// set for it (playing at its own speed, nothing selected, the director at the camera unless
    /// the view is kept), and off to the first stretch's start. A social media clip
    /// (`settings.clip`) goes to the Clips folder, its camera planned from the replay's scan.
    fn start_film(&mut self, settings: crate::film::FilmSettings) {
        use ta_ui::i18n::{t, tf};
        let Some(end) = self.game.playback.as_ref().filter(|p| !p.resume).map(|p| p.end_tick) else { return };
        let (marks, action) = self.game.replay_scan.as_ref().and_then(|s| s.state().lock().ok().map(|s| (s.marks.clone(), s.action.clone()))).unwrap_or_default();
        let stretches = crate::film::stretches(&settings, end, &marks);
        if stretches.is_empty() {
            self.game.message(t("There is nothing to film there"));
            return;
        }
        // (A clip's camera follows its plan, made from what the scan saw.)
        let clip = settings.clip.clone();
        if clip.is_some() && action.is_none() {
            self.game.message(t("The replay is still being looked through: try again in a moment"));
            return;
        }
        let map = map_title(&self.game, &self.assets);
        let (dir, path) = match &clip {
            Some(c) => {
                let dir = crate::clips::clips_dir();
                let path = crate::clips::clip_path(&dir, &map, c.from, c.spec);
                (dir, path)
            }
            None => {
                let dir = ta_config::videos_dir();
                let path = crate::film::film_path(&dir, &map);
                (dir, path)
            }
        };
        let writer = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|()| {
            let made = if clip.is_some() { ta_mp4::Mp4Writer::create_fast_start(&path, settings.pictures(), settings.sound()) } else { ta_mp4::Mp4Writer::create(&path, settings.pictures(), settings.sound()) };
            made.map_err(|e| e.to_string())
        });
        let writer = match writer {
            Ok(w) => w,
            Err(e) => {
                log::warn!("film: {}: {e}", path.display());
                self.game.trouble(tf("The video could not be started: {error}", &[("error", &e)]));
                return;
            }
        };
        let game = &self.game;
        let players = game.setup.players.iter().enumerate().filter_map(|(i, _)| {
            let p = game.sim.players.get(i)?;
            Some((game.player_label(i as u8), p.side.to_uppercase(), crate::palette::team_color(p.color)))
        });
        let card = crate::film::Card { map, players: players.collect() };
        let clip_film = match (&clip, &action) {
            (Some(c), Some(log)) => {
                let plan = crate::clips::plan(log, c.from, c.to, c.climax, c.focus);
                log::info!(
                    "clip: {} to {}, climax {} (spread {:.0}), cuts at {:?}",
                    crate::clips::clock(c.from),
                    crate::clips::clock(c.to),
                    crate::clips::clock(c.climax),
                    plan.at(c.climax as f32).1,
                    plan.cuts().iter().map(|&t| crate::clips::clock(t)).collect::<Vec<_>>()
                );
                let look = crate::clips::ClipLook {
                    shape: c.spec.shape,
                    seconds: c.to.min(end).saturating_sub(c.from) as f32 / 30.0,
                    start: c.from,
                    map: card.map.clone(),
                    players: card.players.clone(),
                    callouts: crate::clips::callouts(&marks, c.from, c.to.min(end), &plan, c.spec.shape),
                    titles: settings.titles,
                    scoreboard: settings.scoreboard,
                };
                Some(crate::film::ClipFilm { plan, look, limiter: crate::clips::Limiter::new(crate::film::RATE) })
            }
            _ => None,
        };
        // (After the EXPORT CLIP dialog's pictures, the replay as it was before them.)
        let (camera, speed) = match self.take_clip_back() {
            Some(back) => (back.camera, back.speed),
            None => (self.camera.clone(), self.game.speed),
        };
        let restore = crate::film::Restore { camera, speed, cast: self.game.cast, action_camera: self.game.action_camera };
        log::info!("film: {} ({} x {}, {} a second, {:.0} s of {} stretches)", path.display(), settings.size.0, settings.size.1, settings.fps, crate::film::length(&stretches), stretches.len());
        let from = stretches[0].from;
        self.game.selection.clear();
        self.game.follow = None;
        self.game.set_pace(Some(false), Some(1.0));
        // (A clip has a scoreboard of its own, and its own camera.)
        self.game.cast = settings.scoreboard && clip.is_none();
        self.game.action_camera = clip.is_none() && settings.shot != crate::film::Shot::AsSet;
        (self.in_game_menu, self.options_open, self.controls_open, self.game.stats_open, self.game.history_open) = (false, false, false, false, false);
        self.renderer.set_film_size(Some(settings.size));
        self.film_ui = None;
        let film = crate::film::Film::new(settings, path, card, stretches, marks, restore, writer);
        self.film = Some(match clip_film {
            Some(c) => film.with_clip(c),
            None => film,
        });
        self.game.seek_replay(from);
    }

    /// After the frame's ticks: a film arrived at a stretch's start begins rolling (the film's
    /// sound in the speakers' place from its first picture, the camera where a big moment is),
    /// and while it rolls the cinematic camera circles and the big moments reached are named.
    fn film_after_update(&mut self, dt: f32) {
        let Some(film) = self.film.as_mut() else { return };
        film.poll();
        if self.screen != Screen::Playing || self.game.playback.is_none() {
            if !matches!(film.phase, crate::film::Phase::Done(_)) {
                film.fail(ta_ui::i18n::t("The replay was left").to_string());
            }
            return;
        }
        let cinematic = film.settings.shot == crate::film::Shot::Cinematic;
        match film.phase {
            crate::film::Phase::Seeking => {
                let arrived = self.game.catch_up_progress().is_none() && self.game.request_rewind.is_none() && self.pending.is_none();
                if !arrived {
                    return;
                }
                // (Done seeking somewhere else, another seek having come between: there again.)
                if !film.arrived_at(self.game.sim.tick) {
                    let from = film.current().from;
                    self.game.seek_replay(from);
                    return;
                }
                if film.speakers.is_none() {
                    let mut audio = GameAudio::film(self.assets.vfs.clone(), &self.settings.audio, crate::film::RATE, film.settings.sounds, film.settings.music);
                    if film.settings.music {
                        audio.set_user_music(&self.jukebox.rescan(&self.settings.audio), self.settings.audio.user_music);
                        audio.set_mood(ta_audio::Mood::Peace);
                    }
                    film.speakers = Some(std::mem::replace(&mut self.audio, audio));
                }
                // A clip's camera follows its plan; a big moment's stretch starts where it is;
                // otherwise the director chooses.
                if film.clip.is_some() {
                    film.roll();
                    self.clip_film_camera();
                    return;
                }
                match film.current().focus {
                    Some((at, spread)) => {
                        self.camera.focus.x = at.x;
                        self.camera.focus.z = at.z;
                        self.camera.zoom = crate::highlights::zoom_for(&self.camera, spread * film.settings.shot.framing());
                        (self.game.action_target, self.game.action_spread) = (Some(at), spread);
                    }
                    None => self.game.action_target = None,
                }
                (self.game.action_held, self.game.action_retarget) = (0.0, 0.0);
                if cinematic {
                    self.camera.mode = CameraMode::Perspective;
                    (self.camera.yaw, self.camera.pitch) = (film.yaw, CINEMATIC_PITCH);
                }
                film.roll();
            }
            crate::film::Phase::Rolling if film.clip.is_some() => self.clip_film_camera(),
            crate::film::Phase::Rolling => {
                if cinematic {
                    // Low, circling slowly round the fight the director chose.
                    film.yaw += dt * 0.045;
                    let k = 1.0 - (-dt * 1.5).exp();
                    let turn = (film.yaw - self.camera.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                    self.camera.mode = CameraMode::Perspective;
                    self.camera.yaw += turn * k;
                    self.camera.pitch += (CINEMATIC_PITCH - self.camera.pitch) * k;
                }
                film.note_moments(self.game.sim.tick);
            }
            _ => {}
        }
    }

    /// Portraits requested by the film's scoreboard and kill feed on the previous frame.
    /// Preparing them here keeps the portrait studio from replacing a prepared battle's buffers.
    fn film_portraits(&mut self) {
        let Some(ui) = self.film_ui.as_mut() else { return };
        let mut wanted = ui.hud.take_portrait_requests();
        wanted.truncate(PORTRAITS_PER_FRAME);
        wanted.sort_by_key(|key| key.2);
        for key @ (type_id, colour, size, enemy) in wanted {
            if !self.portrait_views.contains_key(&key) {
                let view = self.game.scene.model_for_side(type_id, enemy).and_then(|model| {
                    let (pieces, ground) = (crate::scene::standing_pose(self.game.sim.types.get(type_id)), self.game.sim.types.get(type_id).def.stands_on_land());
                    let studio = self.portrait_studio.get_or_insert_with(|| ta_render::portrait::PortraitStudio::new(&self.renderer));
                    let picture = studio.render(&mut self.renderer, &ta_render::portrait::PortraitRequest { model, pieces: &pieces, team: colour, size, ground })?;
                    Some(picture.create_view(&wgpu::TextureViewDescriptor::default()))
                });
                if let Some(view) = view { self.portrait_views.insert(key, view); }
            }
            ui.set_portrait(&self.renderer.gpu.device, key, self.portrait_views.get(&key));
        }
    }

    /// The film's picture of this frame: drawn at its size with its own pass of the interface
    /// over it (the cast, the titles, the fade), the frame's sound mixed with it, both written;
    /// then on, to the next stretch, or to the end.
    fn film_picture(&mut self) {
        let capturing = self.capture_dt().is_some();
        let Some(film) = self.film.as_mut() else { return };
        let (w, h) = film.settings.size;
        let tick = self.game.sim.tick;
        let fade = film.fade(tick);
        let seconds = film.seconds();
        if !capturing {
            self.renderer.set_animation_time(Some(seconds % 65536.0));
        }
        // The film's interface: laid out as the game's is on a screen of its shape, 600 points
        // high (or 1,024 wide) at most, at the picture's size. (A clip's titles: a point a pixel
        // at 1,080 across its narrow side.)
        let ppp = if film.clip.is_some() { (w.min(h) as f32 / 1080.0).max(0.5) } else { (h as f32 / 600.0).min(w as f32 / 1024.0).max(0.5) };
        let device = self.renderer.gpu.device.clone();
        let queue = self.renderer.gpu.queue.clone();
        if self.film_ui.is_none() {
            let Some(ui) = crate::film::PictureUi::new(&self.egui.ctx, &device, self.renderer.gpu.config.format, self.hud.classic) else {
                film.fail(ta_ui::i18n::t("Not enough graphics memory to export the video").to_string());
                self.game.set_pace(Some(true), None);
                return;
            };
            self.film_ui = Some(ui);
        }
        let Some(ui) = self.film_ui.as_mut() else { return };
        let (ctx, egui_renderer, hud) = (&ui.ctx, &mut ui.renderer, &mut ui.hud);
        let zoom = ctx.zoom_factor();
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32 / ppp, h as f32 / ppp))),
            time: Some(self.egui.ctx.input(|i| i.time)),
            focused: true,
            max_texture_side: Some(device.limits().max_texture_dimension_2d as usize),
            ..Default::default()
        };
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp / zoom);
        let (game, vfs) = (&mut self.game, &self.assets.vfs);
        // (A clip draws a scoreboard of its own, from the armies' worth.)
        let cast = film.settings.scoreboard && film.clip.is_none();
        let armies = if film.clip.is_some() { crate::clips::armies(game) } else { Vec::new() };
        let shown: &crate::film::Film = film;
        let output = ctx.run_ui(raw, |ui| {
            hud.film_overlay(ui.ctx(), vfs, game, cast, seconds as f64);
            shown.overlay(ui.ctx(), fade, tick, &armies);
        });
        let out_ppp = output.pixels_per_point;
        let prims = ctx.tessellate(output.shapes, out_ppp);
        let mut textures_delta = output.textures_delta;
        for (id, deltas) in textures_delta.set.drain() {
            for delta in deltas {
                egui_renderer.update_texture(&device, &queue, id, &delta);
            }
        }
        let shot = self.renderer.render_image_with(&self.camera, w, h, |device, queue, encoder, view| {
            let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [w, h], pixels_per_point: out_ppp };
            // (Not when the card refuses its staging memory, as egui-wgpu panics then: this frame
            // goes without the interface.)
            if ta_render::gpu::unless_short_of_memory(|| egui_renderer.update_buffers(device, queue, encoder, &prims, &sd)).is_none() {
                return;
            }
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("film interface"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }, depth_slice: None })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            egui_renderer.render(&mut pass, &prims, &sd);
        });
        for id in textures_delta.free.drain() {
            egui_renderer.free_texture(&id);
        }
        let Some(film) = self.film.as_mut() else { return };
        let Some(shot) = shot else {
            film.fail(ta_ui::i18n::tf("The graphics card could not draw a picture {width} × {height}", &[("width", &w), ("height", &h)]));
            self.game.set_pace(Some(true), None);
            return;
        };
        // Its sound, rising and falling with the picture (a clip's kept under its ceiling).
        let mut sound = vec![0.0f32; film.sound_frames() * 2];
        self.audio.mix(&mut sound);
        if let Some(clip) = film.clip.as_mut() {
            clip.limiter.process(&mut sound);
        }
        let level = film.sound_level(tick);
        if level < 1.0 {
            sound.iter_mut().for_each(|s| *s *= level);
        }
        if let Err(e) = film.write(&self.egui.ctx, shot.rgba, w, h, fade, sound) {
            film.fail(e);
            self.game.set_pace(Some(true), None);
            return;
        }
        match film.next(tick) {
            crate::film::Next::Seek(to) => self.game.seek_replay(to),
            // (The replay waits where the film ended, behind its page.)
            crate::film::Next::Finished => self.game.set_pace(Some(true), None),
            crate::film::Next::Roll => {}
        }
    }

    /// What the film's page asked for: to stop it, to see the film, or to go back to the replay.
    fn film_page_action(&mut self, action: crate::film::PageAction) {
        use crate::film::PageAction;
        let Some(path) = self.film.as_ref().map(|f| f.path.clone()) else { return };
        match action {
            PageAction::ShowFolder => crate::film::show_in_folder(&path),
            PageAction::Watch => crate::film::watch(&path),
            PageAction::Cancel => {
                log::info!("film: cancelled");
                if let Some(playback) = self.game.playback.as_mut().filter(|p| !p.resume) {
                    playback.seek_to = None;
                    self.game.request_rewind = None;
                    // Finish the seek's scene refresh at the current tick, without
                    // simulating the rest of the stretch after export was cancelled.
                    self.game.catch_up(std::time::Duration::ZERO);
                }
                if matches!(self.pending, Some(Transition::Rewind(..))) { self.pending = None; }
                self.end_film();
                self.game.message(ta_ui::i18n::t("Video export cancelled"));
            }
            PageAction::Back => self.end_film(),
        }
    }

    /// The film over (made, failed or left): the speakers' sound back, the camera and the
    /// replay's pace as they were (paused where the film ended), the scene's targets the window's.
    fn end_film(&mut self) {
        let Some(mut film) = self.film.take() else { return };
        self.film_ui = None;
        if let Some(speakers) = film.speakers.take() {
            self.audio = speakers;
        }
        self.renderer.set_film_size(None);
        if self.capture_dt().is_none() {
            self.renderer.set_animation_time(None);
        }
        let restore = &film.restore;
        self.camera = restore.camera.clone();
        (self.game.cast, self.game.action_camera) = (restore.cast, restore.action_camera);
        self.game.set_pace(Some(true), Some(restore.speed));
    }

    fn wallpaper_share(&self, aspect: f32) -> f32 {
        let ctx = &self.egui.ctx;
        let screen = ctx.content_rect();
        crate::encyclopaedia::preview_rect(screen, crate::encyclopaedia::bar_height(ctx), aspect).height() / screen.height().max(1.0)
    }

    /// A wallpaper of the unit on show, `width` × `height`: the view its preview shows drawn at
    /// that size (the unit to the right of its name and numbers, laid out on the left, or alone
    /// in the middle without `words`), saved in the pictures folder's Wallpapers as it is
    /// written.
    fn wallpaper(&mut self, width: u32, height: u32, words: bool) {
        let now = self.egui.ctx.input(|i| i.time);
        let share = self.wallpaper_share(width as f32 / height.max(1) as f32);
        let Some(room) = self.showroom.as_ref() else { return };
        let Some(type_id) = self.game.sim.types.id(&room.unit_name) else { return };
        let camera = crate::showroom::wallpaper_camera(room, &self.camera, (width, height), words, share);
        let saved = self.renderer.post.settings;
        self.renderer.post.settings.dof = Some(crate::showroom::wallpaper_lens(room, &camera, 1.0));
        // The words: an interface of their own, its page 1080 points high.
        let device = self.renderer.gpu.device.clone();
        let queue = self.renderer.gpu.queue.clone();
        let format = self.renderer.gpu.config.format;
        if self.wallpaper_ui.is_none() {
            let Some(renderer) = crate::loading::ui_renderer(&device, format, egui_wgpu::RendererOptions { msaa_samples: 1, depth_stencil_format: None, dithering: true, ..Default::default() }) else {
                log::warn!("wallpaper: not enough graphics memory for its words");
                return;
            };
            let ctx = egui::Context::default();
            crate::theme::install_compact(&ctx);
            self.wallpaper_ui = Some((ctx, renderer));
        }
        let Some((ctx, egui_renderer)) = self.wallpaper_ui.as_mut() else { return };
        let ppp = height as f32 / 1080.0;
        let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width as f32 / ppp, 1080.0))), max_texture_side: Some(device.limits().max_texture_dimension_2d as usize), ..Default::default() };
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
        let game = &self.game;
        let ty = game.sim.types.get(type_id);
        let output = ctx.run_ui(raw, |ui| {
            let (painter, page) = (ui.ctx().layer_painter(egui::LayerId::background()), ui.ctx().content_rect());
            if words {
                crate::showroom::wallpaper_words(&painter, page, game, ty);
            } else {
                crate::showroom::wallpaper_vignette(&painter, page);
            }
        });
        let prims = ctx.tessellate(output.shapes, ppp);
        let mut textures_delta = output.textures_delta;
        for (id, deltas) in textures_delta.set.drain() {
            for delta in deltas {
                egui_renderer.update_texture(&device, &queue, id, &delta);
            }
        }
        let shot = self.renderer.render_image_with(&camera, width, height, |device, queue, encoder, view| {
            let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [width, height], pixels_per_point: ppp };
            // (Not when the card refuses its staging memory, as egui-wgpu panics then: this frame
            // goes without the interface.)
            if ta_render::gpu::unless_short_of_memory(|| egui_renderer.update_buffers(device, queue, encoder, &prims, &sd)).is_none() {
                return;
            }
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("wallpaper words"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }, depth_slice: None })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            egui_renderer.render(&mut pass, &prims, &sd);
        });
        for id in textures_delta.free.drain() {
            egui_renderer.free_texture(&id);
        }
        self.renderer.post.settings = saved;
        let Some(shot) = shot else {
            self.menu.encyclopaedia.wallpaper_saving(None, now);
            return;
        };
        let name = game.sim.types.get(type_id).def.display_name.clone();
        let label = crate::encyclopaedia::SIZES.iter().find(|s| s.1 == width && s.2 == height).map_or_else(|| format!("{width}x{height}"), |s| s.0.to_string());
        let dir = ta_config::pictures_dir().join("Wallpapers");
        let (send, receive) = std::sync::mpsc::channel();
        let write = move || {
            let result = std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|_| {
                let path = ta_config::next_picture(&dir, &format!("{name} {label}"));
                image::save_buffer(&path, &shot.rgba, shot.width, shot.height, image::ColorType::Rgba8).map(|_| path).map_err(|e| e.to_string())
            });
            send.send(result).ok();
        };
        // (A capture waits for it, or the program would end first.)
        if self.exit_after_screenshot {
            write();
        } else {
            std::thread::spawn(write);
        }
        self.menu.encyclopaedia.wallpaper_saving(Some(receive), now);
    }

    fn open_career(&mut self, achievements: bool) {
        let Some(profile) = &self.profile else { return };
        let campaigns = self.assets.campaigns.iter().map(|c| ta_ui::profiles::CampaignSummary {
            id: c.id.clone(), title: c.title.clone(), missions: c.missions.iter().map(|m| m.name.clone()).collect(),
        }).collect();
        self.career_window.open(&self.egui.ctx, profile, campaigns, achievements);
    }

    /// Commit a first profile before the pending launch is allowed to issue orders.
    fn create_profile(&mut self, draft: ta_ui::profiles::ProfileDraft) {
        let result = (|| -> anyhow::Result<ta_config::profiles::Profile> {
            let root = ta_config::config_dir();
            let store = ta_config::profiles::ProfileStore::open(root.clone())?;
            let mut profile = store.create(&draft.name, draft.allegiance, draft.avatar_png.as_deref(), &self.settings)?;
            if !draft.motto.is_empty() { profile = store.set_motto(&profile.id, &draft.motto)?; }
            if draft.clear_avatar { profile = store.remove_avatar(&profile.id)?; }
            Ok(ta_config::profiles::pin(root, &profile.id)?)
        })();
        match result {
            Ok(profile) => {
                self.settings = ta_config::Config::load();
                self.settings.gameplay.player_name = profile.name.clone();
                self.progress = ta_config::CampaignProgress::load();
                let page = self.menu.page;
                self.menu = Menu::new(&self.assets);
                self.menu.page = page;
                self.menu.rules.unit_limit = self.settings.gameplay.unit_limit;
                self.menu.use_profile(&self.assets, &profile);
                self.game.player_name = profile.name.clone();
                self.game.profile_allegiance = Some(profile.allegiance);
                self.career = ta_config::profiles::ProfileStore::open(ta_config::config_dir()).and_then(|s| s.career(&profile.id)).unwrap_or_default();
                self.profile = Some(profile);
                self.profile_editor = None;
                if let Some((screen, pending)) = self.profile_resume.take() {
                    self.screen = screen;
                    self.pending = pending;
                }
                if let Some((host, join)) = self.profile_network.take() {
                    let lobby = match (host, join) {
                        (Some(port), _) => Some(crate::lobby::NetLobby::host_now(port, &self.menu.connect.name, &self.assets)),
                        (None, Some(address)) => Some(crate::lobby::NetLobby::join_now(&address, &self.menu.connect.name, &self.assets)),
                        _ => None,
                    };
                    match lobby {
                        Some(Ok(lobby)) => { self.menu.lobby = Some(lobby); self.menu.page = crate::menu::Page::Lobby; }
                        Some(Err(error)) => { self.menu.connect.tell(error.to_string()); self.menu.page = crate::menu::Page::Multiplayer; }
                        None => {}
                    }
                }
                self.game.paused = false;
                self.input.ignore();
            }
            Err(error) => if let Some(editor) = &mut self.profile_editor { editor.error = Some(error.to_string()); },
        }
    }

    /// A cumulative ledger makes checkpoints, save reloads and process retries idempotent.
    /// Only presentation statistics are observed; simulation and multiplayer stay unchanged.
    fn checkpoint_profile(&mut self, force: bool) {
        if !self.profile_eligible || self.profile_editor.is_some() || self.screen != Screen::Playing || self.game.spectator && !self.game.profile_progress.has_completed() || self.game.staged || self.game.playback.is_some() || self.reel.is_some() {
            return;
        }
        self.game.profile_progress.disqualified |= self.game.cheated();
        if self.game.profile_progress.disqualified {
            // (Cheats: what the battle had earned is taken back, the once.)
            if !self.game.profile_progress.voided && self.game.profile_progress.void_due() {
                self.void_battle();
            }
            return;
        }
        if self.game.sim.tick < self.game.profile_progress.resume_at_tick { return; }
        let Some(profile) = &self.profile else { return };
        let finished = self.game.outcome.is_some();
        if self.game.profile_progress.needs_snapshot(finished) {
            let counters = self.game.profile_progress.counters(&self.game);
            self.game.profile_progress.snapshot(&counters);
        }
        // Between checkpoints, a look ahead once a second of battle: an achievement earned this
        // moment is saved, and pictured, now rather than at the next checkpoint.
        let mut force = force;
        if !force && !finished && !self.game.profile_progress.needs_baseline && self.game.sim.tick >= self.unlock_look_tick {
            self.unlock_look_tick = self.game.sim.tick + 30;
            let counters = self.game.profile_progress.counters(&self.game);
            let saved = self.game.profile_progress.saved_counters.clone().unwrap_or_default();
            let context = crate::profile_progress::context(&self.game);
            let ahead = self.career.with_progress(&saved, &counters, &context, self.game.profile_progress.full_match());
            for achievement in ahead.pending() {
                force |= self.unlock_looked.insert(achievement.id);
            }
        }
        if !self.game.profile_progress.due(self.game.sim.tick, finished, force) { return; }
        let counters = self.game.profile_progress.counters(&self.game);
        let result = (|| -> std::io::Result<_> {
            let store = ta_config::profiles::ProfileStore::open(ta_config::config_dir())?;
            let context = crate::profile_progress::context(&self.game);
            let award = self.game.profile_progress.award(&store, &profile.id, &counters, &context)?;
            Ok((award, store.career(&profile.id)?))
        })();
        match result {
            Ok((award, career)) => {
                self.match_xp = self.match_xp.saturating_add(award.xp);
                self.career = career;
                self.game.profile_progress.saved(self.game.sim.tick, counters.finished);
                self.game.profile_progress.remember_saved(&counters);
                if award.new_rank.level > award.old_rank.level {
                    self.game.message(ta_ui::i18n::tf("Promoted to rank {rank}", &[("rank", &award.new_rank.level)]));
                }
                for id in &award.achievements {
                    if let Some(achievement) = ta_config::achievements::find(id) {
                        self.game.message(ta_ui::i18n::tf("Achievement unlocked: {achievement}", &[("achievement", &ta_ui::i18n::t(achievement.name))]));
                    }
                }
                self.achievement_toasts.push(&award.achievements, &award.earlier);
                // (A new title or a service star: the notice of a promotion, after the achievements.)
                if (award.new_rank.grade, award.new_rank.stars) > (award.old_rank.grade, award.old_rank.stars) {
                    self.achievement_toasts.push_promotion(award.new_rank);
                }
                let moment: Vec<String> = award.achievements.iter().filter(|id| !award.earlier.contains(id)).cloned().collect();
                if !moment.is_empty() { self.unlock_picture.get_or_insert_with(Vec::new).extend(moment); }
                self.battle_achievements.extend(award.achievements.iter().cloned());
                self.battle_achievement_xp = self.battle_achievement_xp.saturating_add(award.achievement_xp);
                if let Some(xp_before) = self.battle_start_xp {
                    self.game.career_record = Some(ta_ui::profiles::BattleRecord {
                        xp_before, xp_now: self.career.total_xp(), battle_xp: self.match_xp,
                        breakdown: self.game.profile_progress.full_match().then_some(award.breakdown),
                        achievements: self.battle_achievements.clone(), achievement_xp: self.battle_achievement_xp, voided: false,
                    });
                }
            }
            Err(error) => {
                log::warn!("could not checkpoint player career: {error}");
                // Retry on the next interval, without dropping a failed import baseline.
                self.game.profile_progress.retry_after();
            }
        }
    }

    /// Cheats were used: what this battle earned is taken back from the career, and it earns
    /// nothing more (`ProfileStore::void_match`); the player is told, and the result's record
    /// says so.
    fn void_battle(&mut self) {
        let Some(profile) = &self.profile else {
            self.game.profile_progress.voided = true;
            return;
        };
        let result = (|| -> std::io::Result<_> {
            let store = ta_config::profiles::ProfileStore::open(ta_config::config_dir())?;
            let voided = store.void_match(&profile.id, &self.game.profile_progress.id)?;
            Ok((voided, store.career(&profile.id)?))
        })();
        match result {
            Ok((voided, career)) => {
                self.game.profile_progress.voided = true;
                self.career = career;
                self.match_xp = 0;
                self.battle_achievements.clear();
                self.battle_achievement_xp = 0;
                self.unlock_picture = None;
                let now = self.career.total_xp();
                self.game.career_record = Some(ta_ui::profiles::BattleRecord { xp_before: now, xp_now: now, voided: true, ..Default::default() });
                self.game.message(if voided.xp > 0 || !voided.achievements.is_empty() {
                    ta_ui::i18n::t("Cheats used: what this battle earned is taken back, and it earns no XP or achievements")
                } else {
                    ta_ui::i18n::t("Cheats used: this battle earns no XP or achievements")
                });
            }
            Err(error) => {
                log::warn!("could not take back the battle's career progress: {error}");
                self.game.profile_progress.retry_after();
            }
        }
    }

    /// The picture of the moment an achievement was earned: the battle as the camera sees it,
    /// without the interface, at the window's size (the scene's own targets, no resizing),
    /// brought down and kept as a JPEG in the profile's `achievements` folder on a worker.
    fn take_unlock_picture(&mut self) {
        let Some(ids) = self.unlock_picture.take() else { return };
        let Some(profile) = self.profile.as_ref().map(|p| p.id.clone()) else { return };
        if self.screen != Screen::Playing { return; }
        let size = self.window.inner_size();
        let scale = self.renderer.render_scale();
        let (w, h) = (((size.width as f32 * scale).round() as u32).max(1), ((size.height as f32 * scale).round() as u32).max(1));
        let Some(shot) = self.renderer.render_image(&self.camera, w, h) else {
            log::warn!("could not draw the picture of an achievement");
            return;
        };
        std::thread::spawn(move || {
            let result = crate::profile_progress::unlock_picture(&shot.rgba, shot.width, shot.height)
                .and_then(|jpeg| ta_config::profiles::ProfileStore::open(ta_config::config_dir())
                    .and_then(|store| store.save_achievement_picture(&profile, &ids, &jpeg)).map_err(|e| e.to_string()));
            if let Err(error) = result { log::warn!("could not keep the picture of an achievement: {error}"); }
        });
    }

    /// A saved game or replay's mods turned on (`order`, and no others), and the game started
    /// again on it: mods are mounted as the game starts. This one closes once the next is on its
    /// way; if it cannot be started, the player is told to start it again.
    fn restart_with_mods(&mut self, path: &std::path::Path, replay: bool, order: Vec<String>) {
        log::info!("starting again with the mods {order:?} for {}", path.display());
        self.settings.mods.enabled = order;
        if let Err(e) = self.settings.save() {
            log::warn!("could not save the settings: {e}");
            self.menu.failure = Some((ta_ui::i18n::n("SETTINGS NOT SAVED"), ta_ui::i18n::tf("Your settings could not be saved: {error}", &[("error", &e)])));
            return;
        }
        match crate::record_mods::restart(path, replay) {
            Ok(()) => self.pending = Some(Transition::Quit),
            Err(e) => {
                log::warn!("could not start again: {e}");
                self.menu.failure = Some((ta_ui::i18n::n("START TA REBORN AGAIN"), ta_ui::i18n::tf("The battle's mods are turned on, but TA Reborn could not start again by itself ({error}). Close it and start it again to play with them.", &[("error", &e)])));
            }
        }
    }

    /// The launcher's Mods page on a mod a record needs (started, or the one running asked).
    fn get_mod_in_launcher(&mut self, id: &str) {
        // (Not from a capture: it would leave a launcher open.)
        if self.exit_after_screenshot {
            log::info!("capture: the launcher would open on the mod {id}");
            return;
        }
        if let Err(e) = crate::record_mods::open_launcher_on(id) {
            log::warn!("could not open the launcher: {e}");
            self.menu.failure = Some((ta_ui::i18n::n("LAUNCHER NOT OPENED"), ta_ui::i18n::tf("The launcher could not be opened ({error}). Open it yourself: its Mods page gets mods.", &[("error", &e)])));
        }
    }

    /// Saves the settings; a failure is said where the player is (in the battle, or a dialog in
    /// the menus): it was only logged, and the changes were gone at the next start.
    fn save_settings(&mut self) {
        if let Err(e) = self.settings.save() {
            log::warn!("could not save the settings: {e}");
            let text = ta_ui::i18n::tf("Your settings could not be saved: {error}", &[("error", &e)]);
            if self.screen == Screen::Playing {
                self.game.trouble(text);
            } else {
                self.menu.failure = Some((ta_ui::i18n::n("SETTINGS NOT SAVED"), text));
            }
        }
    }

    /// The player's name is now `name`: the settings' and, with a profile, its commander's (so
    /// Options, the lobby and the battles say the same).
    fn rename_commander(&mut self, name: String) {
        self.settings.gameplay.player_name = name.clone();
        if let Some(profile) = &self.profile {
            match ta_config::profiles::ProfileStore::open(ta_config::config_dir()).and_then(|store| store.update(&profile.id, &name, profile.allegiance)) {
                Ok(profile) => self.profile = Some(profile),
                Err(error) => log::warn!("could not rename the commander: {error}"),
            }
        }
    }

    fn flush_progress(&mut self, force: bool) {
        let flushed = self.progress_writes.flush(force);
        if let Some(progress) = flushed.progress { self.progress = progress; }
        if flushed.conquest_changed && self.screen == Screen::Menu {
            self.menu.conquest.reload();
        }
        if let Some(error) = flushed.error {
            log::warn!("could not save player progress: {error}");
            // (Said in the battle, and in the menus too: a battle being left takes its messages
            // with it.)
            let text = ta_ui::i18n::tf("Your progress could not be saved: {error}", &[("error", &error)]);
            if self.screen == Screen::Playing {
                self.game.trouble(text.clone());
            }
            self.menu.failure = Some((ta_ui::i18n::n("PROGRESS NOT SAVED"), text));
        }
    }

    fn archive_replay(&mut self) {
        self.checkpoint_profile(true);
        self.flush_progress(true);
        // (The benchmark's battles are nobody's to watch again.)
        if self.benchmark.is_some() {
            return;
        }
        let g = &self.game;
        // Demo scripts place units outside the order stream, so they cannot be replayed. (A
        // player who lost and watched on is a spectator, but it was their battle: kept.)
        if !g.keeps_replay() || !self.script.is_empty() {
            return;
        }
        let Some(ticks) = g.record.snapshot() else { return };
        let title = record_title(g, &self.assets);
        let record = crate::replay::Record::new(crate::replay::RecordKind::Replay, self.assets.data_hash, self.assets.mods.record(), g.setup.clone(), ticks, g.sim.tick, title).with_profile(g);
        match record.write() {
            Ok(p) => log::info!("replay saved to {}", p.display()),
            Err(e) => {
                log::warn!("could not save replay: {e}");
                self.menu.failure = Some((ta_ui::i18n::n("REPLAY NOT KEPT"), ta_ui::i18n::tf("The battle's replay could not be saved: {error}", &[("error", &e)])));
            }
        }
    }
}


/// The interface's scale for the setting `scale` in a window of `size` pixels on a display that
/// scales by `display` ([`State::ui_zoom`]): no larger than leaves the battlefield and the menus
/// their room (`limits::GAME_POINTS`), or the pages that reflow theirs (`OPTIONS_POINTS`).
fn ui_zoom_for(scale: f32, size: (u32, u32), display: f32, reflowing: bool) -> f32 {
    use ta_config::limits;
    let display = display.max(0.5);
    let fit = |(width, height): (f32, f32)| (size.0 as f32 / display / width).min(size.1 as f32 / display / height);
    if reflowing {
        let (least, most) = (*limits::UI_SCALE.start(), *limits::UI_SCALE.end());
        return scale.clamp(least, most).min(fit(limits::OPTIONS_POINTS).max(least));
    }
    scale.clamp(0.6, 2.5).min(fit(limits::GAME_POINTS).max(0.6))
}

/// First start (level Auto): picks the graphics level for the card and applies it. Notes the
/// card either way; true when the settings changed.
fn auto_graphics(g: &mut ta_config::Graphics, info: &wgpu::AdapterInfo, pixels: u64) -> bool {
    use ta_config::Preset;
    let mut changed = g.gpu != info.name;
    g.gpu = info.name.clone();
    if g.preset == Preset::Auto {
        let kind = gpu_kind(info.device_type);
        let level = Preset::for_gpu(kind, &info.name);
        level.apply(g);
        g.preset = level;
        g.fit_render_scale(kind, &info.name, pixels);
        g.scale_fitted = true;
        log::info!("graphics: {} level for {} ({:?}), render scale {:.0}%", level.label(), info.name, info.device_type, g.render_scale * 100.0);
        changed = true;
    }
    changed
}

/// The kind of graphics a device is, as the settings know them.
fn gpu_kind(device: wgpu::DeviceType) -> ta_config::GpuKind {
    use ta_config::GpuKind;
    match device {
        wgpu::DeviceType::Cpu => GpuKind::Software,
        wgpu::DeviceType::IntegratedGpu => GpuKind::Integrated,
        wgpu::DeviceType::DiscreteGpu => GpuKind::Discrete,
        _ => GpuKind::Other,
    }
}

/// Shadows, bloom, ambient occlusion, cloud shadows, heat haze, water reflections and HD
/// surfaces from the settings (the renderer's own strengths when on).
/// Sets `camera` from a `--camera` spec: "x,z[,zoom[,yaw[,pitch]]]" (degrees).
/// `TA_FOLLOW`, read once.
fn follow_unit() -> Option<&'static str> {
    static FOLLOW: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    FOLLOW.get_or_init(|| std::env::var("TA_FOLLOW").ok()).as_deref()
}

impl State {
    /// `TA_CAPTURE_FPS=n` with `--screenshot`: each frame moves the game on 1/n of a second,
    /// however long it took to draw and save, so a strip (`TA_SHOTS`) plays back smoothly.
    fn capture_dt(&self) -> Option<f32> {
        static DT: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
        let dt = *DT.get_or_init(|| std::env::var("TA_CAPTURE_FPS").ok().and_then(|v| v.parse::<f32>().ok()).filter(|&f| f > 0.0).map(|f| 1.0 / f));
        dt.filter(|_| self.exit_after_screenshot)
    }

    /// The interface keeps the same fixed clock in such a capture (its animations, the cockpit
    /// powering up), rather than the wall clock's, and so do the renderer's animations (clouds
    /// and their shadows, waves, lava, construction): on the wall clock they ran as many times
    /// too fast as a frame took longer than 1/n s to draw and save, and jumped on slow frames.
    fn capture_time(&mut self, raw: &mut egui::RawInput) {
        if let Some(dt) = self.capture_dt() {
            self.capture_clock += dt as f64;
            raw.time = Some(self.capture_clock);
            self.renderer.set_animation_time(Some((self.capture_clock % 65536.0) as f32));
        }
    }
}

/// `TA_CAMERA_PATH="0:x,z,zoom,yaw,pitch;5:x,z,zoom,yaw,pitch"`: where the camera is at these
/// seconds of a capture's fixed clock (`TA_CAPTURE_FPS`), as `--camera` takes it (an empty field
/// keeps the camera's own: a demo's focus, `TA_FOLLOW`'s unit; a sixth lifts the focus that far
/// over the ground). Read once.
fn camera_path() -> &'static [crate::stage::CameraKey] {
    static PATH: std::sync::OnceLock<Vec<crate::stage::CameraKey>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let Ok(v) = std::env::var("TA_CAMERA_PATH") else { return Vec::new() };
        v.split(';').filter_map(crate::stage::path_key).collect()
    })
}

/// The camera at `t` seconds along `TA_CAMERA_PATH`: each stretch between two keys eased in and
/// out, held at the ends.
fn apply_camera_path(camera: &mut Camera, t: f32, world: &ta_sim::world::World) {
    let ground = |x: f32, z: f32| world.height_at(ta_sim::Fx::from_f32(x), ta_sim::Fx::from_f32(z)).max(world.sea_level).to_f32();
    crate::stage::apply_path(camera, camera_path(), t, ground);
}

/// `TA_HUD=0`: the battle without its interface, still playing (captures; photo mode's hidden
/// panel stops the battle); a staged scene's `hud` otherwise (it has none by default).
fn hud_hidden() -> bool {
    static HIDDEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HIDDEN.get_or_init(|| match std::env::var("TA_HUD") {
        Ok(v) => v == "0",
        Err(_) => crate::stage::current().is_some_and(|s| !s.hud) || crate::benchmark::running(),
    })
}

/// `TA_ICONS=0`: no strategic icons however far out the camera is (captures that show the
/// units themselves from high up); a staged scene's `icons` otherwise (none by default).
fn icons_hidden() -> bool {
    // (A benchmark's scene each its own.)
    if let Some(icons) = crate::benchmark::scene_icons() {
        return !icons;
    }
    static HIDDEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HIDDEN.get_or_init(|| match std::env::var("TA_ICONS") {
        Ok(v) => v == "0",
        Err(_) => crate::stage::current().is_some_and(|s| !s.icons),
    })
}

/// `TA_RENDER_FROM=n`: a capture's frames before n are not drawn (see `App::frame`).
fn render_from() -> u32 {
    static FROM: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *FROM.get_or_init(|| std::env::var("TA_RENDER_FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// A strip of captures (`TA_SHOTS`).
struct Shots {
    /// The path given, which the frames are numbered after.
    base: std::path::PathBuf,
    /// Frames between two.
    step: u32,
    /// How many there are, and how many are taken.
    count: u32,
    taken: u32,
}

impl Shots {
    fn from_env(base: &std::path::Path) -> Option<Shots> {
        let count: u32 = std::env::var("TA_SHOTS").ok()?.parse().ok().filter(|&n| n > 1)?;
        let step = std::env::var("TA_SHOT_STEP").ok().and_then(|v| v.parse().ok()).unwrap_or(4u32).max(1);
        Some(Shots { base: base.to_path_buf(), step, count, taken: 1 })
    }

    /// The next frame's path, while there are frames left to take.
    fn next(&mut self) -> Option<std::path::PathBuf> {
        if self.taken >= self.count {
            return None;
        }
        let stem = self.base.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let name = format!("{stem}_{}.png", self.taken);
        self.taken += 1;
        Some(self.base.with_file_name(name))
    }
}

/// Sets the camera to the player's chosen way of seeing the battle: the modern 3D camera, or
/// TA's own top-down view, which never turns or tilts (it is TA's oblique projection, the
/// maps as painted).
fn set_camera_style(camera: &mut Camera, style: ta_config::CameraStyle) {
    match style {
        ta_config::CameraStyle::Modern => camera.mode = CameraMode::Perspective,
        ta_config::CameraStyle::Classic => {
            camera.mode = CameraMode::Classic;
            camera.focus.y = 0.0;
            camera.yaw = 0.0;
            camera.pitch = ta_render::camera::TA_ELEVATION;
        }
    }
}

/// The ground the perspective camera's eye keeps above at world (x, z): the map's, beyond its
/// edges its mirror image (as the sky's extended ground shows it, with room for the gentle
/// hills it adds far out, `skirt_height` in terrain_common.wgsl), and never under the water.
fn camera_ground(field: &ta_render::terrain::HeightField, x: f32, z: f32) -> f32 {
    let size = field.size_px();
    let reflect = |v: f32, n: f32| {
        let f = (v / (2.0 * n.max(1.0))).rem_euclid(1.0) * 2.0;
        (1.0 - (f - 1.0).abs()) * n
    };
    let p = Vec2::new(x, z);
    let beyond = p.distance(p.clamp(Vec2::ZERO, size));
    let t = ((beyond - 1500.0) / 6500.0).clamp(0.0, 1.0);
    let hills = 70.0 * t * t * (3.0 - 2.0 * t);
    (field.sample(reflect(x, size.x), reflect(z, size.y)) + hills).max(field.sea_level)
}

/// The menu backdrop's camera: its pitch (radians) at the middle of its cycle, low enough to
/// show the horizon at the bottom of the cycle.
const ATTRACT_PITCH: f32 = 0.33;
/// Ticks of the menu's battle fast-forwarded each frame of the intro (see
/// `State::warm_up_attract`): its five minutes in some 300 frames, a few milliseconds each.
const INTRO_WARMUP_TICKS: u32 = 30;

/// The last column and rows of cells TA never showed (its scrolling stopped short of them).
const UNSHOWN_COLUMNS_PX: f32 = 32.0;
const UNSHOWN_ROWS_PX: f32 = 128.0;

/// Keeps the classic camera's view on the map, as TA's did: the screen rectangle `free` (pixels,
/// the part the interface leaves to the battle) shows the map's art up to its edges and no
/// further, less the column and rows TA never showed; along an axis where the map is smaller
/// than that rectangle, the map is centred in it. (The classic projection shows the art
/// pixel (x, z - y/2) at the screen centre for the focus, `zoom` screen pixels per art pixel.)
/// The part of the window the battle is seen in (pixels), as the interface leaves it free:
/// never more than the window, the whole window when there is none. (The interface's first
/// pass of a battle, before it knows the window's size, measures egui's 10,000-point stand-in
/// screen; the classic camera, finding no room in that, took the map's middle for good, the
/// Commander off-screen.)
fn free_view(free: Option<(Vec2, Vec2)>, viewport: Vec2) -> (Vec2, Vec2) {
    let whole = (Vec2::ZERO, viewport);
    let Some((lo, hi)) = free else { return whole };
    let (lo, hi) = (lo.clamp(Vec2::ZERO, viewport), hi.clamp(Vec2::ZERO, viewport));
    if lo.cmplt(hi).all() { (lo, hi) } else { whole }
}

fn clamp_classic_view(camera: &mut Camera, map_size: Vec2, free: (Vec2, Vec2)) {
    let shown = Vec2::new((map_size.x - UNSHOWN_COLUMNS_PX).max(0.0), (map_size.y - UNSHOWN_ROWS_PX).max(0.0));
    // (Where the focus is on the screen.)
    let half = camera.viewport * 0.5 + camera.view_offset;
    let zoom = camera.zoom.max(1e-3);
    // The art pixel at the screen centre, kept so that the free rectangle's edges, `free.0 -
    // half` and `free.1 - half` pixels from the centre, stay within 0..shown.
    let lo = (half - free.0) / zoom;
    let hi = shown - (free.1 - half) / zoom;
    let fit = |c: f32, lo: f32, hi: f32| if lo <= hi { c.clamp(lo, hi) } else { (lo + hi) * 0.5 };
    let art_y = camera.focus.z - camera.focus.y * 0.5;
    camera.focus.x = fit(camera.focus.x, lo.x, hi.x);
    camera.focus.z += fit(art_y, lo.y, hi.y) - art_y;
}

fn apply_camera_spec(camera: &mut Camera, spec: &str) {
    // An empty field keeps the camera's own value (",,0.9,30,32": the start, closer and lower).
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

/// The farthest the battle's camera zooms out on a `viewport` (pixels a world unit at its focus):
/// until the whole map (`map`, world units) is in view, and no further, as nothing lies beyond it
/// but the dark (`ta_render::Beyond::Clouds`); on the largest maps, as far as before
/// ([`FARTHEST_ZOOM_1080`], scaled with the window's height).
fn zoom_out_limit(viewport: Vec2, map: Option<Vec2>) -> f32 {
    let farthest = FARTHEST_ZOOM_1080 * viewport.y.max(1.0) / 1080.0;
    let whole = map.map_or(0.0, |m| (viewport.x / m.x.max(1.0)).min(viewport.y / m.y.max(1.0)));
    farthest.max(whole).min(MAX_ZOOM)
}

/// How far from the map's edges (x, z) the perspective camera's focus keeps: nowhere while the
/// view takes in less than half the map in that direction, its middle once it takes in all of it,
/// easing between, so that zoomed out the map fills the view rather than the dark beyond it. (The
/// view's extent at the focus, turned with the camera: not foreshortened by its pitch, or a view
/// towards the horizon would be held to the middle of the map.)
fn view_margins(camera: &Camera, map: Vec2) -> Vec2 {
    let half = camera.viewport * 0.5 / camera.zoom.max(1e-3);
    let (s, c) = camera.yaw.sin_cos();
    let reach = Vec2::new(c.abs() * half.x + s.abs() * half.y, s.abs() * half.x + c.abs() * half.y);
    let margin = |reach: f32, size: f32| {
        let t = ((reach - size * 0.25) / (size * 0.25).max(1e-3)).clamp(0.0, 1.0);
        size * 0.5 * t * t * (3.0 - 2.0 * t)
    };
    Vec2::new(margin(reach.x, map.x), margin(reach.y, map.y))
}

/// `TA_TIME=<hours>`: the time of day for captures, over anything else. Read once.
fn forced_time() -> Option<f32> {
    static FORCED: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| std::env::var("TA_TIME").ok().and_then(|v| v.parse().ok()))
}

/// What lies beyond the map's edges: in a `battle` a sea of dark clouds, else (the films) the
/// land carried on. `TA_BEYOND=clouds|land` sets it whatever is shown, for captures. Read once.
fn beyond_the_map(battle: bool) -> ta_render::Beyond {
    static FORCED: std::sync::OnceLock<Option<ta_render::Beyond>> = std::sync::OnceLock::new();
    let forced = *FORCED.get_or_init(|| match std::env::var("TA_BEYOND").ok().as_deref() {
        Some("clouds") => Some(ta_render::Beyond::Clouds),
        Some("land") => Some(ta_render::Beyond::Land),
        _ => None,
    });
    forced.unwrap_or(if battle { ta_render::Beyond::Clouds } else { ta_render::Beyond::Land })
}

/// The clock over the battlefield at `seconds` of game time: the setting's fixed time of day,
/// the cycle's hour (game time, so replays and rewinds see the same sky), or TA's own light.
/// `TA_TIME=<hours>` overrides it, for captures (and a staged scene's `time` the settings).
fn time_of_day(g: &ta_config::Graphics, seconds: f64) -> Option<f32> {
    if let Some(hours) = forced_time() {
        return Some(hours);
    }
    match g.time_of_day {
        ta_config::TimeOfDay::Cycle => {
            // (Where in its cycle the day is, from the f64 seconds: exact in a game of any length.)
            let cycle = ta_config::TimeOfDay::CYCLE_MINUTES as f64 * 60.0;
            Some(ta_render::daylight::cycle_hours(seconds.rem_euclid(cycle) as f32, cycle as f32))
        }
        other => other.fixed_hours(),
    }
}

/// Whether shadows and ambient occlusion are to be ray traced: the setting, or `TA_RAY_TRACING=0|1`
/// for captures.
fn ray_tracing_wanted(g: &ta_config::Graphics) -> bool {
    std::env::var("TA_RAY_TRACING").ok().map_or(g.ray_tracing, |v| v != "0")
}

/// How the classic interface's art is enlarged: the setting, or `TA_CLASSIC_PIXELS=0|1` for
/// captures (TA's own pixels, or smooth).
fn classic_look(p: &ta_config::Gameplay) -> crate::hud_classic_scale::Look {
    let pixels = std::env::var("TA_CLASSIC_PIXELS").ok().map_or(p.classic_pixels, |v| v != "0");
    if pixels { crate::hud_classic_scale::Look::Pixels } else { crate::hud_classic_scale::Look::Smooth }
}

/// Persisted art direction, with a developer override for repeatable capture comparisons (and a
/// staged scene's `style`).
fn art_style_reborn(g: &ta_config::Graphics) -> bool {
    match std::env::var("TA_ART_STYLE").as_deref() {
        Ok("reborn") => true,
        Ok("classic") => false,
        _ => match crate::stage::current().and_then(|s| s.style) {
            Some(style) => style == crate::stage::Style::Reborn,
            None => g.art_style == ta_config::ArtStyle::Reborn,
        },
    }
}

/// How much the ground moves in the wind (grass in waves, blown sand and snow; see
/// `ta_render::Lighting::ground_life`) for these settings: it rides on the ground's materials, so
/// with HD surfaces, and costs the pixel shader a little, so from the High effects up, and only
/// while the player has it on (Graphics: GROUND IN THE WIND). Reduced motion keeps it still in the
/// renderer, whatever this says.
fn ground_life_for(g: &ta_config::Graphics) -> f32 {
    if g.ground_life && g.hd_surfaces && matches!(g.effects, ta_config::Quality::High | ta_config::Quality::Ultra) { 1.0 } else { 0.0 }
}

fn apply_effect_settings(g: &ta_config::Graphics, lighting: &mut ta_render::Lighting, post: &mut ta_render::post::PostSettings) {
    lighting.shadow_strength = if g.shadows == ta_config::Quality::Low { 0.0 } else { ta_render::Lighting::default().shadow_strength };
    lighting.terrain_shadows = if g.shadows == ta_config::Quality::Low { 0.0 } else { ta_render::Lighting::default().terrain_shadows };
    // Soft, contact-hardening shadows from High up.
    lighting.soft_shadows = if matches!(g.shadows, ta_config::Quality::High | ta_config::Quality::Ultra) { ta_render::Lighting::default().soft_shadows } else { 0.0 };
    post.bloom = if g.bloom { ta_render::post::PostSettings::default().bloom } else { 0.0 };
    lighting.ambient_occlusion.enabled = g.ambient_occlusion;
    lighting.water_reflections = match g.water_reflections {
        ta_config::Reflections::Off => ta_render::WaterReflections::Off,
        ta_config::Reflections::Half => ta_render::WaterReflections::Half,
        ta_config::Reflections::Full => ta_render::WaterReflections::Full,
    };
    lighting.cloud_shadows = if g.cloud_shadows { ta_render::Lighting::default().cloud_shadows } else { 0.0 };
    post.heat_haze = if g.heat_haze { ta_render::post::PostSettings::default().heat_haze } else { 0.0 };
    lighting.hd_surfaces = if g.hd_surfaces { 1.0 } else { 0.0 };
    // `TA_GROUND_LIFE=0|1` for captures.
    lighting.ground_life = ground_life_for(g);
    if let Some(v) = std::env::var("TA_GROUND_LIFE").ok().and_then(|v| v.parse().ok()) {
        lighting.ground_life = v;
    }
    if let Some(v) = std::env::var("TA_HD_TEST").ok().and_then(|v| v.parse().ok()) {
        lighting.hd_surfaces = v;
    }
    lighting.ray_tracing = ray_tracing_wanted(g);
    if let Some(v) = std::env::var("TA_TERRAIN_SHADOWS").ok().and_then(|v| v.parse().ok()) {
        lighting.terrain_shadows = v;
    }
    if let Some(v) = std::env::var("TA_SOFT_SHADOWS").ok().and_then(|v| v.parse().ok()) {
        lighting.soft_shadows = v;
    }
    if let Some(v) = std::env::var("TA_GRADE").ok().and_then(|v| v.parse::<f32>().ok()) {
        post.grade = v;
        post.sharpen = v * ta_render::post::PostSettings::default().sharpen;
    }
}

/// Where interface languages are found, and those found: (code, name).
type Languages = (Vec<std::path::PathBuf>, Vec<(String, String)>);

/// The interface languages found when the options first open, and where they are.
fn languages() -> &'static Languages {
    static LANGUAGES: std::sync::OnceLock<Languages> = std::sync::OnceLock::new();
    LANGUAGES.get_or_init(|| {
        let dirs = ta_ui::i18n::language_dirs(&ta_config::config_dir());
        let languages = ta_ui::i18n::languages(&dirs);
        (dirs, languages)
    })
}

/// The options: the settings screen the launcher shows too, each change applied at once
/// (`changes` gathers what to apply) and everything saved to config.toml when it closes.
/// Returns true when it closes.
#[allow(clippy::too_many_arguments)]
fn options_panel(
    ctx: &egui::Context,
    view: &mut ta_ui::settings::SettingsView,
    settings: &mut ta_config::Config,
    msaa_started: u32,
    ray_tracing_running: bool,
    native: Option<(u32, u32)>,
    gpu_kind: ta_config::GpuKind,
    changes: &mut ta_ui::settings::Changes,
) -> bool {
    use crate::theme::*;
    use egui::RichText;
    use ta_ui::panel::{self, Exit, Panel};
    let (dirs, languages) = languages();
    let width = panel::fit_width(ctx, 1100.0);
    let compact = ta_ui::settings::compact(width) || ctx.content_rect().height() < 540.0;
    let host = ta_ui::settings::Host { in_game: true, msaa_running: msaa_started, ray_tracing_running, native, gpu_kind: Some(gpu_kind), languages, language_dirs: dirs, height: (!compact).then(|| panel::fit_height(ctx, 260.0, 260.0, 640.0)) };
    // (A dialog: its close cross, Escape or a click outside closes it, the settings saved.)
    let mut page = Panel::new("OPTIONS").subtitle("CHANGES TAKE EFFECT AT ONCE").width(width).exit(Exit::Close);
    if compact {
        page = page.max_height((ctx.content_rect().height() - 20.0).max(160.0));
    }
    let response = panel::modal(ctx, "options", page, true, |ui| {
        let c = view.show(ui, settings, &host);
        changes.audio |= c.audio;
        changes.graphics |= c.graphics;
        changes.display |= c.display;
        changes.camera |= c.camera;
        ui.add_space(6.0);
        panel::rule(ui);
        ui.label(RichText::new(ta_ui::i18n::t("Mods and the game folder are set in the launcher.")).font(body(12.5)).color(STEEL_DIM));
    });
    // (Closed: the caller saves the settings, and says so if that fails.)
    response.exit
}

impl State {
    /// Applies what the options changed this frame: volumes, the renderer's effects, the
    /// window, the camera, the skirmish page's unit limit.
    fn apply_settings(&mut self, before: &ta_config::Config, changes: ta_ui::settings::Changes) {
        // (Without a profile the name is the settings' alone: the Multiplayer page and the
        // battle take it at once, as they do a profile's.)
        if self.profile.is_none() && before.gameplay.player_name != self.settings.gameplay.player_name {
            self.menu.connect.name = self.settings.gameplay.player_name.clone();
            if self.game.net.is_none() {
                self.game.player_name = self.settings.gameplay.player_name.clone();
            }
        }
        // Commit commander-name edits when options closes, preserving the pinned identity.
        if !self.options_open && let Some(profile) = &self.profile
            && profile.name != self.settings.gameplay.player_name
        {
            match ta_config::profiles::ProfileStore::open(ta_config::config_dir())
                .and_then(|store| store.update(&profile.id, &self.settings.gameplay.player_name, profile.allegiance))
            {
                Ok(profile) => {
                    self.settings.gameplay.player_name = profile.name.clone();
                    self.menu.connect.name = profile.name.clone();
                    if self.game.net.is_none() { self.game.player_name = profile.name.clone(); }
                    self.profile = Some(profile);
                }
                Err(error) => {
                    self.settings.gameplay.player_name = profile.name.clone();
                    let _ = self.settings.save();
                    self.game.trouble(ta_ui::i18n::tf("Could not update profile: {error}", &[("error", &error)]));
                }
            }
        }
        // (The player's music: how it plays, or its folder, changed.)
        if changes.audio && (before.audio.user_music != self.settings.audio.user_music || before.audio.music_folder != self.settings.audio.music_folder) {
            self.apply_user_music();
        }
        // The menu's backdrop chosen afresh (or the season's look): it comes in behind the menu
        // at once.
        let backdrop_changed = before.gameplay.menu_backdrop != self.settings.gameplay.menu_backdrop
            || before.launcher.seasonal_effects != self.settings.launcher.seasonal_effects
            || before.launcher.seasonal_theme != self.settings.launcher.seasonal_theme;
        if backdrop_changed && self.screen == Screen::Menu && self.showroom.is_none() && self.pending.is_none() {
            self.menu_film_world = None;
            self.pending = Some(Transition::RestoreMenuBackdrop);
        }
        let g = &self.settings.graphics;
        if changes.audio {
            self.audio.apply_config(&self.settings.audio);
        }
        if changes.graphics {
            self.renderer.set_art_style(art_style_reborn(g));
            { let renderer = &mut *self.renderer; apply_effect_settings(g, &mut renderer.lighting, &mut renderer.post.settings); }
            self.renderer.lighting.terrain_relight = g.terrain_relight;
        }
        if changes.display {
            let b = &before.graphics;
            if g.vsync != b.vsync {
                self.renderer.gpu.set_vsync(g.vsync);
            }
            if g.window_mode != b.window_mode {
                let full = g.window_mode != ta_config::WindowMode::Windowed;
                self.window.set_fullscreen(full.then_some(winit::window::Fullscreen::Borderless(None)));
            }
            if g.window_mode == ta_config::WindowMode::Windowed && (g.width, g.height, g.window_mode) != (b.width, b.height, b.window_mode) {
                let _ = self.window.request_inner_size(PhysicalSize::new(g.width.max(640), g.height.max(480)));
            }
        }
        // (The menu's backdrop keeps the modern camera; battles take the new choice at once.)
        if changes.camera && self.screen != Screen::Menu {
            set_camera_style(&mut self.camera, g.camera);
        }
        if self.settings.gameplay.unit_limit != before.gameplay.unit_limit {
            self.menu.rules.unit_limit = self.settings.gameplay.unit_limit;
        }
    }

    /// A frame of the map editor: its tools, panels and features over the renderer (the menu's
    /// battle waits meanwhile). Leaving it goes back to the title menu.
    fn editor_frame(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        // (The editor's view is centred in the window, whatever a battle's interface left.)
        self.camera.view_offset = Vec2::ZERO;
        let now = Instant::now();
        let dt = self.capture_dt().unwrap_or_else(|| (now - self.last_frame).as_secs_f32().min(0.1));
        self.last_frame = now;
        self.dev.fps = self.dev.fps * 0.95 + (1.0 / dt.max(1e-4)) * 0.05;
        // The quit key: closes the program once nothing is unsaved (asks first otherwise).
        if self.game.keys.pressed(&self.input, "quit") && self.editor.as_mut().is_none_or(|e| e.may_quit()) {
            self.pending = Some(Transition::Quit);
            return Ok(());
        }
        let Some(mut editor) = self.editor.take() else {
            self.pending = Some(Transition::ToMenu);
            return Ok(());
        };
        let t_update = Instant::now();
        let ui_pointer = self.egui.ctx.egui_wants_pointer_input() || self.egui.ctx.is_pointer_over_egui();
        let action = editor.update(&self.input, &mut self.camera, &mut self.renderer, &mut self.assets, &mut self.settings, ui_pointer, dt);
        // `--camera` wins over the editor's framing of the map as it opens (captures).
        if self.frames < 5 && let Some(spec) = &self.camera_spec {
            apply_camera_spec(&mut self.camera, spec);
        }
        self.update_camera(dt);
        if let Some(t) = &self.renderer.terrain {
            let size = t.field.size_px();
            let cam = &mut self.camera;
            cam.focus.x = cam.focus.x.clamp(0.0, size.x);
            cam.focus.z = cam.focus.z.clamp(0.0, size.y);
            if cam.mode == CameraMode::Perspective {
                let ground = t.field.sample(cam.focus.x, cam.focus.z).max(t.field.sea_level);
                cam.focus.y += (ground - cam.focus.y) * (1.0 - (-dt * 3.0).exp());
            }
            cam.keep_above_ground(|x, z| camera_ground(&t.field, x, z), dt);
        }
        self.audio.update(&self.camera, None, dt);

        crate::theme::set_zoom(&self.egui.ctx, self.ui_zoom());
        let mut raw = self.egui.take_input(&self.window);
        self.capture_time(&mut raw);
        self.feed_pad_to_egui(&mut raw);
        self.ui_script();
        raw.events.append(&mut self.demo_ui);
        let (assets, camera) = (&self.assets, &self.camera);
        let output = self.egui.ctx.run_ui(raw, |ui| editor.ui(ui, assets, camera));
        if let Some(sound) = ta_ui::sound::take(&self.egui.ctx) {
            self.audio.play_interface(sound.event());
        }
        self.egui.state.handle_platform_output(&self.window, output.platform_output);
        let ppp = output.pixels_per_point;
        let prims = self.egui.ctx.tessellate(output.shapes, ppp);
        let mut textures_delta = output.textures_delta;
        for (id, deltas) in textures_delta.set.drain() {
            for delta in deltas {
                self.egui.renderer.update_texture(&self.renderer.gpu.device, &self.renderer.gpu.queue, id, &delta);
            }
        }
        let t_prepare = Instant::now();
        // (The editor shows the map in TA's own light and dry weather, whatever a battle left,
        // and no units.)
        self.renderer.lighting.time_of_day = None;
        self.renderer.lighting.overcast = 0.0;
        self.renderer.lighting.wetness = 0.0;
        self.renderer.lighting.rain = 0.0;
        self.renderer.set_icons(&[]);
        editor.prepare(&mut self.renderer, &self.camera, dt);
        if self.screenshot_path.is_some() && self.frames == self.screenshot_frame {
            self.renderer.request_screenshot();
        }
        let t_render = Instant::now();
        let size = self.window.inner_size();
        let egui_renderer = &mut self.egui.renderer;
        self.renderer.render(&self.camera, |device, queue, encoder, view| {
            let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [size.width, size.height], pixels_per_point: ppp };
            // (Not when the card refuses its staging memory, as egui-wgpu panics then: this frame
            // goes without the interface.)
            if ta_render::gpu::unless_short_of_memory(|| egui_renderer.update_buffers(device, queue, encoder, &prims, &sd)).is_none() {
                return;
            }
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            egui_renderer.render(&mut pass, &prims, &sd);
        });
        let t_end = Instant::now();
        self.dev.times.push(dt * 1000.0, (t_prepare - t_update).as_secs_f32() * 1000.0, (t_render - t_prepare).as_secs_f32() * 1000.0, (t_end - t_render).as_secs_f32() * 1000.0);
        for id in textures_delta.free.drain() {
            self.egui.renderer.free_texture(&id);
        }
        if self.game.keys.pressed(&self.input, "screenshot") && self.screenshot_path.is_none() {
            self.renderer.request_screenshot();
            self.screenshot_path = Some(ta_config::next_picture(&ta_config::pictures_dir(), "Map editor"));
            self.screenshot_frame = self.frames + 1;
        }
        if let Some(shot) = self.renderer.take_screenshot() {
            if let Some(path) = self.screenshot_path.take() {
                self.save_screenshot(&path, &shot)?;
            }
            if self.exit_after_screenshot {
                println!("map editor, {}", self.dev.times.summary());
                event_loop.exit();
            }
        }
        match action {
            crate::editor::EditorAction::Exit => self.pending = Some(Transition::ToMenu),
            crate::editor::EditorAction::Quit => self.pending = Some(Transition::Quit),
            crate::editor::EditorAction::Saved(name) => {
                crate::map_preview::forget(&self.egui.ctx, &name);
                self.editor = Some(editor);
            }
            crate::editor::EditorAction::None => self.editor = Some(editor),
            crate::editor::EditorAction::TestPlay(name) => {
                crate::map_preview::forget(&self.egui.ctx, &name);
                // (The editor waits as it is, its undo history and all, for the battle to end.)
                self.test_editor = Some(editor);
                self.pending = Some(Transition::StartGame(GameSetup::duel(&name)));
            }
        }
        self.frames += 1;
        self.input.end_frame();
        Ok(())
    }
}

impl State {
    /// A frame of a space scene (`--open space`): the scene at its clock's time, nothing over it.
    /// The clock is the capture's (`TA_CAPTURE_FPS`: frame n at n / fps seconds) or the wall's;
    /// by hand, Escape goes to the title menu, Space pauses, the arrow keys step a second, Home
    /// starts again, and the scene is read again when its file changes.
    fn space_frame(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let now = Instant::now();
        let dt = self.capture_dt().unwrap_or_else(|| (now - self.last_frame).as_secs_f32().min(0.1));
        self.last_frame = now;
        self.dev.fps = self.dev.fps * 0.95 + (1.0 / dt.max(1e-4)) * 0.05;
        if self.input.pressed(KeyCode::Escape) || self.space.is_none() {
            self.space = None;
            let _ = self.renderer.set_space(None);
            self.screen = Screen::Menu;
            self.audio.set_mood(ta_audio::Mood::Briefing);
            self.input.end_frame();
            return Ok(());
        }
        let capturing = self.exit_after_screenshot;
        let t_update = Instant::now();
        let frame = {
            let Some(view) = self.space.as_mut() else { return Ok(()) };
            if !capturing {
                if self.input.pressed(KeyCode::Space) {
                    view.paused = !view.paused;
                }
                if self.input.pressed(KeyCode::ArrowLeft) {
                    view.time = (view.time - 1.0).max(0.0);
                }
                if self.input.pressed(KeyCode::ArrowRight) {
                    view.time += 1.0;
                }
                if self.input.pressed(KeyCode::Home) {
                    view.time = 0.0;
                }
                view.reload(&mut self.renderer);
            }
            view.scene.frame(view.time)
        };
        if self.screenshot_path.is_some() && self.frames == self.screenshot_frame {
            self.renderer.request_screenshot();
        }
        let t_render = Instant::now();
        self.renderer.render_space(&frame, |_, _, _, _| {});
        let t_end = Instant::now();
        self.dev.times.push(dt * 1000.0, (t_render - t_update).as_secs_f32() * 1000.0, 0.0, (t_end - t_render).as_secs_f32() * 1000.0);
        if let Some(view) = self.space.as_mut()
            && !view.paused
        {
            view.time += dt;
        }
        // F12: the picture as it is.
        if self.game.keys.pressed(&self.input, "screenshot") && self.screenshot_path.is_none() {
            self.renderer.request_screenshot();
            self.screenshot_path = Some(ta_config::next_picture(&ta_config::pictures_dir(), "Space"));
            self.screenshot_frame = self.frames + 1;
        }
        if let Some(shot) = self.renderer.take_screenshot() {
            if let Some(path) = self.screenshot_path.take() {
                self.save_screenshot(&path, &shot)?;
            }
            if let Some(next) = self.shots.as_mut().and_then(|s| s.next()) {
                self.screenshot_path = Some(next);
                self.screenshot_frame = self.frames + self.shots.as_ref().map_or(1, |s| s.step);
            } else if self.exit_after_screenshot {
                println!("space, {}", self.dev.times.summary());
                event_loop.exit();
            }
        }
        self.frames += 1;
        self.input.end_frame();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zooming out stops once the whole map is in view (nothing beyond it but the dark), on a
    /// big map as far as before, and a larger window takes in no more of the map than a smaller.
    #[test]
    fn the_battle_camera_zooms_out_no_further_than_the_map() {
        let hd = Vec2::new(1920.0, 1080.0);
        // A 10 by 10 map, square: its height fills the window's at the limit.
        let small = Vec2::new(5120.0, 5120.0);
        let limit = zoom_out_limit(hd, Some(small));
        assert!((hd.y / limit - small.y).abs() < 1.0, "the whole map in view at {limit}");
        // Shore to Shore Reborn (48 by 20): as far as the old limit, on any window.
        let wide = Vec2::new(24576.0, 10240.0);
        assert_eq!(zoom_out_limit(hd, Some(wide)), FARTHEST_ZOOM_1080);
        assert!((zoom_out_limit(Vec2::new(3840.0, 2160.0), Some(wide)) - 2.0 * FARTHEST_ZOOM_1080).abs() < 1e-6, "4K sees as much as 1080p");
        assert_eq!(zoom_out_limit(hd, None), FARTHEST_ZOOM_1080);
    }

    /// Zoomed in, the focus goes anywhere on the map; zoomed out over all of it, to its middle;
    /// turned a quarter round, the map's sides swap with the window's.
    #[test]
    fn zoomed_out_the_view_keeps_to_the_map() {
        let map = Vec2::new(5120.0, 5120.0);
        let mut camera = Camera::new(Vec2::new(1920.0, 1080.0));
        camera.zoom = 1.5;
        assert_eq!(view_margins(&camera, map), Vec2::ZERO);
        camera.zoom = zoom_out_limit(camera.viewport, Some(map));
        let at_limit = view_margins(&camera, map);
        assert_eq!(at_limit, map * 0.5, "the map's middle, both ways (the window is wider than high)");
        // Halfway out: some of the way in from the edges.
        camera.zoom = camera.viewport.y / (map.y * 0.75);
        let halfway = view_margins(&camera, map);
        assert!(halfway.y > 0.0 && halfway.y < map.y * 0.5, "{halfway}");
        camera.yaw = std::f32::consts::FRAC_PI_2;
        let turned = view_margins(&camera, map);
        assert!((turned.x - halfway.y).abs() < 1.0, "{turned} against {halfway}");
    }

    /// The commander's card at the top right shows on the title screen alone: every other page
    /// has its way back there (BACK, Conquest's MAIN MENU), which the card covered and whose
    /// clicks it took at 1920 × 1080.
    #[test]
    fn the_identity_card_keeps_off_the_pages_ways_back() {
        use crate::menu::Page;
        assert!(shows_identity_card(Page::Title, false, 0.0));
        for page in [Page::Tutorial, Page::Conquest, Page::Skirmish, Page::Scenarios, Page::Campaign, Page::Multiplayer, Page::Lobby, Page::Records, Page::Credits, Page::Units, Page::Timeline] {
            assert!(!shows_identity_card(page, false, 0.0), "{page:?}");
        }
        assert!(!shows_identity_card(Page::Title, true, 0.0), "under the options");
        assert!(!shows_identity_card(Page::Title, false, 1.0), "the title's film watched");
    }

    /// After a mission the campaign page opens no further than the campaign has got (a victory
    /// with cheats records nothing: its CONTINUE CAMPAIGN selected the locked mission after it),
    /// on the difficulty the mission was played at.
    #[test]
    fn the_campaign_page_reopens_on_an_unlocked_mission_at_the_difficulty_played() {
        let mut progress = ta_config::CampaignProgress { difficulty: "Medium".into(), ..Default::default() };
        progress.complete("arm campaign", 17);
        let played = |index: usize| crate::setup::MissionStart { campaign: "arm campaign".into(), index, file: "AC20.ota".into(), difficulty: "Hard".into() };
        // 18 missions done; the 20th (index 19, still locked) won with cheats, which records
        // nothing: CONTINUE CAMPAIGN asks for index 20, and gets the 19th, the one unlocked.
        assert_eq!(campaign_return(&progress, Some(&played(19)), "arm campaign", 20), (18, "Hard".to_string()));
        // The 19th won for real (recorded as it ended): on to the 20th.
        progress.complete("arm campaign", 18);
        assert_eq!(campaign_return(&progress, Some(&played(18)), "arm campaign", 19).0, 19);
        // BACK TO BRIEFING after a defeat: that mission.
        assert_eq!(campaign_return(&progress, Some(&played(5)), "arm campaign", 5).0, 5);
        // No mission played this session: the difficulty last chosen.
        assert_eq!(campaign_return(&progress, None, "core campaign", 3), (0, "Medium".to_string()));
    }

    #[test]
    fn the_ground_moves_in_the_wind_from_the_high_graphics_levels_up() {
        let at = |preset: ta_config::Preset| {
            let mut g = ta_config::Graphics::default();
            preset.apply(&mut g);
            g
        };
        assert_eq!(ground_life_for(&at(ta_config::Preset::Low)), 0.0);
        assert_eq!(ground_life_for(&at(ta_config::Preset::Medium)), 0.0);
        assert_eq!(ground_life_for(&at(ta_config::Preset::High)), 1.0);
        assert_eq!(ground_life_for(&at(ta_config::Preset::Ultra)), 1.0);
        // It rides on the ground's materials: without HD surfaces it stays still at any level.
        let mut g = at(ta_config::Preset::Ultra);
        g.hd_surfaces = false;
        assert_eq!(ground_life_for(&g), 0.0);
        // And a High effects setting alone is enough with them.
        let mut g = at(ta_config::Preset::Low);
        g.hd_surfaces = true;
        g.effects = ta_config::Quality::High;
        assert_eq!(ground_life_for(&g), 1.0);
        // The player's GROUND IN THE WIND off keeps it still at every level.
        for preset in ta_config::Preset::LEVELS {
            let mut g = at(preset);
            g.ground_life = false;
            assert_eq!(ground_life_for(&g), 0.0, "{preset:?}");
        }
    }

    /// The frame rate counter shows a figure only once half a second has been counted, and then
    /// the frames over that time, not the last frame's.
    #[test]
    fn the_frame_rate_counter_counts_over_half_a_second() {
        let mut meter = FrameMeter::default();
        for _ in 0..29 {
            meter.tick(16.0);
        }
        assert_eq!(meter.shown, None, "not yet half a second");
        meter.tick(36.0);
        let (fps, ms) = meter.shown.unwrap();
        assert!((fps - 30.0 / 0.5).abs() < 0.01 && (ms - 500.0 / 30.0).abs() < 0.01, "{fps} {ms}");
        // A frame that hangs counts as a long one, not as forever.
        meter.tick(f32::INFINITY);
        assert!(meter.shown.unwrap().0 >= 0.99);
    }

    /// The user's tutorial bug: lesson 2's instructions, and the deck and minimap with them,
    /// did not show (the guide's arrow did) after the cockpit view was tried in lesson 1.
    #[test]
    fn panels_up_before_a_loading_screen_show_after_it() {
        // How opaque the panel is drawn in a pass at this time on the input state's clock.
        let pass = |ctx: &egui::Context, time: f64, clock: f64, shown: bool| {
            let mut opacity = None;
            let raw = egui::RawInput { time: Some(time + clock), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))), ..Default::default() };
            ctx.run_ui(raw, |ui| {
                if shown {
                    egui::Area::new(egui::Id::new("tutorial")).fixed_pos(egui::pos2(16.0, 50.0)).show(ui.ctx(), |ui| {
                        opacity = Some(ui.opacity());
                        ui.label("LESSON 2");
                    });
                }
            }).drop_without_applying_deltas();
            opacity.unwrap_or(0.0)
        };
        for carried in [false, true] {
            let ctx = egui::Context::default();
            // Lesson 1: up from the start, gone a moment at two minutes (the cockpit view)
            // and back, up to the lesson's end.
            for (t, shown) in [(0.0, true), (1.0, true), (120.0, false), (121.0, true), (150.0, true)] {
                pass(&ctx, t, 0.0, shown);
            }
            assert_eq!(pass(&ctx, 151.0, 0.0, true), 1.0);
            // The loading screen (a context of its own), then lesson 2 on a rebuilt input
            // state, its clock from zero.
            let clock = if carried { carry_clock(&ctx) } else {
                ctx.input_mut(|input| *input = egui::InputState::default());
                0.0
            };
            let opacity = (1..=5).map(|frame| pass(&ctx, frame as f64 / 60.0 + 0.5, clock, true)).last().unwrap();
            if carried {
                assert_eq!(opacity, 1.0, "the lesson's panel shows half a second in");
            } else {
                assert_eq!(opacity, 0.0, "(without the clock carried on: see-through until 121 s in, as reported)");
            }
        }
    }

    /// A battle's first frame in a small window: laid out for the window, not for the 10,000
    /// points square a state made new has for a size (which the zoom change waiting at that
    /// frame took for the window's: the menu's zoom is not the battle's under 1,024 x 600 points).
    #[test]
    fn the_first_pass_after_a_loading_screen_is_laid_out_for_the_window() {
        // 800 x 600 pixels on a display at 150%, as the window's input gives each pass.
        let (pixels, display) = (egui::vec2(800.0, 600.0), 1.5);
        let pass = |ctx: &egui::Context, time: f64| {
            let mut raw = egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, pixels / (display * ctx.zoom_factor()))),
                max_texture_side: Some(16384),
                ..Default::default()
            };
            raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(display);
            let mut seen = (egui::Vec2::ZERO, 0);
            ctx.run_ui(raw, |ui| seen = (ui.ctx().content_rect().size() * ui.ctx().pixels_per_point(), ui.ctx().input(|i| i.max_texture_side))).drop_without_applying_deltas();
            seen
        };
        let ctx = egui::Context::default();
        // The menu, at its own zoom.
        crate::theme::set_zoom(&ctx, 0.75);
        for frame in 0..3 {
            assert!((pass(&ctx, frame as f64 / 60.0).0 - pixels).length() < 1.0);
        }
        // The loading screen (a context of its own), then the battle at the zoom that fits it.
        let clock = carry_clock(&ctx);
        assert_eq!(ctx.input(|i| (i.content_rect().size(), i.max_texture_side)), (pixels / (display * 0.75), 16384), "the window's size and the card's limit are kept");
        assert!(ctx.input(|i| i.pointer.latest_pos().is_none() && i.keys_down.is_empty() && i.events.is_empty()), "and nothing held or queued");
        crate::theme::set_zoom(&ctx, 0.6);
        for frame in 0..3 {
            let (laid_out, limit) = pass(&ctx, clock + (frame + 1) as f64 / 60.0);
            assert!((laid_out - pixels).length() < 1.0, "battle frame {frame} is laid out for {laid_out:?} pixels, the window has {pixels:?}");
            assert_eq!(limit, 16384);
        }
        assert_eq!(ctx.zoom_factor(), 0.6);
    }

    #[test]
    fn menu_returns_disconnect_peers_before_background_work() {
        use std::time::Duration;
        // Exercise real peer notifications, without a renderer or original game data.
        // Leaving any menu destination must not keep a peer alive without polling it
        // throughout scene creation; ordinary game loading must retain its session.
        for transition in [Transition::ToMenu, Transition::ToConquest, Transition::ToScenarios,
            Transition::ToCampaign { campaign: "arm campaign".into(), mission: 1 }, Transition::RestoreMenuBackdrop]
        {
            let mut host = ta_net::Host::bind(0, 42).expect("loopback host");
            let address = ta_net::resolve(&format!("127.0.0.1:{}", host.port)).unwrap();
            let mut client = ta_net::Client::connect(address, "Returning player", 42).expect("loopback client");
            let deadline = Instant::now() + Duration::from_secs(5);
            while client.peer.is_none() && Instant::now() < deadline {
                host.poll();
                client.poll();
                std::thread::sleep(Duration::from_millis(2));
            }
            let peer = client.peer.expect("welcomed client");
            let mut connection = Some(ta_net::Session::client(client, 1));
            Transition::StartGame(GameSetup::duel("fixture")).leave_network_for_menu(&mut connection);
            assert!(connection.is_some(), "starting a battle must retain its network session");
            transition.leave_network_for_menu(&mut connection);
            assert!(connection.is_none(), "menu warm-up must own no network session");
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut departed = false;
            while !departed && Instant::now() < deadline {
                departed = host.poll().into_iter().any(|event| matches!(event, ta_net::HostEvent::Left(id, _) if id == peer));
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(departed, "the host must see departure without waiting for backdrop warm-up");
        }
    }

    /// Where the free part of the screen shows art pixel `p` of a classic camera's view.
    fn screen_of(camera: &Camera, p: Vec2) -> Vec2 {
        let focus = Vec2::new(camera.focus.x, camera.focus.z - camera.focus.y * 0.5);
        camera.viewport * 0.5 + camera.view_offset + (p - focus) * camera.zoom
    }

    #[test]
    fn the_classic_view_stays_on_the_map() {
        let mut camera = Camera::new(Vec2::new(1920.0, 1080.0));
        camera.mode = CameraMode::Classic;
        camera.zoom = 1.0;
        let map = Vec2::new(4096.0, 3072.0);
        // The interface covers the left 240 pixels and the top 34.
        let free = (Vec2::new(240.0, 34.0), Vec2::new(1920.0, 1080.0));
        // Scrolled past the north-west corner: the free part starts at the map's corner.
        camera.focus = Vec3::new(-500.0, 0.0, -800.0);
        clamp_classic_view(&mut camera, map, free);
        assert!((screen_of(&camera, Vec2::ZERO) - free.0).length() < 1e-3, "{:?}", camera.focus);
        // Past the south-east corner: it ends at the last column and row TA showed.
        camera.focus = Vec3::new(9000.0, 0.0, 9000.0);
        clamp_classic_view(&mut camera, map, free);
        let last = map - Vec2::new(UNSHOWN_COLUMNS_PX, UNSHOWN_ROWS_PX);
        assert!((screen_of(&camera, last) - free.1).length() < 1e-3, "{:?}", camera.focus);
        // Inside the map nothing moves.
        camera.focus = Vec3::new(2000.0, 0.0, 1500.0);
        clamp_classic_view(&mut camera, map, free);
        assert_eq!(camera.focus, Vec3::new(2000.0, 0.0, 1500.0));
        // A map narrower than the view is centred in the free part.
        camera.zoom = 0.25;
        clamp_classic_view(&mut camera, map, free);
        let middle = screen_of(&camera, Vec2::new(last.x * 0.5, 0.0)).x;
        assert!((middle - (free.0.x + free.1.x) * 0.5).abs() < 1e-3, "{middle}");
    }

    #[test]
    fn a_battle_starts_on_the_commander_with_the_classic_camera() {
        // The interface's first pass measures egui's stand-in screen, 10,000 points a side.
        let viewport = Vec2::new(1920.0, 1080.0);
        let stand_in = Some((Vec2::new(0.0, 34.0), Vec2::new(10000.0, 9804.0)));
        assert_eq!(free_view(stand_in, viewport), (Vec2::new(0.0, 34.0), viewport));
        assert_eq!(free_view(None, viewport), (Vec2::ZERO, viewport));
        assert_eq!(free_view(Some((Vec2::new(500.0, 0.0), Vec2::new(400.0, 900.0))), viewport), (Vec2::ZERO, viewport), "an empty rectangle");
        // Coast To Coast, the Commander near its north-east corner: the view stays on him.
        let mut camera = Camera::new(viewport);
        camera.mode = CameraMode::Classic;
        camera.zoom = 1.6;
        let map = Vec2::new(3360.0, 2016.0);
        let commander = Vec3::new(2768.0, 0.0, 236.0);
        camera.focus = commander;
        let free = free_view(stand_in, viewport);
        camera.view_offset = (free.0 + free.1) * 0.5 - viewport * 0.5;
        clamp_classic_view(&mut camera, map, free);
        let seen = screen_of(&camera, Vec2::new(commander.x, commander.z));
        assert!(seen.cmpge(free.0).all() && seen.cmple(free.1).all(), "the Commander at {seen:?}, the view at {:?}", camera.focus);
    }

    /// A Steam Deck's first start, as the game goes on from it: set up under SteamOS (the level
    /// left Auto), then the Deck's graphics opened as Vulkan names them: Medium, all its 1280 ×
    /// 800 pixels drawn. The interface at the 125% set, the most the battlefield's room allows
    /// there, a larger setting held to it; under a desktop's own 125% the same size on screen.
    #[test]
    fn a_steam_deck_plays_at_medium_with_its_interface_as_large_as_the_battle_allows() {
        use ta_config::hardware::{Handheld, Hardware};
        let deck = Hardware { os: "linux", handheld: Some(Handheld::SteamDeck), threads: 8, ram_mb: 14_830, ..Default::default() };
        let mut config = ta_config::Config::default();
        config.set_up_for(&deck);
        assert_eq!(config.graphics.preset, ta_config::Preset::Auto, "left to the game");
        let mut info = wgpu::AdapterInfo::new(wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Vulkan);
        info.name = "AMD Custom GPU 0405 (RADV VANGOGH)".into();
        // As `State::begin` goes: the render scale fitted once, then the level picked.
        let mut g = config.graphics.clone();
        let pixels = g.picture_pixels(Some((1280, 800)));
        assert!(g.fit_render_scale_once(gpu_kind(info.device_type), &info.name, pixels));
        assert!(auto_graphics(&mut g, &info, pixels));
        assert_eq!((g.preset, g.render_scale, g.gpu.as_str()), (ta_config::Preset::Medium, 1.0, info.name.as_str()));
        assert!(!auto_graphics(&mut g, &info, pixels), "once");
        let scale = config.gameplay.ui_scale;
        assert_eq!(scale, 1.25);
        assert_eq!(ui_zoom_for(scale, (1280, 800), 1.0, false), 1.25);
        assert_eq!(ui_zoom_for(scale, (1280, 800), 1.0, true), 1.25, "the options and Conquest");
        assert_eq!(ui_zoom_for(1.5, (1280, 800), 1.0, false), 1.25);
        assert_eq!(ui_zoom_for(scale, (1280, 800), 1.25, false) * 1.25, 1.25);
    }
}

/// A key by its name (`TA_UI_KEYS`): Escape, Enter, Space, Tab, Home, End, F1-F12, A-Z, 0-9.
fn key_named(name: &str) -> Option<KeyCode> {
    let fkeys = [KeyCode::F1, KeyCode::F2, KeyCode::F3, KeyCode::F4, KeyCode::F5, KeyCode::F6, KeyCode::F7, KeyCode::F8, KeyCode::F9, KeyCode::F10, KeyCode::F11, KeyCode::F12];
    let letters = [KeyCode::KeyA, KeyCode::KeyB, KeyCode::KeyC, KeyCode::KeyD, KeyCode::KeyE, KeyCode::KeyF, KeyCode::KeyG, KeyCode::KeyH, KeyCode::KeyI, KeyCode::KeyJ,
        KeyCode::KeyK, KeyCode::KeyL, KeyCode::KeyM, KeyCode::KeyN, KeyCode::KeyO, KeyCode::KeyP, KeyCode::KeyQ, KeyCode::KeyR, KeyCode::KeyS, KeyCode::KeyT,
        KeyCode::KeyU, KeyCode::KeyV, KeyCode::KeyW, KeyCode::KeyX, KeyCode::KeyY, KeyCode::KeyZ];
    let digits = [KeyCode::Digit0, KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9];
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "escape" | "esc" => Some(KeyCode::Escape),
        "enter" | "return" => Some(KeyCode::Enter),
        "space" => Some(KeyCode::Space),
        "tab" => Some(KeyCode::Tab),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        _ if lower.len() > 1 && lower.starts_with('f') => lower[1..].parse::<usize>().ok().and_then(|n| fkeys.get(n.checked_sub(1)?).copied()),
        _ if lower.len() == 1 => {
            let c = lower.as_bytes()[0];
            if c.is_ascii_lowercase() { letters.get((c - b'a') as usize).copied() } else if c.is_ascii_digit() { digits.get((c - b'0') as usize).copied() } else { None }
        }
        _ => None,
    }
}
