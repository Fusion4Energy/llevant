//! The Ruler tool (M): lays the ruler ([`crate::ruler`]) in either view.
//! Two points: click one then the other, or drag from one to the other.
//! Through the layers: click a face (in 3D) or near a boundary (in the
//! slice), and the line goes along the surface's normal through what is
//! behind it, up to the next void. The Ruler window lists the cells crossed.

use llevant::Vec3;

use crate::command::Command;
use crate::document::Document;
use crate::ruler::{measure, through_layers, RulerMode};
use crate::theme;
use crate::tools::Tool;
use crate::viewport::{ViewKind, ViewPort};

#[derive(Default)]
pub struct RulerTool {
    /// The first point clicked, waiting for the second (and in which view).
    first: Option<(ViewKind, Vec3)>,
    /// A drag from a point: in which view, from where.
    dragging: Option<(ViewKind, Vec3)>,
}

impl Tool for RulerTool {
    fn name(&self) -> &'static str {
        "Ruler"
    }

    fn icon(&self) -> &'static str {
        crate::icons::RULER
    }

    fn hint(&self) -> &'static str {
        "two points: click one then the other, or drag · through the layers: click a face · M: this tool · Esc: back to Select"
    }

    fn input(&mut self, view: &ViewPort, doc: &Document, out: &mut Vec<Command>) -> bool {
        let r = &view.response;
        let m = &doc.model;
        let level = doc.vis.level;
        let pick = |pos| view.pick(m, &doc.vis, pos);
        match doc.ruler_mode {
            RulerMode::TwoPoints => {
                if r.drag_started_by(egui::PointerButton::Primary) {
                    let from = r.ctx.input(|i| i.pointer.press_origin());
                    if let Some(p) = from.and_then(pick) {
                        self.dragging = Some((view.kind, p.point));
                        self.first = None;
                    }
                }
                if let Some((kind, a)) = self.dragging {
                    if kind == view.kind {
                        if let Some(p) = r.interact_pointer_pos().and_then(pick) {
                            out.push(Command::SetRuler(measure(
                                m,
                                a,
                                p.point,
                                RulerMode::TwoPoints,
                                level,
                            )));
                        }
                        if r.drag_stopped() {
                            self.dragging = None;
                        }
                        return true;
                    }
                }
                if r.clicked() {
                    if let Some(p) = r.interact_pointer_pos().and_then(pick) {
                        match self.first.take() {
                            Some((_, a)) => out.push(Command::SetRuler(measure(
                                m,
                                a,
                                p.point,
                                RulerMode::TwoPoints,
                                level,
                            ))),
                            None => self.first = Some((view.kind, p.point)),
                        }
                    }
                }
            }
            RulerMode::Layers => {
                self.first = None;
                if r.clicked() {
                    let Some(pos) = r.interact_pointer_pos() else {
                        return false;
                    };
                    let Some(p) = pick(pos) else {
                        return false;
                    };
                    match &p.surface {
                        Some(s) => {
                            // Away from the eye in 3D; in the slice, from the
                            // point clicked across the boundary near it.
                            let across = llevant::vec3::sub(s.point, p.point);
                            let towards = if view.kind == ViewKind::Slice
                                && llevant::vec3::norm(across) > 0.0
                            {
                                across
                            } else {
                                s.dir
                            };
                            let r = through_layers(m, s.shape, s.point, towards, level);
                            if r.is_none() {
                                out.push(Command::Message(
                                    "the ruler found no layers behind that surface".into(),
                                ));
                            }
                            out.push(Command::SetRuler(r));
                        }
                        None => out.push(Command::Message(
                            "click on a face, or near a boundary in the slice".into(),
                        )),
                    }
                }
            }
        }
        false
    }

    fn overlay(&self, view: &ViewPort, doc: &Document, painter: &egui::Painter) {
        // The first point, and the line to the pointer, while waiting for
        // the second.
        let Some((_, a)) = self.first else { return };
        let Some(sa) = view.to_screen(a) else { return };
        let c = egui::Color32::WHITE;
        theme::marker(painter, sa.pos, c, false);
        if let Some(pos) = view.response.hover_pos() {
            if let Some(p) = view.pick(&doc.model, &doc.vis, pos) {
                if let Some(sb) = view.to_screen(p.point) {
                    painter.line_segment([sa.pos, sb.pos], egui::Stroke::new(1.5, c));
                    let d = llevant::vec3::norm(llevant::vec3::sub(p.point, a));
                    painter.text(
                        sb.pos + egui::vec2(10.0, -10.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{d:.2} cm"),
                        egui::FontId::monospace(12.0),
                        c,
                    );
                }
            }
        }
    }
}
