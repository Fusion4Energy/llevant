# Images: colours, realistic shading, the GPU renderer, the viewer

How llevant turns geometry into pictures. The engine ([`engine.md`](engine.md))
gives labels along slice rows and first hits of rays; four crates build on it:

| Crate | Owns |
|---|---|
| `llevant-scene` | What the frontends build from a model: which cells 3D views hide (`Hiding`), what materials look like (`Looks`), field data (`Field`) and tally reading, cell rules, the disk cache |
| `llevant-render` | Colours of cells (by cell, material, universe, realistic, density) in one `Paint`, colour scales (`Scale`), how fields are drawn (`FieldLook`; with the data, an `Overlay`), images of slices and renders, physically based shading, PNG files |
| `llevant-gpu` | First hits of camera rays on the GPU (wgpu compute), approximate by design |
| `llevant-viewer` | The interactive app and its embeddable egui widgets (`View3d`, `SliceView`) |

The CLI's `slice`/`render`/`materials` and the Python `Image`s are thin calls into
these. Rules: images are a function of the engine's answers (labels, hits) and of
values the caller owns (a `Paint`: the colouring, the `Looks`, an `Overlay`); the
model is never changed to draw it. Exact answers (which cell is this pixel?) always
come from the CPU engine.

## Colours

`llevant_render::colours`: a colour per cell that depends only on `(kind, number)`,
so it stays the same between runs and models; void near white when colouring by
material; grey on-surface. Gaps and problems (overlaps, unsupported, too deep) are
patterns fixed to the image's pixels, which no cell colour can be mistaken for
(2026-11-11: a flat magenta read as one more pink cell, black as a dark one):
`marked` gives a black and grey checker of 4-pixel squares for gaps and magenta
stripes (6 pixels along a row) on dark purple for problems, the same at any zoom;
the viewer's slice keeps their size on screen while it traces at a quarter of the
resolution, and its shaders draw the same (`marked` in `prelude.wgsl`). Slices get an
outline where the label changes; flat renders darken each colour by the angle
between ray and normal (`shade`), patterns too. Colours by cell, material and
universe (`hashed`) have every channel at least 64 before shading, and the marks
have none of green, so a script can find marked pixels by green = 0.

