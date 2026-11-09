"""The textures the Perspective cockpits share (original art for TA Reborn), drawn here:

  wear.png    tileable, 512 pixels square, linear data: r the chip field (the shader keeps the
              paint where it lies under a threshold that falls towards edges and where hands go:
              the lower the threshold, the bigger the chips), g scratches, b grime and stains,
              a fine grain. The noise is Blender's, in four dimensions round a torus (so that it
              tiles), baked by Cycles (`bake_wear`, run inside Blender); the scratches are drawn.
  decals.png  the stencils, warning plates, labels and data plates painted on the cockpits
              (sRGB, straight alpha, the colour bled out past each item's edge for its mipmaps),
              in the stencil letters below: strokes on a grid, bridges cut through their bowls.

`build.py --textures` writes both beside the models; the models' decals take their places in
the atlas from `DECALS` (the layout is worked out the same way each time).
"""

import math
import struct
import zlib

import numpy as np

# ---------------------------------------------------------------------------------------------
# PNG
# ---------------------------------------------------------------------------------------------


def write_png(path, rgba):
    """Writes an 8-bit RGBA image (rows from the top) as PNG (filtered by row, compressed)."""
    h, w, _ = rgba.shape
    data = np.ascontiguousarray(rgba, dtype=np.uint8)
    rows = []
    prev = np.zeros((w * 4,), dtype=np.int16)
    for y in range(h):
        row = data[y].reshape(-1).astype(np.int16)
        # The "up" filter (each byte less the one above it) suits noise and flat paint alike.
        rows.append(b"\x02" + ((row - prev) & 0xFF).astype(np.uint8).tobytes())
        prev = row
    raw = b"".join(rows)

    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

    out = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(out)


# ---------------------------------------------------------------------------------------------
# Signed distances (2D, any units)
# ---------------------------------------------------------------------------------------------


def sd_box(X, Y, x0, y0, x1, y1):
    cx, cy, bx, by = (x0 + x1) / 2, (y0 + y1) / 2, abs(x1 - x0) / 2, abs(y1 - y0) / 2
    qx, qy = np.abs(X - cx) - bx, np.abs(Y - cy) - by
    return np.hypot(np.maximum(qx, 0), np.maximum(qy, 0)) + np.minimum(np.maximum(qx, qy), 0)


def sd_round_box(X, Y, x0, y0, x1, y1, r):
    cx, cy, bx, by = (x0 + x1) / 2, (y0 + y1) / 2, abs(x1 - x0) / 2, abs(y1 - y0) / 2
    qx, qy = np.abs(X - cx) - bx + r, np.abs(Y - cy) - by + r
    return np.hypot(np.maximum(qx, 0), np.maximum(qy, 0)) + np.minimum(np.maximum(qx, qy), 0) - r


def sd_stroke(X, Y, x0, y0, x1, y1, hw=0.5):
    """A straight stroke from one point to the other, `hw` either side of it, its ends square
    and half a stroke past the points."""
    dx, dy = x1 - x0, y1 - y0
    length = math.hypot(dx, dy)
    ux, uy = (dx / length, dy / length) if length > 1e-9 else (1.0, 0.0)
    px, py = X - (x0 + x1) / 2, Y - (y0 + y1) / 2
    a, b = px * ux + py * uy, -px * uy + py * ux
    qa, qb = np.abs(a) - (length / 2 + hw), np.abs(b) - hw
    return np.hypot(np.maximum(qa, 0), np.maximum(qb, 0)) + np.minimum(np.maximum(qa, qb), 0)


def sd_ring(X, Y, x0, y0, x1, y1, r, hw=0.5):
    """The outline of a rounded rectangle through those corners (radius `r`), `hw` either side."""
    return np.abs(sd_round_box(X, Y, x0, y0, x1, y1, r)) - hw


def sd_poly(X, Y, pts):
    """A closed polygon (any winding), negative inside."""
    d = np.full(X.shape, np.inf)
    s = np.ones(X.shape)
    n = len(pts)
    for i in range(n):
        (ax, ay), (bx, by) = pts[i], pts[i - 1]
        ex, ey = bx - ax, by - ay
        wx, wy = X - ax, Y - ay
        t = np.clip((wx * ex + wy * ey) / max(ex * ex + ey * ey, 1e-12), 0, 1)
        d = np.minimum(d, (wx - ex * t) ** 2 + (wy - ey * t) ** 2)
        c1 = Y >= ay
        c2 = Y < by
        c3 = ex * wy > ey * wx
        flip = (c1 & c2 & c3) | (~c1 & ~c2 & ~c3)
        s = np.where(flip, -s, s)
    return s * np.sqrt(d)


