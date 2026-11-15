//! The window's layout (decision 0012): four workspaces, each with its
//! panels on the left, the 3D view and the slice side by side in the
//! middle, and the inspector on the right. The layout is the same in all:
//! switching workspaces changes what the left side shows, never the sides'
//! widths or folds nor where the views are. The sides can be resized and
//! folded to a rail; each workspace remembers the panel it last showed.
//! Above the views, a row of tools and what applies to both views; each
//! view has a header with its own settings and can be maximised.
//!
//! The selection, the camera, the slice and what is hidden are the
//! document's, so they stay the same from one workspace to another.

use serde::{Deserialize, Serialize};

use crate::command::Command;
use crate::icons;
use crate::panels::find::Mode;
use crate::panels::{headers, inspector, toolbar};

/// A kind of work, with the panels it needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Workspace {
    /// What is where: the model, saved views, pins, volumes.
    #[default]
    Explore,
    /// Gaps, overlaps and the input's diagnostics; statistics.
    Check,
    /// Fields and tallies over the geometry.
    Results,
    /// FMESH meshes laid out (shown, with their handles, here only).
    Tally,
}

impl Workspace {
    pub const ALL: [Workspace; 4] = [
        Workspace::Explore,
        Workspace::Check,
        Workspace::Results,
        Workspace::Tally,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Workspace::Explore => "Explore",
            Workspace::Check => "Check",
            Workspace::Results => "Results",
            Workspace::Tally => "Tally",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Workspace::Explore => icons::EXPLORE,
            Workspace::Check => icons::CHECK,
            Workspace::Results => icons::RESULTS,
            Workspace::Tally => icons::MESH,
        }
    }

    /// What it is for, for its tab's tooltip.
    pub fn hint(self) -> &'static str {
        match self {
            Workspace::Explore => "What is where: the model, saved views, pins, volumes",
            Workspace::Check => "Gaps, overlaps and the input's diagnostics; statistics",
            Workspace::Results => "Fields and tallies over the geometry",
            Workspace::Tally => "FMESH meshes: lay them out and write their cards",
        }
    }

    /// Its key, Ctrl and this number.
    pub fn number(self) -> usize {
        self.index() + 1
    }

    fn index(self) -> usize {
        Workspace::ALL.iter().position(|&w| w == self).unwrap_or(0)
    }

    /// The panels on its left, the first shown at first.
    pub fn panels(self) -> &'static [Panel] {
        match self {
            Workspace::Explore => &[
                Panel::Find,
                Panel::Materials,
                Panel::Envelopes,
                Panel::Volumes,
                Panel::Places,
            ],
            Workspace::Check => &[Panel::Problems, Panel::Statistics],
            Workspace::Results => &[Panel::Fields],
            Workspace::Tally => &[Panel::Mesh],
        }
    }
}

/// A panel: on the left of its workspace, or the inspector on the right of
/// every one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Panel {
    /// The rule box and its matches (once the outliner, with Materials and
    /// Envelopes in it).
    #[serde(alias = "Outliner")]
    Find,
    /// The legend of the materials.
    Materials,
    /// The level-0 cells and what fills them.
    Envelopes,
    Volumes,
    /// The views saved and the pins (once two panels).
    #[serde(alias = "Views", alias = "Pins")]
    Places,
    Problems,
    Statistics,
    Fields,
    Mesh,
    Inspector,
}

impl Panel {
    pub const ALL: [Panel; 10] = [
        Panel::Find,
        Panel::Materials,
        Panel::Envelopes,
        Panel::Volumes,
        Panel::Places,
        Panel::Problems,
        Panel::Statistics,
        Panel::Fields,
        Panel::Mesh,
        Panel::Inspector,
    ];

    pub fn icon(self) -> &'static str {
        match self {
            Panel::Find => icons::SEARCH,
            Panel::Materials => icons::MATERIAL,
            Panel::Envelopes => icons::UNIVERSE,
            Panel::Volumes => icons::CUBOID_ALT,
            Panel::Places => icons::PIN,
            Panel::Problems => icons::TARGET,
            Panel::Statistics => icons::INFO,
            Panel::Fields => icons::FIELD,
            Panel::Mesh => icons::MESH,
            Panel::Inspector => icons::INFO,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Panel::Find => "Find",
            Panel::Materials => "Materials",
            Panel::Envelopes => "Envelopes",
            Panel::Volumes => "Volumes",
            Panel::Places => "Places",
            Panel::Problems => "Problems",
            Panel::Statistics => "Statistics",
            Panel::Fields => "Fields",
            Panel::Mesh => "FMESH",
            Panel::Inspector => "Inspector",
        }
    }

    /// Its tab's label.
    pub fn title(self) -> String {
        icons::with(self.icon(), self.name())
    }

    /// The workspace it belongs to (`None`: the inspector, in all).
    pub fn home(self) -> Option<Workspace> {
        Workspace::ALL
            .into_iter()
            .find(|w| w.panels().contains(&self))
    }
}

