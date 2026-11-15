//! A cell's page: its number and comment, what can be done with it, then
//! its fields (material, density, geometry with a chip per surface,
//! parameters; [`super::fields`]), or what fills it, where it sits (its
//! universe, the cells holding it, how many times it appears), its size
//! (volume and mass of every instance together, when a volume run has them,
//! or measured on a click, then each instance on its own, folded),
//! and its card, folded.

use llevant_scene::query::Query;
use llevant_scene::source::CardKind;

use super::card::{card, transform_cards};
use super::{drill, section, title, Subject};
use crate::command::{Command, Edit};
use crate::context::{cell_link, material_link, universe_link};
use crate::document::Document;
use crate::jobs::{
    CellOutcome, InstanceVolume, JobKind, VolumeRun, VolumeSpec, MAX_LISTED_INSTANCES,
};
use crate::selection::SelectMode;
use crate::{icons, theme};

/// Cells named in a row at most (holders, users…).
pub const LISTED: usize = 12;

/// The page of cell `c` (index into the model's cells).
pub fn page(
    ui: &mut egui::Ui,
    doc: &Document,
    c: u32,
    forms: &mut super::fields::Forms,
    go: &mut Option<Subject>,
    out: &mut Vec<Command>,
) {
    let m = &doc.model;
    let cell = m.cell(c);
    let id = cell.id;
    let q = Query::cell(id);
    let swatch = {
        let paint = llevant_render::colours::Paint::new(m, &doc.looks, doc.scene.colour_by.into());
        let [r, g, b] = paint.cell_colour(c);
        Some(egui::Color32::from_rgb(r, g, b))
    };
    title(
        ui,
        &format!("c {id}"),
        swatch,
        super::comment(doc, CardKind::Cell, id),
    );
    actions(ui, doc, &q, out);
    // Its fields; read from the model while the input's cards are not.
    let fields = super::fields::show(ui, doc, CardKind::Cell, id, forms, go, out);

    // What fills it; what it is made of and its surfaces, without fields.
    if cell.fill.is_some() || !fields {
        section(ui, ("cell material", id), "Material", true, |ui| {
            if cell.fill.is_some() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("filled with").weak());
                    for &unit in doc.index.fills(c) {
                        let u = m.units()[unit as usize].universe;
                        universe_link(ui, doc, u, out);
                        drill(ui, Subject::Universe(u), go);
                    }
                });
            } else if cell.material == 0 {
                ui.label(egui::RichText::new("void").weak());
            } else {
                ui.horizontal_wrapped(|ui| {
                    material_link(ui, doc, cell.material, out);
                    if let Some(name) = super::comment(doc, CardKind::Material, cell.material) {
                        ui.label(name);
                    }
                    drill(ui, Subject::Material(cell.material), go);
                });
                let a = super::atomic_mass(doc, cell.material);
                ui.label(theme::data(super::density(cell.density, a)));
            }
            if !fields {
                let mut surfaces: Vec<(i64, bool)> = Vec::new();
                for (shape, positive) in m.cell_surfaces(c) {
                    if let Some(s) = m.surface_id(shape) {
                        if !surfaces.contains(&(s.id, !positive)) {
                            surfaces.push((s.id, !positive));
                        }
                    }
                }
                super::fields::surface_chips(ui, doc, &surfaces, go);
            }
        });
    }

    // Where it sits.
    section(ui, ("cell place", id), "Place", true, |ui| {
        let unit = cell.unit;
        let u = m.units()[unit as usize].universe;
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("in").weak());
            universe_link(ui, doc, u, out);
            if let Some(name) = universe_name(doc, unit) {
                ui.label(egui::RichText::new(name).weak());
            }
            drill(ui, Subject::Universe(u), go);
        });
        let holders = doc.index.holders(unit);
        if !holders.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("filled into").weak());
                for &h in holders.iter().take(LISTED) {
                    cell_link(ui, doc, h, out);
                }
                if holders.len() > LISTED {
                    ui.label(
                        egui::RichText::new(format!("+{} more", holders.len() - LISTED)).weak(),
                    );
                }
            });
        }
        let n = m.instances().count_unit(unit);
        ui.label(
            egui::RichText::new(match n {
                1 => "1 instance".to_string(),
                n => format!("{n} instances"),
            })
            .weak(),
        )
        .on_hover_text("How many places of the model this cell is at (every copy of its universe)");
        if cell.imp_zero {
            ui.label(egui::RichText::new("outside the problem (IMP:N=0)").weak());
        }
        if let Some(why) = &cell.unsupported {
            ui.colored_label(ui.visuals().warn_fg_color, format!("not drawn: {why}"));
        }
    });

    size(ui, doc, c, out);

    card(ui, doc, CardKind::Cell, id, false, go, out);
    transform_cards(ui, doc, CardKind::Cell, id, go, out);
}

