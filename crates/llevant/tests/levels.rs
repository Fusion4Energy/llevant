//! Universe levels and ghost cells on random models: tracing and
//! classifying down to a level agree with brute force and with the full
//! labels cut there; first hits at a level, with ghosts, are what tracing
//! to that level says, with or without a skip.

use llevant::Depth;
use llevant::{
    GridMode, Label, Model, Options, Status, Tracing, Visibility, NO_CELL, NO_HIT, NO_SURFACE,
};
use proptest::prelude::*;

mod common;
use common::{lines, points, random_model_with, Rng};

/// `full` cut to `level + 1` steps, when the cut is unambiguous: `None`
/// for a label of `level + 1` steps that is not inside (a gap or problem
/// either in its last cell's universe, which the cut hides, or in a
/// lattice element at the level, which it keeps).
fn cut(full: &Label, level: usize) -> Option<Label> {
    if full.path.len() > level + 1 {
        return Some(Label {
            status: Status::Inside,
            path: full.path[..level + 1].to_vec(),
            involved: Vec::new(),
        });
    }
    if full.path.len() == level + 1 && full.status != Status::Inside {
        return None;
    }
    Some(full.clone())
}

fn check_points(text: &str, seed: u64, grid: GridMode) -> Result<(), TestCaseError> {
    let model = Model::from_text(text, &Options::default().with_grid(grid));
    let pts = points(seed, 3000);
    let full = model.classify(&pts, Depth::All);
    for level in 0..4 {
        let fast = model.classify(&pts, Depth::Level(level));
        let slow = llevant::reference::classify(&model, &pts, Depth::Level(level));
        for i in 0..pts.len() {
            let (a, b) = (
                &fast.labels[fast.ids[i] as usize],
                &slow.labels[slow.ids[i] as usize],
            );
            prop_assert_eq!(a, b, "level {} point {:?}\n{}", level, pts[i], text);
            prop_assert!(a.path.len() <= level + 1);
            if let Some(want) = cut(&full.labels[full.ids[i] as usize], level) {
                prop_assert_eq!(a, &want, "level {} point {:?}\n{}", level, pts[i], text);
            }
        }
    }
    Ok(())
}

fn check_tracing(text: &str, seed: u64, grid: GridMode) -> Result<(), TestCaseError> {
    let model = Model::from_text(text, &Options::default().with_grid(grid));
    let ls = lines(seed, 200);
    for level in 0..4 {
        let fast = model.trace(&ls, Depth::Level(level)).unwrap();
        let slow = llevant::reference::trace(&model, &ls, Depth::Level(level)).unwrap();
        for (i, l) in ls.iter().enumerate() {
            let a: Vec<_> = fast.segments(i).map(|k| fast.segment(k)).collect();
            let b: Vec<_> = slow.segments(i).map(|k| slow.segment(k)).collect();
            prop_assert!(
                llevant::reference::traces_agree(&model, l, &a, &b),
                "level {} line {}: {:?} vs {:?}\n{}",
                level,
                i,
                a,
                b,
                text
            );
        }
        // Points well inside each segment classify to its label at the
        // same level.
        let mut pts = Vec::new();
        let mut want = Vec::new();
        for (i, l) in ls.iter().enumerate() {
            for k in fast.segments(i) {
                let (a, b) = (fast.t0[k], fast.t1[k]);
                let t = a + 0.382 * (b - a);
                if (t - a).min(b - t) < 1e-6 {
                    continue;
                }
                pts.push([0, 1, 2].map(|j| l.origin[j] + t * l.dir[j]));
                want.push(fast.ids[k]);
            }
        }
        let got = model.classify(&pts, Depth::Level(level));
        for i in 0..pts.len() {
            prop_assert_eq!(
                &got.labels[got.ids[i] as usize],
                &fast.labels[want[i] as usize],
                "level {} point {:?}\n{}",
                level,
                pts[i],
                text
            );
        }
    }
    Ok(())
}

