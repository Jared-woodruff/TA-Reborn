//! The launcher application: state, the frame layout (header, navigation rail, status bar)
//! and the actions the pages request.

use ta_ui::tooltip::TooltipExt as _;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{
    Align, Align2, Color32, CornerRadius, Frame, Layout, Margin, Rect, RichText, ScrollArea, Ui,
    UiBuilder, pos2, vec2,
};
use ta_config::Config;

use crate::Args;
use crate::battles::Battles;
use crate::capture::Capture;
use crate::detect::{self, Detection, Picked};
use crate::getmods::Source;
use crate::launch::{self, GameState};
use crate::modlist::{self, ModFolder, ModList};
use crate::persist::{ConfigStore, SaveStatus};
use crate::theme::{self, *};
use crate::update::{Step, UpdateState};
use crate::widgets::{self, NavIcon};
use ta_ui::metal;

mod events;
pub(crate) use events::UpdateWait;
mod profile_page;
mod profiles;
mod polish;
mod maps;

/// Width of the navigation rail.
const NAV_WIDTH: f32 = 208.0;
pub(crate) const PROJECT_URL: &str = "https://github.com/Jared-woodruff/TA-Reborn";
/// TA Reborn's website, where the header's logo leads; and its name as the logo shows it.
pub(crate) const WEBSITE_URL: &str = "https://ta-reborn.com/";
const WEBSITE_LABEL: &str = "TA-REBORN.COM";
/// How long the footer says the settings were saved.
const SAVED_SHOWN: Duration = Duration::from_secs(4);
/// Width of the narrow rail of icons in a small window.
const COMPACT_NAV_WIDTH: f32 = 58.0;
/// The steel pipes between the header, the navigation rail, the status bar and the page:
/// their thickness, and how far their centre lines are from the panels' edges.
const FRAME_PIPE: f32 = 5.0;
const FRAME_PIPE_AT: f32 = 6.0;
/// How wide a page's column grows in a big window (wider, it is centred): the libraries' grids
/// take what room there is, the Play page and the benchmark's charts a good deal, pages of words
/// stay a reading width.
fn content_width(page: Page) -> f32 {
    match page {
        Page::Mods | Page::Maps | Page::Skins | Page::Profile => 1600.0,
        Page::Play | Page::Benchmark => 1400.0,
        Page::Settings | Page::About => 1180.0,
    }
}
/// A page comes in over this long, rising this far (from below when it is further down the rail
/// than the page before, from above when further up).
const PAGE_TIME: f64 = 0.26;
const PAGE_RISE: f32 = 14.0;
/// After this long with nothing touched (seconds), the idle animations (the banner's drift and
/// scan, the brass's gleam, the lamps' pulse) hold still until the mouse or the keys are touched
/// again: a window left open in front draws nothing more. Not while work is under way.
const SETTLE: f64 = 60.0;
/// At enlarged UI scales the navigation moves above the page, leaving its full width for
/// readable settings. Pages with fixed-width cards retain horizontal scrolling.
fn compact_layout(size: egui::Vec2) -> bool {
    size.x < 900.0 || size.y < 640.0
}
/// How long transient notices stay visible, in seconds.
const NOTICE_TIME: f64 = 5.0;
/// How many notices may wait behind the one shown.
const NOTICES_WAITING: usize = 4;

/// What the first start found and set (see [`ta_config::hardware`]).
pub(crate) struct FirstRun {
    /// "NVIDIA GeForce RTX 3050 (8 GB) · 8 threads · 32 GB".
    pub computer: String,
    pub level: ta_config::Preset,
    pub size: (u32, u32),
    /// The language the menus were set to, as Windows' own (its name).
    pub language: Option<String>,
}

/// Pages of the launcher.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, clap::ValueEnum)]
pub enum Page {
    #[default]
    Play,
    Mods,
    Maps,
    Skins,
    Profile,
    Settings,
    Benchmark,
    About,
}

/// The pages in the rail's order: Ctrl+1 to Ctrl+8 and the shortcuts list follow it.
pub(crate) const PAGES: [Page; 8] = [Page::Play, Page::Mods, Page::Maps, Page::Skins, Page::Profile, Page::Settings, Page::Benchmark, Page::About];

/// A page's icon in the rail.
fn page_icon(page: Page) -> NavIcon {
    match page {
        Page::Play => NavIcon::Play,
        Page::Mods => NavIcon::Mods,
        Page::Maps => NavIcon::Maps,
        Page::Skins => NavIcon::Graphics,
        Page::Profile => NavIcon::Profile,
        Page::Settings => NavIcon::Settings,
        Page::Benchmark => NavIcon::Benchmark,
        Page::About => NavIcon::About,
    }
}

/// The rail's groups: playing; the libraries; you; the launcher itself. These pages begin one.
fn starts_group(page: Page) -> bool {
    matches!(page, Page::Mods | Page::Profile | Page::Settings)
}

fn page_label(page: Page) -> &'static str {
    match page {
        Page::Play => ta_ui::i18n::n("PLAY"),
        Page::Mods => ta_ui::i18n::n("MODS"),
        Page::Maps => ta_ui::i18n::n("MAPS"),
        Page::Skins => ta_ui::i18n::n("SKINS"),
        Page::Profile => ta_ui::i18n::n("PROFILE"),
        Page::Settings => ta_ui::i18n::n("SETTINGS"),
        Page::Benchmark => ta_ui::i18n::n("BENCHMARK"),
        Page::About => ta_ui::i18n::n("ABOUT"),
    }
}

/// Something requested from a page; applied after the frame's UI has been built, so page
/// code never has to juggle borrows of the app state.
#[derive(Clone, Debug)]
pub enum Action {
    Play,
    Maps(crate::maps::Action),
    Rescan,
    BrowseInstall,
    /// Forgets the configured game folder and detects the install automatically.
    UseAutoDetect,
    OpenUrl(String),
    /// Shows the game in its store's app: GOG Galaxy's, or Steam's library.
    OpenGalaxy,
    OpenSteam,
    /// Asks the update source again.
    CheckUpdates,
    /// Applies the automatic check preference or a changed source.
    RefreshUpdates,
    /// Gets the new version: its setup downloaded, checked and run (the launcher closes for
    /// it), or its page opened for a copy Setup did not install.
    Update,
    /// Stops the new version's download.
    CancelUpdate,
    /// The new version's notes, in the dialog.
    ReleaseNotes,
    /// Shows the new version's card (on the Play page).
    ShowUpdate,
    /// Opens an existing file or folder in the system file manager.
    OpenPath(PathBuf),
    /// Creates a folder if needed, then opens it.
    OpenFolder(PathBuf),
    RescanMods,
    /// END GAME: the question first.
    EndGame,
    ShowModReport(String),
    ValidateMod(String),
    RescanSkins,
    SelectSkin(String),
    SelectEnemySkin(String),
    BrowseSkin,
    ImportSkin(PathBuf),
    ExportSkin(String),
    /// Opens the editor on a new skin.
    NewSkin,
    /// Opens the editor on a copy of a built-in skin, to save as one's own.
    CustomizeSkin(String),
    /// Opens the editor on one of one's own skins.
    EditSkin(String),
    /// Saves the editor's skin (in place, or as new when never saved).
    SaveSkin,
    /// Saves the editor's skin as a new one under this name.
    SaveSkinAs(String),
    /// A picture of one's own for a texture slot of the editor's skin (for one unit, or all).
    AssignSkinImage { scope: Option<String>, texture: String },
    BrowseExtraModsDir,
    ClearExtraModsDir,
    RetrySave,
    /// Installs a catalogue mod from a file the player picks.
    InstallModFromFile(&'static str),
    /// Starts the game with these arguments too (a saved game to continue, a replay to watch).
    PlayWith(Vec<OsString>),
    /// Asks whether to remove an installed mod (by id).
    Uninstall(String),
    /// Installs a mod from an archive or a folder the player brought.
    InstallPackage(PathBuf),
    /// Adds a mod found in the original game's folder (by id) as a mod of its own.
    AddGameFolderMod(String),
    /// Picks a mod archive to install.
    BrowsePackage,
    /// Turns a mod on (asking first when it is a second total conversion).
    EnableMod(String),
    DisableMod(String),
    /// Hides a mod from the Mods library, or puts it back (it stays as it is, on or off).
    HideMod(String, bool),
    /// Puts every hidden mod back in the library.
    UnhideAllMods,
    /// Turns a catalogue mod on, and the other conversions off, and starts the game.
    PlayMod(String),
    /// Downloads and installs a catalogue mod's tested release.
    GetMod(&'static str),
    /// Gets every tested release newer than the one installed.
    UpdateAll,
    CancelInstall(String),
    /// Forgets a failed install.
    DismissInstall(String),
    /// Puts text on the clipboard (a crash's details, for a report).
    Copy(String),
    /// Runs the game's benchmark (its report comes to the Benchmark page).
    Benchmark,
    /// Its quick run: what the scores are made of.
    QuickBenchmark,
    /// The settings the benchmark shown recommends, put into the player's.
    ApplyRecommendation,
}

/// The Mods page's views.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum ModsTab {
    /// The library: the catalogue's mods and those installed.
    #[default]
    #[value(alias = "get")]
    Library,
    /// The load order of the mods that are on.
    #[value(alias = "installed")]
    LoadOrder,
}

/// A short message shown in the corner of the page.
struct Notice {
    text: String,
    color: Color32,
    /// When it was first shown (egui's clock).
    since: Option<f64>,
}

pub struct LauncherApp {
    pub(crate) page: Page,
    /// The settings being edited; saved by [`ConfigStore`] shortly after each change.
    pub(crate) config: Config,
    pub(crate) store: ConfigStore,
    profiles: profiles::Profiles,
    pub(crate) detection: Detection,
    pub(crate) mods: ModList,
    /// Mods made for other engines in the original game's folder, as their own installers had
    /// players put them there (`ta_vfs::mods::game_folder_mods`): never part of the original game,
    /// offered on the Mods page to be added as mods.
    pub(crate) game_folder_mods: Vec<ta_vfs::mods::GameFolderMod>,
    pub(crate) mod_validation: crate::validation::ModValidation,
    pub(crate) maps: crate::maps::Library,
    pub(crate) skins: crate::skins::Library,
    pub(crate) game: GameState,
    /// The last folder picked with "Browse…" that is not a valid install, and what it lacks.
    pub(crate) browse_error: Option<(PathBuf, Vec<String>)>,
    /// Actions requested during this frame.
    pub(crate) actions: Vec<Action>,
    /// Settings page: "Reset all" awaiting confirmation.
    pub(crate) confirm_reset: bool,
    /// Settings page: the settings screen (shared with the game's options).
    pub(crate) settings_view: ta_ui::settings::SettingsView,
    pretend_missing: bool,
    notice: Option<Notice>,
    /// Notices waiting their turn (a few at most).
    notices: std::collections::VecDeque<Notice>,
    /// `--show achievement-toast:<ids>`: a capture of the notices of achievements earned.
    toasts_in_capture: bool,
    capture: Option<Capture>,
    /// `--scroll`: page offset applied while capturing a screenshot.
    pub(crate) capture_scroll: Option<f32>,
    /// The window has been fitted to the screen (see [`LauncherApp::fit_to_screen`]).
    fitted: bool,
    /// The update check (see [`crate::update`]).
    pub(crate) update: UpdateState,
    /// The new version on its way, whether its setup runs once it is ready (asked for), why it
    /// did not start, and its card put away until the next start.
    pub(crate) download: Option<crate::update::Download>,
    pub(crate) update_go: bool,
    pub(crate) update_error: Option<String>,
    pub(crate) update_hidden: bool,
    /// `--show update`: the launcher drawn as Setup's (UPDATE NOW), for captures.
    pub(crate) pretend_installed: bool,
    /// `--autoupdate`: the update started as soon as one is found.
    autoupdate: bool,
    /// Where interface languages are found, and those found: (code, name).
    pub(crate) language_dirs: Vec<PathBuf>,
    pub(crate) languages: Vec<(String, String)>,
    /// Mods page: the tab shown, the mod installs, ModDB's pictures and feeds, and the
    /// install just finished (celebrated for a moment).
    pub(crate) mods_tab: ModsTab,
    pub(crate) mod_library: crate::mod_library::Library,
    /// A total conversion to turn on while another is on, awaiting the player's word, and the
    /// others.
    pub(crate) conversion_prompt: Option<(String, Vec<String>)>,
    /// A battle of Your battles made with other mods, asked about before it opens.
    pub(crate) battle_prompt: Option<crate::battles::BattlePrompt>,
    /// The mod folders' dialog is open.
    pub(crate) show_mod_folders: bool,
    pub(crate) show_shortcuts: bool,
    /// `--show progress`: every kind of progress rail on show, for captures.
    pub(crate) show_progress: bool,
    pub(crate) focus_update: bool,
    reset_activity_scroll: bool,
    pub(crate) installer: crate::getmods::Installer,
    pub(crate) media: crate::getmods::Media,
    pub(crate) celebrate: Option<(String, f64)>,
    /// A mod the player asked to remove, awaiting confirmation (by id).
    pub(crate) confirm_uninstall: Option<String>,
    /// A package the player brought, of the id of a mod installed already: asked before it takes
    /// that mod's place (one dropped by its name over the player's own, or over a catalogue mod's
    /// tested release).
    pub(crate) confirm_replace: Option<crate::getmods::Source>,
    /// The load order as it was before a PLAY that changes it for its game (a battle's mods,
    /// USE ITS MODS): put back if the game does not start, the change being that game's.
    pub(crate) order_before_play: Option<Vec<String>>,
    /// The window was closed with work at risk: the question is up; and the answer was to
    /// close anyway.
    confirm_close: bool,
    close_confirmed: bool,
    /// END GAME's question is up; and the game was ended so (not a crash to report).
    confirm_end_game: bool,
    ending_game: bool,
    /// A game this launcher did not start (one running when it was opened), or its own that
    /// started itself again with a battle's mods, followed by its lock: [`LauncherApp::game_running`].
    elsewhere: Elsewhere,
    elsewhere_checked: Option<Instant>,
    /// `--show close`: as if a mod were downloading.
    pretend_at_risk: bool,
    /// Catalogue mods asked for while installed (updates: they keep their on or off).
    updates_requested: std::collections::BTreeSet<String>,
    /// The player's newest saved game, replay and picture (the Play page).
    pub(crate) battles: Battles,
    /// The benchmark's reports, and the run under way (the Benchmark page).
    pub(crate) benchmarks: crate::benchmarks::Benchmarks,
    /// What is new in this version, shown once after an update (until dismissed).
    pub(crate) news: Option<crate::news::Release>,
    /// The changelog's notes for a version, in a dialog: (version, its section).
    pub(crate) whats_new: Option<(String, String)>,
    /// One launcher at a time: this one's claim (`None` in captures and tests).
    instance: Option<crate::instance::Instance>,
    /// The window's inside and whether it is maximised, as last seen (kept when it closes);
    /// `None` when the window's size is not the player's to keep (captures, `--window-size`).
    window: Option<([f32; 2], bool)>,
    /// The launcher went down to the taskbar as the game started (it comes back when it ends).
    minimised_for_game: bool,
    /// What the window does once the frame is drawn, the game having started (minimise, close).
    after_launch: Option<ta_config::OnPlay>,
    /// Text to put on the clipboard with the frame.
    copy: Option<String>,
    /// `--show drop`: files held over the window, for a capture.
    show_drop: bool,
    /// No network (tests): the mods page shows what is cached.
    pub(crate) offline: bool,
    /// The first start set the game up for this computer: what it found (the Play page says
    /// so until dismissed).
    pub(crate) first_run: Option<FirstRun>,
    /// TA's interface sounds and the launcher's music, once the install's archives are open;
    /// the header's music control.
    sounds: crate::sounds::Sounds,
    music_control: crate::music_control::MusicControl,
    /// The page shown last, when the one shown now came in, and from where (1: it rises from
    /// below, -1: it settles from above).
    shown_page: Page,
    pub(crate) page_since: f64,
    page_from: f32,
    /// The Play banner's map (when the player has no photo of their own), and the benchmark's
    /// worlds before a first run.
    pub(crate) hero: crate::hero::Hero,
    pub(crate) worlds: crate::hero::Worlds,
    /// The idle animations' clock (seconds): it runs only while they may move, so they hold
    /// where they were rather than jumping back to a resting pose; and egui's time it last
    /// moved at, and when the mouse or the keys were last touched.
    pub(crate) ambient: f64,
    ambient_frame: f64,
    last_input: f64,
    /// The Play button was pressed: the deployment's moment on screen, from then.
    pub(crate) launching: Option<f64>,
    /// `--film`: the launcher filmed from a script (see [`crate::film`]).
    pub(crate) film: Option<crate::film::Film>,
    /// When the last frame began, while the window's frames are paced here ([`MAX_FPS`]).
    paced: Option<Instant>,
    /// The Menu key was down as the last frame began (it opens a library's context menu).
    menu_key: bool,
    /// The gamepads, read while the settings have them on (not in captures, films or tests).
    pad: Option<crate::pad::Pad>,
    /// Pages the pad's bumpers turned, for the next shortcuts (back is negative).
    pad_pages: i32,
    /// The copy of an unreadable settings file the player was told of (see `tell_recovery`).
    backup_told: Option<PathBuf>,
}

/// The most frames a second the launcher draws. Its window does not wait for the screen
/// (vsync): NVIDIA's OpenGL spends that wait spinning the processor, up to sixteen
/// milliseconds a frame against the two the frame itself takes. Frames asked for sooner
/// (an animation's, the pointer's) wait here instead, asleep.
pub const MAX_FPS: f64 = 120.0;

/// A game running that is not this launcher's own child (see [`LauncherApp::game_running`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Elsewhere {
    None,
    /// One holds the lock.
    Running,
    /// This launcher's game started itself again, then (the new one has not the lock yet).
    Restarting(Instant),
}

