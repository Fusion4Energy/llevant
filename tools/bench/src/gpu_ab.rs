//! A/B of shader variants in one process: the views of `gpu_check` rendered
//! in turn with each variant of the override constants, several rounds, so
//! that the GPU's changing speed (heat, the display) hits every variant
//! alike. Prints the best and median frame time of each. Frames are timed
//! back to back after a warm-up (`gpu_time`), so the GPU keeps its clock.
//!
//! cargo run --release -p llevant-bench -- gpu-ab <model> [width height] [rounds] [solid|voids|envelope|filler]
//!
//! `envelope` and `filler` keep one envelope, or one cell of its filler
//! (`bench_view`'s emptied views).

use llevant_gpu::check;

use llevant::{Options, Visibility};
use llevant_gpu::{Gpu, GpuModel};

/// `(name, override constants)`.
const VARIANTS: &[(&str, &[(&str, f64)])] = &[
    ("first crossing", &[("EXIT_BOUND", 0.0)]),
    ("exit bound", &[("EXIT_BOUND", 1.0)]),
];

pub fn run(args: Vec<String>) {
    let a: Vec<String> = args;
    let path = a
        .first()
        .expect("usage: gpu_ab <model> [width height] [rounds] [solid|voids|envelope|filler]");
    let width: usize = a.get(1).map_or(1920, |s| s.parse().unwrap());
    let height: usize = a.get(2).map_or(1080, |s| s.parse().unwrap());
    let rounds: usize = a.get(3).map_or(5, |s| s.parse().unwrap());
    let mode = a.get(4).map_or("solid", String::as_str);
    let voids = mode == "voids";
    let model = crate::load(path, &Options::default()).unwrap();
    let (kept_envelope, kept_filler, _, kept) = check::emptied(&model);
    println!("emptied views: {kept}");
    let gpu = Gpu::new().unwrap();
    let mut gm = GpuModel::new(&gpu, &model.export()).unwrap();
    let views = check::views(&model, 1);
    let mut timer = crate::gpu_time::FrameTimer::new();
    // times[view][variant]
    let mut times = vec![vec![Vec::new(); VARIANTS.len()]; views.len()];
    for _ in 0..rounds {
        for (vi, (_, constants)) in VARIANTS.iter().enumerate() {
            gm.set_constants(&gpu, constants);
            for (wi, (_, cam)) in views.iter().enumerate() {
                let mut cam = cam.clone();
                cam.width = width;
                cam.height = height;
                let vis = if voids {
                    let mut v = Visibility::all(&model);
                    for (c, cell) in model.cells().iter().enumerate() {
                        v.cells[c] = !cell.imp_zero;
                    }
                    if let Some(s) = model.classify_point(cam.eye).path.first() {
                        v.cells[s.cell as usize] = false;
                    }
                    v
                } else if mode == "envelope" {
                    kept_envelope.clone()
                } else if mode == "filler" {
                    kept_filler.clone()
                } else {
                    Visibility::solid(&model)
                };
                gm.set_visibility(&gpu, &model, &vis).unwrap();
                times[wi][vi].push(timer.best(&gpu, &gm, &cam, 1));
            }
        }
    }
    println!("{}", timer.clock_report());
    for (wi, (name, _)) in views.iter().enumerate() {
        println!("{name}, {width}×{height}:");
        for (vi, (vname, _)) in VARIANTS.iter().enumerate() {
            let mut t = times[wi][vi].clone();
            t.sort_by(f64::total_cmp);
            println!(
                "  {vname:22} best {:7.1} ms, median {:7.1} ms",
                1e3 * t[0],
                1e3 * t[t.len() / 2]
            );
        }
    }
}
