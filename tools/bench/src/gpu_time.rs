//! GPU frame times that measure the shader, not the driver's clock policy.
//!
//! A frame rendered and read back leaves the GPU idle while the hits are
//! decoded. Some drivers then keep the GPU at a low clock: on the RTX 3060
//! Laptop, gaps of 2 ms between 24 ms dispatches held it at ~830 of
//! 2100 MHz, and frames timed one at a time came out 2–2.5× slower than back
//! to back (2026-11-15). So frames here are traced one after another
//! without a readback ([`GpuModel::render_seconds`]), after a warm-up that
//! lets the clock rise, and the clock is reported when `nvidia-smi` can
//! read it.

use std::time::Instant;

use llevant::Camera;
use llevant_gpu::{Gpu, GpuModel};

/// Seconds of back-to-back frames before the first timing of a run: an idle
/// GPU took ~4 s to reach its boost clock.
const FIRST_WARM_UP: f64 = 5.0;
/// Seconds of back-to-back frames before each later timing (after a
/// pipeline or visibility change, during which the GPU idled).
const WARM_UP: f64 = 0.5;

/// Times frames, warming the GPU up first.
pub struct FrameTimer {
    warmed: bool,
    clocks: Vec<u32>,
}

impl FrameTimer {
    pub fn new() -> FrameTimer {
        FrameTimer {
            warmed: false,
            clocks: Vec::new(),
        }
    }

    /// Seconds of each of `frames` frames through `camera`, traced back to
    /// back after a warm-up on the same camera.
    pub fn frames(&mut self, gpu: &Gpu, gm: &GpuModel, camera: &Camera, frames: usize) -> Vec<f64> {
        let warm = if self.warmed { WARM_UP } else { FIRST_WARM_UP };
        self.warmed = true;
        let t = Instant::now();
        // At least one frame: the first after a change pays for setting it up.
        loop {
            gm.render_seconds(gpu, camera).unwrap();
            if t.elapsed().as_secs_f64() >= warm {
                break;
            }
        }
        let times = (0..frames)
            .map(|_| gm.render_seconds(gpu, camera).unwrap())
            .collect();
        // Read while the GPU is still at the clock the frames ran at.
        if let Some(c) = sm_clock_mhz() {
            self.clocks.push(c);
        }
        times
    }

    /// The fastest of `frames` frames, in seconds.
    pub fn best(&mut self, gpu: &Gpu, gm: &GpuModel, camera: &Camera, frames: usize) -> f64 {
        self.frames(gpu, gm, camera, frames)
            .into_iter()
            .fold(f64::INFINITY, f64::min)
    }

    /// The GPU clocks read after each timing, as a line to print (empty
    /// without `nvidia-smi`).
    pub fn clock_report(&self) -> String {
        let (Some(lo), Some(hi)) = (self.clocks.iter().min(), self.clocks.iter().max()) else {
            return String::new();
        };
        let max = sm_clock_max_mhz().map_or(String::new(), |m| format!(" of {m} MHz"));
        format!("GPU clock after the timings: {lo}–{hi} MHz{max}")
    }
}

fn nvidia_smi(field: &str) -> Option<u32> {
    let out = std::process::Command::new("nvidia-smi")
        .args([
            &format!("--query-gpu={field}"),
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    String::from_utf8(out.stdout)
        .ok()?
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

/// The GPU's current SM clock, if `nvidia-smi` is there.
pub fn sm_clock_mhz() -> Option<u32> {
    nvidia_smi("clocks.sm")
}

fn sm_clock_max_mhz() -> Option<u32> {
    nvidia_smi("clocks.max.sm")
}
