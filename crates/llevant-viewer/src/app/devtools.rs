//! Hooks for tests, docs and benchmarks ([`DevOptions`], the hidden
//! `--screenshot`, `--orbit` and `--storm` flags of `llevant-viewer`): a
//! screenshot of the window once everything is traced, then quit; before
//! it, turning the camera a while and printing frame times while orbiting
//! and while the view refines (with a screenshot, also
//! `<name>.moving.png` halfway); or the edit storm ([`super::storm`]).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::storm::{Frame, Storm};
use crate::document::Document;

/// What the hooks do; nothing by default.
#[derive(Clone, Debug, Default)]
pub struct DevOptions {
    /// Save the window as this PNG once everything is traced, then quit.
    pub screenshot: Option<PathBuf>,
    /// First turn the camera this many seconds, printing frame times.
    pub orbit_seconds: Option<f64>,
    /// Fail the GPU upload of models opened from now on, as when they do
    /// not fit (tests of the CPU fallback).
    pub fail_gpu_upload: bool,
    /// Show every model opened from now on before its octrees are built,
    /// however small (tests of staged loads; else only large models are).
    pub stage_always: bool,
    /// Run the edit storm, printing the UI thread's own work per step,
    /// then quit.
    pub storm: bool,
    /// The 3D view shows the last image where it was traced until the next
    /// lands, instead of moving the hits to the camera (A/B of
    /// `view3d::reproject`).
    pub no_reprojection: bool,
}

pub struct DevHooks {
    screenshot: Option<PathBuf>,
    orbit: Option<OrbitBench>,
    shot_requested: bool,
    started: Instant,
    fail_gpu_upload: bool,
    stage_always: bool,
    storm: Option<Storm>,
    no_reprojection: bool,
}

struct OrbitBench {
    seconds: f64,
    start: Option<Instant>,
    dts: Vec<f64>,
    scales: Vec<f64>,
    /// Previews shown by the 3D view when the orbit started.
    previews: u64,
    /// When the orbit ended and refinement began.
    refine_start: Option<Instant>,
    /// The screenshot halfway through was asked for.
    mid_shot: bool,
}

fn save_png(path: &Path, img: &egui::ColorImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(
        std::io::BufWriter::new(file),
        img.size[0] as u32,
        img.size[1] as u32,
    );
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    w.write_image_data(&bytes).map_err(|e| e.to_string())
}

/// The screenshot delivered this frame, if any.
fn screenshot_event(ctx: &egui::Context) -> Option<std::sync::Arc<egui::ColorImage>> {
    ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { image, .. } => Some(image.clone()),
            _ => None,
        })
    })
}

impl DevHooks {
    pub fn new(options: &DevOptions) -> DevHooks {
        DevHooks {
            screenshot: options.screenshot.clone(),
            orbit: options.orbit_seconds.map(|seconds| OrbitBench {
                seconds,
                start: None,
                dts: Vec::new(),
                scales: Vec::new(),
                previews: 0,
                refine_start: None,
                mid_shot: false,
            }),
            shot_requested: false,
            started: Instant::now(),
            fail_gpu_upload: options.fail_gpu_upload,
            stage_always: options.stage_always,
            storm: options.storm.then(Storm::new),
            no_reprojection: options.no_reprojection,
        }
    }

    /// What the edit storm does on this frame, if it runs.
    pub fn storm_frame(&mut self, ctx: &egui::Context, doc: Option<&Document>) -> Frame {
        match (&mut self.storm, doc) {
            (Some(s), Some(d)) if !s.done => {
                ctx.request_repaint();
                s.step(d)
            }
            _ => Frame::default(),
        }
    }

