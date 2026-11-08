"""Renders a quick look-dev preview of a built .blend (nothing is saved back):

    blender -b assets-src/blend/kit.blend --python tools/models/preview.py -- out.png [team]

Lays the meshes out in a row, gives `accent`/`glow` a team colour (the game does this with its
own shader), fades `glow` by its UV V like the game, and renders with Eevee. `flow` gets static
3 cm pulses every 0.1 m of UV U (checks the arc-length UVs).

Optional positional args after the team: spacing cols lens dist height. Environment:
PREVIEW_YAW (degrees each mesh turns about the vertical; 180 shows the -Z front towards the
camera), PREVIEW_ONLY (comma-separated mesh names to keep), PREVIEW_TARGET (height in metres the camera
looks at, default 1.0), PREVIEW_SUN (the sun's strength, default 1.5), PREVIEW_SIZE (WxH
pixels, default 1600x900). Avatar heads get copies of the shared eyeball and eyelids in their
eye frames (the `head_*_eye_*` empties), the lids open as the empties say or as
PREVIEW_LIDS=top,bottom (degrees; -5,-5 is shut).
"""

import os

import math
import sys

import bpy

args = sys.argv[sys.argv.index("--") + 1:]
out = args[0]
team = (0.1, 0.8, 1.0) if len(args) < 2 or args[1] == "blue" else (1.0, 0.25, 0.1)
scene = bpy.context.scene

obs = [o for o in scene.objects if o.type == "MESH"]
only = [n for n in os.environ.get("PREVIEW_ONLY", "").split(",") if n]

# Avatars: put a copy of the shared eyeball and eyelids in each head's eye frames (the empties
# `head_*_eye_*`, carrying the lids' rest angles), as the game does.
eye_meshes = {o.name: o for o in scene.objects if o.name == "eyelid" or o.name.startswith("eyeball")}
if "eyeball" in eye_meshes and "eyelid" in eye_meshes:
    M = __import__("mathutils").Matrix
    for e in [o for o in scene.objects if o.type == "EMPTY" and "_eye_" in o.name]:
        head = bpy.data.objects.get(e.name.split("_eye_")[0])
        if head is None:
            continue
        lids = os.environ.get("PREVIEW_LIDS")
        lt, lb = ((float(v) for v in lids.split(",")) if lids else
                  (float(e.get("lid_top", 55)), float(e.get("lid_bot", 55))))
        top = M.Rotation(math.radians(lt), 4, "X")
        # (The game's turn about its Z is about Blender's Y.)
        bot = M.Rotation(math.pi, 4, "Y") @ M.Rotation(math.radians(lb), 4, "X")
        ball = "eyeball_" + head.name[len("head_"):]
        ball = ball if ball in eye_meshes else "eyeball"
        for mesh, local in ((ball, M.Identity(4)), ("eyelid", top), ("eyelid", bot)):
            c = eye_meshes[mesh].copy()
            c.name = "%s.%s" % (e.name, mesh)
            scene.collection.objects.link(c)
            c.parent = head
            c.matrix_local = e.matrix_basis @ local
    for o in eye_meshes.values():
        bpy.data.objects.remove(o)
    obs = [o for o in scene.objects if o.type == "MESH"]
    if only:
        only += [o.name for o in obs if o.parent is not None and o.parent.name in only]
if only:
    for o in [o for o in obs if o.name not in only]:
        bpy.data.objects.remove(o)
    obs = [o for o in scene.objects if o.type == "MESH"]
for o in obs:
    if o.parent is None:
        o.rotation_euler.z = math.radians(float(os.environ.get("PREVIEW_YAW", "0")))
bpy.context.view_layer.update()
target_z = float(os.environ.get("PREVIEW_TARGET", "1.0"))
x = 0.0
spacing = float(args[2]) if len(args) > 2 else 0.6
cols = int(args[3]) if len(args) > 3 else 8
row_z, row_w, rowdepth = 0.0, 0, 0.0
xmax = 0.0
for i, o in enumerate(sorted(obs, key=lambda o: o.name)):
    if o.parent is not None:
        continue
    bb = [o.matrix_world @ __import__("mathutils").Vector(c) for c in o.bound_box]
    w = max(v.x for v in bb) - min(v.x for v in bb)
    d = max(v.y for v in bb) - min(v.y for v in bb)
    if row_w == cols:
        row_z += rowdepth + spacing * 2
        x, row_w, rowdepth = 0.0, 0, 0.0
    o.location.x = x - min(v.x for v in bb)
    o.location.y = -row_z
    x += w + spacing
    xmax = max(xmax, x)
    rowdepth = max(rowdepth, d)
    row_w += 1


