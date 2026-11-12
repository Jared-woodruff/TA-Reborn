# TA Reborn — status and roadmap

Where TA Reborn stands, area by area, and what comes next. `ARCHITECTURE.md` explains how
each part works; this file tracks what is done and what is still open.

## Done

**Platform**
- 64-bit Rust engine on the user's GOG install (registry or folder detection, GOG/Steam
  layouts); NSIS installer that requires the GOG edition and offers to open its store page,
  registers replays and saved games to open in the game, and ships with a portable archive,
  checksums and the symbols for reading crash reports. Licensed GPL-3.0-or-later, with the
  notices of every library compiled in (`ta-tools notices`).
- Launcher: install check; ModDB's Total Annihilation mods to browse and install in one
  click, each tried in TA Reborn; mods (enable and order them); the settings screen the
  game's options share (graphics, display, sound, controls with keys and mouse buttons).
- Mod support through a layered virtual file system: HPI/UFO/CCX/GPF/GP3 archives and loose
  files, later layers override earlier ones; the AI and the rules read unit properties, so
  mods play without code changes. Mods made for the original's patched engines (their own
  folder names, archives under other extensions) and a mod's own sides work too.

**Simulation**
- Deterministic 30 Hz simulation in fixed point: COB unit scripts, pathfinding with shared
  flow fields for big groups, TA's weapons (ballistics, guidance, beams, stockpiles,
  interceptors, paralyzers, D-gun), veterancy, cloaking, transports, construction, reclaim,
  resurrection, capture, economy with stalls, wind and tides, geothermal vents, fires in
  forests, alliances, sharing resources and units.
