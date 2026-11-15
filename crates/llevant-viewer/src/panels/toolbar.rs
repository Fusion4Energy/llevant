//! The bars: the tool row over the views (Select and the Ruler, then what
//! applies to both views: colouring, the level shown, void cells and gases;
//! on the right, the universe opened and the way back), and the controls at
//! the right of the menu bar (search, undo and redo, the theme, the keys
//! window). Cuts and meshes need no tool: their handles work whatever the
//! tool ([`crate::tools`]).

use egui::ThemePreference;

use crate::command::{Command, Edit};
use crate::document::Document;
use crate::icons;
use crate::state::{colouring_name, Colouring};
use crate::tools::Toolbox;

/// The label of a tool's button.
pub fn tool_label(t: &dyn crate::tools::Tool) -> String {
    icons::with(t.icon(), t.name())
}

/// The row over the views.
pub fn tool_row(
    ui: &mut egui::Ui,
    tools: &mut Toolbox,
    d: &Document,
    open: &[i64],
    out: &mut Vec<Command>,
) {
    // Wrapped: a narrow window puts the rest on a second row.
    ui.horizontal_wrapped(|ui| {
        for k in 0..tools.tools.len() {
            let t = tools.tools[k].as_ref();
            let r = ui
                .selectable_label(tools.active == k, tool_label(t))
                .on_hover_text(t.hint());
            if r.clicked() {
                tools.active = k;
            }
        }
        ui.separator();
        // How the cells are coloured, in both views.
        let now = d.scene.colour_by;
        ui.menu_button(
            icons::with(icons::COLOUR, &format!("Colour · {}", colouring_name(now))),
            |ui| {
                for c in [
                    Colouring::Cell,
                    Colouring::Material,
                    Colouring::Universe,
                    Colouring::Realistic,
                    Colouring::Density,
                ] {
                    if ui.selectable_label(c == now, colouring_name(c)).clicked() {
                        out.push(Edit::Colouring(c).into());
                        ui.close();
                    }
                }
            },
        )
        .response
        .on_hover_text("How the cells are coloured, in both views");
        // The universe level shown, as MCNP's plotter `LEVEL n`.
        if d.levels > 1 {
            let now = d.scene.hiding.level;
            let name = |l: Option<usize>| match l {
                Some(k) => format!("Level {k}"),
                None => "All levels".into(),
            };
            ui.menu_button(icons::with(icons::LEVELS, &name(now)), |ui| {
                let mut choice = |ui: &mut egui::Ui, l: Option<usize>, hint: &str| {
                    if ui
                        .selectable_label(l == now, name(l))
                        .on_hover_text(hint)
                        .clicked()
                    {
                        out.push(Edit::Level(l).into());
                        ui.close();
                    }
                };
                choice(ui, None, "The deepest cell of every path");
                for k in 0..d.levels {
                    let hint = match k {
                        0 => "Only level 0: the envelopes as cells, as MCNP's LEVEL 0".to_string(),
                        _ => format!("Only cells at universe level {k}, as MCNP's LEVEL {k}"),
                    };
                    choice(ui, Some(k), &hint);
                }
            })
            .response
            .on_hover_text("Universe level shown in both views (MCNP's LEVEL)");
        }
        // Void cells, shown or hidden in one click.
        let hidden = d.scene.hiding.void;
        if ui
            .selectable_label(!hidden, icons::with(icons::VOID, "Void"))
            .on_hover_text(if hidden {
                "Void cells are hidden in 3D: click to show them"
            } else {
                "Void cells are shown in 3D: click to hide them"
            })
            .clicked()
        {
            out.push(Edit::HideVoid(!hidden).into());
        }
        // Gases (air, helium…): every material whose look is a gas, shown
        // or hidden in one click.
        let gas: Vec<i64> = d
            .looks
            .materials
            .iter()
            .filter(|l| l.mix.main() == llevant_scene::materials::Category::Gas)
            .map(|l| l.id)
            .collect();
        if !gas.is_empty() {
            let hidden = &d.scene.hiding.materials;
            let all_hidden = gas.iter().all(|g| hidden.contains(g));
            let names: Vec<String> = gas.iter().map(|g| format!("m{g}")).collect();
            if ui
                .selectable_label(!all_hidden, icons::with(icons::AIR, "Air"))
                .on_hover_text(format!(
                    "{} ({}): click to {} them",
                    if all_hidden {
                        "Gases are hidden"
                    } else {
                        "Gases are shown"
                    },
                    names.join(", "),
                    if all_hidden { "show" } else { "hide" }
                ))
                .clicked()
            {
                let mut list = hidden.clone();
                if all_hidden {
                    list.retain(|m| !gas.contains(m));
                } else {
                    list.extend(gas.iter().filter(|g| !hidden.contains(g)));
                    list.sort_unstable();
                }
                out.push(Edit::Hidden(crate::command::Hidden::Materials, list).into());
            }
        }
        // Universes opened on their own: where the view is, and the way back.
        if !open.is_empty() {
            ui.separator();
            let path: Vec<String> = open.iter().map(|u| format!("u={u}")).collect();
            ui.label(format!("model › {}", path.join(" › ")));
            if ui
                .button("⬅ Back")
                .on_hover_text("Back to the model this universe was opened from")
                .clicked()
            {
                out.push(Command::Back);
            }
        }
    });
}

