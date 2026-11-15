//! Who holds what in a model, for frontends: cells by material and by
//! universe, which cells a universe fills, and the sets of cells above and
//! below others, and the cells each surface bounds. Built once per model
//! from the engine's public API ([`Model::cells`], [`Model::units`],
//! [`Model::fill_targets`]); the cells of surfaces are found when asked
//! ([`Model::cell_surface_leaves`]).

use std::sync::{Arc, OnceLock};

use llevant::Model;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::labels::Labels;

/// A set of cells of one model, by index into [`Model::cells`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CellSet {
    words: Vec<u64>,
    len: usize,
}

impl CellSet {
    /// No cell of a model with `len` cells.
    pub fn empty(len: usize) -> CellSet {
        CellSet {
            words: vec![0; len.div_ceil(64)],
            len,
        }
    }

    /// Every cell of a model with `len` cells.
    pub fn full(len: usize) -> CellSet {
        let mut s = CellSet {
            words: vec![u64::MAX; len.div_ceil(64)],
            len,
        };
        s.trim();
        s
    }

    pub fn from_cells(len: usize, cells: impl IntoIterator<Item = u32>) -> CellSet {
        let mut s = CellSet::empty(len);
        for c in cells {
            s.insert(c);
        }
        s
    }

    /// Cells of the model this set is over.
    pub fn universe_len(&self) -> usize {
        self.len
    }

    pub fn contains(&self, c: u32) -> bool {
        let c = c as usize;
        c < self.len && self.words[c / 64] >> (c % 64) & 1 == 1
    }

    pub fn insert(&mut self, c: u32) {
        let c = c as usize;
        assert!(c < self.len, "cell {c} out of {}", self.len);
        self.words[c / 64] |= 1 << (c % 64);
    }

    pub fn remove(&mut self, c: u32) {
        let c = c as usize;
        if c < self.len {
            self.words[c / 64] &= !(1 << (c % 64));
        }
    }

    pub fn count(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|&w| w == 0)
    }

    pub fn union(&mut self, o: &CellSet) {
        for (a, b) in self.words.iter_mut().zip(&o.words) {
            *a |= b;
        }
    }

    pub fn intersect(&mut self, o: &CellSet) {
        for (a, b) in self.words.iter_mut().zip(&o.words) {
            *a &= b;
        }
    }

    pub fn subtract(&mut self, o: &CellSet) {
        for (a, b) in self.words.iter_mut().zip(&o.words) {
            *a &= !b;
        }
    }

    pub fn invert(&mut self) {
        for w in &mut self.words {
            *w = !*w;
        }
        self.trim();
    }

    /// The cells in the set, in index order.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.words.iter().enumerate().flat_map(|(k, &w)| {
            let mut w = w;
            std::iter::from_fn(move || {
                if w == 0 {
                    return None;
                }
                let b = w.trailing_zeros();
                w &= w - 1;
                Some(k as u32 * 64 + b)
            })
        })
    }

    /// The bits, 64 cells per word, cell `c` at bit `c % 64` of word
    /// `c / 64` (for GPU buffers).
    pub fn words(&self) -> &[u64] {
        &self.words
    }

    fn trim(&mut self) {
        let r = self.len % 64;
        if r != 0 {
            if let Some(w) = self.words.last_mut() {
                *w &= (1u64 << r) - 1;
            }
        }
    }
}

/// Who holds what in one model.
#[derive(Clone, Debug)]
pub struct ModelIndex {
    cells: usize,
    /// Cells by material number, filled cells left out (what fills them is
    /// what is seen there).
    by_material: FxHashMap<i64, Vec<u32>>,
    /// Index into [`Model::units`] by universe number.
    unit_of: FxHashMap<i64, u32>,
    /// Per unit: the cells it fills (its fill sites), each once.
    holders: Vec<Vec<u32>>,
    /// Per filled cell: the units it fills, each once.
    fills: FxHashMap<u32, Vec<u32>>,
    /// Per cell: its unit (as [`llevant::Cell::unit`], kept compact: a
    /// `Cell` is over 200 bytes, so a pass over all of them is a pass over
    /// ~100 MB on a large model).
    cell_unit: Vec<u32>,
    /// The filled cells.
    filled: CellSet,
    /// The cells' labels, once read from the input ([`ModelIndex::set_labels`]).
    labels: OnceLock<Arc<LabelCells>>,
}

/// Labels with their cells as indices into this model's cells: a rule then
/// sets bits, with no lookup by number per cell (261 k of them for a broad
/// rule on the reference model, 5–12 ms).
#[derive(Debug)]
struct LabelCells {
    labels: Arc<Labels>,
    /// Label `k`'s cells are `cells[start[k]..start[k + 1]]`; numbers this
    /// model lacks (a universe opened alone) left out.
    start: Vec<u32>,
    cells: Vec<u32>,
}

