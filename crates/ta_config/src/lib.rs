//! User configuration, stored as TOML at `%APPDATA%\TA Reborn\config.toml`
//! (`$XDG_CONFIG_HOME/ta-reborn/config.toml` elsewhere). Written by the launcher,
//! read by the game; unknown or missing fields fall back to defaults so old files keep
//! working as settings are added.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub mod achievements;
pub mod benchmark;
pub mod hardware;
pub mod maps;
pub mod profiles;
pub mod records;
pub mod running;
mod validation;

pub const APP_DIR_NAME: &str = "TA Reborn";
/// The folder's name up to 1.9, when the game was called TA Remastered (see
/// [`adopt_old_folder`]).
pub const OLD_APP_DIR_NAME: &str = "TA Remastered";
pub const CONFIG_FILE: &str = "config.toml";
/// Generous headroom for bindings, map preferences and future settings. Reject
/// corrupt/shared files before allocating or parsing them on an application's UI thread.
pub const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

/// Saved-game/replay file format. Version 41 adds bounded compatibility metadata after
/// the unchanged version 36–40 command-history body; version 42 adds a Conquest battle's
/// reinforcements and defence to its setup (36–41 are read with their own layout); 43 the rules'
/// mutators; 44 the constructors' duties (a few bytes the officer of a saved game reads, after the
/// setup: the duties are the player's machine's own and are in no command); 45, 44's layout, its
/// data hash covering the units a campaign mission allows (`camps/useonly`, which 41 to 44 left
/// out: a mod changing them passed the check); 46, the mods it was made with (`records::RecordMod`,
/// in load order, in a trailer of their own: `records::append_mods`), so a player can be told which
/// to get or turn on. Simulation changes no longer imply an unreadable file; update
/// [`SIMULATION_VERSION`] for those instead.
pub const RECORD_VERSION: u32 = 46;
/// Deterministic simulation revision, separate from the record's file layout. Saved
/// games resume only on a matching revision; older replays can be watched with a warning.
/// Revision 40 includes factory ownership, transport recovery and AI cancelled-job fixes; 41,
/// vertically launched missiles (a Merl's rockets, nukes) gathering speed as they climb; 42,
/// signals, and computer players that help their allies; 43, computer players calling their
/// allies for help and to their attacks; 44, resigning in a team game leaves the army to an ally;
/// 45, no captures in a ceasefire, Commanders kept back in transports, units taken apart or
/// frames given up not counted as losses, and the computer players' fixes of the engine and
/// skirmish reviews (upgrades, waves, defeated bases, the D-gun's flight, allies' asks, gifts);
/// 46, a unit captured mid-tick is its new owner's to the target searches still to come in it;
/// 47, a queue's entry moved or dropped as the queue stands when the command runs; 48, the
/// computer players know only what they see (and remember), and look for the enemy; 49,
/// an oversized unit group no longer blocks smaller gifts or inherited units that fit; 50,
/// aircraft receive their activation callback on the first flight as well as later takeoffs;
/// 51, Stop on a factory empties its production queue (the unit in production refunded), and
/// Core builders no longer offer the Hydration Plant (their BUILD tab read as a build button);
/// 52, scripts get `XZ_ATAN` and `ATAN` as TA gives them, 0..65535 (mods' walkers find the
/// ground under their feet); 53, and are called with headings and pitches so too (aims,
/// `StartBuilding`, `SetDirection`: mods' guns keep to their arcs); 54, a vertically launched
/// missile is guided whatever its `guidance` says (Total Mayhem's nine climbed for ever or hung
/// over their silos), and a factory takes the orders its weapons fire on (Escalation's towers
/// are builders; mods' buildpad direction); 55, big walkers stand level whatever the ground (the
/// mods' giants that the ground tilted, `ta_sim::defs::big_walker`: their guns fire from level);
/// 56, submerged units use only underwater-capable weapons and loaded aircraft stay in flight;
/// 57, mission scripts resolve complete target names before types, invalid placed players are
/// skipped, fractional objective timers keep their full duration, and mission passengers
/// respect transport ownership, capacity and cargo size; 58, nuclear defence uses swept
/// missile interception, scripted launch readiness, resource-limited production and
/// visible mobile anti-nuke cover; 59, construction uses exact affordable work, fractional
/// repairs accumulate, guard assists frames, and factory release preserves player orders;
/// 60, aircraft use clear landing footprints and reserved repair decks, carriers repair
/// all aircraft, forward weapons turn into targets, and air transports unload all cargo;
/// 61, ground movement checks swept clearance, preserves exact/failed route endpoints,
/// separates large bodies, follows initial terrain tilt and stops completed firing orders;
/// 62, naval sensors require sonar below water, model/weapon geometry follows WaterLine,
/// torpedoes remain water-bound, underwater launch permissions and draft constrain units;
/// 63, building yards change passability and reserve construction cells, script activation
/// invokes callbacks, and disabled/unpowered buildings stop production or sensor coverage;
/// 64, skirmish AI respects role/order reservations, scripts/cargo and weapon/repair reach,
/// retains outbound strikes, and reconciles friendly intelligence and ally gift capacity;
/// 65, scenarios track distinct hold areas, retire captured hunters, respect zero waves
/// and future commanders, and hash mission objectives, schedules, scripts and progress;
/// 66, strategic (`cruise`) missiles cross the map high and dive onto their target, and missiles
/// at the top of a vertical climb turn over towards their target;
/// 67, idle aircraft land only on level ground (or a seaplane's water) clear of hillsides and
/// of other aircraft's places, the nearest such place, climbing over the ground on the way;
/// 68, a script's `spin ... speed <0>` about an idle axis stops the piece's other spins (the
/// radar vehicles' dishes stop when they are switched off);
/// 69, units placed together (a scenario's waves, spawned units) never inside one another nor
/// all on one spot when no way to their target is found, a wave's aircraft arriving in flight,
/// and scenario buildings square to the grid with their aircraft parked clear of everything;
/// 70, VTOL aircraft (transports, construction aircraft, gunships) slow down at their brake rate
/// and stop over the place they were sent: air transports set their cargo down there instead
/// of circling it;
/// 71, a strategic missile (a nuke, an EMP or tactical missile: stockpiled, guided, no `tracks`)
/// fired at a unit comes down where the unit stood at its launch, not after the unit;
/// 72, a unit never takes on one of its bad targets of its own accord (`BadTargetCategory`, per
/// weapon `wpri_`/`wsec_`/`wspe_badTargetCategory`: anti-air units leave the ground alone,
/// ground units aircraft), nor answers or chases one (`NoChaseCategory`); attack orders still do;
/// 73, `NoAutoFire=1` units (the Big Bertha, Intimidator, Vulcan, Buzzsaw, Doomsday Machine,
/// Annihilator) are set to hold fire on construction, whatever `StandingFireOrder`; the
/// anti-nukes excepted;
/// 74, a mission script's `o` reads its move order first, then its fire order (`o 1 2`:
/// maneuver, fire at will; it was read the other way round);
/// 75, a weapon whose bad targets are every unit it could hit (mods' flak and anti-air missiles
/// listing a tag all their aircraft carry, Total Mayhem's `ALL`) takes on what it can, not
/// nothing;
/// 76, the units a campaign mission places for the player keep their file's fire orders
/// (`StandingFireOrder`): never built, they are not set to hold fire for `NoAutoFire=1` (Com
/// Pattern Five's and A Shortage of Water's Annihilators fire at will);
/// 77, a scenario wave's units whose every weapon fires on order only (bombers) are sent at the
/// nearest of the player's units their side sees, and bomb it, instead of attack-moving;
/// 78, a unit runs its own script (`scripts/<UnitName>.cob`) before its model's: mods' units
/// borrowing another's model (TA: Enhanced's 62, Total Mayhem's 31) ran that unit's, or none;
/// 79, a unit is finished once: other builders' work on one its own side is taking apart only
/// mends it (the reclaim goes on, as in TA), one left part taken apart is built back up without
/// being finished again, and the skirmish AI sends nobody to finish an extractor it is upgrading;
/// 80, a weapon's bad targets are lifted when they are every unit a player can field that it could
/// hit (units nobody builds don't count: Escalation's Raptor fires its Starburst Missiles again);
/// 81, a weapon takes its bad targets on last, when nothing better is in reach (missile towers on
/// tanks with no aircraft about), and turns to a better one coming into reach; a player's
/// `+shootall` makes every target alike; construction left with nobody at it un-builds after ten
/// seconds, its metal coming back; a unit is taken apart in time by its metal (a metal maker at
/// once); a plane idle over the sea circles round its spot; planes sent at one target together
/// come in on it across a front; 82, a bomber on patrol or attack-move with fire at will bombs
/// what its side sees near its way by itself, keeping a building through its turn for the next run;
/// 83, units stepping off a building's site step clear of the builder's next sites, and a unit
/// standing in a building is put on the nearest free cell; 84, a ship is built at the surface (a
/// shipyard's pad is under the water), and construction aircraft drift round what they build;
/// 85, scouts, construction aircraft and transports idle over the sea circle there too, each round
/// a circle of its own; 86, an aircraft on patrol, attack-move or attack worn below two-thirds of
/// its health breaks off for the nearest free repair deck (its orders held until it is mended),
/// pads switched off are not used, pads mend only the aircraft on their decks, and a plane sent
/// onto a pad looks for it as far as it flies on before stopping; 87, scripts asking TA 3.1's
/// questions about units by id (`MIN_ID`, `MAX_ID`, `MY_ID`, `UNIT_TEAM`,
/// `UNIT_BUILD_PERCENT_LEFT`, `UNIT_ALLIED`, `UNIT_IS_ON_THIS_COMP`) are answered, not told 0;
/// 88, mods' buildings make their own upgrade modules: a unit with build power builds what its
/// menu offers whatever its Builder flag, and one without builds nothing; a factory builds on
/// what it has started out of its stance and switched off; a unit with a speed and an
/// acceleration of none stands where it was made, and it or one given no footprint takes no
/// ground; one that cannot move is not sent out of its factory's yard and holds nothing up; a
/// unit that carries none takes on its own finished units its script attaches, which work on it
/// and go with it whatever shields them; only PLANT factories close up when idle, and a mod's other
/// building that builds is switched on when built as its file says; EnergyUse counts
/// only while switched on (Total Mayhem's fusion reactor makes 1,050, not 2,100); scripts cloak
/// their units (`UNIT_CLOAKED`); Core Contingency's mines start cloaked (`init_cloaked`); 89, a
/// building put over what lies loose on the ground clears its footprint of it (the features that
/// do not block and are neither wreckage holding metal nor deposits, vents or indestructible:
/// TA's ferns, fronds, small stones and scars); 90, a builder's build stance is its script's, as in
/// TA, also between jobs (Total Mayhem's construction kbots build a job that comes while their
/// arm is still out from the last), and a job has begun once it is the builder's, whatever the
/// stance; 91, a builder's menu is its GUI pages and download entries, the 3.1 patch's
/// `[CANBUILD]` lists the computer players' (`UnitType::ai_build_options`): a player's
/// constructors on a duty build what their menus offer; 92, a cell a map marks as a cliff
/// (`ta_formats::tnt::CELL_CLIFF`: a map converted from Supreme Commander) is as steep as a
/// slope can be; 93, a piercing shot (TA's `noexplode`: the Commanders' D-gun) harms what stands
/// within its area of effect all along its flight, units and features, from once it is that far
/// from the gun, and runs on along the ground or the water it meets.
pub const SIMULATION_VERSION: u32 = 94;
/// Records shorter than one year of simulated time are supported. Shared with the
/// launcher's metadata reader so impossible durations cannot start endless save catch-up.
pub const MAX_RECORD_TICKS: u64 = 30 * 3600 * 24 * 365;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Original Total Annihilation install; `None` = auto-detect (GOG registry, Steam, defaults).
    pub game_dir: Option<PathBuf>,
    pub graphics: Graphics,
    pub audio: Audio,
    pub mods: Mods,
    pub maps: maps::Maps,
    pub gameplay: Gameplay,
    pub keys: Keys,
    pub updates: Updates,
    pub launcher: Launcher,
    /// The last values this instance loaded or saved, used to merge edits from the game
    /// with changes made by the launcher. This is bookkeeping, never part of the file.
    #[serde(skip)]
    #[doc(hidden)]
    pub save_baseline: SaveBaseline,
}

#[derive(Default)]
pub struct SaveBaseline(std::sync::Mutex<Option<std::sync::Arc<toml::Value>>>);

impl Clone for SaveBaseline {
    fn clone(&self) -> Self {
        Self(std::sync::Mutex::new(self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()))
    }
}

impl std::fmt::Debug for SaveBaseline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SaveBaseline")
    }
}

// Change detection concerns settings, not when they were loaded or written.
impl PartialEq for SaveBaseline {
    fn eq(&self, _: &Self) -> bool { true }
}

