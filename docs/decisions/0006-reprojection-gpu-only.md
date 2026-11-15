# 0006: The 3D view reprojects every frame and traces on the GPU alone

2026-10-31. Accepted. Supersedes the hybrid backend (CPU previews, GPU
refinement; 2026-10-10).

## Context

While the camera moved, the viewer showed the last traced image where it had been
traced until a new preview landed. Smooth motion therefore needed many previews a
second, which only 16 CPU threads tracing next to the GPU could give: the hybrid
backend. That was two tracers to keep in step, CPU threads competing with the UI,
and a slower refinement.

## Decision

- **Reprojection** (`crates/llevant-viewer/src/view3d/reproject.rs`): every frame
  while the camera moves, the hits shown are moved to the camera wanted now and
  shaded from there.
  - Each pixel's point (its hit, its first ghost boundary, or its ray's end) goes
    to the pixel it falls in now; the nearest wins (`atomicMin` on its depth).
  - A pixel no point fell in takes its nearest neighbour's.
  - Distances are measured again along the new rays.
  - New traces replace what was moved as they land.
- **One tracer per machine.** The 3D view traces on the GPU. The CPU traces only
  where there is no hardware GPU. `HybridBackend` is gone.

## Evidence

Measured on this laptop (16 threads, RTX 3060 Laptop), reference model, orbit of
10 s:

| Backend | Orbit frames, 95th percentile | Previews | Refinement of a still view |
|---|---|---|---|
| Hybrid (CPU previews) | 25–26 ms | 26/s | 2.4–2.6 s |
| GPU alone, with reprojection | 19 ms | 12.4/s | 0.62 s |
| CPU alone | 26 ms | | 7.2 s |

With the GPU backend's job preparation moved to its own thread: orbit frames 19–20
ms at the 95th percentile, refinement 0.70–0.75 s.

Reprojected against freshly traced images (`tests/images.rs`, zoo models): an
identity reprojection changes no pixel, and a 3° turn differs at 0.7–1.8 % of
pixels.

## Consequences

- What the camera had not seen shows as nothing until traced. These holes are
  bigger than one pixel and last a few frames, until the next preview lands.
- `llevant-viewer --no-reproject` turns reprojection off, for A/B runs.
- The viewer README and `docs/internals/rendering.md` describe two backends: `gpu`
  and `cpu`.
