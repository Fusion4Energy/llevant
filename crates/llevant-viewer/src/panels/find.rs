//! Three of Explore's panels (decision 0012), one tab each: **Find**, a rule
//! box whose matches are listed grouped by universe or by material, with
//! what can be done to them all, the rules saved with the view and the
//! selections made last; **Materials**, the legend of the materials;
//! **Envelopes**, the level-0 cells, each with what fills it. Rows are drawn only where the
//! list is scrolled to, so the largest models scroll smoothly. Names are
//! links ([`crate::context`]); `›` opens a cell's page in the inspector.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use llevant_render::colours::Paint;
use llevant_scene::query::Query;

use crate::command::{Command, Edit};
use crate::context::{link, material_link, universe_link};
use crate::document::Document;
use crate::panels::inspector::Subject;
use crate::panels::materials::MaterialsPanel;
use crate::scene::SavedRule;
use crate::selection::SelectMode;
use crate::{icons, theme};

const ROW: f32 = 20.0;
/// Cells listed for a rule at most.
const LISTED: usize = 200_000;
/// Recent selections listed.
const RECENT: usize = 8;
/// Groups open at first when a rule has at most this many cells.
const OPEN_BELOW: usize = 300;

/// What the panel lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Find,
    Materials,
    Envelopes,
}

/// How the matches are grouped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grouping {
    Universe,
    Material,
    None,
}

/// The cells a rule selects, sorted by number, and grouped.
struct Matches {
    query: Query,
    cells: Vec<u32>,
    groups: Vec<(i64, Vec<u32>)>,
}

/// A row of the list.
enum Row {
    Group(usize),
    Cell(u32),
}

pub struct FindPanel {
    materials: MaterialsPanel,
    rule: String,
    matches: Option<Result<Matches, String>>,
    grouping: Grouping,
    /// Groups opened or closed by hand (their key), against the default.
    toggled: HashSet<i64>,
    /// The name a rule is saved under, as typed.
    save_name: String,
    /// The model the matches are of (its address).
    model: usize,
    /// The cells `shown` gave for the matches (their address): the view
    /// changing them finds again.
    shown: usize,
}

impl Default for FindPanel {
    fn default() -> FindPanel {
        FindPanel {
            materials: MaterialsPanel::default(),
            rule: String::new(),
            matches: None,
            grouping: Grouping::Universe,
            toggled: HashSet::new(),
            save_name: String::new(),
            model: 0,
            shown: 0,
        }
    }
}

impl FindPanel {
    /// Show `rule`'s matches.
    pub fn set_rule(&mut self, rule: &str) {
        self.rule = rule.to_string();
        self.matches = None;
        self.toggled.clear();
    }

