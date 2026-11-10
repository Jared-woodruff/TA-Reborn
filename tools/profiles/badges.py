"""Draws the achievement badges (original art for TA Reborn): a medallion for each achievement of
`crates/ta_config/src/achievements.rs`, read from there in its order. Its rim and glyph are struck in
the metal of its tier (bronze, silver, gold, platinum; silver's rim beaded, gold's in a laurel,
platinum's with a crest star and gems as well) over an enamel field in its category's colour; each
part lit from the top left through its bevel, outlined in near-black and given a soft shadow. The
glyph is the achievement's `badge` (GLYPHS below, each drawn on a 64-unit grid). A last cell holds
the badge of a secret achievement not yet earned.

The atlas (`crates/ta_ui/badges/achievements.png`) is COLS cells a row, CELL pixels each, the badges
drawn SS times over and brought down. The interface greys a badge not earned itself.

    python tools/profiles/badges.py [--out crates/ta_ui/badges] [--preview <dir>] [--only id,id]
    python tools/profiles/badges.py --ranks [--preview <dir>]    (the rank insignia, ranks.png)

Needs numpy, Pillow and resvg (https://github.com/linebender/resvg) on the PATH; then `oxipng -o 3`
the atlas.
"""
import argparse
import math
import os
import re
import shutil
import subprocess
import tempfile

import numpy as np
from PIL import Image, ImageDraw

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
CELL = 128
COLS = 16
SS = 4
PX = CELL * SS  # the canvas, in pixels; SVG units are the cell's pixels

# Metals: albedo, specular colour, shininess, specular strength.
METALS = {
    "Bronze": ((0.74, 0.45, 0.24), (1.0, 0.82, 0.62), 24.0, 0.55),
    "Silver": ((0.76, 0.78, 0.81), (1.0, 1.0, 1.0), 38.0, 0.70),
    "Gold": ((0.95, 0.73, 0.27), (1.0, 0.95, 0.72), 30.0, 0.75),
    "Platinum": ((0.86, 0.90, 0.94), (0.92, 1.0, 1.0), 46.0, 0.85),
    "steel": ((0.48, 0.50, 0.52), (0.90, 0.93, 0.96), 34.0, 0.55),
    "gem": ((0.10, 0.52, 0.74), (0.85, 1.0, 1.0), 70.0, 0.95),
    "brass": ((0.88, 0.66, 0.30), (1.0, 0.92, 0.70), 28.0, 0.70),
}
# Enamel of each category's field.
ENAMEL = {
    "Campaign": (0.11, 0.22, 0.40),
    "Skirmish": (0.40, 0.10, 0.09),
    "Multiplayer": (0.06, 0.30, 0.30),
    "Battle": (0.21, 0.26, 0.12),
    "Industry": (0.34, 0.22, 0.06),
    "Service": (0.25, 0.14, 0.37),
    "hidden": (0.09, 0.09, 0.10),
}
EDGE = (0.035, 0.035, 0.04)
LIGHT = np.array([-0.50, -0.72, 0.60])
LIGHT /= np.linalg.norm(LIGHT)
HALF = LIGHT + np.array([0.0, 0.0, 1.0])
HALF /= np.linalg.norm(HALF)

CX, CY = 64.0, 65.0  # the medallion's centre
R_RIM, R_FIELD = 49.0, 40.0
GLYPH = 0.88  # the glyph's 64 units, in cell units / 64

# ---------------------------------------------------------------------------------------------
# The catalogue
# ---------------------------------------------------------------------------------------------

LINE = re.compile(r'^\s*(a|hidden)\("([^"]+)", "([^"]+)", "[^"]*", C::(\w+), T::(\w+), "([^"]+)", ')


def catalogue():
    path = os.path.join(ROOT, "crates", "ta_config", "src", "achievements.rs")
    out = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            m = LINE.match(line)
            if m:
                kind, ident, name, category, tier, badge = m.groups()
                out.append({"id": ident, "name": name, "category": category, "tier": tier, "badge": badge, "hidden": kind == "hidden"})
    if len(out) < 100:
        raise SystemExit(f"read {len(out)} achievements from {path}: has the catalogue's form changed?")
    return out


# ---------------------------------------------------------------------------------------------
# Glyphs: SVG on a 64-unit grid, white (black cuts holes in a `masked` body)
# ---------------------------------------------------------------------------------------------

W = 5.0


def line(x1, y1, x2, y2, w=W):
    return f'<line x1="{x1:.2f}" y1="{y1:.2f}" x2="{x2:.2f}" y2="{y2:.2f}" stroke="white" stroke-width="{w}" stroke-linecap="round"/>'


def poly(points):
    pts = " ".join(f"{x:.2f},{y:.2f}" for x, y in points)
    return f'<polygon points="{pts}" fill="white" stroke="white" stroke-width="1" stroke-linejoin="round"/>'


def circle(cx, cy, r):
    return f'<circle cx="{cx:.2f}" cy="{cy:.2f}" r="{r:.2f}" fill="white"/>'


def ring(cx, cy, r, w=W):
    return f'<circle cx="{cx:.2f}" cy="{cy:.2f}" r="{r:.2f}" fill="none" stroke="white" stroke-width="{w}"/>'


def ellipse(cx, cy, rx, ry, rot=0.0, w=None):
    paint = f'fill="none" stroke="white" stroke-width="{w}"' if w else 'fill="white"'
    return f'<ellipse cx="{cx:.2f}" cy="{cy:.2f}" rx="{rx:.2f}" ry="{ry:.2f}" transform="rotate({rot:.2f} {cx:.2f} {cy:.2f})" {paint}/>'


def rect(x, y, w, h, r=0.0, rot=0.0):
    t = f' transform="rotate({rot:.2f} {x + w / 2:.2f} {y + h / 2:.2f})"' if rot else ""
    return f'<rect x="{x:.2f}" y="{y:.2f}" width="{w:.2f}" height="{h:.2f}" rx="{r:.2f}" fill="white"{t}/>'


def path(d, w=None):
    if w:
        return f'<path d="{d}" fill="none" stroke="white" stroke-width="{w}" stroke-linecap="round" stroke-linejoin="round"/>'
    return f'<path d="{d}" fill="white"/>'


