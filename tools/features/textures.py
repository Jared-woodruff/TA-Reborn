"""Draws the textures of TA Reborn's own trees, bushes and rocks (original art): the leaf
clusters on the foliage's cards, the twigs of burnt trees, bark and rock.

Each is a 512x512 RGBA PNG (sRGB). The cut-out ones (leaves, needles, twigs) are drawn four
times larger and averaged down, so their alpha is the coverage of each texel; the colour of the
empty texels is carried out from the edges, so filtering never pulls in black. Bark and rock tile.
The colours are plain middling ones: the renderer tints each tree and rock towards the colours of
the TA feature it stands in for (`ta_render::foliage`).

    python tools/features/textures.py [--out crates/ta_render/features] [--preview <dir>]

The order of LAYERS is the order of the texture array's layers; the models' materials are named
after them (`tools/features/build.py`).
"""
import argparse
import os
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "terrain"))
import detail  # noqa: E402  (its periodic noise, for the bark and the rock)
import festive_textures  # noqa: E402

N = 512
SS = 4
TAU = 2.0 * np.pi


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


def srgb(c):
    """An sRGB colour (0..255) as 0..1."""
    return np.array(c, dtype=np.float64) / 255.0


class Canvas:
    """A supersampled cut-out: colour, coverage, and the height (drawing order) of what is on top."""

    def __init__(self):
        self.n = N * SS
        self.rgb = np.zeros((self.n, self.n, 3))
        self.a = np.zeros((self.n, self.n))
        self.h = np.zeros((self.n, self.n))
        self.order = 0.0

    def region(self, pts, pad):
        """The pixel rectangle around points (card units, 0..1) with `pad` (card units) to spare."""
        p = np.asarray(pts) * self.n
        x0 = int(max(0, np.floor(p[:, 0].min() - pad * self.n)))
        x1 = int(min(self.n, np.ceil(p[:, 0].max() + pad * self.n) + 1))
        y0 = int(max(0, np.floor(p[:, 1].min() - pad * self.n)))
        y1 = int(min(self.n, np.ceil(p[:, 1].max() + pad * self.n) + 1))
        if x1 <= x0 or y1 <= y0:
            return None
        ys, xs = np.mgrid[y0:y1, x0:x1].astype(np.float64)
        return (y0, y1, x0, x1), (xs + 0.5) / self.n, (ys + 0.5) / self.n

    def put(self, box, inside, rgb):
        """Paints `rgb` (h, w, 3) where `inside`, over what is there."""
        y0, y1, x0, x1 = box
        self.order += 1.0
        sub = (slice(y0, y1), slice(x0, x1))
        self.rgb[sub] = np.where(inside[..., None], rgb, self.rgb[sub])
        self.a[sub] = np.where(inside, 1.0, self.a[sub])
        self.h[sub] = np.where(inside, self.order, self.h[sub])

    def stem(self, points, widths, color, rng, bark=0.12):
        """A stem along `points` (card units), `widths` (card units) at each, as capsules."""
        for i in range(len(points) - 1):
            a, b = np.asarray(points[i], float), np.asarray(points[i + 1], float)
            wa, wb = widths[i], widths[i + 1]
            r = self.region([a, b], max(wa, wb))
            if r is None:
                continue
            box, xs, ys = r
            d = b - a
            length2 = max(d @ d, 1e-12)
            t = np.clip(((xs - a[0]) * d[0] + (ys - a[1]) * d[1]) / length2, 0.0, 1.0)
            dist = np.hypot(xs - (a[0] + d[0] * t), ys - (a[1] + d[1] * t))
            w = wa + (wb - wa) * t
            inside = dist <= w
            # Round: lit along one side, shaded along the other.
            side = ((xs - (a[0] + d[0] * t)) * -d[1] + (ys - (a[1] + d[1] * t)) * d[0]) / np.sqrt(length2) / np.maximum(w, 1e-6)
            shade = 1.0 + 0.25 * side - 0.25 * side * side
            grain = 1.0 + bark * rng.standard_normal(inside.shape) * 0.5
            self.put(box, inside, np.clip(color[None, None, :] * (shade * grain)[..., None], 0, 1))

    def leaf(self, base, tip, width, color, rng, shape=0.8, bend=0.0, vein=0.18, fold=0.16, side_veins=0.0, serrate=0.0):
        """A leaf from `base` to `tip` (card units), `width` its widest half-width (card units):
        a midrib, lit on one half and shaded on the other as if folded along it, its edge a
        little darker. `shape` < 1 is widest nearer the base; `bend` curls its midrib."""
        b, t = np.asarray(base, float), np.asarray(tip, float)
        d = t - b
        length = np.hypot(*d)
        if length < 1e-6:
            return
        along, perp = d / length, np.array([-d[1], d[0]]) / length
        r = self.region([b, t], width + abs(bend) * length)
        if r is None:
            return
        box, xs, ys = r
        px, py = xs - b[0], ys - b[1]
        u = (px * along[0] + py * along[1]) / length
        v = px * perp[0] + py * perp[1] - bend * length * u * u
        uc = np.clip(u, 0.0, 1.0)
        half = width * np.sin(np.pi * uc ** shape) ** 0.85
        if serrate > 0.0:
            half *= 1.0 - serrate * (0.5 + 0.5 * np.sin(uc * length * N * 0.9))
        inside = (u >= 0.0) & (u <= 1.0) & (np.abs(v) <= half)
        if not inside.any():
            return
        s = v / np.maximum(half, 1e-6)
        rib = np.exp(-(v / (0.0035 + 0.04 * width)) ** 2) * (1.0 - uc * 0.7)
        shade = 1.0 + fold * np.sign(s) * np.sqrt(np.abs(s)) - 0.12 * smoothstep(0.7, 1.0, np.abs(s)) + vein * rib
        if side_veins > 0.0:
            # Veins running out from the midrib towards the tip.
            phase = (uc * 9.0 - np.abs(s) * 1.6) % 1.0
            shade += side_veins * np.exp(-((phase - 0.5) / 0.06) ** 2) * (np.abs(s) > 0.12)
        shade *= 0.9 + 0.2 * uc
        self.put(box, inside, np.clip(color[None, None, :] * shade[..., None], 0, 1))

    def finish(self, inner=None, inner_dark=0.3, occlusion=0.45):
        """The averaged-down texture (RGBA 0..1): leaves under others darker near their edges, the
        middle of the cluster (`inner`, card units) darker, the empty texels' colour carried out."""
        h = self.h / max(self.order, 1.0)
        # Shadowing by what lies over: the mean height around, above this texel's own.
        around = h * self.a
        weight = self.a.copy()
        for _ in range(3):
            around = blur_once(around, 4 * SS)
            weight = blur_once(weight, 4 * SS)
        over = np.clip(around / np.maximum(weight, 1e-6) - h, 0.0, None)
        shade = 1.0 - occlusion * smoothstep(0.0, 0.25, over)
        if inner is not None:
            ys, xs = np.mgrid[0:self.n, 0:self.n].astype(np.float64) / self.n
            dist = np.hypot(xs - inner[0], ys - inner[1])
            shade *= 1.0 - inner_dark * (1.0 - smoothstep(0.0, 0.55, dist))
        rgb = self.rgb * shade[..., None]
        # Average down, colour weighted by coverage.
        a = self.a.reshape(N, SS, N, SS).mean(axis=(1, 3))
        col = (rgb * self.a[..., None]).reshape(N, SS, N, SS, 3).sum(axis=(1, 3)) / np.maximum(self.a.reshape(N, SS, N, SS).sum(axis=(1, 3)), 1e-6)[..., None]
        return np.dstack([bleed(col, a), a])


