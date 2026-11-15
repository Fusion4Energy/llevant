//! The FMESH panel, on the left of the Tally workspace: the meshes laid out and
//! the input's own, the selected one in detail (numbers typed as on its
//! card, MCNP's shortcuts included), how many bins it makes and what MCNP
//! would object to, and its cards to copy or save.

use std::collections::HashMap;

use crate::command::{Command, Edit};
use crate::document::Document;
use crate::mesh::{
    self, axis_names, axis_text, bins, cards_text, even, parse_axis, parse_numbers, parse_tr,
    tr_values, AxisKind, InputMeshes, MeshShape, MeshState, Problem, TrState,
};
use crate::{icons, theme};

const LO: [&str; 3] = ["lo x", "lo y", "lo z"];
const MESH: [&str; 3] = ["mesh i", "mesh j", "mesh k"];
const INTS: [&str; 3] = ["ints i", "ints j", "ints k"];

/// Text as typed in the window's fields, kept in step with the selected
/// mesh.
#[derive(Default)]
pub struct MeshPanel {
    texts: HashMap<&'static str, String>,
    /// The mesh the texts were last made from.
    filled_from: Option<(usize, MeshState)>,
    /// The field whose text could not be read, and why.
    error: Option<(&'static str, String)>,
    /// The TR's text is angles in degrees (`*TR`).
    degrees: bool,
}

impl MeshPanel {
    /// The panel (Tally workspace).
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
        ui.horizontal_wrapped(|ui| {
            let drawing = doc.editing.drawing_mesh;
            for (cyl, name, hint) in [
                (false, "Box", "Then drag in the slice: a new FMESH box"),
                (
                    true,
                    "Cylinder",
                    "Then drag in the slice from the axis out: a new FMESH cylinder",
                ),
            ] {
                let on = drawing == Some(cyl);
                let r = ui
                    .selectable_label(on, format!("+ {name}"))
                    .on_hover_text(hint);
                if r.clicked() {
                    out.push(Command::DrawMesh((!on).then_some(cyl)));
                }
            }
        });
        if let Some(cyl) = doc.editing.drawing_mesh {
            ui.label(
                egui::RichText::new(format!(
                    "Drag in the slice: a new {} (Esc: cancel)",
                    if cyl { "cylinder" } else { "box" }
                ))
                .color(theme::palette(ui.ctx()).accent),
            );
        }
        let meshes = &doc.scene.meshes;
        let input = doc.input_meshes.as_deref();
        if !meshes.is_empty() {
            ui.add_space(4.0);
            for (k, m) in meshes.iter().enumerate() {
                ui.horizontal(|ui| {
                    let bad = m
                        .problems(k, meshes, input, &doc.region)
                        .iter()
                        .any(|p| matches!(p, Problem::Error(_)));
                    let title = format!(
                        "{}FMESH{}:{} {}",
                        if bad { "⚠ " } else { "" },
                        m.id,
                        m.particle,
                        summary(m)
                    );
                    if ui
                        .selectable_label(doc.editing.mesh_selected == Some(k), theme::data(title))
                        .clicked()
                    {
                        out.push(Command::SelectMesh(Some(k)));
                    }
                    if ui
                        .small_button(icons::DELETE)
                        .on_hover_text("Remove it")
                        .clicked()
                    {
                        out.push(Edit::RemoveMesh(k).into());
                    }
                });
            }
        }
        if let Some(i) = input.filter(|i| !i.cards.is_empty()) {
            egui::CollapsingHeader::new(format!("In the input ({})", i.cards.len()))
                .id_salt("input fmeshes")
                .show(ui, |ui| {
                    for c in &i.cards {
                        ui.horizontal(|ui| match c {
                            Ok(c) => {
                                let m = MeshState::from_card(c);
                                ui.label(theme::data(format!(
                                    "FMESH{} (line {}) {}",
                                    c.id,
                                    c.line,
                                    summary(&m)
                                )));
                                if ui
                                    .small_button(icons::COPY)
                                    .on_hover_text("Edit a copy of it (under a free tally number)")
                                    .clicked()
                                {
                                    let id = InputMeshes::free_tally(Some(i), meshes);
                                    out.push(Edit::AddMesh(Box::new(MeshState { id, ..m })).into());
                                }
                            }
                            Err(e) => {
                                ui.colored_label(theme::palette(ui.ctx()).warning, e.to_string());
                            }
                        });
                    }
                });
        }
        let Some(k) = doc.editing.mesh_selected.filter(|&k| k < meshes.len()) else {
            if meshes.is_empty() {
                ui.label(egui::RichText::new("No meshes yet.").weak());
            }
            return;
        };
        ui.separator();
        self.selected(ui, doc, k, out);
    }