/// One of the two views.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    ThreeD,
    Slice,
}

/// A side of a workspace: shown or folded to a rail, and its width.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Side {
    pub open: bool,
    pub width: f32,
}

impl Default for Side {
    fn default() -> Side {
        Side {
            open: true,
            width: 280.0,
        }
    }
}

/// The layout: the workspace shown, the panel each one shows on its left,
/// the two sides (the same in every workspace; all remembered between
/// runs), and the view maximised (not remembered).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Frame {
    pub workspace: Workspace,
    /// One per workspace, in the order of [`Workspace::ALL`].
    pub panels: Vec<Panel>,
    pub left: Side,
    pub right: Side,
    #[serde(skip)]
    pub maximized: Option<View>,
}

impl Default for Frame {
    fn default() -> Frame {
        Frame {
            workspace: Workspace::Explore,
            panels: Workspace::ALL.map(|w| w.panels()[0]).to_vec(),
            left: Side::default(),
            right: Side {
                open: true,
                width: 320.0,
            },
            maximized: None,
        }
    }
}

/// Side widths are kept within these (points).
const SIDE_WIDTH: std::ops::RangeInclusive<f32> = 200.0..=640.0;
/// A folded side's rail.
const RAIL: f32 = 34.0;

impl Frame {
    /// As read from the settings, made whole: a panel per workspace, each
    /// one of its own, and the sides' widths within bounds.
    pub fn normalized(mut self) -> Frame {
        self.panels.resize(Workspace::ALL.len(), Panel::Find);
        for (w, p) in Workspace::ALL.into_iter().zip(&mut self.panels) {
            if !w.panels().contains(p) {
                *p = w.panels()[0];
            }
        }
        for s in [&mut self.left, &mut self.right] {
            s.width = s.width.clamp(*SIDE_WIDTH.start(), *SIDE_WIDTH.end());
        }
        self
    }

    /// The panel the workspace shown has on its left.
    pub fn panel(&self) -> Panel {
        self.panels[self.workspace.index()]
    }

    fn set_panel(&mut self, p: Panel) {
        let k = self.workspace.index();
        self.panels[k] = p;
    }

    /// Bring `p` into sight: its workspace, its tab, its side unfolded.
    pub fn show(&mut self, p: Panel) {
        match p.home() {
            Some(w) => {
                self.workspace = w;
                self.set_panel(p);
                self.left.open = true;
            }
            None => self.right.open = true,
        }
    }

    /// Whether `p` is in sight.
    pub fn shows(&self, p: Panel) -> bool {
        match p.home() {
            Some(w) => w == self.workspace && self.panel() == p && self.left.open,
            None => self.right.open,
        }
    }

    /// The views shown: both (3D on the left, in every workspace), or the
    /// one maximised.
    pub fn views(&self) -> &'static [View] {
        match self.maximized {
            None => &[View::ThreeD, View::Slice],
            Some(View::ThreeD) => &[View::ThreeD],
            Some(View::Slice) => &[View::Slice],
        }
    }

    /// The share of the width each of [`Frame::views`] takes.
    pub fn shares(&self) -> &'static [f32] {
        match self.views().len() {
            1 => &[1.0],
            _ => &[0.5, 0.5],
        }
    }
}

/// The workspaces' tabs (in the menu bar's row).
pub fn workspace_tabs(ui: &mut egui::Ui, frame: &Frame, out: &mut Vec<Command>) {
    for w in Workspace::ALL {
        let r = ui
            .selectable_label(frame.workspace == w, icons::with(w.icon(), w.name()))
            .on_hover_text(format!("{} (Ctrl+{})", w.hint(), w.number()));
        if r.clicked() && frame.workspace != w {
            out.push(Command::Workspace(w));
        }
    }
}

