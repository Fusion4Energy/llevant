//! The command palette (Ctrl+K): type to find any command, or type what to
//! go to: a rule (`u 20040 & m 5`, a cell number) to select, hide or
//! isolate; `slice z 10`, `slice x=-3`; three numbers to look at a point;
//! `fmesh 14`, `fmesh new cyl` (Tally);
//! `open u 20040`; `level 0` (or `level all`); `ghost c 300`. Up and Down
//! choose, Enter runs, Esc closes.

use llevant_scene::query::Query;

use crate::app::frame::{Panel, View, Workspace};
use crate::command::{Command, Edit};
use crate::document::Document;
use crate::icons;
use crate::pick::pick_point;
use crate::selection::SelectMode;
use crate::state::{AxisName, Colouring};

/// Entries shown at most.
const SHOWN: usize = 12;

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    text: String,
    chosen: usize,
    /// Focus the text field on the next frame.
    focus: bool,
}

/// One thing the palette can do.
struct Entry {
    title: String,
    detail: String,
    commands: Vec<Command>,
}

impl Palette {
    /// Open with `text` typed (closed if it is empty).
    pub fn set_text(&mut self, text: &str) {
        self.open = !text.is_empty();
        self.text = text.into();
    }

    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.text.clear();
            self.chosen = 0;
            self.focus = true;
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context, doc: Option<&Document>, out: &mut Vec<Command>) {
        if !self.open {
            return;
        }
        let entries = entries(&self.text, doc);
        let (up, down, enter, esc) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        let n = entries.len().min(SHOWN);
        if down && n > 0 {
            self.chosen = (self.chosen + 1) % n;
        }
        if up && n > 0 {
            self.chosen = (self.chosen + n - 1) % n;
        }
        self.chosen = self.chosen.min(n.saturating_sub(1));
        let mut run = None;
        egui::Window::new("command palette")
            .title_bar(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 60.0])
            .fixed_size([520.0, 0.0])
            .show(ctx, |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.text)
                        .hint_text("command, rule (u 20040 & m 5), slice z 10, x y z…")
                        .desired_width(f32::INFINITY),
                );
                crate::theme::name(&r, "palette text");
                if self.focus {
                    r.request_focus();
                    self.focus = false;
                }
                if r.changed() {
                    self.chosen = 0;
                }
                ui.separator();
                if entries.is_empty() {
                    ui.label(egui::RichText::new("nothing matches").weak());
                }
                for (k, e) in entries.iter().take(SHOWN).enumerate() {
                    let text = egui::RichText::new(&e.title);
                    let b = egui::Button::selectable(k == self.chosen, text)
                        .right_text(egui::RichText::new(&e.detail).weak());
                    if ui.add_sized([ui.available_width(), 22.0], b).clicked() {
                        run = Some(k);
                    }
                }
            });
        if enter && n > 0 {
            run = Some(self.chosen);
        }
        if let Some(k) = run {
            out.extend(
                entries
                    .into_iter()
                    .nth(k)
                    .map(|e| e.commands)
                    .unwrap_or_default(),
            );
            self.open = false;
        }
        if esc {
            self.open = false;
        }
    }
}

/// What `text` can mean: what it names first, then the commands whose
/// names hold all its words.
fn entries(text: &str, doc: Option<&Document>) -> Vec<Entry> {
    let t = text.trim();
    let mut out = Vec::new();
    if let Some(doc) = doc {
        if !t.is_empty() {
            named(t, doc, &mut out);
        }
    }
    let words: Vec<String> = t
        .to_lowercase()
        .split_whitespace()
        .map(String::from)
        .collect();
    for e in commands(doc) {
        let name = e.title.to_lowercase();
        if words.iter().all(|w| name.contains(w.as_str())) {
            out.push(e);
        }
    }
    out
}

