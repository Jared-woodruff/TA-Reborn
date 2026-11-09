"""Draws the rasters TA Reborn's maps of real places are grown from (`apps/ta_tools/geo/`): for
each map its land and lakes (from Natural Earth's public-domain outlines, `ne_10m_land` and
`ne_50m_lakes`), its relief (from the mountain ranges, plateaus and basins listed here, over
the land rising from its coasts) and its biomes (forest, grassland, desert and cold, and how
warm it is; from latitude and the regions listed here). The map generator reads them
(`mapgen/geo.rs`) and grows the rest: the cliffs, hills and peaks, the shores, the features.

    python -I tools/geo/outlines.py <folder holding ne_10m_land.shp and ne_50m_lakes.shp, or
        folders of them> [--only australia] [--preview scratch/geo]

Natural Earth's data is in the public domain (naturalearthdata.com). Everything else here is
written out by hand: where the ranges run, how high, which regions are desert.
"""
import argparse
import math
import os
import struct

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "..", "apps", "ta_tools", "geo")
SS = 4  # supersampling of the outlines

# ---------------------------------------------------------------------------------------------
# Shapefiles (polygons: rings of points, the outer ones clockwise)
# ---------------------------------------------------------------------------------------------


def polygons(path):
    """The polygon records of a shapefile, each a list of rings (lists of (lon, lat))."""
    data = open(path, "rb").read()
    code, = struct.unpack(">i", data[0:4])
    assert code == 9994, f"{path}: not a shapefile"
    pos, out = 100, []
    while pos + 8 <= len(data):
        _, words = struct.unpack(">ii", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + words * 2]
        pos += 8 + words * 2
        kind, = struct.unpack("<i", body[0:4])
        if kind not in (5, 15, 25):
            continue
        parts, points = struct.unpack("<ii", body[36:44])
        starts = list(struct.unpack(f"<{parts}i", body[44:44 + 4 * parts])) + [points]
        xy = np.frombuffer(body, dtype="<f8", count=points * 2, offset=44 + 4 * parts).reshape(-1, 2)
        out.append([xy[starts[k]:starts[k + 1]] for k in range(parts)])
    return out


def signed_area(ring):
    x, y = ring[:, 0], ring[:, 1]
    return 0.5 * float(np.sum(x[:-1] * y[1:] - x[1:] * y[:-1]))


# ---------------------------------------------------------------------------------------------
# The maps: their projections and bounds
# ---------------------------------------------------------------------------------------------


def miller(lat):
    return 1.25 * np.log(np.tan(np.pi / 4 + 0.4 * np.radians(lat)))


def miller_inverse(y):
    return np.degrees(2.5 * np.arctan(np.exp(0.8 * y)) - 0.625 * np.pi)


class Map:
    """A map `w` x `h` cells over lon0..lon1 and lat0 (south)..lat1 (north): equirectangular, or
    Miller's (`miller`, the world)."""

    def __init__(self, name, w, h, lon0, lon1, lat0, lat1, projection="equirect"):
        self.name, self.w, self.h = name, w, h
        self.lon0, self.lon1, self.lat0, self.lat1 = lon0, lon1, lat0, lat1
        self.projection = projection

    def to_cells(self, lon, lat):
        """Map cells (x east, z south, fractional) of points."""
        x = (np.asarray(lon) - self.lon0) / (self.lon1 - self.lon0) * self.w
        if self.projection == "miller":
            y0, y1 = miller(self.lat0), miller(self.lat1)
            z = (y1 - miller(np.asarray(lat))) / (y1 - y0) * self.h
        else:
            z = (self.lat1 - np.asarray(lat)) / (self.lat1 - self.lat0) * self.h
        return x, z

    def lonlat(self):
        """Each cell's middle (lon, lat), arrays h x w."""
        xs = (np.arange(self.w) + 0.5) / self.w
        zs = (np.arange(self.h) + 0.5) / self.h
        lon = self.lon0 + xs * (self.lon1 - self.lon0)
        if self.projection == "miller":
            y0, y1 = miller(self.lat0), miller(self.lat1)
            lat = miller_inverse(y1 - zs * (y1 - y0))
        else:
            lat = self.lat1 - zs * (self.lat1 - self.lat0)
        return np.meshgrid(lon, lat)


