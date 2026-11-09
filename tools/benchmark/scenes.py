"""The benchmark's eight scenes (`apps/ta_game/benchmark/*.toml`, staged scenes as
`apps/ta_game/src/stage.rs` describes them), on TA Reborn's own worlds so every player has them,
each showing off what the game draws there (their lengths and pictures' times are
`apps/ta_game/src/benchmark.rs`'s `SCENES`, their captions `crates/ta_ui/src/benchmark.rs`'s):

* sapphire: Sapphire Reach at dawn, the war `--demo war` stages (battle lines of ships trading
  broadsides down the channel, a landing under a fortress, the Arm's firing lines advancing on
  the Core's on the plateau, Krogoths striding into the gap, bombers and fighters overhead).
* cockpit: Greenmantle in the morning: the camera dives into the lead Stumpy of a column on the
  forest road (`[[ride]]`) and rides it across the ford into the Core's line on the far bank.
* highland: Highland Rift in a thunderstorm, the Arm's army storming the Core's line and towers
  south of the western lake, the woods on fire; its seconds 16 to 22.5 are the tests' (`TEST_FROM`
  in `benchmark.rs`), seen from behind the towers: the battle, the burning woods, the lake.
* space: Lagrange Station: the Arm storms the station rock across its western gantry over the
  void, fighters and bombers over it, the planet far below.
* build: Redstone Mesas at sunset: a base rising under the nano-lathes, the Commander rising out
  of its lattice first.
* swarm: Reactor Grid filled edge to edge, ten thousand a side in contact (the processor's test),
  the camera rising from the front to the whole map in strategic icons.
* cinder: Cinder Basin from dusk into the night, some 1,800 a side meeting on the eastern plain,
  the lava pool at the end of the front, a nuclear blast on the Core's rear.
* winter: Midnight Square a minute to midnight: the Core storms the town round its clock tower,
  the racks' fireworks going up all round (`fireworks` event).

The output is deterministic and committed (the scores compare runs: every player plays the same
scenes). Units of the expansions (`missing = "skip"`) are left out where they are not installed.

    python tools/benchmark/scenes.py [--heights scratch/bench/maps] [scene ...]   (from the repository root)

`--heights` is a folder with `ta-tools map "Cinder Basin" <folder>`'s and `ta-tools map "Reactor
Grid" <folder>`'s heightmaps (`<map>_height.png`: those armies stand on level ground only). numpy
and Pillow. Scouting a scene: `--demo stage` with `TA_STAGE=apps/ta_game/benchmark/<scene>.toml`
(a contact sheet with `TA_SHOTS`, `TA_SHOT_STEP`, `TA_CAPTURE_FPS=30`).
"""
import argparse
import math
import pathlib
import random

import numpy as np
from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = ROOT / "apps" / "ta_game" / "benchmark"
PX = 16384


class Scene:
    def __init__(self, header):
        self.lines = [header.strip() + "\n"]
        self.count = [0, 0]

    def w(self, text):
        self.lines.append(text.strip() + "\n")

    def unit(self, kind, side, x, z, heading, label=None, count=1, rows=1, spacing=None, **extra):
        text = f'[[unit]]\ntype = "{kind}"\nside = {side}\nat = [{x:.0f}, {z:.0f}]\nheading = {heading:.0f}\n'
        if label:
            text += f'label = "{label}"\n'
        if count > 1:
            text += f"count = {count}\n"
        if rows > 1:
            text += f"rows = {rows}\n"
        if spacing is not None:
            text += f"spacing = {spacing:.0f}\n"
        for k, v in extra.items():
            text += f"{k} = {fmt(v)}\n"
        self.w(text)
        if side < 2:
            self.count[side] += count

    def line(self, kinds, side, x, z, count, heading, spacing, label):
        """A line abreast of `count`, alternating `kinds` (one entry each, interleaved)."""
        n = len(kinds)
        # Facing east or west the line runs along z; north or south, along x.
        along_z = round(heading) % 180 == 90
        for i, kind in enumerate(kinds):
            k = len(range(i, count, n))
            if k == 0:
                continue
            # Unit j of the whole line stands at (j - (count - 1) / 2) * spacing; this kind's
            # are j = i, i + n, ...: their middle is offset by this much.
            first, last = i, i + (k - 1) * n
            offset = ((first + last) / 2 - (count - 1) / 2) * spacing
            ux, uz = (x, z + offset) if along_z else (x + offset, z)
            self.unit(kind, side, ux, uz, heading, label, count=k, spacing=spacing * n)

    def order(self, t, units, do, **kw):
        text = f'[[order]]\nt = {t}\nunits = {fmt(units)}\ndo = "{do}"\n'
        for k, v in kw.items():
            text += f"{k} = {fmt(v)}\n"
        self.w(text)

    def event(self, t, do, **kw):
        text = f'[[event]]\nt = {t}\ndo = "{do}"\n'
        for k, v in kw.items():
            text += f"{k} = {fmt(v)}\n"
        self.w(text)

    def key(self, t, eye, height, look, look_height=0, fov=46):
        self.w(f"[[camera.key]]\nt = {t}\neye = [{eye[0]:.0f}, {eye[1]:.0f}]\nheight = {height}\nlook = [{look[0]:.0f}, {look[1]:.0f}]\nlook_height = {look_height}\nfov = {fov}\n")

    def track(self, t, eye, height, units, look_height=0, fov=46):
        """A key looking at units as they move."""
        self.w(f'[[camera.key]]\nt = {t}\neye = [{eye[0]:.0f}, {eye[1]:.0f}]\nheight = {height}\ntrack = "{units}"\nlook_height = {look_height}\nfov = {fov}\n')

    def ride(self, t, unit, leave=None):
        self.w(f'[[ride]]\nt = {t}\nunit = "{unit}"\n' + (f"leave = {leave}\n" if leave is not None else ""))

    def save(self, name, note):
        text = "".join(self.lines) + f"# {note}: Arm {self.count[0]} units, Core {self.count[1]}\n"
        path = OUT / f"{name}.toml"
        path.write_text(text, encoding="utf-8", newline="\n")
        print(f"{path.relative_to(ROOT)}: Arm {self.count[0]}, Core {self.count[1]}, {sum(self.count)} units")


