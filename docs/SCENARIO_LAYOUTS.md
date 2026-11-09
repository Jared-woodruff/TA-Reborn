# Scenario starting layouts

The thirteen built-in scenarios use authored starting formations. Defense rows face
the threat; economy buildings use spaced grids; extractors claim separate, productive
deposits. Factories face south and reserve their assembly/exit aprons. Buildings are
placed before the garrison, and a formation that cannot fit reports a setup error.

| Scenario | Starting position |
| --- | --- |
| Last Stand | Rear solar array and metal makers, two extractors, a clear kbot lab, separate light/heavy/artillery lines along the pass and anti-air covering the base (four Defenders, two by the Commander; three Jethros on Easy). |
| King of the Hill | Working mining bases and vehicle plants on both sides, with constructors and separate assault groups. |
| Extraction | Mobile escort only; sentry pairs and powered batteries along the north bank, with patrols on the escape route. |
| Decapitation | Mining airfield with sufficient power for its metal makers; inner anti-air square, outer batteries and a powered fortress on the central island. |
| Doomsday Clock | Both bases have mining and power; the silo sits behind separate defensive lines. |
| Metal Rush | Both sides begin with two extractors, four solar collectors, a lab and their scenario-specific constructors. Expansion remains the objective. |
| Armageddon | Mobile formations only. The Core army has energy reserves for its laser kbots. |
| Endless Siege | Mining outpost, clear lab exit, radar and defenses covering the three approaches. |
| Until Dawn | Mining and powered metal makers behind defenses on the three approaches. |
| Titans | Advanced kbot/vehicle factories and an air plant, two basic and two advanced constructors, mining, fusion power and layered defenses. |
| Iron Rain | Spaced two-factory bases with mining; the two Intimidators on a mesa's terrace within range of the Arm base, lasers under its cliff, kbots on its ramp; enough generation to support the battery. |
| The Armistice | Both sides have matching mining infrastructure and labs; difficulty-specific constructors and ceasefire rules remain. |
| Brothers in Arms | All four commanders have mining outposts, labs and forward defensive pairs; computer allies/enemies have constructors. |

Wave-only enemy forces retain their scheduled arrivals and receive off-map energy
reserves for weapons that consume energy. No enemy structures are added to the
army-only battle.

## Authoring

Starting placements accept optional `formation`, `spacing` and `facing` fields:

```toml
units = [
    { unit = "ARMMEX", count = 2, at = "start 1", formation = "deposits" },
    { unit = "ARMSOLAR", count = 4, at = "start 1 + -280 0", formation = "grid", spacing = 128 },
    { unit = "ARMLLT", count = 3, at = "start 1 toward start 2 by 440", formation = "line", spacing = 160 },
]
```

`line` runs perpendicular to the direction toward `facing` (the opposing force by
default). `grid` uses map-aligned rows. Spacing is centre to centre, bounded below
by the footprint plus clearance. Terrain can move individual slots locally, while
minimum separation prevents a row collapsing into a cluster. `deposits` searches
within 64 cells of its position for usable ore: the nearest deposit worth at least
half the richest in reach (a base's own, not a richer one out in the open), and on
it the site that takes the most, keeping extractors apart. It requires an extractor
unit type.

Every slot follows the simulation's own rules. Buildings stand where a constructor
could build them, square to the grid with their yard to the south as TA builds every
building (`facing` turns a line, never a building), a cell clear of trees, rocks and
wrecks (extractors aside: they stand where the metal is), off the metal deposits (left to
the extractors, but on a metal world), and with their footprint and a factory's apron
48 px inside the map's edge.
Units stand where they can move and reach their force's start. Aircraft start parked
where an aircraft lands (dry ground with nothing on it) and keep clear of buildings,
factory aprons and other units like everything else.

Waves arrive through the simulation's `place_units_bound`: a wave's kinds keep clear
of each other and of whatever already stands there, a unit whose way to its target
no distance field finds (a target inside a building) is still placed on the target's
ground rather than on the wave's exact centre, and aircraft come in flying.

These fields are optional. Sources embedded in existing saves/replays that omit
them retain their original placement algorithm and order. Simulation, protocol and
recording layouts do not change.

## Placement review (2026-11-08)

A review of every scenario in the game found, and fixed:

- Every scenario's buildings were spawned turned toward the enemy (a solar field at
  70 degrees to the grid); they now stand square to it.
