"""Builds the arena's structures: assets-src/blend/structures.blend and
assets/models/structures.glb.

    blender -b --factory-startup --python-exit-code 1 --python tools/models/structures.py

The geometry comes from tools/models/structures.json (generated from
crates/lt-sim/src/structures.rs): every structure is a set of solids, upright prisms over convex
polygons, exactly its colliders. The body is those prisms (faces two solids share left out); the
neon is put on by each solid's tag:

- wall: a band along the top edge of its faces (and along the bottom edge over an opening: lit
  slit and doorway heads), a line near the foot, a band at the upper floor's height round tall
  walls; the exposed ends (door jambs, slit and crenel sides) lit up their edges.
- step: a nose line across the front of each tread (on top and on the riser), the treads' sides
  lit along their top edges: the stair reads as a lit zigzag.
- deck / roof: lines along the slab's exposed edges, on top and underneath (roofs: their rim).
- column: rings at foot and head, a line up every other face.
- rail: a line along its top.

Spotlights (`lamps`) get a housing on a short post with a white lens (`lamp`); the game puts the
beam in front of it and sweeps it. One mesh per structure, named as in the JSON, in its own frame
(game coords: Y up, origin in its middle on the floor). Materials: `body`, `accent`, `glow` (as
the kit), `lamp` (white, not team coloured).
"""

import json
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ltlib as L  # noqa: E402
from ltlib import Frame, filled, line  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = json.load(open(os.path.join(HERE, "structures.json")))
STOREY = 3.0
W = 0.04  # standard strip width
INSET = 0.05


def qkey(p):
    return tuple(int(round(c * 1000)) for c in p)


class Solid:
    def __init__(self, d):
        self.poly = [tuple(p) for p in d["poly"]]
        self.y0, self.y1 = d["y"]
        self.tag = d["tag"]
        self.faces = d["faces"]
        self.front = d["front"]

    def side(self, i):
        """Side i: its corners a, b (x, z), outward normal (x, z) and length."""
        a, b = self.poly[i], self.poly[(i + 1) % len(self.poly)]
        e = (b[0] - a[0], b[1] - a[1])
        ln = math.hypot(*e)
        return a, b, (e[1] / ln, -e[0] / ln), ln

    def side_quad(self, i):
        a, b, _, _ = self.side(i)
        return [(a[0], self.y0, a[1]), (b[0], self.y0, b[1]), (b[0], self.y1, b[1]),
                (a[0], self.y1, a[1])]

    def side_frame(self, i):
        """Frame centred on side face i: U along it (from b to a), V up, N out; half sizes."""
        a, b, n, ln = self.side(i)
        o = ((a[0] + b[0]) / 2, (self.y0 + self.y1) / 2, (a[1] + b[1]) / 2)
        return Frame(o, (a[0] - b[0], 0, a[1] - b[1]), L.Y), ln / 2, (self.y1 - self.y0) / 2

    def top(self):
        return [(x, self.y1, z) for x, z in self.poly]

    def bottom(self):
        return [(x, self.y0, z) for x, z in self.poly]


# Neon collected while decorating, and put on once a structure's solids are all done, chained
# into long strips: bands along vertical faces at one height (round a whole tower), lines along
# slab edges (round a whole deck): one strip mitred round a curve, no seams where the segments
# meet. They are flat (no side faces 8 mm deep, only ever seen edge on): most of the neon is
# these, and that leaves out two thirds of their triangles.
BANDS = []  # (a, b, n, y, w, glow): a band on the vertical face over a-b (x, z), n its outside
RIMS = []  # (a, b, y, inset, w, under): a line on a slab's top (or underside) along a-b


def band(s, i, y, w, glow):
    a, b, n, _ = s.side(i)
    BANDS.append((a, b, n, y, w, glow))


def rim(a, b, y, inset, w, under=False):
    RIMS.append((a, b, y, inset, w, under))


def chains(pieces, start, end, kind):
    """Pieces joined end to start into chains (lists of pieces) of the same kind; and whether
    each closes on itself."""
    by_start, by_end = {}, {}
    for k, p in enumerate(pieces):
        by_start.setdefault((kind(p), start(p)), []).append(k)
        by_end.setdefault((kind(p), end(p)), []).append(k)
    used = set()
    out = []
    for k0 in range(len(pieces)):
        if k0 in used:
            continue
        used.add(k0)
        chain = [k0]
        closed = False
        while True:
            key = (kind(pieces[chain[-1]]), end(pieces[chain[-1]]))
            nxt = [k for k in by_start.get(key, []) if k not in used]
            if not nxt:
                closed = key == (kind(pieces[k0]), start(pieces[k0])) and len(chain) > 2
                break
            used.add(nxt[0])
            chain.append(nxt[0])
        if not closed:
            while True:
                key = (kind(pieces[chain[0]]), start(pieces[chain[0]]))
                prv = [k for k in by_end.get(key, []) if k not in used]
                if not prv:
                    break
                used.add(prv[0])
                chain.insert(0, prv[0])
        out.append(([pieces[k] for k in chain], closed))
    return out


