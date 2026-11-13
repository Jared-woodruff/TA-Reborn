//! Whether a game is running, for a launcher that did not start it. The game holds a file of the
//! folder of the settings it plays with locked for as long as it runs (`game.lock`, in the
//! launcher's folder: `TA_PROFILE_ROOT`); a launcher that finds it held knows a game is running,
//! one opened while a game ran, or one whose game started itself again with a battle's mods. It
//! then leaves the mods, the maps and the profiles as they are, and starts no other game. A game
//! starting itself again leaves a note for the launcher that started it (`game.restart`): its own
//! game ends, but the game goes on.
//!
//! The system lets go of the lock when the process ends, however it ends.

use std::path::{Path, PathBuf};

const LOCK: &str = "game.lock";
const RESTART: &str = "game.restart";

/// The folder of the settings the game plays with: the launcher's (`TA_PROFILE_ROOT`), else the
/// usual one ([`crate::config_dir`]).
pub fn profile_root() -> PathBuf {
    std::env::var_os("TA_PROFILE_ROOT").map(PathBuf::from).unwrap_or_else(crate::config_dir)
}

/// Holds the lock for as long as this process runs. A thread waits for it: a game started again by
/// the one before has it once that one has ended.
pub fn hold(root: &Path) {
    let path = root.join(LOCK);
    let _ = std::thread::Builder::new().name("game lock".into()).spawn(move || {
        let file = match std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path) {
            Ok(file) => file,
            Err(e) => {
                log::warn!("{}: {e}", path.display());
                return;
            }
        };
        match file.lock() {
            // (Kept open, and so held, until the process ends.)
            Ok(()) => std::mem::forget(file),
            Err(e) => log::warn!("{}: {e}", path.display()),
        }
    });
}

/// Whether a game holds the lock now (none: no game, or the file never made).
pub fn held(root: &Path) -> bool {
    let Ok(file) = std::fs::OpenOptions::new().write(true).open(root.join(LOCK)) else { return false };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(std::fs::TryLockError::WouldBlock) => true,
        Err(std::fs::TryLockError::Error(_)) => false,
    }
}

/// Leaves the note that this game is starting itself again (see the module).
pub fn mark_restart(root: &Path) {
    if let Err(e) = std::fs::write(root.join(RESTART), b"") {
        log::warn!("{}: {e}", root.join(RESTART).display());
    }
}

/// Whether the game that just ended started itself again: takes the note away. (A note older
/// than a minute is another's, a game started without the launcher: not this one's.)
pub fn take_restart(root: &Path) -> bool {
    let path = root.join(RESTART);
    let recent = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|at| at.elapsed().ok()).is_some_and(|age| age.as_secs() < 60);
    std::fs::remove_file(&path).is_ok() && recent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_lock_is_seen_and_a_restart_note_is_taken_once() {
        let root = std::env::temp_dir().join(format!("ta_running_{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(!held(&root), "no file: no game");
        hold(&root);
        let start = std::time::Instant::now();
        while !held(&root) {
            assert!(start.elapsed() < std::time::Duration::from_secs(10), "the lock is taken");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!take_restart(&root));
        mark_restart(&root);
        assert!(take_restart(&root) && !take_restart(&root), "the note is taken once");
        // (The lock stays held, by this process, until it ends: the folder is left.)
    }
}
