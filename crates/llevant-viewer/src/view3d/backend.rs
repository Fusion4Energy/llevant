//! Who traces the rays of the 3D view: the GPU renderer (fast, f32) or the
//! CPU engine (exact, for machines without a GPU). Both work in the
//! background and hand back hits in the layout of
//! [`llevant_gpu::HitBuffer`], which the view uploads, so everything after
//! them is the same.
//!
//! The GPU renderer runs on a device of its own, not the window's: a trace
//! can take longer than a frame (the latency of single rays through a big
//! model is tens of milliseconds, whatever the image size), and on the
//! window's queue it would hold up every frame drawn after it. It traces in
//! slices ([`llevant_gpu::Job`]), so the GPU switches to the window's
//! frames every few milliseconds whatever the rays: a frame that waited a
//! second behind tracing lost the window's device.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use llevant::draft::DraftPatch;
use llevant::{Camera, Model, Skip, Visibility, NO_HIT};
use llevant_gpu::{
    tile_blocks, Gpu, GpuModel, Hit, HitBuffer, Job, Progress, Status, Tile, GHOST_WORDS,
    HIT_WORDS, NONE,
};

use super::budget::{Batch, Level};

/// Hits of a batch that landed: pieces of it while it is traced, and its
/// end.
pub struct Finished {
    /// The batch submitted.
    pub batch: Batch,
    /// Time of the whole batch, in the last message of it.
    pub seconds: f64,
    /// Pieces of the batch's image with their hits, row by row; with
    /// ghosts, followed by the ghost records of its pixels, row by row
    /// ([`GHOST_WORDS`] each). A piece may hold pixels of blocks not done
    /// yet: only `blocks` are complete.
    pub pieces: Vec<(Tile, Vec<u32>)>,
    /// The 8 × 8 blocks of the image these pieces complete.
    pub blocks: Vec<[u32; 2]>,
    /// Given up because a later generation was submitted: no hits, and
    /// `seconds` says nothing of its cost.
    pub cancelled: bool,
    /// More of this batch follows (it is still in flight).
    pub more: bool,
}

impl Finished {
    fn cancelled(batch: Batch) -> Finished {
        Finished {
            batch,
            seconds: 0.0,
            pieces: Vec::new(),
            blocks: Vec::new(),
            cancelled: true,
            more: false,
        }
    }
}

pub trait TraceBackend {
    fn name(&self) -> &'static str;
    /// Whether a batch of `level` may be submitted now.
    fn ready(&self, level: Level) -> bool;
    /// Start tracing `batch` of `camera`'s image (sized as the batch's
    /// level). The hits come back from [`TraceBackend::poll`].
    fn submit(&mut self, camera: &Camera, batch: Batch) -> llevant::Result<()>;
    /// What landed since the last call: pieces of batches in flight, and
    /// batches finished (`more` false).
    fn poll(&mut self) -> Vec<Finished>;
    /// Trace with `shown` from now on (batches in flight keep theirs).
    fn set_visibility(&mut self, shown: &Shown) -> llevant::Result<()>;
    /// Batches submitted and not yet finished.
    fn in_flight(&self) -> usize;
    /// Why its GPU device was lost (batches on it never finish), if it was.
    fn lost(&self) -> Option<String> {
        None
    }
    /// Trace a step of a dragged surface from now on ([`DraftStep`]):
    /// batches submitted after see it.
    fn draft(&mut self, step: &DraftStep) -> llevant::Result<()>;
    /// The view moved on to `generation`: give up full-resolution batches
    /// of earlier ones (they come back cancelled). Previews in flight run
    /// on, since an older preview is still shown, moved to the camera.
    fn cancel_full_before(&mut self, generation: u64);
}

/// A step of a surface being dragged ([`llevant::draft::Draft`]): its patch for a
/// model on the GPU, and the draft's model when one was made (the CPU
/// backend traces it; the GPU one is patched instead).
#[derive(Clone)]
pub struct DraftStep {
    pub patch: Arc<DraftPatch>,
    pub model: Option<Arc<Model>>,
}

