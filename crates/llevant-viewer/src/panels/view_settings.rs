//! Settings of the views shown in popups: the 3D view's cuts and effects
//! and a radial slice's axis (from the views' headers), and tracing (from
//! the status bar's backend chip).

use crate::command::{Command, Edit};
use crate::cuts;
use crate::document::Document;
use crate::state::{BackendChoice, CutState};
use crate::{icons, theme};

pub fn tracing(ui: &mut egui::Ui, backend: BackendChoice, budget_ms: f64, out: &mut Vec<Command>) {
    theme::section(ui, icons::TRACING, "Tracing", |ui| {
        let mut b = backend;
        if theme::choice(
            ui,
            &mut b,
            &[
                (BackendChoice::Auto, "auto"),
                (BackendChoice::Gpu, "GPU"),
                (BackendChoice::Cpu, "CPU"),
            ],
        ) {
            out.push(Command::Backend(b));
        }
        // The time each preview aims to take, shown as the trade it makes:
        // the number meant little to users, and "shorter is smoother" was
        // wrong (2026-11-14: the window ran at the same frame rate from 16
        // to 100 ms; low-resolution previews flicker).
        let mut ms = budget_ms;
        let tip = "While the camera moves, the 3D view shows quick previews, moved \
                   with the camera between them. Sharp: fewer, sharper previews, \
                   which move steadily; what the camera uncovers fills in a little \
                   later. Responsive: more previews of lower resolution, which \
                   flicker. The window's frame rate is the same either way.";
        ui.label("While moving");
        let changed = ui
            .horizontal(|ui| {
                ui.label(egui::RichText::new("responsive").weak());
                let r = ui
                    .add(egui::Slider::new(&mut ms, crate::state::BUDGET_MS).show_value(false))
                    .on_hover_text(tip);
                ui.label(egui::RichText::new("sharp").weak());
                r.changed()
            })
            .inner;
        if changed {
            out.push(Command::Budget(ms));
        }
    });
}

/// How gaps and overlaps show in 3D: seen through or solid, and the
/// problems found drawn on top. In the Problems panel (Check workspace),
/// under "Problem display".
pub fn problem_options(ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
    let mut see = doc.scene.hiding.see_through;
    if ui
        .checkbox(&mut see, "see through gaps in 3D")
        .on_hover_text(
            "Gaps (and overlaps, on the CPU) as a veil of their pattern over what is behind \
             them; off, solid",
        )
        .changed()
    {
        out.push(Edit::SeeThrough(see).into());
    }
    let mut on_top = doc.problems_on_top;
    if ui
        .checkbox(&mut on_top, "problems found, on top in 3D")
        .on_hover_text(
            "The gaps and overlaps the check found (boxes) and the slice met (markers), drawn \
             through what hides them",
        )
        .changed()
    {
        out.push(Command::ProblemsOnTop(on_top));
    }
}

/// The 3D view's Display popup: effects and the projection.
pub fn effects(ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
    let mut fx = doc.scene.effects;
    let mut changed = ui
        .checkbox(&mut fx.outline, "outlines")
        .on_hover_text("Lines along cell boundaries")
        .changed();
    ui.add_enabled_ui(fx.outline, |ui| {
        if let Some(c) = outline_colour(ui, fx.outline_colour, [0; 3]) {
            fx.outline_colour = c;
            changed = true;
        }
    });
    changed |= ui
        .checkbox(&mut fx.ambient_occlusion, "ambient occlusion")
        .on_hover_text("Darker in corners and creases")
        .changed();
    if changed {
        out.push(Edit::Effects(fx).into());
    }
    let mut ortho = doc.orbit.state.ortho;
    if ui
        .checkbox(&mut ortho, "orthographic")
        .on_hover_text("Seen straight on, every ray parallel (O)")
        .changed()
    {
        out.push(Command::Ortho(ortho));
    }
}

/// The colour of a view's outlines: `current`, none for the view's own
/// (`own`); a few to pick at once (dark ones for light cells, light ones
/// for dark cells and backgrounds) and any other. The new one if changed.
pub fn outline_colour(
    ui: &mut egui::Ui,
    current: Option<[u8; 3]>,
    own: [u8; 3],
) -> Option<Option<[u8; 3]>> {
    let mut out = None;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("colour").weak());
        let now = current.unwrap_or(own);
        for (name, c) in [
            ("dark", own),
            ("grey", [128, 128, 128]),
            ("light", [215, 215, 215]),
            ("white", [255, 255, 255]),
        ] {
            let picked = if name == "dark" {
                current.is_none()
            } else {
                current == Some(c)
            };
            if ui.selectable_label(picked, name).clicked() {
                out = Some((name != "dark").then_some(c));
            }
        }
        let mut c = now;
        if egui::color_picker::color_edit_button_srgb(ui, &mut c)
            .on_hover_text("Any colour")
            .changed()
        {
            out = Some(Some(c));
        }
    });
    out
}

