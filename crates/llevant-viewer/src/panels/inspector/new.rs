//! New cards made in the viewer (decision 0012, phase 8): a cell (its
//! number, material, density, geometry with the editor's help, and
//! parameters; taken out of the cell it is in if asked), a surface (its
//! type and coefficients, named), a material (its entries, or another's to
//! start from). The card is shown as it will be written, and the cell or
//! surface drawn on the slice before Apply ([`crate::marks`]); Apply adds
//! it at the end of its block ([`Command::AddCard`]), one edit, one Undo.

use std::sync::Arc;

use llevant_scene::source::{coefficient_names, wrap_long_lines, CardKind};

use super::fields::{material_picker, name_text, usual_density};
use super::geometry::GeometryEditor;
use crate::command::Command;
use crate::document::Document;
use crate::marks::{MarkRequest, Preview};

/// Surface types offered, with the coefficients they take.
const SURFACES: [(&str, &str); 14] = [
    ("so", "R"),
    ("s", "x y z R"),
    ("px", "x"),
    ("py", "y"),
    ("pz", "z"),
    ("p", "A B C D"),
    ("cx", "R"),
    ("cy", "R"),
    ("cz", "R"),
    ("c/z", "x y R"),
    ("kz", "z t²"),
    ("rpp", "xmin xmax ymin ymax zmin zmax"),
    ("rcc", "x y z hx hy hz R"),
    ("box", "x y z a1 a2 a3 b1 b2 b3 c1 c2 c3"),
];

pub struct NewForm {
    kind: CardKind,
    number: String,
    // A cell.
    material: i64,
    filter: String,
    /// As MCNP takes it (negative: g/cm³).
    density: f64,
    geometry: GeometryEditor,
    params: String,
    /// The cell it is cut out of (`#n` added to that one's geometry).
    carve: Option<i64>,
    // A surface.
    mnemonic: String,
    coeffs: String,
    comment: String,
    // A material.
    entries: String,
    /// The model the card was sent from: done when another is shown.
    applied: Option<usize>,
    error: Option<String>,
    /// The pass it was last shown in (what it draws is marked while shown).
    shown: u64,
}

impl NewForm {
    pub fn new(doc: &Document, kind: CardKind) -> NewForm {
        let source = doc.source.as_ref();
        let number = source.map_or(1, |s| s.next_free(kind));
        // The cell of the point inspected: where a new cell is likely to be
        // (one of the input: not the void around a universe opened on its
        // own).
        let carve = doc
            .focus
            .as_ref()
            .and_then(|p| p.cell())
            .map(|c| doc.model.cell_id(c))
            .filter(|&id| source.is_some_and(|s| s.cell(id).is_some()));
        let keys = source.map(|s| s.per_cell_keys()).unwrap_or_default();
        let on_cards = |k: &str| source.is_some() && !keys.iter().any(|x| x == k);
        let mut params = Vec::new();
        if on_cards("imp:n") {
            params.push("imp:n=1".to_string());
        }
        // In the universe of the cell it is cut out of, else in the one
        // opened on its own.
        let universe = carve
            .and_then(|id| doc.model.cell_index(id))
            .map(|c| doc.model.units()[doc.model.cell(c).unit as usize].universe)
            .or(doc.universe)
            .filter(|&u| u != 0);
        if let (Some(u), true) = (universe, on_cards("u")) {
            params.push(format!("u={u}"));
        }
        NewForm {
            kind,
            number: number.to_string(),
            material: 0,
            filter: String::new(),
            density: -1.0,
            geometry: GeometryEditor::new(""),
            params: params.join(" "),
            carve: if kind == CardKind::Cell { carve } else { None },
            mnemonic: "so".into(),
            coeffs: String::new(),
            comment: String::new(),
            entries: String::new(),
            applied: None,
            error: None,
            shown: 0,
        }
    }

    pub fn kind(&self) -> CardKind {
        self.kind
    }

    /// Whether it still stands (its card has not landed).
    pub fn current(&self, doc: &Document) -> bool {
        self.applied
            .is_none_or(|m| m == Arc::as_ptr(&doc.model) as usize)
    }

    pub fn edit_failed(&mut self, why: &str) {
        if self.applied.take().is_some() {
            self.error = Some(why.to_string());
        }
    }

    fn id(&self) -> Option<i64> {
        self.number.trim().parse().ok().filter(|n| *n >= 0)
    }

