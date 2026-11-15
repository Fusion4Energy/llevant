//! The 3D view's image (backend → resolve → colour pass) against
//! `llevant_render::shading::render_image`, the CPU render of `llevant render`, on
//! the MCNP zoo models. The CPU backend must agree exactly (up to rounding
//! of the shading); the GPU backend at nearly every pixel. Skipped when no
//! GPU adapter is available (lavapipe is enough).

use std::sync::Arc;

use llevant::{Aabb, Camera, Clip, Cutaway, Model, Options, Scope, Visibility};
use llevant_gpu::GpuModel;
use llevant_render::colours::ColourBy;
use llevant_render::colours::Paint;
use llevant_render::shading::{render_image, Probe};
use llevant_scene::materials::{Looks, Spec};
use llevant_viewer::camera::Orbit;
use llevant_viewer::view3d::backend::{CpuBackend, GpuBackend, TraceBackend};
use llevant_viewer::view3d::View3d;

fn model(name: &str) -> Model {
    let path = format!(
        "{}/../../tools/mcnp-oracle/models/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    Model::load(&path, &Options::default(), None).unwrap()
}

/// Pixels whose colours differ by more than `tol` in some channel.
fn differ(rgba: &[u8], rgb: &[u8], tol: i32) -> usize {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .zip(rgb.as_chunks::<3>().0)
        .filter(|(a, b)| (0..3).any(|k| (a[k] as i32 - b[k] as i32).abs() > tol))
        .count()
}

fn cameras(m: &Model) -> Vec<Camera> {
    let region = m.extent(Scope::Contents, 64);
    let mut out = Vec::new();
    for (yaw, pitch) in [(35.0, 25.0), (200.0, -40.0)] {
        let o = Orbit::fit(&region, yaw, pitch);
        out.push(o.camera(96, 64, None));
        out.push(o.camera(
            83,
            61,
            Some(Clip {
                point: region.center(),
                normal: [0.0, 1.0, 0.0],
            }),
        ));
    }
    // A wedge about z through the middle (three quarters cut away), a
    // section box, and an orthographic view of a quarter cut away.
    let c = region.center();
    let o = Orbit::fit(&region, 60.0, 30.0);
    out.push(o.camera(
        90,
        60,
        Cutaway::wedge(c, [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], 30.0, 300.0),
    ));
    let half = (0..3)
        .map(|k| 0.3 * (region.hi[k] - region.lo[k]))
        .collect::<Vec<_>>();
    let b = Aabb {
        lo: [0, 1, 2].map(|k| c[k] - half[k]),
        hi: [0, 1, 2].map(|k| c[k] + half[k]),
    };
    out.push(o.camera(80, 64, Cutaway::keep_box(&b)));
    let mut ortho = Orbit::fit(&region, 120.0, 20.0);
    ortho.state.ortho = true;
    out.push(ortho.camera(
        88,
        56,
        Cutaway::wedge(c, [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], 0.0, 90.0),
    ));
    out
}

fn check(name: &str) {
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    let m = Arc::new(model(name));
    let vis = Visibility::solid(&m);
    let gm = Arc::new(GpuModel::new(&gpu, &m.export()).unwrap());
    gm.set_visibility(&gpu, &m, &vis).unwrap();
    let backends: Vec<(&str, Box<dyn TraceBackend>)> = vec![
        ("cpu", Box::new(CpuBackend::new(m.clone(), vis.clone()))),
        (
            "gpu",
            Box::new(GpuBackend::new(gpu.clone(), gm.clone(), m.clone())),
        ),
    ];
    for (bname, backend) in backends {
        let looks = Arc::new(Looks::new(&m, &[]).unwrap());
        let mut view = View3d::new(
            gpu.clone(),
            m.clone(),
            looks,
            backend,
            &vis,
            0.02,
            ColourBy::Cell,
        );
        view.set_effect("outline", false);
        // Flat shading as `llevant render` always did, and physically
        // based shading (the WGSL mirror of `llevant_render::shading::surface`),
        // then with water and a half-clear material.
        let mut ids: Vec<i64> = m
            .cells()
            .iter()
            .map(|c| c.material)
            .filter(|&x| x != 0)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let clear: Vec<(i64, Spec)> = ids
            .iter()
            .zip(["water", "steel clarity=0.4"])
            .map(|(&id, s)| (id, s.parse().unwrap()))
            .collect();
        for (by, specs) in [
            (ColourBy::Cell, &[][..]),
            (ColourBy::Realistic, &[][..]),
            (ColourBy::Realistic, &clear[..]),
        ] {
            let looks = Looks::new(&m, specs).unwrap();
            view.set_looks(Arc::new(looks.clone()));
            view.set_colouring(by);
            let paint = Paint::new(&m, &looks, by);
            for cam in cameras(&m) {
                let rgba = view.render_now(cam.clone());
                let hits = m.render(&cam, &vis).unwrap();
                let rgb = render_image(&paint, &Probe::from_hits(&m, &hits), &cam).unwrap();
                let n = cam.width * cam.height;
                assert_eq!(rgba.len(), 4 * n);
                let bad = differ(&rgba, &rgb, 2);
                let pbr = paint.pbr();
                println!(
                    "{name} {bname} pbr={pbr} {}×{}: {bad} of {n} pixels differ",
                    cam.width, cam.height
                );
                if bname == "cpu" {
                    assert_eq!(bad, 0, "{name}, CPU backend, pbr={pbr}, {specs:?}");
                } else {
                    assert!(
                        bad as f64 <= 0.01 * n as f64,
                        "{name}, GPU backend, pbr={pbr}: {bad}"
                    );
                }
            }
        }
    }
}

#[test]
fn universes() {
    check("universes.i");
}

#[test]
fn surfaces() {
    check("surfaces.i");
}

#[test]
fn effects_change_the_image() {
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    let m = Arc::new(model("universes.i"));
    let vis = Visibility::solid(&m);
    let cam = cameras(&m).remove(0);
    let mut view = View3d::new(
        gpu.clone(),
        m.clone(),
        Arc::new(Looks::new(&m, &[]).unwrap()),
        Box::new(CpuBackend::new(m.clone(), vis.clone())),
        &vis,
        0.02,
        ColourBy::Cell,
    );
    view.set_effect("outline", false);
    let plain = view.render_now(cam.clone());
    for fx in ["outline", "ambient_occlusion"] {
        view.set_effect(fx, true);
        let img = view.render_now(cam.clone());
        assert_ne!(img, plain, "{fx}");
        view.set_effect(fx, false);
    }
    // A pass of the caller's: invert the colours.
    view.passes().push(
        &gpu,
        Box::new(llevant_viewer::view3d::passes::WgslPass {
            name: "invert".into(),
            source: "fn effect(p: vec2<i32>) -> vec4<f32> {\n    let c = source(p);\n    return vec4<f32>(vec3<f32>(1.0) - c.rgb, c.a);\n}\n".into(),
            params: [0.0; 12],
        }),
        true,
    ).unwrap();
    // A pass that does not compile is refused with the compiler's message.
    let bad = view.passes().push(
        &gpu,
        Box::new(llevant_viewer::view3d::passes::WgslPass {
            name: "broken".into(),
            source: "fn effect(p: vec2<i32>) -> vec4<f32> {
    return no_such_thing(p);
}
"
            .into(),
            params: [0.0; 12],
        }),
        true,
    );
    let msg = bad.unwrap_err();
    assert!(msg.contains("broken"), "{msg}");
    let inv = view.render_now(cam);
    for (a, b) in inv.as_chunks::<4>().0.iter().zip(plain.as_chunks::<4>().0) {
        for k in 0..3 {
            assert!((a[k] as i32 + b[k] as i32 - 255).abs() <= 1);
        }
    }
}

