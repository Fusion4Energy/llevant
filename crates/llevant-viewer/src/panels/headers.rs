//! The views' headers: each view's own settings, over it. The 3D view's are
//! its Section (the cuts and the slice plane in 3D) and Display (outlines,
//! ambient occlusion, gaps seen through, orthographic, problems on top)
//! popups and Fit. The slice's are its plane (x, y, z or radial) and where
//! it is, its Display popup (outlines, numbers and the grid), and Fit. Both have a button that maximises the view or restores
//! both.
//!
//! The slice's plane is navigation, moved directly; the rest goes out as
//! commands.

use crate::app::frame::{named, View};
use crate::command::Command;
use crate::document::Document;
use crate::icons;
use crate::panels::view_settings;
use crate::state::{AxisName, Radial};

pub fn view3d(ui: &mut egui::Ui, doc: &mut Document, maximized: bool, out: &mut Vec<Command>) {
    let (back, forward) = (
        doc.camera_history.can_go_back(),
        doc.camera_history.can_go_forward(),
    );
    row(
        ui,
        View::ThreeD,
        maximized,
        out,
        |ui, out| {
            if ui
                .button(icons::FRAME)
                .on_hover_text("Fit: frame the visible model, looking the same way")
                .clicked()
            {
                out.push(Command::FitCamera);
            }
            // (Right to left: forward, then back.)
            let f = ui
                .add_enabled(forward, egui::Button::new(icons::FORWARD))
                .on_hover_text("Forward to the next place looked at (Alt+→)");
            named(&f, "Camera forward");
            if f.clicked() {
                out.push(Command::CameraForward);
            }
            let b = ui
                .add_enabled(back, egui::Button::new(icons::BACK))
                .on_hover_text("Back to the place looked at before (Alt+←)");
            named(&b, "Camera back");
            if b.clicked() {
                out.push(Command::CameraBack);
            }
        },
        |ui, out| {
            ui.label(egui::RichText::new("3D").strong());
            let section = section_label(doc);
            crate::theme::settings_button(ui, icons::with(icons::CLIP, &section), |ui| {
                ui.set_min_width(320.0);
                view_settings::cuts_ui(ui, doc, out);
                ui.checkbox(&mut doc.show_cuts, "show cuts in 3D")
                    .on_hover_text(
                        "Their outlines and handles (off: they still cut; the one selected \
                     here is still shown)",
                    );
                ui.separator();
                ui.checkbox(
                    &mut doc.slice_in_3d,
                    "the slice plane in 3D, with its handles",
                )
                .on_hover_text(
                    "Off, the plane is still shown while its position is dragged or typed \
                     in the slice's header",
                );
                ui.checkbox(&mut doc.cut_at_slice, "cut 3D at the slice")
                    .on_hover_text("Cut away the side of the slice plane facing the eye");
            })
            .on_hover_text("Cut the 3D view open: planes, wedges, a section box; the slice plane");
            // (Orthographic said on the button, as it changes what is seen.)
            let display = if doc.orbit.state.ortho {
                "Display · ortho"
            } else {
                "Display"
            };
            crate::theme::settings_button(ui, icons::with(icons::EFFECTS, display), |ui| {
                view_settings::effects(ui, doc, out);
            })
            .on_hover_text("Outlines, ambient occlusion, orthographic");
        },
    );
}

/// The 3D header's Section button: with the number of cuts, and how many
/// of them cut when some are off.
pub fn section_label(doc: &Document) -> String {
    let n = doc.scene.cuts.len();
    let on = doc.scene.cuts_on().count();
    match (n, on) {
        (0, _) => "Section".to_string(),
        (n, on) if on == n => format!("Section · {n}"),
        (n, on) => format!("Section · {on} of {n}"),
    }
}