/// A visibility drawn at random: cells, gaps, problems, a level or none,
/// ghosts or none, veils or none, and the cells whose problems show (all
/// or some).
fn random_visibility(model: &Model, r: &mut Rng, quiet: bool) -> Visibility {
    let n = model.cells().len();
    Visibility {
        cells: (0..n).map(|_| r.int(4) > 0).collect(),
        gaps: !quiet && r.int(2) == 0,
        problems: !quiet && r.int(2) == 0,
        level: match r.int(3) {
            0 => None,
            _ => Some(r.int(3)),
        },
        ghosts: if r.int(3) == 0 {
            Vec::new()
        } else {
            (0..n).map(|_| r.int(3) == 0).collect()
        },
        veil: !quiet && r.int(2) == 0,
        problem_cells: if r.int(2) == 0 {
            Vec::new()
        } else {
            (0..n).map(|_| r.int(2) == 0).collect()
        },
    }
}

/// Whether segment `k` of `tr` is a veil under `vis`: a gap or overlap
/// shown, seen through.
fn veiled(tr: &Tracing, k: usize, vis: &Visibility) -> bool {
    let lab = &tr.labels[tr.ids[k] as usize];
    let status = match lab.status {
        Status::Gap => vis.gaps,
        Status::Overlap => vis.problems,
        _ => false,
    };
    vis.veil
        && status
        && vis.owns_problem(lab)
        && lab.path.iter().all(|s| vis.cells[s.cell as usize])
}

/// Whether segment `k` of `tr` shows under `vis`.
fn shown(tr: &Tracing, k: usize, vis: &Visibility) -> bool {
    let lab = &tr.labels[tr.ids[k] as usize];
    let status = match lab.status {
        Status::Inside => {
            vis.level.is_none_or(|l| lab.path.len() == l + 1)
                && !vis.is_ghost(lab.path.last().unwrap().cell)
        }
        Status::Gap => vis.gaps,
        Status::OnSurface => false,
        _ => vis.problems,
    };
    status
        && vis.owns_problem(lab)
        && lab.path.iter().all(|s| vis.cells[s.cell as usize])
        && !veiled(tr, k, vis)
}

/// The ghost boundaries on line `i` of `tr` up to `t_hit`: the ends on a
/// surface of each run of segments that hold the same ghost cell reached
/// through visible cells, within the model's tolerance `tol` counted once,
/// the first named as the engine does (a hit can be a sliver longer than
/// that, with a ghost boundary just after it).
fn ghost_crossings(
    tr: &Tracing,
    i: usize,
    vis: &Visibility,
    t_hit: f64,
    tol: f64,
) -> Vec<(f64, u32)> {
    let mut ends = Vec::new();
    let segs: Vec<usize> = tr.segments(i).collect();
    // Ghost cells on a segment's path, under visible cells only.
    let ghosts = |k: usize| -> Vec<u32> {
        let lab = &tr.labels[tr.ids[k] as usize];
        let mut out = Vec::new();
        for s in &lab.path {
            if !vis.cells[s.cell as usize] {
                break;
            }
            if vis.is_ghost(s.cell) {
                out.push(s.cell);
            }
        }
        out
    };
    let per: Vec<Vec<u32>> = segs.iter().map(|&k| ghosts(k)).collect();
    for (j, &k) in segs.iter().enumerate() {
        for &g in &per[j] {
            let enters = j == 0 || !per[j - 1].contains(&g);
            if enters && tr.surface_in[k] != NO_SURFACE && tr.t0[k] <= t_hit + tol {
                ends.push((tr.t0[k], 0, g));
            }
            let leaves = j + 1 == segs.len() || !per[j + 1].contains(&g);
            if leaves && tr.surface_out[k] != NO_SURFACE && tr.t1[k] <= t_hit + tol {
                ends.push((tr.t1[k], 1, g));
            }
        }
    }
    // At the same place: a ghost cell entered, then one left (as the
    // engine and the GPU name the first).
    ends.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut out: Vec<(f64, u32)> = Vec::new();
    let mut last = f64::NEG_INFINITY;
    for (t, _, g) in ends {
        if t - last > tol {
            out.push((t, g));
        }
        last = t;
    }
    out
}

