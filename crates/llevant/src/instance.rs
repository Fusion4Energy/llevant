//! Cell instances: each place a cell occupies in the model, numbered.
//!
//! A cell of a universe that fills twenty envelopes is one cell card but
//! twenty pieces of the geometry, and a lattice cell is one piece per
//! element. An *instance* is one such piece. It is named by its path from
//! level 0 down (a [`Label`]'s path, with each lattice element), and
//! numbered here by a dense id in `0 .. total`, so that values can be kept
//! per instance in a plain array (results of a repeated structure, colours)
//! and the GPU can compute the id while it descends.
//!
//! # Numbering
//!
//! - A *site* is a place a universe is put: a cell filled with it
//!   (`FILL=u`), or one element of a lattice whose fill array holds it.
//!   Every instance of the cell holding a site gives one instance of the
//!   universe there, so a universe's instances are a block per site, the
//!   sites in order of their cell (and element), each block as long as the
//!   instances of the universe holding the site:
//!   `instance(u) = offset(site) + instance(host)`.
//! - A lattice filled with one universe everywhere (no fill array) is one
//!   site whose elements are numbered within a range of indices bounding
//!   the lattice cell's box: `instance(u) = offset + instance(host) · E +
//!   element`, where `E` is the number of elements in the range and
//!   `element` the element's index in it (`i` fastest). Elements of the
//!   range the cell does not reach are numbered too, so ids have holes there.
//! - The instances of a cell are those of its universe and, for a lattice
//!   cell, one per element of its range (fill array or bounding range):
//!   `id = base(cell) + instance(universe) · E + element`, with `E = 1` and
//!   no element for other cells. Cells are numbered in order, so a cell's
//!   instances are the ids `base(cell) .. base(cell + 1)`.
//!
//! Unit 0 has one instance. A universe nothing fills has none, nor do its
//! cells. If the numbers outgrow 64 bits, or a lattice cell is unbounded
//! along an axis of its lattice, the ids of what lies below it are `None`.

use crate::classify::{Label, Status, Step};
use crate::lattice::{Lattice, LatticeFill};
use crate::model::{Fill, Model};

/// Elements of a lattice numbered in a box of indices `lo .. lo + size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElementRange {
    pub lo: [i32; 3],
    pub size: [i32; 3],
}

