//! Opening a model in the background: compile it (or take one of its
//! universes on its own), measure what to frame, upload it to a GPU device
//! of its own: then ([`How::gpu_now`]), or later, while the model is shown
//! ([`upload`], which the document runs when the 3D view wants the GPU).
//!
//! The GPU copy takes seconds whatever the model's size (the driver
//! compiles the renderer's shader: ~3.5 s on lavapipe, more on some laptop
//! drivers), so a model opened from the start screen does not wait for it:
//! its 3D view traces on the CPU until the GPU's copy lands.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use llevant::{Aabb, Changed, Cut, GridMode, Model, Options, Progress, Scope, Visibility};
use llevant_gpu::{Gpu, GpuModel};
use llevant_scene::index::ModelIndex;
use llevant_scene::labels::Labels;
use llevant_scene::materials::Looks;
use llevant_scene::store::{Staged, Store};

use llevant_scene::source::Source;

use super::input_edits::Change;
use crate::document::Loaded;
use crate::mesh::InputMeshes;
use crate::scene::Scene;
use crate::view3d::backend::Shown;
use crate::view3d::Prepared3d;

/// Log (debug) when a step of loading ends (seconds since `start`), to
/// line up with memory sampled from outside.
pub fn log_step(step: &str, start: Instant) {
    log::debug!(
        "load: {step} done at {:.2} s",
        start.elapsed().as_secs_f64()
    );
}

/// Universes rebuilt holding at most this share of the cells are completed
/// before a reload is shown (one GPU upload, a few seconds); more, and the
/// staged model is shown first.
const COMPLETE_FIRST: f64 = 0.25;

/// Models with fewer cells are completed before they are shown: their
/// octrees take less than a second or two, less than a second upload.
const COMPLETE_BELOW_CELLS: usize = 50_000;

/// How a load goes (the viewer's [`DevOptions`](super::devtools::DevOptions),
/// and whether the GPU copy is made with the model).
#[derive(Clone, Copy, Debug, Default)]
pub struct How {
    /// Fail the GPU upload, as when the model does not fit.
    pub fail_upload: bool,
    /// Show the model staged however small it is.
    pub stage_always: bool,
    /// Upload the model to the GPU before it is shown: when a model is
    /// shown meanwhile (a reload, an edit, a universe opened, the octrees
    /// built) and the 3D view traces on the GPU. Otherwise
    /// [`Loaded::gpu_later`] says how to upload it later.
    pub gpu_now: bool,
}

/// The model on a device of its own.
pub type GpuPair = (Arc<Gpu>, Arc<GpuModel>);

/// What uploading a model later ([`upload`]) needs.
#[derive(Clone)]
pub struct GpuLater {
    pub adapter: wgpu::Adapter,
    pub fail_upload: bool,
}

/// One GPU device made and a model uploaded at a time: devices made at
/// once on several threads (a model's copy made in the background while a
/// reload makes another) can hang the Vulkan driver.
static UPLOADING: Mutex<()> = Mutex::new(());

/// `model` on a device of its own made from `adapter`, the renderer
/// compiled and run once on a tiny image of `region` (so that a driver that
/// compiles on first use does it here, not in the first refinement), each
/// step named in `progress`. `fail_upload`: fail as when the model does
/// not fit (for tests).
pub fn upload(
    model: &Model,
    adapter: &wgpu::Adapter,
    region: &Aabb,
    fail_upload: bool,
    progress: Option<&Progress>,
) -> Result<GpuPair, String> {
    let stage = |s| {
        if let Some(p) = progress {
            p.set_stage(s);
        }
    };
    let start = Instant::now();
    stage("waiting for the GPU");
    let _one = UPLOADING.lock().unwrap_or_else(|e| e.into_inner());
    stage("opening the GPU");
    let gpu = Gpu::from_adapter(adapter, crate::view3d::backend::TIMESTAMPS)
        .map_err(|e| e.to_string())?;
    if fail_upload {
        return Err("not enough GPU memory for the model (DevOptions::fail_gpu_upload)".into());
    }
    stage("compiling the GPU renderer");
    let mut gm = GpuModel::upload(&gpu, model).map_err(|e| e.to_string())?;
    crate::view3d::backend::beside_window(&mut gm);
    log_step("upload", start);
    stage("a first GPU image");
    gm.render(&gpu, &llevant::Camera::fit(region, [1.0, 1.0, -0.5], 8, 8))
        .map_err(|e| e.to_string())?;
    log_step("shader", start);
    Ok((Arc::new(gpu), Arc::new(gm)))
}