# ---------------------------------------------------------------------------------------------
# Rasters
# ---------------------------------------------------------------------------------------------


def rasterize(m, records, holes=True):
    """Coverage (0..1, h x w) of the polygons on map `m`."""
    img = Image.new("L", (m.w * SS, m.h * SS), 0)
    draw = ImageDraw.Draw(img)
    for rings in records:
        for k, ring in enumerate(rings):
            if len(ring) < 3:
                continue
            # (Skip what lies wholly off the map.)
            x, z = m.to_cells(ring[:, 0], ring[:, 1])
            if x.max() < -2 or x.min() > m.w + 2 or z.max() < -2 or z.min() > m.h + 2:
                continue
            pts = list(zip((x * SS).tolist(), (z * SS).tolist()))
            outer = signed_area(ring) < 0 or k == 0
            draw.polygon(pts, fill=255 if (outer or not holes) else 0)
    a = np.asarray(img, dtype=np.float32).reshape(m.h, SS, m.w, SS).mean(axis=(1, 3)) / 255.0
    return a


def edt(mask):
    """The distance (cells) from each cell to the nearest cell of `mask` (exact, Felzenszwalb's
    two passes of the lower envelope of parabolas)."""
    INF = 1e12
    f = np.where(mask, 0.0, INF)

    def line(f):
        n = len(f)
        d = np.empty(n)
        v = np.zeros(n, dtype=np.int64)
        z = np.empty(n + 1)
        k = 0
        v[0] = 0
        z[0], z[1] = -INF, INF
        for q in range(1, n):
            s = ((f[q] + q * q) - (f[v[k]] + v[k] * v[k])) / (2 * q - 2 * v[k])
            while s <= z[k]:
                k -= 1
                s = ((f[q] + q * q) - (f[v[k]] + v[k] * v[k])) / (2 * q - 2 * v[k])
            k += 1
            v[k] = q
            z[k], z[k + 1] = s, INF
        k = 0
        for q in range(n):
            while z[k + 1] < q:
                k += 1
            d[q] = (q - v[k]) ** 2 + f[v[k]]
        return d

    g = np.apply_along_axis(line, 0, f)
    g = np.apply_along_axis(line, 1, g)
    return np.sqrt(g)


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


def km(lon, lat, lon0, lat0):
    """Distances (km) from (lon0, lat0), flat on the sphere's local scale."""
    dx = (lon - lon0) * 111.32 * np.cos(np.radians((lat + lat0) * 0.5))
    dy = (lat - lat0) * 110.57
    return np.hypot(dx, dy)


def to_polyline_km(lon, lat, line):
    """Each cell's distance (km) to a polyline of (lon, lat) points."""
    best = np.full(lon.shape, np.inf)
    for (a0, b0), (a1, b1) in zip(line[:-1], line[1:]):
        c = math.cos(math.radians((b0 + b1) * 0.5))
        ax, ay = a0 * 111.32 * c, b0 * 110.57
        bx, by = a1 * 111.32 * c, b1 * 110.57
        px, py = lon * 111.32 * c, lat * 110.57
        dx, dy = bx - ax, by - ay
        t = np.clip(((px - ax) * dx + (py - ay) * dy) / max(dx * dx + dy * dy, 1e-9), 0.0, 1.0)
        best = np.minimum(best, np.hypot(px - (ax + dx * t), py - (ay + dy * t)))
    return best


def noise(shape, scale, seed):
    """Smooth noise (-1..1) of features about `scale` cells across."""
    rng = np.random.default_rng(seed)
    h, w = shape
    small = rng.uniform(-1, 1, (max(2, int(h / scale) + 3), max(2, int(w / scale) + 3)))
    img = Image.fromarray(((small + 1) * 127.5).astype(np.uint8)).resize((w, h), Image.BICUBIC)
    return np.asarray(img, dtype=np.float32) / 127.5 - 1.0


def blur(a, radius):
    img = Image.fromarray(np.clip(a * 255.0, 0, 255).astype(np.uint8))
    return np.asarray(img.filter(ImageFilter.GaussianBlur(radius)), dtype=np.float32) / 255.0


