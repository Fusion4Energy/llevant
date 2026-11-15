"""Generate universes.i: fills, transforms, nesting and lattices.

Each slot k (box x in [30k, 30k+28], |y|, |z| <= 14) is a level-0 cell filled
with an asymmetric universe through a different kind of transform, or with a
lattice. Universe 1: an off-centre sphere and a half-space, so that any error
in the direction of a transform moves material visibly.

    python3 gen_universes.py > universes.i
"""

cells = []
surfs = []
data = []


def slot_box(k):
    return f"rpp {30 * k} {30 * k + 28} -14 14 -14 14"


def center(k):
    return 30 * k + 14.0


# Universe 1: sphere (mat 1) at (2, 1, 0) r=3, half-space x < -1 outside it
# (mat 2), the rest void.
surfs += ["101 s 2 1 0 3", "102 px -1"]
cells += [
    "c --- universe 1",
    "201 1 -1.0 -101 u=1 imp:n=1",
    "202 2 -2.0 101 -102 u=1 imp:n=1",
    "203 0 101 102 u=1 imp:n=1",
]

# Universe 5: lattice content, a small sphere off-centre.
surfs += ["151 s 0.5 0.3 0 1.2"]
cells += [
    "c --- universe 5",
    "251 1 -1.0 -151 u=5 imp:n=1",
    "252 0 151 u=5 imp:n=1",
]

slots = []


def add_slot(comment, fill, extra_data=()):
    slots.append((comment, fill))
    data.extend(extra_data)


add_slot("fill with TR reference (cosines)", "fill=1 (1)",
         [f"tr1 {center(0)} 1 -1  0.866025403784 0.5 0  -0.5 0.866025403784 0  0 0 1"])
add_slot("*fill inline degrees", f"*fill=1 ({center(1)} 0 0  30 60 90  120 30 90  90 90 0)")
add_slot("fill inline displacement", f"fill=1 ({center(2)} 1 2)")
# Universe 2: a cell filled with universe 1 through *TR3, plus material.
surfs += ["111 so 9"]
cells += [
    "c --- universe 2: nests universe 1",
    "211 0 -111 fill=1 (3) u=2 imp:n=1",
    "212 2 -2.0 111 u=2 imp:n=1",
]
add_slot("nested: fill=2 with no transform, universe 2 fills 1", "fill=2",
         ["*tr3 0 2 0  90 0 90  180 90 90  90 90 0"])
# Universe 3 is a position for universe 2 itself.
add_slot("fill=2 with TR m=-1", "fill=2 (4)",
         [f"tr4 {-center(4)} 0 -1  1 0 0  0 0 1  0 -1 0  -1"])
# Lattice with an RPP element, filled by universe 5.
surfs += ["161 rpp -2 2 -3 3 -1.5 1.5"]
cells += [
    "c --- universe 4: LAT=1 with an RPP element",
    "261 0 -161 lat=1 fill=5 u=4 imp:n=1",
]
add_slot("lattice (RPP element), translated and rotated", f"*fill=4 ({center(5)} 0.4 -0.7  20 70 90  110 20 90  90 90 0)")
# 2-D lattice from planes (infinite along z).
surfs += ["171 px 1.5", "172 px -1.5", "173 py 2", "174 py -2"]
cells += [
    "c --- universe 6: 2-D LAT=1 from four planes",
    "271 0 -171 172 -173 174 lat=1 fill=5 u=6 imp:n=1",
]
add_slot("2-D lattice from planes", f"fill=6 ({center(6)} 0.25 0)")
# Skewed (non-orthogonal) hexahedral lattice.
surfs += ["181 p 1 -0.5 0 1.5", "182 p 1 -0.5 0 -1.5", "183 py 2", "184 py -2", "185 pz 1.7", "186 pz -1.7"]
cells += [
    "c --- universe 7: skewed hexahedral lattice",
    "281 0 -181 182 -183 184 -185 186 lat=1 fill=5 u=7 imp:n=1",
]
add_slot("skewed lattice", f"fill=7 ({center(7)} 0 0.3)")
# Lattice inside a universe that is itself translated.
surfs += ["191 so 12"]
cells += [
    "c --- universe 8: a sphere filled with the RPP lattice",
    "291 0 -191 fill=4 (0.3 0.2 0.1) u=8 imp:n=1",
    "292 2 -2.0 191 u=8 imp:n=1",
]
add_slot("universe containing a lattice", f"fill=8 ({center(8)} 0 0)")


# --- TRCL. Cells defined around the origin, moved into their slot; the rest
# of each slot is the complement `#n` of the moved cell.
surfs += ["301 c/z 1 0 3", "302 pz -4", "303 pz 5", "304 so 6"]
extra_slots = []


def trcl_slot(comment, body, rest_material=2):
    """A slot whose content is `body` (cell text without number) plus its complement."""
    extra_slots.append((comment, body, rest_material))


