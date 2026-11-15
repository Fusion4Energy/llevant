//! The keys and mouse actions, in a window (F1, or the keyboard button at the
//! right of the menu bar).

/// What each key and mouse action does, by group.
pub const KEYS: &[(&str, &[(&str, &str)])] = &[
    (
        "3D view",
        &[
            ("middle-drag", "orbit about the point under the cursor"),
            ("Shift+middle-drag, right-drag", "slide"),
            ("Alt+middle-drag", "look around from where the eye is"),
            ("scroll", "towards the point under the cursor"),
            ("Alt+← / Alt+→", "back / forward to the places looked at"),
            ("W A S D, Q E", "fly (Shift faster, Ctrl slower)"),
            ("axes, top right", "view from ±x, ±y, ±z"),
            ("O", "orthographic or perspective"),
        ],
    ),
    (
        "Slice",
        &[
            ("middle-drag, right-drag", "pan"),
            ("scroll", "zoom about the cursor"),
            (
                "PageUp / PageDown",
                "the next plane, or a degree about a radial axis (Shift 10×)",
            ),
        ],
    ),
    (
        "FMESH (Tally)",
        &[
            (
                "+ Box, + Cylinder, then drag in the slice",
                "a new mesh: a box, or a cylinder from its centre",
            ),
            ("click", "select the mesh there"),
            (
                "drag a handle",
                "move a bound, the whole mesh, or (3D rings) turn it",
            ),
            ("Shift", "while dragging: no rounding"),
            ("Delete", "remove the selected mesh"),
        ],
    ),
    (
        "Section: cuts and the slice plane, in 3D",
        &[
            ("drag a dot", "move the cut (or the slice plane) there"),
            ("click an outline or a dot", "select it: its rings turn it"),
            ("Shift or Alt", "while turning: freely, not in 15° steps"),
            ("click elsewhere, Esc", "done"),
        ],
    ),
    (
        "Ruler (M)",
        &[
            ("click, click / drag", "two points: the cells between them"),
            (
                "click a face",
                "through the layers: along its normal, up to the next void",
            ),
        ],
    ),
    (
        "Selection",
        &[
            ("click", "select the cell there"),
            (
                "drag",
                "select the cells seen in the box (Ctrl adds, Shift removes)",
            ),
            ("Ctrl+click", "add it, or remove it if selected"),
            ("Shift+click", "remove it"),
            ("right-click", "what can be done with what is there"),
            ("Ctrl+↑", "select the cell holding it"),
            ("Ctrl+I", "invert the selection"),
            ("Shift+Alt+← / →", "previous / next selection"),
            (
                "Esc",
                "clear the selection, back to Select, done with a cut",
            ),
            ("F", "frame the selection"),
        ],
    ),
    (
        "Showing and hiding",
        &[
            ("H", "hide the selection"),
            ("Shift+H", "isolate the selection (hide all else)"),
            ("Alt+H", "show all again"),
        ],
    ),
    (
        "Everywhere",
        &[
            ("Ctrl+Z", "undo"),
            ("Ctrl+Shift+Z, Ctrl+Y", "redo"),
            ("Ctrl+K", "search: commands, rules, points"),
            ("Ctrl+1 … Ctrl+4", "Explore, Check, Results, Tally"),
            ("Ctrl+[, Ctrl+]", "fold the left side, the inspector"),
            ("F1", "this window"),
        ],
    ),
];

pub fn window(ctx: &egui::Context, open: &mut bool) {
    // No taller than the window (less the title bar and margins): the rest
    // scrolls.
    let max_h = (ctx.content_rect().height() - 80.0).max(120.0);
    egui::Window::new(crate::icons::with(crate::icons::KEYS, "Keys and mouse"))
        .open(open)
        .resizable(false)
        .collapsible(false)
        .constrain(true)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(max_h)
                .auto_shrink([true, true])
                .show(ui, |ui| {
                    for (group, rows) in KEYS {
                        ui.label(egui::RichText::new(*group).strong());
                        egui::Grid::new(group)
                            .num_columns(2)
                            .spacing([16.0, 2.0])
                            .show(ui, |ui| {
                                for (keys, what) in *rows {
                                    ui.label(crate::theme::data(*keys));
                                    ui.label(*what);
                                    ui.end_row();
                                }
                            });
                        ui.add_space(6.0);
                    }
                });
        });
}
