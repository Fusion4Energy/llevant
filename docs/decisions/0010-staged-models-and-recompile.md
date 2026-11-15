# 0010: Models are usable before their octrees, and an edit recompiles against the last version

2026-11-01. Accepted.

## Context

A cold compile of the reference model takes ~40 s (dist): parse and assemble 1.3 s,
cell boxes 8.3 s, BVHs 0.04 s, octrees 30–40 s. Every change to the input paid all of
it again, and the viewer's reload rebuilt everything (document, undo, selection, GPU
upload).

The octrees are an accelerator: every query is exact through the BVHs alone. Measured
on the reference model (dist, 16 threads, laptop):

| | Without octrees | With octrees |
|---|---|---|
| Compile | 8.7 s | 39.5 s |
| GPU frame, 1920×1080 | 452 ms | 279 ms |
| CPU slice, 1000 rows | 263 ms | 88 ms |
| Points | 7.8·10⁵/s | 6.6·10⁶/s |
| CPU render | 27.9 s | 2.0 s |
| `check` | 393 s | 89 s |

The GPU stays usable without octrees (frames are sliced and previewed, so the view stays
responsive), so a model can be shown before they are built.

## Decision

- **Staged models.** `Model::compile_staged` stops after the BVHs and leaves each
  unit's octree pending; `Model::complete` (or `completed`) builds them; `compile` is
  the two in turn. A staged model answers every query exactly, only more slowly, and
  cannot be saved. Completed, it saves the same bytes as `compile`.
- **Node-box ids local to each cell** (`Cell::node_start`), and octrees seeded by
  universe number: what a unit's boxes and octree are depends on that unit alone.
- **`Model::recompile(old, input)`** runs the front in full (parse, surfaces, cells,
  assembly) and lets each unit take its cell boxes, node boxes and octree from the
  unit of the same universe in `old` when its content is the same: same region, same
  cells by number in the same order, programs equal up to shape indices, shapes equal
  bit for bit, same flags and options. Indices that moved (cells or surfaces added or
  removed before it) are remapped: the octree's candidates and its residual
  programs' leaves are rewritten. Other units are computed again and left pending.
  Reuse is decided on compiled content, so `LIKE BUT`, `#n`, `TR` chains and comments
  need nothing of their own, and migjorn needs no change.
- **The contract is byte identity**: a recompiled model, completed, saves the same
  bytes as a fresh compile of the new input (`tests/recompile.rs`: every kind of edit
  on the zoo models and a proptest, from complete and staged bases, back and forth).
  A reuse bug cannot hide behind a test that only samples queries.
- **The cache recompiles against the last version.** `Store` keeps the last entry of
  each input path (`<cache>/last/`); on a miss it reads it (~1 s) and recompiles
  against it. `Store::load` stays complete (CLI, Python); `Store::load_staged` returns
  a staged model and `Store::complete` finishes and stores it (the viewer).
- **The viewer** shows a staged model at once and swaps in the completed one when its
  octrees land; a reload recompiles against the model shown and replaces it in place
  (history, selection, tools' state kept), completing first when the rebuilt universes
  hold at most a quarter of the cells.

## Numbers (reference model, dist, laptop)

| | Before | Now |
|---|---|---|
| Viewer, cold: first view | ~45 s | 14.9 s (complete at ~52 s) |
| Envelope surface moved: staged / complete | 39.5 s | 3.3 s / 5.1 s |
| Filler surface moved: staged / complete | 39.5 s | 2.5 s / 3.5 s |
| CLI after an edit (cache recompiles) | 42.8 s | 7.4 s |
| Level-0 cell removed (every index after it moves): staged / complete | 10.9 s / 44.4 s (nothing reused) | 4.3 s / 5.8 s (release) |

## Consequences

- `EXPORT_VERSION` 2 (node boxes per cell, `CELL_WORDS` 13) and saved format 3.
- The first GPU upload of a new layout compiled a shader variant in the driver (~9 s
  staged, ~30 s complete, once per driver cache). Since 2026-11-02 the shader depends
  only on the depth of programs and BVHs: staged, complete and most edits share it.
- While octrees build, the viewer's previews were slower (the CPU is shared): 95th
  percentile frames of 33 ms. The viewer builds them on a pool of lower-priority
  threads since 2026-11-02: 19 ms.
- After a reload the viewer checks where the edit can have opened a gap or an
  overlap: the instances of the changed cells, old and new (`Model::instance_boxes`,
  `Model::volumes_within`): 6 s for an envelope surface of the reference model.
- The viewer edits cards in the inspector: migjorn replaces the card's text
  (`llevant_scene::source::edit_card`), then `recompile` against the model shown, the
  cards read again alongside. On the reference model (377 MB of input) migjorn added
  ~2 s to an edit (parse and write) beside the recompile; undo keeps only the bytes
  each edit replaced. Since 2026-11-02 the viewer keeps migjorn's tree, edits it in
  place and compiles it (`Model::recompile_parsed`): no parse per edit.
- Open: on the reference model, the upload of the completed model can wait while
  the views keep the GPU busy (orbiting without a pause, it did not land within 40 s);
  it lands once the view rests.

## Left out

- **Local octree patches:** they save at most 1–4 s per edit, make the model differ
  from a fresh compile, and need their own proof.
- **A cache per universe:** recompiling against the last entry of the same path gets
  the same reuse with no new format or eviction policy.
- **Reusing BVHs:** 0.04 s to build.
- **Stack depths rounded up** so that edits deepening a program keep the shader:
  6–7 % slower frames.
