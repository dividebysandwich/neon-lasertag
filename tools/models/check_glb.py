#!/usr/bin/env python3
"""Checks the exported models against the game's conventions (stdlib only).

    python3 tools/models/check_glb.py   # checks assets/models/{kit,blaster,grenade,avatars,reload,powerups}.glb

kit.glb: every piece of tools/models/kit_dims.json is a mesh named exactly the piece name whose
POSITION extent (union over primitives, read from the binary buffer) equals the piece bounds
within +-0.025 m, protruding at most 0.02 m beyond them, and whose `body` primitives alone also
fill the bounds. All meshes: NORMAL and TEXCOORD_0 on every primitive, only known material
names. avatars.glb: the 8 heads, torso and both hands by exact mesh name, only the avatar
materials, hit_1..3 on the torso, and size sanity (heads within +-0.35 m of the eye point in X/Z
and at most 0.6 m tall, torso top at y = 0, hands within 0.15 m of the palm centre); every head
has `eyes` and `eye_glow`, every part `flow`. reload.glb: one mesh `reload_station` inside its
2 m cell (|x|, |z| <= 1), on the floor, with nothing between 0.04 m and 2.25 m (walkable).
Halo materials (`glow`, `eye_glow`, `halo_white`) and `flow` must have UV V within 0..1 and
`flow` UV U >= 0 (arc length). Exits non-zero on any mismatch.
"""

import json
import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
TOL = 0.025
PROUD = 0.02

COMPONENTS = {5120: "b", 5121: "B", 5122: "h", 5123: "H", 5125: "I", 5126: "f"}
WIDTH = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}


def load_glb(path):
    data = open(path, "rb").read()
    magic, version, total = struct.unpack_from("<4sII", data, 0)
    if magic != b"glTF" or version != 2:
        raise ValueError("%s: not a glTF 2 binary" % path)
    off, gltf, binary = 12, None, b""
    while off < total:
        clen, ctype = struct.unpack_from("<II", data, off)
        chunk = data[off + 8: off + 8 + clen]
        if ctype == 0x4E4F534A:
            gltf = json.loads(chunk.decode("utf-8"))
        elif ctype == 0x004E4942:
            binary = chunk
        off += 8 + clen
    return gltf, binary


def read_accessor(gltf, binary, index):
    acc = gltf["accessors"][index]
    view = gltf["bufferViews"][acc["bufferView"]]
    fmt = COMPONENTS[acc["componentType"]]
    n = WIDTH[acc["type"]]
    size = struct.calcsize(fmt)
    stride = view.get("byteStride", size * n)
    base = view.get("byteOffset", 0) + acc.get("byteOffset", 0)
    return [struct.unpack_from("<%d%s" % (n, fmt), binary, base + i * stride)
            for i in range(acc["count"])]


def extent(points):
    mn = [min(p[i] for p in points) for i in range(3)]
    mx = [max(p[i] for p in points) for i in range(3)]
    return mn, mx


def tri_count(gltf, prim):
    if "indices" in prim:
        return gltf["accessors"][prim["indices"]]["count"] // 3
    return gltf["accessors"][prim["attributes"]["POSITION"]]["count"] // 3


class Checker:
    def __init__(self):
        self.errors = []

    def err(self, msg):
        self.errors.append(msg)
        print("FAIL", msg)


def check_common(ck, label, gltf, binary, allowed):
    mats = [m.get("name") for m in gltf.get("materials", [])]
    meshes = {}
    for mesh in gltf.get("meshes", []):
        pts_all, body, tris = [], [], 0
        for prim in mesh["primitives"]:
            attrs = prim["attributes"]
            for a in ("POSITION", "NORMAL", "TEXCOORD_0"):
                if a not in attrs:
                    ck.err("%s/%s: primitive without %s" % (label, mesh["name"], a))
            mname = mats[prim["material"]] if "material" in prim else None
            if mname not in allowed:
                ck.err("%s/%s: unexpected material %r" % (label, mesh["name"], mname))
            pts = read_accessor(gltf, binary, attrs["POSITION"])
            acc = gltf["accessors"][attrs["POSITION"]]
            mn, mx = extent(pts)
            if "min" in acc and any(abs(a - b) > 1e-4 for a, b in zip(acc["min"] + acc["max"], mn + mx)):
                ck.err("%s/%s: accessor min/max disagree with the data" % (label, mesh["name"]))
            pts_all += pts
            if mname == "body":
                body += pts
            tris += tri_count(gltf, prim)
        meshes[mesh["name"]] = (pts_all, body, tris)
    return meshes


HALOS = {"glow", "eye_glow", "halo_white"}


