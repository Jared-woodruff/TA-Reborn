"""Builds the bedroom the intro plays in (original art for TA Reborn) and exports it as GLB.

    blender -b -P tools/room/build.py -- --out C:/.../crates/ta_render/room
        [--preview scratch/room [--season normal|christmas|newyear]] [--no-bake] [--samples 96]

A teenager's room, late at night, autumn 1997: a beige computer on a wooden desk (a mini-tower
with its drive bays, keylock and a fan's emblem badge, a modem on it, a 19-inch CRT with a
sculpted bezel playing TA's movies, slotted speakers, the keyboard's tapered keys), a lava
lamp glowing in the corner, an anglepoise lamp, a flight stick, a battle planned in pen on a
map's print, moonlight in through the venetian blinds, posters on the wall and the game's big
boxes on the shelf; on the days around Christmas and New Year it is dressed for them (see "The
seasons" below). `ROOM_REPORT=1` lists the heaviest objects. `ROOM_CKPT=<dir>` keeps each bake
pass there as it finishes and uses the kept ones of the same room again: a bake that dies late
(the card shared with other programs out of memory) goes on from where it was. Without gltfpack
on the PATH the GLB is packed here (`quantize`: normals and the baked colours as 16-bit
integers).

Blender axes, metres: x to the right, y away from the viewer, z up; the floor at z = 0, the
desk's top at z = 0.74, the wall behind it at y = 0.62. (The glTF exporter turns them into
glTF's: x right, y up, the viewer looking along -z.)

The seasons: the file holds the room of every day of the year. What a season puts up is a
node named `season.<name>.<material>` (`christmas`, `newyear`), anything else is there always;
the game (`RoomModel::from_glb(bytes, season)`) leaves out the nodes of the other seasons. Each
season's anchors (`anchor.light0..2`: where its lights shine from) carry the tag too. The bakes:
the room is baked without any season's things; each season's own things are then baked standing
in that room, alone with it.

What the renderer (`ta_render::room`) reads from the file:
- Materials by name: their role (see `room.rs`): `wood`, `laminate`, `paint`, `carpet`,
  `plastic` (and `seam`), `keycap`, `rubber`, `metal`, `alu`, `chrome`, `ceramic`, `coffee`, `paper`,
  `cardboard`, `fabric`, `glass`, `lava`, `sky`, `silhouette`, `emit.<name>` (glows: LEDs,
  the lava's light), `picture.<name>` (painted by the game from the install's art: posters,
  boxes, the mouse mat, the manual, the calendar, the notes, the keys' legends; UVs span the
  picture, its extras give its shape: `aspect`, width over height).
- Empties named `anchor.<name>`: the tube (`anchor.screen`: at the glass's middle, its scale
  the glass's half width and height and how far it bulges), the eye and what it looks at, the
  moon's direction (`anchor.moon`'s place), the lava lamp's light, the LEDs, the coffee's
  steam; `anchor.keys` carries the keyboard's legends in its extras (`keys`: JSON, a list of
  [legend, u0, v0, u1, v1, r, g, b]: where on `picture.keys` each key's top is, and its colour).
- Vertex colours, baked by Cycles (each channel the square root of the light over its scale,
  for precision in the dark): COLOR_0 = the moonlight that bounces round the room (rgb, scaled
  by `BOUNCE_SCALE`) and ambient occlusion (a); COLOR_1 = the light the tube's four quarters
  bounce round the room, each at unit white (top left, top right, bottom left, bottom right;
  scaled by `TUBE_SCALE`); COLOR_2 = the lava lamp's light, direct and bounced (rgb, scaled by
  `LAVA_SCALE`). The renderer adds the moon's and the tube's direct light itself, with shadows.
"""

import argparse
import array
import contextlib
import hashlib
import json
import math
import os
import pickle
import random
import shutil
import struct
import subprocess
import sys

import bpy
import bmesh
from mathutils import Matrix, Vector

BOUNCE_SCALE = 4.0
TUBE_SCALE = 1.0
LAVA_SCALE = 4.0

# The light sources, as the renderer has them too (`room.rs`): the moon's direction (towards
# it) and colour, the lava lamp's colour and strength, the tube's emission for the bakes.
# (From the left and a little behind the viewer, so its beams cross the room to the wall
# behind the desk.)
MOON_DIR = Vector((-0.8, -0.45, 0.4)).normalized()
MOON_COLOR = (0.55, 0.68, 1.0)
MOON_STRENGTH = 3.0
LAVA_COLOR = (1.0, 0.42, 0.12)
LAVA_WATTS = 9.0
SKY_GLOW = (0.012, 0.018, 0.04)
# The view's vertical field of view (degrees).
FOV_Y = 50.0

rng = random.Random(1997)

# The seasons besides the ordinary room (see the module's notes). While one is being built,
# SEASON names it and what is made is its own.
SEASONS = ("christmas", "newyear")
SEASON = None


@contextlib.contextmanager
def in_season(name):
    global SEASON
    before, SEASON = SEASON, name
    try:
        yield
    finally:
        SEASON = before


def tag(obj):
    """Marks what is being built with its season (`SEASON`)."""
    if SEASON:
        obj["season"] = SEASON
    return obj


def tagged(name):
    """A node's name with its season's tag, as `RoomModel::from_glb` reads it."""
    if SEASON:
        return f"season.{SEASON}.{name}"
    return name

# ---------------------------------------------------------------------------------------------
# Materials
# ---------------------------------------------------------------------------------------------

# name: (base colour, metallic, roughness, emission colour, emission strength, alpha)
MATERIALS = {
    "wood": ((0.32, 0.17, 0.075), 0.0, 0.42, None, 0.0, 1.0),
    "laminate": ((0.42, 0.33, 0.24), 0.0, 0.5, None, 0.0, 1.0),
    "paint": ((0.52, 0.55, 0.6), 0.0, 0.85, None, 0.0, 1.0),
    "paint_white": ((0.72, 0.71, 0.66), 0.0, 0.55, None, 0.0, 1.0),
    "ceiling": ((0.7, 0.7, 0.68), 0.0, 0.9, None, 0.0, 1.0),
    "carpet": ((0.12, 0.1, 0.13), 0.0, 1.0, None, 0.0, 1.0),
    "plastic": ((0.62, 0.58, 0.47), 0.0, 0.45, None, 0.0, 1.0),
    "plastic_dark": ((0.11, 0.11, 0.11), 0.0, 0.5, None, 0.0, 1.0),
    "mouse": ((0.5, 0.49, 0.45), 0.0, 0.42, None, 0.0, 1.0),
    "foam": ((0.025, 0.025, 0.028), 0.0, 1.0, None, 0.0, 1.0),
    "pen_blue": ((0.03, 0.08, 0.35), 0.0, 0.3, None, 0.0, 1.0),
    "pen_black": ((0.02, 0.02, 0.02), 0.0, 0.3, None, 0.0, 1.0),
    "pen_red": ((0.45, 0.03, 0.02), 0.0, 0.3, None, 0.0, 1.0),
    "keycap": ((0.66, 0.63, 0.53), 0.0, 0.38, None, 0.0, 1.0),
    "keycap_dark": ((0.42, 0.41, 0.38), 0.0, 0.4, None, 0.0, 1.0),
    "rubber": ((0.025, 0.025, 0.025), 0.0, 0.85, None, 0.0, 1.0),
    "metal": ((0.08, 0.08, 0.09), 0.6, 0.45, None, 0.0, 1.0),
    "alu": ((0.62, 0.63, 0.65), 1.0, 0.32, None, 0.0, 1.0),
    "chrome": ((0.8, 0.8, 0.82), 1.0, 0.12, None, 0.0, 1.0),
    "brass": ((0.75, 0.55, 0.25), 1.0, 0.3, None, 0.0, 1.0),
    "ceramic": ((0.78, 0.76, 0.7), 0.0, 0.18, None, 0.0, 1.0),
    "ceramic_blue": ((0.08, 0.16, 0.38), 0.0, 0.2, None, 0.0, 1.0),
    "coffee": ((0.05, 0.025, 0.012), 0.0, 0.08, None, 0.0, 1.0),
    "paper": ((0.8, 0.78, 0.72), 0.0, 0.8, None, 0.0, 1.0),
    "paper_yellow": ((0.85, 0.75, 0.25), 0.0, 0.8, None, 0.0, 1.0),
    "cardboard": ((0.42, 0.3, 0.18), 0.0, 0.9, None, 0.0, 1.0),
    "fabric": ((0.2, 0.05, 0.06), 0.0, 0.95, None, 0.0, 1.0),
    "fabric_blue": ((0.06, 0.08, 0.16), 0.0, 0.95, None, 0.0, 1.0),
    "glass": ((0.6, 0.65, 0.7), 0.0, 0.05, None, 0.0, 0.12),
    "lava": ((0.9, 0.35, 0.1), 0.0, 0.1, (1.0, 0.38, 0.1), 6.0, 1.0),
    "lava_metal": ((0.18, 0.06, 0.32), 0.9, 0.3, None, 0.0, 1.0),
    "sky": ((0.0, 0.0, 0.0), 0.0, 1.0, SKY_GLOW, 1.0, 1.0),
    "moon": ((0.0, 0.0, 0.0), 0.0, 1.0, (0.85, 0.9, 1.0), 25.0, 1.0),
    "silhouette": ((0.004, 0.005, 0.008), 0.0, 1.0, None, 0.0, 1.0),
    "throat": ((0.004, 0.005, 0.005), 0.0, 0.6, None, 0.0, 1.0),
    "cork": ((0.45, 0.3, 0.17), 0.0, 0.95, None, 0.0, 1.0),
    "tape": ((0.75, 0.72, 0.62), 0.0, 0.4, None, 0.0, 0.55),
    "book_red": ((0.35, 0.05, 0.04), 0.0, 0.7, None, 0.0, 1.0),
    "book_green": ((0.06, 0.2, 0.1), 0.0, 0.7, None, 0.0, 1.0),
    "book_blue": ((0.05, 0.1, 0.3), 0.0, 0.7, None, 0.0, 1.0),
    "book_black": ((0.03, 0.03, 0.03), 0.0, 0.6, None, 0.0, 1.0),
    "book_cream": ((0.6, 0.55, 0.42), 0.0, 0.7, None, 0.0, 1.0),
    "box_purple": ((0.18, 0.06, 0.25), 0.0, 0.4, None, 0.0, 1.0),
    "box_teal": ((0.03, 0.22, 0.24), 0.0, 0.4, None, 0.0, 1.0),
    "box_orange": ((0.5, 0.18, 0.03), 0.0, 0.4, None, 0.0, 1.0),
    "floppy_black": ((0.03, 0.03, 0.035), 0.0, 0.35, None, 0.0, 1.0),
    "floppy_blue": ((0.04, 0.08, 0.3), 0.0, 0.35, None, 0.0, 1.0),
    "floppy_red": ((0.35, 0.03, 0.03), 0.0, 0.35, None, 0.0, 1.0),
    "pencil": ((0.75, 0.55, 0.05), 0.0, 0.4, None, 0.0, 1.0),
    "grille": ((0.02, 0.02, 0.022), 0.2, 0.7, None, 0.0, 1.0),
    # The brown of a seam or a slot in beige plastic (never black).
    "seam": ((0.30, 0.27, 0.21), 0.0, 0.7, None, 0.0, 1.0),
    # The seasons' things.
    # Christmas
    "pot_red": ((0.55, 0.04, 0.03), 0.0, 0.3, None, 0.0, 1.0),
    "fabric_fir": ((0.018, 0.11, 0.04), 0.0, 0.9, None, 0.0, 1.0),
    "fabric_red": ((0.5, 0.025, 0.03), 0.0, 0.95, None, 0.0, 1.0),
    "fabric_white": ((0.85, 0.85, 0.82), 0.0, 1.0, None, 0.0, 1.0),
    "wire_green": ((0.02, 0.07, 0.03), 0.0, 0.6, None, 0.0, 1.0),
    "bauble_red": ((0.65, 0.02, 0.02), 0.6, 0.12, None, 0.0, 1.0),
    "bauble_gold": ((0.85, 0.58, 0.14), 0.9, 0.14, None, 0.0, 1.0),
    "bauble_blue": ((0.04, 0.1, 0.6), 0.6, 0.12, None, 0.0, 1.0),
    "bauble_silver": ((0.85, 0.85, 0.9), 0.95, 0.1, None, 0.0, 1.0),
    "tinsel": ((0.8, 0.8, 0.86), 0.9, 0.22, None, 0.0, 1.0),
    "gift_red": ((0.5, 0.03, 0.03), 0.0, 0.45, None, 0.0, 1.0),
    "gift_green": ((0.03, 0.28, 0.08), 0.0, 0.45, None, 0.0, 1.0),
    "gift_blue": ((0.05, 0.12, 0.5), 0.0, 0.45, None, 0.0, 1.0),
    "ribbon_gold": ((0.85, 0.62, 0.16), 0.7, 0.28, None, 0.0, 1.0),
    "ribbon_silver": ((0.8, 0.8, 0.84), 0.8, 0.28, None, 0.0, 1.0),
    "silhouette_snow": ((0.07, 0.09, 0.14), 0.0, 1.0, None, 0.0, 1.0),
    "silhouette_snow_near": ((0.14, 0.17, 0.23), 0.0, 1.0, None, 0.0, 1.0),
    "snow": ((0.85, 0.9, 0.97), 0.0, 0.6, None, 0.0, 1.0),
    # New Year
    "hat_gold": ((0.85, 0.6, 0.14), 0.0, 0.5, None, 0.0, 1.0),
    "hat_blue": ((0.04, 0.12, 0.55), 0.0, 0.5, None, 0.0, 1.0),
    "hat_pink": ((0.7, 0.05, 0.3), 0.0, 0.5, None, 0.0, 1.0),
    "foil_gold": ((0.9, 0.65, 0.16), 0.9, 0.2, None, 0.0, 1.0),
    "balloon_gold": ((0.9, 0.62, 0.14), 0.5, 0.12, None, 0.0, 1.0),
    "balloon_silver": ((0.8, 0.82, 0.88), 0.6, 0.12, None, 0.0, 1.0),
    "balloon_blue": ((0.03, 0.08, 0.6), 0.3, 0.1, None, 0.0, 1.0),
    "balloon_pink": ((0.75, 0.05, 0.35), 0.3, 0.1, None, 0.0, 1.0),
    "balloon_green": ((0.03, 0.4, 0.2), 0.3, 0.1, None, 0.0, 1.0),
    "ribbon_curl": ((0.7, 0.7, 0.75), 0.4, 0.3, None, 0.0, 1.0),
    "streamer_red": ((0.7, 0.04, 0.05), 0.0, 0.5, None, 0.0, 1.0),
    "streamer_gold": ((0.85, 0.6, 0.12), 0.3, 0.4, None, 0.0, 1.0),
    "streamer_blue": ((0.04, 0.2, 0.7), 0.0, 0.5, None, 0.0, 1.0),
    "streamer_green": ((0.04, 0.5, 0.15), 0.0, 0.5, None, 0.0, 1.0),
    "streamer_pink": ((0.8, 0.1, 0.4), 0.0, 0.5, None, 0.0, 1.0),
    "glass_pale": ((0.7, 0.75, 0.78), 0.0, 0.04, None, 0.0, 1.0),
    "glass_green": ((0.012, 0.09, 0.03), 0.0, 0.06, None, 0.0, 1.0),
    "champagne": ((0.85, 0.6, 0.1), 0.0, 0.05, None, 0.0, 1.0),
    "confetti_red": ((0.7, 0.04, 0.05), 0.0, 0.4, None, 0.0, 1.0),
    "confetti_gold": ((0.9, 0.65, 0.1), 0.5, 0.3, None, 0.0, 1.0),
    "confetti_blue": ((0.04, 0.2, 0.8), 0.0, 0.4, None, 0.0, 1.0),
    "confetti_green": ((0.05, 0.6, 0.2), 0.0, 0.4, None, 0.0, 1.0),
    "confetti_pink": ((0.85, 0.12, 0.45), 0.0, 0.4, None, 0.0, 1.0),
}

# How rough each picture's surface is (by the start of its name).
PICTURE_ROUGHNESS = {
    "poster_": 0.32,
    "calendar": 0.75,
    "note": 0.8,
    "manual": 0.4,
    "cd": 0.12,
    "mouse_mat": 0.9,
    "turbo": 0.2,
    "clock": 0.15,
    "phone": 0.12,
    "box": 0.3,
    "spine": 0.3,
    "keys": 0.38,
    "plan": 0.7,
    "badge": 0.25,
}

EMITS = {
    "emit.led_power": (0.1, 1.0, 0.25),
    "emit.led_hdd": (1.0, 0.5, 0.05),
    "emit.led_monitor": (0.15, 1.0, 0.2),
    "emit.digits": (1.0, 0.1, 0.05),
    "emit.window_lit": (1.0, 0.6, 0.25),
    "emit.speaker": (0.15, 1.0, 0.2),
    "emit.fairy": (1.0, 0.76, 0.4),
    "emit.xmas_red": (1.0, 0.1, 0.06),
    "emit.xmas_green": (0.1, 1.0, 0.2),
    "emit.xmas_gold": (1.0, 0.68, 0.14),
    "emit.xmas_blue": (0.15, 0.32, 1.0),
    "emit.star": (1.0, 0.82, 0.4),
}


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.unit_settings.system = 'METRIC'
    world = bpy.data.worlds.new("night")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs[0].default_value = (0.0, 0.0, 0.0, 1.0)
    scene.world = world
    return scene


def material(name):
    m = bpy.data.materials.get(name)
    if m:
        return m
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    bsdf = m.node_tree.nodes.get("Principled BSDF")
    key = name
    if name.startswith("emit."):
        colour = EMITS.get(name, (1.0, 0.8, 0.5))
        if name.startswith("emit.modem"):
            colour = (1.0, 0.15, 0.05) if name.endswith(("0", "1")) else (0.2, 1.0, 0.25)
        base, metal, rough, emit, strength, alpha = colour, 0.0, 0.3, colour, 4.0, 1.0
    elif name.startswith("picture."):
        # (Previews: a mid grey stands in for the game's painting.)
        rough = next((r for k, r in PICTURE_ROUGHNESS.items() if name.removeprefix("picture.").startswith(k)), 0.35)
        base, metal, emit, strength, alpha = (0.6, 0.6, 0.6), 0.0, None, 0.0, 1.0
    else:
        base, metal, rough, emit, strength, alpha = MATERIALS[key]
    bsdf.inputs["Base Color"].default_value = (*base, 1.0)
    bsdf.inputs["Metallic"].default_value = metal
    bsdf.inputs["Roughness"].default_value = rough
    if emit:
        bsdf.inputs["Emission Color"].default_value = (*emit, 1.0)
        bsdf.inputs["Emission Strength"].default_value = strength
    if alpha < 1.0:
        # (See-through by its alpha, not by refraction: Cycles lets light through alpha to
        # what lies beyond, where refractive glass would need caustics.)
        bsdf.inputs["Alpha"].default_value = alpha
    return m


def finish(obj, mat, bevel=0.0, segments=2, smooth=False, angle=35):
    bpy.context.collection.objects.link(obj)
    tag(obj)
    obj.data.materials.append(material(mat))
    if bevel > 0.0:
        mod = obj.modifiers.new("bevel", 'BEVEL')
        mod.width = bevel
        mod.segments = segments
        mod.limit_method = 'ANGLE'
        mod.angle_limit = math.radians(angle)
        mod.harden_normals = True
    for p in obj.data.polygons:
        p.use_smooth = smooth or bevel > 0.0
    if smooth or bevel > 0.0:
        obj.data.set_sharp_from_angle(angle=math.radians(40))
    return obj


def mesh_object(name, bm):
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    return bpy.data.objects.new(name, me)


# ---------------------------------------------------------------------------------------------
# Shapes
# ---------------------------------------------------------------------------------------------

def box(name, lo, hi, mat, bevel=0.002, segments=2):
    """An axis-aligned box from corner `lo` to corner `hi`."""
    lo, hi = Vector(lo), Vector(hi)
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=hi - lo, verts=bm.verts)
    bmesh.ops.translate(bm, vec=(lo + hi) / 2, verts=bm.verts)
    return finish(mesh_object(name, bm), mat, bevel, segments)