def group(fragment, tx=0.0, ty=0.0, s=1.0, mirror=False):
    m = f" translate(64 0) scale(-1 1)" if mirror else ""
    return f'<g transform="translate({tx:.2f} {ty:.2f}) scale({s:.4f}){m}">{fragment}</g>'


def black(fragment):
    return fragment.replace('"white"', '"black"')


_MASKS = [0]


def masked(body, holes):
    _MASKS[0] += 1
    mid = f"m{_MASKS[0]}"
    return (f'<mask id="{mid}" maskUnits="userSpaceOnUse" x="-64" y="-64" width="192" height="192">'
            f'<rect x="-64" y="-64" width="192" height="192" fill="white"/>{holes}</mask><g mask="url(#{mid})">{body}</g>')


def head(tx, ty, dx, dy, length=12.0, half=8.0):
    n = math.hypot(dx, dy)
    dx, dy = dx / n, dy / n
    bx, by = tx - dx * length, ty - dy * length
    return poly([(tx, ty), (bx - dy * half, by + dx * half), (bx + dy * half, by - dx * half)])


def star_points(cx, cy, outer, inner, points, rot=-90.0):
    return [(cx + (outer if k % 2 == 0 else inner) * math.cos(math.radians(rot + 180 * k / points)),
             cy + (outer if k % 2 == 0 else inner) * math.sin(math.radians(rot + 180 * k / points))) for k in range(2 * points)]


def arc_d(cx, cy, r, a0, a1):
    """An arc from angle a0 to a1 (degrees, clockwise from the right, y down)."""
    x0, y0 = cx + r * math.cos(math.radians(a0)), cy + r * math.sin(math.radians(a0))
    x1, y1 = cx + r * math.cos(math.radians(a1)), cy + r * math.sin(math.radians(a1))
    large = 1 if abs(a1 - a0) > 180 else 0
    sweep = 1 if a1 > a0 else 0
    return f"M{x0:.2f},{y0:.2f} A{r},{r} 0 {large},{sweep} {x1:.2f},{y1:.2f}"


def helmet():
    dome = path("M11,41 C11,21 20,9 32,9 C44,9 53,21 53,41 L53,46 L11,46 Z")
    body = masked(dome, black(path("M17,29 L47,29 L45,36 L19,36 Z")))
    return body + poly([(9, 47), (55, 47), (51, 55), (13, 55)]) + rect(30, 3, 4, 9, 1.5)


def bust(cx, top, k=1.0, grow=0.0):
    """A person from the shoulders up: a head and the shoulders below it."""
    head_ = f'<circle cx="{cx:.2f}" cy="{top + 8.5 * k:.2f}" r="{8.5 * k + grow:.2f}" fill="white"/>'
    y0, y1 = top + 20 * k - grow, top + 40 * k + grow
    w = 16 * k + grow
    shoulders = f'<path d="M{cx - w:.2f},{y1:.2f} C{cx - w:.2f},{y0 + 2:.2f} {cx - w * 0.6:.2f},{y0:.2f} {cx:.2f},{y0:.2f} C{cx + w * 0.6:.2f},{y0:.2f} {cx + w:.2f},{y0 + 2:.2f} {cx + w:.2f},{y1:.2f} Z" fill="white"/>'
    return head_ + shoulders


def chevron(y, h=9.0, t=8.0, x0=9.0, x1=55.0):
    m = (x0 + x1) / 2
    return poly([(x0, y + h), (m, y), (x1, y + h), (x1, y + h + t), (m, y + t), (x0, y + h + t)])


def sword(f):
    x = lambda v: 32 + f * (v - 32)
    return (poly([(x(5), 5), (x(17), 9), (x(43), 35), (x(35), 43), (x(9), 17)])
            + line(x(31), 51, x(51), 31, 7.5) + line(x(43), 43, x(52), 52, 7) + circle(x(55), 55, 5))


