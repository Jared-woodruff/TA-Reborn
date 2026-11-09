"""Compare two compiled ta_sim stress test executables, alternating their run order.

Build each revision with:
    cargo test --release --locked -p ta_sim --test stress --no-run
Copy its executable outside Cargo's output before building the next revision.
Requires an original TA installation. Timings are meaningful on a quiet machine.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import statistics
import subprocess


def result(text):
    timing = re.search(
        r"^average ([\d.]+) ms/tick, p50 ([\d.]+), p95 ([\d.]+), p99 ([\d.]+), worst ([\d.]+) ms$",
        text, re.MULTILINE,
    )
    checksum = re.search(r"^final checksum ([0-9a-f]+), (\d+) units, (\d+) features alive$", text, re.MULTILINE)
    if not timing or not checksum:
        raise ValueError("No measured battle in the output; check the TA installation and test name.")
    return {
        **dict(zip(("average_ms", "p50_ms", "p95_ms", "p99_ms", "worst_ms"), map(float, timing.groups()))),
        "checksum": checksum[1], "units": int(checksum[2]), "features": int(checksum[3]),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--test", choices=("five_thousand_unit_battle", "swarm_battle", "spread_move"), default="five_thousand_unit_battle")
    parser.add_argument("--units", type=int, default=10000, help="Per side for battles; total for spread_move")
    parser.add_argument("--ticks", type=int, default=900)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if min(args.units, args.ticks, args.rounds) < 1:
        parser.error("units, ticks and rounds must be positive")
    executables = {name: path.resolve(strict=True) for name, path in (("baseline", args.baseline), ("candidate", args.candidate))}
    args.out.mkdir(parents=True, exist_ok=True)
    env = {**os.environ, "TA_STRESS_UNITS": str(args.units), "TA_STRESS_TICKS": str(args.ticks)}
    report = {
        "test": args.test, "units": args.units, "ticks": args.ticks, "rounds": args.rounds,
        "environment": {k: env[k] for k in ("TA_STRESS_MAP", "TA_STRESS_GIVE_WAY", "TA_STRESS_EVERY", "TA_STRESS_SPIKES", "RAYON_NUM_THREADS") if k in env},
        "executables": {name: {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()} for name, path in executables.items()},
        "runs": [],
    }
    expected = None
    for round_index in range(args.rounds):
        order = ("baseline", "candidate") if round_index % 2 == 0 else ("candidate", "baseline")
        for version in order:
            completed = subprocess.run([str(executables[version]), args.test, "--exact", "--ignored", "--nocapture"], env=env, capture_output=True, text=True, encoding="utf-8", errors="replace")
            text = completed.stdout + completed.stderr
            log = args.out / f"{version}-{round_index + 1}.log"
            log.write_text(text, encoding="utf-8")
            if completed.returncode:
                raise RuntimeError(f"{version} exited with {completed.returncode}; see {log}")
            sample = result(text)
            state = (sample["checksum"], sample["units"], sample["features"])
            if expected is not None and state != expected:
                raise ValueError(f"Simulation results differ: {state} != {expected}; see {log}")
            expected = state
            report["runs"].append({"version": version, "round": round_index + 1, **sample})
            print(f"{version} round {round_index + 1}: {sample['average_ms']:.2f} ms/tick, p99 {sample['p99_ms']:.2f}, checksum {sample['checksum']}", flush=True)
    report["medians"] = {
        version: {key: statistics.median(r[key] for r in report["runs"] if r["version"] == version) for key in ("average_ms", "p50_ms", "p95_ms", "p99_ms")}
        for version in executables
    }
    (args.out / "comparison.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    for key, baseline in report["medians"]["baseline"].items():
        candidate = report["medians"]["candidate"][key]
        delta = (1 - candidate / baseline) * 100 if baseline else 0
        change = f"{abs(delta):.1f}% {'lower' if delta >= 0 else 'higher'}" if baseline else "below timing precision"
        print(f"{key}: {baseline:.2f} -> {candidate:.2f} ({change})")


if __name__ == "__main__":
    main()
