"""TA Reborn's own festive features (original models): snowy pines, decorated firs, snowmen,
presents, lanterns, ice sculptures, cottages, a sleigh, a clock tower, fireworks and strings of
lanterns, for the seasonal maps (`apps/ta_tools/src/mapgen`, the Christmas and New Year
recipes). The game draws these models close up in place of their paintings
(`assets/reborn/anims/festive.gaf`). The paintings are drawn from the same models in TA's view
(`tools/features/sprites.py`), so the two agree.

Their colours are their own: each model's extras say so (`own`). The needles and the bark keep
their textures' colours. The snow and the festive atlas (`festive_textures.py`) are never
tinted to a painting. The ice is the extras' `leaf` colour, with their `light` within it. COLOR_0
on the festive layer: r how open to the sky, g how far the wind bends it, b how much it shines
(for a light, the phase it twinkles at, 0 steady), a how brightly it glows by itself.

Everything faces south (+z), where TA's camera saw it; each model is centred on its footprint,
its foot at 0. Built into features.glb by build.py (the end of its catalogue).
"""
import numpy as np

from festive_textures import cell_uv

B = None  # build.py, given to catalogue()
TAU = 2.0 * np.pi
UP = np.array([0.0, 1.0, 0.0])


def linear(c):
    """An sRGB colour (0..255) in linear light."""
    c = np.asarray(c, float) / 255.0
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def colours(n, ao, bend=0.0, gloss=0.0, emit=0.0):
    """COLOR_0 for `n` vertices (each argument a number or `n` values)."""
    f = lambda v: np.broadcast_to(np.asarray(v, float), (n,))
    return np.stack([f(ao), f(bend), f(gloss), f(emit)], axis=1)


def sky(nrm, floor):
    """Open to the sky on top, less so underneath."""
    return floor + (1.0 - floor) * (0.5 + 0.5 * nrm[:, 1])


def uv_of(cell, n):
    """All `n` vertices at the middle of a cell (a plain colour), or none for the layers laid on
    along the world's axes (snow, crystal)."""
    if cell is None:
        return np.zeros((n, 2))
    return np.repeat(cell_uv(cell, 0.5, 0.5)[None, :], n, axis=0)


def ball(m, layer, centre, radii, level, cell=None, floor=0.6, bend=0.0, gloss=0.0, emit=0.0, rough=0.0, seed=0, frame=None):
    """An ellipsoid (an icosphere stretched to `radii`, turned by `frame`, its columns the local
    axes), roughened by `rough` of its size."""
    v, f = B.icosphere(level)
    r = np.asarray(radii, float)
    pts = v * r
    if rough > 0.0:
        pts = pts * (1.0 + rough * B.value_noise(v * 2.3 + seed * 0.37, seed))[:, None]
    nrm = B.unit(v / r)
    if frame is not None:
        pts, nrm = pts @ frame.T, nrm @ frame.T
    pts = pts + np.asarray(centre, float)
    m.part(layer).add(pts, nrm, uv_of(cell, len(pts)), colours(len(pts), sky(nrm, floor), bend, gloss, emit), f)


def panel(m, layer, origin, du, dv, cell=None, nu=1, nv=1, floor=0.55, bend=0.0, gloss=0.0, emit=0.0, ao=None):
    """A flat face from `origin` (its top left as seen from outside) across `du` and down `dv`,
    in `nu` x `nv` tiles each showing all of its cell."""
    origin, du, dv = (np.asarray(x, float) for x in (origin, du, dv))
    n = B.unit(np.cross(dv, du))
    pos, uv, tris = [], [], []
    for j in range(nv):
        for i in range(nu):
            b = len(pos)
            for (a, c) in ((0, 0), (1, 0), (1, 1), (0, 1)):
                pos.append(origin + du * (i + a) / nu + dv * (j + c) / nv)
                uv.append(cell_uv(cell, a, c) if cell is not None else (0.0, 0.0))
            t = [(b, b + 1, b + 2), (b, b + 2, b + 3)]
            if np.cross(pos[b + 1] - pos[b], pos[b + 2] - pos[b]) @ n < 0:
                t = [(b, b + 2, b + 1), (b, b + 3, b + 2)]
            tris += t
    pos = np.array(pos)
    nrm = np.repeat(n[None, :], len(pos), axis=0)
    a = sky(nrm, floor) if ao is None else ao
    m.part(layer).add(pos, nrm, np.array(uv), colours(len(pos), a, bend, gloss, emit), tris)


def block(m, layer, centre, half, cell=None, yaw=0.0, tile=None, floor=0.55, bend=0.0, gloss=0.0, emit=0.0, skip=(), cells=None):
    """A box round `centre` (`half` its half sizes along x, y, z before it is turned by `yaw`
    about the vertical), each face a panel: tiled every `tile` units if given, else one cell a
    face; `cells` may name a face's own cell (`top`, `bottom`, `front` (+z), `back`, `left`
    (-x), `right`). `skip` leaves faces out."""
    c, h = np.asarray(centre, float), np.asarray(half, float)
    cs, sn = np.cos(yaw), np.sin(yaw)
    ex, ey, ez = np.array([cs, 0.0, -sn]) * h[0], UP * h[1], np.array([sn, 0.0, cs]) * h[2]
    faces = {
        # (top left corner, across, down) as seen from outside.
        "front": (c - ex + ey + ez, 2 * ex, -2 * ey),
        "back": (c + ex + ey - ez, -2 * ex, -2 * ey),
        "right": (c + ex + ey + ez, -2 * ez, -2 * ey),
        "left": (c - ex + ey - ez, 2 * ez, -2 * ey),
        "top": (c - ex + ey - ez, 2 * ex, 2 * ez),
        "bottom": (c - ex - ey + ez, 2 * ex, -2 * ez),
    }
    for name, (o, du, dv) in faces.items():
        if name in skip:
            continue
        nu = max(1, int(round(np.linalg.norm(du) / tile))) if tile else 1
        nv = max(1, int(round(np.linalg.norm(dv) / tile))) if tile else 1
        panel(m, layer, o, du, dv, (cells or {}).get(name, cell), nu, nv, floor, bend, gloss, emit)


def ring_frame(axis, up=None):
    """The axis and two directions across it (the first towards `up`, if given)."""
    axis = B.unit(axis)
    if up is not None:
        u = B.unit(np.asarray(up, float) - axis * (np.asarray(up, float) @ axis))
    else:
        u = B.perpendicular(axis)
    return axis, u, np.cross(axis, u)


