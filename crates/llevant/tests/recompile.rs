//! Recompiles: a model recompiled from an older version and completed saves
//! exactly the bytes of a fresh compile of the new input, whatever the edit,
//! and unchanged universes are reused.

mod common;

use common::{proptest_config, random_model, Rng};
use llevant::{GridMode, Model, Options};
use proptest::prelude::*;

#[derive(Clone, Copy, Debug)]
enum Edit {
    MoveSurface,
    MoveTransform,
    RemoveCell,
    AddCell,
    RenumberCell,
    Unfill,
    /// A cell naming a surface that is not there: unsupported.
    BreakCell,
    /// An unused surface after the first one: every shape after it moves.
    InsertSurface,
    /// A cell after the first one, in its universe: every cell after it
    /// moves.
    InsertCell,
}

const EDITS: [Edit; 9] = [
    Edit::MoveSurface,
    Edit::MoveTransform,
    Edit::RemoveCell,
    Edit::AddCell,
    Edit::RenumberCell,
    Edit::Unfill,
    Edit::BreakCell,
    Edit::InsertSurface,
    Edit::InsertCell,
];

fn pick<T: Copy>(v: &[T], r: &mut Rng) -> Option<T> {
    (!v.is_empty()).then(|| v[r.int(v.len())])
}

/// `input` with one edit of kind `e`, made through migjorn; `None` when the
/// model has nothing to make it on.
fn edit(input: &str, e: Edit, r: &mut Rng) -> Option<String> {
    edit_tree(input, e, r).map(|m| m.to_source())
}

/// [`edit`], as the edited migjorn tree (not parsed again).
fn edit_tree(input: &str, e: Edit, r: &mut Rng) -> Option<migjorn::Model> {
    let mut m = migjorn::Model::parse(input);
    let cells: Vec<i64> = m.cells().filter_map(|c| c.id()).collect();
    let surfaces: Vec<i64> = m.surfaces().filter_map(|s| s.id()).collect();
    let next = cells.iter().max().copied().unwrap_or(0) + 1;
    match e {
        Edit::MoveSurface => {
            let id = pick(&surfaces, r)?;
            let c = m.surface(id)?.coeffs().ok()?;
            let k = c.len().checked_sub(1)?;
            m.surface_mut(id)?.set_coeff(k, c[k] + 0.37).ok()?;
        }
        Edit::MoveTransform => {
            let ids: Vec<i64> = m.transforms().filter_map(|t| t.id()).collect();
            let id = pick(&ids, r)?;
            let mut v = m.transform(id)?.values().numbers().ok()?;
            *v.first_mut()? += 0.25;
            m.transform_mut(id)?.set_values(&v).ok()?;
        }
        Edit::RemoveCell => {
            let id = pick(&cells, r)?;
            m.cell_mut(id)?.remove();
        }
        Edit::AddCell => {
            let s = pick(&surfaces, r)?;
            m.add_cell(&format!("{next} 0 -{s} imp:n=1")).ok()?;
        }
        Edit::RenumberCell => {
            let id = pick(&cells, r)?;
            m.cell_mut(id)?.set_id(next).ok()?;
        }
        Edit::Unfill => {
            let filled: Vec<i64> = m
                .cells()
                .filter(|c| matches!(c.fill(), Ok(Some(_))))
                .filter_map(|c| c.id())
                .collect();
            let id = pick(&filled, r)?;
            m.cell_mut(id)?.set_fill(None).ok()?;
        }
        Edit::BreakCell => {
            let id = pick(&cells, r)?;
            m.cell_mut(id)?.set_geometry("-999999 : 1").ok()?;
        }
        Edit::InsertSurface => {
            let first = m.surface(*surfaces.first()?)?.slot();
            let id = surfaces.iter().max()? + 1;
            m.insert_card_after(first, migjorn::CardKind::Surface, &format!("{id} px 0.123"))
                .ok()?;
        }
        Edit::InsertCell => {
            let c = m.cell(*cells.first()?)?;
            let slot = c.slot();
            let u = c.universe().ok().flatten().unwrap_or(0);
            let s = pick(&surfaces, r)?;
            let text = format!("{next} 0 -{s} imp:n=1 u={u}");
            m.insert_card_after(slot, migjorn::CardKind::Cell, &text)
                .ok()?;
        }
    }
    Some(m)
}

fn saved(m: &Model) -> Vec<u8> {
    let mut b = Vec::new();
    m.save(&mut b).unwrap();
    b
}

/// Recompile `new` from `old` (complete, and staged) and compare with a
/// fresh compile; the universes reused.
fn check(old: &str, new: &str, opts: &Options, what: &str) -> usize {
    let fresh = Model::compile(new.as_bytes(), opts, None).unwrap();
    let want = saved(&fresh);
    let base = Model::compile(old.as_bytes(), opts, None).unwrap();
    let (staged, changed) = base.recompile(new.as_bytes(), opts, None).unwrap();
    assert!(
        saved(&staged.complete(None).unwrap()) == want,
        "{what}: recompiled model differs from a compile"
    );
    let base = Model::compile_staged(old.as_bytes(), opts, None).unwrap();
    let (staged, _) = base.recompile(new.as_bytes(), opts, None).unwrap();
    assert!(
        saved(&staged.complete(None).unwrap()) == want,
        "{what}: recompiled from a staged model, differs from a compile"
    );
    fresh.stats().units - changed.universes_rebuilt.len()
}

