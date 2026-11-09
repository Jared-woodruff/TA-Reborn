# TA Reborn's own maps

TA Reborn ships maps of its own, grown by `ta-tools mapgen` (`apps/ta_tools/src/mapgen/`) from
a seed: original land, art and layout (no tile, picture or height of TA's), the features the
game's own, named. They live in `assets/reborn/maps/` (committed: every player must have the
same maps for multiplayer), mounted over the install and under the mods
(`Vfs::mount_builtin`); the skirmish page, the launcher's map library and the editor list them
with TA's own.

```
target/release/ta-tools mapgen <id>|all assets/reborn --preview scratch/maps
cargo test --release -p ta_tools mapgen
```

`all` takes about ten minutes. The previews: `_art.png` (the map's art, a pixel for every 4 or
8), `_art_detail.png` (a stretch at full size, `MAPGEN_DETAIL=x,y` for where) beside
`_painted_detail.png` (the same stretch as painted, before the tiles were shared), and
`_layout.png` (heights, features, tracks and starts, the water in the bands that matter to what
crosses it: up to 12 deep a tank wades, to 15 a light ship sails, from 15 battleships, from
30 a shipyard stands). Submarines need 20–25 units of depth, depending on their draft.

The tests hold the committed maps to their recipes: `the_built_in_maps_are_the_generators`
makes every map's land again (the heights and the OTA, about ten seconds for all of them) and
fails on any map the generator no longer makes as committed. After changing the generator,
make the maps it names again (`mapgen <id>`) and commit them with the change. It runs where the
maps are made, on Windows x64 with the install (another maths library may round a cell
differently).

## The maps