/// Launcher preferences, including the seasonal theme shared by game loading screens.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Launcher {
    /// What the launcher does once the game has started.
    pub on_play: OnPlay,
    /// Seasonal decoration is enabled by default in the launcher and loading screens.
    pub seasonal_effects: bool,
    /// Automatic follows the calendar; a named theme can be enjoyed at any time.
    pub seasonal_theme: SeasonalTheme,
    /// Crisp contextual Reborn pointers; off preserves the system accessibility pointer.
    pub themed_cursor: bool,
    /// The window's inside when the launcher last closed (points), and whether it was
    /// maximised (as a handheld's first start puts it: [`Config::set_up_for`]); None: the size
    /// it comes at.
    pub window: Option<[f32; 2]>,
    pub maximized: bool,
    /// The newest version whose news the player has been shown (the launcher shows what is new
    /// once after an update); empty before the first start.
    pub seen_version: String,
}

impl Default for Launcher {
    fn default() -> Self {
        Self {
            on_play: OnPlay::default(),
            seasonal_effects: true,
            seasonal_theme: SeasonalTheme::default(),
            themed_cursor: true,
            window: None,
            maximized: false,
            seen_version: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum SeasonalTheme {
    #[default]
    Automatic,
    Christmas,
    NewYear,
    Halloween,
}

/// What the launcher does once the game has started.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum OnPlay {
    /// Stays as it is (the game comes up in front of it).
    #[default]
    Stay,
    /// Goes down to the taskbar, and comes back up when the game ends.
    Minimise,
    /// Closes.
    Close,
}

/// The launcher's update check.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Updates {
    /// Ask the update source for a newer version when the launcher starts.
    pub check: bool,
    /// The update source (a TOML file with `version` and `url`, or a GitHub "latest
    /// release" API address); empty: the build's own.
    pub url: String,
}

impl Default for Updates {
    fn default() -> Self {
        Updates { check: true, url: String::new() }
    }
}

/// Rebindable game keys: action id -> key ("A", "Ctrl+D", "Period"; egui's key names on a
/// US layout). Only changed actions are stored; the rest use [`Keys::ACTIONS`]' defaults.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Keys {
    pub bindings: std::collections::BTreeMap<String, String>,
}

impl Keys {
    /// Every rebindable action: (id, what it does, default key).
    pub const ACTIONS: &'static [(&'static str, &'static str, &'static str)] = &[
        // TA's own orders on TA's own keys (its GUI files' quick keys); TA Reborn's additions
        // on Alt and the letter they would have taken.
        ("attack", "Attack", "A"),
        ("move", "Move", "M"),
        ("fight", "Fight (attack-move)", "Alt+F"),
        ("patrol", "Patrol", "P"),
        ("guard", "Guard", "G"),
        ("stop", "Stop", "S"),
        ("dgun", "D-gun / launch", "D"),
        ("capture", "Capture", "C"),
        ("repair", "Repair", "R"),
        ("load", "Load", "L"),
        ("unload", "Unload", "U"),
        ("resurrect", "Resurrect", "Alt+R"),
        ("reclaim", "Reclaim", "E"),
        ("area_mex", "Extractors on every metal spot in an area", "Alt+X"),
        ("fire_orders", "Fire orders: hold fire, return fire, fire at will", "F"),
        ("move_orders", "Move orders: hold position, maneuver, roam", "V"),
        ("on_off", "On / off", "X"),
        ("duty", "Duty: the constructors work by themselves (economy, repair, reclaim, defence, then none)", "Alt+D"),
        ("cloak", "Cloak / decloak", "K"),
        ("self_destruct", "Self-destruct", "Ctrl+D"),
        ("select_all", "Select everything", "Ctrl+A"),
        ("select_commander", "Select the Commander", "Ctrl+C"),
        ("select_type", "Every unit of the selected types", "Ctrl+Z"),
        ("select_on_screen", "Every unit on the screen", "Ctrl+S"),
        // TA's selections by category: the units whose category says CTRL_B, CTRL_F...
        ("select_builders", "Every constructor (not the Commander)", "Ctrl+B"),
        ("select_factories", "Every factory", "Ctrl+F"),
        ("select_mines", "Every mine", "Ctrl+M"),
        ("select_armed_aircraft", "Every aircraft with weapons", "Ctrl+P"),
        ("select_radar", "Every radar, jammer and sonar", "Ctrl+R"),
        ("select_aircraft", "Every aircraft", "Ctrl+V"),
        ("select_armed", "Every armed unit that moves (not the Commander)", "Ctrl+W"),
        ("idle_builder", "Next idle constructor", "Period"),
        ("idle_factory", "Next idle factory", "Comma"),
        ("follow", "Follow the selected unit", "T"),
        ("next_unit", "Look at the next of your units off the screen (of the selection, if any)", "N"),
        ("perspective", "Perspective: into the selected unit (Escape leaves)", "Q"),
        // TA's function keys: F1 information, F2 the menu, F3 the last report; TA Reborn's
        // map overlays on Alt and the key they would have taken.
        ("last_alert", "Look at the last attack", "F3"),
        ("pause", "Pause", "Space"),
        ("faster", "Faster", "Equals"),
        ("slower", "Slower", "Minus"),
        ("chat", "Chat (with Shift: to allies)", "Enter"),
        ("jukebox", "Jukebox: the music, your own too", "J"),
        ("clear_messages", "Clear the messages and the chat", "Delete"),
        ("keyboard_pointer", "Toggle keyboard pointer controls", "Alt+K"),
        ("event_history", "Battle event history", "Alt+H"),
        ("players", "Players panel (hold)", "Tab"),
        ("share", "Share resources and units with an ally", "H"),
        ("health_bars", "Health bars: damaged units, every damaged unit, the selection", "Backtick"),
        ("quick_save", "Quick save (single player)", "Ctrl+Shift+S"),
        ("cheats", "Cheats (single player)", "Ctrl+Alt+C"),
        ("scroll_up", "Scroll up (Ctrl: tilt)", "Up"),
        ("scroll_down", "Scroll down (Ctrl: tilt)", "Down"),
        ("scroll_left", "Scroll left (Ctrl: orbit)", "Left"),
        ("scroll_right", "Scroll right (Ctrl: orbit)", "Right"),
        ("tilt_up", "Tilt the camera up", "PageUp"),
        ("tilt_down", "Tilt the camera down", "PageDown"),
        ("reset_view", "Reset the view", "Home"),
        ("camera_style", "Modern / classic camera", "F5"),
        ("overlay_path", "Map: where the selected units can go", "Alt+F2"),
        ("overlay_height", "Map: heights", "Alt+F3"),
        ("overlay_metal", "Map: metal", "F4"),
        ("overlay_sight", "Map: sight and radar", "Alt+L"),
        ("help", "Information on the selected unit (with none, the controls reference)", "F1"),
        ("game_menu", "Menu: save, options, statistics", "F2"),
        ("screenshot", "Screenshot", "F12"),
        ("photo_mode", "Photo mode", "F11"),
        ("companion", "Companion screen: the map and intel in a window of their own (on a second display)", "Alt+C"),
        ("quit", "Quit", "F10"),
    ];

    /// The key bound to an action (its default unless changed).
    pub fn get(&self, action: &str) -> &str {
        match self.bindings.get(action) {
            Some(k) => k,
            None => Self::ACTIONS.iter().find(|a| a.0 == action).map_or("", |a| a.2),
        }
    }

    /// Binds an action; binding its default key again forgets the change.
    pub fn set(&mut self, action: &str, key: &str) {
        if Self::ACTIONS.iter().any(|a| a.0 == action && a.2 == key) {
            self.bindings.remove(action);
        } else {
            self.bindings.insert(action.to_string(), key.to_string());
        }
    }

    /// Other actions bound to the same key as `action`.
    pub fn conflicts(&self, action: &str) -> Vec<&'static str> {
        let key = self.get(action);
        Self::ACTIONS.iter().filter(|a| a.0 != action && self.get(a.0).eq_ignore_ascii_case(key)).map(|a| a.1).collect()
    }
}

/// Single-player campaign progress. Kept in its own file (`progress.toml`) so the launcher,
/// which rewrites `config.toml`, can never clobber it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CampaignProgress {
    /// Missions completed per campaign (by campaign file stem); the next one is unlocked.
    pub completed: std::collections::BTreeMap<String, usize>,
    /// Last difficulty chosen ("Easy", "Medium", "Hard").
    pub difficulty: String,
    /// Tutorial lessons completed (by index).
    pub tutorials: Vec<u8>,
    /// Scenario records, by the scenario's id.
    pub scenarios: std::collections::BTreeMap<String, ScenarioRecord>,
}

/// A scenario's record: the difficulties it was won on, and the best time on each (seconds:
/// the fastest win; for an endless one, the longest held).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ScenarioRecord {
    pub won: Vec<String>,
    pub best: std::collections::BTreeMap<String, u32>,
}

impl CampaignProgress {
    /// Index of the furthest playable mission of a campaign.
    pub fn unlocked(&self, campaign: &str) -> usize {
        self.completed.get(campaign).copied().unwrap_or(0)
    }

    /// Records a completed mission (never moves progress backwards).
    pub fn complete(&mut self, campaign: &str, index: usize) {
        let e = self.completed.entry(campaign.to_string()).or_insert(0);
        *e = (*e).max(index + 1);
    }

    /// Notes a scenario played to its end on a difficulty ("easy", "normal", "hard"), won or
    /// lost after `secs`: an endless one keeps the longest time held, the others the fastest
    /// win. Returns whether that is a new best.
    pub fn record_scenario(&mut self, id: &str, difficulty: &str, won: bool, secs: u32, endless: bool) -> bool {
        let r = self.scenarios.entry(id.to_string()).or_default();
        if won && !r.won.iter().any(|d| d == difficulty) {
            r.won.push(difficulty.to_string());
        }
        if !won && !endless {
            return false;
        }
        let best = r.best.get(difficulty).copied();
        let better = match best {
            None => true,
            Some(b) if endless => secs > b,
            Some(b) => secs < b,
        };
        if better {
            r.best.insert(difficulty.to_string(), secs);
        }
        better
    }

    /// Records a completed tutorial lesson.
    pub fn complete_tutorial(&mut self, lesson: usize) {
        let lesson = lesson.min(u8::MAX as usize) as u8;
        if !self.tutorials.contains(&lesson) {
            self.tutorials.push(lesson);
            self.tutorials.sort_unstable();
        }
    }

    pub fn path() -> PathBuf {
        player_data_dir().join("progress.toml")
    }

    /// Loads progress; a missing or unreadable file means a fresh start.
    pub fn load() -> CampaignProgress {
        std::fs::read_to_string(Self::path()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        write_atomic(&Self::path(), &toml::to_string_pretty(self).map_err(std::io::Error::other)?)
    }

    /// Apply completed activity to the latest progress under a stable file lock.
    /// Callers retain operations rather than a stale whole-file snapshot while retrying.
    /// Unreadable progress is preserved and reported instead of being reset.
    pub fn update_at(path: &Path, update: impl FnOnce(&mut Self)) -> std::io::Result<Self> {
        let _transaction = lock_settings(path)?;
        let mut progress = match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str::<Self>(&text).map_err(std::io::Error::other)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(error),
        };
        update(&mut progress);
        write_atomic(path, &toml::to_string_pretty(&progress).map_err(std::io::Error::other)?)?;
        Ok(progress)
    }
}

/// Writes a file via a temporary name and a rename, so readers never see half a file and a
/// crash mid-write keeps the previous version.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    write_atomic_bytes(path, text.as_bytes())
}

/// Binary counterpart used by saved battles and map files. Each writer owns its
/// exclusively created temporary file, including across simultaneous game processes.
pub fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let filename = path.file_name().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "settings path has no file name"))?;
    // Launcher and game can save at once. A shared config.tmp lets one writer
    // replace another's unfinished contents or remove its temporary file. Exclusive
    // creation also skips abandoned files from an earlier process with the same PID.
    let (tmp, mut file) = loop {
        let mut name = filename.to_os_string();
        name.push(format!(".{}-{}.tmp", std::process::id(), NEXT_TEMP.fetch_add(1, Ordering::Relaxed)));
        let tmp = path.with_file_name(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // Same-directory rename retains atomic replacement. Windows can briefly deny
        // replacement while another writer or a file scanner has the destination open.
        // Retrying publication keeps this writer's complete, already-synced temp intact.
        publish_atomic(&tmp, path)
    })();
    if result.is_err() { let _ = std::fs::remove_file(&tmp); }
    result
}

fn publish_atomic(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(not(windows))]
    { std::fs::rename(from, to) }
    #[cfg(windows)]
    {
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut delay = Duration::from_millis(1);
        loop {
            match std::fs::rename(from, to) {
                Ok(()) => return Ok(()),
                Err(error) => {
                    // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
                    // Actual permissions and wrong-kind destinations cannot heal by waiting.
                    if !matches!(error.raw_os_error(), Some(5 | 32 | 33))
                        || Instant::now() >= deadline
                        || std::fs::metadata(to).is_ok_and(|m| m.is_dir() || m.permissions().readonly()) {
                        return Err(error);
                    }
                    std::thread::sleep(delay.min(deadline.saturating_duration_since(Instant::now())));
                    delay = (delay * 2).min(Duration::from_millis(25));
                }
            }
        }
    }
}

