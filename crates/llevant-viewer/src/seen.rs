//! What the 3D view actually shows of each cell and material, and why
//! what it does not show is hidden: what the eyes of the panels say and
//! what clicking them does.
//!
//! A cell's own flag in the scene's [`Visibility`] is not enough: a cell
//! is drawn only if rays reach it, through filled cells that are shown, at
//! the level shown or above ([`ModelIndex::reach_levels`]); and an
//! isolated rule hides everything else whatever the hiding lists say. An
//! eye that ignored either would stay open over hidden cells (or a click
//! on it would change nothing), so the panels read this instead. It scales
//! with the model, so the deriver makes it with each visibility
//! (`derive.rs`) and the loading thread the first one.

use llevant::{Model, Visibility};
use llevant_scene::index::{CellSet, ModelIndex};
use llevant_scene::query::Query;
use std::collections::HashMap;

use crate::command::Edit;
use crate::scene::Scene;

/// What the scene shows of a model.
#[derive(Clone, Debug)]
pub struct Seen {
    /// Cells drawn.
    seen: CellSet,
    /// Cells the scene's flags show (reached or not).
    vis: CellSet,
    /// While isolating: the cells the hiding rules show without the isolated
    /// rule, and the cells the isolated rule keeps.
    isolating: Option<(CellSet, CellSet)>,
    /// Per unit, the shallowest level rays reach it at.
    reach: Vec<Option<u32>>,
    level: Option<usize>,
    /// Per material (0 void): cells seen, cells hidden only by isolating,
    /// cells.
    materials: HashMap<i64, Counts>,
}

/// Of a material's cells (filled ones left out).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub seen: u32,
    /// Hidden only because something else is isolated.
    pub isolated_away: u32,
    /// Left out by the isolated rule (hidden by it or not).
    pub left_out: u32,
    pub cells: u32,
}

/// Why a cell is not seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    Seen,
    /// Hidden by the hiding lists or rules (its own number or others).
    Hidden,
    /// Hidden only because something else is isolated.
    IsolatedAway,
    /// Inside a filled cell (this index) that is hidden.
    Inside(u32),
    /// Deeper than the universe level shown.
    BelowLevel,
}

impl Seen {
    /// What `scene` shows of `m`, `vis` being its visibility.
    pub fn new(m: &Model, x: &ModelIndex, scene: &Scene, vis: &Visibility) -> Seen {
        let n = m.cells().len();
        let flags = |v: &[bool]| CellSet::from_cells(n, (0..n as u32).filter(|&c| v[c as usize]));
        let reach = x.reach_levels(m, &vis.cells);
        let level = vis.level;
        let drawn = |c: u32| {
            vis.cells[c as usize]
                && reach[m.cell(c).unit as usize]
                    .is_some_and(|l| level.is_none_or(|n| l as usize <= n))
        };
        let seen = CellSet::from_cells(n, (0..n as u32).filter(|&c| drawn(c)));
        let isolating = scene.hiding.isolate.as_ref().and_then(|q| {
            let free = llevant_scene::hiding::Hiding {
                isolate: None,
                ..scene.hiding.clone()
            }
            .visibility_with(m, x)
            .ok()?;
            let set = q.resolve(m, x).ok()?;
            let mut keep = x.with_contents(m, &set);
            keep.union(&x.with_holders(m, &set));
            Some((flags(&free.cells), keep))
        });
        let mut materials: HashMap<i64, Counts> = HashMap::new();
        for id in x.materials() {
            let mut k = Counts::default();
            for &c in x.material_cells(id) {
                k.cells += 1;
                if isolating
                    .as_ref()
                    .is_some_and(|(_, keep)| !keep.contains(c))
                {
                    k.left_out += 1;
                }
                if seen.contains(c) {
                    k.seen += 1;
                } else if let Some((free, keep)) = &isolating {
                    if free.contains(c) && !keep.contains(c) && !vis.cells[c as usize] {
                        k.isolated_away += 1;
                    }
                }
            }
            materials.insert(id, k);
        }
        Seen {
            seen,
            vis: flags(&vis.cells),
            isolating,
            reach,
            level,
            materials,
        }
    }