# ---------------------------------------------------------------------------------------------
# Stencil letters: strokes on a grid four units wide and six high (y up), one unit thick, their
# ends square; `o` an outline of a rounded rectangle; each primitive may carry boxes cut out of
# it alone, and `g` boxes are cut through the whole letter (the stencil's bridges).
# ---------------------------------------------------------------------------------------------


def _l(x0, y0, x1, y1, *cuts):
    return ("l", (x0, y0, x1, y1), cuts)


def _o(x0, y0, x1, y1, r, *cuts):
    return ("o", (x0, y0, x1, y1, r), cuts)


def _g(x0, y0, x1, y1):
    return ("g", (x0, y0, x1, y1), ())


_TOP = _g(1.82, 4.6, 2.18, 6.4)
_BOTTOM = _g(1.82, -0.4, 2.18, 1.4)
_STEM = _g(1.0, -0.4, 1.36, 6.4)

GLYPHS = {
    "A": [_l(0.5, 0.5, 2.0, 5.5), _l(2.0, 5.5, 3.5, 0.5), _l(1.15, 2.1, 2.85, 2.1), _g(1.82, 3.9, 2.18, 6.4)],
    "B": [_l(0.5, 0.5, 0.5, 5.5), _o(0.5, 3.0, 3.3, 5.5, 1.0), _o(0.5, 0.5, 3.5, 3.0, 1.05), _STEM],
    "C": [_o(0.5, 0.5, 3.5, 5.5, 1.35, (2.4, 1.75, 4.4, 4.25))],
    "D": [_l(0.5, 0.5, 0.5, 5.5), _o(0.5, 0.5, 3.5, 5.5, 1.5), _g(1.0, 4.6, 1.36, 6.4), _g(1.0, -0.4, 1.36, 1.4)],
    "E": [_l(0.5, 0.5, 0.5, 5.5), _l(0.5, 5.5, 3.5, 5.5), _l(0.5, 3.0, 3.0, 3.0), _l(0.5, 0.5, 3.5, 0.5), _STEM],
    "F": [_l(0.5, 0.5, 0.5, 5.5), _l(0.5, 5.5, 3.5, 5.5), _l(0.5, 3.0, 3.0, 3.0), _g(1.0, 1.9, 1.36, 6.4)],
    "G": [_o(0.5, 0.5, 3.5, 5.5, 1.35, (2.4, 2.75, 4.4, 4.25)), _l(2.2, 2.5, 3.5, 2.5)],
    "H": [_l(0.5, 0.5, 0.5, 5.5), _l(3.5, 0.5, 3.5, 5.5), _l(0.5, 3.0, 3.5, 3.0)],
    "I": [_l(2.0, 0.5, 2.0, 5.5)],
    "J": [_l(3.0, 2.0, 3.0, 5.5), _o(0.5, 0.5, 3.0, 3.6, 1.2, (-0.4, 2.0, 2.3, 4.4))],
    "K": [_l(0.5, 0.5, 0.5, 5.5), _l(1.7, 3.0, 3.5, 5.5), _l(1.7, 3.0, 3.5, 0.5), _STEM],
    "L": [_l(0.5, 0.5, 0.5, 5.5), _l(0.5, 0.5, 3.5, 0.5)],
    "M": [_l(0.5, 0.5, 0.5, 5.5), _l(3.5, 0.5, 3.5, 5.5), _l(0.5, 5.5, 2.0, 2.2), _l(2.0, 2.2, 3.5, 5.5)],
    "N": [_l(0.5, 0.5, 0.5, 5.5), _l(3.5, 0.5, 3.5, 5.5), _l(0.5, 5.5, 3.5, 0.5)],
    "O": [_o(0.5, 0.5, 3.5, 5.5, 1.35), _TOP, _BOTTOM],
    "P": [_l(0.5, 0.5, 0.5, 5.5), _o(0.5, 2.5, 3.5, 5.5, 1.1), _g(1.0, 1.9, 1.36, 6.4)],
    "Q": [_o(0.5, 0.5, 3.5, 5.5, 1.35), _l(2.5, 1.5, 3.7, -0.3), _TOP, _g(1.82, -0.4, 2.18, 1.4)],
    "R": [_l(0.5, 0.5, 0.5, 5.5), _o(0.5, 2.5, 3.5, 5.5, 1.1), _l(2.1, 2.5, 3.5, 0.5), _g(1.0, 1.9, 1.36, 6.4)],
    "S": [_o(0.5, 3.0, 3.5, 5.5, 1.1, (2.0, 2.4, 4.4, 4.45)), _o(0.5, 0.5, 3.5, 3.0, 1.1, (-0.4, 1.55, 2.0, 3.6))],
    "T": [_l(0.5, 5.5, 3.5, 5.5), _l(2.0, 0.5, 2.0, 4.15)],
    "U": [_o(0.5, 0.5, 3.5, 7.5, 1.35), _BOTTOM],
    "V": [_l(0.5, 5.5, 2.0, 0.5), _l(2.0, 0.5, 3.5, 5.5), _g(1.82, -0.4, 2.18, 1.6)],
    "W": [_l(0.4, 5.5, 1.2, 0.5), _l(1.2, 0.5, 2.0, 3.6), _l(2.0, 3.6, 2.8, 0.5), _l(2.8, 0.5, 3.6, 5.5)],
    "X": [_l(0.5, 0.5, 3.5, 5.5), _l(0.5, 5.5, 3.5, 0.5)],
    "Y": [_l(0.5, 5.5, 2.0, 3.0), _l(3.5, 5.5, 2.0, 3.0), _l(2.0, 3.0, 2.0, 0.5)],
    "Z": [_l(0.5, 5.5, 3.5, 5.5), _l(3.5, 5.5, 0.5, 0.5), _l(0.5, 0.5, 3.5, 0.5)],
    "0": [_o(0.75, 0.5, 3.25, 5.5, 1.15), _TOP, _BOTTOM],
    "1": [_l(2.3, 0.5, 2.3, 5.5), _l(1.0, 4.4, 2.3, 5.5)],
    "2": [_o(0.5, 3.0, 3.5, 5.5, 1.2, (-0.4, 2.3, 4.4, 3.8), (-0.4, 3.6, 1.3, 4.3)), _l(3.2, 3.8, 0.95, 1.0), _l(0.5, 0.5, 3.5, 0.5)],
    "3": [_o(0.5, 3.0, 3.3, 5.5, 1.0, (-0.4, 2.4, 1.3, 4.6)), _o(0.5, 0.5, 3.5, 3.0, 1.05, (-0.4, 1.4, 1.3, 3.6))],
    "4": [_l(2.9, 0.5, 2.9, 5.5), _l(2.6, 5.4, 0.5, 2.1), _l(0.5, 2.0, 3.6, 2.0)],
    "5": [_l(3.4, 5.5, 0.5, 5.5), _l(0.5, 5.5, 0.5, 3.3), _o(0.5, 0.5, 3.5, 3.4, 1.1, (-0.4, 1.5, 1.36, 3.95))],
    "6": [_o(0.5, 0.5, 3.5, 3.4, 1.1), _l(0.5, 1.6, 0.5, 4.3), _o(0.5, 2.0, 3.5, 5.5, 1.3, (-0.4, -0.4, 4.4, 4.3), (2.85, 4.3, 4.4, 4.95)), _BOTTOM],
    "7": [_l(0.5, 5.5, 3.5, 5.5), _l(3.5, 5.5, 1.4, 0.5)],
    "8": [_o(0.7, 3.0, 3.3, 5.5, 1.0), _o(0.5, 0.5, 3.5, 3.0, 1.05), _TOP, _BOTTOM],
    "9": [_o(0.5, 2.6, 3.5, 5.5, 1.1), _l(3.5, 1.7, 3.5, 4.4), _o(0.5, 0.5, 3.5, 4.0, 1.3, (-0.4, 1.7, 4.4, 4.4), (-0.4, 1.05, 1.15, 1.7)), _TOP],
    "-": [_l(0.9, 3.0, 3.1, 3.0)],
    "+": [_l(0.8, 3.0, 3.2, 3.0), _l(2.0, 1.8, 2.0, 4.2)],
    "/": [_l(0.6, 0.5, 3.4, 5.5)],
    ".": [_l(2.0, 0.6, 2.0, 0.6)],
    ":": [_l(2.0, 0.6, 2.0, 0.6), _l(2.0, 3.4, 2.0, 3.4)],
    "!": [_l(2.0, 2.3, 2.0, 5.5), _l(2.0, 0.6, 2.0, 0.6)],
    "#": [_l(1.3, 0.5, 1.3, 5.5), _l(2.7, 0.5, 2.7, 5.5), _l(0.4, 2.0, 3.6, 2.0), _l(0.4, 4.0, 3.6, 4.0)],
}
SPACE = 2.4


