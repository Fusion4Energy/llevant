//! The inspector's first page: what is picked. The selection (how many
//! cells, its rule, by universe and material), the point double-clicked
//! (coordinates, the path down to its cell, the field there), the page of
//! its cell (or of the one cell selected), and the surface picked with it.

use llevant::Status;
use llevant_scene::query::Query;
use llevant_scene::source::CardKind;

use super::card::{card, scrub, transform_cards};
use super::{cell, Subject};
use crate::command::{Command, Edit};
use crate::context::{cell_link, link, material_link, surface_link, universe_link};
use crate::document::Document;
use crate::scene::Pin;
use crate::selection::SelectMode;
use crate::{icons, theme};

/// Groups listed for the selection (by universe, by material).
const GROUPS: usize = 6;

pub fn page(
    ui: &mut egui::Ui,
    doc: &Document,
    forms: &mut super::fields::Forms,
    many: &mut Option<super::many::ManyForm>,
    go: &mut Option<Subject>,
    out: &mut Vec<Command>,
) {
    selection(ui, doc, many, out);
    point(ui, doc, out);
    // The cell: the point's, or the one selected.
    let sel = &doc.selection.cells;
    let one = (sel.count() == 1).then(|| sel.iter().next()).flatten();
    let c = doc.focus.as_ref().and_then(|p| p.cell()).or(one);
    if let Some(c) = c {
        ui.separator();
        cell::page(ui, doc, c, forms, go, out);
    }
    surface(ui, doc, go, out);
}

/// Whether there is anything picked.
pub fn has_content(doc: &Document) -> bool {
    !doc.selection.is_empty() || doc.focus.is_some()
}

fn selection(
    ui: &mut egui::Ui,
    doc: &Document,
    many: &mut Option<super::many::ManyForm>,
    out: &mut Vec<Command>,
) {
    let Some(q) = &doc.selection.query else {
        return;
    };
    theme::section(ui, icons::SELECT, "Selection", |ui| {
        let n = doc.selection.cells.count();
        rule(ui, doc, q, n, out);
        ui.horizontal(|ui| {
            let b = |ui: &mut egui::Ui, icon, tip: &str| ui.small_button(icon).on_hover_text(tip);
            if b(ui, icons::HIDE, "Hide (H)").clicked() {
                out.push(Edit::Hide(q.clone()).into());
                out.push(Command::ClearSelection);
            }
            if b(ui, icons::ISOLATE, "Isolate: hide all else (Shift+H)").clicked() {
                out.push(Edit::Isolate(Some(q.clone())).into());
            }
            if b(ui, icons::FRAME, "Frame (F)").clicked() {
                out.push(Command::FrameSelection);
            }
            if b(ui, icons::INVERT, "Invert (Ctrl+I)").clicked() {
                out.push(Command::InvertSelection);
            }
            ui.add_enabled_ui(doc.selection.can_go_back(), |ui| {
                if b(ui, icons::BACK, "Previous selection (Shift+Alt+←)").clicked() {
                    out.push(Command::SelectionBack);
                }
            });
            ui.add_enabled_ui(doc.selection.can_go_forward(), |ui| {
                if b(ui, icons::FORWARD, "Next selection (Shift+Alt+→)").clicked() {
                    out.push(Command::SelectionForward);
                }
            });
            if b(ui, icons::REMOVE, "Clear (Esc)").clicked() {
                out.push(Command::ClearSelection);
            }
        });
        if n > 1 {
            groups(ui, doc, out);
        }
        if many.is_none() {
            let r = ui.add_enabled(
                doc.input_editable.is_ok(),
                egui::Button::new("Edit cells").small(),
            );
            crate::app::frame::named(&r, "Edit the cells selected");
            let r = match doc.input_editable {
                Ok(()) => r.on_hover_text(
                    "Change their material, density, parameters or numbers together, as one edit",
                ),
                Err(why) => r.on_disabled_hover_text(why),
            };
            if r.clicked() {
                *many = super::many::ManyForm::open(doc);
            }
        }
        if let Some(f) = many {
            f.ui(ui, doc, out);
            if ui.small_button("Close").clicked() {
                *many = None;
            }
        }
    });
}

