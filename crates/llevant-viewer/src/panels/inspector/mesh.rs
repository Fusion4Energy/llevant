//! The mesh selected (the Tally workspace's FMESH), in the inspector: its
//! name and shape, what MCNP would object to, the voxels and the cells it
//! covers (sampled on a grid in it, cached per mesh and model).

use crate::command::Command;
use crate::context::{cell_link, material_link};
use crate::document::Document;
use crate::mesh::{MeshState, Problem};
use crate::{icons, theme};

use super::cell::LISTED;
use super::section;

/// Points sampled along each axis of a mesh's box to find the cells it
/// covers.
const COVER_SAMPLES: usize = 16;

/// The cells mesh `m` covers: those at the points of a grid in its box
/// that fall in one of its voxels (deepest cells, sorted, once each).
pub fn covered(doc: &Document, m: &MeshState) -> Vec<u32> {
    let (Ok(g), Some(b)) = (m.geometry(), m.world_box()) else {
        return Vec::new();
    };
    let n = COVER_SAMPLES;
    let mut cells: Vec<u32> = Vec::new();
    for i in 0..n {
        for j in 0..n {
            for k in 0..n {
                let t = [i, j, k].map(|a| (a as f64 + 0.5) / n as f64);
                let p = [0, 1, 2].map(|a| b.lo[a] + t[a] * (b.hi[a] - b.lo[a]));
                if g.locate(p).is_some() {
                    cells.extend(doc.model.classify_point(p).cell());
                }
            }
        }
    }
    cells.sort_unstable();
    cells.dedup();
    cells
}

/// The section of mesh `k` of the scene; `covers` the cells it covers.
pub fn section_of(
    ui: &mut egui::Ui,
    doc: &Document,
    k: usize,
    covers: &[u32],
    out: &mut Vec<Command>,
) {
    let meshes = &doc.scene.meshes;
    let Some(m) = meshes.get(k) else { return };
    ui.separator();
    super::title(
        ui,
        &format!("FMESH {}:{}", m.id, m.particle),
        Some(theme::MESH),
        (!m.comment.trim().is_empty()).then(|| m.comment.trim().to_string()),
    );
    let dims: Vec<String> = m
        .axes()
        .iter()
        .map(|v| v.len().saturating_sub(1).to_string())
        .collect();
    let voxels: usize = m
        .axes()
        .iter()
        .map(|v| v.len().saturating_sub(1).max(1))
        .product();
    ui.label(theme::data(format!(
        "{} {} · {voxels} voxels",
        if m.is_cylinder() { "cylinder" } else { "box" },
        dims.join(" × ")
    )));
    section(ui, ("mesh checks", k), "Checks", true, |ui| {
        let problems = m.problems(k, meshes, doc.input_meshes.as_deref(), &doc.region);
        if problems.is_empty() {
            ui.label(
                egui::RichText::new("MCNP would take it as it is")
                    .color(egui::Color32::from_rgb(110, 190, 120)),
            );
        }
        for p in problems {
            match p {
                Problem::Error(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, format!("● {e}"));
                }
                Problem::Warning(w) => {
                    ui.colored_label(ui.visuals().warn_fg_color, format!("● {w}"));
                }
            }
        }
    });
    section(ui, ("mesh covers", k), "Covers", true, |ui| {
        let n = covers.len();
        ui.label(format!(
            "{n} cell{} (sampled on a {COVER_SAMPLES}³ grid)",
            if n == 1 { "" } else { "s" }
        ));
        let mut mats: Vec<i64> = covers
            .iter()
            .map(|&c| doc.model.cell(c))
            .filter(|c| c.fill.is_none())
            .map(|c| c.material)
            .collect();
        mats.sort_unstable();
        mats.dedup();
        ui.horizontal_wrapped(|ui| {
            for m in mats {
                material_link(ui, doc, m, out);
            }
        });
        ui.horizontal_wrapped(|ui| {
            for &c in covers.iter().take(LISTED) {
                cell_link(ui, doc, c, out);
            }
            if n > LISTED {
                ui.label(egui::RichText::new(format!("+{} more", n - LISTED)).weak());
            }
        });
        if n > 0
            && ui
                .small_button(icons::with(icons::SELECT, "Select them"))
                .clicked()
        {
            let ids = covers.iter().map(|&c| doc.model.cell_id(c)).collect();
            out.push(Command::Select(
                llevant_scene::query::Query::Cells(ids),
                crate::selection::SelectMode::Replace,
            ));
        }
    });
}