def glyph_sdf(ch, X, Y):
    d = np.full(X.shape, np.inf)
    gaps = []
    for kind, a, cuts in GLYPHS[ch]:
        if kind == "g":
            gaps.append(a)
            continue
        p = sd_stroke(X, Y, *a) if kind == "l" else sd_ring(X, Y, *a)
        for c in cuts:
            p = np.maximum(p, -sd_box(X, Y, *c))
        d = np.minimum(d, p)
    for g in gaps:
        d = np.maximum(d, -sd_box(X, Y, *g))
    # Inside the letter's cell: a flat foot and top.
    return np.maximum(d, sd_box(X, Y, -1.0, 0.0, 5.0, 6.0))


def glyph_bounds(ch):
    xs = []
    for kind, a, _ in GLYPHS[ch]:
        if kind == "l":
            xs += [a[0] - 0.5, a[2] + 0.5, a[0] + 0.5, a[2] - 0.5]
        elif kind == "o":
            xs += [a[0] - 0.5, a[2] + 0.5]
    return max(min(xs), -0.2), min(max(xs), 4.2)


def text_units(text, spacing=0.85):
    """How wide `text` is, in grid units."""
    w = 0.0
    for k, ch in enumerate(text):
        if ch == " ":
            w += SPACE
            continue
        lo, hi = glyph_bounds(ch)
        w += hi - lo + (spacing if k < len(text) - 1 else 0.0)
    return w