/// A pass that reads a buffer of its own: every pixel that hits something
/// takes the colour in its first word.
struct Paint1;

impl llevant_viewer::view3d::passes::PostPass for Paint1 {
    fn name(&self) -> &str {
        "paint"
    }
    fn wgsl(&self) -> String {
        "@group(1) @binding(0) var<storage, read> colour: array<u32>;
fn effect(p: vec2<i32>) -> vec4<f32> {
    if !hit_any(p) {
        return source(p);
    }
    return unpack4x8unorm(colour[0]);
}
"
        .into()
    }
    fn data_buffers(&self) -> u32 {
        1
    }
}

#[test]
fn passes_read_buffers_of_their_own() {
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    let m = Arc::new(model("universes.i"));
    let vis = Visibility::solid(&m);
    let cam = cameras(&m).remove(0);
    let mut view = View3d::new(
        gpu.clone(),
        m.clone(),
        Arc::new(Looks::new(&m, &[]).unwrap()),
        Box::new(CpuBackend::new(m.clone(), vis.clone())),
        &vis,
        0.02,
        ColourBy::Cell,
    );
    view.set_effect("outline", false);
    // Right after the base colours.
    view.passes()
        .insert(&gpu, 1, Box::new(Paint1), true)
        .unwrap();
    let count = |img: &[u8], c: [u8; 4]| img.as_chunks::<4>().0.iter().filter(|&&x| x == c).count();
    // Zero until filled.
    let hits = count(&view.render_now(cam.clone()), [0, 0, 0, 0]);
    assert!(hits > 0);
    view.passes()
        .set_data(&gpu, "paint", &[&[200, 10, 20, 255]])
        .unwrap();
    assert_eq!(count(&view.render_now(cam), [200, 10, 20, 255]), hits);
    assert!(view.passes().set_data(&gpu, "paint", &[]).is_err());
    assert!(view.passes().set_data(&gpu, "nothing", &[&[]]).is_err());
}

