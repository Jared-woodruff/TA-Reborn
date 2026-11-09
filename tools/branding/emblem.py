"""Draws TA Reborn's emblem (original art): a hexagonal plate with two stacked chevrons,
machined in brass and gunmetal, lit from the top left. A bevelled brass rim round a sunken plate,
the chevrons raised off it and casting a soft shadow, a rivet at each corner of the rim. Written as
SVG (a 1024-unit square) and rasterised by resvg at every size the icons, the window icons, the
launcher and the installer use:

- 96 px and up: the whole drawing;
- 40 to 64 px: without the rivets (specks at those sizes);
- 16 to 32 px: a cut of its own, drawn on the pixel grid of each size (a rim and chevrons a
  couple of pixels thick, flat brass on a dark plate), not the large drawing shrunk.

    python tools/branding/emblem.py [--out branding] [--preview <dir>]

Writes branding/emblem.svg (the whole drawing) and branding/emblem-<size>.png; then
`ta-tools brand branding` packs the icon and draws the installer's pictures from them, and the
game and the launcher (ta_ui::icon) draw their window icons and the launcher's emblem from them.
Needs Pillow and resvg (https://github.com/linebender/resvg) on the PATH; oxipng, when on the
PATH, makes the PNGs smaller.
"""
import argparse
import math
import pathlib
import shutil
import subprocess
import tempfile

from PIL import Image, ImageDraw

# Every size written: the icon's (ta_tools::brand::ICON_SIZES) and 512 for the README's pictures.
SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256, 512]
SMALL = 32  # this size and below: the small cut
MEDIUM = 64  # this size and below: no rivets

C = 512.0
# Towards the light: the top left.
LIGHT = (math.cos(math.radians(-125)), math.sin(math.radians(-125)))

GOLD_HI = (255, 236, 170)
GOLD = (226, 186, 92)
GOLD_LO = (138, 100, 34)
GOLD_DEEP = (84, 58, 18)
PLATE_HI = (78, 74, 66)
PLATE = (38, 36, 32)
PLATE_LO = (14, 13, 11)
OUTLINE = (10, 9, 7)

# The hexagon's radii (outline, rim, the groove inside the rim, the plate) and the bevels' width.
R_OUTLINE, R_RIM, R_GROOVE, R_PLATE = 508, 496, 418, 404
BEVEL = 20
# The chevrons: apexes (raised a little above the middle so the pair sits optically centred),
# arms reaching HALF either side and dropping DROP, THICK thick.
APEXES = (255, 425)
HALF, DROP, THICK = 220, 180, 96


def hexagon(r, cx=C, cy=C):
    return [(cx + r * math.cos(math.radians(-90 + 60 * k)), cy + r * math.sin(math.radians(-90 + 60 * k))) for k in range(6)]


def pts(points):
    return " ".join(f"{x:.2f},{y:.2f}" for x, y in points)


def path(points):
    return "M" + " L".join(f"{x:.2f},{y:.2f}" for x, y in points) + " Z"


def mix(a, b, t):
    t = max(0.0, min(1.0, t))
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def hexc(c):
    return "#%02x%02x%02x" % c


def chevron(y, half, drop, thick, cx=C):
    """A chevron band: apex at (cx, y), arms reaching `half` either side and dropping `drop`,
    `thick` tall, cut square at their ends."""
    return [(cx, y), (cx + half, y + drop), (cx + half, y + drop + thick), (cx, y + thick), (cx - half, y + drop + thick), (cx - half, y + drop)]


def facet(normal_deg):
    """A bevel facet's colour, by how far its normal faces the light."""
    n = (math.cos(math.radians(normal_deg)), math.sin(math.radians(normal_deg)))
    return hexc(mix(GOLD_DEEP, GOLD_HI, 0.5 + 0.5 * (n[0] * LIGHT[0] + n[1] * LIGHT[1])))


def ring(r_out, r_in, inward):
    """The six trapezoids of a hexagonal band, each shaded by the way it faces (an outer bevel
    faces out, an inner one, sloping down into the plate, in)."""
    o, i = hexagon(r_out), hexagon(r_in)
    return "\n".join(
        f"<path d='{path([o[k], o[(k + 1) % 6], i[(k + 1) % 6], i[k]])}' fill='{facet(-60 + 60 * k + (180 if inward else 0))}'/>" for k in range(6)
    )


