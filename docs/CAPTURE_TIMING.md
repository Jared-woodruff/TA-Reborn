# Capture timing

Capture uses a timer on each capturer's current order. Its duration depends on the target's
costs, health and kill count at the start of that order. It does not use the capturer's
WorkerTime or kill count, and simultaneous capturers do not pool their timers.

Ignoring integer rounding, the duration in seconds is:

```text
min(60, 5 + energy_cost / 2000 + metal_cost / 140)
    × (1 + current_hp / maximum_hp) / 2
    × (10 + floor(target_kills / 5)) / 10
```

The cost component is capped before the health and target-veterancy adjustments. Thus a
full-health Galactic Gate with no kills takes 60 seconds of active capture work, while the
same gate at 1 HP takes 30 seconds. A veteran target can take longer than 60 seconds. Travel,
turning and the capturer's script-controlled build stance add time before active work.

The snapshot survives approaching the target and temporary interruptions of the beam. A new
order starts a new timer and samples the target again. Completed work waits for room under
the unit limit without restarting. The target's progress bar shows the furthest-progressed
capture order; cancelling that order falls back to the remaining capturers' progress.

## Verification against the original executable

The rule was recovered by locally inspecting the installed original `TotalA.exe` with
Ghidra 11.4.2 and Microsoft DUMPBIN. The binary has a PE timestamp of 30 July 1998 and SHA-256:

```text
3B9C0FADABF3DC67ED5F05A70F1E1505A0C65DEADD1A3C930ADFE30E2A84995E
```

The capture handler starts at virtual address `0x00404270`. Capture-specific error strings
identify the handler, and its final state calls ownership transfer at `0x00488570`.

| Addresses | Operation |
| --- | --- |
| `0x00404359`–`0x00404385` | Compute the cost-based timer and truncate to whole ticks |
| `0x0040438A`–`0x0040439B` | Clamp the cost component to 1,800 ticks |
| `0x0040439E`–`0x004043D2` | Multiply by `(current_hp + maximum_hp) / (2 × maximum_hp)`, truncating |
| `0x004043D5`–`0x00404407` | Apply the target's kill count, truncating again |
| `0x004045B3`–`0x004045BB` | Compare elapsed work against the saved timer |
| `0x00404698`–`0x004046A3` | Advance two work ticks and sleep two game ticks |
| `0x00439E80`–`0x00439E9D` | Set the wake-up game tick |
| `0x0042AD3A`–`0x0042AD7B` | Parse energy and metal costs into type offsets `0x186` and `0x18A` |
| `0x0042C38F`–`0x0042C39E` | Parse MaxDamage into type offset `0x1FA` |
| `0x0048699D`–`0x004869CA` | Credit an enemy kill to unit field `0xB8` |
| `0x004E43A0`–`0x004E43C6` | Convert the x87 result to an integer with rounding toward zero |

The original stores its timer and elapsed work on the order, at offsets `0x3A` and `0x36`.
The health and kill-count inputs come from its target, not its capturer.

The cost expression uses the binary32 coefficients `0x3A03126F` and `0x3BEA0EA1`, representing
`8589935 / 2^34` and `15339169 / 2^31`, together with factors of 30 and an offset of 150 ticks.
`UnitType::capture_ticks` evaluates those coefficients as integer ratios to preserve the
original constants without introducing floating-point arithmetic into the simulation.
After the original integer truncations, it rounds the timer up to an even tick count to
match the original two-tick work intervals. Reborn then counts active work once per tick.
For malformed or unusual mod definitions, costs are nonnegative, maximum HP is at least one,
health is bounded by maximum HP, and a capture takes at least two ticks.

| Target | Timer threshold before even-tick rounding | Active work |
| --- | ---: | ---: |
| CORE Galactic Gate, full health, no kills | 1,800 ticks | 60 seconds |
| CORE Galactic Gate, 1 HP, no kills | 900 ticks | 30 seconds |
| ARM Flash, full health, no kills | 185 ticks | 6.2 seconds |
| ARM Flash, full health, 10 kills | 222 ticks | 7.4 seconds |

These are reconstructed timer durations, not stopwatch measurements of a running original
game. No original executable, assets, assembly dump or decompiler output is included here.

## Regression checks

`crates/ta_sim/src/construction_tests.rs` covers the known timer values, independence from
WorkerTime and capturer kills, health/veterancy snapshots, cancellation, multiple capturers,
unit limits and deterministic checksums. `crates/ta_sim/tests/capture_timing.rs` additionally
loads the player's original unit definitions and Commander scripts, checking full-health and
damaged gates and an ordinary tank. That test skips when no original installation is found.

Simulation revision 93 distinguishes the new timing from earlier command-history saves and
replays. Older saves cannot resume under this revision; replay playback retains the existing
compatibility warning behavior.
