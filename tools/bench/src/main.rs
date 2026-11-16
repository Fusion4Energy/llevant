//! `llevant-bench <command> <model> ...`: llevant's benchmarks, and its
//! checks against brute force and of the GPU against the CPU, on any model
//! (the zoo's, the synthetic ones, the reference model).
//!
//! Models load through the disk cache (`llevant_scene::store`, as the
//! frontends do: `LLEVANT_CACHE=off` compiles every time). Switches read
//! from the environment:
//!
//! - `LLEVANT_GRID=off|auto|always`: octrees (A/B);
//! - `LLEVANT_RESIDUALS=off`: octrees without residual programs (A/B);
//! - `LLEVANT_UNIVERSE=U` (`verify`, `trace`, `gpu-check`): that universe
//!   opened on its own;
//! - `LLEVANT_BENCH_SET=<dir>`: save the workloads there, and replay them
//!   in later runs (A/B across builds);
//! - `LLEVANT_PROFILE=1` (Unix): pprof samples of the timed section, the
//!   top functions printed.

/// The system heap on Windows does not scale to large parallel batches
/// (docs/decisions/0009-mimalloc.md).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod profile;
mod rng;
mod workload;

mod compare_trace;
mod drag;
mod gpu_ab;
mod gpu_check;
mod gpu_info;
mod gpu_leaves;
mod gpu_levels;
mod gpu_probe;
mod gpu_ray;
mod gpu_slices;
mod gpu_time;
mod gpu_view;
mod points;
mod report;
mod trace;
mod verify;
mod view;
mod work;

use std::path::Path;

use llevant::{GridMode, Model, Options};

/// A command: its name, what runs it, its arguments and what it does.
type Command = (&'static str, fn(Vec<String>), &'static str);

const COMMANDS: &[Command] = &[
    (
        "points",
        points::run,
        "<model> [points]: classify random points; compile timings, box quality",
    ),
    (
        "trace",
        trace::run,
        "<model> [lines] [length_cm]: trace random lines, checked against points and brute force",
    ),
    (
        "view",
        view::run,
        "<model> [pixels] [out_dir]: slices and first-hit views, timed",
    ),
    (
        "verify",
        verify::run,
        "<model> [points]: classification against brute force",
    ),
    (
        "report",
        report::run,
        "<model...> [--json f] [--markdown f] [--no-gpu]: the numbers of docs/benchmarks.md",
    ),
    (
        "work",
        work::run,
        "<dir|model...> [--write file] [--check file]: work counters of a fixed workload, as JSON",
    ),
    (
        "compare-trace",
        compare_trace::run,
        "<model> <lines.f64>: tracing with octrees against without, every line",
    ),
    (
        "gpu-check",
        gpu_check::run,
        "<model> [pixels]: GPU first hits against the CPU, ray by ray, 9 views",
    ),
    (
        "gpu-view",
        gpu_view::run,
        "<model> [width height] [out_dir]: GPU frames, timed",
    ),
    (
        "gpu-ray",
        gpu_ray::run,
        "<model> <pixels> <view> <ray> [visibility] [shape]: one ray, CPU segments and GPU hit",
    ),
    (
        "gpu-levels",
        gpu_levels::run,
        "<model> [width height]: GPU frames at levels, with ghosts, a filler alone",
    ),
    (
        "gpu-slices",
        gpu_slices::run,
        "<model> <solid|query> <size> <words,...>: frames traced in slices against in one go",
    ),
    (
        "gpu-ab",
        gpu_ab::run,
        "<model> [width height] [rounds] [visibility]: A/B of shader variants",
    ),
    (
        "gpu-probe",
        gpu_probe::run,
        "<model> <size> <steps,...> [visibility]: GPU time and work per ray against the step cap",
    ),
    (
        "gpu-leaves",
        gpu_leaves::run,
        "<model> x y z: a cell's surface sides at a point, f64 and the shader's f32",
    ),
    (
        "drag",
        drag::run,
        "<model> [surface...]: live drags of a surface timed: steps, slices, GPU patches and frames",
    ),
    (
        "gpu-info",
        gpu_info::run,
        ": the adapter llevant-gpu would use, and its limits",
    ),
];

fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    match COMMANDS.iter().find(|c| c.0 == cmd) {
        Some((_, run, _)) => {
            run(args.collect());
            std::process::ExitCode::SUCCESS
        }
        None => {
            eprintln!("usage: llevant-bench <command> ...\n");
            for (name, _, what) in COMMANDS {
                eprintln!("  {name} {what}");
            }
            std::process::ExitCode::from(2)
        }
    }
}

/// The compile options the environment asks for (`LLEVANT_GRID`,
/// `LLEVANT_RESIDUALS`).
pub fn options() -> Options {
    let grid = match std::env::var("LLEVANT_GRID").as_deref() {
        Ok("off") => GridMode::Off,
        Ok("always") => GridMode::Always,
        _ => GridMode::Auto,
    };
    let residuals = std::env::var("LLEVANT_RESIDUALS").as_deref() != Ok("off");
    Options::default().with_grid(grid).with_residuals(residuals)
}

/// The model at `path`, through the disk cache.
pub fn load(path: impl AsRef<Path>, opts: &Options) -> llevant::Result<Model> {
    llevant_scene::store::Store::from_env().load(path.as_ref(), opts, None)
}