fn check_hits(text: &str, seed: u64, grid: GridMode) -> Result<(), TestCaseError> {
    let model = Model::from_text(text, &Options::default().with_grid(grid));
    let mut r = Rng(seed ^ 0x9e3779b97f4a7c15 | 1);
    let ls = lines(seed, 200);
    for _ in 0..3 {
        let vis = random_visibility(&model, &mut r, false);
        let hits = model.first_hits(&ls, &vis, None).unwrap();
        prop_assert_eq!(
            hits.ghosts.len(),
            if vis.has_ghosts() { ls.len() } else { 0 }
        );
        let tr = model.trace(&ls, Depth::from(vis.level)).unwrap();
        for i in 0..ls.len() {
            let first = tr.segments(i).find(|&k| shown(&tr, k, &vis));
            let t_hit = match first {
                None => {
                    prop_assert_eq!(hits.ids[i], NO_HIT, "line {} {:?}\n{}", i, vis.level, text);
                    f64::INFINITY
                }
                Some(k) => {
                    // Long lines are searched in windows: a root found in
                    // one may differ from the whole line's in the last bits.
                    prop_assert!(
                        (hits.t[i] - tr.t0[k]).abs() <= 1e-9,
                        "line {} {:?}: {} vs {}\n{}",
                        i,
                        vis.level,
                        hits.t[i],
                        tr.t0[k],
                        text
                    );
                    prop_assert_eq!(
                        &hits.labels[hits.ids[i] as usize],
                        &tr.labels[tr.ids[k] as usize]
                    );
                    tr.t0[k]
                }
            };
            if vis.veil {
                // The veils before the hit.
                let mut want = llevant::Veils::default();
                for k in tr.segments(i) {
                    if tr.t0[k] < t_hit && veiled(&tr, k, &vis) {
                        let st = tr.labels[tr.ids[k] as usize].status;
                        want.gap |= st == Status::Gap;
                        want.overlap |= st == Status::Overlap;
                    }
                }
                prop_assert_eq!(hits.veils[i], want, "line {}\n{}", i, text);
            }
            if !vis.has_ghosts() {
                continue;
            }
            let g = hits.ghosts[i];
            let want = ghost_crossings(&tr, i, &vis, t_hit, model.tolerance());
            prop_assert_eq!(
                g.count as usize,
                want.len(),
                "line {}: {:?} vs {:?}\n{}",
                i,
                g,
                want,
                text
            );
            match want.first() {
                None => prop_assert_eq!(g.cell, NO_CELL),
                Some(&(t, c)) => {
                    prop_assert!((g.t - t).abs() <= 1e-9, "line {}: {:?} vs {:?}", i, g, want);
                    prop_assert_eq!(g.cell, c);
                    let n = g.normal;
                    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    prop_assert!((len - 1.0).abs() < 1e-9, "{:?}", g);
                    let d = ls[i].dir;
                    prop_assert!(n[0] * d[0] + n[1] * d[1] + n[2] * d[2] <= 0.0);
                }
            }
        }
    }
    Ok(())
}

/// With gaps and problems hidden, a skip gives the same hits and ghosts.
fn check_skipped_hits(text: &str, seed: u64, grid: GridMode) -> Result<(), TestCaseError> {
    let model = Model::from_text(text, &Options::default().with_grid(grid));
    let mut r = Rng(seed ^ 0x2545f4914f6cdd1d | 1);
    let ls = lines(seed, 200);
    for _ in 0..3 {
        let vis = random_visibility(&model, &mut r, true);
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
            if all.ids[i] != NO_HIT {
                prop_assert!((all.t[i] - some.t[i]).abs() <= 1e-6, "line {}\n{}", i, text);
                prop_assert_eq!(
                    &all.labels[all.ids[i] as usize],
                    &some.labels[some.ids[i] as usize],
                    "line {}",
                    i
                );
            }
            if vis.has_ghosts() {
                let (a, b) = (all.ghosts[i], some.ghosts[i]);
                prop_assert_eq!(a.count, b.count, "line {}: {:?} vs {:?}\n{}", i, a, b, text);
                prop_assert_eq!(a.cell, b.cell);
                prop_assert!(a.count == 0 || (a.t - b.t).abs() <= 1e-6);
            }
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(common::proptest_config(32))]

    #[test]
    fn points_to_a_level(seed in any::<u64>()) {
        check_points(&random_model_with(seed, true), seed, GridMode::Auto)?;
        check_points(&random_model_with(seed, true), seed, GridMode::Always)?;
    }

    #[test]
    fn tracing_to_a_level(seed in any::<u64>()) {
        check_tracing(&random_model_with(seed, true), seed, GridMode::Auto)?;
        check_tracing(&random_model_with(seed, true), seed, GridMode::Always)?;
    }

    #[test]
    fn first_hits_at_a_level_with_ghosts(seed in any::<u64>()) {
        check_hits(&random_model_with(seed, true), seed, GridMode::Auto)?;
        check_hits(&random_model_with(seed, true), seed, GridMode::Always)?;
    }

    #[test]
    fn skipped_first_hits_at_a_level_with_ghosts(seed in any::<u64>()) {
        check_skipped_hits(&random_model_with(seed, true), seed, GridMode::Auto)?;
        check_skipped_hits(&random_model_with(seed, true), seed, GridMode::Always)?;
    }
}