def blurred(a, sigma):
    """A Gaussian blur (separable; the edges held)."""
    r = max(1, int(math.ceil(sigma * 3)))
    x = np.arange(-r, r + 1)
    k = np.exp(-0.5 * (x / max(sigma, 1e-6)) ** 2)
    k /= k.sum()
    p = np.pad(a, r, mode="edge")
    p = np.apply_along_axis(lambda v: np.convolve(v, k, mode="valid"), 0, p)
    return np.apply_along_axis(lambda v: np.convolve(v, k, mode="valid"), 1, p)


class Canvas:
    """An RGBA picture in floats (straight alpha; colours in sRGB, 0..1), rows from the top."""

    def __init__(self, w, h):
        self.w, self.h = w, h
        self.rgba = np.zeros((h, w, 4), dtype=np.float64)

    def over(self, x0, y0, cover, colour):
        """Lays `colour` (sRGB) over a block of the picture as much as `cover` (0..1) says."""
        h, w = cover.shape
        x1, y1 = min(x0 + w, self.w), min(y0 + h, self.h)
        cx0, cy0 = max(x0, 0), max(y0, 0)
        if x1 <= cx0 or y1 <= cy0:
            return
        a = np.clip(cover[cy0 - y0:y1 - y0, cx0 - x0:x1 - x0], 0, 1)[..., None]
        dst = self.rgba[cy0:y1, cx0:x1]
        out_a = a + dst[..., 3:4] * (1 - a)
        col = np.asarray(colour, dtype=np.float64)[None, None, :3]
        rgb = (col * a + dst[..., :3] * dst[..., 3:4] * (1 - a)) / np.maximum(out_a, 1e-6)
        dst[..., :3] = rgb
        dst[..., 3:4] = out_a

    def grid(self, x0, y0, x1, y1):
        """Pixel centres of a block (as X, Y arrays) and the block's corner."""
        xs = np.arange(int(math.floor(x0)), int(math.ceil(x1)))
        ys = np.arange(int(math.floor(y0)), int(math.ceil(y1)))
        X, Y = np.meshgrid(xs + 0.5, ys + 0.5)
        return X, Y, (int(xs[0]) if len(xs) else 0, int(ys[0]) if len(ys) else 0)

    def shape(self, sd, bounds, colour, soft=1.0):
        """Fills where the signed distance (in pixels, a function of X, Y) is negative."""
        X, Y, (x0, y0) = self.grid(*bounds)
        if X.size == 0:
            return
        self.over(x0, y0, np.clip(0.5 - sd(X, Y) / soft, 0, 1), colour)

    def rect(self, x0, y0, x1, y1, colour, r=0.0):
        self.shape(lambda X, Y: sd_round_box(X, Y, x0, y0, x1, y1, r), (x0 - 1, y0 - 1, x1 + 1, y1 + 1), colour)

    def text(self, text, cx, cy, height, colour, align="centre", spacing=0.85, weight=0.0, spray=0.0, seed=0):
        """Stencil letters `height` pixels tall, their middle at `cy`; `cx` their middle (or
        their left or right end, by `align`). `weight` thickens them (units); `spray` adds
        the soft, speckled overspray of a stencil (its strength)."""
        k = height / 6.0
        total = text_units(text, spacing) * k
        x = cx - total / 2 if align == "centre" else (cx if align == "left" else cx - total)
        top = cy - height / 2
        rng = np.random.default_rng(seed)
        for ch in text:
            if ch == " ":
                x += SPACE * k
                continue
            lo, hi = glyph_bounds(ch)
            pad = 2 + (0.8 * k if spray else 0)
            X, Y, (x0, y0) = self.grid(x - pad, top - pad, x + (hi - lo) * k + pad, top + height + pad)
            gx = (X - x) / k + lo
            gy = (top + height - Y) / k
            cover = np.clip(0.5 - (glyph_sdf(ch, gx, gy) - weight) * k, 0, 1)
            if spray:
                # The paint that got past the stencil's edges: the letter blurred, speckled.
                mist = blurred(cover, 0.22 * k) * spray * (0.5 + 0.5 * rng.random(cover.shape))
                self.over(x0, y0, mist * (1 - cover), colour)
            self.over(x0, y0, cover, colour)
            x += (hi - lo + spacing) * k

    def bars(self, x0, y0, x1, rows, height, gap, colour, seed=0):
        """Small print too small to read: rows of words as bars."""
        rng = np.random.default_rng(seed)
        y = y0
        for r in range(rows):
            x = x0
            end = x1 - (x1 - x0) * (0.35 * rng.random() if r == rows - 1 else 0.05 * rng.random())
            while x < end:
                w = height * (1.2 + 4.5 * rng.random())
                w = min(w, end - x)
                if w > height * 0.8:
                    self.rect(x, y, x + w, y + height, colour, r=height * 0.25)
                x += w + height * 0.9
            y += height + gap