# ---------------------------------------------------------------------------------------------
# Geography, written out (lon, lat; km; heights 0..1 of the map's relief)
# ---------------------------------------------------------------------------------------------

# Ranges: (points, half width km, height). Blobs (plateaus, highlands; basins sink): (lon, lat,
# radius km, height). Deserts, grasslands and forests: ellipses (lon, lat, radius lon, radius lat,
# weight), soft-edged.
GEOGRAPHY = {
    "australia": {
        "ranges": [
            ([(145.2, -10.7), (145.5, -16.0), (146.3, -19.0), (148.5, -22.0), (150.5, -25.0), (151.8, -28.0), (151.5, -31.0), (150.3, -33.5), (149.0, -36.0), (147.5, -37.2), (146.2, -37.6), (144.0, -37.4)], 120, 0.42),
            ([(148.1, -35.6), (148.4, -36.6), (147.6, -37.0)], 60, 0.32),
            ([(145.5, -41.6), (146.4, -42.0), (146.9, -42.6)], 55, 0.4),
            ([(131.2, -23.7), (133.0, -23.6), (135.3, -23.4)], 35, 0.28),
            ([(129.8, -26.2), (131.3, -26.2), (132.8, -26.0)], 35, 0.24),
            ([(138.5, -34.3), (138.4, -32.3), (139.3, -30.4)], 45, 0.26),
            ([(116.2, -22.4), (118.2, -22.6), (119.8, -22.9)], 60, 0.28),
            ([(115.9, -31.0), (116.2, -32.4), (116.6, -34.2)], 30, 0.12),
        ],
        "blobs": [
            (126.0, -16.6, 260, 0.2), (133.5, -13.2, 160, 0.15), (122.0, -26.0, 700, 0.14), (137.5, -28.5, 330, -0.12), (138.0, -21.5, 300, 0.08),
        ],
        # (The last the Gascoyne's coast, north of Shark Bay.)
        "desert": [(128.0, -25.0, 13.0, 7.0, 1.0), (122.5, -21.5, 6.0, 3.5, 0.8), (137.0, -27.5, 6.0, 5.0, 0.8), (129.0, -30.5, 7.0, 2.0, 0.7), (114.5, -24.5, 2.5, 3.5, 0.9)],
        "grass": [(134.0, -18.5, 14.0, 4.0, 0.9), (145.0, -27.0, 5.0, 8.0, 0.6), (121.0, -30.5, 5.0, 3.0, 0.5), (140.0, -33.0, 4.0, 2.5, 0.5)],
        "forest": [(146.0, -17.5, 2.0, 5.0, 1.0), (150.5, -30.0, 2.5, 7.0, 1.0), (146.5, -37.5, 4.0, 1.8, 1.0), (146.5, -42.0, 2.0, 1.6, 1.0), (116.0, -33.5, 1.8, 2.0, 0.9), (132.0, -12.5, 3.0, 1.2, 0.7), (141.5, -8.5, 4.0, 1.5, 1.0)],
        "cold": [],
    },
    "usa": {
        "ranges": [
            ([(-114.5, 50.5), (-114.0, 48.5), (-112.0, 46.0), (-110.0, 44.5), (-108.0, 43.0), (-106.5, 41.0), (-106.0, 39.5), (-106.0, 37.5), (-106.0, 35.5), (-105.5, 33.5)], 220, 0.62),
            ([(-121.5, 40.5), (-119.5, 37.5), (-118.2, 35.5)], 70, 0.66),
            ([(-121.5, 49.0), (-121.8, 46.5), (-122.0, 44.0), (-121.8, 42.0)], 80, 0.5),
            ([(-123.8, 48.0), (-123.5, 44.0), (-123.2, 40.0), (-121.0, 36.0), (-119.5, 34.5), (-117.0, 33.5)], 45, 0.24),
            ([(-85.5, 34.0), (-83.5, 35.5), (-81.5, 37.0), (-79.5, 38.5), (-77.5, 40.5), (-75.5, 42.0), (-74.0, 43.5), (-72.0, 44.5), (-70.5, 45.0)], 140, 0.3),
            ([(-108.0, 31.5), (-106.0, 28.0), (-104.5, 25.0)], 160, 0.5),
            ([(-101.0, 29.0), (-100.0, 26.0)], 120, 0.3),
            ([(-117.0, 41.5), (-116.0, 39.0)], 50, 0.22),
        ],
        "blobs": [
            (-110.5, 37.0, 330, 0.36), (-116.5, 40.0, 380, 0.3), (-103.5, 27.0, 420, 0.3), (-92.5, 36.6, 200, 0.12), (-103.6, 44.0, 60, 0.18), (-121.8, 46.9, 40, 0.3), (-122.2, 41.4, 40, 0.3), (-121.7, 45.4, 35, 0.25), (-116.0, 44.5, 260, 0.3),
        ],
        "tilts": [(-97.0, -105.5, 0.0, 0.34)],
        # (The last two Mexico's: Baja California and Sonora's coast, the Chihuahuan desert.)
        "desert": [(-115.0, 35.0, 4.5, 3.5, 1.0), (-111.0, 32.5, 4.0, 2.5, 1.0), (-106.0, 30.5, 4.0, 3.0, 0.9), (-117.0, 40.0, 4.0, 3.0, 0.7), (-110.0, 38.5, 2.5, 2.0, 0.5),
                   (-113.0, 28.0, 3.5, 5.0, 1.0), (-104.5, 28.5, 3.0, 3.0, 0.9)],
        "grass": [(-100.0, 41.0, 5.5, 9.5, 1.0), (-104.0, 45.5, 4.0, 4.0, 0.8), (-98.5, 32.5, 3.5, 3.0, 0.8), (-112.0, 44.0, 4.0, 2.5, 0.5)],
        "forest": [(-84.0, 38.0, 11.0, 7.0, 1.0), (-123.0, 46.0, 2.5, 4.5, 1.0), (-122.0, 40.0, 1.5, 2.0, 0.8), (-90.0, 46.5, 6.0, 3.0, 1.0), (-70.0, 44.0, 3.0, 2.5, 1.0), (-82.0, 30.5, 3.5, 2.5, 0.8), (-113.5, 47.5, 3.0, 2.0, 0.7)],
        "cold": [],
    },
    "world": {
        "ranges": [
            ([(73.0, 35.5), (77.0, 33.5), (81.0, 30.5), (86.0, 28.0), (89.0, 27.8), (92.0, 27.6), (95.0, 28.6), (97.5, 28.2)], 260, 0.85),
            ([(71.0, 36.5), (74.5, 36.5), (77.5, 35.5)], 200, 0.7),
            ([(70.0, 41.0), (75.0, 41.5), (80.0, 42.0), (86.0, 43.0), (93.0, 43.0)], 240, 0.5),
            ([(86.0, 50.5), (89.0, 49.0), (95.0, 47.5), (100.0, 47.0)], 260, 0.4),
            ([(59.0, 51.0), (59.5, 55.5), (59.0, 60.5), (60.5, 65.0), (65.0, 68.0)], 160, 0.3),
            ([(39.5, 43.8), (43.0, 42.8), (47.0, 41.5)], 140, 0.48),
            ([(5.5, 44.0), (7.5, 46.0), (10.5, 46.6), (13.5, 47.0), (16.0, 47.6)], 160, 0.5),
            ([(-1.5, 43.0), (1.0, 42.6), (3.0, 42.4)], 90, 0.3),
            ([(18.5, 49.5), (22.5, 49.0), (25.5, 47.5), (25.5, 45.5), (22.5, 45.0)], 140, 0.28),
            ([(5.5, 59.0), (9.0, 61.5), (13.0, 64.0), (16.5, 67.0), (20.0, 69.0)], 180, 0.36),
            ([(-9.5, 30.5), (-5.0, 32.0), (1.0, 34.5), (7.5, 35.5)], 200, 0.42),
            ([(44.0, 37.0), (48.0, 34.0), (52.0, 31.0), (56.0, 28.0)], 220, 0.42),
            ([(36.0, 2.0), (35.0, -3.0), (33.5, -8.0), (31.0, -12.0)], 280, 0.38),
            ([(28.0, -32.5), (29.5, -29.5), (31.0, -27.0)], 200, 0.35),
            ([(-153.0, 62.5), (-147.0, 61.5), (-141.0, 61.0), (-136.0, 59.5), (-132.0, 56.5), (-127.0, 52.0)], 260, 0.55),
            ([(-128.0, 56.0), (-122.0, 52.0), (-117.0, 49.0), (-113.0, 46.0), (-110.0, 44.0), (-107.0, 41.0), (-106.0, 37.0), (-105.5, 33.5)], 380, 0.62),
            ([(-122.0, 48.5), (-121.5, 44.0), (-120.0, 38.0), (-118.0, 35.0)], 160, 0.45),
            ([(-108.0, 31.0), (-105.5, 26.0), (-103.0, 21.0), (-98.0, 18.0), (-94.0, 16.5)], 260, 0.48),
            ([(-85.5, 34.0), (-81.5, 37.0), (-77.5, 40.5), (-73.5, 43.0), (-69.5, 46.5)], 220, 0.26),
            ([(-74.0, 10.5), (-76.5, 4.0), (-78.5, -2.0), (-77.5, -8.0), (-73.5, -14.0), (-69.5, -17.0), (-68.0, -22.0), (-69.0, -28.0), (-70.0, -33.0), (-71.0, -40.0), (-72.5, -46.0), (-73.5, -51.0), (-71.5, -54.5)], 260, 0.8),
            ([(146.0, -17.0), (149.0, -22.5), (151.5, -28.0), (150.0, -33.5), (148.0, -36.5), (145.0, -37.6)], 200, 0.3),
            ([(168.5, -45.5), (170.5, -43.5), (172.5, -42.0)], 120, 0.45),
            ([(136.5, 35.0), (138.5, 36.0), (140.0, 39.0), (140.5, 42.0)], 140, 0.36),
            ([(101.0, 30.0), (102.0, 26.0), (101.0, 22.0)], 220, 0.45),
            ([(39.0, 13.0), (39.5, 9.0), (38.0, 6.0)], 300, 0.48),
            ([(43.5, 17.0), (44.0, 14.0)], 160, 0.36),
            ([(-50.0, 65.0), (-43.0, 72.0), (-38.0, 77.0)], 500, 0.55),
        ],
        "blobs": [
            (88.0, 33.5, 950, 0.62), (55.0, 32.5, 800, 0.24), (45.0, 23.5, 900, 0.14), (-45.0, -17.0, 1000, 0.2), (-63.0, 5.5, 450, 0.18), (20.0, -25.0, 900, 0.18),
            (-40.0, 72.0, 1100, 0.45), (108.0, 46.0, 900, 0.24), (100.0, 60.0, 1200, 0.12), (-112.0, 39.0, 700, 0.3), (37.0, 0.5, 700, 0.22), (10.0, 6.0, 500, 0.12),
            (133.0, -24.0, 1300, 0.12), (80.0, 23.0, 900, 0.12), (-3.0, 40.0, 450, 0.18), (29.0, 39.0, 550, 0.24),
        ],
        "desert": [
            # (The Sahara from the Atlantic to the Red Sea: its middle, its west by the Atlantic,
            # its east down the Nile, its south-west in Mali, its northern ergs up to the Saharan
            # Atlas, the Tademait, northern Sudan; Arabia, its Gulf coast and Oman's, Iran's.)
            (12.0, 23.0, 24.0, 8.5, 1.0), (-10.0, 22.0, 10.0, 7.5, 1.0), (29.0, 23.5, 8.5, 7.0, 1.0), (0.0, 19.5, 10.0, 5.0, 1.0),
            (10.0, 29.5, 13.0, 3.0, 1.0), (6.0, 31.0, 14.0, 2.8, 1.0), (0.0, 27.0, 7.0, 3.0, 1.0), (32.0, 18.5, 6.0, 3.0, 1.0),
            (46.0, 23.0, 11.0, 7.5, 1.0), (56.0, 22.0, 4.8, 4.0, 0.9), (57.0, 27.0, 4.0, 2.0, 0.8), (68.0, 27.0, 6.0, 4.0, 0.8), (60.0, 40.0, 8.0, 4.0, 0.7), (83.0, 39.0, 7.0, 2.6, 1.0),
            (105.0, 42.5, 10.0, 3.5, 0.9), (128.0, -24.5, 14.0, 7.0, 1.0), (20.0, -23.5, 6.0, 4.5, 0.8), (14.5, -22.0, 2.0, 6.0, 0.8), (-70.0, -23.0, 1.8, 7.0, 0.9),
            (-113.0, 34.0, 6.0, 5.0, 0.9), (-106.0, 29.0, 4.0, 4.0, 0.7), (-68.0, -45.0, 3.5, 5.0, 0.5), (55.0, 32.0, 6.0, 3.5, 0.6), (45.0, 15.5, 4.0, 2.5, 0.6),
            # (Where the land would otherwise be wooded: Lake Eyre's basin and the Simpson, the
            # Nullarbor, the Karoo, Namaqualand, the Monte, the Gashun Gobi and the Hexi corridor,
            # the Qaidam, the Alxa's deserts, the Junggar's.)
            (137.0, -28.0, 6.0, 4.5, 0.9), (128.0, -30.0, 8.0, 2.5, 0.9), (21.0, -31.5, 4.5, 2.0, 0.8), (17.5, -29.0, 2.0, 3.0, 0.9), (-67.5, -32.0, 2.5, 6.0, 0.8),
            (93.0, 41.0, 6.0, 2.5, 0.9), (95.5, 37.0, 4.0, 1.5, 0.8), (104.0, 39.5, 4.0, 2.0, 0.9), (87.5, 45.0, 3.5, 1.5, 0.8),
        ],
        "grass": [
            (-100.0, 42.0, 7.0, 9.0, 1.0), (-62.0, -35.0, 6.0, 6.0, 1.0), (60.0, 49.0, 22.0, 4.0, 1.0), (100.0, 47.0, 14.0, 4.0, 0.8), (8.0, 12.0, 32.0, 4.5, 1.0),
            (30.0, -10.0, 14.0, 10.0, 0.8), (-50.0, -14.0, 8.0, 6.0, 0.8), (133.0, -17.0, 13.0, 3.5, 0.9), (145.0, -28.0, 6.0, 8.0, 0.6), (40.0, 2.0, 6.0, 6.0, 0.7),
            (-64.0, 8.0, 5.0, 2.5, 0.6), (77.0, 22.0, 7.0, 6.0, 0.5), (25.0, -30.0, 6.0, 3.0, 0.6),
            # (The steppes of Afghanistan and eastern Iran, the Ordos, eastern Inner Mongolia; the
            # Sahel up to the Sahara, staggered so their meeting wanders.)
            (65.0, 33.5, 7.0, 4.0, 0.9), (110.0, 38.5, 3.5, 2.0, 0.8), (116.0, 44.0, 6.0, 2.5, 0.9),
            (-8.0, 16.8, 9.0, 2.4, 0.85), (6.0, 16.0, 9.0, 2.2, 0.85), (20.0, 16.6, 9.0, 2.4, 0.85), (32.0, 15.8, 7.0, 2.2, 0.85),
        ],
        "forest": [
            (-62.0, -4.0, 15.0, 9.0, 1.0), (20.0, 0.0, 10.0, 5.0, 1.0), (110.0, 2.0, 18.0, 9.0, 1.0), (102.0, 15.0, 8.0, 7.0, 0.9), (-84.0, 39.0, 10.0, 8.0, 1.0),
            (15.0, 50.0, 14.0, 6.0, 1.0), (95.0, 60.0, 45.0, 7.0, 1.0), (-100.0, 56.0, 30.0, 6.0, 1.0), (-123.0, 50.0, 4.0, 8.0, 1.0), (115.0, 30.0, 8.0, 7.0, 0.9),
            (135.0, 47.0, 10.0, 6.0, 0.9), (148.0, -33.0, 4.0, 6.0, 0.7), (-48.0, -24.0, 5.0, 5.0, 0.7), (-75.0, 5.0, 3.0, 5.0, 0.8), (35.0, 60.0, 25.0, 5.0, 1.0),
            (172.0, -42.0, 4.0, 5.0, 0.8), (138.0, 36.0, 4.0, 5.0, 0.9), (77.0, 12.0, 3.0, 4.0, 0.6),
        ],
        "cold": [(-40.0, 73.0, 18.0, 12.0, 1.0), (88.0, 33.5, 9.0, 4.0, 0.7)],
        "ice": [(-41.0, 73.5, 14.0, 10.0, 1.0)],
    },
}