/// At the right of the menu bar: search, undo and redo, the theme and the
/// keys window.
pub fn bar_right(
    ui: &mut egui::Ui,
    doc: Option<&Document>,
    theme: &mut ThemePreference,
    show_keys: &mut bool,
    out: &mut Vec<Command>,
) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.toggle_value(show_keys, icons::KEYS)
            .on_hover_text("Keys and mouse (F1)");
        // One button, turning dark → light → as the system → dark.
        let (icon, name) = theme_look(*theme);
        let next = match *theme {
            ThemePreference::Dark => ThemePreference::Light,
            ThemePreference::Light => ThemePreference::System,
            ThemePreference::System => ThemePreference::Dark,
        };
        if ui
            .button(icon)
            .on_hover_text(format!(
                "Theme: {name} (click: {})",
                theme_look(next).1.to_lowercase()
            ))
            .clicked()
        {
            *theme = next;
            ui.ctx().set_theme(next);
        }
        ui.separator();
        let redo = doc.and_then(|d| d.history.next_redo());
        let r = ui
            .add_enabled(
                redo.is_some(),
                egui::Button::new(icons::with(icons::REDO, "Redo")),
            )
            .on_hover_text(redo.map_or("Ctrl+Shift+Z".into(), |w| {
                format!("Redo {w} (Ctrl+Shift+Z)")
            }));
        if r.clicked() {
            out.push(Command::Redo);
        }
        let undo = doc.and_then(|d| d.history.next_undo());
        let r = ui
            .add_enabled(
                undo.is_some(),
                egui::Button::new(icons::with(icons::UNDO, "Undo")),
            )
            .on_hover_text(undo.map_or("Ctrl+Z".into(), |w| format!("Undo {w} (Ctrl+Z)")));
        if r.clicked() {
            out.push(Command::Undo);
        }
        ui.separator();
        // The palette, behind what looks like a search field.
        let field = egui::Button::new(
            egui::RichText::new(icons::with(
                icons::SEARCH,
                "Go to a cell, a rule, a command…",
            ))
            .weak(),
        )
        .shortcut_text("Ctrl+K")
        .min_size(egui::vec2(260.0, 0.0));
        let r = ui.add(field).on_hover_text(
            "Find a command, select by rule (u 20040 & m 5), go to a point or a slice",
        );
        crate::app::frame::named(&r, "Search");
        if r.clicked() {
            out.push(Command::Palette);
        }
    });
}

/// The icon and name of a theme.
fn theme_look(t: ThemePreference) -> (&'static str, &'static str) {
    match t {
        ThemePreference::Dark => (icons::DARK, "Dark"),
        ThemePreference::Light => (icons::LIGHT, "Light"),
        ThemePreference::System => (icons::SYSTEM, "As the system"),
    }
}

/// Keyboard shortcuts of the bars: undo, redo, Esc for the first tool (and
/// no cut selected, no mesh being drawn).
/// Not while a text field has the keyboard (it has its own undo).
pub fn shortcuts(ctx: &egui::Context, tools: &mut Toolbox, out: &mut Vec<Command>) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    use egui::{Key, KeyboardShortcut, Modifiers};
    let redo = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    let redo_y = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
    let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
    ctx.input_mut(|i| {
        // The longer shortcut first: Ctrl+Shift+Z also matches Ctrl+Z.
        if i.consume_shortcut(&redo) || i.consume_shortcut(&redo_y) {
            out.push(Command::Redo);
        } else if i.consume_shortcut(&undo) {
            out.push(Command::Undo);
        }
        if i.key_pressed(Key::Escape) {
            tools.active = 0;
            out.push(Command::SelectCut(None));
            out.push(Command::DrawMesh(None));
        }
        if i.consume_key(Modifiers::NONE, Key::M) {
            tools.activate("Ruler");
        }
    });
}
