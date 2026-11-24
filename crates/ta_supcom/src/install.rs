//! Finding Supreme Commander: Forged Alliance, and the maps players keep of their own.
//!
//! The game: GOG's (product 1444785261, also inside GOG's Gold Edition), Steam's (app 9420), the
//! old boxed copy's (THQ's registry key), the usual folders, and off Windows Heroic's GOG
//! installs, Wine's, Lutris', Bottles', CrossOver's and Proton's prefixes, and Steam's libraries.
//! A folder counts when it holds the game's data (`gamedata/env.scd`) and its maps (`maps/`); the
//! original Supreme Commander's are laid out alike, and Forged Alliance is told from it by what
//! only it has ([`is_forged_alliance`]).
//!
//! The players' own maps (downloaded, or Forged Alliance Forever's vault):
//! `<Documents>/My Games/Gas Powered Games/Supreme Commander Forged Alliance/maps`, Documents
//! wherever Windows keeps it (a OneDrive folder, often), FAF's own copy of that folder under
//! `ProgramData`, and on Linux the same in Steam's Proton prefix and in the home folder.

use std::path::{Path, PathBuf};

use ta_vfs::install::{find_dir_ci, find_file_ci, join_ci};

/// GOG's product id of Forged Alliance (the Gold Edition installs it under the same id).
pub const GOG_ID: &str = "1444785261";
/// Steam's app id of Forged Alliance.
pub const STEAM_APP_ID: u32 = 9420;
/// The game's pages in its stores, for a player without it: GOG's Gold Edition (Supreme Commander
/// with Forged Alliance, installed under [`GOG_ID`]) and Steam's Forged Alliance ([`STEAM_APP_ID`]).
pub const GOG_STORE_URL: &str = "https://www.gog.com/en/game/supreme_commander_gold_edition";
pub const STEAM_STORE_URL: &str = "https://store.steampowered.com/app/9420/";
/// The folder Steam installs it in, unless its app manifest names another.
pub const STEAM_DIR: &str = "Supreme Commander Forged Alliance";
/// The players' own maps under Documents.
const USER_MAPS: &str = "My Games/Gas Powered Games/Supreme Commander Forged Alliance/maps";

/// Where the game was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Gog,
    Steam,
    /// The boxed copy (THQ's installer).
    Retail,
    /// A folder the player picked, or one where any copy may be.
    Folder,
}

impl Store {
    /// The store's name, as it calls itself (none for a folder).
    pub fn name(self) -> Option<&'static str> {
        match self {
            Store::Gog => Some("GOG"),
            Store::Steam => Some("Steam"),
            Store::Retail => None,
            Store::Folder => None,
        }
    }
}

/// A copy of the game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    pub root: PathBuf,
    pub store: Store,
}

impl Install {
    /// Its `gamedata` folder, whatever its case.
    pub fn gamedata(&self) -> PathBuf {
        find_dir_ci(&self.root, "gamedata").unwrap_or_else(|| self.root.join("gamedata"))
    }

    /// Its `maps` folder, whatever its case.
    pub fn maps(&self) -> PathBuf {
        find_dir_ci(&self.root, "maps").unwrap_or_else(|| self.root.join("maps"))
    }
}

/// Whether `root` holds the game: its data archives (`gamedata/env.scd`) and its maps.
pub fn validate(root: &Path) -> bool {
    let Some(gamedata) = find_dir_ci(root, "gamedata") else { return false };
    find_file_ci(&gamedata, "env.scd").is_some() && find_dir_ci(root, "maps").is_some()
}

/// Whether the game in `root` is Forged Alliance rather than the original Supreme Commander, whose
/// folders are laid out alike: its own maps are in its maps folder (the expansion's, `X1MP_...`
/// and `X1CA_...`, which the original has none of), or its data path names it
/// (`bin/SupComDataPath.lua`, where the original names its own folder in Documents).
pub fn is_forged_alliance(root: &Path) -> bool {
    let expansion_maps = find_dir_ci(root, "maps").and_then(|maps| std::fs::read_dir(maps).ok()).is_some_and(|entries| {
        entries.flatten().any(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("x1"))
    });
    expansion_maps
        || find_dir_ci(root, "bin")
            .and_then(|bin| find_file_ci(&bin, "SupComDataPath.lua"))
            .and_then(|lua| std::fs::read(lua).ok())
            .is_some_and(|text| String::from_utf8_lossy(&text).to_ascii_lowercase().contains("forged alliance"))
}

