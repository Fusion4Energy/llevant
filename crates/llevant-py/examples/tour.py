# %% [markdown]
# # A tour of llevant's Python API
#
# Run it as a script, `python tour.py [model] [out_dir]`, or open it as a
# notebook: VS Code and PyCharm run the `# %%` cells directly, and
# `jupytext --to notebook tour.py` makes a `.ipynb`. Images then show inline;
# as a script they are written to `out_dir`.
#
# Without arguments it uses `universes.i`, a small model of the MCNP
# comparison zoo (fills, lattices, transforms). Point it at your own model:
# every cell below works on any input.

# %%
import sys
import time
from pathlib import Path

import llevant
import numpy as np

ZOO = Path(__file__).resolve().parents[3] / "tools" / "mcnp-oracle" / "models"
path = Path(sys.argv[1]) if len(sys.argv) > 1 else ZOO / "universes.i"
out = Path(sys.argv[2]) if len(sys.argv) > 2 else Path("tour_out")
notebook = "ipykernel" in sys.modules


def show(img, name):
    """Inline in a notebook, a PNG file in a script."""
    if notebook:
        from IPython.display import display

        display(img)
    else:
        out.mkdir(exist_ok=True)
        img.save(out / f"{name}.png")
        print(f"wrote {out / name}.png")


# %% [markdown]
# ## Load once
# Compiling is the slow part (seconds on large models); every query after it
# reuses the compiled model.

# %%
t = time.perf_counter()
m = llevant.load(path)
print(m, f"loaded in {time.perf_counter() - t:.2f} s")
print({k: v for k, v in m.info().items() if k != "timings"})
for d in m.diagnostics[:5]:
    print("  ", d)

# %% [markdown]
# ## Cells, universes, materials (MCNP numbers throughout)

# %%
print(m.cells)
for cell in m.cells.values()[:8]:
    print(cell)
print("universes:", m.universes)
materials = sorted({c.material for c in m.cells.values() if c.material})
if materials:
    print(f"m{materials[0]}:", m.material(materials[0])[:4])

# %% [markdown]
# ## Where is a point?
# A `Label` is the path through the universes: cells, lattice elements, and
# the status (`inside`, `gap`, `overlap`, `on-surface`...).

# %%
lo, hi = m.extent()  # measured box around the contents
centre = [(a + b) / 2 for a, b in zip(lo, hi)]
label = m.locate(centre)
print(label, "| cell", label.cell, "material", label.material, "path", label.path)

# %% [markdown]
# ## Many points at once: numpy in, numpy out
# One call, in parallel; `ids` index the distinct `labels`, and `cell`,
# `material`, `universe`, `status` are columns gathered in Rust.

# %%
rng = np.random.default_rng(1)
points = rng.uniform(lo, hi, size=(1_000_000, 3))
t = time.perf_counter()
r = m.classify(points)
print(r, f"in {time.perf_counter() - t:.2f} s")
print("first cells:", r.cell[:10])
print("first point:", r[0])
# Fraction of the box in each material (-1: not inside a cell).
mats, counts = np.unique(r.material, return_counts=True)
for mat, n in zip(mats, counts):
    print(f"  material {mat:>4}: {n / len(r):6.2%}")

# %% [markdown]
# ## What does a line cross?

# %%
for s in m.trace(lo, hi)[:8]:
    print(f"{s.t0:9.3f} .. {s.t1:9.3f}  {s.label}  ({s.surface_in} > {s.surface_out})")

# %% [markdown]
# Many lines: ragged columns (`offsets`, `t0`, `t1`, `ids`, surfaces), and
# total lengths per cell, material, universe or label.

# %%
starts = rng.uniform(lo, hi, size=(10_000, 3))
ends = rng.uniform(lo, hi, size=(10_000, 3))
tr = m.trace_lines(starts, ends)
print(tr)
print("segments of line 0:", len(tr[0]))
print(
    "length by material:", {k: round(v, 1) for k, v in tr.lengths("material").items()}
)

# %% [markdown]
# ## Images
# A slice is exact along its rows; `at(i, j)` tells what is under a pixel.