    /// The card as it will be written; or what is missing.
    fn card(&self) -> Result<String, String> {
        let id = self.id().ok_or("the number is not a whole number")?;
        let text = match self.kind {
            CardKind::Cell => {
                let g = self
                    .geometry
                    .text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                if g.is_empty() {
                    return Err("give its geometry".into());
                }
                let head = if self.material == 0 {
                    format!("{id} 0")
                } else {
                    format!("{id} {} {}", self.material, self.density)
                };
                format!("{head} {g} {}", self.params.trim())
            }
            CardKind::Surface => {
                let c = self.coeffs.split_whitespace().collect::<Vec<_>>();
                if c.is_empty() {
                    return Err("give its coefficients".into());
                }
                if let Some(bad) = c.iter().find(|w| w.parse::<f64>().is_err()) {
                    return Err(format!("`{bad}` is not a number"));
                }
                let note = match self.comment.trim() {
                    "" => String::new(),
                    n => format!(" $ {n}"),
                };
                format!("{id} {} {}{note}", self.mnemonic.trim(), c.join(" "))
            }
            CardKind::Material => {
                let e = self.entries.split_whitespace().collect::<Vec<_>>();
                if e.is_empty() {
                    return Err("give its nuclides and fractions".into());
                }
                format!("m{id} {}", e.join(" "))
            }
            _ => return Err("not a card made here".into()),
        };
        Ok(wrap_long_lines(&format!("{}\n", text.trim_end())))
    }