/// Select, Hide, Isolate, Ghost and Frame for `q`.
fn actions(ui: &mut egui::Ui, doc: &Document, q: &Query, out: &mut Vec<Command>) {
    ui.horizontal_wrapped(|ui| {
        let selected = doc.selection.query.as_ref() == Some(q);
        if !selected
            && ui
                .small_button(icons::with(icons::SELECT, "Select"))
                .clicked()
        {
            out.push(Command::Select(q.clone(), SelectMode::Replace));
        }
        if ui
            .small_button(icons::with(icons::HIDE, "Hide"))
            .on_hover_text("Hide it (and what it holds)")
            .clicked()
        {
            out.push(Edit::Hide(q.clone()).into());
        }
        if ui
            .small_button(icons::with(icons::ISOLATE, "Isolate"))
            .on_hover_text("Show only it, what it holds and the cells holding it")
            .clicked()
        {
            out.push(Edit::Isolate(Some(q.clone())).into());
        }
        if ui
            .small_button(icons::with(icons::GHOST, "Ghost"))
            .on_hover_text("A shell over what is behind it, what it holds shown")
            .clicked()
        {
            out.push(Edit::Ghost(q.clone()).into());
        }
        if ui
            .small_button(icons::with(icons::FRAME, "Frame"))
            .on_hover_text("Select it, and frame it in both views")
            .clicked()
        {
            out.push(Command::Select(q.clone(), SelectMode::Replace));
            out.push(Command::FrameSelection);
        }
    });
}

/// The volume and mass of cell `c`, from the last volume run, or a button
/// that measures it.
fn size(ui: &mut egui::Ui, doc: &Document, c: u32, out: &mut Vec<Command>) {
    let m = &doc.model;
    let cell = m.cell(c);
    let id = cell.id;
    section(ui, ("cell size", id), "Size", true, |ui| {
        let run = doc.results.volumes.as_ref();
        let got = run.and_then(|r| r.cells.iter().find(|v| v.id == id));
        match got {
            Some(v) => {
                let rel = if v.volume > 0.0 {
                    v.sigma / v.volume
                } else {
                    0.0
                };
                ui.label(theme::data(format!(
                    "{:.5e} cm³ ± {:.2} %",
                    v.volume,
                    100.0 * rel
                )))
                .on_hover_text("Every instance together, from the last volume run");
                let a = super::atomic_mass(doc, cell.material);
                let rho = if cell.material == 0 {
                    Some(0.0)
                } else {
                    llevant_scene::materials::grams_per_cm3(cell.density, a.unwrap_or(0.0))
                };
                let rho = rho.filter(|_| cell.fill.is_none());
                if let Some(rho) = rho {
                    ui.label(theme::data(super::mass(rho * v.volume)))
                        .on_hover_text("Every instance together");
                }
                if let Some(run) = run {
                    instances(ui, doc, run, c, rho, out);
                }
            }
            None => {
                let running = doc.results.jobs.iter().any(|j| j.kind == JobKind::Volumes);
                ui.horizontal_wrapped(|ui| {
                    let r = ui
                        .add_enabled(
                            !running,
                            egui::Button::new(icons::with(icons::TARGET, "Measure")).small(),
                        )
                        .on_hover_text(
                            "Its volume (every instance together, and each on its own) by \
                             tracing lines around it, to 1 %; the result also shows in \
                             Volumes (Explore)",
                        );
                    if r.clicked() {
                        if let Some(spec) = measure(doc, c) {
                            out.push(Command::StartVolumes(spec));
                        }
                    }
                    ui.label(egui::RichText::new("not measured yet").weak());
                });
            }
        }
    });
}

/// Rows shown before the list of instances scrolls.
const INSTANCE_ROWS: usize = 12;

