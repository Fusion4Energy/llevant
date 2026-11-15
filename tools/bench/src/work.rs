//! `work <dir|model...> [--write file] [--check file]`: the engine's work
//! counters (`Model::work_points`, `Model::work_lines`) on a fixed workload,
//! as JSON. They count candidates, surface evaluations and crossings, not
//! time, so they are the same on every run and any thread count: a change
//! in them is a change in what the search structures do. The nightly job
//! compares them on the synthetic models (`tools/synthetic/gen_models.py`)
//! with `tools/bench/work-baseline.json`; a change that moves them on
//! purpose writes that file again (`--write`) in the same commit.
//!
//! The workload is drawn inside each model's extent with a fixed seed and
//! only arithmetic and square roots (the same bits on every platform).
//! Compilation runs again every time (no disk cache), with the options
//! of the environment (`LLEVANT_GRID`, `LLEVANT_RESIDUALS`: the nightly job
//! sets none). Boxes of curved
//! surfaces use the platform's sin and cos: the baseline is written and
//! checked on Windows.

use std::path::PathBuf;

use llevant::{Line, Model, Scope};
use serde_json::{json, Value};

use crate::rng::Rng;

const POINTS: usize = 20_000;
const LINES: usize = 2_000;

pub fn run(args: Vec<String>) {
    let mut models: Vec<PathBuf> = Vec::new();
    let (mut write, mut check) = (None, None);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--write" => write = Some(it.next().expect("--write <file>")),
            "--check" => check = Some(it.next().expect("--check <file>")),
            _ => {
                let p = PathBuf::from(a);
                if p.is_dir() {
                    let mut found: Vec<PathBuf> = std::fs::read_dir(&p)
                        .expect("model directory")
                        .map(|e| e.unwrap().path())
                        .filter(|f| f.extension().is_some_and(|x| x == "i" || x == "mcnp"))
                        .collect();
                    found.sort();
                    models.extend(found);
                } else {
                    models.push(p);
                }
            }
        }
    }
    assert!(
        !models.is_empty(),
        "usage: work <dir|model...> [--write file] [--check file]"
    );

    let mut all = serde_json::Map::new();
    for path in &models {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let model = Model::load(path, &crate::options(), None)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        all.insert(name, counters(&model));
    }
    let all = Value::Object(all);
    let text = serde_json::to_string_pretty(&all).unwrap() + "\n";
    print!("{text}");
    if let Some(f) = write {
        std::fs::write(&f, &text).unwrap();
        eprintln!("wrote {f}");
    }
    if let Some(f) = check {
        let base: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        let diffs = differences(&base, &all, String::new());
        if !diffs.is_empty() {
            eprintln!("work counters differ from {f}:");
            for d in &diffs {
                eprintln!("  {d}");
            }
            std::process::exit(1);
        }
        eprintln!("work counters as in {f}");
    }
}

/// The counters of one model's workload.
fn counters(model: &Model) -> Value {
    let b = model.extent(Scope::Contents, 64);
    assert!(!b.is_empty(), "the model has an empty extent");
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let inside = |r: &mut Rng| [0, 1, 2].map(|i| b.lo[i] + r.next() * (b.hi[i] - b.lo[i]));
    let points: Vec<[f64; 3]> = (0..POINTS).map(|_| inside(&mut rng)).collect();
    let diagonal = (0..3)
        .map(|i| (b.hi[i] - b.lo[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    let lines: Vec<Line> = (0..LINES)
        .map(|_| Line {
            origin: inside(&mut rng),
            dir: rng.direction(),
            t_min: 0.0,
            t_max: 0.25 * diagonal,
        })
        .collect();
    let level = |l: &llevant::LevelWork| {
        json!({
            "visits": l.visits,
            "candidates": l.candidates,
            "surface_evals": l.surface_evals,
            "surfaces_crossed": l.surfaces_crossed,
            "midpoints": l.midpoints,
        })
    };
    let work = |w: &llevant::Work| {
        json!({
            "items": w.items,
            "levels": w.levels.iter().map(level).collect::<Vec<_>>(),
        })
    };
    json!({
        "points": work(&model.work_points(&points)),
        "lines": work(&model.work_lines(&lines).unwrap()),
    })
}

/// Where `a` and `b` differ, as `path: a -> b`.
fn differences(a: &Value, b: &Value, at: String) -> Vec<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            keys.into_iter()
                .flat_map(|k| {
                    let (u, v) = (
                        x.get(k).unwrap_or(&Value::Null),
                        y.get(k).unwrap_or(&Value::Null),
                    );
                    differences(u, v, format!("{at}/{k}"))
                })
                .collect()
        }
        (Value::Array(x), Value::Array(y)) => (0..x.len().max(y.len()))
            .flat_map(|i| {
                let (u, v) = (
                    x.get(i).unwrap_or(&Value::Null),
                    y.get(i).unwrap_or(&Value::Null),
                );
                differences(u, v, format!("{at}/{i}"))
            })
            .collect(),
        _ if a == b => Vec::new(),
        _ => vec![format!("{at}: {a} -> {b}")],
    }
}
