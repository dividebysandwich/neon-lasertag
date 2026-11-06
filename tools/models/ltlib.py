"""Shared helpers for the lasertag Blender build scripts (kit.py, blaster.py, grenade.py).

Everything is authored in GAME coordinates (glTF/Bevy: Y up, metres) and converted to Blender
coordinates only when a mesh is created: game (x, y, z) -> Blender (x, -z, y). Blender's glTF
exporter (export_yup=True) maps it back, so the exported vertex positions are the game coords.

Geometry is collected in a `Geo` as independent faces (no shared vertices: flat shading, split
normals), each with a material name and per-corner UVs. Faces given a `want` direction are
flipped if needed so their normal points that way, which keeps winding mistakes out.
"""

import math
import os

import bpy

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

# Accent conventions (metres).
ACCENT_H = 0.008  # how far accent strips stand proud of a face
GLOW_H = 0.002  # glow halo quads float this far off the face


# ---------------------------------------------------------------------------------------------
# Small vector helpers on tuples.

def add(a, b):
    return tuple(x + y for x, y in zip(a, b))


def sub(a, b):
    return tuple(x - y for x, y in zip(a, b))


def mul(a, s):
    return tuple(x * s for x in a)


def dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def length(a):
    return math.sqrt(dot(a, a))


def norm(a):
    l = length(a)
    return mul(a, 1.0 / l) if l > 1e-12 else a


def to_blender(p):
    """Game (x, y, z) -> Blender (x, -z, y)."""
    return (p[0], -p[2], p[1])


def newell(pts):
    n = [0.0, 0.0, 0.0]
    for i, a in enumerate(pts):
        b = pts[(i + 1) % len(pts)]
        n[0] += (a[1] - b[1]) * (a[2] + b[2])
        n[1] += (a[2] - b[2]) * (a[0] + b[0])
        n[2] += (a[0] - b[0]) * (a[1] + b[1])
    return tuple(n)


# ---------------------------------------------------------------------------------------------
# Geometry accumulator.

class Geo:
    def __init__(self):
        self.faces = []  # (pts, material, uvs)

    def face(self, pts, mat, uvs=None, want=None):
        pts = [tuple(float(c) for c in p) for p in pts]
        if uvs is None:
            uvs = box_uvs(pts)
        uvs = [tuple(uv) for uv in uvs]
        if want is not None and dot(newell(pts), want) < 0:
            pts.reverse()
            uvs.reverse()
        self.faces.append((pts, mat, uvs))

    def tris(self):
        return sum(len(p) - 2 for p, _, _ in self.faces)

    def positions(self):
        for pts, _, _ in self.faces:
            yield from pts

    def mirror_errors(self, q=1e-3):
        """Vertices whose X mirror is not also a vertex (the kit must be X symmetric)."""
        key = lambda p: (round(p[0] / q), round(p[1] / q), round(p[2] / q))
        s = {key(p) for p in self.positions()}
        bad = []
        for k in s:
            m = (-k[0], k[1], k[2])
            if not any((m[0] + dx, m[1] + dy, m[2] + dz) in s
                       for dx in (-1, 0, 1) for dy in (-1, 0, 1) for dz in (-1, 0, 1)):
                bad.append(tuple(c * q for c in k))
        return bad


def box_uvs(pts, scale=0.5):
    """Box projection (0.5 UV units per metre) on the face's dominant axis."""
    n = newell(pts)
    ax = max(range(3), key=lambda i: abs(n[i]))
    a, b = [(1, 2), (0, 2), (0, 1)][ax]
    return [(p[a] * scale, p[b] * scale) for p in pts]


# ---------------------------------------------------------------------------------------------
# A planar 2D frame on a surface: p(u, v, h) = o + u*U + v*V + h*N with N = U x V.

class Frame:
    def __init__(self, o, U, V):
        self.o = tuple(o)
        self.U = norm(U)
        self.V = norm(V)
        self.N = norm(cross(self.U, self.V))

    def p(self, u, v, h=0.0):
        return add(self.o, add(mul(self.U, u), add(mul(self.V, v), mul(self.N, h))))

    def d(self, du, dv):
        """A 2D direction as a 3D vector."""
        return add(mul(self.U, du), mul(self.V, dv))