impl LauncherApp {
    pub fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Self {
        if !theme::install(&cc.egui_ctx) {
            log::info!("Bahnschrift not found: the interface is in Sofia Sans");
        }
        let legacy_path = args.config.clone().unwrap_or_else(|| ta_config::config_dir().join("config.toml"));
        let legacy_path = std::path::absolute(&legacy_path).unwrap_or(legacy_path);
        let device_root = legacy_path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        let profiles = profiles::Profiles::open(device_root.clone(), !cfg!(test) && args.screenshot.is_none() && args.film.is_none());
        let config_path = profiles.store.as_ref().zip(profiles.active.as_ref())
            .and_then(|(s, p)| s.config_path(&p.id).ok()).unwrap_or(legacy_path);
        let (store, mut config) = ConfigStore::open_in(config_path, device_root);
        log::info!("settings file: {}", store.path.display());
        // Recover preferences without replacing the player's established identity.
        if store.load_error.is_some() && let Some(profile) = &profiles.active {
            config.gameplay.player_name = profile.name.clone();
        }
        // The first start: set up for this computer (its graphics level, the screen's own
        // size, full screen; on a handheld the interface's scale and this window maximised:
        // `Config::set_up_for`). After that the settings are the player's. (`TA_FIRST_RUN`
        // shows it in a capture.) A level put back to Auto waits for the game, as it says.
        let first_run = (config.first_start() && (args.screenshot.is_none() || std::env::var_os("TA_FIRST_RUN").is_some())).then(|| {
            let hw = ta_config::hardware::Hardware::detect();
            config.set_up_for(&hw);
            log::info!("first start on {}: {} graphics", hw.summary(), config.graphics.preset.label());
            FirstRun { computer: hw.summary(), level: config.graphics.preset, size: (config.graphics.width, config.graphics.height), language: None }
        });
        let detection = detect::detect(config.game_dir.as_deref(), args.pretend_missing);
        match &detection.result {
            Ok(install) => log::info!("original game found at {} ({})", install.root.display(), install.source),
            Err(e) => log::warn!("{e}"),
        }
        let language_dirs = ta_ui::i18n::language_dirs(&store.dir());
        let language = std::env::var("TA_LANG").unwrap_or_else(|_| config.gameplay.language.clone());
        if let Err(e) = ta_ui::i18n::set_language(&language, &language_dirs) {
            log::warn!("{e}");
        }
        let languages = ta_ui::i18n::languages(&language_dirs);
        // The first start: the menus in the computer's own language, when there is a
        // translation of it.
        let mut first_run = first_run;
        if let Some(first) = first_run.as_mut()
            && config.gameplay.language.is_empty()
            && std::env::var_os("TA_LANG").is_none()
            && let Some(code) = ta_config::hardware::system_language().and_then(|l| ta_ui::i18n::closest(&l, &languages))
        {
            log::info!("first start: the menus in {code}, as the computer's");
            config.gameplay.language = code.clone();
            if let Err(e) = ta_ui::i18n::set_language(&code, &language_dirs) {
                log::warn!("{e}");
            }
            first.language = languages.iter().find(|(c, _)| *c == code).map(|(_, name)| name.clone());
        }
        // (Scanned in the menus' language: a mod's problems are told in it.)
        let mods = ModList::scan(mod_folders(&config, &store), &config.mods.enabled);
        let game_folder_mods = detection.install().map(|i| ta_vfs::mods::game_folder_mods(&i.root)).unwrap_or_default();
        let mut mod_validation = crate::validation::ModValidation::new(store.dir().join("cache/mod-validation"));
        mod_validation.sync(detection.install().map(|i| i.root.clone()), mods.mods.iter().map(|m| m.info.clone()).collect(), false);
        // (Captures and tests stay off the network; a local file source is fine.)
        let capture = args.screenshot.is_some() || args.film.is_some();
        let offline = capture || args.offline || std::env::var_os("TA_LAUNCHER_OFFLINE").is_some();
        // (A capture asks no one: it shows what the launcher would without a source.)
        let update = update_check(&config, capture || offline, false);
        let mut installer = crate::getmods::Installer::new(store.dir().join("mods"), store.dir().join("cache").join("moddb"));
        // Working folders an earlier run left behind (closed mid-download, an update's setup
        // long since run) go, on a thread of their own.
        if !capture {
            let cache = store.dir().join("cache");
            let mods_dir = store.dir().join("mods");
            let sweep = std::thread::Builder::new().name("sweep".into())
                .spawn(move || sweep_working_folders(&cache, &mods_dir, Duration::from_secs(600)));
            if let Err(e) = sweep { log::warn!("could not sweep old working folders: {e}"); }
        }
        if !offline && let Some(entry) = args.get_mod.as_deref().and_then(crate::catalogue::get) {
            installer.install(entry, None);
        }
        // What is new, once after an update: not on the first start (everything is new), but
        // for a player who had an earlier version (whose settings named none).
        let here = env!("CARGO_PKG_VERSION");
        let seen = config.launcher.seen_version.clone();
        let news = if seen.is_empty() && first_run.is_some() {
            config.launcher.seen_version = here.to_string();
            None
        } else if seen.is_empty() || crate::update::newer(here, &seen) {
            let release = launch::app_dir().and_then(|dir| crate::news::changelog(&dir)).and_then(|path| std::fs::read_to_string(path).ok()).and_then(|text| crate::news::newest(&text));
            match release {
                Some(release) if release.version == here => Some(release),
                // (No notes for this version: nothing to show.)
                _ => {
                    config.launcher.seen_version = here.to_string();
                    None
                }
            }
        } else {
            None
        };
        let unattended = args.screenshot.is_some() || args.film.is_some();
        let window = (!unattended && args.window_size.is_none()).then(|| (config.launcher.window.unwrap_or([1120.0, 720.0]), config.launcher.maximized));
        // A handheld's first start maximises this window too (it opened before the settings
        // were set up; from then on it opens as it was left).
        if first_run.is_some() && window.is_some_and(|(_, maximized)| maximized) {
            cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
        LauncherApp {
            skins: crate::skins::Library::new(store.dir().join("skins"), [config.graphics.unit_skin.clone(), config.graphics.enemy_skin().to_string()]),
            confirm_uninstall: None,
            confirm_replace: None,
            elsewhere: Elsewhere::None,
            elsewhere_checked: None,
            order_before_play: None,
            confirm_close: false,
            close_confirmed: false,
            confirm_end_game: false,
            ending_game: false,
            pretend_at_risk: false,
            updates_requested: std::collections::BTreeSet::new(),
            battles: Battles::scan(&profiles.data_dir(store.dir()), ta_config::pictures_dir()),
            benchmarks: crate::benchmarks::Benchmarks::scan(&store.dir()),
            news,
            whats_new: None,
            instance: None,
            window,
            minimised_for_game: false,
            after_launch: None,
            copy: None,
            show_drop: false,
            page: args.page,
            mods_tab: args.mods_tab,
            mod_library: Default::default(),
            conversion_prompt: None,
            battle_prompt: None,
            show_mod_folders: false,
            show_shortcuts: false,
            show_progress: false,
            focus_update: false,
            reset_activity_scroll: false,
            installer,
            media: crate::getmods::Media::new(),
            celebrate: None,
            offline,
            first_run,
            sounds: crate::sounds::Sounds::start(detection.install().map(|i| i.root.clone()).filter(|_| args.screenshot.is_none()), args.film.is_some()),
            music_control: Default::default(),
            shown_page: args.page,
            page_since: 0.0,
            page_from: 1.0,
            hero: Default::default(),
            worlds: Default::default(),
            ambient: 0.0,
            ambient_frame: 0.0,
            last_input: 0.0,
            launching: None,
            film: None,
            paced: None,
            menu_key: false,
            pad: None,
            pad_pages: 0,
            backup_told: None,
            config,
            store,
            profiles,
            detection,
            game_folder_mods,
            mods,
            mod_validation,
            maps: Default::default(),
            game: GameState::Idle,
            browse_error: None,
            actions: match (args.autoplay, args.autobenchmark) {
                (true, _) => vec![Action::Play],
                (_, true) => vec![Action::Benchmark],
                _ => Vec::new(),
            },
            confirm_reset: false,
            settings_view: {
                let mut view = ta_ui::settings::SettingsView::default();
                if let Some(c) = args.settings_category.as_deref().and_then(|name| ta_ui::settings::Category::ALL.into_iter().find(|c| c.label().eq_ignore_ascii_case(name))) {
                    view.open(c);
                }
                view
            },
            pretend_missing: args.pretend_missing,
            notice: None,
            notices: std::collections::VecDeque::new(),
            toasts_in_capture: false,
            capture_scroll: args.scroll.filter(|_| args.screenshot.is_some()),
            // (A size given on the command line stays as it is.)
            fitted: args.window_size.is_some() || args.film.is_some(),
            capture: args.screenshot.map(|path| Capture::new(path, args.screenshot_frames)),
            update,
            download: None,
            update_go: false,
            update_error: None,
            update_hidden: false,
            pretend_installed: false,
            autoupdate: args.autoupdate,
            language_dirs,
            languages,
        }
    }

    pub(crate) fn push(&mut self, action: Action) {
        self.actions.push(action);
    }

    /// The Mods page on the mod `id` (`--mod`: the game asks for one a battle needs), its details
    /// open where it is in the library.
    pub fn open_mod(&mut self, id: &str) {
        log::info!("showing the mod {id} (asked for)");
        self.show_wanted_mod(id);
    }

    /// The Mods page on a mod a battle needs (the game's GET IT IN THE LAUNCHER, the battles'
    /// SHOW IN MODS), its details open; whether the library has a card of it. A mod neither
    /// installed nor in the catalogue (a player's own, from elsewhere) has none: the library
    /// opened on the first card instead, another mod's, as if that were the one to get. The page
    /// now says so.
    pub(crate) fn show_wanted_mod(&mut self, id: &str) -> bool {
        self.page = Page::Mods;
        // (Installed since the library was last looked through: found.)
        self.discover_mods();
        if self.mods.get(id).is_none() && crate::catalogue::get(id).is_none() {
            self.mods_tab = ModsTab::Library;
            self.mod_library.details_open = false;
            self.notify(ta_ui::i18n::tf("{mod} is not installed, and not a mod the launcher can get: install it from a file you have (INSTALL FROM FILE…)", &[("mod", &id)]), AMBER);
            return false;
        }
        self.show_mod_details(id);
        true
    }

    /// The Mods library at the mod `id`, its details open (in HIDDEN when it is hidden).
    pub(crate) fn show_mod_details(&mut self, id: &str) {
        self.mods_tab = ModsTab::Library;
        self.mod_library.show_hidden(self.config.mods.is_hidden(id));
        self.mod_library.choose(id);
        self.mod_library.details_open = true;
    }

    /// `--show`: a dialog or a state put up for a capture.
    pub fn show(&mut self, what: &str) {
        match what.split_once(':').unwrap_or((what, "")) {
            ("mod-report", id) => {
                self.mod_validation.selected = Some(id.to_string());
                if let Some(capture) = &mut self.capture { capture.wait_for_validation(); }
            }
            ("shortcuts", _) => self.show_shortcuts = true,
            ("mod-folders", _) => self.show_mod_folders = true,
            ("mod-details", id) => self.show_mod_details(id),
            // The mods the player hid, listed (`hidden:<id>,<id>` hides those first).
            ("hidden", ids) => {
                for id in ids.split(',').filter(|id| !id.trim().is_empty()) {
                    self.config.mods.set_hidden(id.trim(), true);
                }
                self.mod_library.show_hidden(true);
            }
            ("mods-list", _) => self.mod_library.view = crate::mod_library::View::List,
            ("conversion", id) => self.conversion_prompt = Some((id.to_string(), modlist::other_conversions(&self.config.mods.enabled, id))),
            // (The newest saved game or replay asked about, as CONTINUE or WATCH would.)
            ("battle-mods", which) => {
                let (record, flag) = if which == "replay" { (self.battles.replay.clone(), "--replay") } else { (self.battles.save.clone(), "--load") };
                if let Some(record) = record { self.open_battle(record, flag); }
            }
            ("maps-list", _) => self.maps.view = crate::maps::View::List,
            ("maps-details", _) => self.maps.details_open = true,
            ("progress", _) => self.show_progress = true,
            ("search", query) => match self.page {
                Page::Mods => self.mod_library.query = query.to_string(),
                Page::Skins => self.skins.browser.query = query.to_string(),
                Page::Maps => self.maps.query = query.to_string(),
                Page::Settings => self.settings_view.set_search(query),
                _ => {}
            },
            ("profile", _) => self.profiles.edit(false),
            ("profile-edit", _) => self.profiles.edit(true),
            ("career", _) => self.profiles.show_career(),
            ("achievements", _) => self.profiles.show_achievements(),
            ("achievement", id) => self.profiles.show_achievement(id),
            ("achievement-toast", ids) => {
                let ids: Vec<String> = ids.split(',').map(str::to_string).collect();
                self.profiles.toasts.push(&ids, &[]);
                self.toasts_in_capture = true;
            }
            ("career-campaign", _) => self.profiles.show_campaign(),
            ("career-history", _) => self.profiles.show_history(),
            ("commanders", _) => self.profiles.show_page(profiles::ProfileTab::Commanders),
            ("profile-delete", id) => {
                let id = if id.is_empty() { self.profiles.active.as_ref().map_or(String::new(), |p| p.id.clone()) } else { id.to_string() };
                self.profiles.show_page(profiles::ProfileTab::Commanders);
                self.profiles.confirm_delete = Some(id);
            }
            ("profile-menu", _) => self.profiles.show_menu(),
            ("whats-new", _) => self.open_whats_new(),
            // The music control as if a track played (captures are silent), its panel open.
            ("music", track) => {
                self.sounds.preview_music(if track.is_empty() { "music/14.mp3" } else { track });
                self.music_control.open();
            }
            ("remove", id) => self.confirm_uninstall = Some(id.to_string()),
            // (A package named like an installed mod, as dropped: the question before it replaces it.)
            ("replace", file) => self.confirm_replace = crate::getmods::Source::package(Path::new(file)),
            ("crash", _) => {
                self.game = GameState::Exited {
                    code: Some(101),
                    success: false,
                    ran_for: Duration::from_secs(754),
                    log: Some(self.log_dir().join("game.log")),
                    crash: Some(launch::Crash { report: self.log_dir().join("crash.txt"), message: "panicked at apps\\ta_game\\src\\app.rs:3386:21: called `Option::unwrap()` on a `None` value".into() }),
                };
            }
            ("drop", _) => self.show_drop = true,
            // The question closing the window asks with work under way (as if a mod downloaded).
            ("close", _) => {
                self.pretend_at_risk = true;
                self.confirm_close = true;
            }
            // The copy found shown as a Steam one (its store and build on the Play page).
            ("steam-install", _) => {
                if let Ok(install) = &mut self.detection.result {
                    install.source = ta_vfs::install::InstallSource::Steam;
                    install.store = ta_vfs::install::Store::Steam;
                    install.build = Some("15829472".into());
                }
            }
            ("skin", id) => {
                // (`skin:<id>@commanders`: its preview on those units.)
                let (id, units) = id.split_once('@').unwrap_or((id, ""));
                if let Some(subjects) = crate::skins::Subjects::ALL.into_iter().find(|s| s.label().eq_ignore_ascii_case(units)) {
                    self.skins.browser.subjects = subjects;
                }
                self.skins.browser.inspecting = id.to_string();
                self.skins.browser.details_open = true;
                self.skins.browser.reveal = Some(id.to_string());
            }
            ("skin-new", _) => self.push(Action::NewSkin),
            ("skin-customize", id) => self.push(Action::CustomizeSkin(id.to_string())),
            ("skin-save-as", id) => {
                self.push(Action::CustomizeSkin(id.to_string()));
                self.skins.ask = Some(crate::skins::Ask::SaveAs(ta_ui::i18n::t("My unit skin").into()));
            }
            ("skin-leave", id) => {
                self.push(Action::CustomizeSkin(id.to_string()));
                self.skins.ask = Some(crate::skins::Ask::Leave);
            }
            ("skin-list", _) => self.skins.browser.view = crate::skins::View::List,
            ("update" | "updating" | "update-failed" | "update-notes", _) => {
                let size = 48_091_444;
                self.update = UpdateState::Available(crate::update::Release {
                    version: "v2.2.0".into(),
                    url: "https://github.com/Jared-woodruff/TA-Reborn/releases/tag/v2.2.0".into(),
                    title: "TA Reborn 2.2.0".into(),
                    notes: "The launcher made new: your battles a click away, mods that come and go, crashes told, and updates that install themselves.\n\n### Highlights\n- **The launcher:**\n  - The last saved game and replay on the Play page, a click from going on.\n  - Mods removed to the Recycle Bin, or installed by dropping them on the window.\n  - New versions downloaded, checked and installed from here.\n- **The licence:** TA Reborn is free software under the [GNU GPL](https://www.gnu.org/licenses/gpl-3.0.html), version 3 or later.".into(),
                    setup: Some(crate::update::Asset { name: "TA-Reborn-2.2.0-setup.exe".into(), url: String::new(), size }),
                    sums: None,
                    local_files: false,
                });
                self.pretend_installed = true;
                let since = Instant::now().checked_sub(Duration::from_secs(6)).unwrap_or_else(Instant::now);
                match what {
                    "updating" => self.download = Some(crate::update::Download::shown("2.2.0", Step::Downloading { got: size * 9 / 20, total: size, since })),
                    "update-failed" => self.download = Some(crate::update::Download::shown("2.2.0", Step::Failed(ta_ui::i18n::t("the download does not match the release's SHA-256").into()))),
                    "update-notes" => self.push(Action::ReleaseNotes),
                    _ => {}
                }
            }
            (other, _) => log::warn!("--show: nothing called {other}"),
        }
    }

    /// Opens the dialog with what is new in this version (the newest release in the changelog
    /// that ships with it).
    pub(crate) fn open_whats_new(&mut self) {
        let text = launch::app_dir().and_then(|dir| crate::news::changelog(&dir)).and_then(|path| std::fs::read_to_string(path).ok());
        let notes = text.as_deref().and_then(|text| {
            let release = crate::news::newest(text)?;
            Some((release.version.clone(), crate::news::section(text, &release.version)?))
        });
        match notes {
            Some(notes) => self.whats_new = Some(notes),
            None => self.notify("The changelog is not in this installation", RED),
        }
    }

    /// Keeps this launcher's claim to be the only one, and listens for others' calls.
    pub fn instance(&mut self, instance: crate::instance::Instance, ctx: &egui::Context) {
        instance.listen(ctx.clone());
        self.instance = Some(instance);
    }

    /// Films the launcher (`--film`): the mods' pictures are fetched from the start, so they are
    /// in before the first frame.
    /// Paces the window's frames ([`MAX_FPS`]): its own, not a test's.
    pub fn pace_frames(&mut self) {
        self.paced = Some(Instant::now());
    }

    pub fn film(&mut self, film: crate::film::Film) {
        self.media.start(self.store.dir().join("cache").join("moddb"), self.offline);
        self.film = Some(film);
    }

    /// `--film`, once a frame before the pages: the pages and downloads the script has due.
    fn film_steps(&mut self, ctx: &egui::Context) {
        use crate::film::Step;
        let Some(due) = self.film.as_mut().map(crate::film::Film::take_due) else { return };
        for step in due {
            match step {
                Step::Page { page, tab, .. } => {
                    use clap::ValueEnum as _;
                    match Page::from_str(&page, true) {
                        Ok(page) => self.page = page,
                        Err(e) => log::warn!("film: {e}"),
                    }
                    if let Some(tab) = tab.as_deref().map(|t| ModsTab::from_str(t, true)) {
                        match tab {
                            Ok(tab) => self.mods_tab = tab,
                            Err(e) => log::warn!("film: {e}"),
                        }
                    }
                    if self.page == Page::Mods {
                        self.discover_mods();
                    }
                }
                Step::Download { id, file, seconds, .. } => {
                    if let Some(film) = &mut self.film {
                        film.download(&id, file, seconds);
                    }
                }
                _ => {}
            }
        }
        if let Some(film) = &mut self.film {
            film.play_downloads(&mut self.installer);
        }
        if self.media.poll(ctx) {
            ctx.request_repaint();
        }
    }

    /// A notice for a few seconds (its text translated).
    /// A notice at the foot of the page: shown now, or after those before it (the same words
    /// twice are one notice; the oldest waiting goes when too many wait).
    pub(crate) fn notify(&mut self, text: impl Into<String>, color: Color32) {
        let text = ta_ui::i18n::t(&text.into()).to_string();
        let notice = Notice { text, color, since: None };
        match &self.notice {
            None => self.notice = Some(notice),
            Some(shown) if shown.text == notice.text => {}
            Some(_) => {
                if self.notices.iter().any(|waiting| waiting.text == notice.text) {
                    return;
                }
                if self.notices.len() >= NOTICES_WAITING {
                    self.notices.pop_front();
                }
                self.notices.push_back(notice);
            }
        }
    }

    /// The last notice given (shown or waiting).
    #[cfg(test)]
    fn last_notice(&self) -> Option<&Notice> {
        self.notices.back().or(self.notice.as_ref())
    }

    fn rescan(&mut self) {
        let previous = self.detection.install().map(|i| i.root.clone());
        self.detection = detect::detect(self.config.game_dir.as_deref(), self.pretend_missing);
        let root = self.detection.install().map(|i| i.root.clone());
        if root != previous {
            self.scan_game_folder();
            self.sync_mod_validation();
            self.skins.invalidate_preview();
            self.sounds = crate::sounds::Sounds::start(root.filter(|_| self.capture.is_none()), self.film.is_some());
        }
    }

    /// Looks again for mods made for other engines in the original game's folder.
    fn scan_game_folder(&mut self) {
        self.game_folder_mods = self.detection.install().map(|i| ta_vfs::mods::game_folder_mods(&i.root)).unwrap_or_default();
    }

    pub(crate) fn rescan_mods(&mut self) {
        self.rescan_maps();
        self.skins.invalidate_preview();
        self.scan_game_folder();
        self.mods = ModList::scan(mod_folders(&self.config, &self.store), &self.config.mods.enabled);
        self.sync_mod_validation();
        self.mod_library.changes.clear();
    }

    /// After an install, a removal or a game: the mods looked through again, and only those
    /// new or changed (and those needing them) checked again. F5 checks them all.
    pub(crate) fn refresh_mods(&mut self) {
        self.rescan_maps();
        self.skins.invalidate_preview();
        self.scan_game_folder();
        self.mods = ModList::scan(mod_folders(&self.config, &self.store), &self.config.mods.enabled);
        self.mod_validation.sync_changed(self.detection.install().map(|i| i.root.clone()),
            self.mods.mods.iter().map(|m| m.info.clone()).collect());
        self.mod_library.changes.clear();
    }

    /// Turns a mod on: a total conversion while another is on asks first.
    pub(crate) fn enable_mod(&mut self, id: &str) {
        let others = modlist::other_conversions(&self.config.mods.enabled, id);
        if others.is_empty() || self.film.is_some() { modlist::set_enabled(&mut self.config.mods.enabled, id, true); }
        else { self.conversion_prompt = Some((id.to_string(), others)); }
    }

    /// Navigation picks up newly added mods without restarting validation of an
    /// unchanged library. Explicit refreshes still recheck the installed bytes.
    pub(crate) fn discover_mods(&mut self) {
        self.skins.invalidate_preview();
        self.mods = ModList::scan(mod_folders(&self.config, &self.store), &self.config.mods.enabled);
        self.mod_validation.sync_discovered(self.detection.install().map(|i| i.root.clone()),
            self.mods.mods.iter().map(|m| m.info.clone()).collect());
        self.mod_library.changes.clear();
    }

    /// An explicit rescan, or a new install folder: every file read again.
    fn sync_mod_validation(&mut self) {
        self.mod_validation.sync(self.detection.install().map(|i| i.root.clone()),
            self.mods.mods.iter().map(|m| m.info.clone()).collect(), true);
    }

    /// Opens the editor on `pack`, a skin not saved yet (a new one, or a built-in's copy).
    fn open_skin_editor(&mut self, pack: ta_skins::Pack) {
        self.page = Page::Skins;
        self.skins.feedback = None;
        self.skins.editor = Some(crate::skin_editor::Editor::open(pack, None));
    }

    pub(crate) fn rescan_skins(&mut self) {
        if !self.skins.busy() {
            self.skins.start(self.store.dir().join("skins"), crate::skins::Operation::Scan([self.config.graphics.unit_skin.clone(), self.config.graphics.enemy_skin().to_string()]));
        }
    }

    /// Folder for logs written on behalf of the game.
    pub(crate) fn log_dir(&self) -> PathBuf {
        self.store.dir().join("logs")
    }

    pub(crate) fn apply(&mut self, action: Action, frame: &eframe::Frame) {
        // A film launches, opens and asks for nothing.
        if self.film.is_some() && !matches!(action, Action::Rescan | Action::RescanMods | Action::RescanSkins | Action::UseAutoDetect | Action::ClearExtraModsDir
            | Action::EnableMod(_) | Action::DisableMod(_) | Action::HideMod(..) | Action::UnhideAllMods) {
            return;
        }
        match action {
            Action::Maps(action) => self.maps_action(action),
            Action::ShowModReport(id) => self.mod_validation.selected = Some(id),
            Action::ValidateMod(id) => {
                // Mod authors may have just changed a manifest or added a dependency.
                // Refresh discovery before requesting a fresh content scan.
                self.refresh_mods();
                self.mod_validation.request(&id);
            }
            Action::Play => self.play(&[]),
            Action::Benchmark => self.run_benchmark(ta_config::benchmark::Mode::Full),
            Action::QuickBenchmark => self.run_benchmark(ta_config::benchmark::Mode::Quick),
            Action::ApplyRecommendation => {
                if let Some(rec) = self.benchmarks.report().and_then(crate::benchmarks::recommendation) {
                    rec.apply(&mut self.config.graphics, &mut self.config.gameplay.unit_limit);
                    self.notify(ta_ui::i18n::tf("Recommended settings applied ({level} graphics): the next game starts with them", &[("level", &ta_ui::i18n::t(rec.level.label()))]), GREEN);
                }
            }
            Action::PlayWith(args) => {
                self.play(&args);
                self.keep_order_if_started();
            }
            Action::Uninstall(id) => {
                if !self.game_running() && !self.installer.busy() && !self.map_import_blocks_mod_changes() {
                    self.confirm_uninstall = Some(id);
                }
            }
            Action::InstallPackage(path) => self.install_package(&path),
            Action::AddGameFolderMod(id) => {
                if !self.map_import_blocks_mod_changes() && !self.game_holds_mods(Some(&id))
                    && let Some(found) = self.game_folder_mods.iter().find(|m| m.id == id).cloned()
                {
                    self.installer.start(Source::GameFolder(found));
                }
            }
            Action::EnableMod(id) => self.enable_mod(&id),
            Action::DisableMod(id) => modlist::set_enabled(&mut self.config.mods.enabled, &id, false),
            Action::HideMod(id, hidden) => self.set_mod_hidden(&id, hidden),
            Action::UnhideAllMods => self.unhide_all_mods(),
            Action::PlayMod(id) => {
                // (The mod is turned on for this game: if it does not start, the order stays as
                // it was. Refused for a mod another needs, it is not left turned off.)
                self.order_before_play = Some(self.config.mods.enabled.clone());
                modlist::turn_on_conversion(&mut self.config.mods.enabled, &id);
                self.play(&[]);
                self.keep_order_if_started();
            }
            Action::GetMod(id) => {
                if !self.offline && !self.map_import_blocks_mod_changes() && !self.game_holds_mods(Some(id)) && let Some(entry) = crate::catalogue::get(id) {
                    self.install_catalogue(entry, None);
                }
            }
            Action::UpdateAll => {
                if self.offline || self.map_import_blocks_mod_changes() || self.game_holds_mods(None) { return; }
                // (Those the library shows: a hidden mod's update is on its own card, in HIDDEN.)
                let shown: Vec<&'static crate::catalogue::Entry> = crate::catalogue::MODS.iter().filter(|entry| !self.config.mods.is_hidden(entry.id)).collect();
                for entry in shown {
                    let installed = self.mods.updatable_version(entry.id);
                    if crate::mod_library::waiting_update(entry, installed).is_some() {
                        self.install_catalogue(entry, None);
                    }
                }
            }
            Action::CancelInstall(id) => self.installer.cancel(&id),
            Action::DismissInstall(id) => {
                self.installer.stages.remove(&id);
                self.installer.packages.remove(&id);
            }
            Action::RescanSkins => self.rescan_skins(),
            Action::SelectSkin(id) => self.skins.start(self.store.dir().join("skins"), crate::skins::Operation::Select(id)),
            Action::SelectEnemySkin(id) => {
                if self.skins.busy() { return; }
                if id.is_empty() { self.config.graphics.enemy_unit_skin.clear(); }
                else { self.skins.start(self.store.dir().join("skins"), crate::skins::Operation::SelectEnemy(id)); }
            }
            Action::BrowseSkin => {
                if self.skins.busy() { return; }
                let dialog = rfd::FileDialog::new()
                    .set_title(ta_ui::i18n::t("Choose a skin to import"))
                    .add_filter(ta_ui::i18n::t("TA Reborn skin"), &["taskin"])
                    .set_parent(frame);
                if let Some(path) = dialog.pick_file() {
                    self.skins.start(self.store.dir().join("skins"), crate::skins::Operation::Import(path));
                }
            }
            Action::ImportSkin(path) => {
                self.page = Page::Skins;
                self.skins.start(self.store.dir().join("skins"), crate::skins::Operation::Import(path));
            }
            Action::ExportSkin(id) => {
                if self.skins.busy() { return; }
                let dialog = rfd::FileDialog::new()
                    .set_title(ta_ui::i18n::t("Export a skin to share"))
                    .add_filter(ta_ui::i18n::t("TA Reborn skin"), &["taskin"])
                    .set_file_name(format!("{}.taskin", id.strip_prefix("builtin:").unwrap_or(&id)))
                    .set_parent(frame);
                if let Some(path) = dialog.save_file() {
                    self.skins.start(self.store.dir().join("skins"), crate::skins::Operation::Export { id, to: path });
                }
            }
            Action::NewSkin => {
                if self.skins.editing_pending() { return; }
                let author = self.profiles.active.as_ref().map(|profile| profile.name.clone()).unwrap_or_default();
                let manifest = ta_skins::Manifest {
                    format: ta_skins::FORMAT, id: "new-skin".into(), name: ta_ui::i18n::t("My unit skin").into(), author,
                    description: String::new(), livery: Some(crate::skin_editor::starting_livery()), textures: Default::default(), units: Default::default(),
                };
                self.open_skin_editor(ta_skins::Pack { manifest, images: Default::default() });
            }
            Action::CustomizeSkin(id) => {
                if self.skins.editing_pending() { return; }
                let root = self.store.dir().join("skins");
                match ta_skins::load(&id, &root) {
                    Ok(mut pack) => {
                        let name = crate::pages::builtin_skin_name(&id).unwrap_or_else(|| pack.manifest.name.clone());
                        pack.manifest.id = "new-skin".into();
                        pack.manifest.name = ta_ui::i18n::tf("{name} copy", &[("name", &name)]);
                        pack.manifest.author = self.profiles.active.as_ref().map(|profile| profile.name.clone()).unwrap_or_default();
                        pack.manifest.description = String::new();
                        self.open_skin_editor(pack);
                    }
                    Err(error) => self.notify(error.to_string(), RED),
                }
            }
            Action::EditSkin(id) => {
                if self.skins.busy() { return; }
                self.page = Page::Skins;
                self.skins.start(self.store.dir().join("skins"), crate::skins::Operation::Open(id));
            }
            Action::SaveSkin => {
                if self.skins.busy() { return; }
                let Some(editor) = self.skins.editor.as_ref() else { return };
                let operation = match &editor.id {
                    Some(_) => crate::skins::Operation::Update { manifest: editor.manifest.clone(), added: editor.added() },
                    None => crate::skins::Operation::Add { pack: editor.pack(), reference: self.skin_reference() },
                };
                self.skins.start(self.store.dir().join("skins"), operation);
            }
            Action::SaveSkinAs(name) => {
                if self.skins.busy() { return; }
                let Some(editor) = self.skins.editor.as_ref() else { return };
                let mut pack = editor.pack();
                pack.manifest.name = name;
                // (Saved as new: what it was opened from stays as it is.)
                let reference = self.skin_reference();
                self.skins.start(self.store.dir().join("skins"), crate::skins::Operation::Add { pack, reference });
            }
            Action::AssignSkinImage { scope, texture } => {
                if self.skins.busy() || self.skins.editor.is_none() { return; }
                let dialog = rfd::FileDialog::new()
                    .set_title(ta_ui::i18n::t("Choose a picture for this texture"))
                    .add_filter(ta_ui::i18n::t("Pictures"), &["png", "jpg", "jpeg"])
                    .set_parent(frame);
                if let Some(path) = dialog.pick_file() {
                    self.skins.start(self.store.dir().join("skins"), crate::skins::Operation::ReadImage { path, scope, texture });
                }
            }
            Action::BrowsePackage => {
                if self.map_import_blocks_mod_changes() { return; }
                let dialog = rfd::FileDialog::new()
                    .set_title(ta_ui::i18n::t("Choose a mod to install"))
                    .add_filter(ta_ui::i18n::t("Mod archive"), &[ta_vfs::PACKAGE_EXTENSIONS.as_slice(), ta_vfs::ARCHIVE_EXTENSIONS.as_slice()].concat())
                    .set_parent(frame);
                if let Some(path) = dialog.pick_file() {
                    self.install_package(&path);
                }
            }
            Action::Copy(text) => {
                self.copy = Some(text);
                self.notify("Copied", GREEN);
            }
            Action::Rescan => {
                self.rescan();
                self.browse_error = None;
                match &self.detection.result {
                    Ok(install) => self.notify(ta_ui::i18n::tf("Found Total Annihilation in {path}", &[("path", &install.root.display())]), GREEN),
                    Err(_) => self.notify("Still no Total Annihilation install found", RED),
                }
            }
            Action::BrowseInstall => self.browse_install(frame),
            Action::UseAutoDetect => {
                self.config.game_dir = None;
                self.browse_error = None;
                self.rescan();
                if self.detection.install().is_some() {
                    self.notify("Switched to automatic detection", GREEN);
                }
            }
            Action::OpenUrl(url) => self.open_web(&url),
            Action::OpenGalaxy => self.open(OsStr::new(ta_vfs::install::GOG_GALAXY_URL)),
            Action::OpenSteam => self.open(OsStr::new(ta_vfs::install::STEAM_LIBRARY_URL)),
            // (Not while an update is on its way: its release stays as it was found.)
            Action::CheckUpdates | Action::RefreshUpdates if self.download.is_none() => {
                self.update = update_check(&self.config, self.offline || self.capture.is_some() || self.film.is_some(), matches!(action, Action::CheckUpdates));
            }
            Action::CheckUpdates | Action::RefreshUpdates => {}
            Action::Update => self.start_update(),
            Action::CancelUpdate => {
                self.download = None;
                self.update_go = false;
            }
            Action::ReleaseNotes => {
                if let UpdateState::Available(release) = &self.update {
                    if release.notes.trim().is_empty() {
                        let url = release.url.clone();
                        self.open_web(&url);
                    } else {
                        self.whats_new = Some((release.number().to_string(), release.notes.clone()));
                    }
                }
            }
            Action::ShowUpdate => {
                self.focus_update = true;
                self.page = Page::Play;
                self.update_hidden = false;
            }
            Action::OpenPath(path) => self.open(path.as_os_str()),
            Action::OpenFolder(path) => {
                if let Err(e) = std::fs::create_dir_all(&path) {
                    self.notify(ta_ui::i18n::tf("Could not create {path}: {error}", &[("path", &path.display()), ("error", &e)]), RED);
                } else {
                    self.open(path.as_os_str());
                    // (A mods folder may just have been made: what is put in it is found.)
                    if mod_folders(&self.config, &self.store).iter().any(|folder| folder.path == path) {
                        self.discover_mods();
                    }
                }
            }
            Action::RescanMods => {
                let before: std::collections::BTreeSet<String> = self.mods.mods.iter().map(|m| m.info.manifest.id.clone()).collect();
                self.rescan_mods();
                let found = self.mods.mods.len();
                let new = self.mods.mods.iter().filter(|m| !before.contains(&m.info.manifest.id)).count();
                let gone = before.iter().filter(|id| self.mods.get(id).is_none()).count();
                if self.film.is_none() {
                    self.notify(ta_ui::i18n::tf("Mods looked through again: {found} found, {new} new, {gone} gone", &[("found", &found), ("new", &new), ("gone", &gone)]), GREEN);
                }
            }
            Action::EndGame => {
                if self.game.is_running() { self.confirm_end_game = true; }
            }
            Action::BrowseExtraModsDir => {
                let mut dialog = rfd::FileDialog::new().set_title(ta_ui::i18n::t("Choose an extra mods folder")).set_parent(frame);
                if let Some(current) = &self.config.mods.extra_dir {
                    dialog = dialog.set_directory(current);
                }
                if let Some(dir) = dialog.pick_folder() {
                    // (Total Annihilation's own folder holds TA's archives, every one of which would
                    // show as a mod: the mods its installers put there are offered as they are.)
                    let same = |a: &Path, b: &Path| std::fs::canonicalize(a).ok().zip(std::fs::canonicalize(b).ok()).is_some_and(|(a, b)| a == b);
                    if self.detection.install().is_some_and(|install| same(&install.root, &dir)) {
                        self.notify(ta_ui::i18n::t("Total Annihilation's own folder cannot be a mods folder: the mods its installers put there are offered on the Mods page."), AMBER);
                        return;
                    }
                    self.config.mods.extra_dir = Some(dir);
                    self.refresh_mods();
                }
            }
            Action::ClearExtraModsDir => {
                self.config.mods.extra_dir = None;
                self.refresh_mods();
            }
            Action::RetrySave => self.store.save_now(&mut self.config),
            Action::InstallModFromFile(id) => {
                if self.map_import_blocks_mod_changes() || self.game_holds_mods(Some(id)) { return; }
                let Some(entry) = crate::catalogue::get(id) else { return };
                if !entry.can_import() { return; }
                let name = entry.file.map_or(entry.name, |f| f.name);
                let dialog = rfd::FileDialog::new()
                    .set_title(ta_ui::i18n::tf("Choose {file}", &[("file", &name)]))
                    .add_filter(ta_ui::i18n::t("Mod archive"), &ta_vfs::PACKAGE_EXTENSIONS)
                    .set_parent(frame);
                if let Some(path) = dialog.pick_file() {
                    self.install_catalogue(entry, Some(path));
                }
            }
        }
    }

    /// Starts the game (with `extra` arguments), then does as the player has the launcher do
    /// when the game starts.
    /// Whether a game is running: this launcher's own, or another seen by its lock (one running
    /// when the launcher was opened, its own started again with a battle's mods). While one runs
    /// the mods, maps and profiles are left as they are, and no other game is started.
    pub(crate) fn game_running(&self) -> bool {
        self.game.is_running() || self.elsewhere != Elsewhere::None
    }

    /// A game this launcher did not start, or its own started again, followed by its lock
    /// (`ta_config::running`), a second at a time: when it ends, as when its own does.
    fn follow_game_elsewhere(&mut self, ctx: &egui::Context) {
        // (The new game takes the lock once the one before has ended: given a few seconds.)
        const RESTART_GRACE: Duration = Duration::from_secs(8);
        if self.game.is_running() {
            self.elsewhere = Elsewhere::None;
            return;
        }
        if self.elsewhere_checked.is_none_or(|at| at.elapsed() >= Duration::from_secs(1)) {
            self.elsewhere_checked = Some(Instant::now());
            let held = ta_config::running::held(&self.store.dir());
            let (next, ended) = match self.elsewhere {
                _ if held => (Elsewhere::Running, false),
                Elsewhere::Restarting(since) if since.elapsed() < RESTART_GRACE => (Elsewhere::Restarting(since), false),
                Elsewhere::None => (Elsewhere::None, false),
                Elsewhere::Running | Elsewhere::Restarting(_) => (Elsewhere::None, true),
            };
            self.elsewhere = next;
            if ended {
                self.game_ended(ctx);
            }
        }
        if self.elsewhere != Elsewhere::None {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    /// After a PLAY that changed the load order for its game: the order is kept if the game
    /// started, else put back as it was.
    fn keep_order_if_started(&mut self) {
        if let Some(before) = self.order_before_play.take()
            && !self.game.is_running()
            && self.config.mods.enabled != before
        {
            self.config.mods.enabled = before;
            self.mods.refresh(&self.config.mods.enabled);
        }
    }

    fn play(&mut self, extra: &[OsString]) {
        if self.profiles.active.is_none() && self.profiles.required {
            self.profiles.edit(false);
            return;
        }
        if self.profiles.error.is_some() {
            self.profiles.refresh();
        }
        if let Some(error) = self.profiles.error.clone() {
            self.game = GameState::Failed(ta_ui::i18n::tf("Your profile could not be read, so the game has not started: {error}. Switch commanders on the Profile page, or restart the launcher.", &[("error", &error)]));
            self.page = Page::Play;
            return;
        }
        if self.game_running() {
            self.notify("The game is already running: close it to start another", AMBER);
            return;
        }
        // A queued installer may be about to publish a replacement. Do not start a
        // reader while it is moving the previous install aside.
        if self.installer.busy() {
            self.notify("Wait for mod installations to finish before starting the game.", AMBER);
            return;
        }
        if self.skins.selection_pending() {
            self.notify("Wait for the current skin operation to finish, then try again.", AMBER);
            return;
        }
        if self.maps.busy() {
            self.notify("Wait for map operations to finish before starting the game.", AMBER);
            return;
        }
        // The game reads config.toml when it starts, so write pending edits first.
        let saved = self.store.flush(&mut self.config).and_then(|()| {
            self.sync_profile_name();
            self.store.flush(&mut self.config)
        });
        if let Err(error) = saved {
            self.game = GameState::Failed(ta_ui::i18n::tf("Your settings could not be saved, so the game has not started. Use Retry in the status bar, then launch again. {error}", &[("error", &error)]));
            self.page = Page::Play;
            return;
        }
        if let Some(error) = self.profiles.error.clone() {
            self.game = GameState::Failed(ta_ui::i18n::tf("Your profile could not be read, so the game has not started: {error}. Switch commanders on the Profile page, or restart the launcher.", &[("error", &error)]));
            self.page = Page::Play;
            return;
        }
        // Flush may adopt external changes to the install or enabled mods. Resolve
        // the exact settings the child will read, including manifests edited on disk.
        self.rescan();
        self.discover_mods();
        if !extra.iter().any(|a| a == "--no-mods")
            && let modlist::Resolution::Err(problem) = &self.mods.resolution
        {
            self.game = GameState::Failed(ta_ui::i18n::tf("The mods that are on cannot be loaded together, so the game has not started: {problem}. Put it right in the load order, or turn them off.", &[("problem", problem)]));
            self.page = Page::Play;
            return;
        }
        let Some(root) = self.detection.install().map(|i| i.root.clone()) else { return };
        self.game = launch::launch(&root, &self.log_dir(), &self.store.dir(), self.profiles.active.as_ref().map(|p| p.id.as_str()), extra);
        self.installer.set_game_running(self.game.is_running());
        if !self.game.is_running() {
            self.page = Page::Play;
            return;
        }
        self.notify("Game launched — good hunting, Commander", AMBER);
        match self.config.launcher.on_play {
            ta_config::OnPlay::Stay => {}
            // (Closing would lose work under way, an unsaved skin, a download: the launcher goes
            // down to the taskbar then.)
            ta_config::OnPlay::Close if self.film.is_none() && !self.work_at_risk().is_empty() => {
                self.after_launch = Some(ta_config::OnPlay::Minimise);
            }
            ta_config::OnPlay::Minimise | ta_config::OnPlay::Close if self.film.is_none() => {
                self.after_launch = Some(self.config.launcher.on_play);
            }
            _ => {}
        }
    }

    /// The benchmark: the game started with it (no mods: every run the same game), its report to
    /// come to the Benchmark page when it ends. The launcher goes down to the taskbar meanwhile
    /// (never closes: it waits for the report).
    fn run_benchmark(&mut self, mode: ta_config::benchmark::Mode) {
        if self.game_running() || self.detection.install().is_none() {
            return;
        }
        if let Err(e) = std::fs::create_dir_all(&self.benchmarks.dir) {
            self.notify(ta_ui::i18n::tf("Could not create {path}: {error}", &[("path", &self.benchmarks.dir.display()), ("error", &e)]), RED);
            return;
        }
        // (The game starts in its own folder: the report's path whole.)
        let file = self.benchmarks.next_file();
        let file = std::path::absolute(&file).unwrap_or(file);
        self.benchmarks.stopped = None;
        self.play(&[OsString::from("--benchmark"), file.clone().into_os_string(), OsString::from("--benchmark-mode"), OsString::from(mode.name()), OsString::from("--no-mods")]);
        if self.game.is_running() {
            self.benchmarks.running = Some(file);
            self.benchmarks.running_mode = mode;
            self.notify("Benchmark started. Press Esc in the game to cancel.", AMBER);
            if self.film.is_none() {
                self.after_launch = Some(ta_config::OnPlay::Minimise);
            }
        }
    }

    /// The new version asked for: its setup downloaded, checked and run (the launcher closes for
    /// it: [`LauncherApp::updates`]); a copy Setup did not install opens the release's page.
    fn start_update(&mut self) {
        let UpdateState::Available(release) = &self.update else { return };
        if !(self.pretend_installed || crate::update::installed()) || release.setup.is_none() || release.sums.is_none() || (self.offline && !release.available_offline()) {
            let url = release.url.clone();
            if !url.is_empty() {
                self.open_web(&url);
            }
            return;
        }
        self.update_error = None;
        self.update_go = true;
        if !matches!(self.download.as_ref().map(|d| &d.step), Some(Step::Downloading { .. } | Step::Checking | Step::Ready(_))) {
            self.download = Some(crate::update::Download::start(release, &self.store.dir().join("cache").join("updates")));
        }
    }

    /// A mod brought by the player (an archive or a folder dropped on the window, or picked):
    /// a classic TA archive is copied into the mods folder as it is (it is a mod by itself);
    /// anything else is unpacked and installed by the installer. The Mods page's installed
    /// list shows it coming.
    fn install_package(&mut self, path: &Path) {
        if self.map_import_blocks_mod_changes() { return; }
        match Source::package(path) {
            Some(source) if self.game_holds_mods(Some(source.id())) => return,
            // (Dropped again while it installs: said, not left silent.)
            Some(source) if self.installer.stages.get(source.id()).is_some_and(crate::getmods::Stage::active) => {
                self.notify(ta_ui::i18n::t("Wait for the mod being installed"), AMBER);
                return;
            }
            // A catalogue mod's tested release installs as the card installs it: an update when
            // the mod is installed (its on or off kept, the other conversions left alone), and
            // never behind the game folder's copy, which would hide it.
            Some(Source::Catalogue(entry, from)) => {
                if self.mods.in_game_folder(entry.id) {
                    self.notify(ta_ui::i18n::t("This copy is in the game folder, which comes before your mods folder: replace it there to update it."), AMBER);
                    return;
                }
                self.install_catalogue(entry, from);
            }
            // (Behind the game folder's copy of its id it would change nothing: replaced there.)
            Some(source) if self.mods.in_game_folder(source.id()) => {
                self.notify(ta_ui::i18n::t("This copy is in the game folder, which comes before your mods folder: replace it there to update it."), AMBER);
                return;
            }
            // (Over an installed mod of its id: asked first.)
            Some(source) if self.mods.get(source.id()).is_some() => self.confirm_replace = Some(source),
            Some(source) => self.installer.start(source),
            None => {
                self.notify(ta_ui::i18n::tf("{path} has no name to install it by", &[("path", &path.display())]), RED);
                return;
            }
        }
        self.page = Page::Mods;
        self.mods_tab = ModsTab::Library;
    }

    /// Installs a catalogue mod (from a file the player has, or downloaded), noting whether it
    /// is an update (installed already: it keeps its on or off when it is done).
    fn install_catalogue(&mut self, entry: &'static crate::catalogue::Entry, from: Option<PathBuf>) {
        if self.mods.get(entry.id).is_some() {
            self.updates_requested.insert(entry.id.to_string());
        } else {
            self.updates_requested.remove(entry.id);
        }
        self.installer.install(entry, from);
    }

    /// Whether the running game may be reading the installed mod `id` (any installed mod, for
    /// `None`), so it cannot be replaced now: the player is told so. (Its files open in the game,
    /// the swap would fail half way, or change them under it.)
    fn game_holds_mods(&mut self, id: Option<&str>) -> bool {
        if !self.game_running() || id.is_some_and(|id| self.mods.get(id).is_none()) { return false; }
        self.notify(ta_ui::i18n::t("Close the game first: an installed mod cannot be updated or replaced while the game runs"), AMBER);
        true
    }

    /// The mod `id` removed: its folder (or archive) to the Recycle Bin, and out of the load
    /// order.
    fn uninstall(&mut self, id: &str) {
        use ta_ui::recycle::Outcome;
        if self.game_running() || self.installer.busy() || self.map_import_blocks_mod_changes() { return; }
        let Some(entry) = self.mods.get(id) else { return };
        let (path, name) = (entry.info.path.clone(), entry.info.manifest.name.clone());
        match ta_ui::recycle::recycle(&path) {
            Ok(Outcome::Kept) => self.notify(ta_ui::i18n::tf("{name} is left where it is", &[("name", &name)]), AMBER),
            Ok(outcome) => {
                log::info!("removed {name}: {} ({outcome:?})", path.display());
                modlist::set_enabled(&mut self.config.mods.enabled, id, false);
                self.refresh_mods();
                let text = match outcome {
                    Outcome::Recycled => ta_ui::i18n::tf("{name} is removed (it is in the Recycle Bin)", &[("name", &name)]),
                    _ => ta_ui::i18n::tf("{name} is deleted (it could not go to the Recycle Bin)", &[("name", &name)]),
                };
                self.notify(text, AMBER);
            }
            Err(e) => {
                log::warn!("removing {}: {e}", path.display());
                self.notify(ta_ui::i18n::tf("Could not remove {name}: {error}", &[("name", &name), ("error", &e)]), RED);
            }
        }
    }

    fn browse_install(&mut self, frame: &eframe::Frame) {
        let mut dialog = rfd::FileDialog::new().set_title(ta_ui::i18n::t("Select your Total Annihilation folder")).set_parent(frame);
        if let Some(start) = detect::browse_start_dir() {
            dialog = dialog.set_directory(start);
        }
        let Some(folder) = dialog.pick_folder() else { return };
        self.use_install_folder(&folder);
    }

    /// A folder the player picked (or dropped on the window) as the game's: used when it holds
    /// the game (or a folder in it does), else what it lacks is shown.
    pub(crate) fn use_install_folder(&mut self, folder: &Path) {
        match detect::validate_picked(folder) {
            Picked::Valid { root, nested } => {
                self.browse_error = None;
                self.config.game_dir = Some(root.clone());
                self.rescan();
                let text = if nested {
                    ta_ui::i18n::tf("Found Total Annihilation in {path}", &[("path", &root.display())])
                } else {
                    ta_ui::i18n::t("Install folder set").to_owned()
                };
                self.notify(text, GREEN);
            }
            Picked::Invalid { path, missing } => self.browse_error = Some((path, missing)),
        }
    }

    fn open_web(&mut self, target: &str) {
        match crate::http::address(target, crate::http::Policy::Web) {
            Ok(url) => self.open(OsStr::new(&url)),
            Err(error) => self.notify(error, RED),
        }
    }

    fn open(&mut self, target: &OsStr) {
        if let Err(e) = open::that_detached(target) {
            log::warn!("could not open {}: {e}", target.to_string_lossy());
            self.notify(ta_ui::i18n::tf("Could not open {path}: {error}", &[("path", &target.to_string_lossy()), ("error", &e)]), RED);
        }
    }

    // ---- Frame layout ----------------------------------------------------------------------

    fn compact_header(&mut self, ui: &mut Ui) {
        egui::Panel::top("compact_header").exact_size(64.0).resizable(false)
            .frame(Frame::NONE.inner_margin(Margin::symmetric(12, 6))).show(ui, |ui| {
                let rect = ui.max_rect();
                ui.painter().add(metal::backdrop(ui.ctx(), rect, BG_DEEP));
                let profile_width = (rect.width() * 0.46).clamp(180.0, 250.0);
                let profile_rect = Rect::from_min_max(pos2(rect.max.x - profile_width, rect.min.y), rect.max);
                let mut profile = ui.new_child(UiBuilder::new().max_rect(profile_rect).layout(Layout::top_down(Align::Min)));
                self.profile_widget(&mut profile);
                // PLAY on every page but Play's own (whose banner has it), beside the profile.
                let mut right = profile_rect.min.x - 12.0;
                if self.page != Page::Play && self.detection.install().is_some() {
                    let play = Rect::from_min_max(pos2(right - 92.0, rect.center().y - 17.0), pos2(right, rect.center().y + 17.0));
                    let mut slot = ui.new_child(UiBuilder::new().max_rect(play).layout(Layout::left_to_right(Align::Center)));
                    let running = self.game_running();
                    let button = slot.add_enabled(!running, widgets::ActionButton::primary("PLAY").size(play.width(), play.height()));
                    if button.hover_tip(ta_ui::i18n::t("Start the game")).disabled_tip(ta_ui::i18n::t("The game is running")).clicked() {
                        self.actions.push(Action::Play);
                    }
                    right = play.min.x - 12.0;
                }
                let left = Rect::from_min_max(rect.min, pos2(right, rect.max.y));
                let ui = &mut ui.new_child(UiBuilder::new().max_rect(left).layout(Layout::top_down(Align::Min)));
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.horizontal(|ui| {
                    let title = widgets::galley(ui, RichText::new("TA REBORN").font(strong(16.0)));
                    let fits = title.size().x + 70.0 + ui.spacing().item_spacing.x <= ui.available_width();
                    let title = if fits { title } else { widgets::galley(ui, RichText::new("TA").font(strong(16.0))) };
                    // The website's link, as the full header's logo is: brass and underlined under
                    // the pointer.
                    let (rect, response) = ui.allocate_exact_size(title.size() + vec2(0.0, 4.0), egui::Sense::click());
                    let hot = link_heat(ui, &response);
                    let colour = TEXT_BRIGHT.lerp_to_gamma(AMBER_BRIGHT, hot);
                    ui.painter().galley_with_override_text_color(rect.min, title.clone(), colour);
                    if hot > 0.0 {
                        ui.painter().hline(rect.min.x..=rect.min.x + title.size().x, rect.min.y + title.size().y + 1.0, egui::Stroke::new(1.0, AMBER.gamma_multiply(hot)));
                    }
                    widgets::focus_ring(ui, &response, rect.expand(2.0));
                    self.website_link(response);
                    let (music, _) = ui.allocate_exact_size(vec2(70.0, 30.0), egui::Sense::hover());
                    self.music_switch(ui, music);
                });
                // (The page shown: the rail of icons chooses.)
                ui.add(egui::Label::new(RichText::new(ta_ui::i18n::t(page_label(self.page))).font(strong(13.0)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(1.6))).truncate());
            });
    }

    /// The narrow rail of a small window: the pages' icons, their lamps and badges, the names
    /// and keys in their tooltips.
    fn nav_compact(&mut self, ui: &mut Ui) {
        egui::Panel::left("nav_compact")
            .exact_size(COMPACT_NAV_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::NONE)
            .show(ui, |ui| {
                let rect = ui.max_rect();
                ui.painter().add(metal::backdrop(ui.ctx(), rect, BG_DEEP));
                let x = rect.max.x - 2.0;
                ui.painter().vline(x, rect.y_range(), egui::Stroke::new(1.0, LINE));
                let inside = Rect::from_min_max(rect.min, pos2(rect.max.x - 4.0, rect.max.y));
                let ui = &mut ui.new_child(UiBuilder::new().max_rect(inside).layout(Layout::top_down(Align::Min)));
                ui.add_space(space::S);
                ui.spacing_mut().item_spacing.y = 1.0;
                // (Items as tall as a short window leaves room for.)
                let size = ((inside.height() - 40.0) / PAGES.len() as f32 - 1.0).clamp(30.0, 44.0);
                let mut shown_at = None;
                for page in PAGES {
                    if starts_group(page) {
                        ui.add_space(4.0);
                    }
                    let (badge, why) = self.nav_badge(page);
                    let hint = polish::page_shortcut(page);
                    let hint = match why { Some(why) => format!("{hint} · {why}"), None => hint };
                    let item = widgets::nav_icon_item(ui, page_icon(page), page_label(page), badge, self.page == page, size, Some(&hint));
                    if self.page == page {
                        shown_at = Some(item.rect);
                    }
                    if item.clicked() {
                        self.open_page(page);
                    }
                }
                if let Some(at) = shown_at {
                    widgets::nav_marker(ui, "pages_compact", at);
                }
            });
    }

    /// What closing the launcher now would lose or cut short, said in the question before it
    /// closes.
    pub(crate) fn work_at_risk(&self) -> Vec<&'static str> {
        use ta_ui::i18n::n;
        let mut risks = Vec::new();
        if matches!(self.store.status, SaveStatus::Failed(_)) {
            risks.push(n("Your settings could not be saved. Your changes are still kept in the launcher."));
        }
        if self.profiles.unsaved_edits() {
            risks.push(n("The profile you are editing has changes that are not saved."));
        }
        if self.skins.editor.as_ref().is_some_and(|e| e.changed()) {
            risks.push(n("The skin you are editing has changes that are not saved."));
        }
        if self.installer.busy() {
            risks.push(n("A mod is still downloading or installing: it would start over."));
        }
        if self.download.as_ref().is_some_and(|download| matches!(download.step, Step::Downloading { .. } | Step::Checking)) {
            risks.push(n("The update is still downloading."));
        }
        if self.maps.busy() {
            risks.push(n("Maps are still being imported."));
        }
        if self.pretend_at_risk && risks.is_empty() {
            risks.push(n("A mod is still downloading or installing: it would start over."));
        }
        risks
    }

    /// END GAME's question: the game closed at once, or kept.
    fn end_game_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_end_game {
            return;
        }
        if !self.game.is_running() {
            self.confirm_end_game = false;
            return;
        }
        let mut end = false;
        let response = ta_ui::panel::modal(ctx, "launcher_end_game", ta_ui::panel::Panel::new("END THE GAME?").exit(ta_ui::panel::Exit::Close).width(440.0).accent(RED), true, |ui| {
            ui.add(egui::Label::new(RichText::new(ta_ui::i18n::t("The game is closed at once: anything not saved in it is lost. Use this when it has stopped responding.")).color(TEXT)).wrap());
            ui.add_space(space::M);
            ui.horizontal(|ui| {
                if ui.add(widgets::ActionButton::danger("END GAME")).clicked() {
                    end = true;
                }
                if ui.add(widgets::ActionButton::secondary("KEEP PLAYING")).clicked() {
                    self.confirm_end_game = false;
                }
            });
        });
        if response.exit {
            self.confirm_end_game = false;
        }
        if end {
            self.confirm_end_game = false;
            self.ending_game = true;
            self.game.end();
            ctx.request_repaint();
        }
    }

    /// The question when the window is closed with work at risk: KEEP OPEN or CLOSE ANYWAY.
    fn close_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_close {
            return;
        }
        let risks = self.work_at_risk();
        if risks.is_empty() {
            // (Done meanwhile: nothing to ask any more.)
            self.confirm_close = false;
            return;
        }
        let mut close = false;
        let response = ta_ui::panel::modal(ctx, "launcher_close", ta_ui::panel::Panel::new("CLOSE THE LAUNCHER?").exit(ta_ui::panel::Exit::Close).width(440.0), true, |ui| {
            for risk in &risks {
                ui.add(egui::Label::new(RichText::new(ta_ui::i18n::t(risk)).color(TEXT)).wrap());
            }
            ui.add_space(space::M);
            ui.horizontal(|ui| {
                if ui.add(widgets::ActionButton::primary("KEEP OPEN")).clicked() {
                    self.confirm_close = false;
                }
                if ui.add(widgets::ActionButton::danger("CLOSE ANYWAY")).clicked() {
                    close = true;
                }
            });
        });
        if response.exit {
            self.confirm_close = false;
        }
        if close {
            self.confirm_close = false;
            self.close_confirmed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// A settings file that could not be read is copied aside as the defaults are first saved
    /// over it, and the footer's warning goes with that save (a commander's name is saved at
    /// once, so it was gone before it could be read): the player is told where the copy is, in a
    /// notice that stays until dismissed.
    fn tell_recovery(&mut self) {
        let Some(backup) = self.store.backup.clone() else { return };
        if self.backup_told.as_ref() == Some(&backup) { return; }
        self.notify(ta_ui::i18n::tf("Defaults are in use. The unreadable file was backed up to {path}.", &[("path", &backup.display())]), RED);
        self.backup_told = Some(backup);
    }

    /// The active commander has not finished a battle yet.
    pub(crate) fn newcomer(&self) -> bool {
        self.profiles.newcomer()
    }

    /// Shows `page`, catching up on what may have changed while it was away (mods dropped in
    /// a folder, skins, maps).
    pub(super) fn open_page(&mut self, page: Page) {
        if page == self.page {
            return;
        }
        self.page = page;
        if page == Page::Mods {
            // Pick up mods dropped into a folder while the launcher was open.
            self.discover_mods();
        }
        if page == Page::Skins {
            self.rescan_skins();
        }
        if page == Page::Maps {
            self.visit_maps();
        }
        if page == Page::Profile {
            self.refresh_profile();
        }
    }

    /// What a page's item in the rail flags, and why (its tooltip says): the game not found;
    /// a mod that is on with errors, else updates waiting; the benchmark running.
    fn nav_badge(&self, page: Page) -> (Option<(String, egui::Color32)>, Option<String>) {
        use ta_ui::i18n::{t, tf};
        match page {
            Page::Play if self.detection.install().is_none() => (Some(("!".to_owned(), RED)), Some(t("Total Annihilation was not found").to_string())),
            Page::Mods => {
                let broken = self.config.mods.enabled.iter().filter(|id| matches!(self.mod_validation.status(id), crate::validation::ValidationStatus::Ready(ta_data::validation::Status::Errors))).count();
                if broken > 0 {
                    return (Some(("!".to_owned(), RED)), Some(t("A mod that is on has errors: its MOD INFO lists them").to_string()));
                }
                // (A hidden mod's update waits in HIDDEN, uncounted, as UPDATE ALL leaves it.)
                let updates = self.mods.mods.iter()
                    .filter(|m| !self.config.mods.is_hidden(m.id()))
                    .filter(|m| crate::catalogue::get(m.id()).is_some_and(|entry| crate::mod_library::waiting_update(entry, self.mods.updatable_version(m.id())).is_some()))
                    .count();
                if updates > 0 {
                    (Some((updates.to_string(), AMBER)), Some(tf("Updates waiting: {n}", &[("n", &updates)])))
                } else {
                    (None, None)
                }
            }
            Page::Benchmark if self.benchmarks.running.is_some() => (Some(("●".to_owned(), AMBER)), Some(t("The benchmark is running").to_string())),
            _ => (None, None),
        }
    }

    /// The Play banner's map: asked for while the Play page shows and the player has no photo
    /// of their own (looked for first), and taken in once made.
    fn banner_picture(&mut self, ctx: &egui::Context) {
        if self.hero.poll(ctx) | self.worlds.poll(ctx) {
            ctx.request_repaint();
        }
        if self.page != Page::Play || self.battles.loading() || self.battles.backdrop.is_some() {
            return;
        }
        let newest = [&self.battles.save, &self.battles.replay].into_iter().flatten().max_by_key(|record| record.when).map(|record| record.title.clone());
        let root = self.detection.install().map(|install| install.root.clone());
        self.hero.want(root.as_deref(), newest.as_deref(), crate::library::cache_dir(&self.store.dir(), "banner"), crate::hero::today());
    }

    /// Work under way whose progress moves on screen: a download, an install, an import, a
    /// check, the game starting. (The idle animations never settle meanwhile.)
    fn working(&self) -> bool {
        self.installer.busy() || self.maps.busy() || self.skins.busy() || self.mod_validation.busy() || self.launching.is_some()
            || self.download.as_ref().is_some_and(|download| matches!(download.step, Step::Downloading { .. } | Step::Checking))
    }

    /// The logo is the website's link: a click (or Enter, with the keys on it) opens
    /// ta-reborn.com in the browser, as the launcher opens every link.
    fn website_link(&mut self, response: egui::Response) {
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, true, ta_ui::i18n::t("TA Reborn's website")));
        let response = response.hover_tip(ta_ui::i18n::t("Visit ta-reborn.com, TA Reborn's website, in your browser")).on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() {
            ta_ui::sound::play(&response.ctx, ta_ui::sound::UiSound::Button);
            self.actions.push(Action::OpenUrl(WEBSITE_URL.into()));
        }
    }