    pub fn is_seen(&self, c: u32) -> bool {
        self.seen.contains(c)
    }

    /// The cells the 3D view draws (before its cuts), what the rule `shown`
    /// starts from: those seen that rays stop in. At a level, only the
    /// cells there, filled ones solid (as MCNP's `LEVEL n`); at the deepest
    /// cells, those not filled (rays go on into what fills the others).
    pub fn drawn(&self, m: &Model) -> CellSet {
        let n = m.cells().len();
        let reach = |c: u32| self.reach[m.cell(c).unit as usize];
        CellSet::from_cells(
            n,
            self.seen.iter().filter(|&c| match self.level {
                Some(l) => reach(c) == Some(l as u32),
                None => m.cell(c).fill.is_none(),
            }),
        )
    }

    /// Whether the isolated rule leaves cell `c` out.
    pub fn isolated_out(&self, c: u32) -> bool {
        self.isolating
            .as_ref()
            .is_some_and(|(_, keep)| !keep.contains(c))
    }

    /// Why cell `c` of `m` is not seen (or that it is).
    pub fn why(&self, m: &Model, x: &ModelIndex, c: u32) -> Why {
        if self.seen.contains(c) {
            return Why::Seen;
        }
        if !self.vis.contains(c) {
            return match &self.isolating {
                Some((free, keep)) if free.contains(c) && !keep.contains(c) => Why::IsolatedAway,
                _ => Why::Hidden,
            };
        }
        let unit = m.cell(c).unit;
        if self.reach.get(unit as usize).copied().flatten().is_some() {
            return Why::BelowLevel;
        }
        // The nearest hidden cell above: one holder hidden, else one above
        // a holder that is shown but not reached itself.
        let mut seen_units = vec![false; m.units().len()];
        let mut u = unit;
        loop {
            if std::mem::replace(&mut seen_units[u as usize], true) {
                return Why::Hidden;
            }
            let holders = x.holders(u);
            if let Some(&h) = holders.iter().find(|&&h| !self.vis.contains(h)) {
                return Why::Inside(h);
            }
            match holders.first() {
                Some(&h) => u = m.cell(h).unit,
                None => return Why::Hidden,
            }
        }
    }

    /// Of material `id`'s cells (0: void).
    pub fn material(&self, id: i64) -> Counts {
        self.materials.get(&id).copied().unwrap_or_default()
    }

    /// The universe level shown, if one is.
    pub fn level(&self) -> Option<usize> {
        self.level
    }
}

/// What an eye of a panel shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Eye {
    /// All seen.
    Open,
    /// Nothing to show (a material no cell is made of).
    Empty,
    /// Some seen (of a material: `seen` of `cells`).
    Partly(u32, u32),
    /// Hidden; clicking shows it with this edit.
    Closed(Edit),
    /// Hidden by something else (why); clicking shows it with the edit
    /// when there is one (isolating: it joins the isolated cells).
    Away(String, Option<Edit>),
}

impl Eye {
    pub fn shown(&self) -> bool {
        matches!(self, Eye::Open | Eye::Partly(..))
    }
}