/// Why a config file could not be loaded.
#[derive(Debug)]
pub enum LoadError {
    Missing,
    Unreadable(std::io::Error),
    Invalid(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Missing => write!(f, "no settings file yet"),
            LoadError::Unreadable(e) => write!(f, "settings file unreadable: {e}"),
            LoadError::Invalid(e) => write!(f, "settings file invalid: {e}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowMode {
    #[default]
    Windowed,
    Borderless,
    Fullscreen,
}

/// When units show health bars (selected and hovered units always do).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum HealthBars {
    /// Our damaged units and every nanoframe, when zoomed in.
    #[default]
    Auto,
    /// Every damaged unit in sight, and every nanoframe, at any zoom.
    Damaged,
    /// Only selected and hovered units.
    Selected,
}

/// What plays behind the game's title menu.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum MenuBackdrop {
    /// A film: short scenes staged on TA Reborn's own worlds, cut together.
    #[default]
    Film,
    /// A battle between computer players, as it plays.
    Battle,
}

/// When the coach gives advice in battle (a store running dry, a factory waiting, the
/// Commander in danger).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Coach {
    /// In a profile's first few battles.
    #[default]
    FirstBattles,
    Always,
    Off,
}

/// Which mouse button gives orders in battle.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum OrderButton {
    /// The left button selects, the right one moves, attacks, repairs... by context.
    #[default]
    Right,
    /// TA's original interface: the left button selects and gives orders, the right one
    /// clears the selection (and cancels a pending order).
    Left,
}

/// How the right button opens the radial menu on one of your units (a middle click on the unit
/// opens it whichever this is, and a controller's B is the right button).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum RadialOpens {
    /// A right-click where it would give no order (nothing selected, or the unit in the
    /// selection; with the left button giving orders, any time), or the button held on the unit
    /// with the pointer still.
    #[default]
    ClickOrHold,
    /// Only the button held on the unit, the pointer still: a right-click keeps its other uses.
    Hold,
}

/// The world mirrored in the water (perspective camera), and at what resolution.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Reflections {
    /// The water reflects only the sky.
    Off,
    /// At half the screen resolution.
    #[default]
    Half,
    /// At the full screen resolution.
    Full,
}

/// The time of day over the battlefield (modern camera; the classic view keeps TA's light).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum TimeOfDay {
    /// TA's own light, the light its maps are painted for.
    #[default]
    TaDay,
    Dawn,
    Dusk,
    Night,
    /// A day and a night pass every [`TimeOfDay::CYCLE_MINUTES`] minutes of play.
    Cycle,
}

impl TimeOfDay {
    /// Minutes of play a day and a night take in the cycle.
    pub const CYCLE_MINUTES: f32 = 20.0;

