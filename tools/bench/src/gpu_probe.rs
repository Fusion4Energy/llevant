//! GPU time and work per ray against the step cap, on the views of
//! `gpu_check`: where a model's rays spend their steps, and how long one
//! dispatch takes before it gets near Windows' GPU timeout (~2 s).
//!
//! cargo run --release -p llevant-bench -- gpu-probe <model> <size> <steps,...> [solid|voids|envelope|filler]
//!
//! `solid` (default) shows cells with a material or a fill; `voids` shows
//! every cell but the graveyard (`IMP:N=0`) and the level-0 cell holding the
//! eye; `envelope` and `filler` keep one level-0 envelope, or one cell of
//! its filler, and hide the rest (`bench_view`'s emptied views). `GPU_MAX_WORK` sets [`GpuModel::max_work`] (program words per ray).
//! `GPU_EXIT_BOUND=0` stops cells at the first crossing of any surface (A/B).

use llevant_gpu::check;

use llevant::{Model, Options, Visibility};
use llevant_gpu::{Gpu, GpuModel, Status};

pub fn run(args: Vec<String>) {
    let a: Vec<String> = args;
    let usage = "usage: gpu_probe <model> <size> <steps,...> [solid|voids|envelope|filler]";
    let model = crate::load(a.first().expect(usage), &Options::default()).unwrap();
    let n: usize = a.get(1).expect(usage).parse().unwrap();
    let caps: Vec<u32> = a
        .get(2)
        .expect(usage)
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    let mode = a.get(3).map_or("solid", String::as_str);
    let gpu = Gpu::new().unwrap();
    let mut gm = GpuModel::new(&gpu, &model.export()).unwrap();
    if let Ok(w) = std::env::var("GPU_MAX_WORK") {
        gm.max_work = w.parse().unwrap();
    }
    if std::env::var("GPU_EXIT_BOUND").as_deref() == Ok("0") {
        gm.set_constants(&gpu, &[("EXIT_BOUND", 0.0)]);
        println!("exit bound off: cells stop at the first crossing of any surface");
    }
    println!("max_work {} words", gm.max_work);
    let (kept_envelope, kept_filler, _, kept) = check::emptied(&model);
    println!("emptied views: {kept}");
    for (name, mut cam) in check::views(&model, n) {
        cam.width = n;
        cam.height = n;
        let vis = match mode {
            "voids" => voids_shown(&model, cam.eye),
            "envelope" => kept_envelope.clone(),
            "filler" => kept_filler.clone(),
            _ => Visibility::solid(&model),
        };
        gm.set_visibility(&gpu, &model, &vis).unwrap();
        for &cap in &caps {
            gm.max_steps = cap;
            let f = gm.render(&gpu, &cam).unwrap();
            let mut steps: Vec<u32> = f.hits.iter().map(|h| h.steps as u32).collect();
            steps.sort_unstable();
            let gave_up = f.hits.iter().filter(|h| h.status == Status::Steps).count();
            let hit = f.hits.iter().filter(|h| h.is_hit()).count();
            println!(
                "{name:12} cap {cap:5}: {:8.1} ms, hit {:5.1} %, steps median {} p99 {} max {}, gave up {gave_up}",
                1e3 * f.gpu_seconds,
                100.0 * hit as f64 / f.hits.len() as f64,
                steps[steps.len() / 2],
                steps[steps.len() * 99 / 100],
                steps[steps.len() - 1],
            );
            let st = gm.render_stats(&gpu, &cam).unwrap();
            let pct = |k: usize| {
                let mut v: Vec<f32> = st.hits.iter().map(|h| h.normal[k]).collect();
                v.sort_by(f32::total_cmp);
                (v[v.len() / 2], v[v.len() * 99 / 100], v[v.len() - 1])
            };
            let mut step: Vec<u32> = st.hits.iter().map(|h| h.instance).collect();
            step.sort_unstable();
            println!(
                "    per ray (median, p99, max): exit words {:?}, locate words {:?}, candidates {:?}, \
                 words of its longest step {:?}",
                pct(0),
                pct(1),
                pct(2),
                (step[step.len() / 2], step[step.len() * 99 / 100], step[step.len() - 1])
            );
        }
    }
}

/// Every cell visible except the graveyard and the level-0 cell holding `eye`.
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