/// Fields over the view: the GPU pass against `render_image` with the same
/// field, per cell, per instance and on meshes (rectangular and turned,
/// cylindrical, and per voxel and cell). On meshes a few pixels next to
/// voxel boundaries may fall on the other side in f32.
#[test]
fn fields_match_the_cpu() {
    use aci_tallies_geom::{Geometry, Transform};
    use llevant_render::field::Overlay;
    use llevant_scene::field::{CellUnderVoxel, Field, Values};
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    let m = Arc::new(model("universes.i"));
    let vis = Visibility::solid(&m);
    let r = m.extent(Scope::Contents, 64);
    let c = r.center();
    let lin = |lo: f64, hi: f64, n: usize| -> Vec<f64> {
        (0..=n)
            .map(|k| lo + (hi - lo) * k as f64 / n as f64)
            .collect()
    };
    let (a, b) = (
        std::f64::consts::FRAC_PI_6.cos(),
        std::f64::consts::FRAC_PI_6.sin(),
    );
    let turned = Geometry::Rectangular {
        x_bounds: lin(-300.0, 300.0, 9),
        y_bounds: lin(-20.0, 20.0, 3),
        z_bounds: lin(-20.0, 20.0, 4),
        transform: Some(Transform {
            rotation: [[a, -b, 0.0], [b, a, 0.0], [0.0, 0.0, 1.0]],
            translation: c,
        }),
    };
    let cylinder = Geometry::Cylindrical {
        r_bounds: vec![0.0, 4.0, 9.0, 30.0],
        z_bounds: lin(-10.0, 600.0, 7),
        theta_bounds: vec![0.0, 0.25, 0.5, 1.0],
        origin: [r.lo[0], 0.0, 0.0],
        axis: [1.0, 0.0, 0.0],
        vec: [0.0, 0.0, 1.0],
    };
    let n_turned = turned.n_voxels();
    let numbers = |n: usize| (0..n).map(|k| (k % 13) as f32 + 1.0).collect::<Vec<f32>>();
    let cuv = CellUnderVoxel::new(
        n_turned,
        (0..n_turned)
            .flat_map(|v| {
                (0..m.cells().len() as u32)
                    .filter(move |c| (c + v as u32).is_multiple_of(3))
                    .map(move |c| (v, c, (v % 5) as f32 + c as f32))
            })
            .collect(),
    )
    .unwrap();
    let mut fields = vec![
        (
            "cells",
            Field::cells(&m, "c", m.cells().iter().map(|c| (c.id, (c.id % 7) as f64))).unwrap(),
        ),
        (
            "instances",
            Field::instances(
                &m,
                "i",
                (0..m.instances().total()).map(|k| (k, (k % 50) as f64 + 1.0)),
            )
            .unwrap(),
        ),
        (
            "turned mesh",
            Field::mesh("t", "", turned.clone(), numbers(n_turned), None).unwrap(),
        ),
        (
            "cylinder",
            Field::mesh(
                "y",
                "",
                cylinder.clone(),
                numbers(cylinder.n_voxels()),
                None,
            )
            .unwrap(),
        ),
        (
            "cell under voxel",
            Field::mesh("u", "", turned, numbers(n_turned), Some(cuv)).unwrap(),
        ),
    ];
    let mut fields: Vec<(&str, Overlay)> = fields
        .drain(..)
        .map(|(name, f)| (name, Overlay::new(Arc::new(f))))
        .collect();
    // Half covering, on a log scale.
    fields[1].1.look.opacity = 0.5;
    fields[1].1.look.scale = llevant_render::scale::Scale::log(1.0, 50.0);
    // The cells' colours under it, the others on the neutral grey.
    fields[1].1.look.under = llevant_render::field::Under::Geometry;
    // The grey ramp, under which no value is a tinted grey.
    fields[2].1.look.scale.ramp = llevant_render::scale::Ramp::Grey;
    // A threshold, and colours outside a narrowed scale.
    {
        let look = &mut fields[3].1.look;
        let (lo, hi) = (look.scale.lo, look.scale.hi);
        look.threshold = [Some(lo + 0.1 * (hi - lo)), None];
        look.scale.lo = lo + 0.3 * (hi - lo);
        look.scale.hi = lo + 0.7 * (hi - lo);
        look.below = Some([0, 0, 255]);
        look.above = Some([255, 0, 0]);
    }
    let looks = Arc::new(Looks::new(&m, &[]).unwrap());
    let gm = Arc::new(GpuModel::new(&gpu, &m.export()).unwrap());
    gm.set_visibility(&gpu, &m, &vis).unwrap();
    for (bname, backend) in [
        (
            "cpu",
            Box::new(CpuBackend::new(m.clone(), vis.clone())) as Box<dyn TraceBackend>,
        ),
        (
            "gpu",
            Box::new(GpuBackend::new(gpu.clone(), gm.clone(), m.clone())),
        ),
    ] {
        let mut view = View3d::new(
            gpu.clone(),
            m.clone(),
            looks.clone(),
            backend,
            &vis,
            0.02,
            ColourBy::Material,
        );
        view.set_effect("outline", false);
        for (fname, f) in &fields {
            let mesh = matches!(f.field.values, Values::Mesh(_));
            view.set_field(Some(f.clone())).unwrap();
            let paint = Paint::new(&m, &looks, ColourBy::Material).with_field(Some(f));
            let mut covered = 0;
            for cam in cameras(&m) {
                let rgba = view.render_now(cam.clone());
                let hits = m.render(&cam, &vis).unwrap();
                let probes = Probe::from_hits(&m, &hits);
                let rgb = render_image(&paint, &probes, &cam).unwrap();
                let plain =
                    render_image(&Paint::new(&m, &looks, ColourBy::Material), &probes, &cam)
                        .unwrap();
                covered += (rgb != plain) as usize;
                let n = cam.width * cam.height;
                let bad = differ(&rgba, &rgb, 2);
                println!(
                    "{fname} {bname} {}×{}: {bad} of {n} pixels differ",
                    cam.width, cam.height
                );
                let allowed = match (bname, mesh) {
                    ("cpu", false) => 0.0,
                    ("cpu", true) => 0.005,
                    (_, false) => 0.01,
                    (_, true) => 0.01,
                };
                assert!(
                    bad as f64 <= allowed * n as f64,
                    "{fname}, {bname}: {bad} of {n}"
                );
            }
            assert!(covered > 0, "{fname}: the field shows nowhere");
        }
        view.set_field(None).unwrap();
    }
}

