"""Builds TA Reborn's own trees, bushes and rocks (original models): the 3D stand-ins the game
draws for TA's painted features when the camera comes close (`ta_render::foliage`).

    python tools/features/build.py [--out crates/ta_render/features] [--only broadleaf_a]
    blender -b -P tools/features/build.py -- --preview <dir> [--only ...]

Everything is grown here from numbers: trees by space colonisation (branches grow towards points
scattered through the crown's shape) or, for the conifers, whorl by whorl; their twigs end in
cards cut out of the leaf textures (`textures.py`); rocks are spheres pushed out to the faces of
random planes, then roughened. Each model comes in three levels of detail. The GLB is written
here (no exporter in between): one node per model and level, `<model>.lod<k>`, one primitive per
texture layer (the material's name is the layer's), and these attributes:

    POSITION    float, world units (1 = a map pixel), y up, the foot at the origin
    NORMAL      float; the leaf cards' are bent towards the crown's own outward normal
    TEXCOORD_0  float; the bark repeats round and up the trunk
    COLOR_0     unsigned bytes: r how open to the sky (ambient occlusion), g how far the wind
                bends it, b how much it flutters (leaves), a a phase of its own

The level-0 node's extras give the model's size: height, width (across the crown) and the
crown's centre and radius (where a burning tree's fire goes).
"""
import argparse
import json
import os
import struct
import sys

import numpy as np

TAU = 2.0 * np.pi
HERE = os.path.dirname(os.path.abspath(__file__))

# Texture layers (textures.py's LAYERS, in order); the first ones are cut out.
CUTOUT = {"broadleaf", "needles", "jungle", "tropical", "smallleaf", "twigs", "clump"}


def unit(v):
    v = np.asarray(v, dtype=np.float64)
    n = np.linalg.norm(v, axis=-1, keepdims=True)
    return v / np.maximum(n, 1e-12)


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


def perpendicular(d, rng=None):
    """A unit vector at right angles to `d` (random about it when `rng` is given)."""
    d = unit(d)
    a = np.array([0.0, 1.0, 0.0]) if abs(d[1]) < 0.9 else np.array([1.0, 0.0, 0.0])
    p = unit(np.cross(d, a))
    if rng is not None:
        ang = rng.uniform(0, TAU)
        q = np.cross(d, p)
        p = p * np.cos(ang) + q * np.sin(ang)
    return p


# ---------------------------------------------------------------------------------------------
# Meshes
# ---------------------------------------------------------------------------------------------


class Part:
    """The triangles of one texture layer."""

    def __init__(self, layer):
        self.layer = layer
        self.pos, self.nrm, self.uv, self.col, self.idx = [], [], [], [], []
        self.count = 0

    def add(self, pos, nrm, uv, col, tris):
        pos = np.asarray(pos, np.float64).reshape(-1, 3)
        self.pos.append(pos)
        self.nrm.append(np.asarray(nrm, np.float64).reshape(-1, 3))
        self.uv.append(np.asarray(uv, np.float64).reshape(-1, 2))
        self.col.append(np.asarray(col, np.float64).reshape(-1, 4))
        self.idx.append(np.asarray(tris, np.int64).reshape(-1, 3) + self.count)
        self.count += len(pos)

    def arrays(self):
        if not self.pos:
            return None
        return (np.concatenate(self.pos), unit(np.concatenate(self.nrm)), np.concatenate(self.uv), np.clip(np.concatenate(self.col), 0, 1), np.concatenate(self.idx))


class Model:
    def __init__(self, name):
        self.name = name
        self.parts = {}
        self.meta = {}

    def part(self, layer):
        if layer not in self.parts:
            self.parts[layer] = Part(layer)
        return self.parts[layer]

    def triangles(self):
        return sum(sum(len(i) for i in p.idx) for p in self.parts.values())


def tube(part, pts, radii, sides, col, tex=(48.0, 64.0), twist=0.0):
    """A tube along `pts` (n, 3) with `radii` (n), `sides` round; `col(points, radii)` gives the
    rings' COLOR_0. The bark texture repeats every `tex` units round and along."""
    pts = np.asarray(pts, np.float64)
    radii = np.asarray(radii, np.float64)
    n = len(pts)
    if n < 2:
        return
    t = np.zeros_like(pts)
    t[1:-1] = pts[2:] - pts[:-2]
    t[0] = pts[1] - pts[0]
    t[-1] = pts[-1] - pts[-2]
    t = unit(t)
    # Frames carried along the tube without turning about it.
    normals = np.zeros_like(pts)
    normals[0] = perpendicular(t[0])
    for i in range(1, n):
        v = normals[i - 1] - t[i] * (normals[i - 1] @ t[i])
        normals[i] = unit(v) if np.linalg.norm(v) > 1e-6 else perpendicular(t[i])
    binormals = np.cross(t, normals)
    ang = np.linspace(0.0, TAU, sides + 1)
    along = np.concatenate([[0.0], np.cumsum(np.linalg.norm(np.diff(pts, axis=0), axis=1))])
    round_tex = max(1.0, np.round(TAU * radii.max() / tex[0]))
    ring_col = col(pts, radii)
    pos, nrm, uv, cols = [], [], [], []
    for i in range(n):
        a = ang[None, :].T + twist * along[i]
        d = np.cos(a) * normals[i] + np.sin(a) * binormals[i]
        pos.append(pts[i] + d * radii[i])
        nrm.append(d)
        uv.append(np.stack([ang / TAU * round_tex, np.full(sides + 1, along[i] / tex[1])], axis=1))
        cols.append(np.repeat(ring_col[i][None, :], sides + 1, axis=0))
    tris = []
    w = sides + 1
    for i in range(n - 1):
        for j in range(sides):
            a, b, c, d = i * w + j, i * w + j + 1, (i + 1) * w + j + 1, (i + 1) * w + j
            tris += [(a, b, c), (a, c, d)]
    part.add(np.concatenate(pos), np.concatenate(nrm), np.concatenate(uv), np.concatenate(cols), tris)


def card(part, base, up, right, width, height, normals, col, hang=False, v=(1.0, 0.0)):
    """A leaf card: its texture's attachment point (the middle of its bottom edge, or with `hang`
    of its top edge) at `base`, reaching `height` along `up`, `width` across along `right`.
    `normals` (4, 3) and `col` (4, 4) per corner: bottom left, bottom right, top right, top left.
    `v`: an upright card's texture v at its base and at its far end."""
    base, up, right = np.asarray(base), unit(up), unit(right)
    h = right * width * 0.5
    if hang:
        corners = [base - up * height - h, base - up * height + h, base + h, base - h]
        uv = [(0.0, 1.0), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)]
    else:
        corners = [base - h, base + h, base + up * height + h, base + up * height - h]
        uv = [(0.0, v[0]), (1.0, v[0]), (1.0, v[1]), (0.0, v[1])]
    part.add(corners, normals, uv, col, [(0, 1, 2), (0, 2, 3)])


# ---------------------------------------------------------------------------------------------
# Branches
# ---------------------------------------------------------------------------------------------


class Skeleton:
    """Nodes (positions), each with its parent (-1 for the root), in growing order."""

    def __init__(self, points):
        self.p = [np.asarray(q, np.float64) for q in points]
        self.parent = [-1] + list(range(len(points) - 1))

    def add(self, p, parent):
        self.p.append(np.asarray(p, np.float64))
        self.parent.append(parent)
        return len(self.p) - 1

    def arrays(self):
        return np.array(self.p), np.array(self.parent)


def colonize(skel, attract, step, influence, kill, rng, tropism=(0.0, 0.0, 0.0), iterations=160, start=0):
    """Grows `skel` towards the `attract` points (space colonisation): each point pulls on its
    nearest node within `influence`; a node pulled grows a node `step` on, along the mean pull
    (plus `tropism`); points within `kill` of a node are used up. Nodes before `start` (the
    trunk below the crown) do not grow."""
    attract = np.asarray(attract, np.float64)
    tropism = np.asarray(tropism, np.float64)
    for _ in range(iterations):
        if len(attract) == 0:
            break
        nodes = np.array(skel.p)
        grow_from = np.arange(start, len(nodes))
        cand = nodes[grow_from]
        d = np.linalg.norm(attract[:, None, :] - cand[None, :, :], axis=2)
        nearest = np.argmin(d, axis=1)
        dmin = d[np.arange(len(attract)), nearest]
        active = dmin < influence
        if not active.any():
            break
        pull = np.zeros_like(cand)
        hits = np.zeros(len(cand))
        dirs = unit(attract[active] - cand[nearest[active]])
        np.add.at(pull, nearest[active], dirs)
        np.add.at(hits, nearest[active], 1.0)
        grew = False
        for k in np.nonzero(hits)[0]:
            direction = unit(unit(pull[k]) + tropism + rng.normal(0, 0.05, 3))
            q = cand[k] + direction * step
            if np.min(np.linalg.norm(nodes - q, axis=1)) < step * 0.4:
                continue
            skel.add(q, grow_from[k])
            grew = True
        nodes = np.array(skel.p)
        d = np.linalg.norm(attract[:, None, :] - nodes[None, len(nodes) - 64:, :], axis=2) if len(nodes) > 64 else np.linalg.norm(attract[:, None, :] - nodes[None, :, :], axis=2)
        attract = attract[d.min(axis=1) > kill]
        if not grew:
            break
    return skel


def radii(skel, tip, exponent=2.4, base=None):
    """Branch radii by the pipe model: a node is as thick as all it carries."""
    p, parent = skel.arrays()
    r = np.full(len(p), 0.0)
    children = [[] for _ in p]
    for i, q in enumerate(parent):
        if q >= 0:
            children[q].append(i)
    for i in range(len(p) - 1, -1, -1):
        if not children[i]:
            r[i] = tip
        else:
            r[i] = sum(r[c] ** exponent for c in children[i]) ** (1.0 / exponent)
    if base is not None:
        r *= base / r[0]
        r = np.maximum(r, tip)
    return r, children


def smooth(skel, children, keep, passes=2):
    """Evens out the branches' kinks (the first `keep` nodes stay put)."""
    p, parent = skel.arrays()
    for _ in range(passes):
        q = p.copy()
        for i in range(keep, len(p)):
            near = [p[parent[i]]] + [p[c] for c in children[i]]
            q[i] = p[i] * 0.5 + np.mean(near, axis=0) * 0.5
        p = q
    skel.p = list(p)


def chains(skel, r, children):
    """The branches as polylines: from the root each follows its thickest child; the others
    start branches of their own (at their parent node)."""
    p, parent = skel.arrays()
    out = []
    stack = [(0, None)]
    while stack:
        i, start = stack.pop()
        line = [start] if start is not None else []
        cur = i
        while True:
            line.append(cur)
            kids = sorted(children[cur], key=lambda c: -r[c])
            if not kids:
                break
            for k in kids[1:]:
                stack.append((k, cur))
            cur = kids[0]
        out.append(line)
    return out


def branch_tubes(model, skel, r, children, min_radius, sides_for, col, layer="bark", tex=(48.0, 64.0), stride=1):
    p, _ = skel.arrays()
    for line in chains(skel, r, children):
        line = [i for i in line]
        if len(line) < 2 or r[line[1]] < min_radius:
            continue
        # Stop where the branch gets too thin to see.
        cut = len(line)
        for k in range(1, len(line)):
            if r[line[k]] < min_radius:
                cut = k + 1
                break
        line = line[:cut]
        rr = r[line].copy()
        rr[0] = r[line[1]] if len(line) > 1 else rr[0]
        pts = p[line]
        # (Its end tapers to a point a little further on, among the leaves: cut off at its
        # thickness it read as a log stuck into the crown.)
        d = pts[-1] - pts[-2]
        if np.linalg.norm(d) > 1e-6:
            pts = np.concatenate([pts, [pts[-1] + unit(d) * rr[-1] * 3.0]])
            rr = np.concatenate([rr, [rr[-1] * 0.08]])
        if stride > 1 and len(pts) > 3:
            keep = sorted(set(range(0, len(pts) - 2, stride)) | {len(pts) - 2, len(pts) - 1})
            pts, rr = pts[keep], rr[keep]
        tube(model.part(layer), pts, rr, sides_for(rr.max()), col, tex)


# ---------------------------------------------------------------------------------------------
# Crowns
# ---------------------------------------------------------------------------------------------


class Crown:
    """The shape the foliage fills: a union of ellipsoids (centre, radii)."""

    def __init__(self, lobes):
        self.lobes = [(np.asarray(c, np.float64), np.asarray(r, np.float64)) for c, r in lobes]

    def depth(self, q):
        """0 at the middle of the nearest lobe, 1 on its surface (more outside)."""
        q = np.atleast_2d(q)
        return np.min([np.linalg.norm((q - c) / r, axis=1) for c, r in self.lobes], axis=0)

    def outward(self, q):
        """The outward normal of the nearest lobe at `q`."""
        q = np.atleast_2d(q)
        k = np.argmin([np.linalg.norm((q - c) / r, axis=1) for c, r in self.lobes], axis=0)
        out = np.zeros_like(q)
        for j, (c, r) in enumerate(self.lobes):
            m = k == j
            out[m] = unit((q[m] - c) / (r * r))
        return out

    def sample(self, n, rng, shell=0.0):
        """`n` points inside (with `shell` > 0, only in its outer part)."""
        out = []
        lo = np.min([c - r for c, r in self.lobes], axis=0)
        hi = np.max([c + r for c, r in self.lobes], axis=0)
        while len(out) < n:
            q = rng.uniform(lo, hi, (n * 4, 3))
            dep = self.depth(q)
            keep = (dep <= 1.0) & (dep >= shell)
            out.extend(q[keep])
        return np.array(out[:n])

    def bounds(self):
        lo = np.min([c - r for c, r in self.lobes], axis=0)
        hi = np.max([c + r for c, r in self.lobes], axis=0)
        return lo, hi


def leaf_normals(crown, corners, card_normal, bend=0.75):
    """Leaf card normals: mostly the crown's outward one, so the crown is lit as a whole."""
    out = crown.outward(corners)
    n = unit(card_normal)
    n = np.where((out @ n)[:, None] < 0, -n, n) if n.ndim == 2 else np.where((out @ n)[:, None] < 0, -n[None, :], n[None, :])
    return unit(out * bend + n * (1.0 - bend))


def card_corners(leaf, hang=False, fold=0.0):
    base, up, right, width, height = leaf
    base, up, right = np.asarray(base), unit(up), unit(right)
    h = right * width * 0.5
    if hang:
        return np.array([base - up * height - h, base - up * height + h, base + h, base - h])
    if fold > 0.0:
        cn = np.cross(right, up)
        lift = (cn if cn[1] >= 0.0 else -cn) * fold * width * 0.5
        return np.array([base - h, base + lift, base + h, base + up * height + h, base + up * height + lift, base + up * height - h])
    return np.array([base - h, base + h, base + up * height + h, base + up * height - h])


def card_centre(leaf, hang=False):
    base, up, _, _, height = leaf
    return np.asarray(base) + unit(up) * height * (-0.5 if hang else 0.5)


def card_phase(leaf, hang=False):
    """A card keeps its wind phase when simplified, resized or reordered at another LOD."""
    # Quantisation removes roundoff from resizing about the same centre. Directions also
    # distinguish the crossed sprays at a conifer's tip and a fern's shared root.
    values = np.rint(np.concatenate([card_centre(leaf, hang), unit(leaf[1]), unit(leaf[2])]) * 10000).astype(np.int64)
    phase = 2166136261
    for value in values:
        phase = ((phase ^ int(value)) * 16777619) & 0xffffffff
    return phase / 0xffffffff


def place_cards(model, crown, cards, layer, hang=False, ao_floor=0.3, bend_of=None, flutter=1.0, v=(1.0, 0.0), fold=0.0):
    """Puts the leaf cards (base, up, right, width, height) into `model`. With `fold`, each is
    folded along its middle, its halves drooping (its middle raised by `fold` of its half width
    along the card's normal turned up): a spray with some body, never edge-on as a whole."""
    part = model.part(layer)
    lo, hi = crown.bounds()
    for leaf in cards:
        base, up, right, width, height = leaf
        base, up, right = np.asarray(base), unit(up), unit(right)
        corners = card_corners(leaf, hang, fold)
        cn = np.cross(right, up)
        n = len(corners)
        normals = leaf_normals(crown, corners, cn)
        depth = crown.depth(corners)
        low = np.clip((corners[:, 1] - lo[1]) / max(hi[1] - lo[1], 1e-6), 0, 1)
        ao = ao_floor + (1.0 - ao_floor) * smoothstep(0.15, 1.05, depth) * (0.7 + 0.3 * low)
        bend = bend_of(corners) if bend_of else np.full(n, 1.0)
        if n == 6:
            tip = np.array([0.35, 0.35, 0.35, 1.0, 1.0, 1.0])
            phase = card_phase(leaf, hang)
            col = np.stack([ao, bend, tip * flutter, np.full(n, phase)], axis=1)
            uv = [(0.0, v[0]), (0.5, v[0]), (1.0, v[0]), (1.0, v[1]), (0.5, v[1]), (0.0, v[1])]
            part.add(corners, normals, uv, col, [(0, 1, 4), (0, 4, 5), (1, 2, 3), (1, 3, 4)])
            continue
        tip = np.array([0.35, 0.35, 1.0, 1.0]) if not hang else np.array([1.0, 1.0, 0.35, 0.35])
        phase = card_phase(leaf, hang)
        col = np.stack([ao, bend, tip * flutter, np.full(4, phase)], axis=1)
        card(part, base, up, right, width, height, normals, col, hang, v)


def thin_cards(cards, keep, grow=None, hang=False, fold=0.0):
    """Nested subsets of one crown, retaining its extrema and each card's centre/orientation.
    Remaining sprays grow only inside the full crown's bounds, never expanding its silhouette.
    The same ordered selection at every level prevents random layouts or wind phases popping."""
    if keep >= len(cards):
        return cards
    centres = np.array([card_centre(c, hang) for c in cards])
    corners = np.array([card_corners(c, hang, fold) for c in cards])
    lo, hi = corners.min(axis=(0, 1)), corners.max(axis=(0, 1))
    chosen = []
    for extreme in (corners.min(axis=1).argmin(axis=0), corners.max(axis=1).argmax(axis=0)):
        for k in extreme:
            if int(k) not in chosen:
                chosen.append(int(k))
    keep = max(keep, len(chosen))
    dist = np.min(np.linalg.norm(centres[:, None, :] - centres[chosen], axis=2), axis=1)
    while len(chosen) < keep:
        dist[chosen] = -1.0
        k = int(np.argmax(dist))
        chosen.append(k)
        dist = np.minimum(dist, np.linalg.norm(centres - centres[k], axis=1))
    s = grow if grow is not None else np.sqrt(len(cards) / keep) * 0.92
    out = []
    for k in chosen:
        b, u, r, w, h = cards[k]
        d = corners[k] - centres[k]
        room = np.where(d >= 0.0, hi - centres[k], centres[k] - lo)
        limit = np.min(np.divide(room, np.abs(d), out=np.full_like(d, np.inf), where=np.abs(d) > 1e-10))
        scale = max(1.0, min(s, limit))
        base = centres[k] + unit(u) * h * scale * (0.5 if hang else -0.5)
        out.append((base, u, r, w * scale, h * scale))
    return out


