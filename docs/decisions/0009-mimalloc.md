# 0009: The binaries and the Python module allocate with mimalloc

2026-11-01. Accepted.

## Context

Large parallel batches stopped scaling with threads on Windows. `core.i`, 200 000
lines of 100 cm (37.6 M segments), 16 threads: 2.4 M segments/s, against 10.4 M at
20 000 lines. One thread stayed linear. Timing the phases showed the system heap: just
freeing the per-chunk buffers took 2.3 s, and `Model::volumes` on the same model grew
faster than its line count (×7, then ×10 for 4× the lines).

The same code with mimalloc as the global allocator (release, laptop, 16 threads):

| Workload | System heap | mimalloc |
|---|---|---|
| `core.i` trace, 200 000 × 100 cm | 16.2 s | 4.0 s |
| `core.i` volume tracing, h = 2 cm | 42.5 s | 5.7 s |
| Reference model, 2 M points | 1.16 s | 0.73 s |
| Reference model trace, 200 000 × 100 cm | 2.29 s | 2.16 s |
| `llevant-bench view`, CPU slices and first hits | | 5–30 % faster, every view |
| Viewer orbit (GPU) | 16.6 ms median | 16.7 ms median (vsync) |

Peak memory goes up 4–6 % on the reference model (mimalloc keeps freed pages).

## Decision

`llevant`, `llevant-bench`, `llevant-viewer` and the Python module (`_llevant`) set
`mimalloc::MiMalloc` as their `#[global_allocator]`. The library crates do not: an
application that embeds them chooses its own allocator.

## Consequences

- mimalloc reuses memory, so a fresh `vec![0; n]` is cleared in full; the Windows heap
  instead maps large zeroed blocks lazily. Code that relied on that has to reuse its
  buffers: each `Tracer` takes the last one's surface slots on its thread (a fresh
  array per small task doubled the slice times before that).
- mimalloc is C code built by `cc`: the wheels and binaries need a C compiler, which
  the manylinux images and MSVC have.
- The Python module builds mimalloc with `local_dynamic_tls` (2026-11-12). By default
  mimalloc's thread-locals are initial-exec, which marks `_llevant.abi3.so`
  `STATIC_TLS`: a module loaded with `dlopen` then has to fit in glibc's static TLS
  surplus (512 bytes by default), and `import llevant` failed on Linux after numpy and
  others ("cannot allocate memory in static TLS block"). Raising the surplus with
  `GLIBC_TUNABLES=glibc.rtld.optional_static_tls=…` is not a way round it: glibc
  carves static TLS out of every thread's stack, so 64 MB of it crashed the 2 MB
  rayon threads at the first batch. Local-dynamic TLS costs nothing measurable
  (Python, reference model: 200 000 lines of 100 cm 0.12–0.16 s and 2 M points
  0.07 s, before and after). `test_no_static_tls` (`crates/llevant-py/tests`) reads
  the module's ELF flags. The binaries keep initial-exec (a `cargo build --workspace`
  unifies the feature into them too, harmlessly).
