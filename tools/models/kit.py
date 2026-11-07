"""Builds the modular arena kit: assets-src/blend/kit.blend and assets/models/kit.glb.

    blender -b --factory-startup --python-exit-code 1 --python tools/models/kit.py

Dimensions come from tools/models/kit_dims.json (generated from crates/lt-sim/src/kit.rs).
One mesh per piece, named exactly the piece name, in the piece's own frame (game coords: Y up,
origin at the centre of its footprint on the floor, forward +Z, X mirror symmetric). Materials:
`body` (black structure), `accent` (neon strips, recoloured per team in game), `glow` (halo
quads; UV V = 0 at the strip edge, 1 at the outer edge).
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ltlib as L  # noqa: E402
from ltlib import (box, box_face, chamfer_rect, chevron, diamond, filled, Frame, hexagon,  # noqa: E402
                   line, ring, strip)

DIMS = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "kit_dims.json")))
P = DIMS["pieces"]
C = DIMS["constants"]

W = 0.04  # standard strip width


def bounds(name):
    return tuple(P[name]["bounds"][0]), tuple(P[name]["bounds"][1])


# ---------------------------------------------------------------------------------------------
# Shared decorations.

def edge_grid(geo, f, hu, hv, half_w=0.02, glow_in=0.08, marks=True, mark_w=0.02, mark_glow=0.04):
    """Floor-style grid: a line of half width along each edge (neighbours complete it), plus
    small L corner marks."""
    ring(geo, f, L.rect(-hu + half_w / 2, -hv + half_w / 2, hu - half_w / 2, hv - half_w / 2),
         half_w, glow_in=glow_in, glow_out=0)
    if marks:
        for su in (-1, 1):
            for sv in (-1, 1):
                a, b = 0.84, 0.68
                strip(geo, f, [(su * hu * a, sv * hv * b), (su * hu * a, sv * hv * a),
                               (su * hu * b, sv * hv * a)], mark_w, glow=(mark_glow, mark_glow))


def edge_band(geo, f, hu, hv, w=W):
    """A neon band right on the edges of a face: square corners, its outer side flush with the
    face's edges, no halo. Bands on neighbouring faces meet at the corner, so the edge itself
    is lit."""
    ring(geo, f, L.rect(-hu + w / 2, -hv + w / 2, hu - w / 2, hv - w / 2), w, glow_in=0,
         glow_out=0)


def edge_bands(geo, mn, mx, sides=("+x", "-x", "+y", "+z", "-z"), w=W):
    """Edge bands on the given faces of a box."""
    for side in sides:
        f, hu, hv = box_face(mn, mx, side)
        edge_band(geo, f, hu, hv, w)


def corner_lines(geo, hx, hz, c, y0, y1, w=0.022):
    """Light lines along the chamfered vertical corners of a post-like prism (no halo: they are
    its edges)."""
    for sx in (-1, 1):
        for sz in (-1, 1):
            a = (sx * hx, sz * (hz - c))
            b = (sx * (hx - c), sz * hz)
            mid = ((a[0] + b[0]) / 2, (y0 + y1) / 2, (a[1] + b[1]) / 2)
            n = L.norm((sx, 0, sz))
            U = L.cross(L.Y, n)
            f = Frame(mid, U, L.Y)
            hl = (y1 - y0) / 2
            strip(geo, f, [(0, -hl), (0, hl)], w, glow=(0, 0))


def bands(geo, hx, hz, c, ys, w=0.03, g=0.06, top=None):
    """Horizontal light bands around a chamfered prism (one strip per flat face)."""
    for side in ("+x", "-x", "+z", "-z"):
        f, hu, _ = box_face((-hx, 0, -hz), (hx, top or 1, hz), side)
        span = hu - c - 0.012
        for y in ys:
            fy = Frame(L.add(f.o, (0, y - f.o[1], 0)), f.U, f.V)
            line(geo, fy, (-span, 0), (span, 0), w, glow=g)


def top_cap(geo, hx, hz, y, inset):
    f = Frame((0, y, 0), L.X, L.NZ)
    ring(geo, f, chamfer_rect(-hx + inset, -hz + inset, hx - inset, hz - inset, inset * 0.8),
         0.02, glow_in=0.03, glow_out=0)


# ---------------------------------------------------------------------------------------------
# Pieces.

def floor_tile():
    g = L.Geo()
    mn, mx = bounds("floor_tile")
    box(g, mn, mx, skip=("-y",))
    f, hu, hv = box_face(mn, mx, "+y")
    # Thin, faint lines: the floor is the backdrop, not the show.
    edge_grid(g, f, hu, hv, half_w=0.009, glow_in=0.035, mark_w=0.01, mark_glow=0.02)
    return g


def upper_tile():
    g = L.Geo()
    mn, mx = bounds("upper_tile")
    box(g, mn, mx)
    f, hu, hv = box_face(mn, mx, "+y")
    edge_grid(g, f, hu, hv, half_w=0.009, glow_in=0.035, mark_w=0.01, mark_glow=0.02)
    f, hu, hv = box_face(mn, mx, "-y")
    edge_grid(g, f, hu, hv, glow_in=0.1, marks=False)
    filled(g, f, hexagon(0, 0, 0.16), glow=0.08)
    ring(g, f, hexagon(0, 0, 0.36), 0.03, glow_in=0.07, glow_out=0.07)
    for side in ("+x", "-x", "+z", "-z"):
        f, hu, hv = box_face(mn, mx, side)
        line(g, f, (-hu + 0.06, 0), (hu - 0.06, 0), 0.025, glow=0.06)
    return g


def wall_full():
    g = L.Geo()
    mn, mx = bounds("wall_full")
    box(g, mn, mx, skip=("-y",))
    edge_bands(g, mn, mx)
    for side in ("+z", "-z"):
        f, hu, hv = box_face(mn, mx, side)  # v = y - 1.5
        hc = 1.75 - hv
        ring(g, f, hexagon(0, hc, 0.42), 0.05, glow_in=0.09, glow_out=0.10)
        ring(g, f, hexagon(0, hc, 0.24), 0.03, glow_in=0.06, glow_out=0.05)
        filled(g, f, hexagon(0, hc, 0.08), glow=0.05)
        line(g, f, (-0.86, 0.9 - hv), (0.86, 0.9 - hv), W, glow=0.08)
        for s in (-1, 1):
            line(g, f, (s * 0.62, 1.15 - hv), (s * 0.62, 2.35 - hv), 0.03, glow=0.07)
            # chevrons pointing up from the band towards the hexagon
            strip(g, f, chevron(s * 0.3, 0.55 - hv, 0.14, 0.12), 0.03, glow=(0.06, 0.06))
    return g


def wall_window():
    g = L.Geo()
    mn, mx = bounds("wall_window")
    w2 = C["WINDOW_W"] / 2
    y0, y1 = C["WINDOW_Y0"], C["WINDOW_Y1"]
    L.slab_with_hole(g, mn, mx, ((-w2, y0), (w2, y1)), skip=("-y",))
    edge_bands(g, mn, mx)
    t = mx[2]
    for side in ("+z", "-z"):
        f, hu, hv = box_face(mn, mx, side)
        # Hexagonal frame around the opening (the opening itself stays the collider rectangle).
        e = 0.065
        a0, a1 = y0 - hv - e, y1 - hv + e
        mid = (a0 + a1) / 2
        frame = [(-w2 - e, a0), (w2 + e, a0), (w2 + 0.2, mid), (w2 + e, a1), (-w2 - e, a1),
                 (-w2 - 0.2, mid)]
        ring(g, f, frame, W, glow_in=0.03, glow_out=0.10)
        # Below the window: band with a diamond; above: small hexagon and slits.
        dv = 0.55 - hv
        line(g, f, (-0.86, dv), (-0.22, dv), W, glow=0.08)
        line(g, f, (0.22, dv), (0.86, dv), W, glow=0.08)
        ring(g, f, diamond(0, dv, 0.17), 0.03, glow_in=0.05, glow_out=0.07)
        filled(g, f, diamond(0, dv, 0.06), glow=0.05)
        hv2 = 2.3 - hv
        ring(g, f, hexagon(0, hv2, 0.2), 0.035, glow_in=0.06, glow_out=0.08)
        for s in (-1, 1):
            line(g, f, (s * 0.38, hv2), (s * 0.72, hv2), 0.03, glow=0.07)
    # Lit reveals inside the opening.
    rw, rg = 0.03, 0.05
    sill = Frame((0, y0, 0), L.X, L.NZ)
    line(g, sill, (-w2 + 0.04, 0), (w2 - 0.04, 0), rw, glow=rg)
    head = Frame((0, y1, 0), L.X, L.Z)
    line(g, head, (-w2 + 0.04, 0), (w2 - 0.04, 0), rw, glow=rg)
    yc, hl = (y0 + y1) / 2, (y1 - y0) / 2 - 0.04
    for s in (-1, 1):
        jamb = Frame((s * w2, yc, 0), (0, 0, s), L.Y)  # N = U x Y faces into the opening
        line(g, jamb, (0, -hl), (0, hl), rw, glow=rg)
    assert t > 0
    return g


def wall_half():
    g = L.Geo()
    mn, mx = bounds("wall_half")
    box(g, mn, mx, skip=("-y",))
    edge_bands(g, mn, mx)
    for side in ("+z", "-z"):
        f, hu, hv = box_face(mn, mx, side)  # v = y - 0.6
        ring(g, f, diamond(0, 0, 0.3, 0.3), W, glow_in=0.07, glow_out=0.08)
        filled(g, f, diamond(0, 0, 0.1), glow=0.06)
        for s in (-1, 1):  # chevrons pointing in at the diamond
            for u in (0.48, 0.62, 0.76):
                strip(g, f, [(s * (u + 0.08), 0.15), (s * u, 0), (s * (u + 0.08), -0.15)], 0.03,
                      glow=(0.05, 0.05))
    return g


def railing():
    g = L.Geo()
    mn, mx = bounds("railing")
    t = mx[2]
    pt = 0.035
    box(g, (mn[0], 0, -t), (mx[0], 0.1, t), skip=("-y",))
    box(g, (mn[0], 0.1, -pt), (mx[0], 1.0, pt), skip=("-y", "+y"))
    box(g, (mn[0], 1.0, -t), (mx[0], mx[1], t))
    cap = ((mn[0], 1.0, -t), (mx[0], mx[1], t))
    f, hu, hv = box_face(*cap, "+y")
    line(g, f, (-hu + 0.04, 0), (hu - 0.04, 0), 0.03, glow=0.04)
    for side in ("+z", "-z"):
        f, hu, hv = box_face(*cap, side)
        line(g, f, (-hu + 0.04, 0), (hu - 0.04, 0), 0.02, glow=0.03)
        f, hu, hv = box_face((mn[0], 0, -t), (mx[0], 0.1, t), side)
        line(g, f, (-hu + 0.06, 0), (hu - 0.06, 0), 0.02, glow=0.03)
        f, hu, hv = box_face((mn[0], 0.1, -pt), (mx[0], 1.0, pt), side)  # v = y - 0.55
        ring(g, f, chamfer_rect(-hu + 0.07, -hv + 0.07, hu - 0.07, hv - 0.07, 0.08), 0.03,
             glow_in=0.07, glow_out=0.04)
        for u in (-0.6, -0.3, 0.0, 0.3, 0.6):
            line(g, f, (u, -0.18), (u, 0.18), 0.03, glow=0.06)
    for side in ("+x", "-x"):
        f, hu, hv = box_face(*cap, side)
        line(g, f, (-hu + 0.02, 0), (hu - 0.02, 0), 0.02, glow=0)
    return g


def post(name, bands_at):
    g = L.Geo()
    mn, mx = bounds(name)
    hx, hz, top = mx[0], mx[2], mx[1]
    c = 0.035
    L.prism_y(g, L.octagon_xz(hx, hz, c), 0, top)
    corner_lines(g, hx, hz, c, 0.06, min(top - 0.06, 2.74))
    bands(g, hx, hz, c, bands_at, w=0.025, g=0.05, top=top)
    top_cap(g, hx, hz, top, 0.06)
    return g


def pillar():
    g = L.Geo()
    mn, mx = bounds("pillar")
    hx, hz, top = mx[0], mx[2], mx[1]
    c = 0.06
    L.prism_y(g, L.octagon_xz(hx, hz, c), 0, top)
    corner_lines(g, hx, hz, c, 0.08, 2.72, w=0.03)
    bands(g, hx, hz, c, (0.16, 2.62), w=0.035, g=0.07, top=top)
    for side in ("+x", "-x", "+z", "-z"):
        f, hu, hv = box_face((-hx, 0, -hz), (hx, top, hz), side)  # v = y - 1.5
        for k, y in enumerate((1.2, 1.45, 1.7)):
            strip(g, f, chevron(0, y - hv, 0.17, 0.13), 0.045, glow=(0.07, 0.07))
        line(g, f, (0, 2.0 - hv), (0, 2.42 - hv), 0.04, glow=0.07)
        ring(g, f, diamond(0, 0.62 - hv, 0.13, 0.2), 0.035, glow_in=0.04, glow_out=0.07)
    top_cap(g, hx, hz, top, 0.08)
    return g


def block():
    g = L.Geo()
    mn, mx = bounds("block")
    hx, hz, top = mx[0], mx[2], mx[1]
    c = 0.06
    L.prism_y(g, L.octagon_xz(hx, hz, c), 0, top)
    corner_lines(g, hx, hz, c, 0.06, top - 0.06, w=0.03)
    for side in ("+x", "-x", "+z", "-z"):
        f, hu, hv = box_face((-hx, 0, -hz), (hx, top, hz), side)  # v = y - 0.6
        ring(g, f, diamond(0, 0, 0.27), 0.045, glow_in=0.07, glow_out=0.08)
        filled(g, f, diamond(0, 0, 0.09), glow=0.06)
        for s in (-1, 1):
            for k in (0.34, 0.44):  # chevrons pointing in at the diamond
                strip(g, f, [(s * (k + 0.1), 0.17), (s * k, 0), (s * (k + 0.1), -0.17)], 0.035,
                      glow=(0.05, 0.05))
        for v in (-0.48, 0.48):
            line(g, f, (-0.36, v), (0.36, v), 0.03, glow=0.06)
    f = Frame((0, top, 0), L.X, L.NZ)
    ring(g, f, chamfer_rect(-0.48, -0.48, 0.48, 0.48, 0.14), W, glow_in=0.08, glow_out=0.03)
    ring(g, f, hexagon(0, 0, 0.26), 0.04, glow_in=0.06, glow_out=0.08)
    filled(g, f, hexagon(0, 0, 0.08), glow=0.05)
    return g


def ramp():
    g = L.Geo()
    mn, mx = bounds("ramp")
    x0, x1 = mn[0], mx[0]
    z0, z1, y1 = mn[2], mx[2], mx[1]
    L.prism_x(g, [(z0, 0), (z1, 0), (z1, y1)], x0, x1, skip_edges=(0,))  # wedge, no bottom
    # Low curbs along the slope edges, tapering to nothing at the top so they stay in bounds.
    cw, ch = 0.1, 0.06
    prof = [(z0, 0), (z1, y1), (z0, ch)]
    for s in (-1, 1):
        xa, xb = sorted((s * x1, s * (x1 - cw)))
        L.prism_x(g, prof, xa, xb, skip_edges=(0,))
        top_dir = (0, y1 - ch, z1 - z0)
        f = Frame((s * (x1 - cw / 2), (ch + y1) / 2, 0), L.NX, top_dir)
        hl = L.length(top_dir) / 2
        line(g, f, (0, -hl + 0.04), (0, hl - 0.06), 0.03, glow=0, h=0.006)
    # Slope.
    up = (0, y1, z1 - z0)
    f = Frame((0, y1 / 2, 0), L.NX, up)  # N points up the normal of the slope
    hl = L.length(up) / 2
    for s in (-1, 1):
        u = s * (x1 - cw - 0.05)
        gl, gr = (0.1, 0.025) if s > 0 else (0.025, 0.1)
        strip(g, f, [(u, -hl + 0.08), (u, hl - 0.08)], W, glow=(gl, gr))
    for v in (-2.4, -1.2, 0.0, 1.2, 2.4):
        strip(g, f, chevron(0, v + 0.15, 0.42, 0.32), 0.06, glow=(0.1, 0.1))
    # Side faces: inset triangle outline and a diagonal slit pair.
    for side in ("+x", "-x"):
        fs, _, _ = box_face(mn, mx, side)
        fs = Frame((fs.o[0], 0, 0), fs.U, fs.V)
        tri = [(L.dot(L.sub(p, fs.o), fs.U), p[1]) for p in ((0, 0, z0), (0, 0, z1), (0, y1, z1))]
        tri = L.ccw(tri)
        inset = L.offset_line(tri, 0.14, True)
        ring(g, fs, inset, W, glow_in=0.09, glow_out=0.08)
        inner = L.offset_line(tri, 0.5, True)
        ring(g, fs, inner, 0.03, glow_in=0.06, glow_out=0.07)
    # Tall back face (under the upper floor's edge).
    f, hu, hv = box_face(mn, mx, "+z")
    edge_band(g, f, hu, hv)
    ring(g, f, hexagon(0, 0, 0.4), 0.05, glow_in=0.08, glow_out=0.1)
    filled(g, f, hexagon(0, 0, 0.1), glow=0.06)
    return g


def target_alcove():
    g = L.Geo()
    mn, mx = bounds("target_alcove")
    s = P["target_alcove"]["shapes"]
    back, left, right, roof = [(tuple(b["box"][0]), tuple(b["box"][1])) for b in s]
    wt = back[1][2] - back[0][2]
    ah = back[1][1]
    # Back wall between the side walls (no overlapping solids).
    bmn, bmx = (left[1][0], 0, back[0][2]), (right[0][0], ah, back[1][2])
    box(g, bmn, bmx, skip=("-y", "+y", "+x", "-x"))
    box(g, *left, skip=("-y", "+y"))
    box(g, *right, skip=("-y", "+y"))
    box(g, *roof)
    # Target panel: dark plate with glowing rim and a bold hexagonal bullseye.
    pmn, pmx = (tuple(DIMS["target_panel"][0]), tuple(DIMS["target_panel"][1]))
    box(g, pmn, pmx, skip=("-z",), side_mats={s: "accent" for s in ("+x", "-x", "+y", "-y")})
    f, hu, hv = box_face(pmn, pmx, "+z")
    ring(g, f, hexagon(0, 0, 0.45), 0.05, glow_in=0.05, glow_out=0.0)
    ring(g, f, hexagon(0, 0, 0.31), 0.045, glow_in=0.04, glow_out=0.04)
    ring(g, f, hexagon(0, 0, 0.18), 0.04, glow_in=0.03, glow_out=0.035)
    filled(g, f, hexagon(0, 0, 0.075), glow=0.03)
    # Halo on the back wall around the panel.
    fb, hu, hv = box_face(bmn, bmx, "+z")  # v = y - 1.4
    pr = L.rect(pmn[0], pmn[1] - fb.o[1], pmx[0], pmx[1] - fb.o[1])
    L.glow_band(g, fb, L.ccw(pr), True, 0.0, -0.14)
    # Back wall inside: border, chevrons pointing at the target.
    edge_band(g, fb, hu, hv)
    for y in (0.45, 0.7):
        strip(g, fb, chevron(0, y - hv, 0.3, 0.16), 0.045, glow=(0.07, 0.07))
    strip(g, fb, chevron(0, 2.2 - hv, 0.3, 0.16, up=False), 0.045, glow=(0.07, 0.07))
    # Side walls inside: chevrons pointing back (-Z) toward the target, plus level lines.
    for sx in (-1, 1):
        xin = left[1][0] if sx < 0 else right[0][0]
        zc = (back[1][2] + mx[2]) / 2
        hz = (mx[2] - back[1][2]) / 2
        fi = Frame((xin, ah / 2, zc), (0, 0, sx), L.Y)  # N = U x Y = (-sx, 0, 0): inward
        # u = sx * (z - zc): toward -Z is u direction -sx
        for zz in (-0.1, 0.25, 0.6):
            u = sx * (zz - zc)
            strip(g, fi, [(u + sx * 0.14, 1.65 - ah / 2), (u, 1.4 - ah / 2),
                          (u + sx * 0.14, 1.15 - ah / 2)], 0.04, glow=(0.06, 0.06))
        for y in (0.1, 2.6):
            line(g, fi, (-hz + 0.05, y - ah / 2), (hz - 0.05, y - ah / 2), 0.03, glow=0.06)
        # Front face of the side wall: vertical frame line around the opening.
        ff = Frame((sx * (mx[0] - wt / 2), ah / 2, mx[2]), L.X, L.Y)
        line(g, ff, (0, 0.06 - ah / 2), (0, ah / 2 - 0.06), 0.03, glow=0.06)
    f, hu, hv = box_face(*roof, "+z")
    line(g, f, (-hu + 0.06, 0), (hu - 0.06, 0), 0.03, glow=0.06)
    # Roof underside and top.
    fu = Frame((0, ah, (back[1][2] + mx[2]) / 2), L.X, L.Z)
    ring(g, fu, hexagon(0, 0, 0.34), 0.035, glow_in=0.07, glow_out=0.08)
    filled(g, fu, hexagon(0, 0, 0.09), glow=0.05)
    f, hu, hv = box_face(*roof, "+y")
    edge_grid(g, f, hu, hv)
    # Outside faces.
    f, hu, hv = box_face(mn, mx, "-z")
    edge_band(g, f, hu, hv)
    ring(g, f, hexagon(0, 0.2, 0.45), 0.05, glow_in=0.09, glow_out=0.1)
    ring(g, f, hexagon(0, 0.2, 0.25), 0.035, glow_in=0.06, glow_out=0.06)
    line(g, f, (-0.86, 0.9 - hv), (0.86, 0.9 - hv), W, glow=0.08)
    for side in ("+x", "-x"):
        f, hu, hv = box_face(mn, mx, side)
        edge_band(g, f, hu, hv)
        line(g, f, (-0.86, 0.9 - hv), (0.86, 0.9 - hv), W, glow=0.08)
        ring(g, f, diamond(0, 1.9 - hv, 0.3), 0.04, glow_in=0.07, glow_out=0.08)
    return g


def spawn_pad():
    g = L.Geo()
    mn, mx = bounds("spawn_pad")
    n = 24
    disc = [(p[0], -p[1]) for p in L.ngon(0, 0, mx[0], n, 0.0)]  # (x, z)
    L.prism_y(g, L.ccw([(x, z) for x, z in disc]), 0, mx[1])
    f = Frame((0, mx[1], 0), L.X, L.NZ)
    ring(g, f, L.ngon(0, 0, 0.9, n, 0.0), W, glow_in=0.07, glow_out=0.05)
    ring(g, f, L.ngon(0, 0, 0.62, n, 0.0), W, glow_in=0.07, glow_out=0.07)
    ring(g, f, hexagon(0, 0, 0.36), 0.035, glow_in=0.06, glow_out=0.07)
    filled(g, f, hexagon(0, 0, 0.14), glow=0.07)
    return g


def border_wall():
    """The arena's edge: nothing but the neon on its edges."""
    g = L.Geo()
    mn, mx = bounds("border_wall")
    box(g, mn, mx, skip=("-y",))
    edge_bands(g, mn, mx, w=0.05)
    return g


