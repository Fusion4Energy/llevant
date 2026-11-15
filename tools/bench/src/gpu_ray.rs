//! One camera ray of a `gpu_check` view: CPU segments around the first hit
//! and the GPU hit.
//!
//! cargo run --release -p llevant-bench -- gpu-ray <model> <pixels> <view> <ray> [visibility] [shape]
//!
//! `visibility` indexes `gpu_check`'s list (0 all, 1 solid, 2 random half).

use llevant::Depth;
use llevant_gpu::check;

use llevant::{Line, Model, Options};
use llevant_gpu::{Gpu, GpuModel};

pub fn run(args: Vec<String>) {
    let a: Vec<String> = args;
    let model = crate::load(&a[0], &Options::default()).unwrap();
    let n: usize = a[1].parse().unwrap();
    let view: usize = a[2].parse().unwrap();
    let k: usize = a[3].parse().unwrap();
    let cam = check::views(&model, n).swap_remove(view).1;
    let r: Line = cam.ray(k % n, k / n).unwrap();
    let line = r;
    let vi: usize = a.get(4).map_or(1, |s| s.parse().unwrap());
    let vis = check::visibilities(&model).swap_remove(vi).1;
    let cpu = model.first_hits(&[line], &vis, None).unwrap();
    println!("ray {r:?}\ncpu t {} surface {}", cpu.t[0], cpu.surface[0]);
    let tr = model.trace(&[line], Depth::All).unwrap();
    for s in 0..tr.t0.len() {
        let l = &tr.labels[tr.ids[s] as usize];
        let cells: Vec<i64> = l
            .path
            .iter()
            .map(|st| model.cells()[st.cell as usize].id)
            .collect();
        let shown = l.path.iter().all(|st| vis.cells[st.cell as usize]);
        println!(
            "  {:10.5} {:10.5} {:?} {cells:?} visible {shown} in {} out {}",
            tr.t0[s], tr.t1[s], l.status, tr.surface_in[s], tr.surface_out[s]
        );
    }
    let gpu = Gpu::new().unwrap();
    let gm = GpuModel::new(&gpu, &model.export()).unwrap();
    gm.set_visibility(&gpu, &model, &vis).unwrap();
    println!("gpu {:?}", gm.trace_rays(&gpu, &[r])[0]);
    let (_, steps) = gm.debug_steps(&gpu, &r, 400);
    for w in steps {
        let cells: Vec<i64> = w[4..]
            .iter()
            .take_while(|&&c| c != u32::MAX)
            .map(|&c| model.cells()[c as usize].id)
            .collect();
        println!(
            "  t {:10.5} cross {:10.5} from {} depth {} status {} surf {} cells {cells:?}",
            f32::from_bits(w[0]),
            f32::from_bits(w[1]),
            w[2] & 0xff,
            (w[2] >> 8) & 0xff,
            w[2] >> 16,
            w[3] as i32
        );
    }
    if let Some(s) = a.get(5) {
        shape_info(&model, s.parse().unwrap());
    }
}

#[allow(dead_code)]
fn shape_info(model: &Model, s: usize) {
    let e = model.export();
    let st = e.shape_start[s] as usize;
    println!(
        "shape {s}: kind {} data {:?} id {:?}",
        e.shape_kind[s],
        &e.shape_data[st..st + 19.min(e.shape_data.len() - st)],
        model.surface_id(s as u32)
    );
}
