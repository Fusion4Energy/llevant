//! Recompiling against an older version of a model: the front (parse,
//! surfaces, cells, assembly) runs in full, and each universe whose content
//! is unchanged takes its cell boxes, node boxes and octree from the old
//! model instead of computing them again.
//!
//! A unit is reused when, in the old model, the unit of the same universe
//! - has the same region of interest (bit for bit),
//! - holds the same cells (by number) in the same order,
//! - whose programs are word for word the same (box words aside) up to the
//!   shape indices, the shapes they name having the same coefficients (bit
//!   for bit),
//! - and whose lattice and unsupported flags are the same,
//!
//! under the same tolerance, world and grid options. Everything a unit's
//! boxes and octree are computed from is then the same, so reusing them
//! gives exactly what computing them again would: a recompiled model,
//! completed, saves the same bytes as a fresh compile (`tests/recompile.rs`).
//!
//! The indices may have moved (cells or surfaces added or removed before
//! them, a new assembly): the old cell indices map to the new ones in the
//! same order, and the old shape indices one to one to the new ones. The
//! octree then gets its candidates and its residual programs' leaves
//! rewritten; boxes need nothing (node-box ids are local to each cell, and
//! the positions in a program are the same).

use std::sync::Arc;

use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::bbox::Aabb;
use crate::expr::{MASK, NODE, NO_BOX};
use crate::grid::Octree;
use crate::model::{Model, NodeBox, Options};

/// 128 bits of a BLAKE3 hash.
type Digest = [u8; 16];

fn digest(bytes: &[u8]) -> Digest {
    blake3::hash(bytes).as_bytes()[..16].try_into().unwrap()
}

/// A digest of each shape's exact coefficients.
fn shape_digests(m: &Model) -> Vec<Digest> {
    m.shapes
        .par_iter()
        .map(|s| digest(&bitcode::serialize(s).expect("shapes serialize")))
        .collect()
}

/// What changed between a model and its recompiled version.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct Changed {
    /// Universes whose boxes and octree were computed again (all of them on
    /// a full compile).
    pub universes_rebuilt: Vec<i64>,
    /// Cells (MCNP numbers) that are new, or whose geometry or box changed.
    pub cells_changed: Vec<i64>,
    /// Cells (MCNP numbers) of the old model that are gone.
    pub cells_removed: Vec<i64>,
}

impl Changed {
    /// What changed from `old` to `new`, two compiled versions of a model
    /// (neither rebuilt anything: `universes_rebuilt` is empty).
    pub fn between(old: &Model, new: &Model) -> Changed {
        let (old_shapes, new_shapes) = rayon::join(|| shape_digests(old), || shape_digests(new));
        let base = Base {
            old,
            old_shapes,
            new_shapes,
            unit_of: FxHashMap::default(),
        };
        let mut c = base.changes(new, &vec![true; new.units.len()]);
        c.universes_rebuilt.clear();
        c
    }

    /// Everything changed: no unit was reused from `old`.
    pub(crate) fn everything(new: &Model, old: &Model) -> Changed {
        let mut cells_changed: Vec<i64> = new.cells.iter().map(|c| c.id).collect();
        let mut cells_removed: Vec<i64> = old
            .cells
            .iter()
            .filter(|c| new.cell_index(c.id).is_none())
            .map(|c| c.id)
            .collect();
        cells_changed.sort_unstable();
        cells_removed.sort_unstable();
        Changed {
            universes_rebuilt: new.units.iter().map(|u| u.universe).collect(),
            cells_changed,
            cells_removed,
        }
    }
}

/// The old model, as a recompile reads it.
pub(crate) struct Base<'a> {
    old: &'a Model,
    old_shapes: Vec<Digest>,
    new_shapes: Vec<Digest>,
    unit_of: FxHashMap<i64, u32>,
}

