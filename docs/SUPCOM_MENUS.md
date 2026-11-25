# Supreme Commander menus

TA Reborn can dress its menus in **Supreme Commander: Forged Alliance**'s look: its interface's
colours, its menus' pictures, its panels' border art, its interface sounds and its menu music.
Like the maps ([SUPCOM_MAPS.md](SUPCOM_MAPS.md)), the factions ([SUPCOM_UNITS.md](SUPCOM_UNITS.md))
and the music ([SUPCOM_MUSIC.md](SUPCOM_MUSIC.md)), the look is made on the player's computer from
their own copy of the game (GOG's or Steam's), into a mod in their mods folder. Nothing of Supreme
Commander's is shipped with TA Reborn or kept in this repository. The menus keep their layouts and
their widgets' shapes: only how they are coloured, framed, backed and heard changes.

- **Players:**
  - **Making it.** The Supreme Commander dialog (SUPREME COMMANDER… on the launcher's Maps or Mods
    page) has a MENUS tab. It finds the game as the other tabs do and shows what the copy has of
    the look: its colours, its pictures, its panel art, how many interface sounds, and its menu
    music. IMPORT MENUS makes the mod **Supreme Commander menus** (`supcom-menus`), turned on once
    made (on GOG's Gold Edition, 12 pictures, the panel art, 11 sounds and the 81-second menu
    theme: about 19 MB, in a fraction of a second). STOP installs nothing. As with the other
    imports, the mod is made whole each time, replaces the one installed only once made, and is
    marked UPDATE on the Mods page when an older converter made it.
  - **Choosing it.** In the game, Options, INTERFACE: MENU THEME, TA REBORN (the default) or FORGED
    ALLIANCE. Without the mod installed and on, the row is greyed, with a note on where the look
    comes from. With FORGED ALLIANCE:
    - the menus' colours are Forged Alliance's: its pale cyan lettering, its teal labels, its
      slate panels (see Colours below);
    - its pictures show behind the menus one after another, each for about 24 seconds, growing
      slowly (still with reduced motion), darker at the left where the menu stands, in place of
      the MENU BACKDROP (which is greyed meanwhile);
    - the panels and strips are framed in its panel border art in place of TA Reborn's pipes;
    - its interface sounds play for the menus' clicks: its big buttons', small buttons', tabs',
      the way back, errors;
    - its menu theme is the menus' music, whatever SOUNDTRACK plays in battle.
    The battle keeps its own look (its HUD, its pause menu and dialogs); a unit on show in the
    units' viewer, and the credits' film, show over the battlefield as they do.
- **Developers:** `cargo build --release -p ta_tools`, then `target/release/ta-tools supcom-menus
  <out> [--game <folder>]` makes the mod into `<out>` (mount it as a mod) and prints the palette
  it made of the game's colours. The launcher's dialog for captures: `--page mods --show
  supcom-menus` (`supcom-menus:all` imports it once listed). In the game, with the mod mounted
  and `[gameplay] menu_theme = "ForgedAlliance"` in a scratch profile's settings.

The code:
- `crates/ta_supcom/src/menus.rs`: what the copy has, the palette made of its colours, the mod.
- `apps/ta_launcher/src/supcom_menus.rs` and `pages/supcom.rs`: the tab.
- `apps/ta_game/src/menu_theme.rs`: the theme read, the colours' grade, the pictures.
- `crates/ta_ui/src/panel.rs`: the panel art (`PanelArt`, `set_art`).
- `apps/ta_game/src/audio.rs`: the interface sounds and the menus' music.

## What the game holds

`ta_supcom::menus` reads, from the copy's archives (`gamedata/*.scd`) and its `sounds` folder:

| What | Where | Used as |
|---|---|---|
| The colours | `lua/skins/skins.lua`, the `default` skin's `...Color` fields (`AARRGGBB`) | the palette (below) |
| The pictures | `textures/ui/common/menus02/background*_bmp.dds` (five of space, five paintings, the Colossus), `scx_menu/campaign-select/bg.dds` (the factions' hall) | behind the menus, one after another |
| The panel art | `textures/ui/common/scx_menu/panel-brd/panel_brd_*.dds`, nine pieces | round the panels and strips |
| The sounds | `sounds/Interface.xwb` and `.xsb`: the cues of the menus' clicks (PCM, 32 kHz) | TA Reborn's interface events |
| The menu music | `sounds/Music.xwb` and `.xsb`: `Main_Menu` | the menus' playlist |

The skin's other factions (`uef`, `aeon`, `cybran`, `seraphim`) colour Forged Alliance's own
in-battle screens; the menus take the `default` skin's. Forged Alliance's main menu itself plays a
movie (`movies/main_menu.sfd`), which TA Reborn does not decode: the pictures stand in for it.

## Colours

The skin's colours become TA Reborn's palette as a mod's interface theme's do
(`ta_ui::theme::Palette::themed`, which makes the shades from the few given):

| Palette key | From the skin |
|---|---|
| `text`, `button_text` | `fontColor` (a pale cyan) |
| `text_bright` | `fontOverColor` (white) |
| `dim` | `bodyColor` (a teal) |
| `accent` | half way between `highlightColor` and `fontColor` |
| `panel` | `panelColor` at `transparentPanelColor`'s alpha over black (a dark slate) |
| `line` | `disabledColor` |
| `good` | `dialogButtonColor` (a green) |
| `info` | `tooltipBorderColor`, else `highlightColor` |

In the game the menus are not drawn anew in these colours: what TA Reborn draws them in is moved
onto them after the frame is laid out (`menu_theme::Grade`, on the frame's meshes drawn in egui's
own texture: shapes and text). TA Reborn's palette and its metal's colours (its gunmetal, slots,
pipes and brass, re-hued at their own lightness) are the anchors; any other colour moves as the
anchors nearest it do, weighted by the inverse square of the distance and less the further it is
from the nearest (none past 110 a channel). The players' colours are kept exactly, as are black
and white; pictures, map previews and portraits, drawn in textures of their own, are untouched.

## The mod

- **`mod.toml`:** id `supcom-menus`, name "Supreme Commander menus", `version` the converter's
  (`menus::VERSION`, 1), `requires_reborn`, a description, the tags `interface`, `theme` and
  `supreme commander`.
- **`menus/supcom/pictures/<name>.dds`:** the pictures as the game has them (`space_1`…`space_5`,
  `painting_1`…`painting_5`, `colossus`, `factions`).
- **`menus/supcom/panel/<place>.dds`:** the panel art's nine pieces, named for their places.
- **`sounds/supcom-menus/<cue>.wav`:** each sound once, as WAV.
- **`music/supcom-menus/menu.flac`:** the menu music, lossless.
- **`menus/supcom/theme.toml`:** what each is for, as the game reads it: `pictures`, `music`,
  `[colours]` (the palette's keys), `[panel]` (the nine places) and `[sounds]` (TA's interface
  events: `BIGBUTTON`, `SMLBUTTON`, `SMALLBUTTON`, `PREVIOUS`, `MORE`, `SKIRMISH`, `MULTI`,
  `OPTIONS`, `EXIT`, `MessageArrived`, `ENDGAMESCORE`, `SLIDER`, `PANEL`, `SKIRMISHCHEAT`,
  `ENDGAMESTATBAR`, each a sound's file).

The panel art's top pieces carry a title bar of Forged Alliance's own; the panels lay their titles
out otherwise, so the frame takes the bottom corners and edge, turned upside down, for its top
too. The mod gives battles no data: online, the other players need not have it.

## Tests

- **`ta_supcom`:** `menus::tests`: the skin's colours read and the palette made of them, what a
  copy has listed, the mod made (its theme, its WAVs, each cue once) and stopped, a wave as WAV;
  on a copy written in the tests.
- **`ta_game`:** `menu_theme::tests`: a theme's list read; the grade (TA Reborn's colours onto the
  theme's exactly, the players' colours kept, a colour between two moved between theirs, a far
  colour kept, alpha kept); a picture that does not read passed over, not decoded again; a theme
  none of whose pictures reads showing none (the battlefield behind the menus again).
- **`ta_launcher`:** `supcom_menus::tests` (the words, the outcome told, a picture made small) and
  `pages::supcom::tests::the_menus_tab_shows_the_look_and_imports_it_whole`; the dialog's tests over
  all four tabs.
