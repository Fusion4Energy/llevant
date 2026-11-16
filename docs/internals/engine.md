# The engine

`crates/llevant`: pure Rust, no GPU, no images. It compiles an MCNP input into a
read-only model and answers geometry questions about it: where points are, what lines
cross, where rays first meet visible cells, fingerprints, volumes. The overview and
the other crates are in [`architecture.md`](../architecture.md); numbers in
[`benchmarks.md`](../benchmarks.md), the decisions behind the design in
[`decisions/`](../decisions/README.md).

```
            MCNP input
                │  migjorn (parse, lossless)
                ▼
     ┌────────────────────────┐
     │ compile                │  LIKE BUT, TR, TRCL, macrobody facets, FILL, lattices
     └──────────┬─────────────┘
                ▼
     ┌────────────────────────┐
     │ Model                  │  read-only, shared by all threads: flat arrays of
     │                        │  shapes, cell programs, units (one per universe),
     │                        │  BVHs, octrees, lattices
     └──────────┬─────────────┘
       ┌────────┼──────────────┬──────────────────┬────────────────┐
       ▼        ▼              ▼                  ▼                ▼
   classify   trace         first_hits       fingerprints      volumes
   (points)   (lines →      (rays →          (per universe,    (grids of lines)
              segments)     hit + normal)    per cell)
```

