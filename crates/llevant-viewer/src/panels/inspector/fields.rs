//! Cards edited by their fields (decision 0012, phase 8): a cell's
//! material, density and parameters; a material's nuclides by element,
//! its fractions converted between atoms and mass, normalised, mixed from
//! other materials, compared with another. Apply writes the card with only
//! the words changed ([`llevant_scene::source::Source::cell_with`] and the
//! like) as [`Command::EditCard`]: compiled again, not saved, one Undo.
//!
//! The fields are always shown on a cell's and a material's page (they
//! are its properties), Apply and Discard once something changed. Changes
//! not applied are kept as a draft when another card is shown
//! ([`Forms`]), listed at the top of the inspector.

use std::collections::BTreeMap;

use llevant_scene::materials::{element_mass_fractions, zaid_mass};
use llevant_scene::source::{CardKind, CellChange, CellFields, Entry, MaterialFields};

use super::{section, Subject};
use crate::command::Command;
use crate::document::Document;
use crate::{icons, theme};

/// Parameters offered when adding one to a cell.
const CELL_KEYS: [&str; 16] = [
    "imp:n", "imp:p", "imp:e", "u", "fill", "*fill", "trcl", "*trcl", "lat", "vol", "tmp", "pwt",
    "ext:n", "fcl:n", "elpt:n", "nonu",
];

/// Keywords offered when adding one to a material.
const MATERIAL_KEYS: [&str; 10] = [
    "nlib", "plib", "pnlib", "elib", "hlib", "cond", "gas", "estep", "hstep", "refi",
];

/// Significant digits fractions computed here are written with.
const DIGITS: usize = 6;

/// A card being edited by its fields.
pub struct Form {
    kind: CardKind,
    id: i64,
    /// The card's text when the form was opened (or last reloaded).
    base: String,
    body: Body,
    /// The text sent to be written (the form closes when the card reads so).
    applied: Option<String>,
    /// Why the last Apply could not be made.
    error: Option<String>,
    /// The pass it was last shown in (its surfaces are marked while shown).
    shown: u64,
    /// The card as Apply would write it, for the changes it was made for.
    preview: Option<(String, Result<String, String>)>,
}

enum Body {
    Cell(CellForm),
    Material(MaterialForm),
}

impl Form {
    /// Whether it edits the card of `kind` `id`.
    pub fn is(&self, kind: CardKind, id: i64) -> bool {
        self.kind == kind && self.id == id
    }

    /// The surfaces to mark in the views, while its form is shown (in pass
    /// `pass`): the one hovered in its geometry; those a cell's geometry
    /// names while it is being edited (focused or changed), with the cell
    /// as edited. Nothing for a cell only looked at: no trace starts.
    pub fn marks(&self, pass: u64) -> Option<crate::marks::MarkRequest> {
        let Body::Cell(c) = &self.body else {
            return None;
        };
        if self.shown != pass {
            return None;
        }
        if let Some(s) = c.hovered {
            return Some(crate::marks::MarkRequest {
                cell: self.id,
                surfaces: vec![s],
                preview: None,
                at: None,
            });
        }
        let changed = c.geometry.changed();
        (changed || c.geometry.focused).then(|| crate::marks::MarkRequest {
            cell: self.id,
            surfaces: c.geometry.surfaces(),
            preview: changed.then(|| crate::marks::Preview::Cell {
                id: self.id,
                geometry: c.geometry.text.clone(),
            }),
            at: None,
        })
    }

    /// Whether it holds anything not applied (or being applied).
    fn dirty(&self) -> bool {
        self.applied.is_some() || self.count() > 0
    }

    fn count(&self) -> usize {
        match &self.body {
            Body::Cell(c) => c.count(),
            Body::Material(m) => m.count(),
        }
    }
}

/// The inspector's forms: the one of the card shown, and drafts (forms
/// with changes) of cards shown before, until applied or discarded.
#[derive(Default)]
pub struct Forms {
    open: Option<Form>,
    drafts: Vec<Form>,
}

impl Forms {
    /// What the form shown marks ([`Form::marks`]).
    pub fn marks(&self, pass: u64) -> Option<crate::marks::MarkRequest> {
        self.open.as_ref().and_then(|f| f.marks(pass))
    }

    /// The last Apply did not land: its form ready to try again.
    pub fn edit_failed(&mut self, why: &str) {
        edit_failed(&mut self.open, why);
        for d in &mut self.drafts {
            if d.applied.take().is_some() {
                d.error = Some(why.to_string());
            }
        }
    }

    /// The form of card `kind` `id`, made when it is first shown (or taken
    /// back from the drafts); the one shown before kept as a draft if it
    /// has changes. A form without changes follows its card (an Undo, say).
    fn take(&mut self, doc: &Document, kind: CardKind, id: i64) -> Option<&mut Form> {
        let now = || {
            doc.source
                .as_ref()
                .and_then(|s| s.card(kind, id))
                .map(|c| c.text)
        };
        let here = self.open.as_ref().is_some_and(|f| f.is(kind, id));
        if !here {
            if let Some(f) = self.open.take().filter(Form::dirty) {
                self.drafts.push(f);
            }
            self.open = match self.drafts.iter().position(|f| f.is(kind, id)) {
                Some(k) => Some(self.drafts.remove(k)),
                None => open(doc, kind, id),
            };
        } else if self
            .open
            .as_ref()
            .is_some_and(|f| !f.dirty() && now().is_some_and(|n| n != f.base))
        {
            self.open = open(doc, kind, id);
        }
        self.open.as_mut()
    }

    /// The drafts of cards not shown: each named, with Show and Discard.
    /// Those whose card is gone or reads as applied are dropped.
    pub fn strip(&mut self, ui: &mut egui::Ui, doc: &Document, go: &mut Option<Subject>) {
        self.drafts.retain(|f| {
            let now = doc.source.as_ref().and_then(|s| s.card(f.kind, f.id));
            match (&f.applied, now) {
                (_, None) => false,
                (Some(a), Some(n)) => a.trim_end() != n.text.trim_end(),
                (None, Some(_)) => true,
            }
        });
        if self.drafts.is_empty() {
            return;
        }
        let mut discard = None;
        for (k, f) in self.drafts.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                let n = f.count();
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "{} {}: {n} {} not applied",
                        f.kind.name(),
                        f.id,
                        if n == 1 { "change" } else { "changes" }
                    ),
                );
                let r = ui.small_button("Show");
                crate::app::frame::named(&r, &format!("Show draft of {} {}", f.kind.name(), f.id));
                if r.clicked() {
                    *go = Some(match f.kind {
                        CardKind::Material => Subject::Material(f.id),
                        _ => Subject::Cell(f.id),
                    });
                }
                let r = ui.small_button("Discard");
                crate::app::frame::named(
                    &r,
                    &format!("Discard draft of {} {}", f.kind.name(), f.id),
                );
                if r.clicked() {
                    discard = Some(k);
                }
            });
        }
        if let Some(k) = discard {
            self.drafts.remove(k);
        }
        ui.separator();
    }
}