- Decapitation parked its air wing on top of the air plant and solars, and a Core
  Vengeance on the lava; aircraft now park on clear dry ground.
- Every wave stacked its unit kinds on the same ring spots, and when its target was a
  building (usually a tower: the player's nearest unit) every unit of a kind landed on
  the wave's exact centre: Armageddon's 41-unit army arrived as one blob, Endless
  Siege's late waves as 120 units on one spot. Waves now arrive spread out, apart from
  each other and from what stands there, their aircraft in flight.
- `deposits` sent extractors to the richest deposit within 1,024 px: Endless Siege's
  second extractor stood 1,000 px out toward the enemy (and drew its waves). It now takes
  the nearest worthwhile deposit. (King of the Hill's plain has a single deposit by each
  base, so each side's second extractor still stands forward, toward the hill.)
- Doomsday Clock's Core solars were squeezed against the map's bottom edge.
- Iron Rain's Intimidators, the briefing's battery "on the mesas" with "tanks on the
  ramps", stood on the open canyon floor (the map changed under the old position). The
  battery now stands on the terrace of the mesa south of the canyon's middle, both guns
  within range of the Arm base, lasers under its northern cliff and Thuds on its ramp.
- Extraction's north-bank lasers stood so far back that they reached only half way
  across the lava, against the briefing's "their laser towers reach no further [than
  the lava's edge]"; they now stand on the bank and reach its far edge.
- Until Dawn's south-east and north-east tower pairs faced east instead of the woods
  they guard.
- Buildings stood against trees whose crowns hid half of them, and on metal deposits the
  player's constructors could have mined (Titans' air plant covered one).

`every_scenario_places_its_forces_by_the_simulations_rules` holds all thirteen, on every
difficulty, to these rules: buildings buildable (by the simulation's own
`placement_error` on the bare map), square to the grid, a cell clear of features and off
the metal deposits (but extractors),
factories' exits open, extractors on metal; aircraft parked on dry, empty ground; ground
units on ground they can stand on, out of the water, not stranded, able to join up and
patrolling only where they can go;
nothing inside anything else; every wave (its biggest repeat) arriving within 900 px of
its place, nothing inside anything else, able to reach its target; goal areas reachable
by the player's army; the opening view on the player's own forces.
`scenario_layout_maps` (ignored) draws every scenario from above with its problems
numbered, and a region picture of where a tank can go: `TA_LAYOUT_DIR=<folder>`
(`TA_SCENARIO=<id>`, a custom scenario of the settings folder's too, for trying a layout
without a rebuild).

## Verification

Original-data tests cover all thirteen scenarios on Easy, Normal and Hard:

- Exact starting counts for every owner and type, including difficulty additions.
- No overlapping starts or occupied factory aprons; valid ground at factory exits.
- Productive extractors and positive idle energy surplus in every production base.
- Energy supplies for initial and scheduled energy weapons.
- Every starting factory builds and releases two units using the original scripts
  (combat disabled and extra resources/build speed isolate placement).

Portable tests cover defense spacing, a coincident facing point, separate deposits,
invalid formation fields and the roles of mobile-only and wave-only forces.
`TA_LAYOUT_REPORT=<path.csv>` on `all_scenario_starting_forces_are_complete` exports
footprints for visual inspection against the maps. Original-data tests require a
local TA installation and skip when it is unavailable.

Final checks: `cargo test --workspace --profile ci --locked` passes **2,248 tests**
(107 existing ignored). Separate Easy/Hard opening-minute runs, game all-target
Clippy and a fresh native game build pass. Clippy retains existing repository
warnings. Native captures of Last Stand, Decapitation and Titans were inspected.

Native captures and terrain/footprint contact sheets are local QA artifacts under
`target/scenario-layout-qa`. They are not included in the repository. Opening-minute
simulation checks exercise goals, waves and computer players; this is not a full
victory or balance walkthrough of every mission.

As in the earlier scenario review, the Windows `ci` capture executable is a local
copy with a 16 MiB stack reserve for startup. The shipped build configuration is
unchanged.