impl ElementRange {
    /// Number of elements.
    pub fn len(&self) -> u64 {
        self.size.iter().map(|&s| s.max(0) as u64).product()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Index of element `ix` in the range (`i` fastest), if inside.
    pub fn flat(&self, ix: [i32; 3]) -> Option<u64> {
        let mut flat = 0u64;
        for i in (0..3).rev() {
            let k = ix[i] - self.lo[i];
            if k < 0 || k >= self.size[i] {
                return None;
            }
            flat = flat * self.size[i] as u64 + k as u64;
        }
        Some(flat)
    }

    /// The element at index `flat` ([`ElementRange::flat`]'s inverse).
    pub fn element(&self, mut flat: u64) -> [i32; 3] {
        let mut ix = [0; 3];
        for i in 0..3 {
            let s = self.size[i].max(1) as u64;
            ix[i] = self.lo[i] + (flat % s) as i32;
            flat /= s;
        }
        ix
    }
}

/// What a filled cell does with the instances of its universe.
#[derive(Clone, Debug)]
enum Sites {
    /// Not filled, or below what can be numbered.
    None,
    /// `FILL=u`: the block of `u`'s instances starting at `offset`.
    Fill { offset: u64 },
    /// A lattice with a fill array: per element (in `range` order), the
    /// offset of its block, or `NONE` for elements filled by the lattice
    /// cell itself.
    Array {
        range: ElementRange,
        offsets: Vec<u64>,
    },
    /// A lattice filled with one universe everywhere.
    Uniform { range: ElementRange, offset: u64 },
}

/// No instance.
const NONE: u64 = u64::MAX;

/// Where a block of a universe's instances comes from.
#[derive(Clone, Copy, Debug)]
struct Block {
    offset: u64,
    /// The filled cell holding the site.
    cell: u32,
    /// For a lattice element of a fill array: its index in the range.
    element: Option<u64>,
}

/// The numbering of every cell instance of a model ([`Model::instances`]).
#[derive(Clone, Debug)]
pub struct Instances {
    /// Instances of each unit.
    units: Vec<u64>,
    /// Per cell: the first id of its instances; one more entry at the end
    /// holds the total.
    base: Vec<u64>,
    /// Per cell: its elements (lattice cells), `None` for other cells.
    ranges: Vec<Option<ElementRange>>,
    sites: Vec<Sites>,
    /// Per unit: the blocks of its instances, by offset.
    blocks: Vec<Vec<Block>>,
}

impl Instances {
    pub(crate) fn new(m: &Model) -> Instances {
        let n_cells = m.cells.len();
        let n_units = m.units.len();
        // The range of every lattice cell.
        let ranges: Vec<Option<ElementRange>> = m
            .cells
            .iter()
            .map(|c| match c.fill {
                Some(Fill::Lattice(l)) => element_range(&m.lattices[l as usize], c),
                _ => None,
            })
            .collect();
        // Sites holding each unit, in order of their cell and element:
        // (cell, element, elements per host instance).
        let mut held: Vec<Vec<(u32, Option<u64>, u64)>> = vec![Vec::new(); n_units];
        for (ci, c) in m.cells.iter().enumerate() {
            let ci = ci as u32;
            match &c.fill {
                None => {}
                Some(Fill::Universe(t)) => held[t.unit as usize].push((ci, None, 1)),
                Some(Fill::Lattice(l)) => {
                    let Some(range) = ranges[ci as usize] else {
                        continue;
                    };
                    match &m.lattices[*l as usize].fill {
                        LatticeFill::Uniform(t) => {
                            held[t.unit as usize].push((ci, None, range.len()))
                        }
                        LatticeFill::Array { elements, .. } => {
                            for (k, e) in elements.iter().enumerate() {
                                if let Some(t) = e {
                                    held[t.unit as usize].push((ci, Some(k as u64), 1));
                                }
                            }
                        }
                    }
                }
            }
        }
        // Instances of each unit, through those of its holders.
        let mut units = vec![None; n_units];
        let mut visiting = vec![false; n_units];
        fn count(
            u: usize,
            m: &Model,
            held: &[Vec<(u32, Option<u64>, u64)>],
            units: &mut [Option<u64>],
            visiting: &mut [bool],
        ) -> u64 {
            if let Some(n) = units[u] {
                return n;
            }
            if u == 0 {
                units[0] = Some(1);
                return 1;
            }
            // A cycle cannot be reached by a point (the engine stops at its
            // depth limit): no instances through it.
            if std::mem::replace(&mut visiting[u], true) {
                return 0;
            }
            let mut n = 0u64;
            for &(c, _, per) in &held[u] {
                let host = m.cells[c as usize].unit as usize;
                let h = count(host, m, held, units, visiting);
                n = n.saturating_add(h.saturating_mul(per));
            }
            visiting[u] = false;
            units[u] = Some(n);
            n
        }
        for u in 0..n_units {
            count(u, m, &held, &mut units, &mut visiting);
        }
        let units: Vec<u64> = units.into_iter().map(|n| n.unwrap_or(0)).collect();

        // Blocks and site offsets.
        let mut sites: Vec<Sites> = m
            .cells
            .iter()
            .enumerate()
            .map(|(ci, c)| match (&c.fill, ranges[ci]) {
                (Some(Fill::Lattice(l)), Some(range)) => match &m.lattices[*l as usize].fill {
                    LatticeFill::Uniform(_) => Sites::Uniform {
                        range,
                        offset: NONE,
                    },
                    LatticeFill::Array { .. } => Sites::Array {
                        range,
                        offsets: vec![NONE; range.len() as usize],
                    },
                },
                (Some(Fill::Universe(_)), _) => Sites::Fill { offset: NONE },
                _ => Sites::None,
            })
            .collect();
        let mut blocks: Vec<Vec<Block>> = vec![Vec::new(); n_units];
        for (u, list) in held.iter().enumerate() {
            let mut offset = 0u64;
            for &(c, element, per) in list {
                let host = units[m.cells[c as usize].unit as usize];
                let size = host.saturating_mul(per);
                if size == 0 {
                    continue;
                }
                blocks[u].push(Block {
                    offset,
                    cell: c,
                    element,
                });
                match (&mut sites[c as usize], element) {
                    (Sites::Fill { offset: o }, _) | (Sites::Uniform { offset: o, .. }, _) => {
                        *o = offset
                    }
                    (Sites::Array { offsets, .. }, Some(k)) => offsets[k as usize] = offset,
                    _ => {}
                }
                offset = offset.saturating_add(size);
            }
        }

        let mut base = Vec::with_capacity(n_cells + 1);
        let mut next = 0u64;
        for (ci, c) in m.cells.iter().enumerate() {
            base.push(next);
            // An unbounded lattice cell has no numbered elements.
            let per = match (&c.fill, ranges[ci]) {
                (Some(Fill::Lattice(_)), None) => 0,
                (_, r) => r.map_or(1, |r| r.len()),
            };
            next = next.saturating_add(units[c.unit as usize].saturating_mul(per));
        }
        base.push(next);
        let mut out = Instances {
            units,
            base,
            ranges,
            sites,
            blocks,
        };
        if next == u64::MAX {
            // Saturated: nothing can be numbered.
            out.base.iter_mut().for_each(|b| *b = 0);
            out.sites.iter_mut().for_each(|s| *s = Sites::None);
        }
        out
    }