/// A visibility made ready for the backends: its skip ([`Model::skip`])
/// and, for a model on the GPU, its upload
/// ([`GpuModel::prepare_visibility`]). Making one is the work of a new
/// visibility (~50 ms on the reference model), so the viewer makes it off
/// the UI thread; switching a backend to it costs nothing.
#[derive(Clone)]
pub struct Shown {
    pub vis: Visibility,
    pub skip: Arc<Skip>,
    /// For the model on the GPU it was made with, if any.
    pub gpu: Option<llevant_gpu::GpuVisibility>,
}

impl Shown {
    /// `vis` of `model`, uploaded for `gpu` too when given (the device and
    /// model of a [`GpuBackend`]).
    pub fn new(
        model: &Model,
        gpu: Option<(&Gpu, &GpuModel)>,
        vis: Visibility,
    ) -> llevant::Result<Shown> {
        let skip = Arc::new(model.skip(&vis)?);
        let gpu = gpu
            .map(|(g, gm)| gm.prepare_visibility(g, model, &vis, &skip))
            .transpose()?;
        Ok(Shown { vis, skip, gpu })
    }
}

/// Device features that let the GPU backend time its tracing on the GPU.
pub const TIMESTAMPS: wgpu::Features =
    wgpu::Features::TIMESTAMP_QUERY.union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

/// GPU time of the slices submitted at once, by the time of the last ones:
/// enough that reading a job's progress in between costs little, short
/// enough that a batch of the next camera waits behind little.
const SUBMISSION_SECONDS: f64 = 0.02;

/// Default [`GpuBackend::preview_work`].
pub const PREVIEW_WORK: u32 = 2048;

/// Set `model` up to be traced while a window is drawn on the same GPU:
/// every slice as short as the first ones, so that the window's frames
/// wait a few milliseconds at most. With the copper of the reference model
/// isolated, refining takes ~20 % longer and previews ~30 % (single slow
/// rays then take many slices), where longer slices at the end of a job
/// held frames for up to 0.3 s (`docs/decisions/0005-sliced-gpu-dispatches.md`).
pub fn beside_window(model: &mut GpuModel) {
    model.tail_work = model.slice_work;
}

/// What the backend's thread is sent.
enum Msg {
    Trace(Request),
    /// A draft's patch is waiting ([`GpuBackend::patch`]).
    Wake,
}

/// A batch to trace, sent to the backend's thread.
struct Request {
    batch: Batch,
    camera: Camera,
    /// Locate steps of its rays.
    steps: u32,
    /// Program words of its rays.
    work: u32,
}

/// The GPU renderer on a device of its own. A thread traces each batch a
/// few slices at a time until it is done, reads its hits back and times
/// it: with GPU timestamps around each submission when the device has
/// them, else by the clock.
pub struct GpuBackend {
    gpu: Arc<Gpu>,
    model: Arc<GpuModel>,
    /// The model it was uploaded from, for what visibilities skip.
    engine: Arc<Model>,
    waits: Sender<Msg>,
    /// The latest draft patch not yet written, which the thread writes
    /// between batches (a batch never sees half of one).
    patch: Arc<Mutex<Option<Arc<DraftPatch>>>>,
    done: Receiver<Finished>,
    in_flight: usize,
    /// Locate steps of rays in low-resolution images: fewer than the full
    /// images' bound the time of the slowest rays while the camera moves
    /// (rays that run out are drawn grey, and traced in full once it stops).
    pub preview_steps: u32,
    /// Locate steps of rays in full-resolution tiles. Rays on big models
    /// take ~1 ms per step on the GPU; the reference model needs at most
    /// ~140.
    pub full_steps: u32,
    /// Program words of rays in low-resolution images. A few rays walk
    /// tens of thousands (through void cells with long programs) and
    /// previews wait for them: on the reference model 2048 words give up
    /// on 0.03 % of rays (filled from their neighbours) and make a preview
    /// of the whole window 7× faster. Full-resolution images take the
    /// model's [`GpuModel::max_work`].
    pub preview_work: u32,
    /// Why the device was lost or failed, once it has.
    lost: Arc<Mutex<Option<String>>>,
    /// The latest generation submitted: batches of earlier ones are given up
    /// between slices.
    latest: Arc<AtomicU64>,
    /// Full-resolution batches of generations before this one are given up
    /// between slices ([`TraceBackend::cancel_full_before`]).
    cancel_full: Arc<AtomicU64>,
}