X, Y, Z = (1, 0, 0), (0, 1, 0), (0, 0, 1)
NX, NY, NZ = (-1, 0, 0), (0, -1, 0), (0, 0, -1)

# side -> (U, V); N = U x V is the outward normal. Chosen so a u-symmetric 2D design gives an
# X-mirror-symmetric piece on every face.
FACE_AXES = {
    "+z": (X, Y), "-z": (NX, Y),
    "+x": (NZ, Y), "-x": (Z, Y),
    "+y": (X, NZ), "-y": (X, Z),
}


def box_face(mn, mx, side):
    """Frame centred on one face of an axis-aligned box, plus the face's half sizes (hu, hv)."""
    c = [(a + b) / 2 for a, b in zip(mn, mx)]
    s = [(b - a) / 2 for a, b in zip(mn, mx)]
    ax = "xyz".index(side[1])
    c[ax] = mx[ax] if side[0] == "+" else mn[ax]
    U, V = FACE_AXES[side]
    hu = abs(dot(U, s))
    hv = abs(dot(V, s))
    return Frame(c, U, V), hu, hv


# ---------------------------------------------------------------------------------------------
# Body primitives.

def box(geo, mn, mx, mat="body", skip=(), side_mats=None):
    for side in ("+x", "-x", "+y", "-y", "+z", "-z"):
        if side in skip:
            continue
        f, hu, hv = box_face(mn, mx, side)
        pts = [f.p(-hu, -hv), f.p(hu, -hv), f.p(hu, hv), f.p(-hu, hv)]
        geo.face(pts, (side_mats or {}).get(side, mat), want=f.N)


def rect_face(geo, f, u0, v0, u1, v1, mat="body"):
    geo.face([f.p(u0, v0), f.p(u1, v0), f.p(u1, v1), f.p(u0, v1)], mat, want=f.N)


def slab_with_hole(geo, mn, mx, hole, mat="body", skip=()):
    """A box thin along Z with a rectangular through-hole hole=((x0, y0), (x1, y1))."""
    (hx0, hy0), (hx1, hy1) = hole
    for side in ("+z", "-z"):
        z = mx[2] if side == "+z" else mn[2]
        n = (0, 0, 1 if side == "+z" else -1)
        for (a, b, c, d) in (
            (mn[0], mn[1], hx0, mx[1]), (hx1, mn[1], mx[0], mx[1]),
            (hx0, mn[1], hx1, hy0), (hx0, hy1, hx1, mx[1]),
        ):
            geo.face([(a, b, z), (c, b, z), (c, d, z), (a, d, z)], mat, want=n)
    for side in ("+x", "-x", "+y", "-y"):
        if side in skip:
            continue
        f, hu, hv = box_face(mn, mx, side)
        geo.face([f.p(-hu, -hv), f.p(hu, -hv), f.p(hu, hv), f.p(-hu, hv)], mat, want=f.N)
    z0, z1 = mn[2], mx[2]
    # Reveals, facing into the hole.
    geo.face([(hx0, hy0, z0), (hx1, hy0, z0), (hx1, hy0, z1), (hx0, hy0, z1)], mat, want=Y)
    geo.face([(hx0, hy1, z0), (hx1, hy1, z0), (hx1, hy1, z1), (hx0, hy1, z1)], mat, want=NY)
    geo.face([(hx0, hy0, z0), (hx0, hy1, z0), (hx0, hy1, z1), (hx0, hy0, z1)], mat, want=X)
    geo.face([(hx1, hy0, z0), (hx1, hy1, z0), (hx1, hy1, z1), (hx1, hy0, z1)], mat, want=NX)


def octagon_xz(hx, hz, c):
    """Footprint of a box with vertical edges chamfered by c, CCW seen from +Y in (x, z)."""
    pts = [(hx, hz - c), (hx, -hz + c), (hx - c, -hz), (-hx + c, -hz),
           (-hx, -hz + c), (-hx, hz - c), (-hx + c, hz), (hx - c, hz)]
    return pts


