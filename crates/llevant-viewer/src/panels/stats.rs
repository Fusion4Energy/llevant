//! The model in numbers: sizes, how long each step of loading took, the
//! largest universes and the most used materials (bars whose names are
//! links), and the universes alike up to numbering (from fingerprints, as a
//! job). The numbers walk the whole model (its trees and octrees: 0.1 s on
//! the reference model), so they are worked out once per model, on a task.

use std::sync::Arc;

use llevant::{Model, Stats};
use llevant_scene::index::ModelIndex;
use llevant_scene::query::Query;

use crate::command::Command;
use crate::context::{link, material_link, universe_link};
use crate::document::Document;
use crate::jobs::JobKind;
use crate::tasks::Task;
use crate::{icons, theme};

/// Rows of each ranking.
const TOP: usize = 10;

/// What the panel shows of a model.
struct Summary {
    stats: Stats,
    materials: usize,
    /// (universe, cells), most first, [`TOP`] of them.
    by_cells: Vec<(i64, usize)>,
    /// (universe, instances).
    by_instances: Vec<(i64, usize)>,
    /// (material, cells).
    by_material: Vec<(i64, usize)>,
}

/// The `TOP` rows of `rows` with the most, ties by number.
fn top(mut rows: Vec<(i64, usize)>) -> Vec<(i64, usize)> {
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    rows.truncate(TOP);
    rows
}

fn summary(m: &Model, x: &ModelIndex) -> Summary {
    let units = m.units();
    Summary {
        stats: m.stats(),
        materials: x.materials().iter().filter(|&&m| m != 0).count(),
        by_cells: top(units.iter().map(|u| (u.universe, u.cells.len())).collect()),
        by_instances: top((0..units.len())
            .map(|k| {
                let n = m.instances().count_unit(k as u32);
                (units[k].universe, n.min(usize::MAX as u64) as usize)
            })
            .collect()),
        by_material: top(x
            .materials()
            .into_iter()
            .map(|id| (id, x.material_cells(id).len()))
            .collect()),
    }
}

/// The statistics tab: its numbers, per model.
#[derive(Default)]
pub struct StatsPanel {
    /// The model's address, and its numbers (or the task making them).
    of: usize,
    making: Option<Task<Summary>>,
    made: Option<Arc<Summary>>,
}

impl StatsPanel {
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
        let key = Arc::as_ptr(&doc.model) as usize;
        if key != self.of || (self.made.is_none() && self.making.is_none()) {
            self.of = key;
            self.made = None;
            let (m, x) = (doc.model.clone(), doc.index.clone());
            self.making = Some(Task::run("stats", Some(ui.ctx().clone()), move |_| {
                summary(&m, &x)
            }));
        }
        if let Some(s) = self.making.as_ref().and_then(|t| t.try_take()) {
            self.made = Some(Arc::new(s));
            self.making = None;
        }
        match self.made.clone() {
            Some(s) => panel(ui, doc, &s, out),
            None => {
                ui.horizontal(|ui| {
                    crate::logo::small_spinner(ui);
                    ui.label(egui::RichText::new("counting").weak());
                });
            }
        }
    }
}

fn panel(ui: &mut egui::Ui, doc: &Document, sum: &Summary, out: &mut Vec<Command>) {
    let m = &doc.model;
    let s = &sum.stats;
    theme::section(ui, icons::INFO, "Model", |ui| {
        egui::Grid::new("sizes").num_columns(2).show(ui, |ui| {
            let mut row = |name: &str, v: String| {
                ui.label(egui::RichText::new(name).weak());
                ui.label(theme::data(v));
                ui.end_row();
            };
            row("cells", s.cells.to_string());
            row("surfaces", s.surfaces.to_string());
            row("universes", s.units.to_string());
            row("lattices", s.lattices.to_string());
            row("materials", sum.materials.to_string());
            row("unsupported cells", s.unsupported_cells.to_string());
            row("diagnostics", m.diagnostics().len().to_string());
            row("deepest search tree", s.max_bvh_depth.to_string());
            row("universes with an octree", s.grid_units.to_string());
            if s.pending_units > 0 {
                row("octrees still to build", s.pending_units.to_string());
            }
            let r = &doc.region;
            row(
                "shown extent (cm)",
                format!(
                    "{:.0} × {:.0} × {:.0}",
                    r.hi[0] - r.lo[0],
                    r.hi[1] - r.lo[1],
                    r.hi[2] - r.lo[2]
                ),
            );
        });
        let t = m.timings();
        if !t.is_empty() {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    t.iter()
                        .map(|(k, v)| format!("{k} {v:.2} s"))
                        .collect::<Vec<_>>()
                        .join(" · "),
                )
                .weak(),
            );
        }
    });

    // Rankings, with bars.
    theme::section(ui, icons::UNIVERSE, "Largest universes", |ui| {
        bars(
            ui,
            &sum.by_cells,
            out,
            &|ui, u, out| universe_link(ui, doc, u, out),
            "cells",
        );
    });
    theme::section(ui, icons::UNIVERSE, "Most repeated universes", |ui| {
        bars(
            ui,
            &sum.by_instances,
            out,
            &|ui, u, out| universe_link(ui, doc, u, out),
            "×",
        );
    });
    theme::section(ui, icons::MATERIAL, "Most used materials", |ui| {
        bars(
            ui,
            &sum.by_material,
            out,
            &|ui, id, out| material_link(ui, doc, id, out),
            "cells",
        );
    });

    theme::section(ui, icons::COPY, "Universes alike", |ui| {
        let running = doc
            .results
            .jobs
            .iter()
            .any(|j| j.kind == JobKind::Fingerprints);
        if ui
            .add_enabled(!running, egui::Button::new("Find universes alike"))
            .on_hover_text("Universes with the same geometry and materials under other numbers")
            .clicked()
        {
            out.push(Command::StartJob(
                JobKind::Fingerprints,
                llevant::Aabb::EMPTY,
                0.0,
                String::new(),
                None,
            ));
        }
        let Some(a) = &doc.results.alike else { return };
        if a.groups.is_empty() {
            ui.label(egui::RichText::new("every universe is unlike the others").weak());
        }
        for g in a.groups.iter().take(50) {
            ui.horizontal_wrapped(|ui| {
                link(
                    ui,
                    doc,
                    &Query::Universes(g.clone()),
                    theme::data(format!("{}×", g.len())),
                    out,
                );
                for &u in g.iter().take(12) {
                    universe_link(ui, doc, u, out);
                }
                if g.len() > 12 {
                    ui.label(egui::RichText::new(format!("+{}", g.len() - 12)).weak());
                }
            });
        }
    });
}

/// The first rows of `rows` (number, count) as bars, longest first.
fn bars(
    ui: &mut egui::Ui,
    rows: &[(i64, usize)],
    out: &mut Vec<Command>,
    name: &dyn Fn(&mut egui::Ui, i64, &mut Vec<Command>),
    unit: &str,
) {
    let max = rows.first().map_or(1, |r| r.1.max(1)) as f32;
    egui::Grid::new(ui.next_auto_id())
        .num_columns(3)
        .show(ui, |ui| {
            for &(k, n) in rows {
                name(ui, k, out);
                let w = 120.0;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 10.0), egui::Sense::hover());
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(rect.min, egui::vec2(w * n as f32 / max, 10.0)),
                    2.0,
                    theme::PIVOT.gamma_multiply(0.8),
                );
                ui.label(theme::data(format!("{n} {unit}")));
                ui.end_row();
            }
        });
}
