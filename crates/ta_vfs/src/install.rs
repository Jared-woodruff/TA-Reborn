//! Locating and validating the user's original Total Annihilation install.
//!
//! TA Reborn ships no original game data. At first run we locate a legitimately
//! installed copy — "Total Annihilation: Commander Pack" from GOG or from Steam, which ship the
//! same data files — and read its archives in place. Detection order:
//!
//! 1. an explicit path (command line / config),
//! 2. the `TA_REBORN_GAME_DIR` environment variable,
//! 3. the GOG registry key written by both the offline installer and GOG Galaxy; on Linux and
//!    macOS, Heroic's record of its GOG installs (`gog_store/installed.json`), then the folders a
//!    GOG installer or TA's CD makes in Wine, Lutris, Bottles, Heroic and CrossOver prefixes,
//!    and Heroic's own folders,
//! 4. the Steam release: its uninstall key, then every Steam library (the folder each one's
//!    `appmanifest_298030.acf` names),
//! 5. well-known default install folders (GOG's and Steam's); on Linux, the folders a GOG
//!    installer or TA's CD makes in the Proton prefixes of every Steam library (such an
//!    installer run through Steam as a non-Steam game: a Steam Deck's way to install it).
//!
//! Which store a copy came from ([`Store`]), and its build, are read from its files: GOG's
//! `goggame-1207658880.info`, or the Steam library's app manifest.

use std::path::{Path, PathBuf};

use thiserror::Error;

/// GOG product id of "Total Annihilation: Commander Pack".
pub const GOG_GAME_ID: &str = "1207658880";
/// Store page shown when no install is found.
pub const GOG_STORE_URL: &str = "https://www.gog.com/en/game/total_annihilation_commander_pack";
/// `goggalaxy://` deep link that opens the game in GOG Galaxy (installs it if owned).
pub const GOG_GALAXY_URL: &str = "goggalaxy://openGameView/1207658880";
/// Steam app id of Total Annihilation: Commander Pack, which ships the same data files.
pub const STEAM_APP_ID: u32 = 298030;
/// Its Steam store page.
pub const STEAM_STORE_URL: &str = "https://store.steampowered.com/app/298030/";
/// `steam://` link that shows the game in the Steam client's library (installs it if owned).
pub const STEAM_LIBRARY_URL: &str = "steam://nav/games/details/298030";
/// The folder Steam installs it in, unless its app manifest says otherwise.
const STEAM_DIR: &str = "Total Annihilation";
/// The environment variable naming the original game's folder, and its name up to 1.9 (when
/// the game was called TA Remastered), still read.
pub const ENV_GAME_DIR: &str = "TA_REBORN_GAME_DIR";
pub const OLD_ENV_GAME_DIR: &str = "TA_REMASTERED_GAME_DIR";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallSource {
    Explicit,
    Environment,
    GogRegistry,
    Steam,
    CommonLocation,
}

impl std::fmt::Display for InstallSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            InstallSource::Explicit => "configured path",
            InstallSource::Environment => ENV_GAME_DIR,
            InstallSource::GogRegistry => "GOG registry",
            InstallSource::Steam => "Steam library",
            InstallSource::CommonLocation => "default location",
        })
    }
}

/// Which parts of the TA family are present.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Content {
    pub base_game: bool,
    pub campaigns: bool,
    pub core_contingency: bool,
    pub battle_tactics: bool,
    pub patch_3_1: bool,
    pub music: bool,
    pub movies: bool,
}

/// The store a copy of the game came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Gog,
    Steam,
    /// Neither says it is theirs (a CD install, a copied folder).
    Unknown,
}

impl Store {
    /// The store's name, as it calls itself.
    pub fn name(self) -> Option<&'static str> {
        match self {
            Store::Gog => Some("GOG"),
            Store::Steam => Some("Steam"),
            Store::Unknown => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TaInstall {
    pub root: PathBuf,
    pub source: InstallSource,
    pub content: Content,
    /// The store its files say it came from, and the store's build of it, when they say.
    pub store: Store,
    pub build: Option<String>,
}

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("no Total Annihilation installation was found (searched {} locations)", searched.len())]
    NotFound { searched: Vec<PathBuf> },
    #[error("{} is not a usable Total Annihilation install: missing {}", path.display(), missing.join(", "))]
    Invalid { path: PathBuf, missing: Vec<String> },
}

/// Files without which the game cannot start.
pub const REQUIRED_FILES: &[&str] = &["totala1.hpi", "totala2.hpi"];