def prism_y(geo, foot, y0, y1, mat="body", top=True, bottom=False):
    """Vertical prism over a convex (x, z) footprint."""
    n = len(foot)
    cx = sum(p[0] for p in foot) / n
    cz = sum(p[1] for p in foot) / n
    for i in range(n):
        a, b = foot[i], foot[(i + 1) % n]
        mid = ((a[0] + b[0]) / 2 - cx, 0, (a[1] + b[1]) / 2 - cz)
        geo.face([(a[0], y0, a[1]), (b[0], y0, b[1]), (b[0], y1, b[1]), (a[0], y1, a[1])],
                 mat, want=mid)
    if top:
        geo.face([(p[0], y1, p[1]) for p in foot], mat, want=Y)
    if bottom:
        geo.face([(p[0], y0, p[1]) for p in foot], mat, want=NY)


def prism_x(geo, profile, x0, x1, mat="body", skip_edges=()):
    """Prism along X over a convex (z, y) profile; skip_edges are profile edge indices."""
    n = len(profile)
    cz = sum(p[0] for p in profile) / n
    cy = sum(p[1] for p in profile) / n
    for i in range(n):
        if i in skip_edges:
            continue
        a, b = profile[i], profile[(i + 1) % n]
        mid = (0, (a[1] + b[1]) / 2 - cy, (a[0] + b[0]) / 2 - cz)
        geo.face([(x0, a[1], a[0]), (x0, b[1], b[0]), (x1, b[1], b[0]), (x1, a[1], a[0])],
                 mat, want=mid)
    geo.face([(x0, p[1], p[0]) for p in profile], mat, want=NX)
    geo.face([(x1, p[1], p[0]) for p in profile], mat, want=X)


# ---------------------------------------------------------------------------------------------
# 2D shapes (lists of (u, v)).

def chamfer_rect(u0, v0, u1, v1, c):
    return [(u0 + c, v0), (u1 - c, v0), (u1, v0 + c), (u1, v1 - c),
            (u1 - c, v1), (u0 + c, v1), (u0, v1 - c), (u0, v0 + c)]


def rect(u0, v0, u1, v1):
    return [(u0, v0), (u1, v0), (u1, v1), (u0, v1)]


def ngon(cu, cv, r, n, phase=90.0):
    return [(cu + r * math.cos(math.radians(phase + 360.0 * i / n)),
             cv + r * math.sin(math.radians(phase + 360.0 * i / n))) for i in range(n)]


def hexagon(cu, cv, r, pointy=True):
    return ngon(cu, cv, r, 6, 90.0 if pointy else 0.0)


def diamond(cu, cv, ru, rv=None):
    rv = ru if rv is None else rv
    return [(cu, cv - rv), (cu + ru, cv), (cu, cv + rv), (cu - ru, cv)]


def chevron(cu, cv, half_w, drop, up=True):
    """Open polyline: tip at (cu, cv), arms going down (up=True means it points up)."""
    s = -1 if up else 1
    return [(cu - half_w, cv + s * drop), (cu, cv), (cu + half_w, cv + s * drop)]


def signed_area(pts):
    return 0.5 * sum(pts[i][0] * pts[(i + 1) % len(pts)][1] - pts[(i + 1) % len(pts)][0] * pts[i][1]
                     for i in range(len(pts)))


def ccw(pts):
    return list(pts) if signed_area(pts) > 0 else list(reversed(pts))


def _left(d):
    return (-d[1], d[0])


def _n2(a):
    l = math.hypot(a[0], a[1])
    return (a[0] / l, a[1] / l)


def offset_line(pts, d, closed):
    """Offset a polyline by d to its left, mitred."""
    n = len(pts)
    out = []
    for i in range(n):
        if closed or 0 < i < n - 1:
            a, b, c = pts[i - 1], pts[i], pts[(i + 1) % n]
            n1 = _left(_n2((b[0] - a[0], b[1] - a[1])))
            n2 = _left(_n2((c[0] - b[0], c[1] - b[1])))
            s = max(1.0 + n1[0] * n2[0] + n1[1] * n2[1], 0.15)
            m = ((n1[0] + n2[0]) / s, (n1[1] + n2[1]) / s)
        elif i == 0:
            m = _left(_n2((pts[1][0] - pts[0][0], pts[1][1] - pts[0][1])))
        else:
            m = _left(_n2((pts[-1][0] - pts[-2][0], pts[-1][1] - pts[-2][1])))
        out.append((pts[i][0] + m[0] * d, pts[i][1] + m[1] * d))
    return out