def oriented_box(name, centre, size, mat, rot=(0.0, 0.0, 0.0), bevel=0.002, segments=2):
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=Vector(size), verts=bm.verts)
    r = Matrix.Rotation(rot[2], 3, 'Z') @ Matrix.Rotation(rot[1], 3, 'Y') @ Matrix.Rotation(rot[0], 3, 'X')
    bmesh.ops.rotate(bm, cent=Vector((0, 0, 0)), matrix=r, verts=bm.verts)
    bmesh.ops.translate(bm, vec=Vector(centre), verts=bm.verts)
    return finish(mesh_object(name, bm), mat, bevel, segments)


def rounded(name, lo, hi, mat, radius, segments=4):
    """A box with every edge rounded."""
    obj = box(name, lo, hi, mat, bevel=0.0)
    mod = obj.modifiers.new("round", 'BEVEL')
    mod.width = radius
    mod.segments = segments
    mod.limit_method = 'NONE'
    mod.harden_normals = False
    for p in obj.data.polygons:
        p.use_smooth = True
    return obj


def quad(name, corners, mat, uv=True):
    """A face over four corners (counter-clockwise seen from the side it faces), UVs 0..1 from
    the first corner (bottom left) round."""
    bm = bmesh.new()
    verts = [bm.verts.new(Vector(c)) for c in corners]
    face = bm.faces.new(verts)
    if uv:
        layer = bm.loops.layers.uv.new("UVMap")
        for loop, t in zip(face.loops, [(0, 0), (1, 0), (1, 1), (0, 1)]):
            loop[layer].uv = t
    if mat.startswith("picture."):
        c = [Vector(p) for p in corners]
        picture_aspect(mat, (c[1] - c[0]).length / max((c[3] - c[0]).length, 1e-6))
    return finish(mesh_object(name, bm), mat)


def picture_aspect(mat, aspect):
    """Tells the game the shape to paint a picture in (its extras)."""
    material(mat)["aspect"] = round(aspect, 4)


def grid_quad(name, corners, mat, cuts=(1, 1), uv=True):
    """A quad split into cuts[0] × cuts[1] faces (vertices to bake light into), UVs 0..1."""
    a, b, c, d = [Vector(p) for p in corners]
    nx, ny = cuts
    bm = bmesh.new()
    layer = bm.loops.layers.uv.new("UVMap") if uv else None
    grid = [[bm.verts.new(a.lerp(b, i / nx).lerp(d.lerp(c, i / nx), j / ny)) for i in range(nx + 1)] for j in range(ny + 1)]
    for j in range(ny):
        for i in range(nx):
            f = bm.faces.new((grid[j][i], grid[j][i + 1], grid[j + 1][i + 1], grid[j + 1][i]))
            if layer:
                for loop, (u, v) in zip(f.loops, [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)]):
                    loop[layer].uv = (u / nx, v / ny)
    return finish(mesh_object(name, bm), mat)


def cylinder(name, a, b, radius, mat, segments=16, caps=True, bevel=0.0, radius2=None):
    a, b = Vector(a), Vector(b)
    axis = b - a
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=caps, cap_tris=False, segments=segments, radius1=radius, radius2=radius if radius2 is None else radius2, depth=axis.length)
    rot = axis.to_track_quat('Z', 'Y').to_matrix().to_4x4()
    bmesh.ops.transform(bm, matrix=Matrix.Translation((a + b) / 2) @ rot, verts=bm.verts)
    return finish(mesh_object(name, bm), mat, bevel, smooth=True)