def q2(p):
    return (int(round(p[0] * 1000)), int(round(p[1] * 1000)))


def emit_bands(g):
    kind = lambda p: (round(p[3], 4), p[4], p[5])
    for chain, closed in chains(BANDS, lambda p: q2(p[0]), lambda p: q2(p[1]), kind):
        y, w, glow = chain[0][3], chain[0][4], chain[0][5]
        pts = [c[0] for c in chain] + ([] if closed else [chain[-1][1]])
        segs = [c[2] for c in chain]
        n = len(pts)
        # Each point's way out, mitred between the faces meeting there.
        mit = []
        for k in range(n):
            if closed:
                n1, n2 = segs[k - 1], segs[k]
            else:
                n1 = segs[max(k - 1, 0)]
                n2 = segs[min(k, len(segs) - 1)]
            d = 1.0 + n1[0] * n2[0] + n1[1] * n2[1]
            mit.append(((n1[0] + n2[0]) / d, (n1[1] + n2[1]) / d))
        hw = w / 2
        P = lambda k, out, dy: (pts[k % n][0] + mit[k % n][0] * out, y + dy,
                                pts[k % n][1] + mit[k % n][1] * out)
        h, gh = L.ACCENT_H, L.GLOW_H
        s_len = [0.0]
        for k in range(len(segs)):
            a, b = pts[k], pts[(k + 1) % n]
            s_len.append(s_len[-1] + math.hypot(b[0] - a[0], b[1] - a[1]))
        for k in range(len(segs)):
            j = k + 1
            nn = (segs[k][0], 0, segs[k][1])
            u0, u1 = s_len[k], s_len[k + 1]
            g.face([P(k, h, -hw), P(j, h, -hw), P(j, h, hw), P(k, h, hw)], "accent",
                   [(u0, 0), (u1, 0), (u1, 1), (u0, 1)], want=nn)
            for sv in (1, -1):
                g.face([P(k, gh, sv * hw), P(j, gh, sv * hw), P(j, gh, sv * (hw + glow)),
                        P(k, gh, sv * (hw + glow))], "glow",
                       [(u0, 0), (u1, 0), (u1, 1), (u0, 1)], want=nn)


def emit_rims(g):
    kind = lambda p: (round(p[2], 4), p[3], p[4], p[5])
    for chain, closed in chains(RIMS, lambda p: q2(p[0]), lambda p: q2(p[1]), kind):
        y, inset, w, under = chain[0][2], chain[0][3], chain[0][4], chain[0][5]
        pts = [c[0] for c in chain] + ([] if closed else [chain[-1][1]])
        if under:
            f = Frame((0, y, 0), L.X, L.Z)  # N = -Y; (u, v) = (x, z): inside on the left
            uv, d = [(p[0], p[1]) for p in pts], inset
        else:
            f = Frame((0, y, 0), L.X, L.NZ)  # N = +Y; (u, v) = (x, -z): inside on the right
            uv, d = [(p[0], -p[1]) for p in pts], -inset
        centre = L.offset_line(uv, d, closed)
        L.strip(g, f, centre, w, closed=closed, glow=(0.05, 0.05), caps=False, sides=False)


def face_key(pts):
    return frozenset(qkey(p) for p in pts)


def edge_key(a, b, y):
    return frozenset((qkey((a[0], y, a[1])), qkey((b[0], y, b[1]))))


