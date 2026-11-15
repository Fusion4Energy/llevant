//! GPU frames with levels and ghosts, timed: the whole model solid, at
//! level 0 and at level 1; one envelope kept with what fills it, without
//! and with the envelope a ghost; and the universe filling it opened on its
//! own (`Model::universe`, cut to what it holds), framed alike.
//!
//! cargo run --release -p llevant-bench -- gpu-levels <model> [width height]

use llevant_gpu::check;

use std::time::Instant;

use llevant::{Camera, Cut, Fill, GridMode, Model, Options, Scope, Visibility};
use llevant_gpu::{Gpu, GpuModel};

/// Best of three frames (the first includes the pipeline's first use), ms,
/// and the share of rays that hit and that crossed a ghost.
fn time(gpu: &Gpu, gm: &GpuModel, cam: &Camera) -> (f64, f64, f64) {
    let mut best = f64::INFINITY;
    let mut last = None;
    for _ in 0..3 {
        let f = gm.render(gpu, cam).unwrap();
        best = best.min(f.gpu_seconds);
        last = Some(f);
    }
    let f = last.unwrap();
    let n = f.hits.len() as f64;
    let hit = f.hits.iter().filter(|h| h.is_hit()).count() as f64 / n;
    let ghosted = f.ghosts.iter().filter(|g| g.count > 0).count() as f64 / n;
    (1e3 * best, hit, ghosted)
}

pub fn run(args: Vec<String>) {
    let a: Vec<String> = args;
    let path = a.first().expect("usage: gpu_levels <model> [width height]");
    let width: usize = a.get(1).map_or(1920, |s| s.parse().unwrap());
    let height: usize = a.get(2).map_or(1080, |s| s.parse().unwrap());
    let model = crate::load(path, &Options::default()).unwrap();
    let gpu = Gpu::new().unwrap();
    println!("adapter: {} ({:?})", gpu.info.name, gpu.info.backend);
    let gm = GpuModel::new(&gpu, &model.export()).unwrap();
    let report = |name: &str, gm: &GpuModel, m: &Model, vis: &Visibility, cam: &Camera| {
        let t = Instant::now();
        gm.set_visibility(&gpu, m, vis).unwrap();
        let set = t.elapsed().as_secs_f64();
        let (ms, hit, ghosted) = time(&gpu, gm, cam);
        println!(
            "{name}: {ms:.1} ms/frame, {:.0}% hit, {:.0}% through a ghost (visibility set in {:.0} ms)",
            100.0 * hit,
            100.0 * ghosted,
            1e3 * set
        );
    };

    let whole = {
        let mut c = check::views(&model, 1).swap_remove(0).1;
        c.width = width;
        c.height = height;
        c
    };
    let solid = Visibility::solid(&model);
    for (name, level) in [
        ("whole, solid", None),
        ("whole, level 0", Some(0)),
        ("whole, level 1", Some(1)),
    ] {
        let vis = Visibility {
            level,
            ..solid.clone()
        };
        report(name, &gm, &model, &vis, &whole);
    }

    let (envelope, _, b, what) = check::emptied(&model);
    println!("{what}");
    let e = model.units()[0]
        .cells
        .iter()
        .copied()
        .find(|&c| envelope.cells[c as usize] && model.cell(c).fill.is_some())
        .expect("a kept envelope");
    let Some(Fill::Universe(t)) = &model.cell(e).fill else {
        panic!("the envelope is filled by a universe");
    };
    let u = model.units()[t.unit() as usize].universe;
    let framed = Camera::fit(&b, [1.0, 0.7, -0.5], width, height);
    report("envelope kept", &gm, &model, &envelope, &framed);
    let mut ghosted = envelope.clone();
    ghosted.ghost(&model, &[model.cell(e).id]).unwrap();
    report("envelope kept, a ghost", &gm, &model, &ghosted, &framed);
    let mut shells = solid.clone();
    shells.ghost(&model, &[model.cell(e).id]).unwrap();
    report(
        "whole, solid, the envelope a ghost",
        &gm,
        &model,
        &shells,
        &whole,
    );

    let t = Instant::now();
    let sub = model.universe(u, Cut::Own, GridMode::Auto).unwrap();
    let opened = t.elapsed().as_secs_f64();
    let sgm = GpuModel::new(&gpu, &sub.export()).unwrap();
    let sb = sub.extent(Scope::Contents, 64);
    let cam = Camera::fit(&sb, [1.0, 0.7, -0.5], width, height);
    println!("u={u} opened on its own in {opened:.2} s");
    report(
        "filler on its own",
        &sgm,
        &sub,
        &Visibility::solid(&sub),
        &cam,
    );
}