def glyph(name):
    if name == "flag":
        cloth = path("M21,10 C29,6 35,14 45,10 C48,9 50,8 52,8 L52,31 C45,33 39,27 31,31 C27,33 24,34 21,33 Z")
        return line(18, 9, 18, 56, 5) + cloth + rect(10, 53, 16, 6, 2) + circle(18, 7, 3.4)
    if name == "scroll":
        body = masked(rect(15, 13, 34, 38, 2), black(line(22, 24, 42, 24, 3) + line(22, 31, 42, 31, 3) + line(22, 38, 36, 38, 3)))
        return body + rect(9, 7, 46, 10, 5) + rect(9, 47, 46, 10, 5)
    if name == "arm":
        return poly([(32, 5), (57, 41), (45, 41), (32, 23), (19, 41), (7, 41)]) + poly([(13, 47), (51, 47), (46, 57), (18, 57)])
    if name == "core":
        hexagon = [(32 + 26 * math.cos(math.radians(-90 + 60 * k)), 32 + 26 * math.sin(math.radians(-90 + 60 * k))) for k in range(6)]
        return masked(poly(hexagon), black(circle(32, 32, 8.5) + ring(32, 32, 17, 3.2)))
    if name == "giant":
        s = rect(26, 5, 12, 10, 2.5) + poly([(15, 17), (49, 17), (45, 38), (19, 38)])
        s += rect(7, 18, 7, 19, 3) + rect(50, 18, 7, 19, 3) + rect(21, 38, 22, 5, 1.5)
        s += poly([(21, 43), (29, 43), (28, 54), (20, 54)]) + poly([(35, 43), (43, 43), (44, 54), (36, 54)])
        s += rect(14, 53, 16, 6, 2) + rect(34, 53, 16, 6, 2)
        return masked(s, black(rect(28, 8.5, 8, 2.8, 1.2)))
    if name == "knight":
        horse = path("M18,52 L46,52 L46,47 L42,45 C40,38 45,33 46,25 C47,15 40,8 30,8 L28,3 L23,10 C17,13 13,20 12,26 "
                     "C11,30 13,33 16,33 C19,33 22,31 25,29 C27,35 21,39 20,46 L18,48 Z")
        return masked(horse, black(circle(29, 17, 2.4))) + rect(13, 52, 38, 7, 2.5)
    if name == "flame":
        outer = path("M32,3 C36,13 47,20 49,34 C51,48 43,59 32,59 C21,59 13,50 14,38 C15,30 19,25 23,19 "
                     "C24,25 26,28 29,30 C30,21 33,13 32,3 Z")
        inner = path("M32,31 C35,37 40,41 40,47 C40,53 36,56 32,56 C28,56 24,53 24,48 C24,43 29,39 32,31 Z")
        return masked(outer, black(inner)) + path("M32,41 C34,45 37,47 37,50 C37,53 35,54 32,54 C29,54 27,52 27,50 C27,47 30,45 32,41 Z")
    if name == "sides":
        # The two armies' marks, either side of a line.
        return group(glyph("arm"), -1, 15, 0.52) + line(32, 10, 32, 54, 3.2) + group(glyph("core"), 32, 15, 0.52)
    if name == "crosshair":
        return (ring(32, 32, 18, 5) + line(32, 3, 32, 15, 5) + line(32, 49, 32, 61, 5) + line(3, 32, 15, 32, 5)
                + line(49, 32, 61, 32, 5) + circle(32, 32, 5))
    if name == "magnet":
        u = path("M18,8 L18,33 A14,14 0 0,0 46,33 L46,8", 11)
        return masked(u, black(rect(8, 15, 48, 3.2))) + line(8, 50, 3, 55, 3.5) + line(56, 50, 61, 55, 3.5) + line(32, 56, 32, 62, 3.5)
    if name == "swords":
        front_blade = black(line(8, 8, 41, 41, 17))
        return masked(sword(-1), front_blade) + sword(1)
    if name == "trophy":
        cup = path("M17,7 L47,7 L47,21 C47,32 40,38 32,38 C24,38 17,32 17,21 Z")
        handles = path("M18,12 L10,12 C10,24 14,28 20,29", 4.5) + path("M46,12 L54,12 C54,24 50,28 44,29", 4.5)
        return masked(cup, black(star_star(32, 20, 7, 3))) + handles + rect(29, 37, 6, 9) + poly([(21, 51), (43, 51), (40, 45), (24, 45)]) + rect(16, 51, 32, 7, 2)
    if name == "odds":
        big = poly([(5, 13), (25, 32), (5, 51), (14, 51), (34, 32), (14, 13)])
        small = "".join(poly([(57, y - 7), (46, y), (57, y + 7), (61, y + 4), (54, y), (61, y - 4)]) for y in (13, 32, 51))
        return big + small
    if name == "crown":
        body = poly([(8, 45), (10, 17), (22, 31), (32, 11), (42, 31), (54, 17), (56, 45)])
        band = masked(rect(8, 46, 48, 9, 2), black(circle(20, 50.5, 2.6) + circle(32, 50.5, 2.6) + circle(44, 50.5, 2.6)))
        return body + band + circle(10, 14, 3.8) + circle(32, 8, 3.8) + circle(54, 14, 3.8)
    if name == "stopwatch":
        return (ring(32, 37, 20, 5) + rect(27, 5, 10, 6, 2) + line(32, 11, 32, 16, 4) + line(48, 18, 52, 14, 4)
                + line(32, 37, 32, 24, 4) + line(32, 37, 41, 42, 3.5) + circle(32, 37, 3.2))
    if name == "globe":
        s = ring(32, 32, 25, 4.5) + ellipse(32, 32, 11, 25, 0, 3.6) + line(32, 7, 32, 57, 3.6) + line(7, 32, 57, 32, 3.6)
        return s + line(11, 21, 53, 21, 3.4) + line(11, 43, 53, 43, 3.4)
    if name == "planet":
        # A ringed world: the ring behind it where it passes behind, in front below.
        tilt = -20.0
        ring_ = ellipse(32, 33, 29, 8.5, tilt, 4.2)
        dx, dy = math.cos(math.radians(tilt)), math.sin(math.radians(tilt))
        above = black(poly([(32 - 60 * dx, 33 - 60 * dy), (32 + 60 * dx, 33 + 60 * dy), (32 + 60 * dx, -40), (32 - 60 * dx, -40)]))
        sphere = circle(32, 33, 16)
        front = masked(ring_, above)
        behind = masked(ring_, black(sphere))
        cut = black(ellipse(32, 33, 29, 8.5, tilt, 10))
        return masked(masked(sphere, cut), above.replace('"black"', '"white"')) + masked(sphere, black(front.replace('stroke-width="4.2"', 'stroke-width="10"')) ) + behind + front
    if name == "hourglass":
        glass = path("M19,12 L45,12 L34,32 L45,52 L19,52 L30,32 Z", 4)
        sand = poly([(25, 47), (39, 47), (43, 51), (21, 51)]) + poly([(23, 15), (41, 15), (32, 27)])
        return rect(11, 5, 42, 7, 2.5) + rect(11, 52, 42, 7, 2.5) + glass + sand
    if name == "link":
        def link(cx, cy, w):
            return (f'<rect x="{cx - 15:.2f}" y="{cy - 8:.2f}" width="30" height="16" rx="8" fill="none" stroke="white" '
                    f'stroke-width="{w}" transform="rotate(-45 {cx:.2f} {cy:.2f})"/>')
        # The second link in front, the first broken where it passes behind it.
        return masked(link(23, 41, 5.5), black(link(41, 23, 11.5))) + link(41, 23, 5.5)
    if name == "target":
        ticks = line(32, 3, 32, 11, 4.5) + line(32, 53, 32, 61, 4.5) + line(3, 32, 11, 32, 4.5) + line(53, 32, 61, 32, 4.5)
        return ring(32, 32, 24, 4.2) + ticks + group(helmet(), 32 - 32 * 0.56, 33 - 32 * 0.56, 0.56)
    if name == "helmet":
        return helmet()
    if name == "team":
        back = bust(41, 6, 0.95)
        front = bust(23, 14, 1.05)
        return masked(back, black(bust(23, 14, 1.05, 3.2))) + front
    if name == "skull":
        cranium = path("M32,5 C46,5 55,15 55,28 C55,36 51,41 46,43 L46,50 L18,50 L18,43 C13,41 9,36 9,28 C9,15 18,5 32,5 Z")
        holes = circle(22.5, 28, 6.5) + circle(41.5, 28, 6.5) + poly([(32, 35), (28.5, 42), (35.5, 42)])
        teeth = masked(rect(20, 50, 24, 8, 2), black(line(26, 51, 26, 58, 2.2) + line(32, 51, 32, 58, 2.2) + line(38, 51, 38, 58, 2.2)))
        return masked(cranium, black(holes)) + teeth
    if name == "sword":
        blade = masked(poly([(32, 3), (37.5, 11), (37.5, 40), (26.5, 40), (26.5, 11)]), black(line(32, 12, 32, 35, 2.2)))
        return blade + rect(16, 40, 32, 6, 2.5) + rect(29, 46, 6, 9) + circle(32, 58, 4.2)
    if name == "duel":
        # Two Commanders, and the spark between them.
        return group(helmet(), -1, 20, 0.56) + group(helmet(), 33, 20, 0.56, mirror=True) + poly(star_points(32, 12, 10, 3.6, 4, -90))
    if name == "plane":
        s = path("M32,3 C35,8 36,14 36,20 L36,44 L32,52 L28,44 L28,20 C28,14 29,8 32,3 Z")
        s += poly([(28, 23), (5, 38), (5, 43), (28, 36)]) + poly([(36, 23), (59, 38), (59, 43), (36, 36)])
        s += poly([(29, 44), (19, 54), (19, 58), (31, 52)]) + poly([(35, 44), (45, 54), (45, 58), (33, 52)])
        return s
    if name == "anchor":
        arms = path("M10,37 C12,50 21,56 32,56 C43,56 52,50 54,37", 5)
        return (ring(32, 9, 5, 4) + line(32, 14, 32, 54, 5) + line(19, 21, 45, 21, 5) + arms
                + head(8, 33, -0.25, -1, 11, 6) + head(56, 33, 0.25, -1, 11, 6))
    if name == "rubble":
        building = poly([(10, 58), (10, 22), (17, 17), (22, 25), (28, 12), (34, 22), (39, 18), (44, 27), (44, 58)])
        windows = "".join(black(rect(x, y, 5, 6, 1)) for x in (16, 30) for y in (32, 44))
        return masked(building, windows) + poly([(46, 58), (52, 49), (59, 58)]) + circle(52, 41, 2.6) + circle(57, 47, 1.8)
    if name == "nuke":
        s = circle(32, 32, 5.5) + ring(32, 32, 27, 3.5)
        for k in range(3):
            a = -90 + 120 * k
            a0, a1 = math.radians(a - 30), math.radians(a + 30)
            r0, r1 = 9.0, 23.0
            pts = [(32 + r0 * math.cos(a0), 32 + r0 * math.sin(a0)), (32 + r1 * math.cos(a0), 32 + r1 * math.sin(a0))]
            pts += [(32 + r1 * math.cos(a0 + (a1 - a0) * t / 8), 32 + r1 * math.sin(a0 + (a1 - a0) * t / 8)) for t in range(1, 9)]
            pts += [(32 + r0 * math.cos(a1), 32 + r0 * math.sin(a1))]
            pts += [(32 + r0 * math.cos(a1 - (a1 - a0) * t / 4), 32 + r0 * math.sin(a1 - (a1 - a0) * t / 4)) for t in range(1, 4)]
            s += poly(pts)
        return s
    if name == "shield":
        outer = "M32,4 L56,12 L54,34 C52,46 43,54 32,60 C21,54 12,46 10,34 L8,12 Z"
        inner = "M32,11 L49,17 L47.5,34 C46,43 39.5,49 32,53.5 C24.5,49 18,43 16.5,34 L15,17 Z"
        return masked(path(outer), black(path(inner, 3.2) + chevron(25, 7, 7, 21, 43)))
    if name == "wrench":
        body = line(12, 52, 35, 29, 10) + circle(41, 23, 15)
        return masked(body, black(poly([(43, 23), (63, 3), (63, 23), (53, 32)]) + circle(41, 23, 5.5) + circle(15, 49, 2.6)))
    if name == "gear":
        teeth = "".join(f'<rect x="27.5" y="4" width="9" height="12" rx="1.5" fill="white" transform="rotate({45 * k} 32 32)"/>' for k in range(8))
        return masked(circle(32, 32, 20) + teeth, black(circle(32, 32, 8)))
    if name == "factory":
        body = poly([(6, 58), (6, 31), (17, 23), (17, 31), (28, 23), (28, 31), (39, 23), (39, 31), (58, 31), (58, 58)])
        cuts = "".join(black(rect(x, 40, 6, 6, 1)) for x in (12, 24, 36)) + black(rect(46, 44, 7, 14, 1))
        return masked(body, cuts) + rect(44, 8, 9, 24, 1.5) + circle(47, 5, 2.5) + circle(53, 3, 1.8)
    if name == "crates":
        def crate(x, y, s):
            return masked(rect(x, y, s, s, 2), black(f'<rect x="{x + 3.5:.2f}" y="{y + 3.5:.2f}" width="{s - 7:.2f}" height="{s - 7:.2f}" fill="none" stroke="black" stroke-width="2.5"/>' + line(x + 4, y + 4, x + s - 4, y + s - 4, 2.5)))
        return crate(7, 34, 24) + crate(33, 34, 24) + crate(20, 8, 24)
    if name == "blueprint":
        grid = "".join(black(line(x, 14, x, 50, 1.6)) for x in (20, 30, 40)) + "".join(black(line(12, y, 52, y, 1.6)) for y in (22, 32, 42))
        sheet = masked(poly([(8, 8), (46, 8), (56, 18), (56, 56), (8, 56)]), grid + black(ring(30, 32, 9, 3)))
        return sheet + poly([(46, 8), (46, 18), (56, 18)])
    if name == "silo":
        missile = path("M32,3 C37,9 38,15 38,21 L38,44 L26,44 L26,21 C26,15 27,9 32,3 Z")
        fins = poly([(26, 33), (19, 45), (26, 44)]) + poly([(38, 33), (45, 45), (38, 44)])
        walls = rect(9, 28, 7, 27, 1.5) + rect(48, 28, 7, 27, 1.5) + rect(6, 54, 52, 6, 2)
        return masked(missile + fins, black(line(28.5, 16, 35.5, 16, 2.2))) + walls
    if name == "cannon":
        barrel = line(20, 41, 52, 15, 8) + line(48, 18, 56, 11.5, 11)
        base = masked(path("M7,58 L7,48 C7,40 15,35 24,35 C33,35 39,41 39,49 L39,58 Z"), black(circle(23, 47, 5)))
        return barrel + base + rect(4, 56, 40, 4, 1.5)
    if name == "gatling":
        barrels = "".join(line(18 + dx, 42 + dy, 52 + dx, 15 + dy, 4) for dx, dy in ((-3.1, -3.9), (0, 0), (3.1, 3.9)))
        clamp = line(36, 21, 43, 30, 5.5)
        return barrels + clamp + masked(rect(6, 42, 24, 16, 3), black(circle(18, 50, 4)))
    if name == "atom":
        return circle(32, 32, 5.5) + "".join(ellipse(32, 32, 27, 9.5, a, 3.6) for a in (0, 60, 120))
    if name == "drill":
        s = line(17, 56, 32, 6, 4.5) + line(47, 56, 32, 6, 4.5) + line(22, 40, 42, 40, 4) + line(26, 26, 38, 26, 4)
        s += line(22, 40, 38, 26, 3) + line(42, 40, 26, 26, 3) + line(6, 58, 58, 58, 4.5)
        return s + poly([(28, 46), (36, 46), (32, 55)])
    if name == "solar":
        panel = poly([(8, 31), (44, 22), (53, 46), (15, 57)])
        cuts = black(line(20, 28, 28, 54, 2.2)) + black(line(32, 25, 40, 50, 2.2)) + black(line(11, 44, 49, 34, 2.2))
        rays = "".join(line(51 + 9 * math.cos(math.radians(a)), 12 + 9 * math.sin(math.radians(a)), 51 + 12.5 * math.cos(math.radians(a)), 12 + 12.5 * math.sin(math.radians(a)), 2.6) for a in range(0, 360, 45))
        return masked(panel, cuts) + circle(51, 12, 6) + rays
    if name == "hook":
        # A load slung under a transport: its hook and ropes, and the crate.
        crate = masked(rect(13, 33, 38, 25, 2), black(f'<rect x="17" y="37" width="30" height="17" fill="none" stroke="black" stroke-width="2.6"/>' + line(18, 38, 46, 53, 2.6)))
        return ring(32, 8, 5, 4) + line(30, 13, 17, 33, 3.5) + line(34, 13, 47, 33, 3.5) + crate
    if name == "recycle":
        s = ""
        for k in range(3):
            a0 = -90 + 120 * k + 18
            a1 = a0 + 82
            s += path(arc_d(32, 33, 21, a0, a1), 5.5)
            ex, ey = 32 + 21 * math.cos(math.radians(a1 + 6)), 33 + 21 * math.sin(math.radians(a1 + 6))
            s += head(ex + 6 * math.cos(math.radians(a1 + 96)), ey + 6 * math.sin(math.radians(a1 + 96)), math.cos(math.radians(a1 + 90)), math.sin(math.radians(a1 + 90)), 12, 8)
        return s
    if name == "ingot":
        def bar(x, y):
            body = poly([(x, y + 12), (x + 6, y), (x + 22, y), (x + 28, y + 12)])
            return masked(body, black(line(x + 7.5, y + 4, x + 20.5, y + 4, 1.8)))
        return bar(5, 44) + bar(31, 44) + bar(18, 29)
    if name == "bolt":
        return poly([(37, 3), (13, 36), (29, 36), (23, 61), (51, 24), (35, 24), (44, 3)])
    if name == "graph":
        bars = rect(8, 41, 10, 15, 1.5) + rect(22, 33, 10, 23, 1.5) + rect(36, 24, 10, 32, 1.5)
        arrow = path("M8,31 L23,19 L33,25 L50,9", 4.5) + head(58, 3, 1, -0.7, 12, 7.5)
        return bars + arrow + line(5, 59, 59, 59, 4)
    if name == "army":
        sides = bust(14, 12, 0.78) + bust(50, 12, 0.78)
        middle = bust(32, 16, 0.98)
        return masked(sides, black(bust(32, 16, 0.98, 3.2))) + middle
    if name == "chevron":
        return chevron(9) + chevron(25) + chevron(41)
    if name == "star":
        return poly(star_points(32, 34, 29, 11.5, 5))
    if name == "medal":
        ribbon = poly([(17, 3), (29, 3), (37, 26), (27, 26)]) + poly([(47, 3), (35, 3), (27, 26), (37, 26)])
        disc = masked(circle(32, 42, 17), black(ring(32, 42, 12, 2.5)))
        return masked(ribbon, black(line(25, 27, 39, 27, 6))) + disc + poly(star_points(32, 42.5, 8, 3.3, 5))
    if name == "book":
        left = path("M30,13 C24,9 14,9 5,11 L5,51 C14,49 24,49 30,53 Z")
        right = path("M34,13 C40,9 50,9 59,11 L59,51 C50,49 40,49 34,53 Z")
        lines = "".join(black(line(10, y, 25, y + 1, 2)) + black(line(39, y + 1, 54, y, 2)) for y in (20, 28, 36))
        return masked(left + right, lines)
    if name == "tower":
        top = rect(11, 9, 9, 12, 1) + rect(27.5, 9, 9, 12, 1) + rect(44, 9, 9, 12, 1) + rect(11, 18, 42, 7, 1)
        body = masked(rect(15, 24, 34, 34), black(path("M26,58 L26,49 A6,6 0 0,1 38,49 L38,58 Z") + rect(29, 31, 6, 9, 3)))
        return top + body + rect(9, 56, 46, 4, 1.5)
    if name == "conquest":
        planet = masked(circle(32, 41, 18), black(path("M15,41 C21,36 26,43 32,39 C38,35 43,42 49,38", 3)))
        return planet + line(32, 4, 32, 24, 4) + poly([(34, 5), (51, 10.5), (34, 16)])
    if name == "question":
        return path("M21,22 C21,12 27,7 33,7 C41,7 46,12 46,20 C46,29 36,31 35,40 L35,43", 7.5) + circle(35, 54, 5)
    raise KeyError(name)