def check_uvs(ck, label, gltf, binary):
    mats = [m.get("name") for m in gltf.get("materials", [])]
    for mesh in gltf.get("meshes", []):
        for prim in mesh["primitives"]:
            mname = mats[prim["material"]] if "material" in prim else None
            if mname not in HALOS | {"flow"}:
                continue
            uv = read_accessor(gltf, binary, prim["attributes"]["TEXCOORD_0"])
            if any(v < -1e-4 or v > 1 + 1e-4 for _, v in uv):
                ck.err("%s/%s: %s UV V outside 0..1" % (label, mesh["name"], mname))
            if mname == "flow" and any(u < -1e-4 for u, _ in uv):
                ck.err("%s/%s: flow UV U (arc length) negative" % (label, mesh["name"]))


def check_kit(ck):
    dims = json.load(open(os.path.join(HERE, "kit_dims.json")))
    gltf, binary = load_glb(os.path.join(ROOT, "assets/models/kit.glb"))
    meshes = check_common(ck, "kit", gltf, binary, {"body", "accent", "glow"})
    for name, piece in dims["pieces"].items():
        if name not in meshes:
            ck.err("kit: no mesh named %r" % name)
            continue
        pts, body, tris = meshes[name]
        bmn, bmx = piece["bounds"]
        mn, mx = extent(pts)
        ok = True
        for ax in range(3):
            for got, want, sign in ((mn[ax], bmn[ax], -1), (mx[ax], bmx[ax], 1)):
                if abs(got - want) > TOL or sign * (got - want) > PROUD + 1e-6:
                    ck.err("kit/%s: axis %s extent %.4f vs bounds %.4f" % (name, "xyz"[ax], got, want))
                    ok = False
        if not body:
            ck.err("kit/%s: no body primitive" % name)
        else:
            mn, mx = extent(body)
            for ax in range(3):
                for got, want in ((mn[ax], bmn[ax]), (mx[ax], bmx[ax])):
                    if abs(got - want) > TOL:
                        ck.err("kit/%s: body does not fill bounds on %s (%.4f vs %.4f)"
                               % (name, "xyz"[ax], got, want))
                        ok = False
        print("%-4s kit/%-14s %5d tris" % ("ok" if ok else "BAD", name, tris))
    # Glow fade direction: on floor_tile the halo starts (V = 0) at the edge lines.
    mesh = next((m for m in gltf["meshes"] if m["name"] == "floor_tile"), None)
    mats = [m.get("name") for m in gltf.get("materials", [])]
    for prim in (mesh or {"primitives": []})["primitives"]:
        if mats[prim["material"]] != "glow":
            continue
        pos = read_accessor(gltf, binary, prim["attributes"]["POSITION"])
        uv = read_accessor(gltf, binary, prim["attributes"]["TEXCOORD_0"])
        for p, t in zip(pos, uv):
            edge = max(abs(p[0]), abs(p[2])) > 0.985
            if edge and t[1] > 0.01 and min(abs(p[0]), abs(p[2])) > 0.985:
                ck.err("kit/floor_tile: glow V is %.2f at the edge line (want 0)" % t[1])
                break


def check_model(ck, file, allowed, required, nodes=()):
    path = os.path.join(ROOT, "assets/models", file)
    if not os.path.exists(path):
        ck.err("%s missing" % file)
        return
    gltf, binary = load_glb(path)
    meshes = check_common(ck, file, gltf, binary, allowed)
    used = {gltf["materials"][p["material"]]["name"]
            for m in gltf["meshes"] for p in m["primitives"] if "material" in p}
    for r in required:
        if r not in used:
            ck.err("%s: material %r not used" % (file, r))
    names = {n.get("name"): n for n in gltf.get("nodes", [])}
    for n in nodes:
        if n not in names:
            ck.err("%s: no node %r" % (file, n))
        else:
            print("     %s node %s at %s" % (file, n, names[n].get("translation", [0, 0, 0])))
    for name, (pts, _, tris) in meshes.items():
        mn, mx = extent(pts)
        print("ok   %s/%s %d tris, extent %s .. %s" % (file, name, tris,
              [round(v, 3) for v in mn], [round(v, 3) for v in mx]))


AVATAR_HEADS = ("wolf", "cat", "deer", "fox", "bear", "rabbit", "owl", "boar")


