//! What a 3D view hides, as the `llevant` command, the viewer and the
//! Python bindings ask for it, turned into the engine's [`Visibility`] in
//! one place.
//!
//! The rules are of two kinds:
//!
//! - **Containers**: cells and universes by number. A hidden filled cell
//!   hides everything that fills it (the engine's rule).
//! - **Contents**: void cells and cells by material. These are about what a
//!   ray would show, so they never hide a filled cell: what is seen there is
//!   what fills it, whatever material its card gives, and hiding it would
//!   hide all it holds.
//!
//! On top of them, [`Query`] rules: hidden ones (as containers: a filled
//! cell they select hides what it holds; `m 5` never selects filled
//! cells), and an isolated one, which hides everything but its cells, what
//! they hold and the filled cells above them (without which rays would not
//! reach them). An isolated part shows only its own gaps and problems
//! ([`ProblemScope`]): an overlap elsewhere in the model, between cells
//! hidden or between the envelopes holding it, would draw specks around a
//! part that has nothing to do with them.
//!
//! Then two ways of seeing the levels of a model: the level shown (as
//! MCNP's plotter `LEVEL n`: only cells at that universe level, a filled
//! one as solid), and ghost cells, seen as shells over what lies behind
//! them (a filled one with what it holds shown as the rules say). A ghost
//! cell is shown whatever the rules above say of it. Gaps (and overlaps,
//! which only the CPU engine looks for) can be seen through as veils, so
//! that a hole in front hides nothing behind it.

use llevant::{Error, Model, Result, Visibility};

use crate::index::ModelIndex;
use crate::query::Query;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Which cells a 3D view hides. The default hides void cells only.
///
/// With the `serde` feature its fields are named as in the viewer's view
/// file: `hide_void`, `hidden_cells`, `hidden_universes`,
/// `hidden_materials`, `hide`, `isolate`, `problems`, `level`, `ghost`,
/// `see_through_gaps`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(default))]
pub struct Hiding {
    /// Void cells (material 0) that are not filled.
    #[cfg_attr(feature = "serde", serde(rename = "hide_void"))]
    pub void: bool,
    /// MCNP cell numbers, with what they hold.
    #[cfg_attr(feature = "serde", serde(rename = "hidden_cells"))]
    pub cells: Vec<i64>,
    /// MCNP universe numbers: every cell of each, with what they hold.
    #[cfg_attr(feature = "serde", serde(rename = "hidden_universes"))]
    pub universes: Vec<i64>,
    /// MCNP material numbers: the cells made of them that are not filled
    /// (0 is void, as [`Hiding::void`]).
    #[cfg_attr(feature = "serde", serde(rename = "hidden_materials"))]
    pub materials: Vec<i64>,
    /// Cells hidden by rule.
    #[cfg_attr(
        feature = "serde",
        serde(rename = "hide", skip_serializing_if = "Vec::is_empty")
    )]
    pub rules: Vec<Query>,
    /// Only these cells shown (with what they hold and what holds them).
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub isolate: Option<Query>,
    /// With [`Hiding::isolate`], whose gaps and problems show.
    #[cfg_attr(
        feature = "serde",
        serde(skip_serializing_if = "ProblemScope::is_default")
    )]
    pub problems: ProblemScope,
    /// The universe level shown (0 is the top); `None` shows the deepest
    /// cell of every path.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub level: Option<usize>,
    /// Ghost cells, by rule.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
    pub ghost: Vec<Query>,
    /// Gaps and overlaps seen through, as veils ([`Visibility::veil`]),
    /// rather than solid.
    #[cfg_attr(feature = "serde", serde(rename = "see_through_gaps"))]
    pub see_through: bool,
}

impl Default for Hiding {
    fn default() -> Hiding {
        Hiding {
            void: true,
            cells: Vec::new(),
            universes: Vec::new(),
            materials: Vec::new(),
            rules: Vec::new(),
            isolate: None,
            problems: ProblemScope::default(),
            level: None,
            ghost: Vec::new(),
            see_through: false,
        }
    }
}

