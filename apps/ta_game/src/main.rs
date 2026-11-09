//! TA Reborn — entry point.

// Release builds are windowed programs: no console window (the log goes to a file).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod accessibility;
mod audio;
mod backdrop;
mod benchmark;
mod battle_history;
mod briefing;
mod campaign;
mod chat;
mod chatter;
mod chronicle;
mod clip_dialog;
mod clips;
mod conquest;
mod conquest_ui;
mod controls;
mod credits;
mod credits_show;
mod credits_song;
mod encyclopaedia;
mod cursor;
mod editor;
mod effects;
mod feature_strokes;
mod features;
mod fog;
mod game;
mod gamepad;
mod highlights;
mod hint;
mod hud;
mod coach;
mod hud_classic;
mod hud_classic_art;
mod hud_classic_scale;
mod hud_icons;
mod hud_skin;
mod input;
mod instruments;
mod keys;
mod keyboard_pointer;
mod intro;
mod intro_room;
mod intro_room_art;
mod jukebox;
mod lobby;
mod loading;
mod lore;
mod map_preview;
mod menu;
mod minimap;
mod night;
mod ore;
mod orders;
mod overlay;
mod palette;
mod perspective;
mod perspective_hud;
mod photo;
mod radial;
mod radial_motion;
mod profile_progress;
mod picture;
mod planet;
mod record_mods;
mod replay;
mod replay_scan;
mod film;
mod companion;
mod result;
mod rivals;
mod scenario;
mod scene;
mod setup;
mod signals;
mod showroom;
mod sound_log;
mod space;
mod stage;
mod stats;
mod strategic;
mod team_markers;
mod theme;
mod timeline;
mod tips;
mod tutorial;
mod world_text;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "ta-reborn", about = "Total Annihilation Reborn")]
pub struct Args {
    /// Local player profile ID (normally selected in the launcher).
    #[arg(long)]
    pub profile: Option<String>,
    /// Path to the original Total Annihilation install (auto-detected when omitted).
    #[arg(long)]
    pub game_dir: Option<PathBuf>,
    /// Start a skirmish on this map directly (name without extension, e.g. "Coast To
    /// Coast"), skipping the intro and menu.
    #[arg(long)]
    pub map: Option<String>,
    /// Plays the benchmark (its scenes and tests, measured), writes its report to this file and
    /// closes: the launcher's BENCHMARK.
    #[arg(long, value_name = "FILE")]
    pub benchmark: Option<PathBuf>,
    /// The benchmark's length: `full` (every scene and both tests, about five minutes) or `quick`
    /// (what the scores are made of, about a minute and a half).
    #[arg(long, value_name = "MODE", default_value = "full", value_parser = ["full", "quick"])]
    pub benchmark_mode: String,
    /// Skip the intro movies.
    #[arg(long)]
    pub skip_intro: bool,
    /// Open the skirmish setup page directly (for captures and testing).
    #[arg(long, hide = true)]
    pub open_skirmish: bool,
    /// Open the skirmish setup page with this map chosen (name without extension, e.g.
    /// "Coast To Coast"): the launcher's SET UP SKIRMISH.
    #[arg(long, value_name = "MAP")]
    pub skirmish_map: Option<String>,
    /// Open the campaign page directly (for captures and testing).
    #[arg(long, hide = true)]
    pub open_campaign: bool,
    /// Start a campaign mission directly: "<campaign>:<mission index>", e.g. "arm campaign:0".
    #[arg(long)]
    pub mission: Option<String>,
    /// Start a scenario directly, by its id ("last-stand"); `TA_SCENARIO_DIFFICULTY`
    /// easy|normal|hard.
    #[arg(long)]
    pub scenario: Option<String>,
    /// Start a tutorial lesson directly (1 to 3).
    #[arg(long)]
    pub tutorial: Option<usize>,
    /// Host a multiplayer lobby on this port.
    #[arg(long)]
    pub host: Option<u16>,
    /// Join a multiplayer lobby at "host[:port]".
    #[arg(long)]
    pub join: Option<String>,
    /// With --join: watch the game as a spectator instead of playing.
    #[arg(long)]
    pub watch: bool,
    /// Load a saved game (.tasav) and continue playing it.
    #[arg(long)]
    pub load: Option<std::path::PathBuf>,
    /// Watch a replay (.tarep).
    #[arg(long)]
    pub replay: Option<std::path::PathBuf>,
    /// A replay (.tarep) or saved game (.tasav) to open, as Explorer passes it when one is
    /// double-clicked or dropped on the program.
    #[arg(value_name = "FILE")]
    pub file: Option<PathBuf>,
    /// With --load or --replay: open it as it is, though it was made with other mods or game
    /// data (the launcher asked the player first): a saved game plays its orders over with
    /// today's data, a replay plays back approximately.
    #[arg(long, hide = true)]
    pub anyway: bool,
    /// Hosting: start automatically once this many players are in and ready (testing).
    #[arg(long, hide = true)]
    pub auto_start: Option<usize>,
    /// Print the simulation checksum at this tick, then exit (lockstep testing).
    #[arg(long, hide = true)]
    pub net_check_tick: Option<u64>,
    /// Start in TA's classic oblique projection instead of the perspective camera.
    #[arg(long)]
    pub classic: bool,
    /// Capture a screenshot to this path after `--frames` frames, then exit.
    #[arg(long)]
    pub screenshot: Option<PathBuf>,
    /// Frames to render before taking the screenshot.
    #[arg(long, default_value_t = 20)]
    pub frames: u32,
    /// Window size (defaults to the launcher settings).
    #[arg(long)]
    pub width: Option<u32>,
    #[arg(long)]
    pub height: Option<u32>,
    /// Ignore mods enabled in the launcher.
    #[arg(long)]
    pub no_mods: bool,
    /// Initial camera: focus x,z (map pixels), zoom, yaw and pitch (degrees), e.g. "1500,1200,1.5,0,63".
    #[arg(long)]
    pub camera: Option<String>,
    /// Scripted demo for automated captures ("build", "factory").
    #[arg(long)]
    pub demo: Option<String>,
    /// Open the developer panel (Ctrl+F1) at start, with GPU pass timing on (captures).
    #[arg(long, hide = true)]
    pub dev_panel: bool,
    /// Use the classic interface whatever the settings say (captures).
    #[arg(long, hide = true)]
    pub classic_ui: bool,
    /// Open a page or dialog at start (captures): records, multiplayer, options, loading,
    /// tutorial, space (a scene of the film's, `TA_SPACE=<scene.toml>`);
    /// in a battle also pause, stats, controls, victory, defeat.
    #[arg(long, hide = true)]
    pub open: Option<String>,
    /// Open the map editor on this map (saving writes a new map to the mods folder), or on
    /// a new map when no name follows.
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    pub editor: Option<String>,
    /// With --editor: open one of its dialogs or tools at start (captures): new, open,
    /// properties, picker, tileset, heights, tiles, copy, features, starts, or demo (every
    /// tool used once through scripted mouse input; demo-save then saves it as a mod).
    #[arg(long, hide = true)]
    pub editor_open: Option<String>,
}