    /// The panel showing `mode`.
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document, mode: Mode, out: &mut Vec<Command>) {
        let key = Arc::as_ptr(&doc.model) as usize;
        if key != self.model {
            self.model = key;
            self.matches = None;
        }
        match mode {
            Mode::Find => {
                super::isolating(ui, doc, out);
                self.find(ui, doc, out)
            }
            Mode::Materials => self.materials.ui(ui, doc, out),
            Mode::Envelopes => {
                super::isolating(ui, doc, out);
                envelopes(ui, doc, out)
            }
        }
    }

    fn find(&mut self, ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
        let r = ui.add(
            egui::TextEdit::singleline(&mut self.rule)
                .hint_text(format!(
                    "{}  a rule: u 320 & m 33, c 10 12, s 4120",
                    icons::SEARCH
                ))
                .desired_width(f32::INFINITY),
        );
        theme::name(&r, "find rule");
        if r.changed() {
            self.matches = None;
            self.toggled.clear();
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if self.rule.trim().is_empty() {
            saved_and_recent(ui, doc, &mut self.rule, &mut self.matches, out);
            return;
        }
        let grouping = self.grouping;
        let m = &doc.model;
        // With `shown`: found again when the view changes it, the matches
        // before kept in the meantime.
        let parsed = self.rule.parse::<Query>();
        let uses_shown = parsed.as_ref().is_ok_and(|q| q.uses_shown());
        let shown = if uses_shown { doc.shown_cells() } else { None };
        let waiting = uses_shown && shown.is_none();
        if let Some(s) = &shown {
            if Arc::as_ptr(s) as usize != self.shown {
                self.matches = None;
            }
        }
        if self.matches.is_none() && !waiting {
            if let Some(s) = &shown {
                self.shown = Arc::as_ptr(s) as usize;
            }
            self.matches = Some((|| {
                let q = parsed?;
                let set = q
                    .resolve_with(m, &doc.index, shown.as_deref())
                    .map_err(|e| e.to_string())?;
                let mut cells: Vec<u32> = set.iter().take(LISTED).collect();
                cells.sort_by_key(|&c| m.cell_id(c));
                let groups = group(doc, &cells, grouping);
                Ok(Matches {
                    query: q,
                    cells,
                    groups,
                })
            })());
        }
        if waiting {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(theme::data("working out what the 3D view shows…"));
            });
        }
        let Some(matches) = self.matches.as_mut() else {
            return;
        };
        let matches = match matches {
            Ok(v) => v,
            Err(e) => {
                ui.colored_label(ui.visuals().warn_fg_color, e.as_str());
                return;
            }
        };
        let q = matches.query.clone();
        if enter {
            out.push(Command::Select(q.clone(), SelectMode::Replace));
        }
        let n = matches.cells.len();
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::data(format!(
                "{n}{} {}",
                if n == LISTED { "+" } else { "" },
                if n == 1 { "cell" } else { "cells" }
            )));
            if ui
                .small_button("Select all")
                .on_hover_text("Enter in the box does the same")
                .clicked()
            {
                out.push(Command::Select(q.clone(), SelectMode::Replace));
            }
            let r = ui.add_enabled(
                doc.input_editable.is_ok(),
                egui::Button::new("Edit").small(),
            );
            crate::app::frame::named(&r, "Edit the cells found");
            let r = match doc.input_editable {
                Ok(()) => r.on_hover_text(
                    "Select them and change their material, density, parameters or numbers together",
                ),
                Err(why) => r.on_disabled_hover_text(why),
            };
            if r.clicked() {
                out.push(Command::Select(q.clone(), SelectMode::Replace));
                out.push(Command::EditSelection);
            }
            if ui
                .small_button(icons::HIDE)
                .on_hover_text("Hide them")
                .clicked()
            {
                out.push(Edit::Hide(q.clone()).into());
            }
            if ui
                .small_button(icons::ISOLATE)
                .on_hover_text("Isolate them")
                .clicked()
            {
                out.push(Edit::Isolate(Some(q.clone())).into());
            }
            if ui
                .small_button(icons::GHOST)
                .on_hover_text("Ghost them: shells over what is behind")
                .clicked()
            {
                out.push(Edit::Ghost(q.clone()).into());
            }
            let rule = q.to_string();
            let saved = doc.scene.rules.iter().any(|r| r.rule == rule);
            if !saved {
                theme::settings_button(ui, egui::RichText::new("☆ Save").small(), |ui| {
                    ui.label("Save this rule as");
                    if self.save_name.is_empty() {
                        self.save_name = rule.clone();
                    }
                    let r = ui.text_edit_singleline(&mut self.save_name);
                    theme::name(&r, "saved rule name");
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Save").clicked() || enter {
                        let name = std::mem::take(&mut self.save_name);
                        out.push(
                            Edit::SaveRule(SavedRule {
                                name: if name.trim().is_empty() {
                                    rule.clone()
                                } else {
                                    name
                                },
                                rule: rule.clone(),
                            })
                            .into(),
                        );
                        ui.close();
                    }
                })
                .on_hover_text("Keep this rule, with a name, in the view file");
            }
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("by").weak());
            let before = self.grouping;
            ui.selectable_value(&mut self.grouping, Grouping::Universe, "universe");
            ui.selectable_value(&mut self.grouping, Grouping::Material, "material");
            ui.selectable_value(&mut self.grouping, Grouping::None, "number");
            if self.grouping != before {
                matches.groups = group(doc, &matches.cells, self.grouping);
                self.toggled.clear();
            }
        });
        // The rows: each group's header, and its cells when open.
        let open_default = n <= OPEN_BELOW;
        let mut rows: Vec<Row> = Vec::new();
        if self.grouping == Grouping::None {
            rows.extend(matches.cells.iter().map(|&c| Row::Cell(c)));
        } else {
            for (k, (key, cells)) in matches.groups.iter().enumerate() {
                rows.push(Row::Group(k));
                if open_default != self.toggled.contains(key) {
                    rows.extend(cells.iter().map(|&c| Row::Cell(c)));
                }
            }
        }
        let mut toggle = None;
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show_rows(ui, ROW, rows.len(), |ui, range| {
                for row in &rows[range] {
                    match *row {
                        Row::Group(k) => {
                            let (key, cells) = &matches.groups[k];
                            let open = open_default != self.toggled.contains(key);
                            ui.horizontal(|ui| {
                                let arrow = if open { icons::OPENED } else { icons::CLOSED };
                                if ui
                                    .add(egui::Button::new(arrow).frame(false).small())
                                    .clicked()
                                {
                                    toggle = Some(*key);
                                }
                                match self.grouping {
                                    Grouping::Universe => universe_link(ui, doc, *key, out),
                                    _ => material_link(ui, doc, *key, out),
                                }
                                ui.label(theme::data(format!("{}", cells.len())).weak());
                                if self.grouping == Grouping::Universe {
                                    inspect(ui, Subject::Universe(*key), out);
                                } else if *key != 0 {
                                    inspect(ui, Subject::Material(*key), out);
                                }
                            });
                        }
                        Row::Cell(c) => {
                            let r = ui
                                .horizontal(|ui| {
                                    ui.add_space(if self.grouping == Grouping::None {
                                        0.0
                                    } else {
                                        18.0
                                    });
                                    cell_row(ui, doc, c, out);
                                })
                                .response;
                            if doc.selection.cells.contains(c) {
                                mark_selected(ui, r.rect);
                            }
                        }
                    }
                }
            });
        if let Some(k) = toggle {
            if !self.toggled.remove(&k) {
                self.toggled.insert(k);
            }
        }
    }
}

