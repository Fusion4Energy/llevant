//! First hits on small hand-written models: positions, normals through
//! rotated universes, and visibility.

use llevant::{Line, Model, Options, Visibility, NO_HIT, NO_SURFACE};

fn model(text: &str) -> Model {
    let m = Model::from_text(text, &Options::default());
    assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
    m
}

fn ray(o: [f64; 3], d: [f64; 3]) -> Line {
    let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    Line {
        origin: o,
        dir: d.map(|x| x / n),
        t_min: 0.0,
        t_max: 1000.0,
    }
}

/// A box holding a universe rotated 30° about z and moved to (20, 0, 0):
/// a sphere of radius 3 at (4, 0, 0) of the universe, and a torus about
/// the universe's x axis at its origin.
const ROTATED: &str = "\
t
1 0 -1 *fill=2 (20 0 0  30 60 90  120 30 90  90 90 0) imp:n=1
2 0 1 -9 imp:n=1
9 0 9 imp:n=0
21 1 -1.0 -21 u=2 imp:n=1
22 1 -1.0 -22 u=2 imp:n=1
23 0 21 22 u=2 imp:n=1

1 rpp 5 35 -15 15 -15 15
9 so 200
21 s 4 0 0 3
22 tx 0 -8 0 4 1 1

m1 1001.80c 1
";