impl GpuBackend {
    /// `gpu` is the device `model` was uploaded to (from `engine`); best
    /// one of its own ([`Gpu::from_adapter`]), not the window's.
    pub fn new(gpu: Arc<Gpu>, model: Arc<GpuModel>, engine: Arc<Model>) -> GpuBackend {
        let (waits, rx) = channel::<Msg>();
        let patch = Arc::new(Mutex::new(None::<Arc<DraftPatch>>));
        let thread_patch = patch.clone();
        let (tx, done) = channel();
        let model_steps = model.max_steps;
        let model_work = model.max_work;
        // Say why, when the GPU goes away (a reset after a job too long for
        // the driver, out of memory): the window's device usually goes with
        // it. Any other error on this device (out of memory for a batch's
        // buffers) also ends its use, instead of wgpu's default panic.
        let lost = Arc::new(Mutex::new(None::<String>));
        let flag = lost.clone();
        gpu.device.set_device_lost_callback(move |reason, message| {
            eprintln!("llevant-viewer: GPU trace device lost ({reason:?}): {message}");
            flag.lock()
                .expect("lost")
                .get_or_insert(format!("{reason:?}: {message}"));
        });
        let flag = lost.clone();
        gpu.device
            .on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
                eprintln!("llevant-viewer: GPU trace device error: {e}");
                flag.lock().expect("lost").get_or_insert(e.to_string());
            }));
        let period = gpu.queue.get_timestamp_period() as f64;
        let queries = gpu.device.features().contains(TIMESTAMPS).then(|| {
            gpu.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("trace time"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            })
        });
        let thread_gpu = gpu.clone();
        let thread_model = model.clone();
        let thread_lost = lost.clone();
        let latest = Arc::new(AtomicU64::new(0));
        let thread_latest = latest.clone();
        let cancel_full = Arc::new(AtomicU64::new(0));
        let thread_cancel = cancel_full.clone();
        std::thread::Builder::new()
            .name("llevant gpu trace".into())
            .spawn(move || {
                let (gpu, model) = (thread_gpu, thread_model);
                // Hit buffers by image size (the renderer writes whole
                // images).
                let mut images = HashMap::new();
                for msg in rx {
                    // A draft's patch lands between batches, before any
                    // submitted after it.
                    let pending = thread_patch.lock().expect("patch").take();
                    if let Some(p) = pending {
                        if let Err(e) = model.patch(&gpu, &p) {
                            eprintln!("llevant-viewer: draft patch: {e}");
                        }
                    }
                    let r = match msg {
                        Msg::Wake => continue,
                        Msg::Trace(r) => r,
                    };
                    let generation = r.batch.generation;
                    let full = r.batch.level == Level::Full;
                    let stale = || {
                        thread_latest.load(Ordering::Relaxed) > generation
                            || (full && thread_cancel.load(Ordering::Relaxed) > generation)
                    };
                    if stale() {
                        if tx.send(Finished::cancelled(r.batch)).is_err() {
                            break;
                        }
                        continue;
                    }
                    let (job, blocks) = match prepare(&gpu, &model, &mut images, &r) {
                        Ok(w) => w,
                        Err(e) => {
                            // As any other error on this device: it ends
                            // its use.
                            eprintln!("llevant-viewer: GPU trace: {e}");
                            thread_lost
                                .lock()
                                .expect("lost")
                                .get_or_insert(e.to_string());
                            if tx.send(Finished::cancelled(r.batch)).is_err() {
                                break;
                            }
                            continue;
                        }
                    };
                    let size = (r.camera.width as u32, r.camera.height as u32);
                    let mut land = Landings {
                        out: &images[&size],
                        ghosts: model.has_ghosts(),
                        blocks: &blocks,
                        sent: vec![false; blocks.len()],
                        pending: Vec::new(),
                        batch: &r.batch,
                        tx: &tx,
                    };
                    let traced = trace(&gpu, &job, queries.as_ref(), period, stale, &mut land);
                    let end = match traced {
                        Some(Traced::Done(seconds)) => Finished {
                            batch: r.batch,
                            seconds,
                            pieces: Vec::new(),
                            blocks: Vec::new(),
                            cancelled: false,
                            more: false,
                        },
                        Some(Traced::Cancelled) => Finished::cancelled(r.batch),
                        // The device failed: its batches never finish.
                        None => continue,
                    };
                    if tx.send(end).is_err() {
                        break;
                    }
                }
            })
            .expect("thread");
        GpuBackend {
            gpu,
            model,
            engine,
            waits,
            patch,
            done,
            in_flight: 0,
            preview_steps: 256,
            full_steps: model_steps.min(1024),
            preview_work: PREVIEW_WORK.min(model_work),
            lost,
            latest,
            cancel_full,
        }
    }
}

