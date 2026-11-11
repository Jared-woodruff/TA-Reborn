# Changelog

## Unreleased

### Changed
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
- The music, and the battle behind the menu, now fall silent while a campaign briefing is read
  out, and come back once it is over. The tutorial's spoken lines still only lower the music.

### Fixed
- On a Mac in full screen, the system's pointer could show on top of the game's own. The game
  now has macOS apply the pointer it wants again whenever it takes the screen and once a second
  while it has the keyboard.
- Constructors on the ECONOMY duty now keep to a few plain jobs: metal extractors on free metal,
  power plants when power runs short (solar collectors, or wind generators where the wind pays
  better), and metal makers when there is power to spare, finishing buildings of those kinds
  left unfinished near them. They no longer build storage or geothermal plants, replace
  extractors with better ones, reclaim, or help at the factories.

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
