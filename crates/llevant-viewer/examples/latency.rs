//! GPU time of camera images of growing size: how much of it is a fixed
//! latency (the slowest rays) and how much is per ray.
use llevant::{Model, Options, Scope, Visibility};
use llevant_gpu::{Gpu, GpuModel};
use llevant_viewer::camera::{visible_extent, Orbit};

fn main() {
    let path = std::env::args().nth(1).expect("model");
    let m = Model::load(&path, &Options::default(), None).unwrap();
    let mut region = m.extent(Scope::Contents, 64);
    if region.is_empty() {
        region = m.extent(Scope::Problem, 64);
    }
    let vis = Visibility::solid(&m);
    let region = visible_extent(&m, &vis, &region, 48).unwrap_or(region);
    let gpu = Gpu::new().unwrap();
    println!("{} ({:?})", gpu.info.name, gpu.info.backend);
    let mut gm = GpuModel::upload(&gpu, &m).unwrap();
    gm.set_visibility(&gpu, &m, &vis).unwrap();
    for steps in [4096, 64] {
        gm.max_steps = steps;
        for yaw in [35.0, 120.0] {
            let o = Orbit::fit(&region, yaw, 25.0);
            for side in [8usize, 32, 64, 128, 256, 512] {
                let cam = o.camera(side, side, None);
                let mut best = f64::MAX;
                for _ in 0..3 {
                    best = best.min(gm.render(&gpu, &cam).unwrap().gpu_seconds);
                }
                let f = gm.render(&gpu, &cam).unwrap();
                let max_steps = f.hits.iter().map(|h| h.steps).max().unwrap();
                println!(
                    "steps {steps} yaw {yaw}: {side}x{side} = {} rays: {:.2} ms (max steps {max_steps})",
                    side * side,
                    1e3 * best
                );
            }
        }
    }
}
