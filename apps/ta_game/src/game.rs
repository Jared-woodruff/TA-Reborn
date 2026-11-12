//! Client-side game controller: owns the simulation, turns input into commands, runs the
//! fixed 30 Hz tick loop and exposes state to the renderer and HUD.

use anyhow::{Context, Result};
use glam::{Mat4, Vec2, Vec3, Vec4Swizzles};
use ta_data::MapInfo;
use ta_formats::tdf;
use ta_formats::tnt::TntMap;
use ta_render::{Camera, DrawInstance, Renderer};
use ta_sim::{Command, Fx, FxVec3, Order, PlayerSetup, Seen, Sim, SimEvent, Unit, UnitId};
use winit::keyboard::KeyCode;

use crate::input::{Input, MouseButtonId};
use crate::scene::{Scene, piece_matrices};
use crate::setup::{Assets, Controller, GameSetup};

pub const TICK_SECONDS: f32 = 1.0 / 30.0;
const DRAG_THRESHOLD: f32 = 6.0;

/// Index thrown model pieces once per tick, including the unit generation so a reused
/// simulation slot cannot contribute debris to a different unit's remains.
fn thrown_pieces_by_unit(events: &[SimEvent]) -> rustc_hash::FxHashMap<UnitId, Vec<usize>> {
    let mut thrown = rustc_hash::FxHashMap::<UnitId, Vec<usize>>::default();
    for event in events {
        if let SimEvent::PieceExplode { unit, model_piece, flags, .. } = event && flags & 32 == 0 {
            thrown.entry(*unit).or_default().push(*model_piece);
        }
    }
    thrown
}

/// The wreck (or heap) each unit that died in these events left, a feature's index.
fn wrecks_by_unit(events: &[SimEvent]) -> rustc_hash::FxHashMap<UnitId, usize> {
    events
        .iter()
        .filter_map(|event| match event {
            SimEvent::FeatureCreated { index, wreck_of: Some(unit) } => Some((*unit, *index)),
            _ => None,
        })
        .collect()
}

/// A building being placed with the mouse.
pub struct Placement {
    pub type_id: usize,
    pub pos: Option<Vec3>,
    pub valid: bool,
    pub matrices: Vec<Mat4>,
    /// Dragging out a row: where the drag began, the sites along it (and whether each is
    /// buildable), and a ghost's matrices per site.
    pub line_start: Option<Vec3>,
    pub row: Vec<(Vec3, bool)>,
    row_matrices: Vec<Vec<Mat4>>,
}

impl Placement {
    fn cancel_drag(&mut self) {
        self.line_start = None;
        self.row.clear();
        self.row_matrices.clear();
        self.pos = None;
        self.valid = false;
        self.matrices.clear();
    }
}

/// Most buildings one drag can queue.
const ROW_MAX: usize = 40;
/// Most queued buildings shown as ghosts for the selected constructors.
const QUEUED_GHOSTS_MAX: usize = 120;

/// A pending order type chosen with a hotkey/button; the next left click supplies its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandMode {
    Attack,
    DGun,
    Fight,
    Patrol,
    Guard,
    Capture,
    Unload,
    Resurrect,
    Reclaim,
    /// TA's MOVE, REPAIR and LOAD buttons (the classic interface; a right-click does these
    /// by context).
    Move,
    Repair,
    Load,
    /// Extractors on every metal spot of an area (drag a circle).
    AreaMex,
}

impl CommandMode {
    /// Orders that also work on an area: drag a circle to queue every target inside.
    pub fn takes_area(self) -> bool {
        matches!(self, CommandMode::Attack | CommandMode::Capture | CommandMode::Resurrect | CommandMode::Reclaim | CommandMode::AreaMex)
    }

    pub fn label(self) -> &'static str {
        match self {
            CommandMode::Attack => ta_ui::i18n::n("Attack"),
            CommandMode::DGun => ta_ui::i18n::n("Manual fire"),
            CommandMode::Fight => ta_ui::i18n::n("Fight"),
            CommandMode::Patrol => ta_ui::i18n::n("Patrol"),
            CommandMode::Guard => ta_ui::i18n::n("Guard"),
            CommandMode::Capture => ta_ui::i18n::n("Capture"),
            CommandMode::Unload => ta_ui::i18n::n("Unload"),
            CommandMode::Resurrect => ta_ui::i18n::n("Resurrect"),
            CommandMode::Reclaim => ta_ui::i18n::n("Reclaim"),
            CommandMode::Move => ta_ui::i18n::n("Move"),
            CommandMode::Repair => ta_ui::i18n::n("Repair"),
            CommandMode::Load => ta_ui::i18n::n("Load"),
            CommandMode::AreaMex => ta_ui::i18n::n("Extractors in an area"),
        }
    }

    /// The special-fire command names a disintegrator only when one is selected.
    pub fn label_for(self, game: &Game) -> &'static str {
        if self.is_disintegrator(game) {
            ta_ui::i18n::n("D-Gun")
        } else {
            self.label()
        }
    }

    pub fn hint_for(self, game: &Game) -> &'static str {
        if self.is_disintegrator(game) {
            ta_ui::i18n::n("the Commander's disintegrator: it destroys whatever it touches, yours too")
        } else {
            self.hint()
        }
    }

    fn is_disintegrator(self, game: &Game) -> bool {
        self == CommandMode::DGun && game.selection.iter().filter_map(|&id| game.sim.unit(id)).any(|u| {
            game.sim.unit_type(u).weapons.iter().flatten().any(|&w| game.sim.weapon_types.get(w).def.render_type == ta_data::RenderType::Disintegrator)
        })
    }

    /// What the order does, for its button's tooltip.
    pub fn hint(self) -> &'static str {
        use ta_ui::i18n::n;
        match self {
            CommandMode::Attack => n("click an enemy to destroy it, or the ground to shell it; drag a circle for everything in it"),
            CommandMode::DGun => n("fire the selected unit's special weapon at the target"),
            CommandMode::Fight => n("move there, fighting whatever is met on the way"),
            CommandMode::Patrol => n("go back and forth; constructors repair and reclaim along the way"),
            CommandMode::Guard => n("follow and protect a unit, or help it build"),
            CommandMode::Capture => n("take an enemy unit over; it takes a while"),
            CommandMode::Unload => n("set the cargo down there"),
            CommandMode::Resurrect => n("raise a wreck as the unit it was"),
            CommandMode::Reclaim => n("turn wrecks, rocks and trees into metal and energy; drag a circle for all of them"),
            CommandMode::Move => n("go there"),
            CommandMode::Repair => n("mend a damaged unit"),
            CommandMode::Load => n("pick a unit up"),
            CommandMode::AreaMex => n("drag a circle; a click takes the metal around the cursor"),
        }
    }
}

/// Where an order was just given, for the HUD's acknowledgement: a ring closing in on the
/// ground, or a bracket pulsing round the unit it was given against.
#[derive(Clone, Copy, Debug)]
pub struct OrderMark {
    pub pos: Vec3,
    pub unit: Option<UnitId>,
    pub kind: MarkKind,
    /// A smaller mark (one spot of a formation).
    pub small: bool,
    pub born: std::time::Instant,
}

/// What kind of order a mark acknowledges (its colour).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    Move,
    Fight,
    Patrol,
    Attack,
    /// Repair, guard, load: helping one of ours.
    Help,
    Reclaim,
    Unload,
}

impl MarkKind {
    fn of(order: &Order) -> Option<MarkKind> {
        Some(match order {
            Order::Move(_) => MarkKind::Move,
            Order::Fight(_) => MarkKind::Fight,
            Order::Patrol(_) => MarkKind::Patrol,
            Order::Attack(_) | Order::ManualFire(_) | Order::Capture(_) | Order::AttackType(_) => MarkKind::Attack,
            Order::Repair(_) | Order::Guard(_) | Order::Load(_) => MarkKind::Help,
            Order::Reclaim(_) | Order::ReclaimUnit(_) | Order::Resurrect(_) => MarkKind::Reclaim,
            Order::Unload(_) => MarkKind::Unload,
            _ => return None,
        })
    }
}

/// Seconds an order mark lasts.
pub const MARK_LIFE: f32 = 0.7;

/// Recorded orders being applied: a replay (spectated) or a saved game catching up.
pub struct Playback {
    pub ticks: std::collections::VecDeque<(u64, Vec<Command>)>,
    pub end_tick: u64,
    /// The record already includes AI orders. Their brains still advance for save resumes.
    pub full_commands: bool,
    /// A saved game: once caught up, play continues live.
    pub resume: bool,
    /// Replays: the whole record, to start over from when seeking backwards.
    pub source: Option<std::sync::Arc<crate::replay::Record>>,
    /// Replays: fast-forwarding to this tick (behind a progress bar) before playing on.
    pub seek_to: Option<u64>,
    /// A saved game: the constructors that were on duty (`Record::duties`), put back once it has
    /// caught up.
    pub duties: Vec<u8>,
}

impl Playback {
    fn finished(&self, tick: u64) -> bool {
        !self.resume && tick >= self.end_tick
    }
}

/// [`Game::keeps_replay`], from what decides it: the battle's seat of ours (`seat`, the setup's
/// local player, which watching on after a defeat leaves as it was), whether it is a multiplayer
/// battle (`networked`: one a spectator watched from its start is a real battle all the same,
/// every tick of it received as the players received it), whether it is a record being played
/// back, a lesson, and how long it has run. Whether the player is watching is no part of it.
fn keeps_replay(seat: Option<u8>, networked: bool, playback: bool, tutorial: bool, tick: u64) -> bool {
    (seat.is_some() || networked) && !playback && !tutorial && tick >= 30 * 20
}

/// Choose the historical stream before recording it again. Older files contain only
/// human orders; new ones also keep the AI's decisions so an AI update cannot rewrite
/// the battle. Always consume the AI iterator: a saved game needs its brains warmed up
/// even when their newly generated orders are replaced by historical ones.
fn commands_for_tick(playback: Option<&mut Playback>, tick: u64, live: Vec<Command>, ai_orders: impl IntoIterator<Item = Command>) -> Vec<Command> {
    let full_commands = playback.as_ref().is_some_and(|p| p.full_commands);
    let mut commands = match playback {
        Some(pb) if pb.ticks.front().is_some_and(|(t, _)| *t == tick) => pb.ticks.pop_front().map(|(_, c)| c).unwrap_or_default(),
        Some(_) => Vec::new(),
        None => live,
    };
    for command in ai_orders {
        if !full_commands {
            commands.push(command);
        }
    }
    commands
}

/// Ways of narrowing a selection (see [`Game::filter_selection`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionFilter {
    Combat,
    Builders,
    Damaged,
    Idle,
}

/// A notable kill for the cast's feed: what was destroyed and whose, and what destroyed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KillNote {
    pub victim: usize,
    pub victim_owner: u8,
    pub killer: Option<(usize, u8)>,
    pub tick: u64,
}

/// A fight for the action camera: its middle, how many units are firing in it, how far they
/// spread (world units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fight {
    pub at: Vec3,
    pub firing: u32,
    pub spread: f32,
}

/// Where a battle is left for, asked first ([`Game::leave_asked`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leave {
    /// The menus (the page the battle came from).
    Menu,
    /// The desktop.
    Quit,
    /// The same battle from its start.
    Restart,
}

/// How the match ended for the local player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Victory,
    Defeat,
}

/// What TA's unit information (F1) is about: a kind of unit, and the unit itself when it is
/// about one (its health, rank and owner show while it lives and is in sight).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitInfo {
    pub type_id: usize,
    pub unit: Option<UnitId>,
}

/// Our idle constructors and factories as of a tick, a player and a spectator's seat.
#[derive(Clone)]
struct IdleUnits {
    key: (u64, u8, bool),
    builders: Vec<UnitId>,
    factories: Vec<UnitId>,
}

pub struct Game {
    /// Selected veterans that could not deploy on this map. Recreated deterministically
    /// with the setup when a save/replay loads; they stay in the Conquest reserve.
    pub undeployed_conquest_veterans: Vec<crate::conquest::Veteran>,
    pub profile_progress: crate::profile_progress::MatchProgress,
    pub profile_allegiance: Option<ta_config::profiles::Allegiance>,
    /// The map's world (its OTA's `planet`), for the career's record of where battles were won.
    pub world: String,
    /// The map's title (its OTA's `missionname`, else its file's name).
    pub map_title: String,
    /// What the battle has done for the player's career so far (its result's page shows it).
    pub career_record: Option<ta_ui::profiles::BattleRecord>,
    /// The result screen's own state, once the battle is decided (`result.rs`).
    pub debrief: crate::result::Debrief,
    /// A test battle on a map the editor is editing (its way home is the editor).
    pub from_editor: bool,
    pub sim: Sim,
    pub scene: Scene,
    pub features: crate::features::FeatureView,
    pub effects: crate::effects::Effects,
    ais: Vec<ta_ai::SkirmishAi>,
    pub map: TntMap,
    pub setup: GameSetup,
    pub local_player: u8,
    /// Watching only (attract mode / spectators): no commands are issued.
    pub spectator: bool,
    /// The campaign mission being played (objectives, briefing).
    pub mission: Option<ta_data::mission::Mission>,
    /// The scenario being played (its goals as the HUD shows them), and the enemy's last
    /// few units when a goal wants them all destroyed (marked on radar).
    pub scenario: Option<crate::scenario::ScenarioRun>,
    pub last_enemies: Vec<UnitId>,
    pub briefing: Vec<(ta_data::mission::BriefColor, String)>,
    /// Where the camera starts (mission start position).
    pub start_focus: Option<Vec3>,
    /// Control groups (Ctrl+digit assigns, digit recalls, double tap looks).
    pub groups: [Vec<UnitId>; 10],
    last_group_tap: Option<(usize, std::time::Instant)>,
    /// Last unit clicked and when (double-click selects its whole type on screen).
    last_click: Option<(UnitId, std::time::Instant)>,
    /// Camera move the app should make (double-tapped group, last alert).
    pub focus_request: Option<Vec3>,
    /// Perspective asked for from the interface: into this unit.
    pub perspective_request: Option<UnitId>,
    /// Where our units came under attack, allies' signals and the like, and when (seconds of
    /// game time): F3 looks at the latest.
    pub alerts: Vec<(Vec3, f32)>,
    /// The run of presses of the alert key: the stop it was last at, and when. Pressed again soon
    /// after, it goes to the stop before.
    alert_press: Option<(usize, f32)>,
    /// Where and when a notice of units under attack was last put up for us, and for our allies:
    /// a long fight is told once, not every few seconds.
    last_attack_notice: Option<(Vec3, f32)>,
    last_ally_notice: Option<(Vec3, f32)>,
    /// Where our units, or an ally's (that player), came under attack or arrived, and when: the
    /// minimap's rings and the ally strip's alarm.
    pub attacks: Vec<(Vec3, f32, Option<u8>)>,
    /// Out of the battle, we watch our team fight on (KEEP WATCHING); when it wins, the
    /// victory is ours too (`team_won`).
    pub watching_team: bool,
    pub team_won: bool,
    /// Our computer allies have said hello; when one last told us how to signal them.
    allies_greeted: bool,
    ally_hint_at: Option<f32>,
    /// The camera follows this unit (T) until the view is scrolled.
    pub follow: Option<UnitId>,
    /// Signals from us and our team on the map (see `signals.rs`), and the wheel of meanings
    /// while Alt+drag holds it.
    pub pings: Vec<crate::signals::Ping>,
    pub signal_wheel: Option<crate::signals::Wheel>,
    /// The radial menu of a unit while one is open (see `radial.rs`), whether the setting has it
    /// on and how the right button opens it, when the right button went down (held on a unit of
    /// ours, it opens the menu) and where the middle one did (a click on one opens it too).
    pub radial: Option<crate::radial::Radial>,
    pub radial_enabled: bool,
    pub radial_opens: ta_config::RadialOpens,
    right_held_since: Option<std::time::Instant>,
    middle_press: Option<Vec2>,
    /// Orders just given, acknowledged where they go (see [`OrderMark`]).
    pub order_marks: Vec<OrderMark>,
    /// When the selection was last given an order (its routes show for a moment after).
    pub last_order: Option<std::time::Instant>,
    /// Seconds last announced by the self-destruct countdown voice.
    countdown_said: Option<u64>,
    /// The idle builder picked last (the next press moves on from it).
    last_idle_builder: Option<UnitId>,
    /// The officer that sends the player's constructors on duty to work (made when the first is
    /// put on one): see `game/duty.rs`.
    duty: Option<ta_ai::DutyOfficer>,
    /// Chat: our name, the line being typed (when open) and recent lines.
    pub player_name: String,
    pub chat_input: Option<String>,
    /// The chat line being typed goes to our allies only.
    pub chat_allies: bool,
    /// The chat line opened this frame (the key that opened it must not also send it).
    pub chat_fresh: bool,
    pub chat_log: Vec<crate::chat::ChatLine>,
    /// Every line of the battle's chat (shown while typing).
    pub chat_history: Vec<crate::chat::ChatLine>,
    /// Someone else's line came in this frame: TA's message beep.
    pub chat_beep: bool,
    /// TA's interface sounds for what the player did (`gamedata/allsound.tdf` events:
    /// "ADDBUILD", "CREATESQUAD"...), played by the app.
    pub ui_sounds: Vec<&'static str>,
    /// When a store last ran dry with the "no metal" or "no energy" sound (game seconds).
    dry_said: [f32; 2],
    /// TA's victory sound was played.
    victory_said: bool,
    /// Single player: the computer players' chatter, and its lines not yet taken up by the
    /// HUD's transmission card.
    pub chatter: Option<crate::chatter::Chatter>,
    pub transmissions: Vec<crate::chat::ChatLine>,
    /// Cheats that change only what this computer shows, and whether one was used.
    pub view_cheats: crate::chat::ViewCheats,
    pub view_cheats_used: bool,
    /// The cheat panel is open; its spawn tool is out.
    pub cheat_panel: bool,
    /// The share panel is open (TA's H: resources and units for an ally).
    pub share_panel: bool,
    /// TA's unit information (F1) is open, on this unit or kind.
    pub unit_info: Option<UnitInfo>,
    /// The unit TA's N last looked at (the next press goes on from it).
    next_unit_last: Option<UnitId>,
    /// Ctrl is held with armed units selected (and no order or building waiting): a click
    /// attacks whatever is under the cursor, ours included, or the ground (TA's force fire).
    pub force_fire: bool,
    /// Shift is held over one of our constructors: every constructor's planned buildings show
    /// (TA's Shift over a construction unit).
    pub show_all_plans: bool,
    /// The one unit drawn (a gallery tour's `TA_GALLERY_SOLO=1`, see app.rs): none of the others
    /// stands between the camera and it.
    pub solo: Option<UnitId>,
    /// Shift is held (over the battlefield).
    pub shift_held: bool,
    /// A targeted order/build has been queued; releasing Shift finishes its repeat mode.
    repeat_order: bool,
    pub spawn_tool: Option<crate::chat::SpawnTool>,
    /// Music the chat line asked for (the app plays it).
    pub music_requests: Vec<crate::chat::MusicRequest>,
    /// Statistics window (pause menu) and the graph it shows.
    pub stats_open: bool,
    /// The pause menu's Resign was clicked once (a second click confirms).
    pub resign_armed: bool,
    /// Leaving the battle (or starting it again) was asked for: the question is up.
    pub leave_asked: Option<Leave>,
    /// When the battle was last saved (an autosave, a quick save, SAVE GAME): its tick.
    pub saved_at: Option<u64>,
    /// The battle menu is open (the app says, for the HUD).
    pub menu_open: bool,
    pub stats_series: crate::stats::Series,
    /// Multiplayer lockstep session (`None` for single player).
    pub net: Option<ta_net::Session>,
    /// Multiplayer: how long (seconds, wall clock) the battle has been waiting for players
    /// who fell behind (0: it is not).
    pub net_wait: f32,
    /// Testing: record the state checksum right after this tick.
    pub probe_tick: Option<u64>,
    pub probe_hash: Option<u64>,
    /// All player and AI orders by tick, for replays and saved games.
    pub record: crate::replay::Recording,
    /// Compatibility fingerprint of the inputs used by this battle.
    pub record_data_hash: u64,
    /// Losses and copies of the game for the battle's highlights.
    pub tally: crate::highlights::Tally,
    pub playback: Option<Playback>,
    /// The replay is viewable, but different engine/data inputs may change its outcome.
    pub replay_approximate: bool,
    pub selection: Vec<UnitId>,
    pub placement: Option<Placement>,
    /// Piece matrices of the ghosts of the selected constructors' queued buildings.
    queued_ghosts: Vec<Vec<Mat4>>,
    /// The viewer's fog of war as drawn (eased from one state to the next), and the enemy
    /// structures the viewer remembers (drawn as ghosts where they were last seen): see `fog.rs`.
    pub fog: crate::fog::FogView,
    pub memory: crate::fog::Memory,
    pub command_mode: Option<CommandMode>,
    pub paused: bool,
    pub speed: f32,
    pub outcome: Option<Outcome>,
    pub(crate) commands: Vec<Command>,
    accumulator: f32,
    drag_start: Option<Vec2>,
    right_press: Option<Vec2>,
    /// Ctrl+drag with the order button: the ground path of a formation move being drawn.
    pub formation: Option<Vec<Vec3>>,
    /// Spectators (replays, attract mode): whose fog of war to see through (None = all).
    pub view_as: Option<u8>,
    pub drag_rect: Option<(Vec2, Vec2)>,
    /// An area order being dragged out: ground centre and radius.
    pub area: Option<(Vec3, f32)>,
    /// The part of the window the panels leave to the battle (pixels), set by the app each
    /// frame: a box dragged over a panel stops at its edge.
    pub battle_view: Option<(Vec2, Vec2)>,
    /// Where an area drag started (on the ground), while the button is down.
    area_start: Option<Vec3>,
    pub cursor_ground: Option<Vec3>,
    /// Scripted captures: where the cursor points on the ground, in place of the mouse.
    pub cursor_override: Option<Vec3>,
    /// Scripted captures: Shift held down.
    pub demo_shift: bool,
    /// The map overlay the player asked for (F2 movement, F3 heights, F4 metal, L sight).
    pub overlay: Option<crate::overlay::Overlay>,
    /// The metal map a placing showed by itself was turned off (until that placing ends).
    pub overlay_dismissed: bool,
    /// A tutorial lesson runs this game: no outcome, replay or saves of its own.
    pub tutorial: bool,
    /// A staged scene is played (`--demo stage`): nothing decides the battle, a side left without
    /// units included.
    pub staged: bool,
    /// What the player has used, for the tutorial's checks.
    pub used: crate::tutorial::Used,
    /// Places the tutorial points at: (x, z, radius), drawn like mission areas.
    pub tutorial_markers: Vec<(f32, f32, f32)>,
    /// Replays: a jump back to this tick, which the app makes by starting the replay over.
    pub request_rewind: Option<u64>,
    /// Tab is held: the players panel is up.
    pub show_players: bool,
    /// When units show health bars (from the settings).
    pub health_bars: ta_config::HealthBars,
    /// Key bindings (from the settings).
    pub keys: crate::keys::Bindings,
    /// Spectators and replays: the camera drifts to where the fighting is.
    pub action_camera: bool,
    /// Draw TA's cursors (from the settings).
    pub ta_cursors: bool,
    pub team_markers: bool,
    /// Far out, units show as icons too (from the settings; `strategic::shown`).
    pub strategic_icons: bool,
    /// Size and locator for TA's cursors (the system cursor keeps its operating-system settings).
    pub cursor_scale: f32,
    pub high_contrast_cursor: bool,
    /// Battle notice lifetime multiplier, independent of simulation speed.
    pub message_duration: f32,
    /// Which mouse button gives orders (from the settings).
    pub order_button: ta_config::OrderButton,
    /// The action camera's current goal, and seconds until it looks again.
    pub action_target: Option<Vec3>,
    pub action_retarget: f32,
    /// The action camera's hold on its fight: how far it spreads (world units), and seconds
    /// since it was chosen.
    pub action_spread: f32,
    pub action_held: f32,
    /// Replays and spectating: the cast's view (`hud/cast.rs`), the notable kills not yet taken
    /// up by its feed, and the replay's big moments found ahead (`replay_scan.rs`).
    pub cast: bool,
    pub cast_kills: Vec<KillNote>,
    /// The companion screen is open, and the pause menu asked for it opened or closed.
    pub companion_open: bool,
    pub companion_toggle: bool,
    pub replay_scan: Option<crate::replay_scan::ReplayScan>,
    pub hovered: Option<UnitId>,
    /// Physical pixels per interface point (the strategic icons' size; set by the app).
    pub ppp: f32,
    pub last_tick_ms: f32,
    /// This frame's simulation ticks and their time (the benchmark's).
    pub frame_ticks: u32,
    pub frame_tick_ms: f32,
    /// Whether a frame runs only as many ticks as leave it time to be drawn (a battle too big
    /// for the computer then runs slower than the clock, as TA's did, rather than at a few
    /// frames a second): off where the clock is fixed (captures, films, the benchmark).
    pub paced: bool,
    /// Our idle constructors and factories, found once a tick (see [`Game::idle_builders`]).
    idle_cache: std::sync::Mutex<Option<IdleUnits>>,
    /// The messages under the top bar: their words, reading time left, tone, and where a click on
    /// them goes.
    pub messages: Vec<Notice>,
    pub event_history: crate::battle_history::History,
    pub history_open: bool,
    /// Simulation events from the last update, for the audio system.
    pub events_out: Vec<SimEvent>,
    /// Unit voice acknowledgements requested by input handling.
    pub voices: Vec<(UnitId, ta_audio::SoundEvent)>,
}

