"""Fits ta_config::benchmark's reference from the calibration's quick runs
(`tools/benchmark/calibrate.sh`, in scratch/bench/cal): the graphics card's time on the
reference pass at 1080p with 4x antialiasing, the pass's cost by size (a + b·drawn + c·shown,
megapixels; b kept as measured before: the reference pass draws about 1080p's pixels on every
screen from 1080p to 4K, so only the shown part varies), the
antialiasing's cost against 4x, the largest battle's tick and the reference pass's work besides
the simulation.

    python tools/benchmark/fit.py <tag>
"""
import glob
import json
import statistics
import sys

tag = sys.argv[1] if len(sys.argv) > 1 else "a"
B = 1.09
runs = []
for path in sorted(glob.glob(f"scratch/bench/cal/{tag}_*.json")):
    r = json.load(open(path, encoding="utf-8"))
    ref = r["reference"]
    prep = ref["cpu_ms"] - ref["tick_ms"] * ref["ticks_per_second"] / max(ref["fps"], 1.0)
    swarm = next(p for p in r["parts"] if p["id"] == "swarm")["stats"]
    shown = r["width"] * r["height"] / 1e6
    drawn = shown * r["reference_scale"] ** 2
    runs.append(dict(path=path, w=r["width"], h=r["height"], msaa=r["tested_msaa"], gpu=ref["gpu_ms"], prep=prep, tick=swarm["tick_ms"], units=swarm["units"], shown=shown, drawn=drawn, fps=ref["fps"]))
    print(f"{path}: {r['width']}x{r['height']} msaa {r['tested_msaa']} scale {r['reference_scale']:.2f}  gpu {ref['gpu_ms']:.3f}  prep {prep:.2f}  tick {swarm['tick_ms']:.2f} ({swarm['units']})  ref fps {ref['fps']:.0f}")

at4 = [x for x in runs if x["msaa"] == 4]
base = [x for x in at4 if (x["w"], x["h"]) == (1920, 1080)]
gpu_1080 = statistics.mean(x["gpu"] for x in base)
# gpu = (a + B·drawn) + c·shown: least squares over the sizes at 4x.
xs = [x["shown"] for x in at4]
ys = [x["gpu"] - B * x["drawn"] for x in at4]
mx, my = statistics.mean(xs), statistics.mean(ys)
c = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / max(sum((x - mx) ** 2 for x in xs), 1e-9)
a = my - c * mx
print(f"\nGPU_MS (1080p, 4x) = {gpu_1080:.3f}")
print(f"pass_cost(drawn, shown) = {a:.2f} + {B:.2f}·drawn + {c:.3f}·shown")
for x in at4:
    model = a + B * x["drawn"] + c * x["shown"]
    print(f"  {x['w']}x{x['h']}: measured {x['gpu']:.3f}, model {model:.3f} ({(model / x['gpu'] - 1) * 100:+.1f}%)")
print("msaa cost against 4x (1080p):")
for m in (1, 2, 4, 8):
    at = [x["gpu"] for x in runs if x["msaa"] == m and (x["w"], x["h"]) == (1920, 1080)]
    if at:
        print(f"  {m}x: {statistics.mean(at) / gpu_1080:.3f}")
print(f"TICK_MS = {statistics.mean(x['tick'] for x in runs):.3f} (each: {', '.join(f'{x['tick']:.2f}' for x in runs)})")
print(f"PREP_MS = {statistics.mean(x['prep'] for x in runs):.3f} (each: {', '.join(f'{x['prep']:.2f}' for x in runs)})")