    /// At the end of each frame: the UI thread's own work on it.
    pub fn frame_end(&mut self, ctx: &egui::Context, own: Duration, doc: Option<&Document>) {
        let (Some(s), Some(d)) = (&mut self.storm, doc) else {
            return;
        };
        if s.done {
            return;
        }
        s.record(own, d);
        if s.done && self.screenshot.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// [`DevOptions::fail_gpu_upload`].
    pub fn fail_gpu_upload(&self) -> bool {
        self.fail_gpu_upload
    }

    /// [`DevOptions::stage_always`].
    pub fn stage_always(&self) -> bool {
        self.stage_always
    }

    /// Once per frame. `failed`: loading failed (screenshot what is shown).
    pub fn step(
        &mut self,
        ctx: &egui::Context,
        doc: Option<&mut Document>,
        failed: bool,
        fps: f64,
    ) {
        let Some(doc) = doc else {
            if failed {
                self.screenshot_when_ready(ctx, None, fps);
            } else if self.screenshot.is_some() {
                ctx.request_repaint();
            }
            return;
        };
        if self.no_reprojection {
            doc.view3d.set_reprojection(false);
        }
        if self.storm.as_ref().is_some_and(|s| !s.done) {
            return;
        }
        if self.orbit.is_some() {
            self.orbit_step(ctx, doc);
            return;
        }
        self.screenshot_when_ready(ctx, Some(doc), fps);
    }

    fn orbit_step(&mut self, ctx: &egui::Context, d: &mut Document) {
        let Some(b) = &mut self.orbit else { return };
        let now = Instant::now();
        let start = *b.start.get_or_insert_with(|| {
            b.previews = d.view3d.stats().previews;
            now
        });
        if start != now {
            b.dts.push(ctx.input(|i| i.unstable_dt) as f64);
            b.scales.push(d.view3d.stats().scale);
        }
        let orbiting = (now - start).as_secs_f64() < b.seconds;
        // Halfway through, a screenshot of the moving views next to the
        // final one.
        if let (Some(img), Some(path)) = (screenshot_event(ctx), &self.screenshot) {
            if let Err(e) = save_png(&path.with_extension("moving.png"), &img) {
                eprintln!("screenshot: {e}");
            }
        }
        if orbiting {
            // 2 degrees a frame; the slice pans and zooms in.
            d.orbit.orbit(2.0 / 0.3, 0.0);
            d.slice.centre[0] += 0.004 * d.slice.width;
            d.slice.width *= 0.998;
            if !b.mid_shot && (now - start).as_secs_f64() > 0.5 * b.seconds {
                b.mid_shot = true;
                if self.screenshot.is_some() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                }
            }
            ctx.request_repaint();
            return;
        }
        let summary = |dts: &[f64]| {
            let mut dts = dts.to_vec();
            dts.sort_by(f64::total_cmp);
            let q = |f: f64| 1e3 * dts[((dts.len() - 1) as f64 * f) as usize];
            format!(
                "{} frames; frame time median {:.1} ms, 95 % {:.1} ms, max {:.1} ms",
                dts.len(),
                q(0.5),
                q(0.95),
                q(1.0)
            )
        };
        match b.refine_start {
            None => {
                if !b.dts.is_empty() {
                    let mean_scale = b.scales.iter().sum::<f64>() / b.scales.len() as f64;
                    let previews = d.view3d.stats().previews - b.previews;
                    eprintln!(
                        "orbit {:.1} s: {}; mean scale {:.2}; {:.1} previews/s",
                        b.seconds,
                        summary(&b.dts),
                        mean_scale,
                        previews as f64 / (now - start).as_secs_f64()
                    );
                }
                // Now the frames while the still view refines.
                b.refine_start = Some(now);
                b.dts.clear();
                ctx.request_repaint();
            }
            Some(_) if d.view3d.busy() => ctx.request_repaint(),
            Some(t0) => {
                if !b.dts.is_empty() {
                    eprintln!(
                        "refine {:.2} s: {}",
                        (now - t0).as_secs_f64(),
                        summary(&b.dts)
                    );
                }
                self.orbit = None;
                ctx.request_repaint();
            }
        }
    }

    fn screenshot_when_ready(&mut self, ctx: &egui::Context, doc: Option<&mut Document>, fps: f64) {
        let Some(path) = self.screenshot.clone() else {
            return;
        };
        if let Some(img) = screenshot_event(ctx) {
            if let Err(e) = save_png(&path, &img) {
                eprintln!("screenshot: {e}");
            }
            if let Some(d) = &doc {
                let s = d.view3d.stats();
                eprintln!(
                    "screenshot after {:.2} s: 3D {} scale {:.2} refined {:.2}, preview {:?}, refine {:?}; {:.0} fps",
                    self.started.elapsed().as_secs_f64(),
                    s.backend,
                    s.scale,
                    s.refined,
                    s.preview,
                    s.refine,
                    fps
                );
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let ready = doc.is_none_or(|d| !d.view3d.busy() && d.slice.settled());
        if ready && !self.shot_requested {
            self.shot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint();
    }
}