impl Game {
    pub fn new(assets: &Assets, renderer: &mut Renderer, setup: &GameSetup) -> Result<Game> {
        setup.validate()?;
        let t0 = std::time::Instant::now();
        let vfs = assets.vfs.as_ref();
        let map_name = &setup.map;
        let map_path = format!("maps/{map_name}.tnt");
        let map = TntMap::parse(&vfs.read(&map_path).with_context(|| map_path.clone())?)?;
        let info = MapInfo::from_ota(&vfs.read_opt(&format!("maps/{map_name}.ota")).map(|b| tdf::parse_bytes(&b)).unwrap_or_default());
        // Campaign missions come with their own OTA (objectives, placed units).
        let mission = match &setup.mission {
            Some(ms) => Some(ta_data::mission::Mission::load(vfs, &ms.file).with_context(|| format!("mission {}", ms.file))?),
            None => None,
        };
        let schema_index = match (&mission, &setup.mission) {
            (Some(m), Some(ms)) => m.schemas.iter().position(|s| s.kind.eq_ignore_ascii_case(&ms.difficulty)).unwrap_or(0),
            _ => 0,
        };
        let mission_schema = mission.as_ref().and_then(|m| m.schemas.get(schema_index).cloned());
        let scenario = match &setup.scenario {
            Some(sc) => Some(crate::scenario::Scenario::parse(&sc.source).with_context(|| format!("scenario {}", sc.id))?),
            None => None,
        };
        let schema = info.schemas.get(schema_index).cloned().unwrap_or_default();
        let mut starts: Vec<FxVec3> = schema.start_positions.iter().map(|&(_, x, z)| FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z))).collect();
        // Maps with fewer start positions than players (single-player maps): give the
        // others the spot farthest from everyone placed so far, never a shared one.
        let (w, h) = (map.width_px() as i64, map.height_px() as i64);
        let spots: Vec<FxVec3> = [(1, 1), (3, 3), (3, 1), (1, 3), (2, 1), (2, 3), (1, 2), (3, 2), (2, 2)]
            .iter()
            .map(|&(a, b)| FxVec3::from_ints(w * a / 4, 0, h * b / 4))
            .collect();
        while starts.len() < setup.players.len().max(1) {
            let far = spots
                .iter()
                .copied()
                .max_by_key(|s| starts.iter().map(|t| (*s - *t).xz().length_sq_raw()).min().unwrap_or(i128::MAX))
                .unwrap_or(FxVec3::ZERO);
            starts.push(far);
        }
        // A scenario says where its forces stand, in words the map's size and start positions
        // give meaning to.
        let ground = crate::scenario::Ground { size: (w as f32, h as f32), starts: starts.iter().map(|p| (p.x.to_f32(), p.z.to_f32())).collect() };
        if let Some(s) = &scenario {
            starts = s.starts(&ground)?.into_iter().map(|(x, z)| FxVec3::new(Fx::from_f32(x), Fx::ZERO, Fx::from_f32(z))).collect();
        }
        let players: Vec<PlayerSetup> = setup
            .players
            .iter()
            .enumerate()
            .map(|(i, slot)| {
                let ai = matches!(slot.controller, Controller::Ai(_));
                // Missions set exact starting resources; skirmishes use the chosen amount
                // (Conquest adds the player's bonus from the systems held).
                // (Whole amounts: exact however large, up to what the simulation holds.)
                let whole = |v: u64| Fx::from_int(v.min(i64::MAX as u64) as i64);
                let (metal, energy) = match (&mission_schema, &setup.conquest) {
                    (Some(ms), _) if ai => (Fx::from_f64(ms.computer_metal), Fx::from_f64(ms.computer_energy)),
                    (Some(ms), _) => (Fx::from_f64(ms.human_metal), Fx::from_f64(ms.human_energy)),
                    (None, Some(cs)) if setup.local_player == Some(i as u8) => {
                        (whole(setup.rules.start_metal.saturating_add(cs.metal)), whole(setup.rules.start_energy.saturating_add(cs.energy)))
                    }
                    (None, _) => (whole(setup.rules.start_metal), whole(setup.rules.start_energy)),
                };
                PlayerSetup {
                    side: crate::setup::resolve_side(&assets.data, &slot.side).0.to_string(),
                    color: slot.color,
                    start: starts[slot.start % starts.len()],
                    metal,
                    energy,
                    ai,
                }
            })
            .collect();
        // (A map bigger than the graphics card's textures: said so, instead of a black battle.)
        renderer.map_fits(&map).map_err(anyhow::Error::msg)?;

        let mut sim = Sim::new(assets.types.clone(), assets.data.clone(), &map, &info, schema_index, &players, setup.seed);
        let (ore_w, ore_h, ore) = crate::ore::deposits(&sim);
        for (i, slot) in setup.players.iter().enumerate() {
            sim.set_team(i as u8, slot.team);
        }
        // Cheats: in games with one person in them (the rule replays follow too).
        sim.cheats_allowed = setup.players.iter().filter(|s| s.controller == Controller::Human).count() <= 1;
        let mut briefing = Vec::new();
        let mut start_focus = None;
        let mut scenario_run = None;
        let mut undeployed_conquest_veterans = Vec::new();
        let mut groups: [Vec<UnitId>; 10] = Default::default();
        if let (Some(s), Some(sc)) = (&scenario, &setup.scenario) {
            scenario_run = Some(s.apply(&mut sim, sc.difficulty, &ground).with_context(|| format!("scenario {}", sc.id))?);
            start_focus = starts.first().map(|p| Vec3::new(p.x.to_f32(), 0.0, p.z.to_f32()));
        } else if let (Some(m), Some(ms)) = (&mission, &mission_schema) {
            let allowed = ta_data::mission::use_only_units(vfs, &m.use_only);
            let placed = ta_sim::mission::setup_mission(&mut sim, m, ms, allowed.as_deref());
            if !placed.skipped.is_empty() {
                log::warn!("mission {}: unknown unit types skipped: {:?}", m.file, placed.skipped);
            }
            // `InitialGroup` digits: the player's control groups at the start.
            if setup.local_player == Some(0) {
                for &(g, id) in &placed.control_groups {
                    groups[g as usize % 10].push(id);
                }
            }
            briefing = ta_data::mission::briefing(vfs, &m.brief).unwrap_or_default();
            start_focus = ms.start.map(|(x, z)| Vec3::new(x, 0.0, z));
        } else {
            let rules = &setup.rules;
            sim.commander_ends_game = rules.commander_ends_game;
            sim.unit_limit = rules.unit_limit.max(1);
            sim.los_enabled = rules.fog_of_war;
            sim.local_avoidance = rules.give_way;
            sim.group_formations = rules.give_way;
            sim.set_true_sight(rules.fog_of_war && rules.true_sight);
            sim.set_mutators(rules.mutators);
            for (i, p) in players.iter().enumerate() {
                sim.set_starting_resources(i as u8, p.metal, p.energy);
            }
            for (i, p) in players.iter().enumerate() {
                let key = setup.players.get(i).map_or(p.side.as_str(), |s| s.side.as_str());
                // (A side the data lacks, "ARM" under Total Battletech say: its first side's.)
                let side = crate::setup::resolve_side(&sim.data, key).1.or(sim.data.sides.first());
                let commander = side.map(|s| s.commander.clone()).unwrap_or_else(|| "ARMCOM".into());
                if let Some(t) = assets.types.id(&commander) {
                    let id = sim.spawn_unit(t, i as u8, p.start, 0, true);
                    if let (Some(id), Some(cs)) = (id, setup.conquest.as_ref().filter(|_| setup.local_player == Some(i as u8))) {
                        sim.set_experience(id, cs.commander_xp);
                    }
                }
            }
            if let (Some(cs), Some(me)) = (&setup.conquest, setup.local_player) {
                undeployed_conquest_veterans = deploy_veterans(&mut sim, me, players[me as usize].start, &cs.veterans);
                sim.set_perks(me, crate::conquest::battle_perks(&cs.perks));
                // The systems' reinforcements, fresh, round them (any the map has no room for
                // stay away: they are no veterans to keep).
                let fresh: Vec<(String, Fx)> = cs.reinforcements.iter().map(|unit| (unit.clone(), Fx::ZERO)).collect();
                deploy_veterans(&mut sim, me, players[me as usize].start, &fresh);
            }
            // TA's default "mapped" option: terrain known, enemy units hidden outside sight.
            if rules.mapped {
                for (i, slot) in setup.players.iter().enumerate() {
                    if slot.controller == Controller::Human {
                        sim.reveal_map_for(i as u8);
                    }
                }
            }
        }
        // All fallible setup is complete. Until this point the renderer still belongs
        // to the previous scene, so an invalid scenario can return safely to its menu.
        renderer.load_map(&map, &assets.data.palette);
        // (TA Reborn's own maps were drawn without TA's hatching.)
        let painted_by_ta = vfs.origin(&map_path).is_none_or(|o| o.layer != ta_vfs::BUILTIN_LAYER);
        renderer.set_ground_world(&info.planet, painted_by_ta);
        // Lava and acid worlds say so in their OTA; otherwise the sea is classified from the art.
        // The painted art decides mist (void) seas and lava; a lava world's OTA flag turns
        // seas that merely look like water into lava too (mist maps flagged as lava worlds,
        // such as Slate Gordon, keep their mist), an acid world's (`waterdoesdamage`) into acid.
        renderer.lighting.water.fluid = None;
        let painted = renderer.fluid();
        renderer.lighting.water.fluid = match painted {
            Some(ta_render::Fluid::Water) if info.lava_world => Some(ta_render::Fluid::Lava),
            Some(ta_render::Fluid::Water) if info.water_does_damage => Some(ta_render::Fluid::Acid),
            _ => None,
        };
        renderer.sprites.reset_atlas();
        renderer.set_ore(ore_w, ore_h, &ore);
        let mut scene = Scene::default();
        scene.preload_models(&sim, renderer, &|p| vfs.read_opt(p));
        scene.after_tick(&sim);
        let features = crate::features::FeatureView::new(vfs, renderer, &sim);
        let mut effects = crate::effects::Effects::new(vfs, renderer, &sim);
        effects.measure_hulls(&sim, &scene);
        effects.measure_feet(&sim, &scene);
        effects.set_world(&info.planet, setup.seed ^ (map_name.len() as u64 * 2654435761));
        // A scenario's weather (`TA_RAIN` over it, for captures).
        if let Some(on) = scenario.as_ref().and_then(|s| s.rain)
            && std::env::var_os("TA_RAIN").is_none()
        {
            effects.set_rain(on);
        }
        // (A scenario's enemies nobody plays have no computer player: their units follow
        // their orders.)
        let ais = setup
            .players
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| match slot.controller {
                Controller::Ai(_) if scenario.as_ref().is_some_and(|s| s.passive(i as u8)) => None,
                Controller::Ai(d) => Some(ta_ai::SkirmishAi::new(i as u8, &sim, d)),
                Controller::Human => None,
            })
            .collect();
        log::info!("game on '{}' ready in {:.0} ms", map_name, t0.elapsed().as_secs_f64() * 1000.0);
        Ok(Game {
            undeployed_conquest_veterans,
            profile_progress: Default::default(),
            profile_allegiance: None,
            world: info.planet.clone(),
            map_title: if info.name.trim().is_empty() { map_name.clone() } else { info.name.clone() },
            career_record: None,
            debrief: Default::default(),
            from_editor: false,
            sim,
            scene,
            features,
            effects,
            ais,
            map,
            setup: setup.clone(),
            local_player: setup.local_player.unwrap_or(0),
            spectator: setup.local_player.is_none(),
            mission,
            scenario: scenario_run,
            last_enemies: Vec::new(),
            briefing,
            start_focus,
            groups,
            last_group_tap: None,
            last_click: None,
            focus_request: None,
            perspective_request: None,
            alerts: Vec::new(),
            alert_press: None,
            last_attack_notice: None,
            last_ally_notice: None,
            attacks: Vec::new(),
            watching_team: false,
            team_won: false,
            allies_greeted: false,
            ally_hint_at: None,
            follow: None,
            pings: Vec::new(),
            signal_wheel: None,
            radial: None,
            radial_enabled: true,
            radial_opens: ta_config::RadialOpens::default(),
            right_held_since: None,
            middle_press: None,
            order_marks: Vec::new(),
            last_order: None,
            countdown_said: None,
            last_idle_builder: None,
            duty: None,
            player_name: String::new(),
            chat_input: None,
            chat_allies: false,
            chat_fresh: false,
            chat_log: Vec::new(),
            chat_history: Vec::new(),
            chat_beep: false,
            ui_sounds: Vec::new(),
            dry_said: [-60.0; 2],
            victory_said: false,
            chatter: None,
            transmissions: Vec::new(),
            view_cheats: Default::default(),
            view_cheats_used: false,
            cheat_panel: false,
            share_panel: false,
            unit_info: None,
            next_unit_last: None,
            force_fire: false,
            show_all_plans: false,
            solo: None,
            shift_held: false,
            repeat_order: false,
            spawn_tool: None,
            music_requests: Vec::new(),
            stats_open: false,
            resign_armed: false,
            leave_asked: None,
            saved_at: None,
            menu_open: false,
            stats_series: Default::default(),
            net: None,
            net_wait: 0.0,
            probe_tick: None,
            probe_hash: None,
            record: crate::replay::Recording::default(),
            record_data_hash: assets.record_data_hash(setup),
            tally: Default::default(),
            playback: None,
            replay_approximate: false,
            selection: Vec::new(),
            placement: None,
            queued_ghosts: Vec::new(),
            fog: crate::fog::FogView::default(),
            memory: crate::fog::Memory::default(),
            command_mode: None,
            paused: false,
            speed: 1.0,
            outcome: None,
            commands: Vec::new(),
            accumulator: 0.0,
            drag_start: None,
            right_press: None,
            formation: None,
            view_as: None,
            drag_rect: None,
            area: None,
            area_start: None,
            battle_view: None,
            cursor_ground: None,
            cursor_override: None,
            demo_shift: false,
            overlay: None,
            overlay_dismissed: false,
            tutorial: false,
            staged: false,
            used: Default::default(),
            tutorial_markers: Vec::new(),
            request_rewind: None,
            show_players: false,
            health_bars: ta_config::HealthBars::Auto,
            keys: crate::keys::Bindings::from_config(&ta_config::Keys::default()),
            action_camera: false,
            action_spread: 600.0,
            action_held: 0.0,
            cast: false,
            cast_kills: Vec::new(),
            companion_open: false,
            companion_toggle: false,
            replay_scan: None,
            ta_cursors: true,
            team_markers: false,
            strategic_icons: true,
            cursor_scale: 1.0,
            high_contrast_cursor: false,
            message_duration: 1.0,
            order_button: ta_config::OrderButton::Right,
            action_target: None,
            action_retarget: 0.0,
            hovered: None,
            ppp: 1.0,
            last_tick_ms: 0.0,
            frame_ticks: 0,
            frame_tick_ms: 0.0,
            paced: true,
            idle_cache: Default::default(),
            messages: Vec::new(),
            event_history: Default::default(),
            history_open: false,
            events_out: Vec::new(),
            voices: Vec::new(),
        })
    }

    /// Runs `ticks` simulation steps immediately without presentation (attract-mode warm-up).
    pub fn fast_forward(&mut self, ticks: u32) {
        for _ in 0..ticks {
            let mut commands = std::mem::take(&mut self.commands);
            for ai in &mut self.ais {
                commands.extend(ai.think(&self.sim));
            }
            self.sim.step(&commands);
            // (The battle's record keeps up, where it is kept.)
            for e in self.sim.take_events() {
                self.tally.note(&e, &self.sim);
            }
            self.tally.after_tick(&self.sim, &self.ais);
        }
        self.scene.after_tick(&self.sim);
        self.scene.after_tick(&self.sim);
    }

    /// Fraction of the way from the previous tick to the next (for interpolation).
    pub fn alpha(&self) -> f32 {
        (self.accumulator / TICK_SECONDS).clamp(0.0, 1.0)
    }

    /// Areas the mission asks the player to reach (or the tutorial points at): (x, z,
    /// radius).
    pub fn objective_areas(&self) -> Vec<(f32, f32, f32)> {
        self.mission
            .iter()
            .flat_map(|m| m.victory.iter())
            .filter_map(|g| match g {
                ta_data::mission::Goal::MoveUnitToRadius { x, z, radius, .. } => Some((*x, *z, *radius)),
                _ => None,
            })
            .chain(self.tutorial_markers.iter().copied())
            .chain(self.scenario.iter().flat_map(|s| s.areas.iter().copied()))
            .collect()
    }

    /// Whose eyes the view uses: the local player, or nobody (spectators see everything).
    pub fn viewer(&self) -> Option<u8> {
        if self.spectator {
            return self.view_as;
        }
        match self.view_cheats.reveal {
            crate::chat::Reveal::All => None,
            crate::chat::Reveal::Player(p) => Some(p),
            _ => Some(self.local_player),
        }
    }

    /// Whose economy the HUD shows: the local player, or the one a spectator watches.
    pub fn hud_player(&self) -> u8 {
        if self.spectator { self.view_as.unwrap_or(0) } else { self.local_player }
    }

    /// Replays: jumps ahead `ticks` (never past the end) without effects or sounds.
    pub fn skip_replay(&mut self, ticks: u64) {
        self.seek_replay(self.sim.tick + ticks);
    }

    /// Replays: jumps to `tick`. Ahead, the game fast-forwards there; behind, the app starts
    /// the replay over and fast-forwards (see `request_rewind`).
    pub fn seek_replay(&mut self, tick: u64) {
        let Some(pb) = self.playback.as_mut().filter(|p| !p.resume) else { return };
        let tick = tick.min(pb.end_tick);
        if tick >= self.sim.tick {
            pb.seek_to = Some(tick);
            if tick > self.sim.tick {
                // Skipped ticks deliberately omit effects. Old debris and pending blasts
                // cannot be carried across that gap as if no simulation time had passed.
                self.effects.clear_battle();
                self.events_out.clear();
            }
        } else if pb.source.is_some() {
            self.request_rewind = Some(tick);
        }
    }

    /// How the viewer perceives a unit (fog of war).
    pub fn seen(&self, u: &Unit) -> Seen {
        match self.viewer() {
            // (The +radar cheat: everything else on radar; a scenario's last enemies too.)
            Some(p) => match self.sim.seen_by(u, p) {
                Seen::Hidden if self.view_cheats.reveal == crate::chat::Reveal::Radar && !self.spectator => Seen::Radar,
                Seen::Hidden if self.last_enemies.contains(&u.id) => Seen::Radar,
                s => s,
            },
            None => Seen::Visible,
        }
    }

    /// No computer player commands `player`'s units any more (a staged demo's side, whose
    /// lines would otherwise be sent off to attack).
    pub fn retire_ai(&mut self, player: u8) {
        self.ais.retain(|ai| ai.player != player);
    }

    pub fn local_commander(&self) -> Option<UnitId> {
        self.sim.units().find(|u| u.owner == self.local_player && self.sim.unit_type(u).def.is_commander).map(|u| u.id)
    }

    /// A staged scene's command: the director's, not a player's, so it goes in even when the
    /// battle is only watched (the menu's film).
    pub fn direct(&mut self, c: Command) {
        self.commands.push(c);
    }

    pub fn issue(&mut self, c: Command) {
        if self.spectator {
            return;
        }
        // An order of the player's takes the units it is for off duty.
        self.release_from_duty(&c);
        self.used.record(&self.sim, self.local_player, &c);
        match &mut self.net {
            // Multiplayer: orders run when the host's bundle brings them back.
            Some(net) => net.submit(c),
            None => self.commands.push(c),
        }
    }

    pub fn message(&mut self, text: impl Into<String>) {
        self.show_message(text.into(), 4.0);
    }

    /// A message that something went wrong, was refused or is in danger: in red, and up for
    /// longer.
    pub fn trouble(&mut self, text: impl Into<String>) {
        post_as(&mut self.messages, &mut self.event_history, self.sim.tick, text.into(), 6.0, Tone::Trouble);
    }

    /// A message that something was done: in green, briefly.
    pub fn done(&mut self, text: impl Into<String>) {
        post_as(&mut self.messages, &mut self.event_history, self.sim.tick, text.into(), 3.0, Tone::Done);
    }

    /// A message for `secs` (see [`post`]).
    fn show_message(&mut self, text: String, secs: f32) {
        post(&mut self.messages, &mut self.event_history, self.sim.tick, text, secs);
    }

    /// Why a building of `type_id` cannot go at `at`, in words (what is in the way, what it
    /// needs under it).
    fn cannot_build(&self, type_id: usize, at: Option<Vec3>) -> String {
        use ta_sim::world::PlacementError as E;
        use ta_ui::i18n::n;
        let why = match at.and_then(|p| self.sim.placement_error(type_id, fx_vec(p))) {
            Some(E::NeedsMetal) => n("Cannot build there: an extractor goes on a metal deposit"),
            Some(E::NeedsGeothermal) => n("Cannot build there: a geothermal plant goes on a vent"),
            Some(E::TooSteep) => n("Cannot build there: the ground is too steep"),
            Some(E::TooDeep) => n("Cannot build there: the water is too deep"),
            Some(E::NeedsWater) => n("Cannot build there: it is built on water"),
            Some(E::Occupied) => n("Cannot build there: something is in the way"),
            Some(E::Feature) => n("Cannot build there: rocks or trees are in the way (reclaim them first)"),
            Some(E::OutOfBounds) => n("Cannot build there: it is off the map"),
            None => n("Cannot build there"),
        };
        ta_ui::i18n::t(why).to_string()
    }

    /// Whether leaving this battle loses something (a live battle of the player's own, not a
    /// replay, a watched one or one already over; or, hosting a multiplayer battle, everyone's,
    /// however it went for us): the way out asks first.
    pub fn leaving_asks(&self) -> bool {
        self.hosting_others() || self.playback.is_none() && !self.spectator && self.outcome.is_none() && !self.tutorial
    }

    /// Multiplayer: we host a battle that others still fight (beaten, or watching, we are its
    /// pacemaker all the same): our leaving ends it for everyone.
    pub fn hosting_others(&self) -> bool {
        self.net.as_ref().is_some_and(|n| n.is_host()) && !crate::lobby::battle_over(self)
    }

    /// Whether the battle can be saved here (single player, live).
    pub fn can_save(&self) -> bool {
        self.net.is_none() && self.playback.is_none() && !self.spectator && !self.tutorial
    }

    /// Whether this battle's replay is kept as it ends: one a player of ours played, whether or
    /// not they watch the rest of it (KEEP WATCHING after a defeat makes them a spectator; the
    /// battle is no less theirs), and a multiplayer battle a spectator watched. Not the menu's
    /// film or a single-player battle watched from its start (no seat of ours), a replay, a
    /// lesson, or a battle of under twenty seconds. (Scripted demos are the app's to leave out.)
    pub fn keeps_replay(&self) -> bool {
        keeps_replay(self.setup.local_player, self.net.is_some(), self.playback.is_some(), self.tutorial, self.sim.tick)
    }

    /// Advances the simulation by real time `dt`.
    pub fn update(&mut self, dt: f32) {
        self.frame_ticks = 0;
        self.frame_tick_ms = 0.0;
        expire_messages(&mut self.messages, dt, self.message_duration, self.paused);
        for c in &mut self.chat_log {
            c.secs -= dt;
        }
        self.chat_log.retain(|c| c.secs > 0.0);
        if self.net.is_some() {
            self.update_net(dt);
            return;
        }
        if self.paused {
            return;
        }
        self.accumulator += dt * self.speed;
        let mut steps = 0;
        // (Paced: another tick only while those of the frame took less than a tick's time.)
        let budget = TICK_SECONDS * 1000.0;
        while self.accumulator >= TICK_SECONDS && steps < 8 && !self.replay_finished() && (steps == 0 || !self.paced || self.frame_tick_ms < budget) {
            let commands = std::mem::take(&mut self.commands);
            self.run_tick(commands);
            self.accumulator -= TICK_SECONDS;
            steps += 1;
        }
        if self.replay_finished() {
            // Hold the recorded final state, including its last tick of interpolation.
            // AI must not invent a continuation after the original game ended.
            self.accumulator = TICK_SECONDS;
        } else if steps == 8 {
            self.accumulator = 0.0; // can't keep up: drop time rather than spiral
        } else if self.paced && self.frame_tick_ms >= budget {
            // Ticks slower than the clock: what is behind is let go, a tick's worth kept.
            self.accumulator = self.accumulator.min(TICK_SECONDS);
        }
        self.after_update();
    }

    /// Multiplayer: the host seals a bundle every tick; everyone steps with the bundles in
    /// order. Clients run on the host's clock and catch up quickly when bundles pile up.
    /// Pause and speed are the whole game's (see [`Game::set_pace`]).
    fn update_net(&mut self, dt: f32) {
        let Some(net) = self.net.as_mut() else { return };
        net.poll();
        for name in std::mem::take(&mut net.departed) {
            post(&mut self.messages, &mut self.event_history, self.sim.tick, ta_ui::i18n::tf("{name} left the game", &[("name", &name)]), 6.0);
        }
        for notice in std::mem::take(&mut net.notices) {
            use ta_net::Notice;
            use ta_ui::i18n::{t, tf};
            let notice = match notice {
                Notice::Paused(by) => tf("{name} paused the game", &[("name", &by)]),
                Notice::Resumed(by) => tf("{name} resumed the game", &[("name", &by)]),
                Notice::Speed(by, speed) => tf("{name} set the game speed to ×{speed}", &[("name", &by), ("speed", &format!("{:.2}", speed as f32 / 100.0))]),
                Notice::Back(name) => tf("{name} is back", &[("name", &name)]),
                Notice::ReconnectLimit => t("Reconnect history limit reached; this match can no longer be rejoined.").to_string(),
            };
            post(&mut self.messages, &mut self.event_history, self.sim.tick, notice, 4.0);
        }
        self.net_wait = net.waiting_secs();
        self.paused = net.paused;
        self.speed = net.speed as f32 / 100.0;
        // The host re-checks every frame whether it has to wait for someone.
        let can_produce = net.is_host() && net.can_produce();
        let now = self.sim.tick as f32 / 30.0;
        let me = self.local_player;
        for (from, text) in std::mem::take(&mut net.chat) {
            if let Some(rest) = text.strip_prefix(PING_PREFIX) {
                let v: Vec<i32> = rest.split(' ').filter_map(|s| s.parse().ok()).collect();
                let (player, x, z, kind) = match v[..] {
                    [player, x, z] => (player, x, z, Some(ta_sim::SignalKind::Look)),
                    [player, x, z, k] => (player, x, z, crate::signals::of_index(k as u8)),
                    _ => continue,
                };
                if let Some(kind) = kind && (self.spectator || player as u8 != me && self.sim.allied(player as u8, me)) {
                    let y = self.sim.world.height_at(Fx::from_int(x as i64), Fx::from_int(z as i64)).to_f32();
                    let at = Vec3::new(x as f32, y, z as f32);
                    self.pings.push(crate::signals::Ping { at, from: from.clone(), time: now, kind });
                    self.push_alert(at, now);
                    post_to(&mut self.messages, &mut self.event_history, self.sim.tick, crate::signals::notice(kind, &from), 4.0, Tone::Note, Some(Link::Place(at)));
                }
                continue;
            }
            // Allies-only lines: shown to the sender's allies and to spectators.
            let (from, text, player) = match text.strip_prefix(ALLY_PREFIX).and_then(|rest| rest.split_once(' ')) {
                Some((player, line)) => match player.parse::<u8>() {
                    Ok(p) if self.spectator || self.sim.allied(p, me) => (ta_ui::i18n::tf("{name} (allies)", &[("name", &from)]), line.to_string(), Some(p)),
                    _ => continue,
                },
                None => {
                    let player = self.setup.players.iter().position(|s| !s.name.is_empty() && s.name == from).map(|i| i as u8);
                    (from, text, player)
                }
            };
            self.post_chat(crate::chat::ChatLine::said(from, text, player));
        }
        if !self.paused {
            self.accumulator += dt * self.speed;
        }
        let mut steps = 0;
        let started = std::time::Instant::now();
        loop {
            let Some(net) = self.net.as_mut() else { return };
            let due = self.accumulator >= TICK_SECONDS;
            let next = self.sim.tick + 1;
            // Far behind (back in a game in progress, catching up from the start): simulate
            // for most of the frame rather than a few ticks.
            let catching_up = net.backlog() > 300 && started.elapsed().as_millis() < 12;
            if net.is_host() {
                if !can_produce {
                    // Paused or waiting: no burst of ticks when play resumes.
                    self.accumulator = self.accumulator.min(TICK_SECONDS);
                    break;
                }
                if !due || steps >= 8 || !net.can_produce() {
                    break;
                }
                net.produce_tick();
            } else {
                // A pause stops production. Finish every tick already sealed, including
                // a reconnect's final few historical ticks, before holding the state.
                let ready_to_step = due || net.backlog() > if net.paused { 0 } else { 3 };
                if !ready_to_step || steps >= 16 && !catching_up { break; }
            }
            let Some(bundle) = net.take_tick(next) else { break };
            self.run_tick(bundle);
            self.accumulator = (self.accumulator - TICK_SECONDS).max(0.0);
            steps += 1;
            // Hashes are only exchanged every HASH_INTERVAL ticks; they cost a pass over
            // every unit.
            let hash = if self.sim.tick.is_multiple_of(ta_net::HASH_INTERVAL) { self.sim.checksum() } else { 0 };
            if let Some(net) = self.net.as_mut() {
                net.report_hash(self.sim.tick, hash);
                if self.sim.tick.is_multiple_of(300) {
                    log::info!("net: tick {} backlog {} host {}", self.sim.tick, net.backlog(), net.is_host());
                }
            }
        }
        if self.accumulator > TICK_SECONDS * 8.0 {
            self.accumulator = TICK_SECONDS * 8.0;
        }
        self.after_update();
    }

    /// All orders for the next tick: historical decisions during playback, live player
    /// and AI orders otherwise. The complete stream is recorded for replays and saves.
    fn tick_orders(&mut self, live: Vec<Command>) -> Vec<Command> {
        let tick = self.sim.tick + 1;
        let ai_orders = self.ais.iter_mut().flat_map(|ai| ai.think(&self.sim));
        let commands = commands_for_tick(self.playback.as_mut(), tick, live, ai_orders);
        self.hear_answers();
        if self.record.append(tick, &commands) {
            self.message(ta_ui::i18n::t(crate::replay::RECORDING_STOPPED));
            log::warn!("{}", crate::replay::RECORDING_STOPPED);
        }
        commands
    }

    /// Highlights may also use the complete original file while watching a replay.
    pub fn highlights_available(&self) -> bool {
        self.record.available() || self.playback.as_ref().is_some_and(|p| p.source.is_some())
    }

    /// Saved games: simulates recorded ticks for up to `budget` of wall time (no effects
    /// or sounds). Returns true once caught up and play has gone live.
    pub fn catch_up(&mut self, budget: std::time::Duration) -> bool {
        let start = std::time::Instant::now();
        loop {
            let Some(pb) = &mut self.playback else { return true };
            // A save runs to its end and goes live; a replay seek runs to its tick.
            let target = if pb.resume { pb.end_tick } else { pb.seek_to.unwrap_or(0) };
            if self.sim.tick >= target {
                let mut duties = Vec::new();
                if pb.resume {
                    duties = std::mem::take(&mut pb.duties);
                    self.playback = None;
                } else {
                    pb.seek_to = None;
                }
                // (A saved game is the player's again: its constructors go back on their duties.)
                self.restore_duties(&duties);
                self.skip_past_transmissions();
                self.scene.skin_viewer = self.hud_player();
                self.scene.after_tick(&self.sim);
                self.scene.after_tick(&self.sim);
                self.accumulator = 0.0;
                // Resolve an already-decided saved battle before the profile establishes
                // its import baseline; its historic victory must not become a new reward.
                self.after_update();
                return true;
            }
            if start.elapsed() >= budget {
                return false;
            }
            let commands = self.tick_orders(Vec::new());
            self.sim.step(&commands);
            for e in self.sim.take_events() {
                if let SimEvent::FeatureCreated { index, .. } = &e {
                    self.features.created(*index, self.sim.tick as f32 / 30.0);
                }
                self.profile_progress.note(&e, &self.sim, self.local_player);
                self.tally.note(&e, &self.sim);
            }
            self.profile_progress.sample(&self.sim, self.local_player);
            self.tally.after_tick(&self.sim, &self.ais);
            if self.probe_tick == Some(self.sim.tick) {
                self.probe_hash = Some(self.sim.checksum());
            }
        }
    }

    /// A scenario's transmissions before now are not said (the battle got here by fast
    /// forward: loaded, sought or restored).
    fn skip_past_transmissions(&mut self) {
        let tick = self.sim.tick;
        if let Some(run) = self.scenario.as_mut() {
            run.next_message = run.messages.partition_point(|m| m.tick <= tick);
        }
    }

    /// The computer players (copied with the game for [`Game::restore`]).
    pub fn ais(&self) -> &[ta_ai::SkirmishAi] {
        &self.ais
    }

    /// The highlights (and the benchmark's settings test): back (or ahead) to a copy of the game
    /// (`sim` and its computer players' `ais` at `tick`), with the replay's orders from there on
    /// (`record`, when one plays). The battle's effects so far are forgotten.
    pub fn restore(&mut self, sim: &Sim, ais: &[ta_ai::SkirmishAi], tick: u64, record: Option<&crate::replay::Record>, renderer: &mut ta_render::Renderer) {
        self.sim = sim.clone();
        self.ais = ais.to_vec();
        self.duty = None;
        self.accumulator = 0.0;
        if let Some(pb) = &mut self.playback
            && let Some(record) = record
        {
            pb.ticks = record.ticks.iter().filter(|(t, _)| *t > tick).cloned().collect();
            pb.seek_to = None;
        }
        self.record.retain_through(tick);
        self.skip_past_transmissions();
        self.effects.clear_battle();
        renderer.clear_particles();
        // (The ground levelled under buildings put down since is not.)
        self.scene.forget_pads_after(&self.sim);
        renderer.forget_ground_pads_after(self.sim.tick);
        self.selection.clear();
        self.alerts.clear();
        self.alert_press = None;
        self.last_attack_notice = None;
        self.last_ally_notice = None;
        self.messages.clear();
        self.event_history.clear();
        self.tally.chronicle.forget_after(tick);
        self.scene.skin_viewer = self.hud_player();
        self.scene.after_tick(&self.sim);
        self.scene.after_tick(&self.sim);
    }

    /// Progress of a saved game catching up, or of a replay fast-forwarding (0..1), if one is.
    pub fn catch_up_progress(&self) -> Option<f32> {
        let pb = self.playback.as_ref()?;
        let target = if pb.resume { pb.end_tick } else { pb.seek_to? };
        Some((self.sim.tick as f32 / target.max(1) as f32).clamp(0.0, 1.0))
    }

    /// Replays: true once every recorded tick has been played.
    pub fn replay_finished(&self) -> bool {
        self.playback.as_ref().is_some_and(|p| p.finished(self.sim.tick))
    }

    /// One simulation tick with the given player commands and the appropriate live or
    /// historical AI decisions.
    fn run_tick(&mut self, mut commands: Vec<Command>) {
        {
            let t = std::time::Instant::now();
            // (The constructors on duty are sent to work with orders like the player's.)
            self.duty_orders(&mut commands);
            let commands = self.tick_orders(commands);
            self.sim.step(&commands);
            self.guard_commander_on_duty();
            if self.probe_tick == Some(self.sim.tick) {
                self.probe_hash = Some(self.sim.checksum());
            }
            self.scene.skin_viewer = self.hud_player();
            self.scene.after_tick(&self.sim);
            let ms = t.elapsed().as_secs_f32() * 1000.0;
            self.last_tick_ms = self.last_tick_ms * 0.9 + ms * 0.1;
            self.frame_ticks += 1;
            self.frame_tick_ms += ms;
            let now = self.sim.tick as f32 / 30.0;
            self.effects.viewer = self.viewer();
            let events = self.sim.take_events();
            self.scene.jolt(&self.sim, &events);
            self.profile_progress.sample(&self.sim, self.local_player);
            // Pieces the dead threw clear this tick (they fly as debris, not with the rest), and
            // the wrecks they left (each shown once: see `Effects::unit_died`).
            let thrown = thrown_pieces_by_unit(&events);
            let wrecks = wrecks_by_unit(&events);
            // Removed units are no longer queryable when their COB pieces are drawn. Keep
            // their owners for newly visible debris and its team markings as well as paint.
            let departed_owners: rustc_hash::FxHashMap<_, _> = events.iter().filter_map(|event| match event {
                SimEvent::UnitDestroyed { id, owner, .. } => Some((*id, *owner)),
                _ => None,
            }).collect();
            let watching = self.playback.is_some() || self.spectator;
            // A scenario's transmissions, at their times.
            let due: Vec<crate::chat::ChatLine> = match self.scenario.as_mut() {
                Some(run) => {
                    let mut lines = Vec::new();
                    while let Some(m) = run.messages.get(run.next_message).filter(|m| m.tick <= self.sim.tick) {
                        lines.push(crate::chat::ChatLine::transmission(ta_ui::i18n::t(&m.from).to_string(), m.title, ta_ui::i18n::t(&m.text).to_string(), m.player));
                        run.next_message += 1;
                    }
                    lines
                }
                None => Vec::new(),
            };
            for line in due {
                self.transmissions.push(line.clone());
                self.post_chat(line);
            }
            // No rush: a minute's warning, then the end of the ceasefire.
            let ceasefire = self.sim.mutators.ceasefire_ticks();
            if ceasefire > 0 {
                let notice = if self.sim.tick == ceasefire {
                    Some(ta_ui::i18n::t("The ceasefire is over: weapons free!"))
                } else if ceasefire > 2 * 60 * 30 && self.sim.tick + 60 * 30 == ceasefire {
                    Some(ta_ui::i18n::t("One minute of ceasefire left"))
                } else {
                    None
                };
                if let Some(text) = notice {
                    post(&mut self.messages, &mut self.event_history, self.sim.tick, text.to_string(), 6.0);
                }
            }
            for e in events {
                if watching && let SimEvent::UnitDestroyed { type_id, owner, killer, .. } = &e {
                    self.note_kill(*type_id, *owner, *killer);
                }
                self.profile_progress.note(&e, &self.sim, self.local_player);
                if let Some(c) = self.chatter.as_mut() {
                    c.on_event(&e, &self.sim, now);
                }
                if let SimEvent::UnderAttack { player, pos, unit } = &e && *player == self.local_player && !self.spectator {
                    self.units_attacked(Vec3::from_array(pos.to_f32()), *unit, now);
                }
                // An ally attacked (at most once in ten seconds each): where, and F3 looks.
                if let SimEvent::UnderAttack { player, pos, .. } = &e
                    && *player != self.local_player
                    && !self.spectator
                    && self.sim.allied(*player, self.local_player)
                {
                    let at = Vec3::from_array(pos.to_f32());
                    self.push_alert(at, now);
                    self.attacked(at, now, Some(*player));
                    if attack_is_news(&mut self.last_ally_notice, at, now) {
                        let who = self.player_label(*player);
                        let key = self.keys.text("last_alert");
                        post_to(&mut self.messages, &mut self.event_history, self.sim.tick, ta_ui::i18n::tf("{player} is under attack ({key} to look)", &[("player", &who), ("key", &key)]), 3.0, Tone::Note, Some(Link::Place(at)));
                    }
                }
                // Resources an ally sent us, or we sent.
                if let SimEvent::Shared { from, to, metal, energy } = &e && !self.spectator && (*to == self.local_player || *from == self.local_player) {
                    let (m, en) = (metal.to_int(), energy.to_int());
                    let what = match (m > 0, en > 0) {
                        (true, true) => ta_ui::i18n::tf("{metal} metal and {energy} energy", &[("metal", &m), ("energy", &en)]),
                        (true, false) => ta_ui::i18n::tf("{n} metal", &[("n", &m)]),
                        _ => ta_ui::i18n::tf("{n} energy", &[("n", &en)]),
                    };
                    let text = if *to == self.local_player {
                        ta_ui::i18n::tf("{player} sent you {what}", &[("player", &self.player_label(*from)), ("what", &what)])
                    } else {
                        ta_ui::i18n::tf("Sent {what} to {player}", &[("player", &self.player_label(*to)), ("what", &what)])
                    };
                    post(&mut self.messages, &mut self.event_history, self.sim.tick, text, 4.0);
                }
                // An ally out of the battle (a computer ally says its goodbye).
                if let SimEvent::PlayerDefeated { player } = &e
                    && *player != self.local_player
                    && !self.spectator
                    && self.sim.allied(*player, self.local_player)
                {
                    let who = self.player_label(*player);
                    post(&mut self.messages, &mut self.event_history, self.sim.tick, ta_ui::i18n::tf("Your ally {player} has been defeated", &[("player", &who)]), 5.0);
                    if self.is_computer(*player) && self.playback.is_none() {
                        let from = ta_ui::i18n::tf("{name} (allies)", &[("name", &who)]);
                        self.post_chat(crate::chat::ChatLine::said(from, ta_ui::i18n::t("I'm out. It's up to you now.").to_string(), Some(*player)));
                    }
                }
                // An ally's signal (ours were shown as we gave them): on the map, in a word,
                // and F3 looks at it.
                if let SimEvent::Signal { player, kind, at } = &e
                    && (self.spectator || self.sim.allied(*player, self.local_player))
                {
                    let now = self.sim.tick as f32 / 30.0;
                    let at = Vec3::from_array(at.to_f32());
                    let at = Vec3::new(at.x, self.sim.world.height_at(Fx::from_f32(at.x), Fx::from_f32(at.z)).to_f32(), at.z);
                    let ours = !self.spectator && *player == self.local_player;
                    let who = if ours && !self.player_name.is_empty() { self.player_name.clone() } else { self.player_label(*player) };
                    self.pings.push(crate::signals::Ping { at, from: who.clone(), time: now, kind: *kind });
                    if !ours {
                        self.push_alert(at, now);
                        post_to(&mut self.messages, &mut self.event_history, self.sim.tick, crate::signals::notice(*kind, &who), 4.0, Tone::Note, Some(Link::Place(at)));
                    }
                }
                if let SimEvent::UnitsGiven { from, to, count } = &e {
                    use ta_ui::i18n::tf;
                    if !self.spectator && *to == self.local_player {
                        let who = self.player_label(*from);
                        let text = if *count == 1 { tf("{player} gave you a unit", &[("player", &who)]) } else { tf("{player} gave you {n} units", &[("player", &who), ("n", count)]) };
                        post(&mut self.messages, &mut self.event_history, self.sim.tick, text, 5.0);
                    } else if !self.spectator && *from == self.local_player {
                        let who = self.player_label(*to);
                        let text = if *count == 1 { tf("Gave a unit to {player}", &[("player", &who)]) } else { tf("Gave {n} units to {player}", &[("player", &who), ("n", count)]) };
                        post(&mut self.messages, &mut self.event_history, self.sim.tick, text, 4.0);
                    }
                }
                // Reinforcements for us or an ally: where they came in (Backspace looks).
                if let SimEvent::WaveArrived { owner, units, pos } = &e
                    && !self.spectator
                    && self.sim.allied(*owner, self.local_player)
                {
                    let at = Vec3::from_array(pos.to_f32());
                    post_to(&mut self.messages, &mut self.event_history, self.sim.tick, ta_ui::i18n::tf("Reinforcements: {units} units have arrived ({key} to look)", &[("units", units), ("key", &self.keys.text("last_alert"))]), 5.0, Tone::Note, Some(Link::Place(at)));
                    self.attacked(at, now, Some(*owner));
                    self.push_alert(at, now);
                }
                // A scenario's wave: where it came from (Backspace looks).
                if let SimEvent::WaveArrived { owner, units, pos } = &e
                    && !self.spectator
                    && self.sim.is_enemy(*owner, self.local_player)
                {
                    let n = self.sim.mission.as_ref().map_or(0, |m| m.waves_sent);
                    let at = Vec3::from_array(pos.to_f32());
                    post_to(&mut self.messages, &mut self.event_history, self.sim.tick, ta_ui::i18n::tf("Wave {n}: {units} enemy units incoming ({key} to look)", &[("n", &n), ("units", units), ("key", &self.keys.text("last_alert"))]), 5.0, Tone::Note, Some(Link::Place(at)));
                    self.attacked(at, now, None);
                    self.push_alert(at, now);
                }
                if let SimEvent::Intercepted { pos, defender, attacker } = &e
                    && (nuclear::interception_known(&self.sim, self.viewer(), *defender, *attacker, *pos)
                        || self.view_cheats.reveal == crate::chat::Reveal::Radar && !self.spectator)
                {
                    post_to(&mut self.messages, &mut self.event_history, self.sim.tick, ta_ui::i18n::t("Missile intercepted").to_string(), 4.0, Tone::Note, Some(Link::Place(Vec3::from_array(pos.to_f32()))));
                }
                if let SimEvent::WeaponFired { unit, weapon, target, .. } = &e {
                    let w = self.sim.weapon_types.get(*weapon);
                    if w.def.stockpile && !w.def.interceptor {
                        let owner = nuclear::launch_owner(&self.sim, *unit, *weapon)
                            .or_else(|| self.sim.unit(*unit).map(|u| u.owner)).or_else(|| departed_owners.get(unit).copied());
                        let known = owner.is_some_and(|owner| nuclear::destination_known(&self.sim, self.viewer(), owner));
                        let text = if known { ta_ui::i18n::t("Nuclear missile launched") } else { ta_ui::i18n::t("Nuclear launch detected!") };
                        let link = known.then(|| Link::Place(Vec3::from_array(target.to_f32())));
                        post_to(&mut self.messages, &mut self.event_history, self.sim.tick, text.to_string(), 6.0, Tone::Note, link);
                    }
                }
                if let SimEvent::PieceExplode { unit, type_id, model_piece, .. } = e {
                    let owner = self.sim.unit(unit).map(|u| u.owner).or_else(|| departed_owners.get(&unit).copied());
                    let model = self.scene.model_for_unit(&self.sim, unit, type_id, self.hud_player()).or_else(|| {
                        owner.and_then(|owner| self.scene.model_for_side(type_id, self.sim.is_enemy(owner, self.hud_player())))
                    });
                    let team = owner.and_then(|owner| self.sim.players.get(owner as usize)).map_or(0, |p| p.color as u32);
                    self.effects.spawn_debris(model, team, self.scene.piece_surface(type_id, model_piece), &self.sim, &e, now);
                }
                if let SimEvent::FeatureCreated { index, .. } = e {
                    self.features.created(index, now);
                }
                if let SimEvent::FeatureIgnited { .. } = e
                    && !self.ui_sounds.contains(&"TREEBURN")
                {
                    self.ui_sounds.push("TREEBURN");
                }
                // A factory in a control group puts what it builds there too (TA 3.1's squads).
                if let SimEvent::UnitCompleted { id, .. } = e
                    && let Some(factory) = self.sim.unit(id).filter(|u| u.owner == self.local_player).and_then(|u| u.factory)
                {
                    for group in &mut self.groups {
                        if group.contains(&factory) && !group.contains(&id) {
                            group.push(id);
                        }
                    }
                }
                // (A unit taken off the map, reclaimed or given up, is no death: nothing of it
                // burns or comes down.)
                if let SimEvent::UnitDestroyed { id, type_id, pos, removed: false, complete, .. } = &e {
                    let pieces = thrown.get(id).map_or(&[][..], Vec::as_slice);
                    let death = crate::effects::Death { wreck: wrecks.get(id).copied(), complete: *complete };
                    self.effects.unit_died(self.scene.take_departed(*id), *type_id, Vec3::from_array(pos.to_f32()), pieces, death, &self.sim, now);
                }
                self.effects.on_event(&e, &self.sim, now);
                // (An order of the officer's own that was refused is no news to the player: it looks for another
                // job, and the unit says nothing of it. The player's own orders take a unit off duty first.)
                let officers = matches!(&e, SimEvent::OrderFailed { unit, .. } if self.on_duty(*unit));
                if let SimEvent::OrderFailed { unit, reason } = &e && !officers && !self.spectator && self.sim.unit(*unit).is_some_and(|u| u.owner == self.local_player) {
                    if *reason == "out of reach" && !self.ui_sounds.contains(&"OUTOFRANGE") {
                        self.ui_sounds.push("OUTOFRANGE");
                    }
                    let stands = self.sim.unit(*unit).map(|u| Link::Place(Vec3::from_array(u.pos.to_f32())));
                    post_to(&mut self.messages, &mut self.event_history, self.sim.tick, ta_ui::i18n::tf("Cannot comply: {reason}", &[("reason", &ta_ui::i18n::t(reason))]), 4.0, Tone::Trouble, stands);
                }
                self.tally.note(&e, &self.sim);
                if !officers {
                    self.events_out.push(e);
                }
            }
            self.tally.after_tick(&self.sim, &self.ais);
        }
    }

    /// Bookkeeping after the ticks of a frame: selection and the match outcome.
    fn after_update(&mut self) {
        let now = self.sim.tick as f32 / 30.0;
        self.resource_alarms(now);
        // A scenario that wants every enemy destroyed: once no more waves are due, the last
        // five or fewer are marked on radar (they may have strayed anywhere).
        if let Some(mobile_only) = self.scenario.as_ref().and_then(|s| s.hunt_last)
            && self.sim.tick.is_multiple_of(15)
        {
            let me = self.local_player;
            let waves_due = self.sim.mission.as_ref().is_some_and(|m| m.next_wave().is_some());
            let last: Vec<UnitId> = self
                .sim
                .units()
                .filter(|u| !u.dead && self.sim.is_enemy(u.owner, me) && (!mobile_only || self.sim.unit_type(u).locomotion != ta_sim::Locomotion::Static))
                .map(|u| u.id)
                .take(6)
                .collect();
            self.last_enemies = if waves_due || last.len() > 5 { Vec::new() } else { last };
        }
        // The computer players' lines "typed" by now (single player, live).
        if self.net.is_none() && self.playback.is_none() && !self.spectator {
            let lines = self.chatter.as_mut().map(|c| c.update(&self.sim, now)).unwrap_or_default();
            for line in lines {
                if line.title.is_some() {
                    self.transmissions.push(line.clone());
                }
                self.post_chat(line);
            }
        }
        // Self-destruct countdown voice: five, four, three...
        if !self.spectator {
            let tick = self.sim.tick;
            let next = self
                .sim
                .units()
                .filter(|u| u.owner == self.local_player && u.self_destruct_manual && !u.dead)
                .filter_map(|u| u.self_destruct_at.map(|t| (t.saturating_sub(tick), u.id)))
                .min();
            match next {
                Some((left, id)) => {
                    let secs = left.div_ceil(30);
                    if (1..=5).contains(&secs) && self.countdown_said != Some(secs) {
                        self.countdown_said = Some(secs);
                        self.voices.push((id, ta_audio::SoundEvent::Count(secs as u8)));
                    }
                }
                None => self.countdown_said = None,
            }
        }
        self.selection.retain(|&id| self.sim.unit(id).is_some() && self.sim.unit(id).is_some_and(|u| u.owner == self.local_player || self.spectator));
        let (sim, me, spectator) = (&self.sim, self.local_player, self.spectator);
        for group in &mut self.groups {
            group.retain(|&id| sim.unit(id).is_some_and(|u| spectator || u.owner == me));
        }
        // Watching our team fight on: its victory is ours too.
        if self.outcome.is_none() && self.watching_team {
            let rivals_alive = self.sim.players.iter().any(|p| self.sim.is_enemy(p.index, self.local_player) && !p.defeated);
            let team_alive = self.sim.players.iter().any(|p| self.sim.allied(p.index, self.local_player) && !p.defeated);
            if team_alive && !rivals_alive {
                (self.outcome, self.team_won, self.watching_team) = (Some(Outcome::Victory), true, false);
            } else if !team_alive {
                self.watching_team = false;
            }
        }
        // Kept watching after a defeat, no team of ours left: once one side is left, the result
        // comes up again (it never did).
        if self.outcome.is_none() && self.spectator && !self.watching_team && self.setup.local_player.is_some() && self.net.is_none() && self.playback.is_none()
            && !self.tutorial && !self.staged && crate::lobby::battle_over(self)
        {
            let lost = self.sim.players.get(self.local_player as usize).is_none_or(|p| p.defeated);
            self.outcome = Some(if lost { Outcome::Defeat } else { Outcome::Victory });
        }
        if self.outcome.is_none() && !self.spectator && !self.tutorial && !self.staged {
            if let Some(rules) = &self.sim.mission {
                // Campaign missions end on their objectives only.
                self.outcome = rules.outcome.map(|won| if won { Outcome::Victory } else { Outcome::Defeat });
                return;
            }
            let me = &self.sim.players[self.local_player as usize];
            let rivals_alive = self.sim.players.iter().any(|p| self.sim.is_enemy(p.index, self.local_player) && !p.defeated);
            if me.defeated {
                self.outcome = Some(Outcome::Defeat);
            } else if !rivals_alive {
                self.outcome = Some(Outcome::Victory);
            }
            // (The enemy's personas remember how it went.)
            if let (Some(outcome), Some(chatter)) = (self.outcome, self.chatter.as_mut()) {
                chatter.decided(outcome == Outcome::Victory);
            }
        }
    }

    /// TA's "no metal" and "no energy" sounds when a store runs dry while spending (once in
    /// thirty seconds each), and its victory sound.
    fn resource_alarms(&mut self, now: f32) {
        if self.spectator || self.playback.is_some() {
            return;
        }
        if self.outcome == Some(Outcome::Victory) && !std::mem::replace(&mut self.victory_said, true) {
            self.ui_sounds.push("Victory Condition");
        }
        let Some(p) = self.sim.players.get(self.local_player as usize) else { return };
        let e = &p.economy;
        let dry = |stock: Fx, expense: Fx, income: Fx| stock < Fx::ONE && expense > income && expense > Fx::ZERO;
        for (k, (is_dry, event)) in [(dry(e.metal, e.metal_expense, e.metal_income), "NOMETAL"), (dry(e.energy, e.energy_expense, e.energy_income), "NOENERGY")].into_iter().enumerate() {
            if is_dry && now - self.dry_said[k] >= 30.0 {
                self.dry_said[k] = now;
                self.ui_sounds.push(event);
                // The classic resource bar has no STALLING label: expose the same warning
                // in text for both interfaces, even with sound muted or unavailable.
                let text = if k == 0 { ta_ui::i18n::t("Metal depleted: construction slowed") } else { ta_ui::i18n::t("Energy depleted: construction slowed") };
                post(&mut self.messages, &mut self.event_history, self.sim.tick, text.to_string(), 5.0);
            }
        }
    }

    fn unit_at_screen(&self, camera: &Camera, screen: Vec2) -> Option<UnitId> {
        let (o, d) = camera.screen_ray(screen);
        let alpha = self.alpha();
        let mut best: Option<(f32, UnitId)> = None;
        // (Whether the pointer is on it first: the viewer's sight is asked of those only.)
        for u in self.sim.units() {
            if u.carried_by.is_some() {
                continue;
            }
            let t = self.sim.unit_type(u);
            let Some(p) = self.scene.unit_position(u.id, alpha) else { continue };
            let h = t.model.height.to_f32().max(8.0);
            let r = (t.radius.to_f32()).max(h * 0.5).max(10.0);
            let c = p + Vec3::Y * h * 0.5;
            // Ray-sphere intersection.
            let oc = o - c;
            let b = oc.dot(d);
            let disc = b * b - (oc.length_squared() - r * r);
            if disc < 0.0 {
                continue;
            }
            let t_hit = -b - disc.sqrt();
            if t_hit > 0.0 && best.is_none_or(|(bt, _)| t_hit < bt) && self.seen(u) == Seen::Visible {
                best = Some((t_hit, u.id));
            }
        }
        if best.is_some() || crate::strategic::amount(camera, self.ppp) < 0.25 {
            return best.map(|(_, id)| id);
        }
        // Far out, where units are a pixel or two: the strategic icon under the pointer.
        let mut nearest: Option<(f32, UnitId)> = None;
        let view_proj = camera.view_proj();
        for u in self.sim.units() {
            if u.carried_by.is_some() {
                continue;
            }
            let Some(p) = self.scene.unit_position(u.id, alpha) else { continue };
            let c = view_proj * (p + Vec3::Y * 8.0).extend(1.0);
            if c.w <= 0.0 {
                continue;
            }
            let ndc = c.xy() / c.w;
            let s = Vec2::new((ndc.x + 1.0) * 0.5 * camera.viewport.x, (1.0 - ndc.y) * 0.5 * camera.viewport.y);
            let (_, size) = crate::strategic::icon_of(self.sim.unit_type(u));
            let d = s.distance(screen);
            if d <= size * self.ppp * 0.5 + 3.0 && nearest.is_none_or(|(nd, _)| d < nd) && self.seen(u) == Seen::Visible {
                nearest = Some((d, u.id));
            }
        }
        nearest.map(|(_, id)| id)
    }

    pub(crate) fn screen_of(camera: &Camera, p: Vec3) -> Option<Vec2> {
        let c = camera.view_proj() * p.extend(1.0);
        if c.w <= 0.0 {
            return None;
        }
        let ndc = c.xy() / c.w;
        Some(Vec2::new((ndc.x + 1.0) * 0.5 * camera.viewport.x, (1.0 - ndc.y) * 0.5 * camera.viewport.y))
    }

    /// Losing focus cancels unfinished pointer gestures, rather than treating the cleared
    /// input buttons as a release that commits an order. The selected units and chosen
    /// build/order mode stay ready for the player's next deliberate click.
    pub fn cancel_pointer_gestures(&mut self) {
        self.drag_start = None;
        self.right_press = None;
        self.right_held_since = None;
        self.middle_press = None;
        self.radial = None;
        self.formation = None;
        self.drag_rect = None;
        self.area_start = None;
        self.area = None;
        if let Some(placement) = &mut self.placement {
            placement.cancel_drag();
        }
        self.hovered = None;
        self.cursor_ground = None;
        self.force_fire = false;
        self.show_all_plans = false;
        self.shift_held = false;
        self.show_players = false;
        self.last_click = None;
        self.last_group_tap = None;
    }

    /// Handles mouse/keyboard input for selection and orders. `ui_has_pointer` is true when
    /// the pointer is over the HUD.
    pub fn handle_input(&mut self, input: &Input, camera: &Camera, renderer: &Renderer, ui_has_pointer: bool) {
        let mouse = input.mouse;
        if self.auto_overlay().is_none() {
            self.overlay_dismissed = false;
        }
        self.cursor_ground = self.cursor_override.or_else(|| {
            renderer.terrain.as_ref().and_then(|t| {
                let (o, d) = camera.screen_ray(mouse);
                t.field.raycast(o, d)
            })
        });
        self.hovered = if ui_has_pointer { None } else { self.unit_at_screen(camera, mouse) };
        let shift = input.modifiers.shift_key() || self.demo_shift;
        // A completed Shift+click still needs its armed mode even when Shift was released
        // before this redraw. Finish it after dispatch; otherwise finish on release now.
        let queued_release = [MouseButtonId::Left, MouseButtonId::Right].into_iter()
            .any(|button| input.pointer_released(button) && input.button_released_modifiers(button).shift_key());
        finish_repeat_mode(&mut self.repeat_order, &mut self.command_mode, &mut self.placement, shift, queued_release);
        // (Set again below, once the pointer is known to be over the battlefield.)
        self.force_fire = false;
        self.show_all_plans = false;
        self.shift_held = shift && !ui_has_pointer;

        // The radial menu, while it is up, has the keys and the pad's stick (its pointer is the
        // interface's: it covers the screen).
        if self.radial.is_some() {
            self.radial_input(input);
            return;
        }

        // Placement ghost follows the cursor; dragging lays out a row of sites, one footprint
        // apart along the drag's main axis. Extractors settle on the richest spot within two
        // cells (Ctrl: exactly where the cursor is).
        let placement_modifiers = input.button_released_modifiers(MouseButtonId::Left);
        let exact = placement_modifiers.control_key();
        // Buildings already planned (queued, not started) keep their ground: without Shift the
        // selection's own plans are about to be replaced, so only others' count.
        let replaced: &[UnitId] = if placement_modifiers.shift_key() || self.demo_shift { &[] } else { &self.selection };
        let planned = if self.placement.is_some() { self.planned_footprints(replaced) } else { Vec::new() };
        // (One building put down: the selection's own plans under it never stand in its way. Put
        // down again on one, it undoes it; another over them takes their place: see
        // `own_plans_under`.)
        let others_planned = if self.placement.is_some() { self.planned_footprints(&self.selection) } else { Vec::new() };
        if let Some(pl) = &mut self.placement {
            let extractor = self.sim.types.get(pl.type_id).extracts_metal > Fx::ZERO;
            pl.pos = self.cursor_ground.map(|g| {
                let snapped = if extractor && !exact { self.sim.best_extraction_site(pl.type_id, fx_vec(g), 2) } else { self.sim.snap_to_grid(pl.type_id, fx_vec(g)) };
                Vec3::from_array(snapped.to_f32())
            });
            let t = self.sim.types.get(pl.type_id);
            let clear_of = |planned: &[(i32, i32, i32, i32)], p: Vec3| {
                let r = footprint_rect(t, fx_vec(p));
                !planned.iter().any(|q| r.0 < q.2 && q.0 < r.2 && r.1 < q.3 && q.1 < r.3)
            };
            let free = |p: Vec3| clear_of(&planned, p);
            pl.valid = pl.pos.is_some_and(|p| self.sim.can_place(pl.type_id, fx_vec(p)) && clear_of(&others_planned, p));
            pl.row.clear();
            if let (Some(a), Some(b)) = (pl.line_start, pl.pos) {
                let t = self.sim.types.get(pl.type_id);
                let footprint = Vec2::new(t.footprint.0 as f32 * 16.0, t.footprint.1 as f32 * 16.0);
                // Alt: fill the rectangle between the two corners; else a row along the drag.
                let points = if placement_modifiers.alt_key() { grid_points(a, b, footprint) } else { row_points(a, b, footprint) };
                for p in points {
                    let site = Vec3::from_array(self.sim.snap_to_grid(pl.type_id, fx_vec(p)).to_f32());
                    if pl.row.iter().any(|(q, _)| q.distance_squared(site) < 1.0) {
                        continue;
                    }
                    let ok = self.sim.can_place(pl.type_id, fx_vec(site)) && free(site);
                    pl.row.push((site, ok));
                }
            }
        }

        if input.pressed(KeyCode::Escape) {
            if self.unit_info.is_some() {
                self.unit_info = None;
            } else if self.spawn_tool.is_some() {
                self.spawn_tool = None;
            } else if self.placement.is_some() || self.command_mode.is_some() {
                self.placement = None;
                self.command_mode = None;
            } else {
                self.selection.clear();
            }
        }
        if !self.selection.is_empty() {
            for (action, mode) in [
                ("attack", CommandMode::Attack),
                ("move", CommandMode::Move),
                ("dgun", CommandMode::DGun),
                ("fight", CommandMode::Fight),
                ("patrol", CommandMode::Patrol),
                ("guard", CommandMode::Guard),
                ("capture", CommandMode::Capture),
                ("repair", CommandMode::Repair),
                ("load", CommandMode::Load),
                ("unload", CommandMode::Unload),
                ("resurrect", CommandMode::Resurrect),
                ("reclaim", CommandMode::Reclaim),
                ("area_mex", CommandMode::AreaMex),
            ] {
                if self.keys.pressed(input, action) && self.mode_available(mode) {
                    self.set_command_mode(Some(mode));
                    self.ui_sounds.push("SPECIALORDERS");
                }
            }
            // TA's standing orders: F fire orders, V move orders, X on and off.
            if !self.spectator {
                if self.keys.pressed(input, "fire_orders") {
                    self.cycle_fire_orders();
                }
                if self.keys.pressed(input, "move_orders") {
                    self.cycle_move_orders();
                }
                if self.keys.pressed(input, "on_off") {
                    self.toggle_on_off();
                }
                // TA Reborn's duties: the constructors work by themselves (the key goes through
                // them: economy, repair, reclaim, defence, then none).
                if self.keys.pressed(input, "duty") {
                    self.cycle_duty();
                }
            }
        }
        if self.keys.pressed(input, "stop") {
            self.stop_selection();
        }
        for o in crate::overlay::Overlay::ALL {
            if self.keys.pressed(input, o.action()) {
                self.toggle_overlay(o);
            }
        }
        if self.keys.pressed(input, "self_destruct") && !self.selection.is_empty() {
            self.self_destruct_selection();
        }
        // Control groups.
        const DIGITS: [KeyCode; 10] = [
            KeyCode::Digit0,
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
            KeyCode::Digit5,
            KeyCode::Digit6,
            KeyCode::Digit7,
            KeyCode::Digit8,
            KeyCode::Digit9,
        ];
        for (g, key) in DIGITS.iter().enumerate() {
            if !input.pressed(*key) {
                continue;
            }
            let modifiers = input.key_pressed_modifiers(*key);
            if modifiers.control_key() {
                self.assign_group(g);
            } else {
                self.recall_group(g, modifiers.shift_key() || self.demo_shift, false);
            }
        }
        if self.keys.pressed(input, "idle_builder") {
            self.select_next_idle_builder();
        }
        if self.keys.pressed(input, "idle_factory") {
            self.select_next_idle_factory();
        }
        if self.keys.pressed(input, "follow") {
            self.follow = if self.follow.is_some() { None } else { self.selection.first().copied() };
        }
        if self.keys.pressed(input, "next_unit") && !self.spectator {
            self.look_at_next_unit(camera);
        }
        // (TA's F12 cleared the chat; here the messages fade on their own too.)
        if self.keys.pressed(input, "clear_messages") {
            self.chat_log.clear();
            self.messages.clear();
        }
        // Alt+click the ground: ping it for the whole team; Alt+drag: the wheel of meanings,
        // the one under the pointer given on release (not while placing a building, where
        // Alt+drag fills an area, nor while an order waits for its click).
        if let Some(wheel) = self.signal_wheel.as_mut() {
            wheel.kind = crate::signals::pick(input.mouse - wheel.centre);
            if input.pressed(KeyCode::Escape) {
                self.signal_wheel = None;
            } else if input.button_released(MouseButtonId::Left) || !input.button_held(MouseButtonId::Left) {
                let wheel = *wheel;
                self.signal_wheel = None;
                self.signal(wheel.kind.unwrap_or(ta_sim::SignalKind::Look), wheel.at);
            }
            return;
        }
        if input.modifiers.alt_key()
            && input.button_pressed(MouseButtonId::Left)
            && !ui_has_pointer
            && self.placement.is_none()
            && self.command_mode.is_none()
            && let Some(g) = self.cursor_ground
        {
            self.signal_wheel = Some(crate::signals::Wheel { centre: input.mouse, at: g, kind: None });
            return;
        }
        if self.keys.pressed(input, "cloak") && !self.selection.is_empty() {
            self.toggle_cloak();
        }
        // Enter: chat to everyone (Shift+Enter: to our allies); a + line is one of TA's
        // commands or a cheat code.
        if self.keys.pressed(input, "chat") {
            self.open_chat(input.modifiers.shift_key());
        }
        if self.keys.pressed(input, "cheats") && !self.spectator {
            if self.cheats_available() {
                self.cheat_panel = !self.cheat_panel;
            } else {
                self.message(ta_ui::i18n::t("Cheats are for games with one person in them"));
            }
        }
        // (TA's PAUSE key pauses too.)
        if self.keys.pressed(input, "pause") || input.pressed(KeyCode::Pause) {
            self.set_pace(Some(!self.paused), None);
        }
        if self.keys.pressed(input, "share") && !self.spectator {
            if self.allies().is_empty() {
                self.message(ta_ui::i18n::t("No allies to share with"));
            } else {
                self.share_panel = !self.share_panel;
            }
        }
        // TA's F3 (Backspace too, as before, unless it was given to something else): the last place
        // something happened; pressed again soon after, the one before it, and so on back.
        let backspace = input.pressed(KeyCode::Backspace) && !input.modifiers.control_key() && !input.modifiers.alt_key() && !self.keys.uses(KeyCode::Backspace, false, false);
        if (self.keys.pressed(input, "last_alert") || backspace) && !self.alerts.is_empty() {
            let now = self.sim.tick as f32 / 30.0;
            let step = alert_step(self.alert_press, self.alerts.last().map(|&(_, at)| at), now);
            if let Some((place, stops)) = nth_alert(&self.alerts, step) {
                self.alert_press = Some((step % stops, now));
                self.focus_request = Some(place);
            }
        }
        let faster = self.keys.pressed(input, "faster") || input.pressed(KeyCode::NumpadAdd);
        let slower = self.keys.pressed(input, "slower") || input.pressed(KeyCode::NumpadSubtract);
        if faster || slower {
            let speed = if faster { (self.speed * 1.5).min(10.0) } else { (self.speed / 1.5).max(0.25) };
            self.set_pace(None, Some(speed));
            // (In multiplayer everyone is told, by name.)
            if self.net.is_none() && (speed - self.speed).abs() < 0.001 {
                let shown = format!("{speed:.2}");
                self.message(ta_ui::i18n::tf("Game speed ×{speed}", &[("speed", &shown.trim_end_matches('0').trim_end_matches('.'))]));
            }
        }
        if self.keys.pressed(input, "select_all") {
            self.selection = self.sim.units().filter(|u| u.owner == self.local_player && u.is_complete()).map(|u| u.id).collect();
        }
        self.show_players = self.keys.held(input, "players");
        // Ctrl+C: the Commander, and look at it (as in TA).
        if self.keys.pressed(input, "select_commander") && !self.spectator && let Some(c) = self.local_commander() {
            self.selection = vec![c];
            self.focus_request = self.sim.unit(c).map(|u| Vec3::from_array(u.pos.to_f32()));
        }
        // Ctrl+Z: every unit of the selected types, all over the map.
        if self.keys.pressed(input, "select_type") && !self.selection.is_empty() {
            let types: rustc_hash::FxHashSet<usize> = self.selection.iter().filter_map(|&id| self.sim.unit(id)).map(|u| u.type_id).collect();
            self.selection = self.sim.units().filter(|u| u.owner == self.local_player && u.is_complete() && u.carried_by.is_none() && types.contains(&u.type_id)).map(|u| u.id).collect();
        }
        // Ctrl+S: everything of ours on the screen (Ctrl+Shift+S is the quick save).
        if self.keys.pressed(input, "select_on_screen") && !self.keys.pressed(input, "quick_save") && !self.spectator {
            let alpha = self.alpha();
            let vp = camera.viewport;
            self.selection = self
                .sim
                .units()
                .filter(|u| u.owner == self.local_player && u.is_complete() && u.carried_by.is_none())
                .filter(|u| {
                    let p = self.scene.unit_position(u.id, alpha).unwrap_or(Vec3::from_array(u.pos.to_f32()));
                    Self::screen_of(camera, p).is_some_and(|s| s.x >= 0.0 && s.y >= 0.0 && s.x <= vp.x && s.y <= vp.y)
                })
                .map(|u| u.id)
                .collect();
        }
        // TA's selections by category: Ctrl+B constructors, Ctrl+F factories, Ctrl+M mines,
        // Ctrl+P armed aircraft, Ctrl+R radar, jammers and sonar, Ctrl+V aircraft, Ctrl+W
        // armed units that move. A unit is in one when its category says so (CTRL_B...), so
        // mods' units join in; a Ctrl+letter no key uses selects a mod's own CTRL_ category.
        if !self.spectator {
            for (action, category) in SELECT_CATEGORIES {
                if self.keys.pressed(input, action) {
                    self.select_category(category, shift);
                }
            }
            for (i, key) in LETTERS.iter().enumerate() {
                let modifiers = input.key_pressed_modifiers(*key);
                if modifiers.control_key() && !modifiers.alt_key()
                    && input.pressed(*key) && !self.keys.uses(*key, true, false) {
                    let letter = (b'A' + i as u8) as char;
                    let category = format!("CTRL_{letter}");
                    if self.sim.types.types.iter().any(|t| has_category(&t.def.category, &category)) {
                        self.select_category(&category, modifiers.shift_key() || self.demo_shift);
                    }
                }
            }
        }

        // The cheat panel's spawn tool takes the mouse while it is out.
        if self.spawn_tool_input(input, ui_has_pointer) {
            return;
        }
        // Over a panel nothing new starts, but a drag begun on the battlefield (a box, an area's
        // circle, a row of buildings, a formation) goes on until the button is let go, wherever.
        let dragging = self.drag_start.is_some() || self.formation.is_some() || self.right_press.is_some();
        if ui_has_pointer && !dragging {
            self.drag_rect = None;
            self.area = None;
            self.area_start = None;
            // (A middle click is the battlefield's only if it is let go there too.)
            self.middle_press = None;
            return;
        }
        // A box being dragged keeps to the battlefield.
        let drag_mouse = self.battle_view.map_or(mouse, |(lo, hi)| mouse.clamp(lo, hi));

        // Which button gives orders: the right one, or the left one (TA's original interface,
        // where the right button clears the selection).
        let left_orders = self.order_button == ta_config::OrderButton::Left;
        let order_button = if left_orders { MouseButtonId::Left } else { MouseButtonId::Right };
        let ctrl = input.modifiers.control_key();
        self.force_fire = ctrl && !self.spectator && self.placement.is_none() && self.command_mode.is_none() && self.selection_armed();
        // Shift over one of our constructors: everyone's planned buildings show.
        self.show_all_plans = shift
            && self.hovered.and_then(|h| self.sim.unit(h)).is_some_and(|u| {
                let t = self.sim.unit_type(u);
                u.owner == self.local_player && t.is_builder && !t.is_factory
            });

        // Ctrl with the order button: a drag draws a formation (the selection spreads out along
        // the line); a click attacks whatever is there, ours included, or the ground (TA's force
        // fire).
        if input.button_pressed(order_button)
            && input.button_pressed_modifiers(order_button).control_key()
            && !self.selection.is_empty()
            && self.placement.is_none()
            && (!left_orders || self.command_mode.is_none())
        {
            self.formation = Some(self.cursor_ground.into_iter().collect());
        }
        if let Some(path) = &mut self.formation {
            if input.button_held(order_button) {
                if let Some(g) = self.cursor_ground && path.last().is_none_or(|l| l.distance(g) > 12.0) {
                    path.push(g);
                }
            } else {
                let released = input.button_released_modifiers(order_button);
                let shift = released.shift_key() || self.demo_shift;
                let force_fire = released.control_key() && !self.spectator && self.placement.is_none() && self.command_mode.is_none() && self.selection_armed();
                let path = self.formation.take().unwrap_or_default();
                self.right_press = None;
                self.drag_start = None;
                if polyline_length(&path) >= 32.0 {
                    self.formation_move(&path, shift);
                } else if force_fire {
                    self.force_fire_at_cursor(shift);
                } else if let Some(id) = self.hovered.filter(|&h| self.sim.unit(h).is_some_and(|u| u.owner == self.local_player)) {
                    // (Nothing armed to fire: Ctrl+click on one of ours takes its type on screen.)
                    self.select_type_on_screen(id, camera, shift);
                }
                return;
            }
        }
        // With the right button giving orders, Ctrl+click with the left: on one of our units,
        // every unit of its type on the screen (Shift adds them); elsewhere, force fire, as the
        // left button did in TA's own interface.
        let left_released = input.button_released_modifiers(MouseButtonId::Left);
        if !left_orders && left_released.control_key() && input.button_released(MouseButtonId::Left) && self.placement.is_none() && self.command_mode.is_none() {
            let shift = left_released.shift_key() || self.demo_shift;
            let clicked = self.drag_start.is_some_and(|s| (s - mouse).length() <= DRAG_THRESHOLD);
            if clicked {
                if let Some(id) = self.hovered.filter(|&h| self.sim.unit(h).is_some_and(|u| u.owner == self.local_player)) {
                    self.select_type_on_screen(id, camera, shift);
                    self.drag_start = None;
                    return;
                }
                if !self.spectator && self.selection_armed() {
                    self.force_fire_at_cursor(shift);
                    self.drag_start = None;
                    return;
                }
            }
        }

        // Left button: select, box-select or place; in an area-capable order mode, drag a
        // circle.
        if input.button_pressed(MouseButtonId::Left) && self.formation.is_none() {
            self.drag_start = Some(mouse);
            self.area_start = self.command_mode.filter(|m| m.takes_area()).and(self.cursor_ground);
            if let Some(pl) = &mut self.placement {
                pl.line_start = pl.pos;
            }
        }
        if let Some(start) = self.drag_start {
            let dragged = input.button_held(MouseButtonId::Left) && (mouse - start).length() > DRAG_THRESHOLD;
            if let (true, Some(c), Some(g)) = (dragged, self.area_start, self.cursor_ground) {
                self.area = Some((c, Vec2::new(g.x - c.x, g.z - c.z).length()));
            } else if selection_drag(dragged, self.command_mode, self.placement.is_some()) {
                self.drag_rect = Some((start.min(drag_mouse), start.max(drag_mouse)));
            }
        }
        if input.button_released(MouseButtonId::Left) && self.drag_start.is_some() {
            let shift = left_released.shift_key() || self.demo_shift;
            self.area_start = None;
            if let (Some((c, r)), Some(mode)) = (self.area.take(), self.command_mode) {
                self.area_order(mode, c, r, shift);
                self.repeat_order = shift;
                if !shift {
                    self.command_mode = None;
                }
            } else if let Some((lo, hi)) = self.drag_rect.take() {
                let alpha = self.alpha();
                let picked: Vec<UnitId> = self
                    .sim
                    .units()
                    .filter(|u| (self.spectator || u.owner == self.local_player) && u.is_complete() && u.carried_by.is_none())
                    .filter(|u| {
                        let p = self.scene.unit_position(u.id, alpha).unwrap_or(Vec3::from_array(u.pos.to_f32()));
                        Self::screen_of(camera, p).is_some_and(|s| s.cmpge(lo).all() && s.cmple(hi).all())
                    })
                    .map(|u| u.id)
                    .collect();
                if !shift {
                    self.selection.clear();
                }
                if picked.len() > 1 {
                    self.ui_sounds.push("SelectMultipleUnits");
                }
                self.add_to_selection(picked);
            } else if let Some(mode) = self.command_mode {
                if self.target_command(mode, shift) {
                    self.repeat_order = shift;
                }
                if !shift {
                    self.command_mode = None;
                }
            } else if let Some(pl) = self.placement.as_mut().filter(|pl| pl.row.len() >= 2) {
                // A dragged row: every buildable site, in order along the drag.
                let type_id = pl.type_id;
                let mut sites: Vec<Vec3> = pl.row.iter().filter(|(_, ok)| *ok).map(|(p, _)| *p).collect();
                pl.line_start = None;
                pl.row.clear();
                let builders: Vec<UnitId> = self
                    .selection
                    .iter()
                    .copied()
                    .filter(|&id| self.sim.unit(id).is_some_and(|u| self.sim.unit_type(u).build_options.contains(&type_id)))
                    .collect();
                // A filled rectangle (Alt): its far side first, from where the builders will be,
                // so that they work their way out of it rather than wall themselves in.
                if placement_modifiers.alt_key()
                    && let Some(at) = self.builders_start(&builders, shift)
                {
                    far_side_first(&mut sites, at);
                }
                for (i, site) in sites.iter().enumerate() {
                    self.issue(Command::Order { player: self.local_player, units: builders.clone(), order: Order::Build { type_id, pos: fx_vec(*site) }, queue: shift || i > 0 });
                }
                if !sites.is_empty() && !builders.is_empty() {
                    self.repeat_order = shift;
                }
                self.used.grid |= placement_modifiers.alt_key() && sites.len() >= 4 && !builders.is_empty();
                if sites.is_empty() {
                    let at = self.placement.as_ref().and_then(|pl| pl.pos);
                    let why = self.cannot_build(type_id, at);
                    self.trouble(why);
                    self.ui_sounds.push("NOTOKTOBUILD");
                } else if let Some(&first) = builders.first() {
                    self.voices.push((first, ta_audio::SoundEvent::Ok));
                    self.ui_sounds.push("OKTOBUILD");
                }
                if !shift {
                    self.placement = None;
                }
            } else if let Some((type_id, at, valid)) = self.placement.as_mut().map(|pl| {
                pl.line_start = None;
                (pl.type_id, pl.pos, pl.valid)
            }) {
                // The selection's own plans (queued, not yet started) under it: the same building
                // put down on one again undoes it; another over them takes their place.
                let under = at.map(|p| self.own_plans_under(type_id, fx_vec(p))).unwrap_or_default();
                let here = at.map(|p| footprint_rect(self.sim.types.get(type_id), fx_vec(p)));
                let same: Vec<(UnitId, u32, usize, ta_sim::FxVec3)> = under
                    .iter()
                    .copied()
                    .filter(|&(_, _, kind, pos)| kind == type_id && here == Some(footprint_rect(self.sim.types.get(kind), pos)))
                    .collect();
                if !same.is_empty() {
                    for (unit, index, kind, pos) in same {
                        self.issue(Command::EditQueue { player: self.local_player, unit, index, type_id: kind, pos: Some(pos), to_front: false });
                    }
                    self.repeat_order = shift;
                    self.ui_sounds.push("IMMEDIATEORDERS");
                    if !shift {
                        self.placement = None;
                    }
                } else if let (Some(pos), true) = (at, valid) {
                    let builders: Vec<UnitId> = self
                        .selection
                        .iter()
                        .copied()
                        .filter(|&id| self.sim.unit(id).is_some_and(|u| self.sim.unit_type(u).build_options.contains(&type_id)))
                        .collect();
                    // (Queued after the rest: the plans under it go first. Without Shift the new
                    // order takes the place of the whole queue anyway.)
                    if shift {
                        for (unit, index, kind, pos) in under {
                            self.issue(Command::EditQueue { player: self.local_player, unit, index, type_id: kind, pos: Some(pos), to_front: false });
                        }
                    }
                    self.issue(Command::Order { player: self.local_player, units: builders, order: Order::Build { type_id, pos: fx_vec(pos) }, queue: shift });
                    self.repeat_order = shift;
                    self.ui_sounds.push("OKTOBUILD");
                    if !shift {
                        self.placement = None;
                    }
                } else {
                    let why = self.cannot_build(type_id, at);
                    self.trouble(why);
                    self.ui_sounds.push("NOTOKTOBUILD");
                }
            } else if let Some(id) = self.hovered.filter(|&id| !left_orders || self.clicks_select(id, shift)) {
                // (Spectators, in a replay or watching, pick any unit to look at; a player picks
                // their own.)
                let own = self.spectator || self.sim.unit(id).is_some_and(|u| u.owner == self.local_player);
                if shift && own {
                    if let Some(i) = self.selection.iter().position(|&s| s == id) {
                        self.selection.remove(i);
                    } else {
                        self.selection.push(id);
                    }
                } else if own {
                    // Double-click: every unit of that type on screen.
                    let now = std::time::Instant::now();
                    let double = self.last_click.is_some_and(|(last, t)| last == id && now.duration_since(t).as_secs_f32() < 0.35);
                    if double {
                        self.select_type_on_screen(id, camera, false);
                        self.last_click = None;
                    } else {
                        self.selection = vec![id];
                        self.last_click = Some((id, now));
                    }
                    self.voices.push((id, ta_audio::SoundEvent::Select));
                }
            } else if left_orders && self.can_order() {
                self.context_order(shift);
            } else if !shift {
                self.selection.clear();
            }
            self.drag_start = None;
        }

        // Middle button: a click (let go where it went down) on one of our units opens its radial
        // menu, whichever way the right button opens it; nothing else answers a middle click (a
        // middle-drag pans the view). Not over an order waiting for its click, a building being
        // placed or a formation being drawn.
        if input.button_pressed(MouseButtonId::Middle) {
            self.middle_press = Some(mouse);
        }
        if input.button_released(MouseButtonId::Middle)
            && self.middle_press.take().is_some_and(|p| (p - mouse).length() < DRAG_THRESHOLD)
            && self.formation.is_none()
            && self.placement.is_none()
            && self.command_mode.is_none()
            && let Some(id) = self.radial_target()
        {
            self.open_radial(id, mouse, false);
            return;
        }

        // Right button: the context order (repair/assist, move, rally), or with left-button
        // orders a clear selection; either way it cancels placing a building. On one of our
        // units, where the click would not give an order, it opens the unit's radial menu
        // (`radial.rs`: nothing selected, or the unit in the selection, or always with left-button
        // orders; never when the setting keeps the menu to a hold); so does holding the button
        // there, the pointer still, in every case. A right-drag orbits the camera instead.
        if input.button_pressed(MouseButtonId::Right) {
            self.right_press = Some(mouse);
            self.right_held_since = Some(std::time::Instant::now());
        }
        if let (Some(at), Some(since)) = (self.right_press, self.right_held_since)
            && input.button_held(MouseButtonId::Right)
            && crate::radial::hold_opens(since.elapsed().as_secs_f32(), (at - mouse).length(), input.modifiers.control_key())
            && self.formation.is_none()
            && self.placement.is_none()
            && self.command_mode.is_none()
            && let Some(id) = self.radial_target()
        {
            self.open_radial(id, mouse, true);
            self.right_press = None;
            self.right_held_since = None;
            return;
        }
        let right_click = input.button_released(MouseButtonId::Right)
            && self.right_press.take().is_some_and(|p| (p - mouse).length() < DRAG_THRESHOLD);
        if input.button_released(MouseButtonId::Right) {
            self.right_held_since = None;
        }
        if right_click {
            let shift = input.button_released_modifiers(MouseButtonId::Right).shift_key() || self.demo_shift;
            if self.placement.is_some() {
                self.placement = None;
                // (Shift: the placing ends and the order goes on the end of the queue as well, as
                // it would have without it: a Commander told to build a plant, Shift held, then
                // sent somewhere after it. Without Shift a right-click only ends it, as in TA.)
                if shift && !left_orders {
                    self.context_order(true);
                }
            } else if let Some(id) = self.radial_for_click() {
                self.open_radial(id, mouse, false);
            } else if !left_orders {
                self.context_order(shift);
            } else if self.command_mode.is_some() {
                self.command_mode = None;
            } else if !shift {
                self.selection.clear();
            }
        }
        finish_repeat_mode(&mut self.repeat_order, &mut self.command_mode, &mut self.placement, shift, false);
    }

    /// Target an armed command at the current pointer. The minimap shares this dispatch,
    /// so a Patrol/Fight click cannot silently turn into an ordinary move.
    fn target_command(&mut self, mode: CommandMode, shift: bool) -> bool {
        let mut issued = false;
        let owner = |h: UnitId| self.sim.unit(h).map(|u| u.owner);
        let enemy = self.hovered.filter(|&h| owner(h).is_some_and(|o| o != self.local_player));
        let friend = self.hovered.filter(|&h| owner(h) == Some(self.local_player));
        let ally = clicks::allied_target(&self.sim, self.local_player, self.hovered);
        let ground = self.cursor_ground.map(fx_vec);
        let target = enemy.map(ta_sim::Target::Unit).or(ground.map(ta_sim::Target::Ground));
        let order = match mode {
            CommandMode::Attack => target.map(Order::Attack),
            CommandMode::DGun => target.map(Order::ManualFire),
            CommandMode::Fight => ground.map(Order::Fight),
            CommandMode::Patrol => ground.map(Order::Patrol),
            CommandMode::Guard => ally.map(Order::Guard),
            CommandMode::Capture => enemy.map(Order::Capture),
            CommandMode::Unload => ground.map(Order::Unload),
            CommandMode::Resurrect => self.cursor_ground.and_then(|g| self.reclaimable_feature_at(g)).filter(|&f| self.sim.wreck_type(f).is_some()).map(Order::Resurrect),
            CommandMode::Reclaim => friend.map(Order::ReclaimUnit).or_else(|| self.cursor_ground.and_then(|g| self.reclaimable_feature_at(g)).map(Order::Reclaim)),
            CommandMode::Repair => ally.map(Order::Repair),
            CommandMode::Load => friend.map(Order::Load),
            // A click without a drag: the metal around the cursor.
            CommandMode::AreaMex => {
                if let Some(g) = self.cursor_ground {
                    self.area_order(CommandMode::AreaMex, g, 96.0, shift);
                    issued = true;
                }
                None
            }
            // As an order click on the ground: the selection spreads out around it.
            CommandMode::Move => {
                if let Some(g) = self.cursor_ground {
                    self.move_selection(g, shift);
                    issued = true;
                    if let Some(&first) = self.selection.first() {
                        self.voices.push((first, ta_audio::SoundEvent::Ok));
                    }
                }
                None
            }
        };
        if let Some(order) = order {
            let exclude = match &order {
                Order::Guard(id) | Order::Repair(id) | Order::Load(id) | Order::ReclaimUnit(id) => Some(*id),
                _ => None,
            };
            let mut units = self.units_for(&order, exclude);
            // One transport is enough to pick a unit up.
            if matches!(order, Order::Load(_)) {
                units.truncate(1);
            }
            if !units.is_empty() {
                self.mark_order(&order, false);
                self.group_order(units, order, shift);
                issued = true;
                if let Some(&first) = self.selection.first() {
                    self.voices.push((first, ta_audio::SoundEvent::Ok));
                }
            }
        }
        issued
    }

    /// The selection can take orders: it holds some of our own units.
    pub(crate) fn can_order(&self) -> bool {
        !self.spectator && self.selection.iter().any(|&id| self.sim.unit(id).is_some_and(|u| u.owner == self.local_player))
    }

    /// With left-button orders, whether a click on this unit selects it rather than giving
    /// the selection an order there: our own units do, except a nanoframe, or a factory at
    /// work, that selected constructors can help (the click helps them; Shift selects).
    fn clicks_select(&self, id: UnitId, shift: bool) -> bool {
        clicks::selects(&self.sim, self.local_player, self.spectator, &self.selection, id, shift)
    }

    /// The order a click with the order button gives the selection at the cursor, by what is
    /// there: aircraft land on a repair pad, transports load, the enemy is attacked, our
    /// units and our allies' are repaired or guarded, wrecks, rocks and trees are reclaimed,
    /// and anywhere else the selection moves there.
    fn context_order(&mut self, shift: bool) {
        if self.selection.is_empty() {
            return;
        }
        if let Some(target) = self.hovered
            && let Some(click) = clicks::click_on(&self.sim, self.local_player, &self.selection, target)
        {
            // (The cursor says the same: see `cursor_kind`.)
            let me = self.local_player;
            match click {
                // Aircraft sent to a repair pad fly there and land on it.
                clicks::UnitClick::Land(planes) => {
                    let Some(pad) = self.sim.unit(target).map(|t| t.pos) else { return };
                    self.mark_order(&Order::Guard(target), false);
                    self.issue(Command::Order { player: me, units: planes, order: Order::Move(pad), queue: shift });
                }
                // A transport picks one of ours up.
                clicks::UnitClick::Load(carrier) => {
                    self.mark_order(&Order::Load(target), false);
                    self.issue(Command::Order { player: me, units: vec![carrier], order: Order::Load(target), queue: shift });
                }
                // The armed attack the enemy's.
                clicks::UnitClick::Attack(armed) => {
                    let attack = Order::Attack(ta_sim::Target::Unit(target));
                    self.mark_order(&attack, false);
                    self.issue(Command::Order { player: me, units: armed, order: attack, queue: shift });
                    return;
                }
                // Constructors repair, or help build, a unit that needs work, and guard one that
                // doesn't (a factory: that helps its production, now and later); other mobile
                // units guard it, and factories send their units to.
                clicks::UnitClick::Help { repair, guard, .. } => {
                    if !repair.is_empty() || !guard.is_empty() {
                        self.mark_order(&Order::Repair(target), false);
                    }
                    if !repair.is_empty() {
                        self.issue(Command::Order { player: me, units: repair, order: Order::Repair(target), queue: shift });
                    }
                    if !guard.is_empty() {
                        self.issue(Command::Order { player: me, units: guard, order: Order::Guard(target), queue: shift });
                    }
                }
            }
            if let Some(&first) = self.selection.first() {
                self.voices.push((first, ta_audio::SoundEvent::Ok));
            }
            return;
        }
        // Builders right-clicking a wreck, rock or tree reclaim it.
        if let Some(index) = self.cursor_ground.and_then(|g| self.reclaimable_feature_at(g)) {
            let reclaimers: Vec<UnitId> =
                self.selection.iter().copied().filter(|&id| self.sim.unit(id).is_some_and(|u| self.sim.unit_type(u).def.can_reclaim)).collect();
            if !reclaimers.is_empty() {
                self.mark_order(&Order::Reclaim(index), false);
                self.issue(Command::Order { player: self.local_player, units: reclaimers, order: Order::Reclaim(index), queue: shift });
                if let Some(&first) = self.selection.first() {
                    self.voices.push((first, ta_audio::SoundEvent::Ok));
                }
                return;
            }
        }
        if let Some(g) = self.cursor_ground {
            self.move_selection(g, shift);
            if let Some(&first) = self.selection.first() {
                self.voices.push((first, ta_audio::SoundEvent::Ok));
            }
        }
    }

    /// Acknowledges an order where it goes (see [`OrderMark`]); the selection's routes show
    /// for a moment.
    fn mark_order(&mut self, order: &Order, small: bool) {
        let Some(kind) = MarkKind::of(order) else { return };
        let now = std::time::Instant::now();
        self.last_order = Some(now);
        let ground = |x: f32, z: f32| self.sim.world.height_at(Fx::from_f32(x), Fx::from_f32(z)).to_f32().max(self.sim.world.sea_level.to_f32());
        let (pos, unit) = match order {
            Order::Move(p) | Order::Fight(p) | Order::Patrol(p) | Order::Unload(p) => {
                let p = p.to_f32();
                (Vec3::new(p[0], ground(p[0], p[2]), p[2]), None)
            }
            Order::Attack(ta_sim::Target::Ground(p)) | Order::ManualFire(ta_sim::Target::Ground(p)) => {
                let p = p.to_f32();
                (Vec3::new(p[0], ground(p[0], p[2]), p[2]), None)
            }
            Order::Attack(ta_sim::Target::Unit(id)) | Order::ManualFire(ta_sim::Target::Unit(id)) | Order::Capture(id) | Order::Repair(id) | Order::Guard(id) | Order::Load(id) | Order::ReclaimUnit(id) => {
                let Some(u) = self.sim.unit(*id) else { return };
                (Vec3::from_array(u.pos.to_f32()), Some(*id))
            }
            Order::Reclaim(f) | Order::Resurrect(f) => {
                let Some(f) = self.sim.features.get(*f) else { return };
                (Vec3::from_array(f.pos.to_f32()), None)
            }
            _ => return,
        };
        self.order_marks.retain(|m| now.duration_since(m.born).as_secs_f32() < MARK_LIFE);
        if self.order_marks.len() < 64 {
            self.order_marks.push(OrderMark { pos, unit, kind, small, born: now });
        }
    }

    /// Whether a command mode applies to the current selection.
    /// Every unit of ours of `id`'s type on the screen (`add`: added to the selection).
    fn select_type_on_screen(&mut self, id: UnitId, camera: &Camera, add: bool) {
        let type_id = self.sim.unit(id).map(|u| u.type_id);
        let alpha = self.alpha();
        let vp = camera.viewport;
        let picked: Vec<UnitId> = self
            .sim
            .units()
            .filter(|u| u.owner == self.local_player && Some(u.type_id) == type_id && u.is_complete() && u.carried_by.is_none())
            .filter(|u| {
                let p = self.scene.unit_position(u.id, alpha).unwrap_or(Vec3::from_array(u.pos.to_f32()));
                Self::screen_of(camera, p).is_some_and(|s| s.x >= 0.0 && s.y >= 0.0 && s.x <= vp.x && s.y <= vp.y)
            })
            .map(|u| u.id)
            .collect();
        if !add {
            self.selection.clear();
        }
        for p in picked {
            if !self.selection.contains(&p) {
                self.selection.push(p);
            }
        }
        if let Some(&first) = self.selection.first() {
            self.voices.push((first, ta_audio::SoundEvent::Select));
        }
    }

    /// TA's N: the camera to the next of our units off the screen, in turn (of the selection
    /// when there is one, else every unit of ours that moves).
    fn look_at_next_unit(&mut self, camera: &Camera) {
        let alpha = self.alpha();
        let vp = camera.viewport;
        let ours = |u: &&ta_sim::Unit| u.owner == self.local_player && u.carried_by.is_none();
        let mut off: Vec<(UnitId, Vec3)> = if self.selection.is_empty() {
            self.sim.units().filter(ours).filter(|u| self.sim.unit_type(u).locomotion != ta_sim::Locomotion::Static).map(|u| u.id).collect::<Vec<_>>()
        } else {
            self.selection.iter().filter_map(|&id| self.sim.unit(id)).filter(ours).map(|u| u.id).collect()
        }
        .into_iter()
        .filter_map(|id| {
            let p = self.scene.unit_position(id, alpha).or_else(|| self.sim.unit(id).map(|u| Vec3::from_array(u.pos.to_f32())))?;
            let on_screen = Self::screen_of(camera, p).is_some_and(|s| s.x >= 0.0 && s.y >= 0.0 && s.x <= vp.x && s.y <= vp.y);
            (!on_screen).then_some((id, p))
        })
        .collect();
        off.sort_by_key(|&(id, _)| id);
        // The first after the last one looked at, round again from the start.
        let last = self.next_unit_last;
        let pick = off.iter().find(|(id, _)| last.is_none_or(|l| *id > l)).or(off.first()).copied();
        match pick {
            Some((id, p)) => {
                self.next_unit_last = Some(id);
                self.focus_request = Some(p);
            }
            None => self.message(ta_ui::i18n::t("They are all on the screen")),
        }
    }

    /// What TA's F1 describes: the first selected unit, else the one under the pointer.
    pub fn info_target(&self) -> Option<UnitInfo> {
        let id = self.selection.iter().copied().find(|&id| self.sim.unit(id).is_some()).or(self.hovered)?;
        let u = self.sim.unit(id).filter(|u| self.seen(u) == ta_sim::Seen::Visible)?;
        Some(UnitInfo { type_id: u.type_id, unit: Some(id) })
    }

    /// Every unit of ours whose category names `category` (TA's CTRL_B, CTRL_F...); `add`:
    /// added to the selection.
    pub fn select_category(&mut self, category: &str, add: bool) {
        let picked: Vec<UnitId> = self
            .sim
            .units()
            .filter(|u| u.owner == self.local_player && u.is_complete() && u.carried_by.is_none() && has_category(&self.sim.unit_type(u).def.category, category))
            .map(|u| u.id)
            .collect();
        if picked.is_empty() {
            self.message(ta_ui::i18n::t("You have none of those"));
            return;
        }
        if !add {
            self.selection.clear();
        }
        for p in picked {
            if !self.selection.contains(&p) {
                self.selection.push(p);
            }
        }
        if let Some(&first) = self.selection.first() {
            self.voices.push((first, ta_audio::SoundEvent::Select));
        }
    }

    /// Whether this type can attack: ordinary guns, bombs and stockpiled missiles
    /// take Attack. Interceptors are automatic; factory pad controls use manual fire.
    pub(crate) fn fires_on_attack(&self, t: &ta_sim::UnitType) -> bool {
        fires_on_attack(&self.sim, t)
    }

    /// Whether the selection holds a unit of ours that fires on an attack order.
    fn selection_armed(&self) -> bool {
        self.selection.iter().any(|&id| self.sim.unit(id).is_some_and(|u| u.owner == self.local_player && self.fires_on_attack(self.sim.unit_type(u))))
    }

    /// TA's force fire (Ctrl+click): the armed units attack whatever is under the cursor, a
    /// unit of any side, ours included, or else the ground (a tree there takes the shells).
    fn force_fire_at_cursor(&mut self, queue: bool) {
        let target = match self.hovered.filter(|h| !self.selection.contains(h)) {
            Some(id) => ta_sim::Target::Unit(id),
            None => match self.cursor_ground {
                Some(g) => ta_sim::Target::Ground(fx_vec(g)),
                None => return,
            },
        };
        let armed: Vec<UnitId> = self
            .selection
            .iter()
            .copied()
            .filter(|&id| self.sim.unit(id).is_some_and(|u| u.owner == self.local_player && self.fires_on_attack(self.sim.unit_type(u))))
            .collect();
        if armed.is_empty() {
            return;
        }
        let order = Order::Attack(target);
        self.mark_order(&order, false);
        self.issue(Command::Order { player: self.local_player, units: armed, order, queue });
        if let Some(&first) = self.selection.first() {
            self.voices.push((first, ta_audio::SoundEvent::Ok));
        }
    }

    /// TA's F: the selection's fire orders one step on (hold fire, return fire, fire at will),
    /// from the first armed unit's.
    pub fn cycle_fire_orders(&mut self) {
        self.ui_sounds.push("SETFIREORDERS");
        let Some(fire) = self.selection.iter().filter_map(|&id| self.sim.unit(id)).find(|u| self.sim.unit_type(u).weapons.iter().any(|w| w.is_some())).map(|u| u.fire_state) else { return };
        let next = (fire + 1) % 3;
        self.set_states(Some(next), None);
        self.message(ta_ui::i18n::t([ta_ui::i18n::n("Fire orders: hold fire"), ta_ui::i18n::n("Fire orders: return fire"), ta_ui::i18n::n("Fire orders: fire at will")][usize::from(next)]));
    }

    /// TA's V: the selection's move orders one step on (hold position, maneuver, roam), from
    /// the first armed unit's that moves.
    pub fn cycle_move_orders(&mut self) {
        self.ui_sounds.push("SETMOVEORDERS");
        let Some(movement) = self
            .selection
            .iter()
            .filter_map(|&id| self.sim.unit(id))
            .find(|u| {
                let t = self.sim.unit_type(u);
                t.weapons.iter().any(|w| w.is_some()) && t.locomotion != ta_sim::Locomotion::Static
            })
            .map(|u| u.move_state)
        else {
            return;
        };
        let next = (movement + 1) % 3;
        self.set_states(None, Some(next));
        self.message(ta_ui::i18n::t([ta_ui::i18n::n("Move orders: hold position"), ta_ui::i18n::n("Move orders: maneuver"), ta_ui::i18n::n("Move orders: roam")][usize::from(next)]));
    }

    /// TA's X: turns the selection off if the first unit that can be is on, else on.
    pub fn toggle_on_off(&mut self) {
        self.ui_sounds.push("IMMEDIATEORDERS");
        let Some(active) = self.selection.iter().filter_map(|&id| self.sim.unit(id)).find(|u| self.sim.unit_type(u).def.on_offable).map(|u| u.activated) else { return };
        self.set_active(!active);
    }

    pub fn mode_available(&self, mode: CommandMode) -> bool {
        let any = |f: &dyn Fn(&ta_sim::UnitType) -> bool| {
            self.selection.iter().any(|&id| self.sim.unit(id).is_some_and(|u| f(self.sim.unit_type(u))))
        };
        match mode {
            CommandMode::DGun => self.selection_has_dgun(),
            CommandMode::Unload => self.selection.iter().any(|&id| self.sim.unit(id).is_some_and(|u| !u.cargo.is_empty())),
            CommandMode::AreaMex => self.area_extractor_type().is_some(),
            _ => any(&|t| type_supports_mode(&self.sim, t, mode)),
        }
    }

    /// The best extractor (most metal per cell) the selected constructors can build.
    pub fn area_extractor_type(&self) -> Option<usize> {
        self.selection
            .iter()
            .filter_map(|&id| self.sim.unit(id))
            .filter(|u| u.owner == self.local_player && u.is_complete())
            .flat_map(|u| self.sim.unit_type(u).build_options.iter().copied())
            .filter(|&o| {
                let t = self.sim.types.get(o);
                t.extracts_metal > Fx::ZERO && !t.needs_geo && self.sim.is_allowed(o)
            })
            .max_by_key(|&o| (self.sim.types.get(o).extracts_metal.0, std::cmp::Reverse(o)))
    }

    /// Sites for extractors of `type_id` inside a circle: the richest first, each clear of
    /// the others, of buildings and of our planned ones; none on ground a quarter as rich as
    /// the best (painted faint metal). At most `max`.
    fn extractor_sites(&self, type_id: usize, centre: Vec3, radius: f32, max: usize) -> Vec<Vec3> {
        let t = self.sim.types.get(type_id);
        let planned = self.planned_footprints(&[]);
        let mut cands = Vec::new();
        let (cx, cz) = ((centre.x / 16.0) as i32, (centre.z / 16.0) as i32);
        let r_cells = (radius / 16.0).ceil() as i32 + 1;
        for z in cz - r_cells..=cz + r_cells {
            for x in cx - r_cells..=cx + r_cells {
                let p = Vec3::new(x as f32 * 16.0 + 8.0, 0.0, z as f32 * 16.0 + 8.0);
                if (p.x - centre.x).powi(2) + (p.z - centre.z).powi(2) > radius * radius {
                    continue;
                }
                let site = self.sim.snap_to_grid(type_id, fx_vec(p));
                let yield_ = self.sim.extraction_at(type_id, site);
                if yield_ <= Fx::ZERO || !self.sim.can_place(type_id, site) {
                    continue;
                }
                let rect = footprint_rect(t, site);
                if planned.iter().any(|q| rect.0 < q.2 && q.0 < rect.2 && rect.1 < q.3 && q.1 < rect.3) {
                    continue;
                }
                cands.push((yield_, Vec3::from_array(site.to_f32()), rect));
            }
        }
        cands.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.x.total_cmp(&b.1.x)).then(a.1.z.total_cmp(&b.1.z)));
        let best = cands.first().map_or(Fx::ZERO, |c| c.0);
        let mut taken: Vec<(i32, i32, i32, i32)> = Vec::new();
        let mut out = Vec::new();
        for (y, p, r) in cands {
            if y * 4 < best || out.len() >= max {
                break;
            }
            if taken.iter().any(|q| r.0 < q.2 && q.0 < r.2 && r.1 < q.3 && q.1 < r.3) {
                continue;
            }
            taken.push(r);
            out.push(p);
        }
        out
    }

    /// Pauses or resumes and/or sets the game speed. In multiplayer these belong to the
    /// whole game: the host applies the change for everyone and announces who made it.
    pub fn set_pace(&mut self, paused: Option<bool>, speed: Option<f32>) {
        let name = if self.player_name.is_empty() { "Host".to_string() } else { self.player_name.clone() };
        match self.net.as_mut() {
            Some(net) => net.set_pace(paused, speed.map(|s| (s * 100.0).round() as u16), &name),
            None => {
                self.paused = paused.unwrap_or(self.paused);
                self.speed = speed.unwrap_or(self.speed);
            }
        }
    }

    /// Marks a spot for the team (a plain ping).
    pub fn ping(&mut self, at: Vec3) {
        self.signal(ta_sim::SignalKind::Look, at);
    }

    /// Signals the team: shown here at once. With a computer ally the signal goes into the
    /// battle (every machine's computer players act on it, and people on our team see it);
    /// between people only, in multiplayer, it goes as a private line the host routes to
    /// teammates and spectators.
    pub fn signal(&mut self, kind: ta_sim::SignalKind, at: Vec3) {
        let now = self.sim.tick as f32 / 30.0;
        let name = if self.player_name.is_empty() { ta_ui::i18n::t("You").to_string() } else { self.player_name.clone() };
        let computer_ally = self.allies().into_iter().any(|a| matches!(self.setup.players.get(a as usize).map(|p| &p.controller), Some(crate::setup::Controller::Ai(_))));
        match self.net.as_mut() {
            _ if self.spectator || self.playback.is_some() => self.pings.push(crate::signals::Ping { at, from: name, time: now, kind }),
            Some(net) if !computer_ally => {
                self.pings.push(crate::signals::Ping { at, from: name.clone(), time: now, kind });
                net.send_chat(&name, &format!("{PING_PREFIX}{} {} {} {}", self.local_player, at.x.round() as i32, at.z.round() as i32, crate::signals::index(kind)));
            }
            // (Shown as it comes back from the battle, a tick later.)
            _ => self.issue(Command::Signal { player: self.local_player, kind, at: FxVec3::new(Fx::from_f32(at.x), Fx::ZERO, Fx::from_f32(at.z)) }),
        }
    }

    /// One of our units came under attack at `at`: the place is kept for the alert key and the
    /// minimap, and a notice goes up that looks there when it is clicked (the Commander's, red,
    /// selects it and looks; the rest of a long fight is not told again for a while).
    pub fn units_attacked(&mut self, at: Vec3, unit: UnitId, now: f32) {
        self.push_alert(at, now);
        self.attacked(at, now, None);
        let key = self.keys.text("last_alert");
        let commander = self.sim.unit(unit).is_some_and(|u| self.sim.unit_type(u).def.is_commander);
        if commander {
            post_to(&mut self.messages, &mut self.event_history, self.sim.tick, ta_ui::i18n::tf("YOUR COMMANDER IS UNDER ATTACK ({key} to look)", &[("key", &key)]), 6.0, Tone::Trouble, Some(Link::Unit(unit)));
        } else if attack_is_news(&mut self.last_attack_notice, at, now) {
            post_to(&mut self.messages, &mut self.event_history, self.sim.tick, ta_ui::i18n::tf("Units under attack ({key} to look)", &[("key", &key)]), 3.0, Tone::Note, Some(Link::Place(at)));
        }
    }

    /// Keeps a place worth looking at, for the alert key (the latest ones).
    fn push_alert(&mut self, at: Vec3, now: f32) {
        self.alerts.push((at, now));
        if self.alerts.len() > ALERTS_KEPT {
            self.alerts.remove(0);
        }
    }

    /// Goes where a notice or a piece of advice points: the camera to the place, or to the unit,
    /// which is selected.
    pub fn follow(&mut self, link: Link) {
        match link {
            Link::Place(at) => self.focus_request = Some(at),
            Link::Unit(id) => self.select_and_look(id),
        }
    }

    /// A metal deposit near our Commander with nothing on it, where an extractor could go: the
    /// nearest of the richest (where the coach sends a player whose metal runs dry).
    pub fn free_metal_near_commander(&self) -> Option<Vec3> {
        let commander = self.sim.unit(self.local_commander()?)?;
        let at = Vec3::from_array(commander.pos.to_f32());
        let extractor = self
            .sim
            .unit_type(commander)
            .build_options
            .iter()
            .copied()
            .filter(|&o| {
                let t = self.sim.types.get(o);
                t.extracts_metal > Fx::ZERO && !t.needs_geo && self.sim.is_allowed(o)
            })
            .max_by_key(|&o| (self.sim.types.get(o).extracts_metal.0, std::cmp::Reverse(o)))?;
        self.extractor_sites(extractor, at, 1200.0, 40).into_iter().min_by(|a, b| a.distance_squared(at).total_cmp(&b.distance_squared(at)))
    }

    /// A ring on the minimap where units (ours: `None`, or an ally's) were attacked or came in.
    fn attacked(&mut self, at: Vec3, now: f32, who: Option<u8>) {
        self.attacks.push((at, now, who));
        if self.attacks.len() > 16 {
            self.attacks.remove(0);
        }
    }

    /// Whether a player is played by the computer.
    pub fn is_computer(&self, player: u8) -> bool {
        matches!(self.setup.players.get(player as usize).map(|p| &p.controller), Some(crate::setup::Controller::Ai(_)))
    }

    /// Our computer allies, still in.
    pub fn computer_allies(&self) -> Vec<u8> {
        self.allies().into_iter().filter(|&a| self.is_computer(a)).collect()
    }

    /// Three seconds in, our computer allies say hello, and how to call on them.
    fn greet_allies(&mut self) {
        if self.allies_greeted || self.sim.tick < 90 {
            return;
        }
        self.allies_greeted = true;
        if self.spectator || self.playback.is_some() {
            return;
        }
        for (i, a) in self.computer_allies().into_iter().enumerate() {
            let line = if i == 0 { ta_ui::i18n::t("Ready, Commander. Alt+drag on the map to signal me: attack, defend, metal or energy.") } else { ta_ui::i18n::t("Ready. I'll follow your signals.") };
            let from = ta_ui::i18n::tf("{name} (allies)", &[("name", &self.player_label(a))]);
            self.post_chat(crate::chat::ChatLine::said(from, line.to_string(), Some(a)));
        }
    }

    /// A line to our allies in a battle with computer allies only: they answer how they take
    /// orders (once a minute at most).
    pub(crate) fn computer_allies_hear(&mut self) {
        let now = self.game_secs();
        if self.ally_hint_at.is_some_and(|t| now < t + 60.0) {
            return;
        }
        let Some(&a) = self.computer_allies().first() else { return };
        self.ally_hint_at = Some(now);
        let from = ta_ui::i18n::tf("{name} (allies)", &[("name", &self.player_label(a))]);
        self.post_chat(crate::chat::ChatLine::said(from, ta_ui::i18n::t("Signal me with Alt+drag on the map: I act on attack, defend, metal and energy.").to_string(), Some(a)));
    }

    /// What our computer allies answered: a chat line from each, to the one who asked (all of
    /// them, to someone watching).
    fn hear_answers(&mut self) {
        self.greet_allies();
        if self.playback.is_some() {
            for ai in &mut self.ais {
                ai.take_answers();
            }
            return;
        }
        let mut lines = Vec::new();
        for ai in &mut self.ais {
            for answer in ai.take_answers() {
                if self.spectator || crate::signals::answer_to(answer) == self.local_player {
                    lines.push((ai.player, crate::signals::answer_text(answer)));
                }
            }
        }
        for (player, text) in lines {
            let from = ta_ui::i18n::tf("{name} (allies)", &[("name", &self.player_label(player))]);
            self.post_chat(crate::chat::ChatLine::said(from, text, Some(player)));
        }
    }

    /// A player as the interface names them: "You", their multiplayer name, "AI (Hard)"
    /// or "Player 3".
    pub fn player_label(&self, i: u8) -> String {
        match self.setup.players.get(i as usize) {
            _ if !self.spectator && i == self.local_player => ta_ui::i18n::t("You").to_string(),
            Some(s) if !s.name.is_empty() => s.name.clone(),
            Some(crate::setup::SlotSetup { controller: crate::setup::Controller::Ai(d), .. }) => {
                let level = match d {
                    ta_ai::Difficulty::Easy => ta_ui::i18n::t("Easy"),
                    ta_ai::Difficulty::Normal => ta_ui::i18n::t("Normal"),
                    ta_ai::Difficulty::Hard => ta_ui::i18n::t("Hard"),
                };
                ta_ui::i18n::tf("AI ({level})", &[("level", &level)])
            }
            _ => ta_ui::i18n::tf("Player {n}", &[("n", &(i + 1))]),
        }
    }

    /// Whether the interface may show a player's economy and army: our own and our allies',
    /// everyone's to spectators and once the game is over.
    pub fn may_inspect(&self, player: u8) -> bool {
        self.spectator || self.outcome.is_some() || self.sim.allied(player, self.local_player)
    }

    /// The cursor TA would show: the pending order's, else what a click with the order button
    /// would do on what the pointer is over.
    pub fn cursor_kind(&self, over_ui: bool) -> crate::cursor::CursorKind {
        use crate::cursor::CursorKind as C;
        if over_ui || self.placement.is_some() {
            return C::Normal;
        }
        // (The units that fire on an attack order: bombers alone show TA's air-strike sight.)
        let attackers = || -> Vec<UnitId> {
            self.selection
                .iter()
                .copied()
                .filter(|&id| self.sim.unit(id).is_some_and(|u| u.owner == self.local_player && self.fires_on_attack(self.sim.unit_type(u))))
                .collect()
        };
        if self.force_fire {
            return clicks::attack_cursor(&self.sim, &attackers());
        }
        if let Some(mode) = self.command_mode {
            return match mode {
                CommandMode::Attack => clicks::attack_cursor(&self.sim, &attackers()),
                CommandMode::DGun | CommandMode::Fight => C::Attack,
                CommandMode::Patrol => C::Patrol,
                CommandMode::Guard => C::Defend,
                CommandMode::Capture => C::Capture,
                CommandMode::Unload => C::Unload,
                CommandMode::Resurrect => C::Revive,
                CommandMode::Reclaim => C::Reclaim,
                CommandMode::Move => C::Move,
                CommandMode::Repair => C::Repair,
                CommandMode::Load => C::Load,
                CommandMode::AreaMex => C::Repair,
            };
        }
        let me = self.local_player;
        let hovered = self.hovered.and_then(|h| self.sim.unit(h));
        if self.selection.is_empty() || self.spectator {
            return if hovered.is_some_and(|u| u.owner == me) { C::Select } else { C::Normal };
        }
        // (With left-button orders, a click on our own unit selects it.)
        if self.order_button == ta_config::OrderButton::Left && hovered.is_some_and(|u| self.clicks_select(u.id, self.shift_held)) {
            return C::Select;
        }
        // Over a unit: what a click would do to it, from the account the click works from
        // (`context_order`). A click that has nothing to do to the unit goes on to the ground.
        if let Some(target) = hovered.map(|u| u.id) {
            match clicks::click_on(&self.sim, me, &self.selection, target) {
                Some(click) => return click.cursor(&self.sim),
                None if self.selection.contains(&target) => return C::Select,
                None => {}
            }
        }
        let selected: Vec<&ta_sim::UnitType> = self.selection.iter().filter_map(|&id| self.sim.unit(id)).map(|u| self.sim.unit_type(u)).collect();
        if self.cursor_ground.and_then(|g| self.reclaimable_feature_at(g)).is_some() && selected.iter().any(|s| s.def.can_reclaim) {
            return C::Reclaim;
        }
        if selected.iter().any(|s| s.locomotion != ta_sim::Locomotion::Static || s.is_factory) {
            return C::Move;
        }
        C::Normal
    }

    /// Where the fighting is thickest: the middle of the busiest 512-pixel patch (with its
    /// neighbours) of units shooting at something the viewer can see.
    /// A kill worth the cast's feed: a Commander, a costly unit (200 metal and up), a costly
    /// building (400 and up); not a feature. Kept for the feed to take up (the last 40).
    fn note_kill(&mut self, victim: usize, victim_owner: u8, killer: Option<UnitId>) {
        let t = self.sim.types.get(victim);
        let cost = t.cost_metal.to_f32();
        let building = t.locomotion == ta_sim::Locomotion::Static;
        if t.def.is_feature || !(t.def.is_commander || cost >= 200.0 && !building || building && cost >= 400.0) {
            return;
        }
        let killer = killer.and_then(|k| self.sim.unit(k)).map(|k| (k.type_id, k.owner));
        self.cast_kills.push(KillNote { victim, victim_owner, killer, tick: self.sim.tick });
        if self.cast_kills.len() > 40 {
            self.cast_kills.remove(0);
        }
    }

    /// The fights going on, the busiest first: places where units seen are firing, in 512-unit
    /// cells and those round them, with how many fire and how far they spread.
    pub fn fights(&self) -> Vec<Fight> {
        let mut cells: std::collections::BTreeMap<(i32, i32), Vec<Vec3>> = std::collections::BTreeMap::new();
        for u in self.sim.units() {
            if u.weapons.iter().any(|w| w.target.is_some()) && self.seen(u) == Seen::Visible {
                let p = Vec3::from_array(u.pos.to_f32());
                cells.entry(((p.x / 512.0).floor() as i32, (p.z / 512.0).floor() as i32)).or_default().push(p);
            }
        }
        let mut out: Vec<Fight> = cells
            .keys()
            .map(|&(x, z)| {
                let points: Vec<Vec3> = (-1..=1).flat_map(|dz| (-1..=1).map(move |dx| (x + dx, z + dz))).filter_map(|c| cells.get(&c)).flatten().copied().collect();
                let at = points.iter().copied().sum::<Vec3>() / points.len().max(1) as f32;
                let spread = (points.iter().map(|p| p.distance_squared(at)).sum::<f32>() / points.len().max(1) as f32).sqrt() * 2.0;
                Fight { at, firing: points.len() as u32, spread: spread.max(200.0) }
            })
            .collect();
        out.sort_by(|a, b| b.firing.cmp(&a.firing).then(a.at.x.total_cmp(&b.at.x)));
        out
    }

    /// How busy the fighting is round `at` now: units seen firing within 700 world units.
    pub fn firing_near(&self, at: Vec3) -> u32 {
        self.sim.units().filter(|u| u.weapons.iter().any(|w| w.target.is_some()) && self.seen(u) == Seen::Visible).filter(|u| Vec3::from_array(u.pos.to_f32()).distance(at) < 700.0).count() as u32
    }

    /// Our allies still in the game.
    pub fn allies(&self) -> Vec<u8> {
        if self.spectator {
            return Vec::new();
        }
        (0..self.sim.players.len() as u8).filter(|&p| p != self.local_player && self.sim.allied(p, self.local_player) && !self.sim.players[p as usize].defeated).collect()
    }

    /// Gives an ally metal and energy.
    pub fn share(&mut self, to: u8, metal: i64, energy: i64) {
        self.issue(Command::Share { player: self.local_player, to, metal: Fx::from_int(metal), energy: Fx::from_int(energy) });
    }

    /// Gives the selected units to an ally (the sim keeps Commanders back).
    pub fn give_selection(&mut self, to: u8) {
        if self.spectator || self.selection.is_empty() {
            return;
        }
        let units = self.selection.clone();
        self.issue(Command::GiveUnits { player: self.local_player, units, to });
    }

    /// The units of control group `g` still ours (the group forgets the others).
    pub fn group_units(&mut self, g: usize) -> Vec<UnitId> {
        let sim = &self.sim;
        let me = self.local_player;
        self.groups[g].retain(|&id| sim.unit(id).is_some_and(|u| u.owner == me && !u.dead));
        self.groups[g].clone()
    }

    /// Control group `g` becomes the selection (Ctrl+digit).
    pub fn assign_group(&mut self, g: usize) {
        if !self.selection.is_empty() {
            self.ui_sounds.push("CREATESQUAD");
        }
        self.groups[g] = self.selection.clone();
        self.message(ta_ui::i18n::tf("Group {g}: {n} units", &[("g", &g), ("n", &self.selection.len())]));
    }

    /// Selects control group `g` (a digit): `add` adds it to the selection (Shift); a second
    /// press soon after, or `look`, looks at it too.
    pub fn recall_group(&mut self, g: usize, add: bool, look: bool) {
        if self.groups.get(g).is_some_and(|members| !members.is_empty()) {
            self.ui_sounds.push("SELECTSQUAD");
        }
        let alive = self.group_units(g);
        if alive.is_empty() {
            return;
        }
        if add {
            self.add_to_selection(alive);
            return;
        }
        let now = std::time::Instant::now();
        let double = look || self.last_group_tap.is_some_and(|(lg, t)| lg == g && now.duration_since(t).as_secs_f32() < 0.35);
        self.selection = alive;
        if double {
            let alpha = self.alpha();
            let pts: Vec<Vec3> = self.selection.iter().filter_map(|&id| self.scene.unit_position(id, alpha)).collect();
            if !pts.is_empty() {
                self.focus_request = Some(pts.iter().sum::<Vec3>() / pts.len() as f32);
            }
        }
        self.last_group_tap = Some((g, now));
    }

    /// Adds units to the selection, in order, those not in it already (thousands at a time
    /// with a box round an army).
    fn add_to_selection(&mut self, units: impl IntoIterator<Item = UnitId>) {
        let mut chosen: rustc_hash::FxHashSet<UnitId> = self.selection.iter().copied().collect();
        self.selection.extend(units.into_iter().filter(|&id| chosen.insert(id)));
    }

    /// Selects one unit and looks at it (an idle alert clicked).
    pub fn select_and_look(&mut self, id: UnitId) {
        if self.sim.unit(id).is_none() {
            return;
        }
        self.selection = vec![id];
        if let Some(p) = self.scene.unit_position(id, self.alpha()) {
            self.focus_request = Some(p);
        }
        self.voices.push((id, ta_audio::SoundEvent::Select));
    }

    /// Our mobile constructors with nothing to do, in slot order.
    pub fn idle_builders(&self) -> Vec<UnitId> {
        self.idle_units().builders
    }

    /// Our finished factories with nothing to build.
    pub fn idle_factories(&self) -> Vec<UnitId> {
        self.idle_units().factories
    }

    /// Our idle constructors and factories, found once a tick (the interface asks for them
    /// several times a frame, and a big army is many units to look through). What they do
    /// changes only with the ticks.
    fn idle_units(&self) -> IdleUnits {
        let key = (self.sim.tick, self.local_player, self.spectator);
        let mut cache = self.idle_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(found) = cache.as_ref().filter(|c| c.key == key) {
            return found.clone();
        }
        let found = IdleUnits { key, builders: self.find_idle_builders(), factories: self.find_idle_factories() };
        *cache = Some(found.clone());
        found
    }

    fn find_idle_builders(&self) -> Vec<UnitId> {
        if self.spectator {
            return Vec::new();
        }
        self.sim
            .units()
            .filter(|u| u.owner == self.local_player && u.is_complete() && !u.dead && u.orders.is_empty() && u.build.is_none() && u.carried_by.is_none())
            .filter(|u| {
                let t = self.sim.unit_type(u);
                t.is_builder && !t.is_factory
            })
            // (One on duty has its own work, and waits for it when there is none.)
            .filter(|u| !self.on_duty(u.id))
            .map(|u| u.id)
            .collect()
    }

    fn find_idle_factories(&self) -> Vec<UnitId> {
        if self.spectator {
            return Vec::new();
        }
        self.sim
            .units()
            .filter(|u| u.owner == self.local_player && u.is_complete() && !u.dead && u.factory_queue.is_empty() && u.build.is_none())
            .filter(|u| self.sim.unit_type(u).is_factory)
            .map(|u| u.id)
            .collect()
    }

    /// Selects the next idle constructor (after the last one picked) and looks at it.
    pub fn select_next_idle_builder(&mut self) {
        let idle = self.idle_builders();
        self.select_next_of(idle, ta_ui::i18n::n("No idle constructors"));
    }

    /// Selects the next idle factory and looks at it.
    pub fn select_next_idle_factory(&mut self) {
        let idle = self.idle_factories();
        self.select_next_of(idle, ta_ui::i18n::n("No idle factories"));
    }

    /// Cycles the selection through `idle` (after the last unit picked from it).
    fn select_next_of(&mut self, idle: Vec<UnitId>, none: &str) {
        if idle.is_empty() {
            self.message(ta_ui::i18n::t(none));
            return;
        }
        let next = match self.last_idle_builder.and_then(|l| idle.iter().position(|&i| i == l)) {
            Some(i) => idle[(i + 1) % idle.len()],
            None => idle[0],
        };
        self.last_idle_builder = Some(next);
        self.selection = vec![next];
        if let Some(p) = self.scene.unit_position(next, self.alpha()) {
            self.focus_request = Some(p);
        }
        self.voices.push((next, ta_audio::SoundEvent::Select));
    }

    /// Ctrl+D: starts the selection's self-destruct countdown, or cancels it.
    pub fn self_destruct_selection(&mut self) {
        let cancel = self.selection.iter().all(|&id| self.sim.unit(id).is_some_and(|u| u.self_destruct_at.is_some()));
        if cancel {
            if let Some(&first) = self.selection.first() {
                self.voices.push((first, ta_audio::SoundEvent::CancelDestruct));
            }
            self.countdown_said = None;
        }
        self.issue(Command::SelfDestruct { player: self.local_player, units: self.selection.clone() });
    }

    /// Sets standing fire and/or movement orders for the selection.
    pub fn set_states(&mut self, fire: Option<u8>, movement: Option<u8>) {
        self.issue(Command::SetStates { player: self.local_player, units: self.selection.clone(), fire, movement });
    }

    /// Turns the selected units on or off, with the first one's acknowledgement.
    pub fn set_active(&mut self, on: bool) {
        self.issue(Command::SetActive { player: self.local_player, units: self.selection.clone(), on });
        if let Some(&first) = self.selection.first() {
            self.voices.push((first, if on { ta_audio::SoundEvent::Activate } else { ta_audio::SoundEvent::Deactivate }));
        }
    }

    /// Selected factories: finished units go back on the end of the queue (or not).
    pub fn set_repeat(&mut self, on: bool) {
        self.issue(Command::SetRepeat { player: self.local_player, factories: self.selection.clone(), on });
    }

    /// Queues `count` units of a type in every selected factory (negative: removes them).
    pub fn produce(&mut self, type_id: usize, count: i32) {
        self.ui_sounds.push(if count < 0 { "SUBBUILD" } else { "ADDBUILD" });
        for &factory in &self.selection.clone() {
            self.issue(Command::Produce { player: self.local_player, factory, type_id, count });
        }
    }

    /// Cloaks the selected units that can (or decloaks them if they all are).
    pub fn toggle_cloak(&mut self) {
        let can: Vec<UnitId> = self.selection.iter().copied().filter(|&id| self.sim.unit(id).is_some_and(|u| self.sim.unit_type(u).def.cloak_cost > 0.0)).collect();
        if can.is_empty() {
            return;
        }
        let all_on = can.iter().all(|&id| self.sim.unit(id).is_some_and(|u| u.cloak_on));
        self.issue(Command::SetCloak { player: self.local_player, units: can, on: !all_on });
    }

    /// Stops every selected unit.
    pub fn stop_selection(&mut self) {
        if !self.selection.is_empty() {
            self.ui_sounds.push("IMMEDIATEORDERS");
        }
        if !self.selection.is_empty() {
            let units = self.selection.clone();
            self.issue(Command::Order { player: self.local_player, units, order: Order::Stop, queue: false });
        }
    }

    /// Whether a selected unit supports the D-gun command. Stockpiled missiles
    /// and primary command-fire weapons use Attack, as ordinary weapons do.
    pub fn selection_has_dgun(&self) -> bool {
        self.selection.iter().any(|&id| {
            self.sim.unit(id).is_some_and(|u| {
                has_dgun(&self.sim, self.sim.unit_type(u))
            })
        })
    }

    /// A reclaimable feature whose footprint contains (or is next to) a ground point.
    /// The selected units able to carry out `order` (never `exclude`, the order's own
    /// target).
    fn units_for(&self, order: &Order, exclude: Option<UnitId>) -> Vec<UnitId> {
        let can = |id: UnitId, f: &dyn Fn(&Unit, &ta_sim::UnitType) -> bool| self.sim.unit(id).is_some_and(|u| f(u, self.sim.unit_type(u)));
        self.selection
            .iter()
            .copied()
            .filter(|&id| Some(id) != exclude)
            .filter(|&id| match order {
                Order::Capture(_) => can(id, &|_, t| t.def.can_capture),
                Order::Unload(_) => can(id, &|u, _| !u.cargo.is_empty()),
                Order::Resurrect(_) => can(id, &|_, t| t.def.can_resurrect),
                Order::Reclaim(_) | Order::ReclaimUnit(_) => can(id, &|_, t| t.def.can_reclaim),
                Order::Repair(_) => can(id, &|_, t| t.is_builder && !t.is_factory),
                // An attack goes to the armed units (a constructor keeps its queue), the D-gun's
                // shot to the units that have one.
                Order::Attack(_) => can(id, &|_, t| self.fires_on_attack(t)),
                Order::ManualFire(_) => can(id, &|_, t| has_dgun(&self.sim, t)),
                Order::Guard(_) => can(id, &|_, t| type_supports_mode(&self.sim, t, CommandMode::Guard)),
                Order::Move(_) | Order::Fight(_) | Order::Patrol(_) => can(id, &|_, t| type_supports_mode(&self.sim, t, CommandMode::Move)),
                Order::Load(target) => {
                    let cargo = self.sim.unit(*target).map(|x| self.sim.unit_type(x));
                    can(id, &|u, t| cargo.is_some_and(|c| t.can_carry(c)) && (u.cargo.len() as u32) < t.transport_capacity)
                }
                _ => true,
            })
            .collect()
    }

    /// An order over an area: every target inside the circle (wrecks and rocks to reclaim
    /// or resurrect, enemies to attack or capture), queued nearest-first from where the
    /// selection stands.
    pub(crate) fn area_order(&mut self, mode: CommandMode, centre: Vec3, radius: f32, queue: bool) {
        const MAX_TARGETS: usize = 64;
        let me = self.local_player;
        if mode == CommandMode::AreaMex {
            let Some(type_id) = self.area_extractor_type() else { return };
            let sites = self.extractor_sites(type_id, centre, radius.max(48.0), 32);
            if sites.is_empty() {
                post(&mut self.messages, &mut self.event_history, self.sim.tick, ta_ui::i18n::t("No free metal there").to_string(), 2.5);
                return;
            }
            let builders: Vec<UnitId> = self
                .selection
                .iter()
                .copied()
                .filter(|&id| self.sim.unit(id).is_some_and(|u| self.sim.unit_type(u).build_options.contains(&type_id)))
                .collect();
            let from = builders.iter().filter_map(|&id| self.sim.unit(id)).map(|u| Vec3::from_array(u.pos.to_f32())).sum::<Vec3>() / builders.len().max(1) as f32;
            let stops: Vec<(Vec3, Vec3)> = sites.into_iter().map(|p| (p, p)).collect();
            for (k, site) in nearest_tour(from, stops, MAX_TARGETS).into_iter().enumerate() {
                self.issue(Command::Order { player: me, units: builders.clone(), order: Order::Build { type_id, pos: fx_vec(site) }, queue: queue || k > 0 });
            }
            self.used.area_mex |= !builders.is_empty();
            if let Some(&b) = builders.first() {
                self.voices.push((b, ta_audio::SoundEvent::Ok));
            }
            return;
        }
        let slot = self.sim.vision_slot(me);
        let inside = |p: [f32; 3]| (p[0] - centre.x).powi(2) + (p[2] - centre.z).powi(2) <= radius * radius;
        let mut targets: Vec<(Vec3, Order)> = Vec::new();
        match mode {
            CommandMode::Reclaim | CommandMode::Resurrect => {
                for (i, f) in self.sim.features.iter().enumerate() {
                    if !f.alive || !self.sim.data.features[f.def].reclaimable || !inside(f.pos.to_f32()) {
                        continue;
                    }
                    // Only what we have seen.
                    if !self.spectator && self.sim.vision.flags_at(slot, f.pos) & ta_sim::vision::EXPLORED == 0 {
                        continue;
                    }
                    let order = match mode {
                        CommandMode::Resurrect if self.sim.wreck_type(i).is_some() => Order::Resurrect(i),
                        CommandMode::Resurrect => continue,
                        _ => Order::Reclaim(i),
                    };
                    targets.push((Vec3::from_array(f.pos.to_f32()), order));
                }
            }
            CommandMode::Attack | CommandMode::Capture => {
                for u in self.sim.units() {
                    if !self.sim.is_enemy(me, u.owner) || !inside(u.pos.to_f32()) || self.seen(u) != Seen::Visible || u.carried_by.is_some() {
                        continue;
                    }
                    let order = if mode == CommandMode::Capture { Order::Capture(u.id) } else { Order::Attack(ta_sim::Target::Unit(u.id)) };
                    targets.push((Vec3::from_array(u.pos.to_f32()), order));
                }
            }
            _ => {}
        }
        let Some(first) = targets.first() else {
            post(&mut self.messages, &mut self.event_history, self.sim.tick, ta_ui::i18n::tf("Nothing to {order} there", &[("order", &ta_ui::i18n::t(mode.label()).to_lowercase())]), 2.5);
            return;
        };
        let units = self.units_for(&first.1, None);
        if units.is_empty() {
            return;
        }
        let from = units.iter().filter_map(|&id| self.sim.unit(id)).map(|u| Vec3::from_array(u.pos.to_f32())).sum::<Vec3>() / units.len() as f32;
        for (k, order) in nearest_tour(from, targets, MAX_TARGETS).into_iter().enumerate() {
            self.issue(Command::Order { player: me, units: units.clone(), order, queue: queue || k > 0 });
        }
        self.used.area_reclaim |= mode == CommandMode::Reclaim;
        self.voices.push((units[0], ta_audio::SoundEvent::Ok));
    }

    /// The reclaimable feature under the cursor that the viewer has seen (for its card).
    pub fn hovered_feature(&self) -> Option<usize> {
        let f = self.cursor_ground.and_then(|g| self.reclaimable_feature_at(g))?;
        let pos = self.sim.features[f].pos;
        self.viewer().is_none_or(|p| self.sim.explored(p, pos)).then_some(f)
    }

    fn reclaimable_feature_at(&self, ground: Vec3) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        // (The squares round the point, as far as the biggest feature reaches.)
        let widest = self.sim.data.features.iter().map(|d| d.footprint_x.max(d.footprint_z)).max().unwrap_or(1);
        let mut near = Vec::new();
        self.sim.features_near(fx_vec(ground), Fx::from_int(widest as i64 * 8 + 8), &mut near);
        for i in near {
            let (i, f) = (i as usize, &self.sim.features[i as usize]);
            if !f.alive {
                continue;
            }
            let def = &self.sim.data.features[f.def];
            if !def.reclaimable {
                continue;
            }
            let p = f.pos.to_f32();
            let reach = def.footprint_x.max(def.footprint_z) as f32 * 8.0 + 6.0;
            let d = ((p[0] - ground.x).powi(2) + (p[2] - ground.z).powi(2)).sqrt();
            if d <= reach && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, i));
            }
        }
        best.map(|(_, i)| i)
    }

    /// A minimap order uses the armed command when there is one, else moves the selection.
    /// Unit-only commands keep waiting for a unit; they never fall back to Move.
    pub(crate) fn order_selection_from_map(&mut self, target: Vec3, queue: bool, shift_held: bool) {
        if !self.can_order() {
            return;
        }
        if let Some(mode) = self.command_mode {
            let ground = self.cursor_ground.replace(target);
            let hovered = self.hovered.take();
            let issued = self.target_command(mode, queue);
            self.cursor_ground = ground;
            self.hovered = hovered;
            if issued {
                self.repeat_order = queue;
                if !queue {
                    self.command_mode = None;
                }
                finish_repeat_mode(&mut self.repeat_order, &mut self.command_mode, &mut self.placement, shift_held || self.demo_shift, false);
            }
        } else {
            self.move_selection(target, queue);
            if let Some(&first) = self.selection.first() {
                self.voices.push((first, ta_audio::SoundEvent::Ok));
            }
        }
    }

    /// An order to `units`: a group sent to a point under the rule "modern movement" goes as a
    /// formation (`ta_sim::formation`), facing the way it faces now (so that the places marked
    /// are where they go, whenever the order arrives), each place marked.
    fn group_order(&mut self, units: Vec<UnitId>, order: Order, queue: bool) {
        let point = ta_sim::formation::order_point(&order).filter(|_| self.sim.group_formations && units.len() > 1);
        let Some(target) = point else {
            self.issue(Command::Order { player: self.local_player, units, order, queue });
            return;
        };
        let rows = ta_sim::formation::Shape::Rows { facing: None };
        let plan = self.sim.formation_plan(self.local_player, &units, target, &rows, queue);
        for &(_, _, at) in plan.places.iter().take(32) {
            self.mark_order(&Order::Move(at), true);
        }
        let facing = (plan.places.len() <= ta_sim::formation::ROWS_MAX).then_some(plan.facing);
        let shape = ta_sim::formation::Shape::Rows { facing };
        self.issue(Command::Formation { player: self.local_player, units, order, queue, shape });
    }

    /// Moves the selection to `target`: as a formation under the rule "modern movement", else
    /// spreading units in a grid so they don't pile up.
    fn move_selection(&mut self, target: Vec3, queue: bool) {
        self.mark_order(&Order::Move(fx_vec(target)), false);
        let movers: Vec<UnitId> = self.selection.iter().copied().filter(|&id| self.sim.unit(id).is_some()).collect();
        if self.sim.group_formations && movers.len() > 1 {
            self.group_order(movers, Order::Move(fx_vec(target)), queue);
            return;
        }
        let n = movers.len();
        let cols = (n as f32).sqrt().ceil().max(1.0) as usize;
        let spacing = 40.0;
        let center = Vec3::new(target.x, 0.0, target.z);
        for (i, id) in movers.into_iter().enumerate() {
            let is_static = self.sim.unit(id).is_some_and(|u| self.sim.unit_type(u).locomotion == ta_sim::Locomotion::Static);
            let (r, c) = (i / cols, i % cols);
            let offset = if is_static || n == 1 {
                Vec3::ZERO
            } else {
                Vec3::new((c as f32 - (cols - 1) as f32 / 2.0) * spacing, 0.0, (r as f32 - ((n - 1) / cols) as f32 / 2.0) * spacing)
            };
            let p = center + offset;
            self.issue(Command::Order { player: self.local_player, units: vec![id], order: Order::Move(fx_vec(p)), queue });
        }
    }

    /// Sends the mobile units of the selection to evenly spaced points along `path`, in the
    /// order they lie along it, so their routes don't cross.
    fn formation_move(&mut self, path: &[Vec3], queue: bool) {
        let movers: Vec<(UnitId, Vec3)> = self
            .selection
            .iter()
            .copied()
            .filter_map(|id| self.sim.unit(id).filter(|u| self.sim.unit_type(u).locomotion != ta_sim::Locomotion::Static).map(|u| (id, Vec3::from_array(u.pos.to_f32()))))
            .collect();
        if movers.is_empty() {
            return;
        }
        let spots = spread_along(path, movers.len());
        if movers.len() > 1 {
            // One order: the simulation gives the points to the units in order across the way
            // they go (under the rule "modern movement", a formation that marches there).
            for spot in spots.iter().take(32) {
                self.mark_order(&Order::Move(fx_vec(*spot)), true);
            }
            let shape = ta_sim::formation::Shape::Spots(spots.iter().map(|&s| fx_vec(s)).collect());
            let units = movers.iter().map(|m| m.0).collect();
            let middle = spots.iter().fold(Vec3::ZERO, |a, &s| a + s) / spots.len() as f32;
            self.issue(Command::Formation { player: self.local_player, units, order: Order::Move(fx_vec(middle)), queue, shape });
        } else {
            for (k, ((id, _), spot)) in movers.iter().zip(spots).enumerate() {
                if k < 32 {
                    self.mark_order(&Order::Move(fx_vec(spot)), true);
                }
                self.issue(Command::Order { player: self.local_player, units: vec![*id], order: Order::Move(fx_vec(spot)), queue });
            }
        }
        if let Some(&(first, _)) = movers.first() {
            self.voices.push((first, ta_audio::SoundEvent::Ok));
        }
    }

    /// Where `builders` will start new work from: with Shift (`queued`), where the last of their
    /// orders with a place sends them; else where they stand; the middle of them all.
    fn builders_start(&self, builders: &[UnitId], queued: bool) -> Option<Vec3> {
        let places: Vec<Vec3> = builders
            .iter()
            .filter_map(|&id| self.sim.unit(id))
            .map(|u| {
                let last = queued
                    .then(|| {
                        u.orders.iter().rev().find_map(|o| match o {
                            Order::Move(p) | Order::Fight(p) | Order::Patrol(p) | Order::Unload(p) | Order::Build { pos: p, .. } => Some(*p),
                            _ => None,
                        })
                    })
                    .flatten();
                Vec3::from_array(last.unwrap_or(u.pos).to_f32())
            })
            .collect();
        (!places.is_empty()).then(|| places.iter().copied().sum::<Vec3>() / places.len() as f32)
    }

    /// The selected builders' plans (build orders queued and not yet started) whose ground a
    /// building of `type_id` put down at `pos` would cover: each builder, the order's place in
    /// its queue, and what and where the plan is.
    fn own_plans_under(&self, type_id: usize, pos: ta_sim::FxVec3) -> Vec<(UnitId, u32, usize, ta_sim::FxVec3)> {
        let r = footprint_rect(self.sim.types.get(type_id), pos);
        let mut under = Vec::new();
        for &id in &self.selection {
            let Some(u) = self.sim.unit(id).filter(|u| u.owner == self.local_player && !u.dead) else { continue };
            for (index, order) in u.orders.iter().enumerate() {
                let Order::Build { type_id: kind, pos: at } = order else { continue };
                if index == 0 && u.build.is_some() {
                    continue;
                }
                let q = footprint_rect(self.sim.types.get(*kind), *at);
                if r.0 < q.2 && q.0 < r.2 && r.1 < q.3 && q.1 < r.3 {
                    under.push((id, index as u32, *kind, *at));
                }
            }
        }
        under
    }

    /// Footprints of the buildings our constructors have queued and not yet started (a job
    /// under way has its frame on the ground already), except the plans of `replaced`.
    fn planned_footprints(&self, replaced: &[UnitId]) -> Vec<(i32, i32, i32, i32)> {
        pending_build_plans(self.sim.units(), self.local_player, replaced)
            .map(|(type_id, pos)| footprint_rect(self.sim.types.get(type_id), pos))
            .collect()
    }

    /// The overlay to show now: the player's choice, or the metal map while placing an
    /// extractor or ordering extractors over an area (unless turned off for this placing).
    pub fn shown_overlay(&self) -> Option<crate::overlay::Overlay> {
        self.overlay.or(if self.overlay_dismissed { None } else { self.auto_overlay() })
    }

    /// The overlay a placing shows by itself: the metal map for extractors.
    fn auto_overlay(&self) -> Option<crate::overlay::Overlay> {
        let extractor = self.placement.as_ref().is_some_and(|pl| self.sim.types.get(pl.type_id).extracts_metal > Fx::ZERO)
            || self.command_mode == Some(CommandMode::AreaMex);
        extractor.then_some(crate::overlay::Overlay::Metal)
    }

    /// F2, F3, F4, L and the MAP menu: an overlay on, or off. The metal map a placing shows by
    /// itself counts as on, and turning it off hides it until that placing ends.
    pub fn toggle_overlay(&mut self, o: crate::overlay::Overlay) {
        if self.shown_overlay() == Some(o) {
            self.overlay_dismissed = self.overlay.is_none() || self.auto_overlay() == Some(o);
            self.overlay = None;
        } else {
            self.overlay = Some(o);
            self.overlay_dismissed = false;
        }
    }

    /// A constructor's queued buildings (order index, type) in order.
    pub fn builder_queue(&self, builder: UnitId) -> Vec<(usize, usize)> {
        self.sim
            .unit(builder)
            .map(|u| u.orders.iter().enumerate().filter_map(|(k, o)| if let Order::Build { type_id, .. } = o { Some((k, *type_id)) } else { None }).collect())
            .unwrap_or_default()
    }

    /// Moves a constructor's order `k` (a building) to the front (`to_front`) or drops it, the
    /// rest given again in order: in the simulation, from its orders as they are when the
    /// command runs (`Command::EditQueue`).
    pub fn edit_builder_queue(&mut self, builder: UnitId, k: usize, to_front: bool) {
        let Some(u) = self.sim.unit(builder).filter(|u| u.owner == self.local_player) else { return };
        let Some(&Order::Build { type_id, pos }) = u.orders.get(k) else { return };
        self.issue(Command::EditQueue { player: self.local_player, unit: builder, index: k as u32, type_id, pos: Some(pos), to_front });
    }

    /// Narrows the selection: combat units, constructors, damaged or idle units.
    pub fn filter_selection(&mut self, keep: SelectionFilter) {
        let sim = &self.sim;
        let kept: Vec<UnitId> = self
            .selection
            .iter()
            .copied()
            .filter(|&id| {
                let Some(u) = sim.unit(id) else { return false };
                let t = sim.unit_type(u);
                match keep {
                    SelectionFilter::Combat => t.weapons.iter().any(|w| w.is_some()) && !t.is_builder,
                    SelectionFilter::Builders => t.is_builder && !t.is_factory,
                    SelectionFilter::Damaged => u.health < t.max_health,
                    SelectionFilter::Idle => u.orders.is_empty() && u.build.is_none() && !self.on_duty(u.id),
                }
            })
            .collect();
        if kept.is_empty() {
            post(&mut self.messages, &mut self.event_history, self.sim.tick, ta_ui::i18n::t("No such units in the selection").to_string(), 2.0);
        } else {
            self.selection = kept;
        }
    }

    /// An order waiting for its click (or none): a building being placed is let go, as the
    /// two would fight over the next click.
    pub fn set_command_mode(&mut self, mode: Option<CommandMode>) {
        self.repeat_order = false;
        if mode.is_some() {
            self.placement = None;
        }
        self.command_mode = mode;
    }

    /// Starts placing a building with the selected builders (an order waiting for its click is
    /// dropped: the next click places the building).
    pub fn begin_placement(&mut self, type_id: usize) {
        self.repeat_order = false;
        self.command_mode = None;
        self.placement = Some(Placement { type_id, pos: None, valid: false, matrices: Vec::new(), line_start: None, row: Vec::new(), row_matrices: Vec::new() });
    }

    /// Render instances for units, 3DO features and the placement ghost; also refreshes the
    /// feature sprites for the visible area and the 3D trees and rocks seen from `foliage`.
    /// (`view_min`..`view_max`: the view's footprint on everything up to `top` high.)
    pub fn draw_list(&mut self, renderer: &Renderer, view_min: Vec2, view_max: Vec2, top: f32, time: f32, foliage: &crate::features::FoliageView) -> Vec<DrawInstance<'_>> {
        let alpha = self.alpha();
        let selection: rustc_hash::FxHashSet<UnitId> = self.selection.iter().copied().collect();
        let hovered = self.hovered;
        let viewer = self.viewer();
        self.scene.skin_viewer = self.hud_player();
        // What the viewer remembers of the enemy's structures (those they see are remembered).
        self.memory.update(&self.sim, viewer);
        let Game { sim, scene, features, effects, placement, local_player, queued_ghosts, show_all_plans, solo, selection: selected, memory, .. } = self;
        let waiting: Vec<usize> = effects.unsettled_wrecks().collect();
        features.update(sim, time, view_min, view_max, foliage, &waiting);
        // Ghost: a fresh, unanimated instance of the building at the cursor (one per site
        // while dragging out a row).
        let mut ghost = None;
        let mut row_ghosts = Vec::new();
        if let Some(pl) = placement.as_mut() {
            let model = scene.model_for(pl.type_id);
            pl.row_matrices.resize_with(pl.row.len(), Vec::new);
            if let Some(model) = model {
                let t = sim.types.get(pl.type_id);
                for (i, &(pos, ok)) in pl.row.iter().enumerate() {
                    piece_matrices(&ghost_unit(pl.type_id, pos), t, ghost_root(sim, pl.type_id, pos), &mut pl.row_matrices[i]);
                    row_ghosts.push((model, ta_render::model::FLAG_BUILDING | if ok { 0 } else { ta_render::model::FLAG_INVALID }));
                }
            }
            if let (Some(pos), Some(model), true) = (pl.pos, model, pl.row.is_empty()) {
                let t = sim.types.get(pl.type_id);
                let dummy = ghost_unit(pl.type_id, pos);
                piece_matrices(&dummy, t, ghost_root(sim, pl.type_id, pos), &mut pl.matrices);
                let flags = ta_render::model::FLAG_BUILDING | if pl.valid { 0 } else { ta_render::model::FLAG_INVALID };
                ghost = Some((model, flags));
            }
        }
        // Buildings the selected constructors have queued: ghosts where each will stand (the
        // job under way shows its own frame).
        let queued = if placement.is_some() || *show_all_plans {
            queued_build_ghosts(pending_build_plans(sim.units(), *local_player, &[]))
        } else {
            queued_build_ghosts(pending_build_plans(selected.iter().filter_map(|&id| sim.unit(id)), *local_player, &[]))
        };
        queued_ghosts.resize_with(queued.len(), Vec::new);
        let mut queued_models = Vec::new();
        for (i, &(type_id, pos)) in queued.iter().enumerate() {
            if let Some(model) = scene.model_for(type_id) {
                piece_matrices(&ghost_unit(type_id, pos), sim.types.get(type_id), ghost_root(sim, type_id, pos), &mut queued_ghosts[i]);
                queued_models.push((i, model));
            }
        }
        // The structures remembered, in view: ghosts where they were last seen.
        let in_view = |p: Vec3, r: f32| {
            let m = r + (p.y - top).max(0.0) * 1.5 + 32.0;
            p.x >= view_min.x - m && p.x <= view_max.x + m && p.z >= view_min.y - m && p.z <= view_max.y + m
        };
        let remembered: Vec<(&crate::fog::Remembered, ta_render::ModelId)> = memory
            .ghosts()
            .filter(|r| in_view(r.pos, r.radius))
            .filter_map(|r| scene.model_for_side(r.type_id, true).map(|model| (r, model)))
            .collect();
        let solo = *solo;
        let visible = |u: &Unit| viewer.is_none_or(|p| sim.seen_by(u, p) == Seen::Visible) && solo.is_none_or(|s| s == u.id);
        let mut list = scene.instances(sim, alpha, &|id| selection.contains(&id) || hovered == Some(id), &visible, (view_min, view_max), top);
        for (i, model) in queued_models {
            // The build glow through the cloak's screen door: a faint hologram.
            let flags = ta_render::model::FLAG_BUILDING | ta_render::model::FLAG_CLOAKED;
            list.push(DrawInstance { model, pieces: &queued_ghosts[i], team: *local_player as u32, flags, build: 0.0, damage: 0.0, travel: [0.0; 2], piece: None, playing: ta_render::model::PLAYING_ALL });
        }
        for (r, model) in remembered {
            let mut flags = ta_render::model::FLAG_REMEMBERED;
            if r.build < 1.0 {
                flags |= ta_render::model::FLAG_BUILDING;
            }
            list.push(DrawInstance { model, pieces: &r.matrices, team: r.color, flags, build: r.build, damage: 0.0, travel: [0.0; 2], piece: None, playing: 0 });
        }
        list.extend(features.model_instances(sim, renderer, view_min, view_max, &waiting));
        list.extend(effects.models());
        if let (Some((model, flags)), Some(pl)) = (ghost, placement.as_ref()) {
            list.push(DrawInstance { model, pieces: &pl.matrices, team: *local_player as u32, flags, build: 0.0, damage: 0.0, travel: [0.0; 2], piece: None, playing: ta_render::model::PLAYING_ALL });
        }
        if let Some(pl) = placement.as_ref() {
            for ((model, flags), m) in row_ghosts.into_iter().zip(&pl.row_matrices) {
                list.push(DrawInstance { model, pieces: m, team: *local_player as u32, flags, build: 0.0, damage: 0.0, travel: [0.0; 2], piece: None, playing: ta_render::model::PLAYING_ALL });
            }
        }
        list
    }
}

