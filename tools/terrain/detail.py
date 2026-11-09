"""Draws the ground's detail materials (original art for TA Reborn): tileable maps of surface
structure that the terrain shader lays over TA's painted ground up close, by material.

Each material is a 512x512 PNG with two channels: L its height (0 low, 255 high) and A how much
lighter or darker it makes the painted colour (128 not at all). Everything is made here from
periodic noise (spectral synthesis), periodic cellular noise and shapes drawn on a torus, so every
map tiles without a seam. The order of MATERIALS is the order of the texture array's layers
(`ta_render::terrain_detail`).

    python tools/terrain/detail.py [--out crates/ta_render/terrain_detail] [--preview <dir>]
"""
import argparse
import os

import numpy as np
from PIL import Image

N = 512
TAU = 2.0 * np.pi

# ---------------------------------------------------------------------------------------------
# Periodic building blocks
# ---------------------------------------------------------------------------------------------


def spectral(beta, lo=1.0, hi=None, seed=0, aniso=(1.0, 1.0)):
    """Periodic noise by spectral synthesis: random phases, power falling as f^-beta between
    `lo` and `hi` cycles a tile (`aniso` stretches the frequencies along x and y). Mean 0, std 1."""
    rng = np.random.default_rng(seed)
    fx = np.fft.fftfreq(N) * N * aniso[1]
    fy = np.fft.rfftfreq(N) * N * aniso[0]
    f = np.sqrt(fx[:, None] ** 2 + fy[None, :] ** 2)
    amp = np.zeros_like(f)
    nz = f > 0
    amp[nz] = f[nz] ** (-beta / 2.0)
    amp[f < lo] = 0.0
    if hi is not None:
        amp[f > hi] = 0.0
    spec = amp * np.exp(1j * rng.uniform(0, TAU, amp.shape))
    img = np.fft.irfft2(spec, s=(N, N))
    return (img - img.mean()) / (img.std() + 1e-12)


def cells(grid, seed, jitter=0.9):
    """Periodic cellular (Worley) noise on a `grid` x `grid` jittered lattice: the distances to
    the nearest and second nearest points (in cells) and the nearest point's index."""
    rng = np.random.default_rng(seed)
    off = 0.5 + (rng.random((grid, grid, 2)) - 0.5) * jitter
    ys, xs = np.mgrid[0:N, 0:N].astype(np.float64) * grid / N
    cx = np.floor(xs).astype(int)
    cy = np.floor(ys).astype(int)
    f1 = np.full((N, N), 1e9)
    f2 = np.full((N, N), 1e9)
    idx = np.zeros((N, N), dtype=int)
    for oy in (-1, 0, 1):
        for ox in (-1, 0, 1):
            nx = (cx + ox) % grid
            ny = (cy + oy) % grid
            px = cx + ox + off[ny, nx, 0]
            py = cy + oy + off[ny, nx, 1]
            d = np.hypot(xs - px, ys - py)
            closer = d < f1
            f2 = np.where(closer, f1, np.minimum(f2, d))
            idx = np.where(closer, ny * grid + nx, idx)
            f1 = np.where(closer, d, f1)
    return f1, f2, idx


def per_cell(idx, values):
    """A value for each cell, spread over its pixels."""
    return values[idx]


def stamp(canvas, x, y, patch, op=np.maximum):
    """Draws `patch` with its corner at (x, y) onto the torus `canvas`."""
    h, w = patch.shape
    ys = (np.arange(h) + int(y)) % N
    xs = (np.arange(w) + int(x)) % N
    canvas[np.ix_(ys, xs)] = op(canvas[np.ix_(ys, xs)], patch)


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


def blur(img, passes=1):
    """A small periodic blur (binomial)."""
    for _ in range(passes):
        img = (np.roll(img, 1, 0) + 2 * img + np.roll(img, -1, 0)) / 4.0
        img = (np.roll(img, 1, 1) + 2 * img + np.roll(img, -1, 1)) / 4.0
    return img


