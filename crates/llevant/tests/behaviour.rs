//! Classification behaviour on small hand-written models.

use llevant::Depth;
use llevant::{Line, Model, Options, Status};

fn model(text: &str) -> Model {
    Model::from_text(text, &Options::default())
}

fn cell_at(m: &Model, p: [f64; 3]) -> (Status, Option<i64>) {
    let l = m.classify_point(p);
    (l.status, l.cell().map(|c| m.cells()[c as usize].id))
}

const BASE: &str = "\
t
1 1 -1.0 -1 imp:n=1
2 like 1 but mat=2 rho=-3.5 u=4
3 0 -2 fill=4 (10 0 0) imp:n=1
4 0 1 2 -9 imp:n=1
9 0 9 imp:n=0

1 so 2
2 s 10 0 0 3
9 so 100

m1 1001.32c 1
m2 26056.32c 1
";

#[test]
fn like_but_inherits_geometry_and_overrides_parameters() {
    let m = model(BASE);
    assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
    // Cell 2 is cell 1's sphere, moved into universe 4, which fills cell 3
    // translated by (10, 0, 0).
    let (s, c) = cell_at(&m, [10.5, 0.0, 0.0]);
    assert_eq!((s, c), (Status::Inside, Some(2)));
    let cell2 = &m.cells()[m.cell_index(2).unwrap() as usize];
    assert_eq!((cell2.material, cell2.density), (2, -3.5));
    let label = m.classify_point([10.5, 0.0, 0.0]);
    let path: Vec<i64> = label
        .path
        .iter()
        .map(|s| m.cells()[s.cell as usize].id)
        .collect();
    assert_eq!(path, vec![3, 2]);
    // Inside cell 3 but outside cell 2's sphere: universe 4 has no cell there.
    let label = m.classify_point([12.5, 0.0, 0.0]);
    assert_eq!(label.status, Status::Gap);
    assert_eq!(label.path.len(), 1);
    assert_eq!(cell_at(&m, [0.0, 0.0, 0.0]), (Status::Inside, Some(1)));
    assert_eq!(cell_at(&m, [50.0, 0.0, 0.0]), (Status::Inside, Some(4)));
    assert_eq!(cell_at(&m, [500.0, 0.0, 0.0]), (Status::Inside, Some(9)));
}

#[test]
fn gaps_overlaps_and_surfaces_are_reported() {
    let m = model(
        "t
1 0 -1 imp:n=1
2 0 -2 imp:n=1
9 0 9 imp:n=0

1 so 2
2 s 3 0 0 2
9 so 100

",
    );
    assert_eq!(m.classify_point([1.5, 0.0, 0.0]).status, Status::Overlap);
    assert_eq!(m.classify_point([1.5, 0.0, 0.0]).involved.len(), 2);
    assert_eq!(m.classify_point([10.0, 0.0, 0.0]).status, Status::Gap);
    assert_eq!(m.classify_point([-2.0, 0.0, 0.0]).status, Status::OnSurface);
    assert_eq!(cell_at(&m, [-1.0, 0.0, 0.0]), (Status::Inside, Some(1)));
}

#[test]
fn unsupported_features_only_fail_their_points() {
    let m = model(
        "t
1 0 -1 trcl=77 imp:n=1
2 0 1 -9 imp:n=1
9 0 9 imp:n=0

1 so 2
9 so 100

",
    );
    assert!(
        m.diagnostics().iter().any(|d| d.message.contains("TR77")),
        "{:?}",
        m.diagnostics()
    );
    assert_eq!(
        m.classify_point([0.0, 0.0, 0.0]).status,
        Status::Unsupported
    );
    assert_eq!(cell_at(&m, [50.0, 0.0, 0.0]), (Status::Inside, Some(2)));
}

