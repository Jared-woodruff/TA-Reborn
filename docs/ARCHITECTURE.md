# TA Reborn — architecture

A clean-room engine that plays Total Annihilation from the user's original game files
(the Commander Pack, from GOG or Steam). No original code or assets are shipped.

```
apps/
  ta_game       ta-reborn.exe     — window, input, HUD, client-side presentation
  ta_launcher   ta-launcher.exe   — install detection, mods, settings, play
  ta_tools      ta-tools.exe      — inspect / convert / validate game data
crates/
  ta_formats    parsers: HPI, TDF, GAF, 3DO, COB, TNT, PCX, palette; TNT and TDF writers
  ta_vfs        layered virtual file system, GOG/Steam install discovery, mod ordering
  ta_config     user settings (%APPDATA%\TA Reborn\config.toml; the folder under the name up
                to 1.9, TA Remastered, moves there when either program starts: adopt_old_folder)
  ta_skins      bounded cosmetic skin packs, creator folders and deterministic import/export
  ta_data       typed game data: units, weapons, features, move classes, sides, map info
  ta_sim        deterministic simulation (fixed point, COB VM, pathfinding, combat, economy)
  ta_ai         deterministic skirmish AI (emits the same commands as a human)
  ta_net        lockstep multiplayer: TCP transport, lobby protocol, tick bundles, desync checks
  ta_render     wgpu renderer (terrain, water, units, sprites, beams, lights, shadows, fog of war, HDR/bloom)
  ta_audio      audio engine (own mixer on cpal, symphonia MP3, spatial, music moods)
  ta_video      Smacker (.ZRB) cutscene decoder
  ta_mp4        MP4 films (H.264 and AAC) through Windows' Media Foundation: replays' export
  ta_ui         the shared look: palette, fonts, egui style, custom widgets, panel chrome
installer/      NSIS script + build scripts (requires the GOG install at setup); `licenses/`
                standard licence texts for the third-party notices
branding/       TA Reborn's own art, drawn from code: the emblem by
                `tools/branding/emblem.py` (`emblem.svg` and a PNG per size, a cut of its own
                up to 32 px; the window icons and the launcher's emblem, `ta_ui::icon`), the
                icon and the installer's pictures from it by `ta-tools brand`; embedded in both
                executables by their build scripts (`windows_resources.rs`; the commit a
                build is made from comes from `apps/build_info.rs`)
assets/reborn/  TA Reborn's own maps (`maps/*.tnt, *.ota`), grown by `ta-tools mapgen`
                (`apps/ta_tools/src/mapgen`, docs/MAPS.md): land, water and placement from
                seeded noise (duels and team maps point-symmetric, every base drivable, ramps
                cut where cliffs wall one off), or for the real places from rasters of their
                land, relief and biomes (`apps/ta_tools/geo/`, drawn by `tools/geo/outlines.py`
                from Natural Earth's public-domain outlines; `mapgen/geo.rs`, the painter
                `paint/earth.rs`), tracks, art painted in TA's oblique view with its
                fine texture repeating tile for tile, cut into tiles with alike ones shared to
                fit the tile budget, dithered into TA's palette; only the game's own features are
                named. Mounted over the install and under the mods (`Vfs::mount_builtin`, found
                beside the program or above a build folder, or `TA_REBORN_ASSETS`); the menu's
                battle uses the first three
docs/           this file, FORMATS.md, ROADMAP.md; media/ the README's banners and captures
                (filmed from the game: `--screenshot` strips with `TA_CAPTURE_FPS`)
README.md, CHANGELOG.md, LICENSE (GPL-3.0-or-later)
```

## Local player profiles

`ta_config::profiles` owns local identities and career XP. `config_dir()` remains the
shared device content root (mods, skins, cache and logs); `player_data_dir()` and
`config_path()` resolve the profile pinned at startup. The launcher passes that identity
to its child game. Another launcher's selection never redirects an already running
match. Names are display text, not paths; profile directories use validated 32-digit
hex identities. Unknown explicit identities fail instead of selecting another player.

Each `profiles/<id>/` holds `profile.toml` (the name, allegiance, motto and showcase: a
motto or showcase this version cannot show is left out on reading, never the profile), a
full `config.toml`, `portrait.png`, campaign
and Conquest progress, saves/replays, and `career.toml`. The first profile copies legacy
personal files without deleting their originals. Later profiles start with clean
histories. Photos and screenshots default to `Pictures/TA Reborn/profiles/<id>`;
existing pictures remain in their legacy folder. The explicit `TA_PICTURES` capture
location is used unchanged. Shared libraries are never copied. Avatar imports have decode/memory bounds
and become cropped 256-pixel PNGs; linked profile/adoption paths are rejected.

A standard OS file lock serializes profile mutations between launcher and game. Career
XP and match high-water counters are one atomic, bounded ledger, so retries, restart,
reloading an old save and concurrent checkpoints do not duplicate awards. Imported
saves establish a historical baseline before live play resumes. Concluded matches
cannot earn further XP from alternate endings. The gameplay client excludes replays,
benchmarks, attract scenes and demonstrations from awards. A battle with cheats in it is
voided (`ProfileStore::void_match`, as the first cheat is used, once a session): its entry
takes back what it gave the career (its XP and totals; its records, recounted from the battles
left; the missions and achievements it alone gave, from what `MatchEntry::gave` noted at its
checkpoints) and refuses further awards, from any copy of its saves; the history leaves it out.
Nothing in the deterministic simulation depends on profile IDs, wall-clock dates or rank.

XP rewards completed units/buildings, kills, captures, produced resources, play time,
completed matches and wins. Cumulative score division preserves progress between
checkpoints. Rank thresholds are `250 * (level - 1) * level`, with levels 1 through 100,
counted on `Career::total_xp`: the battles' XP and the achievements' rewards by tier
(`Tier::reward`, worked out from the unlocks, so nothing more is stored). A title every five
levels (`RANKS`, 21 with an insignia each, `crates/ta_ui/badges/ranks.png`), and past level
100 a service star for each `STAR_XP`. `MatchCounters::xp_breakdown` splits a match's XP by
source for its result's page (`ta_ui::profiles::BattleRecord`).
Counters saturate; one match awards at most 1,000,000 XP. Match identities are never
evicted to make room for new ones (which would make an old save eligible again); a
full or invalid ledger reports an error and preserves the existing career.

Achievements (`ta_config::achievements::ALL`, docs/ACHIEVEMENTS.md) are conditions on the
career: totals, per-mode totals, campaign missions won, a battle's records (`feats`, kept
only for matches tracked from their start), event tallies and the maps and worlds won on.
The game's `profile_progress` counts the tallies from simulation events (enemies destroyed
by class and by the Commander, missiles launched, units carried, features reclaimed, units
built by class) and the battle's peak incomes and army; they score no XP. Each checkpoint
unlocks what the career meets and records where (`Career::unlocks`); between checkpoints
the game asks `Career::with_progress` once a second what the next would unlock, and saves at
once when something is earned. It then draws the battle without its interface at the
window's size (`Renderer::render_image`, the scene's own targets) and keeps it as a JPEG in
the profile's `achievements/` (`ProfileStore::save_achievement_picture`, written on a
worker). `refresh_achievements` unlocks what earlier play met (the launcher and the game
call it on opening a profile); a ledger from before the redone list (version 3) loses the
retired identifiers only. `ta_ui::badges` draws the badges from one atlas, greyed when not
earned; `ta_ui::profiles::CareerNotices` the notices; the dossier's list and pages are
`ta_ui::profiles::AchievementsView`.

The launcher's Profile page (`apps/ta_launcher/src/app/profile_page.rs`) reads the record
(`CareerData`, the campaigns from the mounted game) and every commander's career on a
worker, and draws `ta_ui::profiles::hero` (the head, sized by `HeroSize`) over the
`CareerView`'s tabs (`overview`, the achievements, the campaigns, the history) and its own
Commanders tab. The game's dossier (`CareerWindow`) draws the same head and tabs.
`ProfileStore::release` lets go of a profile about to be deleted; the launcher moves its
folder to the Recycle Bin. `ta_ui::i18n::grouped` groups a number's digits as the language
in use writes them.

## Data flow

```
GOG/Steam install ──ta_vfs──► bytes ──ta_formats──► parsed files ──ta_data──► GameData
                                                                         │
                          UnitTypes / WeaponTypes (fixed point, per tick)◄┘
                                          │
   input/AI ──Commands──► ta_sim::Sim::step (30 Hz, deterministic) ──► SimEvents
                                          │                                 │
                               state (units, projectiles)            effects, audio
                                          │
                         apps/ta_game::scene (interpolation) ──► ta_render
```

## Simulation (`ta_sim`)

* **Determinism**: fixed point with TA's 16 fractional bits in an `i64` (`Fx`, 48.16;
  products and quotients through `i128`, saturating at `Fx::MAX`/`Fx::MIN` instead of
  wrapping, sums at the `i64` range), table trigonometry generated with pure IEEE
  arithmetic, integer square roots, PCG RNG, slot-ordered iteration. Two runs with the same
  inputs are bit-identical (tested). Wall-clock timing is only used for `SimStats`
  diagnostics.
* **Room to grow**: game data is read as `f64`/`i64` (`ta_data`), so a mod's big numbers
  keep their precision; up to 64 players (`unit::MAX_PLAYERS`; per-player masks such as
  cloak reveals, the unit grid's owners and enemies are `u64`); 256 distinct category tokens
  (`defs::CategorySet`; stock TA uses 54, more log a warning); counters (kills, builds,
  losses, deaths) in `u64`, stockpiles in `u32`, metal per cell in `u32`, starting
  resources in `u64`; movement regions labelled in `u32`; maps up to 16384 cells a side.
  Values handed to COB scripts saturate at the `i32` range instead of wrapping (packed XZ
  keeps TA's own 16-bit fields that wrap, see FORMATS.md).
* **Tick order**: commands → economy begin (stall factors) → unit grid → vision rebuild
  (every 6 ticks) → shared distance fields due brought up to date (on all cores, each its own
  work) → per unit: orders, factory, flight (a ground unit's way to a place is asked for, not
  planned) → the ways asked for, planned together (see Pathfinding) → formations' pace
  (`Sim::pace_formations`, see Formations) → with the rule "units
  give way", each mover's steer round the units ahead of it (`avoid.rs`: from the tick's unit
  grid and a snapshot of every unit's place and motion, the time and distance of their
  nearest approach; half the movers each tick, by slot; not round those it already touches,
  which are the crowd's to push apart) → ground, hover and naval units'
  movement, all at once (`ground_step` changes only the unit it moves; on all cores in big
  battles, identical to one core) and new routes for those that got stuck → per unit:
  weapons, cloak, air pads → every unit's COB animate+run at once (`Sim::run_scripts`, on all
  cores in big battles: a script changes only its own unit and reads the others from a
  snapshot taken before; what it asks of the sim (events, attach/drop) is applied after, in
  slot order) → collision separation (`crowd.rs`: each unit pushed by the mean of its overlaps
  with the others, so a packed crowd settles; summed, it went to and fro every tick) →
  projectiles → deaths → defeat check → economy end →
  removal. Parallel phases give exactly what one core gives (`parallel` off proves it:
  `tests/stress.rs`).
* **COB VM** (`cob.rs`): cooperative threads, signals, sleeps, wait-for-turn/move, query
  functions via parameters. `host.rs` answers `get`/`set` (PIECE_XZ, BUILD_PERCENT_LEFT,
  INBUILDSTANCE, YARD_OPEN ...).
* **World** (`world.rs`): 16 px cells, slope = corner height range, per-movement-spec
  passability bitsets, **connected regions** (`u32` labels: unreachable goals resolve
  instantly to the nearest reachable cell), building occupancy, metal map.
* **Pathfinding** (`path.rs`): A* on anchor cells, stamped scratch buffers, deepest-first
  tie breaking, line-of-sight smoothing (a shortcut needs every cell the straight segment
  passes over: units steer along it from wherever they stand). Goals inside buildings are
  retargeted; a goal in a pocket walled in by trees, rocks or buildings (fewer than 256 open
  cells round it) is aimed at from the nearest open cell outside (`path::open_goal`). Units
  share **flow fields** (bucket-queue Dijkstra over the movement spec, covering a window
  around the goal and the units asking, cached for 30 s) and read their routes off them by
  steepest descent (never cutting a blocked corner), smoothed against the field's own record
  of which cells were open; a unit standing where its field doesn't reach steps out to the
  nearest cell it does (4 cells off at most). A field is **brought up to date** with new
  buildings and wrecks instead of built again (`FlowField::repair`, from the world's log of
  occupancy changes: the cells that lost their distance and those that opened are found
  again by Dijkstra from their neighbours; exactly what a new build gives, which a test
  checks), at most every 8 ticks, before the units' orders (on all cores). Ground units'
  ways to a place are **planned together** after every unit has had its orders
  (`Sim::plan_ways`): those of a movement class going to places within 160 cells of each
  other form a group; 12 or more share a field (one kept that covers them, else one to the
  goal nearest the middle of theirs; 12 built a tick at most, on all cores, the groups beyond
  wait a tick), and each leaves the way to it for its own place: at the first cell its goal
  is in a clear line from (looking among the cells near it), else where the way down from
  its own goal joins (`FlowField::route_to`). The routes are read on all cores. A field that
  misses a good part of its group is made again over a wider window; a unit it doesn't reach
  is cut off where it stands, and searches a little way for the nearest point it can get to
  (and plans alone while sent there, `Sim::alone`). A goal the field doesn't reach (a
  clearing walled in) is aimed at from the nearest cell it does. Smaller groups plan each
  unit alone: the field to its goal if one is kept, a path cached for its 4×4 block of
  starts (serving another cell of it only if its first waypoint is in a clear line from
  there), else A* within the tick's allowance. A goal in the unit's own region that A* gives
  up on (a long detour round a ridge from its foot floods more than its allowance) gets a
  field over a wide window, then the whole map's, one build a tick. A waypoint counts as
  passed only once the next leg is open from where the unit stands (in reach of it but a
  cell short, the line on can graze a tree the route goes round); one the unit has come past
  close by (on the far side of it from the next leg's point of view: a tank whose turning
  circle is too wide to come within reach) is passed too, rather than come round for. A unit
  sliding along an obstacle without getting closer to its waypoint counts as stuck (turning
  on the spot never does), is set back on its cell's middle and plans again from there. In a
  crowd every step succeeds and the crowd pushes the unit back, so a unit counts the ticks
  the crowd has kept it from coming nearer where it is heading (`Unit::approach_*`): on the
  last leg of its route, after a second within twelve times its radius of its goal, it has
  arrived (the place is held: a rally point, a spot many were sent to); on the way, after a
  second short of its next waypoint (others standing on it), it goes on to the one after if
  the way there is open; held up head on for a second and a half by its own side's units
  coming the other way (two meeting in a gap one unit wide, or square on in the open), or
  standing idle in its way and not fighting (an army gathered in a pass), it slips past them
  for three seconds (`Unit::squeeze_until`: the crowd doesn't push such a pair apart). A new
  order (not queued) starts afresh: the last one's way (`Unit::move_request`) is forgotten,
  or an order given again to the same place counted as already as near as it gets.
* **Formations** (`formation.rs`; the rule "modern movement", `Sim::group_formations`, with
  units giving way): a group order to a point (`Command::Order` with several units, or
  `Command::Formation` with a shape: rows facing a given way, the shape the units stand in, the
  points of a line the player drew) is laid out, each ground, hover and naval unit's order
  going to its own place (`Sim::formation_plan`, which the HUD also calls to mark the places):
  in rows by role (`formation::rank`: the short-ranged in front, longer reach, anti-aircraft,
  builders and Commanders behind), each unit taking 9/4 of its size across (the larger of its
  footprint's radius and its model's reach, `ModelDef::reach`) and 12 px, each row keeping the
  order across its units stand in so that their ways don't cross; groups over 400 keep their
  shape, and a line drawn for more than 400 is taken in the order they stand across, its
  crossing ways not swapped (every pair, eight times over, held up the tick it was given
  in). A place a unit can't stand on, out of its region, taken, or where one of the player's
  other units stands goes to the nearest cell that will do. Sent further than twice its depth
  and 320 px, the group first forms up: the formation set down a row ahead of its foremost
  unit, each unit going to its place there (`Sim::formation_goal`) until three quarters are
  there (or the longest walk is over). Then it marches: each unit's way to its place leaves
  the group's shared field as early as it can (`FlowField::route_to_with`, `far`; a formation
  shares a field however small), so from places side by side the ways run side by side, and
  each unit's speed is held to get it there when the slowest gets there, a tenth late
  (`Sim::paces`, never under a quarter of its top speed: it still steers); the time is put
  back for one that would be late, and one hopelessly late, stuck for 2 s or fighting is let go
  at its own pace; an attack-move with a third of it fighting isn't paced. Members come all
  the way to their places (an attack-move's within 8 px, a crowd holding them off within two
  of their sizes, not a crowd's reach), and those that turn standing then face the way the
  formation faces (`Unit::face`). Patrols take places without marching. `Unit::formations`
  records each unit's formation for each group order in its queue; formations that every unit
  has left are dropped every second. Off, a group order sends every unit to the one point, as
  TA did.
* **Steering** (`Sim::ground_step`): TA's `SteeringMode` (see FORMATS.md). Most kbots and
  the Commanders (2) turn standing; tanks, hovercraft, ships and the kbots that build or see
  radar (1) turn only on the move, setting off the way they face and coming round in an arc
  (but one standing where it can't, a building laid over it, turns on the spot to walk
  straight out). A unit slows into turns: at speed v, turning at its turn rate ω, it runs
  round a circle of radius v / ω, so it keeps under the speed whose circle reaches the place
  it steers for, d away and θ off its heading (ω·d / (2 sin θ); behind it a tighter circle,
  down to d / 4), braking at its `BrakeRate`; those that turn standing also slow by how far
  they face from their way, and stop to turn round. Routes never cut a blocked corner, so
  one through a pass running diagonally across the grid zigzags cell by cell: near such a
  zigzag a unit steers for its furthest waypoint (four on at most) that it can drive
  straight to, sliding past corners open on one side (`Sim::carrot`, `path::line_slides`),
  down the pass's middle, and standing on a waypoint past the next it goes on from there.
* **Scripts** run their threads only when one is ready (most sleep most ticks), and walk
  the piece animations only while something moves. Each unit draws its scripts' random
  numbers from its own generator (seeded from the game's seed and its id), so they run in
  any order. Which engine calls a script answers
  (`defs::Callins`) is looked up once per unit type, and the sim makes them as TA does:
  `SetSpeed` to extractors (their draw) and wind generators (with `SetDirection`, whenever
  the wind turns), `SetMaxReloadTime` after `Create`, `MoveRate0..3` for aircraft,
  `RockUnit` on each shot and `HitByWeapon` on each hit (500 × the direction of the push, in
  the unit's frame: a jolt of about 3°), `SetSFXoccupy` (land, shallow, deep, carried:
  amphibians change form), `TargetCleared`, and `QueryNanoPiece` while building (the client
  sprays from the piece named; factories alternate). `tests/animation.rs` checks each
  against the shipped scripts.
* **Placement**: footprint cells must be free, dry or deep enough and flat enough;
  geothermal plants (`G` yard cells) must cover a vent, and vents never block. Extractors
  must cover a metal deposit cell, except on metal worlds (plain ground of 100 metal or more,
  `World::metal_world`); the HUD says so while placing one, and the AI's metal spots follow
  the same rule. Mobile units standing where a building is laid (its builder too) step off
  it (`Sim::step_aside`: a move to the nearest place they can stand, ahead of their orders;
  from inside an obstacle a unit walks straight out); an enemy standing there blocks the
  site. Units a falling wreck covers are thrown clear of it, a cell or two, their orders
  untouched (`Sim::throw_clear`: sending each off it and back made thousands of new routes
  in a big battle).
* **Combat** (`combat.rs`): targets via a uniform grid; aiming through the unit's own
  `AimPrimary` script; muzzle from `QueryPrimary`; ballistic launch angles solved
  analytically; projectiles swept against unit spheres, going off where they meet the first
  unit along the tick's flight, or the ground or the sea (`Sim::spent` keeps those that ended,
  and the explosion event says how far through the tick, for the client to draw the last
  stretch and set the blast off as the drawn shot arrives); TA/Spring edge falloff; deaths run
  `Killed`, spawn wrecks, and trigger `ExplodeAs` blasts (which can chain; a blast no archive
  has is none, as in TA: see FORMATS.md).
  Aiming never kills a script's aim: the script's own `signal` ends the last one when a new
  one starts, and some guard a step from it (see FORMATS.md). A new aim is asked for only
  once the last has answered (else after three seconds): after every shot, when the target
  has moved (every half second at most, at once past 22°), and after a "no" every 0.2 s;
  older aims still running are `ThreadTag::OldAim`, their answers dropped. A yes holds
  until the next answer.
  Command-fire weapons (D-gun, bombs) fire on `ManualFire` orders, and on attack orders
  when they are the first weapon (bombers; the Phoenix and Hurricane carry an anti-air
  laser beside their bombs, and used to fire only that) or nothing else the unit carries
  can hit the target; the Commanders' D-gun is their third weapon and waits for manual
  fire. A plane's bombs also find their own targets on patrol and attack-move with fire at will
  (TA's bombers stand on hold fire, `StandingFireOrder=0`): what its side sees or has on radar
  within its bombs' lead plus 160 px (`Sim::bomber_lookout`; the lead is 460 to 565 px for
  TA's bombers, beyond their 350 px sight), the nearest first. It keeps the target for its runs
  (not cleared after a drop; a building through the turn out of its sight as well,
  `Sim::held_building`) and flies on along its way once the target is gone; a plain move
  bombs nothing. An attack order closes to the reach of the weapons it fires. Shells pick targets
  they carry to (v² / g on level ground, with the height between: `Sim::weapon_reach`).
  Direct-fire shots pass through friendly units; shells, bombs, blasts and the D-gun
  still cause friendly fire. Flames harm neither the unit that throws them nor its side:
  they pass through friends, and their blasts spare them. Only weapons that can engage
  aircraft hit them in flight: bombs fall past the other bombers of a strike (ten Thunders
  over one target used to shoot each other down) and shells pass under planes overhead; no
  shot strikes a friendly plane in flight, and a plane shot down in the air blows up without
  touching the aircraft flying round it. Only `waterweapon`s reach what is under the water
  (`Sim::submerged`: submarines, units on the sea bed covered by it), by shot or blast.
  `toairweapon` weapons engage only aircraft, other direct-fire and guided weapons both,
  shells and bombs only the ground. A weapon takes its bad targets on last, only when no other
  enemy is in reach, and turns to a better one coming into reach (`Sim::bad_target`,
  `Sim::can_engage`, `Sim::find_target`'s first key): the categories its unit's
  `BadTargetCategory` names, or the weapon's own `wpri_`/`wsec_`/`wspe_badTargetCategory`
  (category tokens are sets, `CategorySet`: a bit for each of up to 256 tokens;
  `UnitType::weapon_bad_masks`). TA's anti-air units name NOTAIR (the AA K-bots, missile trucks
  and towers, flak, fighters take aircraft first, and a tank only with no aircraft about), its
  other armed units VTOL (an aircraft last), the cruisers' and destroyers' depth charges NOTSUB;
  units chase only what they take on first (`Sim::engages`), and an attack order still sends a
  unit at a bad target. A player's `+shootall` (`Player::shoot_all`, `Command::SetShootAll`)
  makes every target alike, the nearest first. (The anti-air hovercraft carry the ground
  hovercraft's tags in TA's files and are read as anti-air: `defs::target_tags`. A weapon whose
  bad targets are every unit a player can field that it could hit would never fire of its own
  accord; TA's own have none, mods' tags of their own make some, Total Mayhem's flak listing
  `ALL`: its bad targets are lifted, `defs::lift_bad_targets_of_everything`. Units nobody builds,
  reached from no side's Commander through the build menus, don't count: Escalation's joke unit
  and mission-only units lack its `ALL`, which kept the Raptor's Starburst Missiles silent.)
  Among the
  rest the unit that hit it in the last two seconds first. Return fire answers only an enemy
  (a friend's shell or blast splashing it is not: in a big battle most of the shots had soon
  turned on friends), only with weapons that can hit the attacker (a bad target too), and only
  once the unit is finished; a weapon takes it on when it has no target, or
  from one of its own choosing that can't fight back (no weapons) with the attacker in reach.
  An idle unit that roams or manoeuvres goes after an attacker out of its reach only if one of
  its weapons would take it on and its `NoChaseCategory` does not name it. Hold fire drops
  targets kept from before.
  Line of fire: before a shot, straight weapons trace the line and cannons their arc
  (every 16 px, short of the last 24) against the ground; a target chosen automatically
  that the ground hides is passed over for two seconds (`WeaponState::blocked`), so units
  behind a hill or ships under a bank take on what they can hit. A target named by an
  attack order is fired at regardless, as in TA. Guided missiles and bombs are not
  checked.
  Friends in the way (`Sim::friend_in_the_way`): nor does a unit fire with one of its side
  in the way: on the line of a straight shot or missile, under the low part of a shell's
  arc (higher than 120 px over the ground it passes over everything), or within half the
  width of a blast of 48 px or more where it comes down (bombs only by their blast).
  Planes in flight, the shooter and the target don't count. An automatic target is passed
  over for two seconds, as one the ground hides; an attack order on an enemy waits for a
  clear shot. Deliberate fire goes as ordered: manual-fire weapons (the D-gun, missiles),
  shots at the ground, at one's own units or at missiles. Only the unit grid's cells with a
  friend in them are searched, and an answer holds for six ticks
  (`WeaponState::friend_check`); at 20,000 units the weapons take about 0.4 ms a tick more.
  In a siege of 600 Peewees against a Core line the line hit its own towers 39 times in 40
  s without it, never with it (`tests/friendly_fire.rs`).
* **Paralyzers** (`paralyzer=1`): stun instead of damage; TA's damage number is the stun
  time at 60 per second, extended per hit up to a minute. Stunned units freeze entirely.
  Aircraft and `ImmuneToParalyzer` units are unaffected.
* **Kamikaze** units (mines, the Invader) blow up as `SelfDestructAs` when an enemy ground
  or sea unit comes within `KamikazeDistance`; weaponless ones walk to attack targets.
* **Stockpiles and interception**: `stockpile` weapons (nuclear missiles, anti-nukes) build
  one missile per reload time, paying metal/energy per tick like construction (up to 10 in
  stock); launches draw from the stock. Sent at a unit, a stockpiled missile that is no
  interceptor (TA's nukes, its EMP and tactical missiles: none says `tracks`) goes to the ground
  the unit stands on as it is launched and never follows it, as TA's did: the projectile's target
  is that place from the launch (`Sim::launch_target`, `WeaponType::strikes_a_place`; the radar
  path, impact marker and countdown show it). Other missiles sent at a unit follow it, `tracks` or
  not (TA's vertically launched rockets don't say it either). Interceptors (`interceptor`) fire
  automatically at enemy `targetable` projectiles headed into their `coverage`; the interceptor
  chases the missile (`Target::Projectile`) and both break up without the warhead detonating.
* **Missile timing**: `flighttime` is a missile's lifetime; on vertically launched missiles
  `weapontimer` is the climb before guidance (elsewhere it doubles as the lifetime). The
  climb starts at the weapon's `startvelocity` (none, for most) and gathers speed at its
  `weaponacceleration`: a Merl's rocket rises about 600 px in its five seconds (at full speed
  throughout it rose 2,000 px and ran out of fuel before it came down). A vertically launched
  missile with a `turnrate` is guided whether it says `guidance` or not (TA's own all do; Total
  Mayhem's nine vertical launchers leave it out, and their missiles climbed for ever or, a nuke
  leaving its tube at rest, hung over the silo). At the top of its climb, straight up, it takes
  its target's heading and turns over towards it (taking north's, it hooked round). TA's `cruise`
  missiles launched upright (its strategic ones: both nukes, the EMP and tactical missiles) then
  fly level at the height of their climb, some 650 px up, and dive onto their target once it is
  60° below them (`combat/projectiles.rs`, `cruise_aim`; one under 128 px over the ground climbs
  back to it first): steered straight at the target from the top of the climb they came in a
  few degrees above the ground and went off against the first hill, building or tall unit in
  front of it, 100 to 500 px short (`tests/nuclear.rs`).
* **Vision** (`vision.rs`): per-player line of sight, radar, sonar and jamming on 32 px
  cells, rebuilt from precomputed disc spans every 6 ticks, plus sticky "explored" bits.
  `Sim::seen_by` gives Visible / Radar / Hidden; automatic targeting ignores hidden units
  (radar contacts can be engaged). Radar doesn't see what is under the water; sonar does.
  Deterministic, so the AI and targeting may use it.
  TA's "true" line of sight (`GameRules::true_sight`, `Sim::set_true_sight`): hills hide
  what is behind them. The ground's height at each cell's centre (the sea's surface over
  water) is kept, and line of sight is traced ring by ring out from the eye (the unit's
  height plus its size, 16 to 72 px; a target's top 16 px over its ground): each cell's
  horizon is the steepest rise on the line back to the eye (its own or the cell's before it,
  from a table per radius), seen when its top clears the horizon before it; integer maths,
  each cell once (2,000 units: about 6 ms a rebuild, against 0.4 ms for discs). Radar and
  sonar stay discs.
* **Acid seas** (TA's `waterdoesdamage`, `Sim::water_damage`): once a second, what is in
  the water (ships, and anything on the bottom; hovercraft and aircraft spared) loses the
  map's `waterdamage` in health, without scripts, alerts or a killer's credit. A second
  being the unit, as Spring's TA engine read it.
* **Orders**: move, attack, fight (attack-move), patrol (reached points cycle to the back of
  the queue; builders assist, repair and reclaim beside the route), guard (follow; builders
  assist the guarded unit's construction), build, repair, reclaim, capture, load, unload,
  manual fire, stop. Work found automatically that turns out unreachable is remembered and
  skipped.
* **Standing orders** (`Command::SetStates`, never touching the queue): fire (hold, return,
  at will) and movement. Idle armed units hit from beyond their reach answer by movement
  state: Maneuver advances on the attacker within a 420 px leash and walks back afterwards,
  Roam follows it anywhere, Hold Position stays.
* **Self-destruct** (`Command::SelfDestruct`): a five-second countdown, cancelled by the same
  command; the unit blows up as its `SelfDestructAs` weapon. **Factory repeat** re-queues
  each finished unit. A **unit limit** per player makes factories and builders wait.
  **Factories** start the next unit once the last is out of their footprint (it ignores the
  factory's walls until then, `Unit::factory`; the factory keeps it in its `yard`), whatever
  its route, or has stopped inside it (given another order there: to guard the factory, say),
  and the pad is clear: their own side's units standing still on it are sent off it
  (`Sim::step_aside`) to carry on from there, others are waited for. Taking the unit in
  production off the queue takes its frame apart, its metal back.
* **Construction counts whole work**: a type's `build_work` is its `BuildTime` in
  worker-time ticks (× 30, raw 16.16) and a builder does its `work_rate` (`WorkerTime`, raw
  16.16) a tick, so any build takes exactly `BuildTime / WorkerTime` seconds (up to 2^40 and
  beyond; a 16.16 fraction a tick rounded down used to make big builds up to twice as slow,
  and the Vulcan and Buzzsaw never finished). What a tick pays, and the health it adds, is
  what the work done so far is worth less what it was worth before, so a build costs
  exactly its price. Repair, reclaiming (features and own units), resurrection, capture and
  stockpiled missiles count their work the same way; `build_progress` and the other 0..1
  progresses are derived from it.
* **Construction under a stall**: each tick a builder's work is scaled by the stall factor
  (what the stores can pay of the demand); the last sliver of a build always gets at least
  one unit of work, so a starved economy still finishes what it builds instead of leaving it
  at 99.99% (and its builder stuck on it).
* **Abandoned construction** (`Sim::decay_abandoned_frames`): a frame nobody has worked on for
  ten seconds (`Unit::worked_at`, set by a builder setting it down or putting work into it, paid
  for or stalled; `FRAME_GRACE`) un-builds, the whole of it in a hundred seconds
  (`FRAME_DECAY`), its metal coming back as the work goes (not its energy), and is gone without
  a wreck when nothing is left; one set down and left at nothing goes at once. A factory's unit
  in its yard does not (the factory's to finish or give up; released when the factory is gone,
  it has its ten seconds from then), nor one a scene or a mission placed unfinished.
* **Unit figures** (`flows.rs`): what a unit makes and uses a second now (`Sim::unit_flows`: its
  production, the wind's and the tide's share, the metal under an extractor, upkeep while it
  is on, cloaking, and what its build job draws at full pace; `Sim::type_flows` for a type put
  somewhere), read-only, taken a second at a time from what `economy_end` credits and charges
  each tick, so the figures of all the units add up to the income and spending of the top bar
  (a test holds them to it). The hover card, TA's unit bar and the placement ghost show them.
* **Transports**: air transports hover over their passenger and attach it to the piece
  `QueryTransport` names (`BeginTransport`/`EndTransport` scripts); ships and hovercraft
  run `TransportPickup`/`TransportDrop`, whose `attach-unit`/`drop-unit` calls reach the
  sim as host requests (piece -1 stows the passenger inside). Their booms reach only so
  far, so a scripted transport comes alongside before it loads; a loading script that ends
  without taking its passenger fails the order at once, and an unloading one that ends
  without letting go (the spot is out of the boom's reach) has the passenger set down on
  the spot directly (`ThreadTag::Transport` tells when the script is done). Passengers
  leave collisions, targeting and orders, and die with their transport.
* **Cloak** (`CloakCost`): energy per tick while cloaked (more while moving); cloaked units
  are hidden from enemies unless one is within `MinCloakDistance`; firing reveals them.
* **Veterancy**: kills earn experience (victim value / killer value); veterans reload up to
  30% faster, scatter up to 60% less and take up to 30% less damage.
* **Reclaim and resurrection**: features give their metal and energy; own units are taken
  apart like construction in reverse, refunding metal, and vanish without a wreck, in time by
  their metal as a feature is (twenty times it in worker time: a metal maker, which costs none,
  at once). A unit is
  finished once (`Unit::finished`: its `UnitCompleted`, its count in the stats, switched on and
  set to hold fire as it is finished): while its side takes it apart (`Sim::being_taken_apart`,
  a builder took work off it in the last second), other builders' work on it only mends what is
  left of it, as TA's repair does, and the reclaim goes on to the end; patrolling builders pass
  it by, and repairers and guards have nothing to do there once it is mended. One left part
  taken apart is built back up as a construction is, paid for, and is whole again without being
  finished a second time. (A builder helping it was finishing it again every tick, and the
  reclaimer taking it apart again, for ever.)
  `CanResurrect` units (the Necro) pour a unit's energy cost into its wreck at their build
  pace and raise the unit whole (a wreck's unit is the first type whose `Corpse` names it).
* **Alliances**: each player has a team; allies share vision (kept per team), never target
  each other, and win together (`Sim::allied`, `is_enemy`, `vision_slot`). What a player
  shares goes to allies with room, in player order (economy end): by default the metal and
  energy its full storage would waste. `Player::sharing` (`Sharing`, set by
  `Command::SetSharing`: TA's `+sharemetal`, `+shareenergy`, `+setsharemetal N`,
  `+setshareenergy N` and the share panel's switches) stops either, or shares everything
  above a kept amount, which the giver loses as allies take it. `Command::Share` gives an ally resources and `Command::GiveUnits` units (not
  Commanders; passengers go with their transport, a factory finishes the unit in its yard
  for the new owner; `Sim::change_owner`, shared with capture). When a player leaves a
  multiplayer game and doesn't come back within a minute (`ta_net::ABANDON_AFTER`), the
  host issues `Command::Abandon`, and the first surviving ally takes over that player's
  army.
* **Air repair pads** (`IsAirBase`) mend landed friendly aircraft at their build pace.
* **Cheats** (`cheats.rs`): `Command::Cheat { player, cheat }`, taken only when
  `Sim::cheats_allowed` (the client sets it for games with at most one human, so replays of
  them agree). Standing cheats live in `Player::cheats`: double or half damage from the
  player's weapons and god mode for its units (`cheat_damage`, applied in `damage_unit`; a
  unit's own blast is never changed), instant build (`apply_build_power` finishes the job
  at once, free) and unlimited resources (storage filled at the economy's end). One-off
  cheats fill storage, place units (`place_units`: buildings on the nearest free sites,
  others on rings around the spot, aircraft anywhere), destroy, mend or promote units,
  lift a mission's build restrictions and defeat the player's enemies (a mission is won).
  `Cheats::used` marks a player who cheated.
* **Fires**: explosions set flammable features alight with the weapon's `firestarter`
  percent chance (most lasers and missiles); a burning feature spreads to flammable
  neighbours (`spreadchance`) and scorches units nearby with its `burnweapon` every
  `sparktime` seconds, then after `burnmin`..`burnmax` seconds becomes its `featureburnt`
  (`Sim::ignite`, `update_fires`; `FeatureIgnited` events). The client shows the
  `seqnameburn` flame animation. Features are found by square (`features.rs`'s
  `FeatureGrid`, 64-cell squares: blasts, fires, patrols' reclaim and the client's view and
  pointer ask it, `Sim::features_near`; the answers in index order, as a look at every
  feature gives them).
* **Alerts and statistics**: enemy fire on a player's units raises `UnderAttack` (at most
  every 10 s per player). The client keeps the places that happened at (`Game::alerts`, the last
  24, for the alert key: F3 or Backspace looks at the newest, and pressed again within eight
  seconds at the one before, a fight being one stop: `nth_alert`) and puts up a notice that goes
  there when clicked (`Link`); a long fight is told once, not every ten seconds
  (`attack_is_news`: not again within 30 s or 700 points of the last). Players keep totals
  (built, lost, kills, resources produced and spent) and a sample every 30 s (income, army
  value, unit count) for the statistics.
* **Aircraft**: climb to cruise altitude (2 px a tick, rising with the ground at least: never
  under it; a staged scene may give one a height of its own, `Sim::set_cruise_alt`, or put it
  in the air at the start, at speed, `Sim::spawn_airborne`), bank into turns, land when idle
  over land (`Activate`/`Deactivate` scripts), take off when ordered. A transport goes down to its cargo's height only while loading or
  unloading. Like ground units they leave their factory once clear of its yard (a transport
  or bomber out on a long errand no longer holds up the next unit). Planes attack in passes
  (steer at the target, run out past it by turn radius + bomb lead, come round); bombs
  release when the predicted impact point is on the target; fixed guns fire inside
  `tolerance`. Planes sent at one target together (one attack order) come in on it across a
  front, each from its place in the group (`Unit::fan`: 0, 1, -1, 2, -2..., `FAN_SPACING`
  apart, drawn in over the last stretch to the target), not one behind the other. Gunships (`HoverAttack`) hover at range like ground units. Landed, they take
  part in the crowd's separation (on dry ground off obstacles), so planes sent to one place
  spread out.
  **Where they land** (`World::air_landing_height`, `UnitType::landing`): an aircraft sits
  level on its footprint's highest corner (no pad is drawn under it, as under a building), so
  it sets down only where its footprint's cells are free ground on the map (not a space map's
  void), dry, or for a seaplane (TA's `Amphibious=1`) water no deeper than its
  `MaxWaterDepth` (it rests on the surface); the corners under it within 3 px a cell of its
  footprint's longer side of each other (`LANDING_RISE_PER_CELL`: 6 under a fighter's 2 × 2
  cells, 9 under a bomber's 3 × 3, about 10 degrees; TA's own rule for aircraft is a
  building's, `MaxSlope=10` a cell, up to 32 degrees, which still bounds a kind that asks for
  less); and round it, as far as its model reaches (at most 96 px), no ground more than that
  over where it rests (its wings, nose and tail would cut into a hillside or the foot of a
  cliff). An idle aircraft (after 2 s with nothing to do) with nowhere to set down under it,
  or where another has landed, is coming down or is flying to land, looks for the nearest such
  place within 16 cells, ring by ring (the nearest middle to it; of equals the northernmost,
  then the westernmost), clear of the places the others have taken (`Sim::parking_spot`:
  landed, flying to one, or over one past its idle delay; of two over one place, the lower
  keeps it, then the first). Finding none it stays in the air, hovering, and looks again
  every second. A plane (armed, without `HoverAttack`) idle over the sea, where it cannot set
  down, does not look: it circles round where it came to rest (`Unit::orbit`, kept a radius
  inside the map), as TA's do, unless it is over a friendly carrier's deck, which it waits over
  to set down on. Flying to its place (or a repair deck) it keeps over the highest
  ground on the straight way there, climbing first where it stands, and planes move as they
  hover when idle (sliding round to face it: a place nearer than their turning circle they
  circled for ever); it comes straight down at the place. `landing_ground_survey` (ignored,
  `ta_sim`) says, for every installed map, how much of the open ground a fighter and a bomber
  may land on and how much of the land has nowhere within the search.

## Rendering (`ta_render`)

* HDR (`Rgba16Float`) with 4× MSAA → bloom (13-tap down / tent up) → tone map (identity
  below 0.8 so palette art keeps its colours) → swapchain; egui on top.
* **Camera** (`camera.rs`): TA's oblique projection, or the perspective camera orbiting a
  focus (yaw, pitch, a zoom in screen pixels per world unit at the focus, a lift that keeps
  the eye over the ground). `Camera::place` puts the perspective eye anywhere, looking any way
  (straight up and down included: the view's up is square to the yaw's right), with a near
  plane of its own (`Camera::near`) for an eye low over the ground; the view's bounds take in
  what stands and flies round an eye that looks up (staged scenes' free camera).
* **Pass order**: (ray tracing: acceleration structures) →
  (the relief's shadows, when the light moved or heights changed) → (ray tracing: depth prepass, trace and blur) → shadow map → (ray tracing) the 3D features' shadow map → (perspective) cloud shadow map and contact occlusion map
  around the view → particle simulation (compute) → (perspective, water in view) the
  reflection pass → units/3D features → 3D trees and rocks (`foliage.rs`) → billboards (trees)
  → terrain → world text → decals → features painted lying → sky on the remaining background
  (what stands on the ground before it: the ground's is the costliest shading of the frame,
  and the depth test skips it wherever something stands in front; nothing standing is blended,
  and what lies on the ground is kept off what stands in front by the depth, so the picture is
  the same; about a fifth of the frame on a forest) → (water in view, or names in the
  world: the pass ends, resolving into the water's copy of the opaque scene when there is
  water, and a second pass keeps its depth read-only; on tiled graphics, `Gpu::tiled`, Metal:
  Apple's own chips, the multisampled colour and the depth are stored only for a pass that
  reads them, the last pass resolving and leaving its samples on the chip, as Apple advises)
  the names in the world again over what
  stands on them (world text's second pass, reading the opaque scene's depth) → water (translucent, depth-tested, no
  depth write, reading the opaque scene's depth and colour) → smoke → dust/smoke/grit/spray particles → additive
  effects → spark/ember/fire/glow particles → beams → heat haze's hot air → post (depth of
  field in photo mode, bloom, heat distortion, tone map) → strategic icons → UI. The frame bindings for lights, fog of war and the
  ground and cloud maps are visible to vertex shaders too (particles sample them).
* **World text** (`world_text.rs`, `shaders/world_text.wgsl`): names and signs in the world,
  drawn with the opaque scene right after the terrain and under the decals (the scorch marks
  of the blasts that fall on them), so what stands in front of a letter hides it and units'
  shadows fall across it. The glyphs are cut by the game (`apps/ta_game/src/world_text.rs`: a
  face's outlines drawn four times larger, two exact distance transforms, averaged down) into
  an atlas of signed distance fields (`R8`, 2048², 64 texels to the em, 8 texels of spread
  either side of an outline; a 512² atlas holds one picture, the emblem); a glyph is a quad,
  `GlyphInstance`: its middle and edge vectors, its cell, colour, how hot it glows, how far it
  is revealed, its own light. Draped over the ground (6 × 6 cells, heights from the ground map
  at each corner, a lift over them, over the sea where the ground is under it) or standing
  free (cloth waves along its normal). The styles are in the shader: paint (lit and shadowed as
  a decal is, worn, dark round its edge to read over any ground, `ember` heat for burning
  coals, cracked and breathing), light (projected: lines and a glow, added, a scan crossing
  it), lattice (a nanolathe's work: solid as far as it is built, its frame green above), a
  picture, cloth. A stage's `[[text]]` puts text in a scene (`stage.rs`); the credits' film
  lays its cards out with it. The pass writes no depth. A second, lighter pass after the opaque
  scene (`fs_text_over`, in the pass that keeps the depth read-only) draws the glyphs again where
  something was drawn in front of them (the depth test the other way round, with the same bias)
  as far as that something stands on the letter's ground: the scene's depth unprojected at the
  nearest sample, over its own ground by more than the ground's own unevenness (not a ridge) and
  within a tank's or kbot's height over the letter (not a building far in front, nor an aircraft):
  so a name reads through the units on it, still the ground's letter, lit as the ground under it
  (`TA_TEXT_OVER=0` leaves it out).
* **GPU particles** (`gpu_particles.rs`): the game queues `ParticleBurst`s
  (`Renderer::emit_particles`, 9 kinds from sparks to fireballs); one compute pass emits,
  moves (gravity, drag, wind, swirl), bounces off the heightmap and compacts survivors
  into two alternating buffers. Draw indices for translucent particles are sorted back to
  front by camera depth, with stable ties independent of compaction order; the water's
  reflected camera gets its own order. Pure additive light skips sorting. Each layer is
  drawn in one indexed pass (and once more mirrored in the water's reflection). Pool 32k-262k by effects
  quality (144 bytes per capacity slot, allocated on first use); past half full new particles thin
  out, past 70% old ones age faster; with nothing alive the passes are skipped. Each particle
  varies its burst's size by its own seed (grit most: a few big clods among many small ones);
  sparks and droplets of spray streak along their flight, grit is irregular clods cut from the
  billow shapes, and a fireball is drawn per pixel (domain-warped turbulence: lumpy outline,
  billows hotter than their creases, a white-hot start cooling into soot that thins into
  wisps), so it keeps its detail at any size. A burst marked `speck` (the embers over lava) is
  never drawn wider than a few pixels however near the eye it passes, a little fainter there.
  The recipes live in `apps/ta_game/src/effects_particles.rs`.
* **Frame bindings** (group 0, shared by every pipeline): frame uniform, shadow map +
  compare sampler, point lights + light grid (storage), fog-of-war texture + sampler, contact
  occlusion, cloud shadows, the ground map (heights, the relief's shadows, ore); with ray
  tracing also its two results (bindings 10 and 11) and the 3D trees' and rocks' own shadow
  map (binding 12: the traced scene holds the models alone, so the surfaces whose sun is traced
  read the features' shadows there, drawn in a pass of their own after the shadow map).
* **Ray tracing** (`raytrace.rs`, `shaders/raytrace.wgsl`; setting `graphics.ray_tracing`,
  taken up at start; `Gpu::ray_tracing` says whether the device opened with wgpu's
  experimental ray queries, `TA_RAY_TRACING=0|1` for captures). Only models are traced
  (the relief keeps `terrain_shadow.rs`): a bottom-level structure per model piece, built the
  first time the piece is drawn, and every frame a top-level one of this frame's pieces
  (`ModelRenderer::traced_pieces`: the draw matrices; cloaked units and placement ghosts
  left out), each instance's custom data its piece's row in a table (first vertex, first
  index; all 24 bits, so 2^24 kinds of piece) and a record by instance of its player colour
  and its treads' travel. Up to 2^21 pieces are traced a frame; a scene exceeding capacity or
  missing a piece's acceleration structure keeps raster shadows for the whole frame.
  A single-sampled depth
  prepass draws terrain and models; a compute pass rebuilds position and normal per pixel
  and traces two rays through the
  light's disc (shadow), two short cosine-weighted ones (ambient occlusion), one to the
  point light that lights the pixel most (explosion and fire shadows; lights marked
  `without_shadows`, as a nano stream's glow inside the building it builds, are left out),
  and on models and wet ground a mirror ray whose hit is shaded from the model's own
  vertices and texture. A two-pass depth-aware blur smooths the shade, stored with the
  pixel's depth; terrain and model fragments use it (`rt_visibility`, `rt_reflection` in
  `rt_on.wgsl`, appended to `common.wgsl` by `util::with_common` when the device traces)
  only where their depth matches the prepass's (on a silhouette edge, the neighbouring
  pixel's whose depth matches theirs best, `rt_pixel`), else the raster shadows and contact
  occlusion (trees, water, effects). The frame gains texture bindings
  per stage, so the device asks for up to 32 sampled textures per stage (the water's
  pipeline, which reads the scene behind it too, then has 18).
* **Shader size and Direct3D 12**: HLSL compilers inline every function call, so a function
  called in five places is compiled five times over, and everything it calls with it. FXC (the
  compiler every Windows has, d3dcompiler_47.dll, which wgpu's Direct3D 12 backend falls back on
  without a dxcompiler.dll; Vulkan is the default) takes time far worse than in proportion to a
  shader's size: the terrain's and the water's fragment shaders, written out call by call, ran
  to some 225,000 expressions and took FXC minutes each. They are written so each heavy lookup
  appears once: `noise_d` hashes its four lattice corners in one vector pass; octaves of noise
  go through `noise_sum`/`noise_at` (the points computed where the call is, so the values are
  as before); repeated lookups (the far land's heights and coasts, the art's texels and mips,
  a material's detail, the swells, the foam's octaves) run in loops, which FXC keeps as loops;
  and `beyond_ground_from`, `shore_distance` and the like take each lookup once whatever path
  they take. Keep it so: a call inside a function used in many places multiplies. The
  `inlined_sizes` test (ta_render, ignored) lists what each function adds to each entry point
  once inlined (`TA_INLINE_WHO=<function>`: who calls it how often); `dump_hlsl` writes the HLSL
  wgpu hands the compiler, and `tools/shaders/hlsl_times.ps1` times FXC and DXC on it. The
  Windows packages carry Microsoft's DXC (`dxcompiler.dll`, a pinned release: installer/README.md),
  and `gpu::backend_options` has Direct3D 12 compile with it by its full path beside the
  program, else with FXC (never a dxcompiler.dll along the PATH; `WGPU_DX12_COMPILER` still
  overrides). With FXC (`util::set_fxc`) the terrain's and the water's pipelines are built on
  threads of their own, side by side (`util::side_by_side`; FXC is a function keeping nothing
  between calls, while DXC is one compiler object wgpu shares, so never with DXC;
  `TA_PIPELINES_ONE_BY_ONE=1` turns it off).
* **naga, patched** (`third_party/naga`, in place of the crates.io copy through the root
  `Cargo.toml`'s `[patch.crates-io]`): wgpu 30's shader translator writes the `SampleMask`
  built-in as a scalar in SPIR-V, where Vulkan requires an array of 32-bit integers
  (gfx-rs/wgpu#7311). The billboards, units and 3D trees and rocks write one (`Covered`,
  common.wgsl), and AMD's Vulkan driver failed building the sprites' pipelines, the first that
  do. The patch declares `array<u32, 1>`; `third_party/naga/TA_REBORN_PATCH.md` has the diff,
  and when to drop it; `sample_masks_are_arrays_in_spirv` (ta_render) checks every such entry
  point.
* **Terrain**: chunked heightmap mesh; fragment shader resolves the virtual map image
  through a tile-index texture and a deduplicated tile atlas with per-tile box mips
  (exact for 32 px tiles) plus a per-tile-average texture for far LODs. Dynamic relighting
  is relative to the painted lighting; sun shadows from a 4096² map fitted to the view.
  The ground is drawn on a smooth surface over TA's heights (`terrain_surface.rs`, in both
  cameras: in TA's oblique projection any height shows the art it covers): smoothed within a
  tolerance (half a unit where units walk, six on the steepest ground, a unit and a half on
  metal and urban worlds, `TerrainGpu::set_built`), pre-sharpened on steep ground, and taken
  at the vertices of a cubic B-spline through those points; vertices whose neighbours all lie
  on one side of the sea stay there. Lone spikes become low mounds, crests and notches round
  off; the heightmap texture (half floats) carries these drawn heights to every shader, the
  waterline's distances follow them, and `terrain::drawn_height` gives them to what stands on
  the ground (`ta_game::scene::drawn_ground`); the simulation keeps TA's.
  The map editor changes it in place: `TerrainGpu::update_heights` / `update_tiles` upload
  the changed region of every mip level (from CPU copies kept for this; the drawn heights
  27 cells round the change), bake the ambient occlusion again 17 cells around it and
  refresh the chunk bounds; new tiles in the atlas, a new sea level, the water's chunks and
  the ground beyond the edges need `load_map`.
  Buildings level it under them (`terrain_pads.rs`, both cameras): the simulation stands a
  building on the highest ground under its footprint (`World::footprint_top`), so on a slope
  or a dune its flat base hung over the drawn ground downhill (TA's camera never showed it; a
  3D camera low over the ground does). When a building on land is put down (a nanoframe yet),
  `ta_game::scene` hands the renderer its pad (footprint, base height, tick:
  `Renderer::add_ground_pads`, once a frame): the footprint's vertices are drawn at its base,
  and what that raised (or cut) at each edge vertex is carried out from it and eased away
  over an apron of 2 to 6 cells, wider the more there is to take up (a bank no steeper than
  about 25° more than the ground under it, falling steadily and easing out at its foot), added
  to the ground's own shape (a cliff beside a pad stays a cliff). Where several aprons meet,
  their offsets are averaged by inverse square distance, so the ground between a high pad and a
  low one runs from one to the other; footprints that share an edge share the higher base
  there (the lower building is dug in a little rather than the higher left hanging); a pad
  over an older one it overlaps (a building gone since) is drawn over it. Pads stay when their
  buildings are destroyed (wrecks and rubble lie on them); `forget_ground_pads_after` takes
  up those laid after a tick the game went back to (the highlights). Footprint vertices under
  the sea stay there and the apron keeps the shore where it is; buildings that float or stand
  in the water level nothing, nor does one whose footprint would need more than 40 units of
  fill (a building forced onto a cliff's top by a mission or a staged scene: it overhangs as
  TA showed it). The ground without pads is kept (`Pads::natural`, laid on again when it is
  redrawn: `set_built`, the editor); pads are indexed by blocks of 16 vertices, and a new one
  redraws only the vertices it reaches (whatever rectangle a vertex is drawn in, it comes out
  the same) and pushes them everywhere: the height field and `drawn_height`'s copy (in place),
  the heightmap's levels, the waterline's distances, the chunk bounds, and the ground map,
  filled again over that area and a shadow's length beyond it
  (`TerrainShadow::heights_changed_in`, its compute pass dispatched over a part of the map).
  Many at once (a game loaded or caught up) are drawn as the one rectangle round them all.
  About 0.15 ms of CPU a pad; 426 at once in 2 ms.
* **Art styles** (`graphics.art_style`, `ta_config::ArtStyle`): Reborn is the default;
  Enhanced Classic saves as `classic`. The shared graphics setting is independent of
  Modern/Classic camera and quality presets, and applies to units, buildings, ground and
  foliage. `TA_ART_STYLE=reborn|classic` is a capture override. `Renderer::set_art_style`
  selects the model path; `Frame::ao.w` carries 1 for Reborn and 0 for Enhanced Classic
  to raster shaders, and the ray-light grid's packed flags carry the same choice. This
  changes visual geometry and shading, not simulation data or map layout.
* **HD surfaces** (`Lighting::hd_surfaces`, setting `graphics.hd_surfaces`; both
  cameras): once the art is magnified, the ground is sampled with a Catmull-Rom spline through
  the 4×4 art pixels (a quarter blended with a B-spline, overshoot clamped), so the dithered
  squares become smooth colour with every painted pixel's hue kept; fine grain in proportion
  to the painted speckle fades in octave by octave as it grows past a few screen pixels, and
  the smoothed luminance's slope (plus the grain's) relights the ground as relief. Models
  mirror the sky and ground on bright metal (Fresnel), and light textures (lamps, beacons,
  glows) shine in HDR for bloom while their unit works (`Sim::working`: switched on, powered,
  not stunned; the scene eases its lights on and out over a third of a second, the flags' bits
  24-31, and its lights by night with them). `TA_HD_TEST=0|1` overrides the setting for A/B captures.
  (Relief from model textures was tried and dropped: TA's noise textures turn it into grain.)
  * **Ground materials** (`terrain_detail.rs`; the maps in `terrain_detail/`, TA Reborn's own
    art drawn by `tools/terrain/detail.py`): sand, soil, grass, rock, gravel, snow, ice,
    volcanic rock, worn metal and lunar dust, each a tileable 512² map of height and
    brightness, uploaded as one texture array (brightness, the height's slopes across and down,
    height; the mips fade to level and plain) in the terrain pipeline's group 2 (the water
    pipeline shares the terrain group and already binds the 16 textures a stage may have). At
    each place the shader takes the two materials whose scores (the art averaged over about
    eight pixels: lightness, saturation and hue; the slope) times the world's weights
    (`world_weights` from the OTA's `planet`, set by `Renderer::set_ground_world`) are highest.
    World priors cover green, desert, water/archipelago, metal, lava, ice, lunar, red/Mars,
    slate, crystal, acid, lush and wet-desert families (urban uses metal) and Earth (TA Reborn's
    real places: every ground from sand to snow), with conservative unknown-world fallbacks. This
    operates over the whole map roster; no map is individually repainted. Detail fades
    with the art's screen footprint and grazing-angle anisotropy. Each material is
    sampled at its own tile (12 to 72 world units) and, for brightness only, as a turned copy
    3.3 times larger, with explicit gradients (sand's wind ripples keep their direction: its
    copy is unturned, 1.13 times larger, and takes over in patches, relief and all, so crests
    fork and the tile does not repeat; the ripples lie low in sheltered patches and fade out
    before they are too fine on the screen to draw). Where a material applies, the paint's dither
    gives way to its four-pixel average (light and shade kept), its grain lightens and darkens
    that, and its slopes relight it as relief. Walls (perspective camera: faces edge-on to or
    hidden from TA's oblique camera, or steep whatever their facing, judged from the bilinear
    heightmap's slope so a step a cell wide counts at its full slope) take no top-down material:
    they are the world's wall material (`wall_surface`: rock, volcanic or lunar rock, plating
    on metal worlds) projected along the x and z axes, in the region's ground colour with the
    paint's shade and smears taken out (much less saturated, but on the red planets, whose
    rock is as red as their ground, and on slate and ice worlds, `terrain_detail::world_wall_colour`;
    at least three quarters as light as the region's ground, as TA painted the faces towards its
    camera dark, but on slate and ice worlds, whose ridges and crevasses are dark rock and ice,
    `world_wall_lift`), rock in beds that
    are a solid texture in world space
    (`bedding`: shade, formations, a recess under each ledge, joints, each filtered by its rate
    on the screen), plating in rows of plates; lit by the sun as it falls on them, the half sky
    they face and the ground's bounce. Reborn applies more material-specific roughness,
    relief and cavity shading across a wider useful viewing range; Enhanced Classic keeps
    more painted pattern with gentler detail. The mip chain fades brightness, slopes and
    height toward neutral at distance, with slope filtering to control shimmer. The
    expensive material/relief path is gated by HD surfaces in both cameras; a cheap Reborn
    palette adjustment remains when HD is off. `TA_GROUND_DETAIL=0` leaves the materials out and
    `TA_GROUND_DEBUG=1` colours each place by its material.
  * **The world's broad look** (Reborn style, `terrain_detail::world_look` per world family,
    `macro_ground` in terrain.wgsl): painted maps are mosaics of a few plain tiles, flat over
    hundreds of units, so the open ground (where the art's tile is close to one of the map's
    most used plain tiles: no features, roads, markings or painted hills) takes on soft
    patches of drier and darker ground (darker in hollows, from a difference of the
    heightmap's mips; winding flows of cooled basalt on lava worlds), wider regions a little
    lighter or darker, and a band by the water or the melt, each a tint and value multiplying
    the ground's own colour, in world space (it runs on past the map's edge); then each
    world's desaturation. `TA_GROUND_MACRO=0` leaves it out.
  * **TA's hatching painted over** (Reborn style, `terrain_strands.rs`): TA drew many hills
    with hair-like strands following the contours, fanning out over the flat ground at their
    foot, which drawn large read as scribbles. When a map of TA's own painting loads (not TA
    Reborn's maps, nor in the editor) on a world of earth, grass and sand
    (`terrain_detail::world_strands`), an oriented line filter finds them in its tiles near
    sloping ground (darker than the pixels either side along some direction, by more than the
    map's own plain ground's dither ever is, grown by hysteresis) and paints them over with the
    ground beside them; the changed tiles go back into the atlas while the perspective camera
    draws (Enhanced Classic, and TA's own camera, whose distance reads them as the shading
    they are, show them as painted). `TA_STRANDS=0` leaves them as painted.
  * **The ground in the wind** (perspective camera, `ground_life` in terrain.wgsl; the frame's
    `rain.z`, `Lighting::ground_life`): the painted ground used to stand still. Where the ground's
    material is grass (soil as green as its paint counts as grass: a meadow's dark turf), sand or
    snow, it moves in the map's wind, carried by `frame.drift` as the gusts that bend the trees
    are (the swell crossing a field is the gust bending the trees in it): over grass cat's paws
    run downwind, soft patches some hundred units across (`PAW_SIZE`, two octaves of gradient
    noise turned off the map's axes) where the blades bend over and show their paler, yellower
    undersides, in packets with calm between; sand smokes with thin strands of blown grains, in
    the gusts, and veils of dust; snow drifts as longer, finer powder. (The grass rolled in waves
    seventy units apart at first: rows of crests over a whole field read as a fault of the
    screen, carried fast in a strong wind, and two sets of them crossing while the wind turned,
    the more often the faster the game ran, as moiré.) The strands lie along one of sixteen fixed
    headings, the two nearest the wind's, never along the wind itself: the simulation rolls a new heading every eight seconds
    (`change_wind`), and a pattern laid out along it swings round the map's corner as it is
    eased, however slowly; with the fixed ones a turn of the wind rolls through the map as a
    front, place by place (`wind_heading`), the strands of one heading giving way to the
    next's. It is shaded with the materials (`mat_k`: it fades as the view pulls out, like their
    grain). Grass lightens by 3.5% at the most in a paw and has no tilt (rows of leaning blades
    catching the sun drew bands of light and shade across the field); blown sand covers at most
    15% of a pixel, its fine streaks fading earlier as the view pulls out, so the wind reads as
    gentle movement over the paint. Each pattern fades before it is narrower on screen than a
    pixel or two. There is less motion in Enhanced Classic, none on airless worlds or in the
    water's reflection, and rain keeps the sand and the powder down. The setting:
    `Lighting::ground_life` is 1 with HD surfaces on and
    the effects at High or Ultra (`ground_life_for` in app.rs: Low and Medium, the levels for
    weak graphics cards, leave the ground still) while the player's GROUND IN THE WIND
    (`graphics.ground_life`, Graphics, on by default, outside the levels; every benchmark pass has
    it on) is on, and 0 for those who asked for reduced motion (`ground_life_strength`);
    `TA_GROUND_LIFE=0|1` for captures. The slow part of it (gusts, the packets of cat's paws, dust:
    hundreds of units across) is found at the mesh's vertices (`life_fields`, passed on in
    `VsOut`'s `life_a` and `life_b`), and only the fine part, the strands and the paws, for
    each pixel: the terrain's pixel shader is already large, and what is added to it slows the
    rest of it down (all of it found for each pixel, the ground in the wind took a sixth of the
    scene pass on grass; found as it is, a few per cent). The foliage and the painted trees
    already sway in the same wind (`wind_bend`; not in TA's own camera).
  * **The world's grade** (Reborn style, `terrain_detail::world_grade`; `world_grade` and
    `world_graded` in common.wgsl, its parameters the frame's `world_grade`): TA painted some
    worlds in a narrow band of dim values (urban, red planets),
    which in 3D read dull and flat whatever the light; lava worlds' basalt is drawn darker,
    so the melt stands out of it. Their ground's whole palette (open
    ground, painted features and markings, walls) is graded before it is lit, and so is all TA
    painted that stands or lies on it, in both cameras: the features' paintings (billboards,
    those laid down, decals) and their 3D stand-ins, so each is in the same light as the
    ground round it and a painting hands over to its model without a step: its value drawn
    towards the world's key from the map's own (the log-average of its land's tiles,
    `terrain::land_key`, so a dim map is lifted more than a bright one of the same world),
    spread about it in stops (easing off towards black and white, lifts easing in under a
    roof), dull colours made richer and the shade and the light tinted apart. A world may
    also draw the map's colour balance (its land's mean colour at luminance 1) part of the way
    towards its own: TA Reborn's Cinder Basin, painted a cooler, mauve grey-brown than TA's
    lava worlds, takes on their warmth (the balance folds into the two tints; the sky's haze
    and the light bounced onto units follow it). Units, sky and water are not graded.
    `TA_WORLD_GRADE=0` leaves it out.
  * Feature sprites magnified past 0.7 texels a pixel (a tree by a unit's eyes) take a
    Catmull-Rom spline through their 4×4 texels mixed with a B-spline (`hd_sprite`), as the
    ground does, instead of bilinear blots; the palette dither's flecks of other hues keep
    their light but take most of the hue of the texels round them. Decals do too.
* **3D trees and rocks** (`foliage.rs`, `shaders/foliage.wgsl`; TA Reborn's own models and
  textures in `features/`, grown by `tools/features/build.py` and drawn by `textures.py`): with
  the perspective camera close enough (the eye's height over what it looks at, as at 1080
  lines: models begin to take over under 780 and have all taken over under 520 with medium
  effects, under 2600 and 1900 with high ones; always in Perspective and the menus' worlds),
  models stand in for TA's painted trees, bushes, ground plants and rocks.
  * **The library**: one GLB of models in three levels of detail (`<model>.lod<k>`: leaf cards
    cut out of a 19-layer texture array by alpha, bark, rock, the gasbags' skin, crystal, the
    Mars plants' tendrils, vehicles' painted metal (`hull`), pipework's steel (`metal`), road
    barriers' concrete (`barrier`) and a second paint (`trim`) opaque, then TA Reborn's festive
    features' own two (`snow`, laid on along the world's axes, and `festive`, an atlas of 8 x 8
    cells of paints, wrapping papers, walls, roofs, lit windows, bulbs: `festive_textures.py`);
    COLOR_0 holds occlusion, how far the wind bends a vertex, how
    much it flutters and a phase). Trees are grown by space colonisation (branches towards points
    scattered through the crown, their ends tapering to points; the trunk tapering from a flared
    foot with low buttress roots) or whorl by whorl (conifers, their sprays folded along their
    middle), their outer shell covered with leaf cards whose normals lean to the crown's own
    (lit as a whole); far levels retain a nested selection of the same cards and their wind
    phases, enlarged about their original centres within the crown's bounds. Every level
    uses the same seeded branch layout and leaf texture, so reducing detail does not regrow
    the tree. The cut-out layers' mips keep their coverage (alpha scaled per level). A rock's
    extras give where it meets the ground (`contact`).
  * **Choosing** (the game's `features.rs`): TA Reborn's own features (`rebornmodel`: the
    festive ones) are their own models, as made, turned as painted, in their own colours
    (`FoliageModel::own`: the needles and bark as their textures are, the snow and the festive
    atlas never tinted, ice its own colour with its light within; `FoliageLibrary::own_colours`),
    snow lying on a snowy fir's needles where they turn up to the sky (`FoliageInstance::snow`),
    their lights glowing and twinkling (the festive layer's COLOR_0: alpha how bright, blue the
    phase). For TA's, each feature's painting (its first frame) is
    measured: width, height above its anchor, how narrow its top is, colours of its crown (and
    of the crown's lit three fifths), foot and all. Its kind comes from its world and shape
    (`kind_of`: green worlds' narrow trees are conifers, lush worlds' wide ones jungle giants,
    Oolok trees tubers, archipelago trees tropical; shrubs bushes; lush and tropical foliage
    ground plants, not vines; rocks boulders (tall ones pillars; not rubble), spires rock
    spires, slate's fog strata clusters of blades, crystal worlds' spires crystals over a plate,
    ice chunks shards (ice shines; crystals have a material of their own, see Shading); acid gasbags as TA paints them (a round camouflage bag on a braided tether, rooted in a pod: five variants by tether length, the one whose height fits the painting picked and the bag kept round), lava energy
    nodes (crystal tubes, their tips glowing: COLOR_0's alpha on opaque layers), Mars plants as TA paints them (a knot of small bulbs, thin tendrils winding flat over the ground and following its slope: `FoliageInstance::extra.y`),
    charred when burnt); a burnt tree is the burnt form of the tree whose
    `featureburnt` it is. A painting drawn flat (TA's features 4 or less tall: shrubs, ground
    plants, small rocks, ice) sizes its model by its width alone; an upright one stretches it:
    as wide, and as TA's oblique view would show it (`FoliageModel::ta_height`: half its height
    plus the ground behind it), as tall above the anchor; trees then lowered to 0.72 of that (a
    painted crown hides a long trunk: three Commanders tall from TA's height). Leaves take the
    colour of the painting's lit side, bark its trunk's (towards a brown), rocks the painting's
    mean; a crystal its painting's top (the crystals), its rock the foot (a spire's plate), its
    light within the painting's blue (a node's tips); the shader takes each layer's mean colour
    to those. Each plant at its place is a little bigger or smaller, broader or taller, its
    leaves lighter or darker, more olive or bluer (`Variety`: its painting tinted with it in the
    perspective camera). A rock on a slope follows it (sheared, upright still:
    `FoliageInstance::extra.zw`) and sinks by what of the ground falls away under it. Urban
    worlds' cars and trucks (painted lying, see Sprites) get vehicles: a rounded pod on small
    wheels under a glass canopy, a cab and a tank on three axles, as long and broad as painted
    (its lift taken off) and turned as painted, the front where its glass is, in its paint's
    colour with the painting's blue in the glass (`vehicle`); the lying painting fades as the
    model takes over. Metal worlds' pipework and urban worlds' road barriers are laid along their
    paintings' lines (`feature_strokes.rs`: the outline thinned to its skeleton, read as a graph,
    pruned of spurs, smoothed and simplified; each line's half-width from a distance
    transform, even through the junctions it runs straight on through): lengths of pipe as wide
    as painted, a ball at each bend and junction, a bolted collar every so often, a domed cap at
    each free end (`pipework`); Jersey barriers end to end, as many as fit at about their own
    length (`barriers`); TA's lift of their tops taken off. Each piece (`Part`) is set on the
    ground at its ends where it lies (`seat_part`, kept once seen). Transit nodes get a squat
    riveted dome on a plinth, as wide as painted; microwave relays cabinets, a tank and a dish
    on a pad, turned so the main cabinet's roof is where the painting's warm roof is, the roofs
    in a second paint (`trim`, the instance's `leaf` colour). What TA scattered is set where the
    painting's patches lie (`feature_strokes::blobs`; a ruin's blocks and rubble's stones by
    their lit tops, the shaded sides between them dropped): desert ruins' masonry blocks as long
    and broad as painted, low; an urban ruin's broken concrete pillars, rods of steel out of the
    break, standing on their painted feet, twice as tall as their painted lift; rubble's stones
    (`scattered`).
  * **Handover**: each model takes over from its painting at its own point of the camera's way
    down (`FoliageView::close`, 0 to 1 over the band above) and of its size on screen (between
    10 and 16 px), over 6% of the way (`HANDOVER_SPAN`), so the forest turns over tree by tree
    and is never all half drawn; a model mid-way takes that share of each pixel's MSAA samples
    and its painting the rest (`handover_mask`: complementary sample masks, so nothing behind
    shows through), the painted shadow fading with it. The billboard is still submitted when
    hidden: its contact occlusion stays. Adjacent levels of detail blend by size on screen
    (216–264 and 63–77 px), partitioning the model's existing samples rather than changing
    the whole mesh at once; outside those bands only one mesh is drawn. Contact occlusion
    still counts each feature once, and reflections and shadows share the same handover.
    Their batches merge adjacent levels that use the same mesh, so a blended tree does not
    draw the same shadow or reflection twice. A model is drawn when it may show
    (`FoliageView::shows`): in the view's frustum, or its shadow
    (its sphere grown by 1.2 times its height), or its mirror image when the water reflects.
  * **In the water** (the reflection pass): the models at their middle level of detail at the
    finest, leaves cut at half coverage; as one takes over from its painting the two split the
    reflection's pixels by a 4×4 ordered dither (`handover_dither`; the pass has one sample a
    pixel, no coverage to crossfade with). About 0.2 ms more in a jungle by a river. With soft
    shadows (High, Ultra) the coarsest level casts shadows too.
  * **Shading**: hemisphere light as on units, the sun wrapping round crowns and shining through
    leaves (shadowed leaves keep 30% of it, through others), the shadow map, clouds, point lights,
    occlusion near the ground, fog of war, rain and moonlight as on sprites. The wind bends trees
    (`wind_bend`, as it sways the paintings) and leaves flutter: its direction and speed eased
    from the simulation's, which changes every eight seconds (`Effects::wind`), its gusts patches
    carried along by how far it has blown (`Renderer::advance_wind_drift`, as the acid's scum
    is), never laid out along its heading (turning about the map's corner, they swept a forest
    far from it whenever the wind turned); a card turning edge-on fades out,
    and twigs thinner than a pixel show as their mean coverage. Crystal is a dielectric: dark
    and glassy as painted, the sky mirrored at its rims, light through it in its body's hue, a
    sun glint in streaks along its striations, an energy node's tips glowing in its painting's
    blue. Where a rock meets the ground the contact occlusion map darkens a ring
    (`occlusion::contact_occluder`). A burning tree chars and glows,
    its flames in its crown (`FeatureView::flames` → `Effects::tree_flames`).
    Reborn emphasizes leaf transmission and physical crystal sheen; Enhanced Classic uses
    more wrapped crown light and restrained transmission/gloss. Both keep the source-derived
    world palette and the same model library, handover and quality controls.
  * **Shadows**: instances at the two finer levels cast, each with its coarsest level's
    triangles (seen from the sun nothing hides any leaf, so every card is drawn: the coarse
    cards cast nearly the same shadow at a fraction of the cost).
  * **Near the eye** foliage within 14-22 units fades; following a unit from behind
    (Perspective's chase view) it gives way round the line from the eye to the unit (34 units).
  * Cost (RTX 3050): nothing measurable in a typical forest view from a unit; about 2.5 ms
    looking low across the densest wood on the largest map (Cavedog Links CC, 10,000 trees).
  `TA_FOLIAGE=0` leaves the models out; `TA_FOLIAGE=1` shows them wherever the perspective
  camera is (to compare with the paintings from TA's height). Setting: Graphics, 3D TREES AND
  ROCKS (off at the Low level).
  * **Edge wear**: when a model loads, each face's edges are judged against the face across
    them in the same piece (sharing the edge's corners, or a face the edge lies on: a turret's
    side on a hull): flat joints show nothing, convex folds are lightened like the worn paint
    on a miniature's edges (and shine as bare metal), creases darkened, a plate's free rim
    mildly lightened (`model.rs` `worn_edges`, tested on a box and a step). Each corner carries
    its distance to up to four such edges (half floats) and their sharpness; interpolated,
    they give every pixel its distance to the edges, and a band 1.1 units wide follows them,
    fading where it would be thinner than a pixel. Any model, a mod's included.
  * **Battle damage**: each unit's missing health (`DrawInstance::damage`) spreads soot over it
    in soft patches of a noise field fixed to its pieces (each piece its own), from a few
    patches when lightly hit to a blackened hull, with embers glowing through thin cracks
    beyond 60% (`TA_TEST_DAMAGE=<0..1>` shows every unit so, for captures).
  (The near-ground occlusion now shades once with the metal reflection kept: before, it
  recomputed the colour without it.)
* **Models**: one vertex/index buffer for every model and its Reborn detail; per-instance
  piece matrices in a storage buffer (animation = one matrix per piece); team colours via
  10-frame textures, other multi-frame textures (flashing lamps, pulsing glows) animate at 8
  frames a second on the pieces whose script lets them play (TA's `dont-cache`, see
  FORMATS.md: `DrawInstance::playing`, a bit a piece, from `Unit::textures_playing`) and hold
  their first frame elsewhere (a metal maker switched off). Only the textures the models name are loaded (`model::drawn_textures` of
  every model in `objects3d`, read on all cores when the game starts, about 10 ms; a mod's
  GAFs often hold thousands more: TA: Enhanced's models name 1,923 of its 4,663), each entry
  whole, numbered on through up to two arrays (`TEXTURE_BANKS`), each as many layers as the
  card allows in one (2,048): TA: Enhanced's textures take 2,475. The shaders look in the
  second from its first layer on (`SECOND_BANK`, an override constant: 0 and folded away
  when one holds them all; the ray tracing compares with the first's layer count).
  `TA_TEXTURE_BANK=n` holds each bank to n layers, to see the second at work on TA's own.
  What still does not fit, and entries over the packed eight-bit frame count, warn and use a
  fallback layer. Corners where faces meet at less than 45 degrees share a normal (`model/smooth.rs`), so
  barrels, domes and rounded hulls shade as curves. Each face carries its corners' distances to
  its four sharpest edges and how convex each is (`worn_edges`): the unit shader rounds convex
  edges with them (`bevelled`: the normal turned towards the face across over the last half
  unit, broader for Reborn), wears them bright and darkens creases.
  * **Reborn's detail kit** (`model/kit.rs`): when a model loads, new geometry is fitted to
    its own faces as a second version of it, the same pieces in the same order. Large flat
    faces become armour plates: the outline moved in by a margin, raised, a dark seam sloping
    down to where the face was; where an edge is open (`open_edges`: no other face meets it,
    common in TA's models) the plate reaches it and lies flush, its seams drawn on the face.
    A big four-sided face is laid with a grid of plates (up to 3 x 3); the biggest take a
    low rivet head at each corner; a face painted with a picture of its own (`PICTURES`)
    takes one plate. The outer side of a track (TA's `Tredside` textures on a face looking
    left or right, away from the model's middle; a side's faces in one plane taken together)
    becomes a relief fitted inside its outline: a dark belt, a row of road wheels and the
    track loop round them standing a little out of it, never into the hull; as the unit drives,
    the loop's links run round it with the ground (its uv runs round the loop, a whole number of
    links to it) and the wheels' hubs turn, a lug nut bigger than the rest (the shader rotates
    the hub's uv by the distance over the wheel's radius); marks passing too fast to follow
    blur away, the fine ones first (the instance's flags carry how fast its treads run). Effect pieces
    (flares, fire points), untextured faces, lights, logos, treads, functional markings
    (`MARKINGS`: runways, landing circles, pads) and curved, small or layered faces stay as
    they are; layering counts the model's other pieces at rest (`kit::Around`), and a plate
    keeps under another piece's face a little over it. TA's faces are one-sided and seen
    through from behind, and so are the plates (their seams and rivets slope gently enough
    to face away from an eye behind them); a track's side gets a dark back, as its wheels
    would show through an open pod. Bolts and wheels have a budget per model.
    `ModelLibrary::detail_for` picks the detailed version for Reborn where the model is at
    least 56 pixels across on screen (always with no camera: the showroom, wallpapers); the
    colour, shadow, ray tracing and debris passes all draw that choice, the water's
    reflection the original. `TA_REBORN_DETAIL=0` shows Reborn's paint on the original
    shapes. A test fits the kit to every installed model and checks each detailed piece keeps
    to its original's extent (about 7.9 times the triangles over the whole install);
    `TA_KIT_DUMP=a,b TA_KIT_OUT=<dir>` with the ignored `dump_kit_models` test writes models'
    detail as OBJ.
  * **Reborn's paint** (`model/remaster.rs`): every texture frame of the install remastered
    when the game loads, 128 texels a side, in two more arrays: colour, and a finish (normal
    XY, roughness, metallic). Each family's lit and shaded variants (ArmCam3a to 3d) are
    brought towards its brighter ones (at most 2.5 times), since Reborn lights the models
    itself; then palette snapping enlarges it: an edge-keeping smoothing takes the dither out,
    a bicubic enlargement gives each texel its rough colour, and that colour is drawn to the
    nearest of those painted around it in the original, so edges come out crisp and every
    colour is the artist's. A faint grain goes back in; the paint's relief becomes the normal
    map. Team colours, lights and animations keep their paint exactly. About 117 MiB with mips
    for the 704 layers the install's models name, made in parallel in under a second; Enhanced Classic keeps
    its 64-texel array (Scale2x from TA's pixels).
  * **The launcher's skins** (`apps/ta_launcher`): `pages/skins.rs` is the library (the cards'
    pictures and the details' preview come from one worker with its own graphics device,
    `skin_preview.rs`: a replaceable request first, thumbnails queued behind it and kept as JPEGs
    under `cache/pictures/skins/`); `pages/skin_editor.rs` draws the editor, `skin_editor.rs`
    holds its skin, undo and saving, and `skin_patterns.rs` draws the pattern tiles on the
    processor as `livery.wgsl` does. The editor's preview is `UnitStudio::set_skin_for_editing`
    plus `set_livery`, so a slider moves the paint without new models; `ta_skins::update` saves
    an edited skin in place (the manifest last, read back, rolled back if it does not load).
  * **Unit skins** (`model/skin.rs`, `shaders/livery.wgsl`; packs are `ta_skins`'): a skin's
    mapped PNGs in a bank of their own (one for our side, one for the enemy's when they
    differ), and its livery as numbers in a small uniform buffer the unit shader and the
    ray-traced reflections both read (one shared WGSL file, prepended to each; a test holds
    its struct to `LiveryUniform`'s size). A face takes the livery when its vertex has bit 30
    (eligible armour a mapped texture leaves, and the kit's seams round it), its bank by bit
    31. The paint is a pattern in the piece's own space, shaded by the face's paint against
    its texture's average (the texture's smallest mip), with trim, chips and grime from the
    face's edges; Reborn's normal maps and kit stay. A livery change is one buffer write
    (`TextureLibrary::set_liveries`); `new_for_liveries` gives every unit the livery bits so
    an editor's preview can change it without new models, and without a livery draws
    Reborn's own to the pixel (a GPU test holds it so).
* **Sprites**: shelf-packed atlas, 4096² to begin with, growing 4096 texels at a time (taller,
  then wider: each widening a new column of shelves; a big mod's a little over 8192 × 16384 is
  12288 × 16384, not twice that) up to the largest texture the device takes, a big mod's
  sprites and all; past that, frames are left out and the log says so once. The weapons'
  explosions (`effects_anims.rs`) take at most `gaf::EXPLOSION_TEXELS` of it (128 Mi texels,
  half the atlas on a card taking less): files read once for their frames' sizes, the set
  planned (`gaf::plan`), then each frame decoded on its own (`GafReader`); a set over the
  budget (TA: Escalation's, 1.2 Gi texels painted) has its animations larger than TA's own
  drawn at one density, each frame reduced as its rows are read and drawn as big as painted
  (`SpriteAtlas::add_reduced`: fewer texels, the same `geom`); everything else exactly as
  painted. TA's own `fx/explode2`-`explode4`, which Reborn's own effects use for a dying
  unit, its pieces and its wreck, are used only at TA's sizes (Escalation's are shield
  bubbles 760 to 1140 px across). Camera-facing
  billboards whose depth is flattened to their ground anchor (TA sprites are paintings), what
  is painted above the anchor as near the eye as an upright tree's crown would be (a slope or
  a cliff behind a tree's foot does not cut it); with HD surfaces the standing features'
  paintings are calmed as they load (`model::calm_sprite`: the palette's static mixed as the
  eye does, their light and shade kept);
  decals (their edge colours spread into the transparent texels round them,
  `add_gaf_decal`, so they fade into the ground without a dark rim; with the perspective
  camera draped over the ground in up to 4×4 cells, and faded where seen edge on); additive
  and smoke layers. A magnified effect (TA's explosion art up close) takes a premultiplied cubic
  B-spline through its 4×4 texels, kept to its own frame, instead of bilinear blocks; with the
  perspective camera additive effects fade out over a quarter of their size where their quad
  meets the ground (soft, as the smoke and particles do). TA painted every feature from its
  camera high above: the low ones it stood up (cars, trucks, pipes, barriers, ruins, transit
  nodes: `sprites::painted_lying`, low or no taller than half again their footprint's length
  with their painting round the anchor, and no 3D stand-in) the perspective camera lays on the
  ground as TA showed them, north up and draped over the ground in 8 × 8 cells, their outlines
  smooth and lit as the ground they lie on (`SpriteLayer::Lying`, `vs_lying`/`fs_lying`); the
  classic camera shows them as TA did. A metal deposit's painted patch is the classic camera's
  alone: the perspective camera lays a bed of ore in the ground itself (terrain.wgsl, `ore_k`:
  steel-grey gravel, veins, a stain round it, a little metal in its finish, glints of a couple
  of pixels at least, none off it) where the game's `ore::deposits` says each deposit's bed
  covers the cells (a disc round its middle as wide as TA paints a deposit), its crystal
  cluster (`ore::cluster`) standing on it.
* **Huge maps**: maps go to 16384 cells a side. `Renderer::map_fits` says when the device's
  largest texture is too small for one (the game refuses the battle with that message), and
  the minimap's fog stays within 1024 texels a side.
* **Beams**: additive ribbons for lasers and lightning.
* **Water** (`water.rs`): one quad per wet terrain chunk at sea level; travelling swells
  and filtered noise, sky-gradient reflection, sun glitter, absorption by depth from the
  heightmap, soft translucent surf along the waterline (on the *painted* shoreline where it
  agrees with the heights, traced coarsely and finely, the bands of the two weighed, never their
  distances; on the ground only up gentle beaches; seen from above, laid thinner as lace; seen
  from low over the sea, a
  broken line of breakers standing at the water's edge), whitecaps where a strong
  wind's sea breaks (perspective camera: on open water, where the waves' crests pile up, fading
  out from about a world unit per pixel; laid out along one of twelve fixed headings at a time,
  crossing over to the next as the wind turns, `turning_whitecaps`: along its own heading, a sea
  far from the map's corner showed new whitecaps every frame while it turned), caustics on the
  shallow sea bed in the terrain shader (soft, curved filaments of light, gone by 26 units
  deep). It reads the opaque scene
  behind it (`WaterScene`: its depth, every sample, and its colour resolved): what lies under
  it is shown shifted by the waves (refraction, the more the deeper; never from what stands
  above the water), and where something stands in it (a hull, a pontoon, a wading unit's
  legs) the water thins out at the contact and a collar of foam lies along it (the model
  draws its own wet waterline, `unit.wgsl`; ships sit a little in the water, `scene.rs`).
  Ships and hovercraft under way leave wakes (`wakes.rs`): the game records each one's way
  (`effects_ambient.rs`), and before the water a ribbon along it is drawn into a wake map half
  the scene's size (foam, the slope of the hull's waves, the slick), whose texels work out a
  bow wave, arms opening at the Kelvin angle in echelon crests, and the churn down the middle
  from where on the ribbon they lie; the water reads it once a pixel. The
  sea's painted colour classifies it as water, lava (glowing crust) or void (no surface).
  Lava's edge follows TA's painted shore within a band of the heightmap's waterline
  (`lava_melt`, shared by the water pass, which leaves painted rock under the level uncovered,
  and the terrain shader, which draws the painted melt above the level as the surface would
  be there, crust, seams and glowing rim and all; `lava_layer`; seen close up the seams stay
  cracks a few pixels wide in grainy basalt glowing beside them, the rim a thin line); the game
  makes acid worlds' water (OTA `waterdoesdamage`) acid: tinted reflections, a murkier body,
  slower waves and drifting scum.
* **Reflections** (`reflection.rs`, perspective camera, water only): every sea is the plane
  `y = sea level`, so the ground, units, 3D trees and rocks, painted trees, effects, smoke,
  particles and beams are drawn once more
  through the view mirrored about it, into a half- or full-resolution HDR target
  (`Lighting::water_reflections`); `*_reflection` fragment entry points drop what lies below
  the water, and the pipelines count clockwise triangles as front faces. A filter chain then
  halves the target into its mip chain and builds the *reach* beside it (how far beyond the
  surface the mirrored world lies, the nearest over each normalized footprint, including
  odd-sized edges). The water shader bends each
  reflected ray by the wave normal, finds where it meets the mirrored world from the reach
  smoothed over a dozen pixels, and samples the reflection filtered by how fast that look-up
  moves across the screen (anisotropic), widened along the view by the ripples too fine for
  the pixels and the rain's rings, falling back to the sky where nothing is mirrored. About
  0.1 ms at 1080p for a naval skirmish, 0.7 ms with 1,300 units in view (RTX 3050).
* **Point lights** (`lights.rs`): up to 256 per frame, culled to the view and binned into a
  ≤32×32 world-space grid; lit on terrain, units, sprites, smoke and water.
* **Fog of war** (presentation only; the rules are `vision.rs`'s): an RGBA8 texture a visibility
  cell, bilinear: how far the cell is in sight, explored and in radar or sonar range (0..1 each),
  kept by `ta_game::fog::FogView`, which eases each from one state to the next as the vision is
  rebuilt (into sight over a third of a second, out of it over a second and a half, explored for
  good, radar in half a second and out in one) and spreads it over the cells round it (1 2 1
  either way) so an edge is a slope a cell or two wide; a fade moves on with the game's clock, a
  battle's first frame, a new viewer or a clock gone back (a replay's seek) draws the sight as it
  is, and a map of more than 262,144 cells is drawn unfaded and unspread. The shaders read it in
  `fog_state` (common.wgsl): each state shaped into a soft edge and pushed about by a slow noise
  that drifts with the wind (more where a cell is between its states), so edges read as mist, not
  a polygon of cells, and applied inside `apply_fog` to every lit surface. The classic view
  dims as TA did (in sight 1, explored a half, never seen a fifth, `fog_brightness`), softly
  and fading. The perspective camera drains what is out of sight towards a cool grey (explored
  72%, never seen 90%) and dims it (60%, 27%) under a slow mist; where a radar or sonar reaches
  the shroud lifts a little (never tinted: only what is out of sight changes colour, and what is
  in sight is the plain lit map, byte for byte what `TA_REVEAL=1` draws), a scan climbs it and
  a thin pale line marks the edge of its range. Beams, particles and sprites take the plain brightness (`los_light`); the ground beyond
  the map fades into the map's mean state (`ground_beyond.y`).
  What the viewer has seen of the enemy's structures is remembered (`ta_game::fog::Memory`, once a
  tick): each one in sight is kept with its piece matrices as it stands (refreshed every
  third of a second), and drawn as a ghost (`FLAG_REMEMBERED`: a cool grey, a little
  translucent, no shadow, dimmed by the fog on top) where it stood while its place is out of
  sight; it is forgotten when the place is in sight and it is not seen there (what a player
  would know, nothing more: a structure destroyed out of sight stays remembered as it was),
  and when it changes hands. The minimap's fog lifts radar ground the same way.
* **Lighting extras** (perspective camera; each has a setting, the classic camera is
  unchanged): ambient occlusion (`occlusion.rs`: terrain creases baked from the heightmap
  at load, plus a per-frame 1024² top-down map into which every unit, building, wreck and
  tree draws its rasterised footprint), cloud shadows (`clouds.rs`, the sky's own cloud
  function), heat haze (`heat.rs`: the game adds sources over blasts, nukes and burning
  wrecks each frame with `Renderer::add_heat_source`; lava seas add their own), sky light
  on units, soft smoke at the ground, and on lava worlds the melt's glow from below on the
  units, trees, rocks, sprites and smoke near it (`lava_light`: how much of the ground around
  lies under the sea, blurred over 16 cells at load, `terrain_shadow::lava_nearness`, in the
  ground map's alpha), and on the 3D rocks and wood from afar (`melt_glow`: the pall over the
  map's melt lights them warm from below, as much as the map has melt, and more on the side
  facing melt nearby, so a basalt spire's shade shows its form). Terrain faces turned away from the light skip the cast shadow (they get
  no direct light to lose), and shade TA painted near black is spared most of the relighting. Together about 0.15 ms on an RTX 3050;
  `examples/showcase.rs` reproduces the comparison views and `Renderer::set_gpu_timing`
  times each pass.
* **Depth of field** (`post.rs`, `shaders/dof.wgsl`; `PostSettings::dof`, photo mode):
  three passes before bloom, their targets made while it is on. The scene's depth (read
  multisampled) becomes distance from the eye (`Camera::depth_mapping`: perspective, or the
  classic camera's linear depth); each 16 × 16 tile keeps its largest blur; then each pixel
  gathers the picture on golden-angle spirals, over its own circle of confusion and out to
  the largest blur of the tiles around (what a blurred thing in front spreads over it).
  Samples behind reach no farther than the pixel's own blur, so sharp things in front of a
  blur stay sharp; a sample that does not reach stands in for the colour gathered so far
  (Gustafsson's single-pass gather), keeping the result weighed by area. Lens: blur grows
  with |distance − focus| / distance; tilt-shift: with the distance from a band across the
  picture. The blur's radius is a fraction of the picture's height (capped at 64 px), so a
  larger photo blurs alike. Bloom and the composite read the blurred picture.
* **Photos** (`Renderer::render_image`): a frame drawn off-screen at another size (the post
  targets resized, the camera's zoom scaled so it frames the same), without the interface
  or the strategic icons, and read back; inside error scopes, so a size the device cannot
  hold fails the photo, not the game. `render` and `render_image` share `draw`.
* **Frames not drawn** (`Renderer::skip_frame`: no swapchain image to draw into, or a capture
  running on ahead, `TA_RENDER_FROM`): the `prepare_*` calls upload as they go,
  and each upload waits on the queue in staging memory of its own until a submission, so a
  frame not drawn still submits (nothing but its uploads) and drops its heat sources and
  particle bursts (the particle queue, should frames go undrawn otherwise, keeps its newest). The ray-traced scene's structures are made inside an
  out-of-memory error scope: one the card has no room for leaves the scene as it was (the
  rest keep the raster shadows) instead of an invalid one failing every frame after.
* **Strategic icons** (`icons.rs`, `shaders/icon.wgsl`): `Renderer::set_icons` takes
  instances (world position, size in pixels, shape, colour and opacity) drawn in one
  instanced pass over the finished image, after the tone map and before the interface, with
  no depth test (they show over hills and buildings). Each is a screen-aligned quad whose
  fragments evaluate its shape's signed distance (circle, square, triangle, diamond, star,
  hexagon, ring, hollow square and circle), filled in the colour, lit a little from above,
  inside a 1.5 px dark outline: crisp at any size.
* **The intro's room** (`room.rs`, `shaders/room.wgsl`; the model `room/room.glb`, built by
  `tools/room/build.py`, read by the GLB reader the cockpits share, `glb.rs`): drawn in place
  of the battlefield (`Renderer::render_room`). Materials by name give each surface its role
  (procedural wood grain with wear and an old mug's ring, wall paint, moulded plastic, carpet,
  paper, fabric, glows the game drives, pictures the game painted: one 1024² layer each).
  Cycles baked three vertex colour sets (each channel the square root of the light over a
  scale; the bounced light smoothed over each vertex's neighbours against the bake's noise):
  the moon's bounce and the occlusion, what each quarter of the tube throws round the room at
  unit white (tinted by the picture's quarters at runtime), the lava lamp's light.
  Directly: the moon through an orthographic shadow map fitted to the window's light, filtered
  as wide as the moon's disc makes the penumbra (PCSS), a procedural tree's branches swaying
  across it; the tube as 3×3 patches each the colour of its part of the picture, shadowed by a
  perspective map from the glass's middle (PCSS as wide as the glass), its reflection in glossy
  surfaces the picture itself at Karis's representative point, blurred through its mipmaps by
  roughness; a fill from the seat in the picture's colours (the light the viewer throws back).
  The shadow maps are drawn once (nothing moves). The lava lamp's glass traces its wax (blobs
  as smooth-unioned ellipsoids rising and sinking, glowing hotter low down) through the glowing
  liquid; dust motes (lit by one look at the moon's map) and the coffee's steam are billboards;
  the light in the air is marched at half resolution through the moon's map and laid over the
  picture with weights by depth (its noise fixed per pixel, the soft shadows' per point of a
  surface: re-rolled every frame or keyed on the screen's pixels as the view drifts, they
  flickered). The tube's glass is a bulging grid showing the movie in
  scanlines and phosphor stripes; it undoes the room look's tone curve so the movies show in
  their own colours. The composite's room look (`PostFrame::room`): a toe-and-shoulder curve
  that keeps the dark readable, softer glow, lens fringes, film grain; photo mode's lens depth
  of field focused on the tube (`RoomFrame::dof`).
  The seasons: one model holds three rooms. A node named `season.<name>.<material>` (`christmas`,
  `newyear`) is put up in that season only; `RoomModel::from_glb(bytes, season)` leaves out what
  the season does not show, so a room carries only what it shows (`Season`).
  Cycles baked the room without any season's things, then each season's own standing in it, so
  the ordinary room's light is as it was. What a season lights is not baked: its glowing parts
  (`GLOW_NAMES`: the tree's four colours, the strings, the star), up to three analytic spread
  lights (`RoomFrame::lights`: the tree's twinkling colours, a string's warm light, a party's
  colour turning over the table) added to every surface and to the light in the air, a flash
  that adds the fireworks' colour to the moon's light through the blinds (`RoomFrame::flash`),
  and, drawn in the window's glass and sky, the snow falling in three depths, the frost creeping
  in from the pane's corners (Christmas) and the fireworks' sparks (`RoomFrame::bursts`, New Year).
* **Space** (`space.rs`, `shaders/space_common.wgsl`, `space_worlds.wgsl`, `space.wgsl`): the
  films' space scenes, drawn in place of the battlefield (`Renderer::render_space`) from a
  `SpaceScene` (what is there, made once) and a `SpaceFrame` each frame (the camera, where the
  bodies are). Everything round the eye (camera-relative f32), depth reversed with the far plane
  at infinity. One pass: the sky (nebulae as a few translucent layers of warped fractal noise over
  the sphere of directions, ragged-edged, clumped, with wisps of ridged noise, a core where the
  gas glows white and dust lanes in front; spiral galaxies as an inclined disc with a bulge,
  logarithmic arms in clumps, dust lanes and knots; the sun's disc), the stars (a catalogue from
  the scene's seed: power-law brightness, black-body colours, a band they crowd in; each a
  Gaussian integrated over the pixel, so they cross the pixels without shimmer, dimmed behind
  the dust), the asteroids (`features.glb`'s boulders bulged towards a ball, smoothed, split,
  lumped and cratered on the CPU; the rock texture along their own axes, its cracks as relief
  by screen-space bump mapping; lunar-Lambert; the four biggest in view with a shadow map each
  for their craters' shadows; a fill light, as a film's fill lamp, showing their shadow sides),
  then planets and moons farthest first, each a full-screen
  triangle whose rays meet the sphere: Conquest's world recipes (`planet.rs`, ported to WGSL with
  its hash and noise, gaining octaves past its own sizes), relief by finite differences of the
  ground's height, a cloud layer turning on its own with its shadows, the sea's GGX glint,
  lights by night, eclipses by the other spheres, planetshine (an airless body's light from the
  moon's dusty to Lambert's, its octaves fewer for a calmer face, a fill on its night side), edges
  antialiased by coverage and
  depth tested (not written) against the asteroids; the air round each in two passes (its
  transmittance multiplied, its glow added: single scattering, Rayleigh and Mie, marched in 24
  steps, the sunlight at each step through a Chapman approximation; the ground lit by the beam
  and half what the molecules scatter out of it, as sky light). The sun's glare after the pass,
  over the resolved picture, as much as 32 points of its disc are seen past the spheres (through
  their air) and the depth (the asteroids). Post: `PostFrame::space` (`SpaceLook`): its own
  exposure and glow, Lottes's filmic curve (each channel through it where bright, so the hottest
  light burns to white), a vignette, the scene's fade as a video fades (the sRGB values scaled,
  keys joined by straight lines), and triangular dither of an 8-bit step added in sRGB.
* **Cockpits** (`cockpit.rs`, `shaders/cockpit.wgsl`; the models in `cockpits/*.glb`, built
  by `tools/cockpits/build.py`): a Perspective view's crew station, drawn after the scene
  pass into the same HDR target with its own depth (cleared, discarded), before bloom and the
  tone map, with the camera's field of view and a near plane of 1 cm; its geometry is in the
  eye's space (glTF: x right, y up, looking along -z; metres), moved by the frame's `mount`
  (the seat's give). The GLB reader takes the default scene's triangles (positions, normals,
  the baked masks in COLOR_0, decals' UVs; float or normalised/integer, quantised by the
  builder as `gltfpack` would, the node transforms dequantising), materials (factors, emissive
  strength, their extras `wear` and `grime`) and their roles by name (`Role::of`: `paint`,
  `paint_dark`, `hazard`, `red`: painted steel, `paint` in the side's own paint
  (`CockpitFrame::paint`); `trim`; `steel`, `brass`, `weld`: bare metal; `rubber`, `vinyl`;
  `canvas`; `concrete`; `decal`; `glow_trim`, `screen.<n>`, `lamp.<colour>.<n>`,
  `lamp.alarm.<n>`, `glass`), and moving parts: a node `anim.<part>` and all under it, kept in
  the part's space, with the frame's transform applied after its rest (`CockpitFrame::parts`).
  Its triangles are ordered for the one draw: opaque, then the decals (blended over the paint),
  then the glass. `TA_COCKPITS=<folder>` reads `<name>.glb`, `wear.png`, `decals.png` and
  `cockpit.wgsl` from a folder where it has them (the builder's output tried unbuilt). Screens become
  [`cockpit::Screen`]s: the rectangle round the glass in its own plane (middle, half extents
  across and up; not round the vertices' average, which the bake's uneven subdivision pulls
  aside). Each screen has a region of the cockpit's `InstrumentAtlas` (3,200 pixels a metre
  of glass, shelf-packed), which the game paints every frame before the frame is drawn: an
  offscreen egui context of its own (`instruments.rs`) runs
  `perspective_hud::draw_instruments`, laid out in millimetres of glass. The shader maps a
  screen fragment through its glass's frame into the region, four taps across the footprint,
  and lights it as a display: the picture (premultiplied, sRGB) times a phosphor gain, scan
  lines every 1.6 mm that fade before they alias, a darker edge, under the glass's own
  reflections. The ground cockpits' `compass` screen (a heading display above the window)
  replaces the HUD's compass. The cockpits are a crew's stations after years in the field, made
  in the shader from the masks the builder bakes into each vertex (r: occlusion within 35 cm;
  g: the distance to the nearest convex edge of the shapes as built, before their bevels,
  0..1 for 0..3 cm, exact across a face as the distance to a straight edge is linear, a vertex
  added in the middle of a face whose corners all lie on edges; b: occlusion within 3.5 cm;
  a: where hands go) and the shared wear texture (`cockpits/wear.png`, 512² tiling: a chip
  field, scratches, grime clouds, grain; laid on each face along the axis it most nearly
  faces, its gradients the place's own so that mips hold where the axis changes): the paint
  is satin to matte, mottled, chipped where the chip field rises over a threshold that falls
  on edges and where hands go (red-brown primer, bare steel in the cores of the big chips),
  scratched, rubbed lighter along its edges and polished where held; grime and soot gather in
  the seams (b) and corners (r) and run down upright faces in streaks; dust lies on what faces
  up, wiped off where hands go; bare steel and brass are dull, rusting in crevices; rubber is
  worn smooth where held; chips and scratches dent the normal (a height's screen-space slope,
  no tangents). The decals (`cockpits/decals.png`: stencils in TA Reborn's own stencil
  letters, caution, danger and keep-clear plates, data plates, labels; `tools/cockpits/art.py`)
  wear with the paint under them. Lighting: the sky dimmed, the sun on what faces it only
  through the openings (less when it is behind) and less in the corners, a cabin lamp over the
  eye, every instrument's glow on what is round it (a light at each screen, a wide one's at its
  thirds, facing out of it), a pulsing red alarm and a hit's flash; GGX highlights held to a
  few times the light and widened where the normal turns within a pixel; the gloss reflects
  the cab, dark, and only the window's view where it looks out through it. Every
  opening has its glass (`glass.<opening>`, a sheet just behind the frame's face, larger than
  the opening so its edges are under the lip, added after the occlusion bake so that it does
  not darken the cabin): faint, more at a grazing angle, dust and fine scratches that catch
  the sun and the cabin lamp, and in the rain (`CockpitFrame::rain`, `Effects::rain`) drops on
  it, lensing the sky, some clinging and coming and going, some running down their column
  with a wet trail (blown up and out over a canopy in flight, `CockpitFrame::flies`): on the
  glass alone, under the frame and mullions. The HUD's screen-space drops are only for the
  periscope and the views without a model. The crew's knobs are rubber and brass: the side's
  colour on a ball at the edge of the view read as a stray sphere. The instruments' glass and
  the windows' keep their own lighting (as before the paint's), so the pictures on them are
  unchanged.
* **Time of day** (`daylight.rs`, `Lighting::time_of_day`, perspective camera): the hour sets
  the light (the sun's height and colour, golden and then red low in the sky; after sunset the
  moon, from TA's own direction), the ambient light on units, the sky's horizon and zenith (a
  warm glow along the horizon on the sun's side at dawn and dusk; at night stars and a pale
  moon disc with grey seas; `Lighting::moon_dir` puts the moon elsewhere, for a film's shot) and how the painted ground is lit (`Frame::daylight`: a tint on
  its sunlit term, 1 at noon; `Frame::night`: the night sky's own light, and by moonlight the
  painted colours fade by half). Noon is exactly TA's light (the sun's path passes through
  it) and the sun stays in the northern half of the sky (it rises in the north and sets in
  the west), so painted hills stay lit on the side their painter lit them. Everything that
  reads the sun (shadows, cloud shadows, the frame uniform) reads the frame's light
  (`Renderer::frame_lighting`); `Renderer::daylight` gives the game the time's light (its
  `lamps` come on as the sun sets). The terrain's relighting of slopes is capped so a low sun
  does not glare off them.
* **Sky and beyond the map** (`sky.rs`, perspective camera only): a far-plane sky pass
  (haze-to-zenith gradient tinted by the map's average colour, HDR sun, drifting clouds),
  which water reflects and distance fog fades into; each world its own sky (`world_sky` from
  the OTA's `planet`: tints of the zenith and haze, smoke, ash or smog in the air, cloud cover,
  haze density; the moon's has no air: a black sky with stars by day, no clouds, little haze,
  deep shade; a world in space, `terrain_detail::space_world` (TA Reborn's asteroid maps), has
  no horizon either: the stars all round and below, the galaxy's band, faint nebulae and a
  banded planet far beneath the map (`space_backdrop`), no ground beyond the map, and its void
  cells drawn as nothing (`TerrainGpu::set_space`: the cells whose four corners are void
  marked in the heightmap's second channel and collapsed by the terrain's vertex shader, the
  void beside the rock drawn a slab's thickness below it, so each rock ends in a level slab
  edge over the stars; the classic view still draws their painting)), and each map's clouds
  where its own seed puts them (an offset of the cloud field the cloud shadows share); a ring
  of ground around the map: next to it the map's own mirror image (each point shows the
  ground reflected into the map across the nearest edges, under a warp that grows with the
  distance), with the map's art, detail materials, relief and lighting, so whatever crosses
  an edge carries on without a seam; from a few hundred units out (`far_mix`, nearer or
  further along the edges, raggedly) a far land of its own (`far_ground`, `far_color` in
  terrain_common.wgsl): the mirror image ran on to the horizon before, drawing a continent's
  coasts again on every side of it. The far land is made after the map's ground round each
  stretch of its edges (`surroundings.rs`: 128 points round the edges, each measuring the
  ground within 768 units of it, how much is sea, the land's mean height and relief, the sea
  bed's depth, the dry ground's lighter and darker colours and the sea's painted colour, a
  uniform the terrain and water shaders share, measured again after the editor's changes):
  as much of it sea, its coasts wandering by noise where the edge is part land and part sea,
  its land as high and as rugged, in patches of those colours; as the far land takes over,
  the mirror image is read ever more coarsely, so land crossing the edge shrinks back from its
  coasts rather than ending on a line. A metal or urban world's far land is blocks of plating
  and the moon's a field of craters (its regolith's detail map enlarged), lit by the sun
  (`far_surface` in terrain.wgsl). The terrain and water shaders are each built twice
  (`FAR_LAND`, `terrain::shader_source_for`): the map's own pipelines without any of this, the
  skirt's and the outer water's with it (a fragment shader is as slow as its longest path: the
  map's ground drew a tenth slower for code only the ground beyond it runs). Darker and greyer a
  little, haze thickening towards the horizon; seas, lava and mist run to the horizon. Its land
  and sea follow the water's own per-pixel
  shoreline, judged where the eye's ray meets the water (`beyond_ground_depth`): painted sea
  on dry ground shows the painted beach; ground standing out of the water where that is sea is
  drawn as the water is there (painted sea, mirroring the sky), and the flats it leaves dry
  under its level gleam as wet sand as the view grazes them. The strip along the north edge
  TA never painted shows the art reflected across the image's edge, subdued to the colour of
  its first rows.
  The classic camera renders exactly as before, and like TA's never scrolls past the map
  (the part of the screen the interface leaves free stays on it, `clamp_classic_view`).

## Skirmish AI (`ta_ai`)

* A deterministic virtual player: it thinks every 15 ticks on simulation state only and
  issues ordinary `Command`s, so every network peer runs it independently. New records
  also retain those commands; older records regenerate them during playback. The
  players' turns are spread evenly over those ticks (one thinking 5 ticks after the other
  and 10 before won 59% of games between equal AIs: it saw the other's moves sooner), and
  its random choices come from the game's seed (`Sim::seed`), so games differ.
  Unit roles come from properties (extractor, generator, factory, combat value...), never
  names, so mods work. Easy / Normal / Hard differ in `Params` (tech timing, wave sizes,
  factories, towers, production pace, how units are chosen, battle handling).
* It knows only what a player in its seat could: what is in its team's sight (`Seen::Visible`:
  what a unit is, its health), where radar shows something (`Seen::Radar`: a contact of a kind
  not known, a threat of an armed unit's worth: `Survey::contacts`), and what it remembers. An
  enemy building seen is remembered (as last seen, finished or not: `enemy_frames`) until its
  place is in sight without it, whatever happened to it out of sight; an army seen, three
  minutes; the Commander, until its place is seen without it. Deposits are the enemy's where
  an extractor of theirs was seen (one never looked at is taken as free); missiles go at
  buildings seen, through anti-missile defences seen (each taken as stocked). Nothing is
  looked up out of sight. Where the enemy is: the map's start positions (`Sim::start_spots`)
  but its own and its allies', as a player knows them from the lobby (`enemy_bases`, not yet
  seen); a scout (`army::run_scouting`: the factories make a fast unit after the first two
  constructors; a scout plane, or any plane, over water) looks at them nearest first. A start
  with enemy buildings in sight round it is their base (and whose); one looked at and found
  empty is dropped; once every enemy's base is found the others are; an enemy factory far
  from them all is a base found. Waves, the fleet and lifts go for what was seen first, and a
  start not yet looked at only when nothing is known (scouting in force). It is never
  "dominant" (going for the kill) before it has found the enemy.
* Allies (`ally.rs`): players signal their team with `Command::Signal` (a ping with a
  meaning: look, attack, defend, on my way, need metal or energy; the sim keeps the last 32
  for 90 s in `Sim::signals`, the same on every machine, and emits `SimEvent::Signal`). A
  computer ally hears its allies' signals as it thinks: "defend here" goes first among its
  raids to meet (with its whole army in reach, half the strength it would want suffices),
  "attack here" becomes its waves' target for two minutes (and they go sooner), and a call
  for metal or energy is answered with `Command::Share` from what it holds beyond a third of
  its storage. Raiders at an ally's buildings count as threats (half weight), and an ally's
  start and buildings keep the AI off its metal unless nothing else is left. It speaks
  through signals too: raided at its base or buildings with too little at home (worth 300 or
  more, more than four fifths of what is there), it calls "defend here" (once a minute), and a
  wave worth 1,500 calls "attack here" (every two minutes), so computer allies answer each
  other as well as people. Each answer
  (`ta_ai::Answer`: defending, cannot reach, will attack, sent N metal...) is taken by the
  game (`SkirmishAi::take_answers`) and said in the chat in the player's language. The game's
  Alt+click pings, Alt+drag opens the wheel of meanings (`apps/ta_game/src/signals.rs`); with
  no computer ally in a multiplayer game, signals between people stay private chat lines.
  The game shows allies under attack (`SimEvent::UnderAttack` for them too: a notice, F3, a
  ring on the minimap in their colour), resources given (`SimEvent::Shared`), an ally's
  defeat, and, in the modern interface, a card per ally (army worth, Commander's health, an
  alarm while attacked; a click looks there). Computer allies greet the player and say how to
  signal them; a line to allies in single player is answered the same way.
* Team games: `setup::assign_starts` seats allies on neighbouring start spots, the teams
  spread round the map (the spots ordered by angle with integer arithmetic only, so every
  peer deals the same; random starts draw the turn and the teams' order from the seed);
  resigning beside a surviving ally leaves the army to them (`Command::Resign` gives units as
  `Abandon` does, then the rest blows up); a player who keeps watching after a defeat gets
  the team's victory (`Game::watching_team`, "Your team won the battle"); the debrief's table
  adds each team's totals.
* Deposits are the metal features plus, where no feature marks rich metal (metal worlds
  such as Metal Heck, painted patches), the best extractor-sized spots of the metal layer
  (`map::find_metal_spots`); those don't reserve ground, and with storage full the
  builders stop claiming more and spend instead. Deposits on dry land that none of our
  ground constructors can get to (`Deposit::far`: another island) are left to
  constructors that fly or hover (`expanders`: at most one on Easy, two on Normal, three
  on Hard, one per six such deposits), which take them before anything else; with nothing
  to make those, an air plant goes up. An island start that had one extractor at the end
  now takes thirty, and sea maps produce about a fifth more metal.
* `economy.rs`: constructors take extractors (land or underwater, by the deposit), power
  (wind at its average, tides at the map's strength, geothermal vents, fusion later),
  factories, towers, radar, makers and upgrades (an advanced constructor takes the old
  extractor apart, then puts the better one up; the old one, being taken apart, is no
  unfinished building for the others to finish); factories balance constructors and army.
  Starved of metal with power to spare, builders put up makers and reclaim wrecks first,
  then help the factories rather than start projects that would share the metal.
  `army.rs`: raid defence, attack waves on the weakest reachable target (sized against
  the enemy army seen in the last three minutes, `opposition` counting each enemy once),
  reinforcements that join a push in a body, the air wing, the Commander and silos. An
  army far stronger than anything seen (`dominant`) goes for what ends the game:
  factories, the Commander, mined deposits, any building, marking places it found empty.
  Orders only go where the unit can reach (movement regions).
  The air wing strikes from home in groups (`strike_group`, three against a target with
  no anti-aircraft cover seen) at the enemy's economy, the outskirts before a base's heart
  (whose cover is often unseen): the best building and up to four valuable ones within
  350 px no better covered, in one sortie. Each strike is accounted for when its planes
  are home or gone (worth destroyed against planes lost, older strikes counting half);
  when planes lose more than twice what they destroy, none are built for five minutes.
  Hunting (`hunting`: far stronger and nothing seen of the enemy but a few buildings):
  planes strike in ones and twos, and with nothing known one plane or idle air transport
  sweeps the parts of the map out of sight; mopping up (`mopping_up`: of the many enemy
  buildings once seen, a quarter or fewer left) also has air plants build bombers before
  more transports.
* Unit choice: from a factory whose land units can't all get at the enemy, only those that
  can (when there are any); among them by `TypeInfo::value` (firepower times health per
  cost, plus reach) or, on Hard, by `efficiency` for land units: firepower times health
  grows with the square of the number of units, so per cost^1.5, less for units too slow
  to keep up. Hard also handles its battles (`micro`): badly hurt units fall back to the
  rally point and a builder off factory duty repairs them (free in TA; hurt warships go
  back off the shipyard, where a naval constructor kept for it mends them), and fire is
  concentrated on the enemy in reach whose loss costs the most firepower per point of
  damage, just enough guns on each for a quick kill.
* `navy.rs`: when the sea reaches within gun range of an enemy base, a shipyard (early if
  the land army is stranded, first of all when there is no room on land), and a second and
  a third where the war is at sea (the ground army can't get across) once the economy
  feeds them; shipyards build warships and a naval constructor or two for underwater
  deposits. What ships can reach comes from one distance field per movement class and sea
  (`map::Reach`: how far every cell lies from that water), so the fleet weighs every enemy
  building it could shell. It meets enemy ships in our waters; strikes once strong enough
  for the enemy navy seen lately and for whatever drove it back last, at the enemy ships,
  coastal buildings or extractors under water worth the most for the least defence (with
  nothing known, it sails for the enemy's coast to look); ships finished meanwhile join a
  push that is going well; it falls back when outgunned, and gives up for a while a place
  where it is held shooting without wearing anything down (at what it can't hit up a
  bank).
* `transport.rs`: ground and hover units that can't get from home to the nearest enemy
  start by themselves (`crosses`, per type) are stranded. Factories then make the units
  that do cross (hovercraft, amphibians: an island start puts up a plant for them, and the
  tech plant chosen is one that makes amphibians), and the others only as far as
  transports can carry them; a factory that makes transports goes up. Transports are
  chosen by capacity, speed and cost among those that can carry what is stranded:
  aircraft (they go anywhere), hovercraft and ships only where their class suits (it
  crosses to the enemy, or the sea does) when no air transport is on offer or air lifts
  were shot down lately. A lift picks a target (the enemy building worth the most for the
  least defence, towers and anti-aircraft counted; what ends the game when far stronger;
  with nothing known, an air transport flies over the enemy start for a look first) and a
  landing zone near it that the passengers can fight from, away from anti-aircraft cover
  on the way and at the spot (for a surface transport, a shore it can come close to). It
  goes in waves: the transports pick passengers up together (the most valuable first, or
  the nearest for a hovercraft or ship), gather, go over together and set their loads down
  around the zone, then come back for more; the units set down fight on over there. A lift
  that loses too many transports is called off, and the place avoided for a while.
* Duties (`duty.rs`): the player's own constructors, sent to work with ordinary orders. A
  `DutyOfficer` wraps a `SkirmishAi` made for the local player (Hard's economy: two power plants
  and two extractor upgrades at once, at most ten metal makers) with `helper` set: it goes only
  where the player has explored (`Sim::explored`: unmapped games), and never blows up one of the
  player's buildings for a better one. Each beat (`THINK_TICKS`, the computer players' own) it
  surveys and remembers as they do, and runs the builders on duty and no one else (no factory,
  army, scout, silo or maker switch, no answering allies): `run_builders` is split into the pieces
  both share (`start_builders`: what is in hand; `leave_alone`: the stuck, the fleeing, the ones
  working; `give_task`: a task as orders), so a computer player plays exactly as before (a 10
  minute Hard against Normal ends on the same checksum). A duty is a chain of the economy's task
  functions: **Economy** power when it is short and none is going up, free metal (the Commander
  within 650 px of home, others 2,200), makers and storage, wrecks and rocks when metal is short,
  better extractors for advanced constructors that can reclaim; **Repair** the most damaged
  building or standing unit near the base (movers are not chased, nor are those under fire);
  **Reclaim** wrecks (richest first) then rocks, trees and other features near the base (metal
  four times energy, nothing of a kind the store is full of); **Defence** anti-aircraft once
  planes were seen, towers on the approach (2 + minutes / 3, at most 10, only what the economy
  can pay for within a minute) and guard towers at outlying extractors; each ends by finishing
  unfinished buildings, helping a busy factory and heading home. A helper at a factory (a
  `Guard` among its orders; the simulation puts the repair of the product in front of it) is
  looked at again every `HELPER_LOOK` beats and at once when it is given a duty, so work of its
  duty's own (a hurt building, a power shortage) takes it off the factory; a unit on its way to a
  job, or at it, finishes that first (a duty starts when the unit is free). The officer keeps no
  duty of a unit lost, given away or not finished (`DutyOfficer::think`); its state is the
  player's machine's alone, its output ordinary `Command::Order`s, so replays and multiplayer
  need nothing; a saved game keeps who was on which duty (`DutyOfficer::export`, a few bytes:
  the unit, which of the four duties; `import` checks every unit is still ours, finished and
  able to serve, and starts it fresh, as a new duty). What a player can trust a unit on duty to do (the helper's rules in
  `duty.rs`, `economy.rs` and `lib.rs`, all behind `helper` so a computer player plays as ever):
  it never works two to a hurt unit or a wreck (`Plan::claims`, filled from every builder's
  orders, the player's own too, and as the think hands jobs out); it keeps to ground nearer home than the
  enemy's, the Commander to `COMMANDER_REACH` for wrecks and rocks as for everything else; armed
  buildings once seen stay in the threat grid after they are out of sight (`remember`), and a
  task whose straight way crosses a threat cell is passed over for another (`route_safe`,
  `shun`; the way home too); a builder with armed enemies within `DANGER_RANGE` gets away from
  them and holds back `HOLD_BACK` (`keep_away`; in the base as well; the job it was at is left
  alone for a minute, so that it does not dance at the edge of their guns), and the Commander in that
  case is handed back to the player with a `DutyEvent::Danger` (the game tells the player; it
  was only when the Commander was hit); a constructor that cannot get home (another island, a
  plateau) is not sent there every beat (`go_home`: `unit_reaches` and the `homeless` memory);
  a feature the simulation gives up the reclaim order for after a long walk is remembered as out
  of reach, three minutes, twelve, forty-eight (`feature_strikes`). What it tells the player:
  `DutyOfficer::status` (`DutyStatus`: working, getting away, or idle with an `IdleReason` found
  only for a unit with nothing to do, every two seconds: no free deposit in reach, the economy
  done, an advanced constructor waiting for a metal income, nothing hurt, nothing to reclaim, the
  stores full, the defences done, saving up for a tower, stranded) and `DutyEvent::Idle`, once for
  each duty and reason after a unit has had nothing to do for `IDLE_NOTICE` (twenty seconds), not
  again for that unit until a duty is given anew, and the same duty and reason not again within
  `IDLE_AGAIN` (two minutes) for any unit. Tests: `tests/duty_status.rs` (each reason on real data, the
  notice), `tests/duty.rs` (on
  real data: the economy builds extractors and power, repair mends, reclaim takes wrecks,
  defence puts up towers, a helper at a factory leaves it for work, only units on duty are
  ordered, same commands twice, nothing outside what is explored), `tests/duty_qa.rs` (the
  rules above, a lab not slowed, each duty on both sides, islands and sea; its `Audit` in
  `tests/common` counts orders repeated, two at a job, time under an enemy's guns and the nearest
  approach to the enemy), `tests/duty_explore.rs` (ignored: the by-hand runs that found them) and
  the asset-free ones in `tests.rs`.
* Soak test: `cargo test --release -p ta_ai --test skirmish -- --nocapture` (see the file
  for `TA_TEST_*` options: map, length, difficulty, players, teams, logs). Ladder:
  `cargo test --release -p ta_ai --test ladder -- --ignored --nocapture` plays one level
  against another (`TA_LADDER_A`, `TA_LADDER_B`) on ten land maps, from both starts and on
  both sides, over every core (`TA_LADDER_MAPS=sea` for ten water and island maps, `all`
  for every map, or a comma-separated list of names); `TA_LADDER_SEEDS=n` plays each game
  under n seeds, the odd ones with B as player 0. A game not decided in
  `TA_LADDER_MINUTES` (25) goes to whoever owns more (Commanders left out: ARM's costs
  6,300 more); it is played to the last unit unless `TA_LADDER_COMMANDER=1` (the skirmish
  default, losing the Commander loses). Undecided games show what the loser keeps; lines
  sum up metal and kills, the warplanes (cost, losses, strikes, kills), what each side owns
  at the end and what its buildings, ground units, ships and aircraft destroyed, and the
  mean points margin with its standard error. Read the margin before the wins: between
  equal AIs, runs of 160 games came out anywhere from 79-81 to 95-65 (margins within
  about 4%). `TA_LADDER_A_TWEAKS=expanders=3,micro=0` (and `_B_`) change one side's
  `Params` (`SkirmishAi::tweak`); `TA_LADDER_GAME=n` plays one game per map,
  `TA_TEST_LOG=1` prints the AIs' debug lines and both armies at the end,
  `TA_LADDER_ORDERS=1` what the ground units are doing. Last runs (320 games each): Hard
  beats Normal 223-97 on land (margin +20%, 15 outright) and 186-134 on the sea maps
  (+3.2% ± 1.4; before island expansion and even turns it did no better than Normal in the
  same seat); Normal beats Easy 80-0. Sea games are economy races: none ends outright in
  25 minutes; given 40 with the Commander rule about a quarter do, mostly after minute 30
  (nukes from minute 18 instead of 24 change nothing). Between equal AIs ARM wins about
  64% of sea games and 42% of land games. `--test transports` checks that lifts carry
  units across (by air, and over three games by hovercraft or ship with air transports
  forbidden).

## Client (`apps/ta_game`)

The intro (`intro.rs` plays the Smacker movies) is shown in a room when the setting is on: a
bedroom modelled for TA Reborn (`ta_render::room`, see the renderer's notes).
`intro_room.rs` reads the model, paints its pictures from the install's bitmaps, unit
pictures and fonts on a worker thread (`intro_room_art.rs`: posters (the game's own from the
install's art, a film's, a sunset's and a band's drawn in code, original), the game's boxes and
spines, the calendar, the manual, a CD's insert, a sticky note, the turbo and clock displays,
the keys' legends laid out as the model's `anchor.keys` says, a battle planned in pen on a
print of Coast To Coast (`ta_render::map_preview` of the install's map, its starts ringed and
crossed), the computer's case badge with TA Reborn's emblem), and paces it: dark, the camera
coming in to the seat from a step back as the tube warms up from a line and degausses
(`ARRIVE`), the movies (the last, the main intro, watched from closer:
`IntroRoom::focus_on_tube`), the lean-in until the glass fills the view; the mouse pulls the
view a little. It drives the room's lamps (`RoomFrame::glows`: the
computer's, the drive's as the movie streams, the modem's data, the displays), the moon behind
passing clouds, the lava lamp's breathing, and hands the renderer the movie's frames and their
3×3 regional colours (`RoomFrame::glow`). (`TA_ROOM_MODEL=<file.glb>` plays another model in
place of the shipped one: `tools/room/build.py --no-bake` writes one in a minute.)
The room is dressed for the season of the launcher's setting (`intro_room::season`, through
`ta_ui::seasonal::on`, so the seasonal effects off, or a theme the player chose, count):
Christmas through December, New Year on 31 December and 1 January, the ordinary room the rest of
the year (the launcher's Christmas runs on to 6 January; the room's does not) and at Halloween.
`TA_ROOM_SEASON=normal|christmas|newyear` overrides it, for captures and tests, and
`tools/room/build.py --preview <dir> --season <name>` renders one in Blender. `IntroRoom::festive`
drives the season's lights and their twinkling and the fireworks' timetable (`fireworks`: four
launchers, a burst every few seconds each, and the light each throws in at the window, none of it
with reduced flashes); the painted art has the season's calendar (December, January) and banner.

`game.rs` owns the sim, local input → commands, the fixed-step loop, AI players and
events → `effects.rs` and `audio.rs` (for films, `TA_SOUND_LOG` mixes the audio offline on the
frames' clock and writes down what plays as the camera hears it, from the engine's record of
what it tells the mixer: `sound_log.rs`, `ta_audio::playlog`). The viewer's fog of war hides enemy units, effects
and projectiles outside sight; radar contacts show as blips, and the enemy structures seen
before stand where they were as ghosts (`fog.rs`). `scene.rs` interpolates poses
and the scripts' piece states between ticks and poses only units inside the view (its
visuals kept by unit slot, followed after a tick and posed on all cores in big battles). What
stands on the ground is drawn on the drawn ground (the drawn heights taken once for the whole
pass, `terrain::drawn_field`), vehicles tilted with it as the simulation tilts them with its
own (over a pad's bank they ride the bank); buildings on land stand at their base, on the pads
they lay (`scene::ground_pad`, laid as each is first seen). On top of
the simulation's pose it adds motion for the eye (`Motion`: springs on pitch and roll
chasing the terrain's tilt and the leans of speeding up, braking and turning, per `Body`
kind; the swell and hover bob; submarines drawn at their `WaterLine` and ships a little
in the water (their draft), whole models tinted
as under water by `FLAG_SUBMERGED`). Jolts rock every unit but a building on a lively spring
of their own (`Jolt`, `Scene::jolt`, fed each tick's events after `after_tick`): a gun's
`WeaponFired` kicks its unit back from where it fired, as hard as the shot's damage over the
unit's health (its weight: a Krogoth shrugs off what rocks a Stumpy), shells most, plasma less,
missiles a little, lasers and bombs not at all; a big `Explosion` (a wide area and heavy damage,
the tick's biggest few) shoves the units drawn lately round it away from it, the nearer the
harder. Aircraft in flight ride both out. Walkers step with the ground they cover (`scene/walk.rs`; turning on the spot they step round,
as far as their feet are carried round;
TA's strides cover a fraction of it and its poses hold from keyframe to keyframe, which read
as gliding up close): each walking type's gait is read once, the first time one is drawn, from
its own script run in a sandbox at millisecond steps (`Create`, then `StartMoving`; every
change to a piece recorded in exact time, turns at a speed reckoned from the target and speed
read back from a copy run on; the cycle where the changes repeat), less the channels anything
that aims or fires moves, and a second gait recorded aiming where the script walks otherwise
then (`walklegs`). Its stride is measured on the model: the planted foot is the fastest-going
back of the feet down at the ground, and the cycle is laid out in a table of poses by the
ground that foot covers. A walker big enough on screen is posed from the table by the distance
it covers (blending in as it sets off, out as it stops, at most twice its script's pace or
three cycles a second); the simulation's pieces are untouched. `TA_WALK_SYNC=0` turns it off.
Guns that point themselves kick back as far as they run clear (`scene/recoil.rs`). TA's fire
scripts slide a gun back in the frame of the piece it hangs from, level whatever the gun's own
pitch. Most guns hang from a sleeve their aim pitches or a turret it turns, and so kick back along
themselves; a gun its aim turns itself kicks back out of line with itself, and TA's camera above
never showed what that does: the Core Raider's barrel, pitched up inside a sleeve no script turns,
came up through the sleeve's top as it fired; the Triton's through its turret's. Such a gun (a
piece the fire functions move that the aim functions turn themselves, turned 8 degrees or more out
of its kick's line) is drawn kicked back only as far as it crosses no face, as drawn, of what it is
mounted in (the pieces held fast to the one it hangs from, up to the first a script turns) that it
was clear of where the kick started: faces it backs into head on (within 60 degrees) aside, and
those it was clearly through, its way in (a graze may stay where it was). Along the script's line
while that runs clear for the whole kick, else along the gun as it points if that runs clear
further. Reckoned once for all the units of a type, per whole degree of the gun's own turns, and
only for units big enough on screen; the simulation's pieces are untouched (a shell leaves the
muzzle before the kick). `TA_RECOIL_ROOM=0` turns it off; `cargo test --release -p ta_game
recoil_room_report -- --ignored --nocapture` lists those guns' kicks as drawn, and the animation
scan's `recoil_scan` what every unit's recoils pass through as the scripts move them.
Big walkers stand level (`ta_sim::defs::big_walker`: a kbot, a Commander or a spider 60 px tall
or more; the simulation tilts them no more than `Upright=1` units, so their guns fire from where
they are drawn): those that were not upright (the mods' giants: TA: Enhanced's SpiderBot and
XBot Fury, TA: Escalation's Juggernaut, Total Mayhem's Wolf...) have their feet brought to the
ground as drawn (`scene/footing.rs`): each foot, a piece whose faces come down to the ground at
rest, goes up or down by how far the ground under it lies from the ground under the walker's
middle, its leg bending to reach there (the hip swings and the knee bends, two bones of inverse
kinematics on the leg's own pieces, the knee kept to the side it bends to, the foot to the
attitude its walk gives it; a strut swings, a foot hung straight under the body is lifted), at
most 35% of the leg's length. (`cargo test --release -p ta_sim --test big_walkers -- --ignored
--nocapture` lists every walker of the game or of a mod, `TA_TEST_MOD`, against the rule;
`footing::tests::feet_on_rough_ground` measures how far their feet are off the ground.)
Walkers leave footprints on dry ground (the ambient effects, `effects_ambient.rs`, fading over 45 s as
the tread marks do): each walking type's feet (`scene::walks`) are measured on its model when the
scene loads it (`scene/feet.rs`: the faces' vertices within two units of the model's origin, which
stands on the ground, clustered by face and by touching; the pads that hang from one leg are a
foot, a Krogoth's heel and toes; effect pieces, flares and flashes, never count, nor do the pieces
its script hides once it is built, `still::standing_pose`, TA: Escalation's Commanders carrying the
body they are upgraded to, nor faces lying flat on the ground in a piece with no face rising from
it, plates: the same Commanders' 108-unit `flip`, measured as their one foot, left a black band
behind them), and every stride the next foot comes down where it stands under
the walker, drawn as the rounded rectangle its sole is (a picture of the nearest of nine
proportions, `effects_ambient::FOOT_SHAPES`, its width the sole's), turned the way the step goes.
A walker's gait (`effects_ambient::Gait`) keeps its prints to soles it could stand on (twice its
footprint either way at most, TA: Enhanced's units, which name no footprint, measured against
their movement class's; else a pair from its footprint), and lengthens its step where a foot's
prints are long, so that no print lies over another of the same foot and each darkens the ground
once. Units gone at a tick stay one
tick as `Remnant`s (as last drawn) for `effects_remains.rs`. Each unit's travel (distance
driven, angle turned) goes to the renderer per instance, and how fast its treads run in its
flags; faces painted with a track (`Tredside*`) carry their tread rate and side per vertex, and
`unit.wgsl` runs their texture by it. `hud.rs` is the egui HUD with portraits of the units
(`ta_render::portrait`: each type's model drawn once per player's colour and size, posed as it
stands finished, `scene::standing_pose`, by the battle's own shader in a studio's light, on a
clear ground the HUD sets on a gunmetal backdrop; TA's `unitpics` with the classic panels, for
players' pictures and while a portrait is not drawn yet), order and state buttons, a hover card per unit (after the pointer rests;
more after longer; under its health, a finished unit of ours or an ally's shows what it makes and
uses of metal and energy a second now, `Sim::unit_flows`), the time left over what is being built
(`hud/eta.rs`: from the builders working on it, scaled by how the work has really gone, so that a
stall or a builder joining shows; our own and our allies' only, not far out nor in photo mode),
the selection's routes (for a moment after an order, `Game::last_order`,
and while Shift is held), what sensors reach (`hud/ranges.rs`: for the selected buildings and units that
only sense, the building being placed and a unit the pointer rests on: a radar's disc shading up to its rim
with a beam sweeping round, a sonar's with pings, a jammer's dashed rim, the unit's own sight thin and
dashed beside them, each captioned, out of the view's reach beside the unit; the colours are the sight
overlay's, and the minimap shows the same discs, a radar's whole coverage at a glance; a unit switched
off dims them and says OFF; `--demo radar`, `TA_RADAR_UNIT`, `TA_RADAR_OFF`), the
acknowledgements of orders (`Game::order_marks`, pushed by
`Game::mark_order` where orders are given by clicks) and the minimap (fog overlay, alert
pings). The top bar's economy (`hud/economy.rs`) reads each store as a `Store` with its
`StoreState` (stalling from the economy's `build_factor`); `hud/deck.rs` shows a constructor's
build menu by `Purpose` on tabs (`Hud::build_tab`).

* **Version, logs and crashes**: both programs show the version with the commit they were
  built from ("1.0.0 (c752ef7fe)", `TA_BUILD_COMMIT` from the build script), in the main
  menu's corner and the launcher's About page, and write it first into their logs
  (`logs\game.log`, `logs\launcher.log` beside the settings). A panic writes
  `logs\crash.txt` (the launcher `launcher-crash.txt`) and says so in a message box: the
  message, the thread, the address the executable was loaded at and every frame with its
  address. Installed programs carry no symbols, so their frames read `<unknown>`; the
  build script keeps each release's `.pdb` files in `TA-Reborn-<version>-symbols.zip`,
  and address minus load address is the offset to look up there.
* **The title menu's film** (`backdrop.rs`; Settings · Interface · MENU BACKDROP, FILM or
  BATTLE): behind the title menu a film of short scenes, each staged live on one of TA Reborn's
  worlds, a world a visit to the menu (`Plan::choose`, `worlds_in_turn`: the one after the
  world shown last; in a season with scenes of its own, Halloween, Christmas, the New Year, the
  launcher's seasonal setting's, its world on the first visit, its scene first, its scenes
  joining their world's own). The scenes are `apps/ta_game/backdrop/*.toml` in `--demo stage`'s
  format (`stage.rs`; each plays alone with `--demo stage`), listed in `SHOTS` with their world,
  caption (translatable), length, season and whether the world opens on them. The loading
  worker builds the battle from the world's scenes (`Plan::setup`: the first scene's, players
  enough for them all, watched by nobody) and puts the first scene's cast down
  (`Film::begin`); the menu's frames then run the scene's orders and events on its own clock
  (`StageRun`, whose commands go in as the director's, `Game::direct`, the battle being only
  watched), and at its end cut to the next (`Film::after_update`, shuffled, the opener
  first, never the same twice running): the screen fades through black (`Showing::black`,
  drawn under the menus), the last scene's fires are put out (`Sim::extinguish_fires`), its
  wrecks and heaps cleared (every feature made since the world was loaded), its effects,
  particles and world sounds dropped (`Effects::clear_battle`, `GameAudio::cut_scene`, which
  leaves a narration, the interface and the wind alone), the next scene's cast put down in its
  weather (`Effects::scene_weather`: its rain, else the world's own, timed from the scene's
  start whenever in the battle it comes) with its moon. The camera is the scene's free camera
  (`StageRun::free_view`), the picture shifted right of the title menu by a lens shift
  (`Camera::view_offset`, half the room `menu::title_column_right` leaves), its lens focused on
  what it looks at (a gentle depth of field, `backdrop::lens`), the eye leaning a little
  towards the pointer, still looking at the same place (`lean`); with reduced motion each
  scene holds its first view. The film's battles play under the menu's music
  (`GameAudio::scene_level`, `SOUND_LEVEL` of the effects) and step back only a little behind
  the title menu (`Post::backdrop` at `TITLE_BACKDROP`), fully behind its pages. A caption
  names the world and the moment as each scene comes up (bottom right, a rule before the
  world's name). Left alone on the title menu 45 seconds, or with WATCH in its footer, the
  menu steps aside (it fades out first, the footer and the profile's card with it), black
  bars come in for 2.39:1, the picture centres and the caption moves into the lower bar, a
  hint at first how to come back; any key, click, wheel or move of the pointer (after a
  moment's grace) brings the menu back (`watch_menu_film`). The battle of old plays instead
  (`attract_setup`, fast-forwarded into its fighting) when chosen, or when no world can play
  (its map missing, or a mod without the scenes' units, `prune_missing`). Changing the setting
  on the menu brings the new backdrop in at once (`RestoreMenuBackdrop`). Captures:
  `TA_BACKDROP=<world>/<scene>` (`highlandrift/crossing`; a world alone: its opener),
  `TA_BACKDROP_WATCH=1`, `TA_BACKDROP_DIR=<dir>` (scenes read from a folder, while writing
  them); `TA_ATTRACT_MAP` plays the battle of old.
* **Archives** (the title menu; `--open archives`): the unit encyclopaedia and the credits.
  The encyclopaedia (`encyclopaedia.rs`, `--open units`) lists every unit type with a side and
  a name, mods' too, by side and name (filters: side and `Kind` by locomotion and TA's KBOT
  category, each choice with its count, and a search; the arrow keys, Page Up/Down, Home and End
  step along it, Enter or Down in the search picks its first find), with the F1 page's facts
  (`hud::info::head`, `facts`), where the unit comes from (`Vfs::origin` of its FBI: the game,
  The Core Contingency, Battle Tactics or a mod), and links along the build chain: the units
  that build it (BUILT BY) and those it builds (BUILDS), each opening that unit (its filters let
  go if they hide it). Units are shown in their side's colours (`showroom::side_colour`: the
  Arm blue, the Core red, a mod's side one of TA's other colours), in the list's portraits and
  on the stage. The unit stands in a TA world behind the page (`showroom.rs`): the menu's
  backdrop (its film or its battle) gives way to a watched battle with one player and no one else on a map of
  the chosen planet (`worlds`: one per planet), the unit on a level stretch near the middle
  (ships at sea; a ship on a dry world moves to a sea world), every feature in a wide circle
  round it cleared (`Sim::clear_features`), switched on, its stores kept full and its own shots
  harmless to it (`Cheat::Unlimited`, `Invulnerable`). The bar over the stage sets it doing
  something (`showroom::Act`: STAND; MOVE, a patrol to and fro across the view; FIRE, its
  weapons at the ground across the view), turns the camera round it or not, and brings the view
  back to the front left, as TA's build pictures have it (`HOME`; the camera glides the nearer
  way round). The camera frames the unit in the stage the panels leave by a lens shift
  (`Camera::view_offset`; `Frame`), its focus on the unit's middle, which `clamp_camera` leaves
  alone there. The deck along the foot picks the world (arrows or a list) and the time of day.
  WALLPAPER opens its preview: the panels give way to the picture's frame (`preview_rect`, the
  wallpaper's shape over its bar, the screen darkened round it), the camera stops turning (and
  comes round to the front if it was turning on its own) and frames the unit as the wallpaper
  will (`Frame::wallpaper`, `wallpaper_area`: right of the words, or in the middle without
  them), and the words are laid out over it as they will be (`wallpaper_words`, at the
  preview's scale); dragging and the wheel still move the camera. The bar picks the size (the
  screen's own first where it is none of 1080p, 1440p, 4K and 8K), the words or the picture
  alone (a vignette, `wallpaper_vignette`), the world and the light; SAVE WALLPAPER (Enter) draws
  it by `Renderer::render_image_with` with `wallpaper_camera`: the preview's eye, its lens
  narrowed to the preview's share of the window, so the picture is the preview's drawn larger
  (a test holds it to that), its words by an egui of their own at 1080 points; it is saved in
  the pictures folder's Wallpapers, and the notice opens it or its folder. Captures: `TA_UNIT`,
  `TA_UNIT_ACT=move|fire`, `TA_WORLD=<planet>`, `TA_WORLD_TIME=0..3`,
  `TA_WALLPAPER=1080p|1440p|4K|8K|screen` (opens the preview, saves at frame 150 with
  `TA_PICTURES`; `TA_WALLPAPER_SAVE=0` only opens it, `TA_WALLPAPER_WORDS=0` leaves the words
  off).
* **Space scenes** (`space.rs`, `--open space`, `TA_SPACE=<scene.toml>`; `Screen::Space`, its
  frames `State::space_frame`): the films' shots in space, TA's intro movie's opening made in the
  game. A TOML scene (its format at the top of `space.rs`) names the sky (stars, nebulae,
  galaxies), the sun, planets and moons (a TA world each, `planet::World`, with air, clouds,
  sea colours, spin), asteroids (one of the 3D rocks, cratered) and fields of them, the camera's
  keys (a curve through them, Catmull-Rom in time, eased at its ends or not; looking at a point
  or by yaw, pitch and roll) and fades. Its clock is the capture's (`TA_CAPTURE_FPS`), so
  `--screenshot`, `TA_SHOTS` and `scratch/trailer/film.py` film it as a battle; by hand Escape
  leaves, Space pauses, the arrow keys step and the file is read again when it changes.
* **The Galactic War** (`timeline.rs`, `--open timeline`,
  `TA_TIMELINE=chronicle|archive|<campaign>,<n>`, shown whole): the story in three parts. The
  CHRONICLE (`lore::CHAPTERS`, TA Reborn's own words, in five `ERAS`) runs along a ribbon of
  chapters over a painting of the galaxy as each chapter tells it stood (`timeline/galaxy.rs`,
  from the chapter's `lore::Sky`: CORE's space round the middle, the ARM's share of the rim,
  the front, the burned worlds, the fleets, the marked worlds named, all painted in egui and
  eased from one chapter's to the next over `EASE`); its text is told a character at a time
  (`lore::told`) in the room the whole text takes, so nothing moves as it is told (the page as
  tall as its tallest chapter); PLAY THE CHRONICLE reads it through (`dwell`). The ARCHIVE
  (`timeline/archive.rs`, `lore::DOCUMENTS`): documents from both sides, each with its seal,
  heading, tilted stamp and `[[blacked-out]]` words; CORE's intercepts break up now and then.
  The CAMPAIGNS: a track for each campaign with a story (Battle Tactics' are left out), a node
  per mission, coloured by its world (`world_colour` of the mission's `planet`) with a band and
  its planet (`planet::Planets`) over each run on one kind of world. A mission's picture, goals
  and briefing are read from the install when it is shown; PLAY THIS MISSION opens the campaign
  page at it once the campaign has unlocked it. A chapter leads to the campaigns that fight it
  and back (`chapter_of`).
* **Credits** (`credits.rs`, `credits_show.rs`, `app/credits_play.rs`, the title screen's
  footer, `--open credits`): a film. `credits.rs` reads what the credits say: Total Annihilation's
  own credits screen in full (the texts of `guis/credits.gui`: the name, "Copyright 1997 Cavedog,
  Inc.", "Have a nice day"), the teams of The Core Contingency, Battle Tactics and the editor from
  the readmes in the install's folder (`readme_credits`: what follows the last "Credits" heading
  up to the legal small print, roles on a line of their own or before a colon; `install_root`),
  and TA Reborn's (`reborn`: the names in `ta_ui::credits`, which the launcher's About page shows
  too, the libraries by what they do, the tools; its last block, the disclaimer, is taken out by
  `disclaimer` for the finale). Nothing of the original is in the repository: it is read when the
  credits open, and a section whose readme is missing is left out. `credits_show.rs` packs the
  sections into cards (a title; roles and their names, up to eight lines; a few lines of words),
  deals the cards out to acts (`plan`: each act a scene of `apps/ta_game/credits/*.toml`, in
  `stage.rs`'s format with an `[act]` and `[[station]]`s, written by `tools/credits/acts.py`:
  a road of stations through a battle on one of TA Reborn's maps at its own time of day, the two
  players' armies drawn up along its flanks and firing across it, invulnerable so the fight
  never ends, clear of the road the cards lie on: an act's `road` is the widest a card is laid,
  `card_ground` the ground a card may cover, which a test keeps every unit off, where it ends up
  standing; nothing marches down the road), and makes each act's camera from its stations (`build_stage`: a key for each card
  to read it by, one of four rigs in turn, so each is framed by its own distance and angle, held
  still while it is read). The names are in the world (`world_text.rs`, `ta_render::world_text`):
  each card laid on the ground at its station, as big as its rig and its act's road let it be
  read, struck by a blast beside it that sets its letters burning in a ripple across it (embers
  that cool and keep their glow; `card_blasts`, whose scorch marks lie on no card's ground), or
  painted (lit as the ground is, shown by the blasts' and the searchlights' light at night), or
  projected as light, or built from the ground up as a nanolathe builds; hills hide letters as
  they hide anything on the ground, and a unit on a name has the name drawn over it, lighter
  (`ta_render::world_text`'s second pass). An act's world is made under a black chapter card
  (`Game::new`, as the showroom does, the card standing on the screen meanwhile). The film plays
  to its song, Hadal Static's "The War Reborn" (the launch trailer's; `credits_song.rs`;
  `assets/reborn/music/the_war_reborn.ogg`, © Hadal Static, used with permission, not under
  the GPL: `NOTICE.txt` beside it). Its cue sheet `apps/ta_game/credits/song.toml`, written by
  `tools/credits/song.py` from the band's master, has every bar line, the song's moments (the
  band coming in, the break and the drop, the loudest bar, the flash, the dead stop) and its
  arrangements: the film is about twice the song, so it is made longer on bar lines, spans of
  it played in turn (`ta_audio::Arrangement`, a score: `MusicPlayer::play_score`, its stream
  `stream::spawn_arranged` joining them with a short crossfade before each join), each join a
  jump back between two bars whose music matches (checked at the seam against the song's own
  bar changes, and aligned to the sample on the downbeat's kick). `plan` lays the film on the
  shortest arrangement that holds it: each card from a bar line for whole bars (its reading time,
  rounded), each act coming in a bar before its first card, three bars of black between acts
  (`lay_out`, a dynamic programme, gives out the bars spare so that a card or an act lands on
  each drop after a break: the battle goes up either side, `hit_blasts`); the first act's
  picture comes in with the song's first note and its first card burns in on the band coming
  in; the finale's last card is read to the loudest bar, where the war's last (nuclear) blast
  goes off on the horizon, the emblem burns into the ground on the flash with the disclaimer
  over it, and the picture is cut to black on the song's dead stop; the last word is TA's, in
  the silence after (FROM THE TOP plays it again). The film follows the song's clock as the
  mixer plays it (`Show::follow`, `AudioEngine::score_position`), not its frames: an act whose
  world took long to make comes in where the song is, its battle moved on to it a step at a time
  (`Show::catch_up`: each blast on its own time among the steps, what is long past by then seen
  but not heard, a blast whose fire would be out left out); on a capture's
  fixed clock, or with no sound, it keeps its own time, and a sound log (`TA_SOUND_LOG`) writes
  the song's spans down as music at their exact times (`sound_log::ScoreLog`). The menu's music
  steps aside as the credits open and comes back as they close. Escape (or BACK) leaves, the
  right arrow skips to the next act (the song from its place); with reduced motion the camera
  cuts from card to card and holds still. Captures:
  `TA_CREDITS_ACT=<n>` starts at act n (the song from its place), `TA_CREDITS_DIR=<folder>` reads
  the scenes and the cue sheet from that folder (no rebuild while they are written).
* **Opening files**: a replay (`.tarep`) or saved game (`.tasav`) named on the command line
  (what Explorer passes for the file types the installer registers) opens straight away,
  without the intro; one that can't be read (another version's, say) says why in a message
  box and leaves the player at the main menu.
* **Look** (`ta_ui`, shared with the launcher): Total Annihilation's own, after its
  screens (the battle room, the load game dialog, the main menu, the in-game buttons):
  grimy dark metal, steel pipe frames, olive-black wells, gunmetal buttons with cream
  stencil lettering, green lamps; brass for the main action and focus; Bahnschrift read
  from the Windows font folder. `theme` holds the palette (`AMBER` and its kin are the
  brass: the names stayed), the spacing scale (`space`) and the egui style (`install` for
  the launcher's roomy one; the game's `install_compact` keeps the HUD, tooltips and
  drop-downs dense). `metal` paints the materials, as shapes so that panels can put them
  behind contents laid out first (`Painter::set` on a `Shape::Noop`): a generated grime
  texture that tiles, anchored to the screen, over every `plate`; `backdrop` (TA's tiled
  plates, browner and bluer, with seams and etched traces); `pipe_frame` (pipes whose ends
  run past the corners into bolted joints, collars on long runs; `pipe_inset` says where
  the contents may start), `pipe_rule`, `bezel`, `well`, `seam`; `slab` (gunmetal, brass,
  rust or a dark slot, bevelled and scuffed; pressed ones light up as TA's do) and
  `arrow_slab`; `lamp`; and the lettering: `embossed` (outlined and shadowed as TA's
  titles are), `slab_text` and `chrome` (the title's polished steel, its bands placed from
  the font's capitals). `widgets` has the buttons, cards (a plate in a thin pipe frame;
  flagged ones get their top joints plated in the colour and a lit lamp), switches (a gunmetal knob
  uncovering a green lamp), segmented selectors and the navigation (a lamp lit on the page
  shown); `panel` the chrome: `Panel` (a plate in a pipe frame; header with an outlined
  title, subtitle, back or close, over a pipe rule capped by joints in its accent; its contents
  use the roomy style; with `max_height` its body scrolls, and `show_with_footer` /
  `modal_with_footer` keep a footer, a dialog's buttons, in sight under it), shown as a `page` over the front end's live battle (which the menu
  dims behind its pages with `page_backdrop`, lighter than a modal's, crossfading with the
  title's `vignette`; the renderer grades it down too, `Post::backdrop`) or as a `modal`
  (dimmed backdrop, input blocked), plus the title menu's entries (dark slots in steel
  with a lamp), list rows, key caps, the busy bar (lamps lighting in turn) and the big
  call-to-action button. Every front-end
  page (title, skirmish, campaign, records, multiplayer, lobby) and battle dialog (pause,
  options, statistics, controls, victory/defeat) is built from them; `--open <page>`
  (records, multiplayer, options, `options-<category>`, `options-search` with
  `TA_SETTINGS_SEARCH`; in a battle pause, stats, controls, victory, defeat) opens one for
  captures.
* **Moving between pages** (`ta_ui::panel`): a page slides in from the side the player
  went (`set_direction`: the menu compares the pages' depths, deeper from the right, back
  from the left) as it fades in, over 0.24 s; a dialog rises into place growing from 96%
  as the backdrop darkens (`entrance` restarts whenever something was not drawn the frame
  before). The title menu's entries build up one after another (`stagger`). Escape goes
  back from pages with a BACK button, unless it only left a text field (egui drops the
  focus first, so the panel remembers whether the last frame was typing) or closed a
  drop-down; it closes the options and the controls reference, and the pause menu
  ignores it while one of those is open. The arrow keys focus the title's first entry when
  nothing has the keyboard, move between entries, and Enter picks one.
* **Interface sounds** (`ta_ui::sound`): widgets ask for a sound when used (one a frame,
  the last asked for wins: a title entry's own over the button's); the game plays TA's
  sound for the event in `gamedata/allsound.tdf` (`ta_audio::UiSounds`): BIGBUTTON for
  title entries and the call to action, SMLBUTTON for buttons, SMALLBUTTON for switches,
  segments and list rows, PREVIOUS for back, MORE for tabs, and SKIRMISH, MULTI, OPTIONS,
  EXIT for those title entries; SLIDER as a slider moves (`value_slider`, at most a dozen a
  second), PANEL as a dialog opens (`panel::modal_shown`, on its first frame shown),
  SKIRMISHCHEAT (TA's "can't do") for a refusal or a failure, ENDGAMESTATBAR for something
  done. The launcher uses the same events while its sound setting is enabled
  (`sound::enable`), its notices heard as they come by their colour (red a failure, green
  something done, amber a warning).
* **Settings** (`ta_ui::settings`, shared with the launcher's Settings page): categories
  down the side (Gameplay, Controls, Interface, Graphics, Display, Sound) under a search
  that matches every setting's name, description and keywords, in English and translated
  (keys by what they do and by their key); results come under their category's heading,
  which opens it. Each setting is a row: name and description on the left, the control on
  the right. RESTORE DEFAULTS resets the shown category (the name and the language stay).
  In the game (the title's and the pause menu's OPTIONS) changes apply at once
  (`SettingsView::show` returns `Changes`; `State::apply_settings`): volumes, the renderer's
  effects, the window's mode and size and vertical sync, the camera; antialiasing when the
  game next starts. The frame rate limit (vertical sync off) waits in `about_to_wait`.
  Gameplay settings: single-player battles start at the game speed (not replays or the
  lessons), the skirmish page's unit limit starts at the setting.

* **Graphics levels** (`ta_config::Preset`): Low, Medium, High and Ultra set the options
  together (Low: no extra passes, the scene at 75% of the window's resolution through
  `Renderer::set_render_scale`, the interface drawn over it at full resolution; the composite
  scales a smaller scene up through a Catmull-Rom cubic kept within the four texels round it,
  `upscaled` in post.wgsl). On integrated graphics a level draws the battlefield at no more
  pixels than its budget (`Preset::pixel_budget`: 1080p's, 1440p's at Ultra; twice that on
  Apple's Pro chips, four times on its Max chips, eight on its Ultras):
  `Graphics::fit_render_scale` lowers its render scale on a screen of more pixels, a Mac's
  Retina screen above all (2.7 to 3.7 times 1080p's), when the level is picked (the options know the graphics' kind, `settings::Host::gpu_kind`; the
  launcher from `Hardware::detect`, which on a Mac asks `sysctl` for Apple's chip), and once
  for a settings file as the game starts with its window and device
  (`Graphics::fit_render_scale_once`, `graphics.scale_fitted`; where the scale is still the
  level's own: the launcher picks the first level before the game has a window, and a Mac's
  screen is not known to it). A render scale under the level's own still matches the level. A new
  settings file starts at Auto: the game opens the device first (`Gpu::new`), picks a level
  from its kind and name (`Preset::for_gpu`: dedicated cards High, recent integrated
  graphics Medium, older ones and software renderers Low), saves it with the card's name,
  and only then builds the renderer (`Renderer::with_gpu`, `RendererOptions::msaa`: the
  largest sample count up to the setting that the device supports). Settings files from
  before levels existed read as Custom and keep their options. The launcher and the in-game
  options show the level the options match. 2x and 8x antialiasing need the device's
  adapter-specific format features (requested when offered); otherwise 4x is the most.

* **A Mac's full screen** (`macos.rs`): the game's full screen is macOS's own (a space of its
  own, winit's `Fullscreen::Borderless`), where the menu bar comes down when the pointer
  touches the top of the screen and the Dock comes up at its edge, over the game, and the
  pointer over them leaves the window (edge scrolling stopped). While the window is in full
  screen the game hides both outright through AppKit's presentation options (`objc2-app-kit`),
  set whenever the window comes into view, takes the keyboard or changes size
  (`WindowEvent::Occluded`, `Focused`, `Resized`), as winit's later "borderless game" option
  does on coming into view: AppKit's own options return when
  the player comes back to the space, and options set before the space opens (as winit 0.30's
  `with_borderless_game` sets them) are those the system restores when it closes, leaving a
  window without its menu bar. Out of full screen, any hiding left over is undone.

* **Colour-blind colours** (`gameplay.colour_blind`; `palette.rs`): player colours from a
  table after Okabe and Ito's palette (TA's red slot orange, green bluish green...),
  health bars blue-yellow-vermillion, the movement map and placement glows blue and
  orange. Units follow: with `detail.w` set, `unit.wgsl` draws a team texture from the
  white team's frame (its greys) tinted with the same table (`team_cb`). Purely what this
  player sees: games, replays and other players are unaffected.

* **Gamepads** (`gamepad.rs`, gilrs; `gameplay.gamepad`): each frame the pad becomes mouse
  and keyboard input before anything reads it. The right stick moves a cursor (from where
  the mouse is; the system cursor follows where the platform allows), A and B are the left
  and right buttons (given to `Input` unless over the interface, and to egui as pointer
  events either way, so menus work), the left stick adds to the arrow keys' pan, the
  triggers to the mouse wheel (zoom towards the cursor), the bumpers hold Shift and Ctrl,
  the d-pad strikes the group keys 1-4, Start Escape, and the other buttons strike actions
  by name (`Input::pad_action`, which `Bindings::pressed` honours whatever the keys are
  bound to). The F1 reference lists the layout (`gamepad::LAYOUT`).

* **Mouse buttons** (`gameplay.order_button`): the right button gives orders (the
  default), or the left one, as in TA's original interface. Then a click on our own unit
  selects it (a nanoframe, or a factory at work, with constructors selected is helped
  instead, unless Shift is held, `Game::clicks_select`), anywhere else it gives the selection the context order
  (`Game::context_order`, the right button's otherwise), and a right-click clears the
  selection or cancels a pending order. Ctrl+drag with the order button draws formations
  (Ctrl+right-drag orbits with left-button orders). The cursor, the F1 reference
  (`{order_click}`, `{order_drag}`, `{deselect}`), the tips and TA's side panel follow.
  `--demo clicks` clicks through both schemes and prints what happened.
  **On one of our units the right button opens its radial menu** (`gameplay.radial_menu`, on by
  default; `radial::Click::opens_menu`) where the click would not give an order: with the right
  button giving orders, when nothing is selected or the unit is in the selection (a move to where
  it stands); with the left button giving orders, where the right one only clears the selection,
  whatever is selected (a click elsewhere still clears it). Never over an order waiting for its
  click or a building being placed (the click cancels those). Holding the button on one of our
  units, the pointer still for `radial::HOLD` (0.32 s) opens it in every case (`hold_opens`; Ctrl
  and any movement leave the formation line and the camera's orbit alone). The player can keep
  the menu to the hold (`gameplay.radial_opens = "Hold"`, RADIAL MENU OPENS WITH in Controls):
  then a right-click always does what the button did before. A middle click on one of our units
  (the button let go where it went down; a middle-drag pans) opens it either way, and a middle
  click on the open menu is the right button's in a menu not held open (back, a queued unit
  fewer). Watching (spectators, replays) any unit in sight has one: see through its eyes, follow
  it, read about it. A pad's B is the right button, so a tap or a hold does the same. The tips
  and the F1 reference say what the setting does (`tips::When`, `controls::radial_rows`).
  **Settings files**: the menu has been on by default since it came, so a file without
  `radial_menu` (written before it) has it on, a `true` is the default written out, and only a
  `false` is the player's own choice, kept through every load and save; a file without
  `radial_opens` opens it on a click or a hold, as it always did.
  What a click does to a unit under the pointer is worked out in one place
  (`game/clicks.rs`, `click_on`: land on a pad, load, attack, or help: repair or help build,
  guard, help a factory's production), which the click executes and the cursor reads, so
  the two cannot disagree. `--demo assist` (`TA_ASSIST_LEFT=1`) puts the Commander over a
  lab at work, with a constructor guarding it, and prints the cursor and the orders.
* **Scrolling**: the arrow keys, and the screen's edges (`gameplay.edge_scroll`: the
  cursor within 3 px of one, the window focused and the cursor in it; never in captures),
  at the scroll speed setting. The classic camera stops at the map's edges as TA's did (the
  part of the screen the interface leaves free shows the map up to its last column and rows
  TA showed, a map smaller than it centred); the perspective camera's focus stays on the
  map, and its eye above the ground: where the ground would reach it the eye rises, looking
  down a little more, just enough (`Camera::lift`, the player's pitch kept), and eases back
  down as the ground falls away.
* **Input**: order, selection and game keys are rebindable (`ta_config::Keys::ACTIONS`,
  stored as `[keys]` in config.toml, edited in the options and the launcher; parsed by
  `keys.rs`; the F1 reference shows them as bound, and CHANGE KEYS opens the options at
  Controls). Defaults: context right-click (attack, repair, guard,
  load, reclaim, move; aircraft on a repair pad land there). TA's own orders on TA's own keys
  (its GUI files' quick keys): M move, A attack, P patrol, G guard, S stop, D D-gun, C capture,
  R repair, L load, U unload, E reclaim, K cloak, F fire orders, V move orders, X on and off;
  TA Reborn's additions on Alt and the letter they would have taken: Alt+F fight, Alt+R
  resurrect, Alt+X extractors over an area, Alt+L the sight map, Alt+D the constructors' duty
  (see Duties). Ctrl+D self-destructs.
  Selecting: Ctrl+0-9 assigns a control group, 0-9 (or TA's Alt+0-9) recalls it (Shift adds,
  a double tap centres the camera), and a factory in a group puts what it builds there (TA
  3.1's squads); double-click selects every unit of that type on screen; Ctrl+A everything,
  Ctrl+C the Commander, Ctrl+Z every unit of the selected types, Ctrl+S everything on screen;
  TA's selections by category: Ctrl+B, F, M, P, R, V, W pick the units whose category says
  CTRL_B... (constructors, factories, mines, armed aircraft, radar, aircraft, armed units: the
  game data decides, so mods' units join in), and a Ctrl+letter no key uses picks a mod's own
  CTRL_ category. Ctrl+click with the order button is TA's force fire: the armed units attack
  whatever is under the cursor, ours included, or the ground (the cursor shows it while Ctrl
  is held); a Ctrl+drag draws a formation line instead. With orders on the right button,
  Ctrl+click with the left takes every unit of that type on screen when it is on one of ours,
  and force-fires elsewhere. Shift over one of our constructors shows every constructor's
  planned buildings, over a unit that cloaks the circle an enemy uncovers it in. `.` and `,`
  cycle idle constructors and factories; Backspace looks at the last attack; Ctrl+F6-F9 store
  camera positions, F6-F9 recall them; Enter chats (Shift+Enter to allies); H shares
  resources and units with an ally; the backtick steps through when health bars show; Space
  or TA's Pause key pauses; Ctrl+Shift+S quick-saves. TA's function keys: F1 tells about the
  selected unit or the one under the pointer (`Game::unit_info`, drawn by `hud/info.rs`: cost,
  build time by its usual builder, health, movement, sensors, weapons, economy, abilities;
  with nothing to tell about, the controls reference), F2 opens the menu, F3 looks at the
  last attack (Backspace too); the map overlays are on Alt+F2 (movement), Alt+F3 (heights),
  F4 (metal) and Alt+L (sight). N looks at the next of our units off the screen, in turn (of
  the selection, if any); Delete clears the messages and the chat (TA's F12, which here takes
  a screenshot). Dragging while placing a building lays
  out a row of them. T follows a unit, Alt+click pings.
* **Models** are drawn with back faces culled, as TA did (its 3DO models are consistently
  wound; drawn double-sided, the undersides of ground plates and the insides of shells
  showed as black or wrongly textured faces). Shadows and ambient occlusion still use both
  sides. So a piece whose only face is a flat untextured quad facing down is not drawn at all
  when it is the root (the Core Mine's unflagged selection plate) or lies on the ground (TA:
  Escalation's Commanders' `flip`, 108 units square: its shadow fell in black wedges where the
  ground dips and its contact occlusion darkened a square round them), `unflagged_ground_plate`.
* **Checking every unit**: `--demo gallery` puts every unit type on show, finished and
  switched on (`TA_GALLERY=<page>`, `TA_GALLERY_SIZE`, `TA_GALLERY_UNITS=a,b`);
  `TA_GALLERY_ACTION=produce|patrol|build` sets them working; `TA_GALLERY_TOUR=<dir>`
  photographs each one close up and exits. The `unit_scripts` test runs every script
  through a unit's life and reports script-engine warnings and pieces flung off. The
  `animation_scan` test plays every unit through its animations (built, switched off and
  on, moving, aiming at four headings, building or producing) and judges the pieces against
  the model's geometry: carried across its middle by moves, far out of its footprint, into
  their mirror-image partner, through each other, a hull into the ground, a turret not
  pointing where it was told. It ranks the units and fails on what isn't TA's own; its
  `script_conventions` re-derives the sign of each script axis from the scripts
  (FORMATS.md), and its `recoil_scan` fires every unit's guns near and far, every tick, and
  lists what their recoils pass through as the scripts move them (aims leave guns' recoils
  out of the main scan: a barrel sliding into its turret is what a recoil is).
* **TA's art, enhanced while loading** (nothing is shipped): upright feature sprites
  (trees, rocks) keep the distance to their outline in alpha (`add_gaf_outlined`, pushed a
  third of a texel out so one-texel twigs survive), so their edges stay smooth up close and
  are anti-aliased by alpha to coverage with MSAA; unit textures smaller than a layer are
  doubled with Scale2x on their palette indices (`model.rs`) instead of blown up into
  blocks, after a stray pinkish edge row many of them have is given the colours inside it
  (`mend_edges`: faces map their texture edge to edge, so it showed as a line). The modern HUD's order and state buttons carry die-stamped icons (`hud_icons.rs`,
  original art drawn by `tools/hud/icons.py` into `apps/ta_game/hud/icons.png`: one row of the
  atlas per pixel size, since egui's wgpu renderer has no mipmaps, the row nearest the size on
  screen drawn on whole pixels) on gunmetal, the order keys on keycaps under them, in full rows
  (`orders::rows`: one row for each kind of order); the command deck's plates and the top bar
  are tiled with the side's own
  panel metal, the tiles TA fills wide screens with (`hud_skin.rs`: `bitmaps/<side>guisidetile.pcx`,
  `<side>guitoptile.pcx`, cut from their sheets by the backdrop colour; a plate when a
  mod's side has none), with bevelled edges; the minimap and unit pictures sit in
  steel bezels.
  * **The top bar** (`Hud::top_bar`, `hud/economy.rs`) holds the economy: for metal and for
    energy a gauge of what is stored with the amount in it, what comes in (green) and what is
    spent (red when it is more), the warnings (STALLING, FULL, EMPTY IN n S) and, where the bar is
    wide (`economy::Tier::Wide`), a sparkline of the last three minutes; narrower bars give up the
    sparklines, then the wind, then the clock, in that order (`Tier`). A click on either opens the
    economy's panel under the bar (`Hud::economy_panel`): the charts (`EconomyTrend`: a sample each
    game second, the mean of that second's frames, three minutes kept, sampled in `Hud::draw`),
    the battle's tally and what is under construction; a click elsewhere closes it.
  * **The minimap** stands at the top right, under the top bar, in a plate of its own
    (`deck::minimap_layout`: as large as fits a box a quarter of the screen high, its shape the
    map's); what hangs at the right edge (the ally strip, the share and cheat panels, the cast's
    kill feed, the achievements' notices, the idle alerts' arrows) keeps below it
    (`Clearance::right_top`, `Hud::notices_room`).
  * **The command deck** (`hud/deck.rs`) stands along the foot of the screen in up to three
    plates, a quarter of the screen high (150 to 180 points), laid out by `deck_layout`: the
    forces at the left (`Hud::forces_plate`: the player's units by `Group`, a row each in one or
    two columns, the group the selection is lit; a click selects it, a double-click looks at it);
    the selection in the middle (only with a selection; at most 620 points, centred where it can
    be): its card (what it is doing, `activity`, read from its build
    job and first order; its health beside its picture; its rank), beside the kinds of a mixed
    selection and the switches that narrow it, its queue, or for one kind with nothing queued
    its figures at a glance (`info::summary_figures`, in the information page's table, F1 a
    click away); at the right, for a selection of ours, the command plate: ORDERS in rows by
    kind (`orders::available`: moving and fighting, work, the special ones, each with its key),
    beside them the STANCE (the standing orders as labelled buttons showing what each is set to,
    self-destruct last and apart; a column of glyphs where the room is short), and the build menu
    grouped by `Purpose` on tabs with the costs on the pictures, as many across as two rows
    need (three to nine; `BuildGrid::fit` sizes the pictures to the room, on pages when they do
    not all fit). Where the room is short the plates give up their words in turn until the
    selection's plate has what it wants (`MIDDLE_WANTED`): the stance to glyphs, then the forces'
    columns narrow. The other overlays keep above the deck (`Clearance`); the battlefield's own
    marks show between its plates.
* **Radial menu** (`radial.rs`, drawn by `hud/radial.rs`): the ring of a unit's choices round the
  pointer, for the mouse, the keys and a pad. What it offers is `radial::entries`, from the same
  list of orders as the command plate (`orders::available`) and the standing orders: a tower's
  attack, stop, Perspective, firing mode and self-destruct; a factory's queue (a ring of the units
  it can make, with their pictures and costs, a click queuing one, Shift five, a right-click one
  fewer), repeat, stop and the routes its units take; a constructor's build (its buildings by
  purpose, then the buildings), repair, reclaim, guard, its duty (a ring of the duties, `Menu::Duties`,
  the one the constructors are on lit); a fighter's move, attack, fight, patrol,
  stop and its fire and move orders; what does not fit the nine places of a ring goes on a page
  after it (MORE). Firing modes and the rest open rings of their own (the middle goes back), and
  self-destruct asks once more on a ring of its own (CANCEL or SELF-DESTRUCT; while the count runs
  it cancels at once). Each choice shows its key, and the key makes it. The menu's choices are the
  selection's, so opening it on a unit that is not selected selects it (in a selection, makes it
  the first). The pointer picks by direction (`radial::pick`: the first choice straight up, the
  rest clockwise, nothing at the middle or well away); held open, letting go over a choice
  makes it; opened by a click, a left click makes it, a right click goes back, then away, or
  takes a queued unit off (`radial::released` is the whole table, tested). A pad's left stick
  lights the choice it points at, the d-pad steps round the ring, A makes it, B (the right
  button) and Start (Escape) go back. It covers the screen's pointer while it is up
  (`Hud::radial_menu`), keeps inside the battlefield the interface leaves free (`Hud::free_view`;
  the cursor moves with it) and goes when its unit does, or when Perspective, photo mode or a
  dialog takes the screen.
  **How it looks** (`hud/radial/shape.rs` cuts the shapes, `hud/radial/paint.rs` paints them; the
  light is from the top left, as the icon atlas's): each sector is tessellated as a grid
  (`shape::Sector`, its sides parallel to the gap between neighbours, its corners round), so every
  vertex has its own light: a dome-shaded face with the interface's grain laid in its own shape, a
  chamfer lit on the sides that face the light, a shadow. The plate under the pointer (or the
  stick) is brass, risen over its neighbours, with a glow and its key lit; what is set (`Entry::lit`)
  is warmer with a lit line along its inner edge and a lamp; the danger tone (self-destruct) is rust;
  a choice that would do nothing now (`Entry::disabled`, with its reason as its hint: nothing to
  stop, nothing loaded) is dull and inert (keys and clicks leave it alone). Round the ring runs a
  steel rim with a rivet between the plates, lit over the plate that is lit; in the middle the
  unit's picture on a seat of steel with its health round it and its name (the selection's, with
  its health summed); in a ring opened from another, a small dial of the ring before with the plate
  it came from lit (`Radial::places`), the ring's name (`radial::ring_name`) and, where a ring has
  more than one page, a dot to each (`radial::paging`). A unit or building shows its cost as two
  figures in the deck's colours (`Entry::cost`), a queued count on a brass roundel, and what leads
  to another ring has an arrowhead after its word.
  **How it moves** (`radial_motion.rs`: no egui, no drawing; `hud/radial.rs` puts it on the screen):
  everything is a function of the seconds since something began, or a spring moved on by the exact
  solution of its equation (`Spring::step`), never of a count of frames, so a frame every 4 ms and
  every 33 ms draw the same pictures and a capture on `TA_CAPTURE_FPS` draws what the game does. The
  ring unfolds a plate at a time from the top (`coming_in`, `Change::Open`), each swinging in from
  before its place; the plate under the pointer or the stick rises on an underdamped spring, its key
  lit and the rim's lamp with it, and a bead of light rides the rim to wherever the choice has gone
  (`Anim::follow`; the short way round); the line of words under the ring slides and crosses over to
  the new words; a choice made presses its plate in, with a brief flash (a queued unit's count
  pops); one that would do nothing shakes. A ring opened from another unfolds from the plate that
  opened it, the nearest plates first, as the ring before folds into the middle (`Change::Into`,
  `going_out`; the ring before is kept as a `Scene` while it goes), and going back is the reverse
  into the plate it came from (`Change::Back`; a stick keeps its place there: its focus is on that
  plate); the pages of a long ring turn like a dial (`Change::Page`) with the dots of the hub
  following. The menu closes by folding into its middle while it drifts towards the pointer
  (`closing`, `closing_pull`: two fifths of the way, at most 70 points, and not when a stick or the
  keys drove it), or, when a choice was made, that plate swells, flares in a ring of light and goes
  last as the others let go: and the game has its pointer back from the first frame (the menu is
  let go of at once; what it showed is `View::fade`, drawn without taking input). With **reduced
  motion** (`ta_ui::motion::still`) nothing travels, swings, sinks, shakes, swells or slides: plates
  fade in a twelfth of a second, the words cross over, the health figure and the lights are there
  at once (`Timing::REDUCED`: `travel` is 0, and `plate_looks` and `going_looks` scale the press's
  sink, the shake and the count's pop by it), and a refusal is a rust tint of the plate in the place
  of the shake (`Look::warn`); with **reduced flashes** (`ta_ui::motion::reduced_flashes`) the ring of
  light, the press flash and the bead's halo are left out and the glow and the tint are dimmer
  (`paint::Space::calm`). The menu asks for frames only while something moves (`Anim::busy`).
  `TA_RADIAL=<unit type>` opens it on a unit in captures (`TA_RADIAL_AT`,
  `TA_RADIAL_RING=fire|move|more|build|buildings|units|confirm|duty`, `TA_RADIAL_HOVER=n`).
  **Adding a choice**: what a unit's ring holds is a list of `radial::Choices`
  functions for its `Role` (factory, defence, builder, fighter; the unit the menu is on says which),
  each an `Option<Entry>` that is `None` where the selection cannot make it, so a new choice is
  one function there and a line in the lists that have it (the ring pages itself past nine); one
  that opens a ring of its own adds a `radial::Menu` (its choices in `radial::entries`, the words
  for the middle in `radial::title`), one that does something adds an `radial::Id` done in
  `Game::radial_pick`.
* **Cursors** (`cursor.rs`): TA's animated cursors from `anims/cursors.gaf`, chosen by
  `Game::cursor_kind` (the pending order's, else what a right-click would do there, from
  `game/clicks.rs`: the wrench over what the selected constructors would repair, help build
  or assist, a factory's production included; the shield for standing guard; TA's air-strike
  sight in place of the attack sight when the units that would attack are all bombers,
  `clicks::bombers_only`); the system cursor is hidden in battle.
* **Classic interface** (`hud_classic.rs`, its data in `hud_classic_art.rs`; setting
  `gameplay.classic_interface`, hidden `--classic-ui` for captures): TA's own panels in
  place of ours, built from its interface data and scaled from its 640×480 layout to the
  window height. The side panel (PANELSIDE of the side's `intgaf`) holds the minimap and
  the selection's GUI page: the builder's build menu (`guis/<unit><page>.gui`, pages and
  places rebuilt with download entries by `Art::menu`), the orders page
  (`guis/<side>gen.gui`) or, with nothing selected, the main page (score, ALLIES, SHARE,
  OPTIONS). The resource and unit bars put their readouts where `gamedata/sidedata.tdf`
  says, in TA's fonts (`ta_formats::fnt`). Buttons call the same `Game` methods as the
  modern HUD (TA's MOVE, REPAIR and LOAD are `CommandMode`s of their own); fight,
  resurrect, factory repeat and self-destruct get plain buttons in the places a page
  leaves free. Wider windows repeat the bars' plain ends. Its art is made for the screen
  (`hud_classic_scale.rs`, on the player's own files, kept in memory), at the size each
  piece is drawn and on whole screen pixels (egui's renderer has no mipmaps): smooth by
  default, each picture doubled by directional cubic convolution (DCCI: a new pixel
  interpolated along the direction the picture varies least, a direction chosen only at an
  edge, never in the grain of TA's metal), resampled to its size and its edges steepened
  back to about a screen pixel (build pictures, renders, more gently); TA's one-bit letters
  doubled the same way as fields whose half-way line is their outline, drawn anti-aliased at
  the nearest quarter pixel (TA's even spacing at 2.25 screen pixels to one of its); the bars
  smooth ramps. ORIGINAL PIXELS (`gameplay.classic_pixels`; `TA_CLASSIC_PIXELS=0|1` for
  captures) keeps TA's square pixels instead, evenly sized (the screen pixels on a seam
  shared by area).
* **Time of day** (setting `graphics.time_of_day`: TA's day, dawn, dusk, night, or a day and a
  night every twenty minutes of play, from the game's clock so replays and rewinds see the
  same sky, `daylight::cycle_hours`: nights pass three times faster than days;
  `TA_TIME=<hours>` for captures): the renderer's time of day, and at night the lights of
  `night.rs`: a floodlight over every finished building (sized to it), a green glow over
  factories that are building, running lights on units (brighter on the Commander) and a
  red beacon blinking on aircraft, the lanterns, lit windows and decorated firs of TA Reborn's
  festive features (`rebornlight`, `FeatureView::night_lights`), all fading in as the sun sets;
  and over their racks of fireworks (`rebornfireworks`) shells climbing on trails of sparks and
  bursting in colours, their flashes lighting the snow (`effects_fireworks.rs`: seen, not
  simulated). The map editor keeps TA's
  light.
* **Battle highlights** (`highlights.rs`): `Game::tally` notes every unit destroyed (its cost
  in metal plus energy / 60, where and when), nuclear strikes and fallen Commanders (worth
  extra), from live ticks and catch-up alike, and every 600 ticks keeps a copy of the game:
  `Sim` and the `SkirmishAi`s are `Clone` (a copy plays on bit for bit: `ta_ai` test
  `snapshot`), and `World` shares its static terrain tables through `Arc`, so a copy is 2-3 MB
  and takes about a millisecond (a battle of 20,000 units: about 95 MB and 20 ms, its units
  copied on all cores; the pathfinder's scratch and the AI's remembered answers start afresh
  in a copy). Each copy is worth the losses 4 to 30 s after it; beyond a memory budget (160
  MB, 6 to 30 copies, but no more than 384 MB for the 6 and 2 at least) the least worth goes.
  `Tally::moments` picks the costliest fights (losses within 520 units and 15 s of the
  costliest, greedily, at most 7, none under 350 metal; the losses looked at by box of that
  reach), each from 4 s before its first loss (at most 16 s), a strike from 7
  s before it and a Commander's fall from 8 s before, merged and in order.
  `Transition::Highlights` starts the battle's replay (the live battle is archived; a
  multiplayer session is left) with a `Reel`: for each moment `Game::restore` puts back the
  copy before it (the replay's orders from there, effects and particles forgotten), the
  replay fast-forwards to the moment's start behind a title card, and the camera circles
  the fight (perspective, pitch 0.72) while it plays, at 0.35x around a strike or a fall.
  The HUD, overlays and icons step aside for letterbox bars, a caption and NEXT / LEAVE;
  Escape goes to the end panel (WATCH AGAIN, the whole battle, main menu).
* **Chat and cheat codes** (`chat.rs`): Enter opens the chat line in any live game
  (`Game::open_chat`; `chat_fresh` keeps the Enter that opened it from sending it). Lines
  go through the host in multiplayer and straight onto `chat_log` in single player, as
  `ChatLine`s in the speaker's colour. A line starting with `+` is one of TA's commands
  (`ta_command`, any game: the sharing switches, `+contour` the heights, `+sharemapping`,
  `+shootall`, `+switchalt` and `+clock` answering how TA Reborn does what they did, and what
  only changes this computer: `+noshake` (for the battle; SCREEN SHAKE in Accessibility,
  `gameplay.screen_shake`, keeps it off for good), the music's `+cd...` (`MusicRequest`) and `+sing`,
  singing acknowledgements, `GameAudio::sing`, a note of the Ode to Joy each) or a cheat code
  (`cheat_code`: TA's `+atm`, `+nowisee`, `+radar`, `+doubleshot`, `+halfshot`, `+view`,
  `+control`, and `+god`, `+instant`, `+unlimited`, `+heal`, `+elite`, `+kill`, `+unlock`,
  `+win`, `+spawn`); the answer goes on the chat in green. Cheats that change the battle are
  `Command::Cheat`s (see the simulation); the others only change this computer's view
  (`ViewCheats`: `Reveal` makes `viewer()` see everything or through another player's eyes,
  or `seen()` put every hidden unit on radar). Cheats need `Game::cheats_available` (live, not multiplayer); the
  cheat panel (`hud/cheats.rs`, key `cheats`, Ctrl+Alt+C, or the pause menu) has them all,
  and a spawn tool with a searchable list of every unit type (mods' too) whose clicks
  place units (`spawn_tool_input`). The end panel says CHEATS USED.
* **Computer players talk** (`chatter.rs`, setting `ai_chatter`): in single-player battles
  but campaign missions, each enemy computer player is a persona: a manner from its side
  (Arm officers correct, Core intelligences cold, a third side or a second of a kind brash),
  a name and a title (`Temper::title`: FIELD COMMANDER, MACHINE INTELLIGENCE, MERCENARY).
  They greet, answer the player's lines (hello, gg, taunts, questions, asked to surrender,
  who they are, where, how they are doing by their kills and losses, what they build, how
  many units they have, truces, compliments, help), cry foul at cheats, and taunt all battle
  long (`look_more` and events): first contact, the player's buildings they destroy and their
  own factories lost (`SimEvent::UnitDestroyed` carries the owner), costly units lost either
  way (700 metal and up, mobile), experimentals finished and destroyed (4000 metal and up),
  units the player captures (`UnitCaptured`), their last stand (factories all gone), the
  player's economy dry two looks running, the player's Commander under their fire below half
  health, armies twice the other's, the minutes (10, 20, 30, 45, 60, 90), Commanders lost,
  big battles, nuclear launches, defeat or victory. Lines with placeholders (`{unit}`,
  `{minutes}`, `{name}`, `{units}`) go through `tf`; a recurring kind of remark waits `AGAIN`
  (150 s), a talker keeps quiet 20 s between unprompted remarks, and an answer waits its turn
  behind one being typed. The personas remember the player (`rivals.rs`: `rivals.toml` in the
  player's folder, battles won and lost by name, the 64 last met): one met before comes back
  one battle in two, greeting the player by who won their last (`RematchWon`, `RematchLost`);
  `Chatter::decided` keeps the record as the battle is decided, in runs that keep records
  (`profile_progress::ordinary_run`; captures set `TA_RIVALS=won|lost`, never kept). Their
  lines come in as transmissions (`hud/comms.rs`): a card at the top left of the battlefield
  (under a goals panel; the messages column keeps clear of it) with their Commander's portrait
  on a little screen (scan lines, a refresh band, static while the link is made and as the
  words begin, a glitch now and then, the signal's bars), their name in their colour, title and
  side, the words typed out at 42 a second and held to be read, the voice's trace under them.
  While a line is "typed" (`Chatter::typing`) the card shows the link being made; one card at
  a time, the rest queued (`Game::transmissions`). The chat keeps the lines in its history but
  not on the screen's log (`ChatLine::title`). Everything here is local and changes nothing in
  the battle.
* **TA's interface sounds** (`Game::ui_sounds`, events of `gamedata/allsound.tdf` played by
  `GameAudio::play_interface`): ADDBUILD and SUBBUILD (`produce`), OKTOBUILD and
  NOTOKTOBUILD (placing), CREATESQUAD and SELECTSQUAD (groups), SelectMultipleUnits (a box),
  SPECIALORDERS (an order key), SETFIREORDERS, SETMOVEORDERS, IMMEDIATEORDERS (on/off,
  stop), OUTOFRANGE (`OrderFailed` "out of reach"), TREEBURN (`FeatureIgnited`), NOMETAL and
  NOENERGY (`resource_alarms`: a store dry while spending, 30 s apart) and "Victory
  Condition". A frame with one of them drops the widgets' generic click. Shift over a mine
  (`kamikaze`, static) draws its trigger circle (`kamikaze_distance`) and its blast's (the
  `SelfDestructAs`/`ExplodeAs` weapon's area), and over a constructor TA's squares round the
  queued buildings (green: the selection's; blue: the others'). Demos `mine`, `plan-squares`.
* **Jukebox** (`jukebox.rs`, key `jukebox` J, the title footer, `--open jukebox`): TA's tracks
  by their part in the default playlists (`ta_track_name`), the player's own by their tags
  (`ta_audio::tags`: ID3v2, Vorbis comments, RIFF INFO; else the file name), a click plays
  one (`MusicRequest::Track`), back (`MusicPlayer::previous`), stop, play, next, and how the
  player's music plays (`ta_config::UserMusic`: off, mixed into the peace, battle and menu
  playlists, or in their place; victory and defeat stay TA's). The player's files
  (`ta_config::music_files`: MP3, Ogg, FLAC, WAV, M4A in `music` in the settings folder and
  `Audio::music_folder`) play as `file:` paths, which the decoder streams from disk.
  Titles and durations are read on a background worker; rescanning invalidates metadata for
  files changed in place and ignores the old worker's replies. Custom-file duration probes
  seek through the container without reading its entire audio payload.
* **Audio transitions** (`audio.rs`, `ta_audio`): scene changes clear old effects, voices and
  combat intensity while keeping music running; loading frames service track sequencing and
  output recovery. Replaying the intro fades out both music decks before the movies. New
  effects use the current frame's listener before distance culling. Narration has priority over
  unit responses. A briefing's (`GameAudio::play_briefing`: the campaign's briefing, the pause
  menu's HEAR IT AGAIN) silences the music, and the menu's film behind the page
  (`GameAudio::briefing_speaking`); the tutorial's lines (`play_narration`) lower the music to
  28% of its chosen level. Both with a quick attack and gentle release; muted narration does not
  lower music. The mixer uses 10 ms gain
  ramps independent of output-buffer size and retries queued volume changes. Startup mute
  applies before the device starts; unavailable outputs are retried every two seconds.
  Crossfades wait for the incoming decoder to buffer, and a song shared by two looping
  playlists continues when the mood changes. Track controls work during playlist gaps.
* **Chat** (`hud.rs`): lines from others play TA's MessageArrived beep (`Game::chat_beep`)
  and go to the log; every line also goes to `Game::chat_history`, shown (scrollable) while
  the player types. Each speaker's picture is beside their lines and in the players panel
  (`Hud::portrait`): the player's own (`ta_ui::portrait`: `portrait.png` in the settings
  folder, an image chosen in the settings' YOUR PICTURE, its middle square at 256 pixels),
  else their side's Commander from `unitpics`.
* **Photo mode** (`photo.rs`, key `photo_mode`, F11; the pause menu): a single-player battle
  or a replay pauses (multiplayer goes on), the HUD, overlays and icons step aside, and the
  camera gets wider limits (pitch 0.02 to 1.55 rad, zoom 0.04 to 24); leaving restores the
  pause, the camera and the post settings. The panel sets the hour (`Lighting::time_of_day`),
  the depth of field (lens: a multiple of the focus distance, so zooming keeps the focus;
  tilt-shift: the band's centre and height; clicking the picture sets either; the classic
  camera offers tilt-shift only), exposure, saturation and vignette, and the photo's size
  (the window's, or up to twice it within 4K). TAKE PHOTO or F12 renders the photo before
  the frame on screen and saves it on a thread to `ta_config::pictures_dir()` (the Pictures
  known folder from the registry, `TA_PICTURES` to override) as `<map> NNN.png`
  (`ta_config::next_picture`); F12 screenshots go there too.
* **Perspective** (`perspective.rs`, `perspective_hud.rs`, key `perspective`, Q; the eye among
  the order buttons): a view from inside a unit and nothing more. The unit goes on with its
  orders and nothing is sent to the simulation, so the view is the player's alone, in
  multiplayer, replays and spectating alike. The key takes the first selected unit, else the
  one under the pointer: the player's own or an ally's (a spectator's, anyone's), finished, in
  sight, and mobile or armed (`Perspective::can_enter`). The camera dives into the unit
  (1.1 s) and climbs back out on Escape, or when a transport picks the unit up, over where the
  unit is then; the cursor is held in the window and hidden, and the mouse looks round by its
  raw motion (`Input::raw_delta`; a pad's left stick too); the wheel magnifies the sight, V
  looks from behind, N is the night sight. The eye sits where the crew would
  (`Cockpit`): in a walker's head, over a turret's gun towards its back (in line with the
  piece under the turret that reaches furthest forward), over an aircraft's nose, high
  amidships on a ship, a periscope at the surface; it is posed from the unit as drawn this
  frame (`Scene::pose`), and the unit's pieces the eye is in are not drawn
  (`Scene::hide_near`, from each piece's bounds). Ground units look in world yaw and pitch
  (the hull turns under the view); aircraft look round from the nose and roll with the bank
  (`Camera::roll`). The ears are in the unit too: the listener moves to the eye, hearing as
  wide as a 1,000-unit battle view (`audio.rs`, `EAR_WIDTH`). The cockpit is egui painted over
  the picture: the glass's marks over the world (under a modelled cockpit, clipped at its
  window's sill, the highest its own geometry rises into the middle of the view below the eye:
  `Perspective::sill_y`), the painted frame of a cockpit without a model, the instruments, the
  boot and a lost signal (the unit destroyed, or taken by a player the view may not follow); the
  instruments' rectangles are snapped to whole pixels. Up close, and where the view grazes
  level ground (the art squeezed two to three times more one way than the other), the
  terrain shader takes the art's colour averaged over 4×4 pixels (keeping the painted
  brightness as grain, less of it grazing) and sprites within 52 px of the eye thin away,
  dithered. Kinds with a modelled cockpit (all but the periscope) bring it in round the eye
  as the dive ends, straight ahead from behind the eye so that its frame closes in from the
  edges of the view, and take it back out on leaving (`Perspective::seat`), not from behind
  or through the magnified sight (under one, none of the unit's own pieces are drawn); the
  painted frame is only for the periscope, and comes in the same way. The cockpit is drawn
  with its kind's own field of view (`CockpitFrame::fov_y`) while the dive's widens to it; its
  lamps and screens come on once it is in, each screen after a self test, and a heading tape
  under a model without a heading screen of its own (a canopy, a gunship) keeps under its
  window's top (`Perspective::lintel_y`). It sways against the
  eye's acceleration (both smoothed, the slightest ignored) and kicks with the gun (`mount`),
  its tillers, sticks and helm follow the unit's speed and turning (with the hull failing its
  cabin lamp stutters, not with flashes reduced), and the instruments are
  painted in the rectangles its screens project to (drive, unit, radar, weapons, heading; the
  Commander's economy; an aircraft's panoramic display). What they read is
  gathered once a frame (`perspective_hud/data.rs`, `Readouts`: hull and hit points, rank, kills
  and veterancy, speed in km/h (a TA pixel taken as a quarter of a metre) and the throttle, the
  hull's heading, pitch and roll as it is drawn (`Perspective::attitude`, from the body's matrix),
  and the environment: the hour the world is lit by (`Perspective::hours`), the wind's strength and
  the bearing it blows to, the weather (`Effects::weather_kind`, the shower's strength) and the
  ground's height over the sea), and drawn with one kit (`perspective_hud/kit.rs`: a `Pen` that
  keeps a unit (a millimetre of glass, or a pixel of a 1080-line picture), a fade and, over the
  world, a dark halo). No display is a car's dial: the drive page (`pages.rs`) is the speed in
  figures with a throttle bar (its stub astern red) over the hull's heading, pitch and roll; the
  unit page is the hull in segments with its hit points, rank, kills and the veterancy against
  where each rank begins. Both are laid out in a box of a set size and so serve the glass of a
  console and a panel painted over the view alike.
  Every fact the cockpit shows is in one place: the glass (the window the crew looks out through:
  `perspective_hud::window`, a modelled cockpit's from its top, `Perspective::lintel_y`, to its sill,
  the periscope's round field, the magnified sight's circle) carries only what the eyes need looking
  out and what is anchored in the world; the unit's own state is on the console's screens, or, where
  none is in view (the periscope, from behind the unit, through the magnified sight), on panels along
  the picture's foot (`Layout`), never both. On the glass (`perspective_hud/sight.rs`, in the
  picture's own pixels, a unit a pixel of a 1080-line picture, with a dark halo): the weapons'
  readiness as arcs hugging each kind's reticle (`sight::readiness`: the first weapon's at the lower
  left, the second's at the lower right, the third's over the top, a lone weapon's on both lower arcs;
  each filling amber as it reloads, green when ready, red while the D-gun lacks the energy for a shot,
  flashing as it fires and pulsing as it comes ready; numbered as the weapons page numbers them, a
  kbot's always); the target bracketed where it stands (`sight::bracket`: corners fitted to its model
  as drawn, each drawn piece's bounds as its script has it, `perspective::drawn_corners`, closing in
  as it is taken, its hull along the bracket's foot, one line by it, its name and range, placed clear of the
  kind's own marks, which each kind's sight returns; out of the window, an amber chevron on the threats'
  ring pointing the way to it with the same line; destroyed, `HudState::destroyed`, the bracket closes
  up where it last stood with DESTROYED); other enemies in small red brackets (blinking while they fire
  at the unit) and friends near by as green carets; threats out of the window (`sight::threats`: enemies
  within 1,400 and any firing at the unit, as red chevrons on an ellipse inside the window's edge
  pointing the way to them on the screen, `View::toward`, behind the eye down, one a bearing, larger the
  nearer, pulsing while they fire; a hit from out of the window flashes the chevron its way and glows the
  edges on that side, `HudState::hits` keeping the attacker's place); one line of messages low in
  the window, the newest, with no plate (`sight::message`, kept clear of the other marks there). Every
  hit flashes the edges red for a moment, harder the harder the blow (`sight::hit_flash`: from the
  picture's edges, the cabin red as with a lamp, fading into the glass, down to the window's foot so the
  console's screens stay as they are; round a round field's rim), not with flashes reduced. The glass
  itself shows damage: a crack for
  each of the hull's halving, quarter and eighth (`perspective::CRACK_AT`, `HudState::crack_to`: the
  same places on the same unit, growing in as the hull falls past them, gone when it is mended, kept to
  the window), and under a quarter sparks falling from the window's top. The heading is shown once: on
  a modelled cockpit's heading screen; on an aircraft's helmet; else (from behind, through the magnified
  sight) a tape over the view; the periscope has its own bearing ring. The environment (`environment.rs`:
  the clock with the sun or the moon, the wind with an arrow from the view's way up, the weather and the
  elevation) is a status line along the foot of the weapons page, or, with no console in view, a strip
  of small plates in the picture's top left corner.
  Aircraft (fighters, bombers, gunships) have a glass panoramic display, one wide screen (`pcd`) of
  three panes (the aircraft's page, the radar, the weapons: `perspective_hud::glass_pcd`), in place of
  the four small instruments, and, after a modern fighter's, the flight symbology projected on the
  helmet (`perspective_hud/air.rs`): a heading tape with the nose and the target marked, a pitch ladder
  drawn on the world (so it banks with the aircraft), the flight path marker (the eye's smoothed
  velocity, `Perspective::velocity`) and a gun cross at the nose with the weapons' readiness round it,
  speed and height on tapes with their figures boxed, and a bank scale with the pitch and roll in
  figures. The flight's figures are on the helmet only, the airframe's on the panel only; from behind
  the aircraft (no ladder there) an attitude ball stands with the panels along the foot.
  Vehicles, ships, hovercraft, gun houses and the periscope have a tank's sight
  (`perspective_hud/armour.rs`): a reticle (the periscope's and the magnified sight's own cross-hairs
  instead) with the readiness arcs round it and a ring on the picture where the gun points
  (`Perspective::gun_dir`, the aim piece's z as it is drawn); beside it, small, the hull seen from above
  under its turret, the view's way up (the hull turned as far as the eye looks off its heading, the gun
  as `Perspective::gun_azimuth` has it: the piece under the root that carries the aim piece; a gun
  house's base square), or, on a ship's bridge, the gun director (the ship from above, its guns' train
  and their bearing off the bow, BRG); and, with nothing bracketed, the range along the line of sight as
  a laser range finder reads it (`data::ground_range`). A low window (a ship's, a gun house's slit) gets
  a smaller sight.
  Kbots (walkers, the Krogoth, the Commander) have a mech's (`perspective_hud/mech.rs`, a little larger):
  a reticle with the weapon groups' readiness round it, numbered, and the torso twist as a short arc
  under it, its middle where the eye looks, the legs marked on it the other way (as far as 120 degrees
  either side, an arrow past that) and the torso's gun where the gun has taken it elsewhere. Their armour
  by section is on the console's unit page in the picture's place (`pages::armour`; `perspective::Armour`,
  presentation only: the simulation knows one hull, and the armour spreads what it loses over eight
  sections by where the blows came from, weighted by each section's share and its facing, and always
  sums to the hull's loss, so repairs and unseen damage settle too). The console's pages: the drive
  (speed, throttle, the hull's heading, pitch and roll), the unit (picture, name, hull, rank, kills,
  experience), the radar (the window's field as a fan, what the guns are on ringed, enemies firing at
  the unit pulsing), the weapons (what the unit is doing as the deck says it, `perspective_hud::doing`,
  without naming the bracketed target again; each weapon numbered, its state, reload and reach; the
  environment along its foot), the heading, the Commander's economy. `TA_PERSPECTIVE_TWIST=<degrees
  right>` and `TA_PERSPECTIVE_PITCH=<degrees>` start a capture looking off the hull's way,
  `TA_PERSPECTIVE_HULL=<share>` with the hull worn down (`--demo perspective`).
* **Strategic zoom** (`strategic.rs`): pulled far back (from 0.5 down to 0.25 screen points
  per world unit, where a light tank is 16 then 8 points long) every unit the viewer sees
  fades into an icon in its owner's colour: the Commander a star, buildings squares
  (factories hollow, armed ones hexagons), land units circles (constructors hollow),
  aircraft triangles, ships and hovercraft diamonds, bigger for costlier units; unfinished
  buildings are paler, the selection wears white rings (the HUD's selection circles give
  way to them). Clicks that miss every unit's sphere pick the icon under the pointer
  (`Game::unit_at_screen`). The player can turn the icons off (STRATEGIC ICONS, Interface,
  `gameplay.strategic_icons`): `strategic::shown` is then nothing, and what the icons stood in
  for comes back at every height (the selection's bars and circles, the markers over the units),
  while what follows the zoom alone (the pick of a unit a pixel wide, the build times' fade) keeps
  `strategic::amount`. A benchmark's scenes keep their own icons whatever the player's. `--camera` now also wins over where a demo moves the camera, for
  captures such as `TA_SWARM=300 --demo swarm --camera 4624,4672,0.16,0,63`.
* **Area orders**: in attack, capture, resurrect or reclaim mode a drag draws a ring;
  everything inside is queued as a nearest-first tour (`Game::area_order`).
* **Placing buildings**: a site must be free on the map and clear of the buildings other
  constructors have queued but not started (`Game::planned_footprints`). The selection's own
  plans never stand in the way of one building put down (`Game::own_plans_under`): put down
  again on one of them (the same building, the same footprint) it undoes it, with
  `Command::EditQueue`; another over them takes their place (with Shift they are taken out of
  the queue, the rest kept; without, the new order replaces the queue anyway). A row dragged out
  with Shift keeps clear of them. A right-click ends the placing; with Shift it also gives the
  right-click's order, queued (a Commander told to build a plant, then sent on). While placing,
  every constructor's queued buildings show as faint ghosts. Buildings stand on the highest
  ground under their footprint (`World::footprint_top`): TA drew them over a flat picture,
  so on the 3D ground a bump would poke through a pad; the drawn ground is levelled up to
  them once they are put down (see Terrain, `terrain_pads.rs`).
* **Top bar** (`Hud::top_bar`): the side, the resource bars (narrower on narrow windows or
  a large interface), units, time, wind, the idle factory and constructor chips; on the
  right the MAP chips, the pace and the simulation time, in the widest form that fits what
  the left leaves (`RightGroup`, measured from its pieces' text): all, without the time, or
  the overlays folded into a "MAP ▾" menu; never drawn over the left.
* **Map overlays** (`overlay.rs`; F2 movement, F3 heights, F4 metal, L sight, or the MAP
  chips in the top bar): metal, movement (the selection's `MoveSpec`, buildings included)
  and sight are colours per heightmap cell uploaded to the terrain
  (`TerrainGpu::set_overlay`, rebuilt when vision, buildings or the selection's type
  change); heights are drawn per pixel by the terrain shader (a ramp with contour lines).
  The metal map shows by itself while placing an extractor.
* **Area extractors** (Alt+X, or the order button): drag a circle (a click takes 96 px around
  the cursor); every metal spot in it gets the best extractor the selection builds, richest
  first, each clear of the others, of buildings and of queued plans, none on ground under a
  quarter as rich as the best; queued nearest-first.
* **Queues**: a click on a factory's queued unit builds it next; a constructor's queued
  buildings show in its panel (click: next, right-click: drop). Both are
  `Command::EditQueue`, naming the entry clicked (its index, type and, for a building, its
  place): the simulation finds it in the queue as it stands when the command runs (the
  nearest of that kind, should a unit or a building have finished meanwhile) and, for a
  constructor, gives the rest of its orders again from there. (`QueueFront`, by index alone,
  is kept for old replays.) Alt+drag while placing fills a rectangle; COMBAT, BUILDERS,
  DAMAGED and IDLE narrow a selection.
* **Duties** (`game/duty.rs`, key `duty` Alt+D, the stance column's DUTY button and its menu, the
  radial menu's DUTY choice and ring: both read `game::duty::CHOICES` and `Game::duty_control`, so
  they never differ; `ta_ai::DutyOfficer`; off in replays, as a spectator and in lessons): the selected constructors
  (the Commander too) go on one of four duties or back under the player's command (what each
  does: `ta_ai::Duty`; the economy duty keeps to extractors, power plants and metal makers, and
  finishing those, `SkirmishAi::economy_duty`). In single
  player the officer's orders join the tick's commands (`Game::duty_orders`, before
  `tick_orders`), in multiplayer they go through `Session::submit` like any command of the
  player's (not while a client is catching up), so they are recorded and replayed with the
  battle; the duties themselves are client state, kept beside the commands in a saved game
  (`Record::duties`, from `RECORD_VERSION` 44, `Record::with_duties` at the three places that
  save: autosave, quick save, Save game) and put back by `Game::restore_duties` when the save has
  caught up (`Playback::duties`); replays have none. Any `Command::Order`, `Formation` or `EditQueue` that comes through `Game::issue`
  takes the units it names off duty (`Game::release_from_duty`: clicks, keys, buttons, the build
  menu, Stop); a Commander on duty that an enemy hits, or that has armed enemies in sight close by
  (`DutyEvent::Danger`, told by `DutyOfficer::take_events`), leaves it at once. Units on duty are left
  out of the idle counters, the forces plate's idle marks, the IDLE filter and alerts, and wear a
  tag on the map (`Hud::duty_tags`) and "ON DUTY · ECONOMY" on the selection card. The card's
  activity line says what a unit on duty is about when its orders do not (`Game::duty_line`: that it
  is getting away from enemies, or has nothing to do, and why: `duty::idle_words`); a tag of a unit
  with nothing to do is dimmer and says IDLE, tags of one duty close together are one tag with their
  number (ECONOMY ×3) and tags that meet are stacked; a group's card counts them ("6 on duty (2
  idle)": `SelectionDuty::idle`) and the hover card over one of ours says ON DUTY and the same
  line; `DutyEvent::Idle` is a notice with LOOK. An
  order of the officer's own that the simulation refuses raises no "Cannot comply" notice and no
  "can't" voice (`Game::run_tick`, the `OrderFailed` events of units on duty are not passed on): it
  looks for another job. Staged scenes have a `duty` event (`stage.rs`) for filming.
* **Extractor placement** snaps onto the richest site within two cells
  (`Sim::best_extraction_site`; Ctrl places exactly), labels the yield and marks the
  deposits around.
* **Panels**: fire and move state buttons, factory queue strip (right-click removes) and
  repeat, rally routes, unit hover card, idle-constructor counter, replay bar (progress,
  speed, skip ahead, whose fog to watch), players panel (hold Tab; other sides' economy
  only for allies, spectators and after the game: `Game::may_inspect`).
* **Factory routes**: move/patrol/fight/guard orders to a factory don't touch production;
  they set `Unit::rally`, the orders each released unit gets (Shift adds waypoints). An armed
  factory (any builder that does not move) keeps the orders its weapons fire on, its production
  going on: attack orders when it has a gun (Escalation's Big Bertha, Guardian and Sentinel are
  builders), manual fire when it has a weapon for it (mods' "buildpad direction" weapon turns a
  factory's pad); Stop takes them back with the route.
* **Effects**: explosions, smoke, projectiles, debris thrown by `explode`, dynamic lights,
  scorch marks, burning wrecks, nuclear blasts (with the perspective camera a fiery explosion
  of TA's, measured when it loads, is drawn as fireballs as big and as long-lived as its
  painting, `AnimLook`, and a splash as a column of spray; the classic camera keeps TA's
  paintings); weather by planet (snow; on lava worlds the odd ember lifting a few pixels off
  the melt, a bubble bursting now and then, the more of them the higher the effects quality;
  dust, spores, glitter; on green, water and Earth worlds showers every few minutes, timed by
  the game clock, with lightning while they pour and `Lighting::overcast` dimming the sun; it all
  drifts with the simulation's wind, `Sim::wind_heading`, the way the wind generators
  face), the ways of ships and hovercraft for their wakes (drawn on the water, see
  `wakes.rs`), tread marks and footprints (decals), dust behind units on dry
  worlds (`dust_for_planet`) and aircraft vapour trails (`effects_ambient.rs`). Particles and
  debris have soft and hard budgets, so mass destruction thins out instead of stalling.
  What is left of the dead (`effects_remains.rs`, from the scene's `Remnant` minus the
  pieces the death script threw clear, which fly as debris): aircraft crash, ships and
  anything afloat sink, buildings blown to nothing collapse; big units set off later blasts
  (`Pop`). It is shown once: the simulation names the unit a wreck is of
  (`SimEvent::FeatureCreated`'s `wreck_of`), so a building's wreck stands in its place at once
  and the building does not come down beside it, and a wreck under a sinking hull (which settles
  on the bottom) or a falling aircraft is not drawn until they come to rest
  (`Effects::unsettled_wrecks`); a frame killed unfinished leaves no charred model
  (`UnitDestroyed`'s `complete`), and a unit taken off the map (`removed`: reclaimed, given up)
  neither that nor a fire. Every wreck left on land burns, then smoulders, the longer the bigger
  the unit was (`Burning`'s `fire` and `smoulder`, at most `MOST_BURNING`).
  Battle damage on units still fighting (`effects_damage.rs`): below a third of their health
  they burn (flames, embers and a smoke column, their firelight and shimmer for the nearest
  48), below 15% they throw sparks, and a damaged aircraft trails smoke (flame too once it
  burns); lighter damage smokes from TA's own SmokeUnit scripts. Paralysed units carry
  `FLAG_STUNNED` (electric blue bands and arcs in `unit.wgsl`). Health bars keep the health
  just lost as a pale chunk that drains after a moment (`hud::BarTrail`), light their frame
  for a moment when hit, beat red under a quarter (steady with reduced flashes), mark their
  quarters when long enough, and show a paralysis's time left in blue over the bar. All of
  it is limited to what is in view (the nearest `MOST_ABLAZE` burning, 48 lit): a battle of
  twenty thousand costs no more than one of two hundred (the quick benchmark showed no change).
  Weapons and impacts (`effects_impacts.rs`): blasts from 40 px send shockwaves out over the
  ground, a soft band and a hard front (`Ripple`s drawn as ground rings with the selection's,
  `rings.rs`, draped over the ground; the nuke's too, where a camera-facing sprite stood up
  like a hoop); shots that splash down ripple the water; an EMP (any paralyzer of 60 px or
  more) pulses blue with electric sparks and no fireball, scorch or smoke (it was drawn as a
  nuke); a blast is nuclear (flash, shockwaves, mushroom cloud, `nuclear`) by its size from
  240 px, or from 128 px when its data calls it so (nuke, nuclear or atomic in its name, the
  name players see or its explosion's art: Total Mayhem's Orion drops `NUKEBOMB`, 225; TA's own
  Bertha shells burst in art named NUKE1 but are 40 across); lightning bolts crowding one
  another (a line of Tigers' five-bolt bursts, a battery of Zeuses at aircraft) are each drawn
  fainter, their light together growing as the root of their number, and after their first
  stroke in some strokes only (`BoltLook`), so they read as lightning, not a wall of light (a
  lone bolt, or two or three, as it always was); impacts flash and light in the weapon's own colour (`impact_tint`: a laser's,
  lightning's, an EMP's blue, else the warm orange); beams burn small marks; a missile shot
  down flares in the sky and falls in burning pieces; cannons throw their muzzle flash forward
  along the shot with a puff of smoke (`muzzle_blast`, in view only); flamethrower streams
  fill with rolling flame, torpedoes trail bubbles, the D-gun, flames and missile exhausts
  shimmer; TA's `shakemagnitude` shakes the camera, less the further from the middle of the
  view; and smoke drifts on the wind (`SMOKE_WIND`). A missile's model is measured as it loads
  (`WeaponModel`: its body, not TA's `flare` piece, moved onto the line it flies, its tail
  kept), lies along its flight with its top up (no roll; straight up it faces the way it turns
  over), turns smoothly between ticks, and its exhaust (`missile_flame`) runs from the nozzle at
  its tail back along its body, its sparks and smoke from there; TA's flare pieces are hidden
  under it. A side's own strategic missiles and interceptors are drawn in flight out of its
  sight, and their blasts and interceptions seen and heard (`game::nuclear::strategic`). The
  minimap tracks a nuclear missile only where it is seen or on radar: a trail of the last ten
  seconds fading behind a pulsing trefoil, a flash where one followed to its end ended; its own
  side sees the way ahead dashed to the blast's ring and the time left
  (`game::nuclear::time_to_impact`), as the world's NUCLEAR IMPACT ring does (neither for a
  unit target out of sight: the missile homes on it).
* **Skirmish options** (`ta_sim::GameRules`, part of `GameSetup` and the multiplayer start
  message): starting resources, fog of war / mapped / permanent sight, commander death
  ends the game, unit limit, random start positions (`setup::assign_starts`, drawn from
  the game seed so every peer agrees), movement (`GameRules::give_way`, "Formations, units
  give way": `Sim::local_avoidance` and `Sim::group_formations`; off keeps TA's movement bit
  for bit; missions and scenarios keep it off). The skirmish page shows them under two tabs,
  RULES and MUTATORS (`menu::battle_options`, the lobby's too), keeps the last setup in the
  profile's `skirmish.toml` (`Menu::keep_skirmish`, `restore_skirmish`: map, players, rules,
  not the unit limit the settings give), picks a map at random with room for the players and
  sets the teams at a click (free for all, the player against the rest).
* **Mutators** (`ta_sim::Mutators` in `GameRules`, `Sim::set_mutators`, mutators.rs), as
  Supreme Commander: Forged Alliance's lobby offered: income in quarters of TA's (applied per
  unit in the economy's tick), build power so many times (`mutator_work_rate`), damage divided
  by toughness in quarters (`mutator_damage`, after the cheats and perks), a ceasefire (no
  rush, `ceasefire_left`): enemies' weapons do no harm (`ceasefire_spares`, paralysis too) and
  nobody may order a building within two fifths of the way to an enemy's start, 1,000 at most
  (`ceasefire_bars`, the start kept in `Player::start`; an `OrderFailed` says why); and kinds
  of unit left out through `Sim::allowed` (aircraft, ships, missile silos with the
  anti-missiles, fixed guns of 3,000 range or more, mobile units of 10,000 metal or more but
  the Commanders; a factory none of whose mobile products are left goes too). The computer
  players respect `allowed` and send no attack (waves, fleets, strikes, lifts, silos) while
  more than 45 s of ceasefire remain (`Survey::peace`). Presets (Blitz, Peacetime, Arms race,
  Ground war, No superweapons, Ironclad, Glass cannons) set several at once; the battlefield
  column, the pause menu's card and a HUD strip with the ceasefire's clock list what is set.
  `Mutators::parse` reads `resources=8,no_rush=5,no_air` (`TA_MUTATORS` for `--map` captures,
  `TA_TEST_MUTATORS` in the AI soak test).
* **Statistics** (`stats.rs`): totals (`table`) and graphs (`chart`: a key, round quarter lines
  (`rounded_top`), the time along its foot, every player's reading under the pointer) on the
  result's pages and the pause menu's statistics page.
* **The battle's result** (`result.rs`, `--open victory|defeat` with `--map`): once
  `Game::outcome` is set the HUD steps aside (the Playing frame draws `result::draw` in its
  place), the battle is graded down behind it (`post.backdrop` 0.8, eased slowly so the
  decisive moment is seen first) and the camera glides to where it was decided (the last
  enemy Commander to fall, or ours, within the last minute: `App::result_glide`; a perspective
  camera then turns slowly). Over it the title (glowing, rays behind a victory) with what decided
  the battle (`verdict`) and its captions (a scenario's, a Conquest battle's own), then the
  debrief in a panel whose footer stays in sight: SUMMARY (the player's figures counting up, the
  best of every player's in brass; the best unit and the key moments from the chronicle; the
  service record, in a line on a short screen: `ta_ui::profiles::battle_record_line`), PLAYERS
  and GRAPHS, both frozen at the moment of the result (`Debrief`'s summary keeps the players and
  their last readings: the battle goes on behind). The footer: WATCH HIGHLIGHTS, the way home
  and the next step (REMATCH for a skirmish of the player's own; scenarios', missions' and
  Conquest's as before; KEEP WATCHING after a defeat others fight on). It comes in after a beat
  (title at 0.6 s, debrief at 1.3 s, figures from 1.6 s; at once with reduced motion). The
  chronicle (`chronicle.rs`, kept in the tally and noted with it, also when fast-forwarding;
  `Game::restore` forgets what came after) keeps each enemy unit destroyed and by which unit,
  the first blood, the Commanders that fell and the nuclear launches. Captures:
  `TA_RESULT_MINUTES=n` plays the battle's first n minutes with a computer player on the
  player's side too, fast-forwarded, before the result (`TA_BATTLE_RECORD` its career record).
* **Tutorial** (`tutorial.rs`, TUTORIAL on the title menu or `--tutorial N`): five lessons
  on Painted Desert against a CORE player with no computer behind it (a non-local
  `Controller::Human` slot): the basics, a first fight, orders like a veteran, defence (a
  tower and radar against a raid, repairs, wrecks) and builders and factories (helping a
  factory, its rally point and Repeat, patrols, standing orders). Each lesson is a list of steps, an instruction and a check
  on the game (selection, units owned, `Game::used` notes of orders given, the overlay
  shown); the panel ticks a step off a moment after it is met, and SKIP passes it. Lesson
  2 sets its scene at tick 0 on the client (squad, lab, an enemy outpost; the enemy
  Commander goes by `Sim::remove_unit`), so lesson games keep no replay, saves or outcome
  (`Game::tutorial`). Markers ride on `Game::objective_areas`. Finished lessons are kept
  in `CampaignProgress::tutorials`. `TA_TUTORIAL_STEP=n` starts at a step (captures);
  `--demo tutorial-fight` plays lesson 2 through.
  Each step is said aloud as it begins, and its hint when that shows (`Tutorial::take_line`;
  the narration channel at the unit voices' volume; Options' SPOKEN TUTORIAL turns it off).
  The clips are TA Reborn's own (`assets/reborn/voice/tutorial/<language>/<id>.ogg`, made by
  `tools/voice/tutorial.py` with Chatterbox Multilingual, MIT, every clip heard back by Whisper):
  a line's id is the start of the SHA-256 of its English words with the keys and mouse buttons
  filled in (`keys::fill_english`), so a reworded line, or one whose key the player rebound,
  has no clip and is not heard; a language without a clip of a line falls back to English's
  (`tutorial::voice_path`). `ta_audio` decodes them (Ogg Vorbis through symphonia). The voice
  says a line a sentence at a time and runs on after a sentence's last word (breaths, clicks,
  mumbles, a hiss, words of its own), so the tool cuts each sentence to its words as it makes it
  (Whisper's word times, then the sentence's sound in bursts: a 150 ms tail kept, only dying
  away, faded) and levels every line to one loudness; `tutorial::tests::every_clip_ends_with_its_words`
  holds every clip's last 120 ms 35 dB under its loudest (the narration ducks the music while it
  plays, so a clip's end is where the music comes back).
  The guide shows the way a click at a time: each step names what it is about (`Aim`: the
  Commander, the squad, building or producing a unit, an order's button, the MAP menu, the
  build queue), and the interface records each frame where it drew what may be pointed at
  (`hud::anchors::Anchors`: the build menu's pictures, tabs and pages, the order buttons, the
  MAP menu, the queue; the modern deck and TA's classic panel alike). The next click gets a
  breathing ring and an arrow: the unit to select on the map (an arrow at the view's edge
  when out of sight), then the tab, the page, the picture or the button; while the building
  is placed or the order armed, the marked place. Steps count as they go ("Ready: 2 of 3",
  "Still standing: 3 of 5"), give a hint after 20 s without progress (`TA_TUTORIAL_STUCK=1`
  at once), SHOW ME brings the camera to the marked place or unit, a step ticking off and the
  lesson's end are heard, and the panel folds away. On the classic interface steps read as it
  is (no tabs, no MAP menu) and the queue step is passed over. `--demo tutorial-select`
  selects everything (captures of the guide's next pointer). A step may set something up as
  it begins (`Enter`): lesson 4's raid (four AK kbots out along the way in, attack-moving on
  it through `Game::direct`, the player's tower in their way) and a building to mend (the most
  damaged, else one wounded with `Sim::set_health`). Lesson 5 starts with a Kbot Lab, two solar
  collectors and three Peewees. `Game::used` notes guard, patrol (a builder's), repair,
  reclaim, rally, repeat and hold-fire orders as they are given.
* **Notices that point somewhere** (`Game::messages`, `Notice`; `Hud::messages`): a notice
  or a piece of the coach's advice about a place or one of our units has a `Link`
  (`Place` or `Unit`) and a LOOK chip (`hint::paint_chip`); a click on it goes there
  (`Game::follow`: the camera to the place, or the unit selected and looked at). Units under
  attack, an ally's attack and signal, reinforcements and waves, a nuclear launch (where it
  will land), an intercepted missile and an order that could not be carried out have one; so has
  the advice about the Commander, an idle factory or builder and a free metal deposit
  (`Game::free_metal_near_commander`). The notices' column is no area to click (it lets the
  battlefield's clicks through): each notice that points somewhere gets an area of its own
  over it. The idle constructors' and factories' alerts (a lamp over those in view, an arrow
  at the view's edge for the rest) come after six seconds idle (`IDLE_ALERT`: a builder that has
  just finished is not forgotten yet) and fade at twenty.
* **Tips** (`tips.rs`): one on the DEPLOYING panel, and one low in the view from two and a
  half seconds into a skirmish to the twelfth, on the hint plate (`hint.rs`: shared with the
  coach: a lamp and caption, the words, the LOOK chip for what points somewhere, a cross, and
  a line that stops hints of its kind for good; it goes pale when the pointer comes near it, so
  that it never hides what the player is working at). `Gameplay::start_tips` (Options ›
  Gameplay: battle tips) turns them off; the tip's own line does too.
* **Coach** (`coach.rs`): in a profile's first five battles (`ta_config::Coach`, Options ›
  Gameplay: first battles, always, off) the coach watches the player's own battles (not
  missions, replays, lessons or scripted runs) for what new players get stuck on: the
  Commander badly hurt, energy or metal running dry, no factory after two minutes, an idle
  factory or idle builders, a full metal store (the advice to build only for a side with
  something that can build: `coach::can_build`; a scenario's army without a Commander or a
  builder hears none of it). Each must last its while (on the battle's
  clock, so a pause waits), after the opening the start tip has; each is said once, one at a
  time with a quiet spell between (forty seconds; the opening is thirty), on the hint plate
  low in the view (with the key that fixes it), gone when its time is up (which stands still
  while the pointer is on it) or soon after it is put right. Advice that points somewhere has
  the LOOK chip (`Advice::link`: the Commander, the idle factory or builder, a metal deposit
  with nothing on it, found as it comes up and looked for again each second while there is
  nowhere yet); the cross puts it away; the line under it
  stops that advice for good (`Gameplay::coach_muted`, advice names; Options › Gameplay:
  hidden advice shows them again). Its timing is a pure step, tested alone. `TA_COACH=1`
  turns it on in any battle, `TA_COACH_SAY=<advice>` says one at once; `--demo hints` stages
  notices and advice to click. In a battle the interface's hover hints wait 0.8 s (0.5 in the
  menus) and a unit's hover card 0.45 s.
* **Map editor** (`editor/`; the title menu's MAP EDITOR, `--editor ["<map>"]`): its own
  screen (`Screen::Editor`, frames in `State::editor_frame`; the menu's battle waits). Round
  the view (`ui.rs`, `status.rs`): the top bar (the map's state, its problems from
  `checks.rs`, the file's buttons, TEST PLAY), the labelled tool palette, a status bar (the
  ground under the pointer, the tool's keys, the KEYS & MOUSE page), switches over the view
  (grid, height map, features, start positions, void cells) and `navigator.rs`'s miniature
  of the map. TEST PLAY saves the map and keeps the editor in `State::test_editor` while a
  duel is fought on it; leaving the battle (`Transition::ToMenu`) becomes
  `Transition::ResumeEditor`, which loads the map into the renderer again (`Editor::resume`).
  A
  new map (size in 512 px steps, sea level, height, the tile set and planet of a stock map)
  or a copy of an existing one (`doc.rs`: TA's TNT grids plus the OTA's properties and
  start positions; heights per cell vertex, tiles in art space, i.e. the tile the art shows
  at ground point (x, y, z) is at (x, z − y/2)). Tools (`tools.rs`, all undoable through
  `history.rs`, which diffs a snapshot taken when the mouse goes down into the smallest
  changed rectangles): raise, lower, smooth, flatten and set level with radius and strength
  (full-precision working heights per stroke); tile blocks from this map (Ctrl+drag) or any
  other (the picker shows a map's art or its whole tile set; imported tiles are deduplicated
  by content), stamped on a grid while dragging; copy and paste of a region's tiles and
  heights (with its features and void cells, `Clip::paste`'s `PasteWith`; heights relative
  to the ground there laid on it by the copy's edge level); features filtered to the
  planet's world (`world_matches`: "Green Planet" → greenworld, "Red Planet" → mars,
  "Lunar" → moon; ALL WORLDS lifts it) and to trees, rocks, metal, vents, wrecks and the rest,
  shown as a grid of pictures, placed (a see-through, tinted ghost under the pointer, its art
  loaded into the view's sprites with `View::load_visual`), scattered (`tools::Scatter`: random
  tries in the brush, kept a density's spacing apart by `MapDoc::feature_near`, on the land or
  the sea as where the stroke began) or erased, and taken from the map with Alt+click
  (`MapDoc::feature_covering`); up to ten numbered start positions, dragged,
  added, removed, renumbered; the properties (name, description, planet, wind, tides,
  gravity, surface and moho metal, sea level). Held strokes and live Properties edits are
  finished before commands or close checks; Properties stays a separate undo step. Undo
  entries retain their before/after revision identities, so returning to the saved revision
  is clean, while a new edit after undo gets a fresh identity. Both the window close button
  and the quit key ask about unsaved edits. Discard permits the requested transition without
  marking the old document saved: a failed replacement keeps its dirty state and history.
  Modal dialogs stop canvas gestures, and a successful map switch clears pending gestures,
  requests and picker state. A failed external tile source is shown as an error and cached
  until Retry/reopen; it never silently substitutes the current map.
  TNT graphic indices use all 65,536 values; an import that needs more rolls back its new
  tiles and reports an error. Startup, map switches and imported tile sets also pass
  `Renderer::map_fits` before GPU upload; a failed switch retains the old document, and a
  rejected import removes appended tiles without replacing the current brush. Feature
  indices keep the format's reserved markers separate from feature names. `view.rs`
  pushes each frame's changes to the terrain in place and
  draws the features as the game does; a changed planet also refreshes world grading and
  fluid selection. H shows the renderer's height overlay (colour ramp and contours).
  Pictures (`import.rs`, the dialog `import_ui.rs`; IMPORT…, a file dropped on the window,
  the system's file chooser through `rfd`): read with `image` (PNG, JPEG; 256 MB and 16,384 px
  a side at most), resampled with a triangle filter a row at a time; heights from brightness
  between two levels; art from colour, matched to TA's palette through a 2¹⁸-entry table of
  nearest colours (redmean distance over 6-bit channels, built once) with optional
  Floyd–Steinberg dithering, then cut into tiles interned by content (alike tiles shared), a
  row at a time, rolled back if the tile set or the GPU's room runs out. The whole map's art
  is one edit; a brush goes to the Tiles tool. EXPORT HEIGHTS… writes a grey PNG.
  Saving (`save.rs`) writes a mod
  of its own to the user's mods folder (`mod.toml`, `maps/<name>.tnt` with unused tiles and
  names dropped and a minimap drawn from the art, `maps/<name>.ota` keeping the source OTA's
  other keys). A library lock covers destination checks and publication. Existing targets
  must be ordinary editor-owned folders; a disabled unrelated mod cannot be overwritten.
  The complete replacement, including retained extra assets/documents, is prepared in a
  private `.installing` folder before the old directory is moved aside. A failed publish
  restores the old directory, or reports the retained backup if restoration also fails.
  Copying rejects links and bounds depth, entry count and total bytes. The save then enables
  the mod in settings and mounts it over the running game's data (a new VFS with the same
  layers), so the skirmish page lists the map at once. Settings failure remains a save error;
  the document is marked saved only after publication and settings persistence succeed.
  `--editor-open new|open|properties|picker|tileset|heights|tiles|copy|copied|features|starts|import`
  opens a dialog, view or tool for captures (`copied`: a copy ready to paste; `import` with
  `TA_EDITOR_PICTURE=<file>`); `demo` plays every tool through the mouse path with scripted
  input (`demo-save` then saves). `--camera` frames an editor capture, and `TA_UI_HOVER`
  rests the pointer over the map.

## Multiplayer (`ta_net`)

* **Lockstep, host as pacemaker**: every tick the host seals a bundle of the commands it
  received since the last one (its own included) and broadcasts it; every peer, host
  included, steps its own simulation with exactly those bundles, in order. Local orders
  are sent to the host and run when they come back in a bundle (one round trip). A client
  sends what it ordered since it last read the network together (`Session::flush`, on
  every poll and after the frame's interface), in messages of at most `BATCH_BYTES` (1 MiB):
  a formation move of thousands of units is one message. AI players run on every machine
  from the same state, so they need no traffic.
* **Transport**: TCP with `TCP_NODELAY`, length-prefixed bincode frames (at most 4 MiB),
  reader and writer workers per connection; the game thread never waits on socket reads
  or writes. Both queues are bounded by bytes and frame count; readers share a 256 MiB
  decoding/backlog budget across the host's connections, and decoded messages and game-thread
  poll batches have their own limits. At most 32 connections are kept; uncompleted handshakes
  expire after ten seconds. Peers have a 1 MiB/20,000-command allowance per tick, including
  while paused; past it the sender's commands are dropped (not a reason to disconnect them).
  Fixed-int bincode decoding rejects trailing bytes. The host owns a
  nonblocking listener, releasing its port immediately when dropped. Closing the program in a
  battle or lobby says goodbye first (`Session::goodbye`, `NetLobby::goodbye`, after the event
  loop), waiting up to 300 ms for it to be written (`Host::drain`, `Client::drain`), so the
  others hear that someone left rather than that a connection failed.
* **Handshake**: `Hello { name, protocol, data_hash, reconnect, password }` — the host rejects
  peers with a different protocol version or game-data fingerprint
  (`ta_data::Compatibility::network`, `Assets::network_hash`), unauthenticated joins after the
  game started, and newcomers without the lobby's password when it has one
  (`Host::set_password`; players coming back with their credential need none). `Reject` carries
  a `Refusal` (version, data, started, credential, name, password), which the joining player's
  game words in their own language. A data refusal carries the host's mods in load order
  (`Refusal::Data { mods }`, `Host::set_mods`: each `NetMod`'s id, name, version, the digest of
  its files as records keep it, and whether it gives the game anything besides maps); the
  joining player's game compares them with its own and those installed (`lobby/data.rs`, on
  `ta_config::records::mods::check`) and names each that differs with what to do: turn it on
  in the launcher's Mods page, not installed here, turn it off (the host plays without it),
  another version or other files (you both need the same), another load order; a mod of maps
  alone is never named. When no mod explains it, it says so: other game files, so the installs
  differ. `Hello` and `Refusal::Version` keep their layout (a test holds their bytes), so
  builds of other protocols still tell each other they differ. A joining player waits for the
  answer (`Client::wait_for_answer`, on the connect page's worker thread) before the lobby
  opens, so a refusal is told on the connect page beside what asked for it. Connected names
  are unique ignoring ASCII case; chat senders are assigned from the connection. The
  network's fingerprint covers the winning VFS definitions, sides, build menus, scripts,
  models, AI and campaigns, but not the maps (`maps/`, from protocol 83): a map pack one player
  has and another lacks keeps nobody out, and the lobby checks the battle's own map. It folds
  SHA-256 file digests in canonical path order and keeps 64 bits for the existing wire
  field. Up to four workers read and hash files without caching potentially stale content.
* **Lobby**: host-authoritative slot list (humans by peer id, AI with difficulty, open),
  sides, colours, ready flags, map choice, game options and chat; `Start { map, seed,
  slots, rules }` makes every peer build the same `GameSetup`. In-game chat is relayed by
  the host (`Session::send_chat`); peer-issued cheats and host-only abandonment commands are
  refused at the network boundary. The protocol version (`PROTOCOL`) changes with the wire
  format or simulation behavior. `Client::poll` ends its batch at Start/Resume and retains
  later messages for the new session, including ticks and pace changes in the same read.
  Ally-chat and map-ping envelopes are authenticated to the connected player and relayed
  only to teammates and spectators; enemy clients do not receive those private messages.
  Spectators may send global chat, but cannot impersonate a player in private envelopes.
  The lobby relays no such envelopes (a player still in the last battle would read them), and
  names a lobby line's sender from its seat. New seats are dealt the game data's sides in
  turn (a mod's own, not ARM and CORE).
* **The battle's map**: once welcomed, a guest tells the host its maps (`Msg::Maps`: each map's
  file stem with the digest of its `.tnt` and `.ota`, `Compatibility::map_digests`, at most
  16,384), which the host's transport keeps for the connection, battles included
  (`Host::peer_map`), so whichever map the host chooses it knows at once who has it, even of a
  player still in the last battle's result. The host works out each person's state for the
  chosen map (`Lobby::map_status`: checking, ready, missing, another version: the same name
  with other contents) and sends it with the lobby. START BATTLE, and `--auto-start`, wait
  until everyone has the host's copy, spectators too (`Lobby::everyone_has_the_map`); the
  players' rows and the spectators' line say who lacks it, a guest's battlefield tells them
  theirs, the host's map list shows the maps someone lacks (their player count in red, who
  under the pointer) and the chat tells it once per map. `TA_LOBBY_MAP=<map>` chooses the
  battlefield of a lobby hosted from the command line (`--host`).
* **Coming back**: `Welcome` supplies a random 256-bit reconnect secret. After a player leaves,
  a connection must supply that secret and the same name to reclaim the slot (`Host::rejoin`).
  The game stores up to 64 credentials in the user's `reconnect.json`, scoped to resolved host
  endpoint, network data hash and normalized name, so restarting the application preserves rejoin.
  Maps are not in that hash, and the lobby's check does not see a returning player again: the
  host's session holds them back (`arriving`) until their maps, told just after the welcome,
  show the battle's map as the host has it (`Session::with_map`, set by the lobby from its own
  digest); with none, another version, or no word within `ARRIVAL_WAIT` (10 s) they are turned
  away (`Refusal::Map`) before anything is restored or sent: their army waits on, nobody is
  told they came or went, the battle goes on (no live ticks reach them meanwhile).
  Secrets are redacted from debug output; this is slot authorization, not encrypted transport.
  The host then sends `Msg::Resume`: the start info, their player and every
  tick's commands so far (the host keeps the non-empty bundles), a long history ahead of it
  in `Msg::History` parts of at most `BATCH_BYTES` that `Client::poll` puts back together.
  The host sends history incrementally as its outbound queue has room, holding live ticks
  for that peer until Resume has been queued. Received history is bounded and checked for
  strictly increasing ticks. Missing empty ticks are synthesized lazily without allocating
  an entry for each elapsed tick. The returning client
  replays them at full speed (`Session::resumed`; the game simulates for most of each frame
  while far behind), the host waits for it as for any slow player, and play goes on in
  lockstep.
  The host retains at most 128 MiB of reconnect history. On reaching that bound it clears
  the history, announces that reconnect is unavailable, and continues the active match;
  transfers in progress are closed rather than handing out incomplete history, and only
  absent players' credentials are forgotten (the next battle's rejoining needs the others').
  A returning player comes back on a new peer number: the host's roster, the lobby to go back
  to and the start it sends on resuming all take it, and `Msg::Returned { old, new }` tells the
  others. A player back before the host noticed their old connection die is let in on their
  credential, the old connection closed (its `Left` first). While a long battle is on its way
  the joining page shows REJOINING THE BATTLE and how much has come
  (`Client::history_received`).
* **Losing the connection**: a client tells apart the host leaving (`Bye`), the host going on
  without it (`Removed`, then `Bye`: `Session::dropped`) and the connection failing (no word:
  `Session::link_failed`). The last two offer REJOIN (`Transition::Rejoin` connects again to
  the address kept by `Session::with_rejoin`; the credential lets them in), and the failed
  link's words allow that the host may have stopped too.
* **LAN discovery** (`lan.rs`): a hosting lobby broadcasts a small UDP announcement (port
  7891, `TAR2`: a random lobby number, protocol, data hash, TCP port, host name, map, players,
  open places, the battle's name, its seats and whether it takes a password) once a second, to
  the network and to this PC; the connect page lists what it hears, one card per lobby number
  (a lobby on this PC is heard both ways and joined by its loopback address), so LAN players
  need no address. A game of another version is greyed out (OTHER VERSION); one whose data
  hash (the network's, no maps) differs is tagged OTHER MODS OR DATA, its JOIN left on so its
  refusal says which mods differ. Each poll processes at most 128 announcements and retains
  at most 256 games.
* **Spectators**: `Lobby.spectators` lists people who watch (`SetSpectator`, the lobby's
  "Watch instead" button, or `--join ADDRESS --watch`). Newcomers to a full lobby, and
  players in slots beyond the map's start positions, watch too. They run the same
  simulation with `local_player: None` (full vision, a vision picker like replays), their
  commands are dropped, and the host does not wait for them.
* **Integrity**: peers report `Sim::checksum()` every 15 ticks (`HASH_INTERVAL`); the host
  compares with its own history and broadcasts `Desync`, which every HUD shows as an
  "OUT OF SYNC" banner. Only connected players' hashes of already-produced report ticks count;
  spectators cannot inject desyncs or advance the progress gate. Clients reject out-of-order
  live ticks. Peers may only command their own player.
* **Clients catch up** by stepping several bundles per frame when a backlog builds.
* **Pace**: pause and game speed are the whole game's. Anyone may change them
  (`Session::set_pace`: a client sends `SetPace`, the host applies it and broadcasts
  `Pace` with who did it). The hash reports double as progress reports: while a peer is
  more than `MAX_LEAD` (75) ticks behind, the host stops producing ticks and broadcasts
  `Waiting` so every HUD names the slow player — everyone plays at the pace of the slowest
  machine (or the one still loading), as in the original. After 20 s of waiting the host
  may go on without them (`Session::drop_waiting` tells them so, `Msg::Removed`, and closes
  their connections; they may come back); the others are told it can. Escape opens the
  BATTLE MENU, which says the battle goes on (it pauses nobody). `Session::paused_by` names who paused (the HUD's PAUSED BY banner, with RESUME
  for anyone playing). What the session announces is a `Notice` (paused, resumed, speed, back,
  the reconnect limit) that the game words in the player's language.
* **Who is in the battle**: every session knows its roster from the `StartInfo` (the host's
  own, a client's through `Session::with_roster`): each peer's player and name, so the round
  trips the host measures (`Msg::Pings`, every `PING_INTERVAL`, in the lobby and the battle)
  are shown by player (`Session::player_pings`: the top bar's bars for the slowest, the
  players panel's PING column), and "left the game" names the host. While a player who left
  may come back, the host tells everyone once a second who is away and the seconds left before
  an ally takes over their army (`Msg::Away`, `Session::away`).
* **Back to the lobby**: the host's session keeps the lobby the battle came from
  (`Session::with_lobby`). Once the battle is over for everyone (one team standing), the host's
  result screen offers BACK TO THE LOBBY: `NetLobby::after_battle` turns the session back into
  the host (`Session::into_host`: taking newcomers again, no rejoin list), keeps the seats of
  those still connected (nobody ready, the others' seats open again) and broadcasts
  `Msg::Rematch(Lobby)`. A player's session marks `rematch` and keeps the lobby as it changes;
  `Client::poll` ends the battle on the connection (a later `Start` is a new battle's), and
  their BACK TO THE LOBBY (the result screen, or a banner while watching) turns the session
  into the client (`Session::into_client`). The hand-off happens before the menu's warm-up,
  during which nothing is polled (`App::draw`, `Transition::ToLobby`). A player still on the
  result screen when the host starts the next battle goes straight into it: the session keeps
  the `Start` and leaves the next battle's messages unread (`Session::next_battle`,
  `NetLobby::next_battle`). WATCH HIGHLIGHTS is not offered in multiplayer (it leaves the
  session), and the profile's pages are not opened over a lobby (whose network the menu's
  drawing pumps).
* Worker socket writes time out after 10 s. A peer whose live traffic fills its bounded
  outbound queue is disconnected immediately, without waiting on the game thread.
* Tested by `crates/ta_net/tests/lockstep.rs` (two simulations over loopback TCP, four
  minutes of AI play, identical checksums) and two real game processes:
  `--host 7890 --auto-start 2 --net-check-tick 1200` / `--join 127.0.0.1:7890 --net-check-tick 1200`
  (`TA_NET_AIS=2` on the host adds two hard computer players, whose orders every machine
  derives)
  (add a third process with `--watch` to check a spectator). `crates/ta_net/tests/hostile_commands.rs`
  has a guest and the host send commands of every kind with any values (others' units, made-up
  types, places off the map, others' players, cheats) through the real session over real data:
  nothing may panic, a player's abandonment and cheats are never bundled, the machines stay
  identical.

## Replays and saved games (`apps/ta_game/src/replay.rs`)

* A game is its `GameSetup` plus the commands of each tick. New records include both
  human and AI orders; legacy records regenerate the AI's decisions. Playback still
  advances AI brains, discarding their replacement orders for a complete stream, so a
  saved game can continue live. Catch-up records the full executed stream, including
  AI-only ticks, when upgrading a legacy save. Every real game is kept as a replay when
  left (`replays/*.tarep` under the active profile), one the player lost and watched on
  (KEEP WATCHING makes them a spectator) included: `Game::keeps_replay` asks whose battle it
  is, not who watches it, and a multiplayer battle is a real one for its spectators too (every
  tick of it reached them); "Save game" writes the same record as a save (`saves/*.tasav`,
  single player).
* Replays play the orders back to a spectator (speed controls work); saves re-simulate
  their ticks in 14 ms slices per frame behind a progress bar, then continue live.
  Replays seek the same way: ahead by fast-forwarding (`Playback::seek_to`), back by
  starting the record over (`Transition::Rewind`, preserving camera, perspective, speed
  and pause state) and fast-forwarding. Construction preserves the recorded local player
  before switching to spectator view: Conquest resources, Commander XP and veterans
  depend on that identity. Seeking clears old effects, restores wreck heat and refreshes
  the skin perspective. Playback and highlight clips stop at the recorded final tick.
* File format (`RECORD_VERSION = 46`) and simulation revision (`SIMULATION_VERSION`) are
  separate. Verified compatible body layouts 36–40 remain readable and have frozen
  fixture coverage; 41 adds bounded compatibility metadata rather than changing commands;
  42 a Conquest battle's reinforcements, 43 the skirmish mutators (older setups read and
  written again in their own layouts: `setup::LegacySetup`, `SetupV42`, `LegacyRules`),
  44 the constructors on duty (a bounded byte list after the setup, empty in a record
  before it, which is written again without it; client state only, so no simulation or
  protocol change), 45 a mission's allowed units in its data hash, 46 the mods it was made
  with (a trailer after the body, docs/FORMATS.md; `Record::mods`, `None` before 46).
  Unsupported layouts and corrupt/trailing data fail validation. The menu reads files on
  workers and keeps summaries rather than all command histories in memory.
* `ta_data::Compatibility` caches content digests when assets load. Network identity is the
  fingerprint of every input but the maps (`Compatibility::network`, from protocol 83; the
  lobby checks the chosen map's own digest, `Compatibility::map_digest`), and which layers give
  anything besides maps (`Compatibility::gives_data`: what a data refusal names); format 41
  records compare shared rules plus the map and mission used in that battle. Adding an
  unrelated map or campaign listing therefore does not invalidate a new save. Legacy records
  still compare the global fingerprint (every input, maps included), which also keys caches.
  Save-time lookups use cached digests without archive reads.
* Saves resume only with a compatible simulation revision and matching data. Readable
  historical replays may be watched with changed data or engine builds; the menu and
  replay controls label this as **Compatibility playback**, with a warning that AI,
  physics and outcomes may differ. Recording AI orders reduces one source of drift; it
  does not make changed physics or missing mod assets equivalent. Replay viewing awards
  no career progress. Files are written atomically through private temporary names.
* **Records and their mods** (`record_mods.rs`, `menu/record_prompt.rs`; shared:
  `ta_config::records::mods`, `ta_ui::record_mods`): every record names the mods mounted
  (`record_mods::InPlay`, made in `main.rs` and remade by the map editor's remount: each mod's
  id, name, version and `ta_vfs::mods::content_digest`, worked out on a thread as the game
  starts; a record written before they are done waits for them). A record whose data hash is
  not today's is asked about as it is opened (the saved games and replays page, Continue, a file
  from Explorer or the launcher): each mod it needs not installed, off, another version or other
  files, each on that it was made without, another load order (`ModCheck`), and what that does
  to a save (played over into another battle) or a replay (it does not show the battle it
  recorded). USE ITS MODS AND RESTART saves its load order (`ModCheck::load_order`: its mods,
  and map packs of the Maps page kept on) and starts the game again on it (`record_mods::restart`:
  the install, profile and a capture's arguments kept); GET IT IN THE LAUNCHER starts the
  launcher with `--mod <id>` (one running already is called to the front and shows it:
  `instance::ask_for_mod`); LOAD ANYWAY opens a save of today's simulation with today's data
  (`Record::opened_anyway`, checked by `validate_anyway`), WATCH ANYWAY a replay as
  Compatibility playback. The list says which mods each record uses, or the first that is not
  as it was. The launcher's Your battles reads the mods from each record's end and asks before
  CONTINUE or WATCH when they differ from those it would start the game with (by version: it
  reads no mod files), offering to turn them on and play, to GET a missing one from the
  catalogue, or to open it anyway (`--anyway`: the game then asks nothing). Records before 46
  name no mods: they are told by their data hash alone, as before.
* Headless tests compare live, legacy and full-command playback checksums and verify
  legacy-save conversion. Native `--net-check-tick` probes can compare a battle, its save
  and its replay at the same tick with matching inputs.
* **The replay's big moments** (`replay_scan.rs`): as a replay opens (`Transition::Play`, not a
  save), a copy of the battle at its first tick (`Sim` and the `SkirmishAi`s, which think along
  as in playback: their orders count when the record has only the humans') runs through the
  whole record on a thread of its own, its losses tallied as the highlights tally them
  (`Tally::note`, `Tally::marks`: `find_moments` over every loss, no copies kept). Every 900
  ticks it shares what it has found (`ScanState`: how far, the marks so far); a 25-minute
  battle takes about 5 s. The replay bar's timeline draws the marks over its track (colour by
  kind, size by worth, a tooltip naming each; a click jumps 3 s before it) and the scan's
  progress along its foot. A rewind hands the scan on (`ReplayScan::is_for` the same record);
  leaving the replay drops it, which stops the thread.
* **The cast** (`hud/cast.rs`, `Game::cast`): a broadcast's view of a replay or of spectating.
  Two players head to head across the top (their colours, names and sides, the clock between,
  the armies' worth as a tug of war: armed mobile units' metal plus energy / 60; income, units,
  kills, the three furthest-along things their factories build), three or more in a table
  with headed columns. Notable kills (`Game::note_kill`: Commanders, mobile units of 200 metal
  and buildings of 400 and up) come into a feed down the right, under the scoreboard where
  the window is too narrow for both, for 8 s. The deck's forces-and-economy plate steps aside
  (the scoreboard has those; a selection still shows), and the replay or spectator bar leaves
  the picture after 3 s without the pointer, a key or a click (`Hud::controls_shown`; not
  while paused or pointed at). The action camera directs (`app.rs`): every 2 s it looks at
  `Game::fights` (units seen firing, in 512-unit cells with their neighbours: the middle, how
  many fire, twice their RMS spread); it holds a fight 3 s at least, moves to another more
  than 700 units away once that one is half as busy again (after 5 s) or its own dies down,
  cutting to one over 1,600 away and gliding to one nearer. In the cast it frames the fight
  (`highlights::zoom_for` the spread) in the open picture under the scoreboard. Scrolling,
  a middle drag or (in the cast) the wheel takes back the camera.
* **A replay made into a film** (`film.rs`, `ta_mp4`): the replay bar's EXPORT VIDEO dialog
  chooses what (the whole battle, a stretch dragged out on a track carrying the scan's marks,
  or the ten worthiest big moments one after another, those that meet joined), the picture
  (720p to 4K, 30 or 60 a second, bits a pixel by quality), the camera (the director, a
  cinematic one at pitch 0.72 slowly circling the director's fight, or the view as set; the
  cast's framing is 0.8 of a fight's spread, the cinematic 0.7), the cast's scoreboard and
  kill feed, titles and sound. The film is made on the game's own frames: `State::frame`
  steps the battle 1/fps a frame (`Film::dt`), the player's input ignored; `film_after_update`
  waits for each stretch's start (`seek_replay`: fast-forward, or a rewind), then swaps in an
  offline `GameAudio::film` (the player's balance at 0.9) for the speakers' and rolls;
  `film_picture` runs a second pass of the main egui context at the film's size (ppp the
  picture's height / 600, so it is laid out as the game is: the HUD's unit pictures and
  portraits are that context's textures) for the cast (`Hud::film_overlay`, on the film's
  clock) and the titles (`Film::overlay`: the battle's card, the big moments named at their
  height, fades), draws the picture under it with `Renderer::render_image_with` (the scene's
  targets kept at the film's size meanwhile, `set_film_size`), mixes 48,000 / fps frames of
  sound, faded with the picture, and hands both to `ta_mp4::Mp4Writer`. Because egui takes
  an Area missing from the pass before as newly shown (and fades it in from nothing), the
  film's interface is drawn without Areas fading (`fade_in(false)` on the cast's, layer
  painters for the titles) and the window shows only the film's page, drawn into its root
  Ui (`Film::page`: the latest picture now and then, progress, time left, CANCEL; at the end
  SHOW IN FOLDER, WATCH IT), presented some twenty times a second (`Renderer::render_ui`, else
  `skip_frame`) so that vsync does not hold the film back. A 1080p60 film is made at about 90
  pictures a second here, 720p30 at about 125. Films go to `Videos\TA Reborn`
  (`ta_config::videos_dir`) as `<map> 001.mp4`; a film left unfinished leaves no file.
* **Social media clips** (`clips.rs`, `clip_dialog.rs`, `app/clip_play.rs`): the replay bar's
  EXPORT CLIP makes 30 or 60 seconds of the battle, vertical (9:16, 1080 × 1920) or horizontal
  (16:9, 1920 × 1080), the stretch chosen by the game. The replay's scan also keeps what went on
  second by second and where (`ActionLog`: shots, a fifth of a shot where fired from and the
  rest where aimed; blasts by their area of effect; losses by worth; a nuclear strike's and a
  Commander's fall more; the places it gathered in, 256-unit cells round the busiest, with their
  spread; and every big blast's tick and place). `candidates` scores every stretch of the length:
  how busy each second is against the battle's own busy level (its 90th percentile, above it on a
  log scale, counting more when it is in one place), weighted to count more late in the stretch,
  the climax (the busiest moment, smoothed) placed between a half and most of the way and clear
  of the end tag, the action rising to it, a nuclear strike or a fallen Commander in it, a dead
  start counting against; the best four that do not overlap much or share a climax are offered,
  each started where no big blast went off in view in the second before (`clean_start`). `plan`
  directs the camera: anchored on the climax's busiest place, it follows that fight forward and
  back in time (the busiest place within 760 units), holds where nothing goes on, cuts to the
  busiest elsewhere only after a shot of four seconds, never just after the start or before the
  end, and is smoothed both ways in time (so it moves before the fighting does), a Catmull-Rom
  glide between its keys a second apart. The camera is the perspective one, lower than the
  battle's (pitch 0.9 vertical, 0.84 horizontal), sweeping some twenty degrees across north,
  framing the fight by the picture's narrow side (vertical 1.6 of its spread across, closer),
  its focus in the middle of the part social apps leave clear (`Shape::safe`: vertical 13% top,
  25% bottom, 14% right, 6.5% left, the strictest of TikTok's, Reels' and Shorts'; horizontal a
  player's title and controls). The dialog offers the stretches as cards: a picture of each
  climax (`ClipPreviews`: the replay taken to a moment before it, played up to it, paused and
  drawn in both shapes, the battle quiet behind the dialog; it goes back where it was when the
  dialog closes without a clip), when, what happens and a line of how the action builds; the
  best chosen. The clip is a film (`FilmSettings::clip`, `Film::clip`): it opens on the action
  (apps take the first picture for the cover), the titles over it (`ClipLook`, a point a pixel at
  1,080 across: the map and who fought, a tug of war of the armies' worth across the top of the
  safe part, the worthiest big moments named at their height, an end tag with the emblem
  darkening the picture), its sound lifted 3 dB and held under -1.5 dBFS (`Limiter`), coming in at
  once and going out over the end tag. H.264 High at about 7.5 Mbit/s (0.12 bits a pixel at 30 a
  second: a minute is under 60 MB), AAC 192 kbit/s, the index first (`ta_mp4::faststart`: `moov`
  moved before `mdat`, chunk offsets shifted, done in Rust after Media Foundation, whose own
  moov-first remux wrote a broken file). Clips go to `Videos\TA Reborn\Clips` as
  `<map> <mm-ss> <shape> <n>s 001.mp4`. Captures: `TA_CLIP=30v|30h|60v|60h` (`TA_CLIP_PICK=n`,
  `TA_CLIP_DIALOG=1`, `TA_CLIP_MUSIC=1`, `TA_CLIP_SOUND=0`, `TA_CLIP_TITLES=0`,
  `TA_CLIP_SCOREBOARD=0`).
* **The companion screen** (`companion.rs`, `hud/companion.rs`): a second window, filling a
  display other than the battle's when there is one (`other_display`, borderless), else a
  window beside it. It has a surface of its own on the battle's device (Fifo, in turn with its
  display: drawn at most every other refresh of it, `pace`, a frame is handed over without
  waiting; Mailbox, on NVIDIA's Vulkan driver, waited a whole refresh in `present` on the
  battle's thread, the battle's frame p95 33 ms in place of 9; three hand-overs in a row over
  4 ms halve its rate, down to 7.5 a second), its own egui context, input state and renderer, and a second
  `Hud` and `Minimap` whose pictures and side skin are loaded in its context; the portraits its
  panels ask for are drawn by the battle's studio and registered with its renderer. On Windows
  it is made a window that does not activate (`WS_EX_NOACTIVATE`, and `WS_EX_APPWINDOW` to stay
  on the taskbar), so a click on it looks or orders while the keyboard stays the battle's.
  `App::window_event` hands it its own window's events first; closing it closes only it.
  `State::companion_frame` draws it after the battle's frame, some thirty times a second:
  `Hud::companion_page` lays out a header (the map, the clock and pace, whose battle, a close
  button: a display it fills has no other), the minimap as large as filling the page's height
  leaves room for two columns of plates of 230 points at least (one on a narrow screen; its
  clicks act on the battle: look, send the selection, ping), and the plates: the economy (the
  top bar's stores over the deck's charts, taller), the forces (one column where they fit;
  the deck's force rows now cut their names short of their counts), production (the
  factories, the idle first, a click selecting and looking; the idle constructors' line a
  click to the next one), the events (the battle history, newest first, an attack's with
  LOOK at the alert posted with it; under the map when a wide map leaves 100 points there),
  and while watching the players' cards (name, side, army worth against the largest, income,
  units, kills, what they build) in place of production. Its key
  (`companion`, Alt+C) or the pause menu opens and closes it (`Game::companion_toggle`); the
  `gameplay.companion` setting opens it with every battle when there is a second display, and
  otherwise a battle with one says once that it can be had. Captures: `TA_COMPANION=1` (or
  `--open companion`, a battle `TA_RESULT_MINUTES` in) open it hidden and
  `TA_COMPANION_SIZE` large (1920x1080), and `TA_COMPANION_SHOT=<file>` at
  `TA_COMPANION_AT=<frame>` saves its picture (drawn again offscreen: a window's surface
  cannot be read back).
* **`ta_mp4`**: Media Foundation's sink writer on a thread of its own (COM there, its own
  apartment), the graphics card's encoder where Windows offers one, else its own (the writer
  is opened again without hardware transforms when the first will not take the pictures).
  H.264 High at the film's size, rate and bits; NV12 in, converted from RGBA on that thread
  (BT.709, studio range, a band of rows a core; `rgba_to_nv12`), the stream tagged BT.709;
  AAC-LC 48 kHz stereo at 192 kbit/s from 16-bit PCM. Pictures queue three deep; each
  sample's time is counted in whole pictures and sound frames, so the sound never drifts
  from the pictures. `cargo run --release -p ta_mp4 --example film -- out.mp4 1920 1080 60 5`
  writes colour bars and a tone to try a size. `Mp4Writer::create_fast_start` (clips) moves the
  finished file's index (`moov`) before its pictures (`faststart.rs`), so it plays as it downloads.

## Campaigns

The briefing (`briefing.rs`) plays before a mission: START MISSION on the campaign page makes
the mission's `GameSetup` and a `Briefing` (the page draws only it while it is open). TA's
MessageArrived beep (`UiSound::Message`), the narration (`camps/briefs/<narration>.wav`;
`wav_envelope` uses the playback decoder to read its length and both channels' loudness
sixty times a second for the trace; unreadable audio falls back to text), the words
revealed in time with it (`Briefing::shown`: over 90% of the recording after a short lead,
30 characters a second without one), the glamour picture, and the theatre of operations: the
map's own minimap with the mission's player-1 units and start pulsing. BEGIN MISSION or
Enter starts it; `--open briefing` (with `TA_BRIEF_AT=<seconds>`) for captures. The pause
menu's briefing replays the narration through `MusicRequest::Narration`.

* **Data** (`ta_data::mission`): `camps/*.tdf` list each campaign's mission OTAs. A mission
  OTA adds briefing keys (`brief`, `narration`, `glamour`, `useonlyunits`) and objective
  keys to the map OTA, and per-difficulty schemas place units with an `InitialMission`
  script (docs/FORMATS.md has the language): `m x z` move, `p x z` patrol (points loop),
  `w n` wait n seconds, `wa [ident]` wait until attacked, `a name` attack a complete ident or
  group before treating it as a unit type, `o move fire` standing orders (move order first),
  `g ident` guard,
  `s` stop, `i ident` start aboard that
  transport, `b TYPE [n]` a factory builds (`b TYPE x z` a constructor), `bw n` missiles,
  `u [x z]` set one unit down, `d` self-destruct. `ta-tools missions` lists what it can't
  read (TA's own slips).
* **Objectives**: every victory goal must be met (TA's rule, `MissionRules::all_victory`;
  scenarios keep any one), any defeat goal loses. Victory: DestroyAllUnits,
  KillAllMobileUnits, KillEnemyCommander, KillUnitType, KillAllOfType, CaptureUnitType,
  BuildUnitType, MoveUnitToRadius (a building named: one finished there; any unit: mobile
  ones), UnitTypePassesX/Z (a line crossed away from the start side,
  `ta_data::mission::Line`), VictoryTimerRunsOut (seconds). Defeat: AllUnitsKilled,
  CommanderKilled, AllUnitsKilledOfType (anyone's), UnitTypeKilled, AnyUnitPassesX/Z (an
  enemy unit), DeathTimerRunsOut, and a `MissionCriticalUnit` of the player's lost
  (`Objective::UnitLost`). A count of 0 means 1. Evaluated in the sim (`ta_sim::mission`)
  every 15 ticks.
* **Setup** (`ta_sim::mission::setup_mission`): TA player n = sim player n-1 (1 = human);
  health %, facing (degrees from south), idents and groups, passengers boarded. No
  commanders are spawned; the mission's own units, resources, `lineofsight`/`mapping`, unit
  whitelist and `maxunits` (`Sim::unit_limit`) apply. Capture targets are exempt from
  automatic fire; units placed with `Immunity=1` take no harm (`Sim::is_immune`, in the
  damage hook `Sim::cheat_damage`). The editor's AI keys: `AiIgnore` (`Sim::ai_ignores`:
  computer players' units never target it automatically, `Sim::find_target`, and `ta_ai`
  leaves it out of its survey), `AiPriorityTarget` (`Sim::is_priority_target`: their
  automatic fire, and `ta_ai`'s waves, air strikes and focused fire, go for it first),
  `BuildPriority` (`Sim::build_priority`: `ta_ai`'s factories make it where they would a
  combat unit). The player's `InitialGroup` digits come back as `MissionSetup::
  control_groups`, which the client makes its control groups. Every computer player of a
  mission has a `SkirmishAi` (`GameSetup::mission`); it sets its placed guns that hold their
  fire (the Big Berthas, Intimidators, Doomsday Machines TA builds on hold fire) going as it
  does its own (`SkirmishAi::arm_defenses`), once no script runs them. The player's placed
  units were never built: `setup_mission` gives them their file's `StandingFireOrder`
  (`sim::file_fire_state`), so I09's and T31's Annihilators fire at will and the Berthas wait
  for the player, as their files say; what anyone builds comes out on TA's hold fire.
* **Scripts** run in the sim (`UnitScript`, every tick from `check_mission`) as steps: runs
  of orders the unit carries out itself (`Move`, `Patrol`, `Wait`, `WaitAttacked`,
  `AttackType`, `Guard`, `SetStates`, a constructor's `Build`), each followed by an end mark
  (a wait of years) whose reaching the front starts the next step; between them what orders
  can't say: a factory's production (its queue) and the way its units leave (its rally), a
  transport setting one unit down (an `Unload` cut short after the first), self-destruct,
  what is chosen when the step comes (`g`/`a` naming a group or a unit type: the nearest;
  `wa` naming a group: any one attacked). Passengers' scripts wait until they are set down,
  their opening waits counted from the start. Boarding checks ownership, cargo size and free
  space; passengers reserve it as they board so a type/group can use the next free transport.
  Explicit `TransportMaxUnits` overrides the legacy capacity (the hovercraft of the last Arm
  Core Contingency mission carries six); air transports still carry one.
  A script ends with its unit, a change of side, or the end mark gone (someone gave
  the unit orders of their own); computer players leave units to their scripts
  (`Sim::runs_mission_script`).
* **Capture**: `CanCapture` units (the Commander) convert enemy units like reclaim:
  time ∝ (metal + energy/60 + 20) / worker time.
* **Client**: campaign page (unlocked missions, glamour art, colour-coded briefing,
  objectives, narration, difficulty), objective rings on the ground and minimap, mission
  outcome screens, progress in `progress.toml`.
* `ta-tools missions` validates every campaign mission against the game data.

## Scenarios (`apps/ta_game/src/scenario.rs`, `menu/scenarios.rs`, `hud/scenario.rs`)

* **Files**: TOML, built in (`apps/ta_game/scenarios/*.toml`, `include_str!`) and from the
  settings folder's `scenarios/` (the page reads the folder when it opens; a bad file is
  logged and skipped; one that needs a map or unit the game lacks is listed, greyed, with
  why). A scenario names its map, a `kind` (survive, endless, hold, escort, strike, clock,
  race, battle: the list's tag), a briefing, the player's force and the enemies' (side,
  start, resources, Commander, placed units with `patrol`, `hold`, `xp`, and `from` /
  `until` a difficulty; an enemy's `ai` is a computer player, "chosen" the difficulty
  picked, or none: its units follow their orders), goals and waves. It may add allies
  (`[[ally]]`, forces of the same kind on the player's team, after the enemies in player
  order), a skirmish's mutators (`[mutators]`, `Sim::set_mutators`: a ceasefire, resources,
  kinds of unit left out), the time of day (`time`: hours, or keys of [seconds, hours] eased
  between, going round the clock past 24: `stage::Hours::at`, over the settings' sky, under
  `TA_TIME`), rain (`rain`, under `TA_RAIN`) and transmissions (`[[message]]`: `at`, `from`,
  `text`, `side` "player", "enemy N" or "ally N", `from_difficulty`), which come in on the
  comms card (`hud/comms.rs`) as the AI's chatter does, with the speaker's side's Commander
  on its screen and HEADQUARTERS, ALLIED COMMAND or ENEMY COMMAND under the name.
* **Positions** are words that fit any map (`Ground::place`): `start N` (the map's start
  positions), `centre`, compass points, `X% Z%`, `X Z` pixels, then `toward <place> by
  <pixels>` and `+ DX DZ`. Units go on rings round the spot (`Sim::place_units_bound`:
  buildings on the nearest free sites; ground units only where they can reach the other
  side's start, trees included, so none begins in a clearing it can't leave).
* **Goals** become `ta_sim::mission::Objective`s: `survive` (a timer to victory), `hold`
  (`HoldArea`: the player's side alone in the circle, ground units, for so long in all;
  `enemy_holds` the reverse), `reach` (a unit, or the Commander, into a circle), `destroy`
  (the enemy Commanders, everything, the army, or every unit of a type: with the waves
  still bringing one, `KillAllOfTypeToCome`), `metal` /
  `enemy_metal` (made in all), `time` (defeat when it runs out), `lose` (the Commander,
  everything, a unit type). "Destroy everything" waits for the waves still due; once
  none are, the last five or fewer enemy units show on radar (`Game::last_enemies`).
  Distinct hold circles keep separate cumulative friendly/enemy clocks in
  `MissionRules::holds`; the HUD reads the clock for each goal's own position/radius.
  Destroying the Commanders also waits for enemy Commanders scheduled in future waves.
* **Waves** (`Wave`, sent by `check_mission`): units of an enemy appear at a place (several
  split by `|` take turns) and attack-move to a place or the player's nearest forces;
  `every`/`times` repeat them (`times` 0: until the end, capped at 200), each `grow` times
  bigger; counts scale with the difficulty (Easy 0.6, Hard 1.5) but `boss` units'. Their
  units then hunt (`hunt`, every 2 s): idle ones attack-move to the player's nearest
  forces; ones that have hardly moved for 6 s with somewhere to go (wedged in a crowd)
  each get a point of their own near it. A unit whose every weapon fires on order only (a
  bomber: TA's bombs are `commandfire`, aimed of their own accord only on patrol or
  attack-move with fire at will, and TA's bombers stand on hold fire, so an attack-move only
  flew it over the player's base) is given an attack order on the nearest of the
  player's side its own side sees (in sight or on radar: `Sim::seen_by`, never through the
  fog) that its weapons can hit (`Sim::engages`: not an aircraft in flight, nor a bad
  target), then an attack-move to where that stood; it keeps on it while it stands and is
  seen, then takes the next, and seeing none attack-moves on as the others do (the skirmish
  AI's strikes order their bombers the same way, `Order::Attack`). `stay` keeps a wave where
  it was sent (the hill's defenders). A player that sends waves is `endless` (never defeated for having no
  units). `SimEvent::WaveArrived` puts a notice and an alert up. Reinforcements are waves
  too (`player = true`, or `ally = N`: `Wave::friendly`): they go to `to` or wait where they
  come in, hunt nobody, and are neither counted with the enemy's waves nor waited for by a
  goal; their notice says so.
  Zero-count entries and non-boss repeats after zero growth spawn nothing. Captured or
  transferred friendly hunters leave the hunter list; carried hunters are not retasked.
* **Setup**: `Scenario::setup` makes the `GameSetup` (its `scenario` carries the id, the
  difficulty and the whole file, so saves and replays play the same whatever becomes of
  it); `Game::new` places the forces from the file (`Scenario::apply`) and leaves out the
  computer players of passive enemies. The HUD strip lists the goals with their clocks,
  bars (the hold, metal) and the waves; the pause menu the briefing; the end panel PLAY
  AGAIN / SCENARIOS / MAIN MENU (the title menu: `result::home_page`). Records
  (`CampaignProgress::scenarios`: won per difficulty, best time:
  the fastest win, the longest endless siege) skip games with cheats.
  Shared preflight checks all sides, unit references and destinations before spawning;
  commander resolution retains duplicate-race side keys. Invalid goal fields, difficulty
  words and time keys are rejected. A requested Commander that cannot be placed aborts setup.
  The simulation checksum covers mission goals, per-area progress, waves, hunters,
  scripts, restrictions and goal history. Simulation revision 65 / network protocol 68
  separate the corrected behavior from older peers and saved-game resumes; record layout
  remains 46.
* **Balance** is checked with an ignored test that plays scenarios with a computer player in
  the player's place, or none, or a scripted run (`scenario::tests::autopilot`: `cargo test
  --release -p ta_game autopilot -- --ignored --nocapture`, `TA_SCENARIO=<id>`). One game is
  one seed and swings a lot: judge a change over many (`TA_TEST_SEED=n`, 7 by default; the
  release QA used 21 a scenario and difficulty). `TA_TEST_LOG=1`
  says what fired and killed by kind, what killed the Commander (its last attacker too: the
  killer is often gone by then) and what still stood on hold fire a minute in;
  `TA_TEST_WATCH=CORSHAD,CORINT` when each of the enemy's units of those kinds fell, and how
  often they fired each minute. `TA_PILOT_TO=x,z|x,z` sends the player's side to several
  points in turn (a scripted run: a radar post, then a mesa).
* **TA's targeting rules** shape them: anti-air units never fire at the ground on their own,
  ground units (the Commander's laser, the towers) never at aircraft, and `NoAutoFire` guns
  start on hold fire, which a computer player sets going, or a placement's `hold = true`
  (Iron Rain's Intimidators are their battery's, a second Core force nobody plays: set going
  with `hold`, on stores of their own, they fire at what a radar post in the canyon sees of
  the Arm base, every eight seconds each, as its briefing says; the post falls and they see
  only what the Core's own units see). A wave's gunships go for whatever of the player's they
  meet first, the Commander too; a wave's bombers go for the nearest of the player's units
  their side sees (above). Bombers that bomb left the autopilot's win rates as they were (21
  seeds a scenario and difficulty, Last Stand 42): the Defenders and the player's other
  anti-air bring most Shadows down within a pass or two.

## Modding

* Any TA mod works as data: drop `.ufo/.hpi/.ccx/.gp3` or a folder into `mods/`; the VFS
  layers it over the base game in the enabled order (`mod.toml` for metadata, dependencies,
  load-after).
* **A mod's archive named like one of the install's replaces it** (`ta_vfs::mods::mount`,
  `replaced_archives`): copying a mod into TA's folder overwrote the original's archives of its
  archives' names, so with the mod on the install's archive of that name (in any case:
  `totala1.hpi`, `ccdata.ccx`, `btdata.ccx`, `rev31.gp3`...) is not mounted at all, and nothing of
  it the mod's lacks leaks through; the mod's own is mounted with the mod, over everything of the
  install's. Whatever its archive holds: an empty placeholder blanks the original's. TA: Enhanced
  ships its whole game as `rev31.gp3` with stubs of `totala1.hpi` and `ccdata.ccx` and an empty
  `btdata.ccx`: layered, the install's 125 other unit types (TA's navy, Core Contingency's, Battle
  Tactics') crept into its menus and the computer players' builds, their weapons missing. A mod
  may name archives it takes the place of beyond its own (`mod.toml` `replaces = ["btdata.ccx"]`):
  the installer writes the placeholders it finds beside the content there instead of copying
  them, and an author may name archives a player was told to remove. Mods TA Reborn knows by their
  catalogue id get what their installation removed whatever their manifest says (`mods::KNOWN`:
  TA: Enhanced's `btdata.ccx`, and the six post-release units and the free unit packs its readme
  has removed), so a copy installed before needs no reinstalling. A mod that does not read
  replaces nothing (no mod is mounted then, and the install is whole), and without mods the
  install is mounted as it always was (`Vfs::mount_game_dir`: the same data hash). Every VFS of the
  game's content is built by `mods::mount` (`mount_enabled` resolves the load order first): the
  game, the map editor's remount, the launcher's mod checks, mod details, skin previews, profile
  and maps scan, `ta-tools` (`--mod`, `validate-mod`), and the tests' `TA_TEST_MOD`; what reads
  the install alone (the launcher's banner pictures and interface sounds) mounts it whole.
* All gameplay derives from FBI/TDF/COB data; the AI chooses units by properties, so it
  plays new units without code changes.
* Maps made in the map editor are mods too: `%APPDATA%\TA Reborn\mods\<map name>\`
  with `mod.toml` and `maps\<name>.tnt/.ota`, enabled when saved. New IDs append a stable
  hash of the lowercased file stem to `map-<slug>`, so punctuation and non-Latin names
  that share a slug remain distinct. Existing editor maps keep their legacy IDs.
* **Folders of a mod's own** (`mod.toml` `[rename]`, `ta_vfs::Renames`): many mods came
  with a copy of TA's engine patched to read, say, `guiM` in place of `guis`. Mounting such
  a mod first hides what the layers beneath have in the folders it replaces, then mounts
  its files under TA's names. An archive of a mod that cannot be read (the empty
  placeholders some ship to blank out the original's) is skipped with a warning; named like
  one of the install's, it still replaces that one.
* **Installing a downloaded mod** (`ta_vfs::package`, used by the launcher and by
  `ta-tools mod-install`): `Unpacker` unpacks the zip, rar or 7z with 7-Zip when installed,
  else Windows' own tar (bsdtar stops without a word inside some RARs, so an unpacked total
  under half the package's size is an error asking for 7-Zip); `plan` takes the folder
  holding the largest TA archive (found by its `HAPI` header, whatever the extension), the
  valid archives and TA folders beside it and the documents (into `docs/`), leaves out
  programs, settings, placeholders and extras elsewhere in the package (the placeholders beside
  the content are named in the manifest's `replaces`: they blanked the original's archives of
  their names), and finds the
  renamed folders: a folder whose name starts like one of TA's (`unit`, `unitpic`, `weap`,
  `gui`, `game`, `download`), mostly holds that folder's kind of file (`.fbi`, `.pcx`,
  `.tdf`, `.gui`), where the mod has none of TA's own; `install` lays it out as
  `mods/<id>/` with its `mod.toml`, replacing an earlier version.
* **Sides** come from the data (`setup::side_choices`): a mod's third race (Talon's TAL),
  or sides of its own (Total Battletech's Inner Sphere and Clans) are offered in skirmish
  and the lobby. A name several sides share keeps the first; the others get keys of their
  own (`ARM#2`, labelled with their Commander), which `resolve_side` turns back into the
  side and its Commander. The title screen names the mods the game runs with.
* **Testing a mod**: `ta-tools --mod <folder> data` loads its data over the game;
  `TA_TEST_MOD=<folder>` (and `TA_TEST_SIDES=TAL,CORE`) has the AI soak test play it.
* **Automatic mod validation** (`ta_data::validation`, launcher `validation.rs` and
  `pages/mod_report.rs`; `ta-tools validate-mod` from the command line): installed and
  discovered mods are checked in a background queue, with persistent reports tied to their
  content, dependencies, base game and the checks' version. Files are decoded in parallel a
  chunk at a time and folded in path order (the same report on any core count). Each
  finding has an origin: the mod's, caused by it (in inherited content, only with the mod:
  `validation::compare` against a baseline of the same content without it, kept by its own
  fingerprint), or inherited (not counted). Fingerprints cover only the files the game reads,
  their digests kept between launches by size and times. Beyond formats and references the
  checks cover TDF structure with line numbers (`tdf::lint`), sounds (WAV codecs, sound
  categories, weapon sounds), duplicate and shadowed definitions, the production pages the
  game reads, packaging (files where the game never looks, nested archives, a wrapping
  folder) and scripts' Query functions. The report groups findings by what they concern
  behind filters with counts and a search. Static findings drive installed badges; curated
  gameplay reviews remain separate. See [MOD_VALIDATION.md](MOD_VALIDATION.md).

## Conquest (`apps/ta_game/src/conquest.rs`, `conquest_ui/`)

* A galaxy of up to 10, 16 or 24 systems (at least four available maps), each a skirmish
  map, laid on a jittered grid from the conquest's seed: our home at the left end of the
  middle row, the enemy capital at the right. Lanes are a minimum spanning tree plus short
  links to near neighbours that cross no other lane. Threat (1 to 3) grows with the jumps
  from home; with the level (Relaxed, Standard, Hard) it sets the enemy's computer players
  (the capital fields two when its map supports three players). Generation reserves a
  suitable capital map before assigning the other systems, if one is available.
* Every system but home and the capital brings a `Reward` to whoever holds it, dealt from the
  seed (`rewards_for`: each kind in turn, shuffled): mines and reactors add to the supply,
  a tank plant or an airfield sends a fresh unit to each battle (`reinforcements`, at most
  four: Stumpy or Raider, Brawler or Rapier), a war college gives the Commander half a point
  of experience after each victory, a sensor array charts two jumps out, a supply depot adds
  a veteran berth (twelve at most). `charted` says which systems are known: ours and their
  neighbours (two jumps with a sensor array), all once the war is won; the rest show only as
  uncharted worlds (the capital keeps its name).
* Each promotion of the Commander (its rank rising to veteran, hardened, elite: `best_rank`,
  kept though a defeat halves its experience) earns a `Perk`, chosen from three offered
  (`perk_offer`, drawn from the seed and the choice's number, never one already taken) and kept
  for the war: Combat Engineer (it builds and repairs half as fast again), Reinforced Armour
  (30% less damage), Marksman (its weapons a quarter harder), Field Instructor (our units' kill
  experience half as much again), Capacitors (its D-gun at half the energy), Quartermaster (a
  quarter more supply). `ConquestStart::perks` names them; `Game::new` gives all but the
  quartermaster's to the simulation (`battle_perks`, `Sim::set_perks`), whose `perks` module
  applies them for that player only, where each changes the rules (build power, damage dealt
  and taken, kill experience, `Sim::shot_energy`). The choice opens after the battle's report
  (or later, from the empire's panel), under the campaign's lock (`choose_perk_at`).
* After a victory (not on Relaxed, nor before the third battle) the enemy may strike back
  (`strike_back`, drawn from the seed and the battle: 40% on Standard, 65% on Hard) at a
  system of ours beside one of theirs, never home: a `Counter` from the strongest enemy
  neighbour, landing `COUNTER_DUE` (2) battles later. It is beaten off by a battle there
  (`defendable`; `setup` makes it a defence, `ConquestStart::defence`, against the attackers'
  strength), broken up by taking the system it comes from, or else it takes the system
  (`fallen`); a lost defence loses it at once. One at a time; the report of the battle that
  began, broke up or ended one says so (`threat`, `broken`, `fell`).
* An enemy system next to one of ours can be attacked: `Conquest::setup` makes the battle's
  `GameSetup` with a `ConquestStart` (bonus metal and energy for the systems held, the
  Commander's experience, the selected veterans up to the slots and the systems'
  reinforcements), or returns `None` when the
  target is invalid or its map no longer has two starts. The page explains unavailable
  battlefields and disables ATTACK. `Game::new` applies it while making the game
  (`deploy_veterans` rings them around the Commander on passable, unoccupied ground,
  `Sim::set_experience`). Each veteran searches independently, and undeployable or unknown
  units are retained for the reserve. Deployment and this retained list are reconstructed
  deterministically for saved games and replays.
* Deployment defaults to the strongest veterans. The Veterans tab can choose a custom
  group or send only the Commander; choices are persisted against the exact displayed
  campaign under the same transaction lock as results. Stale or failed writes preserve
  the visible army and offer Retry/Reload; a pending failed selection blocks ATTACK.
  A battle's recorded starting veterans determine casualties even if the menu selection
  changes while that battle is saved. A completed battle resets selection to automatic.
* When the battle ends, `result_of` lists our ranked mobile units still standing; the
  saved conquest (`conquest.toml` in the active player's data folder) takes the result if
  its seed, battle number and attackable target still match. Won, the system is ours (the
  capital ends the conquest), and survivors join the reserve that stayed home; lost, the
  veterans actually deployed are gone and the Commander keeps half its experience. Units
  that could not deploy stay in reserve and are excluded from casualties. Leaving a battle
  unfinished changes nothing. Each report keeps its details (`ReportDetails`: who came home,
  who was newly ranked, who fell, the Commander's experience before and after) for the
  journal's latest 12 battles; older entries keep only their counts, and validation checks
  the details against the counts.
* Persistence distinguishes a missing save from an unreadable one. `load_at` accepts only
  an ordinary file of at most 1 MiB and validates topology, indices, finite positions,
  ownership/progress consistency and experience before the UI or game uses them. Format 2
  added rewards and counter-attacks (format 1 saves are dealt their rewards when read).
  Progress holds when every victory took a system or beat off a counter-attack and every
  system lost fell to one (`wins == held - 1 + fallen + defended`); the journal is replayed
  battle by battle (what it says of each system's owner must hold from one battle to the
  next and agree with the galaxy now), and a counter-attack must come from an enemy
  neighbour of one of ours, not home, due within `COUNTER_DUE` battles. Records before
  version 42 laid out `ConquestStart` without reinforcements or defence: `setup::LegacySetup`
  reads them, and writes them again in their own layout. An invalid
  save is retained and shown with Retry/explicit abandonment; it is not silently replaced.
  Creation, result read/merge/write and abandonment share a `SettingsTransaction` file lock.
  Creation refuses any existing save; results reload after locking, and stale/duplicate
  outcomes or retries after abandonment do nothing. Writes are validated and atomic.
  Ordinary abandonment compares the displayed campaign with the current save before deleting
  it; the unreadable-save recovery path also rechecks under the lock and refuses deletion
  once the save is readable again. A failed create/delete keeps the visible page state and
  reports the error. Pending results retain their original player path and frozen outcome.
* The pages (`conquest_ui/`): `setup` chooses a new conquest (side, level, size, and a
  preview of the galaxy its seed makes); `galaxy` paints the map; `panels` the side panel (the
  empire's tabs, or a system); `briefing` the battle's briefing; `report` the debrief after a
  battle and the campaign's end.
* The galaxy map is painted with egui shapes: deep space with nebulae and twinkling stars;
  each side's territory (the systems' cells, circles clipped by the bisectors with their
  neighbours, `territories`), the front line where ours meets theirs; lanes (ours solid, the
  front's in flowing amber dashes, the enemy's dashed); worlds in their planet's colour
  (`planet.rs`, Earth and asteroids among them), ringed in the owner's, the capital crowned,
  home marked, the threat in pips; names placed clear of each other, the rings and the lanes
  (`place_labels`). Systems are buttons with names for screen readers; the arrow keys move
  between neighbours, Enter briefs, a double click too; a card follows the pointer. The route
  to the capital can be shown, and a captured system rings out when the galaxy comes back.
* The side panel shows the empire (the campaign's progress and route, its veterans and the
  journal of the latest 128 battles, each with its report) or a system (its battlefield,
  tags, the enemy's strength, your force and supply, the battles fought there), closing with
  its cross; ATTACK and VETERANS stay in a bar under it however it scrolls. ATTACK opens the
  briefing: the battlefield with each side's start marked and numbered, the objective, the
  enemy, your force; LAUNCH BATTLE (Enter) or BACK. After the battle the debrief tallies who
  came home, who was newly ranked and who fell (with their portraits), what it meant for the
  Commander and the campaign; taking the capital ends in the campaign's own page (its record,
  the veterans, a new conquest). In battle a strip under the top bar (`hud/conquest.rs`)
  shows the system at stake, the enemy, the goals and the veterans standing; the result
  screen says whether the system was taken or held.
* Old campaigns keep their totals and begin a journal with the next result. The completed
  campaign can still be reviewed. Compact layouts put the map in a scroll area and the
  details in a sheet over it rather than reducing the selected text scale; Conquest uses the
  requested interface scale without the legacy menu fit cap, and ordinary wide layouts keep
  the side panel. The floating main-menu profile card shows on the title screen alone
  (`app::shows_identity_card`): over the other pages it lay on their way back (Conquest's MAIN
  MENU and battle reports, the other pages' BACK) and took its clicks. Reduced-motion mode freezes decorative animation, and normal
  animation requests are limited to a 33 ms cadence.
  `--open conquest-demo|conquest-target|conquest-briefing|conquest-defence|conquest-perks|conquest-report|conquest-won|conquest-battle|conquest-victory|conquest-defeat`
  capture a conquest some battles in (its counter-attack under way, its tank plants and
  airfields taken first, its Commander's perks chosen), its briefings, a promotion's choice, a
  debrief, the campaign won, a battle and its result (never saved; `TA_CONQUEST_DEFEND=1` makes
  the battle its defence).

## Interface languages (`ta_ui::i18n`)

* English text is the key: `t("SKIRMISH")` looks it up in the loaded language and falls
  back to the English, so untranslated text still reads. Languages are TOML files
  (`[language] name`, `[strings] "English" = "Translation"`) in `lang/` beside the
  programs, in the settings folder's `lang/` (players' own) and, in debug builds, the
  repository's `lang/`; `gameplay.language` picks one (launcher and in-game options; the
  in-game switch applies at once). `TA_LANG` overrides it; `pseudo` brackets and accents
  whatever passes through `t`, so plain text on screen is text that isn't translatable yet.
* The shared widgets translate their labels themselves (buttons, panels, sections,
  settings, switches, segmented choices, captions, tags, menu entries), which covers most
  of the interface; other text calls `t`, text with values `tf` (`{name}` placeholders),
  and constants (tips, lessons, overlay names) are marked with the no-op `n(..)`.
* `ta-tools notices <file>` writes the third-party notices from `cargo metadata`: the
  libraries compiled into the game and the launcher (normal dependencies from both, for a
  target triple, `--target`, default Windows; procedural macros and build tools left out,
  as their code never reaches the programs), each with its licence and source, and every
  licence text once with the libraries it covers: the files at a package's root
  (LICENSE*, COPYING*, NOTICE*...) and licence-named ones below it (vendored C code, the
  fonts egui embeds). A library without a licence file gets the standard text of the
  licence it is used under (MIT when offered) from `installer/licenses`; a GPL text is left
  out where the library also offers a permissive licence; MPL-2.0 libraries are listed
  with where their source is. The build scripts put it beside the programs.
* `ta-tools i18n lang` scans the game, launcher and `ta_ui` sources for strings whose
  innermost call translates them (looking through tuples, arrays, blocks and `Some`),
  writes `lang/template.toml` grouped by source file, and adds new strings to each
  language (translations kept, unused ones listed at the end).
* Also collected: the built-in scenarios' `title`, `tagline` and `briefing`
  (`apps/ta_game/scenarios/*.toml`, shown through `t`; players' own scenarios stay as
  written), and the strings given to `message` (the map editor's translate themselves).
* Longer words: menu entries shrink their title and line, then cut them short
  (`widgets::fitted_galley`). The classic interface letters with TA's own bitmap fonts what
  they have every letter of; other text (Cyrillic, Chinese...) is drawn in the interface's
  font at their size (`TaFont::covers`), and TA's own button words ("Options Menu") are
  translated where the template has them.
* Chinese, Japanese and Korean (`theme::cjk_faces`): Bahnschrift has none of them, so the
  system's own faces follow it in every family, read at runtime (Windows: Microsoft YaHei UI,
  Microsoft JhengHei UI, Yu Gothic UI, Malgun Gothic, regular and bold; elsewhere Noto Sans
  CJK), never shipped: the interface language's first (its bold in the strong and display
  families), so the characters Chinese and Japanese share take its shapes, then Chinese and
  Korean for names and chat in them. Font files are read once and shared (`font_file`); a
  face is only used when its table directory reads (`has_face`: egui panics on fonts it
  cannot parse). A change of language puts the fonts in again at the next frame
  (`follow_language`, on `i18n::generation`). Their text is set solid
  (`i18n::tracking` zeroes the capitals' letter spacing).
* Shipped: `lang/de|es|fr|it|ja|ko|pl|pt-BR|ru|tr|uk|zh-Hans|zh-Hant.toml`, machine
  translations of every string (made one language per agent from chunks of the template,
  each keeping a glossary, checked for placeholders and fitted to the buttons' widths;
  `tools/i18n`: the brief, the glossaries, and the scripts that split the template, gather
  new strings and merge translations).
  `TA_I18N_MISSING=1` logs each text shown without a translation in the language in use
  (what the template lacks shows up there, names from the game's data among it).
  Translated text put in capitals goes through `i18n::upper` (Turkish i is İ).
* The first start (launcher's or game's) sets the language to the computer's
  (`hardware::system_language`: Windows' display language, `LC_ALL`/`LANG` elsewhere) when
  `i18n::closest` finds a translation: the same tag, else the same language; Chinese by
  script (Taiwan's, Hong Kong's and Macao's traditional).

## Launcher (`apps/ta_launcher`)

Sounds (`sounds.rs`): the install's archives are mounted on a thread of their own, then an
`AudioEngine` plays TA's interface sounds (`UiSounds`, `gamedata/allsound.tdf`) for what
the shared widgets ask (`ta_ui::sound`, enabled while `Audio::launcher_sounds` is on);
events are played after the current frame's widgets and audio settings, so clicks and mute
take effect immediately. Captures stay silent. The same engine plays the launcher's music
(`Audio::launcher_music`, at `launcher_music_volume` of the music volume): TA's calmer tracks
(`playlist`: the briefing's drone, 17, and the building music, 11 to 16, shuffled, four
seconds' fades and gaps; the player's own music as the settings have it) on the briefing
mood. `MusicFade` eases its level (1.6 s the whole way) toward its share, or to nothing
while the game runs or the launcher is minimised, the playlist asked for as it comes back
and stopped once silent. The header's music control (`music_control.rs`, beside the
profile; in the compact header beside the title): the note turns it on and off (lit, three bars
dancing under it while a track plays; dim and struck through when off), the arrow opens a
panel with the track playing, the volume and NEXT TRACK. `--show music` shows it as if a
track played, its panel open. Motion: pages slide in (`page_since`), a brass marker follows the page
in the menu (`animate_value_with_time`), the PLAY button's gleam (every five seconds), the
hero's radar (`radar`, redrawn at 30 frames a second), notices sliding in, and the
deployment on PLAY (`paint_launch`, 1.8 s, while the game starts).

* **Navigation**: one ordered list of pages (`app::PAGES`: Play; Mods, Maps, Skins; Profile;
  Settings, Benchmark, About) drives the rail, its groups (a seam between them), Ctrl+1 to
  Ctrl+8 (read even while a search box has the keyboard) and the F1 list. A rail item's
  badge says what needs attention, its tooltip why (`nav_badge`): the game not found, a mod
  that is on with errors, else the updates waiting, the benchmark running. In a small window
  (`compact_layout`) a narrow rail of icons (`nav_compact`, `ta_ui::widgets::nav_icon_item`:
  lamps and badges kept, names and keys in tooltips) replaces it, the compact header names the
  page and carries PLAY beside the profile. The logo (the emblem and TA REBORN, in either
  header) is the website's link: under the pointer or with the keys it warms to brass and its
  subtitle reads TA-REBORN.COM ↗; a click or Enter opens https://ta-reborn.com/
  (`app::WEBSITE_URL`) through `Action::OpenUrl`, as every link opens (`website_link`). The
  project's GitHub page is on About (SOURCE ON GITHUB). Quick start leads a commander yet to
  finish a battle to the tutorial; the recent saves preselect the newest that can still be
  continued, and one that cannot says why in words.
* **Launcher interaction polish**: static labels cannot be selected; text fields keep
  normal editing and selection. Footer
  save indicators are vector marks centred on their first text line. Original steel/brass
  contextual pointers (`cursor.rs`) follow egui hit testing, UI scale, cursor size and
  contrast preferences; they only replace the native pointer inside the focused window.
  Interface settings can disable them to retain the operating system's accessibility
  pointer. `TA_CURSOR_PREVIEW=1` displays an artwork sheet for visual QA. Dossier geometry
  remains stable across loading, tabs and achievement filters; identity/navigation stay
  visible while tab contents scroll. Maps replace placeholder OTA titles using campaign
  metadata or the filename, and select an available playable map by default.
* **The mod library** (the Mods page: `mod_library.rs` holds its items, filters, order
  and the background work out of what a mod changes, `pages/mods.rs` the library and the
  details, `pages/mod_order.rs` the load order; the shared grid, cards and pictures are
  `library.rs`'s). The catalogue's mods and those installed are one list. A mod's changes
  are counted by mounting it alone and looking each of its files up in the game as it would
  load before it and after it. After installs, removals and games `ModValidation::sync_changed`
  checks only the mods that changed and those needing them, however indirectly; F5 checks
  them all, reading every file again.
  Mods can be hidden from the library (`ta_config::Mods::hidden`, ids in lower case, in each
  profile's settings as the maps' favourites are): HIDE FROM LIBRARY in a mod's menu or its
  details. A hidden mod leaves the library's lists and counts (the LIBRARY tab, "n of m mods",
  UPDATE ALL and the rail's update badge) and nothing else: one installed stays installed, and
  one that is on still loads, the load order listing it with a HIDDEN tag (the rail's red
  badge for a mod on with errors still counts it). The toolbar's HIDDEN · n switch (there
  once anything is hidden) lists only the hidden mods, searched and filtered as the library
  is, each card with its ribbon and UNHIDE over its picture, UNHIDE ALL beside the switch;
  leaving it chooses again what was chosen before. The game asking for a hidden mod
  (`--mod`, MANAGE SOURCE IN MODS) opens it there (`show_mod_details`).
* **Context menus** (`library/menu.rs`): a right-click on a card or a list's row of the Mods,
  Maps or Skins library (or the load order's rows) chooses it and opens a plate of what the
  page offers for it, built afresh each frame from what it is now: a vector mark, the action
  in capitals, the page's key for it, and for what cannot be done now the reason under its
  name (what can never apply to it, removal of a built-in skin say, is left out). The plate is
  the hints' metal, heavier: the item's name and what it is head it, a brass edge along its
  top; the hovered or focused entry has a brass lead and a warm band (rust red for removal).
  While open it is egui's top modal layer: the page under it takes no clicks (a click outside
  only closes it) and no keys; Escape, a click outside or the wheel over the page close it.
  Shift+F10 opens it on the card with the keys, its first entry focused; the Menu key comes as
  Shift+F10 (`menu_key` reads VK_APPS as the frame begins: egui has no key for it); Up, Down,
  Tab, Home and End go from entry to entry that can be chosen (egui's own moves of the keys
  held back), Enter chooses, and the keys come back to the card. A right-click outside an open
  menu closes it and, in the next frame, opens the menu of the card or row it was on
  (`hop_id`), as a right-click elsewhere does in Windows; each opening is a popup of its own
  (`Opened::serial`). `--film` steps can right-click (`button = "secondary"`) and press keys
  (`do = "key"`).
* **Community mods** (`catalogue.rs`, `moddb.rs`, `getmods.rs`, `pages/getmods.rs`: a
  catalogue mod's controls and pictures in the library's details): cards credit their creators and show compatibility
  evidence for the tested release, when available. A preferred project page is separate
  from the release provider: `Entry.page()` opens the project, while `source_page()`,
  `source_name()` and `moddb()` retain download resolution and attribution. Total Mayhem
  links [its project site](https://mayhem.tauniverse.com/totalm.htm) while keeping the
  verified 11.3.0 ModDB download and a separate ModDB reference link.
  GET resolves the provider's file, downloads on demand, checks pinned size and digest,
  and safely unpacks/plans/installs it (`ta_vfs::package`). "Have the file?" applies the
  same verification to local copies of pinned releases. Final Frontier remains a
  verified manual-only import. Each install has a private workspace; publication follows
  successful validation, with the previous install retained on failure.
  [Escalation's official downloads page](https://taesc.tauniverse.com/?p=downloads) is its
  preferred source. Gold 10.2.0 (8 August 2026) is pinned to the supplied official archive's
  exact size and SHA-256 and imported manually through the same checked pipeline. The
  website challenges automated requests, so no obsolete ModDB download is offered.
  Compatibility is `Fit::Plays`: AI factory classification and mod tech-level tags
  allow armies to develop, every unit is painted (only the textures models name, in up to
  two arrays) and every explosion drawn (HD art decoded a frame at a time and drawn
  reduced). What is left is the mod's own: a few wreck models and four weapons it never
  ships. It is the catalogue's spotlight, first in its order, ahead of Total Mayhem. Unpinned `Fit::Unverified` entries still support safe local imports; their
  manifests use the explicit `unverified` version rather than guessing from filenames.
  Every catalogue card displays version, release date and archive size, with explicit
  unknown values when the publisher has not supplied them. Published metadata is separate
  from verified installation files. Sorting by newest/oldest release or smallest/largest
  archive uses this metadata, puts unknown values last and retains deterministic ties.
  Spotlight order is the default; choosing a sort does not pin a featured card above it.
  PLAY starts the game with the installed mod. One conversion at a time: turning one on
  turns the others off. Eligible ModDB feeds and pictures are cached for a day and a month
  (`cache/moddb/`); no project archive is fetched merely by browsing. Creators and download
  hosts remain credited, and files are never re-hosted. `--get-mod <id>` starts a supported
  automatic install (manual entries return instructions), `--offline` prevents network
  access, and `--mods-tab installed` opens the other tab.
* Pages: Play (install detection, launch), Mods, Settings (everything in config.toml,
  saved as edited: the settings screen of the game's options, `ta_ui::settings`, with
  RESET ALL over it), Benchmark (below), About (the project, the legal position and licences, with buttons
  that open `LICENSE.txt` and `THIRD-PARTY-NOTICES.txt` when they sit beside the launcher,
  as in an installation). The game reloads nothing from the launcher while running; the
  launcher reloads the settings when the game exits (the game may have changed them).
* **Benchmark** (`benchmarks.rs`, `pages/benchmark.rs`; the game's `benchmark.rs`, the
  report's `ta_config::benchmark`): RUN BENCHMARK starts the game with
  `--benchmark <settings>/benchmarks/benchmark-<seconds>.json --no-mods` (the launcher goes
  down to the taskbar, never closes: it waits for the report). The game plays the three staged
  scenes of `apps/ta_game/benchmark/` (Sapphire Reach at dawn, Highland Rift in the rain,
  Cinder Basin at nightfall with 3,500 units; written by `tools/benchmark/scenes.py`, units of
  a missing expansion left out: `missing = "skip"`) at the player's settings, vertical sync
  and the frame-rate limit off, the battle's interface hidden and its input ignored (Esc or
  the quit key stops it, with no report). Between the second and the third, the settings
  test: the highland battle is copied at its 16th second (`Sim` and the computer players, as
  the highlights copy it, and the scene's `StageRun`) and played again from the copy at Low,
  Medium, High and Ultra, then at High drawing about 1080p's pixels (the reference pass:
  `reference_scale`), each settling 1.5 s and measured 5 s; MSAA stays as the game started
  (it changes only then), the player's settings come back after. Every frame is measured: its
  time, the processor's work before drawing (`t_update`..`t_render`), the graphics card's
  (the timestamp queries' "frame" pass, taken when a new one comes), the simulation's ticks
  and their time; the memory is measured first (copying on up to 16 threads, a chain of
  random reads through 256 MB). The report (JSON) has each part's statistics (average, 1%
  and 0.1% lows, p99), a timeline in quarter seconds, the levels, the reference pass and the
  computer (`Hardware::detect`, the adapter's name and driver, the screen's refresh rate).
  The scores (`Report::scores`) compare it with the reference computer's (RTX 4080, Ryzen 9
  5950X, 64 GB: 10,000 each, the average of six runs at four MSAA levels and three window
  sizes, all within 2%): the graphics card by the reference pass's time, made 1080p's by a
  cost model (a fixed part, one per megapixel drawn, one per megapixel shown) and 4x MSAA's
  by a measured factor; the processor by the largest battle's simulation tick and the rest of
  a frame's work (geometric mean); the memory by bandwidth and latency (square roots) and its
  size; overall, their geometric mean weighted 0.5 / 0.35 / 0.15, with a class from Peewee to
  Krogoth. `Report::recommend` takes the highest level whose slowest 1% held 90% of 60 frames
  a second, MSAA no more than tested, vertical sync when the game outruns the screen, and a
  unit limit no higher than the processor keeps up with (two players at it, a tick within
  25 ms, cost growing as units^1.2); APPLY THESE SETTINGS writes them into config.toml.
  The page shows the scores (dials), the run in time (frame rate and its slowest frames over
  the processor's and the graphics card's milliseconds, each part's band, the values under
  the pointer), each battle, the settings test, the recommendation, the computer and the
  last ten runs (forty kept). `--autobenchmark` runs it as the launcher starts.
* **Loading** (`ta_game::loading`, `ta_ui::loading`): the game paints a dark, animated
  loading screen before revealing its native window. Renderer creation, asset loading,
  initial menu warm-up, maps and benchmark scene setup run on a worker; the window's
  event loop keeps painting hints and handling resize, focus, Esc and close requests.
  Only the loading presenter acquires/configures/presents the shared GPU surface while
  work is in flight. The renderer is returned and resized before normal frames resume.
  Benchmark scene changes and settings-pass rewinds use the same interstitial; loading
  contributes no simulation ticks or frame-time samples. Launcher and loading decorations
  share `ta_ui::seasonal`, including the calendar, enabled/theme preferences and bounded
  animation clocks; reduced motion, reduced flashes and focus rules apply to both. In season
  `seasonal::notes` gives the season's own field notes, their heading and its colour, which
  `LoadingScreen::note` alternates with the usual tips (the season's first), and
  `seasonal::plate` dresses the notes' plate.
  Benchmark pointer, touch and
  gamepad input are suppressed before UI picking, and the native cursor stays hidden.
  **Out of graphics memory.** A full card refuses allocations: the buffer or texture comes back
  invalid. Nothing may panic for it. `ta_render::CheckedBufferInit::create_buffer_checked` makes
  buffers with contents as wgpu's `create_buffer_init` does but skips the write to an unmade one
  (wgpu's `expect`s the mapping: a battlefield loading on a full card crashed the game); clippy's
  `disallowed-methods` (clippy.toml) keeps `create_buffer_init` out. The device's uncaptured-error
  handler (`gpu::report_uncaptured`) counts refusals (`out_of_memory_events`) and writes a repeated
  message once a second with its count. A load that saw a refusal fails cleanly
  (`app::short_of_graphics_memory`): a battlefield goes back to the menu with the reason
  (`Loaded::BattleFailed`), the game's own start stops with it. egui-wgpu panics for what it is
  refused too: `ta_render::can_allocate` (a small mapped probe) is asked before each
  `egui_wgpu::Renderer::new` and before a load lends the renderer (the transition then goes back to
  the menu with the reason), and `update_buffers` runs inside `gpu::unless_short_of_memory` (its
  panic caught and only logged by the panic hook; the frame goes without the interface, the
  loading screen without its own for the rest of that load). For trying it: `TA_GPU_BUDGET`,
  `TA_GPU_BALLAST_MB` (CLAUDE.md).
* **Update check** (`update.rs`, `[updates]` in config.toml): at start, a background thread
  runs the system's curl on the update source, a TOML file (`version`, `url`, `notes`) or
  a GitHub latest-release API address (`tag_name`, `html_url`, `name`), and compares the
  version with the build's (numbers first; a pre-release is older than its release). A
  newer one shows as an UPDATE pill in the header and on the About page, which also has
  the switch, CHECK NOW and the source. `update::DEFAULT_SOURCE` is the build's own
  source (empty until a release sets it); captures only check `file:` sources.
