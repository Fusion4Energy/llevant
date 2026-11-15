use super::*;
use crate::Options;

/// The union of overlapping and abutting boxes is sampled as the region
/// they make up: the same lines, so the same volumes.
#[test]
fn boxes_sample_their_union_once() {
    let m = Model::from_text(
        "t\n1 1 -1.0 -1 imp:n=1\n2 0 1 -9 imp:n=1\n9 0 9 imp:n=0\n\n\
         1 s 0.3 0.2 -0.1 3\n9 rpp -10 10 -10 10 -10 10\n\nm1 1001.80c 1\n",
        &Options::default(),
    );
    let region = Aabb {
        lo: [-4.0; 3],
        hi: [4.0; 3],
    };
    let grid = VolumeGrid::seeded(region, 0.3, 7);
    let whole = m
        .volumes_in(&grid, 0..grid.probes, &[region], &Progress::new())
        .unwrap();
    let part = |lo: [f64; 3], hi: [f64; 3]| Aabb { lo, hi };
    let boxes = [
        part([-4.0, -4.0, -4.0], [1.0, 4.0, 4.0]),
        // Overlaps the first.
        part([-1.0, -4.0, -4.0], [4.0, 0.5, 4.0]),
        // Abuts the second, overlaps the first.
        part([-2.0, 0.5, -4.0], [4.0, 4.0, 4.0]),
        // Inside the first.
        part([-3.0, -3.0, -3.0], [0.0, 0.0, 0.0]),
    ];
    let split = m
        .volumes_in(&grid, 0..grid.probes, &boxes, &Progress::new())
        .unwrap();
    let by_cell = |v: &Volumes| {
        let mut g: Vec<(Option<i64>, f64)> = v
            .grouped(&m, GroupBy::Cell)
            .into_iter()
            .map(|g| (g.number, g.volume.estimate))
            .collect();
        g.sort_by_key(|a| a.0);
        g
    };
    let (a, b) = (by_cell(&whole), by_cell(&split));
    assert_eq!(a.len(), b.len(), "{a:?} vs {b:?}");
    for ((ca, va), (cb, vb)) in a.iter().zip(&b) {
        assert_eq!(ca, cb);
        assert!(
            (va - vb).abs() <= 1e-9 * va.abs().max(1.0),
            "cell {ca:?}: {va} vs {vb}"
        );
    }
}

/// Further probes of a grid, traced apart, are those of one call over
/// all of them: the same lattices, the same sums (integers: exactly).
#[test]
fn probes_traced_apart_pool_exactly() {
    let m = Model::from_text(
        "t
1 1 -1.0 -1 imp:n=1
2 0 1 -9 imp:n=1
9 0 9 imp:n=0

         1 s 0.3 0.2 -0.1 3
9 rpp -10 10 -10 10 -10 10

m1 1001.80c 1
",
        &Options::default(),
    );
    let region = Aabb {
        lo: [-4.0; 3],
        hi: [4.0; 3],
    };
    let grid = VolumeGrid::seeded(region, 0.5, 7);
    let p = Progress::new();
    let all = m.volumes_in(&grid, 0..24, &[region], &p).unwrap();
    let first = m.volumes_in(&grid, 0..12, &[region], &p).unwrap();
    let more = m.volumes_in(&grid, 12..24, &[region], &p).unwrap();
    let sphere = |v: &Volumes| {
        let g = v.grouped(&m, GroupBy::Cell);
        g.into_iter().find(|g| g.number == Some(1)).unwrap().volume
    };
    let mut pooled = sphere(&first);
    pooled.append(&sphere(&more));
    pooled.finish(&grid);
    let whole = sphere(&all);
    assert_eq!(pooled.lengths, whole.lengths);
    assert_eq!(pooled.estimate, whole.estimate);
    assert_eq!(pooled.segments, whole.segments);
    assert_eq!(whole.lengths.len(), 24);
}