/// Characters of the selection's rule shown folded (a few lines), and
/// unfolded (past that, Copy rule in its menu gives it all).
const RULE_FOLDED: usize = 120;
const RULE_UNFOLDED: usize = 20_000;

/// The selection's size, `n` cells, and its rule `q` as a link: folded to
/// its first characters when long, with a toggle beside the size (where it
/// stays put) to show it all (up to [`RULE_UNFOLDED`]), so that a large
/// selection does not push the rest of the inspector down.
fn rule(ui: &mut egui::Ui, doc: &Document, q: &Query, n: usize, out: &mut Vec<Command>) {
    let text = q.to_string();
    let chars = text.chars().count();
    let id = ui.id().with("selection rule unfolded");
    let unfolded = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    ui.horizontal(|ui| {
        ui.label(theme::data(format!(
            "{n} {}",
            if n == 1 { "cell" } else { "cells" }
        )));
        if chars > RULE_FOLDED {
            let toggle = if unfolded { "show less" } else { "show all" };
            if ui
                .add(
                    egui::Button::new(egui::RichText::new(toggle).weak())
                        .small()
                        .frame(false),
                )
                .on_hover_text(format!("The rule has {chars} characters"))
                .clicked()
            {
                ui.data_mut(|d| d.insert_temp(id, !unfolded));
            }
        }
    });
    let (shown, cut) =
        crate::context::elide(&text, if unfolded { RULE_UNFOLDED } else { RULE_FOLDED });
    ui.horizontal_wrapped(|ui| link(ui, doc, q, theme::data(shown), out));
    if cut && unfolded {
        ui.label(egui::RichText::new("(right-click › Copy rule for all of it)").weak());
    }
}

/// The selection's cells by universe and by material, largest first.
fn groups(ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
    let g = &doc.selection.groups;
    let row = |ui: &mut egui::Ui,
               title: &str,
               v: &[(i64, usize)],
               out: &mut Vec<Command>,
               f: &dyn Fn(&mut egui::Ui, i64, &mut Vec<Command>)| {
        if v.is_empty() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(title).weak());
            for &(k, n) in v.iter().take(GROUPS) {
                f(ui, k, out);
                ui.label(theme::data(format!("{n}")));
            }
            if v.len() > GROUPS {
                ui.label(egui::RichText::new(format!("+{} more", v.len() - GROUPS)).weak());
            }
        });
    };
    row(ui, "by universe", &g.by_universe, out, &|ui, u, out| {
        universe_link(ui, doc, u, out)
    });
    row(ui, "by material", &g.by_material, out, &|ui, mat, out| {
        material_link(ui, doc, mat, out)
    });
}

