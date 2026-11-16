#!/usr/bin/env bash
# Raise the version to X.Y.Z in one commit: the workspace's in Cargo.toml, the pins
# of its own crates in [workspace.dependencies] (the Python package takes the same
# version) and Cargo.lock.
#
#   tools/release.sh 0.2.0
#
# Pushing that commit to main releases it: the Release workflow sees a version with
# no tag yet, builds, waits for an approval, publishes and tags vX.Y.Z
# (docs/CONTRIBUTING.md, "Releases").
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

version="${1:-}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] ||
  { echo "usage: tools/release.sh X.Y.Z" >&2; exit 2; }

[ -z "$(git status --porcelain)" ] ||
  { echo "the working tree has changes: commit or stash them first" >&2; exit 1; }
branch=$(git rev-parse --abbrev-ref HEAD)
[ "$branch" = main ] || echo "note: on branch $branch; releases are made from main" >&2

git fetch --quiet --tags origin 2>/dev/null || true
! git rev-parse -q --verify "refs/tags/v$version" >/dev/null ||
  { echo "v$version is already released: versions are never published twice" >&2; exit 1; }

old=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
[ -n "$old" ] || { echo "no version in [workspace.package] of Cargo.toml" >&2; exit 1; }

# The workspace version, and the workspace's own crates in [workspace.dependencies]
# (`llevant = { version = "…", path = "crates/llevant" }` and its companions).
sed -i.bak \
  -e "/^\[workspace.package\]/,/^\[/s/^version = \".*\"/version = \"$version\"/" \
  -e "s/^\(llevant[a-z-]* = { version = \"\)[^\"]*\(\", path = \"crates\/\)/\1$version\2/" \
  Cargo.toml
rm -f Cargo.toml.bak
grep -q "^version = \"$version\"" Cargo.toml ||
  { echo "failed to set the workspace version" >&2; exit 1; }
stale=$(grep -E '^llevant[a-z-]* = \{ version = "[^"]*", path = "crates/' Cargo.toml |
  grep -v "version = \"$version\"" || true)
[ -z "$stale" ] || { echo "pins not updated:"$'\n'"$stale" >&2; exit 1; }

# Only the workspace's own entries in Cargo.lock change.
cargo update --workspace --quiet

git commit --quiet -am "Release $version"
echo "Version $old -> $version committed in $(git rev-parse --short HEAD)."
echo "Pushing it to main releases it (after an approval on the 'release' environment)."