def bark_colour(height, crown_bottom, crown, phase=0.0, open_sky=False):
    """COLOR_0 for branch rings: open to the sky but darker inside the crown and at the foot
    (a burnt tree's crown, with `open_sky`, shades nothing: it has no leaves); bending with
    height."""
    def f(pts, rr):
        dep = crown.depth(pts)
        inside = 0.0 if open_sky else 1.0 - smoothstep(0.6, 1.1, dep)
        ao = (0.85 - 0.4 * inside) * (0.8 + 0.2 * smoothstep(0.0, 12.0, pts[:, 1]))
        bend = np.clip(pts[:, 1] / height, 0, 1) ** 1.6
        return np.stack([ao, bend, np.zeros(len(pts)), np.zeros(len(pts))], axis=1)
    return f


# ---------------------------------------------------------------------------------------------
# The trees
# ---------------------------------------------------------------------------------------------

LOD_CARDS = (1.0, 0.36, 0.18)


def taper_trunk(r, nodes):
    """The pipe model keeps a trunk as thick as the crown it carries all the way down, a
    cylinder: the first `nodes` (the trunk) taper up from a broader foot instead, flaring out
    where it goes into the ground."""
    for i in range(nodes):
        t = 1.0 - i / max(nodes - 1, 1)
        r[i] *= 1.0 + 0.25 * t ** 1.5 + 0.45 * t ** 8


def sides(lod):
    def f(r):
        if lod == 0:
            return 10 if r > 4.0 else 7 if r > 1.5 else 5
        if lod == 1:
            return 7 if r > 4.0 else 5
        return 5
    return f


MIN_RADIUS = (0.35, 0.9, 2.2)


def shell_cards(crown, count, size, rng, inner=0.2, depth=0.45, rise=0.5):
    """Leaf cards over a crown's outer part (and `inner` as many again inside): each centred on
    its point, facing out and a little up, turned at random about that."""
    pts = np.concatenate([crown.sample(count, rng, shell=depth), crown.sample(max(int(count * inner), 1), rng)])
    out = crown.outward(pts)
    cards = []
    for q, o in zip(pts, out):
        up = unit(o * 0.7 + np.array([0.0, rise, 0.0]) + rng.normal(0, 0.3, 3))
        s = size * rng.uniform(0.8, 1.2)
        cards.append((q - up * s * 0.45, up, perpendicular(up, rng), s, s))
    return cards


# Leaf cards per level of detail (as a share of the full count) and how much bigger each is.
LEAF_LEVELS = ((1.0, 1.0), (0.32, 1.55), (0.16, 2.2))


def broadleaf(name, seed, lod, lobes, trunk_top, trunk_radius, leaf_size, layer="broadleaf", count=360, lean=0.0):
    """A broadleaf tree: a trunk up into a crown of `lobes`, branches grown to fill it, its outer
    part covered with sprays of leaves (far away, with clumps of them)."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    crown = Crown(lobes)
    lo, hi = crown.bounds()
    height = hi[1]
    trunk = [np.array([lean * t * t * trunk_top, -SINK + t * (trunk_top + SINK), 0.0]) for t in np.linspace(0, 1, 8)]
    skel = Skeleton(trunk)
    colonize(skel, crown.sample(700, rng), step=4.5, influence=28.0, kill=7.0, rng=rng, tropism=(0.0, 0.08, 0.0), start=len(trunk) - 3)
    r, children = radii(skel, 0.25, base=trunk_radius)
    taper_trunk(r, len(trunk))
    smooth(skel, children, keep=len(trunk))
    bark = bark_colour(height, lo[1], crown, rng.random())
    branch_tubes(m, skel, r, children, MIN_RADIUS[lod], sides(lod), bark, tex=(40.0, 56.0))
    # (Low buttresses, half in the flare: not stilts.)
    roots(m, trunk_radius, 6, trunk_radius * 2.3, bark, lod, rng, rise=1.1, thick=0.45)
    share, grow = LEAF_LEVELS[lod]
    cards = shell_cards(crown, count, leaf_size, np.random.default_rng(seed + 1))
    cards = thin_cards(cards, max(int(len(cards) * share), 8), grow=grow)
    bend = lambda c: np.clip(c[:, 1] / height, 0, 1) ** 1.6
    place_cards(m, crown, cards, layer, bend_of=bend)
    c, rad = crown.lobes[0]
    m.meta = {"height": float(height), "width": float(hi[0] - lo[0] + hi[2] - lo[2]) * 0.5, "crown": [float(x) for x in c] + [float(rad.mean())]}
    return m


def conifer(name, seed, lod, height, radius, base, trunk_radius, leaf_size, count=None):
    """A conifer: a straight trunk to the top, whorls of branches sloping down, shortest near the
    top, each a fan of needle sprays; columnar when `radius` is small for the height."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    profile = lambda t: np.sin(np.pi * np.clip(t, 0, 1) ** 0.75) ** 0.8
    centre = np.array([0.0, base + (height - base) * 0.42, 0.0])
    crown = Crown([(centre, (radius, (height - base) * 0.55, radius))])
    trunk = np.array([[0.0, y, 0.0] for y in np.linspace(-SINK, height, 14)])
    tr = trunk_radius * (1.0 - np.linspace(0, 1, 14) * 0.9)
    bark = bark_colour(height, base, crown, rng.random())
    tube(m.part("bark"), trunk, tr, sides(lod)(trunk_radius), bark, (36.0, 48.0))
    cards = []
    whorls = int((height - base) / 4.6)
    for k in range(whorls):
        t = (k + 0.5) / whorls
        y = base + t * (height - base)
        reach = radius * profile(t) * rng.uniform(0.85, 1.05)
        n = 7
        for j in range(n):
            ang = (j + rng.uniform(-0.3, 0.3)) / n * TAU + k * 0.9
            out = np.array([np.cos(ang), -0.25 - 0.2 * (1 - t), np.sin(ang)])
            start = np.array([0.0, y, 0.0])
            end = start + unit(out) * reach
            if lod == 0 and reach > 6.0:
                tube(m.part("bark"), [start, (start + end) / 2, end], [trunk_radius * 0.18, trunk_radius * 0.1, 0.15], 4, bark, (36.0, 48.0))
            # Sprays along it, flat, pointing outwards and a little up.
            for s in (0.25, 0.65):
                at = start + (end - start) * s
                d = unit(out + np.array([0.0, 0.45, 0.0]) + rng.normal(0, 0.15, 3))
                size = max(leaf_size * (0.55 + 0.6 * profile(t)), 4.0) * rng.uniform(0.85, 1.15)
                right = unit(np.cross(d, np.array([0.0, 1.0, 0.0])) + rng.normal(0, 0.25, 3))
                cards.append((at, d, right, size, size * 1.15))
    # The top shoot.
    cards.append((np.array([0.0, height - leaf_size * 0.9, 0.0]), np.array([0.0, 1.0, 0.0]), np.array([1.0, 0.0, 0.0]), leaf_size * 0.5, leaf_size))
    cards.append((np.array([0.0, height - leaf_size * 0.9, 0.0]), np.array([0.0, 1.0, 0.0]), np.array([0.0, 0.0, 1.0]), leaf_size * 0.5, leaf_size))
    cards = thin_cards(cards, max(int(len(cards) * LOD_CARDS[lod]), 8), fold=0.55)
    bend = lambda c: np.clip(c[:, 1] / height, 0, 1) ** 1.4
    place_cards(m, crown, cards, "needles", ao_floor=0.62, bend_of=bend, flutter=0.5, fold=0.55)
    m.meta = {"height": float(height), "width": float(radius * 2.0), "crown": [float(x) for x in centre] + [float(radius)]}
    return m


# The trees' trunks reach this far below their foot, so on a slope none floats.
SINK = 5.0


def curve(a, b, lift, n, bow=None):
    """A bowed line from `a` to `b` (n points): bowed up by `lift`, or along `bow`."""
    a, b = np.asarray(a, float), np.asarray(b, float)
    mid = (a + b) / 2 + (np.asarray(bow, float) if bow is not None else np.array([0.0, lift, 0.0]))
    t = np.linspace(0.0, 1.0, n)[:, None]
    return (1 - t) ** 2 * a + 2 * (1 - t) * t * mid + t * t * b


def hanging_cards(centre, radii, count, length, rng, low=0.35):
    """Strands hanging from round the rim of a lobe (centre, radii), each card a curtain."""
    c, r = np.asarray(centre, float), np.asarray(radii, float)
    cards = []
    for k in range(count):
        a = (k + rng.uniform(-0.35, 0.35)) / count * TAU
        rim = c + np.array([np.cos(a) * r[0] * rng.uniform(0.82, 1.0), -r[1] * rng.uniform(0.1, low), np.sin(a) * r[2] * rng.uniform(0.82, 1.0)])
        tangent = np.array([-np.sin(a), 0.0, np.cos(a)])
        right = unit(tangent + rng.normal(0, 0.25, 3) * np.array([1, 0, 1]))
        l = length * rng.uniform(0.75, 1.2)
        cards.append((rim, np.array([0.0, 1.0, 0.0]), right, l * 0.8, l))
    return cards


def top_cards(centre, radii, count, size, rng, layer_floor=0.25):
    """Leaf cards over the upper part of a lobe."""
    crown = Crown([(centre, radii)])
    cards = shell_cards(crown, count, size, rng, inner=0.15, rise=0.9)
    floor = centre[1] - radii[1] * layer_floor
    return [c for c in cards if c[0][1] + unit(c[1])[1] * c[4] * 0.5 >= floor]


def twig_cards(points, directions, size, rng, count=2):
    """Bare twigs (the burnt trees' crowns) at the ends of branches."""
    cards = []
    for p, d in zip(points, directions):
        for _ in range(count):
            up = unit(np.asarray(d) + np.array([0.0, 0.6, 0.0]) + rng.normal(0, 0.45, 3))
            s = size * rng.uniform(0.75, 1.15)
            cards.append((np.asarray(p) - up * s * 0.1, up, perpendicular(up, rng), s, s))
    return cards


# The burnt trees' twigs (on the wood drawn, see `bare_ends`): their share and growth by level.
DEAD_LEVELS = ((1.0, 1.0), (0.55, 1.25), (0.35, 1.45))


def finish_tree(m, crown, cards, layer, rng, height, lod, hang_cards=(), hang_layer="jungle", dead=False):
    """The leaf cards (fewer and bigger at coarser levels; bare twigs when `dead`) and the
    model's size."""
    bend = lambda c: np.clip(c[:, 1] / height, 0, 1) ** 1.5
    share, grow = LEAF_LEVELS[lod]
    if cards and dead:
        share, grow = DEAD_LEVELS[lod]
        cards = thin_cards(cards, max(int(len(cards) * share), min(len(cards), 4)), grow=grow if lod else 1.0)
        place_cards(m, crown, cards, "twigs", bend_of=bend, flutter=0.3, ao_floor=0.65)
    elif cards:
        cards = thin_cards(cards, max(int(len(cards) * share), 6), grow=grow if lod else 1.0)
        place_cards(m, crown, cards, layer, bend_of=bend, flutter=1.0)
    if hang_cards and not dead:
        hang = thin_cards(list(hang_cards), max(int(len(hang_cards) * min(share * 1.6, 1.0)), 4), grow=1.0 + (grow - 1.0) * 0.5 if lod else 1.0, hang=True)
        place_cards(m, crown, hang, hang_layer, hang=True, ao_floor=0.35, bend_of=bend, flutter=0.8)
    lo, hi = crown.bounds()
    big = max(crown.lobes, key=lambda l: l[1].prod())
    m.meta = {"height": float(max(height, hi[1])), "width": float(hi[0] - lo[0] + hi[2] - lo[2]) * 0.5, "crown": [float(x) for x in big[0]] + [float(big[1].mean())]}
    return m


def roots(m, trunk_radius, count, reach, bark, lod, rng, layer="bark", rise=2.2, thick=0.55):
    """Roots flaring out from the foot of a trunk, from `rise` trunk radii up it, `thick` of its
    radius where they leave it."""
    for k in range(count):
        a = (k + rng.uniform(-0.3, 0.3)) / count * TAU
        out = np.array([np.cos(a), 0.0, np.sin(a)])
        pts = curve(np.array([0.0, trunk_radius * rise, 0.0]) + out * trunk_radius * 0.4, out * reach * rng.uniform(0.7, 1.1) + np.array([0.0, -SINK * 0.6, 0.0]), 0.0, 6, bow=out * trunk_radius * 0.6 + np.array([0.0, trunk_radius * 0.6, 0.0]))
        # Grow the same layout even when roots are omitted, so the rest of the tree's
        # random stream (branches, hanging strands and leaves) stays identical.
        if lod < 2:
            tube(m.part(layer), pts, np.linspace(trunk_radius * thick, trunk_radius * 0.12, 6), sides(lod)(trunk_radius * 0.5), bark, (36.0, 48.0))


def jungle_canopy(name, seed, lod, lobes, trunk_height, trunk_radius, leaf_size, strand, dead=False):
    """A jungle giant: a thick, twisting trunk that splits into limbs, each bearing a wide dome
    of leaves with strands hanging from its rim."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    crown = Crown(lobes)
    height = crown.bounds()[1][1]
    bark = bark_colour(height, trunk_height, crown, rng.random())
    trunk = curve((0.0, -SINK, 0.0), (rng.uniform(-3, 3), trunk_height, rng.uniform(-3, 3)), 0.0, 9, bow=(rng.uniform(-6, 6), 0.0, rng.uniform(-6, 6)))
    tube(m.part("bark"), trunk, np.linspace(trunk_radius * 1.15, trunk_radius * 0.75, 9), sides(lod)(trunk_radius), bark, (40.0, 56.0), twist=0.02)
    roots(m, trunk_radius, 5, trunk_radius * 4.5, bark, lod, rng)
    top = trunk[-1]
    cards, hang, tips, dirs = [], [], [], []
    for c, r in crown.lobes:
        c, r = np.asarray(c, float), np.asarray(r, float)
        end = c - np.array([0.0, r[1] * 0.45, 0.0])
        limb = curve(top, end, 0.0, 7, bow=(end - top) * np.array([0.25, 0.0, 0.25]) + np.array([0.0, r[1] * 0.5, 0.0]))
        tube(m.part("bark"), limb, np.linspace(trunk_radius * 0.6, trunk_radius * 0.28, 7), sides(lod)(trunk_radius * 0.6), bark, (40.0, 56.0))
        # Smaller branches spreading under the dome.
        for k in range(4):
            a = rng.uniform(0, TAU)
            tip = c + np.array([np.cos(a) * r[0] * 0.7, -r[1] * 0.1, np.sin(a) * r[2] * 0.7])
            twig = curve(end, tip, r[1] * 0.25, 5)
            if dead or lod == 0 or (lod == 1 and k < 2):
                tube(m.part("bark"), twig, np.linspace(trunk_radius * 0.25, 0.3, 5), sides(lod)(1.0), bark, (40.0, 56.0))
            tips.append(tip)
            dirs.append(tip - end)
        cards += top_cards(c, r, int(110 * (r[0] / 20.0) ** 2), leaf_size, rng)
        hang += hanging_cards(c, r, int(30 * r[0] / 20.0), strand, rng)
    if dead:
        cards = twig_cards(tips, dirs, leaf_size * 1.6, rng, 3)
    return finish_tree(m, crown, cards, "broadleaf", rng, height, lod, hang, dead=dead)


def jungle_tall(name, seed, lod, height, lobes, trunk_radius, leaf_size, strand, dead=False):
    """A tall jungle tree: a trunk bending as it climbs, its leaves in hanging clumps along it and
    at its top, roots flaring at its foot."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    crown = Crown(lobes)
    bark = bark_colour(height, lobes[0][0][1] - lobes[0][1][1], crown, rng.random())
    sway = rng.uniform(-1, 1, 2) * 9.0
    pts = np.array([[np.sin(t * 2.6) * sway[0] * t, -SINK + t * (height * 0.86 + SINK), np.sin(t * 2.1 + 1.0) * sway[1] * t] for t in np.linspace(0, 1, 14)])
    tube(m.part("bark"), pts, np.linspace(trunk_radius * 1.2, trunk_radius * 0.45, 14), sides(lod)(trunk_radius), bark, (36.0, 48.0), twist=0.03)
    roots(m, trunk_radius, 6, trunk_radius * 4.0, bark, lod, rng)
    cards, hang, tips, dirs = [], [], [], []
    for c, r in crown.lobes:
        c, r = np.asarray(c, float), np.asarray(r, float)
        # A limb from the nearest point of the trunk to the lobe.
        k = int(np.argmin(np.linalg.norm(pts - c, axis=1)))
        start = pts[max(k - 1, 0)]
        limb = curve(start, c - np.array([0.0, r[1] * 0.3, 0.0]), r[1] * 0.4, 5)
        tube(m.part("bark"), limb, np.linspace(trunk_radius * 0.45, trunk_radius * 0.2, 5), sides(lod)(trunk_radius * 0.5), bark, (36.0, 48.0))
        tips.append(limb[-1])
        dirs.append(limb[-1] - limb[0])
        cards += top_cards(c, r, int(95 * (r[0] / 18.0) ** 2), leaf_size, rng, layer_floor=0.5)
        hang += hanging_cards(c, r, int(26 * r[0] / 18.0), strand, rng, low=0.45)
    if dead:
        cards = twig_cards(tips + [pts[-1]], dirs + [pts[-1] - pts[-3]], leaf_size * 1.5, rng, 3)
    return finish_tree(m, crown, cards, "broadleaf", rng, height, lod, hang, dead=dead)