#[test]
fn points_beyond_the_world_box_are_still_exact() {
    let m = Model::from_text(
        "t
1 0 -1 imp:n=1
2 0 1 imp:n=0

1 so 2

",
        &Options::default().with_world_half_size(50.0),
    );
    assert_eq!(cell_at(&m, [1e4, 0.0, 0.0]), (Status::Inside, Some(2)));
    assert_eq!(cell_at(&m, [0.0, 0.0, 0.0]), (Status::Inside, Some(1)));
}

#[test]
fn fill_cycles_are_rejected() {
    let m = model(
        "t
1 0 -1 fill=5 imp:n=1
2 0 1 imp:n=0
3 0 -1 fill=6 u=5 imp:n=1
4 0 1 u=5 imp:n=1
5 0 -1 fill=5 u=6 imp:n=1
6 0 1 u=6 imp:n=1

1 so 2

",
    );
    assert!(
        m.diagnostics().iter().any(|d| d.message.contains("cycle")),
        "{:?}",
        m.diagnostics()
    );
    assert_eq!(
        m.classify_point([0.0, 0.0, 0.0]).status,
        Status::Unsupported
    );
}

#[test]
fn results_in_mcnp_terms() {
    let m = model(BASE);
    let l = m.classify_point([10.5, 0.0, 0.0]);
    assert_eq!(m.describe(&l), "inside 3 > 2");
    assert_eq!(
        m.describe(&m.classify_point([0.0, 0.0, 2.0])),
        "on surface (1, 4)"
    );
    assert_eq!(m.material(2).unwrap()[0].0.to_lowercase(), "26056.32c");
    assert!(m.material(7).is_none());
    // From x = -50 to 50 along the axis: cell 4, the sphere of cell 1, cell 4,
    // and cell 2 inside the filled cell 3 (the moved copy of surface 1 is
    // still MCNP surface 1).
    let segs = m
        .trace_line(&llevant::Line::between([-50.0, 0.0, 0.0], [50.0, 0.0, 0.0]))
        .unwrap();
    let ids: Vec<i64> = segs
        .iter()
        .filter_map(|s| s.surface_out.and_then(|x| m.surface_id(x)).map(|s| s.id))
        .collect();
    assert_eq!(ids, [1, 1, 2, 1, 1, 2]);
    assert_eq!(segs[0].t0, 0.0);
    assert!((segs.last().unwrap().t1 - 100.0).abs() < 1e-12);
}

#[test]
fn diagnostics_name_their_subject() {
    let m = model(
        "t
1 0 -1 imp:n=1
1 0 -1 imp:n=1
9 0 1 imp:n=0

1 so 2

",
    );
    assert_eq!(m.diagnostics().len(), 1, "{:?}", m.diagnostics());
    assert_eq!(m.diagnostics()[0].subject, llevant::Subject::Cell(1));
    // The line of the second definition.
    assert_eq!(m.diagnostics()[0].line, Some(3));
    assert!(m.diagnostics()[0].to_string().starts_with("line 3: "));
}

/// Cell parameters given on data cards are not read: say so, with the line.
#[test]
fn cell_parameter_data_cards_are_reported() {
    let m = model(
        "t
1 0 -1
9 0 1

1 so 2

imp:n 1 0
",
    );
    let d: Vec<_> = m
        .diagnostics()
        .iter()
        .map(|d| (d.subject, d.line))
        .collect();
    assert_eq!(
        d,
        [(llevant::Subject::Input, Some(7))],
        "{:?}",
        m.diagnostics()
    );
}

/// `IMP:N=0` marks a cell as outside the problem. Only the neutron entry
/// counts, and an entry's one value applies to every particle it lists (as
/// MCNP 6.2 reads them: `imp:p=0 imp:n=1` is neutron 1, photon 0).
#[test]
fn only_the_neutron_importance_puts_a_cell_outside() {
    let m = model(
        "t
1 0 -1 imp:p=0 imp:n=1
2 0 -1 imp:n,p=0
3 0 -1 IMP:N 1 imp:e=0
4 0 -1 imp:p,n=0
5 0 -1 imp:p=1 imp:n=0
6 like 1 but imp:n=0
7 like 5 but imp:p=1
9 0 1 imp:n=0

1 so 1

",
    );
    let zero: Vec<(i64, bool)> = m.cells().iter().map(|c| (c.id, c.imp_zero)).collect();
    assert_eq!(
        zero,
        [
            (1, false),
            (2, true),
            (3, false),
            (4, true),
            (5, true),
            (6, true),
            (7, true),
            (9, true)
        ]
    );
}

