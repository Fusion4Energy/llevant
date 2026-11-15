//! Where cells are placed: the boxes of their instances in the global frame,
//! for the lines of volumes around a few cells and the cells a cutaway keeps.

use super::*;

/// Instances [`Model::instance_volumes`] bounds one by one, over all sets
/// (smallest sets first); the instances of the sets beyond use the boxes of
/// their cells at every placement ([`PlacedBoxes`]).
pub(super) const MAX_BOXED_INSTANCES: usize = 1 << 18;

/// Boxes all instances bounded one by one are covered by in a round, at
/// most ([`COVER_BOXES`] each while there are few).
pub(super) const MAX_COVER_BOXES: usize = 1 << 20;

/// Lines per probe across the longest side of a set's largest box at which
/// [`Model::instance_volumes`] starts tracing it.
pub(super) const START_LINES_ACROSS: f64 = 8.0;

/// Universes placed in more spots than this are bounded in the global frame
/// by one box around all of them ([`Placement::Within`]).
pub(super) const MAX_PLACEMENTS: usize = 1024;

/// Where a universe's frame lies in the global one, to bound its cells there.
#[derive(Clone, Copy, Debug)]
pub(super) enum Placement {
    /// Mapped to the global frame by the transform (`None`: the identity),
    /// clipped to the box (the filled cells above it, in the global frame).
    Exact(Option<Affine>, Aabb),
    /// Somewhere in the box: below a lattice, whose elements are not
    /// followed one by one, or one of too many placements.
    Within(Aabb),
}

impl Placement {
    pub(super) fn clip(&self) -> Aabb {
        match self {
            Placement::Exact(_, b) | Placement::Within(b) => *b,
        }
    }

    /// Global box of what a cell with box `b` (in the universe's frame)
    /// holds here; empty if it can hold nothing.
    pub(super) fn bound(&self, b: &Aabb) -> Aabb {
        if b.is_empty() {
            return Aabb::EMPTY;
        }
        match self {
            Placement::Exact(None, clip) => b.intersect(clip),
            Placement::Exact(Some(t), clip) => b.transformed(t).intersect(clip),
            Placement::Within(clip) => *clip,
        }
    }
}

/// The global box of one instance of a cell ([`Model::placed_boxes`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedBox {
    /// The cell (index into [`Model::cells`]).
    pub cell: u32,
    /// Its box in the global frame (conservative: padded outward).
    pub global: Aabb,
    /// Whether the cell was bounded again where it is placed. `false`: an
    /// outer bound only, the box of where it may be (below a lattice, whose
    /// elements are not followed one by one, or in a universe placed in too
    /// many spots), or the cell's own box moved there (unsupported cells,
    /// lattice cells).
    pub exact: bool,
}

/// One instance's box: in the global frame, and where it came from when
/// it can be bounded again more finely ([`PlacedBoxes::cover`]).
#[derive(Clone, Copy, Debug)]
pub(super) struct Placed {
    pub(super) global: Aabb,
    /// The map from the universe's frame to the global one (`None`: the
    /// identity), the box in the universe's frame, and the clip box.
    pub(super) frame: Option<(Option<Affine>, Aabb, Aabb)>,
}

/// Conservative global boxes of every cell's instances in a region.
pub(super) struct PlacedBoxes<'a> {
    pub(super) model: &'a Model,
    pub(super) bounder: Bounder<'a>,
    /// By cell: its instances with non-empty boxes.
    pub(super) placed: Vec<Vec<Placed>>,
    /// By cell: the global boxes of `placed`.
    pub(super) boxes: Vec<Vec<Aabb>>,
}

/// Size of the boxes [`PlacedBoxes::cover`] stops splitting at, in line
/// spacings of the round: smaller ones would hold too few lines to be worth
/// their own.
pub(super) const COVER_SPACINGS: f64 = 8.0;

/// Boxes a cell's box at one placement is split into at most by
/// [`PlacedBoxes::cover`].
pub(super) const COVER_BOXES: usize = 256;