impl<'a> Base<'a> {
    /// `None` when nothing can be reused: other options, or an old model
    /// cut from another by [`Model::universe`].
    pub(crate) fn new(old: &'a Model, new: &Model, opts: &Options) -> Option<Base<'a>> {
        let half = opts.world_half_size;
        let world = Aabb {
            lo: [-half; 3],
            hi: [half; 3],
        };
        let same = old.cut.is_none()
            && old.tolerance.to_bits() == opts.tolerance.to_bits()
            && same_box(&old.world, &world)
            && old.grid_options == (opts.grid, opts.residuals);
        if !same {
            return None;
        }
        let (old_shapes, new_shapes) = rayon::join(|| shape_digests(old), || shape_digests(new));
        let unit_of = old
            .units
            .iter()
            .enumerate()
            .map(|(i, u)| (u.universe, i as u32))
            .collect();
        Some(Base {
            old,
            old_shapes,
            new_shapes,
            unit_of,
        })
    }

    /// The old unit that unit `u` of `new` (its region `roi`) can take its
    /// boxes and octree from, if any.
    pub(crate) fn old_unit(&self, new: &Model, u: u32, roi: &Aabb) -> Option<u32> {
        let unit = &new.units[u as usize];
        let ou = *self.unit_of.get(&unit.universe)?;
        let old_unit = &self.old.units[ou as usize];
        if !same_box(&old_unit.roi, roi) {
            return None;
        }
        self.remap(new, ou, u).map(|_| ou)
    }

    /// How the indices of old unit `ou` map to those of unit `u` of `new`,
    /// if its content is the same.
    fn remap(&self, new: &Model, ou: u32, u: u32) -> Option<Remap> {
        let a = &self.old.units[ou as usize].cells;
        let b = &new.units[u as usize].cells;
        // Same numbers in the same order, and indices that keep it: the
        // candidates of every octree leaf stay in the same order.
        if a.len() != b.len() || !a.is_sorted_by(|x, y| x < y) || !b.is_sorted_by(|x, y| x < y) {
            return None;
        }
        let pairs: Vec<Vec<(u32, u32)>> = a
            .par_iter()
            .zip(b)
            .map(|(&x, &y)| self.same_cell(new, x, y))
            .collect::<Option<_>>()?;
        let mut shapes: FxHashMap<u32, u32> = FxHashMap::default();
        let mut back: FxHashMap<u32, u32> = FxHashMap::default();
        for (s, t) in pairs.into_iter().flatten() {
            if *shapes.entry(s).or_insert(t) != t || *back.entry(t).or_insert(s) != s {
                return None;
            }
        }
        Some(Remap {
            cells: a.iter().copied().zip(b.iter().copied()).collect(),
            shapes,
        })
    }

    /// Whether old cell `oc` and cell `nc` of `new` are the same cell (its
    /// pairs of old and new shape indices, if they are).
    fn same_cell(&self, new: &Model, oc: u32, nc: u32) -> Option<Vec<(u32, u32)>> {
        let (a, b) = (&self.old.cells[oc as usize], &new.cells[nc as usize]);
        if a.id != b.id
            || a.code_len != b.code_len
            || a.unsupported.is_some() != b.unsupported.is_some()
            || self.old.is_lattice(oc) != new.is_lattice(nc)
        {
            return None;
        }
        let (x, y) = (self.old.code(oc), new.code(nc));
        if b.unsupported.is_some() {
            // Not a program to walk (what lowering left), and no boxes:
            // the same words are the same cell.
            return (x == y).then(Vec::new);
        }
        let mut pairs = Vec::new();
        let mut pos = 0;
        while pos < x.len() {
            let (v, w) = (x[pos], y[pos]);
            if w & NODE != 0 {
                if v != w {
                    return None;
                }
                // The box word that follows is an output of the boxes.
                pos += 2;
            } else {
                let (s, t) = (v & MASK, w & MASK);
                if v & !MASK != w & !MASK
                    || self.old_shapes[s as usize] != self.new_shapes[t as usize]
                {
                    return None;
                }
                pairs.push((s, t));
                pos += 1;
            }
        }
        Some(pairs)
    }

    /// The boxes of the cells of old unit `ou`, as computing them gives
    /// them: each cell's box and its node boxes in the order they were made.
    pub(crate) fn boxes(&self, ou: u32) -> Vec<(Aabb, Vec<NodeBox>)> {
        let old = self.old;
        old.units[ou as usize]
            .cells
            .par_iter()
            .map(|&c| {
                let cell = &old.cells[c as usize];
                if cell.unsupported.is_some() {
                    return (cell.bbox, Vec::new());
                }
                let code = old.code(c);
                let boxes = old.node_boxes_of(c);
                let mut nodes: Vec<(u32, NodeBox)> = Vec::new();
                let mut pos = 0;
                while pos < code.len() {
                    if code[pos] & NODE != 0 {
                        let bw = code[pos + 1];
                        if bw != NO_BOX {
                            let k = bw & !NODE;
                            let [t, f] = boxes[k as usize];
                            nodes.push((k, (pos, t, f)));
                        }
                        pos += 2;
                    } else {
                        pos += 1;
                    }
                }
                nodes.sort_by_key(|n| n.0);
                (
                    old.cells[c as usize].bbox,
                    nodes.into_iter().map(|n| n.1).collect(),
                )
            })
            .collect()
    }

    /// Give unit `u` of `new` the octree of old unit `ou` (or leave it
    /// pending, if the old one still is).
    pub(crate) fn take_grid(&self, new: &mut Model, u: u32, ou: u32) {
        let old = &self.old.units[ou as usize];
        let grid = match &old.grid {
            Some(g) if !old.pending => {
                let r = self.remap(new, ou, u).expect("a reused unit");
                if r.is_identity() {
                    Some(g.clone())
                } else {
                    Some(Arc::new(r.octree(g)))
                }
            }
            _ => None,
        };
        let unit = &mut new.units[u as usize];
        unit.grid = grid;
        unit.pending = old.pending;
    }

    /// What changed from the old model to `new`, whose units `reused` took
    /// their boxes from it.
    pub(crate) fn changes(&self, new: &Model, reused: &[bool]) -> Changed {
        let universes_rebuilt = new
            .units
            .iter()
            .zip(reused)
            .filter(|(_, r)| !**r)
            .map(|(u, _)| u.universe)
            .collect();
        let (old_cells, new_cells) = rayon::join(
            || cell_digests(self.old, &self.old_shapes),
            || cell_digests(new, &self.new_shapes),
        );
        let mut cells_changed: Vec<i64> = new
            .cells
            .par_iter()
            .zip(&new_cells)
            .filter(|(c, d)| {
                self.old
                    .cell_index(c.id)
                    .is_none_or(|i| old_cells[i as usize] != **d)
            })
            .map(|(c, _)| c.id)
            .collect();
        let mut cells_removed: Vec<i64> = self
            .old
            .cells
            .iter()
            .filter(|c| new.cell_index(c.id).is_none())
            .map(|c| c.id)
            .collect();
        cells_changed.sort_unstable();
        cells_removed.sort_unstable();
        Changed {
            universes_rebuilt,
            cells_changed,
            cells_removed,
        }
    }
}

/// Old index to new, of the cells of a reused unit and the shapes they name.
struct Remap {
    cells: FxHashMap<u32, u32>,
    shapes: FxHashMap<u32, u32>,
}

impl Remap {
    fn is_identity(&self) -> bool {
        self.cells.iter().all(|(a, b)| a == b) && self.shapes.iter().all(|(a, b)| a == b)
    }