- 20,000 units battle at about 11 ms per tick (the budget is 33 ms; two armies of 10,000 on
  a map of 1,024 cells a side, one of them a computer player's); a frame with all of them in
  view, zoomed out over the whole map, takes about 16 ms to prepare and submit. A selection
  of thousands sent off together shares one distance field and sets off the next tick.
- Units give way (a rule, on by default; "As in TA" keeps the original's queueing): movers
  look a second ahead and bend round the units in their way, so columns meeting head on pass
  instead of jamming; planned from the tick's snapshot, the same on every core count.
- TA's line of sight choices: circular, true (traced over the ground's heights, so hills
  hide what is behind them) and permanent; acid seas that eat what is in them.

**Rendering**
- wgpu renderer: HDR with bloom, MSAA, sun shadows, point lights, fog of war, water with
  depth colour and foam, lava and void seas, sky and the world beyond the map edge, ambient
  occlusion, cloud shadows, heat haze, planar water reflections (shores, ships, explosions),
  GPU particles (hundreds of thousands), weather per world, tread marks, footprints, dust
  on dry worlds and vapour trails. TA's classic oblique camera is kept exactly; the
  perspective camera gets the extras.
- The relief's own shadows (ridges shade valleys; long shadows under a low sun), soft
  shadows that harden at the contact, a cinematic colour grade with a filmic tone map, fog
  of war that drains colour rather than light, explosions that read as fire and smoke,
  trees in the wind, rain that soaks the ground (puddles mirroring the sky, rings on the
  water), whitecaps and turquoise shallows, rock strata on cliffs the painted art never
  showed, ore crystals glinting on the metal deposits.
- Ray tracing (optional, graphics cards that trace rays in hardware): units and buildings
  traced for exact soft shadows, ambient occlusion, shadows from fires and explosions, and
  reflections in metal and puddles; the raster shadows where it is off.
- Motion with weight, for the eye alone: unit scripts animated between the game's ticks,
  vehicles on springy suspension (squatting, diving, leaning out of turns), hovercraft and
  aircraft banking, gunships tipping towards their motion, ships on the swell, submarines
  running under the water; tank treads that roll as they drive and turn.
- Deaths that play out: shot-down aircraft crash trailing fire, ships list and sink,
  buildings sag into their dust, big units cook off; units flash where they are hit, and
  wrecks come down glowing and cool to soot. Explosions keep their colours over pale ground
  instead of adding up to white.
- TA's own art enhanced while loading: smooth, anti-aliased outlines on trees and rocks
  (distance-field alpha), unit textures doubled with Scale2x instead of blown up into blocks.
- Two art styles, independent of camera and quality: **Reborn** by default, and **Enhanced
  Classic**, TA's models and paint with smooth curves and rounded, lit edges. Reborn rebuilds
  every model when the game loads, mods' included (`model/kit.rs`): armour plates with seams
  and bolts on its large faces, road wheels inside a track loop on its tracks' sides, shown
  where a unit is at least 56 pixels across; and its paint is remastered at 128 texels
  (`model/remaster.rs`), each texture's lit and shaded variants evened out, enlarged by
  palette snapping. A test fits the kit to every installed model and checks it keeps to each
  piece's extent. Original art is read from the player's install and never shipped.
- 3D trees and rocks: close to the ground, models of TA Reborn's own stand in for TA's
  painted trees, bushes, plants and rocks on every world, sized and coloured from the
  paintings; they sway in the wind, cast shadows, are mirrored in the water and burn, and
  hand back to the paintings as the camera rises.
- The ground's materials up close: sand, soil, turf, rock, gravel, snow, ice, volcanic rock,
  worn metal or lunar dust, told from the painted colour and slope and weighed by the world,
  in place of the paint's dither; metal worlds' walls are plating. Both styles cover all world
  families through this runtime pipeline, with Reborn's richer material relief and Enhanced
  Classic's stronger painted identity. Filtered distant detail protects strategy readability.
- HD surfaces: ground without its dither up close (smooth colour, fine grain, relief), models
  in physically based light (metal, lacquered paint, rubber read from their paint) and
  glowing, animated lights; units' edges worn to bare metal and their creases dark, and
  battle damage (soot, then embers through the cracks) as they lose health, worked out from
  the models so a mod's units get them too. Ground HD detail works in either camera and still
  turns off with the quality switch; the selected art style remains active.
- Time of day: dawn, dusk, a moonlit night under the stars, or a day-night cycle; at night
  bases, factories, units and aircraft light up.
- Strategic zoom: far out, units become icons in their owner's colour, their shape saying
  what they are.
- Photo mode: a free camera, the hour, a depth of field (a lens, or tilt-shift's miniature
  look), exposure, colour and vignette; photos at up to 4K without the interface.

**Game**
- Perspective: a view from inside a unit (Q) as it goes on with its orders: the player's own,
  an ally's, or anyone's when spectating or watching a replay; the view is the player's alone,
  so it works in multiplayer too. Seven cockpits modelled for TA Reborn (a tank's hatch, a
  kbot's visor, the Commander's helmet, a ship's bridge, a fighter's canopy, a gunship's glass,
  a gun house's slit) lit by the battle's light and their own lamps, instruments drawn on
  their screens' glass (what the unit is doing and its weapons' state, the unit, radar, speed,
  economy) and a heading display in the frame, a periscope for submarines; brackets on the
  enemy and the name, health and range of what the guns are on, hits, cracks and a lost
  signal, the magnified sight, the night sight, rain on the glass; the battle heard from the
  seat; the mouse or a gamepad looks round.
- Skirmish (up to ten players, teams, AI at three levels, starting rules, random starts),
  campaigns with briefings and TA's mission scripts, multiplayer (lockstep over TCP,
  lobby, LAN game list, spectators, chat, shared pause and speed, dropping stuck players,
  rejoining a game in progress, an ally takes over the army of a player who doesn't come
  back), replays with seeking, saved games.
- Scenarios: eight battles set up in advance with goals of their own (survive the waves,
  hold the hill, escort the Commander, an air strike, a doomsday clock, a metal race, a
  pitched battle, an endless siege), on three difficulties with records kept; written in
  TOML with position words that fit any map, so players can add their own.
- The chat line in single player, with the enemy's computer players answering in character
  (greetings, taunts, grudging "gg"s); TA's cheat codes and new ones, and a cheat panel
  with a spawn tool for any unit (single player only; replays play them back).
- Battle highlights: after a battle, its decisive moments (the costliest fights, nuclear
  strikes, fallen Commanders) replayed in order with a cinematic camera, straight from
  copies of the game kept as it was played.
- One look for the launcher and the game (`ta_ui`), after TA's own screens: metal plates
  in steel pipe frames, gunmetal buttons with stencil lettering, green lamps. Front-end
  pages, modal dialogs in battle (pause, options, statistics, controls, victory and
  defeat), a HUD in the side's own panel metal with icon order buttons; an icon for the
  executables, windows and installer, and installer art, all drawn from code.
- Map overlays (movement, heights, metal, sight and radar), area extractor orders, building
  grids, reorderable factory and construction queues, selection filters.
- A tutorial of three guided lessons (basics, a first battle, advanced orders) and tips on
  the loading panel and at the start of a skirmish.