/// What a load in the background made.
pub struct Load {
    /// The documents: the whole model, then each universe opened in turn.
    pub chain: Vec<Loaded>,
    /// The whole model, staged: its octrees are still to build
    /// ([`complete`]); it answers everything meanwhile, more slowly.
    pub pending: Option<Staged>,
    /// What changed from the model it was recompiled from, if it was.
    pub changed: Option<Changed>,
    /// For each model of the chain: the boxes (in its frame) of every
    /// instance of the cells that changed, in the model before (where a
    /// cell removed or shrunk can leave a gap) and in the new one: where the
    /// edit can have opened or closed something.
    pub changed_boxes: Vec<Option<Arc<Vec<llevant::Aabb>>>>,
    /// The edit of the input this load made, if it was one.
    pub edit: Option<InputEdit>,
    /// The input's FMESH cards and numbers read again (an edit of a `TRn`
    /// card, which they can use).
    pub meshes: Option<Arc<InputMeshes>>,
    /// The cells' labels read again (an edit of a cell's number or `$`
    /// comments).
    pub labels: Option<Arc<Labels>>,
}

/// An edit of the input made in the viewer (in its [`Source`]), compiled.
#[derive(Clone)]
pub enum InputEdit {
    /// Cards changed.
    Made(Change),
    /// The last edit taken back.
    TakenBack(Change),
}

impl InputEdit {
    pub fn change(&self) -> &Change {
        match self {
            InputEdit::Made(c) | InputEdit::TakenBack(c) => c,
        }
    }

    /// Take it back in `source` (its compile failed).
    pub fn revert(&self, source: &Source) {
        let r = match self {
            InputEdit::Made(c) => source.restore_all(&c.cards),
            InputEdit::TakenBack(c) => source.redo_all(&c.cards),
        };
        if let Err(e) = r {
            log::warn!("could not take an edit back: {e}");
        }
    }
}

/// The documents an edit is made in: the models shown, the whole model
/// first, then each universe opened on its own within the one before
/// (`None` for the whole model), each with the scene it shows.
pub struct Opened {
    pub models: Vec<(Option<i64>, Arc<Model>, Scene)>,
}

/// The input as `source` has it after `edit`, compiled against the whole
/// model shown (only what changed is built again; nothing is parsed),
/// completed; then each universe of `opened` cut out of the one before it
/// again ([`Model::universe`]), each prepared at its scene. A universe the
/// edit took away (renumbered, emptied) ends the chain there.
#[allow(clippy::too_many_arguments)]
pub fn load_edit(
    edit: InputEdit,
    source: &Source,
    labels: Option<Arc<Labels>>,
    opened: Opened,
    adapter: &wgpu::Adapter,
    window: &Arc<Gpu>,
    progress: &llevant::Progress,
    how: How,
) -> Result<Load, String> {
    let start = Instant::now();
    let mut levels = opened.models.into_iter();
    let (_, base, scene) = levels.next().ok_or("no model shown")?;
    let (model, changed) = {
        let tree = source.tree();
        let (m, c) = base
            .recompile_parsed(&tree, &Options::default(), Some(progress))
            .map_err(|e| e.to_string())?;
        (m, c)
    };
    let model = Arc::new(
        model
            .into_complete(Some(progress))
            .map_err(|e| e.to_string())?,
    );
    log_step("edit", start);
    let meshes = edit
        .change()
        .touches_transforms()
        .then(|| Arc::new(InputMeshes::of(&source.tree())));
    // The labels read again only when the edit can have changed them.
    let new_labels = edit
        .change()
        .touches_labels()
        .then(|| Arc::new(Labels::of(&source.tree())));
    let labels = new_labels.clone().or(labels);
    let mut boxes = vec![changed_boxes(&base, &model, &changed).map(Arc::new)];
    drop(base);
    // The universes opened, cut again from the new models.
    let mut models = vec![(None, model, scene)];
    for (u, old, scene) in levels {
        let Some(u) = u else { break };
        let within = &models[models.len() - 1].1;
        let Ok(sub) = within.universe(u, Cut::Own, GridMode::Auto) else {
            break;
        };
        boxes.push(changed_boxes(&old, &sub, &changed).map(Arc::new));
        models.push((Some(u), Arc::new(sub), scene));
    }
    log_step("universes opened", start);
    let chain = models
        .into_iter()
        .map(|(u, m, s)| {
            prepare(
                m,
                u,
                labels.as_ref(),
                adapter,
                window,
                &s,
                start,
                how,
                Some(progress),
            )
        })
        .collect::<Result<_, _>>()?;
    Ok(Load {
        chain,
        pending: None,
        changed: Some(changed),
        changed_boxes: boxes,
        edit: Some(edit),
        meshes,
        labels: new_labels,
    })
}

