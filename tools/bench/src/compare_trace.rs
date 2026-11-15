//! Trace the same lines with octrees on (`GridMode::Auto`, residual programs
//! included) and off, compare them line by line (exactly, and as
//! `reference::traces_agree` says), and settle each line that differs with brute
//! force.
//!
//! cargo run --release -p llevant-bench -- compare-trace <model> <lines.f64>
//!
//! `lines.f64` is a line set saved by `bench_trace` (`LLEVANT_BENCH_SET`).

use llevant::Depth;
use llevant::{GridMode, Line, Options, Tracing};

pub fn run(args: Vec<String>) {
    let a: Vec<String> = args;
    let usage = "usage: compare_trace <model> <lines.f64>";
    let path = a.first().expect(usage);
    let set = a.get(1).expect(usage);
    let bytes = std::fs::read(set).unwrap();
    let flat: Vec<f64> = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| f64::from_le_bytes(*c))
        .collect();
    let lines: Vec<Line> = flat
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| Line {
            origin: [c[0], c[1], c[2]],
            dir: [c[3], c[4], c[5]],
            t_min: c[6],
            t_max: c[7],
        })
        .collect();
    println!("{} lines", lines.len());
    let on = crate::load(path, &Options::default()).unwrap();
    let a = on.trace(&lines, Depth::All).unwrap();
    println!("octrees on: {} labels", a.labels.len());
    let b = {
        let off = crate::load(path, &Options::default().with_grid(GridMode::Off)).unwrap();
        off.trace(&lines, Depth::All).unwrap()
    };
    println!("octrees off: {} labels", b.labels.len());
    let agree = |x: &Tracing, i: usize, y: &Tracing, j: usize| {
        let p: Vec<_> = x.segments(i).map(|k| x.segment(k)).collect();
        let q: Vec<_> = y.segments(j).map(|k| y.segment(k)).collect();
        llevant::reference::traces_agree(&on, &lines[i], &p, &q)
    };
    let differ: Vec<usize> = (0..lines.len()).filter(|&i| !agree(&a, i, &b, i)).collect();
    println!("{} lines differ between octrees on and off", differ.len());
    // Same labels and ends within the tolerance: only another of nearly
    // coincident surfaces is named at an end.
    let tol = 2e-8;
    let ends = |x: &Tracing, i: usize, y: &Tracing, j: usize| {
        let (p, q) = (x.segments(i), y.segments(j));
        p.len() == q.len()
            && p.zip(q).all(|(s, t)| {
                let (s, t) = (x.segment(s), y.segment(t));
                s.label == t.label && (s.t0 - t.t0).abs() <= tol && (s.t1 - t.t1).abs() <= tol
            })
    };
    let real: Vec<usize> = differ
        .iter()
        .copied()
        .filter(|&i| !ends(&a, i, &b, i))
        .collect();
    println!(
        "{} of them differ in more than a surface named at an end: {:?}",
        real.len(),
        &real[..real.len().min(20)]
    );
    // For lines that differ only in a surface named at an end: how far each
    // named surface is from that end (smallest tolerance that puts the end
    // point on it).
    let dist = |s: u32, p: [f64; 3]| {
        let shape = &on.shapes()[s as usize];
        let mut t = 1e-12;
        while t < 1.0 && shape.side(p, t) != llevant::internals::Side::On {
            t *= 2.0;
        }
        t
    };
    for &i in differ.iter().filter(|i| !real.contains(i)).take(10) {
        for (s, t) in a.segments(i).zip(b.segments(i)) {
            let (s, t) = (a.segment(s), b.segment(t));
            for (x, y, at) in [
                (s.surface_in, t.surface_in, s.t0),
                (s.surface_out, t.surface_out, s.t1),
            ] {
                if let (Some(x), Some(y)) = (x, y) {
                    if x != y {
                        let p = lines[i].at(at);
                        println!(
                            "line {i} t = {at}: on names {:?} at {:.1e} cm, off {:?} at {:.1e} cm",
                            on.surface_id(x),
                            dist(x, p),
                            on.surface_id(y),
                            dist(y, p)
                        );
                    }
                }
            }
        }
    }
    let few: Vec<Line> = real
        .iter()
        .chain(&differ)
        .take(200)
        .map(|&i| lines[i])
        .collect();
    let differ: Vec<usize> = real.iter().chain(&differ).copied().take(200).collect();
    let brute = llevant::reference::trace(&on, &few, Depth::All).unwrap();
    let (mut on_ok, mut off_ok) = (0, 0);
    for (j, &i) in differ.iter().take(200).enumerate() {
        let (x, y) = (agree(&a, i, &brute, j), agree(&b, i, &brute, j));
        on_ok += x as usize;
        off_ok += y as usize;
        if !ends(&a, i, &brute, j) {
            println!("line {i}: on agrees with brute force {x}, off {y}");
            println!("  {:?}", lines[i]);
            for s in a.segments(i) {
                let s = a.segment(s);
                println!(
                    "  on    {} {} {} {:?} {:?}",
                    s.t0,
                    s.t1,
                    on.describe(&s.label),
                    s.surface_in.and_then(|x| on.surface_id(x)),
                    s.surface_out.and_then(|x| on.surface_id(x))
                );
            }
            for t in brute.segments(j) {
                let t = brute.segment(t);
                println!(
                    "  brute {} {} {} {:?} {:?}",
                    t.t0,
                    t.t1,
                    on.describe(&t.label),
                    t.surface_in.and_then(|x| on.surface_id(x)),
                    t.surface_out.and_then(|x| on.surface_id(x))
                );
            }
            // Point classification (residual programs, no crossings) at the
            // middle of each brute-force segment against brute force.
            for t in brute.segments(j) {
                let t = brute.segment(t);
                let p = lines[i].at(0.5 * (t.t0 + t.t1));
                let fast = on.classify_point(p);
                let slow = llevant::reference::classify(&on, &[p], Depth::All);
                let slow = &slow.labels[slow.ids[0] as usize];
                if &fast != slow {
                    println!(
                        "  point at t = {}: classify {} / brute force {}",
                        0.5 * (t.t0 + t.t1),
                        on.describe(&fast),
                        on.describe(slow)
                    );
                }
            }
        }
    }
    println!(
        "of {} differing lines checked: octrees on agree with brute force on {on_ok}, off on {off_ok}",
        few.len()
    );
}