/// Splits of one instance's box [`Model::cells_kept`] makes at most before
/// taking a cell across a cut as kept.
pub const CUT_SPLITS: usize = 64;

/// Side (cm) below which [`Model::cells_kept`] stops splitting a box across
/// a cut and takes the cell as kept.
pub const CUT_SIDE: f64 = 1e-3;

impl<'a> PlacedBoxes<'a> {
    pub(super) fn new(m: &'a Model, region: Aabb) -> PlacedBoxes<'a> {
        // A filled cell's instances are what fills it.
        PlacedBoxes::of(m, region, |c| {
            m.cells[c as usize].fill.is_none() || m.is_lattice(c)
        })
    }

    /// The instances of the cells `keep` is true for (others have none).
    pub(super) fn of(
        m: &'a Model,
        region: Aabb,
        keep: impl Fn(u32) -> bool + Sync,
    ) -> PlacedBoxes<'a> {
        let bounder = Bounder::new(&m.shapes);
        let place = |c: u32, p: &Placement| place(m, &bounder, c, p);
        // The units that hold a cell kept, or hold one that does: only the
        // filled cells leading to them are placed.
        let order = crate::model::topo_units(m);
        let mut needed = vec![false; m.units.len()];
        for (c, cell) in m.cells.iter().enumerate() {
            if keep(c as u32) {
                needed[cell.unit as usize] = true;
            }
        }
        for &u in order.iter().rev() {
            if !needed[u as usize] {
                needed[u as usize] = m.units[u as usize]
                    .cells
                    .iter()
                    .any(|&c| m.fill_targets(c).any(|t| needed[t.unit as usize]));
            }
        }
        let mut places: Vec<Vec<Placement>> = vec![Vec::new(); m.units.len()];
        places[0].push(Placement::Exact(None, region));
        for u in order {
            if !needed[u as usize] {
                continue;
            }
            let mut here = std::mem::take(&mut places[u as usize]);
            if here.len() > MAX_PLACEMENTS {
                let all = here.iter().fold(Aabb::EMPTY, |a, p| a.union(&p.clip()));
                here = vec![Placement::Within(all)];
            }
            let filled: Vec<u32> = m.units[u as usize]
                .cells
                .iter()
                .copied()
                .filter(|&c| {
                    m.cells[c as usize].fill.is_some()
                        && m.fill_targets(c).any(|t| needed[t.unit as usize])
                })
                .collect();
            let hosts: Vec<Vec<(usize, Aabb)>> = filled
                .par_iter()
                .map(|&c| {
                    here.iter()
                        .enumerate()
                        .map(|(k, p)| (k, place(c, p).global))
                        .filter(|(_, b)| !b.is_empty())
                        .collect()
                })
                .collect();
            for (&c, hosts) in filled.iter().zip(hosts) {
                for (k, host) in hosts {
                    match &m.cells[c as usize].fill {
                        Some(Fill::Universe(t)) => {
                            let child = match &here[k] {
                                Placement::Exact(g, _) => {
                                    let up = t.transform().map(|t| t.inverse());
                                    Placement::Exact(then(*g, up), host)
                                }
                                Placement::Within(_) => Placement::Within(host),
                            };
                            places[t.unit() as usize].push(child);
                        }
                        Some(Fill::Lattice(l)) => {
                            let mut units: Vec<u32> = m.lattices[*l as usize]
                                .targets()
                                .map(|t| t.unit())
                                .collect();
                            units.sort_unstable();
                            units.dedup();
                            for t in units {
                                places[t as usize].push(Placement::Within(host));
                            }
                        }
                        None => unreachable!(),
                    }
                }
            }
            places[u as usize] = here;
        }
        let placed: Vec<Vec<Placed>> = (0..m.cells.len() as u32)
            .into_par_iter()
            .map(|c| {
                if !keep(c) {
                    return Vec::new();
                }
                places[m.cells[c as usize].unit as usize]
                    .iter()
                    .map(|p| place(c, p))
                    .filter(|p| !p.global.is_empty())
                    .collect()
            })
            .collect();
        let boxes = placed
            .iter()
            .map(|ps| ps.iter().map(|p| p.global).collect())
            .collect();
        PlacedBoxes {
            model: m,
            bounder,
            placed,
            boxes,
        }
    }

