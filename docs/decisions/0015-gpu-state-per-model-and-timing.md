# 0015: The GPU keeps per-level state for the model's own levels; GPU frames are timed back to back

2026-11-15. Accepted.

## Context

Two findings of a measurement pass over the GPU renderer.

**Per-ray private state.** The shader kept its per-level state (`lv_*` in
`prelude.wgsl`: the ray in the level's frame, a 3 × 3 rotation, unit, cell, lattice
index, instance, exit) for `MAX_LEVELS` = 8 levels in every model. With vec3 padding
that is ~240 words a ray, beyond any register file. It lives in local memory, and
WGSL zero-fills it for every ray. A ray that does almost nothing cost ~7 ns at boost
clock (`bricks.i`, close-up: one candidate, 7 program words, 14.6 ms for 2.1 M rays).
Frame time was linear in the number of rays, with no fixed cost.

**The clock the bench tools measured.** On the RTX 3060 Laptop (HP Victus, driver
610.91) the driver keeps the GPU at ~780 of 2100 MHz when submissions leave it idle.
It reports no throttle reason but "idle".

| Load (a pure FMA shader, 24 ms dispatches) | Clock | Power |
|---|---|---|
| Back to back | 1675 MHz | 79 W |
| 1 ms gaps | 1115 MHz | 47 W |
| 2 ms gaps | 832 MHz | 40 W |
| 4–25 ms gaps | 780 MHz | 33–39 W |

A pointer chase over 1 GB boosts like the FMA shader (1615 MHz), so the kind of work
does not decide it, only the gaps. `GpuModel::render` reads every frame back and
decodes it on the CPU, which leaves such gaps. So `gpu-ab`, `gpu-probe` and the
report's GPU frame ran at ~800 MHz on 2026-11-15: the whole model 250 ms against
75–98 ms boosted. A/B ratios moved too: the exit bound
([0014](0014-gpu-exit-bound.md)) looked 50–90 % slower on views that never step,
against 3–10 % at boost clock. On 2026-11-14 the same tools ran boosted, so a run
cannot tell which it got. The viewer is not affected: its window keeps the GPU busy
(1400–2000 MHz while orbiting).

## Decision

- **State per model.** `Export::levels` (`Model::levels`, export format 3) sizes the
  shader's per-level arrays: `MAXL = min(levels, MAX_LEVELS)`. A ray never goes deeper
  than the model's levels (cells at the deepest level have no fill), so `locate`
  never writes past them. Models deeper than `MAX_LEVELS` stop there, as before. The
  level count joins the program and BVH depths that already sized the stacks: the
  shader stays shared between a staged model, the same completed and edits of it.
- **Timing.** `llevant-bench`'s `gpu_time` times frames with
  `GpuModel::render_seconds` (`render`'s sliced job without the readback), back to
  back, after a 5 s warm-up (0.5 s before each later timing), and prints the SM clock
  `nvidia-smi` reads after them. `gpu-ab` and `report` use it.

## Evidence

`gpu-ab`, 1920 × 1080, 5 rounds, medians of the exit-bound shader. The clock stayed at
1800–2017 MHz in every run. "8 levels" is the same build with `MAXL` fixed at 8:

| Model (levels), view | 8 levels | Per model |
|---|---|---|
| Reference (4), whole model | 98.7 ms | 92.5 ms (−6 %) |
| Reference, close-up | 24.2 ms | 15.4 ms (−36 %) |
| Reference, inside | 34.2 ms | 25.6 ms (−25 %) |
| Reference, kept envelope | 22.3 ms | 13.1 ms (−41 %) |
| `bricks.i` (1), three views | 10.4–13.8 ms | 5.8–6.7 ms (−44 to −51 %) |
| `tokamak.i` (2), four views | 8.6–14.9 ms | 4.6–9.0 ms (−40 to −47 %) |

- The viewer, reference model, `--orbit 10` (3 runs): previews at mean scale 0.96–0.98
  instead of 0.85 in the same budget, a still view refined in 0.22–0.25 s instead of
  0.25 s, orbit frames unchanged (95th percentile 21.1–21.7 ms).
- `gpu-check` (150² rays) on the reference model, `core.i` (5 levels of lattices) and
  `tokamak.i`: every view and visibility pair gives the same counts as with 8 levels.
  GPU and viewer tests pass.

## Consequences

- Even with 2 levels, a ray that does almost nothing still costs ~4.4 ns: the rest of
  the private state (code and BVH stacks, the paused-ray state) is next. The
  per-level rotation is 40 % of the per-level state.
- At boost clock the exit bound costs 3–10 % on views that never step (11–20 % with
  this change, since the rest got cheaper), more than the 1–3 % 0014 measured at the
  low clock. Its extra `f32` per program node is private state too.
- GPU timings taken with `render` (`gpu-probe`, `gpu-view`, `gpu-levels`,
  `gpu-slices`) depend on the driver's clock: compare them only with each other, in
  one run.
