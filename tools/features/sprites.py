"""Draws the paintings of TA Reborn's own festive features (original art) from their models
(festive.py), as TA painted its features: in its oblique view (looking north, down at 63.4
degrees, so a point (x, y, z) shows at pixel (x, z - y/2)), lit from the south west, a shadow
for each on the ground. Writes the frames as RGBA PNGs, the anchor (the model's foot) at each
one's `origin`, a decorated fir's frames as its lights twinkle, `manifest.json` for
`ta-tools gaf-pack`, and the features' TDF (festive.py's FEATURES).

    "C:/Program Files/Blender Foundation/Blender 5.2/blender.exe" -b -P tools/features/sprites.py -- --out scratch/festive [--only snowpine_a,cottage_a] [--preview]
    target/release/ta-tools gaf-pack scratch/festive/manifest.json assets/reborn/anims/festive.gaf
    copy scratch/festive/festive.tdf to assets/reborn/features/festive/festive.tdf

`--preview` renders each model from a unit's height too, beside its painting, to look at.
"""
import argparse
import json
import math
import os
import struct
import sys
import zlib

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import build  # noqa: E402  (its catalogue ends with festive.py's models)
import festive  # noqa: E402

# Render pixels for a unit of the world, across.
SS = 4
# TA's view: down at atan(2) (63.4 degrees) from the south; a unit south is this much of the
# screen (heights half as much).
ELEV = math.atan(2.0)
SOUTH = math.sin(ELEV)
# The light: from the south west, this high.
SUN_AZIMUTH, SUN_ELEVATION = math.radians(225.0), math.radians(50.0)
# The festive layer's lights glow this strongly in the painting (their vertex alpha times this).
EMIT = 2.6
# The snow's texture spans this many units (as the game lays it on, foliage.wgsl's SNOW_TILE),
# and the snow lying on leaves (linear, its SNOW_ON_LEAVES).
SNOW_TILE = 40.0
SNOW_ON_LEAVES = (0.78, 0.82, 0.89)
FRAMES_PER_SECOND = 10.0


def twinkle_level(phase, t, period):
    """How bright a bulb twinkling at `phase` (0..1, from its COLOR_0) is at `t` seconds, its
    cycle `period` long: once or twice a cycle it fades down and up again (foliage.wgsl's
    `twinkle` does the same)."""
    rate = 1.0 + float(int(phase * 1000.0) % 2)
    s = 0.5 + 0.5 * math.cos(TAU * (t / period * rate + phase))
    k = min(max((s - 0.25) / 0.5, 0.0), 1.0)
    return 0.22 + 0.78 * k * k * (3.0 - 2.0 * k)


TAU = 2.0 * math.pi


def write_png(path, rgba):
    """An RGBA8 PNG (rows top first)."""
    h, w, _ = rgba.shape
    raw = b"".join(b"\x00" + rgba[y].astype(np.uint8).tobytes() for y in range(h))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(raw, 9)))
        f.write(chunk(b"IEND", b""))


def resample(img, w, h):
    """Area-averages an image (rows top first, premultiplied) down to `w` x `h`."""
    sh, sw = img.shape[:2]

    def weights(n_src, n_dst):
        m = np.zeros((n_dst, n_src))
        f = n_src / n_dst
        for j in range(n_dst):
            a, b = j * f, (j + 1) * f
            for i in range(int(math.floor(a)), min(int(math.ceil(b)), n_src)):
                m[j, i] = max(0.0, min(b, i + 1) - max(a, i))
            m[j] /= max(m[j].sum(), 1e-9)
        return m

    wy, wx = weights(sh, h), weights(sw, w)
    return np.einsum("ij,jkc,lk->ilc", wy, img, wx)


def gltf_to_blender(p):
    return np.stack([p[:, 0], -p[:, 2], p[:, 1]], axis=1)


def painted(pos):
    """Pixels (x, z - y/2) of glTF points."""
    return np.stack([pos[:, 0], pos[:, 2] - pos[:, 1] * 0.5], axis=1)


