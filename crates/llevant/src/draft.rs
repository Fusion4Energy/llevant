//! Drafts: a surface moved while it is dragged, previewed live.
//!
//! A [`Draft`] starts from a compiled model and one of its surfaces. Each
//! [`Draft::set`] rebuilds that surface's shapes from new coefficients and
//! updates only what depends on them, in a few ms on the reference model:
//!
//! - the boxes of the cells that use the surface (their node boxes become
//!   unbounded: pruning is an accelerator, so the draft stays exact);
//! - the regions of the universes those cells fill, where they grow beyond
//!   the model's, and then the boxes of those universes' cells, and so on
//!   down (`docs/internals/engine.md`, "Drafts");
//! - the BVHs of the universes touched, refitted;
//! - the octrees of the universes holding the moved cells, switched off
//!   (queries there go through the BVH, exactly).
//!
//! The draft's model ([`Draft::snapshot`]) answers every query exactly as a
//! compile of the edited input would, only more slowly where octrees are
//! off; it is pending, so it cannot be saved. Each step also returns a
//! [`DraftPatch`]: the records it changed, in the layout of
//! [`Model::export`], for a GPU copy of the model to be patched in place.
//! For a universe with an octree and few moved cells, the patch keeps the
//! octree on the GPU: the moved cells' leaf programs become their whole
//! programs and the cells are checked everywhere in the universe (its
//! `always` list), which is exact too.
//!
//! A draft is never committed: when the drag ends, the edit is made to the
//! input and compiled as any other ([`Model::recompile_parsed`]).
//!
//! This module serves interactive frontends (the viewer, and `llevant-gpu`
//! for the patches); it may change in any 0.x release.

use std::sync::Arc;

use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::bbox::{analyse, Aabb, Bounder, QuadInfo};
use crate::error::{Error, Result};
use crate::export::{self, Span, GRID_PROG_FULL, GRID_PROG_TRUE, NONE};
use crate::expr::{NODE, NO_BOX};
use crate::grid::{PROG_FULL, PROG_TRUE};
use crate::model::{build_surface, lattice_element_box, Fill, Model};
use crate::skip::Skip;
use crate::surface::Shape;
use crate::transform::Affine;

/// Most moved cells in a universe for its octree to stay on the GPU (each
/// is then checked at every point of the universe).
pub const MAX_KEPT_CELLS: usize = 64;

/// What a draft step changed, in the layout of [`Model::export`] (indices
/// into its arrays), to patch a copy of the model uploaded from the
/// draft's base model ([`Draft::base`]).
#[derive(Clone, Debug, Default)]
pub struct DraftPatch {
    pub shapes: Vec<PatchShape>,
    /// Cell boxes: `(cell, box)`.
    pub cells: Vec<(u32, Aabb)>,
    /// `(index into Export::node_boxes, boxes)`.
    pub node_boxes: Vec<(u32, [Aabb; 2])>,
    /// `(index into Export::bvh_nodes, box)`.
    pub bvh_nodes: Vec<(u32, Aabb)>,
    /// `(index into Export::bvh_items, box)`.
    pub bvh_items: Vec<(u32, Aabb)>,
    pub units: Vec<PatchUnit>,
    /// `(index into Export::grid_progs, value)`.
    pub grid_progs: Vec<(u32, u32)>,
}

/// A shape's new data ([`export::shape_data`]), and the box of the cells
/// that use it (cells over 100 m left out), for an evaluator that recentres
/// quadrics about it.
#[derive(Clone, Debug)]
pub struct PatchShape {
    pub shape: u32,
    /// Where its data starts in [`export::Export::shape_data`].
    pub start: u32,
    pub kind: u32,
    pub data: Vec<f64>,
    pub usage: Aabb,
}

/// A unit's region, octree root ([`NONE`]: switched off) and the cells it
/// checks everywhere.
#[derive(Clone, Debug)]
pub struct PatchUnit {
    pub unit: u32,
    pub roi: Aabb,
    pub grid_root: u32,
    pub always: PatchAlways,
}

/// A unit's cells checked everywhere: its span of [`export::Export::always`]
/// as uploaded, or a list of its own (stored elsewhere by the evaluator).
#[derive(Clone, Debug)]
pub enum PatchAlways {
    Export(Span),
    Own(Vec<u32>),
}

