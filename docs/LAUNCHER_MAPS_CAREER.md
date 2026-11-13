# Maps, commander profiles and achievements

## Launcher controls

The profile control in the top right opens a commander menu with profile switching,
identity editing, a service record and achievements. It shows the rank's insignia, the
next rank and the level it comes at, and its brass XP meter keeps the numeric progress
beside the track (past the top level, toward the next service star). The career's XP
counts the achievements' rewards with the battles' XP. Profile switching
waits for running games and content operations to finish.

## Profile page

Open **Profile** or press **Ctrl+5** (the profile menu's **View Profile** and
**Achievements** open it too, **Manage Profiles** at its Commanders tab). Its head shows the
commander's picture, name and motto, rank title and insignia, allegiance, the day the profile
was made, time played and career XP, and the bar to the next rank with the rank to come. The
showcase beside it holds up to five earned achievements of the commander's choosing: **Show
on your profile** on an achievement's page adds one, **Remove from your profile** takes it
off, and a click on a showcased badge opens its page. The motto (up to 60 characters) is set
in the profile editor. **Edit Profile** and **Open Profile Folder** are in the page's
heading.

Its tabs are the service record's: Overview, Achievements, Campaign and Match History, and
**Commanders**, everyone who plays on this device with their rank, insignia and motto.
**Switch To** makes another commander the active one; **Edit** changes the active one;
**New Profile** makes one; **Delete** asks first, then moves the profile's folder (its
settings, saves, replays, pictures, career and achievements) to the Recycle Bin. Deleting the
active commander hands over to another first; the last commander cannot be deleted. These
wait, as switching does, for a running game or a mod, map or skin operation to finish.

