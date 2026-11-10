# Memory soak tests

After a player's report (2026-11-11: "I made the radar plane and used it to spy the enemy CPU...
it used a ton of RAM and kept increasing", then a crash), three ways to look for memory that is
kept and never given back, and the watch the game itself keeps.

## The simulation and the computer players (any computer, no graphics card)

```bash
cargo test --release -p ta_ai --test memory -- --nocapture
```

Two computer players on Sherwood under a low unit limit, so that the battle stops growing; then
radar planes patrol over the enemy's base for 14 minutes while units are lost and rebuilt. The
memory must stay as it was (8 MB and 6% are allowed). `TA_TEST_MAP`, `TA_TEST_UNIT_LIMIT` (90),
`TA_TEST_WARM_UP` (14), `TA_TEST_MINUTES` (14), `TA_TEST_LOG=1` (the memory each minute).
Skipped without a TA install.

## The whole game (Windows)

```bash
pwsh -File tools/soak/spy.ps1
```

`--demo spy` with `--map` (`apps/ta_game/src/app/spy.rs`): a skirmish both of whose sides the
computer plays, fast-forwarded `TA_SPY_MINUTES` (12) minutes, then `TA_SPY_PLANES` (3) radar
planes of ours patrolling over the enemy's start, selected, the camera on them (`TA_SPY_RIDE=1`
rides the first in its cockpit, `TA_SPY_GOD=1` keeps them from being shot down). The game logs
its memory every ten seconds of battle; the script fails when, after the first minute, it keeps
climbing faster than `-MaxGrowthMb` (40) MB a minute of battle. `-Map`, `-Minutes`, `-Extra`,
`-Exe`. Its settings are a folder of its own, never the player's.

## What holds the memory (Windows)

```bash
cargo build --release -p ta_game --features memtrack --target-dir scratch/memtrack_target
```

A heap profiler (`apps/ta_game/src/memtrack.rs`): about one allocation in every 128 KiB allocated
is recorded with its call stack, and `--demo spy` logs every ten seconds of battle the places in
the game's own code holding the most of what is still allocated, and what each gained since:
a leak is a place that keeps gaining. Run that build with `-Exe` above. Only the game's own
allocations are seen: the graphics driver's are not (watch the GPU's process memory counters for
those).

## The watch in the game

`apps/ta_game/src/app/memory_guard.rs`: a line in the log every minute of a battle (`memory:
...`); past a warning level (half the computer's memory, three fifths of a small one's, or 4 GB
more than after the battle's first minute) the highlights' copies are let go, a single-player
battle is saved and the player told; at the limit (four fifths of the computer's memory, or under
1 GB of it left) the battle is also paused. `TA_MEMORY_TOTAL_MB=<n>` takes the computer to have
that much memory, to try the warnings. A crash report (`logs\crash.txt`) gives the memory then.
