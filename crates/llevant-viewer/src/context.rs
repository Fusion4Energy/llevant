//! Context menus and links: the same actions wherever a cell, universe or
//! material shows up, in the views (right-click), the inspector and the
//! lists. A link selects what it names on a click (Ctrl adds or removes,
//! Shift removes), frames it on a double-click, and opens the menu of what
//! it names on a right-click.

use llevant::Model;
use llevant_scene::index::CellSet;
use llevant_scene::query::Query;

use crate::app::frame::Panel;
use crate::command::{Command, Edit};
use crate::document::Document;
use crate::icons;
use crate::mesh::{InputMeshes, MeshState};
use crate::pick::Picked;
use crate::scene::Pin;
use crate::selection::{selection_box, SelectMode};

/// Characters of a rule a link's tooltip and menu show at most: a
/// selection of thousands of cells would fill the screen (Copy rule copies
/// all of it).
const RULE_SHOWN: usize = 100;

/// `text` cut to at most `max` characters, at a space when there is one,
/// with "…"; and whether it was cut.
pub fn elide(text: &str, max: usize) -> (String, bool) {
    if text.chars().count() <= max {
        return (text.to_string(), false);
    }
    let head: String = text.chars().take(max).collect();
    let head = match head.rfind(' ') {
        Some(k) if k > 0 => &head[..k],
        _ => &head[..],
    };
    (format!("{head} …"), true)
}

/// A clickable name for `q` (`text`), with its menu.
pub fn link(
    ui: &mut egui::Ui,
    doc: &Document,
    q: &Query,
    text: impl Into<egui::WidgetText>,
    out: &mut Vec<Command>,
) -> egui::Response {
    let r = ui
        .add(egui::Button::new(text).frame(false).small())
        .on_hover_ui(|ui| {
            let (rule, _) = elide(&q.to_string(), RULE_SHOWN);
            ui.label(format!(
                "{rule}: click selects, Ctrl+click adds or removes, double-click frames, \
                 right-click for more"
            ));
        });
    if r.double_clicked() {
        out.push(Command::Select(q.clone(), SelectMode::Replace));
        out.push(Command::FrameSelection);
    } else if r.clicked() {
        let mode = SelectMode::from_modifiers(ui.input(|i| i.modifiers));
        out.push(Command::Select(q.clone(), mode));
    }
    r.context_menu(|ui| query_menu(ui, doc, q, out));
    r
}

/// The link of a cell (index into the model's cells).
pub fn cell_link(ui: &mut egui::Ui, doc: &Document, cell: u32, out: &mut Vec<Command>) {
    let id = doc.model.cell_id(cell);
    link(ui, doc, &Query::cell(id), format!("cell {id}"), out);
}

/// The link of a compiled surface: named with its facet, selecting the
/// cells of every facet of its MCNP number.
pub fn surface_link(ui: &mut egui::Ui, doc: &Document, shape: u32, out: &mut Vec<Command>) {
    let Some(id) = doc.model.surface_id(shape) else {
        return;
    };
    let text = format!("s {}", crate::pick::surface_name(id));
    link(ui, doc, &Query::surface(id.id), text, out);
}

pub fn universe_link(ui: &mut egui::Ui, doc: &Document, u: i64, out: &mut Vec<Command>) {
    link(ui, doc, &Query::universe(u), format!("u={u}"), out);
}

pub fn material_link(ui: &mut egui::Ui, doc: &Document, m: i64, out: &mut Vec<Command>) {
    let text = if m == 0 {
        "void".to_string()
    } else {
        format!("m{m}")
    };
    let q = if m == 0 {
        Query::Void
    } else {
        Query::material(m)
    };
    link(ui, doc, &q, text, out);
}

