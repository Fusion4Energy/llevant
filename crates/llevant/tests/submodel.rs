//! Universes opened as models of their own (`Model::universe`): the fast
//! path against brute force, the cut, and the same answers as in the model
//! they come from.

use llevant::Depth;
use std::path::PathBuf;

use llevant::{Aabb, Cut, GridMode, Line, Model, Options, Status};

fn zoo(name: &str, grid: GridMode) -> Model {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/mcnp-oracle/models");
    let opts = Options::default().with_grid(grid);
    Model::load(dir.join(format!("{name}.i")), &opts, None).unwrap()
}

fn rng(seed: u64) -> impl FnMut() -> f64 {
    let mut s = 0x9e3779b97f4a7c15u64 ^ seed;
    move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// `b` grown by a fifth of its size on every side.
fn grown(b: &Aabb) -> Aabb {
    let pad: Vec<f64> = (0..3).map(|i| 0.2 * (b.hi[i] - b.lo[i])).collect();
    Aabb {
        lo: [0, 1, 2].map(|i| b.lo[i] - pad[i]),
        hi: [0, 1, 2].map(|i| b.hi[i] + pad[i]),
    }
}

/// Points and lines across the cut box and beyond it: fast path and brute
/// force agree, and beyond the box is the cut cell alone.
fn check_alone(sub: &Model, what: &str) {
    let cut = sub.cut_cell().expect("a cut");
    let b = sub.problem_box().expect("the cut box is the problem box");
    let around = grown(&b);
    let mut next = rng(sub.cells().len() as u64);
    let mut at = || [0, 1, 2].map(|i| around.lo[i] + next() * (around.hi[i] - around.lo[i]));
    let pts: Vec<[f64; 3]> = (0..20_000).map(|_| at()).collect();
    let fast = sub.classify(&pts, Depth::All);
    let slow = llevant::reference::classify(sub, &pts, Depth::All);
    let mut inside = 0;
    for (i, p) in pts.iter().enumerate() {
        let l = &fast.labels[fast.ids[i] as usize];
        assert_eq!(l, &slow.labels[slow.ids[i] as usize], "{what} {p:?}");
        if !b.contains(*p) {
            assert_eq!(l.status, Status::Inside, "{what} {p:?}");
            assert_eq!(l.cell(), Some(cut), "{what} {p:?}");
        } else if l.cell() != Some(cut) {
            inside += 1;
        }
    }
    assert!(
        inside > 1000,
        "{what}: only {inside} points in the universe's cells"
    );
    let lines: Vec<Line> = (0..400).map(|_| Line::between(at(), at())).collect();
    let fast = sub.trace(&lines, Depth::All).unwrap();
    let slow = llevant::reference::trace(sub, &lines, Depth::All).unwrap();
    for (i, l) in lines.iter().enumerate() {
        let a: Vec<_> = fast.segments(i).map(|k| fast.segment(k)).collect();
        let s: Vec<_> = slow.segments(i).map(|k| slow.segment(k)).collect();
        assert!(
            llevant::reference::traces_agree(sub, l, &a, &s),
            "{what} line {i}"
        );
        // The cut cell overlaps nothing, even along its planes.
        for x in &a {
            assert!(
                !(x.label.status == Status::Overlap && x.label.involved.contains(&cut)),
                "{what} line {i}: {x:?}"
            );
        }
    }
}

/// Points of the model that land in universe `u` (through fills without
/// lattices), classified in the universe opened on its own: the same path
/// from that level down, inside the cut box.
fn check_same_as_in_model(m: &Model, sub: &Model, u: i64, what: &str) {
    let unit = m.units().iter().position(|x| x.universe == u).unwrap() as u32;
    let b = sub.problem_box().unwrap();
    // In the boxes of the filled level-0 cells.
    let boxes: Vec<Aabb> = m.units()[0]
        .cells
        .iter()
        .map(|&c| m.cell(c))
        .filter(|c| c.fill.is_some() && c.bbox.is_finite())
        .map(|c| c.bbox)
        .collect();
    let mut next = rng(7);
    let pts: Vec<[f64; 3]> = (0..40_000)
        .map(|_| {
            let b = boxes[(next() * boxes.len() as f64) as usize % boxes.len()];
            [0, 1, 2].map(|i| b.lo[i] + next() * (b.hi[i] - b.lo[i]))
        })
        .collect();
    let res = m.classify(&pts, Depth::All);
    let mut compared = 0;
    let ids = |m: &Model, l: &llevant::Label| -> Vec<(i64, Option<[i32; 3]>)> {
        l.path
            .iter()
            .map(|s| (m.cell_id(s.cell), s.lattice))
            .collect()
    };
    for (i, &p) in pts.iter().enumerate() {
        let l = &res.labels[res.ids[i] as usize];
        let Some(k) = l.path.iter().position(|s| m.cell(s.cell).unit == unit) else {
            continue;
        };
        let q = m.point_at_level(l, k, p).unwrap();
        if !b.contains(q) {
            continue;
        }
        let got = sub.classify_point(q);
        assert_eq!(got.status, l.status, "{what} {p:?} -> {q:?}");
        assert_eq!(ids(sub, &got), ids(m, l)[k..], "{what} {p:?} -> {q:?}");
        compared += 1;
    }
    assert!(compared > 100, "{what}: only {compared} points compared");
}

#[test]
fn universes_alone_match_brute_force_and_the_model() {
    for name in ["universes"] {
        for grid in [GridMode::Auto, GridMode::Always] {
            let m = zoo(name, grid);
            let filled: Vec<i64> = m.units().iter().skip(1).map(|u| u.universe).collect();
            assert!(!filled.is_empty(), "{name}");
            for u in filled {
                let what = format!("{name} u={u} {grid:?}");
                let sub = m.universe(u, Cut::Own, grid).unwrap();
                assert!(
                    sub.diagnostics().is_empty(),
                    "{what}: {:?}",
                    sub.diagnostics()
                );
                assert_eq!(sub.units()[0].universe, u);
                check_alone(&sub, &what);
                check_same_as_in_model(&m, &sub, u, &what);
            }
        }
    }
}

#[test]
fn the_whole_model_can_be_cut_too() {
    let m = zoo("universes", GridMode::Auto);
    let b = m.bounds(llevant::Scope::Contents);
    let half = Aabb {
        lo: b.lo,
        hi: [0, 1, 2].map(|i| 0.5 * (b.lo[i] + b.hi[i])),
    };
    let sub = m.universe(0, Cut::Box(half), GridMode::Auto).unwrap();
    assert_eq!(sub.cells().len(), m.cells().len() + 1);
    check_alone(&sub, "universes u=0 cut in half");
    // Inside the box every answer is the model's.
    let mut next = rng(3);
    let pts: Vec<[f64; 3]> = (0..20_000)
        .map(|_| [0, 1, 2].map(|i| half.lo[i] + next() * (half.hi[i] - half.lo[i])))
        .collect();
    let (a, c) = (m.classify(&pts, Depth::All), sub.classify(&pts, Depth::All));
    for i in 0..pts.len() {
        let (x, y) = (&a.labels[a.ids[i] as usize], &c.labels[c.ids[i] as usize]);
        let id = |m: &Model, l: &llevant::Label| l.cell().map(|c| m.cell_id(c));
        if half.contains(pts[i]) && y.cell() != sub.cut_cell() {
            assert_eq!(
                (x.status, id(&m, x)),
                (y.status, id(&sub, y)),
                "{:?}",
                pts[i]
            );
        }
    }
}

#[test]
fn uncut_and_unknown_universes() {
    let m = zoo("universes", GridMode::Auto);
    let u = m.units()[1].universe;
    let sub = m.universe(u, Cut::None, GridMode::Auto).unwrap();
    assert!(sub.cut_cell().is_none());
    assert!(m.universe(-12345, Cut::Own, GridMode::Auto).is_err());
    let flat = Aabb {
        lo: [0.0; 3],
        hi: [1.0, 1.0, 0.0],
    };
    assert!(m.universe(u, Cut::Box(flat), GridMode::Auto).is_err());
}
