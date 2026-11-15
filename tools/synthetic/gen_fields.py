"""Mock results over a tokamak model, to try fields (`llevant --field`, the
viewer's Fields panel) on a real-sized model.

    python3 tools/synthetic/gen_fields.py untracked/big_example_asembled.mcnp untracked/fields

writes into the output directory:

- `flux.msht`: a neutron flux FMESH tally (number 14) in MCNP's meshtal
  format (`out=cf`), on a cylindrical mesh about z: 20 cm in r and z, 15° in
  theta. The flux falls off with the distance from the plasma and with the
  material crossed on the way (llevant traces the line from the plasma ring to
  each voxel, so shields, ports and gaps show), with relative errors growing
  where it is low; voxels below 1e-13 scored nothing (zero: no value).
- `heating.csv`: nuclear heating per cell (W/cm³, `cell,heating [W/cm3]`):
  the flux at points sampled in each cell, times its density, scaled to about
  10 W/cm³ in steel at the first wall.

None of it is physics: only shapes and magnitudes that look like a D-T
tokamak's, so that colour scales, log ranges and missing values get exercised.
The plasma is found as the model's void cell at (R0, 0, Z0) (the defaults fit
the reference model).
"""

import argparse
import math
import time
from pathlib import Path

import numpy as np

import llevant


def plasma_points(p, a, rng):
    """A random point of the plasma (an ellipse in the R-Z plane, swept
    about z) near each point's toroidal angle (within 10°)."""
    phi = np.arctan2(p[:, 1], p[:, 0]) + np.radians(rng.uniform(-10, 10, len(p)))
    rho = 0.8 * np.sqrt(rng.uniform(0, 1, len(p)))
    ang = rng.uniform(0, 2 * np.pi, len(p))
    rr = a.r0 + a.minor * rho * np.cos(ang)
    zz = a.z0 + a.minor * a.elongation * rho * np.sin(ang)
    return np.stack([rr * np.cos(phi), rr * np.sin(phi), zz], axis=1)


def flux(m, p, a, rng, sources=6):
    """Mock flux per source particle (1/cm²) at points `p` ((N, 3)): the
    mean over `sources` random points of the plasma."""
    total = np.zeros(len(p))
    for _ in range(sources):
        total += flux_from(m, p, plasma_points(p, a, rng), a)
    return total / sources


def flux_from(m, p, src, a):
    """Flux at `p` from the source points `src`."""
    # Material crossed between the plasma and each point.
    crossed = np.zeros(len(p))
    for k in range(0, len(p), 200_000):
        tr = m.trace_lines(src[k : k + 200_000], p[k : k + 200_000])
        length = tr.t1 - tr.t0
        solid = np.where(tr.material > 0, length, 0.0)
        starts = tr.offsets[:-1].astype(np.int64)
        sums = np.add.reduceat(solid, np.minimum(starts, len(solid) - 1)) if len(solid) else 0
        empty = tr.offsets[1:].astype(np.int64) == starts
        crossed[k : k + 200_000] = np.where(empty, 0.0, sums)
    # Distance outside the plasma's D shape (an ellipse in the R-Z plane).
    rr = np.hypot(p[:, 0], p[:, 1]) - a.r0
    zz = p[:, 2] - a.z0
    out = np.sqrt((rr / a.minor) ** 2 + (zz / (a.minor * a.elongation)) ** 2) - 1.0
    d = np.maximum(out, 0.0) * a.minor
    return 2e-4 * np.exp(-crossed / a.attenuation) / (1.0 + (d / 400.0) ** 2)


def write_meshtal(path, r, z, theta, phi, err, origin):
    rc, zc, tc = (0.5 * (b[1:] + b[:-1]) for b in (r, z, theta))
    vol = (
        math.pi
        * np.diff(r**2)[:, None, None]
        * np.diff(z)[None, :, None]
        * np.diff(theta)[None, None, :]
    )
    R, Z, T = np.meshgrid(rc, zc, tc, indexing="ij")
    cols = np.stack(
        [R.ravel(), Z.ravel(), T.ravel(), phi.ravel(), err.ravel(), vol.ravel(), (phi * vol).ravel()],
        axis=1,
    )

    def bounds(b, f):
        return "".join(f.format(x) for x in b)

    with open(path, "w") as out:
        out.write(
            "mcnp6  version 6     ld=01/26/23  probid =  10/22/26 12:00:00 \n"
            " Mock neutron flux over the tokamak model (llevant tools/synthetic/gen_fields.py)\n"
            " Number of histories used for normalizing tallies =      100000000.00\n\n"
            " Mesh Tally Number        14\n"
            " neutron  mesh tally.\n\n"
            " Tally bin boundaries:\n"
            "               origin at {:10.2E}{:10.2E}{:10.2E} axis in   0.00E+00  0.00E+00  1.00E+00 direction,"
            " VEC direction   1.00E+00  0.00E+00  0.00E+00\n".format(*origin)
            + "    R direction:   " + bounds(r, "{:10.2f}") + "\n"
            + "    Z direction:   " + bounds(z, "{:10.2f}") + "\n"
            + "    Theta direction (revolutions):  " + bounds(theta, "{:10.3f}") + "\n"
            + "    Energy bin boundaries: 0.00E+00 1.00E+36\n\n"
            "        R         Z         Th    Result     Rel Error     Volume    Rslt * Vol\n"
        )
        np.savetxt(out, cols, fmt="%11.3f%10.3f%10.3f %.5E %.5E %.5E %.5E")


