# Supreme Commander music

TA Reborn can make the soundtrack of **Supreme Commander: Forged Alliance** into its own music, to
play in its menus and battles beside Total Annihilation's or in its place. Like the maps
([SUPCOM_MAPS.md](SUPCOM_MAPS.md)) and the factions ([SUPCOM_UNITS.md](SUPCOM_UNITS.md)), the
music is made on the player's computer from their own copy of the game (GOG's or Steam's), into a
mod in their mods folder. Nothing of Supreme Commander's is shipped with TA Reborn or kept in this
repository.

- **Players:**
  - **Making it.** The Supreme Commander dialog (SUPREME COMMANDER… on the launcher's Maps or Mods
    page) has a MUSIC tab. It finds the game as the other tabs do and lists the soundtrack's
    tracks by their parts, each with how long it plays.
    - IMPORT MUSIC makes the whole soundtrack: on GOG's Gold Edition, 21 tracks, 44 minutes,
      226 MB. That took 3.4 s on the computer it was measured on.
    - The result is the mod **Supreme Commander music** (`supcom-music`), turned on once made.
    - STOP ends the import at the next track and installs nothing.
    - As with the factions, the mod is made whole each time and replaces the one installed only
      once it is made. It is made again where the launcher finds it, and in place of another mod
      of its id only once the player agrees.
    - A mod made by an older converter (its `version` below `soundtrack::VERSION`) is marked
      UPDATE on the Mods page.
  - **Playing it.** In the game, Options, SOUND: SOUNDTRACK chooses whose music plays:
    - **TA**: Total Annihilation's, as before (the default).
    - **FORGED ALLIANCE**: Forged Alliance's in TA's place: its building themes while a battle is
      at peace, its battle themes in battle, and its menu theme and briefings' music in the menus.
    - **BOTH**: the two together, a playlist each mood.
    - **BY SIDE**: Forged Alliance's while the battle's player commands one of its factions (UEF,
      CYBRAN, AEON or SERAPHIM: the factions' mod), TA's otherwise.

    TA's victory and defeat music stays with every choice. A change takes effect at once: a track
    playing that the new playlist doesn't have is skipped. Without the mod installed and on,
    SOUNDTRACK is greyed, with a note on where the music comes from, and TA's plays whatever it
    says.
  - **The jukebox** lists the soundtrack in a section of its own, SUPREME COMMANDER, every track
    by its part (Menu theme, Building theme 1-5, Battle theme 1-4, Briefing 1-6, and the films'
    Intro, Outro, Epilogue, Timeline and Credits).
  - **Online**, the other players need not have it: the mod adds no battle data (see
    Multiplayer).
- **Developers:** `cargo build --release -p ta_tools`, then `target/release/ta-tools supcom-music
  <out> [--game <folder>]`. It makes the mod into `<out>` (mount it as a mod to play it) and
  prints each track with its part, length, bank and wave.
  - The launcher's dialog for captures: `--page mods --show supcom-music` (`supcom-music:all`
    imports the soundtrack once it is listed; `TA_SUPCOM_SHOT_IMPORTING=1` takes the picture
    while it is made).
  - `TA_SUPCOM_MISSING=1` has the launcher find no copy of the game, for the dialog's own words
    when there is none.

The code:
- `crates/ta_supcom/src/xact.rs`: the XACT banks.
- `crates/ta_supcom/src/soundtrack.rs`: the tracks, and the mod made from them.
- `apps/ta_launcher/src/supcom_music.rs` and `pages/supcom.rs`: the tab.
- `apps/ta_game/src/audio.rs`: SOUNDTRACK's playlists.
- `apps/ta_game/src/jukebox.rs`: the jukebox's section.
- `crates/ta_audio`: the music folders listed, `music/` and `music/supcom/`.

## What the game holds

The soundtrack is in the game's `sounds` folder, as XACT 3 banks: a wave bank (`.xwb`, the
sound) and a sound bank (`.xsb`, the cues the game plays by name) for each part. The converter
reads three pairs:

| Banks | Cues | Part | Tracks (Gold Edition) |
|---|---|---|---|
| `Music` | `Main_Menu` | menu | 1 |
| | `Base_Building` (one of several) | building | 5 |
| | `Battle` (one of several) | battle | 4 |
| `Op_Briefing` | one a campaign operation, by name | briefing | 6 |
| `FMV_BG` | `X_FMV_Intro`, `X_FMV_Outro`, `X_FMV_Post_Outro`, `X_Timeline`, `Menu_Credits` | film | 5 |

Within `Base_Building` and `Battle`, the tracks are numbered in the order the cue lists its
waves. The briefings are numbered in their cues' names' order. `FMV_BG`'s other cues, the
publishers' jingles, are left out. On the copies looked at, every wave of these banks is
uncompressed 16-bit PCM at 44.1 kHz, in stereo. Waves of the other codecs XACT knows (XMA,
ADPCM, xWMA) would be left out of the list rather than misread.

### Wave banks (`WBND`, versions 43 to 46)

