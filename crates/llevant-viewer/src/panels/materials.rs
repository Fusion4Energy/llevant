//! The materials of the model: a legend of the colours shown (by material,
//! or what each looks like with realistic colours) and their controls. A
//! row's name selects its cells (a link), its eye hides or shows them in 3D (as
//! [`crate::seen`] says: closed when none is drawn, whatever hides them),
//! and with realistic colours its look can be changed (an undoable edit,
//! saved with the view).

use llevant_render::colours::{material_colour, ColourBy};
use llevant_scene::materials::Category;
use llevant_scene::query::Query;

use crate::command::{Command, Edit};
use crate::context::link;
use crate::document::Document;
use crate::{icons, theme};

/// A material's row: number (0 for void), swatch, kind, cells, and the
/// smallest and largest density (g/cm³).
type Row = (i64, [u8; 3], String, usize, (f64, f64));

/// Height of a row, and widths of the columns before the densities.
const ROW: f32 = 22.0;
const COLUMNS: [f32; 4] = [22.0, 90.0, 120.0, 90.0];

/// Lay out a cell of a row `width` wide.
fn column<R>(ui: &mut egui::Ui, width: f32, f: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        egui::vec2(width, ROW),
        egui::Layout::left_to_right(egui::Align::Center),
        f,
    )
    .inner
}

#[derive(Default)]
pub struct MaterialsPanel {
    filter: String,
}

impl MaterialsPanel {
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
        let by: ColourBy = doc.scene.colour_by.into();
        let realistic = by == ColourBy::Realistic;
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text(format!("{}  number or kind", icons::SEARCH))
                        .desired_width(f32::INFINITY),
                );
                theme::name(&r, "materials filter");
                if !matches!(
                    by,
                    ColourBy::Material | ColourBy::Realistic | ColourBy::Density
                ) {
                    ui.label(
                        egui::RichText::new(
                            "Colour by material or realistically to see these colours",
                        )
                        .weak(),
                    );
                }
            });
        let f = self.filter.trim().to_ascii_lowercase();
        super::isolating(ui, doc, out);
        let mut rows: Vec<Row> = Vec::new();
        let void = doc.index.material_cells(0).len();
        if void > 0 {
            rows.push((
                0,
                material_colour(0),
                "void".into(),
                void,
                (f64::NAN, f64::NAN),
            ));
        }
        for l in &doc.looks.materials {
            let swatch = if realistic {
                l.appearance.srgb()
            } else if by == ColourBy::Density && l.density.0.is_finite() {
                llevant_render::scale::Scale::DENSITY.colour(l.density.0)
            } else {
                material_colour(l.id)
            };
            let cells = doc.index.material_cells(l.id).len();
            rows.push((
                l.id,
                swatch,
                l.appearance.category.name().into(),
                cells,
                l.density,
            ));
        }
        rows.retain(|(id, _, kind, ..)| {
            f.is_empty() || id.to_string().contains(&f) || kind.contains(&f)
        });
        // Only the rows scrolled to are laid out (models have hundreds).
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show_rows(ui, ROW, rows.len(), |ui, range| {
                for (k, (id, swatch, kind, cells, density)) in
                    rows[range.clone()].iter().cloned().enumerate()
                {
                    // Stripes, as the grid had: behind the row.
                    let stripe = ui.painter().add(egui::Shape::Noop);
                    let r = ui.horizontal(|ui| {
                        ui.set_height(ROW);
                        let q = if id == 0 {
                            Query::Void
                        } else {
                            Query::material(id)
                        };
                        let e = crate::seen::material_eye(
                            &doc.model, &doc.index, &doc.scene, &doc.seen, id,
                        );
                        column(ui, COLUMNS[0], |ui| {
                            super::eye(ui, &e, hide(doc, id, &q), "its cells", out)
                        });
                        column(ui, COLUMNS[1], |ui| {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                            ui.painter().rect_filled(
                                rect,
                                2.0,
                                egui::Color32::from_rgb(swatch[0], swatch[1], swatch[2]),
                            );
                            let name = if id == 0 {
                                "void".to_string()
                            } else {
                                format!("m{id}")
                            };
                            link(ui, doc, &q, name, out);
                        });
                        column(ui, COLUMNS[2], |ui| {
                            if realistic && id != 0 {
                                look_menu(ui, doc, id, &kind, out);
                            } else {
                                ui.label(egui::RichText::new(kind).weak());
                            }
                        });
                        column(ui, COLUMNS[3], |ui| {
                            ui.label(theme::data(format!(
                                "{cells} {}",
                                if cells == 1 { "cell" } else { "cells" }
                            )))
                        });
                        ui.label(theme::data(densities(density)));
                    });
                    if (range.start + k) % 2 == 1 {
                        let fill = ui.visuals().faint_bg_color;
                        ui.painter()
                            .set(stripe, egui::Shape::rect_filled(r.response.rect, 0.0, fill));
                    }
                }
            });
    }
}

/// Hide material `id` (or void).
fn hide(doc: &Document, id: i64, q: &Query) -> Command {
    if id == 0 {
        return Edit::HideVoid(true).into();
    }
    if doc.scene.hiding.materials.contains(&id) {
        // Shown though listed (as ghosts): hide by rule.
        return Edit::Hide(q.clone()).into();
    }
    let mut list = doc.scene.hiding.materials.clone();
    list.push(id);
    Edit::Hidden(crate::command::Hidden::Materials, list).into()
}

/// The look of material `id`: its kind, chosen from the list (or as
/// classified from its card again).
fn look_menu(ui: &mut egui::Ui, doc: &Document, id: i64, kind: &str, out: &mut Vec<Command>) {
    let custom = doc.scene.materials.contains_key(&id);
    let text = if custom {
        format!("{kind} *")
    } else {
        kind.to_string()
    };
    egui::ComboBox::from_id_salt(("look", id))
        .selected_text(text)
        .width(110.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(!custom, "as its card says").clicked() {
                out.push(Edit::Material(id, None).into());
            }
            for c in Category::ALL {
                if c == Category::Void {
                    continue;
                }
                if ui
                    .selectable_label(custom && c.name() == kind, c.name())
                    .clicked()
                {
                    out.push(Edit::Material(id, Some(c.name().to_string())).into());
                }
            }
        })
        .response
        .on_hover_text("What it looks like with realistic colours (* changed)");
}

fn densities((lo, hi): (f64, f64)) -> String {
    if !(lo.is_finite() && hi.is_finite()) {
        String::new()
    } else if (hi - lo).abs() <= 1e-9 * hi.abs() {
        format!("{lo:.4} g/cm³")
    } else {
        format!("{lo:.3}–{hi:.3} g/cm³")
    }
}
