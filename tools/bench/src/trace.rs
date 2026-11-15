//! Trace random lines through a model: speed, segment statistics, and checks
//! against point classification and brute-force tracing.
//!
//! cargo run --release -p llevant-bench -- trace model.mcnp [nlines] [length_cm]
//!
//! Two line sets: random directions starting inside random envelope boxes
//! (filled level-0 cells), and x-parallel lines across the hull of the
//! envelope boxes (as a volume grid would trace).
//!
//! With `LLEVANT_UNIVERSE=U`, traces that universe opened on its own
//! (`Model::universe`; without `LLEVANT_BENCH_SET`, whose workloads are
//! saved per file).

use crate::profile;
use crate::workload;
use llevant::Depth;

use std::collections::BTreeMap;
use std::time::Instant;

use llevant::Aabb;
use llevant::{Cut, Line, Model, Tracing};

/// Where segments are sampled. Not their midpoints: in symmetric geometry a
/// segment's midpoint can lie on an internal surface of its cell (one the
/// cell holds on both sides), where a point is on-surface.
const SAMPLE_FRACTIONS: [f64; 3] = [0.382, 0.073, 0.917];

pub fn run(args: Vec<String>) {
    let mut args = args.into_iter();
    let path = args
        .next()
        .expect("usage: bench_trace <file> [nlines] [length]");
    let n: usize = args.next().map_or(100_000, |s| s.parse().unwrap());
    let length: f64 = args.next().map_or(100.0, |s| s.parse().unwrap());

    let t = Instant::now();
    let model = crate::load(&path, &crate::options()).unwrap();
    // `LLEVANT_UNIVERSE=U`: that universe on its own, cut to what it holds.
    let model = match std::env::var("LLEVANT_UNIVERSE") {
        Ok(u) => {
            let grid = crate::options().grid;
            model.universe(u.parse().unwrap(), Cut::Own, grid).unwrap()
        }
        Err(_) => model,
    };
    println!("load {:.3} s", t.elapsed().as_secs_f64());

    let any_filled = model.units()[0]
        .cells
        .iter()
        .any(|&c| model.cells()[c as usize].fill.is_some());
    let envs: Vec<Aabb> = model.units()[0]
        .cells
        .iter()
        .map(|&c| &model.cells()[c as usize])
        .filter(|c| {
            (c.fill.is_some() || !any_filled)
                && c.bbox.is_finite()
                && (0..3).all(|i| c.bbox.hi[i] - c.bbox.lo[i] < 1e4)
                // Bounded by its own surfaces, not cut by universe 0's region.
                && (0..3).all(|i| {
                    let roi = model.units()[0].roi;
                    c.bbox.lo[i] > roi.lo[i] && c.bbox.hi[i] < roi.hi[i]
                })
        })
        .map(|c| c.bbox)
        .collect();
    let hull = envs.iter().fold(Aabb::EMPTY, |a, b| a.union(b));
    println!("{} envelope boxes, hull {hull:?}", envs.len());
    let mut rng = 0x9e3779b97f4a7c15u64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f64 / (1u64 << 53) as f64
    };

    let random = workload::lines(&path, &format!("random{n}x{length}"), || {
        (0..n)
            .map(|_| {
                let b = envs[(next() * envs.len() as f64) as usize];
                let origin = [0, 1, 2].map(|i| b.lo[i] + next() * (b.hi[i] - b.lo[i]));
                let z: f64 = 2.0 * next() - 1.0;
                let phi = 2.0 * std::f64::consts::PI * next();
                let s = (1.0 - z * z).sqrt();
                Line {
                    origin,
                    dir: [s * phi.cos(), s * phi.sin(), z],
                    t_min: 0.0,
                    t_max: length,
                }
            })
            .collect()
    });
    let grid = workload::lines(&path, &format!("hull{}", n / 10), || {
        (0..n / 10)
            .map(|_| Line {
                origin: [
                    hull.lo[0],
                    hull.lo[1] + next() * (hull.hi[1] - hull.lo[1]),
                    hull.lo[2] + next() * (hull.hi[2] - hull.lo[2]),
                ],
                dir: [1.0, 0.0, 0.0],
                t_min: 0.0,
                t_max: hull.hi[0] - hull.lo[0],
            })
            .collect()
    });

    for (name, lines) in [
        (format!("random {length} cm"), &random),
        ("x across hull".to_string(), &grid),
    ] {
        println!("\n== {name}: {} lines", lines.len());
        let prof = profile::start(if lines.len() == n {
            "trace_random"
        } else {
            "trace_hull"
        });
        let t = Instant::now();
        let tr = model.trace(lines, Depth::All).unwrap();
        let secs = t.elapsed().as_secs_f64();
        drop(prof);
        let segs = tr.t0.len();
        println!(
            "traced in {secs:.3} s: {:.3e} lines/s, {:.3e} segments/s, {:.1} segments/line, {} labels",
            lines.len() as f64 / secs,
            segs as f64 / secs,
            segs as f64 / lines.len() as f64,
            tr.labels.len()
        );
        report(&tr);
        let m = (lines.len() / 20).max(1);
        let single = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        let t = Instant::now();
        let one = single.install(|| model.trace(&lines[..m], Depth::All).unwrap());
        let secs = t.elapsed().as_secs_f64();
        println!("1 thread: {:.3e} lines/s", m as f64 / secs);
        let same = (0..m).all(|i| {
            let (a, b) = (tr.segments(i), one.segments(i));
            a.len() == b.len() && a.zip(b).all(|(x, y)| tr.segment(x) == one.segment(y))
        });
        println!("1-thread segments identical: {same}");
        let w = model.work_lines(&lines[..m]).unwrap();
        print!(
            "work per line:
{}",
            w.report()
        );
        let bad = check_points(&model, lines, &tr, 5000.min(lines.len()));
        check_brute(
            &model,
            lines,
            &(0..20.min(lines.len())).collect::<Vec<_>>(),
            &tr,
        );
        if !bad.is_empty() {
            println!("brute force on the lines with disagreements:");
            check_brute(&model, lines, &bad[..bad.len().min(40)], &tr);
        }
    }
}

