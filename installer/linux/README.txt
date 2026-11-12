TA Reborn for Linux
===================

TA Reborn is an unofficial reimplementation of Total Annihilation. It contains no
original game data. Select the data folder from your own copy of Total Annihilation
in the launcher; the original Windows executable is not needed to play in Reborn.

Choose the package matching your CPU: x64 is x86-64; arm64 is 64-bit ARM.

Ubuntu 22.04 or newer:
  sudo apt install ./TA-Reborn-<version>-linux-<architecture>-ubuntu.deb
Start TA Reborn from the applications menu, or run ta-launcher.
Remove the program with: sudo apt remove ta-reborn

Arch Linux (x64) or Arch Linux ARM (arm64):
  sudo pacman -U ./TA-Reborn-<version>-linux-<architecture>-arch.pkg.tar.zst
Start TA Reborn from the applications menu, or run ta-launcher.
Remove the program with: sudo pacman -R ta-reborn
Arch Linux ARM is a separate distribution; these packages do not imply official
Arch Linux support for ARM hardware.

Portable archive:
Extract the .tar.gz and run ./ta-launcher. Optionally run ./install.sh to copy the
program into your user data directory and add an applications-menu entry.
Use ./install.sh --uninstall to remove that per-user installation. Settings, saves,
profiles and imported mods remain in your user configuration directory.

Steam Deck:
Use the portable archive (x64); the .deb and Arch packages install into SteamOS's
read-only system. In Desktop Mode, extract the .tar.gz and run ./install.sh. Then in
Steam choose Add a Non-Steam Game and pick TA Reborn (the launcher, installed as
~/.local/share/ta-reborn/ta-launcher), so that it also starts from Game Mode.
TA Reborn finds Total Annihilation where Steam, Heroic or Proton put it: the Steam
release in any Steam library, the SD card's included; GOG's copy installed with
Heroic, wherever Heroic's own record says; or GOG's installer (or the CD) run through
Steam as a non-Steam game, in that game's Proton prefix. Otherwise choose the folder in
the launcher.
The first start recognises the Deck (Steam's SteamDeck=1, the Deck's firmware, SteamOS
or its graphics) and sets itself up for it: full screen at 1280 x 800 with vertical
sync, the interface at 125%, the launcher maximised, and the Medium graphics level once
the game first opens the Deck's graphics. All of it can be changed in Settings. In
Game Mode the launcher fills the screen.
The Deck's controls work the launcher: the d-pad or left stick moves between controls,
A uses them, B goes back, the bumpers turn the pages and View lists every button.
install.sh also adds "TA Reborn (game only)" to the applications menu: added to Steam in
place of the launcher, it starts the game straight away with the launcher's settings.

The portable build needs glibc 2.35 or newer, ALSA, udev, X11 or Wayland libraries,
OpenGL/EGL and a working Vulkan driver. File pickers use xdg-desktop-portal and a
desktop-specific portal backend. Install these through your distribution.
For X11, include the XKB/XCB integration libraries (Ubuntu: libxkbcommon-x11-0 and
libx11-xcb1; Arch: libxkbcommon-x11 and libx11), not only the base XKB library.
Mod imports also need bsdtar (Ubuntu: libarchive-tools; Arch: libarchive) or 7-Zip.
Install curl for the launcher's news, update checks and mod downloads, and xdg-utils
for opening project websites and folders in your desktop's default applications.

User files normally live in ~/.config/ta-reborn (or $XDG_CONFIG_HOME/ta-reborn).
System packages live in /opt/ta-reborn and should not be edited. Use the launcher's
Mods and Maps pages to import user content instead.

docs/ contains the project documentation; LICENSE.txt and THIRD-PARTY-NOTICES.txt
contain licence information. The separate symbols archive is for crash diagnosis.
Release binaries are built natively on Ubuntu 22.04 for the glibc baseline. The Arch
package uses the same verified binaries; it is not a makepkg source build. Its
.BUILDINFO records the binary packaging recipe, and build-info.json records the
actual builder OS, Rust toolchain and Debian package versions.

Project: https://github.com/Jared-woodruff/TA-Reborn
