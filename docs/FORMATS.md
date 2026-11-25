# Total Annihilation data formats — verified notes

Everything here was verified against the GOG Commander Pack data with `ta-tools`
(`validate`, `data`, `map`, `gaf`, `model`, `cob`, `missions`). Where community documentation
disagreed with the shipped data, the data won.

## Limits for untrusted content

Readers validate complete table extents before allocating from header counts. Invalid
binary content returns a format error rather than attempting the declared allocation.
These limits also bound repeated pointers to the same underlying bytes:

* HPI directories: 64 MiB decoded/indexing budget, 32 levels, 100,000 directory nodes, no shared/cyclic directory
  nodes or unsafe path components. Whole HPI files and loose VFS reads: 256 MiB.
  Compressed chunks: at most 128 KiB stored and 64 KiB decoded; decoders cannot expand
  beyond the chunk's declared output length.
* GAF decoded whole (`Gaf::parse`): 16 million pixels per frame and 256 MiB aggregate
  decoded frames/metadata, including subframes. Read a frame at a time (`GafReader`, the
  weapons' explosions and the mod validator): frame headers within the same 256 MiB; each
  frame and layer decoded costs its pixels, every time it is read, from an allowance of
  256 Mi pixels or 16 a byte of the file if more (a tiny file listing one huge empty frame
  thousands of times stops there); a reduced frame is at most 2048 texels a side, and while a
  frame is read the reader holds one of its rows and one row per depth of layers, never the
  frame. PCX: 16 million pixels and 64 MiB decoded scanlines.
* 3DO: 1,024 pieces, 64 child levels and 128 MiB decoded tables/texture names; repeated
  piece offsets are rejected. COB: 1,024 pieces, 65,536 functions, 4,096 static variables,
  and 64 MiB code/function-name budget. Binary names are at most 4,096 bytes.
* FNT: 16 MiB aggregate glyph pixels. TNT dimensions must be even, with at least one
  graphic tile; existing map size/table bounds still apply.
* TDF remains tolerant and infallible. Sections deeper than 64 levels are skipped with
  their entire subtree; parsing resumes after the matching brace. Their entries are
  never promoted into an outer section.
* Mod installation rejects symlinks and Windows reparse points, more than 64 directory
  levels, more than 100,000 filesystem entries, and more than 8 GiB of extracted files.
  External unpackers receive an empty destination; their own extraction protections
  remain required. HPI extraction rejects linked output paths and replaces files
  atomically so preexisting hard links cannot redirect writes.
* Mod manifests are at most 1 MiB and dependency nesting is at most 64 levels.
* Unit, feature and movement-class footprints are clamped to 1–256 cells per side;
  exceeding the ceiling emits a warning. Unit compilation also applies the ceiling
  before allocating yard occupancy, including for programmatically built definitions.
* Whole-track music decoding for tools is limited to 256 MiB of stereo PCM; untrusted
  container frame counts never drive a reservation. Streaming playback has no track
  length limit and continues decoding one packet at a time. Movie soundtracks are also
  capped at 256 MiB aggregate decoded PCM; excessive tracks play silently.

## Archives (HPI family)

* One container format for `.hpi`, `.ufo`, `.ccx`, `.gp3`, `.gpf`: `"HAPI"`, version
  `0x00010000`. Header key ≠ 0 means byte-level obfuscation
  `plain = ((pos ^ key) as u8) ^ !cipher` with `key = !((hk << 2) | (hk >> 6))`.
* Files are 64 KiB `SQSH` chunks, method 1 = TA's LZ77 (4 KiB window starting at 1),
  2 = zlib; chunks may be scrambled with `(b - i) ^ i`; checksum covers scrambled bytes.
* **Precedence** (verified by diffing overlapping files): extension rank
  `hpi < ufo < ccx < gpf < gp3`; within one extension the *alphabetically first* archive
  wins, as NTFS lists names (by their upper case: `_x.ufo` comes after `a.ufo`). So `rev31.gp3` (3.1 patch) > `btdata.ccx` > `ccdata.ccx` > `totala1.hpi`.
* 30 archives, ~9,900 files, ~1.9 GB unpacked; merged VFS has 5,198 unique paths.
* **A mod's archive of an install's archive's name replaces it.** Mods were installed by
  copying their archives into TA's folder, overwriting any of the same name, so a mod's archive
  named like one of the install's (in any case) takes its place: with the mod on, the install's
  is not mounted at all, and the mod's is mounted with the mod (over the install, as all of a
  mod's content is). Whatever it holds: an empty placeholder (no file in it, or no `HAPI` header
  at all) blanks the install's. A mod's `mod.toml` may name more it replaces (`replaces =
  ["btdata.ccx"]`). Without mods, or with mods shipping no such name (TA: Escalation's
  `TAESC.gp3` and `T*ESC.ufo`, Total Mayhem's `mayhem.gp3` and `TADemoM.ufo`), the install is
  mounted whole, as above.

## Mod packages (as ModDB has them)

* Most were made for the original game: TA archives beside a patched `TotalA.exe` (or
  `TALON.exe`, `TotalBT.exe`), DLLs (`tdraw.dll`, `ddraw.dll`...), `.ini` settings and
  readmes, at the top of the zip, in a folder of their own (`Talon v1.03/`, `TBT140/`) or
  among optional extras (Escalation: `Step_3_Install Main Files .../TAESC.gp3`, with a demo
  recorder, a hotfix and music elsewhere; Frenzy: `All map weapons/<x>/Map Weapon.ccx`).
* Archives are recognised by their `HAPI` header: Total Battletech's is `rev31.mwp`
  (installed as `rev31.gp3`, it replaces the 3.1 patch's as the others' `rev31.gp3` do). Some
  ship empty or unreadable `totala1.hpi`, `btdata.ccx`, `ccdata.ccx` to blank out the
  original's; several ship their whole game as `rev31.gp3`, copied over the patch's. TA:
  Enhanced Alpha 0.8v: `rev31.gp3` (4,591 files, the whole mod), `totala1.hpi` (46: fonts,
  palettes, script headers, `gamedata`), `ccdata.ccx` (3), and `btdata.ccx` of 148 bytes, a
  directory of one empty folder: "IT IS ABSOLUTELY REQUIRED TO OVERRIDE THE THREE FILES KNOWN AS
  TOTALA1.HPI, CCDATA.CCX AND BTDATA.CCX"; it also has the six post-release units' `.ufo`s
  removed. The installer names such placeholders beside the content in the mod's `replaces` and
  leaves the files out.
* The patched engines read folders of their own in place of TA's (verified by the units
  and weapons that load only with them):

  | Mod | Folders (case aside) | In place of |
  | --- | --- | --- |
  | Total Mayhem 11.3 | `guim`, `unitpicm`, `weaponm` | guis, unitpics, weapons |
  | TA: Escalation 9.4 | `unitse`, `unitpice`, `guie`, `weapone`, `gamedate`, `downloadse` | units, unitpics, guis, weapons, gamedata, download |
  | Talon 1.03 | `unittal`, `guital`, `weaptal`, `gametal`, `downloadtal` | units, guis, weapons, gamedata, download |
  | Total Battletech 1.40 | `unitsbt`, `weapsbt` | units, weapons |

  Their engines read only those, so the originals' are hidden beneath (Talon's `unitTAL`
  holds ARM and CORE as well as TAL).
* Their installers put them in TA's own folder, beside TA's archives: TA: Escalation Gold
  10.2.0 as `TAESC.gp3`, `T2ESC.ufo`, `T3ESC.ufo`, `T4ESC1.ufo` (sounds, models and animations
  only), `T4ESC2.ufo`, `T5ESC.ufo`, `TXESC.ufo` and `TADEMO.ufo`; Total Mayhem 11.3 as
  `mayhem.gp3` and `TADemoM.ufo`, with its own `TotalA.exe` over TA's. Read by TA's own engine,
  their folders of their own were never seen, but their shared folders were: TA: Escalation's
  `objects3d` and `scripts` hold models and scripts named after TA's own units (`ARMCOM`,
  `ARMSOLAR`, `CORAK`...), and Total Mayhem's archive holds TA's `units` folder (504 units).

## TDF family (`.tdf .fbi .gui .ota`)

`[Section] { key=value; }`, case-insensitive, `//` and `/* */` comments. Data contains
missing semicolons and stray braces — the parser is deliberately lenient. TA's files are
Windows-1252 (read as Latin-1). The map editor writes an OTA whose text Latin-1 can't hold
(a map named in Cyrillic or Chinese) as UTF-8 after its byte order mark, which the reader
recognises; TA's own files never begin with one.

## Coordinates and projection

* World: x east, y up, z south, **1 unit = 1 map pixel**. Height cells are 16 px, graphic
  tiles 32 px.
* TA's view is an oblique projection: ground maps 1:1 to the screen, height lifts things by
  `y/2`. Painted map art therefore shows ground point `(x, h, z)` at art pixel
  `(x, z - h/2)`. Rendering the heightmap as a 3D mesh sampling art at that coordinate
  reproduces TA exactly in the classic camera and gives true relief otherwise.
* **3DO models are left-handed**: the muzzle of `armstump` is at −z while the
  Commander's right arm is at −x. Negate z on load; units then face +z (south) at heading 0.
* **Script values in that frame: moves along x and turns about z are negated**; moves along
  y and z and turns about x and y apply as they are (`unit::piece_world_position`, the
  renderer's piece matrices). Each was checked against its flip over every script of the
  install (`cargo test --release -p ta_sim --test animation_scan script_conventions --
  --ignored --nocapture`). Of the poses the scripts hold (open, closed, deployed; walk
  cycles, aims and recoils aside), read with one axis the other way round, ours leaves fewer
  pieces through each other, sunk below the model or strayed far out in 78 poses against 5
  for x moves, 153 against 41 for y moves, 38 against 15 for z moves, 202 against 33 for x
  turns, 69 against 8 for y turns and 126 against 21 for z turns. Recoils go back: 90 of the
  92 z moves of guns in fire functions (`move barrel to z-axis [-2.4]`), and all 7 y moves of
  guns that hang down. Walkers' feet on the ground slide back as they walk in 38 of 41 walk
  cycles (in 17 with x turns flipped). And playing every unit through its animations
  (`animation_scan`) finds nothing but TA's own quirks our way, and 7 to 102 defects with
  any one axis flipped.
  * x moves: the Arm Advanced Vehicle Plant's roof is two halves meeting in the middle
    (cylinder1 spans x −25..1, cylinder2 1..27); its `activatescr` moves cylinder1 by +27
    and cylinder2 by −26: read as written they pass through each other and the plant never
    opens, negated they slide out to the base's edges. 61 x moves carry a piece clearly off
    to one side through the model's middle unless negated, one (a Gantry's clamp light,
    riding its clamp) reads better as written.
  * z turns: the solar collector's `Go` turns dish1/dish2 about x (−90°, +90°) and
    dish3/dish4 about z (−90°, +90°), and all four open outwards.
  * y turns are not mirrored with the x moves (a frame mirrored in x would flip both, and z
    turns back: that mirror image of ours loses in 228 poses against 30). The Core Advanced
    Construction Kbot turns its nanolathe cases −90°/+90° about y, then slides the guns out
    along x: only so do they come out and point forwards (flipped, they spray backwards
    through the torso); the Core Vehicle Plant swings its roof plates ±90° round the pad's
    middle and slides them along x: flipped, the front and back plates stack over the pad.
  * Turrets aim with the same sense: the engine's headings (`AimPrimary`, `StartBuilding`,
    `XZ_ATAN`) count +16384 towards +x, as `turn ... to y-axis` turns. The scan checks 289
    turret aims: each points within 30° of where its script was told; with y turns flipped,
    91 aim at the mirror image of their target.
  * Spring, which ran TA's scripts on 3DOs with z negated, negates the same two: x moves and
    z turns (and spins about z).
* **Turns go the short way round** (as Spring's do): five fighters' barrel roll (`MoveRate2`)
  turns the hull about z to 240°, 120° and 0° in turn, a full roll only the short way (120°
  at a time, the same way round); read as numbers it rolls 240° and back. So the Core
  Construction Hovercraft's and Advanced Construction Sub's hatch doors, keyframed to −211°,
  swing down through the hull past the stowed nanolathe (the direct way they would flip up
  and over it).
* Piece transform: `T(offset + move) · Ry · Rx · Rz`; unit: `T(pos) · Ry(heading) ·
  Rx(pitch) · Rz(roll)`. Heading 0 faces +z; +16384 faces +x. A move is in the parent's frame,
  so a gun its aim pitches itself kicks back level when it fires, not along itself: the Core
  Raider's barrel (`base > turret > sleeve > barrel`, `AimPrimary` turning the barrel about x,
  `FirePrimary` moving it `[-2.4]` along z, the sleeve never turned), pitched up to lob a shell,
  slides back through the top of its sleeve. Most guns hang from a pitched sleeve and kick back
  along it. The scene draws the kicks of guns that point themselves only as far as they run
  clear (docs/ARCHITECTURE.md); `recoil_scan` in `crates/ta_sim/tests/animation_scan.rs` lists
  how every unit's guns are rigged and what their recoils pass through.
