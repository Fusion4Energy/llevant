//! Sides of a cell's surfaces at a point: f64, and the f32 values the shader
//! computes from the original and the recentred shape data.
//!
//! cargo run --release -p llevant-bench -- gpu-leaves <model> x y z
//!
//! The cell is the deepest of the point's CPU path (no lattices on the way).

use llevant::export::{SHAPE_PLANE, SHAPE_QUADRIC};
use llevant::Options;

fn f32_value(kind: u32, d: &[f64], p: [f64; 3]) -> Option<f32> {
    let d: Vec<f32> = d.iter().map(|&x| x as f32).collect();
    let p = p.map(|x| x as f32);
    match kind {
        SHAPE_PLANE => Some(d[0] * p[0] + d[1] * p[1] + d[2] * p[2] - d[3]),
        SHAPE_QUADRIC => {
            let v = [p[0] - d[0], p[1] - d[1], p[2] - d[2]];
            let ad = [
                d[3] * v[0] + d[6] * v[1] + d[8] * v[2],
                d[6] * v[0] + d[4] * v[1] + d[7] * v[2],
                d[8] * v[0] + d[7] * v[1] + d[5] * v[2],
            ];
            Some(
                v[0] * ad[0]
                    + v[1] * ad[1]
                    + v[2] * ad[2]
                    + d[9] * v[0]
                    + d[10] * v[1]
                    + d[11] * v[2]
                    + d[12],
            )
        }
        _ => None,
    }
}

pub fn run(args: Vec<String>) {
    let a: Vec<String> = args;
    let model = crate::load(&a[0], &Options::default()).unwrap();
    let p: Vec<f64> = a[1..4].iter().map(|s| s.parse().unwrap()).collect();
    let mut p = [p[0], p[1], p[2]];
    let label = model.classify_point(p);
    println!("{label:?}");
    let path = &label.path;
    for st in &path[..path.len() - 1] {
        match &model.cells()[st.cell as usize].fill {
            Some(llevant::Fill::Universe(t)) => p = t.apply(p),
            _ => panic!("lattice on the path"),
        }
    }
    let c = path.last().unwrap().cell as usize;
    let e = model.export();
    let rc = llevant_gpu::recentred_shapes(&e);
    let cell = &e.cells[c];
    println!("cell {} (index {c}) unit {}", cell.id, cell.unit);
    let code = &e.code[cell.code_start as usize..(cell.code_start + cell.code_len) as usize];
    for (s, want_pos) in llevant::export::leaves(code) {
        let s = s as usize;
        let side = model.shapes()[s].side(p, 0.0);
        let st = e.shape_start[s] as usize;
        let n = [4, 13, 19, 16][e.shape_kind[s] as usize];
        let orig = f32_value(e.shape_kind[s], &e.shape_data[st..st + n], p);
        let rec = f32_value(e.shape_kind[s], &rc[st..st + n], p);
        let flag = match (orig, rec) {
            (Some(o), Some(r)) if (o > 0.0) != (r > 0.0) => "  <-- differ",
            _ => "",
        };
        println!("  shape {s} kind {} want_pos {want_pos} f64 {side:?} f32 orig {orig:?} recentred {rec:?}{flag}", e.shape_kind[s]);
    }
}
