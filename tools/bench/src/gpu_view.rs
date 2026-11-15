//! Frames of a model on the GPU, timed, with void cells hidden: the model
//! from outside, framed by the 5–95th percentiles of its envelope boxes (a
//! few far-away envelopes would make it tiny), and the view from inside of
//! `gpu_check`.
//!
//! cargo run --release -p llevant-bench -- gpu-view <model> [width height] [out_dir]
//!
//! With `GPU_VIEW_CPU=1`, also times CPU `first_hit` on the same rays; with
//! `GPU_VIEW_VEIL=1`, gaps are seen through (`Visibility::veil`); with
//! `LLEVANT_BENCH_SET=<dir>`, saves the cameras there or replays them (A/B
//! runs see the same views even when a change moves the boxes they come from).
//!
//! With `out_dir`, writes each frame as a PPM image (colour hashed from the
//! cell, shaded by the normal).

use llevant_gpu::check;

use crate::workload;

use std::time::Instant;

use llevant::{Model, Options, Visibility};
use llevant_gpu::{Frame, Gpu, GpuModel};

pub fn run(args: Vec<String>) {
    let a: Vec<String> = args;
    let path = a
        .first()
        .expect("usage: gpu_view <model> [width height] [out_dir]");
    let width: usize = a.get(1).map_or(1920, |s| s.parse().unwrap());
    let height: usize = a.get(2).map_or(1080, |s| s.parse().unwrap());
    let out = a.get(3);

    let t = Instant::now();
    let model = crate::load(path, &Options::default()).unwrap();
    println!("load {:.2} s", t.elapsed().as_secs_f64());
    let gpu = Gpu::new().unwrap();
    println!(
        "adapter: {} ({:?}, {:?})",
        gpu.info.name, gpu.info.device_type, gpu.info.backend
    );
    let t = Instant::now();
    let export = model.export();
    let export_secs = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let gm = GpuModel::new(&gpu, &export).unwrap();
    println!(
        "export {export_secs:.2} s, upload {:.2} s, {} MB on the GPU",
        t.elapsed().as_secs_f64(),
        gm.bytes >> 20
    );
    let vis = Visibility {
        veil: std::env::var_os("GPU_VIEW_VEIL").is_some(),
        ..Visibility::solid(&model)
    };
    println!("gaps seen through: {}", vis.veil);
    gm.set_visibility(&gpu, &model, &vis).unwrap();

    let cams = workload::cached(path, "gpu_views", || {
        let inside = check::views(&model, 1).swap_remove(2).1;
        [outside(&model), inside]
            .iter()
            .flat_map(|c| [&c.eye[..], &c.target, &c.up, &[c.fov_deg, c.reach]].concat())
            .collect()
    });
    let cam = |k: usize| {
        let v = &cams[11 * k..11 * (k + 1)];
        llevant::Camera {
            eye: [v[0], v[1], v[2]],
            target: [v[3], v[4], v[5]],
            up: [v[6], v[7], v[8]],
            fov_deg: v[9],
            width,
            height,
            reach: v[10],
            cut: llevant::Cutaway::none(),
            ortho: None,
        }
    };
    for (name, mut cam) in [("outside", cam(0)), ("inside", cam(1))] {
        cam.width = width;
        cam.height = height;
        // The first frame includes the pipeline's first use.
        let mut best = f64::INFINITY;
        let mut frame = None;
        for _ in 0..3 {
            let f = gm.render(&gpu, &cam).unwrap();
            best = best.min(f.gpu_seconds);
            frame = Some(f);
        }
        let f = frame.unwrap();
        let hit = f.hits.iter().filter(|h| h.is_hit()).count();
        let steps: u64 = f.hits.iter().map(|h| h.steps as u64).sum();
        let max_steps = f.hits.iter().map(|h| h.steps).max().unwrap_or(0);
        println!(
            "{name}: {width}×{height}, {:.1} ms/frame ({:.2e} rays/s), {:.0}% hit, {:.1} steps/ray (max {max_steps})",
            1e3 * best,
            (width * height) as f64 / best,
            100.0 * hit as f64 / f.hits.len() as f64,
            steps as f64 / f.hits.len() as f64,
        );
        // Medians and 99th percentiles: lavapipe leaves a few counters
        // with garbage (~4e9 for rays that took a few steps).
        let st = gm.render_stats(&gpu, &cam).unwrap();
        let pct = |k: usize| {
            let mut v: Vec<f32> = st.hits.iter().map(|h| h.normal[k]).collect();
            v.sort_by(f32::total_cmp);
            (v[v.len() / 2], v[v.len() * 99 / 100])
        };
        let (e, w, c) = (pct(0), pct(1), pct(2));
        println!(
            "  per ray, median / 99th percentile: program words for exits {} / {}, \
             evaluated locating {} / {}, candidate cells {} / {}",
            e.0, e.1, w.0, w.1, c.0, c.1
        );
        if std::env::var_os("GPU_VIEW_CPU").is_some() {
            let lines: Vec<llevant::Line> = cam.rays().into_iter().flatten().collect();
            let t = Instant::now();
            model.first_hits(&lines, &vis, None).unwrap();
            let secs = t.elapsed().as_secs_f64();
            println!(
                "  CPU first_hit, {} threads: {:.1} ms/frame ({:.2e} rays/s)",
                rayon::current_num_threads(),
                1e3 * secs,
                lines.len() as f64 / secs
            );
        }
        if let Some(dir) = out {
            let file = format!("{dir}/gpu_{}.ppm", name.replace(' ', "_"));
            write_ppm(&file, &f).unwrap();
            println!("  wrote {file}");
        }
    }
}