/// The entry showing universe level `l` (or every level).
/// What `fmesh …` can mean: a new box or cylinder drawn in the slice, or a
/// mesh by its tally number (one of the scene's, or a copy of the
/// input's).
fn fmesh_entries(rest: &str, doc: &Document, out: &mut Vec<Entry>) {
    let new = |cyl: bool| Entry {
        title: icons::with(
            icons::MESH,
            &format!(
                "New FMESH {}: drag in the slice",
                if cyl { "cylinder" } else { "box" }
            ),
        ),
        detail: "Tally".into(),
        commands: vec![
            Command::ShowPanel(Panel::Mesh),
            Command::DrawMesh(Some(cyl)),
        ],
    };
    match rest.strip_prefix("new").map(str::trim) {
        Some(kind) if "cylinder".starts_with(kind) && !kind.is_empty() => out.push(new(true)),
        Some(kind) if "box".starts_with(kind) || "rect".starts_with(kind) => out.push(new(false)),
        Some(_) => {}
        None => {
            if let Ok(id) = rest.parse::<u32>() {
                if let Some(k) = doc.scene.meshes.iter().position(|m| m.id == id) {
                    out.push(Entry {
                        title: icons::with(icons::MESH, &format!("FMESH{id}")),
                        detail: "Tally".into(),
                        commands: vec![
                            Command::ShowPanel(Panel::Mesh),
                            Command::SelectMesh(Some(k)),
                        ],
                    });
                }
                let input = doc.input_meshes.as_deref();
                let card = input.and_then(|i| i.cards.iter().flatten().find(|c| c.id == id));
                if let Some(c) = card {
                    let free = crate::mesh::InputMeshes::free_tally(input, &doc.scene.meshes);
                    let m = crate::mesh::MeshState {
                        id: free,
                        ..crate::mesh::MeshState::from_card(c)
                    };
                    out.push(Entry {
                        title: icons::with(
                            icons::MESH,
                            &format!("Edit a copy of the input's FMESH{id} (as FMESH{free})"),
                        ),
                        detail: format!("line {}", c.line),
                        commands: vec![
                            Edit::AddMesh(Box::new(m)).into(),
                            Command::ShowPanel(Panel::Mesh),
                        ],
                    });
                }
            }
            if rest.is_empty() {
                out.push(new(false));
                out.push(new(true));
            }
        }
    }
}

fn level_entry(l: Option<usize>) -> Entry {
    Entry {
        title: icons::with(
            icons::LEVELS,
            &match l {
                Some(k) => format!("Show level {k}"),
                None => "Show every level (deepest cells)".into(),
            },
        ),
        detail: match l {
            Some(0) => "the envelopes as cells, as MCNP's LEVEL 0".into(),
            Some(k) => format!("only cells at universe level {k}, as MCNP's LEVEL {k}"),
            None => "level".into(),
        },
        commands: vec![Edit::Level(l).into()],
    }
}