/// The archives the original game's releases install (file names, in lower case): the Commander
/// Pack's thirty (GOG's and Steam's: the game, Core Contingency, Battle Tactics, the 3.1 patch,
/// Cavedog's six post-release units and its bonus maps), and the free Core Contingency and Battle
/// Tactics unit packs of players without the add-ons. Whatever an install holds, a mod's archive of
/// an install's name replaces it ([`crate::mods::replaced_archives`]); one of these names says so
/// in the mod's digest wherever it is installed ([`crate::mods::content_digest`]).
pub const ARCHIVES: &[&str] = &[
    "totala1.hpi", "totala2.hpi", "totala3.hpi", "totala4.hpi", "worlds.hpi",
    "tactics1.hpi", "tactics2.hpi", "tactics3.hpi", "tactics4.hpi", "tactics5.hpi", "tactics6.hpi", "tactics7.hpi", "tactics8.hpi",
    "afark.ufo", "aflea.ufo", "ascarab.ufo", "cornecro.ufo", "cormabm.ufo", "corplas.ufo",
    "cometctr.ufo", "evadrivd.ufo", "example.ufo", "mndsmars.ufo", "floggen.ufo",
    "btdata.ccx", "btmaps.ccx", "ccdata.ccx", "ccmaps.ccx", "ccmiss.ccx", "rev31.gp3",
    "ccunits.ccx", "ccfeatures.ccx", "btunits.ccx",
];

/// Finds a file in `dir` ignoring case (needed on case-sensitive file systems).
pub fn find_file_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name) && e.path().is_file())
        .map(|e| e.path())
}

/// Finds a folder in `dir` ignoring case.
pub fn find_dir_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .find(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.file_name().to_string_lossy().eq_ignore_ascii_case(name))
        .map(|e| e.path())
}

fn dir_has_ext(dir: &Path, ext: &str) -> bool {
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.filter_map(|e| e.ok()).any(|e| {
            e.path().extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case(ext)) && e.path().is_file()
        })
    })
}

/// Checks a candidate directory. Returns detected content, or the missing required files.
pub fn validate(root: &Path) -> std::result::Result<Content, Vec<String>> {
    let missing: Vec<String> =
        REQUIRED_FILES.iter().filter(|f| find_file_ci(root, f).is_none()).map(|f| f.to_string()).collect();
    if !missing.is_empty() {
        return Err(missing);
    }
    let has = |f: &str| find_file_ci(root, f).is_some();
    Ok(Content {
        base_game: true,
        campaigns: has("totala4.hpi"),
        core_contingency: has("ccdata.ccx") && has("ccmaps.ccx"),
        battle_tactics: has("btdata.ccx"),
        patch_3_1: has("rev31.gp3"),
        music: find_dir_ci(root, "music").is_some_and(|d| dir_has_ext(&d, "mp3") || dir_has_ext(&d, "ogg")),
        movies: find_dir_ci(root, "data").is_some_and(|d| dir_has_ext(&d, "zrb")),
    })
}

/// All candidate install locations, in detection order (may contain duplicates/non-existent).
pub fn candidates(explicit: Option<&Path>) -> Vec<(PathBuf, InstallSource)> {
    let mut out = Vec::new();
    if let Some(p) = explicit {
        out.push((p.to_path_buf(), InstallSource::Explicit));
    }
    if let Some(p) = std::env::var_os(ENV_GAME_DIR).or_else(|| std::env::var_os(OLD_ENV_GAME_DIR)) {
        out.push((PathBuf::from(p), InstallSource::Environment));
    }
    for p in gog_registry_paths() {
        out.push((p, InstallSource::GogRegistry));
    }
    for p in steam_paths() {
        out.push((p, InstallSource::Steam));
    }
    // (The installer's default folders, though Steam's: the files tell GOG's copy from a CD's.)
    for p in proton_prefix_paths() {
        out.push((p, InstallSource::CommonLocation));
    }
    for drive in if cfg!(windows) { ["C", "D", "E", "F"].as_slice() } else { [].as_slice() } {
        for sub in [
            r"GOG Games\Total Annihilation",
            r"Program Files (x86)\GOG Galaxy\Games\Total Annihilation",
            r"Program Files (x86)\GOG.com\Total Annihilation",
            r"Program Files\GOG Galaxy\Games\Total Annihilation",
            r"Program Files (x86)\Steam\steamapps\common\Total Annihilation",
            r"SteamLibrary\steamapps\common\Total Annihilation",
            r"Steam\steamapps\common\Total Annihilation",
            r"Games\Total Annihilation",
            r"Cavedog\TotalA",
        ] {
            out.push((PathBuf::from(format!(r"{drive}:\{sub}")), InstallSource::CommonLocation));
        }
    }
    out
}

