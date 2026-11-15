//! Volumes on a grid of lines: analytic shapes, planted defects, determinism.

use std::f64::consts::PI;

use llevant::{
    Aabb, BinTarget, BinVolumes, Depth, Group, GroupBy, Model, Options, Status, Step, VolumeGrid,
    VolumeTarget, Volumes,
};

fn model(text: &str) -> Model {
    let m = Model::from_text(text, &Options::default());
    assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
    m
}

fn cube(lo: f64, hi: f64) -> Aabb {
    Aabb {
        lo: [lo; 3],
        hi: [hi; 3],
    }
}

/// Volume of the cell with MCNP number `id` (the deepest cell of each
/// label inside it).
fn cell_volume(m: &Model, v: &Volumes, id: i64) -> llevant::Volume {
    v.group_by(|l| {
        (l.status == Status::Inside && l.cell().map(|c| m.cell_id(c)) == Some(id)).then_some(())
    })
    .pop()
    .expect("cell not met")
    .1
}

fn assert_close(name: &str, got: &llevant::Volume, exact: f64, rel: f64) {
    let err = (got.estimate - exact).abs();
    assert!(
        err <= rel * exact && err <= 5.0 * got.sigma + 1e-9 * exact,
        "{name}: {} ± {} vs {exact} (by direction {:?})",
        got.estimate,
        got.sigma,
        got.by_probe
    );
}

#[test]
fn box_and_region_within_their_uncertainty() {
    let m = model(
        "t
1 1 -1.0 -1 imp:n=1
2 0 1 -9 imp:n=1
9 0 9 imp:n=0

1 rpp 0 10 0 10 0 10
9 rpp -20 20 -20 20 -20 20

m1 1001.32c 1
",
    );
    let v = m
        .volumes(&VolumeGrid::new(cube(-20.0, 20.0), 0.5), None)
        .unwrap();
    let b = cell_volume(&m, &v, 1);
    assert_close("box", &b, 1000.0, 2e-3);
    assert_eq!(b.by_probe.len(), llevant::DEFAULT_PROBES);
    assert_eq!(b.probes_hit(), llevant::DEFAULT_PROBES);
    // Segment ends lie on the faces: inside the box, close to its corners.
    assert!(b
        .bbox
        .lo
        .iter()
        .chain(&b.bbox.hi)
        .all(|x| (-1e-9..=10.0 + 1e-9).contains(x)));
    assert!(b.bbox.lo.iter().all(|&x| x < 0.5) && b.bbox.hi.iter().all(|&x| x > 9.5));
    let all = v.group_by(|_| Some(())).pop().unwrap().1;
    assert_close("region", &all, 64000.0, 1e-3);
    assert!(v.problems().is_empty());
}

#[test]
fn curved_cells_within_their_uncertainty() {
    let m = model(
        "t
1 1 -1.0 -1 imp:n=1
2 1 -1.0 -2 imp:n=1
3 1 -1.0 -3 imp:n=1
8 0 1 2 3 -9 imp:n=1
9 0 9 imp:n=0

1 s -10 0 0 5
2 rcc -1 -2 -4 0 0 8 3.5
3 tz 12 0 0 6 2 2
9 rpp -20 25 -20 20 -20 20

m1 1001.32c 1
",
    );
    let v = m
        .volumes(
            &VolumeGrid::seeded(
                Aabb {
                    lo: [-20.0; 3],
                    hi: [25.0, 20.0, 20.0],
                },
                0.2,
                7,
            ),
            None,
        )
        .unwrap();
    // At h = 0.2 the sampling error is a few 1e-3 (it shrinks as about
    // h^1.5, with a sign that depends on the grid offset).
    assert_close(
        "sphere",
        &cell_volume(&m, &v, 1),
        4.0 / 3.0 * PI * 125.0,
        5e-3,
    );
    assert_close(
        "cylinder",
        &cell_volume(&m, &v, 2),
        PI * 3.5 * 3.5 * 8.0,
        5e-3,
    );
    assert_close(
        "torus",
        &cell_volume(&m, &v, 3),
        2.0 * PI * PI * 6.0 * 4.0,
        5e-3,
    );
}

#[test]
fn rotated_universes_and_lattices() {
    let m = model(
        "t
1 0 -1 *fill=1 (0 0 0  30 60 90  120 30 90  90 90 0) imp:n=1
2 0 -2 fill=2 imp:n=1
3 1 -1.0 -3 u=1 imp:n=1
4 0 3 u=1 imp:n=1
5 0 -5 lat=1 u=2 fill=3 imp:n=1
6 1 -1.0 -6 u=3 imp:n=1
7 0 6 u=3 imp:n=1
8 0 1 2 -9 imp:n=1
9 0 9 imp:n=0

1 rcc 0 0 -4 0 0 8 4
2 rpp -12 -8 -2 2 -2 2
3 box -3 -1 -1  6 0 0  0 2 0  0 0 2
5 rpp -12 -11 -2 -1 -2 -1
6 s -11.5 -1.5 -1.5 0.3
9 rpp -15 15 -15 15 -15 15

m1 1001.32c 1
",
    );
    let v = m
        .volumes(&VolumeGrid::seeded(cube(-15.0, 15.0), 0.05, 3), None)
        .unwrap();
    // A 6 × 2 × 2 box, rotated about z inside the cylinder.
    assert_close("rotated box", &cell_volume(&m, &v, 3), 24.0, 2e-3);
    // 4 × 4 × 4 lattice elements, a sphere of radius 0.3 in each.
    let sphere = 4.0 / 3.0 * PI * 0.027;
    assert_close(
        "lattice spheres",
        &cell_volume(&m, &v, 6),
        64.0 * sphere,
        1e-2,
    );
}