def emissive(m, col, strength, fade=False, power=0.6, pulses=False):
    nt = m.node_tree
    nt.nodes.clear()
    outn = nt.nodes.new("ShaderNodeOutputMaterial")
    em = nt.nodes.new("ShaderNodeEmission")
    em.inputs["Color"].default_value = (*col, 1)
    em.inputs["Strength"].default_value = strength
    if pulses:
        uv = nt.nodes.new("ShaderNodeUVMap")
        sep = nt.nodes.new("ShaderNodeSeparateXYZ")
        fr = nt.nodes.new("ShaderNodeMath")
        fr.operation = "PINGPONG"
        fr.inputs[1].default_value = 0.05
        gt = nt.nodes.new("ShaderNodeMath")
        gt.operation = "GREATER_THAN"
        gt.inputs[1].default_value = 0.035
        sc = nt.nodes.new("ShaderNodeMath")
        sc.operation = "MULTIPLY_ADD"
        sc.inputs[1].default_value = strength
        sc.inputs[2].default_value = 0.08
        nt.links.new(uv.outputs[0], sep.inputs[0])
        nt.links.new(sep.outputs[0], fr.inputs[0])
        nt.links.new(fr.outputs[0], gt.inputs[0])
        nt.links.new(gt.outputs[0], sc.inputs[0])
        nt.links.new(sc.outputs[0], em.inputs["Strength"])
    if not fade:
        nt.links.new(em.outputs[0], outn.inputs[0])
        return
    uv = nt.nodes.new("ShaderNodeUVMap")
    sep = nt.nodes.new("ShaderNodeSeparateXYZ")
    tr = nt.nodes.new("ShaderNodeBsdfTransparent")
    mix = nt.nodes.new("ShaderNodeMixShader")
    pw = nt.nodes.new("ShaderNodeMath")
    pw.operation = "POWER"
    pw.inputs[1].default_value = power
    nt.links.new(uv.outputs[0], sep.inputs[0])
    inv = nt.nodes.new("ShaderNodeMath")  # Blender V = 1 - glTF V
    inv.operation = "SUBTRACT"
    inv.inputs[0].default_value = 1.0
    nt.links.new(sep.outputs[1], inv.inputs[1])
    nt.links.new(inv.outputs[0], pw.inputs[0])
    nt.links.new(pw.outputs[0], mix.inputs[0])
    nt.links.new(em.outputs[0], mix.inputs[1])
    nt.links.new(tr.outputs[0], mix.inputs[2])
    nt.links.new(mix.outputs[0], outn.inputs[0])
    try:
        m.surface_render_method = "BLENDED"
    except Exception:
        pass


hitcols = {"hit_1": (1, 0.1, 0.1), "hit_2": (1, 0.8, 0.1), "hit_3": (0.1, 1, 0.2)}
for m in bpy.data.materials:
    if m.name == "accent":
        emissive(m, team, 6.0)
    elif m.name == "glow":
        emissive(m, team, 1.5, fade=True)
    elif m.name == "eye_glow":
        emissive(m, team, 3.0, fade=True, power=0.5)  # alpha (1 - v)^2
    elif m.name == "eyes":
        emissive(m, team, 5.0)
    elif m.name == "flow":
        emissive(m, team, 4.0, pulses=True)
    elif m.name == "glow_white":
        emissive(m, (1, 1, 1), 5.0)
    elif m.name == "halo_white":
        emissive(m, (1, 1, 1), 0.8, fade=True)
    elif m.name in hitcols:
        emissive(m, hitcols[m.name], 6.0)
    elif m.name == "muzzle":
        emissive(m, (1, 1, 1), 8.0)

world = bpy.data.worlds.new("w")
world.color = (0.005, 0.006, 0.01)
scene.world = world
try:
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs[0].default_value = (0.01, 0.012, 0.02, 1)
    world.node_tree.nodes["Background"].inputs[1].default_value = 1.0
except Exception:
    pass

# Big floor to catch reflections.
zmin = min((o.matrix_world @ __import__("mathutils").Vector(c)).z for o in obs for c in o.bound_box)
bpy.ops.mesh.primitive_plane_add(size=200, location=(0, 0, min(zmin, 0.0) - 0.005))
fl = bpy.context.object
fm = bpy.data.materials.new("floorprev")
fm.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (0.01, 0.01, 0.012, 1)
fm.node_tree.nodes["Principled BSDF"].inputs["Roughness"].default_value = 0.3
fl.data.materials.append(fm)

sun = bpy.data.lights.new("sun", "SUN")
sun.energy = float(os.environ.get("PREVIEW_SUN", "1.5"))
so = bpy.data.objects.new("sun", sun)
so.rotation_euler = (math.radians(50), 0, math.radians(30))
scene.collection.objects.link(so)

bpy.context.view_layer.update()
_xs = [(o.matrix_world @ __import__("mathutils").Vector(c)).x for o in obs for c in o.bound_box]
cx = (min(_xs) + max(_xs)) / 2
cam = bpy.data.cameras.new("cam")
cam.lens = float(args[4]) if len(args) > 4 else 28
co = bpy.data.objects.new("cam", cam)
scene.collection.objects.link(co)
dist = float(args[5]) if len(args) > 5 else max(8.0, xmax * 0.75)
height = float(args[6]) if len(args) > 6 else 0.45
co.location = (cx, -row_z / 2 - dist, dist * height + target_z)
target = __import__("mathutils").Vector((cx, -row_z / 2, target_z))
direction = target - co.location
co.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()
scene.camera = co

scene.render.engine = "BLENDER_EEVEE"
size = os.environ.get("PREVIEW_SIZE", "1600x900").split("x")
scene.render.resolution_x = int(size[0])
scene.render.resolution_y = int(size[1])
scene.render.filepath = out
try:
    scene.eevee.taa_render_samples = 16
except Exception:
    pass
scene.view_settings.view_transform = "AgX" if "AgX" in [i.identifier for i in scene.view_settings.bl_rna.properties["view_transform"].enum_items] else "Standard"
bpy.ops.render.render(write_still=True)
print("rendered", out)