/// Locates the first valid install. An explicit path that is invalid is an error rather than
/// being silently skipped, so a typo in the config is reported.
pub fn discover(explicit: Option<&Path>) -> Result<TaInstall, InstallError> {
    if let Some(p) = explicit {
        return match validate(p) {
            Ok(content) => Ok(make_install(p, InstallSource::Explicit, content)),
            Err(missing) => Err(InstallError::Invalid { path: p.to_path_buf(), missing }),
        };
    }
    let mut searched = Vec::new();
    for (path, source) in candidates(None) {
        if searched.contains(&path) {
            continue;
        }
        if let Ok(content) = validate(&path) {
            return Ok(make_install(&path, source, content));
        }
        searched.push(path);
    }
    Err(InstallError::NotFound { searched })
}

fn make_install(root: &Path, source: InstallSource, content: Content) -> TaInstall {
    let (store, build) = identify(root, source);
    TaInstall { root: root.to_path_buf(), source, content, store, build }
}

/// Which store a copy came from, and its build: GOG's when its `goggame-<id>.info` is there,
/// Steam's when it stands in a Steam library that lists it (or holds Steam's `steam_appid.txt`),
/// else what the place it was found in says.
pub fn identify(root: &Path, source: InstallSource) -> (Store, Option<String>) {
    if let Some(info) = find_file_ci(root, &format!("goggame-{GOG_GAME_ID}.info")) {
        return (Store::Gog, std::fs::read_to_string(info).ok().and_then(|text| gog_build(&text)));
    }
    if let Some(manifest) = steam_manifest_for(root) {
        return (Store::Steam, std::fs::read_to_string(manifest).ok().and_then(|text| acf_value(&text, "buildid")));
    }
    match source {
        _ if find_file_ci(root, "steam_appid.txt").is_some() => (Store::Steam, None),
        InstallSource::Steam => (Store::Steam, None),
        InstallSource::GogRegistry => (Store::Gog, None),
        _ => (Store::Unknown, None),
    }
}

/// Reads the build id from GOG's `goggame-<id>.info` JSON without a JSON dependency.
fn gog_build(info: &str) -> Option<String> {
    let after = &info[info.find("\"buildId\"")? + 9..];
    let start = after.find('"')? + 1;
    let end = start + after[start..].find('"')?;
    Some(after[start..end].to_string())
}

/// The app manifest of the Steam library `root` stands in (`<library>/steamapps/common/<root>`
/// with `<library>/steamapps/appmanifest_298030.acf` naming it), if it does.
fn steam_manifest_for(root: &Path) -> Option<PathBuf> {
    let name = root.file_name()?.to_string_lossy().to_string();
    let common = root.parent()?;
    let steamapps = common.parent()?;
    if !common.file_name()?.to_string_lossy().eq_ignore_ascii_case("common") || !steamapps.file_name()?.to_string_lossy().eq_ignore_ascii_case("steamapps") {
        return None;
    }
    let manifest = find_file_ci(steamapps, &format!("appmanifest_{STEAM_APP_ID}.acf"))?;
    let text = std::fs::read_to_string(&manifest).ok()?;
    acf_value(&text, "installdir").is_none_or(|dir| dir.eq_ignore_ascii_case(&name)).then_some(manifest)
}

/// A value of Valve's key-value text (`"key"  "value"` a line): the first `key`'s.
pub fn acf_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut quoted = line.split('"').skip(1).step_by(2);
        if quoted.next()?.eq_ignore_ascii_case(key) { quoted.next().map(|v| v.replace("\\\\", "\\")) } else { None }
    })
}

/// Where Steam keeps the game in a library: the folder its app manifest names, else Steam's
/// usual one.
#[cfg(test)]
fn steam_game_dir(library: &Path) -> PathBuf {
    steam_app_dir(library, STEAM_APP_ID, STEAM_DIR)
}

/// Where Steam keeps app `app` in a library: the folder its app manifest names, else `usual`.
pub fn steam_app_dir(library: &Path, app: u32, usual: &str) -> PathBuf {
    let steamapps = library.join("steamapps");
    let named = find_file_ci(&steamapps, &format!("appmanifest_{app}.acf"))
        .and_then(|m| std::fs::read_to_string(m).ok())
        .and_then(|text| acf_value(&text, "installdir"));
    steamapps.join("common").join(named.unwrap_or_else(|| usual.to_string()))
}