/// Recompile from `old` with the edited tree itself (no parse of its text,
/// as the viewer does) and compare with a fresh compile of its text.
fn check_parsed(old: &str, tree: &migjorn::Model, opts: &Options, what: &str) {
    let new = tree.to_source();
    let want = saved(&Model::compile(new.as_bytes(), opts, None).unwrap());
    let base = Model::compile_staged(old.as_bytes(), opts, None).unwrap();
    let (staged, _) = base.recompile_parsed(tree, opts, None).unwrap();
    assert!(
        saved(&staged.complete(None).unwrap()) == want,
        "{what}: recompiled from the edited tree, differs from a compile"
    );
}

#[test]
fn an_unchanged_input_reuses_everything() {
    let text = random_model(7);
    let opts = Options::default();
    let m = Model::compile(text.as_bytes(), &opts, None).unwrap();
    let (r, changed) = m.recompile(text.as_bytes(), &opts, None).unwrap();
    assert!(r.is_complete(), "nothing left to build");
    assert!(changed.universes_rebuilt.is_empty());
    assert!(changed.cells_changed.is_empty());
    assert!(changed.cells_removed.is_empty());
    assert!(saved(&r) == saved(&m));
}

#[test]
fn other_options_reuse_nothing() {
    let text = random_model(9);
    let m = Model::compile(text.as_bytes(), &Options::default(), None).unwrap();
    let opts = Options::default().with_tolerance(1e-7);
    let (r, changed) = m.recompile(text.as_bytes(), &opts, None).unwrap();
    assert_eq!(changed.universes_rebuilt.len(), r.stats().units);
    let fresh = Model::compile(text.as_bytes(), &opts, None).unwrap();
    assert!(saved(&r.complete(None).unwrap()) == saved(&fresh));
}

#[test]
fn zoo_edits_recompile_to_the_compiled_model() {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/mcnp-oracle/models"
    );
    let mut reused = 0;
    let mut r = Rng(0x5eed);
    for name in ["universes.i", "surfaces.i", "periodic.i"] {
        let old = std::fs::read_to_string(format!("{dir}/{name}")).unwrap();
        for e in EDITS {
            for _ in 0..3 {
                let Some(tree) = edit_tree(&old, e, &mut r) else {
                    continue;
                };
                check_parsed(
                    &old,
                    &tree,
                    &Options::default(),
                    &format!("{name} {e:?} tree"),
                );
                let new = tree.to_source();
                for grid in [GridMode::Auto, GridMode::Always] {
                    let opts = Options::default().with_grid(grid);
                    reused += check(&old, &new, &opts, &format!("{name} {e:?} {grid:?}"));
                    // And back: from the edited model (broken, for some).
                    check(&new, &old, &opts, &format!("{name} {e:?} {grid:?} back"));
                }
                // A second edit on top: the first one's model is the base.
                if let Some(next) = edit(&new, Edit::MoveSurface, &mut r) {
                    let opts = Options::default();
                    check(&new, &next, &opts, &format!("{name} {e:?} then a surface"));
                }
            }
        }
    }
    assert!(reused > 0, "no edit reused a universe");
}

proptest! {
    #![proptest_config(proptest_config(48))]

    #[test]
    fn random_edits_recompile_to_the_compiled_model(seed in 1u64.., k in 0usize..9, which in 1u64..) {
        let old = random_model(seed);
        let mut r = Rng(which | 1);
        if let Some(tree) = edit_tree(&old, EDITS[k], &mut r) {
            let new = tree.to_source();
            let opts = Options::default().with_grid(GridMode::Always);
            check_parsed(&old, &tree, &opts, &format!("{:?} tree", EDITS[k]));
            check(&old, &new, &opts, &format!("{:?}", EDITS[k]));
            check(&new, &old, &opts, &format!("{:?} back", EDITS[k]));
            if let Some(next) = edit(&new, Edit::MoveSurface, &mut r) {
                check(&new, &next, &opts, &format!("{:?} then a surface", EDITS[k]));
            }
        }
    }
}

/// Removing one of the first cells moves the indices of every cell (and
/// shape) after it: the universes after it are still reused, their octrees
/// rewritten to the new indices.
#[test]
fn edits_that_move_indices_reuse_universes() {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/mcnp-oracle/models"
    );
    for name in ["universes.i", "surfaces.i", "periodic.i"] {
        let old = std::fs::read_to_string(format!("{dir}/{name}")).unwrap();
        let ids: Vec<i64> = migjorn::Model::parse(&old)
            .cells()
            .filter_map(|c| c.id())
            .take(4)
            .collect();
        let mut reused = 0;
        for id in ids {
            let mut m = migjorn::Model::parse(&old);
            m.cell_mut(id).unwrap().remove();
            let new = m.to_source();
            for grid in [GridMode::Auto, GridMode::Always] {
                let opts = Options::default().with_grid(grid);
                reused += check(&old, &new, &opts, &format!("{name} without {id} {grid:?}"));
                check(
                    &new,
                    &old,
                    &opts,
                    &format!("{name} without {id} {grid:?} back"),
                );
            }
        }
        let units = Model::compile(old.as_bytes(), &Options::default(), None)
            .unwrap()
            .stats()
            .units;
        println!("{name}: {units} universes, {reused} reused over the edits");
        assert!(
            units == 1 || reused > 0,
            "{name}: no universe reused after a removed cell"
        );
    }
}