def cylinder(m, layer, base, axis, r0, r1, length, sides, cell=None, tiles=1, cap0=False, cap1=True, floor=0.6, bend=0.0, gloss=0.0, emit=0.0, cap_cell=None, up=None):
    """A cylinder (a cone where `r1` < `r0`) from `base` along `axis`, `length` long, its side in
    `tiles` strips round it each showing its cell, its ends closed as asked (a cap's cell upright
    towards `up`, if given: a clock's face)."""
    axis, u, w = ring_frame(axis, up)
    base = np.asarray(base, float)
    slope = (r0 - r1) / max(length, 1e-6)
    per = max(sides // tiles, 2)
    pos, nrm, uv, tris = [], [], [], []
    for k in range(tiles):
        b = len(pos)
        for i in range(per + 1):
            a = (k + i / per) / tiles * TAU
            d = np.cos(a) * u + np.sin(a) * w
            n = B.unit(d + axis * slope)
            for j, (r, along) in enumerate(((r0, 0.0), (r1, length))):
                pos.append(base + axis * along + d * r)
                nrm.append(n)
                uv.append(cell_uv(cell, i / per, 1.0 - j) if cell is not None else (0.0, 0.0))
        for i in range(per):
            a0, a1, b0, b1 = b + 2 * i, b + 2 * i + 1, b + 2 * i + 2, b + 2 * i + 3
            tris += [(a0, b0, b1), (a0, b1, a1)]
    for at, r, end, out in ((0.0, r0, cap0, -axis), (length, r1, cap1, axis)):
        if not end or r <= 1e-6:
            continue
        b = len(pos)
        pos.append(base + axis * at)
        nrm.append(out)
        uv.append(cell_uv(cap_cell or cell, 0.5, 0.5) if (cap_cell or cell) is not None else (0.0, 0.0))
        # (Seen from outside: its cell's top towards `u`, its right across.)
        right = np.cross(u, out)
        for i in range(sides):
            a = i / sides * TAU
            d = np.cos(a) * u + np.sin(a) * w
            pos.append(base + axis * at + d * r)
            nrm.append(out)
            uv.append(cell_uv(cap_cell or cell, 0.5 + 0.5 * (d @ right), 0.5 - 0.5 * (d @ u)) if (cap_cell or cell) is not None else (0.0, 0.0))
        for i in range(sides):
            j = (i + 1) % sides
            t = (b, b + 1 + i, b + 1 + j)
            if np.cross(pos[t[1]] - pos[t[0]], pos[t[2]] - pos[t[0]]) @ out < 0:
                t = (b, b + 1 + j, b + 1 + i)
            tris.append(t)
    pos, nrm = np.array(pos), B.unit(np.array(nrm))
    # (Each band faces the right way: its triangles wound to their normals.)
    tris = [t if np.cross(pos[t[1]] - pos[t[0]], pos[t[2]] - pos[t[0]]) @ (nrm[t[0]] + nrm[t[1]] + nrm[t[2]]) >= 0 else (t[0], t[2], t[1]) for t in tris]
    m.part(layer).add(pos, nrm, np.array(uv), colours(len(pos), sky(nrm, floor), bend, gloss, emit), tris)


def torus(m, layer, centre, axis, big, small, sides, rings, cell=None, floor=0.6, bend=0.0, gloss=0.0, emit=0.0, arc=1.0):
    """A ring round `axis` at `centre` (`big` its radius, `small` its tube's), all or `arc` of
    the way round."""
    axis, u, w = ring_frame(axis)
    pos, nrm, tris = [], [], []
    for i in range(rings + 1):
        a = i / rings * TAU * arc
        d = np.cos(a) * u + np.sin(a) * w
        for j in range(sides):
            b = j / sides * TAU
            n = np.cos(b) * d + np.sin(b) * axis
            pos.append(np.asarray(centre, float) + d * big + n * small)
            nrm.append(n)
    for i in range(rings):
        for j in range(sides):
            k = (j + 1) % sides
            a0, a1, b0, b1 = i * sides + j, i * sides + k, (i + 1) * sides + j, (i + 1) * sides + k
            tris += [(a0, b0, b1), (a0, b1, a1)]
    pos, nrm = np.array(pos), B.unit(np.array(nrm))
    tris = [t if np.cross(pos[t[1]] - pos[t[0]], pos[t[2]] - pos[t[0]]) @ nrm[t[0]] >= 0 else (t[0], t[2], t[1]) for t in tris]
    m.part(layer).add(pos, nrm, uv_of(cell, len(pos)), colours(len(pos), sky(nrm, floor), bend, gloss, emit), tris)


def rope(m, layer, pts, radius, sides, cell=None, floor=0.6, bend=0.0, gloss=0.0, emit=0.0):
    """A tube along `pts`, all of it one plain colour of its cell."""
    pts = np.asarray(pts, float)
    n = len(pts)
    bend = np.broadcast_to(np.asarray(bend, float), (n,))
    col = lambda p, r: colours(len(p), 1.0, bend, gloss, emit)
    part = m.part(layer)
    B.tube(part, pts, np.full(n, radius), sides, col)
    nrm = part.nrm[-1]
    part.col[-1][:, 0] = sky(nrm, floor)
    if cell is not None:
        part.uv[-1] = uv_of(cell, len(part.pos[-1]))


def star(m, layer, centre, facing, up, outer, inner, depth, cell=None, gloss=0.0, emit=0.0, floor=0.7, bend=0.0, points=5):
    """A star of `points` points standing in the plane across `facing` (its tip up `up`),
    `depth` thick, its faces raised to a ridge in the middle."""
    f, upv = B.unit(facing), B.unit(up)
    right = np.cross(upv, f)
    c = np.asarray(centre, float)
    rim = []
    for i in range(points * 2):
        a = i / (points * 2) * TAU
        r = outer if i % 2 == 0 else inner
        rim.append(c + (np.sin(a) * right + np.cos(a) * upv) * r)
    tris_pos, tris_n = [], []
    for side in (1.0, -1.0):
        apex = c + f * depth * 0.5 * side
        for i in range(points * 2):
            p0, p1 = rim[i], rim[(i + 1) % (points * 2)]
            tri = [apex, p0, p1]
            n = B.unit(np.cross(p0 - apex, p1 - apex))
            if n @ (f * side) < 0:
                tri, n = [apex, p1, p0], -n
            tris_pos.append(tri)
            tris_n.append(n)
    pts = np.array(tris_pos).reshape(-1, 3)
    nrm = np.repeat(np.array(tris_n), 3, axis=0)
    idx = np.arange(len(pts)).reshape(-1, 3)
    m.part(layer).add(pts, nrm, uv_of(cell, len(pts)), colours(len(pts), sky(nrm, floor), bend, gloss, emit), idx)


def wreath(m, centre, facing, big, small, lod, seed):
    """A wreath of fir hung facing `facing`: berries in it, a red bow at its foot."""
    rng = np.random.default_rng(seed + 101)
    c, f = np.asarray(centre, float), B.unit(facing)
    torus(m, "festive", c, f, big, small, 5, (14, 10, 8)[lod], "garland", floor=0.6)
    _, u, w = ring_frame(f, UP)
    if lod < 2:
        for k in range(9):
            a = k / 9 * TAU + rng.uniform(-0.2, 0.2)
            at = c + (np.cos(a) * u + np.sin(a) * w) * big + f * small * 0.8
            ball(m, "festive", at, np.full(3, small * 0.32), 0, "red", floor=0.8, gloss=0.5)
    for s in (-1.0, 1.0):
        side = np.cross(UP, f)
        ball(m, "festive", c - u * (big + small * 0.5) + side * s * small * 0.9 + f * small, (small * 1.1, small * 0.6, small * 0.4), 1, "ribbon_red", floor=0.7, gloss=0.3)


def twinkle(rng):
    """The phase a bulb twinkles at (never 0, which is steady)."""
    return rng.uniform(0.06, 1.0)


# ---------------------------------------------------------------------------------------------
# Firs: snowy, decorated
# ---------------------------------------------------------------------------------------------


def fir(m, seed, lod, height, radius, trunk_radius, leaf_size, base=6.0, cone=0.95, branches=7, droop=0.25, step=4.6, rise=0.45, spray=1.0):
    """A fir in the shape of a cone: a straight trunk, whorls of branches sloping down, longest
    at the foot, each with sprays of needles. Returns the limbs (start, direction, reach, how
    far up), the whorls (height, reach, how far the limbs fall for a unit out, the limbs' angles)
    and the cone's reach at each height (0..1), for the snow and the decorations. `rise`: how
    far the sprays turn up from their limbs, `spray` how big they are (a snowy fir's lie flat
    along its limbs, short of their ends, under the snow)."""
    rng = np.random.default_rng(seed)
    profile = lambda t: (1.0 - np.clip(t, 0, 1)) ** cone * (0.78 + 0.22 * B.smoothstep(0.0, 0.1, t))
    centre = np.array([0.0, base + (height - base) * 0.4, 0.0])
    crown = B.Crown([(centre, (radius, (height - base) * 0.58, radius))])
    trunk = np.array([[0.0, y, 0.0] for y in np.linspace(-B.SINK, height, 14)])
    tr = trunk_radius * (1.0 - np.linspace(0, 1, 14) * 0.9)
    bark = B.bark_colour(height, base, crown, rng.random())
    B.tube(m.part("bark"), trunk, tr, B.sides(lod)(trunk_radius), bark, (36.0, 48.0))
    cards, limbs, tiers = [], [], []
    whorls = int((height - base) / step)
    for k in range(whorls):
        t = (k + 0.5) / whorls
        y = base + t * (height - base)
        reach = radius * profile(t) * rng.uniform(0.88, 1.06)
        angles = []
        for j in range(branches):
            ang = (j + rng.uniform(-0.3, 0.3)) / branches * TAU + k * 0.9
            angles.append(ang)
            out = B.unit(np.array([np.cos(ang), -droop - 0.2 * (1 - t), np.sin(ang)]))
            start = np.array([0.0, y, 0.0])
            end = start + out * reach
            limbs.append((start, out, reach, t))
            if lod == 0 and reach > 6.0:
                B.tube(m.part("bark"), [start, (start + end) / 2, end], [trunk_radius * 0.18, trunk_radius * 0.1, 0.15], 4, bark, (36.0, 48.0))
            for s in (0.25, 0.65):
                at = start + (end - start) * s
                d = B.unit(out + np.array([0.0, rise, 0.0]) + rng.normal(0, 0.15, 3))
                size = max(leaf_size * (0.55 + 0.6 * profile(t)), 4.0) * rng.uniform(0.85, 1.15) * spray
                right = B.unit(np.cross(d, UP) + rng.normal(0, 0.25, 3))
                cards.append((at, d, right, size, size * 1.15))
        tiers.append((y, reach, droop + 0.2 * (1 - t), angles))
    tip = np.array([0.0, height - leaf_size * 0.9, 0.0])
    cards.append((tip, UP, np.array([1.0, 0.0, 0.0]), leaf_size * 0.5, leaf_size))
    cards.append((tip, UP, np.array([0.0, 0.0, 1.0]), leaf_size * 0.5, leaf_size))
    cards = B.thin_cards(cards, max(int(len(cards) * B.LOD_CARDS[lod]), 8), fold=0.55)
    bend = lambda c: np.clip(c[:, 1] / height, 0, 1) ** 1.4
    B.place_cards(m, crown, cards, "needles", ao_floor=0.62, bend_of=bend, flutter=0.5, fold=0.55)
    return limbs, tiers, profile


def sway(y, height):
    """How far the wind bends a fir at height `y` (as its needles: see `fir`)."""
    return float(np.clip(y / height, 0, 1) ** 1.4)


def snow_on(m, tiers, lod, height, seed, amount=1.0, heavy=1.0, bed=2.6):
    """Snow lying on the whorls: on each a lumpy skirt along its limbs (deepest along each limb,
    reaching out further there), its rim falling to a soft lip, lying `bed` over the limbs (on
    their sprays); on `amount` of the whorls (at the coarsest level every other one) and a cap
    on the top shoot."""
    rng = np.random.default_rng(seed)
    rings, sectors = ((3, 21), (2, 14), (1, 7))[lod]
    for k, (y, reach, fall, angles) in enumerate(tiers):
        keep, spin = rng.random(), rng.uniform(0, TAU)
        if keep > amount or reach < 3.0 or (lod == 2 and k % 2):
            continue
        theta = np.linspace(0.0, TAU, sectors + 1)[:-1]
        # How near a limb each way round is (1 on one), and how far the snow reaches there.
        near = np.max([np.exp(-(((theta - a + np.pi) % TAU - np.pi) / 0.3) ** 2) for a in angles], axis=0)
        wobble = 1.0 + 0.1 * np.sin(theta * 3.0 + spin) * np.sin(theta * 5.0 + spin * 1.7)
        out = reach * (0.56 + 0.4 * near * near) * wobble * (1.04 if lod == 2 else 1.0)
        deep = (0.55 + 0.05 * reach) * heavy * (0.5 + 0.7 * near)
        pos = []
        for i in range(rings + 1):
            f = 0.12 + 0.88 * i / rings
            rho = out * f
            lift = deep * np.sin(np.pi * (0.2 + 0.75 * f)) ** 0.7
            for j in range(sectors):
                # (Over the sprays lying along the limbs: they reach this far above them.)
                pos.append((np.cos(theta[j]) * rho[j], y - fall * rho[j] + bed + lift[j], np.sin(theta[j]) * rho[j]))
        # The lip: just out from the rim and below it.
        for j in range(sectors):
            rho = out[j] * 1.04
            pos.append((np.cos(theta[j]) * rho, y - fall * rho + bed * 0.3 - 0.6 * heavy, np.sin(theta[j]) * rho))
        pos = np.array(pos)
        tris = []
        for i in range(rings + 1):
            for j in range(sectors):
                jj = (j + 1) % sectors
                a, b, c, d = i * sectors + j, i * sectors + jj, (i + 1) * sectors + jj, (i + 1) * sectors + j
                tris += [(a, c, b), (a, d, c)]
        # (The middle closed round the trunk.)
        top = len(pos)
        pos = np.vstack([pos, (0.0, y + bed + deep.mean() * 0.6, 0.0)])
        for j in range(sectors):
            tris.append((top, (j + 1) % sectors, j))
        tris = np.array(tris)
        nrm = np.zeros_like(pos)
        fn = np.cross(pos[tris[:, 1]] - pos[tris[:, 0]], pos[tris[:, 2]] - pos[tris[:, 0]])
        for q in range(3):
            np.add.at(nrm, tris[:, q], fn)
        nrm = B.unit(nrm)
        if nrm[:-1, 1].mean() < 0:
            nrm, tris = -nrm, tris[:, ::-1]
        # Open to the sky at its rim, in the shade of the whorl above near the trunk.
        rho = np.hypot(pos[:, 0], pos[:, 2]) / max(reach, 1.0)
        ao = np.clip((0.55 + 0.45 * B.smoothstep(0.1, 0.8, rho)) * (0.6 + 0.4 * np.clip(nrm[:, 1], 0, 1)), 0.3, 1.0)
        bend = np.clip(pos[:, 1] / height, 0, 1) ** 1.4
        m.part("snow").add(pos, nrm, np.zeros((len(pos), 2)), colours(len(pos), ao, bend, 0.12), tris)
    cylinder(m, "snow", (0.0, height - 7.0, 0.0), UP, 3.2 * heavy, 0.2, 7.5, (10, 8, 6)[lod], floor=0.6, bend=sway(height, height), gloss=0.1)


def snowpine(name, seed, lod, height, radius, trunk_radius=2.6, leaf_size=11.0, snow=1.0, cover=0.5):
    """A fir under snow: snow lying along `snow` of its whorls, and on its needles turned up to
    the sky (`cover` of them, in clumps, see foliage.wgsl's `snowed`), so from above it is white
    with the dark green showing between."""
    m = B.Model(name)
    _, tiers, _ = fir(m, seed, lod, height, radius, trunk_radius, leaf_size, cone=0.9, rise=0.15, spray=0.85)
    snow_on(m, tiers, lod, height, seed + 7, amount=snow)
    m.meta = {"height": float(height), "width": float(radius * 2.0), "crown": [0.0, float(height * 0.45), 0.0, float(radius)], "own": {"snow": float(cover)}}
    return m


BAUBLES = {
    "a": ["red", "gold", "silver", "red", "gold", "blue"],
    "b": ["blue", "silver", "purple", "silver", "gold"],
    "c": ["red", "gold", "green", "silver", "red", "purple", "blue"],
}
BULBS = {
    "a": ["bulb_red", "bulb_green", "bulb_blue", "bulb_amber", "bulb_pink"],
    "b": ["bulb_warm"],
    "c": ["bulb_red", "bulb_amber", "bulb_green", "bulb_blue", "bulb_white", "bulb_pink", "bulb_cyan"],
}


def decorations(m, limbs, profile, lod, height, radius, base, seed, scheme, turns=5.0, bulb_every=3.4, bauble=1.7, star_size=5.5, garland=True):
    """A fir decorated: baubles hung at the ends of its limbs, a string of bulbs wound round it
    (twinkling), tinsel beside them, a star on top (aglow)."""
    rng = np.random.default_rng(seed)
    baubles = BAUBLES[scheme]
    for k, (start, out, reach, t) in enumerate(limbs):
        pick, size = rng.integers(0, 1 << 20), rng.uniform(0.85, 1.15)
        if reach < 4.0 or k % 2 or (lod == 2 and k % 4):
            continue
        at = start + out * reach * 0.88 - UP * (bauble * 1.4)
        cell = baubles[pick % len(baubles)]
        ball(m, "festive", at, np.full(3, bauble * size * (1.25 if lod == 2 else 1.0)), 1 if lod == 0 else 0, cell, floor=0.75, bend=sway(at[1], height), gloss=0.65)
    # The string of lights: wound round the cone from its foot to near its top.
    path = []
    steps = 900
    for i in range(steps + 1):
        s = i / steps
        t = 0.04 + s * 0.88
        y = base + t * (height - base)
        r = radius * profile(t) * 0.9 + 1.0
        a = s * turns * TAU
        path.append(np.array([np.cos(a) * r, y - r * 0.18, np.sin(a) * r]))
    path = np.array(path)
    length = np.concatenate([[0.0], np.cumsum(np.linalg.norm(np.diff(path, axis=0), axis=1))])
    lights = BULBS[scheme]
    count = int(length[-1] / bulb_every)
    for i in range(count):
        phase = twinkle(rng)
        if lod == 2 and i % 2:
            continue
        p = path[np.searchsorted(length, (i + 0.5) * bulb_every)]
        ball(m, "festive", p, np.full(3, 0.75 * (1.35 if lod == 2 else 1.0)), 0, lights[i % len(lights)], floor=0.9, bend=sway(p[1], height), gloss=phase, emit=1.0)
    if garland and lod < 2:
        inner = path[::6] * np.array([0.96, 1.0, 0.96]) + np.array([0.0, -0.8, 0.0])
        rope(m, "festive", inner, 0.45, 5 if lod == 0 else 4, "gold", floor=0.8, bend=[sway(p[1], height) for p in inner], gloss=0.55)
    top = height + star_size * 0.55
    star(m, "festive", (0.0, top, 0.0), (0.0, 0.0, 1.0), UP, star_size, star_size * 0.42, star_size * 0.35, "gold", gloss=0.0, emit=0.75, bend=sway(height, height))


def festive_fir(name, seed, lod, height, radius, scheme, snow=0.0, trunk_radius=2.6, leaf_size=11.0, turns=5.0):
    """A fir decorated for Christmas, dusted with snow or not."""
    m = B.Model(name)
    limbs, tiers, profile = fir(m, seed, lod, height, radius, trunk_radius, leaf_size, cone=0.95, rise=0.3 if snow > 0.0 else 0.45)
    if snow > 0.0:
        snow_on(m, tiers, lod, height, seed + 7, amount=snow, heavy=0.6, bed=3.2)
    decorations(m, limbs, profile, lod, height, radius, 6.0, seed + 11, scheme, turns=turns)
    m.meta = {"height": float(height + 6.0), "width": float(radius * 2.0), "crown": [0.0, float(height * 0.45), 0.0, float(radius)], "own": {"snow": float(0.45 * snow)}}
    return m


def present(m, rng, centre, size, yaw, lod, wrap=None, ribbon=None):
    """A present: a box in wrapping paper, a ribbon round it both ways and a bow on top."""
    wraps = ["wrap_red", "wrap_green", "wrap_blue", "wrap_gold", "wrap_cane", "wrap_plaid"]
    ribbons = {"wrap_red": "ribbon_gold", "wrap_green": "ribbon_red", "wrap_blue": "ribbon_gold", "wrap_gold": "ribbon_red", "wrap_cane": "ribbon_red", "wrap_plaid": "ribbon_gold"}
    wrap = wrap or wraps[rng.integers(0, len(wraps))]
    ribbon = ribbon or ribbons[wrap]
    w, h, d = size
    c = np.array([centre[0], centre[1] + h / 2, centre[2]])
    block(m, "festive", c, (w / 2, h / 2, d / 2), wrap, yaw=yaw, floor=0.6, gloss=0.15, skip=("bottom",))
    band = min(w, d) * 0.12 + 0.3
    block(m, "festive", c, (band, h / 2 + 0.06, d / 2 + 0.06), ribbon, yaw=yaw, floor=0.6, gloss=0.3, skip=("bottom",))
    block(m, "festive", c, (w / 2 + 0.06, h / 2 + 0.06, band), ribbon, yaw=yaw, floor=0.6, gloss=0.3, skip=("bottom",))
    if lod < 2:
        top = c + UP * (h / 2 + band * 0.6)
        cs, sn = np.cos(yaw), np.sin(yaw)
        side = np.array([cs, 0.0, -sn])
        for s in (-1.0, 1.0):
            ball(m, "festive", top + side * s * band * 1.3, (band * 1.5, band * 0.7, band * 0.6), 1, ribbon, floor=0.7, gloss=0.3, frame=np.stack([side, UP, np.cross(side, UP)], axis=1))
        ball(m, "festive", top, np.full(3, band * 0.55), 1, ribbon, floor=0.7, gloss=0.3)


def great_fir(name, seed, lod, height=168.0, radius=40.0):
    """The village's great tree: a tall fir hung with baubles and wound round with lights, a star
    on top and presents piled round its foot."""
    m = B.Model(name)
    limbs, _, profile = fir(m, seed, lod, height, radius, 5.0, 15.0, base=10.0, cone=0.92, branches=9, step=5.2)
    decorations(m, limbs, profile, lod, height, radius, 10.0, seed + 11, "c", turns=7.0, bulb_every=3.0, bauble=2.6, star_size=11.0)
    rng = np.random.default_rng(seed + 23)
    for k in range(9):
        a = k / 9 * TAU + rng.uniform(-0.2, 0.2)
        r = rng.uniform(radius * 0.55, radius * 0.75)
        size = (rng.uniform(7, 12), rng.uniform(5, 10), rng.uniform(7, 12))
        present(m, rng, (np.cos(a) * r, 0.0, np.sin(a) * r), size, rng.uniform(0, TAU), lod)
    m.meta = {"height": float(height + 12.0), "width": float(radius * 2.0), "crown": [0.0, float(height * 0.45), 0.0, float(radius)], "own": {}}
    return m


def presents(name, seed, lod, count):
    """A pile of presents."""
    m = B.Model(name)
    rng = np.random.default_rng(seed)
    placed = []
    for k in range(count):
        size = np.array([rng.uniform(6, 13), rng.uniform(4, 11), rng.uniform(6, 12)])
        for _ in range(40):
            at = rng.uniform(-9, 9, 2)
            if all(np.hypot(*(at - p)) > (size[0] + s[0]) * 0.45 for p, s in placed):
                break
        placed.append((at, size))
        present(m, rng, (at[0], 0.0, at[1]), size, rng.uniform(-0.6, 0.6), lod)
    # One more balanced on the biggest.
    (at, size) = max(placed, key=lambda p: p[1][1] * p[1][0])
    small = size * np.array([0.55, 0.7, 0.55])
    present(m, rng, (at[0], size[1] + 0.12, at[1]), small, rng.uniform(-0.8, 0.8), lod)
    top = max(s[1] for _, s in placed) + small[1]
    m.meta = {"height": float(top), "width": 26.0, "crown": [0.0, float(top * 0.5), 0.0, 12.0], "contact": 10.0, "own": {}}
    return m


# ---------------------------------------------------------------------------------------------
# Snowmen, lanterns, ice
# ---------------------------------------------------------------------------------------------


def snowman(name, seed, lod, hat="top", scarf="knit_red"):
    """A snowman: three balls of snow, coal eyes, buttons and smile, a carrot nose, stick arms,
    a scarf and a hat (a top hat or a knitted one with a bobble)."""
    m = B.Model(name)
    rng = np.random.default_rng(seed)
    level = (2, 2, 1)[lod]
    balls = [((0.0, 6.6, 0.0), (9.0, 7.6, 9.0)), ((0.0, 17.2, 0.2), (6.6, 5.6, 6.6)), ((0.0, 25.6, 0.4), (4.7, 4.5, 4.7))]
    for k, (c, r) in enumerate(balls):
        ball(m, "snow", c, r, level, floor=0.45, gloss=0.1, rough=0.05, seed=seed + k)
    head = np.array(balls[2][0])
    hr = balls[2][1][0]
    coal = lambda p, s=0.55: ball(m, "festive", p, np.full(3, s), 1 if lod < 2 else 0, "black", floor=0.6, gloss=0.2)
    for s in (-1.0, 1.0):
        a = np.radians(24) * s
        coal(head + np.array([np.sin(a) * hr * 0.9, 1.1, np.cos(a) * hr * 0.88]))
    if lod < 2:
        for i in range(5):
            a = np.radians(-30 + i * 15)
            coal(head + np.array([np.sin(a) * hr * 0.92, -1.6 - 0.5 * np.cos(np.radians(i * 45 - 90)), np.cos(a) * hr * 0.9]), 0.38)
    mid = np.array(balls[1][0])
    for i in range(3):
        coal(mid + np.array([0.0, 2.6 - i * 2.6, balls[1][1][2] * 0.97 - abs(i - 1) * 0.15]), 0.62)
    # The nose, a carrot pointing south.
    cylinder(m, "festive", head + np.array([0.0, 0.0, hr * 0.85]), (0.0, -0.15, 1.0), 0.85, 0.08, 4.2, (8, 6, 5)[lod], "carrot", floor=0.7, cap0=True)
    # Stick arms, a twig forked at each hand.
    for s in (-1.0, 1.0):
        sh = mid + np.array([s * balls[1][1][0] * 0.8, 1.6, 0.0])
        hand = sh + np.array([s * 9.0, 5.5 + rng.uniform(-1, 1), rng.uniform(-1.5, 1.5)])
        rope(m, "bark", [sh, (sh + hand) / 2 + np.array([0, 0.6, 0]), hand], 0.42, 4, floor=0.7)
        if lod < 2:
            for f in (-1.0, 1.0):
                rope(m, "bark", [hand, hand + np.array([s * 2.2, 1.8 + f * 0.8, f * 1.4])], 0.25, 3, floor=0.7)
    # The scarf round the neck and its end hanging down the front.
    neck = (mid + head) / 2 + np.array([0.0, 0.6, 0.0])
    torus(m, "festive", neck, UP, 4.4, 1.25, (6, 5, 4)[lod], (16, 12, 8)[lod], scarf, floor=0.6)
    block(m, "festive", neck + np.array([2.0, -3.2, 4.7]), (1.2, 3.4, 0.45), scarf, yaw=-0.25, floor=0.6)
    top = head + UP * (hr * 0.82)
    if hat == "top":
        cylinder(m, "festive", top + UP * -0.2, UP, 5.0, 5.0, 0.6, (14, 10, 8)[lod], "black", floor=0.5, cap0=True)
        cylinder(m, "festive", top + UP * 0.4, UP, 3.3, 3.5, 5.6, (14, 10, 8)[lod], "black", floor=0.6, gloss=0.15)
        cylinder(m, "festive", top + UP * 0.6, UP, 3.38, 3.42, 1.1, (14, 10, 8)[lod], "ribbon_red", floor=0.7, cap1=False)
        height = top[1] + 6.2
    else:
        ball(m, "festive", top + UP * -0.6, (hr * 0.98, hr * 0.75, hr * 0.98), level, scarf, floor=0.6, frame=None)
        torus(m, "festive", top + UP * -1.0, UP, hr * 0.92, 0.8, 5, (16, 12, 8)[lod], "wool", floor=0.7)
        ball(m, "festive", top + UP * (hr * 0.75 + 0.6), np.full(3, 1.4), 1, "wool", floor=0.8)
        height = top[1] + hr * 0.75 + 2.0
    m.meta = {"height": float(height), "width": 18.0, "crown": [0.0, 14.0, 0.0, 9.0], "contact": 8.0, "own": {}}
    return m


def lantern(name, seed, lod, height=34.0):
    """A street lamp: an iron post on a stepped foot, a wreath with a red bow on it, and a
    glazed lantern on top, lit."""
    m = B.Model(name)
    sides = (12, 8, 6)[lod]
    cylinder(m, "festive", (0, -1.0, 0), UP, 2.4, 2.0, 3.2, sides, "iron", floor=0.5, gloss=0.3)
    cylinder(m, "festive", (0, 2.2, 0), UP, 1.2, 0.8, height - 9.0, sides, "iron", floor=0.55, gloss=0.3)
    top = height - 7.0
    block(m, "festive", (0, top - 0.6, 0), (2.8, 0.6, 2.8), "iron", floor=0.5, gloss=0.3)
    # The glass, lit from within, between four corner posts.
    block(m, "festive", (0, top + 3.0, 0), (2.4, 3.0, 2.4), "bulb_warm", floor=1.0, emit=0.9, skip=("bottom", "top"))
    if lod < 2:
        for sx in (-1, 1):
            for sz in (-1, 1):
                block(m, "festive", (sx * 2.5, top + 3.0, sz * 2.5), (0.35, 3.1, 0.35), "iron", floor=0.5)
    cylinder(m, "festive", (0, top + 6.0, 0), UP, 3.8, 0.4, 3.0, 4, "iron", floor=0.6, gloss=0.3, cap0=True)
    ball(m, "festive", (0, top + 9.3, 0), np.full(3, 0.7), 1, "brass", floor=0.8, gloss=0.5)
    if lod < 2:
        wreath(m, (0, 15.0, 1.6), (0, 0, 1), 3.0, 1.0, lod, seed)
    m.meta = {"height": float(height + 3.0), "width": 8.0, "crown": [0.0, float(top + 3.0), 0.0, 3.0], "contact": 2.4, "own": {}}
    return m


def ice_star(name, seed, lod):
    """An ice sculpture: a great star of ice standing on a block of ice, lit from within by night."""
    m = B.Model(name)
    block(m, "crystal", (0, 2.5, 0), (7.0, 2.5, 5.0), floor=0.7, gloss=0.6, emit=0.35)
    lean = np.array([0.0, 0.42, 0.91])
    star(m, "crystal", (0, 15.5, -1.0), lean, B.unit(np.cross(lean, np.array([1.0, 0.0, 0.0]))), 14.5, 6.2, 5.0, gloss=0.75, emit=0.45)
    m.meta = {"height": 33.0, "width": 26.0, "crown": [0.0, 19.0, 0.0, 12.0], "contact": 9.0,
              "own": {"leaf": [float(x) for x in linear((150, 196, 222))], "light": [0.55, 0.85, 1.0]}}
    return m


def ice_tree(name, seed, lod):
    """An ice sculpture: a fir carved of ice in three tiers, a star of ice on top."""
    m = B.Model(name)
    rng = np.random.default_rng(seed)
    sides = (9, 7, 5)[lod]
    block(m, "crystal", (0, 2.0, 0), (7.0, 2.0, 7.0), floor=0.7, gloss=0.6, emit=0.35)
    for k, (y, r, h) in enumerate(((4.0, 9.5, 12.0), (12.0, 7.4, 11.0), (19.5, 5.2, 10.0))):
        tris = B.prism(np.array([0.0, y + 3.0, 0.0]), np.array([0.0, 1.0, 0.0]), h, r, sides, h * 0.98, rng, jag=0.05)
        # (A cone: its base ring at the start, its tip at the top.)
        B.faceted(m.part("crystal"), tris, lambda p: 0.75 + 0.25 * np.clip((p[:, 1] - y) / h, 0, 1), 0.7)
        m.part("crystal").col[-1][:, 3] = 0.45
    star(m, "crystal", (0, 33.0, 0), (0, 0, 1), UP, 4.2, 1.8, 1.8, gloss=0.75, emit=0.6)
    m.meta = {"height": 37.5, "width": 20.0, "crown": [0.0, 16.0, 0.0, 9.0], "contact": 7.0,
              "own": {"leaf": [float(x) for x in linear((150, 196, 222))], "light": [0.55, 0.85, 1.0]}}
    return m


def ice_lanterns(name, seed, lod):
    """Lanterns of ice: three blocks of ice, a candle's warm light within each."""
    m = B.Model(name)
    rng = np.random.default_rng(seed)
    for k, (x, z, s) in enumerate(((-4.0, 1.5, 1.0), (4.2, 2.5, 0.8), (0.5, -4.0, 0.9))):
        h = 8.0 * s
        block(m, "crystal", (x, h / 2, z), (3.2 * s, h / 2, 3.2 * s), yaw=rng.uniform(-0.5, 0.5), floor=0.75, gloss=0.6, emit=0.75)
    m.meta = {"height": 8.0, "width": 16.0, "crown": [0.0, 4.0, 0.0, 7.0], "contact": 7.0,
              "own": {"leaf": [float(x) for x in linear((176, 206, 222))], "light": [1.0, 0.62, 0.28]}}
    return m


def snowrock(name, seed, lod, size, planes=12, sharp=1.2, rough=0.08):
    """A boulder under snow: TA Reborn's rock (build.py's), snow lying on its faces turned up to
    the sky, a little proud of them."""
    m = B.rock(name, seed, lod, size, planes=planes, sharp=sharp, rough=rough)
    rock = m.parts["rock"]
    pos, nrm, _, col, idx = rock.arrays()
    # The snow: the rock's own faces where any corner faces the sky, lifted the more the more it
    # does, and sunk just under the rock where none does, so its edge runs smoothly across the
    # faces where the two meet (not along them in steps).
    deep = np.clip((nrm[:, 1] - 0.3) / 0.55, 0.0, 1.0) ** 0.8
    top = idx[deep[idx].max(axis=1) > 0.0]
    if len(top):
        used = np.unique(top)
        remap = -np.ones(len(pos), np.int64)
        remap[used] = np.arange(len(used))
        lift = nrm[used] * (-0.45 + (0.95 + 0.06 * size[1]) * deep[used])[:, None]
        m.part("snow").add(pos[used] + lift, nrm[used], np.zeros((len(used), 2)), colours(len(used), np.clip(col[used, 0] + 0.15, 0, 1), 0.0, 0.12), remap[top])
    m.meta["own"] = {"bark": "rock"}
    return m


# ---------------------------------------------------------------------------------------------
# Cottages, a sleigh, a clock tower
# ---------------------------------------------------------------------------------------------


def window(m, at, facing, w, h, lod, shutters=None, sill=True):
    """A lit window on a wall facing `facing` (+x, -x, +z or -z: a unit vector), its middle at
    `at`, with a frame, a sill of snow and shutters if given."""
    f = np.asarray(facing, float)
    right = np.cross(UP, f)
    at = np.asarray(at, float) + f * 0.12
    panel(m, "festive", at - right * w / 2 + UP * h / 2, right * w, -UP * h, "window", floor=1.0, emit=0.85)
    if lod < 2:
        block(m, "festive", at - UP * (h / 2 + 0.4) + f * 0.6, (w / 2 + 0.8, 0.4, 0.8) if abs(f[2]) > 0.5 else (0.8, 0.4, w / 2 + 0.8), "white", floor=0.6)
        block(m, "snow", at - UP * (h / 2 - 0.15) + f * 0.7, (w / 2 + 0.6, 0.35, 0.7) if abs(f[2]) > 0.5 else (0.7, 0.35, w / 2 + 0.6), floor=0.7, gloss=0.1)
        if shutters:
            for left in (at - right * (w / 2 + 0.3 + w * 0.45), at + right * (w / 2 + 0.3)):
                panel(m, "festive", left + UP * h / 2 + f * 0.05, right * w * 0.45, -UP * h, shutters, floor=0.7)


def cottage(name, seed, lod, walls="planks", trim="beams", lights=None, shutters=None, dormer=True, roof="shingles", cover=0.62):
    """A cottage under snow: walls of `walls` on a stone footing, lit windows (two and a door
    with a wreath to the south, one on each other side), a steep roof of slates deep in snow
    with a lit dormer in its southern slope (TA's camera sees the roof most), icicles along its
    eaves, a chimney; `lights` (bulbs) strung along its southern eave. The snow lies over the
    upper `cover` of each slope, its roof of `roof` (tiles, slates) showing below."""
    m = B.Model(name)
    rng = np.random.default_rng(seed)
    hx, hz, wall = 22.0, 17.0, 25.0
    ridge, over = 40.0, 3.5
    block(m, "festive", (0, 0.2, 0), (hx + 1.2, 2.2, hz + 1.2), "granite", tile=8.0, floor=0.5, skip=("bottom",))
    block(m, "festive", (0, 2.4 + (wall - 2.4) / 2, 0), (hx, (wall - 2.4) / 2, hz), walls, tile=11.0, floor=0.55, skip=("bottom", "top"))
    # Corner posts and a beam under the eaves.
    if lod < 2:
        for sx in (-1, 1):
            for sz in (-1, 1):
                block(m, "festive", (sx * (hx + 0.4), 12.0, sz * (hz + 0.4)), (0.8, 10.0, 0.8), trim, floor=0.55)
    # The gables, east and west.
    for s in (-1.0, 1.0):
        x = s * hx
        pts = np.array([[x, wall, -hz], [x, wall, hz], [x, ridge, 0.0]])
        n = np.array([s, 0.0, 0.0])
        uv = np.array([cell_uv(walls, 0.0, 1.0), cell_uv(walls, 1.0, 1.0), cell_uv(walls, 0.5, 0.0)])
        tri = [(0, 1, 2)] if np.cross(pts[1] - pts[0], pts[2] - pts[0]) @ n > 0 else [(0, 2, 1)]
        m.part("festive").add(pts, np.repeat(n[None], 3, axis=0), uv, colours(3, 0.75), tri)
    # The roof: a slab of slates each way, snow over it, a lip of snow along the eaves.
    eave_y = wall - over * (ridge - wall) / hz
    for s in (-1.0, 1.0):
        top_l = np.array([-hx - over, ridge, 0.0])
        down = np.array([0.0, eave_y - ridge, s * (hz + over)])
        across = np.array([2 * (hx + over), 0.0, 0.0])
        n = B.unit(np.cross(down, across)) if s > 0 else B.unit(np.cross(across, down))
        if n[1] < 0:
            n = -n
        lift = n * 1.2
        o, du = (top_l, across) if s > 0 else (top_l + across, -across)
        # The roof below the snow, and the snow over the rest, a little proud of it, a lip of
        # snow along its edge.
        panel(m, "festive", o + lift + down * cover, du, down * (1.0 - cover), roof, 5, 1, floor=0.6, gloss=0.2)
        panel(m, "snow", o + lift + n * 1.0, du, down * cover, None, 1, 1, floor=0.85, gloss=0.12)
        edge = o + lift + n * 0.5 + down * cover + np.array([0.0, 0.0, 0.0])
        cylinder(m, "snow", (-hx - over - 0.3, edge[1], edge[2]), (1, 0, 0), 1.1, 1.1, 2 * (hx + over) + 0.6, (8, 6, 4)[lod], floor=0.75, gloss=0.1, cap0=True)
        panel(m, "festive", o + down + lift, du, -lift, roof, 5, 1, floor=0.5)
        panel(m, "festive", o + down, du, -down, "beams", 4, 2, floor=0.35)
        for e in (0.0, 1.0):
            x = -hx - over + e * across[0]
            q = np.array([[x, ridge, 0.0], [x, eave_y, s * (hz + over)], [x, eave_y + lift[1], s * (hz + over) + lift[2]], [x, ridge + lift[1], lift[2]]])
            nn = np.array([-1.0 if e == 0.0 else 1.0, 0.0, 0.0])
            tris = [(0, 1, 2), (0, 2, 3)] if np.cross(q[1] - q[0], q[2] - q[0]) @ nn > 0 else [(0, 2, 1), (0, 3, 2)]
            m.part("festive").add(q, np.repeat(nn[None], 4, axis=0), uv_of(roof, 4), colours(4, 0.6), tris)
        eave = np.array([0.0, eave_y + lift[1] * 0.9, s * (hz + over) + lift[2] * 0.6])
        if lod < 2 and s > 0:
            for k in range(14):
                x = -hx - over + (k + rng.uniform(0.2, 0.8)) * 2 * (hx + over) / 14
                length = rng.uniform(1.5, 4.5)
                cylinder(m, "festive", (x, eave_y - 0.4, eave[2] - 0.2), (0, -1, 0), 0.5, 0.02, length, 5, "ice", floor=0.8, gloss=0.6)
    cylinder(m, "snow", (-hx - over - 0.6, ridge + 2.0, 0.0), (1, 0, 0), 1.6, 1.6, 2 * (hx + over) + 1.2, (8, 6, 4)[lod], floor=0.8, gloss=0.1, cap0=True)
    # (The gables' tops, under the ridge's snow.)
    # The chimney, on the north slope, capped with snow.
    block(m, "festive", (10.0, 39.0, -6.0), (3.0, 11.0, 3.0), "brick", tile=6.0, floor=0.55, skip=("bottom",))
    block(m, "snow", (10.0, 50.6, -6.0), (3.5, 0.9, 3.5), floor=0.8, gloss=0.1)
    if dormer:
        # The dormer: a gabled box out of the southern slope, its window lit.
        roof_at = lambda z: ridge - z * (ridge - wall) / hz
        zf, zb, dw, top, apex = hz * 0.55, 1.5, 5.5, ridge - 1.6, ridge + 2.6
        y0 = roof_at(zf) - 1.0
        panel(m, "festive", (-dw, top, zf), (2 * dw, 0.0, 0.0), (0.0, y0 - top, 0.0), walls, 1, 1, floor=0.6)
        for sx in (-1.0, 1.0):
            pts = np.array([[sx * dw, y0, zf], [sx * dw, top, zf], [sx * dw, top, zb]])
            n = np.array([sx, 0.0, 0.0])
            tri = [(0, 1, 2)] if np.cross(pts[1] - pts[0], pts[2] - pts[0]) @ n > 0 else [(0, 2, 1)]
            m.part("festive").add(pts, np.repeat(n[None], 3, axis=0), uv_of(walls, 3), colours(3, 0.6), tri)
        gable = np.array([[-dw, top, zf], [dw, top, zf], [0.0, apex, zf]])
        tri = [(0, 1, 2)] if np.cross(gable[1] - gable[0], gable[2] - gable[0])[2] > 0 else [(0, 2, 1)]
        m.part("festive").add(gable, np.repeat(np.array([[0.0, 0.0, 1.0]]), 3, axis=0), uv_of(trim, 3), colours(3, 0.65), tri)
        window(m, (0.0, (y0 + top) / 2 + 0.8, zf), (0, 0, 1), 4.6, 4.4, lod, None, sill=False)
        for sx in (-1.0, 1.0):
            # Its roof, deep in snow too.
            o = np.array([0.0, apex + 1.4, zf + 1.2])
            du = np.array([sx * (dw + 1.4), top - apex - 0.4, 0.0])
            dv = np.array([0.0, 0.0, zb - zf - 1.2])
            if np.cross(dv, du)[1] < 0:
                du, dv, o = dv, du, o
            panel(m, "snow", o, du, dv, None, 1, 1, floor=0.85, gloss=0.12)
    # Windows and the door.
    for x in (-12.0, 12.0):
        window(m, (x, 13.5, hz), (0, 0, 1), 6.5, 8.0, lod, shutters)
    for s in (-1.0, 1.0):
        window(m, (s * hx, 13.5, 0.0), (s, 0, 0), 6.0, 7.0, lod, None)
    window(m, (-6.0, 13.5, -hz), (0, 0, -1), 6.0, 7.0, lod, None)
    panel(m, "festive", (-4.0, 15.5, hz + 0.12), (8.0, 0.0, 0.0), (0.0, -13.5, 0.0), "door", floor=0.7)
    if lod < 2:
        wreath(m, (0.0, 11.5, hz + 0.9), (0, 0, 1), 2.4, 0.9, lod, seed)
        block(m, "snow", (0.0, 0.0, hz + 4.0), (5.0, 0.5, 3.5), floor=0.8)
    if lights:
        rng2 = np.random.default_rng(seed + 3)
        for k in range(18):
            x = -hx - over + (k + 0.5) * 2 * (hx + over) / 18
            sag = 0.9 * np.sin((k % 3 + 0.5) / 3 * np.pi)
            if lod == 2 and k % 2:
                rng2.random()
                continue
            ball(m, "festive", (x, eave_y - 1.0 - sag, hz + over + 0.5), np.full(3, 0.7), 0, lights[k % len(lights)], floor=1.0, gloss=twinkle(rng2), emit=1.0)
    m.meta = {"height": 52.0, "width": 50.0, "crown": [0.0, 25.0, 0.0, 24.0], "contact": 24.0, "own": {}}
    return m


def sleigh(name, seed, lod):
    """A sleigh: a red body with gold trim and a high curled front, brass runners, a seat, and a
    sack of presents in the back (its length along x, its front east)."""
    m = B.Model(name)
    rng = np.random.default_rng(seed)
    sides = (8, 6, 4)[lod]
    # Runners: a rail each side, curling up at the front.
    for z in (-6.5, 6.5):
        pts = [(-15.0, 1.0, z), (8.0, 1.0, z), (13.0, 2.5, z), (16.0, 6.5, z), (15.0, 9.5, z), (12.5, 9.0, z)]
        pts = B.curve(pts[0], pts[1], 0.0, 4).tolist() + [list(p) for p in pts[2:]]
        rope(m, "festive", pts, 0.65, sides, "brass", floor=0.6, gloss=0.5)
        for x in (-9.0, 0.0, 7.0):
            block(m, "festive", (x, 3.0, z), (0.5, 2.0, 0.5), "brass", floor=0.6, gloss=0.4)
    block(m, "festive", (-1.0, 7.5, 0.0), (11.0, 2.5, 7.5), "red", floor=0.55, gloss=0.35)
    block(m, "festive", (-1.0, 10.2, 0.0), (11.3, 0.4, 7.8), "gold", floor=0.7, gloss=0.5, skip=("bottom",))
    # The high front, and the seat's back.
    block(m, "festive", (9.5, 11.0, 0.0), (1.2, 4.5, 7.2), "red", floor=0.6, gloss=0.35)
    cylinder(m, "festive", (10.7, 15.5, -7.2), (0, 0, 1), 1.5, 1.5, 14.4, sides, "gold", floor=0.7, gloss=0.5, cap0=True)
    block(m, "festive", (-4.0, 12.5, 0.0), (1.0, 4.0, 7.0), "red", floor=0.6, gloss=0.35)
    block(m, "festive", (1.5, 11.0, 0.0), (4.5, 1.0, 6.5), "knit_green", floor=0.6)
    # The sack, full, and presents.
    ball(m, "festive", (-7.5, 14.0, 0.0), (5.5, 5.2, 5.8), (2, 1, 1)[lod], "burlap", floor=0.5, rough=0.08, seed=seed)
    for k in range(3):
        present(m, rng, (-7.5 + rng.uniform(-3, 3), 17.0 + k * 0.5, rng.uniform(-3, 3)), (rng.uniform(4, 6), rng.uniform(3, 5), rng.uniform(4, 6)), rng.uniform(0, TAU), lod)
    m.meta = {"height": 24.0, "width": 34.0, "crown": [0.0, 10.0, 0.0, 14.0], "contact": 14.0, "length": 34.0, "breadth": 16.0, "own": {}}
    return m


def clock_tower(name, seed, lod):
    """A clock tower: a square shaft of stone on a plinth, ledges with snow on them, a clock
    face lit on every side (at midnight), a belfry, a slate spire and a gilded ball at its top."""
    m = B.Model(name)
    half = 15.0
    block(m, "festive", (0, 1.5, 0), (half + 3.0, 3.5, half + 3.0), "granite", tile=8.0, floor=0.5, skip=("bottom",))
    block(m, "festive", (0, 5.0 + 37.5, 0), (half, 37.5, half), "wall_stone", tile=9.0, floor=0.55, skip=("bottom", "top"))
    for y in (30.0, 60.0):
        block(m, "festive", (0, y, 0), (half + 1.2, 1.0, half + 1.2), "granite", tile=8.0, floor=0.6)
        block(m, "snow", (0, y + 1.3, 0), (half + 1.0, 0.45, half + 1.0), floor=0.8, gloss=0.1, skip=("bottom",))
    # Tall dark windows up the shaft.
    for f in ((0, 0, 1), (1, 0, 0), (-1, 0, 0), (0, 0, -1)):
        f = np.array(f, float)
        right = np.cross(UP, f)
        for y in (17.0, 46.0):
            at = f * (half + 0.1) + UP * y
            panel(m, "festive", at - right * 2.5 + UP * 6.0, right * 5.0, -UP * 12.0, "glass_dark", floor=0.6, gloss=0.4)
    # The clock stage, a little wider, a face on each side.
    block(m, "festive", (0, 92.0, 0), (half + 1.5, 12.0, half + 1.5), "wall_stone", tile=9.0, floor=0.6)
    for f in ((0, 0, 1), (1, 0, 0), (-1, 0, 0), (0, 0, -1)):
        f = np.array(f, float)
        cylinder(m, "festive", f * (half + 1.5) + UP * 92.0, f, 10.0, 10.0, 0.5, (32, 20, 12)[lod], "clock", floor=1.0, emit=0.8, cap0=False, cap1=True, cap_cell="clock", up=UP)
        cylinder(m, "festive", f * (half + 1.5) + UP * 92.0, f, 11.0, 10.4, 0.9, (32, 20, 12)[lod], "brass", floor=0.8, gloss=0.5, cap1=False)
    block(m, "festive", (0, 105.0, 0), (half + 2.4, 1.2, half + 2.4), "granite", tile=8.0, floor=0.6)
    block(m, "snow", (0, 106.6, 0), (half + 2.2, 0.5, half + 2.2), floor=0.8, gloss=0.1, skip=("bottom",))
    # The belfry: corner piers round dark openings.
    bh = 12.0
    for sx in (-1, 1):
        for sz in (-1, 1):
            block(m, "festive", (sx * (half - 2.0), 107.0 + bh / 2, sz * (half - 2.0)), (2.5, bh / 2, 2.5), "wall_stone", tile=6.0, floor=0.6)
    block(m, "festive", (0, 107.0 + bh / 2, 0), (half - 3.5, bh / 2, half - 3.5), "glass_dark", floor=0.3, skip=("bottom",))
    block(m, "festive", (0, 107.0 + bh + 1.0, 0), (half + 0.5, 1.0, half + 0.5), "granite", floor=0.6)
    # The spire: a four-sided pyramid of slate, a gilded ball and spike.
    y0, y1 = 107.0 + bh + 2.0, 170.0
    corners = [np.array([sx * (half + 0.5), y0, sz * (half + 0.5)]) for sx, sz in ((-1, -1), (1, -1), (1, 1), (-1, 1))]
    apex = np.array([0.0, y1, 0.0])
    for i in range(4):
        a, b = corners[i], corners[(i + 1) % 4]
        n = B.unit(np.cross(b - a, apex - a))
        if n @ np.array([(a + b)[0], 0.0, (a + b)[2]]) < 0:
            n = -n
        q = np.array([a, b, apex])
        tri = [(0, 1, 2)] if np.cross(q[1] - q[0], q[2] - q[0]) @ n > 0 else [(0, 2, 1)]
        uv = np.array([cell_uv("shingles", 0.0, 1.0), cell_uv("shingles", 1.0, 1.0), cell_uv("shingles", 0.5, 0.0)])
        m.part("festive").add(q, np.repeat(n[None], 3, axis=0), uv, colours(3, 0.7), tri)
    ball(m, "festive", (0, y1 + 2.0, 0), np.full(3, 2.2), (2, 1, 1)[lod], "gold", floor=0.8, gloss=0.6)
    cylinder(m, "festive", (0, y1 + 3.6, 0), UP, 0.5, 0.05, 7.0, 5, "gold", floor=0.8, gloss=0.6)
    m.meta = {"height": 181.0, "width": 36.0, "crown": [0.0, 90.0, 0.0, 18.0], "contact": 18.0, "own": {}}
    return m


# ---------------------------------------------------------------------------------------------
# New Year: fireworks, lanterns on a line
# ---------------------------------------------------------------------------------------------


def fireworks(name, seed, lod, kind="mortars"):
    """Fireworks waiting for midnight: a trestle of mortar tubes in a fan (`mortars`), or a
    crate of rockets on their sticks (`rockets`)."""
    m = B.Model(name)
    rng = np.random.default_rng(seed)
    sides = (10, 7, 5)[lod]
    if kind == "mortars":
        for x in (-12.0, 12.0):
            block(m, "festive", (x, 4.0, 0.0), (0.8, 4.0, 4.5), "beams", floor=0.5)
        block(m, "festive", (0.0, 7.0, 0.0), (13.5, 0.8, 4.8), "planks", floor=0.55)
        for k in range(7):
            x = -10.5 + k * 3.5
            tilt = (k - 3) * 0.06
            cell = "tube_red" if k % 2 == 0 else "tube_blue"
            cylinder(m, "festive", (x, 7.8, rng.uniform(-1.2, 1.2)), (tilt, 1.0, rng.uniform(-0.05, 0.05)), 1.5, 1.4, 9.0 + rng.uniform(-1, 1.5), sides, cell, floor=0.6, cap1=True, cap_cell="black")
        rope(m, "festive", [(-12.0, 9.0, 3.0), (0.0, 8.4, 3.5), (12.0, 9.0, 3.0)], 0.18, 3, "black", floor=0.6)
        height = 19.0
    else:
        block(m, "festive", (0.0, 4.0, 0.0), (8.0, 4.0, 5.0), "planks", tile=6.0, floor=0.55)
        block(m, "festive", (0.0, 8.2, 0.0), (8.2, 0.3, 5.2), "beams", floor=0.6, skip=("bottom",))
        for k in range(9):
            x = -6.0 + (k % 3) * 6.0 + rng.uniform(-0.8, 0.8)
            z = -3.0 + (k // 3) * 3.0 + rng.uniform(-0.5, 0.5)
            lean = rng.uniform(-0.12, 0.12, 2)
            axis = B.unit(np.array([lean[0], 1.0, lean[1]]))
            base = np.array([x, 6.0, z])
            rope(m, "festive", [base, base + axis * 20.0], 0.22, 3, "wood_dark", floor=0.6)
            body = base + axis * 20.0
            cell = ["tube_red", "tube_blue", "gold"][k % 3]
            cylinder(m, "festive", body, axis, 1.1, 1.1, 5.5, sides, cell, floor=0.65)
            cylinder(m, "festive", body + axis * 5.5, axis, 1.3, 0.05, 2.6, sides, "red" if k % 2 else "silver", floor=0.7, gloss=0.3)
        height = 36.0
    m.meta = {"height": float(height), "width": 28.0, "crown": [0.0, float(height * 0.5), 0.0, 12.0], "contact": 12.0, "own": {}}
    return m


def lantern_line(name, seed, lod):
    """Paper lanterns on a line between two poles (east and west), lit."""
    m = B.Model(name)
    sides = (8, 6, 4)[lod]
    for x in (-21.0, 21.0):
        cylinder(m, "festive", (x, -1.0, 0.0), UP, 0.9, 0.7, 28.0, sides, "wood_dark", floor=0.55)
        ball(m, "festive", (x, 27.4, 0.0), np.full(3, 1.0), 1, "brass", floor=0.8, gloss=0.5)
    string = [np.array([x, 25.0 - 6.0 * (1.0 - (x / 21.0) ** 2), 0.0]) for x in np.linspace(-21.0, 21.0, 25)]
    rope(m, "festive", string, 0.2, 3, "black", floor=0.6)
    for k in range(7):
        x = -15.0 + k * 5.0
        y = 25.0 - 6.0 * (1.0 - (x / 21.0) ** 2) - 3.6
        cell = "lantern_red" if k % 2 == 0 else "lantern_gold"
        ball(m, "festive", (x, y, 0.0), (2.1, 2.8, 2.1), 1 if lod < 2 else 0, cell, floor=1.0, emit=0.85)
        if lod < 2:
            for s in (-1.0, 1.0):
                cylinder(m, "festive", (x, y + s * 2.6, 0.0), (0, s, 0), 1.1, 0.9, 0.6, sides, "black", floor=0.6)
    m.meta = {"height": 29.0, "width": 44.0, "crown": [0.0, 20.0, 0.0, 20.0], "contact": 2.0, "length": 44.0, "breadth": 4.0, "own": {}}
    return m


# ---------------------------------------------------------------------------------------------
# The catalogue
# ---------------------------------------------------------------------------------------------


def catalogue(build):
    """(name, builder(lod) -> Model) for every festive model; `build` is build.py."""
    global B
    B = build
    out = []

    def add(name, make, **kw):
        out.append((name, lambda lod: make(name, kw["seed"], lod, **{k: v for k, v in kw.items() if k != "seed"})))

    # Firs under snow, about as tall as the game stands TA's conifers (two Commanders).
    add("snowpine_a", snowpine, seed=301, height=84.0, radius=17.0, cover=0.5)
    add("snowpine_b", snowpine, seed=302, height=72.0, radius=15.5, cover=0.42)
    add("snowpine_c", snowpine, seed=303, height=96.0, radius=19.0, snow=0.85, cover=0.6)
    # Decorated firs, and the village's great one.
    add("festive_fir_a", festive_fir, seed=311, height=80.0, radius=18.0, scheme="a")
    add("festive_fir_b", festive_fir, seed=312, height=86.0, radius=19.0, scheme="b", snow=0.6)
    add("great_fir_a", great_fir, seed=313)
    add("snowman_a", snowman, seed=321, hat="top", scarf="knit_red")
    add("snowman_b", snowman, seed=322, hat="knit", scarf="knit_green")
    add("presents_a", presents, seed=331, count=3)
    add("presents_b", presents, seed=332, count=5)
    add("lantern_a", lantern, seed=341)
    add("ice_star_a", ice_star, seed=351)
    add("ice_tree_a", ice_tree, seed=352)
    add("ice_lanterns_a", ice_lanterns, seed=353)
    add("cottage_a", cottage, seed=361, walls="planks", trim="beams", shutters="shutter", roof="tiles_red")
    add("cottage_b", cottage, seed=362, walls="wall_stone", trim="beams", dormer=False, roof="shingles", cover=0.7)
    add("cottage_c", cottage, seed=363, walls="brick", trim="beams", lights=BULBS["c"], roof="slate_green", cover=0.55)
    add("sleigh_a", sleigh, seed=371)
    add("clock_tower_a", clock_tower, seed=381)
    add("fireworks_a", fireworks, seed=391, kind="mortars")
    add("fireworks_b", fireworks, seed=392, kind="rockets")
    add("lantern_line_a", lantern_line, seed=393)
    # Boulders under snow.
    add("snowrock_a", snowrock, seed=401, size=(30.0, 20.0, 26.0))
    add("snowrock_b", snowrock, seed=402, size=(26.0, 24.0, 22.0), planes=10, sharp=1.8)
    add("snowrock_c", snowrock, seed=403, size=(42.0, 26.0, 36.0), planes=14, sharp=1.0)
    return out


# ---------------------------------------------------------------------------------------------
# The features (assets/reborn/features/festive/festive.tdf, written by sprites.py)
# ---------------------------------------------------------------------------------------------

# Each feature: its name in the game's data, the model it is drawn from (its painting and its
# stand-in), its world, description, footprint (cells), height (TA's: what weapons strike),
# frames of its painting (the twinkling ones play through them), and the rest of its TDF.
# `rebornlight`: the light it casts by night (linear colour, reach, height over its foot).
FEATURES = [
    ("SnowPine1", "snowpine_a", "Ice", "Snowy Pine", (1, 1), 40, 1, {"reclaimable": 1, "energy": 200, "damage": 500, "hitdensity": 10}),
    ("SnowPine2", "snowpine_b", "Ice", "Snowy Pine", (1, 1), 36, 1, {"reclaimable": 1, "energy": 180, "damage": 450, "hitdensity": 10}),
    ("SnowPine3", "snowpine_c", "Ice", "Snowy Pine", (1, 1), 44, 1, {"reclaimable": 1, "energy": 220, "damage": 550, "hitdensity": 10}),
    ("FestiveFir1", "festive_fir_a", "All Worlds", "Decorated Fir", (1, 1), 40, 16, {"reclaimable": 1, "energy": 250, "damage": 500, "hitdensity": 10, "rebornlight": "1.2 0.75 0.45 80 30"}),
    ("FestiveFir2", "festive_fir_b", "Ice", "Decorated Fir", (1, 1), 42, 16, {"reclaimable": 1, "energy": 250, "damage": 500, "hitdensity": 10, "rebornlight": "1.3 1.0 0.7 80 30"}),
    ("GreatFir", "great_fir_a", "All Worlds", "Great Decorated Fir", (4, 4), 80, 16, {"indestructible": 1, "hitdensity": 20, "rebornlight": "2.4 1.5 0.9 220 70"}),
    ("Snowman1", "snowman_a", "Ice", "Snowman", (1, 1), 24, 1, {"damage": 200, "hitdensity": 5}),
    ("Snowman2", "snowman_b", "Ice", "Snowman", (1, 1), 24, 1, {"damage": 200, "hitdensity": 5}),
    ("Presents1", "presents_a", "All Worlds", "Presents", (2, 2), 10, 1, {"reclaimable": 1, "metal": 25, "damage": 150, "hitdensity": 5}),
    ("Presents2", "presents_b", "All Worlds", "Presents", (2, 2), 10, 1, {"reclaimable": 1, "metal": 40, "damage": 200, "hitdensity": 5}),
    ("StreetLantern", "lantern_a", "All Worlds", "Street Lantern", (1, 1), 30, 1, {"damage": 300, "hitdensity": 5, "rebornlight": "1.6 1.05 0.5 90 30"}),
    ("IceStar", "ice_star_a", "Ice", "Ice Sculpture", (2, 2), 30, 1, {"damage": 600, "hitdensity": 10, "rebornlight": "0.45 0.75 1.1 70 18"}),
    ("IceFir", "ice_tree_a", "Ice", "Ice Sculpture", (2, 2), 32, 1, {"damage": 600, "hitdensity": 10, "rebornlight": "0.45 0.75 1.1 70 18"}),
    ("IceLanterns", "ice_lanterns_a", "Ice", "Ice Lanterns", (1, 1), 8, 1, {"damage": 150, "hitdensity": 5, "rebornlight": "1.2 0.65 0.3 50 6"}),
    ("Cottage1", "cottage_a", "Ice", "Cottage", (4, 4), 40, 1, {"damage": 3000, "hitdensity": 40, "rebornlight": "1.3 0.8 0.4 110 14"}),
    ("Cottage2", "cottage_b", "Ice", "Cottage", (4, 4), 40, 1, {"damage": 3500, "hitdensity": 40, "rebornlight": "1.3 0.8 0.4 110 14"}),
    ("Cottage3", "cottage_c", "Ice", "Cottage", (4, 4), 40, 16, {"damage": 3000, "hitdensity": 40, "rebornlight": "1.4 0.9 0.5 120 14"}),
    ("Sleigh", "sleigh_a", "All Worlds", "Sleigh", (3, 2), 14, 1, {"reclaimable": 1, "metal": 30, "damage": 400, "hitdensity": 10}),
    ("ClockTower", "clock_tower_a", "All Worlds", "Clock Tower", (4, 4), 100, 1, {"indestructible": 1, "hitdensity": 60, "rebornlight": "1.5 1.35 1.0 170 92"}),
    ("Fireworks1", "fireworks_a", "All Worlds", "Fireworks", (2, 1), 16, 1, {"damage": 150, "hitdensity": 5, "rebornfireworks": 1}),
    ("Fireworks2", "fireworks_b", "All Worlds", "Fireworks", (2, 1), 24, 1, {"damage": 150, "hitdensity": 5, "rebornfireworks": 1}),
    ("LanternString", "lantern_line_a", "All Worlds", "Lantern String", (3, 1), 20, 1, {"damage": 200, "hitdensity": 5, "rebornlight": "1.6 0.65 0.3 90 20"}),
    ("SnowRock1", "snowrock_a", "Ice", "Snowy Boulder", (2, 2), 14, 1, {"reclaimable": 1, "metal": 12, "damage": 2000, "hitdensity": 30}),
    ("SnowRock2", "snowrock_b", "Ice", "Snowy Boulder", (2, 2), 16, 1, {"reclaimable": 1, "metal": 12, "damage": 2000, "hitdensity": 30}),
    ("SnowRock3", "snowrock_c", "Ice", "Snowy Boulder", (3, 3), 18, 1, {"reclaimable": 1, "metal": 20, "damage": 3000, "hitdensity": 30}),
]
