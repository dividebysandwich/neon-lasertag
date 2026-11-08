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

HEADS = ("wolf", "cat", "deer", "fox", "bear", "rabbit", "owl", "boar")
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
# Heads.

ANG = (0, 22.5, 45, 67.5, 90, 112.5, 135, 157.5, 180)


def head_ring(z, cy, hw, hh, n=2.4, wt=1.0, wb=1.0):
    """16-sided superellipse section at z (top and bottom vertices on the mirror plane);
    wt/wb scale the width of the upper/lower half."""
    half = []
    for a in ANG:
        s, c = math.sin(math.radians(a)), math.cos(math.radians(a))
        f = wb + (wt - wb) * (c + 1) / 2
        half.append((hw * f * abs(s) ** (2 / n), cy + hh * math.copysign(abs(c) ** (2 / n), c), z))
    return half + [(-x, y, z) for x, y, z in reversed(half[1:-1])]


class Head:
    """secs: (z, cy, hw, hh[, n, wt, wb]) from the back of the skull to the snout tip.
    eye: (target x, y, z, yaw) - a ray towards the target, turned `yaw` degrees outwards from
    straight ahead, finds the right eye; its midpoint with the left eye becomes the origin."""

    def __init__(self, secs, eye, paint=None, nose_mat="body"):
        self.g = L.Geo()  # symmetric part
        self.h = L.Geo()  # +X half, mirrored in finish()
        self.rings = [head_ring(*s) for s in secs]
        self.secs = secs
        loft(self.g, self.rings, paint=paint, cap1=nose_mat)
        tx, ty, tz, yaw = eye
        self.eye_n_out = (math.sin(math.radians(yaw)), 0.0, -math.cos(math.radians(yaw)))
        self.eye_u = (math.cos(math.radians(yaw)), 0.0, math.sin(math.radians(yaw)))
        self.eye_c, self.eye_n = raycast([self.g], add((tx, ty, tz), mul(self.eye_n_out, 0.4)),
                                         mul(self.eye_n_out, -1))

    @property
    def front(self):
        """(centre, half width, half height) of the snout's end cap."""
        s = self.secs[-1]
        return (0.0, s[1], s[0]), s[2], s[3]

    def eye(self, shape, pupil=None, mat="eyes", halo=0.012, seg=0.022):
        plate(self.h, self.eye_c, self.eye_n, shape, mat, uhint=self.eye_u)
        if pupil:
            plate(self.h, self.eye_c, self.eye_n, pupil, "body", h=0.0045, uhint=self.eye_u)
        if halo:
            eye_halo(self.h, [self.g], self.eye_c, self.eye_n, shape, halo, uhint=self.eye_u,
                     seg=seg)

    def axis_y(self, z):
        zs = [s[0] for s in self.secs]
        if z >= zs[0]:
            return self.secs[0][1]
        for a, b in zip(self.secs, self.secs[1:]):
            if b[0] <= z <= a[0]:
                t = (a[0] - z) / (a[0] - b[0])
                return a[1] + (b[1] - a[1]) * t
        return self.secs[-1][1]

    def flow(self, guide, w=0.005, step=0.01):
        """A light line over the skull through guide = [(z, angle)]: angle in degrees around
        the head's long axis from the top (+Y) towards +X; 0 everywhere puts one line on the
        mirror plane, otherwise it is drawn on the +X half and mirrored."""
        mid = all(abs(a) < 1e-6 for _, a in guide)

        def ray(g):
            z, a = g
            d = (math.sin(math.radians(a)), math.cos(math.radians(a)), 0.0)
            c = (0.0, self.axis_y(z), z)
            return add(c, mul(d, 0.5)), mul(d, -1)
        # Resample in metres (angles at a ~0.1 m radius).
        scaled = [(z, math.radians(a) * 0.1) for z, a in guide]
        pts = [(z, math.degrees(a / 0.1)) for z, a in resample(scaled, step)]
        samples = [raycast([self.g], *ray(g)) for g in pts]
        return flow_strip(self.g if mid else self.h, samples, w, mid=mid)

    def nose(self, shape, mat="accent", dy=0.0):
        c, _, _ = self.front
        plate(self.g, add(c, (0, dy, 0)), NZ, shape, mat, depth=0.004)

    def decal(self, target, d, shape, mat="accent", half=True, **kw):
        return decal(self.h if half else self.g, [self.g], target, d, shape, mat, **kw)

    def ear(self, bc, w, dz, apex, inner=0.55, inner_shift=0.0, tilt=0.0):
        """Pointed ear: pyramid over a w x dz base (sunk into the skull at bc), the front face
        carrying an accent inner."""
        hw, hd = w / 2, dz / 2
        base = [add(bc, (-hw, -tilt, -hd)), add(bc, (hw, tilt, -hd)), add(bc, (hw, tilt, hd)),
                add(bc, (-hw, -tilt, hd))]
        sides = pyramid(self.h, base, apex)
        front = sides[0]
        sh = mul(sub(apex, centroid(front)), inner_shift)
        face_plate(self.h, front, NZ, k=inner, shift=sh)
        return front

    def finish(self):
        for f in self.h.faces:
            self.g.faces.append(f)
            self.g.faces.append(mirror_face(f))
        translate(self.g, (0.0, -self.eye_c[1], -self.eye_c[2]))
        return self.g


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