impl super::ViewerApp {
    /// The frame's keys: Ctrl+1 to Ctrl+4 for the workspaces, Ctrl+[ and
    /// Ctrl+] fold the sides.
    pub(super) fn frame_keys(&mut self, ctx: &egui::Context, out: &mut Vec<Command>) {
        if ctx.egui_wants_keyboard_input() || self.doc.is_none() {
            return;
        }
        use egui::{Key, Modifiers};
        ctx.input_mut(|i| {
            for (w, key) in
                Workspace::ALL
                    .into_iter()
                    .zip([Key::Num1, Key::Num2, Key::Num3, Key::Num4])
            {
                if i.consume_key(Modifiers::COMMAND, key) {
                    out.push(Command::Workspace(w));
                }
            }
            if i.consume_key(Modifiers::COMMAND, Key::OpenBracket) {
                out.push(Command::FoldLeft);
            }
            if i.consume_key(Modifiers::COMMAND, Key::CloseBracket) {
                out.push(Command::FoldRight);
            }
        });
    }

    /// The meshes and their handles are on in Tally only; leaving it stops
    /// drawing a new one.
    pub(super) fn sync_tool_and_workspace(&mut self) {
        let ws = self.frame.workspace;
        self.tools.meshes_on = ws == Workspace::Tally;
        if ws != self.last_workspace && self.last_workspace == Workspace::Tally {
            if let Some(d) = &mut self.doc {
                d.editing.drawing_mesh = None;
            }
        }
        self.last_workspace = ws;
    }