/// The job of request `r`, tracing into the hit buffer of its image's size
/// (kept in `images`), and its 8 × 8 blocks in the order traced.
fn prepare(
    gpu: &Gpu,
    model: &GpuModel,
    images: &mut HashMap<(u32, u32), HitBuffer>,
    r: &Request,
) -> llevant::Result<(Job, Vec<[u32; 2]>)> {
    let camera = &r.camera;
    let size = (camera.width as u32, camera.height as u32);
    // Only the buffers of the two latest sizes (the low and full images).
    if !images.contains_key(&size) && images.len() >= 2 {
        images.clear();
    }
    let out = images
        .entry(size)
        .or_insert_with(|| HitBuffer::with_ghosts(gpu, size.0, size.1));
    // The 8 × 8 blocks of the tiles, each once.
    let mut seen = HashSet::new();
    let blocks: Vec<[u32; 2]> = r
        .batch
        .tiles
        .iter()
        .flat_map(|t| tile_blocks(*t))
        .filter(|b| seen.insert(*b))
        .collect();
    let job = model.camera_job(gpu, camera, &blocks, out, r.steps, r.work)?;
    Ok((job, blocks))
}

/// Hits of a job's blocks read back as they are done, and sent as
/// [`Finished`] pieces of its batch.
struct Landings<'a> {
    /// The hit buffer the job writes, sized as its image.
    out: &'a HitBuffer,
    /// Whether ghost records follow the hits.
    ghosts: bool,
    /// The job's blocks, in its order (as [`Progress::blocks_left`]).
    blocks: &'a [[u32; 2]],
    /// Blocks done and read back, or to be.
    sent: Vec<bool>,
    /// Blocks done, not read back yet.
    pending: Vec<usize>,
    batch: &'a Batch,
    tx: &'a Sender<Finished>,
}

/// Blocks read back in one submission, and where.
struct Landing {
    readback: wgpu::Buffer,
    pieces: Vec<Tile>,
    blocks: Vec<[u32; 2]>,
}

