# 0002: Octree leaves keep each candidate's program specialised to the leaf

2026-10-17. Accepted.

## Context

Envelope cells converted from CAD have long programs: about 500 words on average,
up to 4 600 for a void envelope cell. Their boxes are loose: about 93 level-0 boxes
contain a typical point of the reference model. Octrees cut down the candidates,
but each candidate still evaluated its whole program: 221 surface evaluations per
point at level 0.

## Decision

Each candidate of an octree leaf also keeps its program specialised to the leaf box
(`expr::specialize`):

- leaves whose side is decided over the box are folded in;
- nodes whose boxes miss the leaf are folded in;
- a residual no shorter than the program is not copied;
- identical residuals are stored once per batch of 1024 leaves.

Points and lines evaluate the residual of the leaf that holds the exact point.
Tracing takes its crossings from the residuals' surfaces. `Options::residuals`
turns this off, for A/B runs or when memory is short.

## Evidence

On `envelope_structure.mcnp`, 16 threads, workloads replayed:

| | Before | After |
|---|---|---|
| Points | 1.75e6/s | 5.22e6/s |
| Surface evaluations per point at level 0 | 221 | 38 |
| Random 100 cm lines | 8.9e4/s | 3.2e5/s |
| Close-up view of an envelope | 21.1 s | 1.95 s |

On the reference model:

| | Before | After |
|---|---|---|
| Points | 8.9e5/s | 1.43e6/s |
| Lines across the hull | 4.1e4/s | 9.3e4/s |
| Octree memory | 0.23 GB | 0.96 GB |
| Octree build | 9.0 s | 13.7 s |

On the GPU, the reference model's inside view went from 6.42 s to 1.43 s.

## Consequences

- Points stay exactly the brute-force labels.
- Lines no longer equal brute force bit for bit. Of 100 000 random lines on the
  reference model:
  - 516 differ only in which of two surfaces within the tolerance of an end is
    named;
  - 9 differ by an on-surface sliver of about 1.4e-8 cm that brute force shows
    between two nearly coincident planes of one cell.

  Overlaps, gaps and volumes are identical. So fast and brute-force tracing are
  compared with `reference::traces_agree` (`docs/contracts.md`), not by equality.
- Taking crossings from whole programs kept exact equality but gave up most of
  the gain. It was rejected.
