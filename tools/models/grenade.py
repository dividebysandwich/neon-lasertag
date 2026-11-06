"""Builds the lasertag grenade: assets-src/blend/grenade.blend and assets/models/grenade.glb.

    blender -b --factory-startup --python-exit-code 1 --python tools/models/grenade.py

A low-poly segmented "pineapple" grenade, ~0.09 m across and ~0.12 m tall including the fuze,
with spoon (lever) and pull ring. Origin at the centre of the body. Materials: `body` (dark),
`glow_white` (the lit grooves between the segments and the waist ring).
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ltlib as L  # noqa: E402

RX, RY = 0.0405, 0.046  # groove-floor ellipsoid (the segments stand proud of it)
LIFT = 1.1  # segment tops
COLS = 8
ROWS_UP = (4, 27, 51, 75)  # latitude boundaries (deg) of the upper rows; mirrored below
WAIST = 4


def surf(lat, lon, s=1.0):
    la, lo = math.radians(lat), math.radians(lon)
    return (RX * s * math.cos(la) * math.sin(lo), RY * s * math.sin(la), RX * s * math.cos(la) * math.cos(lo))


def lerp(a, b, t):
    return a + (b - a) * t


def build():
    g = L.Geo()
    lats = [-x for x in reversed(ROWS_UP)] + list(ROWS_UP)
    rows = [(lats[i], lats[i + 1]) for i in range(len(lats) - 1) if lats[i] != -WAIST]
    dlon = 360.0 / COLS
    for la0, la1 in rows:
        for c in range(COLS):
            lo0, lo1 = c * dlon, (c + 1) * dlon
            ft, fl = 0.16, 0.13  # inset (fraction of the cell) in latitude / longitude
            ia0, ia1 = lerp(la0, la1, ft), lerp(la0, la1, 1 - ft)
            io0, io1 = lerp(lo0, lo1, fl), lerp(lo0, lo1, 1 - fl)
            outer = [surf(la0, lo0), surf(la0, lo1), surf(la1, lo1), surf(la1, lo0)]
            inner = [surf(ia0, io0), surf(ia0, io1), surf(ia1, io1), surf(ia1, io0)]
            top = [surf(ia0, io0, LIFT), surf(ia0, io1, LIFT), surf(ia1, io1, LIFT),
                   surf(ia1, io0, LIFT)]
            ctr = surf((la0 + la1) / 2, (lo0 + lo1) / 2)
            out = L.norm(ctr)
            for i in range(4):
                j = (i + 1) % 4
                g.face([outer[i], outer[j], inner[j], inner[i]], "glow_white", want=out)
                mid = L.mul(L.add(inner[i], inner[j]), 0.5)
                g.face([inner[i], inner[j], top[j], top[i]], "body",
                       want=L.add(L.norm(L.sub(mid, surf((ia0 + ia1) / 2, (io0 + io1) / 2))),
                                  L.mul(out, 0.3)))
            g.face(top, "body", want=out)
    # Raised glowing waist ring.
    n = COLS * 2
    rings = [[surf(la, i * 360.0 / n, s) for i in range(n)]
             for la, s in ((-WAIST, 1.0), (-WAIST, 1.07), (WAIST, 1.07), (WAIST, 1.0))]
    L.loft(g, rings, L.Y, mats="glow_white")
    # Polar caps.
    top_y = RY * math.sin(math.radians(ROWS_UP[-1]))
    g.face([surf(ROWS_UP[-1], i * dlon) for i in range(COLS)], "body", want=L.Y)
    g.face([surf(-ROWS_UP[-1], i * dlon) for i in range(COLS)], "body", want=L.NY)
    # Fuze: neck, head, cap.

    def oct_ring(y, r):
        return [(r * math.sin(math.radians(22.5 + 45 * i)), y, r * math.cos(math.radians(22.5 + 45 * i)))
                for i in range(8)]

    L.loft(g, [oct_ring(top_y - 0.002, 0.011), oct_ring(0.058, 0.011), oct_ring(0.058, 0.0145),
               oct_ring(0.071, 0.0145), oct_ring(0.073, 0.012), oct_ring(0.073, 0.008),
               oct_ring(0.0755, 0.008)], L.Y, cap1="body")
    L.loft(g, [oct_ring(0.0615, 0.0152), oct_ring(0.0645, 0.0152)], L.Y, mats="glow_white")
    # Spoon (lever) down the +Z side, hinged on the fuze head.
    L.band_x(g, [(0.010, 0.0705), (0.021, 0.069), (0.034, 0.055), (0.045, 0.032),
                 (0.0505, 0.008), (0.050, -0.012), (0.046, -0.024)], 0.0025, -0.006, 0.006)
    # Pin through the head along X and the pull ring on the +X side.
    pin = [[(x, 0.064 + 0.0012 * math.sin(a), 0.0012 * math.cos(a)) for a in
            (math.radians(45 + 90 * k) for k in range(4))] for x in (-0.017, 0.022)]
    L.loft(g, pin, L.X, cap0="body", cap1="body")
    rc, rr, rt, seg = (0.034, 0.058, 0.0), 0.012, 0.0018, 12
    tor = []
    for i in range(seg + 1):
        a = 2 * math.pi * i / seg
        cdir = (math.cos(a), math.sin(a), 0.0)
        cen = L.add(rc, L.mul(cdir, rr))
        tor.append([L.add(cen, L.add(L.mul(cdir, rt * math.cos(b)), (0, 0, rt * math.sin(b))))
                    for b in (math.radians(45 + 90 * k) for k in range(4))])
    for i in range(seg):
        for k in range(4):
            q = [tor[i][k], tor[i + 1][k], tor[i + 1][(k + 1) % 4], tor[i][(k + 1) % 4]]
            cen = L.add(rc, L.mul((math.cos(2 * math.pi * (i + 0.5) / seg),
                                   math.sin(2 * math.pi * (i + 0.5) / seg), 0), rr))
            mid = L.mul(tuple(map(sum, zip(*q))), 0.25)
            g.face(q, "body", want=L.sub(mid, cen))
    return g


def main():
    L.reset_scene()
    L.standard_materials()
    L.material("glow_white", base=(1, 1, 1), emit=(1, 1, 1), strength=4.0)
    g = build()
    L.make_object("grenade", g)
    # drop the unused team materials so only the grenade's own are exported
    for name in ("accent", "glow"):
        m = L.bpy.data.materials.get(name)
        if m and m.users == 0:
            L.bpy.data.materials.remove(m)
    print("grenade %d tris" % g.tris())
    L.save_blend("assets-src/blend/grenade.blend")
    L.export_glb("assets/models/grenade.glb")


main()