/// Log lines to stderr and to our own log file.
struct Tee(std::fs::File);

impl std::io::Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = std::io::stderr().write_all(buf);
        self.0.write_all(buf)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let _ = std::io::stderr().flush();
        self.0.flush()
    }
}

impl Args {
    /// Automated captures, network probes and launcher benchmarks report errors without
    /// a native dialog that would leave their parent waiting for the process forever.
    fn unattended(&self) -> bool {
        self.screenshot.is_some() || self.net_check_tick.is_some() || self.auto_start.is_some() || self.benchmark.is_some()
    }

    fn resolve_record_file(&mut self) -> Result<()> {
        if let Some(file) = self.file.take() {
            match file.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).as_deref() {
                Some("tarep") => self.replay = Some(file),
                Some("tasav") => self.load = Some(file),
                _ => anyhow::bail!(ta_ui::i18n::tf("{file} is not a replay (.tarep) or a saved game (.tasav).", &[("file", &file.display())])),
            }
        }
        Ok(())
    }
}

/// Scripted runs (screenshots, network checks, benchmarks) must never stop at a dialog.
static UNATTENDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The names of the mods the game runs with (shown on the title screen).
static MOD_NAMES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

pub fn mod_names() -> &'static [String] {
    MOD_NAMES.get().map_or(&[], |v| v.as_slice())
}