    /// The old octree `g` with the new indices: its candidates, and the
    /// leaves of its residual programs (whose node and box words, local to
    /// the cell, stay).
    fn octree(&self, g: &Octree) -> Octree {
        let items = g.items.par_iter().map(|c| self.cells[c]).collect();
        // The arena is programs end to end.
        let mut arena = g.arena.clone();
        let mut pos = 0;
        while pos < arena.len() {
            let w = arena[pos];
            if w & NODE != 0 {
                pos += 2;
            } else {
                arena[pos] = (w & !MASK) | self.shapes[&(w & MASK)];
                pos += 1;
            }
        }
        Octree {
            root: g.root,
            nodes: g.nodes.clone(),
            items,
            progs: g.progs.clone(),
            arena,
        }
    }
}

/// A digest of each cell's geometry (its program, with the coefficients of
/// its shapes, wherever they are) and box.
fn cell_digests(m: &Model, shapes: &[Digest]) -> Vec<Digest> {
    (0..m.cells.len() as u32)
        .into_par_iter()
        .map(|c| {
            let mut h = blake3::Hasher::new();
            let cell = &m.cells[c as usize];
            h.update(&[cell.unsupported.is_some() as u8, m.is_lattice(c) as u8]);
            for v in cell.bbox.lo.iter().chain(&cell.bbox.hi) {
                h.update(&v.to_bits().to_le_bytes());
            }
            let code = m.code(c);
            if cell.unsupported.is_some() {
                for w in code {
                    h.update(&w.to_le_bytes());
                }
                return h.finalize().as_bytes()[..16].try_into().unwrap();
            }
            let mut pos = 0;
            while pos < code.len() {
                let w = code[pos];
                if w & NODE != 0 {
                    h.update(&w.to_le_bytes());
                    pos += 2;
                } else {
                    h.update(&(w & !MASK).to_le_bytes());
                    h.update(&shapes[(w & MASK) as usize]);
                    pos += 1;
                }
            }
            h.finalize().as_bytes()[..16].try_into().unwrap()
        })
        .collect()
}

fn same_box(a: &Aabb, b: &Aabb) -> bool {
    (0..3).all(|i| a.lo[i].to_bits() == b.lo[i].to_bits() && a.hi[i].to_bits() == b.hi[i].to_bits())
}
