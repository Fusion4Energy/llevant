//! A universe of a compiled model as a model of its own, cut to a box.

use std::time::Instant;

use rustc_hash::FxHashMap;

use crate::bbox::Aabb;
use crate::bvh::Bvh;
use crate::error::{Error, Result};
use crate::expr::{MASK, NODE, NO_BOX, OR, POS};
use crate::lattice::{LatticeFill, Target};
use crate::model::{
    build_bvhs, build_grids, compute_boxes, Cell, Fill, GridMode, Model, Scope, SurfaceId, Unit,
};
use crate::surface::Shape;

/// The box a universe opened on its own is cut to ([`Model::universe`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cut {
    /// Around what the universe holds, measured ([`Model::extent`] of its
    /// cells with a material or a fill) and widened by 1 %. Where its cells
    /// are not bounded by their own surfaces (CAD cells often get finite
    /// boxes only from the envelopes around them), measured within where
    /// the model uses it, grown by half its size on every side, so that
    /// parts the envelopes clip are seen.
    Own,
    /// This box, in the universe's frame.
    Box(Aabb),
    /// No cut: the universe's cells reach as far as the world box.
    None,
}

/// Margin around a measured [`Cut::Own`] box, relative to its largest edge.
const OWN_MARGIN: f64 = 0.01;
/// How far beyond where the model uses a universe [`Cut::Own`] looks for
/// what it holds, relative to the largest edge of that region.
const USED_MARGIN: f64 = 0.5;

fn largest_edge(b: &Aabb) -> f64 {
    (0..3).map(|i| b.hi[i] - b.lo[i]).fold(0.0, f64::max)
}

fn padded(b: &Aabb, pad: f64) -> Aabb {
    Aabb {
        lo: b.lo.map(|x| x - pad),
        hi: b.hi.map(|x| x + pad),
    }
}

/// `b` grown by `f` times its largest edge on every side.
fn grown(b: &Aabb, f: f64) -> Aabb {
    padded(b, f * largest_edge(b))
}

impl Model {
    /// Universe `universe` as level 0 of a model of its own, in its own frame,
    /// with everything it holds down to the deepest level. Cut to a box
    /// (unless [`Cut::None`]): beyond it lies one void `IMP:N=0` cell
    /// ([`Model::cut_cell`]), and that box is the new model's
    /// [`Model::problem_box`], so inside it every answer is the one the
    /// universe gives on its own. Cell and universe numbers stay the same.
    /// Built from this model (no parsing), with octrees as `grid` says.
    pub fn universe(&self, universe: i64, cut: Cut, grid: GridMode) -> Result<Model> {
        let t = Instant::now();
        let root = self
            .units
            .iter()
            .position(|u| u.universe == universe)
            .ok_or(Error::NoUniverse(universe))? as u32;
        let half = self.world.hi[0];
        let mut m = self.skeleton(root);
        let b = match cut {
            Cut::None => None,
            Cut::Box(b) => {
                if !b.is_finite() || (0..3).any(|i| b.lo[i] >= b.hi[i]) {
                    return Err(Error::Invalid(format!(
                        "the cut box must be finite and not flat: {b:?}"
                    )));
                }
                Some(b)
            }
            Cut::Own => {
                // What it holds, bounded by its own surfaces; else (CAD
                // cells often get finite boxes only from the envelopes
                // around them) within where the model uses it, grown so that
                // parts the envelopes clip are seen.
                let mut b = self.measure(root, None, half, Scope::Contents);
                let used = self.units[root as usize].roi;
                if b.is_empty() && used.is_finite() {
                    let g = grown(&used, USED_MARGIN);
                    b = self.measure(root, Some(g), half, Scope::Contents);
                    if b.is_empty() {
                        b = g;
                    }
                }
                if b.is_empty() {
                    b = self.measure(root, None, half, Scope::Problem);
                }
                b.is_finite()
                    .then(|| padded(&b, (OWN_MARGIN * largest_edge(&b)).max(1e-3)))
            }
        };
        if let Some(b) = b {
            m.add_cut(&b);
        }
        compute_boxes(&mut m, half);
        build_bvhs(&mut m);
        m.grid_options = (grid, true);
        for u in &mut m.units {
            u.pending = true;
        }
        build_grids(&mut m, grid, true, &crate::Progress::new())?;
        m.timings = vec![("universe", t.elapsed().as_secs_f64())];
        Ok(m)
    }

