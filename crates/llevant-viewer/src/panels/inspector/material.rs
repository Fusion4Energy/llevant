//! A material's page: its number and comment, what can be done with its
//! cells, its composition by element (and by nuclide, folded), the cells
//! using it and at what densities, its totals from the last volume run, and
//! its card.

use std::collections::BTreeMap;

use llevant_scene::query::Query;
use llevant_scene::source::CardKind;

use super::card::card;
use super::{drill, section, title, Subject};
use crate::command::{Command, Edit};
use crate::context::universe_link;
use crate::document::Document;
use crate::selection::SelectMode;
use crate::{icons, theme};

/// Element symbols by Z (1-based).
const SYMBOLS: [&str; 100] = [
    "H", "He", "Li", "Be", "B", "C", "N", "O", "F", "Ne", "Na", "Mg", "Al", "Si", "P", "S", "Cl",
    "Ar", "K", "Ca", "Sc", "Ti", "V", "Cr", "Mn", "Fe", "Co", "Ni", "Cu", "Zn", "Ga", "Ge", "As",
    "Se", "Br", "Kr", "Rb", "Sr", "Y", "Zr", "Nb", "Mo", "Tc", "Ru", "Rh", "Pd", "Ag", "Cd", "In",
    "Sn", "Sb", "Te", "I", "Xe", "Cs", "Ba", "La", "Ce", "Pr", "Nd", "Pm", "Sm", "Eu", "Gd", "Tb",
    "Dy", "Ho", "Er", "Tm", "Yb", "Lu", "Hf", "Ta", "W", "Re", "Os", "Ir", "Pt", "Au", "Hg", "Tl",
    "Pb", "Bi", "Po", "At", "Rn", "Fr", "Ra", "Ac", "Th", "Pa", "U", "Np", "Pu", "Am", "Cm", "Bk",
    "Cf", "Es", "Fm",
];

/// Elements listed at most (the rest summed as "other").
const ELEMENTS: usize = 10;

pub fn page(
    ui: &mut egui::Ui,
    doc: &Document,
    id: i64,
    forms: &mut super::fields::Forms,
    go: &mut Option<Subject>,
    out: &mut Vec<Command>,
) {
    let m = &doc.model;
    if id == 0 {
        title(ui, "void", None, None);
    } else {
        let [r, g, b] = llevant_render::colours::material_colour(id);
        title(
            ui,
            &format!("m {id}"),
            Some(egui::Color32::from_rgb(r, g, b)),
            super::comment(doc, CardKind::Material, id),
        );
    }
    let q = if id == 0 {
        Query::Void
    } else {
        Query::material(id)
    };
    ui.horizontal_wrapped(|ui| {
        if ui
            .small_button(icons::with(icons::SELECT, "Select its cells"))
            .clicked()
        {
            out.push(Command::Select(q.clone(), SelectMode::Replace));
        }
        if ui.small_button(icons::with(icons::HIDE, "Hide")).clicked() {
            out.push(Edit::Hide(q.clone()).into());
        }
        if ui
            .small_button(icons::with(icons::ISOLATE, "Isolate"))
            .clicked()
        {
            out.push(Edit::Isolate(Some(q.clone())).into());
        }
    });

    if let Some((_, x)) = doc.whole.as_ref().filter(|_| id != 0) {
        let outside = doc.outside(x.material_cells(id).iter().copied());
        super::shared_outside(ui, doc, "It is", &outside);
    }
    // Its fields (its composition read from the model without them).
    let editing = id != 0 && super::fields::show(ui, doc, CardKind::Material, id, forms, go, out);
    if let Some(entries) = m.material(id).filter(|_| !editing) {
        composition(ui, id, entries);
    }

    // The cells using it: how many, in which universes, at which densities.
    let cells = doc.index.material_cells(id);
    section(ui, ("material users", id), "Used by", true, |ui| {
        let mut universes: BTreeMap<i64, usize> = BTreeMap::new();
        let mut densities: BTreeMap<u64, (f64, usize)> = BTreeMap::new();
        for &c in cells {
            let cell = m.cell(c);
            *universes
                .entry(m.units()[cell.unit as usize].universe)
                .or_default() += 1;
            densities
                .entry(cell.density.to_bits())
                .or_insert((cell.density, 0))
                .1 += 1;
        }
        ui.label(format!(
            "{} cell{} in {} universe{}",
            cells.len(),
            if cells.len() == 1 { "" } else { "s" },
            universes.len(),
            if universes.len() == 1 { "" } else { "s" },
        ));
        let mut by_count: Vec<(i64, usize)> = universes.into_iter().collect();
        by_count.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        ui.horizontal_wrapped(|ui| {
            for &(u, n) in by_count.iter().take(8) {
                universe_link(ui, doc, u, out);
                ui.label(theme::data(format!("{n}")).weak());
                drill(ui, Subject::Universe(u), go);
            }
            if by_count.len() > 8 {
                ui.label(egui::RichText::new(format!("+{} more", by_count.len() - 8)).weak());
            }
        });
        if id != 0 {
            let mut ds: Vec<(f64, usize)> = densities.into_values().collect();
            ds.sort_by_key(|b| std::cmp::Reverse(b.1));
            let a = super::atomic_mass(doc, id);
            for (d, n) in ds.iter().take(4) {
                ui.label(theme::data(format!(
                    "{} · {n} cells",
                    super::density(*d, a)
                )));
            }
            if ds.len() > 4 {
                ui.label(egui::RichText::new(format!("{} more densities", ds.len() - 4)).weak());
            }
        }
    });

    // Totals from the last volume run, for the cells it measured.
    if let Some(run) = &doc.results.volumes {
        let mut v = 0.0;
        let mut s2 = 0.0;
        let mut grams = 0.0;
        let mut measured = 0;
        let a = super::atomic_mass(doc, id).unwrap_or(0.0);
        for g in &run.cells {
            let Some(c) = m.cell_index(g.id) else {
                continue;
            };
            let cell = m.cell(c);
            if cell.fill.is_some() || cell.material != id {
                continue;
            }
            measured += 1;
            v += g.volume;
            s2 += g.sigma * g.sigma;
            if let Some(rho) = llevant_scene::materials::grams_per_cm3(cell.density, a) {
                grams += rho * g.volume;
            }
        }
        if measured > 0 {
            section(ui, ("material totals", id), "Totals", true, |ui| {
                ui.label(theme::data(format!(
                    "{v:.5e} cm³ ± {:.2} %",
                    if v > 0.0 { 100.0 * s2.sqrt() / v } else { 0.0 }
                )));
                if id != 0 && grams > 0.0 {
                    ui.label(theme::data(super::mass(grams)));
                }
                ui.label(
                    egui::RichText::new(format!(
                        "{measured} of its cells, from the last volume run ({})",
                        run.spec.scope
                    ))
                    .weak(),
                );
            });
        }
    }

    if id != 0 {
        card(ui, doc, CardKind::Material, id, false, go, out);
    }
}