/// The rules saved with the view and the selections made last, each a
/// click away from its matches.
fn saved_and_recent(
    ui: &mut egui::Ui,
    doc: &Document,
    rule: &mut String,
    matches: &mut Option<Result<Matches, String>>,
    out: &mut Vec<Command>,
) {
    let heading = |ui: &mut egui::Ui, t: &str| {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(t.to_uppercase())
                .size(11.5)
                .strong()
                .color(ui.visuals().weak_text_color()),
        );
    };
    let mut pick = |r: String| {
        *rule = r;
        *matches = None;
    };
    heading(ui, "Saved");
    if doc.scene.rules.is_empty() {
        ui.label(
            egui::RichText::new("Rules saved here (☆ Save) are kept in the view file.").weak(),
        );
    }
    for (k, s) in doc.scene.rules.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(icons::with("☆", &s.name)).frame(false))
                .on_hover_text(format!("{}: list its cells", s.rule))
                .clicked()
            {
                pick(s.rule.clone());
            }
            // The remove button first (right to left), so a long rule is
            // cut short in the room left rather than widening the side.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button(icons::REMOVE)
                    .on_hover_text("Forget this rule")
                    .clicked()
                {
                    out.push(Edit::RemoveRule(k).into());
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(theme::data(&s.rule).weak()).truncate())
                        .on_hover_text(theme::data(&s.rule));
                });
            });
        });
    }
    let recent = doc.selection.recent(RECENT);
    if !recent.is_empty() {
        heading(ui, "Recent");
        for q in recent {
            // A selection of many cells is a long `c …`: cut short, whole
            // on hover, so that it never widens the side.
            let text = q.to_string();
            if ui
                .add(
                    egui::Button::new(theme::data(icons::with(icons::SELECT, &text)))
                        .frame(false)
                        .truncate(),
                )
                .on_hover_text(format!("{text}\n\nList its cells"))
                .clicked()
            {
                pick(text.clone());
            }
        }
    }
    heading(ui, "Rules");
    ui.label(
        egui::RichText::new(
            "c 10 40-55 · u 320 · m 33 · s 4120 · void · filled · fill 320 (the cells \
             universe 320 fills) · in u 320 (with what they hold) · all · \
             12 < 100 < 5 (12 inside 100 inside 5, as MCNP and Copy cell path write it) · \
             $ BSV or label \"HNB - BSV\" (cells whose $ comments contain it, any case) · \
             a & b (both) · a | b or a, b (either) · !a (all but) · ( )",
        )
        .weak(),
    );
}

