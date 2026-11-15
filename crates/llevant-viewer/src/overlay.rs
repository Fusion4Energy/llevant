//! What the document marks over both views: pins with their banners, the
//! ruler with a tick at each boundary it crosses, the selection, and (in 3D)
//! the point a drag orbits about and the problems found (gaps and overlaps,
//! through what hides them). Drawn every frame over the traced
//! image, so they follow the camera at once, however long tracing takes.
//! In the slice, points are projected onto the plane; those off it are
//! faded and have no banner.

use llevant_render::legend;
use llevant_render::scale::Scale;

use crate::state::Legend;

use crate::document::Document;
use crate::theme;
use crate::viewport::{ViewKind, ViewPort};

/// Points further off the slice plane than this (screen points) are faded.
const NEAR_PLANE: f32 = 3.0;

pub fn draw(view: &ViewPort, doc: &Document, painter: &egui::Painter) {
    let rect = view.rect();
    // The field's scale, or (when there is none) the densities'.
    let area = legend::Rect::new(rect.left(), rect.top(), rect.right(), rect.bottom());
    if let Some(l) = legend(doc, area) {
        colour_bar(painter, &l);
    }
    if view.kind == ViewKind::ThreeD && doc.slice_shown_in_3d() {
        slice_outline(view, doc, painter);
    }
    if view.kind == ViewKind::ThreeD && doc.problems_on_top {
        problems(view, doc, painter);
    }
    if view.kind == ViewKind::ThreeD {
        if let Some(s) = doc.pointer.drag_pivot.and_then(|p| view.to_screen(p)) {
            if rect.contains(s.pos) {
                theme::marker(painter, s.pos, theme::PIVOT, false);
            }
        }
    }
    if view.kind == ViewKind::Slice {
        marks(view, doc, painter);
    }
    if let Some(r) = &doc.ruler {
        ruler(view, r, painter);
    }
    for (k, (pin, info)) in doc.scene.pins.iter().zip(&doc.pin_info).enumerate() {
        let Some(s) = view.to_screen(pin.point) else {
            continue;
        };
        if !rect.contains(s.pos) {
            continue;
        }
        if s.off_plane > NEAR_PLANE {
            theme::marker(
                painter,
                s.pos,
                theme::PIN.gamma_multiply(theme::FADED),
                false,
            );
            continue;
        }
        theme::marker(painter, s.pos, theme::PIN, false);
        let title = if pin.note.is_empty() {
            format!("Pin {}", k + 1)
        } else {
            format!("{} · {}", k + 1, pin.note)
        };
        theme::banner(
            painter,
            rect,
            s.pos,
            theme::PIN,
            &title,
            &info.summary(&doc.model),
        );
    }
    if let Some(sel) = &doc.focus {
        if let Some(s) = view.to_screen(sel.point) {
            if rect.contains(s.pos) {
                let c = if s.off_plane > NEAR_PLANE {
                    theme::SELECTION.gamma_multiply(theme::FADED)
                } else {
                    theme::SELECTION
                };
                theme::marker(painter, s.pos, c, true);
            }
        }
    }
}

/// Problems drawn at most (the largest first).
const PROBLEMS_SHOWN: usize = 200;
/// Pieces each edge of a problem's box is drawn in.
const EDGE_PIECES: usize = 8;

/// The problems found over the 3D view, through whatever hides them, so
/// that one buried in the model is seen where it is: the boxes of the last
/// check's gaps and overlaps (the largest [`PROBLEMS_SHOWN`]), and a marker
/// at the point of each the slice met.
fn problems(view: &ViewPort, doc: &Document, painter: &egui::Painter) {
    let rect = view.rect();
    let painter = painter.with_clip_rect(rect);
    let colour = |s: llevant::Status| match s {
        llevant::Status::Gap => theme::GAP_MARK,
        _ => theme::PROBLEM_MARK,
    };
    let halo = egui::Stroke::new(3.5, egui::Color32::from_black_alpha(150));
    if let Some(run) = &doc.results.check {
        let v = &run.volumes;
        for &i in v.problems().iter().take(PROBLEMS_SHOWN) {
            let b = v.volumes[i].bbox;
            let corner =
                |k: usize| [0, 1, 2].map(|a| if k >> a & 1 == 1 { b.hi[a] } else { b.lo[a] });
            let line = egui::Stroke::new(1.5, colour(v.labels[i].status));
            // The 12 edges (corners that differ in one coordinate), each in
            // pieces, drawn where both ends of a piece are in front of the
            // eye and near the view: a box close to the eye or around it
            // would project far off (and egui is slow to draw such lines).
            let near = rect.expand(2.0 * rect.width().max(rect.height()));
            let mut pieces = Vec::new();
            for k in 0..8usize {
                for a in 0..3 {
                    if k >> a & 1 == 1 {
                        continue;
                    }
                    let (p, q) = (corner(k), corner(k | 1 << a));
                    let at = |s: f64| {
                        let x = [0, 1, 2].map(|d| p[d] + s * (q[d] - p[d]));
                        view.to_screen(x)
                            .map(|o| o.pos)
                            .filter(|o| near.contains(*o))
                    };
                    for n in 0..EDGE_PIECES {
                        let s0 = n as f64 / EDGE_PIECES as f64;
                        let s1 = (n + 1) as f64 / EDGE_PIECES as f64;
                        if let (Some(a), Some(b)) = (at(s0), at(s1)) {
                            pieces.push([a, b]);
                        }
                    }
                }
            }
            for stroke in [halo, line] {
                for &seg in &pieces {
                    painter.line_segment(seg, stroke);
                }
            }
        }
    }
    for (l, p) in &doc.slice.seen {
        if let Some(s) = view.to_screen(*p) {
            if rect.contains(s.pos) {
                theme::marker(&painter, s.pos, colour(l.status), false);
            }
        }
    }
}

