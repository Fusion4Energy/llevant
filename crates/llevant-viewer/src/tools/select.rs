use llevant_scene::query::Query;

use crate::command::Command;
use crate::document::Document;
use crate::selection::SelectMode;
use crate::tools::Tool;
use crate::viewport::{Projection, ViewKind, ViewPort};

/// Samples along each side of a box at most: the cells a box selects are
/// those seen at these points (in 3D the first visible along each ray).
const BOX_SAMPLES: usize = 64;
/// A drag shorter than this (points) is not a box.
const BOX_MIN: f32 = 4.0;

/// Click to select the cell there (Ctrl adds or removes, Shift removes);
/// double-click to mark the point and see it in the inspector; drag a box
/// to select the cells seen in it (with the same modifiers). Clicking the
/// 3D background clears both. With Alt, the surface there instead: marked
/// while Alt is held, opened in the inspector on a click (again at the same
/// place: the next surface through it).
#[derive(Default)]
pub struct SelectTool {
    /// A box being dragged: in which view, from where, to where.
    boxing: Option<(ViewKind, egui::Pos2, egui::Pos2)>,
}

impl Tool for SelectTool {
    fn name(&self) -> &'static str {
        "Select"
    }

    fn icon(&self) -> &'static str {
        crate::icons::SELECT
    }

    fn hint(&self) -> &'static str {
        "click: select (Ctrl adds, Shift removes) · Alt+click: surface · double-click: inspect the point · drag: box select · middle-drag: orbit · right-click: menu"
    }

    fn input(&mut self, view: &ViewPort, doc: &Document, out: &mut Vec<Command>) -> bool {
        let r = &view.response;
        let mode = SelectMode::from_modifiers(r.ctx.input(|i| i.modifiers));
        // A box: from where the left button went down to where it is.
        if r.drag_started_by(egui::PointerButton::Primary) {
            if let Some(from) = r.ctx.input(|i| i.pointer.press_origin()) {
                self.boxing = Some((view.kind, from, from));
            }
        }
        if let Some((kind, from, to)) = &mut self.boxing {
            if *kind == view.kind {
                if let Some(p) = r.interact_pointer_pos() {
                    *to = p;
                }
                if r.drag_stopped() {
                    let rect = egui::Rect::from_two_pos(*from, *to);
                    self.boxing = None;
                    if rect.width().max(rect.height()) >= BOX_MIN {
                        let ids = box_cells(view, doc, rect.intersect(view.rect()));
                        if !ids.is_empty() {
                            out.push(Command::Select(Query::Cells(ids), mode));
                        } else if mode == SelectMode::Replace {
                            out.push(Command::ClearSelection);
                        }
                    }
                }
                return true;
            }
        }
        let Some(pos) = r.interact_pointer_pos() else {
            return false;
        };
        if !r.clicked() {
            return false;
        }
        let picked = view.pick(&doc.model, &doc.vis, pos);
        if r.ctx.input(|i| i.modifiers.alt) {
            if picked.is_some() {
                out.push(Command::PickSurface(picked));
            }
            return false;
        }
        if r.double_clicked() {
            // Selected as by the first click (which may have come in the
            // same frame), without toggling it off again; then marked.
            let mode = match mode {
                SelectMode::Toggle => SelectMode::Add,
                m => m,
            };
            out.push(Command::Pick(picked.clone(), mode));
            if picked.is_some() {
                out.push(Command::Focus(picked));
            }
        } else {
            out.push(Command::Pick(picked, mode));
        }
        false
    }

    fn overlay(&self, view: &ViewPort, doc: &Document, painter: &egui::Painter) {
        surface_tag(view, doc, painter);
        if let Some((kind, from, to)) = self.boxing {
            if kind == view.kind {
                let rect = egui::Rect::from_two_pos(from, to);
                let c = crate::theme::SELECTION;
                painter.rect(
                    rect,
                    0.0,
                    c.gamma_multiply(0.12),
                    egui::Stroke::new(1.5, c),
                    egui::StrokeKind::Middle,
                );
            }
        }
    }
}

/// With Alt held over `view`: the surface an Alt+click would pick, named
/// at the pointer (its trace is marked in both views, [`alt_marks`]).
fn surface_tag(view: &ViewPort, doc: &Document, painter: &egui::Painter) {
    let r = &view.response;
    let (Some(pos), true) = (r.hover_pos(), r.ctx.input(|i| i.modifiers.alt)) else {
        return;
    };
    let Some(h) = &doc.pointer.hover else {
        return;
    };
    let m = &doc.model;
    let text = match h.surface.as_ref().and_then(|s| m.surface_id(s.shape)) {
        Some(id) => {
            let kind = doc
                .source
                .as_ref()
                .and_then(|src| src.surface_coeffs(id.id))
                .map(|(k, _)| format!("  {}", k.to_uppercase()))
                .unwrap_or_default();
            format!("s {}{kind}", crate::pick::surface_name(id))
        }
        None => "no surface here".to_string(),
    };
    let font = egui::FontId::monospace(12.0);
    let at = pos + egui::vec2(14.0, 10.0);
    let galley = painter.layout_no_wrap(text, font, crate::theme::MARKED_SURFACE);
    let bg = egui::Rect::from_min_size(at, galley.size()).expand(4.0);
    painter.rect_filled(bg, 3.0, egui::Color32::from_black_alpha(190));
    painter.galley(at, galley, egui::Color32::WHITE);
}

/// What Alt held over a view marks: the surface under the pointer.
pub fn alt_marks(ctx: &egui::Context, doc: &Document) -> Option<crate::marks::MarkRequest> {
    if !ctx.input(|i| i.modifiers.alt) {
        return None;
    }
    let h = doc.pointer.hover.as_ref()?;
    crate::marks::MarkRequest::surface(&doc.model, h, h.surface.as_ref()?.shape)
}

/// The MCNP numbers of the cells seen in `rect` of `view`: on a grid of at
/// most [`BOX_SAMPLES`]² points, in 3D the first visible cell along each
/// ray (one batch of `first_hits`), in the slice the cell at each point.
fn box_cells(view: &ViewPort, doc: &Document, rect: egui::Rect) -> Vec<i64> {
    let m = &doc.model;
    let side = rect.width().max(rect.height()).max(1.0);
    let step = (side / BOX_SAMPLES as f32).max(1.0);
    let nx = (rect.width() / step).ceil().max(1.0) as usize;
    let ny = (rect.height() / step).ceil().max(1.0) as usize;
    let at = |i: usize, j: usize| {
        rect.min + egui::vec2((i as f32 + 0.5) * step, (j as f32 + 0.5) * step)
    };
    let mut cells: Vec<u32> = Vec::new();
    match &view.projection {
        Projection::Camera(cam) => {
            let rays: Vec<_> = (0..ny)
                .flat_map(|j| (0..nx).map(move |i| (i, j)))
                .filter_map(|(i, j)| view.pixel(at(i, j)))
                .filter_map(|(i, j)| cam.ray(i, j))
                .collect();
            if let Ok(h) = m.first_hits(&rays, &doc.vis, None) {
                for &id in &h.ids {
                    if id != llevant::NO_HIT {
                        cells.extend(h.labels[id as usize].cell());
                    }
                }
            }
        }
        Projection::Plane(plane) => {
            let full = view.rect();
            for j in 0..ny {
                for i in 0..nx {
                    let p = plane.from_screen(full, at(i, j));
                    cells.extend(m.classify_point(p).cell());
                }
            }
        }
    }
    cells.sort_unstable();
    cells.dedup();
    let mut ids: Vec<i64> = cells.into_iter().map(|c| m.cell_id(c)).collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}