    /// Non-empty global boxes of cell `c`'s instances.
    pub(super) fn boxes(&self, c: u32) -> &[Aabb] {
        &self.boxes[c as usize]
    }

    /// Global boxes covering cell `c`, finer than [`PlacedBoxes::boxes`]:
    /// each box in the universe's frame is split in two across its longest
    /// side, largest first, the halves bounded again (those the cell
    /// cannot reach drop out), until the sides are at most `side` or there
    /// are [`COVER_BOXES`]. Conservative like the boxes: a thin cell
    /// across a large box is covered by boxes along it.
    pub(super) fn cover(&self, c: u32, side: f64, out: &mut Vec<Aabb>) {
        for p in &self.placed[c as usize] {
            cover_placed(self.model, &self.bounder, c, p, side, COVER_BOXES, out);
        }
    }
}

/// Cell `c` placed at `p`: its program bounded again within what it can
/// reach there, which can be much less than its box in the universe's frame
/// (or nothing, like the outside of a universe's sphere in envelopes well
/// inside it).
pub(super) fn place(m: &Model, bounder: &Bounder, c: u32, p: &Placement) -> Placed {
    let cell = &m.cells[c as usize];
    let loose = Placed {
        global: p.bound(&cell.bbox),
        frame: None,
    };
    let Placement::Exact(g, clip) = p else {
        return loose;
    };
    if cell.bbox.is_empty() || cell.unsupported.is_some() || m.is_lattice(c) {
        return loose;
    }
    let local = local_clip(g, clip).intersect(&cell.bbox);
    let local = if local.is_empty() {
        local
    } else {
        bounder.bound(m.code(c), local)
    };
    Placed {
        global: to_global(g, &local, clip),
        frame: Some((*g, local, *clip)),
    }
}

/// Global boxes covering cell `c` at one placement `p`
/// ([`PlacedBoxes::cover`]), at most `max_boxes` of them.
pub(super) fn cover_placed(
    m: &Model,
    bounder: &Bounder,
    c: u32,
    p: &Placed,
    side: f64,
    max_boxes: usize,
    out: &mut Vec<Aabb>,
) {
    let Some((g, local, clip)) = &p.frame else {
        out.push(p.global);
        return;
    };
    let code = m.code(c);
    let longest = |b: &Aabb| (0..3).map(|k| b.hi[k] - b.lo[k]).fold(0.0, f64::max);
    let mut done: Vec<Aabb> = Vec::new();
    let mut todo: Vec<Aabb> = vec![*local];
    while let Some(i) =
        (0..todo.len()).max_by(|&a, &b| todo[a].volume().total_cmp(&todo[b].volume()))
    {
        if done.len() + todo.len() >= max_boxes {
            break;
        }
        let b = todo.swap_remove(i);
        if longest(&b) <= side {
            done.push(b);
            continue;
        }
        let k = (0..3)
            .max_by(|&x, &y| (b.hi[x] - b.lo[x]).total_cmp(&(b.hi[y] - b.lo[y])))
            .unwrap();
        let mid = 0.5 * (b.lo[k] + b.hi[k]);
        let (mut lo, mut hi) = (b, b);
        lo.hi[k] = mid;
        hi.lo[k] = mid;
        for half in [lo, hi] {
            let h = bounder.bound(code, half);
            if !h.is_empty() {
                todo.push(h);
            }
        }
    }
    out.extend(
        done.iter()
            .chain(&todo)
            .map(|b| to_global(g, b, clip))
            .filter(|b| !b.is_empty()),
    );
}

/// `outer` after `inner` (`None`: the identity).
pub(super) fn then(outer: Option<Affine>, inner: Option<Affine>) -> Option<Affine> {
    match (outer, inner) {
        (Some(o), Some(i)) => Some(o.compose(&i)),
        (o, i) => o.or(i),
    }
}