ALMOND = [(-0.016, -0.003), (-0.004, -0.008), (0.012, -0.006), (0.019, 0.004), (0.006, 0.007),
          (-0.012, 0.004)]


def scale2(shape, s, sv=None):
    sv = s if sv is None else sv
    return [(u * s, v * sv) for u, v in shape]


def head_wolf():
    def paint(c, n, k):
        if c[2] < 0.0 and n[1] < -0.35:
            return "fur_light"  # jaw and throat
        if c[2] < -0.07 and c[1] < -0.035:
            return "fur_light"  # lower muzzle
        return None

    hd = Head([(0.13, 0.03, 0.06, 0.07), (0.09, 0.035, 0.095, 0.105), (0.03, 0.03, 0.112, 0.112),
               (-0.02, 0.02, 0.105, 0.098), (-0.06, 0.0, 0.07, 0.07), (-0.085, -0.02, 0.058, 0.058, 2.6),
               (-0.16, -0.032, 0.048, 0.045, 2.6), (-0.2, -0.036, 0.036, 0.032, 2.6)],
              eye=(0.05, 0.035, 0.0, 15), paint=paint)
    hd.eye(ALMOND)
    hd.nose([(-0.016, 0.012), (0.016, 0.012), (0.022, 0.002), (0.0, -0.016), (-0.022, 0.002)],
            dy=0.008)
    hd.ear((0.06, 0.1, 0.045), 0.055, 0.035, (0.08, 0.2, 0.065), inner=0.6, inner_shift=-0.1)
    hd.flow([(-0.06, 14), (-0.03, 22), (0.0, 18), (0.04, 10), (0.09, 10), (0.12, 14)])
    hd.flow([(-0.07, 105), (-0.02, 112), (0.04, 108), (0.1, 100)])
    return hd.finish()


def head_fox():
    def paint(c, n, k):
        if c[2] < 0.02 and c[1] < 0.005 and n[2] < 0.5:
            return "fur_light"  # white cheeks, muzzle sides and throat
        return None

    hd = Head([(0.12, 0.03, 0.06, 0.065), (0.08, 0.035, 0.09, 0.095), (0.025, 0.025, 0.115, 0.095, 2.4, 0.9, 1.15),
               (-0.02, 0.012, 0.1, 0.082, 2.4, 0.9, 1.1), (-0.06, -0.008, 0.05, 0.05),
               (-0.13, -0.022, 0.027, 0.029), (-0.19, -0.03, 0.013, 0.015)],
              eye=(0.045, 0.03, 0.0, 15), paint=paint)
    hd.eye(ALMOND)
    hd.nose([(-0.01, 0.008), (0.01, 0.008), (0.012, 0.0), (0.0, -0.01), (-0.012, 0.0)], dy=0.002)
    hd.ear((0.058, 0.085, 0.035), 0.088, 0.035, (0.108, 0.25, 0.05), inner=0.62, inner_shift=-0.1)
    # Cheek ruffs: pale pointed tufts sweeping out and down.
    pyramid(hd.h, [(0.08, 0.015, -0.015), (0.08, 0.015, 0.04), (0.08, -0.05, 0.04),
                   (0.08, -0.05, -0.015)], (0.145, -0.055, 0.03), mat="fur_light",
            base_mat="fur_light")
    # A tear line down from each eye along the muzzle.
    hd.decal((0.045, 0.0, -0.04), (-0.3, 0.2, 0.93), [(-0.005, -0.03), (0.002, -0.03), (0.005, 0.0),
                                                      (-0.002, 0.0)], uhint=X, vhint=Y)
    hd.flow([(-0.12, 0), (0.11, 0)])
    hd.flow([(-0.02, 60), (0.05, 70), (0.11, 60)])
    return hd.finish()