MAPS = {
    "australia": Map("australia", 896, 768, 110.0, 158.0, -45.0, -7.5),
    "usa": Map("usa", 1024, 576, -126.0, -65.0, 23.5, 50.5),
    "world": Map("world", 1280, 640, -180.0, 180.0, -60.0, 84.0, projection="miller"),
}

# The places the starts are chosen near (lon, lat), in the map's start order.
PLACES = {
    "australia": [("Perth", 116.0, -31.9), ("Brisbane", 152.8, -27.6), ("Darwin", 131.1, -12.8), ("Melbourne", 144.6, -37.6), ("Cairns", 145.6, -17.2), ("Adelaide", 138.7, -34.8), ("Port Hedland", 118.9, -20.8), ("Sydney", 150.8, -33.7)],
    "usa": [("Seattle", -122.2, 47.4), ("Miami", -80.4, 26.0), ("Los Angeles", -117.9, 34.1), ("New York", -74.5, 40.9), ("Denver", -104.8, 39.7), ("Chicago", -88.0, 41.8), ("Houston", -95.6, 29.9), ("Minneapolis", -93.4, 45.0)],
    "world": [("Washington", -78.0, 39.0), ("Beijing", 116.0, 39.5), ("Paris", 2.5, 48.7), ("Sydney", 150.0, -33.5), ("Sao Paulo", -47.0, -23.0), ("Moscow", 37.5, 55.5), ("Lagos", 4.0, 7.5), ("Delhi", 77.0, 28.5), ("Los Angeles", -117.5, 34.3), ("Nairobi", 36.8, -1.5)],
}


