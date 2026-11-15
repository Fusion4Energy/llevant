//! What the window shows without a model: the start screen (open a file,
//! the recent ones, drop one on the window) and, while a model loads, how
//! far it is with a Cancel button, under the logo turning
//! ([`crate::logo`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::command::Command;
use crate::{icons, theme};

pub fn screen(ui: &mut egui::Ui, error: Option<&str>, recent: &[PathBuf], out: &mut Vec<Command>) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.12);
        crate::logo::logo(ui, 96.0);
        ui.add_space(8.0);
        ui.label(egui::RichText::new("llevant").size(34.0).strong());
        ui.label(egui::RichText::new("MCNP geometry, exactly").weak());
        ui.add_space(18.0);
        if let Some(e) = error {
            ui.colored_label(ui.visuals().error_fg_color, e);
            ui.add_space(8.0);
        }
        if ui
            .add(
                egui::Button::new(icons::with(icons::OPEN, "Open a model…"))
                    .min_size(egui::vec2(220.0, 32.0)),
            )
            .on_hover_text("Ctrl+O")
            .clicked()
        {
            out.push(Command::OpenDialog);
        }
        ui.label(egui::RichText::new("or drop an MCNP input on the window").weak());
        if recent.is_empty() {
            return;
        }
        ui.add_space(18.0);
        ui.label(egui::RichText::new("Recent").strong());
        ui.add_space(4.0);
        for p in recent {
            let exists = p.exists();
            let name = p
                .file_name()
                .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into());
            let dir = p
                .parent()
                .map_or(String::new(), |d| d.display().to_string());
            let b = egui::Button::new(theme::data(name))
                .right_text(egui::RichText::new(dir).weak())
                .min_size(egui::vec2(420.0, 24.0));
            let r = ui.add_enabled(exists, b);
            let r = if exists {
                r
            } else {
                r.on_disabled_hover_text("not there any more")
            };
            if r.clicked() {
                out.push(Command::OpenFile(p.clone()));
            }
        }
    });
}

/// Loading `path`: the step under way, how far, for how long, Cancel.
pub fn loading(ui: &mut egui::Ui, path: &Path, p: &Arc<llevant::Progress>, started: Instant) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.25);
        crate::logo::spinner(ui, 72.0);
        ui.add_space(14.0);
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        ui.label(egui::RichText::new(format!("Opening {name}")).size(20.0));
        ui.add_space(10.0);
        let stage = match p.stage() {
            "" => "starting",
            s => s,
        };
        theme::progress_bar(ui, p.fraction() as f32, 360.0, stage);
        let t = started.elapsed().as_secs_f64();
        ui.label(egui::RichText::new(format!("{t:.0} s")).weak());
        ui.add_space(10.0);
        if ui
            .add_enabled(!p.cancelled(), egui::Button::new("Cancel"))
            .clicked()
        {
            p.cancel();
        }
        if p.cancelled() {
            ui.label(egui::RichText::new("stopping after this step…").weak());
        }
    });
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(100));
}
