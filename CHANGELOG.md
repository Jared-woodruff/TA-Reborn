# Changelog

## Unreleased

### Added
- The launcher now works with a gamepad, the Steam Deck's controls included: the d-pad or left
  stick moves between controls, A uses them, B goes back, the bumpers turn the pages, the right
  stick moves the pointer and the triggers scroll. X opens a card's menu, Y the search, and View
  the list of keys and buttons, which now shows the controller's too.
- On Linux, `install.sh` also adds *TA Reborn (game only)* to the applications menu: added to
  Steam in place of the launcher, it starts the game straight away with the launcher's settings.
- The Steam Deck is recognised on the first start (by Steam's `SteamDeck=1`, the Deck's
  firmware, SteamOS or its graphics), and the game is set up for it: full screen at 1280 × 800
  with vertical sync, the interface at 125%, as large as the battle's panels allow on its
  screen, the launcher maximised, and the Medium graphics level, which suits the Deck's
  graphics. Whatever you change afterwards is kept.
- In SteamOS's Game Mode (and anywhere else inside gamescope) the launcher now fills the screen,
  drawn pixel for pixel instead of a window scaled up. On the desktop it stays a window, as you
  left it.
- On Linux the launcher now finds Total Annihilation where Heroic installed it, wherever that is
  (another folder, an SD card), as Heroic's own record says; and where GOG's installer or the CD,
  run through Steam as a non-Steam game, put it in that game's Proton prefix, in any Steam
  library. In Wine prefixes it now also looks in `Program Files`, and finds the CD's folders
  whatever the case of their names.

### Changed
- On Linux, the Steam Deck and macOS, the interface's text is now set in Sofia Sans, a typeface
  close to Windows' Bahnschrift that comes with TA Reborn, instead of a thin fallback font. It
  has Cyrillic and Greek, so Russian and Ukrainian menus read in one typeface, in the letter
  forms those languages use.