/// Where Steam may keep app `app` (in its folder `usual` unless its manifest names another): its
/// uninstall key's folder (Windows), then the app's folder in every library of every Steam there
/// is (this user's, the machine's, its usual folder; off Windows the native, Flatpak and macOS
/// ones), each once.
pub fn steam_app_folders(app: u32, usual: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let push = |dir: PathBuf, out: &mut Vec<PathBuf>| {
        if !out.iter().any(|o| same_place(o, &dir)) {
            out.push(dir);
        }
    };
    #[cfg(windows)]
    {
        use winreg::HKLM;
        for key in [
            format!(r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Steam App {app}"),
            format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Steam App {app}"),
        ] {
            if let Ok(k) = HKLM.open_subkey(key) && let Ok(p) = k.get_value::<String, _>("InstallLocation") {
                push(PathBuf::from(p), &mut out);
            }
        }
    }
    for root in steam_roots() {
        for lib in steam_libraries(&root) {
            push(steam_app_dir(&lib, app, usual), &mut out);
        }
    }
    out
}

/// Every Steam there may be: this user's (where it really is), the machine's, else its usual
/// folder (Windows); the native, Flatpak and macOS ones elsewhere.
pub fn steam_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        use winreg::{HKCU, HKLM};
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Ok(p) = HKCU.open_subkey(r"Software\Valve\Steam").and_then(|k| k.get_value::<String, _>("SteamPath")) {
            roots.push(PathBuf::from(p));
        }
        for key in [r"SOFTWARE\WOW6432Node\Valve\Steam", r"SOFTWARE\Valve\Steam"] {
            if let Ok(p) = HKLM.open_subkey(key).and_then(|k| k.get_value::<String, _>("InstallPath")) {
                roots.push(PathBuf::from(p));
            }
        }
        roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
        roots
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map_or_else(Vec::new, |home| unix_steam_roots(Path::new(&home)))
    }
}

/// Whether two paths name one folder (on Windows whatever their case and slashes).
fn same_place(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        let key = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase();
        key(a) == key(b)
    } else {
        a == b
    }
}

/// The folders GOG's registry key for product `id` names (the offline installer's and Galaxy's).
#[cfg(windows)]
pub fn gog_registry_folders(id: &str) -> Vec<PathBuf> {
    use winreg::{HKCU, HKLM};
    let mut out = Vec::new();
    for (hive, key) in [
        (HKLM, format!(r"SOFTWARE\WOW6432Node\GOG.com\Games\{id}")),
        (HKLM, format!(r"SOFTWARE\GOG.com\Games\{id}")),
        (HKCU, format!(r"SOFTWARE\GOG.com\Games\{id}")),
    ] {
        if let Ok(k) = hive.open_subkey(&key) && let Ok(p) = k.get_value::<String, _>("path") {
            out.push(PathBuf::from(p));
        }
    }
    out
}

#[cfg(windows)]
fn gog_registry_paths() -> Vec<PathBuf> {
    gog_registry_folders(GOG_GAME_ID)
}

/// Elsewhere the Windows game lives in a Wine prefix, or where Heroic put it ([`unix_gog_paths`]).
#[cfg(not(windows))]
fn gog_registry_paths() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    let wine_prefix = std::env::var_os("WINEPREFIX").map(PathBuf::from);
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    unix_gog_paths(&home, wine_prefix, config.as_deref())
}

/// The Windows game on Linux and macOS: where Heroic's record of its GOG installs says it put
/// it (any folder, an SD card), the folders a GOG installer or TA's CD makes in plain Wine,
/// Lutris, Bottles, Heroic and CrossOver prefixes, and Heroic's own folders (it puts GOG's
/// games straight in a folder). `config`: the configuration folder (`XDG_CONFIG_HOME`), else
/// `~/.config`.
#[cfg(any(not(windows), test))]
fn unix_gog_paths(home: &Path, wine_prefix: Option<PathBuf>, config: Option<&Path>) -> Vec<PathBuf> {
    let mut out = heroic_paths(home, config);
    for drive_c in wine_drives(home, wine_prefix) {
        out.extend(drive_c_folders(&drive_c));
    }
    // Heroic installs GOG games natively under ~/Games/Heroic/<title>.
    out.push(home.join("Games/Heroic/Total Annihilation"));
    out.push(home.join("Games/Heroic/Total Annihilation Commander Pack"));
    out
}

/// The `drive_c` folders of the Wine prefixes under `home`: `wine_prefix` (`WINEPREFIX`) first,
/// `~/.wine`, and every prefix one level down in the usual homes of Lutris, Bottles, Heroic and
/// CrossOver (a Windows game's install on Linux and macOS).
pub fn wine_drives(home: &Path, wine_prefix: Option<PathBuf>) -> Vec<PathBuf> {
    let mut prefixes = vec![home.join(".wine")];
    if let Some(p) = wine_prefix {
        prefixes.insert(0, p);
    }
    for parent in [
        home.join("Games"),
        home.join("Games/Heroic/Prefixes"),
        home.join(".local/share/bottles/bottles"),
        home.join(".var/app/com.usebottles.bottles/data/bottles/bottles"),
        home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic/Prefixes"),
        home.join("Library/Application Support/CrossOver/Bottles"),
    ] {
        if let Ok(entries) = std::fs::read_dir(&parent) {
            prefixes.extend(entries.flatten().map(|e| e.path()));
        }
    }
    prefixes.into_iter().map(|p| p.join("drive_c")).collect()
}