# ---------------------------------------------------------------------------------------------
# The decal atlas
# ---------------------------------------------------------------------------------------------

ATLAS_W = 1024
PAD = 10
WHITE = (0.86, 0.85, 0.80)
YELLOW = (0.88, 0.67, 0.10)
BLACK = (0.05, 0.05, 0.045)
RED = (0.72, 0.09, 0.06)
STEEL = (0.58, 0.58, 0.56)


def triangle_pts(cx, cy, r):
    return [(cx, cy - r), (cx + r * 0.866, cy + r * 0.5), (cx - r * 0.866, cy + r * 0.5)]


def paint_caution(c, x, y, w, h):
    c.rect(x, y, x + w, y + h, YELLOW, r=h * 0.04)
    t = h * 0.045
    c.shape(lambda X, Y: np.abs(sd_round_box(X, Y, x + t * 1.6, y + t * 1.6, x + w - t * 1.6, y + h - t * 1.6, t)) - t / 2, (x, y, x + w, y + h), BLACK)
    # The warning triangle with its mark.
    tx, ty, tr = x + h * 0.48, y + h * 0.56, h * 0.33
    pts = triangle_pts(tx, ty, tr)
    c.shape(lambda X, Y: np.abs(sd_poly(X, Y, pts)) - h * 0.03, (tx - tr, ty - tr, tx + tr, ty + tr), BLACK)
    c.text("!", tx, ty + h * 0.04, h * 0.3, BLACK)
    c.text("CAUTION", x + h * 0.88, y + h * 0.36, h * 0.27, BLACK, align="left", weight=0.05)
    c.bars(x + h * 0.88, y + h * 0.58, x + w - h * 0.14, 3, h * 0.05, h * 0.045, BLACK, seed=1)


def paint_danger(c, x, y, w, h):
    c.rect(x, y, x + w, y + h, WHITE, r=h * 0.04)
    c.rect(x + h * 0.06, y + h * 0.06, x + w - h * 0.06, y + h * 0.42, RED, r=h * 0.03)
    c.text("DANGER", x + w / 2, y + h * 0.24, h * 0.24, WHITE, weight=0.05)
    # High voltage: a bolt in a triangle.
    tx, ty, tr = x + h * 0.36, y + h * 0.7, h * 0.22
    pts = triangle_pts(tx, ty, tr)
    c.shape(lambda X, Y: sd_poly(X, Y, pts), (tx - tr, ty - tr, tx + tr, ty + tr), YELLOW)
    c.shape(lambda X, Y: np.abs(sd_poly(X, Y, pts)) - h * 0.018, (tx - tr - 3, ty - tr - 3, tx + tr + 3, ty + tr + 3), BLACK)
    s = tr * 0.55
    bolt = [(tx + 0.15 * s, ty - 0.75 * s), (tx - 0.35 * s, ty + 0.12 * s), (tx + 0.02 * s, ty + 0.12 * s), (tx - 0.18 * s, ty + 0.78 * s), (tx + 0.38 * s, ty - 0.12 * s), (tx + 0.02 * s, ty - 0.12 * s)]
    c.shape(lambda X, Y: sd_poly(X, Y, bolt), (tx - s, ty - s, tx + s, ty + s), BLACK)
    c.bars(x + h * 0.66, y + h * 0.56, x + w - h * 0.1, 3, h * 0.055, h * 0.05, BLACK, seed=2)


def paint_keep_clear(c, x, y, w, h):
    # Yellow and black bars round a yellow field.
    def stripes(X, Y):
        band = np.mod((X - x) + (Y - y), h * 0.36) < h * 0.18
        return np.where(band, -1.0, 1.0)
    c.rect(x, y, x + w, y + h, BLACK, r=h * 0.04)
    c.shape(lambda X, Y: np.maximum(stripes(X, Y), sd_box(X, Y, x, y, x + w, y + h)), (x, y, x + w, y + h), YELLOW, soft=0.01)
    c.rect(x + h * 0.16, y + h * 0.16, x + w - h * 0.16, y + h * 0.84, YELLOW, r=h * 0.03)
    c.text("KEEP CLEAR", x + w / 2, y + h / 2, h * 0.36, BLACK, weight=0.05)


