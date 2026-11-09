"""Draws the modern HUD's order and state icons (original art for TA Reborn): each glyph drawn as
an SVG on a 64-unit grid, rasterised by resvg four times over, given a die-stamped bevel lit from
the top left (a height field from the glyph's coverage, its normals, a key light and a glint),
and brought down to each size the interface draws it at.

The atlas is one row per size (SIZES), one cell per glyph in the order of GLYPHS (the order of
`hud_icons::Glyph`), each cell the size plus PAD empty pixels on every side so that filtering never
reaches a neighbour. A pixel's grey is its lighting (the game tints it: the flat face about 0.86,
lit edges up to 1, shaded ones down to 0.5) and its alpha the glyph's coverage. There are no
mipmaps in egui's wgpu renderer, so the game picks the row nearest the size on screen.

    python tools/hud/icons.py [--out apps/ta_game/hud/icons.png] [--preview <dir>]

Needs numpy, Pillow and resvg (https://github.com/linebender/resvg) on the PATH.
"""
import argparse
import math
import os
import shutil
import subprocess
import tempfile

import numpy as np
from PIL import Image

# The order of hud_icons::Glyph.
GLYPHS = [
    "stop", "move", "attack", "fight", "patrol", "guard", "capture", "repair", "load", "unload",
    "resurrect", "reclaim", "dgun", "hold_fire", "return_fire", "fire_at_will", "hold_position",
    "maneuver", "roam", "cloaked", "visible", "power", "self_destruct", "area_mex", "perspective",
    "build", "info", "back", "more", "cancel", "duty", "queue",
]
# Pixel sizes: hud_icons::SIZES.
SIZES = [14, 16, 18, 20, 24, 28, 32, 40, 48, 64]
PAD = 2
SUPER = 4

W = 6.5  # the stroke of most lines, in grid units (2 px at 20 px)

# ---------------------------------------------------------------------------------------------
# Drawing helpers (SVG fragments in the 64-unit grid, white)
# ---------------------------------------------------------------------------------------------


def line(x1, y1, x2, y2, w=W):
    return f'<line x1="{x1:.2f}" y1="{y1:.2f}" x2="{x2:.2f}" y2="{y2:.2f}" stroke="white" stroke-width="{w}" stroke-linecap="round"/>'


def poly(points, fill="white"):
    pts = " ".join(f"{x:.2f},{y:.2f}" for x, y in points)
    return f'<polygon points="{pts}" fill="{fill}" stroke="{fill}" stroke-width="1.5" stroke-linejoin="round"/>'


def circle(cx, cy, r, fill="white", stroke=None, w=W):
    if stroke:
        return f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="{stroke}" stroke-width="{w}"/>'
    return f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{fill}"/>'


def ring(cx, cy, r, w=W, dash=None):
    d = f' stroke-dasharray="{dash}"' if dash else ""
    return f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="white" stroke-width="{w}"{d}/>'


def path(d, w=W, fill="none"):
    if fill != "none":
        return f'<path d="{d}" fill="{fill}"/>'
    return f'<path d="{d}" fill="none" stroke="white" stroke-width="{w}" stroke-linecap="round" stroke-linejoin="round"/>'


def arc_d(cx, cy, r, a0, a1):
    """An arc path from angle a0 to a1 (degrees, clockwise from the right, y down)."""
    x0, y0 = cx + r * math.cos(math.radians(a0)), cy + r * math.sin(math.radians(a0))
    x1, y1 = cx + r * math.cos(math.radians(a1)), cy + r * math.sin(math.radians(a1))
    large = 1 if abs(a1 - a0) > 180 else 0
    sweep = 1 if a1 > a0 else 0
    return f"M{x0:.2f},{y0:.2f} A{r},{r} 0 {large},{sweep} {x1:.2f},{y1:.2f}"


def head(tx, ty, dx, dy, length=15, half=10):
    """A filled arrowhead with its tip at (tx, ty) pointing along (dx, dy)."""
    n = math.hypot(dx, dy)
    dx, dy = dx / n, dy / n
    bx, by = tx - dx * length, ty - dy * length
    return poly([(tx, ty), (bx - dy * half, by + dx * half), (bx + dy * half, by - dx * half)])


