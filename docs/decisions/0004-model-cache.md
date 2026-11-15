# 0004: Compiled models are cached on disk

2026-10-17, format revised 2026-10-31 and 2026-11-01 (sections, memory map). Accepted.

## Context

Compiling the reference model takes about 24 s (boxes 7.6 s, octrees 13.8 s), and
every frontend run paid it. The model is read-only after compiling, so it can be
written out as it is.

## Decision

- **The engine** writes and reads models as bytes: `Model::save` and `Model::read`
  (`saved.rs`). The bytes start with a header: a magic number, `FORMAT_VERSION` and
  `ENGINE_HASH`, a digest of the engine's source, version and `Cargo.lock` made by
  `build.rs` (the packaged lock when built from crates.io; the parser, migjorn, is
  pinned exactly so that the version fixes it). Then sections, each a `u64` length and bitcode: the model without its
  octrees, then each octree's arrays in pieces of 4 M items. Only the build that
  saved a model reads it; anything else (cut short, bytes after the end) is refused
  with `Error::Format`.
- **Saving writes section by section**, each encoded just before it is written, and
  **reading decodes from a slice** (`Model::from_bytes`): the cache maps the entry
  into memory (`memmap2`) instead of reading it into a buffer. Sound because entries
  are never changed in place (written to a temporary file and renamed).
- **The cache policy belongs to the frontends** (`llevant_scene::store::Store`),
  not to the engine:
  - an entry's key is a BLAKE3 hash of the input's bytes, the `Options` and
    `ENGINE_HASH`;
  - entries are written to a temporary file and renamed;
  - the most recently used entries are kept within 8 GB (`LLEVANT_CACHE_GB`);
  - entries live in `LLEVANT_CACHE_DIR` or the user's cache directory;
  - `LLEVANT_CACHE=off` turns the cache off, and an unreadable cache is ignored.

## Evidence

- Reference model: a 1.2–1.6 GB entry, read in 1.5–2.2 s instead of a 24 s compile.
- `llevant slice` on it: 25.6 s → 2.9 s wall time.
- Encodings tried for reading the reference model: bincode 1.3 s (unmaintained),
  postcard 2.1 s, bitcode about 1.5 s (kept).

- Memory, reference model (`llevant info`, peak commit / working set from the OS):

  | | one bitcode payload (before) | sections + memory map |
  |---|---|---|
  | compile, no cache | 5.11 / 4.39 GB | 5.30 / 4.52 GB |
  | compile + store | 9.14 / 7.92 GB | 5.42 / 4.42 GB |
  | cached load | 4.47 / 3.97 GB | 2.33 / 3.28 GB |

  The whole payload was built in memory before writing, and read into a buffer that
  lived while it was decoded; the input (378 MB) was kept to the end. A cached load's
  working set includes the mapped file's pages, which the OS can drop. Entry 1.58 →
  1.47 GB; decoding 1.2 → 1.4 s (pages fault in from the map).

## Consequences

- No migration code: any engine change gives a new hash, and stale entries simply
  age out.
- `tests/saved.rs` checks that a model read back answers exactly as the compiled
  one.
- The instance numbering is not cached; it is rebuilt on first use, in
  milliseconds.