- **Header.** `WBND`, then its version (u32, 43 on Forged Alliance's), a header version (u32),
  and five segments, each an offset and a length (u32, u32) from byte 12:
  1. the bank's own data;
  2. its entries;
  3. seek tables;
  4. names;
  5. the waves' data.
- **Bank data.** Flags (u32: 0x00020000 means compact 4-byte entries, which these banks don't
  use), the wave count (u32), the bank's name (64 bytes, nul-padded), then from +72 the entry
  size (u32, 24 here).
- **Each entry (24 bytes).** Its duration (u32: flags in the low 4 bits, frames above), its
  format (u32), and its data's offset and length within the data segment (u32, u32).
- **The format, bit by bit:**

  | Bits | Field |
  |---|---|
  | 0-1 | codec: 0 PCM, 1 XMA, 2 ADPCM, 3 xWMA |
  | 2-4 | channels |
  | 5-22 | sample rate |
  | 23-30 | block alignment |
  | 31 | 16 bits a sample (else 8) |

- PCM data is little-endian samples with the channels interleaved: 8-bit samples unsigned,
  16-bit signed.

The track list reads only the first 64 KiB of each wave bank (its header and entries). The
import reads each bank's file once, for all of its tracks.

### Sound banks (`SDBK`, version 43)

- **Header.** `SDBK`, then from byte 6 its version (u16, 43). Then, at the offsets below:

  | Offset | Size | Field |
  |---|---|---|
  | 0x13 | u16 | simple cue count |
  | 0x15 | u16 | complex cue count |
  | 0x1B | u8 | wave bank count |
  | 0x1E | u16 | name table's length |
  | 0x22 | u32 | simple cues' offset |
  | 0x26 | u32 | complex cues' offset |
  | 0x2A | u32 | name table's offset (0xFFFFFFFF: none) |
  | 0x3A | u32 | wave banks' names' offset (64 bytes each) |
  | 0x4A | 64 bytes | the bank's name |

- **Cue names** are the name table's nul-separated strings: the simple cues' first, then the
  complex cues'.
- **A simple cue** is 5 bytes: flags (u8), then its sound's offset (u32).
- **A complex cue** is 15 bytes: flags (u8), then, where flag 0x04 is set, its sound's offset
  (u32). Cues of variations among sounds are not used by the soundtrack and are left out.
- **A sound:**
  - flags (u8); the wave it plays, or its clips, from +9.
  - Without flag 0x01, it plays one wave: its index (u16) at +9 and its wave bank (u8) at +11.
  - With flag 0x01, a clip count (u8) follows, then, where any of flags 0x0E is set, its
    effects' settings, skipped by their length (u16). Then each clip is 5 bytes, holding its
    events' offset (u32) at +1.
- **A clip's events:** an event count (u8), then each event's kind in the low 5 bits of a u32,
  with its data 6 bytes on:
  - **Kind 1 plays a wave:** the wave's index (u16) at +2, its bank (u8) at +4, its loop count
    (u8) at +5 (255: on until stopped), 10 bytes in all.
  - **Kind 3 plays one of several:** its loop count (u8) at +2, then 7 bytes on a count (u16),
    then 8 bytes on that many 5-byte entries, each a wave's index (u16) and bank (u8).
  - The soundtrack's cues use no other kind before these; reading a clip stops at any other.

## The mod

- **`mod.toml`:** id `supcom-music`, name "Supreme Commander music", `version` the converter's
  (`soundtrack::VERSION`, 1), `requires_reborn` the TA Reborn that made it, a description, and
  the tags `music` and `supreme commander`. The launcher shows the name and description in the
  language in use.
- **`music/supcom/<stem>.flac`:** a track each, its wave's PCM as FLAC (lossless; a track
  decoded again is the same samples). The stems:
  - `menu`;
  - `building_1`…`building_5` and `battle_1`…`battle_4` (a cue of one wave gives its part's
    name alone);
  - `briefing_1`…`briefing_6`;
  - `film_intro`, `film_outro`, `film_epilogue`, `film_timeline`, `film_credits`.
- **`music/supcom/soundtrack.toml`:** the list the game reads (`soundtrack::LIST`): a `[[track]]`
  each, with its `file`, `part` (`menu`, `building`, `battle`, `briefing` or `film`), `number`
  in its part, and `seconds`.
- **Encoding.** The FLAC files are encoded by `flacenc` (Apache-2.0, pure Rust, on threads), at
  its default block size. Its threaded encoder writes the last block's length as the stream's
  least block size. A decoder then reads the blocks as variable-sized, and symphonia, the
  game's decoder, stops at the first frame as if the stream had ended. `soundtrack::flac` sets
  both sizes to the block size, as FLAC has it: the last block is left out of the least size.
- **Reproducible.** The files are the same, byte for byte, from one run to the next.

## Multiplayer

The mod gives a battle nothing that the multiplayer check compares: only the `ai`, `camps`,
`download`, `features`, `gamedata`, `guis`, `maps`, `objects3d`, `scripts`, `units` and
`weapons` folders are (`ta_data::compatibility`).
- Its `NetMod::data` is false: a host neither asks for it nor is refused over it, and USE THE
  HOST'S MODS AND RESTART keeps it on.
- Each player hears their own SOUNDTRACK.

## Tests

- **`ta_supcom`:**
  - `xact::tests` read banks written in the tests: a wave bank's PCM waves (rates, channels,
    lengths) and their data, and a sound bank's complex cues, of clips playing one wave (looped)
    or one of several.
  - `soundtrack::tests::the_soundtrack_is_its_cues_tracks` lists a copy made of synthetic banks.
  - `soundtrack::tests::the_music_mod_is_made_a_flac_file_a_track` checks the FLAC files'
    stream information, their block sizes among it.
  - `soundtrack::tests::parts_go_by_their_keys`.
- **`ta_game`:**
  - `audio::tests::the_soundtrack_setting_makes_the_playlists` covers each choice, by side on
    either side, and nothing installed.
  - `audio::tests::the_installed_soundtrack_is_read_from_its_list`.
  - The jukebox's names: `jukebox::tests`.
- **`ta_launcher`:**
  - `supcom_music::tests` (the words, the outcome told, the tracks' names).
  - `pages::supcom::tests::the_music_tab_lists_the_soundtrack_by_its_parts_and_imports_it_whole`.
  - The dialog's tab tests over all three tabs, `supcom::tests` (the UPDATE mark, the tab an
    import under way opens at), and `app::tests::an_update_waits_for_a_supreme_commander_import`.