def fmt(v):
    if isinstance(v, bool):
        return str(v).lower()
    if isinstance(v, str):
        return f'"{v}"'
    if isinstance(v, (list, tuple)):
        return "[" + ", ".join(fmt(x) for x in v) + "]"
    if isinstance(v, float):
        return f"{v:g}"
    return str(v)


HEADER = """# TA Reborn's benchmark: {what}. Written by tools/benchmark/scenes.py (edit that, not this).
map = "{map}"
time = {time}
reveal = true
hud = false
icons = false
unlimited = true
seed = {seed}
missing = "skip"
"""


def sapphire():
    """`--demo war`'s battle (app.rs `war_demo`, places as fractions of the map there), at dawn."""
    s = Scene(HEADER.format(what="Sapphire Reach at dawn", map="Sapphire Reach", time="[[0, 6.15], [28, 7.1]]", seed=7) + "orders_per_tick = 20\n")
    mid, cz, gap, advance = 0.4425 * PX, 0.546 * PX, 280.0, 400.0
    s.w(f"[[clear]]\nat = [{mid - 120:.0f}, {cz:.0f}]\nradius = 1050")
    ranks = [
        (["ARMBULL", "ARMHAM"], ["CORREAP", "CORGOL"], 0.0, 62, "front"),
        (["ARMFIDO", "ARMJETH"], ["CORMORT", "CORCRASH"], 110.0, 48, "kbots"),
        (["ARMMART", "ARMMERL"], ["CORMART", "CORVROC"], 260.0, 64, "art"),
    ]
    for arm, core, back, spacing, name in ranks:
        stand = mid - (gap / 2 + back)
        s.line(arm, 0, stand - advance, cz, 20, 90, spacing, f"arm_{name}")
        # The Core's ranks in two halves, 400 px apart in the middle for the Krogoths.
        stand = mid + (gap / 2 + back)
        reach = (8 * 80 + 400) / 2
        for half in (-1, 1):
            s.line(core, 1, stand, cz + half * reach, 9, 270, 80, f"core_{name}")
    for name in ("front", "kbots", "art"):
        s.order(0.05, f"arm_{name}", "move", by=[advance, 0], formation=False)
    s.unit("CORKROG", 1, mid + 560, cz, 270, "krogoths", count=2, spacing=150)
    s.order(4, "krogoths", "move", by=[-560, 0], formation=False)
    # The battle lines at sea, heading east down the channel.
    for side, z, names in ((0, 0.349 * PX, ["ARMROY"] * 3 + ["ARMCRUS"] * 3 + ["ARMBATS"] * 2), (1, 0.3673 * PX, ["CORROY"] * 3 + ["CORCRUS"] * 3 + ["CORBATS"] * 2)):
        for k, name in enumerate(names):
            s.unit(name, side, 0.262 * PX - k * 170, z, 90, "arm_fleet" if side == 0 else "core_fleet")
        s.order(0.05, "arm_fleet" if side == 0 else "core_fleet", "move", by=[(0.47 - 0.262) * PX, 0], formation=False)
    # The fortress on the northern island's shore, and the landing on it.
    for name, fz, every in (("CORHLT", 0.250, 160), ("CORTOAST", 0.238, 240), ("CORFLAK", 0.232, 320), ("CORDOOM", 0.226, 360), ("CORPUN", 0.214, 300)):
        xs = []
        x = 0.295 * PX
        while x < 0.395 * PX:
            xs.append(x)
            x += every
        s.unit(name, 1, (xs[0] + xs[-1]) / 2, fz * PX, 180, "fort", count=len(xs), spacing=every)
    s.unit("ARMANAC", 0, 0.345 * PX, 0.295 * PX, 0, "landing", count=10, spacing=64)
    s.unit("ARMAH", 0, 0.345 * PX, 0.295 * PX + 72, 0, "landing", count=5, spacing=128)
    s.order(0.05, "landing", "fight", by=[0, (0.19 - 0.295) * PX], formation=False)
    # The air war: bombers in formation on each other's artillery, fighters on patrol.
    s.unit("ARMTHUND", 0, 0.29 * PX, 0.52 * PX, 90, "arm_bombers", count=9, rows=3, spacing=150, airborne=True)
    s.unit("CORSHAD", 1, 0.56 * PX, 0.60 * PX, 270, "core_bombers", count=6, rows=2, spacing=150, airborne=True)
    s.order(2, "arm_bombers", "attack", target="core_art")
    s.order(3, "core_bombers", "attack", target="arm_art")
    # (Then on over the lines, to the end: an aircraft with nothing to do lands.)
    s.order(2.1, "arm_bombers", "patrol", at=[round(mid + 0.05 * PX), round(cz)], queue=True)
    s.order(3.1, "core_bombers", "patrol", at=[round(mid - 0.05 * PX), round(cz)], queue=True)
    s.unit("ARMHAWK", 0, 0.33 * PX, 0.50 * PX, 90, "arm_fighters", count=8, rows=2, spacing=130, airborne=True)
    s.unit("CORVAMP", 1, 0.52 * PX, 0.55 * PX, 270, "core_fighters", count=8, rows=2, spacing=130, airborne=True)
    s.order(0.5, "arm_fighters", "patrol", at=[round(mid + 0.02 * PX), round(cz)])
    s.order(0.5, "core_fighters", "patrol", at=[round(mid - 0.02 * PX), round(cz)])
    # The camera: low over the sea between the fleets, out to the landing, round to the plateau,
    # down among the lines, and up over all of it at sunrise.
    s.key(0, (3700, 6520), 38, (3650, 5860), 12, 50)
    s.key(6, (4500, 6560), 60, (4650, 5860), 14, 48)
    s.key(11, (5350, 5650), 240, (5650, 4200), 0, 46)
    s.key(15, (6250, 7350), 230, (7250, 8900), 10, 46)
    s.key(20, (6750, 8300), 50, (7400, 8950), 25, 50)
    s.key(24, (7700, 9750), 95, (7200, 8900), 20, 46)
    s.key(28, (5400, 11400), 1500, (6300, 6600), 0, 46)
    s.save("sapphire", "Sapphire Reach")


