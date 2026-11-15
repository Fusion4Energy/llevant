//! Cell instances: every place a cell occupies gets one id, ids and paths
//! map to each other, and the instance of a point's label is its path's.

use llevant::Depth;
use llevant::{Element, Fill, Model, Options, Status, Step};

mod common;
use common::{points, random_model_with};

fn zoo(name: &str) -> Model {
    let path = format!(
        "{}/../../tools/mcnp-oracle/models/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    Model::load(path, &Options::default(), None).unwrap()
}

/// Every path from level 0 down to each cell instance, by walking the
/// fills (each lattice element of a lattice cell's numbered range).
fn every_path(m: &Model) -> Vec<Vec<Step>> {
    fn walk(m: &Model, unit: u32, above: &mut Vec<Step>, out: &mut Vec<Vec<Step>>) {
        for &c in &m.units()[unit as usize].cells {
            let cell = m.cell(c);
            match &cell.fill {
                Some(Fill::Lattice(l)) => {
                    let Some(range) = m.instances().elements(c) else {
                        continue;
                    };
                    for k in 0..range.len() {
                        let ix = range.element(k);
                        above.push(Step {
                            cell: c,
                            lattice: Some(ix),
                        });
                        out.push(above.clone());
                        if let Element::Fill(t) = m.lattices()[*l as usize].element(ix) {
                            walk(m, t.unit(), above, out);
                        }
                        above.pop();
                    }
                }
                fill => {
                    above.push(Step {
                        cell: c,
                        lattice: None,
                    });
                    out.push(above.clone());
                    if let Some(Fill::Universe(t)) = fill {
                        walk(m, t.unit(), above, out);
                    }
                    above.pop();
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(m, 0, &mut Vec::new(), &mut out);
    out
}

/// Each instance found by walking the model has an id, no two the same,
/// and the id leads back to the path; without lattices filled with one
/// universe everywhere (whose ranges may have holes) the ids are exactly
/// `0 .. total`.
fn check_numbering(m: &Model, what: &str) {
    let inst = m.instances();
    let paths = every_path(m);
    let mut seen = vec![false; inst.total() as usize];
    for p in &paths {
        let id = inst
            .of(m, p)
            .unwrap_or_else(|| panic!("{what}: no id for {}", m.format_path(p)));
        assert!(!seen[id as usize], "{what}: id {id} twice");
        seen[id as usize] = true;
        assert_eq!(
            m.instance_path(id).as_ref(),
            Some(p),
            "{what}: path of {id}"
        );
        let text = m.format_path(p);
        assert_eq!(&m.parse_path(&text).unwrap(), p, "{what}: {text}");
    }
    let uniform = m.cells().iter().any(|c| {
        matches!(&c.fill, Some(Fill::Lattice(l)) if m.lattices()[*l as usize].fill_array().is_none())
    });
    if !uniform {
        assert_eq!(paths.len() as u64, inst.total(), "{what}: ids with holes");
    }
    // Counts per cell add up.
    let sum: u64 = (0..m.cells().len() as u32).map(|c| inst.count(c)).sum();
    assert_eq!(sum, inst.total());
}

#[test]
fn zoo_instances_are_numbered_once_each() {
    for name in ["universes.i", "surfaces.i", "periodic.i"] {
        let m = zoo(name);
        check_numbering(&m, name);
    }
    let m = zoo("universes.i");
    assert!(m.instances().total() > m.cells().len() as u64);
}

#[test]
fn random_model_instances_are_numbered_once_each() {
    for seed in (1..=48u64).step_by(2) {
        let text = random_model_with(seed, true);
        let m = Model::from_text(&text, &Options::default());
        check_numbering(&m, &format!("seed {seed}"));
    }
}

/// The instance of a point is the one its path names, and its path comes
/// back from the id.
#[test]
fn points_know_their_instance() {
    for seed in (1..=24u64).step_by(2) {
        let m = Model::from_text(&random_model_with(seed, true), &Options::default());
        let c = m.classify(&points(seed, 3000), Depth::All);
        let mut deep = 0;
        for l in &c.labels {
            let id = m.instance(l);
            if l.status != Status::Inside {
                assert_eq!(id, None);
                continue;
            }
            let id = id.unwrap_or_else(|| panic!("seed {seed}: no instance for {}", m.describe(l)));
            assert_eq!(m.instance_path(id).unwrap(), l.path);
            assert!(m.instances().ids(l.cell().unwrap()).contains(&id));
            deep += (l.path.len() > 2) as usize;
        }
        assert!(deep > 0, "seed {seed}: no point below level 1");
    }
}

#[test]
fn paths_that_do_not_exist_are_refused() {
    let m = zoo("universes.i");
    let some = every_path(&m).into_iter().find(|p| p.len() >= 2).unwrap();
    let text = m.format_path(&some);
    assert!(text.contains(" < "), "{text}");
    // The deepest cell alone is not at level 0.
    let deepest = text.split(" < ").next().unwrap();
    assert!(m.parse_path(deepest).is_err());
    assert!(m.parse_path("999999").is_err());
    assert!(m.parse_path("1[0 0").is_err());
    assert_eq!(m.instance_path(m.instances().total()), None);
}

/// A tally chain picks the instances whose paths have its levels, checked
/// against every path walked: the whole path as a chain is its one
/// instance; a cell alone is all of its instances; the cell below its
/// level-0 cell (levels left out) is every instance of it under that cell;
/// a lattice level without its element, every element.
#[test]
fn tally_chains_match_their_instances() {
    let models = ["universes.i", "periodic.i"]
        .map(|n| (n.to_string(), zoo(n)))
        .into_iter()
        .chain((1..=11u64).step_by(2).map(|seed| {
            let m = Model::from_text(&random_model_with(seed, true), &Options::default());
            (format!("seed {seed}"), m)
        }));
    let mut deep = 0;
    for (what, m) in models {
        let inst = m.instances();
        let paths = every_path(&m);
        assert!(inst.matching(&m, &[]).is_empty());
        for p in paths.iter().filter(|p| p.len() >= 2).step_by(7).take(40) {
            deep += 1;
            let text = m.format_path(p);
            let chain: Vec<Vec<Step>> = p.iter().rev().map(|s| vec![*s]).collect();
            assert_eq!(
                inst.matching(&m, &chain),
                vec![inst.of(&m, p).unwrap()],
                "{what}: {text}"
            );
            let last = *p.last().unwrap();
            let every: Vec<u64> = inst.ids(last.cell).collect();
            let alone = inst.matching(&m, &[vec![last]]);
            if last.lattice.is_none() {
                assert_eq!(alone, every, "{what}: {text} alone");
            }
            let sorted = |mut v: Vec<u64>| {
                v.sort_unstable();
                v
            };
            let under = sorted(
                paths
                    .iter()
                    .filter(|q| q.last() == Some(&last) && q[0] == p[0])
                    .map(|q| inst.of(&m, q).unwrap())
                    .collect(),
            );
            assert_eq!(
                inst.matching(&m, &[vec![last], vec![p[0]]]),
                under,
                "{what}: {text} skipping"
            );
            if let Some(k) = (0..p.len() - 1).find(|&k| p[k].lattice.is_some()) {
                let mut any = chain.clone();
                any[p.len() - 1 - k][0].lattice = None;
                let expect = sorted(
                    paths
                        .iter()
                        .filter(|q| {
                            q.len() == p.len()
                                && (0..p.len()).all(|j| j == k || q[j] == p[j])
                                && q[k].cell == p[k].cell
                        })
                        .map(|q| inst.of(&m, q).unwrap())
                        .collect(),
                );
                assert!(!expect.is_empty());
                assert_eq!(
                    inst.matching(&m, &any),
                    expect,
                    "{what}: {text} any element"
                );
            }
        }
        // A union at the first level is both cells' instances.
        let two: Vec<&Vec<Step>> = paths.iter().filter(|p| p.len() == 1).take(2).collect();
        if let [a, b] = two[..] {
            let mut expect: Vec<u64> = vec![inst.of(&m, a).unwrap(), inst.of(&m, b).unwrap()];
            expect.sort_unstable();
            assert_eq!(
                inst.matching(&m, &[vec![a[0], b[0]]]),
                expect,
                "{what}: union"
            );
        }
    }
    assert!(deep > 20);
}

/// A bin of a filled cell holds everything inside it, and a bin of a
/// deeper cell takes its instances back: each instance gets the bin of the
/// deepest cell on its path that some bin scores.
#[test]
fn innermost_bins_take_the_deepest_cell() {
    for seed in (1..=11u64).step_by(2) {
        let m = Model::from_text(&random_model_with(seed, true), &Options::default());
        let inst = m.instances();
        let paths = every_path(&m);
        let Some(deep) = paths.iter().find(|p| p.len() >= 3) else {
            continue;
        };
        let (top, low) = (deep[0], *deep.last().unwrap());
        let low = Step {
            lattice: None,
            ..low
        };
        let chains = vec![vec![vec![top]], vec![vec![low]]];
        let bins = inst.innermost_bins(&m, &chains);
        assert_eq!(bins.len() as u64, inst.total());
        for p in &paths {
            let id = inst.of(&m, p).unwrap() as usize;
            let expect = if p.last().unwrap().cell == low.cell {
                1
            } else if p[0] == top {
                0
            } else {
                llevant::NO_BIN
            };
            assert_eq!(bins[id], expect, "seed {seed}: {}", m.format_path(p));
        }
        assert!(inst
            .innermost_bins(&m, &[])
            .iter()
            .all(|&b| b == llevant::NO_BIN));
    }
}