impl ModelIndex {
    pub fn new(m: &Model) -> ModelIndex {
        let cells = m.cells();
        let units = m.units();
        let mut by_material: FxHashMap<i64, Vec<u32>> = FxHashMap::default();
        let mut holders = vec![Vec::new(); units.len()];
        let mut fills: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
        let mut filled = CellSet::empty(cells.len());
        let cell_unit = cells.iter().map(|c| c.unit).collect();
        for (k, c) in cells.iter().enumerate() {
            let k = k as u32;
            if c.fill.is_none() {
                by_material.entry(c.material).or_default().push(k);
                continue;
            }
            let mut targets: Vec<u32> = m.fill_targets(k).map(|t| t.unit()).collect();
            targets.sort_unstable();
            targets.dedup();
            for &u in &targets {
                holders[u as usize].push(k);
            }
            fills.insert(k, targets);
            filled.insert(k);
        }
        let unit_of = units
            .iter()
            .enumerate()
            .map(|(k, u)| (u.universe, k as u32))
            .collect();
        ModelIndex {
            cells: cells.len(),
            by_material,
            unit_of,
            holders,
            fills,
            cell_unit,
            filled,
            labels: OnceLock::new(),
        }
    }

    /// Give rules the cells' labels (`label "BSV"`), read from the input.
    /// Once only, so that an index already shared between threads can get
    /// them when they are read; `false` if it had them already.
    pub fn set_labels(&self, m: &Model, labels: Arc<Labels>) -> bool {
        if self.labels.get().is_some() {
            return false;
        }
        let per_label: Vec<Vec<u32>> = (0..labels.len() as u32)
            .into_par_iter()
            .map(|k| {
                labels
                    .cells(k)
                    .iter()
                    .filter_map(|&id| m.cell_index(id))
                    .collect()
            })
            .collect();
        let mut start = Vec::with_capacity(per_label.len() + 1);
        start.push(0);
        let mut cells = Vec::with_capacity(per_label.iter().map(Vec::len).sum());
        for v in per_label {
            cells.extend(v);
            start.push(cells.len() as u32);
        }
        self.labels
            .set(Arc::new(LabelCells {
                labels,
                start,
                cells,
            }))
            .is_ok()
    }

    /// The labels, if they were given.
    pub fn labels(&self) -> Option<&Labels> {
        self.labels.get().map(|l| &*l.labels)
    }

    /// The cells (indices) with label `k` of [`ModelIndex::labels`].
    pub fn label_cells(&self, k: u32) -> &[u32] {
        let Some(l) = self.labels.get() else {
            return &[];
        };
        &l.cells[l.start[k as usize] as usize..l.start[k as usize + 1] as usize]
    }

    /// Cells of the model (the length of every [`CellSet`] of it).
    pub fn cell_count(&self) -> usize {
        self.cells
    }

    pub fn empty(&self) -> CellSet {
        CellSet::empty(self.cells)
    }