def norm01(img):
    lo, hi = np.percentile(img, 0.5), np.percentile(img, 99.5)
    return np.clip((img - lo) / max(hi - lo, 1e-9), 0.0, 1.0)


def coords():
    ys, xs = np.mgrid[0:N, 0:N].astype(np.float64) / N
    return xs, ys


# ---------------------------------------------------------------------------------------------
# The materials: each returns (height 0..1, brightness -1..1)
# ---------------------------------------------------------------------------------------------


# Sand's wind ripples: their wave vector, cycles a tile (integers, so the map tiles). At sand's
# 36-unit tile (MATERIAL_TILE in terrain.wgsl) (30, 5) puts a crest every 1.18 world units, 16.8
# texels, the wind 9.5 degrees off x.
RIPPLE_K = (30, 5)


def dislocations(xs, ys, pairs, seed):
    """Phase defects in the ripples: pairs of opposite windings a ripple or few apart along the
    crests. Round each the crests gain or lose one, a Y-shaped fork; the pair's phase jump of one
    whole cycle along the line between them is invisible, and a window brings the field back to
    0 well inside the tile, so the map still tiles. Also the distance to the nearest core."""
    rng = np.random.default_rng(seed)
    kx, ky = RIPPLE_K
    k = np.hypot(kx, ky)
    out = np.zeros((N, N))
    core = np.full((N, N), 1e9)
    along = np.array([-ky, kx]) / k
    lam = 1.0 / k
    for _ in range(pairs):
        cx, cy = rng.random(2)
        sep = rng.uniform(1.5, 4.0) * lam
        tilt = rng.uniform(-0.6, 0.6)
        d = along * np.cos(tilt) + np.array([kx, ky]) / k * np.sin(tilt)
        p1 = np.array([cx, cy]) - d * sep / 2
        p2 = np.array([cx, cy]) + d * sep / 2
        # (Torus distances to the pair's centre, so the window wraps.)
        dx = (xs - cx + 0.5) % 1.0 - 0.5
        dy = (ys - cy + 0.5) % 1.0 - 0.5
        r = np.hypot(dx, dy)
        x = cx + dx
        y = cy + dy
        wind = (np.arctan2(y - p1[1], x - p1[0]) - np.arctan2(y - p2[1], x - p2[0])) / TAU
        wind = (wind + 0.5) % 1.0 - 0.5
        r0 = sep * 1.6
        r1 = min(r0 + rng.uniform(5.0, 9.0) * lam, 0.4)
        for q in (p1, p2):
            ex = (xs - q[0] + 0.5) % 1.0 - 0.5
            ey = (ys - q[1] + 0.5) % 1.0 - 0.5
            core = np.minimum(core, np.hypot(ex, ey))
        out += wind * (1.0 - smoothstep(r0, r1, r)) * (r < r1 * 1.01)
    return out, core


def sand():
    """Wind ripples: long crests across the wind, nearly parallel, gently up the windward side
    and steeper down the lee; bending a little, forking and ending here and there; patches where
    they lie low; fine grains over them. (Crests that wander every way read as wood grain or
    fingerprints, and a lee face steeper than the shader's relief can light draws hard lines.)"""
    xs, ys = coords()
    kx, ky = RIPPLE_K
    k = np.hypot(kx, ky)
    # Sinuosity: a slow warp whose slope stays well under the ripples' own (crests turn by no
    # more than about 15 degrees, and never fold back into loops).
    bend = spectral(3.6, lo=1, hi=4, seed=11) * 0.45 + spectral(3.0, lo=4, hi=10, seed=12) * 0.06
    defects, core = dislocations(xs, ys, 26, seed=19)
    th = TAU * (xs * kx + ys * ky + bend + defects)
    # (At a fork's core the crest ends: the ripple sinks away there instead of leaving a knot.)
    ends = smoothstep(0.1 / k, 0.5 / k, core)
    # A skewed wave (the first terms of a sawtooth): about twice as steep down the lee as up the
    # windward side, its crest and trough rounded.
    prof = np.sin(th) + 0.38 * np.sin(2 * th) + 0.12 * np.sin(3 * th)
    prof = (prof - prof.min()) / (prof.max() - prof.min())
    # Where they lie low, where high: patches a third of a tile and more.
    strength = (0.55 + 0.45 * norm01(spectral(2.6, lo=1, hi=5, seed=13))) * (0.7 + 0.3 * ends)
    grains = spectral(0.3, lo=90, hi=256, seed=14)
    height = prof * strength * 0.62 + grains * 0.012 + spectral(2.4, lo=1, hi=6, seed=15) * 0.05
    # Brightness: crests a touch lighter (coarser grains), troughs darker, and the grains.
    shade = (prof - 0.5) * 0.16 * strength + grains * 0.22 + spectral(2.0, lo=2, hi=16, seed=16) * 0.08
    # (Its height in the middle of the range, so the steepest lee faces' slope, stored per texel,
    # is not clipped.)
    return 0.5 + (norm01(height) - 0.5) * 0.55, np.clip(shade, -1, 1)


