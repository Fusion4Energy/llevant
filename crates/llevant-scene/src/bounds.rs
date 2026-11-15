//! Where a set of cells is, in the global frame: the box of every instance
//! of each cell (cells deep in universes counted at each place they are
//! put), and their union, to frame a view on them or to place a mesh or a
//! source around them.
//!
//! The boxes are the engine's ([`Model::placed_boxes`]): conservative,
//! padded outward, and only an outer bound below lattices
//! ([`PlacedBox::exact`]).
//!
//! [`view_from`] picks a direction to look at them from, where they show
//! most (a plate face on rather than edge on).

use llevant::{Aabb, Camera, Model, PlacedBox, Result, Scope, Status, Vec3, Visibility, NO_HIT};

use crate::index::CellSet;

/// The global box of every instance of the cells of `set`, cell by cell in
/// index order, within the problem's bounds ([`Scope::Problem`]).
pub fn instance_boxes(m: &Model, set: &CellSet) -> Vec<PlacedBox> {
    let cells: Vec<u32> = set.iter().collect();
    m.placed_boxes(&cells, m.bounds(Scope::Problem))
}

/// The box around every instance of the cells of `set` (the union of
/// [`instance_boxes`]); `None` when they are nowhere (an empty set, or
/// cells of universes no cell fills).
pub fn selection_box(m: &Model, set: &CellSet) -> Option<Aabb> {
    union(&instance_boxes(m, set))
}

/// The union of the boxes; `None` when there are none.
pub fn union(boxes: &[PlacedBox]) -> Option<Aabb> {
    let b = boxes.iter().fold(Aabb::EMPTY, |a, p| a.union(&p.global));
    (!b.is_empty()).then_some(b)
}

/// Side of the images [`view_from`] compares directions on (pixels).
const TRIAL_PX: usize = 64;

/// A direction from `frame`'s centre towards the eye (as `llevant render
/// --from`) from which what `vis` shows inside `frame` covers most of the
/// image: `preferred` (normalised) unless another of the 26 directions to the
/// faces, edges and corners of a cube, from above or level, shows 10% more
/// (then the one of those that shows most, the nearest to `preferred` among
/// equals). Measured on images of 64 × 64 rays.
pub fn view_from(m: &Model, vis: &Visibility, frame: &Aabb, preferred: Vec3) -> Result<Vec3> {
    let unit = |v: Vec3| {
        let n = v.iter().map(|x| x * x).sum::<f64>().sqrt();
        v.map(|x| x / n)
    };
    let preferred = unit(preferred);
    let shown = |from: Vec3| -> Result<usize> {
        let cam = Camera::fit(frame, from.map(|x| -x), TRIAL_PX, TRIAL_PX);
        let h = m.render(&cam, vis)?;
        Ok(h.ids
            .iter()
            .filter(|&&i| i != NO_HIT && h.labels[i as usize].status == Status::Inside)
            .count())
    };
    let mut candidates: Vec<Vec3> = Vec::new();
    for x in -1..=1 {
        for y in -1..=1 {
            for z in 0..=1 {
                if (x, y, z) != (0, 0, 0) {
                    candidates.push(unit([x as f64, y as f64, z as f64]));
                }
            }
        }
    }
    let base = shown(preferred)?;
    let mut best = (base, preferred);
    for d in candidates {
        let n = shown(d)?;
        let closer = |a: Vec3, b: Vec3| {
            let dot = |v: Vec3| (0..3).map(|k| v[k] * preferred[k]).sum::<f64>();
            dot(a) > dot(b)
        };
        if n > best.0 || (n == best.0 && closer(d, best.1)) {
            best = (n, d);
        }
    }
    Ok(if base as f64 >= 0.9 * best.0 as f64 {
        preferred
    } else {
        best.1
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use llevant::Options;

    const MODEL: &str = "bounds
1 0 -1 fill=1 imp:n=1
2 0 -2 fill=1 (20 0 0) imp:n=1
3 0 -3 fill=1 (0 30 0) imp:n=1
8 0 1 2 3 -9 imp:n=1
9 0 9 imp:n=0
10 3 -7.8 -4 u=1 imp:n=1
11 0 4 u=1 imp:n=1
12 0 -5 u=7 imp:n=1

1 so 5
2 s 20 0 0 5
3 s 0 30 0 5
4 so 2
5 so 1
9 so 100

m3 26000 1
";

    #[test]
    fn every_instance_is_boxed() {
        let m = Model::from_text(MODEL, &Options::default());
        let set = |ids: &[i64]| {
            CellSet::from_cells(
                m.cells().len(),
                ids.iter().map(|&id| m.cell_index(id).unwrap()),
            )
        };
        // Cell 10 at its three places: around (0 0 0), (20 0 0), (0 30 0).
        let boxes = instance_boxes(&m, &set(&[10]));
        assert_eq!(boxes.len(), 3);
        assert!(boxes.iter().all(|b| b.exact));
        let b = selection_box(&m, &set(&[10])).unwrap();
        for (k, (lo, hi)) in [(-2.0, 22.0), (-2.0, 32.0), (-2.0, 2.0)].iter().enumerate() {
            assert!(
                (b.lo[k] - lo).abs() < 0.1 && (b.hi[k] - hi).abs() < 0.1,
                "{b:?}"
            );
        }
        // A plate in the plane x = 0 is seen face on, not edge on from y.
        let plate = Model::from_text(
            "plate
1 3 -7.8 -1 imp:n=1
2 0 1 -9 imp:n=1
9 0 9 imp:n=0

1 rpp -0.5 0.5 -20 20 -20 20
9 so 100

m3 26000 1
",
            &Options::default(),
        );
        let vis = Visibility::solid(&plate);
        let frame = plate.cell(0).bbox;
        let from = view_from(&plate, &vis, &frame, [0.0, -1.0, 0.0]).unwrap();
        assert!(from[0].abs() > 0.99, "{from:?}");
        // A cube is fine from where it is asked.
        let cube = Model::from_text(
            &MODEL.replace("so 2", "rpp -2 2 -2 2 -2 2"),
            &Options::default(),
        );
        let want = [-0.6, -0.7, 0.4];
        let from = view_from(&cube, &Visibility::solid(&cube), &b, want).unwrap();
        let n = (0.36f64 + 0.49 + 0.16).sqrt();
        assert!(
            (0..3).all(|k| (from[k] - want[k] / n).abs() < 1e-12),
            "{from:?}"
        );
        // Universe 7 is filled nowhere.
        assert!(selection_box(&m, &set(&[12])).is_none());
        assert!(selection_box(&m, &CellSet::empty(m.cells().len())).is_none());
    }
}
