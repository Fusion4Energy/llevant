//! What the left mouse button does in the views.
//!
//! **Handles first.** The cuts and the slice plane in 3D ([`CutHandles`]),
//! and in Tally the FMESH meshes ([`MeshHandles`]), are things with handles,
//! whatever the tool: they hear the pointer before the tool and take the left
//! button when it goes down on one of their handles (or outlines, meshes),
//! so there is no tool to pick before dragging a cut or a mesh. A click
//! elsewhere deselects them and goes on to the tool.
//!
//! **Then the active tool** ([`Toolbox`]): Select (the default) or the
//! Ruler. It gets what the handles leave and draws its own overlay. Camera
//! navigation (middle and right drags, scroll, keys) works whatever the
//! tool.
//!
//! A new tool is a type implementing [`Tool`], added to
//! [`Toolbox::standard`]; a new kind of thing with handles, a [`Layer`].

pub mod handles;
mod mesh;
mod ruler;
mod section;
pub mod select;

pub use mesh::MeshHandles;
pub use ruler::RulerTool;
pub use section::{handle_size, slice_handle_base, CutHandles};
pub use select::SelectTool;

use crate::command::Command;
use crate::document::Document;
use crate::viewport::ViewPort;

pub trait Tool {
    /// For the tool row.
    fn name(&self) -> &'static str;
    /// For the tool row: one of [`crate::icons`].
    fn icon(&self) -> &'static str;
    /// For the status bar: what the tool does and how.
    fn hint(&self) -> &'static str;
    /// The pointer over `view` this frame. True if the tool takes this
    /// frame's left-button drag, so that navigation leaves it alone.
    fn input(&mut self, view: &ViewPort, doc: &Document, out: &mut Vec<Command>) -> bool;
    /// Drawn over the view, after the document's overlays.
    fn overlay(&self, _view: &ViewPort, _doc: &Document, _painter: &egui::Painter) {}
}

/// Things with handles in the views, whatever the tool.
pub trait Layer {
    /// The pointer over `view` this frame. True if this frame's left button
    /// (its press, drag or click) is theirs, so the tool must not see it.
    fn input(&mut self, view: &ViewPort, doc: &Document, out: &mut Vec<Command>) -> bool;
    /// Drawn over the view, before the tool's overlay.
    fn overlay(&self, view: &ViewPort, doc: &Document, painter: &egui::Painter);
    /// For the status bar, while there is something of theirs to drag.
    fn hint(&self, doc: &Document) -> Option<&'static str>;
}

/// The tools, which is active, and the things with handles.
pub struct Toolbox {
    pub tools: Vec<Box<dyn Tool>>,
    pub active: usize,
    pub cuts: CutHandles,
    pub meshes: MeshHandles,
    /// The meshes are shown and handled (in the Tally workspace).
    pub meshes_on: bool,
}

impl Toolbox {
    /// Select first (the default), then the Ruler.
    pub fn standard() -> Toolbox {
        Toolbox {
            tools: vec![
                Box::new(SelectTool::default()),
                Box::new(RulerTool::default()),
            ],
            active: 0,
            cuts: CutHandles::default(),
            meshes: MeshHandles::default(),
            meshes_on: false,
        }
    }

    /// Make the tool called `name` the active one (if there is one).
    pub fn activate(&mut self, name: &str) {
        if let Some(k) = self.tools.iter().position(|t| t.name() == name) {
            self.active = k;
        }
    }

    pub fn active(&self) -> &dyn Tool {
        self.tools[self.active].as_ref()
    }

    pub fn active_mut(&mut self) -> &mut dyn Tool {
        self.tools[self.active].as_mut()
    }

    /// The pointer over `view`: the handles, then the active tool with
    /// what they leave. True if the left button's drag is taken.
    pub fn input(&mut self, view: &ViewPort, doc: &Document, out: &mut Vec<Command>) -> bool {
        let mut taken = self.cuts.input(view, doc, out);
        if self.meshes_on && !taken {
            taken = self.meshes.input(view, doc, out);
        }
        if taken {
            return true;
        }
        self.active_mut().input(view, doc, out)
    }

    /// The handles' overlays, then the tool's.
    pub fn overlay(&self, view: &ViewPort, doc: &Document, painter: &egui::Painter) {
        self.cuts.overlay(view, doc, painter);
        if self.meshes_on {
            self.meshes.overlay(view, doc, painter);
        }
        self.active().overlay(view, doc, painter);
    }

    /// For the status bar: about the handles while some are selected (or a
    /// mesh is being drawn), else about the tool.
    pub fn hint(&self, doc: Option<&Document>) -> &'static str {
        let layers = doc.and_then(|d| {
            let meshes = self.meshes_on.then(|| self.meshes.hint(d)).flatten();
            meshes.or_else(|| self.cuts.hint(d))
        });
        // The Ruler says how to measure, whatever is selected.
        if self.active == 0 {
            layers.unwrap_or_else(|| self.active().hint())
        } else {
            self.active().hint()
        }
    }
}