/// The game in `picked`, a folder the player chose, or one level down in it (players often pick
/// the folder the game is in): Forged Alliance before the original Supreme Commander, whichever
/// comes first by name; the original only where it is all there is.
pub fn validate_picked(picked: &Path) -> Option<PathBuf> {
    let here = validate(picked);
    if here && is_forged_alliance(picked) {
        return Some(picked.to_path_buf());
    }
    let mut children: Vec<PathBuf> = std::fs::read_dir(picked).ok().map(|entries| entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).take(512).collect()).unwrap_or_default();
    children.sort();
    let games: Vec<PathBuf> = children.into_iter().filter(|c| validate(c)).collect();
    if let Some(forged) = games.iter().find(|c| is_forged_alliance(c)) {
        return Some(forged.clone());
    }
    if here { Some(picked.to_path_buf()) } else { games.into_iter().next() }
}

/// Every copy of the game found, each once, in the order: GOG, Steam, the boxed copy, the
/// usual folders (and off Windows the prefixes').
pub fn discover() -> Vec<Install> {
    let mut out: Vec<Install> = Vec::new();
    for (path, store) in candidates() {
        if validate(&path) && !out.iter().any(|i| same_folder(&i.root, &path)) {
            out.push(Install { root: path, store });
        }
    }
    out
}

/// Where the game may be, in the order searched (some there, most not).
pub fn candidates() -> Vec<(PathBuf, Store)> {
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        for p in ta_vfs::install::gog_registry_folders(GOG_ID) {
            out.push((p, Store::Gog));
        }
    }
    for p in ta_vfs::install::steam_app_folders(STEAM_APP_ID, STEAM_DIR) {
        out.push((p, Store::Steam));
    }
    #[cfg(windows)]
    {
        use winreg::HKLM;
        for key in [r"SOFTWARE\WOW6432Node\THQ\Gas Powered Games\Supreme Commander - Forged Alliance", r"SOFTWARE\THQ\Gas Powered Games\Supreme Commander - Forged Alliance"] {
            if let Ok(k) = HKLM.open_subkey(key) {
                for value in ["InstallationDirectory", "InstallDir", "Path"] {
                    if let Ok(p) = k.get_value::<String, _>(value) {
                        out.push((PathBuf::from(p), Store::Retail));
                    }
                }
            }
        }
        for drive in ["C", "D", "E", "F"] {
            for (sub, store) in [
                (r"GOG Games\Supreme Commander Forged Alliance", Store::Gog),
                (r"Program Files (x86)\GOG Galaxy\Games\Supreme Commander Forged Alliance", Store::Gog),
                (r"Program Files (x86)\Steam\steamapps\common\Supreme Commander Forged Alliance", Store::Steam),
                (r"SteamLibrary\steamapps\common\Supreme Commander Forged Alliance", Store::Steam),
                (r"Program Files (x86)\THQ\Gas Powered Games\Supreme Commander - Forged Alliance", Store::Retail),
                (r"Games\Supreme Commander Forged Alliance", Store::Folder),
            ] {
                out.push((PathBuf::from(format!(r"{drive}:\{sub}")), store));
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
            for p in ta_vfs::install::heroic_folders(&home, config.as_deref(), GOG_ID) {
                out.push((p, Store::Gog));
            }
            let wine = std::env::var_os("WINEPREFIX").map(PathBuf::from);
            let drives: Vec<PathBuf> = ta_vfs::install::wine_drives(&home, wine).into_iter().chain(ta_vfs::install::proton_drives(&home)).collect();
            for drive_c in drives {
                for (folder, store) in DRIVE_C_FOLDERS {
                    out.push((join_ci(&drive_c, folder), *store));
                }
            }
            out.push((home.join("Games/Heroic/Supreme Commander Forged Alliance"), Store::Gog));
        }
    }
    out
}

