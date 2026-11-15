//! The 3D view: rays traced by a [`TraceBackend`] in batches the
//! [`Scheduler`] sizes to the frame budget, gathered into a G-buffer
//! (`shaders/resolve.wgsl`) and turned into an image by the [`PassChain`],
//! all on the GPU; egui shows the image as a texture.

pub mod backend;
pub mod budget;
pub mod edits;
pub mod field;
pub mod passes;
pub mod reproject;

use std::sync::Arc;

use llevant::{Camera, Model, Visibility};
use llevant_gpu::{Gpu, HitBuffer, GHOST_WORDS, HIT_WORDS, NONE};
use llevant_render::colours::{ColourBy, Paint};
use llevant_render::field::Overlay;
use llevant_scene::index::CellSet;
use llevant_scene::materials::Looks;
use wgpu::util::DeviceExt;

use backend::{upload_tile, Finished, TraceBackend};
use budget::{Batch, Level, Scheduler, BLOCK};
use passes::{FrameUniform, PassChain, Targets};

/// Numbers for the status bar.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub backend: &'static str,
    /// Scale of the last low-resolution image.
    pub scale: f64,
    /// Fraction of the full-resolution image done.
    pub refined: f64,
    /// Measured time of preview images and of full-resolution tiles.
    pub preview: Option<budget::Cost>,
    pub refine: Option<budget::Cost>,
    pub in_flight: usize,
    /// Low images shown since the view was made.
    pub previews: u64,
}

/// Words per pixel of the G-buffer: its hit, then (after all the hits) its
/// ghost record.
const G_WORDS: usize = HIT_WORDS + GHOST_WORDS;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ResolveUniform {
    size: [u32; 4],
    blocks: [u32; 4],
    /// The camera traced (both images are of it).
    cam: reproject::Cam,
}

pub struct View3d {
    gpu: Arc<Gpu>,
    model: Arc<Model>,
    /// What the model's materials look like (realistic colours).
    looks: Arc<Looks>,
    backend: Box<dyn TraceBackend>,
    scheduler: Scheduler,
    width: u32,
    height: u32,
    /// The camera asked for, at full size.
    desired: Option<Camera>,
    /// Generation being traced and its camera (full size).
    trace_generation: u64,
    trace_camera: Option<Camera>,
    /// Low images: `low[shown]` is shown, the other one is being traced.
    low: [HitBuffer; 2],
    shown: usize,
    /// Camera of the image shown (full size), `None` before the first.
    shown_camera: Option<Camera>,
    full: HitBuffer,
    /// Per 8×8 block of the full image: whether its full-resolution hits
    /// are in.
    mask: Vec<u32>,
    /// Blocks of `mask` set.
    landed: usize,
    mask_buffer: wgpu::Buffer,
    gbuf: wgpu::Buffer,
    /// Whether the visibility has ghosts: their records are resolved and
    /// drawn (`ghost.wgsl`).
    ghosts: bool,
    resolve_layout: wgpu::BindGroupLayout,
    resolve: wgpu::ComputePipeline,
    chain: PassChain,
    targets: Targets,
    palette: wgpu::Buffer,
    /// Behind the model with flat colours (realistic ones keep the sky of
    /// `llevant render`).
    background: [u8; 3],
    by: ColourBy,
    /// Realistic colours, lit physically.
    pbr: bool,
    /// What is shown (the CPU backend takes it over if the GPU is lost).
    vis: Visibility,
    /// Numbers drawn over the cells ([`field::FieldPass`]).
    field: Option<Overlay>,
    /// What the field pass's buffer holds, and why it holds nothing if it
    /// is too large for the device.
    field_data: Option<(field::DataKey, Option<String>)>,
    notice: Option<String>,
    /// The cell under the pointer, outlined (or `NONE`).
    hovered: u32,
    /// The selection, a bit per cell (`highlight.wgsl`).
    marks: wgpu::Buffer,
    /// The surfaces marked ([`edits::surface_words`]), empty for none.
    edit_surfaces: Vec<f32>,
    /// The shell of a cell as edited.
    shell: Option<Arc<edits::Shell>>,
    /// The image must be composed again.
    dirty: bool,
    /// Index of `targets` shown.
    out: usize,
    textures: Option<[egui::TextureId; 2]>,
    /// Low images shown so far.
    previews: u64,
    /// Moves the hits shown to the camera wanted, while it moves.
    reproject: reproject::Reprojector,
    /// Whether it does ([`View3d::set_reprojection`]).
    reprojecting: bool,
    /// The camera the image was last composed for.
    composed: Option<Camera>,
    /// Reproject even to the camera traced (tests).
    always_reproject: bool,
}