def sphere(name, centre, radius, mat, segments=16, scale=(1.0, 1.0, 1.0)):
    bm = bmesh.new()
    bmesh.ops.create_uvsphere(bm, u_segments=segments, v_segments=max(segments // 2, 4), radius=radius)
    bmesh.ops.scale(bm, vec=Vector(scale), verts=bm.verts)
    bmesh.ops.translate(bm, vec=Vector(centre), verts=bm.verts)
    return finish(mesh_object(name, bm), mat, smooth=True)


def lathe(name, profile, centre, mat, segments=40, axis='Z', cap_bottom=False, cap_top=False):
    """A surface of revolution: `profile` is (radius, height) from the bottom up."""
    c = Vector(centre)
    bm = bmesh.new()
    rings = []
    for r, h in profile:
        ring = []
        for k in range(segments):
            a = k / segments * math.tau
            p = Vector((r * math.cos(a), r * math.sin(a), h)) if axis == 'Z' else Vector((r * math.cos(a), h, r * math.sin(a)))
            ring.append(bm.verts.new(c + p))
        rings.append(ring)
    for j in range(len(rings) - 1):
        for k in range(segments):
            k2 = (k + 1) % segments
            bm.faces.new((rings[j][k], rings[j][k2], rings[j + 1][k2], rings[j + 1][k]))
    if cap_bottom:
        bm.faces.new(list(reversed(rings[0])))
    if cap_top:
        bm.faces.new(rings[-1])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return finish(mesh_object(name, bm), mat, smooth=True)


def loft(name, sections, mat, bevel=0.0, cap=True, cap_first=None):
    """Faces through a list of closed sections (each a list of points, all of one count);
    `cap_first` (default: `cap`) closes the first section."""
    bm = bmesh.new()
    rings = [[bm.verts.new(Vector(p)) for p in s] for s in sections]
    n = len(rings[0])
    for j in range(len(rings) - 1):
        for k in range(n):
            k2 = (k + 1) % n
            bm.faces.new((rings[j][k], rings[j][k2], rings[j + 1][k2], rings[j + 1][k]))
    if cap if cap_first is None else cap_first:
        bm.faces.new(list(reversed(rings[0])))
    if cap:
        bm.faces.new(rings[-1])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return finish(mesh_object(name, bm), mat, bevel, smooth=True)


def rounded_rect(cx, cz, hw, hh, r, y, n=5):
    """A rectangle with rounded corners in the x-z plane at depth y, counter-clockwise from the
    bottom left, as seen from -y."""
    pts = []
    for (sx, sz, a0) in [(-1, -1, math.pi), (1, -1, 1.5 * math.pi), (1, 1, 0.0), (-1, 1, 0.5 * math.pi)]:
        ccx, ccz = cx + sx * (hw - r), cz + sz * (hh - r)
        for k in range(n + 1):
            a = a0 + k / n * math.pi / 2
            pts.append((ccx + r * math.cos(a), y, ccz + r * math.sin(a)))
    return pts


def frame_between(name, outer, inner, mat, depth=0.0, bevel=0.0):
    """A flat ring between two loops of one count (outer and inner), `depth` thick towards +y."""
    bm = bmesh.new()
    n = len(outer)
    o = [bm.verts.new(Vector(p)) for p in outer]
    i = [bm.verts.new(Vector(p)) for p in inner]
    for k in range(n):
        k2 = (k + 1) % n
        bm.faces.new((o[k], o[k2], i[k2], i[k]))
    if depth > 0:
        ob = [bm.verts.new(Vector(p) + Vector((0, depth, 0))) for p in outer]
        ib = [bm.verts.new(Vector(p) + Vector((0, depth, 0))) for p in inner]
        for k in range(n):
            k2 = (k + 1) % n
            bm.faces.new((o[k2], o[k], ob[k], ob[k2]))
            bm.faces.new((i[k], i[k2], ib[k2], ib[k]))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return finish(mesh_object(name, bm), mat, bevel)


def cable(name, points, radius, mat="rubber", resolution=8, round_steps=2):
    """A cable through `points`, smoothed (a Bézier curve with a round bevel, `resolution`
    steps between points and `round_steps` round each quarter of it)."""
    cu = bpy.data.curves.new(name, 'CURVE')
    cu.dimensions = '3D'
    cu.bevel_depth = radius
    cu.bevel_resolution = round_steps
    cu.resolution_u = resolution
    sp = cu.splines.new('BEZIER')
    sp.bezier_points.add(len(points) - 1)
    for bp, p in zip(sp.bezier_points, points):
        bp.co = Vector(p)
        bp.handle_left_type = 'AUTO'
        bp.handle_right_type = 'AUTO'
    obj = bpy.data.objects.new(name, cu)
    bpy.context.collection.objects.link(obj)
    obj.data.materials.append(material(mat))
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.select_all(action='DESELECT')
    obj.select_set(True)
    bpy.ops.object.convert(target='MESH')
    obj = bpy.context.view_layer.objects.active
    tag(obj)
    for p in obj.data.polygons:
        p.use_smooth = True
    return obj


def anchor(name, location, scale=(1, 1, 1), rotation=(0, 0, 0)):
    e = bpy.data.objects.new(tagged(f"anchor.{name}"), None)
    e.location = Vector(location)
    e.scale = Vector(scale)
    e.rotation_euler = rotation
    bpy.context.collection.objects.link(e)
    return e


def join(objs, name):
    bpy.ops.object.select_all(action='DESELECT')
    for o in objs:
        bpy.context.view_layer.objects.active = o
        for m in list(o.modifiers):
            bpy.ops.object.modifier_apply(modifier=m.name)
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.object.join()
    obj = bpy.context.view_layer.objects.active
    obj.name = name
    return obj


# ---------------------------------------------------------------------------------------------
# The room
# ---------------------------------------------------------------------------------------------

DESK_TOP = 0.74
WALL_Y = 0.62
LEFT_X = -1.55
RIGHT_X = 2.1
FRONT_Y = -2.2
CEILING = 2.5
WINDOW_Y = (-0.42, 0.52)
WINDOW_Z = (0.98, 2.06)


def shell():
    """Floor, walls (the left one round its window), ceiling and skirting."""
    grid_quad("floor", [(LEFT_X, FRONT_Y, 0), (RIGHT_X, FRONT_Y, 0), (RIGHT_X, WALL_Y, 0), (LEFT_X, WALL_Y, 0)], "carpet", cuts=(18, 14), uv=False)
    grid_quad("ceiling", [(LEFT_X, WALL_Y, CEILING), (RIGHT_X, WALL_Y, CEILING), (RIGHT_X, FRONT_Y, CEILING), (LEFT_X, FRONT_Y, CEILING)], "ceiling", cuts=(12, 10), uv=False)
    # The wall behind the desk, finely cut (the posters' and the beams' light lands on it).
    grid_quad("wall_back", [(LEFT_X, WALL_Y, 0), (RIGHT_X, WALL_Y, 0), (RIGHT_X, WALL_Y, CEILING), (LEFT_X, WALL_Y, CEILING)], "paint", cuts=(72, 50), uv=False)
    grid_quad("wall_right", [(RIGHT_X, WALL_Y, 0), (RIGHT_X, FRONT_Y, 0), (RIGHT_X, FRONT_Y, CEILING), (RIGHT_X, WALL_Y, CEILING)], "paint", cuts=(20, 20), uv=False)
    grid_quad("wall_front", [(RIGHT_X, FRONT_Y, 0), (LEFT_X, FRONT_Y, 0), (LEFT_X, FRONT_Y, CEILING), (RIGHT_X, FRONT_Y, CEILING)], "paint", cuts=(20, 20), uv=False)
    # The left wall in pieces round the window.
    y0, y1 = WINDOW_Y
    z0, z1 = WINDOW_Z
    for name, ya, yb, za, zb, cuts in [
        ("wall_left_a", FRONT_Y, y0, 0, CEILING, (30, 26)),
        ("wall_left_b", y1, WALL_Y, 0, CEILING, (4, 26)),
        ("wall_left_c", y0, y1, 0, z0, (12, 10)),
        ("wall_left_d", y0, y1, z1, CEILING, (12, 5)),
    ]:
        grid_quad(name, [(LEFT_X, yb, za), (LEFT_X, ya, za), (LEFT_X, ya, zb), (LEFT_X, yb, zb)], "paint", cuts=cuts, uv=False)
    # The window's reveal (the wall's thickness round it).
    t = 0.14
    for name, a, b, c, d in [
        ("reveal_bottom", (LEFT_X, y0, z0), (LEFT_X - t, y0, z0), (LEFT_X - t, y1, z0), (LEFT_X, y1, z0)),
        ("reveal_top", (LEFT_X, y1, z1), (LEFT_X - t, y1, z1), (LEFT_X - t, y0, z1), (LEFT_X, y0, z1)),
        ("reveal_front", (LEFT_X, y0, z1), (LEFT_X - t, y0, z1), (LEFT_X - t, y0, z0), (LEFT_X, y0, z0)),
        ("reveal_back", (LEFT_X, y1, z0), (LEFT_X - t, y1, z0), (LEFT_X - t, y1, z1), (LEFT_X, y1, z1)),
    ]:
        grid_quad(name, [a, b, c, d], "paint", cuts=(3, 8), uv=False)
    # Skirting boards.
    box("skirting_back", (LEFT_X, WALL_Y - 0.015, 0), (RIGHT_X, WALL_Y, 0.08), "paint_white", bevel=0.003)
    box("skirting_left", (LEFT_X, FRONT_Y, 0), (LEFT_X + 0.015, WALL_Y, 0.08), "paint_white", bevel=0.003)


# The street across from the window (a house: across, width, height, roof).
HOUSES = [(-3.0, 1.6, 1.1, 0.5), (-1.2, 1.3, 1.4, 0.6), (0.4, 1.8, 1.0, 0.45), (2.2, 1.2, 1.3, 0.55), (3.6, 1.5, 0.9, 0.4)]


def window():
    """The sash window in the left wall: its frame and sill, the glass, the venetian blinds
    (tilted part open), curtains either side, and the night outside: sky, moon, the houses
    across the street."""
    y0, y1 = WINDOW_Y
    z0, z1 = WINDOW_Z
    x = LEFT_X - 0.1
    f = 0.045
    # The frame round the glass, and the bar where the sashes meet.
    for name, lo, hi in [
        ("frame_bottom", (x - 0.03, y0, z0), (x + 0.03, y1, z0 + f)),
        ("frame_top", (x - 0.03, y0, z1 - f), (x + 0.03, y1, z1)),
        ("frame_front", (x - 0.03, y0, z0), (x + 0.03, y0 + f, z1)),
        ("frame_back", (x - 0.03, y1 - f, z0), (x + 0.03, y1, z1)),
        ("frame_middle", (x - 0.03, y0, (z0 + z1) / 2 - 0.022), (x + 0.03, y1, (z0 + z1) / 2 + 0.022)),
    ]:
        box(name, lo, hi, "paint_white", bevel=0.004)
    quad("pane", [(x, y1 - f, z0 + f), (x, y0 + f, z0 + f), (x, y0 + f, z1 - f), (x, y1 - f, z1 - f)], "glass", uv=False)
    # The sill.
    box("sill", (LEFT_X - 0.02, y0 - 0.06, z0 - 0.035), (LEFT_X + 0.05, y1 + 0.06, z0), "paint_white", bevel=0.006)
    # The blinds: aluminium slats, tilted, on ladder cords, under a headrail.
    slat_w, pitch = 0.025, 0.021
    # (Tilted with their inner edges low, so the moon, high outside, shines in between them.)
    tilt = math.radians(-38)
    top = z1 - 0.07
    bottom = z0 + 0.05
    sx = LEFT_X + 0.03
    box("headrail", (sx - 0.02, y0 - 0.02, top + 0.012), (sx + 0.025, y1 + 0.02, top + 0.05), "paint_white", bevel=0.004)
    slats = []
    z = top
    k = 0
    while z > bottom:
        # A slat: a thin, slightly curved strip, tilted about its long axis.
        c = Vector((sx, (y0 + y1) / 2, z))
        n = 6
        bm = bmesh.new()
        rows = []
        for i in range(n + 1):
            u = i / n - 0.5
            curve = 0.0025 * (1 - (2 * u) ** 2)
            p = Vector((u * slat_w * math.cos(tilt) - curve * math.sin(tilt), 0, u * slat_w * math.sin(tilt) + curve * math.cos(tilt)))
            rows.append([bm.verts.new(c + p + Vector((0, y, 0))) for y in ((y0 - y1) / 2 - 0.01, (y1 - y0) / 2 + 0.01)])
        for i in range(n):
            bm.faces.new((rows[i][0], rows[i + 1][0], rows[i + 1][1], rows[i][1]))
        slats.append(mesh_object(f"slat{k}", bm))
        bpy.context.collection.objects.link(slats[-1])
        z -= pitch
        k += 1
    blinds = join(slats, "blinds")
    blinds.data.materials.clear()
    blinds.data.materials.append(material("alu"))
    for p in blinds.data.polygons:
        p.use_smooth = True
    box("bottom_rail", (sx - 0.012, y0 - 0.02, z - 0.004), (sx + 0.012, y1 + 0.02, z + 0.01), "alu", bevel=0.003)
    for i, yy in enumerate((y0 + 0.12, (y0 + y1) / 2, y1 - 0.12)):
        for s in (-1, 1):
            cylinder(f"ladder{i}{s}", (sx + s * 0.011, yy, z), (sx + s * 0.011, yy, top + 0.012), 0.0008, "paper", segments=4, caps=False)
    cable("pull_cord", [(sx - 0.03, y1 - 0.03, top + 0.02), (sx - 0.035, y1 - 0.03, top - 0.4), (sx - 0.04, y1 - 0.032, top - 0.75)], 0.0015, "paper")
    lathe("tassel", [(0.0, 0.0), (0.006, 0.004), (0.007, 0.03), (0.003, 0.04)], (sx - 0.04, y1 - 0.032, top - 0.79), "paint_white", segments=10, cap_top=True)
    # The curtains: heavy folds hanging from a rod, drawn to either side.
    rod_z = z1 + 0.12
    cylinder("rod", (LEFT_X + 0.09, y0 - 0.45, rod_z), (LEFT_X + 0.09, y1 + 0.3, rod_z), 0.011, "brass", segments=12)
    for name, ya, yb in [("curtain_front", y0 - 0.45, y0 + 0.05), ("curtain_back", y1 - 0.08, y1 + 0.3)]:
        bm = bmesh.new()
        nu, nv = 40, 30
        rows = []
        for j in range(nv + 1):
            v = j / nv
            zz = rod_z - 0.02 - v * (rod_z - 0.04)
            row = []
            for i in range(nu + 1):
                u = i / nu
                yy = ya + (yb - ya) * u
                fold = 0.035 * math.sin(u * math.tau * 5.5) * (0.8 + 0.4 * v)
                # Gathered at the top, flaring towards the hem.
                xx = LEFT_X + 0.1 + fold
                row.append(bm.verts.new((xx, yy, zz)))
            rows.append(row)
        for j in range(nv):
            for i in range(nu):
                bm.faces.new((rows[j][i], rows[j][i + 1], rows[j + 1][i + 1], rows[j + 1][i]))
        obj = mesh_object(name, bm)
        finish(obj, "fabric", smooth=True)
    # Outside: the sky (and the moon in it), the houses across the street with a lit window or two.
    sky_x = LEFT_X - 6.0
    grid_quad("sky", [(sky_x, y1 + 5, 0), (sky_x, y0 - 5, 0), (sky_x, y0 - 5, 6), (sky_x, y1 + 5, 6)], "sky", cuts=(4, 4), uv=False)
    moon_at = Vector((LEFT_X, (y0 + y1) / 2, (z0 + z1) / 2)) + MOON_DIR * 5.0
    sphere("moon", moon_at, 0.09, "moon", segments=16)
    houses = []
    for i, (yy, w, h, roof) in enumerate(HOUSES):
        base = 0.4
        bm = bmesh.new()
        pts = [(yy - w / 2, base), (yy + w / 2, base), (yy + w / 2, base + h), (yy, base + h + roof), (yy - w / 2, base + h)]
        verts = [bm.verts.new((sky_x + 1.5, p[0], p[1])) for p in pts]
        bm.faces.new(verts)
        houses.append(mesh_object(f"house{i}", bm))
        bpy.context.collection.objects.link(houses[-1])
        if i in (1, 3):
            wx = sky_x + 1.51
            quad(f"house_window{i}", [(wx, yy + 0.18, base + h * 0.55), (wx, yy - 0.02, base + h * 0.55), (wx, yy - 0.02, base + h * 0.72), (wx, yy + 0.18, base + h * 0.72)], "emit.window_lit", uv=False)
    street = join(houses, "houses")
    street.data.materials.clear()
    street.data.materials.append(material("silhouette"))
    # A tree's dark crown against the sky, nearer.
    sphere("tree", (sky_x + 2.6, y0 - 0.6, 2.3), 0.9, "silhouette", segments=12, scale=(0.6, 1.0, 1.1))
    # (The world outside is a backdrop: it casts no shadow, or the moon would not get in.)
    for o in bpy.context.scene.objects:
        if o.type == 'MESH' and o.data.materials and o.data.materials[0].name in ("sky", "silhouette", "moon", "emit.window_lit"):
            o.visible_shadow = False


def desk():
    """The desk: a wooden top with a rounded front edge, legs, a drawer pedestal on the right,
    a modesty panel at the back."""
    x0, x1 = -0.82, 0.82
    y0, y1 = -0.12, WALL_Y - 0.02
    t = 0.028
    top = grid_quad("desk_top", [(x0, y0, DESK_TOP), (x1, y0, DESK_TOP), (x1, y1, DESK_TOP), (x0, y1, DESK_TOP)], "wood", cuts=(64, 28), uv=False)
    # The edge: a rounded nosing along the front, square at the sides.
    bm = bmesh.new()
    n = 8
    rows = []
    for k in range(n + 1):
        a = -math.pi / 2 + k / n * math.pi
        y = y0 - (t / 2) * math.cos(a) * 0.9
        z = DESK_TOP - t / 2 + (t / 2) * math.sin(a)
        rows.append([bm.verts.new((x, y, z)) for x in (x0, x1)])
    for k in range(n):
        bm.faces.new((rows[k][0], rows[k][1], rows[k + 1][1], rows[k + 1][0]))
    finish(mesh_object("desk_nose", bm), "wood", smooth=True)
    box("desk_under", (x0, y0, DESK_TOP - t), (x1, y1, DESK_TOP - t + 0.002), "wood")
    for x in (x0, x1 - 0.03):
        box(f"desk_side{x}", (x, y0 + 0.02, 0), (x + 0.03, y1, DESK_TOP - t), "wood", bevel=0.003)
    box("modesty", (x0 + 0.03, y1 - 0.03, 0.3), (x1 - 0.03, y1 - 0.012, DESK_TOP - t), "wood", bevel=0.002)
    # The pedestal: three drawers with handles.
    px0, px1 = 0.36, x1 - 0.03
    box("pedestal", (px0, y0 + 0.03, 0.03), (px1, y1 - 0.05, DESK_TOP - t), "wood", bevel=0.003)
    for i, (za, zb) in enumerate([(0.05, 0.3), (0.32, 0.5), (0.52, 0.69)]):
        box(f"drawer{i}", (px0 + 0.01, y0 + 0.022, za), (px1 - 0.01, y0 + 0.032, zb), "wood", bevel=0.004)
        cylinder(f"handle{i}", ((px0 + px1) / 2 - 0.05, y0 + 0.012, zb - 0.04), ((px0 + px1) / 2 + 0.05, y0 + 0.012, zb - 0.04), 0.005, "brass", segments=8)


def monitor():
    """A 19-inch CRT in beige plastic: a front shell round the tube's glass, parted by a seam
    from a deep back tapering to a smaller, lower rear with vents over it; a sculpted bezel, its
    flat band rounding over into a lip that slopes in to the glass; a brushed badge, the controls
    moulded in the bezel's own beige (five buttons, the power switch and its lamp); on a swivel
    foot."""
    # (Its sizes are a 17-inch set's, scaled up.)
    S = 1.12
    cx, cz = 0.0, 1.03
    fy = 0.17  # the bezel's face
    hw, hh = 0.208 * S, 0.198 * S
    gw, gh = 0.162 * S, 0.123 * S
    lift = 0.012 * S
    N = 12
    def ring(y, w, h, zc, r):
        return [(p[0], y, p[2]) for p in rounded_rect(cx, zc, w, h, r, y, n=N)]
    # The case: the front shell to the seam, the seam's dark groove, the back to its rear.
    seam = fy + 0.072 * S
    loft("crt_front", [ring(fy + 0.004, hw, hh, cz, 0.026 * S), ring(fy + 0.014, hw + 0.0012, hh + 0.0012, cz, 0.027 * S), ring(seam - 0.0015, hw, hh, cz, 0.028 * S)], "plastic", cap=False)
    loft("crt_seam", [ring(seam - 0.0015, hw - 0.0025, hh - 0.0025, cz, 0.028 * S), ring(seam + 0.0015, hw - 0.0025, hh - 0.0025, cz, 0.028 * S)], "plastic_dark", cap=False)
    frame_between("crt_seam_front", ring(seam - 0.0015, hw, hh, cz, 0.028 * S), ring(seam - 0.0015, hw - 0.0025, hh - 0.0025, cz, 0.028 * S), "plastic")
    frame_between("crt_seam_back", ring(seam + 0.0015, hw - 0.0005, hh - 0.0008, cz, 0.029 * S), ring(seam + 0.0015, hw - 0.0025, hh - 0.0025, cz, 0.028 * S), "plastic")
    loft("crt_back", [ring(seam + 0.0015, hw - 0.0005, hh - 0.0008, cz, 0.029 * S), ring(fy + 0.15 * S, hw - 0.014 * S, hh - 0.016 * S, cz - 0.003 * S, 0.036 * S), ring(fy + 0.25 * S, hw - 0.044 * S, hh - 0.046 * S, cz - 0.013 * S, 0.048 * S), ring(fy + 0.34 * S, 0.142 * S, 0.132 * S, cz - 0.03 * S, 0.046 * S), ring(fy + 0.39 * S, 0.126 * S, 0.116 * S, cz - 0.04 * S, 0.036 * S)], "plastic", cap_first=False)
    # The bezel, sculpted: from its outer edge a flat band, a soft roll over and the lip sloping
    # in to the glass, as (how far in from the edge to the glass's opening, how far back).
    profile = [(0.0, 0.0), (0.012, -0.0022), (0.38, -0.003), (0.62, -0.0012), (0.8, 0.003), (0.9, 0.009), (0.96, 0.017), (1.0, 0.03)]
    rings = []
    for t, back in profile:
        w = hw + (gw - hw) * t
        h = hh + (gh - hh) * t
        zc = cz + lift * t
        r = 0.026 * S + (0.012 * S - 0.026 * S) * t
        rings.append(ring(fy + 0.004 + back, w, h, zc, r))
    bezel = loft("crt_bezel", rings, "plastic", cap=False)
    for p in bezel.data.polygons:
        p.use_smooth = True
    # The tube's dark throat behind the glass (the renderer draws the glass itself).
    box("crt_throat", (cx - gw, fy + 0.035, cz + lift - gh), (cx + gw, fy + 0.06, cz + lift + gh), "throat", bevel=0.0)
    # (Its scale: half the glass's width, how far it bulges, half its height; in glTF's axes
    # half width, half height, bulge.)
    anchor("screen", (cx, fy + 0.028, cz + lift), scale=(gw - 0.004, 0.012 * S, gh - 0.004))
    # A brushed badge on the bezel's band, top left, and its maker's name plate under the glass.
    rounded("crt_badge", (cx - hw + 0.03, fy - 0.0012, cz + hh - 0.034), (cx - hw + 0.082, fy + 0.0015, cz + hh - 0.022), "alu", 0.0025, 3)
    # The controls under the glass: five small buttons and the power switch, moulded in the
    # bezel's own beige and a millimetre proud of it (no strip, no frame, nothing dark round
    # them), the power lamp beside them. `face` is the bezel's flat band there.
    bottom = cz - hh
    face = fy + 0.00125
    zc = bottom + 0.026
    for i in range(5):
        x = cx - 0.11 * S + i * 0.03 * S
        rounded(f"crt_button{i}", (x - 0.0085, face - 0.0010, zc - 0.0052), (x + 0.0085, face + 0.004, zc + 0.0052), "plastic", 0.0015, 3)
    rounded("crt_power", (cx + 0.098 * S, face - 0.0012, zc - 0.0078), (cx + 0.13 * S, face + 0.004, zc + 0.0078), "plastic", 0.0016, 3)
    cylinder("crt_led", (cx + 0.145 * S, face - 0.0004, zc), (cx + 0.145 * S, face + 0.003, zc), 0.0018, "emit.led_monitor", segments=16)
    anchor("led_monitor", (cx + 0.145 * S, face - 0.01, zc))
    # Vents across the top of the back.
    for i in range(14):
        y = fy + 0.12 * S + i * 0.016 * S
        box(f"crt_vent{i}", (cx - 0.11 * S, y, cz + hh - 0.012 - (y - fy) * 0.18), (cx + 0.11 * S, y + 0.006, cz + hh - 0.006 - (y - fy) * 0.18), "plastic_dark", bevel=0.0)
    # The swivel foot: a disc with a lip, the turntable on it, and the neck up to the case.
    lathe("crt_foot", [(0.0, DESK_TOP), (0.124 * S, DESK_TOP), (0.13 * S, DESK_TOP + 0.003), (0.131 * S, DESK_TOP + 0.009), (0.126 * S, DESK_TOP + 0.016), (0.11 * S, DESK_TOP + 0.022), (0.07 * S, DESK_TOP + 0.026), (0.066 * S, DESK_TOP + 0.03)], (cx, fy + 0.18 * S, 0.0), "plastic", segments=72, cap_top=True)
    lathe("crt_turntable", [(0.064 * S, DESK_TOP + 0.03), (0.064 * S, DESK_TOP + 0.034), (0.06 * S, DESK_TOP + 0.036), (0.0, DESK_TOP + 0.036)], (cx, fy + 0.18 * S, 0.0), "plastic_dark", segments=64)
    rounded("crt_neck", (cx - 0.06 * S, fy + 0.1 * S, DESK_TOP + 0.03), (cx + 0.06 * S, fy + 0.26 * S, bottom + 0.012), "plastic", 0.012, 3)
    # Its cables off the back: the signal to the computer, the power to the wall.
    back = fy + 0.39 * S
    cable("crt_signal", [(cx - 0.03, back - 0.02, cz - 0.08), (cx - 0.08, back + 0.03, cz - 0.12), (-0.35, WALL_Y - 0.06, DESK_TOP + 0.01), (-0.56, 0.5, DESK_TOP + 0.08)], 0.004, "rubber")
    cable("crt_mains", [(cx + 0.03, back - 0.02, cz - 0.09), (cx + 0.08, back + 0.04, DESK_TOP + 0.05), (0.2, WALL_Y - 0.02, DESK_TOP + 0.004), (0.3, WALL_Y - 0.01, 0.3)], 0.0035, "rubber")
    # A sticky note on the bezel: what it says is painted by the game.
    quad("note", [(cx + 0.168 * S, fy - 0.0026, cz + 0.13 * S), (cx + 0.205 * S, fy - 0.0026, cz + 0.13 * S), (cx + 0.207 * S, fy - 0.0026, cz + 0.17 * S), (cx + 0.17 * S, fy - 0.0026, cz + 0.169 * S)], "picture.note")
    return cx, cz

def tower():
    """The mini-tower to the left: one moulded beige case, its front rounded into its sides.
    Nothing stands out from it but what a real one's does, a millimetre or two: the drive
    bezels in the same plastic parted by hairline seams (the CD-ROM with its tray, headphone
    socket, volume wheel, lamp and eject button; a blank; the floppy drive in its frame), the
    turbo display (its digits painted by the game), the keylock, the power and reset buttons,
    the lamps, the badge, slots at the foot; a seam and fine slots down the side the viewer
    sees; rubber feet. Only the slits themselves are dark, and then a brown, not a black."""
    x0, x1 = -0.75, -0.555
    fy = 0.1
    y1 = 0.54
    zb, zt = DESK_TOP + 0.008, DESK_TOP + 0.42
    rounded("tower_case", (x0, fy, zb), (x1, y1, zt), "plastic", 0.006, 4)
    for i, (fx, fyy) in enumerate([(x0 + 0.02, fy + 0.03), (x1 - 0.02, fy + 0.03), (x0 + 0.02, y1 - 0.03), (x1 - 0.02, y1 - 0.03)]):
        cylinder(f"tower_foot{i}", (fx, fyy, DESK_TOP), (fx, fyy, zb + 0.001), 0.011, "rubber", segments=16)
    cx = (x0 + x1) / 2

    def plate(name, xa, xb, za, zc, proud, r=0.0012, mat="plastic"):
        """A moulded plate `proud` of the case's face, its edges rounded."""
        return rounded(name, (xa, fy - proud, za), (xb, fy + 0.003, zc), mat, r, 3)

    def seam(name, xa, xb, za, zc, level=0.0):
        """A hairline of the seam's brown, on a face `level` proud of the case's."""
        return box(name, (xa, fy - level - 0.0003, za), (xb, fy + 0.002, zc), "seam", bevel=0.0)

    gap = 0.0008
    half = 0.075
    bay = 0.043
    # The CD-ROM: its bezel, the tray's door over it with the slit under the door, the socket,
    # the wheel, the lamp and the eject button along the foot.
    cd_top = zt - 0.014
    cd_bot = cd_top - bay
    plate("cdrom", cx - half, cx + half, cd_bot, cd_top, 0.0012)
    tray_top = cd_top - 0.0025
    tray_bot = tray_top - 0.024
    plate("cd_tray", cx - 0.064, cx + 0.064, tray_bot, tray_top, 0.0017, r=0.0008)
    seam("cd_tray_slit", cx - 0.064, cx + 0.064, tray_bot - 0.0012, tray_bot - 0.0002, level=0.0012)
    zc = (cd_bot + tray_bot) / 2
    cylinder("cd_socket", (cx - 0.064, fy - 0.0018, zc), (cx - 0.064, fy + 0.002, zc), 0.0034, "plastic", segments=24)
    cylinder("cd_socket_hole", (cx - 0.064, fy - 0.0020, zc), (cx - 0.064, fy + 0.002, zc), 0.0014, "throat", segments=16)
    cylinder("cd_wheel", (cx - 0.049, fy - 0.0022, zc), (cx - 0.049, fy + 0.002, zc), 0.0042, "plastic", segments=32, bevel=0.0005)
    sphere("cd_led", (cx + 0.044, fy - 0.0013, zc), 0.0016, "emit.led_hdd", segments=10)
    plate("cd_eject", cx + 0.052, cx + 0.065, zc - 0.003, zc + 0.003, 0.0026, r=0.0009)
    # A blank under it, and the floppy drive in its frame.
    blank_top = cd_bot - gap
    blank_bot = blank_top - bay
    seam("bay_gap0", cx - half, cx + half, blank_top, cd_bot)
    plate("bay_blank", cx - half, cx + half, blank_bot, blank_top, 0.0012)
    frame_top = blank_bot - gap
    frame_bot = frame_top - bay
    seam("bay_gap1", cx - half, cx + half, frame_top, blank_bot)
    plate("floppy_frame", cx - half, cx + half, frame_bot, frame_top, 0.0012)
    zf = (frame_top + frame_bot) / 2
    plate("floppy_drive", cx - 0.052, cx + 0.052, zf - 0.0135, zf + 0.0135, 0.0019, r=0.0008)
    box("floppy_slot", (cx - 0.036, fy - 0.0022, zf + 0.0038), (cx + 0.026, fy + 0.002, zf + 0.0058), "plastic_dark", bevel=0.0)
    plate("floppy_eject", cx + 0.033, cx + 0.045, zf - 0.0095, zf - 0.0055, 0.0031, r=0.0009)
    sphere("floppy_led", (cx - 0.043, fy - 0.0017, zf - 0.0075), 0.0011, "seam", segments=8)
    # The turbo display (the game paints its digits) in its window, flush with the face.
    zd = frame_bot - 0.022
    quad("display", [(cx - 0.030, fy - 0.0005, zd - 0.0075), (cx + 0.030, fy - 0.0005, zd - 0.0075), (cx + 0.030, fy - 0.0005, zd + 0.0075), (cx - 0.030, fy - 0.0005, zd + 0.0075)], "picture.turbo")
    anchor("turbo", (cx, fy - 0.02, zd))
    # The keylock, the power switch, reset and turbo; the lamps under them.
    zk = frame_bot - 0.062
    cylinder("keylock", (cx - 0.054, fy - 0.0014, zk), (cx - 0.054, fy + 0.002, zk), 0.0065, "chrome", segments=32)
    box("keylock_slot", (cx - 0.0556, fy - 0.0017, zk - 0.0045), (cx - 0.0524, fy + 0.002, zk + 0.0045), "throat", bevel=0.0)
    plate("button_power", cx - 0.030, cx - 0.008, zk - 0.0055, zk + 0.0055, 0.0022, r=0.0012)
    plate("button_reset", cx + 0.008, cx + 0.017, zk - 0.0045, zk + 0.0045, 0.0014, r=0.0008)
    plate("button_turbo", cx + 0.027, cx + 0.036, zk - 0.0045, zk + 0.0045, 0.0014, r=0.0008)
    zl = zk - 0.026
    cylinder("led_power", (cx - 0.020, fy - 0.0008, zl), (cx - 0.020, fy + 0.002, zl), 0.0017, "emit.led_power", segments=16)
    cylinder("led_hdd", (cx + 0.012, fy - 0.0008, zl), (cx + 0.012, fy + 0.002, zl), 0.0017, "emit.led_hdd", segments=16)
    anchor("led_power", (cx - 0.020, fy - 0.014, zl))
    anchor("led_hdd", (cx + 0.012, fy - 0.014, zl))
    # The badge (the game paints it), flush, and slots at the foot.
    zg = zl - 0.04
    quad("tower_badge_face", [(cx - 0.012, fy - 0.0005, zg - 0.012), (cx + 0.012, fy - 0.0005, zg - 0.012), (cx + 0.012, fy - 0.0005, zg + 0.012), (cx - 0.012, fy - 0.0005, zg + 0.012)], "picture.badge")
    for i in range(7):
        zz = zb + 0.018 + i * 0.0075
        box(f"tower_slot{i}", (cx - 0.052, fy - 0.0003, zz), (cx + 0.052, fy + 0.002, zz + 0.0014), "seam", bevel=0.0)
    # Down the side the viewer sees: the side panel's seam and fine slots.
    box("tower_side_seam", (x1 - 0.0003, fy + 0.03, zb + 0.012), (x1 + 0.0004, fy + 0.0318, zt - 0.012), "seam", bevel=0.0)
    for i in range(14):
        zz = zt - 0.14 + i * 0.0075
        box(f"tower_side_vent{i}", (x1 - 0.0003, fy + 0.07, zz), (x1 + 0.0004, fy + 0.15, zz + 0.0016), "seam", bevel=0.0)
    return x0, x1, zt


def modem(x0, x1, top):
    """An external modem lying on the tower: a dark body under a ribbed lid, its lamps along a
    smoked strip at the front (the game blinks them as it talks), a printed label under each,
    rubber feet, the phone cord off the back."""
    y0, y1 = 0.11, 0.33
    xa, xb = x0 + 0.005, x1 - 0.005
    for i, x in enumerate((xa + 0.015, xb - 0.015)):
        for j, y in enumerate((y0 + 0.02, y1 - 0.02)):
            cylinder(f"modem_foot{i}{j}", (x, y, top), (x, y, top + 0.004), 0.006, "rubber", segments=12)
    rounded("modem", (xa, y0, top + 0.004), (xb, y1, top + 0.03), "plastic_dark", 0.006, 4)
    rounded("modem_lid", (xa + 0.006, y0 + 0.012, top + 0.026), (xb - 0.006, y1 - 0.008, top + 0.036), "plastic_dark", 0.004, 3)
    for k in range(7):
        y = y0 + 0.03 + k * 0.024
        box(f"modem_rib{k}", (xa + 0.014, y, top + 0.0355), (xb - 0.014, y + 0.0035, top + 0.0368), "rubber", bevel=0.0)
    # The lamps' strip, smoked, and a label printed under each lamp.
    rounded("modem_strip", (xa + 0.012, y0 - 0.0018, top + 0.008), (xb - 0.012, y0 + 0.004, top + 0.018), "throat", 0.002, 2)
    for i in range(8):
        x = x0 + 0.03 + i * 0.019
        sphere(f"modem_led{i}", (x, y0 - 0.001, top + 0.012), 0.0018, f"emit.modem{i}", segments=10)
        anchor(f"modem{i}", (x, y0 - 0.008, top + 0.012))
        box(f"modem_label{i}", (x - 0.0045, y0 - 0.0006, top + 0.0055), (x + 0.0045, y0 + 0.0002, top + 0.0068), "paper", bevel=0.0)
    cable("phone_cord", [(x1 - 0.02, y1, top + 0.016), (x1 + 0.03, y1 + 0.08, top), (x1 + 0.06, WALL_Y - 0.01, DESK_TOP + 0.3), (x1 + 0.08, WALL_Y - 0.01, 0.35)], 0.002, "paper")


# The legends of the 101 keys, row by row as `keyboard` lays them out (a line break puts the
# shifted sign above; `<up>` and the like are arrows).
KEY_LEGENDS = [
    ["Esc", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "Print\nScreen", "Scroll\nLock", "Pause"],
    ["~\n`", "!\n1", "@\n2", "#\n3", "$\n4", "%\n5", "^\n6", "&\n7", "*\n8", "(\n9", ")\n0", "_\n-", "+\n=", "<back>",
     "Insert", "Home", "Page\nUp", "Num\nLock", "/", "*", "-"],
    ["Tab", "Q", "W", "E", "R", "T", "Y", "U", "I", "O", "P", "{\n[", "}\n]", "|\n\\", "Delete", "End", "Page\nDown",
     "7\nHome", "8\n<up>", "9\nPgUp", "+"],
    ["Caps Lock", "A", "S", "D", "F", "G", "H", "J", "K", "L", ":\n;", "\"\n'", "Enter", "4\n<left>", "5", "6\n<right>"],
    ["Shift", "Z", "X", "C", "V", "B", "N", "M", "<\n,", ">\n.", "?\n/", "Shift", "<up>", "1\nEnd", "2\n<down>", "3\nPgDn", "Enter"],
    ["Ctrl", "Alt", "", "Alt", "Ctrl", "<left>", "<down>", "<right>", "0\nIns", ".\nDel"],
]
# The legends' picture is this many key widths across (and as many down).
ATLAS = 24.0


def legend_top(key, cell):
    """Gives a key's flat top the legends' picture, mapped onto its cell (u0, v0, u1, v1 from the
    picture's top left; the key's back edge at the cell's top)."""
    bpy.context.view_layer.objects.active = key
    bpy.ops.object.modifier_apply(modifier="round")
    me = key.data
    me.materials.append(material("picture.keys"))
    uv = me.uv_layers[0] if me.uv_layers else me.uv_layers.new(name="UVMap")
    top = max(v.co.z for v in me.vertices)
    flat = [p for p in me.polygons if p.normal.z > 0.97 and p.center.z > top - 0.0005]
    xs = [me.vertices[i].co.x for p in flat for i in p.vertices]
    ys = [me.vertices[i].co.y for p in flat for i in p.vertices]
    x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)
    for p in flat:
        p.material_index = 1
        for li in p.loop_indices:
            co = me.vertices[me.loops[li].vertex_index].co
            u = cell[0] + (co.x - x0) / (x1 - x0) * (cell[2] - cell[0])
            v = cell[1] + (y1 - co.y) / (y1 - y0) * (cell[3] - cell[1])
            # (Blender's v runs up the picture, glTF's down.)
            uv.data[li].uv = (u, 1.0 - v)


def keycap(name, lo, hi, mat):
    """A keycap: a box from `lo` to `hi` whose top is drawn in from its foot (more at the back,
    where the keys lean), every edge rounded."""
    lo, hi = Vector(lo), Vector(hi)
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=hi - lo, verts=bm.verts)
    bmesh.ops.translate(bm, vec=(lo + hi) / 2, verts=bm.verts)
    mid = (lo + hi) / 2
    for v in bm.verts:
        if v.co.z > mid.z:
            v.co.x = mid.x + (v.co.x - mid.x) - math.copysign(0.0016, v.co.x - mid.x)
            v.co.y = mid.y + (v.co.y - mid.y) - (0.0022 if v.co.y > mid.y else 0.0012) * math.copysign(1.0, v.co.y - mid.y)
    obj = finish(mesh_object(name, bm), mat)
    mod = obj.modifiers.new("round", 'BEVEL')
    mod.width = 0.0014
    mod.segments = 3
    mod.limit_method = 'NONE'
    for p in obj.data.polygons:
        p.use_smooth = True
    return obj


