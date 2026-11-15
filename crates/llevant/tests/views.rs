//! Slices and renders: pixels must hold what points and rays give.

use llevant::Depth;
use llevant::{
    Aabb, Axis, Camera, Clip, Cutaway, Model, Options, Scope, Slice, Status, Visibility, NO_HIT,
};

mod common;
use common::{random_model_with, Rng};

fn region() -> Aabb {
    Aabb {
        lo: [-11.0; 3],
        hi: [11.0; 3],
    }
}

#[test]
fn slice_pixels_are_the_points_at_their_centres() {
    for seed in 1..25u64 {
        let text = random_model_with(seed, seed % 2 == 0);
        let m = Model::from_text(&text, &Options::default());
        let mut r = Rng((seed * 7919) | 1);
        let s = if seed % 3 == 0 {
            // An oblique plane through the middle.
            let u = llevant::vec3::normalize([1.0, r.range(-1.0, 1.0), 0.3]);
            let w = llevant::vec3::normalize([r.range(-1.0, 1.0), 0.2, 1.0]);
            let v = llevant::vec3::normalize(llevant::vec3::cross(w, u));
            Slice {
                origin: [-8.0, -8.0, -8.0],
                u,
                v,
                width: 16.0,
                height: 13.0,
                nx: 61,
                ny: 47,
            }
        } else {
            Slice::axis(
                [Axis::X, Axis::Y, Axis::Z][seed as usize % 3],
                r.range(-4.0, 4.0),
                &region(),
                64,
            )
        };
        let tr = m.slice(&s, Depth::All).unwrap();
        let px = s.pixels(&tr);
        assert_eq!(px.len(), s.nx * s.ny);
        let centres: Vec<[f64; 3]> = (0..s.ny)
            .flat_map(|j| (0..s.nx).map(move |i| (i, j)))
            .map(|(i, j)| s.pixel_centre(i, j))
            .collect();
        let pts = m.classify(&centres, Depth::All);
        for (k, &id) in px.iter().enumerate() {
            assert_eq!(
                &tr.labels[id as usize], &pts.labels[pts.ids[k] as usize],
                "seed {seed}, pixel {k} at {:?}\n{text}",
                centres[k]
            );
        }
    }
}

#[test]
fn axis_slices_keep_pixels_square() {
    let b = Aabb {
        lo: [0.0, -5.0, 2.0],
        hi: [40.0, 15.0, 12.0],
    };
    let s = Slice::axis(Axis::Y, 3.0, &b, 400);
    assert_eq!((s.nx, s.ny), (400, 100));
    assert_eq!(s.origin, [0.0, 3.0, 2.0]);
    assert_eq!(s.pixel_centre(0, 0), [0.05, 3.0, 11.95]);
}

#[test]
fn render_is_first_hit_of_the_camera_rays() {
    for seed in 1..13u64 {
        let text = random_model_with(seed, true);
        let m = Model::from_text(&text, &Options::default());
        let vis = Visibility::solid(&m);
        let cam = Camera::fit(&region(), [-0.6, -0.7, 0.4], 48, 32);
        let h = m.render(&cam, &vis).unwrap();
        let rays: Vec<_> = cam.rays().into_iter().map(Option::unwrap).collect();
        let f = m.first_hits(&rays, &vis, None).unwrap();
        assert_eq!(h.t, f.t);
        assert_eq!(h.surface, f.surface);
        assert_eq!(h.normal, f.normal);
        for k in 0..rays.len() {
            assert_eq!(h.ids[k] == NO_HIT, f.ids[k] == NO_HIT);
            if h.ids[k] != NO_HIT {
                assert_eq!(h.labels[h.ids[k] as usize], f.labels[f.ids[k] as usize]);
            }
        }
    }
}

#[test]
fn cutaways_hit_only_the_kept_side() {
    let mut shown = 0;
    for seed in 1..13u64 {
        let text = random_model_with(seed, true);
        let m = Model::from_text(&text, &Options::default());
        let mut vis = Visibility::all(&m);
        vis.gaps = false;
        // Looking along -y from far out: the near half (y > 1) is cut away.
        let mut cam = Camera::fit(&region(), [0.3, -1.0, -0.2], 40, 40);
        let clip = Clip {
            point: [0.0, 1.0, 0.0],
            normal: [0.0, 1.0, 0.0],
        };
        cam.cut = Cutaway::plane(clip);
        let h = m.render(&cam, &vis).unwrap();
        for (k, r) in cam.rays().iter().enumerate() {
            if h.ids[k] == NO_HIT {
                continue;
            }
            let r = r.unwrap();
            let p = r.at(h.t[k]);
            assert!(p[1] <= 1.0 + 1e-9, "seed {seed}: hit at {p:?}");
            if h.t[k] == r.t_min && r.t_min > 0.0 {
                // Cut face: shaded by the clip normal, which faces the ray.
                assert_eq!(h.normal[k], clip.normal);
                shown += 1;
            }
        }
    }
    assert!(shown > 0, "no ray started on the cut");
}

