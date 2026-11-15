//! Drafts: after any sequence of steps, a draft's model classifies points
//! exactly as brute force does on a fresh compile of the edited input,
//! traces lines as it does (as `reference::traces_agree` says), keeps cell
//! boxes that contain the fresh ones, and cannot be saved.

mod common;

use std::sync::Arc;

use common::{random_model, Rng};
use llevant::draft::Draft;
use llevant::{Aabb, Cut, Depth, GridMode, Line, Model, Options, Scope};

fn zoo(name: &str) -> String {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/mcnp-oracle/models"
    );
    std::fs::read_to_string(format!("{dir}/{name}")).unwrap()
}

/// Coefficients moved a little: one of them, or all of them.
fn nudge(c: &[f64], r: &mut Rng) -> Vec<f64> {
    let mut v = c.to_vec();
    let all = r.int(3) == 0;
    let k = r.int(v.len());
    for (i, x) in v.iter_mut().enumerate() {
        if all || i == k {
            *x += r.range(-1.0, 1.0) * (0.05 + 0.2 * x.abs().min(5.0));
        }
    }
    v
}

/// Points and lines over the model's contents, a little beyond.
fn workload(m: &Model, seed: u64) -> (Vec<[f64; 3]>, Vec<Line>) {
    let mut b = m.extent(Scope::Problem, 24);
    if b.is_empty() || !b.is_finite() {
        b = Aabb {
            lo: [-12.0; 3],
            hi: [12.0; 3],
        };
    }
    let pad: Vec<f64> = (0..3).map(|i| 0.1 * (b.hi[i] - b.lo[i]) + 0.5).collect();
    let mut r = Rng(seed | 1);
    let mut p = || [0, 1, 2].map(|i| r.range(b.lo[i] - pad[i], b.hi[i] + pad[i]));
    let pts: Vec<[f64; 3]> = (0..1500).map(|_| p()).collect();
    let lines: Vec<Line> = (0..120)
        .map(|_| {
            let (a, e) = (p(), p());
            Line::between(a, e)
        })
        .collect();
    (pts, lines)
}

fn contains(outer: &Aabb, b: &Aabb) -> bool {
    let tol = |x: f64| 1e-6 * (1.0 + x.abs());
    b.is_empty()
        || (0..3)
            .all(|i| outer.lo[i] <= b.lo[i] + tol(b.lo[i]) && b.hi[i] <= outer.hi[i] + tol(b.hi[i]))
}

/// The draft as it is against a fresh compile of `text`.
fn check(draft: &Draft, text: &str, opts: &Options, what: &str, seed: u64) {
    let fresh = Model::compile(text.as_bytes(), opts, None).unwrap();
    check_against(draft, &fresh, what, seed);
}

/// The draft as it is against `fresh`, the model it should answer as.
fn check_against(draft: &Draft, fresh: &Model, what: &str, seed: u64) {
    let snap = draft.snapshot();
    assert!(!snap.is_complete(), "{what}: a draft is pending");
    assert!(
        snap.save(&mut Vec::new()).is_err(),
        "{what}: a draft cannot be saved"
    );
    assert_eq!(snap.cells().len(), fresh.cells().len());
    for (k, (a, b)) in snap.cells().iter().zip(fresh.cells()).enumerate() {
        assert!(
            contains(&a.bbox, &b.bbox),
            "{what}: cell {} box {:?} does not contain {:?} (cell index {k})",
            a.id,
            a.bbox,
            b.bbox
        );
    }
    let (pts, lines) = workload(fresh, seed);
    let fast = snap.classify(&pts, Depth::All);
    let slow = llevant::reference::classify(fresh, &pts, Depth::All);
    for (i, p) in pts.iter().enumerate() {
        assert_eq!(
            fast.labels[fast.ids[i] as usize], slow.labels[slow.ids[i] as usize],
            "{what}: point {p:?}"
        );
    }
    let fast = snap.trace(&lines, Depth::All).unwrap();
    let slow = llevant::reference::trace(fresh, &lines, Depth::All).unwrap();
    for (i, l) in lines.iter().enumerate() {
        let a: Vec<_> = fast.segments(i).map(|k| fast.segment(k)).collect();
        let b: Vec<_> = slow.segments(i).map(|k| slow.segment(k)).collect();
        assert!(
            llevant::reference::traces_agree(fresh, l, &a, &b),
            "{what}: line {i} {l:?}"
        );
    }
}