/// Whether `a` and `b` see from the same place the same way at the same
/// size (whatever they cut away).
fn same_pose(a: &Camera, b: &Camera) -> bool {
    (a.eye, a.target, a.up, a.fov_deg, a.ortho, a.width, a.height)
        == (b.eye, b.target, b.up, b.fov_deg, b.ortho, b.width, b.height)
}

/// `c` at the size of `like`.
fn with_size(c: &Camera, like: &Camera) -> Camera {
    let mut c = c.clone();
    c.width = like.width;
    c.height = like.height;
    c
}

/// What a [`View3d`] needs that takes a while to make and needs no UI:
/// its shader pipelines (compiled by the driver) and the palette of every
/// cell. Made on any thread ([`View3d::with`] takes it).
pub struct Prepared3d {
    gpu: Arc<Gpu>,
    resolve_layout: wgpu::BindGroupLayout,
    resolve: wgpu::ComputePipeline,
    chain: PassChain,
    by: ColourBy,
    palette: Vec<u32>,
}

impl Prepared3d {
    pub fn new(gpu: Arc<Gpu>, model: &Model, looks: &Looks, by: ColourBy) -> Prepared3d {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("resolve.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/resolve.wgsl").into()),
        });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let ro = wgpu::BufferBindingType::Storage { read_only: true };
        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("resolve"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform),
                entry(1, ro),
                entry(2, ro),
                entry(3, ro),
                entry(4, wgpu::BufferBindingType::Storage { read_only: false }),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("resolve"),
            bind_group_layouts: &[Some(&resolve_layout)],
            immediate_size: 0,
        });
        let resolve = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("resolve"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let chain = PassChain::new(&gpu);
        Prepared3d {
            palette: palette_words(model, looks, by),
            gpu,
            resolve_layout,
            resolve,
            chain,
            by,
        }
    }
}

/// The palette of `model`'s cells with `looks` coloured by `by`: two words
/// per cell, the colour (sRGB) and roughness, metalness and clarity.
pub fn palette_words(model: &Model, looks: &Looks, by: ColourBy) -> Vec<u32> {
    let paint = Paint::new(model, looks, by);
    (0..model.cells().len() as u32)
        .flat_map(|c| {
            let l = paint.cell_look(c);
            let [r, g, b] = l.srgb;
            let q = |x: f32| (x * 255.0).round() as u8;
            [
                u32::from_le_bytes([r, g, b, 255]),
                u32::from_le_bytes([q(l.roughness), q(l.metalness), q(l.clarity), 0]),
            ]
        })
        .collect()
}