# %%
img = m.slice(z=centre[2], width=800, colour="material")
show(img, "slice")
i, j = img.width // 2, img.height // 2
print("under the centre pixel:", img.at(i, j), "at", img.point(i, j))
print("gaps and overlaps in the image:", img.problems())

# %%
view = m.render(direction=(0.6, 0.7, -0.4), size=(600, 450))
show(view, "render")

# %% [markdown]
# A cutaway: keep the half below the centre plane, looking down at it.

# %%
cut = m.render(direction=(0.3, 0.5, -1.0), size=(600, 450), clip=(centre, (0, 0, 1)))
show(cut, "cutaway")

# %% [markdown]
# `gpu=True` renders on the GPU (needs an adapter; the model is uploaded
# once and kept for the next renders).

# %%
try:
    show(m.render(size=(600, 450), gpu=True), "render_gpu")
except llevant.LlevantError as e:
    print("no GPU:", e)

# %% [markdown]
# ## Instances and fields
# A cell of a repeated universe, or a lattice cell, occupies many places:
# each is an instance, with an id and a path in MCNP notation. A field is
# numbers over the geometry (per cell, per instance or on a mesh; from a
# CSV or meshtal file with `Field.read`) drawn over images with `field=`.

# %%
here = m.locate(*centre)
print("instance at the centre:", here.instance, here.instance_path, "of", m.instance_count)
rows = m.classify(rng.uniform(lo, hi, size=(2000, 3)))
inside = rows.instance >= 0
print("distinct instances among 2000 random points:", len(np.unique(rows.instance[inside])))
# A field per instance: each copy of a cell gets its own value.
f = llevant.Field.instances(m, np.arange(m.instance_count, dtype=float), name="instance id")
show(m.slice(z=centre[2], width=800, field=f), "slice_instances")
# A field on a mesh: distance from the centre, on 20 x 20 x 20 voxels.
edges = [np.linspace(a, b, 21) for a, b in zip(lo, hi)]
mid = [0.5 * (e[1:] + e[:-1]) for e in edges]
X, Y, Z = np.meshgrid(*mid, indexing="ij")
r = np.sqrt((X - centre[0]) ** 2 + (Y - centre[1]) ** 2 + (Z - centre[2]) ** 2)
g = llevant.Field.mesh(m, r, x=edges[0], y=edges[1], z=edges[2], name="distance", unit="cm")
print(g, "value at the centre:", g.value(*centre))
show(m.render(direction=(0.6, 0.7, -0.4), size=(600, 450), field=g.with_opacity(0.6)), "render_field")

# %% [markdown]
# ## Volumes, gaps and overlaps
# Twelve lattices of lines in random directions, as many lines as three
# axis-parallel families `h` cm apart; exact along each line, with the
# standard error over the lattices. `m.volumes_to(h, 1e-3)` refines each
# cell until its sigma is 0.1 % of its volume.

# %%
h = max(b - a for a, b in zip(lo, hi)) / 200
v = m.volumes(h)
print(v)
for g in v.by("material"):
    print(f"  {g.name:>12} {g.volume:12.4e} ± {g.sigma:.1e} cm3")
problems = v.problems()
print("gaps and overlaps:", problems or "none")

# %% [markdown]
# ## Data frames (with polars or pandas installed)

# %%
try:
    import polars as pl

    df = tr.to_polars()
    print(df.group_by("material").agg(pl.col("length").sum()).sort("material"))
    print(v.to_polars("cell").head())
except ImportError:
    print("pip install polars for data frames")

# %% [markdown]
# ## Did the model change?
# Fingerprints ignore numbering and card order; `diff` tells cell by cell.

# %%
d = m.diff(m.reload())
print(repr(d), "-> unchanged" if not d else d)

# %% [markdown]
# A new version of the input made in memory (an edit, or a new assembly)
# compiles against the model: only the universes that changed are built again.

# %%
if m.path is not None:
    text = open(m.path, encoding="utf-8", errors="replace").read()
    print(m.diff(m.recompile(text)) or "the same model")