impl Landings<'_> {
    /// Words read back per ray: its hit, and its ghost record when there
    /// are ghosts.
    fn per_ray(&self) -> usize {
        HIT_WORDS + if self.ghosts { GHOST_WORDS } else { 0 }
    }

    /// Note the blocks `progress` says are done.
    fn read(&mut self, progress: &Progress) {
        for (k, &left) in progress.blocks_left.iter().enumerate() {
            if left == 0 && !self.sent[k] {
                self.sent[k] = true;
                self.pending.push(k);
            }
        }
    }

    /// Record into `enc` copies of the pending blocks' hits: per row of
    /// blocks, one piece across those done in it.
    fn encode(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder) -> Option<Landing> {
        if self.pending.is_empty() {
            return None;
        }
        let (w, h) = (self.out.width, self.out.height);
        let mut pending = std::mem::take(&mut self.pending);
        pending.sort_by_key(|&k| (self.blocks[k][1], self.blocks[k][0]));
        let blocks: Vec<[u32; 2]> = pending.iter().map(|&k| self.blocks[k]).collect();
        // Per row of blocks: its first and past its last block done.
        let mut rows: Vec<(u32, u32, u32)> = Vec::new();
        for &[bx, by] in &blocks {
            match rows.last_mut() {
                Some((y, _, hi)) if *y == by => *hi = bx + 1,
                _ => rows.push((by, bx, bx + 1)),
            }
        }
        let pieces: Vec<Tile> = rows
            .iter()
            .map(|&(by, lo, hi)| Tile {
                x: 8 * lo,
                y: 8 * by,
                width: (8 * hi).min(w) - 8 * lo,
                height: 8.min(h - 8 * by),
            })
            .collect();
        let rays: u64 = pieces.iter().map(Tile::pixels).sum();
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hits readback"),
            size: (self.per_ray() as u64 * 4 * rays).max(4),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Each piece's rows: its hits, then its ghost records.
        let mut at = 0u64;
        let ghost_at = 4 * self.out.ghost_offset() as u64;
        for t in &pieces {
            let mut rows = |words: usize, base: u64| {
                let row_bytes = words as u64 * 4 * t.width as u64;
                for y in t.y..t.y + t.height {
                    let pixel = y as u64 * w as u64 + t.x as u64;
                    let from = base + pixel * words as u64 * 4;
                    enc.copy_buffer_to_buffer(&self.out.buffer, from, &readback, at, row_bytes);
                    at += row_bytes;
                }
            };
            rows(HIT_WORDS, 0);
            if self.ghosts {
                rows(GHOST_WORDS, ghost_at);
            }
        }
        Some(Landing {
            readback,
            pieces,
            blocks,
        })
    }

    /// Read `l` back (its submission done) and send it. `None` when the
    /// device fails.
    fn send(&self, gpu: &Gpu, l: Landing) -> Option<()> {
        llevant_gpu::map_for_reading(&gpu.device, &l.readback).ok()?;
        let view = l.readback.get_mapped_range(..).ok()?;
        let all: &[u32] = bytemuck::cast_slice(&view);
        let per_ray = self.per_ray();
        let mut at = 0;
        let pieces = l
            .pieces
            .into_iter()
            .map(|t| {
                let n = per_ray * t.pixels() as usize;
                at += n;
                (t, all[at - n..at].to_vec())
            })
            .collect();
        drop(view);
        let _ = self.tx.send(Finished {
            batch: self.batch.clone(),
            seconds: 0.0,
            pieces,
            blocks: l.blocks,
            cancelled: false,
            more: true,
        });
        Some(())
    }
}

/// How a batch's job ended.
enum Traced {
    /// Done, in this many seconds.
    Done(f64),
    Cancelled,
}