    fn compact_status(&mut self, ui: &mut Ui) { self.footer(ui, true); }

    /// The footer's word on the settings: saving, just saved, could not save, unreadable;
    /// nothing otherwise (they save by themselves, as the Settings page says).
    fn save_status_label(&self) -> Option<(&'static str, Color32)> {
        use ta_ui::i18n::n;
        match &self.store.status {
            SaveStatus::Failed(_) => Some((n("COULD NOT SAVE SETTINGS"), RED)),
            SaveStatus::Pending => Some((n("SAVING CHANGES…"), AMBER)),
            SaveStatus::Saved(at) if at.elapsed() < SAVED_SHOWN => Some((n("ALL CHANGES SAVED"), GREEN)),
            SaveStatus::Unchanged | SaveStatus::Saved(_) if self.store.load_error.is_some() => Some((n("SETTINGS FILE UNREADABLE — DEFAULTS IN USE"), AMBER)),
            SaveStatus::Unchanged | SaveStatus::Saved(_) => None,
        }
    }

    fn save_status_ui(&mut self, ui: &mut Ui) {
        use ta_ui::i18n::t;
        // ("Saved" goes again after a moment.)
        if let SaveStatus::Saved(at) = &self.store.status && at.elapsed() < SAVED_SHOWN {
            ui.ctx().request_repaint_after(SAVED_SHOWN - at.elapsed());
        }
        let status = self.save_status_label();
        ui.horizontal_wrapped(|ui| {
            self.shortcuts_button(ui);
            let Some((label, color)) = status else { return };
            let mark = match &self.store.status {
                SaveStatus::Failed(_) => '×', SaveStatus::Saved(_) if self.store.load_error.is_none() => '✓',
                SaveStatus::Unchanged | SaveStatus::Saved(_) if self.store.load_error.is_some() => '!', _ => '●',
            };
            let response = widgets::status_label(ui, &t(label), color, mark);
            if let SaveStatus::Failed(error) = &self.store.status {
                response.hover_tip(error);
                if ui.link(t("Retry")).clicked() { self.push(Action::RetrySave); }
            }
        });
    }

