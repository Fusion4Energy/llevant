//! A universe's page: its number and name, what can be done with it, what
//! it holds, where it is filled and how many times it appears, and its
//! materials.

use std::collections::BTreeMap;

use llevant_scene::query::Query;

use super::cell::LISTED;
use super::{drill, section, title, Subject};
use crate::command::{Command, Edit};
use crate::context::{cell_link, material_link};
use crate::document::Document;
use crate::selection::SelectMode;
use crate::{icons, theme};

pub fn page(
    ui: &mut egui::Ui,
    doc: &Document,
    u: i64,
    go: &mut Option<Subject>,
    out: &mut Vec<Command>,
) {
    let m = &doc.model;
    let Some(unit) = doc.index.unit(u) else {
        super::missing(ui, "universe", u);
        return;
    };
    let cells = &m.units()[unit as usize].cells;
    let name = cells
        .iter()
        .take(4)
        .find_map(|&c| super::comment(doc, llevant_scene::source::CardKind::Cell, m.cell(c).id));
    title(ui, &format!("u={u}"), None, name.filter(|_| u != 0));
    let q = Query::universe(u);
    ui.horizontal_wrapped(|ui| {
        if ui
            .small_button(icons::with(icons::SELECT, "Select its cells"))
            .clicked()
        {
            out.push(Command::Select(q.clone(), SelectMode::Replace));
        }
        if ui
            .small_button(icons::with(icons::ISOLATE, "Isolate"))
            .clicked()
        {
            out.push(Edit::Isolate(Some(q.clone())).into());
        }
        if u != 0
            && ui
                .small_button(icons::with(icons::OPEN, "Open on its own"))
                .on_hover_text("This universe alone, cut to what it holds (Back returns here)")
                .clicked()
        {
            out.push(Command::OpenUniverse(u));
        }
    });
    if u != 0 && doc.source.is_some() {
        super::many::renumber_one(ui, doc, llevant_scene::source::IdKind::Universe, u, out);
    }

    section(ui, ("universe contents", u), "Contents", true, |ui| {
        ui.label(format!(
            "{} cell{}",
            cells.len(),
            if cells.len() == 1 { "" } else { "s" }
        ));
        ui.horizontal_wrapped(|ui| {
            for &c in cells.iter().take(LISTED) {
                cell_link(ui, doc, c, out);
            }
            if cells.len() > LISTED {
                ui.label(egui::RichText::new(format!("+{} more", cells.len() - LISTED)).weak());
            }
        });
        // The universes it fills cells with, one level down.
        let mut below: Vec<i64> = Vec::new();
        for &c in cells {
            for &k in doc.index.fills(c) {
                let v = m.units()[k as usize].universe;
                if !below.contains(&v) {
                    below.push(v);
                }
            }
        }
        if !below.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("fills cells with").weak());
                for v in below.iter().take(LISTED) {
                    crate::context::universe_link(ui, doc, *v, out);
                    drill(ui, Subject::Universe(*v), go);
                }
            });
        }
    });

    if u != 0 {
        section(ui, ("universe place", u), "Filled into", true, |ui| {
            let holders = doc.index.holders(unit);
            ui.horizontal_wrapped(|ui| {
                for &h in holders.iter().take(LISTED) {
                    cell_link(ui, doc, h, out);
                }
                if holders.len() > LISTED {
                    ui.label(
                        egui::RichText::new(format!("+{} more", holders.len() - LISTED)).weak(),
                    );
                }
            });
            let n = m.instances().count_unit(unit);
            ui.label(
                egui::RichText::new(format!(
                    "{n} instance{} in the model",
                    if n == 1 { "" } else { "s" }
                ))
                .weak(),
            );
        });
    }

    section(ui, ("universe materials", u), "Materials", true, |ui| {
        let mut by_m: BTreeMap<i64, usize> = BTreeMap::new();
        for &c in cells {
            let cell = m.cell(c);
            if cell.fill.is_none() {
                *by_m.entry(cell.material).or_default() += 1;
            }
        }
        let mut v: Vec<(i64, usize)> = by_m.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let total: usize = v.iter().map(|x| x.1).sum();
        egui::Grid::new(("universe materials grid", u))
            .num_columns(3)
            .spacing([8.0, 2.0])
            .show(ui, |ui| {
                for &(mat, n) in v.iter().take(LISTED) {
                    ui.horizontal(|ui| {
                        material_link(ui, doc, mat, out);
                        if mat != 0 {
                            drill(ui, Subject::Material(mat), go);
                        }
                    });
                    ui.label(theme::data(format!("{n}")));
                    ui.label(
                        egui::RichText::new(format!(
                            "{:.0} % of its cells",
                            100.0 * n as f64 / total.max(1) as f64
                        ))
                        .weak(),
                    );
                    ui.end_row();
                }
            });
    });
}
