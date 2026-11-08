"""Builds the player avatars: assets-src/blend/avatars.blend and assets/models/avatars.glb.

    blender -b --factory-startup --python-exit-code 1 --python tools/models/avatars.py

Players are a head, a torso and two hands, joined by rigid body segments (neck, pelvis, arms,
legs, boots) that the game poses by IK every frame.

- `head_wolf`, `head_cat`, `head_deer`, `head_fox`, `head_bear`, `head_rabbit`, `head_owl`,
  `head_boar`: faceted low-poly animal heads. Origin at the midpoint between the eyes (the VR
  head pose), looking along -Z, Y up. Each head is a loft of 16-sided rings from the back of
  the skull to the snout tip plus ears / antlers / tusks / beak, all X mirror symmetric.
- `torso`: armoured vest, origin at the top centre (neck point), hanging down -Y, front -Z.
  `hit_1` (top), `hit_2`, `hit_3` (bottom) light bars on the chest front and on the back.
- `hand_left`, `hand_right`: gloved hands gripping, origin at the palm centre, fingers along
  -Z, palm facing +X (left) / -X (right). Built separately (mirror images), not mirrored at
  runtime.
- `neck`, `pelvis`, `upper_arm`, `forearm`, `thigh`, `shin`, `boot`: rigid body segments, each
  hanging from its joint (the origin) down -Y, front -Z; one mesh serves both sides.
- `tail_<animal>_<k>`: each animal's tail, a chain of pieces from the pelvis's back that the game
  swings about their joints, placed at rest by the empties `tail_<animal>_j<k>` (see TAILS).

Materials: `body` (near-black), `fur_light` (mid grey: muzzles, cheeks, facial discs so the
faces read in the dark), `accent` (noses, ear inners, trim; recoloured to the team colour by the
game), `eyes` (eye plates, bright team colour), `eye_glow` (additive halo rings around the eyes:
UV V = 0 at the eye's edge, 1 at the halo's outer edge), `flow` (thin raised light lines with
UV U = arc length in metres along the line and V = 0..1 across; the game runs pulses along
them), `glow` (halos on the torso trim), `hit_1`..`hit_3`.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ltlib as L  # noqa: E402
from ltlib import add, sub, mul, dot, cross, norm, length, X, Y, Z, NX, NY, NZ  # noqa: E402

HEADS = ("wolf", "cat", "deer", "fox", "bear", "rabbit", "owl", "boar", "dragon")
SOLID = ("body", "fur_light")


# ---------------------------------------------------------------------------------------------
# Generic solids.

def centroid(pts):
    return mul(tuple(map(sum, zip(*pts))), 1.0 / len(pts))


def planar(pts, tol=2e-5):
    if len(pts) == 3:
        return True
    n = norm(L.newell(pts))
    c = centroid(pts)
    return all(abs(dot(sub(p, c), n)) < tol for p in pts)


def facet(geo, pts, mat, want):
    """A face; non-planar quads are split into two triangles (crisp facets)."""
    if len(pts) == 4 and not planar(pts):
        a, b, c, d = pts
        tris = [(a, b, c), (a, c, d)] if length(sub(a, c)) <= length(sub(b, d)) else \
            [(a, b, d), (b, c, d)]
        for t in tris:
            geo.face(list(t), mat, want=want)
    else:
        geo.face(pts, mat, want=want)


def loft(geo, rings, mat="body", cap0="body", cap1="body", paint=None):
    """Closed tube through rings (equal-length point lists; the last may be a single apex
    point). Faces point away from the ring-centre axis. `mat` is a material or a list per
    segment; paint(centre, outward, segment) may override it (return None to keep)."""
    cs = [centroid(r) for r in rings]
    for k in range(len(rings) - 1):
        A, B = rings[k], rings[k + 1]
        ax = norm(sub(cs[k + 1], cs[k]))
        m = mat if isinstance(mat, str) else mat[k]
        n = len(A)
        for i in range(n):
            j = (i + 1) % n
            q = [A[i], A[j], B[0]] if len(B) == 1 else [A[i], A[j], B[j], B[i]]
            fc = centroid(q)
            rel = sub(fc, cs[k])
            out = norm(sub(rel, mul(ax, dot(rel, ax))))
            if len(B) > 1:
                # Faces of shoulders (rings changing size along the axis) lean along it.
                dr = length(sub(mul(add(B[i], B[j]), 0.5), cs[k + 1])) - \
                    length(sub(mul(add(A[i], A[j]), 0.5), cs[k]))
                da = length(sub(cs[k + 1], cs[k]))
                out = norm(add(mul(out, max(da, 1e-6)), mul(ax, -dr)))
            mm = (paint(fc, out, k) if paint else None) or m
            facet(geo, q, mm, out)
    if cap0:
        geo.face(list(rings[0]), cap0, want=sub(cs[0], cs[1]))
    if cap1 and len(rings[-1]) > 2:
        geo.face(list(rings[-1]), cap1, want=sub(cs[-1], cs[-2]))


def pyramid(geo, base, apex, mat="body", base_mat="body"):
    """Faces of a pyramid over a convex base polygon; returns the side faces' point lists."""
    loft(geo, [base, [apex]], mat=mat, cap0=base_mat, cap1=None)
    n = len(base)
    return [[base[i], base[(i + 1) % n], apex] for i in range(n)]


def tube(geo, path, radii, sides=4, mat="body", tip=True, up=Y, phase=45.0):
    """Tapering tube along a 3D polyline (antlers, tusks, whiskers, fingers)."""
    rings = []
    for i, p in enumerate(path):
        if i == 0:
            t = sub(path[1], path[0])
        elif i == len(path) - 1:
            t = sub(path[-1], path[-2])
        else:
            t = add(norm(sub(path[i], path[i - 1])), norm(sub(path[i + 1], path[i])))
        t = norm(t)
        a = cross(t, up)
        if length(a) < 1e-4:
            a = cross(t, X)
        a = norm(a)
        b = norm(cross(a, t))
        r = radii[i]
        if tip and i == len(path) - 1:
            rings.append([p])
            break
        rings.append([add(p, add(mul(a, r * math.cos(math.radians(phase + 360.0 * k / sides))),
                                 mul(b, r * math.sin(math.radians(phase + 360.0 * k / sides)))))
                      for k in range(sides)])
    loft(geo, rings, mat=mat, cap0=mat if isinstance(mat, str) else mat[0],
         cap1=None if tip else (mat if isinstance(mat, str) else mat[-1]))


def box_ring(c, U, V, hu, hv, ch=0.0):
    """Chamfered rectangle around c in the plane spanned by U, V."""
    pts = L.chamfer_rect(-hu, -hv, hu, hv, ch) if ch > 0 else L.rect(-hu, -hv, hu, hv)
    return [add(c, add(mul(U, u), mul(V, v))) for u, v in pts]


# ---------------------------------------------------------------------------------------------
# Accents on faceted surfaces.

def tri_hit(o, d, a, b, c):
    e1, e2 = sub(b, a), sub(c, a)
    p = cross(d, e2)
    det = dot(e1, p)
    if abs(det) < 1e-12:
        return None
    s = sub(o, a)
    u = dot(s, p) / det
    if u < -1e-6 or u > 1 + 1e-6:
        return None
    q = cross(s, e1)
    v = dot(d, q) / det
    if v < -1e-6 or u + v > 1 + 1e-6:
        return None
    t = dot(e2, q) / det
    return t if t > 1e-6 else None


def raycast(geos, o, d):
    """First solid face hit along the ray: (point, outward face normal)."""
    d = norm(d)
    best = None
    for geo in geos:
        for pts, mat, _ in geo.faces:
            if mat not in SOLID:
                continue
            for i in range(1, len(pts) - 1):
                t = tri_hit(o, d, pts[0], pts[i], pts[i + 1])
                if t is not None and (best is None or t < best[0]):
                    best = (t, norm(L.newell(pts)))
    if best is None:
        raise RuntimeError("raycast from %s along %s hit nothing" % (o, d))
    return add(o, mul(d, best[0])), best[1]


def plate(geo, c, n, shape, mat="accent", h=0.003, depth=0.008, uhint=X, vhint=Y):
    """A thin convex plate (u, v shape) lying on the plane through c with normal n, standing h
    proud and sunk `depth` into the surface so it survives small facet bends."""
    U = norm(sub(uhint, mul(n, dot(uhint, n))))
    V = sub(vhint, mul(n, dot(vhint, n)))
    V = norm(sub(V, mul(U, dot(V, U))))
    P = lambda u, v, hh: add(c, add(mul(U, u), add(mul(V, v), mul(n, hh))))
    cu = sum(u for u, _ in shape) / len(shape)
    cv = sum(v for _, v in shape) / len(shape)
    geo.face([P(u, v, h) for u, v in shape], mat, [(u * 10, v * 10) for u, v in shape], want=n)
    m = len(shape)
    for i in range(m):
        a, b = shape[i], shape[(i + 1) % m]
        mu, mv = (a[0] + b[0]) / 2 - cu, (a[1] + b[1]) / 2 - cv
        geo.face([P(*a, -depth), P(*b, -depth), P(*b, h), P(*a, h)], mat,
                 want=add(mul(U, mu), mul(V, mv)))


def decal(geo, hit_geos, target, d, shape, mat="accent", h=0.003, depth=0.008, uhint=X,
          vhint=Y):
    """plate() on whatever solid face a ray along d towards `target` meets first."""
    c, n = raycast(hit_geos, sub(target, mul(norm(d), 0.4)), d)
    plate(geo, c, n, shape, mat, h, depth, uhint, vhint)
    return c, n


def face_plate(geo, pts, out, k=0.6, shift=(0, 0, 0), mat="accent", h=0.002, depth=0.004):
    """An accent plate on a known planar face: the face scaled by k about its centroid."""
    n = norm(L.newell(pts))
    if dot(n, out) < 0:
        n = mul(n, -1)
    c = add(centroid(pts), shift)
    inner = [add(c, mul(sub(p, centroid(pts)), k)) for p in pts]
    geo.face([add(p, mul(n, h)) for p in inner], mat, want=n)
    m = len(inner)
    for i in range(m):
        a, b = inner[i], inner[(i + 1) % m]
        mid = sub(mul(add(a, b), 0.5), c)
        geo.face([add(a, mul(n, -depth)), add(b, mul(n, -depth)), add(b, mul(n, h)),
                  add(a, mul(n, h))], mat, want=mid)


def quad(geo, q, mat, uvs, want):
    """A quad with UVs; split into two triangles when it is not planar."""
    if planar(q, 1e-5):
        geo.face(q, mat, uvs, want=want)
        return
    for t in ((0, 1, 2), (0, 2, 3)):
        geo.face([q[i] for i in t], mat, [uvs[i] for i in t], want=want)


def project(geos, p, n, lift, fallback=True):
    """p moved onto the solid surface along n (searching 0.04 m either side), lifted along the
    face normal; falls back to p when nothing is near."""
    try:
        q, fn = raycast(geos, add(p, mul(n, 0.04)), mul(n, -1))
    except RuntimeError:
        q = None
    if q is None or length(sub(q, p)) > 0.04:
        if not fallback:
            raise RuntimeError("no surface near %s" % (p,))
        return add(p, mul(n, lift))
    return add(q, mul(fn, lift))