def keyboard():
    """A beige keyboard, tilted on its feet, every key a keycap with its legend."""
    u = 0.019
    cx, cy = 0.0, -0.025
    tilt = math.radians(6)
    width = 0.462
    depth = 0.17
    # The case: a wedge, its top rising towards the back.
    front, back = cy - depth / 2, cy + depth / 2
    def top_at(y):
        return DESK_TOP + 0.02 + (y - front) * math.tan(tilt)
    xa, xb = cx - width / 2, cx + width / 2
    sections = [[(x, front, DESK_TOP), (x, back, DESK_TOP), (x, back, top_at(back)), (x, front, top_at(front))] for x in (xa, xb)]
    base = loft("kb_base", sections, "plastic", cap=True)
    mod = base.modifiers.new("round", 'BEVEL')
    mod.width = 0.005
    mod.segments = 3
    mod.limit_method = 'ANGLE'
    for p in base.data.polygons:
        p.use_smooth = True
    rows = [
        [("esc", 1), (None, 1), *[("f", 1)] * 4, (None, 0.5), *[("f", 1)] * 4, (None, 0.5), *[("f", 1)] * 4, (None, 0.25), *[("k", 1)] * 3, (None, 0.25), *[(None, 1)] * 4],
        [*[("k", 1)] * 13, ("d", 2), (None, 0.25), *[("k", 1)] * 3, (None, 0.25), *[("k", 1)] * 4],
        [("d", 1.5), *[("k", 1)] * 12, ("d", 1.5), (None, 0.25), *[("k", 1)] * 3, (None, 0.25), *[("k", 1)] * 3, ("k", 1)],
        [("d", 1.75), *[("k", 1)] * 11, ("d", 2.25), (None, 3.5), *[("k", 1)] * 3, (None, 1)],
        [("d", 2.25), *[("k", 1)] * 10, ("d", 2.75), (None, 1.25), ("k", 1), (None, 1.25), *[("k", 1)] * 3, ("d", 1)],
        [("d", 1.5), (None, 1), ("d", 1.5), ("k", 7), ("d", 1.5), (None, 1), ("d", 1.5), (None, 0.25), *[("k", 1)] * 3, (None, 0.25), ("k", 2), ("k", 1), (None, 1)],
    ]
    left = cx - width / 2 + 0.012
    keys = []
    # Each key's top shows its legend: a cell of `picture.keys` as wide as the key, packed in
    # rows of ATLAS units (the game paints them, see the module's notes).
    legends = [label for row in KEY_LEGENDS for label in row]
    atlas_x, atlas_y = 0.0, 0.0
    cells = []
    for r, row in enumerate(rows):
        x = left
        yr = cy + depth / 2 - 0.022 - r * u - (0.006 if r > 0 else 0.0)
        for kind, w in row:
            if kind is not None:
                kw = w * u - 0.0035
                kd = u - 0.0035
                # (Sunk a little into the case's top, which rises towards the back.)
                z0 = top_at(yr) - 0.003
                mat = "keycap_dark" if kind in ("d", "esc") else "keycap"
                k = keycap(f"key{r}_{len(keys)}", (x + 0.00175, yr - kd / 2, z0), (x + 0.00175 + kw, yr + kd / 2, z0 + 0.0095), mat)
                if atlas_x + w > ATLAS:
                    atlas_x, atlas_y = 0.0, atlas_y + 1.0
                cell = (atlas_x / ATLAS, atlas_y / ATLAS, (atlas_x + w) / ATLAS, (atlas_y + 1.0) / ATLAS)
                atlas_x += w
                legend_top(k, cell)
                colour = MATERIALS[mat][0]
                cells.append([legends[len(keys)], *[round(c, 5) for c in cell], *colour])
                keys.append(k)
            x += w * u
    assert len(keys) == len(legends), (len(keys), len(legends))
    picture_aspect("picture.keys", 1.0)
    anchor("keys", (cx, cy, DESK_TOP))["keys"] = json.dumps(cells)
    # The lamps (num lock, caps, scroll) top right.
    for i in range(3):
        sphere(f"kb_lamp{i}", (cx + width / 2 - 0.07 + i * 0.018, back - 0.012, top_at(back - 0.012) + 0.0005), 0.0015, "plastic_dark", segments=6)
    cable("kb_cable", [(cx + 0.05, cy + depth / 2, DESK_TOP + 0.01), (cx + 0.1, cy + depth / 2 + 0.06, DESK_TOP + 0.004), (-0.3, 0.42, DESK_TOP + 0.004), (-0.54, 0.5, DESK_TOP + 0.02)], 0.0025)
    return cx, cy


def mouse_and_mat():
    """A two-button ball mouse on its mat (the mat's picture painted by the game), its cable
    over the desk to the computer."""
    mx, my = 0.36, -0.01
    quad("mouse_mat", [(mx - 0.11, my - 0.1, DESK_TOP + 0.0034), (mx + 0.11, my - 0.1, DESK_TOP + 0.0034), (mx + 0.11, my + 0.1, DESK_TOP + 0.0034), (mx - 0.11, my + 0.1, DESK_TOP + 0.0034)], "picture.mouse_mat")
    box("mat_base", (mx - 0.11, my - 0.1, DESK_TOP), (mx + 0.11, my + 0.1, DESK_TOP + 0.0028), "rubber", bevel=0.001)
    bx, by = mx + 0.01, my + 0.0
    L = 0.112
    point = mouse_shell("mouse", bx, by, L)
    # The buttons: split down the middle, parted from the palm by a seam.
    def strip(name, points, width=0.0011):
        bm = bmesh.new()
        rows = []
        for k, p in enumerate(points):
            q = points[min(k + 1, len(points) - 1)] - points[max(k - 1, 0)]
            side = q.cross(Vector((0, 0, 1))).normalized() * (width / 2)
            lift = Vector((0, 0, 0.0004))
            rows.append((bm.verts.new(p + side + lift), bm.verts.new(p - side + lift)))
        for k in range(len(rows) - 1):
            bm.faces.new((rows[k][0], rows[k + 1][0], rows[k + 1][1], rows[k][1]))
        finish(mesh_object(name, bm), "plastic_dark")
    strip("mouse_split", [point(t, 0.5) for t in [0.6 + k * 0.0375 for k in range(11)]])
    strip("mouse_seam", [point(0.6, a) for a in [0.14 + k * 0.036 for k in range(21)]])
    cable("mouse_cable", [(bx, by + L / 2 - 0.002, DESK_TOP + 0.01), (bx - 0.01, by + 0.12, DESK_TOP + 0.004), (0.2, 0.36, DESK_TOP + 0.004), (-0.1, 0.48, DESK_TOP + 0.004), (-0.54, 0.46, DESK_TOP + 0.03)], 0.0018)


def mouse_shell(name, cx, cy, length):
    """A two-button mouse's shell: rounded in plan, a hump under the palm (towards -y, the
    viewer), low over the buttons (towards +y, where the cable leaves). Returns its surface as a
    function of (t, a): t along it from the palm's end, a across it from the right (0..1)."""
    W, H = 0.06, 0.034
    def point(t, a):
        y = cy - length / 2 + length * t
        end = max(1.0 - abs(2.0 * t - 1.0) ** 2.6, 0.0) ** (1.0 / 2.6)
        half_w = W / 2 * end * (1.0 - 0.13 * t)
        hump = math.exp(-((t - 0.36) / 0.46) ** 2)
        h = H * (0.4 + 0.6 * hump) * end ** 0.8
        c = math.cos(a * math.pi)
        return Vector((cx + half_w * math.copysign(abs(c) ** 0.55, c), y, DESK_TOP + 0.0035 + h * math.sin(a * math.pi) ** 0.6))
    nu, nv = 28, 44
    bm = bmesh.new()
    rings = [[bm.verts.new(point(0.004 + 0.992 * j / nv, i / nu)) for i in range(nu + 1)] for j in range(nv + 1)]
    for j in range(nv):
        for i in range(nu):
            bm.faces.new((rings[j][i], rings[j][i + 1], rings[j + 1][i + 1], rings[j + 1][i]))
    bm.faces.new(rings[0])
    bm.faces.new(list(reversed(rings[-1])))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    finish(mesh_object(name, bm), "mouse", smooth=True, angle=60)
    return point


def speakers():
    """Two little beige speakers either side of the monitor: rounded cabinets, a slotted grille
    over each driver, a bass port, the volume wheel and its lamp on the right one."""
    for s in (-1, 1):
        x = s * 0.31
        y0, y1 = 0.25, 0.36
        rounded(f"speaker{s}", (x - 0.055, y0, DESK_TOP), (x + 0.055, y1, DESK_TOP + 0.2), "plastic", 0.009, 4)
        # The grille's recess and its slots, the cone dark behind them.
        rounded(f"grille{s}", (x - 0.043, y0 - 0.0035, DESK_TOP + 0.062), (x + 0.043, y0 + 0.004, DESK_TOP + 0.182), "foam", 0.006, 3)
        for i in range(13):
            zz = DESK_TOP + 0.068 + i * 0.0084
            rounded(f"grille{s}_bar{i}", (x - 0.04, y0 - 0.0042, zz), (x + 0.04, y0 - 0.001, zz + 0.0042), "plastic", 0.0016, 2)
        # The bass port, and the wheel and lamp on the right one.
        cylinder(f"speaker_port{s}", (x - 0.022, y0 - 0.001, DESK_TOP + 0.032), (x - 0.022, y0 + 0.004, DESK_TOP + 0.032), 0.0095, "throat", segments=24)
        lathe(f"speaker_port_rim{s}", [(0.0095, -0.001), (0.0118, -0.0016), (0.0122, 0.0), (0.0118, 0.002)], (x - 0.022, y0, DESK_TOP + 0.032), "plastic_dark", segments=32, axis='Y')
        if s > 0:
            cylinder(f"speaker_knob{s}", (x + 0.026, y0 - 0.0005, DESK_TOP + 0.03), (x + 0.026, y0 - 0.008, DESK_TOP + 0.03), 0.0085, "plastic_dark", segments=40)
            for k in range(20):
                a = k / 20 * math.tau
                p = Vector((x + 0.026 + 0.0085 * math.cos(a), y0 - 0.0042, DESK_TOP + 0.03 + 0.0085 * math.sin(a)))
                oriented_box(f"speaker_knurl{s}_{k}", tuple(p), (0.0012, 0.0068, 0.0012), "plastic_dark", rot=(0, a, 0), bevel=0.0)
            sphere("speaker_led", (x + 0.04, y0 - 0.001, DESK_TOP + 0.03), 0.0018, "emit.speaker", segments=10)



# The lava lamp's glass, (radius, height above the desk): wide over the bulb, tapering up.
LAVA_GLASS = [(0.042, 0.135), (0.0465, 0.15), (0.0485, 0.17), (0.0488, 0.2), (0.0472, 0.24), (0.0442, 0.28), (0.0402, 0.32), (0.0355, 0.355), (0.0315, 0.375)]


def lava_lamp():
    """A lava lamp in the corner of the desk: a purple metal base flaring to the desk on a
    ribbed foot, its collar under the glass, the glass full of glowing wax (the renderer stirs
    it), a cap rolled over at its rim."""
    cx, cy = 0.66, 0.46
    # The base: a ribbed foot, a cone narrowing up to the collar under the glass. (Its profile,
    # (radius, height above the desk), as the renderer's `lamp_radius` has the glass.)
    base = [(0.0, 0.0), (0.0655, 0.0), (0.0678, 0.0012), (0.0686, 0.0036), (0.0681, 0.0062), (0.0664, 0.0078), (0.0668, 0.0094), (0.0672, 0.0112), (0.0662, 0.013),
            (0.064, 0.016), (0.0595, 0.03), (0.0548, 0.05), (0.0498, 0.072), (0.0452, 0.096), (0.0418, 0.117), (0.0405, 0.1235),
            (0.0438, 0.1255), (0.0452, 0.128), (0.0452, 0.1325), (0.0436, 0.135)]
    lathe("lava_base", [(r, DESK_TOP + h) for r, h in base], (cx, cy, 0.0), "lava_metal", segments=96, cap_bottom=True)
    lathe("lava_glass", [(r, DESK_TOP + h) for r, h in LAVA_GLASS], (cx, cy, 0.0), "lava", segments=96)
    top_r, top_h = LAVA_GLASS[-1]
    cap = [(top_r, top_h), (top_r + 0.0018, top_h + 0.0012), (top_r + 0.0024, top_h + 0.0036), (top_r + 0.0016, top_h + 0.006), (0.0318, top_h + 0.012),
           (0.0294, top_h + 0.026), (0.0262, top_h + 0.04), (0.0236, top_h + 0.05), (0.0218, top_h + 0.0545), (0.0175, top_h + 0.0572), (0.009, top_h + 0.0588), (0.0, top_h + 0.0592)]
    lathe("lava_cap", [(r, DESK_TOP + h) for r, h in cap], (cx, cy, 0.0), "lava_metal", segments=96)
    anchor("lava", (cx, cy, DESK_TOP + 0.24))
    # Its switch on the cord, the cord off to the wall.
    cable("lava_cable", [(cx, cy + 0.065, DESK_TOP + 0.004), (cx - 0.03, cy + 0.11, DESK_TOP + 0.004), (cx - 0.05, WALL_Y - 0.02, DESK_TOP + 0.004), (cx - 0.08, WALL_Y - 0.01, 0.3)], 0.002, "rubber")
    oriented_box("lava_switch", (cx - 0.032, cy + 0.112, DESK_TOP + 0.007), (0.034, 0.016, 0.011), "plastic_dark", rot=(0, 0, 0.6), bevel=0.003)
    return cx, cy

def clock_radio():
    """A clock radio at the back of the desk, its red digits telling how late it is (painted by
    the game), a speaker grille beside them."""
    cx, cy = 0.47, 0.47
    w, d, h = 0.15, 0.085, 0.055
    a = math.radians(-12)
    def at(px, py, pz):
        return (cx + px * math.cos(a) - py * math.sin(a), cy + px * math.sin(a) + py * math.cos(a), DESK_TOP + pz)
    # The body, its face sloping back a little.
    oriented_box("clock", (cx, cy, DESK_TOP + h / 2), (w, d, h), "plastic_dark", rot=(0, 0, a), bevel=0.006)
    # The display's window and the digits in it, the grille to its right, buttons on top.
    f = -d / 2 - 0.0012
    quad("clock_display", [at(-0.065, f, 0.012), at(0.015, f, 0.012), at(0.015, f, 0.042), at(-0.065, f, 0.042)], "picture.clock")
    for i in range(6):
        x = 0.028 + i * 0.0075
        oriented_box(f"clock_slot{i}", at(x, f + 0.0006, 0.027), (0.0035, 0.002, 0.026), "grille", rot=(0, 0, a), bevel=0.0)
    for i in range(3):
        oriented_box(f"clock_button{i}", at(-0.04 + i * 0.03, 0.0, h + 0.002), (0.02, 0.012, 0.004), "plastic", rot=(0, 0, a), bevel=0.0012)
    cable("clock_cable", [at(0.06, d / 2, 0.01), (cx + 0.1, WALL_Y - 0.02, DESK_TOP + 0.004), (cx + 0.12, WALL_Y - 0.01, 0.3)], 0.0018, "rubber")