def blur_once(img, r):
    """A box blur of radius `r` texels (not wrapping)."""
    k = 2 * r + 1
    c = np.cumsum(np.pad(img, ((r + 1, r), (0, 0)), mode="edge"), axis=0)
    img = (c[k:] - c[:-k]) / k
    c = np.cumsum(np.pad(img, ((0, 0), (r + 1, r)), mode="edge"), axis=1)
    return (c[:, k:] - c[:, :-k]) / k


def bleed(rgb, a):
    """Carries the colours of covered texels out into the empty ones round them."""
    rgb = rgb.copy()
    have = a > 0.02
    for _ in range(64):
        if have.all():
            break
        acc = np.zeros_like(rgb)
        cnt = np.zeros(a.shape)
        for dy, dx in ((1, 0), (-1, 0), (0, 1), (0, -1)):
            m = np.roll(have, (dy, dx), (0, 1))
            acc += np.roll(rgb, (dy, dx), (0, 1)) * m[..., None]
            cnt += m
        grow = ~have & (cnt > 0)
        rgb[grow] = acc[grow] / cnt[grow][:, None]
        have |= grow
    if not have.all():
        rgb[~have] = rgb[have].mean(axis=0)
    return rgb


def vary(color, rng, light=0.18, warm=0.06):
    """A colour a little lighter or darker, warmer or cooler."""
    c = np.asarray(color, float) * rng.uniform(1.0 - light, 1.0 + light)
    w = rng.uniform(-warm, warm)
    return np.clip(c + np.array([w, w * 0.5, -w]), 0, 1)


def bezier(p0, p1, p2, n):
    t = np.linspace(0.0, 1.0, n)[:, None]
    return (1 - t) ** 2 * np.asarray(p0) + 2 * (1 - t) * t * np.asarray(p1) + t * t * np.asarray(p2)


# ---------------------------------------------------------------------------------------------
# The cut-out layers. Unless said otherwise a cluster grows from the middle of the card's bottom
# edge (where it sits on its twig) upwards.
# ---------------------------------------------------------------------------------------------

TWIG = srgb((92, 74, 56))
LEAF = srgb((86, 128, 52))


def broadleaf():
    """A spray of broad leaves on forking twigs: the crowns of broadleaf trees."""
    rng = np.random.default_rng(101)
    c = Canvas()
    tips = []
    main = bezier((0.5, 1.0), (0.47, 0.6), (0.52, 0.18), 9)
    c.stem(main, np.linspace(0.016, 0.005, 9), TWIG, rng)
    for k in range(9):
        t = 0.2 + k * 0.085
        at = main[min(int(t * 8), 8)]
        side = -1 if k % 2 else 1
        end = at + np.array([side * rng.uniform(0.2, 0.34), -rng.uniform(0.08, 0.22)])
        mid = (at + end) / 2 + np.array([0.0, -0.05])
        twig = bezier(at, mid, end, 6)
        c.stem(twig, np.linspace(0.008, 0.003, 6), TWIG, rng)
        tips.append(twig)
    tips.append(main[3:])
    leaves = []
    for twig in tips:
        for i in range(1, len(twig)):
            for _ in range(3):
                p = twig[i]
                d = twig[min(i + 1, len(twig) - 1)] - twig[i - 1]
                ang = np.arctan2(d[1], d[0]) + rng.uniform(-1.1, 1.1)
                length = rng.uniform(0.11, 0.17)
                tip = p + length * np.array([np.cos(ang), np.sin(ang)])
                leaves.append((p, tip, length * rng.uniform(0.3, 0.38)))
    rng.shuffle(leaves)
    for base, tip, width in leaves:
        if not (0.02 < tip[0] < 0.98 and 0.02 < tip[1] < 0.98):
            continue
        c.leaf(base, tip, width, vary(LEAF, rng), rng, shape=0.75, bend=rng.uniform(-0.12, 0.12), side_veins=0.05)
    return c.finish(inner=(0.5, 0.75))


