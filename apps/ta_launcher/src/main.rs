//! TA Reborn launcher (`ta-launcher`).
//!
//! TA Reborn ships no original game data. The launcher finds the user's own Total
//! Annihilation install (GOG Commander Pack, Steam, or a folder they pick), manages mods and
//! settings in each local player profile, and starts `ta-reborn.exe` with that install.

// Release builds are GUI-only on Windows; debug builds keep the console for log output.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod battles;
mod benchmarks;
mod capture;
mod cursor;
mod catalogue;
mod detect;
mod film;
mod getmods;
mod hero;
mod http;
mod icon;
mod instance;
mod launch;
mod library;
mod moddb;
mod modlist;
mod music_control;
mod native_crash;
mod maps;
mod mod_library;
mod news;
mod pad;
mod pages;
mod persist;
mod theme;
mod sounds;
mod skins;
mod skin_patterns;
mod skin_editor;
mod update;
mod validation;
mod widgets;

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "ta-launcher", version, about = "TA Reborn launcher")]
pub struct Args {
    /// Use this settings file instead of `%APPDATA%\TA Reborn\config.toml`. Its folder
    /// also holds profiles, shared mods and the game log. Child games use this same root.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Page to show first.
    #[arg(long, value_enum, default_value_t = app::Page::Play)]
    pub page: app::Page,
    /// Debug: press Play as soon as the launcher starts.
    #[arg(long, hide = true)]
    pub autoplay: bool,
    /// Debug: run the benchmark as soon as the launcher starts (its report comes to the
    /// Benchmark page when the game ends).
    #[arg(long, hide = true)]
    pub autobenchmark: bool,
    /// Debug: update as soon as the check finds a new version (the update tried end to end, with
    /// a `file:` update source).
    #[arg(long, hide = true)]
    pub autoupdate: bool,
    /// Debug: make auto-detection find nothing, as if the original game were not installed.
    #[arg(long, hide = true)]
    pub pretend_missing: bool,
    /// Debug: render a few frames, save the window as a PNG to this path, then exit.
    #[arg(long, hide = true, value_name = "PNG")]
    pub screenshot: Option<PathBuf>,
    /// Debug: frames to render before taking the screenshot.
    #[arg(long, hide = true, default_value_t = 12)]
    pub screenshot_frames: u32,
    /// Debug: scroll the page down by this many points before taking the screenshot.
    #[arg(long, hide = true, value_name = "POINTS")]
    pub scroll: Option<f32>,
    /// Debug: start installing this catalogue mod (by id) as the launcher opens.
    #[arg(long, hide = true, value_name = "ID")]
    pub get_mod: Option<String>,
    /// Open the Mods page on this mod (by id): the game asks for one a saved game or replay
    /// needs. With a launcher running already, that one shows it.
    #[arg(long = "mod", value_name = "ID")]
    pub show_mod: Option<String>,
    /// Stay off the network: the mods page shows only what is cached (tests).
    #[arg(long, hide = true)]
    pub offline: bool,
    /// Debug: the Mods page's tab to show first.
    #[arg(long, hide = true, value_enum, default_value_t = app::ModsTab::Library)]
    pub mods_tab: app::ModsTab,
    /// Debug: initial inner window size in points, e.g. `1120x720`.
    #[arg(long, hide = true, value_name = "WxH", value_parser = parse_size)]
    pub window_size: Option<[f32; 2]>,
    /// Debug: the settings category to show first (`graphics`, `sound`...).
    #[arg(long, hide = true, value_name = "CATEGORY")]
    pub settings_category: Option<String>,
    /// Debug: show a dialog or state for a capture: `whats-new`, `remove:<mod id>`, `crash`,
    /// `drop` (files held over the window), `running` (the game running), `update`, `updating`,
    /// `update-failed`, `update-notes` (a new version, its download, its failure, its notes), `shortcuts`, `search:<text>`,
    /// `steam-install` (the copy found shown as Steam's), `hidden[:<id>,<id>]` (the Mods library's
    /// hidden mods listed; those ids hidden first, in the settings).
    #[arg(long, hide = true, value_name = "WHAT")]
    pub show: Option<String>,
    /// Debug: film the launcher from a script (pointer, clicks, pages, a mod's download) on a
    /// steady clock, every frame saved as a PNG and its sounds written down, then exit (`film.rs`).
    #[arg(long, hide = true, value_name = "TOML")]
    pub film: Option<PathBuf>,
}