    /// The clock (hours) a fixed time of day shows; `None` for TA's own light and the cycle.
    pub fn fixed_hours(self) -> Option<f32> {
        match self {
            TimeOfDay::Dawn => Some(6.4),
            TimeOfDay::Dusk => Some(17.75),
            TimeOfDay::Night => Some(0.0),
            TimeOfDay::TaDay | TimeOfDay::Cycle => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum CameraStyle {
    /// Perspective camera with free rotation and tilt.
    #[default]
    Modern,
    /// TA's original oblique projection.
    Classic,
}

/// The visual direction for units, buildings and worlds, independent of camera and quality.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtStyle {
    /// Original TA designs with richer source-faithful geometry and materials.
    #[default]
    Reborn,
    /// Richer detail following the original silhouettes and colours.
    Classic,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Quality {
    Low,
    Medium,
    High,
    Ultra,
}

/// A graphics level: the options it sets together (see [`Preset::apply`]).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Preset {
    /// Picked for the graphics card when the game first starts.
    #[default]
    Auto,
    /// For integrated graphics and older cards: no extra passes, the scene drawn at 75% of
    /// the window's resolution.
    Low,
    Medium,
    High,
    Ultra,
    /// The options were set one by one.
    Custom,
}

/// What kind of graphics device the game runs on (for [`Preset::for_gpu`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuKind {
    /// A software renderer (WARP, llvmpipe).
    Software,
    Integrated,
    Discrete,
    Other,
}

/// How many of Apple's base chips' graphics one named `lowercase` stands for (its Pro chips
/// have about twice their graphics cores, its Max chips four times, its Ultras eight); 1 for
/// any other.
fn apple_class(lowercase: &str) -> f64 {
    if !lowercase.starts_with("apple") {
        1.0
    } else if lowercase.contains("ultra") {
        8.0
    } else if lowercase.contains(" max") {
        4.0
    } else if lowercase.contains(" pro") {
        2.0
    } else {
        1.0
    }
}

impl Preset {
    /// The levels a player picks from.
    pub const LEVELS: [Preset; 4] = [Preset::Low, Preset::Medium, Preset::High, Preset::Ultra];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Auto => "AUTO",
            Preset::Low => "LOW",
            Preset::Medium => "MEDIUM",
            Preset::High => "HIGH",
            Preset::Ultra => "ULTRA",
            Preset::Custom => "CUSTOM",
        }
    }

    /// The level for a graphics device: High on dedicated cards and Apple's Max and Ultra
    /// chips, Medium on recent integrated graphics and the Steam Deck, Low on older ones and
    /// software renderers.
    pub fn for_gpu(kind: GpuKind, name: &str) -> Preset {
        let n = name.to_ascii_lowercase();
        let has = |keys: &[&str]| keys.iter().any(|k| n.contains(k));
        match kind {
            GpuKind::Software => Preset::Low,
            // The Steam Deck's graphics, of the Radeon 660M's class, however its driver reports
            // them (Windows on a Deck gives them a gigabyte of their own: a dedicated card's).
            _ if hardware::steam_deck_gpu(&n) => Preset::Medium,
            GpuKind::Integrated if apple_class(&n) >= 4.0 => Preset::High,
            // Iris Xe, Arc and the Radeon 600M-800M series are fast enough for Medium.
            GpuKind::Integrated if has(&["xe graphics", "arc(", "arc graphics", "610m", "660m", "680m", "740m", "760m", "780m", "880m", "890m", "8060s", "apple"]) => Preset::Medium,
            GpuKind::Integrated => Preset::Low,
            // Entry-level dedicated cards of their day.
            GpuKind::Discrete if has(&["geforce gt ", "geforce mx", "geforce 9", "geforce 7", "radeon r5", "radeon r7", "radeon hd"]) => Preset::Medium,
            GpuKind::Discrete => Preset::High,
            GpuKind::Other => Preset::Medium,
        }
    }

    /// On integrated graphics (Intel's, AMD's and Apple's, which share the computer's memory),
    /// the most pixels this level draws the battlefield at: 1080p's (1440p's at Ultra); twice
    /// that on Apple's Pro chips, four times on its Max chips and eight on its Ultras (a level's
    /// own render scale may draw fewer still: Low's 75%). A Mac's Retina screen has 2.7 to 3.7
    /// times 1080p's pixels, more than such graphics draw the battlefield at in time (an M5
    /// MacBook Air played on Low alone); the interface keeps the screen's own sharpness. None
    /// on other graphics, Auto and Custom.
    pub fn pixel_budget(self, kind: GpuKind, name: &str) -> Option<f64> {
        if kind != GpuKind::Integrated {
            return None;
        }
        let pixels = match self {
            Preset::Low | Preset::Medium | Preset::High => 1920.0 * 1080.0,
            Preset::Ultra => 2560.0 * 1440.0,
            Preset::Auto | Preset::Custom => return None,
        };
        Some(pixels * apple_class(&name.to_ascii_lowercase()))
    }

    /// The render scale this level sets ([`Preset::apply`]), before fitting it to a screen.
    pub fn render_scale(self) -> Option<f32> {
        match self {
            Preset::Low => Some(0.75),
            Preset::Medium | Preset::High | Preset::Ultra => Some(1.0),
            Preset::Auto | Preset::Custom => None,
        }
    }

    /// Sets the options of this level (Auto and Custom leave them as they are).
    pub fn apply(self, g: &mut Graphics) {
        let (msaa, shadows, effects) = match self {
            Preset::Low => (1, Quality::Low, Quality::Low),
            Preset::Medium => (2, Quality::Medium, Quality::Medium),
            Preset::High => (4, Quality::High, Quality::High),
            Preset::Ultra => (8, Quality::Ultra, Quality::Ultra),
            Preset::Auto | Preset::Custom => return,
        };
        let Some(scale) = self.render_scale() else { return };
        let at_least = |level: Preset| self as u8 >= level as u8;
        g.msaa = msaa;
        g.shadows = shadows;
        g.effects = effects;
        g.render_scale = scale;
        g.bloom = at_least(Preset::Medium);
        g.weather = at_least(Preset::Medium);
        g.cloud_shadows = at_least(Preset::Medium);
        g.hd_surfaces = at_least(Preset::Medium);
        g.trees_3d = at_least(Preset::Medium);
        g.ambient_occlusion = at_least(Preset::High);
        g.heat_haze = at_least(Preset::High);
        g.water_reflections = match self {
            Preset::Ultra => Reflections::Full,
            Preset::High => Reflections::Half,
            _ => Reflections::Off,
        };
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Graphics {
    /// The level the options below were last set to; Custom once one was changed on its own.
    /// (Settings files from before presets keep their options: Custom.)
    #[serde(default = "custom_preset")]
    pub preset: Preset,
    /// The graphics card the game last started on (shown in the launcher).
    pub gpu: String,
    pub width: u32,
    pub height: u32,
    pub window_mode: WindowMode,
    pub vsync: bool,
    /// 1, 2, 4 or 8.
    pub msaa: u32,
    pub shadows: Quality,
    pub effects: Quality,
    pub bloom: bool,
    /// 0 = original art only; up to 1.5 = strong dynamic relighting of slopes.
    pub terrain_relight: f32,
    pub camera: CameraStyle,
    /// Art direction, kept when performance presets or camera mode change.
    pub art_style: ArtStyle,
    /// Local cosmetic pack for your units and allies. Loaded on launch; presets keep it.
    pub unit_skin: String,
    /// Enemy pack; empty follows `unit_skin`, preserving older global skin preferences.
    pub enemy_unit_skin: String,
    /// Frame rate cap when vsync is off (0 = unlimited).
    pub fps_limit: u32,
    /// With another window in front the game draws 30 frames a second (minimised, it draws
    /// none either way).
    pub save_power_in_background: bool,
    /// Weather over the battlefield (snow, embers, dust...).
    pub weather: bool,
    /// Soft shading where units, buildings and trees meet the ground, and in the creases
    /// of the terrain.
    pub ambient_occlusion: bool,
    /// The drifting clouds' shadows on the battlefield (perspective camera).
    pub cloud_shadows: bool,
    /// Hot air shimmering over explosions, fires and lava.
    pub heat_haze: bool,
    /// Shores, units, ships and explosions mirrored in the water.
    pub water_reflections: Reflections,
    /// HD surfaces (perspective camera): up close the ground's dithered pixels give way to
    /// smooth colour with fine grain and relief, and units gain worn edges, battle damage,
    /// reflections on metal and glowing lights.
    pub hd_surfaces: bool,
    /// 3D trees and rocks (modern camera): close to the ground TA Reborn's own models of trees,
    /// bushes and rocks stand in for TA's painted ones, swaying in the wind and casting shadows.
    pub trees_3d: bool,
    /// The battlefield is drawn at this fraction of the window's resolution (0.5 to 1) and
    /// scaled up; the interface stays sharp.
    pub render_scale: f32,
    /// The time of day (modern camera): TA's own light, dawn, dusk, night, or a cycle.
    pub time_of_day: TimeOfDay,
    /// Ray-traced shadows and ambient occlusion of units and buildings (modern camera; graphics
    /// cards with hardware ray tracing). Not part of the graphics levels; taken up when the game
    /// starts.
    pub ray_tracing: bool,
    /// Whether the graphics card the game last started on traces rays in hardware.
    pub gpu_ray_tracing: bool,
    /// TA's intro movies play on a CRT in a bedroom at night (off: full screen).
    pub intro_room: bool,
    /// The ground in the wind (modern camera, close up): grass waving, sand streaming, snow
    /// drifting. Only with HD surfaces and High or Ultra effects, and still with reduced motion;
    /// not part of the graphics levels (a matter of taste: those levels have it as their effects
    /// allow).
    pub ground_life: bool,
    /// The frame rate, and a frame's milliseconds, in a corner of the battlefield.
    pub show_frame_rate: bool,
    /// The game has fitted the level's render scale to its screen and graphics once
    /// ([`Graphics::fit_render_scale_once`]): at its first start, or the first after the update
    /// that brought the fitting.
    pub scale_fitted: bool,
    /// The first start has set the game up for this computer ([`Config::set_up_for`]: the
    /// screen's size, full screen, on a handheld the interface's scale). The level may be Auto
    /// after it: on Linux, where the first start knows no graphics card, the game picks it from
    /// the card it opens, as it does a level put back to Auto in the options, the rest left as
    /// the player has it. (Files from before the mark have a level of their own.)
    pub set_up: bool,
}

fn custom_preset() -> Preset {
    Preset::Custom
}

impl Graphics {
    pub fn enemy_skin(&self) -> &str {
        if self.enemy_unit_skin.is_empty() { &self.unit_skin } else { &self.enemy_unit_skin }
    }

    /// Whether the options a level sets ([`Preset::apply`]) differ between the two.
    pub fn level_options_differ(&self, other: &Graphics) -> bool {
        let options = |g: &Graphics| {
            (g.msaa, g.shadows, g.effects, g.render_scale.to_bits(), g.bloom, g.weather, g.cloud_shadows, g.hd_surfaces, g.trees_3d, g.ambient_occlusion, g.heat_haze, g.water_reflections)
        };
        options(self) != options(other)
    }

    /// The level the options match, or Custom. A render scale below the level's own still
    /// matches it: the level fitted to a screen of more pixels than its graphics draw
    /// ([`Graphics::fit_render_scale`]), or lowered for speed alone.
    pub fn matching_preset(&self) -> Preset {
        Preset::LEVELS
            .into_iter()
            .find(|&p| {
                let mut g = self.clone();
                p.apply(&mut g);
                g.render_scale = self.render_scale.min(g.render_scale);
                g == *self
            })
            .unwrap_or(Preset::Custom)
    }

    /// The pixels of the game's picture with these settings: the screen's own in full screen
    /// (`native`, when known), else the window's.
    pub fn picture_pixels(&self, native: Option<(u32, u32)>) -> u64 {
        let (w, h) = match (self.window_mode, native) {
            (WindowMode::Borderless | WindowMode::Fullscreen, Some(native)) => native,
            _ => (self.width, self.height),
        };
        w as u64 * h as u64
    }

    /// Sets the render scale of the level the options are at ([`Graphics::preset`]) for a
    /// picture of `pixels` on graphics of `kind` named `name`: the level's own, or as much less
    /// as draws the battlefield at no more than the level's [`Preset::pixel_budget`] there (on
    /// the options' steps of 5%, half at the least). Other levels and graphics: as it is.
    pub fn fit_render_scale(&mut self, kind: GpuKind, name: &str, pixels: u64) {
        let (Some(budget), Some(own)) = (self.preset.pixel_budget(kind, name), self.preset.render_scale()) else { return };
        let fitted = (budget / pixels.max(1) as f64).sqrt() as f32;
        let stepped = (fitted * 20.0 + 1e-3).floor() / 20.0;
        let lo = *limits::RENDER_SCALE.start();
        self.render_scale = stepped.clamp(lo, own);
    }

    /// Once for the settings ([`Graphics::scale_fitted`]), as the game starts with its window
    /// and device: [`Graphics::fit_render_scale`], where the render scale is the level's own (a
    /// scale set by hand, and Custom, are kept). The launcher picks the first level before the
    /// game has a window, and on a Mac without knowing the screen; settings from before the
    /// fitting are fitted too. True when the settings changed (if only the mark).
    pub fn fit_render_scale_once(&mut self, kind: GpuKind, name: &str, pixels: u64) -> bool {
        if self.scale_fitted {
            return false;
        }
        self.scale_fitted = true;
        if self.preset.render_scale() == Some(self.render_scale) {
            self.fit_render_scale(kind, name, pixels);
        }
        true
    }
}

impl Default for Graphics {
    fn default() -> Self {
        Graphics {
            preset: Preset::Auto,
            gpu: String::new(),
            width: 1600,
            height: 900,
            window_mode: WindowMode::Windowed,
            vsync: true,
            msaa: 4,
            shadows: Quality::High,
            effects: Quality::High,
            bloom: true,
            terrain_relight: 0.35,
            camera: CameraStyle::Modern,
            art_style: ArtStyle::Reborn,
            unit_skin: "reborn".to_string(),
            enemy_unit_skin: String::new(),
            fps_limit: 0,
            save_power_in_background: true,
            weather: true,
            ambient_occlusion: true,
            cloud_shadows: true,
            heat_haze: true,
            water_reflections: Reflections::Half,
            hd_surfaces: true,
            trees_3d: true,
            render_scale: 1.0,
            time_of_day: TimeOfDay::TaDay,
            ray_tracing: false,
            gpu_ray_tracing: false,
            intro_room: true,
            ground_life: true,
            show_frame_rate: false,
            scale_fitted: false,
            set_up: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Audio {
    /// Linear gains in 0..=1.
    pub master: f32,
    pub music: f32,
    pub effects: f32,
    pub voice: f32,
    /// Maximum simultaneous sound effects.
    pub max_voices: u32,
    /// The player's own music: how it plays with TA's.
    pub user_music: UserMusic,
    /// Another folder of the player's music, besides `music` in the settings folder.
    pub music_folder: Option<PathBuf>,
    /// The launcher plays TA's interface sounds.
    pub launcher_sounds: bool,
    /// The launcher plays TA's calmer music, quietly: at this share of the music volume.
    pub launcher_music: bool,
    pub launcher_music_volume: f32,
    /// The jukebox: the next track at random (else the playlist's next), and each track again
    /// once it has played to its end.
    pub music_shuffle: bool,
    pub music_repeat: bool,
    /// The game is silent while another window is in front.
    pub mute_in_background: bool,
    /// Units answer when selected and given orders (TA's select, ok and can't); off: they keep
    /// quiet, while alerts, the self-destruct count, briefings and the tutorial still speak.
    pub unit_replies: bool,
}

impl Default for Audio {
    fn default() -> Self {
        Audio {
            master: 0.8,
            music: 0.6,
            effects: 0.9,
            voice: 1.0,
            max_voices: 64,
            user_music: UserMusic::Mixed,
            music_folder: None,
            launcher_sounds: true,
            launcher_music: true,
            launcher_music_volume: 0.45,
            music_shuffle: true,
            music_repeat: false,
            mute_in_background: false,
            unit_replies: true,
        }
    }
}

/// How the player's own music plays (the jukebox): not at all, mixed in with TA's, or in its
/// place. TA's victory and defeat stay.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum UserMusic {
    Off,
    #[default]
    Mixed,
    Only,
}

/// The audio files the player's music is in: MP3, Ogg, FLAC, WAV, M4A.
pub const MUSIC_EXTENSIONS: [&str; 6] = ["mp3", "ogg", "flac", "wav", "m4a", "aac"];

/// The folders the player's music is looked for in: `music` in the settings folder, and the
/// one chosen.
pub fn music_folders(audio: &Audio) -> Vec<PathBuf> {
    let mut out = vec![config_dir().join("music")];
    if let Some(dir) = audio.music_folder.clone().filter(|d| !out.contains(d)) {
        out.push(dir);
    }
    out
}

/// The player's music files, in those folders and the folders inside them (sorted by path).
pub fn music_files(audio: &Audio) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 4 {
                    walk(&p, depth + 1, out);
                }
            } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| MUSIC_EXTENSIONS.iter().any(|m| x.eq_ignore_ascii_case(m))) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for dir in music_folders(audio) {
        walk(&dir, 0, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Mods {
    /// Enabled mod ids in user-chosen load order (later entries win), in lower case as mods'
    /// own are read (`ta_vfs::mods::ModManifest::id`), each once.
    #[serde(deserialize_with = "mod_ids")]
    pub enabled: Vec<String>,
    /// Extra folder to scan for mods besides `<app dir>/mods` and `<config dir>/mods`.
    pub extra_dir: Option<PathBuf>,
    /// Mods the player hid from the launcher's Mods library (ids as `enabled`'s): the
    /// catalogue's offers of no interest to them, or any of their own. Hiding only takes a mod
    /// out of the library's lists and counts: one installed stays installed, and one that is on
    /// still loads (the load order lists it).
    #[serde(deserialize_with = "mod_ids")]
    pub hidden: Vec<String>,
}

impl Mods {
    /// Whether the mod `id` is hidden from the library.
    pub fn is_hidden(&self, id: &str) -> bool {
        let id = id.to_lowercase();
        self.hidden.iter().any(|hidden| *hidden == id)
    }

    /// Hides the mod `id` from the library (once), or puts it back.
    pub fn set_hidden(&mut self, id: &str, hidden: bool) {
        let id = id.to_lowercase();
        self.hidden.retain(|other| *other != id);
        if hidden {
            self.hidden.push(id);
        }
    }
}

/// Mod ids as read from the settings: in lower case, each once, in the order first given.
fn mod_ids<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let mut ids: Vec<String> = Vec::new();
    for id in <Vec<String> as Deserialize>::deserialize(d)? {
        let id = id.to_lowercase();
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// A multiplayer game joined lately: its address, and the battle's name as last seen.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RecentGame {
    pub address: String,
    pub title: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Gameplay {
    /// Per-player unit cap (TA: 250; TA Reborn supports far more).
    pub unit_limit: u32,
    /// Game speed multiplier applied at start (1.0 = TA's 30 ticks per second).
    pub speed: f32,
    /// Edge-of-screen scrolling.
    pub edge_scroll: bool,
    pub scroll_speed: f32,
    pub player_name: String,
    /// Size of the interface (menus, HUD) relative to the system scale.
    pub ui_scale: f32,
    /// Single-player games save themselves every five minutes of game time.
    pub autosave: bool,
    pub health_bars: HealthBars,
    /// Far out, each unit also shows as an icon in its owner's colour (strategic zoom); off: the
    /// units alone at every height.
    pub strategic_icons: bool,
    /// TA's own animated cursors in battle (off: the system cursor).
    pub ta_cursors: bool,
    /// TA's 1997 in-game interface (side panel, resource bar, build menus) in place of
    /// the modern HUD.
    pub classic_interface: bool,
    /// The classic interface's art and lettering in TA's own square pixels, enlarged as they
    /// are (off: smoothed for the screen, sharp at any size).
    pub classic_pixels: bool,
    /// Player colours, health bars and map marks in colours that stay apart with colour
    /// blindness (units tinted to match).
    pub colour_blind: bool,
    /// Add colour-independent player numbers and relation marks to visible units.
    pub team_markers: bool,
    /// Navigate and click the battlefield pointer with the keyboard.
    pub keyboard_pointer: bool,
    /// Let an active screen reader announce selection changes and battle notices.
    pub screen_reader_announcements: bool,
    /// The tutorial reads its instructions and hints aloud.
    pub tutorial_voice: bool,
    /// Reduce decorative motion, camera shake and cockpit movement.
    pub reduced_motion: bool,
    /// Reduce bright combat flashes and pulsing warnings (not a photosensitivity guarantee).
    pub reduced_flashes: bool,
    /// Big blasts shake the view (TA's `+noshake` stops it for one battle; reduced motion leaves
    /// it out too).
    pub screen_shake: bool,
    /// Size of TA's battle cursor, independent of interface scaling.
    pub cursor_scale: f32,
    /// A black-and-white marker around the battle cursor's hotspot.
    pub high_contrast_cursor: bool,
    /// Multiplier for the time battle notices stay visible.
    pub message_duration: f32,
    /// The interface's language: a `lang/<code>.toml` file's code, "" for English.
    pub language: String,
    /// The address last typed and joined on the Multiplayer page ("" none yet).
    pub join_address: String,
    /// The port last hosted on (0: the game's own), and the battle's name then ("": the
    /// player's own, "Name's battle").
    pub host_port: u16,
    pub host_title: String,
    /// The games joined lately, the latest first.
    pub recent_games: Vec<RecentGame>,
    /// Gamepads (read at start).
    pub gamepad: bool,
    pub gamepad_deadzone: f32,
    pub gamepad_sensitivity: f32,
    /// Which mouse button gives orders.
    pub order_button: OrderButton,
    /// The radial menu: a ring of a unit's orders round the pointer, opened on one of your units
    /// (by the right button as [`Gameplay::radial_opens`] says, a middle click, a controller's B).
    /// On by default, as it has been since it came: a settings file without the key (written
    /// before it, or never changed) has it on, and only a `false` the player chose turns it off,
    /// for good.
    pub radial_menu: bool,
    /// How the right button opens the radial menu; a settings file from before the choice keeps
    /// what the button always did (a click or a hold).
    pub radial_opens: RadialOpens,
    /// In single-player battles the enemy's computer players talk on the chat.
    pub ai_chatter: bool,
    /// When the coach gives advice in battle.
    pub coach: Coach,
    /// A tip at the start of each battle (a tip can be put away, and turns tips off from itself).
    pub start_tips: bool,
    /// The coach's advice the player asked never to hear again (the names it keeps them by).
    pub coach_muted: Vec<String>,
    /// Battles open the companion screen (the map and intel) on a second display, when there
    /// is one.
    pub companion: bool,
    /// What plays behind the title menu.
    pub menu_backdrop: MenuBackdrop,
}

impl Default for Gameplay {
    fn default() -> Self {
        Gameplay {
            unit_limit: 1500,
            speed: 1.0,
            edge_scroll: true,
            scroll_speed: 1.0,
            player_name: "Commander".into(),
            ui_scale: 1.0,
            autosave: true,
            health_bars: HealthBars::Auto,
            strategic_icons: true,
            ta_cursors: true,
            classic_interface: false,
            classic_pixels: false,
            colour_blind: false,
            team_markers: false,
            keyboard_pointer: false,
            screen_reader_announcements: true,
            tutorial_voice: true,
            reduced_motion: false,
            reduced_flashes: false,
            screen_shake: true,
            cursor_scale: 1.0,
            high_contrast_cursor: false,
            message_duration: 1.0,
            language: String::new(),
            join_address: String::new(),
            host_port: 0,
            host_title: String::new(),
            recent_games: Vec::new(),
            gamepad: true,
            gamepad_deadzone: 0.18,
            gamepad_sensitivity: 1.0,
            order_button: OrderButton::Right,
            radial_menu: true,
            radial_opens: RadialOpens::ClickOrHold,
            ai_chatter: true,
            coach: Coach::FirstBattles,
            start_tips: true,
            coach_muted: Vec::new(),
            companion: false,
            menu_backdrop: MenuBackdrop::Film,
        }
    }
}

/// Value ranges the settings screens offer. Loaded values are clamped to the same ranges
/// (see [`Config::sanitize`]), so that merely showing a slider never changes a value.
pub mod limits {
    use std::ops::RangeInclusive;
    pub const GAMEPAD_DEADZONE: RangeInclusive<f32> = 0.0..=0.5;
    pub const GAMEPAD_SENSITIVITY: RangeInclusive<f32> = 0.25..=3.0;

    pub const WIDTH: RangeInclusive<u32> = 640..=16384;
    pub const HEIGHT: RangeInclusive<u32> = 480..=16384;
    pub const FPS_LIMIT: RangeInclusive<u32> = 0..=480;
    pub const MSAA: [u32; 4] = [1, 2, 4, 8];
    pub const RENDER_SCALE: RangeInclusive<f32> = 0.5..=1.0;
    pub const TERRAIN_RELIGHT: RangeInclusive<f32> = 0.0..=1.5;
    pub const VOLUME: RangeInclusive<f32> = 0.0..=1.0;
    pub const MAX_VOICES: RangeInclusive<u32> = 8..=256;
    pub const UNIT_LIMIT: RangeInclusive<u32> = 50..=20_000;
    pub const UI_SCALE: RangeInclusive<f32> = 0.75..=2.0;
    /// The least room (points, width and height) the battlefield and the game's menus are laid
    /// out for: the game shows its interface no larger than leaves them this much.
    pub const GAME_POINTS: (f32, f32) = (1024.0, 600.0);
    /// The least the game's options and Conquest are laid out for (they reflow and scroll, so
    /// they keep a larger scale than the rest).
    pub const OPTIONS_POINTS: (f32, f32) = (800.0, 480.0);
    /// The least the launcher's window and pages are laid out for.
    pub const LAUNCHER_POINTS: (f32, f32) = (940.0, 600.0);
    pub const CURSOR_SCALE: RangeInclusive<f32> = 1.0..=2.5;
    pub const MESSAGE_DURATION: RangeInclusive<f32> = 1.0..=4.0;
    pub const SPEED: RangeInclusive<f32> = 0.25..=4.0;
    pub const SCROLL_SPEED: RangeInclusive<f32> = 0.25..=4.0;
    pub const PLAYER_NAME_CHARS: usize = 24;

    /// The largest interface scale on the settings' 5% steps at which a screen of `pixels` (at
    /// the system's own scale) still leaves `points` of room ([`GAME_POINTS`],
    /// [`LAUNCHER_POINTS`]); 100% where it leaves less, and no more than the settings offer.
    pub fn ui_scale_to_fit(pixels: (u32, u32), points: (f32, f32)) -> f32 {
        let fit = (pixels.0 as f32 / points.0).min(pixels.1 as f32 / points.1);
        let stepped = (fit * 20.0 + 1e-3).floor() / 20.0;
        stepped.clamp(1.0, *UI_SCALE.end())
    }
}

/// The folder under the game's old name, in use for this run because it could not be moved
/// (see [`adopt_old_folder`]).
static OLD_IN_USE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Per-user configuration directory. (Looking it up changes nothing: the folder under the
/// game's old name moves only when a program starts, by [`adopt_old_folder`].)
pub fn config_dir() -> PathBuf {
    if let Some(root) = profiles::context_root() { return root; }
    match OLD_IN_USE.get() {
        Some(old) => old.clone(),
        None => config_dirs().0,
    }
}

/// The game was called TA Remastered up to 1.9: moves the folder kept under that name
/// (settings, campaign progress, saved games, replays, mods, logs) to the new one, unless that
/// exists already. The launcher and the game call it first thing. While the folder cannot be
/// moved (a file in it held open), it is used where it is for this run. Returns what was done,
/// for the log (not open yet when it runs).
pub fn adopt_old_folder() -> Option<String> {
    let (dir, old) = config_dirs();
    let old = old?;
    match move_folder(&old, &dir) {
        Ok(false) => None,
        Ok(true) => Some(format!("moved {} to {}", old.display(), dir.display())),
        Err(e) => {
            let note = format!("could not move {} to {} ({e}); using it where it is", old.display(), dir.display());
            let _ = OLD_IN_USE.set(old);
            Some(note)
        }
    }
}

/// The configuration directory under the game's name, and under its old one.
fn config_dirs() -> (PathBuf, Option<PathBuf>) {
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let base = PathBuf::from(appdata);
            return (base.join(APP_DIR_NAME), Some(base.join(OLD_APP_DIR_NAME)));
        }
    }
    let base = if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".config")
    } else {
        return (PathBuf::from("."), None);
    };
    (base.join("ta-reborn"), Some(base.join("ta-remastered")))
}

/// Moves `old` to `dir` if only `old` exists; true when it did.
fn move_folder(old: &Path, dir: &Path) -> std::io::Result<bool> {
    if dir.exists() || !old.is_dir() {
        return Ok(false);
    }
    std::fs::rename(old, dir).map(|()| true)
}

/// The pinned local player's personal files; legacy storage until onboarding completes.
pub fn player_data_dir() -> PathBuf {
    profiles::context_data_dir().unwrap_or_else(config_dir)
}

pub fn config_path() -> PathBuf {
    player_data_dir().join(CONFIG_FILE)
}

/// Photos and screenshots belong to the pinned player under
/// `Pictures/TA Reborn/profiles/<id>`. Legacy pictures remain in the flat app folder.
/// An explicit `TA_PICTURES` override is used exactly as supplied, including captures.
pub fn pictures_dir() -> PathBuf {
    pictures_path(user_pictures().join(APP_DIR_NAME), std::env::var_os("TA_PICTURES").map(PathBuf::from),
        profiles::current_profile_id().as_deref())
}

/// Films made from replays: `Videos/TA Reborn`, where they are found to be shared (`TA_VIDEOS`
/// overrides it, exactly as supplied).
pub fn videos_dir() -> PathBuf {
    match std::env::var_os("TA_VIDEOS").map(PathBuf::from).filter(|d| !d.as_os_str().is_empty()) {
        Some(dir) => dir,
        None => user_videos().join(APP_DIR_NAME),
    }
}

fn pictures_path(legacy: PathBuf, override_dir: Option<PathBuf>, profile_id: Option<&str>) -> PathBuf {
    if let Some(dir) = override_dir.filter(|dir| !dir.as_os_str().is_empty()) { return dir; }
    profile_id.map_or_else(|| legacy.clone(), |id| legacy.join("profiles").join(id))
}

#[cfg(windows)]
fn user_pictures() -> PathBuf {
    user_folder("My Pictures", "Pictures")
}

#[cfg(windows)]
fn user_videos() -> PathBuf {
    user_folder("My Video", "Videos")
}

/// One of the user's own folders as Explorer has it (`value` among its shell folders), else
/// `fallback` in the user's home.
#[cfg(windows)]
fn user_folder(value: &str, fallback: &str) -> PathBuf {
    use winreg::HKCU;
    // "User Shell Folders" holds the folder as set (with %VARIABLES%), "Shell Folders" the
    // older copy, spelt out.
    let shell = r"Software\Microsoft\Windows\CurrentVersion\Explorer";
    for key in ["User Shell Folders", "Shell Folders"] {
        if let Ok(k) = HKCU.open_subkey(format!("{shell}\\{key}")) && let Ok(path) = k.get_value::<String, _>(value) {
            let path = expand_variables(&path);
            if !path.is_empty() && Path::new(&path).is_absolute() {
                return PathBuf::from(path);
            }
        }
    }
    std::env::var_os("USERPROFILE").map_or_else(|| PathBuf::from("."), |home| PathBuf::from(home).join(fallback))
}

#[cfg(not(windows))]
fn user_videos() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_VIDEOS_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), |home| PathBuf::from(home).join("Videos"))
}