def needles():
    """A conifer's spray: a twig with side shoots, dense with short needles: firs and cypresses."""
    rng = np.random.default_rng(202)
    c = Canvas()
    green = srgb((58, 98, 60))
    shoots = []
    main = bezier((0.5, 1.0), (0.5, 0.55), (0.5, 0.06), 12)
    shoots.append((main, 0.012))
    for k in range(9):
        t = 0.12 + k * 0.085
        at = main[min(int(t * 11), 11)]
        for side in (-1, 1):
            reach = (0.36 - 0.025 * k) * rng.uniform(0.85, 1.1)
            end = at + np.array([side * reach, -reach * 0.55])
            shoot = bezier(at, (at + end) / 2 + np.array([0.0, -0.02]), end, 6)
            shoots.append((shoot, 0.006))
            for j in (2, 4):
                s = shoot[j]
                e = s + np.array([side * reach * 0.3, -reach * 0.35])
                shoots.append((bezier(s, (s + e) / 2, e, 4), 0.004))
    for shoot, w in shoots:
        c.stem(shoot, np.linspace(w, w * 0.4, len(shoot)), TWIG, rng)
    for shoot, _ in shoots:
        for i in range(len(shoot) - 1):
            a, b = shoot[i], shoot[i + 1]
            d = b - a
            seg = np.hypot(*d)
            along = d / max(seg, 1e-9)
            count = max(int(seg / 0.0062), 1)
            for j in range(count):
                p = a + d * (j + rng.random()) / count
                for side in (-1, 1):
                    ang = np.arctan2(along[1], along[0]) + side * rng.uniform(0.7, 1.2)
                    length = rng.uniform(0.04, 0.062)
                    tip = p + length * np.array([np.cos(ang), np.sin(ang)])
                    # Young tips lighter: the sprays' ends catch the light.
                    young = smoothstep(0.55, 0.95, np.hypot(*(p - np.array([0.5, 1.0]))))
                    c.leaf(p, tip, 0.0048, vary(green * (1.0 + 0.35 * young), rng, light=0.22, warm=0.03), rng, shape=0.5, vein=0.0, fold=0.25)
    return c.finish(inner=(0.5, 0.7), inner_dark=0.25)


def jungle():
    """Hanging strands of narrow leaves from a twig along the card's top: the drooping crowns of
    jungle trees. (It hangs from the middle of its top edge.)"""
    rng = np.random.default_rng(303)
    c = Canvas()
    green = srgb((104, 136, 50))
    top = bezier((0.08, 0.07), (0.5, 0.03), (0.92, 0.075), 10)
    c.stem(top, np.full(10, 0.008), TWIG, rng)
    strands = []
    for k in range(15):
        x = 0.12 + k * 0.054 + rng.uniform(-0.015, 0.015)
        start = np.array([x, 0.06 + rng.uniform(0.0, 0.015)])
        length = rng.uniform(0.55, 0.92)
        sway = rng.uniform(-0.08, 0.08)
        strand = bezier(start, start + np.array([sway, length * 0.5]), start + np.array([sway * 0.4, length]), 14)
        c.stem(strand, np.linspace(0.004, 0.0015, 14), TWIG * 0.9, rng)
        strands.append(strand)
    for strand in strands:
        for i in range(1, len(strand)):
            p = strand[i]
            for side in (-1, 1):
                if rng.random() < 0.15:
                    continue
                ang = np.pi / 2 + side * rng.uniform(0.25, 0.65)
                length = rng.uniform(0.07, 0.11)
                tip = p + length * np.array([np.cos(ang), np.sin(ang)])
                c.leaf(p, tip, length * 0.17, vary(green, rng, light=0.2), rng, shape=0.6, bend=side * 0.1, fold=0.12)
    return c.finish(inner=(0.5, 0.0), inner_dark=0.2)


def tropical():
    """A rosette of big broad leaves with veins: tropical trees and the pads of tuber trees."""
    rng = np.random.default_rng(404)
    c = Canvas()
    green = srgb((70, 120, 46))
    base = np.array([0.5, 0.96])
    angles = np.linspace(-1.25, 1.25, 7) + rng.uniform(-0.08, 0.08, 7)
    order = np.argsort(-np.abs(angles))
    for i in order:
        ang = -np.pi / 2 + angles[i]
        length = rng.uniform(0.62, 0.8) * (1.0 - 0.18 * abs(angles[i]))
        tip = base + length * np.array([np.cos(ang), np.sin(ang)])
        stalk = base + 0.16 * length * np.array([np.cos(ang), np.sin(ang)])
        c.stem([base, stalk], [0.008, 0.006], TWIG * 1.1, rng)
        c.leaf(stalk, tip, length * rng.uniform(0.19, 0.24), vary(green, rng, light=0.12), rng, shape=0.9,
               bend=rng.uniform(-0.08, 0.08), vein=0.25, fold=0.12, side_veins=0.12)
    return c.finish(inner=(0.5, 0.95), inner_dark=0.25, occlusion=0.5)