def masked(body, holes):
    """`body` with `holes` (black shapes) cut out of it."""
    mid = f"m{abs(hash(body + holes)) % 10**8}"
    return (f'<mask id="{mid}" maskUnits="userSpaceOnUse" x="0" y="0" width="64" height="64">'
            f'<rect width="64" height="64" fill="white"/>{holes}</mask><g mask="url(#{mid})">{body}</g>')


def black(fragment):
    return fragment.replace('"white"', '"black"')


def crosshair(w=W, inner=True):
    s = ring(32, 32, 17, w)
    s += line(32, 4, 32, 15, w) + line(32, 49, 32, 60, w) + line(4, 32, 15, 32, w) + line(49, 32, 60, 32, w)
    return s


def eye(w=W):
    return path("M5,32 C15,16 49,16 59,32 C49,48 15,48 5,32 Z", w)


def octagon(r, cx=32, cy=32):
    return [(cx + r * math.cos(math.radians(22.5 + 45 * k)), cy + r * math.sin(math.radians(22.5 + 45 * k))) for k in range(8)]


def star(cx, cy, outer, inner, points):
    return [(cx + (outer if k % 2 == 0 else inner) * math.cos(math.radians(-90 + 180 * k / points)),
             cy + (outer if k % 2 == 0 else inner) * math.sin(math.radians(-90 + 180 * k / points))) for k in range(2 * points)]


# ---------------------------------------------------------------------------------------------
# The glyphs
# ---------------------------------------------------------------------------------------------