#[test]
fn planted_gaps_and_overlaps_are_found() {
    let m = model(
        "t
1 1 -1.0 -1 imp:n=1
2 0 2 3 4 -9 imp:n=1
3 1 -1.0 -3 imp:n=1
4 1 -1.0 -4 imp:n=1
9 0 9 imp:n=0

1 so 5
2 so 6
3 rpp 10 12 0 2 0 2
4 rpp 11 13 0 2 0 2
9 rpp -20 20 -20 20 -20 20

m1 1001.32c 1
",
    );
    let v = m
        .volumes(&VolumeGrid::new(cube(-20.0, 20.0), 0.1), None)
        .unwrap();
    let p = v.problems();
    let kinds: Vec<Status> = p.iter().map(|&i| v.labels[i].status).collect();
    assert!(
        kinds.contains(&Status::Gap) && kinds.contains(&Status::Overlap),
        "{kinds:?}"
    );
    let gap: f64 = p
        .iter()
        .filter(|&&i| v.labels[i].status == Status::Gap)
        .map(|&i| v.volumes[i].estimate)
        .sum();
    // The shell between the spheres is claimed by no cell; the boxes of
    // cells 3 and 4 overlap over 1 × 2 × 2.
    let shell = 4.0 / 3.0 * PI * (216.0 - 125.0);
    assert!((gap - shell).abs() < 2e-3 * shell, "gap {gap} vs {shell}");
    let overlap = p
        .iter()
        .find(|&&i| {
            let l = &v.labels[i];
            l.status == Status::Overlap
                && l.involved.iter().all(|&c| [3, 4].contains(&m.cell_id(c)))
        })
        .expect("overlap of cells 3 and 4");
    let o = &v.volumes[*overlap];
    assert_close("overlap", o, 4.0, 1e-2);
    let exact = Aabb {
        lo: [11.0, 0.0, 0.0],
        hi: [12.0, 2.0, 2.0],
    };
    for i in 0..3 {
        assert!(o.bbox.lo[i] >= exact.lo[i] - 1e-9 && o.bbox.lo[i] < exact.lo[i] + 0.1);
        assert!(o.bbox.hi[i] <= exact.hi[i] + 1e-9 && o.bbox.hi[i] > exact.hi[i] - 0.1);
    }
}

/// Shapes with exact volumes: slabs as thick as the spacing and thinner,
/// a box, a sphere and a cylinder, the flat faces parallel to the axes.
const SHAPES: &str = "t
1 1 -1.0 -1 imp:n=1
2 1 -1.0 -2 imp:n=1
3 1 -1.0 -3 imp:n=1
4 1 -1.0 -4 imp:n=1
5 1 -1.0 -5 imp:n=1
8 0 1 2 3 4 5 -9 imp:n=1
9 0 9 imp:n=0

1 rpp -20 0 -20 0 0 1
2 rpp -20 0 -20 0 5 5.3
3 rpp 2 9.3 2 6.1 2 4.7
4 s 10 -10 10 4
5 rcc -8 8 10 0 0 8 2.5
9 rpp -21 15 -21 15 -1 19

m1 1001.32c 1
";

fn shapes_exact() -> [(i64, f64); 5] {
    [
        (1, 400.0),
        (2, 120.0),
        (3, 7.3 * 4.1 * 2.7),
        (4, 4.0 / 3.0 * PI * 64.0),
        (5, PI * 2.5 * 2.5 * 8.0),
    ]
}

#[test]
fn sigma_is_an_honest_standard_error() {
    // Over independent seeds the error stays within 2 sigma about 95% of
    // the time and sigma matches the spread of the errors. The interleaved
    // sub-grids this replaced gave a sigma about 1000 times too large for
    // slab 1, and several times too small for slab 2 and the box.
    let m = model(SHAPES);
    let region = Aabb {
        lo: [-21.0, -21.0, -1.0],
        hi: [15.0, 15.0, 19.0],
    };
    let seeds = 60;
    for (id, exact) in shapes_exact() {
        let (mut within, mut err2, mut sig2, mut mean) = (0, 0.0, 0.0, 0.0);
        for seed in 0..seeds {
            let v = m
                .volumes(&VolumeGrid::seeded(region, 1.0, seed), None)
                .unwrap();
            let c = cell_volume(&m, &v, id);
            let e = c.estimate - exact;
            within += (e.abs() <= 2.0 * c.sigma) as usize;
            err2 += e * e;
            sig2 += c.sigma * c.sigma;
            mean += e;
        }
        let n = seeds as f64;
        let (rms, sig) = ((err2 / n).sqrt(), (sig2 / n).sqrt());
        let cover = within as f64 / n;
        assert!(rms < 0.1 * exact, "cell {id}: rms error {rms}");
        assert!(
            (0.8..=1.0).contains(&cover),
            "cell {id}: 2 sigma covers {cover}"
        );
        assert!(
            (0.7..=1.4).contains(&(sig / rms)),
            "cell {id}: sigma {sig}, rms error {rms}"
        );
        // Unbiased: the mean error is within 3 standard errors of 0.
        assert!(
            (mean / n).abs() < 3.0 * rms / n.sqrt(),
            "cell {id}: bias {}",
            mean / n
        );
    }
}