fn storage(gpu: &Gpu, label: &str, words: usize) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: 4 * words.max(1) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl View3d {
    pub fn new(
        gpu: Arc<Gpu>,
        model: Arc<Model>,
        looks: Arc<Looks>,
        backend: Box<dyn TraceBackend>,
        vis: &Visibility,
        budget_seconds: f64,
        by: ColourBy,
    ) -> View3d {
        let prepared = Prepared3d::new(gpu, &model, &looks, by);
        View3d::with(prepared, model, looks, backend, vis, budget_seconds)
    }

    /// [`View3d::new`] with its pipelines and palette made already
    /// ([`Prepared3d::new`], off the UI thread).
    pub fn with(
        prepared: Prepared3d,
        model: Arc<Model>,
        looks: Arc<Looks>,
        backend: Box<dyn TraceBackend>,
        vis: &Visibility,
        budget_seconds: f64,
    ) -> View3d {
        let Prepared3d {
            gpu,
            resolve_layout,
            resolve,
            chain,
            by,
            palette,
        } = prepared;
        let reproject = reproject::Reprojector::new(&gpu);
        let mut v = View3d {
            low: [
                HitBuffer::with_ghosts(&gpu, 1, 1),
                HitBuffer::with_ghosts(&gpu, 1, 1),
            ],
            full: HitBuffer::with_ghosts(&gpu, 1, 1),
            mask: vec![0],
            landed: 0,
            mask_buffer: storage(&gpu, "mask", 1),
            gbuf: storage(&gpu, "gbuf", G_WORDS),
            ghosts: false,
            targets: Targets::new(&gpu, 1, 1),
            palette: storage(&gpu, "palette", 1),
            marks: storage(&gpu, "selection", 1),
            background: llevant_render::colours::BACKGROUND,
            by,
            pbr: false,
            vis: vis.clone(),
            field: None,
            field_data: None,
            notice: None,
            resolve_layout,
            resolve,
            chain,
            scheduler: Scheduler::new(budget_seconds),
            backend,
            gpu,
            model,
            looks: looks.clone(),
            width: 1,
            height: 1,
            desired: None,
            trace_generation: 0,
            trace_camera: None,
            shown: 0,
            shown_camera: None,
            hovered: NONE,
            edit_surfaces: Vec::new(),
            shell: None,
            dirty: false,
            out: 0,
            textures: None,
            previews: 0,
            reproject,
            reprojecting: true,
            composed: None,
            always_reproject: false,
        };
        v.set_palette(looks, by, &palette);
        v
    }

    pub fn stats(&self) -> Stats {
        Stats {
            backend: self.backend.name(),
            scale: self.scheduler.scale(),
            refined: self.landed as f64 / self.mask.len() as f64,
            preview: self.scheduler.cost(Level::Low {
                width: 1,
                height: 1,
            }),
            refine: self.scheduler.cost(Level::Full),
            in_flight: self.backend.in_flight(),
            previews: self.previews,
        }
    }

    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    /// A message for the user since the last call (the GPU was lost).
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    /// Trace with another backend from now on.
    pub fn set_backend(&mut self, backend: Box<dyn TraceBackend>) {
        self.backend = backend;
        self.scheduler.reset_cost();
        self.restart();
    }

    pub fn set_budget(&mut self, seconds: f64) {
        self.scheduler.budget = seconds;
    }

    /// Colours of the cells, and their lighting: physically based for
    /// realistic colours, flat otherwise (no new trace).
    pub fn set_colouring(&mut self, by: ColourBy) {
        let words = palette_words(&self.model, &self.looks, by);
        self.set_palette(self.looks.clone(), by, &words);
    }

    /// [`View3d::set_colouring`] with `looks`, its words made already
    /// ([`palette_words`] of `looks` and `by`, off the UI thread).
    pub fn set_palette(&mut self, looks: Arc<Looks>, by: ColourBy, words: &[u32]) {
        self.looks = looks;
        self.palette = self
            .gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("palette"),
                contents: bytemuck::cast_slice(if words.is_empty() { &[0] } else { words }),
                usage: wgpu::BufferUsages::STORAGE,
            });
        self.by = by;
        self.pbr = by == ColourBy::Realistic;
        self.chain
            .set_params("colour", passes::Colour::params_for(self.pbr));
        self.dirty = true;
    }

    /// The colour behind the model with flat colours (by default that of
    /// `llevant render`, white); no new trace.
    /// Whether the hits shown follow the camera while it moves (on by
    /// default); off, the last image stays where it was traced until the
    /// next one lands.
    pub fn set_reprojection(&mut self, on: bool) {
        if on != self.reprojecting {
            self.reprojecting = on;
            self.dirty = true;
        }
    }

    pub fn set_background(&mut self, rgb: [u8; 3]) {
        if self.background != rgb {
            self.background = rgb;
            self.dirty = true;
        }
    }

    /// New looks of the materials (other specs): the colours change, the
    /// hits stay.
    pub fn set_looks(&mut self, looks: Arc<Looks>) {
        self.looks = looks;
        self.set_colouring(self.by);
    }

    /// Show `vis`, working out its skip here: [`View3d::show`] takes one
    /// made already (off the UI thread).
    pub fn set_visibility(&mut self, vis: &Visibility) -> llevant::Result<()> {
        self.show(&backend::Shown::new(&self.model, None, vis.clone())?)
    }

    /// Show `shown` (made for this view's model).
    pub fn show(&mut self, shown: &backend::Shown) -> llevant::Result<()> {
        let vis = &shown.vis;
        self.backend.set_visibility(shown)?;
        self.vis = vis.clone();
        self.ghosts = vis.has_ghosts();
        self.chain.set_ghosts(self.ghosts);
        // Rays may now cost far more (hidden cells are stepped through):
        // measure again from small batches, so that none runs long.
        self.scheduler.reset_cost();
        self.restart();
        Ok(())
    }

    /// Trace a step of a surface being dragged from now on, from the start
    /// (previews first).
    pub fn draft(&mut self, step: &backend::DraftStep) -> llevant::Result<()> {
        self.backend.draft(step)?;
        self.restart();
        self.dirty = true;
        Ok(())
    }

    /// Draw `field` over the cells, or none (no new trace). An error,
    /// and no field drawn, if its data is larger than the device takes in
    /// one buffer.
    pub fn set_field(&mut self, field: Option<Overlay>) -> Result<(), String> {
        let words = field
            .as_ref()
            .filter(|f| self.field_data.as_ref().map(|d| d.0) != Some(field::DataKey::of(f)))
            .map(field::field_words);
        self.set_field_words(field, words.as_deref())
    }

    /// [`View3d::set_field`] with the field's data words made already
    /// ([`field::field_words`], off the UI thread), or `None` when they are
    /// those uploaded last (same data and ramp: only the scale or opacity
    /// changed, which costs nothing).
    pub fn set_field_words(
        &mut self,
        field: Option<Overlay>,
        words: Option<&[u32]>,
    ) -> Result<(), String> {
        let Some(f) = field else {
            self.chain.set_enabled("field", false);
            self.field = None;
            self.dirty = true;
            return Ok(());
        };
        let key = field::DataKey::of(&f);
        let words: Option<std::borrow::Cow<[u32]>> = match words {
            Some(w) => Some(w.into()),
            None if self.field_data.as_ref().map(|d| d.0) == Some(key) => None,
            // Not made for this data: make them here.
            None => Some(field::field_words(&f).into()),
        };
        if let Some(words) = words {
            let bytes = 4 * words.len() as u64;
            let limits = self.gpu.device.limits();
            let max = limits
                .max_storage_buffer_binding_size
                .min(limits.max_buffer_size);
            let error = (bytes > max).then(|| {
                format!(
                    "the field needs {} MB on the GPU; this device takes {} MB in one buffer",
                    bytes >> 20,
                    max >> 20
                )
            });
            if error.is_none() {
                self.chain
                    .set_data(&self.gpu, "field", &[bytemuck::cast_slice(&words)])
                    .expect("the field pass is built in");
            }
            self.field_data = Some((key, error));
        }
        let error = self.field_data.as_ref().and_then(|d| d.1.clone());
        let shown = error.is_none();
        self.chain.set_enabled("field", shown);
        if shown {
            self.chain.set_params("field", field::field_params(&f.look));
        }
        self.field = shown.then_some(f);
        self.dirty = true;
        error.map_or(Ok(()), Err)
    }

    /// The field drawn over the cells.
    pub fn field(&self) -> Option<&Overlay> {
        self.field.as_ref()
    }

    /// Tint and outline these cells (no new trace).
    pub fn set_selection(&mut self, cells: &CellSet) {
        let words: &[u32] = bytemuck::cast_slice(cells.words());
        self.marks = self
            .gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("selection"),
                contents: bytemuck::cast_slice(if words.is_empty() { &[0u32] } else { words }),
                usage: wgpu::BufferUsages::STORAGE,
            });
        self.dirty = true;
    }

    /// Mark these surfaces ([`edits::surface_words`], empty for none) and
    /// lay this shell over the image (no new trace).
    pub fn set_edit_marks(&mut self, surfaces: Vec<f32>, shell: Option<Arc<edits::Shell>>) {
        let same_shell = match (&self.shell, &shell) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same_shell && surfaces == self.edit_surfaces {
            return;
        }
        let on = !surfaces.is_empty() || shell.is_some();
        if on {
            let s: &[f32] = shell.as_ref().map_or(&[], |s| &s.words);
            self.chain
                .set_data(
                    &self.gpu,
                    "marks",
                    &[bytemuck::cast_slice(&surfaces), bytemuck::cast_slice(s)],
                )
                .expect("the marks pass is built in");
        }
        self.chain.set_enabled("marks", on);
        self.edit_surfaces = surfaces;
        self.shell = shell;
        self.dirty = true;
    }

    /// Outline the cell under the pointer (index into the model's cells),
    /// or none.
    pub fn hover(&mut self, cell: Option<u32>) {
        let c = cell.unwrap_or(NONE);
        if c != self.hovered {
            self.hovered = c;
            self.dirty = true;
        }
    }

    /// Colours of the selection and of the cell under the pointer.
    pub fn set_highlight(&mut self, h: passes::Highlight) {
        self.chain.set_highlight(h);
        self.dirty = true;
    }

    /// Switch a post-process pass on or off.
    pub fn set_effect(&mut self, name: &str, on: bool) {
        if self.chain.set_enabled(name, on) {
            self.dirty = true;
        }
    }

    /// The pass chain, to add effects.
    pub fn passes(&mut self) -> &mut PassChain {
        self.dirty = true;
        &mut self.chain
    }

    pub fn gpu(&self) -> &Arc<Gpu> {
        &self.gpu
    }

    fn restart(&mut self) {
        let generation = self.scheduler.restart(self.width, self.height);
        self.backend.cancel_full_before(generation);
    }

    /// Whether more frames are needed to finish the image.
    pub fn busy(&self) -> bool {
        !self.scheduler.idle() || self.backend.in_flight() > 0 || self.dirty
    }

    /// Show `camera` (its size is the image's, in physical pixels): trace
    /// what the budget allows this frame and compose the image. Returns
    /// the egui texture to draw.
    pub fn frame(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        camera: Camera,
    ) -> Option<egui::TextureId> {
        let (w, h) = (camera.width as u32, camera.height as u32);
        if (w, h) != (self.width, self.height) {
            self.resize(render_state, w, h);
        }
        if self.desired.as_ref() != Some(&camera) {
            self.desired = Some(camera);
            self.restart();
        }
        let textures = self.textures(render_state);

        if let Some(why) = self.backend.lost() {
            // Its batches will never finish: trace on the CPU from now on.
            let backend = backend::CpuBackend::new(self.model.clone(), self.vis.clone());
            self.set_backend(Box::new(backend));
            self.notice = Some(format!("GPU lost ({why}): the 3D view traces on the CPU"));
        }
        for f in self.backend.poll() {
            self.finished(f);
        }
        // As many batches as the backend takes now (the GPU queues two).
        while let Some(level) = self.scheduler.next_level() {
            if !self.backend.ready(level) {
                break;
            }
            let Some(batch) = self.scheduler.next_batch() else {
                break;
            };
            self.submit(batch);
        }
        // While the camera moves, the hits shown follow it every frame.
        if self.reprojecting && self.composed != self.desired {
            self.dirty = true;
        }
        if self.dirty && self.shown_camera.is_some() {
            let gpu = self.gpu.clone();
            let mut enc = gpu.device.create_command_encoder(&Default::default());
            self.compose(&mut enc);
            self.dirty = false;
            gpu.queue.submit([enc.finish()]);
        }
        self.shown_camera.as_ref()?;
        Some(textures[self.out])
    }

    fn textures(&mut self, rs: &egui_wgpu::RenderState) -> [egui::TextureId; 2] {
        if let Some(t) = self.textures {
            return t;
        }
        let mut r = rs.renderer.write();
        let t = [0, 1].map(|k| {
            r.register_native_texture(
                &rs.device,
                &self.targets.views[k],
                wgpu::FilterMode::Nearest,
            )
        });
        self.textures = Some(t);
        t
    }

    fn resize(&mut self, rs: &egui_wgpu::RenderState, w: u32, h: u32) {
        let gpu = &self.gpu;
        self.width = w;
        self.height = h;
        self.full = HitBuffer::with_ghosts(gpu, w, h);
        self.gbuf = storage(gpu, "gbuf", G_WORDS * (w * h) as usize);
        let blocks = (w.div_ceil(BLOCK) * h.div_ceil(BLOCK)) as usize;
        self.mask = vec![0; blocks];
        self.landed = 0;
        self.mask_buffer = storage(gpu, "mask", blocks);
        self.targets = Targets::new(gpu, w, h);
        if let Some(t) = self.textures {
            let mut r = rs.renderer.write();
            for k in 0..2 {
                r.update_egui_texture_from_wgpu_texture(
                    &rs.device,
                    &self.targets.views[k],
                    wgpu::FilterMode::Nearest,
                    t[k],
                );
            }
        }
        self.dirty = true;
        self.restart();
    }

    /// Start tracing `batch`.
    fn submit(&mut self, batch: Batch) {
        if batch.generation != self.trace_generation || self.trace_camera.is_none() {
            self.trace_generation = batch.generation;
            self.trace_camera = self.desired.clone();
        }
        let Some(full_camera) = self.trace_camera.clone() else {
            return;
        };
        let mut camera = full_camera;
        let pending = 1 - self.shown;
        if let Level::Low { width, height } = batch.level {
            camera.width = width as usize;
            camera.height = height as usize;
            if (self.low[pending].width, self.low[pending].height) != (width, height) {
                self.low[pending] = HitBuffer::with_ghosts(&self.gpu, width, height);
            }
        }
        if let Err(e) = self.backend.submit(&camera, batch) {
            eprintln!("3D view: {e}");
        }
    }

    fn finished(&mut self, f: Finished) {
        if f.cancelled {
            return;
        }
        if !f.more {
            self.scheduler
                .record(f.batch.level, f.batch.rays(), f.seconds);
            log::debug!(
                "batch {:?} {} tiles, {} rays: {:.2} ms",
                f.batch.level,
                f.batch.tiles.len(),
                f.batch.rays(),
                1e3 * f.seconds
            );
        }
        if f.batch.generation != self.trace_generation {
            return;
        }
        let out = match f.batch.level {
            Level::Low { width, height } => {
                let pending = 1 - self.shown;
                if (self.low[pending].width, self.low[pending].height) != (width, height) {
                    return;
                }
                &self.low[pending]
            }
            Level::Full => {
                // Traced before a resize (the new size's trace not started
                // yet): its pieces are of the old buffers.
                let size = (self.width as usize, self.height as usize);
                if self
                    .trace_camera
                    .as_ref()
                    .is_none_or(|c| (c.width, c.height) != size)
                {
                    return;
                }
                &self.full
            }
        };
        for (t, w) in &f.pieces {
            upload_tile(&self.gpu, out, *t, w);
        }
        self.land(&f);
    }

    /// The hits of `f` are in their buffer (in queue order): show them.
    fn land(&mut self, f: &Finished) {
        match f.batch.level {
            Level::Low { .. } => {
                if !f.more && f.batch.completes_level {
                    self.previews += 1;
                    self.shown = 1 - self.shown;
                    self.shown_camera = self.trace_camera.clone();
                    self.mask.fill(0);
                    self.landed = 0;
                    self.write_mask();
                    self.dirty = true;
                }
            }
            Level::Full => {
                let bx = self.width.div_ceil(BLOCK);
                let mut new = false;
                for &[x, y] in &f.blocks {
                    let m = &mut self.mask[(y * bx + x) as usize];
                    if *m == 0 {
                        *m = 1;
                        self.landed += 1;
                        new = true;
                    }
                }
                if new {
                    self.write_mask();
                    self.dirty = true;
                }
            }
        }
    }

    fn write_mask(&self) {
        self.gpu
            .queue
            .write_buffer(&self.mask_buffer, 0, bytemuck::cast_slice(&self.mask));
    }

    fn compose(&mut self, enc: &mut wgpu::CommandEncoder) {
        let Some(camera) = self.shown_camera.clone() else {
            return;
        };
        let gpu = self.gpu.clone();
        let low = &self.low[self.shown];
        let u = ResolveUniform {
            size: [self.width, self.height, low.width, low.height],
            blocks: [self.width.div_ceil(BLOCK), self.ghosts as u32, 0, 0],
            cam: reproject::Cam::new(&camera),
        };
        let uniform = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("resolve"),
                contents: bytemuck::bytes_of(&u),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let buffers = [
            &uniform,
            &low.buffer,
            &self.full.buffer,
            &self.mask_buffer,
            &self.gbuf,
        ];
        let entries: Vec<wgpu::BindGroupEntry> = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource: b.as_entire_binding(),
            })
            .collect();
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("resolve"),
            layout: &self.resolve_layout,
            entries: &entries,
        });
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.resolve);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(self.width.div_ceil(8), self.height.div_ceil(8), 1);
        }
        // Shading uses the directions of the full image of the camera
        // traced, or of the camera wanted when the hits are moved to it.
        let mut cam = camera;
        cam.width = self.width as usize;
        cam.height = self.height as usize;
        let mut gbuf = &self.gbuf;
        if let Some(to) = self
            .desired
            .clone()
            .filter(|d| self.reprojecting && (self.always_reproject || !same_pose(d, &cam)))
        {
            self.reproject.encode(
                &gpu,
                enc,
                &self.gbuf,
                self.width,
                self.height,
                self.ghosts,
                &cam,
                &to,
            );
            gbuf = &self.reproject.out;
            cam = to;
        }
        self.composed = self.desired.clone();
        // The shell only over the image it was traced for (it is traced
        // again when the camera stops).
        let shell = self
            .shell
            .as_ref()
            .filter(|s| same_pose(&with_size(&s.camera, &cam), &cam));
        let on = |b: bool| if b { 1.0 } else { 0.0 };
        self.chain.set_params(
            "marks",
            passes::eight([
                shell.map_or(1.0, |s| s.width as f32),
                shell.map_or(1.0, |s| s.height as f32),
                on(shell.is_some()),
                on(!self.edit_surfaces.is_empty()),
                0.0,
                0.0,
                0.0,
                0.0,
            ]),
        );
        let mut frame = FrameUniform::new(&cam, self.hovered);
        if !self.pbr {
            frame = frame.with_background(self.background);
        }
        let buffers = passes::PassBuffers {
            gbuf,
            palette: &self.palette,
            marks: &self.marks,
        };
        self.out = self
            .chain
            .encode(&gpu, enc, &frame, &buffers, &self.targets);
    }

    /// Compose the image again from the hits already traced and wait for
    /// it: seconds taken (resolve and the passes on), for benchmarks.
    #[doc(hidden)]
    pub fn compose_blocking(&mut self) -> f64 {
        let gpu = self.gpu.clone();
        let start = std::time::Instant::now();
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        self.compose(&mut enc);
        let index = gpu.queue.submit([enc.finish()]);
        let _ = gpu.device.poll(wgpu::PollType::Wait {
            submission_index: Some(index),
            timeout: None,
        });
        start.elapsed().as_secs_f64()
    }

    /// Size of the image, in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Read the composed image back (for tests and screenshots): RGBA rows.
    pub fn read_image(&self) -> Vec<u8> {
        read_texture(&self.gpu, &self.targets.textures[self.out])
    }

    /// Trace everything at once and compose (no egui): for tests and
    /// image export.
    pub fn render_now(&mut self, camera: Camera) -> Vec<u8> {
        let (w, h) = (camera.width as u32, camera.height as u32);
        let gpu = self.gpu.clone();
        if (w, h) != (self.width, self.height) {
            self.width = w;
            self.height = h;
            self.full = HitBuffer::with_ghosts(&gpu, w, h);
            self.gbuf = storage(&gpu, "gbuf", G_WORDS * (w * h) as usize);
            let blocks = (w.div_ceil(BLOCK) * h.div_ceil(BLOCK)) as usize;
            self.mask = vec![0; blocks];
            self.landed = 0;
            self.mask_buffer = storage(&gpu, "mask", blocks);
            self.targets = Targets::new(&gpu, w, h);
        }
        self.desired = Some(camera);
        self.restart();
        loop {
            for f in self.backend.poll() {
                self.finished(f);
            }
            if self.scheduler.idle() && self.backend.in_flight() == 0 {
                break;
            }
            let next = self
                .scheduler
                .next_level()
                .filter(|&l| self.backend.ready(l))
                .and_then(|_| self.scheduler.next_batch());
            match next {
                Some(b) => self.submit(b),
                None => std::thread::sleep(std::time::Duration::from_millis(1)),
            }
        }
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        self.compose(&mut enc);
        self.dirty = false;
        gpu.queue.submit([enc.finish()]);
        self.read_image()
    }
}

impl View3d {
    /// The hits traced so far as seen from `camera` (the same size), without
    /// tracing: the image while the camera moves (blocking, for tests). The
    /// hits are moved even when `camera` is the one they were traced with.
    #[doc(hidden)]
    pub fn reproject_now(&mut self, camera: Camera) -> Vec<u8> {
        let gpu = self.gpu.clone();
        self.desired = Some(camera);
        self.always_reproject = true;
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        self.compose(&mut enc);
        self.always_reproject = false;
        gpu.queue.submit([enc.finish()]);
        self.read_image()
    }
}

/// RGBA rows of a texture (blocking).
pub fn read_texture(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
    let (w, h) = (texture.width(), texture.height());
    let row = (4 * w).div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("image readback"),
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([enc.finish()]);
    llevant_gpu::map_for_reading(&gpu.device, &buffer).expect("GPU readback");
    let data = buffer.get_mapped_range(..).expect("mapped image");
    let mut out = Vec::with_capacity((4 * w * h) as usize);
    for y in 0..h as usize {
        out.extend_from_slice(&data[y * row as usize..][..4 * w as usize]);
    }
    out
}
