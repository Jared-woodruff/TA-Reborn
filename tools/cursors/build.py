"""Draws the launcher's pointers (original art for TA Reborn): each one in shapes on a grid of units
(a unit is a point at pointer size 1), every part given its metal (brass, gunmetal, a dark
recess) and its relief, lit from the top left with a bevel, a specular glint and a polish from
top to bottom, then the whole outlined in near-black and given a soft shadow so that it reads on
any background. The pictures are drawn SS times over and brought down to PX pixels a unit.

    python tools/cursors/build.py [--out apps/ta_launcher/cursors] [--preview <dir>]

The launcher (`apps/ta_launcher/src/cursor.rs`) loads each picture as one texture with mipmaps
and puts its hotspot (SPRITES below, the same numbers as `cursor::Sprite`) on the pointer. What
moves (a lamp breathing, a light running along the brass, a turning cog) it paints itself.

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

PX = 6  # pixels a unit in the shipped pictures (3x pointer size on a 2x display, never enlarged)
SS = 2  # drawn this many times over for the lighting

# Metals: albedo, specular colour, shininess, specular strength.
METALS = {
    "brass": ((0.86, 0.64, 0.29), (1.0, 0.93, 0.72), 28.0, 0.70),
    "brass_dark": ((0.55, 0.38, 0.15), (0.95, 0.80, 0.55), 22.0, 0.45),
    "gunmetal": ((0.40, 0.41, 0.42), (0.88, 0.91, 0.95), 36.0, 0.55),
    "steel": ((0.62, 0.63, 0.62), (1.0, 1.0, 1.0), 40.0, 0.65),
    "dark": ((0.085, 0.09, 0.09), (0.55, 0.58, 0.60), 30.0, 0.30),
    "rust": ((0.66, 0.17, 0.11), (1.0, 0.70, 0.60), 26.0, 0.55),
    "glass": ((0.10, 0.17, 0.19), (0.80, 0.95, 1.0), 60.0, 0.85),
    "green": ((0.34, 0.86, 0.30), (0.85, 1.0, 0.80), 30.0, 0.40),
}
EDGE = (0.030, 0.034, 0.028)
LIGHT = np.array([-0.52, -0.72, 0.58])
LIGHT /= np.linalg.norm(LIGHT)
HALF = (LIGHT + np.array([0.0, 0.0, 1.0]))
HALF /= np.linalg.norm(HALF)

OUTLINE = 1.15  # units of near-black round the whole pointer
SHADOW = (0.85, 1.25, 1.5, 0.48)  # offset x, y, blur (units), opacity


# ---------------------------------------------------------------------------------------------
# Shapes (SVG in units, no fill or stroke of their own: the layer gives them)
# ---------------------------------------------------------------------------------------------


def poly(*points):
    return '<polygon points="' + " ".join(f"{x:.3f},{y:.3f}" for x, y in points) + '"/>'


def rect(x0, y0, x1, y1, r=0.0):
    return f'<rect x="{x0:.3f}" y="{y0:.3f}" width="{x1 - x0:.3f}" height="{y1 - y0:.3f}" rx="{r:.3f}"/>'


def circle(cx, cy, r):
    return f'<circle cx="{cx:.3f}" cy="{cy:.3f}" r="{r:.3f}"/>'


def ring(cx, cy, r0, r1):
    """An annulus between radii r0 < r1."""
    return (f'<path fill-rule="evenodd" d="M{cx + r1:.3f},{cy:.3f} A{r1},{r1} 0 1,0 {cx - r1:.3f},{cy:.3f} '
            f'A{r1},{r1} 0 1,0 {cx + r1:.3f},{cy:.3f} Z M{cx + r0:.3f},{cy:.3f} A{r0},{r0} 0 1,0 {cx - r0:.3f},{cy:.3f} '
            f'A{r0},{r0} 0 1,0 {cx + r0:.3f},{cy:.3f} Z"/>')


def capsule(x0, y0, x1, y1, w):
    """A rounded bar of width w between two points (a filled shape, so that it can grow)."""
    length = math.hypot(x1 - x0, y1 - y0)
    angle = math.degrees(math.atan2(y1 - y0, x1 - x0))
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    return rotated(rect(cx - length / 2 - w / 2, cy - w / 2, cx + length / 2 + w / 2, cy + w / 2, w / 2), angle, cx, cy)


def path(d, w=None):
    if w is None:
        return f'<path d="{d}"/>'
    return f'<path d="{d}" fill="none" stroke-width="{w:.3f}" stroke-linecap="round" stroke-linejoin="round"/>'


def rotated(fragment, angle, cx, cy):
    return f'<g transform="rotate({angle:.3f} {cx:.3f} {cy:.3f})">{fragment}</g>'


def gear(cx, cy, r_out, r_in, teeth, hole):
    """A cog: teeth between r_in and r_out, a hole of radius `hole`."""
    points = []
    for i in range(teeth * 4):
        a = math.tau * i / (teeth * 4) - math.pi / 2
        r = r_out if i % 4 in (1, 2) else r_in
        points.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    d = "M" + " L".join(f"{x:.3f},{y:.3f}" for x, y in points) + " Z"
    d += (f" M{cx + hole:.3f},{cy:.3f} A{hole},{hole} 0 1,0 {cx - hole:.3f},{cy:.3f} "
          f"A{hole},{hole} 0 1,0 {cx + hole:.3f},{cy:.3f} Z")
    return f'<path fill-rule="evenodd" d="{d}"/>'


# ---------------------------------------------------------------------------------------------
# The pointers. Each: canvas (units), hotspot (units), layers bottom to top as
# (shapes, metal, relief, bevel in units, inset in units). Relief "raised" stands out of the
# plate, "sunk" is pressed into it; an inset shrinks the shapes (a panel inside a rim).
# ---------------------------------------------------------------------------------------------

ARROW = [(3.0, 3.0), (3.0, 31.0), (10.0, 24.4), (14.9, 34.6), (19.4, 32.6), (14.6, 22.6), (24.2, 22.6)]
# (The light running along the arrow's brass, cursor.rs ARROW_GLINT, follows its rim a unit in from
# these edges; its lamp, ARROW_LAMP, is the green circle below.)


def arrow_layers():
    outer = poly(*ARROW)
    return [
        ([outer], "brass", "raised", 0.9, 0.0),
        ([outer], "gunmetal", "raised", 1.1, 2.0),
        ([circle(8.3, 19.6, 1.85)], "dark", "sunk", 0.6, 0.0),
        ([circle(8.3, 19.6, 1.05)], "green", "raised", 0.5, 0.0),
    ]


def finger(x0, x1, y0, y1, joints, tip_brass=True):
    """A finger of plates: gunmetal with dark joints, its tip brass."""
    r = (x1 - x0) / 2
    layers = [([rect(x0, y0, x1, y1, r)], "gunmetal", "raised", 0.9, 0.0)]
    if tip_brass:
        cut = joints[0] if joints else y1
        layers.append(([rect(x0, y0, x1, cut + r, r)], "brass", "raised", 0.8, 0.45))
    for y in joints:
        layers.append(([rect(x0 + 0.5, y - 0.45, x1 - 0.5, y + 0.45, 0.4)], "dark", "sunk", 0.3, 0.0))
    return layers


def gauntlet_back(x0, y0, x1, y1, cuff_y0, cuff_y1):
    """Palm plate with the emblem's hexagon, and the brass cuff with its rivets."""
    cx, cy = (x0 + x1) / 2, (y0 + min(y1, cuff_y0)) / 2 + 0.4
    hexagon = poly(*[(cx + 3.1 * math.cos(math.radians(60 * i - 90)), cy + 3.1 * math.sin(math.radians(60 * i - 90))) for i in range(6)])
    chevron = poly((cx - 1.7, cy + 0.9), (cx, cy - 0.8), (cx + 1.7, cy + 0.9), (cx + 1.7, cy + 1.9), (cx, cy + 0.2), (cx - 1.7, cy + 1.9))
    return [
        ([rect(x0, y0, x1, y1, 4.0)], "gunmetal", "raised", 1.2, 0.0),
        ([hexagon], "dark", "sunk", 0.5, 0.0),
        ([hexagon], "brass", "raised", 0.5, 0.75),
        ([chevron], "dark", "sunk", 0.3, 0.0),
        ([rect(x0 + 1.2, cuff_y0, x1 - 1.2, cuff_y1, 1.6)], "brass", "raised", 0.8, 0.0),
        ([rect(x0 + 1.9, cuff_y0 + (cuff_y1 - cuff_y0) / 2 - 0.35, x1 - 1.9, cuff_y0 + (cuff_y1 - cuff_y0) / 2 + 0.35, 0.3)], "brass_dark", "sunk", 0.25, 0.0),
        ([circle(x0 + 3.2, cuff_y0 + 1.5, 0.75), circle(x1 - 3.2, cuff_y0 + 1.5, 0.75)], "steel", "raised", 0.4, 0.0),
    ]