def desk_lamp():
    """An anglepoise lamp at the back left, switched off: a weighted base, two arms of twin rods
    with the springs that balance them, knuckles at the joints, a conical shade rolled at its
    rim and its switch."""
    # (In the corner behind the pen cup, its head reaching forward over the desk's left: clear of
    # the monitor, the speaker and the computer.)
    bx, by = -0.47, 0.52
    lathe("lamp_base", [(0.0, DESK_TOP), (0.074, DESK_TOP), (0.077, DESK_TOP + 0.003), (0.077, DESK_TOP + 0.016), (0.073, DESK_TOP + 0.021), (0.04, DESK_TOP + 0.027), (0.028, DESK_TOP + 0.031), (0.024, DESK_TOP + 0.048)], (bx, by, 0.0), "metal", segments=64, cap_top=True)
    p0 = Vector((bx, by, DESK_TOP + 0.05))
    p1 = p0 + Vector((0.0, -0.04, 0.38))
    p2 = p1 + Vector((0.02, -0.2, -0.04))
    side = Vector((1, 0, 0))
    for i, (a, b) in enumerate([(p0, p1), (p1, p2)]):
        for s in (-1, 1):
            cylinder(f"lamp_arm{i}{s}", a + side * (s * 0.009), b + side * (s * 0.009), 0.0035, "metal", segments=12)
        # A spring alongside each arm, from near its foot to its middle.
        d = (b - a)
        n = d.normalized()
        across = n.cross(side).normalized()
        start = a + d * 0.08 + across * 0.012
        end = a + d * 0.55 + across * 0.012
        coils, r = 22, 0.0042
        u = side
        v = n.cross(u).normalized()
        points = []
        for k in range(coils * 8 + 1):
            t = k / (coils * 8)
            ang = t * coils * math.tau
            c = start.lerp(end, t)
            points.append(tuple(c + u * (r * math.cos(ang)) + v * (r * math.sin(ang))))
        cable(f"lamp_spring{i}", points, 0.0008, "chrome", resolution=1, round_steps=1)
        cylinder(f"lamp_spring_rod{i}", a + d * 0.03 + across * 0.012, start, 0.0009, "chrome", segments=6)
        cylinder(f"lamp_spring_rod_end{i}", end, a + d * 0.62 + across * 0.004, 0.0009, "chrome", segments=6)
    for i, p in enumerate((p0, p1, p2)):
        cylinder(f"lamp_knuckle{i}", p - side * 0.016, p + side * 0.016, 0.0085, "metal", segments=24, bevel=0.002)
        cylinder(f"lamp_knob{i}", p + side * 0.016, p + side * 0.024, 0.0105, "plastic_dark", segments=24, bevel=0.0015)
    # The shade: a cone rolled at its rim, its neck and the switch on it.
    axis = Vector((0.1, -0.5, -0.86)).normalized()
    neck = p2 + axis * 0.012
    rim = neck + axis * 0.11
    def at(t, r):
        return t, r
    profile = [(0.0, 0.016), (0.02, 0.017), (0.03, 0.024), (0.06, 0.044), (0.09, 0.059), (0.105, 0.0655), (0.109, 0.067), (0.111, 0.0668), (0.1112, 0.0655)]
    bm = bmesh.new()
    rings = []
    u0 = axis.orthogonal().normalized()
    v0 = axis.cross(u0)
    seg = 64
    for t, r in profile:
        rings.append([bm.verts.new(neck + axis * t + (u0 * math.cos(k / seg * math.tau) + v0 * math.sin(k / seg * math.tau)) * r) for k in range(seg)])
    for j in range(len(rings) - 1):
        for k in range(seg):
            k2 = (k + 1) % seg
            bm.faces.new((rings[j][k], rings[j][k2], rings[j + 1][k2], rings[j + 1][k]))
    bm.faces.new(list(reversed(rings[0])))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    shade = mesh_object("lamp_head", bm)
    finish(shade, "metal", smooth=True, angle=60)
    mod = shade.modifiers.new("thick", 'SOLIDIFY')
    mod.thickness = 0.0012
    sphere("lamp_bulb", tuple(neck + axis * 0.045), 0.024, "ceramic", segments=24)
    cylinder("lamp_neck", p2, neck + axis * 0.004, 0.012, "metal", segments=24, bevel=0.002)
    cylinder("lamp_switch", tuple(neck + v0 * 0.019 - axis * 0.002), tuple(neck + v0 * 0.027 - axis * 0.002), 0.004, "plastic_dark", segments=16, bevel=0.001)

def joystick(cx, cy, turn):
    """A flight stick of the time at the desk's right front: a heavy rounded base with its
    buttons and throttle wheel, a rubber boot, the stick's moulded grip with a trigger, a hat
    switch and a thumb button, its cable off to the computer."""
    a = math.radians(turn)
    def at(px, py, pz):
        return Vector((cx + px * math.cos(a) - py * math.sin(a), cy + px * math.sin(a) + py * math.cos(a), DESK_TOP + pz))
    # The base, wider at the back, its top sloping down to the front.
    def section(px):
        return [tuple(at(px, -0.07, 0.0)), tuple(at(px, 0.075, 0.0)), tuple(at(px, 0.075, 0.032)), tuple(at(px, 0.02, 0.04)), tuple(at(px, -0.07, 0.026))]
    base = loft("stick_base", [section(-0.07), section(0.07)], "plastic_dark", cap=True)
    mod = base.modifiers.new("round", 'BEVEL')
    mod.width = 0.008
    mod.segments = 4
    mod.limit_method = 'ANGLE'
    for i, (px, py) in enumerate([(-0.055, -0.055), (0.055, -0.055), (-0.055, 0.06), (0.055, 0.06)]):
        cylinder(f"stick_foot{i}", tuple(at(px, py, -0.0005)), tuple(at(px, py, 0.002)), 0.008, "rubber", segments=12)
    for i, (px, mat) in enumerate([(-0.035, "pen_red"), (0.035, "plastic")]):
        p = at(px, -0.045, 0.029)
        cylinder(f"stick_button{i}", tuple(p), tuple(p + Vector((0, -0.002, 0.006))), 0.0085, mat, segments=24, bevel=0.002)
    wheel = at(0.071, 0.02, 0.02)
    cylinder("stick_throttle", tuple(wheel), tuple(wheel + Vector((math.cos(a), math.sin(a), 0)) * 0.008), 0.017, "rubber", segments=32, bevel=0.002)
    # The boot and the stick.
    foot = at(0.0, 0.025, 0.038)
    lathe("stick_boot", [(0.026, 0.0), (0.027, 0.004), (0.022, 0.01), (0.024, 0.015), (0.018, 0.021), (0.019, 0.026), (0.013, 0.032), (0.011, 0.04)], (foot.x, foot.y, foot.z), "rubber", segments=48)
    tilt = Vector((0.0, -0.18, 1.0)).normalized()
    shaft = foot + Vector((0, 0, 0.038))
    cylinder("stick_shaft", tuple(shaft), tuple(shaft + tilt * 0.03), 0.0085, "chrome", segments=24)
    # The grip: rings of a rounded section, swelling under the palm, finger grooves at its front.
    g0 = shaft + tilt * 0.026
    bm = bmesh.new()
    rings = []
    seg = 40
    across = Vector((math.cos(a), math.sin(a), 0.0))
    ahead = tilt.cross(across).normalized()
    heights = [k / 18 for k in range(19)]
    for t in heights:
        z = t * 0.13
        wide = 0.017 + 0.006 * math.sin(math.pi * min(t / 0.75, 1.0)) - 0.004 * max(t - 0.8, 0) / 0.2
        deep = 0.021 + 0.007 * math.sin(math.pi * min(t / 0.75, 1.0)) - 0.005 * max(t - 0.8, 0) / 0.2
        groove = 0.0025 * max(math.sin(t * math.pi * 8), 0.0) if 0.15 < t < 0.65 else 0.0
        ring = []
        for k in range(seg):
            ang = k / seg * math.tau
            c, sn = math.cos(ang), math.sin(ang)
            front = max(-sn, 0.0)
            r_a = wide
            r_b = deep - groove * front ** 2
            ring.append(bm.verts.new(g0 + tilt * z + across * (c * r_a) + ahead * (sn * r_b)))
        rings.append(ring)
    for j in range(len(rings) - 1):
        for k in range(seg):
            k2 = (k + 1) % seg
            bm.faces.new((rings[j][k], rings[j][k2], rings[j + 1][k2], rings[j + 1][k]))
    bm.faces.new(list(reversed(rings[0])))
    top = rings[-1]
    centre = bm.verts.new(g0 + tilt * 0.134)
    for k in range(seg):
        bm.faces.new((top[k], top[(k + 1) % seg], centre))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    finish(mesh_object("stick_grip", bm), "plastic_dark", smooth=True, angle=70)
    trigger = g0 + tilt * 0.095 - ahead * 0.026
    oriented_box("stick_trigger", tuple(trigger), (0.012, 0.008, 0.016), "pen_red", rot=(math.radians(-10), 0, a), bevel=0.003)
    hat = g0 + tilt * 0.128 + ahead * 0.006
    cylinder("stick_hat", tuple(hat), tuple(hat + tilt * 0.008), 0.0065, "plastic", segments=24, bevel=0.0015)
    thumb = g0 + tilt * 0.122 - ahead * 0.012 + across * 0.012
    cylinder("stick_thumb", tuple(thumb), tuple(thumb + tilt * 0.006), 0.0055, "pen_red", segments=20, bevel=0.0015)
    back = at(0.0, 0.075, 0.012)
    cable("stick_cable", [tuple(back), tuple(back + Vector((0.0, 0.06, -0.008))), (0.45, WALL_Y - 0.04, DESK_TOP + 0.003), (0.32, WALL_Y - 0.02, DESK_TOP + 0.003), (0.1, WALL_Y - 0.02, DESK_TOP + 0.003)], 0.0022)


def battle_plan(cx, cy, turn):
    """A colour print of a map, a battle planned on it in pen (painted by the game), lying on
    the desk, curling a little at its corners."""
    a = math.radians(turn)
    w, d = 0.21, 0.2
    bm = bmesh.new()
    layer = bm.loops.layers.uv.new("UVMap")
    n = 10
    grid = []
    for j in range(n + 1):
        row = []
        for i in range(n + 1):
            u, v = i / n, j / n
            px, py = (u - 0.5) * w, (v - 0.5) * d
            lift = 0.006 * (abs(u - 0.5) * 2) ** 4 + 0.004 * max(v - 0.7, 0) / 0.3 * (abs(u - 0.5) * 2) ** 2
            row.append(bm.verts.new((cx + px * math.cos(a) - py * math.sin(a), cy + px * math.sin(a) + py * math.cos(a), DESK_TOP + 0.0008 + lift)))
        grid.append(row)
    for j in range(n):
        for i in range(n):
            f = bm.faces.new((grid[j][i], grid[j][i + 1], grid[j + 1][i + 1], grid[j + 1][i]))
            for loop, (uu, vv) in zip(f.loops, [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)]):
                loop[layer].uv = (uu / n, vv / n)
    finish(mesh_object("plan", bm), "picture.plan", smooth=True)
    picture_aspect("picture.plan", w / d)


def desk_things(kb):
    """The clutter: a mug of coffee (its steam is the renderer's), the manual, CD cases,
    floppy disks, a pencil, a flight stick, a battle plan."""
    # The mug, its handle, the coffee.
    mx, my = -0.42, 0.02
    lathe("mug", [(0.0, DESK_TOP), (0.038, DESK_TOP), (0.041, DESK_TOP + 0.004), (0.041, DESK_TOP + 0.095), (0.043, DESK_TOP + 0.1), (0.039, DESK_TOP + 0.1), (0.037, DESK_TOP + 0.01)], (mx, my, 0.0), "ceramic_blue", segments=72, cap_bottom=True)
    bm = bmesh.new()
    n = 32
    ring = []
    for k in range(n + 1):
        a = -math.pi / 2 + k / n * math.pi
        c = Vector((mx + 0.041 + 0.025 * math.cos(a), my, DESK_TOP + 0.05 + 0.03 * math.sin(a)))
        ring.append([bm.verts.new(c + Vector((0.0062 * math.cos(b) * math.cos(a), 0.0085 * math.sin(b), 0.0062 * math.cos(b) * math.sin(a)))) for b in [j / 16 * math.tau for j in range(16)]])
    for k in range(n):
        for j in range(16):
            j2 = (j + 1) % 16
            bm.faces.new((ring[k][j], ring[k][j2], ring[k + 1][j2], ring[k + 1][j]))
    finish(mesh_object("mug_handle", bm), "ceramic_blue", smooth=True)
    disc_bm = bmesh.new()
    bmesh.ops.create_circle(disc_bm, cap_ends=True, segments=32, radius=0.037)
    bmesh.ops.translate(disc_bm, vec=Vector((mx, my, DESK_TOP + 0.075)), verts=disc_bm.verts)
    finish(mesh_object("coffee", disc_bm), "coffee", smooth=True)
    anchor("steam", (mx, my, DESK_TOP + 0.08))
    # The manual, lying at an angle by the keyboard (its cover painted by the game), a few
    # pages' edge showing.
    mxm, mym, ang = -0.4, -0.17, math.radians(-14)
    w, d = 0.15, 0.2
    def rot(px, py):
        return (mxm + px * math.cos(ang) - py * math.sin(ang), mym + px * math.sin(ang) + py * math.cos(ang))
    corners = [rot(-w / 2, -d / 2), rot(w / 2, -d / 2), rot(w / 2, d / 2), rot(-w / 2, d / 2)]
    quad("manual_cover", [(x, y, DESK_TOP + 0.0078) for x, y in corners], "picture.manual")
    oriented_box("manual_pages", (mxm, mym, DESK_TOP + 0.0035), (w - 0.002, d - 0.002, 0.007), "paper", rot=(0, 0, ang), bevel=0.0008)
    # CD cases stacked by the monitor, the top one's insert painted by the game.
    for i in range(3):
        x, y = 0.16 + i * 0.002, 0.13 + i * 0.003
        a = math.radians(8 + i * 11)
        oriented_box(f"cd_case{i}", (x, y, DESK_TOP + 0.005 + i * 0.0102), (0.142, 0.125, 0.0098), "plastic_dark", rot=(0, 0, a), bevel=0.0008)
    a = math.radians(30)
    cc = [(-0.0705, -0.062), (0.0705, -0.062), (0.0705, 0.062), (-0.0705, 0.062)]
    quad("cd_insert", [(0.164 + px * math.cos(a) - py * math.sin(a), 0.136 + px * math.sin(a) + py * math.cos(a), DESK_TOP + 0.0312) for px, py in cc], "picture.cd")
    # Floppy disks fanned out in front of the computer.
    for i, mat in enumerate(["floppy_black", "floppy_blue", "floppy_red"]):
        a = math.radians(-20 + i * 16)
        x, y = -0.62 + i * 0.03, -0.04 - i * 0.012
        oriented_box(f"floppy{i}", (x, y, DESK_TOP + 0.002 + i * 0.0034), (0.09, 0.094, 0.0033), mat, rot=(0, 0, a), bevel=0.0006)
        oriented_box(f"floppy_shutter{i}", (x, y + 0.031, DESK_TOP + 0.0042 + i * 0.0034), (0.048, 0.03, 0.0004), "alu", rot=(0, 0, a), bevel=0.0)
        oriented_box(f"floppy_label{i}", (x, y - 0.02, DESK_TOP + 0.0041 + i * 0.0034), (0.07, 0.045, 0.0003), "paper", rot=(0, 0, a), bevel=0.0)
    pen_cup(-0.47, 0.3)
    joystick(0.585, 0.0, -14)
    battle_plan(-0.565, 0.0, 6)
    # A pencil.
    a = math.radians(24)
    p0 = Vector((-0.18, -0.21, DESK_TOP + 0.0035))
    d = Vector((math.cos(a), math.sin(a), 0))
    cylinder("pencil", p0, p0 + d * 0.15, 0.0035, "pencil", segments=6)
    cylinder("pencil_tip", p0 + d * 0.15, p0 + d * 0.168, 0.0035, "wood", segments=6, radius2=0.0006)


def wall_shelf():
    """A shelf on the wall over the desk's right end, the lava lamp under it: the game's big
    boxes standing on it face out, leaning on the wall (painted by the game, their spines too),
    a stack of CD cases."""
    x0, x1 = 0.42, 1.14
    z = 1.3
    y1 = WALL_Y
    y0 = y1 - 0.2
    box("wall_shelf", (x0, y0, z - 0.022), (x1, y1 - 0.001, z), "laminate", bevel=0.003)
    # Its brackets: a plate on the wall and a strut under the shelf.
    for i, x in enumerate((x0 + 0.07, x1 - 0.07)):
        box(f"bracket_plate{i}", (x - 0.012, y1 - 0.006, z - 0.17), (x + 0.012, y1 - 0.001, z - 0.022), "metal", bevel=0.001)
        box(f"bracket_arm{i}", (x - 0.006, y0 + 0.02, z - 0.034), (x + 0.006, y1 - 0.006, z - 0.022), "metal", bevel=0.001)
        a, b = Vector((x, y1 - 0.008, z - 0.15)), Vector((x, y0 + 0.04, z - 0.03))
        cylinder(f"bracket_strut{i}", a, b, 0.004, "metal", segments=6)
    bw, bh, bd = 0.205, 0.27, 0.06
    lean = math.radians(-7)
    # Rotation about x by `lean`: the top tips towards the wall.
    def rot(p):
        return Vector((p[0], p[1] * math.cos(lean) - p[2] * math.sin(lean), p[1] * math.sin(lean) + p[2] * math.cos(lean)))
    for i in range(3):
        x = x0 + 0.025 + i * 0.226
        # Its middle, so that its top back edge rests on the wall and its bottom back edge (the
        # lowest, leaning back) on the shelf.
        centre = Vector((x + bw / 2, 0.0, 0.0))
        top_back = rot((0.0, bd / 2, bh / 2))
        bottom_back = rot((0.0, bd / 2, -bh / 2))
        centre.y = y1 - 0.002 - top_back.y
        centre.z = z - bottom_back.z
        oriented_box(f"bigbox{i}", tuple(centre), (bw, bd, bh), "cardboard", rot=(lean, 0, 0), bevel=0.0015)
        # The front, a hair before the box's face, and its left side (the spine the viewer sees).
        def at(px, py, pz, centre=centre):
            return tuple(centre + rot((px, py, pz)))
        f = -bd / 2 - 0.0008
        quad(f"bigbox_front{i}", [at(-bw / 2, f, -bh / 2), at(bw / 2, f, -bh / 2), at(bw / 2, f, bh / 2), at(-bw / 2, f, bh / 2)], f"picture.box{i}")
        s = -bw / 2 - 0.0008
        quad(f"bigbox_spine{i}", [at(s, bd / 2, -bh / 2), at(s, -bd / 2, -bh / 2), at(s, -bd / 2, bh / 2), at(s, bd / 2, bh / 2)], f"picture.spine{i}")
    # A few CD cases lying in a pile at the end.
    for i in range(4):
        a = math.radians(rng.uniform(-8, 8))
        oriented_box(f"shelf_cd{i}", (x1 - 0.045, (y0 + y1) / 2 + rng.uniform(-0.01, 0.01), z + 0.005 + i * 0.0102), (0.142, 0.125, 0.0098), "plastic_dark", rot=(0, 0, math.pi / 2 + a), bevel=0.0008)


def pen_cup(cx, cy):
    """A cup of pens and pencils, a ruler among them."""
    lathe("pen_cup", [(0.0, DESK_TOP), (0.033, DESK_TOP), (0.035, DESK_TOP + 0.004), (0.036, DESK_TOP + 0.1), (0.033, DESK_TOP + 0.1), (0.032, DESK_TOP + 0.006)], (cx, cy, 0.0), "plastic_dark", segments=32, cap_bottom=True)
    for i, (mat, ax, ay, length) in enumerate([("pen_blue", 0.2, 0.1, 0.145), ("pen_black", -0.15, 0.18, 0.14), ("pencil", 0.05, -0.22, 0.17), ("pen_red", -0.22, -0.08, 0.14), ("pencil", 0.24, -0.12, 0.16)]):
        base = Vector((cx + rng.uniform(-0.012, 0.012), cy + rng.uniform(-0.012, 0.012), DESK_TOP + 0.01))
        tip = base + Vector((ax, ay, 1.0)).normalized() * length
        cylinder(f"pen{i}", base, tip, 0.0042 if mat == "pencil" else 0.005, mat, segments=6 if mat == "pencil" else 10)
        if mat == "pencil":
            cylinder(f"pen{i}_tip", tip, tip + (tip - base).normalized() * 0.016, 0.0042, "wood", segments=6, radius2=0.0007)
        else:
            cylinder(f"pen{i}_cap", tip - (tip - base).normalized() * 0.03, tip, 0.0055, "pen_black" if mat != "pen_black" else "alu", segments=10)
    ruler_base = Vector((cx - 0.005, cy + 0.01, DESK_TOP + 0.01))
    ruler_dir = Vector((-0.12, 0.08, 1.0)).normalized()
    oriented_box("ruler", tuple(ruler_base + ruler_dir * 0.1), (0.03, 0.002, 0.2), "paper", rot=(math.atan2(0.08, 1.0), -math.atan2(-0.12, 1.0), 0.3), bevel=0.0005)