/// Open a form for the card of `kind` `id` (cells and materials).
pub fn open(doc: &Document, kind: CardKind, id: i64) -> Option<Form> {
    let source = doc.source.as_ref()?;
    let base = source.card(kind, id)?.text;
    let body = match kind {
        CardKind::Cell => Body::Cell(CellForm::new(
            source.cell_fields(id)?,
            source.per_cell_keys(),
        )),
        CardKind::Material => Body::Material(MaterialForm::new(source.material_fields(id)?)),
        _ => return None,
    };
    Some(Form {
        kind,
        id,
        base,
        body,
        applied: None,
        error: None,
        shown: 0,
        preview: None,
    })
}

/// The fields of the card of `kind` `id`, then (once something changed)
/// what Apply writes, Apply and Discard. False when there are none to show
/// (the input's cards not read yet, or this card not among them).
pub fn show(
    ui: &mut egui::Ui,
    doc: &Document,
    kind: CardKind,
    id: i64,
    forms: &mut Forms,
    go: &mut Option<Subject>,
    out: &mut Vec<Command>,
) -> bool {
    if forms.take(doc, kind, id).is_none() {
        return false;
    }
    let form = &mut forms.open;
    let Some(f) = form.as_mut() else {
        return false;
    };
    let now = doc
        .source
        .as_ref()
        .and_then(|s| s.card(kind, id))
        .map(|c| c.text);
    // Applied and written: done.
    if let (Some(a), Some(n)) = (&f.applied, &now) {
        if a.trim_end() == n.trim_end() {
            *form = None;
            return false;
        }
    }
    f.shown = ui.ctx().cumulative_pass_nr();
    let mut close = false;
    section(ui, ("fields", kind.name(), id), "Fields", true, |ui| {
        let changed_under = now.as_ref().is_some_and(|n| n != &f.base) && f.applied.is_none();
        if changed_under {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "The card changed meanwhile (an Undo or another edit).",
            );
            if ui.small_button("Start again from the card").clicked() {
                if let Some(fresh) = open(doc, kind, id) {
                    *f = fresh;
                }
            }
        }
        let numbered = match kind {
            CardKind::Cell => Some(llevant_scene::source::IdKind::Cell),
            CardKind::Material => Some(llevant_scene::source::IdKind::Material),
            _ => None,
        };
        if let Some(k) = numbered.filter(|_| f.applied.is_none() && doc.input_editable.is_ok()) {
            super::many::renumber_one(ui, doc, k, id, out);
        }
        // Shown as they are when the input cannot be edited, the reason
        // on hover.
        let r = ui.add_enabled_ui(doc.input_editable.is_ok(), |ui| match &mut f.body {
            Body::Cell(c) => c.ui(ui, doc, id, go, out),
            Body::Material(m) => m.ui(ui, doc, id, go),
        });
        if let Err(why) = doc.input_editable {
            r.response.on_disabled_hover_text(why);
        }
        let changes = f.count();
        let pending = f.applied.is_some();
        if changes == 0 && !pending && f.error.is_none() {
            return;
        }
        ui.separator();
        ui.label(
            egui::RichText::new(match changes {
                1 => "1 field changed · 1 card".to_string(),
                n => format!("{n} fields changed · 1 card"),
            })
            .weak(),
        );
        // What Apply would write, before and after.
        if changes > 0 && !pending {
            let key = match &f.body {
                Body::Cell(c) => format!("{:?}", c.changes()),
                Body::Material(m) => format!("{:?}", m.changes()),
            };
            if f.preview.as_ref().is_none_or(|p| p.0 != key) {
                let text = match (&f.body, doc.source.as_ref()) {
                    (_, None) => Err("the input's cards are not read".to_string()),
                    (Body::Cell(c), Some(s)) => s.cell_with(id, &c.changes()),
                    (Body::Material(m), Some(s)) => {
                        let (entries, keywords) = m.changes();
                        s.material_with(id, &entries, &keywords)
                    }
                };
                f.preview = Some((key, text));
            }
            match (&f.preview, &now) {
                (Some((_, Ok(t))), Some(n)) => {
                    super::diff::cards(
                        ui,
                        &[(format!("{} {id}", kind.name()), n.clone(), t.clone())],
                        0,
                    );
                    super::diff::after_apply(ui);
                }
                (Some((_, Err(e))), _) => {
                    ui.colored_label(ui.visuals().warn_fg_color, e);
                }
                _ => {}
            }
        }
        if let Some(e) = &f.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        ui.horizontal(|ui| {
            let flagged = match (&f.body, doc.source.as_ref()) {
                (Body::Cell(c), Some(s)) => !c.geometry.flags(s).is_empty(),
                _ => false,
            };
            let ok = doc.input_editable.is_ok()
                && changes > 0
                && !pending
                && !changed_under
                && !flagged;
            let r = ui.add_enabled(ok, egui::Button::new("Apply"));
            crate::app::frame::named(&r, "Apply fields");
            let r = match doc.input_editable {
                Err(why) => r.on_disabled_hover_text(why),
                Ok(()) if pending => r.on_disabled_hover_text("being written and compiled"),
                Ok(()) if flagged => r.on_disabled_hover_text("the geometry has words flagged"),
                Ok(()) => r.on_hover_text(
                    "Write the words changed into the card and compile again (not saved; Undo edit takes it back)",
                ),
            };
            if r.clicked() {
                let text = f
                    .preview
                    .as_ref()
                    .map_or_else(|| Err("nothing to write".to_string()), |p| p.1.clone());
                match text {
                    Ok(t) if now.as_ref().is_some_and(|n| n.trim_end() == t.trim_end()) => {
                        f.error = Some("the card would read the same".into());
                    }
                    Ok(t) => {
                        out.push(Command::EditCard(kind, id, t.clone()));
                        f.applied = Some(t);
                        f.error = None;
                    }
                    Err(e) => f.error = Some(e),
                }
            }
            let r = ui
                .add_enabled(!pending, egui::Button::new("Discard"))
                .on_hover_text("Back to the card as it is");
            crate::app::frame::named(&r, "Discard fields");
            if r.clicked() {
                close = true;
            }
            if pending {
                crate::logo::small_spinner(ui);
                ui.label(egui::RichText::new("writing the card and compiling…").weak());
            }
        });
    });
    if close {
        *form = None;
    }
    true
}