/// The pieces over a union of boxes are those `Patches` made, line by
/// line, whatever the bands: overlapping, abutting, nested and
/// disjoint boxes, for several probes.
#[test]
fn union_pieces_are_those_of_patches() {
    let mut r = 0x2545f4914f6cdd1du64;
    let mut f = move || {
        r ^= r << 13;
        r ^= r >> 7;
        r ^= r << 17;
        (r >> 11) as f64 / (1u64 << 53) as f64
    };
    let region = Aabb {
        lo: [-10.0; 3],
        hi: [10.0; 3],
    };
    let grid = VolumeGrid::seeded(region, 0.4, 11);
    let mut boxes: Vec<Aabb> = (0..40)
        .map(|_| {
            let lo = [0, 1, 2].map(|_| -9.0 + 14.0 * f());
            let hi = [0, 1, 2].map(|k| lo[k] + 0.2 + 4.0 * f());
            Aabb { lo, hi }
        })
        .collect();
    // Abutting exactly, and one inside another.
    boxes.push(Aabb {
        lo: [1.0, 1.0, 1.0],
        hi: [3.0, 3.0, 3.0],
    });
    boxes.push(Aabb {
        lo: [3.0, 1.0, 1.0],
        hi: [5.0, 3.0, 3.0],
    });
    boxes.push(Aabb {
        lo: [1.5, 1.5, 1.5],
        hi: [2.0, 2.0, 2.0],
    });
    let key = |l: &Line| (l.origin.map(f64::to_bits), l.t_max.to_bits());
    let patches = Patches::new(&boxes);
    for p in 0..4 {
        let l = grid.lattice(p);
        let mut old = Vec::new();
        for (b, x) in boxes.iter().enumerate() {
            let o = grid.lattice_over(&l, x);
            for n in 0..o.count() {
                patches.piece(b, grid.line_point(&o, n), o.dir, &mut old);
            }
        }
        let ids: Vec<u32> = (0..boxes.len() as u32).collect();
        for width in [1, 3, 1000] {
            let (lo, hi) = (-1000, 1000);
            let mut new = Vec::new();
            let mut r0 = lo;
            while r0 < hi {
                union_pieces(&grid, &l, &boxes, &ids, (r0, r0 + width), &mut new);
                r0 += width;
            }
            let mut a: Vec<_> = old.iter().map(key).collect();
            let mut b: Vec<_> = new.iter().map(key).collect();
            a.sort_unstable();
            b.sort_unstable();
            assert_eq!(a.len(), b.len(), "probe {p}, bands of {width}");
            assert!(a == b, "probe {p}, bands of {width}");
        }
    }
}

/// A volume with `n` probes, each `mean · (1 + spread · ±1)`.
fn measured(n: usize, mean: f64, spread: f64, grid: &VolumeGrid) -> Volume {
    let s = grid.spacing();
    let mut v = Volume::empty(n);
    for (i, l) in v.lengths.iter_mut().enumerate() {
        let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
        *l = mean * (1.0 + spread * sign) / (s * s);
    }
    v.segments = 100;
    v.finish(grid);
    v
}

#[test]
fn next_steps() {
    let region = Aabb {
        lo: [0.0; 3],
        hi: [10.0; 3],
    };
    let base = VolumeGrid::seeded(region, 1.0, 1);
    let grid = |l: u32| VolumeGrid {
        h: 1.0 / f64::from(l).exp2(),
        ..base.clone()
    };
    let levels = Levels {
        grid: &grid,
        last: 4,
        rel_sigma: 1e-2,
        abs_sigma: 0.0,
        base: 12,
        max_probes: MAX_PROBES,
    };
    // At the target: done.
    let v = measured(12, 100.0, 0.01, &grid(0));
    assert!(v.sigma <= 1e-2 * v.estimate);
    assert_eq!(levels.next_step(&v, 0, None, 10.0), Step::Done);
    // A little above it: more probes at the same level, in blocks.
    let v = measured(12, 100.0, 0.04, &grid(1));
    let r = v.sigma / v.estimate;
    assert!(r > 1e-2 && r < 1.5e-2, "{r}");
    match levels.next_step(&v, 1, None, 10.0) {
        Step::Trace {
            level: 1,
            probes: (12, e),
        } => assert!(e % PROBE_BLOCK == 0 && e > 12 && e <= MAX_PROBES, "{e}"),
        s => panic!("{s:?}"),
    }
    // Far above it: straight to a finer level, fresh.
    let v = measured(12, 100.0, 0.3, &grid(0));
    match levels.next_step(&v, 0, None, 10.0) {
        Step::Trace {
            level,
            probes: (0, 12),
        } => assert!(level >= 2, "{level}"),
        s => panic!("{s:?}"),
    }
    // Out of reach by the last level even at the best rate: given up.
    let v = measured(12, 100.0, 0.9, &grid(3));
    assert_eq!(levels.next_step(&v, 3, None, 10.0), Step::Stop);
    // Not every probe through it: one level finer.
    let mut v = measured(12, 100.0, 0.001, &grid(0));
    v.lengths[0] = 0.0;
    v.finish(&grid(0));
    assert_eq!(
        levels.next_step(&v, 0, None, 10.0),
        Step::Trace {
            level: 1,
            probes: (0, 12)
        }
    );
    // Never met: finer while lines are coarse next to its boxes (a
    // spacing of 2 cm here), not once 1/32 of their longest side.
    let none = Volume::empty(12);
    assert!(matches!(
        levels.next_step(&none, 0, None, 10.0),
        Step::Trace { level: 1, .. }
    ));
    assert_eq!(levels.next_step(&none, 0, None, 1000.0), Step::Stop);
    // An absolute floor above its sigma: done.
    let floor = Levels {
        abs_sigma: 50.0,
        ..levels
    };
    let v = measured(12, 100.0, 0.3, &grid(0));
    assert!(v.sigma <= 50.0);
    assert_eq!(floor.next_step(&v, 0, None, 10.0), Step::Done);
}