    /// Cells of material `id` that are not filled (0: void ones).
    pub fn material_cells(&self, id: i64) -> &[u32] {
        self.by_material.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Material numbers used by unfilled cells, void (0) included if any.
    pub fn materials(&self) -> Vec<i64> {
        let mut v: Vec<i64> = self.by_material.keys().copied().collect();
        v.sort_unstable();
        v
    }

    /// Index into [`Model::units`] of universe `u`.
    pub fn unit(&self, u: i64) -> Option<u32> {
        self.unit_of.get(&u).copied()
    }

    /// Cells that universe unit `unit` fills.
    pub fn holders(&self, unit: u32) -> &[u32] {
        &self.holders[unit as usize]
    }

    /// Cells whose programs use any of the MCNP surfaces `ids` (any facet,
    /// any moved copy, complements included), in index order. Not indexed:
    /// one parallel pass over the cells' programs (a few ms for 400 k cells
    /// and 24 M leaves), so it costs no memory until asked.
    pub fn surface_cells(&self, m: &Model, ids: &[i64]) -> Vec<u32> {
        let mut shapes: Vec<u32> = (0..m.shape_count() as u32)
            .filter(|&s| m.surface_id(s).is_some_and(|x| ids.contains(&x.id)))
            .collect();
        shapes.sort_unstable();
        self.shape_cells(m, &shapes)
    }

    /// Cells whose programs use any of the compiled surfaces `shapes`
    /// (sorted), in index order.
    pub fn shape_cells(&self, m: &Model, shapes: &[u32]) -> Vec<u32> {
        if shapes.is_empty() {
            return Vec::new();
        }
        let uses = |c: u32| {
            m.cell_surface_leaves(c)
                .any(|(s, _)| shapes.binary_search(&s).is_ok())
        };
        (0..m.cells().len() as u32)
            .into_par_iter()
            .filter(|&c| uses(c))
            .collect()
    }

    /// The unit of cell `c` (index into [`Model::units`]).
    pub fn cell_unit(&self, c: u32) -> u32 {
        self.cell_unit[c as usize]
    }

    /// Units cell `c` fills (empty if it is not filled).
    pub fn fills(&self, c: u32) -> &[u32] {
        self.fills.get(&c).map_or(&[], Vec::as_slice)
    }

    /// `set` and every cell inside its filled cells, at every level below.
    pub fn with_contents(&self, m: &Model, set: &CellSet) -> CellSet {
        let mut out = set.clone();
        let mut seen = vec![false; m.units().len()];
        let mut set_filled = set.clone();
        set_filled.intersect(&self.filled);
        let mut stack: Vec<u32> = set_filled
            .iter()
            .flat_map(|c| self.fills(c))
            .copied()
            .collect();
        while let Some(u) = stack.pop() {
            if std::mem::replace(&mut seen[u as usize], true) {
                continue;
            }
            for &c in &m.units()[u as usize].cells {
                out.insert(c);
                stack.extend_from_slice(self.fills(c));
            }
        }
        out
    }

    /// `set` and every filled cell above its cells, up to level 0: what
    /// must stay visible for rays to reach them.
    pub fn with_holders(&self, m: &Model, set: &CellSet) -> CellSet {
        let mut out = set.clone();
        let mut seen = vec![false; m.units().len()];
        // Each unit of the set's cells once.
        let mut queued = vec![false; m.units().len()];
        let mut stack: Vec<u32> = Vec::new();
        for c in set.iter() {
            let u = self.cell_unit(c);
            if !std::mem::replace(&mut queued[u as usize], true) {
                stack.push(u);
            }
        }
        while let Some(u) = stack.pop() {
            if std::mem::replace(&mut seen[u as usize], true) {
                continue;
            }
            for &h in self.holders(u) {
                out.insert(h);
                stack.push(self.cell_unit(h));
            }
        }
        out
    }

    /// Per unit ([`Model::units`]): the shallowest universe level at which
    /// rays reach it with `vis` (0 for the top), going down through filled
    /// cells `vis` shows; `None` if every way down is through a hidden
    /// cell. `vis.level` is left aside: a cell is drawn if `vis` shows it
    /// and its unit is reached at that level or above.
    pub fn reach_levels(&self, m: &Model, vis: &[bool]) -> Vec<Option<u32>> {
        let mut level = vec![None; m.units().len()];
        if level.is_empty() {
            return level;
        }
        // Breadth first: a unit is first met at its shallowest level.
        level[0] = Some(0);
        let mut queue = std::collections::VecDeque::from([0u32]);
        while let Some(u) = queue.pop_front() {
            let next = level[u as usize].map(|l| l + 1);
            for &c in &m.units()[u as usize].cells {
                if !vis[c as usize] {
                    continue;
                }
                for &f in self.fills(c) {
                    if level[f as usize].is_none() {
                        level[f as usize] = next;
                        queue.push_back(f);
                    }
                }
            }
        }
        level
    }
}

#[cfg(test)]
pub(crate) mod tests_model {
    /// Universe 1 (steel in water) in two envelopes, universe 2 in a 3×1×1
    /// lattice (universe 3) inside universe 1's water.
    pub(crate) const MODEL: &str = "index
1 3 -7.8 -1 fill=1 imp:n=1
4 0 -3 fill=1 imp:n=1
2 0 1 3 -9 imp:n=1
5 0 9 imp:n=0
10 3 -7.8 -2 u=1 imp:n=1
11 5 -1.0 2 -4 u=1 imp:n=1
12 0 4 u=1 fill=3 imp:n=1
20 0 -5 6 -7 8 u=3 lat=1 fill=0:2 0:0 0:0 2 2 2 imp:n=1
30 3 -7.8 -10 u=2 imp:n=1
31 0 10 u=2 imp:n=1

1 so 5
3 s 20 0 0 5
2 so 2
4 so 3
9 so 100
5 px 1
6 px -1
7 py 1
8 py -1
10 so 0.5

m3 26000 1
m5 1001 2 8016 1
";
}

#[cfg(test)]
mod tests {
    use super::tests_model::MODEL;
    use super::*;
    use llevant::Options;

    fn ids(m: &Model, s: &CellSet) -> Vec<i64> {
        s.iter().map(|c| m.cell_id(c)).collect()
    }

