"""Synthetic models of different shapes, for benchmarking the search
structures (not for MCNP comparisons).

    python3 tools/synthetic/gen_models.py untracked/synthetic

tokamak  toroidal sectors × shells × layers (curved cells, overlapping boxes),
         half of them filled with a universe of thin slabs: like the
         reference model's level 0, but with exact cylinders and planes;
         10° sectors and 50 cm shells
tokamak-coarse   the same with 40° sectors and 10 cm shells, whose boxes
         overlap far more
core     PWR-like core: LAT=1 assemblies of LAT=1 pin cells
sparse   1000 small spheres in a 5 km world, plus a rest-of-world cell
         written as the complement of all of them
bricks   40 × 40 × 20 axis-aligned boxes: the BVH case
tori     nested torus shells cut into sectors (tori are undecided over boxes)
"""

import math
import os
import random
import sys


def wrap(line, width=78):
    """Split a long card into MCNP continuation lines (5 leading blanks)."""
    words = line.split(" ")
    out, cur = [], ""
    for w in words:
        if cur and len(cur) + 1 + len(w) > width:
            out.append(cur)
            cur = "     " + w
        else:
            cur = w if not cur else cur + " " + w
    out.append(cur)
    return "\n".join(out)


def write(path, title, cells, surfs, extra=""):
    with open(path, "w") as f:
        f.write(title + "\n")
        f.write("\n".join(wrap(c) for c in cells) + "\n\n")
        f.write("\n".join(wrap(s) for s in surfs) + "\n\n")
        f.write("m1 1001.31c 1\nm2 26056.31c 1\n" + extra)


def sector_planes(n, first):
    """Planes through the z axis at angles 2πk/n; +side is counter-clockwise."""
    surfs = []
    for k in range(n):
        a = 2 * math.pi * k / n
        surfs.append(f"{first + k} p {-math.sin(a):.12f} {math.cos(a):.12f} 0 0")
    return surfs