def build(name, spec):
    g = L.Geo()
    BANDS.clear()
    RIMS.clear()
    solids = [Solid(d) for d in spec["solids"]]
    # Faces two solids share are inside: left out.
    count = {}
    for s in solids:
        faces = [s.top(), s.bottom()] + [s.side_quad(i) for i in range(len(s.poly))]
        for f in faces:
            k = face_key(f)
            count[k] = count.get(k, 0) + 1
    exposed = lambda pts: count.get(face_key(pts), 0) < 2
    # Slab edges two slabs at the same height share are inside the floor: not lit.
    slab_edges = {}
    for s in solids:
        if s.tag in ("deck", "roof"):
            for i in range(len(s.poly)):
                a, b, _, _ = s.side(i)
                for y in (s.y0, s.y1):
                    k = edge_key(a, b, y)
                    slab_edges[k] = slab_edges.get(k, 0) + 1

    for s in solids:
        n = len(s.poly)
        cx = sum(p[0] for p in s.poly) / n
        cz = sum(p[1] for p in s.poly) / n
        if exposed(s.top()):
            g.face(s.top(), "body", want=L.Y)
        if s.y0 > 0.001 and exposed(s.bottom()):
            g.face(s.bottom(), "body", want=L.NY)
        for i in range(n):
            q = s.side_quad(i)
            _, _, nrm, _ = s.side(i)
            if exposed(q):
                g.face(q, "body", want=(nrm[0], 0, nrm[1]))
        decorate(g, s, exposed, slab_edges)
    emit_bands(g)
    emit_rims(g)
    for lamp in spec["lamps"]:
        lamp_housing(g, tuple(lamp["pos"]), tuple(lamp["dir"]))
    return g


def decorate(g, s, exposed, slab_edges):
    n = len(s.poly)
    h = s.y1 - s.y0
    for i in range(n):
        q = s.side_quad(i)
        if not exposed(q):
            continue
        f, hu, hv = s.side_frame(i)
        is_face = bool(s.faces & (1 << i))
        if s.tag == "wall":
            if is_face:
                if hv > 0.08:
                    band(s, i, s.y1 - INSET, W, 0.06)
                if s.y0 > 0.01 and hv > 0.08:
                    band(s, i, s.y0 + INSET, W, 0.06)
                if s.y0 < 0.01 and h > 0.5:
                    band(s, i, s.y0 + 0.14, 0.025, 0.05)
                if s.y0 < STOREY - 0.6 and s.y1 > STOREY + 0.6:
                    band(s, i, STOREY - 0.3, 0.03, 0.07)
                # Over a doorway in a tall wall: chevrons pointing down at it.
                if s.y0 > 2.0 and hv > 0.8 and hu > 0.5:
                    for k in range(3):
                        v = -hv + 0.45 + 0.3 * k
                        L.strip(g, f, L.chevron(0, v, 0.3, 0.16, up=False), 0.04,
                                glow=(0.06, 0.06))
            elif hu > 0.06 and hv > 0.1:
                # An end: a doorway's or opening's jamb, lit up its edges.
                for su in (-1, 1):
                    line(g, f, (su * (hu - 0.035), -hv + 0.04), (su * (hu - 0.035), hv - 0.04),
                         0.025, glow=0.04)
        elif s.tag == "step":
            if i == s.front:
                line(g, f, (-hu + 0.05, hv - 0.035), (hu - 0.05, hv - 0.035), 0.03, glow=0.05)
            elif is_face:
                band(s, i, s.y1 - 0.035, 0.025, 0.05)
        elif s.tag in ("deck", "roof"):
            a, b, _, _ = s.side(i)
            if slab_edges.get(edge_key(a, b, s.y1), 0) < 2 and hu > 0.05:
                band(s, i, (s.y0 + s.y1) / 2, 0.03, 0.05)
        elif s.tag == "column":
            for y in (s.y0 + 0.16, s.y1 - 0.16):
                band(s, i, y, 0.03, 0.05)
            if i % 2 == 0 and hv > 0.6 and hu > 0.06:
                line(g, f, (0, -hv + 0.36), (0, hv - 0.36), 0.03, glow=0.05)
        elif s.tag == "rail":
            if is_face:
                band(s, i, s.y1 - 0.04, 0.025, 0.04)
    # Tops and undersides.
    if s.tag == "step":
        a, b, nrm, ln = s.side(s.front)
        top_line(g, s, a, b, nrm, 0.07, 0.035, s.y1)
    if s.tag in ("deck", "roof"):
        for i in range(n):
            a, b, nrm, ln = s.side(i)
            if ln < 0.1:
                continue
            if slab_edges.get(edge_key(a, b, s.y1), 0) < 2 and exposed(s.top()):
                # (Unbroken round a slab of many pieces.)
                rim(a, b, s.y1, 0.08 if s.tag == "roof" else 0.05, 0.03)
            if s.y0 > 0.01 and slab_edges.get(edge_key(a, b, s.y0), 0) < 2 and exposed(s.bottom()):
                rim(a, b, s.y0, 0.06, 0.03, under=True)
    if s.tag == "rail" and exposed(s.top()):
        # Along the rail's longest side, in the middle of its top.
        i = max(range(n), key=lambda k: s.side(k)[3])
        a, b, nrm, ln = s.side(i)
        width = min(abs((p[0] - a[0]) * nrm[0] + (p[1] - a[1]) * nrm[1]) for p in s.poly
                    if abs((p[0] - a[0]) * nrm[0] + (p[1] - a[1]) * nrm[1]) > 1e-3)
        rim(a, b, s.y1, width / 2, 0.02)


