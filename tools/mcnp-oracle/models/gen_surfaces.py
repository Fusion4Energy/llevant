"""Generate surfaces.i: one slot box per surface form, split by that surface.

Slot k is the box x in [20k, 20k+18], |y|, |z| <= 9. Cell 10+2k (material 1)
is the part of the slot selected by the test geometry; cell 11+2k
(material 2) is the rest of the slot, written as a complement (alternately
`#(...)` and `#n`). Everything else inside the world box is void cell 998.

    python3 gen_surfaces.py > surfaces.i
"""

import math

slots = []  # (comment, [surface cards], geometry using {s} {s2}..., [data cards])


def add(comment, surfs, geom, data=()):
    slots.append((comment, surfs, geom, list(data)))


def gq_axis_cylinder(c, u, r):
    """GQ coefficients of |d|^2 - (d.u)^2 - r^2 with d = p - c."""
    n = math.sqrt(sum(x * x for x in u))
    u = [x / n for x in u]
    a = [[(1.0 if i == j else 0.0) - u[i] * u[j] for j in range(3)] for i in range(3)]
    return gq_from(a, [0.0, 0.0, 0.0], -r * r, c)


def gq_from(a, b, k, c):
    """GQ coefficients of (p-c)^T a (p-c) + b.(p-c) + k."""
    A, B, C = a[0][0], a[1][1], a[2][2]
    D, E, F = 2 * a[0][1], 2 * a[1][2], 2 * a[0][2]
    # linear: -2 a c + b ; constant: c^T a c - b.c + k
    ac = [sum(a[i][j] * c[j] for j in range(3)) for i in range(3)]
    G, H, J = (-2 * ac[i] + b[i] for i in range(3))
    K = sum(c[i] * ac[i] for i in range(3)) - sum(b[i] * c[i] for i in range(3)) + k
    return [A, B, C, D, E, F, G, H, J, K]


def fmt(v):
    return " ".join(f"{x:.12g}" for x in v)


def rot_cosines(axis, deg):
    """Rows: auxiliary axes in main coordinates, for a rotation by `deg` about `axis`."""
    t = math.radians(deg)
    c, s = math.cos(t), math.sin(t)
    if axis == "x":
        m = [[1, 0, 0], [0, c, s], [0, -s, c]]
    elif axis == "y":
        m = [[c, 0, -s], [0, 1, 0], [s, 0, c]]
    else:
        m = [[c, s, 0], [-s, c, 0], [0, 0, 1]]
    return [x for row in m for x in row]


def center(k):
    return 20 * k + 9.0


