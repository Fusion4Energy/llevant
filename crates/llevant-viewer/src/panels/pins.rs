//! The list of pins: notes, where each one is, going to one, removing.

use crate::command::{Command, Edit};
use crate::document::Document;
use crate::{icons, theme};

/// Notes as typed, kept in step with the scene's.
#[derive(Default)]
pub struct PinsPanel {
    notes: Vec<String>,
    /// The scene's notes the text was last made from.
    shown: Vec<String>,
}

impl PinsPanel {
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
        let pins = &doc.scene.pins;
        if pins.is_empty() {
            return;
        }
        let current: Vec<String> = pins.iter().map(|p| p.note.clone()).collect();
        if self.shown != current {
            self.notes = current.clone();
            self.shown = current;
        }
        theme::section(ui, icons::PIN, &format!("Pins ({})", pins.len()), |ui| {
            for (k, (pin, info)) in pins.iter().zip(&doc.pin_info).enumerate() {
                ui.horizontal(|ui| {
                    ui.label(theme::data(format!("{}", k + 1)).color(theme::PIN).strong());
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.notes[k])
                            .hint_text("note")
                            .desired_width(110.0),
                    );
                    if r.lost_focus() && self.notes[k] != pin.note {
                        out.push(Edit::PinNote(k, self.notes[k].clone()).into());
                    }
                    if ui
                        .small_button(icons::GO_TO)
                        .on_hover_text("Select it, slice through it")
                        .clicked()
                    {
                        out.push(Command::Focus(Some(info.clone())));
                        out.push(Command::SliceThrough(pin.point));
                    }
                    if ui
                        .small_button(icons::REMOVE)
                        .on_hover_text("Remove")
                        .clicked()
                    {
                        out.push(Edit::RemovePin(k).into());
                    }
                });
                let p = pin.point;
                ui.label(
                    theme::data(format!(
                        "({:.2}, {:.2}, {:.2}) · {}",
                        p[0],
                        p[1],
                        p[2],
                        info.summary(&doc.model).join(", ")
                    ))
                    .weak(),
                );
            }
            if ui
                .button(icons::with(icons::DELETE, "Remove all"))
                .clicked()
            {
                out.push(Edit::ClearPins.into());
            }
        });
    }
}