def bookshelf():
    """A tall bookshelf in the corner to the right: books, other games' boxes spine out,
    magazines."""
    x0, x1 = 1.24, 2.04
    y0, y1 = 0.3, WALL_Y - 0.005
    h = 1.95
    t = 0.018
    box("shelf_left", (x0, y0, 0), (x0 + t, y1, h), "laminate", bevel=0.002)
    box("shelf_right", (x1 - t, y0, 0), (x1, y1, h), "laminate", bevel=0.002)
    box("shelf_back", (x0, y1 - 0.006, 0), (x1, y1, h), "laminate", bevel=0.0)
    levels = [0.05, 0.42, 0.79, 1.16, 1.53, h - t]
    for i, z in enumerate(levels):
        box(f"shelf{i}", (x0 + t, y0, z), (x1 - t, y1, z + t), "laminate", bevel=0.002)
    # Shelf above: other games' boxes spine out, and books.
    z = levels[3] + t
    x = x0 + t + 0.01
    colours = ["box_purple", "box_teal", "book_black", "box_orange", "book_red", "box_purple", "book_blue", "box_teal"]
    for i, mat in enumerate(colours):
        w = rng.uniform(0.035, 0.065)
        hh = rng.uniform(0.24, 0.3)
        box(f"spine{i}", (x, y0 + 0.05, z), (x + w, y1 - 0.02, z + hh), mat, bevel=0.002)
        x += w + 0.003
    # Books on the other shelves, of all sizes.
    for lvl, start in [(4, 0.0), (2, 0.0), (1, 0.0), (0, 0.27)]:
        z = levels[lvl] + t
        x = x0 + t + 0.005 + start
        while x < x1 - t - 0.08:
            w = rng.uniform(0.018, 0.045)
            hh = rng.uniform(0.19, 0.3)
            mat = rng.choice(["book_red", "book_green", "book_blue", "book_black", "book_cream"])
            box(f"book{lvl}_{x:.3f}", (x, y0 + 0.07, z), (x + w, y1 - 0.02, z + hh), mat, bevel=0.0015)
            x += w + rng.uniform(0.0, 0.004)
    # A CD tower on the bottom right.
    # (Magazines stacked flat on the lowest shelf's left.)
    for i in range(6):
        box(f"magazine{i}", (x0 + 0.03, y0 + 0.06, levels[0] + t + i * 0.006), (x0 + 0.25, y0 + 0.3, levels[0] + t + i * 0.006 + 0.005), rng.choice(["paper", "book_cream", "box_teal"]), bevel=0.0)


def wall_art(cx, cz):
    """The posters on the wall behind the desk, held by tape, a degree or two off true: a band's
    over the calendar by the window, the game's key art in the moonlight, a film's and a
    sunset's over the monitor, the campaign's jungle beside them (all painted by the game: the
    game's own from the install's art, the others drawn there), and the field guide under the
    shelf in the lava lamp's glow; the calendar by the window."""
    y = WALL_Y - 0.003

    def poster(name, x0, x1, z0, z1, curl=0.012, tape=True, tilt=0.0):
        # Slightly lifted at the corners where the tape doesn't hold; hung `tilt` degrees off true.
        bm = bmesh.new()
        n = 8
        layer = bm.loops.layers.uv.new("UVMap")
        mx, mz = (x0 + x1) / 2, (z0 + z1) / 2
        a = math.radians(tilt)

        def turn(px, pz):
            dx, dz = px - mx, pz - mz
            return mx + dx * math.cos(a) - dz * math.sin(a), mz + dx * math.sin(a) + dz * math.cos(a)

        grid = []
        for j in range(n + 1):
            row = []
            for i in range(n + 1):
                u, v = i / n, j / n
                lift = curl * (abs(u - 0.5) * 2) ** 3 * (abs(v - 0.5) * 2) ** 3
                px, pz = turn(x0 + (x1 - x0) * u, z0 + (z1 - z0) * v)
                row.append(bm.verts.new((px, y - lift, pz)))
            grid.append(row)
        for j in range(n):
            for i in range(n):
                f = bm.faces.new((grid[j][i], grid[j][i + 1], grid[j + 1][i + 1], grid[j + 1][i]))
                for loop, (uu, vv) in zip(f.loops, [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)]):
                    loop[layer].uv = (uu / n, vv / n)
        finish(mesh_object(name, bm), f"picture.{name}", smooth=True)
        picture_aspect(f"picture.{name}", (x1 - x0) / (z1 - z0))
        if not tape:
            return
        for (tx, tz) in [(x0, z1), (x1, z1), (x0, z0), (x1, z0)]:
            c = [turn(tx + dx, tz + dz) for dx, dz in [(-0.02, -0.012), (0.02, -0.012), (0.02, 0.012), (-0.02, 0.012)]]
            quad(f"{name}_tape{tx:.2f}{tz:.2f}", [(px, y - 0.0012, pz) for px, pz in c], "tape", uv=False)

    # (What the seat sees of the wall ends a little under 1.65 m: each poster's lettering is
    # low on it.)
    poster("poster_band", -1.15, -0.905, 1.4, 1.74, tilt=-2.0)
    poster("poster_main", -0.84, -0.5, 1.16, 1.67)
    poster("poster_space", -0.455, -0.185, 1.27, 1.67, tilt=1.2)
    poster("poster_outrun", -0.135, 0.145, 1.37, 1.59, tilt=-1.0)
    poster("poster_jungle", 0.2, 0.38, 1.3, 1.6, tilt=0.8)
    poster("poster_guide", 0.8, 1.14, 0.896, 1.12)
    # The calendar, hanging from a pin by the window.
    poster("calendar", -1.12, -0.88, 0.98, 1.35, curl=0.006, tape=False)
    sphere("calendar_pin", (-1.0, y - 0.004, 1.362), 0.004, "pen_red", segments=8)


# ---------------------------------------------------------------------------------------------
# The seasons: what is put up for Christmas and for New Year
# ---------------------------------------------------------------------------------------------

def tube_along(name, path, radii, mat, segments=14, flat=1.0, twist=0.0, cap=True, normal0=None):
    """A surface swept along `path` (a list of points), its section a circle (or an ellipse
    squashed to `flat` of its width, turned `twist` radians a metre) of the radius `radii` gives at
    each point: stockings, hats, garlands, ribbons. (Frames carried along the path, so a bend
    does not twist it.)"""
    pts = [Vector(p) for p in path]
    n = len(pts)
    frames = []
    normal = Vector(normal0).normalized() if normal0 is not None else None
    for i in range(n):
        a = pts[max(i - 1, 0)]
        b = pts[min(i + 1, n - 1)]
        tangent = (b - a).normalized()
        if normal is None:
            normal = tangent.orthogonal().normalized()
        else:
            normal = (normal - tangent * normal.dot(tangent)).normalized()
        if normal.length < 1e-6:
            normal = tangent.orthogonal().normalized()
        frames.append((tangent, normal, tangent.cross(normal)))
    sections = []
    run = 0.0
    for i, p in enumerate(pts):
        if i:
            run += (p - pts[i - 1]).length
        tangent, nx, ny = frames[i]
        turn = twist * run
        ct, st = math.cos(turn), math.sin(turn)
        r = radii[i] if isinstance(radii, (list, tuple)) else radii
        ring = []
        for k in range(segments):
            ang = k / segments * math.tau
            u, v = math.cos(ang) * r, math.sin(ang) * r * flat
            ring.append(tuple(p + nx * (u * ct - v * st) + ny * (u * st + v * ct)))
        sections.append(ring)
    return loft(name, sections, mat, cap=cap)


def swag(p0, p1, sag, n=24):
    """A string hung between two points, sagging `sag` at its middle."""
    a, b = Vector(p0), Vector(p1)
    out = []
    for i in range(n + 1):
        t = i / n
        p = a.lerp(b, t)
        p.z -= sag * 4 * t * (1 - t)
        out.append(p)
    return out


def along(points, every, start=0.0):
    """Points `every` metres apart along a polyline."""
    out = []
    pts = [Vector(p) for p in points]
    need = start
    for a, b in zip(pts, pts[1:]):
        length = (b - a).length
        while need <= length:
            out.append(a.lerp(b, need / length))
            need += every
        need -= length
    return out


def fairy_lights(name, points, every=0.055, mat="emit.fairy"):
    """A string of small warm bulbs hung from a wire along `points`."""
    cable(f"{name}_wire", points, 0.0008, "wire_green", resolution=2, round_steps=1)
    for i, p in enumerate(along(points, every, every / 2)):
        cylinder(f"{name}_cap{i}", (p.x, p.y, p.z - 0.001), (p.x, p.y, p.z - 0.006), 0.0024, "wire_green", segments=8)
        sphere(f"{name}_bulb{i}", (p.x, p.y, p.z - 0.0105), 0.0048, mat, segments=10, scale=(1, 1, 1.25))


def fir_tree(cx, cy, z0, height=0.27, radius=0.08, tiers=7, seed=5, name="tree"):
    """A little fir in a red pot: tiers of boughs, each a cone with a scalloped, drooping
    skirt, a little smaller than the one under it; strung with coloured lights and hung with
    baubles, a star on top, a silver tinsel winding round it."""
    rnd = random.Random(seed)
    pot = 0.045
    lathe(f"{name}_pot", [(0.0, z0), (0.034, z0), (0.039, z0 + 0.006), (0.046, z0 + pot - 0.004), (0.049, z0 + pot), (0.0445, z0 + pot - 0.002)], (cx, cy, 0.0), "pot_red", segments=32, cap_bottom=True)
    cylinder(f"{name}_trunk", (cx, cy, z0 + pot - 0.01), (cx, cy, z0 + pot + height * 0.4), 0.0075, "wood", segments=10)
    base = z0 + pot + 0.012
    t_h = height * 0.27
    boughs = []
    for k in range(tiers):
        f = k / tiers
        r = radius * (1.0 - 0.84 * f ** 0.9)
        z_lo = base + height * 0.78 * f
        petals = max(7, int(14 - 7 * f))
        mid = []
        skirt = []
        tips = []
        for j in range(2 * petals):
            a = j / (2 * petals) * math.tau + k * 0.37
            tip = j % 2 == 0
            rr = r * ((0.94 + 0.12 * rnd.random()) if tip else 0.66)
            mid.append((cx + math.cos(a) * r * 0.5, cy + math.sin(a) * r * 0.5, z_lo + t_h * 0.55))
            droop = 0.011 * (1 - f * 0.5)
            skirt.append((cx + math.cos(a) * rr, cy + math.sin(a) * rr, z_lo - (droop if tip else -0.003)))
            if tip:
                tips.append((a, rr, z_lo - droop))
        apex = [(cx + math.cos(j / (2 * petals) * math.tau) * 0.0008, cy + math.sin(j / (2 * petals) * math.tau) * 0.0008, z_lo + t_h) for j in range(2 * petals)]
        loft(f"{name}_tier{k}", [apex, mid, skirt], "fabric_fir", cap=True, cap_first=False)
        boughs.append((z_lo, r, tips))
    top = boughs[-1][0] + t_h

    def envelope(z):
        """How far out the boughs reach at height z."""
        reach = 0.0
        for z_lo, r, _ in boughs:
            if z >= z_lo - 0.011:
                reach = max(reach, r * max(0.0, 1.0 - (z - z_lo) / t_h))
        return reach

    cols = ["emit.xmas_red", "emit.xmas_green", "emit.xmas_gold", "emit.xmas_blue"]
    n_lights = 0
    for k, (z_lo, r, tips) in enumerate(boughs):
        # Lights on the tips of the boughs, a few to a tier; baubles hanging under others.
        for j, (a, rr, zz) in enumerate(tips):
            if (j + k) % 2 == 0:
                sphere(f"{name}_light{n_lights}", (cx + math.cos(a) * rr * 0.9, cy + math.sin(a) * rr * 0.9, zz + 0.005), 0.0043, cols[n_lights % 4], segments=8)
                n_lights += 1
            elif (j * 3 + k) % 5 == 0 and k < tiers - 1:
                mat = ["bauble_red", "bauble_gold", "bauble_blue", "bauble_silver"][(j + k) % 4]
                sphere(f"{name}_bauble{k}_{j}", (cx + math.cos(a) * rr * 0.86, cy + math.sin(a) * rr * 0.86, zz - 0.006), 0.0092 - 0.0014 * k / tiers, mat, segments=14)
    tinsel = []
    z_a, z_b = base + height * 0.06, top - 0.02
    for i in range(90):
        f = i / 89
        z = z_a + (z_b - z_a) * f
        a = f * 4.0 * math.tau
        rr = envelope(z) * 0.9 + 0.003
        tinsel.append((cx + math.cos(a) * rr, cy + math.sin(a) * rr, z))
    tube_along(f"{name}_tinsel", tinsel, 0.0052, "tinsel", segments=6, twist=60.0)
    # The star: five points, gold, lit, on the top tier.
    pts = []
    for i in range(10):
        ang = math.pi / 2 + i * math.pi / 5
        rr = 0.021 if i % 2 == 0 else 0.0088
        pts.append((math.cos(ang) * rr, math.sin(ang) * rr))

    def star_ring(scale, y):
        return [(cx + px * scale, cy + y, top + 0.012 + pz * scale) for px, pz in pts]

    loft(f"{name}_star", [star_ring(0.85, -0.004), star_ring(1.0, 0.0), star_ring(0.85, 0.004)], "emit.star", cap=True)
    return top


def gift(name, cx, cy, z, w, d, h, paper, ribbon, yaw=0.0):
    """A wrapped present: the box, a ribbon round it each way, a bow."""
    oriented_box(f"{name}_box", (cx, cy, z + h / 2), (w, d, h), paper, rot=(0, 0, yaw), bevel=0.0012)
    r = 0.0035
    oriented_box(f"{name}_band_a", (cx, cy, z + h / 2), (w + 0.0016, 2 * r, h + 0.0016), ribbon, rot=(0, 0, yaw), bevel=0.0004)
    oriented_box(f"{name}_band_b", (cx, cy, z + h / 2), (2 * r, d + 0.0016, h + 0.0016), ribbon, rot=(0, 0, yaw), bevel=0.0004)
    top = Vector((cx, cy, z + h + 0.002))
    for s in (-1, 1):
        oriented_box(f"{name}_bow{s}", tuple(top + Vector((s * 0.008 * math.cos(yaw), s * 0.008 * math.sin(yaw), 0.0025))), (0.014, 0.007, 0.006), ribbon, rot=(0, 0.5 * s, yaw), bevel=0.0015)
    sphere(f"{name}_knot", tuple(top + Vector((0, 0, 0.002))), 0.0042, ribbon, segments=8)


def stocking(x, y, z_top, name="stocking"):
    """A red Christmas stocking hung against the wall by a loop, its foot to the left: a flat
    leg swept down and round to a toe, a white fur cuff, a heel and toe in white."""
    leg = [(x, y, z_top - 0.016 - 0.0125 * i) for i in range(8)] + [(x - 0.003, y, z_top - 0.116), (x - 0.012, y, z_top - 0.132), (x - 0.026, y, z_top - 0.141), (x - 0.042, y, z_top - 0.144), (x - 0.057, y, z_top - 0.141), (x - 0.067, y, z_top - 0.134)]
    radii = [0.0225 - 0.0004 * i for i in range(8)] + [0.0215, 0.021, 0.0207, 0.0204, 0.0196, 0.018, 0.015]
    tube_along(f"{name}_sock", leg, radii, "fabric_red", segments=18, flat=0.5, normal0=(1, 0, 0))
    tube_along(f"{name}_cuff", [(x, y, z_top), (x, y, z_top - 0.03)], 0.026, "fabric_white", segments=20, flat=0.58, normal0=(1, 0, 0))
    tube_along(f"{name}_heel", [(x - 0.007, y, z_top - 0.122), (x - 0.014, y, z_top - 0.138)], 0.0185, "fabric_white", segments=14, flat=0.52, normal0=(1, 0, 0))
    tube_along(f"{name}_toe", [(x - 0.052, y, z_top - 0.1425), (x - 0.0685, y, z_top - 0.1335)], 0.0168, "fabric_white", segments=14, flat=0.52, normal0=(1, 0, 0))
    cable(f"{name}_loop", [(x + 0.009, y + 0.003, z_top - 0.01), (x + 0.015, y + 0.006, z_top + 0.016), (x + 0.0, y + 0.007, z_top + 0.024)], 0.0016, "fabric_white")


def santa_hat(cx, cy, z, name="santa"):
    """A Santa hat sitting on something round (its brim's middle at (cx, cy, z)): a red cone
    bending over, a white brim and a pom-pom."""
    body = [(cx, cy, z + 0.012), (cx, cy, z + 0.036), (cx + 0.003, cy, z + 0.062), (cx + 0.014, cy, z + 0.082), (cx + 0.032, cy, z + 0.087), (cx + 0.049, cy, z + 0.076)]
    tube_along(f"{name}_cap", body, [0.0415, 0.039, 0.031, 0.022, 0.014, 0.008], "fabric_red", segments=22)
    lathe(f"{name}_brim", [(0.0345, 0.0), (0.0452, 0.003), (0.0488, 0.011), (0.0458, 0.0195), (0.0388, 0.0192), (0.0352, 0.008)], (cx, cy, z - 0.002), "fabric_white", segments=32)
    sphere(f"{name}_pom", (cx + 0.052, cy, z + 0.068), 0.0125, "fabric_white", segments=14)


def party_hat(cx, cy, z, tilt=(0.0, 0.0), name="partyhat", mats=("hat_gold", "hat_blue")):
    """A cardboard party cone standing at (cx, cy, z), tilted, striped, a gold trim and a pom-pom."""
    height, r0 = 0.125, 0.046
    base = Vector((cx, cy, z))
    axis = Vector((math.sin(tilt[0]), math.sin(tilt[1]), 1.0)).normalized()
    side = axis.orthogonal().normalized()
    other = axis.cross(side)
    segments = 6
    for k in range(segments):
        a0, a1 = k / segments, (k + 1) / segments
        sections = []
        for t in (a0, a1):
            centre = base + axis * (t * height)
            r = r0 * (1 - t)
            sections.append([tuple(centre + (side * math.cos(i / 24 * math.tau) + other * math.sin(i / 24 * math.tau)) * max(r, 0.0006)) for i in range(24)])
        loft(f"{name}_band{k}", sections, mats[k % 2], cap=(k == 0), cap_first=False)
    ring = [tuple(base + axis * (0.002 * s) + (side * math.cos(i / 24 * math.tau) + other * math.sin(i / 24 * math.tau)) * (r0 + 0.0006)) for s in (0, 3) for i in range(24)]
    sections = [[tuple(base + axis * h + (side * math.cos(i / 24 * math.tau) + other * math.sin(i / 24 * math.tau)) * (r0 * (1 - h / height) + 0.0007)) for i in range(24)] for h in (0.0, 0.0065)]
    loft(f"{name}_trim", sections, "foil_gold", cap=False)
    tip = base + axis * (height + 0.004)
    sphere(f"{name}_pom", tuple(tip), 0.0095, "foil_gold", segments=12)


