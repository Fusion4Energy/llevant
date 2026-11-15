# 0001: The CPU engine is exact and the reference; the GPU only draws

2026-10-08. Accepted.

## Context

An interactive 3D view needs about 1.2·10⁸ rays a second (1920 × 1080 at 60
frames a second). The CPU's `first_hits` does about 6·10⁵ a second on 96 threads,
so no CPU optimisation closes that gap. Everything else llevant answers (points,
lines, volumes, checks, fingerprints, slices) must be exact.

## Decision

There is one compiler and two evaluators of the model it compiles.

- **The CPU engine (`crates/llevant`) is the reference.** It works in f64, is exact
  up to its stated tolerance (1e-8 cm), complete and deterministic. It answers
  every query, including the picks in the viewer and the static renders.
- **The GPU renderer (`crates/llevant-gpu`) only draws the 3D view.** It works in
  f32 and is approximate by design: a pixel may be wrong at an edge, and gaps and
  overlaps are not detected.
- The renderer reads `Model::export()`, never the MCNP input, so MCNP semantics
  live in one place. The format is versioned (`EXPORT_VERSION`).
- The CPU tests the GPU: `tests/vs_cpu.rs` and `llevant-bench gpu-check` compare
  ray by ray. They hold the GPU to agreement with the CPU, not to equality.

## Why the engine stays on the CPU

- Volume work needs f64: models 20 m long with millimetre features, tolerances
  around 1e-5 cm, and cancellation in near-tangent intersections. Consumer GPUs
  run f64 at 1/32 to 1/64 of their f32 speed.
- Work per line varies (cells, program lengths, nesting, crossings sorted per
  line), so GPU threads would diverge.
- CI runners and many users' machines have no GPU, and the CPU is fast enough for
  quality checks.

## Consequences

- f32 needed care, each fix found by comparing against the CPU (`docs/internals/rendering.md`, "GPU renderer"):
  - steps past a crossing scaled by the angle of incidence;
  - quadrics rewritten about a point near the cells that use them;
  - tori refined on the torus function itself.
- Agreement on the reference model: 99.4–100 % of rays per view (`gpu-check`, nine
  views).
- Exact answers shown in the viewer (picks, the inspector, problems) are CPU
  queries drawn over the GPU image.