/// Levels and ghost cells: the view's image against `render_image` with
/// `llevant_render::ghost::overlay` over it, at level 0, and with every
/// filled level-0 cell a ghost (what fills them shown), on both backends.
/// With `LLEVANT_VIEWER_TEST_PNG=<file>`, the images are saved beside it.
#[test]
fn levels_and_ghosts() {
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    let m = Arc::new(model("universes.i"));
    let filled: Vec<i64> = m.units()[0]
        .cells
        .iter()
        .map(|&c| m.cell(c))
        .filter(|c| c.fill.is_some())
        .map(|c| c.id)
        .collect();
    let mut ghosts = Visibility::solid(&m);
    ghosts.ghost(&m, &filled).unwrap();
    let level0 = Visibility {
        level: Some(0),
        ..Visibility::solid(&m)
    };
    let looks = Arc::new(Looks::new(&m, &[]).unwrap());
    let paint = Paint::new(&m, &looks, ColourBy::Cell);
    let gm = Arc::new(GpuModel::new(&gpu, &m.export()).unwrap());
    for (vname, vis) in [("ghosts", &ghosts), ("level 0", &level0)] {
        for bname in ["cpu", "gpu"] {
            let backend: Box<dyn TraceBackend> = match bname {
                "cpu" => Box::new(CpuBackend::new(m.clone(), vis.clone())),
                _ => Box::new(GpuBackend::new(gpu.clone(), gm.clone(), m.clone())),
            };
            let mut view = View3d::new(
                gpu.clone(),
                m.clone(),
                looks.clone(),
                backend,
                vis,
                0.02,
                ColourBy::Cell,
            );
            view.set_effect("outline", false);
            view.set_visibility(vis).unwrap();
            let mut total = 0;
            for (k, cam) in cameras(&m).into_iter().enumerate() {
                let rgba = view.render_now(cam.clone());
                let hits = m.render(&cam, vis).unwrap();
                assert_eq!(hits.ghosts.is_empty(), !vis.has_ghosts());
                let mut rgb = render_image(&paint, &Probe::from_hits(&m, &hits), &cam).unwrap();
                llevant_render::ghost::overlay(&paint, &hits.ghosts, &cam, &mut rgb);
                let n = cam.width * cam.height;
                let bad = differ(&rgba, &rgb, 3);
                let crossed = hits.ghosts.iter().filter(|g| g.count > 0).count();
                println!("{vname} {bname} view {k}: {bad} of {n} differ, {crossed} ghosted");
                total += crossed;
                if let Some(p) = std::env::var_os("LLEVANT_VIEWER_TEST_PNG") {
                    let name = format!("{}.{bname}{k}.png", vname.replace(' ', ""));
                    let rgb: Vec<u8> = rgba
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .flat_map(|p| [p[0], p[1], p[2]])
                        .collect();
                    llevant_render::png::write_png(
                        &std::path::PathBuf::from(&p).with_extension(name),
                        cam.width,
                        cam.height,
                        &rgb,
                    )
                    .unwrap();
                }
                if bname == "cpu" {
                    assert!(
                        bad as f64 <= 0.001 * n as f64,
                        "{vname}, CPU backend: {bad}"
                    );
                } else {
                    assert!(bad as f64 <= 0.01 * n as f64, "{vname}, GPU backend: {bad}");
                }
            }
            assert_eq!(
                total > 100,
                vis.has_ghosts(),
                "{vname}: {total} rays cross a ghost"
            );
        }
    }
}

