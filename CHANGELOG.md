# Changelog

Notable changes to llevant (the library, the `llevant` command, the Python module and the
viewer). The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/).

## 0.1.0 - Unreleased

First public release.

- **Engine** (`llevant`): compiles MCNP inputs (every universe level, lattice,
  transform and macrobody) into a read-only model, and answers batches of exact
  queries on it: point classification, line tracing, first hits of rays, slices and
  camera views, fingerprints of cells and universes, cell instances, volumes with an
  honest uncertainty, and the gaps and overlaps of a model. Compiled models are saved
  and read back, and an edited input recompiles against the previous model.
- **`llevant` command**: `info`, `bbox`, `locate`, `trace`, `slice`, `render`,
  `materials`, `fingerprint`, `volume`, `check` and `view`, with text, CSV and JSON
  output (one JSON shape for labels, surfaces and boxes across commands, model diffs
  included; exit status 1 when the answer is "no", 2 on errors, which JSON output also
  reports as JSON), `locate --points` and `trace --lines` for batches (from stdin
  with `-`), PNG images, results drawn over the geometry (CSV per cell, meshtal, MCTAL,
  D1SUNED), and a disk cache of compiled models.
- **Viewer** (`llevant-viewer`): a GPU 3D view and an exact slice side by side,
  workspaces for exploring, checking, results and tallies, an inspector that edits
  cards in place, live surface drags, rules, cuts, a ruler and FMESH layout.
- **Python** (`pip install llevant`): models kept in memory, numpy batch queries,
  images inline in Jupyter, and `python -m llevant` for the command and the viewer.
