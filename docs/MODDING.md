# Modding TA Reborn

TA Reborn plays Total Annihilation's own data, and a mod changes that data: units, weapons, maps,
build menus, sounds, pictures. A mod needs no code. It is a folder of TA's files or one of TA's
archives, laid over the original game in an order the player chooses. The community's big
conversions made for TA (TA: Escalation, Total Mayhem, TA: Enhanced and others) install from the
launcher and play.

This guide has two parts. [Part 1](#part-1-playing-with-mods) is for players: getting mods,
turning them on, the load order, updates, the compatibility report, saved games and multiplayer.
[Part 2](#part-2-making-mods) is for mod makers: the layout TA Reborn expects, the `mod.toml`
manifest, how mods are layered, what TA Reborn reads (the features of TA's later patched engines
among it), and the tools for checking and testing a mod.

It describes the current development version; [CHANGELOG.md](../CHANGELOG.md) says what each
release brought. Every check the launcher runs on a mod is in
[MOD_VALIDATION.md](MOD_VALIDATION.md); TA's file formats, as TA Reborn reads them, in
[FORMATS.md](FORMATS.md).

**Part 1: playing with mods**
[What a mod is](#what-a-mod-is) ·
[The MODS page](#the-mods-page) ·
[Getting mods](#getting-mods) ·
[Turning mods on and off](#turning-mods-on-and-off) ·
[PLAY on a mod](#play-on-a-mod) ·
[Load order and dependencies](#load-order-and-dependencies) ·
[Updates](#updates) ·
[Removing and hiding](#removing-and-hiding) ·
[The compatibility report](#the-compatibility-report) ·
[Saved games, replays and multiplayer](#saved-games-replays-and-multiplayer) ·
[Where things live](#where-things-live) ·
[Troubleshooting](#troubleshooting)

**Part 2: making mods**
[A first mod](#a-first-mod) ·
[Folder layout](#folder-layout) ·
[The mod.toml manifest](#the-modtoml-manifest) ·
[Archives or loose folders](#archives-or-loose-folders) ·
[How mods are mounted](#how-mods-are-mounted) ·
[Replacing the original game's archives](#replacing-the-original-games-archives) ·
[Renamed folders](#renamed-folders) ·
[What TA Reborn reads](#what-ta-reborn-reads) ·
[Checking a mod](#checking-a-mod) ·
[Installing a package as the launcher does](#installing-a-package-as-the-launcher-does) ·
[Testing in the game](#testing-in-the-game) ·
[Publishing a mod](#publishing-a-mod) ·
[Where to look in the code](#where-to-look-in-the-code)

---

## Part 1: playing with mods

### What a mod is

A mod is one of:

- a folder holding TA's folders (`units/`, `objects3d/`, `maps/`…) and TA archives, usually with a
  `mod.toml` file that names it;
- a single TA archive (`.hpi`, `.ufo`, `.ccx`, `.gpf` or `.gp3`), such as a classic unit pack.

The mods that are on are laid over the original game in the load order: where two provide the
same file, the one lower in the order wins. Nothing of the original game changes on disk: TA
Reborn reads TA's files where they are.

The library marks each mod by what it is: **TOTAL CONVERSION** (one of the catalogue's), **MAP
PACK** (maps imported on the **MAPS** page), **TA ARCHIVE** (a single archive) or **MOD**
(anything else, your own included). Maps the map editor saves are mods too.

### The MODS page

Open **MODS** in the launcher (Ctrl+2). It has two views:

- **LIBRARY · n**: the catalogue's mods and those installed, as cards or as a list. Search
  (Ctrl+F), filter (**ALL**, **INSTALLED**, **ON**, **UPDATES**, **PLAYS WELL**), sort (**ORDER**:
  spotlight, name, release date, download size) and choose **CARDS** or **LIST**. Choosing a mod
  shows its details beside the library, or in its place on a narrow window. A right-click on a
  card or a row (or Shift+F10) opens everything that can be done with that mod.
- **LOAD ORDER · n**: the mods that are on, in the order they load.

Over them: **INSTALL FROM FILE…**, **MOD FOLDERS…** and **RESCAN** (F5). Mods that could not be
read in the last scan are listed under **PROBLEMS (n)**.

### Getting mods

#### From the catalogue

The library starts with the catalogue: community mods TA Reborn has tested, each with its
picture, its creators, the release tested and a review. At the time of writing it holds TA:
Escalation, Total Mayhem, TA: Enhanced, TA Frenzy, Talon, Devolution, Total Battletech, TA:
Twilight, The Secret Forces and The Final Frontier, and links to two mods with nothing to install
(`apps/ta_launcher/src/catalogue.rs` has the list). How one installs depends on where its creators
publish it:

- **GET · size** downloads the tested release (from ModDB for most), checks it against the size
  and digest the catalogue pins, unpacks it and installs it.
- A mod its creators give out only from their own website (TA: Escalation, The Final Frontier)
  has **WEBSITE ↗** instead: download the file there in your browser, then choose **Have the
  file?** in the mod's details and pick it. The launcher installs it only if it is the tested
  release (its exact size and digest); any other file is refused, and a copy installed before
  stays as it was.
- **Have the file?** works for the downloadable ones too, when you have the archive already.

These controls are in a mod's details, and in the menu a right-click on its card opens. A
catalogue mod installed this way for the first time is turned on, and the other catalogue mods are
turned off: each of them changes the whole game. Map packs and mods of your own stay as they are.

Installs need the network for a download, go one at a time with their progress on the mod's card,
and can be cancelled.

#### From a file: INSTALL FROM FILE… or drag and drop

**INSTALL FROM FILE…** picks a file. Dropping a file or a folder on the launcher's window does the
same (on the **MAPS** page a drop imports maps instead).

| You bring | It becomes |
| --- | --- |
| A mod's package: `.zip`, `.7z` or `.rar` | Unpacked, its TA content found and installed as a mod folder |
| A mod's folder | Copied the same way; your folder is left as it was |
| A TA archive: `.ufo`, `.hpi`, `.ccx`, `.gpf` or `.gp3` | Checked that it reads, then copied into your mods folder as it is |

What the installer does with a package or a folder:

- It unpacks with 7-Zip when it is installed, else with the system's `tar` (Windows 10 and later
  have one, as does macOS). When it says no program was found, or that only part of a file could
  be unpacked, install 7-Zip from 7-zip.org and try again: some RAR files need it, and on Linux it
  (or `bsdtar`) is needed for any package.
- It finds the content: the folder of a `mod.toml` that stands beside TA's folders or archives;
  else the folder holding the largest TA archive (recognised by its contents, whatever its
  extension); else the shallowest folder holding TA's folders.
- It keeps the TA archives there (one with an extension of its own, such as Total Battletech's
  `rev31.mwp`, is installed as a `.gp3`), TA's folders, folders the mod's own engine read in place
  of TA's ([renamed folders](#renamed-folders)) and documents (`.txt`, `.md`, `.rtf`, `.pdf`,
  `.doc`, `.docx`, `.htm`, `.html`, `.nfo`), which go in the mod's `docs/` folder: those beside the
  content, and those at the top of a folder of documents of its own (`Docs/`, `Documentation/`,
  `Manual/`...).
- It leaves out the rest: patched `TotalA.exe` copies, DLLs, settings, archives that do not read,
  and optional extras in other folders. Empty placeholder archives, which a few mods ship to blank
  out one of the original game's, are not copied: the manifest records them instead
  ([below](#replacing-the-original-games-archives)).
- It refuses a package that would unpack to more than 8 GiB or 100,000 files (read from its list
  of contents before anything is unpacked, where the unpacker gives one), or that holds links.
- It writes the mod's `mod.toml`. A package laid out as a mod already keeps its own id, name,
  version and dependencies. Anything else is named after its file or folder: its id is the
  name's letters and digits in lower case (`TotalM1130.zip` gives `totalm1130`), its version
  unknown.

A TA archive is a mod by itself: its id is its file name without the extension, in lower case.

A mod installed from a file is off: turn it on to play with it. A package that is exactly a
catalogue mod's tested release installs as that catalogue mod, as **Have the file?** does.
Bringing a mod you have installed already replaces that copy (the same id), and the launcher asks
first: **REPLACE name?**, or **KEEP**. It can tell before unpacking for a folder, a TA archive or a
package named like the installed mod; a package whose own `mod.toml` names a mod installed under
another name stops once unpacked, before anything changes, and is asked about the same way.

#### Mods in Total Annihilation's own folder: ADD AS A MOD

Some mods' own installers put their archives in TA's folder, beside TA's, for a patched engine of
their own: TA: Escalation's `TAESC.gp3` and `T*ESC.ufo`, Total Mayhem's `mayhem.gp3` and
`TADemoM.ufo`. TA Reborn recognises such archives by the folders of their own they use, and never
reads them as part of the original game. The **MODS** page lists each such mod under **IN YOUR
TOTAL ANNIHILATION FOLDER**. **ADD AS A MOD** installs it as a mod: its archives are hard-linked
from TA's folder (copied when your mods folder is on another drive), and TA's folder is left as it
is. The catalogue's mods keep their names and pages; which release the folder holds cannot be told,
so the version is unknown.

Unit packs and maps in TA's folder that use TA's own folders are read as part of the original game,
as TA read them.

#### By hand: the mod folders

**MOD FOLDERS…** shows where mods are looked for, in this order:

| Folder | Where | Notes |
| --- | --- | --- |
| PROGRAM FOLDER | `mods` in TA Reborn's own folder, beside `ta-reborn.exe` (`Contents/Resources/mods` in the macOS app) | Under Program Files it needs an administrator to change. The launcher never creates it, and does not update or remove the mods in it. |
| USER FOLDER | `%APPDATA%\TA Reborn\mods` | Where the launcher installs. |
| EXTRA FOLDER | A folder you choose with **ADD EXTRA FOLDER…** | Optional; **REMOVE EXTRA FOLDER** forgets it. It cannot be TA's own folder. |

When two folders hold a mod of the same id, the first one in this order wins, and the other is
listed under **PROBLEMS**. A mod folder (with or without a `mod.toml`) or a TA archive put in one of
these folders is found with **RESCAN**. A `.zip` put there is not read: install packages with
**INSTALL FROM FILE…**.

### Turning mods on and off

The switch in a mod's details (or **TURN ON** and **TURN OFF** in its menu) turns it on or off for
the next game: the launcher writes the settings, and the game reads them as it starts. A mod
turned on goes to the bottom of the load order.

Turning on a catalogue mod while another is on asks first (**TURN ON name?**): **SWITCH TO name**
turns the other off, **KEEP BOTH ON** keeps both. Such mods are rarely made to be played together.

Turning off a mod that others need says so: they cannot be loaded without it, and the load order
shows the problem until it is put right ([below](#load-order-and-dependencies)).

### PLAY on a mod

An installed catalogue mod has **PLAY** in its details and its menu: it turns that mod on, turns
the other catalogue mods off (map packs and your own mods stay as they are) and starts the game. If
the game does not start (a game running already, an install under way, a load order that does not
resolve), the load order is put back as it was. The **PLAY** page plays with the mods that are on,
and names them ("Mods: … → …").

### Load order and dependencies

**LOAD ORDER** lists the mods that are on, loading from the top down: where two change the same
file, the lower one wins. Drag a row, use its arrow buttons, or press Alt with the arrow keys
(Alt+Home and Alt+End to either end). Each row shows the mod's folder, version and check badge; a
click on the badge opens its report. A mod that is on in the settings but installed nowhere is
marked **NOT FOUND**, with a button to take it out.

Under the list, **EFFECTIVE ORDER** shows the order the game will use ("Base game → …"). It can
differ from the list: a mod's dependencies always load before it, and a mod may ask to load after
others. A mod's manifest can name:

- `depends`: mods that must be on too. They load before it.
- `load_after`: mods it loads after, when they are on. A wish that would close a circle gives way.

When the order cannot be resolved, the problem takes the place of the effective order, with what
puts it right: **ENABLE name** for a dependency installed but off, **GET name** for one the
catalogue can download. Until it is resolved the launcher does not start the game; a game started
another way loads no mod at all, and its title screen says why. A mod that is on and cannot be read
(a damaged archive) also leaves every mod out: the game starts without mods, and its title screen
says why.

| Problem | What to do |
| --- | --- |
| "X is on, but it is not installed" | Install X, or take it out of the load order. |
| "X is on, but its manifest … does not read: …" | Fix its `mod.toml` ([troubleshooting](#troubleshooting)). |
| "X needs Y, which is not on" | Turn Y on (**ENABLE Y**), or turn X off. |
| "These mods each need another loaded before them: …" | Their `depends` make a circle: one of the mods is wrong. |
| "These mods need one another more than 64 deep: …" | A chain of dependencies too long: one of the mods is wrong. |
| "X could not be read: …" | Its archive or folder is damaged or unreadable: install it again, or turn it off. |

### Updates

Updates come from the catalogue. A catalogue mod older than its tested release shows **UPDATE ·
size**; the **UPDATES** filter lists them, and **UPDATE ALL · n** installs them all (hidden mods
aside). An update keeps the mod on or off as it was. One is offered only over a copy whose version
is known and older: never over a newer release you installed yourself. A copy added from TA's
folder has no known version, so it waits for no update, though its details and its menu offer
UPDATE where the catalogue can download the release.

When ModDB lists a release newer than the tested one, the mod's details say so (**Newer on
ModDB: … (not tried here yet)**). **Have the file?** takes only the tested release. A newer file
installed with **INSTALL FROM FILE…** becomes a mod of its own (named after its file, unless it
carries a `mod.toml` with the catalogue's id): turn the catalogue's copy off and the new one on.

A mod of your own is updated by installing its new version from its file, which replaces the copy
installed when it is the same mod (the same id).

- A mod kept in the extra folder is updated there (a catalogue update, or a replacement the
  launcher asked about).
- A copy in the program folder is never updated by the launcher: it comes before your mods folder,
  so replace it there.
- Nothing is installed over a mod while a game runs.

### Removing and hiding

**REMOVE** (in a mod's menu, or in its details: a bin for the catalogue's mods) asks first, naming
any mod that needs it, then takes the mod out of the load order. It goes to the Recycle Bin on
Windows (where it cannot, Windows asks before deleting it), to the trash on Linux (the
freedesktop.org trash your file manager shows) and macOS; where it cannot go to the trash there, it
stays where it was, and **SHOW SOURCE FILES** leads to it.
**REMOVE** is unavailable while a game runs, an install is under way or maps are being imported,
and a copy in the program folder must be removed there.

**HIDE FROM LIBRARY** takes a mod out of the library's lists and counts, and changes nothing else:
an installed mod stays installed, and one that is on still loads (the load order lists it, marked
**HIDDEN**). Catalogue mods you never want to see can be hidden as well. **HIDDEN · n** in the
toolbar lists the hidden mods, each with **UNHIDE**; **UNHIDE ALL** puts them all back.

### The compatibility report

Every installed mod is checked in the background. The check decodes the files the game reads and
follows their references, over the original game, TA Reborn's own content and the mods it depends
on, and compares the result with the same content without the mod. It never runs anything from
the mod and never changes its files, and it waits while a game, an install or the **MAPS** page is
at work.

The badge on a mod's details and in its load order row:

| Badge | Meaning |
| --- | --- |
| DATA CHECKS PASSED | Nothing found that counts against the mod. |
| ISSUES FOUND | Warnings count against it: things the game works round, or that show. |
| ERRORS FOUND | Content the game cannot use, in the mod or caused by it. Its card carries a red lamp. |
| CHECK INCOMPLETE | The check could not cover every file (a dependency missing, a limit reached). Never a pass. |
| NOTHING TO CHECK | The mod holds nothing the checks read (music, documents). |
| CHECK FAILED | The check itself could not run: run it again. |
| NOT CHECKED, CHECK QUEUED, CHECKING MOD…, RECHECK NEEDED | No current result yet. |

Before a catalogue mod is installed, its badge is the catalogue's review: **REVIEW: PLAYS WELL**,
**REVIEW: KNOWN GAPS**, or **ON MODDB ONLY** for one with nothing to install.

**MOD INFO** opens the report, **MOD COMPATIBILITY**, in three tabs:

- **OVERVIEW**: what the check found in a word and a sentence, with its errors, warnings and notes
  (each opens **FINDINGS** showing only those); what the mod holds of its own (units, weapons,
  features, maps, sounds, files); **HOW TA REBORN PLAYS IT** ([below](#how-ta-reborn-plays-it));
  the findings to fix first; for a catalogue mod, what the project saw of it in play; and which
  version was checked, when, and over what.
- **FINDINGS** groups the findings by what they concern (packaging, files, units, weapons,
  features, maps, scripts, models and textures, animations and effects, sounds, menus and sides).
  The filters **ERRORS**, **WARNINGS** and **NOTES** show what counts, with how many; notes never
  count against a mod. A search finds findings by their words, file, archive or code. Three or
  more findings of one check with one fix are one card (**N OF ONE KIND**), the first three shown.
  Each finding gives its file, its line, the archive it came from and its code, and **HOW TO FIX**
  opens what to do about it.
- **INHERITED** findings are there without the mod too: the original game's own slips, TA
  Reborn's, a dependency's. They are hidden until asked for and never count against the mod. A
  finding in inherited content that appears only with the mod is marked **CAUSED BY THIS MOD**: a
  mod that replaces a shared file can break units it never mentions.
- **PROJECT REVIEW** (catalogue mods) is what TA Reborn's own testing of the listed release found,
  kept apart from the check of your copy.

**VALIDATE MOD** (**CHECK AGAIN** once there is a report) checks your copy again; a visit to the
**MODS** page also notices files changed by hand and checks those mods again. **COPY FULL REPORT**
copies every finding, the inherited ones included, and how TA Reborn plays the mod, in English:
send it to a mod's author, or attach it to a bug report. A mod's details show the check in brief
under **COMPATIBILITY** (its counts, and how many of the things it uses are played as made or not
used); a click opens the report.

A pass says the data is sound. It does not certify gameplay, balance, multiplayer or looks.

#### How TA Reborn plays it

Many mods were made for patched versions of TA's engine, or brought files and programs of their
own. The check notes what a mod's own files use of them, and how TA Reborn plays each: **PLAYED**,
as the engine the mod was made for played it, or **NOT USED**. These notes never count against the
mod.

| What the mod has | How TA Reborn plays it |
| --- | --- |
| Build lists for the computer players (TA 3.1's `[CANBUILD]`) | Played: the computer players build from them; menus offer what the mod's pages and download lists put there ([build menus](#build-menus)) |
| Side panels laid out for a taller screen than TA's 480 pixels | Played: the classic interface lays them out as the mod does ([below](#side-panels-made-for-768-pixel-screens)) |
| Build buttons sharing a place, or one taking several | Played ([below](#places-shared-by-several-buttons)) |
| Scripts asking TA 3.1's questions about units, or a later patch's cloak | Played: answered ([below](#scripts-and-ta-31s-questions)) |
| Weapon IDs past 255, or shared with another weapon | Played: TA Reborn finds a weapon by its name and never reads its ID |
| Sides and campaigns of its own | Played: offered in skirmish and multiplayer, and on the campaign screen |
| Files for TA's own computer players (`ai/`) | Not used: TA Reborn's computer players are its own |
| Programs and engine patches (`TotalA.exe`, DLLs) | Not used ([below](#what-ta-reborn-does-not-run)) |

[MOD_VALIDATION.md](MOD_VALIDATION.md) lists the codes the copied report and `ta-tools
validate-mod` give them.

A mod's details also say **WHAT IT CHANGES**: how many files it has, which of the original game's,
TA Reborn's or earlier mods' files it replaces, which of its files later mods replace, and which of
the original game's archives it takes the place of. Under that, its **DOCUMENTS**, the folder it is
in and **SHOW SOURCE FILES**.

### Saved games, replays and multiplayer

Saved games and replays keep the mods they were made with, in load order: each mod's id, name,
version and a digest of its files. The lists of saved games and replays say what each needs.
Opening one whose mods are not as they were (missing, off, another version or other files, others
on, another order) asks first (**MADE WITH OTHER MODS**): in the launcher, which compares the mods'
versions, and in the game, which compares their files too.

| In the launcher | In the game | What it does |
| --- | --- | --- |
| USE ITS MODS AND CONTINUE, USE ITS MODS AND WATCH | USE ITS MODS AND RESTART | Turns its mods on in its order and the others off (map packs stay on), and opens it |
| GET IT, SHOW IN MODS | GET IT IN THE LAUNCHER | A missing mod: downloads it from the catalogue, or shows how to install it |
| CONTINUE ANYWAY, WATCH ANYWAY | LOAD ANYWAY, WATCH ANYWAY | Opens it with the mods on now |

A saved game is its battle played over from the start to where it was saved, and a replay its
orders played over: with other mods they can lead elsewhere. The digest covers the files the game
reads: a folder's archives, and its files in TA's folders or in folders it renames to them. Editing
or adding a read-me file, a picture or a program changes nothing; editing a unit makes earlier
records say the mod's files have changed. (Records made by a version whose digest covered every
file of the folder still know the same copy.) Records made before they kept their mods are judged
by their game data alone.

In multiplayer everyone must have the same game data: in practice the same mods, in the same
versions and order. A join that cannot go ahead says which mods differ and what to do (turn one on
or off, install it, get the same version). Maps are the exception: players with different map
packs can play together, as long as each has the battle's map with the same contents; the lobby
shows who lacks it.

### Where things live

| What | Windows | Linux and macOS |
| --- | --- | --- |
| Settings, the `[mods]` section among them | `%APPDATA%\TA Reborn\config.toml`; each commander profile's in `%APPDATA%\TA Reborn\profiles\<id>\config.toml` | `~/.config/ta-reborn/` (`$XDG_CONFIG_HOME/ta-reborn/` when set), the same layout |
| Installed mods (the user folder) | `%APPDATA%\TA Reborn\mods\` | `~/.config/ta-reborn/mods/` |
| The check's reports | `%APPDATA%\TA Reborn\cache\mod-validation\` | `…/cache/mod-validation/` |
| Catalogue pictures, downloads and unpacking | `%APPDATA%\TA Reborn\cache\moddb\` | `…/cache/moddb/` |
| Logs | `%APPDATA%\TA Reborn\logs\launcher.log` and `game.log` | `…/logs/` |

Each commander profile has settings of its own, so its own load order, hidden mods and extra
folder; installed mods are shared by all of them. The settings' `[mods]` section:

```toml
[mods]
enabled = ["totalmayhem", "naval-additions"]  # the mods that are on, in load order (ids)
hidden = ["czta"]                              # left out of the library
extra_dir = 'D:\TA mods'                       # the extra folder, if one is set
```

Ids are read in lower case. The launcher keeps this section; to edit it by hand, close the
launcher first.

### Troubleshooting

| What you see | What to do |
| --- | --- |
| "No program to unpack archives was found" or "Only part of … could be unpacked" | Install 7-Zip (7-zip.org) and try again. |
| "No Total Annihilation content was found in the package" | It holds no TA archive and none of TA's folders. It may be an installer (`.exe`), or archives inside an archive (some map packs are zips of zips): unpack the outer one and install what is inside. |
| "… does not match the verified release" | Not the catalogue's tested release, or a damaged download. Download it again from the mod's website. |
| "… is in your mods folder already, so it is installed" | Turn it on in the library. |
| "… holds much more than a mod: drop the mod's own folder" | You dropped a drive, or a folder holding the launcher's own folders. |
| "Mod … has a conflicting installation" | That id is installed as a folder and you brought an archive, or the reverse: remove the installed copy first. |
| "Close the game first" | Mods are not installed over, updated or removed while a game runs. |
| "This copy is in TA Reborn's program folder" | Change or remove it there (it needs an administrator). |
| "The manifest … does not read" (under PROBLEMS) | Fix the `mod.toml` (the message points at the line), then **RESCAN**. Common slips: `version = 1.0` (it must be a string, `"1.0"`) and a missing `name`. |
| "Two mods are called …" | Two folders hold mods of one id; the first is used. Remove one. |
| "The id base (or reborn) is that of the original game's or TA Reborn's own content" (under PROBLEMS) | Give the mod another id: rename its folder or archive, or set `id` in its `mod.toml`. |
| "rename …: a folder of the mod's own may stand only for one of TA's" (under PROBLEMS) | Each `[rename]` entry maps a folder of the mod's to one of TA's folders: correct the table ([renamed folders](#renamed-folders)). |
| "… could not go to the trash" (Linux, macOS) | The mod stays where it was: delete its folder yourself (**SHOW SOURCE FILES**). |
| The game plays without the mods | Its title screen says why: a problem of the load order (which **LOAD ORDER** shows too), or a mod that could not be read. |
| A mod's units, menus or weapons are missing | Open its report. Check that it is on, and lower in the load order than the mods it should win over. |

The logs help too: `game.log` names the mods mounted, the original game's archives they replace,
and why mods were not loaded.

---

## Part 2: making mods

### A first mod

1. Make a folder in your mods folder: `%APPDATA%\TA Reborn\mods\naval-additions\`.
2. Put a `mod.toml` in it:

   ```toml
   id = "naval-additions"
   name = "Naval Additions"
   version = "0.1"
   ```

3. Add your files in TA's folders: `units/navscout.fbi`, `objects3d/navscout.3do`,
   `scripts/navscout.cob`, `unitpics/navscout.pcx`, and a way onto a builder's menu (its menu
   page, or a `download/` entry: [build menus](#build-menus)).
4. In the launcher's **MODS** page, press **RESCAN**, turn the mod on and open **MOD INFO**.
5. Play it, or start a battle with it at once ([testing in the game](#testing-in-the-game)).

### Folder layout

A mod's top level stands for the game's: its `units/` adds to and replaces the game's `units/`.
Paths are read in any case.

| Folder | Holds | Read |
| --- | --- | --- |
| `units/` | `*.fbi` unit definitions | Top level only |
| `objects3d/` | `*.3do` models | By the name a unit's `Objectname` (a feature's `object`) gives |
| `scripts/` | `*.cob` unit scripts | `scripts/<UnitName>.cob`, else the model's, `scripts/<Objectname>.cob` |
| `unitpics/` | `<unitname>.pcx` build pictures | |
| `weapons/` | `*.tdf` weapon definitions, any number to a file | Top level only |
| `features/` | `*.tdf` feature definitions | Any depth; a subfolder names the world (`features/<world>/`) |
| `gamedata/` | `sidedata.tdf` (sides, `[CANBUILD]`), `moveinfo.tdf` (movement classes), `sound.tdf` (sound categories) | |
| `guis/` | `*.gui` pages: builders' menus (`<builder><n>.gui`), the side panel's | Top level only |
| `download/` | `*.tdf` entries putting units on builders' menus | Top level only |
| `anims/` | `*.gaf`: effects, explosions, build buttons' art | |
| `textures/` | `*.gaf` textures the models paint with | |
| `maps/` | `<map>.ota` with its `<map>.tnt` | Top level only; a map without its `.tnt` is left out |
| `camps/` | Campaigns and their missions | |
| `sounds/` | `*.wav` sounds | |
| `music/` | Music tracks | A track named as the install names its own (`music/7.mp3` in GOG's copy) replaces it |
| `palettes/`, `fonts/`, `bitmaps/` | The palette, fonts, interface pictures | |

- A file replaces the one at the same path below it. To change one of TA's units, ship its file at
  its path (`units/armcom.fbi`); to add a unit, a file of a new name.
- Definitions are also known by their names. When two files define the same unit (`UnitName`),
  weapon or feature, the one later in path order wins, and the mod check reports it. Do not rely
  on that: replace a definition by replacing its file.
- Archives are opened only at a mod's top level. One in a subfolder is never read (the check says
  so).
- A mod's loose files win over its own archives.
- Files at a mod's top level, such as a read-me file, are harmless: the game reads nothing there.
- Links (shortcuts, junctions, symbolic links) are passed over. A loose file is read up to 256 MiB,
  and folders up to 64 deep.
- A definition file that does not parse is logged and passed over; it does not stop the game.
- TA Reborn's computer players are its own: they play from the units' data and the side data's
  `[CANBUILD]` lists, and read nothing from TA's `ai/` folder.

### The mod.toml manifest

A `mod.toml` at the top of a mod folder names it:

```toml
id = "naval-additions"
name = "Naval Additions"
version = "1.2"
description = "Three new Arm ships and a floating radar tower."
authors = ["A. Modder", "B. Painter"]
source = "https://example.org/naval-additions"
depends = ["ship-hulls"]      # must be on too; loads before this mod
load_after = ["ship-tweaks"]  # loads before this mod when it is on
```

| Field | Type | Default | What it does |
| --- | --- | --- | --- |
| `id` | string | required | The mod's identity: the settings' load order, other mods' `depends`, saved games, replays and multiplayer name the mod by it. Read in lower case. Keep it the same from version to version. |
| `name` | string | required | Shown in the launcher and the game. |
| `version` | string | `""`, shown as unknown | Shown, kept in saved games and replays, and named when two copies differ. A string: `"1.2"`, not `1.2`. |
| `description` | string | `""` | Shown in the mod's details. |
| `authors` | list of strings | `[]` | Shown as "by …". |
| `source` | string | `""` | The mod's web page. The launcher writes it for catalogue installs; it does not show it at present. |
| `depends` | list of ids | `[]` | Mods that must be on too; they load before this one. Shown as "requires …". |
| `load_after` | list of ids | `[]` | Mods this one loads after when they are on. |
| `rename` | table | none | Folders of the mod's own read in place of TA's: see [Renamed folders](#renamed-folders). |
| `replaces` | list of file names | `[]` | Archives of the original game this mod takes the place of, beyond its own archives of their names: see [Replacing the original game's archives](#replacing-the-original-games-archives). |

The id must be usable as a folder's name on every system: not empty, not `.` or `..`, without
control characters or any of `< > : " / \ | ? *`, not ending in a dot or a space, and not a name
Windows keeps for devices (`CON`, `PRN`, `AUX`, `NUL`, `COM1`, `LPT1`…). It may not be `base` or
`reborn`, the ids the original game's and TA Reborn's own content go by. The launcher and
`ta-tools mod-install` install ids of at most 64 letters. Lower-case letters, digits and hyphens
are safest.

- The file is TOML, at most 1 MiB, named `mod.toml` in any case (`Mod.toml` is found on Linux
  too). Keys TA Reborn does not know are ignored.
- A `mod.toml` that does not read leaves its mod out (the launcher lists it under **PROBLEMS**,
  with the line); it is never replaced by a made-up manifest.
- Name the folder after the id. The id need not match it, but the launcher installs a mod in a
  folder of its id, and when a mod is not found it looks for a broken manifest in a folder of that
  name to say why.
- Installing a package that has a `mod.toml`, the launcher writes the installed copy's `mod.toml`
  itself, from these fields (comments are not kept), adding any renamed folders and placeholder
  archives it found.

A mod without a manifest (a folder or an archive put in a mods folder by hand) is given one: its
id is its folder's whole name, or its archive's name without the extension, in lower case
(`TA.Escalation` is `ta.escalation`); its name is as written; it has no version; and the renamed
folders it uses are found from what it holds ([below](#renamed-folders)).

### Archives or loose folders

TA's archives are one format (`HAPI`) under five extensions: `.hpi`, `.ufo`, `.ccx`, `.gpf`,
`.gp3`. A mod may be loose files, archives, or both.

- Loose files are the easiest to work on: change a file, **RESCAN**, try it.
- Archives are how TA's mods were published, and how many players will drop yours in. A single
  archive is a mod by itself, its id its file name.
- Within one mod, archives are layered by extension, `.hpi` under `.ufo` under `.ccx` under
  `.gpf` under `.gp3`; of two with the same extension, the alphabetically first wins, as in TA.
  Loose files are over them all.
- An archive named like one of the original game's (`totala1.hpi`, `rev31.gp3`…) replaces it
  ([below](#replacing-the-original-games-archives)). Name your archives after your mod unless that
  is what you mean.
- In a folder mod, an archive that does not read is passed over (the log says so, the check
  reports an error). A mod that is a single archive that does not read leaves every mod out.

### How mods are mounted

As it starts, the game builds one file system of TA's data, in layers, each over those before:

1. **The original game.** Its archives as TA mounted them (`rev31.gp3` over `btdata.ccx` over
   `ccdata.ccx` over `totala1.hpi`), and the loose files in its folders under all of them. Left
   out: the archives the mods that are on replace, and the archives that mods made for other
   engines put in TA's folder.
2. **TA Reborn's own content**: its maps and the rest of `assets/reborn` beside the program.
3. **The mods that are on**, in the resolved load order.

The load order is resolved from the settings' list: each mod's `depends` come before it (an error
when one is not on, when they make a circle, or when they nest more than 64 deep); then its
`load_after` wishes, where they close no circle; otherwise the player's order. Every mod is read
before any is mounted: if one cannot be read, or the order does not resolve, none is mounted.

Everything that reads the game's data mounts it this way: the game, the map editor, the launcher's
check, previews and maps, `ta-tools` and the tests (`ta_vfs::mods::mount`, in
`crates/ta_vfs/src/mods.rs`).

### Replacing the original game's archives

Mods used to be installed by copying their files into TA's folder, so an archive named like one of
TA's overwrote it. TA Reborn mounts it so: with the mod on, the install's archive of that name (in
any case) is not mounted at all, and the mod's is mounted with the mod. An empty placeholder of
that name (no `HAPI` header, or no file in it) blanks the original's.

`replaces` names more: archives at the top of TA's folder that the mod takes the place of without
shipping one (an archive its read-me told players to remove):

```toml
replaces = ["btdata.ccx"]
```

The names are file names, in any case, without folders. The launcher's installer writes the
placeholders it finds beside a package's content here. TA: Enhanced is the example: its whole game
is in `rev31.gp3`, beside stubs of `totala1.hpi` and `ccdata.ccx` and an empty `btdata.ccx`. (A
copy of TA: Enhanced installed before this rule is known by its catalogue id.)

Use it sparingly: whatever only the replaced archive held is missing with your mod on, TA's own
units among it, and the check tells where that is used.

### Renamed folders

Several mods came with a copy of TA's engine patched to read folders of their own in place of TA's:

| Mod | Its folders | Read as |
| --- | --- | --- |
| Total Mayhem 11.3 | `guim`, `unitpicm`, `weaponm` | `guis`, `unitpics`, `weapons` |
| TA: Escalation | `unitse`, `unitpice`, `guie`, `weapone`, `gamedate`, `downloadse` | `units`, `unitpics`, `guis`, `weapons`, `gamedata`, `download` |
| Talon 1.03 | `unittal`, `guital`, `weaptal`, `gametal`, `downloadtal` | `units`, `guis`, `weapons`, `gamedata`, `download` |
| Total Battletech 1.40 | `unitsbt`, `weapsbt` | `units`, `weapons` |

The manifest's `[rename]` table does the same. Each key is a folder of the mod's (a path within
it, in any case, each of its names an ordinary one), each value the TA folder it is read as (one of
TA's folders, or a folder in one); a manifest with another kind of entry does not read:

```toml
[rename]
"guim" = "guis"
"weaponm" = "weapons"
```

A renamed folder stands for TA's whole folder: whatever the layers below have in `guis/` is
hidden, as the mod's own engine read only its folder. Mods loaded after it can still add to
`guis/`. So do not rename a folder to add a few files: put them in TA's folder.

For a mod without a manifest, the renamed folders are found from what it holds, as the installer
finds them in a package: a top-level folder whose name starts like one of TA's (`unit…`,
`unitpic…`, `weap…`, `gui…`, `game…`, `download…`), more than half of whose files are of
that folder's kind (`.fbi`, `.pcx`, `.tdf`, `.gui`), where the mod has no folder of TA's own
name. A mod with a manifest put in a mods folder gets only the renames it declares; installed from
a package, it gets the folders found so as well. Avoid folder names that start like TA's unless
they stand in for them.

### What TA Reborn reads

TA Reborn reads TA's formats as TA's own data has them; [FORMATS.md](FORMATS.md) has the verified
details. What mod makers ask about most:

#### Units, weapons and features

- A unit's script is `scripts/<UnitName>.cob`, else its model's, `scripts/<Objectname>.cob`.
- Numbers are read as 64-bit values: a mod's values past 16,777,216 keep their precision.
- Footprints are held to 1 to 256 cells a side (with a warning past that). A unit with
  `FootprintX=0` or `FootprintZ=0`, or one that does not fly and gives a `MaxVelocity` with an
  explicit `Acceleration=0`, takes no ground: it stands where it was made ([upgrade
  modules](#buildings-that-build-their-own-upgrade-modules)).

#### Build menus

A builder's menu, what its player is offered, comes from two places:

1. **Its own pages**, `guis/<builder>1.gui`, `2`, `3`… up to 16, until one is missing. A button
   (`[COMMON]` with `id=1`) is on the menu when it is named after a unit the data has and is marked
   as a build button (`attribs` including 32), or as unit production (`commonattribs` including 4,
   which a button without the key is) and not as a page tab (`attribs` including 16). The page's
   tabs and orders are not, even when named like a unit.
2. **Download entries**, sections of `download/*.tdf` with `UNITMENU` (the builder), `UNITNAME`
   (the unit), `MENU` and `BUTTON`. As in TA, `MENU` counts the orders menu as menu 1, so `MENU=2`
   is the first build page; `BUTTON` is the place on it, from 0, two across and then down. A taken
   place passes the unit on to the next free one, or to a page added after the builder's own, laid
   out as its first page is.

Units the data does not define are left out. In the classic interface a page's buttons show
`anims/<page>.gaf`'s art of their names, a download entry's `anims/<unit>_gadget.gaf`; without art,
the unit's picture.

**Build power makes a builder.** A unit with a `WorkerTime` above 0 has a menu and builds what it
offers, whatever its `Builder` flag says: the mods' buildings that build their own upgrades have
`Builder=0`. A unit without `WorkerTime` builds nothing, whatever its pages and lists name (the
mods' missile silos list buttons for their computer players).

**`[CANBUILD]` is the computer players' list, never a menu.** The 3.1 patch's `[CANBUILD]` in
`gamedata/sidedata.tdf` (a section per builder, `canbuild1=UNIT;`…) lists what TA's computer
players build. TA Reborn's computer players build from a builder's menu and this list; a player is
never offered what only the list names. Mods use it for units of their AI alone: The Final
Frontier lists an `AAISOLAR` beside the `ARMSOLAR` its players build.

`ta-tools data --menus` prints every builder's menu, and what the lists add for the computer
players ([checking a mod](#checking-a-mod)).

#### Side panels made for 768-pixel screens

TA's side panel was laid out for a 480-pixel screen: six build places to a page, under the ORDERS
and BUILD tabs. The patched engines of later years read pages laid out for a 768-pixel screen: TA:
Escalation's and Total Mayhem's have twelve build places to a page (two across, six down from the
panel's top), the page arrows under them, and the state buttons and orders on the same page.
TA Reborn's classic interface (**CLASSIC INTERFACE** in the game's options) lays the side panel
out for the screen a side's pages were made for: as far down as its orders page
(`guis/<prefix>gen.gui`, `<prefix>` the side's `nameprefix`), its main page (`<prefix>main.gui`)
and its Commander's first build page reach, and 480 at least. Nothing needs declaring. The modern
interface lays out its own panel from the same menus.

#### Places shared by several buttons

Total Mayhem's build pages lay one place out for several variants of a building, and lay big
buttons over several places. A button no larger than a place and off the grid is a share of the
place its top-left corner is in, and a click goes to the first button in the file that holds the
point. Total Mayhem's long-range guns are four units round one picture, each setting its
`buildangle` in `Create`, so the side of the picture clicked is the way the gun faces; its hover
plants come in halves, for land and for the sea. The classic interface draws such a place as one
picture with its zones, as the file lays them out; the modern interface shows it as one tile, the
variant pointed at shown and built. Units added to a menu (download entries, the rest of its
options) go only to places where a build button of a place's size stands, never where the layout
draws nothing.

#### Scripts and TA 3.1's questions

Scripts read (`get`) and set (`set`) unit values. TA Reborn answers TA's own (1 to 20),
`VETERAN_LEVEL` (32: kills × 100), TA 3.1's questions about units by id, which the later mods'
scripts ask (`exthead.h`), and `UNIT_CLOAKED` from a later patch:

| Value | Name | Answer |
| --- | --- | --- |
| 69 | `MIN_ID` | 1, the lowest id a unit may have |
| 70 | `MAX_ID` | The highest id a unit may have |
| 71 | `MY_ID` | The unit's own id |
| 72 | `UNIT_TEAM` | The player owning the unit whose id is given (none given: the unit itself) |
| 73 | `UNIT_BUILD_PERCENT_LEFT` | How much of that unit is left to build |
| 74 | `UNIT_ALLIED` | Whether that unit is an ally of the unit asking |
| 75 | `UNIT_IS_ON_THIS_COMP` | 1: every computer plays the whole battle |
| 111 | `UNIT_CLOAKED` | Whether the unit is cloaked; setting it cloaks or uncloaks the unit |

Asked about an id no unit has, 72 to 75 read 0. Any other value reads 0, and setting it does
nothing. The mod check names such values
(`script.value`) where a script names them outright. The answers are in `crates/ta_sim/src/host.rs`;
the check's list, `ANSWERED_VALUES` in `crates/ta_data/src/validation.rs`, is held to them by a
test.

#### Buildings that build their own upgrade modules

Total Mayhem and TA: Escalation upgrade buildings with modules: units a building makes on its own
pad and takes on through its scripts. TA Reborn plays them as those engines did:

- The building has build power and a menu page offering its module (`Builder=0` with a
  `WorkerTime` is enough).
- The module takes no ground (no footprint, or a speed with `Acceleration=0`) and stands where it
  was made.
- The building's script finds the finished module through the values above (by `UNIT_HEIGHT`, near
  one of its pieces) and attaches it (`attach-unit`). A module on its building makes and stores
  what its file says, changes sides with it and is destroyed with it.
- A factory needs its build stance to start a unit, not to keep building it: these scripts leave
  the stance in `StartBuilding`.
- A building that builds only its own upgrade is no factory to the player's idle factories or to
  the computer players.

[FORMATS.md](FORMATS.md) gives the details, mod by mod; `crates/ta_sim/tests/mod_upgrades.rs` plays
every building of a mod that offers an upgrade ([testing](#from-the-source-tree)).

#### What TA Reborn does not run

TA Reborn runs no code from a mod: patched executables (`TotalA.exe` and the like), DLLs and
settings files are left out of installs, and the report notes one a mod folder holds as **NOT
USED**. What such an engine did by itself, beyond what the data and scripts say, does not happen;
the script values it answered that TA Reborn does not are named by the check.

### Checking a mod

The launcher checks every installed mod (**MOD INFO**). `ta-tools validate-mod` runs the same check
from the command line:

```text
ta-tools validate-mod "<mod folder or TA archive>" [--mod <dependency>]... [--json] [--fail-on error|warning|never] [--inherited]
```

| Option | What it does |
| --- | --- |
| `--mod <folder or archive>` | A mod this one depends on, and those that one depends on: repeat it. They are mounted in the order the game would mount them together, whatever order they are given in. Without one the mod needs, the check is incomplete. |
| `--json` | The report as JSON instead of text. |
| `--fail-on` | `error` (the default): exit with 1 when an error counts against the mod, or the check is incomplete. `warning`: a warning too. `never`: always 0. |
| `--inherited` | Also list the findings that are there without the mod. |

It prints what counts against the mod, then how TA Reborn plays what it uses
([above](#how-ta-reborn-plays-it)); `--json` gives both. It finds the TA install by itself (the
registry, Steam, the usual folders); `TA_REBORN_GAME_DIR` names another. A mod whose `mod.toml` does not read is refused, as the game leaves it out. It
checks folders and TA archives: to check a `.zip`, install it first with `ta-tools mod-install`
into a scratch folder ([below](#installing-a-package-as-the-launcher-does)).

`ta-tools` is not in TA Reborn's packages. Build it from the source with
`cargo build --release -p ta_tools`: it is then `target/release/ta-tools`.

Its other commands read the game with mods over it: `--mod`, repeatable, mounts each in the order
given (without resolving dependencies).

| Command | What it does |
| --- | --- |
| `ta-tools --mod <mod> data` | Loads every unit, weapon, feature, movement class and side; lists reference problems and builders with nothing to build |
| `ta-tools --mod <mod> data --menus` | Every builder's menu, a line each (`menu NAME n: …`), and what the computer players' lists add (`ai menu NAME n: …`) |
| `ta-tools --mod <mod> data --unit ARMCOM` | One unit's definition as read, with its weapons |
| `ta-tools --mod <mod> find units --ext fbi` | The files under a folder, each with the archive or folder it comes from (`-r` for subfolders) |
| `ta-tools --mod <mod> cat units/armcom.fbi` | A file as the game reads it (`--out <file>` saves a binary one) |
| `ta-tools --mod <mod> validate` | Decodes every GAF, 3DO, COB, TNT, PCX and TDF file and reports failures |
| `ta-tools --mod <mod> cob scripts/armcom.cob` | Disassembles a script |
| `ta-tools --mod <mod> model objects3d/armcom.3do` | A model's pieces (`--obj <dir>` writes it as OBJ) |
| `ta-tools info` | The install found, its folders, and the mods in TA's folder left out of it |

### Installing a package as the launcher does

```text
ta-tools mod-install <package.zip|.7z|.rar> <mods folder> [--id <id>] [--name <name>]
```

It unpacks the package, picks its content as the launcher does, prints what it chose (archives,
folders, documents, renamed folders, placeholders, the units and maps, what it left out) and
installs it in the mods folder given. A package with a `mod.toml` keeps it (one that does not
read is refused, as the launcher refuses it); `--id` and `--name` choose others, and the id is held
to the launcher's rules. Use it to see what players will get from your package.

### Testing in the game

Keep your tests out of your own settings:

- **Windows:** set `APPDATA` to a scratch folder holding `TA Reborn\config.toml` and
  `TA Reborn\mods\<id>\`, then start the game from that shell:

  ```powershell
  $env:APPDATA = 'D:\ta-test'
  # D:\ta-test\TA Reborn\config.toml holds:
  #   [mods]
  #   enabled = ["naval-additions"]
  & 'C:\Program Files\TA Reborn\ta-reborn.exe' --map "Coast To Coast"
  ```

- **Linux and macOS:** `XDG_CONFIG_HOME` does the same (`$XDG_CONFIG_HOME/ta-reborn/`).
- **The launcher:** `ta-launcher --config D:\ta-test\config.toml` keeps its settings, mods
  (`D:\ta-test\mods`), caches and logs in that folder, and the games it starts use it too.

The game's options for trying a mod:

| Option | What it does |
| --- | --- |
| `--map "<map>"` | Starts a skirmish on that map at once |
| `--open-skirmish`, `--skirmish-map "<map>"` | Opens the skirmish page (with that map chosen) |
| `--skip-intro` | Skips the intro movies |
| `--no-mods` | Plays without the mods that are on, to compare |
| `--classic-ui` | The classic interface whatever the settings say (side panels made for 768-pixel screens, shared places) |
| `--game-dir <folder>` | The original game's folder, when it is not found |
| `--load <file.tasav>`, `--replay <file.tarep>`, with `--anyway` | Opens a saved game or replay; `--anyway` opens it as it is, though made with other mods |
| `--screenshot <file.png> --frames <n>` | Saves a picture after n frames and closes |

The game's log (`logs\game.log` in the settings folder) names the mods mounted, and the original
game's archives they replace.

#### From the source tree

With TA Reborn's source and a Rust toolchain, the tests play a mod over the game. `TA_TEST_MOD`
names the mod's folder (an absolute path, with its `mod.toml`):

| Test | What it does |
| --- | --- |
| `cargo test --release -p ta_ai --test skirmish -- --nocapture` | Computer players fight a battle with the mod (`TA_TEST_SIDES=TAL,CORE` its sides, `TA_TEST_MAP`, `TA_TEST_MINUTES`) |
| `cargo test --release -p ta_sim --test animation_scan -- --ignored --nocapture` | Every unit's animations, checked for pieces sunk into the ground, flown off or crossed over, and pieces standing on nothing (`TA_UNITS=a,b` for some) |
| `cargo test --release -p ta_sim --test mod_upgrades -- --ignored --nocapture` | Each building and factory builds the module or upgrade its menu offers, and takes it on |
| `TA_LAYOUT_DIR=<dir> cargo test --release -p ta_game --bin ta-reborn scenario::placement_tests::scenario_layout_maps -- --ignored --nocapture` | Every scenario's layout pictured from above, with its faults listed |

### Publishing a mod

- Give it a `mod.toml` with a stable `id` and a `version` string, and raise the version with every
  release. With a `mod.toml`, a new version replaces the old whatever the package's file is called;
  without one, the id comes from the file's name, and a renamed package installs as another mod.
- Put the content at the top of the package, or in one folder holding the `mod.toml` beside TA's
  folders. Read-me files beside the `mod.toml`, and those at the top of a `docs/` folder, are kept
  in the mod's `docs/`; other folders of your own (sources) are left out.
- A ZIP needs no extra program on Windows and macOS; on Linux, and for some RAR files, players need
  7-Zip.
- Ship no executables or DLLs (they are left out), and none of TA's own files: players have the
  game.
- Name your archives after your mod, not after TA's, unless replacing TA's archive is what you mean.
- Declare in `depends` the mods yours needs: players are told, and turn them on with a click.
- Run `ta-tools validate-mod` (with `--mod` for each dependency) until nothing counts against the
  mod, or you know why. `--fail-on error` suits an automated build.
- Try the package with `ta-tools mod-install`, then play it.

### Where to look in the code

| Topic | Files |
| --- | --- |
| The manifest, discovery, the mod folders, mounting, the load order, digests | `crates/ta_vfs/src/mods.rs` |
| Layers, archive precedence, renamed folders | `crates/ta_vfs/src/lib.rs` |
| Installing packages | `crates/ta_vfs/src/package.rs` |
| Units, weapons, features, sides, build menus | `crates/ta_data/src/lib.rs`, `crates/ta_data/src/unit.rs` |
| The mod check | `crates/ta_data/src/validation.rs`, `apps/ta_launcher/src/validation.rs` |
| Script values | `crates/ta_sim/src/host.rs` |
| The classic build pages, 768-pixel layouts, shared places | `apps/ta_game/src/hud_classic_art.rs` |
| The catalogue, installs, the MODS page | `apps/ta_launcher/src/catalogue.rs`, `getmods.rs`, `modlist.rs`, `mod_library.rs`, `pages/mods.rs`, `pages/mod_order.rs`, `pages/mod_report.rs` |
| The mods of saved games and replays | `crates/ta_config/src/records/mods.rs` |
| `ta-tools` | `apps/ta_tools/src/main.rs` |
