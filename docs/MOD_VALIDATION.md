# Mod validation and compatibility reports

The launcher checks installed mods in the background. Open **Mods**: a mod's details show its
check in brief under **COMPATIBILITY** (its errors, warnings and notes, and how TA Reborn plays
what it uses), and its **MOD INFO** control opens the report or runs the check again. **COPY
FULL REPORT** copies the whole report (every finding, inherited ones included, and how the game
plays what the mod uses) to share with a mod author or attach to a Reborn bug report. Modders can
run the same check from the command line:

```
ta-tools validate-mod "<the mod's folder or archive>" [--mod <dependency>]... [--json] [--fail-on error|warning|never] [--inherited]
```

It prints what counts against the mod (`--inherited`: everything) and exits with 1 when a
finding of `--fail-on`'s severity or worse counts (`error` by default) or the check is
incomplete (`never`: it never does), so a mod's build can run it. Give it each mod the mod
depends on with `--mod`, and what those depend on: without one, the check is incomplete. They
are mounted in the order the game would mount them together, whatever order they are given in;
mods the game would not load together (needing each other) make the check incomplete too. A mod
whose `mod.toml` does not read is refused, as the game leaves it out (a `--mod` too).

Validation never enables the mod, modifies its files or runs executables from its package.
It pauses while a game, an installation or the map library is busy.

## What counts against a mod

Each check layers the mod and the mods it depends on over the original game and Reborn's own
content, as the game does (an archive of the mod's named like one of the original game's
replaces it, the original's left out: see [Archives the mod replaces](#archives-the-mod-replaces)),
and compares the result with **the same content without the mod** (the baseline, checked once
and kept). So:

- a finding in the mod's own files, or in content they use, counts (definition by definition:
  of a file of many, as the original game's weapons or sound categories, what is wrong with
  the ones the mod uses), unless it is there without the mod too (below);
- a finding in inherited content that appears **only with the mod** counts too, marked
  **CAUSED BY THIS MOD**: a mod that replaces a shared definition file breaks units it never
  mentions, and this is how they are blamed on it;
- a finding that is there **without the mod**, the same in the same place (the original
  game's own slips, a dependency's) is **INHERITED**: it is in the report, hidden until asked
  for, and never counted against the mod, though the mod uses what it is about (a unit of the
  mod's in one of TA's sound categories, whose sound TA lacks) or carries a copy of its file
  (Total Mayhem's and TA: Escalation's copies of TA's `gamedata/sound.tdf`, Total Mayhem's of
  `anims/vismasks.gaf`, which is no GAF). A note that is there only with the mod is the mod's,
  though notes never count against it.

An **empty file** is a blank, not a broken file: an empty file over one of the original
game's (or a dependency's) is TA's way of taking that file out (Total Mayhem blanks 57 of the
original animations, 85 wreck definitions and the weapons list so). The empty files of a
folder are one note; what the files taken out leave missing is told where it is used: a
reference to an emptied file is to nothing (an emptied model leaves its unit without one).

Only what the game reads is decoded: files of its kinds in the folders it reads them from
(`units/`, `objects3d/`, `anims/`…). A picture among a mod's documents is never opened by the
game, so it is never broken: the packaging check says where the game would read it.

### Archives the mod replaces

Mods were installed by copying their files into TA's folder, so an archive of a mod's named like
one of the original game's (`totala1.hpi`, `ccdata.ccx`, `btdata.ccx`, `rev31.gp3`..., in any
case) overwrote it. TA Reborn mounts it so: with the mod on, the install's archive of that name
is not mounted at all, whatever the mod's holds (an empty placeholder blanks the original's).
What only the original's held is then missing with the mod, as it was in TA: the check tells it
where it is used (a menu of the mod's offering a unit it does not have; a campaign mission
placing one, marked caused by the mod), and the report's packaging notes say which archives the
mod replaces. A mod author can name more in `mod.toml`:

```toml
# Archives of the original game this mod takes the place of, beyond its own archives of their
# names (an archive its readme had the player remove, a placeholder not shipped).
replaces = ["btdata.ccx"]
```

The installer writes the empty placeholders it finds beside the content there, rather than
copying them; one kept in a mod's folder (installed by hand) is no archive the game cannot read
when it is named like one of the original game's: it blanks that one on purpose. TA: Enhanced is
the conversion that needs it: its whole game is in `rev31.gp3`, beside stubs of `totala1.hpi`
and `ccdata.ccx` and an empty `btdata.ccx`; layered over the original's, 125 of the original
game's units crept into its menus without their weapons (17 of the 52 errors its check
reported). A copy installed before is known by its id, so it needs no reinstalling.

Launch-critical errors (a missing or broken starting commander, unreadable side data) count
for every mod, since the game cannot start without them.

## Reading the badge

- **Data checks passed:** the completed checks found nothing that counts against the mod.
  Gameplay, multiplayer, balance and visual quality still need testing.
- **Issues found:** warnings count against it: things the game works round, or that show.
- **Errors found:** content the game cannot use, in the mod or caused by it. A card of a mod
  with errors carries a red lamp, and a check that finds errors in a mod that is on says so.