/// Whose gaps and problems an isolated view shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "lowercase")
)]
pub enum ProblemScope {
    /// Those of the cells isolated and what they hold
    /// ([`Visibility::problem_cells`](llevant::Visibility::problem_cells)):
    /// overlaps one of them is in, gaps in the universes they are filled
    /// with.
    #[default]
    Selection,
    /// Every one met, wherever it is.
    All,
}

impl ProblemScope {
    pub fn is_default(&self) -> bool {
        *self == ProblemScope::default()
    }
}

impl Hiding {
    /// Nothing hidden, void cells included.
    pub fn none() -> Hiding {
        Hiding {
            void: false,
            ..Hiding::default()
        }
    }

    /// The engine's visibility of `m` with these rules; gaps and problems
    /// shown (gaps and overlaps as veils with [`Hiding::see_through`]). [`Error::NoCell`], [`Error::NoUniverse`] or
    /// [`Error::Invalid`] (materials) for the first number that `m` does
    /// not have.
    pub fn visibility(&self, m: &Model) -> Result<Visibility> {
        if self.rules.is_empty() && self.isolate.is_none() && self.ghost.is_empty() {
            self.base(m, None)
        } else {
            self.visibility_with(m, &ModelIndex::new(m))
        }
    }

    /// [`Hiding::visibility`] with an index of `m` at hand.
    pub fn visibility_with(&self, m: &Model, x: &ModelIndex) -> Result<Visibility> {
        let mut vis = self.base(m, Some(x))?;
        for q in &self.rules {
            for c in q.resolve(m, x)?.iter() {
                vis.cells[c as usize] = false;
            }
        }
        if let Some(q) = &self.isolate {
            let set = q.resolve(m, x)?;
            let mut keep = x.with_contents(m, &set);
            if self.problems == ProblemScope::Selection {
                vis.problem_cells = (0..m.cells().len() as u32)
                    .map(|c| keep.contains(c))
                    .collect();
            }
            keep.union(&x.with_holders(m, &set));
            for (k, v) in vis.cells.iter_mut().enumerate() {
                *v &= keep.contains(k as u32);
            }
        }
        for q in &self.ghost {
            for c in q.resolve(m, x)?.iter() {
                if vis.ghosts.is_empty() {
                    vis.ghosts = vec![false; m.cells().len()];
                }
                vis.ghosts[c as usize] = true;
                vis.cells[c as usize] = true;
            }
        }
        Ok(vis)
    }

    /// Whether a rule (hidden, isolated, ghost) has a `label` term: it needs
    /// the cells' labels on the index.
    pub fn uses_labels(&self) -> bool {
        self.rules
            .iter()
            .chain(&self.isolate)
            .chain(&self.ghost)
            .any(Query::uses_labels)
    }

    /// Whether anything but void cells is hidden (levels and ghosts
    /// aside).
    pub fn hides_more_than_void(&self) -> bool {
        !(self.cells.is_empty()
            && self.universes.is_empty()
            && self.materials.is_empty()
            && self.rules.is_empty()
            && self.isolate.is_none())
    }

    /// The same void rule, level and ghosts, nothing else hidden.
    pub fn shown_all(&self) -> Hiding {
        Hiding {
            void: self.void,
            level: self.level,
            ghost: self.ghost.clone(),
            see_through: self.see_through,
            problems: self.problems,
            ..Hiding::none()
        }
    }