#[test]
fn every_instance_has_its_own_volume() {
    // The same filler in two envelopes, the second cutting its sphere in half.
    let m = model(
        "t
1 1 -1.0 -1 u=1 imp:n=1
2 0 1 u=1 imp:n=1
10 0 -10 fill=1 imp:n=1
20 0 -20 fill=1 (30 0 0) imp:n=1
30 0 10 20 -9 imp:n=1
31 0 9 imp:n=0

1 s 0 0 0 5
10 rpp -10 10 -10 10 -10 10
20 rpp 20 40 -10 10 -10 0
9 rpp -12 42 -12 12 -12 12

m1 1001.32c 1
",
    );
    let v = m
        .volumes(&VolumeGrid::new(cube(-12.0, 42.0), 0.2), None)
        .unwrap();
    let sphere = 4.0 / 3.0 * PI * 125.0;
    let instance = |envelope: i64| {
        v.group_by(|l| {
            let ids: Vec<i64> = l.path.iter().map(|s| m.cell_id(s.cell)).collect();
            (l.status == Status::Inside && ids == [envelope, 1]).then_some(())
        })
        .pop()
        .expect("instance met")
        .1
    };
    assert_close("whole", &instance(10), sphere, 5e-3);
    assert_close("cut", &instance(20), sphere / 2.0, 5e-3);
    assert_close("both", &cell_volume(&m, &v, 1), 1.5 * sphere, 5e-3);
}

/// An absolute floor: a tiny cell that cannot reach its relative target is
/// done once its sigma is that many cm³, no finer than without the floor; a
/// large one still goes to its relative target.
#[test]
fn absolute_sigma_stops_small_cells_early() {
    let m = model(
        "t
1 1 -1.0 -1 imp:n=1
2 1 -1.0 -2 imp:n=1
3 0 1 2 -9 imp:n=1
9 0 9 imp:n=0

1 so 5
2 s 8 0 0 0.3
9 rpp -10 10 -10 10 -10 10

m1 1001.80c 1
",
    );
    let small = 4.0 / 3.0 * PI * 0.3f64.powi(3);
    let large = 4.0 / 3.0 * PI * 125.0;
    let grid = VolumeGrid::seeded(cube(-10.0, 10.0), 2.0, 3);
    let run = |abs_sigma: f64| {
        let target = VolumeTarget {
            rel_sigma: 2e-3,
            h_min: 0.0625,
            abs_sigma,
            max_probes: llevant::MAX_PROBES,
            by: GroupBy::Cell,
        };
        let c = m
            .volumes_to(
                &grid,
                &target,
                |g| matches!(g.number, Some(1 | 2)),
                None,
                None,
            )
            .unwrap();
        let at = |id| c.groups.iter().position(|g| g.number == Some(id)).unwrap();
        let (i, j) = (at(1), at(2));
        (
            c.groups[i].volume.clone(),
            c.h[i],
            c.groups[j].volume.clone(),
            c.h[j],
            c,
        )
    };
    let (_, _, tiny, h_rel, alone) = run(0.0);
    let (big, h_big, tiny_abs, h_abs, c) = run(1e-3);
    // Relative alone, the tiny sphere cannot get there by h_min.
    let at = |c: &llevant::Converged| c.groups.iter().position(|g| g.number == Some(2));
    assert!(alone.unconverged.contains(&at(&alone).unwrap()), "{tiny:?}");
    assert!(
        h_abs >= h_rel,
        "floor at h {h_abs}, relative alone at h {h_rel}"
    );
    assert!(
        tiny_abs.sigma <= 1e-3 && tiny_abs.probes_hit() >= 2,
        "{tiny_abs:?}"
    );
    assert!(c.unconverged.is_empty(), "{:?}", c.unconverged);
    assert!(
        (tiny_abs.estimate - small).abs() < 5.0 * tiny_abs.sigma,
        "{tiny_abs:?}"
    );
    assert!(tiny.estimate > 0.0);
    // The large one is far above the floor: its relative target holds.
    assert!(big.sigma <= 2e-3 * big.estimate && h_big < 2.0, "{big:?}");
    assert_close("large", &big, large, 0.01);
}

