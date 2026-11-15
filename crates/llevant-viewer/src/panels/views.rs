//! Views saved by name: the camera, the slice and the scene, kept in the
//! view file. A click goes back to one (its scene as an undoable edit, the
//! pins as they are now).

use crate::command::Command;
use crate::document::Document;
use crate::{icons, theme};

#[derive(Default)]
pub struct ViewsPanel {
    /// The name of the next view saved.
    name: String,
    /// Names as being typed, by row.
    names: Vec<String>,
}

impl ViewsPanel {
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.name)
                    .hint_text(format!("view {}", doc.views.len() + 1))
                    .desired_width(140.0),
            );
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui
                .button(icons::with(icons::SAVE, "Save this view"))
                .clicked()
                || enter
            {
                let name = match self.name.trim() {
                    "" => format!("view {}", doc.views.len() + 1),
                    n => n.to_string(),
                };
                out.push(Command::SaveNamedView(name));
                self.name.clear();
            }
        });
        if doc.views.is_empty() {
            crate::app::frame::hint(
                ui,
                "Saved views come back with a click: camera, slice, colours, what is hidden.",
            );
            return;
        }
        self.names.resize(doc.views.len(), String::new());
        ui.add_space(4.0);
        for (k, v) in doc.views.iter().enumerate() {
            if self.names[k].is_empty() {
                self.names[k] = v.name.clone();
            }
            ui.horizontal(|ui| {
                if ui
                    .button(icons::GO_TO)
                    .on_hover_text("Go back to this view")
                    .clicked()
                {
                    out.push(Command::GoToView(k));
                }
                let r = ui.add(egui::TextEdit::singleline(&mut self.names[k]).desired_width(140.0));
                if r.lost_focus() && self.names[k] != v.name && !self.names[k].trim().is_empty() {
                    out.push(Command::RenameView(k, self.names[k].trim().to_string()));
                }
                let c = v.camera.target;
                ui.label(theme::data(format!("({:.0}, {:.0}, {:.0})", c[0], c[1], c[2])).weak());
                if ui
                    .small_button(icons::REMOVE)
                    .on_hover_text("Forget it")
                    .clicked()
                {
                    out.push(Command::DeleteView(k));
                    self.names.clear();
                }
            });
        }
    }
}