/// The clip box of a placement in the universe's frame.
pub(super) fn local_clip(g: &Option<Affine>, clip: &Aabb) -> Aabb {
    match g {
        Some(g) => clip.transformed(&g.inverse()),
        None => *clip,
    }
}

/// Instance `id`'s cell and box, its frame followed exactly down its path,
/// lattice elements too, clipped to `region`. `None` for ids without a path.
pub(super) fn place_instance(
    m: &Model,
    bounder: &Bounder,
    region: Aabb,
    id: u64,
) -> Option<(u32, Placed)> {
    let path = m.instance_path(id)?;
    let (last, above) = path.split_last()?;
    // Clip `clip` to element `ix` of `lat`, in the frame `g` maps to the
    // global one.
    let element = |lat: &crate::lattice::Lattice, ix, g: &Option<Affine>, clip: &Aabb| {
        let local = lat.element_box(ix).intersect(&local_clip(g, clip));
        to_global(g, &local, clip)
    };
    let mut g: Option<Affine> = None;
    let mut clip = region;
    for s in above {
        let host = place(m, bounder, s.cell, &Placement::Exact(g, clip)).global;
        if host.is_empty() {
            return Some((
                last.cell,
                Placed {
                    global: host,
                    frame: None,
                },
            ));
        }
        match &m.cells[s.cell as usize].fill {
            Some(Fill::Universe(t)) => {
                g = then(g, t.transform().map(|t| t.inverse()));
                clip = host;
            }
            Some(Fill::Lattice(l)) => {
                let lat = &m.lattices[*l as usize];
                let ix = s.lattice?;
                let crate::lattice::Element::Fill(t) = lat.element(ix) else {
                    return None;
                };
                clip = element(lat, ix, &g, &host);
                let shift = (0..3).fold([0.0; 3], |a, i| add(a, scale(lat.basis[i], ix[i] as f64)));
                g = then(
                    then(g, Some(Affine::translation(shift))),
                    t.transform().map(|t| t.inverse()),
                );
            }
            None => return None,
        }
    }
    if let (Some(Fill::Lattice(l)), Some(ix)) = (&m.cells[last.cell as usize].fill, last.lattice) {
        clip = element(&m.lattices[*l as usize], ix, &g, &clip);
    }
    Some((
        last.cell,
        place(m, bounder, last.cell, &Placement::Exact(g, clip)),
    ))
}

/// A box of a universe's frame in the global frame, clipped.
pub(super) fn to_global(g: &Option<Affine>, b: &Aabb, clip: &Aabb) -> Aabb {
    if b.is_empty() {
        return *b;
    }
    match g {
        Some(g) => b.transformed(g),
        None => *b,
    }
    .intersect(clip)
}

impl Model {
    /// Global boxes covering every instance of `cells` (indices) within
    /// `region`: each instance's box, bounded again where it is placed and
    /// split down to sides of about `side` (at most 256 per instance) where
    /// the cell does not reach the whole of it. A filled cell is covered as
    /// a cell (not by what fills it). Conservative, like the cells' boxes.
    pub fn instance_boxes(&self, cells: &[u32], region: Aabb, side: f64) -> Vec<Aabb> {
        if !region.is_finite() || region.is_empty() || cells.is_empty() {
            return Vec::new();
        }
        let mut wanted = vec![false; self.cells.len()];
        for &c in cells {
            if let Some(w) = wanted.get_mut(c as usize) {
                *w = true;
            }
        }
        let placed = PlacedBoxes::of(self, region, |c| wanted[c as usize]);
        let side = if side > 0.0 { side } else { f64::INFINITY };
        let mut out = Vec::new();
        for (c, &w) in wanted.iter().enumerate() {
            if w {
                placed.cover(c as u32, side, &mut out);
            }
        }
        out
    }