fn parse_size(text: &str) -> Result<[f32; 2], String> {
    let (w, h) = text.split_once(['x', 'X']).ok_or("expected WIDTHxHEIGHT")?;
    let parse = |v: &str| v.trim().parse::<f32>().map_err(|e| e.to_string());
    let size = [parse(w)?, parse(h)?];
    valid_size(size).then_some(size).ok_or_else(|| "window dimensions must be finite, positive and at most 16384 points".into())
}

fn valid_size(size: [f32; 2]) -> bool {
    size.into_iter().all(|v| v.is_finite() && v > 0.0 && v <= 16384.0)
}

/// The version with the commit the build was made from: "1.0.0 (c752ef7a1)".
pub fn version_label() -> String {
    match env!("TA_BUILD_COMMIT") {
        "" => env!("CARGO_PKG_VERSION").to_string(),
        commit => format!("{} ({commit})", env!("CARGO_PKG_VERSION")),
    }
}

/// Whether a panic on the thread named `thread` takes the launcher down: the window's thread,
/// the main one (eframe runs the interface on the thread that starts it). Workers' threads end
/// alone, or catch their panics where they run.
fn crashes_launcher(thread: Option<&str>) -> bool {
    thread == Some("main")
}

/// Log lines to stderr and to the launcher's log file.
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

