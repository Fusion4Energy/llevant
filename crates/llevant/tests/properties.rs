//! Property tests on random models: the fast path (bounding boxes, BVH, node
//! boxes) must give exactly the labels of the brute-force reference, and
//! results must not depend on the number of threads.

use llevant::Depth;
use llevant::{GridMode, Model, Options, Segment, Status, Visibility, NO_HIT, NO_SURFACE};
use proptest::prelude::*;

mod common;
use common::{lines, points, random_model, random_model_with, Rng};

/// Where segments are sampled. Not their midpoints: in symmetric geometry a
/// segment's midpoint can lie on an internal surface of its cell (one the
/// cell holds on both sides), where a point is on-surface.
const SAMPLE_FRACTIONS: [f64; 3] = [0.382, 0.073, 0.917];

proptest! {
    #![proptest_config(common::proptest_config(48))]

    #[test]
    fn fast_path_equals_brute_force(seed in any::<u64>()) {
        let text = random_model(seed);
        let model = Model::from_text(&text, &Options::default());
        prop_assert!(model.diagnostics().is_empty(), "{:?}\n{}", model.diagnostics(), text);
        let pts = points(seed, 4000);
        let fast = model.classify(&pts, Depth::All);
        let slow = llevant::reference::classify(&model, &pts, Depth::All);
        for i in 0..pts.len() {
            let (a, b) = (&fast.labels[fast.ids[i] as usize], &slow.labels[slow.ids[i] as usize]);
            prop_assert_eq!(a, b, "point {:?}\n{}", pts[i], text);
        }
    }
}

proptest! {
    #![proptest_config(common::proptest_config(48))]

    /// The same with an octree in every universe, small enough that its
    /// leaves cut through the cells.
    #[test]
    fn octree_path_equals_brute_force(seed in any::<u64>()) {
        let text = random_model(seed);
        let opts = Options::default().with_grid(GridMode::Always);
        let model = Model::from_text(&text, &opts);
        prop_assert!(model.stats().grid_units > 0);
        prop_assert!(model.stats().grid_nodes > 8, "{:?}", model.stats());
        let pts = points(seed, 4000);
        let fast = model.classify(&pts, Depth::All);
        let slow = llevant::reference::classify(&model, &pts, Depth::All);
        for i in 0..pts.len() {
            let (a, b) = (&fast.labels[fast.ids[i] as usize], &slow.labels[slow.ids[i] as usize]);
            prop_assert_eq!(a, b, "point {:?}\n{}", pts[i], text);
        }
    }
}

#[test]
fn results_do_not_depend_on_thread_count() {
    let text = random_model(12345);
    let model = Model::from_text(&text, &Options::default());
    let pts = points(1, 50_000);
    let many = model.classify(&pts, Depth::All);
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
        .install(|| model.classify(&pts, Depth::All));
    assert_eq!(many.ids, one.ids);
    assert_eq!(many.labels, one.labels);
}