/// A row of chips, one per surface a cell names, with its sense and type
/// (its comment on hover), for a cell without fields (the input's cards not
/// read). A click opens its page; the one hovered is returned.
pub(super) fn surface_chips(
    ui: &mut egui::Ui,
    doc: &Document,
    surfaces: &[(i64, bool)],
    go: &mut Option<Subject>,
) -> Option<i64> {
    let mut hovered = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for &(s, negative) in surfaces {
            let kind = doc
                .source
                .as_ref()
                .and_then(|src| src.surface_coeffs(s))
                .map(|(k, _)| k.to_uppercase())
                .unwrap_or_default();
            let mut text = egui::text::LayoutJob::default();
            let mono = egui::FontId::monospace(11.5);
            let sense = if negative { "−" } else { "+" };
            text.append(
                &format!("{sense}{s}"),
                0.0,
                egui::TextFormat::simple(mono.clone(), super::card::SURFACE),
            );
            if !kind.is_empty() {
                text.append(
                    &format!(" {kind}"),
                    0.0,
                    egui::TextFormat::simple(mono, ui.visuals().weak_text_color()),
                );
            }
            let r = ui.add(egui::Button::new(text).small().corner_radius(8.0));
            crate::app::frame::named(&r, &format!("Surface chip {s}"));
            let tip = match super::comment(doc, CardKind::Surface, s) {
                Some(c) => {
                    format!("s {s}: {c}\nClick: its page · marked in the views while hovered")
                }
                None => format!("s {s}\nClick: its page · marked in the views while hovered"),
            };
            let r = r.on_hover_text(tip);
            if r.hovered() {
                hovered = Some(s);
            }
            if r.clicked() {
                *go = Some(Subject::Surface(s));
            }
        }
    });
    hovered
}

/// The last Apply did not land (an error in the model): ready to try again.
pub fn edit_failed(form: &mut Option<Form>, why: &str) {
    if let Some(f) = form {
        if f.applied.take().is_some() {
            f.error = Some(why.to_string());
        }
    }
}

// --- Cells ---

struct ParamRow {
    /// Its key ([`llevant_scene::source::Param::key`]; a new one as typed).
    key: String,
    label: String,
    value: String,
    /// The value as the card has it (`None`: added here).
    was: Option<String>,
    removed: bool,
}

struct CellForm {
    start: CellFields,
    material: Option<i64>,
    /// As MCNP takes it (negative: g/cm³).
    density: Option<f64>,
    /// The density shown in g/cm³ (else atoms/b·cm).
    grams: bool,
    params: Vec<ParamRow>,
    /// Keys data cards give for every cell.
    blocked: Vec<String>,
    new_key: String,
    new_value: String,
    filter: String,
    geometry: super::geometry::GeometryEditor,
    /// The surface hovered in the geometry (marked in the views).
    hovered: Option<i64>,
}

impl CellForm {
    fn new(start: CellFields, blocked: Vec<String>) -> CellForm {
        let params = start
            .params
            .iter()
            .map(|p| ParamRow {
                key: p.key.clone(),
                label: p.label(),
                value: p.value.clone(),
                was: Some(p.value.clone()),
                removed: false,
            })
            .collect();
        CellForm {
            material: start.material,
            density: start.density,
            grams: start.density.is_none_or(|d| d < 0.0),
            params,
            blocked,
            new_key: String::new(),
            new_value: String::new(),
            filter: String::new(),
            geometry: super::geometry::GeometryEditor::new(&start.geometry),
            hovered: None,
            start,
        }
    }

    fn changes(&self) -> Vec<CellChange> {
        let mut v = Vec::new();
        if self.geometry.changed() {
            v.push(CellChange::Geometry(self.geometry.text.clone()));
        }
        if self.material != self.start.material {
            if let Some(m) = self.material {
                v.push(CellChange::Material(m));
            }
        }
        if self.material != Some(0) && self.material.is_some() {
            if let Some(d) = self.density {
                let was = if self.material == self.start.material {
                    self.start.density
                } else {
                    None
                };
                if Some(d) != was {
                    v.push(CellChange::Density(d));
                }
            }
        }
        for p in &self.params {
            match (&p.was, p.removed) {
                (Some(_), true) => v.push(CellChange::Param {
                    key: p.key.clone(),
                    value: None,
                }),
                (None, true) => {}
                (was, false) if was.as_deref() != Some(p.value.trim()) => {
                    v.push(CellChange::Param {
                        key: p.key.clone(),
                        value: Some(p.value.trim().to_string()),
                    })
                }
                _ => {}
            }
        }
        v
    }