    /// The new cell or surface drawn on the slice, while shown in `pass`.
    pub fn marks(&self, pass: u64) -> Option<MarkRequest> {
        if self.shown != pass {
            return None;
        }
        let card = self.card().ok()?;
        let frame = self.carve.unwrap_or(0);
        match self.kind {
            CardKind::Cell => Some(MarkRequest {
                cell: frame,
                surfaces: self.geometry.surfaces(),
                preview: Some(Preview::Cell {
                    id: self.id()?,
                    geometry: self.geometry.text.clone(),
                }),
                at: None,
            }),
            CardKind::Surface => Some(MarkRequest {
                cell: frame,
                surfaces: Vec::new(),
                preview: Some(Preview::Input {
                    text: format!("new surface\n1 0 -{}\n\n{card}\n", self.id()?),
                    cell: 1,
                }),
                at: None,
            }),
            _ => None,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
        self.shown = ui.ctx().cumulative_pass_nr();
        let Some(source) = doc.source.clone() else {
            ui.label(egui::RichText::new("the input's cards are being read").weak());
            return;
        };
        let pending = self.applied.is_some();
        ui.heading(match self.kind {
            CardKind::Cell => "New cell",
            CardKind::Surface => "New surface",
            _ => "New material",
        });
        ui.add_enabled_ui(!pending, |ui| {
            ui.horizontal(|ui| {
                ui.label("Number");
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.number)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(90.0),
                );
                name_text(&r, "Number of the new card");
            });
            match self.kind {
                CardKind::Cell => self.cell_ui(ui, doc),
                CardKind::Surface => self.surface_ui(ui),
                _ => self.material_ui(ui, doc),
            }
        });
        let id = self.id();
        let in_use = id.is_some_and(|n| source.card(self.kind, n).is_some());
        let card = self.card();
        let flagged = self.kind == CardKind::Cell && !self.geometry.flags(&source).is_empty();
        let problem = match (&card, in_use) {
            (Err(e), _) => Some(e.clone()),
            (_, true) => Some(format!(
                "{} {} is in use",
                self.kind.name(),
                id.unwrap_or(0)
            )),
            _ if flagged => Some("the geometry has words flagged".into()),
            _ => None,
        };
        // As it will be written (and the cell it is cut out of).
        if let (Ok(t), None) = (&card, &problem) {
            let mut shown = vec![(
                format!("new {} {}", self.kind.name(), id.unwrap_or(0)),
                String::new(),
                t.clone(),
            )];
            if let (Some(p), Some(n)) = (self.carve, id) {
                if let Ok(c) = source.cell_cut(p, n) {
                    shown.push((
                        format!("cell {p}"),
                        source.cell(p).map(|c| c.text).unwrap_or_default(),
                        c,
                    ));
                }
            }
            super::diff::cards(ui, &shown, 0);
            super::diff::after_apply(ui);
        }
        ui.horizontal(|ui| {
            let ok = doc.input_editable.is_ok() && problem.is_none() && !pending;
            let r = ui.add_enabled(ok, egui::Button::new("Apply"));
            crate::app::frame::named(&r, "Add the card");
            let r = match (&doc.input_editable, &problem) {
                (Err(why), _) => r.on_disabled_hover_text(*why),
                (_, Some(p)) => r.on_disabled_hover_text(p.as_str()),
                _ => r.on_hover_text("Add the card at the end of its block and compile again (not saved; Undo edit takes it out)"),
            };
            if r.clicked() {
                if let (Ok(text), Some(id)) = (card.clone(), id) {
                    out.push(Command::AddCard {
                        kind: self.kind,
                        id,
                        text,
                        carve: self.carve.filter(|_| self.kind == CardKind::Cell),
                    });
                    self.applied = Some(Arc::as_ptr(&doc.model) as usize);
                    self.error = None;
                }
            }
            if let Some(p) = &problem {
                ui.label(egui::RichText::new(p.as_str()).weak());
            }
            if pending {
                crate::logo::small_spinner(ui);
            }
        });
        if let Some(e) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
    }

    fn cell_ui(&mut self, ui: &mut egui::Ui, doc: &Document) {
        ui.horizontal(|ui| {
            ui.label("Material");
            let before = self.material;
            if material_picker(
                ui,
                doc,
                "new cell material",
                &mut self.material,
                &mut self.filter,
            ) && before == 0
            {
                if let Some(d) = usual_density(doc, self.material) {
                    self.density = d;
                }
            }
        });
        if self.material != 0 {
            ui.horizontal(|ui| {
                ui.label("Density");
                let grams = self.density < 0.0;
                let mut v = self.density.abs();
                let speed = (v * 0.002).max(1e-4);
                if ui
                    .add(
                        egui::DragValue::new(&mut v)
                            .speed(speed)
                            .range(0.0..=f64::INFINITY)
                            .max_decimals(6),
                    )
                    .changed()
                    && v > 0.0
                {
                    self.density = if grams { -v } else { v };
                }
                for (g, name) in [(true, "g/cm³"), (false, "at/b·cm")] {
                    if ui
                        .add(egui::Button::selectable(grams == g, name).small())
                        .clicked()
                    {
                        self.density = if g { -v } else { v };
                    }
                }
            });
        }
        ui.label(egui::RichText::new("GEOMETRY").small().weak());
        let _ = self.geometry.ui(ui, doc, "new cell");
        ui.horizontal(|ui| {
            ui.label("Parameters");
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.params)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("imp:n=1 u=3 …")
                    .desired_width(180.0),
            );
            name_text(&r, "Parameters of the new cell");
        });
        if let Some(p) = self.carve {
            let id = self.id().unwrap_or(0);
            match doc.source.as_ref().map(|s| s.cell_cut(p, id)) {
                Some(Err(why)) => {
                    ui.label(
                        egui::RichText::new(format!("Not taken out of cell {p}: {why}")).weak(),
                    );
                    self.carve = None;
                }
                _ => {
                    let mut on = true;
                    ui.checkbox(&mut on, format!("Take it out of cell {p} (#n added to its geometry)"))
                        .on_hover_text("So that the two do not overlap: the new cell is cut out of the one it lies in");
                    if !on {
                        self.carve = None;
                    }
                }
            }
        }
    }

    fn surface_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Type");
            for (m, _) in SURFACES {
                if ui
                    .add(
                        egui::Button::selectable(
                            self.mnemonic.eq_ignore_ascii_case(m),
                            m.to_uppercase(),
                        )
                        .small(),
                    )
                    .clicked()
                {
                    self.mnemonic = m.to_string();
                }
            }
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.mnemonic)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(50.0),
            );
            name_text(&r, "Type of the new surface");
            r.on_hover_text("Any MCNP surface type");
        });
        let count = self.coeffs.split_whitespace().count();
        let names = coefficient_names(self.mnemonic.trim(), count.max(1));
        let hint = SURFACES
            .iter()
            .find(|(m, _)| m.eq_ignore_ascii_case(self.mnemonic.trim()))
            .map_or_else(|| names.join(" "), |(_, h)| h.to_string());
        ui.horizontal(|ui| {
            ui.label("Coefficients");
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.coeffs)
                    .font(egui::TextStyle::Monospace)
                    .hint_text(hint)
                    .desired_width(200.0),
            );
            name_text(&r, "Coefficients of the new surface");
        });
        ui.horizontal(|ui| {
            ui.label("Comment");
            ui.add(egui::TextEdit::singleline(&mut self.comment).desired_width(200.0));
        });
    }

    fn material_ui(&mut self, ui: &mut egui::Ui, doc: &Document) {
        ui.horizontal(|ui| {
            ui.label("Start from");
            let mut from = 0;
            if material_picker(ui, doc, "new material from", &mut from, &mut self.filter)
                && from != 0
            {
                if let Some(f) = doc.source.as_ref().and_then(|s| s.material_fields(from)) {
                    let mut t: Vec<String> =
                        f.entries.iter().map(|(z, x)| format!("{z} {x}")).collect();
                    t.extend(f.keywords.iter().map(|(k, v)| format!("{k}={v}")));
                    self.entries = t.join("\n");
                }
            }
        });
        let r = ui.add(
            egui::TextEdit::multiline(&mut self.entries)
                .font(egui::TextStyle::Monospace)
                .hint_text("1001.80c 2\n8016.80c 1   (negative: by mass)")
                .desired_rows(4)
                .desired_width(f32::INFINITY),
        );
        name_text(&r, "Entries of the new material");
    }
}
