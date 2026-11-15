//! The field along the ruler (Results workspace): its values at evenly
//! spaced points of the line, on the field's scale (log or linear), with the
//! cells' boundaries marked and the longer cells named; Copy gives the
//! points as CSV.

use crate::document::Document;
use crate::icons;
use crate::ruler::Ruler;

/// Points the line is sampled at.
const SAMPLES: usize = 400;

/// The field's values along `r` (distance, value; `None` where it has
/// none).
pub fn samples(doc: &Document, r: &Ruler) -> Vec<(f64, Option<f64>)> {
    let Some(f) = &doc.field else {
        return Vec::new();
    };
    let m = &doc.model;
    let len = r.length();
    (0..=SAMPLES)
        .map(|k| {
            let d = len * k as f64 / SAMPLES as f64;
            let p = r.at(d);
            let label = m.classify_point(p);
            (d, f.field.value(m, &label, p))
        })
        .collect()
}

pub fn ui(ui: &mut egui::Ui, doc: &Document) {
    let (Some(r), Some(f)) = (&doc.ruler, &doc.field) else {
        return;
    };
    let pts = samples(doc, r);
    let scale = f.look.scale;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(icons::with(icons::RULER, "Along the ruler")).strong());
        ui.label(egui::RichText::new(format!("{} · {:.2} cm", f.field.title(), r.length())).weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button(icons::with(icons::COPY, "Copy (CSV)"))
                .clicked()
            {
                let mut csv = format!("distance_cm,{}\n", f.field.title().replace(',', ";"));
                for (d, v) in &pts {
                    csv.push_str(&format!(
                        "{d:.6},{}\n",
                        v.map_or(String::new(), |v| v.to_string())
                    ));
                }
                ui.ctx().copy_text(csv);
            }
        });
    });
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    let p = ui.painter_at(rect);
    let ink = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let line = ui.visuals().selection.bg_fill;
    let font = egui::FontId::monospace(10.0);
    let plot = egui::Rect::from_min_max(
        rect.min + egui::vec2(52.0, 6.0),
        rect.max - egui::vec2(10.0, 20.0),
    );
    if plot.width() < 20.0 || plot.height() < 20.0 {
        return;
    }
    let len = r.length().max(1e-12);
    let x = |d: f64| plot.left() + plot.width() * (d / len) as f32;
    let y = |v: f64| plot.bottom() - plot.height() * scale.fraction(v) as f32;
    // Grid at the scale's ticks.
    for (v, label) in scale.ticks() {
        let yy = y(v);
        p.line_segment(
            [egui::pos2(plot.left(), yy), egui::pos2(plot.right(), yy)],
            egui::Stroke::new(1.0, weak.gamma_multiply(0.3)),
        );
        p.text(
            egui::pos2(plot.left() - 6.0, yy),
            egui::Align2::RIGHT_CENTER,
            label,
            font.clone(),
            weak,
        );
    }
    // The cells' boundaries, and the names of those wide enough.
    let m = &doc.model;
    for s in &r.stretches {
        let x0 = x(s.from);
        if s.from > 0.0 {
            p.line_segment(
                [egui::pos2(x0, plot.top()), egui::pos2(x0, plot.bottom())],
                egui::Stroke::new(1.0, weak.gamma_multiply(0.5)),
            );
        }
        let w = x(s.to) - x0;
        if w > 46.0 {
            let name = s.cell.map_or_else(
                || crate::ruler::status_name(s.status).to_string(),
                |c| format!("c {}", m.cell_id(c)),
            );
            p.text(
                egui::pos2(x0 + 0.5 * w, plot.bottom() + 3.0),
                egui::Align2::CENTER_TOP,
                name,
                font.clone(),
                weak,
            );
        }
    }
    p.rect_stroke(
        plot,
        0.0,
        egui::Stroke::new(1.0, weak.gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );
    // The values: a line through the points that have one.
    let mut run: Vec<egui::Pos2> = Vec::new();
    let flush = |run: &mut Vec<egui::Pos2>| {
        if run.len() > 1 {
            p.add(egui::Shape::line(
                std::mem::take(run),
                egui::Stroke::new(2.0, line),
            ));
        }
        run.clear();
    };
    for &(d, v) in &pts {
        match v.filter(|v| v.is_finite() && (!scale.log || *v > 0.0)) {
            Some(v) => run.push(egui::pos2(x(d), y(v))),
            None => flush(&mut run),
        }
    }
    flush(&mut run);
    // The value under the pointer.
    if let Some(pos) = resp.hover_pos().filter(|q| plot.contains(*q)) {
        let d = ((pos.x - plot.left()) / plot.width()) as f64 * len;
        let k = ((d / len) * SAMPLES as f64).round() as usize;
        if let Some(&(d, Some(v))) = pts.get(k.min(SAMPLES)) {
            p.line_segment(
                [
                    egui::pos2(x(d), plot.top()),
                    egui::pos2(x(d), plot.bottom()),
                ],
                egui::Stroke::new(1.0, ink),
            );
            p.text(
                egui::pos2(x(d) + 6.0, plot.top() + 4.0),
                egui::Align2::LEFT_TOP,
                format!("{d:.2} cm · {}", llevant_render::scale::value_label(v)),
                font,
                ink,
            );
        }
    }
}