    fn footer(&mut self, ui: &mut Ui, compact: bool) {
        use ta_ui::i18n::t;
        let narrow = ui.available_width() < 700.0;
        let available = (ui.available_width() - 40.0).max(1.0);
        let copyright = widgets::wrapped_galley(ui, RichText::new(t("Total Annihilation is © its respective owners.")).font(body(10.0)),
            if narrow { available } else { available * 0.48 });
        let status_width = if narrow { available - 80.0 } else { available * 0.52 - 96.0 };
        let status = widgets::wrapped_galley(ui, RichText::new(self.save_status_label().map_or(String::new(), |(label, _)| t(label).to_string())).font(strong(10.0)), status_width.max(1.0));
        // (Narrow, the activities' chips take a row of their own between the two.)
        let chips = narrow && self.has_activities();
        let height = if narrow { copyright.size().y + status.size().y.max(20.0) + 30.0 + if chips { 30.0 } else { 0.0 } }
            else { (copyright.size().y.max(status.size().y) + 28.0).max(44.0) };
        egui::Panel::bottom("launcher_footer").exact_size(height).resizable(false).show_separator_line(false)
            .frame(Frame::NONE.inner_margin(Margin { left: 16, right: 16, top: 12, bottom: 4 })).show(ui, |ui| {
                let rect = ui.max_rect().expand2(vec2(16.0, 0.0)).with_min_y(ui.max_rect().min.y - 12.0);
                ui.painter().add(metal::backdrop(ui.ctx(), rect, BG_DEEP));
                let y = rect.min.y + FRAME_PIPE_AT;
                ui.painter().add(metal::pipe(pos2(rect.min.x, y), pos2(rect.max.x, y), FRAME_PIPE));
                // (The footer's words sit just under its pipe: nothing may hang down to them.)
                ta_ui::seasonal::trim(ui.painter(), rect.min.x, rect.max.x, y - FRAME_PIPE * 0.5, FRAME_PIPE, 2.0);
                if !compact { ui.painter().add(metal::joint(pos2(rect.min.x + NAV_WIDTH - FRAME_PIPE_AT, y), FRAME_PIPE + 6.0)); }
                if narrow {
                    self.save_status_ui(ui);
                    if chips { ui.horizontal(|ui| self.activity_chips(ui)); }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(t("Total Annihilation is © its respective owners.")).font(body(10.0)).color(STEEL)).wrap());
                    });
                } else {
                    // Fixed child rectangles keep the copyright anchored to the window edge; the
                    // save status at the left, and what is going on (the activities' chips) in the
                    // room between them.
                    let bounds = ui.available_rect_before_wrap();
                    let split = (bounds.right() - copyright.size().x - 8.0).max(bounds.left() + bounds.width() * 0.52);
                    let left = Rect::from_min_max(bounds.min, pos2(split - 16.0, bounds.bottom()));
                    let right = Rect::from_min_max(pos2(split, bounds.top()), bounds.max);
                    let mut status = ui.new_child(UiBuilder::new().max_rect(left).layout(Layout::top_down(Align::Min)));
                    self.save_status_ui(&mut status);
                    let used = status.min_rect().right();
                    let activities = Rect::from_min_max(pos2(used + 20.0, bounds.top()), pos2(split - 16.0, bounds.bottom()));
                    if activities.width() >= 80.0 {
                        let mut chips = ui.new_child(UiBuilder::new().max_rect(activities).layout(Layout::left_to_right(Align::Center)));
                        chips.set_clip_rect(chips.clip_rect().intersect(activities));
                        self.activity_chips(&mut chips);
                    }
                    let mut copyright = ui.new_child(UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)));
                    copyright.add(egui::Label::new(RichText::new(t("Total Annihilation is © its respective owners.")).font(body(10.0)).color(STEEL)).wrap());
                }
            });
    }

    fn header(&mut self, ui: &mut Ui) {
        egui::Panel::top("header")
            .exact_size(84.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::NONE)
            .show(ui, |ui| {
                let rect = ui.max_rect();
                let painter = ui.painter();
                // Dark plates, the pipe along the bottom meeting the rail's at a joint.
                painter.add(metal::backdrop(ui.ctx(), rect, BG_DEEP));
                let y = rect.max.y - FRAME_PIPE_AT;
                painter.add(metal::pipe(pos2(rect.min.x, y), pos2(rect.max.x, y), FRAME_PIPE));
                painter.add(metal::joint(pos2(rect.min.x + NAV_WIDTH - FRAME_PIPE_AT, y), FRAME_PIPE + 6.0));
                // (The header sits above the pipe: centred in what is left.)
                let band = Rect::from_min_max(rect.min, pos2(rect.max.x, y - FRAME_PIPE));
                let emblem = Rect::from_center_size(pos2(rect.min.x + 46.0, band.center().y), vec2(48.0, 48.0));
                let title = widgets::galley(ui, RichText::new("TA REBORN").font(display(31.0)).extra_letter_spacing(ta_ui::i18n::tracking(2.6)));
                let subtitle = |text: String| widgets::galley(ui, RichText::new(text).font(strong(11.5)).extra_letter_spacing(ta_ui::i18n::tracking(3.0)));
                let version = subtitle(ta_ui::i18n::tf("COMMAND LAUNCHER  ·  v{version}", &[("version", &env!("CARGO_PKG_VERSION"))]));
                // (Under the pointer the subtitle says where the logo leads, an arrow out after it.)
                let address = subtitle(WEBSITE_LABEL.to_string());
                let x = rect.min.x + 86.0;
                let top = band.center().y - (title.size().y + version.size().y - 2.0) / 2.0;
                let title_height = title.size().y;
                let words = Rect::from_min_size(pos2(x, top), vec2(title.size().x.max(version.size().x).max(address.size().x + 16.0), title_height + version.size().y - 2.0));
                let logo_bounds = emblem.union(words);
                // The emblem and the name are the website's link: they warm to brass under the
                // pointer (or with the keys), the emblem in a glow of its own.
                let logo = ui.interact(logo_bounds, ui.id().with("project_logo"), egui::Sense::click());
                let hot = link_heat(ui, &logo);
                let painter = ui.painter();
                if hot > 0.0 {
                    widgets::soft_glow(painter, emblem.center(), 18.0, 36.0, AMBER.gamma_multiply(0.22 * hot));
                }
                widgets::paint_emblem(painter, emblem);
                ta_ui::seasonal::logo(painter, emblem);
                // (The header ends 3.5 points under its pipe: what hangs from it stays inside.)
                ta_ui::seasonal::trim(painter, rect.min.x, rect.max.x, y - FRAME_PIPE * 0.5, FRAME_PIPE, 3.0);
                metal::embossed(painter, pos2(x, top), title, TEXT_BRIGHT.lerp_to_gamma(AMBER_BRIGHT, hot));
                let below = pos2(x + 1.0, top + title_height - 2.0);
                if hot < 1.0 {
                    painter.galley_with_override_text_color(below, version, STEEL.gamma_multiply(1.0 - hot));
                }
                if hot > 0.0 {
                    let (cap_top, cap_bottom) = metal::cap_band(&address);
                    let arrow = pos2(below.x + address.size().x + 10.0, below.y + (cap_top + cap_bottom) / 2.0);
                    painter.galley_with_override_text_color(below, address, AMBER.gamma_multiply(hot));
                    out_arrow(painter, arrow, 3.6, AMBER.gamma_multiply(hot));
                }
                widgets::focus_ring(ui, &logo, logo_bounds.expand(3.0));
                self.website_link(logo);

                let profile_rect = Rect::from_min_max(pos2(band.max.x - 288.0, band.center().y - 26.0), pos2(band.max.x - 24.0, band.center().y + 26.0));
                let mut profile = ui.new_child(UiBuilder::new().max_rect(profile_rect).layout(Layout::top_down(Align::Min)));
                self.profile_widget(&mut profile);
                // The music, a click away.
                let music = Rect::from_min_max(pos2(profile_rect.min.x - 16.0 - 84.0, band.center().y - 20.0), pos2(profile_rect.min.x - 16.0, band.center().y + 20.0));
                self.music_switch(ui, music);

            });
    }

    /// The header's music control at `rect`, doing what it is asked.
    fn music_switch(&mut self, ui: &mut Ui, rect: Rect) {
        let playing = self.sounds.now_playing().map(str::to_string);
        let (on, available) = (self.config.audio.launcher_music, self.sounds.has_music());
        match self.music_control.show(ui, rect, on, playing.as_deref(), &mut self.config.audio.launcher_music_volume, available) {
            Some(crate::music_control::MusicAction::Toggle) => self.config.audio.launcher_music = !on,
            Some(crate::music_control::MusicAction::Skip) => self.sounds.skip_track(),
            None => {}
        }
    }

    fn nav(&mut self, ui: &mut Ui) {
        egui::Panel::left("nav")
            .exact_size(NAV_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::NONE)
            .show(ui, |ui| {
                let rect = ui.max_rect();
                ui.painter().add(metal::backdrop(ui.ctx(), rect, BG_DEEP));
                let x = rect.max.x - FRAME_PIPE_AT;
                ui.painter().add(metal::pipe(pos2(x, rect.min.y), pos2(x, rect.max.y), FRAME_PIPE));
                // (The items stop short of the pipe.)
                let inside = Rect::from_min_max(rect.min, pos2(rect.max.x - 2.0 * FRAME_PIPE_AT - 2.0, rect.max.y));
                let ui = &mut ui.new_child(UiBuilder::new().max_rect(inside).layout(Layout::top_down(Align::Min)));
                ta_ui::seasonal::navigation(ui.painter(), Rect::from_min_size(inside.min + vec2(0.0, 6.0), vec2(inside.width(), space::XL)));
                ui.add_space(space::XL);
                ui.spacing_mut().item_spacing.y = 2.0;
                let missing = self.detection.install().is_none();
                let mut shown_at = None;
                for page in PAGES {
                    if starts_group(page) {
                        // (A seam between the groups.)
                        let (seam, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), egui::Sense::hover());
                        ui.painter().hline(seam.x_range().shrink(18.0), seam.center().y, egui::Stroke::new(1.0, LINE));
                    }
                    let (badge, why) = self.nav_badge(page);
                    let shortcut = polish::page_shortcut(page);
                    let hint = match why { Some(why) => format!("{shortcut} · {why}"), None => shortcut };
                    let item = widgets::nav_item_hinted(ui, page_icon(page), page_label(page), badge, self.page == page, widgets::NAV_ITEM_HEIGHT, Some(&hint));
                    if self.page == page {
                        shown_at = Some(item.rect);
                    }
                    if item.clicked() {
                        self.open_page(page);
                    }
                }
                if let Some(at) = shown_at {
                    widgets::nav_marker(ui, "pages", at);
                }
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.add_space(space::L);
                    // One unified launch control includes readiness on every page.
                    if self.page != Page::Play && !missing {
                        Frame::NONE.inner_margin(Margin { left: 16, right: 12, top: 0, bottom: 14 }).show(ui, |ui| {
                            let running = self.game_running();
                            let state = if running { widgets::PlayState::Running } else { widgets::PlayState::Ready };
                            let button = widgets::play_button_width(ui, state, ui.available_width());
                            if button.hover_tip(ta_ui::i18n::t("Start the game")).clicked() {
                                self.actions.push(Action::Play);
                            }
                        });
                    }
                });
            });
    }

    fn status_bar(&mut self, ui: &mut Ui) { self.footer(ui, false); }

    fn central(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default().frame(Frame::NONE).show(ui, |ui| {
            let rect = ui.max_rect();
            ui.painter().add(metal::backdrop(ui.ctx(), rect, BG));
            ta_ui::seasonal::background(ui.painter(), rect);
            // The header, the rail and the footer stand over the page: their shadows fall on it.
            widgets::gradient_v(ui.painter(), Rect::from_min_size(rect.min, vec2(rect.width(), 20.0)), Color32::from_black_alpha(120), Color32::TRANSPARENT);
            widgets::gradient_h(ui.painter(), Rect::from_min_size(rect.min, vec2(16.0, rect.height())), Color32::from_black_alpha(100), Color32::TRANSPARENT);
            widgets::gradient_v(ui.painter(), Rect::from_min_max(pos2(rect.min.x, rect.max.y - 12.0), rect.max), Color32::TRANSPARENT, Color32::from_black_alpha(90));
            let compact = compact_layout(ui.ctx().content_rect().size());
            // A new page fades in as it rises from below (further down the rail than the page
            // before) or settles from above (further up), on every page alike.
            let now = ui.input(|i| i.time);
            if self.shown_page != self.page {
                let place = |page: Page| PAGES.iter().position(|&p| p == page).unwrap_or(0);
                self.page_from = if place(self.page) >= place(self.shown_page) { 1.0 } else { -1.0 };
                self.shown_page = self.page;
                self.page_since = now;
            }
            let t = if ta_ui::motion::reduced(ui.ctx()) { 1.0 } else { ((now - self.page_since) / PAGE_TIME).clamp(0.0, 1.0) as f32 };
            if t < 1.0 {
                ui.ctx().request_repaint();
            }
            let eased = egui::emath::easing::cubic_out(t);
            let entrance = egui::emath::TSTransform::from_translation(vec2(0.0, PAGE_RISE * self.page_from * (1.0 - eased)));
            // (Faint at once, never a blank frame between two pages.)
            let eased = 0.2 + 0.8 * eased;
            if matches!(self.page, Page::Skins | Page::Maps | Page::Mods | Page::Profile) {
                let margin = if compact { 12.0 } else { 28.0 };
                let width = (rect.width() - 2.0 * margin).max(1.0).min(content_width(self.page));
                let bounds = Rect::from_min_size(pos2(rect.center().x - width / 2.0, rect.min.y + margin), vec2(width, (rect.height() - 2.0 * margin).max(1.0)));
                let mut page = ui.new_child(UiBuilder::new().max_rect(bounds).layout(Layout::top_down(Align::Min)));
                page.set_clip_rect(page.clip_rect().intersect(bounds));
                page.multiply_opacity(eased);
                // At large interface scales, a library's toolbar can consume the
                // entire viewport. Give its inner panes finite working room and
                // let the whole page scroll instead of clipping them out forever.
                let page_id = self.page;
                let offset = self.capture_scroll;
                let mut draw = |ui: &mut Ui| match page_id {
                    Page::Maps => self.maps_page(ui),
                    Page::Mods => self.mods_page(ui),
                    Page::Profile => self.profile_page(ui),
                    _ => self.skins_page(ui),
                };
                const MIN_LIBRARY_HEIGHT: f32 = 520.0;
                if bounds.height() < MIN_LIBRARY_HEIGHT {
                    let mut area = ScrollArea::vertical().id_salt(("launcher_library_page", page_id))
                        .max_height(bounds.height()).auto_shrink([false, false]);
                    if let Some(offset) = offset { area = area.vertical_scroll_offset(offset); }
                    ta_ui::scroll::show(&mut page, area, ta_ui::scroll::Behind::Backdrop(BG), |ui| {
                        ui.with_visual_transform(entrance, |ui| {
                            ui.allocate_ui_with_layout(vec2(ui.available_width(), MIN_LIBRARY_HEIGHT), Layout::top_down(Align::Min), &mut draw);
                        });
                    });
                } else {
                    page.with_visual_transform(entrance, |ui| draw(ui));
                }
                self.paint_notice(ui, rect);
                self.paint_launch(ui, rect);
                return;
            }
            let mut scroll_area = ScrollArea::vertical().id_salt(("launcher_page", self.page)).auto_shrink([false, false]);
            if std::mem::take(&mut self.reset_activity_scroll) {
                scroll_area = scroll_area.vertical_scroll_offset(0.0);
            } else if let Some(offset) = self.capture_scroll {
                scroll_area = scroll_area.vertical_scroll_offset(offset);
            }
            ta_ui::scroll::show(ui, scroll_area, ta_ui::scroll::Behind::Backdrop(BG), |ui| {
                // A centred column, 28 pt from the edges, at most the page's width wide.
                let width = ui.available_width();
                let margin = if compact { 12.0 } else { 28.0 };
                let column = (width - 2.0 * margin).clamp(200.0, content_width(self.page));
                ui.add_space(if compact { 12.0 } else { 26.0 });
                ui.horizontal(|ui| {
                    ui.add_space(((width - column) / 2.0).max(0.0));
                    ui.vertical(|ui| {
                        ui.set_width(column);
                        ui.multiply_opacity(eased);
                        ui.with_visual_transform(entrance, |ui| match self.page {
                            Page::Play => self.play_page(ui),
                            Page::Mods => self.mods_page(ui),
                            Page::Maps => self.maps_page(ui),
                            Page::Skins => self.skins_page(ui),
                            Page::Profile => self.profile_page(ui),
                            Page::Settings => self.settings_page(ui),
                            Page::Benchmark => self.benchmark_page(ui),
                            Page::About => self.about_page(ui),
                        });
                    });
                });
                ui.add_space(if compact { 12.0 } else { 30.0 });
            });
            self.paint_notice(ui, rect);
            self.paint_launch(ui, rect);
        });
    }

    /// The Play button was pressed: the battle station powers up (a lamp, the words, a brass
    /// bar filling) for a moment, while the game starts.
    fn paint_launch(&mut self, ui: &mut Ui, area: Rect) {
        let Some(since) = self.launching else { return };
        let now = ui.input(|i| i.time);
        let t = (now - since) as f32;
        const LENGTH: f32 = 1.8;
        if t > LENGTH || (!self.game.is_running() && self.film.is_none()) {
            self.launching = None;
            return;
        }
        let still = ta_ui::motion::still(ui.ctx());
        if still {
            ui.ctx().request_repaint_after(Duration::from_secs_f32((LENGTH - t).max(0.0)));
        } else {
            ui.ctx().request_repaint();
        }
        let fade = if still { 1.0 } else { (t / 0.2).min(1.0).min((LENGTH - t) / 0.3) };
        // A plated band across the page, opening from its middle line (at once when still), the
        // emblem beside the words, a scan crossing it once.
        let open = if still { 1.0 } else { egui::emath::easing::cubic_out((t / 0.32).min(1.0)) * ((LENGTH - t) / 0.3).min(1.0) };
        let painter = ui.painter();
        painter.rect_filled(area, CornerRadius::ZERO, Color32::from_black_alpha((215.0 * fade) as u8));
        let c = area.center();
        let room = (area.width() - 48.0).max(1.0);
        let title = widgets::wrapped_galley(ui, RichText::new(ta_ui::i18n::t("DEPLOYING COMMANDER")).font(display(34.0)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(2.0)), (room - 90.0).max(1.0));
        let sub = widgets::wrapped_galley(ui, RichText::new(ta_ui::i18n::t("TA REBORN IS STARTING")).font(strong(12.5)).color(STEEL).extra_letter_spacing(ta_ui::i18n::tracking(2.4)), (room - 90.0).max(1.0));
        let words = vec2(title.size().x.max(sub.size().x).max(240.0), title.size().y + 6.0 + sub.size().y + 22.0 + 10.0);
        let band = Rect::from_center_size(c, vec2(area.width(), (words.y + 56.0) * open));
        let ui_painter = ui.painter();
        ui_painter.add(egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha((170.0 * fade) as u8) }.as_shape(band, CornerRadius::ZERO));
        let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(band));
        painter.add(metal::plate(ui.ctx(), band, PANEL));
        for y in [band.min.y, band.max.y] {
            ui_painter.hline(band.x_range(), y, egui::Stroke::new(2.0, AMBER_DIM.gamma_multiply(fade)));
        }
        let content = fade * ((open - 0.5) / 0.5).clamp(0.0, 1.0);
        if !still && open > 0.05 {
            // (The scan: across the band in a second, its afterglow behind it.)
            let x = band.min.x + band.width() * ((t - 0.15) / 1.1).clamp(0.0, 1.0);
            let trail = Rect::from_min_max(pos2(x - 180.0, band.min.y), pos2(x, band.max.y));
            painter.add(metal::hgrad(trail, Color32::from_rgba_unmultiplied(120, 214, 92, 0), Color32::from_rgba_unmultiplied(120, 214, 92, (30.0 * fade) as u8)));
            painter.vline(x, band.y_range(), egui::Stroke::new(1.5, Color32::from_rgba_unmultiplied(120, 214, 92, (110.0 * fade) as u8)));
        }
        let emblem = 64.0;
        let left = c.x - (emblem + 22.0 + words.x) / 2.0;
        let top = c.y - words.y / 2.0;
        let emblem_rect = Rect::from_center_size(pos2(left + emblem / 2.0, c.y), vec2(emblem, emblem));
        let mut faded = painter.clone();
        faded.set_opacity(content);
        widgets::paint_emblem(&faded, emblem_rect);
        let lamp = if still || self.config.gameplay.reduced_flashes { 1.0 } else { 0.5 + 0.5 * (t * 14.0).sin().abs() };
        let x = emblem_rect.max.x + 22.0;
        faded.add(metal::lamp(pos2(emblem_rect.max.x - 4.0, emblem_rect.max.y - 4.0), 5.0, GREEN, lamp));
        metal::embossed(&faded, pos2(x, top), title.clone(), TEXT_BRIGHT);
        faded.galley(pos2(x + 1.0, top + title.size().y + 6.0), sub.clone(), STEEL);
        let bar = Rect::from_min_size(pos2(x + 1.0, top + words.y - 10.0), vec2(words.x.min(360.0), 10.0));
        let mut progress_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar));
        progress_ui.set_clip_rect(progress_ui.clip_rect().intersect(band));
        progress_ui.set_opacity(content);
        ta_ui::progress::paint(&progress_ui, bar, None, true);
    }

    fn paint_notice(&mut self, ui: &mut Ui, area: Rect) {
        let Some(notice) = &mut self.notice else { return };
        let now = ui.input(|i| i.time);
        if notice.since.is_none() {
            let sound = match notice.color {
                RED => Some(ta_ui::sound::UiSound::Refuse),
                GREEN => Some(ta_ui::sound::UiSound::Done),
                AMBER => Some(ta_ui::sound::UiSound::Message),
                _ => None,
            };
            if let Some(sound) = sound {
                ta_ui::sound::play(ui.ctx(), sound);
            }
        }
        let age = now - *notice.since.get_or_insert(now);
        // (An error stays until it is dismissed; others go after their time, the next after.)
        if age >= NOTICE_TIME && notice.color != RED {
            self.notice = self.notices.pop_front();
            ui.ctx().request_repaint();
            return;
        }
        let waiting = self.notices.len();
        let Some(notice) = &self.notice else { return };
        // (It slides in from the right.)
        let slide = if ta_ui::motion::reduced(ui.ctx()) { 0.0 } else { 1.0 - (age as f32 / 0.25).clamp(0.0, 1.0) };
        let error = notice.color == RED;
        if slide > 0.0 {
            ui.ctx().request_repaint();
        } else if !error {
            ui.ctx().request_repaint_after(Duration::from_secs_f64(NOTICE_TIME - age));
        }
        // (Room at its right for the close button, and how many more wait.)
        let more = (waiting > 0).then(|| ta_ui::i18n::tf("+{n} MORE", &[("n", &waiting)]));
        let width = (area.width() - 2.0 * space::L).min(600.0);
        let right = 44.0 + if more.is_some() { 64.0 } else { 0.0 };
        let text = widgets::wrapped_galley(ui, RichText::new(&notice.text).font(strong(13.0)).color(TEXT_BRIGHT), (width - 40.0 - right).max(1.0));
        let size = vec2((text.size().x + 40.0 + right).min(width), (text.size().y + 24.0).max(44.0));
        let rect = Align2::RIGHT_BOTTOM.align_size_within_rect(size, area.shrink(space::L)).translate(vec2(slide.powi(2) * (size.x + 40.0), 0.0));
        let colour = notice.color;
        let painter = ui.painter().clone();
        painter.add(egui::Shadow { offset: [0, 6], blur: 16, spread: 0, color: Color32::from_black_alpha(150) }.as_shape(rect, CornerRadius::same(3)));
        painter.add(metal::riveted_plate(ui.ctx(), rect, PANEL_HI, 0.0));
        painter.add(metal::lamp(pos2(rect.min.x + 24.0, rect.center().y), 4.5, colour, 1.0));
        painter.galley(pos2(rect.min.x + 40.0, rect.center().y - text.size().y / 2.0), text, TEXT_BRIGHT);
        if let Some(more) = more {
            painter.text(pos2(rect.max.x - 44.0, rect.center().y), Align2::RIGHT_CENTER, more, strong(10.5), STEEL);
        }
        // Its close button; the pointer over it holds it (an error stays anyway).
        let close = Rect::from_center_size(pos2(rect.max.x - 22.0, rect.center().y), vec2(26.0, 26.0));
        let dismissed = ui.scope_builder(egui::UiBuilder::new().max_rect(close), |ui| {
            widgets::icon_button(ui, widgets::Glyph::Close, true).hover_tip(ta_ui::i18n::t("Dismiss")).clicked()
        }).inner;
        let hovered = ui.rect_contains_pointer(rect);
        if dismissed {
            self.notice = self.notices.pop_front();
            ui.ctx().request_repaint();
        } else if hovered && !error && let Some(notice) = &mut self.notice {
            notice.since = Some(now - age.min(NOTICE_TIME * 0.5));
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
    }
}

