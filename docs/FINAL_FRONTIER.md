# The Final Frontier 7.4

The launcher offers this historical ARM/CORE space conversion by **Savant, Longbow and
the TA:FF team**, hosted by [TA Universe](https://taff.tauniverse.com/index.html).
The catalogue does not claim that development has officially ended.

## Install and play

1. Open **MODS** in the launcher and find **The Final Frontier** in the **LIBRARY** (search for
   **Final Frontier** or **TAFF**).
2. Choose **WEBSITE** to open the [mod's website](https://taff.tauniverse.com/index.html)
   in your browser and download `taffV74.zip` there. The expected release is 8,581,261
   bytes (about 8.2 MiB).
3. Choose **Have the file?** in its details (or in the menu a right-click on its card opens) and
   select the downloaded archive. The launcher
   verifies its pinned size and SHA-256 before installing it; a different or damaged
   release leaves any existing installation intact.
4. **PLAY** enables the conversion and disables other catalogue conversions. Player-added
   map mods remain enabled; avoid mixing unrelated gameplay modifications.

The Final Frontier is a manual-download catalogue entry. It has no GET/UPDATE action,
backup-download menu or optional download links in the launcher. Automatic installation
requests are rejected before creating a download workspace or contacting the host.
Other catalogue mods keep their existing download behavior.

At verification on 23 October 2026, direct HTTP clients received Cloudflare HTTP 403
challenges from both hosts while browser downloads succeeded. The launcher therefore
uses the website and verified local import for this historical release.

## Optional content

The [mod's website](https://taff.tauniverse.com/index.html) provides the release information,
map packs and installation instructions. Obtain optional content through your browser.

**AI Boost is already included in the main release.** Its `AIBoost.ufo` is byte-for-byte
identical to the standalone patch. Installing it again is unnecessary.

The two map packs contain ZIP files inside ZIP files. They are optional and are not
automatically installed with the conversion. To add a readable map, extract the outer
pack and its individual map ZIP, then drop the map's `.ufo` file on the launcher's **MAPS**
page: it is imported as a map pack and turned on. Import the map archive itself; Asteroid Field's package
also includes an older AI text file that should not replace the main mod's AI data.

Three nested map archives are damaged in the supplied downloads:

- Pack 1: `GalacticRim.zip`.
- Pack 2: `LongNightV2.zip` and `StarShipGraveYardV3.zip`.

Seven nested archives were readable: Space The Frontier, Asteroid Field v3, Beauty in
the Dark v3, Iceteroids, Megaroid v3, Solar Driftwood v2 and Legacy of the Ancients v2.
Readable archives are not a guarantee of complete map compatibility. Some historical
maps may refer to additional feature packs; check their accompanying readmes.

## Compatibility checks

The card is marked **Plays, with gaps**. Tests used an original GOG installation and
isolated local mod folders; no original or third-party mod assets are shipped in Reborn.

- The base imports without folder remapping and contains 172 mod unit records. With the
  original data it loads 450 unit types, 248 weapons and ARM/CORE Command Cruisers as the
  starting units. It contains no maps; the advertised campaign is a separate download.
- A 12-minute simulated Sherwood AI match built 133 units per side and fired 2,137 shots.
- A 20-minute Asteroid Field AI match built 200/174 units and fired 2,050 shots, including
  the conversion's Wildcat and Ironangel ships. Both tests had no duplicate destruction
  events.
- On Space The Frontier, both AIs stalled after initial economy/defences: 10/7 units
  built and no shots over 20 simulated minutes. This is a gameplay limitation, not a
  successful full compatibility check.
- Source data has 33 unresolved legacy references, including some helper weapons,
  explosion definitions, a movement class and two CORE yard wreck models. Two additional
  malformed interface GAF files (`anims/flagship.gaf`, `anims/liberty2.gaf`) were found;
  classic build buttons can fall back to unit pictures. All 932 mounted model files and
  452 scripts parsed successfully. This does not certify every script/animation path.

The original downloads are preserved, not silently rewritten to guess missing content.

## Verified fingerprints

Primary and backup main-mod downloads were identical. SHA-256 values:

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `taffV74.zip` | 8,581,261 | `caca1844d4bb1ce591c25030327a05d106e3f5da4169dc1143787c0149a97fc1` |
| `TAFF-Map_pack_1.zip` | 22,515,333 | `8320bd1debdeda0f4ad0a9fc5feab93449f200c45420ac5d29b36820645aa18c` |
| `TAFF-Map_pack_2.zip` | 12,111,237 | `9ec996af2c8919e3f17ea9f482f141c8392daa0280345105f1f02492909113fc` |
| `AIBoost.zip` | 626,218 | `f2f96c43bc6ba31ce5ba6a4c38f36fe428a692ae4db79a05b8c9901e4459c66e` |

Local investigation logs and downloaded packages are under ignored `scratch/taff/`.
The catalogue includes website attribution and integrity metadata, not copies of these
archives or website artwork. Its bundled banner is newly generated fan artwork guided
by the user's Liberty, Vengeance and ARM dreadnought references; it is not an in-game
screenshot or official mod artwork. See the [asset provenance and prompt](../apps/ta_launcher/assets/README.md).
Synthetic regressions cover verified mirror fallback, cancellation,
manual import preserving an existing installation, HTTPS enforcement and provider routing.

## Original integration validation (23 October 2026)

- `cargo test --release --locked -p ta_launcher`: 150 passed, 0 failed, 3 ignored,
  including seven new download/import regressions and responsive card checks.
- The Windows installer and portable package were built; checksums, ZIP integrity and
  packaged binaries were verified. No original game or mod archives are bundled.
- Release builds of the launcher, game and data tools succeeded. All 13 language files
  contain translations for every active interface string.
- Native screenshots checked the catalogue and installed card at normal scale, plus
  the German card at 150% scale in the compact layout.
- The actual downloaded release was installed through the launcher's catalogue manual
  import path in an isolated profile. The manifest, embedded AI Boost and temporary-file
  cleanup were verified. The installed mod's Command Cruisers, Wildcat and Ironangel
  also rendered in a native game capture without a panic or GPU validation error.

These checks do not imply that automated retrieval bypasses the host's browser challenge
or that every historical map and campaign has been tested.

## Website/import update validation (24 October 2026)

- All 165 launcher tests and 50 shared UI tests passed, including the manual-only card,
  pre-I/O rejection, bounded offline banner decoding and verified import regressions.
- Native captures checked the original banner and website/import controls offline,
  at normal scale and German 150% scale, and checked the installed card's controls.
- The real pinned archive imported through the catalogue path in an isolated profile;
  its manifest, AI Boost hash and temporary workspace cleanup passed verification.
- A native `--get-mod finalfrontier` request created neither an install nor a download
  workspace. All 13 non-English languages translate all 2,978 active interface strings.