/// The scene's cuts: add them, set them by number (their handles drag them in
/// the 3D view, whatever the tool).
pub fn cuts_ui(ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
    let scene = &doc.scene;
    theme::section(ui, icons::CLIP, "Cuts", |ui| {
        ui.horizontal_wrapped(|ui| {
            let focus = doc.focus.as_ref().map_or(doc.region.center(), |f| f.point);
            if ui
                .button("+ plane")
                .on_hover_text("Through the point inspected, cutting away the side facing you")
                .clicked()
            {
                out.push(Edit::AddCut(cuts::plane_facing(doc, focus)).into());
            }
            if ui
                .button("+ wedge")
                .on_hover_text("A quarter about z, open towards you")
                .clicked()
            {
                let p = default_axis_point(&doc.region);
                out.push(Edit::AddCut(cuts::wedge_facing(doc, p)).into());
            }
            if ui
                .button("+ section box")
                .on_hover_text("Only what is inside: around the selection, or the middle")
                .clicked()
            {
                out.push(Edit::AddCut(cuts::section_box(doc)).into());
            }
        });
        for (k, c) in scene.cuts.iter().enumerate() {
            let selected = doc.editing.cut_selected == Some(k);
            let on = scene.cut_on(k);
            // A row: on or off (a cut off cuts nothing and is not drawn), then
            // its name, which selects it; the selected one is open.
            let id = ui.make_persistent_id(("cut", k));
            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                selected,
            );
            state.set_open(selected);
            let (_, header, _) = state
                .show_header(ui, |ui| {
                    let mut want = on;
                    let r = ui.checkbox(&mut want, "").on_hover_text(if on {
                        "On: click to turn it off (kept, cutting nothing)"
                    } else {
                        "Off: click to cut with it again"
                    });
                    theme::name(&r, &format!("cut {} on", k + 1));
                    if r.changed() {
                        out.push(Edit::CutOn(k, want).into());
                    }
                    let mut title = egui::RichText::new(format!("{}  {}", k + 1, c.describe()));
                    title = if on { title.strong() } else { title.weak() };
                    ui.selectable_label(selected, title)
                })
                .body(|ui| {
                    let mut next = *c;
                    let mut changed = false;
                    if ui
                        .small_button(icons::with(icons::REMOVE, "Remove"))
                        .clicked()
                    {
                        out.push(Edit::RemoveCut(k).into());
                    }
                    let drag = |ui: &mut egui::Ui, v: &mut f64, prefix: &str, speed: f64| {
                        ui.add(egui::DragValue::new(v).prefix(prefix).speed(speed))
                            .changed()
                    };
                    let speed = 0.002 * doc.orbit.radius;
                    match &mut next {
                        CutState::Plane { point, normal } => {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("point").weak());
                                for (k, p) in point.iter_mut().enumerate() {
                                    changed |= drag(ui, p, ["x ", "y ", "z "][k], speed);
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("normal").weak());
                                for (k, n) in normal.iter_mut().enumerate() {
                                    changed |= drag(ui, n, ["x ", "y ", "z "][k], 0.01);
                                }
                                if ui.small_button("flip").clicked() {
                                    *normal = normal.map(|x| -x);
                                    changed = true;
                                }
                            });
                            // The plane as the slice has it: along an axis at a
                            // position, or radial (through an axis, at an angle).
                            let n = llevant::vec3::normalize(*normal);
                            let aligned = cuts::along_axis(n);
                            let radial = if aligned.is_some() {
                                None
                            } else {
                                [2, 0, 1]
                                    .into_iter()
                                    .find_map(|k| cuts::radial_angle(k, n).map(|phi| (k, phi)))
                            };
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("plane").weak());
                                for (name, k) in [("x", 0), ("y", 1), ("z", 2)] {
                                    if ui.selectable_label(aligned == Some(k), name).clicked() {
                                        let mut e = [0.0; 3];
                                        e[k] = if n[k] < 0.0 { -1.0 } else { 1.0 };
                                        *normal = e;
                                        changed = true;
                                    }
                                }
                                if ui
                                    .selectable_label(radial.is_some(), "radial")
                                    .on_hover_text("Through an axis (z), turned to an angle φ")
                                    .clicked()
                                    && radial.is_none()
                                {
                                    *normal = cuts::radial_normal(2, 0.0);
                                    changed = true;
                                }
                                if ui
                                    .small_button("view")
                                    .on_hover_text("Facing you")
                                    .clicked()
                                {
                                    *normal = doc.orbit.forward().map(|x| -x);
                                    changed = true;
                                }
                            });
                            if let Some(k) = aligned {
                                let (lo, hi) = (doc.region.lo[k], doc.region.hi[k]);
                                let was = point[k];
                                ui.add(
                                    egui::Slider::new(&mut point[k], lo..=hi)
                                        .clamping(egui::SliderClamping::Never)
                                        .text("at"),
                                );
                                changed |= point[k] != was;
                            }
                            if let Some((k, phi)) = radial {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("axis along").weak());
                                    for (name, a) in [("z", 2), ("x", 0), ("y", 1)] {
                                        if ui.selectable_label(k == a, name).clicked() && k != a {
                                            *normal = cuts::radial_normal(a, 0.0);
                                            changed = true;
                                        }
                                    }
                                });
                                // (Only a real change counts: a slider that rounds or
                                // clamps what it shows would edit the cut every frame.)
                                let mut p = phi;
                                ui.add(
                                    egui::Slider::new(&mut p, -180.0..=180.0)
                                        .clamping(egui::SliderClamping::Never)
                                        .text("φ °"),
                                );
                                if (p - phi).abs() > 1e-9 {
                                    *normal = cuts::radial_normal(k, p);
                                    changed = true;
                                }
                            }
                        }
                        CutState::Wedge {
                            point, from, to, ..
                        } => {
                            ui.horizontal(|ui| {
                                changed |= drag(ui, from, "from ", 0.5);
                                changed |= drag(ui, to, "to ", 0.5);
                                ui.label("°");
                            });
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("axis through").weak());
                                changed |= drag(ui, &mut point[0], "x ", speed);
                                changed |= drag(ui, &mut point[1], "y ", speed);
                            });
                        }
                        CutState::Box { lo, hi, keep } => {
                            for (name, v) in [("min", lo), ("max", hi)] {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(name).weak());
                                    for (k, x) in v.iter_mut().enumerate() {
                                        changed |= drag(ui, x, ["x ", "y ", "z "][k], speed);
                                    }
                                });
                            }
                            changed |= ui.checkbox(keep, "keep only the inside").changed();
                        }
                    }
                    if changed {
                        out.push(Edit::SetCut(k, next).into());
                    }
                });
            if header.inner.clicked() {
                out.push(Command::SelectCut(if selected { None } else { Some(k) }));
            }
        }
        ui.label(
            egui::RichText::new(
                "In 3D: drag a cut's dots to move it; click its outline for the rings that turn it",
            )
            .weak(),
        );
    });
}