def balloon(cx, cy, cz, mat, tether=None, name="balloon", size=1.0, tilt=0.0):
    """A helium balloon, its knot at the bottom, floating at (cx, cy, cz) (the middle of its
    body), a curling ribbon trailing from the knot to `tether` (or hanging)."""
    profile = [(0.0, -0.15), (0.0035, -0.148), (0.011, -0.134), (0.036, -0.1), (0.07, -0.04), (0.092, 0.03), (0.099, 0.09), (0.088, 0.145), (0.058, 0.18), (0.026, 0.195), (0.0, 0.1985)]
    profile = [(r * size, (h + 0.02) * size) for r, h in profile]
    lathe(name, profile, (cx, cy, cz - 0.05 * size), mat, segments=36, cap_top=True)
    knot = Vector((cx, cy, cz - 0.05 * size - 0.13 * size))
    end = Vector(tether) if tether else knot + Vector((0.0, 0.0, -0.45))
    pts = [knot]
    steps = 14
    for i in range(1, steps + 1):
        t = i / steps
        p = knot.lerp(end, t)
        p.x += math.sin(t * 9.0) * 0.016 * (0.4 + t)
        p.y += math.cos(t * 9.0) * 0.016 * (0.4 + t)
        pts.append(p)
    cable(f"{name}_ribbon", pts, 0.0011, "ribbon_curl", resolution=3, round_steps=1)


def streamer(name, points, width, mat, twists=3.0):
    """A paper streamer along `points`: a thin strip twisting as it goes."""
    length = sum((Vector(b) - Vector(a)).length for a, b in zip(points, points[1:]))
    tube_along(name, points, width / 2, mat, segments=4, flat=0.04, twist=twists * math.tau / max(length, 1e-3), cap=True)


def champagne_flute(cx, cy, z, name="flute"):
    """A flute of champagne: a round foot, a thin stem, a slender bowl; the wine in it golden."""
    lathe(f"{name}_foot", [(0.0, z), (0.027, z), (0.028, z + 0.0016), (0.026, z + 0.0034), (0.009, z + 0.0062), (0.0032, z + 0.009)], (cx, cy, 0.0), "glass_pale", segments=28, cap_bottom=True)
    lathe(f"{name}_stem", [(0.0032, z + 0.009), (0.0027, z + 0.04), (0.0028, z + 0.07), (0.005, z + 0.078)], (cx, cy, 0.0), "glass_pale", segments=12)
    lathe(f"{name}_bowl", [(0.005, z + 0.078), (0.016, z + 0.088), (0.0225, z + 0.115), (0.0235, z + 0.15), (0.0215, z + 0.185), (0.0205, z + 0.1905), (0.0191, z + 0.1905), (0.0199, z + 0.185), (0.0208, z + 0.15), (0.0198, z + 0.115), (0.0148, z + 0.09), (0.0036, z + 0.081)], (cx, cy, 0.0), "glass_pale", segments=28)
    lathe(f"{name}_wine", [(0.0, z + 0.083), (0.0145, z + 0.092), (0.0198, z + 0.117), (0.0207, z + 0.15), (0.0192, z + 0.171), (0.0, z + 0.171)], (cx, cy, 0.0), "champagne", segments=28, cap_top=True)


def cider_bottle(cx, cy, z, name="bottle"):
    """A bottle of sparkling cider: dark green glass, a cream label with a gold band, gold foil
    over the neck."""
    profile = [(0.0, z), (0.031, z), (0.0365, z + 0.004), (0.0375, z + 0.02), (0.0375, z + 0.13), (0.035, z + 0.165), (0.026, z + 0.19), (0.0155, z + 0.215), (0.0125, z + 0.235), (0.0122, z + 0.28), (0.0138, z + 0.286), (0.0138, z + 0.296), (0.0, z + 0.296)]
    lathe(f"{name}_glass", profile, (cx, cy, 0.0), "glass_green", segments=40, cap_bottom=True)
    lathe(f"{name}_label", [(0.0379, z + 0.045), (0.0379, z + 0.115)], (cx, cy, 0.0), "paper", segments=40)
    lathe(f"{name}_band", [(0.038, z + 0.07), (0.038, z + 0.082)], (cx, cy, 0.0), "foil_gold", segments=40)
    lathe(f"{name}_foil", [(0.0128, z + 0.222), (0.0129, z + 0.28), (0.0146, z + 0.285), (0.0146, z + 0.297), (0.0, z + 0.297)], (cx, cy, 0.0), "foil_gold", segments=28, cap_top=True)


def confetti(region, count, seed, z):
    """Confetti on a surface at height `z`: scraps of colour scattered over `region` = (x0, y0, x1, y1)."""
    rnd = random.Random(seed)
    mats = ["confetti_red", "confetti_gold", "confetti_blue", "confetti_green", "confetti_pink"]
    for i in range(count):
        x = rnd.uniform(region[0], region[2])
        y = rnd.uniform(region[1], region[3])
        a = rnd.uniform(0, math.tau)
        w, d = rnd.uniform(0.006, 0.011), rnd.uniform(0.003, 0.006)
        c, s = math.cos(a), math.sin(a)
        quad(f"confetti{i}", [(x + (-w * c + d * s), y + (-w * s - d * c), z + 0.0006 + 0.00002 * (i % 7)), (x + (w * c + d * s), y + (w * s - d * c), z + 0.0006 + 0.00002 * (i % 7)), (x + (w * c - d * s), y + (w * s + d * c), z + 0.0006 + 0.00002 * (i % 7)), (x + (-w * c - d * s), y + (-w * s + d * c), z + 0.0006 + 0.00002 * (i % 7))], rnd.choice(mats), uv=False)


def crt_top_at(y):
    """The monitor's top over (any x of its flat middle) at depth y: level to its seam, then
    sloping down to the back."""
    return 1.2518 - max(y - 0.2507, 0.0) * 0.244


def christmas():
    """Christmas in the room: a little tree on the right speaker, presents on the monitor, fairy
    lights over the posters and along the shelf, a garland, a stocking, a Santa hat on the lava
    lamp."""
    # The tree on the right speaker's top (clear of the monitor and the lamp).
    sx, sy = 0.31, 0.305
    fir_tree(sx, sy, DESK_TOP + 0.2 - 0.001, height=0.27, radius=0.068)
    # Presents on the monitor, one on another.
    gift("gift_a", -0.13, 0.275, crt_top_at(0.275) - 0.004, 0.07, 0.06, 0.05, "gift_red", "ribbon_gold", yaw=0.3)
    gift("gift_b", -0.045, 0.30, crt_top_at(0.30) - 0.004, 0.06, 0.055, 0.042, "gift_green", "ribbon_silver", yaw=-0.2)
    gift("gift_c", -0.13, 0.275, crt_top_at(0.275) - 0.004 + 0.05, 0.05, 0.045, 0.036, "gift_blue", "ribbon_gold", yaw=0.7)
    # Fairy lights: over the posters in swags, and along the shelf's front edge.
    hooks = [-1.16, -0.76, -0.36, 0.04, 0.44]
    wire = []
    for a, bb in zip(hooks, hooks[1:]):
        seg = swag((a, 0.61, 1.7), (bb, 0.61, 1.7), 0.1, 16)
        wire += seg if not wire else seg[1:]
    fairy_lights("posters_lights", wire, every=0.06)
    shelf = []
    for a, bb in [(0.43, 0.79), (0.79, 1.13)]:
        seg = swag((a, 0.4, 1.3), (bb, 0.4, 1.3), 0.03, 14)
        shelf += seg if not shelf else seg[1:]
    fairy_lights("shelf_lights", shelf, every=0.05)
    # A fir garland along the shelf's front, red berries in it.
    garland = []
    for a, bb in [(0.43, 0.79), (0.79, 1.13)]:
        seg = swag((a, 0.405, 1.268), (bb, 0.405, 1.268), 0.04, 14)
        garland += seg if not garland else seg[1:]
    tube_along("garland", garland, 0.0125, "fabric_fir", segments=10, twist=40.0)
    for i, p in enumerate(along(garland, 0.075, 0.03)):
        sphere(f"garland_berry{i}", (p.x, p.y - 0.008, p.z + 0.004), 0.0058, "bauble_red", segments=10)
    stocking(0.452, 0.4, 1.268)
    # A Santa hat on the lava lamp.
    santa_hat(0.66, 0.46, 1.098)
    # Where the season's lights shine from: the tree, the lights over the posters, along the shelf.
    anchor("light0", (sx, sy, DESK_TOP + 0.2 + 0.2))
    anchor("light1", (-0.36, 0.5, 1.62))
    anchor("light2", (0.78, 0.36, 1.26))


def window_snow():
    """Snow outside the window: a ledge under the glass with snow heaped on it, icicles from the
    frame's top, snow on the roofs across the street, and a string of lights round the window's
    top inside. (Outside is a backdrop: unlit, and no shadow.)"""
    y0, y1 = WINDOW_Y
    z0, z1 = WINDOW_Z
    sky_x = LEFT_X - 6.0
    outside = []
    outside.append(box("ledge_out", (LEFT_X - 0.31, y0 - 0.06, z0 - 0.045), (LEFT_X - 0.1, y1 + 0.06, z0 - 0.012), "silhouette_snow_near", bevel=0.004))
    n = 36
    sections = []
    for i in range(n + 1):
        t = i / n
        y = (y0 - 0.05) + (y1 - y0 + 0.1) * t
        taper = math.sin(math.pi * t) ** 0.5
        lump = 0.6 + 0.4 * math.sin(t * 17.0) * math.sin(t * 7.3 + 1.0)
        h = (0.02 + 0.055 * lump) * taper + 0.004
        wx = 0.085 * (0.6 + 0.4 * taper)
        xc = LEFT_X - 0.205
        sections.append([(xc + wx * math.cos(a), y, z0 - 0.012 + h * math.sin(a)) for a in [k / 12 * math.pi for k in range(13)]])
    outside.append(loft("ledge_snow", sections, "silhouette_snow_near", cap=True))
    for i in range(9):
        y = y0 + 0.03 + (y1 - y0 - 0.06) * i / 8
        length = 0.04 + 0.05 * abs(math.sin(i * 2.3 + 0.7))
        outside.append(cylinder(f"icicle{i}", (LEFT_X - 0.115, y, z1 + 0.002), (LEFT_X - 0.115, y, z1 - length), 0.006, "silhouette_snow_near", segments=6, radius2=0.0008))
    base = 0.4
    for i, (yy, w, h, roof) in enumerate(HOUSES):
        pts = [(yy - w / 2 - 0.02, base + h - 0.01), (yy + w / 2 + 0.02, base + h - 0.01), (yy, base + h + roof + 0.02)]
        bm = bmesh.new()
        bm.faces.new([bm.verts.new((sky_x + 1.502, p[0], p[1])) for p in pts])
        outside.append(finish(mesh_object(f"roof_snow{i}", bm), "silhouette_snow"))
    for o in outside:
        o.visible_shadow = False
    # Inside: lights along the top of the window.
    fairy_lights("window_lights", swag((LEFT_X + 0.05, y0 - 0.06, z1 + 0.05), (LEFT_X + 0.05, y1 + 0.06, z1 + 0.05), 0.06, 20), every=0.06)


def pennants(letters, p0, p1, sag, flag_w=0.075, flag_h=0.095, name="banner"):
    """A string of triangular flags hung in a swag, each carrying one cell of `picture.banner`
    (a 4 x 4 picture the game paints: `letters` says which, in reading order)."""
    string = swag(p0, p1, sag, 60)
    cable(f"{name}_string", string, 0.0009, "rubber", resolution=1, round_steps=1)
    # Evenly along the string.
    lengths = [0.0]
    for a, bb in zip(string, string[1:]):
        lengths.append(lengths[-1] + (bb - a).length)
    total = lengths[-1]
    n = len(letters)
    bm = bmesh.new()
    layer = bm.loops.layers.uv.new("UVMap")
    for i in range(n):
        s = total * (0.04 + 0.92 * (i + 0.5) / n)
        k = next(j for j, l in enumerate(lengths) if l >= s)
        at = string[k]
        tangent = (string[min(k + 1, len(string) - 1)] - string[max(k - 1, 0)]).normalized()
        left, right = at - tangent * flag_w / 2, at + tangent * flag_w / 2
        tip = at + Vector((0.0, 0.0, -flag_h)) * 1.0
        verts = [bm.verts.new(left), bm.verts.new(right), bm.verts.new(tip)]
        f = bm.faces.new(verts)
        col, row = i % 4, i // 4
        u0, v0 = col / 4, 1.0 - (row + 1) / 4
        for loop, (u, v) in zip(f.loops, [(u0, v0 + 0.25), (u0 + 0.25, v0 + 0.25), (u0 + 0.125, v0)]):
            loop[layer].uv = (u, v)
    flags = finish(mesh_object(f"{name}_flags", bm), "picture.banner")
    picture_aspect("picture.banner", 1.0)
    return flags


def coil(top, length, radius, turns, mat, name):
    """A curling streamer let down from `top`: a spiral, wider as it falls."""
    pts = []
    for i in range(61):
        t = i / 60
        a = t * turns * math.tau
        r = radius * (0.35 + 0.65 * t)
        pts.append((top[0] + math.cos(a) * r, top[1] + math.sin(a) * r, top[2] - length * t))
    streamer(name, pts, 0.016, mat, twists=3.0)


def newyear():
    """New Year in the room: balloons over the right speaker, party hats (on the lava lamp, the
    monitor and the left speaker), streamers and a banner over the posters, a bottle and two
    flutes with confetti on the right of the desk."""
    # Balloons tied to the speaker top.
    tether = (0.31, 0.305, DESK_TOP + 0.202)
    for i, (x, y, z, mat) in enumerate([(0.335, 0.30, 1.29, "balloon_gold"), (0.405, 0.36, 1.43, "balloon_blue"), (0.285, 0.385, 1.46, "balloon_silver"), (0.36, 0.27, 1.575, "balloon_pink")]):
        balloon(x, y, z, mat, tether=tether, name=f"balloon{i}", size=0.8)
    # Party hats.
    party_hat(0.66, 0.46, 1.1055, name="hat_lamp")
    party_hat(-0.12, 0.27, crt_top_at(0.27) - 0.003, tilt=(0.14, -0.1), name="hat_crt", mats=("hat_pink", "hat_gold"))
    party_hat(-0.31, 0.305, DESK_TOP + 0.2 - 0.001, tilt=(-0.08, 0.05), name="hat_speaker", mats=("hat_blue", "hat_gold"))
    # Streamers in swags over the posters, the banner under them, curls from the shelf.
    hooks = [-1.16, -0.76, -0.36, 0.04, 0.44]
    cols = ["streamer_red", "streamer_gold", "streamer_blue", "streamer_pink", "streamer_green"]
    for i, (a, bb) in enumerate(zip(hooks, hooks[1:])):
        for k in range(2):
            pts = swag((a, 0.608 - 0.004 * k, 1.71), (bb, 0.608 - 0.004 * k, 1.71), 0.09 + 0.045 * k, 20)
            streamer(f"streamer{i}_{k}", pts, 0.016, cols[(i + 2 * k) % 5], twists=4.0)
    pennants("HAPPY*NEW*YEAR*"[:14], (-1.08, 0.6, 1.66), (0.4, 0.6, 1.66), 0.075)
    for i, x in enumerate((0.56, 0.86, 1.06)):
        coil((x, 0.41, 1.288), 0.2 + 0.03 * (i % 2), 0.02, 5.5, cols[(i + 1) % 5], f"coil{i}")
    # The table: a bottle of cider and two flutes, confetti about.
    cider_bottle(0.47, 0.3, DESK_TOP)
    champagne_flute(0.57, 0.215, DESK_TOP)
    champagne_flute(0.635, 0.265, DESK_TOP)
    confetti((0.38, 0.1, 0.78, 0.4), 90, 7, DESK_TOP)
    # Where the season's lights shine from: over the table, over the posters, among the balloons.
    anchor("light0", (0.55, 0.25, DESK_TOP + 0.3))
    anchor("light1", (-0.36, 0.5, 1.62))
    anchor("light2", (0.31, 0.3, 1.35))


def lights():
    """The lights the bakes and the previews use (the renderer has its own, matched)."""
    sun = bpy.data.lights.new("moon", 'SUN')
    sun.energy = MOON_STRENGTH
    sun.color = MOON_COLOR
    sun.angle = math.radians(0.6)
    obj = bpy.data.objects.new("light.moon", sun)
    obj.rotation_euler = MOON_DIR.to_track_quat('Z', 'Y').to_euler()
    bpy.context.collection.objects.link(obj)
    # (Its direction, towards the moon, as the anchor's place.)
    anchor("moon", tuple(MOON_DIR))
    lava = bpy.data.lights.new("lava", 'POINT')
    lava.energy = LAVA_WATTS
    lava.color = LAVA_COLOR
    lava.shadow_soft_size = 0.05
    l = bpy.data.objects.new("light.lava", lava)
    l.location = bpy.data.objects["anchor.lava"].location
    bpy.context.collection.objects.link(l)


def tube_quarters():
    """The tube's glass as four emitting quarters (for the bakes and the previews only)."""
    s = bpy.data.objects["anchor.screen"]
    c, hw, hh = s.location, s.scale.x, s.scale.z
    quarters = []
    for i, (sx, sz) in enumerate([(-1, 1), (1, 1), (-1, -1), (1, -1)]):
        x0, x1 = (c.x, c.x + sx * hw) if sx > 0 else (c.x + sx * hw, c.x)
        z0, z1 = (c.z, c.z + sz * hh) if sz > 0 else (c.z + sz * hh, c.z)
        m = bpy.data.materials.new(f"bake.tube{i}")
        m.use_nodes = True
        b = m.node_tree.nodes.get("Principled BSDF")
        b.inputs["Base Color"].default_value = (0, 0, 0, 1)
        b.inputs["Emission Color"].default_value = (1, 1, 1, 1)
        b.inputs["Emission Strength"].default_value = 0.0
        obj = quad(f"bake.tube{i}", [(x0, c.y - 0.003, z0), (x1, c.y - 0.003, z0), (x1, c.y - 0.003, z1), (x0, c.y - 0.003, z1)], "paper", uv=False)
        obj.data.materials.clear()
        obj.data.materials.append(m)
        quarters.append((obj, b))
    return quarters


# ---------------------------------------------------------------------------------------------
# Baking, export, previews
# ---------------------------------------------------------------------------------------------

def meshes():
    # (By name, so that every run walks the room in the same order.)
    return sorted((o for o in bpy.context.scene.objects if o.type == 'MESH' and not o.name.startswith("bake.")), key=lambda o: o.name)


def apply_modifiers():
    for o in meshes():
        bpy.context.view_layer.objects.active = o
        for m in list(o.modifiers):
            bpy.ops.object.modifier_apply(modifier=m.name)


def merge_static():
    """Everything one object per material (the renderer draws it in one go anyway), and per
    season: what a season puts up stays apart from the rest."""
    groups = {}
    for o in meshes():
        groups.setdefault((o.get("season", ""), o.data.materials[0].name), []).append(o)
    for (season, mat), objs in groups.items():
        stem = f"season.{season}." if season else "static."
        obj = join(objs, f"{stem}{mat}") if len(objs) > 1 else objs[0]
        obj.name = f"{stem}{mat}"


def subdivide_for_bake(max_edge=0.06):
    for o in meshes():
        bm = bmesh.new()
        bm.from_mesh(o.data)
        for _ in range(5):
            long = [e for e in bm.edges if e.calc_length() > max_edge]
            if not long:
                break
            bmesh.ops.subdivide_edges(bm, edges=long, cuts=1, use_grid_fill=True)
        bmesh.ops.triangulate(bm, faces=bm.faces)
        # The vertices in the order of their places: the subdivision made the same points in a
        # different order from run to run, and a kept bake (ROOM_CKPT) is per vertex.
        bm.verts.index_update()
        rank = {v.index: i for i, v in enumerate(sorted(bm.verts, key=lambda v: (v.co.x, v.co.y, v.co.z)))}
        bm.verts.sort(key=lambda v: rank[v.index])
        bm.verts.index_update()
        bm.to_mesh(o.data)
        bm.free()


def bake_pass(target, filter_, kind='DIFFUSE', samples=96, objs=None):
    scene = bpy.context.scene
    scene.cycles.samples = samples
    for o in meshes():
        attrs = o.data.color_attributes
        if target not in attrs:
            attrs.new(name=target, type='FLOAT_COLOR', domain='POINT')
        attrs.active_color = attrs[target]
    scene.render.bake.target = 'VERTEX_COLORS'
    if kind == 'DIFFUSE':
        scene.render.bake.use_pass_direct = 'DIRECT' in filter_
        scene.render.bake.use_pass_indirect = 'INDIRECT' in filter_
        scene.render.bake.use_pass_color = False
    bpy.ops.object.select_all(action='DESELECT')
    objs = objs or meshes()
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.object.bake(type=kind)