def main():
    ap = argparse.ArgumentParser(description="Mock results over a tokamak model.")
    ap.add_argument("model")
    ap.add_argument("out", type=Path)
    ap.add_argument("--r0", type=float, default=623.0, help="plasma major radius (cm)")
    ap.add_argument("--z0", type=float, default=50.0, help="plasma centre height (cm)")
    ap.add_argument("--minor", type=float, default=210.0, help="plasma minor radius (cm)")
    ap.add_argument("--elongation", type=float, default=1.7)
    ap.add_argument("--attenuation", type=float, default=11.0, help="cm of material per e-fold")
    ap.add_argument(
        "--points", type=int, default=200_000_000, help="points sampled for heating (90% near the vessel)"
    )
    a = ap.parse_args()
    a.out.mkdir(parents=True, exist_ok=True)

    t = time.time()
    m = llevant.load(a.model)
    lo, hi = m.extent()
    print(f"loaded in {time.time() - t:.1f} s; extent {lo} .. {hi}")
    plasma = m.locate(a.r0, 0.0, a.z0)
    assert plasma.material == 0, f"no void plasma at R0, Z0: {plasma}"

    # The mesh: about z, from below the model up, out to its edge.
    rmax = max(abs(lo[0]), abs(lo[1]), hi[0], hi[1])
    r = np.arange(0.0, rmax + 20.0, 20.0)
    z = np.arange(0.0, hi[2] - math.floor(lo[2] / 10.0) * 10.0 + 20.0, 20.0)
    theta = np.linspace(0.0, 1.0, 25)
    # A round origin, so that the meshtal header (which rounds) is exact.
    origin = (0.0, 0.0, math.floor(lo[2] / 10.0) * 10.0)
    rc, zc, tc = (0.5 * (b[1:] + b[:-1]) for b in (r, z, theta))
    R, Z, T = np.meshgrid(rc, zc, tc, indexing="ij")
    ang = 2 * np.pi * T
    centres = np.stack([R * np.cos(ang), R * np.sin(ang), Z + origin[2]], axis=-1).reshape(-1, 3)
    t = time.time()
    rng = np.random.default_rng(14)
    phi = flux(m, centres, a, rng)
    err = np.clip(0.01 * (2e-4 / np.maximum(phi, 1e-30)) ** 0.12, 0.002, 1.0)
    err *= rng.uniform(0.8, 1.2, size=err.shape)
    phi = np.where(phi < 1e-13, 0.0, phi * rng.normal(1.0, np.minimum(err, 0.3)))
    err = np.where(phi > 0, np.minimum(err, 1.0), 0.0)
    shape = (len(rc), len(zc), len(tc))
    write_meshtal(a.out / "flux.msht", r, z, theta, phi.reshape(shape), err.reshape(shape), origin)
    print(f"flux.msht: {phi.size} voxels ({(phi > 0).mean():.0%} scored) in {time.time() - t:.1f} s")

    # Heating per cell: the mesh flux at points in each cell, times density.
    t = time.time()
    sums, counts = {}, {}
    per = 2_000_000
    # Most points around the vacuum vessel, where cells are small and the
    # heating high; the rest over the whole model.
    near_lo = (-(a.r0 + 3 * a.minor),) * 2 + (a.z0 - 2.2 * a.minor * a.elongation,)
    near_hi = ((a.r0 + 3 * a.minor),) * 2 + (a.z0 + 2.2 * a.minor * a.elongation,)
    for k in range(0, a.points, per):
        box = (near_lo, near_hi) if k < 0.9 * a.points else (lo, hi)
        p = rng.uniform(*box, size=(per, 3))
        c = m.classify(p)
        keep = c.material > 0
        p, cells, ids = p[keep], c.cell[keep], c.ids[keep]
        # g/cm³ from the cards (atom densities taken as steel).
        rho = np.array(
            [-lab.density if (lab.density or 0.0) < 0 else 7.9 for lab in c.labels]
        )[ids]
        # Nearest voxel of the mesh.
        ri = np.clip((np.hypot(p[:, 0], p[:, 1]) / 20.0).astype(int), 0, len(rc) - 1)
        zi = np.clip(((p[:, 2] - origin[2]) / 20.0).astype(int), 0, len(zc) - 1)
        ti = np.clip((np.mod(np.arctan2(p[:, 1], p[:, 0]) / (2 * np.pi), 1.0) * len(tc)).astype(int), 0, len(tc) - 1)
        f = phi.reshape(shape)[ri, zi, ti] * rho
        u, inv = np.unique(cells, return_inverse=True)
        s = np.bincount(inv, weights=f)
        n = np.bincount(inv)
        for cell, x, y in zip(u, s, n):
            sums[cell] = sums.get(cell, 0.0) + x
            counts[cell] = counts.get(cell, 0) + y
    # 10 W/cm³ in steel (7.9 g/cm³) where the flux is 2e-4.
    scale = 10.0 / (2e-4 * 7.9)
    with open(a.out / "heating.csv", "w") as out:
        out.write("cell,heating [W/cm3]\n")
        for cell in sorted(sums):
            h = scale * sums[cell] / counts[cell]
            if h > 0:
                out.write(f"{cell},{h:.4e}\n")
    print(f"heating.csv: {len(sums)} cells in {time.time() - t:.1f} s")


if __name__ == "__main__":
    main()
