//! The Ruler window: how the Ruler tool lays the line, its length, every
//! cell it crosses with its material and length, Copy (CSV) and Clear.
//! Open while the tool is active or a ruler is laid.

use crate::command::Command;
use crate::context::{cell_link, material_link};
use crate::document::Document;
use crate::ruler::{status_name, RulerMode};
use crate::{icons, theme};

pub fn window(ctx: &egui::Context, doc: &Document, out: &mut Vec<Command>) {
    egui::Window::new(icons::with(icons::RULER, "Ruler"))
        .id(egui::Id::new("ruler window"))
        .default_width(300.0)
        // Bottom right of the views, clear of the inspector and the
        // status bar (its height is about 330 points at most).
        .default_pos(ctx.content_rect().right_bottom() + egui::vec2(-660.0, -370.0))
        .resizable(true)
        .collapsible(true)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let mut mode = doc.ruler_mode;
                ui.selectable_value(&mut mode, RulerMode::TwoPoints, "Two points")
                    .on_hover_text("Click one point, then the other, or drag between them");
                ui.selectable_value(&mut mode, RulerMode::Layers, "Through the layers")
                    .on_hover_text(
                        "Click a face (or near a boundary in the slice): along the surface's \
                         normal, through what is behind it, up to the next void",
                    );
                if mode != doc.ruler_mode {
                    out.push(Command::RulerMode(mode));
                }
            });
            let Some(r) = &doc.ruler else {
                ui.label(
                    egui::RichText::new("Nothing measured yet: use the Ruler tool (M).").weak(),
                );
                return;
            };
            let m = &doc.model;
            ui.horizontal(|ui| {
                ui.label(theme::data(format!("{:.3} cm", r.length())).strong());
                ui.label(
                    egui::RichText::new(format!(
                        "from ({:.2}, {:.2}, {:.2})",
                        r.a[0], r.a[1], r.a[2]
                    ))
                    .weak(),
                );
            });
            ui.horizontal(|ui| {
                if ui
                    .small_button(icons::with(icons::COPY, "Copy (CSV)"))
                    .clicked()
                {
                    ui.ctx().copy_text(r.csv(m));
                }
                if ui
                    .small_button(icons::with(icons::REMOVE, "Clear"))
                    .clicked()
                {
                    out.push(Command::SetRuler(None));
                }
            });
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .show(ui, |ui| {
                    egui::Grid::new("ruler cells")
                        .num_columns(4)
                        .striped(true)
                        .spacing([10.0, 2.0])
                        .show(ui, |ui| {
                            for h in ["cell", "material", "from", "length"] {
                                ui.label(egui::RichText::new(h).weak());
                            }
                            ui.end_row();
                            for s in &r.stretches {
                                match s.cell {
                                    Some(c) => {
                                        cell_link(ui, doc, c, out);
                                        let cell = m.cell(c);
                                        if cell.fill.is_some() {
                                            ui.label(egui::RichText::new("filled").weak());
                                        } else {
                                            material_link(ui, doc, cell.material, out);
                                        }
                                    }
                                    None => {
                                        ui.colored_label(
                                            ui.visuals().warn_fg_color,
                                            status_name(s.status),
                                        );
                                        ui.label("");
                                    }
                                }
                                ui.label(theme::data(format!("{:.2}", s.from)).weak());
                                ui.label(theme::data(format!("{:.3} cm", s.length())));
                                ui.end_row();
                            }
                        });
                });
        });
}