fn point(ui: &mut egui::Ui, doc: &Document, out: &mut Vec<Command>) {
    let Some(p) = &doc.focus else {
        return;
    };
    let m = &doc.model;
    theme::section(ui, icons::PICKED, "Point", |ui| {
        let x = p.point;
        let r = x[0].hypot(x[1]);
        let phi = x[1].atan2(x[0]).to_degrees();
        // Cartesian and cylindrical (about z), side by side.
        let grid = egui::Grid::new("point")
            .num_columns(4)
            .spacing([6.0, 1.0])
            .show(ui, |ui| {
                let weak = |s: &str| egui::RichText::new(s).weak();
                for (a, v, b, w) in [
                    ("x", x[0], "r", r),
                    ("y", x[1], "φ", phi),
                    ("z", x[2], "", f64::NAN),
                ] {
                    ui.label(weak(a));
                    ui.label(theme::data(format!("{v:.4}")));
                    ui.label(weak(b));
                    if w.is_finite() {
                        let unit = if b == "φ" { "°" } else { "" };
                        ui.label(theme::data(format!("{w:.4}{unit}")));
                    }
                    ui.end_row();
                }
            })
            .response;
        grid.context_menu(|ui| {
            if ui.button(icons::with(icons::COPY, "Copy point")).clicked() {
                ui.ctx().copy_text(format!("{} {} {}", x[0], x[1], x[2]));
                ui.close();
            }
        });
        if let Some(t) = p.t {
            ui.label(egui::RichText::new(format!("{t:.3} cm from the eye")).weak());
        }
        let status = match p.label.status {
            Status::Inside => None,
            Status::Gap => Some("gap: no cell holds it"),
            Status::Overlap => Some("overlap"),
            Status::OnSurface => Some("on a surface"),
            Status::Unsupported => Some("unsupported cell"),
            Status::TooDeep => Some("nesting too deep"),
            _ => Some("unknown"),
        };
        if let Some(s) = status {
            ui.colored_label(ui.visuals().warn_fg_color, s);
        }
        ui.add_space(4.0);
        // The path, one level per row: cell, universe, lattice element and
        // what the cell is made of.
        let path = &p.label.path;
        egui::Grid::new("path")
            .num_columns(3)
            .spacing([6.0, 2.0])
            .show(ui, |ui| {
                for (level, s) in path.iter().enumerate() {
                    let c = m.cell(s.cell);
                    let u = m.units()[c.unit as usize].universe;
                    let deepest = level + 1 == path.len();
                    let sel = doc.selection.cells.contains(s.cell);
                    let tag = egui::RichText::new(format!("L{level}")).weak();
                    ui.label(if sel {
                        tag.color(theme::SELECTION)
                    } else {
                        tag
                    });
                    ui.horizontal(|ui| {
                        cell_link(ui, doc, s.cell, out);
                        universe_link(ui, doc, u, out);
                        if let Some(ijk) = s.lattice {
                            ui.label(theme::data(format!("[{} {} {}]", ijk[0], ijk[1], ijk[2])));
                        }
                    });
                    ui.horizontal(|ui| {
                        if c.fill.is_some() {
                            ui.label(egui::RichText::new("filled").weak());
                        } else {
                            material_link(ui, doc, c.material, out);
                            if c.material != 0 {
                                let a = super::atomic_mass(doc, c.material);
                                ui.label(theme::data(super::density(c.density, a)));
                            }
                        }
                        if deepest && c.imp_zero {
                            ui.label(egui::RichText::new("imp 0").weak());
                        }
                    });
                    ui.end_row();
                }
            });
        // Which copy of its cell this is, and the field there.
        if let Some(id) = m.instance(&p.label) {
            let text = m.format_path(path);
            let r = ui
                .horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("instance").weak());
                    ui.label(theme::data(format!("{id}")));
                    ui.label(theme::data(&text).weak());
                })
                .response
                .on_hover_text("Which place of its cell this is (MCNP's repeated-structure path)");
            r.context_menu(|ui| {
                if ui
                    .button(icons::with(icons::COPY, "Copy instance path"))
                    .clicked()
                {
                    ui.ctx().copy_text(text.clone());
                    ui.close();
                }
            });
        }
        if let Some(f) = &doc.field {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(icons::with(icons::FIELD, &f.field.name)).weak());
                match f.field.value(m, &p.label, p.point) {
                    Some(v) => {
                        let [r, g, b] = f.look.colour(v);
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter()
                            .rect_filled(rect, 2.0, egui::Color32::from_rgb(r, g, b));
                        ui.label(theme::data(format!(
                            "{} {}",
                            llevant_render::scale::value_label(v),
                            f.field.unit
                        )));
                    }
                    None => {
                        ui.label(egui::RichText::new("no value").weak());
                    }
                }
            });
        }
        if !p.label.involved.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label("cells involved:");
                for &c in &p.label.involved {
                    cell_link(ui, doc, c, out);
                }
            });
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button(icons::with(icons::SLICE, "Slice here")).clicked() {
                out.push(Command::SliceThrough(p.point));
            }
            if ui.button(icons::with(icons::PIN, "Pin")).clicked() {
                out.push(
                    Edit::AddPin(Pin {
                        point: p.point,
                        note: String::new(),
                    })
                    .into(),
                );
            }
            if path.len() > 1
                && ui
                    .button(icons::with(icons::PARENT, "Parent"))
                    .on_hover_text("Select the cell holding the selected one (Ctrl+↑)")
                    .clicked()
            {
                out.push(Command::SelectParent);
            }
            if let Some(c) = p.cell() {
                let id = m.cell_id(c);
                if !doc.selection.cells.contains(c)
                    && ui.button(icons::with(icons::SELECT, "Select")).clicked()
                {
                    out.push(Command::Select(Query::cell(id), SelectMode::Replace));
                }
            }
            if ui
                .button(icons::REMOVE)
                .on_hover_text("Forget this point")
                .clicked()
            {
                out.push(Command::Focus(None));
            }
        });
        // The universes below the top level of the path, each a model of
        // its own to open.
        let mut universes: Vec<i64> = Vec::new();
        for s in path.iter().skip(1) {
            let u = m.units()[m.cell(s.cell).unit as usize].universe;
            if !universes.contains(&u) {
                universes.push(u);
            }
        }
        if !universes.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for u in universes {
                    let r = ui
                        .button(icons::with(icons::OPEN, &format!("Open u={u}")))
                        .on_hover_text(
                            "This universe on its own, cut to what it holds (Back returns here)",
                        );
                    if r.clicked() {
                        out.push(Command::OpenUniverse(u));
                    }
                }
            });
        }
    });
}