- **Nothing to check:** the mod holds nothing the data checks read (music, documents,
  pictures; WAV tracks in `music/` are checked as the WAVs they are).
- **Check incomplete:** the check could not cover every file (a resource limit, a missing
  dependency, a changed manifest). This is never presented as a pass.
- **Check failed:** the check itself could not run; run it again.
- **Not checked / queued / checking / recheck needed:** no current completed result.

The catalogue's curated project review is separate from the local check. Known rendering or
gameplay gaps stay visible even when the automated checks pass.

## The report

The report has three tabs.

**OVERVIEW** is what a player wants to know first:

- what the check found, in a word and a sentence (errors to put right, warnings to look at, no
  problems found...), with the errors, warnings and notes that count, each a way into
  **FINDINGS** showing only those;
- what the mod holds of its own (units, weapons, features, maps, sounds, files);
- **HOW TA REBORN PLAYS IT**: what the mod uses of TA's later patched engines, and of what TA
  left to its own engine, with how the game plays it (below);
- the findings to fix first (its gravest that count), and the way to all of them;
- what the project saw of the mod in play (its review, below), and which version was checked,
  when, with which TA Reborn and over what.

**FINDINGS** is what a mod's makers want. The filters at the top show what counts (**ERRORS**,
**WARNINGS**, **NOTES**, each with its count) and, when asked, what is **INHERITED**; the search
finds findings by their words, file, archive or code. Findings are grouped by what they concern
(packaging, files, units, weapons, features, maps, scripts, models and textures, animations and
effects, sounds, menus and sides). In a group, three or more findings of one check with one fix
are one card (**N OF ONE KIND**): the first three, **SHOW ALL** for the rest, and the fix they
share. Each finding gives its file, its line where it has one, the archive it came from, its
code, and **HOW TO FIX** a click away. Notes never count: they are worth knowing (a unit with no
build picture, a weapon with no muzzle in its script).

**PROJECT REVIEW**, for the catalogue's mods: what the project saw of the release in play.

### How TA Reborn plays it

Many mods were made for patched versions of TA's engine (TA 3.1, and the engines of TA:
Escalation and Total Mayhem), or brought programs and files of their own. The check notes what
a mod's own files use of them, and tells how TA Reborn plays each: **PLAYED** (as in the engine
the mod was made for) or **NOT USED** (what it did in TA is not there). These never count
against the mod. A report kept before the check told them has none: CHECK AGAIN.

| Code | What the mod has | How TA Reborn plays it |
| --- | --- | --- |
| `engine.ai_lists` | Build lists for the computer players, TA 3.1's `[CANBUILD]` in `gamedata/sidedata.tdf` | Played: the computer players build from them; build menus offer what the mod's menu pages and download lists put there, as TA's did |
| `engine.tall_panels` | Side panels laid out for a screen taller than TA's 480 pixels (Total Mayhem's and TA: Escalation's, 768) | Played: the classic interface lays them out as the mod does |
| `engine.shared_places` | Build buttons off TA's grid of 64-pixel places: several sharing a place (Total Mayhem's long-range guns, a button for each way they face), or one taking several (big buttons) | Played: a click builds the unit of the button under it; the modern interface shows a shared place as one picture, its parts marked |
| `engine.script_values` | Scripts asking TA 3.1's questions about units by their IDs (unit values 69 to 75), or a later patch's cloak (111) | Played: answered as those engines did |
| `engine.weapon_ids` | Weapon IDs past 255, the highest TA's own engine numbered | Played: TA Reborn finds a weapon by its name and never reads its ID |
| `engine.weapon_ids_shared` | Weapons sharing an ID with another weapon | Played, as above |
| `engine.sides` | Sides of its own, beside TA's Arm and Core | Played: offered when a skirmish or a multiplayer battle is set up |
| `engine.campaigns` | Campaigns of its own (`camps/*.tdf`) | Played from the campaign screen |
| `engine.ai_profiles` | Files for TA's own computer players (`ai/`) | Not used: TA Reborn's computer players are its own |
| `engine.programs` | Programs and engine patches (a patched `TotalA.exe`, DLLs) | Not used: TA Reborn runs no program a mod brings |

A script value TA Reborn does not answer is a finding of its own (`script.value`, below).

Findings and what to do about them are shown in the interface's language. The check keeps each
finding's words as an English text with placeholders and what fills them: names, which are
shown as they are, words of their own, and lists, which each language joins its own way
(`ta_data::words`). The launcher translates them as it shows them. `ta-tools i18n` collects the
texts marked `en`, and `enf`'s first argument, from the check's sources. The English words are
what a finding is compared by, and what the copied report and `ta-tools validate-mod` give, so
a report shared with a mod's author reads the same in any language. A file reader's own error
(a damaged archive's, a model's) is shown as the reader gives it, inside a translated sentence.