/// Fast tracing must give the segments of brute-force tracing (as
/// `reference::traces_agree` says), and points inside each segment must classify
/// to its label.
fn check_tracing(text: &str, seed: u64, grid: GridMode) -> Result<(), TestCaseError> {
    let opts = Options::default().with_grid(grid);
    let model = Model::from_text(text, &opts);
    let ls = lines(seed, 300);
    let fast = model.trace(&ls, Depth::All).unwrap();
    let slow = llevant::reference::trace(&model, &ls, Depth::All).unwrap();
    for (i, l) in ls.iter().enumerate() {
        let a: Vec<_> = fast.segments(i).map(|k| fast.segment(k)).collect();
        let b: Vec<_> = slow.segments(i).map(|k| slow.segment(k)).collect();
        prop_assert!(
            llevant::reference::traces_agree(&model, l, &a, &b),
            "line {}: {:?} vs {:?}\n{}",
            i,
            a,
            b,
            text
        );
    }
    let mut pts = Vec::new();
    let mut want = Vec::new();
    for (i, l) in ls.iter().enumerate() {
        let r = fast.segments(i);
        prop_assert_eq!(fast.t0[r.start], l.t_min);
        prop_assert_eq!(fast.t1[r.end - 1], l.t_max);
        for k in r {
            let (a, b) = (fast.t0[k], fast.t1[k]);
            if k + 1 < fast.offsets[i + 1] {
                prop_assert_eq!(b, fast.t0[k + 1]);
            }
            for f in SAMPLE_FRACTIONS {
                let t = a + f * (b - a);
                // Within the tolerance of a segment end the point may be
                // on a surface.
                if (t - a).min(b - t) < 1e-6 {
                    continue;
                }
                pts.push([0, 1, 2].map(|j| l.origin[j] + t * l.dir[j]));
                want.push(fast.ids[k]);
            }
        }
    }
    if std::env::var("TRACE_STATS").is_ok() {
        let mut h = std::collections::BTreeMap::new();
        for k in 0..fast.t0.len() {
            let l = &fast.labels[fast.ids[k] as usize];
            *h.entry((
                format!("{:?}", l.status),
                l.path.len(),
                l.path.iter().any(|s| s.lattice.is_some()),
            ))
            .or_insert(0) += 1;
        }
        eprintln!("{} segs {:?}", fast.t0.len(), h);
        let mut ov = std::collections::BTreeMap::new();
        for k in 0..fast.t0.len() {
            let l = &fast.labels[fast.ids[k] as usize];
            if l.status == llevant::Status::Overlap {
                let ids: Vec<i64> = l
                    .involved
                    .iter()
                    .map(|&c| model.cells()[c as usize].id)
                    .collect();
                *ov.entry(ids).or_insert(0) += 1;
            }
        }
        eprintln!("overlaps {:?}", ov);
    }
    let got = model.classify(&pts, Depth::All);
    for i in 0..pts.len() {
        prop_assert_eq!(
            &got.labels[got.ids[i] as usize],
            &fast.labels[want[i] as usize],
            "point {:?}\n{}",
            pts[i],
            text
        );
    }
    Ok(())
}

proptest! {
    #![proptest_config(common::proptest_config(48))]

    #[test]
    fn tracing_equals_brute_force_and_points(seed in any::<u64>()) {
        check_tracing(&random_model_with(seed, true), seed, GridMode::Auto)?;
    }

    #[test]
    fn tracing_with_octrees_equals_brute_force_and_points(seed in any::<u64>()) {
        check_tracing(&random_model_with(seed, true), seed, GridMode::Always)?;
    }
}

#[test]
fn tracing_does_not_depend_on_thread_count() {
    let text = random_model_with(777, true);
    let model = Model::from_text(&text, &Options::default());
    let ls = lines(5, 5000);
    let many = model.trace(&ls, Depth::All).unwrap();
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
        .install(|| model.trace(&ls, Depth::All).unwrap());
    assert_eq!(many.offsets, one.offsets);
    assert_eq!(many.t0, one.t0);
    assert_eq!(many.t1, one.t1);
    assert_eq!(many.ids, one.ids);
    assert_eq!(many.surface_in, one.surface_in);
    assert_eq!(many.labels, one.labels);
}