/// Where GOG's installer, GOG Galaxy and TA's CD put the game on a Windows system's drive: in a
/// Wine or Proton prefix, under its `drive_c`.
#[cfg(any(not(windows), test))]
const DRIVE_C_FOLDERS: &[&str] = &[
    "GOG Games/Total Annihilation",
    "Program Files (x86)/GOG Galaxy/Games/Total Annihilation",
    "Program Files (x86)/GOG.com/Total Annihilation",
    "Program Files (x86)/Total Annihilation",
    "Program Files/Total Annihilation",
    "Games/Total Annihilation",
    "Cavedog/TotalA",
];

/// [`DRIVE_C_FOLDERS`] in the drive `drive_c`, each folder found whatever the case of its name.
#[cfg(any(not(windows), test))]
fn drive_c_folders(drive_c: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    DRIVE_C_FOLDERS.iter().map(move |folder| join_ci(drive_c, folder))
}

/// `relative` ('/' between its names) under `base`, each folder found whatever the case of its
/// name (Linux tells "TotalA" from "TOTALA", and Windows' installers write either); as written
/// from the first that is missing.
pub fn join_ci(base: &Path, relative: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for name in relative.split('/') {
        let exact = path.join(name);
        path = if exact.is_dir() { exact } else { find_dir_ci(&path, name).unwrap_or(exact) };
    }
    path
}

/// Heroic's own records of its GOG installs (`heroic/gog_store/installed.json` in `config`,
/// else `~/.config`, and in its Flatpak's): the Commander Pack's folder, wherever it was put.
#[cfg(any(not(windows), test))]
fn heroic_paths(home: &Path, config: Option<&Path>) -> Vec<PathBuf> {
    heroic_folders(home, config, GOG_GAME_ID)
}

/// Heroic's records of its GOG installs (`heroic/gog_store/installed.json` in `config`, else
/// `~/.config`, and in its Flatpak's): GOG product `id`'s folder, wherever it was put.
pub fn heroic_folders(home: &Path, config: Option<&Path>, id: &str) -> Vec<PathBuf> {
    let native = config.map_or_else(|| home.join(".config"), Path::to_path_buf);
    let flatpak = home.join(".var/app/com.heroicgameslauncher.hgl/config");
    let mut out = Vec::new();
    for dir in [native, flatpak] {
        let record = std::fs::read_to_string(dir.join("heroic").join("gog_store").join("installed.json"));
        if let Some(path) = record.ok().and_then(|text| heroic_install_path_of(&text, id))
            && !out.contains(&path)
        {
            out.push(path);
        }
    }
    out
}

/// The Commander Pack's folder in the text of Heroic's `installed.json`
/// (`{"installed": [{"appName": "1207658880", "install_path": "...", ...}, ...]}`).
#[cfg(test)]
fn heroic_install_path(text: &str) -> Option<PathBuf> {
    heroic_install_path_of(text, GOG_GAME_ID)
}