def setup(textures):
    import bpy

    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 96
    scene.cycles.use_denoising = True
    scene.cycles.seed = 7
    scene.cycles.device = "GPU"
    prefs = bpy.context.preferences.addons["cycles"].preferences
    try:
        prefs.compute_device_type = "OPTIX"
        prefs.get_devices()
        for d in prefs.devices:
            d.use = True
    except Exception:
        pass
    scene.render.film_transparent = True
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.look = "None"
    scene.render.image_settings.file_format = "OPEN_EXR"
    scene.render.image_settings.color_depth = "32"
    world = bpy.data.worlds.new("sky")
    world.use_nodes = True
    bg = next(n for n in world.node_tree.nodes if n.type == "BACKGROUND")
    bg.inputs[0].default_value = (0.66, 0.74, 0.88, 1.0)
    bg.inputs[1].default_value = 0.55
    scene.world = world
    sun = bpy.data.lights.new("sun", "SUN")
    sun.energy = 3.4
    sun.angle = math.radians(1.5)
    so = bpy.data.objects.new("sun", sun)
    import mathutils

    toward = mathutils.Vector((math.sin(SUN_AZIMUTH) * math.cos(SUN_ELEVATION), math.cos(SUN_AZIMUTH) * math.cos(SUN_ELEVATION), math.sin(SUN_ELEVATION)))
    so.rotation_euler = (-toward).to_track_quat("-Z", "Y").to_euler()
    scene.collection.objects.link(so)
    cam = bpy.data.cameras.new("cam")
    cam.type = "ORTHO"
    cam.sensor_fit = "HORIZONTAL"
    co = bpy.data.objects.new("cam", cam)
    co.rotation_euler = (math.pi / 2 - ELEV, 0.0, 0.0)
    scene.collection.objects.link(co)
    scene.camera = co
    return scene, co, materials(textures)


