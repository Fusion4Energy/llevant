# Benchmarks

Current numbers for what the README and the docs quote, measured on one machine.
Regenerate them after a change that moves them (the tables below are
`llevant-bench report`'s Markdown output):

```sh
cargo build --profile dist -p llevant-bench
python tools/synthetic/gen_models.py untracked/synthetic
target/dist/llevant-bench report untracked/big_example_asembled.mcnp untracked/synthetic/*.i \
  --json report.json --markdown report.md
```

The reference model (`big_example_asembled.mcnp`, built by gitronics, not in git) is a
fusion reactor model:

- 378 MB of input, 425 023 cells, 592 588 surfaces;
- 187 universes nested up to 4 levels deep, and six `LAT=1` cells;
- mostly general planes and GQ cylinders converted from CAD.

The synthetic models (`tools/synthetic/gen_models.py`) are other shapes:

- `tokamak`: toroidal sectors with slab fillers;
- `core`: a PWR-like lattice of lattices;
- `sparse`: 1 000 spheres in a 5 km world;
- `bricks`: axis-aligned boxes;
- `tori`: torus shells.

What `report` measures:

- **Workload.** Points and 100 cm lines are drawn uniformly inside each model's
  contents (`Model::extent`) with a fixed seed. Lines go in random directions. "One
  thread" runs a tenth of the batch on a pool of one thread. Rates are the best of
  three calls.
- **Staged compile and recompile.** The compile up to the BVHs, without octrees
  (`Model::compile_staged`: what the viewer shows first). Then the first surface of
  the input's last cell moved by 1e-3 cm and the model recompiled against the
  compiled one (`Model::recompile`), staged and then complete, with the number of
  universes it built again ([decision
  0010](decisions/0010-staged-models-and-recompile.md)).
- **Drag.** The same surface dragged live ([decision
  0011](decisions/0011-live-drag-drafts.md)):
  - `Draft::begin`;
  - the 95th percentile of 41 steps moving its last coefficient by up to 1 cm either
    way, with the cells it moves;
  - with a GPU, the best of three writes of a step into the upload
    (`GpuModel::patch`, waited for).
- **Slice.** 1000 rows across the middle of the contents, at z = mid.
- **CPU render.** 800 × 600, solid cells, from the direction (0.6, 0.7, -0.4).
- **Check.** `Model::volumes` over the contents, on lines about 1/400 of their longest
  side apart, rounded to 1, 2 or 5.
- **GPU.** The upload from the model's lean export, and the best of four 1920 × 1080
  frames of the render's view, solid cells, traced back to back after a 5 s warm-up
  (frames read back one at a time let the driver drop the clock to a third, [decision
  0015](decisions/0015-gpu-state-per-model-and-timing.md)).

## Engine, CLI and GPU

Measured 2026-11-03 at commit `cbc5379`, `dist` profile, on AMD64 Family 25 Model 80 Stepping 0, AuthenticAMD (16 threads, windows), GPU NVIDIA GeForce RTX 3060 Laptop GPU.

| | big_example_asembled.mcnp | bricks.i | core.i | sparse.i | tokamak-coarse.i | tokamak.i | tori.i |
|---|---|---|---|---|---|---|---|
| Cells / compiled surfaces / levels | 425023 / 592637 / 4 | 32001 / 103 / 1 | 16 / 23 / 5 | 1002 / 1001 / 1 | 7805 / 337 / 2 | 6293 / 308 / 2 | 470 / 50 / 1 |
| Compile (no cache) | 37.81 s | 0.06 s | 0.00 s | 0.15 s | 0.05 s | 0.02 s | 0.01 s |
| Compile to a staged model (no octrees) | 9.55 s | 0.05 s | 0.00 s | 0.00 s | 0.02 s | 0.02 s | 0.00 s |
| Recompile, one surface moved: staged / complete (universes rebuilt) | 2.44 s / 2.64 s (1) | 0.05 s / 0.05 s (1) | 0.00 s / 0.00 s (1) | 0.00 s / 0.18 s (1) | 0.01 s / 0.01 s (1) | 0.01 s / 0.01 s (1) | 0.00 s / 0.01 s (1) |
| That surface dragged: start / step, 95th percentile (cells moved) | 90 ms / 0.3 ms (16) | — | — | — | 1 ms / 0.0 ms (2) | 1 ms / 0.0 ms (2) | — |
| Read the compiled model (cache) | 1.41 s (1462 MB) | 0.00 s (6 MB) | 0.00 s (0 MB) | 0.00 s (0 MB) | 0.00 s (4 MB) | 0.00 s (1 MB) | 0.00 s (1 MB) |
| Points, all threads / one | 7.02e6 / 6.18e5 per s | 1.12e7 / 1.26e6 per s | 1.96e7 / 2.44e6 per s | 5.91e7 / 4.71e6 per s | 1.20e7 / 1.37e6 per s | 1.04e7 / 1.05e6 per s | 1.01e7 / 1.11e6 per s |
| 100 cm lines, all threads / one | 4.12e5 / 4.22e4 per s | 8.96e5 / 1.06e5 per s | 1.96e5 / 3.17e4 per s | 1.51e6 / 1.84e5 per s | 7.05e5 / 8.45e4 per s | 1.40e6 / 1.85e5 per s | 6.84e5 / 7.42e4 per s |
| Slice, 1000 rows | 93 ms | 4 ms | 42 ms | 1 ms | 16 ms | 3 ms | 22 ms |
| CPU render, 800 × 600 | 2027 ms | 447 ms | 188 ms | 934 ms | 1324 ms | 473 ms | 1025 ms |
| Fingerprints | 0.43 s | 0.01 s | 0.00 s | 0.00 s | 0.00 s | 0.00 s | 0.00 s |
| Gap and overlap check | 91.2 s (h = 5.0 cm) | 1.0 s (h = 1.0 cm) | 14.2 s (h = 1.0 cm) | 3.5 s (h = 0.2 cm) | 26.6 s (h = 2.0 cm) | 9.9 s (h = 2.0 cm) | 7.4 s (h = 2.0 cm) |
| GPU upload | 1.4 s (1512 MB) | 8.2 s (5 MB) | 0.1 s (0 MB) | 7.7 s (1 MB) | 0.0 s (5 MB) | 0.0 s (1 MB) | 7.8 s (1 MB) |
| GPU frame, 1920 × 1080 | 280 ms | 13 ms | 9 ms | 16 ms | 10 ms | 11 ms | 10 ms |
| GPU: a drag's step written into the upload | 7.4 ms | — | — | — | 0.5 ms | 0.4 ms | — |

Against the previous table (`a719ae9`), the drag rows are new ([decision
0011](decisions/0011-live-drag-drafts.md)); the rest is unchanged within run-to-run
noise. The 8 s uploads of three small models are most likely the driver compiling their
shaders again on a first run (see below); the upload now also reserves 256 KB of slack. The surface measured on the reference model
moves 16 cells; `llevant-bench drag` measures the others (steps of 0.5–13 ms at the
95th percentile, 36 ms where a roi cascade bounds 6 330 cells again). `bricks.i`,
`core.i`, `sparse.i` and `tori.i` cannot be dragged on that surface: it bounds the
problem's `IMP:N=0` cell.

The table at `a719ae9`, against the previous one (`f118416`, `v0.1.0` before the release commit), the
staged compile and the recompile are new ([decision
0010](decisions/0010-staged-models-and-recompile.md)); the rest is unchanged within run
to run noise (a back-to-back A/B of `v0.1.0` and this commit, release profile, reference
model: points 6.86 / 6.65·10⁶ per s, lines 3.95 / 3.99·10⁵ per s, check 94.4 / 94.2 s).
The saved model is 1462 MB: `v0.1.0` saves 1474 MB (the 1577 MB of the previous table
predates it), and node-box ids local to each cell, small numbers, pack a little
tighter.

Point and line rates are the best of three calls, as the other timings. The GPU upload
is erratic on a first run: the driver compiles the shader for each new depth of
programs and BVHs (8–9 s once, then cached by the driver; staged and complete models
share it), and one report took 28 s for the reference model.

## Viewer

Reference model, solid cells, default window, GPU backend, reprojection on.
Measured with:

- `llevant-viewer <model> --orbit 10 --screenshot out.png` for the orbit;
- `llevant-viewer <model> --storm` for the edit storm (the UI thread's own work per
  frame).

| | |
|---|---|
| Orbit, 10 s: frame time median / 95th percentile | 16.1 ms / 21–22 ms (display rate 60 Hz) |
| Previews while orbiting ("While moving" at its default, 66 ms) | ~11 per second, mean scale 0.84–0.86 (at 33 ms: ~20, 0.45–0.50) |
| A still view refined to full resolution | 0.30–0.38 s (one job, blocks shown as they land) |
| Edit storm: worst frame of the UI thread's own work | 18.6–19.4 ms (631 frames, median 0.35 ms) |
| The same orbit on the CPU backend (16 threads): 95th percentile / refinement | 32.5 ms / 4.9 s |

Measured 2026-11-14 at `70182a6`, `release` profile, against `13f08c2` the same day:
previews 12 a second at scale 0.07, refinement 0.65 s, orbit 19.6 ms at the 95th
percentile; CPU backend 32.3 ms / 6.35 s ([decision
0013](decisions/0013-viewer-tracing-latency.md)). Frame times include the display's
vsync; the window waits a little longer behind a GPU that is busy more of the time.
The storm's frames over 8 ms are the label rules over nearly every cell (`select $ e`,
`isolate $ e`, 13–19 ms on the UI thread), added after the previous table.

## History

Earlier numbers, and what was tried and did not help, are in the decision records
([`decisions/`](decisions/README.md)), and in the git history of
`docs/measurements.md`. The 96-thread workstation numbers of October 2026 are kept
there too: about 7·10⁶ points/s and 2.5·10⁵ lines/s on the reference model, and a
5 cm gap check of its 36 × 37 × 35 m building in 45 s.
