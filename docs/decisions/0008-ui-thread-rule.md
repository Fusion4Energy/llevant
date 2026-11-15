# 0008: The viewer's UI thread does no model-sized work

2026-10-31. Accepted.

## Context

The viewer stayed smooth while orbiting, but edits and panels froze it. On the
reference model (425 k cells):

| Work done on the UI thread | Time |
|---|---|
| The Statistics panel's frame | 104 ms |
| A hide or show (`Model::skip`, visibility, packing) | ~50 ms |
| A looks change | 22 ms |
| Opening a document | 110–170 ms |
| A hover pick | 14–21 ms |

## Decision

The UI thread only reads state, swaps `Arc`s and buffers, and lays out what is on
screen. Anything that scales with cells, instances, voxels or pixels runs
elsewhere:

- **Scene edits** go to a deriver thread per document (`derive.rs`). Requests that
  queue up meanwhile are merged.
- **Loading, reading files, the analyses and per-model panel numbers** run as
  `tasks::Task`s: a `Progress` to show and cancel, values polled once a frame, and
  dropping a task cancels it.
- **Hover picks** run on a latest-wins worker (`tasks::Latest`).
- **The 3D view's pipelines and palette** are made on the loading thread
  (`view3d::Prepared3d`).
- **The slice composes its image** on a thread pool of the UI's own.
- **GPU jobs** are prepared on the backend's thread.

The views keep showing what they had until each result lands.

## Enforcement

- `llevant-viewer <model> --storm` drives panels, hide and show, isolate, levels,
  colouring, selection and palette typing, and reports the UI thread's own work
  per frame.
- `RUST_LOG=llevant_viewer::frames=debug` logs every frame over 8 ms, with its
  commands and where the time went.

## Evidence

On the reference model, the storm's worst frame went from 104 ms to 7.6–13.9 ms.
Orbiting stays at 16.6 ms median.