    fn count(&self) -> usize {
        self.changes().len()
    }

    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        doc: &Document,
        id: i64,
        go: &mut Option<Subject>,
        out: &mut Vec<Command>,
    ) {
        let _ = out;
        egui::Grid::new(("cell fields", id))
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                if let Some(like) = self.start.like {
                    ui.label(egui::RichText::new("like").weak());
                    ui.horizontal(|ui| {
                        ui.label(theme::data(format!("c {like}")));
                        super::drill(ui, Subject::Cell(like), go);
                        ui.label(egui::RichText::new("MAT= and RHO= below").weak());
                    });
                    ui.end_row();
                }
                if let Some(mut m) = self.material {
                    ui.label(egui::RichText::new("material").weak());
                    let picked = ui
                        .horizontal(|ui| {
                            let c = material_picker(
                                ui,
                                doc,
                                ("cell material", id),
                                &mut m,
                                &mut self.filter,
                            );
                            if m != 0 {
                                super::drill(ui, Subject::Material(m), go);
                            }
                            c
                        })
                        .inner;
                    if picked {
                        // From void: a density to start from, the one most
                        // cells of that material have.
                        if self.material == Some(0) && m != 0 && self.density.is_none() {
                            self.density = Some(usual_density(doc, m).unwrap_or(-1.0));
                            self.grams = self.density.is_some_and(|d| d < 0.0);
                        }
                        self.material = Some(m);
                    }
                    ui.end_row();
                    if m != 0 {
                        ui.label(egui::RichText::new("density").weak());
                        self.density_field(ui, doc, m);
                        ui.end_row();
                    }
                }
            });

        // The geometry: text, with help.
        ui.add_space(4.0);
        ui.label(egui::RichText::new("GEOMETRY").size(11.0).weak());
        if self.start.like.is_some() {
            ui.label(egui::RichText::new("that of the cell it is like").weak());
            self.hovered = None;
        } else {
            if let Some(s) = self.geometry.ui(ui, doc, ("cell", id)) {
                *go = Some(Subject::Surface(s));
            }
            self.hovered = self.geometry.hovered;
        }

        // Parameters, as written, then one to add.
        ui.add_space(4.0);
        ui.label(egui::RichText::new("PARAMETERS").size(11.0).weak());
        egui::Grid::new(("cell params", id))
            .num_columns(3)
            .spacing([8.0, 3.0])
            .show(ui, |ui| {
                let lattice = self.start.lattice_fill;
                for p in &mut self.params {
                    let label = egui::RichText::new(&p.label).monospace();
                    ui.label(if p.removed {
                        label.strikethrough().weak()
                    } else {
                        label
                    });
                    let fixed = lattice && p.key == "fill";
                    let r = text_field(ui, !p.removed && !fixed, &mut p.value, 170.0);
                    let r = if fixed {
                        r.on_disabled_hover_text(
                            "A lattice's array of fills: edit it in the card's text",
                        )
                    } else {
                        r.on_hover_text(param_hint(&p.key))
                    };
                    let _ = r;
                    let (icon, tip) = if p.removed {
                        (icons::UNDO, "Keep it")
                    } else {
                        (icons::REMOVE, "Remove it from the card")
                    };
                    if ui
                        .add_enabled(!fixed, egui::Button::new(icon).small().frame(false))
                        .on_hover_text(tip)
                        .clicked()
                    {
                        p.removed = !p.removed;
                    }
                    ui.end_row();
                }
            });
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(("new param", id))
                .width(80.0)
                .selected_text(if self.new_key.is_empty() {
                    "+ add".to_string()
                } else {
                    self.new_key.to_uppercase()
                })
                .show_ui(ui, |ui| {
                    for k in CELL_KEYS {
                        let has = self
                            .params
                            .iter()
                            .any(|p| p.key == k.trim_start_matches('*') && !p.removed);
                        let blocked = self.blocked.iter().any(|b| b == k.trim_start_matches('*'));
                        let r = ui.add_enabled(
                            !has && !blocked,
                            egui::Button::selectable(self.new_key == k, k.to_uppercase()),
                        );
                        let r = if blocked {
                            r.on_disabled_hover_text("a data card gives it for every cell")
                        } else {
                            r.on_disabled_hover_text("the card has it")
                        };
                        if r.clicked() {
                            self.new_key = k.to_string();
                        }
                    }
                });
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.new_key)
                    .hint_text("key")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(60.0),
            );
            name_text(&r, "New parameter key");
            ui.label("=");
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.new_value)
                    .hint_text("value")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(70.0),
            );
            name_text(&r, "New parameter value");
            let key = self.new_key.trim().to_ascii_lowercase();
            let bare = key.trim_start_matches('*').to_string();
            let why = if key.is_empty() || self.new_value.trim().is_empty() {
                Some("a key and a value")
            } else if self.params.iter().any(|p| p.key == bare && !p.removed) {
                Some("the card has it: change it above")
            } else if self.blocked.contains(&bare) {
                Some("a data card gives it for every cell: MCNP takes it only one way")
            } else {
                None
            };
            let r = ui
                .add_enabled(why.is_none(), egui::Button::new("Add").small())
                .on_disabled_hover_text(why.unwrap_or_default());
            crate::app::frame::named(&r, "Add parameter");
            if r.clicked() {
                // Added again after removing it: the old one written over.
                if let Some(p) = self.params.iter_mut().find(|p| p.key == bare) {
                    p.removed = false;
                    p.value = self.new_value.trim().to_string();
                } else {
                    self.params.push(ParamRow {
                        key: key.clone(),
                        label: key.to_uppercase(),
                        value: self.new_value.trim().to_string(),
                        was: None,
                        removed: false,
                    });
                }
                self.new_key.clear();
                self.new_value.clear();
            }
        });
        if !self.blocked.is_empty() {
            ui.label(
                egui::RichText::new(format!(
                    "given by data cards for every cell: {}",
                    self.blocked.join(", ").to_uppercase()
                ))
                .weak(),
            );
        }
    }

    /// The density, in the unit chosen, with the other shown.
    fn density_field(&mut self, ui: &mut egui::Ui, doc: &Document, material: i64) {
        let a = super::atomic_mass(doc, material);
        let d = self.density.unwrap_or(-1.0);
        // The value in the unit shown.
        let shown = match (self.grams, d < 0.0, a) {
            (true, true, _) => Some(-d),
            (false, false, _) => Some(d),
            (true, false, Some(a)) => llevant_scene::materials::grams_per_cm3(d, a),
            (false, true, Some(a)) => Some(atoms(-d, a)),
            (_, _, None) => None,
        };
        ui.horizontal(|ui| {
            let mut v = shown.unwrap_or(d.abs());
            let speed = (v * 0.002).max(1e-4);
            let r = ui.add(
                egui::DragValue::new(&mut v)
                    .speed(speed)
                    .range(0.0..=f64::INFINITY)
                    .max_decimals(6),
            );
            if r.changed() && v > 0.0 {
                self.density = Some(if self.grams { -v } else { v });
            }
            let units = [(true, "g/cm³"), (false, "at/b·cm")];
            for (g, name) in units {
                let r = ui.add_enabled(
                    a.is_some() || g == (d < 0.0),
                    egui::Button::selectable(self.grams == g, name).small(),
                );
                let r = r.on_disabled_hover_text("the material's atomic mass is not known");
                if r.clicked() && self.grams != g {
                    self.grams = g;
                    // The same density, written in the other unit.
                    if let (Some(a), Some(s)) = (a, shown) {
                        self.density = Some(if g {
                            -llevant_scene::materials::grams_per_cm3(s, a).unwrap_or(s)
                        } else {
                            atoms(s, a)
                        });
                        self.density = self.density.map(round);
                    }
                }
            }
            let other = match (d < 0.0, a) {
                (true, Some(a)) => Some(format!("= {:.5e} at/b·cm", atoms(-d, a))),
                (false, Some(a)) => {
                    llevant_scene::materials::grams_per_cm3(d, a).map(|g| format!("= {g:.5} g/cm³"))
                }
                _ => None,
            };
            if let Some(o) = other {
                ui.label(theme::data(o).weak());
            }
        });
    }
}

