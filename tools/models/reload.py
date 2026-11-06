"""Builds the blaster reload station: assets-src/blend/reload.blend and assets/models/reload.glb.

    blender -b --factory-startup --python-exit-code 1 --python tools/models/reload.py

One mesh, `reload_station`, filling one 2 m x 2 m arena grid cell: origin at the cell centre on
the floor, Y up, X mirror symmetric. Players walk onto it, so everything below head height is
flat: a chamfered floor pad (1.6 m across, at most 0.04 m high with its inlaid light rings and
chevrons), and nothing at all between 0.04 m and 2.25 m. Above head height (2.38-2.9 m) hovers an
octagonal emitter crown: a ring on four spokes around a hub whose downward-facing emitter disc
beams the refill down, with crown fins on the ring's corners.

Materials: `body` (the kit's black), `glow_white` (white light strips and the emitter; the game
renders it bright white whatever the team), `halo_white` (additive halo quads beside the white
strips; UV V = 0 at the strip edge, 1 at the outer edge, the kit's `glow` convention).
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ltlib as L  # noqa: E402
from ltlib import Frame, X, Y, Z, NX, NY, NZ  # noqa: E402

LIGHT, HALO = "glow_white", "halo_white"
PAD_Y = 0.03  # pad top; inlays stand ACCENT_H (0.008) proud -> 0.038
FLAT_MAX = 0.04
CLEAR_Y = 2.25

# Crown.
RING_Y0, RING_Y1 = 2.6, 2.72  # ring underside / top
RING_IN, RING_OUT = 0.62, 0.8  # vertex radii of the ring's octagons
HUB_R = 0.2
HUB_Y0, HUB_Y1 = 2.5, 2.82
EMIT_Y = 2.4  # hub's bottom face, the emitter
FIN_TOP = 2.9


def light(geo, f, pts, w, closed=False, glow=(0.05, 0.05), **kw):
    L.strip(geo, f, pts, w, closed=closed, glow=glow, mat=LIGHT, glow_mat=HALO, **kw)


def light_ring(geo, f, pts, w, glow_in=0.05, glow_out=0.05, **kw):
    L.ring(geo, f, pts, w, glow_in=glow_in, glow_out=glow_out, mat=LIGHT, glow_mat=HALO, **kw)


def octagon(r):
    """(u, v) octagon with vertex radius r and flats facing the axes."""
    return L.ngon(0, 0, r, 8, 22.5)


def oct3(r, y):
    return [(u, y, v) for u, v in octagon(r)]


# ---------------------------------------------------------------------------------------------

def pad(g):
    # Chamfered slab: vertical edge to 0.018 m, then a 45-degree bevel to the top.
    foot = lambda h: [(x, z) for x, z in L.octagon_xz(h, h, h * 0.3)]
    rings = [[(x, y, z) for x, z in foot(h)] for h, y in ((0.8, 0.0), (0.8, 0.018), (0.788, PAD_Y))]
    L.loft(g, rings, Y, cap1="body")
    f = Frame((0, PAD_Y, 0), X, NZ)  # N = +Y; (u, v) = (x, -z)
    light_ring(g, f, L.chamfer_rect(-0.69, -0.69, 0.69, 0.69, 0.2), 0.035, glow_in=0.06,
               glow_out=0.05)
    light_ring(g, f, octagon(0.44), 0.03, glow_in=0.05, glow_out=0.06)
    light_ring(g, f, octagon(0.25), 0.025, glow_in=0.04, glow_out=0.04)
    L.filled(g, f, octagon(0.12), glow=0.05, mat=LIGHT, glow_mat=HALO)
    # Chevrons on the four sides pointing in at the centre.
    for (du, dv) in ((1, 0), (-1, 0), (0, 1), (0, -1)):
        for r in (0.5, 0.57):
            tip = (du * r, dv * r)
            arm = lambda s: (du * (r + 0.06) - dv * s * 0.15, dv * (r + 0.06) + du * s * 0.15)
            light(g, f, [arm(-1), tip, arm(1)], 0.03, glow=(0.03, 0.03))
    # Small corner ticks on the diagonal chamfers.
    for su in (-1, 1):
        for sv in (-1, 1):
            a = (su * 0.5, sv * 0.62)
            b = (su * 0.62, sv * 0.5)
            light(g, f, [a, b], 0.02, glow=(0.025, 0.025))


def ring(g):
    lo, hi = RING_Y0, RING_Y1
    chamf = 0.04
    # Outer wall with a chamfered lower edge.
    L.loft(g, [oct3(RING_OUT - chamf, lo), oct3(RING_OUT, lo + chamf), oct3(RING_OUT, hi)], Y)
    # Inner wall (faces the hub).
    a, b = oct3(RING_IN, lo), oct3(RING_IN, hi)
    for i in range(8):
        j = (i + 1) % 8
        q = [a[i], a[j], b[j], b[i]]
        mid = L.mul(tuple(map(sum, zip(*q))), 0.25)
        g.face(q, "body", want=(-mid[0], 0, -mid[2]))
    # Top and underside annuli.
    for y, r0, r1, n in ((hi, RING_IN, RING_OUT, Y), (lo, RING_IN, RING_OUT - chamf, NY)):
        p, q = oct3(r0, y), oct3(r1, y)
        for i in range(8):
            j = (i + 1) % 8
            g.face([p[i], p[j], q[j], q[i]], "body", want=n)
    # Light band round the outer faces.
    flat = RING_OUT * math.cos(math.radians(22.5))
    half = RING_OUT * math.sin(math.radians(22.5))
    ym = (lo + chamf + hi) / 2
    for k in range(8):
        a = math.radians(45 * k)
        d = (math.cos(a), 0, math.sin(a))
        t = L.cross(Y, d)  # U x V = N must be d: U = Y x d, V = Y
        f = Frame(L.add(L.mul(d, flat), (0, ym, 0)), t, Y)
        light(g, f, [(-half + 0.08, 0), (half - 0.08, 0)], 0.026, glow=(0.025, 0.025))
    # Underside: a light ring beaming down, between the walls.
    fu = Frame((0, lo, 0), X, Z)  # N = -Y; (u, v) = (x, z)
    rm = (RING_IN + RING_OUT - chamf) / 2
    light_ring(g, fu, octagon(rm), 0.03, glow_in=0.035, glow_out=0.03)
    # Top: a thin inlay.
    ft = Frame((0, hi, 0), X, NZ)
    light_ring(g, ft, octagon((RING_IN + RING_OUT) / 2), 0.016, glow_in=0.025, glow_out=0.025)


def hub(g):
    # Octagonal hub with a chamfered top, tapering below to the emitter face.
    L.loft(g, [oct3(0.14, EMIT_Y), oct3(HUB_R, HUB_Y0), oct3(HUB_R, HUB_Y1),
               oct3(HUB_R - 0.05, HUB_Y1 + 0.04)], Y, cap0="body", cap1="body")
    fe = Frame((0, EMIT_Y, 0), X, Z)  # facing down
    L.filled(g, fe, octagon(0.1), glow=0.03, mat=LIGHT, glow_mat=HALO)
    flat = HUB_R * math.cos(math.radians(22.5))
    half = HUB_R * math.sin(math.radians(22.5))
    for k in range(8):
        a = math.radians(45 * k)
        d = (math.cos(a), 0, math.sin(a))
        f = Frame(L.add(L.mul(d, flat), (0, 2.555, 0)), L.cross(Y, d), Y)
        light(g, f, [(-half + 0.03, 0), (half - 0.03, 0)], 0.022, glow=(0.02, 0.02))
    ftop = Frame((0, HUB_Y1 + 0.04, 0), X, NZ)
    L.filled(g, ftop, octagon(0.07), glow=0.03, mat=LIGHT, glow_mat=HALO)


def spokes(g):
    y0, y1, hw = 2.63, 2.69, 0.03
    r0 = HUB_R * math.cos(math.radians(22.5)) - 0.005
    r1 = RING_IN * math.cos(math.radians(22.5)) + 0.005
    for d in ((1, 0), (-1, 0), (0, 1), (0, -1)):
        if d[0]:
            mn = (min(d[0] * r0, d[0] * r1), y0, -hw)
            mx = (max(d[0] * r0, d[0] * r1), y1, hw)
            skip = ("+x", "-x")
        else:
            mn = (-hw, y0, min(d[1] * r0, d[1] * r1))
            mx = (hw, y1, max(d[1] * r0, d[1] * r1))
            skip = ("+z", "-z")
        L.box(g, mn, mx, skip=skip)
        f, hu, hv = L.box_face(mn, mx, "-y")
        if hu >= hv:
            light(g, f, [(-hu + 0.03, 0), (hu - 0.03, 0)], 0.016, glow=(0.012, 0.012))
        else:
            light(g, f, [(0, -hv + 0.03), (0, hv - 0.03)], 0.016, glow=(0.012, 0.012))


def fins(g):
    """Crown fins: thin radial blades standing on the ring's corners, outer edge lit."""
    th = 0.015
    prof = [(RING_IN + 0.02, RING_Y1), (RING_OUT - 0.005, RING_Y1), (RING_OUT - 0.05, FIN_TOP),
            (RING_OUT - 0.1, FIN_TOP - 0.03)]  # (r, y), CCW seen from the tangent side
    for k in range(8):
        a = math.radians(22.5 + 45 * k)
        d = (math.cos(a), 0, math.sin(a))
        t = (-math.sin(a), 0, math.cos(a))
        P = lambda p, s: L.add(L.add(L.mul(d, p[0]), (0, p[1], 0)), L.mul(t, s * th))
        g.face([P(p, 1) for p in prof], "body", want=t)
        g.face([P(p, -1) for p in prof], "body", want=L.mul(t, -1))
        c = (sum(p[0] for p in prof) / 4, sum(p[1] for p in prof) / 4)
        for i in range(1, 4):  # edge 0 sits on the ring
            p, q = prof[i], prof[(i + 1) % 4]
            m = ((p[0] + q[0]) / 2 - c[0], (p[1] + q[1]) / 2 - c[1])
            out = L.add(L.mul(d, m[0]), (0, m[1], 0))
            g.face([P(p, -1), P(q, -1), P(q, 1), P(p, 1)], LIGHT if i == 1 else "body", want=out)


def build():
    g = L.Geo()
    pad(g)
    ring(g)
    hub(g)
    spokes(g)
    fins(g)
    return g


def main():
    L.reset_scene()
    L.standard_materials()
    L.material(LIGHT, base=(1, 1, 1), emit=(1, 1, 1), strength=4.0)
    L.material(HALO, base=(1, 1, 1), rough=1.0, emit=(1, 1, 1), strength=1.0, alpha=0.35)
    g = build()
    bad = g.mirror_errors()
    if bad:
        raise SystemExit("reload_station is not X mirror symmetric: %s" % bad[:5])
    for p in g.positions():
        if FLAT_MAX + 1e-6 < p[1] < CLEAR_Y or p[1] < -1e-6 or max(abs(p[0]), abs(p[2])) > 1.0:
            raise SystemExit("reload_station: vertex %s outside the walkable envelope" % (p,))
    L.make_object("reload_station", g)
    for name in ("accent", "glow"):
        m = L.bpy.data.materials.get(name)
        if m and m.users == 0:
            L.bpy.data.materials.remove(m)
    print("reload_station %d tris" % g.tris())
    L.save_blend("assets-src/blend/reload.blend")
    L.export_glb("assets/models/reload.glb")


main()