    #[test]
    fn cell_sets_do_set_algebra() {
        let mut a = CellSet::from_cells(130, [0, 63, 64, 129]);
        assert_eq!(a.count(), 4);
        assert_eq!(a.iter().collect::<Vec<_>>(), [0, 63, 64, 129]);
        let b = CellSet::from_cells(130, [63, 100]);
        a.subtract(&b);
        assert_eq!(a.iter().collect::<Vec<_>>(), [0, 64, 129]);
        a.invert();
        assert_eq!(a.count(), 127);
        assert!(!a.contains(129) && a.contains(128));
        assert_eq!(CellSet::full(130).count(), 130);
        assert!(CellSet::empty(5).is_empty());
    }

    #[test]
    fn index_knows_who_holds_what() {
        let m = Model::from_text(MODEL, &Options::default());
        let x = ModelIndex::new(&m);
        let cells = |v: &[u32]| v.iter().map(|&c| m.cell_id(c)).collect::<Vec<_>>();
        assert_eq!(cells(x.material_cells(3)), [10, 30]);
        assert_eq!(cells(x.material_cells(0)), [2, 5, 31]);
        let u1 = x.unit(1).unwrap();
        let u2 = x.unit(2).unwrap();
        assert_eq!(cells(x.holders(u1)), [1, 4]);
        assert_eq!(m.instances().count_unit(u1), 2);
        // Three lattice elements in each of the two instances of universe 1.
        assert_eq!(m.instances().count_unit(u2), 6);
        assert_eq!(m.instances().count_unit(x.unit(0).unwrap()), 1);

        let c4 = CellSet::from_cells(m.cells().len(), [m.cell_index(4).unwrap()]);
        assert_eq!(
            ids(&m, &x.with_contents(&m, &c4)),
            [4, 10, 11, 12, 20, 30, 31]
        );
        let c30 = CellSet::from_cells(m.cells().len(), [m.cell_index(30).unwrap()]);
        assert_eq!(ids(&m, &x.with_holders(&m, &c30)), [1, 4, 12, 20, 30]);
    }

    /// `with_contents` and `with_holders` as first written, from the
    /// model's cells: what the compact index must give.
    fn contents_and_holders_by_cells(m: &Model, x: &ModelIndex, set: &CellSet) -> [CellSet; 2] {
        let mut contents = set.clone();
        let mut stack: Vec<u32> = set.iter().flat_map(|c| x.fills(c)).copied().collect();
        let mut seen = vec![false; m.units().len()];
        while let Some(u) = stack.pop() {
            if !std::mem::replace(&mut seen[u as usize], true) {
                for &c in &m.units()[u as usize].cells {
                    contents.insert(c);
                    stack.extend_from_slice(x.fills(c));
                }
            }
        }
        let mut holders = set.clone();
        let mut stack: Vec<u32> = set.iter().map(|c| m.cell(c).unit).collect();
        let mut seen = vec![false; m.units().len()];
        while let Some(u) = stack.pop() {
            if !std::mem::replace(&mut seen[u as usize], true) {
                for &h in x.holders(u) {
                    holders.insert(h);
                    stack.push(m.cell(h).unit);
                }
            }
        }
        [contents, holders]
    }

    #[test]
    fn contents_and_holders_of_every_pair_of_cells() {
        let m = Model::from_text(MODEL, &Options::default());
        let x = ModelIndex::new(&m);
        let n = m.cells().len() as u32;
        for a in 0..n {
            for b in a..n {
                let set = CellSet::from_cells(n as usize, [a, b]);
                let [contents, holders] = contents_and_holders_by_cells(&m, &x, &set);
                assert_eq!(x.with_contents(&m, &set), contents, "cells {a} {b}");
                assert_eq!(x.with_holders(&m, &set), holders, "cells {a} {b}");
            }
        }
        for c in 0..n {
            assert_eq!(x.cell_unit(c), m.cell(c).unit);
        }
    }

    #[test]
    fn reach_levels_stop_at_hidden_cells() {
        let m = Model::from_text(MODEL, &Options::default());
        let x = ModelIndex::new(&m);
        let at = |vis: &[bool]| {
            let r = x.reach_levels(&m, vis);
            [0, 1, 3, 2].map(|u| r[x.unit(u).unwrap() as usize])
        };
        let mut vis = vec![true; m.cells().len()];
        assert_eq!(at(&vis), [Some(0), Some(1), Some(2), Some(3)]);
        // Universe 1 is still reached through envelope 4.
        vis[m.cell_index(1).unwrap() as usize] = false;
        assert_eq!(at(&vis), [Some(0), Some(1), Some(2), Some(3)]);
        vis[m.cell_index(4).unwrap() as usize] = false;
        assert_eq!(at(&vis), [Some(0), None, None, None]);
        let mut vis = vec![true; m.cells().len()];
        vis[m.cell_index(20).unwrap() as usize] = false;
        assert_eq!(at(&vis), [Some(0), Some(1), Some(2), None]);
    }
}