- TA's intro on a CRT in a bedroom at night, around 1997, modelled for TA Reborn: posters,
  boxes, the manual and the keys' legends made from the install's own art, moonlight through
  the blinds with a tree swaying in it, a lava lamp, dust in the beams, the room lit by the
  movie on the tube with soft shadows and its reflections; or full screen, as before.
- Conquest: a galaxy of TA's worlds taken system by system against computer players that
  grow stronger towards their capital, veterans and the Commander's experience carried
  from battle to battle, supply from the systems held, saved after every battle.
- The first start set up for the computer: the graphics level for its card (Low draws the
  battlefield at 75% resolution for integrated graphics), the screen's own size, full screen
  and the language of Windows; a render scale, and antialiasing that follows the setting.
- The Archives: every unit, mods' too, in a TA world of the player's choosing with all it
  can do, and desktop wallpapers of it from 1080p to 8K; the Galactic War told chapter by
  chapter and world by world; credits that roll TA's own screen and teams from the install,
  then TA Reborn's.
- Briefings as TA gave them (an incoming transmission with its narration), computer players
  who greet, taunt and answer all battle with faces of their own, portraits for players, a
  chat history; a jukebox for TA's soundtrack and the player's own music.
- TA's details honoured: its interface sounds, function keys (F1 to F3), chat commands,
  Ctrl+click and the Ctrl+letter selections, the mine and plan marks under Shift.
- The launcher alive: pages that slide in, the PLAY button's gleam and radar, a deployment
  when the game starts, and TA's own button sounds.
- Colour-blind colours: players (units tinted to match), health bars, movement map and
  placement marks.