    /// The selected mesh in detail.
    fn selected(&mut self, ui: &mut egui::Ui, doc: &Document, k: usize, out: &mut Vec<Command>) {
        let m = &doc.scene.meshes[k];
        if self.filled_from.as_ref() != Some(&(k, m.clone())) {
            self.fill(m);
            self.filled_from = Some((k, m.clone()));
            self.error = None;
            self.degrees = false;
        }
        let input = doc.input_meshes.as_deref();
        let mut next: Option<MeshState> = None;

        egui::Grid::new("mesh card")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("Tally");
                ui.horizontal(|ui| {
                    ui.label("FMESH");
                    if self.field(ui, "id", 60.0) {
                        next = self.read(m, "id", |m, t| {
                            m.id = t.trim().parse().map_err(|_| "a tally number".to_string())?;
                            Ok(())
                        });
                    }
                    ui.label(":");
                    if self.field(ui, "particle", 40.0) {
                        next = self.read(m, "particle", |m, t| {
                            m.particle = t.trim().to_lowercase();
                            Ok(())
                        });
                    }
                    let mut star = m.energy_weighted;
                    if ui
                        .checkbox(&mut star, "*")
                        .on_hover_text("*FMESH: energy-weighted (MeV/cm²)")
                        .changed()
                    {
                        next = Some(MeshState {
                            energy_weighted: star,
                            ..m.clone()
                        });
                    }
                });
                ui.end_row();
                ui.label("Comment");
                if self.field(ui, "comment", 240.0) {
                    next = self.read(m, "comment", |m, t| {
                        m.comment = t.trim().into();
                        Ok(())
                    });
                }
                ui.end_row();
                if let MeshShape::Cyl { .. } = m.shape {
                    for key in ["origin", "axs", "vec"] {
                        ui.label(key.to_uppercase());
                        if self.field(ui, key, 240.0) {
                            next = self.read(m, key, |m, t| {
                                let v: [f64; 3] = parse_numbers(t)?
                                    .try_into()
                                    .map_err(|_| "three numbers".to_string())?;
                                if let MeshShape::Cyl {
                                    origin, axs, vec, ..
                                } = &mut m.shape
                                {
                                    *match key {
                                        "origin" => origin,
                                        "axs" => axs,
                                        _ => vec,
                                    } = v;
                                }
                                Ok(())
                            });
                        }
                        ui.end_row();
                    }
                }
            });

        // The bins: coarse bounds and fine counts as on the card.
        let cyl = m.is_cylinder();
        let names = axis_names(cyl);
        let local = if m.tr.is_some() && !cyl {
            " (its own, before TR)"
        } else {
            ""
        };
        ui.label(egui::RichText::new(format!("Bins{local}")).strong());
        egui::Grid::new("mesh bins")
            .num_columns(5)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                ui.label("");
                ui.label(if cyl { "" } else { "from" });
                ui.label("coarse bounds");
                ui.label("fine bins each");
                ui.label("bins");
                ui.end_row();
                for a in 0..3 {
                    ui.label(names[a]);
                    let mut changed = false;
                    if cyl {
                        ui.label("0");
                    } else {
                        changed |= self.field(ui, LO[a], 50.0);
                    }
                    changed |= self.field(ui, MESH[a], 130.0);
                    changed |= self.field(ui, INTS[a], 70.0);
                    if changed {
                        let kind = match (cyl, a) {
                            (false, _) => AxisKind::Rect,
                            (true, 0) => AxisKind::CylR,
                            (true, 1) => AxisKind::CylZ,
                            _ => AxisKind::CylTheta,
                        };
                        let (lo, mesh_t, ints_t) =
                            (self.text(LO[a]), self.text(MESH[a]), self.text(INTS[a]));
                        next = self.read(m, MESH[a], |m, _| {
                            let lo = if cyl { 0.0 } else { one_number(&lo)? };
                            *m.axes_mut()[a] = parse_axis(kind, lo, &mesh_t, &ints_t)?;
                            Ok(())
                        });
                    }
                    let v = m.axes()[a];
                    let mut n = bins(v);
                    if ui
                        .add(egui::DragValue::new(&mut n).range(1..=100_000).speed(0.2))
                        .on_hover_text("Make them this many even bins")
                        .changed()
                    {
                        let mut x = m.clone();
                        *x.axes_mut()[a] = even(v[0], *v.last().unwrap_or(&v[0]), n);
                        next = Some(x);
                    }
                    ui.end_row();
                }
                for (name, mesh_key, ints_key, kind) in [
                    ("energy", "emesh", "eints", AxisKind::Energy),
                    ("time", "tmesh", "tints", AxisKind::Time),
                ] {
                    ui.label(name);
                    ui.label(if kind == AxisKind::Energy {
                        "0"
                    } else {
                        "-1e36"
                    });
                    let changed = self.field(ui, mesh_key, 130.0) | self.field(ui, ints_key, 70.0);
                    if changed {
                        let (mesh_t, ints_t) = (self.text(mesh_key), self.text(ints_key));
                        next = self.read(m, mesh_key, |m, _| {
                            let b = if mesh_t.trim().is_empty() {
                                None
                            } else {
                                Some(parse_axis(kind, 0.0, &mesh_t, &ints_t)?)
                            };
                            match kind {
                                AxisKind::Energy => m.energy = b,
                                _ => m.time = b,
                            }
                            Ok(())
                        });
                    }
                    ui.end_row();
                }
            });
        ui.horizontal(|ui| {
            ui.label("OUT=");
            let mut out_choice = m.out.clone();
            egui::ComboBox::from_id_salt("mesh out")
                .selected_text(out_choice.as_deref().unwrap_or("(col)"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut out_choice, None, "(col)");
                    for o in mesh::OUTS {
                        ui.selectable_value(&mut out_choice, Some(o.to_string()), o);
                    }
                });
            if out_choice != m.out {
                next = Some(MeshState {
                    out: out_choice,
                    ..m.clone()
                });
            }
        });

        // The placement.
        ui.horizontal(|ui| {
            let mut on = m.tr.is_some();
            if ui
                .checkbox(&mut on, "TR=")
                .on_hover_text("Placed by a TRn card (a box turns only this way)")
                .changed()
            {
                let tr = on.then(|| {
                    let n = InputMeshes::free_tr(input, &doc.scene.meshes);
                    TrState::of(n, &aci_tallies_geom::Transform::IDENTITY)
                });
                next = Some(MeshState { tr, ..m.clone() });
            }
            if let Some(t) = m.tr {
                let mut n = t.number;
                if ui
                    .add(egui::DragValue::new(&mut n).range(1..=999))
                    .changed()
                {
                    next = Some(MeshState {
                        tr: Some(TrState { number: n, ..t }),
                        ..m.clone()
                    });
                }
                ui.checkbox(&mut self.degrees, "angles in degrees (*TR)");
            }
        });
        if let Some(t) = m.tr {
            if self.field(ui, "tr", 360.0) {
                let (text, degrees) = (self.text("tr"), self.degrees);
                next = self.read(m, "tr", |m, _| {
                    m.tr = Some(parse_tr(t.number, &text, degrees)?);
                    Ok(())
                });
            }
        }
        if let Some((key, e)) = &self.error {
            ui.colored_label(theme::palette(ui.ctx()).error, format!("{key}: {e}"));
        }

        // Fitting and placing it.
        ui.horizontal_wrapped(|ui| {
            let selection = crate::selection::selection_box(
                &doc.model,
                &doc.index,
                doc.selected(),
                &doc.region,
            );
            if ui
                .add_enabled(selection.is_some(), egui::Button::new("Fit to selection"))
                .on_hover_text("Around the selected cells, rounded out; bins kept")
                .clicked()
            {
                if let Some(b) = &selection {
                    next = Some(m.fitted(b));
                }
            }
            if ui
                .button("Fit to the model")
                .on_hover_text("Around what the views frame")
                .clicked()
            {
                next = Some(m.fitted(&doc.region));
            }
            if ui
                .button("Along the slice")
                .on_hover_text(
                    "A box along the slice's axes (by its TR), a cylinder about its normal",
                )
                .clicked()
            {
                let f = doc.slice.frame();
                let tr = InputMeshes::free_tr(input, &doc.scene.meshes);
                next = Some(m.aligned([f.u, f.v, f.n], tr));
            }
            if ui.button(icons::with(icons::FRAME, "Frame")).clicked() {
                if let Some(b) = m.world_box() {
                    out.push(Command::FrameBox(b));
                }
            }
        });

        // How big it is, and what is wrong with it.
        let s = m.stats();
        let dims: Vec<String> = m.axes().iter().map(|v| bins(v).to_string()).collect();
        ui.label(format!(
            "{} voxels ({}) · {} values · meshtal ≈ {}",
            s.voxels,
            dims.join(" × "),
            s.bins,
            mesh::bytes(s.meshtal_bytes)
        ));
        let sizes: Vec<String> = names
            .iter()
            .zip(s.sizes)
            .map(|(n, (lo, hi))| {
                if (hi - lo).abs() <= 1e-9 * hi.abs() {
                    format!("{n} {}", short(hi))
                } else {
                    format!("{n} {}–{}", short(lo), short(hi))
                }
            })
            .collect();
        ui.label(
            egui::RichText::new(format!("bins of {} cm", sizes.join(", ")))
                .color(theme::palette(ui.ctx()).text_weak),
        );
        for p in m.problems(k, &doc.scene.meshes, input, &doc.region) {
            let pal = theme::palette(ui.ctx());
            match p {
                Problem::Error(e) => ui.colored_label(pal.error, e),
                Problem::Warning(w) => ui.colored_label(pal.warning, w),
            };
        }

        // The cards.
        let text = cards_text([m], input);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match &text {
                Ok(t) => {
                    ui.add(egui::Label::new(theme::data(t.trim_end()).size(11.5)).selectable(true));
                }
                Err(e) => {
                    ui.colored_label(theme::palette(ui.ctx()).error, e);
                }
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    text.is_ok(),
                    egui::Button::new(icons::with(icons::COPY, "Copy")),
                )
                .on_hover_text("Its cards, to paste in the input's data cards")
                .clicked()
            {
                out.push(Command::CopyMeshes(vec![k]));
            }
            if ui
                .add_enabled(
                    text.is_ok(),
                    egui::Button::new(icons::with(icons::SAVE, "Save…")),
                )
                .clicked()
            {
                out.push(Command::SaveMeshes(vec![k]));
            }
            let all: Vec<usize> = (0..doc.scene.meshes.len()).collect();
            if all.len() > 1
                && ui
                    .button(icons::with(icons::COPY, "Copy all"))
                    .on_hover_text("The cards of every mesh here, each TR once")
                    .clicked()
            {
                out.push(Command::CopyMeshes(all));
            }
        });

        if let Some(n) = next.filter(|n| n != m) {
            out.push(Edit::SetMesh(k, Box::new(n)).into());
        }
    }

    /// The fields' texts from `m`.
    fn fill(&mut self, m: &MeshState) {
        let t = &mut self.texts;
        t.insert("id", m.id.to_string());
        t.insert("particle", m.particle.clone());
        t.insert("comment", m.comment.clone());
        for (a, v) in m.axes().into_iter().enumerate() {
            let (lo, coarse, ints) = axis_text(v);
            t.insert(LO[a], lo);
            t.insert(MESH[a], coarse);
            t.insert(INTS[a], ints);
        }
        if let MeshShape::Cyl {
            origin, axs, vec, ..
        } = &m.shape
        {
            for (key, v) in [("origin", origin), ("axs", axs), ("vec", vec)] {
                t.insert(key, v.map(mesh::number).join(" "));
            }
        }
        for (mesh_key, ints_key, b) in [("emesh", "eints", &m.energy), ("tmesh", "tints", &m.time)]
        {
            let (coarse, ints) = b.as_ref().map_or((String::new(), String::new()), |b| {
                let (_, c, i) = axis_text(b);
                (c, i)
            });
            t.insert(mesh_key, coarse);
            t.insert(ints_key, ints);
        }
        t.insert("tr", m.tr.as_ref().map(tr_values).unwrap_or_default());
    }

    fn text(&self, key: &str) -> String {
        self.texts.get(key).cloned().unwrap_or_default()
    }

    /// A text field; true when its text was given (it lost the focus, or
    /// Enter was pressed).
    fn field(&mut self, ui: &mut egui::Ui, key: &'static str, width: f32) -> bool {
        let wrong = self.error.as_ref().is_some_and(|e| e.0 == key);
        let text = self.texts.entry(key).or_default();
        let mut edit = egui::TextEdit::singleline(text)
            .desired_width(width)
            .font(egui::TextStyle::Monospace);
        if wrong {
            edit = edit.text_color(theme::palette(ui.ctx()).error);
        }
        let r = ui.add(edit);
        theme::name(&r, key);
        r.lost_focus()
    }

    /// `m` changed by `change` from the field `key`, or `None` (and the
    /// field marked) if its text can't be read.
    fn read(
        &mut self,
        m: &MeshState,
        key: &'static str,
        change: impl FnOnce(&mut MeshState, &str) -> Result<(), String>,
    ) -> Option<MeshState> {
        let mut x = m.clone();
        let text = self.text(key);
        match change(&mut x, &text) {
            Ok(()) => {
                if self.error.as_ref().is_some_and(|e| e.0 == key) {
                    self.error = None;
                }
                Some(x)
            }
            Err(e) => {
                self.error = Some((key, e));
                None
            }
        }
    }
}

fn one_number(t: &str) -> Result<f64, String> {
    match parse_numbers(t)?.as_slice() {
        [v] => Ok(*v),
        _ => Err("one number".into()),
    }
}

/// A length to 3 significant digits.
fn short(x: f64) -> String {
    if x == 0.0 || !x.is_finite() {
        return format!("{x}");
    }
    let digits = (2 - x.abs().log10().floor() as i32).max(0) as usize;
    format!("{x:.digits$}")
}

/// What a mesh is, in a few words: `box 20×20×10`, `cylinder 5×12×1`.
fn summary(m: &MeshState) -> String {
    let dims: Vec<String> = m.axes().iter().map(|v| bins(v).to_string()).collect();
    let kind = if m.is_cylinder() { "cylinder" } else { "box" };
    let tr = m.tr.map(|t| format!(" TR{}", t.number)).unwrap_or_default();
    format!("{kind} {}{tr}", dims.join("×"))
}