/// Cells listed by name under a surface, at most.
const SURFACE_USERS: usize = 12;

/// The surface at the point: its number, the side the point's cell takes,
/// the surfaces coincident with it, the cells using it and its card.
fn surface(ui: &mut egui::Ui, doc: &Document, go: &mut Option<Subject>, out: &mut Vec<Command>) {
    let Some(p) = &doc.focus else {
        return;
    };
    let Some(s) = &p.surface else {
        return;
    };
    let m = &doc.model;
    let Some(id) = m.surface_id(s.shape) else {
        return;
    };
    let weak = |s: String| egui::RichText::new(s).weak();
    let name = crate::pick::surface_name(id);
    theme::section(ui, icons::TARGET, &format!("Surface {name}"), |ui| {
        ui.horizontal_wrapped(|ui| {
            surface_link(ui, doc, s.shape, out);
            super::drill(ui, Subject::Surface(id.id), go);
            if s.distance > 0.0 {
                ui.label(weak(format!("{:.4} cm from the point", s.distance)));
            }
        });
        // The side the point's cell takes, when its card bounds it there.
        if let Some(c) = p.cell() {
            let senses: Vec<bool> = m
                .cell_surfaces(c)
                .into_iter()
                .filter(|&(x, _)| x == s.shape)
                .map(|(_, pos)| pos)
                .collect();
            let side = match senses.as_slice() {
                [true] => Some("its positive side (+)"),
                [false] => Some("its negative side (−)"),
                [] => None,
                _ => Some("both sides"),
            };
            if let Some(side) = side {
                ui.horizontal_wrapped(|ui| {
                    cell_link(ui, doc, c, out);
                    ui.label(weak(format!("is on {side}")));
                });
            }
        }
        let others: Vec<u32> = p
            .through
            .iter()
            .copied()
            .filter(|&x| x != s.shape)
            .collect();
        if !others.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(ui.visuals().warn_fg_color, "coincident:")
                    .on_hover_text(
                        "Other surfaces through this point, within the tolerance, that bound a cell here",
                    );
                for x in others {
                    surface_link(ui, doc, x, out);
                }
            });
        }
        if let Some(users) = &p.users {
            ui.horizontal_wrapped(|ui| {
                let n = users.len();
                ui.label(weak(format!(
                    "used by {n} cell{}",
                    if n == 1 { "" } else { "s" }
                )));
                if n <= SURFACE_USERS {
                    for &c in users {
                        cell_link(ui, doc, c, out);
                    }
                }
            });
            if !users.is_empty()
                && ui
                    .button(icons::with(
                        icons::SELECT,
                        &format!("Select cells on s {}", id.id),
                    ))
                    .on_hover_text("Every cell whose card uses this surface (any facet or copy)")
                    .clicked()
            {
                out.push(Command::Select(Query::surface(id.id), SelectMode::Replace));
            }
        }
        scrub(ui, doc, id.id, out);
        card(ui, doc, CardKind::Surface, id.id, true, go, out);
        transform_cards(ui, doc, CardKind::Surface, id.id, go, out);
    });
}

#[cfg(test)]
mod tests {
    use crate::context::elide;

    #[test]
    fn rules_are_cut_at_a_space() {
        assert_eq!(elide("c 1 2 3", 20), ("c 1 2 3".to_string(), false));
        assert_eq!(elide("c 10 11 12 13", 9), ("c 10 11 …".to_string(), true));
        assert_eq!(elide("c 123456789", 5), ("c …".to_string(), true));
        assert_eq!(elide("abcdefgh", 3), ("abc …".to_string(), true));
    }
}