trcl_slot("TRCL inline displacement", "1 -1.0 -301 302 -303 trcl=({cx} 1 0)")
trcl_slot("*TRCL inline rotation", "1 -1.0 -301 302 -303 *trcl=({cx} 0 0  30 60 90  120 30 90  90 90 0)")
trcl_slot("TRCL by TR number", "1 -1.0 -301 302 -303 trcl={tr}")
trcl_slot("LIKE BUT TRCL (overrides the base cell's TRCL)", "like 400 but trcl=({cx} 0.5 -1)")
trcl_slot("TRCL on a filled cell", "0 -304 trcl=({cx} 0 0  0.8 0.6 0  -0.6 0.8 0  0 0 1) fill=1")
trcl_slot("TRCL and fill transform", "0 -304 *trcl=({cx} 0 1  0 90 90  90 0 90  90 90 0) fill=1 (2 1 0)")

# --- fill arrays and hexagonal lattices
surfs += [
    "311 rpp -2 2 -3 3 -15 15",
    # regular hexagon, flat-to-flat 4, sides at 0, 60, 120 degrees
    "321 p 1 0 0 2", "322 p 1 0 0 -2",
    "323 p 0.5 0.866025403784439 0 2", "324 p 0.5 0.866025403784439 0 -2",
    "325 p -0.5 0.866025403784439 0 2", "326 p -0.5 0.866025403784439 0 -2",
    "327 pz 3", "328 pz -3",
    "331 rhp 0 0 -2  0 0 4  0 1.8 0",
    "341 so 11",
]
cells += [
    "c --- universe 9: LAT=1 with a fill array (own universe, transforms)",
    "301 3 -3.0 -311 lat=1 u=9 imp:n=1 fill=-3:3 -2:2 0:0",
    "     5 1 9 5 (0.5 0 0) 1 5 9",
    "     1 5 5 (4) 9 1 5 1",
    "     9 9 1 5 1 5 (0 0.2 0) 9",
    "     5 1 1 9 5 1 5",
    "     1 5 9 1 5 9 1",
    "c --- universe 10: 2-D hexagonal lattice from planes, fill array",
    "302 0 -321 322 -323 324 -325 326 lat=2 u=10 imp:n=1 fill=-8:8 -4:4 0:0",
] + [
    # 17 x 9 entries: universes 5 and 1, and the lattice's own universe 10
    "     " + " ".join(str([5, 1, 10, 5, 5, 1][(3 * row + col) % 6]) for col in range(17))
    for row in range(9)
] + [
    "c --- universe 11: 3-D hexagonal lattice from an RHP, uniform fill",
    "303 0 -331 lat=2 fill=5 u=11 imp:n=1",
    "c --- universe 12: a LAT=1 cell moved by TRCL",
    "304 0 -311 lat=1 fill=5 u=12 imp:n=1 trcl=(0.3 0.1 0  0.8 0.6 0  -0.6 0.8 0  0 0 1)",
]
for comment, fill in [
    ("fill array lattice", "fill=9 (CX 0 0)"),
    ("2-D hexagonal lattice", "fill=10 (CX 0.4 0)"),
    ("3-D hexagonal lattice (RHP element)", "*fill=11 (CX 0 0  20 70 90  110 20 90  90 90 0)"),
    ("lattice cell with TRCL", "fill=12 (CX 0 0)"),
]:
    extra_slots.append((comment, fill, None))

lines = ["universe zoo for llevant / MCNP comparison"]
slot_ids = []
for i, (comment, fill) in enumerate(slots):
    sid = 5000 + i
    slot_ids.append(sid)
    surfs.append(f"{sid} {slot_box(i)}")
    lines.append(f"c --- slot {i}: {comment}")
    lines.append(f"{10 + i} 0 -{sid} {fill} imp:n=1")
for j, (comment, body, rest) in enumerate(extra_slots):
    i = len(slots) + j
    sid = 5000 + i
    slot_ids.append(sid)
    surfs.append(f"{sid} {slot_box(i)}")
    lines.append(f"c --- slot {i}: {comment}")
    cx = center(i)
    if rest is None:
        # A filled slot: `body` is the fill.
        lines.append(f"{10 + i} 0 -{sid} {body.replace('CX', str(cx))} imp:n=1")
        continue
    tr = 100 + i
    data.append(f"tr{tr} {cx} -1 0.5  0 1 0  -1 0 0  0 0 1")
    c = 400 + 2 * j
    lines.append(f"{c} {body.format(cx=cx, tr=tr)} imp:n=1")
    lines.append(f"{c + 1} {rest} -2.0 -{sid} #{c} imp:n=1")
lines += cells
xmax = 30 * (len(slots) + len(extra_slots)) + 10
rest = "998 0 -5999"
for s in slot_ids:
    rest += f" {s}"
lines.append(rest + " imp:n=1")
lines.append("999 0 5999 imp:n=0")
lines.append("")
lines += surfs
lines.append(f"5999 rpp -10 {xmax} -20 20 -20 20")
lines.append("")
lines += ["m1 1001.32c 1", "m2 26056.32c 1", "m3 8016.32c 1"] + data
print("\n".join(lines))