def eye_halo(geo, hit_geos, c, n, shape, width, uhint=X, vhint=Y, mat="eye_glow", lift=0.002,
             seg=0.022):
    """Additive halo ring around a plate() eye: from just inside the eye's edge out to `width`
    beyond it, draped over the surface `lift` proud. UV: U = metres around the eye, V = 0 at
    the eye's edge, 1 at the halo's outer edge (the game fades it by (1 - V)^2)."""
    U = norm(sub(uhint, mul(n, dot(uhint, n))))
    V = sub(vhint, mul(n, dot(vhint, n)))
    V = norm(sub(V, mul(U, dot(V, U))))
    pts = L.ccw(shape)  # left of the outline is inside
    # Split long outline edges and add a middle ring so the halo drapes over facet ridges.
    pts = resample(pts + [pts[0]], seg)[:-1]
    s = L._arclen(pts, True)
    P = lambda uv: project(hit_geos, add(c, add(mul(U, uv[0]), mul(V, uv[1]))), n, lift)
    rings = [([P(q) for q in L.offset_line(pts, d, True)], v)
             for d, v in ((0.0008, 0.0), (-width * 0.45, 0.45), (-width, 1.0))]
    m = len(pts)
    for (A, va), (B, vb) in zip(rings, rings[1:]):
        for i in range(m):
            j = (i + 1) % m
            quad(geo, [A[i], A[j], B[j], B[i]], mat,
                 [(s[i], va), (s[i + 1], va), (s[i + 1], vb), (s[i], vb)], n)


def resample(guide, step):
    out = [tuple(guide[0])]
    for a, b in zip(guide, guide[1:]):
        k = max(1, int(math.ceil(length(sub(b, a)) / step)))
        out += [tuple(a[t] + (b[t] - a[t]) * i / k for t in range(len(a))) for i in range(1, k + 1)]
    return out


def simplify(samples, tol=0.0006, ndot=0.998):
    """Drops samples that lie on the line through their neighbours with the same normal."""
    s = list(samples)
    changed = True
    while changed and len(s) > 2:
        changed = False
        for i in range(1, len(s) - 1):
            (a, na), (p, np_), (b, nb) = s[i - 1], s[i], s[i + 1]
            ab = sub(b, a)
            t = max(0.0, min(1.0, dot(sub(p, a), ab) / max(dot(ab, ab), 1e-12)))
            dev = length(sub(p, add(a, mul(ab, t))))
            if dev < tol and dot(na, np_) > ndot and dot(np_, nb) > ndot:
                del s[i]
                changed = True
                break
    return s


def flow_strip(geo, samples, w, mat="flow", h=0.0025, depth=0.004, mid=False):
    """A thin raised light line through surface samples [(point, outward normal)], standing h
    proud and sunk `depth` into the surface. UV: U = arc length (m) from the first sample,
    V = 0..1 across. mid=True keeps it on the X mirror plane (symmetric)."""
    if mid:
        samples = [((0.0, p[1], p[2]), norm((0.0, q[1], q[2]))) for p, q in samples]
    samples = simplify(samples)
    P = [p for p, _ in samples]
    N0 = [q for _, q in samples]
    k = len(P)
    N = [norm(add(add(N0[max(i - 1, 0)], mul(N0[i], 2.0)), N0[min(i + 1, k - 1)])) for i in range(k)]
    if mid:
        N = [norm((0.0, q[1], q[2])) for q in N]
    S, arc = [], [0.0]
    for i in range(k):
        t = norm(sub(P[min(i + 1, k - 1)], P[max(i - 1, 0)]))
        S.append(norm(cross(N[i], t)))
        if i > 0:
            arc.append(arc[-1] + length(sub(P[i], P[i - 1])))
    hw = w / 2
    Lt = [add(P[i], add(mul(S[i], hw), mul(N[i], h))) for i in range(k)]
    Rt = [add(P[i], add(mul(S[i], -hw), mul(N[i], h))) for i in range(k)]
    Lb = [add(P[i], add(mul(S[i], hw), mul(N[i], -depth))) for i in range(k)]
    Rb = [add(P[i], add(mul(S[i], -hw), mul(N[i], -depth))) for i in range(k)]
    for i in range(k - 1):
        j = i + 1
        n = norm(add(N[i], N[j]))
        sd = norm(add(S[i], S[j]))
        quad(geo, [Rt[i], Rt[j], Lt[j], Lt[i]], mat,
             [(arc[i], 0.0), (arc[j], 0.0), (arc[j], 1.0), (arc[i], 1.0)], n)
        quad(geo, [Lb[i], Lb[j], Lt[j], Lt[i]], mat,
             [(arc[i], 1.0), (arc[j], 1.0), (arc[j], 1.0), (arc[i], 1.0)], sd)
        quad(geo, [Rb[i], Rb[j], Rt[j], Rt[i]], mat,
             [(arc[i], 0.0), (arc[j], 0.0), (arc[j], 0.0), (arc[i], 0.0)], mul(sd, -1))
    for i, sgn in ((0, -1), (k - 1, 1)):
        t = norm(sub(P[min(i + 1, k - 1)], P[max(i - 1, 0)]))
        quad(geo, [Rb[i], Lb[i], Lt[i], Rt[i]], mat,
             [(arc[i], 0.0), (arc[i], 1.0), (arc[i], 1.0), (arc[i], 0.0)], mul(t, sgn))
    return arc[-1]


def ray_strip(geo, hit_geos, guide, ray, w, step=0.01, **kw):
    """flow_strip through the surface points found by ray(point) -> (origin, direction) for
    points resampled every `step` m along the guide polyline."""
    samples = []
    for g in resample(guide, step):
        o, d = ray(g)
        samples.append(raycast(hit_geos, o, d))
    return flow_strip(geo, samples, w, **kw)


def mirror_face(face):
    pts, mat, uvs = face
    return ([(-p[0], p[1], p[2]) for p in reversed(pts)], mat, list(reversed(uvs)))


def translate(geo, off):
    geo.faces = [([add(p, off) for p in pts], m, uv) for pts, m, uv in geo.faces]


# ---------------------------------------------------------------------------------------------
# Heads: cartoon animals with big eyes. The eyeballs and eyelids are two shared meshes
# (`eyeball`, `eyelid`: unit radius, looking along -Z, +Y up) that the game instances at each
# head's `head_<animal>_eye_l` / `_eye_r` nodes (origin at the eyeball's centre, -Z the eye's
# forward, scaled by the eye's radius), turning the balls to look about and the lids to blink.

