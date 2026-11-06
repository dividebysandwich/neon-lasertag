"""Builds the lasertag blaster: assets-src/blend/blaster.blend and assets/models/blaster.glb.

    blender -b --factory-startup --python-exit-code 1 --python tools/models/blaster.py

A low-poly light-gun pistol (long octagonal barrel, slab receiver with a ribbed top rib, angled
grooved grip, trigger and guard, front/rear sights). Game frame: barrel along -Z, up +Y,
origin at the centre of the hand's hold on the grip. Materials: `body`, `accent` (team trim),
`hit_1`/`hit_2`/`hit_3` (remaining-hit lights on the top rib, hit_1 nearest the player),
`muzzle` (emitter disc at the barrel tip). An empty `muzzle` marks the barrel tip.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ltlib as L  # noqa: E402
from ltlib import box, Frame, line, loft  # noqa: E402

BARREL_Y = 0.085  # barrel axis height above the grip centre
TIP_Z = -0.225  # barrel tip (front face of the muzzle shroud)
GRIP_TILT = math.radians(22)  # grip axis leans back from vertical


def octagon_ring(z, apothem, yc=BARREL_Y):
    r = apothem / math.cos(math.pi / 8)
    return [(r * math.cos(math.pi / 8 + i * math.pi / 4), yc + r * math.sin(math.pi / 8 + i * math.pi / 4), z)
            for i in range(8)]


def side_frames(xh):
    """Frames on the receiver's two side planes; author points as (z, y) via uv()."""
    return [(Frame((xh, 0, 0), L.NZ, L.Y), -1), (Frame((-xh, 0, 0), L.Z, L.Y), 1)]


def build():
    g = L.Geo()
    # Receiver: slab with chamfered front corners and a sloped rear.
    xh = 0.019
    prof = [(-0.095, 0.058), (-0.087, 0.05), (0.035, 0.05), (0.048, 0.065), (0.048, 0.1),
            (0.038, 0.112), (-0.085, 0.112), (-0.095, 0.104)]
    L.prism_x(g, list(reversed(prof)), -xh, xh)
    # Ribbed top rib above the rear: a wedge front, a block, hit bands around it.
    rh = 0.013
    L.prism_x(g, [(-0.062, 0.112), (-0.04, 0.112), (-0.04, 0.128)], -rh, rh, skip_edges=(0,))
    box(g, (-rh, 0.112, -0.04), (rh, 0.128, 0.036), skip=("-y",))
    for mat, zc in (("hit_3", -0.026), ("hit_2", -0.006), ("hit_1", 0.014)):
        box(g, (-rh - 0.0015, 0.1115, zc - 0.006), (rh + 0.0015, 0.1295, zc + 0.006), mat=mat,
            skip=("-y",))
    # Rear sight (notch between two posts) on the rib, front sight on the shroud.
    for s in (-1, 1):
        xa, xb = sorted((s * 0.0035, s * 0.009))
        box(g, (xa, 0.128, 0.026), (xb, 0.136, 0.034), skip=("-y",))
    sh = 0.0185
    box(g, (-0.0015, BARREL_Y + sh, TIP_Z + 0.007), (0.0015, BARREL_Y + sh + 0.009, TIP_Z + 0.017),
        skip=("-y",))
    # Barrel: collar at the receiver, long octagon, a team ring, the muzzle shroud and emitter.
    rings = [octagon_ring(-0.09, 0.021), octagon_ring(-0.105, 0.021), octagon_ring(-0.105, 0.016),
             octagon_ring(-0.192, 0.016), octagon_ring(-0.192, sh), octagon_ring(TIP_Z, sh),
             octagon_ring(TIP_Z, 0.0115), octagon_ring(TIP_Z - 0.002, 0.0115)]
    loft(g, rings, L.NZ, mats=["body", "body", "body", "body", "body", "body", "muzzle"],
         cap0="body", cap1="muzzle")
    loft(g, [octagon_ring(-0.168, 0.0172), octagon_ring(-0.178, 0.0172)], L.NZ, mats="accent")
    loft(g, [octagon_ring(-0.140, 0.0172), octagon_ring(-0.144, 0.0172)], L.NZ, mats="accent")
    # Grip: tilted octagonal section with horizontal finger grooves (depth only, so the sides
    # stay flat for the trim line) and a flared butt.
    a = (0, math.cos(GRIP_TILT), -math.sin(GRIP_TILT))  # up the grip
    d = (0, math.sin(GRIP_TILT), math.cos(GRIP_TILT))  # towards the back of the grip

    def section(t, hx=0.015, hd=0.022, ch=0.006):
        c = L.mul(a, t)
        pts = [(hx, hd - ch), (hx - ch, hd), (-hx + ch, hd), (-hx, hd - ch),
               (-hx, -hd + ch), (-hx + ch, -hd), (hx - ch, -hd), (hx, -hd + ch)]
        return [L.add(c, L.add((x, 0, 0), L.mul(d, dd))) for x, dd in pts]

    grip = [section(-0.074, 0.017, 0.025), section(-0.066, 0.017, 0.025),
            section(-0.064, 0.015, 0.022)]
    for k in range(5):
        t0 = -0.052 + k * 0.016
        grip += [section(t0), section(t0 + 0.002, hd=0.0195), section(t0 + 0.008, hd=0.0195),
                 section(t0 + 0.010)]
    grip.append(section(0.066))
    loft(g, grip, a, cap0="body", cap1="body")
    # Team trim: receiver side lines and a line down each grip side.
    for f, sz in side_frames(xh):
        uv = lambda pts: [(sz * z, y) for z, y in pts]
        L.strip(g, f, uv([(-0.088, 0.07), (-0.074, 0.084), (0.03, 0.084), (0.04, 0.074)]), 0.006,
                glow=(0, 0), h=0.0015)
        L.strip(g, f, uv([(-0.05, 0.062), (0.02, 0.062)]), 0.004, glow=(0, 0), h=0.0015)
    for s in (-1, 1):
        f = Frame((s * 0.015, 0, 0), L.mul(d, -s), a)  # N = U x V points to s*X
        line(g, f, (0, -0.05), (0, 0.045), 0.005, glow=0, h=0.0015)
    # Trigger guard and trigger.
    gt = 0.0045
    L.band_x(g, [(-0.08, 0.052), (-0.082, 0.024), (-0.072, 0.004), (-0.05, -0.004),
                 (-0.02, -0.006)], 0.005, -gt, gt)
    L.band_x(g, [(-0.056, 0.052), (-0.059, 0.036), (-0.055, 0.022), (-0.048, 0.014)], 0.006,
             -0.0035, 0.0035)
    return g


def main():
    L.reset_scene()
    L.standard_materials()
    L.material("hit_1", base=(1, 1, 1), emit=(1, 1, 1), strength=3.0)
    L.material("hit_2", base=(1, 1, 1), emit=(1, 1, 1), strength=3.0)
    L.material("hit_3", base=(1, 1, 1), emit=(1, 1, 1), strength=3.0)
    L.material("muzzle", base=(1, 1, 1), emit=(1, 1, 1), strength=5.0)
    g = build()
    L.make_object("blaster", g)
    L.make_empty("muzzle", (0.0, BARREL_Y, TIP_Z))
    print("blaster %d tris, muzzle at %s" % (g.tris(), (0.0, BARREL_Y, TIP_Z)))
    L.save_blend("assets-src/blend/blaster.blend")
    L.export_glb("assets/models/blaster.glb")


main()
