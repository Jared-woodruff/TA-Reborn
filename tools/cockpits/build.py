"""Builds the Perspective cockpits (original art for TA Reborn) and exports them as GLB; and the
textures they share.

    blender -b -P tools/cockpits/build.py -- --out crates/ta_render/cockpits [--only vehicle,walker]
        [--preview scratch/cockpits] [--no-bake]
    blender -b -P tools/cockpits/build.py -- --out crates/ta_render/cockpits --textures

(After changing `art.py`'s atlas, both: the models' decals take their places in it. Then `oxipng
-o 3` the two PNGs. `TA_COCKPITS=<folder>` shows a build in the game without building the game:
the models, the textures and `cockpit.wgsl` from that folder.) The GLBs are packed by `quantize`
here (no `gltfpack`: it would drop the materials' extras).

Every cockpit is modelled round the eye at the origin, looking along +Y with +Z up (Blender's
axes; the exporter turns them into glTF's, -Z forward and +Y up), in metres. The view's edges
decide the frame: the game draws the cockpit with the camera's own field of view (64-74° high,
16:9), so what lies outside the frustum is never seen.

They are a crew's stations after years in the field: painted steel, chipped to primer and bare
metal at its edges and where hands go, grime and soot in its seams; rubber grips, padding;
instruments in deep bezels screwed to the console; stencils, warning plates, labels and data
plates; welds, rivets and bolts. The paint, its wear and the grime are the renderer's
(`cockpit.wgsl`), from the masks baked here and the textures drawn by `art.py`.

Material names carry their role for the renderer (`cockpit.rs`), and their extras how they wear
(`wear`, `grime`: 0 none, 1 as much as the paint):
  paint (the hull's paint: the side's own at run time), paint_dark (black enamel: bezels,
  housings, beams), hazard, hazard_black, red (a switch's cover), trim (the side's colour):
  painted steel, chipped to primer and bare steel;
  steel, brass, weld: bare metal, dull; rubber, vinyl (padding), canvas; concrete;
  decal (paint from the shared atlas `decals.png`, by its UVs: stencils, plates, labels);
  screen.<name> (an instrument's glass: the game draws the instrument over it; the aircraft's
  one wide `screen.pcd` is a panoramic display, the game draws its panes side by side on it),
  lamp.<name> (a lamp's lens: lit by the game), glass (tinted, see-through), glow_trim (glows in
  the side's colour).
Objects named anim.<part> keep their own transform (pivot at their origin): the game moves
them (levers, sticks, a helm).

Each vertex carries four masks in its colour (see `bake_masks`, `write_masks`):
  r  ambient occlusion (Cycles, within 35 cm);
  g  how far it is from the nearest convex edge of the shapes as they were built, before their
     bevels rounded them: 0..1 for 0..3 cm (the paint chips there);
  b  occlusion within 3.5 cm (Cycles): seams, crevices, corners, where grime and soot gather;
  a  where the crew's hands go (grips, the console's lip, a wheel): scuffed and polished.

`--textures` draws the shared textures (`art.py`): `wear.png` and `decals.png`.
"""

import argparse
import json
import math
import os
import random
import struct
import sys

import bpy
import bmesh
import numpy as np
from mathutils import Matrix, Vector
from mathutils.kdtree import KDTree

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import art  # noqa: E402

# ---------------------------------------------------------------------------------------------
# Scene and materials
# ---------------------------------------------------------------------------------------------

MATERIALS = {
    # name: (base colour (linear), metallic, roughness, emission colour, emission strength,
    #        wear, grime)
    "paint": ((0.075, 0.078, 0.074), 0.0, 0.66, None, 0.0, 1.0, 1.0),
    "paint_dark": ((0.024, 0.025, 0.025), 0.0, 0.6, None, 0.0, 0.45, 1.0),
    "hazard": ((0.56, 0.38, 0.04), 0.0, 0.66, None, 0.0, 1.1, 1.0),
    "hazard_black": ((0.02, 0.02, 0.018), 0.0, 0.7, None, 0.0, 1.1, 1.0),
    "red": ((0.34, 0.035, 0.022), 0.0, 0.55, None, 0.0, 0.6, 0.8),
    "trim": ((0.25, 0.42, 0.72), 0.0, 0.6, None, 0.0, 0.22, 0.7),
    "steel": ((0.34, 0.335, 0.32), 1.0, 0.52, None, 0.0, 0.6, 1.0),
    "brass": ((0.34, 0.25, 0.12), 1.0, 0.5, None, 0.0, 0.6, 1.0),
    "weld": ((0.2, 0.19, 0.17), 1.0, 0.72, None, 0.0, 0.2, 1.3),
    "rubber": ((0.021, 0.021, 0.02), 0.0, 0.86, None, 0.0, 0.5, 0.8),
    "vinyl": ((0.028, 0.026, 0.024), 0.0, 0.6, None, 0.0, 0.7, 0.6),
    "canvas": ((0.075, 0.07, 0.045), 0.0, 0.95, None, 0.0, 0.3, 1.0),
    "glass": ((0.4, 0.5, 0.45), 0.0, 0.05, None, 0.0, 0.0, 0.0),
    "concrete": ((0.2, 0.19, 0.17), 0.0, 0.94, None, 0.0, 0.9, 1.3),
    "decal": ((1.0, 1.0, 1.0), 0.0, 0.7, None, 0.0, 0.45, 0.45),
    # Glows in the side's colour (set at run time).
    "glow_trim": ((0.1, 0.25, 0.6), 0.0, 0.4, (0.3, 0.55, 1.0), 2.5, 0.0, 0.0),
}

# The openings' glass, made once the occlusion is baked (glass in the way would darken the
# cabin behind it): (name, outline, distance, the frame's depth and chamfer) of each opening.
PANES = []
# Where the crew's hands go: (a, b, radius), segments in the cockpit's space.
HANDS = []
# The convex edges of the shapes as built (before bevels): (a, b, normal of one face, of the
# other), in the cockpit's space. See `collect_edges`.
EDGES = []
# Every random choice is seeded, so a build gives the same models each time.
RNG = random.Random(7)


def reset():
    PANES.clear()
    HANDS.clear()
    EDGES.clear()
    RNG.seed(7)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.unit_settings.system = 'METRIC'
    return scene


def material(name):
    m = bpy.data.materials.get(name)
    if m:
        return m
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    bsdf = next(n for n in m.node_tree.nodes if n.type == 'BSDF_PRINCIPLED')
    key = name.split(".")[0]
    wear, grime = 0.0, 0.0
    if key == "screen":
        base, metal, rough, emit, strength = (0.004, 0.012, 0.006), 0.0, 0.15, (0.1, 0.35, 0.12), 0.4
    elif key == "lamp":
        colour = {"red": (1.0, 0.12, 0.05), "green": (0.3, 1.0, 0.2), "amber": (1.0, 0.62, 0.12), "alarm": (0.3, 1.0, 0.2)}
        c = next((v for k, v in colour.items() if k in name), (1.0, 0.8, 0.5))
        base, metal, rough, emit, strength = c, 0.0, 0.3, c, 3.0
    else:
        base, metal, rough, emit, strength, wear, grime = MATERIALS[key]
    bsdf.inputs["Base Color"].default_value = (*base, 1.0)
    bsdf.inputs["Metallic"].default_value = metal
    bsdf.inputs["Roughness"].default_value = rough
    if emit:
        bsdf.inputs["Emission Color"].default_value = (*emit, 1.0)
        bsdf.inputs["Emission Strength"].default_value = strength
    if key == "glass":
        bsdf.inputs["Alpha"].default_value = 0.18
    # (Exported as the material's extras.)
    m["wear"] = wear
    m["grime"] = grime
    return m


def finish(obj, mat, bevel=0.0, segments=2, smooth=False):
    """Links `obj`, gives it its material and (optionally) a bevel on its hard edges."""
    bpy.context.collection.objects.link(obj)
    obj.data.materials.append(material(mat))
    if bevel > 0.0:
        mod = obj.modifiers.new("bevel", 'BEVEL')
        mod.width = bevel
        mod.segments = segments
        mod.limit_method = 'ANGLE'
        mod.angle_limit = math.radians(35)
        mod.harden_normals = True
    for p in obj.data.polygons:
        p.use_smooth = smooth
    if smooth or bevel > 0.0:
        # Hard edges stay hard, curved surfaces and bevels shade smooth.
        obj.data.set_sharp_from_angle(angle=math.radians(40))
        for p in obj.data.polygons:
            p.use_smooth = True
    return obj


def mesh_object(name, bm):
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    return bpy.data.objects.new(name, me)


def frame_of(normal, up=None):
    """Right, normal and up (unit vectors) for a face looking along `normal`, `up` as near
    upright as it can be (by default the world's up, or y where the face looks up or down)."""
    n = Vector(normal).normalized()
    u = Vector(up) if up is not None else (Vector((0, 0, 1)) if abs(n.z) < 0.9 else Vector((0, 1, 0)))
    u = (u - n * u.dot(n)).normalized()
    r = u.cross(n).normalized()
    return r, n, u


def basis(r, n, u):
    """The matrix taking a shape's x, y, z to `r`, `n`, `u`."""
    return Matrix((r, n, u)).transposed().to_4x4()


# ---------------------------------------------------------------------------------------------
# Shapes
# ---------------------------------------------------------------------------------------------

def box(name, centre, size, mat, bevel=0.004, rot=(0.0, 0.0, 0.0)):
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=Vector(size), verts=bm.verts)
    bmesh.ops.rotate(bm, cent=Vector((0, 0, 0)), matrix=Matrix.Rotation(rot[0], 3, 'X') @ Matrix.Rotation(rot[1], 3, 'Y') @ Matrix.Rotation(rot[2], 3, 'Z'), verts=bm.verts)
    bmesh.ops.translate(bm, vec=Vector(centre), verts=bm.verts)
    return finish(mesh_object(name, bm), mat, bevel)


def oriented_box(name, centre, frame, size, mat, bevel=0.002):
    """A box `size` (across, out, up) round `centre`, turned to `frame` (right, normal, up)."""
    m = box(name, (0, 0, 0), size, mat, bevel=bevel)
    m.matrix_world = Matrix.Translation(Vector(centre)) @ basis(*frame)
    return m


def quad_slab(name, corners, thickness, mat, bevel=0.003):
    """A plate over four corners (in order round it), `thickness` thick along its normal."""
    bm = bmesh.new()
    vs = [bm.verts.new(Vector(c)) for c in corners]
    f = bm.faces.new(vs)
    bm.normal_update()
    n = f.normal.copy()
    ext = bmesh.ops.extrude_face_region(bm, geom=[f])
    moved = [e for e in ext["geom"] if isinstance(e, bmesh.types.BMVert)]
    bmesh.ops.translate(bm, vec=-n * thickness, verts=moved)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return finish(mesh_object(name, bm), mat, bevel)


def frame_plate(name, outer, inner, depth, mat, chamfer=0.02, bevel=0.004):
    """An armour plate with an opening: `outer` and `inner` are loops of (x, z) at the plate's
    front (y = `depth[0]`), the plate going back to `depth[1]`; the opening's lip is chamfered
    towards the eye."""
    y0, y1 = depth
    bm = bmesh.new()
    n = len(outer)
    assert n == len(inner)
    of = [bm.verts.new((x, y0, z)) for x, z in outer]
    inf = [bm.verts.new((x, y0, z)) for x, z in inner]
    ob = [bm.verts.new((x, y1, z)) for x, z in outer]
    # The back of the opening is wider: the lip slopes away from the eye.
    cx = sum(p[0] for p in inner) / n
    cz = sum(p[1] for p in inner) / n
    ib = [bm.verts.new((cx + (x - cx) + math.copysign(chamfer, x - cx), y1, cz + (z - cz) + math.copysign(chamfer, z - cz))) for x, z in inner]
    for i in range(n):
        j = (i + 1) % n
        bm.faces.new((of[i], of[j], inf[j], inf[i]))      # front face (towards the eye)
        bm.faces.new((ob[j], ob[i], ib[i], ib[j]))        # back face
        bm.faces.new((of[j], of[i], ob[i], ob[j]))        # outer rim
        bm.faces.new((inf[i], inf[j], ib[j], ib[i]))      # the opening's lip
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return finish(mesh_object(name, bm), mat, bevel)


