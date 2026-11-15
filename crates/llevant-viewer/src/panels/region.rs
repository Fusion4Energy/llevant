//! What a volume or check job covers and how finely: the region (what is
//! shown, the whole problem, the selection, the section box, what the last
//! reload changed) and the number of grid lines (the spacing follows from
//! both).

use std::sync::Arc;

use llevant::{Aabb, Scope};

use crate::document::Document;
use crate::jobs::{short, spacing_for};
use crate::selection::selection_box;
use crate::state::CutState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Shown,
    Problem,
    Selection,
    SectionBox,
    /// The boxes of what the last reload changed (checks only).
    Changed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fineness {
    Coarse,
    Medium,
    Fine,
}

impl Fineness {
    /// Grid lines across the region, along the three axes together.
    fn lines(self) -> f64 {
        match self {
            Fineness::Coarse => 1e5,
            Fineness::Medium => 1e6,
            Fineness::Fine => 1e7,
        }
    }
}

pub struct RegionPicker {
    pub area: Area,
    pub fineness: Fineness,
    /// Halved from the last run ("refine").
    pub halvings: u32,
}

impl Default for RegionPicker {
    fn default() -> RegionPicker {
        RegionPicker {
            area: Area::Shown,
            fineness: Fineness::Coarse,
            halvings: 0,
        }
    }
}

/// A region, its spacing and what it is.
pub struct Picked {
    pub region: Aabb,
    pub h: f64,
    pub scope: String,
    /// Only the lines within these boxes (of `region`'s grid).
    pub within: Option<Arc<Vec<Aabb>>>,
}

impl RegionPicker {
    /// The choice; the region and spacing it gives, or why there is none.
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &Document) -> Result<Picked, String> {
        self.ui_with(ui, doc, false)
    }

    /// [`RegionPicker::ui`], offering what the last reload changed when
    /// `changed` and there is something.
    pub fn ui_with(
        &mut self,
        ui: &mut egui::Ui,
        doc: &Document,
        changed: bool,
    ) -> Result<Picked, String> {
        let changed = changed && doc.changed_boxes.is_some();
        if self.area == Area::Changed && !changed {
            self.area = Area::Shown;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("over").weak());
            let areas = [
                (Area::Shown, "what is shown"),
                (Area::Problem, "the problem"),
                (Area::Selection, "the selection"),
                (Area::SectionBox, "the section box"),
                (Area::Changed, "what changed"),
            ];
            for (a, name) in areas
                .into_iter()
                .filter(|a| changed || a.0 != Area::Changed)
            {
                if ui.selectable_label(self.area == a, name).clicked() {
                    self.area = a;
                    self.halvings = 0;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("lines").weak());
            for (f, name) in [
                (Fineness::Coarse, "10⁵"),
                (Fineness::Medium, "10⁶"),
                (Fineness::Fine, "10⁷"),
            ] {
                if ui.selectable_label(self.fineness == f, name).clicked() {
                    self.fineness = f;
                    self.halvings = 0;
                }
            }
        });
        let picked = self.pick(doc);
        match &picked {
            Ok(p) => {
                ui.label(egui::RichText::new(format!("{}, h = {} cm", p.scope, short(p.h))).weak());
            }
            Err(e) => {
                ui.label(egui::RichText::new(e).weak());
            }
        }
        picked
    }

    pub fn pick(&self, doc: &Document) -> Result<Picked, String> {
        if self.area == Area::Changed {
            return changed(doc, self.fineness.lines()).map(|mut p| {
                p.h /= 2f64.powi(self.halvings as i32);
                p
            });
        }
        let (region, scope) = match self.area {
            Area::Shown => (doc.region, "what is shown".to_string()),
            Area::Problem => (doc.model.bounds(Scope::Problem), "the problem".to_string()),
            Area::Selection => {
                let b = selection_box(&doc.model, &doc.index, &doc.selection.cells, &doc.region)
                    .ok_or("select something first")?;
                let what = doc
                    .selection
                    .query
                    .as_ref()
                    .map_or(String::new(), |q| q.to_string());
                (b, format!("around {what}"))
            }
            Area::Changed => unreachable!("above"),
            Area::SectionBox => {
                let b = doc
                    .scene
                    .cuts_on()
                    .find_map(|(_, c)| match *c {
                        CutState::Box { lo, hi, keep: true } => Some(Aabb { lo, hi }),
                        _ => None,
                    })
                    .ok_or("add a section box first (the 3D view's Section menu)")?;
                (b, "the section box".to_string())
            }
        };
        if !region.is_finite() || region.is_empty() {
            return Err(format!("{scope} has no finite extent"));
        }
        let h = spacing_for(&region, self.fineness.lines()) / 2f64.powi(self.halvings as i32);
        Ok(Picked {
            region,
            h,
            scope,
            within: None,
        })
    }
}

/// What the last reload changed, checked on about `lines` lines.
pub fn changed(doc: &Document, lines: f64) -> Result<Picked, String> {
    let boxes = doc
        .changed_boxes
        .clone()
        .ok_or("reload a changed input first")?;
    let problem = doc.model.bounds(Scope::Problem);
    let region = boxes
        .iter()
        .fold(Aabb::EMPTY, |a, b| a.union(b))
        .intersect(&problem);
    if !region.is_finite() || region.is_empty() {
        return Err("what changed has no finite extent".into());
    }
    let h = llevant::spacing_within(&region, &boxes, lines)
        .ok_or("what changed is outside the problem")?;
    Ok(Picked {
        region,
        h,
        scope: "what the reload changed".into(),
        within: Some(boxes),
    })
}