/// The surfaces marked (a cell's, while its geometry is edited): their
/// trace on the plane, each named once.
fn marks(view: &ViewPort, doc: &Document, painter: &egui::Painter) {
    let segs = &doc.marks.segments;
    if segs.is_empty() {
        return;
    }
    let rect = view.rect();
    let colour = theme::MARKED_SURFACE;
    let halo = egui::Stroke::new(4.0, egui::Color32::from_black_alpha(140));
    let line = egui::Stroke::new(2.0, colour);
    // The cell as edited: under the surfaces, wider, in the accent.
    let edited = egui::Stroke::new(4.0, theme::EDITED_CELL);
    let edited_halo = egui::Stroke::new(6.0, egui::Color32::from_black_alpha(140));
    let mut named: Vec<i64> = Vec::new();
    let painter = painter.with_clip_rect(rect);
    for pass in 0..4 {
        for s in segs {
            let is_edited = s.surface == crate::marks::EDITED;
            if is_edited != (pass < 2) {
                continue;
            }
            let (Some(a), Some(b)) = (view.to_screen(s.a), view.to_screen(s.b)) else {
                continue;
            };
            let stroke = match (is_edited, pass % 2) {
                (true, 0) => edited_halo,
                (true, _) => edited,
                (false, 0) => halo,
                (false, _) => line,
            };
            painter.line_segment([a.pos, b.pos], stroke);
            if pass % 2 == 1 && !named.contains(&s.surface) && rect.shrink(20.0).contains(a.pos) {
                named.push(s.surface);
                // (The cell's tag below its edge, the surfaces' above.)
                let (c, text, at) = if is_edited {
                    (
                        theme::EDITED_CELL,
                        "as edited".to_string(),
                        egui::vec2(6.0, 16.0),
                    )
                } else {
                    (colour, s.surface.to_string(), egui::vec2(6.0, -6.0))
                };
                theme::tag(&painter, a.pos + at, c, &text);
            }
        }
    }
}

/// The rectangle the slice shows, in 3D: a faint fill and its edges.
fn slice_outline(view: &ViewPort, doc: &Document, painter: &egui::Painter) {
    let corners = doc.slice.plane().corners();
    let Some(pts) = corners
        .iter()
        .map(|&p| view.to_screen(p).map(|s| s.pos))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    let c = theme::PIVOT;
    painter.add(egui::Shape::convex_polygon(
        pts.clone(),
        c.gamma_multiply(0.08),
        egui::Stroke::new(1.5, c.gamma_multiply(0.8)),
    ));
}

/// The legend of what `doc` colours by, laid out in `area` (points): the
/// field's (with its style), or the densities'; `None` if neither. Drawn on
/// screen by [`colour_bar`] and into saved images (`export.rs`).
pub fn legend(doc: &Document, area: legend::Rect) -> Option<legend::Layout> {
    // Room above the top corners for the 3D view's axes.
    let area = legend::Rect::new(area.x0, area.y0 + TOP_ROOM, area.x1, area.y1);
    if let Some(f) = &doc.field {
        let style = doc.scene.field.as_ref().map(|s| &s.style);
        let l = style.map(|s| s.legend.clone()).unwrap_or_default();
        let title = l.title.clone().unwrap_or_else(|| f.field.title());
        Some(legend::layout(
            &legend::Spec {
                scale: &f.look.scale,
                title: &title,
                style: l.tick_style(),
                vertical: l.vertical,
                length: l.length(),
                thickness: l.thickness(),
                text: l.text.points(),
                corner: l.corner(),
                below: f.look.below,
                above: f.look.above,
            },
            area,
        ))
    } else if doc.scene.colour_by == crate::state::Colouring::Density {
        let l = Legend::default();
        Some(legend::layout(
            &legend::Spec {
                scale: &Scale::DENSITY,
                title: "density g/cm³",
                style: l.tick_style(),
                vertical: false,
                length: l.length(),
                thickness: l.thickness(),
                text: l.text.points(),
                corner: l.corner(),
                below: None,
                above: None,
            },
            area,
        ))
    } else {
        None
    }
}