| Map | Size | Players | World | What it is |
|---|---|---|---|---|
| Highland Rift | 32 x 32 | 10 | Green planet | A river canyon through green highlands (the menu's film) |
| Cinder Basin | 32 x 32 | 8 | Lava | Basalt terraces over rivers of fire (the menu's film) |
| Sapphire Reach | 32 x 32 | 8 | Archipelago | Wooded islands on a turquoise sea (the menu's film) |
| Mossvale | 12 x 12 | 2 | Green planet | An old wood in a mossy vale, a stream and its tarn between two clearings |
| Timberline | 20 x 20 | 6 | Green planet | Forested ridges and bare peaks over a river and its lakes |
| Greenmantle | 28 x 28 | 8 | Lush | A rainforest of rivers, lakes, reed marshes and tarns |
| Sunscar Dunes | 12 x 12 | 2 | Desert | A sea of dunes between two rock plateaus, oases on its flanks |
| Redstone Mesas | 20 x 20 | 6 | Desert | Red mesas and buttes over a dry river's canyon |
| Mirage Basin | 28 x 28 | 8 | Desert | A basin of salt pans and dunes ringed by mesas, oases along dry rivers |
| Twin Reefs | 12 x 12 | 2 | Archipelago | Two islands across a coral sea, a sand bar to wade between them, a harbour by each base |
| Crescent Keys | 20 x 20 | 6 | Archipelago | Six wooded keys in a ring round a lagoon, a key each: a map for fleets |
| Stormbreak Archipelago | 28 x 28 | 8 | Archipelago | Islands without number under rocky peaks, reefs and channels between |
| Shore to Shore Reborn | 48 x 20 | 8 | Archipelago | Two coasts across a wide ocean (TA's Shore to Shore as a map of our own): headlands and bays, coves and river mouths, island chains, a shallow bank cut by deep channels |
| Embergate | 12 x 12 | 2 | Lava | Two basalt plateaus over a river of fire, crossed by bridges of rock |
| Furnace Caldera | 20 x 20 | 6 | Lava | A volcano's crater and its molten lake, bases on the rim and round it |
| Obsidian Wastes | 28 x 28 | 8 | Lava | A continent of basalt in a sea of fire, spires and young flows |
| Foundry Deck | 12 x 12 | 2 | Metal | Decks of plating over coolant channels, ramps and bunkers |
| Reactor Grid | 20 x 20 | 6 | Metal | Armoured blocks on two levels of plating, cut by coolant canals |
| Hammerfall Works | 28 x 28 | 8 | Metal | Decks stacked four high over canals, towers and pipework |
| Rift Belt | 12 x 12 | 2 | Asteroid Belt | Two rocks adrift in the void, gantries between them, outcrops |
| Lagrange Station | 20 x 20 | 6 | Asteroid Belt | Six rocks round a station rock, gantries spoked out to them |
| Starfall Expanse | 28 x 28 | 8 | Asteroid Belt | A field of rocks adrift, a web of gantries between them |
| Frostfall Hollow | 16 x 16 | 2 | Ice | Christmas: a snowbound valley, a frozen river and pond, two villages round their great firs |
| Hollyridge | 20 x 20 | 4 | Ice | Christmas: bases on snowy ridges, a frozen river through the vale, villages along its banks |
| Midnight Square | 16 x 16 | 2 | Ice | New Year: a town's clock tower in the square between the bases, fireworks round it |
| First Light Peaks | 24 x 24 | 6 | Ice | New Year: mountain villages with clock towers and fireworks over three frozen lakes |
| Australia | 28 x 24 | 8 | Earth | The island continent: the red centre's deserts and salt lakes, the tropical north, the Great Dividing Range, bases at the cities round the coasts |
| United States | 32 x 18 | 8 | Earth | Sea to sea: the Rockies and the Sierra, the south-west's deserts, the Great Plains, the Great Lakes, the eastern forests |
| The World | 40 x 20 | 10 | Earth | The world from the Arctic's ice to the Southern Ocean, a base on every inhabited continent |

The first three are made as they were first made (the `Classic` painter, no symmetry, no
tracks): their land and art come out byte for byte the same (the menu's film, the benchmark and
the trailers are staged on them), their features as the maps' QA left them (the same deposit kinds
by every base, none where no extractor fits, the trees kept off the ground a geothermal plant
stands on round each vent); the rest are made with what follows, the last three from rasters of
real places ([Real places](#real-places)).

## How a map is made

1. **The land** (`terrain.rs`): a smooth elevation, warped and ranked so tiers take their
   shares, joined by cliffs or ramps, shaped by guides (bumps raising or sinking parts of the
   map: islands where they are wanted, a lagoon) and rings (a caldera's rim, an atoll); hills,
   ripples and ridged peaks; dunes on the lowest tier; a river with lakes and fords, or a dry
   river bed, or an ocean; shoals (a sand bar a tank wades) and bridges (rock over a river of
   fire); pools in the hollows. Metal worlds are built instead (`decks.rs`): a grid of blocks,
   each a deck at one of a few levels (a smooth noise ranked into them, through the symmetry),
   canals of coolant where a ridged noise runs, vertical walls between decks and straight ramps
   across some of the borders a level apart, armoured bunkers on the decks; their starts are not
   levelled (the decks are). Worlds in space (`space`): the land below the ocean's share is void
   (TA's void cells, `0xFFFC`: impassable to all but aircraft), craters pocked into the rocks,
   their rims falling away to it; gantries laid across it (a profile from the rock at one end to
   the rock at the other, eased as the ramps are, the roads over the rock to them graded in and
   kept clear), and more where a rock is cut off from the first start's. Winter worlds
   (`frozen`): the water frozen once the starts and ramps are laid round it, level ice where it
   was, a unit above its height (`Land::ice`; the map then has no sea, and every unit crosses the
   ice). Real places are grown from rasters of their land, relief and biomes (`geo.rs`, below);
   two coasts across an ocean are laid out (`coasts.rs`, [Two coasts](#two-coasts-across-an-ocean)).
   Duels and team maps are **point-symmetric**: each half is the other turned round the middle (`Sym`;
   noise read through it comes out symmetric, a band between the halves blending the two), the
   starts placed in pairs facing each other (StartPos1 and 2, 3 and 4...), every feature placed in
   the first half and its twin turned into the second.
2. **Starts** on level ground with room round it (most of the ground within about 20 cells
   gentle and dry: an AI on a butte's top starves), dunes eased away round each base.
3. **Every start drivable** (land maps): where a tank (3 x 3, slopes up to 10, water up to 10
   deep: under TA's TANKSH3 and KBOTSF2 limits, judged on the heights as stored, whole units)
   cannot get from a start to the first, the cheapest way through is cut into a ramp
   (`connect`), and its twin; across water, where the recipe allows it, raised into a causeway
   (a sand bar, a bridge of basalt); across the void, a gantry. On the maps whose kit asks for it
   (`corridor`) the way a tank drives between the bases is kept clear of features, twins and all.
   After the features are placed the check runs again round the trees and rocks, and the map is
   not written if it fails (`MAPGEN_NOCHECK=1` writes it all the same, to look at, and names
   what stands in the way). Island maps without a causeway are for fleets, hovercraft and the
   air.
   **Harbours** (`terrain::harbours`, the maps for fleets and the real places, `Shape::harbours`
   the farthest a base may be from the sea): the sea shelves gently, 30 deep (a shipyard's 8 x 8
   needs it under all of it) only some 35 cells out, which put the yards 80 to 140 cells from
   their bases and beyond the computer players' search (56 cells, in rings round the base). At the
   shore the walk from each base reaches first (a cell of land costing four of water), the low
   beach is dug out into a dock (10 cells round, up to 10 over the sea) and a berth dredged 34
   deep, long from north to south (a factory's units leave by its south side, and on a symmetric
   map a berth's twin is turned half round: the yard and its ships' way out of its front fit in
   it either way), its edge wandering, its bed rising steeply to the beach (30 deep two cells out:
   a builder there reaches the yard); from the shore a channel 30 deep and 16 across runs out to
   the open sea (the stretches 18 deep and more, a quarter of the biggest at the least): every
   ship's way to the others' yards. Where the berth is laid is tried first: a yard must stand in it
   as the game has it (30 deep, 3 cells off the map's edge, a battleship's way out of its front),
   in its twin too, else further out along the channel. Only water and the dock are dredged, never
   within 10 cells of the ways the tanks drive between the bases (the dredging easing in over 8
   more), and no channel longer than 80 cells (a trench across a gulf). On fleet maps without
   land bridges, the sea bed falls twice as fast beyond the beach so that the 25-unit draft of
   the deepest submarine can cross the sills between the islands. Crescent Keys and Stormbreak
   Archipelago include these deeper seas; their starts and land topology stay the same.
   The channel ends where the open sea is first 22 deep, enough for every ship but the
   submarines (CORSSUB's 3 x 3 draws 25, the 4 x 4 ones 20): where a harbour's water does not
   reach the heart of that sea for each of them (its largest stretch of their water), the
   cheapest way over the water from the harbour to there is dredged as the channel is, where the
   water round it is under 27 deep, never across the tanks' ways and over no sill longer than 80
   cells (`ways_on`); a harbour whose submarines already get out is left as it is. On Twin Reefs,
   whose sea bed falls gently, the channels ended on a sill along the map's edge and the
   submarines never left their harbours; on The World one of start 3's. The checks hold every map
   with harbours to it: from each base's yard each submarine reaches at least half of the water
   it sails in the sea that yard's battleships sail.
4. **Tracks** (`tracks.rs`): the cheapest ways (slope, water, forest, a wander) from each base to
   the middle, to its neighbour and to the nearest ford, later tracks joining the first, smoothed;
   painted, and kept clear of features so they are passages through the woods.
5. **What stands on the land** (`place.rs`): metal by every base (six on the new maps, turned
   out of a track's way, the same kinds by every base: deposits hold 56 to 223 a cell; on a
   symmetric map each base's turned round into its twin's) and in fields, every deposit where an
   extractor fits over it (dry, no steeper than 8, as the game's 10 with a little to spare); vents,
   the ground a geothermal plant stands on (4 x 4) kept clear round each, level and dry on the new
   maps; forests with glades and a clearing round each base, rocks on the broken ground,
   landmarks, ruins, palm groves round oases; in the sea, reefs of coral and anemones (passable)
   and metal for the fleets, 21 deep and more (ARM's underwater extractor stands 19 deep, CORE's
   10; in the shallows neither could). On a
   winter world, villages (`Kit::village`): the best places on a grid (the most level ground a
   dozen cells from a road, far from the bases, the water and each other, their squares clear of
   metal), each a square with its centrepiece (a great fir, a clock tower), two rings of
   cottages with gaps for the lanes to the road, lanterns round the square and along the roads
   near it, snowmen, presents, ice sculptures, a sleigh, fireworks and strings of lanterns for
   the New Year, decorated firs where the forest opens round it. A middle village's centrepiece
   stands on the line between the halves, its own twin.
6. **The art** (`paint.rs`, `paint/`): painted in TA's oblique view, by one of the painters:
   `wild` (temperate and tropical land, coral reefs in the shallows), `desert`, `volcanic`
   (basalt jointed in polygons, ash, cinder, young flows, scorched and glowing shores, sulphur
   round the vents; the melt painted bright at the shore and deep red out, as the game reads
   lava, which draws its own crust over it), `metal` (a patchwork of plates of 128, 64 and 32
   pixels on a grid 16 pixels off the tiles', so their seams never draw the tile grid: plain,
   tread, grates, hatches, vents, conduits, lamps; each deck its own steel; hazard stripes on
   the lips, tread on the ramps, grime and rust, armoured bunkers, ribbed walls, landing pads
   at the bases), `space` (grey regolith in regions, pitted, strewn with stones, the craters'
   bowls in shade and rims bright, boulders, dark rock faces at the rims, the roads packed dark,
   plating on the gantries and round each base, the void painted as the stars for the classic
   view and the minimap), `snow` (snow rippled across the wind in powder and crust of the one
   colour, glints, blue in its shade bands, rock breaking through on the steep slopes with snow on
   its ledges, needles fallen in the woods, the ice pale and swept dark in broad stretches,
   cracked, snow streaked over it and heaped at its shore, roads of packed snow with ruts of
   slush, the villages' squares cobbled, bare ground round the vents), `earth` (a real place's
   grounds, each by the painter of its biome, below), and the first maps' `Classic`.
7. **Tiles** (`tiles.rs`): the art cut into 32-pixel tiles, alike ones shared to fit the budget,
   each place then picking the shared tile that meets its neighbours as the painting does, the
   tiles' edges settled, dithered into TA's palette.

## Painting for shared tiles

A map's art is many times its tile budget (24,000 tiles over 1024 x 1024 cells, the same detail
per area on every map, 4,000 at the least), so most tiles are shared by many places. What
survives the sharing:

- **The fine texture repeats.** Ripples, tufts, stones, grain are laid in the art's own rows and
  repeat every 256 pixels (`noise::PERIOD`, `tvalue`, `tworley`, `twave`...): a stretch of one
  ground is then made of 64 tiles that meet seamlessly whichever way they are laid, as TA's own
  ground tiles are. Plain tiles share only with tiles of their own phase in the repeat (the
  `phase` of `tiles::Desc`), so their grain carries on across their edges; tiles with more to
  them (an edge, a crest, a track) share with any alike tile.
- **Grounds are distinct, not gradients.** Meadow, straw, heath, sand regions: each its own
  colour through, their edges short and wandering. A gradient across many tiles is cut into
  squares by the sharing.
- **Shading in bands** (`Ground::shade_bands`): lit, level and shaded, with short edges between;
  the game lights the relief itself.
- **Lines kept whole**: tiles of rock in strata and tiles a track crosses keep a tile of their
  own, up to a share of the budget (`Recipe::whole_rock`).

`tiles::blockiness` measures how much the tiles step at their edges against inside them; a test
holds every committed map to 1.33 (TA's own measure 0.92 to 1.12). A duel with much shallow
water (Twin Reefs) gets a budget of its own, 5,000; the winter maps half as much again as their
size's (`winter_budget`): snow is smooth, and a tile shared where another belonged shows on it.
On snow, too, every edge between two colours costs a tile at every step: the woods' floor is the
open snow's colour (their needles and the trees' shadows say where they are), and the crust the
powder's.

A change to the generator must leave the committed maps as they are, or remake them in the same
change: gate new behaviour behind a recipe's settings, and check with `mapgen` into a scratch
folder that the committed maps come out byte for byte.

## Real places

Australia, the United States and the world are grown from rasters of each place, a pixel a
cell (`apps/ta_tools/geo/<map>_<what>.png`), drawn by `tools/geo/outlines.py`:

- **land** and **lakes**: Natural Earth's outlines (`ne_10m_land`, `ne_50m_lakes`, in the public
  domain), projected over the map's bounds (Australia and the USA equirectangular, the world in
  Miller's projection from 60 degrees south to 84 north) and drawn supersampled, so a coast falls
  between the cells where it runs;
- **relief**, 0 to 1: the land rising from its coasts, and the mountain ranges, plateaus and
  basins written out in the script (where each runs, how high);
- **biome**: how much of each cell is forest, grassland, desert and cold (RGBA), and
  **warmth**, 0 polar to 1 tropical: from the latitude and the regions written out there.

```
python -I tools/geo/outlines.py <folder holding ne_10m_land.shp and ne_50m_lakes.shp> [--only australia,usa,world] [--preview scratch/geo]
oxipng -o 3 --strip safe apps/ta_tools/geo/*.png
```

(The two are naturalearthdata.com's 1:10m Land and 1:50m Lakes and Reservoirs, among its
physical vectors; they stay out of the repository.) The generator (`geo.rs`) cuts the relief
into the recipe's tiers along its own contours (`steps`, the relief each tier rises at;
`levels`, their heights): cliffs, or long slopes where a ramp field says, with hills, ripples
and ridged peaks over them. The land slopes down to the sea at the coasts; under the sea a shelf
falls away to the deep, the lakes shallower. Australia's lakes are dry (`dry_lakes`): salt pans,
each level a little below its shore. The starts are cities (Perth, Brisbane and Darwin; Seattle,
Miami and New York; Washington, Beijing, Paris, Sydney and others round the world), each moved to
the nearest level ground well in from the water, and on Australia and the USA every start is
made drivable as on the other maps; the world's continents are for fleets and the air. The
places are as they are, not symmetric.

They are painted by the `earth` painter (`paint/earth.rs`): each place by the painter of its
biome, forest by the wild painter's woods (temperate, tropical or the north's dark conifers by
how warm it is), grassland by its prairie or savanna, desert by the desert painter's (hot or
cold, without the green an oasis has round its water, `Desert::oases`: the deserts run down to
the sea as they are), the polar lands by its tundra or the winter painter's ice. The two strongest biomes meet
over a short, wandering edge, as the grounds do on every map. Snow lies on the peaks above the
recipe's snow line (lower where it is cold), salt crusts the dry lakes, the shallows are
turquoise in the tropics (the deep is one blue everywhere: two would meet in a line across the
sea). Their woods' floors are crisp (`Wild::crisp`: a short edge, no fringe of rank
grass round them), which the tiles share better. The forests grow where the biomes have them
(`geo::forest_share`), and what is placed from the kit's (temperate) lists is given its biome's
own of the same footprint (`place::dress`, `EarthKit`): tropical trees in the tropics, the
north's conifers in the cold, palms in the desert, the desert's and the cold's rocks, metal and
vents. In the game their planet, `Earth`, has the green worlds' look and showers, and the
detail materials of every ground, from sand to snow (`terrain_detail::world_weights`).

Natural Earth asks for no credit; the credits roll gives it all the same, as this page does:
Made with Natural Earth. Free vector and raster map data @ naturalearthdata.com.

## Two coasts across an ocean

Shore to Shore Reborn takes the idea of TA's Shore to Shore (29 x 4: two coasts, a wide ocean
between, a few sandy islands) and none of its land: its own is laid out by `coasts.rs` from the
recipe's `Coasts`, point-symmetric (the west coast is laid out, the east is its twin read from
the south; what lies about the middle is taken with its twin):

- **The coast**: a line wandering north to south, and along it headlands (ridges of the uplands
  reaching out to sea, cliffs round their ends and sea stacks off them), bays (their shores
  sand, the landing beaches), coves (deep basins just inside the coast, the shelf narrow at their
  doors: each base's harbour within a short walk), rivers (wadeable valleys from the highlands,
  sandy at their mouths, a ford where the coast road crosses each) and a lagoon behind its spit.
  Every shape is read through a warp, so none comes out as drawn with compasses.
- **The land**: a coastal plain with knolls, a step up to the uplands and another to the
  highlands and their ridged peaks, each a cliff or a long ramp (as the grown maps' steps are),
  wandering far either way so that some face south, where TA's view sees them.
- **The sea** in depths that read (the art paints it in bands, `Wild::bands`: the shelf's
  turquoise, the deep's blue, the abyss's navy): a shelf off each coast, wide in the bays and
  narrow off the headlands, sandbanks and gullies over it; its slope down to the abyss (64 deep);
  islands with reef flats of their own, the great one in the middle with its peak; seamounts
  rising to 22 deep, volcanoes' cones under the sea (`seamount`: long along an axis of their
  own, lobed along three rift zones, a flank cone or two, gullies down their sides; round, they
  read as discs drawn with compasses); and **the bank**, 13.5 to 18.5 deep
  from coast to coast through the middle island: no tank or kbot wades it (12), no submarine
  (20) or battleship (30) sails it, light ships and hovercraft cross it where they please, and
  the fleets cross it by its two channels, 40 deep (Bulldogs and Sumos, wading 23, walk it as
  far as a channel; Commanders and amphibians cross anywhere, as on every sea of TA's).
- **The bases**: four on each coast, on the plain a little in from the shore, each with its
  bay's beach in front and a cove beside it (`harbours` finds the berth there, the cove's canyon
  runs on across the shelf, and `ways_out` deepens the cheapest way on from the berth to the
  abyss to 32 where it is shallower: the harbours' channels are judged by an open sea 18 to 22
  deep, which left a sill under a submarine's 25 at one base); a tank drives from every base on
  a coast to every other there (ramps cut where needed, and `mapgen` checks it round the
  features), never across the ocean. StartPos1 and 2 face each other across the middle of the
  map; 3 and 4 at the north and south ends, then the rest.
- **The islands** carry the richest deposits (`Kit::isle_metal`, one for every 700 cells of
  island) and the great ones a vent (`Kit::isle_vent`): what the fleets and the hovercraft fight
  over. The islands across the middle are wooded right through (`Kit::tight`: features up to the
  line between the halves, every cell of a footprint on its own side, where the other maps keep
  a footprint and a cell off it).

The size is the generator's and the game's to choose within: the renderer draws ground of up to
`max_texture_dimension_2d` cells a side (16384 asked for, some cards 8192) and holds the tile
atlas in rows of 128 tiles under the same limit, the generator keeps every map to 24,000 tiles,
and the largest maps before it were 32 x 32 (1024 x 1024 cells) and The World (1280 x 640).
48 x 20 (1536 x 640 cells, 983,040 of them, about a 32 x 32's) keeps to what is known to load
and play (about 14 s here), with the most tiles a map may have, 24,000 (its coral shelves and
its bank need more than land of its size: 22 MB), and is 1.6 times as long as TA's original and
4.8 times as wide from north to south: four bases a coast about 150 cells apart, an ocean about
950 cells across.

## The festive features

The winter maps' snowy firs, decorated firs, the great fir, snowmen, presents, lanterns, ice
sculptures, cottages, the sleigh, the clock tower, fireworks, strings of lanterns and snowy
boulders are TA Reborn's own (original art), in `assets/reborn/features/festive/festive.tdf` and
`assets/reborn/anims/festive.gaf`. Each is a model of `tools/features/festive.py` (in
`features.glb`, drawn up close in the perspective view in its own colours, see
docs/ARCHITECTURE.md) and its painting, drawn from that model in TA's view by
`tools/features/sprites.py` (Blender: looking north, down at 63.4 degrees, lit from the south
west, a shadow cast to the north east; a decorated fir's 16 frames as its lights twinkle), then
packed into TA's palette by `ta-tools gaf-pack`:

```
python tools/features/build.py
"C:/Program Files/Blender Foundation/Blender 5.2/blender.exe" -b --factory-startup -P tools/features/sprites.py -- --out scratch/festive [--only cottage_a] [--preview]
target/release/ta-tools gaf-pack scratch/festive/manifest.json assets/reborn/anims/festive.gaf
cp scratch/festive/festive.tdf assets/reborn/features/festive/festive.tdf
```

A feature definition's `rebornmodel` names its model, `rebornlight` the light it casts by night
(colour, reach, height) and `rebornfireworks` sets off fireworks by night (docs/FORMATS.md).
Their categories are `festive`, their worlds `Ice` (snowy ones) or `All Worlds`.

## Checking them as a player would

`cargo test --release -p ta_tools mapgen` holds every committed map to the game's own rules
(`mapgen/qa.rs`, with the install: its movement classes, building placement, metal and vents):
on a land map a tank and a kbot drive from every base to every other round the features (on
the two coasts', from every base to every other on its coast); each
side can mine every deposit (on land or with its underwater extractor) and build a geothermal plant
on every vent (the first maps keep one or two too steep, as first made); every base has its
deposits, the same metal as every other (within 15%), a lab's room within 12 cells and a vehicle
plant's on half the ground within 20; the tides the menus show can be put to use (the winter
maps' water is frozen and the metal maps' coolant 18 deep, a tidal generator needs 20: their tides
are 0, Hammerfall Works' canals take them); on the maps for fleets a shipyard site lies within the
computer players' search of every base (found as they look: the yard fits, 3 cells off the edge,
its biggest ship can sail straight out of its front, the Commander can walk within building reach)
and, where the seas are one, every ship sails from every yard to every other (Twin Reefs' sand bar
parts the subs' and battleships' seas, as it is meant to). The computer players find a yard site
by every base on these maps now, but still build hovercraft there rather than ships (their own
rules: a hovercraft plant counts as reaching the enemy by land, and warships only reach bases
within 30 cells of the sea). The report: `cargo test --release -p ta_tools maps_as_played -- --ignored --nocapture`
(`TA_MAPS=a,b`, TA's own maps too, to compare with; `TA_REBORN_ASSETS=<folder>` for maps made into
a scratch folder, its `features/` and `anims/` copied from `assets/reborn`).

## Playing them

`cargo test --release -p ta_ai --test skirmish -- --nocapture` with `TA_TEST_MAP=<name>`,
`TA_TEST_PLAYERS=<n>`, `TA_TEST_MINUTES=20` (`TA_TEST_TEAMS=1` for team maps: the
starts' pairs then face each other). On a six- or eight-player map played free-for-all, the
starts in the middle are pressed from every side, and the test's "8 metal a second by minute
10" can fail for one of them; as teams they hold.

## Adding a map

A recipe in `recipes.rs` (its id in `IDS`, a name of its own: not one of TA's), made and looked
at in the game (`--map "<name>"`, `TA_REBORN_ASSETS=<folder>` for a scratch folder holding
`maps/`), played by the AI, then made into `assets/reborn` and committed.