def star_star(cx, cy, outer, inner):
    return poly(star_points(cx, cy, outer, inner, 5))


# ---------------------------------------------------------------------------------------------
# The medallion
# ---------------------------------------------------------------------------------------------


def laurel():
    """Two branches of leaves round the medallion's lower part, meeting under it."""
    s = ""
    for side in (-1, 1):
        for k in range(8):
            a = 90 + side * (14 + k * 15.5)
            r = 55.0
            x, y = CX + r * math.cos(math.radians(a)), CY + r * math.sin(math.radians(a))
            rot = a + 90 + side * 38
            s += ellipse(x, y, 7.2, 2.9, rot)
        a = 90 + side * 133
        s += circle(CX + 55 * math.cos(math.radians(a)), CY + 55 * math.sin(math.radians(a)), 2.4)
    return s


def beads():
    return "".join(circle(CX + 44.5 * math.cos(math.radians(a)), CY + 44.5 * math.sin(math.radians(a)), 1.9) for a in range(0, 360, 12))


def crest():
    return poly(star_points(CX, CY - R_RIM + 1, 12, 5, 5))


def gems():
    return "".join(circle(CX + 44.5 * math.cos(math.radians(a)), CY + 44.5 * math.sin(math.radians(a)), 3.0) for a in (35, 145, 215, 325))