def room_signature(samples):
    """What the bakes depend on: every mesh as it stands, with its materials as they are (colour,
    metal, roughness, its own light, see-through), and the lights (how strong, what colour, how
    soft, where from); a kept bake of another room, or of this one before any of that changed, is
    not used."""
    h = hashlib.sha1()
    h.update(repr((samples, BOUNCE_SCALE, LAVA_SCALE, TUBE_SCALE)).encode())
    for o in sorted(meshes(), key=lambda o: o.name):
        n = len(o.data.vertices)
        co = [0.0] * (n * 3)
        o.data.vertices.foreach_get("co", co)
        h.update(f"{o.name}:{n}:{o.get('season', '')}".encode())
        h.update(repr([round(v, 5) for row in o.matrix_world for v in row]).encode())
        h.update(array.array("f", co).tobytes())
        for slot in o.material_slots:
            h.update(material_signature(slot.material).encode())
    for name in ("light.moon", "light.lava"):
        obj = bpy.data.objects[name]
        light = obj.data
        h.update(repr((round(light.energy, 5), [round(c, 5) for c in light.color], round(getattr(light, "angle", 0.0), 5),
                       round(getattr(light, "shadow_soft_size", 0.0), 5), [round(v, 5) for row in obj.matrix_world for v in row])).encode())
    return h.hexdigest()


def material_signature(m):
    """A material's part in a bake: every input of its nodes not driven by another node, rounded
    (and what points at an object or a picture, by name: never where it lies in memory)."""
    if m is None:
        return "none"
    out = [m.name]
    if m.use_nodes and m.node_tree:
        for node in sorted(m.node_tree.nodes, key=lambda node: node.name):
            for socket in node.inputs:
                if socket.is_linked or not hasattr(socket, "default_value"):
                    continue
                v = socket.default_value
                if isinstance(v, float):
                    v = round(v, 5)
                elif isinstance(v, (bool, int, str)):
                    pass
                elif hasattr(v, "name"):
                    v = v.name
                else:
                    try:
                        v = [round(float(x), 5) for x in v]
                    except (TypeError, ValueError):
                        continue
                out.append(f"{node.name}.{socket.identifier}={v}")
    return repr(out)


def keep_bakes(folder, name, objs, signature, attributes):
    """Writes down what a pass baked on a group of objects (`ROOM_CKPT=<dir>`)."""
    os.makedirs(folder, exist_ok=True)
    data = {}
    for o in objs:
        n = len(o.data.vertices)
        arrays = {}
        for a in attributes:
            buf = [0.0] * (n * 4)
            o.data.color_attributes[a].data.foreach_get("color", buf)
            arrays[a] = array.array("f", buf)
        data[o.name] = arrays
    path = os.path.join(folder, f"bake_{name}.pkl")
    with open(path + ".tmp", "wb") as f:
        pickle.dump({"signature": signature, "objects": data}, f)
    os.replace(path + ".tmp", path)


def kept_bakes(folder, name, objs, signature):
    """Reads a pass's bakes back onto the objects; False if there are none for this room."""
    path = os.path.join(folder, f"bake_{name}.pkl")
    if not os.path.exists(path):
        return False
    with open(path, "rb") as f:
        saved = pickle.load(f)
    if saved["signature"] != signature or set(saved["objects"]) != {o.name for o in objs}:
        print("kept bakes", name, "are of another room: baking again")
        return False
    for o in objs:
        attrs = o.data.color_attributes
        for a, values in saved["objects"][o.name].items():
            if a not in attrs:
                attrs.new(name=a, type='FLOAT_COLOR', domain='POINT')
            attrs[a].data.foreach_set("color", list(values))
    print("kept bakes", name, "loaded")
    return True


def bake(samples):
    """The vertex colours: occlusion, the bounce of the moon, the sky and the lava lamp, and
    each quarter of the tube's (see the module's notes). `ROOM_CKPT=<dir>` keeps each group's
    bakes as it finishes and uses them again, so a run that died late can go on."""
    scene = bpy.context.scene
    scene.render.engine = 'CYCLES'
    try:
        prefs = bpy.context.preferences.addons["cycles"].preferences
        prefs.compute_device_type = 'OPTIX'
        prefs.get_devices()
        # The GPUs alone: with the CPU beside them, Blender 5.2's bake to vertex colours
        # crashed in the CPU's kernel on some meshes (the can's finer profile).
        gpus = [d for d in prefs.devices if d.type != 'CPU']
        for d in prefs.devices:
            d.use = d.type != 'CPU' or not gpus
        scene.cycles.device = 'GPU'
    except Exception as e:
        print("cycles on the CPU:", e)
    scene.world.light_settings.distance = 0.25
    quarters = tube_quarters()
    moon = bpy.data.objects["light.moon"].data
    lava = bpy.data.objects["light.lava"].data
    energies = {"moon": moon.energy, "lava": lava.energy}
    # Every glowing material (the lava, the LEDs, the sky...) and its strength.
    glows = []
    for m in bpy.data.materials:
        if m.use_nodes and not m.name.startswith("bake."):
            b = m.node_tree.nodes.get("Principled BSDF")
            if b and b.inputs["Emission Strength"].default_value > 0:
                glows.append((m.name, b, b.inputs["Emission Strength"].default_value))

    def only(lights=(), glowing=()):
        """Everything dark but these lights and glowing materials."""
        moon.energy = energies["moon"] if "moon" in lights else 0.0
        lava.energy = energies["lava"] if "lava" in lights else 0.0
        for name, b, strength in glows:
            b.inputs["Emission Strength"].default_value = strength if name in glowing else 0.0
        for _, emit in quarters:
            emit.inputs["Emission Strength"].default_value = 0.0

    folder = os.environ.get("ROOM_CKPT")
    signature = room_signature(samples) if folder else ""

    def passes(objs, group):
        """The bakes of `objs` (what else is in the room stands as it is)."""
        def step(attribute, run):
            key = f"{group}_{attribute}"
            if folder and kept_bakes(folder, key, objs, signature):
                return
            run()
            if folder:
                keep_bakes(folder, key, objs, signature, [attribute])

        # Occlusion.
        step("ao", lambda: bake_pass("ao", (), kind='AO', samples=samples, objs=objs))
        # The moonlight's bounce (the renderer draws its direct light).
        only(lights=("moon",))
        step("bounce", lambda: bake_pass("bounce", ('INDIRECT',), samples=samples, objs=objs))
        # The lava lamp's light, direct and bounced: its bulb and its glowing glass.
        only(lights=("lava",), glowing=("lava",))
        step("lava", lambda: bake_pass("lava", ('DIRECT', 'INDIRECT'), samples=samples, objs=objs))
        # Each quarter of the tube alone, white, bounced.
        only()
        for i, (_, emit) in enumerate(quarters):
            emit.inputs["Emission Strength"].default_value = 1.0
            step(f"tube{i}", lambda i=i: bake_pass(f"tube{i}", ('INDIRECT',), samples=samples, objs=objs))
            emit.inputs["Emission Strength"].default_value = 0.0

    # The room as it is the rest of the year, without any season's things; then each season's
    # own things, standing in that room, alone with it.
    by_season = {name: [o for o in meshes() if o.get("season") == name] for name in SEASONS}
    for objs in by_season.values():
        for o in objs:
            o.hide_render = True
    passes([o for o in meshes() if not o.get("season")], "base")
    for name, objs in by_season.items():
        if not objs:
            continue
        for other, others in by_season.items():
            for o in others:
                o.hide_render = other != name
        passes(objs, name)
    for objs in by_season.values():
        for o in objs:
            o.hide_render = False
    moon.energy, lava.energy = energies["moon"], energies["lava"]
    for _, b, strength in glows:
        b.inputs["Emission Strength"].default_value = strength
    for obj, _ in quarters:
        bpy.data.objects.remove(obj)
    # Packed for the renderer (see the module's notes); the exporter writes the attributes
    # named with an underscore as they are, and `export` names them COLOR_0..2. The bounced
    # light is smoothed over each vertex's neighbours first: it is smooth by nature, and what
    # varies from one vertex to the next is the bake's noise.
    for o in meshes():
        attrs = o.data.color_attributes
        n = len(o.data.vertices)
        def read(name):
            data = [0.0] * (n * 4)
            attrs[name].data.foreach_get("color", data)
            return data
        neighbours = vertex_neighbours(o.data)
        ao = read("ao")
        bounce, lava_light = smoothed(read("bounce"), neighbours), smoothed(read("lava"), neighbours)
        quarters_data = [smoothed(read(f"tube{i}"), neighbours) for i in range(4)]
        out0 = [0.0] * (n * 4)
        out1 = [0.0] * (n * 4)
        out2 = [0.0] * (n * 4)
        for v in range(n):
            for k in range(3):
                out0[v * 4 + k] = min(bounce[v * 4 + k] / BOUNCE_SCALE, 1.0) ** 0.5
                out2[v * 4 + k] = min(lava_light[v * 4 + k] / LAVA_SCALE, 1.0) ** 0.5
            out0[v * 4 + 3] = ao[v * 4]
            out2[v * 4 + 3] = 1.0
            for i in range(4):
                q = quarters_data[i]
                lum = 0.2126 * q[v * 4] + 0.7152 * q[v * 4 + 1] + 0.0722 * q[v * 4 + 2]
                out1[v * 4 + i] = min(lum / TUBE_SCALE, 1.0) ** 0.5
        for name in ["ao", "bounce", "lava", "tube0", "tube1", "tube2", "tube3"]:
            attrs.remove(attrs[name])
        for name, data in [("_light", out0), ("_tube", out1), ("_lava", out2)]:
            attrs.new(name=name, type='FLOAT_COLOR', domain='POINT').data.foreach_set("color", data)


# The baked sets as the exporter writes them (custom attributes), and as the file gives them.
BAKED = {"_LIGHT": "COLOR_0", "_TUBE": "COLOR_1", "_LAVA": "COLOR_2"}


def rename_attributes(path):
    """Names the baked sets as colours in a GLB. (Blender's exporter writes several colour
    sets wrongly, all but the last white; written as custom attributes, which an optimiser
    drops, they come out right.)"""
    data = open(path, "rb").read()
    length = struct.unpack("<I", data[12:16])[0]
    doc = json.loads(data[20:20 + length])
    for mesh in doc.get("meshes", []):
        for prim in mesh["primitives"]:
            attrs = prim["attributes"]
            for old, new in BAKED.items():
                if old in attrs:
                    attrs[new] = attrs.pop(old)
    text = json.dumps(doc, separators=(",", ":")).encode()
    text += b" " * (-len(text) % 4)
    rest = data[20 + length:]
    out = b"glTF" + struct.pack("<II", 2, 20 + len(text) + len(rest)) + struct.pack("<I", len(text)) + b"JSON" + text + rest
    open(path, "wb").write(out)


def vertex_neighbours(mesh):
    """Each vertex's neighbours along the edges that do not turn a corner (the light on the
    other side of a crease is another's)."""
    out = [[] for _ in mesh.vertices]
    normals = [v.normal.copy() for v in mesh.vertices]
    for e in mesh.edges:
        a, b = e.vertices
        if normals[a].dot(normals[b]) > 0.9:
            out[a].append(b)
            out[b].append(a)
    return out


def smoothed(data, neighbours, passes=3):
    """RGBA per vertex, each pass half its own and half its neighbours' mean."""
    for _ in range(passes):
        out = list(data)
        for v, near in enumerate(neighbours):
            if not near:
                continue
            k = 0.5 / len(near)
            for c in range(3):
                out[v * 4 + c] = data[v * 4 + c] * 0.5 + sum(data[u * 4 + c] for u in near) * k
        data = out
    return data


def quantize(path):
    """Packs a GLB as `gltfpack -vn 12 -vc 12` would where it is not to hand: normals as
    16-bit integers, the baked colours as 16-bit fractions; positions, UVs and indices as they
    are. Every accessor gets a view of its own, 4-byte aligned."""
    data = open(path, "rb").read()
    json_length = struct.unpack("<I", data[12:16])[0]
    doc = json.loads(data[20:20 + json_length])
    bin_start = 20 + json_length + 8
    bin_length = struct.unpack("<I", data[20 + json_length:24 + json_length])[0]
    old = data[bin_start:bin_start + bin_length]
    roles = {}
    for mesh in doc["meshes"]:
        for prim in mesh["primitives"]:
            for name, acc in prim["attributes"].items():
                roles[acc] = name
    sizes = {5120: 1, 5121: 1, 5122: 2, 5123: 2, 5125: 4, 5126: 4}
    widths = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}
    out = bytearray()
    views = []
    for i, a in enumerate(doc["accessors"]):
        view = doc["bufferViews"][a["bufferView"]]
        width = widths[a["type"]]
        size = sizes[a["componentType"]]
        stride = view.get("byteStride", width * size)
        start = view.get("byteOffset", 0) + a.get("byteOffset", 0)
        count = a["count"]
        role = roles.get(i, "")
        if a["componentType"] == 5126 and (role == "NORMAL" or role.startswith("COLOR_")):
            fmt = "<" + "f" * width
            values = [struct.unpack_from(fmt, old, start + k * stride) for k in range(count)]
            if role == "NORMAL":
                body = bytearray()
                for v in values:
                    body += struct.pack("<hhhh", *[max(-32767, min(32767, round(c * 32767))) for c in v], 0)
                a.update(componentType=5122, normalized=True)
                new_stride = 8
            else:
                body = bytearray()
                for v in values:
                    body += struct.pack("<" + "H" * width, *[max(0, min(65535, round(c * 65535))) for c in v])
                a.update(componentType=5123, normalized=True)
                new_stride = width * 2
            a.pop("byteOffset", None)
        else:
            new_stride = width * size
            if stride == new_stride:
                body = old[start:start + count * stride]
            else:
                body = b"".join(old[start + k * stride:start + k * stride + new_stride] for k in range(count))
            a.pop("byteOffset", None)
        while len(out) % 4:
            out.append(0)
        v = {"buffer": 0, "byteOffset": len(out), "byteLength": len(body)}
        if role and new_stride % 4 == 0 and new_stride != width * sizes[a["componentType"]]:
            v["byteStride"] = new_stride
        elif role == "NORMAL":
            v["byteStride"] = 8
        if "target" in view:
            v["target"] = view["target"]
        a["bufferView"] = len(views)
        views.append(v)
        out += body
    while len(out) % 4:
        out.append(0)
    doc["bufferViews"] = views
    doc["buffers"] = [{"byteLength": len(out)}]
    used = set(doc.get("extensionsUsed", [])) | {"KHR_mesh_quantization"}
    doc["extensionsUsed"] = sorted(used)
    doc["extensionsRequired"] = sorted(set(doc.get("extensionsRequired", [])) | {"KHR_mesh_quantization"})
    text = json.dumps(doc, separators=(",", ":")).encode()
    text += b" " * ((4 - len(text) % 4) % 4)
    total = 12 + 8 + len(text) + 8 + len(out)
    with open(path, "wb") as f:
        f.write(struct.pack("<III", 0x46546C67, 2, total))
        f.write(struct.pack("<II", len(text), 0x4E4F534A) + text)
        f.write(struct.pack("<II", len(out), 0x004E4942) + bytes(out))


def export(path):
    for o in list(bpy.context.scene.objects):
        if o.type == 'LIGHT' or o.name.startswith("bake."):
            bpy.data.objects.remove(o)
    bpy.ops.export_scene.gltf(filepath=path, export_format='GLB', export_yup=True, export_apply=True, export_normals=True,
                              export_materials='EXPORT', export_vertex_color='NONE', export_attributes=True,
                              export_texcoords=True, export_extras=True, export_cameras=False, export_lights=False)
    rename_attributes(path)
    packer = shutil.which("gltfpack")
    if not packer:
        quantize(path)
    if packer:
        raw = path + ".raw.glb"
        os.replace(path, raw)
        # (-kv keeps the pictures' UVs and the baked sets, which no texture of the file uses, and
        # -vtf the UVs' own values, with no texture to carry their scale; -ke the extras; -vpf
        # the positions as they are: the decals sit a millimetre off their surfaces, and the
        # scene is metres across, the sky outside.)
        subprocess.run([packer, "-i", raw, "-o", path, "-kn", "-km", "-ke", "-kv", "-vc", "12", "-vn", "12", "-vtf", "-vpf"], check=True, capture_output=True)
        if os.environ.get("ROOM_KEEP_RAW"):
            shutil.copy(raw, path + ".unpacked.glb")
        os.remove(raw)


def preview(path, samples=40):
    """A render from the seat, every light on, the tube a cool white."""
    scene = bpy.context.scene
    eye = bpy.data.objects["anchor.eye"].location
    target = bpy.data.objects["anchor.target"].location
    cam_data = bpy.data.cameras.new("eye")
    cam_data.sensor_fit = 'VERTICAL'
    cam_data.angle_y = math.radians(FOV_Y)
    cam_data.clip_start = 0.01
    cam = bpy.data.objects.new("eye", cam_data)
    scene.collection.objects.link(cam)
    cam.location = eye
    cam.rotation_euler = (target - eye).to_track_quat('-Z', 'Y').to_euler()
    scene.camera = cam
    quarters = tube_quarters()
    for _, emit in quarters:
        emit.inputs["Emission Color"].default_value = (0.75, 0.85, 1.0, 1.0)
        emit.inputs["Emission Strength"].default_value = 6.0
    scene.render.engine = 'CYCLES'
    scene.cycles.samples = samples
    scene.cycles.use_denoising = True
    scene.view_settings.view_transform = 'AgX'
    scene.render.resolution_x = 1600
    scene.render.resolution_y = 900
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)
    for obj, _ in quarters:
        bpy.data.objects.remove(obj)


def build_scene(only=None):
    """The whole room, modelled (modifiers applied), ready for previews and the bakes: with every
    season's things (what the file holds), or, for a look at one, as `only` ('normal',
    'christmas' or 'newyear') has it."""
    reset()
    shell()
    window()
    desk()
    cx, cz = monitor()
    x0, x1, top = tower()
    modem(x0, x1, top)
    kb = keyboard()
    mouse_and_mat()
    speakers()
    lava_lamp()
    clock_radio()
    desk_lamp()
    desk_things(kb)
    wall_shelf()
    bookshelf()
    wall_art(cx, cz)
    for name, builder in (("christmas", christmas), ("newyear", newyear)):
        with in_season(name):
            builder()
    with in_season("christmas"):
        window_snow()
    # The seat's view: the game's camera starts here (with the field of view FOV_Y, as
    # `intro_room.rs` has it).
    anchor("eye", (0.0, -0.78, 1.2))
    anchor("target", (0.0, 0.3, 1.2 - 1.08 * math.tan(math.radians(8.3))))
    lights()
    apply_modifiers()
    if only is not None:
        for o in list(bpy.context.scene.objects):
            season = o.get("season")
            if season and season != only:
                bpy.data.objects.remove(o)


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--preview")
    ap.add_argument("--no-bake", action="store_true")
    ap.add_argument("--samples", type=int, default=160)
    ap.add_argument("--season", choices=["normal", "christmas", "newyear"], help="only a preview of this season's room (the file itself always holds them all)")
    args = ap.parse_args(argv)
    os.makedirs(args.out, exist_ok=True)
    build_scene(args.season)
    if os.environ.get("ROOM_REPORT"):
        sizes = sorted(((len(o.data.vertices), o.name) for o in meshes()), reverse=True)
        print("ROOM_REPORT total verts", sum(v for v, _ in sizes))
        for v, name in sizes[:30]:
            print("ROOM_REPORT", v, name)
    if args.preview:
        os.makedirs(args.preview, exist_ok=True)
        preview(os.path.join(args.preview, "room.png"))
    if args.season:
        return
    merge_static()
    if not args.no_bake:
        subdivide_for_bake()
        bake(args.samples)
    path = os.path.join(args.out, "room.glb")
    export(path)
    faces = sum(len(o.data.polygons) for o in meshes())
    print(f"room: {faces} faces -> {path}")


if __name__ == "__main__":
    main()
