//! Live drags of a surface (`llevant::draft::Draft`), timed: starting one, its
//! steps (CPU), a snapshot for the CPU views and a slice of it, and on the
//! GPU the patch and 1920 × 1080 frames of the patched model against the
//! model's.
//!
//! cargo run --release -p llevant-bench -- drag <model> [surface...]
//!
//! Without surfaces: the surface used by the median and by the most cells,
//! of those used by a filled cell (envelopes) and of the others (fillers).
//! `DRAG_NO_GPU=1` leaves the GPU out.

use std::sync::Arc;
use std::time::Instant;

use llevant::draft::Draft;
use llevant::{Axis, Depth, Model, Options, Scope, Slice, Visibility};
use llevant_gpu::{check, Gpu, GpuModel};
use std::collections::HashMap as FxHashMap;

fn ms(t: Instant) -> f64 {
    1e3 * t.elapsed().as_secs_f64()
}

fn pct(v: &[f64], p: f64) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

/// The surfaces to drag: by the number of cells using them.
fn picks(m: &Model) -> Vec<(String, i64)> {
    let mut cells: FxHashMap<i64, (usize, bool)> = FxHashMap::default();
    for c in 0..m.cells().len() as u32 {
        let mut ids: Vec<i64> = m
            .cell_surface_leaves(c)
            .filter_map(|(s, _)| m.surface_id(s).map(|x| x.id))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let filled = m.cells()[c as usize].fill.is_some();
        for id in ids {
            let e = cells.entry(id).or_default();
            e.0 += 1;
            e.1 |= filled;
        }
    }
    let mut out = Vec::new();
    for (what, env) in [("envelope", true), ("filler", false)] {
        let mut v: Vec<(usize, i64)> = cells
            .iter()
            .filter(|(_, &(_, f))| f == env)
            .map(|(&id, &(n, _))| (n, id))
            .collect();
        v.sort_unstable();
        if let Some(&(n, id)) = v.get(v.len() / 2) {
            out.push((format!("{what}, median ({n} cells)"), id));
        }
        if let Some(&(n, id)) = v.last() {
            out.push((format!("{what}, most cells ({n})"), id));
        }
    }
    out
}

fn frames(gpu: &Gpu, gm: &GpuModel, cams: &[llevant::Camera]) -> Vec<f64> {
    cams.iter()
        .map(|cam| {
            (0..3)
                .map(|_| 1e3 * gm.render(gpu, cam).unwrap().gpu_seconds)
                .fold(f64::INFINITY, f64::min)
        })
        .collect()
}

fn show(v: &[f64]) -> String {
    let s: Vec<String> = v.iter().map(|x| format!("{x:.0}")).collect();
    s.join(", ")
}

pub fn run(args: Vec<String>) {
    let path = args.first().expect("usage: drag <model> [surface...]");
    let t = Instant::now();
    let model = Arc::new(crate::load(path, &Options::default()).unwrap());
    println!("load {:.2} s", t.elapsed().as_secs_f64());
    let given: Vec<(String, i64)> = args[1..]
        .iter()
        .map(|s| (format!("surface {s}"), s.parse().unwrap()))
        .collect();
    let picks = if given.is_empty() {
        picks(&model)
    } else {
        given
    };

    // Drafts begun from the input's tree, dropped before the GPU upload.
    let t = Instant::now();
    let text = std::fs::read(path).unwrap();
    let tree = migjorn::Model::parse(&String::from_utf8_lossy(&text));
    drop(text);
    println!("parse {:.2} s", t.elapsed().as_secs_f64());
    let mut drafts = Vec::new();
    for (what, id) in &picks {
        let t = Instant::now();
        match Draft::begin(model.clone(), &tree, *id) {
            Ok(d) => {
                println!(
                    "{what}: surface {id}, {} cells; begin {:.0} ms; octree off on the CPU in {} units, kept on the GPU in {}",
                    d.moved_cells().len(),
                    ms(t),
                    d.units_without_octree().len(),
                    d.units_kept_on_gpu().len()
                );
                drafts.push((what.clone(), d));
            }
            Err(e) => println!("{what}: {e}"),
        }
    }
    drop(tree);

    let region = model.extent(Scope::Contents, 64);
    let mid = 0.5 * (region.lo[2] + region.hi[2]);
    let slice = Slice::axis(Axis::Z, mid, &region, 1000);
    let slice_ms = |m: &Model| {
        (0..3)
            .map(|_| {
                let t = Instant::now();
                m.slice(&slice, Depth::All).unwrap();
                ms(t)
            })
            .fold(f64::INFINITY, f64::min)
    };
    println!("model: slice 1000 rows {:.0} ms", slice_ms(&model));
    for (what, d) in &mut drafts {
        let start = d.coeffs().to_vec();
        let k = start.len() - 1;
        let mut steps = Vec::new();
        let mut failed = 0;
        for mag in [0.1, 1.0, 10.0] {
            for i in -20..=20 {
                let mut c = start.clone();
                c[k] += mag * i as f64 / 20.0;
                let t = Instant::now();
                if d.set(&c).is_ok() {
                    steps.push(ms(t));
                } else {
                    failed += 1;
                }
            }
        }
        let t = Instant::now();
        let snap = d.snapshot();
        let snap_ms = ms(t);
        println!(
            "{what}: step p50 {:.1} ms, p95 {:.1} ms, max {:.1} ms ({} steps, {failed} refused); snapshot {snap_ms:.0} ms; slice {:.0} ms",
            pct(&steps, 0.5),
            pct(&steps, 0.95),
            pct(&steps, 1.0),
            steps.len(),
            slice_ms(&snap)
        );
    }

    if std::env::var_os("DRAG_NO_GPU").is_some() {
        return;
    }
    let gpu = Gpu::new().unwrap();
    let gm = GpuModel::upload(&gpu, &model).unwrap();
    let cams: Vec<llevant::Camera> = check::views(&model, 1)
        .into_iter()
        .map(|(_, mut c)| {
            c.width = 1920;
            c.height = 1080;
            c
        })
        .collect();
    let vis = Visibility::solid(&model);
    let skip = model.skip(&vis).unwrap();
    gm.set_visibility_with(&gpu, &model, &vis, &skip).unwrap();
    let base = frames(&gpu, &gm, &cams);
    println!("model: 1080p frames {} ms", show(&base));
    for (what, d) in &mut drafts {
        let mut c = d.coeffs().to_vec();
        *c.last_mut().unwrap() += 1.0;
        let Ok(p) = d.set(&c) else {
            println!("{what}: refused");
            continue;
        };
        let t = Instant::now();
        gm.patch(&gpu, &p).unwrap();
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let patch_ms = ms(t);
        let t = Instant::now();
        gm.set_visibility_with(&gpu, &model, &vis, &d.widen_skip(&skip))
            .unwrap();
        let vis_ms = ms(t);
        let f = frames(&gpu, &gm, &cams);
        let worst = f.iter().zip(&base).map(|(a, b)| a / b).fold(0.0, f64::max);
        println!(
            "{what}: patch {patch_ms:.1} ms ({} cells, {} BVH nodes), visibility {vis_ms:.0} ms; frames {} ms (worst {worst:.2}× the model's)",
            p.cells.len(),
            p.bvh_nodes.len(),
            show(&f)
        );
        gm.patch(&gpu, &d.revert_patch()).unwrap();
        gm.set_visibility_with(&gpu, &model, &vis, &skip).unwrap();
    }
}