def hand_point_layers():
    layers = []
    layers += [([capsule(9.6, 25.5, 4.6, 18.8, 4.6)], "gunmetal", "raised", 0.9, 0.0)]
    layers += [([circle(4.6, 18.8, 2.3)], "brass", "raised", 0.7, 0.4)]
    for x0, x1, y0 in [(16.0, 20.4, 14.2), (20.6, 24.9, 15.4), (25.1, 29.0, 17.2)]:
        layers += [([rect(x0, y0, x1, 24.0, 2.0)], "gunmetal", "raised", 0.8, 0.0),
                   ([rect(x0, y0, x1, y0 + 3.6, 2.0)], "brass", "raised", 0.6, 0.45)]
    layers += gauntlet_back(8.8, 20.6, 30.2, 34.0, 32.4, 38.6)
    layers += finger(10.4, 15.6, 2.4, 23.0, [9.0, 15.2])
    layers += [([circle(13.0, 5.2, 0.85)], "green", "raised", 0.4, 0.0)]
    return layers


def hand_open_layers():
    layers = []
    layers += [([capsule(9.2, 26.5, 3.4, 21.0, 4.4)], "gunmetal", "raised", 0.9, 0.0),
               ([circle(3.4, 21.0, 2.2)], "brass", "raised", 0.7, 0.4)]
    layers += gauntlet_back(7.6, 18.0, 29.0, 31.4, 29.8, 36.0)
    for x0, x1, y0, joints in [(8.2, 12.6, 6.2, [11.6, 16.2]), (13.0, 17.4, 3.0, [8.8, 13.8]),
                               (17.8, 22.2, 4.0, [9.6, 14.6]), (22.6, 26.6, 7.4, [12.2, 16.6])]:
        layers += finger(x0, x1, y0, 22.0, joints)
    return layers