fn outside(model: &Model) -> llevant::Camera {
    let boxes = check::level0_boxes(model);
    let pct = |v: &mut Vec<f64>, q: f64| {
        v.sort_by(f64::total_cmp);
        v[((v.len() - 1) as f64 * q) as usize]
    };
    let mut lo = [0.0; 3];
    let mut hi = [0.0; 3];
    for i in 0..3 {
        lo[i] = pct(&mut boxes.iter().map(|b| b.lo[i]).collect(), 0.05);
        hi[i] = pct(&mut boxes.iter().map(|b| b.hi[i]).collect(), 0.95);
    }
    let c = [0, 1, 2].map(|i| 0.5 * (lo[i] + hi[i]));
    let size = (0..3).map(|i| hi[i] - lo[i]).fold(0.0, f64::max);
    let dist = 1.6 * size;
    llevant::Camera {
        eye: [c[0] - 0.6 * dist, c[1] - 0.7 * dist, c[2] + 0.4 * dist],
        target: c,
        up: [0.0, 0.0, 1.0],
        fov_deg: 40.0,
        width: 1,
        height: 1,
        reach: 4.0 * dist,
        cut: llevant::Cutaway::none(),
        ortho: None,
    }
}

fn write_ppm(file: &str, f: &Frame) -> std::io::Result<()> {
    let mut data = format!("P6 {} {} 255\n", f.width, f.height).into_bytes();
    for h in &f.hits {
        let rgb = if !h.is_hit() {
            [255, 255, 255]
        } else if h.cell == llevant_gpu::NONE {
            [255, 0, 255]
        } else {
            let mut x = (h.cell as u64).wrapping_mul(0x9e3779b97f4a7c15);
            x ^= x >> 29;
            let base = [x as u8, (x >> 8) as u8, (x >> 16) as u8].map(|c| 64.0 + 0.75 * c as f32);
            // Normals face the ray; light from the eye, plus some ambient.
            let n = h.normal;
            let shade = if n == [0.0; 3] {
                1.0
            } else {
                0.35 + 0.65 * (n[2] * 0.3 + 0.7).abs()
            };
            base.map(|c| (c * shade).min(255.0) as u8)
        };
        data.extend_from_slice(&rgb);
    }
    std::fs::write(file, data)
}