def glyph(name):
    if name == "stop":
        # A stop sign: an octagon with a bar struck out of it.
        return masked(poly(octagon(27)), '<rect x="15" y="28" width="34" height="8" rx="1.5" fill="black"/>')
    if name == "move":
        # Off to the top right from a mark on the ground.
        return line(13, 51, 41, 23, 7.5) + head(55, 9, 1, -1, 19, 12) + circle(13, 51, 6)
    if name == "attack":
        return crosshair() + circle(32, 32, 5.5)
    if name == "fight":
        # Crossed swords, the one in front with a gap cut round it where it crosses the other:
        # pointed blades, broad guards, grips and pommels.
        def sword(f):
            x = lambda v: 32 + f * (v - 32)
            return (poly([(x(7), 7), (x(15), 10), (x(42), 37), (x(37), 42), (x(10), 15)])
                    + line(x(33), 51, x(51), 33, 7) + line(x(44), 44, x(52), 52, 6.5) + circle(x(55), 55, 4.5))
        front_blade = black(line(9, 9, 40, 40, 15))
        return masked(sword(-1), front_blade) + sword(1)
    if name == "patrol":
        # Round and round: two arcs chasing each other.
        s = path(arc_d(32, 32, 21, 196, 322), W) + head(32 + 21 * math.cos(math.radians(340)), 32 + 21 * math.sin(math.radians(340)), math.cos(math.radians(70)), math.sin(math.radians(70)), 14, 10)
        s += path(arc_d(32, 32, 21, 16, 142), W) + head(32 + 21 * math.cos(math.radians(160)), 32 + 21 * math.sin(math.radians(160)), math.cos(math.radians(250)), math.sin(math.radians(250)), 14, 10)
        return s
    if name == "guard":
        # A shield with a ridge down it.
        shield = "M32,5 L55,12 L53,33 C51,45 42,53 32,59 C22,53 13,45 11,33 L9,12 Z"
        return path(shield, W) + line(32, 17, 32, 46, 6)
    if name == "capture":
        return line(17, 8, 17, 57, 6.5) + poly([(20, 8), (54, 19), (20, 31)]) + line(9, 57, 27, 57, 6)
    if name == "repair":
        # A wrench: its handle, and its head with the jaw open to the top right.
        body = line(12, 52, 36, 28, 10) + circle(42, 22, 15)
        cut = black(poly([(44, 22), (63, 3), (63, 22), (54, 31)])) + black(circle(42, 22, 6))
        return masked(body, cut)
    if name == "load":
        # A crate, and an arrow up into it.
        return path("M10,8 H54 V30 H10 Z", 6) + line(32, 59, 32, 44, 7) + head(32, 30, 0, -1, 15, 11)
    if name == "unload":
        return path("M10,8 H54 V30 H10 Z", 6) + line(32, 24, 32, 46, 7) + head(32, 60, 0, 1, 15, 11)
    if name == "resurrect":
        # Rising from the ground, with a spark.
        return line(6, 56, 58, 56, 6) + line(24, 50, 24, 24, 7) + head(24, 8, 0, -1, 15, 11) + poly(star(47, 22, 11, 3.5, 4))
    if name == "reclaim":
        # A hopper taking something in.
        return path("M7,28 H57 L43,57 H21 Z", 6) + line(32, 3, 32, 22, 7) + head(32, 37, 0, 1, 14, 10)
    if name == "dgun":
        return poly(star(32, 32, 29, 12, 8))
    if name == "hold_fire":
        body = crosshair(5.5)
        return masked(body, line(10, 54, 54, 10, 15).replace('"white"', '"black"')) + line(13, 51, 51, 13, 7)
    if name == "return_fire":
        return crosshair() + ring(32, 32, 6, 4.5)
    if name == "fire_at_will":
        return crosshair() + circle(32, 32, 10)
    if name == "hold_position":
        # A pin in the ground.
        pin = "M32,58 C26,46 15,36 15,24 A17,17 0 1,1 49,24 C49,36 38,46 32,58 Z"
        return masked(path(pin, fill="white"), black(circle(32, 24, 7))) + line(14, 60, 50, 60, 5)
    if name == "maneuver":
        return line(14, 32, 50, 32, 7) + head(4, 32, -1, 0, 14, 11) + head(60, 32, 1, 0, 14, 11) + circle(32, 32, 7)
    if name == "roam":
        s = circle(32, 32, 5.5)
        for a in (45, 135, 225, 315):
            dx, dy = math.cos(math.radians(a)), math.sin(math.radians(a))
            s += line(32 + dx * 10, 32 + dy * 10, 32 + dx * 20, 32 + dy * 20, 6.5) + head(32 + dx * 30, 32 + dy * 30, dx, dy, 12, 9)
        return s
    if name == "visible":
        return eye() + masked(circle(32, 32, 11), black(circle(36, 28, 3.5)))
    if name == "cloaked":
        body = eye() + circle(32, 32, 11)
        return masked(body, black(line(8, 56, 56, 8, 15))) + line(10, 54, 54, 10, 7)
    if name == "power":
        return path(arc_d(32, 34, 21, -55, 235), 7) + line(32, 5, 32, 30, 7.5)
    if name == "self_destruct":
        # A bomb with a lit fuse.
        body = circle(27, 39, 19) + poly([(36, 17), (46, 27), (40, 33), (30, 23)])
        s = masked(body, black(circle(20, 32, 5)))
        s += path("M43,22 C48,16 50,14 54,13", 4.5)
        s += poly(star(55, 11, 8.5, 2.5, 4))
        return s
    if name == "area_mex":
        hexagon = [(32 + 13 * math.cos(math.radians(30 + 60 * k)), 32 + 13 * math.sin(math.radians(30 + 60 * k))) for k in range(6)]
        return ring(32, 32, 27, 5, "9.6,4.54") + masked(poly(hexagon), black(circle(32, 32, 4.5)))
    if name == "perspective":
        # A visor with a sight for its pupil.
        return eye() + ring(32, 32, 9.5, 4.5) + line(32, 19, 32, 26, 4) + line(32, 38, 32, 45, 4) + line(19, 32, 26, 32, 4) + line(38, 32, 45, 32, 4)
    if name == "build":
        # A crane: its mast and jib, the cable and the hook's block, and its foot.
        return (line(16, 58, 16, 8, 6.5) + line(8, 11, 58, 11, 6.5) + line(16, 24, 34, 11, 4.5) + line(47, 11, 47, 33, 4)
                + poly([(40, 33), (54, 33), (54, 48), (40, 48)]) + line(8, 58, 28, 58, 6.5))
    if name == "info":
        # An "i" in a ring.
        return ring(32, 32, 26, 5.5) + circle(32, 19.5, 4.2) + line(32, 29, 32, 46, 6.5)
    if name == "back":
        # An arrow to the left, bent back on itself.
        return path("M52,46 C52,30 42,22 24,22", 7) + head(9, 22, -1, 0, 18, 13)
    if name == "more":
        return circle(11, 32, 8.5) + circle(32, 32, 8.5) + circle(53, 32, 8.5)
    if name == "cancel":
        return line(13, 13, 51, 51, 8) + line(51, 13, 13, 51, 8)
    if name == "duty":
        # A cog: work that goes on by itself. Eight teeth round a hub with a hole through it.
        pts = []
        for k in range(8):
            a = 45 * k
            for da, r in ((-14, 22.5), (-8, 29.5), (8, 29.5), (14, 22.5)):
                pts.append((32 + r * math.cos(math.radians(a + da)), 32 + r * math.sin(math.radians(a + da))))
        return masked(poly(pts), black(circle(32, 32, 9.5)))
    if name == "queue":
        # A crate on a belt, on its way out: what a factory makes, one after another.
        return (poly([(8, 8), (36, 8), (36, 36), (8, 36)]) + line(41, 22, 49, 22, 6) + head(61, 22, 1, 0, 14, 10)
                + line(5, 47, 59, 47, 7) + circle(14, 57, 4.2) + circle(32, 57, 4.2) + circle(50, 57, 4.2))
    raise KeyError(name)