def highland():
    """The Commander trailer's battle south of the western lake (scratch/cine), larger: the Arm's
    army north of the Core's line and towers, the woods to the west of it set alight."""
    s = Scene(HEADER.format(what="Highland Rift in a thunderstorm", map="Highland Rift", time="[[0, 15.1], [30, 15.7]]", seed=11) + "rain = true\norders_per_tick = 20\n")
    s.w("[[clear]]\nat = [5800, 9150]\nradius = 1150")
    # The Arm: ranks facing south, a second wave behind.
    s.line(["ARMBULL", "ARMSTUMP"], 0, 5800, 8640, 20, 180, 52, "arm_front")
    s.line(["ARMHAM", "ARMROCK", "ARMWAR"], 0, 5800, 8555, 24, 180, 40, "arm_kbots")
    s.line(["ARMZEUS", "ARMFIDO"], 0, 5800, 8475, 18, 180, 48, "arm_kbots")
    s.line(["ARMMART", "ARMMERL"], 0, 5800, 8360, 12, 180, 66, "arm_art")
    s.unit("ARMFLASH", 0, 4950, 8600, 160, "arm_flank_w", count=12, rows=2, spacing=44)
    s.unit("ARMFLASH", 0, 6650, 8600, 200, "arm_flank_e", count=12, rows=2, spacing=44)
    s.line(["ARMMAV", "ARMHAM", "ARMJETH"], 0, 5800, 8230, 24, 180, 40, "arm_wave")
    s.unit("ARMLATNK", 0, 5800, 8145, 180, "arm_wave", count=12, spacing=56)
    # The Core: a line before its towers, its artillery behind.
    s.line(["CORGOL", "CORREAP"], 1, 5780, 9450, 16, 0, 56, "core_front")
    s.line(["CORAK", "CORTHUD", "CORSTORM", "CORPYRO"], 1, 5780, 9540, 26, 0, 36, "core_kbots")
    s.unit("CORLLT", 1, 5800, 9660, 0, "core_towers", count=8, spacing=170)
    s.unit("CORHLT", 1, 5800, 9780, 0, "core_towers", count=4, spacing=340)
    s.unit("CORPUN", 1, 5800, 9930, 0, "core_towers", count=3, spacing=420)
    s.line(["CORMART", "CORLEVLR"], 1, 5800, 10060, 10, 0, 80, "core_art")
    # The air: Thunders on the towers, Avengers over the battle.
    s.unit("ARMTHUND", 0, 3600, 7700, 135, "arm_bombers", count=6, rows=2, spacing=150, airborne=True)
    s.unit("CORVENG", 1, 7600, 10500, 315, "core_fighters", count=6, rows=2, spacing=130, airborne=True)
    s.order(4, "arm_bombers", "attack", target="core_towers")
    s.order(4.1, "arm_bombers", "patrol", at=[5800, 9900], queue=True)
    s.order(1.5, "core_fighters", "patrol", at=[5800, 9000])
    # The battle: the Arm advances, the Core's line comes out to meet it, the second wave follows.
    s.order(0.5, ["arm_front", "arm_kbots"], "fight", by=[0, 700], formation=False)
    # The flanks close in on the ends of the Core's line (sent straight on, they drew level with
    # it out of range and stood there on the ridges either side).
    s.order(0.5, "arm_flank_w", "fight", at=[5420, 9470], formation=False)
    s.order(0.5, "arm_flank_e", "fight", at=[6180, 9470], formation=False)
    s.order(9, "core_front", "fight", by=[0, -500], formation=False)
    s.order(18, "arm_wave", "fight", by=[0, 900], formation=False)
    s.event(12, "fire", at=[4950, 8120], size=320)
    s.event(15, "fire", at=[6650, 7900], size=260)
    # The camera: low behind the Arm in the rain, round the western flank, then from behind the
    # Core's towers looking north over the whole battle to the burning woods and the lake beyond
    # (the settings test's and the effects test's seconds, 16 to 22.5: everything at once), and up
    # over the lake.
    s.key(0, (5650, 8000), 24, (5800, 9300), 30, 52)
    s.key(8, (4950, 9050), 90, (5850, 9200), 10, 48)
    s.key(15, (5450, 10450), 240, (5800, 8350), 0, 48)
    s.key(23, (6150, 10350), 270, (5700, 8250), 0, 48)
    s.key(30, (6900, 9500), 520, (5600, 7900), 0, 46)
    s.save("highland", "Highland Rift")


