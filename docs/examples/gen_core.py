"""The showcase model of the README's screenshots: a small PWR-like core.

    python docs/examples/gen_core.py            # writes docs/examples/core.i
    python docs/examples/gen_core.py --power    # and core-power.csv (needs `pip install llevant`)

Not a real design. Fuel pins and guide tubes (universes 1 and 2) in a 17 x 17
lattice (universe 3) make an assembly (universe 4); assemblies and water
(universe 5) fill a 17 x 17 core lattice (universe 6), cut by the core barrel.
Around it: support plates, the barrel, the downcomer, a steel vessel, a
concrete shield on a concrete floor, and air.

`core-power.csv` is a mock pin power per pin (higher in the middle of the core,
lower at its edge), given to every cell instance of the pin's lattice element:
numbers to draw with `--field`, not physics.
"""

import argparse
import math
from pathlib import Path

HERE = Path(__file__).resolve().parent
PITCH, NPIN, NASM = 1.26, 17, 17
APITCH = PITCH * NPIN  # 21.42 cm
GUIDES = {(2, 5), (2, 8), (2, 11), (3, 3), (3, 13), (5, 2), (5, 5), (5, 8), (5, 11), (5, 14),
          (8, 2), (8, 5), (8, 8), (8, 11), (8, 14), (11, 2), (11, 5), (11, 8), (11, 11), (11, 14),
          (13, 3), (13, 13), (14, 5), (14, 8), (14, 11)}
CORE_R = 175.0  # barrel inner radius
FUEL_R = 160.0  # assemblies whose centre is within this radius hold fuel


def wrap(head, words, width=78):
    lines, line = [], head
    for w in words:
        if len(line) + 1 + len(w) > width:
            lines.append(line)
            line = "     " + w
        else:
            line += " " + w
    lines.append(line)
    return "\n".join(lines)