* **Ground plates:** a piece's `SelectionPrimitive` is the plate under a unit that TA's editor
  laid for its selection, a flat untextured quad, never drawn. Of TA's 288 wreck and heap models
  (`*_dead`, `NxNa`...), 21 do not flag theirs: a black (colour 0) quad facing down at the foot of
  the root, the size of the footprint (32, 48, 64, 80 or 128 square), among the rest of its faces
  (`armss_dead`'s and `corss_dead`'s in colour 46, `corpyro_dead`'s in 95, `cordev1_dead`'s 1.25
  under the origin); two more (`armah_dead`, `corckfus_dead`) carry a second copy of their flagged
  one. Their debris mostly reaches lower (to −11.5 in `armemp_dead`). TA Reborn leaves these out
  (`model::flag_ground_plates`, `wreck_ground_plates_are_left_out` lists them).

## Maps (TNT v0x2000 + OTA)

* Heights are per 16 px cell, in world units; sea level from the header. Land is often
  barely above water (Coast To Coast: land 86, sea 85).
* **Layout** (all 275 maps): a 64-byte header of 16 i32 — version `0x2000`, width, height
  (cells, even), tile map ptr (always 64), attribute ptr, tile graphics ptr, tile count,
  feature count, feature names ptr, sea level, minimap ptr, then `1` and four zeros. The
  sections follow in that order without gaps, except that the attributes start on a
  16-byte boundary (126 maps pad 2-14 bytes after the tile map). Attributes are 4 bytes per
  cell: height, feature slot (u16), and a byte that is 0 on every map. Feature name
  records are an i32 index and a 128-byte name; TA's editor left stray bytes after the NUL
  (and in the padding), otherwise `TntMap::write` reproduces the files byte for byte.
* Feature slots: index into the name table, `0xFFFF` none, `0xFFFC` void (impassable to all
  but aircraft; on a world in space the perspective view draws nothing there and the sky shows
  through, see `TerrainGpu::set_space`). A feature's index
  sits in its footprint's top-left cell. `0xFFFE` marks cells covered by a multi-cell
  feature on only some maps (2,580 of 43,468 multi-cell features; campaign maps, Coast To
  Coast, Metal Heck) — the rest leave those cells alone and footprints overlap freely — and
  Pincushion and Cloaked in the Spires have `0xFFFE` cells under no footprint at all. The
  simulation reads the origin cells only; the map editor writes no `0xFFFE` marks and keeps
  those it finds. It places a feature only where its footprint covers no other feature's and
  no void cell (`MapDoc::taken`), and never changes void cells.