/// The hits shown, moved to another camera (`view3d::reproject`): to the
/// same camera, exactly the image; to one turned a few degrees, nearly the
/// image traced from there (what was hidden, and edges, differ).
#[test]
fn reprojection_follows_the_camera() {
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    for name in ["universes.i", "surfaces.i", "periodic.i"] {
        let m = Arc::new(model(name));
        let vis = Visibility::solid(&m);
        let looks = Arc::new(Looks::new(&m, &[]).unwrap());
        let mut view = View3d::new(
            gpu.clone(),
            m.clone(),
            looks,
            Box::new(CpuBackend::new(m.clone(), vis.clone())),
            &vis,
            0.02,
            ColourBy::Cell,
        );
        view.set_effect("outline", false);
        let region = m.extent(Scope::Contents, 64);
        let mut o = Orbit::fit(&region, 35.0, 25.0);
        let a = o.camera(160, 120, None);
        let traced = view.render_now(a.clone());
        let same = view.reproject_now(a.clone());
        assert_eq!(differ(&traced, &rgb(&same), 2), 0, "{name}: moved nowhere");
        o.orbit(3.0 / 0.3, 0.0);
        let b = o.camera(160, 120, None);
        let moved = view.reproject_now(b.clone());
        let fresh = view.render_now(b);
        let n = differ(&moved, &rgb(&fresh), 8);
        let total = 160 * 120;
        eprintln!("{name}: {n} of {total} pixels differ after a 3° turn");
        assert!(n * 100 < 8 * total, "{name}: {n} of {total} pixels differ");
    }
}

