# Historical replay fixture

`format36.tarep` is a synthetic, asset-free binary fixture assembled from the original
fixed-width bincode field order at commit `cca774002c3e32db379259c4a6ed8fc9c9b5203a`.
It was written from primitive little-endian values, without serializing the current
Rust structures. It contains two players (human ARM and hard-AI CORE), Conquest starting
veterancy, a queued patrol formation with explicit positions, a heal cheat and resignation.

The record, setup, rules, commands and their nested wire types were compared at these
version changes:

| Version | Commit | Format change |
| --- | --- | --- |
| 36 | `cca774002c3e32db379259c4a6ed8fc9c9b5203a` | Earliest available history |
| 37 | `3b87c3d6ef898493f7c9d1f80e63a1ea8b5f43d9` | None; simulation/data compatibility |
| 38 | `897014485c61d9a8127f7277030108dc39f2c1ef` | None; simulation/AI compatibility |
| 39 | `bfae33cc3f500ef1104ec321ad3dcbd8733b90ac` | None; simulation/AI compatibility |
| 40 | `7a3c66ca21b0ee53d0b41d43ad02ac7e24a4c995` | None; simulation/AI compatibility |

The test exercises each verified version with these frozen bytes. Versions before 36
and unknown future layouts are deliberately not guessed. This verifies decoding, not
historically identical battle outcomes: old records did not preserve AI orders or their
engine build, so compatibility playback can differ from the original game.