* Map sizes: 207 of 275 maps exceed 4096 px (largest 20480 px, up to 11,561 unique
  tiles) — terrain uses a deduplicated tile atlas with exact cross-tile filtering. The
  reader takes maps up to 16384 cells a side and 2^28 cells in all (`tnt::MAX_SIDE`,
  `MAX_CELLS`; the writer too, as long as the file's i32 offsets reach), checks that the
  header's tables lie within the file before making room for them, and keeps the first
  65,536 tiles (all a u16 tile index reaches).
* Start positions: `GlobalHeader/Schema N/specials/specialN { specialwhat=StartPosK; XPos; ZPos }`
  (in any order; Coast To Coast lists StartPos3 first).
* **TA Reborn's marks per cell** (`TntMap::flags`, `CELL_*` bits): in the attribute's spare 4th
  byte, read only when the header's last word is `FLAGS_MAGIC` (`"RBCF"`, where TA writes 0),
  so no other tool's stray bytes are ever read as marks; a map without marks is written as TA
  writes it. `CELL_CLIFF` (1): the simulation takes the cell as steep as a slope can be (slope
  255), so nothing with a slope limit crosses it or builds on it. Maps converted from Supreme
  Commander carry the game's own cliffs this way, whatever their heights became (docs/SUPCOM_MAPS.md);
  the map editor keeps them.
* **A map's own features** (`maps/<stem>.features`): a TDF of feature definitions, loaded after
  the game's own (`features/`) and never in place of one of them; `rebornworld` names the
  world a definition is of. Under `maps/`, they are no part of the data multiplayer peers
  compare, and the map's digest and its battles' cover them (`Compatibility::map_digest`,
  `for_match`; a map without the file keeps the digest it always had). A battle's data holds the
  game's features and its own map's alone, in that order (`GameData::for_map`): no other map's
  shifts their numbers, which the simulation goes by. The game's data for menus and the editor
  holds every map's. The map editor saves a copy's beside it. Converted maps name their trees,
  bushes and rocks there (`SC1_Green_T1`).
* **OTA keys TA Reborn reads besides TA's**: `rebornart=1` (the art was painted by TA Reborn's
  own tools: the renderer cleans no TA painter's hatching from it); a converted map's
  `supcommap` (its Supreme Commander folder, which also makes its origin Supreme Commander's in
  the menus) and `supcomversion` (the converter's version).
* **Where a map comes from** (`ta_data::map::Origin`, shown in the menus): the layer and file its
  TNT is read from. Of the install's (layer `base`): `cc*.ccx` Core Contingency's, `bt*.ccx` and
  `tactics*.hpi` Battle Tactics', `totala*.hpi`, `worlds.hpi`, the 3.1 patch's `rev31.gp3` and
  Cavedog's eleven `.ufo` downloads Total Annihilation's; any other archive in the game's folder (a
  map pack a player put there) and loose files a map pack's; TA Reborn's layer its own; any other
  layer a mod's or a map pack's.

* **Minimap**: declared 252×252 even for maps that are not square (some campaign maps
  declare, and store, 252×256). It shows the art of all but the last 2 columns and the last
  8 rows of cells, from the top-left corner, scaled to 252 pixels along the longer side, the
  other side rounded down: 252 × ⌊252·(h−8)/(w−2)⌋ for wide maps (Coast To Coast, 210×126
  cells, fills 252×142). All but one of the 188 maps outside the numbered campaign ones
  whose picture leaves filler match exactly (Lava Run's picture ends in filler-coloured
  rows); the campaign maps are less regular. The rest of the
  canvas is a flat filler colour, index 100 on most maps and 0 on some campaign maps. The
  parser strips trailing rows/columns of the filler (the canvas's bottom-right colour);
  `TntMap::minimap_area_px` is the ground the picture shows, which the minimap and the map
  previews place unit dots and start positions by.
* **The strip TA never showed** (the last 2 columns and 8 rows of cells, one tile column and
  four tile rows): on 158 of the 275 maps some of its tiles are nothing like the ground
  beside them (Painted Desert's last column is tile 0, flat blue, from its 32nd row down;
  Shore To Shore's last three rows are a flat grey across its sea, 977 tiles of it). The terrain replaces
  a tile there by the shown ground mirrored at the edge, tile by tile
  (`terrain::shown_tiles`), when its mean colour is more than 0.18 (linear RGB) from the shown
  tile beside it, or looks like none of the shown ground within two tiles of it in the last
  two shown rows or columns (more than 0.30 away as the screen shows the colours, sRGB-encoded:
  Shore To Shore's grey lies 0.1800 from its deep sea in linear light, a hair either side of
  the first test, so by it a tile in eight was left, grey rectangles in the water along the
  map's foot; 0.35 as the screen shows it). Across the install the second test clears 262 more
  tiles in 10 maps, 247 of them Shore To Shore's (`cargo test --release -p ta_render --lib
  the_unseen_strips -- --ignored --nocapture` surveys them, and `TA_STRIP_MAPS=<names>
  TA_STRIP_OUT=<folder>` writes pictures of the strips as stored and as shown). The map editor
  shows the strip as the game does (also after an edit at its edge:
  `TerrainGpu::update_tiles`) and saves the file's own.
* **Writing** (`TntMap::write`, `build_minimap`): the layout above, with the minimap
  declared 252×252 on filler 100 (0 when the picture's own last row or column is all 100, so
  it crops back the same). `build_minimap` box-filters the art of that window to the nearest
  palette colours, never 0 or 100. Every map of the install reads back identical after
  writing (`crates/ta_formats/tests/tnt_write.rs`).
* **OTA**: TDF with TA's own layout, which `TdfDocument::to_text` writes: tab-indented
  `[Section]`, the braces on their own lines one tab further in, `key=value;`, CRLF. Every OTA
  of the install reads back identical. Skirmish maps carry `missionname`,
  `missiondescription`, `planet`, `tidalstrength`, `solarstrength`, `minwindspeed`,
  `maxwindspeed`, `gravity`, `numplayers` ("2, 3, 4"), `size` ("7 x 4": 512 px units,
  rounded), `SCHEMACOUNT` and per schema `SurfaceMetal`, `MohoMetal`, starting resources,
  meteor settings and the start positions. There is no extractor radius or metal maximum:
  extraction is by footprint over the cells' metal (deposits, else `SurfaceMetal`).

## Missions (`camps/*.tdf` and mission OTAs)

* 175 missions in 13 campaigns (`camps/*.tdf` list their OTAs; `example.tdf` is the
  engine's). A mission OTA is a map OTA with briefing keys, goals and three schemas (Easy,
  Medium, Hard) of placed units: `Unitname`, `Ident`, `XPos`/`ZPos`, `Player` (1 is the
  player), `HealthPercentage`, `Angle`, `Kills`, `InitialMission` (a script; about 24,800
  units have one). `Immunity=1` (19 units in 5 missions) is a unit nothing harms: the Gate to
  capture in AC11, the player's own Gates, Core mission 1's Commander, "deactivated" (`o 0 0,
  w 3600`) while the player's strike force clears the Arm out, Titanium Fist Bay's scouts.
* A schema's placed units are the subsections of its `[units]`. TA: Escalation's copies of the
  missions (all 175 of its mission OTAs, in `TAESC.gp3`) keep them in `[unitsE]` alone, as its
  patched engine read `unitse` for TA's `units` folder; TA Reborn reads a schema without
  `[units]` from the section whose entries name units (`Unitname`). Total Mayhem's own
  campaign (`camps/face to face.tdf`) lists three maps made for skirmishes (a `Network 1` schema
  of start positions, no units) as missions, their goals added.
* TA's map and mission editor, TAE (installed with the game: `TAEread.txt`, `TAE.hlp`), says
  what the rest of a unit's keys mean; it writes them all for every unit (Two Continents, a
  multiplayer map, keeps a test schema with each set: `OffMapUnit`, `Immunity`,
  `CreationCountdown`, `InitialGroup`, `InitialMission`, `AiIgnore`, `AiPriorityTarget`,
  `MissionCriticalUnit`, `BuildPriority`). "The AI" there is the computer players (every
  player but 1), and the missions use the keys as it says:
  * `AiIgnore=1` (20 units in 7 missions), "AI Should Ignore Unit ... the AI will act as
    though the unit does not exist (even if being attacked by it!)": computer players' units
    never pick it as a target, and their players never plan against it; the player's units
    still shoot it. The player's immune Gates (AC06, CC05), CC01's deactivated Commander, one
    of Titanium Fist Bay's immune scouts, two of Can Rush's three Construction Kbots ("use
    them wisely"), Mesa Run's doomed Valkyrie (Easy); AC11's Gate to capture (Easy) is the
    computer player's own, where it changes nothing.
  * `AiPriorityTarget=1` (Miniement Landing's Shipyard and a Guardian: "your Shipyard is a
    high priority target"), "will make free units looking for something to attack head for
    that unit first": computer players' units shoot it before anything else in reach, and
    computer players send their waves and air strikes at it first.
  * `MissionCriticalUnit=1`, "if a unit designated a mission critical unit is destroyed the
    mission is lost": the player's (destroyed, or taken). The one TA's missions mark is Sink
    the Battleship's battleship, the enemy's, which the player has to sink (`KillAllOfType=
    ARMBATS`): a computer player's has no part in the goals.
  * `InitialGroup=name` "works much like setting a unique name, except that it allows a group
    of units to be referenced rather than just one": scripts name groups as they name units
    (Skirmish on the Sand's reserve waits `wa patrol` on three Peewees, one also called
    `patrol`). The player's are numbered (Dogfight over Grantham Deep's Vamps 1 and Finks 2,
    Sink the Battleship's Titans 1: "keep your Titans concentrated"): that control group at
    the start. (A computer player's number, AC05's Thuds' 3, is only a name.)
  * `BuildPriority=TYPE`, "Tell AI to Build this Unit with Priority means of all the units
    specified this particular unit should be built first": the owner's factories turn it
    out where they would a combat unit (Attack on Barrack's Dominators, `CORHRK`: "the Core
    are massing a huge base assault force of Dominators").
  * `OffMapUnit` and `CreationCountdown` (seconds) are set by no mission, only by that test
    schema (Commanders standing on the map); not acted on. The help also names a flag "Enemy
    Does Not Target ... (though it may still be targeted by your units)", which is one of
    `OffMapUnit` and `Immunity`.
* `maxunits` (125 missions, 250; Exp1AC12 300, Exp1CC08 200, Krogoth Encounter 400; the
  original fifty have none, and no limit) is every player's unit limit ("these options work
  just like the options in multiplayer play"). Placed units count: Exp1CC08's Hard schema
  places 211 Core units, which build nothing until losses bring them under 200.
* File keys are stems (`brief`, `narration`, `glamour`, `glamoursound`, `useonlyunits`); the
  editor's file picker left the extension on some (105 missions' `brief=T17Brief.txt`,
  `glamour=tabtarm.pcx`), which is cut off. `glamoursound` (3 missions, `exp1armvict`) is
  the sound with the glamour picture when the mission is won. Not read: `nosealeveltrigger`
  (11 missions; by its name the editor's "Water Won't Splash", for seas that aren't water),
  `nomovie` (12), `missionhint` (31 name hint files the install doesn't have),
  `killmul`/`timemul` (the victory screen's score), `aiprofile` (per schema,
  `ai/<profile>.txt`: the computer player's limits and weights).
* **Times are seconds of game time**, the goals' and the scripts' alike.
  `DeathTimerRunsOut=1202` is Fortitude Race's "build a Fortitude in 20 min",
  `VictoryTimerRunsOut=3600` Firebase Prasad's "1 hour". Meraldi Amphibious Assault's
  briefing gives the Tritons "orders to wait five minutes before moving": `w 300`; Cutoff in
  Sector 6895's rescue Atlas is "ten minutes away": `w 577`; Core mission 21's Gate blows
  itself up at `w 7195`, its death timer is 7200; Brawler Buildup's raids leave between
  `w 900` and `w 1200` of its 1202 seconds.
* **Scripts**: commands separated by commas, each a word (case aside) then its arguments, run
  one after another from the start of the mission (a command starts when the one before is
  done):
  * `m x z` move; `p x z` patrol, consecutive points making one route (TA Reborn loops it back
    to where the unit set out, as a player's patrol does). One command may hold several
    points (`p 2600 5950 2760 5750 2930 5930`, 290 times; `m` 3 times).
  * `w n` wait n seconds; `wa [name]` wait until this unit (or the one named) is attacked;
    `a TYPE` attack the nearest enemy unit of a type ("or a unique identifier (for example
    'a FRED')", TAE); `s` stop.
  * `o move fire` standing orders: the move order (0 hold position, 1 maneuver, 2 roam), then
    the fire order (0 hold fire, 1 return fire, 2 fire at will). TAE's help leaves `o` out;
    the missions show the order. Of its 1,029 uses (placed units of every schema), 906 are
    `o 1 2`, the units' own defaults (`StandingMoveOrder=1`, `StandingFireOrder=2`), the rest
    `o 0 0` (51), `o 0 1` (46), `o 0 2` (21) and `o 1 0` (5): the first number is never 2;
    the guns' scripts open fire with the second: AC10's Punishers `o 0 0, w 1200, o 0 2`, the
    Motien Ramp's Intimidator `o 0 1, w 3000, o 0 2, a ARMBRTHA` (holding position means
    nothing to a building).
  * The `NoAutoFire` guns a mission places for a computer player (the Big Berthas,
    Intimidators, Doomsday Machines, Annihilators: on hold fire, `sim::initial_fire_state`)
    are its to set going, as it does its own (`SkirmishAi::arm_defenses`): at once where no
    script runs them
    (AC24's Doomsday Machines), else when the script is over, "if a Unit is given no initial
    commands, or runs out of commands, the AI will take over" (TAE), meanwhile as its `o` says.
    So CC15's Berthas (`wa`), to be captured, hold their fire until attacked, as TAE has `wa`
    used to "hold fire until it is attacked"; AC23's Doomsday Machines (`w 300`) open fire after
    five minutes, CC22's Berthas (`w 120, a CORCOM`) on the Commander after two. The player's
    own were never built, so TA's hold fire on construction does not apply: they keep their
    file's `StandingFireOrder` (`sim::file_fire_state`, set in `setup_mission`): I09's and
    T31's Annihilators fire at will (2), EXP1AC11's and T38's Berthas and Air Assault's
    Intimidator wait for the player (0). No other mission places a `NoAutoFire` gun for the
    player.
  * `g name` guard, until the unit is gone: escorts `g ts1, g ts2, m 200 200, d` move on
    from one transport to the next, then leave.
  * A name is a unit's `Ident`, else a group's (`InitialGroup`), else (`a`, `g`, `i`) a unit
    type: the nearest when the command comes, for `g` of the unit's own side first (Miniement
    Landing's escorts `g CORTSHIP, m 111 111, d` guard the transports, then leave; Can Rush's
    jammer `g larry, g moe, g curly, g armfus`, its Cans gone, follows the player's Fusion
    Reactor). `wa` waits on the unit called so and its group alike. 48 kinds of name stand
    for nothing (an ident of another schema, `wa 600` for a wait); `wa` then waits on the
    unit itself, the rest are left out (`ta-tools missions` lists them).
  * `b TYPE [n]` a factory builds n (one without a number) and then goes on (11,350 times in
    91 missions; its `m`, `p` and `g` set the way its units leave). `b TYPE x z` a
    constructor builds there, also written `b TYPE 1 x z`; `b TYPE` alone, where it stands.
    `bw n`: a Core anti-nuke builds n missiles.
  * `i name` starts aboard that transport ("i ARMSHIP", or an ident "if there is more than
    one transport", TAE; Mesa Run's Roach `i CORVALK`). `u [x z]` sets one unit down there
    (or where the transport is): a transport carrying n units is given n `u`s with a point
    each (Miniement Landing's ships 8, the hovercraft of the Core Contingency's last Arm
    mission 6, each on its own spot), Valkyries `m x z, u`. A passenger's opening waits count
    from the start of the mission, aboard or not: AC14's Roach (`i TRANSPORT4, w 2670, d`)
    rides a Valkyrie that sets off at `w 2640`; AC13's Reapers wait 45 seconds longer than
    their transports.
  * `d` self-destructs, at once (561 times in 22 missions): crawling bombs `m x z, d` walk to
    their mark and go off, `wa HLT1, d` lie in wait by a tower, a shipyard `wa target1, w 20,
    d` goes up after its guard is attacked; transports and escorts finish `m <a corner>, d`.
  * A name is the rest of the command: names hold spaces (`g Power Mike`, `wa solar 2`).
    Attack commands also preserve the complete name and resolve idents/groups before types.
  * TA's own slips (328 pieces in 33 missions; `ta-tools missions` lists them) are left out:
    missing commas (`w 300 b CORFAV` is the wait alone), missing letters (`0 1 2`,
    `1266 5038`), a list of types after one `a`, a count before the type (`b 2 corstorm`).
    So are builds of a type the builder can't make (23 kinds, mostly misspelt, `b CORSH4`,
    `b corcan1`; a few at the wrong plant, `b CORAK` at a vehicle plant).
* **Goals** (`GlobalHeader`): "all the victory conditions must be met for a victory to
  occur, but only one defeat condition met will result in defeat" (TAE; see
  docs/ARCHITECTURE.md, Campaigns). The briefings say so: Attack on Barrack's "destroy the
  Advanced Kbot Lab and all the Dominators" (`KillUnitType=CORALAB, 1`, `KillAllOfType=
  CORHRK`), Core mission 2's "Build a Vehicle Plant and destroy all of the Arm units", Brawler
  Buildup's "Destroy all enemy Brawlers while ensuring your metal production continues for at
  least twenty minutes" (with `VictoryTimerRunsOut=1202`). `KillEnemyCommander` against an
  enemy without a Commander is met: the Core Contingency's last mission (Exp1CC12) sets it,
  with `DestroyAllUnits` and an hour's `VictoryTimerRunsOut`, against an Arm that has only
  Decoy Commanders (no `Commander=1`), and its briefing asks that the implosion device last
  the hour. `MoveUnitToRadius=TYPE, x, z, r`
  naming a building is one raised there, finished: Guardian Hill's "build a Punisher on top
  of Guardian Hill", Waiting's Metal Extractor, River Base Alpha's Vehicle Plant (7 missions);
  `ANYTYPE` counts the units that went there (AC01's Gate stands on its spot).
  `UnitTypePassesX=TYPE, n` and `UnitTypePassesZ` win when a unit of the type gets across
  the line x (or z) = n; `AnyUnitPassesX=n` and `AnyUnitPassesZ` lose when an enemy unit
  does. The line is crossed away from where the units start, the player's of the type (else
  all the player's) or the enemy's: Force the Strait's Hulk from x 80 east past 6000, Cutoff
  in Sector 6895's Commander from z 896 south past 2700, Core mission 10's transport north
  past 408; Lightning Recon is lost to an enemy unit west of 4500 (at the player's base, the
  enemy's being at x 7000 to 8600), Surprise at Labrodorite to one west of 25 ("do not let
  any Arm units escape to the west": its Commander leaves for x 0 at `w 1050`).

## GAF

Entries of frames; frames may be composites of sub-frames aligned on their origins.
Compressed rows: bit0 = skip n>>1 transparent, bit1 = repeat next byte (n>>2)+1, else
copy (n>>2)+1. Unit textures with 10 frames are **team colours** (frame = player colour).
Sprite origin semantics: trees anchor at the trunk base (`leaf2` 47×69, origin 25,68);
shadows are separate `…shad` entries drawn translucent. TA's feature art is lit from the south
west (a tree's shadow falls to the north east, up and right on the screen). `Gaf::write` writes
the layout back (every frame compressed, a repeated frame stored once), for TA Reborn's own
feature art (`ta-tools gaf-pack`).

Explosions are drawn a pixel to a world unit. TA's are small (the largest `fx/alfboom2`,
126×53; its nuke `fx/Nuke1` 76×59); a mod's may be painted as wide as their blasts: every TA:
Escalation explosion's frame side is about its weapon's area of effect (`esc_weap_x_01`, the
B.F.G.'s, 48 raw frames up to 3240×3240 for an `areaofeffect` of 2880; 238 MB). Its frames are
raw (a byte a pixel), the first ten or so composites of two layers (a shock ring, a fireball),
the rest a fireball's canvas; its 86 animations named by weapons come to 1.33 billion pixels.
Escalation also replaces `fx.gaf`'s Explode2-Explode4 and Nuke1 with shield bubbles 760 to
1140 px across, which no weapon of its names.

## Model surfaces (`objects3d/<name>.surface`, TA Reborn's own)

What TA Reborn paints a 3DO with beyond TA's palette colours and GAF textures (`ta_formats::surface`;
the models it makes of Supreme Commander's units have them, docs/SUPCOM_UNITS.md). Beside its
model, of the same name. Little-endian:

| Bytes | What |
| --- | --- |
| 0 | `TASURF`, two zero bytes; the version (1); the number of materials; of pieces (u32 each) |
| 20 | the far model's name: a u16 length and that many bytes of UTF-8 (none: 0) |
| materials | each: its name, its colour map's path, its specular map's, its normal map's (each a u16 length and UTF-8), its colour on average (sRGB, 3 bytes) and a byte unused |
| pieces | in the 3DO's order (depth first, as `Model3do::parse` lists them): the piece's number of vertices (u32, its own), then for each vertex its texture coordinates (two f32: u from the picture's left, v from its top) and normal (three i16 of 32,767ths, in the 3DO's axes as its vertices are, and one unused): 16 bytes |

* A primitive whose texture name is one of the surface's materials (any case) is painted with it;
  any other is drawn as TA draws it. A surface whose pieces or vertices do not match its model's is
  not used (the log and the mod check say so).
* The textures' channels: the colour map's RGB (sRGB; alpha unused, so BC1 serves); the specular
  map's red (how much it mirrors its surroundings), green (how glossy), blue (where it glows by
  itself) and alpha (where it takes the player's colour); the normal map's tangent-space normal, x
  in alpha and y in green (z made up), x along u and y along v down the picture, as Direct3D's are.
  Each is drawn at its own size, 512 or (with HD surfaces) 1024 texels a side (see DDS textures
  below): a material's three may differ.
* How the channels are lit goes by the art style (docs/SUPCOM_UNITS.md, "Light"): Enhanced Classic
  as Forged Alliance lights them (the green a broad, cool sheen); Reborn as a physically based
  finish (the green, over some six texels round each, how glossy its paint is; where it is glossier
  than round about, an edge worn to bare metal; strong red polished metal).
* `far`: a lighter model (a 3DO with its own surface, of the same pieces in the same order) drawn
  in this one's place when it is under 48 pixels across on screen; a model drawn as itself the
  frame before stays so down to 42 (no swapping back and forth as the camera shakes).
* A surface that does not read, or does not fit its model, leaves the model drawn as TA draws its
  faces, without Reborn's kit.

## Model animations (`objects3d/<name>.anims`, TA Reborn's own)

Keyframed movements of a 3DO's pieces that TA Reborn plays on the screen alone (`ta_formats::anims`;
the models it makes of Supreme Commander's units have them, docs/SUPCOM_UNITS.md). Beside its
model, of the same name. The simulation never sees them, as it never sees how its walkers' legs are
posed by the ground they cover. Little-endian:

| Bytes | What |
| --- | --- |
| 0 | `TAANIMS`, a zero byte; the version (1); the number of clips (u32 each) |
| clips | each: its name (a u16 length and UTF-8), its length in seconds (f32), the number of pieces it moves and of its frames (u32 each); the pieces, by their index in the 3DO's order (u32 each); its frames' times, seconds from its start, rising (f32 each); then each frame's keys, piece by piece in that order, each a move and a turn (six i32: 24 bytes) |

* A key holds its piece as a TA script holds one: moved from where it stands in the model along x, y
  and z in 65,536ths of a pixel (x negated as scripts give it), and turned about x, y and z in TA's
  angle units (65,536 a turn; z negated), applied about y, then x, then z, as the engine composes a
  script's (see "Turrets aim with the same sense" above). Between keys a piece moves in a straight
  line and turns the short way round.
* `walk`: a cycle, its last frame leading back into its first over the time left to its length.
  A ground unit whose model has one walks by it, in step with the ground it covers, instead of by its
  script (apps/ta_game's `scene/walk.rs`): its stride is measured from its planted feet as a
  script's is, and the channels its script aims with (a turret, a torso) stay the script's.
* `death`, `death2`...: one of them, at random, plays where a unit of the model dies on the ground
  (an aircraft out of the sky and a ship going under go as TA Reborn's other units do); its wreck
  shows once the clip is over, lying in the pose it ends in.
* `active`: a cycle (as `walk` is) played round and round while the unit works: finished, switched
  on (or with no on/off switch), its energy upkeep paid, neither stunned nor carried, not
  upgrading or installing an enhancement, and, for a factory or any unit with `Builder=1`, building
  something. As the unit stops working the clip slows to a stop where it is (in 0.6 s) and goes on
  from there when it starts again; as it starts an upgrade the clip fades out (in 0.4 s). A unit
  first seen at work is somewhere in its cycle by its unit number, not at its start.
* `open`: played forward as the unit starts working (as `active` has it) and held at its end while
  it works; played back to its start as it stops. A building with no switch opens once, as it is
  finished.
* `upgrade`: while the unit upgrades in place (`UpgradesTo`), posed at its share of the upgrade
  done (its first frame at none, its last at all of it), faded in as the upgrade starts and out if
  it is called off. Once done, the unit it upgrades into stands in its place as its own model rests.
* `active`, `open` and `upgrade` are added to what the unit's script does with the same pieces (a
  piece its script spins spins on as a clip lifts it), each turn taken as running on from the frame
  before (30,000 then -30,000 is 35,536): fading, a clip never turns a piece half a turn back.
  Nothing else plays them: a TA script's own moves are untouched, and the simulation never sees
  them.
* A clip that moves a piece the model has not, more than once, or with its times out of order or
  past its length, is not played (the log and the mod check say so); the file's other clips are.
  A file that does not read is told in the log.

## DDS textures

`ta_formats::dds` reads DirectDraw Surfaces: DXT1, DXT3 and DXT5 (BC1, BC2, BC3) and uncompressed
colour of 8 to 32 bits described by masks, a mip level at a time, or its blocks as stored. The paint
of models with surfaces is drawn from arrays of BC3 layers with every mip level, of 512 × 512
texels and, for textures of 1024 or more a side with HD surfaces, of 1024 × 1024 (`paint_side`:
the largest power of two a texture's larger side reaches, from 512 to the most the arrays take;
`to_bc3_levels`): a DXT5 with a level of that size and every level below it is taken as it is;
DXT1's and DXT3's are made BC3 block by block (DXT1's four-colour blocks keep their colours under
an opaque alpha; its three-colour ones, which BC3 cannot read since it always reads four, are
given the same three colours where four can hold them, one end kept and the other moved half as far
again past it so that a third of the way along is the halfway colour, to a 565's rounding, and are
encoded again where that comes no nearer or a texel takes the see-through black; DXT3's colours
stay, its alpha is encoded); anything else is decoded, halved or grown to that side and encoded
level by level. Growing doubles a texture each way at a time where it can (`enlarge`: each new
texel from a Catmull-Rom spline through the 4 × 4 texels round it, its weights at a quarter of a
texel whole 128ths, kept within the 2 × 2 texels nearest: edges stay sharp, nothing rings), and
samples between texels for what is left. The encoders (an alpha block's ends its least and
greatest, a colour block's ends along the colours' main axis, refined once by least squares) use
plain arithmetic in a fixed order: the same bytes everywhere.
A file claiming more mip levels than its size halves into is read with those its size has (a
300 × 300 claiming four billion has nine), and only those its file holds; uncompressed channels
take up to all 32 bits of a texel.

## COB bytecode (verified against shipped scripts)

* `turn`/`move`: pop **target** (top) then speed; `spin`: pop **speed** (top) then
  acceleration (radars push 0 then their speed; read the other way round no dish ever
  turned, and extractors crawled at their acceleration). Accelerations and decelerations
  are per frame (30 a second), speeds per second: an extractor's arm (accelerate `<0.5>`)
  reaches full speed in seconds. Spins are kept axis by axis (`stop-spin` names its axis); a
  `spin ... speed <0>` (the constant, as written) about an axis a piece is not turning about,
  while it spins about another, stops that spin: the Seer's and the Informer's `Deactivate` stop
  their dish (spinning about y) with `spin dish around z-axis speed <0>`, which would otherwise
  change nothing (the only such spin in the install:
  `only_the_radar_vehicles_stop_a_spin_about_another_axis`).
* `get` pops four arguments then the value id (`get PIECE_XZ(p)` = push 7, p, 0, 0, 0).
* Callers push arguments; the callee's leading `create-local`s claim them. Query
  functions (`QueryPrimary(piece)`) return through their parameters.
* Linear units: 16.16 model units (BOS `[1]` = 163840 = 2.5 px). Angles: 65536 = 360°.
  Speeds are per second.
* `Killed(severity, corpsetype)`: severity = killing blow as % of max health; scripts pick
  corpse 1 (wreck), 2 (heap) or 3 (none).
* **`dont-cache <piece>` lets a piece's animated textures play, `cache <piece>` holds them on
  their first frame**, where every piece starts. TA's scripts `dont-cache` the pieces whose
  flashing lamps and glows should play and `cache` them again to stop them: the Arm metal
  maker's `Go` plays its sphere's glow (`glow`, 8 frames) and its `Stop` holds it, as the Core
  metal maker's and the Arm floating maker's do theirs; the moho makers and the Core floating
  maker close lids, flaps or doors over a glow that plays on; the Core labs' glow and plants'
  pads play only in `Go`, while they build; the factories' blinking lights play from `Create`.
  Pieces no script `dont-cache`s hold theirs (the Commanders' torsos, the targeting
  facilities' lights, the Core Vehicle Plant's base). (TA3D reads `dont-cache` as "animated
  texture" too.) `shade`/`dont-shade` (a piece drawn lit or unlit) are passed over: Reborn's
  lights shine by their textures. `cargo test --release -p ta_sim --lib survey_animated_textures
  -- --ignored --nocapture` lists every unit's animated pieces and the functions that play and
  hold them; `survey_on_off_units` what every on/off unit's script does switched off and on.
* What the engine tells scripts (as TA's, in the units the shipped scripts expect; checked
  by `crates/ta_sim/tests/animation.rs`):
  * `SetSpeed(s)`: extractors on completion, metal per second × 500 (the arm spins at
    `s × 45` a second); wind generators with every wind change, wind × 3000 (the blades'
    speed in angle units a second). `SetDirection(heading)`: wind generators face it.
  * `SetMaxReloadTime(ms)`: the longest weapon reload, once after `Create` (turrets rest
    after twice that).
  * `MoveRate0`..`MoveRate3`: aircraft, when speed ÷ top speed × 3 changes band (planes
    stop at 2; taking off or landing is 1, landed 0). The Freedom Fighter rolls at 2.
  * `RockUnit(x, z)`: at each shot, 500 × the direction away from the target in the unit's
    frame (x from its z component, z from its x): tanks rock back.
  * `HitByWeapon(x, z)`: at each hit, the same: 500 × the direction the hit pushes the unit,
    in its frame. 37 of the 45 scripts that have it turn their hull by that (about 3°, at
    19114 a second) and back at 5461 a second, as `RockUnit`'s do. Read as angles of up to
    45°, hits tipped tanks and radar vehicles half over and up to 17 px into the ground.
  * `SetSFXoccupy(state)`: what the unit moves over, when it changes: 4 land, 1 shallow
    water (up to 8 px deep), 2 deep water, 0 carried. Amphibians change to swim at 2.
  * `TargetCleared(weapon)`: a weapon (1 to 8) lost its target.
  * `QueryNanoPiece(piece)`: while building, the piece the spray leaves from (factories
    alternate between two nozzles on each call).
  * `TargetHeading` (constructors) is only stored for a later `StartBuilding`, and
    `SweetSpot`, `QueryLandingPad` and `Demo` are not called.
* `PACKXZ` values are two 16-bit fields that wrap round (as TA's engine packs them):
  scripts subtract one packed point from another, and the difference is right for points
  less than 32,768 px apart on a map of any size. `GROUND_HEIGHT(packed)` reads its point
  back as the one nearest the unit asking. Other values handed to scripts saturate at the
  `i32` range instead of wrapping (`HYPOT` squares in `i128`, `VETERAN_LEVEL` = kills × 100).
* TA 3.1's questions about units by id (`exthead.h`, as the mods' own script sources define
  them; none of TA's 841 scripts asks them): `MIN_ID` 69 and `MAX_ID` 70, the lowest and highest
  ids a unit may have; `MY_ID` 71; `UNIT_TEAM` 72 (the unit's player),
  `UNIT_BUILD_PERCENT_LEFT` 73, `UNIT_ALLIED` 74 and `UNIT_IS_ON_THIS_COMP` 75 ("is unit owned by
  the local computer"), each of the unit whose id is the first argument (none: the unit
  itself). A script id is the unit's place plus one. The mods made for the patched engines go
  through every id with them: Total Mayhem's geothermal plants find their upgrade module (by its
  height, at their own position, finished) and take it on, but only `if get
  UNIT_IS_ON_THIS_COMP(...)`, as TA ran each player's units on that player's computer and the
  change was made there alone; TA: Escalation asks them in 326 of its 550 scripts. Every
  computer plays the whole battle here, so every unit is each one's own: the change is made in
  every copy alike. Spring's engine took the same numbers up to 74; its 75 is `MAX_SPEED`.
* The mods' upgrades (Total Mayhem 11.3, TA: Escalation 10.2), as their files and scripts have
  them; none of TA's own units is built so:
  * A building makes its own upgrade module, as a factory makes a unit: TM's geothermal plants
    and moho mines and ESC's Aegis are `Builder=0` with a `WorkerTime` and a menu page offering
    it, ESC's towers and reactors `Builder=1`. A unit with build power builds what its menu
    offers whatever its Builder flag; one without builds nothing (TM lists a missile
    pseudo-unit, `MAKENUKEARM`, on its silos' menus for its computer players).
  * A module is given no footprint (`FootprintX=0`), or is `BMcode=1` with `MaxVelocity=1` and
    `Acceleration=0` (TM's underwater moho mines' 1x1 `ARMUMOHOUP`, ESC's `ARMHLT_UPGRADE`): to
    TA a mobile unit, off the map's grid, that never gets going. None of TA's 821 unit files
    gives a speed without an acceleration; a file that gives no acceleration at all is moved as
    it always was. It takes no ground and stands just where it was made, on the building's pad.
  * Building it, the scripts leave the build stance (`StartBuilding` sets INBUILDSTANCE to 0:
    no nano spray), and TM's geothermal plant switches itself off as well (its `Activate`
    readies it, then sets ACTIVATION to 0 70 ms later); ESC's Sentinel has no `Activate` and is
    readied in `Create`. TA built on: a factory needs its stance to start a unit, not to keep
    building it.
  * Done, `StopBuilding` goes through every id for a finished unit of the module's height
    (`UNIT_HEIGHT`, the highest point of its model at rest, exactly: TM's `[29.2]` is
    4,784,128, its `ARMGEOUP`'s; ESC's Sentinel's 1,269,760 its `ARMHLT_UPGRADE`'s) within a few
    paces of one of its pieces (TM's 20 px, ESC's 5 px) and `attach-unit`s it to its pad. A unit
    that carries none takes on its own player's finished, loose units so. On its building a
    module makes and stores what its file says (ESC's fusion reactors' +250 energy, its heavy
    fusion reactors' `EnergyUse=-1250`, switched on by the module's own `Create`), changes sides
    with it and goes with it, whatever shields it (ESC's armour themselves `DamageModifier=0`);
    the building's script shows its new pieces and sets ARMORED.
  * ESC's levels: the advanced and experimental labs make `ALL_L2` and `ALL_L3` (`canmove=0`, no
    footprint), which stand on the pad. Each unit born there afterwards looks once, in
    `Create`, for a finished one of its team within 62.5 px (`ALL_L2` 222,167 high, or an
    `ARMNOMAD`), and takes the level: 14 of its scripts look for `ALL_L2`, 16 for `ALL_L3`.
  * ESC's shields: an Aegis (`ARMSHGEN`, 160 high, `[64]`) armours its side's units round it.
    171 of the mod's scripts go through every id now and then for an allied one by its height,
    and set their own ARMORED (their `DamageModifier`) while one stands near: an Annihilator, a
    Light Laser Tower and a Sentinel beside one are armoured, the same far off are not, and the
    tower drops it once the Aegis is gone (`mod_upgrades.rs`'s `mod_shield_report`).
  * Two of ESC 10.2's Arm scripts look for the Core module's height: the Repair Pad's for
    `CORASP_UPGRADE`'s 3,009,819 (its own module is 1,711,415), Big Bertha's for
    `CORINT_UPGRADE`'s 3,781,490 (its own 1,820,443). Their modules are made and stand by,
    never taken on, as they did in TA.
  * ESC's repair tower upgrades (`ARMASP_UPGRADE`, `CORASP_UPGRADE`: `WorkerTime=360`,
    `Builddistance=448`, `canmove=0`, `canpatrol=0`, `canguard=0`) say
    `DefaultMissionType=RepairPatrol`, a mission none of TA's own units gives: on
    their own (the Arm one standing by its pad, the Core one taken on) they mend the damaged
    units of their side and its allies round them, the nearest first, whenever they have no
    orders (`sim/repair_round.rs`; `mod_upgrades.rs`'s `mod_repair_report`).
  * TM's mines cloak themselves (`UNIT_CLOAKED` 111, from a later patch; free, their files
    giving no `CloakCost`). TA's own mines (Core Contingency's) start cloaked from
    `init_cloaked=1`, as TA spells it.
* Aiming, as the shipped `Aim*` scripts expect it (`crates/ta_sim/src/combat.rs`;
  `tests/play_review.rs` plays the units named here):
  * A script ends an aim it no longer needs itself: every `Aim*` starts `signal SIG_AIM;
    set-signal-mask SIG_AIM`, and the threads it starts take that mask with them (its
    `RestoreAfterDelay`). Some aims guard a step from that signal with `set-signal-mask 0`:
    the Hammer, Zeus and Jethro unfold with a busy static set until they are done, and an aim
    that comes during it waits for it. An engine that kills an aim itself can cut such a step
    short and leave the static set: the unit never aims again.
  * The Slinger (`corah`) starts `RequestState(0)` from its aim after `set-signal-mask 2`, and
    its launcher takes about 0.73 s to unfold: an aim asked for meanwhile sends `signal 2`,
    cuts the state change short with its "changing" static set, and the launcher never opens.
    So a new aim is asked for only once the last has answered (or after three seconds
    without an answer).
  * `RestoreAfterDelay` sleeps twice the longest reload (`SetMaxReloadTime` × 2) before
    turning the turret back to rest: a gun firing on is aimed again at least once a reload
    (TA Reborn asks after every shot), else its turret rests between shots.
  * Only the Commanders' `AimPrimary` answers "no" (0: while building); the others answer 1
    once on target.
* **Firing**: the engine asks the weapon's `Query` function which piece the shot leaves from,
  then starts its `Fire` function, and the shot leaves from where that piece is once the fire
  function has run as far as its first wait (a flash shown, a barrel kicked back; `Sim::fire`).
  TA's Hammer alternates its barrels, its `FirePrimary` switching to the other as it fires and
  its `QueryPrimary` naming the one in use: asked before the fire function runs, the query names
  the barrel that flashes. TA: Escalation's C.O.R.E. (`CORMKL`) moves its eighteen launch points
  (single points in its six pods) 25,000 px off in `AimSecondary` and moves the one firing back
  into its pod in `FireSecondary`, before it shows its launch flash there: with the piece's place
  read before the fire function ran, every rocket left from off the map and went off at once, and
  only the flashes were seen. Read after it: 15 of TA's fire functions move or turn a piece at
  once before their first wait (most kick a barrel back a few px before they show their flash),
  and their shots leave from the muzzle so moved, where the flash shows; TA: Escalation's Armada
  (`ARMUSCAR`), which steps its anti-air rack's piece a slot down as it fires, launches each
  missile a slot (9.4 px) below its flash.
* `XZ_ATAN(dx, dz)` is the heading, relative to the unit's, that points a piece along
  **−(dx, dz)**. Only the four boom transports (`arm/corthovr`, `arm/cortship`) use it, as
  `XZ_ATAN(PIECE_XZ(pivot) − target)`: their booms pivot at the stern and rest pointing
  forward, so the angle must swing them towards the target (checked by
  `crates/ta_sim/tests/transport_boom.rs`; with the plain heading of (dx, dz) they reached
  away from their cargo and set it down on the wrong side).
* **`XZ_ATAN` and `ATAN(a, b)` (the angle of a over b: a slope, a up and b along) answer
  0..65535**, as TA's 16-bit angles: a heading just right of straight ahead, or a slope just
  below level, is just short of 65536, never negative. TA's own scripts only turn pieces to them (the boom transports' `BoomCalc`),
  where either way round is the same turn, but mods' scripts compare them: the walkers that
  keep their feet on the ground by searching for the spot each foot was set on (TA: Enhanced's
  Cavor; TA: Escalation's Gorgon, Gargant, Tick, Courier, C.O.R.E., Arbitrator, Experimental
  Construction Kbot and walking cruisers; Total Mayhem's Amphibious Construction Sub) step a
  foot towards the spot while `XZ_ATAN(spot − foot) > 16380 AND < 49140` (it is ahead) and up
  while `ATAN(spot_y − foot_y, distance) < 16380` (it is above), and transports' unloading arms
  (TA: Escalation's, TA: Enhanced's SpiderBot) take answers of 32768 and up as the other half
  turn. Read from −32768, every spot below a foot counted as above it: the feet climbed 80
  pixels a search into the air and the walkers stood on their backs, legs up (checked by
  `walkers_find_the_spot_their_foot_was_set_on` in `crates/ta_sim/src/sim_tests.rs`; the
  animation scan with `TA_TEST_MOD` finds them standing on nothing).
* **The angles the engine calls scripts with are 0..65535 too**: an aim's heading (from the
  unit's) and pitch (`AimPrimary`..`AimTertiary`), `StartBuilding`'s, and the wind's heading
  in `SetDirection`. TA's scripts turn pieces to them, or keep them for a turn later (the
  constructors' `StartBuilding` heading, the wind generators'), where either way round is the
  same turn. Mods compare them: Total Mayhem's shipyards and flagships pick the gun that covers
  a target by the heading's quarter (`heading > 24575 AND <= 40959`: behind), TA: Escalation's
  aims keep guns to their arcs (`heading > 16384 AND < 49152`: behind), and TA: Enhanced's
  SpiderBot stops its laser 50 degrees down (`pitch > 65536 - <50>`). Read from −32768, every
  target on a unit's right (headings past 32768) fell in the wrong quarter (checked by
  `aims_are_called_with_headings_and_pitches_from_0_to_65535`).

## Units, weapons, features

* 278 units, 198 weapons, 1,644 features, 15 movement classes. Speeds are px/tick
  (30 Hz), turn rates angle-units/tick; weapon velocity px/s, reload seconds. TA Reborn
  reads the numbers as f64 and whole numbers as i64 (up to 1.12, f32 and i32), so a mod's
  values past 16,777,216 keep their precision. The units name 54 distinct `Category`
  tokens.
* **Weapons: three a unit in TA** (`Weapon1` to `Weapon3`, the engine calling `AimPrimary`,
  `AimSecondary` and `AimTertiary`, and their `AimFrom`, `Query` and `Fire` functions). TA Reborn
  reads five more for mods (`Weapon4` to `Weapon8`, `ta_data::unit::MAX_WEAPONS`), calling the
  numbered functions Spring calls for any slot (`AimWeapon4`... `FireWeapon8`; for the first
  three, TA's names where a script defines them, else `AimWeapon1`...:
  `ta_data::unit::WEAPON_FUNCTIONS`), each with its own bad targets (`BadTargetCategory<n>`, after
  TA's `wpri_`, `wsec_` and `wspe_` keys). None of TA's own units has a fourth.
* **A unit's script goes by its `UnitName`**: `scripts/<UnitName>.cob`, whatever model
  (`Objectname`) it borrows. TA's own 278 units go by one name for both, but the mods show
  it: TA: Enhanced gives 62 units another unit's model, and 35 of them (the Enforcer, the
  Praetorian Hysteric, the Poseidon Gantry...) have scripts under their own names only;
  Total Mayhem's 31 have both, their own differing from the model's. A unit with no script
  of its own runs its model's, `scripts/<Objectname>.cob`, as TA Reborn always did:
  Escalation's xARMMLS (the Gremlin, `Objectname=ARMMLS`) and Total Mayhem's eight
  stockpile units on the Commanders' models have only that. (`UnitDef::script_paths`; the
  simulation, the mod check and the launcher's previews find scripts the same way.)
* **Steering**: `SteeringMode` 1 (89 of the 122 units that move on land or water: the
  tanks, hovercraft and ships, the Spider, the kbots that build, see radar or jam, and the
  crawling bombs; the aircraft carry it too) must be moving to turn: such a unit sets off
  the way it faces and comes round in an arc. 2 (the other kbots, the Commanders and their
  decoys, and two tanks, the Leveler and the Copperhead) turns standing and slows down to
  turn. (So the community's FBI guides describe them.) At speed v a unit turning at its
  `TurnRate` ω runs round a circle of radius v / ω: a Flash at full speed 44 px, a Bear
  147, a Millenium 326. (`TurnRate` is per tick: per second, as some guides have it, a
  Peewee would take half a minute to turn round.) `BrakeRate` is the speed lost a tick
  slowing down, and the fast tanks brake little: Flash 0.02, Stumpy 0.01, Bulldog 0.008 (a
  Flash rolls 100 px to a stop), the Bear 0.002.
* A builder finishes a unit in `BuildTime / WorkerTime` seconds: up to the Vulcan's and
  Buzzsaw's 88,000 to 190,000 ticks from a single advanced constructor.
* Factories set `CanMove=1` (they accept rally orders) — immobility is `MaxVelocity=0`.
* Solar collectors have *negative* `EnergyUse` (production while open). `EnergyUse` counts only
  while the unit is switched on (`ActivateWhenBuilt`, the player, or its script's ACTIVATION):
  TM's fusion reactor, `EnergyMake=1050` and `EnergyUse=-1050` and nothing to switch it on, makes
  1,050 ("Produces Energy (1K)"). A unit with no `EnergyUse` and no switch extracts and makes
  metal as it stands. A factory that is `TEDClass=PLANT` (every one of TA's, none of them
  `ActivateWhenBuilt`) opens for work and closes up idle; a mod's other building that builds is
  switched on when built as its file says, and keeps its switch, its work being on (TM's moho
  mines extract, ESC's repair pads repair).
* Radar towers have `EnergyMake=5` and `EnergyUse=5` (net zero).
* Plain ground has the schema's `SurfaceMetal`: 1 to 5 on most maps, 20 to 40 on lava
  worlds, 250 to 255 on metal worlds (Metal Heck, Metal Isles, Core Prime). Deposit cells
  run 56 to 223. TA Reborn lets extractors stand only where they cover a deposit cell,
  except where plain ground has 100 or more (`world::METAL_WORLD`).
* Acid worlds (10 maps, `planet=Acid`) set `waterdoesdamage=1` and `waterdamage` 10 (6 on
  Aqua Verdigris and Less Refreshing Sea, the sea maps; 100 on Cleanup on Kral). No other
  map sets it. TA Reborn takes it as health a second lost by what is in the water.
* Metal deposits are flat indestructible features with `metal>0`; the category is `metal`
  on most worlds but `rocks` for green-world `RockMetal*`. Extraction =
  `ExtractsMetal × Σ cell metal under the footprint`, background `SurfaceMetal`.
* TA Reborn's own features (`assets/reborn/features`) carry keys TA never had: `rebornmodel`
  (the model of TA Reborn's library drawn up close, `features.glb`; its painting drawn from it),
  `rebornlight=r g b reach height` (the light it casts by night, linear colour) and
  `rebornfireworks=1` (it sends up fireworks by night).
* Features block by default (`blocking` absent = 1). Geothermal vents (`geothermal=1`,
  1×1, no `blocking` line) must not: geothermal plants have `G` cells in their `YardMap`
  and are built over a vent.
* A feature with `blocking=0` does not stop a building being put over it. Of TA's 1,580 features
  649 say so: scars and smudges (flat), wreckage heaps and the sea units' wrecks (with metal),
  the lush and archipelago worlds' ferns and fronds (`category=foliage`, flamable, energy), a
  few small green-world stones, and the water world's anemones and corals (indestructible). TA
  Reborn draws the ferns, fronds and stones as 3D models, which would stand through a building,
  so a building clears its footprint as it takes the ground (as its construction starts, or as a
  mission places it) of those that hold no metal and can be destroyed, deposits and vents
  aside; they leave nothing. Heaps keep their metal under it; anemones and corals stay.
* **Wrecks give metal only.** A unit's corpse (its `Corpse`, `features/corpses/`, with the
  `featuredead` it leaves when destroyed: its heap, `category=heaps`) gives metal and no energy: no
  corpse or heap of a unit that costs metal names any. The four corpse files that give energy are
  the remains of TA's indigenous lifeforms, the Scorpions (`ARMSCORP`, `CORSCORP`) and Sea Serpents
  (`ARMSS`, `CORSS`), units that cost energy alone (15,660 and 4,385): `metal=0`, `energy=4698` and
  `1316`, given back as a tree's energy is. TA Reborn holds to it whatever a file says: wreckage,
  the corpse of every unit type that costs metal and what it leaves when destroyed, and so on, pays
  its metal alone when reclaimed (`Sim::reclaim_value`, the set made once with the simulation,
  `sim::wreckage`), so a mod's wreck that gives energy gives none; the remains of a unit that costs
  no metal, rocks and trees pay as their files say. The reclaiming, the time it takes, builders on
  patrol, the computer players and the interface all go by it.
* **A wreck lies as its unit faced when it died**: the feature keeps the dying unit's heading (a
  heap the wreck's), and is drawn turned so; its footprint is the corpse file's, square to the map,
  whichever way it lies. The map's own features have none (TNT files give no heading).
* A feature's `world` (or its folder under `features/`) against the OTA `planet` of the
  maps that use it: Green Planet → `greenworld`, Red Planet → `mars`, Lunar → `moon`, Water
  World → `water`, Wet Desert → `wetdesert`, the others by name (Acid, Archipelago, Crystal,
  Desert, Ice, Lava, Lush, Metal, Slate, Urban); `All Worlds`/`allworld` features (dragon's
  teeth, fortifications, scars) and the unit wrecks (`corpses`) go anywhere. Green and water
  worlds also use urban features. Categories include `trees`, `foliage`, `rocks`, `spires`,
  `metal`, `steamvents`, `arm_corpses`/`core_corpses`, `heaps`, `buildings`, `scars`.
* Categories carry tech tags `LEVEL1`..`LEVEL3` (advanced factories are `LEVEL2`, nuke silos
  and anti-nukes `LEVEL3`). Nukes: `stockpile=1 commandfire=1 targetable=1`, 180,000 E +
  2,000 M per missile over `reloadtime` 180 s, `areaofeffect=512`; anti-nukes
  `interceptor=1 coverage=2000`, 10,000 E + 200 M per interceptor.
* **`interceptor` and `targetable` are classes**, as the Spring engine reads them: bits of the
  number written (0 when not given; a negative number every bit). An interceptor takes a missile
  when its `interceptor` and the missile's `targetable` share a bit (`WeaponDef::intercepts`);
  any number but 0 makes a weapon an interceptor (or targetable). TA's own all say 1
  (`ta_data::STRATEGIC_CLASS`), so every anti-nuke of TA's takes every one of its strategic
  missiles, nukes, EMP and tactical missiles alike. The Supreme Commander factions' strategic
  missiles say 1 and their anti-nukes take them; their tactical missiles say 2 and their tactical
  missile defences and ships' point defences (`interceptor=2`) take those alone.
* An interceptor fires at a missile headed into its `coverage` (or crossing it) once the missile
  is within its `range` (its coverage, where that is larger; with no range written, wherever it
  is), as the Spring engine's do: TA's anti-nukes (`range` 22,000 to 32,000) take a nuke as it
  leaves its silo on TA's maps, a tactical defence a few hundred px across as the missile comes
  down on what it covers (`Sim::update_interceptors`).
* **Missile speeds**: a missile leaves at its `startvelocity` (px/s; none given, at its
  `weaponvelocity`, or launched upright with a `weaponacceleration`, at rest) and its speed goes by
  its `weaponacceleration` (px/s a second) towards its `weaponvelocity`, stopping there. Every one
  of TA's 198 weapons starts no faster than its `weaponvelocity` and gathers speed or holds it.
  TA: Escalation's vertical missiles (12 weapons, among them its missile batteries' and the
  Raptor's `VSPAM_ALL`, its battleships' and mobile fortresses', the Deluge's, Hammerhead's,
  Arbitrator's and Talos's, the Abyssal's and Vortex's `NUKE_SUB_ARM`, `NUKE_SUB_CORE`) leave at
  360 to 1,280 px a second with a `weaponvelocity` of -1 to -10 and a negative acceleration (-12
  to -300): a missile launched faster than its top speed is not pulled down to it, and they slow as
  they fly. Their flight times end before they would stop (half of them with about a tenth of their
  start speed left): their makers timed them by the slowing. Its pulse bombs (`BOMB_MS`,
  `BOMB_SHOCK`) are launched upright at -400 px a second with a `weaponvelocity` of -400: straight
  down.
* `tracks=1` (a guided weapon homes on the unit it is sent at): every guided weapon of TA's
  launched other than upright says it, and of the vertical launchers the anti-nukes
  (`AMD_ROCKET`, `FMD_ROCKET`, `ARMSCAB_WEAPON`, `CORMABM_WEAPON`) and the Krogoth's
  `CORKROG_ROCKET`. The nukes (`NUCLEAR_MISSILE`, `CRBLMSSL`), the EMP and tactical missiles
  (`ARMEMP_WEAPON`, `CORTRON_WEAPON`) and the rockets `ARMTRUCK_ROCKET`, `CORTRUCK_ROCKET`,
  `ARMMSHIP_ROCKET`, `CORMSHIP_ROCKET`, `ARMMH_WEAPON`, `CORMH_WEAPON`, `CORHRK_ROCKET` and
  `SBMISSILE` say `guidance=1` without it. TA's nukes went to the place their target stood at
  their launch; TA Reborn sends a stockpiled missile without `tracks` there too
  (`WeaponType::strikes_a_place`), and the other missiles after their target.
* Targeting tags: of TA's weapons only the flak (`ARMFLAK_GUN`, `CORFLAK_GUN`, `ARMyork_GUN`,
  `CORSENT_GUN`, `ARMAAS_WEAPON3`, `CORARCH_WEAPON3`) say `toairweapon=1`; the anti-air
  missiles (`ARMKBOT_MISSILE`, `ARMRL_MISSILE`, `ARMTRUCK_MISSILE`, the fighters') are guided
  missiles that can hit anything. What keeps an anti-air unit off the ground is its unit's
  `BadTargetCategory=NOTAIR` (with `wpri_badTargetCategory`, the same); the ground's armed
  units say `VTOL`, and most also `NoChaseCategory=VTOL`. Per weapon `wpri_`, `wsec_`,
  `wspe_badTargetCategory` (TA3D documents `wthi_` for the third): the cruisers' depth
  charges say `wsec_badTargetCategory=NOTSUB`, the Doomsday Machine's light laser `NOTAIR`, a
  Panther's and a PT boat's anti-air missile (their second weapon) `NOTAIR`. A unit takes a
  bad target on last: with nothing else in reach (the missile towers fire on tanks when no
  aircraft are about, as players remember TA), and `+shootall` makes every target alike. Slips in the files: the Battle Tactics
  anti-air hovercraft (Swatter `ARMAH`, Slinger `CORAH`, "Anti-Air Hovercraft") carry the
  ground hovercraft's `VTOL` (read as `NOTAIR`); the A.K. (`CORAK`), the seaplane fighters
  (`ARMSFIG`, `CORSFIG`), the Core's naval missile tower (`CORFRT`) and the Krogoth, among
  others, carry none and take on air and ground alike. Mods with category tokens of their own
  can name every unit a weapon could hit: Total Mayhem's flak towers (`ARMFLAK`, `CORFLAK`) list
  `ALL`, which every one of its units carries; Escalation's anti-air missiles `NLND` (not a land
  unit) or `MOB`, which all its aircraft carry; TA: Enhanced's Nightflower `NOTSUB` on rockets
  that only hit aircraft. Held to, those weapons would never fire of their own accord (TA's own
  data has no such weapon): their bad targets are lifted, and they take on what they can.
* `NoAutoFire=1` (the Big Bertha, the Intimidator, the Vulcan, the Buzzsaw, the Doomsday
  Machine, the Annihilator): TA set the unit to hold fire on construction, whatever its
  `StandingFireOrder` (2, fire at will, in the Doomsday Machine's and the Annihilator's files;
  `sim::initial_fire_state`). The player switches it to fire at will. A campaign mission's
  units placed for the player were never built, and keep their file's `StandingFireOrder`
  (`sim::file_fire_state`: the Annihilators placed in I09 and T31 fire at will). The anti-nukes say
  `NoAutoFire=1` too but are the exception: they keep fire at will, and fire at nukes by
  default whatever their fire orders. The others that start at hold fire say
  `StandingFireOrder=0`: the bombers, the crawling bombs, the silos, the EMP launcher and the
  Sniper.
* Weapon `rendertype`: 0 laser, 1 model (missile), 2 point, 3 D-gun, 4 shell, 5 flame,
  6 bomb, 7 lightning.
* Weapon `smokedelay` and `endsmoke`, as TA's own weapons file explains them in its comments: the
  seconds between a smoke trail's puffs, and a puff of smoke where a shot ends. 50 of TA's weapons
  say `smokedelay=.1` (the default too); the Pyro's `FLAMETHROWER` says `smokedelay=1` and
  `endsmoke=1`, the only `endsmoke` of TA's. TA Reborn puffs a `smoketrail` 2.5 times as often as
  `smokedelay` says, smaller (TA's 0.1: every 0.04 s), and a shot of an `endsmoke` weapon ends in a
  puff. The keys a mod gives its weapons' looks beyond TA's are docs/MODDING.md's.
* **`HealTime`**: only the Commanders (`ARMCOM`, `CORCOM`) give one, 27. Modding guides call it
  the time in seconds it takes the unit to heal itself; players timing TA's Commander found it
  mending about 100 hit points every 27 seconds (some 3.7 a second; one test found the same rate
  whatever `HealTime` said). Nothing known gives it an energy cost or a pause after damage. TA
  Reborn: a finished, living unit whose type gives a `HealTime` over 0 regains 100 hit points every
  `HealTime` seconds (`defs::HEAL_POINTS`, `UnitType::heal_ticks`), a share each tick, the shares of
  every `HealTime` of ticks adding up to exactly 100, up to its `MaxDamage` and no further, at no
  cost, under fire, paralysed or carried too (`Sim::heal_units`); a frame does not, nor a unit being
  taken apart. A Commander worn down to 1,000 of its 3,000 is whole again in nine minutes. The mods
  give it widely, lower for what should heal faster: Total Mayhem 118 units (3 to 255, every value
  one less than a power of two), TA: The Final Frontier 55 (1 to 55: its orbital defences 1), TA:
  Escalation 5 (its Commanders 7, its Krogoth, "Self-Heal", 3).
* The Commander's laser (`ARMCOMLASER`, the J7): range 200, reload 0.85 s, 60 damage (about
  70 a second). The Pyro's `FLAMETHROWER`: range 160, reload 1.2 s, a burst of 17 × 10
  (about 140 a second). Two Pyros burn through a Commander's 3,000 in about 11 s, while its
  laser takes about 10 s to kill one of them (700): a Commander left to itself against two
  dies unless its D-gun is used (the mission AC09, played by nobody, is lost so).
* Flames (`rendertype=5`: `FLAMETHROWER`, the Pyro's and the Sea Serpent's, the only one)
  harm neither the unit that throws them nor its side ("its flame will not harm your units",
  StrategyWiki's guide to TA's units says of the Pyro). Its `corpyro=2` (and
  `CORPYRO_BLAST`'s `corpyro=15`, a Pyro's death blast) is the Pyros' resistance to enemy
  Pyros. Burnt by its own flames at a target beside it, a Pyro lost a third of its health in
  20 s; a Sea Serpent, with no such entry, takes five times as much a flame.
* Shells don't always carry as far as their `range`: the farthest a shell of speed v goes on
  level ground is v² / g (`gravity` 112 px/s² on most maps). The Guardian's and the
  battleships' 1,250, the Punisher's 1,200 and the cruisers' 1,300 are past the 1,094 their
  350 px/s shells carry: TA Reborn's guns pick targets their shells reach.
* Submarines are the ships with a `WaterLine` (ARMSUB 20, CORSSUB 25, ARMSUBK and CORSHARK
  20; category `UNDERWATER`); surface ships have none. Only `waterweapon=1` weapons
  (torpedoes, depth charges) reach a submarine, or a unit on the sea bed with the water over
  its top; no stock weapon sets `firesubmersed`.
* TA's buildings in the water come in two kinds. Those on the water give a `WaterLine`, how deep
  the model's base sits (the floating towers ARMFRT 0.3 and CORFRT 4, the floating heavy lasers 3,
  metal makers 4 and 22 and dragon's teeth 6 and 1, the torpedo launchers 10, the advanced ones 30
  and 10, tidal generators 8 and 10, sonar stations, shipyards). Its underwater buildings give
  `MinWaterDepth` with no `WaterLine` and no `Floater` and stand on the sea bed: the underwater
  metal extractors (ARMUWMEX, CORUWMEX), storages (ARMUWES, ARMUWMS, CORUWES, CORUWMS) and
  fusion plants (ARMUWFUS, CORUWFUS). Their models are
  taller than the shallowest water they may stand in (ARMUWMEX 42 px in `MinWaterDepth=19`,
  CORUWMEX 19 in 10, CORUWFUS 30 in 15, CORUWMS 57 in 40), yet TA's surface weapons never fire on
  them (players: a Skeeter never shoots an underwater metal extractor; only torpedoes and depth
  charges take them on). TA Reborn holds them wholly under the water wherever the water covers
  their base (`UnitDef::stands_on_sea_bed`, `Sim::submerged_at`), and sonar, not radar, shows
  them. The advanced torpedo launchers' tops come within a fraction of a pixel of the surface
  (ARMATL 30.3 px tall at `WaterLine=30`, CORATL 10.04 at 10) and count as out of it.
* Torpedoes and depth charges say `weapontimer=3` (5 for the seaplanes' and torpedo bombers') at
  85 to 110 px a second, against ranges of 375 to 650: ARM_TORPEDO and CORE_TORPEDO 300 px in
  3 s against a range of 400, the depth charges 300 and 330 against 400 and 410, CORSSUB_WEAPON 300
  against 500, the advanced torpedo launchers' ARMATL_TORPEDO 270 against 650 and CORATL_TORPEDO
  255 against 600. So `weapontimer` cannot have been their lifetime in TA; TA Reborn runs a
  torpedo at least as far as its range (as far as a projectile without a timer flies). (Two weapons
  besides fly shorter than their range by their `weapontimer`, neither self-propelled: `MINDGUN`
  and `VTOL_EMG2`.)
* Targeting facilities: Core Contingency's ARMTARG and CORTARG ("Targeting Facility", described
  "Automatic Radar Targeting") say `istargetingupgrade=1`, `onoffable=1`, `ActivateWhenBuilt=1`
  and `EnergyUse` 1,250 and 1,300 a second; no other unit says it. As their description says,
  with one at work a player's units firing at will take on what radar shows as well as what is
  seen, which they otherwise leave to orders. TA Reborn counts one at work as it counts a radar:
  finished, switched on, its upkeep paid (short of the energy, it stops) and not paralysed; it
  directs its owner's units only (allies share radar, so they fire on contacts any ally's radar
  shows). TA Reborn has no radar error (contacts are where the units are), so a facility has none
  to take away.
* A second or third weapon's own bad targets (`wsec_`, `wspe_badTargetCategory`) set its role
  apart from its unit's: TA gives them to the Skeeter's and the Searcher's anti-air missiles
  (`NOTAIR`, ARMPT and CORPT, which players know to pick their own targets, the nearest enemy,
  whatever the boat is sent at), the Panther's (ARMLATNK) and the missile ships' (ARMMSHIP, CORMSHIP),
  the Phoenix's and the Hurricane's anti-air lasers (ARMPNIX, CORHURC), the Doomsday Machine's
  light laser (CORDOOM), the stealth fighters' second missiles (ARMHAWK, CORVAMP), the anti-air
  ships' second weapons (ARMAAS, CORARCH), the destroyers' and cruisers' depth charges (`NOTSUB`:
  ARMROY, CORROY, ARMCRUS, CORCRUS), and `VTOL` to the Arm battleship's second gun (ARMBATS) and the
  seaplanes' second torpedoes (ARMSEAP, CORSEAP). Such a weapon leaves an attack order on one of
  them to its unit's other weapons and keeps choosing its own targets (`Sim::keeps_to_own_targets`);
  a weapon whose bad targets come from its unit's `BadTargetCategory` (most units' `VTOL`) takes
  the order as the first does. No third weapon of TA's has one of its own.
* Aircraft have no `Corpse`, their `Killed` scripts only burst their pieces, and their
  `ExplodeAs` is a ground unit's (`SMALL_UNITEX`, `BIG_UNITEX`: shells, AOE 30 and 110,
  damage 30 and 50; the Thunder's `LIGHTNING`): nothing in the data makes a plane crash. TA
  Reborn blows them up where they die, and the blast spares the aircraft flying round it (a
  shell's, which never engages aircraft); the client draws the remains falling.
* Death blasts the data names but no archive has: `MEDIUM_UNITEX` (ExplodeAs of the
  Leveler, Core's advanced torpedo launcher, the torpedo seaplanes and seaplane fighters, the
  radar jammer ships) and `BIG_BUILDINGEX` / `BIG_BUILDING` (the moho metal makers'
  ExplodeAs and SelfDestructAs; the buildings' blasts are SMALL_, MEDIUM_, LARGE_ and
  ESTOR_BUILDING, the units' SMALL_ and BIG_UNIT). All are Core Contingency's units: its
  `weapons/units.tdf` added `MEDIUM_UNIT` (ID 110, outside the file's own "Allowed IDs =
  200-224"), their SelfDestructAs, and no `MEDIUM_UNITEX`, and shipped so through Battle
  Tactics and the 3.1 patch. TA keeps weapon ID 0 for none, `NOWEAPON` in weapons.tdf ("No
  Weapon": range 16, no damage, area, render type or explosion), what an empty weapon slot
  holds. A name no weapon answers to most likely comes to that, or to no weapon at all, as
  the walls' and dragon's teeth's empty names do (the executable is not looked into): no
  blast either way. So these units die without a blast: their `Killed` scripts burst them
  into pieces, as every unit's do (TA Reborn's client draws a burst too).
* **Build pads turn**: 15 of TA's 21 plants spin their build pad (the piece `QueryBuildInfo` names)
  about y while they build: `StartBuilding` does `spin pad around y-axis speed <30>` (5461 a
  second, no acceleration) and `StopBuilding` `stop-spin pad around y-axis` (at once), so the pad
  stops where it is and the next unit starts from there. The Arm and Core vehicle plants, kbot labs,
  aircraft plants and seaplane platforms, their advanced ones and the Core Hovercraft Platform do;
  the shipyards (their pad `slip`, or `base`), the Arm Hovercraft Platform and the Core Gantry (its
  gangplank only slides) don't. What a plant builds stands on the pad and turns with it, from the
  plant's own heading on, as TA's players saw it: the plant's heading and the turns about y of the
  pad and the pieces it hangs from (`unit::piece_world_heading`, `Sim::follow_build_pad`). Done, it
  keeps the heading the pad left it with, and turns to its way out as it leaves.
* Factory yard maps: the cells marked `o` (and `w`, `f`, `s`) block, the others are the yard,
  open to walk on. The Arm lab's `yoccoy ooccoo ooccoo ooccoo ooccoo yoccoy` is a corridor
  two cells wide through its middle, north to south, with walls either side: a unit built on
  the pad walks out through the factory's front wall (the engine lets it, until it is clear
  of the footprint).
* **`UpgradesTo`** (TA Reborn's, for mods; Supreme Commander's `General.UpgradesTo`): the unit a
  unit becomes upgraded in place (`Order::Upgrade`, docs/MODDING.md). None of TA's units gives
  it. It builds that unit on itself with its own `WorkerTime` (and helpers'), at that unit's price
  and build time, as construction pays and stalls; done, a unit of that type stands in its place,
  finished, with its share of health, its queued orders, a factory's queue and route, its fire and
  move orders and experience; the old one is taken off the map without a wreck or a loss. Called
  off, its metal comes back (not its energy). A factory makes nothing while it upgrades.
* **`[ENHANCEMENTS]`** (TA Reborn's, for mods; Supreme Commander's `Enhancements`), a section
  within `[UNITINFO]`, one section an enhancement named by its id, in the menus' order: parts a
  unit installs on itself (`Order::Enhance`), one a `Slot`, a `Prerequisite` first, at a unit's
  price and work (`BuildCostMetal`, `BuildCostEnergy`, `BuildTime`) with its own build power, as an
  upgrade in place is built. Installed, its `MaxDamage`, `HealRate` (hit points a second),
  `WorkerTime`, `SightDistance`, `RadarDistance`, `SonarDistance` and `BuildOptions` add to the
  unit's (`Boost`, `ta_sim::unit`), its `WeaponN` takes the unit's slot N, its `CloakCost`,
  `CloakCostMoving`, `MinCloakDistance` and `Stealth` give it a cloak and stealth. The unit's
  health rises with its most and falls to it, no further, when one is taken out (at once, for
  nothing). The script's `SetEnhancement(index, on)` hears of each (the index in the file's order).
  None of TA's units has any; the state is in the checksum (`Unit::enhancements`).
* **`[EXITS]`** (TA Reborn's, for mods; Supreme Commander's `RollOffPoints`), a section within a
  factory's `[UNITINFO]`, a section a way out in order (`ta_data::ExitDef`): `X` and `Z` where it
  leads (px from the factory's middle at its heading 0, turned with it), `Heading` the way a unit
  built for it faces (degrees, 0 the factory's front, 90 its +x; beside the pad's own turn), and
  `Ramp` (two numbers, px along the way from the pad: the pad's height kept to the first, the
  ground at the second). A unit begun takes the way nearest the first place of the factory's route
  (else its front), of those whose end it may stand on, the first of equals, and stands on the pad
  from its first tick facing it (one that never moves takes none: it is made as the pad faces);
  done, it is sent to the way's end (`Order::Move`, then the route), its height on the way the
  pad's to the ramp's top, down evenly to the ground at its foot (the higher of that and the
  ground, and tipped by it: `unit::Way::floor`, `Sim::settle`), until it is out of the footprint.
  Aircraft lift off the pad as before. A unit's own `BuildHeading` (degrees, as a way's `Heading`)
  is the way it faces while built by such a factory, whichever way it takes
  (`UnitType::build_heading`: Supreme Commander's `ForcedBuildSpin`). None of TA's factories has
  any: they play as ever. The way is in the checksum (`Unit::roll_off`, only where there is one).
* **`OnlyTargetCategory<n>`** (Spring's name; TA Reborn reads it for any slot, `n` from 1): the
  categories of the only units the weapon in slot `n` takes on, automatically, firing back and by
  order: an attack order on another unit, or on the ground, is not that weapon's (and ends when no
  weapon of the unit's takes it), nor is a unit it names where it cannot hit it
  (`Sim::only_allows`: `toairweapon` and a landed aircraft). None of TA's units names any: TA's
  `toairweapon=1` weapons still fire on what an order names (`Sim::takes_order`).
* **`TARGET_ID`** (unit value 83, Spring's: `get TARGET_ID(n)`): weapon `n`'s target (from 1), a
  unit's script id, -1 none, -2 a place, -3 a missile it intercepts, -4 no such weapon.
* **`IMMOBILE`** (unit value 200, TA Reborn's own): set by a script, its unit is held still
  (`Unit::immobile`, in the checksum where set; a unit on the ground or the water: aircraft fly
  on): no movement, no turning, not pushed by the crowd, its orders waiting. With a way to go it
  counts as setting off for its script (`StartMoving`, once, `Unit::moving`), so that the script
  can let it go; its way given up, `StopMoving`. From then until it is let go it fires nothing
  (`Unit::packing_to_go`: an aim answered before no longer fires). The Supreme Commander
  factions' weapons that lock their unit's motion while unpacked set it.
* **`[ADJACENCY]`** and **`AdjacencySize`** (TA Reborn's, for mods; Supreme Commander's
  `Adjacency` and its `SIZE4` to `SIZE20` categories): a section within `[UNITINFO]`, one section a
  bonus (any name), each `Affects` (`BuildEnergy`, `BuildMetal`, `EnergyUse`, `EnergyPerShot`,
  `ReloadTime`, `EnergyMake`, `MetalMake`), `Add` (the share added: -0.0625 a sixteenth less),
  `Size` (the size class it reaches; none, any) and `Category` (tokens a receiver's `Category`
  must all have). A finished building gives its bonuses to the finished buildings of its player
  whose footprints meet its own along an edge, that have what the bonus changes; the shares of every
  neighbour are summed, the figure times one and the sum (never below none), from at most
  `AdjacencySize` neighbours (those along the most of its edge first; none given, any number).
  Looked at again whenever a building there is finished, lost, upgraded or changes hands
  (`sim/adjacency.rs`; docs/MODDING.md). None of TA's units has any.
* **`DefaultMissionType=AssistPatrol`** (TA Reborn's, for mods; Supreme Commander's engineering
  stations): a builder that, with no orders, helps build what its player puts up within its
  `BuildDistance`, else mends its side's and its allies' damaged units there, else reclaims the
  wreckage there, standing where it is (`sim/repair_round.rs`; docs/MODDING.md). None of TA's
  units gives it.
* Build menus: `guis/<BUILDER><page>.gui` + `download/*.tdf`. The 3.1 patch's `[CANBUILD]` in
  sidedata is the computer players' list, never a player's menu: mods list units of their AI
  there (TA: The Final Frontier's AI Boost: `AAISOLAR` beside the `ARMSOLAR` a player builds,
  `AAILAB`, and ground labs its command cruiser builds only for the AI; Total Mayhem's
  `AIAMEXC`).

## In-game interface (GUI, fonts, side data)

* `guis/*.gui` are TDF: `[GADGETn]` sections in file order (numbers repeat), each with a
  `[COMMON]` `id` (0 panel header naming its `panel` art, 1 button, 5 label, 7 font),
  `name`, `xpos/ypos/width/height` relative to the header, which sits at (0,128) of the
  640×480 screen, and `attribs` (16 page tab, 32 build button, 64 order, 256 state
  button). A button's art is the GAF entry of its name: `anims/<gui>.gaf` on build pages,
  `anims/commongui.gaf` for the shared gadgets (`ARMORDERS`, `CORSTOP`..., the panels
  `ARMPAN`/`ARMPAN2`, `igpatch` for empty build places). Buttons have three frames (up,
  down, greyed); state buttons one per state, then "mixed", down and greyed.
* Build pages have six places, two across and three down at x 0/64, y 27/91/155. Download
  menu entries (`download/*.tdf`) count the orders menu as menu 1: `MENU=2` is the first
  build page (every `MENU-1` place is an empty `IGPATCH` of that page or on a new page);
  their buttons are `anims/<unit>_gadget.gaf`.
* The mods made for the patched engines of later years lay their side pages out on a
  768-pixel screen (Total Mayhem's engine refuses a shorter one: `DisplayModeMinHeight768`):
  TA: Escalation's and Total Mayhem's headers are 128×640 at (0,128), their build pages twelve
  places (two across, six down from y 0), the page arrows at y 388, and on the same page the
  state buttons (`CLOAK` 409, `FIREORD` 432, `MOVEORD` 455) and the orders (55×31, rows at
  479 to 607). There are no ORDERS and BUILD tabs on their build pages: their own
  `commongui.gaf` (which replaces TA's) has no art for them. Their panel art stays TA's size
  or a little more (`ARMPAN` 128×352, Total Mayhem's 128×472; `PANELSIDE` 128×480): the
  engines filled the rest of the taller panel. A download entry's `BUTTON` counts that page's
  places. TA: Enhanced keeps TA's six.
* Total Mayhem's build pages also share one place among several build buttons, and lay one
  button over several places. A share is no larger than a place and off the grid; it is seen
  only in the place its top-left corner is in. Its long-range guns (Big Bertha, Vulcan, Dora;
  Core's Intimidator, Buzzsaw, Nator) are four units round one picture, each setting
  `buildangle` in `Create` (0, `<90>` east, `<180>` north, `<270>` west): strips 18×64 at the
  place's left and right edges for west and east, 28×32 at the top middle for north, and the
  plain one at the bottom middle (Core's north and plain ones are whole 64×64 buttons at
  x 82, laid over the rest). A point goes to the first button in the file holding it, so the
  side of the picture clicked is the way the gun faces. Its plants come in halves (32×64,
  `ARMHP`/`ARMHPNS`, land and sea, each with its own 32×64 frame); its missile silos' and
  experimental plants' big buttons (128×192, 128×128) have their own art at that size. Its compass page's own GAF
  (`anims/armack1.gaf`...) is empty (0 bytes), so the four variants' identical build pictures
  are drawn.
* `gamedata/sidedata.tdf` `[SIDEn]`: `intgaf` (PANELSIDE 129×480; PANELTOP ~512×32 from
  x 127; PANELBOT from (128,447)), `font` (bars) and `fontgui`, the bar palette ramps
  `metalcolor`/`energycolor`, and boxes on the 640×480 screen for the resource readouts
  (`METALBAR`, `METALNUM`, `METAL0`, `METALMAX`, `METALPRODUCED`...; the storage number
  must end at `METALMAX`, starting there it would run over the art's "+") and
  the unit bar (`UNITNAME`/`DAMAGEBAR` selected, `UNITNAME2`/`DAMAGEBAR2` hovered,
  `NAME`/`DESCRIPTION` for a hovered button). Some second corners are junk.
* A side's emblem (`LOGO` at the top left, `LOGO2` in the unit bar, 21×21 boxes) is drawn from
  `anims/logos.gaf` (3.1's): `Arm32Lt` and `Core32Lt` (`…Dk` darker ones beside them), ten
  frames of 32×32 (the third 32×33), every pixel opaque: the emblem filled with the player's
  colour, one entry of the palette a frame, in the players' order 227, 212, 255, 235, 108, 219,
  208, 0, 130, 67 (blue, red, white, green, navy, purple, yellow, black, sky blue, khaki; the
  medium swatches of the same GAF's `colorsmd`, but white's and black's, 80 and 93 there),
  outlined in grey (92, or 89 round the lightest and darkest colours) with lighter lines inside,
  on a metal ground of the greys 80 to 87. `32xlogos` is a plain square of the player's colour.
  Side data names no emblem: TA Reborn's `logogaf`/`logo` keys add one (docs/MODDING.md, "Sides
  of a mod's own").
* Fonts (`fonts/*.fnt`): `u16` height, `u16`, 256 `u16` glyph offsets (0 = none); a glyph
  is a width byte and `width × height` bits, most significant first, rows not padded.
  Widths include the spacing to the next letter.

## Media

* Cutscenes: `Data/*.ZRB` are Smacker 2 (640×240 line-interlaced → 640×480, 30 fps,
  DPCM 22 kHz stereo audio). Decoder: `crates/ta_video`.
* Music: `music/0..17.mp3` (GOG converted CD audio). Sounds: 11 kHz 8-bit mono WAV.
* COB `play-sound <n>` (one operand, nothing taken from the stack, as TA Reborn runs it): the
  script's sound `n`, of the sound table TA: Kingdoms' scripts carry (version 6: two more header
  words after the code pointer's and the zero after it, at bytes 44 and 48: where the table of
  pointers to the sounds' names is, and how many; the names nul-ended). TA's own scripts (version
  4) have none; TA Reborn writes a script with sounds as version 6 (`ta_formats::cob`).

## Supreme Commander's sound banks (XACT 3, version 43)

Verified over the 90 sound banks and 89 wave banks of GOG's Forged Alliance (`sounds/` and
`sounds/Voice/US/`; `crates/ta_supcom/src/xact.rs`, docs/SUPCOM_MUSIC.md for the wave banks and
the banks' headers): every event below reads, and every wave a cue names is in a wave bank.

* **A cue** (complex, 15 bytes): flags (u8; 0x04: of one sound), the sound's offset (u32), 4
  bytes, its instance limit (u8: at most this many at once; 255 none), fade in and out (u16 ms
  each), flags (u8: its behaviour past the limit in its top five bits: 0 not played, 2 the oldest
  gives way, 4 the least important). A simple cue is 5 bytes: flags, the sound's offset.
* **A sound:** flags (u8: 0x01 of clips, 0x02 curves of its own, 0x04 its clips' curves),
  its category (u16, the global settings'), volume (a volume byte), pitch (i16, hundredths of a
  semitone), priority (u8), its length (u16); then a wave of its own (track u16, wave bank u8) or
  a clip count (u8); where any of 0x0E is set, the curves (a u16 length counting itself; with
  0x02 a count, u8, and the curves' offsets in the global settings, u32 each; then each clip's);
  then 5 bytes a clip: its volume (a volume byte) and its events' offset (u32).
* **A volume byte** is 3969 · log10(b / 28240) + 8715 hundredths of a decibel (FAudio's reading:
  0xB4 is +0.01 dB, 0 silence).
* **An event:** a u32 (its kind in bits 0–4, its delay after the cue starts in bits 5–20, ms),
  a random delay (u16, ms: up to that much more), then a separator (0xFF) and flags (u8). The four
  kinds the units' banks use, after the header (offsets from the separator):

  | Kind | Times used | Layout |
  | --- | --- | --- |
  | 1, a wave | 108 | +2 track u16, +4 wave bank u8, +5 loop count u8, +6 pan angle u16, +8 arc u16 (10 bytes) |
  | 3, one of several | 5 | +2 loop count u8, +3 angle u16, +5 arc u16, +7 count u16, +9 variation u16, 4 bytes, +15 the tracks |
  | 4, a wave, varied | 1,557 | as 1, then +10 pitch from and to (i16 each), +14 volume from and to (bytes), +16 what varies (u8) (17 bytes) |
  | 6, one of several, varied | 118 | +2 loop count, +3 angle, +5 arc, +7 the 7 bytes of 4's ranges, +14 count u16, +16 variation u16, 4 bytes, +22 the tracks |

  A track is 5 bytes: track u16, wave bank u8, weight from and to (bytes: its weight the
  difference; 0 and 255 in every one). A loop count of 255 plays on until stopped (384 events);
  another plays that many times more (one event once more, one 4 times more). What varies: 0x40 the pitch (from its two values), 0x80 the volume; where a flag is
  clear its values are XACT's defaults, -100 to 100 cents and bytes 151 to 214, and nothing varies
  (held so in every event: 128 events vary the pitch alone, 100 the volume alone, 1,447 both).
  A variation's low three bits choose: 3, never the same twice running (121 of 123; the others
  0x43). 124 events start after a delay (50 ms to 5.3 s), 6 with a random one on top.
* **A wave** is found through its event's wave bank index: the sound bank's list of wave banks
  names the bank by its own name, which is not always its file's (`XAS_Weapon` is in
  `XAS_Weapons.xwb`); cues of one bank play another's waves (170 of the 3,476 waves cues name).
* **The global settings** (`SupCom.xgs`, `XGSF`, tool 43, format 42): after the magic, the tool
  and format versions (u16 each), a checksum (u16), 8 bytes of date and the platform (u8); the
  counts (u16 each from byte 19: categories, variables, two unused, curves, two of DSP), then 11
  offsets (u32 from byte 33: categories, variables, two unused, the category names' index, an
  unused, the variable names' index, the category names, the variable names, the curves, DSP).
  A category is 10 bytes: instance limit, fade in and out, behaviour, its parent (u16, 0xFFFF
  none) at +6, its volume (a volume byte) at +8. A curve: its variable (u16, by the names' order),
  its point count (u8), what it changes (u16: 0 the volume), then 9 bytes a point (x and y f32,
  a shape u8): a volume curve's y in hundredths of a decibel. The units' sounds follow `Distance`
  curves falling 30 dB from about 140 (a builder's loop) to 4,700 (a weapon's) of the game's
  units away.

## Reborn local records and profile metadata

Saved games and replays use Reborn's versioned record body (`RECORD_VERSION = 46`),
independent of the original game's formats above. The fixed-integer bincode body contains
the version, kind, legacy global data hash, `GameSetup`, from version 44 the duties (a byte
list of at most `ta_ai::MAX_SAVED_BYTES`, length first and checked before the bytes are read:
`[1][u32 count]` then per constructor `u32 slot, u32 generation, u8 duty`; empty when no
constructor was on duty and in every replay), ordered tick/command bundles,
final tick, timestamp and title. Versions 36–40 have the same verified body layout and
remain readable, covered by a frozen binary fixture. Unknown versions are rejected before
decoding the body; compatibility is not inferred for an arbitrary older version. Before
version 42 a Conquest battle's setup had no reinforcements, defence or perks
(`setup::LegacySetup`); before 43 the rules (`GameRules`) had no skirmish mutators
(`setup::SetupV42`, `setup::LegacyRules`); before 44 a record has no duties. Each is read in
its own layout and written again in it, so an older record keeps its bytes. The duties are
client state (what the player's constructors were sent to do, which no command says): a loaded
save puts back those still there, finished and able to work, and a record of a replay keeps none.

Format 41 requires a 128-byte compatibility trailer immediately after the body and before
any career trailer. `ta_config::records` shares its parser between the game and launcher:

| Bytes | Meaning |
| --- | --- |
| 0–7 | ASCII `TAREC001` |
| 8–11 | Little-endian nonzero simulation revision (`SIMULATION_VERSION = 40`) |
| 12–19 | Little-endian match data fingerprint |
| 20 | Full command stream flag: 0 or 1 |
| 21 | Build identity length in UTF-8 bytes, at most 96 |
| 22–117 | Build identity followed by zero padding; no control characters |
| 118–127 | Ten zero padding bytes |

All unused bytes must be zero. The full-stream flag means AI orders are included as well
as human orders; legacy streams regenerate AI decisions. New live recordings and resumed
legacy saves write the complete executed stream. Simulation revision is separate from the
file layout version: a format migration alone need not make a compatible save unresumable.
The match fingerprint includes shared rules and this battle's map/mission. The existing
global fingerprint remains the comparison for legacy records (the network compares the same
inputs without the maps, `ta_data::Compatibility::network`).
Save resumption requires matching data and a compatible simulation revision. Historical
replays with readable layouts may use approximate compatibility playback; the UI warns
that changed AI, physics or game data can change outcomes. (Version 45 has 44's layout: its
match fingerprint also covers the units a campaign mission allows.)

Format 46 names the mods the record was made with, in load order, in a trailer between the
body and the compatibility trailer, found from its end (`ta_config::records::append_mods` and
`split_mods`, shared by the game and the launcher's tail reader). Every multi-byte number is
little-endian:

| Bytes | Meaning |
| --- | --- |
| 0–7 | ASCII `TAMODS01` |
| 8–9 | Mod count, at most 1,024 (0: made without mods) |
| … | Per mod: id, name and version, each a `u16` byte length (at most 1,024) and UTF-8 without control characters; then a 16-byte content digest (all zero: its files could not be read) |
| last 12 | The trailer's whole length (`u32`, both magics included), then ASCII `TAMODS01` again |

The id is the mod's folder id in lower case, as the settings' load order names it. The digest
(`ta_vfs::mods::content_digest`, the first 16 bytes of a SHA-256) covers the files the game
mounts from the mod as installed (a folder's archives and loose files at any depth but its
`mod.toml` and links; an archive mod's archive) by lower-case path, size and content, the
folders its manifest renames, and, when it replaces any of the original game's archives, their
names (`mods::replaced_claims`: its archives named like one TA's releases install, those its
manifest's `replaces` names, those its known installation removed): another copy (an update, an
edited unit) has another, the same files anywhere the same. A mod that replaces none has the
digest it had before mods replaced archives; one that does (TA: Enhanced) has another, so a
record made with it before reads as made with other files of it. A record of 46 without the
trailer, or with one that does not parse exactly, is rejected; older records name no mods and are checked by their data hash alone. The
mods change no deterministic body, command or network layout: they explain a data mismatch
(`ta_config::records::mods::check`: a needed mod missing, off, another version or other files;
one on that the record was made without; another load order) and let the game and launcher
offer to turn the record's mods on.

Profiles add an optional 48-byte trailer at the end of either record format. It changes
no deterministic setup, command or network layout. `ta_config::profiles` shares its parser:

| Bytes | Meaning |
| --- | --- |
| 0–7 | ASCII `TAXP0001` |
| 8–39 | Match identity, exactly 32 lowercase hexadecimal ASCII digits |
| 40 | Career disqualification flag: 0 or 1 |
| 41–47 | Seven zero padding bytes |

The trailer is accepted only when every field validates. Legacy records without the
trailer remain readable. A new real match receives an identity independent of its
simulation seed; saves preserve it, and replay viewing never awards XP. Loading a
legacy/imported save establishes a career baseline at its reconstructed counters,
so only subsequent live activity can award new XP to the current profile.

Profile metadata uses TOML version 1 and career ledgers use version 5 under the device
data root. Career versions 1–4 remain readable. Each match records its XP rate: new
battles earn 25% less battle XP, while historical awards and matches already in progress
keep their original rate. Achievement rewards are preserved. Profile IDs are generated
separately from names; names contain 1–24 visible
characters. Profile metadata/selection are limited to 16 KiB, career ledgers to
32 MiB/100,000 matches, and profile count to 64. Avatar input is PNG/JPEG up to 16 MiB,
8192 pixels on either axis and 128 MiB decode allocation; stored portraits are 256²
RGBA PNGs. Legacy adoption copies only personal progress, Conquest/reconnect metadata,
portraits and save/replay files, with 8 GiB total, 256 MiB per record, 10,000 entries per
record tree and eight nested directories. Originals remain intact.