/// The card's composition: by element (mass or atom fractions, as the card
/// gives them), and its nuclides folded below.
fn composition(ui: &mut egui::Ui, id: i64, entries: &[(String, f64)]) {
    let by_mass = entries.first().is_some_and(|e| e.1 < 0.0);
    let mut by_z: BTreeMap<usize, f64> = BTreeMap::new();
    let mut total = 0.0;
    for (zaid, f) in entries {
        let z = zaid
            .split('.')
            .next()
            .and_then(|h| h.trim().parse::<u64>().ok())
            .map_or(0, |n| (n / 1000) as usize);
        let f = f.abs();
        *by_z.entry(z).or_default() += f;
        total += f;
    }
    section(
        ui,
        ("material composition", id),
        "Composition",
        true,
        |ui| {
            ui.label(
                egui::RichText::new(if by_mass {
                    "by mass, as the card gives it"
                } else {
                    "by atoms, as the card gives it"
                })
                .weak(),
            );
            let mut v: Vec<(usize, f64)> = by_z.into_iter().collect();
            v.sort_by(|a, b| b.1.total_cmp(&a.1));
            let other: f64 = v.iter().skip(ELEMENTS).map(|x| x.1).sum();
            let bar_width = (ui.available_width() - 110.0).max(40.0);
            egui::Grid::new(("composition", id))
                .num_columns(3)
                .spacing([8.0, 2.0])
                .show(ui, |ui| {
                    let rows = v
                        .iter()
                        .take(ELEMENTS)
                        .map(|&(z, f)| (element(z), f))
                        .chain((other > 0.0).then(|| ("other".to_string(), other)));
                    for (name, f) in rows {
                        let share = if total > 0.0 { f / total } else { 0.0 };
                        ui.label(theme::data(name));
                        let (r, _) = ui
                            .allocate_exact_size(egui::vec2(bar_width, 8.0), egui::Sense::hover());
                        let p = ui.painter();
                        p.rect_filled(r, 3.0, ui.visuals().faint_bg_color);
                        let mut fill = r;
                        fill.set_width(r.width() * share as f32);
                        p.rect_filled(fill, 3.0, ui.visuals().selection.bg_fill);
                        ui.label(theme::data(format!("{:.2} %", 100.0 * share)));
                        ui.end_row();
                    }
                });
            egui::CollapsingHeader::new(
                egui::RichText::new(format!(
                    "{} nuclide{}",
                    entries.len(),
                    if entries.len() == 1 { "" } else { "s" }
                ))
                .weak(),
            )
            .id_salt(("nuclides", id))
            .show(ui, |ui| {
                egui::Grid::new(("nuclides grid", id))
                    .num_columns(2)
                    .spacing([12.0, 1.0])
                    .show(ui, |ui| {
                        for (zaid, f) in entries {
                            ui.label(theme::data(zaid));
                            ui.label(theme::data(format!("{f}")));
                            ui.end_row();
                        }
                    });
            });
        },
    );
}

/// An element's symbol, from Z (0: what the card does not say).
pub(super) fn element(z: usize) -> String {
    match z {
        1..=100 => SYMBOLS[z - 1].to_string(),
        _ => "?".to_string(),
    }
}
