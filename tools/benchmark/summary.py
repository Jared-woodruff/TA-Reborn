"""Sums up the benchmark's reports (`ta-reborn --benchmark <report.json>`, the launcher's in
`%APPDATA%\\TA Reborn\\benchmarks`): each part (its passes on the graphics card), each level of the
settings test, the reference pass, the effects test and the memory, in figures to set
`ta_config::benchmark`'s `reference` by (the reference pass's graphics card time, the largest
battle's tick, the reference pass's work besides the simulation).

    python tools/benchmark/summary.py <report.json>...
"""
import json
import sys


def main():
    for path in sys.argv[1:]:
        with open(path, encoding="utf-8") as f:
            r = json.load(f)
        content = r.get("content", 0)
        print(f"== {path}: {r.get('mode', 'full')} run, content {content}, {r['width']}x{r['height']}, {r['tested_msaa']}x antialiasing, reference pass at {r.get('reference_scale', 0):.2f}, memory {r['memory']}")
        for p in r["parts"]:
            s = p["stats"]
            print(f"  {p['id']:9} fps {s['fps']:6.1f}  1% {s['low1']:6.1f}  0.1% {s['low01']:6.1f}  stutters {s.get('hitches', 0):3}  cpu {s['cpu_ms']:5.2f}  gpu {s['gpu_ms']:5.2f}  tick {s['tick_ms']:5.2f} ({s['ticks_per_second']:4.1f}/s)  units {s['units']}")
            if p.get("passes"):
                print("            " + ", ".join(f"{q['pass']} {q['ms']:.2f}" for q in p["passes"]))
        for level in r["levels"]:
            s = level["stats"]
            print(f"  {level['level']:9} fps {s['fps']:6.1f}  1% {s['low1']:6.1f}  cpu {s['cpu_ms']:5.2f}  gpu {s['gpu_ms']:5.2f}  frames {s['frames']}")
        s = r.get("reference")
        if s and s.get("frames"):
            prep = s["cpu_ms"] - s["tick_ms"] * s["ticks_per_second"] / max(s["fps"], 1.0)
            print(f"  reference fps {s['fps']:6.1f}  1% {s['low1']:6.1f}  cpu {s['cpu_ms']:5.2f} (besides the simulation {prep:5.2f})  gpu {s['gpu_ms']:5.2f}  frames {s['frames']}")
        effects = r.get("effects") or []
        high = next((e["stats"]["gpu_ms"] for e in effects if e["effect"] == "high"), None)
        for e in effects:
            ms = e["stats"]["gpu_ms"]
            cost = f"  (costs {high - ms:5.2f})" if high is not None and e["effect"] != "high" else ""
            print(f"  effect {e['effect']:18} gpu {ms:5.2f}{cost}")
        largest = next((p for p in r["parts"] if p["id"] == ("swarm" if content >= 2 else "cinder")), None)
        if largest:
            print(f"  largest battle: {largest['stats']['tick_ms']:.2f} ms a tick for {largest['stats']['units']} units")


if __name__ == "__main__":
    main()