def _arclen(pts, closed):
    s = [0.0]
    m = len(pts) if closed else len(pts) - 1
    for i in range(m):
        a, b = pts[i], pts[(i + 1) % len(pts)]
        s.append(s[-1] + math.hypot(b[0] - a[0], b[1] - a[1]))
    return s


# ---------------------------------------------------------------------------------------------
# Accents and glow.

def glow_band(geo, f, pts, closed, d0, d1, mat="glow"):
    """Glow quads between the left offsets d0 and d1 of a centreline (d1 may be negative for the
    right side). UV: U = arc length (m) along the centreline, V = 0 at d0, 1 at d1."""
    if abs(d1 - d0) < 1e-6:
        return
    inner = offset_line(pts, d0, closed)
    outer = offset_line(pts, d1, closed)
    s = _arclen(pts, closed)
    m = len(pts) if closed else len(pts) - 1
    for i in range(m):
        j = (i + 1) % len(pts)
        q = [f.p(*inner[i], GLOW_H), f.p(*inner[j], GLOW_H), f.p(*outer[j], GLOW_H),
             f.p(*outer[i], GLOW_H)]
        uv = [(s[i], 0.0), (s[i + 1], 0.0), (s[i + 1], 1.0), (s[i], 1.0)]
        geo.face(q, mat, uv, want=f.N)


def strip(geo, f, pts, w, closed=False, glow=(0.09, 0.09), mat="accent", h=ACCENT_H,
          glow_mat="glow", caps=True):
    """An accent strip of width w along a polyline, standing h proud of frame f, with glow
    halos of width glow=(left, right) lying on the surface beside it."""
    L = offset_line(pts, w / 2, closed)
    R = offset_line(pts, -w / 2, closed)
    s = _arclen(pts, closed)
    n = len(pts)
    m = n if closed else n - 1
    for i in range(m):
        j = (i + 1) % n
        d = _n2((pts[j][0] - pts[i][0], pts[j][1] - pts[i][1]))
        ln = f.d(*_left(d))
        geo.face([f.p(*R[i], h), f.p(*R[j], h), f.p(*L[j], h), f.p(*L[i], h)], mat,
                 [(s[i], 0), (s[i + 1], 0), (s[i + 1], 1), (s[i], 1)], want=f.N)
        geo.face([f.p(*L[i]), f.p(*L[j]), f.p(*L[j], h), f.p(*L[i], h)], mat, want=ln)
        geo.face([f.p(*R[i]), f.p(*R[j]), f.p(*R[j], h), f.p(*R[i], h)], mat, want=mul(ln, -1))
    if not closed and caps:
        d0 = f.d(*_n2((pts[1][0] - pts[0][0], pts[1][1] - pts[0][1])))
        d1 = f.d(*_n2((pts[-1][0] - pts[-2][0], pts[-1][1] - pts[-2][1])))
        geo.face([f.p(*R[0]), f.p(*L[0]), f.p(*L[0], h), f.p(*R[0], h)], mat, want=mul(d0, -1))
        geo.face([f.p(*R[-1]), f.p(*L[-1]), f.p(*L[-1], h), f.p(*R[-1], h)], mat, want=d1)
    gl, gr = glow
    if gl > 0:
        glow_band(geo, f, pts, closed, w / 2, w / 2 + gl, glow_mat)
    if gr > 0:
        glow_band(geo, f, pts, closed, -w / 2, -w / 2 - gr, glow_mat)


def ring(geo, f, pts, w, glow_in=0.09, glow_out=0.09, **kw):
    """Closed accent outline; pts are made CCW so 'left' is inside."""
    strip(geo, f, ccw(pts), w, closed=True, glow=(glow_in, glow_out), **kw)