/// The eye of cell `c` in `scene`.
pub fn cell_eye(m: &Model, x: &ModelIndex, scene: &Scene, seen: &Seen, c: u32) -> Eye {
    let cell = m.cell(c);
    let q = Query::cell(cell.id);
    let h = &scene.hiding;
    let widen = seen.isolated_out(c);
    match seen.why(m, x, c) {
        Why::Seen => Eye::Open,
        Why::IsolatedAway => Eye::Away(
            format!(
                "Hidden: isolating {}\n\nClick to show it too",
                isolated(scene)
            ),
            Some(Edit::Reveal(q, true)),
        ),
        Why::Hidden if h.cells.contains(&cell.id) || h.rules.contains(&q) => {
            Eye::Closed(Edit::Reveal(q, widen))
        }
        Why::Hidden => Eye::Away(format!("Hidden: {}", hidden_by(m, scene, c)), None),
        Why::Inside(holder) => Eye::Away(
            format!("Hidden: inside cell {}, which is hidden", m.cell(holder).id),
            None,
        ),
        Why::BelowLevel => Eye::Away(
            format!(
                "Not drawn: below the level shown (level {})",
                seen.level().unwrap_or(0)
            ),
            None,
        ),
    }
}

/// The eye of material `id` (0: void cells) in `scene`.
pub fn material_eye(m: &Model, x: &ModelIndex, scene: &Scene, seen: &Seen, id: i64) -> Eye {
    let k = seen.material(id);
    let q = if id == 0 {
        Query::Void
    } else {
        Query::material(id)
    };
    if k.cells == 0 {
        return Eye::Empty;
    }
    if k.seen == k.cells {
        return Eye::Open;
    }
    if k.seen > 0 {
        return Eye::Partly(k.seen, k.cells);
    }
    let h = &scene.hiding;
    let own = if id == 0 {
        h.void || h.rules.contains(&q)
    } else {
        h.materials.contains(&id) || h.rules.contains(&q)
    };
    if own {
        return Eye::Closed(Edit::Reveal(q, k.left_out > 0));
    }
    if k.isolated_away > 0 {
        return Eye::Away(
            format!(
                "Hidden: isolating {}\n\nClick to show it too",
                isolated(scene)
            ),
            Some(Edit::Reveal(q, true)),
        );
    }
    let why = match x.material_cells(id).first() {
        Some(&c) => match cell_eye(m, x, scene, seen, c) {
            Eye::Away(why, _) => why,
            _ => format!("Hidden: {}", hidden_by(m, scene, c)),
        },
        None => unreachable!("a material with cells"),
    };
    Eye::Away(why, None)
}

fn isolated(scene: &Scene) -> String {
    scene
        .hiding
        .isolate
        .as_ref()
        .map_or_else(String::new, |q| q.to_string())
}