def cylinder(name, a, b, radius, mat, segments=16, caps=True, bevel=0.0):
    a, b = Vector(a), Vector(b)
    axis = b - a
    length = axis.length
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=caps, cap_tris=False, segments=segments, radius1=radius, radius2=radius, depth=length)
    rot = axis.to_track_quat('Z', 'Y').to_matrix().to_4x4()
    bmesh.ops.transform(bm, matrix=Matrix.Translation((a + b) / 2) @ rot, verts=bm.verts)
    return finish(mesh_object(name, bm), mat, bevel, smooth=True)


def pipe(name, points, radius, mat, segments=12):
    """A pipe along a polyline, with a joint ball at each bend."""
    objs = []
    for k in range(len(points) - 1):
        objs.append(cylinder(f"{name}_{k}", points[k], points[k + 1], radius, mat, segments, caps=False))
    for k in range(1, len(points) - 1):
        objs.append(sphere(f"{name}_j{k}", points[k], radius * 1.25, mat))
    return objs


def sphere(name, centre, radius, mat, segments=12):
    bm = bmesh.new()
    bmesh.ops.create_uvsphere(bm, u_segments=segments, v_segments=max(segments // 2, 4), radius=radius)
    bmesh.ops.translate(bm, vec=Vector(centre), verts=bm.verts)
    return finish(mesh_object(name, bm), mat, smooth=True)


def torus(name, centre, normal, major, minor, mat, segments=32):
    bm = bmesh.new()
    verts = []
    ring = 8
    for i in range(segments):
        a = i / segments * math.tau
        for j in range(ring):
            b = j / ring * math.tau
            r = major + minor * math.cos(b)
            verts.append(bm.verts.new((r * math.cos(a), r * math.sin(a), minor * math.sin(b))))
    for i in range(segments):
        for j in range(ring):
            i2, j2 = (i + 1) % segments, (j + 1) % ring
            bm.faces.new((verts[i * ring + j], verts[i2 * ring + j], verts[i2 * ring + j2], verts[i * ring + j2]))
    rot = Vector(normal).to_track_quat('Z', 'Y').to_matrix().to_4x4()
    bmesh.ops.transform(bm, matrix=Matrix.Translation(Vector(centre)) @ rot, verts=bm.verts)
    return finish(mesh_object(name, bm), mat, smooth=True)


def disc(name, centre, normal, radius, mat, segments=32):
    bm = bmesh.new()
    bmesh.ops.create_circle(bm, cap_ends=True, segments=segments, radius=radius)
    rot = Vector(normal).to_track_quat('Z', 'Y').to_matrix().to_4x4()
    bmesh.ops.transform(bm, matrix=Matrix.Translation(Vector(centre)) @ rot, verts=bm.verts)
    return finish(mesh_object(name, bm), mat)


def quad(name, corners, mat):
    bm = bmesh.new()
    bm.faces.new([bm.verts.new(Vector(c)) for c in corners])
    return finish(mesh_object(name, bm), mat)


def lathe(name, centre, normal, profile, mat, segments=24, phase=0.0, smooth=True):
    """A shape turned about the axis through `centre` along `normal`: `profile` is (radius,
    height along the axis) from its outer foot over its top (to the axis, closing it, or down
    its inside); its faces look out of it."""
    r, n, u = frame_of(normal)
    c = Vector(centre)
    bm = bmesh.new()
    angles = [phase + k / segments * math.tau for k in range(segments)]
    rings = []
    for rad, h in profile:
        if rad < 1e-7:
            rings.append([bm.verts.new(c + n * h)])
        else:
            rings.append([bm.verts.new(c + n * h + (r * math.cos(a) + u * math.sin(a)) * rad) for a in angles])
    for k in range(len(profile) - 1):
        A, B = rings[k], rings[k + 1]
        dr, dh = profile[k + 1][0] - profile[k][0], profile[k + 1][1] - profile[k][1]
        for i in range(segments):
            j = (i + 1) % segments
            if len(A) == 1 and len(B) == 1:
                continue
            if len(A) == 1:
                f = bm.faces.new((A[0], B[i], B[j]))
            elif len(B) == 1:
                f = bm.faces.new((A[i], A[j], B[0]))
            else:
                f = bm.faces.new((A[i], A[j], B[j], B[i]))
            # Outwards: the profile's right-hand side as it is walked (rotated about the axis).
            mid = (angles[i] + angles[j]) / 2
            radial = r * math.cos(mid) + u * math.sin(mid)
            want = radial * dh - n * dr
            f.normal_update()
            if f.normal.dot(want) < 0:
                f.normal_flip()
    return finish(mesh_object(name, bm), mat, smooth=smooth)


def screw(name, centre, normal, radius=0.0028, mat="steel"):
    """A screw's domed head."""
    h = radius * 0.7
    return lathe(name, centre, normal, [(radius, 0.0), (radius, h * 0.4), (radius * 0.7, h * 0.9), (0.0, h)], mat, segments=10)


def bolt(name, centre, normal, radius=0.0058, mat="steel"):
    """A hex bolt's head on its washer."""
    washer = lathe(f"{name}_w", centre, normal, [(radius * 1.45, 0.0), (radius * 1.45, 0.0012), (0.0, 0.0012)], mat, segments=14)
    head = lathe(f"{name}_h", Vector(centre) + Vector(normal).normalized() * 0.0012, normal, [(radius, 0.0), (radius, radius * 0.55), (radius * 0.84, radius * 0.7), (0.0, radius * 0.7)], mat, segments=6, phase=RNG.random() * math.tau)
    return washer, head


def rivet(name, centre, normal, radius, mat="paint"):
    """A rivet's dome, painted with what it holds."""
    return lathe(name, centre, normal, [(radius, 0.0), (radius * 0.9, radius * 0.22), (radius * 0.55, radius * 0.4), (0.0, radius * 0.46)], mat, segments=10)


def rivets(name, a, b, count, radius, normal, mat="paint"):
    a, b, n = Vector(a), Vector(b), Vector(normal).normalized()
    return [rivet(f"{name}_{k}", a.lerp(b, (k + 0.5) / count) - n * radius * 0.1, n, radius, mat) for k in range(count)]


def bolts(name, a, b, count, radius, normal):
    a, b, n = Vector(a), Vector(b), Vector(normal).normalized()
    return [bolt(f"{name}_{k}", a.lerp(b, (k + 0.5) / count), n, radius) for k in range(count)]


def weld(name, points, normal, width=0.0075, pitch=0.0046):
    """A weld's bead along a seam (a polyline on a surface facing `normal`): a low ridge rippled
    where each pass of the torch ended."""
    n = Vector(normal).normalized()
    pts = [Vector(p) for p in points]
    path = []
    for a, b in zip(pts, pts[1:]):
        steps = max(1, int((b - a).length / (pitch / 2)))
        path += [(a.lerp(b, k / steps), (b - a).normalized()) for k in range(steps)]
    path.append((pts[-1], (pts[-1] - pts[-2]).normalized()))
    bm = bmesh.new()
    rings = []
    side = 7
    for k, (p, d) in enumerate(path):
        across = d.cross(n).normalized()
        s = k * pitch / 2
        ripple = 0.86 + 0.14 * abs(math.sin(math.pi * s / pitch)) ** 0.6
        ends = min(1.0, (k + 0.5) / 3, (len(path) - k - 0.5) / 3)
        w = width / 2 * ripple * (0.6 + 0.4 * ends) * (0.94 + 0.12 * RNG.random())
        hgt = width * 0.34 * ripple * ends
        lean = (RNG.random() - 0.5) * width * 0.12
        ring = []
        for j in range(side):
            a = math.pi * j / (side - 1)
            ring.append(bm.verts.new(p + across * (math.cos(a) * w + lean) + n * (math.sin(a) * hgt - 0.0004)))
        rings.append(ring)
    for k in range(len(rings) - 1):
        for j in range(side - 1):
            f = bm.faces.new((rings[k][j], rings[k + 1][j], rings[k + 1][j + 1], rings[k][j + 1]))
            f.normal_update()
            if f.normal.dot(n) < 0:
                f.normal_flip()
    return finish(mesh_object(name, bm), "weld", smooth=True)


def decal(name, item, centre, normal, height, up=None, lift=0.0008):
    """Paint from the shared atlas (`art.DECALS[item]`): a quad `height` tall (as wide as the
    picture's shape) on a surface facing `normal`, `up` as upright as it can be."""
    x, y, w, h = art.DECALS[item]
    aw, ah = art.ATLAS_SIZE
    width = height * w / h
    r, n, u = frame_of(normal, up)
    c = Vector(centre) + n * lift
    corners = [c - r * width / 2 - u * height / 2, c + r * width / 2 - u * height / 2, c + r * width / 2 + u * height / 2, c - r * width / 2 + u * height / 2]
    # (Half a pixel in from the item's edge: its neighbours never show.)
    u0, u1 = (x + 0.5) / aw, (x + w - 0.5) / aw
    v0, v1 = 1 - (y + h - 0.5) / ah, 1 - (y + 0.5) / ah
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("UVMap")
    f = bm.faces.new([bm.verts.new(p) for p in corners])
    for loop, t in zip(f.loops, [(u0, v0), (u1, v0), (u1, v1), (u0, v1)]):
        loop[uv].uv = t
    f.normal_update()
    if f.normal.dot(n) < 0:
        f.normal_flip()
    return finish(mesh_object(name, bm), "decal"), width


def plate(name, item, centre, normal, height, up=None, mat="paint_dark", thick=0.0016):
    """A warning or data plate: thin steel, riveted at its corners, painted with `item`."""
    x, y, w, h = art.DECALS[item]
    width = height * w / h
    fr = frame_of(normal, up)
    r, n, u = fr
    c = Vector(centre)
    oriented_box(f"{name}_plate", c + n * (thick / 2), fr, (width + 0.002, thick, height + 0.002), mat, bevel=0.0006)
    decal(f"{name}_face", item, c + n * thick, n, height, u, lift=0.0003)
    inset = min(height, width) * 0.075
    for sx in (-1, 1):
        for sy in (-1, 1):
            rivet(f"{name}_rivet{sx}{sy}", c + n * (thick + 0.0002) + r * sx * (width / 2 - inset) + u * sy * (height / 2 - inset), n, min(0.0026, height * 0.06), "steel")
    return width


def pad(name, centre, normal, width, height, depth, up=None, mat="vinyl", tufts=3):
    """A padded cushion: a soft block, its stitched seams dividing it into `tufts`."""
    fr = frame_of(normal, up)
    r, n, u = fr
    c = Vector(centre)
    m = oriented_box(f"{name}_pad", c + n * (depth / 2), fr, (width, depth, height), mat, bevel=min(depth, height) * 0.42)
    m.modifiers["bevel"].segments = 4
    for k in range(1, tufts):
        x = -width / 2 + width * k / tufts
        oriented_box(f"{name}_seam{k}", c + r * x + n * (depth - 0.0012), fr, (0.0022, 0.0028, height * 0.86), "rubber", bevel=0.0)
    return m


def guarded_switch(name, centre, normal, up, label=None, cover=False):
    """A toggle on its hex nut between two guard plates, its label under it; or under a hinged
    red cover (`cover`), its SAFE/LIVE plate beside it."""
    fr = frame_of(normal, up)
    r, n, u = fr
    c = Vector(centre)
    lathe(f"{name}_nut", c, n, [(0.0058, 0.0), (0.0058, 0.0032), (0.0, 0.0032)], "steel", segments=6, phase=0.3)
    tip = c + n * 0.021 + u * 0.007
    cylinder(f"{name}_bat", c + n * 0.003, tip, 0.0021, "steel", segments=8)
    sphere(f"{name}_tip", tip, 0.0032, "steel", segments=8)
    for s in (-1, 1):
        oriented_box(f"{name}_guard{s}", c + r * s * 0.0115 + n * 0.0115 + u * 0.002, fr, (0.0024, 0.023, 0.026), "paint_dark", bevel=0.0008)
    if cover:
        # Shut over the toggle, hinged along its top: red, its SAFE/LIVE plate over it.
        oriented_box(f"{name}_cover", c + n * 0.026 + u * 0.0015, fr, (0.0205, 0.0026, 0.033), "red", bevel=0.001)
        cylinder(f"{name}_hinge", c + n * 0.025 + u * 0.018 - r * 0.012, c + n * 0.025 + u * 0.018 + r * 0.012, 0.0022, "steel", segments=8)
        decal(f"{name}_safe", "safe_live", c + u * 0.04, n, 0.026, u)
    if label:
        decal(f"{name}_label", f"label.{label}", c - u * 0.026, n, 0.0085, u)
    HANDS.append((c, c + n * 0.02, 0.035))


def view_angles(points):
    """Where points fall in the view (the eye at the origin, looking along +y): each one's degrees
    below the view's middle and to the right of it, as the picture shows them (a flat picture
    places a point by its height and its width over its depth)."""
    return [(math.degrees(math.atan2(-p.z, p.y)), math.degrees(math.atan2(p.x, p.y))) for p in points]


def report_view(label, fov, glass, reach, aspect=16 / 9, margin=4.0):
    """Prints where a screen lands in the view (`fov` degrees high, `aspect` wide: the game's
    own), from its glass's four corners and the points its frame reaches (`monitor` returns both),
    and says so when any of it comes within `margin` degrees of the picture's edge: the cockpit
    gives with the unit's ride, and the whole screen must stay in the picture."""
    half_v = fov / 2.0
    half_h = math.degrees(math.atan(math.tan(math.radians(half_v)) * aspect))
    names = ("bottom left", "bottom right", "top right", "top left")
    g = view_angles(glass)
    a = view_angles(glass + reach)
    print(f"view {label}: the picture is {fov:g} degrees high, {2 * half_h:.1f} wide (its edges {half_v:.1f} above and below the middle, {half_h:.1f} beside it)")
    print(f"view {label}: glass {(glass[1] - glass[0]).length:.3f} x {(glass[3] - glass[0]).length:.3f} m")
    for name, (below, right) in zip(names, g):
        print(f"view {label}:   {name:12s} {abs(below):5.1f} {'below' if below >= 0 else 'above'} the middle, {abs(right):5.1f} {'right' if right >= 0 else 'left'}")
    low_g, low_a = max(q[0] for q in g), max(q[0] for q in a)
    side_g, side_a = max(abs(q[1]) for q in g), max(abs(q[1]) for q in a)
    print(f"view {label}: glass {min(q[0] for q in g):.1f} to {low_g:.1f} below the middle, up to {side_g:.1f} beside it; "
          f"with its frame {min(q[0] for q in a):.1f} to {low_a:.1f} below, up to {side_a:.1f} beside")
    room = (half_v - low_a, half_h - side_a)
    ok = room[0] >= margin and room[1] >= margin and min(q[0] for q in a) > -half_v + margin
    print(f"view {label}: room to the picture's edge: {half_v - low_g:.1f} below the glass, {room[0]:.1f} below the frame, {room[1]:.1f} beside the frame "
          f"({'in view' if ok else 'WARNING: LESS THAN %g DEGREES TO THE EDGE' % margin})")


def gauge(name, centre, normal, radius, screen):
    """A round instrument: its glass deep in a bezel (a ring standing proud of it, its lip
    chamfered), on a round flange screwed to the console, a housing behind it."""
    n = Vector(normal).normalized()
    c = Vector(centre)
    cylinder(f"{name}_housing", c - n * 0.04, c - n * 0.002, radius + 0.008, "paint_dark", segments=24)
    # The flange: a disc 3 mm thick on the console's face (4 mm behind the glass's middle).
    flange = radius + 0.019
    lathe(f"{name}_flange", c - n * 0.0045, n, [(flange, 0.0), (flange, 0.0022), (flange - 0.0012, 0.0031), (0.0, 0.0031)], "paint_dark", segments=40)
    # The bezel: from the flange to 7.5 mm in front of the glass, its inside sloping in to it.
    lathe(f"{name}_bezel", c - n * 0.0015, n, [(radius + 0.0105, 0.0), (radius + 0.0105, 0.0055), (radius + 0.0075, 0.009), (radius + 0.0035, 0.009), (radius - 0.0006, 0.0035), (radius - 0.0006, 0.0)], "paint_dark", segments=48)
    r, n2, u = frame_of(n)
    for k in range(4):
        a = math.pi / 4 + k * math.pi / 2
        screw(f"{name}_screw{k}", c - n * 0.0014 + (r * math.cos(a) + u * math.sin(a)) * (flange - 0.0062), n)
    disc(f"{name}_glass", c + n * 0.0005, n, radius, f"screen.{screen}", segments=32)


def monitor(name, centre, normal, up, width, height, screen, depth=0.05, bezel=0.012, pad=0.014, flush=False):
    """A rectangular screen in a bezel (`bezel` thick), set into what it stands on (a base plate
    `pad` wider than the glass all round). Bolted (by default): its bars overlap at the corners,
    where screws sit, and the top one is deeper (a brow against glare); `flush`: a thin frame of
    mitred bars one depth all round, no screws. Returns the glass's four corners (bottom left,
    bottom right, top right, top left) and the points where the whole of it reaches furthest,
    for `report_view`."""
    n = Vector(normal).normalized()
    u = Vector(up).normalized()
    r = u.cross(n).normalized()
    c = Vector(centre)
    hw, hh = width / 2, height / 2
    corners = [c - r * hw - u * hh, c + r * hw - u * hh, c + r * hw + u * hh, c - r * hw + u * hh]
    quad(f"{name}_glass", [p + n * 0.001 for p in corners], f"screen.{screen}")
    # The bezel: four bars round the glass (each centred on the glass's edge).
    t = bezel
    brow = 1.0 if flush else 2.2
    frame = Matrix((r, n, u)).transposed().to_4x4()
    for k, (p0, p1) in enumerate([(corners[0], corners[1]), (corners[1], corners[2]), (corners[2], corners[3]), (corners[3], corners[0])]):
        mid = (p0 + p1) / 2
        along = p1 - p0
        horizontal = abs(along.normalized().dot(r)) > 0.5
        # (Flush: the horizontal bars run over the corners, the vertical ones between them.)
        length = (along.length + t * 2) if not flush else (along.length + (t if horizontal else -t))
        deep = t * (brow if k == 2 else 1.0)
        size = (length, deep, t) if horizontal else (t, deep, length)
        m = box(f"{name}_bezel{k}", (0, 0, 0), size, "paint_dark", bevel=0.002)
        m.matrix_world = Matrix.Translation(mid + n * (deep / 2)) @ frame
        if not flush and k in (0, 2):
            for s in (-1, 1):
                screw(f"{name}_screw{k}{s}", mid + r * s * (length / 2 - t * 0.5) + n * deep, n, 0.0028)
    back = box(f"{name}_back", (0, 0, 0), (width + pad * 2, depth, height + pad * 2), "paint_dark", bevel=0.003)
    back.matrix_world = Matrix.Translation(c - n * depth / 2) @ frame
    # Where it reaches furthest: the base plate's corners, and the bezel's (the top bar's deepest).
    out = t / 2 if flush else t
    reach = []
    for sx in (-1, 1):
        for su in (-1, 1):
            reach.append(c + r * (sx * (hw + pad)) + u * (su * (hh + pad)))
            reach.append(c + r * (sx * (hw + out)) + u * (su * (hh + out)) + n * (t * (brow if su > 0 else 1.0)))
    return corners, reach


def heading_display(top_deg, dist, overlap=0.0, width_deg=24.0, height_deg=2.6):
    """The heading display: a slim screen over the middle of the opening, just above its top
    edge (`overlap` degrees down over it, where the frame leaves no room), facing the eye, in
    front of the frame and inside the view."""
    d = dist - 0.065
    low = math.radians(top_deg + 0.6 - overlap)
    high = math.radians(top_deg + 0.6 - overlap + height_deg)
    mid = (low + high) / 2
    c = Vector((0.0, d, d * math.tan(mid)))
    n = (-c).normalized()
    up = (Vector((0.0, 0.0, 1.0)) - n * n.z).normalized()
    width = 2.0 * d * math.tan(math.radians(width_deg / 2))
    height = d * (math.tan(high) - math.tan(low))
    monitor("compass", c, n, up, width, height, "compass", depth=0.03, bezel=0.005)


def status_lamps(name, centre, normal, up, spacing=0.05, radius=0.0075, colours=("green", "green", "alarm.hull", "amber")):
    """The status lamps, a row of them (by default four: power, systems, the hull (its alarm),
    weapons) or, with `up` along the console's width, a column; `colours` are the lamps in
    turn, so that a pair of them can stand either side of a screen."""
    n = Vector(normal).normalized()
    u = Vector(up).normalized()
    r = u.cross(n).normalized()
    c = Vector(centre)
    count = len(colours)
    plate_ = box(f"{name}_plate", (0, 0, 0), (spacing * (count + 0.4), 0.012, radius * 4.2), "paint_dark", bevel=0.003)
    plate_.matrix_world = Matrix.Translation(c - n * 0.004) @ Matrix((r, n, u)).transposed().to_4x4()
    for k, col in enumerate(colours):
        lamp(f"{name}{k}", c + r * (k - (count - 1) / 2) * spacing + n * 0.004, n, radius, col)


def outline(points_deg, dist):
    """A loop of view angles (degrees: right, up) as (x, z) at `dist` ahead."""
    return [(dist * math.tan(math.radians(h)), dist * math.tan(math.radians(v))) for h, v in points_deg]


def at_view(right_deg, up_deg, dist):
    """The point at those view angles (degrees right of and above the middle) `dist` ahead."""
    return Vector((dist * math.tan(math.radians(right_deg)), dist, dist * math.tan(math.radians(up_deg))))


def scaled(loop, k, dz=0.0):
    cx = sum(p[0] for p in loop) / len(loop)
    cz = sum(p[1] for p in loop) / len(loop)
    return [(cx + (x - cx) * k, cz + dz + (z - cz) * k) for x, z in loop]


def framed_opening(name, points_deg, dist, depth, mat="paint", collar="paint_dark", chamfer=0.04, bolted=True):
    """The frame round what the crew looks out through: a plate reaching past the view's edges
    with the opening cut in it (its lip chamfered away from the eye), and a bolted collar; and
    its glass (see `glaze`)."""
    inner = outline(points_deg, dist)
    PANES.append((name, inner, dist, depth, chamfer))
    # Out past the view's edges (60 degrees from the middle), no further: the bake cuts long
    # edges, and unseen triangles still weigh.
    cx = sum(p[0] for p in inner) / len(inner)
    cz = sum(p[1] for p in inner) / len(inner)
    reach = dist * math.tan(math.radians(60))
    outer = []
    for x, z in inner:
        d = Vector((x - cx, z - cz))
        r = max(d.length * 1.25, reach)
        d = d.normalized() * r
        outer.append((cx + d.x, cz + d.y))
    frame_plate(name, outer, inner, (dist, dist + depth), mat, chamfer=chamfer, bevel=0.006)
    inner2 = scaled(inner, 1.015)
    outer2 = scaled(inner, 1.075)
    frame_plate(f"{name}_collar", outer2, inner2, (dist - 0.04, dist), collar, chamfer=0.01, bevel=0.004)
    if bolted:
        n = len(inner2)
        for k in range(n):
            k2 = (k + 1) % n
            a = Vector((outer2[k][0], 0, outer2[k][1])).lerp(Vector((inner2[k][0], 0, inner2[k][1])), 0.5)
            b = Vector((outer2[k2][0], 0, outer2[k2][1])).lerp(Vector((inner2[k2][0], 0, inner2[k2][1])), 0.5)
            count = max(2, int((b - a).length / 0.085))
            bolts(f"{name}_bolt{k}", (a.x, dist - 0.04, a.z), (b.x, dist - 0.04, b.z), count, 0.0058, (0, -1, 0))
    return inner


class Console:
    """A console's face: from `top_deg` below the view's middle at `dist` ahead, leaning back
    `lean_deg` from upright, `height` tall down its face, `half_width` either side."""

    def __init__(self, name, top_deg, dist, lean_deg, half_width, height=0.4, mat="paint", widen=0.06):
        lean = math.radians(lean_deg)
        self.top = Vector((0.0, dist, -dist * math.tan(math.radians(top_deg))))
        self.down = Vector((0.0, -math.sin(lean), -math.cos(lean)))
        self.height = height
        bottom = self.top + self.down * height
        d0 = (-half_width, self.top.y, self.top.z)
        d1 = (half_width, self.top.y, self.top.z)
        d2 = (half_width + widen, bottom.y, bottom.z)
        d3 = (-half_width - widen, bottom.y, bottom.z)
        quad_slab(name, [d3, d2, d1, d0], 0.04, mat, bevel=0.008)
        n = (Vector(d1) - Vector(d0)).cross(Vector(d3) - Vector(d0)).normalized()
        self.normal = -n if n.y > 0 else n
        self.up = -self.down
        self.half_width = half_width
        # The lip along its top, where the crew's forearms rest.
        HANDS.append((self.at(-half_width * 0.55, 0.0, 0.0), self.at(half_width * 0.55, 0.0, 0.0), 0.06))

    def at(self, x, t, lift=0.004):
        """A point on the face: `x` across, `t` down it (0 at the top, 1 at the bottom)."""
        p = self.top + self.down * (self.height * t)
        p.x = x
        return p + self.normal * lift

    def seams(self, name, ts, half, rivet_every=0.07):
        """Seams across the face where its plates meet, riveted along them."""
        for t in ts:
            a, b = self.at(-half, t, 0.001), self.at(half, t, 0.001)
            m = box(f"{name}{t}", (0, 0, 0), (half * 2, 0.004, 0.004), "paint_dark", bevel=0.0)
            m.matrix_world = Matrix.Translation((a + b) / 2) @ Matrix((Vector((1, 0, 0)), self.normal, self.up)).transposed().to_4x4()
            if rivet_every:
                count = int(half * 2 / rivet_every)
                for k in range(count):
                    x = -half + (k + 0.5) * half * 2 / count
                    rivet(f"{name}{t}_rivet{k}", self.at(x, t, 0.0) + self.up * 0.009, self.normal, 0.0036)

    def switches(self, name, t, x0, x1, count):
        for k in range(count):
            x = x0 + (x1 - x0) * k / max(count - 1, 1)
            p = self.at(x, t, 0.008)
            cylinder(f"{name}{k}", p, p + self.normal * 0.026 + self.up * 0.011, 0.0034, "steel", segments=8)
            lamp(f"{name}lamp{k}", self.at(x + 0.026, t, 0.004), self.normal, 0.0042, "green" if k % 3 else "amber")

    def plate(self, name, item, x, t, height):
        """A warning or data plate on the face."""
        return plate(name, item, self.at(x, t, 0.0), self.normal, height, self.up)

    def decal(self, name, item, x, t, height):
        return decal(name, item, self.at(x, t, 0.0), self.normal, height, self.up)

    def switch(self, name, x, t, label=None, cover=False):
        guarded_switch(name, self.at(x, t, 0.0), self.normal, self.up, label, cover)


def instruments(con, speed, hull, radar, weapons, t=0.3, speed_r=0.058, radar_r=0.066, hull_w=0.17, weapons_w=0.22, h=0.115):
    """The four instruments on a console, at `x` positions (None: not fitted)."""
    if speed is not None:
        gauge("speed", con.at(speed, t), con.normal, speed_r, "speed")
    if hull is not None:
        monitor("hull", con.at(hull, t - 0.02), con.normal, con.up, hull_w, h, "hull")
    if radar is not None:
        gauge("radar", con.at(radar, t), con.normal, radar_r, "radar")
    if weapons is not None:
        monitor("weapons", con.at(weapons, t - 0.02), con.normal, con.up, weapons_w, h, "weapons")


def helm(name, centre, normal, radius, spokes=6):
    """A wheel facing `normal` (its object's origin at the hub: the game turns it)."""
    c = Vector(centre)
    n = Vector(normal).normalized()
    parts = [torus(f"{name}_rim", c, n, radius, 0.011, "steel", segments=48), cylinder(f"{name}_hub", c - n * 0.03, c + n * 0.015, 0.03, "brass", segments=20, bevel=0.003)]
    u = Vector((0, 0, 1))
    r = u.cross(n).normalized()
    u = n.cross(r).normalized()
    for k in range(spokes):
        a = k / spokes * math.tau
        d = r * math.cos(a) + u * math.sin(a)
        parts.append(cylinder(f"{name}_spoke{k}", c + d * 0.025, c + d * (radius - 0.006), 0.006, "steel", segments=8))
        parts.append(sphere(f"{name}_knob{k}", c + d * (radius + 0.03), 0.013, "brass", segments=12))
        parts.append(cylinder(f"{name}_peg{k}", c + d * radius, c + d * (radius + 0.022), 0.006, "brass", segments=8))
    obj = join(parts, f"anim.{name}")
    obj.data.transform(Matrix.Translation(-c))
    obj.location = c
    HANDS.append((c, c, radius + 0.06))
    return obj


def stick(name, base, top, k=1.0):
    """A control stick pivoting at `base`, a pistol grip and a trigger at the top (`k` times as
    stout)."""
    base, top = Vector(base), Vector(top)
    d = (top - base).normalized()
    parts = [cylinder(f"{name}_shaft", base, top - d * 0.05 * k, 0.01 * k, "steel", segments=12),
             cylinder(f"{name}_boot", base, base + d * 0.08 * k, 0.035 * k, "rubber", segments=16),
             cylinder(f"{name}_grip", top - d * 0.07 * k, top + d * 0.03 * k, 0.02 * k, "rubber", segments=16, bevel=0.004),
             box(f"{name}_head", top + d * 0.045 * k, (0.034 * k, 0.03 * k, 0.028 * k), "paint_dark", bevel=0.006),
             sphere(f"{name}_button", top + d * 0.06 * k + Vector((0, -0.01, 0.004)) * k, 0.006 * k, "lamp.red.button", segments=10),
             box(f"{name}_trigger", top + d * 0.01 * k + Vector((0, 0.022, 0)) * k, (0.008 * k, 0.012 * k, 0.02 * k), "trim", bevel=0.002)]
    obj = join(parts, f"anim.{name}")
    obj.data.transform(Matrix.Translation(-base))
    obj.location = base
    HANDS.append((top - d * 0.07 * k, top + d * 0.07 * k, 0.05 * k))
    return obj


def walls(side_pipes=True, y0=0.9, y1=-0.2, spread=0.66):
    """Pipes, cables and junction boxes along the walls either side."""
    for sd in (-1, 1):
        if side_pipes:
            pipe(f"pipe_a{sd}", [(sd * (spread + 0.3), y0, -0.2), (sd * spread, 0.5, -0.16), (sd * (spread - 0.04), 0.2, -0.12), (sd * (spread - 0.02), y1, -0.08)], 0.02, "steel")
            pipe(f"pipe_b{sd}", [(sd * (spread + 0.32), y0, 0.24), (sd * (spread + 0.04), 0.45, 0.3), (sd * spread, 0.1, 0.33)], 0.013, "brass")
            pipe(f"cable{sd}", [(sd * spread, 0.52, -0.42), (sd * (spread - 0.06), 0.3, -0.36), (sd * (spread - 0.04), 0.0, -0.32)], 0.009, "rubber")
        box(f"junction{sd}", (sd * (spread - 0.02), 0.4, 0.06), (0.08, 0.12, 0.14), "paint_dark", bevel=0.006)
        lamp(f"lamp_side{sd}", (sd * (spread - 0.06), 0.38, 0.1), (-sd, -0.4, 0), 0.008, "amber")


def lamp(name, centre, normal, radius, colour):
    n = Vector(normal).normalized()
    c = Vector(centre)
    cylinder(f"{name}_housing", c - n * 0.02, c, radius + 0.005, "paint_dark", segments=16)
    torus(f"{name}_ring", c, n, radius + 0.002, 0.0025, "steel", segments=16)
    sphere(f"{name}_lens", c + n * 0.001, radius, f"lamp.{colour}.{name}", segments=12)


def lever(name, base, top, grip_radius=0.022):
    """A lever pivoting at `base` (its object's origin), a rubber grip at the top."""
    base, top = Vector(base), Vector(top)
    d = (top - base).normalized()
    parts = [cylinder(f"{name}_shaft", base, top - d * 0.06, 0.009, "steel", segments=12),
             cylinder(f"{name}_grip", top - d * 0.08, top, grip_radius, "rubber", segments=16, bevel=0.004),
             sphere(f"{name}_knob", top, grip_radius * 1.05, "rubber", segments=16),
             cylinder(f"{name}_boot", base, base + d * 0.07, 0.03, "rubber", segments=16)]
    obj = join(parts, f"anim.{name}")
    # The pivot at the base.
    obj.data.transform(Matrix.Translation(-base))
    obj.location = base
    HANDS.append((top - d * 0.08, top + d * 0.02, 0.05))
    return obj


def hazard_strip(name, a, b, width, normal, up, stripes=14):
    """Yellow and black diagonal bars along a to b."""
    a, b = Vector(a), Vector(b)
    n = Vector(normal).normalized()
    u = Vector(up).normalized()
    along = (b - a) / stripes
    out = []
    for k in range(stripes):
        p0 = a + along * k
        p1 = p0 + along
        mat = "hazard" if k % 2 == 0 else "hazard_black"
        slant = along * 0.5
        out.append(quad(f"{name}_{k}", [p0 + n * 0.001, p1 + n * 0.001, p1 + slant + u * width + n * 0.001, p0 + slant + u * width + n * 0.001], mat))
    return out


def join(objs, name):
    bpy.ops.object.select_all(action='DESELECT')
    for o in objs:
        o.select_set(True)
        # Modifiers applied before joining.
        bpy.context.view_layer.objects.active = o
        for m in list(o.modifiers):
            bpy.ops.object.modifier_apply(modifier=m.name)
    bpy.context.view_layer.objects.active = objs[0]
    for o in objs:
        o.select_set(True)
    bpy.ops.object.join()
    obj = bpy.context.view_layer.objects.active
    obj.name = name
    return obj


def on_frame(right_deg, up_deg, dist):
    """A point on a framed opening's plate, at those view angles (its face `dist` ahead)."""
    return at_view(right_deg, up_deg, dist) - Vector((0, 0.0004, 0))


def corner_weld(name, inner, dist, corner, length=0.16):
    """A weld from a corner of the opening's collar out across the plate, where two of its
    plates were joined: `corner` (x, z) the opening's corner, out along its diagonal."""
    cx = sum(p[0] for p in inner) / len(inner)
    cz = sum(p[1] for p in inner) / len(inner)
    d = Vector((corner[0] - cx, 0, corner[1] - cz)).normalized()
    start = Vector((cx, dist - 0.0002, cz)) + Vector((corner[0] - cx, 0, corner[1] - cz)) * 1.09
    weld(name, [start, start + d * length], (0, -1, 0))


# ---------------------------------------------------------------------------------------------
# The cockpits (the view: 16:9, the kind's field of view high; the eye at the origin)
# ---------------------------------------------------------------------------------------------

def vehicle():
    """A tank commander's station (64 degrees high): an armoured vision port, a console under
    it facing the eye with its instruments, tillers in the lower corners, pipes on the walls."""
    dist = 0.6
    c = 3.5
    port = [(-45.5 + c, -13.5), (45.5 - c, -13.5), (45.5, -13.5 + c), (45.5, 27 - c), (45.5 - c, 27), (-45.5 + c, 27), (-45.5, 27 - c), (-45.5, -13.5 + c)]
    inner = framed_opening("port", port, dist, 0.14)
    bottom = min(z for _, z in inner)
    top = max(z for _, z in inner)
    con = Console("dash", 15.5, 0.55, 25, 0.64, height=0.36)
    quad_slab("shelf", [(-0.66, dist - 0.05, con.top.z + 0.012), (0.66, dist - 0.05, con.top.z + 0.012), (0.64, con.top.y - 0.01, con.top.z + 0.012), (-0.64, con.top.y - 0.01, con.top.z + 0.012)], 0.02, "paint_dark", bevel=0.004)
    box("trim", (0.0, con.top.y - 0.012, con.top.z + 0.005), (1.24, 0.012, 0.012), "trim", bevel=0.002)
    instruments(con, -0.33, -0.13, 0.08, 0.3)
    heading_display(27, dist)
    status_lamps("status", con.at(-0.02, 0.07), con.normal, con.up, spacing=0.034, radius=0.0062)
    con.switches("switch", 0.53, -0.52, 0.52, 11)
    con.seams("seam", (0.12, 0.44, 0.62), 0.66)
    hazard_strip("sill", (outline([(-41, 0)], dist)[0][0], dist - 0.044, bottom - 0.006), (outline([(41, 0)], dist)[0][0], dist - 0.044, bottom - 0.006), 0.018, (0, -1, 0), (0, 0, -1), stripes=30)
    pipe("handle", [(-0.34, dist - 0.04, top + 0.12), (-0.31, dist - 0.11, top + 0.12), (0.31, dist - 0.11, top + 0.12), (0.34, dist - 0.04, top + 0.12)], 0.011, "steel")
    walls()
    # Plates on the band over the instruments; stencils on the collar's top either side of the
    # heading display.
    con.plate("caution", "caution", -0.47, 0.045, 0.024)
    con.plate("data", "data", 0.47, 0.045, 0.026)
    con.decal("volts", "stencil.24V", 0.17, 0.035, 0.014)
    for k, sd in enumerate((-1, 1)):
        decal(f"vb{k}", f"stencil.VB-{k + 1}", on_frame(sd * 25, 28.6, dist - 0.04), (0, -1, 0), 0.017)
    # (Grips and knobs well in the lower corners: a knob alone at the edge read as a stray ball.
    # In front of the console, clear of it by several centimetres all the way up: they rose from
    # behind its foot, their grips against its face, which they touched upright and went into as
    # the game leaned them forward with the tank's speed.)
    lever("lever_l", (-0.40, 0.33, -0.62), (-0.36, 0.38, -0.15))
    lever("lever_r", (0.40, 0.33, -0.62), (0.36, 0.38, -0.15))


def walker():
    """A kbot's cab (68 degrees high): a visor wider at the brow, cheek plates, a console with
    its instruments low down, control sticks either side."""
    dist = 0.5
    visor = [(-46, 21), (-29, 29.5), (29, 29.5), (46, 21), (48, -4), (37, -15.5), (-37, -15.5), (-48, -4)]
    inner = framed_opening("visor", visor, dist, 0.12, chamfer=0.035)
    top = max(z for _, z in inner)
    # The brow: a beam over the visor with the side's colour glowing along it.
    box("brow", (0, dist - 0.03, top + 0.05), (1.3, 0.06, 0.05), "paint_dark", bevel=0.008)
    box("brow_glow", (0, dist - 0.062, top + 0.035), (0.6, 0.004, 0.006), "glow_trim", bevel=0.0)
    # Cheek plates angling in at the sides.
    for sd in (-1, 1):
        quad_slab(f"cheek{sd}", [(sd * 0.62, 0.52, 0.32), (sd * 0.5, 0.3, 0.3), (sd * 0.5, 0.3, -0.35), (sd * 0.62, 0.52, -0.3)], 0.03, "paint", bevel=0.008)
        for z in (0.2, 0.0, -0.2):
            rivets(f"cheek_bolt{sd}{z}", (sd * 0.6, 0.5, z), (sd * 0.52, 0.34, z), 3, 0.006, (-sd, -0.3, 0))
    con = Console("console", 16.5, 0.44, 32, 0.5, height=0.3)
    box("trim", (0.0, con.top.y - 0.01, con.top.z + 0.006), (1.0, 0.012, 0.012), "trim", bevel=0.002)
    instruments(con, -0.27, -0.09, 0.1, 0.28, t=0.24, speed_r=0.047, radar_r=0.055, hull_w=0.14, weapons_w=0.17, h=0.095)
    heading_display(29.5, dist)
    # The lamps in a column between the hull's screen and the radar (in a row over them, the
    # screen's brow hid half of them).
    status_lamps("status", con.at(0.014, 0.22), con.normal, (1.0, 0.0, 0.0), spacing=0.022, radius=0.0055)
    con.switches("switch", 0.52, -0.42, 0.42, 9)
    con.seams("seam", (0.12, 0.5), 0.5)
    walls(spread=0.6)
    # Stencils in the plate's upper corners, beyond the visor's raked edges and its collar; welds
    # where the plate's pieces meet at its lower corners; a plate at each end of the console,
    # over its stick.
    for k, sd in enumerate((-1, 1)):
        decal(f"vb{k}", f"stencil.VB-{k + 1}", on_frame(sd * 44.5, 30.6, dist), (0, -1, 0), 0.024)
        corner_weld(f"weld{k}", inner, dist, outline([(sd * 43, -11)], dist)[0], length=0.14)
    con.plate("keep", "keep_clear", -0.425, 0.12, 0.017)
    con.plate("danger", "danger", 0.435, 0.12, 0.024)
    stick("stick_l", (-0.37, 0.46, -0.6), (-0.34, 0.42, -0.265))
    stick("stick_r", (0.37, 0.46, -0.6), (0.34, 0.42, -0.265))


def titan():
    """An experimental walker's cab (a Krogoth's; 64 degrees high), the heaviest of them: set
    deep in its armour, a broad visor between two raked struts (the middle, where the sight is,
    clear), under a brow of warning lamps; a console wider than any other with its instruments,
    banks of switches either side, a hazard-striped sill; stout sticks in the corners."""
    dist = 0.62
    visor = [(-49, 14), (-35, 19), (35, 19), (49, 14), (51, -4.5), (42, -12), (-42, -12), (-51, -4.5)]
    inner = framed_opening("visor", visor, dist, 0.26, chamfer=0.075)
    top = max(z for _, z in inner)
    bottom = min(z for _, z in inner)
    # The struts across the visor, raked, riveted down their faces, welded at their feet.
    for sd in (-1, 1):
        x = dist * math.tan(math.radians(30))
        box(f"strut{sd}", (sd * x, dist - 0.03, (top + bottom) / 2), (0.06, 0.07, top - bottom + 0.06), "paint_dark", bevel=0.01, rot=(0, sd * 0.1, 0))
        rivets(f"strut_bolt{sd}", (sd * (x - 0.006), dist - 0.067, bottom + 0.03), (sd * (x + 0.006), dist - 0.067, top - 0.03), 6, 0.007, (0, -1, 0), mat="paint_dark")
        for k, z in enumerate((bottom - 0.006, top + 0.006)):
            lean = sd * 0.1 * (1 if z > 0 else -1) * (top - bottom) / 2
            xs = sd * x + (math.sin(sd * 0.1) * ((top + bottom) / 2 - z))
            weld(f"strut_weld{sd}{k}", [(xs - 0.036, dist - 0.0652, z), (xs + 0.036, dist - 0.0652, z)], (0, -1, 0), width=0.009)
    # The brow: a heavy beam over the visor (and over the heading display), the side's colour
    # glowing along its foot, warning lamps along it; plates between them.
    box("brow", (0, dist - 0.05, top + 0.075), (1.7, 0.1, 0.09), "paint_dark", bevel=0.012)
    box("brow_glow", (0, dist - 0.102, top + 0.034), (1.1, 0.004, 0.006), "glow_trim", bevel=0.0)
    for k in range(6):
        lamp(f"brow_lamp{k}", (-0.45 + k * 0.18, dist - 0.102, top + 0.08), (0, -1, 0), 0.009, "red" if k in (0, 5) else "amber")
    plate("danger", "danger", (-0.18, dist - 0.1, top + 0.08), (0, -1, 0), 0.042)
    decal("brow_code", "stencil.A-7", (0.18, dist - 0.1, top + 0.08), (0, -1, 0), 0.026)
    decal("brow_code2", "stencil.07", (-0.36, dist - 0.1, top + 0.08), (0, -1, 0), 0.022)
    decal("brow_code3", "stencil.12", (0.36, dist - 0.1, top + 0.08), (0, -1, 0), 0.022)
    # Armoured cheeks angling in at the sides, heavily riveted.
    for sd in (-1, 1):
        quad_slab(f"cheek{sd}", [(sd * 0.74, 0.6, 0.38), (sd * 0.58, 0.34, 0.36), (sd * 0.58, 0.34, -0.4), (sd * 0.74, 0.6, -0.36)], 0.045, "paint", bevel=0.01)
        for z in (0.27, 0.09, -0.09, -0.27):
            rivets(f"cheek_bolt{sd}{z}", (sd * 0.72, 0.58, z), (sd * 0.6, 0.37, z), 3, 0.0075, (-sd, -0.3, 0))
    # The console: wide and deep, its instruments a walker's and larger.
    con = Console("console", 12.5, 0.5, 30, 0.66, height=0.36)
    quad_slab("sill", [(-0.95, dist - 0.05, bottom - 0.012), (0.95, dist - 0.05, bottom - 0.012), (0.66, con.top.y, con.top.z + 0.012), (-0.66, con.top.y, con.top.z + 0.012)], 0.025, "paint_dark", bevel=0.005)
    hazard_strip("sill_strip", (-0.62, dist - 0.052, bottom - 0.01), (0.62, dist - 0.052, bottom - 0.01), 0.018, (0, -1, 0), (0, 0, -1), stripes=36)
    box("trim", (0.0, con.top.y - 0.012, con.top.z + 0.006), (1.32, 0.012, 0.012), "trim", bevel=0.002)
    instruments(con, -0.36, -0.145, 0.135, 0.36, t=0.22, speed_r=0.056, radar_r=0.064, hull_w=0.17, weapons_w=0.21, h=0.108)
    heading_display(19, dist)
    # (Between the hull's screen and the radar.)
    status_lamps("status", con.at(0.0, 0.13), con.normal, con.up, spacing=0.024, radius=0.0058)
    for sd, x0, x1 in ((-1, -0.62, -0.06), (1, 0.06, 0.62)):
        con.switches(f"switch{sd}", 0.56, x0, x1, 9)
    con.seams("seam", (0.12, 0.47, 0.7), 0.66)
    walls(spread=0.8)
    stick("stick_l", (-0.54, 0.52, -0.66), (-0.5, 0.48, -0.33), k=1.35)
    stick("stick_r", (0.54, 0.52, -0.66), (0.5, 0.48, -0.33), k=1.35)


def commander():
    """The Commander's own cockpit (68 degrees high): a wide armoured visor under a crest of
    lamps and the side's glowing trim, a deep console with more instruments than any other."""
    dist = 0.55
    visor = [(-47, 23), (-25, 31), (25, 31), (47, 23), (49, -3), (39, -15.5), (-39, -15.5), (-49, -3)]
    inner = framed_opening("visor", visor, dist, 0.16, chamfer=0.05)
    top = max(z for _, z in inner)
    bottom = min(z for _, z in inner)
    # The crest over the visor: a raised plate, lamps along it and glowing trim.
    box("crest", (0, dist - 0.035, top + 0.045), (0.7, 0.05, 0.05), "paint_dark", bevel=0.01)
    for k in range(7):
        lamp(f"crest_lamp{k}", (-0.24 + k * 0.08, dist - 0.062, top + 0.045), (0, -1, 0), 0.007, "amber" if k in (0, 6) else "green")
    # Thin lines of it along the visor's raked upper edges, just outside the collar.
    for k, (a, b) in enumerate([(inner[0], inner[1]), (inner[2], inner[3])]):
        a, b = Vector((a[0], 0, a[1])), Vector((b[0], 0, b[1]))
        mid = (a + b) / 2
        edge = box(f"wing{k}", (0, 0, 0), ((b - a).length * 0.8, 0.004, 0.005), "glow_trim", bevel=0.0)
        # (Its length along the edge: a turn about y takes x to (cos, 0, -sin).)
        edge.matrix_world = Matrix.Translation(Vector((mid.x, dist - 0.046, mid.z + 0.045))) @ Matrix.Rotation(math.atan2(-(b.z - a.z), b.x - a.x), 4, 'Y')
    # The glowing trim along the visor's sill.
    box("sill_glow", (0, dist - 0.045, bottom - 0.012), (0.9, 0.004, 0.006), "glow_trim", bevel=0.0)
    con = Console("console", 16.5, 0.47, 30, 0.62, height=0.34)
    box("trim", (0.0, con.top.y - 0.01, con.top.z + 0.006), (1.24, 0.012, 0.012), "trim", bevel=0.002)
    instruments(con, -0.22, -0.05, 0.14, 0.34, t=0.23, speed_r=0.048, radar_r=0.056, hull_w=0.14, weapons_w=0.19, h=0.1)
    # (The visor reaches nearly to the view's top: the display hangs a little over it.)
    heading_display(31, dist, overlap=1.8)
    # The economy's screen (energy and metal: the D-gun takes energy) at the left, and the lamps
    # in a column between the hull's screen and the radar (in a row over the screens, the trim
    # along the console's top and the screen's brow under them each hid half of them).
    monitor("economy", con.at(-0.38, 0.22), con.normal, con.up, 0.11, 0.095, "economy")
    status_lamps("status", con.at(0.057, 0.21), con.normal, (1.0, 0.0, 0.0), spacing=0.022, radius=0.0058)
    con.switches("switch", 0.5, -0.56, 0.56, 13)
    con.seams("seam", (0.13, 0.52), 0.62)
    # The D-gun's switch under its cover at the console's right end; stencils in the plate's
    # upper corners; a plate in each lower corner; welds out from them.
    con.switch("dgun", 0.465, 0.22, "FIRE", True)
    for k, sd in enumerate((-1, 1)):
        decal(f"vb{k}", f"stencil.VB-{k + 1}", on_frame(sd * 46.4, 31.6, dist), (0, -1, 0), 0.022)
        corner_weld(f"weld{k}", inner, dist, outline([(sd * 44, -10)], dist)[0], length=0.13)
    walls(spread=0.7)
    stick("stick_l", (-0.47, 0.5, -0.64), (-0.43, 0.46, -0.3))
    stick("stick_r", (0.47, 0.5, -0.64), (0.43, 0.46, -0.3))


def bridge():
    """A ship's bridge (60 degrees high): a band of windows across its front, three panes
    between two raked mullions (the middle one, where the sight is, clear and wide), the helm in
    the middle of the console under them, the instruments either side."""
    dist = 0.95
    windows = [(-52, -6), (52, -6), (52, 25), (-52, 25)]
    inner = framed_opening("windows", windows, dist, 0.12, chamfer=0.02, bolted=False)
    top = max(z for _, z in inner)
    bottom = min(z for _, z in inner)
    for k, h in enumerate((-31, 31)):
        x = dist * math.tan(math.radians(h))
        lean = 0.06 * (1 if h > 0 else -1)
        box(f"mullion{k}", (x, dist - 0.02, (top + bottom) / 2), (0.045, 0.05, top - bottom + 0.04), "paint_dark", bevel=0.006, rot=(0, lean, 0))
        # (Its number under the eye's level: the window's top, as the game finds it, stays the
        # mullions' own.)
        decal(f"mullion_no{k}", f"yellow.{k + 1}", (x - math.sin(lean) * 0.035, dist - 0.0455, bottom + 0.058), (0, -1, 0), 0.026)
        for z, kk in ((bottom - 0.006, 0), (top + 0.006, 1)):
            weld(f"mullion_weld{k}{kk}", [(x - 0.03, dist - 0.0004, z), (x + 0.03, dist - 0.0004, z)], (0, -1, 0))
    # Plates and stencils on the band over the windows (above their frame's collar, under the
    # view's top), either side of the heading display.
    plate("caution", "caution", on_frame(-26, 28.3, dist), (0, -1, 0), 0.03)
    plate("data", "data", on_frame(26, 28.3, dist), (0, -1, 0), 0.031)
    decal("code0", "stencil.C-3", on_frame(-40, 28.3, dist), (0, -1, 0), 0.028)
    decal("code1", "stencil.HYD 3", on_frame(40, 28.3, dist), (0, -1, 0), 0.028)
    # The console, the helm on it, the instruments either side.
    con = Console("console", 9.0, 0.62, 18, 0.7, height=0.42)
    quad_slab("sill", [(-0.9, dist - 0.02, bottom - 0.01), (0.9, dist - 0.02, bottom - 0.01), (0.72, con.top.y, con.top.z + 0.01), (-0.72, con.top.y, con.top.z + 0.01)], 0.02, "paint_dark", bevel=0.004)
    box("trim", (0.0, con.top.y - 0.01, con.top.z + 0.006), (1.4, 0.012, 0.012), "trim", bevel=0.002)
    instruments(con, -0.46, -0.24, 0.24, 0.46, t=0.26, speed_r=0.062, radar_r=0.07, hull_w=0.17, weapons_w=0.2, h=0.12)
    heading_display(25, dist)
    status_lamps("status", con.at(0.0, 0.06), con.normal, con.up, spacing=0.036, radius=0.0065)
    helm("helm", con.at(0.0, 0.62, 0.06), con.normal, 0.095)
    con.seams("seam", (0.14,), 0.7)
    con.plate("keep", "keep_clear", 0.0, 0.22, 0.022)
    # The deckhead: beams and a lamp.
    box("beam", (0, 0.62, 0.52), (2.0, 0.08, 0.06), "paint_dark", bevel=0.008)
    walls(spread=0.8)


def canopy():
    """A fighter's cockpit (74 degrees high): the windscreen's frame and the canopy's rails, a
    glare shield over the panel (the flight display is drawn across the whole view, so no
    combining glass stands up in it), and under it one wide panoramic display with the status
    lamps either side."""
    dist = 0.62
    # The windscreen's arch, the canopy rails along the sides: a frame round a tall opening.
    arch = [(-52, -15)] + [(-52 + 104 * k / 10, 38 - 25 * (2 * k / 10 - 1) ** 2) for k in range(1, 10)] + [(52, -15)]
    arch = [(h, min(v, 36)) for h, v in arch]
    framed_opening("canopy", arch, dist, 0.05, mat="paint_dark", collar="paint", chamfer=0.01, bolted=False)
    # The windscreen's posts, raked back along the canopy's edges.
    for sd in (-1, 1):
        x = dist * math.tan(math.radians(47))
        box(f"post{sd}", (sd * x, dist - 0.02, 0.08), (0.028, 0.03, 0.9), "paint_dark", bevel=0.006, rot=(-0.35, 0, sd * 0.12))
    # The glare shield and the panel under it; a padded roll along the shield's lip.
    con = Console("panel", 16.0, 0.5, 20, 0.6, height=0.34)
    quad_slab("shield", [(-0.62, con.top.y + 0.1, con.top.z + 0.02), (0.62, con.top.y + 0.1, con.top.z + 0.02), (0.6, con.top.y - 0.06, con.top.z + 0.03), (-0.6, con.top.y - 0.06, con.top.z + 0.03)], 0.015, "rubber", bevel=0.006)
    lip = cylinder("shield_lip", (-0.58, con.top.y - 0.058, con.top.z + 0.022), (0.58, con.top.y - 0.058, con.top.z + 0.022), 0.011, "vinyl", segments=12, bevel=0.0)
    # The panoramic display: one wide glass in the middle of the panel, a flush black frame, the
    # game's three panes (the unit, the radar, the weapons) side by side on it. As tall as the view
    # lets it be: the panel's visible band runs from under the shield (16 degrees below the
    # middle) to the picture's foot (37), and the screen's frame stays 4 or more degrees clear of
    # that (the cockpit rides with the unit): `report_view` prints where it lands. The status
    # lamps stand in pairs either side of it (the hull's alarm by the unit's pane, the weapons'
    # lamp by theirs), level with its middle.
    middle = 0.234
    report_view("canopy pcd", FOV["canopy"], *monitor("pcd", con.at(0.0, middle), con.normal, con.up, 0.60, 0.135, "pcd", depth=0.03, bezel=0.007, pad=0.006, flush=True))
    for sd, side, colours in ((-1, "L", ("green", "alarm.hull")), (1, "R", ("green", "amber"))):
        status_lamps(f"status{side}", con.at(sd * 0.35, middle), con.normal, (1.0, 0.0, 0.0), spacing=0.03, radius=0.0055, colours=colours)
    # Guarded switches outside the lamps (the master arm under its cover); labels and plates.
    con.switch("arm", 0.45, 0.17, "FIRE", True)
    con.switch("ecm", 0.45, 0.32, "ECM")
    con.switch("pwr", -0.45, 0.17, "PWR")
    con.switch("iff", -0.45, 0.32, "IFF")
    con.plate("caution", "caution", -0.5, 0.06, 0.024)
    con.switches("switch", 0.62, -0.5, 0.5, 9)
    box("trim", (0.0, con.top.y - 0.07, con.top.z + 0.035), (1.2, 0.01, 0.01), "trim", bevel=0.002)


def gunship():
    """A gunship's cab (70 degrees high): framed glass all round and a small panel low down with
    one wide panoramic display on it, the ground seen through the chin windows under it."""
    dist = 0.7
    bubble = [(-50, -30), (50, -30), (50, 30), (-50, 30)]
    framed_opening("bubble", bubble, dist, 0.04, mat="paint_dark", collar="paint", chamfer=0.01, bolted=False)
    # Struts dividing the glass either side (none over the middle: the sight and the compass
    # are there).
    for sd in (-1, 1):
        box(f"strut{sd}", (sd * dist * math.tan(math.radians(41)), dist - 0.01, 0.0), (0.028, 0.03, 1.0), "paint_dark", bevel=0.005, rot=(0, sd * 0.08, 0))
        box(f"rail{sd}", (sd * 0.3, dist - 0.012, -dist * math.tan(math.radians(18))), (0.6, 0.028, 0.026), "paint_dark", bevel=0.005)
        decal(f"rail_code{sd}", "stencil.NO STEP" if sd < 0 else "stencil.P-14", (sd * 0.4, dist - 0.0262, -dist * math.tan(math.radians(18))), (0, -1, 0), 0.019)
    # A small panel low in the middle; chin glass either side of it. (Its top 19 degrees below the
    # middle, not 21: only its top 16 degrees show in the 70 degrees' picture, whose foot is 35
    # below the middle, and the display wants them all. It stays lower than the rails above,
    # whose top the game takes for the window's sill, 17 degrees below the middle.)
    con = Console("panel", 19.0, 0.52, 25, 0.36, height=0.26)
    # The panoramic display (see the canopy's): one wide glass in the middle of the panel for the
    # game's three panes, its frame 4 or more degrees clear of the picture's foot (`report_view`
    # prints where it lands), the status lamps in pairs either side of it.
    middle = 0.229
    report_view("gunship pcd", FOV["gunship"], *monitor("pcd", con.at(0.0, middle), con.normal, con.up, 0.46, 0.095, "pcd", depth=0.03, bezel=0.006, pad=0.006, flush=True))
    for sd, side, colours in ((-1, "L", ("green", "alarm.hull")), (1, "R", ("green", "amber"))):
        status_lamps(f"status{side}", con.at(sd * 0.28, middle), con.normal, (1.0, 0.0, 0.0), spacing=0.024, radius=0.005, colours=colours)
    con.switch("arm", 0.34, 0.12, None, True)
    con.switch("pwr", -0.34, 0.12, "PWR")
    box("trim", (0.0, con.top.y - 0.01, con.top.z + 0.006), (0.7, 0.01, 0.01), "trim", bevel=0.002)


def emplacement():
    """A gun house (58 degrees high): a slit in thick concrete under a steel lintel, a console
    under it, a traversing wheel at the side."""
    dist = 0.7
    slit = [(-40, -8), (40, -8), (42, -5), (42, 12), (40, 14), (-40, 14), (-42, 12), (-42, -5)]
    inner = framed_opening("slit", slit, dist, 0.35, mat="concrete", collar="paint", chamfer=0.08, bolted=True)
    top = max(z for _, z in inner)
    bottom = min(z for _, z in inner)
    box("lintel", (0, dist - 0.05, top + 0.06), (1.5, 0.08, 0.07), "paint", bevel=0.008)
    hazard_strip("lintel_strip", (-0.5, dist - 0.092, top + 0.04), (0.5, dist - 0.092, top + 0.04), 0.03, (0, -1, 0), (0, 0, 1), stripes=28)
    # The concrete over the lintel, up past the view (the slit's plate, cut round it, fell short
    # of the view's top over its middle: the sky showed there).
    roof = top + 0.09
    quad_slab("roof", [(-1.1, dist + 0.002, roof), (1.1, dist + 0.002, roof), (1.1, dist + 0.002, 0.62), (-1.1, dist + 0.002, 0.62)], 0.3, "concrete", bevel=0.004)
    # Stencilled on that concrete, and a plate bolted to it.
    # (Over the lintel's top as the eye sees it, 24 degrees up, under the view's, 29.)
    decal("gun", "stencil.GUN 2", on_frame(-30, 26.4, dist), (0, -1, 0), 0.038)
    decal("bay", "stencil.07", on_frame(37, 26.4, dist), (0, -1, 0), 0.042)
    plate("danger", "danger", on_frame(21, 26.3, dist), (0, -1, 0), 0.038)
    con = Console("console", 10.0, 0.6, 22, 0.62, height=0.34)
    quad_slab("sill", [(-0.8, dist - 0.05, bottom - 0.01), (0.8, dist - 0.05, bottom - 0.01), (0.62, con.top.y, con.top.z + 0.01), (-0.62, con.top.y, con.top.z + 0.01)], 0.02, "concrete", bevel=0.004)
    instruments(con, None, -0.3, -0.05, 0.22, t=0.3, radar_r=0.07, hull_w=0.18, weapons_w=0.24, h=0.12)
    heading_display(14, dist)
    status_lamps("status", con.at(-0.05, 0.07), con.normal, con.up, spacing=0.034, radius=0.0062)
    con.plate("data", "data", -0.52, 0.07, 0.028)
    con.decal("volts", "stencil.24V", 0.42, 0.045, 0.016)
    con.switches("switch", 0.6, -0.5, 0.4, 9)
    helm("helm", (0.5, 0.5, -0.24), (-0.5, -1, 0.3), 0.11)
    walls(spread=0.75)


COCKPITS = {"vehicle": vehicle, "walker": walker, "titan": titan, "commander": commander, "bridge": bridge, "canopy": canopy, "gunship": gunship, "emplacement": emplacement}
# Each one's field of view (degrees high), as the game has it (`Cockpit::fov` in perspective.rs).
FOV = {"vehicle": 64, "walker": 68, "titan": 64, "commander": 68, "bridge": 60, "canopy": 74, "gunship": 70, "emplacement": 58}


# ---------------------------------------------------------------------------------------------
# Finishing: the edges, occlusion and masks into vertex colours, export, previews
# ---------------------------------------------------------------------------------------------

def glaze():
    """The glass in each opening: a sheet just behind the frame's far face (the lip and
    everything of the cabin in front of it: the glass hides what is behind it), as large as
    the opening there and a margin more, its edges behind the plate; facing the eye. The game
    draws dust on it, and the rain."""
    for name, inner, dist, depth, chamfer in PANES:
        cx = sum(p[0] for p in inner) / len(inner)
        cz = sum(p[1] for p in inner) / len(inner)
        grow = chamfer + 0.03
        bm = bmesh.new()
        verts = [bm.verts.new((cx + (x - cx) + math.copysign(grow, x - cx), dist + depth + 0.003, cz + (z - cz) + math.copysign(grow, z - cz))) for x, z in inner]
        face = bm.faces.new(verts)
        face.normal_update()
        if face.normal.y > 0:
            face.normal_flip()
        obj = finish(mesh_object(f"glass.{name}", bm), "glass")
        obj.name = f"glass.{name}"


# Materials whose edges never chip (and whose shapes do not count as edges for others).
UNWORN = ("screen", "lamp", "glass", "decal", "glow_trim")


def collect_edges(min_angle=40.0):
    """The convex edges of every shape as it was built, before its bevel rounded it (where two
    faces meet at `min_angle` degrees or more), in the cockpit's space: the paint chips along
    them. (A cylinder's sides and a sphere's facets are not edges; their rims are.)"""
    EDGES.clear()
    cos_min = math.cos(math.radians(min_angle))
    for o in bpy.context.scene.objects:
        if o.type != 'MESH' or not o.data.materials or o.data.materials[0].name.split(".")[0] in UNWORN:
            continue
        mw = o.matrix_world
        nm = mw.to_3x3().inverted_safe().transposed()
        bm = bmesh.new()
        bm.from_mesh(o.data)
        bm.normal_update()
        for e in bm.edges:
            if len(e.link_faces) != 2:
                continue
            f1, f2 = e.link_faces
            if f1.normal.dot(f2.normal) > cos_min:
                continue
            # Convex: each face lies behind the other's plane.
            if (f2.calc_center_median() - e.verts[0].co).dot(f1.normal) > 1e-7:
                continue
            EDGES.append((mw @ e.verts[0].co, mw @ e.verts[1].co, (nm @ f1.normal).normalized(), (nm @ f2.normal).normalized()))
        bm.free()


def apply_modifiers():
    for o in list(bpy.context.scene.objects):
        if o.type != 'MESH':
            continue
        bpy.context.view_layer.objects.active = o
        for m in list(o.modifiers):
            bpy.ops.object.modifier_apply(modifier=m.name)


def merge_static():
    """Everything that does not move becomes one object per material (fewer draw parts)."""
    by_mat = {}
    for o in list(bpy.context.scene.objects):
        if o.type != 'MESH' or o.name.startswith("anim."):
            continue
        by_mat.setdefault(o.data.materials[0].name, []).append(o)
    for mat, objs in by_mat.items():
        if len(objs) > 1:
            join(objs, f"static.{mat}")
        else:
            objs[0].name = f"static.{mat}"


def edge_tree():
    """The edges (see `collect_edges`) as points every 3 mm, each with its edge's index: for
    finding those near a point."""
    points = []
    for i, (a, b, _, _) in enumerate(EDGES):
        steps = max(1, int((b - a).length / 0.003))
        points += [(a.lerp(b, s / steps), i) for s in range(steps + 1)]
    kd = KDTree(max(len(points), 1))
    for co, i in points:
        kd.insert(co, i)
    kd.balance()
    return kd, bool(points)


def subdivide_for_bake(max_edge=0.06):
    """Vertex occlusion and masks need vertices: long edges are cut; and a face whose corners all
    lie on edges (a narrow bar's, a small plate's) gets one in its middle, so that the paint
    chips along its borders and not right across it (the masks go linearly between vertices)."""
    kd, any_edges = edge_tree()
    for o in bpy.context.scene.objects:
        if o.type != 'MESH':
            continue
        mw = o.matrix_world
        bm = bmesh.new()
        bm.from_mesh(o.data)
        for _ in range(4):
            long = [e for e in bm.edges if e.calc_length() > max_edge]
            if not long:
                break
            bmesh.ops.subdivide_edges(bm, edges=long, cuts=1, use_grid_fill=True)
        if any_edges:
            # (Its corners within a bevel's width of an edge, its middle further in.)
            between = []
            for f in bm.faces:
                corners = max(kd.find(mw @ v.co)[2] for v in f.verts)
                if corners < 0.005 and kd.find(mw @ f.calc_center_median())[2] > corners + 0.002:
                    between.append(f)
            if between:
                bmesh.ops.poke(bm, faces=between)
        bmesh.ops.triangulate(bm, faces=bm.faces)
        bm.to_mesh(o.data)
        bm.free()


# How far from an edge the mask reaches (metres: the green channel's 1), and how near a face's
# plane (or within a bevel) a point must lie to be on that edge's faces.
EDGE_REACH = 0.03
ON_FACE = 0.0018
IN_BEVEL = 0.014


def bake_masks(samples=64, bake=True):
    """Bakes occlusion twice into each mesh's corners (within 3.5 cm: seams and crevices; within
    35 cm: the cabin's corners), then writes the masks (see the module's notes) into the colour
    the exporter writes, `ao`."""
    scene = bpy.context.scene
    meshes = [o for o in scene.objects if o.type == 'MESH']
    for o in meshes:
        attrs = o.data.color_attributes
        for name in ("contact", "ao"):
            if name not in attrs:
                attrs.new(name=name, type='FLOAT_COLOR', domain='CORNER')
    if bake:
        scene.render.engine = 'CYCLES'
        scene.cycles.samples = samples
        try:
            scene.cycles.device = 'GPU'
        except Exception:
            pass
        scene.world = scene.world or bpy.data.worlds.new("world")
        scene.render.bake.target = 'VERTEX_COLORS'
        bpy.ops.object.select_all(action='DESELECT')
        for o in meshes:
            o.select_set(True)
        bpy.context.view_layer.objects.active = meshes[0]
        for name, distance in (("contact", 0.035), ("ao", 0.35)):
            for o in meshes:
                o.data.color_attributes.active_color = o.data.color_attributes[name]
            scene.world.light_settings.distance = distance
            bpy.ops.object.bake(type='AO')
    write_masks(meshes, baked=bake)


def segment_distance(p, a, b):
    ab = b - a
    t = max(0.0, min(1.0, (p - a).dot(ab) / max(ab.length_squared, 1e-12)))
    return (p - (a + ab * t)).length


def write_masks(meshes, baked=True):
    kd, _ = edge_tree()
    for o in meshes:
        me = o.data
        mw = o.matrix_world
        nm = mw.to_3x3().inverted_safe().transposed()
        count = len(me.vertices)
        co = np.empty(count * 3)
        me.vertices.foreach_get("co", co)
        nrm = np.empty(count * 3)
        me.vertex_normals.foreach_get("vector", nrm)
        unworn = me.materials[0].name.split(".")[0] in ("screen", "lamp", "glass", "glow_trim")
        edge = np.ones(count)
        hands = np.zeros(count)
        for v in range(count):
            p = mw @ Vector(co[v * 3:v * 3 + 3])
            n = (nm @ Vector(nrm[v * 3:v * 3 + 3])).normalized()
            if not unworn:
                best = EDGE_REACH
                seen = set()
                for _, i, _ in kd.find_range(p, EDGE_REACH):
                    if i in seen:
                        continue
                    seen.add(i)
                    a, b, n1, n2 = EDGES[i]
                    s1, s2 = (p - a).dot(n1), (p - a).dot(n2)
                    on1 = abs(s1) < ON_FACE and s2 < ON_FACE
                    on2 = abs(s2) < ON_FACE and s1 < ON_FACE
                    bevel = -IN_BEVEL < s1 < ON_FACE and -IN_BEVEL < s2 < ON_FACE
                    if not (on1 or on2 or bevel) or (n.dot(n1) < 0.2 and n.dot(n2) < 0.2):
                        continue
                    best = min(best, segment_distance(p, a, b))
                edge[v] = best / EDGE_REACH
            h = 0.0
            for a, b, radius in HANDS:
                d = segment_distance(p, Vector(a), Vector(b))
                if d < radius:
                    t = max(0.0, min(1.0, (d - radius * 0.35) / (radius * 0.65)))
                    h = max(h, 1.0 - t * t * (3 - 2 * t))
            hands[v] = h
        loops = len(me.loops)
        vi = np.empty(loops, dtype=np.int64)
        me.loops.foreach_get("vertex_index", vi)
        out = np.ones(loops * 4)
        if baked:
            ao = np.empty(loops * 4)
            me.color_attributes["ao"].data.foreach_get("color", ao)
            contact = np.empty(loops * 4)
            me.color_attributes["contact"].data.foreach_get("color", contact)
            out[0::4] = ao[0::4]
            out[2::4] = contact[0::4]
        out[1::4] = edge[vi]
        out[3::4] = hands[vi]
        attrs = me.color_attributes
        attrs["ao"].data.foreach_set("color", out)
        attrs.remove(attrs["contact"])
        # (The exporter writes the colour that renders: removing one leaves none chosen.)
        attrs.active_color_name = "ao"
        attrs.default_color_name = "ao"


def export(path):
    bpy.ops.export_scene.gltf(filepath=path, export_format='GLB', export_yup=True, export_apply=True, export_normals=True,
                              export_materials='EXPORT', export_vertex_color='ACTIVE', export_all_vertex_colors=False,
                              export_texcoords=True, export_extras=True, export_cameras=False, export_lights=False)
    quantize(path)


def quantize(path):
    """Packs a GLB smaller (as `gltfpack` would, which is not at hand): positions as 16-bit
    integers on a grid each mesh's own (one step every way, so that its normals stay true: its
    node scales and places them, or a child node of a moving part's does), normals as 16-bit
    integers, the masks as bytes; UVs as they are, indices 16-bit where they fit. Every accessor
    gets a view of its own, 4-byte aligned."""
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
            if "indices" in prim:
                roles[prim["indices"]] = "INDICES"
    sizes = {5120: 1, 5121: 1, 5122: 2, 5123: 2, 5125: 4, 5126: 4}
    widths = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}

    def floats(a, width):
        view = doc["bufferViews"][a["bufferView"]]
        stride = view.get("byteStride", width * 4)
        start = view.get("byteOffset", 0) + a.get("byteOffset", 0)
        return np.frombuffer(b"".join(old[start + k * stride:start + k * stride + width * 4] for k in range(a["count"])), dtype="<f4").reshape(a["count"], width).astype(np.float64)
    # Each mesh's grid: its lowest corner and its step (the same every way), and the node that
    # places it.
    grids = {}
    for m, mesh in enumerate(doc["meshes"]):
        pts = np.vstack([floats(doc["accessors"][p["attributes"]["POSITION"]], 3) for p in mesh["primitives"]])
        lo, hi = pts.min(axis=0), pts.max(axis=0)
        grids[m] = (lo, max(float((hi - lo).max()), 1e-6) / 65535.0)
    for node in list(doc["nodes"]):
        if "mesh" not in node:
            continue
        lo, step = grids[node["mesh"]]
        place = {"translation": [float(v) for v in lo], "scale": [step] * 3}
        if any(k in node for k in ("translation", "rotation", "scale", "matrix")):
            # (A moving part keeps its own transform: the grid goes on a node under it, named
            # not to be taken for a part of its own.)
            doc["nodes"].append({"name": "grid." + node.get("name", ""), "mesh": node.pop("mesh"), **place})
            node.setdefault("children", []).append(len(doc["nodes"]) - 1)
        else:
            node.update(place)
    position_grid = {}
    for m, mesh in enumerate(doc["meshes"]):
        for p in mesh["primitives"]:
            position_grid[p["attributes"]["POSITION"]] = grids[m]
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
        byte_stride = None
        if role == "POSITION" and a["componentType"] == 5126:
            lo, step = position_grid[i]
            q = np.clip(np.round((floats(a, 3) - lo) / step), 0, 65535).astype("<u2")
            body = np.concatenate([q, np.zeros((count, 1), dtype="<u2")], axis=1).tobytes()
            a.update(componentType=5123, normalized=False, min=[int(v) for v in q.min(axis=0)], max=[int(v) for v in q.max(axis=0)])
            byte_stride = 8
        elif a["componentType"] == 5126 and (role == "NORMAL" or role.startswith("COLOR_")):
            values = np.frombuffer(b"".join(old[start + k * stride:start + k * stride + width * 4] for k in range(count)), dtype="<f4").reshape(count, width)
            if role == "NORMAL":
                q = np.clip(np.round(values * 32767), -32767, 32767).astype("<i2")
                body = np.concatenate([q, np.zeros((count, 1), dtype="<i2")], axis=1).tobytes()
                a.update(componentType=5122, normalized=True)
                byte_stride = 8
            else:
                # (A 255th of 3 cm is a tenth of a millimetre: as fine as the masks need.)
                body = np.clip(np.round(values * 255), 0, 255).astype("<u1").tobytes()
                a.update(componentType=5121, normalized=True)
        elif role == "INDICES" and a["componentType"] == 5125 and a.get("max", [70000])[0] < 65535:
            values = np.frombuffer(old[start:start + count * 4], dtype="<u4")
            body = values.astype("<u2").tobytes()
            a.update(componentType=5123)
        else:
            body = old[start:start + count * stride] if stride == width * size else b"".join(old[start + k * stride:start + k * stride + width * size] for k in range(count))
        a.pop("byteOffset", None)
        while len(out) % 4:
            out.append(0)
        v = {"buffer": 0, "byteOffset": len(out), "byteLength": len(body)}
        if byte_stride:
            v["byteStride"] = byte_stride
        if "target" in view:
            v["target"] = view["target"]
        a["bufferView"] = len(views)
        views.append(v)
        out += body
    while len(out) % 4:
        out.append(0)
    doc["bufferViews"] = views
    doc["buffers"] = [{"byteLength": len(out)}]
    doc["extensionsUsed"] = sorted(set(doc.get("extensionsUsed", [])) | {"KHR_mesh_quantization"})
    doc["extensionsRequired"] = sorted(set(doc.get("extensionsRequired", [])) | {"KHR_mesh_quantization"})
    text = json.dumps(doc, separators=(",", ":")).encode()
    text += b" " * ((4 - len(text) % 4) % 4)
    total = 12 + 8 + len(text) + 8 + len(out)
    with open(path, "wb") as f:
        f.write(struct.pack("<III", 0x46546C67, 2, total))
        f.write(struct.pack("<II", len(text), 0x4E4F534A) + text)
        f.write(struct.pack("<II", len(out), 0x004E4942) + bytes(out))


def preview(path, fov_deg=64.0, aspect=16 / 9, decals=None):
    """A render from the eye, the cockpit lit by a dim interior and a low sun through the port
    (the paint as built: its wear and grime are the game's)."""
    scene = bpy.context.scene
    if decals and os.path.exists(decals) and "decal" in bpy.data.materials:
        m = bpy.data.materials["decal"]
        nodes, links = m.node_tree.nodes, m.node_tree.links
        bsdf = next(n for n in nodes if n.type == 'BSDF_PRINCIPLED')
        tex = nodes.new("ShaderNodeTexImage")
        tex.image = bpy.data.images.load(decals)
        links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
        links.new(tex.outputs["Alpha"], bsdf.inputs["Alpha"])
    cam_data = bpy.data.cameras.new("eye")
    cam_data.sensor_fit = 'VERTICAL'
    cam_data.angle_y = math.radians(fov_deg)
    cam_data.clip_start = 0.01
    cam = bpy.data.objects.new("eye", cam_data)
    scene.collection.objects.link(cam)
    cam.rotation_euler = (math.pi / 2, 0, 0)
    scene.camera = cam
    sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", 'SUN'))
    sun.data.energy = 3.5
    sun.rotation_euler = (math.radians(55), 0, math.radians(20))
    scene.collection.objects.link(sun)
    fill = bpy.data.objects.new("fill", bpy.data.lights.new("fill", 'POINT'))
    fill.data.energy = 6.0
    fill.location = (0, 0.1, 0.35)
    scene.collection.objects.link(fill)
    scene.world = scene.world or bpy.data.worlds.new("world")
    scene.world.color = (0.45, 0.55, 0.7)
    scene.render.engine = 'BLENDER_EEVEE' if 'BLENDER_EEVEE' in [e.identifier for e in bpy.types.RenderSettings.bl_rna.properties['engine'].enum_items] else 'CYCLES'
    scene.render.resolution_x = 1280
    scene.render.resolution_y = int(1280 / aspect)
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--only")
    ap.add_argument("--preview")
    ap.add_argument("--no-bake", action="store_true")
    ap.add_argument("--textures", action="store_true", help="draw the shared textures (wear.png, decals.png) and stop")
    args = ap.parse_args(argv)
    os.makedirs(args.out, exist_ok=True)
    if args.textures:
        print("cockpit textures:", art.draw_decals(os.path.join(args.out, "decals.png")), art.bake_wear(os.path.join(args.out, "wear.png")))
        return
    for name, build in COCKPITS.items():
        if args.only and name not in args.only.split(","):
            continue
        reset()
        build()
        collect_edges()
        apply_modifiers()
        merge_static()
        subdivide_for_bake()
        bake_masks(bake=not args.no_bake)
        glaze()
        path = os.path.join(args.out, f"{name}.glb")
        export(path)
        print(f"cockpit {name}: {sum(len(o.data.polygons) for o in bpy.context.scene.objects if o.type == 'MESH')} faces, {len(EDGES)} edges, {os.path.getsize(path)} bytes -> {path}")
        if args.preview:
            os.makedirs(args.preview, exist_ok=True)
            preview(os.path.join(args.preview, f"{name}.png"), fov_deg=FOV[name], decals=os.path.join(args.out, "decals.png"))


if __name__ == "__main__":
    main()