- In the game's menus and dialogs, a control the d-pad moves to is scrolled into view, and the
  d-pad now leaves a text field (such as the options' search) instead of moving its cursor.
- On Linux the first start no longer sets every computer to the Medium graphics level. The
  game now picks the level from the graphics card it opens: High on a dedicated card, Medium on
  recent integrated graphics and the Steam Deck, Low on older integrated graphics.
- Choosing AUTO for the graphics level in the options now picks the level again the next time
  the game starts, and leaves the window as it is. Before, the window went back to full screen
  at the screen's size too.

### Fixed
- With the classic interface, TA: Escalation, Total Mayhem and the other mods made for the
  patched engines of later years showed only four of a builder's buildings, the same two
  repeated down the panel, with no page arrows and no orders: most of the tech tree could not
  be reached. Their menus were made for a taller screen (768 pixels), with twelve buildings to
  a page and the orders under them, and were squeezed onto TA's 480. The side panel is now laid
  out on the screen a mod's menus were made for: every page with all twelve places, the arrows
  to turn them and the orders, as in the mods' own engines. TA's own menus are unchanged.
- TA: Escalation, Total Mayhem and the other mods made for patched engines did not load when
  dropped into a mods folder by hand (an archive alone, or a folder without the `mod.toml` the
  launcher writes): their units, menus and weapons are in folders of their own (`unitse`,
  `guim`, `weaponm`...), which were not read. TA: Escalation played as the original game, Total
  Mayhem without its menus or weapons. Such a mod's own folders are now found from what it holds,
  as the launcher's installer finds them.
- Mods installed into Total Annihilation's own folder, as their own installers had you do,
  were loaded as part of the original game: TA: Escalation's models and scripts took the place
  of TA's own units' while Escalation itself did not load, and Total Mayhem's units came
  without their menus or weapons. They are now left out of the original game, and the
  launcher's Mods page lists each one under IN YOUR TOTAL ANNIHILATION FOLDER: ADD AS A MOD
  adds it to your mods, its files linked from there (copied when the mods folder is on
  another drive) and the game's folder left as it is.
- On the Skins page, at some window widths and interface sizes, "Use the same skin for both
  sides" was squeezed into a column a letter wide, or the search and the skins below it went
  out of reach. It now goes on a line of its own when it does not fit beside the skins.
- Damaged aircraft never used repair pads unless sent to one. Aircraft on patrol, attack-move
  or an attack now break off when worn below two-thirds of their health, fly to the nearest
  free Air Repair Pad or carrier, however far, land on it and are repaired, then carry on with
  what they were doing. A pad takes one aircraft at a time: the others keep on with their
  mission until it is free, and an idle aircraft parked on a pad makes room for a damaged one.
  A new order calls the trip off. Idle aircraft, and aircraft only moved somewhere, still don't
  go looking for a pad.
- Repair pads also repaired aircraft parked on the ground beside them, and went on working
  switched off. They now repair only the aircraft landed on them, and a pad switched off takes
  no aircraft and repairs none.
- Thunder and Hurricane bombers sent to a repair pad flew on past it, landed on the ground
  beyond and were never repaired. They now come back and land on it.
- The Skins page's turning 3D preview stopped on its second frame; it turns again.
- The skin preview now draws on the same graphics card as the game. On a computer with
  integrated graphics beside its graphics card it drew on the integrated graphics, whose driver
  could crash the launcher.
- When the skin preview cannot draw, the Skins page no longer opens a new graphics device for
  every picture; Retry preview tries again.
- If the launcher crashes in a graphics driver, it now saves `launcher-crash.txt` naming the
  driver and says so, as the game does.
- Opening a skin whose scale has more than two decimals no longer marks it as changed.

## 1.0.3 — 2026-11-12

Battles now end in dark clouds at the map's edge, and the camera zooms out no further than the
whole map. Units fire at lesser targets when nothing better is in range, bombers bomb on patrol,
ships are built at the surface, construction aircraft sway as in TA, and aircraft circle over
the sea instead of stacking up.

### Changed
- In battles, the world beyond the map's edges is now a sea of dark clouds below the map, so
  the battlefield stands out, instead of land and sea carried on to the horizon. Zoomed out,
  that land filled most of the screen and shimmered. The map editor shows the clouds too; the
  menu's film, the credits and staged scenes keep the landscape.
- The battle camera now zooms out only until the whole map is in view, and zoomed out that far
  it keeps the map in view rather than the dark beyond it. On the largest maps it zooms out as
  far as before, and now the same on every screen: a 4K screen used to see twice as far.
- Fire at will: units now fire at their "bad targets" when nothing better is in range, rather
  than never. Missile towers fire on tanks when no aircraft are about, a destroyer's depth
  charges take on an amphibious tank under the water, and a scout ship's missiles a boat. A
  unit firing at a bad target turns to a better one as soon as one comes into range, and units
  still chase only what they are meant for. An attack order works as before.
- `+shootall` in the chat now does what it did in TA: every weapon of your units fires at every
  enemy in range alike, nearest first. Type it again to turn it off.
- Construction left with nobody working on it now un-builds after ten seconds, giving back the
  metal put into it (not the energy), and disappears when nothing is left. A frame placed and
  left before any work went into it goes at once. A factory's unit inside its yard, and
  unfinished buildings a mission or scene places, are left alone.
- Taking one of your own units apart now takes as long as its metal says, as reclaiming a wreck
  does. A Metal Maker, which costs no metal, is reclaimed at once.
- Planes with nothing to do over the sea, where they cannot land, now circle the spot instead
  of hovering in place. New planes sent to a rally point over the water circle there.
- A group of planes ordered onto one target now spread across a front as they come in, instead
  of following each other in single file.
- Putting a building down again where your selected constructor already plans one takes the
  plan back, and putting a different building down over a plan puts it in the plan's place: a
  wind generator instead of a solar collector, or a metal extractor placed in the fog of war
  taken back. Plans of other constructors still keep their ground.
- Shift with the right button while placing a building now ends the placing and queues the
  order there as well, so a Commander told to build a plant can be sent on after it.
- Bombers sent at something show TA's air-strike sight instead of the attack sight.
- Bombers on patrol or attack-move with fire at will now bomb what they come upon near their
  way, then fly on. Before, their bombs dropped only on an attack order, so patrolling bombers
  flew over the enemy and landed at the end of their route. A bomber on its own sees a building
  too late for its first pass and comes round for it; with radar or an ally's sight it lines up
  on targets further ahead. TA's bombers start on hold fire, so set them to fire at will. A
  plain move still bombs nothing.
- The music, and the battle behind the menu, now fall silent while a campaign briefing is read
  out, and come back once it is over. The tutorial's spoken lines still only lower the music.
- Construction aircraft at work now face what they build and sway smoothly round it to one side
  and back, as in TA, instead of hanging still over it. They turn to face it first, never fly
  backwards, stay within reach, and the building goes up as fast as before.
- Constructors on the ECONOMY duty now keep to a few plain jobs: metal extractors on free metal,
  power plants when power runs short (solar collectors, or wind generators where the wind pays
  better), and metal makers when there is power to spare, finishing buildings of those kinds
  left unfinished near them. They no longer build storage or geothermal plants, replace
  extractors with better ones, reclaim, or help at the factories.

### Fixed
- Scouts, construction aircraft and transports with nothing to do over the sea, such as new
  aircraft sent to a rally point on the water, hung still there on top of each other. They now
  circle it as fighters and bombers do, each round its own circle, so they keep apart; slow
  construction aircraft loiter slowly. Gunships still hover.
- Ships were built under the water in their shipyards (most of all the Core's) and only rose to
  the surface once finished. They are now built at the surface.
- A shipyard's ghost, while placing it or queued, showed on the sea floor; it now shows where
  the shipyard will stand.
- Aircraft plants' runways (and other buildings' flat bases, such as solar collectors') could
  flicker, the ground showing through them in lines, most of all zoomed out: the ground under a
  building was drawn up to a fraction of a pixel too high, level with the runway. It is now
  drawn never above where it belongs, and the runway sits a little higher over it.
- A constructor building a block of buildings that touch (Alt+drag while placing) could be
  walled in by them, unable to move again. Units now step off a new building's site clear of
  the builder's next sites, a unit left standing in a building is put on the nearest free
  ground, and a filled rectangle is built from its far side towards the builders.
- On a Mac in full screen, the system's pointer could show on top of the game's own. The game
  now has macOS apply the pointer it wants again whenever it takes the screen and once a second
  while it has the keyboard.

## 1.0.2 — 2026-11-11

Faster on Macs and other integrated graphics, which now draw the battlefield at about 1080p's
pixels. Also a fix for a crash on AMD's newest graphics cards, and a Mac's full screen without
the Dock and the menu bar coming over the game.

### Changed
- Faster on integrated graphics and high-resolution screens. On integrated graphics (Intel,
  AMD and Apple chips) the graphics levels now draw the battlefield at no more than about
  1080p's pixels, and 1440p's at Ultra; Apple's Pro, Max and Ultra chips draw more. The
  interface keeps the screen's full sharpness. A Mac's Retina screen has 2.7 to 3.7 times
  1080p's pixels, and before this an M5 MacBook Air ran well only on Low. The level is fitted
  when it is picked in the options, and once as the game starts: on its first start, or its
  first after this update, where the render scale is still the level's own (a scale set by
  hand is kept). A render scale below the level's own no longer makes the level Custom.
- A render scale below 100% now scales the battlefield up through a sharper filter (a cubic,
  kept from ringing round bright lights), so it loses less detail than it did.
- Units, trees and the painted features are now drawn before the ground, so the ground hidden
  behind them is no longer shaded. On a forest map this saves about a fifth of the frame.
  The picture is the same.
- The ground reads the map's tile table once per tile rather than once per texel (most of all
  for its HD surfaces), and looks up the art's softened colours once where two effects used
  them. The picture is the same.
