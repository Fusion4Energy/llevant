//! Plotting workloads: 2D slices (one `trace` per image row) and 3D views
//! (one `first_hit` ray per pixel), timed per frame.
//!
//! cargo run --release -p llevant-bench -- view model.mcnp [pixels] [out_dir]
//!
//! With `out_dir`, writes the slice and the views as PPM images (cell colours
//! hashed from the deepest cell; views shaded by the normal).

use crate::profile;
use crate::workload;
use llevant::Depth;
use llevant_gpu::check as emptied;

use std::time::Instant;

use llevant::{
    Aabb, Axis, Camera, Clip, Cutaway, Hits, Label, Line, Model, Options, Slice, Status, Tracing,
    Visibility, NO_HIT,
};

pub fn run(args: Vec<String>) {
    let mut args = args.into_iter();
    let path = args
        .next()
        .expect("usage: bench_view <file> [pixels] [out_dir]");
    let n: usize = args.next().map_or(1000, |s| s.parse().unwrap());
    let out = args.next();

    let t = Instant::now();
    let model = crate::load(&path, &Options::default()).unwrap();
    println!("load {:.3} s", t.elapsed().as_secs_f64());
    let hull = workload::boxes(&path, "view_hull", || vec![envelope_hull(&model)])[0];
    println!("hull {hull:?}");
    let all_threads = format!("{} threads", rayon::current_num_threads());
    let pools = [
        (all_threads.as_str(), rayon::current_num_threads()),
        ("16 threads", 16),
    ];

    // Slices: z through the middle across the hull, and zoomed onto a random
    // envelope (through its middle).
    let mut rng0 = 0x2545f4914f6cdd1du64;
    let mut next0 = move || {
        rng0 ^= rng0 << 13;
        rng0 ^= rng0 >> 7;
        rng0 ^= rng0 << 17;
        (rng0 >> 11) as f64 / (1u64 << 53) as f64
    };
    let zoom = workload::boxes(&path, "view_zoom", || {
        vec![random_envelope(&model, &mut next0)]
    })[0];
    let mut slice = None;
    for (sname, b) in [("whole model", hull), ("one envelope", zoom)] {
        let zc = 0.5 * (b.lo[2] + b.hi[2]);
        let sl = Slice {
            ny: n,
            ..Slice::axis(Axis::Z, zc, &b, n)
        };
        println!(
            "\n== slice of the {sname}, z = {zc:.1}: {n} rows of {:.0} cm",
            b.hi[0] - b.lo[0]
        );
        for (name, threads) in pools
            .iter()
            .copied()
            .chain([("4 threads", 4), ("1 thread", 1)])
        {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            let (secs, tr) = best_of(3, || pool.install(|| model.slice(&sl, Depth::All).unwrap()));
            println!(
                "{name}: {:.1} ms/frame, {} segments",
                1e3 * secs,
                tr.t0.len()
            );
            if slice.is_none() {
                slice = Some((sl.clone(), tr));
            }
        }
    }
    let (slice, slice_tr) = slice.unwrap();

    // The three ways models are looked at: fillers (cells with a material or
    // a fill), void envelopes (every cell but the graveyard and the void
    // around the eye), and both together in a cutaway through an envelope.
    let solid = Visibility::solid(&model);
    let mut rng = 0x9e3779b97f4a7c15u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 11) as f64 / (1u64 << 53) as f64
    };
    // The whole model from outside, and a close-up of a random envelope.
    let c = hull.center();
    let size = (0..3).map(|i| hull.hi[i] - hull.lo[i]).fold(0.0, f64::max);
    let env = workload::boxes(&path, "view_env", || {
        vec![random_envelope(&model, &mut next)]
    })[0];
    let ec = env.center();
    let esize = (0..3).map(|i| env.hi[i] - env.lo[i]).fold(0.0, f64::max);
    let (kept_envelope, kept_filler, kept_box, kept) = emptied::emptied(&model);
    println!("emptied views: {kept}");
    let kept_box = kept_box.intersect(&hull);
    let kc = kept_box.center();
    let ksize = (0..3)
        .map(|i| kept_box.hi[i] - kept_box.lo[i])
        .fold(0.0, f64::max);
    let views = [
        ("whole model", c, size * 1.5, 40.0f64, "solid"),
        ("close-up of an envelope", ec, esize * 1.2, 50.0, "solid"),
        ("whole model, voids shown", c, size * 1.5, 40.0, "voids"),
        (
            "cutaway of an envelope, voids shown",
            ec,
            esize * 1.2,
            50.0,
            "cutaway",
        ),
        (
            "whole model, one envelope kept",
            c,
            size * 1.5,
            40.0,
            "envelope",
        ),
        (
            "whole model, one filler cell kept",
            c,
            size * 1.5,
            40.0,
            "filler",
        ),
        (
            "framed, one envelope kept",
            kc,
            ksize * 1.5,
            40.0,
            "envelope",
        ),
        (
            "framed, one filler cell kept",
            kc,
            ksize * 1.5,
            40.0,
            "filler",
        ),
    ];
    for (name, target, dist, fov, mode) in views {
        let eye = [
            target[0] - 0.6 * dist,
            target[1] - 0.7 * dist,
            target[2] + 0.4 * dist,
        ];
        let cam = Camera {
            eye,
            target,
            up: [0.0, 0.0, 1.0],
            fov_deg: fov,
            width: n,
            height: n,
            reach: 2.0 * dist + size,
            cut: if mode == "cutaway" {
                Cutaway::plane(Clip {
                    point: target,
                    normal: [0.0, -1.0, 0.0],
                })
            } else {
                Cutaway::none()
            },
            ortho: None,
        };
        let vis = match mode {
            "solid" => solid.clone(),
            "envelope" => kept_envelope.clone(),
            "filler" => kept_filler.clone(),
            _ => voids_shown(&model, eye),
        };
        let rays: Vec<Line> = cam.rays().into_iter().flatten().collect();
        println!("\n== first_hit, {name}: {n}×{n} rays, eye {eye:.0?}");
        let mut hits = None;
        for (pname, threads) in pools {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            let (secs, h) = best_of(2, || pool.install(|| model.render(&cam, &vis).unwrap()));
            let hit = h.ids.iter().filter(|&&i| i != NO_HIT).count();
            println!(
                "{pname}: {:.0} ms/frame ({:.2e} rays/s), {:.1}% hit",
                1e3 * secs,
                rays.len() as f64 / secs,
                100.0 * hit as f64 / rays.len() as f64
            );
            // Gaps and problems hidden: the whole search, and only where
            // `Model::skip` leaves something to see (as interactive views).
            let quiet = Visibility {
                gaps: false,
                problems: false,
                ..vis.clone()
            };
            let pixels: Vec<(usize, usize)> =
                (0..n).flat_map(|j| (0..n).map(move |i| (i, j))).collect();
            let (all_secs, all) =
                best_of(2, || pool.install(|| model.render(&cam, &quiet).unwrap()));
            let t = Instant::now();
            let skip = model.skip(&quiet).unwrap();
            let skip_secs = t.elapsed().as_secs_f64();
            let (some_secs, some) = best_of(2, || {
                pool.install(|| {
                    model
                        .pixel_hits(&cam, &pixels, &quiet, Some(&skip))
                        .unwrap()
                })
            });
            let differ = (0..pixels.len())
                .filter(|&p| {
                    (all.ids[p] == NO_HIT) != (some.ids[p] == NO_HIT)
                        || (all.ids[p] != NO_HIT
                            && ((all.t[p] - some.t[p]).abs() > 1e-6
                                || all.labels[all.ids[p] as usize]
                                    != some.labels[some.ids[p] as usize]))
                })
                .count();
            println!(
                "  gaps and problems hidden: {:.0} ms/frame, skipping {:.0} ms/frame ({:.1}×; skip {:.0} ms), {differ} differ",
                1e3 * all_secs,
                1e3 * some_secs,
                all_secs / some_secs,
                1e3 * skip_secs
            );
            hits = Some(h);
        }
        let image = hits.unwrap();
        // Per kept ray (a cutaway drops some pixels' rays), for the checks.
        let hits = model.first_hits(&rays, &vis, None).unwrap();
        let prof = profile::start(&format!(
            "first_hit_{}",
            name.split_whitespace().next().unwrap()
        ));
        let (secs, _) = best_of(1, || model.render(&cam, &vis).unwrap());
        drop(prof);
        let (tsecs, tr) = best_of(1, || model.trace(&rays, Depth::All).unwrap());
        println!(
            "full trace of the same rays: {:.0} ms ({:.1}× first_hit), {:.1} segments/ray",
            1e3 * tsecs,
            tsecs / secs,
            tr.t0.len() as f64 / rays.len() as f64
        );
        check(&hits, &tr, &vis);
        let sample: Vec<Line> = rays
            .iter()
            .step_by((rays.len() / 20_000).max(1))
            .copied()
            .collect();
        let w = model.work_lines(&sample).unwrap();
        print!(
            "work per ray (full trace):
{}",
            w.report()
        );
        if let Some(dir) = &out {
            let file = format!("{dir}/view_{}.ppm", name.split_whitespace().next().unwrap());
            write_view(&file, n, &image, &cam);
            println!("wrote {file}");
        }
    }
    if let Some(dir) = &out {
        let file = format!("{dir}/slice.ppm");
        write_slice(&file, &slice, &slice_tr);
        println!("wrote {file}");
    }
}