/// A text field of `width` (in a grid too, whose columns would shrink it).
pub(super) fn text_field(
    ui: &mut egui::Ui,
    enabled: bool,
    value: &mut String,
    width: f32,
) -> egui::Response {
    let h = ui.spacing().interact_size.y;
    ui.add_enabled_ui(enabled, |ui| {
        ui.add_sized(
            [width, h],
            egui::TextEdit::singleline(value).font(egui::TextStyle::Monospace),
        )
    })
    .inner
}

/// A text field's name, for the app's tests and screen readers.
pub(super) fn name_text(r: &egui::Response, name: &str) {
    let enabled = r.enabled();
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, enabled, name));
}

/// Atoms per barn·cm of `grams` g/cm³ of atoms of mean mass `a` u.
fn atoms(grams: f64, a: f64) -> f64 {
    grams * 0.602_214_076 / a
}

/// `v` to [`DIGITS`] significant digits.
fn round(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    format!("{v:.*e}", DIGITS - 1).parse().unwrap_or(v)
}

/// What a parameter's value is, for its hover.
pub(super) fn param_hint(key: &str) -> String {
    let k = key.trim_start_matches('*');
    let what = match k.split(':').next().unwrap_or(k) {
        "imp" => "importance (0: outside the problem)",
        "u" => "the universe this cell belongs to",
        "fill" => "the universe that fills it, and its transform: 100 (7) or 100 (dx dy dz …)",
        "trcl" => "its transform: a TR number, or (dx dy dz …)",
        "lat" => "lattice: 1 hexahedral, 2 hexagonal prism",
        "vol" => "its volume, cm³",
        "tmp" => "its temperature, MeV",
        "mat" => "the material of a LIKE cell",
        "rho" => "the density of a LIKE cell",
        _ => "as MCNP reads it",
    };
    format!("{}: {what}", key.to_uppercase())
}

/// The density most cells of material `m` have, as MCNP takes it.
pub(super) fn usual_density(doc: &Document, m: i64) -> Option<f64> {
    let mut counts: BTreeMap<u64, usize> = BTreeMap::new();
    for &c in doc.index.material_cells(m) {
        *counts
            .entry(doc.model.cell(c).density.to_bits())
            .or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(_, n)| n)
        .map(|(b, _)| f64::from_bits(b))
}

/// A material picked from the model's, with their swatches and names
/// (void first); true if it changed.
pub(super) fn material_picker(
    ui: &mut egui::Ui,
    doc: &Document,
    salt: impl std::hash::Hash + std::fmt::Debug,
    m: &mut i64,
    filter: &mut String,
) -> bool {
    let name = |id: i64| -> String {
        if id == 0 {
            return "void".into();
        }
        match super::comment(doc, CardKind::Material, id) {
            Some(c) => format!("m {id}  {c}"),
            None => format!("m {id}"),
        }
    };
    let mut changed = false;
    let before = *m;
    ui.horizontal(|ui| {
        swatch(ui, *m);
        egui::ComboBox::from_id_salt(salt)
            .width(180.0)
            .selected_text(theme::data(name(*m)))
            .height(320.0)
            .show_ui(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(filter)
                        .hint_text("number or name")
                        .desired_width(160.0),
                );
                let f = filter.trim().to_lowercase();
                let ids = std::iter::once(0).chain(doc.looks.materials.iter().map(|l| l.id));
                for id in ids {
                    let n = name(id);
                    if !f.is_empty() && !n.to_lowercase().contains(&f) {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        swatch(ui, id);
                        if ui.selectable_label(*m == id, theme::data(n)).clicked() {
                            *m = id;
                        }
                    });
                }
            });
    });
    changed |= *m != before;
    changed
}

fn swatch(ui: &mut egui::Ui, m: i64) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    if m == 0 {
        ui.painter().rect_stroke(
            r,
            2.0,
            egui::Stroke::new(1.0, ui.visuals().weak_text_color()),
            egui::StrokeKind::Inside,
        );
    } else {
        let [cr, cg, cb] = llevant_render::colours::material_colour(m);
        ui.painter()
            .rect_filled(r, 2.0, egui::Color32::from_rgb(cr, cg, cb));
    }
}

// --- Materials ---

struct Row {
    /// Its element when it was listed: the group it stays in while its
    /// ZAID is typed.
    z: usize,
    origin: Option<usize>,
    zaid: String,
    /// As MCNP takes it (negative: by mass).
    fraction: f64,
    removed: bool,
}

struct Keyword {
    name: String,
    value: String,
    was: Option<String>,
    removed: bool,
}

/// One part of a mixture: a material, its density and its share of the
/// volume.
struct Part {
    material: i64,
    grams: f64,
    percent: f64,
}

struct MaterialForm {
    start: MaterialFields,
    rows: Vec<Row>,
    keywords: Vec<Keyword>,
    new_zaid: String,
    new_fraction: f64,
    new_key: String,
    new_value: String,
    compare: Option<i64>,
    compare_filter: String,
    mix: Vec<Part>,
    mix_filter: String,
    /// The density of the mixture last written as the composition, g/cm³.
    mixed: Option<f64>,
}

impl MaterialForm {
    fn new(start: MaterialFields) -> MaterialForm {
        let rows = start
            .entries
            .iter()
            .enumerate()
            .map(|(k, (z, f))| Row {
                z: zaid_z(z),
                origin: Some(k),
                zaid: z.clone(),
                fraction: *f,
                removed: false,
            })
            .collect();
        let keywords = start
            .keywords
            .iter()
            .map(|(n, v)| Keyword {
                name: n.clone(),
                value: v.clone(),
                was: Some(v.clone()),
                removed: false,
            })
            .collect();
        MaterialForm {
            start,
            rows,
            keywords,
            new_zaid: String::new(),
            new_fraction: 0.0,
            new_key: String::new(),
            new_value: String::new(),
            compare: None,
            compare_filter: String::new(),
            mix: Vec::new(),
            mix_filter: String::new(),
            mixed: None,
        }
    }