At the foot of OVERVIEW, the report gives the mod's own files, units, maps and sounds, then how
many files it was checked with (its own and those of what it plays over, most of them the
original game's).

## What is checked

- **Every file the game reads, decoded:** 3DO models, COB scripts (every instruction the game
  can run, from each function's start along its jumps, and its pieces, variables, functions
  and jumps; what no function reaches, such as the signature some compilers leave after a
  function's last instruction, is never run and not read), GAF animations, TNT maps, PCX pictures, fonts and
  palettes, sounds (as the game's decoder reads them: a WAV of a codec it plays, an Ogg file of
  a Vorbis or FLAC stream, Humongous's DIGI and TALK blocks holding their samples, or headerless
  8-bit samples saved as `.wav`, as TA's own `honk.wav`), and every text file
  (FBI/TDF/GUI/OTA) for its structure: a section never closed (it swallows what follows), a
  brace that closes nothing, a header without a body or a `]`, a comment never ended, each
  with its line.
- **References:** units to their models, scripts, weapons, explosions, wrecks, factions,
  movement classes and sound categories; weapons to their projectiles, explosion animations and
  sounds; features to their models, the animations the game draws (the animation, the burning
  one and the shadow; not TA's burn shadow, death and reclaim animations, which it never reads)
  and replacements; sound categories to their sounds, by the keys the game plays (`select1`,
  `ok2`… `count0` to `count9`; a repeated key's last value; not `load` and the like); build menus (`download/` menus, and the production pages the
  game reads: a builder's `guis/<name>1.gui`, 2, 3… to the first missing page; a builder being a
  unit with build power, `WorkerTime`, whatever its Builder flag, as the game has it: the mods'
  silos list missile buttons for their computer players, which build nothing) and the computer
  players' build lists (side data's CANBUILD, told as theirs: a unit only they name is on no
  menu, so no panel misses its picture); maps to their
  terrain (without it the map is left out of the lists: a warning), meteors, placed units and
  features. A file naming the same missing thing several times is told it once, with all that
  name it, and a model's missing textures are one finding. A unit's faction missing from the
  side data, and a menu list whose builder is missing, are notes: the game never acts on a
  unit's side, and skips such a list.
- **Duplicates:** a name defined twice in one content (the game keeps the later, in path
  order); a mod's definition the original game's comes after, so it is never used.
- **Scripts' operands:** a piece, axis, variable or function out of range, or a jump out of
  the code, is a warning: the game works round it (the instruction does nothing, the thread
  ends).
- **Scripts against models and duties:** pieces a script names that the model lacks; weapons
  with no Query function (shots from the unit's centre) and builders with no QueryNanoPiece
  (notes; not a building whose every option is an upgrade module of its own, built with no
  spray).
- **Scripts against the engine:** unit values a script asks for (`get`) or sets (`set`) that
  Reborn does not answer (`script.value`: they read 0, setting them does nothing), where the
  code names them outright. A mod made for a later patch of TA's engine shows here what it relies
  on that Reborn lacks. (`ta_data::validation::ANSWERED_VALUES`, held to the script host's.)
- **Packaging:** files where the game never reads them (an FBI outside `units/`, a model
  outside `objects3d/`…), archives in subfolders (only the top level's are opened), and a mod
  wrapped in a folder of its own (`My Mod/units/…`).
- **Limits:** footprints (a footprint of none is an upgrade module's, which takes no ground: kept), texture frames and layers (the textures the models in `objects3d`
  paint with, as the game loads them: an entry no model names takes no room, and the rest fit
  two arrays of the reference card's 2,048 layers), the sprite atlas's room for explosions.

Audio playback, external executables, arbitrary image formats and scripts' dynamic behaviour
are outside these checks. A passing report is useful evidence, not a substitute for a real
match and a look.

## Implementation

`ta_data::validation` supplies the report: files decoded in parallel a chunk at a time
(names and references kept, never pixels or bytecode), folded in path order so a report is
the same on any computer, references resolved, then each finding's origin settled
(`finish_scope`) and compared with the baseline (`validation::compare`). The launcher's
`validation.rs` schedules checks one at a time off the interface's thread, resolves
dependencies, fingerprints inputs and keeps the reports under `cache/mod-validation`:

- the fingerprint covers only the files the game reads (archives and the kinds the checks
  decode) and those whose presence the report notes (programs and their libraries, files in
  `ai/`: `validation::noted_file`), with their digests kept between launches by size and times
  (`digests.json`), and what each mod replaces of the original game; an explicit rescan reads
  every file again, and CHECK AGAIN reads its mod's;
- the checks' version, not the launcher's build, keys the cache, so a rebuild keeps reports;
- baselines are kept by their own fingerprint (`baseline-*.json`, the newest eight);
- after an install or a removal the changed mods are checked again, and every mod depending
  on one, however indirectly;
- on a visit to the Mods page (at most every ten seconds), every mod shown as checked has its
  fingerprint taken again in the background, from the kept digests, so a file edited by hand in
  one of a mod's folders, which changes neither the folder nor its manifest, brings a recheck.
  The badge stays as it is unless the files differ.

A mod that is one archive the game cannot open, or a folder whose content is all in such
archives, is an error of its own, not an incomplete check. Archive problems of the original game
or a dependency are inherited. A definition the mod replaces (a weapon of the same name in a
file of its own) is not the mod's: what was wrong with the replaced one stays inherited. Reports
never carry this computer's folders.