pub fn slice(ui: &mut egui::Ui, doc: &mut Document, maximized: bool, out: &mut Vec<Command>) {
    let fit = std::cell::Cell::new(false);
    row(
        ui,
        View::Slice,
        maximized,
        out,
        |ui, _| {
            if ui
                .button(icons::FRAME)
                .on_hover_text("Fit: the whole model in the slice")
                .clicked()
            {
                fit.set(true);
            }
        },
        |ui, _| {
            ui.label(egui::RichText::new("Slice").strong());
            for (a, name) in [(AxisName::X, "x"), (AxisName::Y, "y"), (AxisName::Z, "z")] {
                let on = doc.slice.radial.is_none() && doc.slice.axis == a;
                if ui
                    .selectable_label(on, name)
                    .on_hover_text(format!("The plane {name} = const"))
                    .clicked()
                {
                    doc.slice.radial = None;
                    doc.slice.axis = a;
                    doc.slice.fit();
                }
            }
            let radial = doc.slice.radial.is_some();
            if ui
                .selectable_label(radial, "radial")
                .on_hover_text("A plane through an axis (z), turned to an angle φ")
                .clicked()
                && !radial
            {
                let p = view_settings::default_axis_point(&doc.region);
                doc.slice.radial = Some(Radial::about_z(p, 0.0));
                doc.slice.fit();
            }
            let (lo, hi) = doc.slice.at_range();
            let speed = (hi - lo).abs().max(1e-6) / 1000.0;
            let moved = if let Some(r) = &mut doc.slice.radial {
                let r = ui
                    .add(
                        egui::DragValue::new(&mut r.phi)
                            .range(-180.0..=180.0)
                            .speed(0.25)
                            .prefix("φ ")
                            .suffix("°"),
                    )
                    .on_hover_text("The plane's angle about its axis (PageUp, PageDown: a degree)");
                crate::theme::name(&r, "slice angle");
                crate::theme::settings_button(ui, icons::OPENED, |ui| {
                    view_settings::radial_axis(ui, doc)
                })
                .on_hover_text("The axis of the radial plane");
                r
            } else {
                let axis = match doc.slice.axis {
                    AxisName::X => "x",
                    AxisName::Y => "y",
                    AxisName::Z => "z",
                };
                let r = ui
                    .add(
                        egui::DragValue::new(&mut doc.slice.at)
                            .range(lo..=hi)
                            .speed(speed)
                            .prefix(format!("{axis} = ")),
                    )
                    .on_hover_text(
                        "Where the plane is: drag, or click and type (PageUp, PageDown step it)",
                    );
                crate::theme::name(&r, "slice position");
                r
            };
            // Shown in 3D while it moves (however the 3D view is set).
            let moving = moved.dragged() || moved.has_focus();
            if moving != doc.editing.slice_moving {
                doc.editing.slice_moving = moving;
                ui.ctx().request_repaint();
            }
            crate::theme::settings_button(ui, icons::with(icons::EFFECTS, "Display"), |ui| {
                ui.checkbox(&mut doc.slice.outline, "outlines")
                    .on_hover_text("Lines along boundaries: between cells, materials or universes");
                ui.add_enabled_ui(doc.slice.outline, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("between").weak());
                        let options = crate::state::OutlineBy::ALL.map(|o| (o, o.name()));
                        crate::theme::choice(ui, &mut doc.slice.outline_by, &options);
                    })
                    .response
                    .on_hover_text(
                        "Cells (every instance), materials (void is one: what explains a \
                         field) or universes (where fills begin), of the deepest cells; at \
                         a level shown, cells",
                    );
                    let own = llevant_render::colours::OUTLINE;
                    if let Some(c) =
                        view_settings::outline_colour(ui, doc.slice.outline_colour, own)
                    {
                        doc.slice.outline_colour = c;
                    }
                });
                ui.checkbox(&mut doc.slice.labels_wanted.cells, "cell numbers")
                    .on_hover_text("Cell numbers in the cells, where they fit");
                ui.checkbox(&mut doc.slice.labels_wanted.surfaces, "surface numbers")
                    .on_hover_text("Surface numbers along the boundaries, where they fit");
                ui.checkbox(&mut doc.slice.grid, "grid")
                    .on_hover_text("Lines of the plane's coordinates, numbered along the edges");
            })
            .on_hover_text("Outlines, cell and surface numbers, grid");
        },
    );
    if fit.get() {
        doc.slice.fit();
    }
}

/// A header: `right` (Fit) and the maximise button on the right, `left`
/// in the width that is left, clipped to it.
fn row(
    ui: &mut egui::Ui,
    v: View,
    maximized: bool,
    out: &mut Vec<Command>,
    right: impl FnOnce(&mut egui::Ui, &mut Vec<Command>),
    left: impl FnOnce(&mut egui::Ui, &mut Vec<Command>),
) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (icon, name) = match (maximized, v) {
                (true, _) => (icons::RESTORE, "Show both views"),
                (false, View::ThreeD) => (icons::MAXIMIZE, "Maximise the 3D view"),
                (false, View::Slice) => (icons::MAXIMIZE, "Maximise the slice"),
            };
            let b = ui.button(icon).on_hover_text(name);
            named(&b, name);
            if b.clicked() {
                out.push(Command::Maximize((!maximized).then_some(v)));
            }
            right(ui, out);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                left(ui, out);
            });
        });
    });
}