def paint_data(c, x, y, w, h):
    # A steel plate, brushed, its words engraved.
    rng = np.random.default_rng(5)
    X, Y, (x0, y0) = c.grid(x, y, x + w, y + h)
    grain = rng.random((Y.shape[0], 1)) * 0.06 + rng.random(Y.shape) * 0.02
    plate = np.ones(X.shape)
    c.over(x0, y0, plate, STEEL)
    c.rgba[y0:y0 + X.shape[0], x0:x0 + X.shape[1], :3] *= (0.94 + grain)[..., None]
    engraved = (0.2, 0.2, 0.2)
    c.shape(lambda X, Y: np.abs(sd_round_box(X, Y, x + h * 0.07, y + h * 0.07, x + w - h * 0.07, y + h - h * 0.07, h * 0.03)) - h * 0.008, (x, y, x + w, y + h), engraved)
    c.text("MK IV", x + h * 0.16, y + h * 0.24, h * 0.17, engraved, align="left", weight=-0.12)
    c.text("SN 0417-2273", x + h * 0.16, y + h * 0.47, h * 0.12, engraved, align="left", weight=-0.15)
    c.text("LOT 27-11", x + h * 0.16, y + h * 0.66, h * 0.12, engraved, align="left", weight=-0.15)
    c.bars(x + h * 0.16, y + h * 0.79, x + w - h * 0.16, 1, h * 0.04, h * 0.04, engraved, seed=7)
    # A gear-like maker's mark at the right.
    mx, my, mr = x + w - h * 0.32, y + h * 0.36, h * 0.16

    def mark(X, Y):
        a = np.arctan2(Y - my, X - mx)
        rad = np.hypot(X - mx, Y - my)
        teeth = mr * (0.86 + 0.14 * (np.cos(a * 8) > 0))
        return np.maximum(rad - teeth, -(rad - mr * 0.45))
    c.shape(mark, (mx - mr - 2, my - mr - 2, mx + mr + 2, my + mr + 2), engraved)


def paint_label(text):
    def paint(c, x, y, w, h):
        c.rect(x, y, x + w, y + h, BLACK, r=h * 0.18)
        c.text(text, x + w / 2, y + h / 2, h * 0.52, WHITE, weight=0.0)
    return paint


def paint_stencil(text, colour=WHITE, size=0.72, seed=0):
    def paint(c, x, y, w, h):
        c.text(text, x + w / 2, y + h / 2, h * size, colour, spray=0.32, seed=seed, weight=0.04)
    return paint


def paint_safe_live(c, x, y, w, h):
    c.rect(x, y, x + w, y + h / 2, BLACK, r=h * 0.08)
    c.rect(x, y + h / 2, x + w, y + h, RED, r=h * 0.08)
    c.text("SAFE", x + w / 2, y + h * 0.25, h * 0.3, WHITE)
    c.text("LIVE", x + w / 2, y + h * 0.75, h * 0.3, WHITE)


LABELS = ["PWR", "IFF", "ECM", "FIRE", "AUX", "NAV"]

# (name, width, height, painter): the atlas's items, packed in this order.
ITEMS = (
    [("caution", 384, 160, paint_caution), ("danger", 384, 176, paint_danger), ("keep_clear", 384, 104, paint_keep_clear),
     ("data", 352, 208, paint_data), ("safe_live", 112, 128, paint_safe_live)]
    + [(f"stencil.{t}", int(96 * (text_units(t) * 0.72 / 6 + 0.4)), 96, paint_stencil(t, seed=k)) for k, t in enumerate(["VB-1", "VB-2", "A-7", "P-14", "24V", "HYD 3", "NO STEP", "07", "12", "C-3", "GUN 2"])]
    + [(f"yellow.{t}", int(128 * (text_units(t) * 0.72 / 6 + 0.4)), 128, paint_stencil(t, YELLOW, seed=20 + k)) for k, t in enumerate(["1", "2"])]
    + [(f"label.{t}", 112 if len(t) < 4 else 136, 40, paint_label(t)) for t in LABELS]
)