class Canvas:
    def __init__(self, resvg, tmp):
        self.resvg, self.tmp, self.n = resvg, tmp, 0

    def mask(self, fragment, grow=0.0):
        """The coverage of `fragment` (SVG in cell units), optionally grown by `grow` units."""
        self.n += 1
        src = os.path.join(self.tmp, f"m{self.n}.svg")
        out = os.path.join(self.tmp, f"m{self.n}.png")
        body = fragment
        if grow > 0:
            body = (f'<g stroke="white" stroke-width="{2 * grow:.2f}" stroke-linejoin="round">'
                    + fragment.replace('stroke-width="', 'data-w="') + "</g>" + fragment)
        svg = f'<svg xmlns="http://www.w3.org/2000/svg" width="{PX}" height="{PX}" viewBox="0 0 {CELL} {CELL}">{body}</svg>'
        with open(src, "w", encoding="utf-8") as f:
            f.write(svg)
        subprocess.run([self.resvg, src, out], check=True, capture_output=True)
        img = np.asarray(Image.open(out).convert("RGBA"), dtype=np.float32) / 255.0
        return img[..., 0] * img[..., 3]


def blur(a, r):
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


def normals(h, strength):
    gy, gx = np.gradient(h)
    n = np.dstack([-gx * strength, -gy * strength, np.ones_like(h)])
    return n / np.linalg.norm(n, axis=2, keepdims=True)


