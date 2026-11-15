# 0012: The viewer is organised in four workspaces around an inspector

2026-11-06. Accepted.

## Context

The viewer grew panel by panel while the engine was built. It ended up with a free dock
of eleven tabs (3D, Slice, Outliner, Inspector, Properties, Pins, Views, Volumes,
Problems, Statistics, Fields), a toolbar mixing tools, history, which views were open,
colouring, level and theme, and a status bar of renderer timings. Reviews with its
users (mockups and questionnaires) found:

- **Too many places to look.** What was needed was often in a hidden tab, properties
  changed with the view last touched, view settings lived far from the views, rare
  jobs weighed as much as constant ones, and empty panels wasted space.
- **Who and where.** People who write MCNP, most often on a laptop, always with a
  mouse, and on monitors too.
- **What they do most.** Finding what is at a point, and checking envelopes and
  fillers. Gaps and overlaps, edits, results, FMESH, volumes, images and showing
  models to others come up sometimes.
- **The inspector matters most.** The tree by levels is cluttered on large models; the
  materials list and the rule box are what is used.
- **Inputs run to 7 M lines,** so the viewer shows the cards of what is picked, not the
  whole input.

## Decision

### The frame

- **Four workspaces**, switched by tabs in the top bar (Ctrl+1 to Ctrl+4):
  - **Explore:** what is where. The left panel has Find, Materials, Envelopes,
    Volumes and Places (saved views and pins).
  - **Check:** gaps and overlaps, with problems and statistics on the left.
  - **Results:** fields and tallies over the geometry.
  - **Tally:** the input's tallies and meshes, and new FMESH cards.

  Card edits and surface drags happen in the inspector, which every workspace has,
  so there is no Edit workspace.
- **The selection, camera, slice and hidden cells are shared** by all workspaces.
  Switching changes the left panel, not what you are looking at.
- **One fixed layout:** the workspace's panel on the left, the 3D view and the slice
  side by side in the middle (either can be maximised), the inspector on the right.
  The sides can be resized and folded to a rail, the same for every workspace: a
  window that rearranged itself at each switch was more disorienting than a
  per-workspace layout was helpful. There is no free docking.
- **A tool row above the views** holds the tools (Select and the Ruler) and what
  applies to both views: the colouring, the level shown, and the void and air
  toggle. Both views always use the same colouring.
- **Each view has a slim header** with its own settings. The 3D header has Section
  (the cuts and the slice plane in 3D), Display (outlines, ambient occlusion,
  orthographic), fit and maximise. The slice header has the plane, its position,
  Display (outlines and numbers), fit and maximise. There is no Properties panel.
- **The menu bar stays** (File, Edit, View, Select, Help). The top bar adds the
  workspace tabs, an always-visible search field (it opens the palette, as Ctrl+K
  does), undo and redo, the theme and the keys window.
- **The status bar says plain things:** the point and cell under the cursor, the
  selection, running jobs and the message of the moment. A message shows while it
  matters (news for 6 s, work while it runs, warnings until dismissed), the last 50
  in a log behind a button. A model shown before its search structures are built is
  a state, shown while it holds. Frame rate, backend and ray costs sit behind a
  backend chip on the right, with the tracing settings.
- **The look is neutral grey** in a dark and a light theme. Blue is the interface's
  accent; yellow is kept for the selection and the picked point in the views, where
  it must show on cells of any colour.

### Interaction

- **CAD-style mouse:** a middle drag orbits about the point under the cursor, Shift +
  middle drag pans, the wheel zooms towards the cursor, and a left drag box-selects.
  The fly keys stay. A click selects; a double-click marks and inspects the point.
- **Cuts, the slice plane in 3D and FMESH meshes are things with handles, not tools.**
  They hear the pointer before the active tool and take the left button only when it
  goes down on a handle, a cut's outline or (in Tally) a mesh. Handles keep a
  constant size on screen. The Ruler stays a tool: it is two clicks or a drag, which
  Select would take. Pins come from the right-click menu and the inspector.