#[test]
fn bounds_cover_the_contents_only() {
    let m = Model::from_text(
        "t
1 1 -1.0 -1 imp:n=1
2 1 -1.0 1 -9 imp:n=1
3 0 9 -2 imp:n=1
4 0 2 3 imp:n=0
5 0 -3 imp:n=0

1 so 2
2 so 300
3 rpp 500 510 0 1 0 1
9 rpp -5 5 -6 6 -7 7

m1 1001.32c 1
",
        &Options::default(),
    );
    let b = m.bounds(Scope::Contents);
    let p = m.bounds(Scope::Problem);
    // Cells 1 and 2 hold something; the void around them (cell 3, bounded
    // by surface 2), the outside (unbounded) and cell 5 (IMP:N=0) do not
    // count.
    assert!(b.lo[0] <= -5.0 && b.hi[0] >= 5.0 && b.hi[2] >= 7.0, "{b:?}");
    assert!(b.hi[0] < 6.0 && b.lo[2] > -8.0, "{b:?}");
    // The problem includes the void cell 3, out to surface 2.
    assert!(
        p.lo[0] <= -300.0 && p.hi[0] >= 300.0 && p.hi[0] < 400.0,
        "{p:?}"
    );
}

#[test]
fn extent_is_measured_inside_loose_boxes() {
    // The void around the material (cell 2) reaches 1 km; the material is
    // within x, y, z in [-5, 5].
    let m = Model::from_text(
        "t
1 1 -1.0 -1 imp:n=1
2 0 1 -2 imp:n=1
3 0 2 imp:n=0

1 rpp -5 5 -5 5 -5 5
2 so 100000

m1 1001.32c 1
",
        &Options::default(),
    );
    let e = m.extent(Scope::Contents, 64);
    for i in 0..3 {
        assert!(e.lo[i] <= -5.0 && e.lo[i] > -6.0, "{e:?}");
        assert!(e.hi[i] >= 5.0 && e.hi[i] < 6.0, "{e:?}");
    }
}

#[test]
fn wedges_and_boxes_remove_what_they_say() {
    let o = [1.0, 2.0, 0.0];
    let at = |deg: f64, r: f64| {
        let (s, c) = deg.to_radians().sin_cos();
        [o[0] + r * c, o[1] + r * s, 5.0]
    };
    let z = [0.0, 0.0, 1.0];
    let x = [1.0, 0.0, 0.0];
    // A quarter, three quarters, one across the zero, the whole turn.
    for (from, to) in [(0.0, 90.0), (30.0, 300.0), (350.0, 10.0), (0.0, 360.0)] {
        let w = Cutaway::wedge(o, z, x, from, to);
        for deg in (0..360).map(|d| d as f64 + 0.5) {
            let want = to - from >= 360.0
                || if from < to {
                    deg > from && deg < to
                } else {
                    deg > from || deg < to
                };
            assert_eq!(w.removes(at(deg, 3.0)), want, "{from}..{to} at {deg}");
        }
    }
    assert!(Cutaway::wedge(o, z, x, 40.0, 40.0).is_empty());
    let b = Aabb {
        lo: [0.0; 3],
        hi: [1.0, 2.0, 3.0],
    };
    let (keep, remove) = (Cutaway::keep_box(&b), Cutaway::remove_box(&b));
    for (p, inside) in [
        ([0.5, 1.0, 1.0], true),
        ([1.5, 1.0, 1.0], false),
        ([0.5, 1.0, -1.0], false),
    ] {
        assert_eq!(keep.removes(p), !inside, "{p:?}");
        assert_eq!(remove.removes(p), inside, "{p:?}");
    }
    // Along a ray through the box: kept before and after it, the second
    // part starting on the far face.
    let k = remove.kept([-1.0, 1.0, 1.0], [1.0, 0.0, 0.0], 0.0, 5.0);
    assert_eq!(k.len(), 2);
    assert_eq!((k[0].t0, k[0].t1, k[0].normal), (0.0, 1.0, None));
    assert_eq!((k[1].t0, k[1].t1), (2.0, 5.0));
    assert_eq!(k[1].normal, Some([-1.0, 0.0, 0.0]));
}

/// Whether a point shows with `Visibility::solid` and gaps hidden: in a
/// cell with material, or in a problem.
fn shows(m: &Model, l: &llevant::Label) -> bool {
    match l.status {
        Status::Inside => l.cell().is_some_and(|c| m.cell(c).material != 0),
        Status::Overlap | Status::Unsupported | Status::TooDeep => true,
        _ => false,
    }
}