def soil():
    """Packed earth: clods, a web of fine cracks where it dried, small stones."""
    f1, f2, idx = cells(22, seed=21)
    clod = 1.0 - smoothstep(0.0, 0.75, f1)
    # (The web only in the patches where it dried, finer there: unbroken over every slope, it
    # read as a net of dried mud.)
    dried = smoothstep(0.45, 0.8, norm01(spectral(2.2, lo=2, hi=12, seed=26)))
    crack = (1.0 - smoothstep(0.0, 0.045, f2 - f1)) * dried
    rng = np.random.default_rng(22)
    lift = per_cell(idx, rng.uniform(0.6, 1.0, 22 * 22))
    s1, _, sidx = cells(56, seed=23)
    stone_r = per_cell(sidx, rng.uniform(0.0, 0.42, 56 * 56) * (rng.random(56 * 56) < 0.35))
    stone = np.sqrt(np.clip(1.0 - (s1 / np.maximum(stone_r, 1e-6)) ** 2, 0.0, 1.0)) * (stone_r > 0.05)
    fine = spectral(1.2, lo=30, hi=256, seed=24)
    height = clod * lift * 0.55 - crack * 0.35 + stone * 0.5 + fine * 0.05 + spectral(2.6, lo=1, hi=8, seed=25) * 0.15
    shade = (clod * lift - 0.5) * 0.3 - crack * 0.4 + stone * 0.45 + fine * 0.18
    return norm01(height), np.clip(shade, -1, 1)


def grass():
    """Turf: thousands of blades leaning every way, thicker in clumps, soil between."""
    rng = np.random.default_rng(31)
    height = np.zeros((N, N))
    tips = np.zeros((N, N))
    clumps = norm01(spectral(2.4, lo=2, hi=14, seed=32))
    count = 16000
    for _ in range(count):
        x, y = rng.random(2) * N
        if rng.random() > 0.35 + 0.65 * clumps[int(y) % N, int(x) % N]:
            continue
        angle = rng.uniform(0, TAU)
        length = rng.uniform(6.0, 16.0)
        width = rng.uniform(0.9, 1.7)
        dx, dy = np.cos(angle), np.sin(angle)
        size = int(length + 4)
        gy, gx = np.mgrid[-size:size + 1, -size:size + 1].astype(np.float64)
        # Distance to the blade's line, how far along it, and its taper.
        along = gx * dx + gy * dy
        across = np.abs(-gx * dy + gy * dx)
        t = along / length
        inside = (t >= 0.0) & (t <= 1.0)
        w = width * (1.0 - t * 0.8)
        blade = np.where(inside, np.clip(1.0 - across / np.maximum(w, 0.2), 0.0, 1.0), 0.0)
        h = blade * (0.3 + 0.7 * t) * rng.uniform(0.6, 1.0)
        stamp(height, x - size, y - size, h)
        stamp(tips, x - size, y - size, blade * t * t)
    height = blur(height, 1)
    shade = (height - 0.35) * 0.9 + tips * 0.35 - (height < 0.05) * 0.35 + spectral(2.0, lo=2, hi=24, seed=33) * 0.1
    return norm01(height + clumps * 0.25), np.clip(shade, -1, 1)