#[cfg(not(windows))]
fn user_pictures() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_PICTURES_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), |home| PathBuf::from(home).join("Pictures"))
}

/// `%NAME%` in `text` replaced by the environment variable (left as it is when unset).
#[cfg_attr(not(windows), allow(dead_code))]
fn expand_variables(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) if !name.is_empty() => out.push_str(&value),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
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
}

/// The first `<stem> 001.png`, `<stem> 002.png`... not yet in `dir` (characters a file name
/// cannot hold become `_`).
pub fn next_picture(dir: &Path, stem: &str) -> PathBuf {
    let stem: String = stem.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let stem = stem.trim();
    let stem = if stem.is_empty() { "TA Reborn" } else { stem };
    (1..).map(|n| dir.join(format!("{stem} {n:03}.png"))).find(|p| !p.exists()).unwrap_or_else(|| dir.join(format!("{stem}.png")))
}

impl Config {
    /// Loads the user config; a missing or unreadable file yields defaults (logged).
    pub fn load() -> Config {
        Self::load_from(&config_path())
    }

    pub fn load_from(path: &Path) -> Config {
        Self::try_load_from(path).unwrap_or_else(|e| {
            if !matches!(e, LoadError::Missing) {
                log::warn!("config {}: {e}; using defaults", path.display());
            }
            Config::default()
        })
    }

    /// Like [`Config::load_from`] but reports why defaults would be used, so a front end
    /// can warn before overwriting a broken file.
    pub fn try_load_from(path: &Path) -> Result<Config, LoadError> {
        match read_settings(path) {
            Ok(text) => {
                let config: Config = toml::from_str(&text).map_err(|e| LoadError::Invalid(e.to_string()))?;
                *config.save_baseline.0.lock().unwrap_or_else(|e| e.into_inner()) = toml::Value::try_from(&config).ok().map(std::sync::Arc::new);
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(LoadError::Missing),
            Err(e) => Err(LoadError::Unreadable(e)),
        }
    }

    /// Saves to the settings file (the game's way: [`Config::save_keeping_launcher`]).
    pub fn save(&self) -> std::io::Result<()> {
        self.save_keeping_launcher(&config_path())
    }

    /// Saves only this instance's edits since its last load/save over the latest file.
    /// The launcher may have changed other settings while the game was running. Its own
    /// section is always kept (but for the seasons, which both show: [`SHARED_LAUNCHER_KEYS`]),
    /// and unknown file keys survive as well.
    pub fn save_keeping_launcher(&self, path: &Path) -> std::io::Result<()> {
        // Atomic replacement protects readers, but the read/merge/write transaction
        // also needs serialization or two running games can discard each other's edits.
        let _transaction = lock_settings(path)?;
        let mine = toml::Value::try_from(self).map_err(std::io::Error::other)?;
        let base = match self.save_baseline.0.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            Some(base) => base,
            None => std::sync::Arc::new(toml::Value::try_from(Config::default()).map_err(std::io::Error::other)?),
        };
        let mut merged = match read_settings(path) {
            Ok(text) => toml::from_str::<toml::Value>(&text).unwrap_or_else(|_| mine.clone()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => mine.clone(),
            Err(e) => return Err(e),
        };
        let launcher = merged.get("launcher").cloned();
        put_changes(&base, &mine, &mut merged);
        if let (Some(mut launcher), Some(table)) = (launcher, merged.as_table_mut()) {
            // (But for the seasons, which the game's options show too: what the game changed of
            // those is the game's.)
            let shared = |config: &toml::Value, key: &str| config.get("launcher").and_then(|l| l.get(key)).cloned();
            for key in SHARED_LAUNCHER_KEYS {
                if let (Some(value), Some(section)) = (shared(&mine, key), launcher.as_table_mut())
                    && shared(&base, key).as_ref() != Some(&value)
                {
                    section.insert(key.into(), value);
                }
            }
            table.insert("launcher".into(), launcher);
        }
        let text = toml::to_string_pretty(&merged).map_err(std::io::Error::other)?;
        check_settings_size(text.len() as u64)?;
        write_atomic(path, &text)?;
        // Track our own view, not merged values this instance has never adopted: a later
        // unrelated save must not put those stale values back over the launcher's edits.
        *self.save_baseline.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::sync::Arc::new(mine));
        Ok(())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        SettingsTransaction::open(path)?.save(self)
    }

    fn write_to(&self, path: &Path) -> std::io::Result<()> {
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        check_settings_size(text.len() as u64)?;
        write_atomic(path, &text)?;
        *self.save_baseline.0.lock().unwrap_or_else(|e| e.into_inner()) = toml::Value::try_from(self).ok().map(std::sync::Arc::new);
        Ok(())
    }
}