    /// The sides and the views of the workspace shown, around `doc`.
    pub(super) fn workspace_ui(&mut self, ui: &mut egui::Ui, out: &mut Vec<Command>) {
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        let ws = self.frame.workspace;
        let fill = crate::theme::palette(ui.ctx()).panel;
        let panel_frame = egui::Frame::NONE
            .fill(fill)
            .inner_margin(egui::Margin::symmetric(8, 6));

        // Left: the workspace's panels, or their rail. (The sides' ids are
        // the same in every workspace: egui keeps their widths.)
        let (left, right, panel) = (self.frame.left, self.frame.right, self.frame.panel());
        if left.open {
            let r = egui::Panel::left(egui::Id::new("left"))
                .resizable(true)
                .default_size(left.width)
                .size_range(SIDE_WIDTH)
                .frame(panel_frame)
                .show(ui, |ui| {
                    contained(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let b = ui
                                .small_button(icons::FOLD_LEFT)
                                .on_hover_text("Fold this side (Ctrl+[)");
                            named(&b, "Fold the left side");
                            if b.clicked() {
                                out.push(Command::FoldLeft);
                            }
                            for &p in ws.panels() {
                                if ui.selectable_label(panel == p, p.title()).clicked() {
                                    self.frame.set_panel(p);
                                }
                            }
                        });
                        ui.separator();
                        let p = &mut self.panels;
                        match panel {
                            Panel::Find => p.find.ui(ui, doc, Mode::Find, out),
                            Panel::Materials => p.find.ui(ui, doc, Mode::Materials, out),
                            Panel::Envelopes => p.find.ui(ui, doc, Mode::Envelopes, out),
                            Panel::Volumes => p.volumes.ui(ui, doc, out),
                            Panel::Places => scrolled(ui, |ui| {
                                crate::theme::section(ui, icons::SAVE, "Saved views", |ui| {
                                    p.views.ui(ui, doc, out)
                                });
                                if doc.scene.pins.is_empty() {
                                    crate::theme::section(ui, icons::PIN, "Pins", |ui| {
                                        hint(ui, "No pins: right-click a point, Pin here.")
                                    });
                                } else {
                                    p.pins.ui(ui, doc, out);
                                }
                            }),
                            Panel::Problems => scrolled(ui, |ui| p.problems.ui(ui, doc, out)),
                            Panel::Statistics => scrolled(ui, |ui| p.stats.ui(ui, doc, out)),
                            Panel::Fields => scrolled(ui, |ui| p.fields.ui(ui, doc, out)),
                            Panel::Mesh => scrolled(ui, |ui| p.mesh.ui(ui, doc, out)),
                            Panel::Inspector => {}
                        }
                    })
                });
            self.frame.left.width = r.response.rect.width();
        } else {
            egui::Panel::left(egui::Id::new("left rail"))
                .resizable(false)
                .exact_size(RAIL)
                .frame(panel_frame.inner_margin(egui::Margin::symmetric(4, 6)))
                .show(ui, |ui| {
                    let b = ui
                        .button(icons::UNFOLD_LEFT)
                        .on_hover_text("Unfold this side (Ctrl+[)");
                    named(&b, "Unfold the left side");
                    if b.clicked() {
                        out.push(Command::FoldLeft);
                    }
                    ui.separator();
                    for &p in ws.panels() {
                        if ui.button(p.icon()).on_hover_text(p.name()).clicked() {
                            out.push(Command::ShowPanel(p));
                        }
                    }
                });
        }

        // Right: the inspector, or its rail.
        if right.open {
            let r = egui::Panel::right(egui::Id::new("right"))
                .resizable(true)
                .default_size(right.width)
                .size_range(SIDE_WIDTH)
                .frame(panel_frame)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(Panel::Inspector.title()).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let b = ui
                                .small_button(icons::FOLD_RIGHT)
                                .on_hover_text("Fold this side (Ctrl+])");
                            named(&b, "Fold the right side");
                            if b.clicked() {
                                out.push(Command::FoldRight);
                            }
                        });
                    });
                    ui.separator();
                    let inspector = &mut self.panels.inspector;
                    scrolled(ui, |ui| {
                        if inspector.has_content(doc) {
                            inspector.ui(ui, doc, out);
                        } else {
                            hint(
                                ui,
                                "Click a cell to select it; double-click to inspect the point.",
                            );
                        }
                    });
                });
            self.frame.right.width = r.response.rect.width();
        } else {
            egui::Panel::right(egui::Id::new("right rail"))
                .resizable(false)
                .exact_size(RAIL)
                .frame(panel_frame.inner_margin(egui::Margin::symmetric(4, 6)))
                .show(ui, |ui| {
                    let b = ui
                        .button(icons::UNFOLD_RIGHT)
                        .on_hover_text("Unfold this side (Ctrl+])");
                    named(&b, "Unfold the right side");
                    if b.clicked() {
                        out.push(Command::FoldRight);
                    }
                    ui.separator();
                    if ui
                        .button(Panel::Inspector.icon())
                        .on_hover_text(Panel::Inspector.name())
                        .clicked()
                    {
                        out.push(Command::ShowPanel(Panel::Inspector));
                    }
                });
        }

        // What is marked in both views: the surface under the pointer while
        // Alt is held; else the surfaces of a geometry being edited, while
        // its form is shown (not when the inspector is folded or empty);
        // else the surface picked with Alt+click.
        if let Some(doc) = self.doc.as_ref() {
            let want = crate::tools::select::alt_marks(ui.ctx(), doc)
                .or_else(|| self.panels.inspector.marks(ui.ctx().cumulative_pass_nr()))
                .or_else(|| doc.surface_pick.as_ref().and_then(|s| s.marks(&doc.model)));
            if doc.marks.request() != want.as_ref() {
                out.push(Command::MarkSurfaces(want));
            }
        }

        // The middle: the tool row, then the views.
        let base = crate::theme::palette(ui.ctx()).base;
        let open = self.open_universes();
        egui::CentralPanel::no_frame()
            .frame(egui::Frame::NONE.fill(base))
            .show(ui, |ui| {
                let Some(doc) = self.doc.as_mut() else {
                    return;
                };
                egui::Frame::NONE
                    .fill(fill)
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        toolbar::tool_row(ui, &mut self.tools, doc, &open, out);
                    });
                let mut area = ui.available_rect_before_wrap();
                // Results: the field along the ruler, under the views.
                if ws == Workspace::Results && doc.ruler.is_some() && doc.field.is_some() {
                    let h = (0.28 * area.height()).clamp(140.0, 260.0);
                    let strip = egui::Rect::from_min_max(
                        egui::pos2(area.left(), area.bottom() - h),
                        area.max,
                    );
                    area.max.y = strip.top() - 2.0;
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(strip)
                            .layout(egui::Layout::top_down(egui::Align::Min))
                            .id_salt("profile"),
                    );
                    egui::Frame::NONE
                        .fill(fill)
                        .inner_margin(egui::Margin::symmetric(8, 6))
                        .show(&mut child, |ui| {
                            ui.set_min_size(ui.available_size());
                            crate::panels::profile::ui(ui, doc);
                        });
                }
                let views = self.frame.views();
                let shares = self.frame.shares();
                // The slice's header not drawn: its position is not moving.
                if !views.contains(&View::Slice) {
                    doc.editing.slice_moving = false;
                }
                let gap = 2.0;
                let room = area.width() - gap * (views.len() - 1) as f32;
                let mut x = area.min.x;
                for (k, &v) in views.iter().enumerate() {
                    let w = room * shares[k];
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(x, area.min.y),
                        egui::vec2(w, area.height()),
                    );
                    x += w + gap;
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect)
                            .layout(egui::Layout::top_down(egui::Align::Min))
                            .id_salt(("view", k)),
                    );
                    child.set_clip_rect(rect.intersect(ui.clip_rect()));
                    let maximized = self.frame.maximized == Some(v);
                    egui::Frame::NONE
                        .fill(fill)
                        .inner_margin(egui::Margin::symmetric(6, 3))
                        .show(&mut child, |ui| {
                            ui.set_width(ui.available_width());
                            match v {
                                View::ThreeD => headers::view3d(ui, doc, maximized, out),
                                View::Slice => headers::slice(ui, doc, maximized, out),
                            }
                        });
                    match v {
                        View::ThreeD => super::view3d_ui(
                            &mut child,
                            doc,
                            &mut self.tools,
                            &self.render_state,
                            out,
                        ),
                        View::Slice => super::slice_ui(&mut child, doc, &mut self.tools, out),
                    }
                }
                ui.allocate_rect(area, egui::Sense::hover());
            });
    }
}