def svg(name):
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">{glyph(name)}</svg>'


# ---------------------------------------------------------------------------------------------
# Rasterising and lighting
# ---------------------------------------------------------------------------------------------


def rasterise(name, px, tmp, resvg):
    src = os.path.join(tmp, f"{name}.svg")
    out = os.path.join(tmp, f"{name}_{px}.png")
    with open(src, "w", encoding="utf-8") as f:
        f.write(svg(name))
    subprocess.run([resvg, "--width", str(px), "--height", str(px), src, out], check=True, capture_output=True)
    return np.asarray(Image.open(out).convert("RGBA"), dtype=np.float32)[..., 3] / 255.0


def blur(a, r):
    """A box blur of radius r, three times (near enough a gaussian)."""
    if r < 1:
        return a
    k = 2 * r + 1
    for _ in range(3):
        for axis in (0, 1):
            c = np.cumsum(np.pad(a, [(r + 1, r)] if axis == 0 and a.ndim == 1 else ([(r + 1, r), (0, 0)] if axis == 0 else [(0, 0), (r + 1, r)]), mode="edge"), axis=axis)
            a = (np.take(c, range(k, c.shape[axis]), axis=axis) - np.take(c, range(0, c.shape[axis] - k), axis=axis)) / k
    return a


def lit(alpha, px):
    """The die-stamped bevel's lighting for a glyph's coverage at `px` pixels a side."""
    bevel = max(1, round(px * 0.05))
    # Height: the coverage, rounded at its edges.
    h = blur(alpha, bevel)
    h = np.clip(h * alpha * 1.0, 0, 1)
    gy, gx = np.gradient(h)
    k = 64.0 / px * 9.0  # the same slope at every size
    n = np.dstack([-gx * k * px / 64.0, -gy * k * px / 64.0, np.ones_like(h)])
    n /= np.linalg.norm(n, axis=2, keepdims=True)
    light = np.array([-0.55, -0.65, 0.52])
    light /= np.linalg.norm(light)
    view = np.array([0.0, 0.0, 1.0])
    half = (light + view) / np.linalg.norm(light + view)
    flat = light[2]
    diffuse = np.clip(n @ light, 0, 1)
    spec = np.clip(n @ half, 0, 1) ** 24
    shade = 0.86 + 0.42 * (diffuse - flat) + 0.22 * spec
    return np.clip(shade, 0.45, 1.0)