def smallleaf():
    """Many small round leaves on fine twigs, a dense ball: bushes and shrubs."""
    rng = np.random.default_rng(505)
    c = Canvas()
    green = srgb((80, 126, 50))
    twigs = []
    for k in range(9):
        ang = -np.pi / 2 + (k - 4) * 0.27 + rng.uniform(-0.08, 0.08)
        end = np.array([0.5, 0.98]) + rng.uniform(0.42, 0.55) * np.array([np.cos(ang) * 0.85, np.sin(ang)])
        twig = bezier((0.5, 0.98), (0.5 + (end[0] - 0.5) * 0.3, 0.7), end, 8)
        c.stem(twig, np.linspace(0.006, 0.0025, 8), TWIG, rng)
        twigs.append(twig)
    leaves = []
    for twig in twigs:
        for i in range(2, len(twig)):
            for _ in range(5):
                p = twig[i] + rng.uniform(-0.03, 0.03, 2)
                ang = rng.uniform(0, TAU)
                length = rng.uniform(0.05, 0.075)
                leaves.append((p, p + length * np.array([np.cos(ang), np.sin(ang)]), length * 0.42))
    rng.shuffle(leaves)
    for base, tip, width in leaves:
        c.leaf(base, tip, width, vary(green, rng, light=0.2, warm=0.05), rng, shape=0.9, fold=0.2, vein=0.1)
    return c.finish(inner=(0.5, 0.7), inner_dark=0.3)


def clump():
    """A dense ball of small leaves, no gaps: the far trees' crowns (fewer, bigger cards)."""
    rng = np.random.default_rng(707)
    c = Canvas()
    green = srgb((76, 118, 48))
    # A lobed outline, so the cards do not read as discs.
    phases = rng.uniform(0, TAU, 4)
    reach = lambda a: 0.46 * (0.8 + 0.08 * np.sin(3 * a + phases[0]) + 0.06 * np.sin(5 * a + phases[1]) + 0.05 * np.sin(7 * a + phases[2]) + 0.03 * np.sin(11 * a + phases[3]))
    leaves = []
    for _ in range(1400):
        a = rng.uniform(0, TAU)
        r = reach(a) * np.sqrt(rng.random()) ** 0.8
        p = np.array([0.5, 0.5]) + r * np.array([np.cos(a), np.sin(a)])
        ang = a + rng.uniform(-1.2, 1.2)
        length = rng.uniform(0.045, 0.07)
        leaves.append((r, p, p + length * np.array([np.cos(ang), np.sin(ang)]), length * 0.42))
    # The inner leaves first: the outer ones lie over them.
    leaves.sort(key=lambda l: l[0])
    for _, base, tip, width in leaves:
        c.leaf(base, tip, width, vary(green, rng, light=0.22, warm=0.05), rng, shape=0.85, fold=0.2, vein=0.08)
    return c.finish(inner=(0.5, 0.5), inner_dark=0.35, occlusion=0.35)


def twigs():
    """Bare forking twigs, charred: the crowns of burnt trees."""
    rng = np.random.default_rng(606)
    c = Canvas()
    dark = srgb((54, 48, 44))

    def grow(p, ang, length, width, depth):
        end = p + length * np.array([np.cos(ang), np.sin(ang)])
        mid = (p + end) / 2 + rng.uniform(-0.015, 0.015, 2)
        c.stem(bezier(p, mid, end, 5), np.linspace(width, width * 0.72, 5), vary(dark, rng, light=0.25, warm=0.02), rng, bark=0.2)
        if depth == 0:
            return
        for k in range(2 if depth > 1 else 3):
            spread = rng.uniform(0.3, 0.7) * (1 if k % 2 else -1)
            grow(end, ang + spread, length * rng.uniform(0.68, 0.85), width * 0.75, depth - 1)

    # (Few, bold strokes: the finest some six texels wide, so a crown far off is a fine lattice
    # of twigs, not loose pixels where thin ones cross.)
    base = np.array([0.5, 1.0])
    for k, ang in enumerate((-2.25, -1.85, -1.57, -1.3, -0.9)):
        grow(base, ang + rng.uniform(-0.08, 0.08), rng.uniform(0.17, 0.22), 0.015, 3)
    return c.finish(inner=None, occlusion=0.2)


# ---------------------------------------------------------------------------------------------
# The tiling layers (opaque)
# ---------------------------------------------------------------------------------------------


def lit(height, color, light=0.35, gain=6.0):
    """Colour lit from the upper left by the height's slopes (a little baked relief)."""
    gx = (np.roll(height, -1, 1) - np.roll(height, 1, 1)) * gain
    gy = (np.roll(height, -1, 0) - np.roll(height, 1, 0)) * gain
    shade = 1.0 + light * np.clip(-gx * 0.7 - gy * 0.5, -1.0, 1.0)
    return np.clip(color * shade[..., None], 0, 1)


def stretched_cells(gx, gy, seed, jitter=0.8):
    """Periodic cellular noise on a `gx` x `gy` lattice (cells taller than wide when gx > gy): the
    distances (in cells) to the nearest and second nearest points, and the nearest's index."""
    rng = np.random.default_rng(seed)
    off = 0.5 + (rng.random((gy, gx, 2)) - 0.5) * jitter
    ys, xs = np.mgrid[0:N, 0:N].astype(np.float64)
    xs, ys = xs * gx / N, ys * gy / N
    cx, cy = np.floor(xs).astype(int), np.floor(ys).astype(int)
    f1 = np.full((N, N), 1e9)
    f2 = np.full((N, N), 1e9)
    idx = np.zeros((N, N), dtype=int)
    for oy in (-1, 0, 1):
        for ox in (-1, 0, 1):
            nx, ny = (cx + ox) % gx, (cy + oy) % gy
            d = np.hypot(xs - (cx + ox + off[ny, nx, 0]), ys - (cy + oy + off[ny, nx, 1]))
            closer = d < f1
            f2 = np.where(closer, f1, np.minimum(f2, d))
            idx = np.where(closer, ny * gx + nx, idx)
            f1 = np.where(closer, d, f1)
    return f1, f2, idx