def shade_metal(mask, metal, bevel, polish=True):
    """A part struck in `metal`: lit through the relief of its coverage (`bevel` units round)."""
    albedo, spec_rgb, shininess, ks = METALS[metal]
    b = max(1.0, bevel * SS)
    h = blur(mask, b) * mask
    n = normals(h, 2.0 * b)
    diffuse = np.clip(n @ LIGHT, 0, 1)
    spec = np.clip(n @ HALF, 0, 1) ** shininess
    y = np.linspace(0, 1, mask.shape[0])[:, None]
    sheen = (1.12 - 0.28 * y) if polish else 1.0
    rgb = np.array(albedo)[None, None, :] * (0.30 + 0.80 * diffuse[..., None]) * np.broadcast_to(sheen, mask.shape)[..., None]
    rgb += np.array(spec_rgb)[None, None, :] * (ks * spec)[..., None]
    return np.clip(rgb, 0, 1)


def shade_enamel(mask, colour, rim):
    """The field: enamel, domed a little, darker in the rim's shadow, with a fine engine turning."""
    yy, xx = np.mgrid[0:PX, 0:PX] / SS
    d = np.hypot(xx - CX, yy - CY) / R_FIELD
    dome = np.clip(1 - d * d, 0, 1) ** 0.5
    h = dome * 0.6 + 0.012 * np.sin(np.hypot(xx - CX, yy - CY) * 2.1)
    n = normals(h, 26.0)
    diffuse = np.clip(n @ LIGHT, 0, 1)
    spec = np.clip(n @ HALF, 0, 1) ** 60
    shadow = 1 - 0.55 * blur(rim, 3.0 * SS)
    rgb = np.array(colour)[None, None, :] * (0.55 + 0.75 * diffuse[..., None]) * shadow[..., None]
    rgb += 0.22 * spec[..., None]
    return np.clip(rgb, 0, 1)


def over(dst, rgb, alpha):
    a = alpha[..., None]
    rgb = np.broadcast_to(rgb, dst.shape[:2] + (3,))
    out = np.empty_like(dst)
    out[..., :3] = rgb * a + dst[..., :3] * (1 - a)
    out[..., 3] = alpha + dst[..., 3] * (1 - alpha)
    return out


class Badges:
    def __init__(self, canvas):
        self.canvas = canvas
        self.frames = {}

    def frame(self, tier, category):
        """The medallion without its glyph (cached by tier and category)."""
        key = (tier, category)
        if key in self.frames:
            return self.frames[key]
        c = self.canvas
        metal = tier if tier in METALS else "steel"
        disc = circle(CX, CY, R_RIM)
        extra = ""
        if tier in ("Gold", "Platinum"):
            extra += laurel()
        if tier == "Platinum":
            extra += crest()
        silhouette = c.mask(disc + extra, grow=1.6)
        shadow = np.zeros_like(silhouette)
        dy = int(round(2.4 * SS))
        shadow[dy:, :] = silhouette[:-dy, :]
        shadow = blur(shadow, 2.4 * SS) * 0.6
        out = np.zeros((PX, PX, 4), dtype=np.float32)
        out[..., 3] = shadow
        out = over(out, np.array(EDGE), silhouette)
        if extra:
            leaves = c.mask(laurel()) if tier in ("Gold", "Platinum") else None
            if leaves is not None:
                out = over(out, shade_metal(leaves, metal, 1.2) * 0.86, leaves)
        rim = c.mask(disc)
        out = over(out, shade_metal(rim, metal, 3.2), rim)
        field = c.mask(circle(CX, CY, R_FIELD))
        groove = c.mask(f'<circle cx="{CX}" cy="{CY}" r="{R_FIELD + 0.9}" fill="none" stroke="white" stroke-width="1.6"/>')
        out = over(out, np.array(EDGE) * 2.2, groove * 0.85)
        out = over(out, shade_enamel(field, ENAMEL[category], 1 - field), field)
        if tier == "Silver":
            b = c.mask(beads())
            out = over(out, shade_metal(b, metal, 0.9), b)
        if tier == "Platinum":
            g = c.mask(gems())
            out = over(out, shade_metal(g, "gem", 1.1), g)
            s = c.mask(crest())
            out = over(out, shade_metal(s, metal, 1.6), s)
            dot = c.mask(circle(CX, CY - R_RIM + 1.5, 2.6))
            out = over(out, shade_metal(dot, "gem", 0.8), dot)
        self.frames[key] = out
        return out

    def badge(self, tier, category, glyph_name):
        out = self.frame(tier, category).copy()
        metal = tier if tier in METALS else "steel"
        s = GLYPH
        g = f'<g transform="translate({CX - 32 * s:.3f} {CY - 32 * s:.3f}) scale({s})">{glyph(glyph_name)}</g>'
        m = self.canvas.mask(g)
        field = self.canvas.mask(circle(CX, CY, R_FIELD - 0.5))
        # Struck into the enamel: a shadow under it first, then the metal.
        sh = np.zeros_like(m)
        dx, dy = int(round(0.9 * SS)), int(round(1.5 * SS))
        sh[dy:, dx:] = m[:-dy, :-dx]
        sh = blur(sh, 1.4 * SS) * field * 0.8
        out = over(out, np.zeros(3), sh)
        out = over(out, shade_metal(m, metal, 1.5), m)
        return out


