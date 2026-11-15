//! A model uploaded from its lean export, its largest arrays streamed from
//! the model (`GpuModel::upload`), renders exactly as one uploaded from the
//! full export (`GpuModel::new`). Skipped when no GPU adapter is available.

use llevant::{Camera, Model, Options, Scope, Visibility};
use llevant_gpu::GpuModel;

/// The shader depends on the model only through the depth of its programs
/// and BVHs: a staged model, the same completed, and one edited share it
/// (the driver compiles it once).
#[test]
fn models_share_the_shader() {
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    let dir = format!(
        "{}/../../tools/mcnp-oracle/models",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(format!("{dir}/universes.i")).unwrap();
    let opts = Options::default().with_grid(llevant::GridMode::Always);
    let staged = Model::compile_staged(text.as_bytes(), &opts, None).unwrap();
    let complete = staged.complete(None).unwrap();
    let mut tree = migjorn::Model::parse(&text);
    let id = tree.surfaces().filter_map(|s| s.id()).next().unwrap();
    let c = tree.surface(id).unwrap().coeffs().unwrap();
    let k = c.len() - 1;
    tree.surface_mut(id)
        .unwrap()
        .set_coeff(k, c[k] + 0.5)
        .unwrap();
    let (edited, _) = complete.recompile_parsed(&tree, &opts, None).unwrap();
    let edited = edited.into_complete(None).unwrap();
    let id = |m: &Model| GpuModel::upload(&gpu, m).unwrap().shader_id();
    let first = id(&staged);
    assert_eq!(id(&complete), first, "staged and complete");
    assert_eq!(id(&edited), first, "edited");
}

#[test]
fn streamed_upload_renders_as_the_full_export() {
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    for name in ["universes.i", "periodic.i", "surfaces.i"] {
        let path = format!(
            "{}/../../tools/mcnp-oracle/models/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        // Octrees everywhere, so that their arrays are streamed too.
        let model = Model::load(
            &path,
            &Options::default().with_grid(llevant::GridMode::Always),
            None,
        )
        .unwrap();
        let full = GpuModel::new(&gpu, &model.export()).unwrap();
        let lean = GpuModel::upload(&gpu, &model).unwrap();
        assert_eq!(full.bytes, lean.bytes, "{name}");
        let vis = Visibility::solid(&model);
        full.set_visibility(&gpu, &model, &vis).unwrap();
        lean.set_visibility(&gpu, &model, &vis).unwrap();
        let region = model.extent(Scope::Contents, 64);
        let cam = Camera::fit(&region, [1.0, 0.7, -0.4], 64, 48);
        let a = full.render(&gpu, &cam).unwrap();
        let b = lean.render(&gpu, &cam).unwrap();
        assert!(a.hits.iter().any(|h| h.is_hit()), "{name}: nothing seen");
        for (i, (x, y)) in a.hits.iter().zip(&b.hits).enumerate() {
            assert_eq!(x.to_words(), y.to_words(), "{name}: ray {i}");
        }
    }
}
