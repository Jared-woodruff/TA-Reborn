# Supreme Commander maps

TA Reborn can make the maps of **Supreme Commander: Forged Alliance** into its own, to play in
skirmish. The maps are made on the player's computer from their own copy of the game (GOG's or
Steam's), into a map pack in their mods folder; nothing of Supreme Commander's is shipped with
TA Reborn or kept in this repository.

- **Players:** the launcher's Maps page, **SUPREME COMMANDER…**. The dialog finds the game,
  lists its skirmish maps and the player's own (downloaded maps and Forged Alliance Forever's
  vault) with the game's own pictures of them, and makes the chosen ones, one after another.
  The largest take about half a minute each; all 54 of the game's take about 15 minutes and
  1.2 GB. They become the map pack **Supreme Commander maps** (`map-pack-supreme-commander`),
  turned on, in the map library and in skirmish. Importing again makes the maps chosen again
  and keeps the others; REMOVE MAP PACK removes them all.
- **Developers:** `cargo build --release -p ta_tools`, then `target/release/ta-tools supcom
  <out> [--only scmp_009,x1mp_001] [--preview <dir>] [--game <folder>] [--budget n]
  [--max-cells n]` makes the maps into `<out>/maps` (mount it as a mod to play them), with
  pictures of each beside the game's own preview, and prints for each its size, world, tiles, how
  much its tiles step at their edges (TA Reborn's own maps are held to 1.33), what stands on it,
  and the cells changed to give tanks a way (`TA_SUPCOM_TIMES=1`: how long each step took).
  The launcher's dialog for captures: `--page maps --show supcom` (`supcom:SCMP_012,SCMP_013`
  imports those once the game is found; `TA_SUPCOM_SHOT_IMPORTING=1` takes the picture while they
  are made).

The code: `crates/ta_supcom` (finding the game, reading it, converting), `crates/ta_mapart` (the
art in TA's form, shared with TA Reborn's own map generator), `apps/ta_launcher/src/supcom.rs` and
`pages/supcom.rs` (the dialog).

## Finding the game

`ta_supcom::install`. A folder is the game when it holds `gamedata/env.scd` and `maps/` (any
case). Looked for, in order:

- GOG: product 1444785261 (Forged Alliance; GOG's Gold Edition installs it under the same id),
  its registry key's `path`; off Windows, Heroic's record of its GOG installs.
- Steam: app 9420, its uninstall key, then `steamapps/common/Supreme Commander Forged Alliance`
  (or the folder its app manifest names) in every library of every Steam there is.
- The boxed copy: THQ's key `SOFTWARE\[WOW6432Node\]THQ\Gas Powered Games\Supreme Commander -
  Forged Alliance`.
- The usual folders on C: to F:, and off Windows the same in Wine's, Lutris', Bottles', Heroic's,
  CrossOver's and Proton's prefixes.
- A folder the player picks (or one level down in it).

The players' own maps: `<Documents>/My Games/Gas Powered Games/Supreme Commander Forged
Alliance/maps` (Documents where Windows keeps it: often a OneDrive folder), FAF's copy of it
under `ProgramData\FAForever\user`, and on Linux the same in the game's Proton prefix and in the
home folder.

Steam's and GOG's copies are laid out alike (`bin/SupremeCommander.exe`, `gamedata/*.scd`,
`maps/`); FAF's client runs the same files.

## Reading the game

All verified on the GOG copy: every map reads to its last byte (`cargo test --release -p
ta_supcom --test installed -- --ignored --nocapture` reads them all, `TA_SUPCOM_DIR` another
copy).

- **`.scd` archives** (`ta_supcom::scd`): plain zip files, almost every member stored. A reader
  of our own: the directory read once, members read by position (threads share an archive), any
  case and either slash, CRC checked.
- **Files a map names** (`ta_supcom::files`), found whatever their case (91% of the paths in the
  game's maps only match ignoring case): a map's own files in the folders of maps (the player's
  first, then the game's `maps/`), the rest in FAF's `.nx2` archives where FAF is installed, then
  the game's `gamedata/*.scd`, then loose in the game's folder.
- **Lua data** (`ta_supcom::lua`): the scenario (`ScenarioInfo = {...}`), save (`Scenario = {...}`)
  and blueprint (`PropBlueprint {...}`) files, read as data: tables, numbers (`6.75917e-005`),
  strings, booleans, and calls (`STRING()`, `FLOAT()`, `BOOLEAN()`, `VECTOR3()`, `RECTANGLE()`,
  `GROUP {}`, `Sound {}`); comments `--`, `--[[ ]]` and `#`; a blueprint's `''` for a missing number.
- **The scenario**: the name (plain), the description (its `<LOC key>` dropped), `type`
  (`skirmish`, `campaign`), `size` in world units ("ogrids": 256 is "5 km"), the files, and the
  armies (`Configurations.standard.teams`). The game's `X1CA_TUT` names another map's `.scmap`.
- **The save file**: the markers (`MasterChain._MASTERCHAIN_.Markers`): `Mass` and
  `Hydrocarbon` (resources), `Blank Marker`s named for the armies (the starts); the armies'
  units (the civilians' walls, buildings and wrecks, `GROUP`s nested); the areas. Winter Duel and
  Fields of Isis limit the battle to an area in their scripts (`SetPlayableArea('AREA_1')`):
  only that area is made.
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

- **Painting** as the game's terrain shader does (`convert::paint`): the lower stratum; over it,
  in order, each of the eight strata by its mask (mask 0's R, G, B, A: strata 1-4; mask 1's:
  5-8; the original game's 4-stratum shader reads mask 0 alone) at `clamp(2m − 1, 0, 1)`; the
  upper stratum by its own alpha. A stratum repeats once every `scale` world units; mask texel
  i covers world units 2i to 2i + 2. Albedo decals (type 1) lie with their corner at `position`,
  `scale.x` along their own x and `scale.z` along z, turned `rotation.y` radians, blended by their
  alpha. Under the water (the surface at its elevation in world units, heights raw × 1/128) the
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
  building. Passable slopes stay within what TA's tanks climb (at most 12 of their 15).
- **World** (`kit::choose`): the environment the map's strata and props come from most (a
  stratum counting as much as a hundred props), as a world of TA's: evergreen is green,
  tropical archipelago, swamp lush, desert desert, tundra ice, lava lava, red rocks the red planet,
  crystalline crystal, geothermal slate. Its `planet` gives the renderer's sky, weather and
  ground materials; its features stand in for the game's. Where this copy of TA lacks a world's
  features, green.
- **What stands on it** (`place::place`):
  - each mass point is a 3 × 3 metal deposit of the world's richest (about 2 metal a second to an
    extractor, as a mass point gives in the game; under water its sea floor's), moved a little
    where two would overlap;
  - each hydrocarbon point a geothermal vent, its plant's site kept clear;
  - the civilians' walls are dragon's teeth (their buildings and wrecks are left out);
  - the props: each blueprint read for what it is (its script class, whether it blocks the way,
    its reclaim, its size, its name). Trees (and groups of them, as several trees), bushes and
    rocks become the map's own features (`maps/<stem>.features`, docs/FORMATS.md): TA's art of
    the world under names of their own (`SC1_<World>_T1`, `_B1`, `_R3M`), passable where the
    game's forests were (TA's trees block), worth what the game's props were (a tree 10 energy,
    a rock 5 to 150 metal by its worth), rocks blocking where the game's did; markers, emitters,
    geysers and the cities' clutter are left out;
  - nothing within six cells of a start.
- **Ways for tanks** (`place::connect`): where the game's land units could drive from one start to
  another and a 3 × 3 TA tank cannot (the game's units are smaller, so some of its passes are too
  narrow, and its rocks stand closer), the cheapest way between them is opened where a tank could
  not pass, three cells wide and a few cells either side: its cliffs unmarked, its ground laid no
  steeper than a tank climbs, the blocking features in it taken away.
- **The art** (`art`): the ground painted in TA's view (`ta_mapart::slant`: TA's camera saw a place
  `h / 2` pixels up the picture), the strata decoded a level finer than a texel a pixel (the grain
  survives at TA's scale), lit by the map's sun over the heightmap's slopes, and matched in
  colour on land to the game's own preview of the map (within limits); then cut into 32-pixel
  tiles. A map with room for every place's own tile keeps them all; a larger one has the alike
  shared within a budget (28,000 tiles: TA Reborn's own largest maps have about 24,000), the cliffs'
  kept whole, their seams settled (`ta_mapart::tiles`, as TA Reborn's own maps are made). Each
  tile is dithered into TA's palette.
- **The OTA**: the map's name and description, its world's planet, the starts (at most TA's ten),
  tidal 20 where there is water, wind 500 to 2500, surface metal 3, and `rebornart=1`,
  `supcommap`, `supcomversion` (docs/FORMATS.md).
- **Its file stem**: its name and `(SC)` (`Seton's Clutch (SC)`); a player's own map's folder too.

## Multiplayer

The pack holds maps alone (`maps/*.tnt`, `.ota`, `.features`): it is no part of the data
multiplayer peers compare, so a player with it plays anyone on other maps. Two players can play a
converted map together when both made it with the same version of TA Reborn's converter (its
digest covers its `.features`); the lobby says when a player's differs.

## Tests

- `cargo test -p ta_supcom`: the readers on maps, archives, textures and data made in the tests
  (none of the game's), the converter end to end on a small map (twice, the same bytes).
- `cargo test --release -p ta_supcom --test installed -- --ignored --nocapture`: every map of the
  player's copy read.
- `cargo test -p ta_data own_features`, `cargo test -p ta_sim a_maps_cliffs`, `cargo test -p
  ta_formats cell_marks`, the editor's `a_copy_of_a_converted_map_keeps_its_own_features_and_cliffs`.
- In the game: mount `ta-tools supcom`'s output as a mod and play it (`--map "Seton's Clutch
  (SC)"`); the AI soak test plays one (`TA_TEST_MOD=<folder> TA_TEST_MAP="..."`).