/// See the module documentation.
pub struct Draft {
    base: Arc<Model>,
    work: Model,
    surface: i64,
    kind: String,
    coeffs: Vec<f64>,
    /// The coefficients of the last step that succeeded.
    last: Vec<f64>,
    tr: Option<Affine>,
    /// The surface's shapes, and where their data starts in the export.
    shapes: Vec<u32>,
    shape_starts: Vec<u32>,
    /// Cells using the surface, and their units.
    moved: Vec<u32>,
    moved_units: Vec<u32>,
    /// Units whose octree the GPU keeps: the export indices of the moved
    /// cells' entries in it, with their values as uploaded.
    kept: FxHashMap<u32, Vec<(u32, u32)>>,
    /// Per unit, the cells filling it (with a universe, or as a lattice
    /// element), and their transform.
    fillers: Vec<Vec<(u32, Option<Affine>)>>,
    info: Vec<Option<QuadInfo>>,
    /// Per unit touched, each cell's place in its BVH.
    bvh_pos: FxHashMap<u32, FxHashMap<u32, usize>>,
    /// Cells and units changed by any step so far (a patch covers them all,
    /// so that a step undoes what an earlier one did and this one does not).
    dirty_cells: FxHashSet<u32>,
    dirty_units: FxHashSet<u32>,
    /// Cells added to a unit's `always` by the last step (outside its BVH:
    /// empty in the base model, not now).
    extras: FxHashMap<u32, Vec<u32>>,
    /// Per unit, as exported: BVH node and item bases, octree root, octree
    /// item base, `always` span.
    export_bvh: Vec<(u32, u32)>,
    export_grid: Vec<(u32, u32)>,
    export_always: Vec<Span>,
    /// The moved cells' units and every unit below them.
    below: Vec<u32>,
}

