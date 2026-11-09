"""The textures of TA Reborn's own festive features (original art): the snow that lies on them,
laid on along the world's axes, and the festive atlas, a sheet of 8 x 8 cells each a paint, a
wrapping paper, a ribbon, a wall, a lit window, a bulb... that the models' faces take their
colours from by their texture coordinates (`tools/features/festive.py`). Both keep their own
colours: the renderer does not tint them to a painting (`ta_render::foliage`, its `snow` and
`festive` layers), and the festive features' paintings are drawn from the same models.

Imported by textures.py (its LAYERS) and festive.py (CELLS, `cell_uv`).
"""
import os
import sys

import numpy as np

N = 512
GRID = 8
CELL = N // GRID
# How far in from a cell's edges its texture coordinates keep (texels): the mipmaps down to a
# cell of eight texels never mix in a neighbour.
MARGIN = 4


def srgb(c):
    return np.array(c, dtype=np.float64) / 255.0


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


# The cells, in the order they lie on the sheet (left to right, top to bottom).
CELLS = [
    # Paints.
    "red", "green", "gold", "silver", "blue", "white", "black", "purple",
    # Wrapping papers and ribbons.
    "wrap_red", "wrap_green", "wrap_blue", "wrap_gold", "wrap_cane", "wrap_plaid", "ribbon_gold", "ribbon_red",
    # Walls and roofs.
    "planks", "beams", "wall_stone", "brick", "shingles", "snow", "ice", "carrot",
    # Metal, lights by night, wood.
    "brass", "iron", "lantern_red", "lantern_gold", "window", "clock", "wood_dark", "burlap",
    # Bulbs.
    "bulb_red", "bulb_green", "bulb_blue", "bulb_amber", "bulb_white", "bulb_pink", "bulb_cyan", "bulb_warm",
    # Cloth, garland, fireworks, plinths.
    "knit_red", "knit_green", "knit_blue", "wool", "garland", "tube_red", "tube_blue", "granite",
    # Doors, wreaths, signs.
    "door", "wreath", "paint_cream", "paint_teal", "shutter", "glass_dark", "copper", "slate_dark",
    # Roofs.
    "tiles_red", "slate_green",
]
assert len(CELLS) <= GRID * GRID


def cell_uv(name, u, v):
    """Texture coordinates (glTF: v down) of the point (u, v) (0..1) of cell `name`, kept off
    its edges."""
    i = CELLS.index(name)
    cx, cy = i % GRID, i // GRID
    inner = (CELL - 2 * MARGIN) / N
    u = np.asarray(u, np.float64)
    v = np.asarray(v, np.float64)
    return np.stack([(cx * CELL + MARGIN) / N + np.clip(u, 0, 1) * inner, (cy * CELL + MARGIN) / N + np.clip(v, 0, 1) * inner], axis=-1)


def _noise(seed, scale):
    """A smooth noise over one cell (CELL x CELL, mean 0, std 1)."""
    rng = np.random.default_rng(seed)
    k = max(int(CELL / scale), 2)
    small = rng.standard_normal((k + 3, k + 3))
    ys = np.linspace(1, k + 1, CELL)
    xs = np.linspace(1, k + 1, CELL)
    y0 = np.floor(ys).astype(int)
    x0 = np.floor(xs).astype(int)
    fy = (ys - y0)[:, None]
    fx = (xs - x0)[None, :]
    fy = fy * fy * (3 - 2 * fy)
    fx = fx * fx * (3 - 2 * fx)
    a = small[np.ix_(y0, x0)]
    b = small[np.ix_(y0, x0 + 1)]
    c = small[np.ix_(y0 + 1, x0)]
    d = small[np.ix_(y0 + 1, x0 + 1)]
    out = (a * (1 - fx) + b * fx) * (1 - fy) + (c * (1 - fx) + d * fx) * fy
    return (out - out.mean()) / (out.std() + 1e-9)


def _grid():
    ys, xs = np.mgrid[0:CELL, 0:CELL].astype(np.float64)
    return (xs + 0.5) / CELL, (ys + 0.5) / CELL


