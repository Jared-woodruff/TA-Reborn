//! The mods a battle is played with, as its saved games and replays keep them (record version
//! 46: `ta_config::records::RecordMod`, in load order, each with the digest of its files), and
//! what the mods of now make of a record's (`ta_config::records::mods`): what the saved games and
//! replays page says of each, and what is asked as one is opened (the menu's prompt). Mods mount
//! as the game starts, so a record's are turned on by starting again ([`restart`]); one not
//! installed is got in the launcher ([`open_launcher_on`]).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use ta_config::records::RecordMod;
use ta_config::records::mods::ModCheck;
use ta_vfs::mods::ModInfo;

/// The mods mounted, in load order, and their digests: worked out on a thread of their own from
/// when they are mounted (a few hundred megabytes of mods take a fraction of a second), and waited
/// for by a record written before they are done.
#[derive(Clone, Default)]
pub struct InPlay {
    mods: Arc<Vec<ModInfo>>,
    digests: Arc<OnceLock<Vec<[u8; 16]>>>,
    worker: Arc<Mutex<Option<std::thread::JoinHandle<Vec<[u8; 16]>>>>>,
}

impl InPlay {
    pub fn new(mods: Vec<ModInfo>) -> InPlay {
        let mods = Arc::new(mods);
        let list = mods.clone();
        let worker = if mods.is_empty() {
            None
        } else {
            std::thread::Builder::new().name("mod digests".into()).spawn(move || digests(&list)).ok()
        };
        InPlay { mods, digests: Default::default(), worker: Arc::new(Mutex::new(worker)) }
    }

    /// What a record made now keeps of them (waiting for their digests if they are not done).
    pub fn record(&self) -> Vec<RecordMod> {
        let digests = self.digests.get_or_init(|| {
            let worker = self.worker.lock().ok().and_then(|mut w| w.take());
            worker.and_then(|w| w.join().ok()).unwrap_or_else(|| digests(&self.mods))
        });
        self.mods.iter().zip(digests).map(|(m, digest)| as_record(m, *digest)).collect()
    }

    /// The same, now: their digests if they are done, unknown if not (for a list, which must not
    /// wait).
    pub fn known(&self) -> Vec<RecordMod> {
        if self.digests.get().is_none() {
            let done = self.worker.lock().map_or(true, |w| w.as_ref().is_none_or(|w| w.is_finished()));
            if !done {
                return self.mods.iter().map(|m| as_record(m, [0; 16])).collect();
            }
        }
        self.record()
    }
}

fn as_record(m: &ModInfo, digest: [u8; 16]) -> RecordMod {
    RecordMod { id: m.manifest.id.clone(), name: m.manifest.name.clone(), version: m.manifest.version.clone(), digest }
}

fn digests(mods: &[ModInfo]) -> Vec<[u8; 16]> {
    let start = std::time::Instant::now();
    let digests = mods.iter().map(|m| ta_vfs::mods::content_digest(m).unwrap_or_else(|e| {
        log::warn!("mod {}: its files could not be read for its digest: {e}", m.manifest.id);
        [0; 16]
    })).collect();
    if !mods.is_empty() {
        log::info!("mods' digests in {:.0} ms", start.elapsed().as_secs_f64() * 1000.0);
    }
    digests
}

/// Every mod installed (on or off), as a record names them (their digests unknown: those off are
/// told by their version).
pub fn installed(settings: &ta_config::Config) -> Vec<RecordMod> {
    let app_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    let folders = ta_vfs::mods::default_folders(app_dir.as_deref(), &ta_config::config_dir(), settings.mods.extra_dir.as_deref());
    ta_vfs::mods::scan_folders(&folders).0.iter().map(|m| as_record(m, [0; 16])).collect()
}

/// What a saved game or replay is to the game of now.
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    /// Its battle's data is today's (its data hash): it plays back as it was.
    pub data: bool,
    /// Made by this simulation (a saved game can carry on only then; a replay of another plays
    /// approximately).
    pub simulation: bool,
    /// Its mods against today's (`None`: made before records kept their mods).
    pub mods: Option<ModCheck>,
}

impl Verdict {
    /// A saved game's or replay's facts against `assets`' data and mods, and the mods `installed`.
    pub fn of(version: u32, metadata: Option<&ta_config::records::RecordMetadata>, data_hash: u64, setup: &crate::setup::GameSetup, mods: Option<&[RecordMod]>, assets: &crate::setup::Assets, installed: &[RecordMod]) -> Verdict {
        let recorded = metadata.map_or(data_hash, |m| m.match_data_hash);
        Verdict {
            data: recorded == assets.expected_record_hash(version, setup),
            simulation: ta_config::records::simulation_compatible(version, metadata),
            mods: mods.map(|mods| ta_config::records::mods::check(mods, &assets.mods.known(), installed)),
        }
    }