/// The launcher's settings that the game's options show too (Interface: the seasons, which deck
/// the game's loading screens and title menu as well as the launcher): the game saves its own
/// changes to them, though it keeps the rest of the launcher's section as the file has it.
const SHARED_LAUNCHER_KEYS: [&str; 2] = ["seasonal_effects", "seasonal_theme"];

fn check_settings_size(bytes: u64) -> std::io::Result<()> {
    if bytes > MAX_CONFIG_BYTES {
        Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "settings file exceeds the 4 MiB limit"))
    } else { Ok(()) }
}

fn read_settings(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    // Check before open too: named pipes/devices are not settings files. Recheck
    // the opened handle, and cap the actual read in case another writer grows it.
    let ordinary = |metadata: std::fs::Metadata| -> std::io::Result<()> {
        if !metadata.is_file() { return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "settings path is not a regular file")); }
        check_settings_size(metadata.len())
    };
    ordinary(std::fs::metadata(path)?)?;
    let file = std::fs::File::open(path)?;
    ordinary(file.metadata()?)?;
    let mut text = String::new();
    file.take(MAX_CONFIG_BYTES + 1).read_to_string(&mut text)?;
    check_settings_size(text.len() as u64)?;
    Ok(text)
}

/// A settings read/merge/write operation sharing the game's save lock. Keep this
/// guard through validation and merging, then use `save` rather than `Config::save_to`
/// to avoid trying to lock the same file twice. Dropping it releases the OS lock.
pub struct SettingsTransaction {
    path: PathBuf,
    _lock: std::fs::File,
}

impl SettingsTransaction {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        Ok(Self { path: path.to_path_buf(), _lock: lock_settings(path)? })
    }

    pub fn load(&self) -> Result<Config, LoadError> { Config::try_load_from(&self.path) }

    pub fn save(&self, config: &Config) -> std::io::Result<()> { config.write_to(&self.path) }
}

/// A separate stable file survives atomic replacement of the settings themselves.
/// Keep it after unlocking: removing it could give a waiting writer and a new writer
/// different locks. The OS releases the lock automatically after a crash or error.
fn lock_settings(path: &Path) -> std::io::Result<std::fs::File> {
    let filename = path.file_name().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "settings path has no file name"))?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) { std::fs::create_dir_all(parent)?; }
    let mut name = filename.to_os_string();
    name.push(".lock");
    let lock_path = path.with_file_name(name);
    match std::fs::symlink_metadata(&lock_path) {
        Ok(meta) => {
            #[cfg(windows)]
            let linked = { use std::os::windows::fs::MetadataExt; meta.file_attributes() & 0x400 != 0 };
            #[cfg(not(windows))]
            let linked = meta.file_type().is_symlink();
            if linked || !meta.is_file() { return Err(std::io::Error::other("settings lock must be an ordinary file")); }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
        Err(e) => return Err(e),
    }
    let file = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(lock_path)?;
    file.lock()?;
    Ok(file)
}