/// `first_hit` must give the start of the first visible segment `trace`
/// gives, with a visibility mask drawn at random.
fn check_hits(text: &str, seed: u64, grid: GridMode) -> Result<(), TestCaseError> {
    let opts = Options::default().with_grid(grid);
    let model = Model::from_text(text, &opts);
    let mut r = Rng(seed ^ 0x5851f42d4c957f2d | 1);
    let vis = Visibility {
        cells: (0..model.cells().len()).map(|_| r.int(4) > 0).collect(),
        gaps: r.int(2) == 0,
        problems: r.int(2) == 0,
        level: None,
        ghosts: Vec::new(),
        veil: false,
        problem_cells: Vec::new(),
    };
    let ls = lines(seed, 300);
    let hits = model.first_hits(&ls, &vis, None).unwrap();
    let tr = model.trace(&ls, Depth::All).unwrap();
    for (i, l) in ls.iter().enumerate() {
        let first = tr.segments(i).find(|&k| {
            let lab = &tr.labels[tr.ids[k] as usize];
            let shown = match lab.status {
                Status::Inside => true,
                Status::Gap => vis.gaps,
                Status::OnSurface => false,
                _ => vis.problems,
            };
            shown && lab.path.iter().all(|s| vis.cells[s.cell as usize])
        });
        match first {
            None => prop_assert_eq!(hits.ids[i], NO_HIT, "line {}\n{}", i, text),
            Some(k) => {
                // The hit is the start of that segment as two tracings agree
                // (`reference::segments_agree`): the same label, `t` within the
                // tolerance, the same surface or another as close to it. A
                // ray traced in windows from its start can round a crossing
                // one ulp away from the whole line's.
                let seg = tr.segment(k);
                let hit = Segment {
                    t0: hits.t[i],
                    label: hits.labels[hits.ids[i] as usize].clone(),
                    surface_in: (hits.surface[i] != NO_SURFACE).then_some(hits.surface[i]),
                    ..seg.clone()
                };
                prop_assert!(
                    llevant::reference::segments_agree(&model, l, &hit, &seg),
                    "line {}: hit {:?}, segment {:?}\n{}",
                    i,
                    hit,
                    seg,
                    text
                );
                let n = hits.normal[i];
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if hits.surface[i] == NO_SURFACE {
                    prop_assert_eq!(len, 0.0);
                } else if len > 0.0 {
                    prop_assert!((len - 1.0).abs() < 1e-12);
                    let facing = n[0] * l.dir[0] + n[1] * l.dir[1] + n[2] * l.dir[2];
                    prop_assert!(facing <= 0.0);
                }
            }
        }
    }
    Ok(())
}

/// With gaps and problems hidden, searching only where `Model::skip` says
/// something visible may be gives the first hits of the whole search (as
/// `reference::traces_agree` has tracings agree), with few cells visible (so
/// that most is skipped) or many.
fn check_skipped_hits(text: &str, seed: u64, grid: GridMode) -> Result<(), TestCaseError> {
    let opts = Options::default().with_grid(grid);
    let model = Model::from_text(text, &opts);
    let mut r = Rng(seed ^ 0x2545f4914f6cdd1d | 1);
    let ls = lines(seed, 300);
    for keep in [1, 3] {
        let vis = Visibility {
            cells: (0..model.cells().len()).map(|_| r.int(4) < keep).collect(),
            gaps: false,
            problems: false,
            level: None,
            ghosts: Vec::new(),
            veil: false,
            problem_cells: Vec::new(),
        };
        let skip = model.skip(&vis).unwrap();
        let all = model.first_hits(&ls, &vis, None).unwrap();
        let some = model.first_hits(&ls, &vis, Some(&skip)).unwrap();
        for i in 0..ls.len() {
            prop_assert_eq!(
                all.ids[i] == NO_HIT,
                some.ids[i] == NO_HIT,
                "line {}\n{}",
                i,
                text
            );
            if all.ids[i] == NO_HIT {
                continue;
            }
            // As `reference::traces_agree`: a crossing found from another start
            // may round differently, and of surfaces within the tolerance
            // another may be named.
            prop_assert!(
                (all.t[i] - some.t[i]).abs() <= 1e-6,
                "line {}: {} vs {}
{}",
                i,
                all.t[i],
                some.t[i],
                text
            );
            prop_assert_eq!(
                &all.labels[all.ids[i] as usize],
                &some.labels[some.ids[i] as usize],
                "line {}",
                i
            );
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(common::proptest_config(48))]

    #[test]
    fn skipped_first_hits_are_the_first_hits(seed in any::<u64>()) {
        check_skipped_hits(&random_model_with(seed, true), seed, GridMode::Auto)?;
        check_skipped_hits(&random_model_with(seed, true), seed, GridMode::Always)?;
    }

    #[test]
    fn first_hit_is_the_first_visible_segment(seed in any::<u64>()) {
        check_hits(&random_model_with(seed, true), seed, GridMode::Auto)?;
        check_hits(&random_model_with(seed, true), seed, GridMode::Always)?;
    }
}