- **Cuts can be hidden or turned off:** hidden (the outlines out of the way, the cut
  still cutting) and off (kept, cutting nothing, to compare with and without; an
  undoable edit saved in the view file).
- **Alt picks surfaces.** With Alt held, the surface under the pointer is marked in
  both views and named; Alt+click opens its page, and again at the same place moves
  to the next surface through the point. A sticky cells/surfaces filter was not
  wanted: one more mode to forget.
- **Camera history:** Alt+← and Alt+→ go back and forward through places looked at.
  Moving the camera stays out of undo.

### The inspector

- **Drill-in with back and forward,** like a browser. A cell's material, surfaces and
  universe each open their own page in place.
- **A cell's page is its fields** (material, density, importances, `U`, `FILL`,
  `TRCL`, other parameters), its place (universe, path, instances), its size (volume
  and mass), and its geometry as text. The card is folded at first.
- **The geometry stays text, with help:** unknown surfaces and cells flagged as you
  type, surface numbers completed with their type and comment, a number under the
  pointer named and marked in the views (Ctrl+click opens its page). Chips or a tree
  were tried and dropped: reference-model cells have nested parentheses, `#`
  complements and fills with transforms, which neither shows well.
- **Material, surface, universe and mesh pages:** a material's composition by element
  (libraries may differ per nuclide), its cells and their densities, and its look; a
  surface's named coefficients to type or drag, the coincident surfaces and the cells
  that use it; a universe's contents, where it is filled and universes alike.
- **Editing:** an edit is previewed (live where possible) with the cards it writes,
  before and after, and applied as one undoable step. A card keeps its text; only the
  words changed are rewritten. Many cells can be edited at once from the selection or
  a rule, and cells, surfaces, materials and universes renumbered with every
  reference updated. New cells, surfaces and materials can be created. In a universe
  opened on its own, an edit compiles against the whole model and the universe is
  cut out again.
- **Transforms are edited as text and compiled on Apply,** not dragged live. A `TR`
  used by many surfaces moves much of a universe at once: on the reference model,
  two such transforms (about 2 400 surfaces, 518 cells of level 0) made the overview
  frame go from 240 ms to 12.6 s while dragging, and keeping the octrees and skips in
  step costs about as much as building them again (0.25–2.6 s a step).

### Finding things

- **Explore's Find:** a rule box with its matches grouped by universe or material,
  saved rules kept in the view file, and recent selections. The level-0 envelopes,
  each with a one-line summary of what fills it, are a list; so is the materials
  legend.
- **Void and air cells are shown or hidden in one click** from the tool row.

### New capabilities

- **A ruler,** in 3D and in the slice, measuring point to point, every cell crossed
  along a line with its thickness, and straight through the layers along a surface's
  normal, with the engine's exact line tracing.
- **Results:** several result files at once; styling in a popover from the colour bar
  or the view header; colour maps (Turbo, Viridis, Inferno, Grey, discrete bands),
  continuous or discrete, log or linear; legend placement, size, orientation, title,
  number format and ticks; a plot of the field along the ruler.
- **Tally** shows the input's own tallies and meshes over the geometry before any run.

### Rejected

- **A canvas with floating panels and the slice as an inset:** not wanted.
- **Free docking:** it is what made the window hard to keep in order.
- **The deep tree of every level:** cluttered on real models. Find, Envelopes and the
  inspector replace it.
- **A full input pane:** inputs of millions of lines.
- **The Jet colour map.**

## Consequences

- `egui_dock` left the viewer. The settings keep the sides' folds and widths and each
  workspace's panel; settings of an older version lose only their layout.
- What the Properties panel held moved into the view headers, the tool row and the
  backend chip.
- The slice marks of surfaces follow the curves: each crossing is placed by 8 halvings
  along its grid edge, in the universe's frame. On the reference model a level-0 cell
  with 197 surfaces is traced in 16.5 ms (`marks::timing`, `--ignored`).
- The app tests drive this frame: workspace tabs, maximise buttons, header controls,
  handles, edits.
- Not done: custom colour maps from a file, and SVG/PDF export of slices (it needs the
  exact curves where surfaces cut the plane, an engine feature).
