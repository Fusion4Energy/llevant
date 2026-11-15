# 0003: Beyond universe 0's problem box, only the `IMP:N=0` cells are checked

2026-10-18. Accepted. Supersedes the "tight world" experiment of 2026-10-17
(branch `exp/tight-region`, shelved).

## Context

Universe 0 used to be bounded by a ±10 km world box. Cells converted from CAD can
have boxes kilometres wide, so boxes and octrees were sized for space that holds
nothing but the graveyard.

## Decision

Universe 0's region is the **problem box**: the intersection of the boxes of the
complements of its `IMP:N=0` cells (±102 m on the reference model). Every point
beyond it is in one of those cells, so only they are checked there, by the fast
path and by brute force alike (`Model::problem_box`).

The only answers that differ from checking every cell are overlaps of another cell
with the graveyard out there. They read as the graveyard cell, and MCNP kills
particles there anyway.

## Evidence

The shelved first attempt tightened the region but still checked, beyond it, every
cell whose loose box could reach out there. Views from outside the sphere became 5
to 24 times slower: 3.5 s to 26.5 s for a solid view. The problem box keeps the
gains inside without that cost. Reference model, 16 threads:

| | Before | After |
|---|---|---|
| Points | 1.98e6/s | 2.36e6/s |
| CPU view from outside the sphere | 1 554 ms | 542 ms |
| Close-up | 1 048 ms | 722 ms |
| GPU view from inside | 1.75 s | 1.43 s |

`verify` gave 0 disagreements and the zoo goldens were unchanged.

## Consequences

- Without a bounded graveyard, the problem box falls back to the ±10 km world box,
  and beyond it every level-0 cell is checked.
- A universe opened on its own (`Model::universe`) is cut to a box that becomes the
  new model's problem box exactly: beyond it lies one void `IMP:N=0` cell.