/// Why the mods that are on were not loaded, when they were not (the title screen says so).
static MOD_PROBLEM: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn mod_problem() -> Option<&'static str> {
    MOD_PROBLEM.get().map(String::as_str)
}

/// A load order's problem in the player's words, the mods by their names (as the launcher
/// tells it). `problems` are the scan's: a mod "not installed" may be one whose manifest does
/// not read.
fn describe_mod_problem(e: &ta_vfs::mods::ModError, available: &[ta_vfs::mods::ModInfo], problems: &[ta_vfs::mods::ModError]) -> String {
    use ta_ui::i18n::tf;
    use ta_vfs::mods::ModError;
    let name = |id: &str| available.iter().find(|m| m.manifest.id == id).map_or(id.to_string(), |m| m.manifest.name.clone());
    match e {
        ModError::Missing(id) => match ta_vfs::mods::broken_manifest(id, problems) {
            Some(ModError::Manifest { path, message }) => tf("{name} is on, but its manifest {file} does not read: {error}", &[("name", &name(id)), ("file", &ta_vfs::mods::shown_path(path)), ("error", &message.trim())]),
            _ => tf("{name} is on, but it is not installed", &[("name", &name(id))]),
        },
        ModError::MissingDependency { module, dependency } => tf("{name} needs {other}, which is not on", &[("name", &name(module)), ("other", &name(dependency))]),
        ModError::Cycle(ids) => tf("These mods each need another loaded before them: {mods}", &[("mods", ids)]),
        ModError::Unreadable { id, message } => tf("{name} could not be read: {error}", &[("name", &name(id)), ("error", message)]),
        ModError::Manifest { .. } | ModError::Depth(_) | ModError::Duplicate { .. } => e.to_string(),
    }
}

/// The original game's folder (its loose files: the readmes the credits come from).
static INSTALL_ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

pub fn install_root() -> Option<&'static std::path::Path> {
    INSTALL_ROOT.get().map(|p| p.as_path())
}

pub fn version_label() -> String {
    match env!("TA_BUILD_COMMIT") {
        "" => env!("CARGO_PKG_VERSION").to_string(),
        commit => format!("{} ({commit})", env!("CARGO_PKG_VERSION")),
    }
}

/// Where the executable was loaded. An installed build has no symbols, so a crash report's
/// frames are bare addresses: less this, they are offsets into the release's symbols.
fn image_base() -> usize {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetModuleHandleW(name: *const u16) -> *mut core::ffi::c_void;
        }
        unsafe { GetModuleHandleW(std::ptr::null()) as usize }
    }
    #[cfg(not(windows))]
    0
}

/// A native error box (Windows); elsewhere, or in scripted runs, the message goes to
/// stderr only.
fn message_box(title: &str, text: &str) {
    if UNATTENDED.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!("{title}: {text}");
        return;
    }
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn MessageBoxW(hwnd: *mut core::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
        }
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (t, c) = (wide(text), wide(title));
        // MB_ICONERROR | MB_SETFOREGROUND
        unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), 0x10 | 0x1_0000) };
    }
    #[cfg(not(windows))]
    eprintln!("{title}: {text}");
}