    /// Number of instance ids (`0 .. total`; holes included).
    pub fn total(&self) -> u64 {
        *self.base.last().unwrap_or(&0)
    }

    /// Instances of the cell at index `cell`.
    pub fn count(&self, cell: u32) -> u64 {
        self.base[cell as usize + 1] - self.base[cell as usize]
    }

    /// Instances of the unit at index `unit` ([`Model::units`]): 1 for unit
    /// 0, 0 for a universe nothing fills.
    pub fn count_unit(&self, unit: u32) -> u64 {
        self.units[unit as usize]
    }

    /// The ids of the instances of cell `cell`.
    pub fn ids(&self, cell: u32) -> std::ops::Range<u64> {
        self.base[cell as usize]..self.base[cell as usize + 1]
    }

    /// The cell of instance `id`, `None` past the total.
    pub fn cell(&self, id: u64) -> Option<u32> {
        (id < self.total()).then(|| (self.base.partition_point(|&b| b <= id) - 1) as u32)
    }

    /// The elements numbered for a lattice cell (its fill array, or a range
    /// bounding its box), `None` for other cells.
    pub fn elements(&self, cell: u32) -> Option<ElementRange> {
        self.ranges[cell as usize]
    }

    /// For the GPU renderer: how a filled cell's site maps the instance of
    /// its host to that of the universe it holds, `(offset, stride)` with
    /// `instance(u) = offset + instance(host) · stride + element`, where
    /// `element` is the element's index in [`Instances::elements`] for a
    /// lattice filled with one universe (stride > 1 or a lattice without a
    /// fill array) and 0 otherwise. For a lattice with a fill array, give
    /// the element's index in its range as `element`. `None` where no
    /// instance can be numbered.
    pub fn site(&self, cell: u32, element: Option<u64>) -> Option<(u64, u64)> {
        match (&self.sites[cell as usize], element) {
            (Sites::Fill { offset }, _) if *offset != NONE => Some((*offset, 1)),
            (Sites::Uniform { range, offset }, _) if *offset != NONE => {
                Some((*offset, range.len()))
            }
            (Sites::Array { offsets, .. }, Some(k)) => offsets
                .get(k as usize)
                .copied()
                .filter(|&o| o != NONE)
                .map(|o| (o, 1)),
            _ => None,
        }
    }

    /// The id of the instance a path leads to: cells from level 0 down,
    /// each with its lattice element for lattice cells (as in a
    /// [`Label`]). `None` for an empty path, a path the model does not
    /// have, or where ids cannot be numbered.
    pub fn of(&self, m: &Model, path: &[Step]) -> Option<u64> {
        let (last, above) = path.split_last()?;
        let mut unit = 0u32;
        let mut inst = 0u64;
        for s in above {
            if m.cells.get(s.cell as usize)?.unit != unit {
                return None;
            }
            let element = self.element(s)?;
            let (offset, stride) = match &self.sites[s.cell as usize] {
                Sites::Uniform { .. } => {
                    let (o, st) = self.site(s.cell, None)?;
                    (o + element.unwrap_or(0), st)
                }
                _ => self.site(s.cell, element)?,
            };
            inst = offset.checked_add(inst.checked_mul(stride)?)?;
            unit = match &m.cells[s.cell as usize].fill {
                Some(Fill::Universe(t)) => t.unit,
                Some(Fill::Lattice(l)) => match &m.lattices[*l as usize].fill {
                    LatticeFill::Uniform(t) => t.unit,
                    LatticeFill::Array { elements, .. } => {
                        elements.get(element? as usize)?.as_ref()?.unit
                    }
                },
                None => return None,
            };
            if inst >= self.units[unit as usize] {
                return None;
            }
        }
        if m.cells.get(last.cell as usize)?.unit != unit {
            return None;
        }
        let element = self.element(last)?;
        let per = self.ranges[last.cell as usize].map_or(1, |r| r.len());
        let id = self.base[last.cell as usize] + inst * per + element.unwrap_or(0);
        (id < self.base[last.cell as usize + 1]).then_some(id)
    }

