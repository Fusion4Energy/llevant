//! The panels around the views. Each reads the document and sends
//! [`Command`]s for what it changes in the scene;
//! panel-only state (text as typed, the clip plane's axis) lives in the
//! panel's own struct. Built from the house widgets of [`crate::theme`].

pub mod fields;
pub mod find;
pub mod headers;
pub mod inspector;
pub mod keys;
pub mod materials;
pub mod mesh;
pub mod palette;
pub mod pins;
pub mod problems;
pub mod profile;
pub mod region;
pub mod ruler;
pub mod stats;
pub mod status;
pub mod toolbar;
pub mod view_settings;
pub mod views;
pub mod volumes;

use crate::command::{Command, Edit};
use crate::icons;
use crate::seen::Eye;
use llevant_scene::hiding::ProblemScope;

/// An eye of a panel's row, as [`Eye`] says: none (nothing to show), open (all shown), faded open
/// (some), closed (hidden; clicking shows it), faded closed (hidden by
/// something else, said on hover; clicking shows it when it can, else it
/// is disabled). Clicking an open eye pushes `hide`. `what` names the
/// row's cells in the tooltips.
pub fn eye(ui: &mut egui::Ui, e: &Eye, hide: Command, what: &str, out: &mut Vec<Command>) {
    let p = crate::theme::palette(ui.ctx());
    let faded = p.text_weak.gamma_multiply(0.45);
    let (icon, colour, tip, click): (_, _, String, Option<Command>) = match e {
        Eye::Empty => {
            // The room of an eye, so that the row lines up with the others.
            let none = egui::RichText::new(icons::SHOW).color(egui::Color32::TRANSPARENT);
            ui.add_enabled(false, egui::Button::new(none).frame(false).small());
            return;
        }
        Eye::Open => (
            icons::SHOW,
            p.text_weak,
            format!("Shown: click to hide {what}"),
            Some(hide),
        ),
        Eye::Partly(seen, cells) => (
            icons::SHOW,
            faded,
            format!("{seen} of {cells} cells shown: click to hide {what}"),
            Some(hide),
        ),
        Eye::Closed(show) => (
            icons::HIDE,
            p.text,
            format!("Hidden: click to show {what}"),
            Some(show.clone().into()),
        ),
        Eye::Away(why, show) => (
            icons::HIDE,
            faded,
            why.clone(),
            show.clone().map(Command::from),
        ),
    };
    let b = egui::Button::new(egui::RichText::new(icon).color(colour))
        .frame(false)
        .small();
    let r = ui.add_enabled(click.is_some(), b);
    let r = if click.is_some() {
        r.on_hover_text(tip)
    } else {
        r.on_disabled_hover_text(tip)
    };
    if r.clicked() {
        out.extend(click);
    }
}

/// Characters of a rule shown on hover at most.
const RULE_HOVER: usize = 2000;

/// While isolating: a line saying what is isolated, with a button to stop
/// (the reason the eyes below are closed). The button comes first, so a
/// long rule is cut to the room left (whole on hover), not the button.
pub fn isolating(ui: &mut egui::Ui, doc: &crate::document::Document, out: &mut Vec<Command>) {
    let Some(q) = &doc.scene.hiding.isolate else {
        return;
    };
    let p = crate::theme::palette(ui.ctx());
    egui::Frame::new()
        .fill(p.accent.gamma_multiply(0.15))
        .stroke(egui::Stroke::new(1.0, p.accent.gamma_multiply(0.6)))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(icons::ISOLATE).color(p.icon));
                ui.label("Isolating");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button(icons::with(icons::REMOVE, "Stop"))
                        .on_hover_text("Stop isolating: show the rest again")
                        .clicked()
                    {
                        out.push(Edit::Isolate(None).into());
                    }
                    let mut all = doc.scene.hiding.problems == ProblemScope::All;
                    if ui
                        .toggle_value(&mut all, "All problems")
                        .on_hover_text(
                            "Draw every gap and overlap met, not only those of the cells \
                             isolated (an overlap between other cells, or between the \
                             envelopes holding them, would show as specks)",
                        )
                        .changed()
                    {
                        let p = if all {
                            ProblemScope::All
                        } else {
                            ProblemScope::Selection
                        };
                        out.push(Edit::IsolatedProblems(p).into());
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        let text = q.to_string();
                        let hover = if text.chars().count() > RULE_HOVER {
                            let cut: String = text.chars().take(RULE_HOVER).collect();
                            format!("{cut}…")
                        } else {
                            text.clone()
                        };
                        ui.add(egui::Label::new(crate::theme::data(text)).truncate())
                            .on_hover_text(hover);
                    });
                });
            });
        });
}