/// [`Loaded::gpu_model`] until the model is uploaded.
pub const GPU_NOT_YET: &str = "the GPU renderer is not ready yet";

/// At most this many cells changed for [`Load::changed_boxes`].
const CHANGED_BOXES_MAX: usize = 20_000;

/// The boxes of [`Load::changed_boxes`]: the changed and removed cells'
/// instances in `old`, the changed ones' in `new`, within the problem.
fn changed_boxes(old: &Model, new: &Model, c: &Changed) -> Option<Vec<llevant::Aabb>> {
    let n = c.cells_changed.len() + c.cells_removed.len();
    if n == 0 || n > CHANGED_BOXES_MAX {
        return None;
    }
    let at = |m: &Model, ids: &mut dyn Iterator<Item = &i64>| -> Vec<u32> {
        ids.filter_map(|&id| m.cell_index(id)).collect()
    };
    let before = at(old, &mut c.cells_changed.iter().chain(&c.cells_removed));
    let after = at(new, &mut c.cells_changed.iter());
    let (a, b) = rayon::join(
        || old.instance_boxes(&before, old.bounds(Scope::Problem), 0.0),
        || new.instance_boxes(&after, new.bounds(Scope::Problem), 0.0),
    );
    let boxes: Vec<llevant::Aabb> = a.into_iter().chain(b).collect();
    (!boxes.is_empty()).then_some(boxes)
}

/// Load `path`, then open each of `universes` in turn within the one before
/// it: the chain from the whole model to the last universe, the last
/// framing what `scene` shows (the others their default scene). With
/// `base` (the model shown, of an earlier version of `path`), only what
/// changed is built again. The whole model alone is returned staged when
/// building its octrees would take long.
#[allow(clippy::too_many_arguments)]
pub fn load(
    path: &Path,
    universes: &[i64],
    adapter: &wgpu::Adapter,
    window: &Arc<Gpu>,
    scene: &Scene,
    progress: &llevant::Progress,
    how: How,
    base: Option<Arc<Model>>,
) -> Result<Load, String> {
    let start = Instant::now();
    let store = Store::from_env();
    let staged = store
        .load_staged(path, &Options::default(), Some(progress), base.as_deref())
        .map_err(|e| e.to_string())?;
    let changed = staged.changed.clone();
    let changed_boxes = match (&base, &changed) {
        (Some(old), Some(c)) => changed_boxes(old, &staged.model, c).map(Arc::new),
        _ => None,
    };
    drop(base);
    let small = changed.as_ref().is_some_and(|c| {
        let m = &staged.model;
        let rebuilt: usize = m
            .units()
            .iter()
            .filter(|u| c.universes_rebuilt.contains(&u.universe))
            .map(|u| u.cells.len())
            .sum();
        rebuilt as f64 <= COMPLETE_FIRST * m.cells().len() as f64
    });
    let quick = !how.stage_always && staged.model.cells().len() < COMPLETE_BELOW_CELLS;
    let (model, pending) = if staged.is_complete() || !universes.is_empty() || small || quick {
        let m = store
            .complete(staged, Some(progress))
            .map_err(|e| e.to_string())?;
        (m, None)
    } else {
        (staged.model.clone(), Some(staged))
    };
    log_step("model", start);
    let mut models = vec![(None, model)];
    for &u in universes {
        let last = &models[models.len() - 1].1;
        let sub = last
            .universe(u, Cut::Own, GridMode::Auto)
            .map_err(|e| e.to_string())?;
        models.push((Some(u), Arc::new(sub)));
    }
    let n = models.len();
    let default = Scene::default();
    let chain = models
        .into_iter()
        .enumerate()
        .map(|(k, (u, m))| {
            let s = if k + 1 == n { scene } else { &default };
            prepare(m, u, None, adapter, window, s, start, how, Some(progress))
        })
        .collect::<Result<_, _>>()?;
    Ok(Load {
        chain,
        pending,
        changed,
        changed_boxes: vec![changed_boxes],
        edit: None,
        meshes: None,
        labels: None,
    })
}

