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
[Presenting a mod in the launcher](#presenting-a-mod-in-the-launcher) ·
[Theming the interface](#theming-the-interface) ·
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
- It finds the content: the folder of a `mod.toml` that stands beside TA's folders (or TA
  Reborn's own, `menus/`) or archives; else the folder holding the largest TA archive (recognised
  by its contents, whatever its extension); else the shallowest folder holding TA's folders; else
  the folder of a `mod.toml` with an interface theme, pictures or archives it replaces of its own
  (a mod that brings only those).
- It keeps the TA archives there (one with an extension of its own, such as Total Battletech's
  `rev31.mwp`, is installed as a `.gp3`), TA's folders, TA Reborn's own (`menus/`: a menu theme's
  pictures, panel art and list, as the Supreme Commander import makes them), folders the mod's own
  engine read in place of TA's ([renamed folders](#renamed-folders)) and documents (`.txt`, `.md`,
  `.rtf`, `.pdf`, `.doc`, `.docx`, `.htm`, `.html`, `.nfo`), which go in the mod's `docs/` folder:
  those beside the content, and those at the top of a folder of documents of its own (`Docs/`,
  `Documentation/`, `Manual/`...).
- It leaves out the rest: patched `TotalA.exe` copies, DLLs, settings, archives that do not read,
  and optional extras in other folders. Empty placeholder archives, which a few mods ship to blank
  out one of the original game's, are not copied: the manifest records them instead
  ([below](#replacing-the-original-games-archives)).
- It refuses a package that would unpack to more than 8 GiB or 100,000 files (read from its list
  of contents before anything is unpacked, where the unpacker gives one), or that holds links.
- It writes the mod's `mod.toml`. A package laid out as a mod already keeps its own: its id,
  name, version and dependencies, its comments and keys TA Reborn does not know. Anything else is
  named after its file or folder: its id is the name's letters and digits in lower case
  (`TotalM1130.zip` gives `totalm1130`; the first 64 of them, with `-mod` after a name no mod may
  take: `base.zip` gives `base-mod`), its version unknown.

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
- `conflicts`: mods that must not be on with it. With one of them on too, the order does not
  resolve: turn one of them off.
- `requires_reborn`: the oldest TA Reborn it plays in. In an older one the order does not
  resolve: update TA Reborn, or turn the mod off. Its card shows a red lamp.

When the order cannot be resolved, the problem takes the place of the effective order, with what
puts it right: **ENABLE name** for a dependency installed but off, **GET name** for one the
catalogue can download, and **TURN OFF name** for the mod that needs it, for a mod that needs a
newer TA Reborn, and for each of two mods that cannot be on together (turn off either). Until it is
resolved the launcher does not start the game; a game started another way loads no mod at all, and
its title screen says why. A mod that is on and cannot be read (a damaged archive) also leaves every
mod out: the game starts without mods, and its title screen says why.

| Problem | What to do |
| --- | --- |
| "X is on, but it is not installed" | Install X, or take it out of the load order. |
| "X cannot be on with Y" | X's manifest says the two break each other: turn one of them off (**TURN OFF X** or **TURN OFF Y**). |
| "X needs TA Reborn 1.0.7 or later" | Update TA Reborn (the launcher's Play page offers updates), or turn X off (**TURN OFF X**). |
| "X is on, but its manifest … does not read: …" | Fix its `mod.toml` ([troubleshooting](#troubleshooting)). |
| "X needs Y, which is not on" | Turn Y on (**ENABLE Y**), or turn X off (**TURN OFF X**). |
| "X needs Y, which is not installed" | Install Y (**GET Y** where the catalogue has it), or turn X off (**TURN OFF X**). |
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
  features, maps, scripts, models and textures, animations and effects, sounds, menus and sides,
  and presentation: the mod's pictures, links and interface theme).
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
reads: a folder's archives, and its files in TA's folders or in folders it renames to them (not a
menu theme's `menus/`, which plays no part in a battle). Editing or adding a read-me file, a
picture or a program changes nothing; editing a unit makes earlier records say the mod's files
have changed. (Records made by a version whose digest covered every file of the folder still know
the same copy.) Records made before they kept their mods are judged by their game data alone.

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

`ta-tools mod-new "%APPDATA%\TA Reborn\mods\Naval Additions"` makes steps 1 and 2: the folder,
a `mod.toml` naming the mod (its id made from the name, every other key written out and
commented), a `media` folder and a read-me of what goes where (`--id` and `--name` choose them).
By hand:

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
| `objects3d/` | `*.3do` models, and beside one its `<name>.surface` ([its own paint](#models-with-surfaces-of-their-own)) | By the name a unit's `Objectname` (a feature's `object`) gives |
| `scripts/` | `*.cob` unit scripts | `scripts/<UnitName>.cob`, else the model's, `scripts/<Objectname>.cob` |
| `unitpics/` | `<unitname>.pcx` build pictures | |
| `weapons/` | `*.tdf` weapon definitions, any number to a file | Top level only |
| `features/` | `*.tdf` feature definitions | Any depth; a subfolder names the world (`features/<world>/`) |
| `gamedata/` | `sidedata.tdf` (sides, `[CANBUILD]`), `sidedata_<name>.tdf` ([side add-ons](#sides-of-a-mods-own)), `moveinfo.tdf` (movement classes), `sound.tdf` (sound categories) | |
| `guis/` | `*.gui` pages: builders' menus (`<builder><n>.gui`), the side panel's | Top level only |
| `download/` | `*.tdf` entries putting units on builders' menus | Top level only |
| `anims/` | `*.gaf`: effects, explosions, build buttons' art | |
| `textures/` | `*.gaf` textures the models paint with; the `*.dds` textures surfaces name (anywhere) | |
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
conflicts = ["old-navy"]      # must not be on with this mod
requires_reborn = "1.0.7"     # the oldest TA Reborn it plays in
tags = ["units", "naval"]     # found by these in the library, shown on its page

[media]                       # how the launcher presents it (see below)
banner = "media/banner.png"
preview = "media/gameplay.gif"
icon = "media/icon.png"
screenshots = ["media/harbour.jpg", "media/battle.jpg"]

[links]                       # buttons on its page, by their words
"Home page" = "https://example.org/naval-additions"
Discord = "https://discord.gg/example"
```

| Field | Type | Default | What it does |
| --- | --- | --- | --- |
| `id` | string | required | The mod's identity: the settings' load order, other mods' `depends`, saved games, replays and multiplayer name the mod by it. Read in lower case. Keep it the same from version to version. |
| `name` | string | required | Shown in the launcher and the game. |
| `version` | string | `""`, shown as unknown | Shown, kept in saved games and replays, and named when two copies differ. A string: `"1.2"`, not `1.2`. |
| `description` | string | `""` | Shown in the mod's details. |
| `authors` | list of strings | `[]` | Shown as "by …". |
| `source` | string | `""` | The mod's web page: a **MOD PAGE** link on its page (a catalogue mod's page is the catalogue's). The launcher writes it for catalogue installs. |
| `depends` | list of ids | `[]` | Mods that must be on too; they load before this one. Shown as "requires …". |
| `load_after` | list of ids | `[]` | Mods this one loads after when they are on. |
| `conflicts` | list of ids | `[]` | Mods that must not be on with this one: with one of them on too, no mod is mounted until one is turned off. Shown as "cannot be on with …". |
| `requires_reborn` | string | `""` | The oldest TA Reborn the mod plays in (`"1.0.7"`): in an older one it is not mounted. Shown on its page, in red when this TA Reborn is older. |
| `rename` | table | none | Folders of the mod's own read in place of TA's: see [Renamed folders](#renamed-folders). |
| `replaces` | list of file names | `[]` | Archives of the original game this mod takes the place of, beyond its own archives of their names: see [Replacing the original game's archives](#replacing-the-original-games-archives). |
| `tags` | list of strings | `[]` | Words the library finds the mod by, shown on its page as written. |
| `media` | table | none | Its banner, gameplay preview, icon and screenshots: see [Presenting a mod in the launcher](#presenting-a-mod-in-the-launcher). |
| `links` | table | none | Its web pages, `"Words on the button" = "https://…"`: links on its page, in the order of their words. Only `http://` and `https://` addresses are opened. |
| `theme` | table | none | The colours, pictures and font its battles' HUD is drawn in: see [Theming the interface](#theming-the-interface). |

The id must be usable as a folder's name on every system: not empty, not `.` or `..`, without
control characters or any of `< > : " / \ | ? *`, not ending in a dot or a space, and not a name
Windows keeps for devices (`CON`, `PRN`, `AUX`, `NUL`, `COM1`, `LPT1`…). It may not be `base` or
`reborn`, the ids the original game's and TA Reborn's own content go by. The launcher and
`ta-tools mod-install` install ids of at most 64 letters. Lower-case letters, digits and hyphens
are safest.

- The file is TOML, at most 1 MiB, named `mod.toml` in any case (`Mod.toml` is found on Linux
  too). Keys TA Reborn does not know are read past, and kept when the launcher or the map editor
  writes the file again. TOML puts every key below a `[table]` line in that table: the keys above
  (`requires_reborn`, `depends`...) go above the first `[table]` line, or they are read as that
  table's and not as the mod's. **MOD INFO**'s check names the keys read past.
- A value of another kind than its key takes, in a key that only presents the mod (`version`,
  `description`, `authors`, `source`, `tags`, `media`, `links`, `theme`), is left out and the
  rest read (`version = 1.2` gives no version: write `"1.2"`). In a key that decides whether the
  mod is mounted (`id`, `name`, `depends`, `load_after`, `conflicts`, `requires_reborn`, `rename`,
  `replaces`), the manifest does not read. One string where a list is wanted is a list of one
  (`tags = "units"`, `depends = "ship-hulls"`); TA Reborn 1.0.6 and earlier refuse it, so write a
  list for them.
- A `mod.toml` that does not read leaves its mod out (the launcher lists it under **PROBLEMS**,
  with the line); it is never replaced by a made-up manifest.
- Name the folder after the id. The id need not match it, but the launcher installs a mod in a
  folder of its id, and when a mod is not found it looks for a broken manifest in a folder of that
  name to say why.
- Installing a package that has a `mod.toml`, the launcher keeps it as written (comments and keys
  it does not know too), changing only what the install sets: the id and the name it is installed
  as, and any renamed folders and placeholder archives it found. A mod installed from the catalogue
  is named by the catalogue (its id, name, version, description, authors and page), and keeps the
  rest of its package's own `mod.toml`: `depends`, `load_after`, `conflicts`, `requires_reborn`,
  `replaces`, `media`, `links`, `tags` and `theme`.

A mod without a manifest (a folder or an archive put in a mods folder by hand) is given one: its
id is its folder's whole name, or its archive's name without the extension, in lower case
(`TA.Escalation` is `ta.escalation`); its name is as written; it has no version; and the renamed
folders it uses are found from what it holds ([below](#renamed-folders)).

### Presenting a mod in the launcher

The launcher's **MODS** library shows each mod as a card, and the chosen mod's page beside the
cards. A mod installed from a file or a folder looks as good there as one from the catalogue when
its `mod.toml` names pictures of its own, in a `media` folder beside it:

```text
naval-additions/
  mod.toml
  media/
    banner.png        1280 × 640, the card and the top of the page
    gameplay.gif      an animated GIF of the mod in play, played in the banner
    icon.png          256 × 256, beside the mod's name
    harbour.jpg       screenshots, shown on the page
    battle.jpg
  units/ ...
```

```toml
[media]
banner = "media/banner.png"
preview = "media/gameplay.gif"
icon = "media/icon.png"
screenshots = ["media/harbour.jpg", "media/battle.jpg"]
```

| Key | What it is | Best |
| --- | --- | --- |
| `banner` | A wide picture: the mod's card in the library (twice as wide as tall) and the top of its page. A PNG or a JPEG. | 1280 × 640, its subject in the middle: the page shows a little less of its sides |
| `preview` | An animated GIF of the mod in play. It plays in the banner on the mod's page, and on its card while the pointer rests on it (a ▶ in the card's corner tells it has one). | 5 to 15 seconds, about 640 pixels wide, at most 32 MB |
| `icon` | A square picture, beside the mod's name on its page; on a card without a banner, with the name on the mod's own plate. | 256 × 256 |
| `screenshots` | Pictures of the mod in play: a row on its page, each opening on the whole window (the arrow keys step through them). The first 12 are shown. | 1920 × 1080 PNGs or JPEGs |

- Each is named by its path from the mod's folder, with `/` between folders, in any case of its
  letters. A path that leaves the folder (`..`, a drive, a root) is never read.
- Pictures may weigh up to 16 MB each (the preview 32 MB) and be up to 4096 pixels a side. The
  launcher reads them on worker threads and keeps no more of them than are on show: a screenshot
  shown on the whole window is let go as it closes, and the previews as the library is left. One
  that is not there, or does not read, shows a quiet cross in its place.
- A preview's frames are kept in memory as it plays: a long or a large one plays in fewer frames
  (every other one kept, each shown for both, as often as it must), and a frame that asks for no
  time at all shows for a tenth of a second, as in a web browser. On the mod's page, **STOP
  PREVIEW** stops it. It does not play with **REDUCE MOTION** on (its page has a **PLAY PREVIEW**
  button then; a card's never plays), nor while the launcher is in the background or stands idle.
  A still picture, or a GIF of one frame, is no preview: the banner shows as it is.
- Any of them may be left out. A mod without a banner gets a plate of its own: dark metal of a
  colour of its id's, its name lettered on it (and its icon beside the name).
- The installer keeps the `media` folder whole (its PNG, JPEG, GIF, TTF and OTF files, up to 256 MB
  together), and a picture the manifest names elsewhere beside the content (`banner.png` beside
  `mod.toml`). The pictures and the font the manifest names go first: the rest of a large folder
  never crowds them out. The game never reads the `media` folder, and the library's **WHAT IT CHANGES**
  leaves it out.
- The pictures are found again every few seconds: one changed on the disk shows without a
  **RESCAN** (a change to `mod.toml` itself needs one).
- **MOD INFO**'s check tells what would not show as meant, under **PRESENTATION**: a picture that
  is not there or does not read (pictures are read by their content and whole, as the launcher
  reads them), one too small or of the wrong shape, a preview that does not move or that the
  launcher plays choppier than it was made (its frames at the size it is shown take more than the
  96 MB it keeps for a preview), a link that is no web address. These are notes: they never change
  the mod's badge, which is how it plays.

The repository's [example mod](examples/example-mod) has a banner, an icon, a preview and
screenshots of its own, and a `mod.toml` naming them.

### Theming the interface

A mod's `[theme]` draws its battles' HUD in colours of its own, with pictures behind its plates
and a font for its titles: the look of the mod in play as well as in its units. The theme is the
last mod's in the load order that has one; a player may turn mods' themes off (**MODS' INTERFACE
THEMES**, in the game's Interface options).

The theme draws the battle's HUD, and only that:

- **Themed**: the top bar, the command deck and the minimap's plate (classic or modern HUD), the
  messages and the overlay's legend, the allies' cards, the radial menu, the unit cards and health
  bars over the battlefield, the HUD's floating bars (the replay and spectator controls, the chat,
  a scenario's or a Conquest battle's goals, the players' table, the pause banner), the enemy
  Commanders' transmissions, the cast's scoreboard and kill feed (on screen and over a film made of
  the replay), and the companion screen. egui's own controls in them (fields, drop-downs, hints)
  take the theme's colours and font too.
- **TA Reborn's own look, whatever the theme**: the menus and the launcher, the loading screen,
  the battle menu and its pages (options, statistics, controls), the dialogs on TA Reborn's plates
  that the HUD opens (sharing, cheats, a unit's page, the battle's history, a replay's video and
  clip export), the result page, the tutorial's windows, the tips and hints, and the cockpit (its
  view, its glass and instruments).

```toml
[theme]
accent = "#5ab4ff"                      # titles, the chosen, the main action (TA Reborn's brass)
text = "#dfe7ef"
dim = "#8d9aa8"                         # labels and secondary text
panel = "#18202b"                       # the panels' metal
metal = "#a8c4dc"                       # the resource bar's metal
energy = "#ffd34d"                      # and its energy
good = "#6fd88a"                        # health, lamps lit
bad = "#ff6b5a"                         # damage, warnings
panel_texture = "media/hud/panel.png"   # tiled behind the command deck's and the minimap's plates
bar_texture = "media/hud/bar.png"       # behind the top bar, stretched to its height
font = "media/hud/MyFont.ttf"           # the HUD's titles, numbers and buttons

[theme.classic]                         # the classic HUD's lettering
text = "#e6eef6"

[theme.sides.core]                      # one side's own, over the rest
accent = "#ff7a4d"
```

| Key | What it colours | TA Reborn's own |
| --- | --- | --- |
| `accent` | Titles (the side's name on the top bar), the chosen, the main action (its button made of the accent as TA Reborn's is of brass, lettered in whatever reads on it), focus, the deck's tabs, lit keycaps. Its brighter and darker shades are made from it, unless `accent_bright` gives the brighter. | `#d8b456` |
| `text` | Text; `text_bright` the headings and values (else made from `text`). | `#d8d3c4` |
| `dim` | Labels and secondary text. | `#9e998a` |
| `panel` | The panels' metal, where the side has no tiles of its own and no `panel_texture`, and the HUD's floating bars' (the replay bar, the chat, the goals); the background, the raised and the hovered from it, and the wells and lines unless `inset` and `line` give them. | `#1f1e1a` |
| `inset`, `line` | Wells sunk in the panels (bars, fields); hairlines and borders. | `#0d0e09`, `#3c3932` |
| `metal`, `energy` | The resource bar's metal and energy. | `#aab8c4`, `#f2d43c` |
| `good`, `bad`, `info` | Health and lamps lit; damage and warnings; radar and allies. | `#78d65c`, `#e05e4a`, `#68b8e8` |
| `button_text` | The buttons' stencil lettering. | `#e2deb6` |
| `panel_texture`, `bar_texture` | Pictures (PNG, JPEG or a GIF's first frame, read by their content, up to 2048 pixels a side and 16 MB) in place of the side's own metal, `bitmaps/<side>guisidetile.pcx` and `<side>guitoptile.pcx`. Made seamless, a few hundred pixels across. | the side's tiles |
| `font` | A TrueType or OpenType font (up to 16 MB) for the HUD's titles, numbers and buttons; TA Reborn's own gives the letters it lacks. It is set on the HUD's lines: scaled so that a line of it is as tall as one of TA Reborn's (1.2 times the size), its capitals in the middle of it, so a font whose line is taller than its letters looks smaller. Body text stays TA Reborn's. | Bahnschrift, else Sofia Sans |
| `classic.text`, `classic.button_text`, `classic.dim`, `classic.pressed` | The classic HUD's lettering: its text, its buttons', dimmed, a button held down. | TA's |

- Colours are written `#rrggbb` (or `#rgb`, `#rrggbbaa`). Anything left out keeps TA Reborn's
  own, so a theme may be a single `accent`.
- `[theme.sides.<side>]` takes the colours (`accent`, `accent_bright`, `text`, `text_bright`,
  `dim`, `panel`, `inset`, `line`, `good`, `bad`, `info`, `button_text`, `metal`, `energy`) and
  the pictures (`panel_texture`, `bar_texture`), for one side (named as `gamedata/sidedata.tdf`
  names it, any case); what it leaves out is the theme's. `font` and `[theme.classic]` are the
  theme's for every side: written under a side, they are not read. A spectator following another
  player sees that player's side's.
- The classic HUD is TA's own art: a mod's interface files (`guis/`, `anims/<side>int.gaf`...)
  change it as they did in TA; the theme gives its lettering.
- Team colours, the units and the map are never themed. Status colours keep their sense: the
  health bars' yellow between `good` and `bad` stays yellow, and the words the HUD sets on its own
  dark backdrops (the messages over the battlefield) are lit until they read there, whatever the
  theme's `text` and `accent`.
- The pictures and the font go in the mod's `media` folder, which the installer keeps; the game
  reads them from the mod's folder (not from its archives).
- The theme draws only in battle, the HUD as listed above. The mod's page in the launcher shows
  a preview of it: its top bar and its panels' picture repeated as the HUD repeats them, and each
  of its colours with its value.
- **MOD INFO**'s check tells under **PRESENTATION** what the game would not draw as meant: a
  colour that is none, a picture or a font that is not there or does not read (both read as the
  game reads them), text or an accent hard to read where it is drawn (on the theme's panels, on
  its plates' picture, and on the sides' own dark metal where it gives the plates no picture; the
  theme's own colours and each side's), a font the HUD shows smaller or larger than its size (its
  lines far from 1.2 times its size: the HUD fits it to its own), a side the game has not. A mod of
  its `mod.toml` alone, a theme and nothing else, is checked complete: nothing of the game's data
  to check.

The [example mod](examples/example-mod) has a theme of its own, with its panel and bar pictures.

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
- A unit carries up to **eight weapons**, `Weapon1` to `Weapon8` (TA's own carry three at most).
  For each, the engine calls TA's script functions for the first three slots (`AimPrimary`,
  `AimFromPrimary`, `QueryPrimary`, `FirePrimary`, then `...Secondary` and `...Tertiary`), or the
  numbered ones for any slot (`AimWeapon1` to `AimWeapon8`, `AimFromWeapon<n>`, `QueryWeapon<n>`,
  `FireWeapon<n>`, as Spring names them): a script defines one or the other. A weapon's bad
  targets are `wpri_`, `wsec_` or `wspe_badTargetCategory` for the first three, or
  `BadTargetCategory<n>` for any slot (`n` from 1), else the unit's `BadTargetCategory`. TA
  itself never fires a fourth weapon, and the mod check reads the numbered functions too.
- **A weapon's only targets.** `OnlyTargetCategory<n>` (Spring's name; `n` the weapon's slot,
  from 1) names the categories of the only units the weapon takes on: of its own accord, firing
  back, and by order too. Told to attack any other unit, or a place on the ground, it does not
  fire (an attack order none of the unit's weapons may take ends at once); nor does it take on a
  unit it names where it cannot hit it (an aircraft landed, for a `toairweapon=1` weapon). TA's
  `toairweapon=1` alone keeps a weapon's own choice of targets to aircraft, an attack order still
  firing it at anything, as TA Reborn plays TA's own units. The Supreme Commander factions'
  anti-air, which shoots aircraft alone in its game, says `OnlyTargetCategory<n>=VTOL` beside it.
- **Missile defences by class.** A weapon's `interceptor` and a missile's `targetable` are read
  as sets of classes, bits of a number, as the Spring engine reads them: an interceptor shoots
  down a missile whose `targetable` shares a bit with its `interceptor`. TA's own all say 1 (its
  anti-nukes take its nukes, EMP and tactical missiles), so they play as ever; a mod may keep its
  defences apart, say `targetable=1` on its nukes and `interceptor=1` on its anti-nukes,
  `targetable=2` on its tactical missiles and `interceptor=2` on its tactical missile defences
  (as the Supreme Commander factions do: an anti-nuke never wastes itself on a cruise missile,
  and a tactical defence never stops a nuke), and `interceptor=3` takes both. A weapon whose
  number is not 0 is an interceptor (or targetable) as before. An interceptor fires once the
  missile is within its `range` (its `coverage`, where that is larger; with no range, wherever it
  is): TA's anti-nukes reach across TA's maps, and a tactical defence with a range of a few hundred
  px takes the missiles as they come down on what it covers, so give its shot the speed to meet
  them there. The no-nukes mutator leaves out
  what fires or stops the strategic class (bit 1), and the computer players build anti-nukes
  against strategic silos alone and count only the defences of a missile's classes in its way.
- **Missile speeds.** A missile leaves at its `startvelocity` (none: at its `weaponvelocity`, or
  launched upright with a `weaponacceleration`, at rest) and its `weaponacceleration` takes its
  speed towards its `weaponvelocity`, stopping there: up to it, or (a negative acceleration) down
  to it. One launched faster than its `weaponvelocity` is never pulled down to it, and with a
  negative acceleration slows as it flies (no slower than half a pixel a tick, never turned
  about): TA: Escalation's vertical missiles say `weaponvelocity=-10` and slow from their start
  speed. Launched upright (`vlaunch=1`) at a negative `startvelocity`, a missile goes straight
  down. Its `flighttime` is its life: give one that slows the time to get where it is sent.
- **Where a shot leaves from.** The piece the weapon's `Query` function names, where it is once
  the weapon's `Fire` function has run as far as its first wait (`sleep`, `wait-for-turn`,
  `wait-for-move`): a fire function may move that piece into place before the shot leaves (as TA:
  Escalation's C.O.R.E. brings its launch points back into its pods), or kick it back, and the shot
  leaves from there. The query is asked first, so a fire function that switches barrels for the
  next shot flashes the barrel the query named.

#### Sides of a mod's own

A mod adds sides without replacing the game's side data, which would take another mod's sides
away: a `gamedata/sidedata_<name>.tdf` (any name after `sidedata_`) adds its `[SIDE<n>]`
sections after the game's sides (and any other add-on's, in order of their paths), and joins
its `[CANBUILD]` lists to the game's. A side named as one before it is passed over. The sides
are numbered on from the game's. What a side does not lay out of its own for the classic
interface (the places of its resource bars and numbers, its logo, its lettering), it takes box by
box from a side before it drawn in the same panels (its `intgaf`), else from the first side: a side
may name its art and nothing more. The Supreme Commander factions add theirs so
(`gamedata/sidedata_supcom.tdf`).

#### Campaigns and missions

A mod's `camps/` adds campaigns, and a mission OTA of its own in `maps/` takes the place of TA's of
that name (TA: Escalation and Total Mayhem ship their copies of TA's). A schema's units are read
from its `[units]` section, or, where it has none, from the section whose entries name units
(`Unitname`) whatever it is called: TA: Escalation's patched engine read its missions' units from
`[unitsE]`, as it read `unitse` for `units`. The units are those of the mod's data by name: TA's
missions with a mod that redefines `ARMCOM` place the mod's.

A mission the data with your mod cannot play is not started: the campaign page marks it
**MISSION NOT PLAYABLE**, with why, when it gives the player no units (a map made for skirmishes
listed as a mission), none the data has (a total conversion's units in place of TA's), no Commander
where it is lost with one, or when every way to win it names a unit type the data lacks. `ta-tools
--mod <folder> missions` lists the unit names a mod's data lacks mission by mission; the campaign
smoke test plays every mission with a mod ([Testing in the game](#testing-in-the-game)).

#### Models with surfaces of their own

A 3DO can be painted beyond TA's palette and 64-pixel textures: with a **surface** beside it,
`objects3d/<name>.surface`, its vertices take texture coordinates and smooth normals, and its
polygons materials of three DDS textures each: a colour map, a specular map (red how much it
mirrors its surroundings, green how glossy, blue where it glows, alpha where it takes the player's
colour) and a normal map (x in alpha, y in green, Direct3D's way). The models TA Reborn makes of
Supreme Commander's units are painted so. The format is in [FORMATS.md](FORMATS.md#model-surfaces-objects3dnamesurface-ta-reborns-own).

- A polygon takes the material its texture name names; one naming no material of the surface is
  drawn as TA draws it. The surface must have the model's pieces in its order and each piece's
  vertices; one that does not (or does not read) is left out, the model drawn as TA draws its
  faces (without Reborn's plates and seams, which would cut a dense mesh up along every
  triangle), and the log and the mod check say so.
- The textures must be DDS; they are drawn BC3 with every mip level, at 512 × 512, or at 1024 ×
  1024 for a texture of 1024 or more a side with HD surfaces (larger ones at their 1024 level; at
  their 512 level without HD surfaces, as small graphics cards want it). Give them so (DXT5, or
  DXT1 for a colour map or DXT3, of 512 × 512 or 1024 × 1024 with every level below it;
  `ta_formats::dds::to_bc3_levels` makes them) and they are read as they are, in about a
  millisecond. Any other is decoded, resized and encoded again the first time it is drawn: tens of
  milliseconds a texture, three to a material (a 1024 × 1024 uncompressed one, about forty); a
  smaller one is enlarged to 512 keeping its edges sharp. That is done on workers of the game's
  own, the model drawn in its colours on average meanwhile, but a battle of many such materials is
  painted seconds late; the mod check notes each such texture. Mip levels past what a texture's
  size halves into, or past the end of its file, are not read (the mod check says so).
- A material's paint is read the first time a model of it is drawn, and kept for the battle (the
  next battle lets it go and reads its own); until then its faces are drawn in its colour on
  average, as they are in ray-traced reflections. Each texture takes a third of a megabyte on the
  graphics card at 512, 1.3 at 1024 (four times that where the card reads no block-compressed
  textures, and never 1024 there), room made about 32 MB at a time; when the card has no more room
  for 1024s, those are drawn at 512, and when it has none at all, the materials left over stay in
  their colours on average.
- A surface may name a lighter model drawn in its place far off (`far`), of the same pieces in the
  same order (the mod check says when it has not): under 48 pixels across on the screen, though a
  model drawn as itself a moment before stays so down to 42 (no swapping back and forth as the
  camera shakes or a walker bobs).
- Models with surfaces take no skins, liveries or Reborn's plates and bolts: their paint is their
  own, in either art style. Enhanced Classic lights it as Supreme Commander does (the specular
  map's green a broad, cool sheen); Reborn finishes it as it finishes TA's paint (greys without
  their cast, painted highlights on a shoulder, matte paint as glossy as the green is round about,
  the green's outlines worn bright, strong red polished metal; its colour map read sharp where it
  is magnified): see docs/SUPCOM_UNITS.md, "Light".
- In TA itself, such a model shows its palette colours.

#### Models that move by clips of their own

A model can have **animations** beside it, `objects3d/<name>.anims`: keyframed movements of its
pieces, played on the screen alone (the simulation, and so multiplayer, never sees them). A clip
named `walk` is played as a ground unit of the model walks, in step with the ground it covers,
instead of its script's walk (the pieces its script aims with keep the script's turns); clips
named `death`, `death2`... play where one dies on the ground, its wreck lying in the pose the
clip ends in. Three more show a unit's work (meant for buildings, played for any unit):

- `active`: played round and round while the unit works: finished, switched on (or with no
  switch), its energy upkeep paid, and, a factory or a unit with `Builder=1`, building something.
  An extractor's pump, a factory's arms at work. It slows to a stop where it is as the unit stops
  working (switched off, out of energy, starting an upgrade), and goes on from there.
- `open`: played once as the unit starts working, held open while it works, and played back as it
  stops: a hatch opening once the building is up, or every time it is switched on.
- `upgrade`: posed by how far an upgrade in place has come ([Units that upgrade in
  place](#units-that-upgrade-in-place)), from its first frame to its last: the building turning
  into the next, which then stands in its place.

These are added to what the unit's script does with the same pieces, so a script can go on
spinning a radar dish that an `active` clip raises and lowers; leave the pieces a weapon aims to
its script. The Supreme Commander units walk and die so, and their buildings pump, open and
upgrade so, their clips made from their game's own animations (their spinning parts are their
scripts' `spin`s). The format is in [FORMATS.md](FORMATS.md#model-animations-objects3dnameanims-ta-reborns-own);
`ta_formats::anims::Anims::to_bytes` writes it. A clip that does not fit its model (it moves pieces
the model has not, or its keys are out of order or past its end) is left out, the others played;
the log and the mod check name it. A death fades into its wreck's look in the moment its last pose
holds.

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

The modern interface lays the same menu out on its own tabs (by what each building is for), and
lists each tab, and a factory's units, by where they go: land first, then what goes on both, then
the sea's, then aircraft, in the pages' order within each. Each picture stands on that ground
(grass, grass and water, water, sky). Where a unit goes is read from its data as the game plays
it: an aircraft (`CanFly`) the air; a hovercraft (`CanHover`) both; a unit that needs water
(`MinWaterDepth` above 0, or `Floater` without a `MaxWaterDepth`) the sea; another unit both when
its `MovementClass` (else its own `MaxWaterDepth`) lets it go 50 or more under the water, else land;
a building the sea when it has a `MinWaterDepth` above 0, both when its `MaxWaterDepth` is 50 or
more, else land. There is no key to order a menu otherwise. A unit with a picture of its own
ground (a model with a surface and a `unitpics` picture, as the Supreme Commander factions' are) is
shown as its picture is.

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
scripts ask (`exthead.h`), Spring's `TARGET_ID`, `UNIT_CLOAKED` from a later patch, and a value of
its own, `IMMOBILE`:

| Value | Name | Answer |
| --- | --- | --- |
| 69 | `MIN_ID` | 1, the lowest id a unit may have |
| 70 | `MAX_ID` | The highest id a unit may have |
| 71 | `MY_ID` | The unit's own id |
| 72 | `UNIT_TEAM` | The player owning the unit whose id is given (none given: the unit itself) |
| 73 | `UNIT_BUILD_PERCENT_LEFT` | How much of that unit is left to build |
| 74 | `UNIT_ALLIED` | Whether that unit is an ally of the unit asking |
| 75 | `UNIT_IS_ON_THIS_COMP` | 1: every computer plays the whole battle |
| 83 | `TARGET_ID` | Spring's: the target of the unit's weapon given (from 1): a unit's id, -1 none, -2 a place on the ground, -3 a missile it intercepts, -4 no such weapon |
| 111 | `UNIT_CLOAKED` | Whether the unit is cloaked; setting it cloaks or uncloaks the unit |
| 200 | `IMMOBILE` | TA Reborn's own: whether the script holds its unit still; setting it holds it or lets it go |

Asked about an id no unit has, 72 to 75 read 0. A unit held still (`IMMOBILE` set to 1, as a
weapon of the Supreme Commander factions does while it is unpacked: `WeaponUnpackLocksMotion`)
neither moves nor turns, its orders waiting, and nothing pushes it (a unit on the ground or the
water: aircraft fly on); told to go somewhere meanwhile, its script is told `StartMoving` at once
(once), as though it were setting off, so that it can pack up and let it go, and `StopMoving` if
the way is given up before then. From then until it is let go, its weapons fire nothing, though
they aimed before (Supreme Commander's weapons fire nothing as they pack up). Let go, it sets off
without being told again. Any other value reads 0, and setting it does nothing. The mod check
names such values (`script.value`) where a script names them outright. The answers are in
`crates/ta_sim/src/host.rs`; the check's list, `ANSWERED_VALUES` in
`crates/ta_data/src/validation.rs`, is held to them by a test.

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
- A module with build power and `DefaultMissionType=RepairPatrol` (TA: Escalation's repair tower
  upgrades) mends the damaged units of its side and its allies within its `BuildDistance` by
  itself whenever it has no orders, the nearest first, standing loose or on its building. Any
  builder with that mission does the same, standing where it is
  ([below](#buildings-that-work-round-them-by-themselves)).

[FORMATS.md](FORMATS.md) gives the details, mod by mod; `crates/ta_sim/tests/mod_upgrades.rs` plays
every building of a mod that offers an upgrade ([testing](#from-the-source-tree)).

#### Factories' ways out

A factory's units drive out of its front, as TA's do: from its pad (the piece its script's
`QueryBuildInfo` names) to a point just past the front of its footprint, through its walls, which
are their way out until they are clear of it. A factory may name its own ways out instead, as
Supreme Commander's factories roll their units off their build platforms to their sides: an
`[EXITS]` section within its `[UNITINFO]`, a section for each way (any name), in order:

```text
[UNITINFO]
{
    UnitName=UEB0101;
    ...
    [EXITS]
    {
        [EXIT1] { X=128; Z=-4; Heading=90; Ramp=36 60; }
        [EXIT2] { X=-128; Z=-4; Heading=-90; Ramp=36 60; }
    }
}
```

- **`X`, `Z`**: where the way leads, in pixels from the factory's middle as it stands facing south
  (x east, z south), turned with the factory. Lead it past the footprint: the factory lets its unit
  go once it is out of the footprint.
- **`Heading`**: the way the unit faces while it is built, in degrees as a unit's heading is
  counted (0 the factory's front, its +z; 90 towards its +x), turned with the factory and with the
  pad as TA's plants turn theirs.
- **`Ramp`**: two numbers, pixels along the way from the pad: the unit keeps the pad's height (its
  build platform's) as far as the first, and comes down evenly to the ground at the second, tipped
  down the slope as it goes. Without it the unit is on the ground as soon as it is done.

A unit may name the way it faces while such a factory builds it, whichever way out it takes:
`BuildHeading` in its own `[UNITINFO]`, in degrees as a way's `Heading` (from the factory's front,
turned with the pad). The Supreme Commander factions' long transports and gunships say so
(`ForcedBuildSpin`), built along their air factories' platforms rather than across them.

A unit is built facing the way out nearest where it goes next (the first place of the factory's
route, else the factory's front), of those whose end it may stand on (not built over), the first of
equals; done, it rolls off along that way to its end, then follows the route. An aircraft lifts
off its pad, as before (built facing its way), and a unit that never moves (a module) takes no
way: it is made as the pad faces. Such a factory's units stand on its pad from the moment they are
begun. With ways out a factory's `YardMap` may be solid (`o` throughout): its units leave by their
way through its walls, and no other unit walks over its platform. The Supreme Commander factions'
factories name theirs so, from their roll-off points and their models' platforms.

#### Buildings that work round them by themselves

A builder that cannot move (a building with `WorkerTime`, `Builder=1` and a `BuildDistance`)
waits for orders, as TA's do. Its `DefaultMissionType` sets it to work round it by itself whenever
it has no orders, standing where it is, within its `BuildDistance`:

| `DefaultMissionType` | What it does |
| --- | --- |
| `RepairPatrol` | Mends the damaged units of its side and its allies, the nearest first (TA: Escalation's repair towers) |
| `AssistPatrol` | Helps build what its player is putting up (a frame, a factory's unit, a building upgrading in place), else mends the damaged units of its side and its allies, else reclaims wreckage (wrecks and their heaps, not trees or rocks), the nearest of each first: as a builder on patrol does on its way, and as Supreme Commander's engineering stations, the Kennel and the Hive, do |

What it finds itself is its only order, and gives way to its player's: any order, queued or not,
takes its place, and once that is done it goes back to its own work. Something that leaves its reach
while it works on it is let go, without a word, and taken up again once it is back within it;
waiting for work it is no idle builder (the
idle builders' button and the coach pass it over). It needs build power: the mod check tells an
`AssistPatrol` without a `WorkerTime` (`unit.assist_patrol`). Give it `canguard=1` to let players
tell it to help a unit of theirs. The Supreme Commander factions' engineering stations work so
([SUPCOM_UNITS.md](SUPCOM_UNITS.md#gameplay)). None of TA's own units has either mission.

#### Adjacency

A building may give the finished buildings of its player that touch it bonuses, as Supreme
Commander's structures do (power generators cutting what the factories beside them pay to build,
storage raising what the extractors beside it make): an `[ADJACENCY]` section within its
`[UNITINFO]` holds one section for each bonus (named as you like), and `AdjacencySize` gives the
building's size class for the bonuses it takes.

```
[UNITINFO]
	{
	UnitName=MYPLANT;
	AdjacencySize=4;
	...
	[ADJACENCY]
		{
		[CheaperBuilds]
			{
			Affects=BuildEnergy;
			Add=-0.05;
			Size=16;
			}
		[QuickerGuns]
			{
			Affects=ReloadTime;
			Add=-0.1;
			Size=4;
			Category=ARTILLERY;
			}
		}
	}
```

- **Touching:** two buildings touch where their footprints meet along an edge, a cell of it at
  least (corners alone do not, nor buildings a cell apart). Both must be finished and the same
  player's: an ally's building gives nothing. Units that take no ground (the mods' modules) neither
  give nor take.
- **`Affects`**: what of the building beside it the bonus changes. The building must have it for
  the bonus to reach it (and for the link between them to show):

  | `Affects` | What it changes | Who has it |
  | --- | --- | --- |
  | `BuildEnergy`, `BuildMetal` | The energy and metal its building costs it a second: construction, its upgrades in place, its stockpiled missiles. Its builds take as long, for less (or more) | Builders and silos (build power, or a stockpiling weapon) |
  | `EnergyUse` | Its upkeep while it is on | A positive `EnergyUse` |
  | `EnergyPerShot` | The energy its weapons' shots cost (not stockpiled missiles') | A weapon with `energypershot` |
  | `ReloadTime` | Its weapons' reload: below one, it fires faster | A weapon |
  | `EnergyMake` | The energy it makes (`EnergyMake`, the wind's and the tide's share, a solar collector's) | Something that makes energy |
  | `MetalMake` | The metal it makes (`MetalMake`, what it extracts, what it makes of energy) | Something that makes metal |

- **`Add`**: the share it adds to that figure: -0.0625 cuts it by a sixteenth, 0.125 raises it by an
  eighth. The shares of every neighbour giving a figure are summed, and the figure multiplied by
  one and the sum, never below none (four generators of -0.25 beside a factory: it builds for
  nothing). A bonus adding nothing does nothing (the mod check says so, `unit.adjacency`, as it
  does of an `Affects` TA Reborn does not play).
- **`Size`**: the size class of the buildings that take it (their `AdjacencySize`); none, any.
- **`Category`**: tokens a building that takes it must have in its `Category`, every one; none,
  any. A token no unit's `Category` names reaches nothing.
- **`AdjacencySize`**: the building's size class, which `Size` names, and the most neighbours it
  takes bonuses from: as many as touch it where it gives none; else that many, those touching it
  along the most of its edge first. (Supreme Commander names its structures' size classes by how
  many of the smallest fit round them, `SIZE4` to `SIZE20`: its wider footprints keep that many
  there, TA's footprints pack closer.)

A bonus is paid where the economy pays, in every player's game alike (in multiplayer and replays
too): a builder's draw shows it, the figures over a building (the hover card, TA's unit bar) say
what it makes and uses with its bonuses, the hover card, the selection's details and the information
page (F1) say what the buildings beside it give it and how many it helps, and a building being
placed shows the links it would make and what it would take. A frame or an upgrade given up, and a
unit its player takes apart, gives back what was paid for it, not its price. The battlefield shows a
link across the seam between two buildings that help each other, a chevron for each kind of bonus
pointing at the one it helps. The computer players put a generator beside a factory it helps and
storage beside an extractor it raises, where that pays. The Supreme Commander factions' structures
carry Forged Alliance's own adjacency ([SUPCOM_UNITS.md](SUPCOM_UNITS.md#adjacency)). TA's own units
have none.

#### Units that upgrade in place

A unit may become another where it stands, as Supreme Commander's factories, extractors and radars
do: `UpgradesTo=<UnitName>` in its `[UNITINFO]` names the unit it becomes. It is offered UPGRADE
(in the modern interface's orders, with its own key, Alt+U, and a factory's build menu offers the
unit it becomes first among its pictures, as Forged Alliance's do; on the classic interface's
orders page, among the buttons TA has no gadget for), and builds that unit on itself:

- **With its own build power** (`WorkerTime`): give a unit that builds nothing else one for its
  upgrades. The upgrade takes the new unit's `BuildTime` at that power, and builders helping it
  (guarding it, or told to repair it) lend theirs, their side paying for their share.
- **At the new unit's price**, paid as construction pays: a share of its metal and energy for each
  share of the work, slowed by the economy as construction is (short of metal, it waits).
- **While it upgrades** a factory makes nothing: the unit on its pad and its queue wait. Any other
  unit works on: an extractor extracts, a power plant makes power, a radar watches. A mobile unit
  stands still.
- **Called off** by Stop, or by any order not queued after it, the metal paid for the upgrade
  comes back (as a frame given up gives its metal back); its energy is spent. A unit captured
  while it upgrades loses the upgrade.
- **Room for it:** a unit whose next form takes more ground than it stands on (a larger footprint)
  upgrades only while no other building stands on that ground: it is refused ("no room to upgrade
  into that"), or called off, its metal back, if one is put there meanwhile.
- **Done**, the new unit stands where the old one stood, facing as it faced, its owner's and
  finished, with the old one's share of its health; it keeps the orders queued after the upgrade
  (so `UpgradesTo` chains may be queued one step after another, with Shift), a factory's queue,
  route, repeat and the unit on its pad, its fire and move orders and its experience; the rest is
  a fresh unit's (its script starts anew). The old unit is gone without a wreck, a blast or a
  loss; the selection and control groups move to the new one.

The computer players upgrade factories into a higher tier, and extractors into better ones (where
`ExtractsMetal` grows), when their economy allows; they climb the tiers so rather than build the
next tier's factory wherever a factory of theirs upgrades into it. A unit of a type the battle does
not allow (a mission's, the mutators') is no upgrade. `ta-tools data --unit <name>` shows a unit's
`upgrades_to`.

#### Enhancements

A unit may install parts on itself, as Supreme Commander's commanders do: an `[ENHANCEMENTS]`
section within its `[UNITINFO]` holds one section for each, named by its id. The order of the
sections is the order the menus show them in.

```
[UNITINFO]
	{
	UnitName=UEL0001;
	...
	[ENHANCEMENTS]
		{
		[AdvancedEngineering]
			{
			Name=Tech 2 Engineering Suite;
			Description=Builds tech 2 structures, builds faster;
			Slot=LCH;
			BuildCostMetal=720;
			BuildCostEnergy=18000;
			BuildTime=9000;
			WorkerTime=200;
			MaxDamage=3000;
			HealRate=20;
			BuildOptions=UEB0201 UEB0202 UEB1202;
			Picture=uel0001_advancedengineering;
			}
		[T3Engineering]
			{
			Prerequisite=AdvancedEngineering;
			Slot=LCH;
			...
			}
		}
	}
```

- **`Slot`**: one enhancement a slot (any name; the menus show it). One installed in a taken slot
  takes the place of the one there.
- **`Prerequisite`**: the id of one that must be installed first. Installed, this one takes its
  slot (give them the same slot, as Supreme Commander's suites do, so that its numbers stand in
  place of the other's).
- **`BuildCostMetal`, `BuildCostEnergy`, `BuildTime`**: its price and work, as a unit's. The unit
  stands and builds it on itself with its own build power, as an upgrade in place is built:
  paid as construction pays, slowed by the economy, helped by builders guarding or repairing it
  (a factory's guards turn from the unit on its pad to it); called off by Stop or another order,
  its metal comes back. A factory with enhancements makes nothing while it installs one.
- **What it changes** while installed, added to the unit's own: `MaxDamage` (its health rises by as
  much), `HealRate` (hit points a second it mends), `WorkerTime` (build power), `SightDistance`,
  `RadarDistance`, `SonarDistance`, `BuildOptions` (units added to its menu, and to what the
  computer players may build with it), `CloakCost`, `CloakCostMoving` and `MinCloakDistance` (a
  cloak it may switch on), `Stealth=1` (radar does not show it).
- **`Weapon1`** to **`Weapon8`**: the weapon a slot fires while it is installed, in place of the
  unit's own there; give the unit's file an empty slot (no `WeaponN`) for a weapon an
  enhancement adds. A launcher that stockpiles its missiles, swapped for another or taken out,
  loses the missiles built and the one under way.
- **`Name`**, **`Description`** and **`Picture`** (`unitpics/<Picture>.pcx`, shown as a unit's) for
  the menus.

The modern interface shows them in the build menu's ENHANCE tab (a click installs one, Shift
queues it, one whose prerequisite is missing comes after it; a right-click takes an installed one
out, at once and for nothing, its health falling to its most); the classic interface, behind an
ENHANCE button among the buttons TA has no gadget for. The information page (F1) lists them, those
installed marked. A unit's script with a function `SetEnhancement(index, on)` is called with an
enhancement's index (in the file's order) as it is installed (`on` 1) or taken out (`on` 0), to show
or hide its parts. The computer players install on their Commander the engineering suites (those
adding units to its menu) and one improving a weapon (reaching farther, hitting harder or firing
faster, or arming an empty slot), near their base and when they can afford it; more health when
rich.

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
| `--mission "<campaign>:<n>"` | Starts a campaign's mission at once (`"arm campaign:0"`, the first; a campaign by its file's name or its title) |
| `--screenshot <file.png> --frames <n>` | Saves a picture after n frames and closes |

The game's log (`logs\game.log` in the settings folder) names the mods mounted, and the original
game's archives they replace; for a mission, how many units it placed (and the names it left out,
unknown), and once it is decided, the goals that decided it.

#### From the source tree

With TA Reborn's source and a Rust toolchain, the tests play a mod over the game. `TA_TEST_MOD`
names the mod's folder (an absolute path, with its `mod.toml`):

| Test | What it does |
| --- | --- |
| `cargo test --release -p ta_ai --test skirmish -- --nocapture` | Computer players fight a battle with the mod (`TA_TEST_SIDES=TAL,CORE` its sides, `TA_TEST_MAP`, `TA_TEST_MINUTES`) |
| `cargo test --release -p ta_sim --test animation_scan -- --ignored --nocapture` | Every unit's animations, checked for pieces sunk into the ground, flown off or crossed over, and pieces standing on nothing (`TA_UNITS=a,b` for some) |
| `cargo test --release -p ta_sim --test mod_upgrades -- --ignored --nocapture` | Each building and factory builds the module or upgrade its menu offers, and takes it on |
| `cargo test --release -p ta_sim --test mission every_mission -- --ignored --nocapture` | Every campaign mission with the mod, 30 seconds each with the computer players: those decided early, and the goals that decided them; `every_campaign_mission_can_be_played` (not ignored) lists those the mod's data cannot play, and why |
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
| A mod's presentation: its pictures, preview and links, and their check | `crates/ta_vfs/src/mods.rs` (`Media`), `apps/ta_launcher/src/mod_media.rs`, `crates/ta_data/src/presentation.rs` |
| A mod's interface theme: the palette, applying it in battle (the HUD alone: `hud`, `reborn`), its check | `crates/ta_vfs/src/mods.rs` (`Theme`), `crates/ta_ui/src/theme.rs` (`Palette`, `hud`), `apps/ta_game/src/mod_theme.rs`, `apps/ta_game/src/hud_skin.rs`, `crates/ta_data/src/presentation.rs` |
| Script values | `crates/ta_sim/src/host.rs` |
| Adjacency, buildings working round them | `crates/ta_data/src/adjacency.rs`, `crates/ta_sim/src/sim/adjacency.rs`, `crates/ta_sim/src/sim/repair_round.rs`, `apps/ta_game/src/hud/adjacency.rs` |
| Models' surfaces and clips (walks, deaths, buildings at work) | `crates/ta_formats/src/surface.rs`, `crates/ta_formats/src/anims.rs`, `apps/ta_game/src/scene/walk.rs`, `apps/ta_game/src/scene/work.rs`, `apps/ta_game/src/effects_remains.rs` |
| The classic build pages, 768-pixel layouts, shared places | `apps/ta_game/src/hud_classic_art.rs` |
| The catalogue, installs, the MODS page | `apps/ta_launcher/src/catalogue.rs`, `getmods.rs`, `modlist.rs`, `mod_library.rs`, `pages/mods.rs`, `pages/mod_order.rs`, `pages/mod_report.rs` |
| The mods of saved games and replays | `crates/ta_config/src/records/mods.rs` |
| `ta-tools` | `apps/ta_tools/src/main.rs` |