/// Room left above a legend in a top corner (points).
const TOP_ROOM: f32 = 34.0;

/// Draw a legend laid out by [`legend`](mod@legend): an opaque box with a border, in
/// the theme's colours.
pub fn colour_bar(painter: &egui::Painter, l: &legend::Layout) {
    let v = &painter.ctx().global_style().visuals;
    let ink = v.text_color();
    let border = v.widgets.noninteractive.bg_stroke.color;
    let base = theme::palette(painter.ctx()).base;
    let r =
        |r: legend::Rect| egui::Rect::from_min_max(egui::pos2(r.x0, r.y0), egui::pos2(r.x1, r.y1));
    let rgb = |[r, g, b]: [u8; 3]| egui::Color32::from_rgb(r, g, b);
    painter.rect(
        r(l.back),
        4.0,
        base,
        egui::Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    for &(p, c) in &l.pieces {
        // A hair wider: no seams between pieces.
        painter.rect_filled(r(p).expand2(egui::vec2(0.3, 0.0)), 0.0, rgb(c));
    }
    for &(t, c) in &l.arrows {
        // Edged, so that a colour like the box's still shows.
        painter.add(egui::Shape::convex_polygon(
            t.iter().map(|q| egui::pos2(q[0], q[1])).collect(),
            rgb(c),
            egui::Stroke::new(1.0, ink),
        ));
    }
    for m in &l.marks {
        painter.line_segment(
            [egui::pos2(m[0], m[1]), egui::pos2(m[2], m[3])],
            egui::Stroke::new(1.0, ink),
        );
    }
    for t in &l.texts {
        let h = |a: f32| {
            if a < 0.25 {
                egui::Align::Min
            } else if a > 0.75 {
                egui::Align::Max
            } else {
                egui::Align::Center
            }
        };
        painter.text(
            egui::pos2(t.x, t.y),
            egui::Align2([h(t.ax), h(t.ay)]),
            &t.text,
            egui::FontId::monospace(t.size),
            ink,
        );
    }
}

/// The ruler: a line with a tick where it crosses from one cell to the
/// next, and its ends; faded where it is off the slice's plane.
fn ruler(view: &ViewPort, r: &crate::ruler::Ruler, painter: &egui::Painter) {
    let (Some(a), Some(b)) = (view.to_screen(r.a), view.to_screen(r.b)) else {
        return;
    };
    let off = a.off_plane.max(b.off_plane) > NEAR_PLANE;
    let c = if off {
        egui::Color32::WHITE.gamma_multiply(theme::FADED)
    } else {
        egui::Color32::WHITE
    };
    let halo = egui::Stroke::new(4.0, egui::Color32::from_black_alpha(150));
    painter.line_segment([a.pos, b.pos], halo);
    painter.line_segment([a.pos, b.pos], egui::Stroke::new(1.8, c));
    let d = b.pos - a.pos;
    let n = egui::vec2(-d.y, d.x).normalized() * 6.0;
    if d.length() > 1.0 {
        for s in r.stretches.iter().skip(1) {
            let t = (s.from / r.length().max(1e-12)) as f32;
            let p = a.pos + d * t;
            painter.line_segment([p - n, p + n], halo);
            painter.line_segment([p - n, p + n], egui::Stroke::new(1.5, c));
        }
    }
    for p in [a.pos, b.pos] {
        painter.circle(p, 4.5, c, egui::Stroke::new(2.0, theme::PIVOT));
    }
    painter.text(
        b.pos + egui::vec2(10.0, -10.0),
        egui::Align2::LEFT_BOTTOM,
        format!("{:.2} cm", r.length()),
        egui::FontId::monospace(12.0),
        c,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legend_settings_choose_ticks_and_numbers() {
        use crate::state::NumberFormat;
        let mut l = Legend {
            numbers: NumberFormat::Powers,
            many_ticks: true,
            ..Legend::default()
        };
        let style = l.tick_style();
        let values = legend::tick_values(&Scale::log(1e9, 1e14), style.ticks);
        assert_eq!(values, vec![1e9, 1e10, 1e11, 1e12, 1e13, 1e14]);
        assert_eq!(
            legend::labels(&[1e12, 3.2e-6], &style),
            ["10¹²", "3.2·10⁻⁶"]
        );
        // A count wins over "many".
        l.tick_count = 3;
        assert_eq!(l.tick_style().ticks, legend::Ticks::Count(3));
        l.numbers = NumberFormat::Short;
        let style = l.tick_style();
        assert_eq!(
            legend::labels(&[1.0, 1e3, 1e12], &style),
            ["1e0", "1e3", "1e12"]
        );
    }
}
