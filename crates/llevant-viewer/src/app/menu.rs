//! The menu bar: File, Edit, View (workspaces, panels, sides, views),
//! Select, Help. Every item sends the same
//! commands as its key or button elsewhere, and shows its key. The rest of
//! the bar (the workspaces' tabs, search, undo and redo) is the caller's.

use super::frame::{Frame, Panel, View, Workspace};
use crate::command::{Command, Edit};
use crate::context;
use crate::document::Document;
use crate::icons;
use crate::state::Colouring;

/// An item: clicked, it sends `c` and closes the menu.
fn item(ui: &mut egui::Ui, text: String, keys: &str, c: Option<Command>, out: &mut Vec<Command>) {
    let b = egui::Button::new(text).shortcut_text(keys);
    if ui.add_enabled(c.is_some(), b).clicked() {
        out.extend(c);
        ui.close();
    }
}

pub fn ui(
    ui: &mut egui::Ui,
    doc: Option<&Document>,
    frame: &Frame,
    recent: &[std::path::PathBuf],
    out: &mut Vec<Command>,
    rest: impl FnOnce(&mut egui::Ui, &mut Vec<Command>),
) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            item(
                ui,
                icons::with(icons::OPEN, "Open…"),
                "Ctrl+O",
                Some(Command::OpenDialog),
                out,
            );
            ui.menu_button(icons::with(icons::OPEN, "Open recent"), |ui| {
                if recent.is_empty() {
                    ui.label(egui::RichText::new("none yet").weak());
                }
                for p in recent {
                    let name = p
                        .file_name()
                        .map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into());
                    let b = egui::Button::new(name).shortcut_text(
                        p.parent()
                            .map_or(String::new(), |d| d.display().to_string()),
                    );
                    if ui.add_enabled(p.exists(), b).clicked() {
                        out.push(Command::OpenFile(p.clone()));
                        ui.close();
                    }
                }
            });
            item(
                ui,
                icons::with(icons::FORWARD, "Reload"),
                "Ctrl+R",
                doc.map(|_| Command::Reload),
                out,
            );
            item(
                ui,
                icons::with(icons::FIELD, "Open field…"),
                "",
                doc.map(|_| Command::OpenFieldDialog),
                out,
            );
            ui.separator();
            let some = doc.is_some();
            item(
                ui,
                icons::with(icons::SAVE, "Save view"),
                "Ctrl+S",
                some.then_some(Command::SaveView),
                out,
            );
            let back = doc.is_some_and(|d| d.universe.is_some());
            item(
                ui,
                icons::with(icons::BACK, "Back to the model"),
                "",
                back.then_some(Command::Back),
                out,
            );
            ui.separator();
            let some = doc.is_some();
            item(
                ui,
                icons::with(icons::SAVE, "Export the 3D view (PNG)"),
                "",
                some.then_some(Command::Export3d),
                out,
            );
            item(
                ui,
                icons::with(icons::SAVE, "Export the slice (PNG)"),
                "",
                some.then_some(Command::ExportSlice),
                out,
            );
            item(
                ui,
                icons::with(icons::COPY, "Copy the 3D view as a command"),
                "",
                some.then_some(Command::CopyCli),
                out,
            );
            ui.separator();
            if ui.button("Quit").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
        ui.menu_button("Edit", |ui| {
            let undo = doc.and_then(|d| d.history.next_undo());
            let redo = doc.and_then(|d| d.history.next_redo());
            let name = |what: &str, w: Option<&str>| match w {
                Some(w) => format!("{what} {w}"),
                None => what.to_string(),
            };
            item(
                ui,
                icons::with(icons::UNDO, &name("Undo", undo)),
                "Ctrl+Z",
                undo.map(|_| Command::Undo),
                out,
            );
            item(
                ui,
                icons::with(icons::REDO, &name("Redo", redo)),
                "Ctrl+Shift+Z",
                redo.map(|_| Command::Redo),
                out,
            );
            ui.separator();
            let editable = doc.is_some_and(|d| d.input_editable.is_ok());
            for (kind, name) in [
                (llevant_scene::source::CardKind::Cell, "New cell"),
                (llevant_scene::source::CardKind::Surface, "New surface"),
                (llevant_scene::source::CardKind::Material, "New material"),
            ] {
                item(
                    ui,
                    name.to_string(),
                    "",
                    editable.then_some(Command::Inspect(crate::panels::inspector::Subject::New(
                        kind,
                    ))),
                    out,
                );
            }
            ui.separator();
            item(
                ui,
                icons::with(icons::SEARCH, "Find a command, a cell, a point…"),
                "Ctrl+K",
                Some(Command::Palette),
                out,
            );
        });
        ui.menu_button("View", |ui| {
            for w in Workspace::ALL {
                let b =
                    egui::Button::selectable(frame.workspace == w, icons::with(w.icon(), w.name()))
                        .shortcut_text(format!("Ctrl+{}", w.number()));
                if ui.add(b).clicked() {
                    out.push(Command::Workspace(w));
                    ui.close();
                }
            }
            ui.separator();
            ui.menu_button(icons::with(icons::INFO, "Panels"), |ui| {
                for p in Panel::ALL {
                    let b = egui::Button::selectable(frame.shows(p), p.title());
                    if ui.add(b).clicked() {
                        out.push(Command::ShowPanel(p));
                        ui.close();
                    }
                }
            });
            let a = frame;
            item(
                ui,
                icons::with(
                    icons::FOLD_LEFT,
                    if a.left.open {
                        "Fold the left side"
                    } else {
                        "Unfold the left side"
                    },
                ),
                "Ctrl+[",
                Some(Command::FoldLeft),
                out,
            );
            item(
                ui,
                icons::with(
                    icons::FOLD_RIGHT,
                    if a.right.open {
                        "Fold the inspector"
                    } else {
                        "Unfold the inspector"
                    },
                ),
                "Ctrl+]",
                Some(Command::FoldRight),
                out,
            );
            ui.separator();
            for (m, label) in [
                (None, icons::with(icons::BOTH_VIEWS, "Both views")),
                (
                    Some(View::ThreeD),
                    icons::with(icons::VIEW_3D, "3D view only"),
                ),
                (Some(View::Slice), icons::with(icons::SLICE, "Slice only")),
            ] {
                let b = egui::Button::selectable(frame.maximized == m, label);
                if ui.add(b).clicked() {
                    out.push(Command::Maximize(m));
                    ui.close();
                }
            }
            ui.separator();
            ui.menu_button(icons::with(icons::COLOUR, "Colour by"), |ui| {
                for (c, name) in [
                    (Colouring::Cell, "cell"),
                    (Colouring::Material, "material"),
                    (Colouring::Universe, "universe"),
                    (Colouring::Realistic, "realistic"),
                    (Colouring::Density, "density"),
                ] {
                    let on = doc.is_some_and(|d| d.scene.colour_by == c);
                    if ui.radio(on, name).clicked() {
                        out.push(Edit::Colouring(c).into());
                        ui.close();
                    }
                }
            });
            item(
                ui,
                icons::with(icons::CAMERA, "Fit the 3D view"),
                "",
                doc.map(|_| Command::FitCamera),
                out,
            );
        });
        ui.menu_button("Select", |ui| match doc {
            Some(d) => {
                item(
                    ui,
                    icons::with(icons::SEARCH, "By rule…"),
                    "Ctrl+K",
                    Some(Command::Palette),
                    out,
                );
                item(
                    ui,
                    icons::with(icons::SELECT, "All"),
                    "",
                    Some(Command::Select(
                        llevant_scene::query::Query::All,
                        crate::selection::SelectMode::Replace,
                    )),
                    out,
                );
                let back = d.selection.can_go_back().then_some(Command::SelectionBack);
                let fwd = d
                    .selection
                    .can_go_forward()
                    .then_some(Command::SelectionForward);
                item(
                    ui,
                    icons::with(icons::BACK, "Previous selection"),
                    "Shift+Alt+←",
                    back,
                    out,
                );
                item(
                    ui,
                    icons::with(icons::FORWARD, "Next selection"),
                    "Shift+Alt+→",
                    fwd,
                    out,
                );
                ui.separator();
                context::selection_items(ui, d, out);
            }
            None => {
                ui.label("no model open");
            }
        });
        ui.menu_button("Help", |ui| {
            item(
                ui,
                icons::with(icons::KEYS, "Keys and mouse"),
                "F1",
                Some(Command::ShowKeys),
                out,
            );
            ui.separator();
            ui.label(
                egui::RichText::new(format!("llevant viewer {}", env!("CARGO_PKG_VERSION"))).weak(),
            );
        });
        rest(ui, out);
    });
}
