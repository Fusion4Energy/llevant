//! GPU time of composing a traced view with each combination of effects,
//! with flat and with physically based shading, then with the marks of a
//! geometry being edited (8 surfaces, and a shell over the whole image):
//! `cargo run --release -p llevant-viewer --example effects -- model.mcnp [1920 1080]`.

use std::sync::Arc;

use llevant::{Model, Options, Scope, Visibility};
use llevant_gpu::{Gpu, GpuModel};
use llevant_render::colours::ColourBy;
use llevant_scene::materials::Looks;
use llevant_viewer::camera::{visible_extent, Orbit};
use llevant_viewer::view3d::backend::GpuBackend;
use llevant_viewer::view3d::edits::{surface_words, Shell};
use llevant_viewer::view3d::View3d;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("model");
    let w: usize = args.get(2).map_or(1920, |s| s.parse().unwrap());
    let h: usize = args.get(3).map_or(1080, |s| s.parse().unwrap());
    let m = Arc::new(Model::load(path, &Options::default(), None).unwrap());
    let vis = Visibility::solid(&m);
    let mut region = m.extent(Scope::Contents, 64);
    if region.is_empty() {
        region = m.extent(Scope::Problem, 64);
    }
    let region = visible_extent(&m, &vis, &region, 48).unwrap_or(region);
    let gpu = Arc::new(Gpu::new().unwrap());
    println!("{} ({:?}), {w}×{h}", gpu.info.name, gpu.info.backend);
    let gm = Arc::new(GpuModel::upload(&gpu, &m).unwrap());
    gm.set_visibility(&gpu, &m, &vis).unwrap();
    let backend = GpuBackend::new(gpu.clone(), gm, m.clone());
    let m2 = m.clone();
    let looks = Arc::new(Looks::new(&m, &[]).unwrap());
    let mut view = View3d::new(
        gpu,
        m,
        looks,
        Box::new(backend),
        &vis,
        0.033,
        ColourBy::Cell,
    );
    let cam = Orbit::fit(&region, 35.0, 25.0).camera(w, h, None);
    view.render_now(cam.clone());
    let fx = ["outline", "ambient_occlusion"];
    for (by, shading) in [
        (ColourBy::Cell, "flat"),
        (ColourBy::Realistic, "physically based"),
    ] {
        view.set_colouring(by);
        println!("{shading} shading ({} colours)", by.name());
        compose_times(&mut view, &fx);
    }

    // Marks: 8 surfaces of the first cell with as many, and a shell
    // covering the image (its worst case).
    let records: Vec<(u32, Vec<f64>)> = (0..m2.cells().len() as u32)
        .map(|c| m2.cell_surfaces(c))
        .find(|s| s.len() >= 8)
        .unwrap_or_default()
        .iter()
        .take(8)
        .filter_map(|&(s, _)| m2.shape_record(s, None))
        .collect();
    let (sw, sh) = Shell::size(&cam);
    let mut words = vec![0.0f32; 2 * sw * sh];
    for k in 0..sw * sh {
        words[2 * k] = 1.0;
        words[2 * k + 1] = 0.7;
    }
    let shell = Shell {
        camera: cam,
        width: sw,
        height: sh,
        words,
    };
    view.set_colouring(ColourBy::Cell);
    for (name, s, sh) in [
        ("surfaces", true, false),
        ("shell", false, true),
        ("surfaces + shell", true, true),
    ] {
        let surfaces = if s {
            surface_words(&records, None)
        } else {
            Vec::new()
        };
        view.set_edit_marks(surfaces, sh.then(|| Arc::new(shell.clone())));
        view.set_effect("outline", true);
        view.set_effect("ambient_occlusion", false);
        view.compose_blocking();
        let mut t: Vec<f64> = (0..20).map(|_| view.compose_blocking()).collect();
        t.sort_by(f64::total_cmp);
        println!(
            "  outline + marks: {name:<20} {:.2} ms (median of 20; {} surfaces)",
            1e3 * t[10],
            records.len()
        );
    }
}

fn compose_times(view: &mut View3d, fx: &[&str; 2]) {
    for mask in 0..4 {
        let on: Vec<&str> = (0..2)
            .filter(|k| mask >> k & 1 == 1)
            .map(|k| fx[k])
            .collect();
        for (k, f) in fx.iter().enumerate() {
            view.set_effect(f, mask >> k & 1 == 1);
        }
        view.compose_blocking();
        let mut t: Vec<f64> = (0..20).map(|_| view.compose_blocking()).collect();
        t.sort_by(f64::total_cmp);
        println!(
            "  {:<38} {:.2} ms (median of 20)",
            if on.is_empty() {
                "colour only".to_string()
            } else {
                on.join(" + ")
            },
            1e3 * t[10]
        );
    }
}