/// Only unstarted plans occupy a future footprint. Hashing the replaced selection
/// keeps each placement frame linear in the number of units and queued orders.
fn pending_build_plans<'a>(units: impl Iterator<Item = &'a Unit> + 'a, player: u8, replaced: &[UnitId]) -> impl Iterator<Item = (usize, ta_sim::FxVec3)> + 'a {
    let replaced: rustc_hash::FxHashSet<UnitId> = replaced.iter().copied().collect();
    units.filter(move |u| u.owner == player && !u.dead && !replaced.contains(&u.id))
        .flat_map(|u| u.orders.iter().enumerate().filter_map(move |(index, order)| {
            match order {
                Order::Build { type_id, pos } if index != 0 || u.build.is_none() => Some((*type_id, *pos)),
                _ => None,
            }
        }))
}

/// First distinct holograms in planner/order order. Once every visible slot is filled,
/// remaining queues cannot affect the result and must not be scanned each render frame.
fn queued_build_ghosts(plans: impl IntoIterator<Item = (usize, ta_sim::FxVec3)>) -> Vec<(usize, Vec3)> {
    let mut queued = Vec::new();
    for (type_id, pos) in plans {
        let pos = Vec3::from_array(pos.to_f32());
        if queued.iter().any(|&(kind, point): &(usize, Vec3)| kind == type_id && point.distance_squared(pos) < 1.0) { continue; }
        queued.push((type_id, pos));
        if queued.len() == QUEUED_GHOSTS_MAX { break; }
    }
    queued
}