def top_line(g, s, a, b, nrm, inset, w, y, under=False, pull=None):
    """A line on the solid's top (or underside) along side a-b, `inset` in from it, its ends
    pulled in by `pull` (default: a little more than the inset)."""
    if under:
        f = Frame((0, y, 0), L.X, L.Z)  # N = -Y; (u, v) = (x, z)
        uv = lambda p: (p[0], p[1])
    else:
        f = Frame((0, y, 0), L.X, L.NZ)  # N = +Y; (u, v) = (x, -z)
        uv = lambda p: (p[0], -p[1])
    ex, ez = b[0] - a[0], b[1] - a[1]
    ln = math.hypot(ex, ez)
    e = (ex / ln, ez / ln)
    pull = min(inset + 0.04 if pull is None else pull, ln / 2 - 0.01)
    if pull < 0:
        return
    pa = (a[0] - nrm[0] * inset + e[0] * pull, a[1] - nrm[1] * inset + e[1] * pull)
    pb = (b[0] - nrm[0] * inset - e[0] * pull, b[1] - nrm[1] * inset - e[1] * pull)
    line(g, f, uv(pa), uv(pb), w, glow=0.05)


def lamp_housing(g, pos, d):
    """A spotlight on the parapet at `pos`, looking out along `d` (level): a post, a housing,
    a white lens on its front with a lit rim."""
    d = L.norm(d)
    side = L.cross(L.Y, d)  # to the lamp's left
    x0, y0, z0 = pos

    def box_along(c, hw, hh, hd):
        pts = {}
        for su in (-1, 1):
            for sv in (-1, 1):
                for sd in (-1, 1):
                    pts[(su, sv, sd)] = L.add(c, L.add(L.mul(side, su * hw), L.add(
                        (0, sv * hh, 0), L.mul(d, sd * hd))))
        quads = [
            ((-1, -1, 1), (1, -1, 1), (1, 1, 1), (-1, 1, 1), d),
            ((-1, -1, -1), (1, -1, -1), (1, 1, -1), (-1, 1, -1), L.mul(d, -1)),
            ((1, -1, -1), (1, -1, 1), (1, 1, 1), (1, 1, -1), side),
            ((-1, -1, -1), (-1, -1, 1), (-1, 1, 1), (-1, 1, -1), L.mul(side, -1)),
            ((-1, 1, -1), (1, 1, -1), (1, 1, 1), (-1, 1, 1), L.Y),
            ((-1, -1, -1), (1, -1, -1), (1, -1, 1), (-1, -1, 1), L.NY),
        ]
        for a, b, c2, e, want in quads:
            g.face([pts[a], pts[b], pts[c2], pts[e]], "body", want=want)

    box_along((x0, y0 + 0.06, z0), 0.05, 0.06, 0.05)
    hc = (x0, y0 + 0.28, z0)
    box_along(hc, 0.17, 0.14, 0.16)
    front = Frame(L.add(hc, L.mul(d, 0.16)), L.mul(side, -1), L.Y)  # N = d
    filled(g, front, L.ngon(0, 0, 0.11, 12, 0.0), glow=0.0, mat="lamp")
    L.ring(g, front, L.ngon(0, 0, 0.135, 12, 0.0), 0.02, glow_in=0.0, glow_out=0.04)
    for su in (-1, 1):
        f = Frame(L.add(hc, L.mul(side, su * 0.17)), L.mul(d, su), L.Y)
        line(g, f, (-0.13, 0.1), (0.13, 0.1), 0.02, glow=0.03)


def main():
    L.reset_scene()
    L.standard_materials()
    L.material("lamp", base=(1, 1, 1), rough=0.3, emit=(1, 1, 1), strength=8.0)
    for name, spec in DATA.items():
        geo = build(name, spec)
        L.make_object(name, geo)
        print("structure %-14s %6d tris" % (name, geo.tris()))
    L.save_blend("assets-src/blend/structures.blend")
    L.export_glb("assets/models/structures.glb")


main()