fn main() {
    // The settings folder kept under the game's old name moves to the new one first.
    let moved = ta_config::adopt_old_folder();
    let log_dir = ta_config::config_dir().join("logs");
    let mut logger = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"));
    // The launcher captures our output into logs\game.log itself; started any other way,
    // we write it.
    if std::env::var_os("TA_LOG_REDIRECTED").is_none() {
        let file = std::fs::create_dir_all(&log_dir).and_then(|_| std::fs::File::create(log_dir.join("game.log")));
        if let Ok(file) = file {
            logger.target(env_logger::Target::Pipe(Box::new(Tee(file))));
        }
    }
    logger.init();
    log::info!("TA Reborn {}", version_label());
    if let Some(note) = moved {
        log::info!("{note}");
    }
    // Crashes: the message and a backtrace go to the log and logs\crash.txt, and the
    // player gets a box saying where. (Frames with their addresses: see `image_base`.)
    let crash_path = log_dir.join("crash.txt");
    std::panic::set_hook(Box::new(move |info| {
        // (A damaged music file, caught where it was read: it is skipped, nothing crashed.)
        if ta_audio::guarded_panic() {
            log::warn!("a music file could not be read: {info}");
            return;
        }
        // (The graphics card refused an interface's drawing its memory: the frame went without it.)
        if ta_render::gpu::short_of_memory_panic() {
            log::warn!("graphics memory: {info}");
            return;
        }
        let backtrace = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current();
        let report = format!(
            "TA Reborn {} crashed: {info}\nthread: {}\nloaded at: {:#x}\n\n{backtrace:#}",
            version_label(),
            thread.name().unwrap_or("unnamed"),
            image_base()
        );
        log::error!("{report}");
        let _ = std::fs::write(&crash_path, &report);
        message_box("TA Reborn crashed", &format!("{info}\n\nThe details were saved to {}.", crash_path.display()));
    }));
    if let Err(e) = run() {
        log::error!("{e:#}");
        message_box("TA Reborn", &format!("{e:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = Args::parse();
    UNATTENDED.store(args.unattended(), std::sync::atomic::Ordering::Relaxed);
    args.resolve_record_file()?;
    // `--demo stage`: the scene file (`TA_STAGE`), read before anything asks for it.
    stage::init(args.demo.as_deref())?;
    let profile_root = std::env::var_os("TA_PROFILE_ROOT").map(PathBuf::from).unwrap_or_else(ta_config::config_dir);
    let profile_id = args.profile.clone().or_else(|| std::env::var("TA_PROFILE_ID").ok());
    let profile = ta_config::profiles::initialize(profile_root, profile_id.as_deref())?;
    let mut config = ta_config::Config::load();
    if config.sanitize() { log::warn!("invalid settings were restored to supported ranges"); }
    if let Some(profile) = &profile { config.gameplay.player_name = profile.name.clone(); }
    // The interface's language (`TA_LANG` overrides it, e.g. for captures).
    let language = std::env::var("TA_LANG").unwrap_or_else(|_| config.gameplay.language.clone());
    if let Err(e) = ta_ui::i18n::set_language(&language, &ta_ui::i18n::language_dirs(&ta_config::config_dir())) {
        log::warn!("{e}");
    }
    let game_dir = args.game_dir.clone().or_else(|| config.game_dir.clone());
    let install = ta_vfs::install::discover(game_dir.as_deref()).map_err(|e| {
        anyhow::anyhow!(
            "{e}\n\nTA Reborn needs the original Total Annihilation data. Install \
             'Total Annihilation: Commander Pack' from GOG ({}) or Steam ({}), or pass --game-dir.",
            ta_vfs::install::GOG_STORE_URL,
            ta_vfs::install::STEAM_STORE_URL
        )
    })?;
    log::info!("using TA install at {} ({})", install.root.display(), install.source);
    let _ = INSTALL_ROOT.set(install.root.clone());
    // (The benchmark plays without mods: every run the same game.)
    let (enabled, available, problems) = if !args.no_mods && args.benchmark.is_none() && !config.mods.enabled.is_empty() {
        let app_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
        let folders = ta_vfs::mods::default_folders(app_dir.as_deref(), &ta_config::config_dir(), config.mods.extra_dir.as_deref());
        let (available, problems) = ta_vfs::mods::scan_folders(&folders);
        for p in &problems {
            log::warn!("mods: {p}");
        }
        (config.mods.enabled.clone(), available, problems)
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };
    // The install (less the archives the mods replace), TA Reborn's own maps over it and under
    // the mods, then the mods.
    let builtin = ta_vfs::builtin_dir();
    let mounted = ta_vfs::mods::mount_enabled(Some(&install.root), builtin.as_deref(), &enabled, &available);
    mounted.install?;
    match (&builtin, &mounted.builtin) {
        (Some(dir), Ok(())) => log::info!("built-in content from {}", dir.display()),
        (Some(dir), Err(e)) => log::warn!("built-in content {}: {e}", dir.display()),
        (None, _) => {}
    }
    let mut in_play = record_mods::InPlay::default();
    if !enabled.is_empty() {
        match mounted.mods {
            Ok(ids) => {
                log::info!("mods mounted: {ids:?}");
                if !mounted.replaced.is_empty() {
                    log::info!("the install's archives the mods replace: {:?}", mounted.replaced);
                }
                let on: Vec<ta_vfs::mods::ModInfo> = ids.iter().filter_map(|id| available.iter().find(|m| m.manifest.id == *id)).cloned().collect();
                let _ = MOD_NAMES.set(on.iter().map(|m| m.manifest.name.clone()).collect());
                // (What saved games and replays made in this session name: their digests worked
                // out meanwhile.)
                in_play = record_mods::InPlay::new(on);
            }
            Err(e) => {
                log::error!("mods not loaded: {e}");
                let _ = MOD_PROBLEM.set(describe_mod_problem(&e, &available, &problems));
            }
        }
    }
    app::run(args, mounted.vfs, in_play, config, profile)
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    #[test]
    fn automated_runs_suppress_error_dialogs_including_launcher_benchmarks() {
        for flags in [vec!["--benchmark", "report.json"], vec!["--screenshot", "frame.png"],
            vec!["--net-check-tick", "30"], vec!["--auto-start", "2"]]
        {
            let args = Args::try_parse_from(std::iter::once("ta-reborn").chain(flags)).unwrap();
            assert!(args.unattended());
        }
        assert!(!Args::try_parse_from(["ta-reborn"]).unwrap().unattended());
    }

    #[test]
    fn invalid_positional_files_are_rejected_under_the_unattended_error_policy() {
        let mut args = Args::try_parse_from(["ta-reborn", "--screenshot", "frame.png", "invalid.txt"]).unwrap();
        assert!(args.unattended(), "error policy must be available before resolving the file");
        assert!(args.resolve_record_file().unwrap_err().to_string().contains("invalid.txt"));
    }

    /// Why the mods are not loaded, as the title screen tells it: a manifest that does not read
    /// is named as that, not as a mod not installed; a mod that does not read says why.
    #[test]
    fn a_mods_problem_is_told_as_it_is() {
        use ta_vfs::mods::ModError;
        let broken = [ModError::Manifest { path: PathBuf::from("mods/mymod/mod.toml"), message: "invalid type: floating point `1.0`, expected a string\n".into() }];
        let text = describe_mod_problem(&ModError::Missing("mymod".into()), &[], &broken);
        assert!(text.contains("does not read") && text.contains("floating point"), "{text}");
        assert!(text.contains(&PathBuf::from("mymod").join("mod.toml").display().to_string()) && !text.contains("mods"), "the mod's folder, not where it is: {text}");
        assert!(describe_mod_problem(&ModError::Missing("mymod".into()), &[], &[]).contains("not installed"));
        let text = describe_mod_problem(&ModError::Unreadable { id: "pack".into(), message: "bad header".into() }, &[], &[]);
        assert!(text.contains("could not be read") && text.contains("bad header"), "{text}");
    }

    #[test]
    fn file_associations_accept_mixed_case_record_extensions() {
        for (path, replay) in [("Battle.TaReP", true), ("Battle.TaSaV", false)] {
            let mut args = Args::try_parse_from(["ta-reborn", path]).unwrap();
            args.resolve_record_file().unwrap();
            assert!(args.file.is_none());
            assert_eq!(if replay { args.replay } else { args.load }, Some(PathBuf::from(path)));
        }
    }
}