/// What can be done with the cells of `q`.
pub fn query_menu(ui: &mut egui::Ui, doc: &Document, q: &Query, out: &mut Vec<Command>) {
    let text = q.to_string();
    let (rule, cut) = elide(&text, RULE_SHOWN);
    let r = ui.label(egui::RichText::new(rule).strong());
    if cut {
        r.on_hover_text(format!(
            "The rule has {} characters: Copy rule copies all of it",
            text.chars().count()
        ));
    }
    ui.separator();
    let mut item = |ui: &mut egui::Ui, icon: &str, text: &str, keys: &str, c: Vec<Command>| {
        let b = egui::Button::new(icons::with(icon, text)).shortcut_text(keys);
        if ui.add(b).clicked() {
            out.extend(c);
            ui.close();
        }
    };
    item(
        ui,
        icons::SELECT,
        "Select",
        "",
        vec![Command::Select(q.clone(), SelectMode::Replace)],
    );
    if !doc.selection.is_empty() {
        item(
            ui,
            icons::SELECT,
            "Add to selection",
            "Ctrl+click",
            vec![Command::Select(q.clone(), SelectMode::Add)],
        );
        item(
            ui,
            icons::REMOVE,
            "Remove from selection",
            "Shift+click",
            vec![Command::Select(q.clone(), SelectMode::Remove)],
        );
    }
    item(
        ui,
        icons::FRAME,
        "Frame",
        "double-click",
        vec![
            Command::Select(q.clone(), SelectMode::Replace),
            Command::FrameSelection,
        ],
    );
    ui.separator();
    item(
        ui,
        icons::HIDE,
        "Hide",
        "",
        vec![Edit::Hide(q.clone()).into()],
    );
    item(
        ui,
        icons::ISOLATE,
        "Isolate (hide others)",
        "",
        vec![Edit::Isolate(Some(q.clone())).into()],
    );
    if hidden_by(doc, q) {
        item(
            ui,
            icons::SHOW,
            "Show again",
            "",
            vec![Edit::Unhide(q.clone()).into()],
        );
    }
    if ghosted_by(doc, q) {
        item(
            ui,
            icons::GHOST,
            "No longer a ghost",
            "",
            vec![Edit::Unghost(q.clone()).into()],
        );
    } else {
        item(
            ui,
            icons::GHOST,
            "Ghost (see through, contents shown)",
            "",
            vec![Edit::Ghost(q.clone()).into()],
        );
    }
    if let Query::Universes(us) = q {
        if let [u] = us.as_slice() {
            ui.separator();
            item(
                ui,
                icons::OPEN,
                &format!("Open u={u} on its own"),
                "",
                vec![Command::OpenUniverse(*u)],
            );
            item(
                ui,
                icons::SELECT,
                &format!("Select the cells that u={u} fills"),
                "",
                vec![Command::Select(
                    Query::Holders(vec![*u]),
                    SelectMode::Replace,
                )],
            );
        }
    }
    ui.separator();
    if ui.button(icons::with(icons::COPY, "Copy rule")).clicked() {
        ui.ctx().copy_text(q.to_string());
        ui.close();
    }
}

/// Whether hiding rules name `q` exactly (so that "Show again" undoes them).
/// Whether `q` is a ghost rule, or its cells are all among ghost cells.
fn ghosted_by(doc: &Document, q: &Query) -> bool {
    let g = &doc.scene.hiding.ghost;
    g.contains(q)
        || matches!(q, Query::Cells(ids) if ids.iter().all(|c| g.iter().any(
            |r| matches!(r, Query::Cells(cs) if cs.contains(c))
        )))
}

fn hidden_by(doc: &Document, q: &Query) -> bool {
    let h = &doc.scene.hiding;
    h.rules.contains(q)
        || h.isolate.as_ref() == Some(q)
        || matches!(q, Query::Cells(ids) if ids.iter().all(|c| h.cells.contains(c)))
}

