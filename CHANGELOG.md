# Changelog

## Unreleased

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
  faster with either compiler. On the computer it was measured on, the first battle opens in about
  20 seconds on DirectX 12 (a minute and a half without DXC). The picture is the same as before,
  and Vulkan draws it in the same time.
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