def rock():
    """Weathered rock: fractured into blocks, the cracks deep and dark, rough within."""
    f1, f2, idx = cells(7, seed=41, jitter=1.0)
    big = 1.0 - smoothstep(0.0, 0.05, f2 - f1)
    g1, g2, _ = cells(17, seed=42, jitter=1.0)
    small = 1.0 - smoothstep(0.0, 0.035, g2 - g1)
    rng = np.random.default_rng(43)
    block = per_cell(idx, rng.uniform(-0.25, 0.25, 7 * 7))
    rough = spectral(2.0, lo=4, hi=256, seed=44)
    ridges = 1.0 - np.abs(spectral(2.8, lo=2, hi=40, seed=45))
    height = ridges * 0.35 + rough * 0.12 + block * 0.6 - big * 0.6 - small * 0.25
    speck = (spectral(0.4, lo=120, hi=256, seed=46) > 2.2) * 0.4
    shade = block * 0.5 + rough * 0.15 - big * 0.6 - small * 0.3 + speck
    return norm01(height), np.clip(shade, -1, 1)


def gravel():
    """Rounded stones of every size, packed, the gaps between them in shadow."""
    f1, _, idx = cells(30, seed=51, jitter=1.0)
    rng = np.random.default_rng(52)
    radius = per_cell(idx, rng.uniform(0.5, 0.85, 30 * 30))
    tone = per_cell(idx, rng.uniform(-0.45, 0.45, 30 * 30))
    cap = np.sqrt(np.clip(1.0 - (f1 / radius) ** 2, 0.0, 1.0))
    h1 = cap * per_cell(idx, rng.uniform(0.6, 1.0, 30 * 30))
    g1, _, gidx = cells(64, seed=53, jitter=1.0)
    rad2 = per_cell(gidx, rng.uniform(0.4, 0.7, 64 * 64))
    cap2 = np.sqrt(np.clip(1.0 - (g1 / rad2) ** 2, 0.0, 1.0)) * 0.55
    height = np.maximum(h1, cap2) + spectral(1.0, lo=60, hi=256, seed=54) * 0.03
    gap = (np.maximum(h1, cap2) < 0.05)
    shade = np.where(h1 >= cap2, tone, tone * 0.5) * (height > 0.05) - gap * 0.7 + (cap - 0.5) * 0.25
    return norm01(height), np.clip(shade, -1, 1)


def snow():
    """Wind-packed snow: soft drifts and faint ripples, glinting crystals."""
    xs, ys = coords()
    drifts = spectral(3.6, lo=1, hi=10, seed=61)
    warp = spectral(3.0, lo=1, hi=6, seed=62)
    ripple = np.sin(TAU * (xs * 6.0 + ys * 3.0 + warp * 0.4)) * 0.5 + 0.5
    height = drifts * 0.7 + ripple * 0.12 + spectral(1.0, lo=60, hi=256, seed=63) * 0.02
    glint = (spectral(0.2, lo=160, hi=256, seed=64) > 2.9) * 0.9
    shade = drifts * 0.06 + glint
    return norm01(height), np.clip(shade, -1, 1)


def ice():
    """Ice: smooth, crossed by white fracture lines, a few trapped bubbles."""
    f1, f2, _ = cells(5, seed=71, jitter=1.0)
    g1, g2, _ = cells(13, seed=72, jitter=1.0)
    crack = 1.0 - smoothstep(0.0, 0.02, f2 - f1)
    thin = (1.0 - smoothstep(0.0, 0.012, g2 - g1)) * norm01(spectral(2.0, lo=2, hi=8, seed=73))
    swell = spectral(3.8, lo=1, hi=6, seed=74)
    height = swell * 0.4 - crack * 0.25 - thin * 0.1
    bubbles = (spectral(0.2, lo=100, hi=256, seed=75) > 2.7) * 0.5
    shade = crack * 0.9 + thin * 0.5 + bubbles + swell * 0.05
    return norm01(height), np.clip(shade, -1, 1)