def line(geo, f, a, b, w, glow=0.09, **kw):
    g = glow if isinstance(glow, tuple) else (glow, glow)
    strip(geo, f, [a, b], w, glow=g, **kw)


def filled(geo, f, pts, glow=0.08, mat="accent", h=ACCENT_H, glow_mat="glow"):
    """A filled convex accent polygon with a glow halo around it."""
    pts = ccw(pts)
    geo.face([f.p(u, v, h) for u, v in pts], mat, [(u, v) for u, v in pts], want=f.N)
    n = len(pts)
    for i in range(n):
        a, b = pts[i], pts[(i + 1) % n]
        d = _n2((b[0] - a[0], b[1] - a[1]))
        out = f.d(d[1], -d[0])
        geo.face([f.p(*a), f.p(*b), f.p(*b, h), f.p(*a, h)], mat, want=out)
    if glow > 0:
        glow_band(geo, f, pts, True, 0.0, -glow, glow_mat)


# ---------------------------------------------------------------------------------------------
# Blender side: materials, objects, export.

def reset_scene():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def _principled(m):
    nt = m.node_tree
    return next(n for n in nt.nodes if n.type == "BSDF_PRINCIPLED")


def material(name, base=(0.02, 0.02, 0.02), rough=0.35, emit=None, strength=0.0, alpha=1.0,
             metal=0.0):
    m = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    try:
        m.use_nodes = True
    except Exception:
        pass
    p = _principled(m)
    p.inputs["Base Color"].default_value = (*base, 1.0)
    p.inputs["Roughness"].default_value = rough
    p.inputs["Metallic"].default_value = metal
    if emit is not None:
        p.inputs["Emission Color"].default_value = (*emit, 1.0)
        p.inputs["Emission Strength"].default_value = strength
    if alpha < 1.0:
        p.inputs["Alpha"].default_value = alpha
        for attr, val in (("surface_render_method", "BLENDED"), ("blend_method", "BLEND")):
            try:
                setattr(m, attr, val)
            except Exception:
                pass
    m.diffuse_color = (*(emit if emit is not None else base), alpha)
    m.roughness = rough
    return m


def standard_materials():
    """The material names the game recognises (it replaces them at load time)."""
    return {
        "body": material("body", base=(0.02, 0.02, 0.02), rough=0.35),
        "accent": material("accent", base=(1, 1, 1), rough=0.4, emit=(1, 1, 1), strength=3.0),
        "glow": material("glow", base=(1, 1, 1), rough=1.0, emit=(1, 1, 1), strength=1.0,
                         alpha=0.35),
    }


def make_object(name, geo, collection=None):
    """Mesh object (object and mesh data both called `name`) from a Geo, identity transform."""
    verts, polys, mats, uvs = [], [], [], []
    slots = {}
    for pts, mat, fuv in geo.faces:
        i0 = len(verts)
        verts += [to_blender(p) for p in pts]
        polys.append(list(range(i0, i0 + len(pts))))
        if mat not in slots:
            slots[mat] = len(slots)
        mats.append(slots[mat])
        uvs += fuv
    me = bpy.data.meshes.new(name)
    me.from_pydata(verts, [], polys)
    for mat in slots:
        m = bpy.data.materials.get(mat)
        if m is None:
            raise KeyError("material %r not created" % mat)
        me.materials.append(m)
    uvl = me.uv_layers.new(name="UVMap")
    for poly, mi in zip(me.polygons, mats):
        poly.material_index = mi
        poly.use_smooth = False
    # Scripts author UVs in glTF convention (V down from the top); the exporter writes
    # V_gltf = 1 - V_blender, so store the flipped value.
    for li, uv in enumerate(uvs):
        uvl.data[li].uv = (uv[0], 1.0 - uv[1])
    me.update()
    if len(me.polygons) != len(polys):
        raise RuntimeError("%s: degenerate faces" % name)
    ob = bpy.data.objects.new(name, me)
    if ob.name != name or me.name != name:
        raise RuntimeError("name clash for %s" % name)
    (collection or bpy.context.scene.collection).objects.link(ob)
    return ob