SIDES = 12
ANG = tuple(180.0 * i / (SIDES // 2) for i in range(SIDES // 2 + 1))
# How wide open each animal's eyes rest: the upper and the lower lid's edge, in degrees above
# and below the eye's forward point (src/assets.rs ANIMALS keeps the same table for the game;
# the blend file carries it on the eye empties for preview.py).
LIDS = {"wolf": (40, 44), "cat": (46, 48), "deer": (54, 52), "fox": (36, 44), "bear": (48, 48),
        "rabbit": (58, 54), "owl": (62, 58), "boar": (40, 42), "dragon": (42, 46)}
# Lids closed: their edges meet this far below the forward point.
LID_SHUT = -5
# The skull surfaces shade smoothly across edges bending less than this (degrees): low, so
# their facets show as the rest of the game's; the eyes stay round.
SMOOTH = 25.0
SMOOTH_EYES = 40.0


def head_ring(z, cy, hw, hh, n=2.4, wt=1.0, wb=1.0):
    """Superellipse section at z (top and bottom vertices on the mirror plane); wt/wb scale the
    width of the upper/lower half."""
    half = []
    for a in ANG:
        s, c = math.sin(math.radians(a)), math.cos(math.radians(a))
        f = wb + (wt - wb) * (c + 1) / 2
        half.append((hw * f * abs(s) ** (2 / n), cy + hh * math.copysign(abs(c) ** (2 / n), c), z))
    return half + [(-x, y, z) for x, y, z in reversed(half[1:-1])]


def circle(c, n, r, k=12, phase=0.0, uhint=Y):
    """k points of a circle of radius r around c in the plane with normal n."""
    n = norm(n)
    a = cross(uhint, n)
    if length(a) < 1e-4:
        a = cross(X, n)
    a = norm(a)
    b = cross(n, a)
    return [add(c, add(mul(a, r * math.cos(math.radians(phase + 360.0 * i / k))),
                       mul(b, r * math.sin(math.radians(phase + 360.0 * i / k)))))
            for i in range(k)]


def torus(geo, c, n, R, r, mat="accent", segs=12, sides=6, uhint=Y, arc=(0.0, 360.0)):
    """A ring (or an arc of one) of radius R, r thick, around c in the plane with normal n."""
    n = norm(n)
    a = cross(uhint, n)
    if length(a) < 1e-4:
        a = cross(X, n)
    a = norm(a)
    b = cross(n, a)
    closed = abs(arc[1] - arc[0] - 360.0) < 1e-6
    k = segs if closed else segs + 1
    rings = []
    for i in range(k):
        t = math.radians(arc[0] + (arc[1] - arc[0]) * i / segs)
        rad = add(mul(a, math.cos(t)), mul(b, math.sin(t)))
        cc = add(c, mul(rad, R))
        rings.append([add(cc, add(mul(rad, r * math.cos(math.radians(360.0 * j / sides))),
                                  mul(n, r * math.sin(math.radians(360.0 * j / sides)))))
                      for j in range(sides)])
    if closed:
        rings.append(rings[0])
    for A, B in zip(rings, rings[1:]):
        for j in range(sides):
            q = [A[j], A[(j + 1) % sides], B[(j + 1) % sides], B[j]]
            cc = centroid(q)
            rad = sub(cc, c)
            rad = norm(sub(rad, mul(n, dot(rad, n))))
            ring_c = add(c, mul(rad, R))
            facet(geo, q, mat, sub(cc, ring_c))
    if not closed:
        geo.face(list(rings[0]), mat, want=sub(centroid(rings[0]), centroid(rings[1])))
        geo.face(list(rings[-1]), mat, want=sub(centroid(rings[-1]), centroid(rings[-2])))


class Head:
    """secs: (z, cy, hw, hh[, n, wt, wb]) from the back of the skull to the snout tip.
    eye: (target x, y, z, yaw) - a ray towards the target, turned `yaw` degrees outwards from
    straight ahead, finds where the right eyeball sits; the ball (radius `radius`) is sunk
    `sink` radii into the skull along that ray and looks ahead, turned `diverge` of the way
    outwards. The midpoint between the eyeballs' centres becomes the origin."""

    def __init__(self, secs, eye, radius, paint=None, nose_mat="body", sink=0.45, diverge=0.3):
        self.g = L.Geo()  # symmetric part
        self.h = L.Geo()  # +X half, mirrored in finish()
        self.rings = [head_ring(*s) for s in secs]
        self.secs = secs
        self.snout = None
        loft(self.g, self.rings, paint=paint, cap1=nose_mat)
        tx, ty, tz, yaw = eye
        out = (math.sin(math.radians(yaw)), 0.0, -math.cos(math.radians(yaw)))
        hit, _ = raycast([self.g], add((tx, ty, tz), mul(out, 0.4)), mul(out, -1))
        self.eye_r = radius
        self.eye_c = sub(hit, mul(out, sink * radius))
        self.eye_fwd = norm(add(mul(NZ, 1.0 - diverge), mul(out, diverge)))
        self.eye_out = out

    def muzzle(self, secs, paint=None, nose_mat="body", throat="fur_light"):
        """The muzzle: a second loft of sections (as the skull's) growing out of the face, from
        inside the skull to the snout tip (then `front`); its back face, where the jaw hangs
        below the skull, is the `throat`."""
        loft(self.g, [head_ring(*s) for s in secs], paint=paint, cap0=throat, cap1=nose_mat)
        self.snout = secs

    @property
    def front(self):
        """(centre, half width, half height) of the snout's end cap."""
        s = (self.snout or self.secs)[-1]
        return (0.0, s[1], s[0]), s[2], s[3]

    def axis_y(self, z):
        """The skull's (or, ahead of it, the muzzle's) centre line height at z."""
        secs = list(self.secs)
        if self.snout:
            secs += [s for s in self.snout if s[0] < secs[-1][0]]
        if z >= secs[0][0]:
            return secs[0][1]
        for a, b in zip(secs, secs[1:]):
            if b[0] <= z <= a[0]:
                t = (a[0] - z) / (a[0] - b[0])
                return a[1] + (b[1] - a[1]) * t
        return secs[-1][1]

    def surface(self, z, angle, lift=0.0):
        """The skull's surface point at section z, `angle` degrees round the long axis from the
        top (+Y) towards +X, and its face normal; lifted along the normal."""
        d = (math.sin(math.radians(angle)), math.cos(math.radians(angle)), 0.0)
        c = (0.0, self.axis_y(z), z)
        p, n = raycast([self.g], add(c, mul(d, 0.5)), mul(d, -1))
        return add(p, mul(n, lift)), n

    def flow(self, guide, w=0.005, step=0.02):
        """A light line over the skull through guide = [(z, angle)]: angle in degrees around
        the head's long axis from the top (+Y) towards +X; 0 everywhere puts one line on the
        mirror plane, otherwise it is drawn on the +X half and mirrored."""
        mid = all(abs(a) < 1e-6 for _, a in guide)
        scaled = [(z, math.radians(a) * 0.1) for z, a in guide]
        pts = [(z, math.degrees(a / 0.1)) for z, a in resample(scaled, step)]
        samples = [self.surface(z, a) for z, a in pts]
        return flow_strip(self.g if mid else self.h, samples, w, mid=mid)

    def brow(self, thick=0.3, lift=1.3, inner=0.0, outer=0.0, span=(10, 170), mat="body",
             fwd=0.0):
        """A ridge over the eye from its inner corner (span[0]) over the top to its outer corner:
        a tube lying on the skull `lift` eye radii from the ball's centre, `thick` radii thick;
        `inner`/`outer` lower those ends (radii) for a frown or a raised brow."""
        r = self.eye_r
        F = self.eye_fwd
        R = norm(cross(Y, F))  # inwards (-X) for the right eye
        path = []
        n = 3
        for i in range(n + 1):
            t = i / n
            a = math.radians(span[0] + (span[1] - span[0]) * t)
            drop = inner + (outer - inner) * t
            d = add(mul(R, math.cos(a)), mul(Y, math.sin(a) - drop))
            p = add(add(self.eye_c, mul(d, lift * r)), mul(F, fwd * r))
            path.append(project([self.g], p, self.eye_out, thick * r * 0.45))
        tube(self.h, path, [thick * r] * len(path), sides=4, tip=False, mat=mat)

    def nose(self, hw, hh, depth=0.012, mat="accent", dy=0.0, k=8):
        """A rounded nose blob on the snout's end cap."""
        c, _, _ = self.front
        c = add(c, (0.0, dy, 0.004))
        rings = []
        for s, dz in ((1.0, 0.0), (0.92, -depth * 0.45), (0.65, -depth * 0.85)):
            rings.append([add(c, (hw * s * math.cos(math.radians(360.0 * i / k)),
                                  hh * s * math.sin(math.radians(360.0 * i / k)), dz))
                          for i in range(k)])
        rings.append([add(c, (0.0, 0.0, -depth))])
        loft(self.g, rings, mat=mat, cap0=None, cap1=None)

    def mouth(self, y, half=0.035, lift=0.012, w=0.0035, mat="body", z=None):
        """A smile: a thin dark line on the muzzle's front from the middle out to each cheek,
        rising `lift` at its ends (seen from the front)."""
        c, _, _ = self.front
        z0 = c[2] if z is None else z
        guide = [(0.0, y, z0), (half * 0.5, y + lift * 0.15, z0), (half, y + lift, z0)]
        ray_strip(self.h, [self.g], guide, lambda p: ((p[0], p[1], -0.5), Z), w, step=0.012,
                  mat=mat, h=0.0012, depth=0.003)

    def ear(self, bc, w, dz, apex, inner=0.55, inner_shift=0.0, tilt=0.0, ch=0.3):
        """Pointed ear: a pyramid over a rounded w x dz base (sunk into the skull at bc), the
        front face carrying an accent inner."""
        hw, hd = w / 2, dz / 2
        base = [add(bc, (x, tilt * x / hw, z)) for x, z in
                L.chamfer_rect(-hw, -hd, hw, hd, min(hw, hd) * ch)]
        sides = pyramid(self.h, base, apex)
        front = min(sides, key=lambda f: centroid(f)[2])
        sh = mul(sub(apex, centroid(front)), inner_shift)
        face_plate(self.h, front, NZ, k=inner, shift=sh)
        return front

    def headband(self, z, r=0.011, span=(0, 100), lift=0.55, flow_w=0.005, mat="body"):
        """A headset's band over the crown at section z: a tube lying on the skull from the top
        down to `span[1]` degrees each side, a light line running along its top."""
        path, tops = [], []
        k = 4
        for i in range(k + 1):
            a = span[0] + (span[1] - span[0]) * i / k
            p, n = self.surface(z, a, r * lift)
            path.append(p)
            tops.append((add(p, mul(n, r)), n))
        tube(self.h, path, [r] * len(path), sides=4, tip=False, mat=mat)
        flow_strip(self.h, tops, flow_w)

    def earcup(self, y, z, r=0.03, depth=0.022, ring=True):
        """A headset's ear cup on the side of the skull: a short cylinder with an accent band
        round it and a glowing disc on its face."""
        p, n = raycast([self.g], (0.5, y, z), NX)
        n = norm((1.0, n[1] * 0.3, n[2] * 0.3))
        c0 = sub(p, mul(n, 0.006))
        loft(self.h, [circle(c0, n, r, 8), circle(add(c0, mul(n, depth * 0.85)), n, r, 8),
                      circle(add(c0, mul(n, depth)), n, r * 0.8, 8)], cap0=None)
        if ring:
            loft(self.h, [circle(add(c0, mul(n, depth * 0.45)), n, r * 1.04, 8),
                          circle(add(c0, mul(n, depth * 0.7)), n, r * 1.04, 8)], mat="accent",
                 cap0=None, cap1=None)
        plate(self.h, add(c0, mul(n, depth)), n, L.ngon(0, 0, r * 0.4, 8), h=0.003, depth=0.006)

    def goggles(self, z, angle, R=0.028, r=0.0055, lens=True):
        """Goggles pushed up on the forehead: two accent rims (and dark lenses) lying on the skull
        at section z, `angle` degrees out from the top, joined by a bridge, on a strap."""
        p, n = self.surface(z, angle, r * 0.9)
        torus(self.h, p, n, R, r, segs=10, sides=4)
        if lens:
            plate(self.h, sub(p, mul(n, r * 0.3)), n, L.ngon(0, 0, R - r * 0.5, 12), "body",
                  h=0.0, depth=0.004)
        # Bridge to the mirror plane, and the strap back round the head.
        q, _ = self.surface(z, 0.0, r * 0.9)
        a = norm(sub(p, q))
        tube(self.h, [q, sub(p, mul(a, R))], [r * 0.8, r * 0.8], sides=4, tip=False,
             mat="accent")
        self.headband(z + 0.004, r=0.007, span=(angle + math.degrees(R / 0.12) + 6, 110),
                      flow_w=0.0035)

    def glasses(self, R=1.3, r=0.1, out=0.95, temple=True):
        """Round glasses before the eyes: an accent rim (R, r in eye radii) `out` radii ahead of
        each eyeball's centre, a bridge between them and temples back to the ears."""
        er = self.eye_r
        c = add(self.eye_c, mul(self.eye_fwd, out * er))
        torus(self.h, c, self.eye_fwd, R * er, r * er, segs=10, sides=4)
        inner = add(c, mul(norm(cross(Y, self.eye_fwd)), R * er))
        mid = (0.0, inner[1] + 0.004, inner[2] + 0.003)
        tube(self.h, [mid, inner], [r * er * 0.8, r * er * 0.8], sides=4, tip=False, mat="accent")
        if temple:
            outer = add(c, mul(norm(cross(self.eye_fwd, Y)), R * er))
            back, n = raycast([self.g], (0.5, outer[1] + 0.01, c[2] + 0.11), NX)
            tube(self.h, [outer, add(outer, (0.004, 0.002, 0.008)), add(back, mul(n, 0.004))],
                 [r * er * 0.8, r * er * 0.7, r * er * 0.6], sides=4, tip=False, mat="accent")

    def tooth(self, x, y, z, w=0.01, h=0.014, up=False, mat="sclera"):
        """A fang hanging from the lip at (x, y, z) (or standing up from the jaw)."""
        s = 1.0 if up else -1.0
        base = [(x - w / 2, y, z - w * 0.4), (x + w / 2, y, z - w * 0.4), (x + w / 2, y, z + w * 0.4),
                (x - w / 2, y, z + w * 0.4)]
        pyramid(self.h if x > 1e-6 else self.g, base, (x, y + s * h, z - w * 0.2), mat=mat,
                base_mat=mat)

    def finish(self):
        for f in self.h.faces:
            self.g.faces.append(f)
            self.g.faces.append(mirror_face(f))
        off = (0.0, -self.eye_c[1], -self.eye_c[2])
        translate(self.g, off)
        self.eye_c = add(self.eye_c, off)
        return self.g

    def eye_nodes(self, name, lids):
        """The eye empties (right, then left mirrored), with the lids' rest angles."""
        props = {"lid_top": lids[0], "lid_bot": lids[1], "radius": self.eye_r}
        L.make_empty(name + "_eye_r", self.eye_c, fwd=self.eye_fwd, scale=self.eye_r, props=props)
        cl = (-self.eye_c[0], self.eye_c[1], self.eye_c[2])
        fl = (-self.eye_fwd[0], self.eye_fwd[1], self.eye_fwd[2])
        L.make_empty(name + "_eye_l", cl, fwd=fl, scale=self.eye_r, props=props)


def leaf_ear(geo, base, axis, fwd_hint, prof, tip, mat="body", strip=(0.22, 0.78)):
    """Long flattened ear (rabbit, deer): a loft of chamfered rectangles along `axis` through
    prof = [(t, half width, half thickness)], ending in a point at t = tip, with a continuous
    accent strip down its front (the side facing fwd_hint)."""
    axis = norm(axis)
    side = norm(cross(axis, fwd_hint))
    fwd = norm(cross(side, axis))
    if dot(fwd, fwd_hint) < 0:
        fwd = mul(fwd, -1)
    rings = [box_ring(add(base, mul(axis, t)), side, fwd, w, d, ch=min(w, d) * 0.6)
             for t, w, d in prof]
    rings.append([add(base, mul(axis, tip))])
    loft(geo, rings, mat=mat)
    lerp = lambda a, b, f: add(a, mul(sub(b, a), f))
    for k in range(len(prof) - 1):
        # The ring's front edge is chamfer_rect edge 4 -> 5 (the points with v = +d).
        a, b, c, d = rings[k][5], rings[k][4], rings[k + 1][4], rings[k + 1][5]
        f0, f1 = strip
        face_plate(geo, [lerp(a, b, f0), lerp(a, b, f1), lerp(d, c, f1), lerp(d, c, f0)], fwd,
                   k=1.0)


def round_ear(geo, c, r, depth, inner=0.55, fwd=NZ):
    """A round ear (bear): a short drum facing `fwd` with an accent inner disc."""
    n = norm(fwd)
    back = circle(sub(c, mul(n, depth * 0.5)), n, r, 10, phase=18)
    front = circle(add(c, mul(n, depth * 0.5)), n, r, 10, phase=18)
    loft(geo, [back, front])
    plate(geo, add(c, mul(n, depth * 0.5)), n, L.ngon(0, 0, r * inner, 10, 18), h=0.003,
          depth=0.006)


def eyeball(slit=False):
    """Unit sphere looking along -Z: a black pupil (round, or a cat's vertical slit), a
    team-coloured iris, a pale sclera and a white glint up and to the left."""
    g = L.Geo()
    pupil = "eyes" if slit else "body"
    # (12 sides, and the back sunk in the skull coarse: every player has two.)
    sides = 12
    steps = [(17, pupil), (25, "eyes"), (38, "eyes"), (49, "sclera"), (75, "sclera"),
             (110, "sclera")]
    rings = []
    for th, _ in steps:
        t = math.radians(th)
        rings.append([(math.sin(t) * math.cos(math.radians(360.0 * i / sides)),
                       math.sin(t) * math.sin(math.radians(360.0 * i / sides)), -math.cos(t))
                      for i in range(sides)])
    rings.append([(0.0, 0.0, 1.0)])
    loft(g, rings, mat=[m for _, m in steps], cap0=pupil, cap1=None)
    if slit:
        # A pointed vertical lens on the sphere: its outline just proud of the iris, fanned from
        # a raised centre, with walls sunk into the ball.
        k = 10
        outline = [norm((0.075 * math.cos(2 * math.pi * i / k), 0.36 * math.sin(2 * math.pi * i / k),
                         -1.0)) for i in range(k)]
        top = [mul(p, 1.012) for p in outline]
        bot = [mul(p, 0.96) for p in outline]
        c = (0.0, 0.0, -1.016)
        for i in range(k):
            j = (i + 1) % k
            g.face([c, top[i], top[j]], "body", want=NZ)
            g.face([bot[i], bot[j], top[j], top[i]], "body", want=sub(top[i], c))
    th, ph = math.radians(30), math.radians(135)
    d = (math.sin(th) * math.cos(ph), math.sin(th) * math.sin(ph), -math.cos(th))
    plate(g, d, d, L.ngon(0, 0, 0.17, 6), "glow_white", h=0.012, depth=0.05)
    return g


def eyelid():
    """A lune of a shell round the eyeball (radii 1.05-1.11), between planes through its X axis:
    from the lid's edge (towards -Z at rest) 118 degrees back over the top. The game turns it
    about X to open and close (and the lower lid is the same, turned over). The edge and the
    outer strip beside it glow in the team colour."""
    g = L.Geo()
    R0, R1 = 1.05, 1.11
    lams = [0, 7, 45, 85, 118]
    inner_lams = [0, 60, 118]
    psis = list(range(0, 181, 30))

    def P(psi, lam, R):
        s, c = math.sin(math.radians(psi)), math.cos(math.radians(psi))
        lr = math.radians(lam)
        return (R * c, R * s * math.sin(lr), -R * s * math.cos(lr))

    def face(pts, mat, want):
        q = []
        for p in pts:
            if not q or length(sub(p, q[-1])) > 1e-9:
                q.append(p)
        if length(sub(q[0], q[-1])) < 1e-9:
            q.pop()
        if len(q) >= 3:
            geo_face(q, mat, want)

    def geo_face(q, mat, want):
        if len(q) == 4:
            facet(g, q, mat, want)
        else:
            g.face(q, mat, want=want)
    for i, (l0, l1) in enumerate(zip(lams, lams[1:])):
        mat = "accent" if i == 0 else "body"
        for p0, p1 in zip(psis, psis[1:]):
            outer = [P(p0, l0, R1), P(p1, l0, R1), P(p1, l1, R1), P(p0, l1, R1)]
            face(outer, mat, centroid(outer))
    for l0, l1 in zip(inner_lams, inner_lams[1:]):
        for p0, p1 in zip(psis, psis[1:]):
            inner = [P(p0, l0, R0), P(p1, l0, R0), P(p1, l1, R0), P(p0, l1, R0)]
            face(inner, "body", mul(centroid(inner), -1))
    for lam, mat, s in ((lams[0], "accent", -1.0), (lams[-1], "body", 1.0)):
        lr = math.radians(lam)
        want = mul((0.0, math.cos(lr), math.sin(lr)), s)
        for p0, p1 in zip(psis, psis[1:]):
            face([P(p0, lam, R0), P(p1, lam, R0), P(p1, lam, R1), P(p0, lam, R1)], mat, want)
    return g


def head_wolf():
    def paint(c, n, k):
        if n[1] < -0.4 and c[2] < 0.04:
            return "fur_light"  # jaw and throat
        return None

    hd = Head([(0.14, 0.03, 0.06, 0.065, 2.2), (0.1, 0.045, 0.11, 0.11, 2.3),
               (0.04, 0.045, 0.125, 0.12, 2.4), (-0.01, 0.04, 0.115, 0.108, 2.5, 0.95, 1.05),
               (-0.04, 0.04, 0.09, 0.085, 2.5, 0.9, 1.05), (-0.06, 0.045, 0.05, 0.045, 2.4)],
              eye=(0.05, 0.045, -0.02, 22), radius=0.034, paint=paint)
    # A long muzzle: a narrow bridge between the eyes over a wide jaw, pale underneath.
    hd.muzzle([(-0.03, -0.01, 0.085, 0.07, 2.4, 0.4, 1.15), (-0.09, -0.015, 0.068, 0.062, 2.5, 0.7, 1.1),
               (-0.14, -0.025, 0.055, 0.05, 2.6, 0.85, 1.05), (-0.175, -0.03, 0.045, 0.04, 2.8),
               (-0.205, -0.035, 0.034, 0.03, 3.0)],
              paint=lambda c, n, k: "fur_light" if n[1] < -0.15 else None)
    hd.brow(thick=0.32, inner=0.3, outer=-0.05)
    hd.nose(0.018, 0.013, depth=0.012, dy=0.008)
    hd.mouth(-0.056, half=0.04, lift=0.014)
    hd.tooth(0.024, -0.054, -0.17, w=0.007, h=0.012)
    hd.ear((0.07, 0.1, 0.05), 0.095, 0.05, (0.1, 0.29, 0.07), inner=0.6, inner_shift=-0.1)
    # Cheek ruffs: pale tufts sweeping out, down and back from behind the muzzle.
    for base_y, apex in (((0.0, 0.035), (0.15, -0.03, 0.01)), ((-0.045, -0.01), (0.135, -0.08, 0.03))):
        y0, y1 = base_y
        pyramid(hd.h, [(0.09, y1, -0.03), (0.09, y1, 0.03), (0.09, y0, 0.03),
                       (0.09, y0, -0.03)], apex, mat="fur_light", base_mat="fur_light")
    hd.flow([(0.14, 0), (0.07, 0)])
    hd.flow([(-0.05, 120), (0.0, 124), (0.07, 118)])
    hd.headband(0.05)
    hd.earcup(-0.005, 0.065)
    return hd


def head_fox():
    def paint(c, n, k):
        if n[1] < -0.4 and c[2] < 0.05:
            return "fur_light"  # throat
        return None

    hd = Head([(0.12, 0.03, 0.055, 0.06, 2.2), (0.09, 0.045, 0.1, 0.105, 2.3),
               (0.03, 0.045, 0.118, 0.112, 2.4), (-0.01, 0.04, 0.105, 0.098, 2.5, 0.9, 1.1),
               (-0.04, 0.04, 0.08, 0.075, 2.5, 0.9, 1.05), (-0.058, 0.045, 0.045, 0.04, 2.4)],
              eye=(0.05, 0.042, -0.02, 18), radius=0.033, paint=paint)
    # A slender pointed muzzle from a narrow bridge, white underneath.
    hd.muzzle([(-0.03, -0.01, 0.075, 0.06, 2.4, 0.4, 1.15), (-0.085, -0.016, 0.052, 0.05, 2.5, 0.7, 1.1),
               (-0.13, -0.026, 0.036, 0.036, 2.6, 0.85, 1.05), (-0.17, -0.034, 0.022, 0.022, 2.8),
               (-0.195, -0.037, 0.014, 0.014, 3.0)],
              paint=lambda c, n, k: "fur_light" if n[1] < -0.1 else None)
    hd.brow(thick=0.28, inner=0.22, outer=-0.1)
    hd.nose(0.012, 0.01, depth=0.011, dy=0.004)
    hd.mouth(-0.058, half=0.035, lift=0.016)
    hd.ear((0.065, 0.095, 0.045), 0.11, 0.045, (0.12, 0.31, 0.06), inner=0.62, inner_shift=-0.1)
    # Big white cheek ruffs sweeping out and down.
    for base_y, apex in (((-0.005, 0.03), (0.15, -0.04, 0.0)), ((-0.05, -0.015), (0.13, -0.09, 0.02))):
        y0, y1 = base_y
        pyramid(hd.h, [(0.08, y1, -0.03), (0.08, y1, 0.03), (0.08, y0, 0.03),
                       (0.08, y0, -0.03)], apex, mat="fur_light", base_mat="fur_light")
    hd.flow([(0.12, 0), (-0.06, 0)])
    hd.flow([(-0.02, 62), (0.05, 70), (0.11, 62)])
    hd.glasses()
    return hd


def head_cat():
    hd = Head([(0.1, 0.03, 0.07, 0.07, 2.2), (0.07, 0.04, 0.12, 0.115, 2.3),
               (0.01, 0.035, 0.14, 0.125, 2.5, 0.95, 1.1), (-0.03, 0.025, 0.135, 0.115, 2.6, 0.9, 1.15),
               (-0.06, 0.015, 0.1, 0.09, 2.6, 0.85, 1.15), (-0.075, 0.0, 0.055, 0.055, 2.4)],
              eye=(0.052, 0.04, -0.03, 14), radius=0.037)
    # A small pale muzzle bump on a wide flat face.
    hd.muzzle([(-0.045, -0.03, 0.05, 0.035, 2.6), (-0.075, -0.032, 0.045, 0.03, 2.6),
               (-0.092, -0.034, 0.03, 0.02, 2.8)], paint=lambda c, n, k: "fur_light",
              nose_mat="fur_light")
    hd.brow(thick=0.2, inner=0.05, outer=-0.08)
    hd.nose(0.011, 0.008, depth=0.009, dy=0.004)
    hd.mouth(-0.04, half=0.03, lift=0.012, z=-0.085)
    hd.ear((0.08, 0.095, 0.03), 0.1, 0.05, (0.115, 0.27, 0.04), inner=0.6, inner_shift=-0.12)
    # Cheek tufts pointing out.
    for base_y, apex in (((-0.02, 0.03), (0.185, -0.04, -0.01)), ((-0.06, -0.025), (0.16, -0.08, 0.0))):
        y0, y1 = base_y
        pyramid(hd.h, [(0.105, y1, -0.035), (0.105, y1, 0.02), (0.105, y0, 0.02),
                       (0.105, y0, -0.035)], apex, mat="fur_light", base_mat="fur_light")
    # Whiskers: three thin glowing spikes from each side of the muzzle.
    for dy, ang in ((0.004, 12), (-0.006, 0), (-0.016, -12)):
        a = math.radians(ang)
        p0 = (0.04, -0.03 + dy, -0.07)
        tube(hd.h, [p0, add(p0, (0.085 * math.cos(a), 0.085 * math.sin(a), 0.02))],
             [0.0022, 0.0], sides=3, mat="accent")
    hd.flow([(-0.05, 118), (0.0, 122), (0.07, 118)])
    hd.goggles(0.06, 36)
    return hd


def head_bear():
    hd = Head([(0.15, 0.035, 0.08, 0.085, 2.2), (0.1, 0.045, 0.14, 0.135, 2.3),
               (0.03, 0.04, 0.155, 0.14, 2.4), (-0.03, 0.03, 0.135, 0.12, 2.5, 0.95, 1.1),
               (-0.065, 0.035, 0.1, 0.09, 2.5, 0.9, 1.05), (-0.085, 0.04, 0.055, 0.05, 2.4)],
              eye=(0.055, 0.04, -0.035, 15), radius=0.034)
    # A broad pale muzzle over a wide jaw.
    hd.muzzle([(-0.05, -0.02, 0.095, 0.072, 2.5, 0.6, 1.15), (-0.11, -0.028, 0.078, 0.062, 2.5, 0.85, 1.08),
               (-0.15, -0.032, 0.062, 0.05, 2.6), (-0.17, -0.034, 0.05, 0.042, 2.8)],
              paint=lambda c, n, k: "fur_light",
              nose_mat="fur_light")
    hd.brow(thick=0.3, inner=0.12, outer=0.0)
    hd.nose(0.024, 0.015, depth=0.014, dy=0.01)
    hd.mouth(-0.058, half=0.045, lift=0.012)
    round_ear(hd.h, (0.115, 0.16, 0.045), 0.058, 0.035, fwd=(0.3, 0.3, -1.0))
    hd.flow([(0.13, 0), (0.0, 0)])
    hd.flow([(-0.06, 118), (-0.01, 122), (0.06, 118)])
    hd.headband(0.06, r=0.012)
    hd.earcup(0.0, 0.07, r=0.04, depth=0.028)
    return hd


def head_rabbit():
    def paint(c, n, k):
        if c[2] < -0.03 and c[1] < 0.0 and n[2] < 0.3:
            return "fur_light"  # cheeks
        return None

    hd = Head([(0.11, 0.025, 0.07, 0.08, 2.2), (0.07, 0.035, 0.115, 0.12, 2.3),
               (0.01, 0.03, 0.128, 0.12, 2.4, 0.95, 1.1), (-0.03, 0.02, 0.115, 0.105, 2.5, 0.9, 1.15),
               (-0.065, 0.005, 0.075, 0.07, 2.5), (-0.085, -0.01, 0.04, 0.04, 2.4)],
              eye=(0.052, 0.042, -0.02, 22), radius=0.037, paint=paint)
    hd.muzzle([(-0.05, -0.03, 0.042, 0.032, 2.6), (-0.085, -0.032, 0.035, 0.026, 2.6),
               (-0.1, -0.034, 0.022, 0.016, 2.8)], paint=lambda c, n, k: "fur_light",
              nose_mat="fur_light")
    hd.brow(thick=0.22, inner=-0.1, outer=0.0, lift=1.35)
    hd.nose(0.01, 0.008, depth=0.008, dy=0.006)
    hd.mouth(-0.044, half=0.03, lift=0.012, z=-0.09)
    # Buck teeth.
    for x in (-0.0055, 0.0055):
        hd.g.face([(x - 0.005, -0.052, -0.094), (x + 0.005, -0.052, -0.094),
                   (x + 0.005, -0.072, -0.09), (x - 0.005, -0.072, -0.09)], "sclera", want=NZ)
    # Long ears leaning a little back and out, accent inner strips.
    leaf_ear(hd.h, (0.04, 0.09, 0.04), (0.22, 1.0, 0.16), NZ,
             [(0.0, 0.026, 0.013), (0.1, 0.038, 0.014), (0.21, 0.038, 0.012), (0.29, 0.02, 0.008)],
             0.33)
    hd.flow([(-0.05, 118), (0.0, 122), (0.07, 118)])
    # A visor band low across the forehead.
    hd.headband(-0.005, r=0.009, span=(0, 70), flow_w=0.006)
    return hd


def head_deer():
    def paint(c, n, k):
        if n[1] < -0.5 and c[2] < 0.05:
            return "fur_light"  # throat
        return None

    hd = Head([(0.12, 0.035, 0.06, 0.065, 2.2), (0.09, 0.045, 0.1, 0.105, 2.3),
               (0.03, 0.04, 0.115, 0.112, 2.4), (-0.01, 0.035, 0.1, 0.095, 2.5, 0.95, 1.05),
               (-0.04, 0.035, 0.078, 0.075, 2.5), (-0.058, 0.04, 0.045, 0.042, 2.4)],
              eye=(0.05, 0.04, -0.01, 28), radius=0.034, paint=paint)
    # A gentle tapering muzzle from a narrow bridge.
    hd.muzzle([(-0.03, -0.012, 0.075, 0.065, 2.4, 0.45, 1.12), (-0.09, -0.03, 0.055, 0.055, 2.5, 0.75, 1.08),
               (-0.135, -0.045, 0.045, 0.045, 2.6, 0.9, 1.0), (-0.165, -0.055, 0.036, 0.035, 2.8),
               (-0.18, -0.06, 0.028, 0.028, 3.0)],
              paint=lambda c, n, k: "fur_light" if n[1] < -0.1 or c[2] < -0.13 else None,
              nose_mat="fur_light")
    hd.brow(thick=0.22, inner=-0.05, outer=0.0, lift=1.35)
    hd.nose(0.016, 0.011, depth=0.011, dy=0.004)
    hd.mouth(-0.07, half=0.034, lift=0.012)
    # Big ears sticking out sideways.
    leaf_ear(hd.h, (0.075, 0.085, 0.05), (1.0, 0.45, 0.15), NZ,
             [(0.0, 0.02, 0.011), (0.06, 0.036, 0.011), (0.12, 0.026, 0.008)], 0.16)
    # Antlers: main beam up/out/back, three tines forwards-up; glowing tips.
    beam = [(0.04, 0.11, 0.04), (0.075, 0.19, 0.055), (0.115, 0.28, 0.045), (0.135, 0.35, 0.01),
            (0.14, 0.39, -0.02)]
    tube(hd.h, beam, [0.016, 0.014, 0.011, 0.008, 0.0], sides=5,
         mat=["body", "body", "body", "accent"])
    for p, q, r in (((0.065, 0.17, 0.052), (0.08, 0.22, -0.015), 0.008),
                    ((0.1, 0.245, 0.05), (0.125, 0.3, -0.01), 0.007),
                    ((0.122, 0.305, 0.04), (0.175, 0.35, 0.025), 0.0065)):
        mid = add(add(p, mul(sub(q, p), 0.72)), (0, 0.006, 0.006))
        tube(hd.h, [p, mid, q], [r, r * 0.6, 0.0], sides=4, mat=["body", "accent"])
    hd.flow([(0.11, 0), (-0.05, 0)])
    hd.flow([(-0.06, 118), (-0.02, 120), (0.04, 115)])
    hd.headband(0.055, r=0.009, span=(0, 95))
    return hd


def head_owl():
    hd = Head([(0.1, 0.0, 0.09, 0.1, 2.3, 1.0, 0.9), (0.06, 0.01, 0.125, 0.135, 2.5, 1.0, 0.9),
               (0.0, 0.015, 0.138, 0.14, 2.7, 1.0, 0.9), (-0.04, 0.012, 0.125, 0.13, 2.8, 1.0, 0.9),
               (-0.06, 0.01, 0.08, 0.09, 2.6)],
              eye=(0.05, 0.03, -0.01, 4), radius=0.045, sink=0.5, diverge=0.05)
    # The facial disc: a pale ruff of feathers ringing each eye, meeting at the V brow.
    torus(hd.h, sub(hd.eye_c, mul(hd.eye_fwd, 0.2 * hd.eye_r)), hd.eye_fwd, hd.eye_r * 1.65,
          hd.eye_r * 0.42, mat="fur_light", segs=12, sides=5)
    hd.brow(thick=0.24, inner=0.4, outer=-0.1, lift=1.4)
    # A small hooked beak below the eyes.
    c, hw, hh = hd.front
    beak = [(-0.013, -0.028, c[2]), (0.013, -0.028, c[2]), (0.0, -0.05, c[2])]
    pyramid(hd.g, beak, (0.0, -0.075, c[2] - 0.035), mat="accent", base_mat="accent")
    # Feather tufts angled out from the top corners.
    hd.ear((0.1, 0.11, 0.0), 0.065, 0.04, (0.17, 0.25, 0.0), inner=0.5, inner_shift=-0.1)
    hd.flow([(-0.04, 118), (0.02, 112), (0.08, 106)])
    hd.headband(0.03, r=0.01, span=(22, 100))
    hd.earcup(-0.02, 0.045, r=0.028, depth=0.02)
    return hd


def head_boar():
    def paint(c, n, k):
        if n[1] < -0.45 and c[2] < 0.03:
            return "fur_light"  # underjaw
        return None

    hd = Head([(0.13, 0.045, 0.085, 0.095, 2.2), (0.08, 0.05, 0.13, 0.135, 2.3),
               (0.02, 0.035, 0.14, 0.13, 2.4), (-0.03, 0.015, 0.12, 0.115, 2.5),
               (-0.07, -0.005, 0.085, 0.085, 2.5), (-0.09, -0.015, 0.055, 0.055, 2.4)],
              eye=(0.06, 0.045, -0.015, 24), radius=0.031, paint=paint)
    # A long thick snout ending in a disc.
    hd.muzzle([(-0.05, -0.03, 0.078, 0.085, 2.8), (-0.13, -0.035, 0.07, 0.075, 2.8),
               (-0.19, -0.035, 0.064, 0.066, 3.0), (-0.205, -0.035, 0.058, 0.058, 3.2)],
              paint=lambda c, n, k: "fur_light" if n[1] < -0.4 else None, nose_mat="fur_light")
    hd.brow(thick=0.34, inner=0.3, outer=-0.1)
    # Nose disc nostrils, and a ring through the snout.
    c, hw, hh = hd.front
    for s in (-1, 1):
        plate(hd.g, add(c, (s * 0.022, 0.004, 0)), NZ, L.ngon(0, 0, 0.012, 8), "body", h=0.002,
              depth=0.004)
    torus(hd.g, add(c, (0.0, -0.048, -0.004)), Z, 0.018, 0.003, segs=10, sides=4)
    # Tusks curling up from the mouth corners.
    tube(hd.h, [(0.058, -0.065, -0.13), (0.085, -0.055, -0.165), (0.098, -0.015, -0.19),
                (0.088, 0.02, -0.195)], [0.012, 0.01, 0.006, 0.0], sides=5, mat="accent")
    hd.mouth(-0.075, half=0.05, lift=0.01, z=-0.19, mat="fur_light")
    # Big floppy ears out and forward, and a bristle crest along the top.
    leaf_ear(hd.h, (0.085, 0.115, 0.05), (1.0, 0.7, -0.3), NZ,
             [(0.0, 0.03, 0.012), (0.06, 0.046, 0.012), (0.11, 0.036, 0.009)], 0.15)
    for z, h in ((-0.02, 0.03), (0.02, 0.045), (0.06, 0.055), (0.095, 0.045), (0.12, 0.03)):
        p, n = hd.surface(z, 0.0)
        pyramid(hd.g, [(-0.014, p[1] - 0.02, z - 0.02), (0.014, p[1] - 0.02, z - 0.02),
                       (0.014, p[1] - 0.02, z + 0.02), (-0.014, p[1] - 0.02, z + 0.02)],
                (0.0, p[1] + h - 0.02, z + 0.025))
    hd.flow([(-0.1, 118), (-0.05, 122), (0.03, 118), (0.1, 112)])
    hd.headband(0.05, r=0.01, span=(18, 100))
    hd.earcup(0.0, 0.06, r=0.03)
    return hd


def head_dragon():
    def paint(c, n, k):
        if n[1] < -0.45 and c[2] < 0.05:
            return "fur_light"  # pale throat
        return None

    hd = Head([(0.15, 0.035, 0.065, 0.07, 2.2), (0.1, 0.045, 0.11, 0.11, 2.3),
               (0.04, 0.045, 0.125, 0.115, 2.4), (-0.01, 0.04, 0.115, 0.1, 2.6, 0.95, 1.1),
               (-0.04, 0.04, 0.09, 0.08, 2.6, 0.9, 1.05), (-0.058, 0.045, 0.05, 0.045, 2.5)],
              eye=(0.052, 0.045, -0.02, 22), radius=0.034, paint=paint)
    # A long boxy snout over a wide jaw, pale underneath.
    hd.muzzle([(-0.03, -0.01, 0.09, 0.075, 2.8, 0.55, 1.15), (-0.09, -0.012, 0.075, 0.066, 3.0, 0.8, 1.1),
               (-0.14, -0.018, 0.065, 0.055, 3.0, 0.9, 1.05), (-0.18, -0.025, 0.054, 0.044, 3.2),
               (-0.21, -0.03, 0.044, 0.034, 3.4)],
              paint=lambda c, n, k: "fur_light" if n[1] < -0.3 else None)
    hd.brow(thick=0.42, inner=0.35, outer=-0.12, lift=1.3)
    # Nostrils at the snout tip, and fangs along the lip.
    c, hw, hh = hd.front
    for s in (-1, 1):
        plate(hd.g, add(c, (s * 0.018, 0.012, 0)), NZ, L.ngon(0, 0, 0.007, 6), "body", h=0.002,
              depth=0.004)
    hd.mouth(-0.052, half=0.05, lift=0.012, z=-0.2)
    for x, z, h in ((0.014, -0.19, 0.013), (0.036, -0.165, 0.011), (0.052, -0.13, 0.009)):
        hd.tooth(x, -0.052, z, w=0.008, h=h)
    # Short horns swept back from the brow, each with two glowing bands.
    base, tip, r = (0.06, 0.1, 0.045), (0.1, 0.215, 0.15), 0.026
    bend = lambda t: (0.0, 0.035 * math.sin(math.pi * t), -0.02 * math.sin(math.pi * t))
    path = [add(add(base, mul(sub(tip, base), t)), bend(t)) for t in (0.0, 0.35, 0.7, 1.0)]
    tube(hd.h, path, [r, r * 0.8, r * 0.5, 0.0], sides=7)
    for t in (0.3, 0.58):
        p = add(add(base, mul(sub(tip, base), t)), bend(t))
        q = add(add(base, mul(sub(tip, base), t + 0.05)), bend(t + 0.05))
        torus(hd.h, p, sub(q, p), r * (1.0 - 0.55 * t) * 1.03, 0.004, segs=10, sides=4)
    # Cheek frills: three spines fanning back from behind the jaw, glowing webs between them.
    root = (0.095, -0.01, 0.0)
    tips = [(0.165, 0.075, 0.09), (0.19, 0.0, 0.1), (0.165, -0.07, 0.075)]
    for t in tips:
        tube(hd.h, [root, t], [0.007, 0.0], sides=5)
    for a, b in zip(tips, tips[1:]):
        web = [add(root, (0.0, 0.0, 0.015)), add(a, mul(sub(root, a), 0.08)),
               add(b, mul(sub(root, b), 0.08))]
        hd.h.face(web, "accent", want=X)
        hd.h.face(list(reversed(web)), "accent", want=NX)
    # A crest of small spines down the midline.
    for z, h in ((0.02, 0.028), (0.055, 0.036), (0.09, 0.034), (0.125, 0.026)):
        p, n = hd.surface(z, 0.0)
        pyramid(hd.g, [(-0.012, p[1] - 0.015, z - 0.014), (0.012, p[1] - 0.015, z - 0.014),
                       (0.012, p[1] - 0.015, z + 0.014), (-0.012, p[1] - 0.015, z + 0.014)],
                (0.0, p[1] + h, z + 0.02), mat="body")
    hd.flow([(-0.06, 118), (0.0, 122), (0.07, 115)])
    hd.flow([(0.14, 0), (-0.04, 0)])
    return hd


# ---------------------------------------------------------------------------------------------
# Torso.

def torso():
    g = L.Geo()
    # (y, half width, half depth, chamfer) top to bottom; constant half depth 0.12 from the
    # shoulders to the belly keeps the chest front/back planar for the light bars.
    secs = [(0.0, 0.1, 0.07, 0.03), (-0.03, 0.14, 0.09, 0.035), (-0.075, 0.21, 0.12, 0.045),
            (-0.4, 0.168, 0.12, 0.045), (-0.47, 0.15, 0.108, 0.04), (-0.5, 0.135, 0.092, 0.035)]
    rings = [[(x, y, z) for x, z in L.octagon_xz(hx, hz, ch)] for y, hx, hz, ch in secs]
    loft(g, rings)
    # Shoulder armour plates.
    for s in (-1, 1):
        prof = [(-0.1, -0.05), (0.1, -0.05), (0.11, -0.085), (0.06, -0.12), (-0.06, -0.12),
                (-0.11, -0.085)]  # (z, y)
        loft(g, [[(s * x, y + 0.03 - (x - 0.13) * 0.45, z) for z, y in prof]
                 for x in (0.13, 0.228)])
    # Accent trim: collar band, hem band.
    def band(y0, y1, grow):
        def r(y):
            for (ya, ax, az, ac), (yb, bx, bz, bc) in zip(secs, secs[1:]):
                if yb <= y <= ya:
                    t = (ya - y) / (ya - yb)
                    return [(x, y, z) for x, z in L.octagon_xz(ax + (bx - ax) * t + grow,
                                                                az + (bz - az) * t + grow,
                                                                ac + (bc - ac) * t)]
        loft(g, [r(y0), r(y1)], mat="accent", cap0=None, cap1=None)
    band(-0.006, -0.014, 0.003)
    band(-0.452, -0.462, 0.003)
    # Chest panel outline + hit bars, front and back.
    for zf, U in ((-0.12, NX), (0.12, X)):
        f = L.Frame((0, 0, zf), U, Y)
        L.ring(g, f, L.chamfer_rect(-0.125, -0.36, 0.125, -0.105, 0.025), 0.007, glow_in=0.012,
               glow_out=0.016, h=0.003)
        for mat, v in (("hit_1", -0.155), ("hit_2", -0.23), ("hit_3", -0.305)):
            L.filled(g, f, L.chamfer_rect(-0.085, v - 0.018, 0.085, v + 0.018, 0.012), glow=0,
                     mat=mat, h=0.004)
    # Flowing light lines (pulses run from the collar down): on each side, front and back, one
    # over the yoke, past the chest panel's upper corner and down the chamfered edge to the hem,
    # and one along the top of each shoulder plate from the neck outwards.
    solid = L.Geo()
    solid.faces = [f for f in g.faces if f[1] == "body"]

    def sec(y):
        for (ya, ax, az, ac), (yb, bx, bz, bc) in zip(secs, secs[1:]):
            if yb <= y <= ya:
                t = (ya - y) / (ya - yb)
                return ax + (bx - ax) * t, az + (bz - az) * t, ac + (bc - ac) * t

    def chamfer_mid(y):
        hx, hz, ch = sec(y)
        return hx - ch * 0.5, hz - ch * 0.5

    def to_axis(p):
        d = norm((p[0], 0.0, p[2]))
        return add((0.0, p[1], 0.0), mul(d, 0.5)), mul(d, -1)

    for sx in (-1, 1):
        for sz in (-1, 1):
            guide = [(0.055, -0.017, 0.08), (0.075, -0.05, 0.1), (0.095, -0.088, 0.12),
                     (0.15, -0.128, 0.12)]
            for y in (-0.16, -0.24, -0.32, -0.4, -0.425, -0.446):
                x, z = chamfer_mid(y)
                guide.append((x, y, z))
            guide = [(sx * x, y, sz * z) for x, y, z in guide]
            ray_strip(g, [solid], guide, to_axis, 0.008, step=0.008)
        ray_strip(g, [solid], [(sx * 0.142, 0.0, 0.0), (sx * 0.222, 0.0, 0.0)],
                  lambda p: ((p[0], 0.3, p[2]), NY), 0.008, step=0.02)
    return g


# ---------------------------------------------------------------------------------------------
# Hands.

def hand_left():
    """Left hand: palm faces +X, fingers -Z (curled towards +X), thumb on top."""
    g = L.Geo()
    xb = -0.017  # flat back of the hand

    def palm_ring(z, x1, hy0, hy1, ch=0.007):
        pts = L.chamfer_rect(xb, hy0, x1, hy1, ch)
        return [(x, y, z) for x, y in pts]

    loft(g, [palm_ring(0.045, 0.01, -0.036, 0.036), palm_ring(0.0, 0.014, -0.046, 0.046),
             palm_ring(-0.045, 0.012, -0.046, 0.042)])
    glove_cuff(g)
    # Fingers (index on top), curled towards the palm side.
    for y, l, w in ((0.031, 1.0, 0.0125), (0.0105, 1.06, 0.0127), (-0.0095, 1.0, 0.0122),
                    (-0.029, 0.84, 0.0112)):
        p0 = (-0.003, y, -0.04)
        p1 = (-0.003, y, -0.04 - 0.032 * l)
        p2 = add(p1, (0.016 * l, 0, -0.022 * l))
        p3 = add(p2, (0.022 * l, 0, -0.004 * l))
        p4 = add(p3, (0.012 * l, 0, 0.008 * l))
        tube(g, [p0, p1, p2, p3, p4], [w, w, w * 0.95, w * 0.9, w * 0.85], sides=4, tip=False,
             phase=45.0)
    # Thumb along the top, pointing forward and in.
    tube(g, [(0.0, 0.038, 0.02), (0.012, 0.05, -0.008), (0.024, 0.052, -0.036),
             (0.032, 0.046, -0.054)], [0.015, 0.014, 0.0125, 0.0115], sides=4, tip=False)
    # Knuckle guard bar across the back of the hand, and three light lines flowing into it from
    # the cuff band.
    plate(g, (xb, 0.0, -0.036), NX, [(-0.004, -0.034), (0.004, -0.034), (0.004, 0.032),
                                     (-0.004, 0.032)], h=0.002, depth=0.003, uhint=NZ, vhint=Y)
    solid = L.Geo()
    solid.faces = [f for f in g.faces if f[1] == "body"]
    for y0, y1 in ((0.026, 0.02), (0.0, 0.0), (-0.028, -0.021)):
        ray_strip(g, [solid], [(0.0, y0, 0.066), (0.0, (y0 + y1) / 2, 0.02), (0.0, y1, -0.0325)],
                  lambda p: ((-0.3, p[1], p[2]), X), 0.004, step=0.006, h=0.002, depth=0.003)
    return g


def glove_cuff(g):
    """The gloves' cuff (shared by the open and gripping gloves: the forearm slides into it),
    with its accent band."""
    def ring(z, x1, h):
        return [(x - 0.004 if x < 0 else x, y, z)
                for x, y in L.chamfer_rect(-0.017, -h, x1, h, 0.01)]
    cuff = [ring(0.04, 0.013, 0.04), ring(0.098, 0.015, 0.043)]
    loft(g, cuff)
    cc = centroid(cuff[0])

    def cuff_at(t, grow):
        r = [add(a, mul(sub(b, a), t)) for a, b in zip(*cuff)]
        c = add(cc, mul(sub(centroid(cuff[1]), cc), t))
        out = []
        for p in r:
            d = sub(p, c)
            d = norm((d[0], d[1], 0.0))
            out.append(add(p, mul(d, grow)))
        return out
    loft(g, [cuff_at(0.5, 0.0018), cuff_at(0.65, 0.0018)], mat="accent", cap0=None, cap1=None)


# The guns' grip (blaster.py, powerups.py's shotgun), in the guns' frame = the hand's frame:
# tilted back GRIP_TILT, octagonal section 0.03 m wide and 0.044 m deep about the origin.
GRIP_TILT = math.radians(22)
GRIP_A = (0.0, math.cos(GRIP_TILT), -math.sin(GRIP_TILT))  # up the grip
GRIP_D = (0.0, math.sin(GRIP_TILT), math.cos(GRIP_TILT))  # towards its back


def grip_pt(t, x, d):
    """A point t up the grip's axis, x across, d towards its back."""
    return add(mul(GRIP_A, t), add((x, 0.0, 0.0), mul(GRIP_D, d)))


def hand_grip_left():
    """Left glove wrapped round the guns' grip (shown while it holds one): the palm along the
    grip's left side, middle, ring and little finger round its front below the trigger guard
    onto its right side, the index finger on the trigger, the thumb across the back strap under
    the receiver and along the frame's right side. Same origin and cuff as `hand_left`."""
    g = L.Geo()

    def slab(t0, t1, xc, d, th, ch=0.007):
        """A ring across the hand: t0..t1 up the grip, `th` thick about x = xc, at depth d."""
        return [grip_pt(t, xc + x, d) for x, t in L.chamfer_rect(-th, t0, th, t1, ch)]

    # Palm: from inside the cuff along the grip's left side to the knuckles at its front.
    back = [(x, y, 0.045) for x, y in L.chamfer_rect(-0.019, -0.036, 0.011, 0.036, 0.007)]
    loft(g, [back, slab(-0.058, 0.036, -0.027, 0.02, 0.0125),
             slab(-0.06, 0.04, -0.028, -0.004, 0.013), slab(-0.058, 0.038, -0.026, -0.022, 0.012)])
    # Middle, ring and little finger round the front onto the right side.
    for t, w, k in ((-0.012, 0.0125, 1.0), (-0.032, 0.0122, 0.98), (-0.05, 0.011, 0.9)):
        path = [(-0.026, -0.016), (-0.012, -0.0315), (0.01, -0.0315), (0.0245, -0.014),
                (0.0245, 0.006 * k)]
        tube(g, [grip_pt(t, x, d) for x, d in path], [w, w, w * 0.95, w * 0.9, w * 0.85],
             sides=4, tip=False, up=GRIP_A)
    # Index finger forward from its knuckle, through the guard and curled round the trigger.
    tube(g, [grip_pt(0.024, -0.025, -0.018), (-0.019, 0.024, -0.046), (-0.006, 0.022, -0.066),
             (0.0, 0.017, -0.058)], [0.012, 0.012, 0.011, 0.0105], sides=4, tip=False)
    # Thumb from the heel across the back strap under the receiver and along its right side.
    tube(g, [(-0.022, 0.026, 0.03), (-0.008, 0.036, 0.024), (0.012, 0.037, 0.016),
             (0.026, 0.035, -0.004), (0.029, 0.033, -0.026)],
         [0.015, 0.014, 0.013, 0.0125, 0.0115], sides=4, tip=False)
    glove_cuff(g)
    # Knuckle guard on the back of the hand at the knuckles, and three light lines flowing into
    # it from the cuff band.
    kc = grip_pt(-0.01, -0.0385, -0.016)
    plate(g, kc, NX, [(-0.004, -0.04), (0.004, -0.04), (0.004, 0.034), (-0.004, 0.034)],
          h=0.002, depth=0.003, uhint=mul(GRIP_D, -1), vhint=GRIP_A)
    solid = L.Geo()
    solid.faces = [f for f in g.faces if f[1] == "body"]
    for y0, t1 in ((0.026, 0.022), (0.0, -0.01), (-0.028, -0.042)):
        end = grip_pt(t1, 0.0, -0.011)
        mid = grip_pt((t1 + y0) / 2, 0.0, 0.026)
        ray_strip(g, [solid], [(0.0, y0, 0.066), (0.0, (y0 + mid[1]) / 2, 0.042), mid, end],
                  lambda p: ((-0.3, p[1], p[2]), X), 0.004, step=0.01, h=0.002, depth=0.003)
    return g


def mirrored(geo):
    m = L.Geo()
    m.faces = [mirror_face(f) for f in geo.faces]
    return m


# ---------------------------------------------------------------------------------------------
# Body segments: rigid pieces the game poses by IK. Each hangs from its origin (the joint it
# turns about) down -Y, front -Z, X mirror symmetric; the game stretches it along Y a little.

def sec_at(secs, y):
    """(half width, half depth, chamfer) of the octagon loft `secs` at height y."""
    for (ya, ax, az, ac), (yb, bx, bz, bc) in zip(secs, secs[1:]):
        if yb <= y <= ya:
            t = (ya - y) / (ya - yb)
            return ax + (bx - ax) * t, az + (bz - az) * t, ac + (bc - ac) * t
    raise ValueError("y = %.3f outside the loft" % y)


def segment(secs, bands=(), flows=(), flow_w=0.006, front_flow=None):
    """Octagonal loft through secs = [(y, half width, half depth, chamfer)] top to bottom, with
    accent bands (y0, y1) and `flow` lines down its sides at the given (y0, y1) (one each side,
    on the X faces) and optionally one down the front at x = 0."""
    g = L.Geo()
    loft(g, [[(x, y, z) for x, z in L.octagon_xz(hx, hz, ch)] for y, hx, hz, ch in secs])
    for y0, y1 in bands:
        rings = []
        for y in (y0, y1):
            hx, hz, ch = sec_at(secs, y)
            rings.append([(x, y, z) for x, z in L.octagon_xz(hx + 0.003, hz + 0.003, ch)])
        loft(g, rings, mat="accent", cap0=None, cap1=None)
    solid = L.Geo()
    solid.faces = [f for f in g.faces if f[1] == "body"]
    for y0, y1 in flows:
        for s in (-1, 1):
            ray_strip(g, [solid], [(s * 0.01, y0, 0.0), (s * 0.01, y1, 0.0)],
                      lambda p, s=s: ((s * 0.4, p[1], 0.0), (-s, 0.0, 0.0)), flow_w, step=0.02)
    if front_flow:
        y0, y1 = front_flow
        ray_strip(g, [solid], [(0.0, y0, 0.0), (0.0, y1, 0.0)],
                  lambda p: ((0.0, p[1], -0.4), Z), flow_w, step=0.02, mid=True)
    return g


def neck():
    """From the neck point (the head's pivot) down 0.2 m into the vest's collar."""
    secs = [(0.03, 0.035, 0.04, 0.012), (-0.03, 0.048, 0.052, 0.016),
            (-0.17, 0.056, 0.058, 0.018), (-0.235, 0.072, 0.065, 0.022)]
    return segment(secs, bands=[(-0.16, -0.172)], flows=[(-0.035, -0.15)], flow_w=0.005)


def pelvis():
    """Hips, origin at the top centre (tucked 4 cm up into the vest's hem), hip joints at
    (+-0.095, -0.12, 0). Round enough at the top to turn inside the hem."""
    secs = [(0.0, 0.088, 0.074, 0.03), (-0.05, 0.118, 0.084, 0.034),
            (-0.11, 0.132, 0.09, 0.038), (-0.17, 0.11, 0.08, 0.034), (-0.205, 0.06, 0.052, 0.02)]
    g = segment(secs, bands=[(-0.055, -0.068)])
    # Buckle, and light lines down the front from the belt to the crotch.
    plate(g, (0.0, -0.0615, -0.087), NZ, L.chamfer_rect(-0.022, -0.012, 0.022, 0.012, 0.005),
          h=0.004, depth=0.006)
    solid = L.Geo()
    solid.faces = [f for f in g.faces if f[1] == "body"]
    for s in (-1, 1):
        ray_strip(g, [solid], [(s * 0.07, -0.075, 0.0), (s * 0.035, -0.18, 0.0)],
                  lambda p: ((p[0], p[1], -0.4), Z), 0.005, step=0.01)
    return g


def upper_arm():
    """Shoulder ball at the origin, the sleeve 0.27 m down to the elbow."""
    secs = [(0.045, 0.028, 0.028, 0.01), (0.02, 0.052, 0.052, 0.017), (-0.025, 0.058, 0.058, 0.02),
            (-0.075, 0.048, 0.048, 0.016), (-0.25, 0.04, 0.04, 0.013), (-0.285, 0.032, 0.032, 0.011)]
    return segment(secs, bands=[(-0.06, -0.072)], flows=[(-0.09, -0.24)], flow_w=0.005)


def forearm():
    """Elbow cap at the origin, a bracer 0.25 m down to the wrist, flattening to slide into the
    glove's cuff (narrow in X, the glove's palm axis; deep in Z, the glove's -Y)."""
    secs = [(0.03, 0.028, 0.028, 0.01), (0.0, 0.044, 0.044, 0.015), (-0.05, 0.044, 0.048, 0.016),
            (-0.17, 0.028, 0.04, 0.011), (-0.265, 0.016, 0.034, 0.007)]
    return segment(secs, bands=[(-0.04, -0.052)], flows=[(-0.06, -0.17)], flow_w=0.005)


def thigh():
    """Hip ball at the origin, the thigh 0.38 m down to the knee, an accent knee guard."""
    secs = [(0.05, 0.035, 0.035, 0.012), (0.02, 0.064, 0.064, 0.022), (-0.06, 0.074, 0.074, 0.025),
            (-0.3, 0.055, 0.058, 0.019), (-0.38, 0.05, 0.052, 0.017), (-0.41, 0.034, 0.034, 0.012)]
    g = segment(secs, bands=[(-0.07, -0.082)], flows=[(-0.1, -0.3)])
    plate(g, (0.0, -0.37, -0.052), NZ, L.chamfer_rect(-0.03, -0.035, 0.03, 0.03, 0.012), h=0.006,
          depth=0.012)
    return g


def shin():
    """Knee at the origin, the shin 0.37 m down to the ankle, a light line down the guard."""
    secs = [(0.035, 0.038, 0.038, 0.013), (0.0, 0.054, 0.058, 0.018), (-0.1, 0.057, 0.062, 0.02),
            (-0.3, 0.042, 0.046, 0.015), (-0.37, 0.04, 0.042, 0.014), (-0.395, 0.03, 0.03, 0.01)]
    return segment(secs, bands=[(-0.31, -0.322)], front_flow=(-0.05, -0.29))


def boot():
    """Ankle at the origin, sole 0.08 m below, toe 0.17 m forward (-Z), heel 0.06 m back."""
    g = L.Geo()

    def ring(z, hx, y0, y1, ch):
        return [(x, y, z) for x, y in L.chamfer_rect(-hx, y0, hx, y1, ch)]
    loft(g, [ring(0.065, 0.04, -0.072, -0.0, 0.015), ring(0.03, 0.05, -0.072, 0.03, 0.018),
             ring(-0.06, 0.052, -0.072, 0.0, 0.018), ring(-0.14, 0.045, -0.072, -0.035, 0.014),
             ring(-0.172, 0.032, -0.07, -0.05, 0.008)])
    # Ankle collar the shin sinks into.
    loft(g, [[(x, y, z + 0.01) for x, z in L.octagon_xz(hx, hz, ch)]
             for y, hx, hz, ch in ((0.045, 0.045, 0.046, 0.015), (-0.02, 0.05, 0.05, 0.017))])
    # Accent sole, a little proud all round.
    loft(g, [ring(0.07, 0.044, -0.08, -0.068, 0.006), ring(-0.06, 0.056, -0.08, -0.068, 0.006),
             ring(-0.178, 0.036, -0.08, -0.068, 0.004)], mat="accent")
    solid = L.Geo()
    solid.faces = [f for f in g.faces if f[1] == "body"]
    ray_strip(g, [solid], [(0.0, 0.0, -0.03), (0.0, 0.0, -0.15)],
              lambda p: ((0.0, 0.4, p[2]), NY), 0.006, step=0.01, mid=True)
    return g


SEGMENTS = (("neck", neck), ("pelvis", pelvis), ("upper_arm", upper_arm), ("forearm", forearm),
            ("thigh", thigh), ("shin", shin), ("boot", boot))


# ---------------------------------------------------------------------------------------------
# Tails: chains of rigid pieces the game swings about their joints (src/tail.rs). Each piece
# `tail_<animal>_<k>` runs from its joint (the origin) along -Z, +Y its top, X mirror symmetric,
# and reaches a little back into the piece before (rounded), so no gap opens as a joint bends.
# The empties `tail_<animal>_j<k>` place joint k on the pelvis at rest (in the pelvis's frame:
# top centre, front -Z), the last one at the tip.

# Where a tail leaves the pelvis: low on its back.
TAIL_ROOT = (0.0, -0.125, 0.075)

# Per animal: the pieces' lengths and the angles (degrees above the horizontal) they leave
# backwards at; the radius along the tail [(arc fraction, m)]; the section's width and height
# per radius; a point at the tip (else rounded); accent bands (arc fractions); a light line down
# the top; and the material by arc fraction.
TAILS = {
    "wolf": dict(lengths=(0.12,) * 5, angles=(-30, -48, -60, -68, -72),
                 radius=((0, 0.024), (0.35, 0.048), (0.8, 0.036), (1, 0.0)), pointy=True,
                 bands=(0.08,), flow=True),
    "fox": dict(lengths=(0.13, 0.14, 0.14, 0.14, 0.13), angles=(-20, -32, -30, -20, -8),
                radius=((0, 0.026), (0.5, 0.066), (0.85, 0.048), (1, 0.0)), pointy=True,
                bands=(0.08,), flow=True, mat=lambda s: "fur_light" if s > 0.8 else "body"),
    "cat": dict(lengths=(0.1,) * 8, angles=(-25, -45, -62, -68, -55, -30, 0, 30),
                radius=((0, 0.019), (0.3, 0.016), (1, 0.011)),
                bands=(0.17, 0.31, 0.45, 0.59, 0.73, 0.87),
                mat=lambda s: "accent" if s > 0.96 else "body"),
    "deer": dict(lengths=(0.11,), angles=(35,), radius=((0, 0.024), (0.5, 0.03), (1, 0.012)),
                 aspect=(1.3, 0.55), flow=True, belly="fur_light"),
    "bear": dict(lengths=(0.05,), angles=(-15,), ball=(0.025, 0.036), mat=lambda s: "body",
                 band=True),
    "rabbit": dict(lengths=(0.06,), angles=(5,), ball=(0.03, 0.042),
                   mat=lambda s: "fur_light"),
    "owl": dict(lengths=(0.17,), angles=(-55,), fan=True),
    "boar": dict(lengths=(0.06, 0.06, 0.055), angles=(-25, -80, -140),
                 radius=((0, 0.011), (1, 0.006)), bands=(0.15,), tuft=True),
    "dragon": dict(lengths=(0.14, 0.14, 0.13, 0.12, 0.11, 0.1), angles=(-30, -45, -50, -40, -25, -10),
                   radius=((0, 0.055), (0.35, 0.04), (1, 0.012)), aspect=(1.0, 0.9), spikes=True,
                   spade=True),
}


def tail_joints(spec):
    pts = [TAIL_ROOT]
    for l, a in zip(spec["lengths"], spec["angles"]):
        a = math.radians(a)
        pts.append(add(pts[-1], (0.0, l * math.sin(a), l * math.cos(a))))
    return pts


def profile(prof, f):
    """Piecewise linear [(x, y)] at x = f (clamped)."""
    f = min(max(f, prof[0][0]), prof[-1][0])
    for (xa, ya), (xb, yb) in zip(prof, prof[1:]):
        if xa <= f <= xb:
            return ya + (yb - ya) * (f - xa) / max(xb - xa, 1e-9)
    return prof[-1][1]


def tail_ring(t, r, aspect):
    hx, hy = r * aspect[0], r * aspect[1]
    return [(x, y, -t) for x, y in L.octagon_xz(hx, hy, 0.55 * min(hx, hy))]


def ball_piece(c, R, mat):
    """A round puff centred `c` along the piece, `R` across, sunk a little into the pelvis."""
    g = L.Geo()
    rings = [tail_ring(c - R * math.cos(math.radians(a)), R * math.sin(math.radians(a)), (1, 1))
             for a in (20, 50, 80, 110, 140, 165)]
    loft(g, rings, mat=mat)
    return g


def fan_piece():
    """An owl's tail: five flat feathers fanned out sideways, layered from the middle up."""
    g = L.Geo()
    loft(g, [tail_ring(t, r, (1.2, 0.8)) for t, r in ((-0.02, 0.012), (0.0, 0.026), (0.03, 0.02))])
    for i in (-2, -1, 0, 1, 2):
        a = math.radians(16.0 * i)
        d = (math.sin(a), 0.0, -math.cos(a))
        u = (math.cos(a), 0.0, math.sin(a))
        lift = 0.004 * abs(i)
        lf = 0.17 - 0.015 * abs(i)
        rings = [box_ring(add((0.0, lift, 0.0), mul(d, t)), u, Y, hu, 0.005, ch=0.002)
                 for t, hu in ((0.0, 0.011), (0.6 * lf, 0.022), (0.9 * lf, 0.018))]
        rings.append([add((0.0, lift, 0.0), mul(d, lf + 0.02))])
        loft(g, rings, mat=["body", "body", "accent"])
    return g


def tail_pieces(spec):
    """The pieces of one tail (Geos, each in its own joint's frame)."""
    lengths = spec["lengths"]
    total = sum(lengths)
    if spec.get("fan"):
        return [fan_piece()]
    if "ball" in spec:
        c, R = spec["ball"]
        g = ball_piece(c, R, spec["mat"](0.0))
        if spec.get("band"):
            loft(g, [tail_ring(c + dt, R * 0.72 + 0.003, (1, 1)) for dt in (0.012, 0.022)],
                 mat="accent", cap0=None, cap1=None)
        return [g]
    prof, aspect = spec["radius"], spec.get("aspect", (1.0, 1.0))
    matf = spec.get("mat", lambda s: "body")
    rad = lambda s: profile(prof, s / total)
    pieces, s0 = [], 0.0
    for k, L_ in enumerate(lengths):
        last = k == len(lengths) - 1
        g = L.Geo()
        r0 = rad(s0)
        ts = [(-0.5 * r0, 0.55 * r0), (-0.22 * r0, 0.92 * r0)]
        ts += [(L_ * i / 3.0, rad(s0 + L_ * i / 3.0)) for i in range(4)]
        apex = last and spec.get("pointy")
        if last and not apex and not spec.get("spade") and not spec.get("tuft"):
            re = rad(total)
            ts += [(L_ + 0.3 * re, 0.85 * re), (L_ + 0.55 * re, 0.45 * re)]
        rings = [tail_ring(t, max(r, 0.003), aspect) for t, r in ts]
        if apex:
            rings[-1] = [(0.0, 0.0, -L_)]
        mats = [matf((s0 + max(0.0, (ta + tb) / 2)) / total)
                for (ta, _), (tb, _) in zip(ts, ts[1:])]
        belly = spec.get("belly")
        paint = (lambda fc, out, seg: belly if out[1] < -0.35 else None) if belly else None
        loft(g, rings, mat=mats, cap0=mats[0], cap1=mats[-1], paint=paint)
        for f in spec.get("bands", ()):
            t = f * total - s0
            if 0.01 <= t <= L_ - 0.02:
                loft(g, [tail_ring(tt, rad(s0 + tt) + 0.003, aspect) for tt in (t, t + 0.012)],
                     mat="accent", cap0=None, cap1=None)
        if spec.get("flow"):
            solid = L.Geo()
            solid.faces = [f for f in g.faces if f[1] in SOLID]
            t1 = L_ * (0.7 if apex else 0.95)
            n0 = len(g.faces)
            ray_strip(g, [solid], [(0.0, 0.0, -0.01), (0.0, 0.0, -t1)],
                      lambda p: ((0.0, 0.4, p[2]), NY), 0.005, step=0.02, mid=True)
            # And down each side, so it reads from beside the player too.
            for sx in (-1, 1):
                ray_strip(g, [solid], [(0.0, 0.0, -0.01), (0.0, 0.0, -t1)],
                          lambda p, sx=sx: ((sx * 0.4, 0.0, p[2]), (-sx, 0.0, 0.0)), 0.005,
                          step=0.02)
            # Arc length along the whole tail, so the pulses run on from piece to piece.
            g.faces[n0:] = [(p, m, [(u + s0, v) for u, v in uv]) if m == "flow" else (p, m, uv)
                            for p, m, uv in g.faces[n0:]]
        if spec.get("spikes"):
            t, r = 0.5 * L_, rad(s0 + 0.5 * L_)
            top = r * aspect[1] - 0.002
            w, l, h = 0.3 * r, 0.5 * r, 0.9 * r
            pyramid(g, [(-w, top, -t + l), (w, top, -t + l), (w, top, -t - l), (-w, top, -t - l)],
                    (0.0, top + h, -t - 1.4 * l), mat="accent", base_mat="accent")
        if last and spec.get("spade"):
            loft(g, [box_ring((0.0, 0.0, -t), X, Y, hu, 0.006, ch=0.002)
                     for t, hu in ((L_ - 0.01, 0.009), (L_ + 0.03, 0.045))]
                 + [[(0.0, 0.0, -(L_ + 0.09))]], mat="accent")
        if last and spec.get("tuft"):
            re = rad(total)
            loft(g, [tail_ring(L_ - 0.005, re, aspect), tail_ring(L_ + 0.012, 2.3 * re, aspect),
                     [(0.0, 0.0, -(L_ + 0.045))]], mat="fur_light")
        pieces.append(g)
        s0 += L_
    return pieces


def tail_nodes(name, joints):
    """The joint empties: each at its joint, -Z along its piece, +Y its top."""
    for k, p in enumerate(joints):
        a, b = (p, joints[k + 1]) if k + 1 < len(joints) else (joints[k - 1], p)
        d = norm(sub(b, a))
        top = cross(mul(d, -1), NX)
        L.make_empty("%s_j%d" % (name, k), p, fwd=d, up=top)


# ---------------------------------------------------------------------------------------------

def main():
    L.reset_scene()
    L.standard_materials()
    L.material("fur_light", base=(0.16, 0.16, 0.17), rough=0.6)
    for h in ("hit_1", "hit_2", "hit_3"):
        L.material(h, base=(1, 1, 1), emit=(1, 1, 1), strength=3.0)
    L.material("eyes", base=(1, 1, 1), rough=0.4, emit=(1, 1, 1), strength=6.0)
    L.material("sclera", base=(0.8, 0.8, 0.82), rough=0.45)
    L.material("glow_white", base=(1, 1, 1), rough=0.4, emit=(1, 1, 1), strength=4.0)
    L.material("eye_glow", base=(1, 1, 1), rough=1.0, emit=(1, 1, 1), strength=1.5, alpha=0.5)
    L.material("flow", base=(1, 1, 1), rough=0.4, emit=(1, 1, 1), strength=3.0)
    pieces = []
    for n in HEADS:
        hd = globals()["head_" + n]()
        pieces.append(("head_" + n, hd.finish()))
        hd.eye_nodes("head_" + n, LIDS[n])
    pieces += [("eyeball", eyeball()), ("eyeball_cat", eyeball(slit=True)), ("eyelid", eyelid())]
    pieces += [("torso", torso())]
    left = hand_left()
    pieces += [("hand_left", left), ("hand_right", mirrored(left))]
    grip = hand_grip_left()
    pieces += [("hand_grip_left", grip), ("hand_grip_right", mirrored(grip))]
    pieces += [(name, build()) for name, build in SEGMENTS]
    for n in HEADS:
        spec = TAILS[n]
        pieces += [("tail_%s_%d" % (n, k), g) for k, g in enumerate(tail_pieces(spec))]
        tail_nodes("tail_" + n, tail_joints(spec))
    for name, g in pieces:
        if not name.startswith(("hand_", "eyeball")) and name != "eyelid":
            bad = g.mirror_errors()
            if bad:
                raise RuntimeError("%s is not X mirror symmetric: %s" % (name, bad[:5]))
        smooth = (SMOOTH if name.startswith("head_") else
                  SMOOTH_EYES if name.startswith("eyeball") or name == "eyelid" else None)
        L.make_object(name, g, smooth=smooth)
        mn = [min(p[i] for p in g.positions()) for i in range(3)]
        mx = [max(p[i] for p in g.positions()) for i in range(3)]
        print("avatar %-12s %4d tris, extent %s .. %s" % (
            name, g.tris(), [round(v, 3) for v in mn], [round(v, 3) for v in mx]))
    L.save_blend("assets-src/blend/avatars.blend")
    L.export_glb("assets/models/avatars.glb")


main()