def tokamak(path, nphi=36, dr=50):
    radii, zs = [200 + dr * i for i in range(700 // dr + 1)], [-600 + 100 * i for i in range(13)]
    surfs = sector_planes(nphi, 1000)
    surfs += [f"{2000 + i} cz {r}" for i, r in enumerate(radii)]
    surfs += [f"{3000 + i} pz {z}" for i, z in enumerate(zs)]
    surfs += [f"{4000 + i} pz {-600 + 5 * i}" for i in range(241)]
    surfs.append("9000 rcc 0 0 -800 0 0 1600 1100")
    cells, cid = [], 1
    rng = random.Random(1)
    for k in range(nphi):
        for i in range(len(radii) - 1):
            for j in range(len(zs) - 1):
                geo = f"{1000 + k} -{1000 + (k + 1) % nphi} {2000 + i} -{2000 + i + 1} {3000 + j} -{3000 + j + 1}"
                fill = " fill=100" if rng.random() < 0.5 else ""
                mat = " 0" if fill else " 1 -1.0"
                cells.append(f"{cid}{mat} {geo}{fill} imp:n=1")
                cid += 1
    # Central column and the rest inside the outer cylinder.
    cells.append(f"{cid} 2 -7.9 -2000 3000 -{3000 + len(zs) - 1} imp:n=1")
    cid += 1
    cells.append(f"{cid} 0 -9000 (-3000 : {3000 + len(zs) - 1} : {2000 + len(radii) - 1}) imp:n=1")
    cid += 1
    cells.append(f"{cid} 0 9000 imp:n=0")
    # Filler: thin slabs in the global frame, the last one closing the top.
    for i in range(240):
        cells.append(f"{10000 + i} 1 -1.0 {4000 + i} -{4001 + i} u=100 imp:n=1")
    cells.append("10240 0 -4000 u=100 imp:n=1")
    cells.append("10241 0 4240 u=100 imp:n=1")
    write(path, f"synthetic tokamak, {nphi} sectors, {dr} cm shells", cells, surfs)


def core(path):
    pitch, npin, nasm = 1.26, 17, 15
    apitch = pitch * npin
    half = apitch * nasm / 2
    surfs = [
        "1 cz 0.41", "2 cz 0.475", "3 cz 0.56", "4 cz 0.61",
        f"10 px {-pitch / 2}", f"11 px {pitch / 2}", f"12 py {-pitch / 2}", f"13 py {pitch / 2}",
        f"20 px {-apitch / 2}", f"21 px {apitch / 2}", f"22 py {-apitch / 2}", f"23 py {apitch / 2}",
        f"30 px {-half}", f"31 px {half}", f"32 py {-half}", f"33 py {half}",
        "40 pz -200", "41 pz 200", f"50 cz {half * 1.45}", f"51 cz {half * 1.45 + 20}",
        "52 pz -250", "53 pz 250",
    ]
    cells = [
        "1 1 -10.4 -1 u=1 imp:n=1", "2 0 1 -2 u=1 imp:n=1", "3 2 -6.5 2 u=1 imp:n=1",
        "4 1 -1.0 -3 u=2 imp:n=1", "5 2 -6.5 3 -4 u=2 imp:n=1", "6 1 -1.0 4 u=2 imp:n=1",
    ]
    guides = {(2, 5), (2, 8), (2, 11), (3, 3), (3, 13), (5, 2), (5, 5), (5, 8), (5, 11), (5, 14),
              (8, 2), (8, 5), (8, 8), (8, 11), (8, 14), (11, 2), (11, 5), (11, 8), (11, 11), (11, 14),
              (13, 3), (13, 13), (14, 5), (14, 8), (14, 11)}
    ent = ["2" if (i, j) in guides else "1" for j in range(npin) for i in range(npin)]
    lo = -(npin // 2)
    cells.append(f"7 1 -1.0 10 -11 12 -13 lat=1 u=3 imp:n=1 "
                 f"fill={lo}:{-lo} {lo}:{-lo} 0:0 " + " ".join(ent))
    cells.append("8 0 20 -21 22 -23 fill=3 u=4 imp:n=1")
    cells.append("9 1 -1.0 -20 : 21 : -22 : 23 u=4 imp:n=1")
    alo = -(nasm // 2)
    aent = []
    for j in range(nasm):
        for i in range(nasm):
            x, y = (i + alo) * apitch, (j + alo) * apitch
            aent.append("4" if math.hypot(x, y) < half * 1.05 else "5")
    cells.append(f"10 1 -1.0 20 -21 22 -23 lat=1 u=6 imp:n=1 "
                 f"fill={alo}:{-alo} {alo}:{-alo} 0:0 " + " ".join(aent))
    cells.append("11 1 -1.0 -999 u=5 imp:n=1")
    cells.append("12 1 -1.0 999 u=5 imp:n=1")
    cells.append("20 0 30 -31 32 -33 40 -41 fill=6 imp:n=1")
    cells.append("21 1 -1.0 -50 52 -53 #20 imp:n=1")
    cells.append("22 2 -7.9 50 -51 52 -53 imp:n=1")
    cells.append("23 0 51 : -52 : 53 imp:n=0")
    surfs.append("999 so 1e9")
    write(path, "synthetic lattice core", cells, surfs)


def sparse(path):
    rng = random.Random(2)
    surfs, cells = [], []
    n = 0
    # Jittered 10 × 10 × 10 grid of spheres in a 2 m cube: no overlaps.
    for i in range(10):
        for j in range(10):
            for k in range(10):
                c = [-100 + 20 * a + 10 + rng.uniform(-4, 4) for a in (i, j, k)]
                r = rng.uniform(1, 5)
                n += 1
                surfs.append(f"{n} s {c[0]:.4f} {c[1]:.4f} {c[2]:.4f} {r:.4f}")
                cells.append(f"{n} 1 -1.0 -{n} imp:n=1")
    surfs.append("5000 so 500000")
    cells.append("5000 0 -5000 " + " ".join(str(i) for i in range(1, n + 1)) + " imp:n=1")
    cells.append("5001 0 5000 imp:n=0")
    write(path, "synthetic sparse world", cells, surfs)


def bricks(path):
    nx, ny, nz = 40, 40, 20
    surfs = [f"{100 + i} px {-200 + 10 * i}" for i in range(nx + 1)]
    surfs += [f"{200 + i} py {-200 + 10 * i}" for i in range(ny + 1)]
    surfs += [f"{300 + i} pz {-100 + 10 * i}" for i in range(nz + 1)]
    cells, cid = [], 1
    for i in range(nx):
        for j in range(ny):
            for k in range(nz):
                cells.append(f"{cid} 1 -1.0 {100 + i} -{101 + i} {200 + j} -{201 + j} "
                             f"{300 + k} -{301 + k} imp:n=1")
                cid += 1
    cells.append(f"{cid} 0 -100 : {100 + nx} : -200 : {200 + ny} : -300 : {300 + nz} imp:n=0")
    write(path, "synthetic bricks", cells, surfs)


def tori(path):
    nphi, nshell = 36, 12
    surfs = sector_planes(nphi, 1000)
    surfs += [f"{2000 + i} tz 0 0 0 600 {20 + 15 * i} {20 + 15 * i}" for i in range(nshell + 1)]
    surfs.append("9000 so 2000")
    cells, cid = [], 1
    for k in range(nphi):
        cells.append(f"{cid} 1 -1.0 -2000 {1000 + k} -{1000 + (k + 1) % nphi} imp:n=1")
        cid += 1
        for i in range(nshell):
            cells.append(f"{cid} 1 -1.0 {2000 + i} -{2001 + i} {1000 + k} -{1000 + (k + 1) % nphi} imp:n=1")
            cid += 1
    cells.append(f"{cid} 0 -9000 {2000 + nshell} imp:n=1")
    cells.append(f"{cid + 1} 0 9000 imp:n=0")
    write(path, "synthetic torus shells", cells, surfs)


if __name__ == "__main__":
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    coarse = lambda p: tokamak(p, nphi=9, dr=10)
    for name, f in [("tokamak", tokamak), ("tokamak-coarse", coarse), ("core", core), ("sparse", sparse),
                    ("bricks", bricks), ("tori", tori)]:
        f(os.path.join(out, name + ".i"))
        print("wrote", os.path.join(out, name + ".i"))