def save_blend(rel):
    path = os.path.join(ROOT, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    bpy.context.preferences.filepaths.save_version = 0  # no .blend1 backups
    bpy.ops.wm.save_as_mainfile(filepath=path, compress=True)


def export_glb(rel):
    path = os.path.join(ROOT, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    bpy.ops.export_scene.gltf(
        filepath=path, export_format="GLB", export_yup=True, export_apply=True,
        export_texcoords=True, export_normals=True, export_tangents=False,
        export_materials="EXPORT", use_selection=False, use_visible=False,
        export_animations=False, export_cameras=False, export_lights=False, export_extras=False,
    )
    print("exported", path)


# ---------------------------------------------------------------------------------------------
# Solids for props (blaster, grenade).

def loft(geo, rings, axis, mats="body", cap0=None, cap1=None):
    """Tube through rings (equal-length 3D point lists) progressing along `axis`. Side faces
    face outward (shoulders where the radius steps face along/against the axis). mats is one
    material or a list per segment; cap0/cap1 give the end-cap material (None = open)."""
    axis = norm(axis)
    cs = [mul(tuple(map(sum, zip(*r))), 1.0 / len(r)) for r in rings]
    rs = [sum(length(sub(p, c)) for p in r) / len(r) for r, c in zip(rings, cs)]
    n = len(rings[0])
    for k in range(len(rings) - 1):
        mat = mats if isinstance(mats, str) else mats[k]
        da = dot(sub(cs[k + 1], cs[k]), axis)
        dr = rs[k + 1] - rs[k]
        if abs(da) < 1e-9 and abs(dr) < 1e-9:
            continue
        cm = mul(add(cs[k], cs[k + 1]), 0.5)
        for i in range(n):
            j = (i + 1) % n
            q = [rings[k][i], rings[k][j], rings[k + 1][j], rings[k + 1][i]]
            mid = mul(tuple(map(sum, zip(*q))), 0.25)
            rad = sub(mid, cm)
            rad = norm(sub(rad, mul(axis, dot(rad, axis))))
            geo.face(q, mat, want=add(mul(rad, da), mul(axis, -dr)))
    if cap0:
        geo.face(rings[0], cap0, want=mul(axis, -1))
    if cap1:
        geo.face(rings[-1], cap1, want=axis)


def band_x(geo, pts, thick, x0, x1, mat="body", caps=True):
    """A bar of in-plane thickness `thick` following a (z, y) polyline, spanning x0..x1."""
    Lp = offset_line(pts, thick / 2, False)
    Rp = offset_line(pts, -thick / 2, False)
    P = lambda q, x: (x, q[1], q[0])
    for i in range(len(pts) - 1):
        j = i + 1
        d = _n2((pts[j][0] - pts[i][0], pts[j][1] - pts[i][1]))
        ln = (0, _left(d)[1], _left(d)[0])  # left normal in (z, y), as (x, y, z)
        geo.face([P(Lp[i], x0), P(Lp[j], x0), P(Lp[j], x1), P(Lp[i], x1)], mat, want=ln)
        geo.face([P(Rp[i], x0), P(Rp[j], x0), P(Rp[j], x1), P(Rp[i], x1)], mat, want=mul(ln, -1))
        geo.face([P(Lp[i], x0), P(Lp[j], x0), P(Rp[j], x0), P(Rp[i], x0)], mat, want=NX)
        geo.face([P(Lp[i], x1), P(Lp[j], x1), P(Rp[j], x1), P(Rp[i], x1)], mat, want=X)
    if caps:
        for a, b, o in ((0, 1, 0), (-1, -2, -1)):
            d = _n2((pts[a][0] - pts[b][0], pts[a][1] - pts[b][1]))
            geo.face([P(Lp[o], x0), P(Rp[o], x0), P(Rp[o], x1), P(Lp[o], x1)], mat,
                     want=(0, d[1], d[0]))


def make_empty(name, game_pos, collection=None):
    ob = bpy.data.objects.new(name, None)
    ob.empty_display_type = "PLAIN_AXES"
    ob.empty_display_size = 0.02
    ob.location = to_blender(game_pos)
    (collection or bpy.context.scene.collection).objects.link(ob)
    if ob.name != name:
        raise RuntimeError("name clash for %s" % name)
    return ob
