//! Compare BVH classification with the brute-force reference on points
//! sampled inside random level-0 filled cells (the envelopes).
//!
//! cargo run --release -p llevant-bench -- verify model.mcnp [npoints]
//!
//! With `LLEVANT_UNIVERSE=U`, checks that universe opened on its own
//! (`Model::universe`, cut to what it holds).
use llevant::Depth;
use std::time::Instant;

use llevant::{Cut, Model};

pub fn run(args: Vec<String>) {
    let mut args = args.into_iter();
    let path = args.next().expect("usage: verify <file> [npoints]");
    let n: usize = args.next().map_or(200_000, |s| s.parse().unwrap());
    let model = crate::load(&path, &crate::options()).unwrap();
    // `LLEVANT_UNIVERSE=U`: that universe on its own, cut to what it holds.
    let model = match std::env::var("LLEVANT_UNIVERSE") {
        Ok(u) => {
            let grid = crate::options().grid;
            model.universe(u.parse().unwrap(), Cut::Own, grid).unwrap()
        }
        Err(_) => model,
    };
    let points = llevant_examples_sample(&model, n);
    let t = Instant::now();
    let fast = model.classify(&points, Depth::All);
    let tf = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let slow = llevant::reference::classify(&model, &points, Depth::All);
    let ts = t.elapsed().as_secs_f64();
    println!(
        "bvh {:.3e} points/s, brute force {:.3e} points/s",
        n as f64 / tf,
        n as f64 / ts
    );
    let mut bad = 0;
    for (i, (a, b)) in fast.ids.iter().zip(&slow.ids).enumerate() {
        let (la, lb) = (&fast.labels[*a as usize], &slow.labels[*b as usize]);
        if la != lb {
            bad += 1;
            if bad <= 10 {
                let ids = |l: &llevant::Label| {
                    l.path
                        .iter()
                        .map(|s| model.cells()[s.cell as usize].id)
                        .collect::<Vec<_>>()
                };
                println!(
                    "point {:?}: bvh {:?} {:?} vs brute {:?} {:?}",
                    points[i],
                    la.status,
                    ids(la),
                    lb.status,
                    ids(lb)
                );
            }
        }
    }
    let mut by: std::collections::BTreeMap<String, usize> = Default::default();
    let mut depth: std::collections::BTreeMap<usize, usize> = Default::default();
    for &id in &fast.ids {
        let l = &fast.labels[id as usize];
        *by.entry(format!("{:?}", l.status)).or_default() += 1;
        *depth.entry(l.path.len()).or_default() += 1;
    }
    println!(
        "status {by:?}\ndepth {depth:?}\nlabels {}",
        fast.labels.len()
    );
    println!("{bad} disagreements out of {n}");
}

/// Points uniformly inside the boxes of randomly chosen level-0 filled cells
/// (any level-0 cells if none is filled).
fn llevant_examples_sample(model: &Model, n: usize) -> Vec<[f64; 3]> {
    let any_filled = model.units()[0]
        .cells
        .iter()
        .any(|&c| model.cells()[c as usize].fill.is_some());
    let envs: Vec<_> = model.units()[0]
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
        .collect();
    let mut rng = 0x9e3779b97f4a7c15u64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f64 / (1u64 << 53) as f64
    };
    (0..n)
        .map(|_| {
            let b = envs[(next() * envs.len() as f64) as usize].bbox;
            let mut p = [0.0; 3];
            for i in 0..3 {
                p[i] = b.lo[i] + next() * (b.hi[i] - b.lo[i]);
            }
            p
        })
        .collect()
}