def bark():
    """Rough bark: long plates up the trunk split by deep fissures, fibrous, a little lichen."""
    xs, ys = detail.coords()
    warp = detail.spectral(3.0, lo=1, hi=8, seed=701) * 0.6
    f1, f2, idx = stretched_cells(11, 3, seed=702)
    fissure = 1.0 - smoothstep(0.0, 0.22 + 0.1 * warp, f2 - f1)
    rng = np.random.default_rng(703)
    lift = rng.uniform(0.7, 1.0, 33)[idx]
    fibre = detail.spectral(1.2, lo=40, hi=256, seed=704, aniso=(0.12, 1.0))
    rough = detail.spectral(2.0, lo=8, hi=128, seed=705)
    height = (1.0 - fissure) * lift * (0.75 + 0.25 * (1.0 - f1)) + fibre * 0.05 + rough * 0.04
    tone = detail.norm01(detail.spectral(2.5, lo=1, hi=12, seed=706))
    base = srgb((122, 106, 90)) * (0.85 + 0.3 * tone[..., None]) * (0.9 + 0.2 * lift[..., None])
    color = base * (1.0 - 0.6 * fissure)[..., None] * (1.0 + fibre[..., None] * 0.06)
    lichen = smoothstep(0.66, 0.8, detail.norm01(detail.spectral(2.8, lo=3, hi=20, seed=707))) * (1.0 - fissure)
    color = color * (1.0 - 0.3 * lichen[..., None]) + srgb((132, 134, 112)) * 0.3 * lichen[..., None]
    return np.dstack([lit(height, color, 0.35, 3.0), np.ones((N, N))])


def bark_smooth():
    """Smooth bark: grey-brown, banded across, with small dark pores: jungle and tropical trees."""
    xs, ys = detail.coords()
    tone = detail.norm01(detail.spectral(2.4, lo=1, hi=16, seed=801, aniso=(1.0, 0.3)))
    bands = 0.5 + 0.5 * np.sin(TAU * (ys * 14.0 + detail.spectral(3.0, lo=1, hi=6, seed=802) * 0.15))
    pores = smoothstep(2.0, 2.8, detail.spectral(0.3, lo=60, hi=200, seed=803, aniso=(1.0, 0.25)))
    height = tone * 0.4 + bands * 0.15 - pores * 0.4
    color = srgb((128, 114, 96)) * (0.82 + 0.3 * tone[..., None]) * (0.94 + 0.08 * bands[..., None]) * (1.0 - 0.45 * pores[..., None])
    return np.dstack([lit(height, color, 0.2, 4.0), np.ones((N, N))])


def warped_cells(grid, seed, warp, warp_seed, jitter=1.0):
    """Periodic cellular noise on a `grid` x `grid` lattice (as `stretched_cells`), its
    coordinates warped by periodic noise (`warp` cells): the cells' sides wander, uneven."""
    rng = np.random.default_rng(seed)
    off = 0.5 + (rng.random((grid, grid, 2)) - 0.5) * jitter
    ys, xs = np.mgrid[0:N, 0:N].astype(np.float64) * grid / N
    xs = xs + detail.spectral(2.6, lo=1, hi=12, seed=warp_seed) * warp
    ys = ys + detail.spectral(2.6, lo=1, hi=12, seed=warp_seed + 1) * warp
    cx, cy = np.floor(xs).astype(int), np.floor(ys).astype(int)
    f1 = np.full((N, N), 1e9)
    f2 = np.full((N, N), 1e9)
    idx = np.zeros((N, N), dtype=int)
    for oy in (-1, 0, 1):
        for ox in (-1, 0, 1):
            nx, ny = (cx + ox) % grid, (cy + oy) % grid
            d = np.hypot(xs - (cx + ox + off[ny, nx, 0]), ys - (cy + oy + off[ny, nx, 1]))
            closer = d < f1
            f2 = np.where(closer, f1, np.minimum(f2, d))
            idx = np.where(closer, ny * grid + nx, idx)
            f1 = np.where(closer, d, f1)
    return f1, f2, idx


def rock():
    """Rock for boulders and spires: weathered stone, its tone mottled, lumpy, a few fractures
    that wander and peter out (the sides of each a little offset), hairline cracks here and
    there, a fine grain and flecks of its minerals; not a paving of even cells."""
    f1, f2, idx = warped_cells(5, seed=911, warp=0.1, warp_seed=912)
    keep = smoothstep(0.38, 0.6, detail.norm01(detail.spectral(2.2, lo=1, hi=6, seed=913)))
    crack = (1.0 - smoothstep(0.0, 0.04, f2 - f1)) * keep
    g1, g2, _ = warped_cells(12, seed=914, warp=0.16, warp_seed=915)
    keep2 = smoothstep(0.55, 0.75, detail.norm01(detail.spectral(2.0, lo=2, hi=12, seed=916)))
    hair = (1.0 - smoothstep(0.0, 0.028, g2 - g1)) * keep2
    rng = np.random.default_rng(917)
    side = rng.uniform(-1.0, 1.0, 25)[idx] * keep
    lumps = detail.spectral(2.3, lo=2, hi=48, seed=918)
    grain = detail.spectral(1.0, lo=64, hi=256, seed=919)
    height = lumps * 0.3 + grain * 0.06 + side * 0.12 - crack * 0.7 - hair * 0.25
    tone = detail.norm01(detail.spectral(2.6, lo=1, hi=10, seed=920))
    fleck = detail.spectral(0.5, lo=100, hi=256, seed=921)
    shade = (tone - 0.5) * 0.45 + lumps * 0.05 + side * 0.07 - crack * 0.5 - hair * 0.2 + grain * 0.05 + (fleck > 2.3) * 0.22 - (fleck < -2.4) * 0.18
    color = srgb((128, 124, 118)) * np.clip(1.0 + shade, 0.2, 2.0)[..., None]
    return np.dstack([lit(detail.norm01(height), color, 0.3, 5.0), np.ones((N, N))])