    /// Opened as it is, nothing asked: its data is today's (a replay of another simulation is
    /// watched approximately, as before).
    pub fn opens(&self) -> bool {
        self.data
    }
}

/// The arguments the game starts again with to open `record` (`replay`: watch it): those that
/// choose the install, the profile and a capture of the first run, the rest left behind (what it
/// was opened with, a battle to start...).
pub fn restart_args(original: impl IntoIterator<Item = OsString>, record: &Path, replay: bool) -> Vec<OsString> {
    const KEPT: &[&str] = &["--profile", "--game-dir", "--screenshot", "--frames"];
    let mut args: Vec<OsString> = Vec::new();
    let mut original = original.into_iter().skip(1);
    while let Some(arg) = original.next() {
        let text = arg.to_string_lossy().to_string();
        let (name, value) = match text.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name.to_string(), Some(OsString::from(value))),
            _ => (text.clone(), None),
        };
        if KEPT.contains(&name.as_str()) {
            args.push(name.into());
            if let Some(value) = value.or_else(|| original.next()) {
                args.push(value);
            }
        }
    }
    args.push(if replay { "--replay" } else { "--load" }.into());
    args.push(record.as_os_str().to_os_string());
    args
}

/// The interface's scripted input of a capture (left out of a game started again: it was this
/// run's).
const SCRIPTED_INPUT: &[&str] = &["TA_UI_CLICKS", "TA_UI_TEXT", "TA_UI_KEYS", "TA_UI_PAD", "TA_UI_HOVER", "TA_UI_MOUSE"];

/// Starts the game again (it opens `record`; `replay`: to watch); the caller then closes this one.
pub fn restart(record: &Path, replay: bool) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut command = std::process::Command::new(exe);
    command.args(restart_args(std::env::args_os(), record, replay));
    for name in SCRIPTED_INPUT {
        command.env_remove(name);
    }
    no_console(&mut command);
    // (The launcher that started this game follows the new one: `ta_config::running`.)
    let root = ta_config::running::profile_root();
    ta_config::running::mark_restart(&root);
    command.spawn().map(|_| ()).inspect_err(|_| {
        ta_config::running::take_restart(&root);
    })
}

/// The launcher beside the game, on its Mods page at the mod `id` (to get it there): started, or,
/// where one runs already, asked to show it.
pub fn open_launcher_on(id: &str) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let name = if cfg!(windows) { "ta-launcher.exe" } else { "ta-launcher" };
    let launcher: PathBuf = exe.parent().map(|dir| dir.join(name)).unwrap_or_else(|| PathBuf::from(name));
    if !launcher.is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, launcher.display().to_string()));
    }
    let mut command = std::process::Command::new(launcher);
    command.args(["--mod", id]);
    if let Some(dir) = exe.parent() {
        command.current_dir(dir);
    }
    command.spawn().map(|_| ())
}

/// (The game is a console program: started from a game that has none, it would open one.)
fn no_console(command: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_again_keeps_the_install_and_profile_and_opens_the_record() {
        let original = ["ta-reborn", "--game-dir", "C:\\TA", "--profile=abc", "--map", "Coast To Coast", "--load", "old.tasav", "--skip-intro"].map(OsString::from);
        let args = restart_args(original, Path::new("D:\\saves\\new.tasav"), false);
        assert_eq!(args, ["--game-dir", "C:\\TA", "--profile", "abc", "--load", "D:\\saves\\new.tasav"].map(OsString::from));
        let replay = restart_args(["ta-reborn".into(), "x.tarep".into()], Path::new("x.tarep"), true);
        assert_eq!(replay, ["--replay", "x.tarep"].map(OsString::from), "what it was opened with is left behind");
    }

    #[test]
    fn records_made_now_name_the_mods_in_play_in_their_order() {
        let root = std::env::temp_dir().join(format!("ta-in-play-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut mods = Vec::new();
        for (id, version) in [("b", "2.0"), ("a", "")] {
            let dir = root.join(id);
            std::fs::create_dir_all(dir.join("units")).unwrap();
            std::fs::write(dir.join("mod.toml"), format!("id = \"{id}\"\nname = \"Mod {id}\"\nversion = \"{version}\"\n")).unwrap();
            std::fs::write(dir.join("units/x.fbi"), id).unwrap();
            mods.push(ta_vfs::mods::read_mod(&dir).unwrap().unwrap());
        }
        let in_play = InPlay::new(mods);
        let record = in_play.record();
        assert_eq!(record.iter().map(|m| (m.id.as_str(), m.name.as_str(), m.version.as_str())).collect::<Vec<_>>(), [("b", "Mod b", "2.0"), ("a", "Mod a", "")]);
        assert!(record.iter().all(RecordMod::digest_known) && record[0].digest != record[1].digest);
        assert_eq!(in_play.known(), record);
        assert!(InPlay::default().record().is_empty(), "no mods: none named");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
