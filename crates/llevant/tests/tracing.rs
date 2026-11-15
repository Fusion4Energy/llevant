//! Line tracing on small hand-written models.

use llevant::Depth;
use llevant::{Line, Model, Options, Segment, Status};

/// Where segments are sampled. Not their midpoints: in symmetric geometry a
/// segment's midpoint can lie on an internal surface of its cell (one the
/// cell holds on both sides), where a point is on-surface.
const SAMPLE_FRACTIONS: [f64; 3] = [0.382, 0.073, 0.917];

fn model(text: &str) -> Model {
    let m = Model::from_text(text, &Options::default());
    assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
    m
}

fn line(o: [f64; 3], d: [f64; 3], t0: f64, t1: f64) -> Line {
    Line {
        origin: o,
        dir: d,
        t_min: t0,
        t_max: t1,
    }
}

/// `(t0, t1, status, MCNP number of the last cell of the path)`.
fn summary(m: &Model, segs: &[Segment]) -> Vec<(f64, f64, Status, Option<i64>)> {
    segs.iter()
        .map(|s| {
            (
                s.t0,
                s.t1,
                s.label.status,
                s.label.path.last().map(|p| m.cells()[p.cell as usize].id),
            )
        })
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

const NESTED: &str = "\
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
fn segments_cover_the_line_with_exact_ends() {
    let m = model(NESTED);
    let segs = m
        .trace_line(&line([-200.0, 0.0, 0.0], [1.0, 0.0, 0.0], 0.0, 400.0))
        .unwrap();
    let s = summary(&m, &segs);
    let want: [(f64, f64, Status, Option<i64>); 9] = [
        (0.0, 100.0, Status::Inside, Some(9)),
        (100.0, 198.0, Status::Inside, Some(4)),
        (198.0, 202.0, Status::Inside, Some(1)),
        (202.0, 207.0, Status::Inside, Some(4)),
        // Cell 3 (the sphere at x = 10) holds universe 4: cell 2 inside
        // its sphere of radius 2, nothing around it.
        (207.0, 208.0, Status::Gap, Some(3)),
        (208.0, 212.0, Status::Inside, Some(2)),
        (212.0, 213.0, Status::Gap, Some(3)),
        (213.0, 300.0, Status::Inside, Some(4)),
        (300.0, 400.0, Status::Inside, Some(9)),
    ];
    assert_eq!(s.len(), want.len(), "{s:?}");
    for (a, b) in s.iter().zip(want) {
        assert!(
            close(a.0, b.0) && close(a.1, b.1) && a.2 == b.2 && a.3 == b.3,
            "{a:?} vs {b:?}"
        );
    }
    // Contiguous, starting and ending exactly at the line's range.
    assert_eq!(segs[0].t0, 0.0);
    assert_eq!(segs.last().unwrap().t1, 400.0);
    for w in segs.windows(2) {
        assert_eq!(w[0].t1, w[1].t0);
        assert_eq!(w[0].surface_out, w[1].surface_in);
    }
    assert_eq!(segs[0].surface_in, None);
    let id = |s: Option<u32>| s.map(|s| m.surface_id(s).unwrap().id);
    assert_eq!(id(segs[0].surface_out), Some(9));
    assert_eq!(id(segs[2].surface_in), Some(1));
    assert_eq!(id(segs[4].surface_in), Some(2));
    // Cell 2 is cell 1's sphere moved into universe 4: a moved copy of
    // surface 1.
    assert_eq!(id(segs[5].surface_in), Some(1));
    // The path goes through the filled cell.
    let path: Vec<i64> = segs[5]
        .label
        .path
        .iter()
        .map(|p| m.cells()[p.cell as usize].id)
        .collect();
    assert_eq!(path, vec![3, 2]);
}

#[test]
fn segment_labels_match_point_labels() {
    let m = model(NESTED);
    let lines: Vec<Line> = (0..200)
        .map(|i| {
            let a = i as f64 * 0.1;
            let d = [a.cos() * 0.8, a.sin() * 0.8, 0.6];
            line(
                [10.0 - 50.0 * d[0], 0.3 - 50.0 * d[1], -0.2 - 50.0 * d[2]],
                d,
                0.0,
                100.0,
            )
        })
        .collect();
    let tr = m.trace(&lines, Depth::All).unwrap();
    for (i, l) in lines.iter().enumerate() {
        for k in tr.segments(i) {
            let (a, b) = (tr.t0[k], tr.t1[k]);
            let label = &tr.labels[tr.ids[k] as usize];
            for f in SAMPLE_FRACTIONS {
                let t = a + f * (b - a);
                let p = [0, 1, 2].map(|j| l.origin[j] + t * l.dir[j]);
                assert_eq!(&m.classify_point(p), label, "line {i} t={t}");
            }
        }
    }
}

#[test]
fn gaps_overlaps_and_lines_on_surfaces() {
    let m = model(
        "t
1 0 -1 imp:n=1
2 0 -2 imp:n=1
3 0 -3 imp:n=1
9 0 9 imp:n=0

1 so 2
2 s 3 0 0 2
3 rpp 10 12 -1 1 -1 1
9 so 100

",
    );
    let segs = m
        .trace_line(&line([-3.0, 0.0, 0.0], [1.0, 0.0, 0.0], 0.0, 20.0))
        .unwrap();
    let s: Vec<(f64, f64, Status)> = segs.iter().map(|s| (s.t0, s.t1, s.label.status)).collect();
    let want = [
        (0.0, 1.0, Status::Gap),
        (1.0, 4.0, Status::Inside),
        (4.0, 5.0, Status::Overlap),
        (5.0, 8.0, Status::Inside),
        (8.0, 13.0, Status::Gap),
        (13.0, 15.0, Status::Inside),
        (15.0, 20.0, Status::Gap),
    ];
    assert_eq!(s.len(), want.len(), "{s:?}");
    for (a, b) in s.iter().zip(want) {
        assert!(
            close(a.0, b.0) && close(a.1, b.1) && a.2 == b.2,
            "{a:?} vs {b:?}"
        );
    }
    assert_eq!(segs[2].label.involved.len(), 2);
    // Along a face of the box: on its surface over the face's length.
    let segs = m
        .trace_line(&line([5.0, 1.0, 0.0], [1.0, 0.0, 0.0], 0.0, 10.0))
        .unwrap();
    let s: Vec<(f64, f64, Status)> = segs.iter().map(|s| (s.t0, s.t1, s.label.status)).collect();
    assert_eq!(s.len(), 3, "{s:?}");
    assert!(
        close(s[1].0, 5.0) && close(s[1].1, 7.0) && s[1].2 == Status::OnSurface,
        "{s:?}"
    );
}

/// A 1-cm square lattice in a 5 × 1 × 1 box; elements -2..=2 along x
/// alternate between universe 5 (a small sphere) and the lattice's own
/// material (universe 6). The box's faces are element faces.
const LATTICE: &str = "\
t
1 0 -1 fill=6 imp:n=1
9 0 1 imp:n=0
10 1 -1.0 -10 11 -12 13 lat=1 u=6 fill=-2:2 0:0 0:0 5 6 5 6 5 imp:n=1
20 2 -2.0 -20 u=5 imp:n=1
21 0 20 u=5 imp:n=1

1 rpp -2.5 2.5 -0.5 0.5 -0.5 0.5
10 px 0.5
11 px -0.5
12 py 0.5
13 py -0.5
20 so 0.25

m1 1001.32c 1
m2 26056.32c 1
";

#[test]
fn lattice_elements_along_a_line() {
    let m = model(LATTICE);
    let segs = m
        .trace_line(&line([-3.0, 0.1, 0.0], [1.0, 0.0, 0.0], 0.0, 6.0))
        .unwrap();
    type Row = (f64, f64, Option<i64>, Option<[i32; 3]>);
    let s: Vec<Row> = segs
        .iter()
        .map(|s| {
            (
                s.t0,
                s.t1,
                s.label.path.last().map(|p| m.cells()[p.cell as usize].id),
                s.label.path.get(1).and_then(|p| p.lattice),
            )
        })
        .collect();
    // In elements filled with universe 5 the line crosses the sphere
    // (chord ±0.229 about the element centre).
    let h = (0.25f64 * 0.25 - 0.01).sqrt();
    let mut want = vec![(0.0, 0.5, Some(9), None)];
    for i in -2..=2 {
        let c = 3.0 + i as f64;
        let ix = Some([i, 0, 0]);
        if i % 2 == 0 {
            want.push((c - 0.5, c - h, Some(21), ix));
            want.push((c - h, c + h, Some(20), ix));
            want.push((c + h, c + 0.5, Some(21), ix));
        } else {
            want.push((c - 0.5, c + 0.5, Some(10), ix));
        }
    }
    want.push((5.5, 6.0, Some(9), None));
    assert_eq!(s.len(), want.len(), "{s:?}");
    for (a, b) in s.iter().zip(&want) {
        assert!(
            close(a.0, b.0) && close(a.1, b.1) && a.2 == b.2 && a.3 == b.3,
            "{a:?} vs {b:?}"
        );
    }
}

#[test]
fn chord_lengths_give_the_volume_of_a_sphere() {
    let m = model(
        "t
1 1 -1.0 -1 imp:n=1
2 0 1 -9 imp:n=1
9 0 9 imp:n=0

1 s 0.3 -0.2 0.1 4
9 so 100

m1 1001.32c 1
",
    );
    let h = 0.05;
    let n = 200;
    let lines: Vec<Line> = (0..n * n)
        .map(|k| {
            let (i, j) = ((k / n) as f64, (k % n) as f64);
            line(
                [-6.0, -5.0 + (i + 0.37) * h, -5.0 + (j + 0.61) * h],
                [1.0, 0.0, 0.0],
                0.0,
                12.0,
            )
        })
        .collect();
    let tr = m.trace(&lines, Depth::All).unwrap();
    let c1 = m.cell_index(1).unwrap();
    let mut len = 0.0;
    for k in 0..tr.t0.len() {
        if tr.labels[tr.ids[k] as usize].cell() == Some(c1) {
            len += tr.t1[k] - tr.t0[k];
        }
    }
    let vol = len * h * h;
    let exact = 4.0 / 3.0 * std::f64::consts::PI * 64.0;
    assert!((vol / exact - 1.0).abs() < 2e-3, "{vol} vs {exact}");
}

/// Where the envelope's faces coincide with element faces, rounding must
/// not leave slivers of the elements beyond them (outside the fill array:
/// gaps).
#[test]
fn lattice_faces_on_the_envelope_leave_no_slivers() {
    let m = model(LATTICE);
    let lines: Vec<Line> = (0..2000)
        .map(|i| {
            let a = i as f64 * 0.37;
            let b = i as f64 * 0.11;
            let d = [a.cos() * b.cos(), a.sin() * b.cos(), b.sin()];
            let o = [0.3 * (i as f64).sin(), 0.2 * (i as f64 * 1.3).cos(), 0.1];
            line(o, d, -10.0, 10.0)
        })
        .collect();
    let tr = m.trace(&lines, Depth::All).unwrap();
    for k in 0..tr.t0.len() {
        let l = &tr.labels[tr.ids[k] as usize];
        assert_eq!(
            l.status,
            Status::Inside,
            "{:?} [{}, {}]",
            l,
            tr.t0[k],
            tr.t1[k]
        );
    }
}
