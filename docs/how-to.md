# How do I…?

The common jobs, each in the viewer, on the command line and in Python. Pick the tab
you use; the choice is remembered on every page. `model.i` stands for your MCNP input.

Cells, surfaces, materials and universes are always MCNP numbers, and lengths are in
cm.

## …find gaps and overlaps?

=== "Viewer"

    Open the **Check** workspace (Ctrl+2), then **Problems** on the left. Choose the
    region and run the check: it works in the background. Each problem found is a
    row, largest first; its arrow frames it, slices through it and selects its cells.

    While you look around, the slice also lists the gaps and overlaps it runs into.
    Gaps are drawn black and overlaps magenta.

=== "Command line"

    ```sh
    llevant check model.i --h 2
    ```

    `--h` is the spacing of the lines traced through the model, in cm: smaller finds
    thinner problems and takes longer (the time goes as 1/h²). The command exits with
    status 1 when it finds a problem, so scripts can test it.

=== "Python"

    ```python
    m = llevant.load("model.i")
    for g in m.check(h=2):
        print(g)              # overlap (3, 4): 7.696016e4 ± 3.2e1 cm3, with g.bbox
    ```

The check traces lines through the whole model, so it finds every gap or overlap that
a line crosses, however thin: a 1 mm gap is found whenever a line crosses it. Lines
are `h` apart, so a problem smaller than `h` in every direction can be missed.

## …measure volumes and masses?

=== "Viewer"

    **Explore** workspace, **Volumes** panel: volumes by material, cell or universe,
    with their uncertainty and mass, sortable. **Refine** halves the line spacing; the
    table can be saved as CSV next to the input. A cell's page in the inspector also
    has **Measure**.

=== "Command line"

    ```sh
    llevant volume model.i --h 2 --by material          # or --by cell, --by universe
    llevant volume model.i --by cell --rel-sigma 1e-3   # refine until each is within 0.1 %
    llevant volume model.i --h 2 --by cell --format csv > volumes.csv
    ```

=== "Python"

    ```python
    v = m.volumes(h=2)
    v.by("material")                      # [Group(number, volume, sigma, ...)]
    v.to_pandas(by="cell")                # a data frame (pandas installed)
    m.volumes_to(2, 1e-3, by="cell")      # every cell to 0.1 %
    ```

Each volume comes with `sigma`, one standard deviation: about 95 % of the time the
true volume is within 2 sigma.

## …find which cell is at a point?

=== "Viewer"

    Double-click it, in the 3D view or the slice: the inspector shows the cell, its
    material and density, and the path through universes and lattices. Type three
    numbers in the search box at the top (Ctrl+K) to go to a point.

=== "Command line"

    ```sh
    llevant locate model.i 0 0 0  120 -3.5 40
    llevant locate model.i --points points.txt --format csv > cells.csv   # one "x y z" per line
    ```

=== "Python"

    ```python
    m.locate(0, 0, 0)                     # <Label inside 12 > 304[1,-2,0] > 7>
    r = m.classify(points)                # an (N, 3) numpy array
    r.cell, r.material                    # numpy columns
    ```

## …see what a line crosses?

=== "Viewer"

    The **Ruler** tool (M): click two points, or click a face to go through the
    layers behind it. The Ruler window lists each cell crossed, with its material and
    length, and copies them as CSV.

=== "Command line"

    ```sh
    llevant trace model.i --from -100,0,0 --to 100,0,0
    ```

=== "Python"

    ```python
    for seg in m.trace((-100, 0, 0), (100, 0, 0)):
        print(seg)
    tr = m.trace_lines(starts, ends)      # many lines at once
    tr.lengths("material")                # length in each material
    ```

## …draw tally results over the geometry?

llevant reads FMESH results (meshtal files, and D1SUNED `out=cuv` files), MCTAL cell
tallies (F4, F6, F7, F8), and a CSV file of values per cell or material.

=== "Viewer"

    Drop the file on the window, or open the **Results** workspace (Ctrl+3), **Fields**
    panel, **Open field…**. Choose the tally and its bins; **Style…**
    sets the colour map, the scale and the legend.