/// The probe of MCNP's plotter (`docs/mcnp-behaviour.md`):
/// `LEVEL 0` shows cells 1 to 4 (the filled 1 and 3 as cells), `LEVEL 1`
/// only 11, 12, 13 and the lattice cell 31 (with its elements), `LEVEL 2`
/// only 21, 22, 41 and 42.
#[test]
fn levels_shown_as_mcnp_plots_them() {
    let model = Model::from_text(PROBE, &Options::default());
    let rows: Vec<llevant::Line> = (0..200)
        .map(|j| {
            let y = -11.0 + 22.0 * (j as f64 + 0.5) / 200.0;
            llevant::Line::new([-6.0, y, 0.0], [1.0, 0.0, 0.0], 0.0, 32.0)
        })
        .collect();
    let want: [&[i64]; 3] = [&[1, 2, 3, 4], &[11, 12, 13, 31], &[21, 22, 41, 42]];
    for (level, want) in want.iter().enumerate() {
        let tr = model.trace(&rows, Depth::Level(level)).unwrap();
        let mut seen: Vec<i64> = tr
            .labels
            .iter()
            .filter(|l| l.status == Status::Inside && l.path.len() == level + 1)
            .map(|l| model.cells()[l.path.last().unwrap().cell as usize].id)
            .collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(&seen, want, "level {level}");
    }
    // Elements of the lattice cell show at its level: labels differ by element.
    let tr = model.trace(&rows, Depth::Level(1)).unwrap();
    let elements = tr
        .labels
        .iter()
        .filter(|l| l.path.len() == 2 && l.path[1].lattice.is_some())
        .count();
    assert!(elements > 20, "{elements}");
}

/// The model plotted with MCNP for `levels_shown_as_mcnp_plots_them`.
const PROBE: &str = "\
level probe
1 0 -1 fill=1 imp:n=1
2 1 -1.0 1 -2 imp:n=1
3 0 -3 fill=3 imp:n=1
4 0 2 3 -9 imp:n=1
9 0 9 imp:n=0
11 2 -2.0 -11 u=1 imp:n=1
12 0 -12 fill=2 u=1 imp:n=1
13 0 11 12 u=1 imp:n=1
21 1 -1.0 -21 u=2 imp:n=1
22 3 -3.0 21 u=2 imp:n=1
31 0 -31 lat=1 fill=4 u=3 imp:n=1
41 2 -2.0 -41 u=4 imp:n=1
42 0 41 u=4 imp:n=1

1 rpp -10 10 -10 10 -10 10
2 rpp -12 12 -12 12 -12 12
3 rpp 15 25 -5 5 -5 5
9 so 100
11 s 0 5 0 3
12 s 0 -4 0 4
21 s 0 -4 0 2
31 rpp -1 1 -1 1 -1 1
41 so 0.8

m1 1001 1
m2 8016 1
m3 26056 1
";

/// The random visibilities above do meet ghosts and levels: rays cross
/// several ghost boundaries, and hit cells at each level.
#[test]
fn random_visibilities_exercise_ghosts_and_levels() {
    let (mut crossed, mut several, mut at_level) = (0, 0, [0; 3]);
    for seed in 1..20u64 {
        let model = Model::from_text(&random_model_with(seed, true), &Options::default());
        let mut r = Rng(seed ^ 0x9e3779b97f4a7c15 | 1);
        let ls = lines(seed, 200);
        for _ in 0..3 {
            let vis = random_visibility(&model, &mut r, false);
            let hits = model.first_hits(&ls, &vis, None).unwrap();
            for g in &hits.ghosts {
                crossed += (g.count > 0) as usize;
                several += (g.count > 2) as usize;
            }
            if let Some(l) = vis.level {
                at_level[l] += hits.ids.iter().filter(|&&id| id != NO_HIT).count();
            }
        }
    }
    assert!(crossed > 500 && several > 100, "{crossed} {several}");
    assert!(at_level.iter().all(|&n| n > 100), "{at_level:?}");
}