def crystal():
    """Crystal (laid on along the world's axes, so its v runs up it): growth striations running
    up it, faint veils and inclusions within; a mid grey (the renderer tints it)."""
    stria = detail.spectral(1.2, lo=4, hi=220, seed=1201, aniso=(0.08, 1.0))
    veil = detail.norm01(detail.spectral(2.8, lo=1, hi=8, seed=1202))
    inclusion = smoothstep(2.0, 2.9, detail.spectral(0.8, lo=30, hi=140, seed=1203))
    shade = stria * 0.07 + (veil - 0.5) * 0.3 + inclusion * 0.25
    color = srgb((128, 128, 128)) * np.clip(1.0 + shade, 0.3, 2.0)[..., None]
    return np.dstack([np.clip(color, 0, 1), np.ones((N, N))])


def tendril():
    """A Mars plant's tendrils, as TA paints them (a tube's u round it, v along it): fibres
    along them, lighter and darker, ringed at uneven joints, mottled; a golden tan."""
    xs, ys = detail.coords()
    fibre = detail.spectral(1.1, lo=20, hi=220, seed=1301, aniso=(0.1, 1.0))
    # (Joints straight across, unevenly spaced along.)
    wander = detail.spectral(3.0, lo=1, hi=5, seed=1302)[:, :1]
    ring = smoothstep(0.86, 0.98, 0.5 + 0.5 * np.sin(TAU * (ys * 5.0 + wander * 0.12)))
    mottle = detail.norm01(detail.spectral(2.4, lo=2, hi=16, seed=1303))
    fleck = detail.spectral(0.6, lo=60, hi=200, seed=1304)
    shade = fibre * 0.16 + (mottle - 0.5) * 0.45 - ring * 0.4 - (fleck > 2.4) * 0.3
    height = fibre * 0.2 - ring * 0.6 + mottle * 0.2
    color = srgb((150, 124, 80)) * np.clip(1.0 + shade, 0.2, 2.0)[..., None]
    return np.dstack([lit(detail.norm01(height), color, 0.25, 4.0), np.ones((N, N))])


# The gasbag layer's bands (v): the bag's skin, the tether's vine, the pod at its foot.
GASBAG_SKIN, GASBAG_VINE = 0.85, 0.95


def gasbag():
    """An acid world's gasbag, after TA's painting: its bag's skin in three-tone yellow-olive
    camouflage with fine mottling and eight gore seams (on a bag mapped by longitude in u and
    latitude in v, they meet at its poles as TA's do), creases where it gathers to its neck; then a
    band of the tether's braided vine, and one of the pod at its foot. Every band tiles in u."""
    xs, ys = detail.coords()
    # Blotches stretched 2:1 in v (u spans the bag's girth, v only half of it).
    blotch = detail.norm01(detail.spectral(2.6, lo=2, hi=10, seed=1101, aniso=(1.0, 0.5)))
    fine = detail.spectral(1.0, lo=60, hi=200, seed=1102)
    dark, mid, light = srgb((42, 46, 32)), srgb((98, 104, 62)), srgb((148, 175, 86))
    t1 = smoothstep(0.36, 0.42, blotch)[..., None]
    t2 = smoothstep(0.62, 0.68, blotch)[..., None]
    skin = dark * (1 - t1) + mid * t1
    skin = skin * (1 - t2) + light * t2
    skin = skin * (1.0 + 0.08 * fine[..., None])
    gore = np.abs(((xs * 8.0 + 0.5) % 1.0) - 0.5)
    seam = 1.0 - smoothstep(0.0, 0.035, 0.5 - gore)
    crease = smoothstep(0.72, 0.84, ys) * (0.5 + 0.5 * np.sin(TAU * xs * 24.0 + detail.spectral(3.0, lo=1, hi=6, seed=1103) * 2.0))
    height = -seam * 0.6 - crease * 0.3 + blotch * 0.1
    skin = lit(height, skin * (1.0 - 0.35 * seam[..., None]) * (1.0 - 0.2 * crease[..., None]), 0.3, 5.0)
    # The vine: fibres along u, a lighter twist across them.
    fibre = detail.spectral(1.2, lo=30, hi=200, seed=1104, aniso=(1.0, 0.1))
    twist = 0.5 + 0.5 * np.sin(TAU * (xs * 18.0 + ys * 40.0))
    vine = srgb((50, 56, 40)) * (0.8 + 0.4 * twist[..., None]) * (1.0 + 0.2 * fibre[..., None])
    # The pod: yellow-olive, scaled.
    f1, f2, _ = stretched_cells(24, 4, seed=1105)
    scale = smoothstep(0.0, 0.3, f2 - f1)
    pod = lit(scale * 0.5, srgb((130, 125, 60)) * (0.7 + 0.3 * scale[..., None]), 0.3, 3.0)
    color = np.where((ys < GASBAG_SKIN)[..., None], skin, np.where((ys < GASBAG_VINE)[..., None], vine, pod))
    return np.dstack([np.clip(color, 0, 1), np.ones((N, N))])