/// A geometry being edited over the 3D image (`view3d::edits`): a surface
/// marked draws a line where it cuts the faces seen, a shell traced apart
/// is laid over them with its outline, and nothing marked leaves the image
/// as it was.
#[test]
fn edit_marks_drawn() {
    use llevant_viewer::view3d::edits::{surface_words, Shell};
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    let m = Arc::new(Model::from_text(
        "edits
1 1 -1.0 -1 -2 imp:n=1
2 1 -1.0 -1 2 imp:n=1
3 0 1 -9 imp:n=1
9 0 9 imp:n=0

1 so 5
2 px 0
9 so 50

m1 1001.80c 1
",
        &Options::default(),
    ));
    assert!(m.diagnostics().is_empty(), "{:?}", m.diagnostics());
    let vis = Visibility::solid(&m);
    let region = Aabb {
        lo: [-6.0; 3],
        hi: [6.0; 3],
    };
    let cam = Orbit::fit(&region, 30.0, 20.0).camera(120, 90, None);
    let mut view = View3d::new(
        gpu.clone(),
        m.clone(),
        Arc::new(Looks::new(&m, &[]).unwrap()),
        Box::new(CpuBackend::new(m.clone(), vis.clone())),
        &vis,
        0.02,
        ColourBy::Cell,
    );
    // Without its outlines, so that only marks are lines.
    view.set_effect("outline", false);
    let plain = view.render_now(cam.clone());
    let count = |img: &[u8], c: [u8; 3]| {
        img.as_chunks::<4>()
            .0
            .iter()
            .filter(|x| (0..3).all(|k| (x[k] as i32 - c[k] as i32).abs() <= 1))
            .count()
    };
    const MARKED: [u8; 3] = [96, 160, 240];
    const EDITED: [u8; 3] = [240, 150, 60];
    assert_eq!(count(&plain, MARKED), 0);

    // Surface 2 (px 0): a line across the ball, a few pixels wide.
    let c1 = m.cell_index(1).unwrap();
    let s2 = m
        .cell_surfaces(c1)
        .into_iter()
        .map(|(s, _)| s)
        .find(|&s| m.surface_id(s).unwrap().id == 2)
        .unwrap();
    let words = surface_words(&[m.shape_record(s2, None).unwrap()], None);
    view.set_edit_marks(words.clone(), None);
    let marked = view.render_now(cam.clone());
    let line = count(&marked, MARKED);
    assert!(line > 30 && line < 120 * 90 / 8, "{line}");
    // Every marked pixel next to x = 0 on the ball (a pixel or two away,
    // more where the ball is seen edge on).
    let hits = m.render(&cam, &vis).unwrap();
    let mut off: Vec<f64> = Vec::new();
    for (k, px) in marked.as_chunks::<4>().0.iter().enumerate() {
        if (0..3).all(|i| (px[i] as i32 - MARKED[i] as i32).abs() <= 1) {
            let (i, j) = (k % 120, k / 120);
            let p = cam.point(i, j, hits.t[k]);
            assert!(p[0].abs() < 1.5, "{p:?} at ({i}, {j})");
            off.push(p[0].abs());
        }
    }
    off.sort_by(f64::total_cmp);
    assert!(off[off.len() / 2] < 0.25, "{off:?}");

    // A ball of radius 2 about (3, 0, 0) of a model of its own: its shell
    // over the image, outlined.
    let alone = Model::from_text(
        "alone\n1 0 -1 imp:n=1\n2 0 1 imp:n=0\n\n1 s 3 0 0 2\n\n",
        &Options::default(),
    );
    let shell = Shell::trace(
        &alone,
        alone.cell_index(1).unwrap(),
        &cam,
        &llevant::Affine::IDENTITY,
    )
    .unwrap();
    assert!(shell.words.iter().step_by(2).any(|&t| t > 0.0));
    view.set_edit_marks(words, Some(Arc::new(shell)));
    let both = view.render_now(cam.clone());
    assert!(count(&both, EDITED) > 20, "{}", count(&both, EDITED));
    assert!(count(&both, MARKED) > 0);
    // Nothing marked: the image as it was.
    view.set_edit_marks(Vec::new(), None);
    assert_eq!(differ(&view.render_now(cam), &rgb(&plain), 0), 0);
}