def border_post():
    """The arena's corners: lit corners and a band round the top edge, nothing else."""
    g = L.Geo()
    mn, mx = bounds("border_post")
    hx, hz, top = mx[0], mx[2], mx[1]
    c = 0.04
    L.prism_y(g, L.octagon_xz(hx, hz, c), 0, top)
    corner_lines(g, hx, hz, c, 0, top, w=0.025)
    w = 0.05
    f = Frame((0, top, 0), L.X, L.NZ)
    d = w / 2
    ring(g, f, chamfer_rect(-hx + d, -hz + d, hx - d, hz - d, c - d * (2 ** 0.5 - 1)), w,
         glow_in=0, glow_out=0)
    return g


BUILDERS = {
    "floor_tile": floor_tile,
    "upper_tile": upper_tile,
    "wall_full": wall_full,
    "wall_window": wall_window,
    "wall_half": wall_half,
    "railing": railing,
    "post_full": lambda: post("post_full", (0.12, 2.62)),
    "post_low": lambda: post("post_low", (0.12, 1.06)),
    "pillar": pillar,
    "block": block,
    "ramp": ramp,
    "target_alcove": target_alcove,
    "spawn_pad": spawn_pad,
    "border_wall": border_wall,
    "border_post": border_post,
}


def main():
    L.reset_scene()
    L.standard_materials()
    missing = set(P) - set(BUILDERS)
    if missing:
        raise SystemExit("no builder for %s" % sorted(missing))
    for name in P:
        geo = BUILDERS[name]()
        bad = geo.mirror_errors()
        if bad:
            raise SystemExit("%s is not X-mirror symmetric: %s" % (name, bad[:5]))
        L.make_object(name, geo)
        print("piece %-14s %5d tris" % (name, geo.tris()))
    L.save_blend("assets-src/blend/kit.blend")
    L.export_glb("assets/models/kit.glb")


main()