=== "Command line"

    ```sh
    llevant slice model.i --plane z=0.5 --field run.msht --tally 14 --log -o flux.png
    llevant render model.i --field mctal --tally 4 -o f4.png
    llevant slice model.i --plane z=0.5 --field heat.csv -o heat.png
    ```

    A CSV file has a column `cell` (or `material`) and a column of values whose header
    names them, with their unit:

    ```text
    cell,heating [W/cm3]
    12,3.5
    13,0.8
    ```

=== "Python"

    ```python
    f = llevant.Field.read(m, "run.msht", tally=14)
    m.slice(z=0.5, field=f.with_scale(1e-6, 1e-2)).save("flux.png")
    m.slice(z=0.5, field=llevant.Field.cells(m, {12: 3.5, 13: 0.8}, name="heating", unit="W/cm3"))
    ```

More options (energy and time bins, scales, colour bars):
[Fields](guide/cli.md#fields).

## …compare two versions of a model?

=== "Viewer"

    When the input changes on disk, the viewer offers to reload it; after reloading,
    the cells that changed are selected and checked for gaps and overlaps.

=== "Command line"

    ```sh
    llevant fingerprint old.i --diff new.i
    ```

    It lists the cells and universes whose geometry or material changed, the new
    ones, the removed ones and those only renumbered. Exits with status 1 if they
    differ.

=== "Python"

    ```python
    print(llevant.load("old.i").diff(llevant.load("new.i")))
    ```

## …look inside one universe on its own?

=== "Viewer"

    Right-click inside a filled cell, then **Open u=N on its own**. The universe opens
    in its own frame, cut to a box around its contents; **⬅ Back** returns to the
    model.

=== "Command line"

    Every command takes `--universe`:

    ```sh
    llevant render model.i --universe 20130 -o filler.png
    llevant check  model.i --universe 20130 --h 1
    ```

=== "Python"

    ```python
    f = m.universe(20130)                 # a Model of its own
    f.check(h=1)
    ```

## …hide or isolate cells?

=== "Viewer"

    Click a cell to select it, then **H** to hide it, **Shift+H** to show only it,
    **Alt+H** to show everything again. The **Find** panel (or Ctrl+K) selects cells
    by a rule:

    | Rule | Selects |
    |---|---|
    | `c 12 40-55` | cells 12 and 40 to 55 |
    | `m 5` | cells of material 5 |
    | `u 20040` | cells of universe 20040 |
    | `fill 20040` | the cells universe 20040 fills |
    | `in c 100` | cell 100 and everything inside it |
    | `$ shield` | cells whose `$` comment contains "shield" |
    | `u 20040 & m 5`, `m 5 \| m 6`, `! void` | and, or, not |

=== "Command line"

    ```sh
    llevant render model.i --hide 12,13 -o view.png
    llevant render model.i --isolate "u 20040 & m 5" -o steel.png
    ```

=== "Python"

    ```python
    m.render(hide=[12, 13]).save("view.png")
    ```

## …make an image for a report?

=== "Viewer"

    Set the view up, then in the **File** menu, **Export the 3D view (PNG)** or
    **Export the slice (PNG)**: the image is saved next to the input, with the colour
    bar. The File menu can also copy the `llevant render` command that draws the same
    3D view again, to put in a script.

=== "Command line"

    ```sh
    llevant slice  model.i --plane x=0.5 --px 2000 --colour material -o slice.png
    llevant render model.i --colour realistic --clip 0,0,0,0,-1,0 --size 1600x1200 -o view.png
    ```

=== "Python"

    ```python
    m.slice(x=0.5, colour="material").save("slice.png")
    m.render(colour="realistic", direction=(1, 1, -0.5)).save("view.png")
    ```

`--colour` takes `cell`, `material`, `universe` or `realistic` (what materials look
like: steel, concrete, water…). To choose a material's look yourself, see
[Realistic images](guide/cli.md#realistic-images).
