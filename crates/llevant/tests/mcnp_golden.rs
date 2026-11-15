//! Points whose cell MCNP itself reported (PTRAC source events), recorded by
//! `mcnp-oracle check --golden`. Regenerate them with a local MCNP when the
//! zoo models change.

use llevant::Depth;
use std::path::PathBuf;

use llevant::{GridMode, Line, Model, Options, Status};

/// Where segments are sampled. Not their midpoints: in symmetric geometry a
/// segment's midpoint can lie on an internal surface of its cell (one the
/// cell holds on both sides), where a point is on-surface.
const SAMPLE_FRACTIONS: [f64; 3] = [0.382, 0.073, 0.917];

fn models_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/mcnp-oracle/models")
}

/// Checked with the default search structures and with an octree in every
/// universe, for points and for lines through the points.
fn check(name: &str) {
    for grid in [GridMode::Auto, GridMode::Always] {
        let opts = Options::default().with_grid(grid);
        let model = Model::load(models_dir().join(format!("{name}.i")), &opts, None).unwrap();
        assert!(model.diagnostics().is_empty(), "{:?}", model.diagnostics());
        let rows = golden(name);
        check_points(&model, &rows, grid);
        check_lines(&model, &rows, grid);
    }
}

fn golden(name: &str) -> Vec<([f64; 3], i64)> {
    let golden = std::fs::read_to_string(models_dir().join(format!("{name}.golden"))).unwrap();
    golden
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| {
            let v: Vec<&str> = l.split_whitespace().collect();
            (
                [v[0], v[1], v[2]].map(|x| x.parse().unwrap()),
                v[3].parse().unwrap(),
            )
        })
        .collect()
}

fn check_points(model: &Model, rows: &[([f64; 3], i64)], grid: GridMode) {
    assert!(rows.len() >= 1000);
    let points: Vec<[f64; 3]> = rows.iter().map(|r| r.0).collect();
    let res = model.classify(&points, Depth::All);
    let mut wrong = Vec::new();
    for (i, &id) in res.ids.iter().enumerate() {
        let label = &res.labels[id as usize];
        let got = label.cell().map(|c| model.cells()[c as usize].id);
        if label.status != Status::Inside || got != Some(rows[i].1) {
            wrong.push((rows[i], label.status, got));
        }
    }
    assert!(
        wrong.is_empty(),
        "{grid:?}: {} of {} wrong, e.g. {:?}",
        wrong.len(),
        rows.len(),
        &wrong[..wrong.len().min(5)]
    );
}

#[test]
fn surface_zoo_matches_mcnp() {
    check("surfaces");
}

#[test]
fn universe_zoo_matches_mcnp() {
    check("universes");
}

#[test]
fn periodic_surfaces_match_mcnp() {
    check("periodic");
}

/// Lines from each golden point towards another: the segment at `t = 0`
/// must be inside the cell MCNP reported, tracing must equal brute-force
/// tracing, and points along every segment must classify to its label.
fn check_lines(model: &Model, rows: &[([f64; 3], i64)], grid: GridMode) {
    let n = rows.len().min(1500);
    let lines: Vec<Line> = (0..n)
        .map(|i| {
            let (p, q) = (rows[i].0, rows[(i * 7 + 13) % rows.len()].0);
            let v = [0, 1, 2].map(|k| q[k] - p[k]);
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            Line {
                origin: p,
                dir: v.map(|x| x / len),
                t_min: -0.25 * len,
                t_max: len,
            }
        })
        .collect();
    let fast = model.trace(&lines, Depth::All).unwrap();
    let slow = llevant::reference::trace(model, &lines, Depth::All).unwrap();
    let mut pts = Vec::new();
    let mut want = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let a: Vec<_> = fast.segments(i).map(|k| fast.segment(k)).collect();
        let b: Vec<_> = slow.segments(i).map(|k| slow.segment(k)).collect();
        assert!(
            llevant::reference::traces_agree(model, l, &a, &b),
            "{grid:?} line {i}"
        );
        for k in fast.segments(i) {
            let (a, b) = (fast.t0[k], fast.t1[k]);
            if a <= 0.0 && 0.0 < b {
                let label = &fast.labels[fast.ids[k] as usize];
                let got = label.cell().map(|c| model.cells()[c as usize].id);
                assert_eq!(got, Some(rows[i].1), "{grid:?} line {i} {label:?}");
            }
            for f in SAMPLE_FRACTIONS {
                let t = a + f * (b - a);
                if (t - a).min(b - t) > 1e-6 {
                    pts.push([0, 1, 2].map(|j| l.origin[j] + t * l.dir[j]));
                    want.push(fast.ids[k]);
                }
            }
        }
    }
    let got = model.classify(&pts, Depth::All);
    for i in 0..pts.len() {
        assert_eq!(
            got.labels[got.ids[i] as usize], fast.labels[want[i] as usize],
            "{grid:?} point {:?}",
            pts[i]
        );
    }
}