# --- planes
cx = center(len(slots))
add("P general", [f"p 1 2 3 {cx + 2}"], "-{s}")
cx = center(len(slots))
add("P by 3 points", [f"p {cx} 1 0  {cx + 1} 3 1  {cx - 2} 0 4"], "-{s}")
cx = center(len(slots))
add("P by 3 points through origin", [f"p 0 0 0  {cx} 0 0  {cx} 1 1"], "-{s}")
cx = center(len(slots))
add("PX", [f"px {cx + 2}"], "-{s}")
cx = center(len(slots))
add("PY", ["py 3"], "{s}")
cx = center(len(slots))
add("PZ", ["pz -2"], "-{s}")
# --- spheres
cx = center(len(slots))
add("SO", [f"so {cx + 3}"], "-{s}")
cx = center(len(slots))
add("S", [f"s {cx + 2} 1 -1 6"], "-{s}")
cx = center(len(slots))
add("SX", [f"sx {cx - 1} 7"], "-{s}")
cx = center(len(slots))
add("SY", [f"sy 3 {math.hypot(cx, 3) + 1}"], "-{s}")
cx = center(len(slots))
add("SZ", [f"sz -2 {math.hypot(cx, 2) - 2}"], "-{s}")
# --- cylinders
cx = center(len(slots))
add("C/X", ["c/x 1 -1 5"], "-{s}")
cx = center(len(slots))
add("C/Y", [f"c/y {cx + 1} 2 6"], "-{s}")
cx = center(len(slots))
add("C/Z", [f"c/z {cx - 2} 1 5"], "{s}")
cx = center(len(slots))
add("CX", ["cx 4"], "-{s}")
cx = center(len(slots))
add("CY", [f"cy {cx + 2}"], "-{s}")
cx = center(len(slots))
add("CZ", [f"cz {cx - 3}"], "-{s}")
# --- cones
cx = center(len(slots))
add("K/X two sheets", [f"k/x {cx - 3} 0 0 0.5"], "-{s}")
cx = center(len(slots))
add("K/Y sheet +1", [f"k/y {cx} -4 0 1 1"], "-{s}")
cx = center(len(slots))
add("K/Z sheet -1", [f"k/z {cx} 0 3 0.7 -1"], "-{s}")
cx = center(len(slots))
add("KX sheet -1", [f"kx {cx} 0.3 -1"], "-{s}")
cx = center(len(slots))
add("KY sheet +1", [f"ky {-cx} 1 1"], "-{s}")
cx = center(len(slots))
add("KZ sheet -1", [f"kz {cx} 1 -1"], "-{s}")
cx = center(len(slots))
add("KZ two sheets", [f"kz {cx / 2} 4"], "{s}")
# --- general quadrics
cx = center(len(slots))
add("SQ ellipsoid", [f"sq 1 4 9 0 0 0 -49 {cx} 0 0"], "-{s}")
cx = center(len(slots))
add("SQ with linear terms", [f"sq 1 1 0 2 0 1 -30 {cx} 1 0"], "-{s}")
cx = center(len(slots))
add("GQ rotated cylinder", ["gq " + fmt(gq_axis_cylinder([cx, 1, -1], [1, 1, 1], 4))], "-{s}")
cx = center(len(slots))
add("GQ hyperboloid", ["gq " + fmt(gq_from([[1, 0.3, 0], [0.3, -0.5, 0.2], [0, 0.2, 1]], [0.5, -1, 0], -9, [cx, 0, 0]))], "-{s}")
# --- tori
cx = center(len(slots))
add("TX", [f"tx {cx} 0 0 6 2 1.5"], "-{s}")
cx = center(len(slots))
add("TY elliptical", [f"ty {cx} 0 0 5 1 2"], "-{s}")
cx = center(len(slots))
add("TZ outside", [f"tz {cx} 1 0 4 2 2"], "{s}")
cx = center(len(slots))
add("TZ degenerate A=0", [f"tz {cx} 0 0 0 6 4"], "-{s}")
# --- axisymmetric by points
cx = center(len(slots))
add("X plane", [f"x {cx + 1} 3 {cx + 1} 7"], "-{s}")
cx = center(len(slots))
add("X cylinder", [f"x {cx - 5} 4 {cx + 5} 4"], "-{s}")
cx = center(len(slots))
add("X cone opening +x", [f"x {cx - 6} 2 {cx + 6} 7"], "-{s}")
cx = center(len(slots))
add("X cone opening -x", [f"x {cx - 6} 7 {cx + 6} 2"], "-{s}")
# --- macrobodies
cx = center(len(slots))
add("RPP", [f"rpp {cx - 5} {cx + 3} -4 6 -2 7"], "-{s}")
cx = center(len(slots))
add("RPP facets", [f"rpp {cx - 5} {cx + 3} -4 6 -2 7"], "-{s}.1 {s}.4")
s2 = math.sqrt(0.5)
cx = center(len(slots))
box = f"box {cx - 4} -1 -3  {6 * s2} {6 * s2} 0  {-5 * s2} {5 * s2} 0  0 0 7"
add("BOX rotated", [box], "-{s}")
cx = center(len(slots))
add("BOX facets", [box.replace(f"{cx - 4}", f"{center(len(slots)) - 4}")], "{s}.2 -{s}.5")
cx = center(len(slots))
add("SPH", [f"sph {cx} 1 1 6"], "-{s}")
cx = center(len(slots))
add("RCC tilted", [f"rcc {cx - 4} -2 -3  8 4 6  3"], "-{s}")
cx = center(len(slots))
add("RCC facets", [f"rcc {cx - 4} -2 -3  8 4 6  3"], "-{s}.1 {s}.3")
cx = center(len(slots))
add("TRC", [f"trc {cx - 5} 0 0  10 0 0  6 2"], "-{s}")
cx = center(len(slots))
add("TRC lateral facet", [f"trc {cx - 5} 1 0  9 2 1  2 6"], "-{s}.1")
# --- transforms
cx = center(len(slots))
add("TR displacement only", [f"{{tr}} c/z 0 0 5"], "-{s}", [f"tr{{tr}} {cx} 1 0"])
cx = center(len(slots))
add("TR cosines", [f"{{tr}} c/z 1 0 4"], "-{s}", [f"tr{{tr}} {cx} 0 0 " + fmt(rot_cosines("y", 30))])
cx = center(len(slots))
# MCNP rejects one-sheet cones and tori under skew transforms (axis not
# parallel to x, y or z), so these rotations only permute axes.
add("*TR degrees", [f"{{tr}} kz 2 0.5 1"], "-{s}", [f"*tr{{tr}} {cx} 0 0  90 0 90  180 90 90  90 90 0"])
cx = center(len(slots))
add("*TR skew, two-sheet cone", [f"{{tr}} kz 2 0.5"], "-{s}", [f"*tr{{tr}} {cx} 0 0  90 60 150  90 30 60  0 90 90"])
cx = center(len(slots))
add("TR m=-1", [f"{{tr}} s 1 0 0 5"], "-{s}", [f"tr{{tr}} {-cx} 0 0 1 0 0 0 1 0 0 0 1 -1"])
cx = center(len(slots))
add("*TR torus", [f"{{tr}} tx 0 0 0 5 1 1.5"], "-{s}", [f"*tr{{tr}} {cx} 0 1  90 0 90  90 90 0  0 90 90"])
cx = center(len(slots))
add("TR GQ", [f"{{tr}} gq " + fmt(gq_axis_cylinder([0, 1, 0], [1, 2, 0], 3))], "-{s}", [f"tr{{tr}} {cx} 0 1 " + fmt(rot_cosines("x", 40))])
cx = center(len(slots))
add("TR RCC", [f"{{tr}} rcc -4 0 0 8 0 0 4"], "-{s}", [f"tr{{tr}} {cx} 0 0 " + fmt(rot_cosines("z", 25))])
# --- more macrobodies: whole body, then every facet on its own
def cross(a, b):
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def scaled(v, length):
    n = math.sqrt(sum(x * x for x in v))
    return [x * length / n for x in v]


