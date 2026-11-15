# 0014: A cell's GPU exit is where its program can first turn false

2026-11-14. Accepted. Takes up what [0013](0013-viewer-tracing-latency.md) left
undone: making the slowest rays cheaper.

## Context

The GPU found a cell's exit as the first crossing of any surface in its program. A
CAD void cell is the space around parts (`#1 #2 …`), so its program holds the
surfaces of every part it is cut around. A ray passing a part by crossed that part's
planes and stopped at each one. It located itself again, found the same cell, and
walked the void's program again.

On the reference model, the four slowest rays of a 1024² whole-model view took 96–125
steps. 81–85 % of those steps kept the same cell. About 140 rays per million walked
20–40 k program words each, and a GPU job lasts as long as its slowest rays. They
set the time of a still view, and they made previews of cut views give up in patches.

## Decision

`exit_bound` (`shader/tracking.wgsl`) finds, in one pass over the program, where it
can first turn false:

- A leaf keeps its side from the ray's position to its next crossing. If it is true
  at the midpoint between the two, it is true up to that crossing.
- An AND of such leaves is true until the earliest of its children's crossings.
- An OR with such a child is true until the latest of its true children's crossings.

The result is never past the true exit, so no cell is skipped. When the program is
not surely true (a crossing within the tolerance of the start), the exit is the first
crossing, as before. Node boxes still decide whole subtrees along the stretch.
`gap_exit` keeps the first crossing: in a gap, any cell may start there.

Each open program node keeps only one extra `f32`:

- "not sure" is encoded as −10³⁰, so an OR is a max and an AND a min;
- the surface at the bound is kept in the BVH stack, which is idle while an exit is
  found;
- the AND/OR flag moved into the top bit of the node's end word.

Every private word is held for every ray of a dispatch. The first version used four
words per node and made views that never step ~20 % slower.

`EXIT_BOUND` is an override constant, for A/B runs (`gpu-ab`, and
`GPU_EXIT_BOUND=0` for `gpu-probe` and `gpu-check`).

## Evidence

Reference model, RTX 3060 Laptop. `gpu-ab`, 1920 × 1080, 11 rounds, median frame
time. "Before" is the shader before this change, taken from the first A/B of the day:

| View | Before | First crossing (this shader) | Exit bound |
|---|---|---|---|
| Whole model | 251 ms | 246 ms | 98 ms |
| Close-up | 23.5 ms | 23.0 ms | 24.0 ms |
| Inside | 33.4 ms | 32.4 ms | 33.6 ms |
| Kept envelope | 21.8 ms | 21.1 ms | 22.5 ms |

The views that never step are within +1–3 % of before. Packing the AND/OR flag paid
for most of the bound's cost there.

Per ray, whole-model view at 1024², uncapped (`gpu-probe`):

| | First crossing | Exit bound |
|---|---|---|
| Max steps | 124 | 24 |
| Max words (exit + locate) | ~64 k | ~17 k |
| p99 words (exit + locate) | 45 + 39 | 38 + 24 |
| GPU time | 581 ms | 232 ms |

The viewer, `--orbit 10` (two runs each):

| | Before | After |
|---|---|---|
| Still view refined | 0.26–0.31 s | 0.19–0.20 s |
| Previews per second, mean scale | 10.5–11.2, 0.86–0.90 | 10.8–11.2, 0.88–0.91 |
| Orbit frames, 95th percentile | 21.4–21.6 ms | 21.7–22.4 ms |

Agreement with the CPU (`gpu-check`, 150² rays, 80 view and visibility pairs): the
same cell and hit/miss counts in every pair, and fewer or equal normals off (for
example 9 → 3). Fewer relocations mean fewer f32 roundings. `tests/vs_cpu.rs`
(`void_exits_skip_crossings_that_keep_the_cell`) traces rays past a row of parts in
a void cut around them: 11 337 steps → 378 without octrees, 7 337 → 378 with them,
with every hit as the CPU's.

## Consequences

- Rays that pass parts by are about as cheap as rays through empty space. What is
  left of a still view's refine time is ordinary work, not a tail of a few rays.
- The preview work cap (`GpuBackend::preview_work`, 2048 words) stays, as a bound on
  any single ray's time. Rays that reached it were the ones walking void programs, so
  cut views should give up less often. Not measured yet: 0013's cut-view table was
  measured by hand.
- The CPU engine is unchanged: `trace` still collects every crossing, for exactness
  ([decision 0001](0001-cpu-exact-gpu-approximate.md)).