def materials(textures):
    import bpy

    def image(nt, name):
        node = nt.nodes.new("ShaderNodeTexImage")
        node.image = bpy.data.images.load(os.path.join(textures, f"{name}.png"), check_existing=True)
        node.interpolation = "Cubic"
        return node

    def vertex(nt):
        attr = nt.nodes.new("ShaderNodeVertexColor")
        attr.layer_name = "Col"
        sep = nt.nodes.new("ShaderNodeSeparateColor")
        nt.links.new(attr.outputs["Color"], sep.inputs["Color"])
        return attr, sep

    def times(nt, a, b):
        mix = nt.nodes.new("ShaderNodeMix")
        mix.data_type = "RGBA"
        mix.blend_type = "MULTIPLY"
        mix.inputs["Factor"].default_value = 1.0
        nt.links.new(a, mix.inputs[6])
        nt.links.new(b, mix.inputs[7])
        return mix.outputs[2]

    def base(name):
        mat = bpy.data.materials.new(name)
        mat.use_nodes = True
        nt = mat.node_tree
        bsdf = next(n for n in nt.nodes if n.type == "BSDF_PRINCIPLED")
        return mat, nt, bsdf

    def box(nt, img, tile):
        coord = nt.nodes.new("ShaderNodeTexCoord")
        mapping = nt.nodes.new("ShaderNodeMapping")
        mapping.inputs["Scale"].default_value = (1 / tile, 1 / tile, 1 / tile)
        nt.links.new(coord.outputs["Object"], mapping.inputs["Vector"])
        nt.links.new(mapping.outputs["Vector"], img.inputs["Vector"])
        img.projection = "BOX"
        img.projection_blend = 0.35

    out = {}
    for layer in ("needles", "bark", "rock"):
        mat, nt, bsdf = base(layer)
        img = image(nt, layer)
        if layer == "rock":
            box(nt, img, 44.0)
        _, sep = vertex(nt)
        nt.links.new(times(nt, img.outputs["Color"], sep.outputs["Red"]), bsdf.inputs["Base Color"])
        bsdf.inputs["Roughness"].default_value = 0.75
        if layer == "needles":
            gt = nt.nodes.new("ShaderNodeMath")
            gt.operation = "GREATER_THAN"
            gt.inputs[1].default_value = 0.5
            nt.links.new(img.outputs["Alpha"], gt.inputs[0])
            nt.links.new(gt.outputs[0], bsdf.inputs["Alpha"])
        out[layer] = mat
    # Snow: soft, a little translucent, sparkling.
    mat, nt, bsdf = base("snow")
    img = image(nt, "snow")
    box(nt, img, SNOW_TILE)
    _, sep = vertex(nt)
    nt.links.new(times(nt, img.outputs["Color"], sep.outputs["Red"]), bsdf.inputs["Base Color"])
    bsdf.inputs["Roughness"].default_value = 0.55
    bsdf.inputs["Subsurface Weight"].default_value = 0.08
    out["snow"] = mat
    # The festive atlas: its colours, shining where COLOR_0's blue says (not on the lights,
    # whose blue is their phase), glowing by COLOR_0's alpha.
    mat, nt, bsdf = base("festive")
    img = image(nt, "festive")
    img.interpolation = "Linear"
    _, sep = vertex(nt)
    nt.links.new(times(nt, img.outputs["Color"], sep.outputs["Red"]), bsdf.inputs["Base Color"])
    nolight = nt.nodes.new("ShaderNodeMath")
    nolight.operation = "LESS_THAN"
    nolight.inputs[1].default_value = 0.001
    va = nt.nodes.new("ShaderNodeAttribute")
    va.attribute_name = "Col"
    nt.links.new(va.outputs["Alpha"], nolight.inputs[0])
    gloss = nt.nodes.new("ShaderNodeMath")
    gloss.operation = "MULTIPLY"
    nt.links.new(sep.outputs["Blue"], gloss.inputs[0])
    nt.links.new(nolight.outputs[0], gloss.inputs[1])
    rough = nt.nodes.new("ShaderNodeMapRange")
    rough.inputs["To Min"].default_value = 0.8
    rough.inputs["To Max"].default_value = 0.22
    nt.links.new(gloss.outputs[0], rough.inputs["Value"])
    nt.links.new(rough.outputs["Result"], bsdf.inputs["Roughness"])
    strength = nt.nodes.new("ShaderNodeMath")
    strength.operation = "MULTIPLY"
    strength.inputs[1].default_value = EMIT
    nt.links.new(va.outputs["Alpha"], strength.inputs[0])
    nt.links.new(img.outputs["Color"], bsdf.inputs["Emission Color"])
    nt.links.new(strength.outputs[0], bsdf.inputs["Emission Strength"])
    out["festive"] = mat
    return out