/// The menu of a right-click in a view: what is at the point (`target`),
/// then the selection, then the whole scene.
pub fn view_menu(
    ui: &mut egui::Ui,
    doc: &Document,
    target: Option<&Picked>,
    out: &mut Vec<Command>,
) {
    ui.set_min_width(240.0);
    let m = &doc.model;
    if let Some(t) = target {
        if let Some(c) = t.cell() {
            let cell = m.cell(c);
            let u = m.units()[cell.unit as usize].universe;
            ui.label(egui::RichText::new(header(m, t)).strong());
            ui.separator();
            let id = cell.id;
            submenu(
                ui,
                doc,
                icons::CELL,
                &format!("Cell {id}"),
                &Query::cell(id),
                out,
            );
            submenu(
                ui,
                doc,
                icons::UNIVERSE,
                &format!("Universe {u}"),
                &Query::universe(u),
                out,
            );
            surface_items(ui, doc, t, out);
            if cell.fill.is_none() {
                let (name, q) = if cell.material == 0 {
                    ("Void cells".to_string(), Query::Void)
                } else {
                    (
                        format!("Material {}", cell.material),
                        Query::material(cell.material),
                    )
                };
                submenu(ui, doc, icons::MATERIAL, &name, &q, out);
            }
            // The cells holding it, level by level up.
            let path = &t.label.path;
            for s in path.iter().rev().skip(1) {
                let e = m.cell(s.cell);
                let eu = m.units()[e.unit as usize].universe;
                let title = format!("Envelope: cell {} (u={eu})", e.id);
                submenu(ui, doc, icons::VIEW_3D, &title, &Query::cell(e.id), out);
            }
            let mut opened = Vec::new();
            for s in path.iter().skip(1) {
                let su = m.units()[m.cell(s.cell).unit as usize].universe;
                if !opened.contains(&su) {
                    opened.push(su);
                }
            }
            for su in opened {
                if ui
                    .button(icons::with(icons::OPEN, &format!("Open u={su} on its own")))
                    .clicked()
                {
                    out.push(Command::OpenUniverse(su));
                    ui.close();
                }
            }
        } else {
            ui.label(egui::RichText::new(t.label.status.name()).strong());
        }
        ui.separator();
        if ui.button(icons::with(icons::SLICE, "Slice here")).clicked() {
            out.push(Command::SliceThrough(t.point));
            ui.close();
        }
        if ui
            .button(icons::with(icons::SLICE, "Radial slice through here"))
            .on_hover_text("The plane through the z axis and this point")
            .clicked()
        {
            let a = crate::panels::view_settings::default_axis_point(&doc.region);
            let phi = (t.point[1] - a[1]).atan2(t.point[0] - a[0]).to_degrees();
            out.push(Command::SliceRadial(phi));
            out.push(Command::SliceThrough(t.point));
            ui.close();
        }
        ui.menu_button(icons::with(icons::CLIP, "Cut here"), |ui| {
            if ui.button("Plane through this point, facing you").clicked() {
                out.push(Edit::AddCut(crate::cuts::plane_facing(doc, t.point)).into());
                ui.close();
            }
            if ui
                .button("Quarter wedge about z, open towards you")
                .clicked()
            {
                let a = crate::panels::view_settings::default_axis_point(&doc.region);
                out.push(Edit::AddCut(crate::cuts::wedge_facing(doc, a)).into());
                ui.close();
            }
            if ui.button("Section box around the selection").clicked() {
                out.push(Edit::AddCut(crate::cuts::section_box(doc)).into());
                ui.close();
            }
        });
        ui.menu_button(icons::with(icons::MESH, "FMESH"), |ui| {
            let sel = selection_box(&doc.model, &doc.index, doc.selected(), &doc.region);
            if ui
                .add_enabled(sel.is_some(), egui::Button::new("Box around the selection"))
                .clicked()
            {
                if let Some(b) = sel {
                    out.extend(mesh_around(doc, &b));
                }
                ui.close();
            }
            if let Some(c) = t.cell() {
                let one = CellSet::from_cells(doc.model.cells().len(), [c]);
                if ui
                    .button(format!("Box around cell {}", m.cell_id(c)))
                    .on_hover_text("Around all of its instances (a filler's: its envelopes)")
                    .clicked()
                {
                    if let Some(b) = selection_box(&doc.model, &doc.index, &one, &doc.region) {
                        out.extend(mesh_around(doc, &b));
                    }
                    ui.close();
                }
            }
            if ui.button("Draw a box in the slice…").clicked() {
                out.push(Command::ShowPanel(Panel::Mesh));
                out.push(Command::DrawMesh(Some(false)));
                ui.close();
            }
        });
        if ui.button(icons::with(icons::PIN, "Pin here")).clicked() {
            out.push(
                Edit::AddPin(Pin {
                    point: t.point,
                    note: String::new(),
                })
                .into(),
            );
            // (Shown in the inspector, where it can be noted.)
            out.push(Command::Focus(Some(t.clone())));
            ui.close();
        }
        if ui.button(icons::with(icons::COPY, "Copy point")).clicked() {
            let p = t.point;
            ui.ctx().copy_text(format!("{} {} {}", p[0], p[1], p[2]));
            ui.close();
        }
        // MCNP's repeated-structure notation (`12 < 100[1 0 0] < 5`): what
        // tally cards take, and a rule Find takes back.
        if !t.label.path.is_empty()
            && ui
                .button(icons::with(icons::COPY, "Copy cell path"))
                .on_hover_text(
                    "As MCNP writes it (12 < 100[1 0 0] < 5): for tally cards, and a rule \
                     Find selects it by",
                )
                .clicked()
        {
            ui.ctx().copy_text(m.format_path(&t.label.path));
            ui.close();
        }
        ui.separator();
    }
    selection_items(ui, doc, out);
}

