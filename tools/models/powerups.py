"""Builds the powerups: assets-src/blend/powerups.blend and assets/models/powerups.glb.

    blender -b --factory-startup --python-exit-code 1 --python tools/models/powerups.py

Meshes (game frame, Y up, metres):
- `rapid`: two stacked chevrons pointing up, ~0.34 m wide, ~0.36 m tall, origin at the centre.
- `shield`: a heater shield facing -Z, ~0.34 m wide, ~0.42 m tall, origin at the centre.
- `cloak`: a cape hanging from a collar, folds towards the hem, facing -Z (its outside), ~0.6 m
  wide at the hem and ~0.7 m tall, origin at the centre.
- `speed`: a chevron pointing +X (running) with three speed lines trailing it, ~0.4 m wide,
  ~0.32 m tall, origin near the centre.
- `charge`: a lightning bolt (the charge gun), ~0.26 m wide, ~0.46 m tall, origin near the
  centre.
- `barrier`: a wall of hard light between two posts: a tall panel with a glowing face and a
  band across it, ~0.4 m wide, ~0.34 m tall, origin at the centre.
- `ricochet`: a bolt's zigzag path bouncing between two walls, ~0.38 m wide, ~0.4 m tall,
  origin at the centre.
- `shotgun`: the pump shotgun a player holds, in the blaster's frame (barrel along -Z, up +Y,
  origin at the middle of the grip hold), with `hit_1`/`hit_2`/`hit_3` lights on top of the
  receiver (hit_1 nearest the player) and a `muzzle` disc; the empty `shotgun_muzzle` marks the
  barrel tip.
Materials: `body`, `accent` (the powerup's colour, or the team's on the shotgun), and the
shotgun's `hit_N` and `muzzle`.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ltlib as L  # noqa: E402
from ltlib import box, loft  # noqa: E402

BARREL_Y = 0.098
TIP_Z = -0.56


def slab(g, poly, z0, z1, mat="body", front_mat=None, back_mat=None):
    """A convex (x, y) polygon extruded from z0 (front, -Z side) to z1."""
    n = len(poly)
    cx = sum(p[0] for p in poly) / n
    cy = sum(p[1] for p in poly) / n
    for i in range(n):
        a, b = poly[i], poly[(i + 1) % n]
        mid = ((a[0] + b[0]) / 2 - cx, (a[1] + b[1]) / 2 - cy, 0)
        g.face([(a[0], a[1], z0), (b[0], b[1], z0), (b[0], b[1], z1), (a[0], a[1], z1)], mat,
               want=mid)
    g.face([(x, y, z0) for x, y in poly], front_mat or mat, want=L.NZ)
    g.face([(x, y, z1) for x, y in poly], back_mat or mat, want=L.Z)


def inset(poly, d):
    """A convex polygon shrunk towards its centroid by about d."""
    n = len(poly)
    cx = sum(p[0] for p in poly) / n
    cy = sum(p[1] for p in poly) / n
    out = []
    for x, y in poly:
        dx, dy = x - cx, y - cy
        r = math.hypot(dx, dy)
        k = max(r - d, 0.001) / r
        out.append((cx + dx * k, cy + dy * k))
    return out


def chevron(g, cy, hw, drop, thick, depth):
    """A solid chevron pointing up, tip at (0, cy): two arms (convex quads), body with an
    accent face front and back."""
    tip_out = (0.0, cy)
    tip_in = (0.0, cy - thick)
    for s in (-1, 1):
        arm = [tip_out, (s * hw, cy - drop), (s * hw, cy - drop - thick), tip_in]
        if s < 0:
            arm.reverse()
        slab(g, arm, -depth, depth, "body", front_mat="accent", back_mat="accent")


def build_rapid():
    g = L.Geo()
    for cy in (0.18, 0.04):
        chevron(g, cy, 0.17, 0.12, 0.07, 0.03)
    return g


def shield_outline(w=0.17, top=0.21, shoulder=0.07, bottom=-0.21):
    pts = [(-w, top), (w, top)]
    # Sides curving in to the point.
    k = 8
    right = []
    for i in range(1, k):
        t = i / k
        y = shoulder + (bottom - shoulder) * t
        x = w * math.cos(t * math.pi / 2) ** 0.8
        right.append((x, y))
    pts = [(-w, top), (-w, shoulder)] + [(-x, y) for x, y in right] + [(0.0, bottom)]
    pts += [(x, y) for x, y in reversed(right)] + [(w, shoulder), (w, top)]
    return list(reversed(pts)) if L.signed_area(pts) < 0 else pts


def build_shield():
    g = L.Geo()
    outer = shield_outline()
    # A thick plate with a bevelled front rim (accent) and a raised boss.
    slab(g, outer, -0.01, 0.025)
    rim = inset(outer, 0.03)
    n = len(outer)
    for i in range(n):
        j = (i + 1) % n
        q = [(outer[i][0], outer[i][1], -0.01), (outer[j][0], outer[j][1], -0.01),
             (rim[j][0], rim[j][1], -0.03), (rim[i][0], rim[i][1], -0.03)]
        g.face(q, "accent", want=L.NZ)
    g.face([(x, y, -0.03) for x, y in rim], "body", want=L.NZ)
    # Emblem: a vertical bar and a diamond boss.
    box(g, (-0.015, -0.13, -0.04), (0.015, 0.15, -0.03), mat="accent", skip=("+z",))
    d = [(0.0, 0.08), (0.06, 0.01), (0.0, -0.06), (-0.06, 0.01)]
    slab(g, list(reversed(d)), -0.05, -0.03, "body", front_mat="accent")
    return g


def build_cloak():
    g = L.Geo()
    cols, rows = 12, 10
    top_w, hem_w, height = 0.17, 0.32, 0.7
    thick = 0.012

    def surf(u, v):
        # u -1..1 across, v 0 (collar) .. 1 (hem); the cape curves round the back (+Z) and
        # falls in folds that deepen towards the hem.
        w = top_w + (hem_w - top_w) * max(v, 0.0) ** 0.8
        x = u * w
        y = height / 2 - v * height
        z = 0.08 * (1 - u * u) * (0.5 + 0.5 * v) + 0.025 * v * math.sin(u * math.pi * 3.0)
        return (x, y, z)

    def normal(u, v):
        e = 1e-3
        a, b = surf(u - e, v), surf(u + e, v)
        c, d = surf(u, v - e), surf(u, v + e)
        return L.norm(L.cross(L.sub(b, a), L.sub(d, c)))

    grid = [[surf(-1 + 2 * i / cols, j / rows) for i in range(cols + 1)] for j in range(rows + 1)]
    # The outside faces -Z (towards the viewer of the pickup), the inside +Z.
    off = lambda i, j, s: L.add(grid[j][i], L.mul(normal(-1 + 2 * i / cols, j / rows), s))
    for j in range(rows):
        for i in range(cols):
            out = [off(i, j, -thick / 2), off(i + 1, j, -thick / 2), off(i + 1, j + 1, -thick / 2),
                   off(i, j + 1, -thick / 2)]
            inn = [off(i, j, thick / 2), off(i + 1, j, thick / 2), off(i + 1, j + 1, thick / 2),
                   off(i, j + 1, thick / 2)]
            n_out = L.mul(normal(-1 + 2 * (i + 0.5) / cols, (j + 0.5) / rows), -1)
            mat = "accent" if j == rows - 1 else "body"
            g.face(out, mat, want=n_out)
            g.face(inn, "body", want=L.mul(n_out, -1))
    # Edges.
    for j in range(rows):
        for i, s in ((0, -1), (cols, 1)):
            q = [off(i, j, -thick / 2), off(i, j + 1, -thick / 2), off(i, j + 1, thick / 2),
                 off(i, j, thick / 2)]
            g.face(q, "accent", want=(s, 0, 0))
    for j, s in ((0, 1), (rows, -1)):
        for i in range(cols):
            q = [off(i, j, -thick / 2), off(i + 1, j, -thick / 2), off(i + 1, j, thick / 2),
                 off(i, j, thick / 2)]
            g.face(q, "accent" if j == rows else "body", want=(0, s, 0))
    # Collar: a band over the top, and a round clasp at each front corner.
    y0 = height / 2
    box(g, (-top_w - 0.01, y0 - 0.03, -0.02), (top_w + 0.01, y0 + 0.02, 0.1), mat="body")
    box(g, (-top_w - 0.012, y0 - 0.02, -0.022), (top_w + 0.012, y0 + 0.004, 0.102), mat="accent",
        skip=("+y", "-y"))
    for s in (-1, 1):
        c = (s * (top_w - 0.02), y0 - 0.01, -0.03)
        ring = [(c[0] + 0.03 * math.cos(a), c[1] + 0.03 * math.sin(a))
                for a in (2 * math.pi * k / 8 for k in range(8))]
        slab(g, ring, -0.045, -0.02, "body", front_mat="accent")
    return g


def build_speed():
    g = L.Geo()
    tip, hh, drop, thick, depth = 0.17, 0.16, 0.12, 0.07, 0.03
    for s in (-1, 1):
        arm = [(tip, 0.0), (tip - drop, s * hh), (tip - drop - thick, s * hh), (tip - thick, 0.0)]
        if L.signed_area(arm) < 0:
            arm.reverse()
        slab(g, arm, -depth, depth, "body", front_mat="accent", back_mat="accent")
    for y, x0 in ((0.08, -0.2), (0.0, -0.25), (-0.08, -0.2)):
        box(g, (x0, y - 0.016, -0.022), (-0.05, y + 0.016, 0.022), mat="accent")
    return g


def build_charge():
    g = L.Geo()
    # Two convex slabs: the upper stroke slanting down to the left, the lower one from a step
    # to the right down to the point.
    upper = [(-0.04, 0.23), (0.12, 0.23), (0.02, 0.01), (-0.13, 0.01)]
    lower = [(-0.05, 0.05), (0.13, 0.05), (-0.09, -0.23)]
    for poly in (upper, lower):
        if L.signed_area(poly) < 0:
            poly.reverse()
        slab(g, poly, -0.03, 0.03, "body", front_mat="accent", back_mat="accent")
    return g


def build_barrier():
    g = L.Geo()
    # The panel: a body slab with an accent face front and back, between two posts.
    slab(g, [(-0.16, -0.15), (0.16, -0.15), (0.16, 0.15), (-0.16, 0.15)], -0.015, 0.015, "body",
         front_mat="accent", back_mat="accent")
    for s in (-1, 1):
        box(g, (s * 0.2 - 0.03, -0.17, -0.03), (s * 0.2 + 0.03, 0.17, 0.03), mat="body")
        box(g, (s * 0.2 - 0.032, 0.12, -0.032), (s * 0.2 + 0.032, 0.15, 0.032), mat="accent")
    # A band across the middle (body, standing out of the glow).
    box(g, (-0.17, -0.02, -0.022), (0.17, 0.02, 0.022), mat="body")
    return g


def stroke(g, a, b, w, depth, mat="accent"):
    """A straight bar of width w from a to b (x, y), as a convex slab."""
    dx, dy = b[0] - a[0], b[1] - a[1]
    r = math.hypot(dx, dy)
    nx, ny = -dy / r * w / 2, dx / r * w / 2
    poly = [(a[0] + nx, a[1] + ny), (a[0] - nx, a[1] - ny), (b[0] - nx, b[1] - ny),
            (b[0] + nx, b[1] + ny)]
    if L.signed_area(poly) < 0:
        poly.reverse()
    slab(g, poly, -depth, depth, mat)


def build_ricochet():
    g = L.Geo()
    # Two walls (body), and the path between them: in from the top left, three bounces.
    for s in (-1, 1):
        box(g, (s * 0.19 - 0.02, -0.2, -0.04), (s * 0.19 + 0.02, 0.2, 0.04), mat="body")
    pts = [(-0.1, 0.2), (0.16, 0.09), (-0.16, -0.03), (0.16, -0.15), (0.0, -0.21)]
    for a, b in zip(pts, pts[1:]):
        stroke(g, a, b, 0.045, 0.025)
    # A spark at each bounce.
    for x, y in pts[1:4]:
        ring = [(x + 0.035 * math.cos(t), y + 0.035 * math.sin(t))
                for t in (2 * math.pi * k / 8 for k in range(8))]
        slab(g, ring, -0.035, 0.035, "accent")
    return g


def build_shotgun():
    g = L.Geo()

    def octagon_ring(z, apothem, yc):
        r = apothem / math.cos(math.pi / 8)
        return [(r * math.cos(math.pi / 8 + i * math.pi / 4), yc + r * math.sin(math.pi / 8 + i * math.pi / 4),
                 z) for i in range(8)]

    # Receiver: a slab from the stock to the barrels.
    xh = 0.022
    prof = [(-0.13, 0.05), (0.06, 0.05), (0.07, 0.065), (0.07, 0.112), (0.06, 0.122),
            (-0.12, 0.122), (-0.13, 0.112)]
    L.prism_x(g, list(reversed(prof)), -xh, xh)
    # Hit lights along the top, hit_1 nearest the player.
    for mat, zc in (("hit_3", -0.085), ("hit_2", -0.04), ("hit_1", 0.005)):
        box(g, (-0.014, 0.121, zc - 0.015), (0.014, 0.129, zc + 0.015), mat=mat, skip=("-y",))
    # Team trim along the receiver's sides.
    for s in (-1, 1):
        x0, x1 = sorted((s * xh, s * (xh + 0.0015)))
        box(g, (x0, 0.07, -0.125), (x1, 0.078, 0.06), mat="accent")
        box(g, (x0, 0.095, -0.125), (x1, 0.1, 0.06), mat="accent")
    # Barrel, with the muzzle emitter.
    loft(g, [octagon_ring(-0.13, 0.017, BARREL_Y), octagon_ring(TIP_Z + 0.02, 0.017, BARREL_Y),
             octagon_ring(TIP_Z + 0.02, 0.021, BARREL_Y), octagon_ring(TIP_Z, 0.021, BARREL_Y),
             octagon_ring(TIP_Z, 0.013, BARREL_Y), octagon_ring(TIP_Z - 0.002, 0.013, BARREL_Y)],
         L.NZ, mats=["body", "body", "body", "body", "muzzle"], cap0="body", cap1="muzzle")
    loft(g, [octagon_ring(TIP_Z + 0.045, 0.0185, BARREL_Y), octagon_ring(TIP_Z + 0.055, 0.0185, BARREL_Y)],
         L.NZ, mats="accent")
    # Magazine tube under the barrel and the pump on it.
    my = 0.06
    loft(g, [octagon_ring(-0.13, 0.014, my), octagon_ring(TIP_Z + 0.07, 0.014, my)], L.NZ,
         cap1="body")
    pump = [octagon_ring(-0.24, 0.024, my), octagon_ring(-0.25, 0.028, my),
            octagon_ring(-0.38, 0.028, my), octagon_ring(-0.39, 0.024, my)]
    loft(g, pump, L.NZ, cap0="body", cap1="body")
    for z in (-0.28, -0.31, -0.34):
        loft(g, [octagon_ring(z, 0.0295, my), octagon_ring(z - 0.008, 0.0295, my)], L.NZ,
             mats="accent")
    # Stock: tapering back and down from the receiver.
    L.prism_x(g, [(0.06, 0.122), (0.06, 0.05), (0.27, -0.0), (0.29, 0.02), (0.29, 0.095)],
              -0.018, 0.018)
    box(g, (-0.0195, 0.0, 0.27), (0.0195, 0.098, 0.292), mat="accent", skip=("-z",))
    # Grip, as the blaster's: tilted octagonal section.
    tilt = math.radians(22)
    a = (0, math.cos(tilt), -math.sin(tilt))
    d = (0, math.sin(tilt), math.cos(tilt))

    def section(t, hx=0.015, hd=0.022, ch=0.006):
        c = L.mul(a, t)
        pts = [(hx, hd - ch), (hx - ch, hd), (-hx + ch, hd), (-hx, hd - ch),
               (-hx, -hd + ch), (-hx + ch, -hd), (hx - ch, -hd), (hx, -hd + ch)]
        return [L.add(c, L.add((x, 0, 0), L.mul(d, dd))) for x, dd in pts]

    loft(g, [section(-0.07, 0.017, 0.025), section(-0.062), section(0.06)], a, cap0="body",
         cap1="body")
    # Trigger guard and trigger.
    L.band_x(g, [(-0.08, 0.052), (-0.082, 0.024), (-0.072, 0.004), (-0.05, -0.004),
                 (-0.02, -0.006)], 0.005, -0.0045, 0.0045)
    L.band_x(g, [(-0.056, 0.052), (-0.059, 0.036), (-0.055, 0.022), (-0.048, 0.014)], 0.006,
             -0.0035, 0.0035)
    return g


def main():
    L.reset_scene()
    L.standard_materials()
    for m in ("hit_1", "hit_2", "hit_3"):
        L.material(m, base=(1, 1, 1), emit=(1, 1, 1), strength=3.0)
    L.material("muzzle", base=(1, 1, 1), emit=(1, 1, 1), strength=5.0)
    for name, build in (("rapid", build_rapid), ("shield", build_shield), ("cloak", build_cloak),
                        ("shotgun", build_shotgun), ("speed", build_speed),
                        ("charge", build_charge), ("barrier", build_barrier),
                        ("ricochet", build_ricochet)):
        g = build()
        L.make_object(name, g)
        print("powerup %s %d tris" % (name, g.tris()))
    L.make_empty("shotgun_muzzle", (0.0, BARREL_Y, TIP_Z))
    m = L.bpy.data.materials.get("glow")
    if m and m.users == 0:
        L.bpy.data.materials.remove(m)
    L.save_blend("assets-src/blend/powerups.blend")
    L.export_glb("assets/models/powerups.glb")


main()