    /// The void `IMP:N=0` cell beyond the box of a model made by
    /// [`Model::universe`], if it was cut.
    pub fn cut_cell(&self) -> Option<u32> {
        self.cut.map(|c| c.0)
    }

    /// Unit `root` and every unit it reaches, `root` first, with cells,
    /// programs, surfaces and lattices renumbered compactly; no boxes yet.
    fn skeleton(&self, root: u32) -> Model {
        const NONE: u32 = u32::MAX;
        let mut unit_map = vec![NONE; self.units.len()];
        unit_map[root as usize] = 0;
        let mut order = vec![root];
        let mut i = 0;
        while i < order.len() {
            for &c in &self.units[order[i] as usize].cells {
                for t in self.fill_targets(c) {
                    if unit_map[t.unit as usize] == NONE {
                        unit_map[t.unit as usize] = order.len() as u32;
                        order.push(t.unit);
                    }
                }
            }
            i += 1;
        }
        let target = |t: &Target| Target {
            unit: unit_map[t.unit as usize],
            transform: t.transform,
        };

        let mut shape_map = vec![NONE; self.shapes.len()];
        let mut shapes = Vec::new();
        let mut surface_ids = Vec::new();
        let mut shape = |s: u32| {
            let k = &mut shape_map[s as usize];
            if *k == NONE {
                *k = shapes.len() as u32;
                shapes.push(self.shapes[s as usize].clone());
                surface_ids.push(self.surface_ids[s as usize]);
            }
            *k
        };
        let mut cell_map = vec![NONE; self.cells.len()];
        let mut cells = Vec::new();
        let mut code = Vec::new();
        let mut lattices = Vec::new();
        for (i, c) in self.cells.iter().enumerate() {
            let unit = unit_map[c.unit as usize];
            if unit == NONE {
                continue;
            }
            cell_map[i] = cells.len() as u32;
            // Leaves renumbered; box words dropped (rebuilt with the boxes).
            let src = self.code(i as u32);
            let start = code.len();
            let mut pos = 0;
            while pos < src.len() {
                let w = src[pos];
                if w & NODE == 0 {
                    code.push((w & !MASK) | shape(w & MASK));
                    pos += 1;
                } else {
                    code.extend([w, NO_BOX]);
                    pos += 2;
                }
            }
            let fill = c.fill.as_ref().map(|f| match f {
                Fill::Universe(t) => Fill::Universe(target(t)),
                Fill::Lattice(l) => {
                    let mut lat = self.lattices[*l as usize].clone();
                    for s in &mut lat.face_surfaces {
                        *s = shape(*s);
                    }
                    lat.fill = match &lat.fill {
                        LatticeFill::Uniform(t) => LatticeFill::Uniform(target(t)),
                        LatticeFill::Array { lo, size, elements } => LatticeFill::Array {
                            lo: *lo,
                            size: *size,
                            elements: elements.iter().map(|e| e.as_ref().map(target)).collect(),
                        },
                    };
                    lattices.push(lat);
                    Fill::Lattice(lattices.len() as u32 - 1)
                }
            });
            cells.push(Cell {
                id: c.id,
                unit,
                material: c.material,
                density: c.density,
                code_start: start,
                code_len: (code.len() - start) as u32,
                node_start: 0,
                bbox: Aabb::INFINITE,
                fill,
                imp_zero: c.imp_zero,
                unsupported: c.unsupported.clone(),
            });
        }
        let units = order
            .iter()
            .map(|&u| {
                let u = &self.units[u as usize];
                Unit {
                    universe: u.universe,
                    cells: u.cells.iter().map(|&c| cell_map[c as usize]).collect(),
                    roi: Aabb::INFINITE,
                    bvh: Bvh::default(),
                    grid: None,
                    pending: false,
                    always: Vec::new(),
                    outside: None,
                }
            })
            .collect();
        let cell_index: FxHashMap<i64, u32> = cells
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id, i as u32))
            .collect();
        let diagnostics = self
            .diagnostics
            .iter()
            .filter(|d| match d.subject {
                crate::Subject::Cell(id) => cell_index.contains_key(&id),
                _ => false,
            })
            .cloned()
            .collect();
        Model {
            shapes,
            surface_ids,
            cells,
            code,
            node_boxes: Vec::new(),
            units,
            lattices,
            cell_index,
            materials: self.materials.clone(),
            tolerance: self.tolerance,
            world: Aabb::EMPTY,
            cut: None,
            diagnostics,
            grid_options: (GridMode::Off, true),
            timings: Vec::new(),
            instances: Default::default(),
        }
    }

    /// [`Model::extent`] of `scope` in unit `root` on its own, cut to
    /// `region` if given (a scratch model, without octrees).
    fn measure(&self, root: u32, region: Option<Aabb>, half: f64, scope: Scope) -> Aabb {
        let mut m = self.skeleton(root);
        if let Some(r) = region {
            m.add_cut(&r);
        }
        compute_boxes(&mut m, half);
        build_bvhs(&mut m);
        m.extent(scope, 64)
    }

    /// Add the void `IMP:N=0` cell beyond box `b` at level 0, bounded by six
    /// new planes numbered after the highest surface number. (The old
    /// programs of the cut cells are left in `code`, unused.)
    fn add_cut(&mut self, b: &Aabb) {
        let first = self.shapes.len() as u32;
        let id = self.surface_ids.iter().map(|s| s.id).max().unwrap_or(0) + 1;
        for axis in 0..3 {
            let mut n = [0.0; 3];
            n[axis] = 1.0;
            for (k, v) in [b.lo[axis], b.hi[axis]].into_iter().enumerate() {
                self.shapes.push(Shape::plane(n, v));
                self.surface_ids.push(SurfaceId {
                    id: id + (2 * axis + k) as i64,
                    facet: 0,
                });
            }
        }
        // `IMP:N=0` cells already there end at the box, so that beyond it
        // the new cell is alone.
        for k in 0..self.units[0].cells.len() {
            let c = self.units[0].cells[k] as usize;
            if !self.cells[c].imp_zero || self.cells[c].unsupported.is_some() {
                continue;
            }
            let start = self.code.len();
            let old = self.cells[c].code_start
                ..self.cells[c].code_start + self.cells[c].code_len as usize;
            self.code.extend([NODE | (7 + old.len() as u32), NO_BOX]);
            for axis in 0..3 {
                let lo = first + 2 * axis as u32;
                self.code.extend([lo | POS, lo + 1]);
            }
            self.code.extend_from_within(old);
            self.cells[c].code_start = start;
            self.cells[c].code_len = (self.code.len() - start) as u32;
        }
        // Below the low plane or above the high one, on any axis.
        let start = self.code.len();
        self.code.extend([NODE | OR | 7, NO_BOX]);
        for axis in 0..3 {
            let lo = first + 2 * axis as u32;
            self.code.extend([lo, (lo + 1) | POS]);
        }
        let id = self.cells.iter().map(|c| c.id).max().unwrap_or(0) + 1;
        let idx = self.cells.len() as u32;
        self.cells.push(Cell {
            id,
            unit: 0,
            material: 0,
            density: 0.0,
            code_start: start,
            code_len: 8,
            node_start: 0,
            bbox: Aabb::INFINITE,
            fill: None,
            imp_zero: true,
            unsupported: None,
        });
        self.units[0].cells.push(idx);
        self.cell_index.insert(id, idx);
        self.cut = Some((idx, *b));
    }
}