/// A new FMESH box around `b` (rounded out), selected, in Tally.
fn mesh_around(doc: &Document, b: &llevant::Aabb) -> Vec<Command> {
    let id = InputMeshes::free_tally(doc.input_meshes.as_deref(), &doc.scene.meshes);
    let m = MeshState::rect(id, b.lo, b.hi).fitted(b);
    vec![
        Edit::AddMesh(Box::new(m)).into(),
        Command::ShowPanel(Panel::Mesh),
    ]
}

/// One line naming what is at a point.
/// The surface at the point right-clicked and those coincident with it:
/// each opens in the inspector (as an Alt+click does), with what can be
/// done with the cells using it.
fn surface_items(ui: &mut egui::Ui, doc: &Document, t: &Picked, out: &mut Vec<Command>) {
    let m = &doc.model;
    let Some(s) = &t.surface else {
        return;
    };
    let mut shapes = vec![s.shape];
    shapes.extend(t.through.iter().copied().filter(|&x| x != s.shape));
    for (k, shape) in shapes.into_iter().enumerate() {
        let Some(id) = m.surface_id(shape) else {
            continue;
        };
        let kind = doc
            .source
            .as_ref()
            .and_then(|src| src.surface_coeffs(id.id))
            .map(|(k, _)| format!(" ({})", k.to_uppercase()))
            .unwrap_or_default();
        let name = crate::pick::surface_name(id);
        let title = match k {
            0 => format!("Surface {name}{kind}"),
            _ => format!("Coincident: surface {name}{kind}"),
        };
        ui.menu_button(icons::with(icons::TARGET, &title), |ui| {
            let b =
                egui::Button::new(icons::with(icons::OPEN, "Inspect")).shortcut_text("Alt+click");
            if ui
                .add(b)
                .on_hover_text("Open it in the inspector and mark it in both views")
                .clicked()
            {
                out.push(Command::PickSurface(Some(t.with_surface(shape))));
                ui.close();
            }
            ui.separator();
            query_menu(ui, doc, &Query::surface(id.id), out);
        });
    }
}

fn header(m: &Model, t: &Picked) -> String {
    t.summary(m).join(" · ")
}

fn submenu(
    ui: &mut egui::Ui,
    doc: &Document,
    icon: &str,
    title: &str,
    q: &Query,
    out: &mut Vec<Command>,
) {
    ui.menu_button(icons::with(icon, title), |ui| query_menu(ui, doc, q, out));
}

