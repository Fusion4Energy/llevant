//! Benchmark workloads kept in files, so that runs before and after a change
//! see exactly the same points, lines and views even when the change moves
//! the cell boxes they are sampled from.
//!
//! With `LLEVANT_BENCH_SET=<dir>`, `cached(model, name, gen)` reads
//! `<dir>/<model stem>.<name>.f64` if it exists, else calls `gen` and writes
//! its result there. Without it, `gen` runs every time.

use std::path::{Path, PathBuf};

use llevant::{Aabb, Line};

pub fn cached(model: &str, name: &str, gen: impl FnOnce() -> Vec<f64>) -> Vec<f64> {
    let Some(dir) = std::env::var_os("LLEVANT_BENCH_SET") else {
        return gen();
    };
    let stem = Path::new(model)
        .file_stem()
        .map_or("model".into(), |s| s.to_string_lossy());
    let file: PathBuf = Path::new(&dir).join(format!("{stem}.{name}.f64"));
    if let Ok(bytes) = std::fs::read(&file) {
        println!("workload {name}: read {}", file.display());
        return bytes
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| f64::from_le_bytes(*c))
            .collect();
    }
    let v = gen();
    let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&file, bytes).unwrap();
    println!("workload {name}: wrote {}", file.display());
    v
}

pub fn points(model: &str, name: &str, gen: impl FnOnce() -> Vec<[f64; 3]>) -> Vec<[f64; 3]> {
    cached(model, name, || gen().into_iter().flatten().collect())
        .as_chunks::<3>()
        .0
        .iter()
        .map(|c| [c[0], c[1], c[2]])
        .collect()
}

pub fn lines(model: &str, name: &str, gen: impl FnOnce() -> Vec<Line>) -> Vec<Line> {
    let flat = |l: Line| {
        let [a, b, c] = l.origin;
        let [d, e, f] = l.dir;
        [a, b, c, d, e, f, l.t_min, l.t_max]
    };
    cached(model, name, || gen().into_iter().flat_map(flat).collect())
        .as_chunks::<8>()
        .0
        .iter()
        .map(|c| Line {
            origin: [c[0], c[1], c[2]],
            dir: [c[3], c[4], c[5]],
            t_min: c[6],
            t_max: c[7],
        })
        .collect()
}

pub fn boxes(model: &str, name: &str, gen: impl FnOnce() -> Vec<Aabb>) -> Vec<Aabb> {
    cached(model, name, || {
        gen()
            .into_iter()
            .flat_map(|b| [b.lo, b.hi].concat())
            .collect()
    })
    .as_chunks::<6>()
    .0
    .iter()
    .map(|c| Aabb {
        lo: [c[0], c[1], c[2]],
        hi: [c[3], c[4], c[5]],
    })
    .collect()
}