def model():
    pins = ["2" if (i, j) in GUIDES else "1" for j in range(NPIN) for i in range(NPIN)]
    lo = -(NASM // 2)
    asm = []
    for j in range(NASM):
        for i in range(NASM):
            x, y = (i + lo) * APITCH, (j + lo) * APITCH
            asm.append("4" if math.hypot(x, y) < FUEL_R else "5")
    plo = -(NPIN // 2)
    cells = [
        "c --- universe 1: a fuel pin",
        "1  3 -10.4   -1             u=1 imp:n=1  $ fuel",
        "2  0          1 -2          u=1 imp:n=1  $ gap",
        "3  2 -6.5     2 -3          u=1 imp:n=1  $ cladding",
        "4  1 -1.0     3             u=1 imp:n=1  $ water",
        "c --- universe 2: a guide tube",
        "5  1 -1.0    -4             u=2 imp:n=1  $ water in the tube",
        "6  2 -6.5     4 -5          u=2 imp:n=1  $ guide tube",
        "7  1 -1.0     5             u=2 imp:n=1  $ water",
        "c --- universe 3: 17 x 17 pins",
        wrap(f"8  1 -1.0  10 -11 12 -13 lat=1 u=3 imp:n=1 fill={plo}:{-plo} {plo}:{-plo} 0:0", pins),
        "c --- universe 4: an assembly; universe 5: water",
        "9  0         20 -21 22 -23  fill=3 u=4 imp:n=1  $ assembly",
        "10 1 -1.0   -20:21:-22:23        u=4 imp:n=1  $ water around it",
        "11 1 -1.0   -99                  u=5 imp:n=1  $ water",
        "12 1 -1.0    99                  u=5 imp:n=1  $ water",
        "c --- universe 6: 17 x 17 assemblies",
        wrap(f"13 1 -1.0  20 -21 22 -23 lat=1 u=6 imp:n=1 fill={lo}:{-lo} {lo}:{-lo} 0:0", asm),
        "c --- the reactor",
        "20 0        -50 40 -41   fill=6     imp:n=1  $ core",
        "21 4 -7.9   -50 42 -40              imp:n=1  $ lower core plate",
        "22 4 -7.9   -50 41 -43              imp:n=1  $ upper core plate",
        "23 1 -1.0   -50 52 -42              imp:n=1  $ lower plenum",
        "24 1 -1.0   -50 43 -53              imp:n=1  $ upper plenum",
        "25 4 -7.9    50 -51 52 -53          imp:n=1  $ core barrel",
        "26 1 -1.0   -54 56 -57 (51:-52:53)  imp:n=1  $ downcomer",
        "27 4 -7.9   -55 58 -59 (54:-56:57)  imp:n=1  $ vessel",
        "28 6 -2.3    60 -61 62 -63          imp:n=1  $ concrete shield",
        "29 6 -2.3   -61 64 -62              imp:n=1  $ concrete floor",
        "30 5 -0.0012 -98 (55:-58:59) #28 #29 imp:n=1  $ air",
        "31 0         98                     imp:n=0  $ outside world",
    ]
    surfs = [
        "1  cz 0.41", "2  cz 0.418", "3  cz 0.475", "4  cz 0.56", "5  cz 0.61",
        f"10 px {-PITCH / 2}", f"11 px {PITCH / 2}", f"12 py {-PITCH / 2}", f"13 py {PITCH / 2}",
        f"20 px {-APITCH / 2}", f"21 px {APITCH / 2}", f"22 py {-APITCH / 2}", f"23 py {APITCH / 2}",
        "40 pz -183", "41 pz 183", "42 pz -200", "43 pz 200",
        f"50 cz {CORE_R}", "51 cz 180", "52 pz -220", "53 pz 220",
        "54 cz 195", "55 cz 215", "56 pz -240", "57 pz 240", "58 pz -260", "59 pz 260",
        "60 cz 240", "61 cz 330", "62 pz -300", "63 pz 300", "64 pz -350",
        "98 so 600", "99 so 1e9",
    ]
    mats = [
        "m1 1001 2  8016 1                                  $ water",
        "m2 40090 0.98  50120 0.015  26056 0.005            $ zircaloy",
        "m3 92235 0.04  92238 0.96  8016 2                  $ uranium dioxide",
        "m4 26056 0.70  24052 0.19  28058 0.11              $ stainless steel",
        "m5 7014 0.79  8016 0.21                            $ air",
        "m6 1001 0.17  8016 0.56  14028 0.20  20040 0.06  13027 0.01   $ concrete",
    ]
    head = [
        "Example for the llevant docs: a small PWR-like core (not a real design)",
        "c Written by docs/examples/gen_core.py.",
    ]
    return "\n".join(head + cells + [""] + surfs + [""] + mats + ["nps 1", ""])


def power(path):
    import llevant
    import numpy as np

    # The centre of every fuel pin, at mid-height; the cell there gives the
    # pin's instance id.
    lo, plo = -(NASM // 2), -(NPIN // 2)
    centres = []
    for j in range(NASM):
        for i in range(NASM):
            ax, ay = (i + lo) * APITCH, (j + lo) * APITCH
            if math.hypot(ax, ay) >= FUEL_R:
                continue
            for q in range(NPIN):
                for p in range(NPIN):
                    if (p, q) not in GUIDES:
                        centres.append((ax + (p + plo) * PITCH, ay + (q + plo) * PITCH, 0.0))
    # Each pin's value goes to all of its lattice element (fuel, gap, cladding,
    # water), so that a slice of the whole core reads as a map.
    m = llevant.load(path)
    rows = ["instance,pin power [relative]"]
    for dx, dy in [(0, 0), (0.414, 0), (0.445, 0), (0.55, 0.55)]:
        r = m.classify(np.array([(x + dx, y + dy, z) for x, y, z in centres]))
        for (x, y, _), cell, inst in zip(centres, r.cell, r.instance):
            if cell not in (1, 2, 3, 4):  # a pin the barrel cuts
                continue
            shape = math.cos(0.5 * math.pi * min(math.hypot(x, y) / (FUEL_R + 25), 1.0))
            rows.append(f"{inst},{0.3 + 1.2 * shape:.4f}")
    return "\n".join(rows) + "\n"


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--power", action="store_true", help="also write core-power.csv")
    a = ap.parse_args()
    core = HERE / "core.i"
    core.write_text(model(), newline="\n")
    print(f"wrote {core}")
    if a.power:
        csv = HERE / "core-power.csv"
        csv.write_text(power(str(core)), newline="\n")
        print(f"wrote {csv}")