/// Run `job` to its end, [`SUBMISSION_SECONDS`]' worth of slices per
/// submission, unless `stale` says so before one; its GPU time: the sum of
/// each submission's between `queries`' timestamps, else by the clock.
/// `None` when the device fails.
fn trace(
    gpu: &Gpu,
    job: &Job,
    queries: Option<&wgpu::QuerySet>,
    period: f64,
    stale: impl Fn() -> bool,
    land: &mut Landings,
) -> Option<Traced> {
    // The two timestamps first (16 bytes), then the job's progress.
    let stamps = queries.map(|_| {
        gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("trace time"),
            size: 16,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    });
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("trace progress"),
        size: 16 + job.progress_bytes(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let start = Instant::now();
    let mut stamped = 0.0;
    let mut slices = 1;
    loop {
        if stale() {
            return Some(Traced::Cancelled);
        }
        let sent = Instant::now();
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        // The blocks done by the last submission go back with this one.
        let landing = land.encode(gpu, &mut enc);
        if let Some(q) = queries {
            enc.write_timestamp(q, 0);
        }
        job.encode_slices(&mut enc, slices);
        if let (Some(q), Some(b)) = (queries, &stamps) {
            enc.write_timestamp(q, 1);
            enc.resolve_query_set(q, 0..2, b, 0);
            enc.copy_buffer_to_buffer(b, 0, &read, 0, 16);
        }
        job.encode_progress(&mut enc, &read, 16);
        gpu.queue.submit([enc.finish()]);
        llevant_gpu::map_for_reading(&gpu.device, &read).ok()?;
        let progress = {
            let view = read.get_mapped_range(..).ok()?;
            let words: &[u32] = bytemuck::cast_slice(&view);
            let t: &[u64] = bytemuck::cast_slice(&view[..16]);
            let mut took = sent.elapsed().as_secs_f64();
            if t[1] > t[0] {
                took = (t[1] - t[0]) as f64 * period * 1e-9;
                stamped += took;
            }
            (Progress::from_words(&words[4..]), took)
        };
        let (progress, took) = progress;
        read.unmap();
        if let Some(l) = landing {
            land.send(gpu, l)?;
        }
        land.read(&progress);
        if progress.done {
            let clock = start.elapsed().as_secs_f64();
            let mut enc = gpu.device.create_command_encoder(&Default::default());
            if let Some(l) = land.encode(gpu, &mut enc) {
                gpu.queue.submit([enc.finish()]);
                land.send(gpu, l)?;
            }
            return Some(Traced::Done(if stamped > 0.0 { stamped } else { clock }));
        }
        let per_slice = took / slices as f64;
        slices = ((SUBMISSION_SECONDS / per_slice.max(1e-5)) as usize).clamp(1, 256);
    }
}

impl TraceBackend for GpuBackend {
    fn name(&self) -> &'static str {
        "GPU"
    }

    /// One batch at a time. A preview sent behind another while the camera
    /// moves makes that one stale before it lands, and none would be
    /// shown. The full-resolution image (one batch) only once the preview
    /// has landed: never in the frame a preview is sent, for a camera that
    /// has moved on by the next one.
    fn ready(&self, _level: Level) -> bool {
        self.in_flight == 0
    }

    fn submit(&mut self, camera: &Camera, batch: Batch) -> llevant::Result<()> {
        self.latest.fetch_max(batch.generation, Ordering::Relaxed);
        let (steps, work) = match batch.level {
            Level::Low { .. } => (self.preview_steps, self.preview_work),
            Level::Full => (self.full_steps, self.model.max_work),
        };
        self.in_flight += 1;
        let _ = self.waits.send(Msg::Trace(Request {
            batch,
            camera: camera.clone(),
            steps,
            work,
        }));
        Ok(())
    }

    fn poll(&mut self) -> Vec<Finished> {
        let out: Vec<Finished> = self.done.try_iter().collect();
        self.in_flight -= out.iter().filter(|f| !f.more).count();
        out
    }

    fn set_visibility(&mut self, shown: &Shown) -> llevant::Result<()> {
        match &shown.gpu {
            Some(v) => {
                self.model.use_visibility(v);
                Ok(())
            }
            None => {
                self.model
                    .set_visibility_with(&self.gpu, &self.engine, &shown.vis, &shown.skip)
            }
        }
    }

    fn in_flight(&self) -> usize {
        self.in_flight
    }

    fn lost(&self) -> Option<String> {
        self.lost.lock().expect("lost").clone()
    }

    fn draft(&mut self, step: &DraftStep) -> llevant::Result<()> {
        *self.patch.lock().expect("patch") = Some(step.patch.clone());
        let _ = self.waits.send(Msg::Wake);
        Ok(())
    }

    fn cancel_full_before(&mut self, generation: u64) {
        self.cancel_full.fetch_max(generation, Ordering::Relaxed);
    }
}