/// Best wall time of `k` runs, and the last result.
fn best_of<T>(k: usize, mut f: impl FnMut() -> T) -> (f64, T) {
    let mut best = f64::INFINITY;
    let mut last = None;
    for _ in 0..k {
        let t = Instant::now();
        let r = f();
        best = best.min(t.elapsed().as_secs_f64());
        last = Some(r);
    }
    (best, last.unwrap())
}

/// Every cell visible except the graveyard (`IMP:N=0`) and the level-0
/// cell holding `eye` (the void around the model).
fn voids_shown(model: &Model, eye: [f64; 3]) -> Visibility {
    let mut vis = Visibility::all(model);
    for (c, cell) in model.cells().iter().enumerate() {
        vis.cells[c] = !cell.imp_zero;
    }
    if let Some(step) = model.classify_point(eye).path.first() {
        vis.cells[step.cell as usize] = false;
    }
    vis
}

fn level0_boxes(model: &Model) -> Vec<Aabb> {
    let any_filled = model.units()[0]
        .cells
        .iter()
        .any(|&c| model.cells()[c as usize].fill.is_some());
    model.units()[0]
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
        .collect()
}

fn envelope_hull(model: &Model) -> Aabb {
    level0_boxes(model)
        .iter()
        .fold(Aabb::EMPTY, |a, b| a.union(b))
}

