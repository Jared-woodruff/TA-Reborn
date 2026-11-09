# Translating TA Reborn's interface

TA Reborn is an unofficial, fan-made reimplementation of Total Annihilation, Cavedog's 1997
real-time strategy game (two factions, the Arm and the Core, fight the Galactic War across
many worlds; everything is built from metal and energy; the Commander is each side's most
important unit). It plays on the original game's data. Its interface text is English, in a
plain, terse, slightly British voice ("colour", "centre"), like the original game's own
screens. You are translating that interface into ONE language.

## What to produce

The English strings to translate are in `scratch/i18n/chunks/`: `chunk_NN.toml` files of
about 140 strings each (a whole new language), or `topup.toml` (the strings added since a
language was translated). `#` comments name the source file a group comes from, which tells
you where the text appears; read that source for context when a string is unclear. For each
of those files write one of the same name in `scratch/i18n/<code>/`, with exactly the same
keys and your translations as the values:

```toml
# apps/ta_game/src/menu.rs
"SKIRMISH" = "GEFECHT"
"{done} of {total} done" = "{done} von {total} erledigt"
```

* Copy each key exactly as it is in the chunk (byte for byte, escapes included). Only the
  value after `=` is yours.
* Values are TOML basic strings: write `\"` for a quote and `\\` for a backslash; keep any
  `\n` from the English as `\n`. Everything else (accents, CJK, Cyrillic) is written as is.
* Every key gets a non-empty value. When the best translation is the English itself (a
  symbol, a number format, a proper name), repeat it.
* Your language's glossary is `tools/i18n/glossary/<code>.md`: read it first when it exists,
  stay consistent with it and with `lang/<code>.toml` (the translations so far: grep it for
  any term), and add the terms you choose.
* Write each chunk file in one go (a Write of the whole file), then move on.
* When all are written run, from the repository root:
  `python tools/i18n/merge.py <code> "<language name>"`
  It writes `lang/<code>.toml` (the translations so far with yours over them) and lists
  anything MISSING (keys left out) and every PROBLEM (placeholders that differ, keys not in
  the template, invalid TOML). Fix your chunk files and run it again until it reports no
  MISSING and no PROBLEM lines (exit code 0).
* Do not edit any other file of the repository than your chunks, the glossary and (through
  merge.py) `lang/<code>.toml`.

## Rules

* `{placeholders}` stay exactly as they are (same name inside the braces, never translated);
  you may move them to where your language needs them.
* Do not translate: Total Annihilation, TA, TA Reborn, Cavedog, GOG, ModDB, Windows,
  Bahnschrift, Steam, file extensions (.tarep, .tasav, .toml, .ufo...), chat commands that
  start with `+` (+atm, +cdplay...), environment variable names, URLs, keyboard key names
  (Ctrl, Shift, Alt, Enter, Esc, Tab, Space, Backspace, Delete, F1-F12, PageUp...), and the
  faction names Arm and Core (ARM and CORE in capitals). Unit and map names come from the
  game data in English: where the tutorial or a briefing names one, keep it as it is.
* Capitals: strings in CAPITALS are buttons, captions and titles; write them in capitals
  too (scripts without case, like Chinese, Japanese and Korean, just translate). Turkish:
  i becomes İ in capitals. German: ß becomes SS in capitals.
* Length matters: buttons, captions and labels have little room. Keep a translation of a
  short string (up to about three words) within roughly 1.3 times the English length; use
  the shorter of two good options, an established abbreviation if needed. Sentences
  (tooltips, help, briefings, tutorial steps) may be as long as natural.
* No plural forms exist: one string serves every number. Where a count meets a noun, word
  it so it reads for any number (e.g. "Units: {n}", or the noun in a form that suits all
  counts; Chinese and Japanese counters are fine).
* Keep the symbols: the `·` separators, `×`, `–`, `…`, `→`, `%`, `/`. Chinese and Japanese
  use full-width punctuation in sentences where natural (，。：！？), but keep `·` and `×`.
* Voice: terse and matter-of-fact, like a military game interface; don't add exclamation
  marks the English lacks. Use the register your language's games normally use for
  addressing the player (German: du; French: vous; Spanish: tú; Japanese: plain polite
  です/ます in sentences, nouns for labels).
* Where the text appears (the `#` comment above it):
  * `chatter.rs`: the computer players' chat, taunting banter between commanders: translate
    it idiomatically, keep the attitude and the jokes' intent, not word for word.
  * `timeline.rs`, `briefing.rs`, `credits.rs`: lore, in a serious, slightly epic tone.
    Real people's names in the credits stay as they are; translate only the headings and
    sentences.
  * `controls.rs`: the controls reference. In its key column ` / ` separates alternative
    keys; keep ` / `, the `{placeholders}` and key names, translate the mouse's words
    ("Click", "drag", "Mouse wheel", "screen edges").
  * `hud_classic.rs`: TA's own 1997 side panel; its button words are tiny (e.g. "SELF-D" is
    the short form of "SELF-DESTRUCT" for when the long one does not fit): keep them as
    short as you can.
  * `apps/ta_game/scenarios/*.toml`: TA Reborn's own scenarios: a title, a one-line tagline
    and a briefing (a commander's orders, terse and tense); `\n\n` separates the briefing's
    paragraphs: keep them.
  * `ta_launcher`: the launcher program; `settings.rs`: settings names and their help.
* Game terms, kept consistent everywhere (choose your language's usual RTS terms):
  Commander, metal, energy, unit, building, factory, constructor (builder), kbot, vehicle,
  aircraft, hovercraft, ship, submarine, wreck, reclaim, repair, guard, patrol, attack,
  fight (attack-move), hold fire / return fire / fire at will, hold position / maneuver /
  roam, self-destruct, cloak, radar, sonar, jammer, line of sight, fog of war, mapped /
  unmapped, skirmish, campaign, mission, briefing, scenario, conquest, multiplayer, lobby,
  host / join, replay, highlights, saved game, autosave, map editor, mod, archives,
  encyclopaedia, timeline, credits, jukebox, launcher, settings, key bindings, nuke /
  anti-nuke, stockpile, transport (load / unload), queue, squad (group), ally / enemy /
  team, victory / defeat, computer player (AI) with difficulties Easy / Normal / Hard,
  D-gun (the Commander's disintegrator gun), the Galactic War.