#[test]
fn hits_and_normals_through_a_rotated_universe() {
    let m = model(ROTATED);
    // The sphere's centre in the global frame: (20, 0, 0) + R(30°)·(4, 0, 0).
    let (s, c) = (30f64.to_radians().sin(), 30f64.to_radians().cos());
    let centre = [20.0 + 4.0 * c, 4.0 * s, 0.0];
    let vis = Visibility::solid(&m);
    let rays: Vec<Line> = (0..200)
        .map(|i| {
            let a = i as f64 * 0.031;
            let target = [
                centre[0] + 2.0 * a.cos(),
                centre[1] + 2.0 * a.sin(),
                1.0 * (a * 3.0).sin(),
            ];
            let o = [-50.0, 7.0 * (i as f64).sin(), 3.0];
            ray(o, [0, 1, 2].map(|k| target[k] - o[k]))
        })
        .collect();
    let h = m.first_hits(&rays, &vis, None).unwrap();
    let sphere = m.cell_index(21).unwrap();
    let mut on_sphere = 0;
    for (i, r) in rays.iter().enumerate() {
        assert_ne!(h.ids[i], NO_HIT, "ray {i}");
        let label = &h.labels[h.ids[i] as usize];
        let p = [0, 1, 2].map(|k| r.origin[k] + h.t[i] * r.dir[k]);
        if label.cell() == Some(sphere) {
            on_sphere += 1;
            let rad = [0, 1, 2].map(|k| (p[k] - centre[k]) / 3.0);
            let len = (rad[0] * rad[0] + rad[1] * rad[1] + rad[2] * rad[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-9, "ray {i}: hit off the sphere");
            for k in 0..3 {
                assert!(
                    (h.normal[i][k] - rad[k]).abs() < 1e-9,
                    "ray {i}: {:?} vs {rad:?}",
                    h.normal[i]
                );
            }
        }
    }
    assert!(on_sphere > 150, "{on_sphere}");
}

#[test]
fn torus_normals_are_perpendicular_to_the_surface() {
    let m = model(ROTATED);
    let vis = Visibility::solid(&m);
    let torus = m.cell_index(22).unwrap();
    let rays: Vec<Line> = (0..400)
        .map(|i| {
            let a = i as f64 * 0.7;
            ray(
                [20.0 + 3.0 * a.sin(), -40.0, 2.0 * a.cos()],
                [0.3 * a.cos(), 1.0, 0.2 * a.sin()],
            )
        })
        .collect();
    let h = m.first_hits(&rays, &vis, None).unwrap();
    let mut n_torus = 0;
    for (i, r) in rays.iter().enumerate() {
        if h.ids[i] == NO_HIT || h.labels[h.ids[i] as usize].cell() != Some(torus) {
            continue;
        }
        n_torus += 1;
        // Points just off the hit point along the surface stay on it: the
        // classification changes across the normal, not along it.
        let p = [0, 1, 2].map(|k| r.origin[k] + h.t[i] * r.dir[k]);
        let n = h.normal[i];
        let inside = m.classify_point([0, 1, 2].map(|k| p[k] - 1e-6 * n[k]));
        let outside = m.classify_point([0, 1, 2].map(|k| p[k] + 1e-6 * n[k]));
        assert_eq!(inside.cell(), Some(torus), "ray {i}");
        assert_ne!(outside.cell(), Some(torus), "ray {i}");
        assert!(n[0] * r.dir[0] + n[1] * r.dir[1] + n[2] * r.dir[2] < 0.0);
    }
    assert!(n_torus > 20, "{n_torus}");
}

#[test]
fn hidden_cells_hide_their_contents() {
    let m = model(ROTATED);
    let r = ray([-50.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    let mut vis = Visibility::solid(&m);
    let h = m.first_hits(&[r], &vis, None).unwrap();
    assert_ne!(h.ids[0], NO_HIT);
    // Hide the box that holds the universe: nothing left to hit.
    vis.cells[m.cell_index(1).unwrap() as usize] = false;
    let h = m.first_hits(&[r], &vis, None).unwrap();
    assert_eq!(
        (h.ids[0], h.t[0], h.surface[0]),
        (NO_HIT, f64::INFINITY, NO_SURFACE)
    );
    // Everything visible: the void outside the graveyard sphere is hit at
    // once, without a surface.
    let h = m
        .first_hits(
            &[ray([-500.0, 0.0, 0.0], [1.0, 0.0, 0.0])],
            &Visibility::all(&m),
            None,
        )
        .unwrap();
    assert_eq!(
        (h.t[0], h.surface[0], h.normal[0]),
        (0.0, NO_SURFACE, [0.0; 3])
    );
}

/// Two overlaps: cells 1 and 2 at x = -9, cells 3 and 4 at x = 11.
const TWO_OVERLAPS: &str = "\
t
1 1 -1.0 -1 imp:n=1
2 1 -1.0 -2 imp:n=1
3 1 -1.0 -3 imp:n=1
4 1 -1.0 -4 imp:n=1
8 0 1 2 3 4 -9 imp:n=1
9 0 9 imp:n=0

1 s -10 0 0 3
2 s -8 0 0 3
3 s 10 0 0 3
4 s 12 0 0 3
9 so 100

m1 1001.80c 1
";

#[test]
fn problem_cells_keep_only_their_problems() {
    let m = model(TWO_OVERLAPS);
    let rays = [
        ray([-9.0, -50.0, 0.0], [0.0, 1.0, 0.0]),
        ray([11.0, -50.0, 0.0], [0.0, 1.0, 0.0]),
    ];
    let status = |vis: &Visibility| -> Vec<Option<llevant::Status>> {
        let h = m.first_hits(&rays, vis, None).unwrap();
        h.ids
            .iter()
            .map(|&i| (i != NO_HIT).then(|| h.labels[i as usize].status))
            .collect()
    };
    let overlap = Some(llevant::Status::Overlap);
    let all = Visibility::solid(&m);
    assert_eq!(status(&all), [overlap, overlap]);
    // Only the problems of cell 1: the overlap of 3 and 4 is gone, and
    // nothing is behind it.
    let mut mask = vec![false; m.cells().len()];
    mask[m.cell_index(1).unwrap() as usize] = true;
    let one = Visibility {
        problem_cells: mask,
        ..Visibility::solid(&m)
    };
    assert_eq!(status(&one), [overlap, None]);
    let wrong = Visibility {
        problem_cells: vec![true; 2],
        ..Visibility::solid(&m)
    };
    assert!(m.first_hits(&rays, &wrong, None).is_err());
}