#[test]
fn volumes_to_a_target_sigma() {
    let m = model(SHAPES);
    let region = Aabb {
        lo: [-21.0, -21.0, -1.0],
        hi: [15.0, 15.0, 19.0],
    };
    let target = VolumeTarget {
        rel_sigma: 1e-3,
        h_min: 0.05,
        abs_sigma: 0.0,
        max_probes: llevant::MAX_PROBES,
        by: GroupBy::Cell,
    };
    let wanted = |g: &Group| g.number.is_some_and(|n| n < 8);
    let grid = VolumeGrid::seeded(region, 2.0, 5);
    let c = m.volumes_to(&grid, &target, wanted, None, None).unwrap();
    assert!(c.unconverged.is_empty(), "{:?}", c.unconverged);
    for (id, exact) in shapes_exact() {
        let i = c.groups.iter().position(|g| g.number == Some(id)).unwrap();
        let v = &c.groups[i].volume;
        assert!(v.sigma <= 1e-3 * v.estimate, "cell {id}: {v:?}");
        assert!(c.h[i] < 2.0, "cell {id} was refined");
        assert!(
            (v.estimate - exact).abs() < 4e-3 * exact,
            "cell {id}: {} vs {exact}",
            v.estimate
        );
    }
    // Groups not wanted keep the first grid's volume.
    let outer = c.groups.iter().position(|g| g.number == Some(8)).unwrap();
    assert_eq!(c.h[outer], 2.0);

    // The states before each round: the first pass first, each finer
    // than the last, the run itself the same.
    let mut states: Vec<(Vec<f64>, usize)> = Vec::new();
    let same = m
        .volumes_to(
            &grid,
            &target,
            wanted,
            None,
            Some(&mut |s| states.push((s.h.clone(), s.unconverged.len()))),
        )
        .unwrap();
    assert_eq!(same.h, c.h);
    assert!(states.len() > 1, "{} states", states.len());
    assert!(states[0].0.iter().all(|&h| h == 2.0));
    assert!(states.iter().all(|s| s.1 > 0));
    for w in states.windows(2) {
        assert!(w[0].0.iter().zip(&w[1].0).all(|(a, b)| b <= a));
    }
    let first = m.volumes(&grid, None).unwrap().grouped(&m, GroupBy::Cell);
    let pass = states[0].1;
    assert!(pass <= first.iter().filter(|g| wanted(g)).count());

    // Slab 2 (0.3 cm) cannot get to 1e-6 by h = 0.5.
    let target = VolumeTarget {
        rel_sigma: 1e-6,
        h_min: 0.5,
        ..target
    };
    let c = m
        .volumes_to(&grid, &target, |g| g.number == Some(2), None, None)
        .unwrap();
    assert_eq!(c.unconverged.len(), 1);
    assert_eq!(c.groups[c.unconverged[0]].number, Some(2));
    // Given up as soon as no level down to h_min could get there.
    assert!(c.h[c.unconverged[0]] >= 0.5);
}

#[test]
fn volumes_do_not_depend_on_thread_count() {
    let text = "t
1 1 -1.0 -1 imp:n=1
2 2 -2.0 -2 1 imp:n=1
8 0 2 -9 imp:n=1
9 0 9 imp:n=0

1 s 0.3 0.1 0 4
2 tx 0 0 0 6 2.5 2.5
9 rpp -20 20 -20 20 -20 20

m1 1001.32c 1
m2 26056.32c 1
";
    let m = model(text);
    let g = VolumeGrid::seeded(cube(-20.0, 20.0), 0.37, 11);
    let many = m.volumes(&g, None).unwrap();
    let one = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap()
        .install(|| m.volumes(&g, None).unwrap());
    assert_eq!(many.labels, one.labels);
    assert_eq!(many.volumes, one.volumes);
}

/// A small sphere in a universe filled twice (once moved), a cell its
/// envelopes cut away, a universe nothing fills and a shell its envelope
/// cannot reach.
const HIDDEN: &str = "t
1 0 -1 fill=1 imp:n=1
2 0 -2 fill=1 (10 0 0) imp:n=1
3 0 1 2 -9 imp:n=1
9 0 9 imp:n=0
10 1 -1.0 -10 u=1 imp:n=1
30 1 -1.0 -30 u=1 imp:n=1
40 1 -1.0 -40 41 u=1 imp:n=1
11 0 10 30 #40 u=1 imp:n=1
20 1 -1.0 -20 u=2 imp:n=1
21 0 20 u=2 imp:n=1

1 rpp -2 2 -2 2 -2 2
2 rpp 8 12 -2 2 -2 2
9 rpp -20 20 -20 20 -20 20
10 s 0.3 -0.2 0.1 0.3
20 so 1
30 s 5 5 5 1
40 so 3.6
41 so 3.5

m1 1001.32c 1
";

