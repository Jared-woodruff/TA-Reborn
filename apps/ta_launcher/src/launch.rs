//! Starting `ta-reborn.exe` (which sits next to the launcher) and tracking the process.

use std::ffi::OsString;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

/// File name of the game executable.
pub const GAME_EXE: &str = if cfg!(windows) { "ta-reborn.exe" } else { "ta-reborn" };

/// Folder containing the launcher executable (and the game, mods and assets beside it).
pub fn app_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// Where the game executable is expected.
pub fn game_exe() -> Option<PathBuf> {
    app_dir().map(|dir| dir.join(GAME_EXE))
}

/// The game's crash report, when it crashed: where, and what went wrong (its first line).
#[derive(Clone, Debug, PartialEq)]
pub struct Crash {
    pub report: PathBuf,
    pub message: String,
}

/// Lifecycle of the game process.
#[derive(Debug, Default)]
pub enum GameState {
    #[default]
    Idle,
    Running {
        child: Child,
        since: Instant,
        /// When it started, by the clock (to tell a crash report written since).
        started: SystemTime,
        log: Option<PathBuf>,
    },
    Exited {
        code: Option<i32>,
        success: bool,
        ran_for: Duration,
        log: Option<PathBuf>,
        crash: Option<Crash>,
    },
    Failed(String),
}

impl GameState {
    pub fn is_running(&self) -> bool {
        matches!(self, GameState::Running { .. })
    }

    /// Ends the game at once (the player asked: it stopped responding); the next [`poll`]
    /// finds it gone.
    pub fn end(&mut self) {
        if let GameState::Running { child, .. } = self
            && let Err(e) = child.kill()
        {
            log::warn!("could not end the game: {e}");
        }
    }

    /// The last game crashed, stopped with an error or failed to start (the Play page says
    /// what happened).
    pub fn went_wrong(&self) -> bool {
        matches!(self, GameState::Failed(_) | GameState::Exited { success: false, .. })
    }

    /// How long the game has been running.
    pub fn running_for(&self) -> Option<Duration> {
        match self {
            GameState::Running { since, .. } => Some(since.elapsed()),
            _ => None,
        }
    }
}