/// The panels of the sides, drawn by `ViewerApp::workspace_ui`.
#[derive(Default)]
pub struct Panels {
    pub find: crate::panels::find::FindPanel,
    pub volumes: crate::panels::volumes::VolumesPanel,
    pub views: crate::panels::views::ViewsPanel,
    pub pins: crate::panels::pins::PinsPanel,
    pub problems: crate::panels::problems::ProblemsPanel,
    pub stats: crate::panels::stats::StatsPanel,
    pub fields: crate::panels::fields::FieldsPanel,
    pub mesh: crate::panels::mesh::MeshPanel,
    pub inspector: inspector::Inspector,
}

/// `f` in a vertical scroll area filling what is left.
/// `f` in exactly the room `ui` has left, clipped to it: a row too wide is
/// cut off rather than widening the side panel, which egui cannot then
/// drag narrower than its contents.
fn contained(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui)) {
    let rect = ui.available_rect_before_wrap();
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("contained")
            .max_rect(rect)
            .layout(*ui.layout()),
    );
    child.shrink_clip_rect(rect);
    f(&mut child);
    ui.advance_cursor_after_rect(rect);
}

fn scrolled(ui: &mut egui::Ui, f: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, f);
}

/// A line saying what goes in an empty panel.
pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(egui::RichText::new(text).weak());
}

/// Name an icon button for screen readers and tests.
pub fn named(r: &egui::Response, name: &str) {
    let enabled = r.enabled();
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_panel_has_one_home_but_the_inspector() {
        for p in Panel::ALL {
            let homes = Workspace::ALL
                .iter()
                .filter(|w| w.panels().contains(&p))
                .count();
            assert_eq!(homes, usize::from(p != Panel::Inspector), "{p:?}");
        }
    }

    #[test]
    fn showing_a_panel_brings_its_workspace() {
        let mut f = Frame::default();
        f.left.open = false;
        f.show(Panel::Problems);
        assert_eq!(f.workspace, Workspace::Check);
        assert!(f.shows(Panel::Problems));
        f.workspace = Workspace::Explore;
        assert!(!f.shows(Panel::Problems));
        f.right.open = false;
        f.show(Panel::Inspector);
        assert!(f.shows(Panel::Inspector));
    }

    #[test]
    fn settings_are_made_whole() {
        let f: Frame = serde_json::from_str(
            r#"{"workspace": "Check", "panels": ["Fields"], "left": {"width": 9000}}"#,
        )
        .unwrap();
        let f = f.normalized();
        assert_eq!(f.workspace, Workspace::Check);
        assert_eq!(f.panels.len(), 4);
        // Fields is not Explore's: back to Explore's first panel.
        assert_eq!(f.panels[0], Panel::Find);
        assert_eq!(f.panels[1], Panel::Problems);
        assert_eq!(f.left.width, *SIDE_WIDTH.end());
        // The layout of an older version (one per workspace): its sides lost.
        let old: Frame = serde_json::from_str(
            r#"{"workspace": "Results", "arrangements": [{"panel": "Volumes"}]}"#,
        )
        .unwrap();
        let old = old.normalized();
        assert_eq!(old.workspace, Workspace::Results);
        assert_eq!(old.left, Frame::default().left);
    }

    #[test]
    fn workspaces_share_the_sides_and_the_views() {
        let mut f = Frame::default();
        f.left.width = 400.0;
        f.right.open = false;
        f.show(Panel::Volumes);
        let views = f.views();
        for w in Workspace::ALL {
            f.workspace = w;
            assert_eq!((f.left.width, f.right.open), (400.0, false));
            assert_eq!(f.views(), views);
            assert_eq!(f.shares(), &[0.5, 0.5]);
        }
        // Each workspace keeps its own panel.
        f.workspace = Workspace::Explore;
        assert_eq!(f.panel(), Panel::Volumes);
        f.workspace = Workspace::Check;
        assert_eq!(f.panel(), Panel::Problems);
    }
}