/// Drag up to `max` surfaces of `text` through a few steps each, checking
/// every step, and back to where they started. Returns the drafts made.
fn drag_all(text: &str, opts: &Options, max: usize, r: &mut Rng, what: &str) -> usize {
    let base = Arc::new(Model::compile(text.as_bytes(), opts, None).unwrap());
    let tree = migjorn::Model::parse(text);
    let ids: Vec<i64> = tree.surfaces().filter_map(|s| s.id()).collect();
    let mut made = 0;
    for &id in ids.iter().take(max) {
        let Ok(mut draft) = Draft::begin(base.clone(), &tree, id) else {
            continue;
        };
        made += 1;
        let start = draft.coeffs().to_vec();
        let mut edited = migjorn::Model::parse(text);
        for step in 0..3 {
            let c = nudge(&start, r);
            if draft.set(&c).is_err() {
                continue;
            }
            edited.surface_mut(id).unwrap().set_coeffs(&c).unwrap();
            let w = format!("{what} surface {id} step {step} {c:?}");
            check(&draft, &edited.to_source(), opts, &w, r.int(1 << 30) as u64);
        }
        draft.set(&start).unwrap();
        check(&draft, text, opts, &format!("{what} surface {id} back"), 7);
        assert!(!draft.revert_patch().shapes.is_empty());
    }
    made
}

#[test]
fn zoo_drafts_match_fresh_compiles() {
    let mut r = Rng(0xd4af7);
    for name in ["universes.i", "surfaces.i", "periodic.i"] {
        let text = zoo(name);
        for grid in [GridMode::Auto, GridMode::Always] {
            let opts = Options::default().with_grid(grid);
            let made = drag_all(&text, &opts, 40, &mut r, &format!("{name} {grid:?}"));
            assert!(made > 0, "{name}: no surface could be dragged");
        }
    }
}

/// Drafts of a universe cut out on its own (the viewer's filler opened
/// alone): each step answers as that universe cut, to the same box, from a
/// fresh compile of the edited input.
#[test]
fn universe_drafts_match_fresh_cuts() {
    let mut r = Rng(0x5ab);
    let text = zoo("universes.i");
    let tree = migjorn::Model::parse(&text);
    let mut made = 0;
    for grid in [GridMode::Auto, GridMode::Always] {
        let opts = Options::default().with_grid(grid);
        let whole = Model::compile(text.as_bytes(), &opts, None).unwrap();
        let mut universes: Vec<i64> = whole.units().iter().map(|u| u.universe).collect();
        universes.retain(|&u| u != 0);
        universes.sort_unstable();
        universes.dedup();
        for u in universes {
            let Ok(sub) = whole.universe(u, Cut::Own, grid) else {
                continue;
            };
            let b = sub.problem_box().expect("cut to a box");
            let sub = Arc::new(sub);
            for id in tree.surfaces().filter_map(|s| s.id()) {
                let Ok(mut draft) = Draft::begin(sub.clone(), &tree, id) else {
                    continue;
                };
                made += 1;
                let start = draft.coeffs().to_vec();
                let mut edited = migjorn::Model::parse(&text);
                for step in 0..2 {
                    let c = nudge(&start, &mut r);
                    if draft.set(&c).is_err() {
                        continue;
                    }
                    edited.surface_mut(id).unwrap().set_coeffs(&c).unwrap();
                    let fresh = Model::compile(edited.to_source().as_bytes(), &opts, None)
                        .unwrap()
                        .universe(u, Cut::Box(b), grid)
                        .unwrap();
                    let w = format!("u={u} {grid:?} surface {id} step {step} {c:?}");
                    check_against(&draft, &fresh, &w, r.int(1 << 30) as u64);
                }
            }
        }
    }
    assert!(made > 10, "only {made} drafts");
}

#[test]
fn random_model_drafts_match_fresh_compiles() {
    let mut r = Rng(0x0d4a);
    let mut made = 0;
    for seed in 0..12 {
        let text = random_model(seed);
        let opts = Options::default().with_grid(GridMode::Always);
        made += drag_all(&text, &opts, 6, &mut r, &format!("random {seed}"));
    }
    assert!(made > 20, "only {made} drafts");
}

#[test]
fn a_bad_step_leaves_the_draft_as_it_was() {
    let text = zoo("universes.i");
    let opts = Options::default().with_grid(GridMode::Always);
    let base = Arc::new(Model::compile(text.as_bytes(), &opts, None).unwrap());
    let tree = migjorn::Model::parse(&text);
    let id = tree
        .surfaces()
        .filter_map(|s| s.id())
        .find(|&id| Draft::begin(base.clone(), &tree, id).is_ok())
        .unwrap();
    let mut draft = Draft::begin(base.clone(), &tree, id).unwrap();
    let mut c = draft.coeffs().to_vec();
    *c.last_mut().unwrap() += 0.3;
    draft.set(&c).unwrap();
    assert!(draft.set(&[]).is_err(), "no coefficients");
    let mut edited = migjorn::Model::parse(&text);
    edited.surface_mut(id).unwrap().set_coeffs(&c).unwrap();
    check(&draft, &edited.to_source(), &opts, "after a bad step", 3);
}

#[test]
fn surfaces_that_cannot_be_dragged_say_why() {
    let text = zoo("universes.i");
    let base = Arc::new(Model::compile(text.as_bytes(), &Options::default(), None).unwrap());
    let tree = migjorn::Model::parse(&text);
    let e = Draft::begin(base.clone(), &tree, 987_654).err().unwrap();
    assert!(e.to_string().contains("not in the input"), "{e}");
}
