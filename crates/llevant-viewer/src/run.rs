//! Opens the viewer window on a model: what `llevant-viewer` and
//! `python -m llevant view` both do.

use std::path::PathBuf;

use crate::settings::Settings;
use crate::state::BackendChoice;
use crate::{DevOptions, ViewState, ViewerApp};

/// Show `model` in the viewer window and return when it is closed.
///
/// `state_path`: the view to open (JSON, as "Save view" writes it), also
/// where "Save view" writes; by default `<model>.view.json`, read if it
/// exists. `backend`: `auto`, `gpu` or `cpu`. `dev`: hooks for tests and
/// benchmarks. Must run on the main thread on most platforms.
pub fn run(
    model: PathBuf,
    state_path: Option<PathBuf>,
    backend: Option<&str>,
    dev: DevOptions,
) -> Result<(), String> {
    let default_state = {
        let mut p = model.clone().into_os_string();
        p.push(".view.json");
        PathBuf::from(p)
    };
    let mut state = match &state_path {
        Some(p) if p.exists() => ViewState::load(p)?,
        Some(_) => ViewState::new(),
        None if default_state.exists() => ViewState::load(&default_state)?,
        None => ViewState::new(),
    };
    if let Some(b) = backend {
        state.backend = backend_choice(b)?;
    }
    let title = format!(
        "llevant: {}",
        model.file_name().map_or_else(
            || model.display().to_string(),
            |n| n.to_string_lossy().into()
        )
    );
    window(title, move |cc| {
        let mut app = ViewerApp::new(cc, model, state, state_path)?.with_dev(&dev);
        app.use_settings(Settings::load(), &cc.egui_ctx);
        Ok(app)
    })
}

/// The viewer window with no model open: its start screen. `backend`
/// (`auto`, `gpu` or `cpu`) applies to the models opened from it.
pub fn start_screen(backend: Option<&str>, dev: DevOptions) -> Result<(), String> {
    let mut state = ViewState::new();
    if let Some(b) = backend {
        state.backend = backend_choice(b)?;
    }
    window("llevant".into(), move |cc| {
        let mut app = ViewerApp::new(cc, PathBuf::new(), state, None)?.with_dev(&dev);
        app.use_settings(Settings::load(), &cc.egui_ctx);
        Ok(app)
    })
}

/// The `--backend` value `b`.
fn backend_choice(b: &str) -> Result<BackendChoice, String> {
    match b {
        "auto" => Ok(BackendChoice::Auto),
        "gpu" => Ok(BackendChoice::Gpu),
        "cpu" => Ok(BackendChoice::Cpu),
        _ => Err(format!("backend {b}: auto, gpu or cpu")),
    }
}

/// Run the window `make` builds the application of, until it is closed.
fn window(
    title: String,
    make: impl FnOnce(&eframe::CreationContext<'_>) -> Result<ViewerApp, String> + 'static,
) -> Result<(), String> {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: crate::app::wgpu_options(),
        viewport: egui::ViewportBuilder::default()
            .with_title(title.clone())
            .with_inner_size([1400.0, 850.0])
            .with_icon(std::sync::Arc::new(crate::logo::icon())),
        ..Default::default()
    };
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eframe::run_native(
            &title,
            options,
            Box::new(move |cc| {
                let app = make(cc)?;
                Ok(Box::new(app))
            }),
        )
    }));
    match run {
        Ok(r) => r.map_err(|e| e.to_string()),
        // egui panics drawing on a lost device.
        Err(_) if crate::app::WINDOW_LOST.load(std::sync::atomic::Ordering::Relaxed) => {
            Err("the GPU device of the window was lost (a driver reset?); the viewer closed".into())
        }
        Err(p) => std::panic::resume_unwind(p),
    }
}
