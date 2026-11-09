# TA Reborn — installers and release packages

This folder builds x64 and ARM64 packages for Windows, macOS, Ubuntu and Arch Linux
(Arch Linux ARM for ARM64). See [the release guide](../docs/RELEASING.md) for the complete
matrix, CI workflow, signing requirements and local commands. Windows setup uses
[NSIS 3](https://nsis.sourceforge.io) (Unicode, Modern UI 2).

| File | Purpose |
| --- | --- |
| `build_installer.ps1` | Builds the release binaries, stages the files, runs the NSIS compiler (`makensis`). |
| `ta_reborn.nsi` | The NSIS script (files, shortcuts, uninstaller, original-game detection and page, settings). |
| `artwork.nsh` | Filtered scaling of the installer and uninstaller artwork to the displayed control size. |
| `files/README.txt` | Read-me installed into the program folder. |
| `files/mods/README.txt` | Read-me installed into the (otherwise empty) `mods` folder. |
| `licenses/` | Standard licence texts, for the third-party libraries that ship none of their own. |
| `build_linux.sh`, `linux/install.sh` | The Linux archive and its per-user install script. |
| `package_unix.py`, `build_macos.sh` | Native Unix packages: portable Linux, DEB, pacman, macOS app ZIP/DMG, and symbols. |
| `windows_package.ps1` | Windows payload architecture checks and safe staging helpers. |

TA Reborn ships **no original game data**. Neither the build nor the installer ever copies
anything from a Total Annihilation install; the build script refuses to package TA archives
(`.hpi`, `.ufo`, `.ccx`, `.gp3`, `.gpf`) should one ever end up in the staging folder, and the NSIS
script excludes those extensions as a second guard.

## Building

Prerequisites:

* the Rust toolchain (<https://rustup.rs>) and licensed Visual Studio C++ build tools,
  including release CRT redistributables for the target architecture;
* NSIS 3 (release workflow uses 3.13). The script does not install it; get it with
  `winget install --id NSIS.NSIS -e` or from <https://nsis.sourceforge.io/Download>.

From the repository root:

```powershell
powershell -ExecutionPolicy Bypass -File installer\build_installer.ps1
```

The script

1. runs `cargo build --release -p ta_game -p ta_launcher -p ta_tools` (warning when the working
   tree has uncommitted changes: the programs show the commit they were built from);
2. stages everything to install in `dist\stage-windows-<arch>\TA Reborn\`:
   `ta-reborn.exe`, `ta-launcher.exe`, their required app-local Microsoft runtime DLLs,
   `MICROSOFT-RUNTIME-NOTICE.txt`, `README.txt`, `LICENSE.txt` (the repository's `LICENSE`),
   `THIRD-PARTY-NOTICES.txt` (written by `ta-tools notices` from Cargo's metadata: every library
   compiled into the two programs, with its licence texts), `assets\`, `lang\`, `mods\README.txt`
   and `docs\` (with `CHANGELOG.md`; not `docs\media`, the README's pictures);
3. packs the staged folder as `dist\TA-Reborn-<version>-windows-<arch>.zip`,
   and the programs with their `.pdb` symbols as `dist\TA-Reborn-<version>-windows-<arch>-symbols.zip`. Keep
   the symbols of every published version: installed programs have none, so a crash report
   (`%APPDATA%\TA Reborn\logs\crash.txt`) lists bare addresses and the address the game was
   loaded at; the difference is the offset to look up in that version's symbols;
4. finds `makensis.exe` (on `PATH`, in `Program Files (x86)\NSIS` or `Program Files\NSIS`, or via
   the folder recorded in `HKLM\SOFTWARE\WOW6432Node\NSIS`) and compiles `ta_reborn.nsi` with
   `/DAPP_VERSION=<version> /DAPP_ARCH=<x64|arm64> /DSTAGE_DIR=<stage folder> /DOUTPUT_FILE=<setup>`, writing
   `dist\TA-Reborn-<version>-windows-<arch>-setup.exe`, then the SHA-256 sums of setup, portable
   archive and symbols to `dist\TA-Reborn-<version>-windows-<arch>-SHA256SUMS.txt`. The version comes from
   `[workspace.package] version` in the root `Cargo.toml`.

If NSIS is not installed, the script stops after step 3, prints how to install it, and exits with
code 1. `dist\` is git-ignored.

### Artwork and display scaling

The welcome/finish panel and page header use the existing brass emblem on a gunmetal
background. `cargo run --release -p ta_tools -- brand branding` regenerates their 24-bit BMP
masters at **656 × 1256** and **600 × 228** (four times the original dimensions), alongside
the unchanged multi-resolution application icon. Commit the generated BMPs with the generator
in `apps/ta_tools/src/brand.rs`.

Setup uses Segoe UI and resamples these masters to the actual control dimensions with GDI
HALFTONE filtering, preserving the emblem's proportions even when font metrics change the
panel's shape. This avoids enlarging a small logo or using MUI's basic bitmap resizing.
The same treatment applies to the uninstaller header. MUI retains its default image if a GDI
operation fails; page images are released when leaving the page and the header when closing.

The wizard remains system-DPI aware. Check it at the intended display scale when it opens;
Windows can still scale the entire window when it is moved to a monitor with a different DPI.

Run `cargo test --locked -p ta_tools brand::tests` for the generated BMP contract, and
`powershell -ExecutionPolicy Bypass -File installer\tests\artwork.ps1` (or pass
`-Makensis <path-to-makensis.exe>`) for the native scaling regression. The latter checks
filtered pixels, aspect ratio, control sizes equivalent to 100–400% scaling, repeated bitmap
replacement, and the missing-file fallback. It opens no test page and installs nothing.

Options:

| Option | Effect |
| --- | --- |
| `-SkipBuild` | Reuse existing release binaries instead of running cargo. |
| `-StageOnly` | Stage the files but do not compile the installer. |
| `-TargetDir <dir>` | Cargo target directory (default: `$env:CARGO_TARGET_DIR`, else `target`). |
| `-Target <triple>` | Explicit Windows x64 or ARM64 target; reads `<TargetDir>/<triple>/release`. Omit for the native host's `release` folder. |
| `-HostTools <path>` | Runnable host `ta-tools.exe` for target-specific notices during cross-compilation. |
| `-VCRedistDir <dir>` | Override discovery with a licensed Visual Studio `VC/Redist/MSVC/<version>` directory. The script selects the target architecture's release CRT subfolder. |
| `-DistDir <dir>` | Output folder for the staged files and the installer (default: `dist`). |
| `-SignScript <file>` | Sign the programs, the uninstaller and the setup (see below). |

### Code signing

Unsigned programs make Windows SmartScreen warn people who download them. With a code-signing
certificate, pass `-SignScript` a `.cmd`, `.bat` or `.exe` that signs the file named by its first
argument; the build calls it for both programs, and NSIS for the uninstaller and the setup once
each is written (`!uninstfinalize`, `!finalize`). A script for a certificate in the Windows
certificate store, with `signtool` from the Windows SDK:

```bat
@echo off
signtool sign /sha1 <certificate thumbprint> /fd sha256 /tr http://timestamp.digicert.com /td sha256 %1
```

Without `-SignScript` Reborn's programs and setup remain unsigned; Microsoft's bundled
runtime DLLs retain their existing signatures.

### Microsoft runtime dependencies

Both Windows packages include the Microsoft release CRT DLLs imported by the game and
launcher, so a clean machine can start them without installing Visual Studio or a global
Visual C++ Redistributable first. Currently both architectures need `vcruntime140.dll`.
The packager reads ordinary and delay-loaded PE imports, follows runtime dependencies,
and copies the required files unchanged from the newest installed Visual Studio release
redistributables. It checks each DLL's architecture and rejects runtime versions older
than the executable's linker toolset. It never copies DLLs from Windows system folders
or `debug_nonredist`. The ARM64 folder can contain emulation-only DLLs; its directory name
alone is not proof that a DLL works with a native ARM64 program.

`MICROSOFT-RUNTIME-NOTICE.txt` records Microsoft ownership and the included files' versions
and hashes. These files are distributed under the build tools' Microsoft license terms,
separately from TA Reborn's license. Microsoft's [deployment guidance](https://learn.microsoft.com/en-us/cpp/windows/determining-which-dlls-to-redistribute)
and [Visual Studio redistributable list](https://learn.microsoft.com/en-us/visualstudio/releases/2026/redistribution)
describe these app-local files. Their servicing is part of Reborn's releases: update the
Visual Studio toolchain and rebuild packages when Microsoft publishes runtime fixes.
Windows 10/11 already supply the UCRT and Windows system DLLs, which are not bundled.

The signing hook signs Reborn's executables and setup, preserving Microsoft's signatures
on the runtime DLLs. Uninstall removes only runtime filenames bundled by that installer;
other DLLs in the installation folder are retained.

The script runs on Windows PowerShell 5.1 and PowerShell 7. To compile by hand after staging:
`makensis /DAPP_VERSION=0.1.0 installer\ta_reborn.nsi` (other defines: `STAGE_DIR`,
`OUTPUT_FILE`, `OUTPUT_DIR`, and `APP_URL` for the web site shown in Add/Remove Programs). The
Windows version resource gets the numeric part of the version (`0.2.0-beta.1` → `0.2.0.0`).

## What the installer does

* **Windows 10+ for x64, Windows 11 for ARM64**; setup checks the native OS architecture before installation. The x64 package also supports Windows 11 ARM64's x64 emulation.
* Installs **for all users** to `C:\Program Files\TA Reborn` by default, so it asks for
  administrator rights (UAC).
* Installs `ta-launcher.exe`, `ta-reborn.exe`, `assets\`, `docs\`, `README.txt`, and an empty
  `mods\` folder containing a README.
* Start menu folder **TA Reborn** with "TA Reborn" (the launcher), "TA Reborn (direct)"
  (the game without the launcher) and "Uninstall TA Reborn". A desktop shortcut for all users is
  optional (Components page, off by default).
* The last page offers to launch TA Reborn. It is started through Explorer, i.e. as the
  signed-in user rather than elevated (the launcher writes the user's settings); the "Get it on
  GOG.com" and "Get it on Steam" buttons open the browser the same way.
* Registers a normal uninstaller under
  `HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\TA Reborn` (64-bit view) with
  display name, version, publisher, icon, install location, size and quiet uninstall command.
* Replays (`.tarep`) and saved games (`.tasav`) open in the game when double-clicked: the
  ProgIDs `TAReborn.Replay` and `TAReborn.SavedGame` run `ta-reborn.exe "<file>"`.
  The uninstaller removes them (an extension's default only while it still names ours).

### Upgrades

Running a setup over an existing installation reuses its folder (the directory page is skipped)
and overwrites the program files; `assets\` and `docs\` are replaced so that no stale files remain.
The desktop shortcut keeps its previous state, and the user's settings are never touched. Setup
does not continue while the launcher or the game is running from the installation folder (it
offers Retry/Cancel; copies of the programs elsewhere, e.g. development builds, do not count).

The launcher updates an installation itself (`apps/ta_launcher/src/update.rs`): it downloads a
GitHub release's setup, checks it against the release's `SHA256SUMS`, and runs it with
`/UPDATE /D=<its folder>` (Windows asks for elevation). `/UPDATE` over an installed version skips
every page but the progress, gives the closing launcher up to ten seconds before asking about
it, closes by itself and starts the launcher again, as the user (`OpenAsUser`).

### Replacing TA Remastered (the name up to 1.9)

When `HKLM\…\Uninstall\TA Remastered` names an installation, Setup replaces it:

* The new version goes beside it under the new name: `C:\Program Files\TA Remastered` becomes
  `C:\Program Files\TA Reborn`. If that folder has a name of its own, the new version goes into
  it. The directory page is shown, so this can be changed. An installed TA Reborn or `/D=` wins.
* Before copying its files, Setup runs the old version's own uninstaller silently
  (`Uninstall.exe /S _?=<folder>`). It removes the old programs, shortcuts, associations and
  Add/Remove Programs entry, and keeps the user's files, as silent uninstalls do. Setup then
  deletes that uninstaller, moves any mods left in the old `mods\` folder to the new one, and
  removes the old folder if it is empty. If the old version is running, it is not removed: Setup
  says so and installs anyway.
* A desktop shortcut under the old name brings a new one.
* The user's folder `%APPDATA%\TA Remastered` is not moved by Setup: the launcher or the game
  moves it to `%APPDATA%\TA Reborn` when it first starts (`ta_config::adopt_old_folder`). Until then,
  Setup writes `config.toml` into the old folder, so it moves along with everything else.

### Finding the original game

Setup looks for Total Annihilation the same way the launcher does:

1. the GOG registry key `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\1207658880`, value `path` (written
   by the GOG offline installer and by GOG Galaxy; the non-WOW64 key and the `HKCU` key are
   checked too);
2. the Steam uninstall key `Steam App 298030`, value `InstallLocation` (32- and 64-bit views);
3. Steam's own library: `<Steam>\steamapps\common\Total Annihilation`, Steam found through
   `HKCU\Software\Valve\Steam` `SteamPath`, `HKLM\SOFTWARE\WOW6432Node\Valve\Steam` `InstallPath`,
   or `Program Files (x86)\Steam` (Steam libraries on other drives are the launcher's to find: it
   reads Steam's `libraryfolders.vdf` and each library's `appmanifest_298030.acf`);
4. the usual folders: `%SystemDrive%\GOG Games\Total Annihilation`,
   `Program Files (x86)\GOG Galaxy\Games\Total Annihilation` and
   `Program Files (x86)\GOG.com\Total Annihilation`.

A folder counts as an install when it contains `totala1.hpi` and `totala2.hpi`.

Before installing, Setup shows an **"Original game"** page:

* If the game was found, the page confirms the folder; **Browse…** picks another one (it is
  validated, and picking the parent folder, e.g. `C:\GOG Games`, also works).
* If not, the **"Original game required"** page explains that TA Reborn needs a legitimately
  owned copy and offers **Get it on GOG.com** (opens
  <https://www.gog.com/game/total_annihilation_commander_pack>), **Get it on Steam** (opens
  <https://store.steampowered.com/app/298030/>), **Detect again**, **Use my existing
  installation in this folder** with **Browse…** (a folder that does not contain the game is
  rejected with a warning), and **Continue without it and set it up later in the launcher** (the
  default), with a warning that TA Reborn cannot be played until then.

Detection runs again every time the page is shown, so a user who installs the game meanwhile can
go Back and Next. Setup also refuses to install into the original game's folder.

### Settings written by Setup

After installing, the chosen or detected game folder is written to
`%APPDATA%\TA Reborn\config.toml` of the user running Setup (the `APPDATA` environment variable,
as the game reads it, else the roaming AppData folder):

```toml
# Written by the TA Reborn installer. Other settings are managed by the launcher.
game_dir = 'C:\GOG Games\Total Annihilation'
```

It uses a TOML literal string (single quotes), so backslashes need no escaping; the file is UTF-8.
For the rare path containing a `'`, Setup writes a basic string with `\\` and `\"` escapes instead.
The file is written **only if `config.toml` does not exist yet**, so reinstalling or upgrading never
overwrites the user's settings, and only for a folder that contains the game. Nothing is written
when the user chooses to set the game up later.

If a *different* administrator account approves the UAC prompt, Setup runs as that account and the
file goes to its profile; the launcher then simply detects the game on first start.

### Uninstalling

The uninstaller removes the program files, the shortcuts, the file types and the Add/Remove
Programs entry. Files that users added to the program's `mods\` folder are kept (and with them the
folder). It then asks whether to also delete the current user's `%APPDATA%\TA Reborn` —
settings, campaign progress, saved games, replays, personal mods and logs; the default answer is
**No**, and silent uninstalls always keep them. Like Setup, it does not run while TA Reborn
runs from the installation folder.

### Command line and silent installs

| Switch | Effect |
| --- | --- |
| `/S` | Silent install (or uninstall, for `Uninstall.exe`). |
| `/D=<folder>` | Installation folder. Must be the **last** argument and is not quoted, even with spaces. |
| `/DESKTOPSHORTCUT` | Also create the desktop shortcut (the Components page is not shown in silent mode). |
| `/GAMEDIR=<folder>` | Original game folder to record, if valid: `/GAMEDIR="D:\Games\Total Annihilation"`. |
| `/UPDATE` | The launcher updating itself, over an installed version: only the progress shows, and the launcher starts again when it is done. |

For example `TA-Reborn-2.1.0-windows-x64-setup.exe /S /DESKTOPSHORTCUT`. In silent mode the detected game
(or `/GAMEDIR`) is recorded as described above; otherwise the game is set up later in the launcher.
A silent uninstall is `"C:\Program Files\TA Reborn\Uninstall.exe" /S`.

Exit codes: 0 success, 1 cancelled by the user, 2 aborted (unsupported Windows, TA Reborn still
running, …), 740 no administrator rights, 1633 incompatible CPU architecture. NSIS writes no log file; **Show details** on the progress
page lists what was done, including whether `config.toml` was written.

### Testing the installer without installing it

Compiling with `/DTEST_PER_USER` produces a per-user test build that runs without elevation,
installs only to the folder given with `/D=` (it refuses to run without one unless a test install
exists), registers its uninstaller under `HKCU`, creates its shortcuts in the current user's Start
menu, and also accepts `/NODETECT` to behave as if the original game were not installed. Never
publish it. With `APPDATA` pointed at a scratch folder, it leaves your real settings alone:

```powershell
$test = "$PWD\scratch\nsis_test"
& "${env:ProgramFiles(x86)}\NSIS\makensis.exe" /DTEST_PER_USER /DAPP_VERSION=2.1.0 /DAPP_ARCH=x64 `
    "/DSTAGE_DIR=$PWD\dist\stage-windows-x64\TA Reborn" "/DOUTPUT_FILE=$test\setup-test.exe" installer\ta_reborn.nsi
$env:APPDATA = "$test\AppData"   # for this PowerShell session only
Start-Process "$test\setup-test.exe" "/S /D=$test\Program" -WindowStyle Hidden -Wait
Start-Process "$test\Program\Uninstall.exe" "/S _?=$test\Program" -WindowStyle Hidden -Wait
```

(`_?=` runs the uninstaller in place and waits for it; it cannot delete itself then.)

## Legal

Total Annihilation is © its respective owners. TA Reborn is an unofficial, non-commercial fan
project, contains no original game assets, and requires a legitimately owned copy of the game.
