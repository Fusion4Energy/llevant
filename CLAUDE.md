# llevant

Fast MCNP geometry engine in Rust. Read `docs/architecture.md` for the crates, the rules
between them and the data flow; `docs/internals/engine.md` and `docs/internals/rendering.md`
for details; `docs/decisions/` for why things are as they are (with the numbers);
`docs/mcnp-behaviour.md` for what MCNP does; `docs/benchmarks.md` for current numbers
(`llevant-bench report`, dist profile). `docs/CONTRIBUTING.md` has the commands and the release
steps. The exact user-facing references, kept in step with the code: `docs/guide/cli.md`
("Scripting and agents": JSON shapes, exit codes), `docs/reference/viewer.md` (viewer, view
file), `docs/reference/supported-input.md` (input read and not read) and the stubs
`crates/llevant-py/python/llevant/_llevant.pyi` (the site's Python API page is generated from them).

## Commands

The commands (tests, lints, benchmarks, checks, the viewer, Python, the MCNP oracle) are in
`docs/CONTRIBUTING.md`, "Commands"; releases in its "Releases". Mesh geometry and the
meshtal/MCTAL readers come from aci-tallies (crates.io); the tally files the tests read are
copies of its fixtures in `tests/fixtures/tallies`. CI: `.github/workflows` (fixed proptest
seed; `nightly.yml` random seeds and the work-counter baseline). Machine-specific
commands and paths go in `CLAUDE.local.md` (not in git). Here:

```sh
LLEVANT_BIG_MODEL=<model> cargo test --release -- --ignored
cargo run --release -p llevant-bench -- points <model> 4000000
target/release/llevant-viewer <model> --orbit 10 --screenshot out.png
                                # frame times while orbiting, then a PNG of the window
                                # (RUST_LOG=llevant_viewer=debug: batches and load steps;
                                # RUST_LOG=llevant_viewer::frames=debug with --storm: slow frames)
cargo run --release -p llevant-bench -- gpu-ray <model> 150 <view> <ray> [vis]   # one ray: CPU segments, GPU steps
cargo run --release -p llevant-viewer --example latency -- <model>   # GPU time vs image size
python crates/llevant-py/examples/tour.py [model] [out_dir]   # API tour (also a `# %%` notebook)
```

CLI options worth knowing: `slice/render --field f.csv|run.msht|mctal [--tally N --bin m=1
--log --scale lo,hi]`, `--universe U [--cut own|none|box]`, `--level N`, `render --ghost
"c 300"`, `bbox --cells "c 10-20"` with `render --isolate`, rule options taking `@file`,
rules by `$` label (`--isolate '$ HNB - BSV & m 69'`), `info --format json`.

## Rules of thumb

- MCNP itself is the reference for semantics. When in doubt, probe MCNP with a tiny
  input (MCNP 6.2 as `mcnp6`; lines must fit in 128 columns) or extend the zoo models in `tools/mcnp-oracle/models`.
- After changing a zoo generator, regenerate the `.i` and its `.golden` with
  `mcnp-oracle check <model> --sample cells --golden <file>`.
- The fast path must give exactly the brute-force labels (`llevant::reference`) for
  points, and for lines agree with brute force as `reference::traces_agree` says
  (on-surface slivers up to 1e-6 cm and which of surfaces within the tolerance is named
  at an end may differ; `docs/internals/engine.md`, "Deliberate semantics"); boxes must stay
  conservative (outward padding, LP failures keep the old box). Every guarantee and the
  test that holds it: `docs/contracts.md`.
- The engine (`crates/llevant`) stays geometry only and its `Model` read-only: images,
  material looks and UI state live in `llevant-render` and the frontends
  (`docs/architecture.md`, "Rules").
- MCNP needs ~15 min to set up a 70 k-cell sub-model and 82 min for the full reference
  model: run oracle checks on big models in the background.
- `untracked/` holds the large models (not in git).