#[test]
fn cut_renders_see_the_first_shown_point_that_is_kept() {
    let mut caps = 0;
    for seed in 1..10u64 {
        let text = random_model_with(seed, true);
        let m = Model::from_text(&text, &Options::default());
        let mut vis = Visibility::solid(&m);
        vis.gaps = false;
        let cuts = [
            Cutaway::wedge([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], 0.0, 90.0),
            Cutaway::wedge(
                [0.5, -0.3, 0.0],
                [0.1, 0.2, 1.0],
                [1.0, 0.0, 0.0],
                20.0,
                290.0,
            ),
            Cutaway::keep_box(&Aabb {
                lo: [-6.0, -5.0, -4.0],
                hi: [5.0, 4.0, 6.0],
            }),
            Cutaway::remove_box(&Aabb {
                lo: [-3.0, -9.0, -3.0],
                hi: [3.0, 3.0, 3.0],
            }),
        ];
        for (c, cut) in cuts.into_iter().enumerate() {
            let mut cam = Camera::fit(&region(), [-0.6, -0.7, 0.4], 24, 18);
            if c % 2 == 1 {
                cam.ortho = Some(24.0);
            }
            cam.cut = cut.clone();
            let h = m.render(&cam, &vis).unwrap();
            for j in 0..cam.height {
                for i in 0..cam.width {
                    let k = j * cam.width + i;
                    let hit = (h.ids[k] != NO_HIT).then_some(h.t[k]);
                    // Points of the kept parts before the hit (all of them
                    // without one) do not show.
                    for (line, _) in cam.segments(i, j) {
                        let end = hit.map_or(line.t_max, |t| t.min(line.t_max));
                        let n = 60;
                        for s in 1..n {
                            let t = line.t_min + (end - line.t_min) * s as f64 / n as f64;
                            if hit.is_some_and(|h| h - t < 1e-6) {
                                continue;
                            }
                            let p = cam.point(i, j, t);
                            let l = m.classify_point(p);
                            assert!(
                                !shows(&m, &l),
                                "seed {seed}, cut {c}, pixel {i} {j}: {p:?} shows before {hit:?}"
                            );
                        }
                        if hit.is_some_and(|h| h <= line.t_max) {
                            break;
                        }
                    }
                    if let Some(t) = hit {
                        let p = cam.point(i, j, t);
                        let on_cut = cut.regions.iter().flatten().any(|c| {
                            let n = llevant::vec3::normalize(c.normal);
                            llevant::vec3::dot(llevant::vec3::sub(p, c.point), n).abs() < 1e-9
                        });
                        assert!(
                            !cut.removes(p) || on_cut,
                            "seed {seed}, cut {c}: hit at {p:?}"
                        );
                        if h.surface[k] == llevant::NO_SURFACE && on_cut {
                            caps += 1;
                            // The cut face faces the ray.
                            let d = cam.direction(i, j);
                            assert!(llevant::vec3::dot(h.normal[k], d) < 0.0);
                        }
                    }
                }
            }
        }
    }
    assert!(caps > 0, "no ray hit a cut face");
}

#[test]
fn orthographic_rays_are_parallel_across_the_view() {
    let mut cam = Camera::fit(&region(), [0.0, -1.0, 0.0], 40, 20);
    cam.ortho = Some(10.0);
    let d = cam.direction(0, 0);
    assert_eq!(d, cam.direction(39, 19));
    let (a, b) = (cam.origin(0, 10), cam.origin(39, 10));
    // 40 pixels across a view 10 cm high and 20 cm wide.
    assert!((llevant::vec3::norm(llevant::vec3::sub(b, a)) - 19.5).abs() < 1e-9);
    assert!(llevant::vec3::dot(llevant::vec3::sub(b, a), d).abs() < 1e-12);
}

#[test]
fn radial_slices_hold_the_axis_and_turn_about_it() {
    let (p, z) = ([1.0, -2.0, 0.5], [0.0, 0.0, 1.0]);
    for phi in [0.0, 30.0, 90.0, 200.0] {
        let s = Slice::radial(p, z, [1.0, 0.0, 0.0], phi, 8.0, 6.0, 40).unwrap();
        assert_eq!((s.nx, s.ny), (40, 30));
        let (sn, cs) = f64::to_radians(phi).sin_cos();
        // Every pixel centre is on the plane through the axis at phi.
        let n = [-sn, cs, 0.0];
        for (i, j) in [(0, 0), (39, 29), (20, 3)] {
            let q = llevant::vec3::sub(s.pixel_centre(i, j), p);
            assert!(llevant::vec3::dot(q, n).abs() < 1e-12, "{phi}: {q:?}");
        }
        // Rows run out along the radius, columns up the axis.
        assert!((llevant::vec3::dot(s.u, [cs, sn, 0.0]) - 1.0).abs() < 1e-12);
        assert_eq!(s.v, z);
        // Centred on the point.
        let mid = llevant::vec3::add(
            s.origin,
            llevant::vec3::add(
                llevant::vec3::scale(s.u, 4.0),
                llevant::vec3::scale(s.v, 3.0),
            ),
        );
        assert!(llevant::vec3::norm(llevant::vec3::sub(mid, p)) < 1e-12);
    }
    assert!(Slice::radial(p, z, z, 0.0, 1.0, 1.0, 4).is_none());
    assert!(Slice::plane(p, z, z, 1.0, 1.0, 4).is_none());
}