def snowy_needles(textures, amount):
    """The needles with snow lying on them, as the game's shader lays it (foliage.wgsl's
    `snowed`): `amount` of it where a spray's face is turned up to the sky on the side seen, in
    clumps."""
    import bpy

    mat = bpy.data.materials.new(f"needles_snow_{amount:.2f}")
    mat.use_nodes = True
    nt = mat.node_tree
    bsdf = next(n for n in nt.nodes if n.type == "BSDF_PRINCIPLED")
    img = nt.nodes.new("ShaderNodeTexImage")
    img.image = bpy.data.images.load(os.path.join(textures, "needles.png"), check_existing=True)
    img.interpolation = "Cubic"
    attr = nt.nodes.new("ShaderNodeVertexColor")
    attr.layer_name = "Col"
    sep = nt.nodes.new("ShaderNodeSeparateColor")
    nt.links.new(attr.outputs["Color"], sep.inputs["Color"])
    shaded = nt.nodes.new("ShaderNodeMix")
    shaded.data_type = "RGBA"
    shaded.blend_type = "MULTIPLY"
    shaded.inputs["Factor"].default_value = 1.0
    nt.links.new(img.outputs["Color"], shaded.inputs[6])
    nt.links.new(sep.outputs["Red"], shaded.inputs[7])
    geom = nt.nodes.new("ShaderNodeNewGeometry")
    xyz = nt.nodes.new("ShaderNodeSeparateXYZ")
    nt.links.new(geom.outputs["True Normal"], xyz.inputs["Vector"])
    # (Turned towards the eye: flipped where its back is seen.)
    flip = nt.nodes.new("ShaderNodeMath")
    flip.operation = "MULTIPLY_ADD"
    flip.inputs[1].default_value = -2.0
    flip.inputs[2].default_value = 1.0
    nt.links.new(geom.outputs["Backfacing"], flip.inputs[0])
    up = nt.nodes.new("ShaderNodeMath")
    up.operation = "MULTIPLY"
    nt.links.new(xyz.outputs["Z"], up.inputs[0])
    nt.links.new(flip.outputs[0], up.inputs[1])
    facing = nt.nodes.new("ShaderNodeMapRange")
    facing.interpolation_type = "SMOOTHSTEP"
    facing.inputs["From Min"].default_value = 0.3
    facing.inputs["From Max"].default_value = 0.75
    nt.links.new(up.outputs[0], facing.inputs["Value"])
    coord = nt.nodes.new("ShaderNodeTexCoord")
    clumps = []
    for scale, w in ((0.42, 0.7), (1.3, 0.3)):
        noise = nt.nodes.new("ShaderNodeTexNoise")
        noise.inputs["Scale"].default_value = scale
        noise.inputs["Detail"].default_value = 0.0
        nt.links.new(coord.outputs["Object"], noise.inputs["Vector"])
        mul = nt.nodes.new("ShaderNodeMath")
        mul.operation = "MULTIPLY"
        mul.inputs[1].default_value = w
        nt.links.new(noise.outputs["Fac"], mul.inputs[0])
        clumps.append(mul)
    clump = nt.nodes.new("ShaderNodeMath")
    clump.operation = "ADD"
    nt.links.new(clumps[0].outputs[0], clump.inputs[0])
    nt.links.new(clumps[1].outputs[0], clump.inputs[1])
    lumps = nt.nodes.new("ShaderNodeMapRange")
    lumps.interpolation_type = "SMOOTHSTEP"
    lumps.inputs["From Min"].default_value = 0.62 - 0.45 * amount
    lumps.inputs["From Max"].default_value = 0.78 - 0.45 * amount
    nt.links.new(clump.outputs[0], lumps.inputs["Value"])
    k = nt.nodes.new("ShaderNodeMath")
    k.operation = "MULTIPLY"
    nt.links.new(facing.outputs["Result"], k.inputs[0])
    nt.links.new(lumps.outputs["Result"], k.inputs[1])
    ka = nt.nodes.new("ShaderNodeMath")
    ka.operation = "MULTIPLY"
    ka.inputs[1].default_value = amount
    ka.use_clamp = True
    nt.links.new(k.outputs[0], ka.inputs[0])
    snow = nt.nodes.new("ShaderNodeMix")
    snow.data_type = "RGBA"
    nt.links.new(ka.outputs[0], snow.inputs["Factor"])
    nt.links.new(shaded.outputs[2], snow.inputs[6])
    snow.inputs[7].default_value = (*SNOW_ON_LEAVES, 1.0)
    nt.links.new(snow.outputs[2], bsdf.inputs["Base Color"])
    bsdf.inputs["Roughness"].default_value = 0.75
    gt = nt.nodes.new("ShaderNodeMath")
    gt.operation = "GREATER_THAN"
    gt.inputs[1].default_value = 0.5
    nt.links.new(img.outputs["Alpha"], gt.inputs[0])
    nt.links.new(gt.outputs[0], bsdf.inputs["Alpha"])
    return mat


