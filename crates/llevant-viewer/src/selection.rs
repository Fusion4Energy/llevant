//! What is selected: a rule ([`Query`]) and the cells it gives, with a
//! history to step back and forward through like a browser's. Selecting
//! never traces again: the views tint and outline the cells over what they
//! have traced.

use std::sync::Arc;

use llevant::{Aabb, Model};
use llevant_scene::index::{CellSet, ModelIndex};
use llevant_scene::query::Query;

/// How a new rule combines with the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectMode {
    Replace,
    Add,
    Remove,
    /// Add if not all selected yet, otherwise remove.
    Toggle,
}

impl SelectMode {
    /// From the modifiers of a click: Ctrl adds (toggles), Shift removes.
    pub fn from_modifiers(m: egui::Modifiers) -> SelectMode {
        if m.command || m.ctrl {
            SelectMode::Toggle
        } else if m.shift {
            SelectMode::Remove
        } else {
            SelectMode::Replace
        }
    }
}

/// Steps kept in the history.
const HISTORY: usize = 100;

pub struct Selection {
    /// The rule, `None` for nothing.
    pub query: Option<Query>,
    pub cells: Arc<CellSet>,
    /// The cells by universe and by material, made with `cells` (the
    /// inspector shows them every frame).
    pub groups: Arc<Groups>,
    back: Vec<Option<Query>>,
    forward: Vec<Option<Query>>,
}