/// GOG product `id`'s folder in the text of Heroic's `installed.json`.
fn heroic_install_path_of(text: &str, id: &str) -> Option<PathBuf> {
    use serde_json::Value;
    let installed = serde_json::from_str::<Value>(text).ok()?.get("installed")?.clone();
    let entries: Vec<Value> = match installed {
        Value::Array(entries) => entries,
        Value::Object(entries) => entries.into_iter().map(|(_, entry)| entry).collect(),
        _ => return None,
    };
    let ours = |entry: &&Value| match entry.get("appName") {
        Some(Value::String(name)) => name == id,
        Some(Value::Number(name)) => name.to_string() == id,
        _ => false,
    };
    let path = entries.iter().find(ours)?.get("install_path")?.as_str()?;
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// Linux's Steam runs Windows programs through Proton ([`proton_paths`]).
#[cfg(not(windows))]
fn proton_prefix_paths() -> Vec<PathBuf> {
    std::env::var_os("HOME").map_or_else(Vec::new, |home| proton_paths(Path::new(&home)))
}

#[cfg(windows)]
fn proton_prefix_paths() -> Vec<PathBuf> {
    Vec::new()
}

/// The game's folders in the Proton prefixes of the Steam libraries under `home`
/// (`<library>/steamapps/compatdata/<app>/pfx/drive_c`): a GOG installer or TA's CD run through
/// Steam as a non-Steam game installs it there. Only folders there are (every game Steam runs
/// through Proton has a prefix: listing seven guesses for each would bury the search's list).
#[cfg(any(not(windows), test))]
fn proton_paths(home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for drive_c in proton_drives(home) {
        out.extend(drive_c_folders(&drive_c).filter(|folder| folder.is_dir()));
    }
    out
}

/// The `drive_c` folders of every Proton prefix in the Steam libraries under `home`
/// (`<library>/steamapps/compatdata/<app>/pfx/drive_c`), each library once, in order.
pub fn proton_drives(home: &Path) -> Vec<PathBuf> {
    // (`~/.steam/steam` is usually a link to `~/.local/share/Steam`: each library once.)
    let (mut libraries, mut seen) = (Vec::new(), Vec::new());
    for root in unix_steam_roots(home) {
        for library in steam_libraries(&root) {
            let real = std::fs::canonicalize(&library).unwrap_or_else(|_| library.clone());
            if !seen.contains(&real) {
                seen.push(real);
                libraries.push(library);
            }
        }
    }
    let mut out = Vec::new();
    for library in libraries {
        let Ok(apps) = std::fs::read_dir(library.join("steamapps").join("compatdata")) else { continue };
        let mut drives: Vec<PathBuf> = apps.flatten().map(|app| app.path().join("pfx").join("drive_c")).filter(|c| c.is_dir()).collect();
        drives.sort();
        out.extend(drives);
    }
    out
}

/// The Steam release's folders: its uninstall key's, then the one in every library of every
/// Steam there is ([`steam_app_folders`]).
fn steam_paths() -> Vec<PathBuf> {
    steam_app_folders(STEAM_APP_ID, STEAM_DIR)
}

/// Native Steam homes on Linux (including Flatpak) and macOS. Original Windows game
/// files can be supplied through a compatibility install or selected explicitly.
pub fn unix_steam_roots(home: &Path) -> Vec<PathBuf> {
    [".local/share/Steam", ".steam/steam", ".var/app/com.valvesoftware.Steam/.local/share/Steam", "Library/Application Support/Steam"]
        .into_iter().map(|path| home.join(path)).collect()
}

/// Parses the `"path"` entries of Steam's `libraryfolders.vdf`.
pub fn steam_libraries(steam_root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![steam_root.to_path_buf()];
    if let Ok(vdf) = std::fs::read_to_string(steam_root.join("steamapps").join("libraryfolders.vdf")) {
        for line in vdf.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("\"path\"") {
                let v = rest.trim().trim_matches('"').replace("\\\\", "\\");
                if !v.is_empty() {
                    libs.push(PathBuf::from(v));
                }
            }
        }
    }
    libs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_libraries_include_macos_and_linux_homes_and_external_libraries() {
        let home = std::env::temp_dir().join(format!("ta_steam_platforms_{}", std::process::id()));
        let roots = unix_steam_roots(&home);
        assert!(roots.contains(&home.join("Library/Application Support/Steam")));
        assert!(roots.contains(&home.join(".local/share/Steam")));
        assert!(roots.contains(&home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam")));
        let mac = home.join("Library/Application Support/Steam");
        std::fs::create_dir_all(mac.join("steamapps")).unwrap();
        std::fs::write(mac.join("steamapps/libraryfolders.vdf"), "\"libraryfolders\"\n{\n\"path\" \"/Volumes/Games/SteamLibrary\"\n}\n").unwrap();
        assert_eq!(steam_libraries(&mac), [mac, PathBuf::from("/Volumes/Games/SteamLibrary")]);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn steam_app_manifests_name_the_games_folder_and_build() {
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"298030\"\n\t\"name\"\t\t\"Total Annihilation: Commander Pack\"\n\t\"buildid\"\t\t\"1234567\"\n\t\"installdir\"\t\t\"Total Annihilation\"\n}\n";
        assert_eq!(acf_value(acf, "installdir").as_deref(), Some("Total Annihilation"));
        assert_eq!(acf_value(acf, "BuildId").as_deref(), Some("1234567"));
        assert_eq!(acf_value(acf, "nothing"), None);
        // A library whose manifest names another folder: that one.
        let lib = std::env::temp_dir().join(format!("ta_steam_lib_{}", std::process::id()));
        std::fs::create_dir_all(lib.join("steamapps")).unwrap();
        assert_eq!(steam_game_dir(&lib), lib.join("steamapps").join("common").join(STEAM_DIR));
        std::fs::write(lib.join("steamapps").join("appmanifest_298030.acf"), acf.replace("\"Total Annihilation\"\n}", "\"TA Commander Pack\"\n}")).unwrap();
        assert_eq!(steam_game_dir(&lib), lib.join("steamapps").join("common").join("TA Commander Pack"));
        std::fs::remove_dir_all(lib).unwrap();
    }

    #[test]
    fn copies_say_which_store_they_came_from() {
        let base = std::env::temp_dir().join(format!("ta_stores_{}", std::process::id()));
        // GOG's: its info file, with the build in it.
        let gog = base.join("GOG Games").join("Total Annihilation");
        std::fs::create_dir_all(&gog).unwrap();
        std::fs::write(gog.join("goggame-1207658880.info"), "{\n  \"buildId\": \"53419283\",\n  \"name\": \"Total Annihilation\"\n}").unwrap();
        assert_eq!(identify(&gog, InstallSource::CommonLocation), (Store::Gog, Some("53419283".into())));
        // Steam's: in a library's common folder, its app manifest listing it.
        let steam = base.join("SteamLibrary").join("steamapps").join("common").join("Total Annihilation");
        std::fs::create_dir_all(&steam).unwrap();
        assert_eq!(identify(&steam, InstallSource::CommonLocation), (Store::Unknown, None));
        assert_eq!(identify(&steam, InstallSource::Steam), (Store::Steam, None));
        std::fs::write(base.join("SteamLibrary").join("steamapps").join("appmanifest_298030.acf"), "\"AppState\"\n{\n\t\"buildid\"\t\t\"99\"\n\t\"installdir\"\t\t\"Total Annihilation\"\n}").unwrap();
        assert_eq!(identify(&steam, InstallSource::CommonLocation), (Store::Steam, Some("99".into())));
        // A folder copied anywhere else says nothing.
        let copied = base.join("Games").join("TA");
        std::fs::create_dir_all(&copied).unwrap();
        assert_eq!(identify(&copied, InstallSource::Explicit), (Store::Unknown, None));
        std::fs::write(copied.join("steam_appid.txt"), "298030").unwrap();
        assert_eq!(identify(&copied, InstallSource::Explicit), (Store::Steam, None));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_empty_dir() {
        let tmp = std::env::temp_dir().join("ta_vfs_install_test_empty");
        let _ = std::fs::create_dir_all(&tmp);
        assert!(validate(&tmp).is_err());
    }

    #[test]
    fn installation_detection_requires_files_instead_of_same_named_directories() {
        let root = std::env::temp_dir().join(format!("ta-install-file-kinds-{}", std::process::id()));
        std::fs::create_dir_all(root.join("TOTALA1.HPI")).unwrap();
        std::fs::write(root.join("TOTALA2.HPI"), []).unwrap();
        assert!(find_file_ci(&root, "totala1.hpi").is_none());
        assert_eq!(validate(&root).unwrap_err(), ["totala1.hpi"]);
        assert!(matches!(discover(Some(&root)), Err(InstallError::Invalid { .. })));
        std::fs::remove_dir(root.join("TOTALA1.HPI")).unwrap();
        std::fs::write(root.join("TOTALA1.HPI"), []).unwrap();
        for dir in ["CCDATA.CCX", "REV31.GP3", "music/fake.MP3", "data/fake.ZRB"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        std::fs::write(root.join("CCMAPS.CCX"), []).unwrap();
        let content = validate(&root).unwrap();
        assert!(content.base_game);
        assert!(!content.core_contingency && !content.patch_3_1 && !content.music && !content.movies);
        std::fs::write(root.join("music/track.OGG"), []).unwrap();
        std::fs::write(root.join("data/intro.ZRB"), []).unwrap();
        let content = validate(&root).unwrap();
        assert!(content.music && content.movies);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explicit_invalid_path_is_reported() {
        let err = discover(Some(Path::new("Z:/definitely/not/here"))).unwrap_err();
        assert!(matches!(err, InstallError::Invalid { .. }));
    }

    /// An empty home for a test.
    fn home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("ta_install_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    /// The game's files where an installer put them.
    fn install_at(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        for file in REQUIRED_FILES {
            std::fs::write(dir.join(file), b"HAPI").unwrap();
        }
    }

    /// One folder, however its name is spelt (Linux keeps the case on disk; Windows and macOS
    /// match any).
    fn same_folder(a: &Path, b: &Path) -> bool {
        std::fs::canonicalize(a).unwrap() == std::fs::canonicalize(b).unwrap()
    }

    /// A GOG installer or TA's CD run through Steam as a non-Steam game (a Steam Deck's way)
    /// installs the game in that game's Proton prefix: found there, in any library (an SD card's
    /// too), whatever the case of the CD's folders; other games' prefixes add nothing to the
    /// search.
    #[test]
    fn installs_in_steams_proton_prefixes_are_found() {
        let home = home("proton");
        let steam = home.join(".local/share/Steam");
        let gog = steam.join("steamapps/compatdata/3087654321/pfx/drive_c/GOG Games/Total Annihilation");
        install_at(&gog);
        std::fs::write(gog.join("goggame-1207658880.info"), "{\n  \"buildId\": \"53419283\"\n}").unwrap();
        // Another game's prefix.
        std::fs::create_dir_all(steam.join("steamapps/compatdata/1091500/pfx/drive_c/Program Files/Common Files")).unwrap();
        // The SD card, a library of its own, where the CD went (its folders in capitals).
        let card = home.join("run/media/deck/SD");
        let vdf = format!("\"libraryfolders\"\n{{\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n", card.display());
        std::fs::write(steam.join("steamapps/libraryfolders.vdf"), vdf).unwrap();
        let cd = card.join("steamapps/compatdata/2999999999/pfx/drive_c/CAVEDOG/TOTALA");
        install_at(&cd);
        let found = proton_paths(&home);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(same_folder(&found[0], &gog) && same_folder(&found[1], &cd), "{found:?}");
        for path in &found {
            assert!(validate(path).is_ok(), "{}", path.display());
        }
        // Its files say which copy it is.
        assert_eq!(identify(&found[0], InstallSource::CommonLocation), (Store::Gog, Some("53419283".into())));
        assert_eq!(identify(&found[1], InstallSource::CommonLocation), (Store::Unknown, None));
        std::fs::remove_dir_all(home).unwrap();
    }

    /// Heroic's record of its GOG installs names the game's folder wherever it was put (an SD
    /// card here): its own app's record and its Flatpak's, first among the places searched off
    /// Windows. Other games' entries, and records without the game, add nothing.
    #[test]
    fn heroics_record_of_its_gog_installs_is_read() {
        let home = home("heroic");
        let record = |path: &Path| {
            serde_json::json!({ "installed": [
                { "platform": "windows", "install_path": "/home/deck/Games/Heroic/Another Game", "is_dlc": false, "appName": "1207658924" },
                { "platform": "windows", "install_path": path, "is_dlc": false, "appName": GOG_GAME_ID, "buildId": "53419283" },
            ] })
            .to_string()
        };
        let card = home.join("run/media/deck/SD/Heroic/Total Annihilation");
        install_at(&card);
        let native = home.join(".config/heroic/gog_store");
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(native.join("installed.json"), record(&card)).unwrap();
        assert_eq!(heroic_paths(&home, None), std::slice::from_ref(&card));
        assert_eq!(unix_gog_paths(&home, None, None).first(), Some(&card));
        // The Flatpak's record too; and a configuration folder of the player's own, which holds none.
        let flatpak_install = home.join("Games/Heroic/TA");
        let flatpak = home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic/gog_store");
        std::fs::create_dir_all(&flatpak).unwrap();
        std::fs::write(flatpak.join("installed.json"), record(&flatpak_install)).unwrap();
        assert_eq!(heroic_paths(&home, None), [card.clone(), flatpak_install.clone()]);
        assert_eq!(heroic_paths(&home, Some(&home.join("elsewhere"))), [flatpak_install]);
        // Records without the game, or not to be read.
        assert_eq!(heroic_install_path("{\"installed\": []}"), None);
        assert_eq!(heroic_install_path("{\"installed\": [{\"appName\": \"1207658880\", \"install_path\": \"\"}]}"), None);
        assert_eq!(heroic_install_path("{\"installed\": [{\"appName\": \"1207658880\"}]}"), None);
        assert_eq!(heroic_install_path("not json"), None);
        assert_eq!(heroic_install_path("{\"installed\": {\"1207658880\": {\"appName\": 1207658880, \"install_path\": \"/games/ta\"}}}"), Some(PathBuf::from("/games/ta")));
        std::fs::remove_dir_all(home).unwrap();
    }

    /// Wine's prefixes are searched for the same folders as Proton's: the CD's install in
    /// `Program Files`, in `WINEPREFIX` and in `~/.wine`.
    #[test]
    fn wine_prefixes_are_searched_for_the_installers_folders() {
        let home = home("wine");
        let cd = home.join(".wine/drive_c/Program Files/Total Annihilation");
        install_at(&cd);
        let own = home.join("prefixes/ta");
        let gog = own.join("drive_c/Program Files (x86)/Total Annihilation");
        install_at(&gog);
        let found = unix_gog_paths(&home, Some(own), None);
        let at = |dir: &Path| found.iter().position(|p| p.as_path() == dir);
        assert!(at(&gog).unwrap() < at(&cd).unwrap(), "{found:?}");
        for folder in DRIVE_C_FOLDERS {
            assert!(found.contains(&join_ci(&home.join(".wine/drive_c"), folder)), "{folder}");
        }
        std::fs::remove_dir_all(home).unwrap();
    }
}
