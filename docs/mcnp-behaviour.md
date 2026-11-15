# MCNP behaviour

What MCNP 6.2 (D1SUNED 4.1.2 build) does where the manual is silent or easy to
misread, found while checking llevant against it. llevant follows MCNP on each of
these. MCNP is the reference for semantics (`CONTRIBUTING.md`); to settle a new
question, probe it with a tiny input, or extend the zoo models in
`tools/mcnp-oracle/models` and check them with `mcnp-oracle`.

## How llevant is checked against MCNP

`mcnp-oracle check` classifies random points with llevant, then has MCNP report
the cell of each one. The points go in as an `SDEF` source list (`POS=D1`,
`SI1 L`, the point's index encoded in its energy), and MCNP's answers are read
from the source events in PTRAC. Only the deepest cell is compared: the `VOID`
card makes MCNP report material 0. Points MCNP confirms become the golden tables
that `cargo test` replays (`crates/llevant/tests/mcnp_golden.rs`).

| Model | Points | Disagree |
|---|---|---|
| `surfaces.i`: every surface type, every facet of every macrobody, the TR forms | 98 192 | 0 |
| `universes.i`: fill transforms, `*FILL`, `m=-1`, nesting, TRCL in every form, `LIKE BUT TRCL`, `#n` of moved cells, `LAT=1` and `LAT=2`, fill arrays | 99 999 | 0 |
| `periodic.i`: periodic surfaces | 20 000 | 0 |
| A sub-model of the reference model: every level-0 cell, 30 random envelopes filled (68 k cells) | 58 949 | 0 |
| The 41 envelopes that hold the reference model's six `LAT=1` cells, plus 10 random ones (70 k cells) | 70 000 | 0 |
| The same 41 envelopes, points only inside lattice elements | 20 000 | 0 |
| `assembled.i` (17 k cells), points sampled per cell | 60 000 | 0. The 59 points in the `IMP:N=0` graveyard, where MCNP starts no particle, are counted apart |

The comparison found two llevant bugs: the REC lateral surface was scaled by
`|a|²` instead of `|a|⁴`, and WED facets 2 and 3 were swapped.

## Geometry

- **TRCL and a FILL transform on the same cell.** The fill transform *replaces*
  the TRCL for the filling universe; it does not compose with it. This was probed
  with translations and with a rotating `*TRCL`. Without a fill transform, the
  universe moves with the cell.
- **WED facets.** Facet 1 is the slanted face, 2 the face containing `a2`, 3 the
  face containing `a1`, 4 the top and 5 the bottom.
- **Rejected inputs.** MCNP rejects:
  - one-sheet cones and tori whose transform is skewed (axis not parallel to x, y
    or z);
  - RHP, REC and WED whose vectors are not orthogonal to the height.

  llevant reports such cells as unsupported, with that reason.
- **`LIKE n BUT ... U=0`.** Applied to a cell from a universe that is used as a
  fill, it can fail with a misleading "too many entries on fill card". The zoo
  copies a level-0 cell instead.
- **Tally bins in repeated-structure notation** may leave out the levels in
  between: `(8 < 3)` scores cell 8 across lattice 7. A lattice cell given without
  an element means all of its elements. This is checked against MCNP output with
  the pencil-beam fixture `tests/fixtures/tallies/mctal/probe.mcnp`
  (`crates/llevant-scene/tests/tallies.rs`). MCTAL writes 0 for bins that are
  groups, levels or the T bin.

## The plotter's `LEVEL n`

This was probed with a 13-cell input whose structure has both a chain of fills
(0 → u=1 → u=2) and a lattice (0 → lattice u=3 → u=4). The input is `PROBE` in
`crates/llevant/tests/levels.rs`. It was plotted with
`mcnp6 ip i=lv.i notek com=com` on z = 0 at `level 0`, `1`, `2` and `-1`, and the
labels were read in `plotm.ps`.

- `LEVEL n` draws only the cells at exactly level n, and everything shallower is
  blank. It is not a cap: at `LEVEL 2`, the level-0 cells and the level-1 cell are
  not drawn.
- A filled cell at level n is drawn and labelled as a cell, whatever fills it.
- A lattice cell at its own level is labelled once, and its element boundaries
  are drawn.
- `LEVEL -1` (the default) draws the deepest cell everywhere.

llevant's `--level`, `Visibility::level` and `Depth::Level` give the same picture
(`levels_shown_as_mcnp_plots_them`).

## Running MCNP on big inputs

- **Column limit.** MCNP 6.2 rejects lines beyond column 128. The reference model
  has 84 k such lines (long `$` comments); the oracle drops those comments and
  rewraps the cards.
- **Set-up time.** `VOL NO` skips MCNP's cell-volume calculation. Even so, setting
  up a 68 k-cell sub-model takes about 14 minutes. The full 425 k-cell model took
  82 minutes without `VOL NO`. So the oracle works on sub-models: `mcnp-oracle
  submodel` keeps every level-0 cell, and a chosen subset of envelopes keeps its
  filler.
- **Time cutoff.** With a `CUT` time cutoff of 1e-30, every history is "lost
  (infinite loop)" and nothing reaches PTRAC. Histories must be left to stream
  out, or a realistic cutoff used: `cut:n 100` for the periodic model, where
  particles would otherwise circulate forever.
- **Lost particles.** Particles lost in lattice elements outside a fill array can
  crash MCNP mid-run; the oracle resubmits the points MCNP never reported.
- **Source points in gaps.** A run aborts ("source is not in any cell") if any
  source point is in a gap. So the oracle only sends points that llevant finds
  inside a cell, and splits the batch in two if MCNP still aborts.
- **Stochastic volumes are not pure volumes** when the source sphere does not hold
  the whole model. Particles leaving it keep scoring until they reach an
  `IMP:N=0` cell, and some of those (universe graveyards) lie inside envelopes.

## Not probed yet

- MCNP's numbering of surfaces moved by `TRCL` (1000 · cell + surface?). Labels
  and picks show the card's number.
- Crossings directly (PTRAC surface events of pencil beams). Tracing reaches MCNP
  only through point classification.