# ---------------------------------------------------------------------------------------------
# Rank insignia: a shield each, in the order of `ta_config::profiles::RANKS`
# ---------------------------------------------------------------------------------------------

RANK_TITLES = [
    "Recruit", "Private", "Private First Class", "Lance Corporal", "Corporal", "Sergeant", "Staff Sergeant",
    "Master Sergeant", "Sergeant Major", "Warrant Officer", "Second Lieutenant", "Lieutenant", "Captain", "Major",
    "Lieutenant Colonel", "Colonel", "Brigadier", "Major General", "Lieutenant General", "General", "Commander",
]
SHIELD = "M22,16 Q64,4 106,16 L106,66 C106,94 86,112 64,121 C42,112 22,94 22,66 Z"
SHIELD_FIELD = "M30,23 Q64,13 98,23 L98,66 C98,89 81,104 64,112 C47,104 30,89 30,66 Z"


def stripe(y, x0=38.0, x1=90.0, h=13.0, t=8.0):
    """A chevron, its point up."""
    m = (x0 + x1) / 2
    return poly([(x0, y + h), (m, y), (x1, y + h), (x1, y + h + t), (m, y + t), (x0, y + h + t)])


def rocker(y, x0=38.0, x1=90.0, sag=9.0, t=7.0):
    """An arc under the chevrons."""
    m = (x0 + x1) / 2
    return path(f"M{x0},{y} Q{m},{y + 2 * sag} {x1},{y} L{x1},{y + t} Q{m},{y + 2 * sag + t} {x0},{y + t} Z")


def leaf(cx=64.0, cy=60.0):
    body = path(f"M{cx},{cy - 26} C{cx + 20},{cy - 14} {cx + 20},{cy + 12} {cx},{cy + 24} C{cx - 20},{cy + 12} {cx - 20},{cy - 14} {cx},{cy - 26} Z")
    veins = black(line(cx, cy - 18, cx, cy + 18, 2.4)) + "".join(black(line(cx, cy + d, cx + sx * 10, cy + d - 7, 2.0)) for d in (-8, 2, 12) for sx in (-1, 1))
    return masked(body, veins) + line(cx, cy + 22, cx, cy + 32, 4)


def wings(cx=64.0, cy=62.0):
    s = ""
    for sx in (-1, 1):
        for k, (dx, dy, l) in enumerate([(9, -10, 30), (9, -2, 27), (9, 6, 22), (9, 13, 16)]):
            x0, y0 = cx + sx * dx, cy + dy
            s += path(f"M{x0},{y0} L{x0 + sx * l},{y0 - 9 + k * 2} L{x0 + sx * (l - 4)},{y0 + 3} Z")
    body = path(f"M{cx - 7},{cy - 14} L{cx + 7},{cy - 14} L{cx + 6},{cy + 12} L{cx},{cy + 22} L{cx - 6},{cy + 12} Z")
    return s + body + circle(cx, cy - 20, 6)


def stars(n):
    if n == 1:
        spots = [(64, 62, 22)]
    elif n == 2:
        spots = [(47, 62, 15), (81, 62, 15)]
    elif n == 3:
        spots = [(64, 44, 13), (45, 74, 13), (83, 74, 13)]
    elif n == 4:
        spots = [(48, 46, 12), (80, 46, 12), (48, 78, 12), (80, 78, 12)]
    else:
        spots = [(64 + 22 * math.cos(math.radians(-90 + 72 * k)), 63 + 22 * math.sin(math.radians(-90 + 72 * k)), 10) for k in range(5)]
    return "".join(poly(star_points(x, y, r, r * 0.42, 5)) for x, y, r in spots)


def insignia(grade):
    """The shield: its rim metal, field enamel, the insignia metal and the insignia (cell units)."""
    enlisted, officer, general = (0.13, 0.17, 0.11), (0.10, 0.15, 0.28), (0.28, 0.08, 0.08)
    chevrons = {1: [46], 2: [38, 56], 3: [32, 50], 4: [28, 44, 60], 5: [24, 39, 54], 6: [22, 36, 50], 7: [20, 33, 46], 8: [20, 33, 46]}
    rockers = {3: [76], 5: [78], 6: [72, 86], 7: [68, 80, 92], 8: [68, 80, 92]}
    if grade == 0:
        return "steel", enlisted, "brass", circle(64, 60, 9) + ring(64, 60, 16, 3)
    if grade <= 8:
        shapes = "".join(stripe(y) for y in chevrons[grade]) + "".join(rocker(y) for y in rockers.get(grade, []))
        if grade == 8:
            shapes += poly(star_points(64, 64, 8, 3.4, 5))
        return "steel", enlisted, "brass", shapes
    if grade == 9:
        return "Silver", officer, "Silver", masked(rect(38, 52, 52, 16, 3), black(rect(46, 56, 8, 8) + rect(74, 56, 8, 8)))
    if grade == 10:
        return "Silver", officer, "Gold", rect(55, 32, 18, 58, 3)
    if grade == 11:
        return "Silver", officer, "Silver", rect(55, 32, 18, 58, 3)
    if grade == 12:
        return "Silver", officer, "Silver", rect(43, 32, 15, 58, 3) + rect(70, 32, 15, 58, 3)
    if grade == 13:
        return "Gold", officer, "Gold", leaf()
    if grade == 14:
        return "Gold", officer, "Silver", leaf()
    if grade == 15:
        return "Gold", officer, "Silver", wings()
    if grade <= 19:
        return "Gold", general, "Gold", stars(grade - 15)
    return "Platinum", (0.07, 0.07, 0.09), "Platinum", stars(5)