/// Entries for a rule, a slice, a point or a universe to open.
fn named(t: &str, doc: &Document, out: &mut Vec<Entry>) {
    let m = &doc.model;
    let lower = t.to_lowercase();
    // `slice z 10`, `slice x=-3`, `slice phi 30`
    if let Some(rest) = lower.strip_prefix("slice") {
        let rest = rest.replace('=', " ");
        let mut it = rest.split_whitespace();
        let word = it.next();
        let num = it.next().map(str::parse::<f64>);
        if let (Some("phi" | "φ" | "radial" | "r"), Some(Ok(phi))) = (word, &num) {
            out.push(Entry {
                title: icons::with(icons::SLICE, &format!("Radial slice at φ = {phi}°")),
                detail: "slice".into(),
                commands: vec![Command::SliceRadial(*phi)],
            });
            return;
        }
        let axis = match word {
            Some("x") => Some(AxisName::X),
            Some("y") => Some(AxisName::Y),
            Some("z") => Some(AxisName::Z),
            _ => None,
        };
        if let (Some(a), Some(Ok(at))) = (axis, num) {
            out.push(Entry {
                title: icons::with(icons::SLICE, &format!("Slice at {} = {at}", axis_name(a))),
                detail: "slice".into(),
                commands: vec![Command::SliceAt(a, at)],
            });
        }
        return;
    }
    // `fmesh 14`: that mesh (or a copy of the input's); `fmesh new cyl`
    if let Some(rest) = lower.strip_prefix("fmesh") {
        fmesh_entries(rest.trim(), doc, out);
        return;
    }
    // `level 0`, `level all`
    if let Some(rest) = lower.strip_prefix("level") {
        let rest = rest.trim();
        let level = match rest {
            "all" | "every" | "deepest" | "-1" => Some(None),
            _ => rest.parse::<usize>().ok().map(Some),
        };
        if let Some(l) = level.filter(|l| l.is_none_or(|k| k < doc.levels)) {
            out.push(level_entry(l));
        }
        return;
    }
    // `ghost c 300`
    if let Some(rest) = t
        .trim()
        .strip_prefix("ghost")
        .or(t.trim().strip_prefix("Ghost"))
    {
        if let Ok(q) = rest.trim().parse::<Query>() {
            out.push(Entry {
                title: icons::with(icons::GHOST, &format!("Ghost {q}")),
                detail: "see through it, its contents shown".into(),
                commands: vec![Edit::Ghost(q).into()],
            });
        }
        return;
    }
    // `open u 20040`
    if let Some(rest) = lower.strip_prefix("open") {
        let digits: String = rest
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        if let Ok(u) = digits.parse::<i64>() {
            if doc.index.unit(u).is_some() {
                out.push(Entry {
                    title: icons::with(icons::OPEN, &format!("Open u={u} on its own")),
                    detail: "universe".into(),
                    commands: vec![Command::OpenUniverse(u)],
                });
            }
        }
        return;
    }
    // Three numbers: a point.
    let nums: Vec<f64> = t
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    if nums.len() == 3 && t.split_whitespace().count() >= 3 {
        let p = [nums[0], nums[1], nums[2]];
        let picked = pick_point(m, p);
        out.push(Entry {
            title: icons::with(
                icons::TARGET,
                &format!("Go to ({}, {}, {})", p[0], p[1], p[2]),
            ),
            detail: picked.summary(m).join(" · "),
            commands: vec![
                Command::Focus(Some(picked)),
                Command::LookAt(p),
                Command::SliceThrough(p),
            ],
        });
    }
    // A rule.
    if let Ok(q) = t.parse::<Query>() {
        let Some(found) = doc.try_resolve(&q) else {
            // `shown` being worked out: the window is woken when it is.
            out.push(Entry {
                title: icons::with(icons::SELECT, &format!("Select {q}")),
                detail: "working out what the 3D view shows…".into(),
                commands: vec![Command::Select(q.clone(), SelectMode::Replace)],
            });
            return;
        };
        match found {
            Ok(set) => {
                let n = set.count();
                let detail = format!("{n} {}", if n == 1 { "cell" } else { "cells" });
                let mut push = |icon: &str, what: &str, commands: Vec<Command>| {
                    out.push(Entry {
                        title: icons::with(icon, &format!("{what} {q}")),
                        detail: detail.clone(),
                        commands,
                    })
                };
                push(
                    icons::SELECT,
                    "Select",
                    vec![Command::Select(q.clone(), SelectMode::Replace)],
                );
                push(
                    icons::FRAME,
                    "Frame",
                    vec![
                        Command::Select(q.clone(), SelectMode::Replace),
                        Command::FrameSelection,
                    ],
                );
                if !doc.selection.is_empty() {
                    push(
                        icons::SELECT,
                        "Add to selection:",
                        vec![Command::Select(q.clone(), SelectMode::Add)],
                    );
                }
                push(icons::HIDE, "Hide", vec![Edit::Hide(q.clone()).into()]);
                push(
                    icons::ISOLATE,
                    "Isolate",
                    vec![Edit::Isolate(Some(q.clone())).into()],
                );
            }
            Err(e) => out.push(Entry {
                title: e.to_string(),
                detail: String::new(),
                commands: Vec::new(),
            }),
        }
    }
}

fn axis_name(a: AxisName) -> &'static str {
    match a {
        AxisName::X => "x",
        AxisName::Y => "y",
        AxisName::Z => "z",
    }
}

