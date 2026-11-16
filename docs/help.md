# Help and FAQ

## Does llevant give the same answers as MCNP?

That is its aim, and it is tested: random points in models covering every surface
type, macrobody, transform form, universe and lattice kind were located by both
llevant and MCNP 6.2, with no disagreement
([how it is checked](mcnp-behaviour.md)). Where MCNP behaves in a surprising way,
llevant does the same, and [MCNP behaviour](mcnp-behaviour.md) lists those cases.

If you find a point where llevant and MCNP disagree, please
[report it](#reporting-a-problem): it is a bug.

## My model looks wrong or incomplete

Run `llevant info model.i` and read the **diagnostics** at the end. They list, with
their line numbers, anything llevant could not read as MCNP would. Two cases come up
most:

- **Cell parameters on data cards** (`IMP:N 1 1 1 0` as a data card, rather than
  `imp:n=1` on each cell): llevant reads `IMP`, `U`, `FILL`, `TRCL` and `LAT` only
  from the cell cards.
- **`READ` cards**: the files they name are not read. Assemble the input into one
  file.

See [Supported input](reference/supported-input.md) for the full list.

## The viewer only shows a big box

That is a cell around everything, often air. Click it and press **H** to hide it.
Void cells are hidden from the start; cells with a material are not.

## A slice is grey, or has grey lines

The slice lies exactly on a surface: points on a surface are in no cell, and are
drawn grey. Move the slice a little off round numbers (`z=0.5` rather than `z=0`).

## The first time a model opens is slow

llevant compiles the model the first time, building search structures that make every
later question fast: about 36 s for a 425 000-cell model, under a second for small
ones. The result is cached, and the model then opens in about a second, until the
file changes.

## The model cache

Compiled models are kept in:

| System | Folder |
|---|---|
| Windows | `%LOCALAPPDATA%\llevant\cache` |
| Linux | `$XDG_CACHE_HOME/llevant`, or `~/.cache/llevant` |

The oldest entries are removed beyond 8 GB. It is safe to delete the folder at any
time. Environment variables change it:

| Variable | Effect |
|---|---|
| `LLEVANT_CACHE_DIR=<folder>` | keep the cache there |
| `LLEVANT_CACHE_GB=<n>` | keep at most n GB |
| `LLEVANT_CACHE=off` | no cache: compile every time |

## How much memory do I need?

Small and medium models need little. The 425 000-cell reference model (378 MB of
input) takes about 5 GB while it is compiled the first time, and 2–4 GB when it is
read back from the cache.

## The viewer is slow, or does not start

- The 3D view uses the graphics card. Check that its drivers are installed (Vulkan or
  DirectX 12 on Windows, Vulkan on Linux).
- Start it with `--backend cpu` to draw on the processor instead: slower, but it
  works on any machine.
- On a remote desktop or a virtual machine without a graphics card, `--backend cpu`
  is the way.

## The volumes do not match MCNP's

MCNP computes volumes analytically only for some cell shapes, and reports "not calculated"
for the rest. llevant measures every cell by tracing lines through it, so each volume
comes with an uncertainty (`sigma`): about 95 % of the time the exact volume is
within 2 sigma. To make it smaller, use a smaller `--h`, or `--rel-sigma 1e-3` to
refine every cell to 0.1 %.

## Can I use it from my own program?

- From **Python**: `import llevant` ([guide](guide/python.md),
  [API reference](reference/python-api.md)).
- From **Rust**: the [`llevant`](https://docs.rs/llevant) crate and its companions on
  docs.rs.
- From **any language**: the command line's JSON output
  ([scripting](guide/cli.md#scripting-and-agents)). Exit status 0 means done, 1 means
  done and the answer is "no" (gaps found, models differ…), 2 means an error.

## Reporting a problem

Open an issue on [GitHub](https://github.com/Fusion4Energy/llevant/issues) with:

- what you ran and what you expected;
- the output of `llevant --version` and `llevant info model.i`;
- if you can, a small input that shows the problem. Models are often confidential:
  a few cells cut out of the model are usually enough.
