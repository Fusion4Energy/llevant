//! `view`: the interactive viewer (with the `viewer` feature).

use std::path::PathBuf;

use crate::args::Open;

#[derive(clap::Args)]
pub(crate) struct ViewArgs {
    /// The MCNP input file (none: the start screen, to open one from).
    model: Option<PathBuf>,
    /// View file to open and save to (default `<model>.view.json`).
    #[arg(long)]
    state: Option<PathBuf>,
    /// Who traces the 3D view: auto, gpu or cpu.
    #[arg(long)]
    backend: Option<String>,
}

impl ViewArgs {
    pub(crate) fn run(self, _open: &Open) -> Result<u8, String> {
        let ViewArgs {
            model,
            state,
            backend,
        } = self;
        view(model, state, backend)?;
        Ok(0)
    }
}

#[cfg(feature = "viewer")]
fn view(
    model: Option<PathBuf>,
    state: Option<PathBuf>,
    backend: Option<String>,
) -> Result<(), String> {
    match (model, state) {
        (Some(m), state) => llevant_viewer::run(m, state, backend.as_deref(), Default::default()),
        (None, Some(_)) => Err("--state needs a model".into()),
        (None, None) => llevant_viewer::run::start_screen(backend.as_deref(), Default::default()),
    }
}

#[cfg(not(feature = "viewer"))]
fn view(_: Option<PathBuf>, _: Option<PathBuf>, _: Option<String>) -> Result<(), String> {
    Err("this build has no viewer (feature `viewer`; or run `llevant-viewer`)".into())
}