def _paint(c, seed, mottle=0.035, grain=0.02):
    base = srgb(c)
    k = 1.0 + _noise(seed, 16) * mottle + _noise(seed + 1, 2) * grain
    return base[None, None, :] * k[..., None]


def _metal(c, seed):
    """A polished metal: a soft band of light across it, as round things catch the sky."""
    u, v = _grid()
    base = srgb(c)
    band = 0.75 + 0.45 * np.exp(-((v - 0.32) / 0.18) ** 2) - 0.2 * smoothstep(0.6, 1.0, v)
    k = band * (1.0 + _noise(seed, 8) * 0.03)
    return base[None, None, :] * k[..., None]


def _stripes(a, b, seed, period=0.25, width=0.5, slant=1.0):
    u, v = _grid()
    t = ((u + v * slant) / period) % 1.0
    s = smoothstep(width - 0.04, width + 0.04, t)[..., None]
    return (srgb(a)[None, None, :] * (1 - s) + srgb(b)[None, None, :] * s) * (1.0 + _noise(seed, 8) * 0.025)[..., None]


def _dots(a, b, seed, every=0.25, r=0.06):
    u, v = _grid()
    gu = (u / every) % 1.0 - 0.5
    gv = (v / every + 0.5 * (np.floor(u / every) % 2)) % 1.0 - 0.5
    d = smoothstep(r * 1.2 / every, r / every, np.hypot(gu, gv))[..., None]
    return (srgb(a)[None, None, :] * (1 - d) + srgb(b)[None, None, :] * d) * (1.0 + _noise(seed, 8) * 0.025)[..., None]


def _stars(a, b, seed):
    """Little five-pointed stars scattered on a paper."""
    u, v = _grid()
    out = np.zeros(u.shape)
    rng = np.random.default_rng(seed)
    for _ in range(9):
        cx, cy, s = rng.uniform(0.1, 0.9), rng.uniform(0.1, 0.9), rng.uniform(0.06, 0.1)
        ang = np.arctan2(v - cy, u - cx)
        r = np.hypot(u - cx, v - cy)
        star = s * (0.55 + 0.45 * np.cos(5 * (ang + np.pi / 2)))
        out = np.maximum(out, smoothstep(star * 1.1, star * 0.9, r))
    return (srgb(a)[None, None, :] * (1 - out[..., None]) + srgb(b)[None, None, :] * out[..., None])


def _plaid(seed):
    u, v = _grid()
    red, green, gold = srgb((150, 26, 34)), srgb((26, 78, 46)), srgb((198, 156, 64))
    bu = (np.abs(((u * 3.0) % 1.0) - 0.5) < 0.17).astype(float)
    bv = (np.abs(((v * 3.0) % 1.0) - 0.5) < 0.17).astype(float)
    thin = ((np.abs(((u * 6.0) % 1.0) - 0.5) < 0.04) | (np.abs(((v * 6.0) % 1.0) - 0.5) < 0.04)).astype(float)
    c = red[None, None, :] * np.ones(u.shape)[..., None]
    c = c * (1 - 0.55 * bu[..., None]) + green[None, None, :] * 0.55 * bu[..., None]
    c = c * (1 - 0.55 * bv[..., None]) + green[None, None, :] * 0.55 * bv[..., None]
    c = c * (1 - thin[..., None]) + gold[None, None, :] * thin[..., None]
    return c


def _satin(c, seed):
    """A ribbon: a sheen down its middle, its edges darker."""
    u, v = _grid()
    k = 0.8 + 0.35 * np.exp(-((u - 0.45) / 0.22) ** 2) - 0.2 * smoothstep(0.4, 0.5, np.abs(u - 0.5))
    return srgb(c)[None, None, :] * (k * (1.0 + _noise(seed, 4) * 0.02))[..., None]