/// Where the game's installers put it on a Windows drive (in a Wine or Proton prefix).
#[cfg_attr(windows, allow(dead_code))]
const DRIVE_C_FOLDERS: &[(&str, Store)] = &[
    ("GOG Games/Supreme Commander Forged Alliance", Store::Gog),
    ("Program Files (x86)/GOG Galaxy/Games/Supreme Commander Forged Alliance", Store::Gog),
    ("Program Files (x86)/THQ/Gas Powered Games/Supreme Commander - Forged Alliance", Store::Retail),
    ("Program Files/THQ/Gas Powered Games/Supreme Commander - Forged Alliance", Store::Retail),
];

/// The folders players' own maps are kept in that are there: each holds a folder per map (a
/// downloaded one, or one of Forged Alliance Forever's vault, `<name>.v0003`).
pub fn user_map_folders() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut add = |dir: PathBuf| {
        if dir.is_dir() && !out.iter().any(|o| same_folder(o, &dir)) {
            out.push(dir);
        }
    };
    for documents in documents_folders() {
        add(join_ci(&documents, USER_MAPS));
    }
    #[cfg(windows)]
    {
        let program_data = std::env::var_os("ProgramData").map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
        add(join_ci(&program_data, &format!("FAForever/user/{USER_MAPS}")));
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            // Steam's Proton prefix of the game, and FAF's Linux client's own home.
            for root in ta_vfs::install::unix_steam_roots(&home) {
                for lib in ta_vfs::install::steam_libraries(&root) {
                    let user = lib.join(format!("steamapps/compatdata/{STEAM_APP_ID}/pfx/drive_c/users/steamuser"));
                    for docs in ["Documents", "My Documents"] {
                        add(join_ci(&user.join(docs), USER_MAPS));
                    }
                }
            }
            add(join_ci(&home, USER_MAPS));
        }
    }
    out
}

/// Where the players' own maps are looked for first, whether there are any or not (Documents'
/// `My Games/Gas Powered Games/Supreme Commander Forged Alliance/maps`, as the game and Forged
/// Alliance Forever's vault keep them): what is named when none are found.
pub fn user_maps_folder() -> Option<PathBuf> {
    documents_folders().into_iter().next().map(|documents| join_ci(&documents, USER_MAPS))
}

/// The player's Documents folder: where Windows keeps it (the shell's own record of it, which
/// follows OneDrive), else `Documents` in the home folder.
fn documents_folders() -> Vec<PathBuf> {
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        use winreg::HKCU;
        let expand = |s: &str| -> String {
            // `%USERPROFILE%\Documents`: the variables the shell's record may hold.
            let mut out = String::new();
            let mut rest = s;
            while let Some(start) = rest.find('%') {
                out.push_str(&rest[..start]);
                let after = &rest[start + 1..];
                match after.find('%') {
                    Some(end) => {
                        let name = &after[..end];
                        out.push_str(&std::env::var(name).unwrap_or_else(|_| format!("%{name}%")));
                        rest = &after[end + 1..];
                    }
                    None => {
                        out.push_str(&rest[start..]);
                        rest = "";
                    }
                }
            }
            out.push_str(rest);
            out
        };
        for key in [r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders", r"Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders"] {
            if let Ok(k) = HKCU.open_subkey(key) && let Ok(p) = k.get_value::<String, _>("Personal") {
                out.push(PathBuf::from(expand(&p)));
            }
        }
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            out.push(PathBuf::from(profile).join("Documents"));
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            out.push(PathBuf::from(home).join("Documents"));
        }
    }
    out
}