/// Every command with a name.
fn commands(doc: Option<&Document>) -> Vec<Entry> {
    let e = |icon: &str, title: &str, keys: &str, commands: Vec<Command>| Entry {
        title: icons::with(icon, title),
        detail: keys.into(),
        commands,
    };
    let mut v = vec![
        e(icons::UNDO, "Undo", "Ctrl+Z", vec![Command::Undo]),
        e(icons::REDO, "Redo", "Ctrl+Shift+Z", vec![Command::Redo]),
        e(icons::SHOW, "Show all", "Alt+H", vec![Edit::ShowAll.into()]),
        e(
            icons::FIELD,
            "Open field…",
            "",
            vec![Command::OpenFieldDialog],
        ),
        e(icons::FIELD, "No field", "", vec![Edit::Field(None).into()]),
        e(
            icons::MESH,
            "FMESH: lay out mesh tallies (Tally)",
            "",
            vec![Command::ShowPanel(Panel::Mesh)],
        ),
        e(
            icons::SELECT,
            "Select all",
            "",
            vec![Command::Select(Query::All, SelectMode::Replace)],
        ),
        e(
            icons::INVERT,
            "Invert selection",
            "Ctrl+I",
            vec![Command::InvertSelection],
        ),
        e(
            icons::REMOVE,
            "Clear selection",
            "Esc",
            vec![Command::ClearSelection],
        ),
        e(
            icons::PARENT,
            "Select parent",
            "Ctrl+↑",
            vec![Command::SelectParent],
        ),
        e(
            icons::BACK,
            "Previous selection",
            "Shift+Alt+←",
            vec![Command::SelectionBack],
        ),
        e(
            icons::FORWARD,
            "Next selection",
            "Shift+Alt+→",
            vec![Command::SelectionForward],
        ),
        e(
            icons::FRAME,
            "Frame selection",
            "F",
            vec![Command::FrameSelection],
        ),
        e(
            icons::CAMERA,
            "Fit the 3D view to the model",
            "",
            vec![Command::FitCamera],
        ),
        e(
            icons::CAMERA,
            "Orthographic view",
            "O",
            vec![Command::Ortho(true)],
        ),
        e(
            icons::CAMERA,
            "Perspective view",
            "O",
            vec![Command::Ortho(false)],
        ),
        e(icons::SAVE, "Save view", "Ctrl+S", vec![Command::SaveView]),
        e(icons::KEYS, "Keys and mouse", "F1", vec![Command::ShowKeys]),
    ];
    if let Some(d) = doc {
        if let Some(q) = &d.selection.query {
            v.push(e(
                icons::HIDE,
                "Hide selection",
                "H",
                vec![Edit::Hide(q.clone()).into()],
            ));
            v.push(e(
                icons::ISOLATE,
                "Isolate selection",
                "Shift+H",
                vec![Edit::Isolate(Some(q.clone())).into()],
            ));
        }
        if let Some(q) = &d.selection.query {
            v.push(e(
                icons::GHOST,
                "Ghost selection",
                "G",
                vec![Edit::Ghost(q.clone()).into()],
            ));
        }
        if !d.scene.hiding.ghost.is_empty() {
            v.push(e(
                icons::GHOST,
                "No ghosts",
                "Alt+G",
                vec![Edit::NoGhosts.into()],
            ));
        }
        if d.levels > 1 {
            v.push(level_entry(None));
            for k in 0..d.levels {
                v.push(level_entry(Some(k)));
            }
        }
        if d.universe.is_some() {
            v.push(e(icons::BACK, "Back to the model", "", vec![Command::Back]));
        }
    }
    for (c, name) in [
        (Colouring::Cell, "cell"),
        (Colouring::Material, "material"),
        (Colouring::Universe, "universe"),
        (Colouring::Realistic, "realistic"),
        (Colouring::Density, "density"),
    ] {
        v.push(e(
            icons::COLOUR,
            &format!("Colour by {name}"),
            "",
            vec![Edit::Colouring(c).into()],
        ));
    }
    for w in Workspace::ALL {
        v.push(e(
            w.icon(),
            &format!("Workspace: {}", w.name()),
            &format!("Ctrl+{}", w.number()),
            vec![Command::Workspace(w)],
        ));
    }
    for p in Panel::ALL {
        v.push(Entry {
            title: format!("Show panel: {}", p.title()),
            detail: String::new(),
            commands: vec![Command::ShowPanel(p)],
        });
    }
    for (m, name) in [
        (None, "both views"),
        (Some(View::ThreeD), "the 3D view only"),
        (Some(View::Slice), "the slice only"),
    ] {
        v.push(e(
            icons::BOTH_VIEWS,
            &format!("Show {name}"),
            "",
            vec![Command::Maximize(m)],
        ));
    }
    v
}