def hand_closed_layers():
    layers = []
    layers += gauntlet_back(7.6, 15.0, 29.0, 29.0, 27.6, 33.8)
    for x0, x1 in [(8.4, 13.0), (13.2, 17.8), (18.0, 22.6), (22.8, 27.4)]:
        layers += [([rect(x0, 11.2, x1, 19.4, 2.2)], "gunmetal", "raised", 0.8, 0.0),
                   ([rect(x0, 11.2, x1, 14.6, 2.2)], "brass", "raised", 0.6, 0.45)]
    layers += [([capsule(6.4, 22.0, 15.6, 21.6, 4.4)], "gunmetal", "raised", 0.9, 0.0),
               ([circle(15.6, 21.6, 2.2)], "brass", "raised", 0.7, 0.4)]
    return layers


def ibeam_layers():
    serif = lambda y: path(f"M3.0,{y - 1.3:.2f} Q8.0,{y + 0.6:.2f} 13.0,{y - 1.3:.2f} L13.0,{y + 1.3:.2f} Q8.0,{y + 3.2:.2f} 3.0,{y + 1.3:.2f} Z")
    return [
        ([rect(6.9, 4.0, 9.1, 26.0, 0.6)], "gunmetal", "raised", 0.6, 0.0),
        ([serif(3.4)], "brass", "raised", 0.7, 0.0),
        ([rotated(serif(3.4), 180, 8.0, 15.0)], "brass", "raised", 0.7, 0.0),
    ]


def gear_layers(cx=13.0, cy=13.0, s=1.0):
    return [
        ([gear(cx, cy, 11.6 * s, 9.0 * s, 10, 5.4 * s)], "gunmetal", "raised", 0.9 * s, 0.0),
        ([ring(cx, cy, 5.4 * s, 7.4 * s)], "dark", "sunk", 0.5 * s, 0.0),
        ([ring(cx, cy, 2.4 * s, 5.4 * s)], "brass", "raised", 0.7 * s, 0.0),
        ([circle(cx, cy, 2.4 * s)], "dark", "sunk", 0.5 * s, 0.0),
    ]


def resize_layers():
    head = lambda tip, base: poly((tip, 8.0), (base, 2.6), (base, 13.4))
    return [
        ([rect(8.0, 6.6, 26.0, 9.4, 0.8)], "gunmetal", "raised", 0.7, 0.0),
        ([head(1.6, 9.6)], "brass", "raised", 0.8, 0.0),
        ([head(32.4, 24.4)], "brass", "raised", 0.8, 0.0),
    ]


