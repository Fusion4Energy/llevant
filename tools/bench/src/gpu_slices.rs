//! Frames traced in slices against frames traced to the end in one
//! dispatch, on `gpu_check`'s views: pixels that differ, the time of the
//! whole frame, and the time of each slice (one per submission, waited
//! for), for each slice size.
//!
//! cargo run --release -p llevant-bench -- gpu-slices <model> <solid|query> <size> <words,...>
//!
//! `query` isolates cells as the viewer does (e.g. "m 302"). `GPU_CAPACITY`
//! and `GPU_TAIL_WORK` set [`GpuModel::capacity`] and
//! [`GpuModel::tail_work`].

use llevant_gpu::check;

use std::time::Instant;

use llevant::{Options, Visibility};
use llevant_gpu::{tile_blocks, Gpu, GpuModel, HitBuffer, Progress, Tile};
use llevant_scene::hiding::Hiding;

pub fn run(args: Vec<String>) {
    let a: Vec<String> = args;
    let usage = "usage: gpu_slices <model> <solid|query> <size> <words,...>";
    let model = crate::load(a.first().expect(usage), &Options::default()).unwrap();
    let vis = match a.get(1).expect(usage).as_str() {
        "solid" => Visibility::solid(&model),
        q => Hiding {
            isolate: Some(q.parse().unwrap()),
            ..Hiding::default()
        }
        .visibility(&model)
        .unwrap(),
    };
    let n: usize = a.get(2).expect(usage).parse().unwrap();
    let sizes: Vec<u32> = a
        .get(3)
        .expect(usage)
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    let gpu = Gpu::new().unwrap();
    let mut gm = GpuModel::upload(&gpu, &model).unwrap();
    if let Ok(c) = std::env::var("GPU_CAPACITY") {
        gm.capacity = c.parse().unwrap();
    }
    if let Ok(c) = std::env::var("GPU_TAIL_WORK") {
        gm.tail_work = c.parse().unwrap();
    }
    println!("capacity {} rays", gm.capacity);
    gm.set_visibility(&gpu, &model, &vis).unwrap();
    for (name, mut cam) in check::views(&model, n) {
        cam.width = n;
        cam.height = n;
        gm.slice_work = u32::MAX;
        gm.render(&gpu, &cam).unwrap();
        let whole = gm.render(&gpu, &cam).unwrap();
        println!(
            "{name}, {n}²: one dispatch {:.1} ms",
            1e3 * whole.gpu_seconds
        );
        for &k in &sizes {
            gm.slice_work = k;
            let f = gm.render(&gpu, &cam).unwrap();
            let differ = f
                .hits
                .iter()
                .zip(&whole.hits)
                .filter(|(a, b)| a.to_words() != b.to_words())
                .count();
            let cells = f
                .hits
                .iter()
                .zip(&whole.hits)
                .filter(|(a, b)| a.cell != b.cell || a.status != b.status)
                .count();
            let slices = slice_times(&gpu, &gm, &cam);
            let mut sorted = slices.clone();
            sorted.sort_by(f64::total_cmp);
            println!(
                "  {k:6} words: frame {:7.1} ms, {} slices (median {:.2} ms, max {:.2} ms, sum {:.1} ms); \
                 {differ} pixels differ, {cells} in cell or status",
                1e3 * f.gpu_seconds,
                slices.len(),
                1e3 * sorted[sorted.len() / 2],
                1e3 * sorted[sorted.len() - 1],
                1e3 * slices.iter().sum::<f64>()
            );
        }
    }
}

/// Seconds of each slice of the camera's frame, one per submission.
fn slice_times(gpu: &Gpu, gm: &GpuModel, cam: &llevant::Camera) -> Vec<f64> {
    let (w, h) = (cam.width as u32, cam.height as u32);
    let out = HitBuffer::new(gpu, w, h);
    let job = gm
        .camera_job(
            gpu,
            cam,
            &tile_blocks(Tile::full(w, h)),
            &out,
            gm.max_steps,
            gm.max_work,
        )
        .unwrap();
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: job.progress_bytes(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut times = Vec::new();
    loop {
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        job.encode_slices(&mut enc, 1);
        job.encode_progress(&mut enc, &readback, 0);
        let t = Instant::now();
        gpu.queue.submit([enc.finish()]);
        llevant_gpu::map_for_reading(&gpu.device, &readback).unwrap();
        times.push(t.elapsed().as_secs_f64());
        let words: Vec<u32> =
            bytemuck::cast_slice(&readback.get_mapped_range(..).unwrap()).to_vec();
        readback.unmap();
        let p = Progress::from_words(&words);
        if std::env::var_os("GPU_SLICES_VERBOSE").is_some() {
            println!(
                "    slice {:3}: {:7.2} ms, {} workgroups next, {} rays done",
                times.len(),
                1e3 * times[times.len() - 1],
                p.groups,
                p.rays_done
            );
        }
        if p.done {
            return times;
        }
    }
}