/// `cells` grouped by universe or by material, largest group first.
fn group(doc: &Document, cells: &[u32], g: Grouping) -> Vec<(i64, Vec<u32>)> {
    let m = &doc.model;
    let mut map: BTreeMap<i64, Vec<u32>> = BTreeMap::new();
    for &c in cells {
        let cell = m.cell(c);
        let key = match g {
            Grouping::Universe => m.units()[cell.unit as usize].universe,
            Grouping::Material => {
                if cell.fill.is_some() {
                    continue;
                }
                cell.material
            }
            Grouping::None => return Vec::new(),
        };
        map.entry(key).or_default().push(c);
    }
    let mut v: Vec<(i64, Vec<u32>)> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    v
}

/// The level-0 cells, each with what it holds or is made of.
fn envelopes(ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
    let m = &doc.model;
    let Some(top) = doc.index.unit(m.units()[0].universe) else {
        return;
    };
    let cells = &m.units()[top as usize].cells;
    ui.label(
        egui::RichText::new(format!(
            "{} cells at level 0; what fills them opens in the inspector (›)",
            cells.len()
        ))
        .weak(),
    );
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show_rows(ui, ROW, cells.len(), |ui, range| {
            for &c in &cells[range] {
                let r = ui.horizontal(|ui| cell_row(ui, doc, c, out)).response;
                if doc.selection.cells.contains(c) {
                    mark_selected(ui, r.rect);
                }
            }
        });
}

/// A cell's row: eye, swatch, name, what it holds or is made of, `›`.
fn cell_row(ui: &mut egui::Ui, doc: &Document, c: u32, out: &mut Vec<Command>) {
    let m = &doc.model;
    let cell = m.cell(c);
    let q = Query::cell(cell.id);
    let e = crate::seen::cell_eye(m, &doc.index, &doc.scene, &doc.seen, c);
    super::eye(
        ui,
        &e,
        Edit::Hide(q.clone()).into(),
        "it (with what it holds)",
        out,
    );
    let swatch = Paint::new(m, &doc.looks, doc.scene.colour_by.into()).cell_colour(c);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().rect_filled(
        rect,
        2.0,
        egui::Color32::from_rgb(swatch[0], swatch[1], swatch[2]),
    );
    link(ui, doc, &q, format!("cell {}", cell.id), out);
    if cell.fill.is_some() {
        let fills = doc.index.fills(c);
        match fills {
            [k] => {
                let unit = &m.units()[*k as usize];
                universe_link(ui, doc, unit.universe, out);
                ui.label(egui::RichText::new(format!("{} cells", unit.cells.len())).weak());
            }
            _ => {
                ui.label(
                    egui::RichText::new(format!("lattice of {} universes", fills.len())).weak(),
                );
            }
        }
    } else {
        material_link(ui, doc, cell.material, out);
    }
    if cell.imp_zero {
        ui.label(egui::RichText::new("imp 0").weak());
    }
    if let Some(name) = comment(doc, cell.id) {
        ui.label(egui::RichText::new(name).weak());
    }
    inspect(ui, Subject::Cell(cell.id), out);
}

/// The comment on cell `id`'s card (`$ …` on its first line).
fn comment(doc: &Document, id: i64) -> Option<String> {
    let card = doc.source.as_ref()?.cell(id)?;
    let (_, c) = card.text.lines().next()?.split_once('$')?;
    let c = c.trim();
    (!c.is_empty()).then(|| c.to_string())
}

/// The `›` that opens `s` in the inspector.
fn inspect(ui: &mut egui::Ui, s: Subject, out: &mut Vec<Command>) {
    if ui
        .add(egui::Button::new(icons::CLOSED).small().frame(false))
        .on_hover_text(format!("Inspect {}", s.title()))
        .clicked()
    {
        out.push(Command::Inspect(s));
    }
}

fn mark_selected(ui: &egui::Ui, rect: egui::Rect) {
    let r = egui::Rect::from_min_max(
        egui::pos2(ui.clip_rect().min.x, rect.min.y),
        egui::pos2(ui.clip_rect().min.x + 3.0, rect.max.y),
    );
    ui.painter().rect_filled(r, 0.0, theme::SELECTION);
}
