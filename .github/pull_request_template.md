<!-- Pull requests come from the maintainers, or from someone a maintainer invited on an
issue labelled `pr-welcome`; others are closed automatically. To propose a change, open
an issue instead (docs/CONTRIBUTING.md, "How to contribute"). -->

Issue: #

## What and why

<!-- What changes for a user or a caller, and why. -->

## Checks

- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --release`
- [ ] GPU or viewer changes: `cargo test --release -p llevant-gpu -p llevant-viewer`
- [ ] Engine changes: fast path still equals brute force (`verify`, `compare_trace` on the reference model)
- [ ] Speed changes: A/B on a saved workload (`LLEVANT_BENCH_SET`), numbers in the description