def check_avatars(ck):
    path = os.path.join(ROOT, "assets/models/avatars.glb")
    if not os.path.exists(path):
        ck.err("avatars.glb missing")
        return
    gltf, binary = load_glb(path)
    allowed = {"body", "fur_light", "accent", "glow", "hit_1", "hit_2", "hit_3", "eyes",
               "eye_glow", "flow"}
    meshes = check_common(ck, "avatars.glb", gltf, binary, allowed)
    check_uvs(ck, "avatars.glb", gltf, binary)
    mats = [m.get("name") for m in gltf.get("materials", [])]
    used = {m["name"]: {mats[p["material"]] for p in m["primitives"] if "material" in p}
            for m in gltf["meshes"]}
    heads = ["head_" + h for h in AVATAR_HEADS]
    for name in heads + ["torso", "hand_left", "hand_right"]:
        if name not in meshes:
            ck.err("avatars.glb: no mesh named %r" % name)
            continue
        pts, _, tris = meshes[name]
        mn, mx = extent(pts)
        ok = True

        def bad(msg):
            nonlocal ok
            ck.err("avatars.glb/%s: %s" % (name, msg))
            ok = False
        need = {"body", "accent", "flow"}
        if name == "torso":
            need |= {"hit_1", "hit_2", "hit_3"}
        elif name.startswith("head_"):
            need |= {"eyes", "eye_glow"}
        for m in sorted(need - used[name]):
            bad("material %r not used" % m)
        if name.startswith("head_"):
            if max(abs(mn[0]), mx[0], abs(mn[2]), mx[2]) > 0.35:
                bad("extends more than 0.35 m from the eye point in X/Z")
            if mx[1] - mn[1] > 0.6:
                bad("%.3f m tall (max 0.6)" % (mx[1] - mn[1]))
            if not (mn[1] < 0 < mx[1] and mn[2] < 0 < mx[2]):
                bad("origin (eye point) is not inside the head's extent")
        elif name == "torso":
            if abs(mx[1]) > 0.03:
                bad("top at y = %.3f (want 0, the neck point)" % mx[1])
            if mn[1] > -0.3:
                bad("does not hang down from the origin")
        else:
            if max(max(abs(a), abs(b)) for a, b in zip(mn, mx)) > 0.15:
                bad("extends more than 0.15 m from the palm centre")
        print("%-4s avatars.glb/%-12s %4d tris, extent %s .. %s, materials %s" % (
            "ok" if ok else "BAD", name, tris, [round(v, 3) for v in mn],
            [round(v, 3) for v in mx], ",".join(sorted(used[name]))))
    extra = set(meshes) - set(heads) - {"torso", "hand_left", "hand_right"}
    if extra:
        ck.err("avatars.glb: unexpected meshes %s" % sorted(extra))


def check_reload(ck):
    path = os.path.join(ROOT, "assets/models/reload.glb")
    if not os.path.exists(path):
        ck.err("reload.glb missing")
        return
    gltf, binary = load_glb(path)
    meshes = check_common(ck, "reload.glb", gltf, binary, {"body", "glow_white", "halo_white"})
    check_uvs(ck, "reload.glb", gltf, binary)
    mats = [m.get("name") for m in gltf.get("materials", [])]
    if set(meshes) != {"reload_station"}:
        ck.err("reload.glb: want exactly one mesh 'reload_station', got %s" % sorted(meshes))
    for mesh in gltf["meshes"]:
        used = {mats[p["material"]] for p in mesh["primitives"] if "material" in p}
        for m in sorted({"body", "glow_white", "halo_white"} - used):
            ck.err("reload.glb/%s: material %r not used" % (mesh["name"], m))
        pts, _, tris = meshes[mesh["name"]]
        mn, mx = extent(pts)
        ok = True
        if max(abs(mn[0]), mx[0], abs(mn[2]), mx[2]) > 1.0 + 1e-4:
            ck.err("reload.glb/%s: leaves its 2 m cell" % mesh["name"])
            ok = False
        if mn[1] < -1e-4:
            ck.err("reload.glb/%s: below the floor (y = %.3f)" % (mesh["name"], mn[1]))
            ok = False
        blocking = [p for p in pts if 0.04 + 1e-4 < p[1] < 2.25 - 1e-4]
        if blocking:
            ck.err("reload.glb/%s: %d vertices between 0.04 m and 2.25 m, e.g. %s"
                   % (mesh["name"], len(blocking), [round(v, 3) for v in blocking[0]]))
            ok = False
        print("%-4s reload.glb/%s %d tris, extent %s .. %s, materials %s" % (
            "ok" if ok else "BAD", mesh["name"], tris, [round(v, 3) for v in mn],
            [round(v, 3) for v in mx], ",".join(sorted(used))))


def main():
    ck = Checker()
    check_kit(ck)
    check_model(ck, "blaster.glb", {"body", "accent", "glow", "hit_1", "hit_2", "hit_3", "muzzle"},
                ("body", "accent", "hit_1", "hit_2", "hit_3", "muzzle"), nodes=("muzzle",))
    check_model(ck, "grenade.glb", {"body", "glow_white", "glow"}, ("body", "glow_white"))
    check_avatars(ck)
    check_reload(ck)
    check_model(ck, "powerups.glb", {"body", "accent", "hit_1", "hit_2", "hit_3", "muzzle"},
                ("body", "accent", "hit_1", "hit_2", "hit_3", "muzzle"), nodes=("shotgun_muzzle",))
    if ck.errors:
        print("%d problem(s)" % len(ck.errors))
        sys.exit(1)
    print("all good")


if __name__ == "__main__":
    main()