/// How warm a link is (0 at rest, 1 under the pointer or with the keys): it warms over a moment,
/// at once under reduced motion.
fn link_heat(ui: &Ui, response: &egui::Response) -> f32 {
    let hot = response.hovered() || response.has_focus();
    if ta_ui::motion::reduced(ui.ctx()) {
        return if hot { 1.0 } else { 0.0 };
    }
    ui.ctx().animate_bool_with_time(response.id.with("hot"), hot, 0.12)
}

/// An arrow up and out to the right (a link that leaves the launcher), `reach` from its centre
/// to its tip.
fn out_arrow(painter: &egui::Painter, centre: egui::Pos2, reach: f32, colour: Color32) {
    let stroke = egui::Stroke::new(1.4, colour);
    let (tail, tip) = (centre + vec2(-reach, reach), centre + vec2(reach, -reach));
    painter.line_segment([tail, tip], stroke);
    painter.add(egui::Shape::line(vec![tip + vec2(-reach * 1.2, 0.0), tip, tip + vec2(0.0, reach * 1.2)], stroke));
}

/// Mod folders for the current settings.
/// Working folders an earlier run left behind, untouched for `age` (closed mid-download, an
/// update's setup long since run, a map import cut short: each can hold gigabytes), and an
/// install stopped half way (closed by force, the power gone) put right. Never this run's own.
fn sweep_working_folders(cache: &Path, mods_dir: &Path, age: Duration) {
    crate::getmods::sweep_stale(&cache.join("updates"), "setup", age);
    crate::getmods::sweep_stale(&cache.join("moddb").join("catalogue"), "install", age);
    crate::getmods::sweep_stale(&cache.join("moddb").join("unpack"), "install", age);
    crate::getmods::sweep_stale(&cache.join("map-imports"), ".maps", age);
    for done in ta_vfs::package::recover(mods_dir) { log::info!("mods folder: {done}"); }
    crate::getmods::sweep_stale_imports(mods_dir, age);
}

fn mod_folders(config: &Config, store: &ConfigStore) -> Vec<ModFolder> {
    modlist::mod_folders(launch::app_dir().as_deref(), &store.dir(), config.mods.extra_dir.as_deref())
}

/// Offline sessions may inspect a deliberately configured local release, but never contact
/// a remote source, including after the source or the check preference changes in About.
fn update_check(config: &Config, offline: bool, manual: bool) -> UpdateState {
    let local = config.updates.url.trim_start().starts_with("file:");
    let enabled = manual || config.updates.check;
    match (offline && !local, enabled) {
        (true, true) => UpdateState::NoSource,
        (true, false) => UpdateState::Off,
        (false, _) => UpdateState::start(enabled, &config.updates.url),
    }
}

impl LauncherApp {
    /// Once, as the window first shows: a window taller or wider than the screen leaves (a small
    /// or scaled-up display: 1280 × 720 points on a 1080p laptop at 150%) shrinks to fit above
    /// the taskbar, down to the smallest size the pages are laid out for.
    fn fit_to_screen(&mut self, ctx: &egui::Context) {
        if self.fitted {
            return;
        }
        let (monitor, outer, inner, maximized, fullscreen) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().outer_rect, i.viewport().inner_rect, i.viewport().maximized, i.viewport().fullscreen));
        let (Some(monitor), Some(outer), Some(inner)) = (monitor, outer, inner) else { return };
        self.fitted = true;
        // (A maximised window, or one full screen, fills the screen as it should.)
        if maximized == Some(true) || fullscreen == Some(true) {
            return;
        }
        // (The monitor's size includes the taskbar; the window's frame and title bar come on top
        // of its inside.)
        let zoom = ctx.zoom_factor();
        let chrome = outer.size() - inner.size();
        let room = (monitor - chrome - egui::vec2(16.0, 64.0) / zoom).max(egui::vec2(1.0, 1.0));
        let size = inner.size();
        if size.x <= room.x && size.y <= room.y {
            return;
        }
        let minimum = (egui::Vec2::from(ta_config::limits::LAUNCHER_POINTS) / zoom).min(room);
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(minimum));
        let fit = size.min(room).max(minimum);
        log::info!("window {:.0} x {:.0} does not fit the screen ({:.0} x {:.0}): {:.0} x {:.0}", size.x, size.y, monitor.x, monitor.y, fit.x, fit.y);
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(fit));
        // Keep the window on its current display. Monitor size carries no desktop origin,
        // so centering at (0, 0) would move windows from a secondary display to the primary.
    }
}

impl eframe::App for LauncherApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(last) = &mut self.paced {
            let gap = Duration::from_secs_f64(1.0 / MAX_FPS);
            let since = last.elapsed();
            if since < gap {
                std::thread::sleep(gap - since);
                repaint_slept(ctx, gap - since);
            }
            *last = Instant::now();
        }
        repaint_causes(ctx);
        // The window closed with work at risk: asked first (the question in `close_dialog`).
        if ctx.input(|i| i.viewport().close_requested()) && !self.close_confirmed && self.film.is_none() && self.capture.is_none() {
            // on_exit cannot keep the window open after a failed write. Save here,
            // before accepting the close, while Retry can still preserve the edits.
            if self.store.flush(&mut self.config).is_ok() { self.sync_profile_name(); }
            if !self.work_at_risk().is_empty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                self.confirm_close = true;
            }
        }
        // Another launcher was started: this one comes to the front instead (minimised too:
        // then this is all that runs).
        if self.instance.as_ref().is_some_and(crate::instance::Instance::take_wanted) {
            log::info!("another launcher was started: this one comes to the front");
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            // (Started to show a mod: the game asked for one a battle needs.)
            if let Some(id) = crate::instance::take_mod_request() {
                self.open_mod(&id);
            }
        }
        let was_running = matches!(self.game, launch::GameState::Running { .. });
        let logs = self.log_dir();
        if launch::poll(&mut self.game, &logs) {
            ctx.request_repaint_after(Duration::from_millis(500));
        } else if was_running {
            // (Ended from here: no crash to report.)
            if std::mem::take(&mut self.ending_game) {
                self.game = GameState::Idle;
                self.notify("The game was ended", AMBER);
                self.game_ended(ctx);
            } else if ta_config::running::take_restart(&self.store.dir()) {
                // (It started itself again, with a battle's mods: the new one is followed.)
                self.elsewhere = Elsewhere::Restarting(Instant::now());
                self.elsewhere_checked = None;
            } else {
                self.game_ended(ctx);
            }
        }
        self.follow_game_elsewhere(ctx);
        self.store.tick(&mut self.config, ctx);
        self.tell_recovery();
        self.installer.set_game_running(self.game_running());
        // logic runs even with the window minimized: completed installs must stop blocking
        // a ready update and make their results available when the launcher comes back.
        self.installs(ctx);
        self.mod_validation.set_paused(self.game_running() || self.installer.busy() || self.maps.busy());
        // A check just made that found errors in a mod that is on: said, not only badged.
        if let Some(done) = self.mod_validation.tick(ctx)
            && done.status == ta_data::validation::Status::Errors
            && self.config.mods.enabled.contains(&done.id)
        {
            let name = self.mods.get(&done.id).map_or_else(|| done.id.clone(), |m| m.info.manifest.name.clone());
            self.notify(ta_ui::i18n::tf("{mod}: the data check found errors. MOD INFO on the Mods page lists them.", &[("mod", &name)]), RED);
        }
        self.maps_tick(ctx);
        if self.skins.poll() {
            if let Some((side, id)) = self.skins.take_selected() {
                match side {
                    crate::skins::Side::Friendly => self.config.graphics.unit_skin = id,
                    crate::skins::Side::Enemy => self.config.graphics.enemy_unit_skin = id,
                }
            }
            ctx.request_repaint();
        }
        if self.skins.busy() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if self.update.poll() {
            ctx.request_repaint();
        } else if matches!(self.update, UpdateState::Checking(_)) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        self.updates(ctx);
        // (Here, not in `ui`: the music fades out as the window goes down to the taskbar, and
        // goes on to its next track while nothing is drawn.)
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        self.sounds.music(ctx, &self.config.audio, minimized || self.game_running());
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(film) = &mut self.film {
            film.input(raw_input);
            return;
        }
        // The Menu key reaches egui as no key at all: it is read here, and given as Shift+F10
        // (which opens a library's context menu on the card or row with the keys).
        if crate::library::menu::menu_key(&mut self.menu_key, raw_input.focused) {
            raw_input.events.push(egui::Event::Key { key: egui::Key::F10, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::SHIFT });
        }
        self.pad_input(ctx, raw_input);
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        repaint_cost(ui.ctx(), frame.info().cpu_usage);
        let ctx = ui.ctx().clone();
        if self.film.is_none() {
            let scale = self.config.gameplay.ui_scale.clamp(*ta_config::limits::UI_SCALE.start(), *ta_config::limits::UI_SCALE.end());
            if (ctx.zoom_factor() - scale).abs() > 0.001 {
                ctx.set_zoom_factor(scale);
            }
        }
        // The radar's sweep and the lamps' pulse hold still while the window is in the
        // background or minimised (the game is usually in front, and each frame drawn here
        // would be one taken from it).
        let (focused, minimized) = ui.input(|i| (i.viewport().focused.unwrap_or(true), i.viewport().minimized.unwrap_or(false)));
        // (And after a minute with nothing touched: see SETTLE.)
        let now = ctx.input(|i| i.time);
        if ctx.input(|i| i.raw.events.iter().any(|e| !matches!(e, egui::Event::Screenshot { .. }))) {
            self.last_input = now;
        }
        let settled = self.film.is_none() && self.capture.is_none() && now - self.last_input > SETTLE && !self.working();
        ta_ui::motion::set_still(&ctx, minimized || !focused || settled);
        ta_ui::motion::set_reduced(&ctx, self.config.gameplay.reduced_motion);
        // The idle animations' clock moves on while they may move (a long frame counts short).
        if !ta_ui::motion::still(&ctx) {
            self.ambient += (now - self.ambient_frame).clamp(0.0, 0.1);
        }
        self.ambient_frame = now;
        ta_ui::tooltip::set_pointer_clearance(&ctx, crate::cursor::tooltip_clearance(&ctx, &self.config));
        ta_ui::seasonal::begin(&ctx, &self.config.launcher, self.config.gameplay.reduced_flashes);
        self.film_steps(&ctx);
        self.fit_to_screen(&ctx);
        self.keep_window(&ctx);
        self.sounds.begin_frame(&ctx, &self.config.audio);
        self.skins.browser.preview.begin_frame();
        if self.battles.poll(&ctx) {
            ctx.request_repaint();
        }
        self.banner_picture(&ctx);
        self.dropped_files(&ctx);
        self.shortcuts(&ctx);
        self.mods.refresh(&self.config.mods.enabled);
        if compact_layout(ctx.content_rect().size()) {
            self.compact_header(ui);
            self.compact_status(ui);
            self.nav_compact(ui);
        } else {
            self.header(ui);
            self.status_bar(ui);
            self.nav(ui);
        }
        self.central(ui);
        self.shortcuts_dialog(&ctx);
        self.end_game_dialog(&ctx);
        self.progress_gallery(&ctx);
        self.uninstall_dialog(&ctx);
        self.replace_dialog(&ctx);
        self.skin_dialogs(&ctx);
        self.conversion_dialog(&ctx);
        self.battle_dialog(&ctx);
        self.mod_folders_dialog(&ctx);
        self.whats_new_dialog(&ctx);
        self.mod_report_dialog(&ctx);
        self.paint_drop_target(&ctx);
        self.profile_dialog(&ctx);
        // Closing must remain actionable above any editor or other open dialog.
        self.close_dialog(&ctx);
        // (The profile menu and the captures ask for the Profile page at a tab.)
        if let Some(tab) = self.profiles.open_page.take() {
            self.open_page(Page::Profile);
            self.profiles.tab = tab;
        }
        // Achievements earned from the service record: their notices under the profile card.
        if !self.profiles.toasts.is_empty() && !self.profiles.dialog_open() && (self.capture.is_none() || self.toasts_in_capture) {
            let r = ctx.content_rect();
            self.profiles.toasts.show(&ctx, Rect::from_min_max(pos2(r.min.x, r.min.y + 76.0), r.max));
        }
        let actions = std::mem::take(&mut self.actions);
        if !actions.is_empty() {
            // Show the outcome right away rather than on the next input event.
            ctx.request_repaint();
        }
        let was_running = self.game.is_running();
        // Filming previews the same feedback without starting an actual game.
        let preview_launch = self.film.is_some() && actions.iter().any(|action| matches!(action, Action::Play | Action::PlayWith(_) | Action::Benchmark | Action::QuickBenchmark));
        for action in actions {
            self.apply(action, frame);
        }
        self.sounds.update(&ctx, &self.config.audio);
        if preview_launch || (!was_running && self.game.is_running()) {
            self.launching = Some(ctx.input(|i| i.time));
        }
        if let Some(text) = self.copy.take() {
            ctx.copy_text(text);
        }
        // The game has started: the window goes down to the taskbar, or closes, as the player
        // has it (after the deployment's moment on screen).
        if let Some(then) = self.after_launch
            && self.launching.is_none()
        {
            self.after_launch = None;
            match then {
                ta_config::OnPlay::Close if self.game_running() && !self.installer.busy() && !self.skins.busy() && !self.maps.busy() && self.download.is_none() => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                ta_config::OnPlay::Close | ta_config::OnPlay::Minimise if self.game_running() => {
                    self.minimised_for_game = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                _ => {}
            }
        }
        self.store.tick(&mut self.config, &ctx);
        // (A name being typed on the Settings page is not taken back half typed: it becomes
        // the profile's once the page is left, the game starts or the launcher closes.)
        if self.page != Page::Settings {
            self.sync_profile_name();
        }
        crate::cursor::show(&ctx, &self.config);
        // A drag in the load order ends where the button is let go, whatever was shown then: one
        // let go while the order was out of sight moves nothing (the order's view has dropped one
        // let go over it, this frame).
        if self.mod_library.dragging.is_some() && ctx.input(|i| !i.pointer.any_down()) {
            self.mod_library.dragging = None;
        }
        let bundled_pictures_ready = self.page != Page::Mods || self.mods_tab != ModsTab::Library || crate::catalogue::MODS.iter()
            .filter(|entry| entry.bundled_image().is_some()).all(|entry| self.media.pictures.contains_key(entry.id))
            && (self.page != Page::Mods || self.mod_library.changes.idle());
        if let Some(capture) = &mut self.capture {
            capture.tick(&ctx, !self.battles.loading() && !self.hero.loading() && !self.worlds.loading() && !self.skins.busy() && self.mod_validation.selected.as_ref().is_none_or(|id| !matches!(self.mod_validation.status(id), crate::validation::ValidationStatus::Queued | crate::validation::ValidationStatus::Checking)) && (self.page != Page::Skins || self.skins.preview_ready()) && (self.page != Page::Maps || self.maps.ready()) && self.profiles.capture_ready(self.page == Page::Profile) && bundled_pictures_ready);
        }
        let pictures = crate::catalogue::MODS.iter().filter(|e| !e.image.is_empty() || e.bundled_image().is_some()).count();
        let ready = self.sounds.ready() && self.media.pictures.len() >= pictures && !self.battles.loading() && !self.hero.loading() && !self.worlds.loading()
            && (self.page != Page::Maps || self.maps.ready()) && self.profiles.capture_ready(self.page == Page::Profile);
        if let Some(film) = &mut self.film {
            let sound = self.sounds.take_named(ui.ctx());
            film.after(ui.ctx(), ready, sound);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.mod_validation.shutdown();
        self.installer.shutdown();
        // A publication may finish after the final UI frame or while shutdown waits.
        // Apply its on/off decision before flushing the profile, just as in logic().
        self.installs(&egui::Context::default());
        for id in self.maps.finish(&egui::Context::default()) {
            modlist::set_enabled(&mut self.config.mods.enabled, &id, true);
        }
        self.skins.shutdown();
        if let Some((side, id)) = self.skins.take_selected() {
            match side {
                crate::skins::Side::Friendly => self.config.graphics.unit_skin = id,
                crate::skins::Side::Enemy => self.config.graphics.enemy_unit_skin = id,
            }
        }
        // (An update's download stops; a setup already started goes on.)
        self.download = None;
        // The window as the player left it, for the next start.
        if let Some((size, maximized)) = self.window {
            self.config.launcher.window = Some(size);
            self.config.launcher.maximized = maximized;
        }
        if self.store.flush(&mut self.config).is_ok() {
            self.sync_profile_name();
        }
    }
}

/// `TA_REPAINT_CAUSES=1`: what the frames cost the interface's thread (seconds, summed; eframe's
/// measure of the frame before), logged with the causes (see [`repaint_causes`]).
fn repaint_cost(ctx: &egui::Context, cpu: Option<f32>) {
    if std::env::var_os("TA_REPAINT_CAUSES").is_some() {
        let id = egui::Id::new("repaint_cost");
        ctx.data_mut(|d| *d.get_temp_mut_or_default::<f32>(id) += cpu.unwrap_or(0.0));
    }
}

/// `TA_REPAINT_CAUSES=1`: time the frames spent asleep, paced ([`MAX_FPS`]): not their cost.
fn repaint_slept(ctx: &egui::Context, slept: Duration) {
    if std::env::var_os("TA_REPAINT_CAUSES").is_some() {
        ctx.data_mut(|d| *d.get_temp_mut_or_default::<f32>(egui::Id::new("repaint_cost")) -= slept.as_secs_f32());
    }
}

