# Capture timing

Capture follows the three-stage specification supplied by the contributor:

1. Start with 5 seconds, add energy cost / 2000 and metal cost / 140, and cap the base at 60 seconds.
2. Multiply by (1 + current HP / maximum HP) / 2.
3. Multiply by 1 + floor(target kills / 5) / 10.

Costs, health and kills are the target's. WorkerTime and the capturer's kills do not affect the
duration. The cost contributions are fractional; only the number of five-kill groups is rounded
down. The base is capped before the health and kill adjustments, so veteran targets may take
longer than 60 seconds.

## Deterministic implementation and rounding

The implementation combines the cost fractions over a common denominator of 14,000 and uses
integer ratios throughout. It preserves fractional health and time until converting the final
duration to ticks, rounding up to the next whole tick at 30 ticks per second. It does not use
the executable-specific binary32 coefficients, intermediate truncations or two-tick rounding
from the earlier implementation.

Negative costs are treated as zero and health is clamped to the target's valid range.
Overflow-safe ratio evaluation saturates the final tick count for extreme mod values and kills.

| Target | Active work |
| --- | ---: |
| CORE Galactic Gate, full health, no kills | 1,800 ticks / 60 seconds |
| CORE Galactic Gate, 0 HP, no kills (formula boundary) | 900 ticks / 30 seconds |
| CORE Galactic Gate, 1 HP, no kills | 901 ticks / about 30.033 seconds |
| ARM Flash, full health, no kills | 186 ticks / 6.2 seconds |
| ARM Flash, full health, 10 kills | 223 ticks / about 7.433 seconds |

## Order lifecycle

Each capture order snapshots the target's costs, health and kills before approaching. Travel,
turning and the capturer's script-controlled build stance add time before active work. A new
order starts a new timer. Simultaneous capturers do not pool work, and a completed capture
waits for enough room under the unit limit. The target's progress bar shows the furthest
advanced active capture order and falls back to the others when that order is cancelled.

## Verification and provenance

The tests validate the contributor-supplied specification, not a claim of independently measured
original-engine behavior. Earlier work in PR #63 derived timings through disassembly of the
original executable. That history conflicts with the upstream contributing guide's clean-room
rule and remains disclosed for maintainer review; this revision does not claim that replacing
the calculation removes that provenance concern. No original executable, game assets or
disassembly output is included.

Regression tests cover cost and kill thresholds, health scaling, final rounding, extreme values,
WorkerTime independence, snapshots, restarts, simultaneous capturers, unit limits and checksums.
The installed-data test loads the player's own Commander scripts and definitions and skips
when no original installation is found.

## Compatibility

Simulation revision 94 distinguishes the new capture timing from earlier command-history saves
and replays. Older saves cannot resume under this revision; older replays retain the existing
compatibility warning behavior. Network protocol 98 prevents multiplayer sessions with earlier
builds whose capture timing would diverge. The record layout and mod-validation schema versions
are unchanged.