/// Cell `c`'s instances one by one from `run`, folded: where each is (the
/// cell holding it a link), its volume and, with density `rho`, its mass;
/// largest first.
fn instances(
    ui: &mut egui::Ui,
    doc: &Document,
    run: &VolumeRun,
    c: u32,
    rho: Option<f64>,
    out: &mut Vec<Command>,
) {
    let m = &doc.model;
    let id = m.cell_id(c);
    let n = m.instances().ids(c).count();
    if n < 2 {
        return;
    }
    let mut rows: Vec<&InstanceVolume> = run.instances.iter().filter(|i| i.cell == id).collect();
    if rows.is_empty() {
        if run.spec.instances && n as u64 > MAX_LISTED_INSTANCES {
            ui.label(
                egui::RichText::new(format!("{n} instances: too many to measure one by one"))
                    .weak(),
            );
        }
        return;
    }
    rows.sort_by(|a, b| b.volume.total_cmp(&a.volume).then(a.id.cmp(&b.id)));
    let total: f64 = rows.iter().map(|r| r.volume).sum();
    egui::CollapsingHeader::new(format!("{n} instances"))
        .id_salt(("cell instances", id))
        .default_open(false)
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("each on its own, largest first: the cell holding it, then its size and its share").weak(),
            );
            let mut grid = |ui: &mut egui::Ui| {
                egui::Grid::new(("cell instance rows", id))
                    .num_columns(3)
                    .spacing([10.0, 2.0])
                    .striped(true)
                    .show(ui, |ui| {
                        for r in &rows {
                            instance_row(ui, doc, r, rho, total, out);
                            ui.end_row();
                        }
                    });
            };
            if rows.len() > INSTANCE_ROWS {
                egui::ScrollArea::vertical()
                    .id_salt(("cell instance scroll", id))
                    .max_height(INSTANCE_ROWS as f32 * 20.0)
                    .show(ui, grid);
            } else {
                grid(ui);
            }
        });
}

/// One instance: the cell holding it (a link, its full place on hover),
/// its volume ± σ or what kept it from one, its mass or share of the total.
fn instance_row(
    ui: &mut egui::Ui,
    doc: &Document,
    r: &InstanceVolume,
    rho: Option<f64>,
    total: f64,
    out: &mut Vec<Command>,
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("in").weak());
        match r.holder {
            Some(h) => cell_link(ui, doc, h, out),
            None => {
                ui.label("level 0");
            }
        }
    })
    .response
    .on_hover_text(format!("instance {}: {}", r.id, r.place));
    let size = match r.outcome {
        CellOutcome::Empty => egui::RichText::new("none").weak(),
        CellOutcome::Unseen => egui::RichText::new("too small to meet").weak(),
        _ => {
            let rel = if r.volume > 0.0 {
                r.sigma / r.volume
            } else {
                0.0
            };
            let text = match rho.filter(|&d| d > 0.0) {
                Some(d) => super::mass(d * r.volume),
                None => format!("{:.4e} cm³", r.volume),
            };
            theme::data(format!("{text} ± {:.1} %", 100.0 * rel))
        }
    };
    let short = r.outcome == CellOutcome::Unconverged;
    let l = ui.label(size);
    if short {
        l.on_hover_text("did not reach the target σ");
    } else if rho.is_some_and(|d| d > 0.0) {
        l.on_hover_text(format!("{:.5e} cm³", r.volume));
    }
    let share = if total > 0.0 { r.volume / total } else { 0.0 };
    ui.label(egui::RichText::new(format!("{:.1} %", 100.0 * share)).weak());
}

/// Lines across the problem through cell `c`'s instances alone, to 1 %.
fn measure(doc: &Document, c: u32) -> Option<VolumeSpec> {
    let region = doc.model.bounds(llevant::Scope::Problem);
    if !region.is_finite() || region.is_empty() {
        return None;
    }
    let cells = llevant_scene::index::CellSet::from_cells(doc.index.cell_count(), [c]);
    let around = crate::selection::selection_box(&doc.model, &doc.index, &cells, &doc.region)?;
    Some(VolumeSpec {
        region,
        h: crate::jobs::spacing_for(&around, 1e5),
        target: Some(1e-2),
        cells: Some(vec![c]),
        instances: true,
        scope: format!("c {}", doc.model.cell_id(c)),
    })
}

/// The name of universe unit `unit`: the comment of the first card of its
/// cells that has one.
fn universe_name(doc: &Document, unit: u32) -> Option<String> {
    let m = &doc.model;
    let u = &m.units()[unit as usize];
    if u.universe == 0 {
        return None;
    }
    u.cells
        .iter()
        .take(4)
        .find_map(|&c| super::comment(doc, CardKind::Cell, m.cell(c).id))
}