#[test]
fn beyond_the_problem_box_only_the_graveyard_counts() {
    // Cell 7 pokes out of the graveyard sphere: an overlap out there, which
    // the problem box rule leaves unreported.
    let m = model(
        "t
1 0 -1 imp:n=1
5 0 1 -9 -3 imp:n=1
6 0 1 -9 3 #7 imp:n=1
7 0 -7 3 1 imp:n=1
9 0 9 imp:n=0

1 so 2
3 px 0
7 s 50 0 0 100
9 so 100

",
    );
    assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
    let b = m.problem_box().expect("a problem box");
    for i in 0..3 {
        assert!(b.lo[i] > -101.0 && b.lo[i] < -99.0, "{b:?}");
        assert!(b.hi[i] < 101.0 && b.hi[i] > 99.0, "{b:?}");
    }
    // Inside the box (in a corner, beyond the sphere) the overlap is found;
    // beyond it, the graveyard.
    assert_eq!(cell_at(&m, [80.0, 80.0, 0.0]).0, Status::Overlap);
    assert_eq!(cell_at(&m, [140.0, 0.0, 0.0]), (Status::Inside, Some(9)));
    assert_eq!(cell_at(&m, [1e7, 0.0, 0.0]), (Status::Inside, Some(9)));
    let mut pts = Vec::new();
    for k in 0..400 {
        let a = k as f64 * 0.37;
        let r = 20.0 + k as f64 * 0.6;
        pts.push([r * a.cos(), r * a.sin(), 0.3 * r * (2.0 * a).sin()]);
    }
    let fast = m.classify(&pts, Depth::All);
    let slow = llevant::reference::classify(&m, &pts, Depth::All);
    for (i, p) in pts.iter().enumerate() {
        assert_eq!(
            fast.labels[fast.ids[i] as usize], slow.labels[slow.ids[i] as usize],
            "{p:?}"
        );
    }
    let lines: Vec<Line> = (0..60)
        .map(|k| {
            let a = k as f64 * 0.11;
            let o = [-1000.0, 30.0 * a.sin(), 20.0 * a.cos()];
            let e = [1000.0, -40.0 * a.cos(), 10.0 * a.sin()];
            Line::between(o, e)
        })
        .collect();
    let fast = m.trace(&lines, Depth::All).unwrap();
    let slow = llevant::reference::trace(&m, &lines, Depth::All).unwrap();
    for (i, l) in lines.iter().enumerate() {
        let a: Vec<_> = fast.segments(i).map(|k| fast.segment(k)).collect();
        let b: Vec<_> = slow.segments(i).map(|k| slow.segment(k)).collect();
        assert!(llevant::reference::traces_agree(&m, l, &a, &b), "line {i}");
        // Beyond the box: the graveyard alone, from the start of the line.
        assert_eq!(a[0].label.cell().map(|c| m.cell_id(c)), Some(9), "line {i}");
    }
}

#[test]
fn no_problem_box_without_a_bounded_graveyard() {
    let m = model(
        "t
1 0 -1 imp:n=1
2 0 1 imp:n=1

1 so 2

",
    );
    assert!(m.problem_box().is_none());
    // A graveyard that leaves a half-space bounds nothing either.
    let m = model(
        "t
1 0 -1 imp:n=1
2 0 1 -3 imp:n=1
3 0 3 imp:n=0

1 so 2
3 px 10

",
    );
    assert!(m.problem_box().is_none());
    assert_eq!(cell_at(&m, [-1e8, 0.0, 0.0]), (Status::Inside, Some(2)));
}
