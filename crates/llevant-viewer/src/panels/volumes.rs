//! Volumes (and masses) by cell, material or universe, computed in the
//! background (`jobs`): over a region, one pass at a spacing (the default;
//! refined by halving it everywhere), or every cell refined to a target σ
//! where it falls short (`Model::volumes_to`; shown after each round, kept
//! if cancelled: on a big model rounds take long); or the selected cells alone, lines
//! traced only around them (`Model::instance_volumes`). Sortable, each row
//! a link, with the selection's total, exported as CSV next to the input.
//! Materials and universes add up their cells (σ in quadrature).

use std::collections::HashMap;
use std::sync::Arc;

use llevant::{GroupBy, Scope};
use llevant_scene::index::CellSet;
use llevant_scene::materials::grams_per_cm3;
use llevant_scene::query::Query;

use crate::command::Command;
use crate::context::link;
use crate::document::Document;
use crate::jobs::{CellOutcome, JobKind, VolumeRun, VolumeSpec};
use crate::panels::region::{Area, RegionPicker};
use crate::{icons, theme};

/// A row of the table.
struct Row {
    query: Option<Query>,
    name: String,
    volume: f64,
    sigma: f64,
    /// Grams, where every cell's density is in g/cm³.
    mass: Option<f64>,
    /// Why it is less sure than its σ says, if it is.
    warning: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sort {
    Name,
    Volume,
    Sigma,
    Mass,
}

pub struct VolumesPanel {
    pub region: RegionPicker,
    /// σ / volume to refine every cell to; `None`: one pass (the default:
    /// whole-model targets on big models take hours).
    target: Option<f64>,
    by: GroupBy,
    sort: (Sort, bool),
    only_selected: bool,
    /// The run and grouping the rows are of.
    rows_of: Option<(usize, GroupBy)>,
    rows: Vec<Row>,
    /// Per cell number: volume, σ² and mass (for selection totals).
    cells: HashMap<i64, (f64, f64, Option<f64>)>,
    /// The selection's total, summed again only when the selection or the
    /// rows change.
    total: Option<SelectionTotal>,
}

/// Volume, σ² and mass (`None` unless every cell has one).
type Total = (f64, f64, Option<f64>);

struct SelectionTotal {
    /// The selection summed (held, so that its address is not reused).
    cells: Arc<CellSet>,
    /// [`VolumesPanel::rows_of`] when it was summed.
    rows_of: Option<(usize, GroupBy)>,
    total: Total,
}

impl Default for VolumesPanel {
    fn default() -> VolumesPanel {
        VolumesPanel {
            region: RegionPicker::default(),
            target: None,
            by: GroupBy::Material,
            sort: (Sort::Volume, true),
            only_selected: false,
            rows_of: None,
            rows: Vec::new(),
            cells: HashMap::new(),
            total: None,
        }
    }
}

impl VolumesPanel {
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
        let picked = self.region.ui(ui, doc);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("σ/V").weak());
            for (t, name, hint) in [
                (None, "one pass", "Lines at the spacing above, everywhere"),
                (
                    Some(1e-2),
                    "1 %",
                    "Refine each cell until its σ is 1 % of its volume",
                ),
                (
                    Some(1e-3),
                    "0.1 %",
                    "Refine each cell until its σ is 0.1 % of its volume",
                ),
            ] {
                if ui
                    .selectable_label(self.target == t, name)
                    .on_hover_text(hint)
                    .clicked()
                {
                    self.target = t;
                    self.region.halvings = 0;
                }
            }
        });
        let selection = self.region.area == Area::Selection;
        if selection {
            ui.label(
                egui::RichText::new(
                    "the selected cells alone, every instance, lines only around them",
                )
                .weak(),
            );
        }
        // The selected cells and the problem's box, for the selection alone.
        let spec = |p: &crate::panels::region::Picked| -> Result<VolumeSpec, String> {
            let (region, cells) = if selection {
                let cells: Vec<u32> = doc.selection.cells.iter().collect();
                let region = doc.model.bounds(Scope::Problem);
                if !region.is_finite() || region.is_empty() {
                    return Err("the problem has no finite extent".into());
                }
                (region, Some(cells))
            } else {
                (p.region, None)
            };
            Ok(VolumeSpec {
                region,
                h: p.h,
                target: self.target,
                cells,
                instances: false,
                scope: p.scope.clone(),
            })
        };
        let running = doc.results.jobs.iter().any(|j| j.kind == JobKind::Volumes);
        ui.horizontal(|ui| {
            let can = picked.is_ok() && !running;
            if ui
                .add_enabled(
                    can,
                    egui::Button::new(icons::with(icons::TARGET, "Compute volumes")),
                )
                .clicked()
            {
                match picked.as_ref().map_err(Clone::clone).and_then(&spec) {
                    Ok(s) => out.push(Command::StartVolumes(s)),
                    Err(e) => out.push(Command::Warn(e)),
                }
            }
            if self.target.is_none()
                && !selection
                && doc.results.volumes.is_some()
                && ui
                    .add_enabled(!running, egui::Button::new("Refine"))
                    .on_hover_text("Halve the spacing: 4× the lines, σ about halved")
                    .clicked()
            {
                self.region.halvings += 1;
                if let Ok(s) = self.region.pick(doc).and_then(|p| spec(&p)) {
                    out.push(Command::StartVolumes(s));
                }
            }
        });
        let Some(run) = &doc.results.volumes else {
            crate::app::frame::hint(
                ui,
                "Volumes of every cell, material and universe in a region, or of the \
                 selected cells, from lines traced exactly through them, with their \
                 uncertainty.",
            );
            return;
        };
        ui.separator();
        let short = run.short();
        ui.label(
            egui::RichText::new(format!(
                "{}{}: {}h = {} cm, {:.1} s{}",
                if run.partial { "so far, " } else { "" },
                run.spec.scope,
                run.spec
                    .target
                    .map_or(String::new(), |t| format!("σ/V ≤ {}% from ", 100.0 * t)),
                crate::jobs::short(run.spec.h),
                run.seconds,
                match short {
                    0 => String::new(),
                    _ if run.partial => format!(", {short} cells not refined yet"),
                    _ => format!(", {short} cells short of it"),
                }
            ))
            .weak(),
        );
        ui.horizontal(|ui| {
            for (b, name) in [
                (GroupBy::Material, "material"),
                (GroupBy::Cell, "cell"),
                (GroupBy::Universe, "universe"),
            ] {
                ui.selectable_value(&mut self.by, b, name);
            }
            if self.by == GroupBy::Cell {
                ui.checkbox(&mut self.only_selected, "only selected");
            }
            if ui
                .small_button(icons::with(icons::SAVE, "CSV"))
                .on_hover_text("Write the table next to the input")
                .clicked()
            {
                out.push(match self.export(doc) {
                    Ok(m) => Command::Message(m),
                    Err(e) => Command::Warn(e),
                });
            }
        });
        self.refresh(doc, run);
        // The selection's total.
        if !doc.selection.is_empty() {
            let (v, s2, m) = self.selection_total(doc);
            ui.label(
                theme::data(format!(
                    "selection: {} ± {}{}",
                    volume(v),
                    volume(s2.sqrt()),
                    match m {
                        Some(m) if m > 0.0 => format!(" · {}", mass(m)),
                        _ => String::new(),
                    }
                ))
                .color(theme::SELECTION),
            );
        }
        let selected: Option<Vec<usize>> =
            (self.by == GroupBy::Cell && self.only_selected).then(|| {
                (0..self.rows.len())
                    .filter(|&k| {
                        let Some(Query::Cells(ids)) = &self.rows[k].query else {
                            return false;
                        };
                        ids.iter().any(|&id| {
                            doc.model
                                .cell_index(id)
                                .is_some_and(|c| doc.selection.cells.contains(c))
                        })
                    })
                    .collect()
            });
        let order: Vec<usize> = selected.unwrap_or_else(|| (0..self.rows.len()).collect());
        let heads = [
            (Sort::Name, "what"),
            (Sort::Volume, "cm³"),
            (Sort::Sigma, "± %"),
            (Sort::Mass, "mass"),
        ];
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("volumes")
                    .num_columns(4)
                    .striped(true)
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for (s, h) in heads {
                            let arrow = match self.sort {
                                (x, false) if x == s => icons::ASCENDING,
                                (x, true) if x == s => icons::DESCENDING,
                                _ => "",
                            };
                            if ui
                                .add(egui::Button::new(format!("{h} {arrow}")).frame(false))
                                .clicked()
                            {
                                self.sort = (s, self.sort != (s, true));
                                self.rows_of = None;
                            }
                        }
                        ui.end_row();
                        for &k in &order {
                            let r = &self.rows[k];
                            match (&r.query, r.warning) {
                                (Some(q), None) => {
                                    link(ui, doc, q, r.name.clone(), out);
                                }
                                (Some(q), Some(w)) => {
                                    ui.horizontal(|ui| {
                                        link(ui, doc, q, r.name.clone(), out);
                                        ui.colored_label(ui.visuals().warn_fg_color, "?")
                                            .on_hover_text(w);
                                    });
                                }
                                (None, _) => {
                                    ui.colored_label(ui.visuals().warn_fg_color, &r.name);
                                }
                            }
                            ui.label(theme::data(volume(r.volume)));
                            let rel = if r.volume > 0.0 {
                                100.0 * r.sigma / r.volume
                            } else {
                                f64::NAN
                            };
                            ui.label(theme::data(format!("{rel:.2}")));
                            ui.label(theme::data(r.mass.map_or(String::new(), mass)));
                            ui.end_row();
                        }
                    });
            });
    }

    /// The selected cells' volume, σ² and mass (`None` unless every cell
    /// has one), summed once per selection and run.
    fn selection_total(&mut self, doc: &Document) -> Total {
        let sel = &doc.selection.cells;
        if let Some(t) = &self.total {
            if Arc::ptr_eq(&t.cells, sel) && t.rows_of == self.rows_of {
                return t.total;
            }
        }
        let (mut v, mut s2, mut m, mut all_mass) = (0.0, 0.0, 0.0, true);
        for c in sel.iter() {
            if let Some(&(cv, cs2, cm)) = self.cells.get(&doc.model.cell_id(c)) {
                v += cv;
                s2 += cs2;
                match cm {
                    Some(x) => m += x,
                    None => all_mass = false,
                }
            }
        }
        let t = (v, s2, all_mass.then_some(m));
        self.total = Some(SelectionTotal {
            cells: sel.clone(),
            rows_of: self.rows_of,
            total: t,
        });
        t
    }

    /// The rows of `run` grouped as chosen, sorted.
    fn refresh(&mut self, doc: &Document, run: &Arc<VolumeRun>) {
        let key = (Arc::as_ptr(run) as usize, self.by);
        if self.rows_of == Some(key) {
            return;
        }
        let m = &doc.model;
        // Cells first: their masses add up into materials and universes.
        self.cells.clear();
        let atomic_mass: HashMap<i64, f64> = doc
            .looks
            .materials
            .iter()
            .map(|l| (l.id, l.mix.atomic_mass))
            .collect();
        let mut by_material_sum: HashMap<i64, (f64, f64, Option<f64>)> = HashMap::new();
        let mut by_universe_sum: HashMap<i64, (f64, f64, Option<f64>)> = HashMap::new();
        let add = |acc: &mut (f64, f64, Option<f64>), v: f64, s: f64, ms: Option<f64>| {
            acc.0 += v;
            acc.1 += s * s;
            acc.2 = match (acc.2, ms) {
                (Some(a), Some(b)) => Some(a + b),
                _ => None,
            };
        };
        let warning = |o: CellOutcome| match (o, run.partial) {
            (CellOutcome::Unconverged, false) => Some("did not reach the target σ"),
            (CellOutcome::Unseen, false) => {
                Some("no line met it: its volume is small, not known to be 0")
            }
            (CellOutcome::Unconverged, true) => Some("not at the target σ yet"),
            (CellOutcome::Unseen, true) => Some("no line has met it yet"),
            _ => None,
        };
        let mut by_material: HashMap<i64, Option<&'static str>> = HashMap::new();
        let mut by_universe: HashMap<i64, Option<&'static str>> = HashMap::new();
        let mut rows: Vec<Row> = Vec::new();
        for g in &run.cells {
            let Some(c) = m.cell_index(g.id) else {
                continue;
            };
            let cell = m.cell(c);
            let (v, s) = (g.volume, g.sigma);
            let ms = if cell.material == 0 {
                Some(0.0)
            } else {
                grams_per_cm3(
                    cell.density,
                    atomic_mass.get(&cell.material).copied().unwrap_or(0.0),
                )
                .map(|rho| rho * v)
            };
            self.cells.insert(g.id, (v, s * s, ms));
            let w = warning(g.outcome);
            if cell.fill.is_none() {
                add(
                    by_material_sum
                        .entry(cell.material)
                        .or_insert((0.0, 0.0, Some(0.0))),
                    v,
                    s,
                    ms,
                );
                let e = by_material.entry(cell.material).or_insert(None);
                *e = e.or(w);
            }
            let u = m.units()[cell.unit as usize].universe;
            add(
                by_universe_sum.entry(u).or_insert((0.0, 0.0, Some(0.0))),
                v,
                s,
                ms,
            );
            let e = by_universe.entry(u).or_insert(None);
            *e = e.or(w);
            if self.by == GroupBy::Cell {
                rows.push(Row {
                    query: Some(Query::cell(g.id)),
                    name: format!("cell {}", g.id),
                    volume: v,
                    sigma: s,
                    mass: ms,
                    warning: w,
                });
            }
        }
        let sums = match self.by {
            GroupBy::Material => Some((&by_material_sum, &by_material)),
            GroupBy::Universe => Some((&by_universe_sum, &by_universe)),
            _ => None,
        };
        for (&n, &(v, s2, ms)) in sums.iter().flat_map(|s| s.0.iter()) {
            let (query, name) = match self.by {
                GroupBy::Material if n == 0 => (Query::Void, "void".to_string()),
                GroupBy::Material => (Query::material(n), format!("m{n}")),
                _ => (Query::universe(n), format!("u={n}")),
            };
            rows.push(Row {
                query: Some(query),
                name,
                volume: v,
                sigma: s2.sqrt(),
                mass: ms,
                warning: sums.and_then(|s| s.1.get(&n).copied().flatten()),
            });
        }
        // Problems stay as their labels, named in full.
        for (name, v, s) in &run.problems {
            rows.push(Row {
                query: None,
                name: name.clone(),
                volume: *v,
                sigma: *s,
                mass: None,
                warning: None,
            });
        }
        let (s, desc) = self.sort;
        rows.sort_by(|a, b| {
            let o = match s {
                Sort::Name => a.name.cmp(&b.name),
                Sort::Volume => a.volume.total_cmp(&b.volume),
                Sort::Sigma => (a.sigma / a.volume).total_cmp(&(b.sigma / b.volume)),
                Sort::Mass => a.mass.unwrap_or(-1.0).total_cmp(&b.mass.unwrap_or(-1.0)),
            };
            if desc {
                o.reverse()
            } else {
                o
            }
        });
        self.rows = rows;
        self.rows_of = Some(key);
    }

    /// Write the table as CSV next to the input: what was written.
    fn export(&self, doc: &Document) -> Result<String, String> {
        let Some(input) = &doc.input else {
            return Err("no input file to write next to".into());
        };
        let by = match self.by {
            GroupBy::Cell => "cell",
            GroupBy::Material => "material",
            GroupBy::Universe => "universe",
            GroupBy::Label => "label",
        };
        let mut path = input.clone().into_os_string();
        path.push(format!(".volumes-by-{by}.csv"));
        let path = std::path::PathBuf::from(path);
        let mut text = format!("{by},volume_cm3,sigma_cm3,mass_g\n");
        for r in &self.rows {
            let name = r.name.replace(',', ";");
            let m = r.mass.map_or(String::new(), |m| m.to_string());
            text += &format!("{name},{},{},{m}\n", r.volume, r.sigma);
        }
        match std::fs::write(&path, text) {
            Ok(()) => Ok(format!("wrote {}", path.display())),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}

fn volume(v: f64) -> String {
    format!("{v:.4e}")
}

fn mass(g: f64) -> String {
    if g >= 1e6 {
        format!("{:.4} t", g / 1e6)
    } else if g >= 1e3 {
        format!("{:.4} kg", g / 1e3)
    } else {
        format!("{g:.4} g")
    }
}