- On Apple's own chips, the multisampled picture and its depth are stored only for a pass that
  reads them, as Apple advises for its tiled graphics.
- On a Mac, the launcher and the first start now know the computer's chip (an "Apple M5"), its
  memory and processor, and pick its graphics level from them: Medium on the base and Pro
  chips, High on the Max and Ultra.

### Fixed
- On a Mac in full screen, the menu bar and the Dock no longer come over the game when the
  pointer touches the screen's edges. Scrolling the map by the edges brought them up, and with
  the pointer over them the scrolling stopped. They now stay hidden while the game is in full
  screen; Cmd+Tab still switches apps, and they come back when the game leaves full screen.
- On AMD's newest graphics cards (a Radeon RX 9070 XT), the game could crash on Vulkan while
  preparing its graphics, the crash report naming the sprites' pipelines. One value in its Vulkan
  shaders, the sample mask, was written in a form Vulkan's rules do not allow, and AMD's driver
  expects the allowed form; it is now written as the rules require. The picture is the same as
  before.

## 1.0.1 — 2026-11-11

DirectX 12 works: the game no longer stays on its first loading screen there. Also a watch on
the game's memory, and crash reports for graphics drivers.

### Added
- A watch on the game's memory. Every minute of a battle the log records how much memory the
  game holds. Should it climb far past what the battle needs, the game lets go of its highlight
  copies, saves a single-player battle (the autosave) and says so. Near the computer's limit it
  also pauses the battle, rather than let the computer run out and take the battle with it.
- Crash reports give the game's memory at the time, and a game started without the launcher now
  keeps the last words of a crash for want of memory in its log.