def icon(name, px, tmp, resvg):
    big = px * SUPER
    alpha = rasterise(name, big, tmp, resvg)
    shade = lit(alpha, big)
    # Down to size: premultiplied, so the edges keep their light.
    rgba = np.dstack([shade * alpha, alpha])
    img = Image.fromarray((np.clip(rgba, 0, 1) * 255).astype(np.uint8), "LA").resize((px, px), Image.LANCZOS)
    la = np.asarray(img, dtype=np.float32) / 255.0
    a = la[..., 1]
    g = np.where(a > 1e-3, la[..., 0] / np.maximum(a, 1e-3), 0.0)
    return np.clip(g, 0, 1), np.clip(a, 0, 1)


def atlas(resvg):
    width = max((s + 2 * PAD) * len(GLYPHS) for s in SIZES)
    height = sum(s + 2 * PAD for s in SIZES)
    out = np.zeros((height, width, 4), dtype=np.float32)
    y = 0
    with tempfile.TemporaryDirectory() as tmp:
        for s in SIZES:
            for i, name in enumerate(GLYPHS):
                g, a = icon(name, s, tmp, resvg)
                x = i * (s + 2 * PAD) + PAD
                out[y + PAD:y + PAD + s, x:x + s, 0:3] = g[..., None]
                out[y + PAD:y + PAD + s, x:x + s, 3] = a
            y += s + 2 * PAD
    return out


def preview(sheet, path):
    """The icons on gunmetal buttons at two sizes, lettered as the HUD tints them."""
    rows = []
    for s, tint in ((20, (214, 206, 180)), (20, (255, 248, 208)), (40, (214, 206, 180))):
        y = sum(z + 2 * PAD for z in SIZES[:SIZES.index(s)])
        cell = max(56, s + 26)
        row = np.zeros((cell, cell * len(GLYPHS), 3), dtype=np.float32)
        top, bottom = np.array([64, 62, 55]) / 255, np.array([34, 33, 29]) / 255
        for i in range(len(GLYPHS)):
            grad = np.linspace(0, 1, cell)[:, None, None]
            row[:, i * cell:(i + 1) * cell] = top + (bottom - top) * grad
            row[:, i * cell] = row[:, (i + 1) * cell - 1] = 0.05
            x = i * (s + 2 * PAD) + PAD
            g = sheet[y + PAD:y + PAD + s, x:x + s, 0]
            a = sheet[y + PAD:y + PAD + s, x:x + s, 3]
            ox, oy = i * cell + (cell - s) // 2, (cell - s) // 2
            for dx, dy, col, k in ((1, 1, np.zeros(3), 0.75), (0, 0, None, 1.0)):
                region = row[oy + dy:oy + dy + s, ox + dx:ox + dx + s]
                c = (g[..., None] * np.array(tint) / 255) if col is None else col
                region[:] = region * (1 - a[..., None] * k) + c * a[..., None] * k
        rows.append(row)
    w = max(r.shape[1] for r in rows)
    img = np.concatenate([np.pad(r, [(0, 4), (0, w - r.shape[1]), (0, 0)]) for r in rows])
    Image.fromarray((np.clip(img, 0, 1) * 255).astype(np.uint8)).save(path)


def main():
    ap = argparse.ArgumentParser()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    ap.add_argument("--out", default=os.path.join(root, "apps", "ta_game", "hud", "icons.png"))
    ap.add_argument("--preview", default=None)
    args = ap.parse_args()
    resvg = shutil.which("resvg")
    if not resvg:
        raise SystemExit("resvg is not on the PATH: https://github.com/linebender/resvg")
    sheet = atlas(resvg)
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    Image.fromarray((np.clip(sheet, 0, 1) * 255).round().astype(np.uint8), "RGBA").save(args.out, optimize=True)
    print(f"{args.out}: {sheet.shape[1]}x{sheet.shape[0]}, {len(GLYPHS)} glyphs at {SIZES}")
    if args.preview:
        os.makedirs(args.preview, exist_ok=True)
        preview(sheet, os.path.join(args.preview, "hud_icons.png"))
        for name in GLYPHS:
            with open(os.path.join(args.preview, f"{name}.svg"), "w", encoding="utf-8") as f:
                f.write(svg(name))
        print(f"preview in {args.preview}")


if __name__ == "__main__":
    main()
