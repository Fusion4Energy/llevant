//! Drafts patched into an uploaded model ([`GpuModel::patch`]): its first
//! hits agree with the CPU's on a fresh compile of the edited input, ray by
//! ray (as `vs_cpu` holds an upload to), step after step; the revert patch
//! gives back the model uploaded; the shader stays the same. Skipped
//! without a GPU adapter.

use std::sync::Arc;

use llevant::draft::Draft;
use llevant::{GridMode, Line, Model, Options, Visibility};
use llevant_gpu::{check, Gpu, GpuModel};

fn zoo(name: &str) -> String {
    let dir = format!(
        "{}/../../tools/mcnp-oracle/models",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(format!("{dir}/{name}")).unwrap()
}

/// `gm` (with `vis`, its skip from `skip_model` widened by `draft`)
/// against the CPU on `model`.
fn agree(
    gpu: &Gpu,
    gm: &GpuModel,
    model: &Model,
    skip_model: &Model,
    draft: Option<&Draft>,
    what: &str,
) {
    for (name, vis) in check::visibilities(model)
        .into_iter()
        .filter(|(n, _)| *n != "level 1")
    {
        let skip = skip_model.skip(&vis).unwrap();
        let skip = match draft {
            Some(d) => d.widen_skip(&skip),
            None => skip,
        };
        gm.set_visibility_with(gpu, skip_model, &vis, &skip)
            .unwrap();
        for (vname, cam) in check::views(model, 40) {
            let rays: Vec<Line> = cam.rays().into_iter().flatten().collect();
            let r = check::compare(model, gm, gpu, &rays, &vis);
            let w = format!("{what}, {name}, {vname}: {}", r.text);
            assert_eq!(r.out_of_steps, 0, "{w}");
            assert!(r.same as f64 >= 0.995 * r.rays as f64, "{w}");
            assert!(r.normal_diff as f64 <= 0.005 * r.same as f64, "{w}");
        }
    }
    let _ = Visibility::all;
}

fn nudge(c: &[f64], k: usize, by: f64) -> Vec<f64> {
    let mut v = c.to_vec();
    let i = k % v.len();
    v[i] += by * (0.1 + 0.2 * v[i].abs().min(5.0));
    v
}

#[test]
fn patched_drafts_render_as_fresh_compiles() {
    let Some(gpu) = llevant_gpu::check::shared_gpu() else {
        return;
    };
    let mut kept = 0;
    for name in ["universes.i", "surfaces.i"] {
        let text = zoo(name);
        let opts = Options::default().with_grid(GridMode::Always);
        let base = Arc::new(Model::compile(text.as_bytes(), &opts, None).unwrap());
        let gm = GpuModel::upload(&gpu, &base).unwrap();
        let shader = gm.shader_id();
        let tree = migjorn::Model::parse(&text);
        let ids: Vec<i64> = tree.surfaces().filter_map(|s| s.id()).collect();
        // Every few surfaces, to keep the test short.
        for (n, &id) in ids.iter().enumerate().filter(|(n, _)| n % 5 == 0) {
            let Ok(mut draft) = Draft::begin(base.clone(), &tree, id) else {
                continue;
            };
            kept += draft.units_kept_on_gpu().len();
            let start = draft.coeffs().to_vec();
            let mut edited = migjorn::Model::parse(&text);
            for (step, by) in [0.6, -0.9].into_iter().enumerate() {
                let c = nudge(&start, n + step, by);
                let Ok(p) = draft.set(&c) else {
                    continue;
                };
                gm.patch(&gpu, &p).unwrap();
                edited.surface_mut(id).unwrap().set_coeffs(&c).unwrap();
                let fresh = Model::compile(edited.to_source().as_bytes(), &opts, None).unwrap();
                agree(
                    &gpu,
                    &gm,
                    &fresh,
                    &base,
                    Some(&draft),
                    &format!("{name} surface {id} step {step}"),
                );
            }
            gm.patch(&gpu, &draft.revert_patch()).unwrap();
            agree(
                &gpu,
                &gm,
                &base,
                &base,
                None,
                &format!("{name} surface {id} reverted"),
            );
        }
        assert_eq!(gm.shader_id(), shader, "{name}: patches keep the shader");
    }
    assert!(kept > 0, "no draft kept an octree on the GPU");
}