fn random_envelope(model: &Model, next: &mut impl FnMut() -> f64) -> Aabb {
    let b = level0_boxes(model);
    b[(next() * b.len() as f64) as usize % b.len()]
}

/// First hits must be the first visible segment of the full trace.
fn check(hits: &Hits, tr: &Tracing, vis: &Visibility) {
    let shown = |l: &Label| {
        (match l.status {
            Status::Inside => true,
            Status::Gap => vis.gaps,
            Status::OnSurface => false,
            _ => vis.problems,
        }) && l.path.iter().all(|s| vis.cells[s.cell as usize])
    };
    let mut bad = 0;
    for i in 0..hits.t.len() {
        let first = tr
            .segments(i)
            .find(|&k| shown(&tr.labels[tr.ids[k] as usize]));
        let ok = match first {
            None => hits.ids[i] == NO_HIT,
            Some(k) => {
                hits.ids[i] != NO_HIT
                    && hits.t[i] == tr.t0[k]
                    && hits.labels[hits.ids[i] as usize] == tr.labels[tr.ids[k] as usize]
            }
        };
        bad += !ok as usize;
    }
    println!("first_hit vs first visible segment of trace: {bad} differ");
}

fn colour(cell: u32) -> [u8; 3] {
    let h = (cell as u64 + 1).wrapping_mul(0x9e3779b97f4a7c15);
    [
        (h >> 16) as u8 | 0x40,
        (h >> 32) as u8 | 0x40,
        (h >> 48) as u8 | 0x40,
    ]
}

fn write_view(file: &str, n: usize, h: &Hits, cam: &Camera) {
    let mut img = Vec::with_capacity(3 * n * n);
    for i in 0..n * n {
        let px = if h.ids[i] == NO_HIT {
            [255, 255, 255]
        } else {
            let l = &h.labels[h.ids[i] as usize];
            let base = match l.status {
                Status::Inside => colour(l.path.last().map_or(0, |s| s.cell)),
                _ => [255, 0, 255],
            };
            let dir = cam.direction(i % n, i / n);
            let nd = -(0..3).map(|k| h.normal[i][k] * dir[k]).sum::<f64>();
            let shade = if h.normal[i] == [0.0; 3] {
                1.0
            } else {
                0.25 + 0.75 * nd.max(0.0)
            };
            base.map(|c| (c as f64 * shade) as u8)
        };
        img.extend_from_slice(&px);
    }
    write_ppm(file, n, n, &img);
}

fn write_slice(file: &str, sl: &Slice, tr: &Tracing) {
    let mut img = Vec::with_capacity(3 * sl.nx * sl.ny);
    for id in sl.pixels(tr) {
        let l = &tr.labels[id as usize];
        img.extend_from_slice(&match l.status {
            Status::Inside => colour(l.path.last().map_or(0, |s| s.cell)),
            Status::Gap => [0, 0, 0],
            _ => [255, 0, 255],
        });
    }
    write_ppm(file, sl.nx, sl.ny, &img);
}

fn write_ppm(file: &str, w: usize, h: usize, img: &[u8]) {
    let mut data = format!("P6\n{w} {h}\n255\n").into_bytes();
    data.extend_from_slice(img);
    std::fs::write(file, data).unwrap();
}
