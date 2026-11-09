# Unit skins

Open **Skins** in the launcher (**Ctrl+4**): every skin is a card with your units wearing it.
Choose one to see it turning beside the cards (drag across it to turn it yourself, double-click
to bring it back to the front; the tabs over it show it on **tanks**, **kbots**, the
**Commanders**, **aircraft** or **buildings**), then **Use for your units** or **Use for
enemies**. Your choice also covers allies; enemy skins are
relative to your player, including captured units and alliance changes. Replays follow
the player being watched; an unrestricted view and benchmarks use player 1 as the
reference. **Use the same skin for both sides** links the enemy choice to yours. Older
settings keep their previous appearance on both sides until you choose separately.

Twelve skins are included. **Reborn** is TA Reborn's own: TA's designs rebuilt in detail
(armour plates, bolts and road wheels), their paint redrawn crisp and clean, TA's mottled
camouflage and noise calmed into a factory finish. The others are liveries (see below), paint
laid over every unit:

- **Heritage**: TA as it was: its own shapes and paint as TA drew them (no plates, bolts or road
  wheels; the textures' crisp texels with their painted light and camouflage), battle-worn,
  chipped, grimy and a little sun-faded.
- **Christmas**: evergreen and festive red with snowflakes and gold trim.
- **Halloween**: witching purple and black splinters, trimmed in pumpkin orange.
- **Desert**, **Winter** and **Woodlands**: camouflage blotches of sand and earth, of worn
  whitewash over grey, of hard-edged green, brown and black.
- **Navy**: splinters of haze and ocean grey, as a warship's dazzle paint.
- **Urban**: pixelated greys. **Midnight**: dark gunmetal hex plating. **Tiger**: burnt orange
  slashed with black. **Ceremonial**: parade white with polished gold edges.

Each built-in can be exported as an editable starter and shared like a custom pack.

Selections take effect the next time the game or benchmark starts. Benchmarks keep
both choices through every scene and graphics level, and record both skins actually
rendered. Compare scores using the same pair; custom paint can change the rendering
workload. Older reports used Reborn, or one recorded skin for both sides.

Skins change the local appearance of units in both art styles. Detached pieces and dying
units retain their source skin; independent wreck features keep their original materials.
Skins retain unit geometry,
animations, player colours, shadows and gameplay. Other multiplayer participants can
choose different skins; skin files do not enter the match's compatibility fingerprint.

## Make your own

Choose **New skin**, or **Customize** on a built-in to start from a copy of it. The editor shows
two units wearing the skin, turning; choose any Arm and Core unit to look at. Every change shows
at once:

- **Paint**: switch the livery on or off. Choose its pattern from tiles drawn in your colours.
  Set up to three colours and a trim: click a swatch for a colour picker, a hex code and the
  palettes' colours, or take a whole palette with one click. **Pattern size** and **Sharpness**
  shape the pattern; **Shuffle** lays it out another way.
- **Finish and wear**: coverage, detail, gloss, metal, wear and grime (see Liveries below).
- **Texture slots**: put a picture of your own on the units' armour: every armour surface,
  armour painted in a plain colour, or one of the textures the shown units use, for every unit
  or for one of them. PNG, JPEG, BMP, TGA and WebP pictures all work; the editor makes them
  square, a power of two and opaque. Clear a slot to show the livery there again.
- **Undo** and **Redo** (Ctrl+Z, Ctrl+Y) step back and forth; **Revert** goes back to the skin as
  last saved. **Save** (Ctrl+S) keeps it in the settings folder's `skins/`; **Save as…** keeps a
  copy under another name. Leaving the editor with changes not saved asks first.

Your own skins have **Edit**, **Export…** and **Delete** (to the Recycle Bin) in the library.
A saved skin's folder also holds `UNIT-TEXTURES.toml`, the unit and texture names of your game
and enabled mods, for editing `skin.toml` by hand (see Manifest). Saving in the editor changes
only `skin.toml` and the pictures it adds, so working files kept in the folder stay as they are.

## Share

Choose **Export…** to make one `.taskin` file. Share that file; another player uses **Import**
or drops it onto the launcher. Importing adds the skin without selecting it.

Export includes only the manifest and its referenced PNGs. Working documents, the unit
reference and unrelated files stay in the author folder. Existing skins and export files
are never overwritten. For a new edition, change the manifest's `id` and containing folder
to a new matching name; export to a new filename. Built-in exports create editable
copies such as `desert-copy` or `christmas-copy`.

The settings folder is `%APPDATA%\TA Reborn` on Windows, or the application's configuration
folder on Linux. To create a unit reference from a development build:

```text
ta-tools skin-catalogue UNIT-TEXTURES.toml
ta-tools --mod <mod-folder> skin-catalogue MOD-UNIT-TEXTURES.toml
```

The destination must not already exist. These guides list identifiers and display names,
not textures, models or scripts.

## Manifest

```toml
format = 2
id = "winter-armour"
name = "Winter Armour"
author = "Your name"
description = "Painted ceramic armour with copper trim."

# Paint over every unit's eligible surfaces (see Liveries). Leave it out for Reborn's own.
[livery]
colours = ["#e4e7e6", "#a6adb0"]
trim = "#b87333"
pattern = "blotch"

# Global mappings apply to eligible surfaces on every unit using that source texture.
[textures]
"*" = "textures/armour.png"

# Per-unit mappings take precedence over the global mappings.
[units.armcom.textures]
"*" = "textures/commander.png"
"_untextured" = "textures/commander.png"
```

Unit and texture identifiers are case-insensitive. The renderer resolves an exact unit
texture mapping, then that unit's `*`, then an exact global mapping, then the global `*`.
For example, replace `*` with an identifier listed in the reference to repaint only that
material. `_untextured` addresses eligible armour faces painted with a plain palette
colour; `*` also covers those faces. Mapped textures paint over the livery; unmapped surfaces
keep it.

The `id` must match the containing folder and use 1–64 lowercase ASCII letters, digits or
hyphens, beginning with a letter or digit. `reborn` and `christmas` are reserved. The other
built-ins use a separate `builtin:` namespace so they cannot replace an existing custom pack
with the same name. Names
may contain Unicode. PNG paths use `/` and stay inside the pack folder; avoid paths that
differ only by letter case. Unknown manifest fields are rejected to catch spelling errors.

## Painting and quality

- Use opaque, square PNGs: 64, 128, 256, 512 or 1024 pixels per side. Alpha must be fully
  opaque, including pixels along the edges.
- Existing polygon texture coordinates are retained. Each source texture is a material
  tile, rather than one unique full-body UV sheet. Keep important details away from tile
  edges and inspect several differently shaped faces using that texture.
- Team-colour textures, animated textures, lights, tracks and protected functional
  markings retain their original material. Dark cavities and effect geometry are also
  protected. These restrictions apply to exact mappings as well as wildcards.
- Paint is sampled in the main view, water reflections and ray-traced reflections. The
  renderer builds mip levels in linear light to keep distant patterns stable. Existing
  lighting and material finish remain; skin PNGs supply the visible colour.
- Reuse PNGs across mappings where possible. A pack's images share a GPU array sized to
  its largest image, so mixing one large image with many small images still costs memory.
- Test both art styles, daylight/night, team colours, damage and construction, and moving
  cameras at several distances before sharing.

## Liveries

A livery is paint laid over the units' own, in a few numbers rather than images: it costs
no texture memory, keeps every unit's normal maps, panels and bolts, and is fixed to each
of a unit's pieces, so it moves and turns with them. It paints the same armour as `*`
(never team colours, lights, tracks or markings), and a mapped texture replaces it where
it is mapped.

| Field | Values | What it does |
| --- | --- | --- |
| `colours` (or `colors`) | one to three `"#rrggbb"` | The ground colour, then the pattern's. |
| `trim` | `"#rrggbb"`, optional | A band of this colour along the convex edges. |
| `pattern` | `solid`, `blotch`, `splinter`, `digital`, `stripes`, `hex`, `festive` | How the colours are laid out: patches, angular shards, square pixels, bands, hexagonal plates, or Christmas waves with snowflakes of the third colour. |
| `scale` | 0.25 to 4 (1) | The pattern's size; at 1, patches about a third of a light tank. |
| `sharpness` | 0 to 1 (0.7) | Soft to crisp pattern edges. |
| `seed` | a whole number (0) | Another layout of the same pattern. |
| `coverage` | 0 to 1 (1) | How much of the units' own paint it covers. At 0 they keep their colours and only the wear, grime and fade below apply (Heritage). |
| `detail` | 0 to 1 (0.8) | How much of the paint's own light and dark (panel lines, seams, markings) shows through. |
| `gloss` | 0 to 1 (0.35) | Matte to glossy. |
| `metal` | 0 to 1 (0) | Paint to bare metal. |
| `wear` | 0 to 1 (0.5) | Factory fresh to battle-worn; 0.5 is Reborn's own wear, and past it the paint chips along the edges to the steel. |
| `grime` | 0 to 1 (0) | Dirt in the creases, on what faces down, and in streaks. |
| `fade` | 0 to 1 (0) | Sun-bleached paint: its colours washed out towards a dusty grey, in patches, and its finish chalky. |
| `original` | `true` or `false` (`false`) | Painted on TA's originals: TA's own shapes (no plates, bolts or road wheels, at any size) and paint as TA drew it (its textures' crisp texels and painted light) rather than TA Reborn's rebuilt units. The editor's **Units** choice; the same in Enhanced Classic, which shows TA's anyway. |

Seen from afar, patterns blend into their average colour rather than shimmer.

Format 1 packs still load: `base = "christmas"` and `base = "heritage"` become the Christmas
and Heritage liveries, and Heritage's `_heritage_detail` control image, no longer used, is
left out. Packs exported now are format 2, which versions before liveries reject rather than
show differently.

## Limits and recovery

A pack may reference up to 512 unique PNGs, with at most 256 MiB of decoded pixels. The
GPU array, including its fallback layer, must satisfy:

```text
(number of unique images + 1) × largest image side² ≤ 67,108,864 texels
```

The base array is at most 256 MiB; mip levels add approximately one third. A graphics
card may impose a lower layer or dimension limit. Images load once at startup; there are
no skin-file reads or extra unit draw calls per frame. The default Reborn skin allocates
only a tiny fallback texture for the additional binding. Matching side selections share
one texture bank and model cache. Different selections may need two banks and material
variants; these are prepared once at launch, not while a battle runs.

Individual PNG files are limited to 8 MiB, the UTF-8 manifest to 1 MiB, and the share file
to 272 MiB. The importer rejects unsupported versions, unsafe paths, duplicate names,
links, unexpected payloads, truncated files and out-of-budget images. Import completes
validation before publishing a new installed folder. `.taskin` is a versioned container,
not a ZIP file; use the launcher's import/export actions.

A missing, invalid or unsupported selected pack falls back to Reborn and records the
reason in `logs/game.log`. The launcher reports missing selections and validates a pack
before applying it. Refresh after external edits, or select Reborn to recover.

## Seasonal launcher and loading screens

**Seasonal effects** starts enabled with the **Automatic** theme. An explicitly saved
disabled preference stays disabled. Change these choices under **Settings → Interface**.

Automatic mode uses the local calendar on Windows (UTC on other platforms):

- **Halloween:** 24 October through 1 November, with a witch's hat on the logo, orange and purple
  bands round the pipes hung with pumpkin lanterns that flicker like candles, and bats flitting
  over a low mist with embers rising from it.
- **Christmas:** 1 December through 6 January, with a Santa hat, snowy trim and gentle snow.
- **New Year:** 31 December and 1 January, taking precedence over Christmas, with blue ARM and red
  CORE balloons tied to the logo, gold and cyan trim, confetti and restrained firework motifs.

While a battle loads in season, the loading screen's **field notes** take turns between the
season's own funny notes (**Festive**, **New Year** and **Spooky field notes**, under a heading in
the season's colour) and the usual tips, the season's first; the notes' plate is dressed for the
season too (snow drifts and holly, gold stars and confetti, a cobweb and a spider letting itself
down).

Choose **Halloween**, **Christmas** or **New Year** to use that theme year-round. Outside these dates,
Automatic leaves the launcher and loading screens undecorated. **Reduced motion** holds the decoration
still, **Reduced flashes** holds firework outlines still, and unfocused windows stop animation. Decorations do not intercept input.
Game startup, benchmark scene changes and benchmark quality changes follow the same
enabled theme, including the animated logo, trim and background effects. Turning seasonal
effects off removes these decorations, and the season's notes, from both the launcher and loading
screens. Pumpkin lanterns glow steadily with reduced flashes.
This option is independent of unit skins.

The intro's bedroom (the **INTRO ROOM** graphics setting) follows the same choice with three
rooms: the ordinary one, a Christmas one (December, or when chosen) and a New Year one (31 December
and 1 January, or when chosen). Halloween has no room of its own, and the room's Christmas ends
with December, not on 6 January. Seasonal effects off means the ordinary room. With reduced flashes the New Year room's fireworks do not light the room, and
the Christmas tree's lights shine steadily. `TA_ROOM_SEASON=normal|christmas|newyear` overrides the
choice (for captures and tests).