/// The CPU engine on the rayon pool: full-resolution tiles exact, previews
/// (interactive, so permissive as the GPU renderer is) without gaps and
/// problems, where only what holds something visible is searched
/// ([`Model::pixel_hits`]).
pub struct CpuBackend {
    model: Arc<Model>,
    vis: Arc<Visibility>,
    /// `vis` without gaps and problems, and its skip, for previews.
    preview: Arc<(Visibility, Arc<Skip>)>,
    tx: Sender<Finished>,
    done: Receiver<Finished>,
    in_flight: usize,
    /// As [`GpuBackend`]'s: full-resolution batches of earlier generations
    /// are given up between tiles.
    cancel_full: Arc<AtomicU64>,
}

impl CpuBackend {
    /// Showing `vis` (its skip found here: [`CpuBackend::with`] takes one
    /// made already).
    pub fn new(model: Arc<Model>, vis: Visibility) -> CpuBackend {
        let shown = Shown::new(&model, None, vis).expect("a visibility of the model");
        CpuBackend::with(model, &shown)
    }

    /// Showing `shown`.
    pub fn with(model: Arc<Model>, shown: &Shown) -> CpuBackend {
        let (tx, done) = channel();
        CpuBackend {
            preview: Arc::new((quiet(&shown.vis), shown.skip.clone())),
            model,
            vis: Arc::new(shown.vis.clone()),
            tx,
            done,
            in_flight: 0,
            cancel_full: Arc::new(AtomicU64::new(0)),
        }
    }
}

/// `vis` without gaps and problems.
fn quiet(vis: &Visibility) -> Visibility {
    Visibility {
        gaps: false,
        problems: false,
        ..vis.clone()
    }
}

impl TraceBackend for CpuBackend {
    fn name(&self) -> &'static str {
        "CPU"
    }

    fn ready(&self, _level: Level) -> bool {
        self.in_flight == 0
    }

    fn submit(&mut self, camera: &Camera, batch: Batch) -> llevant::Result<()> {
        let (m, vis, tx) = (self.model.clone(), self.vis.clone(), self.tx.clone());
        let preview = self.preview.clone();
        let camera = camera.clone();
        let cancel = self.cancel_full.clone();
        self.in_flight += 1;
        rayon::spawn(move || {
            let start = Instant::now();
            let (vis, skip) = match batch.level {
                Level::Low { .. } => (&preview.0, Some(&*preview.1)),
                Level::Full => (&*vis, None),
            };
            let stale =
                || batch.level == Level::Full && cancel.load(Ordering::Relaxed) > batch.generation;
            // Each tile lands as it is done.
            for t in &batch.tiles {
                if stale() {
                    let _ = tx.send(Finished::cancelled(batch));
                    return;
                }
                let words = cpu_tile(&m, vis, &camera, *t, skip);
                let _ = tx.send(Finished {
                    batch: batch.clone(),
                    seconds: 0.0,
                    pieces: vec![(*t, words)],
                    blocks: tile_blocks(*t),
                    cancelled: false,
                    more: true,
                });
            }
            let _ = tx.send(Finished {
                seconds: start.elapsed().as_secs_f64(),
                pieces: Vec::new(),
                blocks: Vec::new(),
                cancelled: false,
                more: false,
                batch,
            });
        });
        Ok(())
    }

    fn poll(&mut self) -> Vec<Finished> {
        let out: Vec<Finished> = self.done.try_iter().collect();
        self.in_flight -= out.iter().filter(|f| !f.more).count();
        out
    }

    fn set_visibility(&mut self, shown: &Shown) -> llevant::Result<()> {
        self.vis = Arc::new(shown.vis.clone());
        self.preview = Arc::new((quiet(&shown.vis), shown.skip.clone()));
        Ok(())
    }

    fn in_flight(&self) -> usize {
        self.in_flight
    }

    fn draft(&mut self, step: &DraftStep) -> llevant::Result<()> {
        if let Some(m) = &step.model {
            self.model = m.clone();
        }
        Ok(())
    }

    fn cancel_full_before(&mut self, generation: u64) {
        self.cancel_full.fetch_max(generation, Ordering::Relaxed);
    }
}

