//! `llevant-viewer [model] [--state view.json]`: the interactive viewer
//! (without a model, its start screen).

/// The system heap on Windows does not scale to large parallel batches
/// (docs/decisions/0009-mimalloc.md).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::path::PathBuf;

use clap::Parser;

/// Interactive 3D views and 2D slices of an MCNP model.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// MCNP input file (none: the start screen, to open one from).
    model: Option<PathBuf>,
    /// View to open (JSON, as "Save view" writes it); also where "Save
    /// view" writes. Default: `<model>.view.json`, read if it exists.
    #[arg(long)]
    state: Option<PathBuf>,
    /// Who traces the 3D view: auto, gpu or cpu.
    #[arg(long)]
    backend: Option<String>,
    /// Save the window as this PNG once everything is traced, then quit.
    #[arg(long, hide = true)]
    screenshot: Option<PathBuf>,
    /// First turn the camera this many seconds, printing frame times.
    #[arg(long, hide = true)]
    orbit: Option<f64>,
    /// Run the edit storm (panels, hide/show, isolate, levels, colouring,
    /// palette typing), printing the UI thread's own work per step; quit.
    #[arg(long, hide = true)]
    storm: bool,
    /// Show the last 3D image where it was traced while the camera moves,
    /// instead of moving the hits with it (A/B).
    #[arg(long, hide = true)]
    no_reproject: bool,
}

fn main() -> Result<(), String> {
    // `RUST_LOG=llevant=debug`: the engine's debug log (volume rounds).
    let _ = env_logger::try_init();
    let args = Args::parse();
    let dev = llevant_viewer::DevOptions {
        screenshot: args.screenshot,
        orbit_seconds: args.orbit,
        storm: args.storm,
        no_reprojection: args.no_reproject,
        ..Default::default()
    };
    match args.model {
        Some(m) => llevant_viewer::run(m, args.state, args.backend.as_deref(), dev),
        None => llevant_viewer::run::start_screen(dev),
    }
}