fn report(tr: &Tracing) {
    let mut by_status: BTreeMap<String, (usize, f64)> = BTreeMap::new();
    let mut short = 0;
    for k in 0..tr.t0.len() {
        let l = &tr.labels[tr.ids[k] as usize];
        let e = by_status.entry(format!("{:?}", l.status)).or_default();
        e.0 += 1;
        e.1 += tr.t1[k] - tr.t0[k];
        if tr.t1[k] - tr.t0[k] < 1e-4 {
            short += 1;
        }
    }
    println!("status (segments, cm): {by_status:?}");
    println!("segments shorter than 1e-4 cm: {short}");
}

/// Points along every segment of the first `m` lines must classify to the
/// segment's label.
fn check_points(model: &Model, lines: &[Line], tr: &Tracing, m: usize) -> Vec<usize> {
    let mut pts = Vec::new();
    let mut want = Vec::new();
    let mut line_of = Vec::new();
    for (i, l) in lines.iter().enumerate().take(m) {
        for k in tr.segments(i) {
            let (a, b) = (tr.t0[k], tr.t1[k]);
            for f in SAMPLE_FRACTIONS {
                let t = a + f * (b - a);
                if (t - a).min(b - t) > 1e-6 {
                    pts.push([0, 1, 2].map(|j| l.origin[j] + t * l.dir[j]));
                    want.push(tr.ids[k]);
                    line_of.push(i);
                }
            }
        }
    }
    let got = model.classify(&pts, Depth::All);
    // A point the line passes at grazing incidence may be within the
    // tolerance of the surface its segment ends on: undecided, between the
    // segment's cell (at that level) and another.
    let grazing = llevant::reference::on_surface_next_to;
    let (mut bad, mut undecided) = (Vec::new(), 0);
    for i in 0..pts.len() {
        let (p, s) = (
            &got.labels[got.ids[i] as usize],
            &tr.labels[want[i] as usize],
        );
        if p != s {
            if grazing(p, s) {
                undecided += 1;
            } else {
                bad.push(i);
            }
        }
    }
    println!(
        "points along segments: {} checked, {} disagree, {undecided} on-surface next to the segment's cell",
        pts.len(),
        bad.len()
    );
    for &i in bad.iter().take(5) {
        println!(
            "  {:?}: point {:?}, segment {:?}",
            pts[i], got.labels[got.ids[i] as usize], tr.labels[want[i] as usize]
        );
    }
    for &i in bad.iter().take(2) {
        let li = line_of[i];
        let l = lines[li];
        let tp = (0..3)
            .map(|j| (pts[i][j] - l.origin[j]) * l.dir[j])
            .sum::<f64>();
        println!("  line {li} {l:?} t={tp}");
        for k in tr.segments(li) {
            let s = tr.segment(k);
            let sid = |x: Option<u32>| x.map(|x| model.surface_id(x).unwrap().id);
            println!(
                "    {} {} {:?} {:?} in {:?} out {:?}",
                s.t0,
                s.t1,
                s.label.status,
                s.label
                    .path
                    .iter()
                    .map(|p| model.cells()[p.cell as usize].id)
                    .collect::<Vec<_>>(),
                sid(s.surface_in),
                sid(s.surface_out)
            );
        }
        for dt in [-1e-3, -1e-6, -1e-9, 0.0, 1e-9, 1e-6, 1e-3] {
            let t = tp + dt;
            let q = [0, 1, 2].map(|j| l.origin[j] + t * l.dir[j]);
            let lab = model.classify_point(q);
            println!(
                "    t{dt:+e}: {:?} {:?} {:?}",
                lab.status,
                lab.path
                    .iter()
                    .map(|p| model.cells()[p.cell as usize].id)
                    .collect::<Vec<_>>(),
                lab.involved
                    .iter()
                    .map(|&c| model.cells()[c as usize].id)
                    .collect::<Vec<_>>()
            );
        }
    }
    let mut lines: Vec<usize> = bad.iter().map(|&i| line_of[i]).collect();
    lines.dedup();
    lines
}