def head_cat():
    def paint(c, n, k):
        if c[2] < -0.035 and c[1] < -0.012:
            return "fur_light"  # muzzle and chin
        return None

    hd = Head([(0.1, 0.02, 0.06, 0.065), (0.06, 0.03, 0.098, 0.1), (0.01, 0.025, 0.118, 0.1, 2.6, 0.95, 1.1),
               (-0.03, 0.01, 0.108, 0.088, 2.6, 0.95, 1.05), (-0.055, -0.015, 0.06, 0.05),
               (-0.08, -0.027, 0.046, 0.034)],
              eye=(0.045, 0.02, 0.0, 12), paint=paint)
    eye = [(-0.018, -0.006), (0.0, -0.013), (0.016, -0.008), (0.021, 0.008), (0.004, 0.012),
           (-0.016, 0.006)]
    hd.eye(eye, pupil=[(-0.0025, -0.011), (0.0025, -0.011), (0.0025, 0.01), (-0.0025, 0.01)])
    hd.nose([(-0.012, 0.006), (0.012, 0.006), (0.0, -0.01)], dy=0.006)
    hd.ear((0.07, 0.075, 0.025), 0.075, 0.04, (0.098, 0.18, 0.035), inner=0.6, inner_shift=-0.12)
    # Whiskers: three thin glowing spikes from each side of the muzzle.
    for dy, ang in ((0.004, 12), (-0.006, 0), (-0.016, -12)):
        a = math.radians(ang)
        p0 = (0.035, -0.02 + dy, -0.075)
        tube(hd.h, [p0, add(p0, (0.075 * math.cos(a), 0.075 * math.sin(a), 0.02))],
             [0.0022, 0.0], sides=3, mat="accent")
    hd.flow([(-0.05, 12), (-0.02, 15), (0.03, 12), (0.09, 12)])
    hd.flow([(-0.03, 100), (0.02, 105), (0.07, 100)])
    return hd.finish()


def head_bear():
    def paint(c, n, k):
        if c[2] < -0.07:
            return "fur_light"  # muzzle
        return None

    hd = Head([(0.12, 0.03, 0.08, 0.08), (0.08, 0.035, 0.12, 0.118), (0.02, 0.03, 0.135, 0.12),
               (-0.03, 0.01, 0.12, 0.1), (-0.07, -0.025, 0.07, 0.06, 2.6), (-0.13, -0.035, 0.058, 0.048, 2.6),
               (-0.15, -0.04, 0.042, 0.034, 2.6)],
              eye=(0.045, 0.03, 0.0, 10), paint=paint)
    hd.eye(scale2(L.ngon(0, 0, 0.011, 6), 1.0))
    hd.nose([(-0.022, 0.012), (0.022, 0.012), (0.026, 0.0), (0.008, -0.012), (-0.008, -0.012),
             (-0.026, 0.0)], dy=0.008)
    # Round ears: short hexagonal discs facing forward, accent inners.
    for _ in (0,):
        c = (0.095, 0.13, 0.04)
        r = 0.04
        ring0 = [add(c, (r * math.cos(math.radians(a)), r * math.sin(math.radians(a)), 0.012))
                 for a in range(-30, 211, 60)]
        ring1 = [add(p, (0, 0, -0.03)) for p in ring0]
        loft(hd.h, [ring0, ring1])
        face_plate(hd.h, ring1, NZ, k=0.55, shift=(0, 0.006, 0))
    hd.flow([(-0.07, 15), (-0.02, 25), (0.04, 25), (0.11, 20)])
    hd.flow([(-0.06, 110), (0.0, 115), (0.08, 105)])
    return hd.finish()


def head_rabbit():
    def paint(c, n, k):
        if c[2] < -0.035 and c[1] < 0.0:
            return "fur_light"  # muzzle and cheeks
        return None

    hd = Head([(0.1, 0.02, 0.065, 0.075), (0.05, 0.03, 0.095, 0.1), (0.0, 0.02, 0.1, 0.095),
               (-0.04, 0.0, 0.085, 0.078), (-0.07, -0.02, 0.055, 0.052), (-0.095, -0.032, 0.034, 0.034)],
              eye=(0.0, 0.025, -0.03, 60), paint=paint)
    hd.eye(scale2(L.ngon(0, 0, 0.014, 8, 22.5), 1.0))
    hd.nose([(-0.01, 0.006), (0.01, 0.006), (0.0, -0.007)], dy=0.008)
    # Teeth.
    hd.g.face([(-0.009, -0.05, -0.083), (0.009, -0.05, -0.083), (0.009, -0.07, -0.08),
               (-0.009, -0.07, -0.08)], "fur_light", want=NZ)
    # Long ears leaning slightly back and out, accent inner strips.
    leaf_ear(hd.h, (0.035, 0.07, 0.035), (0.25, 1.0, 0.18), NZ,
             [(0.0, 0.022, 0.012), (0.09, 0.03, 0.013), (0.19, 0.03, 0.012), (0.26, 0.018, 0.008)],
             0.3)
    hd.flow([(-0.08, 0), (0.09, 0)])
    hd.flow([(-0.05, 105), (0.0, 110), (0.07, 100)])
    return hd.finish()