def cinder(heights_dir):
    """The scale trailer's battle (scratch/scale/stages/gen_cinder.py), some 1,800 a side on the
    eastern plain, from dusk into the night, a nuclear blast at the end."""
    rng = random.Random(7)
    heights = np.asarray(Image.open(pathlib.Path(heights_dir) / "Cinder Basin_height.png").convert("L")).astype(int)
    cell = PX / heights.shape[0]

    def level(x0, x1, z0, z1):
        xs = np.clip(np.linspace(x0, x1, 6) // cell, 0, heights.shape[1] - 1).astype(int)
        zs = np.clip(np.linspace(z0, z1, 6) // cell, 0, heights.shape[0] - 1).astype(int)
        h = heights[np.ix_(zs, xs)]
        return h.max() - h.min() <= 6

    pool = (11000, 13600, 8450, 10700)

    def in_pool(x, z):
        return pool[0] - 120 < x < pool[1] + 120 and pool[2] - 120 < z < pool[3] + 120

    s = Scene(HEADER.format(what="Cinder Basin at nightfall", map="Cinder Basin", time="[[0, 17.55], [10, 17.85], [25, 18.6]]", seed=7) + "orders_per_tick = 800\n")
    s.w("[[clear]]\nat = [13700, 4800]\nradius = 4200")
    per_side, z_lo, z_hi, across = 2600, 1500, 8100, 10
    arm = [(["ARMBULL", "ARMSTUMP"], 42, 0.17), (["ARMZEUS", "ARMHAM", "ARMJETH"], 34, 0.22), (["ARMFIDO", "ARMROCK"], 36, 0.16), (["ARMLATNK", "ARMFLASH"], 40, 0.12), (["ARMMART", "ARMMERL"], 46, 0.21), (["ARMSAM", "ARMYORK"], 46, 0.12)]
    core = [(["CORREAP", "CORRAID"], 42, 0.17), (["CORPYRO", "CORTHUD", "CORAK"], 34, 0.22), (["CORMORT", "CORSTORM"], 36, 0.16), (["CORGOL", "CORLEVLR"], 50, 0.12), (["CORMART", "CORVROC"], 46, 0.21), (["CORSENT", "CORMIST"], 46, 0.12)]

    def rank(side, kinds, spacing, share, x_front, sign, heading):
        blocks = int((z_hi - z_lo) / (across * spacing * 1.08))
        rows = max(1, round(per_side * share / (blocks * across)))
        z = z_lo + rng.uniform(0, spacing)
        k = 0
        while z + across * spacing <= z_hi:
            sp = spacing * rng.uniform(0.92, 1.12)
            width = across * sp
            stagger = rng.uniform(-0.6, 0.6) * spacing
            xc = x_front + sign * ((rows - 1) * sp / 2 + stagger)
            zc = z + width / 2
            x_back = xc + sign * rows * sp / 2
            if not in_pool(xc, zc) and not in_pool(x_back, zc) and level(min(xc - sign * rows * sp / 2, x_back), max(xc - sign * rows * sp / 2, x_back), zc - width / 2, zc + width / 2):
                s.unit(kinds[k % len(kinds)], side, xc, zc, heading + rng.uniform(-5, 5), "arm" if side == 0 else "core", count=across * rows, rows=rows, spacing=sp)
            z += width + rng.uniform(0.15, 1.2) * spacing
            k += 1
        return rows * spacing * 1.12 + 30

    for side, ranks, x0, sign, heading in ((0, arm, 13300, -1, 90), (1, core, 14000, 1, 270)):
        x = x0
        for kinds, spacing, share in ranks:
            x += sign * rank(side, kinds, spacing, share, x, sign, heading)
    for i in range(8):
        z = z_lo + 500 + i * (z_hi - z_lo - 1000) / 7
        s.unit("CORKROG", 1, 14000 - 240, z, 270, "krogoths")
    for i in range(12):
        z = z_lo + 300 + i * (z_hi - z_lo - 600) / 11
        s.unit("ARMBRTHA", 0, 11300, z, 90, "berthas")
    s.order(0.5, "arm", "fight", at=[13900, 4800])
    s.order(0.5, "core", "fight", at=[13800, 4800])
    s.order(2, "krogoths", "fight", by=[-2200, 0], formation=False)
    # A nuclear strike on the Core's rear, on cue: the Retaliator's blast, seen and heard (it harms
    # nothing: the battle plays the same whatever the computer).
    s.event(19.5, "explosion", at=[15300, 4800], weapon="NUCLEAR_MISSILE")
    # The camera: high behind the Arm at dusk, down to the clash from the north, south along the
    # front to the lava pool glowing at its end, then back for the strike.
    s.key(0, (12100, 4800), 700, (13700, 4800), 0, 46)
    s.key(5, (12950, 2350), 230, (13650, 4300), 10, 48)
    s.key(10, (13300, 5600), 160, (13600, 8800), 0, 50)
    s.key(14.5, (13900, 6800), 140, (12800, 9300), 0, 50)
    s.key(19, (12600, 4300), 700, (15100, 4900), 0, 46)
    s.key(25, (11500, 4500), 1100, (15200, 4800), 0, 46)
    s.save("cinder", "Cinder Basin")


def heading_to(a, b):
    """TA's heading from `a` to `b` (degrees: 0 north, 90 east)."""
    return math.degrees(math.atan2(b[0] - a[0], -(b[1] - a[1]))) % 360


def cockpit():
    """Greenmantle in the morning: an Arm column comes up the forest road from the south-west
    to the ford, the camera dives into its lead Bulldog and rides it through the trees, across the
    river under the Core's fire and up the far bank; Thunders go over, the Lugers' shells too."""
    s = Scene(HEADER.format(what="Greenmantle, riding in a Stumpy", map="Greenmantle", time="[[0, 8.4], [25, 8.7]]", seed=21))
    # The Arm takes no harm: the tank ridden rides to the end on every computer.
    s.w('[[player]]\nindex = 0\nfaction = "ARM"\ninvulnerable = true')
    ford, beyond = (9358, 9266), (10200, 8300)
    h = heading_to(ford, beyond)
    ux, uz = math.sin(math.radians(h)), -math.cos(math.radians(h))
    along = lambda p, d, side=0.0: (p[0] + ux * d - uz * side, p[1] + uz * d + ux * side)
    # (The road runs a little south-east of the line through the ford: the column on it.)
    start = along(ford, -560, 72)
    rnd = lambda p: [round(p[0]), round(p[1])]
    # The Stumpy ridden in front, two more at its shoulders (as fast: nothing overtakes it), the
    # Bulldogs and the Lugers falling behind.
    s.unit("ARMSTUMP", 0, *start, h, "lead")
    s.order(0.3, "lead", "move", at=rnd(along(ford, 1100)), formation=False)
    for side in (-1, 1):
        s.unit("ARMSTUMP", 0, *along(start, -80, 46 * side), h, "wing")
    s.order(0.3, "wing", "fight", by=[round(ux * 1500), round(uz * 1500)], formation=False)
    for k in range(6):
        s.unit("ARMBULL", 0, *along(start, -190 - 60 * (k // 2), 30 if k % 2 else -30), h, "column")
    s.order(0.3, "column", "fight", by=[round(ux * 1500), round(uz * 1500)], formation=False)
    for k in range(3):
        s.unit("ARMMART", 0, *along(start, -420 - 60 * k), h, "lugers")
    s.order(3, "lugers", "fight", by=[round(ux * 600), round(uz * 600)], formation=False)
    # The Core holds the far bank: a line of tanks before two light towers and a heavy one.
    face = (h + 180) % 360
    s.unit("CORRAID", 1, *along(ford, 400), face, "core_line", count=5, spacing=70)
    s.unit("CORREAP", 1, *along(ford, 480), face, "core_line", count=4, spacing=90)
    for side in (-170, 170):
        s.unit("CORLLT", 1, *along(ford, 620, side), face, "core_towers")
    s.unit("CORHLT", 1, *along(ford, 720), face, "core_towers")
    # Thunders over the column at treetop height, on the towers.
    s.unit("ARMTHUND", 0, *along(start, -1000, 140), h, "thunders", count=5, spacing=60, airborne=True, altitude=150)
    s.order(4, "thunders", "attack", target="core_towers")
    s.order(4.1, "thunders", "patrol", at=rnd(along(ford, 700)), queue=True)
    # The camera: low on the road ahead of the column, looking back at it as it comes up through
    # the trees, then into the lead tank (from 3 s) to the end.
    s.track(0, along(start, 190, 14), 9, "lead", 9, 50)
    s.track(2.5, along(start, 165, 12), 8, "lead", 9, 48)
    s.ride(3, "lead")
    s.save("cockpit", "Greenmantle")


def space():
    """Lagrange Station in the void: the Arm storms the station rock across its western gantry,
    its vanguard already on the terrace under the Core's towers, the column strung out over the
    void, fighters and bombers over it; the planet far below and the stars."""
    s = Scene(HEADER.format(what="Lagrange Station, the void", map="Lagrange Station", time="[[0, 15.6], [25, 15.9]]", seed=31))
    gz = 5116
    # The Arm: a vanguard on the station's western terrace, a column on the gantry, more behind on
    # the western rock; all on to the middle of the station.
    s.unit("ARMZEUS", 0, 3880, gz, 90, "arm", count=6, rows=2, spacing=34)
    s.unit("ARMHAM", 0, 3800, gz, 90, "arm", count=6, rows=2, spacing=30)
    for k, kind in enumerate(["ARMSTUMP", "ARMBULL", "ARMSTUMP", "ARMROCK", "ARMSTUMP", "ARMBULL", "ARMROCK", "ARMSTUMP"]):
        s.unit(kind, 0, 3600 - k * 115, gz + (-26 if k % 2 else 26), 90, "arm")
    s.unit("ARMSTUMP", 0, 2380, gz, 90, "arm", count=6, rows=2, spacing=44)
    s.unit("ARMMART", 0, 2250, gz, 90, "arm", count=4, spacing=60)
    s.order(0.3, "arm", "fight", at=[5100, gz], formation=False)
    # The Core: towers where the gantry comes in, tanks and kbots on the terrace above them.
    s.unit("CORLLT", 1, 3960, gz - 150, 270, "core_towers")
    s.unit("CORLLT", 1, 3960, gz + 150, 270, "core_towers")
    s.unit("CORHLT", 1, 4180, gz, 270, "core_towers")
    s.unit("CORPUN", 1, 4560, gz - 220, 270, "core_towers")
    s.unit("CORREAP", 1, 4350, gz, 270, "core", count=6, spacing=60)
    s.unit("CORRAID", 1, 4450, gz, 270, "core", count=6, spacing=55)
    s.unit("CORAK", 1, 4250, gz, 270, "core", count=10, rows=2, spacing=32)
    s.order(1, "core", "fight", at=[3900, gz], formation=False)
    # The air over the void: fighters at each other, Thunders on the towers, Shadows on the gantry.
    s.unit("ARMHAWK", 0, 2600, gz - 700, 90, "arm_fighters", count=8, rows=2, spacing=120, airborne=True)
    s.unit("CORVENG", 1, 5600, gz + 700, 270, "core_fighters", count=8, rows=2, spacing=120, airborne=True)
    s.order(0.5, "arm_fighters", "patrol", at=[4200, gz])
    s.order(0.5, "core_fighters", "patrol", at=[3600, gz])
    s.unit("ARMTHUND", 0, 1600, gz - 300, 90, "arm_bombers", count=6, rows=2, spacing=140, airborne=True)
    s.order(3, "arm_bombers", "attack", target="core_towers")
    s.order(3.1, "arm_bombers", "patrol", at=[4400, gz], queue=True)
    s.unit("CORSHAD", 1, 6800, gz + 400, 270, "core_bombers", count=4, rows=2, spacing=140, airborne=True)
    s.order(5, "core_bombers", "attack", target="arm")
    s.order(5.1, "core_bombers", "patrol", at=[3500, gz], queue=True)
    # The camera: over the void beside the gantry, looking along it at the station; gliding in, up
    # over the terrace's fight, and away up over the whole station, the rocks round it and the
    # planet below.
    s.key(0, (2700, gz + 260), 112, (3900, gz - 40), 100, 50)
    s.key(7, (3250, gz + 330), 120, (4200, gz - 60), 120, 50)
    s.key(14, (3600, gz + 520), 210, (4300, gz - 80), 120, 48)
    s.key(20, (3300, gz + 1300), 700, (4700, gz - 300), 100, 46)
    s.key(25, (2900, gz + 2600), 1500, (5100, 5100), 60, 46)
    s.save("space", "Lagrange Station")


def build():
    """Redstone Mesas at sunset: a base rising on the desert pavement under the mesas. The
    Commander rises out of its lattice and lathes a vehicle plant; constructors put extractors on
    the deposits, solar collectors, towers and a fusion plant up; a construction aircraft builds
    the radar from the air; the kbot lab sends Peewees out."""
    s = Scene(HEADER.format(what="Redstone Mesas, a base rising", map="Redstone Mesas", time="[[0, 17.55], [25, 17.9]]", seed=41))
    # Close together, so that the camera among them sees every lattice and beam at once.
    # The Commander arrives, then lathes the vehicle plant beside it.
    s.unit("ARMCOM", 0, 4940, 2950, 270, "com", built=0.0, build_seconds=2.8, rise_at=0.3)
    s.order(3.4, "com", "build", at=[4810, 2990], type="ARMVP", seconds=14, label="plant")
    # The fusion plant, the advanced constructor's, north of it; solar collectors to the south,
    # unfolding as each is done; a tower and a Guardian to the east.
    s.unit("ARMACK", 0, 5150, 2860, 250, "ack")
    s.unit("ARMACK", 0, 5000, 2860, 70, "ack")
    s.unit("ARMFUS", 0, 5080, 2800, 180, "fusion", built=0.05, builder="ack", build_seconds=18)
    s.unit("ARMCK", 0, 4880, 3110, 330, "ck3")
    s.order(4.5, "ck3", "repair", target="plant")
    s.unit("ARMCK", 0, 5150, 3020, 200, "ck1")
    s.unit("ARMSOLAR", 0, 5110, 3100, 180, "solar", built=0.2, builder="ck1", build_seconds=6)
    s.order(6.5, "ck1", "build", at=[5200, 3100], type="ARMSOLAR", seconds=6, label="solar")
    s.order(13, "ck1", "build", at=[5020, 3180], type="ARMSOLAR", seconds=6, label="solar")
    s.unit("ARMCK", 0, 5190, 2940, 90, "ck2")
    s.unit("ARMLLT", 0, 5250, 2890, 90, "towers", built=0.2, builder="ck2", build_seconds=7)
    s.order(8, "ck2", "build", at=[5300, 2990], type="ARMGUARD", seconds=10, label="towers")
    # Extractors on the deposits round it.
    s.unit("ARMCV", 0, 5220, 2790, 30, "cv1")
    s.unit("ARMMEX", 0, 5272, 2728, 180, "mex", built=0.1, builder="cv1", build_seconds=7)
    s.order(8, "cv1", "build", at=[4808, 2616], type="ARMMEX", seconds=7, label="mex")
    s.unit("ARMCV", 0, 5250, 3040, 150, "cv2")
    s.unit("ARMMEX", 0, 5304, 3112, 180, "mex", built=0.05, builder="cv2", build_seconds=8)
    s.order(9, "cv2", "build", at=[5112, 3272], type="ARMMEX", seconds=7, label="mex")
    # The radar, from the air; then solar collectors and towers to the north-west through to the
    # scene's end (each a few seconds' work for it), and the Commander to guard after (with
    # nothing to do it patrolled to and fro, doing nothing; idle, it would land).
    s.unit("ARMCA", 0, 4900, 2800, 90, "ca", airborne=True, altitude=70)
    s.unit("ARMRAD", 0, 4950, 2730, 180, "radar", built=0.1, builder="ca", build_seconds=9)
    for k, (kind, at) in enumerate([("ARMSOLAR", [4840, 2770]), ("ARMRL", [5020, 2680]), ("ARMSOLAR", [4740, 2850]), ("ARMLLT", [4950, 2640]), ("ARMSOLAR", [4880, 2620])]):
        s.order(0.5 + 0.1 * k, "ca", "build", at=at, type=kind, label="ca_builds", queue=True)
    s.order(1.1, "ca", "guard", target="com", queue=True)
    # The kbot lab, finished, makes Peewees; they gather east of the base.
    s.unit("ARMLAB", 0, 5390, 3270, 180, "lab")
    s.order(1, "lab", "produce", type="ARMPW", count=6, seconds=2.6)
    s.order(1, "lab", "move", at=[5470, 3000])
    # The camera: low east of the Commander as it rises against the setting sun; round to the
    # south-east to see its beam on the vehicle plant's lattice; past the solar collectors and the
    # fusion plant's lattice; then up over the whole base, looking east to the mesas the sun lights.
    s.key(0, (5110, 2930), 12, (4940, 2950), 24, 50)
    s.key(3.5, (5010, 3110), 20, (4870, 2985), 14, 50)
    s.key(9, (5200, 3240), 18, (5130, 3070), 15, 50)
    s.key(15, (4960, 3250), 30, (5080, 2830), 20, 48)
    s.key(25, (4640, 3460), 240, (5360, 2870), 0, 46)
    s.save("build", "Redstone Mesas")


def swarm(heights_dir, per_side=10_000):
    """Reactor Grid at dusk, filled edge to edge: the Arm's ten thousand on the west, the Core's on
    the east, in contact along a wavy front through the middle and every unit attack-moving across
    (the processor's test: the largest battle). The camera rises from the front to the whole map,
    where every unit becomes an icon."""
    rng = random.Random(1997)
    heights = np.asarray(Image.open(pathlib.Path(heights_dir) / "Reactor Grid_height.png").convert("L")).astype(int)
    cells = heights.shape[0]
    size = cells * 16
    s = Scene(HEADER.format(what="Reactor Grid, twenty thousand", map="Reactor Grid", time="[[0, 17.95], [30, 18.6]]", seed=51).replace("icons = false", "icons = true") + "orders_per_tick = 2500\n")
    across, rows, sp = 4, 2, 40.0
    bw, bd = across * sp, rows * sp

    def level(x, z, heading):
        """The block's ground: one deck (60 or 92), not by its edge."""
        w, d = (bd, bw) if heading in (90, 270) else (bw, bd)
        x0, x1 = int((x - w / 2) // 16) - 1, int((x + w / 2) // 16) + 1
        z0, z1 = int((z - d / 2) // 16) - 1, int((z + d / 2) // 16) + 1
        if x0 < 2 or z0 < 2 or x1 >= cells - 2 or z1 >= cells - 2:
            return False
        block = heights[z0:z1 + 1, x0:x1 + 1]
        return block.max() - block.min() <= 2 and int(block.mean()) in range(55, 98)

    def front(z):
        return size / 2 + 260 * math.sin(z / 1500.0) + 140 * math.sin(z / 610.0 + 1.3)

    tiers = {
        0: [(0.30, ["ARMPW", "ARMFLASH", "ARMHAM", "ARMSTUMP", "ARMWAR"]), (0.65, ["ARMSTUMP", "ARMROCK", "ARMHAM", "ARMBULL", "ARMZEUS"]), (1.01, ["ARMSAM", "ARMMART", "ARMBULL", "ARMMERL"])],
        1: [(0.30, ["CORAK", "CORGATOR", "CORTHUD", "CORRAID", "CORPYRO"]), (0.65, ["CORRAID", "CORSTORM", "CORTHUD", "CORREAP", "CORCAN"]), (1.01, ["CORMIST", "CORMART", "CORREAP", "CORVROC"])],
    }
    for side, heading in ((0, 90), (1, 270)):
        spots = []
        for z in np.arange(bw / 2 + 64, size - bw / 2 - 64, bw + 14):
            for x in np.arange(bd / 2 + 64, size - bd / 2 - 64, bd + 12):
                d = x - front(z)
                if (d < -70 if side == 0 else d > 70) and level(x, z, heading):
                    spots.append((x + rng.uniform(-8, 8), z + rng.uniform(-8, 8), abs(d)))
        rng.shuffle(spots)
        need = per_side
        for x, z, d in spots:
            if need <= 0:
                break
            depth = min(d / (size / 2), 1.0)
            kinds = next(k for edge, k in tiers[side] if depth <= edge)
            n = min(across * rows, need)
            s.unit(rng.choice(kinds), side, x, z, heading + rng.uniform(-6, 6), "arm" if side == 0 else "core", count=n, rows=rows if n == across * rows else 1, spacing=sp)
            need -= n
        if need > 0:
            print(f"swarm: side {side} short by {need}")
    s.order(0.4, "arm", "fight", by=[7000, 0], formation=False)
    s.order(0.4, "core", "fight", by=[-7000, 0], formation=False)
    # The camera: low at the front in the middle, looking along it as the lines meet; rising over
    # it, then up and back until the whole map is in view, every unit an icon.
    mid = size / 2
    s.key(0, (front(mid - 900), mid - 900), 28, (front(mid - 100), mid - 100), 18, 50)
    s.key(7, (front(mid - 700) - 80, mid - 700), 110, (front(mid + 300), mid + 300), 10, 48)
    s.key(15, (mid - 1600, mid - 2200), 900, (mid, mid), 0, 46)
    s.key(23, (mid - 1900, mid + 4300), 3800, (mid, mid - 200), 0, 48)
    s.key(30, (mid - 1500, mid + 6200), 6200, (mid, mid - 500), 0, 50)
    s.save("swarm", "Reactor Grid")


def winter():
    """Midnight Square, a minute to midnight: the Core storms the town from the north, Pyros'
    flames lighting the snow, the Arm holding the square round the clock tower; gunships over the
    roofs, their beacons blinking; the town's fireworks going up all round."""
    s = Scene(HEADER.format(what="Midnight Square at midnight", map="Midnight Square", time="[[0, 23.9], [25, 23.99]]", seed=61) + "moon = [200, 34]\n")
    tower = (4072, 4072)
    # The Arm: the square's defenders round the tower and in the street to the north of it.
    s.unit("ARMZEUS", 0, 4072, 3880, 0, "arm", count=5, spacing=38)
    s.unit("ARMHAM", 0, 4072, 3930, 0, "arm", count=7, spacing=30)
    s.unit("ARMWAR", 0, 3900, 3950, 20, "arm", count=4, spacing=36)
    s.unit("ARMSTUMP", 0, 4250, 3960, 340, "arm", count=4, spacing=40)
    s.unit("ARMLLT", 0, 3880, 3880, 0, "arm_towers")
    s.unit("ARMLLT", 0, 4290, 3870, 0, "arm_towers")
    s.unit("ARMHLT", 0, 4072, 3790, 0, "arm_towers")
    # The Core comes down from the north, its flamethrowers in front.
    s.unit("CORPYRO", 1, 4072, 3420, 180, "core", count=8, rows=2, spacing=34)
    s.unit("CORAK", 1, 4072, 3340, 180, "core", count=12, rows=2, spacing=30)
    s.unit("CORTHUD", 1, 4000, 3260, 180, "core", count=6, spacing=38)
    s.unit("CORSTORM", 1, 4160, 3250, 180, "core", count=6, spacing=36)
    s.unit("CORRAID", 1, 4072, 3160, 180, "core", count=6, spacing=44)
    s.order(0.5, "core", "fight", at=[4072, 4100], formation=False)
    s.order(6, "arm", "fight", at=[4072, 3500], formation=False)
    # Gunships over the roofs: the Arm's Brawlers on patrol, the Core's Rapiers coming in.
    s.unit("ARMBRAWL", 0, 4072, 4300, 0, "arm_gunships", count=4, spacing=90, airborne=True, altitude=110)
    s.order(0.3, "arm_gunships", "patrol", at=[4072, 3600])
    s.unit("CORAPE", 1, 4072, 2700, 180, "core_gunships", count=4, spacing=90, airborne=True, altitude=120)
    s.order(1.5, "core_gunships", "fight", at=[4072, 4000])
    # Midnight: every rack round the square sends its shells up, one after another.
    s.event(15.5, "fireworks", at=list(tower), size=700, seconds=9.5)
    # The camera: low in the square south of the clock tower, looking up at it and the fireworks;
    # past the tower into the street fight to the north; up over the roofs, the fireworks at eye
    # level; and away up over the town.
    s.key(0, (4040, 4330), 10, (4072, 4072), 70, 52)
    s.key(5, (4200, 4300), 26, (4072, 3950), 40, 52)
    s.key(11, (4380, 4020), 70, (4060, 3620), 10, 50)
    s.key(18, (4300, 4250), 200, (4072, 3850), 60, 48)
    s.key(25, (4620, 4820), 480, (4072, 3900), 120, 46)
    s.save("winter", "Midnight Square")


SCENES = {"sapphire": sapphire, "cockpit": cockpit, "highland": highland, "space": space, "build": build, "swarm": swarm, "cinder": cinder, "winter": winter}

if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--heights", default=str(ROOT / "scratch" / "bench" / "maps"))
    ap.add_argument("only", nargs="*", help="the scenes to write (all when none)")
    a = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    for name in a.only or list(SCENES):
        if name in ("cinder", "swarm"):
            SCENES[name](a.heights)
        else:
            SCENES[name]()