def hull():
    """A vehicle's painted metal (laid on along the world's axes): even paint, a faint mottle
    of dust and wear, thin panel seams here and there, a few scuffs; a mid grey (the renderer
    tints it to its painting's colour)."""
    xs, ys = detail.coords()
    mottle = detail.norm01(detail.spectral(2.6, lo=1, hi=8, seed=1401))
    grain = detail.spectral(1.0, lo=80, hi=256, seed=1402)
    # Seams: a grid of panels, a seam left out here and there.
    gx = np.abs(((xs * 4.0 + 0.5) % 1.0) - 0.5)
    gy = np.abs(((ys * 3.0 + 0.5) % 1.0) - 0.5)
    keep_x = smoothstep(0.3, 0.5, detail.norm01(detail.spectral(3.0, lo=1, hi=4, seed=1403)))
    keep_y = smoothstep(0.3, 0.5, detail.norm01(detail.spectral(3.0, lo=1, hi=4, seed=1404)))
    seam = np.maximum((1.0 - smoothstep(0.004, 0.009, 0.5 - gx)) * keep_x, (1.0 - smoothstep(0.004, 0.009, 0.5 - gy)) * keep_y)
    scuff = smoothstep(2.2, 2.9, detail.spectral(0.7, lo=20, hi=120, seed=1405, aniso=(1.0, 0.3)))
    shade = (mottle - 0.5) * 0.12 + grain * 0.025 - seam * 0.35 + scuff * 0.12
    color = srgb((128, 128, 128)) * np.clip(1.0 + shade, 0.3, 2.0)[..., None]
    return np.dstack([np.clip(color, 0, 1), np.ones((N, N))])


def metal():
    """Pipework's steel (laid on along the world's axes): an even grey under a mottle of wear and
    grime, faint streaks of weather running down it, rust in a few small blooms and fine
    scratches; a mid grey (the renderer tints it to its painting's colour)."""
    mottle = detail.norm01(detail.spectral(2.4, lo=1, hi=10, seed=1501))
    grime = smoothstep(0.55, 0.85, detail.norm01(detail.spectral(2.2, lo=2, hi=24, seed=1502)))
    streak = detail.spectral(1.1, lo=3, hi=180, seed=1503, aniso=(0.12, 1.0))
    grain = detail.spectral(0.9, lo=90, hi=256, seed=1504)
    bloom = smoothstep(2.4, 3.1, detail.spectral(1.6, lo=6, hi=60, seed=1505))
    scratch = smoothstep(2.6, 3.4, detail.spectral(0.6, lo=40, hi=200, seed=1506, aniso=(1.0, 0.15)))
    shade = (mottle - 0.5) * 0.16 - grime * 0.16 + streak * 0.035 + grain * 0.03 + scratch * 0.18 - bloom * 0.12
    color = srgb((128, 128, 128)) * np.clip(1.0 + shade, 0.3, 2.0)[..., None]
    # Rust in its blooms: a warm brown, mixed in.
    rust = srgb((122, 82, 56)) * np.clip(1.0 + shade * 0.5, 0.3, 2.0)[..., None]
    color = color * (1.0 - bloom[..., None] * 0.7) + rust * bloom[..., None] * 0.7
    return np.dstack([np.clip(color, 0, 1), np.ones((N, N))])


# A concrete road barrier's profile, as tools/features/build.py's BARRIER_PROFILE (across, up)
# and the length of one: its faces' v runs round the profile, its u along the barrier.
BARRIER_PROFILE = [(7.0, 0.0), (7.0, 0.9), (3.7, 4.0), (2.5, 10.3), (2.1, 11.0), (-2.1, 11.0), (-2.5, 10.3), (-3.7, 4.0), (-7.0, 0.9), (-7.0, 0.0)]
BARRIER_LENGTH = 27.0