def volcanic():
    """Basalt: rough and porous, pitted with gas bubbles, ropy where it flowed."""
    xs, ys = coords()
    f1, _, idx = cells(44, seed=81, jitter=1.0)
    rng = np.random.default_rng(82)
    pit_r = per_cell(idx, rng.uniform(0.0, 0.55, 44 * 44) * (rng.random(44 * 44) < 0.55))
    pit = np.clip(1.0 - f1 / np.maximum(pit_r, 1e-6), 0.0, 1.0) * (pit_r > 0.1)
    warp = spectral(3.0, lo=1, hi=6, seed=83)
    rope = (np.sin(TAU * (ys * 14.0 + warp * 1.2 + np.sin(TAU * xs * 3.0) * 0.15)) * 0.5 + 0.5) * norm01(spectral(2.5, lo=1, hi=6, seed=84))
    rough = spectral(1.8, lo=6, hi=256, seed=85)
    height = rough * 0.25 + rope * 0.35 - pit * 0.6
    shade = rough * 0.25 - pit * 0.8 + rope * 0.12
    return norm01(height), np.clip(shade, -1, 1)


def metal():
    """Worn metal: grain brushed one way, small dents, scratches, grime in patches. (No plates:
    TA paints its own plating, which the detail must not cross.)"""
    rng = np.random.default_rng(91)
    brushed = spectral(0.8, lo=30, hi=256, seed=92, aniso=(1.0, 10.0))
    f1, _, idx = cells(11, seed=93, jitter=1.0)
    dent_r = per_cell(idx, rng.uniform(0.0, 0.5, 11 * 11) * (rng.random(11 * 11) < 0.45))
    dent = -np.clip(1.0 - (f1 / np.maximum(dent_r, 1e-6)) ** 2, 0.0, 1.0) * (dent_r > 0.15)
    scratches = np.zeros((N, N))
    for _ in range(260):
        x, y = rng.random(2) * N
        a = rng.uniform(0, TAU)
        length = rng.uniform(8, 50)
        size = int(length + 2)
        gy, gx = np.mgrid[-size:size + 1, -size:size + 1].astype(np.float64)
        along = gx * np.cos(a) + gy * np.sin(a)
        across = np.abs(-gx * np.sin(a) + gy * np.cos(a))
        line = ((along >= 0) & (along <= length)) * np.clip(1.0 - across / 0.7, 0.0, 1.0) * rng.uniform(0.3, 1.0)
        stamp(scratches, x - size, y - size, line)
    grime = spectral(2.6, lo=1, hi=16, seed=94)
    height = brushed * 0.03 + dent * 0.5 - scratches * 0.06 + spectral(2.0, lo=2, hi=20, seed=95) * 0.04
    shade = brushed * 0.14 + scratches * 0.45 + dent * 0.2 - np.clip(grime, 0.0, None) * 0.22
    return norm01(height), np.clip(shade, -1, 1)


def regolith():
    """Lunar dust: fine and soft, pocked with craters of every size, a few stones."""
    xs, ys = coords()
    rng = np.random.default_rng(101)
    height = spectral(2.2, lo=2, hi=256, seed=102) * 0.06
    shade = spectral(1.4, lo=8, hi=256, seed=103) * 0.15
    for _ in range(46):
        cx, cy = rng.random(2)
        r = rng.uniform(0.008, 0.07) ** 1.0
        dx = np.abs(xs - cx)
        dx = np.minimum(dx, 1 - dx)
        dy = np.abs(ys - cy)
        dy = np.minimum(dy, 1 - dy)
        d = np.hypot(dx, dy) / r
        bowl = -np.clip(1.0 - d * d, 0.0, 1.0) * 0.9
        rim = np.exp(-((d - 1.0) / 0.22) ** 2) * 0.35
        crater = (bowl + rim) * min(1.0, r / 0.03)
        height += crater * 0.5
        shade += rim * 0.4 + bowl * 0.15
    f1, _, idx = cells(40, seed=104, jitter=1.0)
    stone_r = per_cell(idx, rng.uniform(0.0, 0.35, 40 * 40) * (rng.random(40 * 40) < 0.12))
    stone = np.sqrt(np.clip(1.0 - (f1 / np.maximum(stone_r, 1e-6)) ** 2, 0.0, 1.0)) * (stone_r > 0.08)
    height += stone * 0.25
    shade += stone * 0.35
    return norm01(height), np.clip(shade, -1, 1)