def layout():
    """Each item's place in the atlas (x, y, width, height in pixels, from the top left) and the
    atlas's size: shelves, the tallest first."""
    order = sorted(range(len(ITEMS)), key=lambda i: -ITEMS[i][2])
    rects = {}
    x, y, shelf = PAD, PAD, 0
    for i in order:
        name, w, h, _ = ITEMS[i]
        if x + w + PAD > ATLAS_W:
            x, y, shelf = PAD, y + shelf + PAD * 2, 0
        rects[name] = (x, y, w, h)
        x += w + PAD * 2
        shelf = max(shelf, h)
    height = (y + shelf + PAD + 63) // 64 * 64
    return rects, (ATLAS_W, height)


DECALS, ATLAS_SIZE = layout()


def bleed(rgba, passes=PAD + 6):
    """Spreads each item's colours out into the clear pixels round it (its alpha stays), so that
    filtering and mipmaps never pull in black."""
    rgb = rgba[..., :3].copy()
    known = rgba[..., 3] > 0.004
    for _ in range(passes):
        acc = np.zeros_like(rgb)
        cnt = np.zeros(known.shape)
        for dy in (-1, 0, 1):
            for dx in (-1, 0, 1):
                if dx == 0 and dy == 0:
                    continue
                k = np.roll(np.roll(known, dy, 0), dx, 1)
                acc += np.roll(np.roll(rgb, dy, 0), dx, 1) * k[..., None]
                cnt += k
        grow = (~known) & (cnt > 0)
        rgb[grow] = acc[grow] / cnt[grow][..., None]
        known = known | grow
    out = rgba.copy()
    out[..., :3] = rgb
    return out


def draw_decals(path):
    w, h = ATLAS_SIZE
    c = Canvas(w, h)
    for name, iw, ih, paint in ITEMS:
        x, y, _, _ = DECALS[name]
        paint(c, x, y, iw, ih)
    out = bleed(c.rgba)
    write_png(path, np.clip(np.round(out * 255), 0, 255).astype(np.uint8))
    return path


# ---------------------------------------------------------------------------------------------
# The wear texture (inside Blender)
# ---------------------------------------------------------------------------------------------


def equalised(a):
    """The values' ranks, 0..1: a threshold t then keeps a share 1 - t of the picture."""
    flat = a.reshape(-1)
    ranks = np.empty(flat.size)
    ranks[np.argsort(flat, kind="stable")] = np.linspace(0.0, 1.0, flat.size)
    return ranks.reshape(a.shape)


def scratches(size, seed=11):
    """Scratches, drawn: thin lines of every length, most near level (wiped across), some any
    way; each fading at its ends, deepest in its middle. Tiles (drawn round the edges)."""
    rng = np.random.default_rng(seed)
    out = np.zeros((size, size))
    for k in range(650):
        length = size * (0.012 + 0.2 * rng.random() ** 2.4)
        angle = rng.normal(0.0, 0.35) if rng.random() < 0.7 else rng.random() * math.pi
        width = 0.45 + 0.9 * rng.random() ** 2
        depth = 0.25 + 0.75 * rng.random()
        cx, cy = rng.random() * size, rng.random() * size
        dx, dy = math.cos(angle) * length / 2, math.sin(angle) * length / 2
        x0, x1 = int(math.floor(cx - abs(dx) - 3)), int(math.ceil(cx + abs(dx) + 3))
        y0, y1 = int(math.floor(cy - abs(dy) - 3)), int(math.ceil(cy + abs(dy) + 3))
        xs, ys = np.arange(x0, x1) + 0.5, np.arange(y0, y1) + 0.5
        X, Y = np.meshgrid(xs, ys)
        ex, ey = 2 * dx, 2 * dy
        t = np.clip(((X - (cx - dx)) * ex + (Y - (cy - dy)) * ey) / max(ex * ex + ey * ey, 1e-9), 0, 1)
        d = np.hypot(X - (cx - dx) - ex * t, Y - (cy - dy) - ey * t)
        v = np.clip(1 - d / width, 0, 1) * depth * np.sin(np.pi * t) ** 0.6
        iy = np.mod(np.arange(y0, y1), size)
        ix = np.mod(np.arange(x0, x1), size)
        block = out[np.ix_(iy, ix)]
        out[np.ix_(iy, ix)] = np.maximum(block, v)
    return out