#[test]
fn volumes_to_lists_every_cell_met_or_not() {
    let m = model(HIDDEN);
    let grid = VolumeGrid::seeded(cube(-20.0, 20.0), 4.0, 3);
    let first = m.volumes(&grid, None).unwrap().grouped(&m, GroupBy::Cell);
    assert!(
        !first.iter().any(|g| g.number == Some(10)),
        "the first lines should miss the sphere"
    );
    let target = VolumeTarget {
        rel_sigma: 2e-2,
        h_min: 0.005,
        abs_sigma: 0.0,
        max_probes: llevant::MAX_PROBES,
        by: GroupBy::Cell,
    };
    let c = m
        .volumes_to(&grid, &target, |g| g.status == Status::Inside, None, None)
        .unwrap();
    assert!(c.listed_all);
    let at = |id: i64| {
        c.groups
            .iter()
            .position(|g| g.number == Some(id))
            .unwrap_or_else(|| panic!("cell {id} not listed"))
    };
    // Both instances of the sphere, found and measured.
    let sphere = 4.0 / 3.0 * PI * 0.3f64.powi(3);
    let i = at(10);
    assert!(!c.unconverged.contains(&i), "{:?}", c.groups[i]);
    assert_close("hidden sphere", &c.groups[i].volume, 2.0 * sphere, 0.05);
    // Cut away by its envelopes, or in a universe nothing fills: empty.
    for id in [20, 30] {
        let i = at(id);
        assert!(c.empty.contains(&i), "cell {id}: {:?}", c.groups[i]);
        assert_eq!(c.groups[i].volume.estimate, 0.0);
    }
    // A shell beyond the envelope's corners: listed, with no volume, as
    // empty or as not found.
    let i = at(40);
    assert!(
        c.empty.contains(&i) || c.unseen.contains(&i),
        "{:?}",
        c.groups[i]
    );
    assert_eq!(c.groups[i].volume.estimate, 0.0);
    for i in &c.unseen {
        assert!(c.unconverged.contains(i));
    }

    // By label: every instance, the sphere's two measured apart.
    let c = m
        .volumes_to(
            &grid,
            &VolumeTarget {
                by: GroupBy::Label,
                ..target
            },
            |g| g.status == Status::Inside,
            None,
            None,
        )
        .unwrap();
    assert!(c.listed_all);
    for name in ["inside 1 > 10", "inside 2 > 10"] {
        let i = c
            .groups
            .iter()
            .position(|g| g.name == name)
            .unwrap_or_else(|| panic!("{name} not listed"));
        assert!(!c.unconverged.contains(&i), "{:?}", c.groups[i]);
        assert_close(name, &c.groups[i].volume, sphere, 0.05);
    }
}

/// A tally chain of cells by MCNP number, the cell scored first, each with
/// its lattice element if any.
fn chain(m: &Model, levels: &[(i64, Option<[i32; 3]>)]) -> Vec<Vec<Step>> {
    levels
        .iter()
        .map(|&(id, lattice)| {
            vec![Step {
                cell: m.cell_index(id).unwrap(),
                lattice,
            }]
        })
        .collect()
}

/// Volumes of tally bins: the instances each chain covers
/// ([`Instances::matching`](llevant::Instances::matching)).
fn bin_volumes(m: &Model, chains: &[Vec<Vec<Step>>], target: &BinTarget) -> BinVolumes {
    let inst = m.instances();
    let sets: Vec<Vec<u64>> = chains.iter().map(|c| inst.matching(m, c)).collect();
    m.instance_volumes(&sets, target, None).unwrap()
}

#[test]
fn bin_volumes_measure_each_chain_on_its_own() {
    let m = model(HIDDEN);
    let target = BinTarget::new(cube(-20.0, 20.0), 2e-2, 0.005);
    let bins = [
        chain(&m, &[(10, None), (1, None)]),
        chain(&m, &[(10, None), (2, None)]),
        chain(&m, &[(10, None)]),
        // Cut away by its envelopes; in a universe nothing fills.
        chain(&m, &[(30, None)]),
        chain(&m, &[(20, None)]),
        // Not where the chain says.
        chain(&m, &[(10, None), (3, None)]),
    ];
    let v = bin_volumes(&m, &bins, &target);
    assert_eq!(v.instances, [1, 1, 2, 2, 0, 0]);
    let sphere = 4.0 / 3.0 * PI * 0.3f64.powi(3);
    for (b, exact) in [(0, sphere), (1, sphere), (2, 2.0 * sphere)] {
        assert!(!v.unconverged.contains(&b), "{b}: {:?}", v.volumes[b]);
        assert_close(&format!("bin {b}"), &v.volumes[b], exact, 0.05);
        assert!(v.h[b] > 0.0);
    }
    assert_eq!(v.empty, [3]);
    for b in 3..6 {
        assert_eq!(v.volumes[b].estimate, 0.0);
        assert_eq!(v.h[b], 0.0);
    }
    // The same lines whatever else is asked for (pieces split elsewhere:
    // equal to round-off).
    let alone = bin_volumes(&m, &bins[..1], &target);
    let (a, b) = (alone.volumes[0].estimate, v.volumes[0].estimate);
    assert!((a - b).abs() <= 1e-12 * b, "{a} vs {b}");
}

