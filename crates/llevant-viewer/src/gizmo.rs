//! The axes in a corner of the 3D view, turning with the camera: click the
//! end of an axis to look along it (from +x, -y, above...), keeping what
//! the view is centred on and how far away it is.

use crate::camera::Orbit;

/// Size of the gizmo's square (points).
const SIZE: f32 = 84.0;
/// Radius of an axis end's disc.
const END: f32 = 9.0;

/// The direction the eye sits in from the target for each end: ±x, ±y, ±z,
/// as yaw and pitch (degrees).
const ENDS: [(&str, [f32; 3], f64, f64); 6] = [
    ("X", [1.0, 0.0, 0.0], 0.0, 0.0),
    ("-X", [-1.0, 0.0, 0.0], 180.0, 0.0),
    ("Y", [0.0, 1.0, 0.0], 90.0, 0.0),
    ("-Y", [0.0, -1.0, 0.0], 270.0, 0.0),
    ("Z", [0.0, 0.0, 1.0], 270.0, 89.0),
    ("-Z", [0.0, 0.0, -1.0], 270.0, -89.0),
];

const COLOURS: [egui::Color32; 3] = [
    egui::Color32::from_rgb(230, 80, 80),
    egui::Color32::from_rgb(110, 200, 90),
    egui::Color32::from_rgb(90, 150, 255),
];

/// Draw the gizmo in the top right of `view` and snap `orbit` when an end
/// is clicked. Whether the pointer is over it (so that the view leaves the
/// click alone).
pub fn ui(ui: &mut egui::Ui, view: egui::Rect, orbit: &mut Orbit) -> bool {
    let rect = egui::Rect::from_min_size(
        egui::pos2(view.max.x - SIZE - 8.0, view.min.y + 8.0),
        egui::vec2(SIZE, SIZE),
    );
    if !view.contains_rect(rect) {
        return false;
    }
    let centre = rect.center();
    let reach = 0.5 * SIZE - END - 2.0;
    let cam = orbit.camera(64, 64, None);
    let [_, r, u] = cam.basis();
    let f = orbit.forward();
    let painter = ui.painter_at(rect);
    let bg = ui.visuals().extreme_bg_color.gamma_multiply(0.7);
    painter.circle_filled(centre, 0.5 * SIZE, bg);
    // Back to front, so that the ends nearer the eye cover the others.
    let mut ends: Vec<(usize, egui::Pos2, f32)> = ENDS
        .iter()
        .enumerate()
        .map(|(k, (_, d, ..))| {
            let d = d.map(|x| x as f64);
            let x = (d[0] * r[0] + d[1] * r[1] + d[2] * r[2]) as f32;
            let y = (d[0] * u[0] + d[1] * u[1] + d[2] * u[2]) as f32;
            let depth = (d[0] * f[0] + d[1] * f[1] + d[2] * f[2]) as f32;
            (k, centre + egui::vec2(x, -y) * reach, depth)
        })
        .collect();
    ends.sort_by(|a, b| b.2.total_cmp(&a.2));
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let mut over = pointer.is_some_and(|p| (p - centre).length() < 0.5 * SIZE);
    let mut clicked = None;
    for &(k, pos, _) in &ends {
        let (name, ..) = ENDS[k];
        let colour = COLOURS[k / 2];
        let positive = k % 2 == 0;
        if positive {
            painter.line_segment([centre, pos], egui::Stroke::new(2.0, colour));
        }
        let id = ui.id().with(("gizmo", k));
        let hit = egui::Rect::from_center_size(pos, egui::vec2(2.0 * END, 2.0 * END));
        let resp = ui.interact(hit, id, egui::Sense::click());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("View from {name}"))
        });
        let resp = resp.on_hover_text(format!("View from {name}"));
        let hot = resp.hovered();
        over |= hot;
        if positive {
            painter.circle_filled(
                pos,
                END,
                if hot {
                    colour.gamma_multiply(1.3)
                } else {
                    colour
                },
            );
            painter.text(
                pos,
                egui::Align2::CENTER_CENTER,
                name,
                egui::FontId::proportional(11.0),
                egui::Color32::BLACK,
            );
        } else {
            let fill = if hot { colour.gamma_multiply(0.6) } else { bg };
            painter.circle(pos, END - 1.0, fill, egui::Stroke::new(1.5, colour));
        }
        if resp.clicked() {
            clicked = Some(k);
        }
    }
    if let Some(k) = clicked {
        let (_, _, yaw, pitch) = ENDS[k];
        orbit.state.yaw_deg = yaw;
        orbit.state.pitch_deg = pitch;
    }
    over
}