fn check_brute(model: &Model, all: &[Line], which: &[usize], tr: &Tracing) {
    let t = Instant::now();
    let lines: Vec<Line> = which.iter().map(|&i| all[i]).collect();
    let slow = llevant::reference::trace(model, &lines, Depth::All).unwrap();
    let mut diff = 0;
    let mut disagree = 0;
    for (j, &i) in which.iter().enumerate() {
        let (a, b) = (tr.segments(i), slow.segments(j));
        let identical = a.len() == b.len()
            && a.clone()
                .zip(b.clone())
                .all(|(x, y)| tr.segment(x) == slow.segment(y));
        diff += !identical as usize;
        // Not identical, but the same as `reference::traces_agree` says (another
        // of surfaces within the tolerance named at an end, a sub-tolerance
        // on-surface sliver): fine.
        let (p, q): (Vec<_>, Vec<_>) = (
            a.clone().map(|k| tr.segment(k)).collect(),
            b.clone().map(|k| slow.segment(k)).collect(),
        );
        if !llevant::reference::traces_agree(model, &all[i], &p, &q) {
            disagree += 1;
            if disagree <= 2 {
                println!("  line {i} {:?}", all[i]);
                for x in a {
                    let s = tr.segment(x);
                    println!(
                        "    fast {} {} {:?} {:?}",
                        s.t0,
                        s.t1,
                        s.label.status,
                        s.label
                            .path
                            .iter()
                            .map(|p| model.cells()[p.cell as usize].id)
                            .collect::<Vec<_>>()
                    );
                }
                for y in b {
                    let s = slow.segment(y);
                    println!(
                        "    slow {} {} {:?} {:?}",
                        s.t0,
                        s.t1,
                        s.label.status,
                        s.label
                            .path
                            .iter()
                            .map(|p| model.cells()[p.cell as usize].id)
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
    }
    println!(
        "brute force on {} lines ({:.1} s): {diff} not identical, {disagree} disagree",
        lines.len(),
        t.elapsed().as_secs_f64()
    );
}