def rec_a2():
    return scaled(cross([1, 0.5, 11], [5.5, 0, -0.5]), 2.5)


def wed_h():
    return scaled(cross([8, 0, 0], [0, 7, 0.5]), 11)


def bodies(cx):
    return {
        "RHP regular": f"rhp {cx - 1} 0.5 -6  0.5 1 11  4 -2 0",
        "RHP with s t": f"rhp {cx} 0 -5  0 0 10  5 0 0  2 4 0  -2.5 4 0",
        "HEX": f"hex {cx} 0.3 -5  0 1 10  3 2 -0.2",
        "REC vector": f"rec {cx - 1} 0 -6  1 0.5 11  5.5 0 -0.5  " + fmt(rec_a2()),
        "REC scalar": f"rec {cx} -6 0  0.3 11 1  4 0 -1.2  2.5",
        "WED": f"wed {cx - 5} -4 -6  8 0 0  0 7 0.5  " + fmt(wed_h()),
        "ARB": (f"arb {cx - 5} -5 -5  {cx + 5} -5 -5  {cx + 5} 5 -5  {cx - 5} 5 -5"
                f"  {cx - 3} -3 5  {cx + 3} -3 5  {cx + 3} 3 5  {cx - 3} 3 5"
                "  1234 5678 1265 2376 3487 1485"),
    }


facets = {"RHP regular": 8, "RHP with s t": 8, "HEX": 8, "REC vector": 3, "REC scalar": 3, "WED": 5, "ARB": 6}
for name, nf in facets.items():
    cx = center(len(slots))
    add(name, [bodies(cx)[name]], "-{s}")
    for f in range(1, nf + 1):
        cx = center(len(slots))
        add(f"{name} facet {f}", [bodies(cx)[name]], f"-{{s}}.{f}")
cx = center(len(slots))
add("ELL by foci", [f"ell {cx - 3} -1 0  {cx + 3} 1 1  6"], "-{s}")
cx = center(len(slots))
add("ELL by centre", [f"ell {cx} 0 0  3 5 2  -3"], "-{s}")
cx = center(len(slots))
add("ARB with a missing face (wedge-like)", [
    f"arb {cx - 5} -5 -5  {cx + 5} -5 -5  {cx - 5} 5 -5  0 0 0"
    f"  {cx - 5} -5 6  {cx + 5} -5 6  {cx - 5} 5 6  0 0 0"
    "  1230 5670 1265 2376 1375 0"], "-{s}")

# --- boolean structure
cx = center(len(slots))
add("union and nesting", [f"px {cx}", f"so {cx + 4}", f"c/x 0 0 5"], "(-{s} {s2}) : (-{s3} -{s2})")

# ---------------------------------------------------------------------------
cells, surfs, data = [], [], []
slot_ids = []
for i, (comment, scards, geom, dcards) in enumerate(slots):
    cx = center(i)
    sid = 5000 + i
    slot_ids.append(sid)
    surfs.append(f"c --- slot {i}: {comment}")
    surfs.append(f"{sid} rpp {20 * i} {20 * i + 18} -9 9 -9 9")
    ids = {}
    for j, sc in enumerate(scards):
        s = 100 + 10 * i + j
        ids["s" if j == 0 else f"s{j + 1}"] = s
        surfs.append(f"{s} " + sc.format(tr=i + 1))
    for dc in dcards:
        data.append(dc.format(tr=i + 1))
    g = geom.format(**ids)
    t, o = 10 + 2 * i, 11 + 2 * i
    cells.append(f"c --- slot {i}: {comment}")
    cells.append(f"{t} 1 -1.0 -{sid} ({g}) imp:n=1")
    if i % 2 == 0:
        cells.append(f"{o} 2 -2.0 -{sid} #({g}) imp:n=1")
    else:
        cells.append(f"{o} 2 -2.0 -{sid} #{t} imp:n=1")

world = 5999
xmax = 20 * len(slots) + 10
surfs.append(f"{world} rpp -10 {xmax} -20 20 -20 20")
rest = " ".join(str(s) for s in slot_ids)
lines = ["surface zoo for llevant / MCNP comparison"]
lines += cells
# the rest of the world: outside every slot
rest_lines = []
cur = f"998 0 -{world}"
for s in slot_ids:
    if len(cur) > 70:
        rest_lines.append(cur)
        cur = "     "
    cur += f" {s}"
rest_lines.append(cur + " imp:n=1")
lines += rest_lines
lines.append(f"999 0 {world} imp:n=0")
lines.append("")
lines += surfs
lines.append("")
lines += ["m1 1001.32c 1", "m2 26056.32c 1"] + data
print("\n".join(lines))