def move_layers():
    c = 17.0
    heads = []
    for angle in (0, 90, 180, 270):
        heads.append(rotated(poly((c + 15.4, c), (c + 9.4, c - 5.0), (c + 9.4, c + 5.0)), angle, c, c))
    return [
        ([rect(c - 10.0, c - 1.4, c + 10.0, c + 1.4, 0.6), rect(c - 1.4, c - 10.0, c + 1.4, c + 10.0, 0.6)], "gunmetal", "raised", 0.7, 0.0),
        (heads, "brass", "raised", 0.8, 0.0),
        ([circle(c, c, 3.4)], "brass", "raised", 0.8, 0.0),
        ([circle(c, c, 1.5)], "dark", "sunk", 0.4, 0.0),
    ]


def forbidden_layers():
    c = 14.0
    return [
        ([ring(c, c, 8.0, 11.8)], "rust", "raised", 1.0, 0.0),
        ([rotated(rect(c - 9.0, c - 1.8, c + 9.0, c + 1.8, 0.8), -45, c, c)], "rust", "raised", 0.9, 0.0),
        ([ring(c, c, 11.0, 11.8)], "brass", "raised", 0.4, 0.0),
    ]


def crosshair_layers():
    c = 15.0
    ticks = [rotated(rect(c + 5.2, c - 0.85, c + 13.6, c + 0.85, 0.5), a, c, c) for a in (0, 90, 180, 270)]
    return [
        ([ring(c, c, 7.9, 9.7)], "brass", "raised", 0.6, 0.0),
        (ticks, "gunmetal", "raised", 0.5, 0.0),
        ([circle(c, c, 1.3)], "green", "raised", 0.4, 0.0),
    ]


def zoom_layers(plus):
    c = 12.0
    glyph = [rect(c - 4.2, c - 1.0, c + 4.2, c + 1.0, 0.5)]
    if plus:
        glyph.append(rect(c - 1.0, c - 4.2, c + 1.0, c + 4.2, 0.5))
    return [
        ([capsule(c + 7.6, c + 7.6, c + 15.4, c + 15.4, 4.6)], "brass", "raised", 0.9, 0.0),
        ([circle(c, c, 7.4)], "glass", "sunk", 1.2, 0.0),
        ([ring(c, c, 7.4, 10.2)], "gunmetal", "raised", 0.9, 0.0),
        (glyph, "brass", "raised", 0.5, 0.0),
    ]


def badge_layers(kind):
    layers = [([rect(1.0, 1.0, 15.0, 15.0, 2.6)], "brass", "raised", 0.7, 0.0),
              ([rect(1.0, 1.0, 15.0, 15.0, 2.6)], "gunmetal", "raised", 0.8, 1.3)]
    if kind == "copy":
        layers += [([rect(4.0, 4.0, 10.0, 10.0, 1.0)], "dark", "sunk", 0.4, 0.0),
                   ([rect(6.4, 6.4, 12.4, 12.4, 1.0)], "brass", "raised", 0.5, 0.0)]
    elif kind == "alias":
        layers += [([path("M4.4,12.0 L4.4,8.4 Q4.4,6.0 7.0,6.0 L10.0,6.0", 1.7)], "brass", "raised", 0.5, 0.0),
                   ([poly((9.2, 3.4), (12.8, 6.0), (9.2, 8.6))], "brass", "raised", 0.5, 0.0)]
    elif kind == "menu":
        layers += [([rect(4.2, y - 0.8, 11.8, y + 0.8, 0.5) for y in (4.8, 8.0, 11.2)], "brass", "raised", 0.4, 0.0)]
    elif kind == "help":
        layers += [([path("M5.6,6.0 Q5.6,3.8 8.0,3.8 Q10.4,3.8 10.4,6.0 Q10.4,7.6 8.0,8.4 L8.0,9.6", 1.8)], "brass", "raised", 0.5, 0.0),
                   ([circle(8.0, 12.0, 1.1)], "brass", "raised", 0.4, 0.0)]
    return layers