Everything is stored as flat arrays indexed by integers (no pointers, no per-object
allocations): compact at 10⁶ surfaces, cache-friendly, and exportable as it is to the
GPU renderer ([`export`](https://github.com/AlvaroCubi/llevant/blob/main/crates/llevant/src/export.rs)).

## Input

The `migjorn` crate from crates.io (0.3.18), used at the Rust level, with the split
*migjorn owns MCNP syntax and MCNP's transform conventions, llevant owns geometry*.
llevant reads typed views: geometry expressions as trees (`geometry_expr`, facet
references included), `trcl()` and `fill()` (transforms as TR numbers or inline values,
fill arrays with their shortcuts expanded as MCNP reads them), `importance("n")`,
`universe()`, `lattice()`, `material_override()`/`density_override()` for `LIKE BUT`,
and the line of each card for diagnostics. A value migjorn cannot read makes the cell
unsupported, with migjorn's message. Transforms (`TRn` cards, `TRCL`, `FILL`) are built
by migjorn (`Transform::affine`, `TransformSpec::affine`) and converted to llevant's
main-to-local `Affine`. llevant keeps the semantics: the `LIKE BUT` chain and everything
after. Cell parameters given on data cards and `READ` cards are not read; migjorn lists
them (`cell_parameter_cards`) and llevant reports each as a diagnostic. Reading every cell and
surface typed takes a fraction of a second on the reference model.

`Progress` (`progress.rs`) is how a long call reports how far it is and is
stopped, passed as `Option<&Progress>`: `Model::volumes` counts chunks of lines in it and
returns `Error::Cancelled` soon after `Progress::cancel`; `Model::load` names its
steps (reading, parsing, then the six steps of compiling) and stops between them
(`llevant_scene::store::Store::load` adds the cache's).

`llevant_scene::source::Source` keeps the input as written for frontends: the text and line
of each cell, surface and material card by number (the first card of a number, as
the model). It is a parse of its own, not part of `Model`, which may come from the
cache without the input being parsed at all.

## Compiled model

- **Surfaces** (`surface.rs`): planes, quadrics stored about a local origin (to avoid
  cancellation far from the global origin), one-sheet cones (quadric + nappe), tori in
  their own frame. All MCNP surface types and all macrobodies (RPP, BOX, SPH, RCC, TRC,
  RHP/HEX, REC, ELL, WED, ARB) with facets; TR folded in at compile time; periodic
  surfaces treated as ordinary surfaces. Each compiled shape keeps its MCNP number and
  facet (`SurfaceId`), so results are reported in the user's terms.
- **Cells** (`expr.rs`): a flat prefix program of `u32` words: n-ary AND/OR nodes with
  skip lengths (short-circuit), complements pushed to the leaves (De Morgan), `#n`
  inlined, `LIKE BUT` and `TRCL` resolved at compile time (TRCL makes moved copies of
  surfaces, shared between cells). Evaluation uses three-valued logic, so a point
  within the tolerance (1e-8 cm) of a deciding surface is reported as on-surface. A
  cell keeps universe, material, density (with the MCNP sign: negative is g/cm³), fill,
  box and MCNP number; one that uses an unsupported feature is compiled with a reason,
  which only matters where it is evaluated.
- **Fills and lattices** (`model.rs`, `lattice.rs`): `Fill::Universe(Target)` or
  `Fill::Lattice`; `LAT=1` (any parallelepiped, 2-D or 3-D) and `LAT=2` (hexagonal,
  2-D or 3-D, from planes or an RHP), uniform fills or fill arrays (per-element
  transforms, elements filled by the lattice cell itself, elements outside the array
  → gap). One compilation unit per universe, holding its cells and search structures.
- **Labels**: the full path of a point, `[(universe, cell, lattice element), …]` from
  level 0 down, with a status: inside, gap, overlap, on-surface, unsupported, too deep.
  Interned, and referred to by `u32` ids in results.

## Search structures

- **Bounding** (`bbox.rs`, `lp.rs`): the crux of performance. GEOUNED cells are mostly
  general planes and GQ cylinders, so per-axis interval contraction alone left half
  the cells unbounded. Boxes combine interval contraction, eigen-based bounds of
  quadrics in any orientation (with "soft" near-zero eigenvalues, since GEOUNED's
  7-digit GQ cylinders are really ±1e-9 hyperboloids/ellipsoids), and a 3-variable
  Seidel LP over each AND node's planes plus octagonal outer approximations of
  cylinders and one-sheet cones. Universe 0 is bounded by its **problem box**, the
  intersection of the boxes of the complements of its `IMP:N=0` cells (usually the
  graveyard sphere's box, ±102 m on the reference model): every point beyond it is in
  one of them, so only they are checked there (`Model::problem_box`; "Deliberate
  semantics"). Without a bounded graveyard it is a ±10 km world box, and points
  beyond that are checked against every level-0 cell. Each universe's reachable region
  (ROI) is propagated down the fill graph, so filler cells defined by complements still
  get finite boxes. Boxes are conservative: outward padding, and an LP that fails keeps
  the old box.
- **BVH** (`bvh.rs`): one per unit over the cells' boxes, flat array of nodes. Cells
  that cannot be bounded (and lattice cells) are in a per-unit "always" list, counted
  in `Stats`.
- **Node boxes**: large program nodes carry a may-be-true and a may-be-false box, so a
  candidate cell is usually rejected after a few comparisons (exact for points inside
  the cell's ROI, which the BVH guarantees). A node's box word is `NODE | k`, `k`
  counted from the cell's own `node_start` (`Model::node_boxes_of`), so a cell's
  program does not depend on the boxes of other cells; `Model::export` gives the GPU
  each cell's start.
- **Octrees of candidate cells** (`grid.rs`): curved cells have boxes far larger than
  themselves (~93 level-0 boxes contain a typical point of the reference model). Where
  a universe's boxes overlap a lot (sampled mean ≥ 8 per point), an adaptive octree
  keeps per leaf only the cells whose program is not false over the whole leaf box
  (`expr::eval_box` with `Shape::side_over`: exact for planes and for quadrics without
  cross terms, centred interval form for other quadrics and cones, tori undecided). Refinement is best-first by volume ×
  candidates within a memory budget (1024 words per cell); the octree is dropped
  unless it halves the sampled candidates, or (rebuilt with 64 words per cell) its
  residual programs cut the sampled words per point 10× (a cell written as the
  complement of many others, `sparse.i`). Points outside its root use the BVH.
  `GridMode::{Auto, Always, Off}`; tests force `Always`. The BVH alone stays a
  complete search structure.
- **Residual programs** (`expr::specialize`, `Octree::specialize`): each candidate of
  a leaf also keeps its program specialized to the leaf box: leaves whose side is
  decided over the box (`side_over`, beyond the tolerance) and nodes whose node boxes
  miss it are folded in, so a void envelope cell of 4 600 words keeps a handful within
  a 60 cm leaf; one true over the whole leaf keeps nothing (it costs no evaluation).
  Every use looks up the leaf holding the exact point (`Octree::leaf_span`), where
  the residual gives the whole program's value; a cell absent from that leaf is false
  there, as the octree's build test proved. Residuals no shorter than the program are
  not copied, and identical ones are stored once per batch of 1024 leaves. On the
  reference model they take 0.72 GB more than the octrees' 0.23 GB and 4.5 s more to
  build; `Options::residuals` off keeps whole programs (for A/B runs or little memory).
- **What a visibility leaves to see** (`skip.rs`, `Model::skip`): per cell whether it
  is worth entering (visible, and unfilled or filled with something worth entering),
  per universe the box of those cells, and per octree node whether some leaf below
  holds one (none when a cell checked everywhere is worth entering). Universe 0's box
  is unbounded when a cell checked beyond its problem box is worth entering. Beyond
  these boxes and nodes a point is in a hidden cell, a gap or a problem. The GPU
  renderer jumps over them (`rendering.md`, "How it traces"). `Model::first_hits`
  and `pixel_hits` take an optional skip and use it only where it is exact, when the
  visibility shows neither gaps nor problems: level 0 of each line is searched only
  in the spans that may hold a cell worth entering (the unit's box; its octree's
  leaves too when at most a quarter of them hold one, else the walk cost more than
  it saved), padded by 1e-3 cm, the windows of a later span as large as they would
  have grown to by then. Results agree with the whole search as tracings do
  (`reference::traces_agree`). ~50 ms per visibility on the reference model (16 threads;
  `skip` 45 ms, `export_skip` 9 ms for the GPU).

## Staged models and recompiles

`Model::compile` is `compile_staged` (everything up to the BVHs, ~9 s on the reference
model) then `completed` (the octrees, ~30 s). A staged model has each unit's octree
pending (`Unit::pending`); every query is exact through the BVHs, only slower, and
`save` refuses it. Octrees are `Arc`s, so `complete(&self)` shares everything else,
and are seeded by universe number: a unit's octree depends on that unit alone.
Cancellation is checked between compile stages and between octrees.

`Model::recompile(&old, input)` (`recompile.rs`) runs the front in full, then in the
top-down boxes loop asks, for each unit, whether the unit of the same universe in `old`
has the same content:

- the same region of interest, bit for bit (it is computed top-down first);
- the same cells by number, in the same order, at indices that keep that order;
- programs word for word equal, box words aside, up to shape indices: each leaf's sense
  bit equal, its shapes' coefficients equal (BLAKE3 of the serialized `Shape`), and the
  old-to-new shape map one to one over the unit;
- the same lattice and unsupported flags (unsupported cells: the same raw words);
- the same tolerance, world, grid options, and an `old` that is not cut.

Such a unit takes its cell boxes and node boxes from `old` (by position in the program:
the structure is the same) and its octree: the same `Arc` when no index moved, else a
copy with its candidates and the leaves of its residual programs rewritten (node-box
words are local, so they stay). Everything else is computed as a compile would, and
pending. Completed, the model saves exactly the bytes of `compile` (`tests/recompile.rs`;
a unit whose content is the same gets the same inputs, in the same order).
`Changed` lists the universes rebuilt and the cells (by number) new, changed or removed.
`Model::recompile_parsed` does the same from a migjorn tree already parsed (the
viewer's, edited in place), with the bytes `recompile` gives for its text.

`llevant_scene::store::Store` recompiles on a cache miss against the last entry of the
same path (`last/`, ~1 s to read), or against a model given by the caller (the viewer's
reload, Python's `Model.reload`).

## Drafts

A `Draft` (`draft.rs`; [decision 0011](../decisions/0011-live-drag-drafts.md)) is a
model with one surface moved, for the viewer to preview a drag. It is never saved; the
edit itself is compiled from the text as in "Staged models and recompiles".

`Draft::begin(base, tree, surface)` does three things:

- finds the surface's shapes (`surface_ids`) and the cells using them (their program
  leaves);
- checks that the shapes rebuild from the card as compiled (no `TRCL` copies);
- refuses, with the reason, lattice cells and an `IMP:N=0` cell of universe 0 under a
  problem box.

A universe cut out on its own (`Model::universe`) can be the base: its cut box stays
where it is, and each step answers as that universe cut to the same box from a fresh
compile of the edited input (`tests/draft.rs`, `universe_drafts_match_fresh_cuts`).

The work copy shares the octrees (`Arc`s) and takes 60–130 ms on the reference model.

`Draft::set(coeffs)` takes these steps:

1. **Shapes.** `build_surface`, the compiler's own path (macrobody check, MCNP's skew
   check, the surface's `TR`), written over the old shapes. A change of shape count or
   kind is refused, and the draft keeps its last good step.
2. **Moved cells.** Each is bounded against its unit's roi with `bound` (as
   `compute_boxes_with` does), with its node boxes set to `Aabb::INFINITE`. Node
   boxes only prune; unbounded, they never change a label. Bounding them would cost
   tens of ms per step.
3. **Roi cascade.** For each filled cell whose box changed, the target universes'
   rois (lattice elements by `lattice_element_box`) are grown to contain the new
   region, never shrunk. The cells of a grown unit are bounded again, and the cascade
   continues below them. A roi that only grows keeps every box conservative.
4. **BVHs.** `Bvh::refit`: the same topology, boxes recomputed bottom-up.
5. **Octrees.** The units holding moved cells get `grid = None` and `pending`. Queries
   there go through the BVH, exactly, and `save` refuses the draft.

Changed cells and units accumulate across steps, and each `DraftPatch` covers all of
them, in export layout:

- shapes;
- cell boxes;
- node boxes;
- BVH nodes and items;
- units: roi, the octree root, the always list;
- octree leaf programs set to `PROG_FULL` where the GPU keeps a unit's octree
  (at most `MAX_KEPT_CELLS` moved cells; they then join the unit's always list).

A patch alone brings an upload of the base to the current step, and `revert_patch`
brings it back to the base. `widen_skip` clears what a `Skip` would skip in or under a
moved unit: the base's skip boxes say nothing of the moved shapes. `snapshot` clones
the work model for the CPU views (46–68 ms on the reference model).

`tests/draft.rs` compares random steps on the zoo and random models against a fresh
compile of the edited input:

- classify labels are equal to brute force;
- traces agree;
- the draft's cell boxes contain the fresh compile's;
- a refused step leaves the draft as it was.

## Queries

- **Points** (`classify.rs`): `Model::classify(&[point], Depth::All)`. Labels are interned per
  chunk of points into flat buffers, then merged in parallel by hash shards
  (`intern.rs`), ids numbered by first occurrence in point order. Deterministic for any
  thread count. `reference::classify` (brute force) is the reference used by tests.
- **Levels**: `classify`, `trace` and `slice` with `Depth::Level(n)` stop at a
  universe level (0 is the top): there a filled cell is a cell like any other
  (`Inside`, the path ending with it; a lattice cell keeps its element, so element
  boundaries show at its level), and nothing below is searched. Labels are the full
  ones cut to `level + 1` steps; gaps and problems below are not seen. Off, the cost
  is one comparison per filled cell entered; on, levels below are not traced at all.
  `Model::levels()` counts the levels (1 + the longest chain of fills).
- **Lines** (`trace.rs`): `Model::trace(&[Line])` gives ragged segments (`offsets`,
  `t0`, `t1`, label ids, `surface_in`/`surface_out` as compiled-shape indices) that
  cover `[t_min, t_max]` exactly, gaps included, with the same labels as points.
  Labels are interned per thread, not per chunk (lines rarely repeat a label within
  a chunk: on `core.i` per-chunk tables held 30 M keys for 167 k labels), then
  numbered by first occurrence along the lines.
  Per level (one line in one universe):
  1. Candidates with a range of t each: octree leaves along the line (leaf boxes
     widened like cell boxes, so rounding opens no gap between leaves; a cell's
     range is the hull of its leaves', clipped to its box) or the BVH; beyond the
     unit's region every cell, or the `IMP:N=0` ones beyond the problem box.
     Nothing after this step depends on which one ran.
  2. Crossings, once per surface per line and level (a sparse-set cache; a cell
     skips surfaces it already met), of the leaves that can matter: for a cell found
     only through octree leaves, the surfaces of its residual programs in the leaves
     the line crosses (a surface decided over a leaf cannot bound the cell there);
     otherwise its whole program. Where the cell's stretch lies in the unit's region,
     a program node whose may-be-true or may-be-false box the stretch misses is
     constant along it, so its leaves are skipped (as point evaluation skips them). Planes; quadrics written about the
     point of the line nearest their origin, cancellation-free roots (a tiny leading
     coefficient just gives one far root); one-sheet cones add their apex plane; tori
     via `((ρ-a)²-K)((ρ+a)²-K)`, a quartic whose second factor only adds roots
     (harmless), solved within the bounding sphere by splitting at the derivative's
     roots and bisecting.
  3. Runs: each candidate evaluated with the point function at the midpoint
     between consecutive crossings of its own surfaces (with the residual of the
     octree leaf holding the midpoint, when it has residuals). Midpoints instead of
     sign flips at roots: no errors at double roots (tangent lines), and one source of
     truth with point classification.
  4. Run ends closer than the tolerance are snapped together (onto the range ends
     when near them), so coincident surfaces of neighbouring cells leave no slivers;
     a sweep then gives each stretch the status a point would get. Among coincident
     surfaces the lowest index is reported, whatever the program order.
  5. Pieces in filled cells recurse with the line moved into the universe (rigid, so
     t is shared). Lattices are walked face to face (`Lattice::element_range`, the
     26 neighbours at corners); each element piece takes the element `index` gives
     at its midpoint, and boundaries within the tolerance of the ends are dropped.

  `reference::trace` (every cell of every universe over the whole line, no boxes) is
  the reference; fast and brute force agree as `reference::traces_agree` says (see
  "Deliberate semantics"): segment by segment the same labels, ends within the
  tolerance, and at each end the same surface or another that passes within the
  tolerance of it, once on-surface slivers up to 1e-6 cm are passed over. Batches are split into chunks of up to 64 lines, fewer for small
  batches so an image's rows still occupy every thread. No `flags` array: the status
  is in the label, and a sliver is just `t1 - t0` below the caller's threshold.
  Recursion uses per-level buffers (depth is capped at 64); ranges must be finite
  and directions unit vectors.
- **First hits** (`hit.rs`): `Model::first_hits(&[Line], &Visibility, skip)` gives, per ray,
  where it enters its first visible segment: `t`, label, surface, and the surface's
  unit normal in the global frame, facing the ray (boundaries carry the depth of the
  universe whose frame their surface is in). `Visibility` is a flag per cell (a
  segment inside a cell is visible when every cell on its path is, so a hidden filled
  cell hides its contents) plus whether gaps and problems show; on-surface stretches
  never do. The ray is traced in windows of doubling length from its start (16 cm
  first), universes are entered only until a hit, never through hidden cells. The
  result is the first visible segment `trace` gives, as two tracings agree
  (`reference::traces_agree`: the same label and surface, `t` within the tolerance; a ray
  traced in windows can round a crossing one ulp away). It serves picking, tests and
  CPU renders; interactive 3D is the GPU renderer's ([`rendering.md`](rendering.md)).

  Two more parts of `Visibility` see the levels of a model. `level` shows one
  universe level as MCNP's plotter shows `LEVEL n` ([`mcnp-behaviour.md`](../mcnp-behaviour.md)):
  rays are traced down to it only, and only segments whose path is `level + 1` steps
  long are visible (a filled cell there is solid; cells above it that are not filled
  show nothing; gaps and problems show at any level, as asked). `ghosts` (a flag per
  cell, empty for none) are cells seen as shells, never as solids: a ray goes on
  through them (a filled one's contents show as `cells` says), and `Hits::ghosts`
  gives per ray the ghost boundaries crossed before its hit (up to and at it, a
  boundary within the tolerance of another counted once) and the first one's cell,
  `t` and normal. Ghost stretches are recorded only when there are ghosts; their ends
  on a surface (not at a line's or a window's end) are the boundaries, the same with
  or without a skip. `Model::skip` counts ghost cells as worth entering (their shells
  show) and, with a level, cells by the levels their universe is reached at.
  At the same place, the ghost cell entered is named first, as the GPU names it.
  With `veil` (2026-11-11), gaps and overlaps that are shown are seen through: a ray
  goes on through them, and `Hits::veils` says per ray whether it went through a
  gap, an overlap (which the GPU renderer cannot match: it does not look for
  overlaps), before its hit. Unsupported cells and too-deep nesting still stop rays.
- **Surfaces** (`surfaces.rs`, 2026-10-31; for inspectors and labels, never on the
  paths above): `cell_surfaces(cell)` (each compiled surface of a cell's program once,
  with its sense; complements expanded) and `cell_surface_leaves` (per leaf, no
  allocation, for passes over every cell); `nearest_surface(p, dirs, reach, level)`,
  the crossing nearest `p` on a few short lines through it, traced on the calling
  thread with one tracer whose surface slots are kept between calls (0.07 ms per pick
  on the reference model; a parallel batch cost 0.33 ms, nearly all of it setting up);
  `surfaces_across(p, dir)`, every surface within the tolerance of `p` that bounds a
  cell on either side of it (a surface "bounds" a cell's program there when the
  program's value depends on the side of it taken, other surfaces through `p` left
  undecided); `bounding_surface(shape, p, dir)`, the one to name for a crossing (see
  "Deliberate semantics"); `surface_normal(shape, p)`, the unit gradient of its
  function, towards its positive side (the viewer's ruler through the layers).
- **Batches and threads**: every query takes arrays; rayon splits them over chunks;
  the model is shared and immutable. Results are written by index, never in the order
  threads finish, so they are identical for any thread count.

## Fingerprints

`Model::fingerprints(precision)` (`fingerprint.rs`) gives every cell and every
universe two BLAKE3 digests: `geometry` (what segments depend on, the key for cached
segments) and `content` (geometry plus material compositions and densities). Purpose:
across two versions of a model, prove that a cell and everything inside it are
unchanged, without tracing. They are computed from the compiled model, so transforms,
`LIKE BUT`, `TRCL`, `#n` and macrobodies are already resolved and numbering never
enters:

- Surfaces in canonical form: planes and quadrics get a sign convention (leaves
  using a flipped surface flip their side); quadrics are written about the point of
  their centre set nearest the origin and scaled to a largest second-order
  coefficient of 1; tori by centre, axis up to sign and radii. So an RPP equals its
  six planes, `C/Z` a scaled and negated `GQ`, a `TRCL` cell its moved surfaces.
- Each value is rounded to a relative precision of its own magnitude (of 1 cm or 1
  when smaller), default 1e-9: a binary exponent and a multiple of the step.
- AND/OR children are hashed as sorted sets, except in lattice cells (face order
  numbers the elements). `IMP:N=0` and unsupported reasons count as geometry.
- A filled cell includes its fill transform and the filling universe's digest
  (fill arrays element by element), so a digest covers everything inside; a
  universe is the sorted set of its cells' digests.
- Materials: ZAIDs case-folded, duplicate entries summed, fractions normalised by
  their total (keeping the atom/weight sign), sorted; the material number is
  ignored.
- The hasher is keyed by `FINGERPRINT_VERSION` and the precision, so digests of
  different versions or precisions never match. Bump the version whenever llevant
  could trace an unchanged input differently.

Two caveats, both "slower, never wrong": envelopes converted from CAD again may come
out decomposed differently even if the geometry is the same, and coefficients near a
rounding boundary can make two identical surfaces hash differently. Never the other
way round: a changed geometry must not hash the same, so the rounding must be finer
than any change that matters.

Tests: renumbering cells, surfaces, materials and universes with migjorn (everything
migjorn can renumber: macrobody facet references, transforms in `FILL`/`TRCL`,
universes in fill arrays) leaves every digest unchanged on the zoo models; equivalent
ways of writing the same geometry hash the same; a change reaches exactly the cells
that contain it (and the cells that fill them), below the precision none; card order
and thread count do not matter.

Still open, for when a downstream tool compares versions: how stable they are across
two real conversions of the same envelope, and how cell labels of a cached envelope
map onto the new numbering (cells with equal digests within a universe are
interchangeable; duplicates need the order they appear in).

## Universes on their own

`submodel.rs`: `Model::universe(u, cut, grid)` makes universe `u` level 0 of a new
model, in its own frame, with every universe it reaches below it. It is built from
the compiled model, not the input, so it works on a model read from the cache:
- the cells, programs, surfaces and lattices it reaches are copied and renumbered;
- node boxes are dropped;
- boxes, BVHs and octrees are computed again for the new root.

Cell and universe numbers stay the same, so hidden cells, looks and labels mean the
same things.

`Cut` is the box it is cut to:
- `Own` is measured from what the universe holds: `extent` of its cells with a material
  or fill, widened by 1 %. It does not use the shapes of the envelopes the model
  places it in, so parts of a filler that an envelope clips are seen. CAD cells often
  get finite boxes only from the envelopes around them. When none is bounded by its
  own surfaces, the contents are measured within the universe's region in the model
  (the union of its envelopes' boxes), grown by half its largest edge on every side.
- `Box(b)` uses a box you give.
- `None` means no cut.

Beyond the box lies one void `IMP:N=0` cell, numbered after the highest cell. It is
bounded by six planes numbered after the highest surface (`Model::cut_cell`). The
box is the new model's problem box exactly, not padded as bounds are. So beyond the
cut planes only that cell counts (with any `IMP:N=0` cells the universe already had,
also cut to the box), and inside them every answer is the universe's own: the
programs of the cells it cuts are not rewritten. Padding would have let the cut cell
overlap the filler's unbounded void cells in the shell between the planes and the
padding.

`Model::point_at_level(label, k, p)` gives a point in the frame of the universe at
level `k` of its path, through fills and lattice elements, as classification goes
down. It maps a pick into a universe opened on its own.

Derived on the reference model in 0.3 s for universe 64 (892 cells), 1.6 s for 20130
(2 381 cells) and 4.8 s for 20040 (14 791 cells; its CAD cells are bounded only
within where the model uses it, so its contents are measured twice), octrees included.
The tests check the fast path against brute force,
the cut cell alone beyond the box, and the model's own labels from the universe's
level down (`tests/submodel.rs`). `llevant-bench verify`, `trace` and `gpu-check` take
`LLEVANT_UNIVERSE=U`.

## Cell instances

`instance.rs`: a cell of a universe that fills twenty envelopes is one cell card
but twenty pieces of geometry, and a lattice cell is one piece per element. An
*instance* is one such piece. `Model::instances()` numbers every instance with
a dense id in `0 .. total`, so that values can be kept per instance in a plain
array (results of a repeated structure, colours) and the GPU can compute the id
while it descends.

- **Sites.** A cell filled with a universe (`FILL=u`) is one site of it. Each
  element of a fill array that holds a universe is one site too. Every instance
  of the cell holding a site gives one instance of the universe there. So a
  universe's instances come as one block per site, sites in cell (and element)
  order, each block as long as the host universe's instance count:
  `instance(u) = offset(site) + instance(host)`.
- **Uniform lattices.** A lattice filled with one universe everywhere (no fill
  array) is one site. Its elements are numbered within a box of indices that
  bounds the lattice cell's box (one wider for hexagons):
  `instance(u) = offset + instance(host) · E + element`. Elements of the box the
  cell does not reach still get ids, so the ids have holes there.
- **Cells.** A cell's instances are those of its universe, and for a lattice
  cell, one per element of its fill array or box:
  `id = base(cell) + instance(universe) · E + element`. The ids of cell `c` are
  `base(c) .. base(c + 1)`.
- **Edge cases.** Unit 0 has one instance; a universe nothing fills has none. A
  lattice cell unbounded along one of its lattice's axes numbers no elements,
  and nothing below it gets an id. If the numbers overflow 64 bits, nothing
  gets one.

API: `Model::instance(&label)` gives the instance holding a point (for
`Inside` labels), `Instances::of(model, path)` the instance of any path, and
`Model::instance_path(id)` the inverse (a binary search over cell bases, then
over each universe's blocks). `Model::format_path` and `Model::parse_path`
write and read MCNP's repeated-structure notation, deepest cell first:
`12 < 100[1 0 0] < 5`. The numbering is made on first use from the compiled
model (a `OnceLock`, not cached on disk). That takes milliseconds.

Tally bins (for fields from MCTAL files, `rendering.md`, "Fields"): a bin of an
MCNP tally card in repeated-structure notation is a chain of levels from the cell
scored outwards, each a set of `Step`s (`(8 9 < 7[0 0 0] < 3)` is
`[[8, 9], [7[0 0 0]], [3]]`; a lattice cell without an element is all of them).
- `Instances::matching(m, chain)`: the instances whose cell is in the first level
  and whose path, going up, has each further level in order. Levels in between
  may be left out, as MCNP allows (`(8 < 3)` across lattice 7 scores; checked
  against MCNP output in `llevant-scene/tests/tallies.rs`).
- `Instances::innermost_bins(m, chains)`: for every instance, the bin that holds it
  most closely, or `NO_BIN`. A bin holds the instances of its cells and, when a
  cell on the path above is the bin's, the instances inside it too (an F4 on a
  filled cell scores all it holds). Ties go to the bin whose cell is deepest on the
  path, then more levels, then a single cell rather than a union, then the first
  bin: a deliberate choice, MCNP scores each bin on its own. Only the scored cells'
  instances are walked unless one of them is filled; then all of them, in parallel.

The export carries it to the GPU. Each fill target has its offset and stride;
each lattice its numbered elements; each cell its base. `llevant-gpu` writes
the instance id as a ninth word of each hit (`NONE` past 2³² ids).

Tests (`tests/instances.rs`):
- A brute-force walk over every fill and element of the zoo and random models
  finds each id exactly once, and `instance_path` gives the path back.
- Without uniform lattices the ids are exactly `0 .. total`.
- Classified points know their instance.
- Paths that do not exist are refused.
- Tally chains: a whole path as a chain is its one instance, a cell alone all of
  its instances, a cell below its level-0 cell (levels left out) and a lattice
  level without its element what walking the model says; `innermost_bins` gives
  each instance the bin of the deepest cell scored on its path.
- `vs_cpu` checks the GPU's instances against these: all equal but at lattice
  element edges.

## Views

`view.rs`: `Slice` (a rectangle of a plane, `nx × ny`; `rows()` gives one line per
row, `pixels(&tracing)` the label at each pixel centre; `Slice::axis`, `Slice::plane`
for any plane, `Slice::radial` for the plane through an axis at an angle) and
`Camera` (pinhole, or orthographic with `ortho` the view's height; `fit` to a box;
`origin`, `direction` and `point` of each pixel's ray). A camera's `cut`
(`Cutaway`) removes a union of convex regions, each an intersection of half-spaces
(`Clip`): one plane, a wedge about an axis (`Cutaway::wedge`: one region of two
half-spaces up to a half-turn, two regions of one beyond), a box kept
(`keep_box`: six regions of one half-space) or removed (`remove_box`: one region of
six). `Cutaway::kept` gives the parts of a ray outside it, each with the normal of the
cut face it starts on. `Model::slice` traces a slice's rows; `Model::first_hits` casts
the rays of given pixels part by part (each part in a batch of `first_hits`) until one
hits, a hit at the start of a part inside a visible cell taking the cut face's normal;
`Model::render` is that for every pixel. Tested against brute force: no point of the
kept parts before a hit shows (`tests/views.rs`). Colouring them into images is
`llevant-render`'s job.

`Model::extent(scope, n)` measures the box around the contents (`Scope::Contents`,
cells with a material or fill) or the problem (`Scope::Problem`, all but `IMP:N=0`)
by tracing `n × n` lines per axis twice: cell boxes from CAD conversions can be
kilometres wide (±3.7 km on the reference model, whose contents span ±18 m).

## Volumes

Volumes live in the engine, on top of `trace`: at whole-model scale the segments do
not fit in memory, so lengths are added up per chunk of lines inside the parallel
loop, which only the engine can do cheaply.

- **Grid** (`VolumeGrid`): `probes` (default 12) independent lattices of parallel
  lines, each with a random direction, rotation about it and offset drawn from the
  seed (uniform rotations, Shoemake). `h` sets the cost: as many lines as three
  axis-parallel families `h` apart, so each probe's lines are `h·√(probes/3)` apart
  (`2h` by default). Lines are identified by integers (probe, row, column), so two
  versions of a model share them and can be compared line by line. Random directions
  mean no flat face runs parallel to the lines: with axis-parallel lines, faces
  parallel to them made errors of order `h` (a 1.3 cm slab 25 % off at h = 1) that
  hid behind the round numbers of MCNP models; thin plates and gaps are still
  crossed by every line over them.
- **Estimator** (`Model::volumes`, `volume/`: `mod.rs` traces, `grid.rs` makes the lines, `placed.rs` the boxes of instances, `refine.rs` the rounds of `volumes_to`, `instances.rs` the volumes of instances and chosen cells): a probe's volume of a label =
  Σ length × spacing², unbiased over the random shift; `estimate` is the mean over
  the probes and `sigma` its standard error, honest because the probes are
  independent (2σ covers the error ~95 % of the time, `tests/volumes.rs`). Chunks
  of lines are traced one per thread and summed by encoded label, lengths as
  integers in units of 2⁻⁶⁴ cm (exact in any order, so identical for any thread
  count), and each is folded into shards by label hash as soon as it is done:
  memory follows the chunks in flight and the distinct labels, not the lines; only
  distinct labels are decoded. Boundaries along a line are
  exact; features are only missed sideways, if their cross-section is under about a
  spacing squared. `segments` and `probes_hit()` say how many lines met a label: σ
  from a few is itself uncertain. Errors shrink about as h² (h^1.5–h³ by shape).
- **Within boxes** (`Model::volumes_within`): the grid's lines inside the union of a
  list of boxes alone, each stretch of the union traced once. The lines are those of
  the whole region, so a label's length inside the boxes is what the whole check
  gives there. `Model::instance_boxes(cells, region, side)` gives the boxes: every
  instance of the cells in the global frame (filled cells as cells), from the placed
  boxes below, split down to `side`; `Model::placed_boxes(cells, region)` the same
  boxes unsplit, one per instance with its cell and whether it was bounded again
  where it is placed (`exact`: not below a lattice, whose elements are not followed
  one by one) — what `llevant bbox`, `render --isolate`'s framing and Python's
  `Model.bbox` give, through `llevant_scene::bounds`; `spacing_within` the spacing for a number of
  lines through their union (measured on rasters of its projections, overlaps
  counted once). After an edit, the old model's boxes of the
  cells changed or removed and the new model's of those changed hold every gap or
  overlap the edit can have made (the viewer checks them after a reload).
  `Model::cells_kept(cells, region, cut)` says per cell whether some part of it may
  be outside a `Cutaway` (the viewer's rule `shown`): each instance's placed box is
  classed against the cut (`Cutaway::place_box`: wholly in one region removed,
  outside every region, or neither); one across a cut is halved along its longest
  side and the halves bounded again, depth first, until a piece is wholly kept or
  every piece is wholly removed, giving up as kept after `CUT_SPLITS` (64) splits or
  below `CUT_SIDE` (1e-3 cm). Conservative: a cell found wholly removed is; one kept
  may only come near what is kept (box padding, a cell touching a cut plane, below
  a lattice only the lattice's box). Placing boxes only follows the filled cells
  that lead to a cell asked for, so a universe's cells are placed in tens of ms.
- `Volumes::group_by` / `grouped` (by cell, material, universe or label: every
  instance of a cell on its own) and `problems()` (gaps, overlaps, unsupported, too
  deep; with the box of their segments). Groups add their labels' lengths per probe
  before taking the spread: instances share lines (lattices especially), so their
  errors are correlated and their σ² do not add.
- **To a target** (`Model::volumes_to`, `VolumeTarget`): groups wanted are refined
  until `sigma / estimate ≤ rel_sigma` and every probe met them, or (with
  `abs_sigma` > 0, default 0) `sigma ≤ abs_sigma` cm³ with at least two probes
  through them: a relative target alone drives `h` down to a fraction of the
  smallest cell's size, though a tiny cell's volume rarely matters in absolute
  terms (`BinTarget` has the same rule). Nothing is left out silently:
  - *Every group is listed*, met or not: the groups come from the model (every cell,
    material or universe of a cell points can end in; by label every instance, from
    the instance numbering, up to `MAX_LISTED_INSTANCES`, beyond which only the
    labels met are, `Converged::listed_all`), not from what the first lines hit.
    Coarse lines miss whatever is under about a spacing across, however many cells
    that is (2 276 MCNP-tallied cells of a 17 k-cell model at h = 10).
  - *Placed boxes* (`PlacedBoxes`): every universe's placements in the global frame,
    each a transform and a clip box (the filled cells above), or below a lattice and
    beyond 1024 placements just a box. A cell's box at each placement is its
    program bounded again (`Bounder::bound`) within the clip box taken to the
    universe's frame: tighter than its box in the universe's frame, and empty where
    the cell cannot reach (a universe's graveyard well outside its envelopes).
    Conservative, so all of a group is inside them.
  - *Covering* (`PlacedBoxes::cover`): a placed box with a side over
    `COVER_SPACINGS` (8) spacings is halved along its longest side and each half
    bounded again, largest first, up to `COVER_BOXES` (256) boxes a cell: a curved
    or diagonal cell is then traced over little more than itself.
  - *Levels and jobs* (`Model::refine`, shared with `instance_volumes`): level `l`
    is `h / 2^l` with its own seed, down to `h_min`. After each measurement a
    group's next step (`Levels::next_step`) is the cheaper of
    - more probes at the same level (further lattices of the same seed and
      spacing, `volumes_in` over a probe range, pooled with those it has: the
      variance falls as 1/n), in blocks of `PROBE_BLOCK` (6) up to the target's
      `max_probes`, and
    - a fresh start at the coarsest finer level predicted to reach the target:
      the spread per probe falls as `2^(-jα)`, α from the group's last two levels
      (clamped to 1–3) or 1.5 until measured; it costs 4× the lines per level.

    `max_probes` sets the effort: at the grid's probes (the CLI's and Python's
    default for a whole model, and the viewer's) no probes are added, groups only
    go finer or stop, which is quick and leaves groups near the target
    unconverged; `MAX_PROBES` (48, the default for `BinTarget` and cell lists)
    converges more of them for more time.
    A group not met by every probe goes one level finer; one that no level down to
    `h_min` would bring to the target, even at α = 3, stops there
    (`unconverged`, with the volume and `h` it has). Jobs run coarsest level
    first; at a level, consecutive probes wanted by the same groups are one job,
    traced once over the union of their boxes (`union_pieces`): a probe's rows of
    lines are cut into bands (each a chunk, about `total / CHUNKS` lines over
    boxes, from the grid and boxes alone); a band takes each of its boxes' lines,
    the range of each line in the box, sorts them by line and entry and merges
    ranges that overlap or abut, so each stretch of the union along a line is one
    piece and the union is sampled as one region; boxes inside another are
    dropped first. Before 2026-11-01 each box's lines asked a BVH of the boxes
    which box owned each stretch: half the thread time of a round on the
    reference model. With the sweep, making pieces is 1.5–3 % of a round's thread
    time at h = 20 down to 2.5; the rest is tracing. Gaps, overlaps and other
    groups without cells use the box of their segments, padded by the spacing
    that found them.
  - Outcomes: converged; `Converged::unconverged`, still above the target at
    `h_min`; `Converged::unseen` (also unconverged), no line met it, and refining
    stops once lines 1/32 of its boxes' longest side apart have crossed them all
    (`UNMET_LINES_ACROSS`): what it holds is then far smaller than its boxes,
    about the cube of the spacing or less (cells that cannot be shown empty, like
    the outside of a universe's sphere, would otherwise be traced at every
    finer `h`); `Converged::empty`, its placed boxes are empty: volume exactly 0
    (cut away by its envelopes, or in a universe nothing fills).
  - *So far* (`Model::volumes_to` with `each`): the same, handing a callback the
    `Converged` it would return if it stopped there, before every further level
    (groups yet to refine in `unconverged`). Rounds on a big model can take tens of
    minutes each; the viewer shows each state and keeps the last on Cancel.
  - The debug log (`RUST_LOG=llevant=debug` in the CLI and the viewer) has each
    job: level, `h`, probes, groups, boxes, lines tried, pieces traced, time, and
    the thread time spent making pieces and tracing them.
- **Of chosen instances** (`Model::instance_volumes`, `BinTarget`): the volume
  of each set of instance ids; a tally bin is the set `Instances::matching` gives
  for its chain (MCNP's rules for left-out levels and lattice elements
  included), resolved in parallel over the scored cells' ids only. For callers
  that know what they want (new_rad: each tallied cell in its envelopes). No
  whole-model pass and no `MAX_LISTED_INSTANCES`: lines go only around the
  instances.
  - *Instance boxes* (`place_instance`): each instance's frame followed down its
    path, through fill transforms and lattice elements (each element's box,
    `Lattice::element_box`: corners of its planes, padded, unbounded along a 2D
    lattice's axis), so an instance below a lattice is bounded alone, not by
    the box around all the elements as in `PlacedBoxes`. Then bounded again and
    covered as above (`cover_placed`), up to `MAX_COVER_BOXES` (2²⁰) boxes a
    round over all instances. Past `MAX_BOXED_INSTANCES` (2¹⁸) instances, the
    largest sets use `PlacedBoxes` for their cells instead.
  - *Levels*: `h = h_min · 2^k`, coarse to fine, refined as `volumes_to` refines
    (`Model::refine`: more probes or a finer level, whichever is cheaper). A set
    starts at the `h` that puts `START_LINES_ACROSS` (8) lines across its largest
    box (at most `h_max`), so a small instance is never left to lines that would
    miss it, and goes on until it reaches the target. Lengths of a set's
    instances are added per probe, so σ is the set's own. Outcomes as for `volumes_to`
    (`unconverged`, `unseen`, `empty`); `instances` counts each set's instances,
    0 for chains the model does not have.
  - *Filled cells*: when a set holds an instance of a filled cell, each label's
    lengths go to the sets of every level of its path (each set once), so a filled
    cell's volume is that of its region, whatever fills it.
  - Tests: one bin per envelope and one over both (`tests/volumes.rs`), a 3 × 3
    lattice of spheres under a transformed fill (all of it, one element by its
    full path, one with the levels between left out, a missing element); unit
    tests for element boxes (square, hexagonal, 2D) and instance boxes.
- **Of chosen cells** (`Model::cell_volumes`): one set per cell, all its instances,
  through `instance_volumes`; returned as a `Converged` by cell, in the order asked
  (a cell with no instance is `empty`). The viewer's selection, `llevant volume
  --cells` and Python's `cell_volumes` use it: no pass over the whole model.
- Open: envelopes converted again can have surfaces moved by ~1e-5 cm, which makes
  thin differences along every boundary. A tolerance for "surface noise" in version
  comparisons (around 1e-4 cm) is to be set from real conversions.

## Public API

`Model` fields are crate-private behind read accessors (`cells()`, `units()`,
`material_ids()`, `diagnostics()`...). The engine modules are private; the public
surface is the crate root, `export` (the flat model and its program encoding, for
`llevant-gpu`) and `vec3` (helpers on `[f64; 3]`); `internals` (`#[doc(hidden)]`, no
stability) serves `llevant-gpu`, `llevant-bench` and the oracle. `Lattice` and `Target` are opaque
but for what lies in an element (`Lattice::element`, `fill_array`) and a target's
universe.

Results keep cell and shape indices (compact, numpy-friendly); `cell_id`,
`surface_id` (MCNP number and facet; a `TRCL` copy keeps its original's), `material`
and `describe(label)`
(`inside 12 > 304[1,-2,0] > 7`) give MCNP terms. Diagnostics name their cell, surface
or material (`Subject`). Queries check their arguments and return `llevant::Error`
instead of panicking: `trace`, `first_hits`, `slice`, `render` (lines with a finite,
ordered range and a unit direction; one visibility flag per cell), `volumes`
(spacing, region, directions), `fingerprints` (precision), `Visibility::hide`
(unknown cells or universes), `load` (I/O, with the path). Problems in the input stay
diagnostics. `Status`, `Subject`, `Options`, `Cell`, `Unit`, `Diagnostic`, `Stats` and
`Change` are `#[non_exhaustive]` (`Options` has `with_*` builders). The library, not
the CLI, computes `Volumes::grouped`, `Model::fingerprint_diff` (content, geometry,
renumbered, removed, new) and `Status::name`.

The model never changes after compiling: anything a frontend varies (what materials
look like, visibility, cameras) is a value the caller owns and passes in.

## Deliberate semantics

- An unsupported cell only decides points no supported cell claims (then the status is
  unsupported instead of gap); MCNP's own input restrictions (skewed one-sheet cones and
  tori, non-orthogonal macrobody vectors) are reported as unsupported cells.
- A lattice cell is treated as filling all of its universe.
- A lattice's elements tile space exactly: their faces are rebuilt from the lattice's
  translations (`lattice::tiling_faces`) instead of taken from the input's planes, whose
  rounded normals leave slivers between neighbours (a hexagon's third pair of sides is
  not quite at 60° to the others: gaps of ~1e-6 cm at its corners). A hexagon's sides
  are the planes halfway to its six neighbours' centres; a parallelepiped's are where
  the element index changes. Points are located and lines walked with the same faces,
  so a point inside a traced segment is in the segment's element even at a corner.
  The faces move by the input's rounding, far below the tolerance.
- At a level (`Depth::Level`), a lattice element outside the fill array is
  still a gap (MCNP has no cell there), and one filled by the lattice cell itself is
  the lattice cell; a filled element is the lattice cell with its element.
- Beyond the problem box only the `IMP:N=0` cells are checked, by the fast path and
  brute force alike. Every point there is in one of them (the box is a conservative
  bound of what they leave), so the only answers that differ from checking every cell
  are overlaps of another cell with the graveyard out there, where MCNP kills
  particles anyway: they read as the graveyard cell. Inside the box nothing changes.
  Checking every cell beyond the box made views from outside the sphere 5–24× slower
  ([decision 0003](../decisions/0003-problem-box.md)).
- Segments are decided at midpoints between crossings, points where they are. So a
  point within the tolerance of an internal surface of its cell (one the cell holds on
  both sides, such as the symmetry plane between the two halves of a union) is
  on-surface, while the segment through it is inside the cell. Likewise a line at
  grazing incidence stays within the tolerance of a surface over a stretch of
  `tol / sin θ`; points there are on-surface next to the segment's cell.
- Because of that, which crossings a tracer collects decides where such on-surface
  bands show. Brute force intersects every surface of every cell, including surfaces
  that cannot bound a cell where they cross the line; when one crosses within the
  tolerance of an internal surface of the cell (two nearly coincident planes of a
  GEOUNED union, 1.35e-8 cm apart), a midpoint falls in the band and an on-surface
  sliver of ~1e-8 cm shows. The fast path collects crossings only from surfaces that
  can bound each cell (residual programs) and may not show it; nor which of two
  surfaces within the tolerance of an end is named. Points are unaffected (exactly the
  brute-force labels), and so are gaps and overlaps (stretches between the cells'
  real boundaries) and volumes: on the reference model's 100 000 random lines the
  overlap segments and lengths are identical and 2e-7 cm of 1e7 cm moves from
  on-surface to a neighbouring cell ([decision 0002](../decisions/0002-residual-programs.md)).
- Of the surfaces through a crossing, tracing names the lowest-indexed one the
  candidate cells use, which may not bound anything there: a macrobody's facet is an
  infinite plane, so where two boxes sit side by side the top facet of one is also
  the top of the other. `trace` keeps that (it is cheap and `traces_agree` accepts it);
  picks and labels name `bounding_surface` instead, the surface that bounds a cell
  there (the lowest MCNP number of those that do), and inspectors list the others
  (`surfaces_across`).

## Checks

- **MCNP itself is the reference for semantics.** `mcnp-oracle check` sends points as
  an `SDEF` source list and reads the cell of each from PTRAC source events;
  `mcnp-oracle submodel` cuts large models down so MCNP can set them up in minutes.
  Synthetic models (`tools/mcnp-oracle/models`) cover every feature; MCNP-confirmed
  points are kept as golden tables that `cargo test` replays. Zero disagreements with
  MCNP in every comparison so far; it caught two llevant bugs (REC, WED facet order)
  and several MCNP behaviours not obvious from the manual ([`mcnp-behaviour.md`](../mcnp-behaviour.md), e.g. a
  FILL transform replaces, not composes with, a TRCL). A direct check of crossings
  (PTRAC surface events of pencil beams) is possible but not done; tracing reaches
  MCNP through point classification.
- **Fast path against brute force**: property tests on random models (lattices and
  TRCL included) and on the reference model require the BVH/box path to give exactly
  the labels of evaluating every cell; again with an octree forced into every universe
  (`GridMode::Always`, residual programs included), and the golden tables are replayed
  in both modes. `llevant-bench compare-trace` compares whole line sets with octrees on and off.
- **Between queries**: fast tracing agrees with brute-force tracing line by line
  (`reference::traces_agree`),
  points along every segment classify to its label (random models, the zoo models,
  the reference model), and the segment at each golden point holds the cell MCNP
  reported. Results are identical in every `GridMode` and for any thread count.

## Performance

Current numbers are in [`benchmarks.md`](../benchmarks.md) (`llevant-bench report`). On
the reference model, level 0 used to do ~80 % of the surface evaluations of a point
(envelope programs average ~500 words, and their boxes are loose); residual programs
and the problem box brought it down to a few tens ([decision
0002](../decisions/0002-residual-programs.md), [0003](../decisions/0003-problem-box.md)),
and the fillers (level 1) now dominate.

Profiling works without perf permissions: `LLEVANT_PROFILE=<dir>` makes `llevant-bench`
`points`, `trace` and `view` sample their timed section with pprof (flamegraph SVG
and top functions; `samply` would need `perf_event_paranoid` ≤ 1, i.e. sudo); self
times are unreliable under inlining, inclusive times are better.

Compiled models can be saved and read back (`saved.rs`, `Model::save` /
`Model::read` / `Model::from_bytes`; [decision 0004](../decisions/0004-model-cache.md)): a header (magic, `FORMAT_VERSION`, `ENGINE_HASH`, a digest of the
engine's source, version and `Cargo.lock` from `build.rs`; migjorn is pinned exactly), then sections, each a length and
bitcode: the `Model` without its octrees (timings left out), then each octree's
arrays in pieces of 4 M items. Sections are made and written one at a time, so saving
needs one piece beyond the model, not a second copy of it. The cache decodes an entry
from a memory map of the file (`from_bytes`), so its bytes are never copied to the
heap. Only the build that saved a model reads it. The disk cache built
on it is the frontends' (`llevant_scene::store::Store`): an entry is keyed by a BLAKE3
hash of the input's bytes, the `Options` and `ENGINE_HASH`, written to a temporary
file and renamed; the most recently used entries are kept within 8 GB
(`LLEVANT_CACHE_GB`), in `LLEVANT_CACHE_DIR` or the user's cache directory;
`LLEVANT_CACHE=off` turns it off and an unreadable cache is ignored. On the laptop the
reference model's entry is 1.5 GB and reads in about 1.4 s; compiling and storing it
peaks at 5.4 GB committed (the compile alone: 5.3 GB), a cached load at 2.3 GB; `tests/saved.rs` checks that a saved model answers exactly as the
compiled one.

Work counters (`Model::work_points`, `Model::work_lines`, behind the `bench` feature; `work.rs`) run the
same code as `classify` and `trace` and report, per universe level, candidates,
surface evaluations, surfaces intersected and midpoint evaluations per point or line.
`llevant-bench points`, `trace` and `view` print them, and `llevant-bench work`
writes them as JSON for the nightly regression check
(`tools/bench/work-baseline.json`); `view` covers the three ways
models are looked at: fillers (`Visibility::solid`), void envelopes (every cell but
the graveyard and the void around the eye) and a cutaway through an envelope with
voids shown. `llevant-bench gpu-probe` gives the GPU side: time, steps and words
per ray against the step cap, for `solid` or `voids`.

`LLEVANT_BENCH_SET=<dir>` makes `llevant-bench points`, `trace` and `view` save their
points, lines and view boxes there on the first run and replay them afterwards: they
are sampled from cell boxes, so a change that moves boxes would otherwise change the
workload it is measured on. For an A/B, run the old build first.

Parsing: migjorn's parse takes ~1.1 s and its tree ~2 GB at peak. Inputs the zoo does
not cover yet should give clear diagnostics, never wrong geometry. Cell parameters as
data cards, `READ` cards and the vertical format are out of scope (no example model
uses them) and reported when present.

Deferred work, with what would make each worthwhile:

| Optimization | Worth doing when |
|---|---|
| Evaluate each surface once per point and share it between cells | Unlikely to pay: a memo of surface sides in tracing cost as much as it saved, since a repeated surface is already in cache |
| Tracing: track leaf signs across crossings instead of re-evaluating each cell at every midpoint | Only with a cheaper bookkeeping than a per-surface memo, which gave nothing and broke exactness (on-surface bands within a stretch); evaluation is ~39 % of tracing |
| Faster octree build (cells proven true over a parent need no test in its children; bounds for tori) | The +3 s load on a 16-core workstation becomes a nuisance |
| Compact plane storage (shapes are 112 B, planes need 32 B), shapes and programs ordered by locality | Likely ≤ 10 %: tracing the same lines in Morton order gained ~10 %; dropping the quadric square root gained nothing |
| Finer level-0 octree (smaller `min_edge`, larger budget) | Memory is cheap: 1.3× for points at 8× the octree memory |
| Reuse between `first_hits` rays from one eye (the eye's cell, candidates per tile of pixels), window tuning | CPU views without a GPU (picking, fallback) need to be faster; neighbour lists go to the GPU renderer |
