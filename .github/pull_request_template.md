## What and why

<!-- What changes for a user or a caller, and why. -->

## Checks

- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --release`
- [ ] GPU or viewer changes: `cargo test --release -p llevant-gpu -p llevant-viewer`
- [ ] Engine changes: fast path still equals brute force (`verify`, `compare_trace` on the reference model)
- [ ] Speed changes: A/B on a saved workload (`LLEVANT_BENCH_SET`), numbers in the description
- [ ] `CHANGELOG.md` updated for anything users notice