/// A radial slice's axis: the point it goes through and its direction.
pub fn radial_axis(ui: &mut egui::Ui, doc: &mut Document) {
    let Some(r) = &mut doc.slice.radial else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("axis through").weak());
        ui.add(
            egui::DragValue::new(&mut r.point[0])
                .prefix("x ")
                .speed(0.5),
        );
        ui.add(
            egui::DragValue::new(&mut r.point[1])
                .prefix("y ")
                .speed(0.5),
        );
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("axis along").weak());
        for (name, axis, zero) in [
            ("z", [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            ("x", [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ("y", [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ] {
            if ui.selectable_label(r.axis == axis, name).clicked() {
                r.axis = axis;
                r.zero = zero;
            }
        }
    });
    let (lo, hi) = doc.slice.at_range();
    ui.add(egui::Slider::new(&mut doc.slice.at, lo..=hi).text("off axis"));
}

/// Where a radial slice's axis goes through by default: x = y = 0 when the
/// region spans it (tori and cylinders are mostly built about the origin),
/// else the region's centre.
pub fn default_axis_point(r: &llevant::Aabb) -> [f64; 3] {
    let c = r.center();
    let at = |k: usize| {
        if r.lo[k] <= 0.0 && 0.0 <= r.hi[k] {
            0.0
        } else {
            c[k]
        }
    };
    [at(0), at(1), c[2]]
}