/// Program node boxes of a cell: `node_start + k` for each box word.
fn node_box_count(code: &[u32]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < code.len() {
        if code[i] & NODE != 0 {
            if code[i + 1] != NO_BOX {
                n += 1;
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    n
}

impl Draft {
    /// Start dragging surface `surface` of `base`, whose input as parsed is
    /// `src` (the tree `base` was compiled from). [`Error::Invalid`], saying
    /// why, when the surface cannot be previewed this way: not in the
    /// input, unused, used by a lattice cell (whose planes the lattice
    /// copies), or by an `IMP:N=0` cell of universe 0 (which makes the
    /// problem's box). `base` may be a universe cut from a model
    /// ([`Model::universe`]): its cut box stays where it is, and the draft
    /// answers as that universe cut from the edited model to the same box.
    pub fn begin(base: Arc<Model>, src: &migjorn::Model, surface: i64) -> Result<Draft> {
        let invalid = |m: String| Error::Invalid(format!("surface {surface}: {m}"));
        let v = src
            .surface(surface)
            .ok_or_else(|| invalid("not in the input".into()))?;
        let kind = v
            .mnemonic()
            .ok_or_else(|| invalid("no surface type".into()))?
            .to_string();
        let coeffs = v.coeffs().map_err(|e| invalid(e.message))?;
        let tr = match v.transform().filter(|&n| n > 0) {
            None => None,
            Some(n) => {
                // As the compiler: the last card of that number.
                let t = src
                    .transforms()
                    .filter(|t| t.id() == Some(n))
                    .last()
                    .ok_or_else(|| invalid(format!("TR{n} is not defined")))?;
                Some(Affine::from(t.affine().map_err(|e| invalid(e.message))?))
            }
        };
        let shapes: Vec<u32> = (0..base.shapes.len() as u32)
            .into_par_iter()
            .filter(|&s| base.surface_ids[s as usize].id == surface)
            .collect();
        let built = build_surface(&kind, &coeffs, tr.as_ref()).map_err(invalid)?;
        if shapes.is_empty() {
            return Err(invalid("not compiled into the model".into()));
        }
        if built.len() != shapes.len()
            || built
                .iter()
                .zip(&shapes)
                .any(|(b, &s)| *b != base.shapes[s as usize])
        {
            return Err(invalid(
                "the model was not compiled from this input (or the surface is moved by TRCL)"
                    .into(),
            ));
        }
        let moved: Vec<u32> = (0..base.cells.len() as u32)
            .into_par_iter()
            .filter(|&c| {
                crate::expr::leaves(base.code(c)).any(|(s, _)| shapes.binary_search(&s).is_ok())
            })
            .collect();
        if moved.is_empty() {
            return Err(invalid("no cell uses it".into()));
        }
        for &c in &moved {
            let cell = &base.cells[c as usize];
            if base.is_lattice(c) {
                return Err(invalid(format!("lattice cell {} uses it", cell.id)));
            }
            if cell.unit == 0 && cell.imp_zero && base.units[0].outside.is_some() {
                return Err(invalid(format!(
                    "it bounds the problem (IMP:N=0 cell {})",
                    cell.id
                )));
            }
        }
        let mut moved_units: Vec<u32> =
            moved.iter().map(|&c| base.cells[c as usize].unit).collect();
        moved_units.sort_unstable();
        moved_units.dedup();

        let mut fillers: Vec<Vec<(u32, Option<Affine>)>> = vec![Vec::new(); base.units.len()];
        for c in 0..base.cells.len() as u32 {
            for t in base.fill_targets(c) {
                fillers[t.unit as usize].push((c, t.transform));
            }
        }
        let mut below: Vec<u32> = moved_units.clone();
        let mut seen: FxHashSet<u32> = below.iter().copied().collect();
        let mut k = 0;
        while k < below.len() {
            let u = below[k] as usize;
            for &c in &base.units[u].cells {
                for t in base.fill_targets(c) {
                    if seen.insert(t.unit) {
                        below.push(t.unit);
                    }
                }
            }
            k += 1;
        }
        below.sort_unstable();

        // Where each unit's records are in the export (as `export_lean`).
        let mut export_bvh = Vec::with_capacity(base.units.len());
        let mut export_grid = Vec::with_capacity(base.units.len());
        let mut export_always = Vec::with_capacity(base.units.len());
        let (mut nodes, mut items, mut gnodes, mut gitems, mut always) =
            (0u32, 0u32, 0u32, 0u32, 0u32);
        let mut arena = base.code.len() as u32;
        let mut kept = FxHashMap::default();
        for (u, unit) in base.units.iter().enumerate() {
            export_bvh.push((nodes, items));
            if !unit.bvh.nodes.is_empty() {
                nodes += unit.bvh.nodes.len() as u32;
                items += unit.bvh.items.len() as u32;
            }
            export_always.push(Span {
                start: always,
                len: unit.always.len() as u32,
            });
            always += unit.always.len() as u32;
            match &unit.grid {
                None => export_grid.push((NONE, 0)),
                Some(g) => {
                    export_grid.push((gnodes, gitems));
                    let mine: Vec<u32> = moved
                        .iter()
                        .copied()
                        .filter(|&c| base.cells[c as usize].unit == u as u32)
                        .collect();
                    if !mine.is_empty() && mine.len() <= MAX_KEPT_CELLS {
                        let entries: Vec<(u32, u32)> = g
                            .items
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| mine.binary_search(c).is_ok())
                            .map(|(k, _)| {
                                let v = match g.progs.get(k).copied().unwrap_or(PROG_FULL) {
                                    PROG_FULL => GRID_PROG_FULL,
                                    PROG_TRUE => GRID_PROG_TRUE,
                                    p => arena + p,
                                };
                                (gitems + k as u32, v)
                            })
                            .collect();
                        kept.insert(u as u32, entries);
                    }
                    gnodes += g.nodes.len() as u32;
                    gitems += g.items.len() as u32;
                    arena += g.arena.len() as u32;
                }
            }
        }

        let mut shape_starts = Vec::with_capacity(shapes.len());
        let mut at = 0usize;
        for (s, shape) in base.shapes.iter().enumerate() {
            if shapes.binary_search(&(s as u32)).is_ok() {
                shape_starts.push(at as u32);
            }
            at += export::shape_len(shape);
        }
        let mut work = base.clone_shared();
        for &u in &moved_units {
            work.units[u as usize].grid = None;
            work.units[u as usize].pending = true;
        }
        let info = Bounder::new(&base.shapes).info;
        let mut d = Draft {
            base,
            work,
            surface,
            kind,
            last: coeffs.clone(),
            coeffs,
            tr,
            shapes,
            shape_starts,
            moved,
            moved_units,
            kept,
            fillers,
            info,
            bvh_pos: FxHashMap::default(),
            dirty_cells: FxHashSet::default(),
            dirty_units: FxHashSet::default(),
            extras: FxHashMap::default(),
            export_bvh,
            export_grid,
            export_always,
            below,
        };
        for u in d.moved_units.clone() {
            d.positions(u);
        }
        Ok(d)
    }

    /// The model the draft started from.
    pub fn base(&self) -> &Arc<Model> {
        &self.base
    }

    /// The surface dragged.
    pub fn surface(&self) -> i64 {
        self.surface
    }

    /// The surface's type and its coefficients in the base model.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn coeffs(&self) -> &[f64] {
        &self.coeffs
    }

    /// The cells that use the surface (indices into [`Model::cells`]).
    pub fn moved_cells(&self) -> &[u32] {
        &self.moved
    }

    /// The units whose octree the CPU no longer uses (the moved cells').
    pub fn units_without_octree(&self) -> &[u32] {
        &self.moved_units
    }

    /// The units whose octree a GPU copy keeps ([`DraftPatch`]).
    pub fn units_kept_on_gpu(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.kept.keys().copied().collect();
        v.sort_unstable();
        v
    }

    /// A copy of the draft's model as it is now (octrees shared): exact for
    /// every query, pending (it cannot be saved).
    pub fn snapshot(&self) -> Model {
        self.work.clone_shared()
    }

    /// `skip` (of the base model) with nothing skipped in the units a draft
    /// can change (the moved cells' and those below them): boxes and
    /// octree nodes there may hold cells anywhere now.
    pub fn widen_skip(&self, skip: &Skip) -> Skip {
        let mut s = skip.clone();
        for &u in &self.below {
            let u = u as usize;
            s.boxes[u] = Aabb::INFINITE;
            s.nodes[u] = None;
            s.walk[u] = false;
        }
        s
    }

    fn positions(&mut self, u: u32) {
        if !self.bvh_pos.contains_key(&u) {
            let pos = self.base.units[u as usize]
                .bvh
                .items
                .iter()
                .enumerate()
                .map(|(i, &c)| (c, i))
                .collect();
            self.bvh_pos.insert(u, pos);
        }
    }

    /// Move the surface to `coeffs` (as written on its card, its `TR`
    /// applied as the compiler does). [`Error::Invalid`] if they do not make
    /// a surface of the same shapes (another type, or a number of
    /// coefficients that changes it), or a moved cell becomes unbounded in a
    /// universe without a region; the draft is then as before.
    pub fn set(&mut self, coeffs: &[f64]) -> Result<DraftPatch> {
        let surface = self.surface;
        let invalid = move |m: String| Error::Invalid(format!("surface {surface}: {m}"));
        let built = build_surface(&self.kind, coeffs, self.tr.as_ref()).map_err(invalid)?;
        if built.len() != self.shapes.len()
            || built.iter().zip(&self.shapes).any(|(b, &s)| {
                std::mem::discriminant(b) != std::mem::discriminant(&self.base.shapes[s as usize])
            })
        {
            return Err(invalid("these coefficients change its shape".into()));
        }
        let base = self.base.clone();
        // Back to the base model where an earlier step reached.
        for &c in &self.dirty_cells {
            let (b, w) = (&base.cells[c as usize], &mut self.work.cells[c as usize]);
            w.bbox = b.bbox;
            let start = b.node_start as usize;
            let n = node_box_count(base.code(c));
            self.work.node_boxes[start..start + n]
                .copy_from_slice(&base.node_boxes[start..start + n]);
        }
        for &u in &self.dirty_units {
            let u = u as usize;
            self.work.units[u].roi = base.units[u].roi;
            self.work.units[u].always = base.units[u].always.clone();
        }
        for (&s, shape) in self.shapes.iter().zip(built) {
            self.info[s as usize] = match &shape {
                Shape::Quadric(q) => analyse(q),
                Shape::Cone(c) => analyse(&c.q),
                _ => None,
            };
            self.work.shapes[s as usize] = shape;
        }

        let shapes = std::mem::take(&mut self.work.shapes);
        let bounder = Bounder {
            shapes: &shapes,
            info: std::mem::take(&mut self.info),
        };
        let mut result = bound_cells(&mut self.work, &bounder, &self.moved);
        let mut touched: Vec<u32> = self.moved.clone();
        let mut units: FxHashSet<u32> = self.moved_units.iter().copied().collect();
        // Universes filled by cells whose box grew beyond what they had:
        // their region grows, and their cells are bound again in it.
        let mut frontier = self.moved.clone();
        while result.is_ok() && !frontier.is_empty() {
            let mut targets: Vec<u32> = frontier
                .iter()
                .flat_map(|&c| self.work.fill_targets(c).map(|t| t.unit))
                .collect();
            targets.sort_unstable();
            targets.dedup();
            frontier.clear();
            for t in targets {
                let region = self.fillers[t as usize]
                    .iter()
                    .map(|(c, tr)| {
                        // As `compute_boxes_with`: a lattice hands its
                        // element, a universe its filling cell's box.
                        let cell = &self.work.cells[*c as usize];
                        let b = match &cell.fill {
                            Some(Fill::Lattice(l)) => lattice_element_box(
                                &self.work,
                                &bounder,
                                *c,
                                &self.work.lattices[*l as usize],
                                self.work.units[cell.unit as usize].roi,
                            ),
                            _ => cell.bbox,
                        };
                        match tr {
                            Some(tr) => b.transformed(tr),
                            None => b,
                        }
                    })
                    .fold(Aabb::EMPTY, |a, b| a.union(&b));
                let orig = base.units[t as usize].roi;
                if contains(&orig, &region) {
                    continue;
                }
                self.work.units[t as usize].roi = orig.union(&region);
                let cells = base.units[t as usize].cells.clone();
                if let Err(e) = bound_cells(&mut self.work, &bounder, &cells) {
                    result = Err(e);
                    break;
                }
                units.insert(t);
                touched.extend_from_slice(&cells);
                frontier.extend_from_slice(&cells);
            }
        }
        self.info = bounder.info;
        self.work.shapes = shapes;
        self.dirty_cells.extend(touched.iter().copied());
        self.dirty_units.extend(units.iter().copied());
        if let Err(e) = result {
            // Back to the last step that worked (it works again).
            let last = self.last.clone();
            self.set(&last)
                .expect("the coefficients of the last step make a draft");
            return Err(invalid(e));
        }
        // BVHs refitted over the boxes as they are now; a cell outside its
        // BVH (empty in the base model) that is not empty now is checked
        // everywhere instead.
        self.extras.clear();
        let dirty_units: Vec<u32> = self.dirty_units.iter().copied().collect();
        for &u in &dirty_units {
            self.positions(u);
        }
        let mut by_unit: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
        for &c in &self.dirty_cells {
            by_unit
                .entry(self.work.cells[c as usize].unit)
                .or_default()
                .push(c);
        }
        for (u, cells) in by_unit {
            let pos = &self.bvh_pos[&u];
            let base_always = &base.units[u as usize].always;
            let unit = &mut self.work.units[u as usize];
            for c in cells {
                let b = self.work.cells[c as usize].bbox;
                match pos.get(&c) {
                    Some(&p) => unit.bvh.boxes[p] = b,
                    None => {
                        if !b.is_empty() && !base_always.contains(&c) {
                            unit.always.push(c);
                            self.extras.entry(u).or_default().push(c);
                        }
                    }
                }
            }
            unit.bvh.refit();
        }
        self.last = coeffs.to_vec();
        Ok(self.patch(&self.work, false))
    }

    /// The patch that puts a GPU copy back as the base model was uploaded:
    /// everything any step changed.
    pub fn revert_patch(&self) -> DraftPatch {
        self.patch(&self.base, true)
    }

    fn patch(&self, m: &Model, restoring: bool) -> DraftPatch {
        let mut p = DraftPatch::default();
        let mut usage = Aabb::EMPTY;
        for &c in &self.moved {
            let b = m.cells[c as usize].bbox;
            if b.is_finite() && (0..3).all(|i| b.hi[i] - b.lo[i] <= 1e4) {
                usage = usage.union(&b);
            }
        }
        for (&s, &start) in self.shapes.iter().zip(&self.shape_starts) {
            let (kind, data) = export::shape_data(&m.shapes[s as usize]);
            p.shapes.push(PatchShape {
                shape: s,
                start,
                kind,
                data,
                usage,
            });
        }
        let mut cells: Vec<u32> = self.dirty_cells.iter().copied().collect();
        cells.sort_unstable();
        for &c in &cells {
            let cell = &m.cells[c as usize];
            p.cells.push((c, cell.bbox));
            let start = cell.node_start as usize;
            for k in 0..node_box_count(m.code(c)) {
                p.node_boxes
                    .push(((start + k) as u32, m.node_boxes[start + k]));
            }
        }
        let mut units: Vec<u32> = self.dirty_units.iter().copied().collect();
        units.sort_unstable();
        for &u in &units {
            let unit = &m.units[u as usize];
            let (nodes, items) = self.export_bvh[u as usize];
            for (i, n) in unit.bvh.nodes.iter().enumerate() {
                p.bvh_nodes.push((nodes + i as u32, n.bbox));
            }
            for (i, b) in unit.bvh.boxes.iter().enumerate() {
                p.bvh_items.push((items + i as u32, *b));
            }
            let span = self.export_always[u as usize];
            let base_root = self.export_grid[u as usize].0;
            let moved_here = self.moved_units.binary_search(&u).is_ok();
            let kept = self.kept.contains_key(&u);
            let extras = self.extras.get(&u).filter(|e| !e.is_empty());
            let (grid_root, always) = if restoring || (!moved_here && extras.is_none()) {
                (base_root, PatchAlways::Export(span))
            } else {
                let mut list = self.base.units[u as usize].always.clone();
                if kept {
                    list.extend(
                        self.moved
                            .iter()
                            .filter(|&&c| self.base.cells[c as usize].unit == u),
                    );
                }
                if let Some(e) = extras {
                    let add: Vec<u32> = e.iter().copied().filter(|c| !list.contains(c)).collect();
                    list.extend(add);
                }
                let root = if !moved_here || kept { base_root } else { NONE };
                if list.len() == span.len as usize {
                    (root, PatchAlways::Export(span))
                } else {
                    (root, PatchAlways::Own(list))
                }
            };
            p.units.push(PatchUnit {
                unit: u,
                roi: unit.roi,
                grid_root,
                always,
            });
        }
        for entries in self.kept.values() {
            for &(k, v) in entries {
                p.grid_progs
                    .push((k, if restoring { v } else { GRID_PROG_FULL }));
            }
        }
        p
    }
}

fn contains(outer: &Aabb, b: &Aabb) -> bool {
    b.is_empty() || (0..3).all(|i| outer.lo[i] <= b.lo[i] && b.hi[i] <= outer.hi[i])
}

/// Bound `cells` again in their unit's region of `m`, their node boxes left
/// unbounded. An error if one is unbounded in a region without bounds.
fn bound_cells(m: &mut Model, bounder: &Bounder, cells: &[u32]) -> std::result::Result<(), String> {
    let w = &*m;
    let boxes: Vec<Option<Aabb>> = cells
        .par_iter()
        .map(|&c| {
            let cell = &w.cells[c as usize];
            let roi = w.units[cell.unit as usize].roi;
            if cell.unsupported.is_some() || w.is_lattice(c) {
                return Some(roi);
            }
            let b = bounder.bound(w.code(c), roi);
            if b.is_empty() || b.is_finite() {
                Some(b)
            } else if roi.is_finite() {
                Some(roi)
            } else {
                None
            }
        })
        .collect();
    for (&c, b) in cells.iter().zip(boxes) {
        let Some(b) = b else {
            return Err(format!(
                "cell {} would be unbounded",
                m.cells[c as usize].id
            ));
        };
        let start = m.cells[c as usize].node_start as usize;
        let n = node_box_count(m.code(c));
        m.cells[c as usize].bbox = b;
        m.node_boxes[start..start + n].fill([Aabb::INFINITE, Aabb::INFINITE]);
    }
    Ok(())
}