Gaps and problems are drawn whatever cells are hidden: they are where no single
cell is. An isolated view (`Hiding::isolate`) draws only those of what it isolates
(`Hiding::problems`, `ProblemScope::Selection` by default; `render --problems all`
and the viewer's "All problems" toggle in the Isolating banner draw every one): the
engine's `Visibility::problem_cells`, the cells isolated and what they hold, without
the envelopes above them. An overlap shows when one of the cells in it is among
them, a gap or too-deep nesting when the filled cell its path ends at is
(`Visibility::owns_problem`; none at level 0, so the model's own gaps are left out).
Rendering a component of a 500 k-cell model drew, before (2026-11-12), the specks of
unrelated overlaps between other cells far from it. The GPU ignores the mask: it
does not detect overlaps, and stops at every gap and unsupported cell.
`llevant render` says how many pixels show gaps and problems, and which
`llevant check --region` lists them.

A `Paint` (model, looks, `ColourBy`, optional field) is what every image is coloured
by: `slice_rgb`, `render_image` and the viewer's palette take one, so a new way of
colouring is one place to change.

## Colour scales

`llevant_render::scale::Scale`: numbers to colours between `lo` and `hi`, linear or
logarithmic, along a `Ramp`. There is one ramp, close to viridis: even to the eye,
readable in grey. `Scale::DENSITY` (10⁻⁴ to 25 g/cm³, log) colours by density.
`Scale::auto` fits a scale to values, from their 1st to 99th percentiles, so a few
outliers do not squeeze the rest into one colour. `ticks` gives the marks of a colour
bar: both ends and, in between, powers of ten or round numbers, which `legend` lays
out as the colour bar under the CLI's images and in the viewer. `lut` gives the
colours for a GPU, which interpolates between them.

## Fields

`llevant_scene::field::Field`: numbers over the geometry; how they are drawn over
the cells is a `llevant_render::field::FieldLook` (a colour scale, fitted to the
values by default, and an opacity), and an `Overlay` pairs the shared data with a
look, so changing the look never copies the numbers. A field is attached to one of
three things:
- **cells**: one value per cell, the same in every instance of it;
- **cell instances** (`engine.md`, "Cell instances"): each copy of a repeated
  universe and each lattice element can differ;
- **a mesh**: rectangular or cylindrical, placed as MCNP's `TR=`
  (`aci_tallies_geom::Geometry`, from the aci-tallies crates), with one
  value per voxel or per voxel and cell (cell under voxel, as D1SUNED and
  R2SUNED give them).

Under a field the cells are a neutral grey (`field::NO_VALUE`, `Under::Neutral`, the
default; `FieldLook::no_value` gives a grey tinted blue, `NO_VALUE_ON_GREY`, on the
grey ramp, which runs through the neutral one; the GPU pass gets it as
`params[1].yzw`): a cell the field has no value for, or a point outside a mesh, is that
grey, and a value's colour goes over it `opacity` of the way (`mix`). So no colour
but the scale's reads as a value, and a lower opacity fades towards grey rather
than shifting the hue towards a cell's random colour (2026-11-10; before, cells
without a value kept their colours). `Under::Geometry` (the viewer's "cell colours
under it", the CLI's `--cell-colours-under`, Python's `with_cell_colours_under`)
keeps the geometry's colours under the field and where it has no value. Values
outside the scale take the colour of its nearer end; only no value is grey. In
renders the field and the grey are shaded flat whatever the colouring. Gaps and
problems keep their colours. The legend shows only the scale: the grey is not named
on it (2026-11-10).
Fields
are read from CSV files and, with the `tallies` feature, from MCNP tally files
(through aci_tallies):
- CSV files (`read_csv`): the first column is `cell`, `material`, `instance` or
  `path` (MCNP's repeated-structure notation); the second holds the values, and
  its header is the name, with the unit in brackets.
- Tally files (`read_tallies`; or `TallyFile::open` once and `tally_field` for each
  tally or bin): the file is indexed first (one parallel pass that keeps where each
  tally and its values are, not the file), then only the tally asked for is read.
  Kept, the index reads another tally or bin of the same file at once. With the
  model's input (`read_input`, parsed once for all the tally files of its runs),
  each file gets what it leaves out.
- Meshtal files: one FMESH tally at one energy and one time bin, on its mesh. With
  the input, the meshes are the exact ones of their FMESH cards, `TR=` included.
  Only that bin's rows are read and parsed.
- MCTAL files: one `F` cell tally (F4, F6, F7, F8, with `*` or `+`), at one bin
  along each dimension but its cells (`TallyChoice`: energy, time and `other` for
  d, u, s, m, c; `Total`, or the only bin, by default, and an error naming the
  dimension where there are several and no Total; `Sum` adds the bins up, errors in
  quadrature as if independent, and is the Total where there is one). With the input, each bin is what
  its `F` card makes (every level, groups, lattice elements, the T bin); without
  it, only the first cell of each bin is known, and a cell in two bins (chains the
  file can't show) is an error.
  - Every bin plain cells, none filled: a field per cell (a group `(1 2)` gives each
    of its cells the group's value; a single cell wins over a group holding it).
  - Otherwise a field per instance, from the engine's `Instances::innermost_bins`
    (`engine.md`, "Cell instances"): a bin holds the instances of its cells and,
    for a filled cell, everything inside it (an F4 on an envelope scores all it
    holds); where bins overlap, the one whose cell is deepest gives the value, then
    the one of more levels, then a single cell rather than a union, then the first.
    So in `probe.mcnp`'s F44, `(8 < 7[1 0 0] < 3)` colours its piece of cell 8 and
    `(8 < 3)` the other pieces. The T bin is never drawn (it would colour all its
    cells alike).
  - Surface tallies, detectors and TMESH mesh tallies are an error ("not drawn
    yet"). The tally by default is the first that can be drawn as it is: a Total (or
    one bin) along every axis but its cells, and some cells the file or input names
    (else the first that can be drawn, whose error then says what to choose).
    MCTAL writes 0 for a bin that is a group, levels or the T bin; without the
    input's F card such bins are left out, so a tally of nothing but those draws
    nothing.
  - Units per source particle: `1/cm²` (F4), `MeV/cm²` (`*F4`), `MeV/g` (F6, F7,
    `+F6`), `jerks/g` (`*F6`), `MeV` (`*F8`); none for relative errors or with
    several `FM` multiplier bins. A one-bin `FM` card goes unnoticed (MCTAL writes
    `m 0` and `m 1` alike), so its unit is the plain tally's.
  - `tests/fixtures/tallies/mctal/probe.mcnp` checks this against MCNP: every
    history is one ray through void, so each bin is its chord over its volume, and
    the field along the ray must be the value of the bin holding each point
    (`llevant-scene/tests/tallies.rs`).

`TallyFile::summary` describes an indexed file without reading values, for
interfaces to choose from: the run (code, title, histories), each tally (number,
kind as the input writes it, `+F6:n,p`, comment, number of cell bins or voxels,
why it can't be drawn, notes such as bins only the run's input names, warnings),
and each axis with several bins, with a label per bin (`0.1–1 MeV`, `m 2`) and
whether it has a Total; `default` is the tally read when none is chosen.
`TallySummary::choice` is a tally at its Totals, or its first bins where there are
none. An MCTAL cell tally's `bins` say what each cell bin is: its name as MCNP's
output gives it (`(8<7[1 0 0]<3)`, `T`; `?` when only the input names it), the cells
it scores (its first level), and whether it scores every instance of them (not so
for chains). `tally_field_and_bins` reads the field and, for MCTAL, each bin's result
and relative error at the bins chosen along the others.

`field::read` tells CSV and tally files apart by the first 64 kB (`tally_format`).
A zero tally result (nothing scored) is a value, 0: on a log scale it falls below
the scale and takes its low end's colour, like any value below it, and the scale is
fitted to the positive values (still log when they span two decades). Only its
relative error, which MCNP writes as 0, is no value (2026-11-10; zeros were no value
before, grey like cells the tally does not cover). Values below what `f32` holds
(1e-38) are kept as 0.

Slice outlines (`colours::Outlines`, `slice_rgb`; the CLI's `--outline-by`, Python's
`outline_by`, the viewer's slice Display) are drawn where the label changes, or where
the material or universe of the deepest cell does (`Outlines::classes`; labels
outside cells, and every label at a level shown, count as their own class).

The CLI's `slice` and `render` take `--field FILE` with `--tally`, `--energy`,
`--time`, `--bin d=0` (repeatable, MCTAL; `m=sum` adds a dimension's bins up), `--error`,
`--run-input` (the MCNP input of the run, when it is not the model's), `--scale lo,hi`, `--log`/`--linear`, `--opacity` and `--cell-colours-under`, and put a
colour bar under the image. Python has `llevant.Field` (`cells`, `materials`,
`instances`, `mesh`, `read`) and `field=` on `slice` and `render`. The viewer has
the Fields panel (below). On the GPU, the viewer's `field` pass draws what
`render_image` draws: `tests/images.rs` checks that on cells, instances, a turned
rectangular mesh, a cylindrical mesh and cell under voxel. CPU and GPU agree on
every pixel per cell and per instance. On meshes, at most 12 pixels of a view
differ, at voxel edges.

What fields do not do (yet): hiding or selecting by value, per-instance hiding,
outlines along voxel boundaries in slices, and fields in a universe opened on its
own (its cells and frame are not the model's).

## Hiding

`llevant_scene::hiding::Hiding` holds what a 3D view hides, for the CLI (`render
--hide, --hide-universe, --hide-material, --show-void`), the Python bindings
(`hide`, `hide_universes`, `hide_materials`, `show_void`) and the viewer (its
`Scene`, the `hide_void`/`hidden_*` keys of the view file). `Hiding::visibility`
turns it into the engine's per-cell `Visibility`, where a hidden filled cell hides
everything it holds. Its rules are of two kinds:

- **Containers**: cells and universes by number, hidden with what fills them.
- **Contents**: void cells and cells by material, about what a ray would show. They
  never hide a filled cell: what shows there is what fills it, whatever material
  the card gives (hiding material 0 would otherwise hide every void envelope and
  so nearly the whole model).

Two more fields see the levels of a model, for every frontend (`render --level
--ghost`, `slice --level`; `level=`, `ghost=` in Python; the viewer's level menu and
"Ghost" items; `level` and `ghost` in the view file):

- **`level`**: the universe level shown, as MCNP's plotter shows `LEVEL n` (probed on
  MCNP, [`mcnp-behaviour.md`](../mcnp-behaviour.md)): only cells at that level, a filled one drawn
  as a solid cell (its card's material, void white by material, as MCNP colours it),
  a lattice cell with its element boundaries. Above it, cells that are not filled are
  blank in slices (`Paint::blank`, no outlines between blank labels) and see-through
  in 3D. Level 0 is the envelope structure on its own, to check it for gaps and
  mistakes whatever fills it.
- **`ghost`**: rules whose cells are drawn as shells over what lies behind them, what
  they hold still shown: an envelope over its filler, to see how one fills the other.
  A ghost cell is shown whatever the other rules say of it. The shell
  (`llevant_render::ghost`) is the first ghost boundary each ray crossed, in its
  cell's colour shaded by its normal, with opacity 0.16 face on, up to 0.71 where the
  ray grazes it, compounded over every boundary crossed before the hit; drawn over
  the finished image (after the field and effects).
- **`see_through`** (`see_through_gaps` in the view file, off by default;
  `render --see-through-gaps`): gaps, and on the CPU overlaps, seen through
  as veils (`Visibility::veil`): where a ray went through one, the gaps' checker
  (the problems' stripes where the CPU went through an overlap) half over what it
  sees beyond (`colours::veiled`, opacity 0.5, flat, before the field). A solid hole
  hid everything behind it; with the veil, a gap in front of a component shows
  both. Per ray this is two bits of the hit (`Hits::veils`; on the GPU bits 14 and
  15 of the status word, depth keeping 6), so it costs no memory or pass of its
  own: the ghost record (4 words a pixel and the ghost pass) stays for ghost cells.

New rules (by density, importance; "show only") belong here, so every frontend gets
them. The flags are per cell, not per instance: a cell of a universe that fills
several places is shown or hidden in all of them.

## Realistic images

`ColourBy::Realistic` colours a cell by what its material is made of
(`llevant_scene::materials`). Each `Mn` card is turned into element mass fractions
and split into parts: water (H and O in ratio), oxides (left-over oxygen handed to
cations, the most electropositive first: concrete with Ca, ceramic without; iron oxide
darkens it), polymer (H and C, told from graphite by the H before water took its
share), then metals (steel, copper, tungsten, lead, light metal; B/C/Si/P/S under a
tenth of the metals are alloying elements); noble gases and cells under 0.02 g/cm³
are gas. The water may instead be bound: in a polymer at least as heavy (epoxy,
cellulose) or a mineral at least as heavy (hydration; with carbon in a mineral 4×
heavier, carbonate). In a mineral 4× heavier than water and polymer (concrete) it is
always bound; otherwise the median density of the material's cells picks the reading
whose parts at nominal densities come nearer (`analyse_at`), bound in a polymer when
no cell gives a density. Parts are weighed by volume (mass over a nominal density,
minerals heavier with iron oxide) and their `Appearance`s (linear albedo, roughness,
metalness, clarity) blended, water and gas counting less.

On the reference model's 742 cards: steel for 434 (157 k cells), copper 99, water
mixtures 46, concrete 43 (all 100 % concrete, heavy ones included), and the organic
insulators (M201, M250–M254, M110801...) polymer with their glass filler.

`Spec`s override the heuristic per material (`12 copper roughness=0.2`).
`Looks::new(model, specs)` classifies every material (22 ms on the reference model)
and is owned by the caller: the CLI builds it per command, the viewer per document,
Python per `Model` (replaced by `set_materials`). Functions that colour realistically
take it as an argument.

### Shading

Realistic colours are lit physically, the others flat (no separate choice):
Cook-Torrance GGX + Lambert under a key and a fill light fixed to the view and a
sky-to-ground environment whose horizon sharpens with smoothness (split-sum with
Karis's analytic fit, no lookup tables), ACES, sRGB. Clear materials (clarity 1;
water has 0.5) are a dielectric with F0 = 0.02 over a depth that scatters a
blue-green albedo back (sky and key light, no n·l), blended by clarity with the
opaque model. It is a function of each pixel's hit only: the viewer's
`colour.wgsl` mirrors `llevant_render::shading` in f32, and the viewer test checks
the two agree to 2 levels on every pixel (also with water and a half-clear material);
the colour pass costs +0.9 ms at 1080p on llvmpipe. Roughness, metalness and clarity
go to the GPU as a second palette word, 8-bit, and the CPU quantises them the same
way.

## GPU renderer

Two evaluators of one compiled model (decided 2026-10-08):

- **CPU engine: the reference.** Classification, tracing, volumes, checks,
  fingerprints, 2D slices, and `first_hits` for picking, tests and static 3D renders.
  f64, exact up to the stated tolerance, complete, deterministic.
- **GPU renderer (`llevant-gpu`): the 3D view only.** Approximate by design: f32
  (~0.5 µm at 50 m), a pixel may be wrong at an edge. Target 60 frames/s at
  1920 × 1080, about 1.2·10⁸ rays/s; CPU `first_hits` does 6·10⁵ rays/s on 96 threads,
  so no CPU optimisation closes the gap ([decision
  0001](../decisions/0001-cpu-exact-gpu-approximate.md)).

Why the engine stays on the CPU: volume work needs f64 (20 m models, millimetre
features, tolerances around 1e-5 cm, cancellation in near-tangent intersections), and
consumer GPUs run f64 at 1/32–1/64 of f32; work per line varies (cells, program
lengths, nesting, crossings sorted per line), so GPU threads would diverge; CI runners
and many user machines have no GPU; and CPU speed is enough for QA.

Rules for the split:

- **One compiler, two evaluators.** The renderer reads `Model::export()` (flat arrays
  of shapes, programs, node boxes, cells, units, BVH and octree nodes, fill targets
  and lattices, f64 as compiled), never MCNP input: MCNP semantics live in one place.
  Octree candidate lists are conservative supersets proven in f64, so the f32
  renderer uses them as they are.
- **Exact answers come from the CPU.** Picking and overlays of gaps, overlaps or
  version differences are CPU queries drawn on top.
- **The CPU engine tests the renderer**: GPU hits against CPU `first_hits`.
- **CPU `first_hits` gets only cheap improvements**; neighbour lists belong to the
  renderer.

### How it traces

Packing for the GPU rounds boxes outward to f32 and puts everything in one pool of
words, split into as many storage bindings as the device's per-binding limit needs
(lavapipe and WebGPU: 128 MB; the reference model took 417 MB in 4 before the
octrees' residual programs, 1.45 GB now). `GpuModel::upload` streams the pool into
its buffers through the queue in 16 MB pieces, waiting every 256 MB, from the
model's lean export (`Model::export_lean`: the programs and octree arrays are handed
out from the model itself instead of copied): the viewer's host memory peaks at
5.3 GB instead of 10.4 GB for the reference model, and GPU memory at 0.13 GB over
what the model takes instead of 1.45 GB. Running out of GPU memory there is an
error (`push_error_scope`), so callers trace on the CPU instead. WARP, Windows' software
adapter, is refused unless `WGPU_ADAPTER_NAME` names it (`is_warp`): the shader compiles
there with DXC, but renders never finish (seen on GitHub's runners, cause unknown), and
the viewer then traces on the CPU.

The crate's modules follow that path: `device` (the device and adapter, reading
buffers back), `pack` (the upload), `visibility`, `job` (parameters, jobs traced a slice
at a time, explicit rays and camera images) and `hits` (what the shader writes per ray
and its decoding). Each `GpuModel::set_visibility` uploads a new visibility buffer
instead of writing over the current one: a job binds the visibility current when it is
made and reads it over all its slices, so a job in flight never sees a visibility set
for a newer camera or scene (the old buffer is freed with the last job that reads it).

The shader source is the same for every model but for the depth of its programs and
BVHs (`CODE_STACK`, `BVH_STACK`, which size private stacks), its levels (`MAXL`, which
sizes the per-level state, at most `MAX_LEVELS`; every private word costs every ray, and
4 levels instead of 8 made the reference model's views 6–41 % faster, [decision
0015](../decisions/0015-gpu-state-per-model-and-timing.md)) and the number of pool
bindings, rounded up to a power of two (spare ones bound to a word): where the parts of
the visibility buffer start are parameters (`P.vis_*`). A staged model, the same
completed and most edits of it so share the shader (`GpuModel::shader_id`,
`tests/upload.rs`), which the driver compiles once and then caches (8–30 s for a new
one on the reference model). Stacks rounded up to 16 were 6–7 % slower per frame, so
their sizes stay exact.

Per ray (`src/shader/*.wgsl`, joined in order into one module; no neighbour lists yet): locate the start through the
levels (octree leaf or BVH, the first candidate whose program is true; lattices by
index); while the path is hidden, take for each level its cell's exit (below) or its
lattice element's faces, step past the nearest, and locate again from the highest
level whose exit the step passed. Levels above keep their cells and exits.

A cell's exit is where its program can first turn false along the ray (`exit_bound`,
2026-11-14), not the first crossing of any of its surfaces. A leaf keeps its side
from the ray's position to its next crossing, so one true there (tested at the
midpoint, away from both ends) is true until that crossing; an AND of such until its
first, an OR with one until its last. That is never past the true exit, so no cell is
skipped. A ray then passes the parts a CAD void is cut around without stopping at
their planes. Those stops were 81–85 % of the steps of the reference model's slowest
rays, each one a relocation and another walk of the void's program. When the program
is not surely true (a crossing within the tolerance of the start), the exit is the
first crossing, as before. Node boxes the stretch misses still prune whole subtrees,
as in `trace`: a node constant along it is true or false all the way. The per-node
state is one `f32` (with "not sure" as −10³⁰ an OR is a max and an AND a min) plus a
surface kept in the BVH stack, which is idle while an exit is found: every private
word costs every ray of a dispatch, and four words a node made views that never step
~20 % slower. On the reference model, 1920 × 1080 (`gpu-ab`, 11 rounds): the whole
model 246 → 98 ms, views that never step within +1–3 % of before. Hits and cells
agree with the CPU as before (`gpu-check`, every view), with fewer normals off. The
override constant `EXIT_BOUND` turns it off for A/B runs; `tests/vs_cpu.rs`
(`void_exits_skip_crossings_that_keep_the_cell`) holds it.
A ray stops at a visible cell, a gap or a problem; overlaps are not detected (the
first candidate wins).

Hidden space is skipped (2026-10-25): the renderer is permissive, for interactivity,
and the CPU engine stays the exact reference. With each visibility the engine works out
what is *worth entering* (`Model::skip`: a visible cell that is not filled, or whose
universe or some lattice element has a cell worth entering) and, per universe, the box
of those cells and a bit per octree node, set where a leaf below holds one
(`Model::export_skip`, uploaded after the cell bits). When the path ends in a hidden
cell, its level first tries to jump: beyond that box, to near the ray's entry into it
(or to a miss); in the octree, across up to 256 nodes that hold nothing worth entering,
up to where a level above exits. A jump stops two steps short of the face it reached,
so the visible cell after it is entered through its own surface and keeps its normal.
Where the located leaf may hold something visible, nothing changes (one bit tested).
Gaps and overlaps in what is jumped are not seen. With one envelope of the reference
model kept, 1080p frames take 12–28 ms instead of 0.3–1.7 s; solid views are unchanged
(2026-10-25). `USE_SKIP` turns it off for A/B runs.

In an octree the kernel uses the engine's residual programs (`Export::grid_progs`,
each candidate's program specialized to its leaf): candidates are evaluated with them,
and a cell's exit walks the leaves along the ray with the cell's residual in each,
until one of its surfaces is crossed, the cell has no residual in a leaf (exit at the
leaf face, then relocate), or `MAX_LEAVES` (64) leaves are walked (the rest of the
exit from the whole program: one long jump, as across the void around a model, where
leaves are many and small). A leaf exit never steps back, whatever f32 rounding says
about the leaf's faces. Exits that stopped at every leaf face, with the next leaf
found by relocating, cost 1.5–2.7× more ([decision
0002](../decisions/0002-residual-programs.md)). `USE_RESIDUALS` and `MAX_LEAVES` are override constants;
`GpuModel::set_constants` rebuilds the pipelines with other values, which `llevant-bench gpu-ab`
uses to compare variants in one process: on a laptop GPU, frame times of separate
runs drift by up to 40 % with heat and the display, more than most differences
between variants. It times frames back to back after a warm-up and prints the clock:
frames read back one at a time leave the GPU idle long enough for a driver to hold it
at a third of its clock ([decision 0015](../decisions/0015-gpu-state-per-model-and-timing.md)).

A ray also gives up (`Status::Steps`, drawn neutral grey) after `max_steps` locate
steps or after walking `max_work` program words (`GpuModel::max_work`, default 2¹⁶),
whichever comes first. The word budget bounds the time of one ray whatever the model:
in void-heavy views (`envelope_structure.mcnp` with only its 161 material cells shown)
a step only reached the next crossing of any surface of a void cell's program (up to
4 600 words; before the exit bound), so rays took thousands of steps and one ray ran for seconds (then one
dispatch; Windows resets a GPU that does not answer for ~2 s). One ray at the budget takes ≤ ~0.5 s on an RTX
3060 Laptop (a step can overshoot it by one locate and exit).

Rays are traced in slices (2026-10-28), so that no dispatch runs long whatever the
rays: a `Job` (`GpuModel::camera_job`; `render` and `trace_rays` run one to its end)
goes by blocks of 64 rays (8 × 8 pixels, in an order the caller gives), one block per
workgroup of 64 threads. In each slice every ray in flight walks at most
`GpuModel::slice_work` program words (default 256, checked between steps; up to 64
times more when fewer than 32 768 rays are in flight, where the GPU waits on single
rays rather than on their number), then pauses: the trace loop's state (20 words: position, crossing, surface and level last
crossed, steps, work, the cut part, the ghost record) goes to its lane's slot, and on
resuming the ray locates itself again from level 0 (at no cost to its `max_work`, so
that slicing does not change where a ray gives up). A ray keeps its lane from slice to
slice, because rays of a block take alike paths. Compacting the paused rays into
fewer warps mixed rays from all over the image, and warps then ran their lanes' paths
one after another. A slice runs only the workgroups with rays left (an indirect
dispatch that a one-thread `plan` dispatch sizes). A workgroup whose rays all ended
takes the job's next block, so the slowest rays of one part of an image run
alongside the next part, at most `GpuModel::capacity` rays (2¹⁷) in flight. A
counter per block (64 rays, those past the image's edge too) says when its pixels are
done (`Progress::blocks_left`). The GPU switches between devices
at dispatch boundaries, so short dispatches let a window's frames through within a
few milliseconds. One long dispatch held them for up to 0.4 s, and wgpu loses a
window's device once a frame waits a full second ([decision 0005](../decisions/0005-sliced-gpu-dispatches.md)).
Slices of the reference model take ~3–25 ms, up to ~50 ms at a job's tail. Frames take
about as long as in one dispatch when rays are short (solid views, +2 %) and less when
hidden cells make them long (25–40 % less with copper alone, in three views of four), except
where relocating each resumed ray is a large part of short slices (copper close-up,
+20 %). Sliced and whole traces agree up to an f32 rounding of single pixels: locating
again from level 0 can settle a point within `eps` of a boundary differently
(`llevant-bench gpu-check` on the reference model: the same at the default slices, one pixel in
22 500 in three views with 16-word slices).

Levels and ghosts come with the visibility, after its skip words: a bit per ghost
cell and three words, the level shown, whether there are ghosts and whether gaps
are veils. At the level shown
`locate` stops at a filled cell (a lattice cell after finding its element) and a cell
at another level, or a ghost, is located as hidden, so rays step on through it; a
visible lattice cell at the bottom then needs its element's exit, as the next element
may show. A ghost boundary is a change of the innermost ghost cell on the path after a
step: the first one's cell, `t` and normal and their count go to a ghost record
(4 words: count, cell, `t`, normal as two snorm16 octahedral), written after all the
hits of the buffer when `Params::ghost_out` says where (`HitBuffer::with_ghosts`),
never without ghosts. Without levels and ghosts the kernel pays one comparison per
filled cell entered and one uniform test per step (frame times as before).

A veiled gap (2026-11-11) is stepped through as a hidden stretch, and sets a flag
written with the ray's hit (bit 15 of the status word; a resumed ray keeps it in its
state). A first version counted veils in the ghost record, which made every frame
carry and resolve 4 more words a pixel with the veil on, whether or not there were
gaps: a bit is enough to draw the pattern. A gap at a
lattice element out of the fill's range leaves at the element's exit; a gap where no
cell of a unit holds the point leaves at `gap_exit`: the first crossing of a surface
of any cell that could hold a point further on (the candidates of the octree leaf,
with their residual programs, up to the leaf's exit; the BVH items whose boxes the
ray passes; beyond the unit's region every cell of its list; the cells checked
everywhere), the entry into an unsupported cell's box, or the region's or octree's
boundary, since only there can the point's cell change. `tests/vs_cpu.rs`
(`gaps_seen_through`, `tests/gaps.i`: holes at level 0, in a filled universe and in
lattice elements, with octrees and with BVHs) holds it to the CPU. Gaps in what a
skip jumps are not seen, veiled or not: there the void around them is hidden, and
the jump is what makes hidden space cheap.

f32 needed these, each found by comparing against CPU `first_hits`:

- The step past a crossing is `eps / |cos θ|` (θ between ray and normal, capped at
  20 `eps`; `eps = 1e-4 cm + 1e-6 t`) so a grazing ray ends up `eps` off the surface,
  and at least 4× the root's own f32 error, estimated from the size of the terms that
  cancel and the slope at the root.
- Quadrics are rewritten (exactly, in f64) about the point of their cells' region
  nearest their stored origin: GEOUNED's GQ cylinders are stored about points metres
  away, and f32 values there were off by ~1e-3 cm. Cells with boxes over 100 m are
  left out of the region, or the void around everything pulls the origin away again.
- Tori: the quartic of the CPU loses ~5e-3 cm in f32 for a thin tube on a large ring
  (and the root altogether for 1.8 cm on 857 cm), so roots are refined on the torus
  function itself, and thin tori (ring > 8× tube) skip the quartic: the ray is
  clipped to the shell around the ring (two cylinders and a slab, well-conditioned
  quadratics), where the function has one minimum per piece.
- The reported `t` is the crossing, not the stepped point.
- A cell's exit looks for surface crossings a step past its box's exit: a surface on
  a box face (a cylinder's end plane) rounds to either side of it in f32, and an exit
  taken at the box face has no surface, so the hit had no normal and rendered
  unshaded.

Render options match the CPU's `Model::render`: hidden cells and universes and void
through `Visibility`'s cell flags, colour modes, orthographic cameras, and the
camera's cut (up to 8 regions of 16 planes in all, the eye's distance from each
plane worked out in f64; the shader walks the kept parts as `Cutaway::kept`, with
no local arrays (FXC, D3D12's old compiler, unrolls loops over them), recomputing each region's interval
along the ray; a hit at the start of a part takes the cut face's normal). Not on the GPU:
`Visibility::gaps` and `problems` (a gap or problem always stops a ray where it steps,
and is never seen where it jumps; the CPU can hide them only because `first_hits`
traces every surface).

Agreement with CPU `first_hits` (`llevant-bench gpu-check`, 150 × 150 rays per view, three views,
every cell / solid cells / a random half of the solid cells visible): 100 % on the zoo
and synthetic models up to single pixels at shared faces or slivers thinner than a
step; 99.93–100 % per view on the reference model. Of the same hits, ≤ 0.2 % per view
have another normal than the CPU (edges). Tests (`tests/vs_cpu.rs`) require 99.5 % on
the zoo models, normals on 99.5 % of those, whole cutaway frames compared pixel by
pixel, and skip without an adapter.

Speed: [`benchmarks.md`](../benchmarks.md) (upload and a 1920 × 1080 frame per model).
Next: neighbour lists for interior views. The latency of single rays
(~1 ms per locate step on the reference model) still bounds how fast a GPU preview can
be.

### Drafts patched in place

`GpuModel::patch(gpu, &DraftPatch)` (`patch.rs`; [decision
0011](../decisions/0011-live-drag-drafts.md)) writes a step of a `llevant::draft::Draft` into
the model uploaded from the draft's base.

- **What it writes.**
  - Shapes, recentred and cast to f32 as at upload (`pack::recentre`).
  - Cell, node, BVH and unit boxes, rounded out.
  - Units' octree roots and always lists.
  - Octree leaf programs.
- **How.** The writes are sorted, contiguous runs merged, and the runs copied from one
  staging buffer in one submission. Many small queue writes cost ~2 µs each, and a
  drag on the reference model writes tens of thousands.
- **Always lists.** A unit whose always list grows takes it from a slack area of
  `SLACK_WORDS` reserved at the end of the pool at upload. Nothing else moves, and the
  shader and its id stay the same.
- **When.** A job already under way would see the patch from its next slice, so the
  viewer writes patches on the trace thread between jobs.
- **Tested by** `tests/draft.rs`: patched drafts against `check::compare` of a fresh
  compile, with the skip widened (`Draft::widen_skip`); the revert patch gives back
  the base; the shader id is unchanged.

## 2D slices

A slice is a set of parallel lines in a plane: each image row is one `trace` call.
Boundaries land at their exact positions, independent of resolution; zooming in only
traces new lines. Cost scales with rows (and how much each row crosses), not columns:
on the reference model, 1000 rows of the whole model take 41 / 72 / 221 / 800 ms on
96 / 16 / 4 / 1 threads, a zoom onto one envelope 20 / 17 / 54 / 194 ms. So the
viewer traces progressive rows (a quarter first), and while the view moves it keeps
every column and traces every k-th row, k the smallest of 1, 2 and 4 whose rows fit
30 ms at the last complete trace's time per row, chosen when the view starts moving
and kept until it stops (chosen per trace, timings near the limit flipped it and thin
layers flashed). Panning the reference model, most moving traces are the whole view,
~18 ms, where every one was a quarter of the rows and columns (2026-11-14).

At a level (`Model::slice` with `Depth::Level`), rows are traced down to it only: level 0 of the
reference model's whole slice costs what level 0 costs, nothing below.

**Labels** (`llevant_render::labels`, 2026-10-31), as MCNP's plotter writes them:
cell numbers at the point of each connected region farthest from its border (a
chamfer distance transform, then a fill per region), surface numbers on the borders.
Borders are grouped by the pair of regions they separate and by connection, sampled
every `spacing` pixels, and each sample is a short line traced across, all in one
batch; each surface met in a group is labelled once, at its middle sample, named by
`bounding_surface`. A border made of several surfaces (a corner of a box) gets one
label per surface. Images larger than 300 k pixels are looked at on every 2nd or 3rd
pixel (labels need no pixel's precision): 12–30 ms for a 1920×1080 slice of the
reference model, against 11–170 ms to trace it. `place` keeps, greedily, the labels
whose boxes do not overlap (at most 400 in the viewer).

Not done yet: overlays of version differences.

## The viewer

`crates/llevant-viewer` (2026-10-10; usage and the view file in
[`reference/viewer.md`](../reference/viewer.md); embedding, effects and how the
application is put together in its [README](https://github.com/Fusion4Energy/llevant/blob/main/crates/llevant-viewer/README.md)):
an egui/eframe 0.36 desktop app on wgpu 30 (as `llevant-gpu`): the 3D view and a 2D
slice side by side, tools (select, pin, section, mesh), picking (exact, CPU), pins with banners,
undo, visibility, clip plane, colouring, effects, a view state as JSON (`--state`,
"Save view"). Library and binary: the views are egui widgets (`view3d::View3d`,
`slice::SliceView`) other programs can embed, and `ViewState` is what other tools pass
to open a model at a view. The CSG is never tessellated (marching cubes loses thin
features; exact trimmed tessellation of 10⁶ quadric cells is a project in itself):
every pixel is a ray.

- **3D**: rays are traced in batches a scheduler (`view3d/budget.rs`) makes: while
  the camera moves, the whole view at a reduced scale, sized from a cost
  `fixed + per_ray · rays` whose fixed part is the fastest preview since the costs were
  last reset (mostly the latency of its slowest rays) and whose per-ray part is the
  slope of the samples about it; when it stops, the whole view at full resolution in
  one batch, its blocks ordered from the centre out and shown as they land. Hits land
  in a G-buffer (`resolve.wgsl`: low image upscaled, refined 8×8 blocks from the full
  one; nine words per hit, the last the cell instance), and a chain of compute passes
  makes the image (`passes.rs`): the base colours as `llevant render`, the field (off
  without one), then outlines and ambient occlusion; then, after the effects,
  ghost shells (`ghost.wgsl`, when the visibility has ghosts:
  their records follow the hits in the hit buffers and the G-buffer, read back and
  resolved only then) and the selection. An upscaled pixel takes the distance along its own
  ray to the tangent plane of its low hit (moved at most 5 times the gap between that
  hit and its ray): with the low hit's distance as it is, each low pixel was a terrace
  a fixed distance from the eye, and ambient occlusion shaded the step at its edges
  (2026-11-10; no measurable cost, the pass is bound by memory). A pass is WGSL defining
  `fn effect(p) -> vec4<f32>` over a prelude of hit accessors
  (`shaders/prelude.wgsl`), so other tools add effects without touching tracing. A
  pass may also read storage buffers of its own (`PostPass::data_buffers`, bound at
  `@group(1)`, filled with `PassChain::set_data`), as the field pass does.
- **Backends** (2026-10-31): GPU (`llevant-gpu` on a device of its own, hits read
  back) when there is a hardware GPU, else CPU (`pixel_hits`, exact). The CPU+GPU
  hybrid (previews on the CPU) is gone: with reprojection the window no longer waits
  for previews, and on the reference model GPU-only orbited at 19 ms (95 %) against
  25 ms with 16 CPU threads tracing beside the UI, and refined a still view in 0.7 s
  against 2.4 s (12 previews a second against 26, which reprojection covers; now 20
  a second, [decision 0013](../decisions/0013-viewer-tracing-latency.md)). CPU
  previews are permissive like the GPU renderer: they hide gaps and problems and
  search only where the visibility's `Model::skip` leaves something to see
  (`Model::pixel_hits` with a skip, exact for that visibility); full-resolution tiles
  on the CPU and picking keep the scene's visibility. The GPU backend makes each
  batch's job and readback on its own thread (several milliseconds on big images).
  Both backends take one batch at a time, so a full-resolution image never starts in
  the frame its preview is sent, for a camera that has moved on by the next one; a
  restart gives up full-resolution batches of older generations
  (`TraceBackend::cancel_full_before`), while a preview in flight still lands (an
  older preview is shown, moved to the camera). The GPU backend traces each batch as
  one `Job` (all its tiles' blocks), ~20 ms of slices per submission, reading the
  job's progress in between; the blocks done by one submission are read back with
  the next (`Landings`) and land as `Finished` pieces (`more`), so a still view shows
  as it is traced. It gives a batch up between submissions once a later camera's is
  submitted or the batch is cancelled. Previews bound each ray's work
  (`GpuBackend::preview_work`, 2048 program words); a ray that gives up there takes
  the hit of the nearest ray that did not, up to 16 preview pixels away, in
  `resolve.wgsl`. Every slice stays as short as the
  first ones (`backend::beside_window`: `tail_work` = `slice_work`), so the window's
  frames wait a few milliseconds behind tracing at most. GPU time is the sum of the
  submissions' timestamps. Previews are capped (`MAX_BATCH_SECONDS`) beyond their fixed
  part only: the slowest rays' latency is paid whatever the batch's size. The CPU
  backend lands a batch tile by tile (64 × 64 at full resolution). A lost trace device (`TraceBackend::lost`), or any
  error on it (out of memory for a batch's buffers; the device has its own handler
  instead of wgpu's panic), switches the view to the CPU backend with a notice that
  says why; a lost window device makes egui panic, which `run` turns into an error.
- **Reprojection** (2026-10-31, `view3d/reproject.rs`; [decision
  0006](../decisions/0006-reprojection-gpu-only.md)): every frame while the camera
  moves, the hits shown are moved to where the camera is and shaded from there. Each
  pixel's point (its hit, its first ghost boundary, or its ray's end where it hit
  nothing) goes to the pixel it falls in now, the nearest winning (`atomicMin` on its
  depth); a pixel no point fell in takes its nearest neighbour's; distances are
  measured again along the new rays. New traces replace what was moved as they land.
  On the zoo models a 3° turn differs from a fresh trace at 0.7–1.8 % of pixels
  (`tests/images.rs`); `llevant-viewer --no-reproject` turns it off (A/B).
- **Application** (2026-10-14): one `Document` (model, looks, scene, selection, the
  two views). What is shown is a `Scene` (colours, hidden cells, clip, effects,
  materials, pins), changed only by `Edit`s, which a `History` undoes as whole
  scenes; the document then updates only what the change touches (a new `Looks`
  recolours both views without tracing the 3D view again; new visibility re-traces).
  Panels and tools read the document and send commands, run after the frame is laid
  out. The left button belongs to the active `Tool`; navigation works whatever the
  tool. Tools and overlays see either view as a `ViewPort` (projection of model
  points, exact pick under a position), so a tool works in both. Overlays (pins
  with banners, selection, the point a drag orbits about) are drawn every frame over the traced image, so
  they stay at 60 fps however long tracing takes.
- **Runtime** (2026-10-31): the UI thread does no model-sized work (`architecture.md`;
  [decision 0008](../decisions/0008-ui-thread-rule.md)). An edit is checked on the UI thread (known numbers, valid specs, the new
  visibility) and recorded at once; the deriver (`derive.rs`, a thread per document)
  makes what the views need of it (looks, the palette of every cell, the
  visibility's skip and GPU buffer, the field's buffer when its data changed) and
  the document swaps them in when they land. Requests queued meanwhile are merged,
  so an edit storm costs one derivation. A field's data is uploaded once per data
  and colour ramp (`view3d::field::DataKey`); scale and opacity are pass
  parameters. Other work runs as `tasks::Task`s (loading, the input's cards,
  opening a universe, reading a field, the analyses, the Statistics numbers): a
  `Progress` to show and cancel, values on a channel polled once a frame, the
  window woken when one lands; dropping a task cancels it, so a new one of a kind
  stops the one before. A model opened from nothing is shown before its GPU copy
  is made (`app::loading::How::gpu_now`; seconds of shader compiling whatever the
  model): the document makes it as a task when its 3D view wants the GPU
  (`Document::poll_gpu`), traces on the CPU meanwhile, hands the copy to the deriver
  (`Deriver::set_gpu`, which uploads the visibility for it), then switches. Devices
  are made one at a time (`loading::UPLOADING`: several at once can hang Vulkan).
  The renderer's diagnostic variants (`debug_steps`, `render_stats`) compile on
  first use. Hover picks run on a latest-wins worker
  (`tasks::Latest`), the 3D view's pipelines and palette are made on the loading
  thread (`view3d::Prepared3d`), and the slice composes its image on a pool of
  the UI's own. A surface dragged in the inspector moves a `llevant::draft::Draft` on a
  worker of its own (`app/drag.rs`, latest step wins), whose patches the GPU backend
  writes between jobs and whose snapshots the slice traces; the release compiles the
  card as any edit. On the reference model, `llevant-viewer --storm` (panels,
  hide/show, isolate, levels, colouring, selection, palette typing) has no frame
  over 8 ms of the UI thread's own work (it had 104 ms frames, from the Statistics
  panel). What remains over 8 ms is the first second of a refinement: submitting
  and landing GPU batches (up to 10 ms a frame).
- **Selection** (2026-10-18): a set of cells given by a rule
  (`llevant_scene::query::Query`: cells, universes, materials, the cells a
  universe fills, what is inside others, and, or, not), resolved over a
  `ModelIndex` (`llevant_scene::index`: cells by material and universe, who fills
  what, instance counts, the cells above and below a set) built with the model on
  the loading thread. Selecting never traces: the 3D view's last pass
  (`highlight.wgsl`, after the effects) tints and outlines the
  cells of a bit-per-cell buffer and outlines the cell under the pointer; the slice
  does the same from the cell id it keeps per pixel. Hiding takes rules too
  (`Hiding::rules`) and isolating (`Hiding::isolate`) keeps a rule's cells, what they
  hold and the filled cells above them, without which rays would not reach them.
  Links to cells, universes and materials everywhere (`context.rs`), a right-click
  menu in both views, H / Shift+H / Alt+H, G / Alt+G (ghost the selection, no
  ghosts), and a selection history (Alt+←/→).
  Navigation (CAD style since 2026-11-06, decision 0012): a middle drag orbits
  about the point under the cursor where it starts, Shift+middle or right pans;
  the left button is the tools' (Select's box: up to 64² rays in one
  `first_hits`, or points classified in the slice, once on release); the axes in
  the corner snap the view. `camera::CameraHistory` keeps the places the camera
  rested at (0.6 s) for Alt+← and Alt+→; the cuts' handles are sized
  from the view's height at the point looked at (`tools::handle_size`), so they
  keep their size on screen.
- **Ruler** (2026-11-06, `ruler.rs`, `tools/ruler.rs`, `panels/ruler.rs`): one
  `Model::trace` of the line at the level shown, its segments merged by cell; through
  the layers, from `Model::surface_normal` at the surface picked, traced across the
  problem and cut at the first void after the first layer. Not part of the scene
  (not undone, not saved): `Document::ruler`, drawn by `overlay.rs` in both views.
- **Field styles** (2026-11-06): `Ramp` gained Turbo, Inferno and Grey and `Scale`
  discrete `bands` (baked into the GPU's lookup table, so the 3D pass's `DataKey`
  includes them); the scene's `FieldState::style` holds the ramp, bands and legend;
  `panels/profile.rs` samples the field at 400 points of the ruler.
- **Legends** (2026-11-10, `llevant_render::legend`): which values a bar marks
  (`tick_values`: auto or many, round values whose labels are left out where they
  would overlap; or exact, every one kept: N evenly spaced along the bar, in decades
  on a log scale, or the band edges, `Ticks::Edges`, at the fractions `Scale::banded`
  changes colour; for exact ticks the bar grows to fit their labels and, failing
  that, a horizontal bar puts every other label on a second row), their numbers in
  one notation for the bar
  (`labels`), a fitted scale's ends rounded out (`rounded`), and the layout
  (`layout`: text measured in IBM Plex Mono, `MONO_FONT`, moved here from the viewer's
  assets; the end labels inside the bar's span, the others left out when they would
  overlap; the bar shortened to fit its view; arrows for `FieldLook::below` and
  `above`). The viewer draws the layout with egui (`overlay::colour_bar`, an opaque
  box) and into saved images (`export::with_legend`, `legend::rasterize`, glyphs by
  `ab_glyph`), so a saved image carries the legend seen. The CLI's colour bar under its images is
the same layout, horizontal across the width on white (`legend::strip`; the bitmap
digits of `Scale::bar_rgb` it replaced are gone). `FieldLook::threshold`
  draws values outside it as no value; with the colours outside the scale it reaches
  the GPU pass as `params[2]` (the passes' numbers are 12 since then,
  `passes::Params`). Outline colours: `Effects::outline_colour` (the 3D pass mixes
  it in `strength` of the way, black darkening as before) and
  `SliceState::outline_colour` (the slice composes with it, no new trace).
- **Workspaces** (2026-11-06, [decision 0012](../decisions/0012-viewer-redesign.md)):
  `app/frame.rs` lays the window out as one of four workspaces (Explore, Check,
  Results, Tally), each with its `Panel`s on the left, the 3D view and the slice in
  the middle (either maximised) and the inspector on the right; the sides are egui
  side panels, resized and folded to a rail, and the `Frame` (workspace shown, each
  one's panel, the sides' folds and widths, the same for all) is kept in the
  settings; the sides' egui ids do not name the workspace, so egui keeps one width
  for all (2026-11-10: switching no longer moves anything but the left panel). The left side's contents
  are clipped to its width (`contained`): egui widens a panel to fit a row that does
  not wrap and then cannot drag it narrower, as Find's Recent did with a long
  `c …` selection (now cut short too; `tests/app/explore.rs`,
  `a_long_selection_keeps_the_left_side_narrow`). The meshes and their handles
  are shown in Tally only (`Toolbox::meshes_on`); leaving it stops drawing one.
  Explore's panels are one row of tabs: Find, Materials, Envelopes (one
  `FindPanel`, three modes), Volumes, Places (saved views and pins). The views are 3D then slice, half
  the width each, in every workspace (`Frame::views`, `Frame::shares`); Tally's inspector shows the mesh selected, with the
  cells it covers sampled on a 16³ grid (`inspector/mesh.rs`, cached per mesh and
  model). Over the views, the tool row (Select and the Ruler, then colouring,
  level, void and air; `panels/toolbar.rs`); on each view, its header
  (`panels/headers.rs`: the 3D view's Section and Display popups, the slice's
  plane and Display; popups in `panels/view_settings.rs`).
  Cuts, the slice plane in 3D and (in Tally) FMESH meshes are not tools but
  `tools::Layer`s (2026-11-09): they hear the pointer before the active tool and
  take the left button when it goes down on one of their handles, a cut's
  outline or a mesh, so they are dragged whatever the tool; a click elsewhere
  deselects them and reaches the tool. Cuts not selected are faint outlines with
  their dots; the selected one is filled and has its rings. The slice plane is
  selected by its handle only (its outline runs along the model's edges). This
  replaced the Pin, Section and Mesh tools (pins: the right-click menu and the
  inspector). The slice plane is drawn in 3D when the Section menu asks, and
  otherwise while it is selected or its position is dragged or typed in the
  slice's header (`Document::slice_shown_in_3d`, `Editing::slice_moving`). The
  status bar is plain, with the renderer's numbers and the tracing settings behind
  the backend chip; its message is `Document::messages` (`messages.rs`): news shown
  for 6 s, work while the application is busy (`status::Busy`), warnings until
  dismissed, and a log of the last 50 behind a button. The search structures
  being built are a spinner of their own while `ViewerApp::completing` runs. This replaced a free dock of eleven tabs (`egui_dock`) and the Properties
  panel.
- **Panels** (2026-10-18): a menu bar and a command palette (Ctrl+K, or the search
  field) that takes command names, rules, `slice z 10`, points, `open u N`, `level 0`
  (`level all`) and `ghost c 300`; the tool row's level menu shows one universe
  level in both views. Find (`panels/find.rs`, 2026-11-06, replacing the outliner's
  tree) resolves a rule once (`Query::resolve`, cached until the rule or the model
  changes), groups its cells and draws only the rows scrolled to; its saved rules
  are the scene's `rules` (view file, undoable). The materials panel is a legend
  with eyes and looks. The eyes (2026-11-10, `seen.rs`) show what is drawn, not
  the scene's flags: a cell is drawn if its flag is on and rays reach its universe
  through shown filled cells at the level shown or above
  (`ModelIndex::reach_levels`); with the visibility without the isolated rule and
  the cells it keeps, each hidden cell gets a reason (its rules, isolated away, inside
  a hidden cell, below the level). `Seen` is made with each visibility by the
  deriver (~7 ms on the reference model; the loading thread makes the first), so
  the eyes catch up when the 3D view does; a click on a shut eye is
  `Edit::Reveal` (its rule taken out and, if isolating, `iso | q`). Saved views keep camera,
  slice and scene in the view file. Cards come from `llevant_scene::source::Source`,
  read on a thread beside the model (one more parse of the input; the model itself
  may come from the cache).
- **Inspector** (2026-11-06, decision 0012, `panels/inspector/`): a stack of pages
  with back and forward (`Inspector`, reset when the pick changes): what is picked
  (selection, point, the cell's page, the surface picked), and pages of a cell, a
  material, a surface and a universe built from the model, its index and the
  input's cards. Cards are shown as words (`tokens.rs`: a small reader of MCNP
  card syntax for colour and links only; the model is compiled by the engine's own
  parser). A surface's users come from `ModelIndex::surface_cells` (one parallel
  pass over the programs), cached per surface and model; a cell's size comes from
  the last volume run or a "Measure" run around it alone.
- **Universes on their own** (2026-10-18): the inspector offers "Open u=N" for each
  universe under the top level of a pick. It opens `Model::universe(N, Cut::Own)` on a
  thread (`app/loading.rs`) as a new `Document`, while the one it came from stays
  loaded behind it in `ViewerApp::parents`. The tool row then shows "⬅ Back" and the
  breadcrumb (`model › u=…`), and Back is instant. The new document starts from the
  model's scene without its clip plane and pins (both in the model's frame), hiding
  only cells and universes it has, with the camera fitted to it. The view file's
  `open` lists the stack, so `--state` opens it again (with the models before at
  their default view). Opening a filler on its own is the fast way to look at it:
  hiding everything else leaves rays to step across every boundary of the hidden
  levels.
- **Looks**: the document's `Looks` (from the scene's `materials`) go to both
  views.
- **Framing**: the visible extent, measured by 6 × 48² CPU rays along the axes
  (0.25 s); models in a 200 m world cell otherwise frame as a dot.
- **Tests**: headless against `render_image` (CPU backend exact, GPU backend equal on
  the zoo views); the whole application driven headlessly with egui_kittest
  (`tests/app/`: pins, a 3D pick checked against the CPU engine, undo and redo,
  opening a universe and going back, selecting through links, hiding and
  isolating, the context menu, the axes);
  and `--screenshot` / `--orbit` / `--storm` runs of the real window.

- **Look**: dark and light themes in neutral grey with a blue accent (yellow stays
  for the selection in the views), IBM Plex Sans
  and Plex Mono (data), Lucide icons, all bundled (`theme.rs`, `icons.rs`).
  The logo is the rosette of the Fusion for Energy emblem (`assets/logo*.png`,
  `logo.rs`): on the start screen, as the window's icon, and as the spinner
  everywhere one is shown (its petals turn about the stars). Progress bars are
  the house `theme::progress_bar`, whose text changes colour where the fill
  ends (egui's draws it in the accent's text colour throughout, unreadable on
  the empty part).

- **Cuts and slices** (2026-10-18): the scene's `cuts` (planes at any angle, wedges
  about an axis, section boxes; `state::CutState`) make the
  camera's `Cutaway` (with the view file's `clip` plane, which the viewer no longer edits); `tools::CutHandles` draws them and drags their handles in 3D, whatever the tool (a plane's centre
  along its normal and, once selected, three rings about x, y and z to turn it, in steps of 15°
  or freely with Shift or Alt; a wedge's sides about its axis, where the pointer's ray meets the
  plane normal to it; a box's faces), each drag one step of the history. A cut can
  be turned off (`Scene::cuts_off`, `Edit::CutOn`: kept, cutting nothing, not drawn;
  `cutaway`, the render command and the section-box region use `Scene::cuts_on`), and
  `Document::show_cuts` (the Section menu's "show cuts in 3D") hides the outlines and
  handles of all but the selected cut. The slice
  is `axis = at` or radial (`state::Radial`: through an axis at φ, moved off it by
  `at`), mapped through a `slice::Frame`; PageUp and PageDown step it; its outline is
  drawn in 3D with handles of its own (moving it, and once selected rings turning
  it: each a step of the history, `History::plane`), and "cut 3D at the slice" cuts
  away the side facing the eye, both from the 3D view's Section menu. O
  switches to an orthographic camera.

- **Analyses** (2026-10-18): jobs (`jobs.rs`) run volumes and checks
  (`Model::volumes`, with a `llevant::Progress` to show and cancel) and
  fingerprints on threads of their own, each document keeping its last results
  (Volumes, Problems, Statistics panels).
  Volumes (2026-10-30) are per cell: over a region either one pass at `h` (the
  default; Refine halves it everywhere) or to a target σ/V (1 % or 0.1 %;
  `Model::volumes_to` by cell, with a callback per round, down to `h / 64`, so only the cells short of
  it are traced again). A target run shows the volumes so far after each round
  ("so far", cells not refined yet marked) and keeps them if cancelled: on the
  425 k-cell model a whole-model 1 % run takes hours,
  hence not the default. "The selection" measures the selected cells alone, every
  instance, lines traced only around them (`Model::instance_volumes` over the
  problem's box), not the box around the selection with all it holds.
  Materials and universes add up their cells (σ in quadrature: cells refined to
  different `h` share no lines). Cells short of the target or met by no line
  are marked. The check keeps the whole-region pass (`Model::volumes`).
  The slice's tracer reports the problem labels it meets,
  so gaps and overlaps seen while looking around are listed. File exports PNGs and
  the view as a `llevant render` command, which takes `--wedge`, `--box`,
  `--remove-box`, repeated `--clip`, `--ortho`, `--up`, `--hide-rule` and `--isolate`
  for that. With `--isolate` and no `--region` or `--eye`, `llevant render` (and
  Python's `render(isolate=…)`) frames every instance of the cells isolated
  (`llevant_scene::bounds::selection_box`, the box `llevant bbox` prints) rather
  than the model's extent, where a component is a few pixels wide; when under 1% of
  the rays hit, it says what it framed. Without `--from`, it looks from where the
  cells show most (`bounds::view_from`: 64² rays from each of the 17 directions to
  a cube's faces, edges and corners that are level or above, ~0.1–0.3 s on a
  500 k-cell model), unless the default `-0.6,-0.7,0.4` shows 90% as much: a
  closure plate edge on from the default is seen face on. Rule options
  (`--isolate`, `--hide-rule`, `--ghost`, `bbox --cells`) take `@file`: a list of cell
  numbers and ranges, or a rule per line (any of them), `#` starting a comment. (The viewer's selection box is still the
  level-0 holders' boxes: the placed boxes take ~30 ms on a 500 k-cell model, too
  much for the UI thread, decision 0008.)

- **Fields** (2026-10-22; MCTAL 2026-10-23; cuvmsh 2026-11-03): the scene's `field` (a CSV, meshtal,
  MCTAL or D1SUNED cuvmsh file, as its voxel fluxes, its tally and bins, scale and opacity; undoable, in the view file) and the document's data,
  read on a thread and kept per source, so that undo and redo do not read again.
  Fields are never read on the UI thread: a scene field without data (from the view
  file, a saved view, or undone back to after a failed read) is drawn once read.
  Tally files stay indexed (`document::TallyFiles`, per file and run input, with
  their summaries; made again when the file changes), so choosing another tally
  or bin reads only that.
  Both views draw it: the 3D view with the `field` pass (`view3d/field.rs`,
  `shaders/field.wgsl`: values per cell, per instance through the hit's instance,
  or per voxel through the hit point, with cell under voxel; NaN tested by its
  bits, since WGSL may assume floats are never NaN), the slice through its tracer's
  `Paint`. The Fields panel opens files
  (or take one dropped on the window) and sets the scale (fitted or by hand, log or
  linear) and the opacity. For tally files (2026-10-26) it is built from the file's
  `TallyFile::summary`: the run, a menu of its tallies (those that can't be drawn
  greyed, saying why), a menu per axis with several bins (labelled by their bounds;
  the Total, or "sum of all" where there is none), results or relative errors, and
  the run's input (the source's `input`, the model's by default; tally files are
  indexed per file and input). A tally file whose choice fails stays the scene's
  field, the panel showing the error over the menus, so that another can be chosen;
  picking a tally starts it at its Totals, or at its first bins where there are
  none. An MCTAL cell tally lists its cell bins (2026-10-26), each with its value ±
  relative error (read with the field, `Document::field_bins`): a click selects the
  bin's cells (Ctrl adds, Shift removes; a double click frames them), a button
  isolates them, and the cells of every bin are selected or isolated at once.
  Isolating is by cell (`Hiding::isolate`, so a filled cell brings all it holds, as
  the tally scores it); a chain's bin shows every instance of its cells, which the
  panel says. Its colour bar is drawn
  in both views. The inspector shows the point's instance (id and MCNP path) and
  the field's value there. "Copy as command" adds `--field`, its tally, bins and run input, and the scale.
  A universe opened on its own starts without the field.

- **FMESH meshes** (2026-10-29): the Tally workspace lays out FMESH tallies over the
  model; the inputs are never changed, the cards are copied or saved. The scene's
  `meshes` (`mesh::MeshState`: tally number, particle, `*`, a box or a cylinder,
  `TR=`, EMESH, TMESH, `OUT=`, a comment for `FC`; undoable, in the view file) keep
  fine bounds, as MCNP has them once it has read a card. Cards are written by
  `aci_tallies_geom::mcnp_input` (`FmeshCard::to_input`, `transform_card`), which
  reads them back the same to the bit (MCNP 6.2 scores the pencil-beam fixture's
  meshes, rewritten this way, exactly as the originals), its coarse bins the
  roundest that give the fine bounds (`coarse_bins`). The FMESH panel's fields are
  read as a one-axis card by the same reader, so MCNP's shortcuts (`2i`, `3r`,
  `5log`) and rules apply. After the panel's "+ Box" or "+ Cylinder"
  (`Command::DrawMesh`), a drag in the slice makes a box (along the model's axes;
  on a radial slice along the plane's, by a new `TRn`) or a cylinder about the
  plane's normal, as deep as the model, bounds rounded to about 10 screen points
  (Shift: not). Handles move outer bounds (`mesh::moved_end`: coarse bins passed
  are dropped, the one reached keeps its fine bins), the whole mesh, a cylinder's
  base; in 3D rings turn it (a cylinder without `TR=` turns its own `AXS`/`VEC`,
  anything else its `TR`). The slice draws each mesh as an image of its voxels'
  boundaries (`mesh::draw::raster`): every pixel located in the mesh by
  `Geometry::locate` (in parallel, again only when the mesh, plane or view
  changes), exact for any `TR=`, cylinder and plane; an axis whose bins would be
  under ~5 pixels shows its coarse bins, or none, and says so. 3D draws wireframes.
  The input's FMESH cards, tally and TR numbers are read beside its cards
  (`mesh::InputMeshes`): new meshes take free numbers, the window lists the input's
  meshes (to edit a copy) and flags clashes (a tally number or a different `TRn` in
  the input or another mesh, a number not ending in 4), meshes outside the model,
  more than 10⁸ bins, very long voxels; copied cards leave out a `TRn` the input
  has as it is. The tool hears from both views every frame: a drag belongs to the
  view it started in. A universe opened on its own starts without meshes (they are
  in the model's frame).

- **Around the model** (2026-10-18): a start screen (recent files from the
  settings, `settings.rs`; "Open…" through the system's dialog, `rfd`; files dropped
  on the window); loading with `Store::load`'s steps, a bar and Cancel; the
  input watched (its modification time, every 2 s) for a Reload that keeps the view;
  colour by density (`ColourBy::Density`: log scale, viridis-like ramp). Settings
  are applied by the binary only (`run`), so tests and embedding programs see the
  defaults.
- **Surfaces** (2026-10-31): a pick carries the surface there (a 3D hit's, free from
  `first_hits`; in the slice the nearest crossing within 6 screen points, as the slice
  shows it at its level), named in the hover banner and status bar (`s 15`, `s 15.3`
  for a facet). The inspector's Surface section, for the point double-clicked: the side the
  cell takes, coincident surfaces, the cells using it, its card, and "Select cells on
  s N". The rule `s 15` (`surface 15 20-25`) selects, hides or isolates the cells whose
  cards use those surfaces (any facet, `TRCL` copies, through `#n`); it is not indexed
  but found by one parallel pass over the programs when asked (10–20 ms on the
  reference model; an index cost 0.25–1 s to build and tens of MB). The rule
  `label "HNB - BSV"` (`$ HNB - BSV`, 2026-11-12) selects the cells whose inline `$`
  comments contain the text, any case: the names CAD conversions give cells. The
  compiled model has no text, so the labels (`llevant_scene::labels`: the distinct
  labels, each with its cells' numbers; 41 k on the 512 k-cell outbio model) are read
  from the input and set once on the `ModelIndex`: the viewer reads them with the
  cards, in the background, and gives them to every document (a scene from a view
  file whose rules use them is shown once they come); an edit reads them again only
  when it changes a cell's number or `$` comments; `llevant` and Python read them only
  for a rule that has `label` (0.4 s warm, 1.4–1.9 s cold, on outbio). A label rule
  resolves in 0.5 ms (16 ms for `$ e`, nearly every cell; isolating by it 6–35 ms). The slice's
  properties have "numbers: cells, surfaces" (saved in the view file): labels found
  once each still, full-resolution image is in (`slice/labels.rs`), on the pool, and
  drawn over that image while it shows.
  The rule `shown` (`visible`, 2026-11-12) is what the 3D view shows: the cells it
  draws (`Seen::drawn`: not hidden, reached at the level shown; filled cells only as
  solids at a level) with some part its cuts leave (`Document::cut3d`: the clip
  plane, the cuts that are on, the slice plane's side when it cuts), behind others
  too. It is a term of `llevant_scene::query` that only a view resolves
  (`Query::resolve_with`; elsewhere an error). Cuts are tested by
  `Model::cells_kept` (engine.md, "Within boxes"): about 40 ms for a universe of
  800 cells on the reference model, 2–3 s for its 420 k unfilled cells under a plane
  (placing 3.4 M instance boxes), so under cuts the document works it out on a
  thread of its own (`Document::shown_cells`), Find and the palette saying so in the
  meantime, and keeps it until the model, `Seen` or the cuts change. A command
  carrying a rule with `shown` (select, hide, isolate, ghost, save) freezes it into the
  cells it gives (`Document::freeze`): a selection or hiding rule that followed the
  view would change as the view does, and hiding by `shown` would feed on itself.

Next for the viewer, as tools over the same document: pins hidden behind geometry drawn faded (a CPU ray per pin
when the camera stops). Anything the tools need from the engine is designed and
tested as an engine feature.
