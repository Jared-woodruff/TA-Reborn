Total Annihilation Reborn (TA Reborn)
=====================================

A clean-room reimplementation of the Total Annihilation (1997) engine, written in Rust.
It was called TA Remastered up to version 1.9.

TA Reborn contains NO original game data. It plays the data files of your own,
legitimately owned copy of Total Annihilation - for example "Total Annihilation:
Commander Pack" from GOG.com - read in place from its install folder. Nothing in that
folder is copied or modified.


Getting started
---------------

1. Start "TA Reborn" from your applications menu, or open TA Reborn.app on macOS.
   Portable Windows/Linux packages can also run ta-launcher directly.
2. The launcher finds GOG and Steam installs of Total Annihilation automatically.
   If yours is somewhere else, use "Browse for existing install..." on the Play page.
   No copy yet? "Get it on GOG" opens the store page.
3. Adjust settings and mods if you like, then press PLAY.


Playing
-------

- F1 in battle shows every control. Keys, and which mouse button gives orders (the
  right one, or the left as in the original), are set in Options, Controls (in the game
  or the launcher's Settings); camera bookmarks (F6-F9) and control groups (0-9) stay
  fixed.
- TA Reborn keeps TA's rules and adds conveniences: drag a ring in attack, capture,
  resurrect or reclaim mode to order a whole area; Shift+orders to a factory lay out the
  route its units take; extractors snap onto the richest metal; Tab lists the players;
  Ctrl+S saves (single player) and Continue in the main menu picks up the newest save.
- The settings (graphics, display, sound, controls...) are the same in the game's
  Options and the launcher's Settings, with a search box to find one by name.
- Mods: the launcher's Mods page, GET MODS, lists the Total Annihilation mods on ModDB
  (moddb.com) with how each plays in TA Reborn; GET downloads and installs one, PLAY
  starts the game with it. Mods you already have go in the mods folder (Mods, INSTALLED).
- Multiplayer: one player hosts (default port 7890, allow it through the firewall), the
  others pick the game from the list of games on their network, or join with the host's
  address. Anyone can pause or change the game speed; the
  game waits for a player who falls behind. A player who drops out can join again with
  the same name and pick up where they were; after a minute away an ally takes over their
  army. Players can also join to watch. In team
  games the Share menu (top bar) sends allies metal, energy or the selected units.
- Replays of every game are kept; the replay bar can jump backwards and forwards and
  follow the action. Double-click a replay (.tarep) or a saved game (.tasav) to open it.
- Gamepads work too (the Steam Deck's controls included): the left stick scrolls, the
  right stick moves the cursor, A and B click, the triggers zoom. F1 lists every button.
- Also on the main menu: Conquest (take a galaxy of TA's worlds system by system), a
  Tutorial of three short lessons, and the Map Editor.


macOS
-----

Choose the Intel x64 or Apple Silicon ARM64 package for your Mac (macOS 13 or later).
Open the DMG and drag TA Reborn.app to Applications, or unpack the app ZIP. The app
contains both the launcher and game; shared resources are inside Contents/Resources.
The game uses Metal. Your original TA data can be selected in the launcher.
Settings and saves currently use ~/.config/ta-reborn, as on Linux.

Linux and the Steam Deck
------------------------

Choose x64 or ARM64 for your system. Ubuntu 22.04+ uses the .deb package (install with
sudo apt install ./<package>.deb); Arch Linux / Arch Linux ARM uses the .pkg.tar.zst
package (sudo pacman -U ./<package>.pkg.tar.zst). These install in /opt/ta-reborn
with applications-menu entries. Use the same package manager to update or uninstall.

Alternatively, unpack the Linux archive and run ./install.sh: it copies TA Reborn to
~/.local/share/ta-reborn and adds it to the applications menu (on the Steam Deck, add
it to Steam from there as a non-Steam game). ./install.sh --uninstall removes it.

The launcher looks for Total Annihilation in Steam (played through Proton), Heroic, Lutris,
Bottles and Wine prefixes; otherwise point it at the game's folder. Settings and saves
live in ~/.config/ta-reborn. The game needs Vulkan drivers.


What is installed
-----------------

  ta-launcher.exe     The launcher: install detection, mods and settings.
  ta-reborn.exe   The game. The launcher starts it with --game-dir <your TA folder>.
  assets\             Engine resources (no original game assets).
  lang\               Interface languages (translation files).
  mods\               Mods for every user of this PC (see mods\README.txt).
  docs\               Documentation; CHANGELOG.md lists what each version brought.
  LICENSE.txt, THIRD-PARTY-NOTICES.txt
                      The licences (see below).

The portable archive holds the same files: unpack it anywhere and start ta-launcher.exe.
Windows packages include the matching Microsoft Visual C++ runtime DLLs beside the
programs. Keep them with the executables; no separate runtime installation is needed.
MICROSOFT-RUNTIME-NOTICE.txt lists these Microsoft components, their versions and hashes.

Your own files are kept in %APPDATA%\TA Reborn. Uninstalling TA Reborn asks
whether to delete them as well; unless you answer Yes, they stay in place:

  config.toml         Settings, written by the launcher and the game's options.
  progress.toml       Campaign progress.
  saves\, replays\    Saved games and replays.
  mods\               Your personal mods.
  logs\game.log       Output of the last game session (useful for bug reports).
  logs\launcher.log   The same for the launcher.
  logs\crash.txt      What went wrong, if the game ever crashes (launcher-crash.txt for
                      the launcher). Please include it, with the version shown in the
                      corner of the main menu, when you report a problem.


Legal
-----

Total Annihilation is (c) its respective owners. TA Reborn is an unofficial,
non-commercial fan project and is not affiliated with or endorsed by them. This project
contains no original game assets and requires a legitimately owned copy of the game.

TA Reborn is free software under the GNU General Public License, version 3 or any later
version (LICENSE.txt), and comes with no warranty. It is built with open-source libraries
that keep their own licences; THIRD-PARTY-NOTICES.txt lists them with their licence texts.