def barrier():
    """A concrete road barrier's faces (u along it, v round its profile, see BARRIER_PROFILE):
    weathered concrete, pitted, stained by the rain and darker towards its foot, its lower slopes
    painted with worn yellow and black hazard stripes. Its colours are its own (the renderer
    takes it to its painting's brightness)."""
    prof = np.array(BARRIER_PROFILE)
    seg = np.linalg.norm(np.diff(prof, axis=0), axis=1)
    arc = np.concatenate([[0.0], np.cumsum(seg)])
    total = arc[-1]
    xs, ys = detail.coords()
    # Each texel's place: along the barrier (world units) and round the profile, and its height.
    along = xs * BARRIER_LENGTH
    round_ = ys * total
    height = np.interp(round_, arc, prof[:, 1])
    # Concrete: a mottle of tone, rain stains running down, pits and a fine grain.
    tone = detail.norm01(detail.spectral(2.5, lo=1, hi=12, seed=1601))
    stain = detail.spectral(1.3, lo=2, hi=60, seed=1602, aniso=(0.15, 1.0))
    grain = detail.spectral(0.8, lo=100, hi=256, seed=1603)
    pit = smoothstep(2.3, 3.0, detail.spectral(0.4, lo=120, hi=256, seed=1604))
    shade = (tone - 0.5) * 0.14 + stain * 0.04 + grain * 0.05 - pit * 0.35 - (1.0 - smoothstep(0.0, 6.0, height)) * 0.18
    concrete = srgb((168, 166, 160)) * np.clip(1.0 + shade, 0.3, 2.0)[..., None]
    # The hazard stripes on the lower slopes (between the toe and the break of the profile),
    # running up the slope at 45 degrees, six to a barrier; worn through to the concrete in
    # patches and at the slope's edges.
    band = np.zeros((N, N))
    for lo, hi in ((arc[1], arc[2]), (arc[7], arc[8])):
        band = np.maximum(band, smoothstep(lo + 0.15, lo + 0.45, round_) * (1.0 - smoothstep(hi - 0.45, hi - 0.15, round_)))
    period = BARRIER_LENGTH / 6.0
    t = (along + round_) / period
    stripe = smoothstep(0.46, 0.54, np.abs(((t % 1.0) - 0.5) * 2.0))
    # (Chipped: small flakes gone, more of them towards the slope's foot where it is knocked.)
    chip = smoothstep(1.6, 2.4, detail.spectral(1.2, lo=20, hi=120, seed=1605) + (1.0 - smoothstep(0.0, 3.0, height)) * 0.9)
    paint = band * (1.0 - chip * 0.9) * (1.0 - pit * 0.8)
    yellow = srgb((206, 160, 34)) * np.clip(1.0 + shade * 0.6, 0.3, 2.0)[..., None]
    black = srgb((38, 36, 34)) * np.clip(1.0 + shade * 0.6, 0.3, 2.0)[..., None]
    painted = yellow * (1.0 - stripe[..., None]) + black * stripe[..., None]
    color = concrete * (1.0 - paint[..., None]) + painted * paint[..., None]
    return np.dstack([np.clip(color, 0, 1), np.ones((N, N))])


def trim():
    """A second paint (laid on along the world's axes): sheet metal under a fine grating of
    raised squares, worn at their edges, a mottle of grime; a mid grey (the renderer tints it to
    its instance's second colour, its `leaf`: a relay's warm roof)."""
    xs, ys = detail.coords()
    mottle = detail.norm01(detail.spectral(2.4, lo=1, hi=10, seed=1701))
    grain = detail.spectral(0.9, lo=90, hi=256, seed=1702)
    # The grating: 32 squares a tile each way, their gaps dark.
    gx = np.abs(((xs * 32.0) % 1.0) - 0.5)
    gy = np.abs(((ys * 32.0) % 1.0) - 0.5)
    gap = np.maximum(smoothstep(0.36, 0.44, gx), smoothstep(0.36, 0.44, gy))
    wear = smoothstep(0.6, 0.85, detail.norm01(detail.spectral(2.0, lo=3, hi=40, seed=1703)))
    shade = (mottle - 0.5) * 0.18 + grain * 0.03 - gap * 0.32 * (1.0 - wear * 0.6) + wear * 0.06
    color = srgb((128, 128, 128)) * np.clip(1.0 + shade, 0.3, 2.0)[..., None]
    return np.dstack([np.clip(color, 0, 1), np.ones((N, N))])


LAYERS = [
    ("broadleaf", broadleaf),
    ("needles", needles),
    ("jungle", jungle),
    ("tropical", tropical),
    ("smallleaf", smallleaf),
    ("twigs", twigs),
    ("clump", clump),
    ("bark", bark),
    ("bark_smooth", bark_smooth),
    ("rock", rock),
    ("gasbag", gasbag),
    ("crystal", crystal),
    ("tendril", tendril),
    ("hull", hull),
    ("metal", metal),
    ("barrier", barrier),
    ("trim", trim),
    # (The festive features' own: festive_textures.py.)
    ("snow", festive_textures.snow),
    ("festive", festive_textures.festive),
]


def to_png(rgba):
    return Image.fromarray(np.round(np.clip(rgba, 0, 1) * 255).astype(np.uint8), "RGBA")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=os.path.join(os.path.dirname(__file__), "..", "..", "crates", "ta_render", "features"))
    ap.add_argument("--preview")
    ap.add_argument("--only")
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    tiles = []
    for name, make in LAYERS:
        if args.only and name != args.only:
            continue
        rgba = make()
        # (Every opaque layer tiles but the festive atlas, a sheet of cells.)
        if rgba[..., 3].min() >= 0.999 and name != "festive":
            detail.check_tiles(rgba[..., :3].mean(axis=2), name)
        to_png(rgba).save(os.path.join(args.out, f"{name}.png"), optimize=True)
        cover = rgba[..., 3].mean()
        mean = (rgba[..., :3] * rgba[..., 3:]).sum(axis=(0, 1)) / max(rgba[..., 3].sum(), 1e-6)
        print(f"{name}: coverage {cover:.2f}, mean colour {np.round(mean * 255).astype(int)}")
        if args.preview:
            grey = np.full((N, N, 3), 0.45)
            tiles.append(np.clip(rgba[..., :3] * rgba[..., 3:] + grey * (1.0 - rgba[..., 3:]), 0, 1))
    if args.preview and tiles:
        os.makedirs(args.preview, exist_ok=True)
        cols = 3
        rows = (len(tiles) + cols - 1) // cols
        sheet = np.full((rows * N, cols * N, 3), 0.3)
        for i, t in enumerate(tiles):
            sheet[(i // cols) * N:(i // cols + 1) * N, (i % cols) * N:(i % cols + 1) * N] = t
        Image.fromarray(np.round(sheet * 255).astype(np.uint8)).save(os.path.join(args.preview, "textures.png"))


if __name__ == "__main__":
    main()