/// Volumes of chosen cells: every instance of each together, a filled cell
/// as its whole region (once, though lines end in the cells below it), and
/// a cell nothing places as empty.
#[test]
fn cell_volumes_sum_instances_and_fill_regions() {
    let m = model(HIDDEN);
    let target = BinTarget::new(cube(-20.0, 20.0), 1e-2, 0.005);
    let cells: Vec<u32> = [10, 1, 2, 20]
        .iter()
        .map(|&c| m.cell_index(c).unwrap())
        .collect();
    let c = m.cell_volumes(&cells, &target, None).unwrap();
    let names: Vec<&str> = c.groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["10", "1", "2", "20"]);
    let sphere = 4.0 / 3.0 * PI * 0.3f64.powi(3);
    for (i, exact) in [(0, 2.0 * sphere), (1, 64.0), (2, 64.0)] {
        assert!(!c.unconverged.contains(&i), "{i}: {:?}", c.groups[i]);
        assert_close(&c.groups[i].name, &c.groups[i].volume, exact, 0.05);
    }
    // Cell 20 is in a universe nothing fills: no instance, volume 0.
    assert_eq!(c.empty, [3]);
    assert_eq!(c.groups[3].volume.estimate, 0.0);
    assert!(m.cell_volumes(&[u32::MAX], &target, None).is_err());
}

/// A 3 × 3 lattice of spheres, placed off the origin.
const SPHERES: &str = "t
1 0 -1 fill=2 (1 0.5 0) imp:n=1
9 0 1 imp:n=0
5 0 -5 lat=1 u=2 fill=-1:1 -1:1 0:0 3 3 3 3 3 3 3 3 3 imp:n=1
10 1 -1.0 -10 u=3 imp:n=1
11 0 10 u=3 imp:n=1

1 rpp -2 4 -2.5 3.5 -1 1
5 rpp -1 1 -1 1 -1 1
10 s 0.3 0.2 0 0.4

m1 1001.32c 1
";

#[test]
fn bin_volumes_follow_lattice_elements() {
    let m = model(SPHERES);
    let target = BinTarget::new(cube(-5.0, 5.0), 1e-2, 0.002);
    let bins = [
        chain(&m, &[(10, None), (1, None)]),
        chain(&m, &[(10, None), (5, Some([1, 0, 0])), (1, None)]),
        chain(&m, &[(10, None), (5, Some([-1, 1, 0]))]),
        chain(&m, &[(10, None), (5, Some([5, 0, 0]))]),
    ];
    let v = bin_volumes(&m, &bins, &target);
    assert_eq!(v.instances, [9, 1, 1, 0]);
    let sphere = 4.0 / 3.0 * PI * 0.4f64.powi(3);
    for (b, exact) in [(0, 9.0 * sphere), (1, sphere), (2, sphere)] {
        assert!(!v.unconverged.contains(&b), "{b}: {:?}", v.volumes[b]);
        assert_close(&format!("bin {b}"), &v.volumes[b], exact, 0.03);
    }
    // One sphere is traced over little more than itself: its segments lie
    // in the element's sphere.
    let b = v.volumes[1].bbox;
    let centre = [1.0 + 2.0 + 0.3, 0.5 + 0.2, 0.0];
    for k in 0..3 {
        assert!(
            b.lo[k] >= centre[k] - 0.4 - 1e-9 && b.hi[k] <= centre[k] + 0.4 + 1e-9,
            "{b:?}"
        );
    }
}

/// The planted model of `planted_gaps_and_overlaps_are_found`.
const PLANTED: &str = "t
1 1 -1.0 -1 imp:n=1
2 0 2 3 4 -9 imp:n=1
3 1 -1.0 -3 imp:n=1
4 1 -1.0 -4 imp:n=1
9 0 9 imp:n=0

1 so 5
2 so 6
3 rpp 10 12 0 2 0 2
4 rpp 11 13 0 2 0 2
9 rpp -20 20 -20 20 -20 20

m1 1001.32c 1
";

#[test]
fn a_check_within_boxes_finds_what_the_whole_check_finds_there() {
    let m = model(PLANTED);
    let grid = VolumeGrid::new(cube(-20.0, 20.0), 0.1);
    let whole = m.volumes(&grid, None).unwrap();
    // Over the whole region as one box: the same check.
    let same = m.volumes_within(&grid, &[grid.region], None).unwrap();
    assert_eq!(same.labels.len(), whole.labels.len());
    for (a, b) in same.volumes.iter().zip(&whole.volumes) {
        assert!((a.estimate - b.estimate).abs() <= 1e-9 * b.estimate.max(1.0));
    }
    // Around cells 3 and 4 alone: their overlap, as large, and not the gap
    // between the spheres.
    let cells: Vec<u32> = [3, 4].map(|id| m.cell_index(id).unwrap()).to_vec();
    let boxes = m.instance_boxes(&cells, grid.region, 0.8);
    assert!(!boxes.is_empty());
    let near = m.volumes_within(&grid, &boxes, None).unwrap();
    let overlap = |v: &Volumes| {
        v.problems()
            .iter()
            .find(|&&i| v.labels[i].status == Status::Overlap)
            .map(|&i| v.volumes[i].estimate)
            .expect("the overlap of cells 3 and 4")
    };
    let (a, b) = (overlap(&near), overlap(&whole));
    assert!((a - b).abs() <= 1e-9 * b, "{a} vs {b}");
    assert!(near
        .problems()
        .iter()
        .all(|&i| near.labels[i].status != Status::Gap));
    // Empty or NaN boxes.
    assert!(m
        .volumes_within(&grid, &[], None)
        .unwrap()
        .labels
        .is_empty());
    let nan = Aabb {
        lo: [f64::NAN; 3],
        hi: [1.0; 3],
    };
    assert!(m.volumes_within(&grid, &[nan], None).is_err());
}