    fn changes(&self) -> (Vec<Entry>, Vec<(String, Option<String>)>) {
        let entries = self
            .rows
            .iter()
            .filter(|r| !r.removed)
            .map(|r| Entry {
                origin: r.origin,
                zaid: r.zaid.trim().to_string(),
                fraction: r.fraction,
            })
            .collect();
        let keywords = self
            .keywords
            .iter()
            .filter_map(|k| match (&k.was, k.removed) {
                (Some(_), true) => Some((k.name.clone(), None)),
                (None, true) => None,
                (was, false) if was.as_deref() != Some(k.value.trim()) => {
                    Some((k.name.clone(), Some(k.value.trim().to_string())))
                }
                _ => None,
            })
            .collect();
        (entries, keywords)
    }

    fn count(&self) -> usize {
        let rows = self
            .rows
            .iter()
            .filter(|r| match r.origin {
                None => !r.removed,
                Some(k) => {
                    r.removed || {
                        let (z, f) = &self.start.entries[k];
                        z != r.zaid.trim() || *f != r.fraction
                    }
                }
            })
            .count();
        rows + self.changes().1.len()
    }

    fn by_mass(&self) -> bool {
        self.rows
            .iter()
            .find(|r| !r.removed)
            .is_some_and(|r| r.fraction < 0.0)
    }

    /// The entries as `(zaid, fraction)`, those kept.
    fn entries(&self) -> Vec<(String, f64)> {
        self.rows
            .iter()
            .filter(|r| !r.removed)
            .map(|r| (r.zaid.trim().to_string(), r.fraction))
            .collect()
    }

