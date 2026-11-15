//! Load a model, report compile timings and box quality, then classify random
//! points inside the bounding box of the level-0 cells.
//!
//! cargo run --release -p llevant-bench -- points model.mcnp [npoints]

use crate::profile;
use crate::workload;
use llevant::Depth;

use std::collections::BTreeMap;
use std::time::Instant;

use llevant::Aabb;
use llevant::Status;

pub fn run(args: Vec<String>) {
    let mut args = args.into_iter();
    let path = args.next().expect("usage: bench <file> [npoints]");
    let n: usize = args.next().map_or(1_000_000, |s| s.parse().unwrap());

    let t = Instant::now();
    let model = crate::load(&path, &crate::options()).unwrap();
    let total = t.elapsed().as_secs_f64();
    for (name, secs) in model.timings() {
        println!("{name:<10} {secs:8.3} s");
    }
    println!("{:<10} {total:8.3} s", "total");
    println!("{:#?}", model.stats());
    print_rss();

    for d in model.diagnostics().iter().take(10) {
        println!("diagnostic: {d}");
    }
    if model.diagnostics().len() > 10 {
        println!("... {} diagnostics", model.diagnostics().len());
    }

    let t = Instant::now();
    let fp = model.fingerprints(llevant::DEFAULT_PRECISION).unwrap();
    let secs = t.elapsed().as_secs_f64();
    let distinct = |v: Vec<llevant::Digest>| {
        let mut v = v;
        v.sort_unstable();
        v.dedup();
        v.len()
    };
    println!(
        "fingerprints {secs:.3} s: {} distinct cell geometries of {}, {} distinct universe geometries of {}",
        distinct(fp.cells.iter().map(|f| f.geometry).collect()),
        fp.cells.len(),
        distinct(fp.units.iter().map(|f| f.geometry).collect()),
        fp.units.len()
    );

    // Box quality: how many cells per unit have finite boxes, and how large.
    let mut finite = 0;
    let mut infinite_by_unit: BTreeMap<i64, usize> = BTreeMap::new();
    for c in model.cells() {
        if c.bbox.is_finite() {
            finite += 1;
        } else if !c.bbox.is_empty() {
            *infinite_by_unit
                .entry(model.units()[c.unit as usize].universe)
                .or_default() += 1;
        }
    }
    println!("finite boxes: {finite} / {}", model.cells().len());
    let mut worst: Vec<_> = infinite_by_unit.into_iter().collect();
    worst.sort_by_key(|w| std::cmp::Reverse(w.1));
    println!(
        "unbounded cells by universe (top 10): {:?}",
        &worst[..worst.len().min(10)]
    );

    // Sample uniformly inside the boxes of randomly chosen envelopes (filled
    // level-0 cells), where the detail is; any level-0 cell if none is filled.
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
    println!("sampling inside {} envelope boxes", envs.len());
    let mut rng = 0x9e3779b97f4a7c15u64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f64 / (1u64 << 53) as f64
    };
    let points = workload::points(&path, &format!("points{n}"), || {
        (0..n)
            .map(|_| {
                let b = envs[(next() * envs.len() as f64) as usize];
                [0, 1, 2].map(|i| b.lo[i] + next() * (b.hi[i] - b.lo[i]))
            })
            .collect()
    });
    let prof = profile::start("classify");
    let t = Instant::now();
    let res = model.classify(&points, Depth::All);
    let secs = t.elapsed().as_secs_f64();
    drop(prof);
    println!(
        "classified {n} points in {secs:.3} s: {:.3e} points/s, {} labels",
        n as f64 / secs,
        res.labels.len()
    );
    let mut by_status: BTreeMap<String, usize> = BTreeMap::new();
    let mut depth: BTreeMap<usize, usize> = BTreeMap::new();
    for &id in &res.ids {
        let l = &res.labels[id as usize];
        *by_status.entry(format!("{:?}", l.status)).or_default() += 1;
        if l.status == Status::Inside {
            *depth.entry(l.path.len()).or_default() += 1;
        }
    }
    println!("status: {by_status:?}");
    println!("depth of inside points: {depth:?}");
    let t = Instant::now();
    let single = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let m = 100_000.min(n);
    let res1 = single.install(|| model.classify(&points[..m], Depth::All));
    let secs = t.elapsed().as_secs_f64();
    println!("1 thread: {:.3e} points/s", m as f64 / secs);
    let same = res1
        .ids
        .iter()
        .zip(&res.ids)
        .all(|(a, b)| res1.labels[*a as usize] == res.labels[*b as usize]);
    println!("1-thread labels identical: {same}");
    let w = model.work_points(&points[..m]);
    print!(
        "work per point:
{}",
        w.report()
    );
}

fn print_rss() {
    if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
        for l in s
            .lines()
            .filter(|l| l.starts_with("VmHWM") || l.starts_with("VmRSS"))
        {
            println!("{l}");
        }
    }
}
