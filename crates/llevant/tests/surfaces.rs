//! Surface queries: the surfaces of a cell, the one nearest a point, and the
//! ones through a point.

use llevant::{Model, Options, SurfaceId};

/// A box (macrobody), a moved copy of it, and two coincident planes at x = 5.
const TEXT: &str = "surfaces
10 0 -16 imp:n=1
40 0 -16 trcl=(0 10 0) imp:n=1
20 0 16 #40 -21 -99 imp:n=1
30 0 22 -99 imp:n=1
99 0 99 imp:n=0

16 rpp -1 1 -1 1 -1 1
21 px 5
22 px 5
99 so 50

";

fn model() -> Model {
    let m = Model::from_text(TEXT, &Options::default());
    assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
    m
}

fn id(m: &Model, s: u32) -> (i64, u8) {
    let SurfaceId { id, facet } = m.surface_id(s).unwrap();
    (id, facet)
}

const DIRS: [[f64; 3]; 4] = [
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
];

#[test]
fn nearest_names_the_facet() {
    let m = model();
    let n = m
        .nearest_surface([1.2, 0.0, 0.0], &DIRS, 0.5, None)
        .unwrap()
        .unwrap();
    // RPP facets in MCNP order: +x is the first.
    assert_eq!(id(&m, n.shape), (16, 1));
    assert!((n.distance - 0.2).abs() < 1e-9, "{n:?}");
    assert!((n.point[0] - 1.0).abs() < 1e-9, "{n:?}");
}

#[test]
fn nearest_in_a_moved_copy_keeps_the_number() {
    let m = model();
    let n = m
        .nearest_surface([0.0, 11.2, 0.0], &DIRS, 0.5, None)
        .unwrap()
        .unwrap();
    assert_eq!(id(&m, n.shape), (16, 3));
    assert!((n.point[1] - 11.0).abs() < 1e-9, "{n:?}");
}

#[test]
fn nothing_within_reach() {
    let m = model();
    assert!(m
        .nearest_surface([3.0, 0.0, 0.0], &DIRS, 0.5, None)
        .unwrap()
        .is_none());
}

#[test]
fn bad_reach_is_an_error() {
    let m = model();
    assert!(m
        .nearest_surface([3.0, 0.0, 0.0], &DIRS, f64::NAN, None)
        .is_err());
}

#[test]
fn coincident_surfaces_are_all_named() {
    let m = model();
    let n = m
        .nearest_surface([5.3, 0.0, 0.0], &DIRS, 0.5, None)
        .unwrap()
        .unwrap();
    assert!(matches!(id(&m, n.shape), (21, 0) | (22, 0)), "{n:?}");
    let across: Vec<_> = m
        .surfaces_across(n.point, n.dir)
        .into_iter()
        .map(|s| id(&m, s))
        .collect();
    assert_eq!(across, vec![(21, 0), (22, 0)]);
}

#[test]
fn across_a_facet_names_only_it() {
    let m = model();
    let across: Vec<_> = m
        .surfaces_across([1.0, 0.0, 0.0], [1.0, 0.0, 0.0])
        .into_iter()
        .map(|s| id(&m, s))
        .collect();
    assert_eq!(across, vec![(16, 1)]);
}

#[test]
fn cell_surfaces_with_senses() {
    let m = model();
    let c = m.cell_index(20).unwrap();
    let mut got: Vec<(i64, u8, bool)> = m
        .cell_surfaces(c)
        .into_iter()
        .map(|(s, pos)| {
            let (i, f) = id(&m, s);
            (i, f, pos)
        })
        .collect();
    got.sort();
    got.dedup();
    assert!(got.contains(&(21, 0, false)), "{got:?}");
    assert!(got.contains(&(99, 0, false)), "{got:?}");
    // `16`: outside the box, so the positive side of some facet; `#40`
    // brings the moved box's facets.
    assert!(got.iter().any(|&(i, _, pos)| i == 16 && pos), "{got:?}");
    // Moved copies are other compiled surfaces with the same number.
    let shapes_16 = m
        .cell_surfaces(c)
        .into_iter()
        .filter(|&(s, _)| id(&m, s).0 == 16)
        .map(|(s, _)| s)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(shapes_16.len(), 12, "{shapes_16:?}");
    assert!(!m.cell_surfaces(m.cell_index(30).unwrap()).is_empty());
}

/// A box filled, through a transform, with a universe holding a sphere.
const NESTED: &str = "nested
1 0 -16 fill=1 (3 0 0) imp:n=1
2 0 -15 u=1 imp:n=1
3 0 15 u=1 imp:n=1
9 0 16 -99 imp:n=1
99 0 99 imp:n=0

15 so 0.5
16 rpp 2 4 -1 1 -1 1
99 so 50

";

#[test]
fn surfaces_in_a_filled_universe_and_levels() {
    let m = Model::from_text(NESTED, &Options::default());
    assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
    // The sphere, in the universe's frame, is centred at x = 3.
    let n = m
        .nearest_surface([3.6, 0.0, 0.0], &DIRS, 0.3, None)
        .unwrap()
        .unwrap();
    assert_eq!(id(&m, n.shape), (15, 0));
    assert!((n.point[0] - 3.5).abs() < 1e-9, "{n:?}");
    let across: Vec<_> = m
        .surfaces_across(n.point, n.dir)
        .into_iter()
        .map(|s| id(&m, s))
        .collect();
    assert_eq!(across, vec![(15, 0)]);
    // At level 0 the universe is not traced: the box's +x facet is nearest.
    let n = m
        .nearest_surface([3.6, 0.0, 0.0], &DIRS, 0.5, Some(0))
        .unwrap()
        .unwrap();
    assert_eq!(id(&m, n.shape), (16, 1));
}

