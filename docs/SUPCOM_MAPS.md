# Supreme Commander maps

TA Reborn can make the maps of **Supreme Commander: Forged Alliance** into its own, to play in
skirmish. The maps are made on the player's computer from their own copy of the game (GOG's or
Steam's), into a map pack in their mods folder; nothing of Supreme Commander's is shipped with
TA Reborn or kept in this repository.

The import was an experimental feature (Settings, LAUNCHER: ENABLE EXPERIMENTAL FEATURES) up to
1.0.9; it is now offered to every player.

- **Players:** the launcher's Maps page, **SUPREME COMMANDER…** (its MAPS tab; the Mods page's
  opens the same dialog at its FACTIONS; with nothing of the game imported yet, at its wizard,
  which makes every part in one go: [below](#the-wizard-and-update-all)). The dialog finds the
  game (a folder chosen with CHANGE FOLDER…, or the copy switched to, is kept for the next start
  in the launcher's settings folder, `supcom.toml`), lists its skirmish maps and the player's own
  (downloaded maps and Forged
  Alliance Forever's vault) with the game's own pictures of them, and makes the chosen ones, one
  after another (at first, where the pack holds maps an older converter made, those alone are
  chosen, listed first, to be made again: its update; else, while the pack holds none of them,
  the game's own maps; once it holds some, none (twenty minutes of work is never one button away);
  the player's own never are; after an import, only those not made stay chosen, to try again;
  once the pack is removed, they are chosen afresh): most in seconds, the largest in about a
  minute; all 54 of the game's take about 20 minutes and 1.2 GB on disk (the dialog says both
  for the maps chosen), and the converter needs under 1 GB of memory at most. Without the game
  (most players), the dialog says what it needs (the player's own installed copy of Forged
  Alliance: GOG's Gold Edition or Steam's), that nothing of it is downloaded or shipped, and
  offers both stores' pages, CHOOSE FOLDER… and LOOK AGAIN. The import checks first that
  the disk has room for them (the maps the pack holds are kept by linking their files, not
  copying them), and goes on with the dialog closed: the launcher's footer shows how far it has
  got, and a click there opens the dialog again. STOP AFTER THIS MAP ends it once the map in hand
  is made; a map that cannot be written (the disk full) ends it there, said against that map;
  closing the launcher asks first, then ends it at once, the map in hand left out; an update of
  the launcher waits for it. Each way the maps made become the map pack **Supreme Commander maps**
  (`map-pack-supreme-commander`), turned on, in the map library and in skirmish: made again where
  the launcher finds it (a copy kept in the extra mods folder there; none behind a copy in TA
  Reborn's program folder), and in place of another mod of that id only once the player agrees.
  Importing again makes the maps chosen again and keeps the others; REMOVE MAP PACK removes them
  all. The pack's `version` is that of the converter that made its oldest map: older than this TA
  Reborn's (`convert::VERSION`), the Mods page marks it UPDATE, as the dialog marks each map (and
  says how to update it); it needs the TA Reborn that made it (`requires_reborn`).
  On every tab, a mod of an import's that is installed but turned off is said to be so, with TURN
  ON (as its switch on the Mods page); on the Mods page an import's card shows its kind (MAPS,
  FACTIONS, MUSIC, MENUS) on its banner, the word its name is cut before there.
- **Developers:** `cargo build --release -p ta_tools`, then `target/release/ta-tools supcom
  <out> [--only scmp_009,x1mp_001] [--preview <dir>] [--game <folder>] [--budget n]
  [--max-cells n]` makes the maps into `<out>/maps` (mount it as a mod to play them), with
  pictures of each beside the game's own preview, and prints for each its size, world, tiles, how
  much its tiles step at their edges (TA Reborn's own maps are held to 1.33), what stands on it,
  the cells changed to give tanks a way and the deposits whose ground was made one extractors
  stand on (`TA_SUPCOM_TIMES=1`: how long each step took; `--budget`, 28,000 tiles by default,
  1 to 65,536: a place names its tile in 16 bits).
  The launcher's dialog for captures: `--page maps --show supcom` (`supcom:SCMP_012,SCMP_013`
  imports those once the game is found; `TA_SUPCOM_SHOT_IMPORTING=1` takes the picture while they
  are made).

The code: `crates/ta_supcom` (finding the game, reading it, converting), `crates/ta_mapart` (the
art in TA's form, shared with TA Reborn's own map generator), `apps/ta_launcher/src/supcom.rs` and
`pages/supcom.rs` (the dialog), `supcom_wizard.rs` and `pages/supcom_wizard.rs` (its wizard).

## The wizard, and UPDATE ALL

The four imports (the maps here, the factions in [SUPCOM_UNITS.md](SUPCOM_UNITS.md), the music in
[SUPCOM_MUSIC.md](SUPCOM_MUSIC.md), the menus' look in [SUPCOM_MENUS.md](SUPCOM_MENUS.md)) each
have their tab, their choice and their import. The dialog's wizard makes them in one go:

- **When it shows.** With none of the four imports' mods installed (named as their imports name
  them; another mod of an id is not one), SUPREME COMMANDER… opens the wizard rather than the tabs.
  The Mods page offers it once, in a card over the library, where Forged Alliance is found on the
  computer (looked for on a worker the first time the page is shown in a launch) and nothing of
  it is imported: IMPORT… opens it, NOT NOW puts the card away; either way it is not offered again
  (`offered = true` in the launcher's `supcom.toml`, beside the game's folder as chosen). Nothing
  ever opens by itself.
- **Its steps** (named in a row at its top: FORGED ALLIANCE, WHAT TO MAKE, IMPORTING, DONE).
  *The game:* the copy found (GOG's, Steam's or a folder, a switch each where there
  are several) with CHANGE FOLDER…, and a picture of each part made of it; without the game, the
  card that says what is needed, its stores, CHOOSE FOLDER… and LOOK AGAIN (which looks again for
  every part). *What to make:* a card for each part, ticked, in the order they are made (the
  factions, the music, the menus' look, the maps), each saying what it makes (all four factions and
  their units; the soundtrack's tracks; the menus' pictures and sounds; the game's own skirmish
  maps, not the player's own, which the MAPS tab offers), what it takes on the disk and about how
  long, by its tab's own estimates (`time_estimate`, `size_estimate`), and the total; a part that
  cannot be made says why and is held unticked (TA's own install not found, for the maps; no
  tracks in the copy; a copy of its mod an import cannot replace). CHOOSE IN DETAIL… opens the
  tabs. *Importing:* every part ticked, one after another, in one view: each waiting, being made
  (its tab's own progress and words), made (its import's own words) or not (why). *Done:* what
  each came to, turned on, with the settings it set (MENU THEME and SOUNDTRACK, said in the
  imports' own words), a note that the tabs make or remove each part on its own, and PLAY A
  SKIRMISH (the game opened at a skirmish's setup; PLAY where neither factions nor maps were
  made).
- **The import.** The wizard imports nothing itself: `LauncherApp::wizard_tick`, from the app's
  update where the imports are polled, starts each part once the one before has ended. It waits
  for its tab's look (made again where it is not current), sets its tab's choice (the maps chosen,
  the sides chosen), then starts its tab's own import (`begin_supcom_import`, as IMPORT on the tab
  does: the mod made again where the launcher finds it, a question first over another mod of its
  id). A part waits, said why, for what passes (the game started, a mod being installed) and is
  left out, said why, for what does not (KEEP at the question, a copy in TA Reborn's program
  folder). The maps come last: the game's 54 take twenty minutes, and the others a minute. STOP
  ends it once the part in hand is made (the factions, the music and the menus' look are finished,
  their imports installing nothing when stopped); with the maps in hand, their import stops after
  the map in hand, those made installed. The dialog may be closed meanwhile: the footer's chip shows
  the part in hand and opens the wizard again; the parts' own notices are left to one at the end,
  and the wizard opens at what it came to once. The game does not start while it waits between
  parts.
- **After a first import** SUPREME COMMANDER… opens the tabs as before, which say at their top what
  is not imported yet, with IMPORT IN ONE GO… (the wizard at what to make, with those parts only).
- **UPDATE ALL.** Where any of the imports' mods was made by an older converter (`update_tab`), the
  Mods page says so in one bar over the library ("Supreme Commander imports were made by an older
  TA Reborn: …", UPDATE ALL), and the tabs at their top. It opens the wizard straight at its
  import, of those parts only: the maps an older converter made (the player's own among them; a
  map whose folder is gone stays in the pack as it was), the factions the mod holds (the same
  sides), the music, the menus' look. The music's and the menus' settings follow as their imports
  make them follow. Each mod's own UPDATE… on the Mods page still opens its tab.
- **Captures.** `--show supcom-wizard` (its first step, whatever is imported), `:choose` (what to
  make), `:import` and `:done` (its import of every part begun; `TA_SUPCOM_SHOT_IMPORTING=1` takes
  the picture during it), a third field naming the maps the maps' part makes
  (`supcom-wizard:done:SCMP_009,SCMP_003`: all 54 take twenty minutes); `--show supcom-rest[:choose…]`
  the parts not imported yet; `--show supcom-update[:<maps>]` UPDATE ALL.
- **Tests:** `supcom_wizard::tests` (the parts' order, the import told as it went) and
  `pages::supcom_wizard::tests` in the launcher: the steps walked, the queue's order and each
  part's choice set before it starts, STOP (after the part in hand; the maps after the map in
  hand), KEEP at the question, UPDATE ALL's parts and choices, the tabs' IMPORT IN ONE GO…, the
  Mods page's offer made once, and every step's buttons in sight in a small window (imports
  pretended, `Import::pretend_start`, never this computer's copy).

## Finding the game

`ta_supcom::install`. A folder is the game when it holds `gamedata/env.scd` and `maps/` (any
case). Looked for, in order:

- GOG: product 1444785261 (Forged Alliance; GOG's Gold Edition installs it under the same id),
  its registry key's `path`; off Windows, Heroic's record of its GOG installs.
- Steam: app 9420, its uninstall key (Steam writes none for it), then `steamapps/common/Supreme
  Commander Forged Alliance` (or the folder its app manifest names) in every library of every
  Steam there is. Steam records its own folder as `c:/program files (x86)/steam`: the folders
  found in it are spelt as Windows spells them (`ta_vfs::install`), as the dialog shows them.
- The boxed copy: THQ's key `SOFTWARE\[WOW6432Node\]THQ\Gas Powered Games\Supreme Commander -
  Forged Alliance`.
- The usual folders on C: to F:, and off Windows the same in Wine's, Lutris', Bottles', Heroic's,
  CrossOver's and Proton's prefixes.
- A folder the player picks (or one level down in it), looked through on a worker and kept in
  the launcher's settings. The original Supreme Commander is laid out as Forged Alliance is:
  Forged Alliance is told from it by its own maps in its maps folder (`X1MP_…`, `X1CA_…`) or by its
  data path naming it (`bin/SupComDataPath.lua`), and is taken first where a folder holds both;
  the dialog says when the folder chosen holds the original. A folder chosen before that no longer
  holds the game is said so, the copy found used meanwhile.

The players' own maps: `<Documents>/My Games/Gas Powered Games/Supreme Commander Forged
Alliance/maps` (Documents where Windows keeps it: often a OneDrive folder; Forged Alliance
Forever's client keeps its vault there too unless moved, as its `client.prefs` records:
`vaultBaseDirectory`, `mapsDirectory`), the older FAF client's copy of it under
`ProgramData\FAForever\user`, and on Linux the same in the game's Proton prefix and in the home
folder. With none found, the dialog names the folder it looks in first.

Steam's and GOG's copies are laid out alike (`bin/SupremeCommander.exe`, `gamedata/*.scd`,
`maps/`); FAF's client runs the same files. Both read whole, and make the same maps byte for
byte (checked with both installed: Steam's build 2845). With more than one copy found, the
dialog shows each as a switch (GOG, Steam), to choose the one it reads; a folder picked that is
one of them keeps its name. `cargo test --release -p ta_supcom --test installed every_copy_found
-- --ignored --nocapture` lists the copies found.

Where a map comes from is shown wherever maps are chosen (`ta_data::map::Origin`): its OTA's
`supcommap` makes it one made from Supreme Commander's, whatever folder it is in. Skirmish's and
the lobby's lists tag it SUPCOM (and can show those alone); the launcher's map library puts a
SUPREME COMMANDER ribbon on its card and a note in its details.

## Reading the game

All verified on the GOG copy: every map reads to its last byte (`cargo test --release -p
ta_supcom --test installed -- --ignored --nocapture` reads them all, `TA_SUPCOM_DIR` another
copy).

- **`.scd` archives** (`ta_supcom::scd`): plain zip files, almost every member stored. A reader
  of our own: the directory read once, members read by position (threads share an archive), any
  case and either slash, CRC checked.
- **Files a map names** (`ta_supcom::files`), found whatever their case (91% of the paths in the
  game's maps only match ignoring case): a map's own files in the folders of maps (the player's
  first, then the game's `maps/`), the rest in the game's `gamedata/*.scd` and FAF's `.nx2`
  archives where FAF is installed, then loose in the game's folder. The game's own maps read the
  game's archives first, so that they come out the same on every computer, FAF or not (FAF
  changes some of the game's files); a player's own maps read FAF's first (a map of its vault may
  use its newer textures). A folder behind a junction or a symbolic link (a game moved to another
  drive) is followed.
- **Lua data** (`ta_supcom::lua`): the scenario (`ScenarioInfo = {...}`), save (`Scenario = {...}`)
  and blueprint (`PropBlueprint {...}`) files, read as data: tables, numbers (`6.75917e-005`),
  strings (long ones too, `[[ ]]`, `[==[ ]==]`), booleans, and calls (`STRING()`, `FLOAT()`,
  `BOOLEAN()`, `VECTOR3()`, `RECTANGLE()`, `GROUP {}`, `Sound {}`); comments `--`, `--[[ ]]`,
  `--[==[ ]==]` and `#`; a blueprint's `''` for a missing number. Tables and calls nested more
  than 64 deep are refused (a player's map from the vault is anybody's file). Of a map's script,
  only the calls it makes are looked for, what is commented out left out.
- **The scenario**: the name (plain), the description (its `<LOC key>` dropped), `type`
  (`skirmish`, `campaign`), `size` in world units ("ogrids": 256 is "5 km"), the files, and the
  armies (`Configurations.standard.teams`). The game's `X1CA_TUT` names another map's `.scmap`.
- **The save file**: the markers (`MasterChain._MASTERCHAIN_.Markers`): `Mass` and
  `Hydrocarbon` (resources), `Blank Marker`s named for the armies (the starts); the armies'
  units (the civilians' walls, buildings and wrecks, `GROUP`s nested); the areas. Winter Duel and
  Fields of Isis limit the battle to an area in their scripts (`SetPlayableArea('AREA_1')`):
  only that area is made. 25 of the game's scripts carry the call commented out
  (`--ScenarioFramework.SetPlayableArea(...)`): those maps are made whole.
- **The `.scmap`** (`ta_supcom::scmap`), versions 60 (Forged Alliance, and the original game's
  maps re-saved) and 56 (the original game's editor, many of FAF's maps; 60 less the unknown
  float after the minimap's colours and the sky):

  | Block | What |
  |---|---|
  | header | `Map\x1a`, major version 2, `0xBEEFFEED`, 2, width and height (f32), 0, 0 (i16) |
  | preview | length + DDS (256², A8R8G8B8) |
  | version | i32: 56 or 60 |
  | heightmap | width, height (i32), scale (f32, 1/128); u16 × (w+1)(h+1), rows by z; a 0 byte |
  | shaders | terrain shader, background, sky cubemap (C strings); environment cubemaps (count, name + path) |
  | lighting | multiplier; sun direction, ambience, sun colour, shadow fill (vec3); specular (vec4, garbage in many maps); bloom; fog colour, start, end |
  | water | has water (u8); surface, deep, abyss elevations; shading (80 bytes, the colour first); cubemap, ramp |
  | waves | 4 repeat rates; 4 × (movement vec2, texture); wave generators (count × 2 strings + 68 bytes) |
  | minimap | contour interval and 5 colours (24 bytes); (60) an unknown f32 |
  | layers | 10 albedo (path, scale: lower stratum, 8 strata, upper) and 9 normal (path, scale) |
  | decals | 2 counters; count × (id, type, texture count, length-prefixed textures, scale, position, rotation (vec3s), 2 cut-offs, army) |
  | decal groups | count × (id, name, count, ids) |
  | normal map | width, height, count × (length + DDS): one, or four 2048² tiles on the 81 km maps |
  | masks | strata 1-4, strata 5-8 (length + DDS each, A8R8G8B8, half size) |
  | water map | 1, length + DDS; foam, flatness, depth bias (u8 × half size each) |
  | terrain types | u8 a world unit |
  | sky | (60) its block, to step over |
  | props | count × (blueprint, position, x, y, z axes, scale) to the file's end |

- **Painting** as the game's terrain shader does (`convert::paint`, after the game's
  `effects/terrain.fx`): the lower stratum; over it, in order, each of the eight strata by its
  mask (mask 0's R, G, B, A: strata 1-4; mask 1's: 5-8; the original game's shaders, `TTerrain`
  and `TTerrainGlow`, read mask 0 alone and four strata; Forged Alliance's `TTerrainXP` and FAF's
  later ones, `Terrain250`, `Terrain301`..., eight) at `clamp(2m − 1, 0, 1)`; the upper stratum
  by its own alpha. A stratum repeats once every `scale` world units; mask texel i covers world
  units 2i to 2i + 2. Albedo decals (type 1) lie with their corner at `position`, `scale.x` along
  their own x and `scale.z` along z, turned `rotation.y` radians, blended by their alpha. Lit as
  the shader lights it: `l = sun colour × max(0, N·sun) + ambience`, then `multiplier × l +
  shadow fill × (1 − l)`, times the colour (the normal from the heightmap's slopes; most of the
  game's maps have no ambience, their slopes facing away from the sun lit by the fill alone).
  Under the water (the surface at its elevation in world units, heights raw × 1/128) the
  ground takes the water ramp's colour at `(surface − h)/(surface − abyss)` by the ramp's alpha.
- **Textures** (`ta_supcom::dds`): DXT1, DXT3, DXT5, and uncompressed 8 to 32-bit by masks; one
  mip level decoded, the one that suits the map's scale.

## Making a TA map

`ta_supcom::convert`. A map converted from the same files by the same version of the converter
is the same, byte for byte, on every computer: players compare their maps' bytes before a
battle. Nothing in it uses a platform's own powers, roots or trigonometry (`ta_mapart::tiles::
Portable`, `convert::trig`), and nothing depends on thread order.

- **Scale** (`land::Frame`): 16 TA pixels a world unit. A T1 tank, a mass extractor and a wall
  are about one world unit in the game, so its units come out as big beside the land as TA's
  own: 5 km maps become 8 × 8, 10 km 16 × 16, 20 km 32 × 32. Larger maps are made smaller to fit
  1,024 cells a side (40 km at 8 pixels a unit, 81 km at 4), the heights averaged over each cell.
- **Heights** (`land::build`): pressed into TA's 0-255. Land and sea share a scale (at most the
  map's own, 16 to a world unit at full scale) unless that leaves the game's deep water (its
  `deep` elevation) shallower than 20, too shallow for TA's warships; then the sea is deepened and
  the land given the height left. TA's sea level is the game's water surface.
- **Cliffs** (`land::cliffs`): a cell of the game whose corners differ by more than 0.75 world
  units is too steep for its land units (its own rule). Pressing the heights into 0-255 makes
  every slope gentle, so the cells the game's units could not cross are marked as cliffs in the
  TNT (`CELL_CLIFF`, docs/FORMATS.md): too steep for anything of TA's that minds slopes, and for
  building. At full scale a TA cell is the game's cell, and the slopes the game's units climbed
  stay within what TA's tanks climb (at most 12 of their 15). The 40 and 81 km maps, made
  smaller, put 2 × 2 and 4 × 4 of the game's cells in one of TA's: a cell is a cliff where most
  of them are; one where half or fewer are, across a cliff's edge, is not marked but keeps the
  slope its averaged heights give (up to some hundreds of cells a map steeper than 15, the
  steepest about 45), which TA's own rules of slope then judge: no tank climbs it, a kbot may.
- **World** (`kit::choose`): the environment the map's strata and props come from most (a
  stratum counting as much as a hundred props), as a world of TA's: evergreen is green,
  tropical archipelago, swamp lush, desert desert, tundra ice, lava lava, red rocks the red planet,
  crystalline crystal, geothermal slate. Its `planet` gives the renderer's sky, weather and
  ground materials; its features stand in for the game's. Where this copy of TA lacks a world's
  features, green.
- **What stands on it** (`place::place`):
  - each mass point is a 3 × 3 metal deposit of the world's richest (about 2 metal a second to an
    extractor, as a mass point gives in the game; where the water is deeper than 9 at its middle,
    its sea floor's), moved a little where two would overlap. Its ground is made one TA's
    extractors stand on where it is not (they want dry ground no steeper than 10 and no cliff,
    or water at least 19 deep): its corners' heights moved as little as that takes, each within
    5 of their mean and out of the water, its cliff marks taken away; under water, deepened;
  - each hydrocarbon point a geothermal vent, its plant's site kept clear;
  - the civilians' walls are dragon's teeth over all the cells each section covers: the
    factions' sections (`ueb5101`, `uab5101`, `urb5101`, `xsb5101`, one unit across) and Forged
    Alliance's civilian wall segments (UEF's `xec80xx`, the Aeon's `xac80xx` and `xac81xx`, the
    Cybran's `xrc80xx` and `xrc81xx`, the Seraphim's `xsc80xx`, three across). Their buildings
    and wrecks are left out;
  - the props: each blueprint read for what it is (its script class, whether it blocks the way,
    its reclaim, its size, its name, what it is drawn with). Trees (and groups of them, as
    several trees), bushes and rocks become the map's own features (`maps/<stem>.features`,
    docs/FORMATS.md): TA's art of the world under names of their own (`SC1_<World>_T1`, `_B1`,
    `_R3M`), passable where the game's forests were (TA's trees block), a tree worth 10 energy and
    a rock metal by the game's reclaim in tiers, about half (up to 10 mass: 5 metal; to 40: 20; to
    120: 60; more: 150), rocks blocking where the game's did. What one leaves when destroyed is
    TA's remains of its template, never worth more than it nor blocking more (TA's rocks crack
    into rocks as big and as rich, its Martian ones into larger: those steps taken over under the
    map's own names, `_D1`, `_D2`, held to it), then TA's rubble or scar; trees do not burn.
    Markers, emitters and geysers are left out, and so are the cities' props: the factions'
    (`/env/uef/`, `/env/aeon/`, `/env/cybran/`, `/env/seraphim/`: buses, cars, docks, fences,
    street lights, adjacency nodes) and anything drawn as the factions' units are (the `Unit`,
    `Aeon`, `Insect`, `Seraphim` shaders), whatever their script class (the fences and street
    lights are `Tree`s to the game) or reclaim (300 mass a bus);
  - nothing within six cells of a start.
- **Ways for tanks** (`place::connect`): where the game's land units could drive from one start to
  another and a 3 × 3 TA tank cannot (the game's units are smaller, so some of its passes are too
  narrow, and its rocks stand closer), the cheapest way between them is opened where a tank could
  not pass, three cells wide and a few cells either side: its cliffs unmarked, its ground laid no
  steeper than a tank climbs, the blocking features in it taken away. The cliff's cells a way
  crosses are painted as the ground nearest each, level (`Painter::open_ways`).
- **The art** (`art`): the ground painted in TA's view (`ta_mapart::slant`: TA's camera saw a place
  `h / 2` pixels up the picture), the strata decoded a level finer than a texel a pixel (the grain
  survives at TA's scale), lit as the game's shader lights it (above), and matched in colour on
  land to the game's own preview of the map: the mean of each channel over the land, on a grid of
  48 × 48 places (192 × 192 where that finds too little land: maps mostly sea), scaled to the
  picture's, by 0.75 to 2 and one channel against the others by 12% at most (the game's pictures
  are brighter than its shader lights its maps, most by a quarter to two fifths:
  `how_each_map_matches_its_preview`). A map that is not square fills the middle of its square
  picture, black above and below or either side: it is matched there. Then cut into 32-pixel
  tiles. A map with room for every place's own tile keeps them all; a larger one has the alike
  shared within a budget (28,000 tiles: TA Reborn's own largest maps have about 24,000), the cliffs'
  kept whole, their seams settled over twelve rounds (`ta_mapart::tiles`, as TA Reborn's own maps
  are made; each place's candidates kept to the 64 most alike as they are found). Each tile is
  dithered into TA's palette.
- **The OTA**: the map's name and description (its size first, as TA's maps' open: `32 X 32  `,
  the menus showing the map's own in its place), its world's planet, its size in 512-pixel
  squares, rounded (`8 x 5`), the starts (at most TA's ten), tidal 20 where there is water, wind
  500 to 2500, surface metal 3, and `rebornart=1`, `supcommap`, `supcomversion` (docs/FORMATS.md;
  `convert::VERSION`, raised whenever what the converter makes changes, so that the launcher
  offers to make maps made before again). A map whose markers leave fewer than two starts is
  refused: no battle is fought on it.
- **Its file stem**: its name and `(SC)` (`Seton's Clutch (SC)`); a player's own map's folder
  too, whole (`Seton's Clutch (SC setons_clutch v0003)`: versions of a map share all but the end
  of their folders' names), past 48 characters cut with a mark of the whole name (`~` and six hex
  digits of its FNV-1a). The same for every player who has the map: the lobby finds maps by it.
  An import never writes over another map's files of the same name; it says so instead.

## Banners

Each import's mod has a banner, the picture the launcher's Mods library shows it by on its card
and its page (`media/banner.png`, named by its manifest's `[media] banner`), 1280 by 640, made by
the import from the player's own copy of the game as the mod itself is (`ta_supcom::banner`):

- **The map pack's:** the game's pictures of the maps it holds, as cards in a row over the game's
  loading rays, the middle one facing, the others turned away and darker further out. The maps of
  `banner::SHOWCASE` come first where the pack has them (Seton's Clutch in the middle), then the
  rest by folder. A pack of a few maps shows them round and round along the row, one alone stands
  by itself. Each picture is found by the folder its map was made from (its OTA's `supcommap`), in
  the game's maps folder or the player's own.
- **The factions':** the game's hall of the factions (its campaign's, with their banners), and
  along its floor the emblems of the factions the mod holds, on medallions of their colours.
- **The music's:** the menu theme's loudest eight seconds as a row of bars, amber to the game's
  cyan, mirrored on a line of light, over the game's painting of a fleet under a red sky.
- **The menus':** the game's logo, and a panel of its menus holding one of its paintings, over the
  space its menus show.

A banner is made in whole numbers, the same from the same copy on every computer, in about a
second for all four. Neither `media/` nor the manifest counts in the digests saved games, replays
and multiplayer compare. Where the copy lacks the pictures a banner is made of, the mod has none,
and the library draws its plate with its name.

**Mods imported before banners were made.** The launcher makes theirs (`supcom_banners.rs`): once
a launch, on a worker, while no import or install is at work, for each import's mod it lists
without a banner (named as the import names it; not an archive, nor a copy in TA Reborn's program
folder), from the copy in use. It writes `media/banner.png` into the mod's folder and adds the
`[media]` table to the end of its manifest, the rest kept as it is (a manifest with a `[media]`
table of its own is left alone), then reads the mods again. Without the game, nothing changes.

**Tests:** `banner::tests` in `ta_supcom` (pictures made smaller and larger evenly, the PNG read
back, a medallion, a turned card, the showcase's order, the loudness, the four banners made of a
copy written in the tests); `supcom_banners::tests` and
`supcom::tests::the_packs_banner_shows_its_maps` in the launcher. From the player's copy:
`cargo test --release -p ta_supcom --test installed the_banners_of_the_installed_copy -- --ignored
--nocapture` makes them all (`TA_BANNER_OUT=<dir>`: written there to look at).

## Multiplayer

The pack holds maps alone (`maps/*.tnt`, `.ota`, `.features`): it is no part of the data
multiplayer peers compare, so a player with it plays anyone on other maps. Two players can play a
converted map together when both made it with the same version of TA Reborn's converter (its
digest covers its `.features`); the lobby says when a player's differs. A battle plays with the
game's features and its own map's alone (`GameData::for_map`), so the features' numbers, which
the simulation's checksum and a saved game's replay go by, are the same whichever other maps
each player has imported.

## Tests

- `cargo test -p ta_supcom`: the readers on maps, archives, textures and data made in the tests
  (none of the game's), the converter end to end on a small map (twice, the same bytes, its
  features' too).
- `cargo test --release -p ta_supcom --test installed -- --ignored --nocapture`: every map of the
  player's copy read; `every_installed_map_converts` (about twenty minutes) makes every skirmish
  map and checks that each deposit takes an extractor, the starts the game's land units drive
  between are joined for a tank, what the map's own features leave is held to them, and the 5 km
  maps come out the same on three threads as on all; `how_each_map_matches_its_preview` (seconds)
  shows each map's colours against the game's picture of it and the gain that matched them.
- `cargo test -p ta_data own_features`, `cargo test -p ta_sim a_maps_cliffs`, `cargo test -p
  ta_formats cell_marks`, the editor's `a_copy_of_a_converted_map_keeps_its_own_features_and_cliffs`.
- In the game: mount `ta-tools supcom`'s output as a mod and play it (`--map "Seton's Clutch
  (SC)"`); the AI soak test plays one (`TA_TEST_MOD=<folder> TA_TEST_MAP="..."`).