#[test]
fn placed_boxes_hold_every_point_of_their_cells() {
    // Copies by TRCL fill, a lattice, a cell at level 0.
    let m = model(
        "t
1 0 -1 *fill=1 (0 0 0  30 60 90  120 30 90  90 90 0) imp:n=1
2 0 -2 fill=2 imp:n=1
3 1 -1.0 -3 u=1 imp:n=1
4 0 3 u=1 imp:n=1
5 0 -5 lat=1 u=2 fill=3 imp:n=1
6 1 -1.0 -6 u=3 imp:n=1
7 0 6 u=3 imp:n=1
8 0 1 2 -9 imp:n=1
9 0 9 imp:n=0

1 rcc 0 0 -4 0 0 8 4
2 rpp -12 -8 -2 2 -2 2
3 box -3 -1 -1  6 0 0  0 2 0  0 0 2
5 rpp -12 -11 -2 -1 -2 -1
6 s -11.5 -1.5 -1.5 0.3
9 rpp -15 15 -15 15 -15 15

m1 1001.32c 1
",
    );
    let region = cube(-15.0, 15.0);
    let all: Vec<u32> = (0..m.cells().len() as u32).collect();
    let boxes = m.placed_boxes(&all, region);
    let of = |id: i64| -> Vec<&llevant::PlacedBox> {
        let c = m.cell_index(id).unwrap();
        boxes.iter().filter(|b| b.cell == c).collect()
    };
    // One instance each, but where it may be only below the lattice.
    assert_eq!(of(3).len(), 1);
    assert!(of(3)[0].exact && of(8)[0].exact);
    assert!(!of(6).is_empty() && of(6).iter().all(|b| !b.exact));
    // Boxes in the order asked, each cell once.
    let c3 = m.cell_index(3).unwrap();
    let twice = m.placed_boxes(&[c3, c3, u32::MAX], region);
    assert_eq!(twice.len(), 1);
    assert_eq!(twice[0], *of(3)[0]);
    // Every point of a cell, at every level, is in one of its boxes.
    let n = 120;
    let at = |k: usize| -15.0 + 30.0 * (k as f64 + 0.5) / n as f64;
    let points: Vec<[f64; 3]> = (0..n * n * n)
        .map(|k| [at(k % n), at(k / n % n), at(k / (n * n))])
        .collect();
    let found = m.classify(&points, Depth::All);
    for (p, &id) in points.iter().zip(&found.ids) {
        let l = &found.labels[id as usize];
        if l.status != Status::Inside {
            continue;
        }
        for s in &l.path {
            let held = boxes
                .iter()
                .any(|b| b.cell == s.cell && b.global.contains(*p));
            assert!(
                held,
                "{p:?} of cell {} outside its boxes",
                m.cell(s.cell).id
            );
        }
    }
}

#[test]
fn instance_boxes_cover_every_instance() {
    let m = model(
        "t
1 0 -1 *fill=1 (0 0 0  30 60 90  120 30 90  90 90 0) imp:n=1
2 0 -2 fill=2 imp:n=1
3 1 -1.0 -3 u=1 imp:n=1
4 0 3 u=1 imp:n=1
5 0 -5 lat=1 u=2 fill=3 imp:n=1
6 1 -1.0 -6 u=3 imp:n=1
7 0 6 u=3 imp:n=1
8 0 1 2 -9 imp:n=1
9 0 9 imp:n=0

1 rcc 0 0 -4 0 0 8 4
2 rpp -12 -8 -2 2 -2 2
3 box -3 -1 -1  6 0 0  0 2 0  0 0 2
5 rpp -12 -11 -2 -1 -2 -1
6 s -11.5 -1.5 -1.5 0.3
9 rpp -15 15 -15 15 -15 15

m1 1001.32c 1
",
    );
    let grid = VolumeGrid::seeded(cube(-15.0, 15.0), 0.05, 3);
    let whole = m.volumes(&grid, None).unwrap();
    for (id, side) in [(3, 0.4), (6, 0.0), (8, 2.0)] {
        let c = m.cell_index(id).unwrap();
        let boxes = m.instance_boxes(&[c], grid.region, side);
        let near = m.volumes_within(&grid, &boxes, None).unwrap();
        let (a, b) = (cell_volume(&m, &near, id), cell_volume(&m, &whole, id));
        assert!(
            (a.estimate - b.estimate).abs() <= 1e-9 * b.estimate,
            "cell {id}: {} within its boxes, {} in all",
            a.estimate,
            b.estimate
        );
    }
}