    fn ui(&mut self, ui: &mut egui::Ui, doc: &Document, id: i64, go: &mut Option<Subject>) {
        let by_mass = self.by_mass();
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("fractions").weak());
            let mut mass = by_mass;
            for (v, name) in [(false, "atoms"), (true, "mass")] {
                let r = ui
                    .add(egui::Button::selectable(mass == v, name).small())
                    .on_hover_text(
                        "Written again as fractions of the other kind, summing to 1 \
                         (masses of nuclides from the mass formula, of natural elements their standard atomic weights)",
                    );
                if r.clicked() {
                    mass = v;
                }
            }
            if mass != by_mass {
                if let Err(e) = self.convert(mass) {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
            }
            if ui
                .small_button("Normalise")
                .on_hover_text("Fractions scaled to sum to 1")
                .clicked()
            {
                self.normalise();
            }
        });
        let sum: f64 = self
            .rows
            .iter()
            .filter(|r| !r.removed)
            .map(|r| r.fraction.abs())
            .sum();
        ui.label(
            theme::data(format!(
                "Σ = {sum:.6} ({}) · {} nuclide{}",
                if by_mass { "mass" } else { "atoms" },
                self.rows.iter().filter(|r| !r.removed).count(),
                if self.rows.len() == 1 { "" } else { "s" }
            ))
            .weak(),
        );
        if self
            .rows
            .iter()
            .any(|r| !r.removed && (r.fraction < 0.0) != by_mass)
        {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Atom and mass fractions mixed: MCNP rejects it",
            );
        }

        // By element, each folded over its nuclides.
        let mut by_z: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (k, r) in self.rows.iter().enumerate() {
            by_z.entry(r.z).or_default().push(k);
        }
        let total = sum.max(f64::MIN_POSITIVE);
        for (z, ks) in &by_z {
            let share: f64 = ks
                .iter()
                .filter(|&&k| !self.rows[k].removed)
                .map(|&k| self.rows[k].fraction.abs())
                .sum::<f64>()
                / total;
            let head = format!(
                "{:<3} {:>6.2} %  {} nuclide{}",
                element(*z),
                100.0 * share,
                ks.len(),
                if ks.len() == 1 { "" } else { "s" }
            );
            egui::CollapsingHeader::new(theme::data(head))
                .id_salt(("element", id, *z))
                .default_open(ks.len() == 1 && self.rows.len() <= 6)
                .show(ui, |ui| {
                    egui::Grid::new(("element rows", id, *z))
                        .num_columns(3)
                        .spacing([6.0, 2.0])
                        .show(ui, |ui| {
                            for &k in ks {
                                let r = &mut self.rows[k];
                                text_field(ui, !r.removed, &mut r.zaid, 90.0);
                                let mut f = r.fraction.abs();
                                let speed = (f * 0.002).max(1e-6);
                                let resp = ui.add_enabled(
                                    !r.removed,
                                    egui::DragValue::new(&mut f)
                                        .speed(speed)
                                        .range(0.0..=f64::INFINITY)
                                        .max_decimals(8),
                                );
                                if resp.changed() {
                                    r.fraction = if by_mass { -f } else { f };
                                }
                                let (icon, tip) = if r.removed {
                                    (icons::UNDO, "Keep it")
                                } else {
                                    (icons::REMOVE, "Remove it")
                                };
                                if ui
                                    .add(egui::Button::new(icon).small().frame(false))
                                    .on_hover_text(tip)
                                    .clicked()
                                {
                                    r.removed = !r.removed;
                                }
                                ui.end_row();
                            }
                        });
                });
        }
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_zaid)
                    .hint_text("ZAID (26056.80c)")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(130.0),
            );
            ui.add(
                egui::DragValue::new(&mut self.new_fraction)
                    .speed(0.001)
                    .range(0.0..=f64::INFINITY)
                    .max_decimals(8),
            );
            let ok = zaid_z(&self.new_zaid) > 0 && self.new_fraction > 0.0;
            if ui
                .add_enabled(ok, egui::Button::new("+ nuclide").small())
                .on_disabled_hover_text("a ZAID naming an element, and a fraction")
                .clicked()
            {
                self.rows.push(Row {
                    z: zaid_z(&self.new_zaid),
                    origin: None,
                    zaid: self.new_zaid.trim().to_string(),
                    fraction: if by_mass {
                        -self.new_fraction
                    } else {
                        self.new_fraction
                    },
                    removed: false,
                });
                self.new_zaid.clear();
                self.new_fraction = 0.0;
            }
        });

        // Keywords.
        ui.add_space(4.0);
        ui.label(egui::RichText::new("KEYWORDS").size(11.0).weak());
        egui::Grid::new(("material keywords", id))
            .num_columns(3)
            .spacing([8.0, 3.0])
            .show(ui, |ui| {
                for k in &mut self.keywords {
                    let label = egui::RichText::new(k.name.to_uppercase()).monospace();
                    ui.label(if k.removed {
                        label.strikethrough().weak()
                    } else {
                        label
                    });
                    text_field(ui, !k.removed, &mut k.value, 90.0);
                    let (icon, tip) = if k.removed {
                        (icons::UNDO, "Keep it")
                    } else {
                        (icons::REMOVE, "Remove it")
                    };
                    if ui
                        .add(egui::Button::new(icon).small().frame(false))
                        .on_hover_text(tip)
                        .clicked()
                    {
                        k.removed = !k.removed;
                    }
                    ui.end_row();
                }
            });
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(("new keyword", id))
                .width(70.0)
                .selected_text(if self.new_key.is_empty() {
                    "+ add".to_string()
                } else {
                    self.new_key.to_uppercase()
                })
                .show_ui(ui, |ui| {
                    for k in MATERIAL_KEYS {
                        let has = self.keywords.iter().any(|w| w.name.eq_ignore_ascii_case(k));
                        if ui
                            .add_enabled(
                                !has,
                                egui::Button::selectable(self.new_key == k, k.to_uppercase()),
                            )
                            .clicked()
                        {
                            self.new_key = k.to_string();
                        }
                    }
                });
            ui.label("=");
            ui.add(
                egui::TextEdit::singleline(&mut self.new_value)
                    .hint_text("value (.80c)")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(70.0),
            );
            let ok = !self.new_key.is_empty() && !self.new_value.trim().is_empty();
            if ui
                .add_enabled(ok, egui::Button::new("Add").small())
                .clicked()
            {
                match self
                    .keywords
                    .iter_mut()
                    .find(|w| w.name.eq_ignore_ascii_case(&self.new_key))
                {
                    Some(w) => {
                        w.removed = false;
                        w.value = self.new_value.trim().to_string();
                    }
                    None => self.keywords.push(Keyword {
                        name: self.new_key.clone(),
                        value: self.new_value.trim().to_string(),
                        was: None,
                        removed: false,
                    }),
                }
                self.new_key.clear();
                self.new_value.clear();
            }
        });

        self.mixture(ui, doc, id);
        self.comparison(ui, doc, id, go);
    }

    /// Fractions written as the other kind (mass or atoms), summing to 1.
    fn convert(&mut self, to_mass: bool) -> Result<(), String> {
        let mut masses = Vec::new();
        for r in self.rows.iter().filter(|r| !r.removed) {
            masses.push(zaid_mass(&r.zaid).ok_or_else(|| format!("no mass known for {}", r.zaid))?);
        }
        let kept: Vec<&mut Row> = self.rows.iter_mut().filter(|r| !r.removed).collect();
        let new: Vec<f64> = kept
            .iter()
            .zip(&masses)
            .map(|(r, m)| {
                let f = r.fraction.abs();
                if to_mass {
                    f * m
                } else {
                    f / m
                }
            })
            .collect();
        let sum: f64 = new.iter().sum();
        if !(sum > 0.0) {
            return Err("the fractions sum to 0".into());
        }
        for (r, v) in kept.into_iter().zip(new) {
            let v = round(v / sum);
            r.fraction = if to_mass { -v } else { v };
        }
        Ok(())
    }

    fn normalise(&mut self) {
        let sum: f64 = self
            .rows
            .iter()
            .filter(|r| !r.removed)
            .map(|r| r.fraction.abs())
            .sum();
        if sum > 0.0 {
            for r in self.rows.iter_mut().filter(|r| !r.removed) {
                r.fraction = round(r.fraction / sum);
            }
        }
    }

    /// A mixture of other materials by volume: its composition (by mass)
    /// written as this one's, and its density.
    fn mixture(&mut self, ui: &mut egui::Ui, doc: &Document, id: i64) {
        egui::CollapsingHeader::new(egui::RichText::new("Mix from other materials").weak())
            .id_salt(("mixture", id))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Parts by volume, each at its density: the composition by mass and the density of the mixture.",
                    )
                    .weak(),
                );
                let mut drop = None;
                egui::Grid::new(("mixture parts", id))
                    .num_columns(4)
                    .spacing([6.0, 3.0])
                    .show(ui, |ui| {
                        for (k, p) in self.mix.iter_mut().enumerate() {
                            let before = p.material;
                            material_picker(ui, doc, ("mix part", id, k), &mut p.material, &mut self.mix_filter);
                            if p.material != before {
                                p.grams = usual_grams(doc, p.material).unwrap_or(p.grams);
                            }
                            ui.add(
                                egui::DragValue::new(&mut p.grams)
                                    .speed(0.01)
                                    .range(0.0..=f64::INFINITY)
                                    .suffix(" g/cm³")
                                    .max_decimals(5),
                            );
                            ui.add(
                                egui::DragValue::new(&mut p.percent)
                                    .speed(0.2)
                                    .range(0.0..=100.0)
                                    .suffix(" %vol")
                                    .max_decimals(3),
                            );
                            if ui.add(egui::Button::new(icons::REMOVE).small().frame(false)).clicked() {
                                drop = Some(k);
                            }
                            ui.end_row();
                        }
                    });
                if let Some(k) = drop {
                    self.mix.remove(k);
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("+ part").clicked() {
                        let m = doc.looks.materials.first().map_or(0, |l| l.id);
                        self.mix.push(Part {
                            material: m,
                            grams: usual_grams(doc, m).unwrap_or(1.0),
                            percent: 0.0,
                        });
                    }
                    let vol: f64 = self.mix.iter().map(|p| p.percent).sum();
                    if !self.mix.is_empty() {
                        ui.label(theme::data(format!("Σ {vol:.2} %vol")).weak());
                    }
                    match mix(doc, &self.mix) {
                        Ok((entries, rho)) => {
                            ui.label(theme::data(format!("{rho:.5} g/cm³")))
                                .on_hover_text("The mixture's density (the parts' volume shares scaled to 100 %)");
                            if ui
                                .small_button("Use as the composition")
                                .on_hover_text("This material's entries replaced by the mixture's, by mass; set the cells' density to the one shown")
                                .clicked()
                            {
                                for r in &mut self.rows {
                                    r.removed = true;
                                }
                                self.rows.retain(|r| r.origin.is_some());
                                for (zaid, f) in entries {
                                    // An entry the card has (same ZAID): kept.
                                    match self.rows.iter_mut().find(|r| r.zaid.trim() == zaid) {
                                        Some(r) => {
                                            r.removed = false;
                                            r.fraction = -f;
                                        }
                                        None => self.rows.push(Row {
                                            z: zaid_z(&zaid),
                                            origin: None,
                                            zaid,
                                            fraction: -f,
                                            removed: false,
                                        }),
                                    }
                                }
                                self.mixed = Some(rho);
                            }
                        }
                        Err(e) if !self.mix.is_empty() => {
                            ui.label(egui::RichText::new(e).weak());
                        }
                        Err(_) => {}
                    }
                });
                if let Some(rho) = self.mixed {
                    ui.label(
                        egui::RichText::new(format!(
                            "Composition from the mixture; its density is {rho:.5} g/cm³ (write it as -{rho:.5} on the cells)"
                        ))
                        .weak(),
                    );
                }
            });
    }

    /// Another material beside this one, element by element (by mass).
    fn comparison(&mut self, ui: &mut egui::Ui, doc: &Document, id: i64, go: &mut Option<Subject>) {
        egui::CollapsingHeader::new(egui::RichText::new("Compare with another material").weak())
            .id_salt(("compare", id))
            .show(ui, |ui| {
                let mut other = self.compare.unwrap_or(0);
                ui.horizontal(|ui| {
                    material_picker(
                        ui,
                        doc,
                        ("compare with", id),
                        &mut other,
                        &mut self.compare_filter,
                    );
                    if other != 0 {
                        super::drill(ui, Subject::Material(other), go);
                    }
                });
                self.compare = (other != 0).then_some(other);
                let Some(o) = self.compare else { return };
                let Some(theirs) = doc.model.material(o) else {
                    return;
                };
                let mine = element_mass_fractions(&self.entries());
                let theirs = element_mass_fractions(theirs);
                let mut zs: Vec<usize> = mine.iter().chain(&theirs).map(|e| e.0).collect();
                zs.sort();
                zs.dedup();
                let get = |v: &[(usize, f64)], z| v.iter().find(|e| e.0 == z).map_or(0.0, |e| e.1);
                zs.sort_by(|&a, &b| {
                    (get(&theirs, b) + get(&mine, b)).total_cmp(&(get(&theirs, a) + get(&mine, a)))
                });
                egui::Grid::new(("comparison", id))
                    .num_columns(4)
                    .spacing([10.0, 2.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("% mass").weak());
                        ui.label(theme::data(format!("m {id}")).weak());
                        ui.label(theme::data(format!("m {o}")).weak());
                        ui.label(egui::RichText::new("Δ").weak());
                        ui.end_row();
                        for z in zs {
                            let (a, b) = (100.0 * get(&mine, z), 100.0 * get(&theirs, z));
                            ui.label(theme::data(element(z)));
                            ui.label(theme::data(format!("{a:.3}")));
                            ui.label(theme::data(format!("{b:.3}")));
                            let d = theme::data(format!("{:+.3}", a - b));
                            ui.label(if (a - b).abs() < 1e-3 { d.weak() } else { d });
                            ui.end_row();
                        }
                    });
            });
    }
}

