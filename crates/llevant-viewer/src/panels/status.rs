//! The status bar, in plain words: the point and cell under the cursor, the
//! selection, jobs running, the search structures being built and the
//! message of the moment (news for a few seconds, work while it runs, a
//! warning until dismissed: [`crate::messages`]), with the log of the
//! last ones behind a button; then the active tool's hint. On the right,
//! the 3D backend: a chip whose popup has the frame rate, how far tracing
//! is, its measured costs, and the tracing settings.

use crate::command::Command;
use crate::document::Document;
use crate::messages::Level;
use crate::panels::view_settings;
use crate::state::BackendChoice;
use crate::theme;
use crate::view3d::budget::Cost;

#[allow(clippy::too_many_arguments)]
pub fn ui(
    ui: &mut egui::Ui,
    fps: f64,
    doc: Option<&Document>,
    hint: &str,
    backend: BackendChoice,
    budget_ms: f64,
    busy: Busy,
    out: &mut Vec<Command>,
) {
    ui.horizontal(|ui| {
        let Some(d) = doc else {
            return;
        };
        if let Some(h) = &d.pointer.hover {
            let p = h.point;
            ui.label(theme::data(format!(
                "x {:.2}  y {:.2}  z {:.2}",
                p[0], p[1], p[2]
            )));
            ui.label(theme::data(h.summary(&d.model).join(" · ")));
        } else if let Some(p) = d.pointer.hovered {
            ui.label(theme::data(format!(
                "x {:.2}  y {:.2}  z {:.2}",
                p[0], p[1], p[2]
            )));
        }
        if let Some(s) = d.selection.summary() {
            ui.separator();
            ui.label(theme::data(format!("selected: {s}")).color(theme::SELECTION));
        }
        for (k, j) in d.results.jobs.iter().enumerate() {
            ui.separator();
            let t = j.started().elapsed().as_secs_f64();
            match j.fraction() {
                Some(f) => {
                    theme::progress_bar(ui, f as f32, 80.0, &format!("{:.0} %", 100.0 * f))
                        .on_hover_text(&j.title);
                }
                None => {
                    crate::logo::small_spinner(ui).on_hover_text(&j.title);
                }
            }
            ui.label(theme::data(format!("{} {t:.0} s", j.kind.name())));
            if ui
                .small_button(crate::icons::REMOVE)
                .on_hover_text("Cancel")
                .clicked()
            {
                out.push(Command::CancelJob(k));
            }
        }
        if let Some(t) = busy.building {
            ui.separator();
            crate::logo::small_spinner(ui);
            ui.label(theme::data(format!("search structures {t:.0} s")))
                .on_hover_text(
                    "The model is shown before its search structures are built: views and \
                     analyses are slower until then",
                );
        }
        messages(ui, d, busy.working, out);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let s = d.view3d.stats();
            let busy = s.refined < 1.0 || d.slice.progress() < 1.0;
            let chip = format!("{}{}", s.backend, if busy { " …" } else { "" });
            crate::theme::settings_button(ui, theme::data(chip), |ui| {
                ui.set_min_width(300.0);
                ui.label(theme::data(format!("{fps:.0} fps")));
                ui.label(theme::data(format!(
                    "3D {} · scale {:.0} % · refined {:.0} %",
                    s.backend,
                    100.0 * s.scale,
                    100.0 * s.refined
                )));
                let cost = |c: Option<Cost>| {
                    c.map_or("-".to_string(), |c| {
                        format!("{:.0} ms + {:.0} ms/Mray", 1e3 * c.fixed, 1e9 * c.per_ray)
                    })
                };
                ui.label(theme::data(format!(
                    "preview {} · refine {}",
                    cost(s.preview),
                    cost(s.refine)
                )))
                .on_hover_text(
                    "Fitted cost of a batch of rays: what the scheduler sizes batches by",
                );
                ui.label(theme::data(format!(
                    "slice {:.0} %",
                    100.0 * d.slice.progress()
                )));
                ui.separator();
                view_settings::tracing(ui, backend, budget_ms, out);
            })
            .on_hover_text("The 3D backend: frame rate, tracing and its settings");
            ui.add(egui::Label::new(egui::RichText::new(hint).weak()).truncate());
        });
    });
}

/// What the application is doing besides the document's jobs.
#[derive(Clone, Copy, Debug, Default)]
pub struct Busy {
    /// Loading, editing, saving, opening a universe or reading a field:
    /// what a message of work under way is about.
    pub working: bool,
    /// Seconds the search structures of a model shown have been building.
    pub building: Option<f64>,
}

/// The message of the moment, and the log of the last ones.
fn messages(ui: &mut egui::Ui, d: &Document, working: bool, out: &mut Vec<Command>) {
    let log = &d.messages;
    if log.is_empty() {
        return;
    }
    let now = std::time::Instant::now();
    ui.separator();
    theme::settings_button(ui, crate::icons::HISTORY, |ui| {
        ui.set_min_width(360.0);
        ui.set_max_width(640.0);
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                for m in log.iter().rev() {
                    ui.horizontal_wrapped(|ui| {
                        let ago = now.saturating_duration_since(m.at).as_secs();
                        ui.label(theme::data(ago_text(ago)).weak());
                        let text = egui::RichText::new(&m.text);
                        ui.label(match m.level {
                            Level::Warning => text.color(ui.visuals().warn_fg_color),
                            Level::Info | Level::Progress => text,
                        });
                    });
                }
            });
    })
    .on_hover_text("The last messages");
    if let Some(m) = log.current(now, working) {
        match m.level {
            Level::Info => {
                ui.label(&m.text);
            }
            Level::Progress => {
                crate::logo::small_spinner(ui);
                ui.label(&m.text);
            }
            Level::Warning => {
                ui.label(egui::RichText::new(&m.text).color(ui.visuals().warn_fg_color));
                if ui
                    .small_button(crate::icons::REMOVE)
                    .on_hover_text("Dismiss (it stays in the log)")
                    .clicked()
                {
                    out.push(Command::DismissMessage);
                }
            }
        }
    }
    if let Some(left) = log.expires_in(now) {
        ui.ctx().request_repaint_after(left);
    }
}

/// How long ago, shortly.
fn ago_text(s: u64) -> String {
    match s {
        0..60 => format!("{s:>2} s"),
        60..3600 => format!("{:>2} min", s / 60),
        _ => format!("{:>2} h", s / 3600),
    }
}