- An update check in the launcher (a TOML file or GitHub's latest release).
- Interface languages: translation files keyed by the English text, the shared widgets
  translating their labels, a language setting, a pseudo-language, and a tool that gathers
  every string into a template. Thirteen translations ship (German, French, Spanish,
  Italian, Brazilian Portuguese, Polish, Russian, Ukrainian, Turkish, Japanese, Korean,
  Simplified and Traditional Chinese), machine-made; Chinese, Japanese and Korean in
  Windows' own fonts; the first start picks the computer's language.
- Gamepads (the Steam Deck's controls included): sticks scroll and aim a cursor, A and B
  click in battle and in every menu, triggers zoom, bumpers hold Shift and Ctrl.
- Linux: released for x64 and ARM64 since 1.0.0, built natively on Ubuntu 22.04 by the
  release workflow (`.github/workflows/packages.yml`, `installer/build_linux.sh`) as an Ubuntu
  package, an Arch package and a portable archive with a per-user `install.sh` and menu
  entry; Linux check and test jobs in `quality.yml`. The install found in Steam libraries,
  Heroic's record of its GOG installs, and Wine, Lutris, Bottles, Heroic and Proton prefixes.
- The Steam Deck: recognised at the first start (Steam's `SteamDeck=1`, the firmware, SteamOS
  or its graphics), which sets the game up for it: full screen at 1280 × 800, vertical sync,
  the interface at 125%, the launcher maximised (full screen inside gamescope, Game Mode); the
  Medium level for its graphics.
- HUD with TA's build pictures, hover cards (waiting for the pointer to rest), range rings,
  area orders, factory routes, formation moves, control groups, camera bookmarks, TA's
  animated cursors, statistics; a top bar that speaks up only when a store stalls, runs dry
  or overflows; orders acknowledged where they go, the selection's routes shown for a moment
  and while Shift is held; a side panel with the forces at a glance, the economy's last
  minutes, what each unit is doing, order buttons with their keys, standing orders that
  name their setting, and a constructor's build menu on tabs by purpose; a control-group
  bar, and idle alerts pointing at the constructor or factory out of work;
  or TA's own 1997 interface (side panel, build menus, order buttons and resource bar from
  the install's GUI files, art and fonts, and TA's own shortcut keys).
- Map editor: new maps (size, sea, ground, any stock map's tiles) or copies of existing
  ones; height brushes, tile blocks from any map, copy and paste, features of the planet,
  start positions, map properties, undo and redo, all shown at once in the 3D view; saved
  as a mod (TNT and OTA written in TA's own layout) that the skirmish page lists right away.

**AI**
- Economy, tech, defences, raids, attack waves, air strikes, nukes; navies on sea maps
  (shipyards, fleets, underwater extractors, tidal power); metal worlds.
- Waves sized against the enemy army it has seen, pushes that finish a beaten enemy, and
  on Hard retreat-and-repair (warships too) and concentrated fire; a ladder test keeps the
  levels apart (Hard beats Normal 223-97 on the land maps and 186-134 on the sea maps,
  Normal beats Easy 80-0), with each game played under several seeds and both player
  orders. Computer players draw their random choices from the game's seed and take turns
  spread evenly over the think cycle (player 0 used to see player 1's moves sooner and won
  59% of games between equal AIs).
- Across water and cliffs: transports carry the stranded land army (by air, or by
  hovercraft and ship where those suit), after a look at the enemy, in waves that land
  together away from anti-aircraft cover and towers; units that cross by themselves
  (hovercraft, amphibians) and their plants come first; fleets push on coastal buildings,
  underwater extractors and fleets, fall back when outgunned and come again reinforced.
  Metal on islands no ground constructor reaches goes to constructors that fly or hover
  (an air plant goes up for them): about a fifth more metal on sea maps, and a stranded
  start that ended with one extractor now takes thirty.
  On ten water and island maps a game now sees 84 units killed instead of 25 (before,
  none at all on the maps nothing but aircraft can cross), and Normal finishes Easy
  outright in 5 of 40 games (before: 1); given 40 minutes, 5 of 40 games between Hard and
  Normal end outright there (before: none).
- Hard's air doctrine for island wars: a scout finds the enemy Commander and bombers massed
  for it strike it together (against Hard without it at sea: 45 of 80 games won, 24
  outright to 6).

## Open

- **Further Reborn art**: modelled detail for more of TA's parts (barrels with a bore and a
  muzzle brake, hatches, vents, landing gear, ships' rails), the paint's functional markings
  (runways, landing pads) checked one by one, and a lighter Reborn texture set for small
  graphics cards (compressed, or built only for the units in play).
- **Continuous quality checks**: Windows/Linux workspace check/test jobs now live in
  `.github/workflows/quality.yml`. Hosted runners do not have the original game's data;
  installed-game and GPU visual checks remain local release work.
- **Linux**: the code keeps to cross-platform crates (winit, wgpu on Vulkan, cpal on ALSA,
  gilrs on udev, rfd through the XDG portal) and guards the Windows-only parts; the release
  workflow builds and packages it, and CI checks and tests it, but on hosted runners with no
  screen and no game data. What is left is play-testing on Linux machines, the Steam Deck in
  particular: its first start's setup in Game Mode and Desktop Mode, the launcher full screen
  under gamescope, and installs found in Proton prefixes and on an SD card.
- **Beyond 20,000 units**: the slowest ticks of a 20,000-unit battle (25-40 ms, about one in
  a hundred) bring a distance field over most of the map up to date when wrecks fall by its
  goal, one core's work each; such a field could be split, or brought up to date over
  several ticks. Path searches keep to an allowance a tick that costs up to 15 ms when
  hundreds of units plan alone at once (cut off by trees after a big order); they could run
  on all cores in batches. Orders and weapons still run on one core. An order sending
  thousands of units each to a place of its own still goes as one order a unit (1 MB for
  20,000); a group order would be a fifth of that. Zoomed out over everything, 190,000 piece
  matrices (9 MB) go to the graphics card each frame; a distant level of detail with one
  matrix per unit would save most of the 2 ms. A copy of a 20,000-unit battle for the
  highlights takes about 20 ms every 20 seconds.
- **AI**: between equal AIs ARM wins about 64% of sea games and CORE 58% of land games.
  Sea games are economy races: with the Commander rule about a quarter end by minute 40.
  Hard's air doctrine for island wars (a scout finds the enemy Commander, bombers massed for
  it strike it) ends more of them: against Hard without it, 45 of 80 sea games won, 24
  outright (6 the other way), +19.5% margin. Normal and Easy play without it; whether
  Normal should have a gentler version is open.
- **Classic interface**: the modern HUD's mixed-selection filter, factory queue strip and
  missile count have no place in it yet.
- **Multiplayer over the internet**: the host must forward a port (no relay or NAT
  traversal); games on a LAN are listed automatically, and dropped players can rejoin.

## Ahead

The port is done: TA plays, looks and sounds better than it ever has, on the player's own
files. What comes next is bigger. The aim is for TA Reborn to become the definitive way to
play Total Annihilation, and then the most ambitious open real-time strategy engine there is.
Four things make that credible rather than wishful:
- a simulation that is fast and deterministic: a 12-minute AI game simulates in about a
  second, and any battle can be copied, rewound, replayed or reproduced bit for bit;
- a renderer that already traces rays, and 20,000 units at playable speed;
- a mod layer that plays the original's mods without code changes;
- an interface toolkit that makes new screens cheap.

The milestones below are in rough order; each ends with a bar that says when it is done.
Version numbers are aspirations, not promises.

### What stays sacred

- **Classic stays classic.** TA's rules, TA's camera and TA's 1997 interface stay one click
  away. New rules arrive as options or modes, and new looks are presentation only.
- **Lockstep determinism.** Multiplayer, replays, highlights, the coach and the AI league all
  stand on it. Nothing that feeds the simulation may read a clock, a float or a hash map's
  order.
- **The player's own install.** Nothing of Cavedog's ships. Everything is read from the
  install, derived from it or painted on the player's machine.
- **A budget.** 20,000 units in one battle at full speed, at 60 frames a second on a mid-range
  graphics card. What cannot meet it becomes a setting.

### 2.1: Boneyards reborn (online)

TA had Boneyards, Cavedog's online service. TA Reborn should have better.

- **Play anyone from anywhere, with no port forwarding.** Connections go over QUIC with hole
  punching, falling back to a relay: a small open-source server that anyone can host, and
  one we run for everyone.
- **Matchmaking and rankings.** A game browser, quick match (1v1, 2v2, free-for-all),
  Glicko-2 ratings, seasons, profiles and friends, and invites that open straight into a
  lobby.
- **Spectating and casting.** Live spectating with a delay against ghosting, and a casting
  kit:
  - production tabs and army value and income graphs for every player;
  - actions per minute;
  - fog of war switched per player;
  - picture-in-picture of a second fight.
- **Replay codes.** Six letters share a battle, which then opens in the game.

**Bar:** two players behind ordinary home routers are in a game within a minute of pressing
QUICK MATCH, and it stays in sync.

### 2.2: Everywhere (platforms and access)

- **Linux and the Steam Deck, to Verified standard.** A controller-first HUD with a radial
  build wheel, and suspend and resume.
- **Other platforms:** macOS on Metal, and Windows on ARM.
- **Accessibility:**
  - screen-reader menus (through AccessKit, which egui supports);
  - subtitles for unit voices and battle events;
  - a high-contrast HUD;
  - one-handed and hold-free controls;
  - a tactical pause for giving orders in single player.
- **Native speakers' review of the translations**, through a community translation page
  (the files and the tools exist today), and right-to-left and shaped scripts (Arabic,
  Hindi), which egui cannot lay out yet.

**Bar:** a whole campaign played on a Steam Deck without touching its screen, and every menu
read aloud by a screen reader.

### 3.0: The General's Table (seeing the war)

- **One continuous zoom from a single Peewee to the whole planet.** Far out, the battlefield
  becomes a living tactical map: units as icons, the fronts, and heat maps of territory and
  income. The minimap becomes optional.
- **Battle scars that last the match.** Craters and scorch pile up where the fighting was.
  As a rule option, the ground itself deforms: nukes leave craters that change paths and
  lines of fire (deterministic height edits, with the routes and flow fields rebuilt where
  they changed).
- **Light, the next step:**
  - one bounce of ray-traced global illumination: fires light the valleys, and the ground's
    colour shows on hulls;
  - HDR displays;
  - a temporal upscaler, so 4K runs at speed;
  - a photo mode that path-traces its picture over a few seconds.
- **Sound:**
  - music in layers that rise and fall with the battle;
  - positional sound that echoes off cliffs and dies in forests.
- **The table itself, in VR (OpenXR).** The battle as a holographic table-top in a war room:
  lean in to a skirmish, and point to give orders.

**Bar:** a 4K battle of 5,000 units at 60 frames a second on an RTX 3060-class card, and a
photo from it you would hang on a wall.

### 3.1: Every player a Cavedog (making)

- **A scenario and campaign editor inside the map editor:**
  - triggers, objectives and waves;
  - briefings and dialogue;
  - cutscenes filmed with the highlights camera;
  - shared as mods.
- **Procedural worlds.** Seeded planets of every world type (lush, desert, lava, metal,
  ice, archipelago), laid out from the stock maps' own tiles, with fair starts and metal. A
  seed typed into the lobby is a new map for everyone.
- **A unit workshop:**
  - the FBI and weapon files edited with a live preview in the gallery;
  - balance reports from AI games run in the background;
  - a COB script debugger (step through a script and watch its pieces move);
  - glTF models imported as 3DO with GAF textures.
- **Publishing and certification.**
  - Mods publish to ModDB and mod.io from the launcher, with ratings.
  - The AI soak test certifies each mod in the launcher's list ("a 12-minute AI game plays
    through").
  - The great TA mods (ProTA, Escalation, Uberhack and their kin) run as their authors meant,
    each with its own soak test in continuous integration.

**Bar:** someone new to modding makes a scenario with a twist and shares it in one evening,
without leaving the game.

### 3.2: A coach in the machine (learning)

- **A coach after every battle, from the replay.** It shows:
  - where you stalled, and the factories and builders you left idle;
  - resources left unspent;
  - fights that cost more than they killed;
  - the minutes you went unscouted.

  Each point jumps into the replay at that moment.
- **An AI league.** Computer players are trained by self-play on the simulation: a thousand
  games an hour on one desktop, far more on a cloud of them. They come as personalities
  (the Turtle, the Raider, the Air Marshal) and a Grandmaster level. The league doubles as
  the balance lab for mods and rules options.
- **Challenges:** a daily challenge on a shared seed, puzzle missions ("win with 2,000
  metal"), medals, and drills for build orders and micro.
- **Battle reports.** A written account of every battle (the turning points, the numbers,
  the costliest fight), shared with its replay code.

**Bar:** the Grandmaster AI takes a best-of-five from a top-ten ladder player, and players
say the coach made them better.

### 3.5: Galactic War (a living universe)

- **A persistent war between ARM and CORE across a shared galaxy.**
  - Seasons in which every ranked game moves the front.
  - Worlds with rules of their own (ash storms that ground aircraft on lava worlds, tides
    on water worlds, no wind on metal worlds).
  - A season finale fought over Core Prime and Empyrrean.
- **Together:**
  - co-op campaigns and co-op Conquest;
  - clans that hold systems;
  - tournaments run inside the game, with brackets, casters' seats and honours that last.

**Bar:** a season in which a thousand players decide which capital falls.

### Moonshots

Worth trying, even if not all of them land.

- **TA in a browser** (WebAssembly and WebGPU), reading the player's own GOG files through
  the browser's file access: a link opens a battle.
- **Movie export:** a battle's highlights as a 4K video with its music and sound, encoded by
  permissively licensed encoders (rav1e, OpenH264).
- **50,000 units at 30 ticks a second:** weapons, orders and scripts in parallel phases, and
  routes rebuilt incrementally.
- **Voice orders**, recognised offline ("Fight. Over there."), for accessibility first.
- **A companion screen:** economy, minimap and build queue on a phone or tablet on the same
  network; later a tablet edition with touch controls.
- **TA's art rebuilt at load time on the player's machine:** textures, unit pictures and map
  art upscaled by neural models, as Scale2x does today; never shipped, never uploaded.
- **A title screen you can drop into:** the attract-mode battle behind the menu as a game
  in progress, yours to take over with one click.

## Checks before a release

1. `cargo test -p ta_formats -p ta_vfs -p ta_sim -p ta_net -p ta_render`,
   `cargo test -p ta_game` and the AI soak test on a land map (`Sherwood`), a sea map
   (`Coast To Coast`, with `TA_TEST_MINUTES=20`: the armies there first meet after about
   17 minutes) and a metal world (`Metal Heck`).
2. `cargo run -p ta_game -- --map "Coast To Coast" --demo battle --screenshot scratch/x.png
   --frames 300` and look at the picture; the same with `--classic`.
3. A two-process multiplayer game (`--host` and `--join`) reaching identical checksums.
4. Commit, then `installer/build_installer.ps1` (no warning about uncommitted changes; with
   `-SignScript` once there is a code-signing certificate); install and play from the
   Start menu shortcut, and open a replay by double-clicking it. For the first release
   under the name TA Reborn, install it over TA Remastered 1.9.0: it should replace that
   version (Add/Remove Programs, Start menu, associations), and the first start should move
   `%APPDATA%\TA Remastered` to `%APPDATA%\TA Reborn`. Publish the setup, the portable
   archive and the SHA256SUMS file; keep the symbols archive.
5. Set `update::DEFAULT_SOURCE` in `apps/ta_launcher/src/update.rs` to where releases are
   announced (a TOML file with `version` and `url`, or a GitHub latest-release API address),
   and publish that file with each release.