class Insignia:
    def __init__(self, canvas):
        self.canvas = canvas

    def draw(self, grade):
        c = self.canvas
        rim_metal, enamel, metal, shapes = insignia(grade)
        shield = f'<path d="{SHIELD}" fill="white"/>'
        extra = laurel() if grade == 20 else ""
        silhouette = c.mask(shield + extra, grow=1.6)
        shadow = np.zeros_like(silhouette)
        dy = int(round(2.4 * SS))
        shadow[dy:, :] = silhouette[:-dy, :]
        shadow = blur(shadow, 2.4 * SS) * 0.6
        out = np.zeros((PX, PX, 4), dtype=np.float32)
        out[..., 3] = shadow
        out = over(out, np.array(EDGE), silhouette)
        if extra:
            leaves = c.mask(laurel())
            out = over(out, shade_metal(leaves, rim_metal, 1.2) * 0.86, leaves)
        rim = c.mask(shield)
        out = over(out, shade_metal(rim, rim_metal, 3.0), rim)
        field = c.mask(f'<path d="{SHIELD_FIELD}" fill="white"/>')
        out = over(out, shade_enamel(field, enamel, 1 - field), field)
        m = c.mask(shapes)
        sh = np.zeros_like(m)
        dx, dyy = int(round(0.9 * SS)), int(round(1.5 * SS))
        sh[dyy:, dx:] = m[:-dyy, :-dx]
        out = over(out, np.zeros(3), blur(sh, 1.4 * SS) * field * 0.8)
        out = over(out, shade_metal(m, metal, 1.4), m)
        if grade == 20:
            g = c.mask(circle(64, 63, 4.2))
            out = over(out, shade_metal(g, "gem", 0.8), g)
        return out


def to_image(arr, size):
    data = (np.clip(arr, 0, 1) * 255 + 0.5).astype(np.uint8)
    img = Image.frombytes("RGBa", (PX, PX), data.tobytes())
    return img.resize((size, size), Image.LANCZOS).convert("RGBA")


def preview(cells, names, path):
    """Every badge at 96 px with its name, and a strip at 48 and 32 px, on the interface's plate."""
    n = len(cells)
    cols = 12
    rows = (n + cols - 1) // cols
    cw, ch = 132, 132
    sheet = Image.new("RGB", (cols * cw, rows * ch + 120), (22, 21, 19))
    draw = ImageDraw.Draw(sheet)
    for i, (img, name) in enumerate(zip(cells, names)):
        x, y = (i % cols) * cw, (i // cols) * ch
        sheet.paste(img.resize((96, 96), Image.LANCZOS), (x + 18, y + 4), img.resize((96, 96), Image.LANCZOS))
        draw.text((x + 4, y + 104), name[:21], fill=(200, 196, 186))
    y0 = rows * ch + 8
    for i, img in enumerate(cells[:36]):
        small = img.resize((48, 48), Image.LANCZOS)
        sheet.paste(small, (8 + i * 52, y0), small)
        tiny = img.resize((32, 32), Image.LANCZOS)
        sheet.paste(tiny, (8 + i * 52 + 8, y0 + 60), tiny)
    sheet.save(path)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", default=os.path.join(ROOT, "crates", "ta_ui", "badges"))
    parser.add_argument("--preview")
    parser.add_argument("--only", help="draw only these achievements (no atlas), for a preview")
    parser.add_argument("--ranks", action="store_true", help="draw the rank insignia (ranks.png) instead")
    args = parser.parse_args()
    resvg = shutil.which("resvg")
    if not resvg:
        raise SystemExit("resvg is not on the PATH")
    if args.ranks:
        with tempfile.TemporaryDirectory() as tmp:
            drawn = Insignia(Canvas(resvg, tmp))
            cells = [to_image(drawn.draw(g), CELL) for g in range(len(RANK_TITLES))]
        atlas = Image.new("RGBA", (COLS * CELL, ((len(cells) + COLS - 1) // COLS) * CELL), (0, 0, 0, 0))
        for i, img in enumerate(cells):
            atlas.paste(img, ((i % COLS) * CELL, (i // COLS) * CELL))
        os.makedirs(args.out, exist_ok=True)
        out = os.path.join(args.out, "ranks.png")
        atlas.save(out, optimize=True)
        print(f"{out}: {len(cells)} insignia, {atlas.size[0]}x{atlas.size[1]}")
        if args.preview:
            os.makedirs(args.preview, exist_ok=True)
            p = os.path.join(args.preview, "ranks.png")
            preview(cells, RANK_TITLES, p)
            print("preview:", p)
        return
    items = catalogue()
    if args.only:
        wanted = args.only.split(",")
        items = [a for a in items if a["id"] in wanted or a["badge"] in wanted]
    cells, names = [], []
    with tempfile.TemporaryDirectory() as tmp:
        badges = Badges(Canvas(resvg, tmp))
        for a in items:
            cells.append(to_image(badges.badge(a["tier"], a["category"], a["badge"]), CELL))
            names.append(a["id"])
        if not args.only:
            cells.append(to_image(badges.badge("steel", "hidden", "question"), CELL))
            names.append("(secret)")
    if not args.only:
        rows = (len(cells) + COLS - 1) // COLS
        atlas = Image.new("RGBA", (COLS * CELL, rows * CELL), (0, 0, 0, 0))
        for i, img in enumerate(cells):
            atlas.paste(img, ((i % COLS) * CELL, (i // COLS) * CELL))
        os.makedirs(args.out, exist_ok=True)
        out = os.path.join(args.out, "achievements.png")
        atlas.save(out, optimize=True)
        print(f"{out}: {len(cells)} badges, {atlas.size[0]}x{atlas.size[1]}")
    if args.preview:
        os.makedirs(args.preview, exist_ok=True)
        p = os.path.join(args.preview, "badges.png")
        preview(cells, names, p)
        print("preview:", p)


if __name__ == "__main__":
    main()