/// Hits of one tile of `camera`'s image, [`HIT_WORDS`] words per pixel, row
/// by row: as the GPU renderer writes them, but exact (with `skip`, as
/// [`Model::pixel_hits`]).
pub fn cpu_tile(
    m: &Model,
    vis: &Visibility,
    camera: &Camera,
    tile: Tile,
    skip: Option<&Skip>,
) -> Vec<u32> {
    let mut pixels = Vec::with_capacity(tile.pixels() as usize);
    for j in tile.y..tile.y + tile.height {
        for i in tile.x..tile.x + tile.width {
            pixels.push((i as usize, j as usize));
        }
    }
    // Cut faces take their normal (`Model::first_hits`).
    let hits = m.pixel_hits(camera, &pixels, vis, skip);
    let mut words = Vec::with_capacity(HIT_WORDS * pixels.len());
    let miss = Hit {
        t: camera.reach as f32,
        status: Status::Miss,
        cell: NONE,
        top_cell: NONE,
        surface: NONE,
        normal: [0.0; 3],
        instance: NONE,
        depth: 0,
        veils: Default::default(),
        steps: 0,
    };
    let Ok(h) = hits else {
        // A degenerate camera: nothing to see.
        for _ in &pixels {
            words.extend(miss.to_words());
        }
        return words;
    };
    let total = m.instances().total();
    let instances: Vec<u32> = h
        .labels
        .iter()
        .map(|l| llevant_gpu::instance_word(total, m.instance(l)))
        .collect();
    for k in 0..pixels.len() {
        let id = h.ids[k];
        let veils = h.veils.get(k).copied().unwrap_or_default();
        let hit = if id == NO_HIT {
            Hit { veils, ..miss }
        } else {
            use llevant::Status as S;
            let label = &h.labels[id as usize];
            let status = match label.status {
                S::Inside if label.cell().is_some() => Status::Inside,
                S::Gap => Status::Gap,
                S::TooDeep => Status::TooDeep,
                _ => Status::Problem,
            };
            Hit {
                t: h.t[k] as f32,
                status,
                cell: label.cell().unwrap_or(NONE),
                top_cell: label.path.first().map_or(NONE, |s| s.cell),
                surface: h.surface[k],
                normal: h.normal[k].map(|x| x as f32),
                instance: instances[id as usize],
                depth: label.path.len().min(63) as u8,
                veils,
                steps: 0,
            }
        };
        words.extend(hit.to_words());
    }
    for g in &h.ghosts {
        words.extend(llevant_gpu::ghost_to_words(g));
    }
    words
}

/// Write the rows of `tile` (`words` as [`cpu_tile`] gives them) into
/// `out`, and its ghost records when they follow and `out` has room.
pub fn upload_tile(gpu: &Gpu, out: &HitBuffer, tile: Tile, words: &[u32]) {
    let n = tile.pixels() as usize;
    let (hits, ghosts) = words.split_at((HIT_WORDS * n).min(words.len()));
    let rows = |words: &[u32], per: usize, base: u64| {
        for (r, chunk) in words.chunks_exact(per * tile.width as usize).enumerate() {
            let pixel = (tile.y as u64 + r as u64) * out.width as u64 + tile.x as u64;
            gpu.queue.write_buffer(
                &out.buffer,
                base + pixel * per as u64 * 4,
                bytemuck::cast_slice(chunk),
            );
        }
    };
    rows(hits, HIT_WORDS, 0);
    if out.ghosts && ghosts.len() == GHOST_WORDS * n {
        rows(ghosts, GHOST_WORDS, 4 * out.ghost_offset() as u64);
    }
}