/// The density in g/cm³ most cells of material `m` have.
fn usual_grams(doc: &Document, m: i64) -> Option<f64> {
    let d = usual_density(doc, m)?;
    llevant_scene::materials::grams_per_cm3(d, super::atomic_mass(doc, m)?)
}

/// The mixture of `parts`: its entries by mass (summing to 1, the parts'
/// nuclides as their cards give them) and its density, g/cm³.
fn mix(doc: &Document, parts: &[Part]) -> Result<(Vec<(String, f64)>, f64), String> {
    let vol: f64 = parts.iter().map(|p| p.percent).sum();
    if parts.is_empty() || !(vol > 0.0) {
        return Err("give the parts' shares of the volume".into());
    }
    let mut out: Vec<(String, f64)> = Vec::new();
    let mut rho = 0.0;
    for p in parts {
        let entries = doc
            .model
            .material(p.material)
            .ok_or_else(|| format!("no material {}", p.material))?;
        let share = p.percent / vol;
        let grams = share * p.grams;
        rho += grams;
        // The part's nuclides by mass, summing to 1.
        let by_mass = entries.first().is_some_and(|e| e.1 < 0.0);
        let mut w = Vec::new();
        for (zaid, f) in entries {
            let f = f.abs();
            let m = if by_mass {
                1.0
            } else {
                zaid_mass(zaid).ok_or_else(|| format!("no mass known for {zaid}"))?
            };
            w.push((zaid.clone(), f * m));
        }
        let sum: f64 = w.iter().map(|e| e.1).sum();
        if !(sum > 0.0) {
            return Err(format!("m {} has no fractions", p.material));
        }
        for (zaid, x) in w {
            let x = grams * x / sum;
            match out.iter_mut().find(|e| e.0 == zaid) {
                Some(e) => e.1 += x,
                None => out.push((zaid, x)),
            }
        }
    }
    if !(rho > 0.0) {
        return Err("give the parts' densities".into());
    }
    for e in &mut out {
        e.1 = round(e.1 / rho);
    }
    Ok((out, rho))
}

/// Element (Z) of a ZAID, 0 if none.
fn zaid_z(zaid: &str) -> usize {
    zaid.split('.')
        .next()
        .and_then(|h| h.trim().parse::<u64>().ok())
        .map_or(0, |n| (n / 1000) as usize)
}

fn element(z: usize) -> String {
    super::material::element(z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_rounded_and_converted() {
        assert_eq!(round(0.123_456_789), 0.123_457);
        assert_eq!(round(-2.5e-9), -2.5e-9);
        let mut f = MaterialForm::new(MaterialFields {
            entries: vec![("1001".into(), 2.0), ("8016".into(), 1.0)],
            keywords: vec![],
        });
        f.convert(true).unwrap();
        assert!(f.by_mass());
        assert!(
            (f.rows[0].fraction + 0.111_9).abs() < 1e-3,
            "{}",
            f.rows[0].fraction
        );
        f.convert(false).unwrap();
        assert!((f.rows[0].fraction - 2.0 / 3.0).abs() < 1e-5);
        assert_eq!(f.count(), 2);
        f.rows[0].fraction = 2.0;
        f.rows[1].fraction = 1.0;
        assert_eq!(f.count(), 0);
        f.normalise();
        assert!((f.rows[1].fraction - 0.333_333).abs() < 1e-9);
    }
}