def ice(own):
    """Ice of the model's own colour (`own['leaf']`), glassy, a little of its light within."""
    import bpy

    mat = bpy.data.materials.new("crystal")
    mat.use_nodes = True
    nt = mat.node_tree
    bsdf = next(n for n in nt.nodes if n.type == "BSDF_PRINCIPLED")
    leaf = own.get("leaf", [0.3, 0.55, 0.73])
    light = own.get("light", [0.5, 0.8, 1.0])
    bsdf.inputs["Base Color"].default_value = (*[min(1.0, c * 1.25) for c in leaf], 1.0)
    bsdf.inputs["Roughness"].default_value = 0.08
    bsdf.inputs["Transmission Weight"].default_value = 0.3
    bsdf.inputs["Coat Weight"].default_value = 0.6
    bsdf.inputs["IOR"].default_value = 1.31
    va = nt.nodes.new("ShaderNodeAttribute")
    va.attribute_name = "Col"
    strength = nt.nodes.new("ShaderNodeMath")
    strength.operation = "MULTIPLY"
    strength.inputs[1].default_value = 0.6
    nt.links.new(va.outputs["Alpha"], strength.inputs[0])
    bsdf.inputs["Emission Color"].default_value = (*light, 1.0)
    nt.links.new(strength.outputs[0], bsdf.inputs["Emission Strength"])
    return mat


def objects(scene, model, mats, textures):
    """The model's parts as Blender objects; returns them with their COLOR_0 (to twinkle)."""
    import bpy

    made = []
    own = model.meta.get("own", {})
    crystal = ice(own) if "crystal" in model.parts else None
    needles = snowy_needles(textures, own["snow"]) if own.get("snow", 0.0) > 0.0 else mats.get("needles")
    for layer, part in model.parts.items():
        arr = part.arrays()
        if arr is None:
            continue
        pos, nrm, uv, col, idx = arr
        mesh = bpy.data.meshes.new(f"{model.name}.{layer}")
        mesh.from_pydata(gltf_to_blender(pos).tolist(), [], idx.tolist())
        uvl = mesh.uv_layers.new(name="UV")
        loops = idx.reshape(-1)
        uvl.data.foreach_set("uv", np.stack([uv[loops, 0], 1.0 - uv[loops, 1]], axis=1).reshape(-1))
        ca = mesh.color_attributes.new("Col", "FLOAT_COLOR", "POINT")
        ca.data.foreach_set("color", col.reshape(-1))
        mesh.normals_split_custom_set_from_vertices(gltf_to_blender(nrm).tolist())
        mesh.materials.append(crystal if layer == "crystal" else needles if layer == "needles" else mats[layer])
        obj = bpy.data.objects.new(mesh.name, mesh)
        scene.collection.objects.link(obj)
        made.append((obj, col.copy(), layer))
    return made


def render(scene, cam, x0, x1, y0, y1, path):
    """Renders the painted rectangle [x0, x1] x [y0, y1] (pixels: x, z - y/2) to `path` and
    returns it at a pixel a unit (RGBA 0..1 premultiplied, rows top first)."""
    import bpy

    w, h = int(round(x1 - x0)), int(round(y1 - y0))
    scene.render.resolution_x = w * SS
    scene.render.resolution_y = max(1, int(round(h * SOUTH * SS)))
    scene.render.resolution_percentage = 100
    cam.data.ortho_scale = w
    cx, cy = (x0 + x1) / 2.0, (y0 + y1) / 2.0
    up = np.array([0.0, math.sin(ELEV), math.cos(ELEV)])
    look = np.array([0.0, math.cos(ELEV), -math.sin(ELEV)])
    cam.location = tuple(np.array([cx, 0.0, 0.0]) + up * (-SOUTH * cy) - look * 900.0)
    cam.data.clip_end = 3000.0
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)
    img = bpy.data.images.load(path, check_existing=False)
    px = np.array(img.pixels[:], dtype=np.float32).reshape(img.size[1], img.size[0], 4)[::-1]
    bpy.data.images.remove(img)
    # (Standard view: the EXR holds scene light; to the display's sRGB, premultiplied.)
    rgb = np.clip(px[..., :3], 0.0, None)
    a = np.clip(px[..., 3:4], 0.0, 1.0)
    straight = np.where(a > 1e-6, rgb / np.maximum(a, 1e-6), 0.0)
    srgb = np.where(straight <= 0.0031308, straight * 12.92, 1.055 * np.power(np.clip(straight, 0, 1), 1 / 2.4) - 0.055)
    pre = np.concatenate([srgb * a, a], axis=2)
    return resample(pre, w, h)