impl Selection {
    pub fn new(cells: usize) -> Selection {
        Selection {
            query: None,
            cells: Arc::new(CellSet::empty(cells)),
            groups: Arc::default(),
            back: Vec::new(),
            forward: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.query.is_none()
    }

    /// Combine `q` with the selection; `Err` (and nothing changed) if the
    /// rule names numbers the model does not have.
    pub fn apply(
        &mut self,
        q: Query,
        mode: SelectMode,
        m: &Model,
        x: &ModelIndex,
    ) -> Result<(), String> {
        let next = match (self.query.clone(), mode) {
            (None, SelectMode::Remove) => return Ok(()),
            (None, _) | (_, SelectMode::Replace) => Some(q),
            (Some(cur), SelectMode::Add) => Some(cur.plus(q)),
            (Some(cur), SelectMode::Remove) => Some(cur.minus(q)),
            (Some(cur), SelectMode::Toggle) => {
                let new = q.resolve(m, x).map_err(|e| e.to_string())?;
                let mut missing = new.clone();
                missing.subtract(&self.cells);
                if missing.is_empty() {
                    Some(cur.minus(q))
                } else {
                    Some(cur.plus(q))
                }
            }
        };
        self.set(next, m, x)
    }

    /// Select `q` (or nothing), remembering the selection before.
    pub fn set(&mut self, q: Option<Query>, m: &Model, x: &ModelIndex) -> Result<(), String> {
        let q = q.filter(|q| *q != Query::Cells(Vec::new()));
        if q == self.query {
            return Ok(());
        }
        let cells = self.resolve(&q, m, x)?;
        self.back.push(self.query.take());
        if self.back.len() > HISTORY {
            self.back.remove(0);
        }
        self.forward.clear();
        self.query = q;
        self.take(cells, m, x);
        Ok(())
    }

    /// The same rule on another model (a new version of this one): cells
    /// it names that are gone are left out, and a rule that does not
    /// resolve any more selects nothing.
    pub fn rebind(&mut self, m: &Model, x: &ModelIndex) {
        if let Some(Query::Cells(ids)) = &mut self.query {
            ids.retain(|&id| m.cell_index(id).is_some());
        }
        self.resolve_again(m, x);
    }

    /// Step back in the history; whether there was a step.
    pub fn back(&mut self, m: &Model, x: &ModelIndex) -> bool {
        let Some(q) = self.back.pop() else {
            return false;
        };
        let cur = std::mem::replace(&mut self.query, q);
        self.forward.push(cur);
        self.resolve_again(m, x);
        true
    }

    /// Step forward again; whether there was a step.
    pub fn forward(&mut self, m: &Model, x: &ModelIndex) -> bool {
        let Some(q) = self.forward.pop() else {
            return false;
        };
        let cur = std::mem::replace(&mut self.query, q);
        self.back.push(cur);
        self.resolve_again(m, x);
        true
    }

    /// The rules selected last, newest first, each once (the one now too).
    pub fn recent(&self, n: usize) -> Vec<Query> {
        let mut out: Vec<Query> = Vec::new();
        let now = self.query.iter();
        for q in now.chain(self.back.iter().rev().flatten()) {
            if !out.contains(q) {
                out.push(q.clone());
            }
            if out.len() == n {
                break;
            }
        }
        out
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// The cells of the rule selected now (none if it does not resolve).
    fn resolve_again(&mut self, m: &Model, x: &ModelIndex) {
        let cells = self
            .resolve(&self.query, m, x)
            .unwrap_or_else(|_| Arc::new(x.empty()));
        self.take(cells, m, x);
    }

    /// Select `cells`, with their groups.
    fn take(&mut self, cells: Arc<CellSet>, m: &Model, x: &ModelIndex) {
        self.groups = Arc::new(Groups::of(m, x, &cells));
        self.cells = cells;
    }

    fn resolve(
        &self,
        q: &Option<Query>,
        m: &Model,
        x: &ModelIndex,
    ) -> Result<Arc<CellSet>, String> {
        Ok(Arc::new(match q {
            Some(q) => q.resolve(m, x).map_err(|e| e.to_string())?,
            None => x.empty(),
        }))
    }

    /// One line for the status bar: how many cells, by what rule.
    pub fn summary(&self) -> Option<String> {
        let q = self.query.as_ref()?;
        let n = self.cells.count();
        let cells = if n == 1 { "cell" } else { "cells" };
        let (q, _) = crate::context::elide(&q.to_string(), 60);
        Some(format!("{n} {cells} ({q})"))
    }
}

/// Selected cells by universe and by material (unfilled cells), each
/// `(number, cells)`, largest first.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Groups {
    pub by_universe: Vec<(i64, usize)>,
    pub by_material: Vec<(i64, usize)>,
}

impl Groups {
    /// From the index's compact tables: a pass over the cells' 224-byte
    /// records costs ~6 ms on 400 k cells.
    pub fn of(m: &Model, x: &ModelIndex, set: &CellSet) -> Groups {
        if set.is_empty() {
            return Groups::default();
        }
        let mut per_unit = vec![0usize; m.units().len()];
        for c in set.iter() {
            per_unit[x.cell_unit(c) as usize] += 1;
        }
        let by_universe = per_unit
            .iter()
            .enumerate()
            .filter(|(_, &n)| n > 0)
            .map(|(u, &n)| (m.units()[u].universe, n))
            .collect();
        let by_material = x
            .materials()
            .into_iter()
            .map(|id| {
                let n = x
                    .material_cells(id)
                    .iter()
                    .filter(|&&c| set.contains(c))
                    .count();
                (id, n)
            })
            .filter(|&(_, n)| n > 0)
            .collect();
        Groups {
            by_universe: largest(by_universe),
            by_material: largest(by_material),
        }
    }
}

/// Largest count first, then by number.
fn largest(mut v: Vec<(i64, usize)>) -> Vec<(i64, usize)> {
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// Box in the global frame around where the cells of `set` can be: their
/// own boxes at level 0, the boxes of the level-0 cells that hold them
/// deeper down (every instance of a universe is selected with it), within
/// `region`. `None` for an empty set.
pub fn selection_box(m: &Model, x: &ModelIndex, set: &CellSet, region: &Aabb) -> Option<Aabb> {
    let top = x.unit(0)?;
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let mut grow = |b: &Aabb| {
        for k in 0..3 {
            lo[k] = lo[k].min(b.lo[k].max(region.lo[k]));
            hi[k] = hi[k].max(b.hi[k].min(region.hi[k]));
        }
    };
    let mut deeper = CellSet::empty(x.cell_count());
    for c in set.iter() {
        if m.cell(c).unit == top {
            grow(&m.cell(c).bbox);
        } else {
            deeper.insert(c);
        }
    }
    if !deeper.is_empty() {
        for c in x.with_holders(m, &deeper).iter() {
            if m.cell(c).unit == top {
                grow(&m.cell(c).bbox);
            }
        }
    }
    (lo[0] <= hi[0] && lo[1] <= hi[1] && lo[2] <= hi[2]).then_some(Aabb { lo, hi })
}

#[cfg(test)]
mod tests {
    use super::*;
    use llevant::Options;

    const MODEL: &str = "selection
1 3 -7.8 -1 fill=1 imp:n=1
2 0 1 -9 imp:n=1
5 0 9 imp:n=0
10 3 -7.8 -2 u=1 imp:n=1
11 5 -1.0 2 u=1 imp:n=1

1 so 5
2 so 2
9 so 100

m3 26000 1
m5 1001 2 8016 1
";

    fn ids(m: &Model, s: &Selection) -> Vec<i64> {
        s.cells.iter().map(|c| m.cell_id(c)).collect()
    }

    #[test]
    fn modes_combine_and_history_steps() {
        let m = Model::from_text(MODEL, &Options::default());
        let x = ModelIndex::new(&m);
        let mut s = Selection::new(m.cells().len());
        s.apply(Query::cell(10), SelectMode::Replace, &m, &x)
            .unwrap();
        s.apply(Query::cell(11), SelectMode::Toggle, &m, &x)
            .unwrap();
        assert_eq!(ids(&m, &s), [10, 11]);
        assert_eq!(s.query, Some("c 10 11".parse().unwrap()));
        s.apply(Query::cell(10), SelectMode::Toggle, &m, &x)
            .unwrap();
        assert_eq!(ids(&m, &s), [11]);
        s.apply(Query::material(3), SelectMode::Add, &m, &x)
            .unwrap();
        assert_eq!(ids(&m, &s), [10, 11]);
        assert!(s
            .apply(Query::cell(99), SelectMode::Replace, &m, &x)
            .is_err());
        assert_eq!(ids(&m, &s), [10, 11]);

        assert!(s.back(&m, &x));
        assert_eq!(ids(&m, &s), [11]);
        assert!(s.forward(&m, &x));
        assert_eq!(ids(&m, &s), [10, 11]);
        assert!(!s.forward(&m, &x));
        s.set(None, &m, &x).unwrap();
        assert!(s.is_empty() && s.cells.is_empty());
        assert_eq!(s.summary(), None);
    }

    /// Groups as the inspector first counted them: from each cell's record.
    #[test]
    fn groups_count_as_the_cells_say() {
        let m = Model::from_text(MODEL, &Options::default());
        let x = ModelIndex::new(&m);
        let n = m.cells().len() as u32;
        for mask in 0u32..1 << n.min(12) {
            let set = CellSet::from_cells(n as usize, (0..n).filter(|c| mask >> c & 1 == 1));
            let mut by_u = std::collections::BTreeMap::<i64, usize>::new();
            let mut by_m = std::collections::BTreeMap::<i64, usize>::new();
            for c in set.iter() {
                let cell = m.cell(c);
                *by_u
                    .entry(m.units()[cell.unit as usize].universe)
                    .or_default() += 1;
                if cell.fill.is_none() {
                    *by_m.entry(cell.material).or_default() += 1;
                }
            }
            let expect = Groups {
                by_universe: largest(by_u.into_iter().collect()),
                by_material: largest(by_m.into_iter().collect()),
            };
            assert_eq!(Groups::of(&m, &x, &set), expect, "cells {mask:b}");
        }
        let mut s = Selection::new(n as usize);
        s.set(Some(Query::material(3)), &m, &x).unwrap();
        assert_eq!(*s.groups, Groups::of(&m, &x, &s.cells));
        s.set(None, &m, &x).unwrap();
        assert_eq!(*s.groups, Groups::default());
    }

    #[test]
    fn box_of_deeper_cells_is_their_holders() {
        let m = Model::from_text(MODEL, &Options::default());
        let x = ModelIndex::new(&m);
        let region = Aabb {
            lo: [-50.0; 3],
            hi: [50.0; 3],
        };
        let set = CellSet::from_cells(m.cells().len(), [m.cell_index(10).unwrap()]);
        let b = selection_box(&m, &x, &set, &region).unwrap();
        // The box of cell 1, the sphere of radius 5 (padded outwards).
        assert!(b.lo.iter().all(|&v| (-5.1..=-5.0).contains(&v)), "{b:?}");
        assert!(b.hi.iter().all(|&v| (5.0..=5.1).contains(&v)), "{b:?}");
        assert!(selection_box(&m, &x, &x.empty(), &region).is_none());
    }
}