def _planks(c, seed, boards=5, gap=0.05):
    """Boards running up (v), each its own shade, grain along them, dark gaps between."""
    u, v = _grid()
    rng = np.random.default_rng(seed)
    which = np.floor(u * boards).astype(int)
    shade = rng.uniform(0.82, 1.1, boards + 1)[which]
    grain = 1.0 + 0.06 * np.sin((u * boards % 1.0) * 40.0 + _noise(seed, 3) * 2.5) + _noise(seed + 2, 2) * 0.03
    gaps = 1.0 - 0.55 * smoothstep(gap, 0.0, np.minimum(u * boards % 1.0, 1.0 - u * boards % 1.0))
    return srgb(c)[None, None, :] * (shade * grain * gaps)[..., None]


def _blocks(mortar, c, seed, rows=6, cols=3.0, jitter=0.12, rough=0.05):
    """Courses of blocks (stone, brick), each its own shade, mortar between, every other course
    offset by half a block."""
    u, v = _grid()
    rng = np.random.default_rng(seed)
    row = np.floor(v * rows)
    x = u * cols + 0.5 * (row % 2)
    col = np.floor(x)
    fx, fy = x - col, v * rows - row
    edge = np.minimum(np.minimum(fx, 1 - fx) / cols * rows, np.minimum(fy, 1 - fy))
    joint = smoothstep(0.08, 0.02, edge)
    table = rng.uniform(1.0 - jitter, 1.0 + jitter, 512)
    k = table[((row * 37 + col * 11) % 512).astype(int)] * (1.0 + _noise(seed, 4) * rough + _noise(seed + 1, 1.5) * rough * 0.6)
    face = srgb(c)[None, None, :] * k[..., None]
    return face * (1 - joint[..., None]) + srgb(mortar)[None, None, :] * joint[..., None]


def _shingles(seed):
    """Rows of slates, each overlapping the one below, their lower edges in shade."""
    u, v = _grid()
    rows = 7
    rng = np.random.default_rng(seed)
    row = np.floor(v * rows)
    x = u * 4.0 + 0.5 * (row % 2)
    col = np.floor(x)
    fy = v * rows - row
    k = rng.uniform(0.85, 1.1, 1024)[((row * 13 + col * 7) % 1024).astype(int)]
    shade = k * (0.78 + 0.3 * fy) * (1.0 - 0.3 * smoothstep(0.08, 0.0, np.minimum(x - col, 1 - (x - col))))
    return srgb((70, 76, 88))[None, None, :] * (shade * (1.0 + _noise(seed, 3) * 0.04))[..., None]


def _roof_tiles(c, seed, rows=7, cols=5.0, round_=0.3):
    """Rows of roof tiles (clay, slate), each a little curved and lit along its top, the row below
    shaded under it."""
    u, v = _grid()
    rng = np.random.default_rng(seed)
    row = np.floor(v * rows)
    x = u * cols + 0.5 * (row % 2)
    col = np.floor(x)
    fx, fy = x - col, v * rows - row
    k = rng.uniform(0.86, 1.12, 1024)[((row * 13 + col * 7) % 1024).astype(int)]
    curve = 1.0 + round_ * (0.25 - (fx - 0.5) ** 2)
    shade = k * curve * (0.72 + 0.34 * fy) * (1.0 - 0.32 * smoothstep(0.07, 0.0, np.minimum(fx, 1 - fx)))
    return srgb(c)[None, None, :] * (shade * (1.0 + _noise(seed, 3) * 0.04))[..., None]


def _snowcell(seed):
    u, v = _grid()
    k = 1.0 + _noise(seed, 10) * 0.025 + _noise(seed + 1, 2) * 0.015
    c = srgb((236, 240, 247))[None, None, :] * k[..., None]
    glint = (np.random.default_rng(seed).random(u.shape) > 0.985) * 0.06
    return c + glint[..., None]