/// A message on the screen for `secs`; the same one still showing is kept up longer instead:
/// a dozen builders that cannot comply say so once, and alerts that come again while one
/// shows (units under attack, at a high game speed) do not stack up copies of it.
fn post(messages: &mut Vec<Notice>, history: &mut crate::battle_history::History, tick: u64, text: String, secs: f32) {
    post_as(messages, history, tick, text, secs, Tone::Note);
}

/// [`post`] in a tone of its own.
fn post_as(messages: &mut Vec<Notice>, history: &mut crate::battle_history::History, tick: u64, text: String, secs: f32, tone: Tone) {
    post_to(messages, history, tick, text, secs, tone, None);
}

/// [`post_as`] with somewhere to go when it is clicked. The same one still showing points at the
/// newest place.
fn post_to(messages: &mut Vec<Notice>, history: &mut crate::battle_history::History, tick: u64, text: String, secs: f32, tone: Tone, link: Option<Link>) {
    history.record(tick, &text);
    match messages.iter_mut().find(|m| m.text == text) {
        Some(m) => {
            m.secs = m.secs.max(secs);
            if link.is_some() {
                m.link = link;
            }
        }
        None => {
            // Longer reading time must not let an army's notices cover the whole battlefield.
            // (The oldest that is no trouble goes first.)
            if messages.len() >= 6 {
                let oldest = messages.iter().position(|m| m.tone != Tone::Trouble).unwrap_or(0);
                messages.remove(oldest);
            }
            messages.push(Notice { text, secs, tone, link });
        }
    }
}