#[test]
fn spacing_within_counts_overlapping_boxes_once() {
    let region = cube(0.0, 100.0);
    let b = Aabb {
        lo: [10.0, 10.0, 10.0],
        hi: [30.0, 40.0, 50.0],
    };
    let one = llevant::spacing_within(&region, &[b], 1e4).unwrap();
    // Faces seen along x, y and z: 30·40 + 20·40 + 20·30.
    let exact = ((1200.0 + 800.0 + 600.0) / 1e4f64).sqrt();
    assert!((one - exact).abs() < 1e-9 * exact, "{one} vs {exact}");
    let twice = llevant::spacing_within(&region, &[b, b, b], 1e4).unwrap();
    assert!((twice - one).abs() < 1e-12, "{twice} vs {one}");
    // Thin: its own faces.
    let thin = Aabb {
        lo: [10.0, 10.0, 10.0],
        hi: [10.01, 90.0, 90.0],
    };
    let h = llevant::spacing_within(&region, &[thin], 1e4).unwrap();
    let own = ((80.0 * 80.0 + 0.01 * 80.0 * 2.0) / 1e4f64).sqrt();
    assert!((h - own).abs() < 1e-9 * own, "{h} vs {own}");
    assert!(llevant::spacing_within(&region, &[cube(200.0, 300.0)], 1e4).is_none());
}

#[test]
fn cells_kept_by_cuts() {
    use llevant::{Clip, Cutaway};
    // As above: copies by TRCL fill, a lattice, a cell at level 0.
    let m = model(
        "t
1 0 -1 *fill=1 (0 0 0  30 60 90  120 30 90  90 90 0) imp:n=1
2 0 -2 fill=2 imp:n=1
3 1 -1.0 -3 u=1 imp:n=1
4 0 3 u=1 imp:n=1
5 0 -5 lat=1 u=2 fill=3 imp:n=1
6 1 -1.0 -6 u=3 imp:n=1
7 0 6 u=3 imp:n=1
8 0 1 2 -9 imp:n=1
9 0 9 imp:n=0

1 rcc 0 0 -4 0 0 8 4
2 rpp -12 -8 -2 2 -2 2
3 box -3 -1 -1  6 0 0  0 2 0  0 0 2
5 rpp -12 -11 -2 -1 -2 -1
6 s -11.5 -1.5 -1.5 0.3
9 rpp -15 15 -15 15 -15 15

m1 1001.32c 1
",
    );
    let region = cube(-15.0, 15.0);
    let all: Vec<u32> = (0..m.cells().len() as u32).collect();
    let plane = |point: [f64; 3], normal: [f64; 3]| Cutaway::plane(Clip { point, normal });
    let kept = |cut: &Cutaway| -> Vec<i64> {
        let k = m.cells_kept(&all, region, cut);
        all.iter()
            .filter(|&&c| k[c as usize])
            .map(|&c| m.cell_id(c))
            .collect()
    };
    // Nothing cut: every cell.
    assert_eq!(kept(&Cutaway::none()), [1, 2, 3, 4, 5, 6, 7, 8, 9]);
    // x < -5 cut away: the lattice and all in it go, the rest stays (the
    // outside too, within the region).
    assert_eq!(
        kept(&plane([-5.0, 0.0, 0.0], [-1.0, 0.0, 0.0])),
        [1, 3, 4, 8, 9]
    );
    // Below a lattice only the box where the lattice is: its cells are
    // kept while some of that is.
    let away = |x: f64| plane([x, 0.0, 0.0], [1.0, 0.0, 0.0]);
    assert!(kept(&away(-11.9)).contains(&6));
    assert!(!kept(&away(-12.1)).contains(&6));
    // A copy turned 30° about z: the box reaches x = -(3 cos 30° + sin 30°)
    // = -3.098.
    assert!(kept(&away(-3.05)).contains(&3));
    assert!(!kept(&away(-3.15)).contains(&3));
    assert!(kept(&away(-3.15)).contains(&4));
    // Conservative under any cut: every cell met at a point not cut away
    // is kept, at every level.
    let n = 60;
    let at = |k: usize| -15.0 + 30.0 * (k as f64 + 0.5) / n as f64;
    let points: Vec<[f64; 3]> = (0..n * n * n)
        .map(|k| [at(k % n), at(k / n % n), at(k / (n * n))])
        .collect();
    let found = m.classify(&points, Depth::All);
    let cuts = [
        plane([0.5, 0.2, 0.0], [1.0, 1.0, 0.3]),
        plane([-11.5, -1.5, -1.5], [0.0, 0.0, 1.0]),
        Cutaway::wedge([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], 10.0, 290.0),
        Cutaway::keep_box(&Aabb {
            lo: [-11.8, -1.8, -1.8],
            hi: [-11.4, -1.4, -1.0],
        }),
        Cutaway::remove_box(&cube(-2.0, 2.0)),
    ];
    for cut in &cuts {
        let k = m.cells_kept(&all, region, cut);
        for (p, &id) in points.iter().zip(&found.ids) {
            let l = &found.labels[id as usize];
            if l.status != Status::Inside || cut.removes(*p) {
                continue;
            }
            for s in &l.path {
                assert!(
                    k[s.cell as usize],
                    "cell {} at {p:?} not kept by {cut:?}",
                    m.cell(s.cell).id
                );
            }
        }
    }
}
