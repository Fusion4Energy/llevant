//! A surface dragged live: a thread of its own holds the
//! [`llevant::draft::Draft`], moves it to the latest coefficients asked for
//! (those asked meanwhile are skipped), and hands back each step: a patch
//! for the model on the GPU and, when no newer step waits, a snapshot of the
//! draft's model for the CPU views (the slice, picking). When the drag ends
//! the edit is made to the input and compiled as any other; until that
//! lands the preview stays, and if it fails the GPU copy is patched back.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use llevant::draft::{Draft, DraftPatch};
use llevant::Model;
use llevant_gpu::{Gpu, GpuModel};
use llevant_scene::source::Source;

use crate::view3d::backend::{DraftStep, Shown};

enum ToWorker {
    Set(Vec<f64>),
    /// The scene's visibility changed: widen this one.
    Show(Shown),
    Stop {
        revert: bool,
    },
}

pub enum FromWorker {
    /// The draft is begun: the visibility to trace it with (nothing
    /// skipped where it can change), made for the GPU copy when there is one.
    Ready(Shown),
    /// A step, and how long it took (ms).
    Step(DraftStep, f64),
    /// The coefficients asked for were refused (the draft stays at the
    /// last ones that worked).
    Refused(String),
    /// The surface cannot be dragged live.
    Unavailable(String),
    /// Stopped: the patch that undoes every step, when asked for.
    Stopped(Option<Arc<DraftPatch>>),
}

pub struct Dragging {
    pub surface: i64,
    /// The coefficients as written when the drag began.
    pub start: Vec<f64>,
    /// The latest asked for.
    pub coeffs: Vec<f64>,
    tx: Sender<ToWorker>,
    pub rx: Receiver<FromWorker>,
    /// Ended, the edit being compiled.
    pub committing: bool,
    /// Asked to stop.
    pub stopping: bool,
}

impl Dragging {
    /// Start dragging `surface` of `base`, whose cards are `source`; the 3D
    /// view shows `shown`, and `gpu` is its model on the GPU, if any.
    pub fn start(
        surface: i64,
        start: Vec<f64>,
        base: Arc<Model>,
        source: Arc<Source>,
        shown: Shown,
        gpu: Option<(Arc<Gpu>, Arc<GpuModel>)>,
        ctx: egui::Context,
    ) -> Dragging {
        let (tx, worker_rx) = channel();
        let (worker_tx, rx) = channel();
        std::thread::Builder::new()
            .name("llevant drag".into())
            .spawn(move || work(surface, base, source, shown, gpu, worker_rx, worker_tx, ctx))
            .expect("thread");
        Dragging {
            surface,
            coeffs: start.clone(),
            start,
            tx,
            rx,
            committing: false,
            stopping: false,
        }
    }

    /// Move the surface to `coeffs`.
    pub fn set(&mut self, coeffs: Vec<f64>) {
        self.coeffs = coeffs.clone();
        let _ = self.tx.send(ToWorker::Set(coeffs));
    }

    /// The scene's visibility is now `shown`: a [`FromWorker::Ready`] with
    /// it widened comes back.
    pub fn show(&self, shown: Shown) {
        let _ = self.tx.send(ToWorker::Show(shown));
    }

    /// End the drag; with `revert`, the patch undoing it comes back
    /// ([`FromWorker::Stopped`]).
    pub fn stop(&mut self, revert: bool) {
        self.stopping = true;
        let _ = self.tx.send(ToWorker::Stop { revert });
    }
}

#[allow(clippy::too_many_arguments)]
fn work(
    surface: i64,
    base: Arc<Model>,
    source: Arc<Source>,
    shown: Shown,
    gpu: Option<(Arc<Gpu>, Arc<GpuModel>)>,
    rx: Receiver<ToWorker>,
    tx: Sender<FromWorker>,
    ctx: egui::Context,
) {
    let send = |m: FromWorker| {
        let _ = tx.send(m);
        ctx.request_repaint();
    };
    let begun = {
        let tree = source.tree();
        Draft::begin(base.clone(), &tree, surface)
    };
    let mut draft = match begun {
        Ok(d) => Some(d),
        Err(e) => {
            send(FromWorker::Unavailable(e.to_string()));
            None
        }
    };
    // The visibility `shown` widened by the draft, made for the GPU.
    let widened = |d: &Draft, shown: &Shown| -> llevant::Result<Shown> {
        let skip = Arc::new(d.widen_skip(&shown.skip));
        let gpu = gpu
            .as_ref()
            .map(|(g, gm)| gm.prepare_visibility(g, &base, &shown.vis, &skip))
            .transpose()?;
        Ok(Shown {
            vis: shown.vis.clone(),
            skip,
            gpu,
        })
    };
    if let Some(d) = &draft {
        match widened(d, &shown) {
            Ok(s) => send(FromWorker::Ready(s)),
            Err(e) => {
                send(FromWorker::Unavailable(e.to_string()));
                draft = None;
            }
        }
    }
    let mut stepped = false;
    // Messages taken to see whether any waited.
    let mut held: Vec<ToWorker> = Vec::new();
    loop {
        // What was asked meanwhile: the latest visibility and coefficients,
        // or a stop.
        let (mut set, mut show, mut stop) = (None, None, None);
        let mut take = |m: ToWorker| match m {
            ToWorker::Set(c) => set = Some(c),
            ToWorker::Show(s) => show = Some(s),
            ToWorker::Stop { revert } => stop = Some(revert),
        };
        if held.is_empty() {
            match rx.recv() {
                Ok(m) => take(m),
                Err(_) => return,
            }
        }
        for m in held.drain(..) {
            take(m);
        }
        while let Ok(m) = rx.try_recv() {
            take(m);
        }
        if let Some(revert) = stop {
            let patch = match &draft {
                Some(d) if revert && stepped => Some(Arc::new(d.revert_patch())),
                _ => None,
            };
            send(FromWorker::Stopped(patch));
            return;
        }
        let Some(d) = &mut draft else { continue };
        if let Some(s) = show {
            match widened(d, &s) {
                Ok(s) => send(FromWorker::Ready(s)),
                Err(e) => send(FromWorker::Refused(e.to_string())),
            }
        }
        let Some(c) = set else { continue };
        let t = Instant::now();
        match d.set(&c) {
            Ok(patch) => {
                stepped = true;
                let ms = 1e3 * t.elapsed().as_secs_f64();
                // The CPU views follow when no newer step waits (a
                // snapshot copies the model's tables: tens of ms).
                let newer = rx.try_recv().map(|m| held.push(m)).is_ok();
                let model = (!newer).then(|| Arc::new(d.snapshot()));
                send(FromWorker::Step(
                    DraftStep {
                        patch: Arc::new(patch),
                        model,
                    },
                    ms,
                ));
            }
            Err(e) => send(FromWorker::Refused(e.to_string())),
        }
    }
}