def head_deer():
    def paint(c, n, k):
        if c[2] < -0.13:
            return "fur_light"  # muzzle
        if n[1] < -0.5 and c[2] < 0.05:
            return "fur_light"  # throat
        return None

    hd = Head([(0.09, 0.03, 0.06, 0.068), (0.05, 0.035, 0.082, 0.085), (0.0, 0.025, 0.088, 0.08),
               (-0.04, 0.0, 0.074, 0.07), (-0.09, -0.03, 0.05, 0.055), (-0.15, -0.055, 0.04, 0.045),
               (-0.18, -0.066, 0.03, 0.033)],
              eye=(0.0, 0.02, -0.025, 55), paint=paint)
    hd.eye(ALMOND)
    hd.nose([(-0.016, 0.01), (0.016, 0.01), (0.018, -0.004), (0.0, -0.012), (-0.018, -0.004)],
            dy=0.004)
    # Ears sticking out sideways.
    leaf_ear(hd.h, (0.055, 0.075, 0.045), (1.0, 0.5, 0.15), NZ,
             [(0.0, 0.016, 0.01), (0.05, 0.026, 0.01), (0.09, 0.02, 0.008)], 0.12)
    # Antlers: main beam up/out/back, three tines forwards-up; glowing tips.
    beam = [(0.035, 0.09, 0.03), (0.07, 0.17, 0.045), (0.11, 0.26, 0.035), (0.13, 0.33, 0.0),
            (0.135, 0.37, -0.03)]
    tube(hd.h, beam, [0.014, 0.012, 0.0095, 0.007, 0.0], sides=4,
         mat=["body", "body", "body", "accent"])
    for p, q, r in (((0.06, 0.15, 0.042), (0.075, 0.2, -0.02), 0.006),
                    ((0.095, 0.225, 0.04), (0.12, 0.28, -0.015), 0.005),
                    ((0.117, 0.285, 0.03), (0.17, 0.33, 0.02), 0.0045)):
        r *= 1.4
        mid = add(add(p, mul(sub(q, p), 0.72)), (0, 0.006, 0.006))
        tube(hd.h, [p, mid, q], [r, r * 0.6, 0.0], sides=4, mat=["body", "accent"])
    hd.flow([(-0.16, 0), (0.08, 0)])
    hd.flow([(-0.12, 105), (-0.05, 105), (0.04, 100)])
    return hd.finish()


def head_owl():
    hd = Head([(0.1, 0.0, 0.08, 0.09, 2.4, 1.0, 0.8), (0.06, 0.01, 0.112, 0.12, 2.6, 1.0, 0.85),
               (0.02, 0.012, 0.125, 0.128, 2.8, 1.0, 0.85), (-0.015, 0.008, 0.12, 0.12, 3.0, 1.0, 0.85),
               (-0.03, 0.005, 0.11, 0.11, 3.0, 1.0, 0.85)],
              eye=(0.048, 0.02, 0.0, 0), nose_mat="fur_light")
    c, hw, hh = hd.front
    hd.eye(L.ngon(0, 0, 0.034, 8, 22.5), pupil=L.ngon(0, 0, 0.012, 6), halo=0.01,
           seg=0.05)
    # V brows above the eyes (the classic owl frown), and a hooked beak.
    plate(hd.h, add(c, (0, 0.0, 0)), NZ, [(0.004, 0.05), (0.008, 0.042), (0.09, 0.072),
                                         (0.085, 0.082)], h=0.003, depth=0.004)
    beak = [(-0.014, 0.0, -0.026), (0.014, 0.0, -0.026), (0.0, -0.02, -0.026)]
    pyramid(hd.g, [add(p, (0, 0.0, 0)) for p in beak], (0.0, -0.032, -0.062), mat="accent",
            base_mat="accent")
    # Ear tufts.
    hd.ear((0.085, 0.1, 0.0), 0.05, 0.03, (0.125, 0.205, 0.015), inner=0.0001)
    hd.flow([(-0.028, 52), (0.0, 40), (0.05, 30), (0.095, 25)])
    hd.flow([(-0.028, 118), (0.03, 112), (0.09, 105)])
    return hd.finish()