def _window(seed):
    """A lit window: warm light, brighter low in its middle where the room's lamp is, behind
    four panes in a dark frame."""
    u, v = _grid()
    glow = 0.82 + 0.28 * np.exp(-(((u - 0.5) / 0.4) ** 2 + ((v - 0.68) / 0.45) ** 2))
    light = srgb((255, 196, 104))[None, None, :] * glow[..., None]
    frame = (np.minimum(np.minimum(u, 1 - u), np.minimum(v, 1 - v)) < 0.08) | (np.abs(u - 0.5) < 0.035) | (np.abs(v - 0.5) < 0.035)
    dark = srgb((48, 34, 26))
    return np.where(frame[..., None], dark[None, None, :], light)


def _clock(seed):
    """A clock's face, lit from within: hours marked round it, both hands at twelve (midnight)."""
    u, v = _grid()
    x, y = u - 0.5, v - 0.5
    r = np.hypot(x, y)
    ang = np.arctan2(x, -y)
    face = srgb((246, 238, 210))[None, None, :] * (1.0 - 0.06 * smoothstep(0.2, 0.47, r))[..., None]
    ink = srgb((30, 26, 24))
    marks = (np.abs(((ang / (2 * np.pi) * 12.0 + 0.5) % 1.0) - 0.5) < 0.09) & (r > 0.36) & (r < 0.44)
    rim = (r > 0.46)
    hour = (np.abs(x) < 0.025) & (y < 0.0) & (y > -0.24)
    minute = (np.abs(x - 0.012) < 0.016) & (y < 0.0) & (y > -0.36)
    hub = r < 0.035
    ink_mask = marks | hour | minute | hub
    out = np.where(ink_mask[..., None], ink[None, None, :], face)
    brass = srgb((176, 136, 64))
    return np.where(rim[..., None], brass[None, None, :] * (0.9 + 0.2 * np.cos(ang))[..., None], out)


def _lantern(c, seed):
    """Paper lantern: ribs round it (rows), the paper lit from within."""
    u, v = _grid()
    ribs = 1.0 - 0.35 * smoothstep(0.06, 0.0, np.abs(((v * 6.0) % 1.0) - 0.5) - 0.44)
    glow = 0.85 + 0.2 * np.exp(-((u - 0.5) / 0.35) ** 2)
    return srgb(c)[None, None, :] * (ribs * glow * (1.0 + _noise(seed, 6) * 0.03))[..., None]


def _knit(c, seed, stripe=None):
    u, v = _grid()
    k = 1.0 + 0.08 * np.sin(u * 2 * np.pi * 10) * np.sin(v * 2 * np.pi * 14) + _noise(seed, 4) * 0.03
    out = srgb(c)[None, None, :] * k[..., None]
    if stripe is not None:
        s = (np.abs(((v * 4.0) % 1.0) - 0.5) < 0.12)[..., None]
        out = np.where(s, srgb(stripe)[None, None, :] * k[..., None], out)
    return out


def _garland(seed):
    """Fir garland (opaque): dark needles in strokes over a darker heart."""
    u, v = _grid()
    rng = np.random.default_rng(seed)
    out = srgb((22, 52, 30))[None, None, :] * np.ones(u.shape)[..., None]
    for _ in range(140):
        x, y = rng.uniform(0, 1, 2)
        ang = rng.uniform(0, np.pi)
        d = np.abs((u - x) * np.sin(ang) - (v - y) * np.cos(ang))
        along = np.abs((u - x) * np.cos(ang) + (v - y) * np.sin(ang))
        m = (d < 0.012) & (along < 0.09)
        out[m] = srgb((44, 96, 52)) * rng.uniform(0.8, 1.25)
    return out


def _wreath(seed):
    out = _garland(seed)
    u, v = _grid()
    rng = np.random.default_rng(seed + 5)
    for _ in range(14):
        x, y = rng.uniform(0.05, 0.95, 2)
        r = np.hypot(u - x, v - y)
        out = np.where((r < 0.035)[..., None], srgb((190, 24, 32))[None, None, :] * (1.1 - r * 6)[..., None], out)
    return out