/// Three-way merge of settings: take fields changed by this writer, keep the rest of
/// the current file. Tables merge recursively; arrays are one setting.
fn put_changes(base: &toml::Value, mine: &toml::Value, theirs: &mut toml::Value) {
    match (base, mine, theirs) {
        (toml::Value::Table(base), toml::Value::Table(mine), toml::Value::Table(theirs)) => {
            for (key, value) in mine {
                match (base.get(key), theirs.get_mut(key)) {
                    (Some(old), _) if old == value => {}
                    (Some(old), Some(slot)) => put_changes(old, value, slot),
                    _ => { theirs.insert(key.clone(), value.clone()); }
                }
            }
            for key in base.keys().filter(|k| !mine.contains_key(*k)) {
                theirs.remove(key);
            }
        }
        (_, mine, theirs) => *theirs = mine.clone(),
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;

    #[test]
    fn alternating_game_and_launcher_edits_merge_by_field() {
        let dir = std::env::temp_dir().join(format!("ta_config_alternating_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        Config::default().save_to(&path).unwrap();
        let mut game = Config::load_from(&path);
        let mut launcher = Config::load_from(&path);
        launcher.mods.enabled.push("new_mod".into());
        launcher.updates.check = false;
        launcher.audio.effects = 0.2;
        launcher.graphics.width = 1600;
        launcher.keys.set("attack", "Z");
        launcher.save_to(&path).unwrap();
        // Unknown future fields should survive, too.
        let mut file: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        file["audio"].as_table_mut().unwrap().insert("future_option".into(), toml::Value::Boolean(true));
        std::fs::write(&path, toml::to_string(&file).unwrap()).unwrap();

        game.audio.music = 0.3;
        game.save_keeping_launcher(&path).unwrap();
        let saved = Config::load_from(&path);
        assert_eq!(saved.mods.enabled, ["new_mod"]);
        assert!(!saved.updates.check);
        assert_eq!((saved.audio.music, saved.audio.effects, saved.graphics.width), (0.3, 0.2, 1600));
        assert_eq!(saved.keys.get("attack"), "Z");
        let file: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(file["audio"]["future_option"].as_bool(), Some(true));

        // The launcher edits a field the game previously saved. A later unrelated game
        // save must leave it alone, despite the game's in-memory value still being 0.3.
        let mut launcher = Config::load_from(&path);
        launcher.audio.music = 0.6;
        launcher.save_to(&path).unwrap();
        game.gameplay.player_name = "Game Commander".into();
        game.save_keeping_launcher(&path).unwrap();
        let saved = Config::load_from(&path);
        assert_eq!(saved.audio.music, 0.6);
        assert_eq!(saved.gameplay.player_name, "Game Commander");
        assert_eq!(saved.mods.enabled, ["new_mod"]);

        // A deliberate edit to that field wins, then can be reset on a later save.
        game.audio.music = 0.8;
        game.save_keeping_launcher(&path).unwrap();
        game.audio.music = 0.3;
        game.save_keeping_launcher(&path).unwrap();
        assert_eq!(Config::load_from(&path).audio.music, 0.3);
        let file: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(!file.as_table().unwrap().contains_key("save_baseline"));
        let clone = game.clone();
        *clone.save_baseline.0.lock().unwrap() = None;
        assert_eq!(game, clone, "bookkeeping does not trigger settings change detection");
        assert!(game.save_baseline.0.lock().unwrap().is_some(), "clones keep independent baselines");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn simultaneous_settings_writers_merge_after_the_previous_transaction() {
        use std::sync::mpsc;
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("ta_config_transaction_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        Config::default().save_to(&path).unwrap();
        let mut first = Config::load_from(&path);
        let mut second = Config::load_from(&path);
        first.audio.effects = 0.2;
        second.audio.music = 0.3;
        let held = lock_settings(&path).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let other_path = path.clone();
        let writer = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            done_tx.send(second.save_keeping_launcher(&other_path)).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(done_rx.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)), "a second writer must wait before reading the settings it will merge");
        // Complete the first transaction while retaining its lock.
        write_atomic(&path, &toml::to_string(&first).unwrap()).unwrap();
        drop(held);
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        writer.join().unwrap();
        let saved = Config::load_from(&path);
        assert_eq!((saved.audio.music, saved.audio.effects), (0.3, 0.2));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn progress_updates_serialize_and_merge_completions_and_scenario_records() {
        use std::sync::mpsc;
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("ta_progress_transaction_{}", profiles::new_id()));
        let path = dir.join("progress.toml");
        CampaignProgress::update_at(&path, |p| {
            p.complete("arm", 0);
            p.record_scenario("fixed", "hard", true, 200, false);
            p.record_scenario("endless", "hard", false, 200, true);
        }).unwrap();
        let held = lock_settings(&path).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let other_path = path.clone();
        let writer = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            done_tx.send(CampaignProgress::update_at(&other_path, |p| {
                p.complete("core", 1);
                p.complete_tutorial(2);
                p.record_scenario("fixed", "hard", true, 175, false);
                p.record_scenario("endless", "hard", false, 250, true);
            })).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(done_rx.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)), "a second writer must wait before reading current progress");
        let mut first: CampaignProgress = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        first.complete("arm", 3);
        first.complete_tutorial(1);
        first.record_scenario("fixed", "hard", true, 150, false);
        first.record_scenario("endless", "hard", false, 300, true);
        write_atomic(&path, &toml::to_string(&first).unwrap()).unwrap();
        drop(held);
        let merged = done_rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        writer.join().unwrap();
        assert_eq!((merged.unlocked("arm"), merged.unlocked("core")), (4, 2));
        assert_eq!(merged.tutorials, [1, 2]);
        assert_eq!(merged.scenarios["fixed"].best["hard"], 150);
        assert_eq!(merged.scenarios["endless"].best["hard"], 300);
        assert_eq!(merged, toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn progress_update_preserves_a_malformed_existing_file() {
        let dir = std::env::temp_dir().join(format!("ta_progress_corrupt_{}", profiles::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("progress.toml");
        let original = "[completed\narm = 8\n";
        std::fs::write(&path, original).unwrap();
        assert!(CampaignProgress::update_at(&path, |_| panic!("must not apply activity to a reset ledger")).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_game_keeps_the_launchers_section() {
        let dir = std::env::temp_dir().join(format!("ta_config_launcher_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let mut launcher = Config::default();
        launcher.launcher.window = Some([1300.0, 800.0]);
        launcher.save_to(&path).unwrap();
        // The game, started before that, saves its options.
        let game = Config { audio: Audio { music: 0.3, ..Default::default() }, ..Default::default() };
        game.save_keeping_launcher(&path).unwrap();
        let file = Config::load_from(&path);
        assert_eq!((file.audio.music, file.launcher.window), (0.3, Some([1300.0, 800.0])));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn presets_are_recognised_after_applying() {
        for p in Preset::LEVELS {
            let mut g = Graphics::default();
            p.apply(&mut g);
            assert_eq!(g.matching_preset(), p);
        }
        let mut g = Graphics::default();
        Preset::High.apply(&mut g);
        g.bloom = false;
        assert_eq!(g.matching_preset(), Preset::Custom);
    }

    /// On integrated graphics a level draws the battlefield at no more pixels than its budget
    /// (on the options' steps, half at the least); elsewhere at its own render scale. The level
    /// is still recognised.
    #[test]
    fn levels_fit_their_render_scale_to_the_screen_on_integrated_graphics() {
        let fitted = |level: Preset, kind: GpuKind, name: &str, (w, h): (u64, u64)| {
            let mut g = Graphics::default();
            level.apply(&mut g);
            g.preset = level;
            g.fit_render_scale(kind, name, w * h);
            assert_eq!(g.matching_preset(), level, "{level:?} {name} {w}x{h}");
            g.render_scale
        };
        let integrated = GpuKind::Integrated;
        // A 1080p laptop: as the levels have always been.
        for level in Preset::LEVELS {
            assert_eq!(fitted(level, integrated, "Intel(R) Iris(R) Xe Graphics", (1920, 1080)), level.render_scale().unwrap());
        }
        // A 13-inch MacBook Air (its Retina picture is 2940 x 1912 by default) and a 15-inch one.
        assert_eq!(fitted(Preset::Medium, integrated, "Apple M5", (2940, 1912)), 0.6);
        assert_eq!(fitted(Preset::High, integrated, "Apple M5", (2940, 1912)), 0.6);
        assert_eq!(fitted(Preset::Low, integrated, "Apple M5", (2940, 1912)), 0.6);
        assert_eq!(fitted(Preset::Ultra, integrated, "Apple M5", (2940, 1912)), 0.8);
        assert_eq!(fitted(Preset::Medium, integrated, "Apple M5", (3420, 2224)), 0.5);
        // A 14-inch MacBook Pro's Pro and Max chips draw more of it.
        assert_eq!(fitted(Preset::Medium, integrated, "Apple M4 Pro", (3024, 1964)), 0.8);
        assert_eq!(fitted(Preset::High, integrated, "Apple M4 Max", (3024, 1964)), 1.0);
        // A dedicated card draws the whole picture.
        assert_eq!(fitted(Preset::Ultra, GpuKind::Discrete, "NVIDIA GeForce RTX 4080", (3840, 2160)), 1.0);
        // A render scale under the level's own, set by itself, is still the level.
        let mut g = Graphics::default();
        Preset::High.apply(&mut g);
        g.render_scale = 0.7;
        assert_eq!(g.matching_preset(), Preset::High);
        // Once, as the game starts: where the scale is the level's own (left as the level set
        // it); a scale set by hand is kept, and the next start leaves it.
        let retina = 2940 * 1912;
        let mut untouched = Graphics { preset: Preset::Medium, ..Default::default() };
        Preset::Medium.apply(&mut untouched);
        assert!(untouched.fit_render_scale_once(integrated, "Apple M5", retina));
        assert_eq!(untouched.render_scale, 0.6);
        untouched.render_scale = 1.0;
        assert!(!untouched.fit_render_scale_once(integrated, "Apple M5", retina), "once only");
        assert_eq!(untouched.render_scale, 1.0);
        let mut by_hand = Graphics { preset: Preset::Medium, ..Default::default() };
        Preset::Medium.apply(&mut by_hand);
        by_hand.render_scale = 0.85;
        assert!(by_hand.fit_render_scale_once(integrated, "Apple M5", retina));
        assert_eq!(by_hand.render_scale, 0.85);
        // (Settings from before it read as not yet fitted.)
        let older: Config = toml::from_str("[graphics]\npreset = \"Medium\"\nrender_scale = 1.0\n").unwrap();
        assert!(!older.graphics.scale_fitted);
        // The full screen's pixels are the screen's own; a window's, its own.
        g.window_mode = WindowMode::Fullscreen;
        assert_eq!(g.picture_pixels(Some((2940, 1912))), 2940 * 1912);
        g.window_mode = WindowMode::Windowed;
        assert_eq!(g.picture_pixels(Some((2940, 1912))), g.width as u64 * g.height as u64);
    }

    /// The Steam Deck's graphics are of the Radeon 660M's class: Medium, however its driver
    /// names and reports them, and at that level its 1280 × 800 pixels are drawn whole.
    #[test]
    fn the_steam_decks_graphics_are_medium() {
        for (kind, name) in [
            (GpuKind::Integrated, "AMD Custom GPU 0405 (RADV VANGOGH)"),
            (GpuKind::Integrated, "AMD Custom GPU 0932 (RADV VANGOGH)"),
            (GpuKind::Discrete, "AMD Custom GPU 0405"),
            (GpuKind::Other, "AMD Custom GPU 0405 (radeonsi, vangogh, LLVM 15.0.7, DRM 3.54)"),
        ] {
            assert_eq!(Preset::for_gpu(kind, name), Preset::Medium, "{name}");
        }
        assert_eq!(Preset::for_gpu(GpuKind::Integrated, "AMD Radeon 660M"), Preset::Medium, "its class");
        assert_eq!(Preset::for_gpu(GpuKind::Integrated, "AMD Radeon(TM) Graphics"), Preset::Low, "not every AMD chip");
        assert_eq!(Preset::for_gpu(GpuKind::Software, "llvmpipe (LLVM 15.0.7, 256 bits)"), Preset::Low);
        let mut g = Graphics::default();
        Preset::Medium.apply(&mut g);
        g.preset = Preset::Medium;
        assert!(g.fit_render_scale_once(GpuKind::Integrated, "AMD Custom GPU 0405 (RADV VANGOGH)", 1280 * 800));
        assert_eq!(g.render_scale, 1.0);
        assert_eq!(g.matching_preset(), Preset::Medium);
    }

    /// The interface's scale for a screen: the largest 5% step that leaves the layout its room,
    /// never under 100% nor over the settings' most.
    #[test]
    fn the_interface_scale_fills_a_screen_as_far_as_its_layout_allows() {
        use limits::{GAME_POINTS, LAUNCHER_POINTS, ui_scale_to_fit};
        assert_eq!(ui_scale_to_fit((1280, 800), GAME_POINTS), 1.25);
        assert_eq!(ui_scale_to_fit((1280, 800), LAUNCHER_POINTS), 1.3, "4/3, on the steps");
        assert_eq!(ui_scale_to_fit((1920, 1200), GAME_POINTS), 1.85);
        assert_eq!(ui_scale_to_fit((1920, 1080), GAME_POINTS), 1.8);
        assert_eq!(ui_scale_to_fit((1024, 600), GAME_POINTS), 1.0);
        assert_eq!(ui_scale_to_fit((800, 480), GAME_POINTS), 1.0, "never smaller than it comes");
        assert_eq!(ui_scale_to_fit((7680, 4320), GAME_POINTS), *limits::UI_SCALE.end());
    }

    #[test]
    fn old_settings_files_keep_their_options() {
        let old: Config = toml::from_str("[graphics]\nmsaa = 8\n").unwrap();
        assert_eq!(old.graphics.preset, Preset::Custom);
        assert_eq!(old.graphics.msaa, 8);
        assert_eq!(Config::default().graphics.preset, Preset::Auto);
    }

    #[test]
    fn art_style_is_independent_of_camera_and_quality_presets() {
        for art_style in [ArtStyle::Reborn, ArtStyle::Classic] {
            for camera in [CameraStyle::Modern, CameraStyle::Classic] {
                for preset in Preset::LEVELS {
                    let mut graphics = Graphics { art_style, camera, ..Default::default() };
                    preset.apply(&mut graphics);
                    assert_eq!(graphics.art_style, art_style);
                    assert_eq!(graphics.camera, camera);
                    assert_eq!(graphics.matching_preset(), preset);
                    let mut other_art = graphics.clone();
                    other_art.art_style = if art_style == ArtStyle::Reborn { ArtStyle::Classic } else { ArtStyle::Reborn };
                    assert!(!graphics.level_options_differ(&other_art));
                    assert_eq!(other_art.matching_preset(), preset, "art style never turns a quality level into Custom");
                    graphics.hd_surfaces = !graphics.hd_surfaces;
                    assert_eq!(graphics.art_style, art_style);
                }
            }
        }
    }

    #[test]
    fn graphics_cards_get_sensible_levels() {
        assert_eq!(Preset::for_gpu(GpuKind::Software, "Microsoft Basic Render Driver"), Preset::Low);
        assert_eq!(Preset::for_gpu(GpuKind::Integrated, "Intel(R) UHD Graphics 620"), Preset::Low);
        assert_eq!(Preset::for_gpu(GpuKind::Integrated, "Intel(R) Iris(R) Xe Graphics"), Preset::Medium);
        assert_eq!(Preset::for_gpu(GpuKind::Integrated, "AMD Radeon 780M Graphics"), Preset::Medium);
        assert_eq!(Preset::for_gpu(GpuKind::Discrete, "NVIDIA GeForce RTX 3070"), Preset::High);
        assert_eq!(Preset::for_gpu(GpuKind::Discrete, "NVIDIA GeForce MX450"), Preset::Medium);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Enabled mods are read in lower case, as mods' own ids are, each once.
    #[test]
    fn enabled_mod_ids_read_in_lower_case_once() {
        let config: Config = toml::from_str("[mods]\nenabled = [\"MyMod\", \"other\", \"mymod\"]\n").unwrap();
        assert_eq!(config.mods.enabled, ["mymod", "other"]);
    }

    /// Mods hidden from the library are kept as the enabled ones are (lower case, once), are
    /// written and read back, and an older file without them hides nothing. Hiding is apart from
    /// being on.
    #[test]
    fn hidden_mods_are_kept_once_in_lower_case_and_roundtrip() {
        let old: Config = toml::from_str("[mods]\nenabled = [\"escalation\"]\n").unwrap();
        assert!(old.mods.hidden.is_empty(), "an older settings file hides nothing");
        let read: Config = toml::from_str("[mods]\nhidden = [\"TotalMayhem\", \"secretforces\", \"totalmayhem\"]\n").unwrap();
        assert_eq!(read.mods.hidden, ["totalmayhem", "secretforces"]);
        let mut config = Config::default();
        config.mods.enabled = vec!["escalation".into()];
        config.mods.set_hidden("Escalation", true);
        config.mods.set_hidden("escalation", true);
        config.mods.set_hidden("battletech", true);
        assert_eq!(config.mods.hidden, ["escalation", "battletech"]);
        assert!(config.mods.is_hidden("ESCALATION") && !config.mods.is_hidden("totalmayhem"));
        assert_eq!(config.mods.enabled, ["escalation"], "hiding never turns a mod off");
        let dir = std::env::temp_dir().join(format!("ta-config-hidden-{}", profiles::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONFIG_FILE);
        config.save_to(&path).unwrap();
        let loaded = Config::load_from(&path);
        assert_eq!(loaded.mods.hidden, ["escalation", "battletech"]);
        config.mods.set_hidden("escalation", false);
        assert_eq!(config.mods.hidden, ["battletech"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn oversized_settings_never_replace_the_existing_file() {
        let dir = std::env::temp_dir().join(format!("ta-config-bounds-{}", profiles::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONFIG_FILE);
        let mut config = Config::default();
        config.save_to(&path).unwrap();
        std::fs::File::create(&path).unwrap().set_len(MAX_CONFIG_BYTES + 1).unwrap();
        assert!(matches!(Config::try_load_from(&path), Err(LoadError::Unreadable(error)) if error.kind() == std::io::ErrorKind::InvalidData));
        config.audio.music = 0.25;
        assert!(config.save_keeping_launcher(&path).is_err(), "game merge must not overwrite an unread oversized file");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), MAX_CONFIG_BYTES + 1);
        std::fs::remove_file(&path).unwrap();
        config.save_to(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        config.updates.url = "x".repeat(MAX_CONFIG_BYTES as usize);
        assert!(config.save_to(&path).is_err(), "writers must not produce settings their readers reject");
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(config.save_keeping_launcher(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(matches!(Config::try_load_from(&dir), Err(LoadError::Unreadable(_))), "directories are not settings files");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unit_skins_default_round_trip_and_survive_performance_presets() {
        for text in ["", "[graphics]\npreset = \"Low\"\n"] {
            let config: Config = toml::from_str(text).unwrap();
            assert_eq!(config.graphics.unit_skin, "reborn");
        }
        let mut config = Config::default();
        config.graphics.unit_skin = "my-armour".into();
        assert_eq!(config.graphics.enemy_skin(), "my-armour");
        config.graphics.enemy_unit_skin = "desert".into();
        for preset in Preset::LEVELS {
            preset.apply(&mut config.graphics);
            assert_eq!(config.graphics.unit_skin, "my-armour");
            assert_eq!(config.graphics.enemy_skin(), "desert");
        }
        let text = toml::to_string(&config).unwrap();
        let restored: Config = toml::from_str(&text).unwrap();
        assert_eq!(restored.graphics.unit_skin, "my-armour");
        assert_eq!(restored.graphics.enemy_skin(), "desert");
        let old: Config = toml::from_str("[graphics]\nunit_skin = 'christmas'\n").unwrap();
        assert_eq!(old.graphics.enemy_skin(), "christmas");
    }

    #[test]
    fn launcher_cursor_defaults_and_system_pointer_opt_out_round_trip() {
        for source in ["", "[launcher]", "[launcher]\non_play = 'Minimise'"] {
            let config: Config = toml::from_str(source).unwrap();
            assert!(config.launcher.themed_cursor);
        }
        let config: Config = toml::from_str("[launcher]\nthemed_cursor = false").unwrap();
        assert!(!config.launcher.themed_cursor);
        let restored: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert!(!restored.launcher.themed_cursor);
    }

    #[test]
    fn seasonal_defaults_follow_the_calendar_and_preserve_an_explicit_opt_out() {
        for source in ["", "[launcher]", "[launcher]\non_play = 'Minimise'"] {
            let config: Config = toml::from_str(source).unwrap();
            assert!(config.launcher.seasonal_effects);
            assert_eq!(config.launcher.seasonal_theme, SeasonalTheme::Automatic);
        }
        assert!(Config::default().launcher.seasonal_effects);
        let disabled: Config = toml::from_str("[launcher]\nseasonal_effects = false\nseasonal_theme = 'NewYear'").unwrap();
        assert!(!disabled.launcher.seasonal_effects);
        assert_eq!(disabled.launcher.seasonal_theme, SeasonalTheme::NewYear);
        let restored: Config = toml::from_str(&toml::to_string(&disabled).unwrap()).unwrap();
        assert_eq!(restored, disabled, "saving must not re-enable an explicit opt-out");
        let spooky: Config = toml::from_str("[launcher]\nseasonal_theme = 'Halloween'").unwrap();
        assert_eq!(spooky.launcher.seasonal_theme, SeasonalTheme::Halloween);
    }

    #[test]
    fn art_style_defaults_to_reborn_in_new_and_older_settings() {
        assert_eq!(Config::default().graphics.art_style, ArtStyle::Reborn);
        for text in ["", "[graphics]\n", "[graphics]\ncamera = \"Classic\"\nhd_surfaces = false\npreset = \"Low\"\n"] {
            let config: Config = toml::from_str(text).unwrap();
            assert_eq!(config.graphics.art_style, ArtStyle::Reborn);
        }
    }

    #[test]
    fn art_style_saves_and_loads_with_stable_names() {
        let dir = std::env::temp_dir().join(format!("ta_art_style_{}", std::process::id()));
        let path = dir.join(CONFIG_FILE);
        for (art_style, name) in [(ArtStyle::Reborn, "reborn"), (ArtStyle::Classic, "classic")] {
            let config = Config {
                graphics: Graphics { art_style, camera: CameraStyle::Classic, hd_surfaces: false, ..Default::default() },
                ..Default::default()
            };
            config.save_to(&path).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains(&format!("art_style = \"{name}\"")));
            assert_eq!(Config::try_load_from(&path).unwrap(), config);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_folder_under_the_old_name_moves_to_the_new_one() {
        let base = std::env::temp_dir().join(format!("ta_config_move_{}", std::process::id()));
        let (new, old) = (base.join(APP_DIR_NAME), base.join(OLD_APP_DIR_NAME));
        std::fs::create_dir_all(old.join("saves")).unwrap();
        std::fs::write(old.join(CONFIG_FILE), "[audio]\n").unwrap();
        // Only the old one: it moves, with everything in it.
        assert!(move_folder(&old, &new).unwrap());
        assert!(new.join(CONFIG_FILE).is_file() && new.join("saves").is_dir() && !old.exists());
        // Both (an older version run since): the new one is used, the old one left alone.
        std::fs::create_dir_all(&old).unwrap();
        assert!(!move_folder(&old, &new).unwrap());
        assert!(old.is_dir());
        // Neither: nothing to do (the new one is made when something is written there).
        std::fs::remove_dir_all(&base).unwrap();
        assert!(!move_folder(&old, &new).unwrap());
        assert!(!new.exists());
    }

    #[test]
    fn pictures_are_personal_unless_explicitly_overridden() {
        let legacy = PathBuf::from("Pictures").join(APP_DIR_NAME);
        let first = "00000000000000000000000000000001";
        let second = "00000000000000000000000000000002";
        assert_eq!(pictures_path(legacy.clone(), None, None), legacy);
        assert_eq!(pictures_path(legacy.clone(), None, Some(first)), legacy.join("profiles").join(first));
        assert_ne!(pictures_path(legacy.clone(), None, Some(first)), pictures_path(legacy.clone(), None, Some(second)));
        let capture = PathBuf::from("custom-capture-folder");
        assert_eq!(pictures_path(legacy.clone(), Some(capture.clone()), Some(first)), capture);
        assert_eq!(pictures_path(legacy.clone(), Some(capture.clone()), None), capture);
        assert_eq!(pictures_path(legacy.clone(), Some(PathBuf::new()), Some(first)), legacy.join("profiles").join(first));
    }

    #[test]
    fn picture_names_count_up_and_stay_valid() {
        let dir = std::env::temp_dir().join(format!("ta_pictures_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = next_picture(&dir, "Coast To Coast");
        assert_eq!(first.file_name().unwrap(), "Coast To Coast 001.png");
        std::fs::write(&first, b"").unwrap();
        assert_eq!(next_picture(&dir, "Coast To Coast").file_name().unwrap(), "Coast To Coast 002.png");
        assert_eq!(next_picture(&dir, "What? A:B").file_name().unwrap(), "What_ A_B 001.png");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn variables_expand_in_folder_paths() {
        // SAFETY: tests in this module do not read this variable elsewhere.
        unsafe { std::env::set_var("TA_TEST_PICTURES_HOME", r"C:\Users\Someone") };
        assert_eq!(expand_variables(r"%TA_TEST_PICTURES_HOME%\Pictures"), r"C:\Users\Someone\Pictures");
        assert_eq!(expand_variables("100% sure"), "100% sure");
        assert_eq!(expand_variables("%TA_UNSET_VARIABLE_XYZ%\\x"), "%TA_UNSET_VARIABLE_XYZ%\\x");
    }

    #[test]
    fn round_trips_and_tolerates_partial_files() {
        let c = Config {
            game_dir: Some(PathBuf::from(r"C:\GOG Games\Total Annihilation")),
            mods: Mods { enabled: vec!["escalation".into()], ..Default::default() },
            ..Default::default()
        };
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back, c);

        let partial: Config = toml::from_str("[graphics]\nvsync = false\n").unwrap();
        assert!(!partial.graphics.vsync);
        assert_eq!(partial.audio, Audio::default());
        // Settings files from before the classic interface keep the modern HUD, and those
        // from before the order button setting give orders with the right button.
        let older: Config = toml::from_str("[gameplay]\nta_cursors = false\n").unwrap();
        assert!(!older.gameplay.ta_cursors && !older.gameplay.classic_interface);
        // ...and those from before its art was smoothed see it smoothed.
        let classic: Config = toml::from_str("[gameplay]\nclassic_interface = true\n").unwrap();
        assert!(classic.gameplay.classic_interface && !classic.gameplay.classic_pixels);
        assert_eq!(older.gameplay.order_button, OrderButton::Right);
        assert!(!older.gameplay.reduced_motion && !older.gameplay.reduced_flashes && !older.gameplay.high_contrast_cursor);
        assert_eq!(older.gameplay.cursor_scale, 1.0);
        assert_eq!(older.gameplay.message_duration, 1.0);
        assert!(!older.gameplay.team_markers && !older.gameplay.keyboard_pointer);
        assert!(older.gameplay.screen_reader_announcements);
        assert!(older.gameplay.start_tips && older.gameplay.coach_muted.is_empty(), "tips on and nothing hidden, as before");
        assert_eq!(older.gameplay.gamepad_deadzone, 0.18);
        assert_eq!(older.gameplay.gamepad_sensitivity, 1.0);
        let left: Config = toml::from_str("[gameplay]\norder_button = \"Left\"\n").unwrap();
        assert_eq!(left.gameplay.order_button, OrderButton::Left);
    }

    /// The radial menu is on in a new settings file and in every file that never chose: one from
    /// before the menu, one saved since with the default written out. A player's `false` stays off
    /// through loads and saves, the game's and the launcher's, and so does how it opens.
    #[test]
    fn the_radial_menu_is_on_unless_the_player_turned_it_off() {
        let fresh = Config::default();
        assert!(fresh.gameplay.radial_menu);
        assert_eq!(fresh.gameplay.radial_opens, RadialOpens::ClickOrHold);
        for never_chosen in ["", "[gameplay]\norder_button = \"Left\"\n", "[gameplay]\nradial_menu = true\n"] {
            let config: Config = toml::from_str(never_chosen).unwrap();
            assert!(config.gameplay.radial_menu, "{never_chosen:?}");
            assert_eq!(config.gameplay.radial_opens, RadialOpens::ClickOrHold, "{never_chosen:?}");
        }
        let off: Config = toml::from_str("[gameplay]\nradial_menu = false\n").unwrap();
        assert!(!off.gameplay.radial_menu, "a deliberate off stays off");
        let hold: Config = toml::from_str("[gameplay]\nradial_opens = \"Hold\"\n").unwrap();
        assert!(hold.gameplay.radial_menu && hold.gameplay.radial_opens == RadialOpens::Hold);

        // Through the files: the launcher writes the player's choice, the game saves something else
        // of its own, and the choice is still there (and the other way round).
        let dir = std::env::temp_dir().join(format!("ta_config_radial_{}", profiles::new_id()));
        let path = dir.join(CONFIG_FILE);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "[gameplay]\nplayer_name = \"Old\"\n").unwrap();
        let game = Config::load_from(&path);
        assert!(game.gameplay.radial_menu, "a file from before the menu: on");
        let mut launcher = Config::load_from(&path);
        launcher.gameplay.radial_menu = false;
        launcher.gameplay.radial_opens = RadialOpens::Hold;
        launcher.save_to(&path).unwrap();
        let mut game = game;
        game.audio.music = 0.25;
        game.save_keeping_launcher(&path).unwrap();
        let saved = Config::load_from(&path);
        assert!(!saved.gameplay.radial_menu, "the game's save keeps the player's off");
        assert_eq!(saved.gameplay.radial_opens, RadialOpens::Hold);
        assert_eq!(saved.audio.music, 0.25);
        let mut game = saved;
        game.gameplay.radial_menu = true;
        game.save_keeping_launcher(&path).unwrap();
        assert!(Config::load_from(&path).gameplay.radial_menu, "and turned on again, it is on");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The switches added for what used to be fixed keep it as it was in files from before them
    /// (icons, shake, the ground in the wind and units' replies on; no frame rate in the corner),
    /// and the player's choices round-trip.
    #[test]
    fn new_switches_keep_what_older_files_had() {
        for older in ["", "[gameplay]\nreduced_motion = true\n", "[graphics]\npreset = \"Low\"\n[audio]\nvoice = 0.5\n"] {
            let config: Config = toml::from_str(older).unwrap();
            assert!(config.gameplay.strategic_icons && config.gameplay.screen_shake, "{older:?}");
            assert!(config.graphics.ground_life && !config.graphics.show_frame_rate, "{older:?}");
            assert!(config.audio.unit_replies, "{older:?}");
        }
        let mut config = Config::default();
        config.gameplay.strategic_icons = false;
        config.gameplay.screen_shake = false;
        config.graphics.ground_life = false;
        config.graphics.show_frame_rate = true;
        config.audio.unit_replies = false;
        let back: Config = toml::from_str(&toml::to_string_pretty(&config).unwrap()).unwrap();
        assert_eq!(back, config);
        // The graphics levels leave the two of them alone (a matter of taste, not of the card).
        for level in Preset::LEVELS {
            let mut g = config.graphics.clone();
            level.apply(&mut g);
            assert!(!g.ground_life && g.show_frame_rate);
            assert_eq!(g.matching_preset(), level);
        }
    }

    /// The seasons are the launcher's settings, but the game's options show them too: what the
    /// game changes of them is saved, while the rest of the launcher's section stays as the file
    /// has it, and a season the launcher changed meanwhile is not put back by the game.
    #[test]
    fn the_game_saves_its_own_changes_to_the_seasons() {
        let dir = std::env::temp_dir().join(format!("ta_config_seasons_{}", profiles::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONFIG_FILE);
        Config::default().save_to(&path).unwrap();
        let mut game = Config::load_from(&path);
        let mut launcher = Config::load_from(&path);
        launcher.launcher.window = Some([1200.0, 760.0]);
        launcher.launcher.seasonal_theme = SeasonalTheme::Halloween;
        launcher.save_to(&path).unwrap();
        game.launcher.seasonal_effects = false;
        game.save_keeping_launcher(&path).unwrap();
        let saved = Config::load_from(&path);
        assert!(!saved.launcher.seasonal_effects, "the game's change is kept");
        assert_eq!(saved.launcher.seasonal_theme, SeasonalTheme::Halloween, "the launcher's change is not undone");
        assert_eq!(saved.launcher.window, Some([1200.0, 760.0]), "the rest of the launcher's section stays");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hidden_hints_survive_a_settings_round_trip() {
        let mut config = Config::default();
        config.gameplay.start_tips = false;
        config.gameplay.coach_muted = vec!["metal_full".into(), "builders_idle".into()];
        let text = toml::to_string_pretty(&config).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back, config);
        assert!(!back.gameplay.start_tips && back.gameplay.coach_muted.len() == 2);
    }

    #[test]
    fn accessibility_preferences_survive_a_settings_round_trip() {
        let mut config = Config::default();
        config.gameplay.reduced_motion = true;
        config.gameplay.reduced_flashes = true;
        config.gameplay.high_contrast_cursor = true;
        config.gameplay.cursor_scale = 2.5;
        config.gameplay.message_duration = 4.0;
        config.gameplay.team_markers = true;
        config.gameplay.keyboard_pointer = true;
        config.gameplay.screen_reader_announcements = false;
        config.gameplay.gamepad_deadzone = 0.3;
        config.gameplay.gamepad_sensitivity = 2.5;
        let text = toml::to_string_pretty(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }
}

#[cfg(test)]
mod atomic_write_tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn concurrent_writers_publish_complete_files_without_sharing_temporary_names() {
        let dir = std::env::temp_dir().join(format!("ta_config_atomic_concurrent_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let legacy_tmp = dir.join("config.tmp");
        std::fs::write(&legacy_tmp, "unrelated recovery data").unwrap();
        let payloads: Vec<_> = (0..8).map(|id| format!("writer = {id}\n# {}\n", char::from(b'a' + id).to_string().repeat(32 * 1024))).collect();
        write_atomic(&path, &payloads[0]).unwrap();
        let barrier = Arc::new(Barrier::new(payloads.len()));
        std::thread::scope(|scope| {
            for payload in &payloads {
                let barrier = barrier.clone();
                let path = &path;
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..16 { write_atomic(path, payload).unwrap(); }
                });
            }
        });
        let final_file = std::fs::read_to_string(&path).unwrap();
        assert!(payloads.contains(&final_file), "one complete writer's contents are published");
        assert_eq!(std::fs::read_to_string(&legacy_tmp).unwrap(), "unrelated recovery data");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "successful writes leave no temporary files");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn atomic_replacement_waits_for_a_temporary_windows_sharing_conflict() {
        use std::os::windows::fs::OpenOptionsExt;
        use std::sync::mpsc;
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("ta_config_atomic_sharing_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("record.bin");
        std::fs::write(&path, b"previous complete record").unwrap();
        // A reader such as a scanner can allow reads/writes while denying deletion,
        // which Windows needs when replacing this file with the completed temp file.
        let reader = std::fs::OpenOptions::new().read(true).share_mode(0x1 | 0x2).open(&path).unwrap();
        let payload = vec![73u8; 32 * 1024];
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let output = path.clone();
        let expected = payload.clone();
        let writer = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            done_tx.send(write_atomic_bytes(&output, &payload)).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(done_rx.recv_timeout(Duration::from_millis(75)), Err(mpsc::RecvTimeoutError::Timeout)), "publication must wait for a transient sharing conflict");
        assert_eq!(std::fs::read(&path).unwrap(), b"previous complete record");
        drop(reader);
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        writer.join().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), expected);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "successful retry cleans up its owned temp file");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_atomic_replacement_cleans_up_its_own_temporary_file() {
        let dir = std::env::temp_dir().join(format!("ta_config_atomic_failure_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.toml");
        std::fs::create_dir_all(&path).unwrap();
        assert!(write_atomic(&path, "contents").is_err(), "a directory cannot be replaced by settings");
        assert!(path.is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "failure removes only its temporary file");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