/// What hides cell `c` (hidden by the lists or rules), in words.
fn hidden_by(m: &Model, scene: &Scene, c: u32) -> String {
    let h = &scene.hiding;
    let cell = m.cell(c);
    let universe = m.units()[cell.unit as usize].universe;
    if h.cells.contains(&cell.id) {
        "hidden by number".into()
    } else if h.universes.contains(&universe) {
        format!("universe {universe} is hidden")
    } else if cell.fill.is_none() && cell.material == 0 && h.void {
        "void cells are hidden".into()
    } else if cell.fill.is_none() && h.materials.contains(&cell.material) {
        format!("material {} is hidden", cell.material)
    } else {
        match h.rules.as_slice() {
            [] => "hidden".into(),
            [r] => format!("hidden by the rule {r}"),
            rs => format!(
                "hidden by one of the rules {}",
                rs.iter()
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llevant::Options;

    /// Universe 1 (steel 10 in water 11) in envelopes 1 and 4; cell 2 void
    /// around them.
    const MODEL: &str = "seen
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

    fn setup() -> (Model, ModelIndex) {
        let m = Model::from_text(MODEL, &Options::default());
        let x = ModelIndex::new(&m);
        (m, x)
    }

    fn after(m: &Model, x: &ModelIndex, edits: &[Edit]) -> (Scene, Seen) {
        let mut s = Scene::default();
        for e in edits {
            e.apply(&mut s);
        }
        let vis = s.visibility(m, x).unwrap();
        let seen = Seen::new(m, x, &s, &vis);
        (s, seen)
    }

    fn c(m: &Model, id: i64) -> u32 {
        m.cell_index(id).unwrap()
    }

    #[test]
    fn isolating_closes_the_other_eyes_and_a_click_shows_them_again() {
        let (m, x) = setup();
        let (s, seen) = after(&m, &x, &[Edit::Isolate(Some(Query::cell(10)))]);
        assert_eq!(material_eye(&m, &x, &s, &seen, 3), Eye::Open);
        let water = material_eye(&m, &x, &s, &seen, 5);
        let Eye::Away(why, Some(show)) = water else {
            panic!("{water:?}");
        };
        assert!(why.contains("isolating c 10"), "{why}");
        // Clicking it shows the water: the isolated rule widened.
        let (s, seen) = after(&m, &x, &[Edit::Isolate(Some(Query::cell(10))), show]);
        assert_eq!(
            s.hiding.isolate,
            Some(Query::cell(10).plus(Query::material(5)))
        );
        assert_eq!(material_eye(&m, &x, &s, &seen, 5), Eye::Open);
        assert!(seen.is_seen(c(&m, 11)));
    }

    #[test]
    fn cell_eyes_follow_isolating_hiding_and_holders() {
        let (m, x) = setup();
        let iso = Edit::Isolate(Some(Query::cell(1)));
        let (s, seen) = after(&m, &x, std::slice::from_ref(&iso));
        // Envelope 4 is isolated away; clicking adds it to the isolated.
        let Eye::Away(_, Some(show)) = cell_eye(&m, &x, &s, &seen, c(&m, 4)) else {
            panic!();
        };
        let (s, seen) = after(&m, &x, &[iso.clone(), show]);
        assert!(seen.is_seen(c(&m, 4)));
        assert_eq!(s.hiding.isolate, Some(Query::Cells(vec![1, 4])));

        // Hidden by number and isolated away: one click shows it.
        let hide = Edit::Hide(Query::cell(4));
        let (s, seen) = after(&m, &x, &[hide.clone(), iso]);
        let Eye::Closed(show) = cell_eye(&m, &x, &s, &seen, c(&m, 4)) else {
            panic!();
        };
        assert_eq!(show, Edit::Reveal(Query::cell(4), true));

        // Inside hidden envelopes: no click shows it, the tooltip says why.
        let (s, seen) = after(&m, &x, &[Edit::Hide(Query::cell(1)), hide]);
        let eye = cell_eye(&m, &x, &s, &seen, c(&m, 10));
        assert!(
            matches!(&eye, Eye::Away(w, None) if w.contains("inside cell")),
            "{eye:?}"
        );
        assert!(!eye.shown());
        let steel = material_eye(&m, &x, &s, &seen, 3);
        assert!(
            matches!(&steel, Eye::Away(w, None) if w.contains("inside cell")),
            "{steel:?}"
        );
    }

    #[test]
    fn void_eyes_close_and_open_by_the_void_rule() {
        let (m, x) = setup();
        let (s, seen) = after(&m, &x, &[]);
        let Eye::Closed(show) = material_eye(&m, &x, &s, &seen, 0) else {
            panic!();
        };
        assert_eq!(show, Edit::Reveal(Query::Void, false));
        let (s, seen) = after(&m, &x, &[show.clone(), Edit::Hide(Query::cell(2))]);
        assert!(!s.hiding.void);
        assert_eq!(material_eye(&m, &x, &s, &seen, 0), Eye::Partly(1, 2));
    }

    #[test]
    fn levels_hide_what_is_deeper() {
        let (m, x) = setup();
        let (s, seen) = after(&m, &x, &[Edit::Level(Some(0))]);
        assert_eq!(seen.why(&m, &x, c(&m, 10)), Why::BelowLevel);
        assert!(seen.is_seen(c(&m, 1)));
        assert!(!cell_eye(&m, &x, &s, &seen, c(&m, 11)).shown());
    }
}