    /// The global box of every instance of `cells` (indices) within
    /// `region`, cell by cell in the order of `cells` (duplicates once):
    /// the cell bounded again where each instance is placed, as
    /// [`Model::instance_boxes`] without splitting. A filled cell is boxed
    /// as a cell. Conservative (padded outward); [`PlacedBox::exact`] says
    /// which boxes are only an outer bound.
    pub fn placed_boxes(&self, cells: &[u32], region: Aabb) -> Vec<PlacedBox> {
        if !region.is_finite() || region.is_empty() || cells.is_empty() {
            return Vec::new();
        }
        let mut wanted = vec![false; self.cells.len()];
        for &c in cells {
            if let Some(w) = wanted.get_mut(c as usize) {
                *w = true;
            }
        }
        let placed = PlacedBoxes::of(self, region, |c| wanted[c as usize]);
        let mut out = Vec::new();
        for &c in cells {
            // Out of range or seen already.
            if !wanted.get_mut(c as usize).is_some_and(std::mem::take) {
                continue;
            }
            out.extend(placed.placed[c as usize].iter().map(|p| PlacedBox {
                cell: c,
                global: p.global,
                exact: p.frame.is_some(),
            }));
        }
        out
    }

    /// Whether each of `cells` (indices) may have some part, in some
    /// instance within `region`, that `cut` does not cut away: per cell, in
    /// the order of `cells`. Each instance's box is bounded again where it
    /// is placed and, while it lies across a cut, split in two across its
    /// longest side and the halves bounded again (as
    /// [`Model::instance_boxes`]), until a piece is wholly kept (the cell
    /// is kept) or every piece is wholly cut away (that instance is). After
    /// 64 splits of an instance, or below a side of 1e-3 cm, the
    /// cell counts as kept: conservative, a cell `false` here is wholly cut
    /// away, while one `true` may only come close to what is kept (by the
    /// padding of the boxes, or by the size of the last pieces). A filled
    /// cell is taken as a cell (not by what fills it).
    pub fn cells_kept(&self, cells: &[u32], region: Aabb, cut: &crate::Cutaway) -> Vec<bool> {
        use crate::view::BoxCut;
        if cut.is_empty() {
            return cells
                .iter()
                .map(|&c| (c as usize) < self.cells.len())
                .collect();
        }
        if !region.is_finite() || region.is_empty() || cells.is_empty() {
            return vec![false; cells.len()];
        }
        let mut wanted = vec![false; self.cells.len()];
        for &c in cells {
            if let Some(w) = wanted.get_mut(c as usize) {
                *w = true;
            }
        }
        let placed = PlacedBoxes::of(self, region, |c| wanted[c as usize]);
        let kept_at = |c: u32, p: &Placed| -> bool {
            match cut.place_box(&p.global) {
                BoxCut::Kept => return true,
                BoxCut::Removed => return false,
                BoxCut::Unknown => {}
            }
            let Some((g, local, clip)) = &p.frame else {
                return true;
            };
            let code = self.code(c);
            let mut todo = vec![*local];
            let mut splits = 0;
            while let Some(b) = todo.pop() {
                let global = to_global(g, &b, clip);
                if global.is_empty() {
                    continue;
                }
                match cut.place_box(&global) {
                    BoxCut::Kept => return true,
                    BoxCut::Removed => continue,
                    BoxCut::Unknown => {}
                }
                let k = (0..3)
                    .max_by(|&x, &y| (b.hi[x] - b.lo[x]).total_cmp(&(b.hi[y] - b.lo[y])))
                    .unwrap();
                if splits == CUT_SPLITS || b.hi[k] - b.lo[k] <= CUT_SIDE {
                    return true;
                }
                splits += 1;
                let mid = 0.5 * (b.lo[k] + b.hi[k]);
                let (mut lo, mut hi) = (b, b);
                lo.hi[k] = mid;
                hi.lo[k] = mid;
                for half in [lo, hi] {
                    let h = placed.bounder.bound(code, half);
                    if !h.is_empty() {
                        todo.push(h);
                    }
                }
            }
            false
        };
        cells
            .par_iter()
            .map(|&c| {
                placed
                    .placed
                    .get(c as usize)
                    .is_some_and(|ps| ps.iter().any(|p| kept_at(c, p)))
            })
            .collect()
    }
}
