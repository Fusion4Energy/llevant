# Contributing to llevant

## Setup

```sh
git clone https://github.com/AlvaroCubi/llevant
cd llevant && cargo test --release
```

`release` is the quick optimised profile for tests and benchmarks; `--profile dist`
(fat LTO) is what is shipped and what published numbers are measured on. Large models
(the reference model of the benchmarks is one) are kept out of git, in `untracked/`.

## Commands

Tests and lints:

```sh
cargo test --release                  # engine, scene, render, CLI: unit, MCNP golden, property, CLI tests
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all
cargo test --release -p llevant-gpu -p llevant-viewer   # GPU renderer and viewer (skip without
                                      # a GPU adapter; Mesa's lavapipe is enough, and slow)
LLEVANT_BIG_MODEL=<model> cargo test --release -- --ignored   # checks on a large model
PROPTEST_RNG_SEED=20261031 cargo test --release   # property tests with CI's seed
                                      # (without it, every run draws new cases)
cd crates/llevant-py && maturin develop --release && pytest tests   # Python (a venv with numpy)
```

Checks and benchmarks on any model (`llevant-bench`; `cargo run --release -p llevant-bench`
lists every command):

```sh
cargo run --release -p llevant-bench -- verify <model> 50000          # BVH vs brute force
cargo run --release -p llevant-bench -- compare-trace <model> <lines.f64>   # octrees on vs off
cargo run --release -p llevant-bench -- points <model> 4000000
cargo run --release -p llevant-bench -- trace <model> 200000 100
cargo run --release -p llevant-bench -- view <model> 1000 [out_dir]   # slices and views (+ PPM)
cargo run --release -p llevant-bench -- gpu-check <model> 150         # GPU vs CPU, ray by ray
cargo run --release -p llevant-bench -- work untracked/synthetic --check tools/bench/work-baseline.json
cargo build --profile dist -p llevant-bench && target/dist/llevant-bench report <models...> --json r.json --markdown r.md
                                      # the numbers of docs/benchmarks.md
python3 tools/synthetic/gen_models.py untracked/synthetic   # benchmark models of other shapes
python3 tools/synthetic/gen_fields.py <model> untracked/fields   # mock results for --field
```

Environment variables for A/B runs: `LLEVANT_BENCH_SET=<dir>` saves and replays a
workload, `LLEVANT_GRID=off|auto|always` and `LLEVANT_RESIDUALS=off` change the search
structures, `LLEVANT_CACHE=off` compiles instead of reading the cache,
`LLEVANT_UNIVERSE=<u>` takes one universe on its own. Profiles:
`cargo build --profile bench -p llevant-bench`, then
`RAYON_NUM_THREADS=1 LLEVANT_PROFILE=<dir> target/bench/llevant-bench trace <model> 20000 100`
writes flamegraphs.

The command line and the viewer:

```sh
cargo run --release -p llevant-cli -- <command> <model> ...   # `llevant`; --help on each
cargo run --release -p llevant-viewer -- <model>
LLEVANT_VIEWER_TEST_PNG=out.png cargo test --release -p llevant-viewer --test app
                                      # the whole app driven headlessly, final window as a PNG
target/release/llevant-viewer <model> --storm   # the UI thread's work per frame through edits
```

Against MCNP itself (needs `mcnp6`; a large model takes MCNP tens of minutes to set up):

```sh
cargo run --release -p mcnp-oracle -- check <model> --points 60000
cargo run --release -p mcnp-oracle -- submodel big.mcnp sub.mcnp --keep 30
```

## Changes

- Branch from `main`; one pull request per change, small enough to review in one sitting.
  `main` is always releasable: CI must be green before merging.
- Commit messages: a summary line in the imperative, under 72 characters, naming what
  changes for a user or a caller (`Trace lines in windows of doubling length`, not
  `Improvements`); a body when the reason is not obvious, with measurements when the
  change is about speed.
- Before pushing: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo test --release`. Changes to the GPU renderer or the viewer also run
  `cargo test --release -p llevant-gpu -p llevant-viewer`.
- Add an entry to [`CHANGELOG.md`](CHANGELOG.md) under the top section ("Unreleased", or
  the version being prepared) for anything a user of the command, the library, Python or
  the viewer would notice.

## What a change must keep

- **MCNP is the reference for semantics.** When unsure what MCNP does, probe it with a tiny
  input or extend the models in `tools/mcnp-oracle/models` (and regenerate their `.golden`
  tables with `mcnp-oracle check <model> --sample cells --golden <file>`).
- **The fast path gives exactly the brute-force labels** for points; lines agree with brute
  force as `reference::traces_agree` says. Bounding boxes stay conservative. What each
  query guarantees, and the test that holds it: `docs/contracts.md`.
- **Results do not depend on the number of threads.**
- Property tests run with a fixed seed in CI and with random seeds nightly. When the nightly
  run finds a case, commit the line it adds to the `*.proptest-regressions` file with the fix.
- Speed changes come with an A/B on a saved workload (`LLEVANT_BENCH_SET`), old build first.
- The nightly job also compares the engine's work counters on the synthetic models with
  `tools/bench/work-baseline.json`. A change meant to move them (a better octree, say) writes
  the baseline again in the same commit:
  `llevant-bench work untracked/synthetic --write tools/bench/work-baseline.json`, after
  `python tools/synthetic/gen_models.py untracked/synthetic`.

## Releases

A tag `vX.Y.Z` on `main` runs `.github/workflows/release.yml`: the binaries, the wheels
and the sdist are built, then the crates go to crates.io, the wheels and the sdist to
PyPI, and everything to a GitHub release. Versions are never published twice, so check
before tagging:

1. `version` in `[workspace.package]` (`Cargo.toml`) is X.Y.Z, and so are the workspace's
   own crates in `[workspace.dependencies]`; the Python package takes the same version.
2. `CHANGELOG.md`: the "Unreleased" section becomes `## X.Y.Z - YYYY-MM-DD`. The workflow
   refuses a tag without that dated section, or that differs from the version.
3. CI is green on the commit, the Release workflow run by hand on it (Actions, "Release",
   "Run workflow": it builds and tests everything and publishes nothing) is green, and
   these pass locally (nothing is uploaded):
   ```sh
   cargo publish --workspace --dry-run --exclude llevant-py --exclude llevant-bench --exclude mcnp-oracle
   cd crates/llevant-py && maturin build --release && maturin sdist   # then `twine check` them
   ```
4. `git tag vX.Y.Z && git push origin vX.Y.Z`.

The parser (migjorn and migjorn-syntax) is pinned exactly in `Cargo.toml`: what it
parses is part of the engine build that keys the cache of compiled models, so a new
migjorn is taken by raising both pins, and reaches users with a new llevant release.

Set up once, in the repository settings and on the two registries:

- a GitHub environment `release`; required reviewers on it make the crates.io and PyPI
  jobs wait for an approval after the builds;
- in that environment, the secret `CARGO_REGISTRY_TOKEN`: a crates.io API token with the
  `publish-new` and `publish-update` scopes (once the crates exist, it can be narrowed to
  `llevant*` or replaced by crates.io trusted publishing);
- on PyPI, a trusted publisher for the project `llevant` (a "pending publisher" before
  the first release): owner `AlvaroCubi`, repository `llevant`, workflow `release.yml`,
  environment `release`. No PyPI token is stored.

If the crates job fails part-way, the crates already uploaded stay published: publish the
remaining ones by hand in dependency order (`cargo publish -p <crate>`; llevant,
llevant-scene, llevant-render, llevant-gpu, llevant-viewer, llevant-cli).