def tuber(name, seed, lod, bulb, stems, height, pad, dead=False):
    """A tuber tree: a swollen root mass, stems curving up out of it, each with flat pads of big
    leaves."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    crown = Crown([((0.0, height * 0.75, 0.0), (height * 0.42, height * 0.3, height * 0.42)), ((0.0, bulb * 0.5, 0.0), (bulb * 1.1, bulb * 0.8, bulb * 1.1))])
    # The root mass: a lumpy, squashed ball, sunk a little.
    v, f = icosphere((2, 2, 1)[lod])
    pts = v * (1.0 + 0.16 * fbm(v * 2.0, seed, 3))[:, None] * np.array([bulb, bulb * 0.72, bulb]) + np.array([0.0, bulb * 0.45, 0.0])
    nrm = np.zeros_like(pts)
    fn = np.cross(pts[f[:, 1]] - pts[f[:, 0]], pts[f[:, 2]] - pts[f[:, 0]])
    for k in range(3):
        np.add.at(nrm, f[:, k], fn)
    nrm = unit(nrm)
    ao = 0.55 + 0.45 * smoothstep(-0.3, 0.8, nrm[:, 1])
    m.part("bark_smooth").add(pts, nrm, np.stack([np.arctan2(v[:, 2], v[:, 0]) / TAU * 3.0, v[:, 1] * 1.2], axis=1), np.stack([ao, np.zeros(len(pts)), np.zeros(len(pts)), np.zeros(len(pts))], axis=1), f)
    bark = bark_colour(height, bulb, crown, rng.random())
    cards, tips, dirs = [], [], []
    for k in range(stems):
        a = (k + rng.uniform(-0.3, 0.3)) / stems * TAU
        out = np.array([np.cos(a), 0.0, np.sin(a)])
        top = out * height * rng.uniform(0.2, 0.42) + np.array([0.0, height * rng.uniform(0.62, 0.98), 0.0])
        start = out * bulb * 0.3 + np.array([0.0, bulb * 0.9, 0.0])
        stem = curve(start, top, 0.0, 7, bow=-out * height * 0.08 + np.array([0.0, height * 0.12, 0.0]))
        tube(m.part("bark_smooth"), stem, np.linspace(bulb * 0.16, bulb * 0.07, 7), sides(lod)(1.0), bark, (24.0, 32.0))
        tips.append(top)
        dirs.append(top - start)
        # A pad: big leaves spread flat round the stem's end.
        for j in range(7):
            b = a + j * TAU / 7 + rng.uniform(-0.25, 0.25)
            d = np.array([np.cos(b), rng.uniform(0.05, 0.4), np.sin(b)])
            s = pad * rng.uniform(0.85, 1.15)
            cards.append((top - unit(d) * s * 0.12, d, unit(np.cross(d, np.array([0.0, 1.0, 0.0]))), s, s))
    if dead:
        cards = twig_cards(tips, dirs, pad * 1.2, rng, 2)
    return finish_tree(m, crown, cards, "tropical", rng, height, lod, dead=dead)


def tropical(name, seed, lod, height, spread, branches, leaf_size, dead=False):
    """A tropical tree: a slender trunk forking into a few long, curving branches, each ending in
    a rosette of big leaves, smaller clusters along them."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    fork = np.array([rng.uniform(-3, 3), height * rng.uniform(0.3, 0.4), rng.uniform(-3, 3)])
    lobes, tips, dirs, cards = [], [], [], []
    ends = []
    for k in range(branches):
        a = (k + rng.uniform(-0.25, 0.25)) / branches * TAU
        out = np.array([np.cos(a), 0.0, np.sin(a)])
        end = out * spread * rng.uniform(0.32, 0.5) + np.array([0.0, height * rng.uniform(0.78, 1.0), 0.0])
        ends.append((out, end))
        lobes.append((end + np.array([0.0, leaf_size * 0.15, 0.0]), np.array([leaf_size * 1.1, leaf_size * 0.45, leaf_size * 1.1])))
    crown = Crown(lobes)
    bark = bark_colour(height, fork[1], crown, rng.random(), open_sky=dead)
    trunk = curve((0.0, -SINK, 0.0), fork, 0.0, 7, bow=(rng.uniform(-4, 4), 0.0, rng.uniform(-4, 4)))
    tube(m.part("bark"), trunk, np.linspace(2.8, 2.1, 7) if dead else np.linspace(2.6, 1.9, 7), sides(lod)(2.5), bark, (30.0, 40.0), twist=0.04)
    for out, end in ends:
        branch = curve(fork, end, 0.0, 8, bow=out * spread * 0.12 + np.array([0.0, height * 0.12, 0.0]))
        tube(m.part("bark"), branch, np.linspace(2.0, 1.0, 8) if dead else np.linspace(1.6, 0.55, 8), sides(lod)(1.6), bark, (30.0, 40.0))
        tips.append(end)
        dirs.append(end - fork)
        # The rosette at its end.
        for j in range(10):
            b = (j + rng.uniform(-0.3, 0.3)) / 10 * TAU
            d = np.array([np.cos(b), rng.uniform(-0.1, 0.65), np.sin(b)])
            s = leaf_size * rng.uniform(0.9, 1.25)
            cards.append((end - unit(d) * s * 0.1, d, unit(np.cross(d, np.array([0.0, 1.0, 0.0])) + rng.normal(0, 0.2, 3)), s, s))
        # Smaller clusters partway along.
        for at in (3, 5):
            mid = branch[at]
            for j in range(4):
                b = rng.uniform(0, TAU)
                d = np.array([np.cos(b), rng.uniform(0.1, 0.6), np.sin(b)])
                s = leaf_size * 0.75
                cards.append((mid, d, unit(np.cross(d, np.array([0.0, 1.0, 0.0]))), s, s))
    if dead:
        cards = twig_cards(tips, dirs, leaf_size * 1.1, rng, 3)
    return finish_tree(m, crown, cards, "tropical", rng, height, lod, dead=dead)


