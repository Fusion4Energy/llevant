# 0011: A surface dragged is previewed by a draft, patched in place; the edit compiles on release

2026-11-03. Accepted.

## Context

An edit of the input recompiles against the model shown ([decision
0010](0010-staged-models-and-recompile.md)): 2.5–5 s on the reference model, most of it
the front end, whole universes rebuilt and a 1.5 GB GPU upload. That is fine for an
edit typed and applied, but too slow to follow a surface as it is dragged.

Moving one surface changes little:

- its shapes (one or a few, shared by every cell that uses it);
- the boxes of the cells using it;
- through the rois, the boxes of the universes those cells fill;
- the BVH boxes above all of these.

Every query stays exact without octrees and without node boxes: both are accelerators.
A census of the reference model (592 587 surfaces used) found 6 surfaces used by lattice
cells, none with `TRCL` copies, and each in one universe. Cells per surface: p50 6,
p99 164, max 6 350.

## Decision

- **`llevant::draft::Draft`** (`draft.rs`) holds a copy of the model shown (sharing its octrees)
  and moves one surface in it.
  - `Draft::begin` refuses what it cannot follow exactly, with the reason:
    - a cut model;
    - a surface not in the input, or not compiled;
    - `TRCL` copies;
    - surfaces used by lattice cells;
    - an `IMP:N=0` cell of universe 0 when a problem box exists.
  - `Draft::set(coeffs)` does the following:
    - rebuilds the shapes on the compiler's own path (`build_surface`);
    - bounds the moved cells against their unit's roi, leaving their node boxes
      unrestricted;
    - grows the rois of the universes they fill, lattices too, and bounds those
      cells again (rois only grow, so boxes stay conservative);
    - refits the BVHs;
    - turns the CPU octree of every moved unit off (`pending`, so the draft cannot be
      saved).
  - It returns a `DraftPatch`: what changed, in export layout. Patches are cumulative:
    each one covers every record any step changed, so only the latest is ever needed.
    `revert_patch` gives the model back.
- **`GpuModel::patch`** writes a patch into the model uploaded, in place:
  - one staging buffer and one copy per run of contiguous words;
  - no new upload and no new shader.
  - A unit with at most 64 moved cells keeps its octree on the GPU: their leaf
    programs become `PROG_FULL`, and they join the unit's always list in a slack area
    of 2¹⁶ words reserved at the end of the pool. A unit with more moved cells has its
    octree switched off (`grid_root = NONE`), and the GPU falls back to the BVH there.
  - `Draft::widen_skip` makes the skip of the visibility shown safe for the draft:
    nothing is skipped in, or under, a unit that moved.
- **The viewer** offers a surface's coefficients as scrub fields in the inspector.
  - A worker thread owns the draft and runs the latest step asked for.
  - The GPU backend's trace thread writes the latest patch between jobs, so no job
    mixes two models.
  - When no newer step waits, a snapshot of the draft goes to the slice view (and to
    the CPU backend).
  - On release, the card is written (`Source::surface_with_coeffs`) and compiled
    through the existing edit path. The preview stays until the new model lands.
  - Esc, a drag back to where it began, or a failed edit write the revert patch.
- **The edit itself is unchanged.** The model kept is always the one recompiled from the
  text, with the byte identity of decision 0010. A draft is never saved, cached or
  reused.

## Numbers (reference model, release, laptop)

| | |
|---|---|
| `Draft::begin` (model copy, analysis, cells using the surface) | 92–140 ms |
| A step, p95: envelope and filler surfaces, median and most cells | 0.5–13 ms |
| A step whose roi cascade bounds 6 330 filler cells again | 36 ms |
| Snapshot for the CPU views | 46–68 ms |
| CPU slice, 1000 rows: model / draft with the level-0 octree off | 97 / 213 ms |
| GPU patch (written and waited for) | 7–28 ms |
| GPU 1080p frames, draft vs model: level-0 octree kept | 1.01× |
| The same, the 6 350-cell filler surface (its moved cells' node boxes unrestricted) | 1.23–1.41× |

Before this design, with node boxes bounded and the octree switched off, a level-0 step
took up to 325 ms, and frames ran up to 10× the model's. Those numbers are why node
boxes are left unrestricted and level-0 octrees are kept on the GPU.

## Consequences

- A draft is exact. Its classify labels are those of brute force on a fresh compile of
  the edited input, its traces agree with that compile, and its cell boxes contain
  that compile's (`crates/llevant/tests/draft.rs`). A patched upload renders as a fresh
  compile does (`crates/llevant-gpu/tests/draft.rs`).
- The GPU patch (7–28 ms) misses the 5 ms gate the plan set. It runs on the trace
  thread between jobs, not on the UI thread, and patches are cumulative, so a slow one
  only delays the next frame of the preview.
- The upload reserves 256 KB of slack, whether or not a drag ever happens.
- What is not draggable falls back to typing the card and Apply. While a drag runs, the
  3D backend cannot be switched.
- Drafts move one surface at a time. A draft for a `TR` card was built and dropped
  (2026-11-08, decision 0012 step 6): one `TR` can move thousands of surfaces in
  universe 0, and keeping the octree, its programs per leaf and the hidden-space skip
  valid in place cost 0.25–2.6 s a step. Without them, the reference model's
  overview was 50× slower while dragging.