/// How many places worth looking at are kept for the alert key.
const ALERTS_KEPT: usize = 24;
/// Presses of the alert key within this many seconds of each other step back through the places.
const ALERT_REPEAT: f32 = 8.0;
/// Alerts closer than this to the one before them are one stop (a fight), and units under attack
/// within it, or within `ATTACK_QUIET` seconds, are not told again.
const ALERT_APART: f32 = 400.0;
const ATTACK_QUIET: f32 = 30.0;
const ATTACK_FAR: f32 = 700.0;

/// Which stop the alert key goes to: the newest (0) on a first press, after a pause, or when
/// something new has happened since the last press (`newest`: when); on a press soon after the
/// last, the stop before it.
fn alert_step(press: Option<(usize, f32)>, newest: Option<f32>, now: f32) -> usize {
    match press {
        Some((stop, pressed)) if newest.is_none_or(|at| at <= pressed) && now - pressed < ALERT_REPEAT => stop + 1,
        _ => 0,
    }
}

/// The place the `step`th press of the alert key in a run looks at (the newest at 0, round again
/// after the last), and how many stops there are: the alerts newest first, those near the one
/// before them (a fight is one stop) left out.
fn nth_alert(alerts: &[(Vec3, f32)], step: usize) -> Option<(Vec3, usize)> {
    let mut stops: Vec<Vec3> = Vec::new();
    for &(at, _) in alerts.iter().rev() {
        if stops.last().is_none_or(|&last| last.distance(at) > ALERT_APART) {
            stops.push(at);
        }
    }
    let stop = *stops.get(step % stops.len().max(1))?;
    Some((stop, stops.len()))
}