/// `TA_REPAINT_CAUSES=1`: every two seconds, how many frames were drawn and what asked for the
/// last of them (a window left open should draw next to nothing).
fn repaint_causes(ctx: &egui::Context) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("TA_REPAINT_CAUSES").is_some()) {
        return;
    }
    let id = egui::Id::new("repaint_causes");
    let now = ctx.input(|i| i.time);
    let (since, frames) = ctx.data_mut(|d| {
        let entry = d.get_temp_mut_or_insert_with(id, || (now, 0u32));
        entry.1 += 1;
        *entry
    });
    if now - since >= 2.0 {
        let cost = ctx.data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<f32>(egui::Id::new("repaint_cost"))));
        log::info!("{frames} frames in {:.1} s, {:.1} ms each in the interface; the last asked for by {:?}", now - since, cost * 1000.0 / frames.max(1) as f32, ctx.repaint_causes());
        ctx.data_mut(|d| d.insert_temp(id, (now, 0u32)));
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::Parser as _;
    use ta_vfs::install::InstallError;
    use eframe::App as _;
    use egui::{Pos2, RawInput};

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ta_launcher_app_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A settings file that could not be read: once the defaults are saved over it (with a
    /// commander's profile that is at once, their name saved), the footer's warning goes, and the
    /// player is told where the unreadable file was kept, until they dismiss it.
    #[test]
    fn a_settings_file_replaced_by_defaults_is_told_with_its_copy_until_dismissed() {
        let dir = temp_dir("recovery_notice");
        std::fs::write(dir.join("config.toml"), "graphics = [[[ broken by hand").unwrap();
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        assert!(app.store.load_error.is_some());
        app.config.audio.music = 0.3;
        app.store.flush(&mut app.config).unwrap();
        assert!(app.store.load_error.is_none(), "the footer's warning goes with the save");
        let backup = app.store.backup.clone().expect("the unreadable file is kept");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "graphics = [[[ broken by hand");
        let told = |app: &LauncherApp| app.notice.iter().chain(&app.notices)
            .any(|notice| notice.color == RED && notice.text.contains(&backup.display().to_string()));
        let mut input = RawInput { time: Some(1.0), ..Default::default() };
        frame(&ctx, &mut app, input.clone());
        assert!(told(&app), "the player is told where the unreadable file is");
        input.time = Some(1.0 + NOTICE_TIME * 3.0);
        frame(&ctx, &mut app, input.clone());
        assert!(told(&app), "and it stays until dismissed");
        app.notice = None;
        input.time = Some(2.0 + NOTICE_TIME * 3.0);
        frame(&ctx, &mut app, input);
        assert!(!told(&app), "once");
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A map import cut short (the launcher killed, the power gone) leaves its private copy of the
    /// archive and what it unpacked: the next start sweeps it away, as it does a mod's download.
    #[test]
    fn working_folders_left_by_a_map_import_are_swept() {
        let dir = temp_dir("sweep_map_imports");
        let imports = dir.join("cache").join("map-imports");
        let left = imports.join(".maps-1-1793000000000000000-0");
        let ours = imports.join(format!(".maps-{}-1793000000000000000-0", std::process::id()));
        let unrelated = imports.join("notes");
        for folder in [&left, &ours, &unrelated] {
            std::fs::create_dir_all(folder.join("unpacked")).unwrap();
            std::fs::write(folder.join("unpacked").join("big.ufo"), b"HAPI").unwrap();
        }
        sweep_working_folders(&dir.join("cache"), &dir.join("mods"), Duration::ZERO);
        assert!(!left.exists(), "another run's import workspace is removed");
        assert!(ours.exists(), "an import under way in this run is left alone");
        assert!(unrelated.exists(), "only the imports' own folders are touched");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn launcher(config: &Path, extra: &[&str]) -> (egui::Context, LauncherApp) {
        let mut argv = vec!["ta-launcher", "--offline", "--config", config.to_str().unwrap()];
        argv.extend_from_slice(extra);
        let ctx = egui::Context::default();
        let app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), Args::parse_from(argv));
        (ctx, app)
    }

    /// A mod a battle needs that the library has no card of (not installed, not in the catalogue:
    /// a player's own) is said so; the library no longer opens on its first card, another mod's,
    /// as if that were the one to get. One installed, or in the catalogue, is shown as before.
    #[test]
    fn a_wanted_mod_the_launcher_cannot_get_is_said_so_not_shown_as_another() {
        let dir = temp_dir("wanted_mod");
        std::fs::create_dir_all(dir.join("mods/mine")).unwrap();
        std::fs::write(dir.join("mods/mine/mod.toml"), "id = \"mine\"\nname = \"Mine\"\n").unwrap();
        let (_ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        let told = |app: &LauncherApp, id: &str| app.notice.iter().chain(&app.notices).any(|notice| notice.text.contains(id));
        app.open_mod("qa_private_mod");
        assert_eq!(app.page, Page::Mods);
        assert!(!app.mod_library.details_open, "no other mod's card opened in its place");
        assert!(told(&app, "qa_private_mod"), "the player is told");
        app.notice = None;
        app.notices.clear();
        // Installed since the launcher looked through the library: found.
        std::fs::create_dir_all(dir.join("mods/late")).unwrap();
        std::fs::write(dir.join("mods/late/mod.toml"), "id = \"late\"\nname = \"Late\"\n").unwrap();
        for id in ["mine", "late", "escalation"] {
            assert!(app.show_wanted_mod(id), "{id}");
            assert_eq!((app.mod_library.selected.as_str(), app.mod_library.details_open), (id, true));
            assert!(!told(&app, id));
        }
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn launcher_qa_close_preserves_settings_after_a_failed_save() {
        let dir = temp_dir("qa_close_save_failure");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, b"not a directory").unwrap();
        app.store.path = blocked.join("config.toml");
        app.config.audio.master = 0.17;
        let mut input = RawInput::default();
        input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().events.push(egui::ViewportEvent::Close);
        let output = frame(&ctx, &mut app, input);
        assert!(output.viewport_output[&egui::ViewportId::ROOT].commands.contains(&egui::ViewportCommand::CancelClose), "a failed exit save must leave the edits available to retry");
        assert!(app.confirm_close);
        assert!(matches!(app.store.status, SaveStatus::Failed(_)));
        assert_eq!(app.config.audio.master, 0.17);
        app.store.path = dir.join("config.toml");
        app.store.flush(&mut app.config).unwrap();
        app.close_dialog(&ctx);
        assert!(!app.confirm_close, "a successful retry removes the reason to keep the window open");
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn launcher_qa_manual_update_check_works_when_startup_checks_are_off() {
        let dir = temp_dir("qa_manual_update");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        let source = dir.join("release.toml");
        std::fs::write(&source, "version = '99.0.0'\nurl = 'https://example.invalid/release'\n").unwrap();
        app.config.updates.check = false;
        app.config.updates.url = url::Url::from_file_path(&source).unwrap().to_string();
        app.push(Action::CheckUpdates);
        frame(&ctx, &mut app, RawInput::default());
        let until = Instant::now() + Duration::from_secs(5);
        while matches!(app.update, UpdateState::Checking(_)) && Instant::now() < until {
            app.update.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(app.update, UpdateState::Available(_)), "an explicit Check now must work independently of automatic startup checks");
        assert!(!app.config.updates.check, "a manual check must not enable future automatic checks");
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn launcher_qa_revisiting_profile_refreshes_campaigns_for_the_enabled_mods() {
        use ta_config::profiles::{Allegiance, ProfileStore};
        let dir = temp_dir("qa_profile_campaigns");
        let game = dir.join("game");
        std::fs::create_dir_all(&game).unwrap();
        for name in ta_vfs::install::REQUIRED_FILES { crate::maps::tests::write_map_archive(&game.join(name)); }
        let profiles = ProfileStore::open(&dir).unwrap();
        let profile = profiles.create("Commander", Allegiance::Arm, None, &Config { game_dir: Some(game), ..Default::default() }).unwrap();
        profiles.select(&profile.id).unwrap();
        let campaign = dir.join("mods/custom/camps");
        std::fs::create_dir_all(&campaign).unwrap();
        std::fs::write(campaign.join("qa campaign.tdf"), b"[HEADER]{campaignside=ARM;}[MISSION0]{missionfile=QA.ota;missionname=QA Mission;}").unwrap();
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        app.skins.shutdown();
        let load = |app: &mut LauncherApp| {
            let until = Instant::now() + Duration::from_secs(5);
            while app.profiles.details.is_none() && Instant::now() < until {
                frame(&ctx, app, RawInput::default());
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(app.profiles.details.is_some(), "profile details must finish loading");
        };
        app.open_page(Page::Profile);
        load(&mut app);
        assert!(!app.profiles.details.as_ref().unwrap().record.campaigns.iter().any(|c| c.id == "qa campaign"));
        app.open_page(Page::Mods);
        app.enable_mod("custom");
        app.open_page(Page::Profile);
        load(&mut app);
        let campaigns = &app.profiles.details.as_ref().unwrap().record.campaigns;
        assert!(campaigns.iter().any(|c| c.id == "qa campaign" && c.missions == ["QA Mission"]), "the service record must use the current load order");
        app.skins.shutdown();
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn launcher_review_maps_refresh_changed_manifests_at_the_same_path() {
        for explicit_rescan in [false, true] {
            let dir = temp_dir(if explicit_rescan { "review_map_key" } else { "review_map_visit" });
            let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--page", "maps", "--pretend-missing"]);
            app.skins.shutdown();
            let source = app.store.dir().join("mods/manifest-change");
            std::fs::create_dir_all(&source).unwrap();
            let manifest = source.join("mod.toml");
            std::fs::write(&manifest, "id = 'manifest-change'\nname = 'Test'\nversion = '1'").unwrap();
            app.discover_mods();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                app.maps_tick(&ctx);
                if app.maps.loaded_for.is_some() && !app.maps.busy() { break; }
                assert!(Instant::now() < deadline, "initial map scan must finish");
                std::thread::sleep(Duration::from_millis(5));
            }
            let old_key = app.maps.loaded_for.clone();
            std::fs::write(&manifest, "id = 'manifest-change'\nname = 'Test'\nversion = '2'\nload_after = ['base']\n[rename]\ncustom = 'maps'").unwrap();
            if explicit_rescan { app.discover_mods(); } else { app.visit_maps(); }
            assert_eq!(app.mods.get("manifest-change").unwrap().info.manifest.version, "2");
            app.maps_tick(&ctx);
            assert_ne!(app.maps.loaded_for, old_key, "manifest changes must invalidate the map source cache");
            drop(app);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    /// Both headers' logos are the website's link: a pointing hand and a tooltip over them, a
    /// click (or Enter, with the keys on them) asks for ta-reborn.com to open, nothing else.
    #[test]
    fn both_header_logos_open_the_website_and_nothing_else() {
        let dir = temp_dir("project_logo");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        let opened = |app: &LauncherApp| app.actions.iter().filter(|action| matches!(action, Action::OpenUrl(url) if url == WEBSITE_URL)).count();
        for compact in [false, true] {
            let draw = |ui: &mut Ui, app: &mut LauncherApp| {
                if compact { app.compact_header(ui); } else { app.header(ui); }
            };
            let at = if compact { pos2(55., 15.) } else { pos2(46., 36.) };
            app.page = Page::About;
            app.actions.clear();
            let raw = || RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1120., 850.))), ..Default::default() };
            ctx.run_ui(raw(), |ui| draw(ui, &mut app)).drop_without_applying_deltas();
            let mut hover = raw();
            hover.events = vec![egui::Event::PointerMoved(at)];
            let output = ctx.run_ui(hover, |ui| draw(ui, &mut app));
            assert_eq!(output.platform_output.cursor_icon, egui::CursorIcon::PointingHand, "the logo shows it is a link");
            output.drop_without_applying_deltas();
            for pressed in [true, false] {
                let mut input = raw();
                input.events = vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }];
                ctx.run_ui(input, |ui| draw(ui, &mut app)).drop_without_applying_deltas();
            }
            assert_eq!(opened(&app), 1, "the logo opens the website");
            assert_eq!(app.actions.len(), 1, "and launches nothing: {:?}", app.actions);
            assert_eq!(app.page, Page::About, "the page stays as it was");
        }
        // With the keys: Tab comes to the full header's logo first, Enter opens the website.
        app.actions.clear();
        let raw = |events: Vec<egui::Event>| RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1120., 850.))), events, ..Default::default() };
        let press = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        ctx.run_ui(raw(vec![egui::Event::PointerGone]), |ui| app.header(ui)).drop_without_applying_deltas();
        ctx.run_ui(raw(vec![press(egui::Key::Tab)]), |ui| app.header(ui)).drop_without_applying_deltas();
        ctx.run_ui(raw(Vec::new()), |ui| app.header(ui)).drop_without_applying_deltas();
        ctx.run_ui(raw(vec![press(egui::Key::Enter)]), |ui| app.header(ui)).drop_without_applying_deltas();
        assert_eq!(opened(&app), 1, "Tab reaches the logo and Enter opens the website: {:?}", app.actions);
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn first_launch_creation_selects_profile_and_later_profiles_start_fresh() {
        use ta_config::profiles::Allegiance;
        let root = temp_dir("profile_creation");
        let (ctx, mut app) = launcher(&root.join("config.toml"), &[]);
        app.skins.shutdown();
        app.config.audio.music = 0.25;
        app.profiles.edit(false);
        app.save_profile(ta_ui::profiles::ProfileDraft { name: "ARM Commander".into(), motto: String::new(), allegiance: Allegiance::Arm, avatar_png: None, clear_avatar: true });
        let first = app.profiles.active.as_ref().unwrap().id.clone();
        assert!(!app.profiles.dialog_open());
        assert_eq!(app.config.gameplay.player_name, "ARM Commander");
        assert_eq!(app.store.path, root.join("profiles").join(&first).join("config.toml"));
        std::fs::write(app.profiles.data_dir(root.clone()).join("progress.toml"), "marker = true").unwrap();
        app.skins.shutdown();
        app.profiles.edit(false);
        app.save_profile(ta_ui::profiles::ProfileDraft { name: "CORE Commander".into(), motto: "For the Core".into(), allegiance: Allegiance::Core, avatar_png: None, clear_avatar: true });
        let second = app.profiles.active.as_ref().unwrap();
        assert_ne!(second.id, first);
        assert_eq!(second.allegiance, Allegiance::Core);
        assert_eq!(second.motto, "For the Core", "a new profile keeps its motto");
        assert!(!app.profiles.data_dir(root.clone()).join("progress.toml").exists());
        assert_eq!(app.config.audio.music, 0.25, "new profiles inherit preferences, not progress");
        app.skins.shutdown();
        drop((ctx, app));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn switching_profiles_keeps_settings_and_shared_content_separate() {
        use ta_config::profiles::{Allegiance, ProfileStore};
        let root = temp_dir("profile_switch");
        let profiles = ProfileStore::open(&root).unwrap();
        let mut seed = Config::default();
        seed.audio.music = 0.2;
        let first = profiles.create("First", Allegiance::Arm, None, &seed).unwrap();
        seed.audio.music = 0.8;
        let second = profiles.create("Second", Allegiance::Core, None, &seed).unwrap();
        profiles.select(&first.id).unwrap();
        let (ctx, mut app) = launcher(&root.join("config.toml"), &[]);
        app.skins.shutdown();
        assert_eq!(app.config.audio.music, 0.2);
        app.config.audio.music = 0.3;
        assert!(app.switch_profile(&second.id));
        assert_eq!(app.config.audio.music, 0.8);
        assert_eq!(app.config.gameplay.player_name, "Second");
        assert_eq!(app.store.dir(), root);
        assert_eq!(app.profiles.data_dir(root.clone()), profiles.data_dir(&second.id).unwrap());
        assert_eq!(Config::try_load_from(&profiles.config_path(&first.id).unwrap()).unwrap().audio.music, 0.3);
        app.skins.shutdown();
        assert!(app.switch_profile(&first.id));
        assert_eq!(app.config.audio.music, 0.3);
        assert_eq!(app.profiles.active.as_ref().unwrap().allegiance, Allegiance::Arm);
        let before = app.store.path.clone();
        assert!(!app.switch_profile("../bad-id"));
        assert_eq!(app.store.path, before);
        app.skins.shutdown();
        drop((ctx, app));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn text_shapes(shape: &egui::Shape, texts: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Text(text) => texts.push((text.galley.text().to_string(), text.galley.rect.translate(text.pos.to_vec2()))),
            egui::Shape::Vec(shapes) => for shape in shapes { text_shapes(shape, texts); },
            _ => {}
        }
    }

    #[test]
    fn footer_places_copyright_at_right_without_showing_configuration_paths() {
        let dir = temp_dir("footer_layout");
        let (ctx, mut app) = launcher(&dir.join("private-player-config.toml"), &[]);
        for width in [470.0, 700.0, 1120.0] {
            let mut texts = Vec::new();
            for _ in 0..3 {
                let mut output = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 700.0))), ..Default::default() }, |ui| app.footer(ui, width < 900.0));
                output.textures_delta.clear();
                texts.clear();
                for shape in output.shapes { text_shapes(&shape.shape, &mut texts); }
            }
            let (_, copyright) = texts.iter().find(|(text, _)| text == "Total Annihilation is © its respective owners.").expect("copyright stays visible");
            assert!(copyright.max.x <= width - 8.0 && copyright.max.x >= width - 45.0, "copyright aligned to right at {width}: {copyright:?}");
            assert!(copyright.max.y <= 700.0, "copyright clipped: {copyright:?}");
            assert!(!texts.iter().any(|(text, _)| text.contains("private-player-config") || text == "CONFIG"));
        }
        app.skins.shutdown();
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn unified_play_control_keeps_readiness_inside_its_target() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        for width in [168.0, 214.0, 270.0] {
            for state in [widgets::PlayState::Ready, widgets::PlayState::Running, widgets::PlayState::Unavailable] {
                let mut button = Rect::NOTHING;
                let mut output = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 180.0))), ..Default::default() }, |ui| {
                    button = widgets::play_button_width(ui, state, width).rect;
                });
                output.textures_delta.clear();
                let mut texts = Vec::new();
                for shape in output.shapes { text_shapes(&shape.shape, &mut texts); }
                let expected = match state { widgets::PlayState::Ready => "GAME DATA VERIFIED", widgets::PlayState::Running => "GAME RUNNING", _ => "ORIGINAL GAME NOT FOUND" };
                let (_, bounds) = texts.iter().find(|(text, _)| text == expected).expect("readiness inside the button");
                assert!(button.contains_rect(*bounds), "{width}: readiness {bounds:?} outside button {button:?}");
            }
        }
    }

    #[test]
    fn enlarged_launcher_uses_compact_navigation_and_keeps_settings_reachable() {
        let dir = temp_dir("accessibility_scale");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        app.config.gameplay.ui_scale = 2.0;
        app.config.gameplay.reduced_motion = true;
        app.page = Page::Settings;
        app.settings_view.open(ta_ui::settings::Category::Accessibility);
        let mut frame = eframe::Frame::_new_kittest();
        for _ in 0..4 {
            let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(470.0, 300.0))), ..Default::default() };
            ctx.run_ui(raw, |ui| app.ui(ui, &mut frame)).drop_without_applying_deltas();
        }
        assert_eq!(ctx.zoom_factor(), 2.0);
        assert!(compact_layout(vec2(470.0, 300.0)));
        assert!(!compact_layout(vec2(1120.0, 720.0)));
        assert_eq!(app.settings_view.category(), ta_ui::settings::Category::Accessibility);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn enlarged_library_pages_can_scroll_past_the_toolbar_to_reach_their_cards() {
        let dir = temp_dir("library_accessibility_scale");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        app.config.gameplay.ui_scale = 2.0;
        app.config.gameplay.reduced_motion = true;
        ctx.set_zoom_factor(2.0);
        // The same page offset is available to ordinary mouse/touch scrolling.
        app.capture_scroll = Some(220.0);
        for (page, card) in [(Page::Mods, ("mod_card", "escalation")), (Page::Skins, ("skin_card", "reborn"))] {
            app.page = page;
            for _ in 0..5 {
                frame(&ctx, &mut app, RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(470.0, 300.0))), ..Default::default()
                });
            }
            let response = ctx.read_response(egui::Id::new(card)).expect("a card below the toolbar is rendered after scrolling");
            assert!(response.interact_rect.height() > 10.0 && response.interact_rect.width() > 10.0,
                "{page:?}: a card is visible and clickable inside the clipped viewport: {response:?}");
        }
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Runs every page for a few frames; a panic anywhere in the UI code fails the test.
    fn render_all_pages(ctx: &egui::Context, app: &mut LauncherApp) {
        let mut frame = eframe::Frame::_new_kittest();
        for (page, tab) in [(Page::Play, ModsTab::Library), (Page::Mods, ModsTab::Library), (Page::Mods, ModsTab::LoadOrder), (Page::Maps, ModsTab::Library), (Page::Skins, ModsTab::Library), (Page::Profile, ModsTab::Library), (Page::Settings, ModsTab::Library), (Page::Benchmark, ModsTab::Library), (Page::About, ModsTab::Library)] {
            app.page = page;
            app.mods_tab = tab;
            for _ in 0..3 {
                let input = RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1120.0, 720.0))),
                    ..Default::default()
                };
                ctx.run_ui(input, |ui| app.ui(ui, &mut frame)).drop_without_applying_deltas();
            }
        }
    }

    /// A launcher left alone in front settles (its idle animations still: no more frames asked
    /// for them) after a minute, and wakes at the next touch of the mouse; its ambient clock
    /// holds meanwhile, so nothing jumps when it wakes.
    #[test]
    fn a_launcher_left_alone_settles_and_wakes_at_a_touch() {
        let dir = temp_dir("settle");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        let mut frame = eframe::Frame::_new_kittest();
        let mut run = |app: &mut LauncherApp, time: f64, events: Vec<egui::Event>| {
            let raw = RawInput { time: Some(time), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1120.0, 720.0))), events, ..Default::default() };
            ctx.run_ui(raw, |ui| app.ui(ui, &mut frame)).drop_without_applying_deltas();
        };
        let moved = || vec![egui::Event::PointerMoved(pos2(600.0, 400.0))];
        run(&mut app, 0.0, moved());
        for k in 1..=20 {
            run(&mut app, k as f64 * 0.05, Vec::new());
        }
        assert!(!ta_ui::motion::still(&ctx), "moving a second after a touch");
        let ambient = app.ambient;
        assert!(ambient > 0.9, "its clock runs: {ambient}");
        run(&mut app, 30.0, Vec::new());
        assert!(!ta_ui::motion::still(&ctx), "still moving half a minute on");
        run(&mut app, SETTLE + 5.0, Vec::new());
        assert!(ta_ui::motion::still(&ctx), "settled after a minute alone");
        let held = app.ambient;
        run(&mut app, SETTLE + 9.0, Vec::new());
        assert_eq!(app.ambient, held, "its clock holds while settled");
        run(&mut app, SETTLE + 10.0, moved());
        assert!(!ta_ui::motion::still(&ctx), "awake at a touch");
        run(&mut app, SETTLE + 10.05, Vec::new());
        assert!(app.ambient > held && app.ambient - held < 0.2, "on from where it held: {held} -> {}", app.ambient);
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_window_taller_than_the_screen_shrinks_to_fit() {
        let dir = temp_dir("fit");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        let mut frame = eframe::Frame::_new_kittest();
        // A 1080p laptop at 150%: 1280 x 720 points, the window's inside as tall as the screen.
        let mut input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1120.0, 720.0))), ..Default::default() };
        let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
        info.monitor_size = Some(vec2(1280.0, 720.0));
        info.inner_rect = Some(Rect::from_min_size(egui::pos2(80.0, 31.0), vec2(1120.0, 720.0)));
        info.outer_rect = Some(Rect::from_min_size(egui::pos2(72.0, 0.0), vec2(1136.0, 759.0)));
        let mut run = |input: RawInput| {
            let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
            output.textures_delta.clear();
            output.viewport_output[&egui::ViewportId::ROOT].commands.clone()
        };
        let commands = run(input);
        let size = commands.iter().find_map(|c| if let egui::ViewportCommand::InnerSize(s) = c { Some(*s) } else { None }).expect("resized");
        assert!(size.y + 39.0 + 56.0 <= 720.0 && size.y >= 600.0, "{size:?}");
        assert!(!commands.iter().any(|c| matches!(c, egui::ViewportCommand::OuterPosition(_))), "fitting must keep the window on its current monitor");
        // Once only.
        assert!(!run(RawInput::default()).iter().any(|c| matches!(c, egui::ViewportCommand::InnerSize(_))));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fitting_an_enlarged_interface_keeps_native_window_bounds() {
        let dir = temp_dir("fit_scaled");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        app.config.gameplay.ui_scale = 2.0;
        ctx.set_zoom_factor(2.0);
        frame(&ctx, &mut app, RawInput::default());
        let mut input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(560.0, 360.0))), ..Default::default() };
        let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
        info.monitor_size = Some(vec2(640.0, 360.0));
        info.inner_rect = Some(Rect::from_min_size(pos2(700.0, 15.5), vec2(560.0, 360.0)));
        info.outer_rect = Some(Rect::from_min_size(pos2(696.0, 0.0), vec2(568.0, 379.5)));
        let output = frame(&ctx, &mut app, input);
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
        let size = commands.iter().find_map(|c| if let egui::ViewportCommand::InnerSize(size) = c { Some(*size) } else { None }).expect("resized");
        assert!(size.x <= 560.0 && size.y <= 308.5, "must fit at 200%: {size:?}");
        assert!(!commands.iter().any(|c| matches!(c, egui::ViewportCommand::OuterPosition(_))));
        app.skins.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn closing_retains_a_skin_selection_that_finishes_after_the_last_frame() {
        let dir = temp_dir("close_skin_selection");
        let path = dir.join("config.toml");
        let (_, mut app) = launcher(&path, &[]);
        app.skins.shutdown();
        app.skins.start(dir.join("skins"), crate::skins::Operation::SelectEnemy("builtin:navy".into()));
        app.on_exit(None);
        assert_eq!(Config::try_load_from(&path).unwrap().graphics.enemy_unit_skin, "builtin:navy");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn closing_applies_completed_mod_installs_before_saving_the_enabled_list() {
        use crate::getmods::Stage;
        for updated in [false, true] {
            let dir = temp_dir(if updated { "close_mod_update" } else { "close_mod_install" });
            let path = dir.join("config.toml");
            let (_, mut app) = launcher(&path, &["--pretend-missing"]);
            app.skins.shutdown();
            let entry = crate::catalogue::get("totalmayhem").unwrap();
            app.config.mods.enabled = vec!["unrelated-addon".into()];
            let folder = dir.join("mods").join(entry.id);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(folder.join("mod.toml"), format!("id = {:?}\nname = {:?}\n", entry.id, entry.name)).unwrap();
            if updated { app.updates_requested.insert(entry.id.into()); }
            // The install finished after the last UI frame; its completion has not yet
            // reached installs(), just as when shutdown waits for final publication.
            app.installer.stages.insert(entry.id.into(), Stage::Done { units: 3, maps: 0, id: entry.id.into() });
            app.installer.finished.push(entry.id.into());
            app.on_exit(None);
            let config = Config::try_load_from(&path).unwrap();
            assert_eq!(config.mods.enabled.contains(&entry.id.to_string()), !updated, "a first install turns on; an update preserves off");
            assert!(config.mods.enabled.contains(&"unrelated-addon".to_string()));
            drop(app);
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn offline_startup_keeps_other_updates_and_does_not_start_requested_downloads() {
        let dir = temp_dir("offline_startup");
        let cache = dir.join("cache/updates");
        std::fs::create_dir_all(&cache).unwrap();
        let pending = cache.join("another-launcher.part");
        std::fs::write(&pending, "an active download").unwrap();
        let (_, mut app) = launcher(&dir.join("config.toml"), &["--get-mod", "totalmayhem"]);
        assert!(!app.installer.busy());
        assert!(pending.is_file(), "startup cannot delete another process's files");
        app.skins.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A report as a run of the benchmark writes one: the parts in turn on its timeline (the
    /// scenes' passes, the effects test), the settings test's levels, the reference pass; or as
    /// the first benchmark wrote one (`first`: three battles).
    fn sample_report(when: u64, first: bool) -> ta_config::benchmark::Report {
        use ta_config::benchmark::{CONTENT, Effect, EffectRun, FIRST_PARTS, FULL_PARTS, LEVELS, LevelRun, Memory, Part, PassTime, Report, Sample, Stats, System};
        let stats = |fps: f32, units: u32| Stats { frames: 4000, fps, low1: fps * 0.7, low01: fps * 0.6, frame_ms: 1000.0 / fps, frame_p99: 1400.0 / fps, frame_p50: 1000.0 / fps, frame_p90: 1200.0 / fps, hitches: (fps < 80.0) as u32 * 3, cpu_ms: 4.0, gpu_ms: 900.0 / fps, tick_ms: units as f32 / 900.0, ticks_per_second: 30.0, units };
        let ids: &[&str] = if first { &FIRST_PARTS } else { &FULL_PARTS };
        let passes = |fps: f32| ["shadow", "scene", "reflection", "water and effects", "bloom"].iter().zip([0.1, 0.55, 0.12, 0.08, 0.02]).map(|(pass, share)| PassTime { pass: pass.to_string(), ms: 900.0 / fps * share }).collect();
        let parts: Vec<Part> = ids.iter().enumerate().map(|(i, id)| {
            let fps = 60.0 + i as f32 * 10.0;
            Part { id: id.to_string(), start: i as f32 * 30.0, seconds: 30.0, stats: stats(fps, match *id { "cinder" => 3544, "swarm" => 20_000, _ => 213 }), passes: if first { Vec::new() } else { passes(fps) }, picture: None }
        }).collect();
        let timeline = (0..640).map(|k| Sample { t: k as f32 * 0.25, part: (k * ids.len() / 640) as u8, fps: 80.0 + (k % 37) as f32, low: 60.0, cpu_ms: 4.0, gpu_ms: 9.0, tick_ms: 0.5, units: 213 }).collect();
        let effects = if first { Vec::new() } else { Effect::ALL.iter().zip([10.0, 8.6, 9.2, 8.1, 10.3, 7.1, 9.1, 10.2, 10.3, 7.9]).map(|(&effect, gpu_ms)| EffectRun { effect, stats: Stats { gpu_ms, frame_ms: gpu_ms + 1.0, ..stats(90.0, 199) } }).collect() };
        Report {
            format: if first { 1 } else { 2 },
            content: if first { 0 } else { CONTENT },
            version: "2.2.0".into(),
            when,
            effects,
            histogram: if first { Vec::new() } else { vec![900, 1400, 800, 120, 40, 10, 3] },
            system: System { gpu: "NVIDIA GeForce RTX 4080".into(), backend: "Vulkan".into(), driver: "NVIDIA 617.42".into(), cpu: "AMD Ryzen 9 5950X 16-Core Processor".into(), threads: 32, ram_mb: 65_461, vram_mb: 16_048 },
            width: 3840,
            height: 2160,
            refresh_hz: 120.0,
            parts,
            timeline,
            levels: LEVELS.iter().zip([200.0, 90.0, 72.0, 66.0]).map(|(&level, fps)| LevelRun { level, stats: stats(fps, 199) }).collect(),
            reference: stats(146.0, 199),
            reference_scale: 0.5,
            tested_msaa: 8,
            memory: Memory { peak_mb: 1_575, bandwidth_gbs: 40.6, latency_ns: 100.0 },
            gpu_timed: true,
            ..Default::default()
        }
    }

    /// The Benchmark page with reports (one shown, the others earlier), while a run is under
    /// way, and after one that left none; its recommendation applied.
    #[test]
    fn the_benchmark_page_shows_its_reports() {
        let dir = temp_dir("benchmark");
        std::fs::create_dir_all(dir.join("benchmarks")).unwrap();
        for when in [1_790_000_000, 1_791_000_000] {
            std::fs::write(dir.join("benchmarks").join(format!("benchmark-{when}.json")), serde_json::to_string(&sample_report(when, when == 1_790_000_000)).unwrap()).unwrap();
        }
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        assert_eq!(app.benchmarks.runs.len(), 2);
        assert_eq!(app.benchmarks.report().map(|r| r.when), Some(1_791_000_000), "the newest shown");
        app.page = Page::Benchmark;
        for _ in 0..3 {
            frame(&ctx, &mut app, RawInput::default());
        }
        // A pointer over the run's chart: its values.
        frame(&ctx, &mut app, RawInput { events: vec![egui::Event::PointerMoved(egui::pos2(700.0, 900.0))], ..Default::default() });
        app.benchmarks.shown = 1;
        app.benchmarks.stopped = Some("The benchmark was stopped before its end.".into());
        frame(&ctx, &mut app, RawInput::default());
        // A computer that told nothing of its memory or processor (outside Windows, some of it):
        // shown as not known, scored as enough memory.
        if let Some((_, r)) = app.benchmarks.runs.get_mut(1) {
            (r.system.ram_mb, r.system.cpu, r.memory.peak_mb) = (0, String::new(), 0);
            let mut enough = r.clone();
            enough.system.ram_mb = 32_768;
            assert_eq!(r.scores().ram, enough.scores().ram);
        }
        frame(&ctx, &mut app, RawInput::default());
        // The recommendation applied: Ultra's and High's slowest frames (46 and 50 a second)
        // fall short of 60, Medium's (63) hold: Medium; the unit limit no higher than it was.
        let rec = app.benchmarks.report().unwrap().recommend();
        assert_eq!(rec.level, ta_config::Preset::Medium);
        let limit = app.config.gameplay.unit_limit;
        app.push(Action::ApplyRecommendation);
        frame(&ctx, &mut app, RawInput::default());
        assert!(rec.applied(&app.config.graphics, app.config.gameplay.unit_limit));
        assert_eq!(app.config.graphics.preset, ta_config::Preset::Medium);
        assert_eq!(app.config.gameplay.unit_limit, limit.min(rec.unit_limit));
        // No reports: what the benchmark does.
        let empty = temp_dir("benchmark_none");
        let (ctx, mut app) = launcher(&empty.join("config.toml"), &["--pretend-missing"]);
        app.page = Page::Benchmark;
        frame(&ctx, &mut app, RawInput::default());
        assert!(app.benchmarks.report().is_none());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(empty);
    }

    #[test]
    fn pages_render_without_an_install() {
        let dir = temp_dir("missing");
        let path = dir.join("config.toml");
        let show = |app: &mut LauncherApp| {
            app.settings_view.open(ta_ui::settings::Category::Display);
            app.confirm_reset = true;
        };
        // The first start sets up for this computer (written once that settles, however long
        // the pages take to draw): showing them changes nothing more.
        let (ctx, mut app) = launcher(&path, &["--pretend-missing"]);
        assert!(app.detection.install().is_none());
        show(&mut app);
        let set_up = app.config.clone();
        render_all_pages(&ctx, &mut app);
        assert!(app.config == set_up, "merely showing the pages must not change settings");
        app.store.save_now(&mut app.config);
        // A later start has nothing to set up: past the save delay, nothing has been written.
        let written = std::fs::read(&path).unwrap();
        let (ctx, mut app) = launcher(&path, &["--pretend-missing"]);
        show(&mut app);
        render_all_pages(&ctx, &mut app);
        std::thread::sleep(crate::persist::SAVE_DELAY);
        render_all_pages(&ctx, &mut app);
        assert_eq!(app.store.status, SaveStatus::Unchanged);
        assert!(std::fs::read(&path).unwrap() == written, "merely showing the pages must not write settings");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pages_render_with_invalid_folder_mod_problems_and_launch_errors() {
        let dir = temp_dir("problems");
        let empty_game = dir.join("empty_game");
        std::fs::create_dir_all(&empty_game).unwrap();
        let mods = dir.join("mods");
        for (name, manifest) in [
            ("needs_base", "id = \"needs_base\"\nname = \"Needs Base\"\ndepends = [\"base\"]\n"),
            ("base", "id = \"base\"\nname = \"Base\"\n"),
            ("broken", "id = \n"),
        ] {
            std::fs::create_dir_all(mods.join(name)).unwrap();
            std::fs::write(mods.join(name).join("mod.toml"), manifest).unwrap();
        }
        std::fs::write(
            dir.join("config.toml"),
            format!("game_dir = '{}'\n[mods]\nenabled = [\"needs_base\", \"gone\"]\n", empty_game.display()),
        )
        .unwrap();
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        assert!(matches!(app.detection.result, Err(InstallError::Invalid { .. })));
        assert_eq!(app.mods.problems.len(), 1, "{:?}", app.mods.problems);
        assert!(matches!(app.mods.resolution, modlist::Resolution::Err(_)));
        app.browse_error = Some((dir.clone(), vec!["totala1.hpi".into()]));
        app.notify("Test notice", GREEN);
        render_all_pages(&ctx, &mut app);

        // Launch feedback cards are shown on the Play page of a valid install.
        let fake_game = dir.join("fake_game");
        std::fs::create_dir_all(&fake_game).unwrap();
        for file in ta_vfs::install::REQUIRED_FILES {
            std::fs::write(fake_game.join(file), b"").unwrap();
        }
        app.detection = detect::detect(Some(&fake_game), true);
        assert!(app.detection.install().is_some());
        app.game = GameState::Failed("ta-reborn.exe was not found".into());
        render_all_pages(&ctx, &mut app);
        app.game = GameState::Exited { code: Some(3), success: false, ran_for: Duration::from_secs(2), log: None, crash: None };
        render_all_pages(&ctx, &mut app);
        // A crash, with its report: the card says so and offers the report.
        app.game = GameState::Exited {
            code: Some(101),
            success: false,
            ran_for: Duration::from_secs(600),
            log: Some(dir.join("game.log")),
            crash: Some(launch::Crash { report: dir.join("crash.txt"), message: "panicked at src/app.rs:1:1: boom".into() }),
        };
        render_all_pages(&ctx, &mut app);

        // An unreadable settings file is reported on the Settings page and in the status bar.
        std::fs::write(dir.join("config.toml"), "vsync = = maybe").unwrap();
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        assert!(app.store.load_error.is_some());
        render_all_pages(&ctx, &mut app);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// One frame with `input` (the screen's size filled in).
    fn frame(ctx: &egui::Context, app: &mut LauncherApp, mut input: RawInput) -> egui::FullOutput {
        let mut frame = eframe::Frame::_new_kittest();
        input.screen_rect.get_or_insert(Rect::from_min_size(Pos2::ZERO, vec2(1120.0, 720.0)));
        let mut output = ctx.run_ui(input, |ui| {
            app.logic(ui.ctx(), &mut frame);
            app.ui(ui, &mut frame);
        });
        output.textures_delta.clear();
        output
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> RawInput {
        RawInput { events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }], ..Default::default() }
    }

    /// A file dropped on the window, as the backend hands it over.
    #[derive(Debug)]
    struct Dropped(PathBuf);

    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|e| e.to_string())
        }
    }

    fn dropped(path: &Path) -> RawInput {
        RawInput { dropped_files: vec![std::sync::Arc::new(Dropped(path.to_path_buf()))], ..Default::default() }
    }

    /// Ctrl+2 goes to the mods, Ctrl+4 to About; F5 on the Play page looks for the game again;
    /// nothing is taken while a text field has the keyboard.
    #[test]
    fn shortcut_help_blocks_navigation_and_page_search_until_dismissed() {
        let dir = temp_dir("shortcut_help");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        frame(&ctx, &mut app, RawInput::default());
        frame(&ctx, &mut app, key(egui::Key::F1, egui::Modifiers::NONE));
        assert!(app.show_shortcuts);
        frame(&ctx, &mut app, key(egui::Key::Num2, egui::Modifiers::COMMAND));
        assert_eq!(app.page, Page::Play);
        frame(&ctx, &mut app, key(egui::Key::Escape, egui::Modifiers::NONE));
        assert!(!app.show_shortcuts);
        frame(&ctx, &mut app, RawInput::default());
        frame(&ctx, &mut app, key(egui::Key::Num2, egui::Modifiers::COMMAND));
        assert_eq!(app.page, Page::Mods);
        frame(&ctx, &mut app, key(egui::Key::F, egui::Modifiers::COMMAND));
        assert!(ctx.egui_wants_keyboard_input());
        frame(&ctx, &mut app, key(egui::Key::F1, egui::Modifiers::NONE));
        assert!(app.show_shortcuts, "F1 remains available while searching");
        app.mod_library.query = "keep this search".into();
        frame(&ctx, &mut app, key(egui::Key::F, egui::Modifiers::COMMAND));
        frame(&ctx, &mut app, key(egui::Key::Escape, egui::Modifiers::NONE));
        assert!(!app.show_shortcuts);
        assert_eq!(app.mod_library.query, "keep this search");
        app.skins.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn help_does_not_focus_settings_search_and_missing_game_keeps_update_controls() {
        let dir = temp_dir("help_settings_update");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.page = Page::Settings;
        app.show_shortcuts = true;
        frame(&ctx, &mut app, key(egui::Key::F, egui::Modifiers::COMMAND));
        assert!(!ctx.egui_wants_keyboard_input(), "the covered settings search must not acquire focus");
        frame(&ctx, &mut app, key(egui::Key::Escape, egui::Modifiers::NONE));
        assert!(!app.show_shortcuts);
        app.show("update");
        app.push(Action::ShowUpdate);
        frame(&ctx, &mut app, RawInput::default());
        frame(&ctx, &mut app, RawInput::default());
        assert_eq!(app.page, Page::Play);
        assert!(!app.focus_update, "the update card must be rendered and consume the reveal request even without game data");
        app.skins.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A pad works the launcher (its state as the pads' thread feeds it, the frames as the input
    /// hook runs them): the bumpers turn the pages, not under a dialog; the d-pad moves the
    /// focus down a long page, each control it reaches scrolled into view; B goes back.
    #[test]
    fn a_pad_turns_the_pages_and_keeps_the_focus_in_view() {
        use crate::pad::Reading;
        use gilrs::Button;
        let dir = temp_dir("pad");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1120.0, 720.0));
        let mut pad = crate::pad::State::default();
        let mut time = 0.0;
        let mut step = |app: &mut LauncherApp, readings: &[Reading]| {
            time += 0.05;
            let focused = ctx.memory(|m| m.focused());
            let f = pad.frame(readings, time, None, None, screen.size(), focused, ctx.text_edit_focused(), 0.18, 1.0);
            app.pad_pages += f.pages;
            frame(&ctx, app, RawInput { events: f.events, screen_rect: Some(screen), ..Default::default() });
        };
        let press = |button| [Reading::Pressed(button), Reading::Released(button)];
        step(&mut app, &[]);
        for _ in 0..3 {
            step(&mut app, &press(Button::RightTrigger));
        }
        assert_eq!(app.page, Page::Skins);
        step(&mut app, &press(Button::LeftTrigger));
        assert_eq!(app.page, Page::Maps);
        for _ in 0..12 {
            step(&mut app, &press(Button::RightTrigger));
        }
        assert_eq!(app.page, Page::About, "the last page: no further");
        // Under a dialog the bumpers do nothing.
        app.show_shortcuts = true;
        step(&mut app, &press(Button::LeftTrigger));
        assert_eq!(app.page, Page::About);
        step(&mut app, &press(Button::East));
        assert!(!app.show_shortcuts, "B closes the dialog");
        // Into the graphics settings (down the categories, right into their controls), then down
        // them, far past the window's foot: every control the focus reaches is in view.
        app.open_page(Page::Settings);
        app.settings_view.open(ta_ui::settings::Category::Graphics);
        step(&mut app, &[]);
        let mut go = |app: &mut LauncherApp, button| {
            step(app, &press(button));
            for _ in 0..3 {
                step(app, &[]);
            }
            let id = ctx.memory(|m| m.focused())?;
            let rect = ctx.read_response(id).map(|r| r.rect).expect("the focused control is laid out");
            assert!(rect.top() >= -1.0 && rect.bottom() <= screen.bottom() + 1.0, "the control with the focus is in view: {rect:?}");
            Some((id, rect))
        };
        // (From nothing to the header, down the rail, right to the settings' search: a text
        // field, which the d-pad leaves with Tab, to the first category; right into its controls.)
        for _ in 0..3 {
            go(&mut app, Button::DPadDown);
        }
        go(&mut app, Button::DPadRight);
        assert!(ctx.text_edit_focused(), "the settings' search");
        let (_, category) = go(&mut app, Button::DPadDown).expect("the first category");
        assert!(!ctx.text_edit_focused(), "the d-pad left the search");
        go(&mut app, Button::DPadRight).expect("a control has the focus");
        let mut seen = std::collections::HashSet::new();
        for _ in 0..30 {
            if let Some((id, rect)) = go(&mut app, Button::DPadDown) {
                assert!(rect.left() > category.right(), "among the controls: {rect:?}");
                seen.insert(id);
            }
        }
        assert!(seen.len() > 12, "the d-pad went down the controls: {}", seen.len());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keys_go_to_the_pages() {
        let dir = temp_dir("keys");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        frame(&ctx, &mut app, RawInput::default());
        // (Ctrl and the page's place in the rail.)
        for (k, page) in [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6, egui::Key::Num7, egui::Key::Num8].into_iter().zip(PAGES).rev() {
            frame(&ctx, &mut app, key(k, egui::Modifiers::COMMAND));
            assert_eq!(app.page, page, "{k:?}");
        }
        assert_eq!(app.page, Page::Play);
        frame(&ctx, &mut app, key(egui::Key::F5, egui::Modifiers::NONE));
        assert!(app.last_notice().is_some_and(|n| n.color == RED), "rescanned: still not found");
        // (Enter plays only with the game found.)
        frame(&ctx, &mut app, key(egui::Key::Enter, egui::Modifiers::NONE));
        assert!(matches!(app.game, GameState::Idle));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_map_library_shows_cards_and_rows_and_the_keys_and_clicks_choose_and_open_maps() {
        let dir = temp_dir("map_library_page");
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        crate::maps::tests::write_map_archive(&dir.join("mods/new-maps.ufo"));
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        app.page = Page::Maps;
        app.visit_maps();
        // (A narrow page: the details take the library's place when a map is opened.)
        let narrow = || RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(720.0, 640.0))), ..Default::default() };
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline && !(app.maps.ready() && !app.maps.selected.is_empty()) {
            frame(&ctx, &mut app, narrow());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.maps.ready() && !app.maps.selected.is_empty(), "the maps, their pictures and the chosen one's come in");
        assert!(app.maps.catalogue.entries.iter().any(|entry| entry.layer == "new-maps" && entry.stem == "moon"));
        frame(&ctx, &mut app, narrow());
        let visible = app.maps.visible(&app.config.maps);
        let card = |app: &LauncherApp, at: usize| egui::Id::new(("map_card", &app.maps.catalogue.entries[visible[at]].key));
        let first = ctx.read_response(card(&app, 0)).expect("the first card is drawn");
        assert!(first.rect.width() >= 160.0, "cards keep their least width: {:?}", first.rect);
        // The keys move from card to card, choosing each.
        ctx.memory_mut(|memory| memory.request_focus(card(&app, 0)));
        frame(&ctx, &mut app, narrow());
        frame(&ctx, &mut app, RawInput { screen_rect: narrow().screen_rect, ..key(egui::Key::ArrowRight, egui::Modifiers::NONE) });
        frame(&ctx, &mut app, narrow());
        assert_eq!(app.maps.selected, app.maps.catalogue.entries[visible[1]].key, "the arrow keys choose the next card");
        assert!(!app.maps.details_open, "moving with the keys does not open a map");
        // A click opens the map's details in the library's place; Escape goes back.
        let second = ctx.read_response(card(&app, 1)).unwrap().rect.center();
        for pressed in [true, false] {
            frame(&ctx, &mut app, RawInput { events: vec![egui::Event::PointerMoved(second), egui::Event::PointerButton { pos: second, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }],
                screen_rect: narrow().screen_rect, ..Default::default() });
        }
        assert!(app.maps.details_open);
        ctx.memory_mut(|memory| memory.stop_text_input());
        ctx.memory_mut(|memory| if let Some(id) = memory.focused() { memory.surrender_focus(id) });
        frame(&ctx, &mut app, RawInput { screen_rect: narrow().screen_rect, ..key(egui::Key::Escape, egui::Modifiers::NONE) });
        assert!(!app.maps.details_open, "Escape goes back to the library");
        // The list draws a row for each map.
        app.maps.view = crate::maps::View::List;
        frame(&ctx, &mut app, narrow());
        frame(&ctx, &mut app, narrow());
        let row = ctx.read_response(card(&app, 0)).expect("the first row is drawn");
        assert!((row.rect.height() - 58.0).abs() < 0.5, "{:?}", row.rect);
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The tested release of an installed catalogue mod, dropped on the window (to repair or
    /// re-import it, as a failed download advises), is an update of it: it keeps its on or off
    /// and leaves the other conversions as they are, as an update from the card does.
    #[test]
    fn a_dropped_release_of_an_installed_mod_is_its_update() {
        let dir = temp_dir("dropped_release_update");
        let entry = crate::catalogue::MODS.iter().find(|entry| entry.file.is_some()).unwrap();
        let folder = dir.join("mods").join(entry.id);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("mod.toml"), format!("id = \"{}\"\nname = \"{}\"\nversion = \"0.1\"\n", entry.id, entry.name)).unwrap();
        let (_ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        assert!(app.mods.get(entry.id).is_some());
        let release = dir.join("downloaded release.zip");
        std::fs::File::create(&release).unwrap().set_len(entry.file.unwrap().bytes).unwrap();
        app.install_package(&release);
        assert!(app.updates_requested.contains(entry.id), "an update, as the card's");
        app.installer.cancel(entry.id);
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A package the player brings, named like a mod installed already (their own hand-made
    /// folder, or a catalogue mod's tested release that this file is not), takes its place only
    /// once asked: nothing is installed meanwhile. One of a new id installs at once.
    #[test]
    fn a_dropped_package_over_an_installed_mod_asks_first() {
        let dir = temp_dir("dropped_package_replace");
        let folder = dir.join("mods").join("myunits");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("mod.toml"), "id = \"myunits\"
name = \"My Units\"
version = \"2\"
").unwrap();
        let (_ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        let package = dir.join("MyUnits.zip");
        std::fs::write(&package, b"not opened before it is asked").unwrap();
        app.install_package(&package);
        assert!(app.confirm_replace.as_ref().is_some_and(|source| source.id() == "myunits"), "asked first");
        assert!(!app.installer.stages.contains_key("myunits"), "nothing installed meanwhile");
        app.confirm_replace = None;
        let other = dir.join("Other Units.zip");
        std::fs::write(&other, b"x").unwrap();
        app.install_package(&other);
        assert!(app.confirm_replace.is_none(), "a new mod is not asked about");
        app.installer.cancel("otherunits");
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_update_keeps_a_mod_on_or_off_and_a_first_install_turns_it_on() {
        use crate::getmods::Stage;
        let dir = temp_dir("install_on_off");
        let entry = crate::catalogue::get("totalmayhem").unwrap();
        let folder = dir.join("mods").join(entry.id);
        let install = |folder: &Path| {
            std::fs::create_dir_all(folder).unwrap();
            std::fs::write(folder.join("mod.toml"), format!("id = \"{}\"
name = \"{}\"
", entry.id, entry.name)).unwrap();
        };
        install(&folder);
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        app.config.mods.enabled.clear();
        assert!(app.mods.get(entry.id).is_some());
        let finish = |app: &mut LauncherApp| {
            // (As asked for: an update while installed.)
            if app.mods.get(entry.id).is_some() {
                app.updates_requested.insert(entry.id.to_string());
            }
            app.installer.stages.insert(entry.id.into(), Stage::Done { units: 3, maps: 1, id: entry.id.into() });
            app.installer.finished.push(entry.id.into());
            app.installs(&ctx);
        };
        // Updated while off: still off, and said to be updated.
        finish(&mut app);
        assert!(!app.config.mods.enabled.iter().any(|id| id == entry.id), "an update does not turn a mod on");
        assert!(app.last_notice().is_some_and(|n| n.text.contains("updated")), "{:?}", app.last_notice().map(|n| &n.text));
        // Updated while on: still on.
        modlist::set_enabled(&mut app.config.mods.enabled, entry.id, true);
        finish(&mut app);
        assert!(app.config.mods.enabled.iter().any(|id| id == entry.id));
        // Installed for the first time: turned on.
        std::fs::remove_dir_all(&folder).unwrap();
        app.refresh_mods();
        app.config.mods.enabled.clear();
        assert!(app.mods.get(entry.id).is_none());
        install(&folder);
        finish(&mut app);
        assert!(app.config.mods.enabled.iter().any(|id| id == entry.id), "a first install is on for the next game");
        assert!(app.last_notice().is_some_and(|n| n.text.contains("installed and on")));
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn end_game_asks_first_and_a_game_ended_so_is_not_a_crash_to_report() {
        let dir = temp_dir("end_game");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        // (Its settings saved, as a game started from here has them.)
        app.store.flush(&mut app.config).unwrap();
        // (A stand-in for a game that stopped responding.)
        let child = std::process::Command::new("ping").args(["-n", "60", "127.0.0.1"])
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
        app.game = GameState::Running { child, since: Instant::now(), started: std::time::SystemTime::now(), log: None };
        app.push(Action::EndGame);
        frame(&ctx, &mut app, RawInput::default());
        assert!(app.confirm_end_game, "asked first");
        assert!(app.game.is_running(), "and nothing ended yet");
        // END GAME answered: the game goes, and the Play page has no crash to show.
        app.confirm_end_game = false;
        app.ending_game = true;
        app.game.end();
        let until = Instant::now() + Duration::from_secs(10);
        while app.game.is_running() && Instant::now() < until {
            frame(&ctx, &mut app, RawInput::default());
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(matches!(app.game, GameState::Idle), "ended, not crashed");
        assert!(!app.game.went_wrong());
        assert!(app.last_notice().is_some_and(|n| n.text.contains("ended")), "{:?}", app.last_notice().map(|n| &n.text));
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn notices_wait_their_turn_and_an_error_stays_until_dismissed() {
        let dir = temp_dir("notices");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        // (Words of no language: never sent for translation.)
        let (first, second, trouble) = (String::from("First"), String::from("Second"), String::from("Trouble"));
        app.notify(first.clone(), GREEN);
        app.notify(first, GREEN);
        app.notify(second, AMBER);
        app.notify(trouble.clone(), RED);
        assert_eq!(app.notice.as_ref().map(|n| n.text.as_str()), Some("First"));
        assert_eq!(app.notices.len(), 2, "the same words twice are one notice");
        for _ in 0..(NOTICES_WAITING + 3) {
            app.notify(format!("Busy {}", app.notices.len()), AMBER);
        }
        assert!(app.notices.len() <= NOTICES_WAITING, "a few at most wait");
        // An error does not go by itself.
        app.notices.clear();
        app.notice = None;
        app.notify(trouble, RED);
        let mut input = RawInput { time: Some(1.0), ..Default::default() };
        frame(&ctx, &mut app, input.clone());
        input.time = Some(1.0 + NOTICE_TIME * 3.0);
        frame(&ctx, &mut app, input);
        assert_eq!(app.notice.as_ref().map(|n| n.text.as_str()), Some("Trouble"), "an error stays until dismissed");
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn closing_with_work_at_risk_asks_first_and_otherwise_closes() {
        let dir = temp_dir("close_question");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        let close = || {
            let mut input = RawInput::default();
            input.viewports.insert(egui::ViewportId::ROOT, egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() });
            input
        };
        let cancelled = |output: &egui::FullOutput| output.viewport_output.get(&egui::ViewportId::ROOT)
            .is_some_and(|viewport| viewport.commands.contains(&egui::ViewportCommand::CancelClose));
        // Nothing at risk: the window closes.
        assert!(app.work_at_risk().is_empty());
        let output = frame(&ctx, &mut app, close());
        assert!(!cancelled(&output) && !app.confirm_close);
        // A skin with changes not saved: the close waits for an answer.
        let pack = ta_skins::load("builtin:desert", &dir.join("skins")).unwrap();
        let mut editor = crate::skin_editor::Editor::open(pack, Some("desert-copy".into()));
        editor.livery("wear", 0.0, |livery| livery.wear = 0.9);
        app.skins.editor = Some(editor);
        assert_eq!(app.work_at_risk().len(), 1);
        let output = frame(&ctx, &mut app, close());
        assert!(cancelled(&output), "the close is held back");
        assert!(app.confirm_close, "and the question is up");
        // CLOSE ANYWAY: the next close goes through.
        app.confirm_close = false;
        app.close_confirmed = true;
        let output = frame(&ctx, &mut app, close());
        assert!(!cancelled(&output));
        drop(app);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn maps_f5_discovers_map_packs_installed_since_the_last_scan() {
        let dir = temp_dir("maps_refresh_key");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        app.page = Page::Maps;
        frame(&ctx, &mut app, RawInput::default());
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        crate::maps::tests::write_map_archive(&dir.join("mods/new-maps.ufo"));
        frame(&ctx, &mut app, key(egui::Key::F5, egui::Modifiers::NONE));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            app.maps_tick(&ctx);
            if !app.maps.busy() && !app.maps.needs_scan { break; }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.mods.get("new-maps").is_some(), "F5 must refresh installed sources, just like the RESCAN button");
        assert!(app.maps.catalogue.entries.iter().any(|entry| entry.layer == "new-maps" && entry.stem == "moon"));
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_launch_route_waits_for_pending_friendly_and_enemy_skin_assignments() {
        let dir = temp_dir("launch_pending_skin");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        use_fake_game(&mut app, &dir);
        let frame = eframe::Frame::_new_kittest();
        for enemy in [false, true] {
            for route in 0..3 {
                app.game = GameState::Idle;
                let operation = if enemy { crate::skins::Operation::SelectEnemy("builtin:navy".into()) }
                    else { crate::skins::Operation::Select("builtin:desert".into()) };
                app.skins.start(dir.join("skins"), operation);
                assert!(app.skins.busy());
                app.apply(match route {
                    0 => Action::Play,
                    1 => Action::PlayWith(vec!["--open-skirmish".into()]),
                    _ => Action::Benchmark,
                }, &frame);
                assert!(matches!(app.game, GameState::Idle), "a pending assignment cannot launch with the previous skin: {:?}", app.game);
                assert!(app.last_notice().is_some_and(|notice| notice.text.contains("skin operation")));
                assert!(app.benchmarks.running.is_none());
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                while app.skins.busy() && std::time::Instant::now() < deadline {
                    app.logic(&ctx, &mut eframe::Frame::_new_kittest());
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert!(!app.skins.busy());
                assert_eq!(if enemy { &app.config.graphics.enemy_unit_skin } else { &app.config.graphics.unit_skin },
                    if enemy { "builtin:navy" } else { "builtin:desert" });
            }
        }
        app.play(&[]);
        assert!(matches!(app.game, GameState::Failed(_)), "once validation completes the launcher reaches the test fixture's missing game executable");
        let saved = Config::try_load_from(&app.store.path).unwrap();
        assert_eq!(saved.graphics.unit_skin, "builtin:desert");
        assert_eq!(saved.graphics.enemy_unit_skin, "builtin:navy");
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dropped_skin_uses_its_library_and_selection_is_saved_after_validation() {
        let dir = temp_dir("skin_drop");
        let config = dir.join("config.toml");
        let package = dir.join("winter.taskin");
        ta_skins::export("christmas", &dir.join("source"), &package).unwrap();
        let (ctx, mut app) = launcher(&config, &["--pretend-missing"]);
        let finish = |app: &mut LauncherApp| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.skins.busy() {
                frame(&ctx, app, RawInput::default());
                assert!(Instant::now() < deadline, "skin worker did not finish");
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        finish(&mut app);
        frame(&ctx, &mut app, dropped(&package));
        finish(&mut app);
        assert_eq!(app.page, Page::Skins);
        assert!(app.installer.packages.is_empty(), "skin packs must not reach the mod installer");
        assert_eq!(app.config.graphics.unit_skin, "reborn", "import must not activate a skin");
        assert!(app.skins.catalog.entries.iter().any(|entry| entry.id == "christmas-copy"));
        app.apply(Action::SelectSkin("christmas-copy".into()), &eframe::Frame::_new_kittest());
        finish(&mut app);
        assert_eq!(app.config.graphics.unit_skin, "christmas-copy");
        app.apply(Action::SelectEnemySkin("builtin:desert".into()), &eframe::Frame::_new_kittest());
        finish(&mut app);
        assert_eq!(app.config.graphics.enemy_skin(), "builtin:desert");
        assert_eq!(app.config.graphics.unit_skin, "christmas-copy");
        app.store.flush(&mut app.config).unwrap();
        let saved = Config::try_load_from(&config).unwrap();
        assert_eq!(saved.graphics.unit_skin, "christmas-copy");
        assert_eq!(saved.graphics.enemy_skin(), "builtin:desert");
        app.apply(Action::SelectEnemySkin(String::new()), &eframe::Frame::_new_kittest());
        assert_eq!(app.config.graphics.enemy_skin(), "christmas-copy");
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The keys wait while a key is awaited for an action (Ctrl+1 is then that key, not a page)
    /// and while a dialog is open (Enter is not Play under it).
    #[test]
    fn keys_wait_for_the_key_capture_and_dialogs() {
        let dir = temp_dir("keys_wait");
        let game = dir.join("TA");
        std::fs::create_dir_all(&game).unwrap();
        for f in ta_vfs::install::REQUIRED_FILES {
            std::fs::write(game.join(f), b"").unwrap();
        }
        let config = dir.join("config.toml");
        ta_config::Config { game_dir: Some(game), ..Default::default() }.save_to(&config).unwrap();
        let (ctx, mut app) = launcher(&config, &[]);
        frame(&ctx, &mut app, RawInput::default());
        assert!(app.detection.install().is_some(), "the made-up install is taken for the game");
        app.page = Page::Settings;
        app.settings_view.open(ta_ui::settings::Category::Controls);
        app.settings_view.capture("chat");
        frame(&ctx, &mut app, key(egui::Key::Num1, egui::Modifiers::COMMAND));
        assert_eq!(app.page, Page::Settings, "Ctrl+1 is the key wanted, not a page");
        assert!(!app.settings_view.capturing(), "the key was taken");
        app.page = Page::Play;
        app.whats_new = Some(("2.1.0".into(), "Notes.".into()));
        frame(&ctx, &mut app, key(egui::Key::Enter, egui::Modifiers::NONE));
        assert!(app.launching.is_none(), "Enter is not Play under a dialog");
        app.whats_new = None;
        app.mod_validation.selected = Some("escalation".into());
        frame(&ctx, &mut app, key(egui::Key::Enter, egui::Modifiers::NONE));
        assert!(app.launching.is_none() && matches!(app.game, GameState::Idle), "the report must not launch a game underneath it");
        app.mod_validation.selected = None;
        egui::Popup::open_id(&ctx, egui::Id::new("test_profile_menu"));
        frame(&ctx, &mut app, key(egui::Key::Num6, egui::Modifiers::COMMAND));
        assert_eq!(app.page, Page::Play, "page shortcuts wait for the open menu");
        egui::Popup::close_all(&ctx);
        frame(&ctx, &mut app, key(egui::Key::Enter, egui::Modifiers::NONE));
        assert!(matches!(app.game, GameState::Failed(_)), "Enter attempts to launch otherwise (the game executable is not built here)");
        assert!(app.launching.is_none(), "a failed start does not show a deployment animation");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Dropping: a folder of mod content is installed (shown coming on the Mods page's
    /// installed list), a classic TA archive copied in as a mod, a saved game handed to the
    /// game (which is not built here: the launch fails, and says so).
    #[test]
    fn dropped_files_do_what_they_are() {
        let dir = temp_dir("drop");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        let package = dir.join("Some Units");
        std::fs::create_dir_all(package.join("units")).unwrap();
        std::fs::write(package.join("units").join("one.fbi"), "[UNITINFO]\n{\n}\n").unwrap();
        frame(&ctx, &mut app, dropped(&package));
        frame(&ctx, &mut app, RawInput::default());
        assert_eq!((app.page, app.mods_tab), (Page::Mods, ModsTab::Library));
        let until = std::time::Instant::now() + Duration::from_secs(20);
        while app.mods.get("someunits").is_none() && std::time::Instant::now() < until {
            frame(&ctx, &mut app, RawInput::default());
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(app.mods.get("someunits").is_some(), "installed and found");
        assert!(!app.config.mods.enabled.iter().any(|e| e == "someunits"), "a mod of the player's own is not turned on by itself");
        // A classic archive.
        let ufo = dir.join("Extra Units.ufo");
        crate::maps::tests::write_map_archive(&ufo);
        frame(&ctx, &mut app, dropped(&ufo));
        // No painted frames while minimized: logic alone must finish and discover the import.
        let until = Instant::now() + Duration::from_secs(20);
        let mut hidden_frame = eframe::Frame::_new_kittest();
        while app.installer.busy() && Instant::now() < until {
            app.logic(&ctx, &mut hidden_frame);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(dir.join("mods").join("Extra Units.ufo").is_file());
        assert!(app.mods.get("extra units").is_some(), "the import is discovered without painting the launcher");
        assert!(!matches!(app.mod_validation.status("extra units"), crate::validation::ValidationStatus::Unknown), "completed imports automatically enter validation");
        assert!(!app.installer.busy(), "a completed hidden import no longer blocks an application update");
        // A saved game: the game is not built here.
        let save = dir.join("game.tasav");
        std::fs::write(&save, b"").unwrap();
        use_fake_game(&mut app, &dir);
        frame(&ctx, &mut app, dropped(&save));
        frame(&ctx, &mut app, RawInput::default());
        assert!(matches!(&app.game, GameState::Failed(m) if m.contains(launch::GAME_EXE)), "{:?}", app.game);
        // Files held over the window: the drop target shows (no panic, nothing done).
        let hovering = RawInput { hovered_files: vec![egui::HoveredFile { path: Some(save.clone()), mime: String::new() }], ..Default::default() };
        frame(&ctx, &mut app, hovering);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn drop_instructions_draw_above_their_dimmer() {
        let dir = temp_dir("drop_layer");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        app.show_drop = true;
        for _ in 0..4 { frame(&ctx, &mut app, RawInput::default()); }
        let output = frame(&ctx, &mut app, RawInput::default());
        fn inspect(shape: &egui::Shape, seen_dimmer: &mut bool, saw_title: &mut bool) {
            match shape {
                egui::Shape::Vec(shapes) => for shape in shapes { inspect(shape, seen_dimmer, saw_title); },
                egui::Shape::Rect(rect) if rect.fill == Color32::from_black_alpha(225) => {
                    assert!(!*saw_title, "the dimmer must not be painted over the instructions");
                    *seen_dimmer = true;
                }
                egui::Shape::Text(text) if text.galley.job.text.contains("DROP TO INSTALL") => {
                    assert!(*seen_dimmer, "instructions paint after their backdrop");
                    *saw_title = true;
                }
                _ => {}
            }
        }
        let (mut dimmer, mut title) = (false, false);
        for shape in output.shapes { inspect(&shape.shape, &mut dimmer, &mut title); }
        assert!(dimmer && title);
        app.skins.shutdown();
        let _ = std::fs::remove_dir_all(dir);
    }

    fn fake_game(dir: &Path) -> PathBuf {
        let game = dir.join("fake_game");
        std::fs::create_dir_all(&game).unwrap();
        for file in ta_vfs::install::REQUIRED_FILES {
            std::fs::write(game.join(file), b"").unwrap();
        }
        game
    }

    /// The fixture's game as the launcher's own: in the settings, where a launch looks again
    /// (`play` rescans before it starts the game), and detected. Set in `detection` alone, a
    /// rescan lost it on a machine without TA installed (the release runners), and found the
    /// computer's own copy on one with it. (The tests that launch also start the launcher with
    /// `--pretend-missing`, so a computer's own TA is never found in the fixture's place.)
    fn use_fake_game(app: &mut LauncherApp, dir: &Path) {
        let game = fake_game(dir);
        app.config.game_dir = Some(game.clone());
        app.detection = detect::detect(Some(&game), true);
    }

    #[test]
    fn launch_rechecks_external_settings_and_changed_mod_manifests_after_flushing() {
        let dir = temp_dir("launch_external_changes");
        let path = dir.join("config.toml");
        let config = Config { game_dir: Some(fake_game(&dir)), ..Config::default() };
        config.save_to(&path).unwrap();
        let (_ctx, mut app) = launcher(&path, &["--pretend-missing"]);
        app.skins.shutdown();

        let mut external = Config::try_load_from(&app.store.path).unwrap();
        external.mods.enabled = vec!["missing-after-save".into()];
        external.save_to(&app.store.path).unwrap();
        app.play(&[]);
        assert!(matches!(&app.game, GameState::Failed(message) if message.contains("missing-after-save")), "{:?}", app.game);

        external.mods.enabled.clear();
        external.game_dir = Some(dir.join("missing-install"));
        external.save_to(&app.store.path).unwrap();
        app.game = GameState::Idle;
        app.play(&[]);
        assert!(app.detection.install().is_none(), "the old install must not override the settings the child reads");
        assert!(!app.game.is_running());

        // Discovery must also see a dependency added without changing enabled IDs.
        let mod_dir = dir.join("mods/local");
        std::fs::create_dir_all(&mod_dir).unwrap();
        let manifest = mod_dir.join("mod.toml");
        std::fs::write(&manifest, "id = 'local'\nname = 'Local mod'\n").unwrap();
        app.config.game_dir = config.game_dir;
        app.config.mods.enabled = vec!["local".into()];
        app.discover_mods();
        assert!(matches!(app.mods.resolution, modlist::Resolution::Ok(_)));
        std::fs::write(&manifest, "id = 'local'\nname = 'Local mod'\ndepends = ['new-dependency']\n").unwrap();
        app.play(&[]);
        assert!(matches!(&app.game, GameState::Failed(message) if message.contains("new-dependency")), "{:?}", app.game);
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A game this launcher did not start (one running when it was opened, or its own started
    /// again with a battle's mods) is seen by its lock: while it runs, no mod is removed and no
    /// other game started.
    #[test]
    fn a_game_this_launcher_did_not_start_holds_its_mods() {
        let dir = temp_dir("game_elsewhere");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        app.config.game_dir = Some(fake_game(&dir));
        app.rescan();
        app.follow_game_elsewhere(&ctx);
        assert!(!app.game_running(), "no game");
        // (A game takes the lock in the folder of the launcher's settings.)
        ta_config::running::hold(&app.store.dir());
        let start = Instant::now();
        while !ta_config::running::held(&app.store.dir()) {
            assert!(start.elapsed() < Duration::from_secs(10), "the lock is taken");
            std::thread::sleep(Duration::from_millis(10));
        }
        app.elsewhere_checked = None;
        app.follow_game_elsewhere(&ctx);
        assert!(app.game_running(), "a game holds the lock");
        // (The pages' buttons wait for it, as their actions do; the update tells what it waits for.)
        assert!(app.removal_blocked().is_some() && app.maps_busy_reason().is_some(), "REMOVE and the map library wait");
        assert_eq!(app.update_blocked(), Some(UpdateWait::Game));
        assert_eq!(app.update_waits_for().map(UpdateWait::queued), Some("READY · IT INSTALLS WHEN THE GAME CLOSES"));
        app.apply(Action::Uninstall("escalation".into()), &eframe::Frame::_new_kittest());
        assert!(app.confirm_uninstall.is_none(), "no mod is removed while it runs");
        app.apply(Action::Play, &eframe::Frame::_new_kittest());
        assert!(matches!(app.game, GameState::Idle), "no second game");
        assert!(app.last_notice().is_some_and(|notice| notice.text.contains("already running")));
        drop(ctx);
        drop(app);
        // (The lock stays held by this test's process until it ends: its folder is left.)
    }

    #[test]
    fn every_launch_route_waits_for_queued_mod_publication() {
        let dir = temp_dir("launch_install_busy");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        app.config.game_dir = Some(fake_game(&dir));
        app.rescan();
        app.installer.stages.insert("updating-mod".into(), crate::getmods::Stage::Queued);
        app.config.mods.enabled = vec!["escalation".into(), "my-addon".into()];
        for action in [Action::Play, Action::PlayWith(vec!["--open-skirmish".into()]), Action::Benchmark, Action::QuickBenchmark, Action::PlayMod("totalmayhem".into())] {
            app.apply(action, &eframe::Frame::_new_kittest());
            assert!(matches!(app.game, GameState::Idle), "no child may read files during replacement");
            assert!(app.last_notice().is_some_and(|notice| notice.text.contains("Wait for mod installations")));
            assert!(app.benchmarks.running.is_none());
            // (The card's PLAY turns its mod on for its game: refused, the order stays as it was.)
            assert_eq!(app.config.mods.enabled, ["escalation", "my-addon"]);
        }
        app.installer.stages.clear();
        drop(ctx); drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_skin_save_as_retains_the_original_edit_target() {
        let dir = temp_dir("skin_save_as_failure");
        let (_ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        let mut pack = ta_skins::load("builtin:desert", &dir).unwrap();
        pack.manifest.id = "existing-skin".into();
        app.skins.editor = Some(crate::skin_editor::Editor::open(pack, Some("existing-skin".into())));
        // An unwritable library path makes the asynchronous save fail.
        let root = dir.join("skins");
        std::fs::write(&root, b"not a directory").unwrap();
        app.apply(Action::SaveSkinAs("Copy".into()), &eframe::Frame::_new_kittest());
        app.skins.shutdown();
        assert!(app.skins.feedback.as_ref().is_some_and(|feedback| feedback.error));
        assert_eq!(app.skins.editor.as_ref().unwrap().id.as_deref(), Some("existing-skin"), "a failed copy must not redirect the next Save");
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn pending_skin_operations_cannot_complete_into_a_replacement_editor() {
        let dir = temp_dir("skin_pending_editor");
        let (_ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        // A passive startup scan may finish after a new editor is opened.
        app.apply(Action::NewSkin, &eframe::Frame::_new_kittest());
        let serial = app.skins.editor.as_ref().unwrap().serial;
        app.skins.shutdown();
        app.skins.start(dir.join("skins"), crate::skins::Operation::ReadImage {
            path: dir.join("missing.png"), scope: None, texture: "armour".into(),
        });
        app.apply(Action::NewSkin, &eframe::Frame::_new_kittest());
        app.apply(Action::CustomizeSkin("builtin:desert".into()), &eframe::Frame::_new_kittest());
        assert_eq!(app.skins.editor.as_ref().unwrap().serial, serial, "the pending image belongs to this editor");
        app.skins.shutdown();
        assert_eq!(app.skins.editor.as_ref().unwrap().serial, serial);
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn mods_that_cannot_be_loaded_together_are_refused_at_play() {
        let dir = temp_dir("launch_mods_refused");
        std::fs::write(dir.join("config.toml"), "[mods]\nenabled = [\"gone\"]\n").unwrap();
        let (_ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        use_fake_game(&mut app, &dir);
        app.play(&[]);
        assert!(matches!(&app.game, GameState::Failed(message) if message.contains("cannot be loaded together") && message.contains("gone")), "{:?}", app.game);
        // Without mods (the benchmark's runs), it goes on to start the game (not built here).
        app.game = GameState::Idle;
        app.play(&[OsString::from("--no-mods")]);
        assert!(matches!(&app.game, GameState::Failed(message) if !message.contains("cannot be loaded together")), "{:?}", app.game);
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_launch_route_stops_when_settings_cannot_be_saved() {
        let dir = temp_dir("launch_save_failure");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        use_fake_game(&mut app, &dir);
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, "a file cannot hold config.toml").unwrap();
        app.store.path = blocked.join("config.toml");
        app.config.audio.master = 0.17;
        for action in [Action::Play, Action::PlayWith(vec!["--open-skirmish".into()]), Action::Benchmark] {
            app.page = Page::Benchmark;
            app.push(action);
            frame(&ctx, &mut app, RawInput::default());
            assert!(matches!(&app.game, GameState::Failed(message) if message.contains("Retry")), "{:?}", app.game);
            assert!(matches!(app.store.status, SaveStatus::Failed(_)), "the save error stays actionable");
            assert_eq!(app.page, Page::Play, "the persistent explanation is visible from every launch route");
            assert!(app.launching.is_none() && app.after_launch.is_none() && app.benchmarks.running.is_none());
            assert_eq!(app.config.audio.master, 0.17);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_start_keeps_the_launcher_open() {
        let dir = temp_dir("failed_start_stays");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        app.game = GameState::Exited { code: Some(1), success: false, ran_for: Duration::from_millis(100), log: None, crash: None };
        app.after_launch = Some(ta_config::OnPlay::Close);
        let output = frame(&ctx, &mut app, RawInput::default());
        assert!(!output.viewport_output.values().flat_map(|v| &v.commands).any(|c| matches!(c, egui::ViewportCommand::Close | egui::ViewportCommand::Minimized(true))));
        app.after_launch = Some(ta_config::OnPlay::Close);
        app.launching = Some(0.0);
        app.game_ended(&ctx);
        assert!(app.after_launch.is_none() && app.launching.is_none(), "an early exit cancels deferred window actions and the animation");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn offline_mode_also_applies_to_manual_update_checks() {
        let dir = temp_dir("offline_check");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        app.config.updates.url = "https://example.invalid/releases".into();
        app.config.updates.check = true;
        app.push(Action::CheckUpdates);
        frame(&ctx, &mut app, RawInput::default());
        assert!(matches!(app.update, UpdateState::NoSource));
        app.config.updates.check = false;
        app.push(Action::RefreshUpdates);
        frame(&ctx, &mut app, RawInput::default());
        assert!(matches!(app.update, UpdateState::Off));
        app.push(Action::CheckUpdates);
        frame(&ctx, &mut app, RawInput::default());
        assert!(matches!(app.update, UpdateState::NoSource), "manual checks still honor offline mode");
        // A local source remains useful for offline setup testing.
        app.config.updates.check = true;
        let source = dir.join("release.toml");
        std::fs::write(&source, "version = '0.0.1'\nurl = 'https://example.invalid/release'\n").unwrap();
        app.config.updates.url = format!("file:{}", source.display());
        app.push(Action::CheckUpdates);
        frame(&ctx, &mut app, RawInput::default());
        let until = Instant::now() + Duration::from_secs(5);
        while matches!(app.update, UpdateState::Checking(_)) && Instant::now() < until {
            app.update.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(app.update, UpdateState::UpToDate));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Removing a mod asks first; KEEP leaves it, REMOVE takes it out of the load order (its
    /// folder to the Recycle Bin: not done here, the dialog only).
    #[test]
    fn removing_a_mod_asks_first() {
        let dir = temp_dir("remove");
        std::fs::create_dir_all(dir.join("mods").join("keepme")).unwrap();
        std::fs::write(dir.join("mods").join("keepme").join("mod.toml"), "id = \"keepme\"\nname = \"Keep Me\"\n").unwrap();
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        app.page = Page::Mods;
        app.mods_tab = ModsTab::LoadOrder;
        app.push(Action::Uninstall("keepme".into()));
        for _ in 0..3 {
            frame(&ctx, &mut app, RawInput::default());
        }
        assert_eq!(app.confirm_uninstall.as_deref(), Some("keepme"), "the dialog stays up");
        // Escape closes it, keeping the mod.
        frame(&ctx, &mut app, key(egui::Key::Escape, egui::Modifiers::NONE));
        frame(&ctx, &mut app, RawInput::default());
        assert!(app.confirm_uninstall.is_none());
        assert!(dir.join("mods").join("keepme").is_dir());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Every state of the Play page draws: the player's battles, what is new after an update
    /// and its dialog, packages being installed and one that failed.
    #[test]
    fn pages_render_with_battles_news_and_installs() {
        let dir = temp_dir("battles");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        use_fake_game(&mut app, &dir);
        let record = |title: &str, autosave: bool, playable: bool| crate::battles::Record {
            path: dir.join("x.tasav"),
            title: title.into(),
            length: 754,
            when: std::time::SystemTime::now() - Duration::from_secs(3000),
            autosave,
            playable,
            mods: None,
        };
        app.battles.save = Some(record("Coast To Coast — 2 players", true, true));
        app.battles.replay = Some(record("The Pass — 4 players", false, false));
        app.news = Some(crate::news::Release { version: "2.1.0".into(), date: "2026-10-20".into(), summary: "A. B. C. D.".into() });
        app.whats_new = Some(("2.1.0".into(), "Intro.\n\n### Part\n\n**The world**\n- A fix,\n  running on.\n\nA paragraph.".into()));
        app.installer.packages.insert("brought".into(), "Brought".into());
        app.installer.stages.insert("brought".into(), crate::getmods::Stage::Unpacking);
        app.installer.packages.insert("broken".into(), "Broken".into());
        app.installer.stages.insert("broken".into(), crate::getmods::Stage::Failed("no Total Annihilation content was found in the package".into()));
        render_all_pages(&ctx, &mut app);
        // The game running: its time on the banner, the buttons to continue and watch held.
        app.whats_new = None;
        render_all_pages(&ctx, &mut app);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The game ending: the settings it wrote taken in as they are (nothing saved again), and
    /// the launcher's own kept.
    #[test]
    fn the_game_ending_takes_in_its_settings() {
        let dir = temp_dir("ended");
        let path = dir.join("config.toml");
        let (ctx, mut app) = launcher(&path, &[]);
        app.config.launcher.on_play = ta_config::OnPlay::Minimise;
        app.store.flush(&mut app.config).unwrap();
        // The game changed the unit limit in its options, and saved.
        let mut theirs = ta_config::Config::try_load_from(&path).unwrap();
        theirs.gameplay.unit_limit = 777;
        theirs.game_dir = Some(fake_game(&dir));
        let extra = dir.join("additional-mods");
        std::fs::create_dir_all(extra.join("after-game")).unwrap();
        std::fs::write(extra.join("after-game/mod.toml"), "id = 'after-game'\nname = 'After game'\n").unwrap();
        theirs.mods.extra_dir = Some(extra);
        theirs.save_to(&path).unwrap();
        app.game_ended(&ctx);
        assert_eq!(app.config.gameplay.unit_limit, 777);
        assert!(app.detection.install().is_some());
        assert!(app.mods.get("after-game").is_some(), "external mod folders refreshed after returning from game");
        app.skins.shutdown();
        assert_eq!(app.config.launcher.on_play, ta_config::OnPlay::Minimise);
        assert_eq!(app.store.status, SaveStatus::Unchanged, "nothing to save: the file is what the game wrote");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The window's size is kept as the player leaves it (not when maximised), and written
    /// when the launcher closes.
    #[test]
    fn the_window_size_is_kept() {
        let dir = temp_dir("window");
        let path = dir.join("config.toml");
        let (ctx, mut app) = launcher(&path, &[]);
        let mut input = RawInput::default();
        let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
        info.inner_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1300.0, 800.0)));
        info.maximized = Some(false);
        frame(&ctx, &mut app, input);
        let mut input = RawInput::default();
        let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
        info.inner_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(2560.0, 1400.0)));
        info.maximized = Some(true);
        frame(&ctx, &mut app, input);
        app.on_exit(None);
        let saved = ta_config::Config::try_load_from(&path).unwrap();
        assert_eq!(saved.launcher.window, Some([1300.0, 800.0]));
        assert!(saved.launcher.maximized);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn enlarged_interface_does_not_shrink_the_saved_window_size() {
        let dir = temp_dir("scaled_window");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        app.config.gameplay.ui_scale = 2.0;
        ctx.set_zoom_factor(2.0);
        frame(&ctx, &mut app, RawInput::default());
        let mut input = RawInput::default();
        let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
        info.inner_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(650.0, 400.0)));
        info.maximized = Some(false);
        frame(&ctx, &mut app, input);
        assert_eq!(app.window, Some(([1300.0, 800.0], false)));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A window full screen (inside gamescope, SteamOS's Game Mode) keeps the size and the
    /// maximising it has on the desktop, and is not fitted to the screen as a window is.
    #[test]
    fn a_full_screen_window_keeps_its_desktop_size() {
        let dir = temp_dir("fullscreen_window");
        let path = dir.join("config.toml");
        let (ctx, mut app) = launcher(&path, &[]);
        app.window = Some(([1120.0, 720.0], true));
        let mut input = RawInput::default();
        let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
        info.monitor_size = Some(vec2(1280.0, 800.0));
        info.outer_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0)));
        info.inner_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0)));
        (info.maximized, info.fullscreen) = (Some(false), Some(true));
        let output = frame(&ctx, &mut app, input);
        assert_eq!(app.window, Some(([1120.0, 720.0], true)));
        assert!(app.fitted, "looked at once");
        let resized = output.viewport_output.values().flat_map(|v| &v.commands).any(|c| matches!(c, egui::ViewportCommand::InnerSize(_) | egui::ViewportCommand::MinInnerSize(_)));
        assert!(!resized, "a full screen is left as it is");
        app.on_exit(None);
        let saved = ta_config::Config::try_load_from(&path).unwrap();
        assert_eq!((saved.launcher.window, saved.launcher.maximized), (Some([1120.0, 720.0]), true));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A new version: its card on the Play page and the About page's controls draw in each of
    /// its states, its notes open in the dialog, the header's pill brings back a card put away,
    /// and cancelling drops the download.
    #[test]
    fn an_update_shows_in_each_state() {
        let dir = temp_dir("update");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        for what in ["update", "updating", "update-failed"] {
            app.download = None;
            app.show(what);
            render_all_pages(&ctx, &mut app);
        }
        app.download = None;
        app.push(Action::ReleaseNotes);
        frame(&ctx, &mut app, RawInput::default());
        assert_eq!(app.whats_new.as_ref().map(|(version, _)| version.as_str()), Some("2.2.0"));
        frame(&ctx, &mut app, RawInput::default());
        app.whats_new = None;
        app.update_hidden = true;
        app.page = Page::About;
        app.push(Action::ShowUpdate);
        frame(&ctx, &mut app, RawInput::default());
        assert!(app.page == Page::Play && !app.update_hidden);
        app.show("updating");
        app.push(Action::CancelUpdate);
        frame(&ctx, &mut app, RawInput::default());
        assert!(app.download.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_ready_update_waits_for_unsaved_profile_edits() {
        let dir = temp_dir("update_profile_editor");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        app.skins.shutdown();
        app.profiles.edit(false);
        app.download = Some(crate::update::Download::shown("9.9.9", Step::Ready(dir.join("not-started.exe"))));
        app.update_go = true;
        app.updates(&ctx);
        assert!(app.update_go, "editing identity must not be interrupted by setup closing the launcher");
        assert!(app.update_error.is_none());
        app.download = None;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_ready_update_preserves_settings_when_saving_fails() {
        let dir = temp_dir("update_save_failure");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        app.skins.shutdown();
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, "not a directory").unwrap();
        app.store.path = blocked.join("config.toml");
        app.config.audio.master = 0.17;
        app.download = Some(crate::update::Download::shown("9.9.9", Step::Ready(dir.join("not-started.exe"))));
        app.update_go = true;
        let output = ctx.run_ui(RawInput::default(), |ui| app.updates(ui.ctx()));
        let SaveStatus::Failed(error) = &app.store.status else { panic!("save failure must remain actionable"); };
        assert_eq!(app.update_error.as_ref(), Some(error), "setup must not run after the failed preflight");
        assert!(!app.update_go, "retry is explicit, not repeated every frame");
        assert_eq!(app.config.audio.master, 0.17);
        assert!(!output.viewport_output[&egui::ViewportId::ROOT].commands.iter().any(|c| matches!(c, egui::ViewportCommand::Close)));
        output.drop_without_applying_deltas();
        app.download = None;
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_drops_do_not_launch_or_import_behind_a_dialog() {
        let dir = temp_dir("modal_drop");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        let install = fake_game(&dir);
        for dialog in 0..10 {
            match dialog {
                0 => app.show_shortcuts = true,
                1 => app.whats_new = Some(("test".into(), "notes".into())),
                2 => app.confirm_uninstall = Some("test".into()),
                3 => app.mod_validation.selected = Some("test".into()),
                4 => { app.page = Page::Settings; app.settings_view.capture("chat"); }
                5 => egui::Popup::open_id(&ctx, egui::Id::new("drop_menu")),
                6 => app.confirm_close = true,
                7 => app.confirm_end_game = true,
                8 => app.show_progress = true,
                _ => app.profiles.edit(false),
            }
            for path in [&install, &dir.join("game.tasav"), &dir.join("units.zip"), &dir.join("skin.taskin")] {
                ctx.run_ui(dropped(path), |ui| {
                    // A real menu is kept open by its widget on each frame.
                    if dialog == 5 { egui::Popup::open_id(ui.ctx(), egui::Id::new("drop_menu")); }
                    app.dropped_files(ui.ctx());
                }).drop_without_applying_deltas();
                assert!(app.actions.is_empty(), "no launch or import behind dialog {dialog}");
                assert!(app.config.game_dir.is_none(), "a drop cannot change the installation behind a dialog");
                assert!(app.last_notice().is_some_and(|n| n.text.contains("File drops are unavailable")));
            }
            app.show_shortcuts = false;
            app.whats_new = None;
            app.confirm_uninstall = None;
            app.mod_validation.selected = None;
            app.confirm_close = false;
            app.confirm_end_game = false;
            app.show_progress = false;
            app.settings_view = Default::default();
            egui::Popup::close_all(&ctx);
        }
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn closing_ending_and_progress_dialogs_block_navigation_and_page_shortcuts() {
        let dir = temp_dir("modal_shortcuts");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &["--pretend-missing"]);
        app.skins.shutdown();
        for dialog in 0..3 {
            app.confirm_close = dialog == 0;
            app.confirm_end_game = dialog == 1;
            app.show_progress = dialog == 2;
            app.page = Page::Settings;
            ctx.run_ui(key(egui::Key::Num2, egui::Modifiers::COMMAND), |ui| app.shortcuts(ui.ctx())).drop_without_applying_deltas();
            assert_eq!(app.page, Page::Settings);
            ctx.run_ui(key(egui::Key::F, egui::Modifiers::COMMAND), |ui| app.settings_page(ui)).drop_without_applying_deltas();
            assert!(!ctx.egui_wants_keyboard_input(), "a covered search must not steal the dialog's keys");
            ctx.run_ui(key(egui::Key::F1, egui::Modifiers::NONE), |ui| app.shortcuts(ui.ctx())).drop_without_applying_deltas();
            assert!(!app.show_shortcuts, "a second modal must not open over the current question");
        }
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A window opened maximised (as it was left) stays so: it fills the screen, which is not
    /// taken for a window too big for it.
    #[test]
    fn a_maximised_window_stays_maximised() {
        let dir = temp_dir("maximised");
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        let screen = |maximized: bool, inner: egui::Vec2| {
            let mut input = RawInput::default();
            let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
            info.monitor_size = Some(vec2(1920.0, 1080.0));
            info.inner_rect = Some(Rect::from_min_size(Pos2::ZERO, inner));
            info.outer_rect = Some(Rect::from_min_size(Pos2::ZERO, inner + vec2(0.0, 31.0)));
            info.maximized = Some(maximized);
            input
        };
        let resized = |output: &egui::FullOutput| output.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::InnerSize(_))));
        let output = frame(&ctx, &mut app, screen(true, vec2(1920.0, 1017.0)));
        assert!(!resized(&output), "left maximised");
        // (Too big a window that is not maximised is fitted.)
        let (ctx, mut app) = launcher(&dir.join("config.toml"), &[]);
        let output = frame(&ctx, &mut app, screen(false, vec2(1920.0, 1040.0)));
        assert!(resized(&output));
        let _ = std::fs::remove_dir_all(dir);
    }
}