def _tube(c, d, seed):
    """A firework's paper tube: its colour, a band of another round it, stars."""
    out = _stars(c, (236, 210, 120), seed)
    u, v = _grid()
    band = (np.abs(v - 0.3) < 0.07)[..., None]
    return np.where(band, srgb(d)[None, None, :], out) * (1.0 + _noise(seed, 6) * 0.03)[..., None]


def _door(seed):
    u, v = _grid()
    c = _planks((96, 42, 32), seed, boards=4, gap=0.04)
    frame = (np.minimum(u, 1 - u) < 0.07) | (v < 0.06)
    knob = np.hypot(u - 0.8, v - 0.55) < 0.04
    c = np.where(frame[..., None], srgb((58, 30, 24))[None, None, :], c)
    return np.where(knob[..., None], srgb((200, 160, 70))[None, None, :], c)


def _shutter(seed):
    u, v = _grid()
    slats = 1.0 - 0.3 * smoothstep(0.1, 0.0, np.abs(((v * 9.0) % 1.0) - 0.5) - 0.38)
    return srgb((34, 82, 62))[None, None, :] * (slats * (1.0 + _noise(seed, 5) * 0.04))[..., None]


def _bulb(c):
    u, v = _grid()
    k = 0.92 + 0.12 * np.exp(-(((u - 0.4) / 0.3) ** 2 + ((v - 0.35) / 0.3) ** 2))
    return srgb(c)[None, None, :] * k[..., None]


def _ice(seed):
    u, v = _grid()
    k = 1.0 + _noise(seed, 6) * 0.05
    cracks = smoothstep(0.03, 0.0, np.abs(_noise(seed + 3, 10))) * 0.12
    return srgb((176, 214, 232))[None, None, :] * (k + cracks)[..., None]