/// Whether an attack at `at` is news: not when another was told lately, and near it. Keeps where
/// and when it was told when it is.
fn attack_is_news(last: &mut Option<(Vec3, f32)>, at: Vec3, now: f32) -> bool {
    let news = last.is_none_or(|(place, told)| now - told > ATTACK_QUIET || place.distance(at) > ATTACK_FAR);
    if news {
        *last = Some((at, now));
    }
    news
}

/// Where a notice or a piece of advice points: a place to look at, or one of our units to select
/// and look at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Link {
    Place(Vec3),
    Unit(UnitId),
}

/// A message under the top bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub text: String,
    /// Reading time left (seconds).
    pub secs: f32,
    pub tone: Tone,
    /// Where a click on it goes, if it points somewhere.
    pub link: Option<Link>,
}

/// How a message under the top bar reads: its colour, and how long it is up for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Something to know (amber).
    Note,
    /// Something done (green, briefly).
    Done,
    /// Something gone wrong, refused or in danger (red, and longer).
    Trouble,
}

/// Count in reading time, not simulation ticks. Pausing gives the player time to read;
/// changing the multiplier also applies to notices already on screen.
fn expire_messages(messages: &mut Vec<Notice>, dt: f32, duration: f32, paused: bool) {
    if paused {
        return;
    }
    let elapsed = dt.max(0.0) / duration.max(1.0).min(4.0);
    for message in messages.iter_mut() {
        message.secs -= elapsed;
    }
    messages.retain(|m| m.secs > 0.0);
}