MATERIALS = [
    ("sand", sand),
    ("soil", soil),
    ("grass", grass),
    ("rock", rock),
    ("gravel", gravel),
    ("snow", snow),
    ("ice", ice),
    ("volcanic", volcanic),
    ("metal", metal),
    ("regolith", regolith),
]


def check_tiles(img, name):
    """The map tiles: its edges continue each other as its middle does."""
    inner = np.abs(np.diff(img, axis=1)).mean()
    seam = np.abs(img[:, 0] - img[:, -1]).mean()
    assert seam < inner * 3.0 + 1e-3, f"{name}: seam {seam:.4f} against {inner:.4f} inside"


def preview(height, shade, base):
    """A lit swatch: the height's relief under a low sun, the brightness over a base colour, 2x2."""
    gx = (np.roll(height, -1, 1) - np.roll(height, 1, 1)) * 8.0
    gy = (np.roll(height, -1, 0) - np.roll(height, 1, 0)) * 8.0
    n = np.dstack([-gx, -gy, np.ones_like(height)])
    n /= np.linalg.norm(n, axis=2, keepdims=True)
    light = np.array([-0.6, -0.5, 0.62])
    light /= np.linalg.norm(light)
    lit = np.clip(n @ light, 0.0, 1.0) * 0.85 + 0.3
    col = np.array(base)[None, None, :] / 255.0 * (1.0 + shade[..., None] * 0.35) * lit[..., None]
    img = (np.clip(col, 0, 1) ** (1 / 1.0) * 255).astype(np.uint8)
    return np.tile(img, (2, 2, 1))


BASES = {"sand": (205, 170, 120), "soil": (120, 92, 64), "grass": (70, 120, 45), "rock": (120, 115, 108), "gravel": (140, 132, 120),
         "snow": (225, 230, 238), "ice": (170, 200, 220), "volcanic": (60, 52, 50), "metal": (120, 124, 128), "regolith": (150, 148, 145)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=os.path.join(os.path.dirname(__file__), "..", "..", "crates", "ta_render", "terrain_detail"))
    ap.add_argument("--preview")
    ap.add_argument("--only")
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    sheets = []
    for name, make in MATERIALS:
        if args.only and name != args.only:
            continue
        height, shade = make()
        check_tiles(height, name)
        la = np.dstack([np.round(height * 255), np.round(np.clip(shade * 0.5 + 0.5, 0, 1) * 255)]).astype(np.uint8)
        Image.fromarray(la, "LA").save(os.path.join(args.out, f"{name}.png"), optimize=True)
        print(f"{name}: height {height.mean():.2f}±{height.std():.2f}, brightness {shade.mean():+.2f}±{shade.std():.2f}")
        if args.preview:
            sheets.append(preview(height, shade, BASES[name]))
    if args.preview and sheets:
        os.makedirs(args.preview, exist_ok=True)
        cols = 5
        rows = (len(sheets) + cols - 1) // cols
        h, w, _ = sheets[0].shape
        sheet = np.zeros((rows * h, cols * w, 3), np.uint8)
        for i, s in enumerate(sheets):
            sheet[(i // cols) * h:(i // cols + 1) * h, (i % cols) * w:(i % cols + 1) * w] = s
        Image.fromarray(sheet).save(os.path.join(args.preview, "materials.png"))


if __name__ == "__main__":
    main()