/// Whether two paths name one folder (on Windows whatever their case and slashes).
pub fn same_folder(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        let key = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase();
        key(a) == key(b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A copy of the game's folders (empty files where its archives are), with the expansion's
    /// own maps.
    pub(crate) fn fake_install(root: &Path) {
        fake_original(root);
        std::fs::create_dir_all(root.join("Maps").join("X1MP_001")).unwrap();
    }

    /// The original Supreme Commander's folders, laid out as Forged Alliance's are (its maps but
    /// not the expansion's, its data path naming its own folder).
    fn fake_original(root: &Path) {
        std::fs::create_dir_all(root.join("GameData")).unwrap();
        std::fs::write(root.join("GameData").join("ENV.scd"), b"PK").unwrap();
        std::fs::create_dir_all(root.join("Maps").join("SCMP_001")).unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("bin").join("SupComDataPath.lua"), "mount_contents(SHGetFolderPath('PERSONAL') .. 'My Games\\\\Gas Powered Games\\\\Supreme Commander\\\\maps', '/maps')").unwrap();
    }

    #[test]
    fn a_folder_is_the_game_when_it_holds_its_data_and_its_maps_in_any_case() {
        let dir = crate::tests::Temp::new("install");
        let root = dir.0.join("Supreme Commander Forged Alliance");
        assert!(!validate(&root));
        fake_install(&root);
        assert!(validate(&root));
        let install = Install { root: root.clone(), store: Store::Folder };
        assert_eq!(install.gamedata(), root.join("GameData"));
        assert_eq!(install.maps(), root.join("Maps"));
        // The folder it is in, picked: found one level down.
        assert_eq!(validate_picked(&dir.0), Some(root.clone()));
        assert_eq!(validate_picked(&root), Some(root.clone()));
        std::fs::remove_dir_all(root.join("Maps")).unwrap();
        assert!(!validate(&root), "no maps: not the game");
        assert_eq!(validate_picked(&dir.0), None);
    }

    /// Forged Alliance is told from the original Supreme Commander (laid out alike) by its own
    /// maps or its data path; picked, a folder holding both gives Forged Alliance, though the
    /// original comes first by name, and the original only where it is all there is.
    #[test]
    fn forged_alliance_is_told_from_the_original_game() {
        let dir = crate::tests::Temp::new("original");
        let original = dir.0.join("Supreme Commander");
        let forged = dir.0.join("Supreme Commander Forged Alliance");
        fake_original(&original);
        assert!(validate(&original) && !is_forged_alliance(&original));
        assert_eq!(validate_picked(&dir.0), Some(original.clone()), "the original, where it is all there is");
        fake_install(&forged);
        assert!(is_forged_alliance(&forged));
        assert_eq!(validate_picked(&dir.0), Some(forged.clone()), "Forged Alliance before the original");
        assert_eq!(validate_picked(&original), Some(original.clone()), "the original picked itself is still the game found there");
        // Its data path names it, though its maps are not the expansion's.
        std::fs::remove_dir_all(forged.join("Maps").join("X1MP_001")).unwrap();
        assert!(!is_forged_alliance(&forged));
        std::fs::write(forged.join("bin").join("SupComDataPath.lua"), "mount_contents(SHGetFolderPath('PERSONAL') .. 'My Games\\\\Gas Powered Games\\\\Supreme Commander Forged Alliance\\\\maps', '/maps')").unwrap();
        assert!(is_forged_alliance(&forged));
    }

    /// Steam's libraries are searched for the game's folder, as its app manifest names it.
    #[test]
    fn steam_libraries_name_the_games_folder() {
        let dir = crate::tests::Temp::new("steam");
        let lib = dir.0.join("SteamLibrary");
        std::fs::create_dir_all(lib.join("steamapps")).unwrap();
        assert_eq!(ta_vfs::install::steam_app_dir(&lib, STEAM_APP_ID, STEAM_DIR), lib.join("steamapps/common").join(STEAM_DIR));
        std::fs::write(lib.join("steamapps/appmanifest_9420.acf"), "\"AppState\"\n{\n\t\"appid\"\t\t\"9420\"\n\t\"installdir\"\t\t\"FA\"\n}\n").unwrap();
        assert_eq!(ta_vfs::install::steam_app_dir(&lib, STEAM_APP_ID, STEAM_DIR), lib.join("steamapps/common/FA"));
    }
}