def festive():
    """The festive atlas (RGBA, alpha 1): each cell of CELLS drawn in its own 64 texels."""
    out = np.zeros((N, N, 3))
    draw = {
        "red": lambda s: _paint((168, 22, 30), s),
        "green": lambda s: _paint((22, 92, 50), s),
        "gold": lambda s: _metal((214, 166, 62), s),
        "silver": lambda s: _metal((198, 204, 214), s),
        "blue": lambda s: _metal((40, 78, 176), s),
        "white": lambda s: _paint((236, 234, 228), s, mottle=0.02),
        "black": lambda s: _paint((24, 24, 28), s, mottle=0.05),
        "purple": lambda s: _metal((112, 46, 146), s),
        "wrap_red": lambda s: _stripes((168, 24, 34), (226, 186, 92), s, period=0.22, width=0.78),
        "wrap_green": lambda s: _dots((24, 96, 54), (240, 238, 230), s),
        "wrap_blue": lambda s: _stars((34, 64, 150), (230, 230, 236), s),
        "wrap_gold": lambda s: _dots((206, 158, 66), (170, 28, 36), s, every=0.2, r=0.05),
        "wrap_cane": lambda s: _stripes((240, 238, 232), (196, 26, 36), s, period=0.2, width=0.55),
        "wrap_plaid": _plaid,
        "ribbon_gold": lambda s: _satin((222, 172, 64), s),
        "ribbon_red": lambda s: _satin((190, 24, 34), s),
        "planks": lambda s: _planks((150, 104, 64), s),
        "beams": lambda s: _planks((86, 58, 38), s, boards=2, gap=0.02),
        "wall_stone": lambda s: _blocks((120, 116, 110), (164, 158, 148), s, rows=5, cols=2.2, jitter=0.16, rough=0.07),
        "brick": lambda s: _blocks((178, 168, 156), (150, 62, 46), s, rows=8, cols=3.0, jitter=0.1),
        "shingles": _shingles,
        "snow": _snowcell,
        "ice": _ice,
        "carrot": lambda s: _stripes((226, 108, 28), (196, 86, 20), s, period=0.18, width=0.85, slant=0.0),
        "brass": lambda s: _metal((188, 146, 70), s),
        "iron": lambda s: _metal((52, 54, 58), s),
        "lantern_red": lambda s: _lantern((236, 60, 40), s),
        "lantern_gold": lambda s: _lantern((250, 186, 76), s),
        "window": _window,
        "clock": _clock,
        "wood_dark": lambda s: _planks((92, 56, 36), s, boards=3, gap=0.03),
        "burlap": lambda s: _knit((168, 136, 92), s),
        "bulb_red": lambda s: _bulb((255, 54, 40)),
        "bulb_green": lambda s: _bulb((60, 236, 84)),
        "bulb_blue": lambda s: _bulb((70, 120, 255)),
        "bulb_amber": lambda s: _bulb((255, 176, 40)),
        "bulb_white": lambda s: _bulb((250, 246, 236)),
        "bulb_pink": lambda s: _bulb((255, 92, 190)),
        "bulb_cyan": lambda s: _bulb((80, 230, 240)),
        "bulb_warm": lambda s: _bulb((255, 214, 150)),
        "knit_red": lambda s: _knit((176, 30, 38), s, stripe=(236, 232, 220)),
        "knit_green": lambda s: _knit((30, 104, 60), s, stripe=(236, 232, 220)),
        "knit_blue": lambda s: _knit((44, 76, 160), s, stripe=(236, 232, 220)),
        "wool": lambda s: _knit((236, 234, 228), s),
        "garland": _garland,
        "tube_red": lambda s: _tube((180, 30, 40), (232, 230, 220), s),
        "tube_blue": lambda s: _tube((40, 70, 160), (220, 40, 50), s),
        "granite": lambda s: _blocks((110, 108, 106), (150, 148, 146), s, rows=3, cols=1.5, jitter=0.08, rough=0.09),
        "door": _door,
        "wreath": _wreath,
        "paint_cream": lambda s: _paint((228, 214, 182), s),
        "paint_teal": lambda s: _paint((44, 112, 118), s),
        "shutter": _shutter,
        "glass_dark": lambda s: _metal((46, 56, 70), s),
        "copper": lambda s: _metal((176, 102, 62), s),
        "slate_dark": lambda s: _paint((54, 58, 68), s, mottle=0.06),
        "tiles_red": lambda s: _roof_tiles((156, 62, 44), s),
        "slate_green": lambda s: _roof_tiles((70, 96, 88), s, rows=8, cols=4.0, round_=0.1),
    }
    # (The cells not used yet a middling grey: the coarsest mipmaps average the whole sheet.)
    out[:] = srgb((128, 124, 120))
    for i, name in enumerate(CELLS):
        cx, cy = i % GRID, i // GRID
        out[cy * CELL:(cy + 1) * CELL, cx * CELL:(cx + 1) * CELL] = np.clip(draw[name](1000 + i * 17), 0, 1)
    return np.dstack([out, np.ones((N, N))])


def snow():
    """Snow (laid on along the world's axes): soft drifts, faint ripples the wind left, blue in
    its hollows, glinting crystals; its own colours (untinted)."""
    # (The terrain's periodic noise; imported here, as Blender, drawing the paintings, has no
    # Pillow for it.)
    sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "terrain"))
    import detail
    xs, ys = detail.coords()
    drifts = detail.spectral(3.4, lo=1, hi=10, seed=1801)
    warp = detail.spectral(3.0, lo=1, hi=6, seed=1802)
    ripple = np.sin(2 * np.pi * (xs * 5.0 + ys * 2.0 + warp * 0.35)) * 0.5 + 0.5
    grain = detail.spectral(0.9, lo=80, hi=256, seed=1803)
    glint = (detail.spectral(0.2, lo=160, hi=256, seed=1804) > 2.8).astype(float)
    k = 1.0 + drifts * 0.03 + (ripple - 0.5) * 0.025 + grain * 0.012 + glint * 0.05
    white = srgb((238, 242, 248))
    blue = srgb((214, 226, 242))
    hollow = 0.7 * smoothstep(0.2, -1.4, drifts)[..., None]
    c = (white[None, None, :] * (1 - hollow) + blue[None, None, :] * hollow) * k[..., None]
    return np.dstack([np.clip(c, 0, 1), np.ones((N, N))])