/// Two boxes side by side: their top facets are one plane.
const SIDE_BY_SIDE: &str = "side by side
1 0 -1 imp:n=1
2 0 -2 imp:n=1
3 0 1 2 -9 imp:n=1
9 0 9 imp:n=0

1 rpp -10 0 -5 5 -5 5
2 rpp 0 10 -5 5 -5 5
9 so 100

";

#[test]
fn the_box_that_bounds_is_named() {
    let m = Model::from_text(SIDE_BY_SIDE, &Options::default());
    assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
    // Above the right box, the top of the right box (facet 3 of 2), though
    // facet 3 of box 1 is the same plane.
    for (x, want) in [(5.0, 2), (-5.0, 1)] {
        let n = m
            .nearest_surface([x, 5.3, 0.1], &DIRS, 0.5, None)
            .unwrap()
            .unwrap();
        assert_eq!(id(&m, n.shape), (want, 3), "x = {x}");
        let across: Vec<_> = m
            .surfaces_across(n.point, n.dir)
            .into_iter()
            .map(|s| id(&m, s))
            .collect();
        assert_eq!(across, vec![(want, 3)], "x = {x}");
    }
}

#[test]
fn normals_point_to_the_positive_side() {
    let m = model();
    let shape = |id: i64| {
        (0..m.shape_count() as u32)
            .find(|&s| m.surface_id(s).is_some_and(|x| x.id == id))
            .unwrap()
    };
    // The sphere: outwards, radial.
    let n = m.surface_normal(shape(99), [0.0, 0.0, 50.0]).unwrap();
    assert!((n[2] - 1.0).abs() < 1e-12 && n[0].abs() < 1e-12, "{n:?}");
    let n = m.surface_normal(shape(99), [30.0, 40.0, 0.0]).unwrap();
    assert!(
        (n[0] - 0.6).abs() < 1e-12 && (n[1] - 0.8).abs() < 1e-12,
        "{n:?}"
    );
    // A plane: +x.
    assert_eq!(
        m.surface_normal(shape(21), [5.0, 3.0, 1.0]),
        Some([1.0, 0.0, 0.0])
    );
    assert_eq!(m.surface_normal(u32::MAX, [0.0; 3]), None);
}

/// Points in the frame of a universe: through the transform of the cell
/// filled with it, one key per instance.
#[test]
fn points_in_a_universe_frame() {
    let m = Model::from_text(
        "frames
1 0 -1 fill=5 (10 0 0) imp:n=1
2 0 -2 fill=5 (-10 0 0) imp:n=1
3 0 1 2 -99 imp:n=1
9 0 99 imp:n=0
50 0 -3 u=5 imp:n=1
51 0 3 u=5 imp:n=1

1 s 10 0 0 4
2 s -10 0 0 4
3 so 1
99 so 50

",
        &Options::default(),
    );
    assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
    let unit = m.cell(m.cell_index(50).unwrap()).unit;
    assert_ne!(unit, 0);
    let (q, a) = m.point_in_unit([10.5, 0.0, 0.0], unit).unwrap();
    assert!((q[0] - 0.5).abs() < 1e-12, "{q:?}");
    let (q, b) = m.point_in_unit([-12.0, 1.0, 0.0], unit).unwrap();
    assert!(
        (q[0] + 2.0).abs() < 1e-12 && (q[1] - 1.0).abs() < 1e-12,
        "{q:?}"
    );
    assert_ne!(a, b, "two instances");
    assert!(m.point_in_unit([0.0, 30.0, 0.0], unit).is_none());
    // Cells by their own programs, in that frame.
    let (c50, c51) = (m.cell_index(50).unwrap(), m.cell_index(51).unwrap());
    assert!(m.cell_holds(c50, [0.5, 0.0, 0.0]) && !m.cell_holds(c51, [0.5, 0.0, 0.0]));
    assert!(m.cell_holds(c51, [2.0, 0.0, 0.0]) && !m.cell_holds(c50, [2.0, 0.0, 0.0]));
    assert_eq!(
        m.point_in_unit([0.0, 30.0, 0.0], 0),
        Some(([0.0, 30.0, 0.0], 0))
    );
    // The same as a map, and surface 3 of the instance at -10 written in
    // the problem's frame: a sphere of radius 1 about (-10, 0, 0).
    let p = [-12.0, 1.0, 0.0];
    let (g, key) = m.unit_frame(p, unit).unwrap();
    assert_eq!(key, b);
    let q = g.apply(p);
    assert!((q[0] + 2.0).abs() < 1e-12 && (q[1] - 1.0).abs() < 1e-12);
    let s3 = m.cell_surfaces(c50)[0].0;
    let (kind, own) = m.shape_record(s3, None).unwrap();
    assert_eq!(kind, llevant::export::SHAPE_QUADRIC);
    assert_eq!(own[..3], [0.0, 0.0, 0.0]);
    let (_, moved) = m.shape_record(s3, Some(&g)).unwrap();
    assert_eq!(moved[..3], [-10.0, 0.0, 0.0]);
    assert!(m.unit_frame([0.0, 30.0, 0.0], unit).is_none());
}