The service record is also available from the in-game profile menu. Overview shows the
battles finished, victories (with the share won), time played, achievements and career XP;
the share of the career's XP from each source (kills, construction, captures, economy, time,
results, achievements); the record by mode; the best battles (enemies destroyed, units built,
the Commander's kills, the highest incomes and the largest army in one battle); favourite
units by completed constructions and factions by eligible playing time; and the latest
battles. Numbers are grouped as the interface's language writes them. Campaign shows existing completion progress alongside newly verified
victories. Match History contains recorded modes, maps, factions, results, duration,
kills, constructions and XP. Achievements shows how many are earned of each tier and the
latest earned, and lists them all with their badges (greyed until earned): filtered by
category and progress, searched, and sorted by list order, recently earned, nearest to
earning or rarest tier. A chosen achievement's page shows its badge large, its progress or
the date it was earned, and the picture the game kept of that moment, with the map, mode
and battle time. Large histories are paginated.

The launcher footer keeps settings-save feedback on the left and
“Total Annihilation is © its respective owners.” on the right. It no longer displays
the personal configuration path. The main and navigation Play controls include their
own game-data readiness indicator.

## Maps library

Open **Maps** or press **Ctrl+3**. **Ctrl+F** focuses map search, **Escape** clears that
search while it has focus, and **F5** rescans installed sources. Visiting the page looks
through the sources again only if they changed (new or removed mods and map packs, mods
enabled or disabled, another game folder).

The library lists original, Reborn and installed mod maps, including maps supplied by
disabled sources, as cards or as a list. Each card shows a picture of the map's own terrain
(`ta_render::map_preview`, not TA's stored minimap): pictures are drawn on worker threads,
those on show first, and kept in `cache/pictures/maps/` beside the settings with the
terrain's size, keyed by the terrain file, its palette and how pictures are drawn. Search
by map name, description, world or source (every word must match); filter by availability,
players, size, world, source and favourites; order by name, players, size, when the profile
last played it, or source. A map's source is what it comes with, by name: the game's own maps
by where they come from (**Total Annihilation**, **Core Contingency**, **Battle Tactics**;
`ta_data::map::Origin`), an archive a player put in the game's folder by its file's name,
**TA Reborn**, or the mod or map pack (the SOURCE filter goes by these names). The arrow keys
move between cards, choosing each; Enter opens the details on a narrow page.

Whatever the search, filters and order, the library comes in sections, each under its heading
with how many it holds (`library::sections`; one section alone has none): **Your
Favourites** first, then **TA Reborn Maps** (TA Reborn's own maps, `assets/reborn/maps`:
made for its engine and fair to every start), then **More Maps**, each in the order chosen
(`Entry::group`). TA Reborn's maps carry a brass **REBORN** ribbon on their cards (a tag after
the name in the list) and a brass frame, and their details a TA Reborn banner; maps made from
Supreme Commander's (docs/SUPCOM_MAPS.md) a **SUPREME COMMANDER** ribbon (`Entry::ribbon`) and a
note in their details. In the game, skirmish's and the lobby's map lists tag every map with its
origin (TA, CC, BT, REBORN, SUPCOM, MOD), show one origin or the expansions together, and order
by name, origin or players (`apps/ta_game/src/map_list.rs`). The page opens
on the first of them all. A card's star, over its picture's corner (always on a favourite's,
under the pointer on the others'), marks a favourite at once; **F** marks or unmarks the chosen
map.

The selected map shows a large picture with numbered start positions, its source and state,
its exact size (in the menus' units of 512 pixels, from the terrain's header; 274 of the 275
sizes TA's maps type into their descriptions agree), players, world, wind, tides, gravity,
an acid sea or a lava world, and its description without the typed size.

- **Import Maps** accepts TA map archives and ZIP, 7z or RAR packages. Map-only content is
  checked before installation, copied into an isolated workspace, and installed as a
  separate pack. Nested archives must be extracted individually. Packages containing
  gameplay modifications belong in Mods.
- **Activate / Deactivate** affects new battle choices for the current profile. Preferences
  identify both the map and its source, so a mod replacement and the original have
  separate settings. Campaigns, saved games and original files remain available.
- **Play** launches the selected available battle map against a computer player. **Set Up
  Skirmish** opens the game's skirmish page with the map chosen (`--skirmish-map <name>`).
  **Edit a Copy** opens it in the map editor; the editor saves a separate map mod. **New
  Map** opens the editor for a new map. These use the launcher's selected profile and saved
  settings.
- The star (on the card, in the details, or **F**) marks a favourite map for the current
  profile (whichever source supplies it): the favourites come first in the library.
  **Remove Map Pack** sends an imported map pack to the Recycle Bin after asking.
  **Manage Source in Mods** opens Mods at the mod or pack a map comes from.
- A disabled source must first be enabled in **Mods**. An overridden map identifies that
  another enabled source supplies its effective version. **Show Source Files** opens the
  containing folder without modifying the original data.

Map visibility only filters new local battle choices and the host's chooser. The full
map catalogue remains authoritative for network setup, campaign, scenario and editor
metadata; another player's local visibility preferences cannot change random start slots.
An empty available-map list prevents starting a new battle until a map is activated.

## Progress and compatibility

The [achievement catalogue](ACHIEVEMENTS.md) has 134 achievements in six categories
(campaign, skirmish, multiplayer, battle, industry, service) and four tiers, a few of them
secret. Conditions come from eligible gameplay, with persistent unlock dates, the place in a
battle each was earned, a picture of the moment, and one-time notices in game. Achievements
of the previous list kept their identifiers and dates; those it dropped are removed, and
whatever earlier play met is earned from the service record when the profile is opened.

Existing XP, ranks and campaign completion are preserved. Details that old releases did
not record are not invented; detailed match history and mode-specific achievements begin
when this release observes eligible play. Imported saves establish a baseline, so another
player's historical actions do not become new rewards. Repeated checkpoints, loading old
saves, reconnecting and alternative endings do not duplicate career awards. Technical
runs, recorded replays and spectators are excluded by the existing career eligibility
rules; a battle with cheats in it gives back what it had earned and earns nothing more.
Multiplayer achievements require a connected human opponent.

Career ledgers migrate from versions 1 to 3 to version 4 on write (event tallies, battle
records, maps and worlds won on, unlock details; battles voided for cheats, and what each
battle gave beyond the totals). Older executables reject the new ledger
instead of silently discarding achievements and detailed history. This is a
local profile format change; simulation, save/replay and network formats are unchanged.

## Validation for this pass

- `cargo test --release --locked -p ta_config -p ta_ui -p ta_launcher -p ta_game -p ta_tools --lib --bins`:
  488 passed, 12 existing opt-in tests ignored. This includes real map-archive import,
  profile handover and isolation, filtered-map network starts, legacy ledger migration,
  imported-save baselines, repeated results, co-op/PvP classification and achievement totals.
- `cargo check --workspace --all-targets --locked` and the optimized launcher/game/tools
  build pass. The workspace check retains the existing unused-field warning in the
  unrelated `crowd_jitter` test.
- Native launcher captures cover the profile menu, all four dossier tabs, real installed
  map previews, German at 150% UI scale and Japanese achievements. Game captures cover
  the dossier and achievements. Populated career data in these captures is a synthetic
  QA fixture; the captures leave its ledger byte-for-byte unchanged.
- All 2,980 active strings are populated in each of the 13 non-English languages,
  with matching placeholders and all previously populated active translations preserved.
- Windows installer, portable archive and symbols are rebuilt. Archive integrity,
  SHA-256 manifests and packaged binaries are checked; no original TA archives are shipped.

GitHub's Quality workflow is configured for manual dispatch only. These are local Windows
checks; this pass does not claim a fresh Linux run or a played network endurance test.