- A crash outside the game's own code (a graphics driver failing as it compiles a shader) now
  writes a crash report too, naming the driver and what the game was preparing, and the
  launcher shows it. The log says how long the graphics card took over each of the game's
  shaders, and which it is still on when one takes long.

### Fixed
- On DirectX 12 the game no longer stays on its first loading screen. Windows' own shader
  compiler took many minutes over the ground's and the water's shaders. The game now ships
  Microsoft's DirectX Shader Compiler (DXC), and the shaders are written to compile several times
  faster with either compiler. On the computer it was measured on, the first battle opens on
  DirectX 12 in about half a minute the first time and in about 20 seconds after that (a minute
  and a half without DXC). The picture is the same as before, and Vulkan draws it in the same
  time.
- In a window smaller than 1024×600 points, a battle's first frame after its loading screen was
  laid out for a window of 10,000 points: the classic interface drew its panel and its letters
  dozens of times too large for one frame, which a graphics card could refuse.
- A row of choices (photo mode's tabs and looks, the options' selectors) no longer slides its
  marker in from the side of the screen when the panel it is in moves or first appears.

## 1.0.0 — 2026-11-09

The first public release of Total Annihilation Reborn: an unofficial, clean-room
reimplementation of the Total Annihilation engine in Rust. It plays your own copy of the game
(the Commander Pack from GOG or Steam), reading every unit, map, campaign and sound in place
from your install; everything around them is new.

### The battlefield
- Every world with its own sky, light and colour grade; physically based rendering, ray-traced
  shadows and reflections where the graphics card traces rays.
- Water with surf, bow waves, wakes and caustics; lava that crusts over and glows through its
  cracks; rain that soaks the ground and dries after; dawn, dusk and moonlit nights.
- Fire, smoke, fireballs and nuclear strikes; 3D trees, bushes and rocks; the land beyond the
  map's edge.
- Twenty thousand units in one battle.
- Two art styles: Reborn, which rebuilds TA's designs with armour plates, seams and bolts, and
  Enhanced Classic. Nineteen skins, and an editor for your own.

### Construction
- Nano-lathe beams and build lattices; buildings rising out of the green and sagging into
  rubble; factories that open and roll their units out; reclaim, wrecks and heaps.

### Perspective
- Ride along in any unit, with eight modelled cockpits, live instruments and rain on the glass.

### Command and control
- The command deck, or TA's own 1997 panel; a radial menu on any unit, and builders on duty.
- Formations that march as armies.
- Strategic zoom, and a companion screen for a second display.
- Photo mode.
- All of TA's keys and commands, and gamepads.
- TA's rules of engagement: units leave alone what their files call bad targets unless
  ordered, the long-range guns come out of the factory on hold fire, and strategic missiles
  strike the place their target stood.

### Ways to play
- Skirmish for ten players, with mutators.
- Every campaign of your install.
- Conquest, a galaxy to take battle by battle.
- Thirteen scenarios of TA Reborn's own, and a tutorial in five lessons with a coach, spoken
  in thirteen languages.
- Lockstep multiplayer, with allies, spectators and a lobby. Players with different map packs
  play together, and a join that can't go ahead says which mods differ.
- Replays with a cast view, highlights, MP4 export and social clips; saved games; a debrief
  after every battle.

### Computer players
- Three difficulties that build economies, defences and armies, raid, send waves, air strikes
  and nukes, fight at sea and ferry armies across the water.
- They play fair: they see what their team sees and scout for your base.
- Personas with names and manners talk in transmissions, and remember you.

### The launcher
- Finds GOG and Steam copies of the game.
- Mods from ModDB in a click, checked by a mod validator. Total conversions that replace TA's
  own archives, as TA: Enhanced does, replace them here too.
- A map library, and a skins library with its editor.
- Commander profiles with XP, 21 ranks and 134 achievements.
- A benchmark, music, and updates.

### TA Reborn's worlds
- Twenty-nine maps of its own: forests, deserts, islands, lava, metal, space, winter villages,
  and Australia, the USA and the whole world.
- A map editor that saves maps as mods.

### The Archives
- Every unit on show in a world of your choosing, with wallpapers up to 8K.
- The intro played on a 1997 CRT in a modelled bedroom.
- The Galactic War: the war's chronicle over a living galaxy, an archive of documents from both
  sides, and every campaign's missions.
- A title screen that films TA Reborn's worlds.
- A jukebox, and credits timed to a song of their own.
- Fourteen languages.

### Platforms
- Windows 10 and 11 (x64; ARM64 on Windows 11).
- macOS 13 or later (Intel and Apple Silicon).
- Ubuntu 22.04 or later and Arch Linux (x64 and ARM64).