/// Points of a lattice cell's frame lie in the box of the element they
/// index, square and hexagonal, two-dimensional too.
#[test]
fn element_boxes_hold_their_elements() {
    let m = Model::from_text(
        "t\n1 0 -1 fill=2 imp:n=1\n9 0 1 imp:n=0\n\
         5 0 -51 52 -53 54 -55 56 lat=2 u=2 fill=3 imp:n=1\n\
         6 0 -61 62 -63 64 -65 66 lat=1 u=4 fill=3 imp:n=1\n\
         10 1 -1.0 -10 u=3 imp:n=1\n11 0 10 u=3 imp:n=1\n\n\
         1 rpp -9 9 -9 9 -9 9\n10 so 0.4\n\
         51 px 1\n52 px -1\n53 p 1 1.7320508075688772 0 2\n\
         54 p 1 1.7320508075688772 0 -2\n55 p -1 1.7320508075688772 0 2\n\
         56 p -1 1.7320508075688772 0 -2\n\
         61 px 1.5\n62 px -0.5\n63 py 1\n64 py -1\n65 pz 0.7\n66 pz -0.3\n\n\
         m1 1001.80c 1\n",
        &Options::default(),
    );
    assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
    let mut rng = Rng(5);
    for lat in &m.lattices {
        for _ in 0..20000 {
            let p = [0, 1, 2].map(|_| 16.0 * rng.next() - 8.0);
            let ix = lat.index(p);
            let b = lat.element_box(ix);
            assert!(b.contains(p), "{p:?} in {ix:?}, not in {b:?}");
            for k in 0..3 {
                assert!(b.hi[k] - b.lo[k] < 3.0 || !b.is_finite(), "{b:?}");
            }
        }
    }
}

/// An instance's box is that of its piece alone: each sphere of a
/// lattice under a transformed fill.
#[test]
fn instances_are_bounded_one_by_one() {
    let m = Model::from_text(
        "t\n1 0 -1 fill=2 (1 0.5 0) imp:n=1\n9 0 1 imp:n=0\n\
         5 0 -5 lat=1 u=2 fill=-1:1 -1:1 0:0 3 3 3 3 3 3 3 3 3 imp:n=1\n\
         10 1 -1.0 -10 u=3 imp:n=1\n11 0 10 u=3 imp:n=1\n\n\
         1 rpp -2 4 -2.5 3.5 -1 1\n5 rpp -1 1 -1 1 -1 1\n10 s 0.3 0.2 0 0.4\n\n\
         m1 1001.80c 1\n",
        &Options::default(),
    );
    let bounder = Bounder::new(&m.shapes);
    let region = Aabb {
        lo: [-5.0; 3],
        hi: [5.0; 3],
    };
    let sphere = m.cell_index(10).unwrap();
    let ids = m.instances().ids(sphere);
    assert_eq!(ids.end - ids.start, 9);
    for id in ids {
        let (c, p) = place_instance(&m, &bounder, region, id).unwrap();
        assert_eq!(c, sphere);
        let ix = m.instance_path(id).unwrap()[1].lattice.unwrap();
        let centre = [
            1.0 + 2.0 * ix[0] as f64 + 0.3,
            0.5 + 2.0 * ix[1] as f64 + 0.2,
            0.0,
        ];
        for k in 0..3 {
            assert!(
                p.global.lo[k] <= centre[k] - 0.4 && p.global.hi[k] >= centre[k] + 0.4,
                "{ix:?}: {:?} misses the sphere",
                p.global
            );
            assert!(
                p.global.hi[k] - p.global.lo[k] <= 0.8 + 1e-3,
                "{ix:?}: {:?} is loose",
                p.global
            );
        }
    }
}