fn main() {
    let args = Args::parse();
    let unattended = args.screenshot.is_some() || args.film.is_some();
    // One launcher at a time (not for captures and tests, nor with settings of their own): a
    // second brings the first to the front and goes, before it opens the log (the first's); a mod
    // it was asked to show (`--mod`), left for the first to show.
    let single = args.config.is_none() && !unattended;
    if single && let Some(id) = &args.show_mod {
        instance::ask_for_mod(id);
    }
    let instance = match single.then(instance::claim) {
        Some(instance::Claim::Another) => return,
        Some(instance::Claim::First(instance)) => {
            // (The first: it shows the mod itself.)
            let _ = instance::take_mod_request();
            Some(instance)
        }
        None => None,
    };
    let show_mod = args.show_mod.clone();
    // The settings folder kept under the game's old name moves to the new one first (unless
    // another settings file is named).
    let moved = if args.config.is_none() { ta_config::adopt_old_folder() } else { None };
    // Release builds have no console: the log goes to logs\launcher.log beside the settings
    // (next to the game's log), and a crash to logs\launcher-crash.txt with a box saying so.
    let config_path = args.config.clone().unwrap_or_else(ta_config::config_path);
    let config_path = std::path::absolute(&config_path).unwrap_or(config_path);
    let log_dir = config_path.parent().map(|d| d.join("logs")).unwrap_or_else(|| PathBuf::from("logs"));
    let mut logger = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if let Ok(file) = std::fs::create_dir_all(&log_dir).and_then(|_| std::fs::File::create(log_dir.join("launcher.log"))) {
        logger.target(env_logger::Target::Pipe(Box::new(Tee(file))));
    }
    logger.init();
    log::info!("TA Reborn launcher {}", version_label());
    if let Some(note) = moved {
        log::info!("{note}");
    }
    let crash_path = log_dir.join("launcher-crash.txt");
    // (A driver's own fault, which no panic hook sees: named in the same file.)
    native_crash::report_native_crashes(crash_path.clone(), unattended);
    std::panic::set_hook(Box::new(move |info| {
        // (A damaged music file, caught where it was read: it is skipped, nothing crashed.)
        if ta_audio::guarded_panic() {
            log::warn!("a music file could not be read: {info}");
            return;
        }
        let backtrace = std::backtrace::Backtrace::force_capture();
        // Only the window's thread going down takes the launcher with it. A worker's panic (a
        // preview the graphics driver could not draw, a mod or a picture that could not be read)
        // is caught where it ran, or ends that work alone, which the page says: logged, not a
        // crash to report in a box that would say the launcher crashed while it runs on.
        if !crashes_launcher(std::thread::current().name()) {
            log::error!("a background task stopped: {info}\n\n{backtrace:#}");
            return;
        }
        let report = format!("TA Reborn launcher {} crashed: {info}\n\n{backtrace:#}", version_label());
        log::error!("{report}");
        let _ = std::fs::write(&crash_path, &report);
        if !unattended {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("TA Reborn")
                .set_description(format!("The launcher crashed: {info}\n\nThe details were saved to {}.", crash_path.display()))
                .show();
        }
    }));

    // The window as the player left it last time (a size of their own on the command line
    // aside).
    let active_path = config_path.parent().and_then(|root| {
        let store = ta_config::profiles::ProfileStore::open(root).ok()?;
        let active = store.active().ok()??;
        store.config_path(&active.id).ok()
    }).unwrap_or_else(|| config_path.clone());
    let kept = ta_config::Config::try_load_from(&active_path).map(|c| c.launcher).unwrap_or_default();
    let (least_w, least_h) = ta_config::limits::LAUNCHER_POINTS;
    let size = args.window_size.or(kept.window.filter(|size| !unattended && valid_size(*size))).map_or([1120.0, 720.0], |[w, h]| [w.max(least_w), h.max(least_h)]);
    let maximized = kept.maximized && args.window_size.is_none() && !unattended;
    // Inside gamescope (SteamOS's Game Mode), which scales what it shows to fill the screen, the
    // window fills it itself, pixel for pixel (Steam closes it there). Never kept: on the desktop
    // it is the window it was.
    let fullscreen = ta_config::hardware::gamescope() && args.window_size.is_none() && !unattended;
    let show = args.show.clone();
    let film = match args.film.as_deref().map(film::Film::open).transpose() {
        Ok(film) => film,
        Err(e) => {
            log::error!("--film: {e:#}");
            std::process::exit(2);
        }
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("TA Reborn")
        .with_app_id("ta-reborn-launcher")
        .with_inner_size(size)
        .with_maximized(maximized)
        .with_fullscreen(fullscreen)
        .with_min_inner_size([least_w, least_h])
        .with_icon(icon::window_icon());
    if let Some(film) = &film {
        // (No frame round it: the film's size is the window's inside, as big as a screen.)
        let (size, at) = film.window();
        viewport = viewport.with_inner_size(size).with_decorations(false).with_resizable(false);
        if let Some(at) = at {
            viewport = viewport.with_position(at);
        }
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        // (Paced by the launcher instead: `app::MAX_FPS`.)
        glow_options: eframe::egui_glow::GlowConfiguration { vsync: false, ..Default::default() },
        centered: film.as_ref().is_none_or(|f| f.window().1.is_none()),
        ..Default::default()
    };
    let result = eframe::run_native(
        "TA Reborn Launcher",
        options,
        Box::new(move |cc| {
            let mut app = app::LauncherApp::new(cc, args);
            app.pace_frames();
            if let Some(film) = film {
                app.film(film);
            }
            if let Some(instance) = instance {
                app.instance(instance, &cc.egui_ctx);
            }
            if let Some(what) = show.as_deref() {
                app.show(what);
            }
            if let Some(id) = show_mod.as_deref() {
                app.open_mod(id);
            }
            Ok(Box::new(app))
        }),
    );
    if let Err(e) = result {
        // Release builds have no console, so report start-up failures in a message box.
        log::error!("launcher failed: {e}");
        if !unattended {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("TA Reborn")
                .set_description(format!(
                    "The launcher window could not be created:\n\n{e}\n\nUpdating your graphics driver usually fixes this \
                     (the launcher needs OpenGL 2.0 or newer). You can also start ta-reborn.exe directly."
                ))
                .show();
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A worker's panic (caught where it ran: a skin preview, a mod's check, a picture) is not
    /// reported as the launcher crashing; the window's thread going down is.
    #[test]
    fn only_the_window_threads_panic_is_a_crash_to_report() {
        assert!(crashes_launcher(Some("main")));
        assert!(!crashes_launcher(None));
        let worker = std::thread::Builder::new().name("map library".into())
            .spawn(|| crashes_launcher(std::thread::current().name())).unwrap();
        assert!(!worker.join().unwrap(), "a named worker");
        let unnamed = std::thread::spawn(|| crashes_launcher(std::thread::current().name()));
        assert!(!unnamed.join().unwrap(), "an unnamed worker (the skin preview's)");
    }

    #[test]
    fn window_dimensions_reject_invalid_gpu_surface_sizes() {
        for value in ["NaNx720", "1120xinf", "-1x600", "0x600", "999999x720"] {
            assert!(parse_size(value).is_err(), "{value}");
        }
        assert_eq!(parse_size("1120x720").unwrap(), [1120.0, 720.0]);
    }
}