    /// A step's element index in its cell's range: `Some(None)` for a cell
    /// that is not a lattice, `None` when the step does not fit its cell.
    fn element(&self, s: &Step) -> Option<Option<u64>> {
        match (self.ranges[s.cell as usize], s.lattice) {
            (None, None) => Some(None),
            (Some(r), Some(ix)) => r.flat(ix).map(Some),
            _ => None,
        }
    }

    /// The cell and the path of instance `id` ([`Instances::of`]'s
    /// inverse), `None` past the total or in a hole.
    pub fn path(&self, m: &Model, id: u64) -> Option<Vec<Step>> {
        if id >= self.total() {
            return None;
        }
        // The last cell whose ids start at or before `id`.
        let cell = self.base.partition_point(|&b| b <= id) - 1;
        let mut rest = id - self.base[cell];
        let mut steps = Vec::new();
        let mut cell = cell as u32;
        let range = self.ranges[cell as usize];
        let per = range.map_or(1, |r| r.len());
        steps.push(Step {
            cell,
            lattice: range.map(|r| r.element(rest % per)),
        });
        rest /= per;
        let mut unit = m.cells[cell as usize].unit;
        while unit != 0 {
            let blocks = &self.blocks[unit as usize];
            let k = blocks
                .partition_point(|b| b.offset <= rest)
                .checked_sub(1)?;
            let b = blocks[k];
            let local = rest - b.offset;
            cell = b.cell;
            let lattice = match (&self.sites[cell as usize], b.element) {
                (Sites::Uniform { range, .. }, _) => {
                    let e = range.len();
                    rest = local / e;
                    Some(range.element(local % e))
                }
                (Sites::Array { range, .. }, Some(k)) => {
                    rest = local;
                    Some(range.element(k))
                }
                _ => {
                    rest = local;
                    None
                }
            };
            steps.push(Step { cell, lattice });
            unit = m.cells[cell as usize].unit;
            if rest >= self.units[unit as usize] {
                return None;
            }
        }
        steps.reverse();
        Some(steps)
    }

    /// The instances a bin of an MCNP tally covers, written in its
    /// repeated-structure notation: `chain` has the bin's levels from the
    /// cell scored outwards, as on the tally card (`(8 9 < 7[0 0 0] < 3)` is
    /// `[[8, 9], [7[0 0 0]], [3]]`, cell indices), and an instance is
    /// covered when its cell is one of the first level's and, going up its
    /// path, each further level has one of its cells there, in order. As in
    /// MCNP, levels in between may be left out (`(8 < 3)` is cell 8
    /// wherever it is below cell 3), and a lattice cell without an element
    /// is every element of it. Ids in increasing order; none for an empty
    /// chain.
    pub fn matching(&self, m: &Model, chain: &[Vec<Step>]) -> Vec<u64> {
        use rayon::prelude::*;
        let Some((first, above)) = chain.split_first() else {
            return Vec::new();
        };
        let covered = |id: u64| {
            let Some(path) = self.path(m, id) else {
                return false;
            };
            let Some((last, up)) = path.split_last() else {
                return false;
            };
            fits(last, first) && climbs(up, above)
        };
        let mut cells: Vec<u32> = first.iter().map(|s| s.cell).collect();
        cells.sort_unstable();
        cells.dedup();
        let mut out = Vec::new();
        for c in cells {
            out.par_extend(self.ids(c).into_par_iter().filter(|&id| covered(id)));
        }
        out
    }

