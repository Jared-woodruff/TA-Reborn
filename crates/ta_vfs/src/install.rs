//! Locating and validating the user's original Total Annihilation install.
//!
//! TA Reborn ships no original game data. At first run we locate a legitimately
//! installed copy — "Total Annihilation: Commander Pack" from GOG or from Steam, which ship the
//! same data files — and read its archives in place. Detection order:
//!
//! 1. an explicit path (command line / config),
//! 2. the `TA_REBORN_GAME_DIR` environment variable,
//! 3. the GOG registry key written by both the offline installer and GOG Galaxy,
//! 4. the Steam release: its uninstall key, then every Steam library (the folder each one's
//!    `appmanifest_298030.acf` names),
//! 5. well-known default install folders (GOG's and Steam's).
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

pub(crate) fn find_dir_ci(dir: &Path, name: &str) -> Option<PathBuf> {
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
fn acf_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut quoted = line.split('"').skip(1).step_by(2);
        if quoted.next()?.eq_ignore_ascii_case(key) { quoted.next().map(|v| v.replace("\\\\", "\\")) } else { None }
    })
}

/// Where Steam keeps the game in a library: the folder its app manifest names, else Steam's
/// usual one.
fn steam_game_dir(library: &Path) -> PathBuf {
    let steamapps = library.join("steamapps");
    let named = find_file_ci(&steamapps, &format!("appmanifest_{STEAM_APP_ID}.acf"))
        .and_then(|m| std::fs::read_to_string(m).ok())
        .and_then(|text| acf_value(&text, "installdir"));
    steamapps.join("common").join(named.unwrap_or_else(|| STEAM_DIR.to_string()))
}

#[cfg(windows)]
fn gog_registry_paths() -> Vec<PathBuf> {
    use winreg::{HKCU, HKLM};
    let mut out = Vec::new();
    for (hive, key) in [
        (HKLM, format!(r"SOFTWARE\WOW6432Node\GOG.com\Games\{GOG_GAME_ID}")),
        (HKLM, format!(r"SOFTWARE\GOG.com\Games\{GOG_GAME_ID}")),
        (HKCU, format!(r"SOFTWARE\GOG.com\Games\{GOG_GAME_ID}")),
    ] {
        if let Ok(k) = hive.open_subkey(&key) && let Ok(p) = k.get_value::<String, _>("path") {
            out.push(PathBuf::from(p));
        }
    }
    out
}

/// Elsewhere the Windows game lives in a Wine prefix: plain Wine, Lutris and Bottles
/// prefixes, or Heroic's GOG installs (which put the game straight in a folder).
#[cfg(not(windows))]
fn gog_registry_paths() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return Vec::new() };
    let mut prefixes = vec![home.join(".wine")];
    if let Some(p) = std::env::var_os("WINEPREFIX") {
        prefixes.insert(0, PathBuf::from(p));
    }
    // Every prefix one level down in the usual homes of Lutris, Bottles and Heroic.
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
    let mut out = Vec::new();
    for prefix in prefixes {
        let c = prefix.join("drive_c");
        for sub in ["GOG Games/Total Annihilation", "Program Files (x86)/GOG Galaxy/Games/Total Annihilation", "Program Files (x86)/GOG.com/Total Annihilation", "Games/Total Annihilation", "Cavedog/TotalA"] {
            out.push(c.join(sub));
        }
    }
    // Heroic installs GOG games natively under ~/Games/Heroic/<title>.
    out.push(home.join("Games/Heroic/Total Annihilation"));
    out.push(home.join("Games/Heroic/Total Annihilation Commander Pack"));
    out
}

#[cfg(windows)]
fn steam_paths() -> Vec<PathBuf> {
    use winreg::{HKCU, HKLM};
    let mut out = Vec::new();
    for key in [
        format!(r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Steam App {STEAM_APP_ID}"),
        format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Steam App {STEAM_APP_ID}"),
    ] {
        if let Ok(k) = HKLM.open_subkey(key) && let Ok(p) = k.get_value::<String, _>("InstallLocation") {
            out.push(PathBuf::from(p));
        }
    }
    // Steam itself: this user's (where it really is), the machine's, else its usual folder;
    // every library each lists.
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
    for root in roots {
        for lib in steam_libraries(&root) {
            let dir = steam_game_dir(&lib);
            if !out.iter().any(|o| same_path(o, &dir)) {
                out.push(dir);
            }
        }
    }
    out
}

/// Whether two paths name one folder (Windows: whatever their case and slashes).
#[cfg(windows)]
fn same_path(a: &Path, b: &Path) -> bool {
    let key = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase();
    key(a) == key(b)
}

/// Steam (run through Proton): the native client, the Flatpak one, and the libraries they
/// list (the Steam Deck's SD card included).
#[cfg(not(windows))]
fn steam_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for root in unix_steam_roots(&home) {
            for lib in steam_libraries(&root) {
                out.push(steam_game_dir(&lib));
            }
        }
    }
    out
}

/// Native Steam homes on Linux (including Flatpak) and macOS. Original Windows game
/// files can be supplied through a compatibility install or selected explicitly.
#[cfg(any(not(windows), test))]
fn unix_steam_roots(home: &Path) -> Vec<PathBuf> {
    [".local/share/Steam", ".steam/steam", ".var/app/com.valvesoftware.Steam/.local/share/Steam", "Library/Application Support/Steam"]
        .into_iter().map(|path| home.join(path)).collect()
}

/// Parses the `"path"` entries of Steam's `libraryfolders.vdf`.
fn steam_libraries(steam_root: &Path) -> Vec<PathBuf> {
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
}