# name: (canvas w, h in units, hotspot x, y in units, layers)
SPRITES = {
    "arrow": (28, 38, 3.0, 3.0, arrow_layers),
    "hand_point": (33, 42, 13.0, 2.4, hand_point_layers),
    "hand_open": (32, 39, 17.6, 20.0, hand_open_layers),
    "hand_closed": (32, 37, 17.6, 20.0, hand_closed_layers),
    "ibeam": (16, 30, 8.0, 15.0, ibeam_layers),
    "gear": (26, 26, 13.0, 13.0, gear_layers),
    "resize": (34, 16, 17.0, 8.0, resize_layers),
    "move": (34, 34, 17.0, 17.0, move_layers),
    "forbidden": (28, 28, 14.0, 14.0, forbidden_layers),
    "crosshair": (30, 30, 15.0, 15.0, crosshair_layers),
    "zoom_in": (30, 30, 12.0, 12.0, lambda: zoom_layers(True)),
    "zoom_out": (30, 30, 12.0, 12.0, lambda: zoom_layers(False)),
    "badge_copy": (16, 16, 8.0, 8.0, lambda: badge_layers("copy")),
    "badge_alias": (16, 16, 8.0, 8.0, lambda: badge_layers("alias")),
    "badge_menu": (16, 16, 8.0, 8.0, lambda: badge_layers("menu")),
    "badge_help": (16, 16, 8.0, 8.0, lambda: badge_layers("help")),
}


# ---------------------------------------------------------------------------------------------
# Rasterising, lighting, compositing
# ---------------------------------------------------------------------------------------------


class Canvas:
    def __init__(self, w, h, resvg, tmp):
        self.w, self.h, self.resvg, self.tmp = w, h, resvg, tmp
        self.scale = PX * SS
        self.n = 0

    def mask(self, shapes, inset=0.0, grow=0.0):
        """The coverage of `shapes`: shrunk by `inset` units, or grown by `grow` units."""
        body = "".join(shapes)
        if inset > 0:
            group = (f'<g fill="white" stroke="white" stroke-width="0">{body}</g>'
                     f'<g fill="none" stroke="black" stroke-width="{2 * inset:.3f}" stroke-linejoin="round">{body}</g>')
        elif grow > 0:
            group = f'<g fill="white" stroke="white" stroke-width="{2 * grow:.3f}" stroke-linejoin="round" stroke-linecap="round">{body}</g>'
        else:
            group = f'<g fill="white" stroke="white" stroke-width="0">{body}</g>'
        # (Strokes the shapes set themselves (capsules, paths) keep their own widths.)
        svg = (f'<svg xmlns="http://www.w3.org/2000/svg" width="{self.w * self.scale}" height="{self.h * self.scale}" '
               f'viewBox="0 0 {self.w} {self.h}"><rect width="{self.w}" height="{self.h}" fill="black"/>{group}</svg>')
        self.n += 1
        src = os.path.join(self.tmp, f"m{self.n}.svg")
        out = os.path.join(self.tmp, f"m{self.n}.png")
        with open(src, "w", encoding="utf-8") as f:
            f.write(svg)
        subprocess.run([self.resvg, src, out], check=True, capture_output=True)
        rgb = np.asarray(Image.open(out).convert("RGB"), dtype=np.float32) / 255.0
        return rgb[..., 0]


def blur(a, r):
    """A box blur of radius r pixels, three times over (near enough a gaussian)."""
    r = int(round(r))
    if r < 1:
        return a
    k = 2 * r + 1
    for _ in range(3):
        for axis in (0, 1):
            pad = [(r + 1, r), (0, 0)] if axis == 0 else [(0, 0), (r + 1, r)]
            c = np.cumsum(np.pad(a, pad, mode="edge"), axis=axis)
            n = a.shape[axis]
            a = (np.take(c, range(k, k + n), axis=axis) - np.take(c, range(0, n), axis=axis)) / k
    return a


def shade(mask, metal, relief, bevel_px):
    """A part's colour: its metal, lit through the relief of its coverage."""
    albedo, spec_rgb, shininess, ks = METALS[metal]
    h = blur(mask, max(1, bevel_px)) * mask
    if relief == "sunk":
        h = -blur(mask, max(1, bevel_px))
    gy, gx = np.gradient(h)
    k = 2.2 * max(1, bevel_px)
    n = np.dstack([-gx * k, -gy * k, np.ones_like(h)])
    n /= np.linalg.norm(n, axis=2, keepdims=True)
    diffuse = np.clip(n @ LIGHT, 0, 1)
    spec = np.clip(n @ HALF, 0, 1) ** shininess
    # A polish from top to bottom across the part: brighter where the sky reflects.
    rows = np.where(mask.max(axis=1) > 0.5)[0]
    top, bottom = (rows.min(), rows.max()) if len(rows) else (0, mask.shape[0])
    y = np.clip((np.arange(mask.shape[0]) - top) / max(1, bottom - top), 0, 1)[:, None]
    polish = 1.12 - 0.26 * y
    rgb = np.array(albedo)[None, None, :] * (0.34 + 0.78 * diffuse[..., None]) * polish[..., None]
    rgb += np.array(spec_rgb)[None, None, :] * (ks * spec)[..., None]
    if metal == "green":
        rgb += np.array([0.10, 0.30, 0.06])[None, None, :]  # (it is a lamp: it glows)
    return np.clip(rgb, 0, 1)