    /// For each instance id, which of a tally's bins (`chains`, each as
    /// [`Instances::matching`] takes it) holds it most closely, or
    /// [`NO_BIN`]. A bin holds an instance when a cell on the instance's
    /// path, its own or one above it, is scored by the bin and the levels
    /// above match (an F4 on a filled cell scores everything inside it). Of
    /// the bins that hold an instance, the one whose cell is deepest on its
    /// path gives it; then the one of more levels; then a single cell at
    /// the first level rather than a union; then the first.
    pub fn innermost_bins(&self, m: &Model, chains: &[Vec<Vec<Step>>]) -> Vec<u32> {
        use rayon::prelude::*;
        use std::cmp::Reverse;
        let mut by_cell: rustc_hash::FxHashMap<u32, Vec<u32>> = Default::default();
        for (k, chain) in chains.iter().enumerate() {
            for s in chain.first().into_iter().flatten() {
                let ks = by_cell.entry(s.cell).or_default();
                if ks.last() != Some(&(k as u32)) {
                    ks.push(k as u32);
                }
            }
        }
        let best = |id: u64| -> u32 {
            let Some(path) = self.path(m, id) else {
                return NO_BIN;
            };
            let mut best = None;
            for (depth, s) in path.iter().enumerate() {
                for &k in by_cell.get(&s.cell).into_iter().flatten() {
                    let chain = &chains[k as usize];
                    if fits(s, &chain[0]) && climbs(&path[..depth], &chain[1..]) {
                        let rank = (depth, chain.len(), chain[0].len() == 1, Reverse(k));
                        if best.is_none_or(|(r, _)| r < rank) {
                            best = Some((rank, k));
                        }
                    }
                }
            }
            best.map_or(NO_BIN, |b| b.1)
        };
        let mut out = vec![NO_BIN; self.total() as usize];
        if by_cell.keys().any(|&c| m.cells[c as usize].fill.is_some()) {
            out.par_iter_mut()
                .enumerate()
                .for_each(|(id, o)| *o = best(id as u64));
        } else {
            // Only the scored cells' own instances.
            for &c in by_cell.keys() {
                let ids = self.ids(c);
                out[ids.start as usize..ids.end as usize]
                    .par_iter_mut()
                    .enumerate()
                    .for_each(|(k, o)| *o = best(ids.start + k as u64));
            }
        }
        out
    }
}

/// No bin holds the instance ([`Instances::innermost_bins`]).
pub const NO_BIN: u32 = u32::MAX;

/// Whether step `s` is one of `level`'s cells (any element of a lattice
/// cell given without one).
fn fits(s: &Step, level: &[Step]) -> bool {
    level
        .iter()
        .any(|l| l.cell == s.cell && (l.lattice.is_none() || l.lattice == s.lattice))
}

/// Whether, going up from the end of `path`, each of `levels` is found in
/// order.
fn climbs(path: &[Step], levels: &[Vec<Step>]) -> bool {
    let mut up = path.iter().rev();
    levels.iter().all(|level| up.any(|s| fits(s, level)))
}

/// The elements of a lattice cell: its fill array, or for a lattice filled
/// with one universe, the indices of the elements its box can reach
/// (widened by one for hexagons, whose index is the nearest of the
/// neighbours). `None` if unbounded along an axis of the lattice.
fn element_range(l: &Lattice, cell: &crate::model::Cell) -> Option<ElementRange> {
    if let LatticeFill::Array { lo, size, .. } = &l.fill {
        return Some(ElementRange {
            lo: *lo,
            size: *size,
        });
    }
    let b = &cell.bbox;
    let mut lo = [0i32; 3];
    let mut size = [1i32; 3];
    for i in 0..3 {
        if !l.bounded[i] {
            continue;
        }
        let (mut a, mut z) = (f64::INFINITY, f64::NEG_INFINITY);
        for corner in 0..8 {
            let p = [0, 1, 2].map(|k| {
                if corner >> k & 1 == 0 {
                    b.lo[k]
                } else {
                    b.hi[k]
                }
            });
            let x = crate::vec3::dot(l.dual[i], crate::vec3::sub(p, l.center)) + 0.5;
            a = a.min(x.floor());
            z = z.max(x.floor());
        }
        if l.hexagonal {
            a -= 1.0;
            z += 1.0;
        }
        if !(a.is_finite() && z.is_finite()) || z - a >= (1u64 << 20) as f64 {
            return None;
        }
        lo[i] = a as i32;
        size[i] = (z - a) as i32 + 1;
    }
    let r = ElementRange { lo, size };
    (r.len() < 1 << 40).then_some(r)
}

impl Model {
    /// The numbering of every cell instance ([`crate::instance`]), made on
    /// first use.
    pub fn instances(&self) -> &Instances {
        self.instances.get_or_init(|| Instances::new(self))
    }

