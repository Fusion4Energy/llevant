//! Staged models: compiling without the octrees and completing later gives
//! exactly the model a full compile does, and a staged model answers as the
//! complete one.

use llevant::{Depth, GridMode, Line, Model, Options, Scope};

fn zoo() -> Vec<std::path::PathBuf> {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/mcnp-oracle/models"
    );
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "i"))
        .collect();
    v.sort();
    assert!(!v.is_empty());
    v
}

fn saved(m: &Model) -> Vec<u8> {
    let mut b = Vec::new();
    m.save(&mut b).unwrap();
    b
}

#[test]
fn staged_then_completed_is_the_compiled_model() {
    for path in zoo() {
        let input = std::fs::read(&path).unwrap();
        for grid in [GridMode::Auto, GridMode::Always, GridMode::Off] {
            let opts = Options::default().with_grid(grid);
            let full = Model::compile(&input, &opts, None).unwrap();
            assert!(full.is_complete());
            let staged = Model::compile_staged(&input, &opts, None).unwrap();
            assert_eq!(staged.is_complete(), grid == GridMode::Off, "{path:?}");
            if grid != GridMode::Off {
                assert!(staged.save(&mut Vec::new()).is_err(), "{path:?}");
                assert_eq!(staged.stats().pending_units, staged.stats().units);
                assert_eq!(staged.stats().grid_units, 0);
            }
            let done = staged.complete(None).unwrap();
            assert!(done.is_complete());
            assert_eq!(done.stats().pending_units, 0);
            assert_eq!(done.stats().grid_units, full.stats().grid_units);
            assert!(
                saved(&full) == saved(&done),
                "{path:?} {grid:?}: completed staged model differs from a compile"
            );
        }
    }
}

#[test]
fn staged_models_answer_as_complete_ones() {
    let mut r = 0x9e3779b97f4a7c15u64;
    let mut f = move || {
        r ^= r << 13;
        r ^= r >> 7;
        r ^= r << 17;
        (r >> 11) as f64 / (1u64 << 53) as f64
    };
    let opts = Options::default().with_grid(GridMode::Always);
    for path in zoo() {
        let input = std::fs::read(&path).unwrap();
        let staged = Model::compile_staged(&input, &opts, None).unwrap();
        let done = staged.complete(None).unwrap();
        let b = done.bounds(Scope::Problem);
        if !b.is_finite() {
            continue;
        }
        let pts: Vec<[f64; 3]> = (0..4000)
            .map(|_| [0, 1, 2].map(|i| b.lo[i] + f() * (b.hi[i] - b.lo[i])))
            .collect();
        let (x, y) = (
            staged.classify(&pts, Depth::All),
            done.classify(&pts, Depth::All),
        );
        assert_eq!(x.ids, y.ids, "{path:?}");
        assert_eq!(x.labels, y.labels, "{path:?}");
        let lines: Vec<Line> = pts
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| Line::between(p[0], p[1]))
            .filter(|l| l.t_max > 0.0)
            .collect();
        let (x, y) = (
            staged.trace(&lines, Depth::All).unwrap(),
            done.trace(&lines, Depth::All).unwrap(),
        );
        for (i, l) in lines.iter().enumerate() {
            let a: Vec<_> = x.segments(i).map(|k| x.segment(k)).collect();
            let b: Vec<_> = y.segments(i).map(|k| y.segment(k)).collect();
            assert!(
                llevant::reference::traces_agree(&done, l, &a, &b),
                "{path:?}: line {i}"
            );
        }
    }
}