    /// The cells, universes and contents hidden. With an index, contents
    /// come from its cells by material; without, from a pass over the
    /// cells (a `Cell` is over 200 bytes: ~3 ms on 400 k cells).
    fn base(&self, m: &Model, x: Option<&ModelIndex>) -> Result<Visibility> {
        for &id in &self.materials {
            let used = match x {
                Some(x) => !x.material_cells(id).is_empty(),
                None => m.cells().iter().any(|c| c.material == id),
            };
            if id != 0 && m.material(id).is_none() && !used {
                return Err(Error::Invalid(format!("no material {id}")));
            }
        }
        let mut vis = Visibility::all(m);
        vis.level = self.level;
        vis.veil = self.see_through;
        match x {
            Some(x) => {
                let void = self.void.then_some(&0);
                for id in void.into_iter().chain(&self.materials) {
                    for &c in x.material_cells(*id) {
                        vis.cells[c as usize] = false;
                    }
                }
            }
            None => {
                for (k, c) in m.cells().iter().enumerate() {
                    let content = c.fill.is_none()
                        && ((self.void && c.material == 0) || self.materials.contains(&c.material));
                    if content {
                        vis.cells[k] = false;
                    }
                }
            }
        }
        vis.hide(m, &self.cells)?;
        for &u in &self.universes {
            vis.hide_universe(m, u)?;
        }
        Ok(vis)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llevant::Options;

    /// Two envelopes filled with universe 1 (steel in water): cell 1 with a
    /// material on its card, cell 4 void.
    const MODEL: &str = "hiding
1 3 -7.8 -1 fill=1 imp:n=1
4 0 -3 fill=1 imp:n=1
2 0 1 3 -9 imp:n=1
5 0 9 imp:n=0
10 3 -7.8 -2 u=1 imp:n=1
11 5 -1.0 2 u=1 imp:n=1

1 so 5
3 s 20 0 0 5
2 so 2
9 so 100

m3 26000 1
m5 1001 2 8016 1
";

    fn shown(m: &Model, h: &Hiding) -> Vec<i64> {
        let vis = h.visibility(m).unwrap();
        m.cells()
            .iter()
            .zip(&vis.cells)
            .filter(|(_, &v)| v)
            .map(|(c, _)| c.id)
            .collect()
    }

    #[test]
    fn base_is_the_same_with_an_index() {
        let m = Model::from_text(MODEL, &Options::default());
        let x = ModelIndex::new(&m);
        for void in [false, true] {
            for materials in [vec![], vec![0], vec![3], vec![5, 3], vec![3, 3, 0]] {
                for cells in [vec![], vec![10], vec![1, 2]] {
                    let h = Hiding {
                        void,
                        materials: materials.clone(),
                        cells: cells.clone(),
                        universes: vec![1],
                        ..Hiding::none()
                    };
                    assert_eq!(
                        h.base(&m, Some(&x)).unwrap().cells,
                        h.base(&m, None).unwrap().cells,
                        "void {void} materials {materials:?} cells {cells:?}"
                    );
                }
            }
        }
        let unknown = Hiding {
            materials: vec![77],
            ..Hiding::none()
        };
        assert!(unknown.base(&m, Some(&x)).is_err());
        assert!(unknown.base(&m, None).is_err());
    }

    #[test]
    fn content_rules_skip_filled_cells() {
        let m = Model::from_text(MODEL, &Options::default());
        assert_eq!(shown(&m, &Hiding::default()), [1, 4, 10, 11]);
        assert_eq!(
            Hiding::default().visibility(&m).unwrap().cells,
            Visibility::solid(&m).cells
        );
        assert_eq!(shown(&m, &Hiding::none()), [1, 4, 2, 5, 10, 11]);
        let steel = Hiding {
            materials: vec![3],
            ..Hiding::default()
        };
        assert_eq!(shown(&m, &steel), [1, 4, 11]);
        let void = Hiding {
            materials: vec![0],
            ..Hiding::none()
        };
        assert_eq!(shown(&m, &void), [1, 4, 10, 11]);
    }

    #[test]
    fn container_rules_hide_what_cells_hold() {
        let m = Model::from_text(MODEL, &Options::default());
        let h = Hiding {
            cells: vec![1],
            universes: vec![1],
            ..Hiding::default()
        };
        assert_eq!(shown(&m, &h), [4]);
    }

    #[test]
    fn rules_hide_and_isolate() {
        let m = Model::from_text(MODEL, &Options::default());
        let hide = Hiding {
            rules: vec!["u 1 & m 3".parse().unwrap()],
            ..Hiding::default()
        };
        assert_eq!(shown(&m, &hide), [1, 4, 11]);
        // Isolating a cell of universe 1 keeps the envelopes that hold it.
        let iso = Hiding {
            isolate: Some(Query::cell(10)),
            ..Hiding::default()
        };
        assert_eq!(shown(&m, &iso), [1, 4, 10]);
        // Isolating an envelope keeps what it holds.
        let iso = Hiding {
            isolate: Some(Query::cell(4)),
            ..Hiding::default()
        };
        assert_eq!(shown(&m, &iso), [4, 10, 11]);
        // Only the problems of what is isolated, not of its envelope.
        let owns = |h: &Hiding| -> Vec<i64> {
            let vis = h.visibility(&m).unwrap();
            m.cells()
                .iter()
                .zip(&vis.problem_cells)
                .filter(|(_, &p)| p)
                .map(|(c, _)| c.id)
                .collect()
        };
        assert_eq!(owns(&iso), [4, 10, 11]);
        let one = Hiding {
            isolate: Some(Query::cell(10)),
            ..Hiding::default()
        };
        assert_eq!(owns(&one), [10]);
        let every = Hiding {
            problems: ProblemScope::All,
            ..one.clone()
        };
        assert!(every.visibility(&m).unwrap().problem_cells.is_empty());
        assert!(Hiding::default()
            .visibility(&m)
            .unwrap()
            .problem_cells
            .is_empty());
        assert_eq!(iso.shown_all(), Hiding::default());
        assert!(iso.hides_more_than_void() && !Hiding::default().hides_more_than_void());
    }

    #[test]
    fn isolating_by_label() {
        let text = MODEL.replace(
            "10 3 -7.8 -2 u=1 imp:n=1",
            "10 3 -7.8 -2 u=1 imp:n=1 $ steel",
        );
        let m = Model::from_text(&text, &Options::default());
        let x = ModelIndex::new(&m);
        let iso = Hiding {
            isolate: Some("label STEEL".parse().unwrap()),
            ..Hiding::default()
        };
        assert!(iso.visibility_with(&m, &x).is_err());
        x.set_labels(
            &m,
            std::sync::Arc::new(crate::labels::Labels::of_text(&text)),
        );
        let vis = iso.visibility_with(&m, &x).unwrap();
        let shown: Vec<i64> = m
            .cells()
            .iter()
            .zip(&vis.cells)
            .filter(|(_, &v)| v)
            .map(|(c, _)| c.id)
            .collect();
        assert_eq!(shown, [1, 4, 10]);
    }

    #[test]
    fn levels_and_ghosts() {
        let m = Model::from_text(MODEL, &Options::default());
        let h = Hiding {
            level: Some(0),
            ghost: vec![Query::cell(4), Query::cell(2)],
            ..Hiding::default()
        };
        let vis = h.visibility(&m).unwrap();
        assert_eq!(vis.level, Some(0));
        // The void cell 2 is shown as a ghost, whatever the void rule says.
        assert_eq!(shown(&m, &h), [1, 4, 2, 10, 11]);
        let ghosts: Vec<i64> = m
            .cells()
            .iter()
            .zip(&vis.ghosts)
            .filter(|(_, &g)| g)
            .map(|(c, _)| c.id)
            .collect();
        assert_eq!(ghosts, [4, 2]);
        assert_eq!(h.shown_all().ghost, h.ghost);
        assert!(!h.hides_more_than_void());
    }

    #[test]
    fn unknown_numbers_are_errors() {
        let m = Model::from_text(MODEL, &Options::default());
        let with = |f: fn(&mut Hiding)| {
            let mut h = Hiding::default();
            f(&mut h);
            h.visibility(&m).unwrap_err().to_string()
        };
        assert_eq!(with(|h| h.cells = vec![7]), "no cell 7");
        assert_eq!(with(|h| h.universes = vec![7]), "no universe 7");
        assert_eq!(with(|h| h.materials = vec![7]), "no material 7");
    }
}