/// Actions on the selection and on what is hidden, with their keys.
pub fn selection_items(ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
    let sel = doc.selection.query.clone();
    // Enabled when there is a command.
    let mut item = |ui: &mut egui::Ui, icon: &str, text: &str, keys: &str, c: Option<Command>| {
        let b = egui::Button::new(icons::with(icon, text)).shortcut_text(keys);
        if ui.add_enabled(c.is_some(), b).clicked() {
            out.extend(c);
            ui.close();
        }
    };
    let some = sel.is_some();
    let hiding = doc.scene.hiding.hides_more_than_void();
    let hide = sel.clone().map(|q| Edit::Hide(q).into());
    let ghost = sel.clone().map(|q| Edit::Ghost(q).into());
    let iso = sel.map(|q| Edit::Isolate(Some(q)).into());
    item(ui, icons::HIDE, "Hide selection", "H", hide);
    item(ui, icons::ISOLATE, "Isolate selection", "Shift+H", iso);
    item(ui, icons::GHOST, "Ghost selection", "G", ghost);
    let ghosts = !doc.scene.hiding.ghost.is_empty();
    item(
        ui,
        icons::GHOST,
        "No ghosts",
        "Alt+G",
        ghosts.then(|| Edit::NoGhosts.into()),
    );
    item(
        ui,
        icons::SHOW,
        "Show all",
        "Alt+H",
        hiding.then(|| Edit::ShowAll.into()),
    );
    item(
        ui,
        icons::FRAME,
        "Frame selection",
        "F",
        some.then_some(Command::FrameSelection),
    );
    item(
        ui,
        icons::INVERT,
        "Invert selection",
        "Ctrl+I",
        Some(Command::InvertSelection),
    );
    let parent = doc.focus.is_some().then_some(Command::SelectParent);
    item(ui, icons::PARENT, "Select parent", "Ctrl+↑", parent);
    item(
        ui,
        icons::REMOVE,
        "Clear selection",
        "Esc",
        some.then_some(Command::ClearSelection),
    );
}

/// Keyboard shortcuts on the selection and visibility, when no text field
/// has the keyboard.
pub fn shortcuts(ctx: &egui::Context, doc: &Document, out: &mut Vec<Command>) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    use egui::{Key, KeyboardShortcut, Modifiers};
    let sel = doc.selection.query.clone();
    ctx.input_mut(|i| {
        let mut k = |m: Modifiers, key: Key| i.consume_shortcut(&KeyboardShortcut::new(m, key));
        if k(Modifiers::ALT, Key::H) {
            out.push(Edit::ShowAll.into());
        } else if k(Modifiers::SHIFT, Key::H) {
            if let Some(q) = &sel {
                out.push(Edit::Isolate(Some(q.clone())).into());
            }
        } else if k(Modifiers::NONE, Key::H) {
            if let Some(q) = &sel {
                out.push(Edit::Hide(q.clone()).into());
                out.push(Command::ClearSelection);
            }
        }
        if k(Modifiers::ALT, Key::G) {
            out.push(Edit::NoGhosts.into());
        } else if k(Modifiers::NONE, Key::G) {
            if let Some(q) = &sel {
                out.push(Edit::Ghost(q.clone()).into());
            }
        }
        if k(Modifiers::NONE, Key::F) {
            out.push(Command::FrameSelection);
        }
        if k(Modifiers::NONE, Key::O) {
            out.push(Command::Ortho(!doc.orbit.state.ortho));
        }
        if k(Modifiers::COMMAND, Key::I) {
            out.push(Command::InvertSelection);
        }
        if k(Modifiers::COMMAND, Key::ArrowUp) {
            out.push(Command::SelectParent);
        }
        // (The longer shortcuts first: Shift+Alt+← also matches Alt+←.)
        if k(Modifiers::SHIFT | Modifiers::ALT, Key::ArrowLeft) {
            out.push(Command::SelectionBack);
        } else if k(Modifiers::ALT, Key::ArrowLeft) {
            out.push(Command::CameraBack);
        }
        if k(Modifiers::SHIFT | Modifiers::ALT, Key::ArrowRight) {
            out.push(Command::SelectionForward);
        } else if k(Modifiers::ALT, Key::ArrowRight) {
            out.push(Command::CameraForward);
        }
        // Esc lets go of the selection and the marked point at once
        // (as a click on the 3D background does); during a surface's drag
        // it only takes the drag back (the inspector's).
        if i.key_pressed(Key::Escape) && doc.drag.is_none() {
            if sel.is_some() {
                out.push(Command::ClearSelection);
            }
            if doc.focus.is_some() {
                out.push(Command::Focus(None));
            }
            if doc.surface_pick.is_some() {
                out.push(Command::PickSurface(None));
            }
        }
    });
}