def bush(name, seed, lod, width, height, leaf_size, layer="smallleaf", count=46):
    """A bush: a low dome of small leaves on a few twigs."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    crown = Crown([((0.0, height * 0.42, 0.0), (width * 0.5, height * 0.62, width * 0.5)), ((width * 0.18, height * 0.3, -width * 0.12), (width * 0.32, height * 0.45, width * 0.3))])
    bark = bark_colour(height, 0.0, crown, rng.random())
    for k in range(5):
        a = rng.uniform(0, TAU)
        tip = np.array([np.cos(a) * width * 0.3, height * 0.6, np.sin(a) * width * 0.3])
        if lod == 0:
            tube(m.part("bark"), curve((0.0, -1.0, 0.0), tip, height * 0.1, 4), np.linspace(0.6, 0.2, 4), 4, bark, (20.0, 24.0))
    share, grow = ((1.0, 1.0), (0.55, 1.3), (0.3, 1.7))[lod]
    cards = shell_cards(crown, count, leaf_size, rng, inner=0.25, rise=0.9)
    cards = thin_cards(cards, max(int(len(cards) * share), 5), grow=grow)
    place_cards(m, crown, cards, layer, bend_of=lambda c: np.clip(c[:, 1] / height, 0, 1) * 0.4, flutter=0.7)
    m.meta = {"height": float(height), "width": float(width), "crown": [0.0, float(height * 0.42), 0.0, float(width * 0.5)]}
    return m


def ground_plant(name, seed, lod, width, height, layer="tropical", fronds=9, v=(1.0, 0.0)):
    """A ground plant: a rosette of big leaves arching out from its middle (`v`: see `card`)."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    crown = Crown([((0.0, height * 0.3, 0.0), (width * 0.5, height * 0.7, width * 0.5))])
    cards = []
    n = fronds
    for k in range(n):
        a = (k + rng.uniform(-0.3, 0.3)) / n * TAU
        tilt = rng.uniform(0.35, 0.95)
        d = np.array([np.cos(a), tilt, np.sin(a)])
        s = width * rng.uniform(0.38, 0.5)
        cards.append((np.array([0.0, -0.5, 0.0]), d, unit(np.cross(d, np.array([0.0, 1.0, 0.0]))), s * 0.9, s))
    cards = thin_cards(cards, (fronds, max(fronds * 2 // 3, 4), 4)[lod], grow=1.15 if lod == 2 else 1.0)
    place_cards(m, crown, cards, layer, ao_floor=0.45, bend_of=lambda c: np.clip(c[:, 1] / height, 0, 1) * 0.3, flutter=0.8, v=v)
    m.meta = {"height": float(height), "width": float(width), "crown": [0.0, float(height * 0.4), 0.0, float(width * 0.5)]}
    return m


def dead_broadleaf(name, seed, lod, lobes, trunk_top, trunk_radius, twig_size):
    """A broadleaf tree burnt: its branches bare, charred twigs at their ends."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    crown = Crown(lobes)
    lo, hi = crown.bounds()
    height = hi[1]
    trunk = [np.array([0.0, -SINK + t * (trunk_top + SINK), 0.0]) for t in np.linspace(0, 1, 8)]
    skel = Skeleton(trunk)
    colonize(skel, crown.sample(260, rng), step=5.0, influence=30.0, kill=9.0, rng=rng, tropism=(0.0, 0.12, 0.0), start=len(trunk) - 3)
    r, children = radii(skel, 0.3, base=trunk_radius)
    taper_trunk(r, len(trunk))
    smooth(skel, children, keep=len(trunk))
    # (Its limbs thicker than a live tree's twigs, and kept at every level: a bare pole with
    # tufts beside it read as no tree at all.)
    r = np.maximum(r * 1.35, 0.45)
    # A bare tree's branch ends define its outline. Simplify their tubes, rather than
    # shortening them and relocating every twig onto a different branch at each LOD.
    least = 0.45
    bark = bark_colour(height, lo[1], crown, rng.random(), open_sky=True)
    branch_tubes(m, skel, r, children, least, sides(lod) if lod < 2 else lambda r: 3, bark, tex=(40.0, 56.0), stride=(1, 2, 3)[lod])
    roots(m, trunk_radius, 5, trunk_radius * 2.2, bark, lod, rng, rise=1.1, thick=0.45)
    p, parent = skel.arrays()
    ends = bare_ends(p, parent, r, children, least)
    cards = twig_cards([p[i] for i in ends], [p[i] - p[parent[i]] for i in ends], twig_size, rng, 1)
    return finish_tree(m, crown, cards, "twigs", rng, height, lod, dead=True)


def bare_ends(p, parent, r, children, least):
    """The ends of the branches as drawn (`branch_tubes` stops where they thin under `least`):
    each tip's nearest node drawn, once each."""
    ends = []
    for i in range(len(p)):
        if children[i] or parent[i] < 0:
            continue
        k = i
        while parent[k] >= 0 and r[k] < least:
            k = parent[k]
        if parent[k] >= 0 and k not in ends:
            ends.append(k)
    return ends


def dead_conifer(name, seed, lod, height, radius, trunk_radius, twig_size):
    """A conifer burnt: its trunk and stubs of its branches, a few charred twigs."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    crown = Crown([((0.0, height * 0.5, 0.0), (radius, height * 0.5, radius))])
    bark = bark_colour(height, 0.0, crown, rng.random(), open_sky=True)
    trunk = np.array([[0.0, y, 0.0] for y in np.linspace(-SINK, height, 12)])
    taper = lambda t: trunk_radius * (1.0 - min(max(t, 0.0), 1.0) * 0.8)
    tube(m.part("bark"), trunk, [taper((y + SINK) / (height + SINK)) for y in trunk[:, 1]], sides(lod)(trunk_radius), bark, (36.0, 48.0))
    tips, dirs = [], []
    y = 9.0
    k = 0
    while y < height - 4.0:
        t = y / height
        whorl = int(rng.integers(3, 6))
        for j in range(whorl):
            a = (j + rng.uniform(-0.3, 0.3)) / whorl * TAU + k
            out = unit(np.array([np.cos(a), -0.25, np.sin(a)]))
            full = radius * np.sin(np.pi * min(t, 1.0) ** 0.75)
            # Many broken off short, low down most.
            broken = rng.random() < 0.65 * (1.0 - t) + 0.15
            reach = full * (rng.uniform(0.2, 0.45) if broken else rng.uniform(0.7, 1.0))
            start = np.array([0.0, y, 0.0])
            r0 = max(0.42 * taper(t), 0.55)
            # (The coarsest level keeps every other long one, straight, their mass from afar.)
            droop = start + out * reach * 0.55 + np.array([0.0, -reach * 0.08, 0.0])
            end = start + out * reach + np.array([0.0, -reach * 0.2, 0.0])
            if reach > 2.0 and lod < 2:
                tube(m.part("bark"), [start, droop, end], [r0, max(0.6 * r0, 0.4), max(0.32, 0.4 * r0)], (5, 4)[lod], bark, (36.0, 48.0))
            elif reach > 2.0 and not broken and j % 2 == 0:
                tube(m.part("bark"), [start, end], [r0, max(0.7, 0.5 * r0)], 3, bark, (36.0, 48.0))
            if reach > 2.0 and not broken and j % 2 == 0:
                for f in (0.55, 0.85):
                    tips.append(start + out * reach * f + np.array([0.0, -reach * 0.2 * f, 0.0]))
                    dirs.append(out)
        y += rng.uniform(6.0, 10.0)
        k += 1
    cards = twig_cards(tips, dirs, twig_size, rng, 1)
    return finish_tree(m, crown, cards, "twigs", rng, height, lod, dead=True)


# ---------------------------------------------------------------------------------------------
# Rocks
# ---------------------------------------------------------------------------------------------


def icosphere(level):
    t = (1.0 + 5 ** 0.5) / 2.0
    v = [(-1, t, 0), (1, t, 0), (-1, -t, 0), (1, -t, 0), (0, -1, t), (0, 1, t), (0, -1, -t), (0, 1, -t), (t, 0, -1), (t, 0, 1), (-t, 0, -1), (-t, 0, 1)]
    f = [(0, 11, 5), (0, 5, 1), (0, 1, 7), (0, 7, 10), (0, 10, 11), (1, 5, 9), (5, 11, 4), (11, 10, 2), (10, 7, 6), (7, 1, 8),
         (3, 9, 4), (3, 4, 2), (3, 2, 6), (3, 6, 8), (3, 8, 9), (4, 9, 5), (2, 4, 11), (6, 2, 10), (8, 6, 7), (9, 8, 1)]
    verts = [unit(np.array(p, float)) for p in v]
    cache = {}

    def mid(a, b):
        key = (min(a, b), max(a, b))
        if key not in cache:
            verts.append(unit(verts[a] + verts[b]))
            cache[key] = len(verts) - 1
        return cache[key]

    for _ in range(level):
        nf = []
        for a, b, c in f:
            ab, bc, ca = mid(a, b), mid(b, c), mid(c, a)
            nf += [(a, ab, ca), (b, bc, ab), (c, ca, bc), (ab, bc, ca)]
        f = nf
    return np.array(verts), np.array(f)


def value_noise(p, seed):
    """Smooth 3D value noise (-1..1) at points `p` (n, 3)."""
    rng = np.random.default_rng(seed)
    table = rng.uniform(-1, 1, 4096)
    perm = rng.permutation(4096)
    i = np.floor(p).astype(np.int64)
    f = p - i
    u = f * f * (3 - 2 * f)

    def h(x, y, z):
        return table[perm[(perm[(perm[x % 4096] + y) % 4096] + z) % 4096]]

    out = 0.0
    for dx in (0, 1):
        for dy in (0, 1):
            for dz in (0, 1):
                w = (u[:, 0] if dx else 1 - u[:, 0]) * (u[:, 1] if dy else 1 - u[:, 1]) * (u[:, 2] if dz else 1 - u[:, 2])
                out = out + w * h(i[:, 0] + dx, i[:, 1] + dy, i[:, 2] + dz)
    return out


def fbm(p, seed, octaves=4):
    out, amp, freq = 0.0, 1.0, 1.0
    for o in range(octaves):
        out = out + amp * value_noise(p * freq, seed + o)
        amp *= 0.5
        freq *= 2.03
    return out


def rock(name, seed, lod, size, planes=14, sharp=1.0, rough=0.08, sink=0.12):
    """A rock: the faces of `planes` random planes round a sphere (stretched to `size`: width,
    height, depth), roughened; set into the ground by `sink` of its height."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    v, f = icosphere((3, 2, 1)[lod])
    normals = unit(rng.normal(0, 1, (planes, 3)) * np.array([1.0, 0.6, 1.0]))
    offsets = rng.uniform(0.72, 1.0, planes)
    # How far out each direction reaches before a plane cuts it: the polytope, rounded a little.
    cos = v @ normals.T
    reach = np.where(cos > 1e-3, offsets[None, :] / np.maximum(cos, 1e-3), 10.0)
    soft = -np.log(np.sum(np.exp(-reach * 12.0 * sharp), axis=1)) / (12.0 * sharp)
    rad = np.clip(soft, 0.5, 1.3)
    pts = v * rad[:, None]
    pts = pts + v * (fbm(pts * 2.2, seed + 10, 4) * rough)[:, None]
    pts *= np.array(size) * 0.5
    bottom = pts[:, 1].min()
    pts[:, 1] -= bottom + size[1] * sink
    # Smooth normals from the faces.
    nrm = np.zeros_like(pts)
    fn = np.cross(pts[f[:, 1]] - pts[f[:, 0]], pts[f[:, 2]] - pts[f[:, 0]])
    for k in range(3):
        np.add.at(nrm, f[:, k], fn)
    nrm = unit(nrm)
    # Open to the sky on top and out at the sides, dark underneath and in hollows.
    curv = (rad - rad.mean()) / max(rad.std(), 1e-6)
    ao = np.clip(0.55 + 0.35 * (nrm[:, 1] * 0.5 + 0.5) + 0.08 * curv, 0.2, 1.0) * (0.75 + 0.25 * smoothstep(0.0, size[1] * 0.3, pts[:, 1]))
    col = np.stack([ao, np.zeros(len(pts)), np.zeros(len(pts)), np.zeros(len(pts))], axis=1)
    m.part("rock").add(pts, nrm, np.zeros((len(pts), 2)), col, f)
    m.meta = {"height": float(pts[:, 1].max()), "width": float((size[0] + size[2]) * 0.5), "crown": [0.0, float(pts[:, 1].max() * 0.5), 0.0, float(size[0] * 0.5)]}
    m.meta["contact"] = contact(m)
    return m


def contact(m):
    """How far from its foot a rock meets the ground (its outline there): the game darkens the
    ground in a ring round that (the `contact` of its extras)."""
    pts = np.concatenate([np.concatenate(part.pos) for part in m.parts.values() if part.pos])
    top = max(m.meta.get("height", 1.0), 1.0)
    near = pts[(pts[:, 1] > -1.0) & (pts[:, 1] < max(2.0, top * 0.06))]
    if len(near) == 0:
        return 0.0
    return float(np.percentile(np.hypot(near[:, 0], near[:, 2]), 85))


def faceted(part, tris, col, gloss=0.0):
    """Triangles (n, 3, 3) with flat normals (each its own vertices): crystals and shards.
    `col(points)` gives each corner's occlusion; `gloss` (0..1) how much it shines, in COLOR_0's
    blue (for opaque layers)."""
    tris = np.asarray(tris, np.float64).reshape(-1, 3, 3)
    if len(tris) == 0:
        return
    n = unit(np.cross(tris[:, 1] - tris[:, 0], tris[:, 2] - tris[:, 0]))
    pts = tris.reshape(-1, 3)
    nrm = np.repeat(n, 3, axis=0)
    ao = col(pts)
    c = np.stack([ao, np.zeros(len(pts)), np.full(len(pts), gloss), np.zeros(len(pts))], axis=1)
    part.add(pts, nrm, np.zeros((len(pts), 2)), c, np.arange(len(pts)).reshape(-1, 3))


def prism(base, axis, length, radius, sides, tip, rng, squash=1.0, jag=0.0):
    """A prism along `axis` from `base` (sunk a little), `length` long with a pointed tip `tip`
    long: its triangles (outward). `squash` flattens it across; `jag` makes its tip ragged."""
    axis = unit(axis)
    u = perpendicular(axis, rng)
    v = np.cross(axis, u)
    a = np.linspace(0.0, TAU, sides + 1)[:-1] + rng.uniform(0, TAU)
    ring = lambda c, r: [c + (np.cos(t) * u * squash + np.sin(t) * v) * r * rng.uniform(0.9, 1.1) for t in a]
    b0 = ring(base - axis * 3.0, radius)
    b1 = ring(base + axis * (length - tip), radius * rng.uniform(0.85, 1.0))
    apex = base + axis * length + (u * rng.uniform(-1, 1) + v * rng.uniform(-1, 1)) * jag * radius
    tris = []
    for i in range(sides):
        j = (i + 1) % sides
        tris += [(b0[i], b0[j], b1[j]), (b0[i], b1[j], b1[i]), (b1[i], b1[j], apex)]
    return tris


def broken_prism(base, axis, length, radius, sides, tip, rng, squash=1.0, jag=0.0, breaks=2):
    """A blade of rock broken along its length: `breaks` + 1 pieces, each a little narrower than
    the one under it and knocked a little aside at the break (a step in, a ledge), its sides
    kinked between (a prism's flat sides read as plastic), a ragged point: its triangles."""
    axis = unit(axis)
    u = perpendicular(axis, rng)
    v = np.cross(axis, u)
    a = np.linspace(0.0, TAU, sides + 1)[:-1] + rng.uniform(0, TAU)
    shaft = length - tip

    def ring(h, r, offset, jitter):
        c = base + axis * h + offset
        return [c + (np.cos(t) * u * squash + np.sin(t) * v) * r * rng.uniform(1.0 - jitter, 1.0 + jitter) for t in a]

    r, offset = radius, np.zeros(3)
    rings = [ring(-3.0, r, offset, 0.04)]
    below = 0.0
    for b in sorted(rng.uniform(0.22, 0.82, breaks)):
        h = b * shaft
        rings.append(ring((below + h) * 0.5, r * 0.97, offset, 0.09))
        rings.append(ring(h, r * 0.95, offset, 0.05))
        r *= rng.uniform(0.76, 0.9)
        offset = offset + (u * rng.uniform(-1, 1) + v * rng.uniform(-1, 1)) * radius * 0.12
        rings.append(ring(h + radius * 0.2, r, offset, 0.05))
        below = h
    rings.append(ring(shaft, r * rng.uniform(0.85, 1.0), offset, 0.05))
    apex = base + axis * length + offset + (u * rng.uniform(-1, 1) + v * rng.uniform(-1, 1)) * jag * radius
    tris = []
    for k in range(len(rings) - 1):
        r0, r1 = rings[k], rings[k + 1]
        for i in range(sides):
            j = (i + 1) % sides
            tris += [(r0[i], r0[j], r1[j]), (r0[i], r1[j], r1[i])]
    for i in range(sides):
        tris.append((rings[-1][i], rings[-1][(i + 1) % sides], apex))
    return tris


def shard_cluster(name, seed, lod, height, spread, count, radius, sides=4, squash=0.5, tilt=0.25, layer="rock", gloss=0.0, jag=0.3, low=0.25, breaks=0):
    """A cluster of tall blades or crystals: the tallest in the middle, the others shorter
    round it, leaning out. (Slate's fog strata, crystal spires, ice.) With `breaks`, each blade
    is broken along its length (see `broken_prism`)."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    n = (count, max(count * 2 // 3, 3), max(count // 3, 2))[lod]
    tris = []
    for k in range(n):
        r = 0.0 if k == 0 else spread * rng.random() * 0.5
        a = rng.uniform(0, TAU)
        base = np.array([np.cos(a) * r, 0.0, np.sin(a) * r])
        out = np.array([np.cos(a), 0.0, np.sin(a)])
        axis = unit(np.array([0.0, 1.0, 0.0]) + out * tilt * (r / max(spread * 0.5, 1e-6)) + rng.normal(0, tilt * 0.3, 3))
        length = height * (1.0 if k == 0 else rng.uniform(low, 0.92) * (1.0 - 0.45 * r / max(spread * 0.5, 1e-6)))
        rad = radius * rng.uniform(0.8, 1.15) * (2.0 if k == 0 else 1.0)
        if breaks and lod < 2:
            tris += broken_prism(base, axis, length, rad, sides, length * rng.uniform(0.18, 0.35), rng, squash=squash, jag=jag, breaks=max(breaks - lod, 1))
        else:
            tris += prism(base, axis, length, rad, sides, length * rng.uniform(0.18, 0.35), rng, squash=squash, jag=jag)
    tris = np.array(tris)
    top = max(tris[..., 1].max(), 1.0)
    # (Broken rock darker at its foot, weathered paler towards its top.)
    floor = 0.42 if breaks else 0.55
    faceted(m.part(layer), tris, lambda p: np.clip(floor + (1.0 - floor) * np.clip(p[:, 1] / top, 0, 1) ** 0.6, 0, 1), gloss)
    lo, hi = tris.reshape(-1, 3).min(axis=0), tris.reshape(-1, 3).max(axis=0)
    m.meta = {"height": float(hi[1]), "width": float((hi[0] - lo[0] + hi[2] - lo[2]) * 0.5), "crown": [0.0, float(hi[1] * 0.5), 0.0, float(spread * 0.5)]}
    m.meta["contact"] = contact(m)
    return m


def crystal_spire(name, seed, lod, height, spread, plate):
    """A crystal spire: a cluster of dark crystals over a wide, cracked plate of crystal flat on
    the ground."""
    m = shard_cluster(name, seed, lod, height, spread, 9, spread * 0.13, sides=6, squash=1.0, tilt=0.35, layer="crystal", gloss=0.8, jag=0.05, low=0.3)
    rng = np.random.default_rng(seed + 7)
    # The plate: a ring of facets rising a little from its rim to the cluster.
    sides = (18, 12, 8)[lod]
    a = np.linspace(0.0, TAU, sides + 1)[:-1]
    reach = plate * 0.5 * (0.75 + 0.25 * rng.random(sides))
    rim = np.stack([np.cos(a) * reach, np.full(sides, -0.6), np.sin(a) * reach], axis=1)
    mid = np.stack([np.cos(a) * reach * 0.45, rng.uniform(1.0, 2.6, sides), np.sin(a) * reach * 0.45], axis=1)
    centre = np.array([0.0, 2.0, 0.0])
    tris = []
    for i in range(sides):
        j = (i + 1) % sides
        tris += [(rim[i], mid[j], rim[j]), (rim[i], mid[i], mid[j]), (mid[i], centre, mid[j])]
    faceted(m.part("rock"), tris, lambda p: np.full(len(p), 0.85), 0.25)
    m.meta["width"] = float(plate)
    m.meta["contact"] = float(plate * 0.42)
    return m


def spire(name, seed, lod, height, radius, ridges=7.0, strata=0.06, lean=0.0, flare=0.35):
    """A rock spire: a tall cone of rock, flared at its foot, ribbed by gullies and banded by
    strata, roughened. (Lava worlds' spires, wet deserts' pillars.)"""
    rng = np.random.default_rng(seed)
    m = Model(name)
    rings, sides = ((30, 26), (16, 14), (9, 8))[lod]
    ys = np.linspace(-SINK, height, rings)
    t = np.clip(ys / height, 0.0, 1.0)
    profile = radius * (1.0 - t) ** 0.75 * (1.0 + flare * (1.0 - t) ** 6) + radius * 0.06
    a = np.linspace(0.0, TAU, sides + 1)
    phase = rng.uniform(0, TAU)
    pos, nrm_out, aos = [], [], []
    for i, (y, r) in enumerate(zip(ys, profile)):
        d = np.stack([np.cos(a), np.zeros_like(a), np.sin(a)], axis=1)
        gully = 1.0 - 0.14 * np.abs(np.sin(a * ridges * 0.5 + phase + y * 0.02))
        # Strata: ledges where harder beds stand out, wandering round the spire.
        bed = np.sin(y * 0.21 + fbm(np.stack([np.cos(a) * 2.0, np.full_like(a, y * 0.01), np.sin(a) * 2.0], axis=1), seed + 5, 2) * 1.5)
        band = 1.0 + strata * smoothstep(0.55, 0.9, bed)
        p = d * (r * gully * band)[:, None] + np.array([lean * t[i] ** 2 * height, y, 0.0])
        p = p + d * (fbm(p * 0.04, seed + 3, 4) * r * 0.3)[:, None]
        pos.append(p)
        aos.append(0.55 + 0.45 * gully ** 6 * (0.75 + 0.25 * t[i]))
    pos = np.array(pos)
    apex = np.array([lean * height, height + radius * 0.08, 0.0])
    w = sides + 1
    tris = []
    for i in range(rings - 1):
        for j in range(sides):
            a0, a1, b0, b1 = i * w + j, i * w + j + 1, (i + 1) * w + j, (i + 1) * w + j + 1
            tris += [(a0, b1, a1), (a0, b0, b1)]
    flat = pos.reshape(-1, 3)
    apex_index = len(flat)
    flat = np.concatenate([flat, apex[None, :]])
    for j in range(sides):
        tris.append(((rings - 1) * w + j, apex_index, (rings - 1) * w + j + 1))
    tris = np.array(tris)
    nrm = np.zeros_like(flat)
    fn = np.cross(flat[tris[:, 1]] - flat[tris[:, 0]], flat[tris[:, 2]] - flat[tris[:, 0]])
    for k in range(3):
        np.add.at(nrm, tris[:, k], fn)
    nrm = unit(nrm)
    ao = np.concatenate([np.array(aos).reshape(-1), [1.0]])
    col = np.stack([ao, np.zeros(len(flat)), np.zeros(len(flat)), np.zeros(len(flat))], axis=1)
    m.part("rock").add(flat, nrm, np.zeros((len(flat), 2)), col, tris)
    # Rubble round its foot.
    if lod < 2:
        for k in range(5 if lod == 0 else 3):
            ang = rng.uniform(0, TAU)
            at = np.array([np.cos(ang), 0.0, np.sin(ang)]) * radius * rng.uniform(1.0, 1.5)
            s = radius * rng.uniform(0.18, 0.3)
            r = rock(name, seed + 20 + k, 2, (s * 2, s * 1.2, s * 1.8), planes=9, sharp=1.6, rough=0.08)
            p, n2, uv, c2, idx = r.parts["rock"].arrays()
            m.part("rock").add(p + at, n2, uv, c2, idx)
    m.meta = {"height": float(height), "width": float(radius * 2.0), "crown": [0.0, float(height * 0.4), 0.0, float(radius)]}
    m.meta["contact"] = contact(m)
    return m


def hoodoos(name, seed, lod, height, count, spread, row=1.0):
    """A wet desert's cluster of rock spires as TA paints them: `count` tall columns of banded
    rock standing close together, the tallest `height` in the middle, drawn to a blunt point at
    their tops as a pointed arch is (not cones, nor domes), their beds weathered into ledges and
    grooves round them (the grooves in the shade), a little rubble at their feet. With `row` under 1 they stand more in a row (east
    to west) than round each other."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    part = m.part("rock")
    rings, sides = ((34, 18), (18, 12), (10, 8))[lod]
    placed = []
    for k in range(count):
        tall = height * (1.0 if k == 0 else rng.uniform(0.35, 0.8))
        radius = tall * rng.uniform(0.14, 0.18) * (1.15 if k == 0 else 1.0)
        if k == 0:
            at = np.zeros(3)
        else:
            # Beside the ones standing, touching them.
            for _ in range(20):
                a = rng.uniform(0, TAU) if row >= 1.0 else rng.choice([0.0, np.pi]) + rng.normal(0.0, 0.45)
                near = placed[rng.integers(len(placed))]
                at = near[0] + np.array([np.cos(a), 0.0, np.sin(a) * row]) * (near[1] + radius) * rng.uniform(0.7, 0.9)
                if np.hypot(at[0], at[2] / row) < spread * 0.5:
                    break
        placed.append((at, radius))
        lean = rng.normal(0, 0.03, 2)
        ys = np.linspace(-SINK, tall, rings)
        t = np.clip(ys / tall, 0.0, 1.0)
        # A column swelling a little to its foot, its top drawn in to a point as a pointed arch
        # is (an ogive: an arc of a circle wider than the column, tangent to its side), over a
        # length of two of its radii or so.
        # (Of its own random numbers: the cluster stands as it was laid out.)
        tip = min(radius * np.random.default_rng(seed * 31 + k).uniform(1.9, 2.4), tall * 0.45) / radius
        arc = (1.0 + tip * tip) * 0.5
        u = np.clip((t - (1.0 - tip * radius / tall)) / (tip * radius / tall), 0.0, 1.0) * tip
        profile = radius * (1.0 + 0.3 * (1.0 - t) ** 5) * (np.sqrt(np.maximum(arc * arc - u * u, 0.0)) - (arc - 1.0)) + radius * 0.03
        a = np.linspace(0.0, TAU, sides + 1)
        phase = rng.uniform(0, TAU)
        beds = rng.uniform(0.35, 0.55)
        pos, aos = [], []
        for i, (y, r) in enumerate(zip(ys, profile)):
            d = np.stack([np.cos(a), np.zeros_like(a), np.sin(a)], axis=1)
            gully = 1.0 - 0.08 * np.abs(np.sin(a * 2.5 + phase + y * 0.015))
            # Beds a few units thick, of uneven thickness, the harder ones standing out as
            # ledges, wandering round it.
            wander = fbm(np.stack([np.cos(a) * 1.5, np.full_like(a, y * 0.03), np.sin(a) * 1.5], axis=1), seed + 5 + k, 3)
            bed = np.sin(y * beds + k + wander * 3.0 + 1.5 * np.sin(y * beds * 0.37 + phase))
            ledge = smoothstep(0.2, 0.8, bed)
            p = d * (r * gully * (1.0 + 0.07 * ledge))[:, None] + np.array([lean[0] * t[i] * tall, y, lean[1] * t[i] * tall]) + at
            p = p + d * (fbm(p * 0.05, seed + 3 + k, 3) * r * 0.12)[:, None]
            pos.append(p)
            aos.append(np.clip(0.4 + 0.6 * ledge * gully ** 4, 0.0, 1.0) * (0.7 + 0.3 * t[i]))
        pos = np.array(pos)
        apex = np.array([at[0] + lean[0] * tall, tall + radius * 0.05, at[2] + lean[1] * tall])
        w = sides + 1
        tris = []
        for i in range(rings - 1):
            for j in range(sides):
                a0, a1, b0, b1 = i * w + j, i * w + j + 1, (i + 1) * w + j, (i + 1) * w + j + 1
                tris += [(a0, b1, a1), (a0, b0, b1)]
        flat = pos.reshape(-1, 3)
        apex_index = len(flat)
        flat = np.concatenate([flat, apex[None, :]])
        for j in range(sides):
            tris.append(((rings - 1) * w + j, apex_index, (rings - 1) * w + j + 1))
        tris = np.array(tris)
        nrm = np.zeros_like(flat)
        fn = np.cross(flat[tris[:, 1]] - flat[tris[:, 0]], flat[tris[:, 2]] - flat[tris[:, 0]])
        for c in range(3):
            np.add.at(nrm, tris[:, c], fn)
        nrm = unit(nrm)
        ao = np.concatenate([np.array(aos).reshape(-1), [0.95]])
        part.add(flat, nrm, np.zeros((len(flat), 2)), np.stack([ao, np.zeros(len(flat)), np.zeros(len(flat)), np.zeros(len(flat))], axis=1), tris)
    if lod < 2:
        for k in range(6 if lod == 0 else 3):
            ang = rng.uniform(0, TAU)
            base, radius = placed[rng.integers(len(placed))]
            at = base + np.array([np.cos(ang), 0.0, np.sin(ang)]) * radius * rng.uniform(1.1, 1.5)
            s = radius * rng.uniform(0.2, 0.35)
            r = rock(name, seed + 20 + k, 2, (s * 2, s * 1.2, s * 1.8), planes=9, sharp=1.6, rough=0.08)
            p, n2, uv, c2, idx = r.parts["rock"].arrays()
            part.add(p + at, n2, uv, c2, idx)
    pts = np.concatenate(part.pos)
    lo, hi = pts.min(axis=0), pts.max(axis=0)
    # (Its width east to west: the game stands it as painted, unturned.)
    m.meta = {"height": float(hi[1]), "width": float(hi[0] - lo[0]), "crown": [0.0, float(height * 0.4), 0.0, float(placed[0][1])]}
    m.meta["contact"] = contact(m)
    return m


def blob(part, centre, radii, level, seed, bumps=0.15, bend=0.0, emit=0.0, ao_floor=0.55, uv_scale=2.0):
    """A lumpy ball (a bag, a bulb, a mound) on `part`."""
    v, f = icosphere(level)
    pts = v * (1.0 + bumps * fbm(v * 2.2, seed, 3))[:, None] * np.asarray(radii, float) + np.asarray(centre, float)
    nrm = np.zeros_like(pts)
    fn = np.cross(pts[f[:, 1]] - pts[f[:, 0]], pts[f[:, 2]] - pts[f[:, 0]])
    for k in range(3):
        np.add.at(nrm, f[:, k], fn)
    nrm = unit(nrm)
    ao = ao_floor + (1.0 - ao_floor) * smoothstep(-0.6, 0.7, nrm[:, 1])
    uv = np.stack([np.arctan2(v[:, 2], v[:, 0]) / TAU * uv_scale * 2, v[:, 1] * uv_scale], axis=1)
    col = np.stack([ao, np.full(len(pts), bend), np.zeros(len(pts)), np.full(len(pts), emit)], axis=1)
    part.add(pts, nrm, uv, col, f)


# The gasbag layer's bands in v (see textures.py `gasbag`).
GASBAG_SKIN, GASBAG_VINE = 0.85, 0.95


def balloon(part, centre, radius, sectors, rings, seed, puff=0.02, col=None):
    """A gas bag: a sphere by longitude and latitude (u round it once, a seam column of its own so
    the texture closes), narrowing to a neck at its foot, a little puffed between eight gores,
    lumped only gently (no noise as fine as its facets, which would kink its outline). Its
    texture takes the gasbag layer's skin band; `col(points)` its COLOR_0."""
    rng = np.random.default_rng(seed)
    centre = np.asarray(centre, np.float64)
    phase = rng.uniform(0, TAU, 3)
    pos, nrm, uv = [], [], []
    for i in range(rings + 1):
        th = np.pi * i / rings
        for j in range(sectors + 1):
            ph = TAU * j / sectors
            d = np.array([np.sin(th) * np.cos(ph), np.cos(th), np.sin(th) * np.sin(ph)])
            r = radius * (1.0 - 0.22 * smoothstep(0.62 * np.pi, np.pi, th))
            r *= 1.0 + puff * abs(np.sin(ph * 4.0)) ** 0.5 * np.sin(th)
            r *= 1.0 + 0.035 * np.sin(1.3 * ph + phase[0]) * np.sin(th) + 0.02 * np.sin(2.0 * th + phase[1])
            pos.append(centre + d * np.array([r, r * 1.02, r]))
            nrm.append(d)
            uv.append((j / sectors, GASBAG_SKIN * i / rings))
    pos, nrm = np.array(pos), np.array(nrm)
    tris = []
    w = sectors + 1
    for i in range(rings):
        for j in range(sectors):
            a, b, c, d = i * w + j, i * w + j + 1, (i + 1) * w + j + 1, (i + 1) * w + j
            tris += [(a, c, b), (a, d, c)]
    cols = col(pos) if col else np.stack([0.6 + 0.4 * (nrm[:, 1] * 0.5 + 0.5), np.ones(len(pos)), np.zeros(len(pos)), np.zeros(len(pos))], axis=1)
    part.add(pos, nrm, np.array(uv), cols, tris)


def vine_uv(part, start):
    """Lays the tubes added to `part` from its `start`th on the gasbag layer's vine band: the
    fibres along the tube (u, repeating every 30 units), round it across the band (v)."""
    for k in range(start, len(part.uv)):
        uv = part.uv[k]
        part.uv[k] = np.stack([uv[:, 1] / 30.0, GASBAG_SKIN + 0.005 + (GASBAG_VINE - GASBAG_SKIN - 0.01) * np.clip(uv[:, 0], 0.0, 1.0)], axis=1)


def band_uv(part, start, v0, v1):
    """Lays the blobs added to `part` from its `start`th on the band `v0`..`v1` of the gasbag layer:
    round them in u as they were, up them across the band."""
    for k in range(start, len(part.uv)):
        y = part.pos[k][:, 1]
        t = (y - y.min()) / max(np.ptp(y), 1e-6)
        part.uv[k] = np.stack([part.uv[k][:, 0], v0 + 0.005 + (v1 - v0 - 0.01) * t], axis=1)


def gasbag(name, seed, lod, bag, neck, lean):
    """An acid world's gasbag as TA paints it: a round bag of gas in yellow-olive camouflage,
    gores meeting at a tie on top, floating a little to one side on a braided, knotted tether
    about a sixth of its width, rooted in a pod. `neck`: the tether's length in bag widths (TA's
    five paintings differ in it, not in the bag)."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    part = m.part("gasbag")
    length = neck * 2.0 * bag
    ry = bag * 1.02
    centre = np.array([-lean * length, length + ry, 0.0])
    height = centre[1] + ry + bag * 0.12
    bend = lambda p: np.clip(p[:, 1] / height, 0, 1) ** 1.2
    def colour(p, ao=None):
        a = np.full(len(p), 0.8) if ao is None else ao
        return np.stack([a, bend(p), np.zeros(len(p)), np.zeros(len(p))], axis=1)
    sectors, rings = ((40, 28), (24, 16), (12, 8))[lod]
    balloon(part, centre, bag, sectors, rings, seed, puff=0.02 if lod < 2 else 0.0,
            col=lambda p: colour(p, 0.6 + 0.4 * smoothstep(-bag, bag, p[:, 1] - centre[1])))
    # The tether: a bow from the foot to the bag's neck, three strands braided round it.
    foot = np.array([0.0, -SINK * 0.5, 0.0])
    top = centre - np.array([0.0, ry * 0.72, 0.0])
    t = np.linspace(0.0, 1.0, (28, 18, 8)[lod])
    bow = (foot + top) * 0.5 + np.array([-lean * length * 0.3, 0.0, rng.uniform(-2.0, 2.0)])
    axis = ((1 - t) ** 2)[:, None] * foot + (2 * (1 - t) * t)[:, None] * bow + (t * t)[:, None] * top
    rs = 0.075 * bag
    tang = unit(np.gradient(axis, axis=0))
    side = unit(np.cross(tang, np.array([0.0, 0.0, 1.0])) + 1e-6)
    other = np.cross(tang, side)
    along = np.concatenate([[0.0], np.cumsum(np.linalg.norm(np.diff(axis, axis=0), axis=1))])
    if lod < 2:
        for k in range(3):
            ang = along / 22.0 * TAU + k * TAU / 3.0
            pts = axis + (np.cos(ang)[:, None] * side + np.sin(ang)[:, None] * other) * rs * 1.05
            start = len(part.uv)
            tube(part, pts, np.linspace(rs, rs * 0.8, len(pts)), (6, 5)[lod], lambda p, r: colour(p, np.full(len(p), 0.7)), (1e9, 1.0))
            vine_uv(part, start)
        for at in (0.3, 0.75):
            p = axis[int(at * (len(axis) - 1))]
            start = len(part.uv)
            blob(part, tuple(p), (rs * 1.9, rs * 1.4, rs * 1.9), 1, seed + int(at * 10), bumps=0.1, uv_scale=1.0)
            band_uv(part, start, GASBAG_SKIN, GASBAG_VINE)
    else:
        start = len(part.uv)
        tube(part, axis, np.full(len(axis), rs * 2.2), 4, lambda p, r: colour(p, np.full(len(p), 0.7)), (1e9, 1.0))
        vine_uv(part, start)
    # The tie on top, and the pod at the foot.
    if lod < 2:
        start = len(part.uv)
        blob(part, tuple(centre + np.array([0.0, ry * 1.02, 0.0])), (bag * 0.12, bag * 0.08, bag * 0.12), 1, seed + 7, bumps=0.05, uv_scale=1.0)
        band_uv(part, start, GASBAG_SKIN, GASBAG_VINE)
    start = len(part.uv)
    blob(part, (0.0, bag * 0.08, 0.0), (bag * 0.28, bag * 0.36, bag * 0.28), (2, 1, 1)[lod], seed + 9, bumps=0.12, uv_scale=1.0)
    band_uv(part, start, GASBAG_VINE, 1.0)
    m.meta = {"height": float(height), "width": float(bag * 2.0), "crown": [float(centre[0]), float(centre[1]), 0.0, float(bag)]}
    return m


def energy_node(name, seed, lod, size, tubes):
    """A lava world's energy node as TA paints it: a low mound of rock with fingers of crystal
    standing close together out of it, leaning apart, rounded off at their ends, each with a
    pointed gem at its tip glowing with the light within, which shows through the finger too, the
    more towards the gem."""
    rng = np.random.default_rng(seed)
    m = rock(name, seed, lod, (size, size * 0.42, size * 0.85), planes=13, sharp=0.9, rough=0.12, sink=0.4)
    part = m.part("crystal")
    sides, shaft, cap = (12, 8, 6)[lod], (6, 4, 2)[lod], (5, 3, 2)[lod]
    top = 1.0
    for k in range(tubes):
        a = (k + rng.uniform(-0.3, 0.3)) / tubes * TAU
        out = np.array([np.cos(a), 0.0, np.sin(a)])
        base = out * size * rng.uniform(0.04, 0.1) + np.array([0.0, size * 0.08, 0.0])
        axis = unit(np.array([0.0, 1.0, 0.0]) + out * rng.uniform(0.15, 0.32))
        length = size * rng.uniform(0.62, 0.85)
        r = size * rng.uniform(0.11, 0.135)
        # Its profile: a stalk swelling a little above its middle, then a dome; the rings from
        # the foot (in the rock) to the end.
        s = np.linspace(0.0, 1.0, shaft + 1)
        along = list(-3.0 + s * (length - r + 3.0))
        radii = list(r * (0.9 + 0.12 * np.sin(np.pi * s * 0.8)))
        for t in np.linspace(0.0, np.pi * 0.5, cap + 1)[1:]:
            along.append(length - r + np.sin(t) * r * 0.9)
            radii.append(max(radii[shaft] * np.cos(t), r * 0.12))
        along, radii = np.array(along), np.array(radii)
        pts = base[None, :] + axis[None, :] * along[:, None]
        top = max(top, float(pts[:, 1].max() + r * 0.5))

        # COLOR_0: darker where it comes out of the rock; cloudy (blue: how clear: little, the
        # dark olive TA paints, not lit through pale by the sun); the light within (alpha, under
        # 1 on a finger: see the shader's `crystal`) rising to its end.
        def finger(p, rr, along=along, length=length):
            h = np.clip(along / length, 0.0, 1.0)
            return np.stack([0.55 + 0.45 * h, np.zeros(len(p)), np.full(len(p), 0.1), 0.7 * smoothstep(0.3, 1.0, h) ** 2], axis=1)

        tube(part, pts, radii, sides, finger, (24.0, 24.0))
        # The gem: a pointed crystal set into the dome, wholly alight (alpha 1).
        gem = prism(base + axis * (length - r * 0.45), axis, r * 0.85, r * 0.3, 6, r * 0.45, rng, jag=0.1)
        faceted(part, gem, lambda p: np.ones(len(p)), 0.8)
        part.col[-1][:, 3] = 1.0
    m.meta["height"] = float(max(m.meta["height"], top))
    return m


def tendrils(name, seed, lod, width, arms):
    """A Mars plant as TA paints it: a knot of two or three small bulbs, thin tendrils winding
    flat over the ground from it, every other one curling off a branch of its own. Every choice
    is drawn first, so each level of detail follows the same paths."""
    rng = np.random.default_rng(seed)
    m = Model(name)
    reach = width * 0.5
    node = width * 0.032
    plans = []
    for k in range(arms):
        plans.append({
            "a0": (k + rng.uniform(-0.35, 0.35)) / arms * TAU,
            "length": reach * rng.uniform(1.2, 1.7),
            "amp": rng.uniform(0.9, 1.3), "f1": rng.uniform(1.3, 2.3), "f2": rng.uniform(3.0, 5.0),
            "p1": rng.uniform(0, TAU), "p2": rng.uniform(0, TAU), "sweep": rng.uniform(-0.9, 0.9),
            "curl": k % 2 == 0, "curl_at": rng.uniform(0.3, 0.6), "curl_side": rng.choice([-1.0, 1.0]),
        })
    bulbs = [(np.array([0.0, 0.0, 0.0]), 1.0), (np.array([1.25, 0.0, 0.4]), 0.78), (np.array([-0.5, 0.0, 1.2]), 0.6)]
    def flat(p, r):
        # Occlusion: darker at the knot, and towards the tips as TA paints them (olive brown);
        # no wind; a waxy sheen.
        out = np.linalg.norm(p[:, [0, 2]], axis=1) / reach
        ao = 0.72 + 0.2 * smoothstep(0.0, 0.3, out) - 0.32 * smoothstep(0.55, 1.5, out)
        return np.stack([ao, np.zeros(len(p)), np.full(len(p), 0.3), np.zeros(len(p))], axis=1)
    steps, rings = (40, 24, 12)[lod], (5, 4, 3)[lod]
    for plan in plans:
        s_ = np.linspace(0.0, 1.0, steps)
        heading = plan["a0"] + plan["amp"] * np.sin(TAU * plan["f1"] * s_ + plan["p1"]) * smoothstep(0.0, 0.15, s_) \
            + 0.45 * np.sin(TAU * plan["f2"] * s_ + plan["p2"]) * smoothstep(0.0, 0.3, s_) + plan["sweep"] * s_ \
            + 0.22 * np.sin(TAU * plan["f2"] * 2.3 * s_ + plan["p1"] * 1.7) * smoothstep(0.1, 0.4, s_)
        ds = plan["length"] / (steps - 1)
        start = np.array([np.cos(plan["a0"]), np.sin(plan["a0"])]) * node * 0.9
        xz = start + np.concatenate([[[0.0, 0.0]], np.cumsum(np.stack([np.cos(heading[:-1]), np.sin(heading[:-1])], axis=1) * ds, axis=0)])
        radius = width * (0.002 + 0.0075 * (1.0 - s_) ** 0.8)
        # (Lifting off the ground here and there over a stone, as a vine does: shade under it.)
        arch = np.maximum(np.sin(TAU * plan["f1"] * 1.6 * s_ + plan["p2"]), 0.0) ** 3 * radius * 1.6 * smoothstep(0.15, 0.35, s_)
        y = 0.75 * radius + 0.55 * node * (1.0 - smoothstep(0.0, 0.1, s_)) + arch
        pts = np.stack([xz[:, 0], y, xz[:, 1]], axis=1)
        tube(m.part("tendril"), pts, radius, rings, flat, (10.0, 14.0))
        if plan["curl"] and lod < 2:
            j = int(plan["curl_at"] * (steps - 1))
            n_c = steps // 2
            t = np.linspace(0.0, 1.0, n_c)
            h = heading[j] + plan["curl_side"] * (0.9 + 4.2 * t ** 1.4)
            dc = reach * 0.28 / (n_c - 1)
            cxz = xz[j] + np.concatenate([[[0.0, 0.0]], np.cumsum(np.stack([np.cos(h[:-1]), np.sin(h[:-1])], axis=1) * dc, axis=0)])
            cr = np.maximum(radius[j] * 0.65 * (1.0 - t) + width * 0.0016 * t, width * 0.0016)
            tube(m.part("tendril"), np.stack([cxz[:, 0], 0.75 * cr, cxz[:, 1]], axis=1), cr, max(rings - 1, 3), flat, (10.0, 14.0))
    # The bulbs: pale as TA paints them (open to the sky: the tendrils are darker).
    for centre, size in bulbs[: (3, 2, 2)[lod]]:
        r = node * size
        blob(m.part("tendril"), tuple(centre * node + np.array([0.0, 0.7 * r * 0.85, 0.0])), (r, r * 0.85, r), (2, 1, 0)[lod], seed, bumps=0.12, ao_floor=0.85)
    m.meta = {"height": float(node * 1.6), "width": float(width), "crown": [0.0, float(node), 0.0, float(width * 0.08)]}
    return m


# ---------------------------------------------------------------------------------------------
# Vehicles (TA's urban worlds' cars and trucks, which TA painted from above)
# ---------------------------------------------------------------------------------------------


def sgnpow(t, e):
    return np.sign(t) * np.abs(t) ** e


def superquadric(part, centre, radii, e_v, e_h, rings, sectors, col, keep=None):
    """A rounded box on `part`: a superquadric round `centre` (`radii` along x, y and z), its
    sides the flatter and its edges the sharper the smaller `e_v` (up and down) and `e_h`
    (round), 1 an ellipsoid. `keep(points)` (optional) keeps the triangles whose corners it
    passes (a dome: its upper half); `col(points, normals)` gives COLOR_0. No texture
    coordinates (its layer is laid on along the world's axes)."""
    th = np.linspace(-np.pi / 2, np.pi / 2, rings + 1)
    ph = np.linspace(-np.pi, np.pi, sectors + 1)[:-1]
    T, P = np.meshgrid(th, ph, indexing="ij")
    ct, st = np.cos(T), np.sin(T)
    pts = np.stack([radii[0] * sgnpow(ct, e_v) * sgnpow(np.cos(P), e_h), radii[1] * sgnpow(st, e_v), radii[2] * sgnpow(ct, e_v) * sgnpow(np.sin(P), e_h)], axis=-1).reshape(-1, 3) + np.asarray(centre, float)
    w = sectors
    tris = []
    for i in range(rings):
        for j in range(w):
            a, b, c, d = i * w + j, i * w + (j + 1) % w, (i + 1) * w + (j + 1) % w, (i + 1) * w + j
            tris += [(a, c, b), (a, d, c)]
    tris = np.array(tris)
    if keep is not None:
        ok = keep(pts)
        tris = tris[ok[tris].all(axis=1)]
    nrm = np.zeros_like(pts)
    fn = np.cross(pts[tris[:, 1]] - pts[tris[:, 0]], pts[tris[:, 2]] - pts[tris[:, 0]])
    for k in range(3):
        np.add.at(nrm, tris[:, k], fn)
    # (The poles' rings collapse to a point: their normals straight up and down.)
    nrm = unit(nrm + np.where(np.abs(pts - centre)[:, 1:2] >= radii[1] * 0.999, np.array([0.0, 1.0, 0.0]) * np.sign(pts[:, 1:2] - centre[1]), 0.0))
    part.add(pts, nrm, np.zeros((len(pts), 2)), col(pts, nrm), tris)


def wheel(part, centre, radius, width, sides, col):
    """A wheel on its side (its axle along z), a little rounded at its rims."""
    a = np.linspace(0.0, TAU, sides + 1)[:-1]
    rings = [(-0.5, 0.86), (-0.38, 1.0), (0.38, 1.0), (0.5, 0.86)]
    pts, nrm = [], []
    for z, r in rings:
        for t in a:
            d = np.array([np.cos(t), np.sin(t), 0.0])
            pts.append(np.asarray(centre, float) + d * radius * r + np.array([0.0, 0.0, z * width]))
            nrm.append(unit(d + np.array([0.0, 0.0, np.sign(z) * (1.0 - r) * 4.0])))
    tris = []
    for k in range(len(rings) - 1):
        for i in range(sides):
            j = (i + 1) % sides
            p, q = k * sides, (k + 1) * sides
            tris += [(p + i, p + j, q + j), (p + i, q + j, q + i)]
    # Its hubs, flat.
    base = len(pts)
    for z, side in ((-0.5, -1.0), (0.5, 1.0)):
        pts.append(np.asarray(centre, float) + np.array([0.0, 0.0, z * width]))
        nrm.append(np.array([0.0, 0.0, side]))
    for i in range(sides):
        j = (i + 1) % sides
        tris += [(base, j, i), (base + 1, 3 * sides + i, 3 * sides + j)]
    pts, nrm = np.array(pts), np.array(nrm)
    part.add(pts, nrm, np.zeros((len(pts), 2)), col(pts, nrm), tris)


def vehicle_colour(top, gloss, floor=0.45):
    """COLOR_0 for a vehicle's paint: darker underneath and at its foot, open to the sky on
    top; no wind; `gloss` (its lacquer, COLOR_0's blue)."""
    def f(p, n):
        ao = np.clip(floor + (1.0 - floor) * (0.55 + 0.45 * n[:, 1]) * (0.7 + 0.3 * smoothstep(0.0, top * 0.5, p[:, 1])), 0, 1)
        return np.stack([ao, np.zeros(len(p)), np.full(len(p), gloss), np.zeros(len(p))], axis=1)
    return f


def car(name, seed, lod, length, width, height, dome=0.55, nose=0.0):
    """A car of TA's urban worlds: a rounded pod on four small wheels, a glass canopy over its
    middle (`dome` of its length, pushed forward by `nose`); its front along +x."""
    m = Model(name)
    rings, sectors = ((14, 28), (9, 18), (6, 12))[lod]
    clear = height * 0.2
    body_h = height * 0.52
    hull = m.part("hull")
    superquadric(hull, (0.0, clear + body_h * 0.5, 0.0), (length * 0.5, body_h * 0.5, width * 0.5), 0.45, 0.42, rings, sectors, vehicle_colour(height, 0.22))
    # The canopy: the upper half of an ellipsoid, on the glass.
    glass = m.part("crystal")
    top = clear + body_h * 0.82
    superquadric(glass, (length * nose, top, 0.0), (length * dome * 0.5, height - top, width * 0.36), 0.85, 0.7, rings, sectors,
                 lambda p, n: np.stack([np.clip(0.75 + 0.25 * n[:, 1], 0, 1), np.zeros(len(p)), np.full(len(p), 0.8), np.zeros(len(p))], axis=1),
                 keep=lambda p: p[:, 1] >= top - 1e-6)
    if lod < 2:
        r = height * 0.19
        for sx in (-0.31, 0.31):
            for sz in (-1.0, 1.0):
                wheel(hull, (length * sx, r, sz * width * 0.43), r, width * 0.13, (14, 10)[lod], lambda p, n: np.stack([np.full(len(p), 0.22), np.zeros(len(p)), np.full(len(p), 0.1), np.zeros(len(p))], axis=1))
    m.meta = {"height": float(height), "width": float(max(length, width)), "crown": [0.0, float(height * 0.5), 0.0, float(width * 0.5)],
              "length": float(length), "breadth": float(width), "contact": float(width * 0.5)}
    return m


def truck(name, seed, lod, length, width, height):
    """A truck of TA's urban worlds: a rounded cab at its front (+x), its windscreen glass, and
    behind it a great rounded tank on three axles of small wheels."""
    m = Model(name)
    rings, sectors = ((14, 28), (9, 18), (6, 12))[lod]
    hull = m.part("hull")
    clear = height * 0.14
    cab_l = length * 0.24
    cab_x = length * 0.5 - cab_l * 0.5
    cab_h = height * 0.62
    superquadric(hull, (cab_x, clear + cab_h * 0.5, 0.0), (cab_l * 0.5, cab_h * 0.5, width * 0.4), 0.45, 0.45, rings, sectors, vehicle_colour(height, 0.22))
    # Its windscreen: a band of glass round the cab's front, high up.
    glass = m.part("crystal")
    wy = clear + cab_h * 0.72
    superquadric(glass, (cab_x + cab_l * 0.08, wy, 0.0), (cab_l * 0.45, cab_h * 0.2, width * 0.37), 0.45, 0.45, rings, sectors,
                 lambda p, n: np.stack([np.full(len(p), 0.8), np.zeros(len(p)), np.full(len(p), 0.8), np.zeros(len(p))], axis=1),
                 keep=lambda p: p[:, 0] >= cab_x + cab_l * 0.05)
    # The tank behind, and a coupling between.
    tank_l = length - cab_l - length * 0.03
    tank_x = -length * 0.5 + tank_l * 0.5
    tank_h = height * 0.8
    superquadric(hull, (tank_x, clear + tank_h * 0.5, 0.0), (tank_l * 0.5, tank_h * 0.5, width * 0.48), 0.6, 0.5, rings, sectors, vehicle_colour(height, 0.18))
    superquadric(hull, (length * 0.5 - cab_l - length * 0.01, clear + height * 0.18, 0.0), (length * 0.04, height * 0.1, width * 0.18), 0.5, 0.5, 4, 8, vehicle_colour(height, 0.2))
    if lod < 2:
        r = height * 0.13
        for sx in (cab_x / length, -0.08, -0.34):
            for sz in (-1.0, 1.0):
                wheel(hull, (length * sx, r, sz * width * 0.44), r, width * 0.1, (14, 10)[lod], lambda p, n: np.stack([np.full(len(p), 0.22), np.zeros(len(p)), np.full(len(p), 0.1), np.zeros(len(p))], axis=1))
    m.meta = {"height": float(height), "width": float(max(length, width)), "crown": [0.0, float(height * 0.5), 0.0, float(width * 0.5)],
              "length": float(length), "breadth": float(width), "contact": float(width * 0.5)}
    return m


# ---------------------------------------------------------------------------------------------
# Pipework and road barriers (TA's metal worlds' pipes and urban worlds' barriers, which TA
# painted from above): pieces the game lays along the lines it traces through each painting
# (`apps/ta_game/src/feature_strokes.rs`), each stretched to its line's length and width.
# ---------------------------------------------------------------------------------------------


def lathe(part, profile, sides, col, axis_y=1.0):
    """A solid of revolution about a line along x at height `axis_y`: `profile` (x, radius) from
    one end to the other, each of its steps a band of its own (hard edges between them, as
    turned metal has; round the axis the normals are smooth). No texture coordinates (its layer
    is laid on along the world's axes)."""
    a = np.linspace(0.0, TAU, sides + 1)[:-1]
    round_ = np.stack([np.cos(a), np.sin(a)], axis=1)
    pos, nrm, tris = [], [], []
    for (x0, r0), (x1, r1) in zip(profile[:-1], profile[1:]):
        dx, dr = x1 - x0, r1 - r0
        length = np.hypot(dx, dr)
        if length < 1e-9:
            continue
        # The band's outward normal in the profile's plane (along x, outwards).
        nx, nr = -dr / length, dx / length
        base = len(pos)
        for x, r in ((x0, r0), (x1, r1)):
            for c, s in round_:
                pos.append((x, axis_y + c * r, s * r))
                nrm.append((nx, c * nr, s * nr))
        for j in range(sides):
            k = (j + 1) % sides
            a0, b0, a1, b1 = base + j, base + k, base + sides + j, base + sides + k
            tris += [(a0, b0, b1), (a0, b1, a1)]
    pos, nrm = np.array(pos, np.float64), unit(np.array(nrm, np.float64))
    part.add(pos, nrm, np.zeros((len(pos), 2)), col(pos, nrm), tris)


def pipe_colour(gloss):
    """COLOR_0 for a pipe lying on the ground (its axis at height 1): its underside, towards the
    ground, in the shade; no wind; `gloss` its sheen."""
    def f(p, n):
        ao = np.clip(0.55 + 0.45 * smoothstep(-0.9, 0.5, n[:, 1]), 0, 1)
        return np.stack([ao, np.zeros(len(p)), np.full(len(p), gloss), np.zeros(len(p))], axis=1)
    return f


PIPE_SIDES = (18, 12, 7)


def pipe_run(name, seed, lod):
    """A straight length of pipe: radius 1, its axis along x at height 1 (lying on the ground),
    from x -0.5 to 0.5 (the game stretches it along its run and to its painted width). Its ends
    are open: joints, collars and caps close them."""
    m = Model(name)
    lathe(m.part("metal"), [(-0.5, 1.0), (0.5, 1.0)], PIPE_SIDES[lod], pipe_colour(0.28))
    m.meta = {"height": 2.0, "width": 2.0, "crown": [0.0, 1.0, 0.0, 1.0], "length": 1.0, "breadth": 2.0, "contact": 1.0}
    return m


def pipe_joint(name, seed, lod):
    """A ball where lengths of pipe meet (radius 1, the pipe's own: it fills the outside of a
    bend, and where pipes cross or branch it joins them)."""
    m = Model(name)
    rings, sectors = ((10, PIPE_SIDES[0]), (7, PIPE_SIDES[1]), (5, PIPE_SIDES[2]))[lod]
    superquadric(m.part("metal"), (0.0, 1.0, 0.0), (1.0, 1.0, 1.0), 1.0, 1.0, rings, sectors, pipe_colour(0.28))
    m.meta = {"height": 2.0, "width": 2.0, "crown": [0.0, 1.0, 0.0, 1.0]}
    return m


def pipe_collar(name, seed, lod):
    """A bolted joint along a pipe: two flanges pressed together, a little proud of the pipe
    (radius 1, its axis at height 1, centred on x 0)."""
    m = Model(name)
    profile = [(-0.36, 0.97), (-0.3, 1.22), (-0.27, 1.28), (-0.05, 1.28), (-0.02, 1.2), (0.02, 1.2), (0.05, 1.28), (0.27, 1.28), (0.3, 1.22), (0.36, 0.97)]
    lathe(m.part("metal"), profile, PIPE_SIDES[lod], pipe_colour(0.22))
    m.meta = {"height": 2.28, "width": 2.56, "crown": [0.0, 1.0, 0.0, 1.28]}
    return m


def pipe_cap(name, seed, lod):
    """A pipe's end: a flange and a domed cap, from x 0 (where the pipe's open end meets it) to
    0.42 (radius 1, its axis at height 1)."""
    m = Model(name)
    dome = [(0.1 + 0.32 * np.sin(t), 0.92 * np.cos(t)) for t in np.linspace(0.0, np.pi / 2, (7, 5, 3)[lod])]
    profile = [(-0.02, 0.97), (0.02, 1.22), (0.05, 1.27), (0.1, 1.27), (0.1, 0.92)] + dome[1:]
    lathe(m.part("metal"), profile, PIPE_SIDES[lod], pipe_colour(0.22))
    m.meta = {"height": 2.27, "width": 2.54, "crown": [0.2, 1.0, 0.0, 1.27]}
    return m


# A concrete road barrier's profile (the Jersey shape): from its foot on one side up over its top
# and down the other, (across, up); and the length of one. textures.py's `barrier` paints its
# hazard stripes on the lower slopes by this profile's lengths.
BARRIER_PROFILE = [(7.0, 0.0), (7.0, 0.9), (3.7, 4.0), (2.5, 10.3), (2.1, 11.0), (-2.1, 11.0), (-2.5, 10.3), (-3.7, 4.0), (-7.0, 0.9), (-7.0, 0.0)]
BARRIER_LENGTH = 27.0


def barrier(name, seed, lod):
    """A concrete road barrier: the Jersey profile run along x (centred, `BARRIER_LENGTH` long),
    its faces mapped to the `barrier` layer round the profile (v) and along it (u), its ends
    flat. The game lays them end to end along each painted row."""
    m = Model(name)
    part = m.part("barrier")
    prof = np.array(BARRIER_PROFILE, np.float64)
    seg = np.linalg.norm(np.diff(prof, axis=0), axis=1)
    along = np.concatenate([[0.0], np.cumsum(seg)]) / seg.sum()
    half = BARRIER_LENGTH * 0.5

    def col(pts):
        ao = np.clip(0.6 + 0.4 * smoothstep(0.0, 8.0, pts[:, 1]), 0, 1)
        return np.stack([ao, np.zeros(len(pts)), np.full(len(pts), 0.04), np.zeros(len(pts))], axis=1)

    # The sides and top: a band of faces for each step of the profile.
    for i in range(len(prof) - 1):
        (z0, y0), (z1, y1) = prof[i], prof[i + 1]
        n = unit(np.array([0.0, z1 - z0, -(y1 - y0)]))
        # (Outwards: away from the profile's middle.)
        mid = np.array([0.0, (y0 + y1) * 0.5 - 5.5, (z0 + z1) * 0.5])
        if n @ mid < 0:
            n = -n
        pos = np.array([(-half, y0, z0), (half, y0, z0), (half, y1, z1), (-half, y1, z1)])
        uv = np.array([(0.0, along[i]), (1.0, along[i]), (1.0, along[i + 1]), (0.0, along[i + 1])])
        tri = [(0, 1, 2), (0, 2, 3)]
        if np.cross(pos[1] - pos[0], pos[2] - pos[0]) @ n < 0:
            tri = [(0, 2, 1), (0, 3, 2)]
        part.add(pos, np.repeat(n[None, :], 4, axis=0), uv, col(pos), tri)
    # The ends: the profile, flat, fanned from its middle (their concrete from the upper slopes').
    for side in (-1.0, 1.0):
        centre = np.array([side * half, 5.0, 0.0])
        ring = np.array([(side * half, y, z) for z, y in prof])
        pos = np.vstack([centre[None, :], ring])
        uv = np.stack([0.5 + pos[:, 2] / BARRIER_LENGTH, along[2] + 0.02 + pos[:, 1] / prof[:, 1].max() * (along[3] - along[2] - 0.04)], axis=1)
        n = np.array([side, 0.0, 0.0])
        tris = []
        for i in range(len(ring) - 1):
            t = (0, i + 1, i + 2)
            if np.cross(pos[t[1]] - pos[t[0]], pos[t[2]] - pos[t[0]]) @ n < 0:
                t = (0, i + 2, i + 1)
            tris.append(t)
        part.add(pos, np.repeat(n[None, :], len(pos), axis=0), uv, col(pos), tris)
    top = float(prof[:, 1].max())
    m.meta = {"height": top, "width": float(BARRIER_LENGTH), "crown": [0.0, top * 0.5, 0.0, 7.0], "length": float(BARRIER_LENGTH), "breadth": 14.0, "contact": 7.0}
    return m


def transit_node(name, seed, lod, radius=11.5, height=11.0):
    """A transit node of TA's urban worlds: a squat dome of riveted steel plates on a low round
    plinth, a ring of vents round its foot and a hatch on its crown (centred, its foot at 0)."""
    m = Model(name)
    sides = (24, 16, 10)[lod]
    part = m.part("metal")
    col = pipe_colour(0.25)

    plinth_h = height * 0.12
    dome_r = radius * 0.86
    dome_h = height - plinth_h
    # The plinth: a low drum with a bevelled top edge, its foot a little sunk.
    prof = [(-0.6, radius * 0.97), (plinth_h * 0.75, radius), (plinth_h, radius * 0.95), (plinth_h, dome_r)]
    # The dome: a squashed half-sphere in bands (each a hard-edged band of plates). (Its profile
    # is turned about the vertical: (height, radius).)
    bands = (7, 5, 3)[lod]
    for k in range(1, bands + 1):
        t = k / bands * (np.pi / 2) * 0.93
        prof.append((plinth_h + dome_h * np.sin(t) * 0.94, dome_r * np.cos(t)))
    # The hatch on its crown: a low collar and a flat lid.
    top = prof[-1][0]
    lid_r = prof[-1][1]
    prof += [(top + height * 0.035, lid_r * 0.92), (top + height * 0.05, lid_r * 0.6), (top + height * 0.05, 0.0)]
    pts, nrm, tris = [], [], []
    a = np.linspace(0.0, TAU, sides + 1)[:-1]
    for (y0, r0), (y1, r1) in zip(prof[:-1], prof[1:]):
        dy, dr = y1 - y0, r1 - r0
        length = np.hypot(dy, dr)
        if length < 1e-9:
            continue
        ny, nr = -dr / length, dy / length
        base = len(pts)
        for y, r in ((y0, r0), (y1, r1)):
            for t in a:
                pts.append((np.cos(t) * r, y, np.sin(t) * r))
                nrm.append((np.cos(t) * nr, ny, np.sin(t) * nr))
        for j in range(sides):
            k2 = (j + 1) % sides
            a0, b0, a1, b1 = base + j, base + k2, base + sides + j, base + sides + k2
            tris += [(a0, b1, b0), (a0, a1, b1)]
    pts, nrm = np.array(pts, np.float64), unit(np.array(nrm, np.float64))
    part.add(pts, nrm, np.zeros((len(pts), 2)), col(pts, nrm), tris)
    # A ring of vents round its foot: short dark boxes standing out of the dome.
    if lod < 2:
        vents = 8
        for i in range(vents):
            t = (i + 0.5) / vents * TAU
            c, s = np.cos(t), np.sin(t)
            r = dome_r * 0.97
            y = plinth_h + dome_h * 0.16
            w, h_, d = radius * 0.16, height * 0.1, radius * 0.08
            centre = np.array([c * r, y, s * r])
            out = np.array([c, 0.0, s])
            side = np.array([-s, 0.0, c])
            up = np.array([0.0, 1.0, 0.0])
            corners = []
            for dx in (-1, 1):
                for dy in (-1, 1):
                    for dz in (-1, 1):
                        corners.append(centre + side * dx * w * 0.5 + up * dy * h_ * 0.5 + out * dz * d * 0.5)
            corners = np.array(corners)
            faces = [((0, 2, 6, 4), -side), ((1, 5, 7, 3), side), ((0, 4, 5, 1), -up), ((2, 3, 7, 6), up), ((0, 1, 3, 2), -out), ((4, 6, 7, 5), out)]
            vp, vn, vt = [], [], []
            for quad, n in faces:
                b = len(vp)
                for q in quad:
                    vp.append(corners[q])
                    vn.append(n)
                tri = [(b, b + 1, b + 2), (b, b + 2, b + 3)]
                if np.cross(vp[b + 1] - vp[b], vp[b + 2] - vp[b]) @ n < 0:
                    tri = [(b, b + 2, b + 1), (b, b + 3, b + 2)]
                vt += tri
            vp, vn = np.array(vp), np.array(vn)
            vents_col = np.stack([np.full(len(vp), 0.35), np.zeros(len(vp)), np.full(len(vp), 0.1), np.zeros(len(vp))], axis=1)
            part.add(vp, vn, np.zeros((len(vp), 2)), vents_col, vt)
    m.meta = {"height": float(height), "width": float(2.0 * radius), "crown": [0.0, float(height * 0.5), 0.0, float(radius)], "contact": float(radius * 0.95)}
    return m


def revolve(part, prof, sides, col):
    """A solid of revolution about the vertical: `prof` (height, radius) from its foot up, each
    of its steps a band of its own (hard edges between them; round the axis the normals are
    smooth)."""
    a = np.linspace(0.0, TAU, sides + 1)[:-1]
    pts, nrm, tris = [], [], []
    for (y0, r0), (y1, r1) in zip(prof[:-1], prof[1:]):
        dy, dr = y1 - y0, r1 - r0
        length = np.hypot(dy, dr)
        if length < 1e-9:
            continue
        ny, nr = -dr / length, dy / length
        base = len(pts)
        for y, r in ((y0, r0), (y1, r1)):
            for t in a:
                pts.append((np.cos(t) * r, y, np.sin(t) * r))
                nrm.append((np.cos(t) * nr, ny, np.sin(t) * nr))
        for j in range(sides):
            k = (j + 1) % sides
            a0, b0, a1, b1 = base + j, base + k, base + sides + j, base + sides + k
            tris += [(a0, b1, b0), (a0, a1, b1)]
    pts, nrm = np.array(pts, np.float64), unit(np.array(nrm, np.float64))
    part.add(pts, nrm, np.zeros((len(pts), 2)), col(pts, nrm), tris)


def ellipsoid(part, centre, radii, level, col):
    """An ellipsoid (an icosphere stretched to `radii`) at `centre`, its normals smooth."""
    v, f = icosphere(level)
    pts = np.asarray(centre, float) + v * np.asarray(radii, float)
    nrm = unit(v / np.asarray(radii, float))
    part.add(pts, nrm, np.zeros((len(pts), 2)), col(pts, nrm), f)


def optimizer(name, seed, lod, radius=21.0, height=62.0):
    """A climatic optimizer of TA's urban worlds: a squat machine of two stacked drums, the lower
    one finned, on a low plinth, under a round head with a bulge at its back and a lamp in its
    face (to the south, where TA's camera saw it; centred, its foot at 0)."""
    m = Model(name)
    rng = np.random.default_rng(seed)
    sides = (24, 16, 10)[lod]
    part = m.part("metal")
    col = pipe_colour(0.25)
    r, h = radius, height
    revolve(part, [
        # The plinth, its foot a little sunk; the lower drum with a lip at its top; the upper
        # drum so too; the neck.
        (-0.6, r), (h * 0.055, r), (h * 0.07, r * 0.93),
        (h * 0.07, r * 0.88), (h * 0.31, r * 0.84), (h * 0.325, r * 0.89), (h * 0.355, r * 0.89), (h * 0.37, r * 0.72),
        (h * 0.37, r * 0.7), (h * 0.55, r * 0.66), (h * 0.565, r * 0.7), (h * 0.585, r * 0.7), (h * 0.6, r * 0.48),
        (h * 0.6, r * 0.44), (h * 0.68, r * 0.42),
    ], sides, col)
    level = (3, 2, 1)[lod]
    head = np.array([0.0, h * 0.8, r * 0.05])
    radii = np.array([r * 0.76, h * 0.21, r * 0.72])
    ellipsoid(part, head, radii, level, col)
    ellipsoid(part, head + np.array([-r * 0.45, -h * 0.05, -r * 0.35]), np.array([r * 0.42, h * 0.13, r * 0.4]), max(level - 1, 1), col)
    # Fins round the lower drum.
    if lod < 2:
        fins = 10 if lod == 0 else 6
        for i in range(fins):
            t = (i + 0.5) / fins * TAU + rng.uniform(-0.05, 0.05)
            c, s = np.cos(t), np.sin(t)
            out, side = np.array([c, 0.0, s]), np.array([-s, 0.0, c])
            corners = [np.array([0.0, y, 0.0]) + out * d + side * w for y in (h * 0.1, h * 0.29) for d in (r * 0.8, r * 0.95) for w in (-r * 0.03, r * 0.03)]
            # (Corner index: y * 4 + d * 2 + w; each face's corners round it.)
            faces = [((0, 4, 6, 2), -side), ((1, 3, 7, 5), side), ((0, 1, 5, 4), -out), ((2, 6, 7, 3), out), ((0, 2, 3, 1), np.array([0.0, -1.0, 0.0])), ((4, 5, 7, 6), np.array([0.0, 1.0, 0.0]))]
            vp, vn, vt = [], [], []
            for quad, n in faces:
                b = len(vp)
                for q in quad:
                    vp.append(corners[q])
                    vn.append(n)
                tri = [(b, b + 1, b + 2), (b, b + 2, b + 3)]
                if np.cross(vp[b + 1] - vp[b], vp[b + 2] - vp[b]) @ n < 0:
                    tri = [(b, b + 2, b + 1), (b, b + 3, b + 2)]
                vt += tri
            vp, vn = np.array(vp), unit(np.array(vn, float))
            part.add(vp, vn, np.zeros((len(vp), 2)), col(vp, vn), vt)
    # The lamp: a gem set into the head's face, wholly alight (the crystal's COLOR_0 alpha 1).
    d = unit(np.array([0.35, 0.3, 0.88]))
    at = head + d * radii * 0.92
    lamp = prism(at - d * r * 0.08, d, r * 0.2, r * 0.1, 6, r * 0.08, rng)
    faceted(m.part("crystal"), lamp, lambda p: np.ones(len(p)), 0.8)
    m.part("crystal").col[-1][:, 3] = 1.0
    m.meta = {"height": float(head[1] + radii[1]), "width": float(2.0 * radius), "crown": [0.0, float(h * 0.5), 0.0, float(radius)], "contact": float(radius * 0.95)}
    return m


def box(part, lo, hi, col, uv_tile=None):
    """An axis-aligned box from `lo` to `hi` (its faces hard-edged; no texture coordinates)."""
    lo, hi = np.asarray(lo, float), np.asarray(hi, float)
    c = [np.array([x, y, z]) for x in (lo[0], hi[0]) for y in (lo[1], hi[1]) for z in (lo[2], hi[2])]
    # (Corner index: x * 4 + y * 2 + z.)
    faces = [((0, 1, 3, 2), (-1, 0, 0)), ((4, 6, 7, 5), (1, 0, 0)), ((0, 4, 5, 1), (0, -1, 0)), ((2, 3, 7, 6), (0, 1, 0)), ((0, 2, 6, 4), (0, 0, -1)), ((1, 5, 7, 3), (0, 0, 1))]
    pts, nrm, tris = [], [], []
    for quad, n in faces:
        n = np.array(n, float)
        b = len(pts)
        for q in quad:
            pts.append(c[q])
            nrm.append(n)
        t = [(b, b + 1, b + 2), (b, b + 2, b + 3)]
        if np.cross(pts[b + 1] - pts[b], pts[b + 2] - pts[b]) @ n < 0:
            t = [(b, b + 2, b + 1), (b, b + 3, b + 2)]
        tris += t
    pts, nrm = np.array(pts), np.array(nrm)
    part.add(pts, nrm, np.zeros((len(pts), 2)), col(pts, nrm), tris)


def drum(part, centre, radius, y0, y1, sides, col, cap=True):
    """An upright cylinder at `centre` (x, z) from height `y0` to `y1`, its top closed."""
    a = np.linspace(0.0, TAU, sides + 1)[:-1]
    pts, nrm, tris = [], [], []
    for y in (y0, y1):
        for t in a:
            pts.append((centre[0] + np.cos(t) * radius, y, centre[1] + np.sin(t) * radius))
            nrm.append((np.cos(t), 0.0, np.sin(t)))
    for j in range(sides):
        k = (j + 1) % sides
        tris += [(j, sides + k, k), (j, sides + j, sides + k)]
    if cap:
        b = len(pts)
        pts.append((centre[0], y1, centre[1]))
        nrm.append((0.0, 1.0, 0.0))
        for t in a:
            pts.append((centre[0] + np.cos(t) * radius, y1, centre[1] + np.sin(t) * radius))
            nrm.append((0.0, 1.0, 0.0))
        for j in range(sides):
            k = (j + 1) % sides
            tris.append((b, b + 1 + k, b + 1 + j))
    pts, nrm = np.array(pts, float), np.array(nrm, float)
    part.add(pts, nrm, np.zeros((len(pts), 2)), col(pts, nrm), tris)


def relay(name, seed, lod, width=44.0):
    """A microwave relay of TA's urban worlds: a rounded concrete pad, on it a tall equipment
    cabinet roofed in a second paint (`trim`, its painting's warm roof) at +x, a smaller cabinet
    beside it, a tank and a mast carrying a dish (centred, its foot at 0)."""
    m = Model(name)
    metal = m.part("metal")
    trim = m.part("trim")
    w = width
    sides = (20, 12, 8)[lod]
    col = pipe_colour(0.2)

    def cab_col(p, n):
        ao = np.clip(0.6 + 0.4 * smoothstep(0.0, w * 0.2, p[:, 1]), 0, 1) * (0.8 + 0.2 * np.clip(n[:, 1] * 0.5 + 0.5, 0, 1))
        return np.stack([ao, np.zeros(len(p)), np.full(len(p), 0.15), np.zeros(len(p))], axis=1)

    # The pad: a low rounded slab.
    superquadric(metal, (0.0, w * 0.02, 0.0), (w * 0.5, w * 0.05, w * 0.38), 0.25, 0.35, (6, 4, 3)[lod], sides * 2, cab_col)
    pad = w * 0.07
    # The main cabinet and its roof.
    box(metal, (w * 0.0, pad, -w * 0.2), (w * 0.36, pad + w * 0.34, w * 0.14), cab_col)
    box(trim, (-w * 0.01, pad + w * 0.34, -w * 0.21), (w * 0.37, pad + w * 0.37, w * 0.15), cab_col)
    # A smaller cabinet beside it, its door side lit.
    box(metal, (-w * 0.3, pad, w * 0.04), (-w * 0.04, pad + w * 0.2, w * 0.26), cab_col)
    box(trim, (-w * 0.31, pad + w * 0.2, w * 0.03), (-w * 0.03, pad + w * 0.22, w * 0.27), cab_col)
    # A tank.
    drum(metal, (-w * 0.2, -w * 0.18), w * 0.09, pad, pad + w * 0.24, sides, col)
    if lod < 2:
        # The mast and its dish, turned up to the sky a little.
        drum(metal, (w * 0.1, w * 0.24), w * 0.025, pad, pad + w * 0.44, max(sides // 2, 6), col, cap=False)
        dish_c = np.array([w * 0.1, pad + w * 0.46, w * 0.24])
        rings = (5, 3)[lod]
        th = np.linspace(0.0, 1.0, rings + 1)
        a = np.linspace(0.0, TAU, sides + 1)[:-1]
        # (A shallow bowl facing up and out, its normal tilted 35 degrees towards +z.)
        tilt = np.radians(35.0)
        up = np.array([0.0, np.cos(tilt), np.sin(tilt)])
        side1 = np.array([1.0, 0.0, 0.0])
        side2 = np.cross(up, side1)
        r = w * 0.11
        pts, nrm, tris = [], [], []
        for i, t in enumerate(th):
            for ang in a:
                d = side1 * np.cos(ang) + side2 * np.sin(ang)
                pts.append(dish_c + d * r * t + up * (r * 0.35 * t * t))
                nrm.append(unit(up - d * t * 0.7))
        for i in range(rings):
            for j in range(sides):
                k = (j + 1) % sides
                p0, p1 = i * sides, (i + 1) * sides
                tris += [(p0 + j, p1 + k, p0 + k), (p0 + j, p1 + j, p1 + k)]
        pts, nrm = np.array(pts), np.array(nrm)
        # (Both sides of the bowl: its back too.)
        tris_back = [(a_, c_, b_) for a_, b_, c_ in tris]
        n = len(pts)
        metal.add(np.vstack([pts, pts]), np.vstack([nrm, -nrm]), np.zeros((2 * n, 2)), col(np.vstack([pts, pts]), np.vstack([nrm, -nrm])), tris + [(a_ + n, b_ + n, c_ + n) for a_, b_, c_ in tris_back])
    m.meta = {"height": float(pad + w * 0.57), "width": float(w), "crown": [0.0, float(w * 0.2), 0.0, float(w * 0.4)], "contact": float(w * 0.42)}
    return m


def rough_block(part, rng, half, height, bevel, jag, col, top_jag=0.0):
    """A block of stone or concrete, its foot at 0, `half` (x, z) across either way and `height`
    up: its edges bevelled by `bevel`, its corners pushed about by `jag`, its top's corners by
    `top_jag` more (broken off). Hard-edged faces; no texture coordinates (its layer is laid on
    along the world's axes)."""
    hx, hz = half
    # Eight corners of the core, each nudged; then each face inset by the bevel.
    def corner(sx, sy, sz):
        j = rng.uniform(-jag, jag, 3)
        y = (height if sy > 0 else 0.0)
        if sy > 0:
            y -= rng.uniform(0.0, top_jag)
        return np.array([sx * hx + j[0], y + (j[1] if sy > 0 else 0.0), sz * hz + j[2]])
    c = {(sx, sy, sz): corner(sx, sy, sz) for sx in (-1, 1) for sy in (-1, 1) for sz in (-1, 1)}
    faces = [
        ([(-1, -1, -1), (-1, -1, 1), (-1, 1, 1), (-1, 1, -1)], np.array([-1.0, 0, 0])),
        ([(1, -1, -1), (1, 1, -1), (1, 1, 1), (1, -1, 1)], np.array([1.0, 0, 0])),
        ([(-1, 1, -1), (-1, 1, 1), (1, 1, 1), (1, 1, -1)], np.array([0, 1.0, 0])),
        ([(-1, -1, -1), (1, -1, -1), (1, 1, -1), (-1, 1, -1)], np.array([0, 0, -1.0])),
        ([(-1, -1, 1), (-1, 1, 1), (1, 1, 1), (1, -1, 1)], np.array([0, 0, 1.0])),
    ]
    pts, nrm, tris = [], [], []
    centre = np.mean([v for v in c.values()], axis=0)
    for keys, _ in faces:
        q = [c[k] for k in keys]
        mid = np.mean(q, axis=0)
        n = unit(np.cross(q[1] - q[0], q[2] - q[0]))
        if n @ (mid - centre) < 0:
            q = q[::-1]
            n = -n
        # (The face drawn in towards its middle and out along its normal by the bevel: its edges
        # slope down to the block's, worn round.)
        inner = [p + (mid - p) * bevel + n * bevel * 0.4 for p in q]
        b = len(pts)
        for p in inner:
            pts.append(p)
            nrm.append(n)
        tris += [(b, b + 1, b + 2), (b, b + 2, b + 3)]
        # The bevel: a strip from each edge of the face out to the core's edge.
        for k in range(4):
            a0, a1 = inner[k], inner[(k + 1) % 4]
            o0, o1 = q[k], q[(k + 1) % 4]
            bn = unit(np.cross(a1 - a0, o0 - a0))
            if bn @ (((a0 + a1) * 0.5) - centre) < 0:
                bn = -bn
            b = len(pts)
            pts += [a0, a1, o1, o0]
            nrm += [bn] * 4
            t = [(b, b + 1, b + 2), (b, b + 2, b + 3)]
            if np.cross(pts[b + 1] - pts[b], pts[b + 2] - pts[b]) @ bn < 0:
                t = [(b, b + 2, b + 1), (b, b + 3, b + 2)]
            tris += t
    pts, nrm = np.array(pts), np.array(nrm)
    part.add(pts, nrm, np.zeros((len(pts), 2)), col(pts, nrm), tris)


def stone_col(height):
    def f(p, n):
        ao = np.clip(0.55 + 0.45 * smoothstep(0.0, max(height, 1e-3) * 0.6, p[:, 1]), 0, 1) * (0.85 + 0.15 * np.clip(n[:, 1], 0, 1))
        return np.stack([ao, np.zeros(len(p)), np.full(len(p), 0.03), np.zeros(len(p))], axis=1)
    return f


def block(name, seed, lod):
    """A block of a ruin's masonry, 1 across each way and 1 tall (the game stretches it to each
    patch of its painting): bevelled, its corners a little out of true, chipped at the top."""
    m = Model(name)
    rng = np.random.default_rng(seed)
    rough_block(m.part("rock"), rng, (0.5, 0.5), 1.0, (0.12, 0.1, 0.0)[lod], 0.03, stone_col(1.0), top_jag=0.12)
    m.meta = {"height": 1.0, "width": 1.0, "crown": [0.0, 0.5, 0.0, 0.5], "contact": 0.5}
    return m


def stump(name, seed, lod):
    """A broken concrete pillar, 1 across and 1 tall (the game stretches it to each painted
    stump): its top broken off unevenly, bent rods of steel standing out of the break."""
    m = Model(name)
    rng = np.random.default_rng(seed)
    rough_block(m.part("rock"), rng, (0.5, 0.5), 1.0, (0.06, 0.05, 0.0)[lod], 0.02, stone_col(1.0), top_jag=0.3)
    if lod < 2:
        metal = m.part("metal")
        for k in range(4 if lod == 0 else 2):
            x, z = (k % 2 - 0.5) * 0.55, (k // 2 - 0.5) * 0.55
            lean = rng.uniform(-0.25, 0.25, 2)
            top = 1.0 + rng.uniform(0.1, 0.35)
            pts = [(x, 0.6, z), (x + lean[0] * 0.3, 0.95, z + lean[1] * 0.3), (x + lean[0], top, z + lean[1])]
            tube(metal, np.array(pts), np.full(3, 0.035), (5, 4)[lod], lambda p, r: np.tile([0.5, 0.0, 0.15, 0.0], (len(p), 1)))
    m.meta = {"height": 1.0, "width": 1.0, "crown": [0.0, 0.5, 0.0, 0.5], "contact": 0.5}
    return m


# ---------------------------------------------------------------------------------------------
# The catalogue: every model, by name (the game picks among them, `ta_render::foliage`)
# ---------------------------------------------------------------------------------------------


def catalogue():
    """(name, builder(lod) -> Model) for every model."""
    out = []

    # Broadleaf trees, as TA's green worlds' round ones stand: a crown about 52 across whose
    # middle is some 80 up (the painted tree, seen from TA's height, hides most of the trunk).
    def broad(name, seed, lobes, **kw):
        out.append((name, lambda lod: broadleaf(name, seed, lod, lobes, **kw)))

    broad("broadleaf_a", 11, [((0, 74, 0), (27, 32, 27)), ((-9, 60, 7), (19, 18, 19))], trunk_top=50.0, trunk_radius=3.4, leaf_size=14.0)
    broad("broadleaf_b", 12, [((0, 72, 0), (28, 30, 26)), ((11, 62, -6), (19, 17, 17)), ((-10, 84, 4), (17, 16, 17))], trunk_top=48.0, trunk_radius=3.6, leaf_size=14.0, lean=0.04)
    broad("broadleaf_c", 13, [((0, 76, 0), (24, 36, 24)), ((5, 56, -4), (18, 14, 18))], trunk_top=46.0, trunk_radius=3.2, leaf_size=13.0)
    # (Clumps of leaves with gaps between, the crown lopsided: not every tree a round ball.)
    broad("broadleaf_d", 14, [((-6, 70, 3), (21, 25, 20)), ((15, 79, -5), (16, 19, 15)), ((-13, 88, -9), (14, 13, 13)), ((5, 57, 13), (15, 12, 13))], trunk_top=46.0, trunk_radius=3.3, leaf_size=13.5, lean=-0.03)
    broad("broadleaf_e", 15, [((3, 81, -2), (22, 23, 21)), ((-15, 66, 7), (15, 15, 14)), ((17, 63, 9), (14, 14, 13))], trunk_top=50.0, trunk_radius=3.5, leaf_size=14.5, lean=0.05)

    def coni(name, seed, **kw):
        out.append((name, lambda lod: conifer(name, seed, lod, **kw)))

    # Conifers, columnar like TA's: about 28 across and 110 tall.
    coni("conifer_a", 21, height=112.0, radius=15.0, base=10.0, trunk_radius=3.0, leaf_size=13.0)
    coni("conifer_b", 22, height=104.0, radius=17.0, base=8.0, trunk_radius=3.2, leaf_size=14.0)
    coni("conifer_c", 23, height=118.0, radius=13.0, base=12.0, trunk_radius=2.8, leaf_size=12.0)

    def add(name, build, **kw):
        out.append((name, lambda lod: build(name, kw.pop("seed_", 0), lod, **kw) if False else build(name, kw["seed"], lod, **{k: v for k, v in kw.items() if k != "seed"})))

    # Jungle giants, as TA's lush worlds' wide ones: about 85 across.
    add("jungle_canopy_a", jungle_canopy, seed=41, lobes=[((-20, 70, -4), (22, 12, 20)), ((18, 74, 6), (24, 13, 21)), ((2, 80, -18), (19, 11, 17)), ((0, 84, 16), (18, 10, 17))], trunk_height=44.0, trunk_radius=4.2, leaf_size=13.0, strand=17.0)
    add("jungle_canopy_b", jungle_canopy, seed=42, lobes=[((-16, 66, 10), (24, 13, 22)), ((20, 72, -4), (23, 12, 22)), ((-4, 82, -14), (21, 12, 19))], trunk_height=40.0, trunk_radius=4.6, leaf_size=13.0, strand=18.0)
    # Tall jungle trees, about 44 across and twice that tall.
    add("jungle_tall_a", jungle_tall, seed=43, height=104.0, lobes=[((4, 92, 0), (19, 13, 18)), ((-9, 66, 6), (16, 11, 15)), ((10, 48, -6), (13, 9, 12))], trunk_radius=3.8, leaf_size=12.0, strand=16.0)
    add("jungle_tall_b", jungle_tall, seed=44, height=110.0, lobes=[((-3, 98, 2), (18, 12, 18)), ((10, 72, -5), (17, 11, 15))], trunk_radius=4.0, leaf_size=12.0, strand=17.0)
    # Tuber trees, about 42 across.
    add("tuber_a", tuber, seed=45, bulb=10.0, stems=5, height=60.0, pad=16.0)
    add("tuber_b", tuber, seed=46, bulb=11.0, stems=6, height=56.0, pad=15.0)
    # Tropical trees, about 55 across.
    add("tropical_a", tropical, seed=47, height=76.0, spread=58.0, branches=3, leaf_size=17.0)
    add("tropical_b", tropical, seed=48, height=82.0, spread=62.0, branches=4, leaf_size=16.0)
    add("tropical_c", tropical, seed=49, height=70.0, spread=50.0, branches=3, leaf_size=18.0)
    # Burnt ones.
    add("dead_broadleaf_a", dead_broadleaf, seed=51, lobes=[((0, 70, 0), (22, 28, 22))], trunk_top=44.0, trunk_radius=3.2, twig_size=16.0)
    add("dead_broadleaf_b", dead_broadleaf, seed=52, lobes=[((0, 66, 0), (20, 26, 20)), ((8, 56, 6), (14, 14, 14))], trunk_top=40.0, trunk_radius=3.4, twig_size=15.0)
    add("dead_conifer_a", dead_conifer, seed=53, height=100.0, radius=13.0, trunk_radius=2.8, twig_size=11.0)
    add("dead_jungle_a", jungle_canopy, seed=54, lobes=[((-18, 66, -4), (20, 11, 18)), ((16, 70, 6), (21, 12, 19)), ((2, 76, -16), (17, 10, 16))], trunk_height=42.0, trunk_radius=4.2, leaf_size=13.0, strand=16.0, dead=True)
    add("dead_tall_a", jungle_tall, seed=55, height=100.0, lobes=[((4, 90, 0), (17, 12, 16)), ((-8, 64, 6), (14, 10, 14))], trunk_radius=3.6, leaf_size=12.0, strand=15.0, dead=True)
    add("dead_tuber_a", tuber, seed=56, bulb=10.0, stems=5, height=52.0, pad=15.0, dead=True)
    add("dead_tropical_a", tropical, seed=57, height=74.0, spread=56.0, branches=3, leaf_size=16.0, dead=True)
    # Bushes and ground plants.
    add("bush_a", bush, seed=61, width=28.0, height=13.0, leaf_size=8.0)
    add("bush_b", bush, seed=62, width=32.0, height=11.0, leaf_size=8.5)
    add("fern_a", ground_plant, seed=63, width=40.0, height=14.0, layer="tropical", fronds=9)
    # (The jungle texture's strands laid along each frond, their tips outwards; its twig, the top
    # rows the jungle trees' hanging cards hang from, left off: across a frond's end it read as
    # a line floating over the leaves.)
    add("fern_b", ground_plant, seed=64, width=46.0, height=16.0, layer="jungle", fronds=11, v=(0.12, 1.0))

    def stone(name, seed, size, **kw):
        out.append((name, lambda lod: rock(name, seed, lod, size, **kw)))

    # Spires: lava worlds' rock towers, wet deserts' pillars.
    add("spire_a", spire, seed=71, height=150.0, radius=46.0, ridges=7.0)
    add("spire_b", spire, seed=72, height=190.0, radius=50.0, ridges=9.0, lean=0.06)
    add("spire_c", spire, seed=73, height=120.0, radius=44.0, ridges=6.0, strata=0.09)
    add("pillar_a", spire, seed=74, height=110.0, radius=24.0, ridges=11.0, strata=0.1, flare=0.2)
    add("pillar_b", spire, seed=75, height=90.0, radius=22.0, ridges=9.0, strata=0.12, flare=0.15, lean=0.05)
    # Wet deserts' clusters of banded spires, rounded at their tops.
    add("hoodoos_a", hoodoos, seed=86, height=110.0, count=3, spread=70.0)
    add("hoodoos_b", hoodoos, seed=87, height=100.0, count=2, spread=60.0)
    add("hoodoos_c", hoodoos, seed=88, height=120.0, count=5, spread=90.0)
    add("hoodoos_d", hoodoos, seed=89, height=105.0, count=6, spread=150.0, row=0.45)
    # Slate worlds' fog strata: clusters of tall blades.
    add("shards_a", shard_cluster, seed=76, height=180.0, spread=70.0, count=14, radius=9.0, sides=5, breaks=2)
    add("shards_b", shard_cluster, seed=77, height=150.0, spread=60.0, count=11, radius=10.0, sides=5, breaks=3)
    # Crystal worlds' spires.
    add("crystal_a", crystal_spire, seed=78, height=170.0, spread=60.0, plate=170.0)
    add("crystal_b", crystal_spire, seed=79, height=130.0, spread=70.0, plate=190.0)
    # Ice worlds' chunks: shards lying about.
    add("ice_a", shard_cluster, seed=80, height=16.0, spread=56.0, count=8, radius=6.0, sides=5, squash=0.8, tilt=2.2, gloss=0.55, jag=0.15, low=0.4)
    add("ice_b", shard_cluster, seed=81, height=20.0, spread=48.0, count=6, radius=7.0, sides=4, squash=0.7, tilt=1.8, gloss=0.55, jag=0.2, low=0.45)

    # Acid worlds' gasbags, lava worlds' energy nodes, Mars plants.
    # (Five, their tethers as long as those of TA's five paintings: the game picks the one whose
    # height fits, the bag kept round.)
    add("gasbag_a", gasbag, seed=82, bag=24.0, neck=2.6, lean=0.14)
    add("gasbag_b", gasbag, seed=83, bag=26.0, neck=2.0, lean=0.16)
    add("gasbag_c", gasbag, seed=88, bag=26.0, neck=1.25, lean=0.18)
    add("gasbag_d", gasbag, seed=89, bag=25.0, neck=0.9, lean=0.10)
    add("gasbag_e", gasbag, seed=90, bag=27.0, neck=0.6, lean=0.06)
    add("node_a", energy_node, seed=84, size=90.0, tubes=3)
    add("node_b", energy_node, seed=85, size=70.0, tubes=2)
    # (Six or seven winding arms: eight spaced evenly read as a spider.)
    add("tendrils_a", tendrils, seed=86, width=150.0, arms=6)
    add("tendrils_b", tendrils, seed=87, width=170.0, arms=7)

    # Vehicles: each is stretched to its painting's length, breadth and height.
    add("car_a", car, seed=91, length=30.0, width=15.0, height=14.0, dome=0.55, nose=0.04)
    add("car_b", car, seed=92, length=32.0, width=14.0, height=14.0, dome=0.42, nose=-0.06)
    add("truck_a", truck, seed=93, length=76.0, width=36.0, height=40.0)

    # Pipework and barriers: pieces laid along each painting's lines.
    add("pipe_run", pipe_run, seed=0)
    add("pipe_joint", pipe_joint, seed=0)
    add("pipe_collar", pipe_collar, seed=0)
    add("pipe_cap", pipe_cap, seed=0)
    add("barrier_a", barrier, seed=0)
    # Urban worlds' transit nodes: a squat dome, stretched to its painting's width.
    add("transit_a", transit_node, seed=0)
    # And their microwave relays, turned as painted (their roofs where the painting's are).
    add("relay_a", relay, seed=0)
    # And their climatic optimizers, a lamp in their faces.
    add("optimizer_a", optimizer, seed=0)
    # Pieces set where a painting's patches lie: a ruin's masonry, an urban ruin's broken pillars.
    add("block_a", block, seed=94)
    add("block_b", block, seed=95)
    add("stump_a", stump, seed=96)
    add("stump_b", stump, seed=97)

    stone("boulder_round", 31, (40.0, 26.0, 34.0), planes=18, sharp=0.6, rough=0.1)
    stone("boulder_flat", 32, (46.0, 16.0, 38.0), planes=12, sharp=1.0, rough=0.07, sink=0.15)
    stone("boulder_angular", 33, (38.0, 30.0, 32.0), planes=9, sharp=2.0, rough=0.06)
    stone("boulder_tall", 34, (26.0, 44.0, 24.0), planes=11, sharp=1.4, rough=0.08)

    # TA Reborn's own festive features (festive.py), their colours their own.
    import festive
    out += festive.catalogue(sys.modules[__name__])
    return out


# ---------------------------------------------------------------------------------------------
# GLB
# ---------------------------------------------------------------------------------------------


def write_glb(path, models):
    """Writes the models (each a list of levels of detail) as one GLB."""
    bin_ = bytearray()
    accessors, views, meshes, nodes, materials = [], [], [], [], []
    mat_index = {}

    def blob(data, target=None):
        while len(bin_) % 4:
            bin_.append(0)
        views.append({"buffer": 0, "byteOffset": len(bin_), "byteLength": len(data), **({"target": target} if target else {})})
        bin_.extend(data)
        return len(views) - 1

    def accessor(arr, ctype, kind, normalized=False, minmax=False, target=34962):
        view = blob(arr.tobytes(), target)
        a = {"bufferView": view, "componentType": ctype, "count": int(len(arr)), "type": kind}
        if normalized:
            a["normalized"] = True
        if minmax:
            a["min"] = [float(x) for x in arr.min(axis=0)]
            a["max"] = [float(x) for x in arr.max(axis=0)]
        accessors.append(a)
        return len(accessors) - 1

    for levels in models:
        for lod, m in enumerate(levels):
            prims = []
            for layer, part in sorted(m.parts.items()):
                arr = part.arrays()
                if arr is None:
                    continue
                pos, nrm, uv, col, idx = arr
                if layer not in mat_index:
                    materials.append({"name": layer})
                    mat_index[layer] = len(materials) - 1
                col8 = np.round(col * 255).astype(np.uint8)
                index = idx.astype(np.uint16 if len(pos) < 65536 else np.uint32).reshape(-1)
                prims.append({
                    "attributes": {
                        "POSITION": accessor(pos.astype(np.float32), 5126, "VEC3", minmax=True),
                        "NORMAL": accessor(nrm.astype(np.float32), 5126, "VEC3"),
                        "TEXCOORD_0": accessor(uv.astype(np.float32), 5126, "VEC2"),
                        "COLOR_0": accessor(col8, 5121, "VEC4", normalized=True),
                    },
                    "indices": accessor(index, 5123 if index.dtype == np.uint16 else 5125, "SCALAR", target=34963),
                    "material": mat_index[layer],
                })
            meshes.append({"name": f"{m.name}.lod{lod}", "primitives": prims})
            node = {"name": f"{m.name}.lod{lod}", "mesh": len(meshes) - 1}
            if lod == 0:
                node["extras"] = m.meta
            nodes.append(node)
    doc = {
        "asset": {"version": "2.0", "generator": "TA Reborn tools/features/build.py"},
        "scene": 0,
        "scenes": [{"nodes": list(range(len(nodes)))}],
        "nodes": nodes,
        "meshes": meshes,
        "materials": materials,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": len(bin_)}],
    }
    js = json.dumps(doc, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    while len(bin_) % 4:
        bin_.append(0)
    with open(path, "wb") as f:
        f.write(struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(js) + 8 + len(bin_)))
        f.write(struct.pack("<II", len(js), 0x4E4F534A))
        f.write(js)
        f.write(struct.pack("<II", len(bin_), 0x004E4942))
        f.write(bin_)


# ---------------------------------------------------------------------------------------------
# Previews (inside Blender)
# ---------------------------------------------------------------------------------------------


def preview(models, out_dir, textures):
    import bpy

    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 24
    scene.cycles.device = "GPU"
    prefs = bpy.context.preferences.addons["cycles"].preferences
    try:
        prefs.compute_device_type = "OPTIX"
        prefs.get_devices()
        for d in prefs.devices:
            d.use = True
    except Exception:
        pass
    scene.render.resolution_x = 1800
    scene.render.resolution_y = 900
    scene.render.film_transparent = False
    world = bpy.data.worlds.new("sky")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs[0].default_value = (0.55, 0.65, 0.8, 1.0)
    world.node_tree.nodes["Background"].inputs[1].default_value = 0.8
    scene.world = world
    mats = {}
    for layer in set(l for levels in models for m in levels for l in m.parts):
        mat = bpy.data.materials.new(layer)
        mat.use_nodes = True
        nt = mat.node_tree
        bsdf = nt.nodes["Principled BSDF"]
        img = nt.nodes.new("ShaderNodeTexImage")
        img.image = bpy.data.images.load(os.path.join(textures, f"{layer}.png"))
        if layer in ("rock", "crystal", "hull", "metal", "trim"):
            coord = nt.nodes.new("ShaderNodeTexCoord")
            img.projection = "BOX"
            img.projection_blend = 0.3
            mapping = nt.nodes.new("ShaderNodeMapping")
            mapping.inputs["Scale"].default_value = (1 / 40.0, 1 / 40.0, 1 / 40.0)
            nt.links.new(coord.outputs["Object"], mapping.inputs["Vector"])
            nt.links.new(mapping.outputs["Vector"], img.inputs["Vector"])
        attr = nt.nodes.new("ShaderNodeVertexColor")
        attr.layer_name = "Col"
        sep = nt.nodes.new("ShaderNodeSeparateColor")
        nt.links.new(attr.outputs["Color"], sep.inputs["Color"])
        mul = nt.nodes.new("ShaderNodeMix")
        mul.data_type = "RGBA"
        mul.blend_type = "MULTIPLY"
        mul.inputs["Factor"].default_value = 1.0
        nt.links.new(img.outputs["Color"], mul.inputs[6])
        nt.links.new(sep.outputs["Red"], mul.inputs[7])
        nt.links.new(mul.outputs[2], bsdf.inputs["Base Color"])
        bsdf.inputs["Roughness"].default_value = 0.8
        if layer in CUTOUT:
            gt = nt.nodes.new("ShaderNodeMath")
            gt.operation = "GREATER_THAN"
            gt.inputs[1].default_value = 0.5
            nt.links.new(img.outputs["Alpha"], gt.inputs[0])
            nt.links.new(gt.outputs[0], bsdf.inputs["Alpha"])
            bsdf.inputs["Subsurface Weight"].default_value = 0.0
            bsdf.inputs["Transmission Weight"].default_value = 0.0
        mats[layer] = mat
    spacing = 120.0
    columns = []
    for row, levels in enumerate(models):
        for lod, m in enumerate(levels):
            x = lod * spacing
            y = row * spacing
            columns.append((x, y))
            for layer, part in m.parts.items():
                arr = part.arrays()
                if arr is None:
                    continue
                pos, nrm, uv, col, idx = arr
                # glTF y-up to Blender z-up.
                bp = np.stack([pos[:, 0] + x, -pos[:, 2] + y, pos[:, 1]], axis=1)
                mesh = bpy.data.meshes.new(f"{m.name}.{lod}.{layer}")
                mesh.from_pydata(bp.tolist(), [], idx.tolist())
                uvl = mesh.uv_layers.new(name="UV")
                loops = idx.reshape(-1)
                uvl.data.foreach_set("uv", np.stack([uv[loops, 0], 1.0 - uv[loops, 1]], axis=1).reshape(-1))
                ca = mesh.color_attributes.new("Col", "FLOAT_COLOR", "POINT")
                ca.data.foreach_set("color", col.reshape(-1))
                bn = np.stack([nrm[:, 0], -nrm[:, 2], nrm[:, 1]], axis=1)
                mesh.normals_split_custom_set_from_vertices(bn.tolist())
                mesh.materials.append(mats[layer])
                obj = bpy.data.objects.new(mesh.name, mesh)
                obj["row"] = row
                scene.collection.objects.link(obj)
    sun = bpy.data.lights.new("sun", "SUN")
    sun.energy = 4.0
    sun.angle = 0.05
    so = bpy.data.objects.new("sun", sun)
    so.rotation_euler = (0.9, 0.2, 0.8)
    scene.collection.objects.link(so)
    ground = bpy.data.meshes.new("ground")
    ground.from_pydata([(-400, -400, 0), (800, -400, 0), (800, len(models) * spacing + 400, 0), (-400, len(models) * spacing + 400, 0)], [], [(0, 1, 2, 3)])
    gm = bpy.data.materials.new("ground")
    gm.use_nodes = True
    gm.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (0.25, 0.3, 0.15, 1)
    ground.materials.append(gm)
    scene.collection.objects.link(bpy.data.objects.new("ground", ground))
    cam = bpy.data.cameras.new("cam")
    cam.lens = 50
    co = bpy.data.objects.new("cam", cam)
    scene.collection.objects.link(co)
    scene.camera = co
    scene.render.resolution_x = 1500
    scene.render.resolution_y = 600
    os.makedirs(out_dir, exist_ok=True)
    # Each model's three levels side by side, from a little above (as a unit sees them) and from
    # TA's own height.
    for row, levels in enumerate(models):
        y = row * spacing
        for obj in scene.objects:
            if "row" in obj:
                obj.hide_render = obj["row"] != row
        for label, elev, dist in (("side", 8.0, 520.0), ("ta", 63.4, 600.0)):
            e = np.radians(elev)
            co.location = (spacing, y - dist * np.cos(e), 40.0 + dist * np.sin(e))
            co.rotation_euler = (np.pi / 2 - e, 0.0, 0.0)
            scene.render.filepath = os.path.join(out_dir, f"{levels[0].name}_{label}.png")
            bpy.ops.render.render(write_still=True)


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else sys.argv[1:]
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=os.path.join(HERE, "..", "..", "crates", "ta_render", "features"))
    ap.add_argument("--preview")
    ap.add_argument("--textures")
    ap.add_argument("--only")
    args = ap.parse_args(argv)
    models = []
    for name, build in catalogue():
        if args.only and not any(name.startswith(o) for o in args.only.split(",")):
            continue
        levels = [build(lod) for lod in range(3)]
        print(f"{name}: " + ", ".join(f"lod{k} {m.triangles()} tris" for k, m in enumerate(levels)) + f"  {levels[0].meta}")
        models.append(levels)
    if args.preview:
        preview(models, args.preview, args.textures or args.out)
        return
    os.makedirs(args.out, exist_ok=True)
    path = os.path.join(args.out, "features.glb")
    write_glb(path, models)
    print(f"wrote {path} ({os.path.getsize(path) / 1e6:.2f} MB)")


if __name__ == "__main__":
    main()