def bake_wear(path, size=512, samples=16):
    """Bakes Blender's noise round a torus in four dimensions (so that it tiles) into the chip,
    grime and grain fields, draws the scratches, and writes `wear.png`."""
    import bpy
    import bmesh

    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = 'CYCLES'
    scene.cycles.samples = samples
    try:
        scene.cycles.device = 'GPU'
    except Exception:
        pass
    me = bpy.data.meshes.new("tile")
    bm = bmesh.new()
    face = bm.faces.new([bm.verts.new(v) for v in [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)]])
    uv = bm.loops.layers.uv.new("UVMap")
    for loop, t in zip(face.loops, [(0, 0), (1, 0), (1, 1), (0, 1)]):
        loop[uv].uv = t
    bm.to_mesh(me)
    bm.free()
    obj = bpy.data.objects.new("tile", me)
    scene.collection.objects.link(obj)
    mat = bpy.data.materials.new("wear")
    mat.use_nodes = True
    nodes, links = mat.node_tree.nodes, mat.node_tree.links
    nodes.clear()
    out = nodes.new("ShaderNodeOutputMaterial")
    emit = nodes.new("ShaderNodeEmission")
    links.new(emit.outputs["Emission"], out.inputs["Surface"])
    coords = nodes.new("ShaderNodeTexCoord")
    sep = nodes.new("ShaderNodeSeparateXYZ")
    links.new(coords.outputs["UV"], sep.inputs[0])

    def math_node(op, a, b=None):
        n = nodes.new("ShaderNodeMath")
        n.operation = op
        for k, v in enumerate((a, b)):
            if v is None:
                continue
            if isinstance(v, (int, float)):
                n.inputs[k].default_value = v
            else:
                links.new(v, n.inputs[k])
        return n.outputs[0]

    t1 = math_node('MULTIPLY', sep.outputs["X"], math.tau)
    t2 = math_node('MULTIPLY', sep.outputs["Y"], math.tau)
    ring = [math_node('COSINE', t1), math_node('SINE', t1), math_node('COSINE', t2), math_node('SINE', t2)]

    def torus(radius, offset):
        """A point on a torus in four dimensions: going once round the tile goes once round
        each circle (about 2π `radius` features fit across it)."""
        xyz = nodes.new("ShaderNodeCombineXYZ")
        for k in range(3):
            links.new(math_node('ADD', math_node('MULTIPLY', ring[k], radius), offset[k]), xyz.inputs[k])
        return xyz.outputs[0], math_node('ADD', math_node('MULTIPLY', ring[3], radius), offset[3])

    def noise(radius, detail, roughness, offset, lacunarity=2.0, distortion=0.0):
        n = nodes.new("ShaderNodeTexNoise")
        n.noise_dimensions = '4D'
        vec, w = torus(radius, offset)
        links.new(vec, n.inputs["Vector"])
        links.new(w, n.inputs["W"])
        n.inputs["Scale"].default_value = 1.0
        n.inputs["Detail"].default_value = detail
        n.inputs["Roughness"].default_value = roughness
        n.inputs["Lacunarity"].default_value = lacunarity
        n.inputs["Distortion"].default_value = distortion
        return n.outputs["Fac"]

    def voronoi(radius, offset):
        n = nodes.new("ShaderNodeTexVoronoi")
        n.voronoi_dimensions = '4D'
        vec, w = torus(radius, offset)
        links.new(vec, n.inputs["Vector"])
        links.new(w, n.inputs["W"])
        n.inputs["Scale"].default_value = 1.0
        return n.outputs["Distance"]

    # Chips: ragged noise of every size over broken cells (angular flakes); grime: clouds a
    # tile or two across; grain: a pixel or two.
    chips = math_node('ADD', math_node('MULTIPLY', noise(0.95, 15.0, 0.7, (3.1, 7.7, 1.3, 9.2), 2.2, 0.45), 0.76), math_node('MULTIPLY', voronoi(2.4, (5.5, 2.2, 8.1, 4.4)), 0.24))
    grime = noise(0.34, 8.0, 0.6, (11.0, 4.0, 6.5, 2.5), 2.1, 0.3)
    grain = noise(36.0, 2.0, 0.5, (1.5, 13.0, 3.3, 7.1))
    rgb = nodes.new("ShaderNodeCombineXYZ")
    links.new(chips, rgb.inputs[0])
    links.new(grime, rgb.inputs[1])
    links.new(grain, rgb.inputs[2])
    links.new(rgb.outputs[0], emit.inputs["Color"])
    img = bpy.data.images.new("wear", size, size, alpha=False, float_buffer=True)
    img.colorspace_settings.name = 'Non-Color'
    tex = nodes.new("ShaderNodeTexImage")
    tex.image = img
    nodes.active = tex
    tex.select = True
    obj.data.materials.append(mat)
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    scene.render.bake.target = 'IMAGE_TEXTURES'
    scene.render.bake.margin = 0
    bpy.ops.object.bake(type='EMIT')
    px = np.array(img.pixels[:], dtype=np.float64).reshape(size, size, 4)[::-1]
    rgba = np.zeros((size, size, 4))
    rgba[..., 0] = equalised(px[..., 0])
    rgba[..., 1] = scratches(size)
    rgba[..., 2] = equalised(px[..., 1])
    rgba[..., 3] = equalised(px[..., 2])
    write_png(path, np.clip(np.round(rgba * 255), 0, 255).astype(np.uint8))
    return path