def head_boar():
    def paint(c, n, k):
        if c[2] < -0.02 and n[1] < -0.45:
            return "fur_light"  # underjaw
        return None

    hd = Head([(0.12, 0.045, 0.075, 0.085), (0.07, 0.045, 0.11, 0.12), (0.01, 0.025, 0.118, 0.112),
               (-0.035, 0.005, 0.1, 0.095), (-0.08, -0.02, 0.072, 0.075), (-0.16, -0.032, 0.062, 0.062),
               (-0.2, -0.032, 0.056, 0.054, 3.0)],
              eye=(0.055, 0.035, 0.0, 25), paint=paint, nose_mat="fur_light")
    hd.eye([(-0.012, -0.004), (0.012, -0.006), (0.014, 0.004), (-0.01, 0.004)])
    # Nose disc nostrils.
    c, hw, hh = hd.front
    for s in (-1, 1):
        plate(hd.g, add(c, (s * 0.02, 0.0, 0)), NZ, L.ngon(0, 0, 0.011, 6), depth=0.004)
    # Tusks curling up from the mouth corners.
    tube(hd.h, [(0.05, -0.055, -0.12), (0.07, -0.045, -0.15), (0.08, -0.015, -0.17),
                (0.072, 0.012, -0.175)], [0.009, 0.007, 0.004, 0.0], sides=4, mat="accent")
    # Small ears flopping forward-out, and a bristle ridge along the top.
    hd.ear((0.07, 0.12, 0.06), 0.05, 0.03, (0.12, 0.17, 0.02), inner=0.6, tilt=0.008)
    for z, h in ((-0.02, 0.025), (0.02, 0.04), (0.06, 0.05), (0.1, 0.04), (0.135, 0.025)):
        y = 0.135 - (z - 0.04) ** 2 * 2.5
        pyramid(hd.g, [(-0.012, y - 0.02, z - 0.02), (0.012, y - 0.02, z - 0.02),
                       (0.012, y - 0.02, z + 0.02), (-0.012, y - 0.02, z + 0.02)],
                (0.0, y + h - 0.02, z + 0.02))
    hd.flow([(-0.12, 16), (-0.05, 22), (0.03, 18), (0.11, 16)])
    hd.flow([(-0.1, 105), (-0.03, 110), (0.06, 105)])
    return hd.finish()


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
    # Cuff.
    cuff = [palm_ring(z, x1, -h, h, 0.01) for z, x1, h in ((0.04, 0.013, 0.04), (0.098, 0.015, 0.043))]
    cuff = [[(x - 0.004 if x < 0 else x, y, z) for x, y, z in r] for r in cuff]
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

def main():
    L.reset_scene()
    L.standard_materials()
    L.material("fur_light", base=(0.16, 0.16, 0.17), rough=0.6)
    for h in ("hit_1", "hit_2", "hit_3"):
        L.material(h, base=(1, 1, 1), emit=(1, 1, 1), strength=3.0)
    L.material("eyes", base=(1, 1, 1), rough=0.4, emit=(1, 1, 1), strength=6.0)
    L.material("eye_glow", base=(1, 1, 1), rough=1.0, emit=(1, 1, 1), strength=1.5, alpha=0.5)
    L.material("flow", base=(1, 1, 1), rough=0.4, emit=(1, 1, 1), strength=3.0)
    pieces = [("head_" + n, globals()["head_" + n]()) for n in HEADS]
    pieces += [("torso", torso())]
    left = hand_left()
    pieces += [("hand_left", left), ("hand_right", mirrored(left))]
    pieces += [(name, build()) for name, build in SEGMENTS]
    for name, g in pieces:
        if not name.startswith("hand_"):
            bad = g.mirror_errors()
            if bad:
                raise RuntimeError("%s is not X mirror symmetric: %s" % (name, bad[:5]))
        L.make_object(name, g)
        mn = [min(p[i] for p in g.positions()) for i in range(3)]
        mx = [max(p[i] for p in g.positions()) for i in range(3)]
        print("avatar %-12s %4d tris, extent %s .. %s" % (
            name, g.tris(), [round(v, 3) for v in mn], [round(v, 3) for v in mx]))
    L.save_blend("assets-src/blend/avatars.blend")
    L.export_glb("assets/models/avatars.glb")


main()