/// Visits `stops` nearest-first (a greedy tour from `from`), up to `max` of them.
fn nearest_tour<T>(from: Vec3, mut stops: Vec<(Vec3, T)>, max: usize) -> Vec<T> {
    let mut at = from;
    let mut tour = Vec::new();
    while tour.len() < max {
        let Some((k, _)) = stops.iter().enumerate().map(|(k, (p, _))| (k, p.distance_squared(at))).min_by(|a, b| a.1.total_cmp(&b.1)) else { break };
        let (p, stop) = stops.swap_remove(k);
        at = p;
        tour.push(stop);
    }
    tour
}

/// Total length of a polyline.
fn polyline_length(path: &[Vec3]) -> f32 {
    path.windows(2).map(|w| w[0].distance(w[1])).sum()
}

/// `n` points spread evenly along a polyline, ends included (one point: the middle).
pub fn spread_along(path: &[Vec3], n: usize) -> Vec<Vec3> {
    let total = polyline_length(path);
    if path.len() < 2 || n == 0 || total <= 0.0 {
        return vec![path.first().copied().unwrap_or(Vec3::ZERO); n];
    }
    let at = |d: f32| {
        let mut left = d;
        for w in path.windows(2) {
            let seg = w[0].distance(w[1]);
            if left <= seg && seg > 0.0 {
                return w[0].lerp(w[1], left / seg);
            }
            left -= seg;
        }
        path[path.len() - 1]
    };
    if n == 1 {
        return vec![at(total / 2.0)];
    }
    (0..n).map(|i| at(total * i as f32 / (n - 1) as f32)).collect()
}

/// Points from `a` to `b` one footprint apart along the drag's main axis (none when the drag
/// is shorter than a footprint), at most [`ROW_MAX`].
/// Sites filling the rectangle between corners `a` and `b`, one footprint apart (at most
/// `ROW_MAX` of them).
/// Conquest: the veterans that came along stand in rings around the Commander, on ground
/// they can stand on (aircraft anywhere). Deterministic: part of making the game.
fn deploy_veterans(sim: &mut Sim, owner: u8, start: FxVec3, veterans: &[(String, Fx)]) -> Vec<crate::conquest::Veteran> {
    let mut undeployed = Vec::new();
    for (name, xp) in veterans {
        let Some(t) = sim.types.id(name) else {
            log::warn!("conquest: unknown veteran type {name}");
            undeployed.push(crate::conquest::Veteran { unit: name.clone(), xp: *xp });
            continue;
        };
        if sim.types.get(t).locomotion == ta_sim::Locomotion::Static || sim.types.get(t).max_speed <= Fx::ZERO || sim.types.get(t).def.is_commander {
            undeployed.push(crate::conquest::Veteran { unit: name.clone(), xp: *xp });
            continue;
        }
        let air = sim.types.get(t).locomotion == ta_sim::Locomotion::Air;
        let radius = sim.types.get(t).radius;
        let spec = sim.move_spec(t);
        // Search independently for each type: an undeployable ship must not push
        // the next land unit's first probe beyond the map. Occupied sites stay taken.
        let spot = (0..64).find_map(|k| {
            let ring = 1 + k / 8;
            let angle = (k % 8) * 8192 + ring * 3000;
            let r = Fx::from_int(64 + ring as i64 * 72);
            let p = start + FxVec3::new(ta_sim::fixed::cos(angle) * r, Fx::ZERO, ta_sim::fixed::sin(angle) * r);
            let (w, h) = sim.world.size_px();
            let inside = p.x > Fx::from_int(32) && p.z > Fx::from_int(32) && p.x < w - Fx::from_int(32) && p.z < h - Fx::from_int(32);
            let clear = !sim.units().any(|u| {
                let other = sim.unit_type(u);
                let clearance = (radius + other.radius + Fx::from_int(8)).0 as i128;
                !u.dead && u.carried_by.is_none() && (other.locomotion == ta_sim::Locomotion::Air) == air
                    && (u.pos - p).xz().length_sq_raw() < clearance * clearance
            });
            (inside && clear && (air || sim.world.passable(&spec, ta_sim::world::World::pos_to_anchor(p, spec.footprint), None))).then_some(p)
        });
        if let Some(id) = spot.and_then(|p| sim.spawn_unit(t, owner, p, 0, true)) {
            sim.set_experience(id, *xp);
        } else {
            undeployed.push(crate::conquest::Veteran { unit: name.clone(), xp: *xp });
        }
    }
    undeployed
}

fn grid_points(a: Vec3, b: Vec3, footprint: Vec2) -> Vec<Vec3> {
    let d = b - a;
    let (nx, nz) = ((d.x.abs() / footprint.x.max(1.0)).floor() as i32, (d.z.abs() / footprint.y.max(1.0)).floor() as i32);
    if nx == 0 && nz == 0 {
        return Vec::new();
    }
    let (sx, sz) = (footprint.x * d.x.signum(), footprint.y * d.z.signum());
    let mut out = Vec::new();
    for j in 0..=nz {
        for i in 0..=nx {
            if out.len() < ROW_MAX {
                out.push(Vec3::new(a.x + sx * i as f32, a.y, a.z + sz * j as f32));
            }
        }
    }
    out
}

/// A filled rectangle's sites in the order to build them from `at`: the farthest first (on the
/// ground: across), so that builders working from `at` stand in what is still open and finish
/// at its edge nearest them.
fn far_side_first(sites: &mut [Vec3], at: Vec3) {
    let across = |p: &Vec3| (p.x - at.x).powi(2) + (p.z - at.z).powi(2);
    sites.sort_by(|a, b| across(b).total_cmp(&across(a)));
}

fn row_points(a: Vec3, b: Vec3, footprint: Vec2) -> Vec<Vec3> {
    let d = b - a;
    let steps = (d.x.abs() / footprint.x.max(1.0)).max(d.z.abs() / footprint.y.max(1.0)).floor() as usize;
    if steps == 0 {
        return Vec::new();
    }
    let steps = steps.min(ROW_MAX - 1);
    (0..=steps).map(|i| a + d * (i as f32 / steps as f32)).collect()
}

/// Chat lines starting with this carry a map ping: "player x z" (a private-use character,
/// which survives the network's stripping of control characters).
const PING_PREFIX: &str = ta_net::PING_PREFIX;
/// Chat lines for the sender's allies only: "player text". The host authenticates the
/// sender and routes them only to the sender's teammates and spectators.
pub(crate) const ALLY_PREFIX: &str = ta_net::ALLY_PREFIX;

pub fn fx_vec(v: Vec3) -> FxVec3 {
    FxVec3::new(Fx::from_f32(v.x), Fx::from_f32(v.y), Fx::from_f32(v.z))
}

/// A throwaway unit used only to evaluate a rest-pose model for the placement ghost.
/// The cells a building of type `t` centred at `pos` covers: (x0, z0, x1, z1), exclusive.
fn footprint_rect(t: &ta_sim::UnitType, pos: ta_sim::FxVec3) -> (i32, i32, i32, i32) {
    use ta_sim::world::World;
    let (x0, z0) = World::footprint_origin(World::pos_to_anchor(pos, t.footprint), t.footprint);
    (x0, z0, x0 + t.footprint.0 as i32, z0 + t.footprint.1 as i32)
}

/// Where a building's ghost is drawn from: where the building will stand once put down there
/// (`Sim::resting_pos`: a shipyard at the surface, not on the sea floor), its model as the
/// scene draws a unit's (`Unit::model_pos`).
fn ghost_root(sim: &Sim, type_id: usize, pos: Vec3) -> Mat4 {
    let rest = sim.resting_pos(type_id, fx_vec(pos));
    let y = (rest.y - sim.types.get(type_id).waterline).to_f32();
    Mat4::from_translation(Vec3::new(rest.x.to_f32(), y, rest.z.to_f32()))
}

fn ghost_unit(type_id: usize, pos: Vec3) -> ta_sim::Unit {
    let mut u = ta_sim::Unit::new(UnitId { slot: u32::MAX, generation: 0 }, type_id, 0, fx_vec(pos));
    u.build_progress = Fx::ZERO;
    u
}


/// Why the simulation refused an order (`SimEvent::OrderFailed`), for the translations: shown
/// through `t`.
#[allow(dead_code)]
const ORDER_FAILURES: [&str; 8] = [
    ta_ui::i18n::n("cannot carry that"),
    ta_ui::i18n::n("out of reach"),
    ta_ui::i18n::n("no room to unload"),
    ta_ui::i18n::n("unreachable"),
    ta_ui::i18n::n("can't reach the build site"),
    ta_ui::i18n::n("build site blocked"),
    ta_ui::i18n::n("too near an enemy base in the ceasefire"),
    ta_ui::i18n::n("no captures in the ceasefire"),
];

/// TA's selections by category: the key's action and the category it selects.
const SELECT_CATEGORIES: [(&str, &str); 7] = [
    ("select_builders", "CTRL_B"),
    ("select_factories", "CTRL_F"),
    ("select_mines", "CTRL_M"),
    ("select_armed_aircraft", "CTRL_P"),
    ("select_radar", "CTRL_R"),
    ("select_aircraft", "CTRL_V"),
    ("select_armed", "CTRL_W"),
];

const LETTERS: [KeyCode; 26] = [
    KeyCode::KeyA,
    KeyCode::KeyB,
    KeyCode::KeyC,
    KeyCode::KeyD,
    KeyCode::KeyE,
    KeyCode::KeyF,
    KeyCode::KeyG,
    KeyCode::KeyH,
    KeyCode::KeyI,
    KeyCode::KeyJ,
    KeyCode::KeyK,
    KeyCode::KeyL,
    KeyCode::KeyM,
    KeyCode::KeyN,
    KeyCode::KeyO,
    KeyCode::KeyP,
    KeyCode::KeyQ,
    KeyCode::KeyR,
    KeyCode::KeyS,
    KeyCode::KeyT,
    KeyCode::KeyU,
    KeyCode::KeyV,
    KeyCode::KeyW,
    KeyCode::KeyX,
    KeyCode::KeyY,
    KeyCode::KeyZ,
];

/// A fresh armed tool survives releasing Shift before its first target. After a queued
/// target, releasing Shift returns to selection, but a pending queued click must run first.
fn finish_repeat_mode(repeated: &mut bool, mode: &mut Option<CommandMode>, placement: &mut Option<Placement>, shift: bool, queued_release: bool) {
    if *repeated && !shift && !queued_release {
        *mode = None;
        *placement = None;
        *repeated = false;
    }
}

/// A point order must remain a point order even if the pointer moves while clicking.
/// Only the ordinary selection tool draws a selection box; area orders and placements
/// already have their own gestures.
fn selection_drag(dragged: bool, mode: Option<CommandMode>, placing: bool) -> bool {
    dragged && mode.is_none() && !placing
}

/// Whether a unit's category (TA's space-separated words) has `word` in it, in any case.
pub(crate) fn has_category(category: &str, word: &str) -> bool {
    category.split_whitespace().any(|w| w.trim_end_matches(';').eq_ignore_ascii_case(word))
}