def render(name, resvg, tmp):
    w, h, _, _, make = SPRITES[name]
    canvas = Canvas(w, h, resvg, tmp)
    layers = make()
    big = (h * PX * SS, w * PX * SS)
    out = np.zeros(big + (4,), dtype=np.float32)  # premultiplied
    every = [s for shapes, _, _, _, _ in layers for s in shapes]
    outline = canvas.mask(every, grow=OUTLINE)
    # The shadow: the outline's coverage, moved and blurred.
    sx, sy, sb, so = SHADOW
    dx, dy = int(round(sx * PX * SS)), int(round(sy * PX * SS))
    shadow = np.zeros_like(outline)
    shadow[dy:, dx:] = outline[: outline.shape[0] - dy, : outline.shape[1] - dx]
    shadow = blur(shadow, sb * PX * SS / 2) * so
    out[..., 3] = shadow
    edge = np.array(EDGE)
    out = over(out, edge, outline)
    for shapes, metal, relief, bevel, inset in layers:
        m = canvas.mask(shapes, inset=inset)
        rgb = shade(m, metal, relief, bevel * PX * SS)
        out = over(out, rgb, m)
    # Down to size, premultiplied, so that the edges keep their light.
    data = (np.clip(out, 0, 1) * 255 + 0.5).astype(np.uint8)
    img = Image.frombytes("RGBa", (data.shape[1], data.shape[0]), data.tobytes())
    img = img.resize((w * PX, h * PX), Image.LANCZOS).convert("RGBA")
    return img


def over(dst, rgb, alpha):
    a = alpha[..., None]
    rgb = np.broadcast_to(rgb, dst.shape[:2] + (3,))
    res = np.empty_like(dst)
    res[..., :3] = rgb * a + dst[..., :3] * (1 - a)
    res[..., 3] = alpha + dst[..., 3] * (1 - alpha)
    return res


# ---------------------------------------------------------------------------------------------
# Preview
# ---------------------------------------------------------------------------------------------


def preview(images, path):
    """Every pointer at 1x, 1.5x and 3x pointer size on the launcher's dark plate and on white."""
    scales = [1.0, 1.5, 3.0]
    cell = 46 * 3
    sheet = Image.new("RGBA", (cell * len(images), cell * len(scales) * 2), (0, 0, 0, 255))
    for row, (bg, colour) in enumerate([("dark", (24, 23, 20, 255)), ("light", (226, 226, 222, 255))]):
        for j, s in enumerate(scales):
            y0 = (row * len(scales) + j) * cell
            sheet.paste(Image.new("RGBA", (cell * len(images), cell), colour), (0, y0))
            for i, (name, img) in enumerate(images.items()):
                w, h = img.size
                size = (max(1, round(w / PX * s)), max(1, round(h / PX * s)))
                small = img.resize(size, Image.LANCZOS)
                x = i * cell + (cell - size[0]) // 2
                y = y0 + (cell - size[1]) // 2
                sheet.alpha_composite(small, (x, y))
    sheet.convert("RGB").save(path)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", default=os.path.join(os.path.dirname(__file__), "..", "..", "apps", "ta_launcher", "cursors"))
    parser.add_argument("--preview")
    parser.add_argument("--only")
    args = parser.parse_args()
    resvg = shutil.which("resvg")
    if not resvg:
        raise SystemExit("resvg is not on the PATH")
    names = args.only.split(",") if args.only else list(SPRITES)
    os.makedirs(args.out, exist_ok=True)
    images = {}
    with tempfile.TemporaryDirectory() as tmp:
        for name in names:
            img = render(name, resvg, tmp)
            img.save(os.path.join(args.out, f"{name}.png"), optimize=True)
            images[name] = img
            print(f"{name}: {img.size[0]}x{img.size[1]}")
    if args.preview:
        os.makedirs(args.preview, exist_ok=True)
        preview(images, os.path.join(args.preview, "cursors.png"))
        print("preview:", os.path.join(args.preview, "cursors.png"))


if __name__ == "__main__":
    main()