/// RGB of RGBA.
fn rgb(rgba: &[u8]) -> Vec<u8> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

/// A full-resolution batch of a generation the view has moved on from
/// comes back cancelled; a preview of that generation still lands (an
/// older preview is shown, moved to the camera).
#[test]
fn stale_full_batches_are_cancelled() {
    use llevant_gpu::Tile;
    use llevant_viewer::view3d::budget::{Batch, Level};
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    let m = Arc::new(model("universes.i"));
    let vis = Visibility::solid(&m);
    let gm = Arc::new(GpuModel::new(&gpu, &m.export()).unwrap());
    gm.set_visibility(&gpu, &m, &vis).unwrap();
    let backends: Vec<(&str, Box<dyn TraceBackend>)> = vec![
        ("cpu", Box::new(CpuBackend::new(m.clone(), vis.clone()))),
        (
            "gpu",
            Box::new(GpuBackend::new(gpu.clone(), gm.clone(), m.clone())),
        ),
    ];
    let cam = cameras(&m).remove(0);
    let (w, h) = (cam.width as u32, cam.height as u32);
    for (name, mut b) in backends {
        b.cancel_full_before(2);
        for (level, cancelled) in [
            (Level::Full, true),
            (
                Level::Low {
                    width: w,
                    height: h,
                },
                false,
            ),
        ] {
            let batch = Batch {
                generation: 1,
                level,
                tiles: vec![Tile::full(w, h)],
                completes_level: true,
            };
            b.submit(&cam, batch).unwrap();
            // Pieces until the batch's end.
            let mut pixels = 0;
            let f = 'end: loop {
                for f in b.poll() {
                    if !f.more {
                        break 'end f;
                    }
                    pixels += f.pieces.iter().map(|(t, _)| t.pixels()).sum::<u64>();
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            };
            assert_eq!(f.cancelled, cancelled, "{name} {level:?}");
            assert_eq!(pixels == 0, cancelled, "{name} {level:?}");
        }
    }
}
