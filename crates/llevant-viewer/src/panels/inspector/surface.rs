//! A surface's page: its number, mnemonic and comment, its coefficients as
//! fields to drag (the views following live), the cells using it, and its
//! card and transform.

use llevant_scene::query::Query;
use llevant_scene::source::CardKind;

use super::card::{card, scrub, transform_cards};
use super::cell::LISTED;
use super::{section, title, Subject};
use crate::command::Command;
use crate::context::cell_link;
use crate::document::Document;
use crate::selection::SelectMode;
use crate::{icons, theme};

/// The page of surface `id`; `users` are the cells using it, `outside`
/// those of the whole model outside the universe opened on its own.
pub fn page(
    ui: &mut egui::Ui,
    doc: &Document,
    id: i64,
    users: &[u32],
    outside: &[i64],
    go: &mut Option<Subject>,
    out: &mut Vec<Command>,
) {
    let kind = doc
        .source
        .as_ref()
        .and_then(|s| s.surface_coeffs(id))
        .map(|(k, _)| k.to_uppercase());
    let name = match &kind {
        Some(k) => format!("s {id}  {k}"),
        None => format!("s {id}"),
    };
    title(ui, &name, None, super::comment(doc, CardKind::Surface, id));
    super::shared_outside(ui, doc, "It is", outside);
    ui.horizontal_wrapped(|ui| {
        if ui
            .small_button(icons::with(icons::SELECT, "Select cells on it"))
            .on_hover_text("Every cell whose card uses this surface (any facet or copy)")
            .clicked()
        {
            out.push(Command::Select(Query::surface(id), SelectMode::Replace));
        }
    });
    if doc.source.as_ref().is_some_and(|s| s.surface(id).is_some()) {
        super::many::renumber_one(ui, doc, llevant_scene::source::IdKind::Surface, id, out);
    }
    section(
        ui,
        ("surface coefficients", id),
        "Coefficients",
        true,
        |ui| {
            scrub(ui, doc, id, out);
        },
    );
    other_forms(ui, doc, id, out);
    section(ui, ("surface users", id), "Used by", true, |ui| {
        let n = users.len();
        ui.label(format!("{n} cell{}", if n == 1 { "" } else { "s" }));
        ui.horizontal_wrapped(|ui| {
            for &c in users.iter().take(LISTED) {
                cell_link(ui, doc, c, out);
            }
            if n > LISTED {
                ui.label(egui::RichText::new(format!("+{} more", n - LISTED)).weak());
            }
        });
        if n == 0 {
            ui.label(theme::data("no compiled cell uses it").weak());
        }
    });
    card(ui, doc, CardKind::Surface, id, true, go, out);
    transform_cards(ui, doc, CardKind::Surface, id, go, out);
}

/// The more general types surface `id` can be written as (a `CZ` as `C/Z`
/// to move it off the axis, a `PX` as `P` to tilt it), each written only
/// once confirmed.
fn other_forms(ui: &mut egui::Ui, doc: &Document, id: i64, out: &mut Vec<Command>) {
    let Some(source) = doc.source.as_ref() else {
        return;
    };
    let Some((kind, coeffs)) = source.surface_coeffs(id) else {
        return;
    };
    let forms = llevant_scene::source::general_forms(&kind, &coeffs);
    if forms.is_empty() {
        return;
    }
    // The form waiting to be confirmed.
    let key = egui::Id::new(("surface retype", id));
    let mut asked: Option<usize> = ui.data(|d| d.get_temp(key));
    section(ui, ("surface forms", id), "Other forms", false, |ui| {
        let upper = kind.to_uppercase();
        for (k, (m, c)) in forms.iter().enumerate() {
            let text = format!(
                "{m} {}",
                c.iter()
                    .map(|v| format!("{v}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::data(&text));
                if asked == Some(k) {
                    ui.label(egui::RichText::new(format!("write s {id} as {m}?")).weak());
                    if ui.small_button("Write it").clicked() {
                        match source.surface_retyped(id, m, c) {
                            Ok(t) => out.push(Command::EditCard(CardKind::Surface, id, t)),
                            Err(e) => out.push(Command::Warn(format!("could not edit: {e}"))),
                        }
                        asked = None;
                    }
                    if ui.small_button("Cancel").clicked() {
                        asked = None;
                    }
                } else {
                    let r = ui
                        .add_enabled(
                            doc.input_editable.is_ok(),
                            egui::Button::new(format!("As {m}")).small(),
                        )
                        .on_hover_text(format!(
                            "The same surface written as {m}, whose coefficients can move it where {upper}'s cannot (asks first)"
                        ));
                    if r.clicked() {
                        asked = Some(k);
                    }
                }
            });
            // What it writes, before it is confirmed.
            if asked == Some(k) {
                if let Ok(t) = source.surface_retyped(id, m, c) {
                    let before = source.surface(id).map(|c| c.text).unwrap_or_default();
                    super::diff::cards(ui, &[(format!("surface {id}"), before, t)], 0);
                }
            }
        }
    });
    ui.data_mut(|d| match asked {
        Some(k) => {
            d.insert_temp(key, k);
        }
        None => d.remove::<usize>(key),
    });
}