/// The staged model of a load completed (its octrees built, then cached),
/// ready to show in place of it at `scene`.
pub fn complete(
    staged: Staged,
    labels: Option<Arc<Labels>>,
    adapter: &wgpu::Adapter,
    window: &Arc<Gpu>,
    scene: &Scene,
    progress: &llevant::Progress,
    how: How,
) -> Result<Loaded, String> {
    let start = Instant::now();
    // Built while the staged model is shown: below the views' work.
    let model = crate::tasks::background_pool()
        .install(|| Store::from_env().complete(staged, Some(progress)))
        .map_err(|e| e.to_string())?;
    log_step("octrees", start);
    prepare(
        model,
        None,
        labels.as_ref(),
        adapter,
        window,
        scene,
        start,
        how,
        Some(progress),
    )
}

/// Universe `u` of `model` on its own, framing what `scene` shows.
pub fn open_universe(
    model: &Model,
    u: i64,
    labels: Option<Arc<Labels>>,
    adapter: &wgpu::Adapter,
    window: &Arc<Gpu>,
    scene: &Scene,
    how: How,
) -> Result<Loaded, String> {
    let start = Instant::now();
    let sub = model
        .universe(u, Cut::Own, GridMode::Auto)
        .map_err(|e| e.to_string())?;
    let scene = known_to(scene.clone(), &sub);
    prepare(
        Arc::new(sub),
        Some(u),
        labels.as_ref(),
        adapter,
        window,
        &scene,
        start,
        how,
        None,
    )
}

/// `s` hiding only the cells and universes `m` has.
pub fn known_to(mut s: Scene, m: &Model) -> Scene {
    s.hiding.cells.retain(|&c| m.cell_index(c).is_some());
    s.hiding
        .universes
        .retain(|&u| m.units().iter().any(|x| x.universe == u));
    s
}

/// Frame what `scene` shows of `model`, and upload it to a device of its
/// own if `how` says so now ([`How::gpu_now`]), else leave how to upload
/// it later in [`Loaded::gpu_later`]. Each step named in `progress`.
#[allow(clippy::too_many_arguments)]
fn prepare(
    model: Arc<Model>,
    universe: Option<i64>,
    labels: Option<&Arc<Labels>>,
    adapter: &wgpu::Adapter,
    window: &Arc<Gpu>,
    scene: &Scene,
    start: Instant,
    how: How,
    progress: Option<&Progress>,
) -> Result<Loaded, String> {
    let stage = |s| {
        if let Some(p) = progress {
            p.set_stage(s);
        }
    };
    stage("indexing the cells");
    let index = Arc::new(ModelIndex::new(&model));
    if let Some(l) = labels {
        index.set_labels(&model, l.clone());
    }
    log_step("index", start);
    stage("framing the view");
    let mut region = model.extent(Scope::Contents, 64);
    if region.is_empty() {
        region = model.extent(Scope::Problem, 64);
    }
    if region.is_empty() {
        region = model.bounds(Scope::Problem);
    }
    // Frame what the 3D view shows, not a world cell around it.
    // What is wrong with the scene is said once the model is open.
    let mut problems = Vec::new();
    let vis = scene.visibility(&model, &index).unwrap_or_else(|e| {
        problems.push(format!("view state: {e}"));
        Visibility::solid(&model)
    });
    let looks = Arc::new(scene.looks(&model).unwrap_or_else(|e| {
        problems.push(format!("view state, materials: {e}"));
        Looks::new(&model, &[]).expect("no specs")
    }));
    if let Some(r) = crate::camera::visible_extent(&model, &vis, &region, 48) {
        region = r;
    }
    log_step("framing", start);
    let (gpu_model, gpu_later) = if how.gpu_now {
        (
            upload(&model, adapter, &region, how.fail_upload, progress),
            None,
        )
    } else {
        let later = GpuLater {
            adapter: adapter.clone(),
            fail_upload: how.fail_upload,
        };
        (Err(GPU_NOT_YET.to_string()), Some(later))
    };
    stage("preparing the views");
    let seen = Arc::new(crate::seen::Seen::new(&model, &index, scene, &vis));
    // The visibility made ready for the backends here, not on the UI
    // thread.
    let shown = Shown::new(
        &model,
        gpu_model.as_ref().ok().map(|(g, gm)| (&**g, &**gm)),
        vis,
    )
    .map_err(|e| e.to_string())?;
    log_step("visibility", start);
    // The 3D view's pipelines (the driver compiles them) and palette.
    let view3d = Prepared3d::new(window.clone(), &model, &looks, scene.colour_by.into());
    log_step("3D view", start);
    Ok(Loaded {
        model,
        index,
        universe,
        region,
        gpu_model,
        gpu_later,
        shown,
        looks,
        seen,
        problems,
        view3d,
        seconds: start.elapsed().as_secs_f64(),
    })
}