/// Whether units of type `t` fire on a plain attack order (see [`Game::fires_on_attack`]).
fn fires_on_attack(sim: &Sim, t: &ta_sim::UnitType) -> bool {
    t.weapons.iter().flatten().any(|&w| {
        let w = sim.weapon_types.get(w);
        !w.def.interceptor && (!t.is_factory || !w.command_fire || w.def.stockpile)
    })
}

fn has_dgun(sim: &Sim, t: &ta_sim::UnitType) -> bool {
    t.def.can_dgun && t.weapons.iter().flatten().any(|&w| {
        let w = sim.weapon_types.get(w);
        w.command_fire && !w.def.stockpile && !w.def.interceptor
    })
}

/// Type capabilities shared by command availability and dispatch. Unload still
/// needs actual cargo, and area extraction needs a permitted build option.
fn type_supports_mode(sim: &Sim, t: &ta_sim::UnitType, mode: CommandMode) -> bool {
    match mode {
        CommandMode::Attack => fires_on_attack(sim, t),
        CommandMode::DGun => has_dgun(sim, t),
        // Factories give these orders to the units they release.
        CommandMode::Fight | CommandMode::Patrol | CommandMode::Move => t.locomotion != ta_sim::Locomotion::Static || t.is_factory,
        CommandMode::Guard => t.locomotion != ta_sim::Locomotion::Static || t.is_builder,
        CommandMode::Repair => t.is_builder && !t.is_factory,
        CommandMode::Load | CommandMode::Unload => t.transport_capacity > 0,
        CommandMode::Capture => t.def.can_capture,
        CommandMode::Resurrect => t.def.can_resurrect,
        CommandMode::Reclaim => t.def.can_reclaim,
        CommandMode::AreaMex => t.build_options.iter().any(|&o| {
            let o = sim.types.get(o);
            o.extracts_metal > Fx::ZERO && !o.needs_geo && sim.is_allowed(o.id)
        }),
    }
}

mod clicks;
pub(crate) mod nuclear;
pub mod duty;

#[cfg(all(test, target_os = "windows"))]
mod input_tests;

#[cfg(test)]
mod replay_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// A player beaten in a battle that goes on, who chose KEEP WATCHING, is a spectator from
    /// then on: leaving, their battle's replay was thrown away (the spectator flag stood for
    /// the menu's film). Whose battle it is decides, as the setup has it; a multiplayer battle
    /// is a real one whoever watches it (a spectator from its start has no seat of ours).
    #[test]
    fn a_battle_lost_and_watched_on_keeps_its_replay() {
        let minute = 30 * 60;
        assert!(keeps_replay(Some(0), false, false, false, minute), "our battle, watched on or not");
        assert!(keeps_replay(Some(2), true, false, false, 30 * 20), "twenty seconds is enough");
        assert!(keeps_replay(None, true, false, false, minute), "a multiplayer battle a spectator watched");
        assert!(!keeps_replay(None, false, false, false, minute), "the menu's film, or a single-player battle watched from its start");
        assert!(!keeps_replay(Some(0), false, true, false, minute), "a replay played back");
        assert!(!keeps_replay(Some(0), false, false, true, minute), "a lesson");
        assert!(!keeps_replay(None, true, false, false, 30 * 20 - 1), "a few seconds");
    }

    #[test]
    fn undeployable_veterans_do_not_consume_other_units_landing_sites() {
        let mut data = ta_data::GameData::load(&ta_vfs::Vfs::new());
        for (name, ship) in [("LAND", false), ("SHIP", true)] {
            data.units.push(ta_data::unit::UnitDef {
                name: name.into(), max_damage: 100.0, can_move: true, max_velocity: 1.0,
                footprint_x: 2, footprint_z: 2, max_slope: 90.0, floater: ship,
                min_water_depth: if ship { 10.0 } else { 0.0 }, max_water_depth: if ship { 1000.0 } else { 0.0 },
                ..Default::default()
            });
        }
        let types = std::sync::Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
        let data = std::sync::Arc::new(data);
        let map = ta_formats::tnt::TntMap { width: 32, height: 32, sea_level: 0,
            heights: vec![60; 32 * 32], features: vec![ta_formats::tnt::FEATURE_NONE; 32 * 32],
            tiles: vec![], tile_map: vec![], feature_names: vec![], minimap: Default::default() };
        let start = FxVec3::from_ints(256, 60, 256);
        let player = ta_sim::PlayerSetup { side: "ARM".into(), color: 0, start, metal: Fx::ZERO, energy: Fx::ZERO, ai: false };
        let make = || Sim::new(types.clone(), data.clone(), &map, &ta_data::MapInfo::default(), 0, std::slice::from_ref(&player), 7);
        let veterans = vec![("SHIP".into(), Fx::from_int(8)), ("LAND".into(), Fx::from_int(5)), ("MISSING".into(), Fx::from_int(3)), ("LAND".into(), Fx::from_int(2))];
        let mut sim = make();
        let left = deploy_veterans(&mut sim, 0, start, &veterans);
        assert_eq!(left.iter().map(|v| v.unit.as_str()).collect::<Vec<_>>(), ["SHIP", "MISSING"]);
        let standing: Vec<_> = sim.units().map(|u| (u.pos, u.xp)).collect();
        assert_eq!(standing.len(), 2, "a ship without a landing site cannot block later land veterans");
        assert_eq!((standing[0].1, standing[1].1), (Fx::from_int(5), Fx::from_int(2)));
        assert!((standing[0].0 - standing[1].0).xz().length() >= Fx::from_int(40), "veterans do not overlap");
        let mut second = make();
        assert_eq!(deploy_veterans(&mut second, 0, start, &veterans), left);
        assert_eq!(second.units().map(|u| (u.pos, u.xp)).collect::<Vec<_>>(), standing, "save/replay reconstruction is deterministic");
    }

    #[test]
    fn construction_preview_keeps_unstarted_owned_plans_and_replaced_generations() {
        let id = UnitId { slot: 5, generation: 2 };
        let mut builder = Unit::new(id, 0, 0, ta_sim::FxVec3::ZERO);
        builder.orders = [
            Order::Build { type_id: 1, pos: ta_sim::FxVec3::from_ints(32, 0, 64) },
            Order::Move(ta_sim::FxVec3::ZERO),
            Order::Build { type_id: 2, pos: ta_sim::FxVec3::from_ints(64, 0, 64) },
        ].into();
        let mut enemy = builder.clone(); enemy.owner = 1;
        let mut dead = builder.clone(); dead.dead = true;
        let units = [builder.clone(), enemy, dead];
        let plans = |replaced: &[UnitId]| pending_build_plans(units.iter(), 0, replaced).collect::<Vec<_>>();
        assert_eq!(plans(&[]).len(), 2, "enemy and dead plans do not reserve ground");
        assert!(plans(&[id, id]).is_empty(), "all plans of a replaced constructor are removed");
        assert_eq!(plans(&[UnitId { generation: 1, ..id }]).len(), 2, "reused slots cannot discard a different unit's plans");
        builder.build = Some(ta_sim::unit::BuildJob { target: UnitId { slot: 9, generation: 0 }, started: true });
        let plans: Vec<_> = pending_build_plans(std::iter::once(&builder), 0, &[]).collect();
        assert_eq!(plans, [(2, ta_sim::FxVec3::from_ints(64, 0, 64))], "the started frame reserves its ground through the simulation instead");
    }

    #[test]
    fn queued_holograms_preserve_first_distinct_sites_and_stop_when_full() {
        let a = ta_sim::FxVec3::from_ints(32, 0, 64);
        let near = a + ta_sim::FxVec3::new(Fx::from_f32(0.25), Fx::ZERO, Fx::ZERO);
        assert_eq!(queued_build_ghosts([(1, a), (1, near), (2, a)]),
            [(1, Vec3::from_array(a.to_f32())), (2, Vec3::from_array(a.to_f32()))], "near duplicates merge only within the same building type");
        let visits = std::cell::Cell::new(0);
        let plans = (0..1_000_000).map(|i| {
            visits.set(visits.get() + 1);
            (1, ta_sim::FxVec3::from_ints(i * 32, 0, 64))
        });
        let queued = queued_build_ghosts(plans);
        assert_eq!(queued.len(), QUEUED_GHOSTS_MAX);
        assert_eq!(queued[0].1, Vec3::new(0.0, 0.0, 64.0));
        assert_eq!(queued.last().unwrap().1, Vec3::new((QUEUED_GHOSTS_MAX - 1) as f32 * 32.0, 0.0, 64.0));
        assert_eq!(visits.get(), QUEUED_GHOSTS_MAX, "the unused tail must never be evaluated each frame");
    }

    /// Run explicitly for comparative timing; default tests assert bounded work instead
    /// of making hardware-dependent frame-time assertions.
    #[test]
    #[ignore = "manual construction-preview performance measurement"]
    fn construction_preview_benchmark() {
        use std::{hint::black_box, time::Instant};
        let plans: Vec<_> = (0..50_000).map(|i| (1, ta_sim::FxVec3::from_ints(i * 32, 0, 64))).collect();
        let baseline = |plans: &[(usize, ta_sim::FxVec3)]| {
            let mut queued: Vec<(usize, Vec3)> = Vec::new();
            for &(kind, pos) in plans {
                let pos = Vec3::from_array(pos.to_f32());
                if queued.iter().any(|&(k, p)| k == kind && p.distance_squared(pos) < 1.0) { continue; }
                if queued.len() < QUEUED_GHOSTS_MAX { queued.push((kind, pos)); }
            }
            queued
        };
        assert_eq!(baseline(&plans), queued_build_ghosts(plans.iter().copied()));
        let mut times = [Vec::new(), Vec::new()];
        for round in 0..9 {
            for index in [round % 2, 1 - round % 2] {
                let start = Instant::now();
                for _ in 0..10 {
                    if index == 0 { black_box(baseline(black_box(&plans))); }
                    else { black_box(queued_build_ghosts(black_box(&plans).iter().copied())); }
                }
                times[index].push(start.elapsed().as_secs_f64() * 1e6 / 10.0);
            }
        }
        for samples in &mut times { samples.sort_by(f64::total_cmp); }
        println!("construction ghosts, 50000 plans: baseline {:.2} us/frame, bounded {:.2} us/frame", times[0][4], times[1][4]);

        let units: Vec<_> = (0..10_000).map(|slot| {
            let mut unit = Unit::new(UnitId { slot, generation: 1 }, 0, 0, ta_sim::FxVec3::ZERO);
            unit.orders.push_back(Order::Build { type_id: 1, pos: ta_sim::FxVec3::from_ints(i64::from(slot) * 32, 0, 64) });
            unit
        }).collect();
        let replaced: Vec<_> = units.iter().step_by(2).map(|u| u.id).collect();
        let baseline = |units: &[Unit], replaced: &[UnitId]| {
            units.iter().filter(|u| u.owner == 0 && !u.dead && !replaced.contains(&u.id))
                .flat_map(|u| u.orders.iter().enumerate().filter_map(move |(index, order)| match order {
                    Order::Build { type_id, pos } if index != 0 || u.build.is_none() => Some((*type_id, *pos)),
                    _ => None,
                })).collect::<Vec<_>>()
        };
        assert_eq!(baseline(&units, &replaced), pending_build_plans(units.iter(), 0, &replaced).collect::<Vec<_>>());
        let mut times = [Vec::new(), Vec::new()];
        for round in 0..9 {
            for index in [round % 2, 1 - round % 2] {
                let start = Instant::now();
                for _ in 0..10 {
                    if index == 0 { black_box(baseline(black_box(&units), black_box(&replaced))); }
                    else { black_box(pending_build_plans(black_box(&units).iter(), 0, black_box(&replaced)).collect::<Vec<_>>()); }
                }
                times[index].push(start.elapsed().as_secs_f64() * 1e6 / 10.0);
            }
        }
        for samples in &mut times { samples.sort_by(f64::total_cmp); }
        println!("planned footprints, 10000 builders / 5000 replaced: baseline {:.2} us/frame, hashed {:.2} us/frame", times[0][4], times[1][4]);
    }

    #[test]
    fn releasing_shift_finishes_repeated_orders_after_dispatching_the_last_click() {
        let mut repeated = false;
        let mut mode = Some(CommandMode::Patrol);
        let mut placement = None;
        finish_repeat_mode(&mut repeated, &mut mode, &mut placement, false, false);
        assert_eq!(mode, Some(CommandMode::Patrol), "a fresh Patrol choice waits for its first target");
        repeated = true; // first Shift+click was dispatched
        finish_repeat_mode(&mut repeated, &mut mode, &mut placement, true, false);
        assert_eq!(mode, Some(CommandMode::Patrol), "holding Shift keeps adding waypoints");
        finish_repeat_mode(&mut repeated, &mut mode, &mut placement, false, true);
        assert_eq!(mode, Some(CommandMode::Patrol), "a last Shift+click queued before redraw must dispatch before cleanup");
        finish_repeat_mode(&mut repeated, &mut mode, &mut placement, false, false);
        assert!(mode.is_none() && !repeated, "the next plain click selects rather than replacing the patrol");
    }

    #[test]
    fn releasing_shift_finishes_build_repetition_but_preserves_a_fresh_build_choice() {
        let mut repeated = false;
        let mut mode = None;
        let mut placement = Some(Placement {
            type_id: 7, pos: Some(Vec3::X), valid: true, matrices: Vec::new(),
            line_start: None, row: Vec::new(), row_matrices: Vec::new(),
        });
        finish_repeat_mode(&mut repeated, &mut mode, &mut placement, false, false);
        assert!(placement.is_some(), "opening a build tool with Shift held still permits its first placement");
        repeated = true;
        finish_repeat_mode(&mut repeated, &mut mode, &mut placement, true, false);
        assert!(placement.is_some());
        finish_repeat_mode(&mut repeated, &mut mode, &mut placement, false, false);
        assert!(placement.is_none() && !repeated, "releasing Shift exits repeated placement without an extra build click");
    }

    #[test]
    fn dragging_an_armed_patrol_or_other_order_never_becomes_box_selection() {
        for mode in [
            CommandMode::Patrol, CommandMode::Fight, CommandMode::Move, CommandMode::Guard,
            CommandMode::Repair, CommandMode::Load, CommandMode::Unload, CommandMode::DGun,
            CommandMode::Attack, CommandMode::Capture, CommandMode::Resurrect,
            CommandMode::Reclaim, CommandMode::AreaMex,
        ] {
            assert!(!selection_drag(true, Some(mode), false), "{mode:?} must keep its target gesture after the pointer moves");
        }
        assert!(selection_drag(true, None, false), "normal drag selection still works");
        assert!(!selection_drag(false, None, false), "a click does not select a box");
        assert!(!selection_drag(true, None, true), "dragging a build row is placement");
    }

    #[test]
    fn battle_notices_allow_more_reading_time_and_hold_while_paused() {
        let mut messages = Vec::new();
        let mut history = crate::battle_history::History::default();
        post(&mut messages, &mut history, 0, "attack".into(), 3.0);
        expire_messages(&mut messages, 2.0, 2.0, false);
        assert_eq!(messages[0].secs, 2.0);
        expire_messages(&mut messages, 20.0, 2.0, true);
        assert_eq!(messages[0].secs, 2.0, "pausing preserves the notice");
        expire_messages(&mut messages, 2.0, 1.0, false);
        assert!(messages.is_empty(), "the current reading setting applies immediately");
        assert_eq!(history.latest().unwrap().text, "attack", "expired notices remain in history");
    }

    #[test]
    fn repeated_battle_notices_refresh_without_crowding_out_the_battlefield() {
        let mut messages = Vec::new();
        let mut history = crate::battle_history::History::default();
        for index in 0..8 {
            post(&mut messages, &mut history, 0, index.to_string(), 3.0);
        }
        assert_eq!(messages.len(), 6);
        assert_eq!(messages.first().unwrap().text, "2");
        post(&mut messages, &mut history, 0, "7".into(), 6.0);
        assert_eq!(messages.len(), 6);
        assert_eq!(messages.last().unwrap().secs, 6.0);
        expire_messages(&mut messages, 3.0, 0.0, false);
        assert_eq!(messages, vec![Notice { text: "7".into(), secs: 3.0, tone: Tone::Note, link: None }], "invalid short duration keeps the standard lifetime");
        // Trouble outstays notes: with the column full, the oldest that is no trouble goes.
        post_as(&mut messages, &mut history, 0, "refused".into(), 6.0, Tone::Trouble);
        for index in 0..6 {
            post(&mut messages, &mut history, 0, format!("note {index}"), 3.0);
        }
        assert_eq!(messages.len(), 6);
        assert_eq!((messages[0].text.as_str(), messages[0].tone), ("refused", Tone::Trouble));
    }

    /// A notice that points somewhere keeps pointing at the newest place when it comes again,
    /// and one that comes again without a place does not lose the one it had.
    #[test]
    fn a_notice_that_points_somewhere_points_at_the_newest_place() {
        let mut messages = Vec::new();
        let mut history = crate::battle_history::History::default();
        let (here, there) = (Link::Place(Vec3::new(10.0, 0.0, 20.0)), Link::Place(Vec3::new(900.0, 0.0, 40.0)));
        post_to(&mut messages, &mut history, 0, "attack".into(), 3.0, Tone::Note, Some(here));
        post_to(&mut messages, &mut history, 30, "attack".into(), 3.0, Tone::Note, Some(there));
        assert_eq!(messages.len(), 1, "one notice, not two");
        assert_eq!(messages[0].link, Some(there));
        post(&mut messages, &mut history, 60, "attack".into(), 3.0);
        assert_eq!(messages[0].link, Some(there), "told again without a place: the place stays");
        post(&mut messages, &mut history, 60, "other".into(), 3.0);
        assert_eq!(messages[1].link, None);
    }

    /// The alert key: the newest place first, then, pressed again, the ones before it, a fight
    /// being one stop however many times it was told, round to the newest again.
    #[test]
    fn the_alert_key_steps_back_through_the_places() {
        let at = |x: f32, z: f32| Vec3::new(x, 0.0, z);
        let alerts = vec![(at(100.0, 100.0), 10.0), (at(2000.0, 500.0), 40.0), (at(2010.0, 480.0), 50.0), (at(2030.0, 520.0), 60.0), (at(100.0, 160.0), 90.0)];
        let (newest, stops) = nth_alert(&alerts, 0).unwrap();
        assert_eq!((newest, stops), (at(100.0, 160.0), 3), "the fight at 2000 is one stop, and the first place is another");
        assert_eq!(nth_alert(&alerts, 1).unwrap().0, at(2030.0, 520.0), "the fight, at its latest");
        assert_eq!(nth_alert(&alerts, 2).unwrap().0, at(100.0, 100.0));
        assert_eq!(nth_alert(&alerts, 3).unwrap().0, at(100.0, 160.0), "round again");
        assert!(nth_alert(&[], 0).is_none());
    }

    /// Pressed again soon after, the alert key steps back; a pause, or something new, starts
    /// again at the newest.
    #[test]
    fn pressing_the_alert_key_again_steps_back_until_something_new_happens() {
        assert_eq!(alert_step(None, Some(10.0), 20.0), 0, "the first press: the newest");
        assert_eq!(alert_step(Some((0, 20.0)), Some(10.0), 22.0), 1, "again soon after: the one before");
        assert_eq!(alert_step(Some((1, 22.0)), Some(10.0), 23.0), 2);
        assert_eq!(alert_step(Some((1, 22.0)), Some(10.0), 22.0 + ALERT_REPEAT + 0.5), 0, "after a pause: the newest again");
        assert_eq!(alert_step(Some((1, 22.0)), Some(30.0), 23.0), 0, "something new since: the newest");
        assert_eq!(alert_step(Some((1, 22.0)), None, 23.0), 2);
    }

    /// A long fight is told once, then again when it has gone on a while or is somewhere else.
    #[test]
    fn a_long_fight_is_told_once() {
        let mut told = None;
        let (a, b) = (Vec3::new(100.0, 0.0, 100.0), Vec3::new(2000.0, 0.0, 100.0));
        assert!(attack_is_news(&mut told, a, 10.0), "the first is news");
        assert!(!attack_is_news(&mut told, a + Vec3::X * 50.0, 20.0), "the same fight ten seconds on is not");
        assert!(attack_is_news(&mut told, b, 21.0), "a fight far away is");
        assert!(!attack_is_news(&mut told, b, 40.0));
        assert!(attack_is_news(&mut told, b, 52.0), "and one that has gone on for a while is told again");
    }

    #[test]
    fn cancelled_placement_drag_keeps_the_build_choice_without_stale_sites() {
        let mut placement = Placement {
            type_id: 7, pos: Some(Vec3::X), valid: true, matrices: vec![Mat4::IDENTITY],
            line_start: Some(Vec3::ZERO), row: vec![(Vec3::X, true), (Vec3::Z, false)],
            row_matrices: vec![vec![Mat4::IDENTITY]; 2],
        };
        placement.cancel_drag();
        assert_eq!(placement.type_id, 7);
        assert!(placement.line_start.is_none() && placement.pos.is_none());
        assert!(!placement.valid);
        assert!(placement.row.is_empty() && placement.row_matrices.is_empty() && placement.matrices.is_empty());
    }

    #[test]
    fn replay_ends_at_its_recorded_tick_even_after_a_silent_tail() {
        let mut playback = Playback { ticks: Default::default(), end_tick: 50, full_commands: true, resume: false, source: None, seek_to: None, duties: Vec::new() };
        assert!(!playback.finished(0), "an empty command history can still contain a battle");
        assert!(!playback.finished(49), "the last recorded tick must run");
        assert!(playback.finished(50), "the next step in the same fast frame must stop");
        assert!(playback.finished(51));
        assert!(!playback.finished(20), "seeking backwards makes playback runnable again");
        playback.end_tick = 0;
        assert!(playback.finished(0), "a recording made before the first tick stays there");
        playback.resume = true;
        assert!(!playback.finished(50), "a loaded save continues as a live game");
    }

    #[test]
    fn thrown_pieces_keep_unit_generations_and_exclude_bitmap_only_effects() {
        let old = UnitId { slot: 7, generation: 1 };
        let new = UnitId { slot: 7, generation: 2 };
        let other = UnitId { slot: 8, generation: 1 };
        let piece = |unit, model_piece, flags| SimEvent::PieceExplode { unit, type_id: 0, model_piece, pos: FxVec3::ZERO, heading: 0, flags };
        let events = [
            piece(old, 3, 4),
            piece(new, 9, 16),
            SimEvent::UnitDestroyed { id: old, type_id: 0, owner: 0, pos: FxVec3::ZERO, killer: None, removed: false, complete: true },
            piece(old, 4, 0),
            piece(new, 10, 32),
            piece(other, 2, 33),
        ];
        let thrown = thrown_pieces_by_unit(&events);
        assert_eq!(thrown.len(), 2);
        assert_eq!(thrown[&old], [3, 4]);
        assert_eq!(thrown[&new], [9]);
        assert!(!thrown.contains_key(&other));
        assert!(thrown_pieces_by_unit(&[]).is_empty());
    }

    #[test]
    fn a_wreck_is_known_by_the_unit_whose_death_left_it() {
        let (dead, other) = (UnitId { slot: 3, generation: 2 }, UnitId { slot: 4, generation: 1 });
        let events = [
            SimEvent::FeatureCreated { index: 40, wreck_of: None },
            SimEvent::FeatureCreated { index: 41, wreck_of: Some(dead) },
            SimEvent::UnitDestroyed { id: dead, type_id: 0, owner: 1, pos: FxVec3::ZERO, killer: None, removed: false, complete: true },
            SimEvent::UnitDestroyed { id: other, type_id: 0, owner: 1, pos: FxVec3::ZERO, killer: None, removed: false, complete: true },
        ];
        let wrecks = wrecks_by_unit(&events);
        assert_eq!(wrecks.len(), 1, "a tree's heap or a placed rock is nobody's wreck");
        assert_eq!(wrecks[&dead], 41);
        assert!(!wrecks.contains_key(&other), "blown to nothing");
    }

    #[test]
    fn rows_space_buildings_one_footprint_apart() {
        let fp = Vec2::new(32.0, 32.0);
        assert!(row_points(Vec3::ZERO, Vec3::new(20.0, 0.0, 5.0), fp).is_empty(), "a click is not a row");
        let east = row_points(Vec3::ZERO, Vec3::new(100.0, 0.0, 10.0), fp);
        assert_eq!(east.len(), 4);
        for w in east.windows(2) {
            assert!((w[1].x - w[0].x) >= 32.0, "no overlap along the main axis");
        }
        assert_eq!(east.last().copied(), Some(Vec3::new(100.0, 0.0, 10.0)), "ends under the cursor");
        let diagonal = row_points(Vec3::ZERO, Vec3::new(-64.0, 0.0, 200.0), Vec2::new(80.0, 48.0));
        assert_eq!(diagonal.len(), 5, "the longer axis in footprints sets the count");
        assert_eq!(row_points(Vec3::ZERO, Vec3::new(10_000.0, 0.0, 0.0), fp).len(), ROW_MAX);
    }

    #[test]
    fn area_orders_visit_the_nearest_target_next() {
        let stops = vec![(Vec3::new(100.0, 0.0, 0.0), 'c'), (Vec3::new(10.0, 0.0, 0.0), 'a'), (Vec3::new(40.0, 0.0, 0.0), 'b'), (Vec3::new(-30.0, 0.0, 0.0), 'd')];
        // From the origin: a (10), then b (30 on), c (60 on), and d last (130 back).
        assert_eq!(nearest_tour(Vec3::ZERO, stops.clone(), 10), vec!['a', 'b', 'c', 'd']);
        assert_eq!(nearest_tour(Vec3::ZERO, stops, 2), vec!['a', 'b'], "capped");
        assert!(nearest_tour::<u8>(Vec3::ZERO, Vec::new(), 5).is_empty());
    }

    #[test]
    fn alt_dragging_fills_the_rectangle() {
        let fp = Vec2::new(32.0, 32.0);
        let pts = grid_points(Vec3::ZERO, Vec3::new(70.0, 0.0, -40.0), fp);
        // Two footprints across (0, 32, 64) and one down (0, -32): a 3 × 2 block.
        assert_eq!(pts.len(), 6);
        assert!(pts.contains(&Vec3::new(64.0, 0.0, -32.0)));
        assert!(grid_points(Vec3::ZERO, Vec3::new(10.0, 0.0, 10.0), fp).is_empty(), "no drag, no grid");
        let big = grid_points(Vec3::ZERO, Vec3::new(5000.0, 0.0, 5000.0), fp);
        assert!(big.len() <= ROW_MAX);
    }

    /// A filled rectangle is built from its far side to its near one, as the builders see it,
    /// so that they stand in what is still open and never wall themselves in.
    #[test]
    fn a_filled_rectangle_is_built_far_side_first() {
        let fp = Vec2::new(80.0, 80.0);
        let mut sites = grid_points(Vec3::ZERO, Vec3::new(160.0, 0.0, 160.0), fp);
        assert_eq!(sites.len(), 9);
        // Builders to the west of the block: the eastern column first, the western last.
        far_side_first(&mut sites, Vec3::new(-200.0, 0.0, 80.0));
        assert!(sites[..3].iter().all(|p| p.x == 160.0), "{sites:?}");
        assert!(sites[6..].iter().all(|p| p.x == 0.0), "{sites:?}");
        assert_eq!(sites.last(), Some(&Vec3::new(0.0, 0.0, 80.0)), "the nearest last");
        // (Height plays no part: on the ground, across.)
        let mut high = sites.clone();
        far_side_first(&mut high, Vec3::new(-200.0, 900.0, 80.0));
        assert_eq!(high, sites);
    }

    #[test]
    fn formations_spread_evenly_along_the_drawn_path() {
        let path = [Vec3::ZERO, Vec3::new(100.0, 0.0, 0.0), Vec3::new(100.0, 0.0, 100.0)];
        assert_eq!(polyline_length(&path), 200.0);
        let spots = spread_along(&path, 5);
        assert_eq!(spots.len(), 5);
        assert_eq!(spots[0], Vec3::ZERO);
        assert_eq!(spots[2], Vec3::new(100.0, 0.0, 0.0), "the corner is halfway");
        assert_eq!(spots[4], Vec3::new(100.0, 0.0, 100.0));
        assert_eq!(spread_along(&path, 1), vec![Vec3::new(100.0, 0.0, 0.0)]);
    }
}
