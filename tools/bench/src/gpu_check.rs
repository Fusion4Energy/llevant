//! GPU first hits against CPU `first_hit`, ray by ray, on views of a model,
//! and whole cutaway frames against CPU `render`.
//!
//! cargo run --release -p llevant-bench -- gpu-check <model> [pixels]
//!
//! With `LLEVANT_UNIVERSE=U`, checks that universe opened on its own
//! (`Model::universe`, cut to what it holds). `GPU_SLICE_WORK` sets
//! [`GpuModel::slice_work`] (A/B: `4294967295` traces each ray in one go).
//! `GPU_EXIT_BOUND=0` stops cells at the first crossing of any surface (A/B).

use llevant::{Cut, GridMode, Line, Options};
use llevant_gpu::check::{compare, compare_frame, cutaways, views, visibilities};
use llevant_gpu::{Gpu, GpuModel};

pub fn run(args: Vec<String>) {
    let mut args = args.into_iter();
    let path = args.next().expect("usage: gpu_check <model> [pixels]");
    let n: usize = args.next().map_or(200, |s| s.parse().unwrap());
    let model = crate::load(&path, &Options::default()).unwrap();
    let model = match std::env::var("LLEVANT_UNIVERSE") {
        Ok(u) => model
            .universe(u.parse().unwrap(), Cut::Own, GridMode::Auto)
            .unwrap(),
        Err(_) => model,
    };
    let gpu = Gpu::new().unwrap();
    println!("adapter: {} ({:?})", gpu.info.name, gpu.info.backend);
    let mut gm = GpuModel::new(&gpu, &model.export()).unwrap();
    // `GPU_NO_SKIP`: step through hidden cells, as before skipping;
    // `GPU_EXIT_BOUND=0`: cells stop at the first crossing of any surface
    // (A/B).
    let mut constants = Vec::new();
    if std::env::var_os("GPU_NO_SKIP").is_some() {
        constants.push(("USE_SKIP", 0.0));
    }
    if std::env::var("GPU_EXIT_BOUND").as_deref() == Ok("0") {
        constants.push(("EXIT_BOUND", 0.0));
    }
    if !constants.is_empty() {
        gm.set_constants(&gpu, &constants);
    }
    if let Ok(w) = std::env::var("GPU_SLICE_WORK") {
        gm.slice_work = w.parse().unwrap();
    }
    for (name, vis) in visibilities(&model) {
        gm.set_visibility(&gpu, &model, &vis).unwrap();
        for (vname, cam) in views(&model, n) {
            let rays: Vec<Line> = cam.rays().into_iter().flatten().collect();
            let r = compare(&model, &gm, &gpu, &rays, &vis);
            println!("{name}, {vname}: {}", r.text);
        }
        for (vname, cam) in cutaways(&model, n) {
            let r = compare_frame(&model, &gm, &gpu, &cam, &vis);
            println!("{name}, {vname}: {}", r.text);
        }
    }
}