def ellipses(lon, lat, items, wobble):
    """Soft-edged regions (their edges wandering by `wobble`), each its weight inside: the most
    of any at each cell."""
    out = np.zeros(lon.shape, dtype=np.float32)
    for lo, la, rx, ry, k in items:
        dl = ((lon - lo + 180.0) % 360.0) - 180.0
        r = np.hypot(dl / rx, (lat - la) / ry) + wobble * 0.34
        out = np.maximum(out, k * smoothstep(1.25, 0.65, r))
    return out


def build(name, land_shp, lakes_shp, preview):
    m = MAPS[name]
    g = GEOGRAPHY[name]
    lon, lat = m.lonlat()
    land = rasterize(m, land_shp)
    lakes = rasterize(m, lakes_shp) * (land > 0.5)
    ground = (land >= 0.5) & (lakes < 0.5)
    # How far inland (km): from the cells' distance to the nearest water.
    cell_km = (m.lon1 - m.lon0) / m.w * 111.32 * np.cos(np.radians(np.clip(lat, -70, 70)))
    inland = edt(~ground) * cell_km
    relief = 0.03 + 0.1 * smoothstep(0.0, 450.0, inland)
    for line, half, k in g["ranges"]:
        d = to_polyline_km(lon, lat, line)
        relief += k * np.exp(-(d / half) ** 2)
    for lo, la, r, k in g["blobs"]:
        d = km(lon, lat, lo, la)
        relief += k * np.exp(-(d / r) ** 2)
    # Tilts: the land rising from one longitude to another (the Great Plains to the Rockies),
    # from nothing.
    for lon_a, lon_b, ka, kb in g.get("tilts", []):
        t = smoothstep(0.0, 1.0, (lon - lon_a) / (lon_b - lon_a))
        relief += (ka + (kb - ka) * t) * t * smoothstep(-0.05, 0.1, (lat - 26.0) / 30.0)
    # (Above 0.7 eased towards 1: where ranges and plateaus pile up, the top keeps its shape.)
    relief = np.where(relief < 0.7, relief, 0.7 + 0.3 * np.tanh((relief - 0.7) / 0.3))
    relief = np.clip(relief, 0.0, 1.0) * ground
    # Biomes: forest, grass, desert and cold, each as much as its regions say, the rest of the
    # land temperate forest and grass by turns; how warm it is from the latitude and the height.
    wob = noise(lon.shape, max(m.w, m.h) / 18.0, 7) + 0.6 * noise(lon.shape, max(m.w, m.h) / 50.0, 8) + 0.3 * noise(lon.shape, max(m.w, m.h) / 140.0, 9)
    desert = ellipses(lon, lat, g["desert"], wob)
    grass = ellipses(lon, lat, g["grass"], wob)
    forest = ellipses(lon, lat, g["forest"], wob)
    # (Cold: the polar lands and ice caps; the snow on the peaks is the generator's, by height.)
    cold = np.maximum(ellipses(lon, lat, g["cold"], wob), smoothstep(60.0, 70.0, np.abs(lat) + 4.0 * wob))
    base = 0.35 + 0.1 * wob
    forest = np.maximum(forest, base * (1.0 - desert) * (1.0 - grass * 0.7))
    grass = np.maximum(grass, (0.3 - 0.1 * wob) * (1.0 - forest * 0.5) * (1.0 - desert))
    # (Over the land never quite polar, but on the ice caps: there the cold is ice, elsewhere
    # tundra.)
    warmth = np.clip(1.0 - (np.abs(lat) - 14.0) / 48.0, 0.0, 1.0) - 0.45 * relief
    ice = ellipses(lon, lat, g.get("ice", []), wob)
    warmth = np.where(ground, np.maximum(warmth, 0.12 * (1.0 - ice)), warmth) * (1.0 - ice)
    biome = np.stack([forest * (1 - cold), grass * (1 - cold), desert * (1 - cold), cold], axis=-1)
    biome = np.clip(biome, 0.0, 1.0) * ground[..., None]
    os.makedirs(OUT, exist_ok=True)
    save = lambda a, mode, suffix: Image.fromarray(np.round(np.clip(a, 0, 1) * 255).astype(np.uint8), mode).save(os.path.join(OUT, f"{name}_{suffix}.png"), optimize=True)
    save(land, "L", "land")
    save(lakes, "L", "lakes")
    save(relief, "L", "relief")
    save(biome, "RGBA", "biome")
    save(np.clip(warmth, 0, 1), "L", "warmth")
    starts = []
    for place, lo, la in PLACES[name]:
        x, z = m.to_cells(lo, la)
        starts.append((place, float(x) / m.w, float(z) / m.h))
    print(f"{name}: {m.w} x {m.h} cells, land {ground.mean() * 100:.0f}%, relief {relief.max():.2f} at most")
    print("  starts: " + ", ".join(f"({x:.4f}, {z:.4f}) {p}" for p, x, z in starts))
    if preview:
        os.makedirs(preview, exist_ok=True)
        rgb = np.zeros((m.h, m.w, 3))
        sea = np.array([0.12, 0.25, 0.45])
        cols = np.array([[0.2, 0.45, 0.18], [0.62, 0.66, 0.3], [0.84, 0.72, 0.45], [0.92, 0.94, 0.97]])
        w = biome / np.maximum(biome.sum(axis=-1, keepdims=True), 1e-6)
        rgb = np.where(ground[..., None], (w[..., :, None] * cols[None, None]).sum(axis=2) * (0.65 + 0.6 * relief[..., None]), sea)
        rgb = np.where((lakes > 0.5)[..., None], np.array([0.2, 0.4, 0.7]), rgb)
        img = Image.fromarray(np.round(np.clip(rgb, 0, 1) * 255).astype(np.uint8))
        draw = ImageDraw.Draw(img)
        for p, x, z in starts:
            draw.ellipse([x * m.w - 4, z * m.h - 4, x * m.w + 4, z * m.h + 4], outline=(255, 40, 40), width=2)
        img.save(os.path.join(preview, f"{name}_preview.png"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("data")
    ap.add_argument("--only")
    ap.add_argument("--preview")
    args = ap.parse_args()
    found = {f: os.path.join(d, f) for d, _, files in os.walk(args.data) for f in files}
    land = polygons(found["ne_10m_land.shp"])
    lakes = polygons(found["ne_50m_lakes.shp"])
    for name in MAPS:
        if args.only and name not in args.only.split(","):
            continue
        build(name, land, lakes, args.preview)


if __name__ == "__main__":
    main()
