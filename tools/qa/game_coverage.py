#!/usr/bin/env python3
"""Check line-coverage floors for game boundaries and asset-free data decoders."""
import argparse
import json
from pathlib import Path


FLOORS = {
    "apps/ta_game/src/app/lifecycle.rs": 100.0,
    "apps/ta_game/src/keys.rs": 95.0,
    "crates/ta_config/src/validation.rs": 100.0,
}
DATA_FLOORS = {
    "crates/ta_formats/src/gaf.rs": 95.0,
    "crates/ta_audio/src/decode/wav.rs": 90.0,
}


def check(report, floors):
    """Return summaries and failures; malformed evidence must never pass a floor."""
    datasets = report["data"]
    if not isinstance(datasets, list) or not datasets:
        raise ValueError("data must be a nonempty list")
    files = []
    for dataset in datasets:
        entries = dataset["files"]
        if not isinstance(entries, list):
            raise ValueError("files must be a list")
        for entry in entries:
            if not isinstance(entry["filename"], str):
                raise ValueError("filename must be text")
            files.append(entry)

    summaries, failures = [], []
    for path, floor in floors.items():
        matches = [file for file in files if file["filename"].replace("\\", "/").endswith("/" + path)
                   or file["filename"].replace("\\", "/") == path]
        if len(matches) != 1:
            failures.append(f"{path}: expected one coverage entry, found {len(matches)}")
            continue
        lines = matches[0]["summary"]["lines"]
        count, covered = lines["count"], lines["covered"]
        if type(count) is not int or type(covered) is not int or not 0 <= covered <= count:
            raise ValueError(f"{path}: invalid line counters")
        percent = 100 * covered / count if count else 0
        summaries.append(f"{path}: {covered}/{count} lines ({percent:.2f}%; minimum {floor:.0f}%)")
        if percent < floor:
            failures.append(f"{path}: coverage is below {floor:.0f}%")
    return summaries, failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path, help="cargo llvm-cov JSON report")
    parser.add_argument("--scope", choices=("game", "data", "all"), default="game")
    args = parser.parse_args()
    floors = FLOORS if args.scope == "game" else DATA_FLOORS if args.scope == "data" else {**FLOORS, **DATA_FLOORS}
    try:
        report = json.loads(args.report.read_text(encoding="utf-8"))
        summaries, failures = check(report, floors)
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.error(f"cannot read coverage report: {error}")
    for summary in summaries:
        print(summary)
    if failures:
        parser.exit(1, "\n".join(failures) + "\n")


if __name__ == "__main__":
    main()