/// Starts the game for the install at `game_dir`, with `extra` arguments (a saved game to
/// load...). Its console output goes to `<log_dir>/game.log` (the launcher has no console to
/// show it in).
pub fn launch(game_dir: &Path, log_dir: &Path, profile_root: &Path, profile_id: Option<&str>, extra: &[OsString]) -> GameState {
    let Some(exe) = game_exe() else {
        return GameState::Failed(ta_ui::i18n::t("Could not tell where the launcher is installed.").to_string());
    };
    if !exe.is_file() {
        return GameState::Failed(ta_ui::i18n::tf(
            "{exe} was not found next to the launcher (expected {path}). Reinstall TA Reborn, or build it with `cargo build -p ta_game`.",
            &[("exe", &GAME_EXE), ("path", &exe.display())],
        ));
    }

    let mut command = match game_command(&exe, game_dir, profile_root, profile_id, extra) {
        Ok(command) => command,
        Err(e) => return GameState::Failed(ta_ui::i18n::tf("Could not start {exe}: {error}", &[("exe", &exe.display()), ("error", &e)])),
    };
    let log = match open_log(log_dir) {
        Some((file, path)) => match file.try_clone() {
            Ok(copy) => {
                command.stdout(copy).stderr(file);
                Some(path)
            }
            Err(_) => {
                command.stdout(Stdio::null()).stderr(file);
                Some(path)
            }
        },
        None => {
            command.stdout(Stdio::null()).stderr(Stdio::null());
            None
        }
    };
    #[cfg(windows)]
    {
        // The game is a console-subsystem program; don't flash a console window.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    log::info!("launching {} --game-dir {} {}", exe.display(), game_dir.display(), extra.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
    match command.spawn() {
        Ok(child) => GameState::Running { child, since: Instant::now(), started: SystemTime::now(), log },
        Err(e) => GameState::Failed(ta_ui::i18n::tf("Could not start {exe}: {error}", &[("exe", &exe.display()), ("error", &e)])),
    }
}

/// Resolve launcher-relative paths before changing the child's working directory to the
/// installed executable folder, which may be different from where the launcher started.
fn game_command(exe: &Path, game_dir: &Path, profile_root: &Path, profile_id: Option<&str>, extra: &[OsString]) -> std::io::Result<Command> {
    let mut command = Command::new(exe);
    command.arg("--game-dir").arg(std::path::absolute(game_dir)?).args(extra).stdin(Stdio::null());
    // We capture the game's output into game.log; tell it not to write its own.
    command.env("TA_LOG_REDIRECTED", "1");
    command.env("TA_PROFILE_ROOT", std::path::absolute(profile_root)?);
    if let Some(id) = profile_id { command.env("TA_PROFILE_ID", id); }
    else { command.env_remove("TA_PROFILE_ID"); }
    if let Some(dir) = exe.parent() { command.current_dir(dir); }
    Ok(command)
}

fn open_log(dir: &Path) -> Option<(File, PathBuf)> {
    std::fs::create_dir_all(dir).ok()?;
    let path = dir.join("game.log");
    match File::create(&path) {
        Ok(file) => Some((file, path)),
        Err(e) => {
            log::warn!("cannot create {}: {e}", path.display());
            None
        }
    }
}

/// Updates `state` if the game has exited (its crash report looked for in `log_dir` when it
/// failed). Returns `true` while it is still running.
pub fn poll(state: &mut GameState, log_dir: &Path) -> bool {
    let GameState::Running { child, since, started, log } = state else { return false };
    let next = match child.try_wait() {
        Ok(None) => return true,
        Ok(Some(status)) => {
            log::info!("game exited: {status}");
            let crash = if status.success() { None } else { crash_report(log_dir, *started) };
            GameState::Exited { code: status.code(), success: status.success(), ran_for: since.elapsed(), log: log.take(), crash }
        }
        Err(e) => GameState::Failed(ta_ui::i18n::tf("Lost track of the game process: {error}", &[("error", &e)])),
    };
    *state = next;
    false
}

/// The game's crash report (`crash.txt` in the logs), if it was written since `since`: its
/// message, from "crashed: " on its first line to the line naming the thread, on one line.
pub fn crash_report(log_dir: &Path, since: SystemTime) -> Option<Crash> {
    use std::io::Read;
    // This preview is read on the launcher thread after the game exits. Keep an
    // oversized/damaged log out of the UI; Open report still exposes the full file.
    const MAX_PREVIEW_BYTES: u64 = 16 << 10;
    const MAX_MESSAGE_CHARS: usize = 2048;
    let report = log_dir.join("crash.txt");
    let metadata = std::fs::metadata(&report).ok()?;
    if !metadata.is_file() { return None; }
    let written = metadata.modified().ok()?;
    // (A second's grace: file times are coarse on some file systems.)
    if since.duration_since(written).is_ok_and(|age| age > Duration::from_secs(1)) {
        return None;
    }
    let mut bytes = Vec::new();
    File::open(&report).ok()?.take(MAX_PREVIEW_BYTES).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let from = text.split_once("crashed: ").map_or(text.as_ref(), |(_, m)| m);
    let message = from.lines().take_while(|l| !l.starts_with("thread:") && !l.trim().is_empty()).map(str::trim).collect::<Vec<_>>().join(" ");
    let mut chars = message.chars();
    let mut message: String = chars.by_ref().take(MAX_MESSAGE_CHARS).collect();
    if chars.next().is_some() { message.push('…'); }
    Some(Crash { report, message })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_exe_is_next_to_the_running_binary() {
        let exe = game_exe().unwrap();
        assert_eq!(exe.parent(), std::env::current_exe().unwrap().parent());
        assert_eq!(exe.file_name().unwrap(), GAME_EXE);
    }

    #[test]
    fn missing_game_executable_is_reported() {
        // Test binaries live in target/<profile>/deps, where no game executable is built.
        let state = launch(Path::new(r"C:\GOG Games\Total Annihilation"), &std::env::temp_dir(), &std::env::temp_dir(), None, &[]);
        match state {
            GameState::Failed(message) => assert!(message.contains(GAME_EXE), "{message}"),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn child_paths_stay_anchored_when_its_working_directory_changes() {
        let exe = std::env::temp_dir().join("Reborn install").join(GAME_EXE);
        let extra = [OsString::from("--load"), OsString::from("save with spaces.tasav")];
        let command = game_command(&exe, Path::new("original game"), Path::new("player data"), Some("selected-profile"), &extra).unwrap();
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args[1], std::path::absolute("original game").unwrap());
        assert_eq!(&args[2..], &[extra[0].as_os_str(), extra[1].as_os_str()]);
        assert_eq!(command.get_current_dir(), exe.parent());
        assert_eq!(command.get_envs().find(|(key, _)| *key == "TA_PROFILE_ROOT").unwrap().1.unwrap(), std::path::absolute("player data").unwrap());
        assert_eq!(command.get_envs().find(|(key, _)| *key == "TA_PROFILE_ID").unwrap().1.unwrap(), "selected-profile");
        let command = game_command(&exe, Path::new("original game"), Path::new("player data"), None, &[]).unwrap();
        assert_eq!(command.get_envs().find(|(key, _)| *key == "TA_PROFILE_ID").unwrap().1, None, "no inherited profile leaks into a launch without one");
    }

    #[test]
    fn idle_state_is_not_running() {
        let mut state = GameState::Idle;
        assert!(!poll(&mut state, &std::env::temp_dir()));
        assert!(matches!(state, GameState::Idle));
    }

    /// A crash report written since the game started is read; an older one is not this crash.
    #[test]
    fn a_crash_report_is_read_when_it_is_new() {
        let dir = std::env::temp_dir().join(format!("ta_launcher_crash_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let started = SystemTime::now();
        std::fs::write(dir.join("crash.txt"), "TA Reborn 2.1.0 (abc) crashed: panicked at src/app.rs:10:5:\nindex out of bounds\nthread: main\n").unwrap();
        let crash = crash_report(&dir, started).expect("read");
        assert_eq!(crash.message, "panicked at src/app.rs:10:5: index out of bounds");
        assert_eq!(crash.report, dir.join("crash.txt"));
        assert!(crash_report(&dir, started + Duration::from_secs(60)).is_none(), "written before the game started");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn crash_preview_is_bounded_without_truncating_the_original_report() {
        let dir = std::env::temp_dir().join(format!("ta_launcher_crash_bounded_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let report = dir.join("crash.txt");
        let text = format!("TA Reborn crashed: {}\nthread: main\n", "界".repeat(1 << 18));
        std::fs::write(&report, text.as_bytes()).unwrap();
        let preview = crash_report(&dir, SystemTime::now()).unwrap();
        assert_eq!(preview.message.chars().count(), 2049);
        assert!(preview.message.ends_with('…'));
        assert!(preview.message.chars().take(2048).all(|ch| ch == '界'), "truncate Unicode by characters");
        assert_eq!(std::fs::metadata(&report).unwrap().len(), text.len() as u64, "the full report is still available to open");
        std::fs::remove_file(&report).unwrap();
        std::fs::create_dir(&report).unwrap();
        assert!(crash_report(&dir, SystemTime::now()).is_none(), "non-files are not read as crash previews");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
