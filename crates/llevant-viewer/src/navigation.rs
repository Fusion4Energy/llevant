//! Moving the 3D camera, whatever the tool, CAD style (decision 0012):
//! middle-drag to orbit about the point under the cursor where the drag
//! starts (over the background, about the centre of the view, which
//! scrolling moves), Alt+middle-drag to look around from where the eye is,
//! Shift+middle-drag or right-drag to slide, scroll towards the point under
//! the cursor, W/A/S/D/Q/E to fly. The left button belongs to the tools
//! (Select draws a box). The slice view pans with the middle and right
//! buttons and zooms itself (`SliceView::ui`).

use crate::document::Document;
use crate::viewport::ViewPort;

/// Apply this frame's pointer and keys over the 3D `view` to the camera.
pub fn navigate_3d(ui: &egui::Ui, view: &ViewPort, doc: &mut Document) {
    let response = &view.response;
    let rect = view.rect();
    let mods = ui.input(|i| i.modifiers);
    let middle = response.dragged_by(egui::PointerButton::Middle);
    if response.drag_started_by(egui::PointerButton::Middle) {
        // What is under the cursor where the drag starts is what it turns
        // about.
        let origin = ui.input(|i| i.pointer.press_origin());
        doc.pointer.drag_pivot = origin
            .and_then(|pos| view.pick(&doc.model, &doc.vis, pos))
            .map(|p| p.point);
    }
    if !middle || mods.shift {
        doc.pointer.drag_pivot = None;
    }
    if middle && !mods.shift {
        let dl = response.drag_delta();
        if mods.alt {
            doc.orbit.look(dl.x as f64, dl.y as f64);
        } else {
            let pivot = doc.pointer.drag_pivot.unwrap_or(doc.orbit.state.target);
            doc.orbit.orbit_about(pivot, dl.x as f64, dl.y as f64);
        }
    }
    if (middle && mods.shift) || response.dragged_by(egui::PointerButton::Secondary) {
        let dl = response.drag_delta();
        doc.orbit
            .pan(dl.x as f64, dl.y as f64, rect.height() as f64);
    }
    if !response.hovered() {
        return;
    }
    // Scroll: towards the point under the cursor (or along its ray, as far
    // as the target, over the background).
    let scroll = ui.input(|i| i.smooth_scroll_delta.y) as f64;
    if let (true, Some(pos)) = (scroll != 0.0, response.hover_pos()) {
        let p = match view.pick(&doc.model, &doc.vis, pos) {
            Some(p) => p.point,
            None => {
                let (i, j) = view.pixel(pos).expect("3D view");
                let crate::viewport::Projection::Camera(cam) = &view.projection else {
                    unreachable!("3D view")
                };
                cam.point(i, j, doc.orbit.state.distance)
            }
        };
        doc.orbit.towards(p, scroll);
    }
    // Fly with the keys while the pointer is over the view (and no text
    // field has the keyboard): a quarter of the model's size per second,
    // Shift 4× faster, Ctrl 4× slower.
    if !ui.ctx().egui_wants_keyboard_input() {
        use egui::Key;
        let (axes, dt, mods) = ui.input(|i| {
            let k = |a: Key, b: Key| i.key_down(a) as i32 as f64 - i.key_down(b) as i32 as f64;
            (
                [k(Key::D, Key::A), k(Key::E, Key::Q), k(Key::W, Key::S)],
                i.stable_dt.min(0.1) as f64,
                i.modifiers,
            )
        });
        if axes != [0.0; 3] {
            let mut speed = 0.25 * doc.orbit.radius;
            if mods.shift {
                speed *= 4.0;
            }
            if mods.ctrl {
                speed *= 0.25;
            }
            let s = speed * dt;
            doc.orbit.fly(s * axes[0], s * axes[1], s * axes[2]);
            ui.ctx().request_repaint();
        }
    }
}