DEFS = f"""<defs>
  <linearGradient id="rim" x1="0" y1="0" x2="0.35" y2="1">
    <stop offset="0" stop-color="{hexc(GOLD_HI)}"/><stop offset="0.45" stop-color="{hexc(GOLD)}"/><stop offset="1" stop-color="{hexc(GOLD_LO)}"/>
  </linearGradient>
  <radialGradient id="plate" cx="0.46" cy="0.32" r="0.75">
    <stop offset="0" stop-color="{hexc(PLATE_HI)}"/><stop offset="0.55" stop-color="{hexc(PLATE)}"/><stop offset="1" stop-color="{hexc(PLATE_LO)}"/>
  </radialGradient>
  <linearGradient id="chevron" x1="0" y1="0" x2="0.2" y2="1">
    <stop offset="0" stop-color="{hexc(GOLD_HI)}"/><stop offset="0.5" stop-color="{hexc(GOLD)}"/><stop offset="1" stop-color="{hexc(GOLD_LO)}"/>
  </linearGradient>
  <linearGradient id="lip" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="{hexc(GOLD_LO)}"/><stop offset="1" stop-color="{hexc(GOLD_DEEP)}"/>
  </linearGradient>
  <radialGradient id="rivet" cx="0.35" cy="0.3" r="0.8">
    <stop offset="0" stop-color="#fff4d0"/><stop offset="0.35" stop-color="{hexc(GOLD)}"/><stop offset="1" stop-color="{hexc(GOLD_DEEP)}"/>
  </radialGradient>
  <filter id="shadow" x="-20%" y="-20%" width="140%" height="140%">
    <feGaussianBlur in="SourceAlpha" stdDeviation="9"/><feOffset dx="7" dy="14"/>
    <feComponentTransfer><feFuncA type="linear" slope="0.75"/></feComponentTransfer>
  </filter>
  <filter id="sunk" x="-10%" y="-10%" width="120%" height="120%">
    <feGaussianBlur in="SourceAlpha" stdDeviation="14"/><feOffset dx="0" dy="16"/>
    <feComposite in2="SourceAlpha" operator="arithmetic" k2="-1" k3="1"/>
    <feComponentTransfer><feFuncA type="linear" slope="0.85"/></feComponentTransfer>
  </filter>
</defs>"""


def raised(shape, lip=10):
    """A raised brass piece: its shadow on the plate, its darker edge below, its lit face and a
    bright line along its edges."""
    lower = [(x, y + lip) for x, y in shape]
    return "\n".join([
        f"<path d='{path(shape)}' fill='black' filter='url(#shadow)'/>",
        f"<path d='{path(lower)}' fill='url(#lip)'/>",
        f"<path d='{path(shape)}' fill='url(#chevron)'/>",
        f"<path d='{path(shape)}' fill='none' stroke='#fff2c2' stroke-opacity='0.55' stroke-width='3'/>",
    ])


def large(rivets=True):
    """The whole drawing (rivets only where they are more than specks)."""
    parts = [
        f"<polygon points='{pts(hexagon(R_OUTLINE))}' fill='{hexc(OUTLINE)}'/>",
        f"<polygon points='{pts(hexagon(R_RIM))}' fill='url(#rim)'/>",
        ring(R_RIM, R_RIM - BEVEL, inward=False),
        ring(R_GROOVE + BEVEL, R_GROOVE, inward=True),
        f"<polygon points='{pts(hexagon(R_GROOVE))}' fill='{hexc(OUTLINE)}'/>",
        f"<polygon points='{pts(hexagon(R_PLATE))}' fill='url(#plate)'/>",
        f"<polygon points='{pts(hexagon(R_PLATE))}' fill='black' filter='url(#sunk)'/>",
    ]
    if rivets:
        for x, y in hexagon((R_RIM + R_GROOVE) / 2 - 2):
            parts.append(f"<circle cx='{x:.1f}' cy='{y:.1f}' r='15' fill='{hexc(GOLD_DEEP)}'/>")
            parts.append(f"<circle cx='{x - 1:.1f}' cy='{y - 1:.1f}' r='11.5' fill='url(#rivet)'/>")
    for y in APEXES:
        parts.append(raised(chevron(y, HALF, DROP, THICK)))
    body = "\n".join(parts)
    return f"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 1024 1024' width='1024' height='1024'>\n{DEFS}\n{body}\n</svg>\n"


# The small cut at each size (pixels): the outline, the rim, the chevrons' reach and drop (shares
# of the size), their thickness and the gap between them. From 24 the large drawing's chevrons,
# thickened; at 16 and 20 wider and flatter, so a pixel of plate still shows between them.
SMALL_CUTS = {
    16: (1.0, 1.25, 0.25, 0.16, 2.0, 1.0),
    20: (1.0, 1.5, 0.25, 0.17, 2.0, 1.5),
    24: (1.0, 2.0, 0.215, 0.176, 2.4, 1.5),
    32: (1.25, 2.5, 0.215, 0.176, 3.1, 2.0),
}