def unpremultiply(img):
    a = img[..., 3:4]
    rgb = np.where(a > 1e-6, img[..., :3] / np.maximum(a, 1e-6), 0.0)
    return np.concatenate([np.clip(rgb, 0, 1), a], axis=2)


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else sys.argv[1:]
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--textures", default=os.path.join(HERE, "..", "..", "crates", "ta_render", "features"))
    ap.add_argument("--only")
    ap.add_argument("--preview", action="store_true")
    args = ap.parse_args(argv)
    import bpy

    # (Absolute: Blender reads a relative path against the .blend file's folder, not ours.)
    args.out = os.path.abspath(args.out)
    os.makedirs(args.out, exist_ok=True)
    builders = dict(festive.catalogue(build))
    scene, cam, mats = setup(os.path.abspath(args.textures))
    toward = np.array([math.sin(SUN_AZIMUTH), math.cos(SUN_AZIMUTH)]) * math.cos(SUN_ELEVATION)
    for name, model_name, world, description, (fx, fz), height, frames, extra in festive.FEATURES:
        if args.only and model_name not in args.only.split(",") and name not in args.only.split(","):
            continue
        model = builders[model_name](0)
        made = objects(scene, model, mats, os.path.abspath(args.textures))
        pos = np.concatenate([np.concatenate(p.pos) for p in model.parts.values() if p.pos])
        ink = painted(pos)
        pad = 3.0
        x0, x1 = math.floor(ink[:, 0].min() - pad), math.ceil(ink[:, 0].max() + pad)
        y0, y1 = math.floor(ink[:, 1].min() - pad), math.ceil(ink[:, 1].max() + pad)
        seq = name.lower()
        entry = {"name": seq, "frames": []}
        period = frames / FRAMES_PER_SECOND
        for k in range(frames):
            for obj, col, layer in made:
                if layer != "festive":
                    continue
                c = col.copy()
                if frames > 1:
                    lights = (c[:, 3] > 0.0) & (c[:, 2] > 0.0)
                    for i in np.nonzero(lights)[0]:
                        c[i, 3] = col[i, 3] * twinkle_level(col[i, 2], k / FRAMES_PER_SECOND, period)
                obj.data.color_attributes["Col"].data.foreach_set("color", c.reshape(-1))
                obj.data.update()
            img = unpremultiply(render(scene, cam, x0, x1, y0, y1, os.path.join(args.out, "_render.exr")))
            png = f"{seq}_{k:02d}.png"
            write_png(os.path.join(args.out, png), np.round(img * 255.0))
            entry["frames"].append({"png": png, "origin": [int(-x0), int(-y0)]})
        # The shadow: the model unseen, its shadow caught on the ground.
        for obj, _, _ in made:
            obj.visible_camera = False
        plane = bpy.data.meshes.new("ground")
        reach = height_of(pos) / math.tan(SUN_ELEVATION)
        gx0, gx1 = pos[:, 0].min() - 4, pos[:, 0].max() + reach * abs(toward[0]) / max(np.hypot(*toward), 1e-6) + 8
        gz0, gz1 = pos[:, 2].min() - reach * abs(toward[1]) / max(np.hypot(*toward), 1e-6) - 8, pos[:, 2].max() + 4
        plane.from_pydata([(gx0 - 50, -(gz1 + 50), 0.0), (gx1 + 50, -(gz1 + 50), 0.0), (gx1 + 50, -(gz0 - 50), 0.0), (gx0 - 50, -(gz0 - 50), 0.0)], [], [(0, 1, 2, 3)])
        ground = bpy.data.objects.new("ground", plane)
        ground.is_shadow_catcher = True
        scene.collection.objects.link(ground)
        shadow = render(scene, cam, math.floor(gx0), math.ceil(gx1), math.floor(gz0), math.ceil(gz1), os.path.join(args.out, "_render.exr"))
        a = shadow[..., 3]
        write_png(os.path.join(args.out, f"{seq}shad.png"), np.round(np.dstack([np.zeros(a.shape + (3,)), a[..., None]]) * 255.0))
        shadow_entry = {"name": f"{seq}shad", "shadow": True, "frames": [{"png": f"{seq}shad.png", "origin": [int(-math.floor(gx0)), int(-math.floor(gz0))]}]}
        # (Each feature's own entries beside its pictures: drawing some anew keeps the rest.)
        with open(os.path.join(args.out, f"{seq}.json"), "w", encoding="utf-8") as f:
            json.dump([entry, shadow_entry], f, indent=1)
        if args.preview:
            preview(scene, cam, model, os.path.join(args.out, f"{seq}_preview.png"))
        bpy.data.objects.remove(ground)
        for obj, _, _ in made:
            bpy.data.objects.remove(obj)
        print(f"{name}: {len(entry['frames'])} frames {x1 - x0}x{y1 - y0}, origin {-x0},{-y0}")
    # The manifest of every feature drawn so far, and the TDF of them all.
    manifest = {"entries": []}
    tdf = []
    for name, model_name, world, description, (fx, fz), height, frames, extra in festive.FEATURES:
        seq = name.lower()
        drawn = os.path.join(args.out, f"{seq}.json")
        if os.path.exists(drawn):
            with open(drawn, encoding="utf-8") as f:
                manifest["entries"] += json.load(f)
        else:
            print(f"{name}: not drawn yet")
        lines = [f"[{name}]", "\t{", f"\tworld={world};", f"\tdescription={description};", "\tcategory=festive;", f"\tfootprintx={fx};", f"\tfootprintz={fz};", f"\theight={height};", "\tblocking=1;", "\tfilename=festive;", f"\tseqname={seq};", f"\tseqnameshad={seq}shad;", "\tshadtrans=1;", "\tflamable=0;"]
        lines += [f"\t{k}={v};" for k, v in extra.items()]
        lines += [f"\trebornmodel={model_name};", "\t}"]
        tdf.append("\n".join(lines))
    with open(os.path.join(args.out, "manifest.json"), "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=1)
    header = "// TA Reborn's own festive features (original art), written by tools/features/sprites.py from\n// festive.py's FEATURES: their paintings anims/festive.gaf, their models in features.glb.\n"
    with open(os.path.join(args.out, "festive.tdf"), "w", encoding="utf-8", newline="\n") as f:
        f.write(header + "\n".join(tdf) + "\n")


def height_of(pos):
    return float(pos[:, 1].max())


def preview(scene, cam, model, path):
    """The model from a unit's height, lit as the painting is (to look at, not shipped)."""
    import bpy

    for obj in scene.objects:
        obj.visible_camera = True
    pos = np.concatenate([np.concatenate(p.pos) for p in model.parts.values() if p.pos])
    size = max(np.ptp(pos[:, 0]), np.ptp(pos[:, 1]), np.ptp(pos[:, 2]))
    cam.data.type = "PERSP"
    cam.data.lens = 50
    e = math.radians(14.0)
    d = size * 2.4
    cy = float(pos[:, 1].max()) * 0.45
    cam.location = (size * 0.3, -d * math.cos(e), cy + d * math.sin(e))
    cam.rotation_euler = (math.pi / 2 - e, 0.0, math.atan2(size * 0.3, d) * 0.6)
    scene.render.resolution_x, scene.render.resolution_y = 640, 640
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_depth = "8"
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)
    scene.render.image_settings.file_format = "OPEN_EXR"
    scene.render.image_settings.color_depth = "32"
    cam.data.type = "ORTHO"
    cam.rotation_euler = (math.pi / 2 - ELEV, 0.0, 0.0)


if __name__ == "__main__":
    main()