    /// The id of the instance holding a point: for an [`Status::Inside`]
    /// label, its path's instance.
    pub fn instance(&self, label: &Label) -> Option<u64> {
        if label.status != Status::Inside {
            return None;
        }
        self.instances().of(self, &label.path)
    }

    /// The path of instance `id` (from level 0 down, as in a [`Label`]).
    pub fn instance_path(&self, id: u64) -> Option<Vec<Step>> {
        self.instances().path(self, id)
    }

    /// A path in MCNP's repeated-structure notation, deepest cell first:
    /// `12 < 100[1 0 0] < 5` (cell 12, in element `[1 0 0]` of lattice cell
    /// 100, in cell 5 at level 0).
    pub fn format_path(&self, path: &[Step]) -> String {
        let parts: Vec<String> = path
            .iter()
            .rev()
            .map(|s| match s.lattice {
                Some([i, j, k]) => format!("{}[{i} {j} {k}]", self.cell_id(s.cell)),
                None => self.cell_id(s.cell).to_string(),
            })
            .collect();
        parts.join(" < ")
    }

    /// A path written as [`Model::format_path`] writes it. Lattice elements
    /// may have one to three indices (missing ones are 0), separated by
    /// spaces or commas. The path must lead from level 0 down through
    /// cells that fill each other.
    pub fn parse_path(&self, s: &str) -> crate::Result<Vec<Step>> {
        let bad = |why: String| crate::Error::Invalid(format!("path `{s}`: {why}"));
        let mut steps = Vec::new();
        for part in s.split('<').rev() {
            let part = part.trim();
            let (num, lattice) = match part.split_once('[') {
                Some((n, rest)) => {
                    let inner = rest
                        .strip_suffix(']')
                        .ok_or_else(|| bad(format!("missing `]` in `{part}`")))?;
                    let ix: Vec<i32> = inner
                        .split([' ', ','])
                        .filter(|w| !w.is_empty())
                        .map(|w| w.parse().map_err(|_| bad(format!("bad index `{w}`"))))
                        .collect::<crate::Result<_>>()?;
                    if ix.is_empty() || ix.len() > 3 {
                        return Err(bad(format!("give one to three indices in `{part}`")));
                    }
                    let mut e = [0; 3];
                    e[..ix.len()].copy_from_slice(&ix);
                    (n.trim(), Some(e))
                }
                None => (part, None),
            };
            let id: i64 = num
                .parse()
                .map_err(|_| bad(format!("`{num}` is not a cell number")))?;
            let cell = self
                .cell_index(id)
                .ok_or_else(|| bad(format!("no cell {id}")))?;
            steps.push(Step { cell, lattice });
        }
        // Each cell must lie in the universe filling the one above.
        let mut unit = 0u32;
        for (k, st) in steps.iter().enumerate() {
            let c = &self.cells[st.cell as usize];
            if c.unit != unit {
                return Err(bad(format!(
                    "cell {} is not in the universe {}",
                    c.id,
                    if k == 0 {
                        "at level 0".to_string()
                    } else {
                        format!("filling cell {}", self.cell_id(steps[k - 1].cell))
                    }
                )));
            }
            let is_lattice = matches!(c.fill, Some(Fill::Lattice(_)));
            if is_lattice != st.lattice.is_some() {
                return Err(bad(format!(
                    "cell {} {}",
                    c.id,
                    if is_lattice {
                        "is a lattice: give its element, `[i j k]`"
                    } else {
                        "is not a lattice"
                    }
                )));
            }
            if k + 1 < steps.len() {
                unit = match &c.fill {
                    Some(Fill::Universe(t)) => t.unit,
                    Some(Fill::Lattice(l)) => {
                        match self.lattices[*l as usize].element(st.lattice.unwrap_or_default()) {
                            crate::lattice::Element::Fill(t) => t.unit,
                            _ => {
                                return Err(bad(format!(
                                    "element of cell {} holds no universe",
                                    c.id
                                )))
                            }
                        }
                    }
                    None => return Err(bad(format!("cell {} is not filled", c.id))),
                };
            }
        }
        Ok(steps)
    }
}