def small(n):
    """The cut for small icons, `n` pixels square (16, 20, 24 or 32): a dark outline a pixel
    wide, a brass rim of a pixel or two, the plate, and two chevrons at least two pixels thick
    with a pixel or two of plate between them, flat brass without the large drawing's bevels,
    shadows and rivets (at these sizes they only blur it)."""
    c = n / 2
    r_out = c - 0.25
    outline, rim, half_k, drop_k, thick, gap = SMALL_CUTS[n]
    r_rim, r_plate = r_out - outline, r_out - outline - rim
    half, drop = half_k * n, drop_k * n
    # The pair centred a little above the middle, as the large drawing's is.
    height = (thick + gap) + drop + thick
    top = c - height / 2 - n * 0.03
    tops = (top, top + thick + gap)
    hexs = lambda r: pts(hexagon(r, c, c))
    chevrons = "".join(f"<path d='{path(chevron(y, half, drop, thick, c))}' fill='url(#chevron)'/>" for y in tops)
    return f"""<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 {n} {n}' width='{n}' height='{n}'>
<defs>
  <linearGradient id="rim" x1="0" y1="0" x2="0.3" y2="1"><stop offset="0" stop-color="{hexc(GOLD_HI)}"/><stop offset="0.5" stop-color="{hexc(GOLD)}"/><stop offset="1" stop-color="{hexc(GOLD_LO)}"/></linearGradient>
  <linearGradient id="plate" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{hexc(PLATE_HI)}"/><stop offset="1" stop-color="{hexc(PLATE_LO)}"/></linearGradient>
  <linearGradient id="chevron" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="{hexc(GOLD_HI)}"/><stop offset="1" stop-color="{hexc(GOLD)}"/></linearGradient>
</defs>
<polygon points='{hexs(r_out)}' fill='{hexc(OUTLINE)}' fill-opacity='0.9'/>
<polygon points='{hexs(r_rim)}' fill='url(#rim)'/>
<polygon points='{hexs(r_plate)}' fill='url(#plate)'/>
{chevrons}
</svg>
"""


def render(svg, size, tmp, resvg):
    src = pathlib.Path(tmp) / f"e{size}.svg"
    out = pathlib.Path(tmp) / f"e{size}.png"
    src.write_text(svg, encoding="utf-8")
    subprocess.run([resvg, "--width", str(size), "--height", str(size), str(src), str(out)], check=True, capture_output=True)
    return Image.open(out).convert("RGBA")


def drawing(size):
    return small(size) if size <= SMALL else large(rivets=size > MEDIUM)


def preview(images, out):
    """Every size at 1:1, then the small ones enlarged ×4, on a dark and a light ground."""
    w, h = 1360, 1040
    sheet = Image.new("RGB", (w, h), (30, 34, 40))
    d = ImageDraw.Draw(sheet)
    d.rectangle([0, h // 2, w, h], fill=(236, 238, 240))
    for top in (0, h // 2):
        x = 16
        for s in SIZES[:-1]:
            sheet.paste(images[s], (x, top + 16), images[s])
            x += s + 16
        x = 16
        for s in [16, 20, 24, 32, 40, 48]:
            big = images[s].resize((s * 4, s * 4), Image.NEAREST)
            sheet.paste(big, (x, top + 300), big)
            x += s * 4 + 16
    sheet.save(out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="branding")
    ap.add_argument("--preview", default=None)
    a = ap.parse_args()
    resvg = shutil.which("resvg")
    if not resvg:
        raise SystemExit("resvg is not on the PATH: https://github.com/linebender/resvg")
    out = pathlib.Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "emblem.svg").write_text(large(), encoding="utf-8", newline="\n")
    images = {}
    with tempfile.TemporaryDirectory() as tmp:
        for s in SIZES:
            images[s] = render(drawing(s), s, tmp, resvg)
            images[s].save(out / f"emblem-{s}.png")
    if oxipng := shutil.which("oxipng"):
        subprocess.run([oxipng, "-o", "3", "--strip", "safe", "-q", *[str(out / f"emblem-{s}.png") for s in SIZES]], check=True)
    if a.preview:
        p = pathlib.Path(a.preview)
        p.mkdir(parents=True, exist_ok=True)
        preview(images, p / "emblem-sizes.png")
        for s in SIZES:
            (p / f"emblem-{s}.svg").write_text(drawing(s), encoding="utf-8", newline="\n")
    print(f"wrote branding: emblem.svg and {len(SIZES)} sizes to {out}")


if __name__ == "__main__":
    main()
